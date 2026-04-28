//! Librarian pipeline — end-to-end write path per
//! `docs/concepts/librarian-pipeline.md`.
//!
//! The pipeline compiles agent-emitted Lisp S-expression input into
//! canonical bytecode records by chaining the five stages:
//!
//! ```text
//! &str ─► [Parse] ─► [Bind] ─► [Semantic] ─► [Emit] ─► Vec<CanonicalRecord>
//! ```
//!
//! (Lex is internal to `parse::parse`, so the public entry points are
//! unified into a single `Pipeline::compile_batch` call.)
//!
//! Per invariant § 11.3 (batch atomicity), a batch either commits in
//! full or leaves no trace. `Pipeline` holds the workspace's live
//! `SymbolTable` and a monotonic memory-ID counter; both are cloned at
//! the start of each batch and swapped back only on full-stage success.
//!
//! The pipeline emits:
//!
//! - The four memory record kinds (`Sem`, `Epi`, `Pro`, `Inf`).
//! - `SymbolAlloc` / `SymbolRename` / `SymbolAlias` / `SymbolRetire`
//!   derived from the `bind` mutation journal (spec § 3.4). Symbol
//!   events precede the memory records in the output so log replay
//!   sees allocations before the memory records that reference them.
//!
//! Unsupported forms (tracked under the matching spec):
//!
//! - `correct` and `promote` return `EmitError::Unsupported` pending
//!   the correction + ephemeral-promotion work on the
//!   `temporal-model` / `episode-semantics` tracks.
//! - `query` returns `EmitError::Unsupported` pending the
//!   `read-protocol` track.
//! - ML-callout interface (spec § 6) is not wired; v1 is noop-ML.

use thiserror::Error;

use std::collections::{BTreeMap, BTreeSet};

use crate::bind::{self, BindError, SymbolMutation, SymbolTable};
use crate::canonical::{
    CanonicalRecord, Clocks, EdgeRecord, EpiRecord, InfFlags, InfRecord, ProRecord, SemFlags,
    SemRecord, SymbolEventRecord,
};
use crate::clock::ClockTime;
use crate::dag::{Edge as DagEdge, EdgeKind, SupersessionDag};
use crate::parse::{self, ParseError};
use crate::semantic::{self, SemanticError, ValidatedForm};
use crate::symbol::{SymbolId, SymbolKind};

/// The librarian pipeline — single-writer compiler from Lisp S-expression
/// input to canonical bytecode.
///
/// Holds the workspace's symbol table, memory-ID counter, monotonic
/// `committed_at` watermark, supersession DAG, and current-state
/// supersession index; mutations to all five are batch-atomic
/// (invariant § 11.3, plus `temporal-model.md` § 12 #1 and § 6.2 #1).
///
/// Does not derive `Eq` because the stored record vectors carry
/// `Value`, which contains `f64` and therefore is only `PartialEq`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pipeline {
    table: SymbolTable,
    next_memory_counter: u64,
    /// Highest `committed_at` assigned by this pipeline so far, or
    /// `None` if no batch has committed yet. Per `temporal-model.md`
    /// § 9.2, the next commit clock is `max(wall_now, self + 1)`.
    last_committed_at: Option<ClockTime>,
    /// Workspace-scoped supersession graph. Extended at emit time by
    /// auto-supersession writes (temporal-model.md § 5).
    dag: SupersessionDag,
    /// Current-state index for supersession detection (§ 5).
    supersession_index: SupersessionIndex,
    /// Every Semantic memory ever emitted (or replayed from the log),
    /// in commit order. Feeds the as-of query resolver
    /// (`temporal-model.md` § 7).
    semantic_records: Vec<SemRecord>,
    /// Secondary index over `semantic_records`: `(s, p) → indices`.
    /// Per `read-protocol.md` § 3.1 the current-state index makes
    /// single-predicate Semantic lookups O(k) in the size of the
    /// `(s, p)` history (typically 1–3) instead of O(n) in the
    /// whole store. The resolver consults this index when `:s` and
    /// `:p` are both pinned; other paths still scan.
    semantic_by_sp_history: BTreeMap<(SymbolId, SymbolId), Vec<usize>>,
    /// Every Episodic memory ever emitted or replayed, in commit
    /// order. Episodic records do not currently participate in
    /// supersession, but retaining them lets downstream tooling
    /// perform exact duplicate checks and future event-oriented reads.
    episodic_records: Vec<EpiRecord>,
    /// Every Procedural memory ever emitted or replayed, in commit
    /// order. Same contract as `semantic_records`.
    procedural_records: Vec<ProRecord>,
    /// Secondary index over `procedural_records`: `rule_id → indices`.
    /// Powers O(k) lookup for `:kind pro` reads once the `rule_id`
    /// read predicate is wired; for now the resolver iterates the
    /// full vec from the `resolve_procedural` entry point.
    procedural_by_rule_history: BTreeMap<SymbolId, Vec<usize>>,
    /// Every Inferential memory ever emitted or replayed, in commit
    /// order. Feeds the Inferential resolver per `temporal-model.md`
    /// § 5.4 + `read-protocol.md` § 3.1 (Inf keyed by `(s, p)` like
    /// Sem; re-derivation with same `(s, p)` + later `valid_at`
    /// supersedes the prior).
    inferential_records: Vec<InfRecord>,
    /// Secondary index over `inferential_records`: `(s, p) → indices`.
    /// Mirrors `semantic_by_sp_history`; lets the resolver walk only
    /// the records at a specific `(s, p)` on a pinned-subject-and-
    /// predicate read.
    inferential_by_sp_history: BTreeMap<(SymbolId, SymbolId), Vec<usize>>,
    /// Decay parameters used by the read path to compute effective
    /// confidence (`confidence-decay.md`). Defaults to the librarian's
    /// v1 parameter table; `Store::open` will eventually load an
    /// `mimir.toml` override and call [`Pipeline::set_decay_config`].
    decay_config: crate::decay::DecayConfig,
    /// Committed-at clock for each Episode the pipeline has seen.
    /// Populated by [`Pipeline::register_episode`] — `Store` calls
    /// this after a successful `commit_batch` and during log replay
    /// when a `Checkpoint` record is seen. Used by the read path's
    /// `:in_episode` / `:after_episode` / `:before_episode`
    /// predicates (`read-protocol.md` § 4.1).
    episode_committed_at: BTreeMap<SymbolId, ClockTime>,
    /// Parent Episode for each Episode that declared one via
    /// `(episode :start :parent_episode @E)`. Populated by
    /// [`Pipeline::register_episode_parent`]. Backs the
    /// `:episode_chain @E` read predicate
    /// (`read-protocol.md` § 4.1 / `episode-semantics.md` § 5.1).
    episode_parent: BTreeMap<SymbolId, SymbolId>,
    /// Metadata captured from an `(episode :start ...)` form during
    /// the most recent `compile_batch`. Consumed by the store at
    /// commit time — see [`Pipeline::take_pending_episode_metadata`].
    pending_episode_metadata: Option<PendingEpisodeMetadata>,
    /// Memories currently pinned (`confidence-decay.md` § 7).
    /// Suspends decay at read time via `DecayFlags::pinned`.
    pinned_memories: BTreeSet<SymbolId>,
    /// Memories currently flagged operator-authoritative
    /// (`confidence-decay.md` § 8). Also suspends decay; populates
    /// `Framing::Authoritative { set_by: OperatorAuthoritative }`.
    authoritative_memories: BTreeSet<SymbolId>,
    /// Reverse parent index for Inferential memories: maps a parent
    /// memory's `SymbolId` to the list of Inferentials that derived
    /// from it (`temporal-model.md` § 5.4). Populated by
    /// [`replay_memory_record`] on Inferential records and by emit.
    /// Used when a Semantic or Procedural parent gets auto-
    /// superseded — the index tells us which Inferentials to attach
    /// `StaleParent` edges to. O(log n) lookup rather than scanning
    /// `semantic_records` / `procedural_records` for every
    /// supersession.
    inferentials_by_parent: BTreeMap<SymbolId, Vec<SymbolId>>,
}

/// Episode metadata captured from an `(episode :start …)` form in
/// the write surface. Flows out of `compile_batch` via
/// [`Pipeline::take_pending_episode_metadata`] so the store layer
/// can attach it to the Episode it's about to emit.
///
/// An `(episode :close)` form carries no metadata; the pipeline
/// still produces `Some(default)` to signal "batch carried an
/// explicit Episode directive" separately from the no-directive
/// case.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PendingEpisodeMetadata {
    /// Label from `:label`.
    pub label: Option<String>,
    /// Parent from `:parent_episode`.
    pub parent_episode: Option<SymbolId>,
    /// Retracted Episodes from `:retracts (@E1 …)`.
    pub retracts: Vec<SymbolId>,
}

/// Current-state index used by auto-supersession detection.
///
/// For each memory-type supersession key (per `temporal-model.md` § 5),
/// tracks the currently-authoritative memory so new writes can look up
/// their predecessor in O(log n). The index is rebuilt from the log at
/// `Store::open`.
///
/// Scope: Semantic (§ 5.1), Procedural (§ 5.2), and Inferential
/// (§ 5.4) supersession on re-derivation. Episodic is out of scope
/// (no auto-supersession per § 5.3).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SupersessionIndex {
    /// `(subject, predicate) -> (memory_id, valid_at)` for the current
    /// Semantic memory at that `(s, p)` — `None` if no live memory.
    semantic_by_sp: BTreeMap<(SymbolId, SymbolId), CurrentSemantic>,
    /// `(subject, predicate) -> (memory_id, valid_at)` for the current
    /// Inferential memory at that `(s, p)` per `temporal-model.md`
    /// § 5.4 (Inf re-derivation supersession mirrors Sem § 5.1). Reuses
    /// [`CurrentSemantic`] shape — identical `(memory_id, valid_at)`
    /// record — to avoid a duplicate struct for the same data.
    inferential_by_sp: BTreeMap<(SymbolId, SymbolId), CurrentSemantic>,
    /// `rule_id -> (memory_id, committed_at)` for current Procedural
    /// memories keyed by rule identifier (§ 5.2 primary key).
    procedural_by_rule: BTreeMap<SymbolId, CurrentProcedural>,
    /// `(canonical(trigger), scope) -> (memory_id, committed_at)` for
    /// current Procedural memories keyed by the `(trigger, scope)`
    /// pair (§ 5.2 secondary key). `Value` has no `Ord` impl because
    /// of `f64`, so the trigger is keyed by its canonical-byte
    /// encoding (stable within a process; not persisted).
    procedural_by_trigger_scope: BTreeMap<(Vec<u8>, SymbolId), CurrentProcedural>,
    /// Reverse index from `memory_id` back to the two Procedural
    /// index keys it occupies. Used during supersession to clear out
    /// the OTHER key of a memory that was matched via only one key
    /// (spec § 5.2 invalidates on either key matching).
    procedural_keys_by_memory: BTreeMap<SymbolId, ProceduralKeys>,
}

/// Index entry for a currently-authoritative Semantic memory.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct CurrentSemantic {
    memory_id: SymbolId,
    valid_at: ClockTime,
}

/// Safety cap on Episode-chain traversal. Parent cycles cannot
/// form via the write path (parent must already be committed before
/// a child declares it) but replay of a corrupted log could present
/// one; the cap keeps `episode_chain` bounded in pathological cases.
pub const MAX_EPISODE_CHAIN_DEPTH: usize = 1024;

/// The two index keys a Procedural memory occupies.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ProceduralKeys {
    rule_id: SymbolId,
    trigger_scope: (Vec<u8>, SymbolId),
}

/// Index entry for a currently-authoritative Procedural memory.
/// Tracks `committed_at` so the emit path can detect intra-batch
/// conflicts — two Pro writes in the same batch with overlapping
/// supersession keys share `committed_at`, producing a
/// zero-duration "supersession" that's almost certainly an agent
/// bug.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct CurrentProcedural {
    memory_id: SymbolId,
    committed_at: ClockTime,
}

impl Pipeline {
    /// Construct a pipeline with an empty symbol table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Read-only view of the workspace symbol table.
    #[must_use]
    pub fn table(&self) -> &SymbolTable {
        &self.table
    }

    /// Read-only view of the decay parameters used by the read path.
    /// Defaults to [`crate::decay::DecayConfig::librarian_defaults`];
    /// the store layer installs per-workspace overrides when an
    /// `mimir.toml` is present.
    #[must_use]
    pub fn decay_config(&self) -> &crate::decay::DecayConfig {
        &self.decay_config
    }

    /// Replace the decay parameters used by the read path. Intended
    /// for the store layer to wire `mimir.toml` overrides in at
    /// `Store::open`; tests may also call this directly.
    pub fn set_decay_config(&mut self, cfg: crate::decay::DecayConfig) {
        self.decay_config = cfg;
    }

    /// Replay a `SYMBOL_ALLOC` record into the pipeline's symbol table.
    /// Thin pass-through to [`SymbolTable::replay_allocate`]; exposed
    /// for [`Store::open`](crate::store::Store::open).
    ///
    /// # Errors
    ///
    /// Propagates [`BindError`] variants from the underlying
    /// `SymbolTable::replay_allocate` call.
    pub fn replay_allocate(
        &mut self,
        id: SymbolId,
        name: String,
        kind: SymbolKind,
    ) -> Result<(), BindError> {
        self.table.replay_allocate(id, name, kind)
    }

