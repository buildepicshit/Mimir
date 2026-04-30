# Temporal Model

> **Status: authoritative 2026-04-19.** Graduated from `citation-verified` on 2026-04-19. All four citations (Graphiti, SQL:2011, Snodgrass, Lamport) are verified per `docs/attribution.md`. Code evidence: `ClockTime` newtype (6.1), monotonic `committed_at` watermark with property coverage (6.1), supersession DAG with acyclicity invariant + property coverage (6.2), Semantic + Procedural auto-supersession (6.3a/b), the bi-temporal as-of resolver with property coverage on a generated supersession chain (6.4), and Inferential staling write-path (§ 5.4) — reverse-parent index on `Pipeline`, retroactive `StaleParent` edge emission on every auto-supersession, and the write-time `flags.stale` check against an already-superseded parent. Inferential resolver (§ 5.4 auto-supersession on re-derivation + `:kind inf` reads) wired in Phase 3.1 (`resolver::resolve_inferential`, `Pipeline::inferential_records`, `Pipeline::inferential_history_at`, `EmitError::InferentialSupersessionConflict`). All in `mimir_core`. The *read-time* stale-flag overlay — surfacing `StaleParent` edges as a distinct read flag — is deferred pending a `read-protocol.md` § 5 `ReadFlags` amendment to allocate a bit for it; the resolver's definition of authoritativeness is unchanged by the overlay (stale Inferentials remain authoritative).

Mimir's canonical store is bi-temporal and append-only. Every memory carries four clocks; supersession happens through edge invalidation rather than in-place overwrite. This specification defines the clocks, their assigners, the supersession rules per memory type, and the read-resolution semantics.

## 1. Scope

This specification defines:

- The four clocks (`valid_at`, `invalid_at`, `committed_at`, `observed_at`).
- Per-memory-type clock assignment (who provides each clock; reconciles with `memory-type-taxonomy.md` which surfaces only agent-provided clocks).
- Bi-temporal edge invalidation as the supersession mechanism.
- Per-memory-type supersession rules.
- Read resolution: current-state queries and as-of-time queries.
- Clock source and representation.
- Future-dated validity and the `projected` flag.

This specification does **not** define:

- Memory type shapes — `memory-type-taxonomy.md`.
- Write-path checkpoint, episode atomicity, or WAL mechanics — `write-protocol.md`.
- Read-path hot-path vs escalation — `read-protocol.md` (this spec defines resolution *semantics*; that spec defines *protocol*).

**Out of scope (anti-goals).** Concurrent-writer reconciliation, SSI rw-antidependency detection, and DAG merge across concurrent writes. Each workspace has a single writer; there is nothing to reconcile.

### Graduation criteria

Graduates draft → authoritative when:

1. Graphiti (arXiv:2501.13956) and SQL:2011 temporal features are verified in `docs/attribution.md`.
2. A Rust `ClockTime` newtype + clock-assignment logic compiles in `mimir_core`, with the invariants in § 12 covered by unit and property tests.
3. Property tests exist for: monotonic `committed_at`, supersession DAG acyclicity, as-of query correctness on a generated supersession chain, future-validity rejection without `projected`.

## 2. Design thesis: bi-temporal identity, append-only truth

Mimir's store is append-only (PRINCIPLES.md architectural boundary #3). In-place overwrite is forbidden; every change appends. The question becomes: what happens when a fact changes? A semantic memory says "Alice's email is `x@…`"; six months later she changes it to `y@…`. Both facts must be true of the record — *y* is true *now*, *x* was true *earlier* — without either overwriting the other.

Bi-temporal modelling is the answer. Every memory carries:

- A **valid-time** interval `[valid_at, invalid_at)` — when the fact was true in the world.
- A **transaction-time** `committed_at` — when the librarian durably committed the memory.
- An **observation-time** `observed_at` — when the recording agent observed the fact (the agent's wall-clock-at-write; for Episodic, this is distinct from `at_time`).

Supersession is achieved by setting the prior memory's `invalid_at` — an *edge* operation that preserves the prior memory intact but marks its validity as ending. Reads that specify "what was true at T?" walk back through these edges to reconstruct point-in-time truth.

This model is drawn most directly from **Graphiti** (Rasmussen et al., arXiv:2501.13956 — verified; see `docs/attribution.md`), which implements a bi-temporal knowledge graph with four timestamps per edge: `t'_created` and `t'_expired` on the transactional timeline `T'`, plus `t_valid` and `t_invalid` on the valid-time timeline `T`. Graphiti states this as "a novel advancement in LLM-based knowledge graph construction" (§ 2.1). SQL:2011 temporal tables (application-time + system-time periods, pending verification) are the broader database-theory antecedent.

**Mimir's four clocks are a variant of Graphiti's four-timestamp model**, not a novel construction:

| Graphiti | Mimir | Notes |
|---|---|---|
| `t'_created` | `committed_at` | when the librarian durably committed the record |
| `t'_expired` | *(no direct analog)* | Mimir records invalidation via a separate `SUPERSEDES` edge in the canonical log, preserving the prior record untouched |
| `t_valid` | `valid_at` | when the fact held true in the world |
| `t_invalid` | `invalid_at` | when the fact stopped being true |
| *(no analog)* | `observed_at` | agent's wall-clock at write (Episodic) — distinct from `at_time` / `valid_at` |

Mimir's variations — substituting `observed_at` for `t'_expired`, and using append-only supersession edges rather than in-place edge mutation — are driven by (a) the Episodic memory type's need to distinguish event-time from observation-time, and (b) PRINCIPLES.md architectural boundary #3 (append-only canonical store, no in-place overwrite). A further divergence from Graphiti: Mimir uses **deterministic supersession rules** (§ 5), whereas Graphiti uses an LLM to "compare new edges against semantically related existing edges to identify potential contradictions" (§ 2.2.3). Mimir's `librarian-pipeline.md` § 7 allows ML-proposed supersession candidates but always wraps them in a deterministic commit decision per `PRINCIPLES.md` § 4.

## 3. The four clocks

### 3.1 Definitions

| Clock | Meaning | Type | Nullable |
|---|---|---|---|
| `valid_at` | When the fact becomes true in the world | `ClockTime` | No |
| `invalid_at` | When the fact stopped being true (set at supersession) | `Option<ClockTime>` | Yes — `None` while the fact is current |
| `committed_at` | When the librarian durably committed the memory | `ClockTime` | No |
| `observed_at` | When the memory was observed — agent's wall clock at write (Episodic only) or `= committed_at` (other types) | `ClockTime` | No |

All clocks use `ClockTime(u64)` — milliseconds since Unix epoch UTC. See § 10.

### 3.2 Who assigns each clock

| Clock | Assigner | Notes |
|---|---|---|
| `valid_at` | Agent (Semantic / Inferential / Episodic-as-`at_time`). Librarian assigns `= committed_at` for Procedural (no semantic notion of "when true") | For Episodic, the write surface field is `at_time`; the librarian stores it as `valid_at` internally. |
| `invalid_at` | Librarian only; set at supersession time | Agents cannot set this directly. Setting it requires a write that supersedes a prior memory. |
| `committed_at` | Librarian; monotonic per workspace | Agent cannot provide. Blocks the write until the clock advances past the previous `committed_at`. |
| `observed_at` | Agent for Episodic; librarian `= committed_at` for all other types | For Episodic: agent's wall clock when they observed the event, distinct from `at_time` (when the event occurred). |

### 3.3 Reconciliation with `memory-type-taxonomy.md`

Memory-type-taxonomy surfaced only the *agent-visible* temporal fields per type:

- Semantic: `valid_at`
- Episodic: `at_time`, `observed_at`
- Procedural: (none — librarian assigns all three active clocks)
- Inferential: `valid_at`

At the librarian / canonical-form level, **all four clocks exist on every memory**. The agent-facing write surface just expects the subset each type actively provides; the librarian fills the rest.

Implementation: the canonical-form struct carries all four clocks as mandatory fields (`invalid_at` is `Option`). The write surface grammar per `ir-write-surface.md` parses only the agent-exposed subset and the librarian completes the record at bind.

## 4. Per-memory-type clock assignment

### 4.1 Semantic

```
agent provides: valid_at
librarian assigns: committed_at, observed_at (= committed_at)
invalid_at: None until superseded
```

### 4.2 Episodic

```
agent provides: at_time (stored as valid_at), observed_at
librarian assigns: committed_at
invalid_at: None (Episodic does not auto-supersede; see § 5)
validation: observed_at >= at_time
```

### 4.3 Procedural

```
agent provides: (no temporal fields)
librarian assigns: valid_at = committed_at = observed_at = T(commit)
invalid_at: None until superseded
```

### 4.4 Inferential

```
agent provides: valid_at
librarian assigns: committed_at, observed_at (= committed_at)
invalid_at: None until parent supersession triggers stale flag;
           does NOT auto-close (see § 5)
```

## 5. Supersession rules per memory type

Supersession is the act of ending a memory's validity. The librarian sets the prior memory's `invalid_at` to a timestamp; the memory itself is untouched.

### 5.1 Semantic — auto-supersede on `(s, p)` with later `valid_at`

When a new Semantic memory `M_new` is committed with the same `(s, p)` as an existing Semantic memory `M_old` and `M_new.valid_at > M_old.valid_at`:

- Librarian sets `M_old.invalid_at = M_new.valid_at`.
- A supersession edge is recorded in the DAG: `M_new supersedes M_old`.

If `M_new.valid_at < M_old.valid_at` (a retroactive correction asserting an earlier validity period), the handling is:

- `M_new.invalid_at` is set to `M_old.valid_at` (the new memory is valid only for the period up to when M_old took over).
- A supersession edge still records `M_new ≺ M_old` by `valid_at` ordering; edge direction in the DAG reflects retroactivity.

Equality (`M_new.valid_at == M_old.valid_at`) is rejected with `PipelineError::SemanticSupersessionConflict` — two memories claiming the same validity start against the same `(s, p)` cannot both be authoritative under the single-writer invariant. The writer re-batches with a distinct `valid_at`.

### 5.2 Procedural — auto-supersede on `rule_id` or `(trigger, scope)`

When a new Procedural memory `P_new` has the same `rule_id` as an existing `P_old`, or the same `(trigger, scope)` pair:

- Librarian sets `P_old.invalid_at = P_new.committed_at`.
- Supersession edge recorded: `P_new supersedes P_old`.

### 5.3 Episodic — no auto-supersession

Episodic memories do not supersede each other. Two Episodic memories about "what happened at T" can both be retained; they are separate observations, potentially from different witnesses.

Corrections are **explicit**: the write surface accepts a `(correct @target_ep)` form that emits a new Episodic memory marked as correcting `@target_ep`. The librarian records the correction edge; it does **not** set `@target_ep.invalid_at` — the original observation is preserved as-is, and readers see both linked by the correction edge.

Rationale: events are evidence, not fact. Competing eyewitness accounts must both survive.

Exact grammar and semantics of `(correct ...)` are in `write-protocol.md`.

### 5.4 Inferential — no auto-supersession; stale flag on parent change

> **Implementation status:** the reverse parent index
> (`Pipeline::inferentials_by_parent`), `StaleParent` edge
> emission on retroactive propagation, and write-time `flags.stale`
> detection (Inferential born from an already-superseded parent)
> are live. The Inferential resolver wired in Phase 3.1 — the
> re-derivation supersession rule below is enforced at emit time
> (`resolve_inferential_supersession` + `EmitError::InferentialSupersessionConflict`),
> and `:kind inf` queries now return authoritative Inferentials
> via `resolver::resolve_inferential`. The *runtime stale-flag
> overlay* — surfacing `StaleParent` edges as a distinct read
> flag — remains deferred pending a `read-protocol.md` § 5
> `ReadFlags` amendment to allocate a bit for it; the resolver's
> definition of authoritativeness is unchanged by the overlay.

When any parent of an Inferential memory is superseded, the Inferential memory is flagged *stale* (per `memory-type-taxonomy.md` § 3.4):

- `invalid_at` is **not** auto-set. The Inferential's conclusion may or may not still be true; re-derivation is required to decide.
- A stale-parent edge is recorded.
- Re-derivation is explicit (agent-requested or librarian consolidation pass); it produces a new Inferential memory that either supersedes the stale one (auto-supersession rule as if Inferential were Semantic — same `(s, p)` later `valid_at`; enforced at emit time in Phase 3.1 via `resolve_inferential_supersession`, mirroring `resolve_semantic_supersession`) or confirms the original (emitting a `@reconfirm` Episodic event).

## 6. Bi-temporal edge invalidation algorithm

### 6.1 Data structure

Each workspace's supersession graph is a DAG of memories connected by typed edges:

```rust
pub enum SupersessionEdge {
    Supersedes { from: MemoryId, to: MemoryId, at: ClockTime },
    Corrects  { from: MemoryId, to: MemoryId, at: ClockTime },  // Episodic only
    StaleParent { from: MemoryId, to: MemoryId, at: ClockTime }, // Inferential only
    Reconfirms { from: MemoryId, to: MemoryId, at: ClockTime },  // Inferential only
}
```

The DAG is persisted in the workspace's `dag.snapshot` (per `workspace-model.md` § 4.2), updated via WAL.

### 6.2 Invariants

1. **Acyclicity.** Union of all edges forms a DAG. Cycle-introducing writes are rejected with `BindError::SupersessionCycle`.
2. **Source precedes target in transaction time.** For any edge, `from.committed_at >= to.committed_at`. A supersession cannot predate its target's commit.
3. **Supersedes edges close validity.** For every `Supersedes { from, to }`, one side's `invalid_at` is set. The invariant is direction-sensitive:
    - **Forward case** (`from.valid_at > to.valid_at` — § 5.1): `to.invalid_at = from.valid_at`. The older memory's validity closes at the newer memory's validity start.
    - **Retroactive case** (`from.valid_at < to.valid_at` — § 5.1 backward): `from.invalid_at = to.valid_at`. The NEW memory's validity closes at the (already-current) older memory's validity start; the older memory stays valid. This is an asymmetry relative to the forward case — the `invalid_at` sits on the *source* of the edge, not the target — and reflects append-only semantics: we cannot retroactively mutate the already-committed older memory. The read-time resolver (§ 7) compensates by reading `invalid_at` from the record itself rather than deriving it uniformly from edges.

    Other edge types (`Corrects`, `StaleParent`, `Reconfirms`) do not close validity.

### 6.3 Algorithm (on write)

```
commit(memory M):
    assign_librarian_clocks(M)
    detect_supersession_targets(M) -> [M_old_1, M_old_2, …]
    for each M_old:
        match M.kind, M_old.kind:
            Semantic-Semantic matching (s, p), Procedural-Procedural matching rule/trigger:
                set M_old.invalid_at = M.valid_at (or M.committed_at for Procedural)
                emit Supersedes edge
            else:
                skip (no auto-supersession for Episodic/Inferential)
    if M.kind is Inferential and any parent was superseded:
        mark M stale; emit StaleParent edge
    append M to canonical log
    durable flush
```

`detect_supersession_targets` is a deterministic lookup against the workspace's current-state index (memories whose `invalid_at is None`).

## 7. Read resolution

### 7.1 Current-state query (default)

```
query(...)  # default as_of = now
```

Returns memories where `invalid_at is None` (current truth). This is the hot-path read per `read-protocol.md`.

### 7.2 As-of-time query

```
query(..., as_of: ClockTime)
```

Returns memories where:

```
valid_at ≤ as_of
AND (invalid_at > as_of OR invalid_at is None)
AND committed_at ≤ query_time
```

This is standard bi-temporal semantics (compatible with SQL:2011's `AS OF SYSTEM TIME` / period-predicate queries). The query treats the store as it appeared at `as_of`.

### 7.3 Transaction-time query

```
query(..., as_committed: ClockTime)
```

Returns the state of the canonical store as it existed at `as_committed` — what the librarian knew then, regardless of what the agent later asserted was valid. Useful for audit and reproducibility.

### 7.4 Retroactive-correction-aware queries

When a Semantic memory has been retroactively superseded (§ 5.1), as-of queries reflect the retroactive correction only for timestamps after the correction's `committed_at`. A query with `as_of = T0` and `as_committed = T1 < T0 < correction.committed_at` returns the pre-correction view; same query with `as_committed > correction.committed_at` returns the corrected view.

Formal semantics: a two-dimensional query space `(valid_at, committed_at)` — bi-temporal. The hot path uses `(now, now)`; reproducible audits use explicit pairs.

## 8. Single-writer DAG invariants

Each workspace has a single writer, so there is no concurrent-write reconciliation. The invariants on the supersession DAG are purely structural:

- Supersession edges form a DAG (see § 6.2).
- Two memories at the same `(s, p)` claiming identical `valid_at` are rejected at emit time with `PipelineError::SemanticSupersessionConflict`; the writer chooses a distinct `valid_at` and re-batches.
- Cross-workspace supersession does not exist — workspaces do not share memories.

## 9. Clock source and representation

### 9.1 `ClockTime`

```rust
pub struct ClockTime(u64);  // milliseconds since Unix epoch, UTC
```

- `u64` capacity exceeds year 584,000,000 — far past any Mimir-relevant horizon.
- Millisecond precision only in v1. Nanosecond is out of scope.
- Timezone is UTC exclusively. Agent-provided times must be UTC; local-time grammar is rejected.

### 9.2 `committed_at` is monotonic per workspace

`committed_at` is assigned by the librarian from a monotonic clock:

```
committed_at = max(wall_clock_now(), previous_committed_at + 1)
```

If wall clock regresses (NTP correction, VM clock jump backward), the librarian advances using the monotonic rule. A committed memory's `committed_at` never appears before any prior memory's `committed_at` in the same workspace.

### 9.3 Agent-provided clock validation

`valid_at`, `at_time`, and `observed_at` are agent-provided and validated:

- Must parse as a `ClockTime` (millis-since-epoch or ISO-8601-UTC that converts).
- `observed_at >= at_time` for Episodic.
- For Semantic / Inferential: if `valid_at > now`, require the `projected: true` flag (§ 10); otherwise `BindError::FutureValidity`.

## 10. Future-dated validity: the `projected` flag

By default, a write with `valid_at > now` is rejected. Rationale: future-dated writes are usually bugs (agent clock skew, client-side time drift, stale-state errors); the rule catches them cheaply.

Explicit projections — assertions of intent about future state — are allowed with an explicit flag:

```
(sem @mimir target_release @v0_1 :src @agent_instruction :c 0.7 :v 2026-12-31 :projected true)
```

`projected: true` means:

- `valid_at > committed_at` is allowed.
- The memory is retained as-is; it's a projection, not a hallucination.
- At read time, memories with `projected: true` carry a `projected` flag in the result so agents can treat them differently (e.g., skip them in current-truth queries).

Projections are uncommon but legitimate (plans, commitments, schedules). Requiring explicit declaration makes them auditable.

## 11. Interactions with other specs

- **Write protocol:** bind-time clock assignment, supersession detection, WAL durability — `write-protocol.md`.
- **Read protocol:** hot-path vs as-of resolution, stale-symbol escalation, projection filtering — `read-protocol.md`.
- **Memory type taxonomy:** agent-visible temporal fields per type — `memory-type-taxonomy.md`.
- **Canonical form:** on-disk layout of the four clocks + DAG — `ir-canonical-form.md`.

## 12. Invariants

1. **Monotonic `committed_at`.** For any two memories `A`, `B` in the same workspace, if `A` was committed before `B`, then `A.committed_at < B.committed_at`. Enforced at bind.
2. **Agent-provided clock validation.** `observed_at >= at_time` (Episodic). `valid_at <= now` unless `projected: true`. UTC only. `BindError::FutureValidity` / `BindError::InvalidClockOrder` on violation.
3. **Supersession DAG acyclicity.** Edges form a DAG. `BindError::SupersessionCycle` on violation.
4. **Supersedes closes validity.** Every `Supersedes { from, to }` edge implies `to.invalid_at = from.valid_at`. Librarian-enforced at supersession time.
5. **`invalid_at` is librarian-assigned only.** Agents cannot set `invalid_at` directly. Any wire value in the `invalid_at` field is ignored; the librarian assigns based on supersession rules.
6. **No retroactive commit-time.** `committed_at` is the librarian's current monotonic value; agents cannot back-date commits.

## 13. Open questions and non-goals for v1

### 13.1 Open questions

**Sub-millisecond precision.** `ClockTime` is milliseconds. If real workloads show concurrent commits hitting the same millisecond and contending via the `+1` monotonic-bump rule at high rate, revisit — switch to microseconds or nanoseconds. Post-MVP.

**Logical clocks vs wall clocks.** `committed_at` is wall-clock-derived with monotonic enforcement. A pure logical clock (Lamport / vector) would be more precise about ordering under clock skew but less useful for human audit. Defer.

**Timezone ingestion.** Agents provide UTC only. Local-time ingestion + normalization at the librarian is a convenience feature post-MVP.

### 13.2 Non-goals for v1

- **Sub-second `valid_at` granularity in agent reasoning.** Agents may emit sub-second `valid_at`, but Mimir makes no promises about distinguishing events <1 second apart for ordering purposes.
- **Automatic clock correction.** If an agent's clock is wrong, the librarian does not correct it beyond rejecting future-dated writes. Agents must keep their clocks reasonably accurate.
- **Arbitrary-dimensional temporal queries.** Only bi-temporal (`valid_at`, `committed_at`) queries are supported. N-temporal (multiple validity dimensions) is out of scope.
- **Calendar-aware time.** No handling of timezones, daylight-saving transitions, leap seconds. UTC ms-since-epoch only.

## 14. Primary-source attribution

All entries are **pending verification** per `docs/attribution.md`. Bi-temporal modelling is a well-studied area; Mimir's specific choices (four clocks, per-type supersession rules, DAG over edges) draw from this literature.

- **Graphiti** (Rasmussen et al., arXiv:2501.13956; ✓ verified 2026-04-17, see `docs/attribution.md` § Verified sources) — bi-temporal invalidation of graph edges with four timestamps per edge. Directly informs § 2 design thesis, § 3 four-clock model, and § 5–6 supersession mechanics. Mimir's four-clock model is a variant of Graphiti's four-timestamp model (substituting `observed_at` for `t'_expired`, using append-only supersession edges rather than in-place mutation, and using deterministic supersession rules rather than LLM-based contradiction detection — see § 2).
- **SQL:2011 temporal features** ([ISO/IEC 9075:2011 Part 2, pending](https://www.iso.org/standard/53682.html)) — application-time + system-time period tables, `AS OF SYSTEM TIME` queries. Cited for § 7 query semantics.
- **Snodgrass, *Developing Time-Oriented Database Applications in SQL*** (1999, pending) — canonical text on bi-temporal database design, valid-time vs transaction-time distinction.
- **Lamport, *Time, Clocks, and the Ordering of Events in a Distributed System*** (1978, pending) — foundational text on logical vs wall clocks, referenced for § 9.2 monotonicity discussion.
