//! Hot-path read API per `read-protocol.md`.
//!
//! The entry point is [`Pipeline::execute_query`]. It parses a single
//! `(query ...)` form, resolves the keyword predicates against the
//! pipeline's symbol table (without mutation), and projects over the
//! pipeline's record history and supersession DAG via the as-of
//! resolver (6.4).
//!
//! Scope (7.2 — filters, flags, framing, filtered array):
//!
//! | Predicate             | Status |
//! |-----------------------|---|
//! | `:kind`               | `sem`, `pro` supported; `epi`, `inf` return empty |
//! | `:s @S`, `:p @P`      | supported (Semantic only) |
//! | `:as_of T`            | supported (delegates to resolver § 7.2) |
//! | `:as_committed T`     | supported (§ 7.3) |
//! | `:limit N`            | supported; sets `TRUNCATED` flag when hit |
//! | `:include_retired`    | default `false`; retired-symbol records drop unless set |
//! | `:include_projected`  | default `false`; projected records drop unless set |
//! | `:confidence_threshold` | default 0.5; flag-only threshold (does not filter) |
//! | `:explain_filtered`   | default `false`; when true, dropped records surface in `filtered` |
//! | `:show_framing`       | default `false`; when true, `framings` parallels `records` |
//! | `:debug_mode`         | shorthand for `:explain_filtered` + `:show_framing` |
//! | everything else       | [`ReadError::UnsupportedPredicate`] |
//!
//! Flag surface (§ 6):
//! - `STALE_SYMBOL` — any kept record references a retired symbol.
//! - `LOW_CONFIDENCE` — any kept record's effective (decay-adjusted)
//!   confidence < threshold. Computed via
//!   [`crate::decay::effective_confidence`] against the pipeline's
//!   current [`crate::decay::DecayConfig`].
//! - `PROJECTED_PRESENT` — any kept record has `flags.projected`.
//! - `TRUNCATED` — `:limit` was hit.
//! - `EXPLAIN_FILTERED_ACTIVE` — `:explain_filtered` (or `:debug_mode`) active.
//!
//! `Framing` values at 7.2: `Advisory` (default), `Historical`
//! (`:as_of` predates `query_committed_at`), `Projected` (record's
//! `flags.projected` bit set). `Authoritative` lands in Step 4 when
//! pin / authoritative write forms are wired.

use thiserror::Error;

use crate::bind::SymbolTable;
use crate::canonical::{CanonicalRecord, EpiRecord, InfRecord, ProRecord, SemRecord};
use crate::clock::ClockTime;
use crate::confidence::Confidence;
use crate::decay::{effective_confidence, DecayFlags};
use crate::memory_kind::MemoryKindTag;
use crate::parse::{self, ParseError, RawSymbolName, RawValue, UnboundForm};
use crate::pipeline::Pipeline;
use crate::resolver::{self, TemporalQuery};
use crate::semantic::source_kind_from_name;
use crate::source_kind::SourceKind;
use crate::symbol::SymbolId;
use crate::value::Value;

/// Result of a read-path query per `read-protocol.md` § 5.
#[derive(Clone, Debug, PartialEq)]
pub struct ReadResult {
    /// Matched canonical records — mixed kinds when `:kind` is
    /// unspecified, single-kind otherwise. Order: by `committed_at`
    /// ascending.
    pub records: Vec<CanonicalRecord>,
    /// Per-record framing, parallel to `records`. Empty unless
    /// `:show_framing true` (or `:debug_mode true`). When populated,
    /// `framings.len() == records.len()`.
    pub framings: Vec<Framing>,
    /// Records that were dropped by a filter, surfaced when
    /// `:explain_filtered true` (or `:debug_mode true`). Empty
    /// otherwise — the default silent-filter UX per spec § 11.1.
    pub filtered: Vec<FilteredMemory>,
    /// Flag bitset; see [`ReadFlags`] constants.
    pub flags: ReadFlags,
    /// Effective `as_of` used for this query (the pipeline's latest
    /// commit if the predicate was absent).
    pub as_of: ClockTime,
    /// Effective `as_committed`.
    pub as_committed: ClockTime,
    /// Snapshot boundary — pipeline's latest committed clock at
    /// query start. Per `read-protocol.md` § 9.
    pub query_committed_at: ClockTime,
}

/// Framing classification per `read-protocol.md` § 5. Attached
/// per-record in [`ReadResult::framings`] when `:show_framing true`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Framing {
    /// Normal current-state record.
    Advisory,
    /// Returned because `:as_of` predates `query_committed_at`.
    Historical,
    /// Record has `flags.projected` — an intent / plan, not current
    /// truth. Only reachable when `:include_projected true`.
    Projected,
    /// Record is pinned or operator-authoritative — decay suspended,
    /// high trust. See `confidence-decay.md` §§ 7 / 8.
    Authoritative {
        /// Which flag source authorised the pin.
        set_by: FramingSource,
    },
}

/// Who authorised an authoritative framing. Spec § 5 /
/// `confidence-decay.md` § 7 / § 8.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FramingSource {
    /// Agent-invokable `(pin @mem)` flag (spec § 7).
    AgentPinned,
    /// User-applied `(authoritative_set @mem)` flag (spec § 8).
    OperatorAuthoritative,
}

/// Why a record was filtered out of [`ReadResult::records`]. Exposed
/// via [`ReadResult::filtered`] when `:explain_filtered true`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FilterReason {
    /// The record references at least one retired symbol and
    /// `:include_retired` was not set.
    RetiredSymbolExcluded,
    /// The record has `flags.projected` and `:include_projected`
    /// was not set.
    ProjectedExcluded,
}

/// A record that was dropped from the result set by a filter.
#[derive(Clone, Debug, PartialEq)]
pub struct FilteredMemory {
    /// The record that was filtered. Agents may still inspect its
    /// full shape.
    pub record: CanonicalRecord,
    /// Reason for the drop.
    pub reason: FilterReason,
}

/// Read-result flag bitset per `read-protocol.md` § 5 / § 6.
///
/// Bits reserved but not issued in v1 (part of the stable on-wire
/// layout, always clear): bit 1 (was `CONFLICT`), bit 5 (was
/// `CROSS_WORKSPACE`), bit 6 (was `CONTESTED`). See spec note.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadFlags(u32);

impl ReadFlags {
    /// At least one kept record references a retired symbol.
    pub const STALE_SYMBOL: u32 = 1 << 0;
    // bit 1 reserved — previously CONFLICT (SSI); single writer never sets it.
    /// At least one kept record's confidence is below the query's
    /// `:confidence_threshold` (default 0.5).
    pub const LOW_CONFIDENCE: u32 = 1 << 2;
    /// At least one kept record has `flags.projected`.
    pub const PROJECTED_PRESENT: u32 = 1 << 3;
    /// The result was capped by `:limit` before exhausting all matches.
    pub const TRUNCATED: u32 = 1 << 4;
    // bit 5 reserved — previously CROSS_WORKSPACE; out of scope.
    // bit 6 reserved — previously CONTESTED; out of scope.
    /// `:explain_filtered` is active for this query; `filtered` may
    /// be non-empty.
    pub const EXPLAIN_FILTERED_ACTIVE: u32 = 1 << 7;

    /// Construct an empty flag set.
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// `true` if any of `bits` is set.
    #[must_use]
    pub const fn contains(self, bits: u32) -> bool {
        self.0 & bits != 0
    }

    /// Set `bits`, returning the new flags.
    #[must_use]
    pub const fn with(self, bits: u32) -> Self {
        Self(self.0 | bits)
    }

    /// Raw u32 view — for interop with e.g. wire serialization.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }
}

/// Memory kind filter used by `:kind`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum KindFilter {
    /// Semantic memory.
    Sem,
    /// Procedural memory.
    Pro,
    /// Episodic memory (no Episodic resolver yet — queries return empty).
    Epi,
    /// Inferential memory. Resolver wired in Phase 3.1 per
    /// `temporal-model.md` § 5.4; keyed by `(s, p)` with
    /// re-derivation-based auto-supersession mirroring Sem § 5.1.
    Inf,
}