    /// Replay a `SYMBOL_ALIAS` record.
    ///
    /// # Errors
    ///
    /// Propagates [`BindError`] variants from the underlying
    /// `SymbolTable::replay_alias` call.
    pub fn replay_alias(&mut self, id: SymbolId, alias: String) -> Result<(), BindError> {
        self.table.replay_alias(id, alias)
    }

    /// Replay a `SYMBOL_RENAME` record.
    ///
    /// # Errors
    ///
    /// Propagates [`BindError`] variants from the underlying
    /// `SymbolTable::replay_rename` call.
    pub fn replay_rename(&mut self, id: SymbolId, new_canonical: String) -> Result<(), BindError> {
        self.table.replay_rename(id, new_canonical)
    }

    /// Replay a `SYMBOL_RETIRE` record.
    ///
    /// # Errors
    ///
    /// Propagates [`BindError`] variants from the underlying
    /// `SymbolTable::replay_retire` call.
    pub fn replay_retire(&mut self, id: SymbolId, name: String) -> Result<(), BindError> {
        self.table.replay_retire(id, name)
    }

    /// Set the pipeline's memory-ID counter. Used by
    /// [`Store::open`](crate::store::Store::open) to restore the
    /// counter from durable state.
    pub fn set_next_memory_counter(&mut self, counter: u64) {
        self.next_memory_counter = counter;
    }

    /// Read the pipeline's memory-ID counter — the value that the
    /// next `allocate_memory_id` call will synthesise into
    /// `__mem_{n}`. Exposed for tests and for inspection tooling.
    #[must_use]
    pub fn next_memory_counter(&self) -> u64 {
        self.next_memory_counter
    }

    /// The highest `committed_at` this pipeline has assigned, or
    /// `None` before the first successful batch. Per `temporal-model.md`
    /// § 9.2 / § 12 invariant #1, every successful commit must exceed
    /// this value; the pipeline enforces that internally via
    /// `max(wall_now, last_committed_at + 1)`.
    #[must_use]
    pub fn last_committed_at(&self) -> Option<ClockTime> {
        self.last_committed_at
    }

    /// Advance the pipeline's monotonic commit watermark to `at`.
    ///
    /// Used by [`Store::open`](crate::store::Store) during replay to
    /// restore the watermark from the highest `committed_at` durably
    /// recorded in the log. No-op if `at <= last_committed_at`.
    pub fn advance_last_committed_at(&mut self, at: ClockTime) {
        if self.last_committed_at.is_none_or(|prev| at > prev) {
            self.last_committed_at = Some(at);
        }
    }

    /// Read-only view of the supersession DAG.
    #[must_use]
    pub fn dag(&self) -> &SupersessionDag {
        &self.dag
    }

    /// Every Semantic memory this pipeline has committed or replayed,
    /// in commit order. Consumed by the as-of resolver
    /// (`crate::resolver`) per `temporal-model.md` § 7.
    #[must_use]
    pub fn semantic_records(&self) -> &[SemRecord] {
        &self.semantic_records
    }

    /// Every Episodic memory this pipeline has committed or replayed,
    /// in commit order.
    #[must_use]
    pub fn episodic_records(&self) -> &[EpiRecord] {
        &self.episodic_records
    }

    /// Every Procedural memory this pipeline has committed or
    /// replayed, in commit order.
    #[must_use]
    pub fn procedural_records(&self) -> &[ProRecord] {
        &self.procedural_records
    }

    /// Indices in [`Pipeline::semantic_records`] of every Semantic
    /// record ever emitted at `(s, p)`. Returns an empty slice when
    /// the pair has no history. O(log n) lookup; the resolver uses
    /// this to avoid scanning the full record history.
    #[must_use]
    pub fn semantic_history_at(&self, s: SymbolId, p: SymbolId) -> &[usize] {
        self.semantic_by_sp_history
            .get(&(s, p))
            .map_or(&[], Vec::as_slice)
    }

    /// Indices in [`Pipeline::procedural_records`] of every
    /// Procedural record ever emitted under `rule_id`. Returns an
    /// empty slice when the rule has no history.
    #[must_use]
    pub fn procedural_history_for(&self, rule_id: SymbolId) -> &[usize] {
        self.procedural_by_rule_history
            .get(&rule_id)
            .map_or(&[], Vec::as_slice)
    }

    /// Every Inferential memory this pipeline has committed or
    /// replayed, in commit order. Consumed by the Inferential resolver
    /// (`crate::resolver::resolve_inferential`) per `temporal-model.md`
    /// § 5.4 — Inf is keyed by `(s, p)` like Sem, with re-derivation
    /// as the auto-supersession trigger.
    #[must_use]
    pub fn inferential_records(&self) -> &[InfRecord] {
        &self.inferential_records
    }

    /// Indices in [`Pipeline::inferential_records`] of every
    /// Inferential record ever emitted at `(s, p)`. Returns an empty
    /// slice when the pair has no history. Mirrors
    /// [`Pipeline::semantic_history_at`].
    #[must_use]
    pub fn inferential_history_at(&self, s: SymbolId, p: SymbolId) -> &[usize] {
        self.inferential_by_sp_history
            .get(&(s, p))
            .map_or(&[], Vec::as_slice)
    }

    /// Register that Episode `episode_id` committed at `at`. Called
    /// by [`crate::store::Store`] after a successful commit and
    /// during log replay for every `Checkpoint` record. The mapping
    /// backs the read path's Episode-scoped predicates per
    /// `read-protocol.md` § 4.1 — an `(s, p)` → `(episode_id, at)`
    /// mapping lets `:in_episode @E` resolve by comparing a
    /// candidate's `committed_at` against `@E`'s registered clock.
    ///
    /// Redundant registrations (same id + same clock) are a no-op;
    /// id collisions with a different clock are ignored on the
    /// assumption that replay walks records in order and the first
    /// registration wins.
    pub fn register_episode(&mut self, episode_id: SymbolId, at: ClockTime) {
        self.episode_committed_at.entry(episode_id).or_insert(at);
    }

    /// The `committed_at` clock Episode `episode_id` was committed at,
    /// or `None` if the pipeline has not seen that Episode. See
    /// [`Pipeline::register_episode`].
    #[must_use]
    pub fn episode_committed_at(&self, episode_id: SymbolId) -> Option<ClockTime> {
        self.episode_committed_at.get(&episode_id).copied()
    }

