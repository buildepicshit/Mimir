# Read Protocol

> **Status: authoritative — graduated 2026-04-18; scope amended 2026-04-24.** Scope reduced 2026-04-18 — raw cross-workspace reads, `CONFLICT` flag (SSI), and `CONTESTED` flag (multi-writer conflict surface) were removed from this workspace-local read protocol. Mimir's current workspace store is an isolation boundary; each workspace has a single writer and no implicit inter-workspace reads. The 2026-04-24 mandate adds a separate draft `scope-model.md` for explicit cross-scope retrieval after librarian-governed promotion. Graduation evidence: `mimir_core::read::Pipeline::execute_query` implements the grammar + flags + framing + filtered-array surface (7.1–7.3); `Pipeline::semantic_by_sp_history` + `procedural_by_rule_history` implement the § 3.1 current-state index; `resolver::resolve_semantic` and `resolve_procedural` consult the index for O(k) lookup in the `(s, p)` or `rule_id` bucket instead of scanning; 7.4 property tests cover criterion #3 (snapshot isolation, STALE_SYMBOL propagation, as-of + retroactive supersession); the `benches/read_path.rs` criterion bench measures p50 ≈ **0.57 µs** for a single-predicate Semantic lookup against a 1 M-memory warm index, clearing criterion #4's p50 < 1 ms target by ~1,750×. Inferential reads (`:kind inf`) wired in Phase 3.1: `resolver::resolve_inferential` + `Pipeline::inferential_records` + `Pipeline::inferential_history_at` deliver the same `(s, p)`-keyed lookup semantics the § 3.1 `inferential` index specifies, and `EmitError::InferentialSupersessionConflict` enforces the § 5.4 re-derivation rule at write time. The read-time stale-flag overlay for `StaleParent` edges is a follow-up pending a § 5 `ReadFlags` amendment to allocate a dedicated bit for it.

This specification defines how a Claude instance reads from its workspace's canonical store. It implements PRINCIPLES.md architectural boundary #2 (bifurcated reads): the agent reads canonical state directly on the hot path, escalating to the librarian only when result flags indicate fuller context is needed.

## 1. Scope

This specification defines:

- The current-state index structure maintained per workspace.
- The hot-path query grammar.
- Result shape — records + flags bitset.
- Escalation triggers and the librarian `inspect` API.
- Read consistency (snapshot isolation at query time).
- As-of and as-committed temporal query semantics (pointers to `temporal-model.md`).
- Performance targets.

This specification does **not** define:

- Memory type shapes — `memory-type-taxonomy.md`.
- Workspace identity and partitioning — `workspace-model.md`.
- Symbol resolution — `symbol-identity-semantics.md`.
- Grounding source taxonomy — `grounding-model.md`.
- The four-clock bi-temporal model — `temporal-model.md`.
- The IR write-surface grammar (query is part of the surface per `ir-write-surface.md` § 5.10).
- Librarian pipeline (reads do not go through Bind / Semantic / Emit; they read bound state directly).
- Agent API contract — `wire-architecture.md` (v1 is synchronous in-process; `Pipeline::execute_query` is the read entry point).

**Out of scope for this protocol.** Raw cross-workspace reads, multi-writer conflict detection (SSI), and multi-agent coordination protocols are not part of the workspace-local read path. Cross-scope recall is a future scope-aware read mode defined by [`scope-model.md`](scope-model.md), and only for promoted records.

### Graduation criteria

Graduates draft → authoritative when:

