# `mimir-librarian`

The librarian. Ingests prose memory drafts; sanitises them (separates observations from directives); structures them into canonical Mimir Lisp; commits them to the canonical log via the in-process `mimir_core::Pipeline`.

> **Pre-1.0 status.** This crate is part of Mimir's active-development tree. Draft envelopes, CLI flags, quorum artifacts, and processing behavior may change before v1. Public crates.io releases wait for the first alpha.

Current release and launch readiness status lives in [`STATUS.md`](../../STATUS.md) and [`docs/launch-readiness.md`](../../docs/launch-readiness.md).

## Status

**Draft-ingestion foundation.** The scope-aware draft boundary is wired: `mimir-librarian submit` writes v2 JSON draft envelopes into `pending/` with source surface, agent, project, operator, provenance, and tags, including the dedicated `consensus_quorum` surface for governed quorum evidence and `copilot_session_store` for Copilot checkpoint drafts. `mimir-librarian sweep` stages explicit file/directory Codex, Claude, repo-handoff, directory, or agent-export memories into the same draft store; directory sweeps recurse only through `.md`, `.markdown`, and `.txt` files. `DraftStore` owns the rename-based lifecycle API for `pending -> processing -> accepted | skipped | failed | quarantined`, including claim-age stale `processing` recovery. `mimir-librarian copilot schema-check|recent|files|checkpoints|search|submit-drafts` opens Copilot CLI's SQLite session store read-only, validates schema before every query, scopes to the current repository when detectable, fails safely on missing/locked/drifted stores, and submits only untrusted checkpoint drafts through the normal draft store. `mimir-librarian run` executes the lifecycle-safe one-shot runner with the bounded LLM validation retry processor by default, and `mimir-librarian watch` repeats that same path on a polling cadence. `--archive-raw` adds a deterministic no-LLM drainage path that commits raw drafts as low-confidence `pending_verification` evidence plus librarian-assigned provenance/data-boundary facts; `--defer` keeps the lifecycle-only dry run available. `PreEmitValidator` validates candidate canonical Lisp in-process against a scratch `mimir_core::Pipeline` with exact rollback on rejection, then accepted batches commit through `mimir_core::Store::commit_batch` while holding the shared `mimir_core::WorkspaceWriteLock`. Exact duplicate records across all four memory types are filtered before commit, and otherwise-identical Semantic / Inferential records inside the configurable same-day `valid_at` window skip as duplicates; deterministic supersession conflicts skip by default or write a review artifact and quarantine under `--review-conflicts`. Librarian run and processor observability are wired without logging raw draft prose, paths, LLM text, retry prompts, validation errors, or canonical Lisp payloads. Remaining Category 1 work still lands one concern at a time per the build discipline in the Rolls Royce plan.

## Architecture