    /// Iterate every Episode the pipeline has registered as
    /// `(episode_id, committed_at)` pairs.
    ///
    /// Iteration order is **unspecified** (the underlying storage is a
    /// `HashMap`); callers that need a stable order — e.g. paginated
    /// listing for `mimir-mcp::mimir_list_episodes` — must collect
    /// and sort. Sorting by `committed_at` is the canonical UI choice
    /// since it matches the durability-order on disk.
    pub fn iter_episodes(&self) -> impl Iterator<Item = (SymbolId, ClockTime)> + '_ {
        self.episode_committed_at.iter().map(|(id, at)| (*id, *at))
    }

    /// Record that Episode `child` has `parent` as its parent Episode.
    /// Called by the store after a batch with `(episode :start
    /// :parent_episode @E)` metadata, and during replay when an
    /// `EpisodeMeta` record carries `parent_episode_id`. Idempotent
    /// on duplicate calls with the same parent; a conflicting parent
    /// on an Episode already registered is ignored (first-write wins,
    /// matching replay's append-only semantics).
    pub fn register_episode_parent(&mut self, child: SymbolId, parent: SymbolId) {
        self.episode_parent.entry(child).or_insert(parent);
    }

    /// The parent Episode of `episode_id`, or `None` if the Episode
    /// has no parent (or the pipeline has not seen it).
    #[must_use]
    pub fn episode_parent(&self, episode_id: SymbolId) -> Option<SymbolId> {
        self.episode_parent.get(&episode_id).copied()
    }

    /// Take the metadata captured from the most recent
    /// `compile_batch`'s `(episode :start …)` form, if any. Clears
    /// the pending slot so a subsequent batch without a directive
    /// doesn't reuse stale metadata.
    pub fn take_pending_episode_metadata(&mut self) -> Option<PendingEpisodeMetadata> {
        self.pending_episode_metadata.take()
    }

    /// `true` if `memory_id` is currently pinned
    /// (`confidence-decay.md` § 7). Pinned memories skip decay.
    #[must_use]
    pub fn is_pinned(&self, memory_id: SymbolId) -> bool {
        self.pinned_memories.contains(&memory_id)
    }

    /// `true` if `memory_id` is currently flagged
    /// operator-authoritative (`confidence-decay.md` § 8).
    /// Authoritative memories skip decay.
    #[must_use]
    pub fn is_authoritative(&self, memory_id: SymbolId) -> bool {
        self.authoritative_memories.contains(&memory_id)
    }

    /// Replay a `Pin` / `Unpin` / `AuthoritativeSet` /
    /// `AuthoritativeClear` flag event from the canonical log.
    /// Called by [`crate::store::Store::open`] during recovery so
    /// the pipeline's pin / authoritative sets reflect the durable
    /// state.
    pub fn replay_flag(&mut self, record: &CanonicalRecord) {
        match record {
            CanonicalRecord::Pin(r) => {
                self.pinned_memories.insert(r.memory_id);
            }
            CanonicalRecord::Unpin(r) => {
                self.pinned_memories.remove(&r.memory_id);
            }
            CanonicalRecord::AuthoritativeSet(r) => {
                self.authoritative_memories.insert(r.memory_id);
            }
            CanonicalRecord::AuthoritativeClear(r) => {
                self.authoritative_memories.remove(&r.memory_id);
            }
            _ => {} // not a flag event — no-op
        }
    }

    /// Walk the Episode chain from `episode_id` up through parents.
    /// Yields `episode_id` first, then its parent, grandparent, and
    /// so on. Bounded by [`MAX_EPISODE_CHAIN_DEPTH`] to guard
    /// against pathological or corrupt parent cycles (the binder
    /// rejects cycles at write time, but replay of a corrupted log
    /// might still present one).
    pub fn episode_chain(&self, episode_id: SymbolId) -> impl Iterator<Item = SymbolId> + '_ {
        let mut current = Some(episode_id);
        let mut depth = 0_usize;
        std::iter::from_fn(move || {
            if depth >= MAX_EPISODE_CHAIN_DEPTH {
                return None;
            }
            let id = current?;
            depth += 1;
            current = self.episode_parent.get(&id).copied();
            Some(id)
        })
    }

    /// Replay one edge from the canonical log into the supersession
    /// DAG. Called by [`Store::open`](crate::store::Store) during
    /// recovery; the acyclicity check still runs, so a log with a
    /// corruption-introduced cycle surfaces as an error rather than a
    /// silent invariant violation.
    ///
    /// # Errors
    ///
    /// Propagates [`DagError`](crate::dag::DagError) variants from
    /// `SupersessionDag::add_edge`.
    pub fn replay_edge(&mut self, edge: crate::dag::Edge) -> Result<(), crate::dag::DagError> {
        self.dag.add_edge(edge)
    }

    /// Replay a canonical record into the pipeline's
    /// supersession-detection indices. Idempotent when records are
    /// applied in log order; a replayed forward-supersession Sem
    /// record replaces the prior `(s, p)` entry, a retroactive Sem
    /// record does not; a Procedural record matching either
    /// supersession key (§ 5.2) clears the prior entries and inserts
    /// under both of its keys.
    ///
    /// Non-memory records are ignored; callers can pass every record
    /// and let this method filter internally.
    ///
    /// Scope: Semantic + Procedural. Inferential staling (§ 5.4) is
    /// tracked in issue #29.
    pub fn replay_memory_record(&mut self, record: &CanonicalRecord) {
        match record {
            CanonicalRecord::Sem(sem) => {
                let key = (sem.s, sem.p);
                let sem_index = self.semantic_records.len();
                self.semantic_records.push(sem.clone());
                self.semantic_by_sp_history
                    .entry(key)
                    .or_default()
                    .push(sem_index);
                let replace = self
                    .supersession_index
                    .semantic_by_sp
                    .get(&key)
                    .is_none_or(|existing| sem.clocks.valid_at > existing.valid_at);
                if replace {
                    self.supersession_index.semantic_by_sp.insert(
                        key,
                        CurrentSemantic {
                            memory_id: sem.memory_id,
                            valid_at: sem.clocks.valid_at,
                        },
                    );
                }
            }
            CanonicalRecord::Epi(epi) => {
                self.episodic_records.push(epi.clone());
            }
            CanonicalRecord::Inf(inf) => {
                // Keep the reverse parent index in sync on replay
                // so post-open supersessions can emit StaleParent
                // edges against already-committed Inferentials
                // (temporal-model.md § 5.4).
                for parent in &inf.derived_from {
                    self.inferentials_by_parent
                        .entry(*parent)
                        .or_default()
                        .push(inf.memory_id);
                }
                // Record + history for the resolver. Mirror Sem: push
                // the record in commit order, record its index in the
                // `(s, p)` history, and update the current-state
                // supersession index so post-open writes can auto-
                // supersede. Replay skips the conflict check — the
                // original write time rejected any such conflict and
                // could not have reached the log.
                let inf_index = self.inferential_records.len();
                let key = (inf.s, inf.p);
                let valid_at = inf.clocks.valid_at;
                self.inferential_records.push(inf.clone());
                self.inferential_by_sp_history
                    .entry(key)
                    .or_default()
                    .push(inf_index);
                let replace = self
                    .supersession_index
                    .inferential_by_sp
                    .get(&key)
                    .is_none_or(|existing| valid_at > existing.valid_at);
                if replace {
                    self.supersession_index.inferential_by_sp.insert(
                        key,
                        CurrentSemantic {
                            memory_id: inf.memory_id,
                            valid_at,
                        },
                    );
                }
            }
            CanonicalRecord::Pro(pro) => {
                let pro_index = self.procedural_records.len();
                self.procedural_records.push(pro.clone());
                self.procedural_by_rule_history
                    .entry(pro.rule_id)
                    .or_default()
                    .push(pro_index);
                // Log replay path — skip the intra-batch-conflict
                // check because any such conflict was already
                // rejected at the original write time (and therefore
                // couldn't have reached the log). Returned list of
                // superseded memories is discarded: replay doesn't
                // emit edges, the log already carries them as
                // separate `Supersedes` records that `replay_edge`
                // handles.
                replay_procedural_supersession(
                    &mut self.supersession_index,
                    pro.memory_id,
                    pro.clocks.committed_at,
                    pro.rule_id,
                    &pro.trigger,
                    pro.scope,
                );
            }
            _ => {}
        }
    }

    /// Allocate a fresh `__ep_{counter}` symbol for use as a
    /// `CHECKPOINT` record's `episode_id`. The synthesized name follows
    /// the same reserved-prefix convention as `__mem_{n}` (see
    /// `allocate_memory_id`'s collision-handling note). Used by the
    /// [`Store`](crate::store::Store) commit path; exposed as `pub`
    /// because `Store` composes `Pipeline` rather than inheriting from
    /// it.
    ///
    /// # Errors
    ///
    /// - [`EmitError::MemoryIdAllocation`] if the synthesized name
    ///   collides with an existing symbol (same pathological-agent
    ///   case as memory-ID allocation).
    pub fn allocate_episode_symbol(&mut self, counter: u64) -> Result<SymbolId, EmitError> {
        let name = format!("__ep_{counter}");
        self.table
            .allocate(name.clone(), SymbolKind::Memory)
            .map_err(|cause| EmitError::MemoryIdAllocation { name, cause })
    }

    /// Compile one batch of agent input into canonical records.
    ///
    /// `wall_now` is the librarian's host wall clock at the start of
    /// this batch; injected for determinism (tests pass a fixed
    /// `ClockTime`). The batch's `committed_at` is derived from
    /// `wall_now` via the monotonic rule in `temporal-model.md` § 9.2:
    /// `effective_now = max(wall_now, last_committed_at + 1)`. All
    /// memory records within the batch share that same `effective_now`
    /// as their `committed_at` and — for non-Episodic kinds — their
    /// `observed_at`.
    ///
    /// Future-validity rejection (semantic stage) uses the raw
    /// `wall_now`, not the monotonic watermark, so a transiently
    /// inflated watermark cannot relax the "no future writes without
    /// `:projected true`" rule.
    ///
    /// # Errors
    ///
    /// Any stage may return a [`PipelineError`]; on error the workspace
    /// state (symbol table, memory counter, and commit watermark) is
    /// untouched. [`PipelineError::ClockExhausted`] fires if the
    /// monotonic bump would reach the reserved `u64::MAX` sentinel.
    ///
    /// # Example
    ///
    /// ```
    /// # #![allow(clippy::unwrap_used)]
    /// use mimir_core::pipeline::Pipeline;
    /// use mimir_core::ClockTime;
    ///
    /// let mut pipe = Pipeline::new();
    /// let now = ClockTime::try_from_millis(1_713_350_400_000).expect("non-sentinel");
    /// let input = "(sem @alice @knows @bob :src @observation :c 0.8 :v 2024-01-15)";
    /// let records = pipe.compile_batch(input, now).unwrap();
    /// // The batch emits a `Sem` memory record preceded by `SymbolAlloc`
    /// // records for every first-use symbol name.
    /// assert!(records.iter().any(|r| matches!(r, mimir_core::canonical::CanonicalRecord::Sem(_))));
    /// ```
    pub fn compile_batch(
        &mut self,
        input: &str,
        wall_now: ClockTime,
    ) -> Result<Vec<CanonicalRecord>, PipelineError> {
        // observability.md: `mimir.pipeline.compile_batch` span. Fields
        // are `Empty` until the emit stage finishes so the span still
        // records timing on error paths (counts stay unset).
        let span = tracing::info_span!(
            "mimir.pipeline.compile_batch",
            input_len = input.len(),
            record_count = tracing::field::Empty,
            memory_count = tracing::field::Empty,
            edge_count = tracing::field::Empty,
        );
        let _enter = span.enter();

        let forms = parse::parse(input).map_err(PipelineError::Parse)?;

        // Compute the batch's `committed_at` before running stateful
        // stages so a clock-exhaustion error cannot leave partial work.
        let effective_now = monotonic_commit_clock(wall_now, self.last_committed_at)?;

        // Clone live state so mid-batch failures cannot leak partial
        // mutations. Every state field the emit stage may touch lands
        // here so full-batch rollback is trivial — drop the working
        // copies and leave `self` untouched.
        let mut working_table = self.table.clone();
        let mut working_counter = self.next_memory_counter;
        let mut working_dag = self.dag.clone();
        let mut working_index = self.supersession_index.clone();
        let mut working_sem_records = self.semantic_records.clone();
        let mut working_sem_by_sp = self.semantic_by_sp_history.clone();
        let mut working_epi_records = self.episodic_records.clone();
        let mut working_pro_records = self.procedural_records.clone();
        let mut working_pro_by_rule = self.procedural_by_rule_history.clone();
        let mut working_inf_records = self.inferential_records.clone();
        let mut working_inf_by_sp = self.inferential_by_sp_history.clone();

        let (bound, journal) =
            bind::bind(forms, &mut working_table).map_err(PipelineError::Bind)?;

        // Semantic validation (future-validity rejection) uses the raw
        // wall clock, which is the librarian's estimate of "now" for
        // judging agent clock skew — not the monotonic watermark, which
        // may transiently lead the wall clock after a regression.
        let validated =
            semantic::validate(bound, &working_table, wall_now).map_err(PipelineError::Semantic)?;

        let mut working_pending_meta: Option<PendingEpisodeMetadata> = None;
        let mut working_pinned = self.pinned_memories.clone();
        let mut working_authoritative = self.authoritative_memories.clone();
        let mut working_infs_by_parent = self.inferentials_by_parent.clone();
        let mut emit_state = EmitState {
            table: &mut working_table,
            counter: &mut working_counter,
            dag: &mut working_dag,
            index: &mut working_index,
            semantic_records: &mut working_sem_records,
            semantic_by_sp: &mut working_sem_by_sp,
            episodic_records: &mut working_epi_records,
            procedural_records: &mut working_pro_records,
            procedural_by_rule: &mut working_pro_by_rule,
            inferential_records: &mut working_inf_records,
            inferential_by_sp: &mut working_inf_by_sp,
            pending_episode: &mut working_pending_meta,
            pinned: &mut working_pinned,
            authoritative: &mut working_authoritative,
            inferentials_by_parent: &mut working_infs_by_parent,
            now: effective_now,
        };
        let records = emit(&validated, &journal, &mut emit_state).map_err(PipelineError::Emit)?;

        // All stages succeeded — commit.
        self.table = working_table;
        self.next_memory_counter = working_counter;
        self.last_committed_at = Some(effective_now);
        self.dag = working_dag;
        self.supersession_index = working_index;
        self.semantic_records = working_sem_records;
        self.semantic_by_sp_history = working_sem_by_sp;
        self.episodic_records = working_epi_records;
        self.procedural_records = working_pro_records;
        self.procedural_by_rule_history = working_pro_by_rule;
        self.inferential_records = working_inf_records;
        self.inferential_by_sp_history = working_inf_by_sp;
        self.pending_episode_metadata = working_pending_meta;
        self.pinned_memories = working_pinned;
        self.authoritative_memories = working_authoritative;
        self.inferentials_by_parent = working_infs_by_parent;

        let (memory_count, edge_count) = count_memory_and_edge_records(&records);
        span.record("record_count", records.len());
        span.record("memory_count", memory_count);
        span.record("edge_count", edge_count);

        Ok(records)
    }
}

/// `(memory_count, edge_count)` tally over a record batch — identifiers
/// only, no payload inspection. Consumed by the
/// `mimir.pipeline.compile_batch` span.
fn count_memory_and_edge_records(records: &[CanonicalRecord]) -> (usize, usize) {
    let mut memory = 0_usize;
    let mut edge = 0_usize;
    for r in records {
        match r {
            CanonicalRecord::Sem(_)
            | CanonicalRecord::Epi(_)
            | CanonicalRecord::Pro(_)
            | CanonicalRecord::Inf(_) => memory += 1,
            CanonicalRecord::Supersedes(_)
            | CanonicalRecord::Corrects(_)
            | CanonicalRecord::StaleParent(_)
            | CanonicalRecord::Reconfirms(_) => edge += 1,
            _ => {}
        }
    }
    (memory, edge)
}

/// Mutable write-time state threaded through the emit stage.
///
/// Bundles every field that `emit` / `emit_form` may mutate so the
/// function signatures don't grow a parameter for each. Individual
/// helpers borrow what they need.
struct EmitState<'a> {
    table: &'a mut SymbolTable,
    counter: &'a mut u64,
    dag: &'a mut SupersessionDag,
    index: &'a mut SupersessionIndex,
    semantic_records: &'a mut Vec<SemRecord>,
    /// `(s, p) -> indices` over `semantic_records`; see
    /// [`Pipeline::semantic_by_sp_history`].
    semantic_by_sp: &'a mut BTreeMap<(SymbolId, SymbolId), Vec<usize>>,
    episodic_records: &'a mut Vec<EpiRecord>,
    procedural_records: &'a mut Vec<ProRecord>,
    /// `rule_id -> indices` over `procedural_records`; see
    /// [`Pipeline::procedural_by_rule_history`].
    procedural_by_rule: &'a mut BTreeMap<SymbolId, Vec<usize>>,
    inferential_records: &'a mut Vec<InfRecord>,
    /// `(s, p) -> indices` over `inferential_records`; see
    /// [`Pipeline::inferential_history_at`].
    inferential_by_sp: &'a mut BTreeMap<(SymbolId, SymbolId), Vec<usize>>,
    /// Pending batch-level Episode metadata from an
    /// `(episode :start …)` form, threaded through so emit can
    /// populate and `compile_batch` can commit. Rolled back with the
    /// rest of working state on failure.
    pending_episode: &'a mut Option<PendingEpisodeMetadata>,
    /// Pinned-memory set (`confidence-decay.md` § 7).
    pinned: &'a mut BTreeSet<SymbolId>,
    /// Operator-authoritative memory set
    /// (`confidence-decay.md` § 8).
    authoritative: &'a mut BTreeSet<SymbolId>,
    /// Reverse parent index for Inferential staling
    /// (`temporal-model.md` § 5.4).
    inferentials_by_parent: &'a mut BTreeMap<SymbolId, Vec<SymbolId>>,
    now: ClockTime,
}

/// Compute the batch commit clock per `temporal-model.md` § 9.2:
/// `max(wall_now, last_committed_at + 1)`. Returns
/// [`PipelineError::ClockExhausted`] if bumping past `last_committed_at`
/// would reach the `u64::MAX` sentinel that [`ClockTime`] refuses.
fn monotonic_commit_clock(
    wall_now: ClockTime,
    last_committed_at: Option<ClockTime>,
) -> Result<ClockTime, PipelineError> {
    let Some(prev) = last_committed_at else {
        return Ok(wall_now);
    };
    if wall_now > prev {
        return Ok(wall_now);
    }
    // Wall clock did not advance past the previous commit. Bump by 1ms.
    let next_raw = prev
        .as_millis()
        .checked_add(1)
        .ok_or(PipelineError::ClockExhausted {
            last_committed_at: prev,
        })?;
    ClockTime::try_from_millis(next_raw).map_err(|_| PipelineError::ClockExhausted {
        last_committed_at: prev,
    })
}

fn emit(
    forms: &[ValidatedForm],
    journal: &[SymbolMutation],
    state: &mut EmitState,
) -> Result<Vec<CanonicalRecord>, EmitError> {
    // Symbol events first, so replay sees allocations before the memory
    // records that reference their IDs.
    let mut out = Vec::with_capacity(journal.len() + forms.len());
    for mutation in journal {
        out.push(emit_symbol_mutation(mutation, state.now));
    }
    for form in forms {
        // Alias / Rename / Retire produce no memory-level record —
        // their durability comes via the SYMBOL_* canonical records
        // emitted above from the bind journal.
        if matches!(
            form,
            ValidatedForm::Alias { .. }
                | ValidatedForm::Rename { .. }
                | ValidatedForm::Retire { .. }
        ) {
            continue;
        }
        emit_form(form, state, &mut out)?;
    }
    Ok(out)
}

