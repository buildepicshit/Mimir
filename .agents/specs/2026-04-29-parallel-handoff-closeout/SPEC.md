---
id: mimir-parallel-handoff-closeout-2026-04-29
status: owner-paused
owner: HasNoBeef
repo: Mimir
source_specs:
  - root:.agents/specs/2026-04-29-fleet-realignment-and-handoff/SPEC.md
  - .agents/specs/2026-04-29-realignment-handoff/SPEC.md
  - .agents/specs/2026-04-29-repo-audit/SPEC.md
branch_policy: local-only-public-oss-parallel-lane
risk: medium
requires_network: false
requires_secrets: []
acceptance_commands:
  - "node ../.agents/scripts/preflight.mjs"
  - "git status --short --branch --untracked-files=all"
  - "cargo build --workspace"
  - "cargo test --workspace"
  - "cargo test --workspace --all-features"
  - "cargo fmt --all -- --check"
  - "cargo clippy --all-targets --all-features -- -D warnings"
  - "cargo deny check"
  - "cargo doc --workspace --no-deps"
---

# SPEC: Mimir Parallel Handoff Closeout

## 1. Problem

Mimir still has local handoff/setup work after the BES fleet realignment pass.
The repo is public OSS and CI-budget-sensitive, so the remaining closeout must
preserve local work, avoid public noise, and define the exact point at which a
green room product evaluation may safely begin.

This spec is a local agent-control handoff artifact. It does not approve product
code, public docs, commits, pushes, tags, releases, or publication.

## 2. Goals

- Record the current Mimir branch, head, dirty state, in-flight work, and
  verification gates from fresh command output.
- Define a public-OSS-safe parallel closeout lane that is disjoint from other
  BES handoff workers.
- Identify owner decisions needed before any Mimir product closeout, public docs
  change, PR, push, tag, or release.
- Define stop conditions that protect user work, public OSS posture, and CI
  quota.
- State that Mimir green room evaluation may begin only after this closeout is
  done or the owner explicitly marks it paused.

## 3. Non-Goals

- Do not edit product code.
- Do not edit root files or sibling repos.
- Do not commit, push, tag, publish, open PRs, or mutate GitHub/Linear.
- Do not re-enable BES use of Mimir hooks, MCP servers, or raw memory as work
  authority.
- Do not resolve launch-readiness documentation drift in this lane unless the
  owner expands scope in a new approved spec.

## 4. Current System Facts

- Root `AGENTS.md` says the root checkout is the company control plane, product
  code lives in active child repos, non-trivial work starts with a spec, and
  public OSS repos including Mimir must not receive public agent-control churn
  without owner-approved low-noise PR planning.
- `.agents/OPERATING_MODEL.md` requires root preflight, spec-first execution,
  isolated workspaces/branches for parallel writers, explicit verification, and
  public OSS release hygiene.
- `.agents/GREEN_ROOM_EVALUATION.md` says remaining repo handoffs should run in
  parallel only where write scopes are disjoint, and green room evaluation for a
  repo may start only after that repo's handoff lane is closed or owner-paused.
- `.agents/MODEL_ROUTING.md` routes public OSS release/spec work through Codex
  `gpt-5.5` with Claude Opus 4.7 independent review when useful, and says
  write-capable agents need disjoint file ownership or worktree boundaries.
- Root preflight command `node .agents/scripts/preflight.mjs` passed with zero
  warnings on 2026-04-29.
- Mimir `AGENTS.md` says Mimir is public pre-1.0 active development, requires
  local verification before pushing, and each tracked branch/PR push triggers a
  costly GitHub Actions matrix run.
- Mimir `WORKFLOW.md` lists the canonical local verify command as
  `cargo build --workspace && cargo test --workspace && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings`.
- Mimir `STATUS.md` says workspace version is `0.1.0`, no release tag exists,
  and `v0.1.0` must wait for owner approval.