/// Read-path error family. Every variant means the query did not
/// execute; no state was observed or mutated.
#[derive(Debug, Error, PartialEq)]
pub enum ReadError {
    /// Parse failure on the query input.
    #[error("parse error: {0}")]
    Parse(#[from] ParseError),

    /// Input was not a single `(query ...)` form — the read API
    /// expects exactly one query per call.
    #[error("expected a single (query ...) form, got {count} forms")]
    NotASingleQuery {
        /// Number of forms parsed.
        count: usize,
    },

    /// Input parsed as something other than a `(query ...)` form.
    #[error("input is not a query form")]
    NotAQuery,

    /// Parsed but includes a predicate whose behavior hasn't been
    /// wired yet. 7.1 scope note: see the module docstring.
    #[error("query predicate {predicate} is not supported in this milestone")]
    UnsupportedPredicate {
        /// Predicate keyword (e.g. `"in_episode"`).
        predicate: &'static str,
    },

    /// A predicate's value had the wrong shape for its keyword
    /// (e.g. `:limit @not_a_number`).
    #[error("invalid value for {keyword}: {reason}")]
    InvalidPredicate {
        /// Predicate keyword.
        keyword: &'static str,
        /// Human-readable reason.
        reason: String,
    },

    /// `:kind` was provided with a value that isn't one of the four
    /// canonical kinds.
    #[error("invalid kind {got}: expected one of sem, pro, epi, inf")]
    InvalidKind {
        /// The literal bareword supplied.
        got: String,
    },

    /// A predicate was combined with a `:kind` value it doesn't
    /// apply to — e.g. `:s @alice :kind pro`. Per
    /// `read-protocol.md` § 4.1 the `:s` / `:p` predicates are
    /// SEM / INF only; silently returning no records on this input
    /// would violate spec invariant § 13 #9 "no silent information
    /// loss," so v1 rejects with a typed error.
    #[error(
        "predicate {predicate} is not compatible with :kind {kind:?} (it applies to SEM / INF only)"
    )]
    IncompatiblePredicates {
        /// The offending predicate keyword.
        predicate: &'static str,
        /// The `:kind` value it was combined with.
        kind: KindFilter,
    },
}

impl Pipeline {
    /// Execute a single `(query ...)` form against this pipeline.
    ///
    /// Does NOT mutate pipeline state: no symbol allocations, no
    /// clock advances, no log writes. Symbol references in the
    /// query that do not resolve against the pipeline's current
    /// symbol table produce no matches (a nonexistent symbol is
    /// treated as "no memories at this key") rather than an error.
    ///
    /// # Errors
    ///
    /// See [`ReadError`] variants.
    pub fn execute_query(&self, input: &str) -> Result<ReadResult, ReadError> {
        let forms = parse::parse(input)?;
        let count = forms.len();
        let Some(form) = forms.into_iter().next().filter(|_| count == 1) else {
            return Err(ReadError::NotASingleQuery { count });
        };
        let UnboundForm::Query { selector, keywords } = form else {
            return Err(ReadError::NotAQuery);
        };
        // 7.1 rejects a positional selector — spec § 4 doesn't
        // define selector semantics concretely yet.
        if selector.is_some() {
            return Err(ReadError::UnsupportedPredicate {
                predicate: "selector",
            });
        }
        execute(self, keywords)
    }
}

fn execute(pipeline: &Pipeline, keywords: parse::KeywordArgs) -> Result<ReadResult, ReadError> {
    let predicates = parse_predicates(pipeline, keywords)?;

    // Snapshot boundary — read-protocol.md § 9 snapshot isolation.
    // An empty pipeline has no commits yet; by § 9 convention every
    // clock defaults to that watermark, so there's nothing to match.
    let Some(query_committed_at) = pipeline.last_committed_at() else {
        return Ok(empty_result(&predicates));
    };

    let effective_as_of = predicates.as_of.unwrap_or(query_committed_at);
    let effective_as_committed = predicates.as_committed.unwrap_or(query_committed_at);
    let temporal = TemporalQuery::bi_temporal(effective_as_of, effective_as_committed);

    check_predicate_compatibility(&predicates)?;

    let candidates = collect_candidates(pipeline, &predicates, temporal);
    let (kept, filtered) = apply_filters(candidates, pipeline.table(), &predicates);
    let mut flags = compute_flags(&kept, pipeline, query_committed_at, &predicates);

    let limit_value = predicates.limit.unwrap_or(DEFAULT_LIMIT);
    let (records, flags_with_limit) = apply_limit(kept, limit_value, flags);
    flags = flags_with_limit;

    let framings = if predicates.show_framing {
        records
            .iter()
            .map(|r| compute_framing(r, pipeline, predicates.as_of, query_committed_at))
            .collect()
    } else {
        Vec::new()
    };

    Ok(ReadResult {
        records,
        framings,
        filtered,
        flags,
        as_of: effective_as_of,
        as_committed: effective_as_committed,
        query_committed_at,
    })
}

/// `:s` and `:p` are SEM / INF only per read-protocol.md § 4.1.
/// Combining them with `:kind pro` or `:kind epi` is a spec
/// violation — reject loudly rather than silently returning an
/// empty result (spec § 13 #9 "no silent information loss").
fn check_predicate_compatibility(predicates: &Predicates) -> Result<(), ReadError> {
    let Some(k) = predicates.kind else {
        return Ok(());
    };
    if !matches!(k, KindFilter::Pro | KindFilter::Epi) {
        return Ok(());
    }
    if !matches!(predicates.subject, SymbolFilter::Absent) {
        return Err(ReadError::IncompatiblePredicates {
            predicate: "s",
            kind: k,
        });
    }
    if !matches!(predicates.predicate, SymbolFilter::Absent) {
        return Err(ReadError::IncompatiblePredicates {
            predicate: "p",
            kind: k,
        });
    }
    Ok(())
}

/// Gather candidate records via the temporal resolver, plus projected
/// records (which bypass the resolver because their `valid_at` is
/// future) when `:include_projected true`. Applies the Episode-scoped
/// predicate (`:in_episode` / `:after_episode` / `:before_episode`)
/// last as a `committed_at`-based prune.
fn collect_candidates(
    pipeline: &Pipeline,
    predicates: &Predicates,
    temporal: TemporalQuery,
) -> Vec<CanonicalRecord> {
    // A NoMatch on either symbol predicate guarantees an empty
    // result — the agent referenced an unknown name, which the spec
    // treats as "no memories at this key" rather than an error.
    if predicates.subject.is_no_match() || predicates.predicate.is_no_match() {
        return Vec::new();
    }
    // Similarly, an Episode-scoped predicate that names an unknown
    // Episode returns empty rather than erroring.
    if predicates
        .episode
        .as_ref()
        .is_some_and(EpisodeFilter::is_empty_set)
    {
        return Vec::new();
    }
    let mut candidates: Vec<CanonicalRecord> = Vec::new();
    if matches!(predicates.kind, None | Some(KindFilter::Sem)) {
        collect_semantic(
            pipeline,
            predicates.subject,
            predicates.predicate,
            temporal,
            &mut candidates,
        );
    }
    if matches!(predicates.kind, None | Some(KindFilter::Pro))
        && matches!(predicates.subject, SymbolFilter::Absent)
        && matches!(predicates.predicate, SymbolFilter::Absent)
    {
        collect_procedural(pipeline, temporal, &mut candidates);
    }
    if matches!(predicates.kind, None | Some(KindFilter::Inf)) {
        collect_inferential(
            pipeline,
            predicates.subject,
            predicates.predicate,
            temporal,
            &mut candidates,
        );
    }
    if predicates.include_projected {
        collect_projected(
            pipeline,
            predicates.kind,
            predicates.subject,
            predicates.predicate,
            &mut candidates,
        );
    }
    if let Some(filter) = predicates.episode.as_ref() {
        candidates.retain(|r| filter.matches(r.committed_at()));
    }
    candidates.sort_by_key(CanonicalRecord::committed_at);
    candidates
}

/// Apply the retired-symbol and projected filters. Dropped records
/// surface in `filtered` only when `:explain_filtered true`;
/// otherwise they drop silently per spec § 11.1.
fn apply_filters(
    candidates: Vec<CanonicalRecord>,
    table: &SymbolTable,
    predicates: &Predicates,
) -> (Vec<CanonicalRecord>, Vec<FilteredMemory>) {
    let mut kept: Vec<CanonicalRecord> = Vec::with_capacity(candidates.len());
    let mut filtered: Vec<FilteredMemory> = Vec::new();
    for record in candidates {
        let retired_ref = record_references_retired(&record, table);
        let projected = record_is_projected(&record);

        if retired_ref && !predicates.include_retired {
            if predicates.explain_filtered {
                filtered.push(FilteredMemory {
                    record,
                    reason: FilterReason::RetiredSymbolExcluded,
                });
            }
            continue;
        }
        if projected && !predicates.include_projected {
            if predicates.explain_filtered {
                filtered.push(FilteredMemory {
                    record,
                    reason: FilterReason::ProjectedExcluded,
                });
            }
            continue;
        }
        kept.push(record);
    }
    (kept, filtered)
}

/// Compute flags over the kept record set. Flags describe the
/// returned rows; they don't double-count filtered drops. The
/// confidence comparison uses *effective* confidence (spec § 3):
/// decay-adjusted per the pipeline's `DecayConfig` and relative to
/// the query's snapshot time.
fn compute_flags(
    kept: &[CanonicalRecord],
    pipeline: &Pipeline,
    query_committed_at: ClockTime,
    predicates: &Predicates,
) -> ReadFlags {
    let mut flags = ReadFlags::empty();
    let table = pipeline.table();
    for record in kept {
        if record_references_retired(record, table) {
            flags = flags.with(ReadFlags::STALE_SYMBOL);
        }
        if record_is_projected(record) {
            flags = flags.with(ReadFlags::PROJECTED_PRESENT);
        }
        let effective = record_effective_confidence(record, pipeline, query_committed_at);
        if effective < predicates.confidence_threshold {
            flags = flags.with(ReadFlags::LOW_CONFIDENCE);
        }
    }
    if predicates.explain_filtered {
        flags = flags.with(ReadFlags::EXPLAIN_FILTERED_ACTIVE);
    }
    flags
}

