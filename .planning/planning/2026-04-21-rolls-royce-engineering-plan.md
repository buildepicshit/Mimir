# Rolls Royce engineering plan — the full pre-pilot delta

> **Document type:** Planning — the canonical engineering scope Mimir must complete before any recovery pilot is a meaningful measurement.
> **Last updated:** 2026-04-24
> **Status:** Locked. Commits to a per-category build discipline. Supersedes [`2026-04-20-delivery-plan.md`](2026-04-20-delivery-plan.md) § Pre-flip deliverables.
> **Cross-links:** [`2026-04-20-mission-scope-and-recovery-benchmark.md`](2026-04-20-mission-scope-and-recovery-benchmark.md) · [`2026-04-19-roadmap-to-prime-time.md`](2026-04-19-roadmap-to-prime-time.md) · [`../../AGENTS.md`](../../AGENTS.md)
>
> **2026-04-27 CI update:** The owner added more GitHub Actions usage and approved re-enabling Actions for `buildepicshit/Mimir`. Category 10's infrastructure concern is no longer "Actions are disabled right now"; it is ongoing discipline: verify locally first, batch pushes, and avoid empty/speculative reruns.

## Why this exists

Two weeks of prior work converged on a clear course correction:

- **Prior delivery plans treated testing as the gate**, then constructed pilots whose fairness had known holes (hand-curated corpora; missing client-integration layer; mismatched environments; no consistent rehydration discipline). Results were muddled even when the underlying system worked.
- **This plan treats the engineering harness as the gate**, and tests as the output of the harness. No pilot runs until the infrastructure that makes it an apples-to-apples comparison exists. When it does, running a pilot becomes a one-command invocation that produces a scored, reproducible result.
- **The bar is Rolls Royce, not MVP.** No dead systems. No broken code. No half-wired prototypes left on main. Each category is built to production standard — observable, idempotent, testable, documented — before the next category starts.

**Skunkworks pace applies.** There is no external clock. Phase 5 public flip is a phase ordering, not a deadline. Quality over speed on every category; iterate over ship when the evidence isn't there yet.

## The 10 critical-path categories

Each category is a self-contained workstream with defined acceptance criteria. The dependency DAG is at the end; the numbering reflects the proposed build order.

---

### Category 1 — Production-grade librarian

**What it is.** The `librarian` process that ingests prose drafts, sanitises them (separates observations from directives), structures them into canonical Mimir Lisp, and commits them to the canonical log. The retired Python prototype was a three-iteration proof-of-concept — it hit 100% parse on real drafts but could not commit arbitrary batches because it did not yet handle binder / semantic / emit constraints at commit time.

> 2026-04-24 status note: the Rust `mimir-librarian` crate has replaced the Python prototype, which is no longer shipped in the public tree. The librarian and MCP write surface now share `mimir_core::WorkspaceWriteLock`, an atomic `<canonical-log>.lock` guard that prevents concurrent canonical-log writers without adding an MCP dependency to the librarian crate.

**Current state.**
- Retired prototype findings record a `claude -p` harness with a 200-line system prompt covering parse syntax + three static binder/semantic constraints (source×memory-kind, symbol-kind first-use, confidence×source bound).
- 100% parse rate on real auto-memory content (9 drafts → 90 records, per iteration 3).
- Single-record and small-batch commits proven end-to-end through MCP.
- Known gap: `(s, p, valid_at)` uniqueness collisions reject batches; ~5 other cross-record constraints surfaced only on commit.

**What "done" means (acceptance criteria).**