- `README.md` and `STATUS.md` both limit public claims: no production-ready
  claim, no stable storage/API/MCP schema claim, no hosted-service claim, no
  benchmark-proven superiority, and no direct agent writes to canonical memory.
- `docs/launch-readiness.md` records local cargo gates, `cargo deny check`,
  `cargo doc --workspace --no-deps`, crate dry-run expectations, recovery
  benchmark checks, public-surface sweeps, and first tag target `v0.1.0` after
  owner approval.
- `docs/README.md`, `README.md`, `STATUS.md`, `AGENTS.md`, and
  `docs/launch-readiness.md` still reference `docs/launch-posting-plan.md`.
  Command `test -e docs/launch-posting-plan.md` returned exit code 1, and
  recent `git log --oneline --decorate -n 12` shows
  `4d38614 Delete docs/launch-posting-plan.md (#16)`.
- Current Mimir branch command output:

```text
## main...origin/main
 M .gitignore
 M AGENTS.md
?? .agents/DOCUMENTATION_GUIDE.md
?? .agents/skills/code-review/SKILL.md
?? .agents/skills/implementation-execution/SKILL.md
?? .agents/skills/release-pr/SKILL.md
?? .agents/skills/repo-orientation/SKILL.md
?? .agents/skills/spec-driven-development/SKILL.md
?? .agents/skills/spec-evidence-governance/SKILL.md
?? .agents/skills/spec-review/SKILL.md
?? .agents/skills/symphony-dispatch/SKILL.md
?? .agents/skills/verification/SKILL.md
?? .agents/specs/2026-04-29-realignment-handoff/SPEC.md
?? .agents/specs/2026-04-29-repo-audit/SPEC.md
?? .agents/specs/SPEC.template.md
?? .agents/workflows/author-spec.md
?? .agents/workflows/execute-spec.md
?? .agents/workflows/orient.md
?? .agents/workflows/release-pr.md
?? .agents/workflows/review-diff.md
?? .agents/workflows/review-spec.md
?? .agents/workflows/spec-evidence.md
?? .agents/workflows/symphony-dispatch-check.md
?? .agents/workflows/verify-spec.md
?? .claude/commands/author-spec.md
?? .claude/commands/execute-spec.md
?? .claude/commands/orient.md
?? .claude/commands/release-pr.md
?? .claude/commands/review-diff.md
?? .claude/commands/review-spec.md
?? .claude/commands/spec-evidence.md
?? .claude/commands/symphony-dispatch-check.md
?? .claude/commands/verify-spec.md
?? .claude/settings.json
?? .claude/skills/code-review/SKILL.md
?? .claude/skills/implementation-execution/SKILL.md
?? .claude/skills/release-pr/SKILL.md
?? .claude/skills/repo-orientation/SKILL.md
?? .claude/skills/spec-driven-development/SKILL.md
?? .claude/skills/spec-evidence-governance/SKILL.md
?? .claude/skills/spec-review/SKILL.md
?? .claude/skills/symphony-dispatch/SKILL.md
?? .claude/skills/verification/SKILL.md
?? CLAUDE.md
?? WORKFLOW.md
```

- Current Mimir head is `9e81c0f` on `main`, tracking `origin/main`.
- `git diff --name-status` currently reports modified tracked files
  `.gitignore` and `AGENTS.md`.
- `git diff -- AGENTS.md` shows the local tracked change adds BES fleet
  operating model instructions to Mimir's operating manual.
- `git diff -- .gitignore` shows local tracked changes that stop ignoring
  `.claude/settings.json` and `.claude/skills/`, and add `.codex`,
  `.mcp.json`, and `.mcp.local.json` ignores.
- Local MCP posture remains zero-default: no Mimir `.mcp.json` is present.

## 5. Desired Behavior

The next Mimir worker can close or pause the handoff without disturbing other
parallel lanes. It must know which local files are pre-existing work, which work
requires owner decisions, which gates are required before public activity, and
when green room evaluation is allowed to start.

