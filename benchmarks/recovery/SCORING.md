# Recovery benchmark — scoring rubric

Five metrics, scored per baseline per scenario. Rubric granularity is where the real design risk lives — the metrics themselves come from [`docs/planning/2026-04-20-mission-scope-and-recovery-benchmark.md`](../../docs/planning/2026-04-20-mission-scope-and-recovery-benchmark.md) § 8, but how we score them is a methodology choice that shapes the go/no-go call.

## Metric 1 — Time to productive state

**What it measures.** Minutes from cold-start to the agent being able to correctly act on a scenario-specific work instruction. "Correctly act" means either executing the instruction, or asking a clarifying question that demonstrates the agent has the right context.

**How to score.** Operator times the recovery attempt. Stopwatch starts when the cold-start prompt is issued; stops when the agent either (a) produces a correct response to the work-instruction test, or (b) hits the scenario cutoff without getting there.

**Output.** Integer minutes (round up to the next minute); or `DNR` (did not recover) if the cutoff is hit without productive state reached.

**Why it matters.** This is the headline BC/DR metric. A memory layer that eventually recovers correctly but takes an hour to do so is worse for real workflows than a markdown layer that recovers 80% correctly in two minutes.

## Metric 2 — Fact correctness

**What it measures.** Of the scenario's ground-truth checklist, what percentage did the agent correctly surface during recovery?

**How to score.** Each scenario has a ground-truth checklist provided by the operator. Example items:
- Operator role and key preferences.
- Current project phase and headline goal.
- Three most-recent decisions made.
- Currently-open work / branches / PRs.
- Load-bearing constraints (e.g., CI quota, specific tooling).

During the recovery transcript, the operator ticks each checklist item as one of:
- **C — Correct** — agent surfaced the fact accurately, without prompting.
- **P — Correct on prompt** — agent surfaced correctly only after the operator asked.
- **W — Incorrect** — agent asserted the wrong fact, or confidently missing.
- **S — Silent** — agent never surfaced the fact, and operator never asked.

**Output.** Raw counts per category, and the **C** percentage of the total checklist (C-count / N).

**Why it matters.** Recovery that omits load-bearing facts silently is worse than recovery that surfaces them on prompt; both are worse than unprompted correctness. Four buckets catch the meaningful differences.

## Metric 3 — Hallucination on "what did we decide"

**What it measures.** Per scenario, a list of scenario-specific decisions (from the ground truth). How often did the agent *confidently assert* a decision that was not actually made?

**How to score.** Operator reviews the recovery transcript and tallies:
- Each confident assertion of a decision that matches ground truth → **OK**.
- Each confident assertion of a decision *not* in ground truth → **HALLUCINATION**.
- Each hedged statement ("I'm not sure, but possibly X") is not a hallucination; ambiguity is allowed.

**Output.** Ratio: hallucinations / total decision-assertions. Also:
absolute hallucination count. The total decision-assertions denominator must be
greater than zero for a completed score.

**Why it matters.** The single most dangerous recovery failure mode is the agent silently reconstructing a plausible-but-wrong decision history and acting on it. Token efficiency and recovery speed are meaningless if the recovered state is a lie.

## Metric 4 — Staleness

**What it measures.** How many superseded facts did the agent surface *as current* during recovery?

**How to score.** The scenario's ground truth notes which facts have been superseded (e.g., "we were going to use `ld.bfd` but switched to `mold`"). For each superseded fact, check whether the agent:
- Correctly surfaced only the current version → **Fresh**.
- Surfaced the superseded version as current → **Stale**.
- Surfaced both and flagged the supersession → **Qualified** (counts as fresh).
- Never surfaced either version → not counted.

**Output.** Count of **Stale** occurrences. Lower is better; zero is the goal.

**Why it matters.** Bi-temporal supersession is one of Mimir's load-bearing architectural claims. If baseline D doesn't outperform baseline B here, the claim isn't paying off in practice.

## Metric 5 — Rehydration token cost

**What it measures.** Tokens consumed by the agent during the recovery attempt (between cold-start prompt and the work-instruction test).

**How to score.** Read the token counts from Claude's session metadata (Claude Code surfaces cumulative token totals). For baselines A–C this is just the session's input+output tokens up to the cutoff. For baseline D, include the MCP round-trip payloads as counted by the session.

**Output.** Two numbers: input tokens, output tokens. Record both; interpret context-dependent.

**Why it matters.** A recovery layer that rehydrates correctly at 50k tokens is a worse product than one that rehydrates correctly at 3k — both on cost and on how much context window remains for real work. For the scope doc's "cheaper recovery = higher-value recovery" framing.

## Scorecard template

Copy into `benchmarks/recovery/results/<scenario-id>/scorecard.md`:

```markdown
# Scenario <id> — scorecard

Scenario: <one-line description>
Run date: <YYYY-MM-DD>
Operator: <name>

## Baseline A — no memory

- Time to productive state: <N minutes | DNR>
- Fact correctness: C <N> / P <N> / W <N> / S <N> — <C %> unprompted-correct
- Hallucinations: <N> out of <M> decision-assertions
- Staleness: <N stale occurrences>
- Rehydration tokens: input <N>, output <N>

## Baseline B — preserved markdown directory
(as above)

## Baseline C — curated handoff doc
(as above)

## Baseline D — Mimir Mode 1
(as above)

## Operator commentary

<What surprised. Where the signal was strongest. Gaps Mimir revealed in itself.>
```

## Pass / fail thresholds

Thresholds are scenario-dependent and **operator-owned**. Reasonable shape:

- **Baseline D must beat the best of A/B/C on at least 3 of 5 metrics** for the scenario to count as a Mimir win.
- **Hallucination count for D must not exceed B's** — Mimir must not be *less* truthful than markdown.
- **Staleness count for D must be lower than B's** — otherwise supersession isn't paying off.

An aggregate recommendation: Mimir wins the benchmark if it wins ≥2/3 scenarios under these thresholds. Operator confirms or amends the threshold shape before the first scenario runs.

`./bench recovery --summary-results` applies this default threshold shape to
complete `scores.json` files and reports `scenario_verdict` plus
`benchmark_verdict` payloads. If the operator changes the threshold shape before
a real run, update the harness and this rubric in the same PR.

## What goes on the go / no-go recommendation

After all scenarios run, `summary.md` answers:

1. **Did Mimir win the benchmark** under agreed thresholds?
2. **Which metrics drove the result** — recovery speed, correctness, hallucination, staleness, or token cost?
3. **What surprised** that should feed back into Mode 1 client-integration design (Delivery Plan item C)?
4. **Is the rigorous Phase 6 version worth the investment** — a clear yes, a clear no, or a qualified "yes, with scope changes"?
