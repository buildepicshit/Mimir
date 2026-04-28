# Recovery benchmark (qualitative v0)

Scaffolding for Delivery Plan item **B** — the qualitative BC/DR recovery benchmark that measures whether Mimir delivers meaningfully better catastrophic-loss recovery than the markdown-file baselines. The *why* lives in [`docs/planning/2026-04-20-mission-scope-and-recovery-benchmark.md`](../../docs/planning/2026-04-20-mission-scope-and-recovery-benchmark.md) § 8 and [`docs/planning/2026-04-20-delivery-plan.md`](../../docs/planning/2026-04-20-delivery-plan.md) § 2 / B. This directory holds the *how*.

## Status

Scouting — scaffolding, methodology, scoring rubric, one worked example, four machine-readable illustrative scenario fixtures, scenario schema/ID validation, a results-skeleton initializer, per-baseline environment scaffolds, environment population validation, non-executing launch planning, materialized launch contracts, launch-contract/prompt validation, prompt-evidence transcript validation, transcript-gated score validation, cutoff/staleness/decision-denominator/integer-type validation, fixture-complete aggregate score summaries, and threshold verdict aggregation. The current scenarios validate harness shape and reviewer-auditable ground truth, but production benchmark runs still need operator-confirmed catastrophic-loss cases and curated baseline-C handoff docs. Treat invented scenarios as scaffolding, not evidence.

## What the operator needs to provide before this runs

- **Operator-confirmed catastrophic-loss scenarios.** Situations the operator has actually experienced or genuinely fears — machine wipe, `.claude/` corruption, context-compaction dropping load-bearing detail, move between machines, a session that crashed mid-work. The illustrative fixtures in `scenarios/` can seed the shape, but the operator owns which cases count as real benchmark evidence.
- **Ground truth per scenario.** What does "recovered correctly" look like — which operator-profile facts, project-state facts, open decisions, recent feedback, and open work must the cold-start agent surface within N queries to score as successful recovery?
- **A curated-handoff reference doc per scenario (for baseline C).** Not a task for Mimir or for Claude — this is the operator hand-writing what an ideal pre-prepared STATUS.md-style handoff doc would have said for that scenario. Without it, baseline C is fake.
- **Pass/fail thresholds on the rubric** (see [`SCORING.md`](SCORING.md)). Threshold choices are scenario-dependent and shape the go/no-go decision; operator-owned.

One batched decision covering all four unblocks both this benchmark and Delivery Plan item A (which separately needs `ANTHROPIC_API_KEY` to run).

## Methodology

Qualitative, single-operator, directional-signal. Explicitly **not** a publishable result — this benchmark's job is to answer one question: *"is the Mimir-vs-markdown thesis worth defending rigorously?"* A positive signal gates the Phase 6 rigorous version; a negative signal forces a design rethink before the public flip.

### Baselines

Four baselines, run per scenario:

| Baseline | What is preserved | Simulates |
|---|---|---|
| **A — no memory** | Nothing. Cold-start Claude with only the session prompt. | Total loss, no preparation. |
| **B — preserved markdown directory** | `.claude/` auto-memory directory intact. | Partial loss where only local Claude state survived, or full loss with pre-existing local backup. |
| **C — curated handoff doc** | An operator-hand-written STATUS.md-style summary for this scenario. No md auto-memory. | A well-prepared operator who maintains manual snapshots. |
| **D — Mimir Mode 1** | Canonical Mimir log + Claude-as-librarian MCP access. No other preserved state. | What this project is building. |

### Per-baseline protocol