## 6. Domain Model / Contract

Closeout states:

- `preserve`: local or user work that must not be touched by this lane.
- `verify`: work that may be complete but needs fresh local gates.
- `owner-decision`: work that cannot proceed without HasNoBeef selecting a
  public or product posture.
- `ready-for-spec`: future work that needs its own approved spec before edits.
- `closed`: no unresolved in-flight work remains for the handoff lane.
- `owner-paused`: the owner explicitly allows green room evaluation to begin
  while named closeout work remains paused.

## 7. In-Flight Work

| Item | State | Required next action |
| --- | --- | --- |
| BES agent-control setup in `.agents/`, `.claude/`, `CLAUDE.md`, `WORKFLOW.md`, `.gitignore`, and `AGENTS.md` | preserve | Keep local/draft unless owner approves a low-noise public OSS PR plan. |
| Existing realignment handoff and repo-audit specs | preserve | Use as local source material; do not publish unless owner approves. |
| Active processing adapters at head `9e81c0f` | verify | Run full local cargo gate before any product release, tag, or PR closeout claim. |
| Missing `docs/launch-posting-plan.md` with remaining references | owner-decision | Decide whether to restore, replace, or remove stale references in a separate public-doc-safe spec. |
| Pre-1.0 launch cleanup and `v0.1.0` tag | owner-decision | Requires explicit owner approval, local green gates, and low-noise push/tag plan. |
| BES spec-authority integration research | ready-for-spec | Design only; do not re-enable hooks/MCP/memory authority without approved scope. |
| Green room product evaluation | blocked | May begin only after this handoff is `closed` or explicitly `owner-paused`. |

## 8. Public-OSS Safe Parallel Lane

The current lane may write only:

- `.agents/specs/2026-04-29-parallel-handoff-closeout/SPEC.md`

Rules for any continuation:

- Keep all output local.
- Do not push or publish.
- Do not edit product code, public docs, root files, or sibling repos.
- Do not stage files or commit.
- Do not normalize or delete untracked `.agents/` or `.claude/` work from other
  agents.
- If owner expands scope beyond this SPEC, create or switch to a dedicated
  branch/worktree and name the exact allowed files before editing.
- If another write-capable agent is active in Mimir, stop unless file ownership
  and worktree boundaries are explicit.

## 9. Required Verification Gates

For this local handoff spec:

```bash
node ../.agents/scripts/preflight.mjs
git status --short --branch --untracked-files=all
```

For any future product, public-doc, PR, push, tag, or release closeout:

```bash
cargo build --workspace
cargo test --workspace
cargo test --workspace --all-features
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo deny check
cargo doc --workspace --no-deps
```

Additional launch-readiness checks to run when release/public-doc scope is
approved:

```bash
cargo publish --dry-run -p mimir-core --allow-dirty
cargo test -p mimir-harness --test recovery_benchmark
python3 benchmarks/recovery/test_bench.py
rg -n 'production-ready|stable API|benchmark-proven|hosted service|direct agent writes|mimir hook-context|mimir-checkpoint' README.md STATUS.md docs crates plugins
rg -n 'launch-posting-plan.md' README.md STATUS.md AGENTS.md docs
```

Do not run GitHub Actions retries or remote CI probes unless the owner approves
the CI-budget tradeoff.

## 10. Owner Decisions

- Owner triage approval on 2026-04-30 marks Mimir `owner-paused` for local BES
  setup and public release actions. Green room evaluation may run local-only
  after at least one private repo validates the protocol.
- Public docs, PRs, pushes, tags, releases, CI-triggering work, and publication
  remain blocked until a separate owner-approved public OSS spec exists.
- Should Mimir close out launch cleanup first, or should spec-authority research
  be designed first?
- Should the BES integration pause remain root-only, or should Mimir public docs
  mention it in public-facing language?
