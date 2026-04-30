# Mimir Green Room Product Evaluation

## Repo State

| Field | Value |
|---|---|
| Repo | `Mimir` |
| Branch | `main` |
| Head | `9e81c0f` |
| Tracking | `origin/main` |
| Dirty state | Yes — `M .gitignore`, `M AGENTS.md`, ~40 untracked agent-control/setup files |
| Public/private | **Public OSS** (Apache-2.0) |
| Handoff status | `owner-paused` per triage SPEC 2026-04-29 |
| Version | `0.1.0` (no release tag yet) |

Dirty state detail captured from
`Mimir/.agents/specs/2026-04-29-parallel-handoff-closeout/SPEC.md` Section 4:

- `M .gitignore` — BES agent-control addition.
- `M AGENTS.md` — BES operating manual update.
- Untracked: `.agents/` tree (~25 files including specs, skills, workflows,
  scripts, mcp config), `.claude/` tree (~10 files including commands, settings,
  skills), `CLAUDE.md`, `WORKFLOW.md`, `.mcp.example.json`.
- No tracked product source files are modified.

## Primary Agent

| Field | Value |
|---|---|
| Agent | Claude Code |
| Model | `claude-opus-4-6` (Claude Opus 4.6) |
| Reasoning mode | default (xhigh not explicitly set for this run) |
| Date | 2026-04-30 |
| Network used | No (local file reads only; git commands to Mimir denied by sandbox) |

## Sources Read

Root authority:

| File | Purpose |
|---|---|
| `.agents/GREEN_ROOM_EVALUATION.md` | Green room protocol |
| `.agents/OPERATING_MODEL.md` | Fleet operating contract |
| `.agents/MODEL_ROUTING.md` | Agent/model routing policy |
| `.agents/DOCUMENTATION_GUIDE.md` | Documentation placement rules |
| `.agents/WORKSPACE_LAYOUT.md` | Root workspace layout |
| `.agents/specs/2026-04-29-green-room-product-evaluations/SPEC.md` | Fleet evaluation dispatch spec |
| `.agents/specs/2026-04-29-handoff-triage/SPEC.md` | Handoff triage decisions |

Mimir authority:

| File | Purpose |
|---|---|
| `Mimir/AGENTS.md` | Repo operating manual, architectural invariants, design space |
| `Mimir/CLAUDE.md` | Claude entry point |
| `Mimir/WORKFLOW.md` | Symphony workflow contract |
| `Mimir/STATUS.md` | Current phase, CI state, launch work order |
| `Mimir/README.md` | Public entry point, what-works / what-is-not-claimed |
| `Mimir/PRINCIPLES.md` | Engineering principles, testing strategy, error handling |
| `Mimir/CHANGELOG.md` | Unreleased section (first 100 lines) |
| `Mimir/RELEASING.md` | Release runbook |
| `Mimir/Cargo.toml` | Workspace members, dependencies, lint config |
| `Mimir/docs/README.md` | Public docs index |
| `Mimir/docs/concepts/README.md` | Authoritative implementation spec catalog |
| `Mimir/docs/launch-readiness.md` | OSS readiness checklist, engineering gate evidence |
| `Mimir/.github/workflows/ci.yml` | CI pipeline (first 80 lines) |

Mimir handoff/audit specs:

| File | Purpose |
|---|---|
| `Mimir/.agents/specs/2026-04-29-parallel-handoff-closeout/SPEC.md` | Captured git state, in-flight work table, owner decisions |
| `Mimir/.agents/specs/2026-04-29-realignment-handoff/SPEC.md` | Branch/dirty state, work classification |
| `Mimir/.agents/specs/2026-04-29-repo-audit/SPEC.md` | Code inventory, proposed specs |

Code inventory (via `ls`, `wc -l`, `find -type f`):

