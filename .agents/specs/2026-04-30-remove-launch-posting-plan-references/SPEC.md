---
id: remove-launch-posting-plan-references
status: ready-for-review
owner: HasNoBeef
repo: Mimir
branch_policy: worktree-preferred
risk: low
requires_network: false
requires_secrets: []
acceptance_commands:
  - test ! -e docs/launch-posting-plan.md
  - '! rg -n "docs/launch-posting-plan\\.md|\\(launch-posting-plan\\.md\\)|launch-posting-plan\\.md" AGENTS.md STATUS.md README.md docs/README.md docs/launch-readiness.md'
  - "bash -lc 'rg -n \"launch-posting-plan|launch posting|posting plan|Publishing plan\" AGENTS.md STATUS.md README.md docs/README.md docs/launch-readiness.md CHANGELOG.md || test $? -eq 1'"
  - git diff --check -- AGENTS.md STATUS.md README.md docs/README.md docs/launch-readiness.md CHANGELOG.md
---

# SPEC: Remove Launch Posting Plan References

## 1. Problem

`docs/launch-posting-plan.md` is absent from the public Mimir tree, but current
repo documentation still links to it or describes it as an active launch
posting plan. Because Mimir is public OSS, a broken launch/publishing reference
creates public documentation drift and can imply a release or announcement path
that is not owner-approved.

This spec is local agent-control work only. It authorizes a future
implementation to remove or redirect stale references to the missing file; it
does not authorize restoring a launch posting plan, committing local
agent-control files, pushing, opening a PR, tagging, publishing crates, spending
CI minutes, or announcing a release.

## 2. Goals

- Remove or redirect every current reference that points readers to the missing
  `docs/launch-posting-plan.md` file.
- Keep public wording factual, pre-1.0, and consistent with current approved
  release boundaries.
- Preserve the owner decision that launch/posting/release execution remains
  separate release-pr work.
- Keep `.agents/` and other BES agent-control artifacts local-only unless a
  separate owner-approved public rollout spec says otherwise.

## 3. Non-Goals

- Do not restore, recreate, rewrite, or replace `docs/launch-posting-plan.md`.
- Do not create a new launch-posting, marketing, social, Show HN, crates.io,
  docs.rs, MCP Registry, or announcement plan.
- Do not tag `v0.1.0`, publish crates, open PRs, push branches, mutate tracker
  state, or trigger release workflows.
- Do not resolve broader dirty-state questions for `.agents/`, `.claude/`,
  `CLAUDE.md`, `WORKFLOW.md`, `.gitignore`, or existing modified `AGENTS.md`.
- Do not edit product code, Cargo metadata, CI workflows, release automation,
  benchmark assets, or unrelated docs.

## 4. Current System Facts

- Owner instruction for this task: remove or redirect stale
  launch-posting-plan references; do not restore a launch-posting plan;
  agent-control files stay local-only; public release/publish decisions remain
  separate release-pr work.
- Root `AGENTS.md`: public OSS repos `Wick` and `Mimir` must not receive
  internal agent-control language unless the owner approves a public-facing
  rollout; product code lives in child repos, not the root.
- Root `.agents/OPERATING_MODEL.md`: non-trivial work starts with an
  executable spec, public OSS repos require extra release hygiene, and public
  OSS doc-only churn must not be pushed without an intentional owner-approved
  low-noise PR plan.
- Root `.agents/GREEN_ROOM_EVALUATION.md`: green-room packets are local,
  isolated, and do not authorize implementation, public docs publication, PRs,
  tags, or releases.
- Root
  `.agents/specs/2026-04-29-green-room-product-evaluations/CROSS_PRODUCT_SEQUENCE.md`:
  Mimir is actionable as local-only public-OSS-safe cleanup to remove or
  redirect stale launch-posting-plan references; agent-control files remain
  local-only; public release/publish decisions remain separate release-pr work.
