# Grounding Model

> **Status: authoritative — graduated 2026-04-17; scope amended 2026-04-24.** `@cross_workspace_import` was removed from the taxonomy on 2026-04-18 when raw cross-workspace reads/imports were removed from the workspace-local implementation. The 2026-04-24 mandate introduces governed cross-scope promotion in draft `scope-model.md`; this spec does not yet define a dedicated promoted-scope source kind. The reduced taxonomy is 11 kinds. The `SourceKind::CrossWorkspaceImport` variant still exists in `mimir_core::source_kind` pending a follow-up code cleanup; no write path emits it. All other cited sources verified (see `docs/attribution.md`), and the grounding rules are implemented by `mimir_core::semantic::validate` (plus `mimir_core::source_kind::SourceKind::{confidence_bound, admits}` as compile-time constants). The kind taxonomy + admission matrix + per-kind confidence bounds are enforced at the semantic pipeline stage with typed `SemanticError::{ConfidenceExceedsSourceBound, SourceKindNotAdmitted, FutureValidity, InvalidClockOrder, EmptyDerivedFrom, CorrectsNonEpisodic}` errors. Source-kind derivation from symbol names uses the reserved grounding-kind names (`@profile`, `@observation`, `@self_report`, `@participant_report`, `@document`, `@registry`, `@policy`, `@agent_instruction`, `@external_authority`, `@pending_verification`, `@librarian_assignment`) with `Observation` as the default for arbitrary agent-supplied source symbols (e.g. witness names in Episodic memories). Graduated on milestone 5.4.

Mimir grounds every memory in a typed **source**. Semantic, Episodic, and Procedural memories carry a `source: Symbol` field; Inferential memories carry `derived_from: Vec<Symbol>` + `method: Symbol` instead. This specification defines the source-type taxonomy, confidence bounds by source kind, enforcement rules at write time, and the provenance-chain semantics for Inferential memories.

## 1. Scope

This specification defines:

- The source-type taxonomy used by the `source` field in Semantic, Episodic, and Procedural memories.
- Per-source-type: what the source represents, which memory types admit it, the default confidence upper bound, and the symbol-kind obligation.
- Confidence-bound enforcement at write time.
- Provenance-chain semantics for Inferential memories (`derived_from` + `method`).
- How grounding interacts with the workspace partition.

This specification does **not** define:

- Memory type shapes — `memory-type-taxonomy.md`.
- Workspace identity and partitioning — `workspace-model.md`.
- Symbol identity or symbol-kind taxonomy — `symbol-identity-semantics.md` (this spec *names* symbol-kind obligations; the catalog is there).
- Confidence decay formulas over time — `confidence-decay.md` (source upper bounds are fixed here; decay is there).
- The registered inference-method enum — `librarian-pipeline.md` § Inferential methods (method symbols are opaque here).

### Graduation criteria

Graduates draft → authoritative when:

1. Related art is verified in `docs/attribution.md`.
2. A Rust `SourceKind` enum matching § 7 compiles in `mimir_core`, with confidence-bound enforcement under unit and property tests.
3. Every memory type in `memory-type-taxonomy.md` is tested against the admits-source-kind matrix in § 3.
4. The Inferential `method` registry in `librarian-pipeline.md` is defined so method resolution (§ 5.2) is testable.

## 2. Design thesis: why grounding is typed

A memory's confidence is a function of its grounding. A fact grounded in a profile ("Alice said her email is X") carries different epistemic weight than the same fact grounded in an observation ("I saw Alice's email in the 'From' field of her message"). Free-form, string-valued grounding loses this distinction — the librarian cannot reason about source trust without parsing free text.

Typed grounding is a determinism lever. The binder knows at bind time:

- **Which memory types admit this source.** A Procedural memory cannot be grounded in a single `@observation` — crystallizing an observation into a policy requires an explicit act of rule-making attributed to a rule-maker. The binder rejects `Procedural { source: @observation, … }`.
- **What confidence upper bound applies.** An `@profile`-grounded memory cannot exceed confidence 0.95; no agent self-certifies a profile claim at 1.0.
- **What symbol-kind obligation the source carries.** An `@document`-grounded memory's source symbol must resolve to a symbol of kind Document (carries a citation pointer); an `@observation`-grounded memory's source symbol must resolve to a symbol of kind Agent (the observer).

Typed grounding lets the librarian compute trust, route supersession correctly, and audit provenance without heuristic parsing.

## 3. Source-type taxonomy

Twelve source kinds. The Rust enum in § 7 is `#[non_exhaustive]` to allow extensions without a breaking change; additions require a PR and a spec update per `PRINCIPLES.md` § 10 (Semantic versioning).

### 3.1 Overview

| Kind | Represents | Admits | Default conf bound | Symbol-kind obligation |
|---|---|---|---|---|
| `@profile` | User / entity-provided identity or attribute data | Semantic | 0.95 | source resolves to Agent (the profile's subject) |
| `@observation` | Directly witnessed by an agent | Episodic, Semantic | 1.0 | source resolves to Agent (the observer) |
| `@self_report` | Subject reported the fact about themselves | Semantic, Episodic | 0.9 | source resolves to Agent (subject = reporter) |
| `@participant_report` | A participant in an event reported it | Episodic | 0.85 | source resolves to Agent (the reporting participant) |
| `@document` | A cited document, URL, paper, or canonical spec | Semantic | 0.9 | source resolves to Document |
| `@registry` | An authoritative registry (package manifest, DNS, filesystem metadata) | Semantic | 0.95 | source resolves to Registry |
| `@policy` | A deliberate act of policy-making by a rule-maker | Procedural | 1.0 | source resolves to Agent or Policy |
| `@agent_instruction` | An instruction from the agent's operator / owner | Procedural, Semantic | 0.95 | source resolves to Agent (the operator) |
| `@external_authority` | A trusted third-party service or API (not a static document) | Semantic | 0.9 | source resolves to Service |
| `@pending_verification` | A claim made without primary-source verification (transitional) | any | capped at 0.6 | no symbol-kind obligation |
| `@librarian_assignment` | A fact the librarian emitted during bind (timestamps, symbol IDs, derived clocks) | Semantic | 1.0 | source resolves to the librarian instance |

### 3.2 Notes per kind (where the table is insufficient)

**`@observation` confidence 1.0.** Defaults to 1.0 because a direct observation is as confident as the observer. Time *decay* of observation-grounded memories (per `confidence-decay.md`) brings confidence down over time; the source upper bound doesn't.

**`@self_report` confidence 0.9.** Not 1.0 because people / agents can misreport about themselves (intentionally or otherwise). The librarian applies this bound even when the subject's own asserted confidence is 1.0.

**`@participant_report` confidence 0.85.** Slightly lower than `@self_report` because a participant's view of an event may be partial. Used for Episodic memories where the grounding agent was involved in the event but is not the primary subject.

**`@policy` confidence 1.0, with activity weighting.** Procedural memories grounded in `@policy` do not time-decay (memory-type-taxonomy § 3.3), but they may be *activity-decayed* — a rule never invoked may be flagged stale. Activity decay is distinct from time decay; see `confidence-decay.md`.

**`@pending_verification` cap at 0.6.** A deliberate floor that signals "this claim is in the system but not authoritative." Any memory with source `@pending_verification` displays its pending status to the agent at read time. Promotion to a real source happens via a new write with the verified source; the `@pending_verification` original is superseded.

**`@librarian_assignment` confidence 1.0.** The librarian is authoritative for facts it assigns (timestamps, internal symbol IDs, bind-derived clocks). These memories do not route through the normal write surface — they're emitted during bind. Listed here because they populate the same `source` field.

## 4. Confidence bounds and enforcement

### 4.1 Strict enforcement at write

Confidence bounds are **strictly enforced at write time**. The librarian rejects writes whose stated confidence exceeds the source kind's bound:

```
BindError::ConfidenceExceedsSourceBound {
    requested: Confidence,
    bound: Confidence,
    source_kind: SourceKind,
}
```

**Rationale (per `PRINCIPLES.md` § 1, precision-over-drift):** silently clipping confidence would hide the mismatch from the agent. An explicit error lets the agent re-ground with a higher-bound source, explicitly lower its stated confidence, or accept that the bound is correct for its evidence.

### 4.2 No soft / advisory bounds

There is no "soft bound that warns but accepts." The librarian treats confidence bounds as type-system constraints: either the memory is constructible or it isn't. This matches the type-safety policy in `PRINCIPLES.md` § 3 — invariants enforced by construction, not by runtime warning.

### 4.3 Deriving confidence from multiple sources

v1 does not support multi-source grounding for a single memory. If an agent has multiple sources supporting the same claim, it emits either:

- A single memory with the strongest source, or
- Multiple memories (one per source) plus an Inferential memory that consolidates them via a `@multi_source_consensus` method (candidate method, registered in `librarian-pipeline.md`).

Multi-source single-memory grounding is a post-MVP candidate (§ 8.2).

## 5. Inferential grounding: `derived_from` and `method`

Inferential memories do not use the `source` field. Their grounding is the combination of:

- `derived_from: Vec<Symbol>` — one or more parent memory IDs.
- `method: Symbol` — a symbol resolving to a registered inference method.

### 5.1 Provenance-chain semantics

The relationship "A is derived from B" creates a directed edge in a provenance graph. The graph:

- Is **directed** (derived_from → parent).
- Is **acyclic** by construction — a memory cannot derive from itself or from any of its descendants. The binder rejects cycle-introducing writes with `BindError::InferentialCycle`.
- Is **shared** — one parent can appear in multiple derivations. The graph is not a tree.

### 5.2 Method registry (opaque at this spec's level)

`method` is an opaque Symbol here. The set of valid method symbols and the deterministic evaluation rule per method are defined in `librarian-pipeline.md` § Inferential methods.

Candidate method symbols, drawn from the tokenizer bake-off corpus (not final):

`@direct_lookup`, `@majority_vote`, `@citation_link`, `@analogy_inference`, `@pattern_summarize`, `@architectural_chain`, `@dominance_analysis`, `@entity_count`, `@interval_calc`, `@feedback_consolidation`, `@qualitative_inference`, `@provenance_chain`, `@multi_source_consensus`.

From the grounding perspective the only requirement is: `method` must resolve to a symbol in the registered method set at bind time, or `BindError::UnregisteredInferenceMethod` applies.

### 5.3 Confidence of Inferential memories

Each method carries a deterministic rule for computing the derived memory's confidence from its parents' confidences. Exact per-method formulas are in `confidence-decay.md`. Minimum constraint: **a derived memory's confidence cannot exceed the minimum confidence of its parents** — a chain cannot be stronger than its weakest link. This is enforced at write time by the librarian, independently of the per-method formula.

### 5.4 Stale-parent propagation

When any parent of an Inferential memory is superseded, the Inferential memory is flagged *stale*. Stale does not mean invalid — the original derivation is preserved — but it signals that the Inferential memory's continued validity requires re-derivation against the current parent set.

Re-derivation is **explicit**, never implicit: either an agent-requested operation or a librarian-scheduled consolidation pass. Lazy staleness is intentional; auto-re-deriving Inferentials on every parent supersession would create cascades that violate determinism and could introduce infinite loops.

## 6. Workspace-scoped grounding

All grounding in this spec is **workspace-scoped** per `workspace-model.md`. A memory grounded in `@profile` is grounded in that agent's profile *within this workspace*. Each workspace has a single Claude writer; the grounding taxonomy describes how that writer types its own provenance, not how multiple agents reconcile claims.

Concretely:

- An `@observation` source resolves to a symbol of kind Agent. That symbol is workspace-scoped: if workspaces A and B both have an `@alain`, they are distinct symbols. Scope-aware reuse must promote a new record with provenance rather than reading raw workspace records together.

## 7. Invariants and the Rust enum

### 7.1 `SourceKind`

```rust
#[non_exhaustive]
pub enum SourceKind {
    Profile,
    Observation,
    SelfReport,
    ParticipantReport,
    Document,
    Registry,
    Policy,
    AgentInstruction,
    ExternalAuthority,
    PendingVerification,
    LibrarianAssignment,
}

impl SourceKind {
    pub const fn confidence_bound(self) -> Confidence { /* per § 3 */ }
    pub const fn admits(self, kind: MemoryKindTag) -> bool { /* per § 3 */ }
}
```

Both `confidence_bound` and `admits` are `const fn` — the taxonomy is compile-time-resolved, not runtime-configurable. Changing bounds or admission rules requires a code change plus a spec update.

### 7.2 Invariants enforced at write

1. **Confidence bound.** `memory.confidence <= source_kind.confidence_bound()` for the memory's `source` field.
2. **Type admission.** `source_kind.admits(memory.kind_tag())` holds.
3. **Symbol-kind obligation.** The `source` field resolves to a symbol of the kind required in § 3.
4. **Inferential acyclicity.** For every Inferential memory, `derived_from` forms a DAG when unioned with the existing provenance graph in the workspace.
5. **Method resolution.** For every Inferential memory, `method` resolves to a symbol in the registered inference-method enum.

Every invariant produces a typed `BindError::*` on violation. Agents parse errors by variant; they never regex-match error messages (per `PRINCIPLES.md` § 2).

## 8. Open questions and non-goals for v1

### 8.1 Open questions

**Registry-kind granularity.** The `@registry` source currently covers package manifests, DNS, filesystem metadata as a single kind. Should these split into granular kinds (`@package_registry`, `@dns_registry`, `@fs_metadata`) with different confidence bounds? Candidate driver: empirical disagreement between registries. Defer until workload data demands it.

**Time-limited sources.** Should some source kinds carry an inherent TTL (e.g., `@external_authority` facts expire after N days, forcing re-grounding)? Currently all source-bound confidence is time-invariant at the bound level; time decay is `confidence-decay.md`'s concern. Revisit if re-grounding pressure becomes real.

**Method-level confidence.** Inferential memories use `method` as a symbol. Should the method itself carry confidence metadata (some methods are intrinsically more trustworthy than others)? Defer — if this emerges, it lands in `confidence-decay.md`.

### 8.2 Non-goals for v1

- **Multi-source single-memory grounding.** Consolidating multiple grounding sources into one memory is out of scope. Use multiple memories + an Inferential consolidation memory.
- **Agent-defined source kinds.** The source-kind enum is librarian-controlled. Agents cannot define new source kinds at write time. Registry additions require a PR and a spec update.
- **Dynamic confidence-bound adjustment.** Confidence bounds are fixed per kind. Adjusting bounds based on observed trust ("this profile source has been wrong often; lower its bound") is post-MVP.
- **Raw cross-workspace grounding / import.** Workspace-local stores do not share raw memories. Governed cross-scope promotion is specified separately in `scope-model.md`.

## 9. Primary-source attribution

All entries are verified per `docs/attribution.md`. None is load-bearing for the taxonomy above — those derive from Mimir's architectural principles (workspace isolation, typed provenance, determinism-over-speed).

- **Provenance in databases** — Green, Karvounarakis, Tannen, *Provenance Semirings*, PODS 2007 (verified) — formal framework for typed provenance over relational queries. Candidate for authoritative citation of the provenance-chain structure in § 5.
- **Belief revision under typed sources** — Dubois, Prade and related work on reasoning under uncertainty with typed evidence (pending, speculative — verification pass will decide which, if any, are load-bearing).