| Path | Files | LOC |
|---|---|---|
| `mimir-core/src/` | 22 | ~18,054 |
| `mimir-librarian/src/` | 12 | ~16,139 |
| `mimir-harness/src/` | 2 | ~10,980 |
| `mimir-mcp/src/` | 3 | ~1,357 |
| `mimir-cli/src/` | 2 | ~964 |
| **Source total** | **41** | **~47,494** |
| Test files | 13 | ~8,575 |
| Total `.rs` files | 162 | — |

## Commands Run

| Command | Result |
|---|---|
| `ls Mimir/` | Success — listed repo top-level contents |
| `ls Mimir/mimir-core/src/` | Success — 22 source files |
| `ls Mimir/mimir-librarian/src/` | Success — 12 source files |
| `ls Mimir/mimir-harness/src/` | Success — 2 source files |
| `ls Mimir/mimir-mcp/src/` | Success — 3 source files |
| `ls Mimir/mimir-cli/src/` | Success — 2 source files |
| `wc -l Mimir/mimir-core/src/*.rs` | Success — 18,054 LOC |
| `wc -l Mimir/mimir-librarian/src/*.rs` | Success — 16,139 LOC |
| `wc -l Mimir/mimir-harness/src/*.rs` | Success — 10,980 LOC |
| `wc -l Mimir/mimir-mcp/src/*.rs` | Success — 1,357 LOC |
| `wc -l Mimir/mimir-cli/src/*.rs` | Success — 964 LOC |
| `find Mimir/mimir-core/tests -type f -name '*.rs'` | Success — 5 test files |
| `find Mimir/mimir-harness/tests -type f -name '*.rs'` | Success — 4 test files |
| `find Mimir/mimir-cli/tests -type f -name '*.rs'` | Success — 2 test files |
| `find Mimir/mimir-librarian/tests -type f -name '*.rs'` | Success — 2 test files |
| `wc -l` on all test files | Success — 8,575 LOC total |
| `find Mimir -type f -name '*.rs' \| wc -l` | Success — 162 files |
| `mkdir -p Mimir/.agents/specs/2026-04-30-green-room-product-evaluation/` | Success |
| `git -C Mimir status` | **Denied** — sandbox blocked cd-then-git |
| `git -C Mimir log` | **Denied** — sandbox blocked cd-then-git |
| `git -C Mimir diff` | **Denied** — sandbox blocked cd-then-git |

**Skipped gates:**

| Gate | Reason |
|---|---|
| `cargo build --workspace` | Bash denied for Mimir git/cargo; dispatch predicted this; launch-readiness.md records pass on 2026-04-28 |
| `cargo test --workspace` | Same; launch-readiness.md records pass on 2026-04-28 |
| `cargo fmt --all -- --check` | Same |
| `cargo clippy --all-targets --all-features -- -D warnings` | Same |
| `cargo deny check` | Same |
| `cargo doc --no-deps --all-features` | Same |
| `cargo publish --dry-run` per crate | Same |

