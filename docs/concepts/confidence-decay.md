# Confidence and Decay

> **Status: authoritative 2026-04-18. Scope reduced 2026-04-18** — the `@cross_workspace_import` decay row and the `:surface_conflicts` / `Framing::Contested` surfacing references were removed following the multi-agent scope reduction. The `sem_cross_workspace_import_ms` field still exists in `DecayConfig` pending a follow-up code cleanup; no decay calculation reaches it. Graduated from `citation-verified` on 2026-04-18 backed by `mimir_core::decay`: the § 5.1 exponential formula in u16 fixed-point via a hand-hardcoded 256-entry lookup table (bit-identical bytes across builds + architectures, satisfying § 13 invariant 2 without relying on libm `powf`); the § 5.2 v1 default parameter table exposed as `DecayConfig::librarian_defaults`; § 5.3 `NO_DECAY = 0` infinity encoding; § 13 invariants 3 (pinned / authoritative skip) and 5 (runtime user sovereignty via in-memory mutation and `DecayConfig::{from_toml, apply_toml}` per criterion #4). Property tests `decay_determinism`, `decay_is_monotonic`, `pinned_or_authoritative_skip_decay`, and `no_decay_half_life_never_decays` cover the spec § 13 invariants verifiable at this layer. Deferred and explicitly flagged in the module docstring: Procedural activity weighting (§ 6), Inferential parent-tracking (§ 9 — composes with `InferenceMethod::compute` at the caller).

This specification defines how Mimir ages memories over time — the deterministic exponential-decay model, the activity-weighted variant for Procedural memories, the pinning and authoritative-flag mechanisms for suspending decay, the Inferential decay chain through current parent confidences, and the user-tunable tolerance knobs. The canonical store remains append-only; decay is a computed-on-read effective value, never a stored mutation.

## 1. Scope

This specification defines:

- Stored vs effective confidence.
- Exponential decay formula with class-parameterized half-lives.
- The v1 librarian decay-parameter defaults per `(memory-type × grounding)`, user-overridable.
- Activity weighting for Procedural memories.
- Pinning and the operator-authoritative flag.
- Inferential decay (derived from current parent effective confidences plus method factor).
- Lazy on-read computation model.
- Precision and rounding rules.
- Read-side interaction (silent-by-default, opt-in surfacing via toggles).

This specification does **not** define:

- Memory type shapes — `memory-type-taxonomy.md`.
- The source-kind taxonomy — `grounding-model.md`.
- Symbol identity — `symbol-identity-semantics.md`.
- The four-clock temporal model — `temporal-model.md`.
- Librarian pipeline stages or the inference-method registry's per-method formulas — `librarian-pipeline.md`.
- The on-disk confidence encoding (fixed-point `u16`) — `ir-canonical-form.md` § 3.1.
- Read-path query grammar — `read-protocol.md`.
- Framing metadata and conflict surfacing (forthcoming amendment to `read-protocol.md`).

### Graduation criteria

Graduates draft → authoritative when:

1. Related art is verified in `docs/attribution.md` (forgetting-curve cognitive-psychology literature; bi-temporal confidence-decay in knowledge graphs).
2. Rust decay-computation logic compiles in `mimir_core::decay`, with the invariants in § 13 covered by unit and property tests.
3. Property tests cover: bit-identical effective-confidence computation across architectures; pinned / authoritative memories do not decay; Inferential effective confidence tracks current parent effective confidences.
4. Integration tests confirm user config overrides (`mimir.toml`) take effect at runtime without librarian restart.

## 2. Design thesis: deterministic decay + user-tunable tolerances

Agent memory accumulates. Without a principled aging mechanism, every memory weighs the same as it did at write time — old observations compete with fresh ones as if time never passed. Agents working against undifferentiated memory stores produce incoherent answers.

Mimir's design:

- **Deterministic decay.** Given the same input (memory + current time + config), every client computes the same effective confidence. No ML, no stochastic sampling, no per-call randomness.
- **Stored confidence is immutable.** The canonical record carries the *base* confidence at write time. Effective confidence is a function of (stored, elapsed, class parameters, activity state) computed lazily at read.
- **User-tunable tolerances, librarian-shipped defaults.** The v1 decay-parameter table (§ 5) is a starting point. Users override per workspace via `mimir.toml`. No parameter is invariant; all are knobs.
- **Pinning and authoritative flags suspend decay.** User-controlled mechanisms for saying "this memory doesn't decay" — with distinct semantics for pinning (agent-invokable) vs. operator-authoritative (user-applied, agent-transparent).

The approach matches the principle in `PRINCIPLES.md` § 1: precision over speed. Decay adds compute at read time; the computation is deterministic so results are reproducible; the storage cost stays zero.

## 3. Stored vs effective confidence

```
stored:    Confidence  (u16 fixed-point, per ir-canonical-form.md § 3.1)
effective: Confidence  (computed lazily at read)
```

Relationship:

```
effective = stored × decay_factor(elapsed, class_params) × activity_factor (Procedural only)
```

If the memory is pinned (§ 8) or flagged operator-authoritative (§ 9):

```
effective = stored
```

Storage is canonical. Effective is a read-derived view. The decoder tool (spec 3.15) shows both.

## 4. User sovereignty

Every decay parameter and every read-side surfacing behavior is user-tunable via `mimir.toml` at the workspace level. Per-query overrides on read forms further tune behavior for specific queries.

### 4.1 What the user controls

- **Decay parameters.** Per-(memory-type × grounding) half-lives, activity-weight floor, method factors. Full parameter table in § 5 is a librarian-supplied default; `mimir.toml` overrides.
- **Tolerance thresholds.** The per-query `:confidence_threshold` default (`read-protocol.md` § 4.2), workspace-level minimum floors.
- **Pinning / authoritative flags.** Any memory can be pinned or marked authoritative at any time.
- **Surfacing toggles.** `:explain_filtered`, `:show_framing` — defaults configurable per workspace, per-query overrides always available.

### 4.2 Default stance: silent, clean, low-friction

The v1 librarian ships defaults matching the current-industry norm for agent memory: silent filtering of below-threshold memories, clean query results, no opinionated interrupts. Users who want investigation / debug mode opt in via the surfacing toggles.

### 4.3 The user decides, the agent surfaces

Mimir computes effective confidence, applies filters per current tolerances, and returns results. The user configures the rules; the agent's job is to compute + surface (within the user-chosen surface area). The agent never decides on the user's behalf whether a below-threshold memory "should" be filtered — the user's configuration decides, and the librarian executes.

## 5. Exponential decay formula and parameter table

### 5.1 Formula

```
decay_factor(elapsed, half_life) = (1/2) ^ (elapsed / half_life)
                                 = exp(-elapsed × ln(2) / half_life)
```

- `elapsed` = `now − valid_at` for non-pinned, non-authoritative memories.
- `half_life` = per-class parameter from the table below (or user override).
- `decay_factor ∈ (0.0, 1.0]`, monotonically decreasing with elapsed.

### 5.2 v1 librarian defaults (user-overridable)

| Memory type | Grounding kind | Default half-life | Notes |
|---|---|---|---|
| Semantic | `@profile` | 730 days (~2 years) | identity attributes are stable |
| Semantic | `@observation` | 180 days | direct observation moderately durable |
| Semantic | `@self_report` | 90 days | subjects misremember their own attributes |
| Semantic | `@document` | 365 days | printed sources age moderately |
| Semantic | `@registry` | 90 days | registries mutate (package versions, DNS) |
| Semantic | `@external_authority` | 180 days | service-returned facts age |
| Semantic | `@agent_instruction` | 730 days | operator directives are durable intent |
| Semantic | `@librarian_assignment` | ∞ (no decay) | timestamps, symbol IDs, internal facts |
| Semantic | `@pending_verification` | 30 days | unverified claims age fastest |
| Episodic | `@observation` | 90 days | events fade |
| Episodic | `@self_report` | 30 days | self-reported events fade fastest |
| Episodic | `@participant_report` | 60 days | intermediate |
| Procedural | any | ∞ (no time decay) | activity-weighted instead — see § 6 |
| Inferential | any | bound by current parent effective confidences × method factor — see § 9 | derived memories track their parents |

Defaults live in the librarian's compiled-in table; user overrides live in `mimir.toml`:

```toml
[decay.semantic]
profile = 730
observation = 180
# ... override whichever keys you like; unlisted keys fall back to librarian defaults
```

### 5.3 Infinity encoding

An `∞` half-life is encoded as `0` in `mimir.toml` (the user writes `librarian_assignment = 0`); internally the decay routine short-circuits to `decay_factor = 1.0` when half-life is 0, avoiding division-by-zero.

## 6. Activity weighting for Procedural memories

Procedural memories (rules, policies) do not time-decay — a rule is active or superseded, not gradually stale (per `memory-type-taxonomy.md` § 3.3). But unused rules may warrant flagging as potentially stale, while frequently-fired rules accumulate implicit trust.

### 6.1 Firing events

Each time a Procedural rule is matched + its action invoked, the librarian records an Episodic memory:

```
(epi <fresh_id> @proc_fired (@rule_id) @librarian :at T :obs T :src @librarian_assignment :c 1.0)
```

The `participants` field carries the rule ID. These Episodics are cheap (one per firing) and constitute the rule's activity trace.

### 6.2 Activity-weight formula

```
last_fire = max(Episodic[kind=@proc_fired, participants contains @rule].observed_at)
days_since_last_fire = (now - last_fire) / 86_400_000  # ms to days

activity_factor = max(floor, exp(-days_since_last_fire / activity_half_life))
```

v1 defaults:
- `floor = 0.3` (rules never drop below 30% activity weight)
- `activity_half_life = 180 days`

Rules fired recently weight ~1.0; rules fired 6 months ago weight ~0.5; rules never fired (or not fired in ~2 years) weight at the 0.3 floor.

### 6.3 Never-fired rules

A rule allocated but never fired has no `@proc_fired` Episodics. The librarian treats `last_fire = rule.committed_at` in that case, so `activity_factor` starts at `1.0` and decays from rule-creation time. After ~2 years of never firing, the rule reaches the 0.3 floor.

### 6.4 User override

Users can override `floor` and `activity_half_life` per workspace:

```toml
[decay.procedural]
activity_floor = 0.3
activity_half_life = 180
```

## 7. Pinning

### 7.1 Agent-invokable pinning

Any agent with write access can pin a memory:

```
(pin @memory_id)
```

Effect: a `pinned: true` flag is set on the memory's canonical record (via a `SYMBOL_*`-analog pin record in `canonical.log` — concrete opcode registration is a spec-3.7 follow-up). While pinned:

```
effective = stored    # no time decay, no activity weighting
```

Pinning emits an Episodic `@pin` event for audit.

### 7.2 Unpinning

```
(unpin @memory_id)
```

Emits an Episodic `@unpin` event. Decay resumes — but from the memory's **original** `valid_at`, not from unpin time. Pinning **suspends** decay rather than **resets** it. A memory pinned for 5 years and then unpinned behaves as if its 5-year-old age matters, not as if it's newly written.

### 7.3 Pinning is not authoritative

Pinning suspends decay but does not affect the `Framing` a memory receives at read time — pinned memories are still `Advisory` unless separately flagged as operator-authoritative.

## 8. Operator-authoritative flag

### 8.1 User-applied, agent-transparent

Distinct from pinning: the operator-authoritative flag signals "this memory is user-declared authoritative; the agent should not question its validity." Agent tools cannot set this flag; only the user (via CLI or `mimir.toml` per-memory entry):

```
mimir-cli authoritative @memory_id --on
mimir-cli authoritative @memory_id --off
```

Equivalent to pinning in decay effect (`effective = stored`) plus a `Framing::Authoritative { set_by: User }` label on read results (per the forthcoming `read-protocol.md` amendment).

### 8.2 Audit

Setting / clearing the authoritative flag emits Episodic events (`@operator_authoritative_set`, `@operator_authoritative_cleared`) so the decoder tool reconstructs the flag's history.

### 8.3 Authoritative is trust, not erasure

Authoritative framing does not delete or hide prior memories — it pins a specific memory as high-trust without modifying the append-only record of alternatives. The supersession chain and prior observations remain inspectable via `mimir-cli inspect`; authoritative is a label on the current-state pick, not a censor on history.

## 9. Inferential decay

Inferential memories derive their effective confidence from their parents at read time:

```
inferential.effective = min(parent.effective for parent in derived_from) × method_factor
```

Where:

- `parent.effective` is the parent's current effective confidence, recursively computed (base case: non-Inferential memories decay normally per § 5).
- `method_factor` is the per-inference-method factor from `librarian-pipeline.md` § 5. Examples: `@direct_lookup` = 1.0, `@pattern_summarize` = 0.8, `@qualitative_inference` = 0.5.

### 9.1 Inferential's own `valid_at` is not the decay anchor

Unlike Semantic / Episodic memories, an Inferential memory's effective confidence is **not** computed from its own `valid_at` — it's computed from its *current parents' effective confidences*. This is the mechanism by which parent decay / supersession propagates through derived conclusions automatically.

### 9.2 Recursive decay

If a parent is itself Inferential, its effective confidence is recursively derived from its parents. Cycles are forbidden by `grounding-model.md` § 5.1 (acyclic provenance DAG) so recursion terminates.

### 9.3 Stale-parent propagation

When any parent is superseded, the Inferential is flagged *stale* (per `memory-type-taxonomy.md` § 3.4 and `temporal-model.md` § 5.4). The Inferential continues to return its stored value's effective confidence (computed from the **pre-supersession** parents) until explicit re-derivation produces a new Inferential that supersedes it.

## 10. Lazy on-read computation + precision

### 10.1 No background decay jobs

Decay is computed **lazily at read time**. No scheduled jobs, no background rewrites of canonical records, no eager propagation.

### 10.2 Computation path

On each read-path result:

```
1. Load stored confidence from canonical record.
2. Determine elapsed = now - valid_at (or use parent recursion for Inferential).
3. Lookup half-life from (memory-type, grounding) in the workspace's current config.
4. Apply decay formula.
5. If Procedural: apply activity factor.
6. If pinned or authoritative: skip steps 2-5, set effective = stored.
7. Round to u16 fixed-point for the result.
```

### 10.3 Precision

Intermediate computation in `f32`. Input confidence and output effective confidence are `u16` fixed-point (1.53e-5 resolution per `ir-canonical-form.md` § 3.1). Rounding on conversion is round-half-to-even.

### 10.4 Determinism across clients

Any client running the same librarian version with the same workspace config computes bit-identical effective confidence for the same `(memory, query_time)` pair. This matches `PRINCIPLES.md` § 4 determinism-vs-ML boundary: decay is fully deterministic.

## 11. Read-side interaction: silent default, opt-in surfacing

### 11.1 Default behavior

Reads silently filter memories whose effective confidence falls below the query's threshold (default 0.5, overridable via `:confidence_threshold`). Results contain only memories that pass. No noise, no surfaced filter reasons.

This matches how Claude Code, ChatGPT, Letta, and Mem0 present memory today — clean results, no friction.

### 11.2 Surfacing toggles

Users can opt into visibility at the workspace or per-query level:

- **`:explain_filtered true`** — returns filtered memories in a separate `filtered` array on the `ReadResult`, each with `{ memory_id, effective_confidence, filter_reason }`.
- **`:show_framing true`** — attaches `Framing` metadata to every result record (`Advisory`, `Historical`, `Authoritative`, `Projected`).
- **`:debug_mode true`** — shorthand for both above.

Workspace defaults live in `mimir.toml`:

```toml
[read_defaults]
explain_filtered = false
show_framing = false
debug_mode = false
```

### 11.3 Cross-reference

The `Framing` enum mechanics live in `read-protocol.md`. This spec constrains their *semantics* with respect to decay: pinned / authoritative memories have effective = stored.

## 12. Interaction with other specs

- **`temporal-model.md` § 5:** supersession sets `invalid_at`; as-of queries respect invalidation. Decay is computed at the query's `as_of` time, not at `now`, for historical queries.
- **`read-protocol.md`:** default threshold and escalation via `inspect()`. Read-after-write consistency is automatic under the synchronous in-process API (`wire-architecture.md` § 3.3) — every read after a successful `commit_batch` return sees that Episode's committed state.
- **`grounding-model.md` § 4:** source-confidence upper bounds apply at **write time** (stored confidence cannot exceed bound); decay further lowers effective confidence at read.
- **`librarian-pipeline.md` § 5:** inference-method confidence formulas establish the Inferential's *stored* confidence at bind; § 9 of this spec computes the Inferential's *effective* confidence at read.

## 13. Invariants

1. **Append-only stored confidence.** A memory's `stored` confidence never changes after its write. Decay affects only the computed effective value.
2. **Deterministic decay.** Same input + same config + same query time produces bit-identical effective confidence.
3. **Pinned / authoritative skip decay.** `effective = stored` for pinned or operator-authoritative memories.
4. **Inferential decay tracks current parents.** Inferential effective confidence is recomputed from current parent effective confidences at every read.
5. **User sovereignty over parameters.** Every decay parameter is user-overridable. No parameter is an invariant.
6. **No silent information loss.** Filtered memories and framing metadata are always recoverable via the surfacing toggles. Default behavior is silent; information is not thrown away.
7. **Agent truthfulness.** The agent does not misrepresent information on the read result. In debug mode, filtered memories reach the user; in silent mode, the agent acts on what it sees without inventing or suppressing detail.
8. **Authoritative is trust, not erasure.** Operator-authoritative framing suspends decay but never deletes or hides prior memories; the append-only record remains inspectable.

## 14. Open questions and follow-ups

### 14.1 Follow-up spec amendments (same branch or short follow-up PR)

- **`ir-canonical-form.md` amendment:** opcodes for `PIN`, `UNPIN`, `AUTHORITATIVE_SET`, `AUTHORITATIVE_CLEAR` in the 0x30–0x3F range.

### 14.2 Open questions

**Non-exponential decay curves.** Some memory domains may suit sigmoidal or step-wise decay better than exponential (e.g., event reports that stay fresh for N days then drop off). v1 ships exponential only; richer curves are post-MVP.

**Per-symbol decay overrides.** Currently decay is parameterized by `(memory-type × grounding)`. Users may want per-symbol overrides ("decay memories referencing `@critical_system` slower than others"). Post-MVP.

**Retroactive parameter changes.** If a user changes a half-life from 180 to 90 days, memories previously returned at effective = 0.7 will now return at effective = 0.5 for the same query time. This is intentional (the user retuned the model), but may surprise. v1 accepts this; skill documentation teaches it.

### 14.3 Skill-document dependency

The agent-facing skill document (post-Phase-3 artifact, candidate name `skill/mimir-consumer.md`) must cover:

- What effective confidence means operationally (rules of thumb: 0.9+ high trust, 0.5 review-recommended, 0.1 unreliable).
- How to interpret per-source-kind confidence caps from `grounding-model.md` § 3.
- When to enable debug mode vs when to stay silent.
- When to escalate via `inspect()` vs when to proceed on the hot path.
- How to ask the user to override tolerances or mark a memory authoritative.

This is not in Phase 3 scope but referenced in this spec so the contract isn't lost.

## 15. Primary-source attribution

All entries are verified per `docs/attribution.md`.

- **Ebbinghaus, *Über das Gedächtnis* (1885), and forgetting-curve literature** (verified) — foundational work on exponential memory decay in cognitive psychology. Mimir's exponential half-life model parallels this framework.
- **Murre & Dros, *Replication and Analysis of Ebbinghaus' Forgetting Curve*, PLOS ONE 2015** (verified) — modern replication; confirms exponential form with half-life parameterization.
- **Zep / Graphiti documentation — bi-temporal confidence in knowledge graphs** (already pending) — closest architectural cousin; cited for the stored-vs-effective distinction.
- **Pearl, *Probabilistic Reasoning in Intelligent Systems*, 1988** (verified, already cited for Librarian Pipeline) — the belief-revision framework that informs `method_factor` in § 9.
