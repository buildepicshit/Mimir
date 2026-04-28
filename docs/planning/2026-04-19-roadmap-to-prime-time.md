# Roadmap to Prime Time

> **Document type:** Planning — phased path from "feature-complete pre-1.0" to "publicly listed Rolls Royce product."
> **Last updated:** 2026-04-19
> **Status:** v1, locked decisions; phases sequenced for solo execution.
> **Source:** historical pre-public engineering assessments from the parent workspace. The public repo keeps this roadmap for provenance, but the assessment scratch files are not shipped here.
> **Cross-link:** the workspace-level master plan lives at [`../../../analysis/plans/2026-04-19-engram-to-prime-time.md`](../../../analysis/plans/2026-04-19-engram-to-prime-time.md). This file is the in-repo authoritative copy.

## Purpose

Mimir has tier-1 engineering substrate (live test count in [`STATUS.md`](../../STATUS.md) frontmatter; fuzz harness, crash-injection matrix, integer-fixed-point determinism, all 14 architecture specs `authoritative`). The "agent unreachable" P0 that drove this roadmap's original drafting has **closed**: `mimir-mcp` v0.1 shipped in Phase 2 with 9 tools (read + write + lease state machine). The forward work is earning v1.0 honestly: close remaining post-cutover residue, validate the wire-surface thesis (Phase 3.2 LLM-fluency gate), ship the first verified-use writeup, tag + publish to crates.io, flip public, land the marketplace listings.

`STATUS.md` points at this document for the detailed roadmap and carries only the current-phase snapshot. The post-cutover re-audit (2026-04-20) is what the "Next milestones" in `STATUS.md` now track.

## Locked decisions

Four Day-1 decisions; revisit only if a phase surfaces blocking evidence:

1. **Agent surface shape: `mimir-mcp` Rust crate.** Targets Claude (Claude Desktop + Claude Code) for the shipped workspace-local MCP surface. As of the 2026-04-24 mandate update, future agent surfaces are adapter-mediated; Claude and Codex are first targets, and unsupported clients require explicit adapter/spec work.
2. **Public flip timing: Phase 5** (after marketplace listing + first public-test writeup). Crates.io publication doesn't require the GitHub repo to be public — package source goes public via crates.io, but issues / PRs stay private until the flip. First impression at flip is "real product with verified external use," not "library waiting for a client."
3. **LLM-fluency hypothesis: blocks v0.1.0.** Two days of measurement in Phase 3 prevents a v1.x architecture reckoning. Mimir's existential thesis ("agent-native bytecode beats markdown for LLMs") is verified at >98% Claude-emit success rate before the API is locked.
4. **Public name: `Mimir`. Cutover executed 2026-04-20.** This repository (`buildepicshit/Mimir`) was created with fresh history as the initial commit of the rename pass. The pre-cutover history (Phase 0 → Phase 2.4 + the 2026-04-20 v1.1 re-audit + pre-cutover hardening) lives in the now-archived `buildepicshit/Engram` repo. See the `Naming + cutover history` section below for what was renamed and why.

## Naming + cutover history

**Trigger.** Discovered 2026-04-19 during the Phase 1.5 dry-run-publish smoke that the originally-planned crate names (`engram-core`, `engram-cli`, `engram-mcp`, and the bare `engram`) are all already taken on crates.io by unrelated active projects in the agent-memory space:

| Crate (formerly assumed) | Owner / project | Description | Snapshot |
|---|---|---|---|
| `engram` | mbednarek360 (gitlab) | Backup version-control system (off-domain) | Established, 57k downloads |
| `engram-core` (≡ `engram_core` for cargo lookup) | limaronaldo | "AI Memory Infrastructure — Persistent memory for AI agents with semantic search" | Active, 11 versions, latest 2026-03-19 |
| `engram-cli` | nexusentis | "CLI for the Engram AI agent memory system" | Active, 4 versions, latest 2026-03-08 |
| `engram-mcp` | edg-l | "MCP server for AI agent persistent memory with SQLite and local embeddings" | Active, 4 versions, latest 2026-03-23 — directly occupied our planned Phase 2 product surface |