- Should the local BES agent-control setup be committed to Mimir at all, and if
  yes, what is the low-noise public OSS PR plan?
- Should `docs/launch-posting-plan.md` be restored, replaced, or removed from
  remaining references after PR #16 deleted it?
- What release posture is acceptable after local gates pass: no tag, `v0.1.0`,
  or a new pre-release?
- Is green room evaluation allowed to start now as `owner-paused`, or only after
  the local handoff/setup state is closed?

## 11. Stop Conditions

Stop and report before editing if any of these occur:

- The requested file scope expands beyond this SPEC without an approved spec or
  explicit owner instruction.
- `git status` changes in files this lane did not touch and the change affects
  closeout facts.
- A command would push, publish, tag, re-enable Actions, mutate GitHub/Linear,
  install tools, or use network/secrets.
- Product docs or code need edits to resolve the `launch-posting-plan.md`
  references.
- Any source conflicts on whether Mimir hooks/MCP/raw memory are active BES work
  authority.
- Another write-capable Mimir worker is assigned overlapping files without a
  branch/worktree boundary.

## 12. Execution Plan

1. Preserve this SPEC as the current Mimir closeout handoff artifact.
2. Ask the owner to resolve the decisions in section 10.
3. If the owner chooses closeout, draft the next executable spec with exact
   files and public OSS posture.
4. Run the required local gates before any public-facing PR/push/tag/release.
5. Mark this handoff `closed` only after owner decisions are resolved or
   explicitly deferred and there is no unresolved closeout work blocking green
   room evaluation.
6. If the owner chooses not to close now, mark the unresolved items
   `owner-paused`; only then may green room evaluation begin.

## 13. Safety Invariants

- Mimir remains public OSS; internal BES agent-control output stays local until
  owner-approved public wording and CI-cost posture exist.
- The librarian remains the product write boundary; no agent writes trusted
  shared memory directly.
- BES fleet operation remains spec-first and zero-default-MCP until a future
  approved spec changes it.
- Existing local changes and untracked files are user/agent work and must be
  preserved.
- CI quota is protected by local verification and batched public activity.

## 14. Acceptance Criteria

- [x] Current branch, head, and dirty state are refreshed before closeout.
- [x] Owner decisions in section 10 are resolved or explicitly paused.
- [x] No product code, public docs, root files, or sibling repos are edited by
      this lane.
- [x] Required verification gates are run for any approved product/public-doc
      closeout.
- [x] Completion report lists commands, results, residual risk, and files
      changed.
- [x] Green room evaluation starts only after closeout is `closed` or
      `owner-paused`.

## 15. Rollback Plan

If this handoff spec is rejected, delete only:

```text
.agents/specs/2026-04-29-parallel-handoff-closeout/SPEC.md
```

Do not revert, delete, stage, or normalize any other local Mimir changes as part
of rollback.

## 16. Completion Report

- Files changed:
  - `.agents/specs/2026-04-29-parallel-handoff-closeout/SPEC.md`
- Commands run:
  - `node ../.agents/scripts/preflight.mjs` - passed with 0 warnings before
    this owner-pause decision.
  - `git status --short --branch --untracked-files=all` - captured branch
    `main...origin/main`, tracked `.gitignore` and `AGENTS.md` edits, and
    untracked local agent/Claude setup files.
- Verification result: local control-plane handoff is owner-paused. Product,
  public-doc, release, tag, and publication gates were intentionally not run
  because no public OSS action is approved.
- Anything intentionally left untouched: product code, public docs, launch
  references, root files, sibling repos, untracked `.agents/**`, `.claude/**`,
  `CLAUDE.md`, and `WORKFLOW.md`.
- Residual risk: Mimir remains public-OSS sensitive; all public actions remain
  blocked until a low-noise owner-approved public spec exists.
- Spec evidence candidates:
  - Public OSS green room packets can be local-only after owner pause, but
    publication and public-doc changes need a separate public-facing approval.