All cargo gate evidence is from `docs/launch-readiness.md` (2026-04-28 pass)
and `STATUS.md` (CI green on main after PR #11). This is **second-hand
evidence**, not a fresh gate run. The verifier should note this gap.

## Product Thesis and Target User

**Thesis** (from AGENTS.md mandate update 2026-04-24): Mimir is an
experimental local-first memory governance system for AI coding agents. It
provides a librarian-mediated, append-only, symbol-tracking memory store that
agents write to through a structured IR surface and read from through governed
retrieval. The goal is durable, auditable, cross-agent memory that survives
context resets, model changes, and session boundaries.

**Target user**: AI coding agents (Codex, Claude Code, Cursor, Copilot) and
the developers who operate them. Mimir is infrastructure, not a direct
end-user product. The human operator configures and audits; agents interact
through the harness, MCP server, or Codex plugin.

**Differentiation**: single-writer gate (librarian), append-only canonical
store, agent-native IR (not human-readable prose), compiler-shaped pipeline,
bi-temporal model, confidence decay, cross-agent consensus quorum, transparent
harness wrapping, and scoped memory isolation with governed promotion.

## Current Status vs. Last Known Roadmap

`STATUS.md` (last updated 2026-04-28) places Mimir in **pre-1.0 public launch
cleanup**:

| Status item | Evidence |
|---|---|
| CI state | Main green after PR #11 (2026-04-28) |
| Core store | Implemented (canonical.rs 2090 LOC, store.rs 2089 LOC) |
| Librarian pipeline | Implemented (pipeline.rs 2727 LOC, full lex→parse→bind→semantic→emit chain) |
| Harness | Implemented (lib.rs 8212 LOC, main.rs 2768 LOC) |
| Operator controls | Implemented (bounded context, operator memory controls, project doctor, hook validation) |
| MCP server | Implemented (server.rs 1179 LOC, rmcp 1.5.0) |
| Recovery framework | Implemented (benchmarks/recovery/ with scenarios, scoring, test_bench.py) |
| Codex plugin | Implemented (plugins/mimir/ with skills and marketplace entry) |
| Adapters | Processing adapters present (copilot_session_store.rs 1489 LOC in librarian) |
| Fuzz targets | Present (fuzz/ directory with corpus) |
| Launch readiness | All checklist items Done per docs/launch-readiness.md |
| Release tag | **Not created** — pending owner approval |
| Public publish | **Not done** — crates.io publish paused |

The launch work order in STATUS.md is:
1. Public surface scrub
2. README/docs cleanup
3. OSS readiness
4. Marketing
5. Local verification
6. Batched commit/push

Steps 1–5 appear substantially complete based on launch-readiness.md evidence.
Step 6 (batched commit/push) has not happened — the dirty state confirms local
work is uncommitted/unpushed.

**Drift from roadmap**: The BES fleet realignment paused Mimir before the
final batched commit/push. The handoff triage spec marks Mimir
`owner-paused`. The product is closer to launch-ready than any other BES repo
but the public release action was blocked by fleet policy, not by engineering
gaps.

## Engineering Quality

### Architecture

**Strong.** The architecture is well-specified across 14 authoritative concept
docs plus 2 drafts (scope-model, consensus-quorum). The crate structure
cleanly separates concerns:

| Crate | Role |
|---|---|
| `mimir-core` | Canonical store, IR pipeline, read/write, decay, inference, symbol tracking |
| `mimir-librarian` | Single-writer gate, LLM integration, drafts, quorum, processing adapters |
| `mimir-harness` | Transparent agent wrapper, operator controls, context management |
| `mimir-mcp` | MCP server exposing governed memory tools |
| `mimir-cli` | Operator CLI for direct store interaction |

Eight non-negotiable architectural invariants are documented in AGENTS.md:
1. Librarian-mediated writes (single writer gate)
2. Append-only canonical store
3. Bi-temporal model (assertion time + valid time)
4. Agent-native IR (not human-readable)
5. Structured write surface (Lisp S-expression)
6. Compiler-shaped pipeline (lex→parse→bind→semantic→emit)
7. Symbol-tracking IR with stable identity
8. Confidence decay with grounding-aware parameters

These invariants are enforced through the type system and pipeline design, not
just documentation.

**Risk**: `mimir-librarian/src/main.rs` at 8,455 LOC and
`mimir-harness/src/lib.rs` at 8,212 LOC are large single files. These are
complexity hot spots that will resist review, refactoring, and onboarding.

### Build and Test Health

**Good, with caveats.**

- Workspace lint config is strict: `missing_docs = "deny"`,
  `unsafe_code = "forbid"`, clippy pedantic with `unwrap_used`,
  `expect_used`, `panic`, `todo`, `dbg_macro` all denied.
- CI matrix covers Ubuntu, macOS, Windows with fmt, clippy, test, deny.
- CI uses pinned action SHAs, Swatinem/rust-cache, concurrency groups.
- Test corpus: ~8,575 LOC across 13 test files, plus property tests (proptest)
  and fuzz targets.
- Engineering gate passed locally on 2026-04-28 per launch-readiness.md.

**Caveats**:
- No fresh gate run during this evaluation (sandbox restrictions).
- Test-to-source ratio is ~18% by LOC — adequate for a pre-1.0 project but
  the large librarian and harness files likely have lower coverage density.
- No code coverage tooling configured.
- No integration test for the full harness→librarian→store pipeline visible
  in the test file listing (harness tests exist but the integration boundary
  is unclear without reading test content).

### CI

**Well-configured.** Cross-platform matrix (Ubuntu, macOS, Windows), cargo-deny
for dependency auditing, fmt and clippy checks, all-features test runs with
doctests. Concurrency groups prevent parallel CI waste.

**CI cost constraint**: AGENTS.md documents a hard rule — monthly GitHub
Actions budget is capped. The repo must not generate churn that wastes CI
minutes. This is a real operational constraint for any post-evaluation work.

### Dependency Risk

**Low.** `Cargo.toml` shows a focused dependency set:
- Core: `thiserror`, `ulid`, `sha2`, `serde`, `serde_json`, `toml`
- Async: `tokio`
- Database: `rusqlite` (bundled SQLite)
- MCP: `rmcp` (pinned =1.5.0)
- Testing: `proptest`, `tempfile`, `criterion`, `wait-timeout`
- Observability: `tracing`, `tracing-subscriber`
- Schema: `schemars`
- Other: `anyhow`, `getrandom`

`cargo-deny` is integrated into CI. The `rmcp` pin at `=1.5.0` is tight and
may need updating as the MCP ecosystem evolves, but for pre-1.0 this is
acceptable.

### Observability

**Present.** `tracing` and `tracing-subscriber` are workspace dependencies.
`PRINCIPLES.md` documents structured event logging with privacy rules. The
harness includes a `log.rs` (607 LOC in core) and the librarian has
`test_tracing.rs`. Actual observability depth requires code-level review.

### Security

**Good posture for pre-1.0.** `unsafe_code = "forbid"` workspace-wide.
`SECURITY.md` exists (per launch-readiness.md). Dependency auditing via
`cargo-deny`. No network-facing attack surface in the core library — the MCP
server is the primary external boundary. `rusqlite` bundled mode avoids system
SQLite version issues. Sanitisation boundary is documented in
`docs/concepts/`.

### Release Posture

**Ready but blocked.** `RELEASING.md` documents a full tag-triggered release
pipeline: verify-version → dry-run-publish → smoke-install → build-binaries
(5 targets) → github-release → crates-publish. `cargo publish --dry-run`
passed locally on 2026-04-28. The release workflow exists in
`.github/workflows/` (inferred from RELEASING.md). No tag has been created.
Owner approval is required for the first `v0.1.0` tag.

### Operational Risk

**Low for pre-1.0.** No production users, no hosted service, no SLA. The
primary operational risk is CI cost — any PR churn costs Actions minutes
against a capped monthly budget.

## Code Quality

### Maintainability

**Mixed.** The crate boundaries and type system are strong. The lint
configuration is among the strictest possible in Rust (forbid unsafe, deny
missing_docs, pedantic clippy). But two files dominate the codebase:

| File | LOC | Risk |
|---|---|---|
| `mimir-librarian/src/main.rs` | 8,455 | God-file risk; combines CLI entry, server logic, and processing in one file |
| `mimir-harness/src/lib.rs` | 8,212 | Large lib with harness logic, context management, operator controls |

These files are individually larger than many entire crates. They will be
difficult to review, test in isolation, and onboard new contributors to.

### Test Coverage

**Adequate for pre-1.0, with gaps.**

- 13 test files, ~8,575 LOC.
- Property tests via proptest.
- Fuzz targets present.
- Snapshot tests implied by the testing strategy in PRINCIPLES.md.
- No coverage tooling configured — coverage density of the large files is
  unknown.
- Recovery benchmark framework provides structured scenario testing but is
  separate from the unit/integration test suite.

### Complexity Hot Spots

1. `mimir-librarian/src/main.rs` (8,455 LOC) — needs decomposition.
2. `mimir-harness/src/lib.rs` (8,212 LOC) — needs decomposition.
3. `mimir-core/src/pipeline.rs` (2,727 LOC) — large but arguably appropriate
   for a compiler pipeline; should be reviewed for internal modularity.
4. `mimir-core/src/canonical.rs` (2,090 LOC) and `store.rs` (2,089 LOC) —
   core storage; size is proportional to responsibility but warrants review.

### Stale Code

- `docs/launch-posting-plan.md` was deleted in PR #16 but is still referenced
  from `docs/README.md` (confirmed via docs index read). This is a known
  broken link.
- `.planning/planning/` contains 14 historical planning docs. These are
  archive material and should not be treated as authority.

### Duplication

Cannot assess without code-level review. The large file sizes suggest
potential internal duplication within librarian and harness, but this is
speculative.

### Unsafe Assumptions

- `unsafe_code = "forbid"` eliminates Rust-level unsafe.
- The LLM integration in `mimir-librarian/src/llm.rs` (1,015 LOC) is a
  correctness boundary — LLM outputs fed into the compiler pipeline must be
  validated. The validator exists (`validator.rs`, 149 LOC) but its coverage
  of adversarial LLM output is unknown.
- The `rmcp` pin at `=1.5.0` assumes API stability of an early-stage MCP
  library.

### Correctness Risks

- The compiler pipeline (lex→parse→bind→semantic→emit) is the core
  correctness boundary. Property tests exist for some stages. Fuzz targets
  exist. But the pipeline is 2,727 LOC and correctness of the full chain
  depends on integration testing that was not directly observed.
- Confidence decay (`decay.rs`, 1,000 LOC) implements a mathematical model;
  correctness requires property tests and potentially formal verification.
  Property tests are present (proptest dependency) but depth is unknown.
- Bi-temporal model correctness in the canonical store is critical — temporal
  queries must respect both assertion and valid time. Testing depth unknown.

## Product Quality

### Feature Completeness

**Core feature set is implemented.** Per STATUS.md and README.md:

| Feature | Status |
|---|---|
| Canonical append-only store | Implemented |
| Librarian-mediated writes | Implemented |
| Compiler pipeline (lex→parse→bind→semantic→emit) | Implemented |
| Agent-native IR | Implemented |
| Symbol tracking with stable identity | Implemented |
| Confidence decay | Implemented |
| Bi-temporal model | Implemented |
| Transparent agent harness | Implemented |
| Operator controls (bounded context, memory controls, project doctor) | Implemented |
| MCP server | Implemented |
| Recovery benchmarks | Implemented |
| Codex plugin | Implemented |
| CLI operator tools | Implemented |
| Processing adapters (Copilot session store) | Implemented |
| BC/DR restore drill | Implemented |

**Not implemented / not claimed** (per README.md):

| Feature | Status |
|---|---|
| Production readiness | Not claimed |
| Stable API | Not claimed |
| Hosted service | Not claimed |
| Benchmark-proven superiority | Not claimed |
| Relationship/timeline APIs | Deferred |
| OCI/MCPB package | Deferred |
| Broader client recipes | Deferred |
| OpenSSF Scorecard / Best Practices Badge | Deferred |

### Demo / Showcase Readiness

**Moderate.** The transparent harness (`mimir <agent> [agent args...]`) is the
primary demo surface. The Codex plugin bundle provides an integration path.
The MCP server enables tool-based interaction. But there is no recorded demo
script, no video, no interactive tutorial beyond the README quickstart. For an
infrastructure product targeting AI agent operators, the current entry point
is adequate for technical early adopters but not for broader discovery.

### Asset and Content Readiness

**Good for pre-1.0 OSS.**

- README with quickstart.
- Docs index with concept specs, integration guides, observability docs.
- Launch article draft (docs/blog/).
- CHANGELOG with detailed unreleased section.
- Contributing guide, Code of Conduct, Security policy, issue/PR templates.
- CODEOWNERS, Dependabot config, Citation file.

### User-Facing Gaps

1. No recorded demo or tutorial beyond README quickstart.
2. Broken link: `docs/launch-posting-plan.md` deleted but still referenced.
3. No published crate — users cannot `cargo install` yet.
4. No release tag — users cannot pin a version.
5. The agent-native IR is intentionally not human-readable, which is a design
   choice but creates an onboarding barrier for operators who want to inspect
   memory state.

## Roadmap Assessment

### What Is Done

- Core architecture implemented across 5 crates (~47.5k LOC).
- All 14 authoritative concept specs have corresponding implementations.
- Engineering quality gates pass locally (2026-04-28 evidence).
- CI is green on main.
- Launch readiness checklist is fully Done.
- Public surface (README, docs, legal, community) is prepared.
- Release pipeline exists and dry-run passed.

### What Is Blocked

1. **Release tag and crates.io publish**: blocked on owner approval.
2. **Public docs/PR/CI actions**: blocked by BES fleet policy (public OSS
   constraint).
3. **Batched commit of local agent-control files**: blocked on owner approval
   of what to include vs. exclude.
4. **BES spec-authority integration**: paused per fleet realignment; design
   research needed before resuming.

### What Is Stale

- `docs/launch-posting-plan.md` reference in `docs/README.md` — the file was
  deleted in PR #16.
- `.planning/planning/` historical archive — 14 docs from pre-mandate
  planning. Not harmful but not authority.
- The `STATUS.md` launch work order implies a linear sequence that was
  interrupted by fleet realignment. The remaining steps (batched commit/push)
  are valid but the context has changed.

### Critical Path

The critical path to v0.1.0 public launch:

1. Owner decides on local agent-control file commit scope.
2. Owner decides on launch-posting-plan reference cleanup.
3. Owner approves release tag posture (v0.1.0).
4. Batched commit/push of approved changes.
5. Tag v0.1.0 and trigger release pipeline.
6. Verify crates.io publish and binary artifacts.
7. Announce (launch article is drafted).

### What Can Be Cut

- OpenSSF Scorecard / Best Practices Badge — deferred, not blocking launch.
- OCI/MCPB package — deferred.
- Broader client recipes — deferred.
- Relationship/timeline APIs — deferred.
- Live benchmark report hosting — deferred.
- BES spec-authority integration — explicitly paused; separate concern from
  launch.

## Next-Build Plan

The smallest sequence of specs to move Mimir measurably toward green:

### Spec 1: Local Agent-Control Commit Plan

Decide which untracked agent-control files to commit, which to gitignore, and
which to remove. This unblocks the batched commit/push without polluting the
public repo with internal fleet language.

### Spec 2: Pre-1.0 Launch Cleanup Batch

Fix the broken `docs/launch-posting-plan.md` reference, verify all docs links,
run a fresh full engineering gate, and prepare the batched commit. This is the
final cleanup before tagging.

### Spec 3: v0.1.0 Release Tag and Publish

Create the v0.1.0 tag, trigger the release pipeline, verify crates.io
publish, verify binary artifacts for all 5 targets, and execute the launch
announcement plan.

### Stretch Spec: Librarian/Harness Decomposition

Split `mimir-librarian/src/main.rs` (8,455 LOC) and
`mimir-harness/src/lib.rs` (8,212 LOC) into focused modules. This is not
blocking launch but is the highest-impact maintainability improvement for
post-launch development.

## Proposed Issue List

| # | Title | Depends on | Risk | Verification gate | Model routing |
|---|---|---|---|---|---|
| 1 | Local agent-control commit plan | Owner decisions | Medium | `git status` shows only approved files staged | Any frontier model |
| 2 | Pre-1.0 launch cleanup batch | #1 | Low | Full cargo gate pass + docs link check | Any frontier model |
| 3 | v0.1.0 release tag and publish | #2 + owner approval | High | Release pipeline succeeds, crates.io publish verified, binaries downloadable | Codex `gpt-5.5` primary, Claude Opus verification |
| 4 | Librarian main.rs decomposition | None (can parallel after #1) | Medium | All tests pass, no public API change | Any frontier model |
| 5 | Harness lib.rs decomposition | None (can parallel after #1) | Medium | All tests pass, no public API change | Any frontier model |
| 6 | BES spec-authority research design | Owner approval to resume | High | Design doc reviewed by second model | Frontier model primary + different family verification |
| 7 | Test coverage tooling setup | None | Low | Coverage report generated, baseline established | Sonnet or fast model |
| 8 | Benchmark claim evidence | None (can parallel) | Low | Benchmark results recorded with methodology | Any model |

## Owner Decisions Needed

1. **Local agent-control commit scope**: which of the ~40 untracked
   agent-control files should be committed to the public repo, which should
   be gitignored, and which should be removed?

2. **Launch-posting-plan reference**: the file was deleted in PR #16 but
   `docs/README.md` still links to it. Should the reference be removed,
   redirected, or should the file be restored?

3. **Release tag posture**: is the owner ready to approve v0.1.0 tagging and
   crates.io publish? What conditions must be met first?

4. **BES integration pause visibility**: should the public repo acknowledge
   that BES/Mimir spec-authority integration is paused, or should this remain
   internal?

5. **CI budget for post-evaluation work**: how many PR cycles can Mimir
   consume from the monthly GitHub Actions budget for cleanup and release?

6. **Librarian/harness decomposition priority**: should the large file
   decomposition happen before or after v0.1.0 launch?

## Residual Risks

1. **No fresh cargo gate run**: all engineering gate evidence is from
   2026-04-28. Any changes since then (even agent-control file additions)
   could affect the build. The verifier should note this gap.

2. **Large file complexity**: the two 8k+ LOC files are technical debt that
   will compound. Post-launch development velocity will suffer without
   decomposition.

3. **LLM integration boundary**: the librarian's LLM integration
   (`llm.rs` + `validator.rs`) is a correctness boundary where adversarial
   or malformed LLM output could corrupt the memory store. Testing depth at
   this boundary is unknown.

4. **rmcp version pin**: `=1.5.0` is a tight pin on an early-stage library.
   MCP ecosystem changes may require updates that affect the server's API
   surface.

5. **Public OSS exposure**: once published, any internal agent-control
   language accidentally committed becomes public. The commit scope decision
   (#1 above) is critical.

6. **Single-maintainer risk**: the repo appears to have one maintainer
   (HasNoBeef). Bus factor is 1.

7. **No code coverage baseline**: without coverage tooling, it is impossible
   to quantify which paths are tested and which are not.

## Evidence Gaps

1. **Test content not read**: test files were counted and measured by LOC but
   their content was not read. Coverage quality, assertion depth, and edge
   case handling are unknown.

2. **Librarian main.rs and harness lib.rs not read**: the two largest files
   were not read due to context constraints. Internal structure, duplication,
   and specific complexity risks are inferred from size only.

3. **LLM integration not read**: `llm.rs`, `validator.rs`, `quorum.rs`,
   and `drafts.rs` in the librarian were not read. The correctness of LLM
   output validation is unknown.

4. **CI workflow not fully read**: only the first 80 lines of `ci.yml` were
   read. Release workflow, Dependabot config, and other workflow files were
   not read.

5. **Git history not available**: sandbox restrictions prevented git log, git
   diff, and git status commands against the Mimir repo. All git state is
   from captured handoff spec evidence dated 2026-04-29.

6. **No live cargo gate**: build, test, clippy, fmt, deny, doc, and
   publish-dry-run were not executed. All pass evidence is second-hand from
   `docs/launch-readiness.md` (2026-04-28).

7. **CHANGELOG.md partially read**: only the first 100 lines (Unreleased
   section header and early entries). Full change history since last release
   is not captured.

8. **Adapter coverage unknown**: `copilot_session_store.rs` (1,489 LOC) is
   the only visible adapter. Whether other agent adapters exist or are
   planned is unclear from the files read.
