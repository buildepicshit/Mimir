# Architectural Specifications

This directory holds Mimir's architectural specifications. Each spec graduates here from in-chat draft once both conditions hold:

1. The concept is formally specified to a level implementable via TDD (per `PRINCIPLES.md` § Engineering Standards).
2. Primary sources cited by the spec are verified against the actual literature, not training memory. See `../attribution.md` for the verification log.

## Current specs

Every spec carries a `> **Status: <state>**` banner in its first paragraph. As of 2026-04-20, the original **14 implementation specs are `authoritative`** — drift detected by `crates/mimir_core/tests/doc_drift_tests.rs::status_banner_consistency` on every CI run.

As of 2026-04-24, Mimir also has draft mandate-expansion specs for [`scope-model.md`](scope-model.md) and [`consensus-quorum.md`](consensus-quorum.md). They reflect the accepted shift from absolute workspace isolation to scoped memory governance plus governed cross-agent deliberation. They are not fully implemented yet and do not make the older implementation specs false; they define the next architecture layer those specs must evolve toward.

The accepted product launch-boundary direction is now part of the public quickstart: users launch normal agents as `mimir <agent> [agent args...]`, and Mimir preserves the native terminal flow while wrapping the session with governed memory. See [`../../README.md`](../../README.md#running-mimir) and [`../first-run.md`](../first-run.md).

## The 14 authoritative implementation specs

- [`memory-type-taxonomy.md`](memory-type-taxonomy.md) — semantic / episodic / procedural / inferential + ephemeral tier.
- [`grounding-model.md`](grounding-model.md) — source taxonomy, confidence bounds, provenance chains.
- [`symbol-identity-semantics.md`](symbol-identity-semantics.md) — symbol allocation, rename propagation, alias chains, retirement flags.
- [`temporal-model.md`](temporal-model.md) — four clocks, supersession via edge invalidation, DAG merge.
- [`ir-write-surface.md`](ir-write-surface.md) — Lisp S-expression grammar (v1).
- [`ir-canonical-form.md`](ir-canonical-form.md) — positional bytecode, opcode table, symbol-table layout.
- [`librarian-pipeline.md`](librarian-pipeline.md) — lexer, parser, binder, semantic analyzer, emit; determinism-vs-ML boundary.
- [`read-protocol.md`](read-protocol.md) — hot path, escalation triggers, stale-symbol semantics.
- [`write-protocol.md`](write-protocol.md) — checkpoint triggers, episode atomicity, rollback, failure taxonomy.
- [`episode-semantics.md`](episode-semantics.md) — formation, linking, query semantics.
- [`confidence-decay.md`](confidence-decay.md) — parameter tables, activity weighting, pinning.
- [`workspace-model.md`](workspace-model.md) — workspace identity, partition layout, isolation invariants.
- [`wire-architecture.md`](wire-architecture.md) — agent API contract (in-process-only; daemon / async queue / `:read_after` predicate dropped 2026-04-19).
- [`decoder-tool-contract.md`](decoder-tool-contract.md) — inspection-tool CLI/API surface and round-trip guarantees.

## Draft mandate-expansion specs

- [`scope-model.md`](scope-model.md) — multi-agent memory governance, scope taxonomy, trust tiers, and promotion rules.
- [`consensus-quorum.md`](consensus-quorum.md) — cross-agent, cross-model deliberation episodes whose outputs enter memory only through the draft/librarian path.

A previously-planned 15th spec (`multi-agent-coherence.md`) was deleted on 2026-04-18 as out of scope. The 2026-04-24 mandate shift does not restore free-form multi-agent write coordination. It introduces governed memory promotion across explicit scopes and governed deliberation episodes whose artifacts remain separate from canonical memory until the librarian accepts them.

## Adding a new spec

If you're proposing a new architectural concern that doesn't fit an existing spec:

1. Open it as a draft in chat or in an ignored local scratch area until both graduation conditions above hold.
2. When ready, add a file to this directory with the `> **Status: draft|citation-verified|authoritative**` banner in its first paragraph (the drift gate at `crates/mimir_core/tests/doc_drift_tests.rs::status_banner_consistency` enforces the format).
3. Cite primary sources; record citation verification in `../attribution.md`.
4. Link the concept from the public documentation index when it becomes part of the supported surface.
