# Memory Type Taxonomy

> **Status: authoritative — graduated 2026-04-17.** All three cited sources verified (see `docs/attribution.md`) and the Rust `MemoryKind` enum + `MemoryKindTag` + `Semantic` / `Episodic` / `Procedural` / `Inferential` variant structs compile in `crates/mimir_core` against the § 6 invariants; property tests cover the type-admission matrix (§ 3 × § 7.2 in `grounding-model.md`) and the tag-reflects-variant invariant. This is the first spec to reach `authoritative`.

This specification defines the memory types that Mimir admits. It is the foundational spec: nothing under `docs/concepts/` depends on specs that depend on this document (memory-type-taxonomy is upstream of everything else).

## 1. Scope

This specification defines:

- The four canonical memory types (semantic, episodic, procedural, inferential).
- The ephemeral memory tier alongside the canonical store.
- The canonical tuple shape for each type.
- The per-type lifecycle, grounding rule, and decay profile.
- Type invariants and disambiguation rules.

This specification does **not** define:

- Symbol identity semantics — see `symbol-identity-semantics.md`.
- Temporal clock mechanics — see `temporal-model.md`.
- The source-type taxonomy — per-type *grounding rules* are here; the *source taxonomy* is in `grounding-model.md`.
- Confidence decay formulas — per-type decay *profiles* are named here; the *formula parameters* are in `confidence-decay.md`.
- The librarian pipeline — see `librarian-pipeline.md`.
- The IR grammar — see `ir-write-surface.md`.

### Graduation criteria

This spec graduates from draft to authoritative when all three hold:

1. ✅ LangMem, Tulving 1972, and (as rejected-with-known-risk alternative) Park et al. 2023 Generative Agents verified in `docs/attribution.md` on 2026-04-17.
2. A Rust `MemoryKind` enum matching § 5 compiles and exports from the `mimir_core` crate, with the invariants in § 6 enforced by construction.
3. Property tests exist covering every invariant in § 6 and every disambiguation rule in § 6.2.

## 2. Design thesis: why typed memory

Homogeneous fact tuples (`(s, p, o)` for everything) are insufficient for agent coherence. Three reasons:

1. **Different grounding obligations.** A fact grounded in a profile carries different epistemic weight than a fact derived from other facts. Conflating them loses provenance, which breaks supersession reasoning.
2. **Different decay profiles.** Episodic memories decay faster than semantic attributes: an event from five years ago is less trusted than a typed attribute like "Alice's email." Procedural rules don't decay with time while active — their validity is binary (active or superseded), though they may carry activity-weighted freshness. Decay parameterization by type is a design lever for realistic recall, not an implementation detail.
3. **Different supersession semantics.** Superseding a semantic fact ("Alice's email changed") is distinct from superseding an episodic memory ("the event didn't happen the way we thought"). The librarian dispatches on type to pick the right supersession rule — without typed memory, every supersession path would carry a runtime type-branch.

Typing at the system boundary is a determinism lever. The binder refuses to bind a tuple whose type doesn't match its shape before any downstream operation runs. Per `PRINCIPLES.md` § 3 (Type safety policy), invariants are enforced by construction, not by runtime assertion.

**Prior art** (verified in `docs/attribution.md`):

- **LangMem** uses a semantic / episodic / procedural split for agent memory.
- **Tulving (1972)** coined the term "episodic memory" in this chapter (crediting Quillian 1966 for "semantic memory") and established the episodic-vs-semantic distinction in cognitive psychology. Important scope note: Tulving 1972 framed the distinction as pre-theoretical — "for the convenience of communication, rather than as an expression of any profound belief about structural or functional separation" — and does **not** discuss procedural memory. Mimir's procedural-as-third-type attribution derives from later work (Tulving 1985, Cohen & Squire 1980 lineage) and from LangMem's agent-memory adaptation.

Mimir adds **inferential** as a fourth type — memories whose grounding is other memories, not sources. Verification pass (2026-04-17) confirmed LangMem does **not** define a fourth type; Inferential is an Mimir-specific addition for typed-provenance reasoning. Mimir also adds an **ephemeral** tier (§ 4); LangMem does not have an equivalent.

## 3. The four canonical types

Each type has a canonical tuple shape below. Fields shown are load-bearing for this spec. The full field list at the IR grammar surface (`ir-write-surface.md`) may add librarian-internal fields (resolved symbol IDs, assigned clock values) that aren't part of the agent-facing write shape.