1. **Preparation.** Set up a fresh Claude session with only the preserved state for this baseline. For D, the agent has MCP access to Mimir but no prior session context.
2. **Cold-start prompt.** Operator issues the same prompt across all four baselines for a given scenario. Prompt format: terse, realistic — e.g., *"I'm back. What were we working on?"* or *"New machine. What's the state of the project?"*
3. **Free-form recovery.** Agent does whatever it does — queries memories, reads files, asks clarifying questions.
4. **Cutoff.** Recovery attempt ends when the agent claims it's ready to work, or after N minutes (scenario-dependent; typical 10 minutes).
5. **Work-instruction test.** Operator issues one scenario-specific work instruction. The agent either (a) proceeds correctly, (b) asks for clarification that demonstrates it has the right context, or (c) proceeds incorrectly. This is the primary "recovered to productive state" gate.
6. **Scoring.** Operator applies the rubric in [`SCORING.md`](SCORING.md) against the ground-truth checklist for the scenario. Scoring is open-book — operator knows ground truth; the agent never does.

### Run procedure (per scenario)

The repository-local harness is dry-run by default and launches agents
only after every baseline contract validates and the operator supplies an
explicit scenario approval token:

```bash
./bench recovery --list
./bench recovery --scenario 01-example-session-context-loss --dry-run
./bench recovery --scenario 01-example-session-context-loss --init-results
./bench recovery --scenario 01-example-session-context-loss --prepare-envs
./bench recovery --scenario 01-example-session-context-loss --validate-envs
./bench recovery --scenario 01-example-session-context-loss --launch-plan
./bench recovery --scenario 01-example-session-context-loss --write-launch-contracts
./bench recovery --scenario 01-example-session-context-loss --validate-launch-contracts
./bench recovery --scenario 01-example-session-context-loss --execute-launch-contracts --approve-live-execution 01-example-session-context-loss
./bench recovery --scenario 01-example-session-context-loss --validate-transcripts
./bench recovery --scenario 01-example-session-context-loss --score-results
./bench recovery --summary-results
```

The planner reads structured scenario JSON and prints the baseline
transcript and scorecard artifacts the live harness writes after
contract validation and explicit approval.
Scenario JSON is validated at load time, including the expected A/B/C/D
baselines, typed ground-truth rows, typed staleness probes, and unique
ground-truth / staleness-probe IDs.
The initializer writes non-clobbering transcript placeholders,
`scorecard.md`, `scores.json`, `notes.md`, and `run-plan.json` under
`benchmarks/recovery/results/<scenario-id>/`. The environment
preparer writes non-clobbering per-baseline scaffolds under
`results/<scenario-id>/environments/<baseline-id>/`, including
`manifest.json`, prompts, preserved-state notes, setup README files, and
`materialized-inputs.json`. The environment validator checks those
materialized-input contracts, verifies declared file/directory/manual
paths, and reports which baselines are ready or blocked before launch.
The launch planner emits non-executing per-baseline launch contracts only
for baselines that passed environment validation.
The launch-contract writer materializes per-baseline `launch-contract.json`
artifacts only when every baseline environment is ready, so partial
benchmark runs cannot proceed from generated contracts.
The launch-contract validator checks that those materialized files are
present, parseable, still match the current scenario/environment
contract, and still point at untampered cold-start and work-instruction
prompt files before execution can depend on them.
The live executor refuses to run unless `--approve-live-execution`
matches the scenario id. It executes the validated contract argv, feeds
the cold-start prompt and work-instruction test through stdin, captures
stdout/stderr into the baseline transcript, writes a non-clobbering
`live-run.json`, and fills only null mechanical score fields
(`time_to_productive_minutes`, `rehydration_tokens.input`,
`rehydration_tokens.output`). Operator-graded correctness, hallucination,
and staleness fields remain manual.
The transcript validator checks that per-baseline transcript files have
been captured, no longer contain initializer placeholders, and include the
scenario cold-start prompt plus work-instruction test.
The score validator reads filled `scores.json` files, reports missing
fields while the template is incomplete, and emits per-baseline metric
summaries only once scoring is complete and captured transcripts are
present. `time_to_productive_minutes` must be within the scenario cutoff;
missed-cutoff recoveries are recorded as `DNR`, decision-assertion totals
must be greater than zero, and stale-occurrence counts must not exceed the
scenario's declared staleness probe count. Integer score fields must be
JSON numbers, not booleans or strings.
The summary command scans every scenario fixture, reports missing
`scores.json` files separately from incomplete score fields, requires
transcript evidence, and emits the default scenario/benchmark threshold
verdict only for complete scenario evidence.