/// The empty-pipeline result — no memories exist to match, but the
/// result must still carry the query's effective clocks and the
/// correct `EXPLAIN_FILTERED_ACTIVE` flag if the toggle was set.
fn empty_result(predicates: &Predicates) -> ReadResult {
    let mut flags = ReadFlags::empty();
    if predicates.explain_filtered {
        flags = flags.with(ReadFlags::EXPLAIN_FILTERED_ACTIVE);
    }
    ReadResult {
        records: Vec::new(),
        framings: Vec::new(),
        filtered: Vec::new(),
        flags,
        as_of: predicates.as_of.unwrap_or_else(epoch_zero),
        as_committed: predicates.as_committed.unwrap_or_else(epoch_zero),
        query_committed_at: epoch_zero(),
    }
}

/// Walk the pipeline's Semantic history, keep each record the
/// resolver would classify as authoritative at `temporal`. When
/// `s` / `p` are set, only memories at that specific `(s, p)` can
/// match — the resolver already tie-breaks within a pair.
///
/// When `s` / `p` are unset we iterate distinct `(s, p)` pairs that
/// appear in the history and run the resolver once per pair.
fn collect_semantic(
    pipeline: &Pipeline,
    s: SymbolFilter,
    p: SymbolFilter,
    temporal: TemporalQuery,
    out: &mut Vec<CanonicalRecord>,
) {
    if let (Some(sub), Some(pred)) = (s.as_id(), p.as_id()) {
        if let Some(rec) = resolver::resolve_semantic(pipeline, sub, pred, temporal) {
            out.push(CanonicalRecord::Sem(rec));
        }
        return;
    }
    // Enumerate distinct (s, p) pairs, applying the filter (which
    // may be Absent on one axis) as we go.
    let mut seen: std::collections::BTreeSet<(SymbolId, SymbolId)> =
        std::collections::BTreeSet::new();
    for record in pipeline.semantic_records() {
        if !s.matches(record.s) || !p.matches(record.p) {
            continue;
        }
        let key = (record.s, record.p);
        if !seen.insert(key) {
            continue;
        }
        if let Some(rec) = resolver::resolve_semantic(pipeline, key.0, key.1, temporal) {
            out.push(CanonicalRecord::Sem(rec));
        }
    }
}

fn collect_procedural(
    pipeline: &Pipeline,
    temporal: TemporalQuery,
    out: &mut Vec<CanonicalRecord>,
) {
    let mut seen_rules: std::collections::BTreeSet<SymbolId> = std::collections::BTreeSet::new();
    for record in pipeline.procedural_records() {
        if !seen_rules.insert(record.rule_id) {
            continue;
        }
        if let Some(rec) = resolver::resolve_procedural(pipeline, record.rule_id, temporal) {
            out.push(CanonicalRecord::Pro(rec));
        }
    }
}

/// Walk the pipeline's Inferential history, keep each record the
/// resolver classifies as authoritative at `temporal`. Mirrors
/// [`collect_semantic`]: when `s` / `p` are both pinned we resolve
/// the single `(s, p)` bucket; otherwise we enumerate distinct
/// `(s, p)` pairs from the history and run the resolver once per
/// pair.
fn collect_inferential(
    pipeline: &Pipeline,
    s: SymbolFilter,
    p: SymbolFilter,
    temporal: TemporalQuery,
    out: &mut Vec<CanonicalRecord>,
) {
    if let (Some(sub), Some(pred)) = (s.as_id(), p.as_id()) {
        if let Some(rec) = resolver::resolve_inferential(pipeline, sub, pred, temporal) {
            out.push(CanonicalRecord::Inf(rec));
        }
        return;
    }
    let mut seen: std::collections::BTreeSet<(SymbolId, SymbolId)> =
        std::collections::BTreeSet::new();
    for record in pipeline.inferential_records() {
        if !s.matches(record.s) || !p.matches(record.p) {
            continue;
        }
        let key = (record.s, record.p);
        if !seen.insert(key) {
            continue;
        }
        if let Some(rec) = resolver::resolve_inferential(pipeline, key.0, key.1, temporal) {
            out.push(CanonicalRecord::Inf(rec));
        }
    }
}

/// Collect memories whose `flags.projected` is set. These bypass the
/// as-of resolver because projections are by definition future-valid.
/// Records already present in `out` (via the resolver) are skipped to
/// avoid duplicates.
fn collect_projected(
    pipeline: &Pipeline,
    kind: Option<KindFilter>,
    s: SymbolFilter,
    p: SymbolFilter,
    out: &mut Vec<CanonicalRecord>,
) {
    let existing: std::collections::BTreeSet<SymbolId> = out
        .iter()
        .filter_map(|r| match r {
            CanonicalRecord::Sem(sem) => Some(sem.memory_id),
            CanonicalRecord::Pro(pro) => Some(pro.memory_id),
            CanonicalRecord::Inf(inf) => Some(inf.memory_id),
            CanonicalRecord::Epi(epi) => Some(epi.memory_id),
            _ => None,
        })
        .collect();
    if matches!(kind, None | Some(KindFilter::Sem)) {
        for record in pipeline.semantic_records() {
            if !record.flags.projected {
                continue;
            }
            if !s.matches(record.s) || !p.matches(record.p) {
                continue;
            }
            if existing.contains(&record.memory_id) {
                continue;
            }
            out.push(CanonicalRecord::Sem(record.clone()));
        }
    }
    // Procedural records cannot carry `projected` after the wire-format
    // split (ir-canonical-form.md § 5): `ProRecord` has no flags byte.
    // Any future projection semantics for Pro would need its own opcode.
    if matches!(kind, None | Some(KindFilter::Inf)) {
        for record in pipeline.inferential_records() {
            if !record.flags.projected {
                continue;
            }
            if !s.matches(record.s) || !p.matches(record.p) {
                continue;
            }
            if existing.contains(&record.memory_id) {
                continue;
            }
            out.push(CanonicalRecord::Inf(record.clone()));
        }
    }
}

/// Apply `:limit` — truncate the record set and set the `TRUNCATED`
/// flag on top of `existing_flags` if the limit was hit.
fn apply_limit(
    records: Vec<CanonicalRecord>,
    limit: usize,
    existing_flags: ReadFlags,
) -> (Vec<CanonicalRecord>, ReadFlags) {
    if records.len() > limit {
        let truncated: Vec<_> = records.into_iter().take(limit).collect();
        (truncated, existing_flags.with(ReadFlags::TRUNCATED))
    } else {
        (records, existing_flags)
    }
}

/// True if any `SymbolId` the record references is currently retired
/// in `table`. Per spec § 7 the check covers top-level symbol fields
/// plus `Value::Symbol` payloads inside object / trigger / action /
/// precondition slots. Inferential parent IDs (`derived_from`) are
/// memory-id symbols; retirement on memory IDs is not meaningful in
/// v1, but we still check them for spec completeness.
fn record_references_retired(record: &CanonicalRecord, table: &SymbolTable) -> bool {
    match record {
        CanonicalRecord::Sem(r) => sem_has_retired_ref(r, table),
        CanonicalRecord::Epi(r) => epi_has_retired_ref(r, table),
        CanonicalRecord::Pro(r) => pro_has_retired_ref(r, table),
        CanonicalRecord::Inf(r) => inf_has_retired_ref(r, table),
        // Edge / checkpoint / symbol-event records are never returned
        // as memory records, so this arm is unreachable in practice.
        _ => false,
    }
}

fn sem_has_retired_ref(r: &SemRecord, table: &SymbolTable) -> bool {
    table.is_retired(r.s)
        || table.is_retired(r.p)
        || table.is_retired(r.source)
        || value_has_retired_symbol(&r.o, table)
}

fn epi_has_retired_ref(r: &EpiRecord, table: &SymbolTable) -> bool {
    table.is_retired(r.event_id)
        || table.is_retired(r.kind)
        || table.is_retired(r.location)
        || table.is_retired(r.source)
        || r.participants.iter().any(|p| table.is_retired(*p))
}

fn pro_has_retired_ref(r: &ProRecord, table: &SymbolTable) -> bool {
    table.is_retired(r.rule_id)
        || table.is_retired(r.scope)
        || table.is_retired(r.source)
        || value_has_retired_symbol(&r.trigger, table)
        || value_has_retired_symbol(&r.action, table)
        || r.precondition
            .as_ref()
            .is_some_and(|v| value_has_retired_symbol(v, table))
}