- Mimir `AGENTS.md`: the repo is public OSS, uses BES spec-first operation, and
  has a hard CI quota rule requiring local verification before any push.
- Mimir `WORKFLOW.md`: canonical verification is
  `cargo build --workspace && cargo test --workspace && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings`.
- Mimir `STATUS.md`: Mimir is pre-1.0 public active development; release tags
  are absent; `v0.1.0` may be tagged only after owner approval.
- Mimir `README.md`: public claims are intentionally limited and do not claim
  production readiness, stable APIs, hosted service availability, benchmark
  superiority, direct agent writes, or ungoverned cross-project promotion.
- Mimir `docs/launch-readiness.md`: release/tag/publish state remains
  pre-release; docs.rs and crates.io publishing wait for release workflow.
- Mimir green-room verifier
  `.agents/specs/2026-04-30-green-room-product-evaluation/VERIFICATION.md`:
  `docs/launch-posting-plan.md` is missing, and stale references were found in
  `AGENTS.md`, `STATUS.md`, `README.md`, `docs/README.md`, and
  `docs/launch-readiness.md`.
- Command: `git -C Mimir log --oneline --decorate -n 5` from the workspace
  root showed `4d38614 Delete docs/launch-posting-plan.md (#16)` immediately
  before current `HEAD` `9e81c0f`, so the missing file was deliberately deleted
  in repository history.
- Command:
  `test -e docs/launch-posting-plan.md; printf 'docs/launch-posting-plan.md exists: %s\n' "$?"`
  from `Mimir` returned `docs/launch-posting-plan.md exists: 1`, confirming the
  file does not exist in the current worktree.
- Command:
  `rg -n "launch-posting-plan|posting plan|launch posting|Publishing plan" .`
  from `Mimir` found current references in:
  - `AGENTS.md`
  - `STATUS.md`
  - `README.md`
  - `docs/README.md`
  - `docs/launch-readiness.md`
  - `CHANGELOG.md`
- The same search shows `CHANGELOG.md` mentions launch posting assets in the
  Unreleased historical change summary but does not link to
  `docs/launch-posting-plan.md`.
- Command: `git -C Mimir status --short --branch --untracked-files=all` showed
  branch `main...origin/main`, existing modified `.gitignore` and `AGENTS.md`,
  and many untracked `.agents/`, `.claude/`, `CLAUDE.md`, and `WORKFLOW.md`
  files. Those existing changes predate this spec and must be preserved.

## 5. Desired Behavior

After implementation:

- No current public or operating document points to the missing
  `docs/launch-posting-plan.md` path.
- `docs/launch-posting-plan.md` remains absent.
- The public docs continue to point users to existing launch/release status
  surfaces, primarily `STATUS.md`, `docs/README.md`,
  `docs/launch-readiness.md`, `RELEASING.md`, and the existing launch article
  only where those links already exist or are directly relevant.
- Any replacement wording is descriptive and non-promissory. Public-facing
  replacements may say that release, tag, publish, listing, and announcement
  steps remain pending or are handled separately by the normal release process.
  They must not introduce internal terms such as `release-pr`, `agent-control`,
  or `BES fleet`, and must not provide new channel strategy, marketing copy,
  publish order, launch timing, ownership/approval language, or
  public-readiness claims.
- If an implementer believes a user-facing sentence needs subjective marketing
  or launch-positioning judgment, the implementer must stop and mark that
  sentence owner-blocking instead of inventing wording.

## 6. Domain Model / Contract

- `docs/launch-posting-plan.md`: deleted public doc. It is not an implementation
  target and must not be restored by this cleanup.
- Stale reference: any link, path mention, index row, status row, or active
  current-state claim that directs a reader to `docs/launch-posting-plan.md` or
  says the missing plan is the active launch/posting/listing plan.
