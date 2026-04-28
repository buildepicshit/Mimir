# Public Readiness Plan - 2026-04-27

Mimir can be public as an experimental, pre-1.0 project in active development. The public stance should be explicit: Mimir started as an experiment to reduce cross-agent memory-management friction, context contamination, and memory loss during restarts or system failures. The architecture, local implementation, and transparent harness are real, but benchmark claims and stable release claims wait for recorded evidence.

## Public stance

Safe to say now:

- Mimir is an experimental multi-agent memory governance/control plane.
- The motivating problem is practical agent work: keeping memory scoped, preventing contamination, and preserving useful context through failures.
- Canonical memory writes are librarian-mediated; agents do not write trusted shared memory directly.
- The local append-only store, canonical log replay, MCP surface, librarian draft path, transparent harness, read-only Copilot session-store adapter, and recovery-benchmark validation scaffolding are implemented and locally tested.
- The project is looking for review and contributors, especially around Rust correctness, security boundaries, benchmark methodology, adapter UX, and docs clarity.

Do not claim yet:

- Production readiness.
- Stable storage, API, CLI, or wire-format compatibility.
- Benchmark-proven recovery improvement.
- Hosted service availability.
- Published crates or binary releases.
- Cross-agent memory reuse without governed promotion.

## Done enough for early public viewing

- Apache-2.0 license, code of conduct, contribution guide, security policy, citation metadata, changelog, release runbook, issue templates, PR template, CODEOWNERS, Dependabot, and CI/release workflows exist.
- README now states the pre-1.0 active-development status and calls out live-pilot boundaries.
- `STATUS.md` records the implementation snapshot and public-readiness caveat.
- `AGENTS.md` captures the non-negotiable architecture and contribution rules.
- `docs/concepts/` holds the authoritative implementation specs plus draft scope/quorum specs.
- Sanitisation, observability, and BC/DR restore docs exist for the highest-risk public questions.

## P0 before wider announcement

1. **Live harness pilots.** Run real Claude/Codex sessions through `mimir <agent> [agent args...]` only after the operator-populated baseline environments validate and the explicit scenario approval token is supplied. Capture transcripts, validate transcript evidence, score results, and publish the result as a bounded benchmark note.
2. **Repository settings pass (#64).** Set a concise GitHub description, add topics, confirm public visibility, confirm security-policy links, and keep Actions usage deliberate now that the owner added usage and Actions were re-enabled on 2026-04-27.
3. **Seed public issues.** Create focused issues for contributor lanes instead of leaving newcomers to reverse-engineer the roadmap from `STATUS.md`.
4. **Crate docs audit.** Run `cargo doc --workspace --no-deps` and fix broken or misleading public API docs before publishing crates.
5. **First-run harness walkthrough.** Record a clean fresh-clone setup path, including config init, native agent setup status, and a wrapped no-op session.

2026-04-28 local verification: repository metadata was prepared without changing visibility, public issues already cover the contributor lanes, `cargo doc --workspace --no-deps` passed, locked path installs for all four public binaries succeeded from temporary roots, and the first-run no-op walkthrough completed in an isolated `/tmp` project. The visibility switch and live harness benchmark evidence remain the owner-gated public-announcement blockers.

## P1 before first prerelease

1. Promote a `CHANGELOG.md` section for `0.1.0-alpha.1`.
2. Dry-run the release workflow locally where possible and through manual dispatch only when Actions budget is available and the owner agrees the run is worth the minutes.
3. Smoke install `mimir-cli`, `mimir-librarian`, `mimir-mcp`, and `mimir-harness` from clean paths.
4. Publish an alpha release only after the live harness pilot report exists.
5. Confirm every crate README says the same pre-1.0 compatibility story as the root README.

## P2 before stable v1

1. Define semver compatibility promises for canonical records, draft JSON, config, CLI flags, and MCP tool schemas.
2. Decide whether the service remote adapter is in or out of the v1 surface.
3. Run a dependency/license audit and document accepted risk for any pinned fast-moving agent SDKs.
4. Complete a threat-model review of prompt/data boundaries, subprocess launch, native setup artifact writes, recovery remotes, and governed promotion.
5. Produce a public benchmark report with reproducible fixtures, transcript evidence, scoring code, and caveats.

## Contributor lanes

- **Rust correctness:** log replay, append-only invariants, crash recovery, write-lock behavior, canonical decoder strictness.
- **Security review:** prompt/data boundaries, native agent setup, subprocess handling, recovery remotes, draft quarantine, scoped promotion.
- **Benchmark methodology:** scenario representativeness, transcript evidence, stale-context scoring, hallucination denominator, aggregate verdict rules.
- **Adapter UX:** Claude/Codex launch behavior, Copilot session-store recall, setup diagnostics, config init, native memory sweep, checkpoint ergonomics.
- **Docs and onboarding:** reducing internal-history density, improving first-run guides, aligning crate READMEs with the root public stance.

See [`2026-04-28-memory-product-gap-scan.md`](2026-04-28-memory-product-gap-scan.md) for the external memory-product feature scan. See [`2026-04-28-publication-readiness-and-channels.md`](2026-04-28-publication-readiness-and-channels.md) for the repository/documentation bar and recommended publication channels. The launch-relevant follow-up lanes are operator memory controls, one-command client onboarding, richer context sections, relationship/timeline recall, connector ingestion, and benchmark evidence.

## Verification expectations

Before pushing public-readiness changes, run the local gate from `AGENTS.md`:

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
```

When GitHub Actions is enabled, push once after the local gate and watch the resulting checks. When Actions is disabled for quota, PRs should include fresh local gate output. In both states, do not use empty retry commits or speculative reruns.