1. Related art is verified in `docs/attribution.md` (SQL snapshot isolation, MVCC literature).
2. Rust reader + index + escalation API compile in `mimir_core`, with the invariants in § 13 covered by unit, property, and load tests.
3. Property tests cover: snapshot isolation (a reader does not see writes committed after the reader's query-start time); stale-symbol flag propagation; as-of queries respect retroactive-supersession edges.
4. Load tests confirm p50 < 1 ms for single-predicate Semantic lookups against a 1M-memory warm index.

## 2. Design thesis: bifurcated reads

Agents read from Mimir constantly. Reads on the hot path must be cheap and deterministic; adding a librarian round-trip to every read would kill the latency budget.

PRINCIPLES.md architectural boundary #2 defines the bifurcation: **agents read canonical directly on the hot path; escalate to the librarian on conflict, low confidence, or stale-symbol flag.**

Two consequences:

- **The hot path is a library call, not an IPC.** The agent links against `mimir_core::read` and queries an in-memory index that is always current as of the last committed CHECKPOINT. No network, no marshalling, no librarian process involvement on a clean read.
- **Escalation is explicit.** The agent sees flags in its result; it decides whether to escalate. The librarian's `inspect` API returns full audit trails (supersession chain, decay history, provenance) — richer than the hot path, but only when asked.

This structure matches the determinism-over-speed principle (`PRINCIPLES.md` § 1) by making the common case fast and the expensive-inspection case opt-in.

## 3. Current-state index

Every workspace maintains an in-memory index of un-superseded memories. The index is rebuilt from `canonical.log` at startup and updated incrementally on every CHECKPOINT.

### 3.1 Index structure

```rust
pub struct CurrentStateIndex {
    // Semantic memories — keyed by (subject, predicate)
    semantic: BTreeMap<(SymbolId, SymbolId), MemoryId>,

    // Procedural memories — keyed by rule_id, and secondarily by (trigger, scope)
    procedural_by_rule: BTreeMap<SymbolId, MemoryId>,
    procedural_by_trigger_scope: BTreeMap<(Value, SymbolId), MemoryId>,

    // Episodic — no natural unique key; flat vec + secondary indexes
    episodic: Vec<MemoryId>,
    episodic_by_kind: HashMap<SymbolId, Vec<MemoryId>>,
    episodic_by_participant: HashMap<SymbolId, Vec<MemoryId>>,

    // Inferential — keyed same as Semantic on (s, p)
    inferential: BTreeMap<(SymbolId, SymbolId), MemoryId>,
    inferential_by_method: HashMap<SymbolId, Vec<MemoryId>>,
}
```

The index stores `MemoryId` only (not full records). Record bodies are fetched from `canonical.log` on demand (mmap or pread — a cheap seek on SSD).

### 3.2 Updates

On every committed CHECKPOINT:

1. For each new memory in the batch, add its entry to the relevant index.
2. For each supersession edge, remove the prior memory from the current-state index (it is no longer current). The memory stays in `canonical.log`; only its index entry is dropped.
3. For each `SYMBOL_RETIRE` record, mark the symbol in a separate `retired: HashSet<SymbolId>` used for stale-symbol flag detection (§ 7).

Updates are applied after CHECKPOINT fsync, matching the `write-protocol.md` § 4.3 "post-commit derived-state updates" flow. Readers at the moment just before the update may miss the newest Episode for a few microseconds; the snapshot-isolation rule (§ 9) handles this cleanly — a reader whose `query_committed_at` predates the new CHECKPOINT simply does not see it.

### 3.3 Startup rebuild

On workspace startup the index rebuilds from log replay (per `write-protocol.md` § 10). Time complexity is linear in the committed record count; for the 1M-memory target this completes within the 2-second cold-start target (per `PRINCIPLES.md` § 6).

## 4. Hot-path query grammar

The query form is part of the write-surface grammar (`ir-write-surface.md` § 5.10) but is issued on the read path.

```
(query :kind K?
       :s @S? :p @P? :o V?
       :in_episode @E? :after_episode @E? :before_episode @E? :episode_chain @E?
       :as_of T? :as_committed T?
       :include_retired false? :include_projected false?
       :confidence_threshold 0.5? :limit N?
       :explain_filtered false? :show_framing false? :debug_mode false?)
```

### 4.1 Predicate semantics

All predicates are **AND-combined**. An empty query (just `(query)`) returns every current-state memory in the workspace (bounded by `:limit`, default 1000, to prevent accidental workspace dumps).

| Predicate | Matches / effect |
|---|---|
| `:kind K` | memories of type `K ∈ {sem, epi, pro, inf}` |
| `:s @S` | memories where subject = `@S` (SEM / INF) |
| `:p @P` | memories where predicate = `@P` (SEM / INF) |
| `:o V` | memories where object = `V` (SEM / INF) |
| `:in_episode @E` | memories whose CHECKPOINT is `@E` |
| `:after_episode @E` | memories whose Episode's `committed_at` > `@E.committed_at` |
| `:before_episode @E` | memories whose Episode's `committed_at` < `@E.committed_at` |
| `:episode_chain @E` | `@E` and all ancestors via `parent_episode_id` |
| `:as_of T` | bi-temporal valid-at filter (see `temporal-model.md` § 7.2) |
| `:as_committed T` | bi-temporal transaction-time filter (see `temporal-model.md` § 7.3) |
| `:include_retired true` | include memories whose symbols are retired |
| `:include_projected true` | include memories with `projected: true` flag |
| `:confidence_threshold X` | override default 0.5 threshold for the `low_confidence` flag |
| `:limit N` | cap result set at N records |
| `:explain_filtered true` | surface filtered memories with `filter_reason` in a separate `filtered` array on the result |
| `:show_framing true` | attach `Framing` metadata to every result record |
| `:debug_mode true` | shorthand — enables `:explain_filtered` and `:show_framing` together |

### 4.2 Default values

If not provided:
- `:as_of = now`
- `:as_committed = now`
- `:include_retired = false`
- `:include_projected = false`
- `:confidence_threshold = 0.5`
- `:limit = 1000`
- `:explain_filtered = false` (workspace-overridable via `mimir.toml [read_defaults]`)
- `:show_framing = false` (workspace-overridable)
- `:debug_mode = false` (workspace-overridable)

These defaults make the hot-path query "current truth, non-retired, non-projected, silent-filtered, limit 1000" — the common case. Debug / investigation paths opt into surfacing via the toggles above or the workspace-level `debug_mode` default in `mimir.toml`. See `confidence-decay.md` § 11 for the user-sovereignty framing.

## 5. Result shape

```rust
pub struct ReadResult {
    pub records: Vec<BoundMemory>,
    pub filtered: Vec<FilteredMemory>,   // populated when :explain_filtered true (or workspace default); empty otherwise
    pub flags: ReadFlags,
    pub as_of: ClockTime,
    pub as_committed: ClockTime,
    pub query_committed_at: ClockTime,
}

bitflags! {
    pub struct ReadFlags: u32 {
        const STALE_SYMBOL            = 0b0000_0000_0000_0001;
        const LOW_CONFIDENCE          = 0b0000_0000_0000_0100;
        const PROJECTED_PRESENT       = 0b0000_0000_0000_1000;
        const TRUNCATED               = 0b0000_0000_0001_0000;
        const EXPLAIN_FILTERED_ACTIVE = 0b0000_0000_1000_0000;
    }
}
```

Bit 1 (previously `CONFLICT`), bit 5 (previously `CROSS_WORKSPACE`), and bit 6 (previously `CONTESTED`) are intentionally left **reserved** so the on-wire `u32` layout is stable; they stay clear in v1 and aren't reused.

`BoundMemory` carries the full canonical record with resolved symbols. When `:show_framing true` is active, each record's `framing` field is populated; otherwise it is `None`:

```rust
pub struct BoundMemory {
    pub memory_id: SymbolId,
    pub kind: MemoryKind,
    pub clocks: FourClocks,
    pub source_chain: SourceSummary,
    pub framing: Option<Framing>,   // Some(..) when :show_framing true or :debug_mode true
}

pub enum Framing {
    Advisory,                                      // normal case
    Historical,                                    // returned due to :as_of < now
    Authoritative { set_by: FramingSource },       // pinned or operator-authoritative (see confidence-decay.md §§ 7-8)
    Projected,                                     // :projected true memory
}

pub enum FramingSource {
    AgentPinned,        // agent-invokable (pin) flag set — confidence-decay § 7
    OperatorAuthoritative, // user-applied authoritative flag — confidence-decay § 8
    LibrarianAssignment,   // librarian-emitted (internal facts)
}

pub struct FilteredMemory {
    pub memory_id: SymbolId,
    pub effective_confidence: Confidence,
    pub filter_reason: FilterReason,
}

pub enum FilterReason {
    BelowConfidenceThreshold { threshold: Confidence },
    RetiredSymbolExcluded,
    ProjectedExcluded,
    OutsideAsOfWindow { valid_at: ClockTime, invalid_at: Option<ClockTime> },
}
```

`filtered` is populated only when `:explain_filtered true` (per-query) or the workspace's `read_defaults.explain_filtered = true` (mimir.toml). Otherwise it is empty — the default silent-filter UX. Filtered memories are always computable by the librarian; the toggle controls whether they surface.

## 6. Flag semantics

### 6.1 `STALE_SYMBOL`

Set when the result set includes any memory referencing a retired symbol (per `symbol-identity-semantics.md` § 8). Default behavior: retired-symbol memories are excluded from results (per § 4.2 default `:include_retired = false`). When set, it means the query explicitly asked to include them or the current-state index picked them up via an indirect reference that hasn't been filtered.

Agent response: escalate via `inspect(memory_id)` to get the rename / retirement history and decide whether to rename references in new writes.

### 6.2 `LOW_CONFIDENCE`

Set when any memory's confidence < `:confidence_threshold`. By default a 0.5 threshold; many memories below 0.5 typically indicate grounding issues or heavy decay.

Agent response: filter client-side, re-query with stricter threshold, or escalate for per-memory decay history.

### 6.3 `PROJECTED_PRESENT`

Set when at least one result has `projected: true` and the query included projections (`:include_projected true`). Always clear otherwise (projections excluded by default).

Agent response: treat projected memories as intent / plan, not as current truth.

### 6.4 `TRUNCATED`

Set when the result set exceeded `:limit` and was truncated. Agent should paginate if full results are needed.

### 6.5 `EXPLAIN_FILTERED_ACTIVE`

Set when the `filtered` array in `ReadResult` is populated — that is, when `:explain_filtered true` is active (per-query or workspace default). Diagnostic flag so agents can check whether the filtered surface is live without inspecting the array.

## 7. Stale-symbol flag mechanics

A result memory triggers the `STALE_SYMBOL` flag when any `SymbolId` it references (`s`, `p`, `o`, `source`, `derived_from`, `method`, `participants`, `event_id`, `rule_id`, `scope`, `location`) is in the workspace's `retired` set.

The check is fast: `retired: HashSet<SymbolId>` lookup per symbol in the record. For a 1M-memory store with thousands of retired symbols, the per-record cost is microseconds.

## 8. Escalation API — `inspect`

When a hot-path result flags require fuller context, the agent calls the librarian's `inspect` API:

```rust
pub struct InspectResult {
    pub memory: BoundMemory,
    pub supersession_chain: Vec<SupersessionStep>,   // chronological (oldest first)
    pub decay_history: Vec<(ClockTime, Confidence)>, // confidence snapshots over time
    pub grounding: GroundingChain,                   // full derived_from graph (Inferential) or source details
    pub episode_context: EpisodeSummary,             // which Episode, position in it, neighboring memories
    pub rename_history: Vec<RenameStep>,             // past names of any renamed symbols
}

pub struct SupersessionStep {
    pub edge_kind: SupersessionEdgeKind, // Supersedes | Corrects | StaleParent | Reconfirms
    pub from: MemoryId,
    pub to: MemoryId,
    pub at: ClockTime,
}
```

The escalation call goes through the same in-process `Pipeline::execute_query` entry point as any other read (`wire-architecture.md` § 3.2) and is resolved by reading additional state from `dag.snapshot`, `symbols.snapshot`, `episodes.log`, and confidence-decay state.

### 8.1 `inspect` is read-only

The escalation API does not mutate state. It reads deeper audit trails but commits nothing. No side effects.

### 8.2 Latency budget

Escalation is expensive by design — p50 target ~10 ms, p99 ~100 ms. Agents should use it selectively, driven by flags.

## 9. Read consistency: snapshot isolation at query start

A hot-path read captures `query_committed_at = last_committed_checkpoint.committed_at` at query start. The query sees:

- All memories with `committed_at ≤ query_committed_at`.
- All supersession edges with `committed_at ≤ query_committed_at`.

Writes committed after `query_committed_at` are invisible to this query, even if they commit during the query's execution. This is standard snapshot isolation (analogous to PostgreSQL's `REPEATABLE READ`).