- Redirect: replacing a stale reference with a link to an existing current
  document that already carries the relevant authority, without adding new
  launch strategy. Acceptable redirect targets are:
  - `STATUS.md` for current state and release tags.
  - `docs/launch-readiness.md` for OSS readiness and promise audit.
  - `RELEASING.md` for release mechanics, if the surrounding context is release
    workflow rather than launch announcement.
  - `docs/blog/2026-04-28-agent-memory-compiler-pipeline.md` only for the
    existing public article reference, not as a posting plan replacement.
- Removal: deleting the stale bullet, table entry, or sentence when no existing
  public doc is an objective replacement.
- Historical changelog entry: a past-tense release-note statement that does not
  link to the missing file. It may remain unchanged unless implementation
  evidence shows it is now misleading as a current-state claim.

## 7. Interfaces And Files

Expected implementation touch points:

- `AGENTS.md`: remove or redirect the `Where to Look` row that currently
  includes `docs/launch-posting-plan.md`.
- `STATUS.md`: remove or redirect the `References` bullet for
  `docs/launch-posting-plan.md`.
- `README.md`: remove or redirect the `Documentation` bullet for
  `docs/launch-posting-plan.md`.
- `docs/README.md`: remove or redirect the `Start Here` bullet and the
  `Launch Execution` paragraph that reference `launch-posting-plan.md`.
- `docs/launch-readiness.md`: update the `Publishing plan` row so it does not
  claim the missing file is done or authoritative.

Files to inspect but not edit unless implementation evidence proves a current
stale reference:

- `CHANGELOG.md`: current evidence shows only a historical Unreleased summary
  mentioning posting assets, with no missing-file link.

Files and directories out of scope:

- `docs/launch-posting-plan.md`
- Product source files and tests.
- `Cargo.toml`, `Cargo.lock`, `.github/`, `RELEASING.md`, release workflows,
  benchmark assets, and package metadata.
- `.agents/` files other than this spec, `.claude/`, `CLAUDE.md`,
  `WORKFLOW.md`, `.gitignore`, git metadata, and tracker state.

Public interfaces affected:

- Public README/documentation navigation only.
- No CLI, API, storage, MCP, package, or release interface changes.

## 8. Execution Plan

1. Reconfirm the worktree state with
   `git status --short --branch --untracked-files=all` and preserve all
   pre-existing modified/untracked files.
2. Reconfirm the missing-file reference set with
   `rg -n "launch-posting-plan|launch posting|posting plan|Publishing plan" AGENTS.md STATUS.md README.md docs/README.md docs/launch-readiness.md CHANGELOG.md`.
3. Edit only the expected implementation touch points listed in Section 7.
4. For each stale reference, either remove it or redirect it to an existing
   current authority document using neutral wording.
5. Do not edit `CHANGELOG.md` unless the implementation search proves it
   contains a current missing-file path or active-current-state claim rather
   than historical release-note language.
6. Do not create `docs/launch-posting-plan.md`.
7. Run the acceptance commands in Section 10.
8. Report changed files, command results, unchanged pre-existing dirty files,
   and any owner-blocking wording decisions encountered.

## 9. Safety Invariants

- Do not overwrite or revert pre-existing modifications in `AGENTS.md`,
  `.gitignore`, `.agents/`, `.claude/`, `CLAUDE.md`, `WORKFLOW.md`, or any other
  file.
- Do not stage, commit, push, open a PR, tag, publish, mutate tracker state, or
  run public release workflows.
- Do not restore the deleted launch-posting plan.
- Do not introduce internal BES fleet details into public-facing docs beyond
  already-existing local agent-control surfaces.
- Do not add AI attribution to docs, commits, release notes, or generated
  output.
- Do not broaden this cleanup into broader launch-readiness, release-pr,
  package, CI, benchmark, or public-announcement work.
- If implementation needs subjective launch messaging, owner review is required
  before the wording is written.

## 10. Test Plan

