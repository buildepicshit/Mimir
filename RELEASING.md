# Releasing Mimir

This document is the canonical runbook for cutting a release. The pipeline lives in `.github/workflows/release.yml`.

Mimir follows [SemVer](https://semver.org/) for published releases. Pre-1.0 series may carry breaking wire-format changes between minor versions and will be called out in `CHANGELOG.md` under `### Changed — wire format`.

## Versioning policy

- **`vMAJOR.MINOR.PATCH`** for stable releases (`v1.0.0`, `v1.1.0`, `v1.0.1`).
- **`vMAJOR.MINOR.PATCH-alpha.N` / `-beta.N` / `-rc.N`** for prereleases. The release workflow auto-flags any tag containing `-` as a GitHub prerelease.
- The `[workspace.package].version` field in the root `Cargo.toml` is the source of truth. The release workflow refuses to run if a pushed tag does not match.

## Pre-release checklist

Before pushing a tag:

1. **Confirm CI is green on `main`.** The release workflow re-runs the same checks but failing them mid-release is annoying.
2. **Update `CHANGELOG.md`.** Promote the `## [Unreleased]` block to `## [x.y.z] - YYYY-MM-DD`. Re-add an empty `## [Unreleased]` at the top. The release notes are extracted verbatim from this section.
3. **Bump `[workspace.package].version`** in the root `Cargo.toml` to the new version. Run `cargo build` once locally to refresh `Cargo.lock`.
4. **Sanity-run the dry-run path** via `Actions` → `Release` → `Run workflow`, leaving `real_publish` as `false`. This exercises every job except real `crates-publish`; the `github-release` job still verifies release artifacts but does not create a GitHub Release on manual dispatch. ~6–8 minutes wall clock.
5. **Open a PR with the version bump + CHANGELOG promotion.** Squash-merge.

## Cutting the release

```bash
git checkout main
git pull --ff-only
# Confirm the version is what you intend.
grep '^version' Cargo.toml | head -n 1

# Sign and push the tag. Triggers the release workflow.
git tag -s v0.1.0-alpha.1 -m "v0.1.0-alpha.1"
git push origin v0.1.0-alpha.1
```

The workflow will:

1. **Verify the tag matches `Cargo.toml`** (`verify-version` job).
2. **`cargo publish --dry-run`** for `mimir-core` (`dry-run-publish`). Dependent crates are dry-run inside `crates-publish` after their upstream crate exists in the crates.io index.
3. **`cargo install --path crates/mimir-cli`** and **`cargo install --path crates/mimir-harness`** + assert `mimir-cli --version` and `mimir --version` report the expected version (`smoke-install`).
4. **Build release binaries** for 5 targets (`build-binaries`):
   - `x86_64-unknown-linux-gnu`
   - `aarch64-unknown-linux-gnu`
   - `x86_64-apple-darwin`
   - `aarch64-apple-darwin`
   - `x86_64-pc-windows-msvc`
5. **Draft a GitHub Release** with the matching CHANGELOG section as the body and all 5 binary archives + SHA-256 checksums attached (`github-release`).
6. **Publish to crates.io** (`crates-publish`) — `mimir-core` first, 30s wait for index propagation, then dry-run/publish `mimir-cli`, `mimir-mcp`, `mimir-librarian`, and `mimir-harness` in dependency order with an index-propagation wait between each publish. Requires the `crates-io` deployment environment + `CRATES_IO_TOKEN` repo secret. On tag pushes, crates publish only after release binaries build and the draft GitHub Release is created.

## Post-release

1. Open the drafted GitHub Release in the UI, double-check the body and assets, then click **Publish release**.
2. Verify the crates land:
   - https://crates.io/crates/mimir-core/x.y.z
   - https://crates.io/crates/mimir-cli/x.y.z
   - https://crates.io/crates/mimir-mcp/x.y.z
   - https://crates.io/crates/mimir-librarian/x.y.z
   - https://crates.io/crates/mimir-harness/x.y.z
3. Smoke from a clean machine:
   ```bash
   cargo install mimir-cli --version x.y.z
   mimir-cli --version
   cargo install mimir-librarian --version x.y.z
   mimir-librarian --version
   cargo install mimir-harness --version x.y.z
   mimir --version
   ```
4. Announce per the roadmap's Phase 5 communications plan.

## Recovery

- **Tag pushed with a mismatched version.** The `verify-version` job fails fast before any publish. Delete the tag (`git tag -d vX.Y.Z; git push --delete origin vX.Y.Z`), fix the version, re-tag.
- **Earlier crate published, later crate failed.** crates.io is append-only. Bump to the next patch (`x.y.(z+1)`), promote a fresh CHANGELOG section, retry. The orphaned version of an earlier crate (`mimir-core`, `mimir-cli`, `mimir-mcp`, `mimir-librarian`, or `mimir-harness`) is harmless.
- **`crates-publish` failed because `CRATES_IO_TOKEN` is missing.** Configure under repo settings → Secrets and variables → Actions → New repository secret. Re-run the failed job.
- **Yanked release.** `cargo yank --vers x.y.z -p mimir-core` (and/or `-p mimir-cli`, `-p mimir-mcp`, `-p mimir-librarian`, `-p mimir-harness`). yanking blocks new uses but preserves resolution for existing `Cargo.lock`s.

## Why hand-rolled (vs. cargo-dist)

The roadmap (Phase 1.5) names `cargo-dist` as a future option. The current pipeline is hand-rolled because:

- Pre-1.0, every release surface is something we want to *read* end-to-end, not delegate.
- We already have only 5 explicit targets and no installer story; cargo-dist's strengths (multi-target installers, shell installers, brew formulas) are post-1.0 concerns.
- The hand-rolled pipeline is ~250 lines of YAML we control completely. cargo-dist would replace this with a generated workflow + `[workspace.metadata.dist]` config block.

Migrate to cargo-dist when:
- We start cutting `vN.N.0` minors at a steady cadence (~quarterly).
- We need a one-line installer story (`curl -sSfL ... | sh`).
- We want auto-generated brew/scoop/winget submissions.

Until then, this file is the source of truth for how Mimir ships.