### 9.1 No read locks

Snapshot isolation is implemented via index state + commit ordering. Readers do not take locks, and there is a single writer per workspace so no write-side contention exists either. This matches the LSM-tree pattern — append-only + MVCC-style read resolution.

## 10. Bi-temporal queries

`temporal-model.md` § 7 defines the bi-temporal query space. This spec's role is to expose those semantics on the wire:

- **`(query :as_of T)`** — return memories whose `[valid_at, invalid_at)` interval contains `T`. Default `T = now`.
- **`(query :as_committed T)`** — use the store state as it existed at transaction time `T`. Default `T = now`.
- **Both together** give full bi-temporal queries: "what did we know at `:as_committed T1` about what was true at `:as_of T2`?"

## 11. Performance targets

Per `PRINCIPLES.md` § 6 (targets are directional, not SLOs):

| Query shape | p50 | p99 | Notes |
|---|---|---|---|
| Single-predicate Semantic lookup `(query :s @a :p email)` | < 1 ms | < 5 ms | BTree lookup on warm index |
| Type-only scan `(query :kind sem :limit 100)` | < 5 ms | < 20 ms | Index iteration |
| Episode-scoped `(query :in_episode @E)` | < 5 ms | < 20 ms | Episode metadata lookup + log-range scan |
| Temporal as-of | < 10 ms | < 50 ms | Extra DAG traversal for invalidation check |
| Escalation `inspect` | < 10 ms | < 100 ms | Multiple snapshot reads + DAG walk |