fn inf_has_retired_ref(r: &InfRecord, table: &SymbolTable) -> bool {
    table.is_retired(r.s)
        || table.is_retired(r.p)
        || table.is_retired(r.method)
        || value_has_retired_symbol(&r.o, table)
        || r.derived_from.iter().any(|p| table.is_retired(*p))
}

fn value_has_retired_symbol(v: &Value, table: &SymbolTable) -> bool {
    matches!(v, Value::Symbol(id) if table.is_retired(*id))
}

/// True if the record carries the `projected` flag. Only Sem and Inf
/// carry a flags byte on the wire (ir-canonical-form.md § 5); Epi and
/// Pro have no flags and therefore cannot be projected.
fn record_is_projected(record: &CanonicalRecord) -> bool {
    match record {
        CanonicalRecord::Sem(r) => r.flags.projected,
        CanonicalRecord::Inf(r) => r.flags.projected,
        _ => false,
    }
}

/// Compute the effective (decay-adjusted) confidence for a memory
/// record at the query's snapshot time.
///
/// The confidence-decay spec (§ 3) defines effective as
/// `stored × decay_factor(elapsed, half_life)` for non-Inferential
/// memories, modulo pin / authoritative short-circuits (not yet
/// wired — see PLAN.md Step 4). For Inferential memories the spec
/// says decay composes from current parent effective confidences;
/// that composition lands with the Inferential resolver, so for
/// now we return stored.
///
/// `elapsed_ms` is `query_committed_at - valid_at` in milliseconds.
/// The source kind is looked up from the source symbol's canonical
/// name via [`source_kind_from_name`]; unknown symbols fall back to
/// `SourceKind::Observation`, the same default the semantic stage
/// uses at write time.
fn record_effective_confidence(
    record: &CanonicalRecord,
    pipeline: &Pipeline,
    query_committed_at: ClockTime,
) -> Confidence {
    let table = pipeline.table();
    let decay_config = pipeline.decay_config();
    let (stored, memory_kind, source_id, valid_at) = match record {
        CanonicalRecord::Sem(r) => (
            r.confidence,
            MemoryKindTag::Semantic,
            r.source,
            r.clocks.valid_at,
        ),
        CanonicalRecord::Epi(r) => (r.confidence, MemoryKindTag::Episodic, r.source, r.at_time),
        CanonicalRecord::Pro(r) => (
            r.confidence,
            MemoryKindTag::Procedural,
            r.source,
            r.clocks.valid_at,
        ),
        // Inferential decay composes from parents; defer to the
        // Inferential resolver work (#29). For now use stored as a
        // safe upper bound.
        CanonicalRecord::Inf(r) => return r.confidence,
        _ => return Confidence::ONE,
    };

    let source_kind = table.entry(source_id).map_or(SourceKind::Observation, |e| {
        source_kind_from_name(e.canonical_name.as_str())
    });
    let elapsed_ms = query_committed_at
        .as_millis()
        .saturating_sub(valid_at.as_millis());
    let memory_id = record_memory_id(record);
    let pinned = memory_id.is_some_and(|id| pipeline.is_pinned(id));
    let authoritative = memory_id.is_some_and(|id| pipeline.is_authoritative(id));
    let flags = DecayFlags {
        pinned,
        authoritative,
    };
    effective_confidence(
        stored,
        elapsed_ms,
        memory_kind,
        source_kind,
        flags,
        decay_config,
    )
}

/// Per-record framing. Priority order (spec § 5):
/// - `Projected` if the record has `flags.projected` set — dominates
///   other classifications because the agent needs to know the
///   memory is intent, not current truth.
/// - `Authoritative { set_by }` if the record is pinned or
///   operator-authoritative. Pin takes precedence over operator
///   flag when both are set (it's the narrower, agent-owned source
///   — the operator flag is broader).
/// - `Historical` if the query asked for `:as_of T` with T predating
///   `query_committed_at`.
/// - `Advisory` otherwise.
fn compute_framing(
    record: &CanonicalRecord,
    pipeline: &Pipeline,
    as_of: Option<ClockTime>,
    query_committed_at: ClockTime,
) -> Framing {
    if record_is_projected(record) {
        return Framing::Projected;
    }
    if let Some(mem_id) = record_memory_id(record) {
        if pipeline.is_pinned(mem_id) {
            return Framing::Authoritative {
                set_by: FramingSource::AgentPinned,
            };
        }
        if pipeline.is_authoritative(mem_id) {
            return Framing::Authoritative {
                set_by: FramingSource::OperatorAuthoritative,
            };
        }
    }
    if as_of.is_some_and(|t| t < query_committed_at) {
        return Framing::Historical;
    }
    Framing::Advisory
}

/// Extract the `memory_id` from a record — used to check the
/// pipeline's pin / authoritative sets.
fn record_memory_id(record: &CanonicalRecord) -> Option<SymbolId> {
    match record {
        CanonicalRecord::Sem(r) => Some(r.memory_id),
        CanonicalRecord::Epi(r) => Some(r.memory_id),
        CanonicalRecord::Pro(r) => Some(r.memory_id),
        CanonicalRecord::Inf(r) => Some(r.memory_id),
        _ => None,
    }
}

/// Default `:limit` per `read-protocol.md` § 4.2.
const DEFAULT_LIMIT: usize = 1000;

/// Parsed keyword arguments, resolved to pipeline state.
///
/// The four boolean toggles mirror the spec's independent read-side
/// knobs (`include_retired`, `include_projected`, `explain_filtered`,
/// `show_framing`). Collapsing them into a state machine would hide
/// the one-to-one mapping with spec § 4.1, so we allow the pedantic
/// bool-count here.
#[allow(clippy::struct_excessive_bools)]
struct Predicates {
    kind: Option<KindFilter>,
    subject: SymbolFilter,
    predicate: SymbolFilter,
    as_of: Option<ClockTime>,
    as_committed: Option<ClockTime>,
    limit: Option<usize>,
    include_retired: bool,
    include_projected: bool,
    confidence_threshold: Confidence,
    explain_filtered: bool,
    show_framing: bool,
    episode: Option<EpisodeFilter>,
}

/// How an Episode-scoped read predicate restricts the result set.
/// See `read-protocol.md` § 4.1 and 6.
#[derive(Clone, Debug, PartialEq, Eq)]
enum EpisodeFilter {
    /// Keep memories whose `committed_at` equals `at` — i.e. whose
    /// Episode is the one the user specified.
    In { at: ClockTime },
    /// Keep memories whose `committed_at` > `at`.
    After { at: ClockTime },
    /// Keep memories whose `committed_at` < `at`.
    Before { at: ClockTime },
    /// Keep memories whose `committed_at` equals any Episode in
    /// the chain `@E → parent → grandparent → …`. Backed by
    /// `Pipeline::episode_chain` walking `parent_episode`.
    Chain { ats: Vec<ClockTime> },
    /// Predicate referenced an Episode symbol that the pipeline has
    /// no record of — yields an empty result set rather than an
    /// error, matching the `NoMatch` semantics of `:s` / `:p`.
    UnknownEpisode,
}

impl EpisodeFilter {
    fn matches(&self, committed_at: ClockTime) -> bool {
        match self {
            Self::In { at } => committed_at == *at,
            Self::After { at } => committed_at > *at,
            Self::Before { at } => committed_at < *at,
            Self::Chain { ats } => ats.contains(&committed_at),
            Self::UnknownEpisode => false,
        }
    }

    fn is_empty_set(&self) -> bool {
        matches!(self, Self::UnknownEpisode)
    }
}

/// Tri-state for a symbol-valued predicate:
/// - `Absent` — the predicate wasn't set; don't filter.
/// - `Match(id)` — the predicate resolved to `id`; filter to memories at that key.
/// - `NoMatch` — the predicate was set but its symbol doesn't exist
///   in the workspace; the result is necessarily empty.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum SymbolFilter {
    Absent,
    Match(SymbolId),
    NoMatch,
}

impl SymbolFilter {
    fn from_lookup(resolved: Option<SymbolId>, set: bool) -> Self {
        match (set, resolved) {
            (false, _) => Self::Absent,
            (true, Some(id)) => Self::Match(id),
            (true, None) => Self::NoMatch,
        }
    }

    /// `true` if this filter guarantees zero matches.
    fn is_no_match(self) -> bool {
        matches!(self, Self::NoMatch)
    }

    fn matches(self, id: SymbolId) -> bool {
        match self {
            Self::Absent => true,
            Self::Match(expected) => id == expected,
            Self::NoMatch => false,
        }
    }

    fn as_id(self) -> Option<SymbolId> {
        // `NoMatch` is short-circuited by `guaranteed_empty` in the
        // caller before we reach `collect_semantic`, so in practice
        // the `_ => None` arm only fires for `Absent`. Keeping it
        // total rather than `unreachable!()` so a future caller that
        // skips the short-circuit can't panic.
        match self {
            Self::Match(id) => Some(id),
            _ => None,
        }
    }
}