fn emit_symbol_mutation(mutation: &SymbolMutation, now: ClockTime) -> CanonicalRecord {
    match mutation {
        SymbolMutation::Allocate { id, name, kind } => {
            CanonicalRecord::SymbolAlloc(SymbolEventRecord {
                symbol_id: *id,
                name: name.clone(),
                symbol_kind: *kind,
                at: now,
            })
        }
        SymbolMutation::Rename {
            id,
            new_canonical,
            kind,
        } => CanonicalRecord::SymbolRename(SymbolEventRecord {
            symbol_id: *id,
            name: new_canonical.clone(),
            symbol_kind: *kind,
            at: now,
        }),
        SymbolMutation::Alias { id, alias, kind } => {
            CanonicalRecord::SymbolAlias(SymbolEventRecord {
                symbol_id: *id,
                name: alias.clone(),
                symbol_kind: *kind,
                at: now,
            })
        }
        SymbolMutation::Retire { id, name, kind } => {
            CanonicalRecord::SymbolRetire(SymbolEventRecord {
                symbol_id: *id,
                name: name.clone(),
                symbol_kind: *kind,
                at: now,
            })
        }
    }
}

#[allow(clippy::too_many_lines)]
fn emit_form(
    form: &ValidatedForm,
    state: &mut EmitState,
    out: &mut Vec<CanonicalRecord>,
) -> Result<(), EmitError> {
    match form {
        ValidatedForm::Sem {
            s,
            p,
            o,
            source,
            confidence,
            valid_at,
            projected,
            ..
        } => {
            let memory_id = allocate_memory_id(state, out)?;
            // Auto-supersession per temporal-model.md § 5.1: look up
            // an existing Semantic memory at `(s, p)` and decide
            // forward / retroactive / conflict.
            let (record_invalid_at, supersession) =
                resolve_semantic_supersession(state.index, memory_id, *s, *p, *valid_at)?;
            let sem = SemRecord {
                memory_id,
                s: *s,
                p: *p,
                o: o.clone(),
                source: *source,
                confidence: *confidence,
                clocks: Clocks {
                    valid_at: *valid_at,
                    observed_at: state.now,
                    committed_at: state.now,
                    invalid_at: record_invalid_at,
                },
                flags: SemFlags {
                    projected: *projected,
                },
            };
            let sem_index = state.semantic_records.len();
            state.semantic_records.push(sem.clone());
            state
                .semantic_by_sp
                .entry((*s, *p))
                .or_default()
                .push(sem_index);
            out.push(CanonicalRecord::Sem(sem));
            // Emit the Supersedes edge *after* the new memory so a
            // log reader sees the memory first, then the edge that
            // refers to it.
            if let Some(target) = supersession {
                emit_supersedes_edge(state, out, memory_id, target)?;
            }
        }
        ValidatedForm::Epi {
            event_id,
            kind,
            participants,
            location,
            at_time,
            observed_at,
            source,
            confidence,
            ..
        } => {
            let memory_id = allocate_memory_id(state, out)?;
            let epi = EpiRecord {
                memory_id,
                event_id: *event_id,
                kind: *kind,
                participants: participants.clone(),
                location: *location,
                at_time: *at_time,
                observed_at: *observed_at,
                source: *source,
                confidence: *confidence,
                committed_at: state.now,
                invalid_at: None,
            };
            state.episodic_records.push(epi.clone());
            out.push(CanonicalRecord::Epi(epi));
        }
        ValidatedForm::Pro {
            rule_id,
            trigger,
            action,
            precondition,
            scope,
            source,
            confidence,
            ..
        } => {
            let memory_id = allocate_memory_id(state, out)?;
            let pro = ProRecord {
                memory_id,
                rule_id: *rule_id,
                trigger: trigger.clone(),
                action: action.clone(),
                precondition: precondition.clone(),
                scope: *scope,
                source: *source,
                confidence: *confidence,
                clocks: Clocks {
                    valid_at: state.now,
                    observed_at: state.now,
                    committed_at: state.now,
                    invalid_at: None,
                },
            };
            let pro_index = state.procedural_records.len();
            state.procedural_records.push(pro.clone());
            state
                .procedural_by_rule
                .entry(*rule_id)
                .or_default()
                .push(pro_index);
            out.push(CanonicalRecord::Pro(pro));
            // Auto-supersession per § 5.2: dedup by `rule_id` OR
            // `(trigger, scope)`. Either match triggers supersession;
            // if both match distinct prior memories, both get edges.
            let superseded = apply_procedural_supersession(
                state.index,
                memory_id,
                state.now,
                *rule_id,
                trigger,
                *scope,
            )?;
            for old in superseded {
                emit_supersedes_edge(state, out, memory_id, old)?;
            }
        }
        ValidatedForm::Inf {
            s,
            p,
            o,
            derived_from,
            method,
            confidence,
            valid_at,
            projected,
        } => {
            let memory_id = allocate_memory_id(state, out)?;
            // Write-time stale flag (`temporal-model.md` § 5.4):
            // an Inferential derived from an already-superseded
            // parent is born stale. A parent is superseded iff the
            // DAG carries an incoming `Supersedes` edge on it.
            let born_stale = derived_from
                .iter()
                .any(|parent| parent_is_superseded(state.dag, *parent));
            // Auto-supersession per temporal-model.md § 5.4 ("auto-
            // supersession rule as if Inferential were Semantic —
            // same (s, p) later valid_at"). Shares the Sem § 5.1
            // forward / retroactive / conflict logic.
            let (record_invalid_at, supersession) =
                resolve_inferential_supersession(state.index, memory_id, *s, *p, *valid_at)?;
            let inf = InfRecord {
                memory_id,
                s: *s,
                p: *p,
                o: o.clone(),
                derived_from: derived_from.clone(),
                method: *method,
                confidence: *confidence,
                clocks: Clocks {
                    valid_at: *valid_at,
                    observed_at: state.now,
                    committed_at: state.now,
                    invalid_at: record_invalid_at,
                },
                flags: InfFlags {
                    projected: *projected,
                    stale: born_stale,
                },
            };
            let inf_index = state.inferential_records.len();
            state.inferential_records.push(inf.clone());
            state
                .inferential_by_sp
                .entry((*s, *p))
                .or_default()
                .push(inf_index);
            out.push(CanonicalRecord::Inf(inf));
            // Register in the reverse-parent index so future
            // supersessions on any parent can emit `StaleParent`
            // edges against this Inferential without scanning the
            // full history.
            for parent in derived_from {
                state
                    .inferentials_by_parent
                    .entry(*parent)
                    .or_default()
                    .push(memory_id);
            }
            // Emit the Supersedes edge *after* the new memory so a
            // log reader sees the memory first, then the edge that
            // refers to it. Mirrors Sem § 5.1 ordering.
            if let Some(target) = supersession {
                emit_supersedes_edge(state, out, memory_id, target)?;
            }
        }
        // Alias / Rename / Retire are filtered out by emit() before
        // reaching here; their canonical form is the SYMBOL_* record
        // from the bind journal.
        ValidatedForm::Alias { .. }
        | ValidatedForm::Rename { .. }
        | ValidatedForm::Retire { .. } => {
            return Err(EmitError::Unsupported {
                form: "symbol-event-form-without-journal",
            })
        }
        ValidatedForm::Correct { .. } => return Err(EmitError::Unsupported { form: "correct" }),
        ValidatedForm::Promote { .. } => return Err(EmitError::Unsupported { form: "promote" }),
        ValidatedForm::Query { .. } => return Err(EmitError::Unsupported { form: "query" }),
        ValidatedForm::Episode {
            action,
            label,
            parent_episode,
            retracts,
        } => {
            // Episode forms emit no canonical record at this layer.
            // `:start` deposits metadata in the batch's pending slot
            // for Store to attach to the CHECKPOINT; `:close` is a
            // no-op under the single-batch-per-Episode model (spec
            // § 3.1 — the compile_batch return implicitly closes).
            // The semantic stage guarantees at most one Episode
            // form per batch, so we can unconditionally overwrite.
            if matches!(action, crate::parse::EpisodeAction::Start) {
                *state.pending_episode = Some(PendingEpisodeMetadata {
                    label: label.clone(),
                    parent_episode: *parent_episode,
                    retracts: retracts.clone(),
                });
            } else {
                // `:close` — still clear any stale pending metadata
                // so the batch signals "no new Episode metadata."
                *state.pending_episode = None;
            }
        }
        ValidatedForm::Flag {
            action,
            memory,
            actor,
        } => {
            let record = crate::canonical::FlagEventRecord {
                memory_id: *memory,
                at: state.now,
                actor_symbol: *actor,
            };
            // Flip the pipeline's pin / auth sets alongside the
            // canonical emission so the read path picks up the new
            // state without needing a round-trip through replay.
            match action {
                crate::parse::FlagAction::Pin => {
                    state.pinned.insert(*memory);
                    out.push(CanonicalRecord::Pin(record));
                }
                crate::parse::FlagAction::Unpin => {
                    state.pinned.remove(memory);
                    out.push(CanonicalRecord::Unpin(record));
                }
                crate::parse::FlagAction::AuthoritativeSet => {
                    state.authoritative.insert(*memory);
                    out.push(CanonicalRecord::AuthoritativeSet(record));
                }
                crate::parse::FlagAction::AuthoritativeClear => {
                    state.authoritative.remove(memory);
                    out.push(CanonicalRecord::AuthoritativeClear(record));
                }
            }
        }
    }
    Ok(())
}

/// Resolve the auto-supersession decision for a new Semantic memory
/// against the current-state index. Per `temporal-model.md` § 5.1:
///
/// - No prior at `(s, p)`: insert, no edge.
/// - New `valid_at > old.valid_at` (forward): replace the index
///   entry; the caller emits a Supersedes edge `new → old`.
/// - New `valid_at < old.valid_at` (retroactive correction): the new
///   memory is valid only for the period up to `old.valid_at`, so
///   its `invalid_at` is set at write time and the index entry for
///   `(s, p)` stays pointed at `old`. A Supersedes edge still records
///   the temporal relationship.
/// - Equal `valid_at`: two memories at the same `(s, p)` claiming
///   identical validity start cannot both be authoritative under the
///   single-writer-per-workspace invariant. Surface as an emit-time
///   error so the agent picks a distinct `valid_at` and re-batches.
fn resolve_semantic_supersession(
    index: &mut SupersessionIndex,
    new_memory_id: SymbolId,
    s: SymbolId,
    p: SymbolId,
    new_valid_at: ClockTime,
) -> Result<(Option<ClockTime>, Option<SymbolId>), EmitError> {
    let key = (s, p);
    let Some(old) = index.semantic_by_sp.get(&key).copied() else {
        index.semantic_by_sp.insert(
            key,
            CurrentSemantic {
                memory_id: new_memory_id,
                valid_at: new_valid_at,
            },
        );
        return Ok((None, None));
    };
    match new_valid_at.cmp(&old.valid_at) {
        std::cmp::Ordering::Greater => {
            // Forward supersession.
            index.semantic_by_sp.insert(
                key,
                CurrentSemantic {
                    memory_id: new_memory_id,
                    valid_at: new_valid_at,
                },
            );
            tracing::info!(
                target: "mimir.supersession",
                kind = "semantic",
                direction = "forward",
                s = %s,
                p = %p,
                old_memory_id = %old.memory_id,
                new_memory_id = %new_memory_id,
                "semantic auto-supersession",
            );
            Ok((None, Some(old.memory_id)))
        }
        std::cmp::Ordering::Less => {
            // Retroactive: new memory's validity closes at old's start.
            // Index entry preserves `old` as current.
            //
            // Note the asymmetry vs forward supersession: § 6.2 #4
            // says `to.invalid_at = from.valid_at` for every
            // Supersedes edge, but § 5.1 retroactive instead sets
            // `from.invalid_at = to.valid_at` (the NEW memory's
            // invalid_at, not the old one's). We follow § 5.1 here;
            // 6.4's as-of resolver must not derive invalid_at from
            // edges alone — it has to read it from the memory record
            // directly.
            tracing::info!(
                target: "mimir.supersession",
                kind = "semantic",
                direction = "retroactive",
                s = %s,
                p = %p,
                old_memory_id = %old.memory_id,
                new_memory_id = %new_memory_id,
                "semantic auto-supersession",
            );
            Ok((Some(old.valid_at), Some(old.memory_id)))
        }
        std::cmp::Ordering::Equal => Err(EmitError::SemanticSupersessionConflict {
            s,
            p,
            valid_at: new_valid_at,
            existing: old.memory_id,
        }),
    }
}

