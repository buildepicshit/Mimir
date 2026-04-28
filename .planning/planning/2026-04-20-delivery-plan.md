# Delivery plan — from here to public-flip ready

> **⚠ SUPERSEDED 2026-04-21** by [`2026-04-21-rolls-royce-engineering-plan.md`](2026-04-21-rolls-royce-engineering-plan.md).
>
> **2026-04-27 CI update:** Historical quota-exhaustion notes below are superseded for current operation. The owner added more GitHub Actions usage and approved re-enabling Actions for `buildepicshit/Mimir`; the current rule is still local gate first, one batched push, no empty retry commits, and no transient-infra reruns without owner approval.
>
> This document's four-deliverable framing (A / B / C / D) was built around a testing-first mindset — run pilots early, annotate the confounds, iterate. That proved to be spinning. The replacement plan inverts it: build the full engineering harness for apples-to-apples comparison first, then testing becomes a one-command invocation that produces a real signal. The new plan also absorbs the 2026-04-20 architecture pivot (agents write prose, not Lisp; librarian structures and commits) which invalidated Deliverable A's existential gating and redefined Deliverable C.
>
> Specifically:
>
> - **Deliverable A** (Phase 3.2 parse-rate benchmark) is no longer an existential gate. The private scouting writeup is not part of the public tree.
> - **Deliverable B** (qualitative recovery benchmark v0) is reshaped into the Rolls Royce plan's categories 7–9 (BC/DR plumbing, benchmark harness, corpus). Pilot 01 (PR #17) was closed unmerged as an unfair-comparison historical record; see the closure note on that PR.
> - **Deliverable C** (Mode 1 client integration) maps to the Rolls Royce plan's categories 3 + 4 (client integration bundle + cold-start rehydration Skill), now explicitly scoped as real-distribution engineering rather than a prototype skill.
> - **Deliverable D** (public-flip readiness) is marked **deferred** in the new plan — post-first-pilot-that-succeeds, not pre-pilot.
>
> Content below is retained for historical lineage. Do not plan new work against this document; use the Rolls Royce plan.

---

> **Document type:** Planning — living checklist from current state to the Phase 5 public-flip readiness bar.
> **Last updated:** 2026-04-20 *(superseded 2026-04-21)*
> **Status:** Historical — superseded. Do not update.
> **Cross-links:** [`2026-04-19-roadmap-to-prime-time.md`](2026-04-19-roadmap-to-prime-time.md) · [`2026-04-20-mission-scope-and-recovery-benchmark.md`](2026-04-20-mission-scope-and-recovery-benchmark.md) · [`../../AGENTS.md`](../../AGENTS.md) · [`../../STATUS.md`](../../STATUS.md)

## 0. Operating context

- **Skunkworks pace.** Mimir stays private until we are happy it delivers on its promises. Phase 5 (public flip) is a phase ordering, not a deadline. No external clock.
- **Primary value prop.** Catastrophic-local-loss recovery (BC/DR) is the load-bearing scenario; memory graduation and sanitized cross-agent sharing are the other two pillars; agent-native at the runtime surface is the governing design principle. See [`2026-04-20-mission-scope-and-recovery-benchmark.md`](2026-04-20-mission-scope-and-recovery-benchmark.md).
- **CI state.** Historical 2026-04-20 state: GitHub Actions free-tier monthly minutes burned through on 2026-04-20 (second burn of the month). Workflows reported `active`; all runs failed in ~2s with empty `steps` arrays — the quota-exhaustion fingerprint. Superseded 2026-04-27: additional usage exists and Actions may be enabled after owner approval, but local verification remains mandatory. Batch pushes; no speculative retries.
- **Verification discipline.** Before any `git push`, run the full local gate:
  ```
  cargo build --workspace
  cargo test --workspace
  cargo fmt --all -- --check
  cargo clippy --all-targets --all-features -- -D warnings
  ```
  Documented in [`AGENTS.md`](../../AGENTS.md). Non-negotiable regardless of CI state.

## 1. Merge hygiene