1. Prepare the four baseline environments per the table above.
2. Commit a fresh `results/<scenario-id>/` directory with:
   - `transcript-A.md` — full agent transcript for baseline A.
   - `transcript-B.md` — baseline B.
   - `transcript-C.md` — baseline C.
   - `transcript-D.md` — baseline D (Mimir).
   - `scorecard.md` — filled rubric (copy from `SCORING.md` template) with each metric tallied per baseline.
   - `scores.json` — structured machine-readable scoring data.
   - `environments/` — per-baseline setup manifests, prompt files, and materialized-input contracts.
   - `notes.md` — operator commentary: what surprised, where the signal was strongest, what the scenario revealed about Mimir's gaps.
3. After all scenarios run, write `summary.md` aggregating across scenarios with a go/no-go recommendation.

### What this benchmark does not do

- **Not blinded.** The operator knows which baseline is which. Blinding is a rigorous-version concern.
- **Not statistically robust.** One operator, one trial per baseline per scenario. Noise is real; interpret directionally.
- **Not an automated regression.** Each run is manual. If automation becomes valuable, it lands under Phase 6.
- **Not measuring steady-state value.** Token cost and retrieval precision in normal operation are bonus metrics for the scope doc, not the primary question here.

## Directory layout

```
benchmarks/recovery/
├── README.md                                  — this file
├── SCORING.md                                 — rubric, metrics, threshold shape
├── scenarios/
│   ├── 01-example-session-context-loss.md     — worked example drawn from this session (illustrative only)
│   ├── 01-example-session-context-loss.json   — machine-readable scenario data for harness/scorer input
│   ├── 02-fresh-machine-recovery.*            — illustrative remote-restore scenario
│   ├── 03-native-adapter-drift.*              — illustrative adapter drift scenario
│   └── 04-consensus-quorum-handoff-loss.*     — illustrative quorum recovery scenario
└── results/                                   — per-scenario output directory, created on run
```

Machine-readable scenario files are validated by
`cargo test -p mimir-harness --test recovery_benchmark`. Each scenario
JSON file must define the four baselines, cold-start prompt,
work-instruction test, typed ground-truth checklist, and staleness tests.
Load-bearing decisions are explicitly marked as auto-fail when wrong.
The dry-run planner is covered by
`python3 benchmarks/recovery/test_bench.py`.
The same test covers result-skeleton creation and verifies reruns do not
overwrite operator-edited transcript or scorecard files. It also covers
per-baseline environment scaffold creation, `scores.json` completeness
validation, environment population validation, per-scenario metric
summary output, non-executing launch contract output, transcript
validation, materialized launch-contract writing, launch-contract
validation, transcript-gated score validation,
cutoff/staleness/decision-denominator/integer-type validation,
missing-score reporting, aggregate summary output, and threshold verdict
aggregation across result directories.

## Relationship to other research tracks

- Earlier parse-rate scouting measured whether Claude could write canonical Lisp reliably. This benchmark measures whether the written-then-retrieved flow delivers recovery value. Different questions; both matter, but only the recovery harness is shipped here as a public benchmark asset.
- Phase 6 rigorous recovery benchmark — gated on positive signal from this v0. Would be N-replicated, multi-operator, blinded; out of scope here.

## How to update this directory

- Methodology changes: edit [`README.md`](README.md) and [`SCORING.md`](SCORING.md) in place; these are living docs.
- New scenario: add a new numbered markdown file plus a same-stem JSON file under `scenarios/`; run `cargo test -p mimir-harness --test recovery_benchmark`; update the table in [`SCORING.md`](SCORING.md) if the scenario introduces a new scored dimension.
- Harness planner, result-initializer, environment-preparer, environment-validator, launch-plan, launch-contract writer, launch-contract validator, transcript-validator, score-validator, summary, or verdict changes: run `python3 benchmarks/recovery/test_bench.py`.
- Results: land as a dedicated PR per scenario batch so the transcripts + scorecard are reviewable together.
