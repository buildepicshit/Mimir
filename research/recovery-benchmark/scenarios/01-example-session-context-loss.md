# Scenario 01 — worked example — 2026-04-20 session context loss

> **Status:** Illustrative example, not a real benchmark run. Uses this session's *actual* state as ground truth to demonstrate scenario shape. Additional illustrative scenarios now exercise fresh-machine recovery, adapter drift, and quorum handoff loss; real production scenarios still require operator confirmation before live runs. See [`../README.md`](../README.md) § "What the operator needs to provide."
>
> **Purpose of this example:** show what a scenario spec looks like end-to-end — situation, cold-start prompt, ground-truth checklist, load-bearing decisions, staleness items, illustrative baseline sketches — using content that is already independently verifiable from `git log`, memory files, and the merged planning docs.

## Situation

On 2026-04-20 the operator and their Claude Code agent produced a meaningful chunk of work in a single session: a mission scope reframe (PR #6), a delivery plan (PR #7), a merge wave that landed seven PRs to main, and a first scouting pass for two benchmarks (A and B). The session also crystallized several design principles that had not yet been durable (agent-native runtime surface, skunkworks pace, three-pillar mission: BC/DR + memory graduation + sanitized sharing).

At the end of that session, the operator's local Claude Code state is wiped — `.claude/` directory corrupted, machine moved, compaction dropped the load-bearing detail, whichever concrete failure mode fits the scenario. The operator returns and wants to resume work on the same project with a fresh agent.

## Cold-start prompt

The operator types, once the fresh agent is active:

> *"I'm back. What were we working on, and what's the next step?"*

Identical prompt across all four baselines.

## Ground-truth checklist

Each item is a fact the agent should surface during recovery. Independently verifiable from `git log`, `docs/planning/2026-04-20-*.md`, the operator-specific memory files under `.claude/projects/.../memory/`, or a direct `mimir_read` query against the Mimir log.

### Operator profile (5 items)
1. Operator is Alain Dormehl, owner of Mimir / BES Studios.
2. Engagement protocol: Propose → Wait → Execute → Report → Stop.
3. CI quota is a hard rule: verify locally before every push; GitHub Actions monthly budget has been burned through twice.
4. Operates under skunkworks pace: Mimir stays private until we are happy it delivers on its promises; Phase 5 flip is phase-ordering, not a deadline.
5. Fedora 43 tooling: `ld.bfd` segfaults on linking large Rust workspaces; uses `mold` linker.

### Project state (5 items)
6. Main is at commit `6e6bcd6`; seven PRs merged 2026-04-20 (`#1 → #6 → #7 → #3 → #5 → #4 → #8`).
7. 463 tests passing in the workspace.
8. GitHub Actions quota exhausted as of 2026-04-20 — all recent CI runs fail in ~2 s with empty `steps` arrays (the exhaustion fingerprint). Workflow state API still reports `active`; this is not a config bug.
9. Three planning docs on main: `2026-04-19-roadmap-to-prime-time.md`, `2026-04-20-mission-scope-and-recovery-benchmark.md`, `2026-04-20-delivery-plan.md`.
10. Four pre-flip deliverables in the delivery plan: A (parse-rate benchmark execution), B (recovery benchmark v0), C (Mode 1 client integration), D (public-readiness pass).

### Load-bearing decisions (5 items — auto-fail if hallucinated wrong)
11. Mimir's **primary** value proposition is catastrophic-loss recovery (BC/DR). Token savings, cross-session continuity, proactive recall are bonuses, not the thesis.
12. Three-pillar mission: BC/DR + memory graduation (specific → broad) + sanitized cross-agent sharing. Mimir is **not** a daily-memory replacement.
13. **Agent-native at the runtime surface** — MCP responses, retrieved payloads, recovery digests must be token-dense and structured; human-readable output is confined to the observability surface (mimir-cli, docs, STATUS.md).
14. Phase 5 public flip is gated on A + B + C + D completing — *not* on any external deadline.
15. Two "modes" under consideration for the librarian — Mode 1 (Claude-as-librarian + client integration) and Mode 2 (local Ollama arbiter implementing the already-spec'd § 6.2 `InferenceProposer` hook) — are **extensions** of the shipping design, not a realignment.

### Recent feedback / open work (5 items)
16. Two CHANGELOG conflicts resolved during the 2026-04-20 merge wave by concatenating "both sides added `### Added`" sections; no content dropped.
17. Four memory files were saved this session (value prop, mission, agent-native, skunkworks) and one was updated (CI quota diagnostic fingerprint added).
18. Immediate next step per the delivery plan: scout A and B in parallel while CI quota is out. Execution of both is blocked on an operator-provided batched unblock (API key for A; real scenarios + ground truth + thresholds for B).
19. A's scaffold is verified ready (corpus parses 100/100 via `verify_corpus.py` on 2026-04-20).
20. There is **no** production Anthropic API dependency in Mimir; the Phase 3.2 benchmark uses the API as a measurement instrument only.

**Load-bearing decision count:** 5 (items 11–15). Any confident-but-wrong claim on these triggers auto-fail for that baseline.

## Staleness test items

Facts that were superseded during the session. The agent should **not** surface the earlier version as current.

- **S1.** There was an early framing where the benchmark would use the Anthropic API as Mimir's operating surface. This was superseded when the operator challenged the framing; the durable decision is "production has no API dependency; API is measurement-only." Agent must not surface the earlier framing as current.
- **S2.** An initial proposal had the benchmark comparing "five invented scenarios." This was superseded by the advisor-guided "one worked example + blanks for operator to fill" pattern. Agent must not surface the five-scenarios plan as current.
- **S3.** The initial merge plan was "start with #1 and #6 myself, or do you want to review/merge them?" — ambiguity was resolved by the operator's "Yes lets go." Agent must not surface the ambiguity as unresolved.

## Illustrative baseline sketches (not executed)

*This section is illustrative only* — projecting what each baseline would likely produce on this scenario based on how each baseline is constructed. The scorecard is not filled in; a real run would populate `results/01/scorecard.md`.

- **A (no memory).** Near-complete recovery failure. Fresh agent has access to `git log` on the repo, so can reconstruct items 6, 7, 9, 10, 16 from commits + doc presence. Items 1–5, 11–15, 17–20 largely silent or hallucinated. Very likely to hallucinate at least one load-bearing decision.
- **B (preserved markdown directory).** `.claude/projects/.../memory/` preserved. Recovers operator-profile items strongly (1–5), load-bearing decisions from the saved memories (11–15), recent-work items partially. Likely gaps: session-specific state (6, 7, 8), staleness handling (the memories are snapshots, not bi-temporal).
- **C (curated handoff doc).** Depends entirely on what the operator chose to put in the handoff. Likely strong on project state and next-step items if the operator maintained it; risk of staleness if not updated recently.
- **D (Mimir Mode 1).** Hypothesis: recovers load-bearing items strongly via `mimir_read` against graduated memories; surfaces staleness-test items correctly via supersession-aware resolver; token cost lower than B on equivalent recall because the retrieved payload is structured rather than full markdown. This is the hypothesis that the benchmark actually tests.

## Scenario limitations

- **Self-referential.** Uses this session as ground truth; a real scenario would concern work the operator personally cares about losing, not the session's own content. The production scenarios must come from the operator.
- **Cleanly observable.** All ground-truth items here are independently verifiable from repo artefacts. Real scenarios may include facts that were *only* in the agent's head — harder to specify ground truth for, and arguably more important.
- **One operator, one trial.** Statistical considerations do not apply. Single-operator qualitative.

## What a real scenario entry looks like

A production scenario replaces the *content* of this file (sections "Situation" through "Staleness test items") but keeps the structure. `results/<scenario-id>/` holds the transcripts and scorecard; the scenario file itself is the spec, not the result.