Load tests are part of graduation criterion § 1.

## 12. Invariants

1. **Snapshot isolation.** A query's result set reflects state at `query_committed_at`, fixed at query start. Writes committed after `query_committed_at` are never visible to this query.
2. **Current-state index consistency.** For every memory in the current-state index, there is no committed `SUPERSEDES` edge pointing to it whose commit time ≤ `query_committed_at`.
3. **Retired-symbol filtering default.** Results never include retired-symbol memories unless `:include_retired true` is explicitly set.
4. **Projection filtering default.** Results never include `projected: true` memories unless `:include_projected true` is explicitly set.
5. **Escalation is read-only.** `inspect` does not mutate state.
6. **Flag monotonicity.** A result's flags are computed from the records it contains; setting a flag on a record whose memory changes after `query_committed_at` does not retroactively change already-returned flags.
7. **Reader non-blocking.** Readers take no locks against the single writer or themselves.
8. **Single-workspace boundary.** A workspace-local query only reads the workspace it was issued against. Cross-scope reads require a separate scope-aware query path and can only return promoted records.
9. **No silent information loss.** Filter reasons and framing metadata are always computable/recoverable via the read toggles (`:explain_filtered`, `:show_framing`, `:debug_mode`). Default behavior is silent; information is never discarded at query time — only suppressed from the output surface unless opted in.
10. **Silent-by-default contracting.** Without opt-in toggles or workspace overrides, reads return only the records that pass the active tolerances — matching the UX of Claude Code / ChatGPT / Letta / Mem0. Debug-mode paths are explicitly user-invoked.

