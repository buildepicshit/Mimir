# Mimir Green Room Roadmap

## Current State

Mimir is a pre-1.0 public OSS memory governance system for AI coding agents.
The core architecture is implemented across 5 Rust crates (~47.5k LOC source,
~8.5k LOC tests). CI is green on main (2026-04-28). All launch readiness
checklist items are Done. The release pipeline exists and dry-run passed. No
release tag has been created. No crates.io publish has occurred. The repo is
`owner-paused` per BES fleet triage. Local dirty state consists of ~40
untracked agent-control/setup files and two modified files (`.gitignore`,
`AGENTS.md`); no tracked product source is modified.

The product is closer to public launch than any other BES repo. The gap is
owner decisions and a final cleanup pass, not engineering work.

## Milestones

### M1: Commit Scope Resolution

Decide and execute which local agent-control files to commit, gitignore, or
remove. This is the gate for all subsequent work — the dirty state must be
resolved before any clean batched commit/push.

**Owner decision required.**

### M2: Pre-Launch Cleanup

Fix the broken `docs/launch-posting-plan.md` reference. Verify all docs links.
Run a fresh full engineering gate (`cargo build/test/fmt/clippy/deny/doc` +
`cargo publish --dry-run` for all crates). Prepare the batched commit of
approved changes.

**Depends on: M1.**

### M3: v0.1.0 Release

Create the v0.1.0 tag. Trigger the release pipeline. Verify crates.io publish
and binary artifacts for all 5 targets (per RELEASING.md). Execute the launch
announcement.

**Depends on: M2 + owner approval for tag and publish.**

### M4: Post-Launch Maintainability

Decompose `mimir-librarian/src/main.rs` (8,455 LOC) and
`mimir-harness/src/lib.rs` (8,212 LOC) into focused modules. Set up code
coverage tooling. Establish a coverage baseline.

**Depends on: M3 (or can start after M1 if owner approves parallel work).**

### M5: BES Integration Research

Resume the paused BES spec-authority integration design. Requires a separate
approved research spec and second-model verification before implementation.

**Depends on: owner decision to resume. Independent of M1–M4.**

## Critical Path

```
M1 (commit scope) → M2 (cleanup) → M3 (release)
```

All three milestones are sequentially dependent. M1 is owner-blocked. The
total engineering work for M1–M3 is small — the blocking constraint is owner
decisions, not implementation effort.

## Parallelizable Work

The following can proceed in parallel with the critical path after M1 is
resolved:

| Work | Can start after | Blocks |
|---|---|---|
| Librarian main.rs decomposition | M1 | Nothing on critical path |
| Harness lib.rs decomposition | M1 | Nothing on critical path |
| Test coverage tooling setup | M1 | Nothing on critical path |
| Benchmark claim evidence collection | Any time | Nothing on critical path |
| Adapter inventory and planning | Any time (read-only) | Nothing on critical path |

## Work That Should Not Start Yet

| Work | Why not yet |
|---|---|
| BES spec-authority integration | Owner has not approved resumption; requires separate research spec |
| New feature development (relationship/timeline APIs, OCI package, hosted service) | Pre-1.0 — launch first |
| Public PR churn for agent-control scaffolding | CI budget constraint; owner must approve PR plan |
| OpenSSF Scorecard / Best Practices Badge | Deferred per launch-readiness.md; post-launch concern |
| Broader client recipes beyond Codex plugin | Post-launch; current Codex plugin and MCP server are sufficient for v0.1.0 |

## First Three Executable Specs

### Spec 1: Local Agent-Control Commit Plan

**Goal**: Resolve the dirty state by classifying all ~40 untracked files as
commit, gitignore, or remove.

**Scope**:
- Inventory every untracked file and its purpose.
- Propose a classification for owner review.
- After owner approval, stage only approved files and commit.
- Ensure no internal fleet language leaks into the public repo.

**Verification**: `git status` shows clean working tree with only approved
changes committed. No internal agent-control language in committed files
visible to public.

**Risk**: Medium — wrong classification could expose internal fleet language
publicly or lose useful local configuration.

**Model routing**: Any frontier model. Low-risk enough for Sonnet with
frontier verification.

---

### Spec 2: Pre-1.0 Launch Cleanup Batch

**Goal**: Fix all known documentation drift, verify engineering gates, and
prepare the final batched commit before tagging.

**Scope**:
- Remove or redirect the broken `docs/launch-posting-plan.md` reference in
  `docs/README.md`.
- Verify all documentation links resolve.
- Run the full engineering gate:
  `cargo build --workspace && cargo test --workspace && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings && cargo deny check && cargo doc --no-deps --all-features`.
- Run `cargo publish --dry-run` for each crate in dependency order.
- Fix any issues found.
- Prepare the batched commit.

**Verification**: Full cargo gate passes. No broken documentation links.
`cargo publish --dry-run` succeeds for all crates.

**Risk**: Low — this is cleanup, not feature work. The gate passed on
2026-04-28; regressions are unlikely unless dependency updates occurred.

**Model routing**: Any frontier model.

---

### Spec 3: v0.1.0 Release Tag and Publish

**Goal**: Execute the first public release of Mimir.

**Scope**:
- Push the batched commit from Spec 2.
- Create the `v0.1.0` tag per RELEASING.md.
- Verify the tag-triggered release pipeline:
  - verify-version
  - dry-run-publish
  - smoke-install
  - build-binaries (5 targets)
  - github-release
  - crates-publish (dependency order: mimir-core → mimir-librarian →
    mimir-harness → mimir-mcp → mimir-cli)
- Verify crates.io pages and binary artifact downloads.
- Execute launch announcement per the drafted launch article.

**Verification**: Release pipeline succeeds. All 5 crates published on
crates.io. Binary artifacts downloadable for all targets. `cargo install mimir-cli` works from a clean environment.

**Risk**: High — first public release; irreversible once crates.io publish
completes. Requires owner approval at multiple gates.

**Model routing**: Codex `gpt-5.5` primary with Claude Opus verification, per
MODEL_ROUTING.md public OSS release guidance.

**Owner approval gates**:
1. Approve the tag name and version.
2. Approve the crates.io publish.
3. Approve the launch announcement wording.
4. Approve the CI budget spend for the release pipeline.