- **Pre-emit validator with bounded retry.** Every candidate record is validated against a scratch `mimir-mcp` before batch commit. Validation errors are classified (parse / bind / semantic / emit / uniqueness), fed back to the librarian as typed retry hints, and the librarian re-emits the specific record. Bounded retries (default 3); on exhaustion the record is logged-and-skipped with operator-visible reporting.
- **Supersession-aware emission.** `(s, p, valid_at)` collisions do not reject — they emit a proper `:supersedes` edge. The bi-temporal model is used as designed, not as an error.
- **Dedup against existing log.** Before committing a new record, the librarian queries for near-matches on the same `(s, p)` and within a configurable time window; duplicates collapse to a single record with updated `:observed_at`.
- **Idempotent + resumable.** Re-running over an already-processed draft is a no-op. Crash mid-batch does not corrupt log or draft state; recovery is automatic on next run.
- **Scheduled operation.** Continuous mode (systemd timer / cron / filesystem watcher). Configurable cadence. One-shot CLI mode preserved for ad-hoc use.
- **Observability.** Structured tracing per `docs/observability.md`: per-draft span, per-record event, error classification. Per-run metrics: drafts processed, records emitted, commit rate, retry rate, skip rate. All surfaced via a command like `mimir-librarian status`.
- **Lock discipline.** Two librarian instances cannot step on each other's drafts or commits. Shared workspace lock coordination against the Mimir log; draft-level locks for in-flight processing.
- **Error taxonomy.** Typed error classes (`LibrarianError::ValidationFailed`, `::LeaseContest`, `::EmitRejected`, etc.) with documented operator-escalation rules.
- **Tests.** Unit tests for retry logic, idempotency, and lock behaviour. Integration tests: prose draft in → canonical log out, end-to-end.
- **Moved to a real crate** (`mimir-librarian`) in the workspace once the above is met. Python prototype is retired.

**Effort estimate.** 1–2 weeks of focused engineering.

---

### Category 2 — Drafts surface

**What it is.** The defined input boundary: where prose memory drafts come from, how the librarian discovers them, how they flow through the system, and how their provenance is preserved.

**Current state.**
- Prototype read from a hand-written JSONL file with 9 drafts extracted from Claude auto-memory.
- No integration with Claude's auto-memory as a live source.
- No explicit submit-via-MCP or submit-via-CLI path.
- No retention / lifecycle management.

**What "done" means.**