Five PRs open on 2026-04-20, zero merged. Main is at the initial-import commit. The stacking below is what the current branches actually encode; merge order respects dependencies.

| PR | Branch | Title | Base | Status |
|---|---|---|---|---|
| [#1](https://github.com/buildepicshit/Mimir/pull/1) | `chore/post-cutover-cleanup` | post-cutover cleanup (release.yml P0 + rename residue) | `main` | Merge-ready (local gate passed pre-push) |
| [#6](https://github.com/buildepicshit/Mimir/pull/6) | `docs/mission-scope-and-recovery-benchmark` | mission scope reframe and recovery-benchmark design | `main` | Merge-ready (docs-only) |
| [#3](https://github.com/buildepicshit/Mimir/pull/3) | `feat/inferential-resolver` | wire Inferential resolver (Phase 3.1) | `chore/post-cutover-cleanup` | Needs rebase onto new `main` after #1 merges |
| [#5](https://github.com/buildepicshit/Mimir/pull/5) | `fix/mimir-mcp-clock-injection` | `Clock` trait + `MimirServer::with_clock` | `chore/post-cutover-cleanup` | Needs rebase onto new `main` after #1 merges |
| [#4](https://github.com/buildepicshit/Mimir/pull/4) | `feat/parse-fluency-benchmark` | Phase 3.2 parse-rate benchmark scaffolding | `main` (likely) | Needs rebase verified after #1 merges |

**Merge order when we land the wave:** #1 → #6 → #3 → #5 → #4. Rebase each stacked branch onto the post-merge `main`, re-run the local gate, push, merge. Expect 5 CI runs to be triggered by the wave; run them only when quota allows full gating, not during exhaustion.

## 2. Pre-flip deliverables

The four deliverables below are the gate between "where we are" and "ready to flip the repo public." All are mostly local work; none is blocked by CI quota.

### Deliverable A — Phase 3.2 parse-rate benchmark (execution)

- **Goal.** Measure Claude canonical-Lisp emit success on the 100-prompt corpus shipped in [#4](https://github.com/buildepicshit/Mimir/pull/4). Commit the numbers.
- **Acceptance.** ≥98% parse success on both Claude Sonnet 4.6 and Claude Opus 4.7 across ≥N trials. Results checked into a dated benchmark report with full run logs.
- **Note on API use.** The benchmark's harness calls the Anthropic API as a *measurement instrument*. Production has no hosted API dependency; the benchmark is out-of-band infrastructure to verify the wire-surface thesis.
- **Dependencies.** [#4](https://github.com/buildepicshit/Mimir/pull/4) scaffold (usable from its branch without merging).
- **Scale.** 1–2 operator-days once API budget is in place.
- **Gate.** Required before Deliverable C — Mode 1 client integration presumes the emit surface is stable.
- **Scouting complete (2026-04-20).** Corpus verified at 100/100 parse against the `target/debug/mimir-cli` binary. Two harness styles were used: a primary CLI-dispatch path matching the production Claude-Code-over-MCP route and an SDK cross-check path.
- **First baseline executed 2026-04-20 (Sonnet 4.6, CLI harness, 100 prompts, 24m36s).** Result: **74/100 = 74%**, misses the ≥98% gate. Per-shape: `sem` 92%, `epi` 96%, `pro` 92%, **`query` 16%**. The query cliff is a wire-surface discoverability issue: grammar uses `:s` / `:p` / `:o` as single-char keywords; Claude naturally emits `:subject` / `:predicate` / `:object` (or `:subj`) and the parser rejects all of them; the few-shot never exercised a subject-filtered query to demonstrate the compressed form. Decisions pending: (1) Opus 4.7 cross-model baseline, (2) query-keyword question — expand parser / change canonical form / add query few-shot coverage — operator-level call.

### Deliverable B — Qualitative recovery benchmark v0

- **Goal.** 3–5 hand-crafted cold-start scenarios comparing Mimir against the markdown-file baselines from [`2026-04-20-mission-scope-and-recovery-benchmark.md`](2026-04-20-mission-scope-and-recovery-benchmark.md) § 8. Directional signal on whether Mimir earns its complexity over raw markdown on the BC/DR scenario.
- **Acceptance.** Each scenario scored on: time-to-productive, fact correctness, hallucination rate on "what did we decide," staleness handling, rehydration token cost. Results written up as `benchmarks/recovery/2026-04-XX-v0.md` with a go/no-go recommendation on proceeding to a rigorous version.
- **Dependencies.** None strictly. Can run against the current substrate + manual retrieval. Easier after Deliverable C lands, but qualitative v0 is designed to be cheap enough that it does not wait.
- **Scale.** 2–3 operator-days.
- **Gate.** A positive qualitative result is the go-signal for Deliverable D public-flip prep. A negative result forces a design rethink before any flip.
- **Scouting complete (2026-04-20).** Methodology, scoring rubric, and one worked example committed under [`benchmarks/recovery/`](../../benchmarks/recovery/). The remaining three production scenarios (02, 03, 04) need operator-provided ground truth — see `benchmarks/recovery/README.md` § "What the operator needs to provide." One batched operator decision unblocks both A (API key) and B (3 real scenarios + ground truth + pass/fail thresholds + baseline-C handoff docs).

### Deliverable C — Mode 1 client integration

- **Goal.** Wire Claude-as-librarian into the natural interaction loop: Skills, hooks, harness, and `CLAUDE.md` conventions so writes happen on memory-worthy events and retrievals happen on cold-start without the operator issuing explicit tool calls.
- **Acceptance.**
  - A Claude Skill bundle (or equivalent harness surface) that invokes `mimir_write` on natural memory events with local dedup against recent writes.
  - A documented cold-start query pattern the agent executes on session start / on explicit "I lost context."
  - End-to-end demonstration: in a fresh session with a wiped local memory, the agent rehydrates operator + project context from Mimir in under N queries.
- **Dependencies.** Deliverable A complete (stable emit format) and informed by Deliverable B (what recovery actually needs on the retrieval side).
- **Scale.** ~1 operator-week.
- **Gate.** Required before public flip. This is the layer that makes Mimir *usable*, not just storable.

### Deliverable D — Public-readiness pass

Runs last. Only begins once A, B, and C have cleared.

- **D.1 — Agent-native render-surface audit.** Every agent-facing render site (`mimir_read`, `mimir_render_memory`, any cold-start / recovery-digest tool) inspected for agent-native-ness (token-dense, structured, no narrative filler). Decision logged on whether to keep current Lisp form or tighten further. Observability surface (mimir-cli, docs) stays human-readable; do not conflate. See [`2026-04-20-mission-scope-and-recovery-benchmark.md`](2026-04-20-mission-scope-and-recovery-benchmark.md) § 7.
  - **Scouting complete (2026-04-20).** Audit + decision log at [`2026-04-20-render-surface-audit.md`](2026-04-20-render-surface-audit.md). 7 of 9 MCP tools score ≥ 8/10 agent-native; six concrete tightening items identified (est. 300–500 tokens/session saving). Implementation PR lands after CI capacity returns.
- **D.2 — README repositioning.** First paragraph must communicate: experimental memory-health layer, not a memory-app replacement; agent-native runtime surface; BC/DR as the primary value proposition. Link to [`2026-04-19-roadmap-to-prime-time.md`](2026-04-19-roadmap-to-prime-time.md), the scope doc, and this delivery plan.
- **D.3 — CONTRIBUTING refresh.** Propose → Wait → Execute → Report → Stop engagement protocol; conventional commits; squash-merge; no AI attribution; TDD expectation; CI-quota sensitivity note.
- **D.4 — CI infrastructure fix.** Decide: self-hosted runner on operator Fedora 43 box (free unlimited minutes, runner setup overhead, security considerations) vs. paid GitHub minutes add-on (simpler, recurring cost). Execute and verify green `main`. Dependabot cadence stays monthly.
- **D.5 — SECURITY.md review.** Confirm reporting channel still accurate. Threat-model section checked against the Pillar C sanitisation framing.
- **D.6 — `v0.1.0-alpha.1` cut.** Tag, release notes, `cargo publish -p mimir-core -p mimir-cli -p mimir-mcp` in order. First public artefact under the Mimir name.
- **D.7 — Phase 5 flip.** Repo goes public; [Glama](https://glama.ai) + [`modelcontextprotocol/servers`](https://github.com/modelcontextprotocol/servers) marketplace submissions; 2nd writeup per the roadmap.

## 3. Sequencing

```
Now ─┬─► A (parse-rate benchmark execution)         ──┐
     ├─► B (qualitative recovery benchmark v0)      ──┤
     └─► (local engineering on C scouting, render
          surface inventory for D.1)                ──┤
                                                     │
Quota resets / CI capacity restored ─► Merge wave    │
  (#1 → #6 → #3 → #5 → #4)                          │
                                                     │
After A stable + B positive ─► C (Mode 1 integration)┤
                                                     │
After C ships ──────────────────► D (public-ready)   │
                                                     │
D.7 ──► Public flip                                  ┘
```

A, B, and C scouting run in parallel while quota is out. Merge wave happens when CI is restored *or* when we decide the red-X on main is acceptable. D is sequential; D.4 (CI fix) is the first D-item and unblocks the cleaner main-branch optics for the rest of D.

## 4. Open questions (tracked for resolution)

- **Quota reset date.** Unknown — likely at operator's GitHub billing-cycle anchor. Should be confirmed so the merge wave can be scheduled.
- **CI infrastructure decision.** Self-hosted runner vs. paid minutes. Decide during D.4.
- **Graduation design spike.** Pillar B mechanics (confirmation threshold, de-identification, broadcast flag, projection rules, reversibility) — resolve on paper in a follow-up design doc before any graduation code. Not on this delivery plan's critical path for the flip.
- **Rigorous recovery benchmark.** Only scope after qualitative v0 lands and shows a positive signal. Not on this plan's critical path; potential Phase 6 work.

## 5. Progress checklist

Cross off as each item lands. Update this file in-place; don't open parallel tracking docs.

### Merge hygiene
- [x] #1 merged (2026-04-20, squash `57ffa63`)
- [x] #6 merged (2026-04-20, squash `ceb82d9`)
- [x] #7 merged (2026-04-20, squash `eee3a1c` — this delivery plan itself)
- [x] #3 rebased and merged (2026-04-20, squash `1e4dd94`; 459 tests green locally)
- [x] #5 rebased and merged (2026-04-20, squash `3ab321d`; 459 tests green locally; CHANGELOG conflict resolved by concatenation)
- [x] #4 rebased and merged (2026-04-20, squash `f2bbe58`; 463 tests green locally; CHANGELOG conflict resolved by concatenation)

Merge wave completed under local-gate-only discipline (GitHub Actions quota exhausted; CI infrastructure fix is tracked under D.4). Main is at `f2bbe58`; zero open PRs.

### Deliverable A — parse-rate benchmark
- [ ] Claude Sonnet 4.6 baseline recorded at ≥98%
- [ ] Claude Opus 4.7 baseline recorded at ≥98%
- [ ] dated parse-rate benchmark results checked in
- [ ] Summary writeup committed

### Deliverable B — qualitative recovery benchmark v0
- [ ] 3–5 scenarios designed and agreed
- [ ] Baselines A/B/C/D executed on each scenario
- [ ] Scored writeup committed to `benchmarks/recovery/`
- [ ] Go/no-go call recorded

### Deliverable C — Mode 1 client integration
- [ ] Skill bundle / harness surface shipped
- [ ] Cold-start query pattern documented
- [ ] End-to-end recovery demonstration recorded
- [ ] `CLAUDE.md` convention published

### Deliverable D — public-readiness pass
- [ ] D.1 render-surface audit complete
- [ ] D.2 README repositioned
- [ ] D.3 CONTRIBUTING refreshed
- [ ] D.4 CI infrastructure fixed; `main` green
- [ ] D.5 SECURITY.md reviewed
- [ ] D.6 `v0.1.0-alpha.1` tagged and published
- [ ] D.7 Repo public; marketplace submissions in flight