/// Resolve auto-supersession for a new Inferential memory against
/// the current-state index per `temporal-model.md` § 5.4. Mirrors
/// [`resolve_semantic_supersession`]: a re-derivation with the same
/// `(s, p)` and a later `valid_at` forward-supersedes; an earlier
/// `valid_at` is retroactive and closes the *new* memory's validity
/// at the existing record's `valid_at`; equal `valid_at` is a
/// conflict under the single-writer invariant.
fn resolve_inferential_supersession(
    index: &mut SupersessionIndex,
    new_memory_id: SymbolId,
    s: SymbolId,
    p: SymbolId,
    new_valid_at: ClockTime,
) -> Result<(Option<ClockTime>, Option<SymbolId>), EmitError> {
    let key = (s, p);
    let Some(old) = index.inferential_by_sp.get(&key).copied() else {
        index.inferential_by_sp.insert(
            key,
            CurrentSemantic {
                memory_id: new_memory_id,
                valid_at: new_valid_at,
            },
        );
        return Ok((None, None));
    };
    match new_valid_at.cmp(&old.valid_at) {
        std::cmp::Ordering::Greater => {
            index.inferential_by_sp.insert(
                key,
                CurrentSemantic {
                    memory_id: new_memory_id,
                    valid_at: new_valid_at,
                },
            );
            tracing::info!(
                target: "mimir.supersession",
                kind = "inferential",
                direction = "forward",
                s = %s,
                p = %p,
                old_memory_id = %old.memory_id,
                new_memory_id = %new_memory_id,
                "inferential auto-supersession",
            );
            Ok((None, Some(old.memory_id)))
        }
        std::cmp::Ordering::Less => {
            // Retroactive: new memory's validity closes at old's
            // start. Index entry preserves `old` as current.
            tracing::info!(
                target: "mimir.supersession",
                kind = "inferential",
                direction = "retroactive",
                s = %s,
                p = %p,
                old_memory_id = %old.memory_id,
                new_memory_id = %new_memory_id,
                "inferential auto-supersession",
            );
            Ok((Some(old.valid_at), Some(old.memory_id)))
        }
        std::cmp::Ordering::Equal => Err(EmitError::InferentialSupersessionConflict {
            s,
            p,
            valid_at: new_valid_at,
            existing: old.memory_id,
        }),
    }
}

/// Resolve auto-supersession for a new Procedural memory against
/// the current-state index per `temporal-model.md` § 5.2.
///
/// Either a `rule_id` match or a `(trigger, scope)` match triggers
/// supersession. If both keys match distinct existing memories (i.e.
/// the new memory's `rule_id` points to memory A and its
/// `(trigger, scope)` points to memory B ≠ A), BOTH are superseded
/// and the caller emits a `Supersedes` edge for each. The case
/// where both lookups converge to the same memory (a write that
/// duplicates an existing Pro on both keys) dedupes to a single
/// edge.
///
/// Side effects: removes the superseded memories' index entries
/// under both of their keys (once superseded, a memory is no longer
/// current on *any* key) and inserts the new memory under both of
/// its keys.
///
/// # Errors
///
/// [`EmitError::ProceduralSupersessionConflict`] if any matched
/// predecessor shares `committed_at` with the new memory — i.e.,
/// both were emitted in the same batch. This is the Pro analog of
/// Semantic's equal-`valid_at` rejection: silently accepting the
/// edge would produce a zero-duration "supersession" that's almost
/// certainly an agent bug.
fn apply_procedural_supersession(
    index: &mut SupersessionIndex,
    new_memory_id: SymbolId,
    new_committed_at: ClockTime,
    rule_id: SymbolId,
    trigger: &crate::Value,
    scope: SymbolId,
) -> Result<Vec<SymbolId>, EmitError> {
    let (superseded, trigger_scope_key) = procedural_lookup(index, rule_id, trigger, scope);

    // Reject intra-batch conflict before mutating the index — equal
    // committed_at means NEW and OLD were both produced in the same
    // batch's emit pass, which would yield a zero-duration
    // supersession.
    for old in &superseded {
        if old.committed_at == new_committed_at {
            return Err(EmitError::ProceduralSupersessionConflict {
                rule_id,
                existing: old.memory_id,
            });
        }
    }

    procedural_install(
        index,
        &superseded,
        new_memory_id,
        new_committed_at,
        rule_id,
        trigger_scope_key,
    );
    if !superseded.is_empty() {
        tracing::info!(
            target: "mimir.supersession",
            kind = "procedural",
            rule_id = %rule_id,
            new_memory_id = %new_memory_id,
            superseded_count = superseded.len(),
            "procedural auto-supersession",
        );
    }
    Ok(superseded.into_iter().map(|c| c.memory_id).collect())
}

/// Replay entrypoint — applies the same index mutations as the emit
/// path but skips the intra-batch-conflict check. Safe because the
/// emit path rejected any such conflict at original write time, so
/// no log record can contain one.
fn replay_procedural_supersession(
    index: &mut SupersessionIndex,
    new_memory_id: SymbolId,
    new_committed_at: ClockTime,
    rule_id: SymbolId,
    trigger: &crate::Value,
    scope: SymbolId,
) {
    let (superseded, trigger_scope_key) = procedural_lookup(index, rule_id, trigger, scope);
    procedural_install(
        index,
        &superseded,
        new_memory_id,
        new_committed_at,
        rule_id,
        trigger_scope_key,
    );
}

/// Look up the (up to two) prior Procedural memories matched by
/// `rule_id` and `(trigger, scope)`. Returns the canonical
/// `(trigger_bytes, scope)` key alongside the matches so callers can
/// reuse it for insertion without re-encoding.
fn procedural_lookup(
    index: &SupersessionIndex,
    rule_id: SymbolId,
    trigger: &crate::Value,
    scope: SymbolId,
) -> (Vec<CurrentProcedural>, (Vec<u8>, SymbolId)) {
    let trigger_scope_key = (trigger.index_key_bytes(), scope);
    let by_rule = index.procedural_by_rule.get(&rule_id).copied();
    let by_ts = index
        .procedural_by_trigger_scope
        .get(&trigger_scope_key)
        .copied();

    let mut superseded: Vec<CurrentProcedural> = Vec::new();
    if let Some(old) = by_rule {
        superseded.push(old);
    }
    if let Some(old) = by_ts {
        if !superseded
            .iter()
            .any(|existing| existing.memory_id == old.memory_id)
        {
            superseded.push(old);
        }
    }
    (superseded, trigger_scope_key)
}

/// Clear every superseded memory's BOTH keys and insert NEW under
/// both of its keys. Shared by emit + replay.
fn procedural_install(
    index: &mut SupersessionIndex,
    superseded: &[CurrentProcedural],
    new_memory_id: SymbolId,
    new_committed_at: ClockTime,
    rule_id: SymbolId,
    trigger_scope_key: (Vec<u8>, SymbolId),
) {
    for old in superseded {
        if let Some(keys) = index.procedural_keys_by_memory.remove(&old.memory_id) {
            index.procedural_by_rule.remove(&keys.rule_id);
            index
                .procedural_by_trigger_scope
                .remove(&keys.trigger_scope);
        }
    }
    let new_entry = CurrentProcedural {
        memory_id: new_memory_id,
        committed_at: new_committed_at,
    };
    let new_keys = ProceduralKeys {
        rule_id,
        trigger_scope: trigger_scope_key.clone(),
    };
    index.procedural_by_rule.insert(rule_id, new_entry);
    index
        .procedural_by_trigger_scope
        .insert(trigger_scope_key, new_entry);
    index
        .procedural_keys_by_memory
        .insert(new_memory_id, new_keys);
}

/// Push a `Supersedes` edge record into the output stream and add
/// the matching in-memory edge to the working DAG. Also emits a
/// `StaleParent` edge from each Inferential derived from `to` —
/// per `temporal-model.md` § 5.4, a superseded parent invalidates
/// every dependent Inferential at the supersession instant.
fn emit_supersedes_edge(
    state: &mut EmitState,
    out: &mut Vec<CanonicalRecord>,
    from: SymbolId,
    to: SymbolId,
) -> Result<(), EmitError> {
    out.push(CanonicalRecord::Supersedes(EdgeRecord {
        from,
        to,
        at: state.now,
    }));
    state
        .dag
        .add_edge(DagEdge {
            kind: EdgeKind::Supersedes,
            from,
            to,
            at: state.now,
        })
        .map_err(EmitError::SupersessionDag)?;
    // Inferential staling: every Inferential that derived from the
    // now-superseded `to` gets a `StaleParent` edge committed in the
    // same batch. The reverse index is populated at Inf emit + log
    // replay so this lookup is O(log n) + O(k) in the dependent
    // count, typically ≤ 3.
    if let Some(dependents) = state.inferentials_by_parent.get(&to).cloned() {
        for inf_id in dependents {
            emit_stale_parent_edge(state, out, inf_id, to)?;
        }
    }
    Ok(())
}

/// Emit a `StaleParent` edge (`inf → parent`) as part of the
/// Inferential-staling retroactive propagation. Added to both the
/// output stream (for durability) and the working DAG (so further
/// batch-local logic sees it).
fn emit_stale_parent_edge(
    state: &mut EmitState,
    out: &mut Vec<CanonicalRecord>,
    inf_id: SymbolId,
    parent_id: SymbolId,
) -> Result<(), EmitError> {
    out.push(CanonicalRecord::StaleParent(EdgeRecord {
        from: inf_id,
        to: parent_id,
        at: state.now,
    }));
    state
        .dag
        .add_edge(DagEdge {
            kind: EdgeKind::StaleParent,
            from: inf_id,
            to: parent_id,
            at: state.now,
        })
        .map_err(EmitError::SupersessionDag)?;
    Ok(())
}

/// True if `parent` has an incoming `Supersedes` edge in `dag` —
/// i.e. it has been superseded. Used at Inf emit time for the
/// write-time `stale` flag per `temporal-model.md` § 5.4.
fn parent_is_superseded(dag: &SupersessionDag, parent: SymbolId) -> bool {
    dag.edges_to(parent)
        .any(|e| matches!(e.kind, EdgeKind::Supersedes))
}

fn allocate_memory_id(
    state: &mut EmitState,
    out: &mut Vec<CanonicalRecord>,
) -> Result<SymbolId, EmitError> {
    // `__mem_{n}` is the librarian's conventional memory-ID name; the
    // identifier grammar (ir-write-surface.md § 3.1) does not formally
    // reserve the `__` prefix, so a pathological agent could in
    // principle land a collision — if so, bind surfaces it as
    // `EmitError::MemoryIdAllocation` (the collision does not silently
    // overwrite an existing symbol).
    let name = format!("__mem_{}", *state.counter);
    *state.counter += 1;
    let id = state
        .table
        .allocate(name.clone(), SymbolKind::Memory)
        .map_err(|cause| EmitError::MemoryIdAllocation {
            name: name.clone(),
            cause,
        })?;
    // Emit a SymbolAlloc canonical record so the memory symbol is
    // recoverable on log replay (same constraint that applies to the
    // bind-journal-derived SymbolAlloc records).
    out.push(CanonicalRecord::SymbolAlloc(SymbolEventRecord {
        symbol_id: id,
        name,
        symbol_kind: SymbolKind::Memory,
        at: state.now,
    }));
    Ok(id)
}

/// Pipeline-level error — tags the stage at which compilation failed.
/// Every variant means the batch did **not** commit; the workspace
/// state is unchanged.
#[derive(Debug, Error, PartialEq)]
pub enum PipelineError {
    /// Lex/parse stage failure.
    #[error("parse error: {0}")]
    Parse(#[from] ParseError),

    /// Bind stage failure.
    #[error("bind error: {0}")]
    Bind(#[from] BindError),

    /// Semantic stage failure.
    #[error("semantic error: {0}")]
    Semantic(#[from] SemanticError),

    /// Emit stage failure.
    #[error("emit error: {0}")]
    Emit(#[from] EmitError),

    /// The monotonic `committed_at` rule would advance into the
    /// reserved `u64::MAX` sentinel that `ClockTime` refuses to
    /// represent. Per `temporal-model.md` § 9.1, `ClockTime::MAX - 1`
    /// reaches year ≈584 000 000, so this error is effectively a guard
    /// against pathological inputs rather than a real-world condition.
    #[error(
        "committed_at clock exhausted: monotonic advance past {last_committed_at} would hit reserved sentinel"
    )]
    ClockExhausted {
        /// The previous commit clock before the attempted advance.
        last_committed_at: ClockTime,
    },
}

/// Errors produced by the Emit stage. An `EmitError` always means an
/// invariant should have been caught earlier or the form is not yet
/// supported at this milestone.
#[derive(Debug, Error, PartialEq)]
pub enum EmitError {
    /// A form shape is not yet wired to a canonical record. See the
    /// module-level scope notes for which forms emit records in this
    /// milestone.
    #[error("form {form} is not yet emitted by this pipeline milestone")]
    Unsupported {
        /// Form name (`alias`, `rename`, `retire`, `correct`, `promote`, `query`).
        form: &'static str,
    },

    /// Allocating the synthesized `__mem_{n}` symbol failed. The only
    /// realistic cause is a name collision with an agent-emitted symbol
    /// that used the reserved `__mem_` prefix, which would itself be a
    /// prior bug; preserved as a typed variant for diagnosability.
    #[error("memory-id allocation failed for {name}: {cause}")]
    MemoryIdAllocation {
        /// The synthesized name that collided.
        name: String,
        /// Underlying bind error.
        cause: BindError,
    },

    /// Two Semantic writes land at the same `(s, p)` with identical
    /// `valid_at`. Per `temporal-model.md` § 5.1 two memories cannot
    /// both be authoritative at the same conflict key and identical
    /// validity start under the single-writer invariant — surface as
    /// a deterministic emit-time error so the agent can correct the
    /// `valid_at` or choose a different `(s, p)`.
    #[error(
        "semantic supersession conflict at (s={s:?}, p={p:?}) valid_at={valid_at}: new memory has the same valid_at as existing memory {existing:?}"
    )]
    SemanticSupersessionConflict {
        /// Subject of the conflicting write.
        s: SymbolId,
        /// Predicate of the conflicting write.
        p: SymbolId,
        /// Shared `valid_at` that triggered the conflict.
        valid_at: ClockTime,
        /// Memory ID of the existing current-state memory.
        existing: SymbolId,
    },

