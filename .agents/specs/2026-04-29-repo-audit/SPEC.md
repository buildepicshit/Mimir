---
id: mimir-repo-audit-2026-04-29
status: draft
owner: HasNoBeef
repo: Mimir
branch_policy: local-only-public-oss
risk: medium
requires_network: false
requires_secrets: []
acceptance_commands:
  - cargo build --workspace
  - cargo test --workspace
  - cargo test --workspace --all-features
  - cargo fmt --all -- --check
  - cargo clippy --all-targets --all-features -- -D warnings
  - cargo deny check
  - cargo doc --workspace --no-deps
  - rg -n 'production-ready|stable API|benchmark-proven|hosted service|direct agent writes|mimir hook-context|mimir-checkpoint' README.md STATUS.md docs crates plugins
---

# SPEC: Mimir Repo Audit And Spec Migration

## 1. Problem

Mimir is a public pre-1.0 memory governance product. BES is temporarily
removing Mimir hook/setup surfaces from the active agent operating layer while
the company moves to spec-first Symphony dispatch. The repo needs to preserve
its product roadmap while making clear that BES agents do not currently rely on
Mimir hooks or raw memory as source-of-truth.

## 2. Current Facts

- `STATUS.md` says Mimir is in pre-1.0 public launch cleanup, version `0.1.0`,
  with no release tag yet.
- `README.md` says agents may propose memory, but do not write trusted shared
  memory directly.
- `docs/launch-readiness.md` records OSS readiness, engineering quality gates,
  promise-audit boundaries, and deferred work.
- Product surfaces include core append-only store, librarian, harness, operator
  tools, Claude/Codex setup paths, MCP, recovery mirroring, and benchmarks.
- Public claims are explicitly limited: no production-ready claim, no stable
  API/storage claim, no hosted-service claim, no benchmark-proven claim.
- Code inventory from `rg --files`: 63 Rust files, 1 Python file, 10 TOML
  files, and 54 Markdown files.
- Active BES agent operating surfaces now use `spec-evidence-governance`; the
  repo product may still legitimately mention Mimir hook and checkpoint
  features in code/docs/tests.

## 3. Preserve

- Product mission: local-first governed memory, append-only canonical store,
  librarian-mediated writes, transparent harness, and explicit recovery.
- Launch-readiness checklist and promise-audit discipline.
- Public pre-1.0 honesty.
- Product docs/tests for `mimir hook-context`, `mimir-checkpoint`, and native
  setup paths, because those are Mimir product features.

## 4. Archive Or Supersede

- Do not archive Mimir's product memory-governance docs. They remain product
  architecture.
- Do supersede any BES operating instruction that tells agents to use Mimir
  hooks or raw memory as work authority.
- Future BES integration should be designed as a spec/delivery evidence system
  before re-enabling hooks.

## 5. Proposed New Executable Specs

1. **BES Integration Pause Note**
   - Scope: add a small product/docs note, if owner approves, clarifying that
     BES company agents currently use spec evidence instead of Mimir hooks.
   - Acceptance: no claim that Mimir product functionality is deprecated.

2. **Pre-1.0 Launch Cleanup Batch**
   - Scope: continue launch-readiness gates, public-surface scrub, crate/docs
     dry-runs, and owner-approved release tagging.
   - Acceptance: all `docs/launch-readiness.md` local gates pass.

3. **Spec Authority Research Design**
   - Scope: design how Mimir could later store and govern spec evidence,
     delivery records, and supersession decisions instead of generic memories.
   - Acceptance: design spec only; no hook re-enable until approved.

4. **Benchmark Claim Evidence**
   - Scope: live recovery benchmark report with transcripts and scorecards.
   - Acceptance: benchmark claim is either supported by evidence or kept out of
     public copy.

5. **OSS Release Rollout**
   - Scope: batched public changes, crates.io order, docs.rs expectations,
     launch article/posting plan, and CI-cost-aware push.
   - Acceptance: no remote push/tag until owner approves.

## 6. Open Questions

- Should Mimir's first post-audit work be launch cleanup or spec-authority
  research?
- Do you want a visible docs note about BES pausing Mimir hooks, or should that
  stay in the company control-plane docs only?
- When public launch resumes, should the release be `v0.1.0` exactly or a new
  pre-release tag after the current local audit batch?

## 7. Verification Status

This audit read docs and performed a lightweight code inventory only. Cargo
gates were not run because no Mimir product code changed in this session and
public OSS CI churn is intentionally avoided.

The one-time migration bootstrap for Mimir has been actioned locally and should
be deleted in this local audit batch.