- `crate::LlmInvoker` — trait over "ask Claude to structure this prose as canonical Lisp." Default impl `ClaudeCliInvoker` shells out to `claude -p` non-interactively. Tests mock via any `LlmInvoker` implementation.
- `crate::PreEmitValidator` — validates candidate canonical Lisp against a scratch `mimir_core::Pipeline`. In-process; no subprocess, no IPC. Clone-on-write rollback is exact for "try this record; on failure, no state mutated."
- `crate::RetryingDraftProcessor` — calls the LLM, parses the JSON output contract, validates candidate Lisp transactionally, filters duplicates across all four memory types, commits accepted batches durably, re-prompts with structured retry hints for JSON / parse / bind / semantic / emit failures, and applies deterministic supersession-conflict policy.
- `crate::RawArchiveDraftProcessor` — deterministic no-LLM processor that archives each raw draft as governed `pending_verification` evidence plus provenance/data-boundary facts through the same append-only store and writer lock. It is for fast drainage, not semantic distillation.
- `crate::DedupPolicy` — deterministic duplicate policy. Default is a one-day `valid_at` window for otherwise-identical Semantic and Inferential records; exact-only mode is available with `DedupPolicy::exact()` or `--dedup-valid-at-window-secs 0`.
- `crate::SupersessionConflictPolicy` — skip-with-warning default or review-artifact + quarantine mode for equal-key supersession collisions.
- `crate::Draft` / `crate::DraftMetadata` / `crate::DraftStore` — scope-aware v2 draft envelopes, filesystem storage, atomic state transitions, and stale-processing recovery.
- `crate::DraftId` / `crate::DraftState` / `crate::DraftTransition` — provenance-aware content IDs and lifecycle directories (`pending -> processing -> accepted | skipped | failed | quarantined`).
- `crate::QuorumEpisode` / `crate::QuorumParticipantOutput` / `crate::QuorumAdapterRequest` / `crate::QuorumResult` / `crate::QuorumStore` — typed consensus-quorum episode/output/adapter-request/result envelopes with file-backed create/load, append/load participant output, visibility-gated round reads, adapter request generation, native Claude/Codex adapter-plan materialization, bounded adapter-run and multi-round status capture, validated status-to-output append, result storage, synthesis adapter planning/running/status-backed acceptance with result validation, replayable/resumable pilot-plan/status/run artifact paths, manifest-level required proposed-draft exit criteria, non-participant pilot-review certification artifacts, pilot-summary operator snapshots, and proposed-draft submission through the governed draft store.
- `crate::DraftProcessor` / `crate::run_once` — one-shot processing skeleton: recover stale `processing/`, claim pending drafts, invoke an injected processor, and move drafts to terminal states or safely back to `pending/`.
- `crate::LibrarianConfig` — paths, retry budget, timeouts, dedup window, review-conflicts toggle.
- `crate::LibrarianError` — typed error taxonomy; every externally-observable failure mode has a variant.
- `mimir-librarian` binary — CLI with `submit`, `sweep`, lifecycle-safe `run`, polling `watch`, file-backed `quorum create|pilot-plan|pilot-status|pilot-run|pilot-review|pilot-summary|append-output|append-status-output|outputs|visible|adapter-request|adapter-plan|adapter-run|adapter-run-round|adapter-run-rounds|synthesize-plan|synthesize-run|accept-synthesis|synthesize|submit-drafts`, and read-only `copilot schema-check|recent|files|checkpoints|search|submit-drafts` wired. `run` and `watch` validate LLM output with bounded retry by default and support `--archive-raw`, `--review-conflicts`, `--dedup-valid-at-window-secs`, and `--defer` for lifecycle dry-runs; `quorum` records auditable artifacts, writes, checks, and executes a replayable live-pilot manifest through the existing gates, reports partial failed runs, skips already-complete gates on rerun, enforces optional minimum proposed-draft counts before certification, records non-participant pilot reviews, summarizes accepted result/review/draft state for operators, emits adapter request JSON, materializes and executes bounded native Claude/Codex prompt/command plans for one participant, one full round, a gated multi-round sequence, or synthesis, validates successful status artifacts before recording participant outputs through the existing append path, validates proposed synthesis status/result artifacts before saving them, and submits proposed memory drafts to the normal draft store, but adapter execution still does not append participant outputs or write memory directly. `copilot` surfaces native session recall as untrusted JSON and submits checkpoint drafts only through `DraftStore`.

## Architectural decisions (2026-04-21 Category 1 conversation)

Confirmed in the Category 1 design conversation; each of these shapes the skeleton:

- **Language:** Rust crate, not Python. Matches workspace conventions, typed errors, observability. The earlier Python prototype is retired and is no longer shipped in the public tree.
- **LLM invocation:** `claude -p` subprocess from Rust, wrapped behind a `LlmInvoker` trait so tests can mock. No direct Anthropic SDK dependency; `--bare` not used (OAuth via the operator's existing `claude` auth).
- **Pre-emit validation:** in-process via `mimir_core::Pipeline::compile_batch` on a scratch `Pipeline` instance. Zero subprocess overhead; full typed errors.
- **Supersession-conflict resolution:** skip-with-warning by default (D.1); operator-review mode via `--review-conflicts` flag (D.3). "Adjust valid_at" (D.2) was rejected as intent-shaping-by-stealth.
- **Lease coordination:** librarian still opens `Store` directly (E.2) and does not depend on `mimir-mcp`, but direct-open now acquires the shared `mimir_core::WorkspaceWriteLock` before `Store::open`. MCP and librarian therefore share the same `<canonical-log>.lock` writer-exclusion boundary without routing librarian commits through MCP. Revisit MCP-client mode (E.1) only if future deployments need remote lease brokering rather than local lockfile coordination.

*(Note: earlier proposal leaned E.1 — MCP client. The operator's "sure lets go" approval locked in E.2 as the shipped choice for the skeleton; the shared core lock keeps that shape without leaving multi-process writer exclusion to operator discipline.)*

- **CI integration tests:** unit tests mock `LlmInvoker`. Integration tests that exercise the full prose → log flow skip the LLM leg unless the `claude` CLI is on `PATH` (gated via `#[ignore]` + a feature flag; real coverage lands when the self-hosted CI runner does, per Category 10).
- **Prompt storage:** system prompt lives in `src/prompts/system_prompt.md` and is loaded via `include_str!()`. Versionable, diffable, reviewable in isolation.
- **Drafts surface:** state-directory filesystem flow under a configurable `drafts_dir` (default `~/.mimir/drafts/`). Draft IDs are 8-byte SHA-256 hashes over raw text plus stable provenance fields (hex-encoded, 16 chars), so repeated sweeps are idempotent without collapsing identical text from different sources.

## Roadmap within Category 1

The full acceptance criteria for Category 1 ship across follow-up PRs. Each is a focused, tested concern:

- [x] PR: skeleton — this PR. Types, traits, CLI entry, tests compile.
- [x] PR: scope-aware draft submit — v2 draft schema, provenance metadata, quarantine/skipped lifecycle states, idempotent pending-file submission, `mimir-librarian submit`.
- [x] PR: `LlmInvoker::invoke` with real `claude -p` subprocess handling (timeout, stderr capture, typed errors on non-zero exit or non-JSON response).
- [x] PR: explicit memory sweeps — `mimir-librarian sweep --path ...` stages configured Codex/Claude/repo-handoff/directory files into the same pending draft store without ambient filesystem search.
- [x] PR: draft processing state transitions — atomic renames, crash-safe claim-age stale `processing` recovery, typed transition errors, terminal state movement.
- [x] PR: `run` processing skeleton — recover stale `processing`, claim pending drafts, invoke injected processor, emit JSON summary, and defer safely until real processing is wired.
- [x] PR: `PreEmitValidator` wired to `mimir_core::Pipeline`. Given a candidate record, returns `Ok(())` or a typed `PipelineError`.
- [x] PR: retry loop — invoke LLM, validate, on failure produce a structured retry hint, re-invoke, bounded N retries.
- [x] PR: durable commit — accepted drafts commit validated batches through `mimir_core::Store::commit_batch`; store-level pipeline rejections feed back into the retry loop without marking drafts accepted.
- [x] PR: supersession-conflict policy — skip-with-warning default + `--review-conflicts` drop-into-queue mode.
- [x] PR: exact duplicate filter — pre-commit skip for already-committed Semantic, Episodic, Procedural, and Inferential records; mixed batches commit only unique records.
- [x] PR: dedup expansion — configurable valid-at window (initially same-day) for otherwise-identical Semantic and Inferential records.
- [x] PR: scheduling — `run` one-shot and polling `watch` modes; systemd timer + cron recipe documented.
- [x] PR: observability — runner and processor spans/events per `docs/observability.md`, with retry/error/record metrics and no user prose in logs.
- [x] PR: retire the Python prototype once this crate reaches feature parity.
- [x] PR: workspace write lock — shared `mimir_core` lockfile guard prevents concurrent `mimir-mcp` and `mimir-librarian` writers from opening the same canonical log.
- [x] PR: Copilot session-store adapter — read-only SQLite recall with schema checks, fixture DB tests, repository scoping where detectable, and optional checkpoint draft submission through `copilot_session_store` provenance.

## Running

Until the first alpha release, build from the repository root:

```bash
cargo install --locked --path crates/mimir-librarian
```

For local development without installing:

```bash
cargo build -p mimir-librarian --release
./target/release/mimir-librarian submit \
  --drafts-dir ~/.mimir/drafts \
  --source-surface codex-memory \
  --agent codex \
  --project buildepicshit/Mimir \
  --operator AlainDor \
  --provenance file:///home/hasnobeef/.codex/memories/mimir.md \
  --text "Mimir should import Codex memory as an untrusted draft."

./target/release/mimir-librarian submit \
  --drafts-dir ~/.mimir/drafts \
  --source-surface consensus-quorum \
  --agent quorum \
  --project buildepicshit/Mimir \
  --operator AlainDor \
  --provenance quorum://episode/2026-04-24T21:00:00Z \
  --tag quorum \
  --tag strong_majority \
  --text "Quorum recommends keeping remote sync explicit; dissent preserved."

./target/release/mimir-librarian sweep \
  --drafts-dir ~/.mimir/drafts \
  --path ~/.codex/memories \
  --source-surface codex-memory \
  --project buildepicshit/Mimir \
  --operator AlainDor \
  --tag codex-sweep

./target/release/mimir-librarian copilot recent \
  --repo buildepicshit/Mimir \
  --limit 5

./target/release/mimir-librarian copilot submit-drafts \
  --drafts-dir ~/.mimir/drafts \
  --repo buildepicshit/Mimir \
  --operator AlainDor \
  --tag copilot-session-store

./target/release/mimir-librarian run --drafts-dir ~/.mimir/drafts --workspace /path/to/canonical.log
```

`run` emits a compact JSON summary. By default it invokes the LLM, validates candidate records with bounded retry, filters duplicate records, commits accepted batches to the canonical log, and skips deterministic supersession conflicts with a warning. The default duplicate policy treats otherwise-identical Semantic and Inferential records inside a one-day `valid_at` window as duplicates; use `--dedup-valid-at-window-secs 0` for exact-only behavior. Use `--archive-raw` to drain drafts without an LLM by archiving each raw draft as low-confidence governed evidence plus provenance records. Use `--review-conflicts` to write conflict artifacts under `drafts/conflicts/` and move those drafts to `quarantined/`. Use `--defer` to exercise lifecycle movement without invoking the LLM; deferred drafts return to `pending/` and the process exits `70`.

`watch` repeats the same run path until stopped:

```bash
./target/release/mimir-librarian watch \
  --drafts-dir ~/.mimir/drafts \
  --workspace ~/.mimir/canonical.log \
  --poll-secs 30
```

For a user systemd timer, keep `run` as the scheduled command:

```ini
# ~/.config/systemd/user/mimir-librarian.service
[Service]
Type=oneshot
ExecStart=%h/.cargo/bin/mimir-librarian run --drafts-dir %h/.mimir/drafts --workspace %h/.mimir/canonical.log
```

```ini
# ~/.config/systemd/user/mimir-librarian.timer
[Timer]
OnBootSec=2min
OnUnitActiveSec=1min
Unit=mimir-librarian.service

[Install]
WantedBy=timers.target
```

Cron equivalent:

```cron
* * * * * $HOME/.cargo/bin/mimir-librarian run --drafts-dir $HOME/.mimir/drafts --workspace $HOME/.mimir/canonical.log
```

## Testing

```bash
cargo test -p mimir-librarian
```

Unit tests mock `LlmInvoker`. Integration tests that require the real `claude` CLI are `#[ignore]`-gated and not run by default.

## License

Apache-2.0. See [`LICENSE`](LICENSE).