## 13. Open questions and non-goals for v1

### 13.1 Open questions

**Index persistence.** The current-state index is rebuilt from `canonical.log` at startup. For very large stores (100M+ memories), this may exceed the 2-second cold-start target. Options: persistent index snapshots (additional file type, additional consistency rules) or lazy index population on first query. Defer until workload data shows the need.

**Query planning.** v1 executes queries via direct index lookups; there is no query planner choosing between candidate indexes. For queries with multiple predicates, the implementation picks the most-selective index (typically `(s, p)` if both are provided) and filters. A cost-based planner is a post-MVP optimization.

**Result streaming.** Large result sets block until fully assembled. Streaming results (returning records as they're found) is useful for huge scans. Post-MVP — requires wire-protocol support.

**Read-your-own-writes consistency.** A writer's own subsequent reads see its committed writes because `query_committed_at = now` reflects the latest CHECKPOINT. In-flight writes (before CHECKPOINT) are not visible. Is this the right semantic, or should there be a "writer-local view" that includes in-flight batch records? Defer; most agent workflows don't need pre-commit reads.

**Fuzzy matching / approximate queries.** v1 supports only exact-match predicates. Semantic-similarity queries (find memories "like" this one) are a post-MVP ML-powered extension that would live in the librarian pipeline's escalation path, not the hot path.

### 13.2 Non-goals for v1

- **Raw cross-workspace reads and multi-agent coordination.** Workspace-local reads only see their own state and never reconcile with another workspace. Scope-aware recall over promoted records is specified separately in `scope-model.md`.
- **SQL-compatible query language.** The query form is Lisp S-expr per `ir-write-surface.md`. No SQL frontend.
- **Write-through the read API.** Reads are strictly read-only; the escalation API does not mutate state.
- **Server-side result aggregation.** Aggregations (count, sum, group-by) are not supported in v1's query grammar. Agents can scan and aggregate client-side.
- **Real-time subscriptions.** No push notifications for committed Episodes or memory changes. Agents poll with `:as_committed :greater_than T` if they need monitoring. Post-MVP subscription API is a candidate.

## 14. Primary-source attribution

All entries are **pending verification** per `docs/attribution.md`.

- **PostgreSQL snapshot isolation / MVCC documentation** ([postgresql.org/docs/current/mvcc-intro.html](https://www.postgresql.org/docs/current/mvcc-intro.html), pending) — reference for the snapshot-isolation semantics in § 9. Note: Mimir uses snapshot isolation for read-of-in-flight-write ordering within a single writer, not for multi-writer conflict detection.
- **SQL:2011 temporal query semantics** (already pending, cited in Temporal Model) — informs § 10 as-of / as-committed queries.
- **LSM-tree read-path design** (O'Neil 1996, already pending) — informs § 3 index-over-append-log structure.
