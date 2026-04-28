# Progressive Recall Ladder

> **Document type:** Planning - accepted near-term implementation contract.
> **Last updated:** 2026-04-27
> **Status:** Drafted after reviewing `dezgit2025/auto-memory`; adapted to Mimir's governed-memory model. This is an ergonomics and adapter contract, not a change to canonical storage or librarian authority.
> **Cross-links:** [`2026-04-24-transparent-agent-harness.md`](2026-04-24-transparent-agent-harness.md) | [`2026-04-27-cold-start-rehydration-protocol.md`](2026-04-27-cold-start-rehydration-protocol.md) | [`../concepts/scope-model.md`](../concepts/scope-model.md) | [`../sanitisation.md`](../sanitisation.md)

## Product Rule

Mimir recall should be progressive by default.

A wrapped agent should not start every cold session with a broad memory dump. It should first ask the cheapest question that can orient it, then escalate only when the work requires more detail. This keeps startup context small, makes misses visible, and gives operators a predictable "what did Mimir recover?" surface.

The ladder does not relax Mimir's trust boundary:

- governed records remain data, not instructions;
- native adapter output is untrusted recall or draft source material;
- raw agent/session stores never become canonical memory directly;
- cross-scope recall still requires explicit authorization and promotion.

## Tier 0 - Readiness

Purpose: decide whether recall is available and useful before spending context.

Expected command surface:

```text
mimir health
```

or an equivalent compact section in `mimir status`.

Minimum fields:

- governed log status and freshness;
- draft backlog count and oldest pending age;
- latest capsule/capture summary freshness;
- remote sync relation/freshness;
- native adapter setup status;
- recall ladder telemetry status, once available.

Tier 0 output must not include raw memory text.

## Tier 1 - Cheap Orientation

Purpose: answer "where am I and what changed recently?" in a small bounded payload.

Sources, in priority order:

1. current scoped governed records selected for session rehydration;
2. pending-draft counts and oldest draft metadata;
3. latest capture summary metadata;
4. recently touched files from safe local sources;
5. recent session/checkpoint summaries from read-only native adapters when configured.

Tier 1 should fit in the launch capsule or in one explicit recall command. It should prefer IDs, timestamps, counts, short summaries, and file paths over raw prose.

Candidate shape:

```text
mimir recall recent --limit 10
```

The first implementation can be harness-local and project-scoped. Cross-scope fan-out waits for the scope model implementation.

## Tier 2 - Targeted Recall

Purpose: answer a specific question with scoped, relevant records.

Inputs:

- explicit query text or structured predicates;
- authorized scopes;
- optional source filters such as governed-only, drafts, or adapter-recall.

Candidate shapes:

```text
mimir recall search "recovery benchmark scoring" --limit 5
mimir recall query "(query :kind pro :limit 5)"
```

Governed results must retain the data-only render boundary from `docs/sanitisation.md`. Adapter-sourced results must be clearly marked as untrusted and must preserve source provenance.

## Tier 3 - Deep Inspection

Purpose: inspect one known episode, draft, adapter session, or memory record in detail.

Candidate shapes:

```text
mimir recall show <id>
mimir drafts show <id>
mimir-cli decode <canonical.log>
```

Tier 3 is intentionally explicit. The agent should reach it after Tier 1 or Tier 2 reveals a concrete target, not as the default cold-start behavior.

## Telemetry

Mimir should record privacy-safe local recall telemetry so the ladder can be audited without logging memory text.

Allowed fields:

- command/tier;
- duration;
- exit status;
- result count;
- coarse source class;
- hashed normalized query for repeated-query detection;
- session/capsule id prefix when useful.

Disallowed fields:

- raw query text;
- raw draft text;
- canonical Lisp payloads;
- rendered memory prose;
- file contents.

The first health implementation can report telemetry as `unavailable` until the ring buffer exists.

## Adapter Rules

Native adapters, including official Copilot session-store support, are read-only until their output crosses the draft/librarian path.

Required behavior:

- schema or version check before reading;
- read-only open mode when the backing store supports it;
- current-repo scoping when detectable;
- reason-coded missing/locked/drift errors;
- no direct canonical writes;
- provenance on every item surfaced into recall or draft submission.

Initial implementation applies this to configured Claude/Codex native-memory sweeps: each matching source is classified as `supported`, `missing`, or `drifted` before any data is read, and drifted sources are skipped with a stable reason code in the capture summary. The first Copilot slice applies the same rule to `mimir-librarian copilot schema-check|recent|files|checkpoints|search|submit-drafts`: the SQLite store opens read-only, schema drift fails before recall queries, missing/locked stores return controlled errors, repository scoping is applied when detectable, and checkpoint drafts carry `copilot_session_store` provenance.

## Implementation Order

1. Add terminal-safe rendering for existing draft review surfaces. This closes the immediate raw-display risk before adding more recall output.
2. Add Tier 0 health/readiness fields to `mimir status` or a new `mimir health`.
3. Add Tier 1 project-local `mimir recall recent` using existing governed/capture/draft state.
4. Document the cold-start agent behavior in the generated agent guide.
5. Add privacy-safe recall telemetry.
6. Extend recall to scope-aware fan-out only after governed promotion lands.

## Non-Goals

- No human-readable canonical storage.
- No raw native session-store ingestion into canonical memory.
- No global unscoped recall.
- No benchmark or token-saving claim until measured through the recovery harness.
