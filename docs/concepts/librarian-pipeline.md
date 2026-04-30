# Librarian Pipeline

> **Status: authoritative 2026-04-18.** Graduated from `citation-verified` on 2026-04-18 backed by `mimir_core::pipeline::Pipeline::compile_batch` (five-stage wiring, clone-on-write batch atomicity per invariant § 11.3, byte-for-byte determinism per invariant § 11.2) and `mimir_core::inference_methods::InferenceMethod` (14-method registry with deterministic integer fixed-point confidence formulas and per-method doctests per § 5 and § 1 graduation criterion #4). Parent-count cap of 8 on methods that compute a joint product (`@pattern_summarize`, `@architectural_chain`, `@provenance_chain`, `@analogy_inference`, `@multi_source_consensus`) is a bounded-determinism caveat pending a follow-up log-table implementation for unbounded N; all practical v1 inference chains fall under the cap.

The librarian pipeline compiles agent-emitted Lisp S-expressions into canonical-form records. It is the Roslyn-analog of Mimir — lexer, parser, binder, semantic analyzer, emitter — running as a single-writer, compile-style pipeline per PRINCIPLES.md architectural boundary #4. This specification defines the pipeline stages, the inference-method registry, and the boundary between deterministic operations and ML-proposed candidates.

## 1. Scope

This specification defines:

- The five pipeline stages (Lex, Parse, Bind, Semantic, Emit) with per-stage inputs, outputs, typed errors, and determinism status.
- The inference-method registry for Inferential memory derivations.
- The ML-callout interface for dedup / synonymy / supersession-candidate proposers.
- Batched processing semantics and Episode-atomicity handshake.
- Error-propagation rules.

This specification does **not** define:

- Memory type shapes — `memory-type-taxonomy.md`.
- Workspace identity — `workspace-model.md`.
- Grounding source taxonomy — `grounding-model.md`.
- Symbol allocation and lifecycle — `symbol-identity-semantics.md`.
- Temporal model — `temporal-model.md`.
- The IR write-surface grammar — `ir-write-surface.md` (this spec consumes parser output, not the grammar).
- The canonical-form bytecode — `ir-canonical-form.md` (this spec consumes and emits through the encoder, not the byte layout).
- Write-path WAL / durability / rollback — `write-protocol.md`.
- Read-path — `read-protocol.md`.

**Out of scope (anti-goals).** Concurrent-writer arbitration, SSI, rw-antidependency detection. Each workspace has a single writer; the pipeline runs one batch at a time.

### Graduation criteria

Graduates draft → authoritative when:

1. Roslyn compiler-pipeline reference and classical compiler-construction texts (Aho/Sethi/Ullman) are verified in `docs/attribution.md`.
2. The pipeline stages compile in `mimir_core` with the stage-boundary invariants in § 11. Shipped layout: `lex`, `parse`, `bind`, `semantic` as first-class modules plus `pipeline` which owns the `Emit` stage — a deliberate consolidation because `emit` was a thin per-record-shape dispatch that would have had a single function and lived inside `pipeline::compile_batch` regardless. The stage-boundary invariants (§ 11.1 stage purity, § 11.2 determinism, § 11.3 batch atomicity) hold across this layout.
3. Property tests cover: per-stage error propagation; determinism (same input produces same output byte-for-byte); batched-atomicity (partial-batch failures roll back).
4. The 14 registered inference methods (§ 5) each have a deterministic formula + doctest covering the claimed confidence bound.

## 2. Design thesis: compiler-pipeline architecture

Mimir's librarian is a compiler. Agent input is a structured language; the canonical store is the target representation. Compilation is the right metaphor because it brings known-good architectural discipline:

- **Stage separation.** Lex, parse, bind, semantic, emit are distinct responsibilities with narrow interfaces. A bug in one stage can't corrupt another's state.
- **Typed error routing.** Each stage owns a specific error taxonomy. The agent receives an error tagged with the stage at which it failed — the feedback is actionable.
- **Deterministic core.** Every stage is deterministic by construction. Same input, same output. ML proposals are a *side-channel* (dedup / synonymy / supersession candidates), always wrapped in a deterministic decision before any state change.
- **Batch atomicity.** A batch (an Episode) runs through all five stages before any durable commit. Partial-failure rollback is always possible because nothing is persisted until the whole batch succeeds.

The model is Roslyn (Microsoft's .NET Compiler Platform) compressed to the minimum viable compiler — lex / parse / bind / semantic / emit — without codegen backends or incremental-compilation machinery.

## 3. The five stages

### 3.1 Overview

```
agent input  ─►  [Lex]  ─►  [Parse]  ─►  [Bind]  ─►  [Semantic]  ─►  [Emit]  ─►  canonical log
   (bytes)       tokens     AST nodes    bound AST   validated     canonical
                                                     AST           records
```

Each arrow is a typed value; each stage is a pure function over its input (modulo symbol-table state in Bind, which is workspace-global).

| Stage | Input | Output | Error type | Determinism |
|---|---|---|---|---|
| Lex | `&[u8]` | `Vec<Token>` | `LexError` | fully deterministic |
| Parse | `Vec<Token>` | `Vec<UnboundForm>` | `ParseError` | fully deterministic |
| Bind | `Vec<UnboundForm>` + symbol table | `Vec<BoundForm>` + symbol mutations | `BindError` | fully deterministic |
| Semantic | `Vec<BoundForm>` + canonical store state | `Vec<ValidatedForm>` + proposed candidates | `SemanticError` | mostly deterministic (ML-callout hooks) |
| Emit | `Vec<ValidatedForm>` | `Vec<CanonicalRecord>` | `EmitError` | fully deterministic |

### 3.2 Lex — tokens from bytes

**Input:** `&[u8]` (UTF-8 agent input).
**Output:** `Vec<Token>` per `ir-write-surface.md` § 3.
**Error:** `LexError` (malformed UTF-8, invalid character in identifier, unterminated string, etc.).
**Determinism:** fully deterministic. Same bytes, same tokens.

The lexer is a state-machine recognizer. No heuristics. Newlines are whitespace (per `ir-write-surface.md` § 9.2).

### 3.3 Parse — AST from tokens

**Input:** `Vec<Token>`.
**Output:** `Vec<UnboundForm>`. An `UnboundForm` is the AST shape before symbol resolution — symbols are `RawSymbolName` (the `@name` string) rather than `SymbolId`.
**Error:** `ParseError` per `ir-write-surface.md` § 8.
**Determinism:** fully deterministic. Recursive-descent parser; no ambiguity per the grammar's unambiguous invariant.

### 3.4 Bind — symbol resolution

**Input:** `Vec<UnboundForm>` + current workspace symbol table.
**Output:** `Vec<BoundForm>` (AST with `SymbolId`s substituted for `RawSymbolName`s) + a record of symbol-table mutations (new allocations, rename edges, alias edges).
**Error:** `BindError` — `SymbolKindMismatch`, `SymbolRenameConflict`, `AliasCycle`, `AliasChainLengthExceeded`, `InferentialCycle`, `UnregisteredInferenceMethod`, `NoActiveWorkspace`, `ForeignSymbolForbidden` (foreign-workspace symbol rejected; workspaces never share symbols), etc.
**Determinism:** fully deterministic. Symbol resolution is a pure lookup + (optional) allocation under the single-writer boundary (PRINCIPLES.md architectural boundary #1).

Bind applies `symbol-identity-semantics.md` § 9 resolution algorithm to every `RawSymbolName` in the form. First-use allocations happen here; kind inference happens here.

### 3.5 Semantic — typecheck + grounding validation + candidate proposals

**Input:** `Vec<BoundForm>` + canonical-store state (current-state index, supersession DAG).
**Output:** `Vec<ValidatedForm>` (each form annotated with: librarian-assigned clocks, supersession targets, staleness flags, proposed dedup/synonymy/supersession candidates).
**Error:** `SemanticError` — `ConfidenceExceedsSourceBound`, `FutureValidity`, `InvalidClockOrder`, `SourceKindNotAdmitted`, `DanglingMemoryReference` (a `derived_from` parent doesn't exist), `CorrectsNonEpisodic`, etc.
**Determinism:** **mostly deterministic**. Validation is fully deterministic; *proposed candidates* may come from ML proposers via the callout interface (§ 6), but no state change happens at this stage — proposals are metadata attached to the validated form for the Emit stage to consume deterministically.

Semantic validation:

- Grounding rule enforcement per `grounding-model.md` § 3.
- Confidence bound per source kind (§ 4.1 of grounding-model).
- Clock validation per `temporal-model.md` § 9.3 (including future-validity without `projected` flag).
- Supersession-target detection per `temporal-model.md` § 5.
- Inferential parent existence check.
- Episodic `correct` target is an Episodic memory (`CorrectsNonEpisodic` if not).

### 3.6 Emit — canonical bytes

**Input:** `Vec<ValidatedForm>`.
**Output:** `Vec<CanonicalRecord>` ready for append to `canonical.log` per `ir-canonical-form.md` § 7.2.
**Error:** `EmitError` (internal; typically an invariant violation that should have been caught earlier — e.g., `InvariantViolation { stage: Emit, kind: ... }`).
**Determinism:** fully deterministic. Byte-for-byte.

Emit applies the byte layouts in `ir-canonical-form.md` § 5–6. No further validation; any inconsistency at this point is a librarian bug, not an agent error.

## 4. Batched processing and Episode atomicity

### 4.1 A batch = an Episode

The pipeline operates on **batches**. Each batch is one Episode per `memory-type-taxonomy.md` and PRINCIPLES.md architectural boundary #6 (checkpoint-triggered write batches).

A single-form "write" is a degenerate batch of size 1. An agent-initiated multi-form commit is a larger batch. The pipeline does not distinguish.

### 4.2 All-or-nothing stages

Stages run on the **whole batch** before advancing:

```
Lex(batch) → tokens
Parse(tokens) → forms
Bind(forms) → bound_forms  (allocates symbols; accumulates pending symbol-table mutations)
Semantic(bound_forms) → validated_forms
Emit(validated_forms) → canonical_records
```

If any stage fails on any form in the batch, the **whole batch fails**. Accumulated symbol-table mutations from Bind are discarded (not committed to the workspace's symbol table). No canonical records are appended.

### 4.3 Commit handshake

On successful Emit, the pipeline hands off to `write-protocol.md`'s commit mechanism:

1. Append all canonical records to the workspace's pending-commit WAL segment.
2. Apply supersession edges (set prior `invalid_at`).
3. Commit symbol-table mutations.
4. Emit a `CHECKPOINT` record with the Episode's `memory_count` and commit time.
5. Fsync the WAL.

On commit failure (disk full, I/O error), roll back: discard WAL segment, revert symbol-table mutations, return `PipelineError::CommitFailed`.

Full commit / rollback semantics are in `write-protocol.md`; this spec's obligation is only to hand off a validated, emitted batch.

## 5. Inference-method registry

The `method` field in an Inferential memory resolves to a registered inference-method symbol. Each method has:

- A **type signature** (number of parents, expected parent types).
- A **deterministic confidence formula** (how output confidence is derived from parent confidences).
- A **staleness predicate** (when this method's output becomes stale on parent change).

### 5.1 Registered methods (v1)

| Symbol | Parents | Formula | Staleness predicate |
|---|---|---|---|
| `@direct_lookup` | exactly 1 | `output.conf = parent.conf` | parent superseded |
| `@majority_vote` | N ≥ 3 (odd) | `output.conf = min(parents.conf)` (see v1 convention below) | any parent superseded |
| `@citation_link` | exactly 2 | `output.conf = min(parents.conf) * 0.9` | either parent superseded |
| `@analogy_inference` | exactly 2 | `output.conf = product(parents.conf) * 0.7` | either parent superseded |
| `@pattern_summarize` | N ≥ 2 | `output.conf = geomean(parents.conf) * 0.8` | > 50% of parents superseded |
| `@architectural_chain` | N ≥ 2 | `output.conf = product(parents.conf)` | any parent superseded |
| `@dominance_analysis` | N ≥ 2 | `output.conf = min(parents.conf) * 0.6` | any parent superseded |
| `@entity_count` | N ≥ 1 | `output.conf = min(parents.conf) * 0.8` | any parent count changes |
| `@interval_calc` | exactly 2 | `output.conf = min(parents.conf) * 0.9` | either endpoint superseded |
| `@feedback_consolidation` | N ≥ 1 | `output.conf = min(parents.conf) * 0.85` | any parent superseded |
| `@qualitative_inference` | N ≥ 1 | `output.conf = min(parents.conf) * 0.5` | any parent superseded |
| `@provenance_chain` | N ≥ 2 | `output.conf = product(parents.conf)` | any parent superseded |
| `@multi_source_consensus` | N ≥ 2 | `output.conf = 1 - product(1 - parents.conf)` (noisy-OR) | any parent superseded, < 2 remain |
| `@conflict_reconciliation` | N ≥ 2 peers the agent identifies as conflicting at the same conflict key (same `(s, p)` for SEM/INF or same `rule_id`/`(trigger, scope)` for PRO) | `output.conf = max(parents.conf) * 0.8` | any parent superseded |

All formulas are deterministic and computed in fixed-point arithmetic at the `u16` confidence resolution (per `ir-canonical-form.md` § 3.1), not floating-point — so output is bit-identical across architectures.

**`@majority_vote` v1 convention:** the write surface does not carry a separate `votes_against` field, so an agent expresses a majority-vote inference by listing only the *voters-in-favor* as `derived_from`. Under this convention `votes_for == N` and the formula `(votes_for / N) * min(parents.conf)` collapses to `min(parents.conf)` — the implementation in `mimir_core::inference_methods` computes that collapsed form. Lifting this convention (to carry explicit against-counts at the write surface) is post-v1.

**`@conflict_reconciliation` semantics:** an agent that identifies two or more current memories as conflicting can emit a new Inferential memory whose `derived_from` lists those parents and whose `method` is `@conflict_reconciliation`. When the Inferential commits, the librarian applies supersession edges to each parent — the reconciliation memory becomes the current-state entry at the shared conflict key. Under single-writer semantics auto-supersession by `(s, p)` normally resolves Semantic conflicts at write time; this method is the sanctioned path when an agent re-reads historical state and chooses to consolidate two memories that both survived (e.g., Inferentials from different methods).

### 5.2 Registry is closed per release

The registered method set is closed at each release. Adding a method requires a PR, a spec update (this section), and a librarian version bump per `PRINCIPLES.md` § 10 semver.

Agents emitting an unregistered method symbol (`@my_custom_method` not in the table) receive `BindError::UnregisteredInferenceMethod`.

### 5.3 Confidence formula doctest requirement

Every method's formula is implemented in `mimir_core::inference_methods::<method>` with a rustdoc doctest covering the claimed bound. CI rejects PRs that add a method without a doctest asserting the formula's range behavior.

## 6. ML-callout interface

The Semantic stage may invoke ML proposers for dedup / synonymy / supersession candidates. All proposals are wrapped in deterministic decisions; ML output never mutates state.

> **Scope note.** This section describes a post-MVP surface. v1 Mimir ships `NoopInferenceProposer` only (see § 6.4); no ML proposers, no IPC to ML processes, no out-of-process workers exist in the shipped librarian. The out-of-process model described in § 6.2 is the intended design for real proposers when they land — it is **not** contradicted by `wire-architecture.md`'s in-process-only scope, which governs the agent ↔ librarian boundary, not the librarian ↔ ML-proposer boundary.

### 6.1 Trait

```rust
pub trait InferenceProposer: Send + Sync {
    fn propose_candidates(
        &self,
        batch: &[BoundForm],
        store_state: &StoreStateReadGuard,
    ) -> Vec<Candidate>;
}

pub struct Candidate {
    pub kind: CandidateKind,
    pub target: MemoryRef,
    pub related: Vec<MemoryRef>,
    pub score: f32,           // ML score; advisory, never load-bearing
    pub model_version: String,
    pub input_hash: [u8; 32],
}

pub enum CandidateKind {
    Dedup,
    Synonymy,
    Supersession,
}
```

### 6.2 IPC boundary

ML proposers run **out-of-process**. The librarian calls via a local Unix socket or named pipe; the ML process reads a serialized batch, returns candidates, exits or persists.

Rationale:

- Isolates nondeterministic ML dependencies from the librarian's deterministic core.
- Allows the librarian process to run unmodified when no ML is configured.
- Lets ML crashes fail softly (no candidate proposals for this batch) without taking down the librarian.

Serialization over IPC uses the same canonical form as on-disk — not JSON, not prose — so the ML process reads the librarian's native representation without a translation layer.

### 6.3 Deterministic decision wrapper

For every candidate returned:

1. The Semantic stage records the candidate as *proposed*, tagged with `model_version + input_hash`.
2. A deterministic decision rule decides whether to apply the proposal. For dedup: apply if exact canonical-form equality (ML can only *suggest*; merge only happens on exact match). For supersession: apply only if the `(s, p)` / `(trigger, scope)` deterministic rule also fires. For synonymy: record as a pending candidate Episodic memory; never auto-applied.
3. The decision, the `model_version`, and the `input_hash` become part of the committed canonical record. Replay produces the same decision.

### 6.4 v1 ships with no-op ML

v1 Mimir ships a `NoopInferenceProposer` — returns an empty candidate list. This keeps the v1 librarian fully deterministic end-to-end. Real ML proposers are post-MVP.

## 7. Dedup / synonymy / supersession-candidate proposers

Three concrete proposer roles, each with a v1 deterministic default:

### 7.1 `DedupProposer` (v1 default: exact-tuple match)

```rust
trait DedupProposer {
    fn propose_dedups(
        &self,
        batch: &[BoundForm],
        store_state: &StoreStateReadGuard,
    ) -> Vec<DedupCandidate>;
}
```

v1 implementation: canonical-form byte-equality comparison against the workspace's current-state index. Two incoming forms that would produce byte-identical canonical records are flagged; the Semantic stage collapses them into one.

### 7.2 `SynonymyProposer` (v1 default: disabled)

```rust
trait SynonymyProposer {
    fn propose_synonyms(
        &self,
        symbols: &[SymbolId],
        store_state: &StoreStateReadGuard,
    ) -> Vec<SynonymyCandidate>;
}
```

v1 implementation: no-op, returns empty. ML-only problem; defer entirely.

### 7.3 `SupersessionCandidateProposer` (v1 default: deterministic per temporal-model)

```rust
trait SupersessionCandidateProposer {
    fn propose_supersessions(
        &self,
        batch: &[BoundForm],
        store_state: &StoreStateReadGuard,
    ) -> Vec<SupersessionCandidate>;
}
```

v1 implementation: apply the rules in `temporal-model.md` § 5 — same `(s, p)` for Semantic, same `rule_id` or `(trigger, scope)` for Procedural. Purely deterministic.

### 7.4 Proposer composition

The Semantic stage invokes the three proposers in order: Dedup → SupersessionCandidate → Synonymy. Each can produce candidates; all candidates are attached to the batch's validated forms before Emit runs. No proposer mutates state.

## 8. Symbol-table mutation ordering within a batch

Bind processes forms in input order, accumulating symbol-table mutations. All Bind-stage allocations complete before Semantic runs. This ensures:

- Semantic sees a fully-bound batch (no dangling `RawSymbolName`s).
- Forms within a batch that reference symbols allocated *earlier in the same batch* resolve correctly.
- Symbol-table mutations are discarded atomically on batch failure.

Ordering within Bind is stable (input order); no reordering optimization in v1.

## 9. Backpressure

The pipeline does not buffer. Under the v1 in-process synchronous API (`wire-architecture.md` § 3), there is no queue and no separate wire-layer backpressure mechanism — Rust's borrow checker serializes writes by rejecting concurrent `&mut Store` access, so the librarian runs one batch through all stages, commits or rolls back, then returns to the caller. No parallel batch processing in v1 (per PRINCIPLES.md architectural boundary #1, single-writer).

Batch size is bounded by configuration (default: 256 forms per batch). Batches larger than the limit are rejected synchronously by the parse / semantic stage before emit.

## 10. Error propagation

Every stage failure returns a typed `PipelineError`:

```rust
pub enum PipelineError {
    Lex(LexError),
    Parse(ParseError),
    Bind(BindError),
    Semantic(SemanticError),
    Emit(EmitError),
    CommitFailed(StoreError),
    BatchTooLarge { limit: usize, attempted: usize },
    InferenceProposerFailure { kind: CandidateKind, cause: String },
    InvariantViolation { stage: Stage, kind: String },
}
```

The variant identifies which stage failed; the inner error type gives the specific failure. The agent sees this typed error per `PRINCIPLES.md` § 2 (errors are data, not strings).

A `PipelineError` always means the batch did **not** commit. On error, the librarian returns the error to the agent and waits for the next batch; no state mutation.

## 11. Invariants

1. **Stage boundaries.** Each stage reads only from its input type and its designated state; no cross-stage back-channels. Stages are pure functions.
2. **Determinism.** Same input + same workspace state + same inference-proposer output → same canonical records, byte-for-byte.
3. **Batch atomicity.** Either the whole batch commits, or none of it does. Partial commits are impossible.
4. **Symbol-table consistency.** Bind-stage mutations are discarded if any downstream stage fails on any form in the batch.
5. **ML advisory.** ML proposer output never mutates state without a deterministic decision wrapper (§ 6.3).
6. **Single-writer.** One pipeline instance per workspace. Cross-batch concurrency is forbidden by construction; batches flow through the wire layer in arrival order and the pipeline consumes them serially.
7. **Error stage identification.** Every `PipelineError` carries the stage that produced it. Agents can route errors by stage (e.g., parse errors go to the prompt-correction flow; semantic errors go to the grounding-correction flow).

## 12. Open questions and non-goals for v1

### 12.1 Open questions

**Incremental recompilation.** The pipeline processes batches from scratch. Incremental compilation (reusing bound state across related batches) is a post-MVP optimization. Revisit if batch latency becomes a hotspot.

**Warm-start symbol table.** On librarian startup, the symbol table loads from snapshot + WAL (per `ir-canonical-form.md` § 7.3). How quickly can Bind become operational? v1 accepts a cold start; post-MVP might add a background warm-up for large symbol tables.

**Pipeline parallelism within a batch.** Forms within a batch could in principle be Lex/Parse'd in parallel. v1 is sequential for simplicity and determinism debuggability. Revisit if batch-size × complexity grows.

**Observability hooks.** The pipeline emits `tracing` events per `PRINCIPLES.md` § 5. Which stage transitions deserve spans? Currently: one span per stage per batch, plus per-form events in Bind and Semantic. Refine with real workload data.

**ML-proposer timeout semantics.** The IPC callout to an ML process could hang. v1 uses a per-call timeout (default 500 ms), beyond which the proposer's candidates are dropped for that batch. Should slow proposers be disabled after N timeouts? Post-MVP.

### 12.2 Non-goals for v1

- **Multiple concurrent batches per workspace.** Strictly serial, one batch at a time, per single-writer invariant.
- **ML-driven semantic validation.** ML can propose candidates; it cannot override deterministic validation.
- **Pipeline stage plugins by agents.** Stages are librarian-controlled. Agents do not register custom stages.
- **Cross-workspace batches.** A batch targets exactly one workspace. Cross-workspace memory copies use the explicit import flow in `workspace-model.md` § 5.3.
- **Arbitrary-length batches.** Batch size is bounded (default 256 forms). Larger batches are split at the wire layer.

## 13. Primary-source attribution

All entries are verified per `docs/attribution.md`.

- **Roslyn compiler architecture** (verified, already cited for Symbol Identity Semantics) — the lex / parse / bind / semantic / emit stage decomposition and the compiler-pipeline-as-service pattern. Directly informs § 2 and § 3.
- **Aho, Sethi, Ullman, *Compilers: Principles, Techniques, and Tools*** (verified, already cited) — classical compiler-pipeline structure, typed intermediate representations.
- **Appel, *Modern Compiler Implementation in ML*** (1998, pending) — discussion of pipeline-as-pure-functions-over-IR, informing § 11 invariant 1 (stage-purity).
- **Noisy-OR / weighted-sum inference method families** (verified — e.g., Pearl, *Probabilistic Reasoning in Intelligent Systems*, 1988) — the confidence-combination formulas in § 5 draw from this literature. Verification pass will decide which specific citations are load-bearing.