Three different teams shipping "engram"-branded agent-memory products in the past six months. The chosen replacement: **Mimir** (Norse: Mímir, the wise being whose preserved head Odin consulted for counsel). Verified free 2026-04-19 and re-verified 2026-04-20: `mimir-core`, `mimir-cli`, `mimir-mcp`, `mimir-store`, `mimirdb`. The bare `mimir` is taken by an abandoned 2018 Oracle DB binding (off-domain, not blocking).

**Cutover sequence (executed 2026-04-20):**

1. Verified `mimir-core` / `mimir-cli` / `mimir-mcp` still free on crates.io.
2. Throwaway branch from `Engram` `main` containing the pre-cutover hardening (`chore/pre-cutover-cleanup`).
3. Full mechanical rename pass on top of that branch: directory renames, source identifier renames, env-var renames, MCP tool-name renames, magic-header rename (`EGRM` → `MIMR`), tracing event renames (`engram.*` → `mimir.*`), repo URL updates, prose rebrand.
4. Verified `cargo build --workspace`, `cargo test --workspace` (453 passing), `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and the full drift gate suite all green.
5. Created `buildepicshit/Mimir` as a new private GitHub repo.
6. Pushed the rename branch to the new repo as its initial `main` with **fresh history** (squashed to a single "Initial: import Mimir from Engram cutover" commit).
7. Archived (didn't delete) `buildepicshit/Engram` so internal links + this repo's pre-history stay live.
8. Local `~/buildepicshit/Engram/` clone dropped; `~/buildepicshit/Mimir/` cloned fresh.
9. (Phase 4) Cut `v0.1.0-alpha.1` and `cargo publish -p mimir-core` then `-p mimir-cli` from the public repo. First published artifact under the final name.
10. (Phase 5) Marketplace listing for `mimir-mcp`.

This document — and the rest of the codebase — uses `mimir-*` everywhere. References to "Engram" persist only in this historical section and in `CHANGELOG.md`'s "public-name cutover" entry.

## North star

Mimir reaches the bar Wick now meets ("Rolls Royce product"), publicly listed on crates.io and the Claude / MCP marketplace, with at least one external-user signal and zero unverified claims in any first-contact doc surface.

## Phase summary

| Phase | Days | Output | Headline grade trajectory |
|---|---|---|---|
| 0. Audit-to-fix sweep | Day 1 | One PR closing 13 quick wins (proves the loop) | D → D (still capped by P0) |
| 1. Foundation hardening | Days 2–7 | Threat model + 3-OS CI + drift gates + decoder DoS fixes + release pipeline scaffold | D → D |
| 2. Build `mimir-mcp` v0 | Days 8–15 | The wedge — closes the P0 | **D → C** |
| 3. Validation + first public test | Days 16–21 | Inferential resolver + LLM-fluency benchmark + 1 dogfood writeup | **C → B** |
| 4. v0.1.0-alpha cut + crates.io publish | Days 22–28 | Tag + release pipeline + first published artifact | B → B |
| 5. Public flip + marketplace listing | Days 29–42 | Repo public + Glama + `modelcontextprotocol/servers` PR + 2nd writeup | B → B+ |
| 6. v1.0 path | Days 43–60+ | All Rolls Royce backlog closed + v1.0 cut earned | B+ → A |

## Sequencing principles

1. **Prove the audit-to-fix loop first.** Phase 0 is one sub-day PR closing 13 quick wins. This is itself a Rolls Royce property (Wick's distinguishing one). Demonstrates the team can absorb an external audit and ship the response in <24 hours.
2. **Foundation before features.** Phase 1 ships everything that doesn't depend on `mimir-mcp`. Closes ~half the backlog before any new feature work.
3. **The wedge before everything downstream.** Phase 2 (`mimir-mcp`) is the gate. Public-testing writeup, demo screencap, "Built with Mimir" pattern, marketplace listing — all blocked on this.
4. **Validate the thesis before locking the API.** Phase 3 LLM-fluency benchmark catches a wrong wire surface before publication.
5. **Earn the version tag.** Phase 4 cuts `v0.1.0-alpha.1` *only after* the substrate + the wedge + one verified use exist. Don't repeat Wick's v1.0-incongruity mistake.
6. **Public flip last.** Phase 5 needs the substrate + the wedge + the first writeup to be honest at the flip moment.

---

## Phase 0 — Audit-to-fix sweep (Day 1)

> **Goal:** One PR landing 13 morning quick-wins in a sub-day cycle. Proves the audit-to-fix loop.

### Deliverables (XS each, one PR — *this PR*)

1. **README sweep** — drop "Design. No production code yet"; add 5-line quickstart; truthful status (245 tests, 14 specs authoritative). `README.md`.
2. **AGENTS.md "Where to Look" table** — mark `docs/concepts/` and `docs/attribution.md` as authored. `AGENTS.md:90-96`.
3. **CONTRIBUTING design-phase removal** — drop the "no build setup required" stanza; promote actual `cargo build / test / clippy / fmt` commands. `CONTRIBUTING.md:11-25`.
4. **CHANGELOG `[Unreleased]` Pending cleanup** — drop the cargo-fuzz-pending line (the harness shipped). `CHANGELOG.md:84-86`.
5. **CHANGELOG header gitignored claim** — drop "(gitignored during design phase)" reference to PRINCIPLES.md. `CHANGELOG.md:5`.
6. **PRINCIPLES.md graduation banner** — replace "Draft, gitignored" with "Tracked since Phase 2 (PR #2)". `PRINCIPLES.md:3`.
7. **`.gitignore` design-phase comment** — drop the empty design-phase block. `.gitignore:23-24`.
8. **`.gitignore` runtime artifacts** — add `fuzz/artifacts/`, `fuzz/coverage/`, `*.profraw`, `*.profdata`.
9. **Per-crate Cargo metadata** — add `keywords`, `categories`, `readme`, `homepage`, `documentation` to both `[package]` blocks.
10. **Per-crate `LICENSE`** — symlinks from each crate dir to root LICENSE (Apache-2.0 § 4(a) compliance gap on first publish).
11. **Per-crate `README.md`** — 50-line `crates/mimir_core/README.md` and 30-line `crates/mimir-cli/README.md`, tailored to each crate's API.
12. **CODEOWNERS per-path rules** — explicit ownership for `/crates/mimir_core/src/{log,store,canonical}.rs` and `/.github/workflows/`.
13. **STATUS.md "What's shipped" body catch-up** — collapse the per-milestone narration tail to a CHANGELOG-fed summary so 6.x → 9.x stop being invisible.

(Out-of-tree: prune ~50 stale remote branches via `gh pr list --state merged | xargs git push origin --delete` after this PR merges. Enable "Automatically delete head branches" in repo settings.)

### Exit criteria
- One PR open, CI green.
- PR description references this plan + the analysis report.
- Cycle time PR-open → merge: **<8 hours.** Missing this is a real signal.

---

## Phase 1 — Foundation hardening (Days 2–7)

> **Goal:** Everything that doesn't depend on `mimir-mcp` but raises the bar across security, drift, CI, and release scaffold.

### 1.1 — Threat model in SECURITY.md (1 day)

- Replace `SECURITY.md:27-39` generic policy with explicit:
  - **In-scope (5–7 bullets):** untrusted Lisp into the librarian; untrusted canonical bytes into `mimir-cli verify`; future MCP stdio listener; unsafe-code regression; supply-chain (cargo-deny escape).
  - **Out-of-scope (4–5 bullets):** network transport (in-process-only by `wire-architecture.md`); multi-writer concurrency (single-`&mut Store` invariant); side-channel; physical access; compromised local toolchain.
- Add triage SLA target.
- Add "Data classification expectations" subsection per Security F9 (canonical store opaque to Mimir; symbol table at same classification level; tracing emits identifiers only).

### 1.2 — 3-OS CI + Dependabot + CodeQL + Scorecard + cargo audit (1 day)

- Expand `.github/workflows/ci.yml` `test` and `clippy` jobs to `strategy.matrix.os: [ubuntu-latest, macos-latest, windows-latest]` + `fail-fast: false`.
- Add `x86_64-pc-windows-msvc` to `deny.toml:4-8` `targets`.
- `.github/dependabot.yml` for `package-ecosystem: cargo` (weekly) + `github-actions` (weekly).
- `.github/workflows/codeql.yml` from standard template.
- `.github/workflows/scorecard.yml` from `ossf/scorecard-action`.
- `cargo audit` step in CI as defense-in-depth.
- Pin all GitHub Actions to commit SHAs with version comments.

### 1.3 — Drift gates (1 day)

Add `crates/mimir_core/tests/doc_drift_tests.rs`:
- `status_banner_consistency` — every `docs/concepts/*.md` has a parseable `> **Status: <state>` first-paragraph banner; if `Cargo.toml::version >= 1.0.0` then every spec must be `authoritative`.
- `readme_no_design_phase_lies` — `README.md` MUST NOT contain "no production code yet" or "to be authored" while `crates/` has any `.rs` file > 50 lines.
- `version_consistency` — `Cargo.toml [workspace.package] version` matches latest git tag (skipped when no tags; required after Phase 4).
- `agents_md_table_consistency` — AGENTS.md table cannot mark a `docs/concepts/*.md` file as "to be authored" when that file exists.

### 1.4 — Close three security latents (1 day)

- **F1 (Store::open destructive truncate):** Add 4-byte magic header `b"MIMR"` + 4-byte format-version `u32::to_le_bytes(1)` written by `CanonicalLog::append` on first write; validate on every `open`; refuse to truncate non-empty file with mismatched magic. New `LogError::IncompatibleFormat { found, expected }`.
- **F2 (decoder Vec::with_capacity OOM):** Cap `count.min(bytes.len() - *offset)` in all three sites (`canonical.rs:929-933, 1017-1022, 1125-1130`). Add fuzz seed `fuzz/corpus/fuzz_decoder/seed_huge_count.bin` reproducing the pre-fix OOM.
- **F3 (parser stack overflow):** Track `depth` in parser state; return `ParseError::NestingTooDeep { pos, limit, max: 256 }`. Unit tests at boundary.

### 1.5 — Release pipeline scaffold (no actual publish yet) (1 day)

`.github/workflows/release.yml` triggered on `v*` tag push:
- Verify tag matches `[workspace.package] version`.
- `cargo publish --dry-run -p mimir_core` then `--dry-run -p mimir-cli`.
- `cargo install --locked --path crates/mimir-cli && mimir-cli --version` smoke step.
- Generate GitHub Release from CHANGELOG section.
- `cargo-dist` cross-platform `mimir-cli` binaries (Linux x86_64/ARM64, macOS x86_64/ARM64, Windows x86_64).
- Crates.io publish step gated on `secrets.CRATES_IO_TOKEN` + `inputs.real_publish == true` so the workflow is end-to-end testable via `workflow_dispatch` without publishing.

### Phase 1 exit criteria
- All 4 sub-phases merged.
- 249 tests passing on all 3 OSes.
- All 4 security workflows green.
- Release workflow dry-run passes.
- 0 P1 security findings remain.

---

## Phase 2 — Build `mimir-mcp` v0 (Days 8–15)

> **Goal:** Close the P0. Make Mimir callable by an actual Claude. The wedge.

### 2.1 — Scaffold (Day 8)

- `crates/mimir-mcp/` binary crate depending on `mimir_core` only.
- Add to `Cargo.toml` workspace `members`.
- Pull in `rmcp` (verify in `deny.toml` allowlist).
- First tool: `mimir_status` — returns `{ workspace_id?, log_path?, version }`.

### 2.2 — Read-side tools (Days 9–10)

- `mimir_read` — wraps `Pipeline::execute_query`.
- `mimir_verify` — wraps `mimir-cli::verify`.
- `mimir_list_episodes` — paginated, `committed_at` ordered.
- `mimir_render_memory` — wraps `LispRenderer::render_memory`.

### 2.3 — Write-side tools with workspace lease (Days 11–13)

- `mimir_open_workspace` returns `{ workspace_id, lease_token, lease_expires_at }`.
- `mimir_write` — takes batch + lease token; wraps `Store::commit_batch`.
- `mimir_close_episode` — lease-gated.
- `mimir_release_workspace` — explicit release. Auto on TTL.
- Single-writer invariant enforced by lease holder.

### 2.4 — Documentation + integration test (Days 14–15)

- `docs/integrations/claude-code-hook.md` — `PreToolUse` / `PostToolUse` / `SessionStart` / `SessionStop` recipe.
- `docs/integrations/claude-desktop-config.md` — `claude_desktop_config.json` snippet.

(Originally planned: a `cursor-mcp-config.md` doc. Dropped during Phase 2.4 implementation under the then-Claude-only mandate. After the 2026-04-24 mandate update, unsupported client docs still require explicit adapter/spec work.)
- `crates/mimir-mcp/README.md` — embedded NuGet-style README.
- `crates/mimir-mcp/tests/end_to_end.rs` driving the full lifecycle through stdio.
- `crates/mimir-mcp/tests/tool_catalog_drift.rs` — every advertised tool exists in the registry and vice versa.

### Phase 2 exit criteria
- 9 tools (4 read + 5 write/lifecycle).
- Integration test green.
- Drift gate prevents tool-vs-doc regressions.
- **P0 closed. Headline grade: D → C.**

### Risks
- **`rmcp` API churn.** Pin version. Document choice in `PRINCIPLES.md` § 8.
- **Lease model complexity.** Day-11 spike before committing. Fall back to file-lock if lease state machine surfaces fundamental issues.
- **Stdio framing edge cases.** Test with `mcp-inspector` early (Day 9, not Day 14).

---

## Phase 3 — Validation + first public test (Days 16–21)

> **Goal:** Verify the LLM-fluency hypothesis. Wire the Inferential resolver. Run the first real public-test pass and write it up.

### 3.1 — Inferential resolver wired (Days 16–17)

- Implement deferred resolver tracked by issue #29.
- `(query :kind inf)` returns committed inferentials with proper supersession + parent-tracking.
- Property test: "for any committed Inferential, a matching `(query :kind inf)` returns it."
- Update `read-protocol.md` graduation banner.

### 3.2 — LLM-fluency benchmark (Day 18)

- 100-prompt corpus across 4 task shapes: `(epi ...)` (×25), `(sem ...)` (×25), queries (×25), `(pro ...)` (×25).
- Run each through Claude (current default) ×3 trials = 300 attempts.
- Measure: parse-success rate, error distributions, mean tokens per fact.
- Document in a dated benchmark report after the run.
- **Exit gate: >98% parse-success rate.** If <98%: stop, course-correct the wire surface, re-run.

### 3.3 — First public-test pass (Days 19–21)

- Pick a target. Real BES Studios codebase or a fresh small Godot C# project.
- Configure `mimir-mcp` against the target. Multi-hour Claude session.
- Capture session trace from `~/.claude/projects/...`.
- Write `docs/public-testing/2026-MM-DD-<target>-pass.md` matching `Wick/docs/public-testing/2026-04-15-bes-splash-3d-pass.md` shape.
- ≥5 specific actionable findings filed.

### Phase 3 exit criteria
- All 4 memory types work end-to-end.
- LLM-fluency >98% (or course-correction landed).
- One real-session writeup with verified `mcp__engram__*` tool calls.
- Headline grade: **C → B**.

---

## Phase 4 — v0.1.0-alpha cut + crates.io publish (Days 22–28)

### 4.1 — Demo / visual artifact (Days 22–23)
- Architecture SVG (Lex → Parse → Bind → Semantic → Emit → CanonicalLog).
- Asciicast of `mimir-cli verify` + `mimir-cli decode` against a 3-Episode example.
- 30-second screencap of Claude-via-mimir-mcp.

### 4.2 — README rewrite for public discovery (Day 24)
- One-sentence value prop in lines 1–3.
- Badge row.
- Install + quickstart that work.
- Architecture diagram inline.
- Privacy disclosure: "No outbound network requests, no telemetry, no crash reporter."

### 4.3 — `mimir-cli diagnose` fan-out tool (Day 25)
Mimir analogue of Wick's `runtime_diagnose`. One-call replacement for `log` + `symbols` + `verify` + `decode`.

### 4.4 — Naming check + crates.io reservation rehearsal (Day 26)
- `cargo search` for all 3 names.
- Decide snake-case vs kebab-case (one-way door once published).
- `cargo publish --dry-run` for both crates locally.

### 4.5 — Tag + publish (Days 27–28)
- `version = "0.1.0-alpha.1"` in `Cargo.toml`.
- Roll `[Unreleased]` into `[0.1.0-alpha.1] - 2026-MM-DD`.
- `git tag -s v0.1.0-alpha.1`.
- `release.yml` does the rest.
- Verify `cargo install mimir-cli --version 0.1.0-alpha.1` works on all 3 OSes.

### Phase 4 exit criteria
- v0.1.0-alpha.1 published on crates.io.
- `cargo install mimir-cli` works on all 3 OSes.
- README is publication-ready.
- Headline grade: **B (4.05 weighted, OSS Readiness 4/5)**.

---

## Phase 5 — Public flip + marketplace listing (Days 29–42)

### 5.1 — Coordinated wording flip (Day 29)
Single PR sweeping every "private" / "design phase" reference. The drift gate from Phase 1.3 catches future regressions.

### 5.2 — Repo public flip (Day 30)
- Pre-flip audit: `git log --all -p` secret scan.
- Enable Private Vulnerability Reporting + Discussions.
- Branch protection on `main` (3-OS CI green, linear history, signed commits).
- Tag protection on `v*`.
- Flip public via Settings → General.

### 5.3 — Marketplace listings (Days 31–35)
- `glama.json` manifest at repo root.
- PR to `modelcontextprotocol/servers` community section (Claude reference + community implementations index).
- `smithery.ai` submission (deployable container image).
- Claude marketplace — official Anthropic listing once available, otherwise the `claude.ai/mcp` discovery surface.
- (Cross-model in-client directories — Cursor, Cline, Continue, Windsurf — remain out of scope for `mimir-mcp` until explicit adapter/spec work exists.)

### 5.4 — `mimir-mcp` v0.1.0 cut + publish (Day 36)
Promote `mimir-mcp` from local-only to crates.io.

### 5.5 — Second public-test writeup (Days 37–42)
Different target from Phase 3.3. Diff against the first writeup. Demonstrates the audit-to-fix loop is repeatable.

### Phase 5 exit criteria
- Repo public.
- Listed on at least 2 marketplaces.
- Two public-test writeups landed.
- First external user signal realistic.
- Headline grade: **B+ (4.2 weighted, OSS Readiness 5/5)**.

---

## Phase 6 — v1.0 path (Days 43–60+)

> **Goal:** Earn the v1.0 tag Wick had to half-fake. Land all `before-1.0` Rolls Royce backlog items.

Loosely sequenced (no hard ordering between items):

- Write-path criterion bench parallel to `read_path.rs`. Closes Architecture F1 / Testing F9.
- Cross-OS `fsync` / `truncate` semantics tests.
- Continuous fuzzing in CI (nightly per PRINCIPLES.md § 11). Adds `fuzz_compile_batch` end-to-end target.
- `cargo-llvm-cov` HTML coverage as workflow artifact (signal, not gate).
- OIDC trusted publishing migration (drop `CRATES_IO_TOKEN`).
- Pre-commit hooks (`lefthook.yml`).
- `.editorconfig`.
- Submodule decomposition for files >1700 LOC (`pipeline.rs`, `store.rs`, `canonical.rs`, `read.rs`).
- WorkspaceId git-remote canonicalization (SSH ↔ HTTPS).
- `parse_timestamp` fail-fast.
- Procedural activity weighting + Inferential parent-tracking (Testing F12).
- `docs/operations.md` deployment guide.
- `docs/integrations/built-with-engram.md` attribution surface.
- Claude Code Skill wrapping `mimir-mcp` (Decision 1's deferred follow-up).

### Phase 6 exit criteria
- All 26 rows of the Rolls Royce backlog closed.
- Two public-test writeups + at least one external bug filed.
- LLM-fluency re-run on current Claude with >98% sustained.
- Headline grade: **A (4.6 weighted)**.

### v1.0 cut (when Phase 6 exit criteria met)
- `version = "1.0.0"`.
- `[Unreleased]` → `[1.0.0] - 2026-MM-DD`.
- Pre-cut audit: SECURITY.md threat-model items either shipped or honestly framed as v1.x scope.
- `git tag -s v1.0.0`; `release.yml` does the rest.
- Announcement post (HN, lobste.rs, r/rust, Anthropic Discord MCP channel) referencing the audit-to-fix loop as the marketable artifact.

---

## Risks and mitigations

| Risk | Phase | Likelihood | Mitigation |
|---|---|---|---|
| LLM-fluency hypothesis fails (<98%) | 3 | Medium | Course-correct wire surface BEFORE locking the API. Pivot plan: Lisp-with-builder-helpers or JSON fallback ingest. |
| `rmcp` immature / API churn | 2 | Medium | Pin version. TS fallback plan: write `mimir-mcp` as TypeScript wrapper shelling to `mimir-cli` (changes Phase 2 from XL to L). |
| Solo-author bus factor | All | Low day-to-day, High strategically | Document everything as you go. The plan itself is a contributor-onboarding artifact. Phase 5.2 enables Discussions specifically. |
| Single-writer lease model surfaces fundamental issues | 2 | Medium | Day-11 spike. Fall back to file-lock if too complex. |
| Naming collision on registries | 4 | Low | Phase 4.4 runs check before publish. Reserve names with `0.0.0-rc.0` shells if uncertain. |
| Public flip surfaces a forgotten secret in git history | 5 | Low | Pre-flip audit in Phase 5.2. |
| First public-test writeup reveals deal-breaker | 3 | Low-Medium | That's the point of running it before v0.1.0-alpha. |
| Marketplace inclusion PRs sit unreviewed for weeks | 5 | High | Don't block subsequent phases on marketplace acceptance. |
| `cargo-dist` cross-platform binary build breaks on Windows | 4 | Medium | Test in Phase 1.5 release-pipeline scaffold. Fallback: Linux + macOS only for `cargo-dist`. |

---

## Decision checkpoints (between phases)

Before starting each phase, gate on the previous phase's exit criteria. Premature advancement is the most common way phased plans collapse.

| Gate | Question | If "no" |
|---|---|---|
| 0 → 1 | Did the morning quick-wins PR merge in <8 hours? | Identify bottleneck (review latency? CI flakiness? scope creep?) and fix that *first*. |
| 1 → 2 | All 4 sub-phases merged + 249 tests green on 3 OSes? | Don't start `mimir-mcp`; new code surface inherits any unresolved CI / drift gaps. |
| 2 → 3 | Does `mcp-inspector` show a working write+read+close roundtrip? | Don't write the LLM-fluency benchmark or the public-test writeup against a half-broken surface. |
| 3 → 4 | LLM-fluency ≥98%? First public-test writeup landed with verified tool calls? | Don't cut the alpha tag without these. |
| 4 → 5 | Does `cargo install mimir-cli` work on all 3 OSes from a clean shell? | Don't go public until install works. |
| 5 → 6 | Two writeups + 2 marketplace listings + at least one external user signal? | Without external signal, v1.0 is the same lie Wick's v1.0 currently is. Wait. |

---

## Out of scope for this plan

- `mimird` daemon / network transport (`wire-architecture.md` graduated to in-process only on 2026-04-19).
- Multi-agent / multi-writer support (single-writer-per-workspace is the architectural commitment).
- Vector embeddings (Mimir's category position is "structured agent memory," not vector DB).
- Hosted SaaS (Rust crate + local MCP server).
- Web UI (CLI + MCP is enough for v1.0).
- Coverage gating (Phase 6 ships coverage as signal, not gate; PRINCIPLES.md § 7 stance).
- README rewrite into corporate-marketing voice.

---

## Plan version history

| Version | Date | Notes |
|---|---|---|
| v1 | 2026-04-19 | Initial draft after the v1.1 fresh assessment. Sequenced for solo execution; 6–9 weeks wall time. |

> **How to use this plan:** check off deliverables as PRs merge. Update phase exit criteria with actual evidence (PR links, test counts, CI runs). When a decision checkpoint says "no," document why and what changed. This plan is a working document, not a contract — but the sequencing is real, and the gate criteria are load-bearing.