### 3.1 Semantic

**Purpose:** general facts about the world. Entity attributes, relationships, category memberships, type-level claims.

Canonical examples:
- `(@alice, email, "alice@example.com", source=@profile, conf=0.95, valid_at=T)`
- `(@mimir, language, @rust, source=@design_doc, conf=0.99, valid_at=T)`

**Canonical shape:**

```
Semantic {
    s: Symbol,            // subject
    p: Symbol,            // predicate
    o: Value,             // Symbol | string | int | float | bool
    source: Symbol,       // epistemic grounding (see grounding-model.md)
    confidence: Confidence,
    valid_at: ClockTime,
}
```

**Lifecycle:**

- Created via direct agent write through the librarian pipeline.
- Superseded via bi-temporal edge invalidation when a new semantic memory with the same `(s, p)` and a later `valid_at` is written, subject to supersession rules in `write-protocol.md`.
- Decays **slowly** — semantic attributes are stable over time absent superseding evidence. Exact rate per `(grounding, symbol-kind)` is in `confidence-decay.md`.

**Grounding rule:** `source` must resolve to a symbol whose grounding-kind is in `{profile, observation, document, registry, external-authority, …}` per `grounding-model.md`. `source` must **not** be an inference-method or derivation chain — those belong to Inferential.

### 3.2 Episodic

**Purpose:** events at a point in time. What happened, who was involved, where, when, who witnessed.

Canonical examples:
- `(@ep_001, kind=@rename, participants=[@luamemories, @mimir], location=@github, at_time=T, observed_at=T, source=@alain, conf=1.0)`
- `(@ep_008, kind=@discussion, participants=[@alain, @claude_4_7, @cahill_2008], location=@chat, at_time=T, observed_at=T, source=@alain, conf=0.8)`

**Canonical shape:**

```
Episodic {
    event_id: Symbol,
    kind: Symbol,                 // event-type symbol (e.g. @rename, @commit, @discussion)
    participants: Vec<Symbol>,    // ontic — actors in the event itself
    location: Symbol,
    at_time: ClockTime,           // when the event occurred
    observed_at: ClockTime,       // when it was recorded
    source: Symbol,               // epistemic — who witnessed/reported
    confidence: Confidence,
}
```

**Key distinction: `participants` ≠ `source`.**

- `participants` is *ontic* — the actors in the event itself. Bob and the ball are participants in "Bob kicked the ball."
- `source` is *epistemic* — who witnessed or reported the event to the librarian. Alice can be the source of an episodic memory whose participants are Bob and the ball.
- The witness may or may not be a participant. Both fields are load-bearing; they do not collapse into one.

**Lifecycle:**

- Created via direct agent write through the librarian pipeline.
- Episodic memories do **not** supersede in the semantic sense. They are "what happened." A *correction* to an episodic memory (e.g., "it happened at T, not T'") is itself a new Episodic memory with supersession metadata — the original is invalidated via edge, never overwritten.
- Decays **faster** than semantic — recent events are more trusted than older ones, reflecting realistic recall. Exact rate in `confidence-decay.md`.

**Grounding rule:** `source` must resolve to a symbol whose grounding-kind is in `{observation, self-report, participant-report, external-authority, …}`. Episodic source requires a witness — a symbol whose kind supports epistemic provenance.

### 3.3 Procedural

**Purpose:** rules, routines, triggers-and-actions. Policies that direct future behavior rather than describe the world.

Canonical examples:
- `(@proc_001, trigger="agent about to write memory", action="route via librarian", scope=@mimir, source=@agents_md, conf=1.0)`
- `(@proc_008, trigger="commit requested", action="follow conventional commits format", scope=@mimir_repo, source=@agents_md, conf=1.0)`

**Canonical shape:**

```
Procedural {
    rule_id: Symbol,
    trigger: Value,             // condition description
    action: Value,              // action description
    precondition: Option<Value>, // optional additional gating
    scope: Symbol,              // where this rule applies
    source: Symbol,
    confidence: Confidence,
}
```

**Lifecycle:**

- Created via direct agent write.
- Superseded via bi-temporal edge invalidation when a new rule with the same `rule_id`, or with the same `(trigger, scope)` pair, is written.
- Does **not decay with time** while the rule is active. Procedural validity is binary (active or superseded); time is not a decay axis.
- Optionally activity-weighted: rules that fire often are reinforced, rules that never fire may be flagged stale. Activity decay is distinct from time decay — see `confidence-decay.md` § activity weighting.

