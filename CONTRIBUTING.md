# Contributing to Mimir

Mimir is a BES Studios research project in active pre-1.0 development. Public review is welcome, especially while the architecture is still early enough to change in response to good evidence.

This file documents the contribution workflow for human contributors and agents (Claude Code, Codex, Cursor, Gemini CLI) working in-repo under `AGENTS.md`.

## Read first

- [`AGENTS.md`](AGENTS.md) — authoritative operating manual (architectural invariants, engineering standards, engagement protocol, anti-patterns, commit conventions).
- [`STATUS.md`](STATUS.md) — current phase and next milestone.
- [`PRINCIPLES.md`](PRINCIPLES.md) — engineering principles and tooling policy (twelve sections covering testing, error handling, type safety, determinism, observability, performance targets, code style, dependency policy, documentation, semver, deprecation, release).
- [`docs/planning/2026-04-19-roadmap-to-prime-time.md`](docs/planning/2026-04-19-roadmap-to-prime-time.md) — phased roadmap from feature-complete pre-1.0 to publicly listed v1.0.

## Development setup

Mimir's librarian is written in Rust.

Requirements:

- [Rust](https://rustup.rs) toolchain — pinned to `1.89.0` via [`rust-toolchain.toml`](rust-toolchain.toml). `rustup` will install the right version automatically on first build. (Bump history in the `rust-toolchain.toml` rationale block; pre-1.0 alpha tracks ecosystem MSRV.)
- `cargo` ships with `rustup`.

```bash
git clone https://github.com/buildepicshit/Mimir.git
cd Mimir
cargo build --workspace --all-features
cargo test --workspace --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

CI is configured to run all four commands on PRs per [`.github/workflows/ci.yml`](.github/workflows/ci.yml). While the project is on constrained GitHub Actions quota, maintainers may keep Actions disabled and require fresh local gate output instead. Either way, all four checks must pass before merge.

## Where help is useful now

- Rust correctness and API review for the append-only store, log replay, workspace write lock, and crash-recovery behavior.
- Security and threat-model review around prompt/data boundaries, native agent setup, subprocess execution, and Git-backed recovery remotes.
- Benchmark methodology review for recovery scenarios, transcript evidence gates, score validation, and live wrapped-agent pilots.
- Adapter and harness UX feedback from people who regularly run Claude, Codex, MCP clients, or other local agent tools.
- Documentation and onboarding cleanup where design history is too dense for a first-time reader.

### Linux (Fedora / RHEL-family) linker note

The default GNU `ld.bfd` linker segfaults on Mimir's release-profile link target on Fedora 43 (reproduced 2026-04-20). Use [`mold`](https://github.com/rui314/mold) instead:

```bash
sudo dnf install mold
```

Then either wrap single builds (`mold -run cargo build --release`) or make it default by adding the following to `~/.cargo/config.toml` (user-wide) or `.cargo/config.toml` at the repo root (project-local, not tracked):

```toml
[target.x86_64-unknown-linux-gnu]
rustflags = ["-C", "link-arg=-fuse-ld=mold"]
```

`mold` is also faster on debug/incremental links, so adopting it user-wide is a net ergonomic win. Ubuntu, Debian, Arch, and macOS users do not hit this; their default linkers handle Mimir's link graph without issue.

## Commit conventions

Follow [Conventional Commits](https://www.conventionalcommits.org/) with Mimir's type list (see `AGENTS.md` § Commit Conventions):

`feat`, `fix`, `refactor`, `test`, `docs`, `chore`, `research`, `spec`, `perf`, `build`, `ci`.

- Atomic, reviewable commits.
- Commit bodies stay under 15 lines. Depth belongs in the spec or PR description.
- **No AI attribution.** No `Co-Authored-By` lines for AI tools, no generation footers, no tool-attribution emojis.
- Never skip hooks (`--no-verify`) or bypass signing without explicit owner approval.

## PR workflow

- Branch off `main`. One branch per phase or one branch per logical change.
- Open a PR when the branch is ready for review.
- Squash merge only. Linear history on `main`.
- Never force-push to `main`.
- PRs that create new architectural artifacts must cite primary sources or flag them `pending verification` per `AGENTS.md` § Engineering Standards.

## Engagement protocol (agents working in-repo)

This section is operational guidance for agents working directly in the maintainer workflow. It is not a barrier to public issues, questions, or normal human-authored PRs.

Per `AGENTS.md` § Engagement Protocol:

1. **Propose** in 2–3 sentences. What and why.
2. **Wait** for yes / change / no.
3. **Execute** the single concrete action. No scope expansion.
4. **Report** what shipped plus the logical next step. Do not auto-roll.
5. **Stop.** Owner directs the next step.

Agents that deviate from this protocol are out of spec for Mimir work.

## Primary sources

Design claims cite primary literature (papers, specifications, canonical repositories). Training-memory claims are flagged `pending verification` until checked against the real source. The verification log is `docs/attribution.md`.

## Conduct

All contributors — human and agent — follow [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md).

## Reporting vulnerabilities

See [`SECURITY.md`](SECURITY.md).