    /// Two Inferential writes land at the same `(s, p)` with identical
    /// `valid_at`. Per `temporal-model.md` § 5.4 Inferential
    /// supersession mirrors Semantic § 5.1 — equal `valid_at` against
    /// the same `(s, p)` is a deterministic write conflict the agent
    /// must resolve by choosing a distinct `valid_at` or re-keying.
    #[error(
        "inferential supersession conflict at (s={s:?}, p={p:?}) valid_at={valid_at}: new memory has the same valid_at as existing memory {existing:?}"
    )]
    InferentialSupersessionConflict {
        /// Subject of the conflicting write.
        s: SymbolId,
        /// Predicate of the conflicting write.
        p: SymbolId,
        /// Shared `valid_at` that triggered the conflict.
        valid_at: ClockTime,
        /// Memory ID of the existing current-state memory.
        existing: SymbolId,
    },

    /// An auto-supersession edge failed the DAG's acyclicity check.
    /// Cannot happen from auto-supersession alone (new memories have
    /// fresh IDs that cannot appear as ancestors), but the DAG's
    /// contract surfaces any violation rather than silently accepting.
    #[error("supersession DAG rejected edge: {0}")]
    SupersessionDag(#[from] crate::dag::DagError),

    /// Two Procedural writes in the same batch land on overlapping
    /// supersession keys (same `rule_id` or same `(trigger, scope)`).
    /// They would share `committed_at` per § 9.2 monotonic-batch
    /// semantics, producing a zero-duration Supersedes edge. Per the
    /// Semantic analog at § 5.1 this is a deterministic write
    /// conflict rather than a silent accept — the agent should split
    /// the batch or choose distinct keys.
    #[error(
        "procedural supersession conflict: batch contains two Pro writes at the same supersession key (rule_id={rule_id:?}), first bound to {existing:?}"
    )]
    ProceduralSupersessionConflict {
        /// The `rule_id` under conflict.
        rule_id: SymbolId,
        /// The first in-batch Pro memory that occupies the key.
        existing: SymbolId,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::Opcode;

    fn now() -> ClockTime {
        ClockTime::try_from_millis(1_713_350_400_000).expect("non-sentinel")
    }

    const SEM_OK: &str = "(sem @alice @knows @bob :src @observation :c 0.8 :v 2024-01-15)";

    fn memory_records(records: &[CanonicalRecord]) -> Vec<&CanonicalRecord> {
        records
            .iter()
            .filter(|r| {
                matches!(
                    r.opcode(),
                    Opcode::Sem | Opcode::Epi | Opcode::Pro | Opcode::Inf
                )
            })
            .collect()
    }

    #[test]
    fn pathological_agent_collides_with_mem_counter() {
        // A pathological agent mentions `@__mem_0` as a symbol in
        // the same batch that produces a librarian-synthesised
        // `__mem_0` memory-id. `allocate_memory_id` surfaces the
        // collision as `EmitError::MemoryIdAllocation` and rolls
        // back the batch — no silent skip, no overwrite.
        let mut pipe = Pipeline::new();
        let input = "(sem @alice @knows @__mem_0 :src @observation :c 0.8 :v 2024-01-15)";
        let err = pipe
            .compile_batch(input, now())
            .expect_err("__mem_0 collision");
        let PipelineError::Emit(EmitError::MemoryIdAllocation { name, .. }) = &err else {
            panic!("expected MemoryIdAllocation error, got {err:?}");
        };
        assert_eq!(name, "__mem_0");
        // Batch rolled back: neither the agent's @__mem_0 symbol
        // nor the other batch allocations committed.
        assert!(pipe.table.lookup("__mem_0").is_none());
        assert!(pipe.table.lookup("alice").is_none());
        assert_eq!(pipe.next_memory_counter, 0);
    }

    #[test]
    fn single_sem_form_roundtrips_through_pipeline() {
        let mut pipe = Pipeline::new();
        let records = pipe.compile_batch(SEM_OK, now()).expect("compile");
        // SYMBOL_ALLOC records for @alice, @knows, @bob, @observation,
        // and __mem_0 precede the memory record.
        let mems = memory_records(&records);
        assert_eq!(mems.len(), 1);
        assert_eq!(mems[0].opcode(), Opcode::Sem);
        // Every non-memory record in this batch is a SymbolAlloc.
        for r in &records {
            assert!(matches!(r.opcode(), Opcode::Sem | Opcode::SymbolAlloc));
        }
    }

    #[test]
    fn epi_records_are_retained_after_compile() {
        let mut pipe = Pipeline::new();
        let input = "(epi @evt_001 @rename (@old @new) @github \
                     :at 2024-01-15T10:00:00Z :obs 2024-01-15T10:00:05Z \
                     :src @observation :c 0.9)";

        let records = pipe.compile_batch(input, now()).expect("compile");

        let mems = memory_records(&records);
        assert_eq!(mems.len(), 1);
        assert_eq!(mems[0].opcode(), Opcode::Epi);
        assert_eq!(pipe.episodic_records().len(), 1);
        let retained = &pipe.episodic_records()[0];
        assert_eq!(
            retained.event_id,
            pipe.table.lookup("evt_001").expect("evt")
        );
        assert_eq!(retained.kind, pipe.table.lookup("rename").expect("kind"));
        assert_eq!(retained.participants.len(), 2);
    }

    #[test]
    fn multi_form_batch_emits_in_input_order() {
        let mut pipe = Pipeline::new();
        let input = "
            (sem @alice @knows @bob :src @observation :c 0.8 :v 2024-01-15)
            (sem @alice @knows @carol :src @observation :c 0.7 :v 2024-01-16)
        ";
        let records = pipe.compile_batch(input, now()).expect("compile");
        let mems = memory_records(&records);
        assert_eq!(mems.len(), 2);
        for r in &mems {
            assert_eq!(r.opcode(), Opcode::Sem);
        }
    }

    #[test]
    fn empty_input_is_a_no_op_batch() {
        // The semantics choice: `compile_batch("")` parses as zero
        // forms and produces zero records. It does NOT error. No
        // symbols allocated, no memory-id counter advancement, no
        // records emitted. This locks in the "empty is valid" path
        // so later callers (e.g. batched wire transport) don't
        // accidentally introduce a different behavior.
        let mut pipe = Pipeline::new();
        let records = pipe.compile_batch("", now()).expect("empty compiles");
        assert!(records.is_empty());
        assert_eq!(pipe.next_memory_counter, 0);
        // Empty batches DO advance the commit watermark — the batch
        // still ran, and a subsequent batch submitted with the same
        // wall clock must bump past it. `Store::commit_batch` relies
        // on this: it reads `pipeline.last_committed_at()` to stamp
        // the CHECKPOINT + episode alloc, so an empty batch that
        // silently skipped the advance would let the checkpoint
        // regress under a repeated wall clock.
        assert_eq!(pipe.last_committed_at(), Some(now()));
        // Whitespace-only input is equivalent.
        let records = pipe
            .compile_batch("   \n\t  ", now())
            .expect("whitespace compiles");
        assert!(records.is_empty());
        assert_eq!(pipe.next_memory_counter, 0);
        // Second empty batch bumped the watermark by 1 ms.
        assert_eq!(
            pipe.last_committed_at().expect("set").as_millis(),
            now().as_millis() + 1
        );
    }

    #[test]
    fn parse_error_does_not_mutate_table() {
        let mut pipe = Pipeline::new();
        let before_table = pipe.table.clone();
        let err = pipe.compile_batch("(sem @a", now()).expect_err("malformed");
        assert!(matches!(err, PipelineError::Parse(_)));
        assert_eq!(pipe.table, before_table);
        assert_eq!(pipe.next_memory_counter, 0);
    }

    #[test]
    fn bind_error_in_mid_batch_rolls_back_all_prior_allocations() {
        let mut pipe = Pipeline::new();
        // First form allocates @x as Agent (subject slot) and @rel as
        // Predicate. Second form reuses @x in the predicate slot, where
        // it must be Predicate — kind mismatch.
        let input = "
            (sem @x @rel @y :src @observation :c 0.8 :v 2024-01-15)
            (sem @alice @x @z :src @observation :c 0.8 :v 2024-01-16)
        ";
        let err = pipe.compile_batch(input, now()).expect_err("kind mismatch");
        assert!(matches!(err, PipelineError::Bind(_)));
        // Batch rollback: no symbol from this batch committed.
        assert!(pipe.table.lookup("x").is_none());
        assert!(pipe.table.lookup("rel").is_none());
        assert!(pipe.table.lookup("y").is_none());
        assert_eq!(pipe.next_memory_counter, 0);
    }

    #[test]
    fn semantic_error_rolls_back_all_prior_allocations() {
        let mut pipe = Pipeline::new();
        // Registry bound is 0.98; request 0.99 — semantic rejects.
        let input = "(sem @a @knows @b :src @registry :c 0.99 :v 2024-01-15)";
        let err = pipe
            .compile_batch(input, now())
            .expect_err("conf over bound");
        assert!(matches!(err, PipelineError::Semantic(_)));
        assert!(pipe.table.lookup("a").is_none());
        assert!(pipe.table.lookup("b").is_none());
    }

    #[test]
    fn successful_batch_commits_table_and_counter() {
        let mut pipe = Pipeline::new();
        let _ = pipe.compile_batch(SEM_OK, now()).expect("first");
        assert!(pipe.table.lookup("alice").is_some());
        assert_eq!(pipe.next_memory_counter, 1);

        let input2 = "(sem @alice @likes @carol :src @observation :c 0.7 :v 2024-01-16)";
        let _ = pipe.compile_batch(input2, now()).expect("second");
        // Alice from first batch persisted — reused, not reallocated.
        assert_eq!(pipe.next_memory_counter, 2);
    }

    #[test]
    fn successive_calls_produce_distinct_memory_ids() {
        let mut pipe = Pipeline::new();
        let r1 = pipe.compile_batch(SEM_OK, now()).expect("first");
        let r2 = pipe
            .compile_batch(
                "(sem @alice @knows @carol :src @observation :c 0.8 :v 2024-01-16)",
                now(),
            )
            .expect("second");
        let Some(CanonicalRecord::Sem(s1)) = memory_records(&r1).first().copied() else {
            panic!("expected Sem in first batch");
        };
        let Some(CanonicalRecord::Sem(s2)) = memory_records(&r2).first().copied() else {
            panic!("expected Sem in second batch");
        };
        assert_ne!(s1.memory_id, s2.memory_id);
    }

    #[test]
    fn retire_form_emits_symbol_retire_not_error() {
        // A retire form commits as a SymbolRetire canonical record via
        // the bind journal; it does NOT surface a memory-level record
        // and does NOT return `EmitError::Unsupported`.
        let mut pipe = Pipeline::new();
        let _ = pipe.compile_batch(SEM_OK, now()).expect("first");
        let records = pipe
            .compile_batch("(retire @alice)", now())
            .expect("retire supported");
        // Exactly one SymbolRetire; no memory records.
        assert!(records.iter().any(|r| r.opcode() == Opcode::SymbolRetire));
        assert!(memory_records(&records).is_empty());
    }

    #[test]
    fn same_input_produces_byte_identical_records() {
        let input = "
            (sem @alice @knows @bob :src @observation :c 0.8 :v 2024-01-15)
            (sem @alice @knows @carol :src @observation :c 0.7 :v 2024-01-16)
        ";
        let fixed_now = now();
        let mut pipe_a = Pipeline::new();
        let mut pipe_b = Pipeline::new();
        let a = pipe_a.compile_batch(input, fixed_now).expect("a");
        let b = pipe_b.compile_batch(input, fixed_now).expect("b");
        assert_eq!(a, b);
    }

    #[test]
    fn clocks_populated_from_now_parameter() {
        let mut pipe = Pipeline::new();
        // `now` must be >= the form's `:v` date or semantic rejects as
        // future-validity; set both consistently.
        let t = now();
        let records = pipe.compile_batch(SEM_OK, t).expect("compile");
        let Some(CanonicalRecord::Sem(sem)) = memory_records(&records).first().copied() else {
            panic!("expected Sem");
        };
        assert_eq!(sem.clocks.observed_at, t);
        assert_eq!(sem.clocks.committed_at, t);
        assert_eq!(sem.clocks.invalid_at, None);
    }

    #[test]
    fn first_batch_uses_wall_clock_as_committed_at() {
        let mut pipe = Pipeline::new();
        assert_eq!(pipe.last_committed_at(), None);
        let t = now();
        let _ = pipe.compile_batch(SEM_OK, t).expect("compile");
        assert_eq!(pipe.last_committed_at(), Some(t));
    }

    #[test]
    fn monotonic_commit_clock_bumps_past_regressing_wall_clock() {
        // temporal-model.md § 9.2 / § 12 #1: committed_at must be
        // strictly monotonic per workspace even when the host wall
        // clock regresses (NTP correction, VM clock warp, etc.).
        let mut pipe = Pipeline::new();
        let t1 = ClockTime::try_from_millis(1_713_350_400_000).expect("non-sentinel");
        let t_regressed = ClockTime::try_from_millis(1_713_350_300_000).expect("non-sentinel");

        let first = pipe
            .compile_batch(SEM_OK, t1)
            .expect("first batch commits at t1");
        let first_sem = first.iter().find_map(|r| match r {
            CanonicalRecord::Sem(s) => Some(s),
            _ => None,
        });
        assert_eq!(first_sem.expect("sem").clocks.committed_at, t1);

        // Second batch submitted with wall clock that regressed; the
        // librarian must bump committed_at to t1 + 1 ms. Use a
        // distinct predicate so auto-supersession doesn't interfere.
        let second = pipe
            .compile_batch(
                "(sem @alice @likes @carol :src @observation :c 0.8 :v 2024-01-15)",
                t_regressed,
            )
            .expect("second batch");
        let second_sem = second.iter().find_map(|r| match r {
            CanonicalRecord::Sem(s) => Some(s),
            _ => None,
        });
        let expected = ClockTime::try_from_millis(t1.as_millis() + 1).expect("non-sentinel");
        assert_eq!(second_sem.expect("sem").clocks.committed_at, expected);
        assert_eq!(pipe.last_committed_at(), Some(expected));
    }

    #[test]
    fn identical_wall_clock_across_batches_still_bumps_committed_at() {
        let mut pipe = Pipeline::new();
        let t = now();
        let _ = pipe.compile_batch(SEM_OK, t).expect("first");
        // Same `t` on purpose — two batches in the same millisecond.
        // Use a distinct predicate so Semantic auto-supersession
        // doesn't reject this as a (s, p, valid_at) conflict — we're
        // testing the commit-clock bump, not supersession.
        let second = pipe
            .compile_batch(
                "(sem @alice @likes @dave :src @observation :c 0.8 :v 2024-01-15)",
                t,
            )
            .expect("second");
        let second_sem = second.iter().find_map(|r| match r {
            CanonicalRecord::Sem(s) => Some(s),
            _ => None,
        });
        assert_eq!(
            second_sem.expect("sem").clocks.committed_at.as_millis(),
            t.as_millis() + 1
        );
    }

    #[test]
    fn failed_batch_does_not_advance_commit_watermark() {
        let mut pipe = Pipeline::new();
        let t1 = now();
        let _ = pipe.compile_batch(SEM_OK, t1).expect("seed");
        let watermark_before = pipe.last_committed_at();
        // This batch fails in semantic (confidence > registry bound).
        let t2 = ClockTime::try_from_millis(t1.as_millis() + 10_000).expect("non-sentinel");
        let err = pipe
            .compile_batch(
                "(sem @alice @knows @bob :src @registry :c 0.99 :v 2024-01-15)",
                t2,
            )
            .expect_err("semantic reject");
        assert!(matches!(err, PipelineError::Semantic(_)));
        // Watermark must not have advanced — the monotonic commit
        // clock is part of the batch-atomic state per § 11.3.
        assert_eq!(pipe.last_committed_at(), watermark_before);
    }

    #[test]
    fn monotonic_commit_clock_helper_returns_wall_clock_when_unset() {
        let t = now();
        assert_eq!(monotonic_commit_clock(t, None).expect("fresh"), t);
    }

    #[test]
    fn monotonic_commit_clock_helper_bumps_when_wall_clock_not_ahead() {
        let prev = ClockTime::try_from_millis(1_000_000).expect("non-sentinel");
        // Wall clock exactly at prev.
        let at = monotonic_commit_clock(prev, Some(prev)).expect("bump");
        assert_eq!(at.as_millis(), 1_000_001);
        // Wall clock behind prev.
        let behind = ClockTime::try_from_millis(500_000).expect("non-sentinel");
        let at = monotonic_commit_clock(behind, Some(prev)).expect("bump");
        assert_eq!(at.as_millis(), 1_000_001);
    }

    #[test]
    fn clock_exhaustion_returns_typed_error() {
        // `ClockTime::try_from_millis` refuses `u64::MAX` (reserved
        // sentinel), so the bump rule at `MAX - 1` must surface the
        // typed error rather than panic.
        let max_valid = ClockTime::try_from_millis(u64::MAX - 1).expect("non-sentinel");
        let err = monotonic_commit_clock(max_valid, Some(max_valid))
            .expect_err("must exhaust past MAX - 1");
        let PipelineError::ClockExhausted { last_committed_at } = err else {
            panic!("expected ClockExhausted, got {err:?}");
        };
        assert_eq!(last_committed_at, max_valid);
    }

    #[test]
    fn semantic_future_validity_uses_wall_clock_not_monotonic_watermark() {
        // Discriminating scenario: place `valid_at` strictly between
        // the regressed wall clock and the inflated monotonic
        // watermark.
        //
        //   wall_now      < valid_at < watermark = effective_now
        //
        // Under the wall-clock choice (the spec's intent per § 9.3 —
        // "is this future relative to the librarian's estimate of
        // current time?"), semantic must reject with `FutureValidity`.
        // Under an (incorrect) monotonic-watermark choice, the check
        // would pass because valid_at < watermark. This test fails if
        // a future refactor silently swaps to the watermark.
        let mut pipe = Pipeline::new();
        // Inflate the watermark to 2024-01-15T00:10:00Z.
        let seed_wall = ClockTime::try_from_millis(1_705_277_400_000).expect("non-sentinel");
        let _ = pipe
            .compile_batch(
                "(sem @seed_a @seed_r @seed_b :src @observation :c 0.8 :v 2024-01-14)",
                seed_wall,
            )
            .expect("seed");
        let watermark = pipe.last_committed_at().expect("set");

        // Regress wall clock to 2024-01-15T00:00:00Z.
        let regressed_wall = ClockTime::try_from_millis(1_705_276_800_000).expect("non-sentinel");
        assert!(regressed_wall < watermark);

        // valid_at = 2024-01-15T00:05:00Z — future relative to
        // regressed_wall but past relative to watermark. Wall-clock
        // semantic must reject.
        let err = pipe
            .compile_batch(
                "(sem @alice @knows @emma :src @observation :c 0.8 :v 2024-01-15T00:05:00Z)",
                regressed_wall,
            )
            .expect_err("must reject future valid_at under regressed wall clock");
        assert!(
            matches!(
                err,
                PipelineError::Semantic(SemanticError::FutureValidity { .. })
            ),
            "expected FutureValidity, got {err:?}"
        );

        // Sanity: the same form with a past valid_at must still be
        // accepted, and committed_at must advance past the watermark.
        let records = pipe
            .compile_batch(
                "(sem @alice @knows @emma :src @observation :c 0.8 :v 2024-01-14)",
                regressed_wall,
            )
            .expect("past valid_at under regressed wall clock must succeed");
        let sem = records.iter().find_map(|r| match r {
            CanonicalRecord::Sem(s) => Some(s),
            _ => None,
        });
        assert!(sem.expect("sem").clocks.committed_at > watermark);
    }

    // ----------------------------------------------------------------
    // 6.3a — Semantic auto-supersession
    // ----------------------------------------------------------------

    fn sem_records(records: &[CanonicalRecord]) -> Vec<&SemRecord> {
        records
            .iter()
            .filter_map(|r| match r {
                CanonicalRecord::Sem(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    fn supersedes_edges(records: &[CanonicalRecord]) -> Vec<&EdgeRecord> {
        records
            .iter()
            .filter_map(|r| match r {
                CanonicalRecord::Supersedes(e) => Some(e),
                _ => None,
            })
            .collect()
    }

    fn stale_parent_edges(records: &[CanonicalRecord]) -> Vec<&EdgeRecord> {
        records
            .iter()
            .filter_map(|r| match r {
                CanonicalRecord::StaleParent(e) => Some(e),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn inf_against_current_parent_is_not_born_stale() {
        let mut pipe = Pipeline::new();
        pipe.compile_batch(SEM_OK, now()).expect("sem");
        let inf_src = "(inf @alice @likes @coffee (@__mem_0) @majority_vote :c 0.7 :v 2024-01-15)";
        let records = pipe
            .compile_batch(inf_src, later_now())
            .expect("inferential");
        let inf = records
            .iter()
            .find_map(|r| match r {
                CanonicalRecord::Inf(i) => Some(i),
                _ => None,
            })
            .expect("inf emitted");
        assert!(!inf.flags.stale, "parent still current — not born stale");
    }

    #[test]
    fn supersession_emits_stale_parent_edges_to_dependent_inferentials() {
        let mut pipe = Pipeline::new();
        pipe.compile_batch(SEM_OK, now()).expect("first sem");
        // Inferential derived from the first sem (__mem_0).
        pipe.compile_batch(
            "(inf @alice @likes @coffee (@__mem_0) @majority_vote :c 0.7 :v 2024-01-15)",
            later_now(),
        )
        .expect("inf");
        // Supersede the first Sem with a newer one at same (s, p).
        let records = pipe
            .compile_batch(
                "(sem @alice @knows @carol :src @observation :c 0.8 :v 2024-02-01)",
                even_later_now(),
            )
            .expect("supersede");
        let stales = stale_parent_edges(&records);
        assert_eq!(
            stales.len(),
            1,
            "one dependent Inferential should receive a StaleParent edge"
        );
        let inf_id = pipe.table().lookup("__mem_1").expect("inf id");
        let old_parent = pipe.table().lookup("__mem_0").expect("parent id");
        assert_eq!(stales[0].from, inf_id);
        assert_eq!(stales[0].to, old_parent);
    }

    #[test]
    fn inf_born_stale_when_parent_already_superseded() {
        let mut pipe = Pipeline::new();
        pipe.compile_batch(SEM_OK, now()).expect("first sem");
        pipe.compile_batch(
            "(sem @alice @knows @carol :src @observation :c 0.8 :v 2024-02-01)",
            later_now(),
        )
        .expect("supersede — __mem_0 is now superseded");
        // Now create an Inf against the already-superseded parent.
        let records = pipe
            .compile_batch(
                "(inf @alice @likes @coffee (@__mem_0) @majority_vote :c 0.7 :v 2024-01-15)",
                even_later_now(),
            )
            .expect("inf");
        let inf = records
            .iter()
            .find_map(|r| match r {
                CanonicalRecord::Inf(i) => Some(i),
                _ => None,
            })
            .expect("inf");
        assert!(
            inf.flags.stale,
            "Inferential born from already-superseded parent must carry stale=true"
        );
    }

    fn later_now() -> ClockTime {
        ClockTime::try_from_millis(1_713_350_400_000 + 1_000).expect("non-sentinel")
    }

    fn even_later_now() -> ClockTime {
        ClockTime::try_from_millis(1_713_350_400_000 + 2_000).expect("non-sentinel")
    }

    #[test]
    fn sem_with_fresh_sp_does_not_emit_supersedes_edge() {
        let mut pipe = Pipeline::new();
        let records = pipe.compile_batch(SEM_OK, now()).expect("first");
        assert!(
            supersedes_edges(&records).is_empty(),
            "first write at (s, p) has nothing to supersede"
        );
        assert_eq!(pipe.dag().len(), 0);
    }

    #[test]
    fn forward_sem_emits_supersedes_edge_and_updates_index() {
        // First write `@alice @knows @bob` at 2024-01-15; second at
        // 2024-03-01 with same (s, p) supersedes forward. Both
        // valid_ats are before `now()` (2024-04-17) so semantic
        // future-validity doesn't reject.
        let mut pipe = Pipeline::new();
        let first = pipe.compile_batch(SEM_OK, now()).expect("first");
        let first_mem = sem_records(&first)[0].memory_id;

        let second_input = "(sem @alice @knows @carol :src @observation :c 0.8 :v 2024-03-01)";
        let second = pipe.compile_batch(second_input, now()).expect("second");
        let sems = sem_records(&second);
        assert_eq!(sems.len(), 1);
        let second_mem = sems[0].memory_id;
        // Forward supersession: new's invalid_at stays None.
        assert_eq!(sems[0].clocks.invalid_at, None);

        let edges = supersedes_edges(&second);
        assert_eq!(edges.len(), 1, "exactly one Supersedes edge");
        assert_eq!(edges[0].from, second_mem);
        assert_eq!(edges[0].to, first_mem);
        // DAG mirrors the edge.
        assert_eq!(pipe.dag().len(), 1);
    }

    #[test]
    fn retroactive_sem_sets_invalid_at_and_preserves_existing_as_current() {
        // First write with valid_at 2024-03-01. Second with valid_at
        // 2024-01-15 (earlier) is a retroactive correction: it's
        // valid only up to 2024-03-01 (`new.invalid_at = old.valid_at`),
        // and the index keeps pointing to the newer-valid_at memory.
        let mut pipe = Pipeline::new();
        let first_input = "(sem @alice @knows @bob :src @observation :c 0.8 :v 2024-03-01)";
        let first = pipe.compile_batch(first_input, now()).expect("first");
        let first_mem = sem_records(&first)[0].memory_id;
        let first_valid_at = sem_records(&first)[0].clocks.valid_at;

        let retro_input = "(sem @alice @knows @zoe :src @observation :c 0.8 :v 2024-01-15)";
        let retro = pipe.compile_batch(retro_input, now()).expect("retro");
        let sems = sem_records(&retro);
        let retro_mem = sems[0].memory_id;
        assert_eq!(
            sems[0].clocks.invalid_at,
            Some(first_valid_at),
            "retroactive new memory's invalid_at closes at old's valid_at"
        );

        let edges = supersedes_edges(&retro);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, retro_mem);
        assert_eq!(edges[0].to, first_mem);

        // A third forward write at 2024-04-01 must still supersede
        // `first` (not `retro`) — the index preserved `first` as
        // current after the retroactive insert.
        let third_input = "(sem @alice @knows @dan :src @observation :c 0.8 :v 2024-04-01)";
        let third = pipe.compile_batch(third_input, now()).expect("third");
        let third_mem = sem_records(&third)[0].memory_id;
        let third_edges = supersedes_edges(&third);
        assert_eq!(third_edges.len(), 1);
        assert_eq!(third_edges[0].from, third_mem);
        assert_eq!(
            third_edges[0].to, first_mem,
            "forward supersession targets the most-recent-valid_at memory, not the retroactive one"
        );
    }

    #[test]
    fn equal_valid_at_at_same_sp_returns_supersession_conflict() {
        let mut pipe = Pipeline::new();
        let _ = pipe.compile_batch(SEM_OK, now()).expect("first");
        // Same (@alice, @knows) and same valid_at 2024-01-15 — single
        // writer per workspace, so this is a deterministic error.
        let err = pipe
            .compile_batch(SEM_OK, now())
            .expect_err("equal valid_at conflicts");
        assert!(
            matches!(
                err,
                PipelineError::Emit(EmitError::SemanticSupersessionConflict { .. })
            ),
            "expected SemanticSupersessionConflict, got {err:?}"
        );
    }

    #[test]
    fn disjoint_sp_pairs_do_not_supersede_each_other() {
        let mut pipe = Pipeline::new();
        let _ = pipe.compile_batch(SEM_OK, now()).expect("first");
        // Different predicate — no supersession.
        let other = pipe
            .compile_batch(
                "(sem @alice @likes @bob :src @observation :c 0.8 :v 2024-01-15)",
                now(),
            )
            .expect("disjoint");
        assert!(supersedes_edges(&other).is_empty());
        assert_eq!(pipe.dag().len(), 0);
    }

    #[test]
    fn forward_chain_produces_edge_per_link() {
        let mut pipe = Pipeline::new();
        let vs = ["2024-01-15", "2024-02-15", "2024-03-15", "2024-04-15"];
        for v in vs {
            let input = format!("(sem @alice @knows @bob :src @observation :c 0.8 :v {v})");
            let _ = pipe.compile_batch(&input, now()).expect("compile");
        }
        // Three supersession events across four writes.
        assert_eq!(pipe.dag().len(), 3);
    }

    #[test]
    fn failed_batch_does_not_leak_edge_or_index_mutation() {
        // Discriminating test for the 6.2 clone-on-write contract as
        // applied to 6.3a's DAG + index mutations: form 1 passes all
        // stages and mutates both `working_dag` (edge) and
        // `working_index` (new (s, p) entry); form 2 passes parse +
        // bind + semantic but fails at EMIT because it lands on the
        // same (s, p, valid_at) that form 1 just wrote into
        // `working_index`. Per batch atomicity, BOTH mutations must
        // be dropped — neither the edge nor the index update can
        // survive into `self`.
        //
        // Without the DAG + index clones in `compile_batch`, form
        // 1's edge and index entry would leak past form 2's failure.
        let mut pipe = Pipeline::new();
        let _ = pipe.compile_batch(SEM_OK, now()).expect("seed");
        assert_eq!(pipe.dag().len(), 0, "seed did not supersede");

        // Both forms pass semantic (confidence ≤ bound, valid_at is
        // past). Form 1 forward-supersedes the seed; form 2 hits the
        // same (s, p, valid_at) form 1 just wrote into working_index.
        let two_forms = "\
            (sem @alice @knows @carol :src @observation :c 0.8 :v 2024-03-01)\n\
            (sem @alice @knows @dan :src @observation :c 0.7 :v 2024-03-01)";
        let err = pipe
            .compile_batch(two_forms, now())
            .expect_err("emit conflict");
        assert!(
            matches!(
                err,
                PipelineError::Emit(EmitError::SemanticSupersessionConflict { .. })
            ),
            "expected SemanticSupersessionConflict from form 2, got {err:?}"
        );

        // Form 1's edge did NOT land in self.dag.
        assert_eq!(pipe.dag().len(), 0, "failed batch must not leak edge");

        // Form 1's index update did NOT land either — a fresh
        // follow-up commit at (alice, knows, 2024-03-01) must target
        // the SEED as its predecessor (not carol, which form 1 would
        // have made current).
        let clean = pipe
            .compile_batch(
                "(sem @alice @knows @eve :src @observation :c 0.8 :v 2024-03-01)",
                now(),
            )
            .expect("clean follow-up");
        assert_eq!(pipe.dag().len(), 1);
        let seed_memory = pipe.table.lookup("__mem_0").expect("seed mem alloc");
        assert_eq!(
            pipe.dag().edges()[0].to,
            seed_memory,
            "post-rollback commit still sees SEED as predecessor"
        );
        assert_eq!(sem_records(&clean)[0].memory_id, pipe.dag().edges()[0].from);
    }

    // ----------------------------------------------------------------
    // 6.3b — Procedural auto-supersession
    // ----------------------------------------------------------------

    const PRO_OK: &str = r#"(pro @rule_route "agent_write" "route_via_librarian"
        :scp @mimir :src @policy :c 1.0)"#;

    fn pro_records(records: &[CanonicalRecord]) -> Vec<&ProRecord> {
        records
            .iter()
            .filter_map(|r| match r {
                CanonicalRecord::Pro(p) => Some(p),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn pro_fresh_rule_does_not_supersede() {
        let mut pipe = Pipeline::new();
        let records = pipe.compile_batch(PRO_OK, now()).expect("first pro");
        assert_eq!(pro_records(&records).len(), 1);
        assert!(supersedes_edges(&records).is_empty());
        assert_eq!(pipe.dag().len(), 0);
    }

    #[test]
    fn pro_same_rule_id_triggers_supersession() {
        let mut pipe = Pipeline::new();
        let first = pipe.compile_batch(PRO_OK, now()).expect("first");
        let first_mem = pro_records(&first)[0].memory_id;

        let second_input = r#"(pro @rule_route "other_trigger" "other_action"
            :scp @other_scope :src @policy :c 0.9)"#;
        let second = pipe.compile_batch(second_input, now()).expect("second");
        let second_mem = pro_records(&second)[0].memory_id;

        let edges = supersedes_edges(&second);
        assert_eq!(edges.len(), 1, "same rule_id → one Supersedes edge");
        assert_eq!(edges[0].from, second_mem);
        assert_eq!(edges[0].to, first_mem);
        assert_eq!(pipe.dag().len(), 1);
    }

    #[test]
    fn pro_same_trigger_scope_triggers_supersession() {
        let mut pipe = Pipeline::new();
        let first = pipe.compile_batch(PRO_OK, now()).expect("first");
        let first_mem = pro_records(&first)[0].memory_id;

        // Different rule_id but identical trigger + scope ("agent_write" + @mimir) — § 5.2 secondary key.
        let second_input = r#"(pro @rule_other "agent_write" "different_action"
            :scp @mimir :src @policy :c 0.9)"#;
        let second = pipe.compile_batch(second_input, now()).expect("second");
        let second_mem = pro_records(&second)[0].memory_id;

        let edges = supersedes_edges(&second);
        assert_eq!(edges.len(), 1, "same (trigger, scope) → one edge");
        assert_eq!(edges[0].from, second_mem);
        assert_eq!(edges[0].to, first_mem);
    }

    #[test]
    fn pro_dual_key_match_supersedes_both_distinct_olds() {
        // OLD1: rule @rule_a, trigger "t_a", scope @scope_a
        // OLD2: rule @rule_b, trigger "t_b", scope @scope_b
        // NEW:  rule @rule_a, trigger "t_b", scope @scope_b
        // NEW matches OLD1 by rule_id AND OLD2 by (trigger, scope) — both must be superseded.
        let mut pipe = Pipeline::new();
        let old1 = pipe
            .compile_batch(
                r#"(pro @rule_a "t_a" "act_a" :scp @scope_a :src @policy :c 1.0)"#,
                now(),
            )
            .expect("old1");
        let old1_mem = pro_records(&old1)[0].memory_id;
        let old2 = pipe
            .compile_batch(
                r#"(pro @rule_b "t_b" "act_b" :scp @scope_b :src @policy :c 1.0)"#,
                now(),
            )
            .expect("old2");
        let old2_mem = pro_records(&old2)[0].memory_id;

        let new_input = r#"(pro @rule_a "t_b" "act_new" :scp @scope_b :src @policy :c 1.0)"#;
        let new = pipe.compile_batch(new_input, now()).expect("new");
        let new_mem = pro_records(&new)[0].memory_id;

        let edges = supersedes_edges(&new);
        assert_eq!(edges.len(), 2, "dual-key match → two Supersedes edges");
        let targets: std::collections::BTreeSet<_> = edges.iter().map(|e| e.to).collect();
        assert!(targets.contains(&old1_mem));
        assert!(targets.contains(&old2_mem));
        for e in &edges {
            assert_eq!(e.from, new_mem);
        }
    }

    #[test]
    fn pro_duplicate_cross_batch_commit_emits_one_edge() {
        // Commit the exact same Pro in two successive batches. OLD
        // sits at both indices (rule_id + (trigger, scope)); NEW's
        // two lookups converge to OLD. The dedup branch collapses
        // them to a single Supersedes edge — the only observable
        // shape of "both keys converge to same old," since non-dup
        // Pros are inserted under both their own keys and a second
        // Pro can only converge both lookups onto a single
        // predecessor by duplicating all of its keys.
        let mut pipe = Pipeline::new();
        let _ = pipe.compile_batch(PRO_OK, now()).expect("seed");
        let second = pipe.compile_batch(PRO_OK, now()).expect("same again");
        let edges = supersedes_edges(&second);
        assert_eq!(edges.len(), 1, "same memory matched twice → one edge");
    }

    #[test]
    fn pro_intra_batch_same_rule_id_is_rejected() {
        // Two Pro forms in the same batch with identical rule_id
        // would share `committed_at`, producing a zero-duration
        // supersession. Per the Semantic analog (equal valid_at →
        // SemanticSupersessionConflict), v1 surfaces this
        // deterministically rather than silently accepting.
        let mut pipe = Pipeline::new();
        let two_forms = r#"
            (pro @rule_a "t_a" "act_a" :scp @scope_a :src @policy :c 1.0)
            (pro @rule_a "t_b" "act_b" :scp @scope_b :src @policy :c 1.0)
        "#;
        let err = pipe
            .compile_batch(two_forms, now())
            .expect_err("intra-batch rule_id conflict");
        assert!(
            matches!(
                err,
                PipelineError::Emit(EmitError::ProceduralSupersessionConflict { .. })
            ),
            "expected ProceduralSupersessionConflict, got {err:?}"
        );
        // Atomicity — form 1's index + DAG mutations must have rolled back.
        assert_eq!(pipe.dag().len(), 0);
    }

    #[test]
    fn pro_intra_batch_same_trigger_scope_is_rejected() {
        // Same sanity check for the secondary key — two Pros in one
        // batch sharing (trigger, scope) but with distinct rule_ids.
        let mut pipe = Pipeline::new();
        let two_forms = r#"
            (pro @rule_a "shared_t" "act_a" :scp @shared_scope :src @policy :c 1.0)
            (pro @rule_b "shared_t" "act_b" :scp @shared_scope :src @policy :c 1.0)
        "#;
        let err = pipe
            .compile_batch(two_forms, now())
            .expect_err("intra-batch (trigger, scope) conflict");
        assert!(matches!(
            err,
            PipelineError::Emit(EmitError::ProceduralSupersessionConflict { .. })
        ));
        assert_eq!(pipe.dag().len(), 0);
    }

    #[test]
    fn pro_supersession_clears_old_from_both_keys() {
        // After NEW supersedes OLD via rule_id match, OLD must no
        // longer be reachable via its OTHER key either.
        let mut pipe = Pipeline::new();
        let old = pipe
            .compile_batch(
                r#"(pro @rule_a "t_a" "act_a" :scp @scope_a :src @policy :c 1.0)"#,
                now(),
            )
            .expect("old");
        let old_mem = pro_records(&old)[0].memory_id;

        // Supersede by rule_id.
        let _ = pipe
            .compile_batch(
                r#"(pro @rule_a "different" "new_act" :scp @different :src @policy :c 1.0)"#,
                now(),
            )
            .expect("super by rule");

        // A third write that matches OLD's (trigger, scope) only.
        // OLD was cleared from both indices when superseded, so no
        // edge from this write should point to OLD.
        let third = pipe
            .compile_batch(
                r#"(pro @rule_fresh "t_a" "act_x" :scp @scope_a :src @policy :c 1.0)"#,
                now(),
            )
            .expect("third");
        for edge in supersedes_edges(&third) {
            assert_ne!(
                edge.to, old_mem,
                "already-superseded OLD must not be superseded again"
            );
        }
    }
}