**Grounding rule:** `source` must resolve to a symbol whose grounding-kind is in `{policy, learned-pattern, agent-instruction, …}`. Procedural rules cannot be grounded in an observation alone — a single observation produces an Episodic memory; crystallizing it into a policy requires an explicit act of rule-making attributed to the rule's author.

### 3.4 Inferential

**Purpose:** facts derived from other memories rather than from external sources. Consolidations, pattern summaries, implication chains.

Canonical examples:
- `(@alice, prefers, @coffee, derived_from=[@ep_orders_mon, @ep_orders_tue, …], method=@pattern_summarize, conf=0.7, valid_at=T)`
- `(@mimir, v1_target_model, @claude_4_7, derived_from=[@sem_003, @sem_004], method=@direct_lookup, conf=0.9, valid_at=T)`

**Canonical shape:**

```
Inferential {
    s: Symbol,
    p: Symbol,
    o: Value,
    derived_from: Vec<Symbol>,    // parent memory IDs (any type)
    method: Symbol,               // how the inference was computed
    confidence: Confidence,
    valid_at: ClockTime,
}
```

**Key distinction from Semantic:** grounding is `derived_from` (memories), not `source` (external). An inferential memory's confidence is bounded by a method-specific function of its parents' confidences — exact formula in `confidence-decay.md`.

**Lifecycle:**

- Created by either the librarian (deterministic consolidation passes) or an agent (explicit derivation with provenance).
- Superseded when any parent is superseded *and* the new parent's content invalidates the derivation. Supersession propagation is **lazy**: the inferential memory is flagged stale on parent change; re-derivation is an explicit operation, not implicit.
- Decays with the weighted product of parent decays plus a method-specific decay factor (inference methods with weaker confidence priors decay faster).

**Grounding rule:** `derived_from` must be non-empty and every referenced memory ID must resolve. `method` must resolve to a symbol in the registered inference-method enum (enumerated in `librarian-pipeline.md` § Inferential methods).

## 4. Ephemeral tier

**Purpose:** intra-session state that does not survive session end. Scratch computations, in-flight consolidation candidates, short-lived observations not yet promoted to canonical.

### Scope

Scope is per-session by default. Optionally narrower (per-task) or broader (per-process, per-named-context) via an explicit scope tag.

### Physicality

Memory-only. No disk backing. No bi-temporal clocks beyond the session-local clock. Ephemeral memories do not persist across restarts; this is by design.

### Write path

Ephemeral writes take a **separate lightweight pipeline**:

1. **Type validation** — the memory must have one of the four `MemoryKind` shapes (§ 5).
2. **Symbol binding** — ephemeral writes share the symbol table with canonical. An ephemeral memory may reference canonical symbols; the binder resolves them normally.
3. **In-memory append** — to the ephemeral log for the current scope.

Skipped relative to the canonical pipeline:

- Bi-temporal edge invalidation (no `invalid_at` / `valid_at` management).
- Supersession DAG maintenance.
- Persistent canonical encoding.

This split is motivated by `PRINCIPLES.md` § 1 (precision-over-drift): canonical writes are rigorous because drift cost is high; ephemeral writes are scoped and lightweight because their drift cost is bounded by scope-end eviction.

### Promotion

Ephemeral → canonical promotion is **explicit**. An agent (or the librarian during consolidation) calls `promote(ephemeral_id) → canonical_id`; this routes the ephemeral memory through the full librarian pipeline as a new canonical write. The new canonical memory may carry `derived_from` referencing the ephemeral's context if it becomes an Inferential.

**There is no auto-promotion.** Auto-promotion rules would be ML-adjacent fuzz; explicit promotion is a deterministic gate that fits the determinism-vs-ML boundary in `PRINCIPLES.md` § 4.

### Eviction

Ephemeral memories evict terminally when their scope ends — session end, task completion, process exit, or named-scope close. Eviction is not soft; un-promoted ephemeral memories are gone.

## 5. Canonical shape (formal)

Unified Rust enum:

```rust
pub enum MemoryKind {
    Semantic {
        s: Symbol,
        p: Symbol,
        o: Value,
        source: Symbol,
        confidence: Confidence,
        valid_at: ClockTime,
    },
    Episodic {
        event_id: Symbol,
        kind: Symbol,
        participants: Vec<Symbol>,
        location: Symbol,
        at_time: ClockTime,
        observed_at: ClockTime,
        source: Symbol,
        confidence: Confidence,
    },
    Procedural {
        rule_id: Symbol,
        trigger: Value,
        action: Value,
        precondition: Option<Value>,
        scope: Symbol,
        source: Symbol,
        confidence: Confidence,
    },
    Inferential {
        s: Symbol,
        p: Symbol,
        o: Value,
        derived_from: Vec<Symbol>,
        method: Symbol,
        confidence: Confidence,
        valid_at: ClockTime,
    },
}
```

The enum is **not** `#[non_exhaustive]`. Variants are frozen at this spec's graduation. Adding a fifth type is a breaking change under `PRINCIPLES.md` § 10 (Semantic versioning).

Ephemeral memories wrap a `MemoryKind` with scope metadata:

```rust
pub struct EphemeralMemory {
    kind: MemoryKind,
    scope: EphemeralScope,
    created_at: SessionClockTime,
}

pub enum EphemeralScope {
    Session(SessionId),
    Task(TaskId),
    Process(ProcessId),
    Named(Symbol),
}
```

## 6. Type invariants and disambiguation rules

### 6.1 Invariants

Every memory satisfies:

1. **Symbol resolution.** Every `Symbol` field resolves to an entry in the symbol table at bind time.
2. **Confidence bound.** `confidence ∈ [0.0, 1.0]`. Enforced at construction via the `Confidence` newtype per `PRINCIPLES.md` § 3.
3. **Grounding completeness.** The type's grounding rule (§ 3.x) is satisfied at bind time.
4. **Librarian-assigned clocks.** `valid_at` and `observed_at` are assigned by the librarian, not by the agent. The wire may accept agent-provided values but the binder overwrites them with librarian-authoritative timestamps per `temporal-model.md`.

Type-specific:

- **Episodic:** `observed_at >= at_time`; `kind` must bind; `participants` may be empty (observer-only events).
- **Inferential:** `derived_from` is non-empty; every referenced memory ID exists in the canonical store or the current ephemeral scope; `method` is in the registered inference-method set.
- **Procedural:** `scope` resolves to a valid scope symbol; `trigger` and `action` are non-empty Values.

### 6.2 Disambiguation rules

When the agent's intent is ambiguous across types:

**Semantic vs Inferential.** If `derived_from` is present and non-empty, the memory is Inferential. Otherwise Semantic. The grammar rejects a Semantic memory with a `derived_from` field — the write surface syntactically distinguishes the two.

**Semantic vs Episodic.** If the memory is tied to a specific `at_time` and has `participants`, it is Episodic. A timeless attribute ("Alice's favorite color is blue") is Semantic. A time-tied event ("Alice chose blue at T") is Episodic. Overlapping cases resolve by whichever type's invariants the agent can populate from the information at hand.

**Procedural vs Semantic.** If the memory expresses a rule (trigger-action shape) that directs future behavior, it is Procedural. "X must happen before Y" is Procedural. "X typically happens before Y" is Semantic. Test: does this memory direct behavior (Procedural) or describe the world (Semantic)?

The grammar spec (`ir-write-surface.md`) routes the write surface's leading opcode directly to the matching enum variant. **Agents declare memory type explicitly at emission.** The librarian does not classify types heuristically; type is agent-declared and librarian-validated.

## 7. Interactions with other specs (pointers only)

- **Symbol identity:** every `Symbol` field — `symbol-identity-semantics.md`.
- **Temporal clocks:** `at_time`, `observed_at`, `valid_at` — `temporal-model.md`.
- **Grounding source taxonomy:** valid source kinds per type — `grounding-model.md`.
- **Confidence decay parameters:** per-type decay rates, activity weighting — `confidence-decay.md`.
- **Write path:** canonical pipeline stages — `librarian-pipeline.md` and `write-protocol.md`.
- **Supersession:** edge invalidation, DAG structure — `temporal-model.md`.
- **IR write surface:** the syntactic form that routes to each variant — `ir-write-surface.md`.

## 8. Open questions and non-goals for v1

### 8.1 Open questions (not blocking this spec's graduation)

**Inference-method registry.** The set of valid `method` symbols for Inferential memories needs a formal enum. Candidates derived from the bake-off corpus:

`@direct_lookup`, `@majority_vote`, `@citation_link`, `@analogy_inference`, `@pattern_summarize`, `@architectural_chain`, `@dominance_analysis`, `@entity_count`, `@interval_calc`, `@feedback_consolidation`, `@qualitative_inference`, `@provenance_chain`, `@pending_primary_source_verification`.

The final list and the deterministic evaluation rule for each method are defined in `librarian-pipeline.md` § Inferential methods.

**Compound events.** Can an Episodic memory carry a sub-event list (compound observation)? Current spec says no — agents decompose into multiple Episodic memories and link them via an Inferential memory (`method=@pattern_summarize` or similar). Revisit if real workloads demand first-class compound events.

**Temporal (time-series) memories.** Repeated measurements of the same attribute over time. Current spec represents these as a sequence of Episodic memories consolidated by an Inferential memory on query. Revisit if query patterns demand first-class time-series support.

**Procedural chaining.** Can one Procedural rule's action invoke another Procedural rule's trigger? Currently out of scope for this spec; behavior is defined at the librarian level (`librarian-pipeline.md`) or not at all in v1.

### 8.2 Non-goals for v1

- **Spatial memories.** Memory of physical location or layout as a distinct type. Not modeled.
- **Skill / capability memories.** Agent skill capture as a first-class type (as in Evolver's capsules) is not modeled. A skill can be encoded as a Procedural memory plus linked Inferential memories; we do not add a dedicated type.
- **Emotion / affect memories.** Not modeled.
- **Librarian-inferred type.** Agents always declare memory type at emission. The librarian validates; it does not infer type from content. Heuristic classification is an anti-pattern under determinism-first.

## 9. Primary-source attribution

All entries are verified per `docs/attribution.md`. None is load-bearing for the type system specified above — the shapes and rules derive from Mimir's architectural principles and the bake-off corpus, not from these sources.

- **LangMem** (✓ verified 2026-04-17; see `docs/attribution.md` § Verified sources) — primary citation for the three-way semantic / episodic / procedural agent-memory split. LangMem confirmed no equivalent of Mimir's Inferential or Ephemeral types; those are Mimir-specific additions. LangMem's scope per type is narrower than Mimir's (interaction-focused rather than general-knowledge-focused), but the definitions are subset-compatible — Mimir's types extend LangMem's, they do not contradict them.
- **Tulving (1972), *Episodic and Semantic Memory*** (✓ verified 2026-04-17; see `docs/attribution.md` § Verified sources) — foundational cognitive-psychology distinction between event-tied (episodic) and general-knowledge (semantic) memory. Supports 2 of Mimir's 4 memory types. Does **not** support procedural memory as a third type (that attribution belongs to later cognitive-psychology work + LangMem's agent-memory adaptation — LangMem still pending). Tulving 1972's framing is pre-theoretical (p. 384): the distinction is "a convenience of communication" rather than a structural-systems claim; Mimir's commit to typed categorical separation is a stronger engineering position than Tulving 1972 takes, justified by determinism-first design principles.
- **Park et al. (2023), *Generative Agents: Interactive Simulacra of Human Behavior*** (✓ verified 2026-04-17; see `docs/attribution.md` § Verified sources) — their reflection-based consolidation produces an analogue of Inferential memories, implemented via LLM synthesis of higher-level memories from lower-level observation streams. **Mimir's rejection of LLM-reflection in favor of deterministic consolidation is a known-risk architectural bet**, not a clean rejection: Park's ablation study demonstrates reflection is load-bearing for long-horizon agent coherence (removing it degrades multi-day planning within 48 simulated hours). Mimir's wager is that deterministic graph-rewrite consolidation via registered Inferential methods (`librarian-pipeline.md` § 5: `@pattern_summarize`, `@conflict_reconciliation`, etc.) can provide equivalent coherence benefit without ML unpredictability. If empirical use demonstrates deterministic methods are insufficient, the `PRINCIPLES.md` § 4 determinism-vs-ML boundary permits adding ML-proposed Inferential methods wrapped in deterministic commit decisions (`librarian-pipeline.md` § 6) — the Mimir-compatible analog of reflection. Also notable: Park et al. do **not** use a semantic/episodic/procedural split; they use a unified memory stream. Mimir's 3-way base split is attributed to LangMem and Tulving, not Park.