- **Draft input is one or more of:** (a) a sweep over a configured directory (e.g. `~/.mimir/drafts/`), (b) direct Claude-auto-memory ingestion (read the operator's `~/.claude/projects/.../memory/` and treat each file as a draft), (c) explicit submission via MCP tool (`mimir_submit_draft`) or CLI (`mimir-cli submit`).
- **Provenance.** Each committed record carries provenance: which draft file (or submission event) it came from, which librarian run processed it. Traceable via a `provenance_of(record_id)` query.
- **Lifecycle.** Drafts move through typed states: `pending → processing → committed | skipped | failed`. State is visible, queryable, and retained per policy (e.g. 30 days for successful drafts, indefinitely for failed).
- **Schema.** Minimal — just `{id, source, submitted_at, prose}` with optional `{context_tags}` — but schema-locked and version-tagged.
- **Retention.** Failed drafts are retained with their classification so operator can review. Successful drafts optionally archived or deleted after N days (configurable).
- **Tests.** Draft flows through each lifecycle state; sweep-dir and submit-MCP both covered; provenance is queryable and correct.

**Effort estimate.** 3–5 days.

---

### Category 3 — Client integration (Mode 1 real distribution)

**What it is.** The thing agents actually use. A Claude Skill bundle (and / or CLAUDE.md conventions + hooks) that encodes the **write-trigger hooks** (drop prose drafts in the right surface at the right moments) and the **cold-start rehydration Skill** (category 4). Without this, Mimir is a library nobody uses.

**Current state.**
- No Skill exists.
- No CLAUDE.md convention documented.
- No post-tool-use / post-task hook integration.
- The operator currently writes memories via Claude Code's native `save memory` flow; those end up in auto-memory, not in Mimir.

**What "done" means.**

- **Claude Skill bundle** installable via standard Skill mechanism. Packages:
  - The cold-start rehydration Skill (see category 4).
  - The write-trigger hooks (explicit save, user affirmation, post-task-summary — events that drop a prose draft in the drafts surface).
  - Minimal configuration surface (path to Mimir log, librarian cadence, rehydration depth).
- **CLAUDE.md recipe** documented: a canonical block an operator drops into their CLAUDE.md to opt in.
- **Auto-memory sweep mode** as a fallback: even without explicit hooks, the librarian can sweep Claude's auto-memory directory on a schedule and ingest what's there. This keeps content synced even if an operator forgets to install the Skill.
- **Works for operators who are not you.** Documented install path; tested on a clean machine; distributable.
- **Uninstall path.** Removing the Skill is clean — no lingering state, no orphaned processes.
- **Tests.** End-to-end: operator installs Skill → writes a memory in a normal conversation → librarian picks it up → record lands in Mimir log → cold-start Skill retrieves it.

**Effort estimate.** 1 week.

---

### Category 4 — Cold-start rehydration Skill

**What it is.** The backend-agnostic query discipline a cold-start agent runs on session start to rehydrate working context. Same Skill works against auto-memory (filesystem), Mimir (MCP), or both. Distinct from category 3 (which packages it); this category is the protocol itself.

**Current state.**
- No coded rehydration discipline exists.
- In recovery pilot 01, the cold-start Claude queried Mimir MCP with a single broad `mimir_read`, summarised, and produced a response that surfaced 7 of 20 loaded records (~35%). The 13 missed records were in the log, queryable, and simply not asked for.

**What "done" means.**

- **Structured query protocol.** Sequential queries for: operator profile → project state → open work → open decisions → recent feedback → applicable `pro` rules for current scope. Each stage has a specified query shape and a specified summarisation output. Not improvised per session.
- **Backend-agnostic.** The Skill determines what it has access to (auto-memory files / Mimir MCP / both) and runs the same protocol, sourcing from whichever backend is available. Identical rehydration discipline regardless of backend.
- **Context-budget aware.** The Skill knows the agent's context window budget; rehydrates within a configurable fraction (default 10%), summarising rather than inlining when the raw payload is too large.
- **Self-priming preamble.** After rehydration, the agent emits a "here's what I recovered, from where, and what gaps I noticed" preamble before the operator's first real task. Operator can sanity-check immediately.
- **Failure modes documented.** What happens if Mimir MCP is unavailable, if auto-memory is empty, if both sources disagree on a fact. Deterministic behaviour, documented precedence rules.
- **Tests.** Rehydration against each backend in isolation; rehydration against both simultaneously; rehydration when one backend is empty; rehydration when both backends conflict on a fact.

**Effort estimate.** 3–5 days.

---

### Category 5 — Render-surface execution

**What it is.** Actually implementing the six tightening items from the [2026-04-20 render-surface audit](2026-04-20-render-surface-audit.md). Agent-native runtime surface is the governing design principle; currently the runtime surface pretty-prints JSON and carries ~200-char narrative error strings.

**Current state.**
- Audit complete: 7 of 9 MCP tools score ≥ 8/10 agent-native; 2 tools and 3 cross-cutting patterns identified for tightening.
- 2026-04-26 status note: MCP runtime responses now use compact JSON, all nine tool descriptions are capped by tests (`mimir_status` ≤50 chars, all tools ≤100), `mimir_verify` exposes `tail_type` + corrupt-tail `tail_error` code instead of narrative `tail_status`, and workspace / lease runtime errors are code-first.

**What "done" means.**

- **Compact JSON on the runtime surface.** `serde_json::to_string()` instead of `to_string_pretty()` for all MCP tool responses. Quick compat audit first (grep tests + known client patterns) to confirm no consumer depends on pretty-printed output.
- **`mimir_verify` tail split.** `tail_status` splits into `tail_type: enum` + `tail_error: Option<String>`. The narrative `DecodeError` prose is only populated on corrupted-tail; happy path carries zero narrative.
- **`mimir_status` description shrink.** Tool-invocation description goes from ~250 chars to ≤ 50 chars; detail moves to input-schema field descriptions.
- **Error codes replace narrative errors.** `no_workspace_open: no workspace is open; call...` → `no_workspace_open` (code only); explanatory prose moves to docs / a single reference in the MCP server instructions.
- **Tool-description compression.** All 9 `#[tool(description = "...")]` descriptions compressed to ≤ 100 chars; syntactic detail moves to input-schema field descriptions.
- **Tests.** Wire-format tests confirming the new shapes; snapshot tests that lock in the compact responses so regressions are caught.
- **Acceptance gate.** Estimated token saving of ~300–500 tokens per session (per the audit); measurable via a simple before/after test.

**Effort estimate.** 2–3 days.

---

### Category 6 — Sanitisation hardening

**What it is.** Production-grade implementation of the sanitisation pillar. Not just splitting observation from directive at write time (category 1 already does that) — also the **render-time** discipline: retrieved records arrive in agent context as visibly data, never re-inlinable as instructions. Plus the adversarial corpus that regression-tests the claim.

**Current state.**
- Librarian iteration 1 surfaced the prompt-injection failure mode (drafts containing imperatives caused the librarian to obey content). Fixed with `<draft>` envelope + hardened system prompt.
- 2026-04-26 status note: transparent-harness rehydration now marks launch-capsule records as `mimir.governed_memory.data.v1` with `instruction_boundary = data_only_never_execute`, adds the matching `agent-guide.md` consumer rule, and documents the first render-time threat boundary in `docs/sanitisation.md`.
- 2026-04-26 status note: librarian draft processing now marks first-attempt and retry prompts as `mimir.raw_draft.data.v1` with `instruction_boundary = data_only_never_execute`, and a committed fixture-backed adversarial corpus exercises the write path through the real retry, validator, and store commit flow.
- 2026-04-26 status note: MCP read surfaces now mark `mimir_read` and `mimir_render_memory` payloads as governed memory data instead of returning bare Lisp strings.
- Remaining gap: any future retrieval adapter still needs equivalent render-boundary markers and tests.

**What "done" means.**

- **Render-boundary markers.** Retrieved records carry explicit data-surface markers (structural framing or a designated wrapper) that distinguish them from instructions at the token level in the consumer agent's context.
- **Documented threat model.** Short doc enumerating what sanitisation defends against (prose that *looks like* instructions at write time; prose that *executes as* instructions at read time), what it does not defend against, and what consumer agents must do to uphold their end (e.g. the cold-start Skill must render retrieved records inside its documented data-surface markers).
- **Adversarial corpus.** A committed set of prose drafts designed to probe sanitisation: prompt-injection attempts, role-confusion strings, instruction-disguised-as-memory, memory-disguised-as-instruction. Each with an expected post-librarian outcome.
- **Regression tests.** The adversarial corpus runs as part of the librarian's test suite. Any future change to the librarian prompt must not regress these tests.
- **Render-time tests.** The cold-start Skill (category 4) renders retrieved records correctly inside the data-surface markers — tested.

**Effort estimate.** 3–5 days.

---

### Category 7 — BC/DR plumbing

**What it is.** The *actual* business-continuity / disaster-recovery path. Mimir's primary value proposition is "survive catastrophic local loss." That requires more than a durable log format — it requires an automated backup path, a documented-and-drilled restore procedure, and integrity checks that catch corruption rather than just failing to decode.

**Current state.**
- Log format is append-only with a `MIMR` magic header; per-record checksum / hash-chain integrity remains deferred.
- `mimir-cli verify` exists and detects corruption / orphan tails.
- 2026-04-26 status note: Git-backed remote mirroring can publish and restore workspace `canonical.log` plus draft JSON files; `mimir remote push` and `mimir remote pull` verify canonical-log integrity before and after sync; `[remote] auto_push_after_capture = true` runs the same verified push path after wrapped-session capture and librarian handoff; `mimir remote drill --destructive` and `scripts/bcdr-drill.sh` now delete the local log, restore from the configured Git recovery remote, verify integrity, reopen the store, and run `(query :limit 1)` as a sanity query. `Store::open` truncates recoverable orphan/torn tails but refuses non-recoverable corrupt tails without truncating, preserving bytes for verify/restore. `docs/bc-dr-restore.md` documents backup, fresh-machine restore, corrupted-local-log handling, auto-push, and the drill.
- Remaining gap: no timer or generic on-every-commit automation outside wrapped-session capture.
- Remaining gap: log-integrity hardening still needs a designed per-record checksum or hash-chain plus the remaining concurrent-read-during-write matrix.

**What "done" means.**

- **Automated backup.** Configurable backup target (local mirror, rsync target, S3 bucket, syncthing folder — operator choice). Backup runs on a schedule and/or on every commit (configurable). Backup integrity is verified post-copy.
- **Restore procedure.** Documented, step-by-step, in `docs/bc-dr-restore.md`. Produces a working Mimir workspace from a backup + an empty local environment. Handles the case where the backup is intact but the local log is corrupted.
- **Restore drill.** Scripted: `./scripts/bcdr-drill.sh` deletes the local log, restores from backup, verifies integrity, runs a sanity query. Green = drill passed.
- **Corruption-recovery semantics.** Where possible, the log recovers from certain corruption classes (orphan-tail truncation already handles some; document the class). Where not possible, the backup is the fallback.
- **Log-integrity hardening.** Per-record checksum or hash chain verified on every `Store::open`. Tests for torn-write recovery, concurrent-read-during-write, and log-truncation mid-record.
- **Documentation.** The "how to back up" and "how to restore" stories are prominent in README + `docs/bc-dr-restore.md`. No operator should have to guess.

**Effort estimate.** 3–5 days.

---

### Category 8 — Benchmark harness

**What it is.** The scripted test runner that turns a pilot from "half a day of manual setup" into a one-command invocation. Scenarios are data, environments are built by the harness, metrics are aggregated automatically where possible and operator-graded where not.

**Current state.**
- Methodology + scoring rubric exist (`benchmarks/recovery/README.md`, `SCORING.md`).
- One worked example scenario (scenario 01, self-referential), now mirrored into structured JSON.
- Scenario JSON is validated at load time for expected baselines, typed ground truth, typed staleness probes, and unique probe IDs.
- `./bench recovery --list` and `./bench recovery --scenario <id> --dry-run` render deterministic scenario/run-plan data without launching agents.
- `./bench recovery --scenario <id> --init-results` materializes non-clobbering transcript placeholders, `scorecard.md`, `scores.json`, `notes.md`, and `run-plan.json` from scenario data.
- `./bench recovery --scenario <id> --prepare-envs` materializes non-clobbering per-baseline environment scaffolds and manifests without launching agents.
- `./bench recovery --scenario <id> --validate-envs` validates per-baseline materialized-input contracts, verifies declared input paths, and reports ready vs blocked baselines.
- `./bench recovery --scenario <id> --launch-plan` emits non-executing launch contracts for validated baselines without launching agents.
- `./bench recovery --scenario <id> --write-launch-contracts` materializes all per-baseline launch contracts only after every baseline environment validates as ready.
- `./bench recovery --scenario <id> --validate-launch-contracts` validates materialized launch contracts and prompt files against the current generated contracts before any execution step.
- `./bench recovery --scenario <id> --execute-launch-contracts --approve-live-execution <id>` executes only validated contracts after explicit operator approval, captures transcripts automatically, writes `live-run.json`, and fills null mechanical score fields without overwriting operator grades.
- `./bench recovery --scenario <id> --validate-transcripts` checks per-baseline transcript files for missing, placeholder, or prompt-mismatched captures before scoring.
- `./bench recovery --scenario <id> --score-results` validates filled structured scores, cutoff/staleness/decision-denominator/integer-type bounds, and transcript evidence, then emits per-baseline metric summaries.
- `./bench recovery --summary-results` scans every scenario fixture and reports missing, complete, or incomplete score/evidence sets plus threshold verdicts.
- Live recovery pilots remain blocked only on operator-populated environments and the explicit per-scenario live-execution approval token.

**What "done" means.**

- **Scenarios as data.** YAML or JSON per-scenario definitions: prose corpus, ground-truth checklist, cold-start prompt, cutoff threshold, scoring notes. Not prose docs.
- **Environment automation.** The harness sets up matched environments (clean / md-files-only / mimir-only / both) automatically: builds the cleanroom, populates the relevant backend(s) from the same source corpus via the librarian, launches the cold-start agent with the right flags, captures transcripts.
- **Per-metric scoring.** Automated where mechanical (time-to-productive, token cost, hallucination-on-named-decisions); operator-graded where nuanced (overall fact correctness, staleness). Scorecards generated, aggregate report produced.
- **Reproducibility.** `./bench recovery` runs the full benchmark. Corpus and scenarios are version-locked; reruns are comparable.
- **Self-tests.** The harness has its own tests so it doesn't silently break when corpus schema changes.

**Effort estimate.** 1 week.

---

### Category 9 — Corpus + ground truth

**What it is.** The real prose content the benchmark is measured against. Not 10 synthetic drafts; not 9 hand-picked from auto-memory. A proper corpus representative of what an operator's Mimir would actually hold, with scenario-specific ground-truth checklists.

**Current state.**
- 9 drafts extracted from the operator's auto-memory (iteration 2).
- 10 synthetic drafts (iteration 1, mixed shapes).
- One worked-example scenario with 20 ground-truth items (scenario 01), now mirrored into machine-readable JSON and guarded by `cargo test -p mimir-harness --test recovery_benchmark`.
- No realistic-scale corpus. Corpus lockdown discipline has started for scenario JSON shape, but the production corpus/scenario content is still missing.

**What "done" means.**

- **Corpus target.** ≥100 prose drafts representing real operator memory content. Drawn from: current auto-memory (starting baseline), augmented with additional scenario-relevant drafts written by the operator.
- **Scenario set.** 3–5 catastrophic-loss scenarios per the recovery-benchmark methodology, each with a defined ground-truth checklist (N items typed as operator-profile / project-state / decisions / feedback / open-work).
- **Corpus lockdown.** Same discipline as the earlier parse-rate corpus — committed, version-locked, any change reviewed as a corpus-integrity event.
- **Ground-truth checklists as data.** Structured format consumable by the harness scorer; per-item classification so automated scoring can handle what's scoreable.
- **Distinct from training.** The librarian's few-shot examples are not drawn from the corpus; the cold-start Skill's priming does not leak ground-truth items. Corpus integrity for measurement.

**Effort estimate.** 3–5 days (operator-effort-heavy; technical effort is minimal).

---

### Category 10 — Test infrastructure

**What it is.** The testing scaffolding that makes "no dead systems, no broken code" enforceable rather than aspirational. Covers what's currently tested (Rust workspace: 463 unit + integration + property + doctest + fuzz) plus what needs to be added across the new surfaces (librarian, Skill, harness, BC/DR drill).

**Current state.**
- Rust workspace: 463 tests passing. Strong foundation.
- Python librarian: no unit tests (prototype, only manual runs).
- Skill + cold-start: does not exist yet, so not tested.
- Harness: does not exist yet.
- BC/DR drill: does not exist.

**What "done" means.**

- **Librarian unit tests.** Retry logic, classification, idempotency, lock behaviour, supersession-emission correctness, dedup.
- **Librarian integration tests.** Prose draft → canonical log → MCP query → expected record shape. End-to-end, exercised per category-1 acceptance.
- **Skill / cold-start tests.** Rehydration protocol runs correctly against each backend; context-budget respected; self-priming preamble is emitted; failure modes behave as documented.
- **Harness self-tests.** The benchmark doesn't silently break when corpus schema changes; scorecards aggregate correctly; deterministic replays.
- **Adversarial regression tests.** The sanitisation corpus (category 6) runs in CI.
- **BC/DR drill tests.** The restore drill is automated, runs in CI (or at minimum is executable from a single command by the operator).
- **CI infrastructure decision.** Paid GitHub Actions usage was added on 2026-04-27 and Actions may be enabled with owner approval. Category 10 still includes keeping CI economical and reliable; if paid minutes become insufficient again, choose between self-hosted runner capacity and more paid minutes before relying on CI as the public gate.

**Effort estimate.** 3–5 days spread across categories (tests ship with the thing they test) + ~1 day for CI infrastructure.

---

## Explicitly deferred (not in the critical path for first pilot)

These categories belong to the full Rolls Royce Mimir but are not required for the first apples-to-apples recovery pilot. They gate other milestones (public flip, broader use) but not category 8's first run.

- **Memory graduation mechanics.** The specific → broad transformation (see [scope doc § 5](2026-04-20-mission-scope-and-recovery-benchmark.md)). Open design questions: confirmation threshold, de-identification step, broadcast flag, projection rules. Design spike separately; implementation after first recovery pilot.
- **Cross-agent sharing.** Multi-workspace + graduated-memory broadcast. Builds on graduation mechanics. Post first-pilot.
- **Public-flip readiness.** README repositioning, CONTRIBUTING refresh, SECURITY.md review, `v0.1.0-alpha.1` cut, crates.io publish, marketplace listings. Former Deliverable D. Post first-pilot-that-succeeds.

## Dependency DAG

```
    ┌─────────────┐
    │ 1. Librarian│
    │ (production)│
    └──────┬──────┘
           │
           ▼
    ┌─────────────┐    ┌────────────────┐    ┌───────────────┐
    │ 2. Drafts   │    │ 5. Render exec │    │ 6. Sanitisation│
    │    surface  │    │  (independent) │    │   hardening    │
    └──────┬──────┘    └────────┬───────┘    └────────┬──────┘
           │                    │                     │
           ▼                    │                     │
    ┌─────────────┐             │                     │
    │ 3. Client   │             │                     │
    │ integration │             │                     │
    └──────┬──────┘             │                     │
           │                    │                     │
           ▼                    │                     │
    ┌─────────────┐             │                     │
    │ 4. Cold-    │             │                     │
    │ start Skill │             │                     │
    └──────┬──────┘             │                     │
           │                    │                     │
           ├────────────────────┴─────────────────────┤
           ▼                                          ▼
    ┌─────────────┐                           ┌────────────────┐
    │ 7. BC/DR    │                           │ 9. Corpus +    │
    │  plumbing   │                           │  ground truth  │
    └──────┬──────┘                           └────────┬───────┘
           │                                           │
           └─────────────┬─────────────────────────────┘
                         ▼
                  ┌──────────────┐
                  │ 8. Benchmark │
                  │   harness    │
                  └──────┬───────┘
                         │
                         ▼
                  ┌──────────────┐
                  │ First real   │
                  │   pilot      │
                  └──────────────┘

Category 10 (test infrastructure) runs horizontally — tests ship with each
category they test; CI infrastructure lands once, covers all.
```

**Hard rule:** No pilot runs until category 8 exists and is wired to the corpus from category 9. All other pilots, smokes, or "let's just quickly try" tests are declined until then.

## Proposed sequence

1. **Category 1** — production-grade librarian
2. **Category 2** — drafts surface
3. **Category 3** — client integration
4. **Category 4** — cold-start rehydration
5. **Category 5** + **Category 6** — render-surface execution + sanitisation hardening (in parallel, small and independent)
6. **Category 7** — BC/DR plumbing
7. **Category 9** — corpus + ground truth (operator-driven, can start in parallel with category 6/7)
8. **Category 8** — benchmark harness
9. **First real pilot** — run the harness, get a scored result
10. **Category 10** tests run throughout; CI infrastructure lands early (prerequisite for the librarian's CI tests)

**Total honest estimate:** 4–8 weeks of focused work by a single operator. Not days. Parallelism exists in (5 + 6), and (9) can run while (6/7) land, but the critical path is serial.

## Per-category build discipline

For each category, in order:

1. **Conversation first.** Before any code is written, a design conversation: what's the shape, what are the open questions, what does "done" look like for this specific category, what trade-offs exist.
2. **No uninformed guesses.** Where a design question can be resolved by reading the existing architecture specs (`docs/concepts/`), the answer is found there first. Where not, the question is surfaced explicitly and answered before coding.
3. **Build to the acceptance criteria above.** Not to "we can probably test now." To the full list.
4. **Tests ship with the thing.** No "we'll add tests later." No untested code reaches main.
5. **Observable.** Every new component emits structured traces per `docs/observability.md`. Debugging is a first-class feature.
6. **No dead systems.** If a category replaces an earlier prototype, the prototype is retired in the same PR. No parallel rotting code paths.
7. **Documented.** Every category lands with documentation updated — specs, READMEs, CLAUDE.md if relevant.
8. **Review gate.** Category PR merges only when the acceptance criteria are met. Partial landings accepted only if explicitly scoped and next steps named.

## What does NOT change

- The architectural invariants in [`AGENTS.md`](../../AGENTS.md) remain canonical.
- The three-pillar mission (BC/DR + memory graduation + sanitized cross-agent sharing) stands.
- The engagement protocol (Propose → Wait → Execute → Report → Stop) stands — each category's build cycle follows it.
- The CI quota discipline stands (verify locally before push; no speculative pushes or empty retry commits).
- No AI attribution in any commit, PR, or project output.
- The canonical log format + MCP wire surface do not change *except* through the category-5 render-surface tightening, which is wire-change-aware and gated by the pre-v0.1.0-alpha.1 window per the audit's decision log.
