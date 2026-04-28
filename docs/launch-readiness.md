# Launch Readiness - 2026-04-28

This checklist is the current public-launch sign-off surface for Mimir. It covers OSS readiness, engineering quality, product-promise accuracy, release state, and deferred work.

## External Bar Used

- GitHub community profile expectations: <https://docs.github.com/en/communities/setting-up-your-project-for-healthy-contributions/about-community-profiles-for-public-repositories>
- GitHub issue and pull request template expectations: <https://docs.github.com/en/communities/using-templates-to-encourage-useful-issues-and-pull-requests/about-issue-and-pull-request-templates>
- GitHub security policy guidance: <https://docs.github.com/en/code-security/how-tos/report-and-fix-vulnerabilities/configure-vulnerability-reporting/adding-a-security-policy-to-your-repository>
- OpenSSF Scorecard: <https://github.com/ossf/scorecard>
- OpenSSF Best Practices Badge: <https://openssf.org/best-practices-badge/>
- Cargo publishing guidance: <https://doc.rust-lang.org/cargo/reference/publishing.html>
- Rust API Guidelines checklist: <https://rust-lang.github.io/api-guidelines/checklist.html>
- docs.rs build behavior: <https://docs.rs/about/builds>

## OSS Readiness

| Item | Status | Evidence |
|---|---|---|
| README | Done | `README.md` states pre-1.0 status, implemented surfaces, quickstart, and non-claims. |
| License | Done | `LICENSE` is Apache-2.0. |
| Contributing guide | Done | `CONTRIBUTING.md`. |
| Code of conduct | Done | `CODE_OF_CONDUCT.md`. |
| Security policy | Done | `SECURITY.md`, linked from README. |
| Issue templates | Done | `.github/ISSUE_TEMPLATE/`. |
| PR template | Done | `.github/pull_request_template.md`. |
| CODEOWNERS | Done | `.github/CODEOWNERS`. |
| Changelog | Done | `CHANGELOG.md`, with public-surface cleanup recorded under Unreleased. |
| Release runbook | Done | `RELEASING.md`. |
| Citation metadata | Done | `CITATION.cff`. |
| Dependabot | Done | `.github/dependabot.yml`, monthly cadence. |
| CI | Done on main | GitHub Actions main run was green on 2026-04-28 after PR #11. |
| Docs index | Done | `docs/README.md`. |
| Public artifact hygiene | Done | Scratch research fixtures removed from tracked files; recovery benchmark promoted to `benchmarks/recovery`; stale internal-path sweep returned no hits. |

## Engineering Quality Gate

Passed locally on 2026-04-28:

```bash
cargo build --workspace
cargo test --workspace
cargo test --workspace --all-features
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
```

Additional launch checks:

```bash
cargo deny check
cargo doc --workspace --no-deps
cargo package -p mimir-core --allow-dirty
cargo test -p mimir-harness --test recovery_benchmark
python3 benchmarks/recovery/test_bench.py
```

Dependent crates cannot complete `cargo package` verification before `mimir-core` exists in the crates.io index. The release workflow handles that by dry-running `mimir-core` first, then dry-running dependent crates immediately before their real publish after index propagation.

Passed locally on 2026-04-28:

- `cargo deny check`
- `cargo doc --workspace --no-deps`
- `cargo publish --dry-run -p mimir-core --allow-dirty`
- `cargo test --workspace --all-features`
- `cargo test -p mimir-harness --test recovery_benchmark`
- `python3 benchmarks/recovery/test_bench.py`
- tracked scratch-directory check returned no files
- stale internal-path text sweep returned no hits

Public-surface checks: confirm no tracked files remain under removed scratch directories, then run a text sweep for stale internal path markers across README, STATUS, docs, crates, `.github`, and Cargo metadata before pushing.

## Promise Audit

| Public claim | Evidence | Launch wording |
|---|---|---|
| Mimir is local-first memory governance for agents. | Harness, librarian, MCP, and core store are in the workspace. | Allowed. |
| Agents do not write trusted shared memory directly. | Checkpoint/native memories become draft envelopes; canonical commits flow through librarian/store code. | Allowed. |
| Append-only canonical storage exists. | `mimir-core` log/store and decoder verification are implemented and tested. | Allowed. |
| Transparent harness exists. | `mimir-harness` wraps native agent commands and records capture summaries. | Allowed, local/pre-1.0 wording only. |
| Recovery mirroring exists. | `mimir remote status|push|pull|drill` exists. | Allowed as local Git-backed BC/DR, not hosted sync. |
| Mimir improves recovery outcomes. | Benchmark harness exists; live transcript-backed results are not published yet. | Not allowed yet. |
| Stable public API/storage format. | Pre-1.0 workspace version and no release tag. | Not allowed yet. |
| Hosted service or daemonized librarian. | Service remote is adapter-boundary only. | Not allowed yet. |

## Version And Release State

- Workspace version: `0.1.0`.
- Release tags: none.
- First public tag target: `v0.1.0`, after cleanup branch is locally green and owner approves tagging.
- Crates.io publishing waits for the `mimir-core` package dry-run, crate README audit, and release ordering through `mimir-core` first.
- docs.rs will build after crates are published to crates.io.

## Deferred After Public Opening

- OpenSSF Scorecard run and remediation.
- OpenSSF Best Practices Badge self-certification.
- Live recovery benchmark report with transcripts and scorecards.
- Broader client setup recipes beyond Claude/Codex/Copilot.
- Relationship/timeline recall APIs.
- OCI or MCPB package for official MCP Registry submission.
- Hosted service or service remote transport.