fn parse_predicates(
    pipeline: &Pipeline,
    keywords: parse::KeywordArgs,
) -> Result<Predicates, ReadError> {
    let table = pipeline.table();
    let mut out = Predicates {
        kind: None,
        subject: SymbolFilter::Absent,
        predicate: SymbolFilter::Absent,
        as_of: None,
        as_committed: None,
        limit: None,
        include_retired: false,
        include_projected: false,
        confidence_threshold: default_confidence_threshold(),
        explain_filtered: false,
        show_framing: false,
        episode: None,
    };
    let mut debug_mode = false;

    for (key, value) in keywords {
        match key.as_str() {
            "kind" => out.kind = Some(parse_kind(&value)?),
            "s" => {
                out.subject = SymbolFilter::from_lookup(resolve_symbol(table, &value, "s")?, true);
            }
            "p" => {
                out.predicate =
                    SymbolFilter::from_lookup(resolve_symbol(table, &value, "p")?, true);
            }
            "as_of" => out.as_of = Some(parse_timestamp(&value, "as_of")?),
            "as_committed" => out.as_committed = Some(parse_timestamp(&value, "as_committed")?),
            "limit" => out.limit = Some(parse_limit(&value)?),
            "include_retired" => out.include_retired = parse_bool(&value, "include_retired")?,
            "include_projected" => out.include_projected = parse_bool(&value, "include_projected")?,
            "confidence_threshold" => {
                out.confidence_threshold = parse_confidence(&value)?;
            }
            "explain_filtered" => out.explain_filtered = parse_bool(&value, "explain_filtered")?,
            "show_framing" => out.show_framing = parse_bool(&value, "show_framing")?,
            "debug_mode" => debug_mode = parse_bool(&value, "debug_mode")?,
            "in_episode" | "after_episode" | "before_episode" | "episode_chain" => {
                // Only one Episode-scoped predicate can be active at
                // a time; rejecting the combination is friendlier
                // than letting the last one silently overwrite.
                if out.episode.is_some() {
                    return Err(ReadError::InvalidPredicate {
                        keyword: "in_episode / after_episode / before_episode / episode_chain",
                        reason: "at most one Episode-scoped predicate per query".into(),
                    });
                }
                out.episode = Some(parse_episode_filter(pipeline, &key, &value)?);
            }
            // Every other predicate — known-but-unwired or truly
            // unknown — surfaces as `UnsupportedPredicate`. The
            // `static_key_name` helper returns a stable static name
            // for the known ones and `"unknown_predicate"` otherwise.
            _ => {
                return Err(ReadError::UnsupportedPredicate {
                    predicate: static_key_name(&key),
                });
            }
        }
    }

    // `:debug_mode true` implies both surfacing toggles per spec
    // § 4.1. Explicit per-toggle values can still turn things on;
    // debug mode is pure disjunction (OR), never disables.
    if debug_mode {
        out.explain_filtered = true;
        out.show_framing = true;
    }

    Ok(out)
}

/// Resolve `:in_episode @E` / `:after_episode @E` /
/// `:before_episode @E` / `:episode_chain @E` against the pipeline's
/// registered Episodes. An unknown-to-pipeline symbol yields
/// `UnknownEpisode`, producing an empty result set (matching the
/// `:s` / `:p` `NoMatch` semantics per spec § 4.1).
fn parse_episode_filter(
    pipeline: &Pipeline,
    keyword: &str,
    value: &RawValue,
) -> Result<EpisodeFilter, ReadError> {
    let static_key = match keyword {
        "in_episode" => "in_episode",
        "after_episode" => "after_episode",
        "before_episode" => "before_episode",
        "episode_chain" => "episode_chain",
        _ => "unknown_predicate",
    };
    let Some(id) = resolve_symbol(pipeline.table(), value, static_key)? else {
        return Ok(EpisodeFilter::UnknownEpisode);
    };
    let Some(at) = pipeline.episode_committed_at(id) else {
        return Ok(EpisodeFilter::UnknownEpisode);
    };
    Ok(match keyword {
        "in_episode" => EpisodeFilter::In { at },
        "after_episode" => EpisodeFilter::After { at },
        "before_episode" => EpisodeFilter::Before { at },
        // Precondition guaranteed by caller — only these four
        // keywords reach this helper.
        _ => {
            let ats: Vec<ClockTime> = pipeline
                .episode_chain(id)
                .filter_map(|ep| pipeline.episode_committed_at(ep))
                .collect();
            EpisodeFilter::Chain { ats }
        }
    })
}

/// Default `:confidence_threshold` per spec § 4.2 — 0.5.
fn default_confidence_threshold() -> Confidence {
    // 0.5 is exactly representable in u16 fixed-point
    // (u16::MAX / 2 rounded), so try_from_f32 cannot fail.
    #[allow(clippy::expect_used)]
    Confidence::try_from_f32(0.5).expect("0.5 is a valid Confidence")
}

fn parse_kind(value: &RawValue) -> Result<KindFilter, ReadError> {
    let name = match value {
        RawValue::Bareword(s) => s.as_str(),
        RawValue::RawSymbol(RawSymbolName { name, .. }) => name.as_str(),
        _ => {
            return Err(ReadError::InvalidPredicate {
                keyword: "kind",
                reason: "expected a bareword (sem, pro, epi, inf)".into(),
            })
        }
    };
    match name {
        "sem" => Ok(KindFilter::Sem),
        "pro" => Ok(KindFilter::Pro),
        "epi" => Ok(KindFilter::Epi),
        "inf" => Ok(KindFilter::Inf),
        other => Err(ReadError::InvalidKind {
            got: other.to_string(),
        }),
    }
}

/// Resolve a symbol-valued predicate (`:s @X`, `:p @Y`) against the
/// pipeline's table. Unknown symbols return `Ok(None)` — the query
/// simply won't match any memories at that key, which is the
/// correct no-crash read semantic.
fn resolve_symbol(
    table: &SymbolTable,
    value: &RawValue,
    keyword: &'static str,
) -> Result<Option<SymbolId>, ReadError> {
    let name: &str = match value {
        RawValue::RawSymbol(sym) => sym.as_str(),
        RawValue::TypedSymbol { name, .. } => name.as_str(),
        RawValue::Bareword(text) => text.as_str(),
        _ => {
            return Err(ReadError::InvalidPredicate {
                keyword,
                reason: "expected a symbol reference like @name".into(),
            })
        }
    };
    Ok(table.lookup(name))
}

fn parse_timestamp(value: &RawValue, keyword: &'static str) -> Result<ClockTime, ReadError> {
    match value {
        RawValue::Timestamp(t) => Ok(*t),
        _ => Err(ReadError::InvalidPredicate {
            keyword,
            reason: "expected an ISO-8601 timestamp".into(),
        }),
    }
}

fn parse_limit(value: &RawValue) -> Result<usize, ReadError> {
    match value {
        RawValue::Integer(n) if *n >= 0 => {
            usize::try_from(*n).map_err(|_| ReadError::InvalidPredicate {
                keyword: "limit",
                reason: "limit exceeds usize".into(),
            })
        }
        _ => Err(ReadError::InvalidPredicate {
            keyword: "limit",
            reason: "expected a non-negative integer".into(),
        }),
    }
}

fn parse_bool(value: &RawValue, keyword: &'static str) -> Result<bool, ReadError> {
    match value {
        RawValue::Boolean(b) => Ok(*b),
        _ => Err(ReadError::InvalidPredicate {
            keyword,
            reason: "expected a boolean".into(),
        }),
    }
}

fn parse_confidence(value: &RawValue) -> Result<Confidence, ReadError> {
    let f = match value {
        RawValue::Float(f) => *f,
        // Accept `1` / `0` as shorthand for the extremes.
        RawValue::Integer(n) if *n == 0 || *n == 1 => f64::from(i32::try_from(*n).unwrap_or(0)),
        _ => {
            return Err(ReadError::InvalidPredicate {
                keyword: "confidence_threshold",
                reason: "expected a float in [0.0, 1.0]".into(),
            });
        }
    };
    #[allow(clippy::cast_possible_truncation)]
    Confidence::try_from_f32(f as f32).map_err(|_| ReadError::InvalidPredicate {
        keyword: "confidence_threshold",
        reason: "expected a float in [0.0, 1.0]".into(),
    })
}

/// Fallback used for the `None`-pipeline branch. The only failing
/// input to `ClockTime::try_from_millis` is `u64::MAX`; `0` always
/// succeeds. The `expect` lives here (rather than bubbling up the
/// `Result`) because an empty pipeline's `ReadResult` already
/// carries zero records — the clocks are diagnostic, not load-
/// bearing, so there's no information loss in panicking on an
/// impossible branch.
#[allow(clippy::expect_used)]
fn epoch_zero() -> ClockTime {
    ClockTime::try_from_millis(0).expect("0ms is always a valid ClockTime")
}

/// Map a dynamic keyword string back to the `&'static str` slot
/// used in the `UnsupportedPredicate` variant. Only *known-but-
/// unwired* keywords get a stable name here; wired keywords are
/// handled upstream in `parse_predicates`. Unknown keywords fall
/// back to a generic label.
fn static_key_name(key: &str) -> &'static str {
    match key {
        "o" => "o",
        "read_after" => "read_after",
        "timeout_ms" => "timeout_ms",
        _ => "unknown_predicate",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> ClockTime {
        ClockTime::try_from_millis(1_713_350_400_000).expect("non-sentinel")
    }

    fn compile(pipe: &mut Pipeline, src: &str) {
        pipe.compile_batch(src, now()).expect("compile");
    }

    const SEM_ALICE: &str = "(sem @alice @knows @bob :src @observation :c 0.8 :v 2024-01-15)";
    // Distinct (s, p) pair so both memories stay current (no supersession conflict).
    const SEM_TRUSTS: &str = "(sem @alice @trusts @carol :src @observation :c 0.8 :v 2024-01-15)";
    const PRO_RULE: &str = r#"(pro @rule_route "agent_write" "route_via_librarian"
        :scp @mimir :src @policy :c 1.0)"#;

    #[test]
    fn empty_pipeline_returns_empty_result() {
        let pipe = Pipeline::new();
        let got = pipe.execute_query("(query :s @alice :p @knows)").unwrap();
        assert!(got.records.is_empty());
        assert_eq!(got.flags, ReadFlags::empty());
    }

    #[test]
    fn exact_sp_match_returns_current_memory() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        let got = pipe
            .execute_query("(query :s @alice :p @knows)")
            .expect("query");
        assert_eq!(got.records.len(), 1);
        let CanonicalRecord::Sem(sem) = &got.records[0] else {
            panic!("expected Sem");
        };
        let alice = pipe.table().lookup("alice").unwrap();
        assert_eq!(sem.s, alice);
    }

    #[test]
    fn unknown_symbol_returns_empty_not_error() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        let got = pipe
            .execute_query("(query :s @nonexistent :p @knows)")
            .expect("unknown symbol is OK");
        assert!(got.records.is_empty());
    }

    #[test]
    fn unscoped_query_returns_current_across_pairs() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        compile(&mut pipe, SEM_TRUSTS);
        let got = pipe.execute_query("(query)").expect("all");
        assert_eq!(got.records.len(), 2);
    }

    #[test]
    fn kind_filter_isolates_sem_from_pro() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        compile(&mut pipe, PRO_RULE);

        let sem_only = pipe.execute_query("(query :kind sem)").expect("sem");
        assert_eq!(sem_only.records.len(), 1);
        assert!(matches!(sem_only.records[0], CanonicalRecord::Sem(_)));

        let pro_only = pipe.execute_query("(query :kind pro)").expect("pro");
        assert_eq!(pro_only.records.len(), 1);
        assert!(matches!(pro_only.records[0], CanonicalRecord::Pro(_)));
    }

    #[test]
    fn kind_epi_returns_empty_in_71_scope() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        let got = pipe.execute_query("(query :kind epi)").expect("epi");
        assert!(got.records.is_empty());
    }

    #[test]
    fn invalid_kind_bareword_is_rejected() {
        let pipe = Pipeline::new();
        let err = pipe
            .execute_query("(query :kind bogus)")
            .expect_err("bad kind");
        assert!(matches!(err, ReadError::InvalidKind { .. }));
    }

    #[test]
    fn as_of_past_valid_time_returns_earlier_record() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        compile(
            &mut pipe,
            "(sem @alice @knows @carol :src @observation :c 0.8 :v 2024-03-01)",
        );

        // Query at 2024-02-01 — @bob was current, @carol not yet.
        let got = pipe
            .execute_query("(query :s @alice :p @knows :as_of 2024-02-01)")
            .expect("as_of");
        let CanonicalRecord::Sem(sem) = &got.records[0] else {
            panic!();
        };
        let bob = pipe.table().lookup("bob").unwrap();
        assert!(matches!(&sem.o, crate::Value::Symbol(id) if *id == bob));

        // Current read returns @carol.
        let current = pipe
            .execute_query("(query :s @alice :p @knows)")
            .expect("current");
        let CanonicalRecord::Sem(sem) = &current.records[0] else {
            panic!();
        };
        let carol = pipe.table().lookup("carol").unwrap();
        assert!(matches!(&sem.o, crate::Value::Symbol(id) if *id == carol));
    }

    #[test]
    fn limit_truncates_and_sets_flag() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        compile(&mut pipe, SEM_TRUSTS);
        let got = pipe.execute_query("(query :limit 1)").expect("limit");
        assert_eq!(got.records.len(), 1);
        assert!(got.flags.contains(ReadFlags::TRUNCATED));
    }

    #[test]
    fn limit_not_hit_leaves_flag_clear() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        let got = pipe.execute_query("(query :limit 10)").expect("limit");
        assert_eq!(got.records.len(), 1);
        assert!(!got.flags.contains(ReadFlags::TRUNCATED));
    }

    #[test]
    fn unsupported_predicate_returns_typed_error() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        // `:read_after` is known-but-unwired — wire-architecture
        // work (Step 6) needs to land before it becomes useful.
        let err = pipe
            .execute_query("(query :read_after @foo)")
            .expect_err("unsupported");
        assert!(matches!(
            err,
            ReadError::UnsupportedPredicate {
                predicate: "read_after"
            }
        ));
    }

    #[test]
    fn s_predicate_with_kind_pro_is_rejected() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        compile(&mut pipe, PRO_RULE);
        let err = pipe
            .execute_query("(query :s @alice :kind pro)")
            .expect_err("s + kind pro must reject");
        assert!(matches!(
            err,
            ReadError::IncompatiblePredicates {
                predicate: "s",
                kind: KindFilter::Pro,
            }
        ));
    }

    #[test]
    fn p_predicate_with_kind_epi_is_rejected() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        let err = pipe
            .execute_query("(query :p @knows :kind epi)")
            .expect_err("p + kind epi must reject");
        assert!(matches!(
            err,
            ReadError::IncompatiblePredicates {
                predicate: "p",
                kind: KindFilter::Epi,
            }
        ));
    }

    #[test]
    fn write_path_query_still_unsupported() {
        // The (query ...) form going through `compile_batch` still
        // returns EmitError::Unsupported — writes don't execute
        // reads. This keeps the read path cleanly separated until
        // the read-protocol spec defines how the two interact.
        let mut pipe = Pipeline::new();
        let err = pipe
            .compile_batch("(query :s @alice :p @knows)", now())
            .expect_err("write path rejects query");
        assert!(matches!(
            err,
            crate::pipeline::PipelineError::Emit(crate::pipeline::EmitError::Unsupported {
                form: "query"
            })
        ));
    }

    #[test]
    fn multiple_forms_rejected() {
        let pipe = Pipeline::new();
        let err = pipe
            .execute_query("(query) (query)")
            .expect_err("two forms");
        assert!(matches!(err, ReadError::NotASingleQuery { count: 2 }));
    }

    #[test]
    fn non_query_form_rejected() {
        let pipe = Pipeline::new();
        let err = pipe
            .execute_query("(sem @a @b @c :src @observation :c 0.8 :v 2024-01-15)")
            .expect_err("not a query");
        assert!(matches!(err, ReadError::NotAQuery));
    }

    // ----- 7.2: filter predicates + flags + framing + filtered -----

    const SEM_LOW_CONF: &str = "(sem @mira @likes @tea :src @self_report :c 0.3 :v 2024-01-15)";
    const SEM_PROJECTED: &str =
        "(sem @plan @deploys @mimir :src @agent_instruction :c 0.9 :v 2099-01-01 :projected true)";

    #[test]
    fn retired_symbol_default_drops_record() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        // Retire @bob — SEM_ALICE references it as the object.
        compile(&mut pipe, "(retire @bob)");
        let got = pipe.execute_query("(query)").expect("query");
        assert!(
            got.records.is_empty(),
            "retired-symbol record should drop by default, got {:?}",
            got.records
        );
        assert!(!got.flags.contains(ReadFlags::STALE_SYMBOL));
    }

    #[test]
    fn include_retired_keeps_record_and_sets_flag() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        compile(&mut pipe, "(retire @bob)");
        let got = pipe
            .execute_query("(query :include_retired true)")
            .expect("query");
        assert_eq!(got.records.len(), 1);
        assert!(got.flags.contains(ReadFlags::STALE_SYMBOL));
    }

    #[test]
    fn projected_default_drops_record() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        compile(&mut pipe, SEM_PROJECTED);
        let got = pipe.execute_query("(query)").expect("query");
        // Only SEM_ALICE should come back; SEM_PROJECTED is filtered.
        assert_eq!(got.records.len(), 1);
        assert!(!got.flags.contains(ReadFlags::PROJECTED_PRESENT));
    }

    #[test]
    fn include_projected_keeps_record_and_sets_flag() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        compile(&mut pipe, SEM_PROJECTED);
        let got = pipe
            .execute_query("(query :include_projected true)")
            .expect("query");
        assert_eq!(got.records.len(), 2);
        assert!(got.flags.contains(ReadFlags::PROJECTED_PRESENT));
    }

    #[test]
    fn low_confidence_flag_fires_on_default_threshold() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_LOW_CONF);
        let got = pipe.execute_query("(query)").expect("query");
        assert_eq!(got.records.len(), 1);
        // 0.3 < 0.5 default → flag set.
        assert!(got.flags.contains(ReadFlags::LOW_CONFIDENCE));
    }

    #[test]
    fn confidence_threshold_override_tightens_flag() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE); // c=0.8
                                       // At threshold 0.9, 0.8 should flag as low.
        let got = pipe
            .execute_query("(query :confidence_threshold 0.9)")
            .expect("query");
        assert!(got.flags.contains(ReadFlags::LOW_CONFIDENCE));
        // Default (0.5) — 0.8 should NOT flag.
        let got_default = pipe.execute_query("(query)").expect("query");
        assert!(!got_default.flags.contains(ReadFlags::LOW_CONFIDENCE));
    }

    #[test]
    fn confidence_threshold_flag_only_does_not_filter() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_LOW_CONF); // c=0.3
        let got = pipe.execute_query("(query)").expect("query");
        // The low-confidence record is kept; flag just warns.
        assert_eq!(got.records.len(), 1);
        assert!(got.flags.contains(ReadFlags::LOW_CONFIDENCE));
    }

    #[test]
    fn explain_filtered_surfaces_dropped_records() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        compile(&mut pipe, "(retire @bob)");
        let got = pipe
            .execute_query("(query :explain_filtered true)")
            .expect("query");
        assert!(got.records.is_empty());
        assert_eq!(got.filtered.len(), 1);
        assert_eq!(got.filtered[0].reason, FilterReason::RetiredSymbolExcluded);
        assert!(got.flags.contains(ReadFlags::EXPLAIN_FILTERED_ACTIVE));
    }

    #[test]
    fn explain_filtered_off_keeps_filtered_empty() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        compile(&mut pipe, "(retire @bob)");
        let got = pipe.execute_query("(query)").expect("query");
        assert!(got.filtered.is_empty());
        assert!(!got.flags.contains(ReadFlags::EXPLAIN_FILTERED_ACTIVE));
    }

    #[test]
    fn show_framing_populates_per_record() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        let got = pipe
            .execute_query("(query :show_framing true)")
            .expect("query");
        assert_eq!(got.framings.len(), got.records.len());
        assert_eq!(got.framings[0], Framing::Advisory);
    }

    #[test]
    fn show_framing_off_leaves_framings_empty() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        let got = pipe.execute_query("(query)").expect("query");
        assert!(got.framings.is_empty());
    }

    #[test]
    fn framing_historical_when_as_of_is_past() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        let got = pipe
            .execute_query("(query :as_of 2024-01-20 :show_framing true)")
            .expect("query");
        assert_eq!(got.framings.len(), 1);
        assert_eq!(got.framings[0], Framing::Historical);
    }

    #[test]
    fn framing_projected_for_projected_record() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_PROJECTED);
        let got = pipe
            .execute_query("(query :include_projected true :show_framing true)")
            .expect("query");
        assert_eq!(got.framings.len(), 1);
        assert_eq!(got.framings[0], Framing::Projected);
    }

    #[test]
    fn debug_mode_enables_both_toggles() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        compile(&mut pipe, "(retire @bob)");
        let got = pipe
            .execute_query("(query :debug_mode true)")
            .expect("query");
        // Filtered surfaces AND framings parallels records.
        assert!(got.flags.contains(ReadFlags::EXPLAIN_FILTERED_ACTIVE));
        assert_eq!(got.filtered.len(), 1);
        // records is empty (retired dropped by default); framings
        // should also be empty and same length as records.
        assert_eq!(got.framings.len(), got.records.len());
    }

    #[test]
    fn include_retired_with_explain_filtered_shows_no_filtered() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        compile(&mut pipe, "(retire @bob)");
        let got = pipe
            .execute_query("(query :include_retired true :explain_filtered true)")
            .expect("query");
        // Record is kept, so nothing gets filtered — filtered stays empty.
        assert_eq!(got.records.len(), 1);
        assert!(got.filtered.is_empty());
        assert!(got.flags.contains(ReadFlags::STALE_SYMBOL));
        assert!(got.flags.contains(ReadFlags::EXPLAIN_FILTERED_ACTIVE));
    }

    #[test]
    fn invalid_boolean_predicate_is_rejected() {
        let pipe = Pipeline::new();
        let err = pipe
            .execute_query("(query :include_retired 5)")
            .expect_err("expected bool error");
        assert!(matches!(
            err,
            ReadError::InvalidPredicate {
                keyword: "include_retired",
                ..
            }
        ));
    }

    #[test]
    fn invalid_confidence_threshold_is_rejected() {
        let pipe = Pipeline::new();
        let err = pipe
            .execute_query("(query :confidence_threshold 1.5)")
            .expect_err("out of range");
        assert!(matches!(
            err,
            ReadError::InvalidPredicate {
                keyword: "confidence_threshold",
                ..
            }
        ));
    }

    // ----- 7.3: effective confidence drives LOW_CONFIDENCE -----

    /// Stored 0.8, `valid_at` 136 days before `now()`. Semantic ×
    /// `@observation` → 180-day half-life → factor ≈ 0.59 →
    /// effective ≈ 0.47 → below the default 0.5 threshold.
    const SEM_DECAYED_BELOW: &str =
        "(sem @mira @saw @kilroy :src @observation :c 0.8 :v 2023-12-01)";

    #[test]
    fn stored_above_threshold_but_effective_below_triggers_low_confidence() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_DECAYED_BELOW);
        let got = pipe.execute_query("(query)").expect("query");
        assert_eq!(got.records.len(), 1);
        let CanonicalRecord::Sem(sem) = &got.records[0] else {
            panic!("expected Sem");
        };
        assert!(sem.confidence.as_f32() > 0.5, "stored should be > 0.5");
        assert!(
            got.flags.contains(ReadFlags::LOW_CONFIDENCE),
            "effective (decay-adjusted) confidence should be < 0.5"
        );
    }

    #[test]
    fn recent_memory_stays_above_threshold() {
        // SEM_ALICE is valid_at 2024-01-15 → 93 days before now().
        // Semantic × @observation, factor ≈ 0.70, effective ≈ 0.56.
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        let got = pipe.execute_query("(query)").expect("query");
        assert!(!got.flags.contains(ReadFlags::LOW_CONFIDENCE));
    }

    // ----- 8.1: Episode-scoped read predicates -----

    /// Register the Episode that `compile_batch` implicitly commits
    /// under. Pipeline-level tests use this because `compile_batch`
    /// doesn't allocate the `__ep_{n}` symbol — that's `Store`'s job.
    /// Here we fabricate a Memory-kind symbol named `name`, point
    /// it at the current watermark, and register. An integration
    /// test against `Store` in `tests/round_trip.rs` covers the
    /// genuine end-to-end flow.
    fn register_latest_episode(pipe: &mut Pipeline, name: &str) -> crate::symbol::SymbolId {
        let at = pipe.last_committed_at().expect("committed");
        // Use a non-reserved name; `__ep_N` allocations from the
        // store path would collide across test calls since we don't
        // thread a counter here.
        let table_snapshot_len = pipe.table().iter_entries().count();
        let id = crate::symbol::SymbolId::new(u64::MAX - table_snapshot_len as u64);
        pipe.replay_allocate(id, name.into(), crate::symbol::SymbolKind::Memory)
            .expect("allocate");
        pipe.register_episode(id, at);
        id
    }

    #[test]
    fn in_episode_filters_to_that_commit() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        let ep_id = register_latest_episode(&mut pipe, "ep_alpha");
        // Commit a second batch under a different Episode.
        pipe.compile_batch(SEM_TRUSTS, later_now()).unwrap();
        let _beta = register_latest_episode(&mut pipe, "ep_beta");

        let got = pipe
            .execute_query("(query :in_episode @ep_alpha)")
            .expect("query");
        assert_eq!(got.records.len(), 1);
        let CanonicalRecord::Sem(sem) = &got.records[0] else {
            panic!();
        };
        let alice = pipe.table().lookup("alice").unwrap();
        assert_eq!(sem.s, alice, "should be the SEM_ALICE record");
        // Just to make sure the second Episode's id was different
        // from ep_alpha's.
        assert_ne!(ep_id.as_u64(), 0);
    }

    #[test]
    fn after_episode_filters_later_commits() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        let _alpha = register_latest_episode(&mut pipe, "ep_alpha");
        pipe.compile_batch(SEM_TRUSTS, later_now()).unwrap();
        let _beta = register_latest_episode(&mut pipe, "ep_beta");

        let got = pipe
            .execute_query("(query :after_episode @ep_alpha)")
            .expect("query");
        assert_eq!(got.records.len(), 1);
        let CanonicalRecord::Sem(sem) = &got.records[0] else {
            panic!();
        };
        let trusts = pipe.table().lookup("trusts").unwrap();
        assert_eq!(sem.p, trusts, "should be the SEM_TRUSTS record");
    }

    #[test]
    fn before_episode_filters_earlier_commits() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        let _alpha = register_latest_episode(&mut pipe, "ep_alpha");
        pipe.compile_batch(SEM_TRUSTS, later_now()).unwrap();
        let _beta = register_latest_episode(&mut pipe, "ep_beta");

        let got = pipe
            .execute_query("(query :before_episode @ep_beta)")
            .expect("query");
        assert_eq!(got.records.len(), 1);
        let CanonicalRecord::Sem(sem) = &got.records[0] else {
            panic!();
        };
        let alice = pipe.table().lookup("alice").unwrap();
        assert_eq!(sem.s, alice, "should be SEM_ALICE");
    }

    #[test]
    fn unknown_episode_symbol_returns_empty() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        let got = pipe
            .execute_query("(query :in_episode @nonexistent)")
            .expect("query");
        assert!(got.records.is_empty());
    }

    #[test]
    fn multiple_episode_scopes_rejected() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        let _ = register_latest_episode(&mut pipe, "ep_alpha");
        let err = pipe
            .execute_query("(query :in_episode @ep_alpha :after_episode @ep_alpha)")
            .expect_err("two Episode-scoped predicates must reject");
        assert!(matches!(
            err,
            ReadError::InvalidPredicate {
                keyword: "in_episode / after_episode / before_episode / episode_chain",
                ..
            }
        ));
    }

    fn later_now() -> ClockTime {
        ClockTime::try_from_millis(1_713_350_400_000 + 1_000).expect("non-sentinel")
    }

    #[test]
    fn decay_config_override_suppresses_decay() {
        // Same record as SEM_DECAYED_BELOW, but we disable decay
        // for Sem×Observation — effective equals stored, which is
        // above the threshold.
        let mut pipe = Pipeline::new();
        let mut cfg = crate::decay::DecayConfig::librarian_defaults();
        cfg.sem_observation = crate::decay::HalfLife::no_decay();
        pipe.set_decay_config(cfg);
        compile(&mut pipe, SEM_DECAYED_BELOW);
        let got = pipe.execute_query("(query)").expect("query");
        assert!(
            !got.flags.contains(ReadFlags::LOW_CONFIDENCE),
            "with decay disabled, stored 0.8 stays above 0.5"
        );
    }

    // ----------------------------------------------------------------
    // Inferential resolver — Phase 3.1
    //
    // Until Phase 3.1 wires the resolver, `(query :kind inf)` returned
    // empty. These tests drive the wiring per `temporal-model.md` § 5.4
    // + `read-protocol.md` § 3.1 (Inferentials keyed on `(s, p)` like
    // Semantic; re-derivation with same `(s, p)` + later `valid_at`
    // supersedes the prior, matching the Semantic rule).
    // ----------------------------------------------------------------

    /// A committed Inferential matching `:kind inf` must appear in the
    /// result set. Roadmap Phase 3.1 exit criterion: "for any committed
    /// Inferential, a matching `(query :kind inf)` returns it."
    ///
    /// We seed one `@alice @knows @bob` Sem (so `@alice @knows` exists
    /// as bound symbols that the Inf can reference as a parent) and
    /// commit an Inferential `(inf @alice @friend_of @bob (@__mem_0)
    /// @citation_link :c 0.7 :v 2024-03-15)`.
    #[test]
    fn inf_kind_query_returns_committed_inf() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        compile(
            &mut pipe,
            "(inf @alice @friend_of @bob (@__mem_0) @citation_link \
             :c 0.7 :v 2024-03-15)",
        );
        let got = pipe.execute_query("(query :kind inf)").expect("query");
        assert_eq!(
            got.records.len(),
            1,
            "expected the committed inferential to be returned; \
             got {} records: {:?}",
            got.records.len(),
            got.records,
        );
        let CanonicalRecord::Inf(inf) = &got.records[0] else {
            panic!("expected Inf record, got {:?}", got.records[0]);
        };
        let alice = pipe.table().lookup("alice").expect("alice bound");
        let friend_of = pipe.table().lookup("friend_of").expect("friend_of bound");
        assert_eq!(inf.s, alice);
        assert_eq!(inf.p, friend_of);
    }

    /// `:s` / `:p` predicates filter Inferentials by subject /
    /// predicate — same semantics as for Sem per read-protocol.md
    /// § 4.1.
    #[test]
    fn inf_sp_query_filters_by_subject_predicate() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        compile(
            &mut pipe,
            "(inf @alice @friend_of @bob (@__mem_0) @citation_link \
             :c 0.7 :v 2024-03-15)",
        );
        compile(
            &mut pipe,
            "(inf @alice @colleague_of @dave (@__mem_0) @citation_link \
             :c 0.7 :v 2024-03-15)",
        );
        let got = pipe
            .execute_query("(query :kind inf :s @alice :p @friend_of)")
            .expect("query");
        assert_eq!(
            got.records.len(),
            1,
            ":s @alice :p @friend_of must match exactly one Inf",
        );
        let CanonicalRecord::Inf(inf) = &got.records[0] else {
            panic!("expected Inf record");
        };
        let friend_of = pipe.table().lookup("friend_of").unwrap();
        assert_eq!(inf.p, friend_of);
    }

    /// A bare `(query)` without `:kind` must return Inferentials too
    /// (in addition to Sem / Pro). "All memory types end-to-end"
    /// is the Phase 3.1 deliverable.
    #[test]
    fn bare_query_includes_inferentials() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        compile(
            &mut pipe,
            "(inf @alice @friend_of @bob (@__mem_0) @citation_link \
             :c 0.7 :v 2024-03-15)",
        );
        let got = pipe.execute_query("(query)").expect("query");
        let has_sem = got
            .records
            .iter()
            .any(|r| matches!(r, CanonicalRecord::Sem(_)));
        let has_inf = got
            .records
            .iter()
            .any(|r| matches!(r, CanonicalRecord::Inf(_)));
        assert!(has_sem, "bare query must include Sem records");
        assert!(has_inf, "bare query must include Inf records");
    }

    /// Two Inferentials at the same `(s, p)` with the same `valid_at`
    /// are a conflict under the single-writer invariant (same rule as
    /// Sem § 5.1 per temporal-model.md § 5.4). Verify the emit path
    /// rejects it rather than silently overwriting.
    #[test]
    fn inf_same_sp_same_valid_at_is_conflict() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        compile(
            &mut pipe,
            "(inf @alice @friend_of @bob (@__mem_0) @citation_link \
             :c 0.7 :v 2024-01-15)",
        );
        let err = pipe
            .compile_batch(
                "(inf @alice @friend_of @carol (@__mem_0) @citation_link \
                 :c 0.8 :v 2024-01-15)",
                now(),
            )
            .expect_err("identical (s, p, valid_at) must conflict");
        assert!(
            matches!(
                err,
                crate::pipeline::PipelineError::Emit(
                    crate::pipeline::EmitError::InferentialSupersessionConflict { .. }
                )
            ),
            "expected InferentialSupersessionConflict; got {err:?}",
        );
    }

    /// Re-derivation with the same `(s, p)` and a later `valid_at`
    /// supersedes the prior Inferential — mirroring Semantic § 5.1 per
    /// temporal-model.md § 5.4's "auto-supersession rule as if
    /// Inferential were Semantic." Only the newer record appears in
    /// the current-state result.
    #[test]
    fn inf_re_derivation_supersedes_earlier_inf() {
        let mut pipe = Pipeline::new();
        compile(&mut pipe, SEM_ALICE);
        compile(
            &mut pipe,
            "(inf @alice @friend_of @bob (@__mem_0) @citation_link \
             :c 0.7 :v 2024-01-15)",
        );
        compile(
            &mut pipe,
            "(inf @alice @friend_of @carol (@__mem_0) @citation_link \
             :c 0.9 :v 2024-03-15)",
        );
        let got = pipe.execute_query("(query :kind inf)").expect("query");
        assert_eq!(
            got.records.len(),
            1,
            "later valid_at re-derivation must supersede earlier Inf; \
             current-state query should return only one record. Got: {:?}",
            got.records,
        );
        let CanonicalRecord::Inf(inf) = &got.records[0] else {
            panic!("expected Inf");
        };
        let carol = pipe.table().lookup("carol").expect("carol bound");
        assert!(
            matches!(&inf.o, crate::Value::Symbol(id) if *id == carol),
            "expected the later-valid_at (carol) record; got {:?}",
            inf.o,
        );
    }
}