Run from `/var/home/hasnobeef/buildepicshit/Mimir` after implementation:

```bash
git status --short --branch --untracked-files=all
test ! -e docs/launch-posting-plan.md
! rg -n "docs/launch-posting-plan\\.md|\\(launch-posting-plan\\.md\\)|launch-posting-plan\\.md" AGENTS.md STATUS.md README.md docs/README.md docs/launch-readiness.md
bash -lc 'rg -n "launch-posting-plan|launch posting|posting plan|Publishing plan" AGENTS.md STATUS.md README.md docs/README.md docs/launch-readiness.md CHANGELOG.md || test $? -eq 1'
git diff --check -- AGENTS.md STATUS.md README.md docs/README.md docs/launch-readiness.md CHANGELOG.md
```

Expected results:

- `git status` shows only the implementation files plus pre-existing dirty and
  untracked files; no unrelated files appear.
- `test ! -e docs/launch-posting-plan.md` passes.
- The negative `rg` command returns no matches.
- The broad `rg` evidence command returns either no matches in current docs or
  only explicitly reviewed non-stale historical language such as `CHANGELOG.md`.
  Exit code 1 from no matches is acceptable; any other `rg` failure is not.
- `git diff --check` passes.

Do not run cargo gates for this link-only cleanup unless implementation touches
Rust, Cargo, CI, release, package, benchmark, or generated documentation
surfaces. The full local gate remains mandatory before any future push or
release-pr work.

Manual checks:

- Open each changed diff hunk and confirm it either removes the stale reference
  or redirects to an existing file.
- Confirm no replacement sentence invents a public release date, launch
  channel strategy, posting copy, publish approval, or benchmark/performance
  claim.
- Confirm `docs/launch-readiness.md` no longer marks the missing publishing
  plan as `Done` evidence.

## 11. Acceptance Criteria

- [ ] `docs/launch-posting-plan.md` remains absent.
- [ ] No active current-state doc among `AGENTS.md`, `STATUS.md`, `README.md`,
      `docs/README.md`, or `docs/launch-readiness.md` references
      `docs/launch-posting-plan.md` or `launch-posting-plan.md`.
- [ ] Any remaining "posting plan", "launch posting", or "Publishing plan"
      wording is either removed, redirected to an existing authority document,
      or explicitly historical and non-actionable.
- [ ] `docs/launch-readiness.md` does not claim a missing publishing plan is
      complete evidence.
- [ ] No new launch/posting plan, marketing strategy, release approval, tag,
      publish, PR, or CI-spend action is introduced.
- [ ] Only files required by Section 7 are edited, except `CHANGELOG.md` may be
      edited only if implementation evidence proves it contains a stale current
      reference.
- [ ] Pre-existing dirty/untracked work is preserved.
- [ ] Acceptance commands pass or any failure is classified as expected,
      new, or owner-blocking with exact output.
- [ ] Completion report lists files changed, commands run and results,
      intentionally untouched files, residual risks, and any spec evidence
      candidates.

## 12. Rollback Plan

Before commit or PR work, rollback is a normal file-level revert of only the
future implementation hunks in the touched public docs. Do not use
`git reset --hard` or broad checkout commands because the worktree already
contains unrelated owner/agent changes that must be preserved.

If a later review rejects a redirect target, replace only that sentence or
bullet with a removal or owner-approved redirect. Do not restore
`docs/launch-posting-plan.md` as rollback.

## 13. Open Questions

- [ ] None before spec review. Owner has already decided to remove or redirect
      stale launch-posting-plan references and not restore the plan.
- [ ] Owner-blocking during implementation: any proposed replacement wording
      that requires subjective launch messaging, public positioning, channel
      strategy, publish timing, or public-readiness judgment.

## 14. Completion Report

To be filled by the executor/verifier:

- Files changed:
- Commands run:
- Verification result:
- Intentionally untouched:
- Residual risk:
- Spec evidence candidates:
