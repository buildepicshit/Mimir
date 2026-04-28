# Current Handoff - 2026-04-24

> **Supersession note — 2026-04-27:** This handoff is historical for PR #22-era productization work. Its "Actions disabled" guidance was superseded when the owner added more GitHub Actions usage and explicitly approved re-enabling Actions for `buildepicshit/Mimir` on 2026-04-27. The quota discipline still stands: full local gate before push, one batched push, no empty retry commits, and ask before retrying transient infra failures.

> **Purpose:** compact project-state handoff for continuing the Mimir productization work from the current branch.
> **Branch:** `feat/scope-aware-drafts`
> **PR:** [#22](https://github.com/buildepicshit/Mimir/pull/22)
> **Base:** `main` at `be76b0d`
> **Head before lifecycle continuation:** `bbfd3f5`
> **CI:** Historical state at handoff time: GitHub Actions intentionally disabled to conserve the org's monthly Actions quota. Superseded 2026-04-27 as noted above.

## Current Product Direction

Mimir is now a multi-agent memory governance/control plane.

The active invariant is:

> Memory is local until governed.

Raw agent and project memories stay isolated by default. Cross-project, operator-level, and ecosystem reuse happens only through librarian-governed promotion with provenance, trust tier, scope, and revocation.

Consensus quorum is now also first-class: Claude, Codex, and future adapters can deliberate from explicit personas, but quorum outputs are governed evidence drafts. They are not truth and never write canonical memory directly.

The accepted product entry point is a transparent launch harness:

```bash
mimir claude --r
mimir codex
mimir copilot --resume
```

Mimir should preserve the native agent terminal flow while wrapping the session with bootstrap, rehydration, capture, and governance. There is no required separate `mimir setup`; first-run configuration happens inside the requested agent session. MCP, hooks, and native config are optional adapter conveniences, not the foundational trust boundary.

## What PR #22 Contains

Product commits on `feat/scope-aware-drafts` before this handoff doc:

- `c71183b feat(librarian): add scope-aware draft submit`
- `96a342f spec(quorum): add consensus mandate`
- `bb4ead0 feat(librarian): add explicit memory sweep`
- `bbfd3f5 docs(handoff): record current productization state`

Functional changes:

- v2 draft metadata with source surface, agent, project, operator, provenance, tags, and submitted timestamp.
- Filesystem-backed `DraftStore` with `pending`, `processing`, `accepted`, `skipped`, `failed`, and `quarantined` lifecycle directories.
- `mimir-librarian submit --text ...` for explicit scoped draft submission.
- `mimir-librarian submit --source-surface consensus-quorum` for governed quorum evidence drafts with explicit episode provenance.
- Typed `QuorumEpisode` / `QuorumParticipantOutput` / `QuorumResult` envelopes plus `QuorumStore` create/load result storage, participant output append/load, and independent-first visibility gating for the file-backed quorum foundation.
- `mimir-librarian quorum create|pilot-plan|pilot-status|pilot-run|pilot-review|pilot-summary|append-output|outputs|visible` for recorded quorum episode/output artifacts and replayable, reviewable pilot manifests/status/execution/summary before service-adapter design.
- Typed `QuorumAdapterRequest` plus `mimir-librarian quorum adapter-request` for stable participant request JSON with visibility-gated prior outputs.
- `mimir-librarian quorum adapter-plan` for native Claude/Codex command plans that write request/prompt/response artifacts and return the matching file-based `append-output` command without executing adapters or writing memory directly.
- `mimir-librarian quorum adapter-run` for bounded Claude/Codex plan execution with prompt stdin, response/status artifacts, stdout/stderr byte counts, timeout handling, and no direct participant-output append.
- `mimir-librarian quorum adapter-run-round` for running every participant adapter in one round, aggregating status artifacts, preserving independent-first visibility gates, and still requiring explicit `append-output` to record participant outputs.
- `mimir-librarian quorum append-status-output` for validating successful adapter-run or adapter-run-round status artifacts and then recording participant outputs through the existing `append-output` command path.
- `mimir-librarian quorum adapter-run-rounds` for sequencing independent, critique, and revision rounds through explicit round status artifacts and append-status-output gates before later-round visibility opens.
- `mimir-librarian quorum synthesize` for explicit structured `QuorumResult` recording from supplied synthesis fields plus stored participant-output evidence references.
- `mimir-librarian quorum submit-drafts` for submitting saved `QuorumResult.proposed_memory_drafts` into the existing `consensus_quorum` draft surface with episode provenance and decision/consensus tags.
- A recorded fixture smoke test covering `quorum create -> adapter-request -> append-output -> synthesize -> submit-drafts` without live adapters.
- `mimir-librarian sweep --path ...` for explicit file/directory ingestion into pending drafts.
- Directory sweeps recurse only through `.md`, `.markdown`, and `.txt` files.
- Sweeps require explicit `--source-surface`; `mcp` and `cli` are rejected for sweep because they are not file-sweep surfaces.
- Codex and Claude sweep surfaces default `source_agent` to `codex` and `claude` when not provided.
- Draft lifecycle transitions are now owned by `DraftStore`: `pending -> processing`, `processing -> accepted | skipped | failed | quarantined`, and claim-age stale `processing -> pending` recovery.
- Transition errors are typed: invalid edge, missing source draft, and occupied target draft are distinct failures.
- `DraftProcessor` / `run_once` now provide the one-shot processing skeleton: recover stale `processing`, claim pending drafts, invoke an injected processor, and move drafts to terminal states or safely back to `pending`.
- `mimir-librarian run` is wired to the bounded LLM validation retry processor by default. It emits a JSON summary; `--defer` keeps the lifecycle-only dry-run behavior and exits `70` when drafts are deferred.
- `PreEmitValidator` now validates candidate canonical Lisp against a scratch `mimir_core::Pipeline`, commits successful validations into scratch state, and rolls rejected candidates back exactly.
- `RetryingDraftProcessor` invokes the LLM, parses the JSON output contract, validates candidate Lisp transactionally, commits accepted batches through `mimir_core::Store::commit_batch`, and re-prompts with structured retry hints for JSON / parse / bind / semantic / emit failures. Store-level pipeline rejections also feed back into the retry loop.
- Deterministic supersession conflicts now branch out of the retry loop: default policy skips with a warning; `--review-conflicts` writes a JSON artifact to `drafts/conflicts/` and quarantines the draft.
- Exact duplicate Semantic, Episodic, Procedural, and Inferential records are filtered before commit. All-duplicate drafts skip; mixed batches commit only the unique candidate records.
- Configurable same-day `valid_at` dedup is wired for otherwise-identical Semantic and Inferential records. The default window is one day; `--dedup-valid-at-window-secs 0` restores exact-only behavior.
- `mimir-librarian watch` is wired as a polling scheduler over the same run path. `--poll-secs N` controls cadence; `--iterations N` bounds the loop for scripts and tests.
- Runner-level observability is wired: `run_once` emits `mimir.librarian.run` with run counters and `mimir.librarian.draft_processed` per moved draft, without raw draft prose.
- Processor-level observability is wired: `RetryingDraftProcessor::process` emits `mimir.librarian.process` with retry/error/record counters, plus structured retry/duplicate/supersession events without raw draft prose, paths, LLM text, retry prompts, validation errors, or canonical Lisp payloads.
- Shared workspace write-lock discipline is wired: `mimir_core::WorkspaceWriteLock` acquires `<canonical-log>.lock` atomically, `mimir-librarian` holds it before opening `Store`, and `mimir-mcp` holds it for the lifetime of a write lease.
- The transparent harness scaffold is wired in `crates/mimir-harness`: `mimir [flags] <agent> [agent args...]` preserves user-supplied child args, launches with inherited terminal streams, propagates the child exit code, discovers bootstrap/config state, derives workspace-log env when storage is configured, writes a structured session capsule, emits first-run bootstrap guide/template artifacts when config is missing, validates setup, surfaces next actions plus governed-log / pending-draft / remote-recovery memory status, rehydrates current governed records from an existing canonical log without creating missing logs or repairing tails, writes `MIMIR_AGENT_GUIDE_PATH`, exposes `MIMIR_SESSION_DRAFTS_DIR` plus `mimir checkpoint` for intentional checkpoint notes, exposes `mimir status` and `mimir drafts status|list|show|next|skip|quarantine` for operator setup/draft inspection and triage, exposes `mimir config init` for safe first-run config creation, exposes `mimir remote status|push|pull` for explicit Git-backed recovery mirroring, injects Claude context via `--append-system-prompt-file` and Codex context via `-c developer_instructions=...`, sweeps configured Claude/Codex native-memory files, stages checkpoint and post-session `agent_export` drafts under `drafts/pending/`, records capture plus config-driven librarian handoff results in `capture-summary.json`, blocks process-mode handoff when drafts/workspace/LLM prerequisites are missing, and has integration proofs for captured draft -> accepted memory -> next-launch rehydration plus Git remote push/pull of governed logs and drafts.
- Release publish ordering now matches crates.io first-publish constraints: dry-run `mimir-core` before publishing anything, then dry-run each dependent crate immediately before its real publish after the previous dependency has propagated. `mimir-librarian` now publishes before `mimir-harness` because the harness links the librarian library for after-capture handoff.

Documentation changes:

- `docs/concepts/consensus-quorum.md` added as a draft spec.
- `docs/planning/2026-04-24-current-handoff.md` added as this continuation note.
- `AGENTS.md`, `STATUS.md`, `docs/concepts/README.md`, `docs/concepts/scope-model.md`, and `docs/planning/2026-04-24-multi-agent-memory-control-plane.md` updated to include consensus quorum.
- `docs/planning/2026-04-24-transparent-agent-harness.md` added to capture the `mimir <agent> [agent args...]` launch-boundary agreement, with cross-links from `AGENTS.md`, `STATUS.md`, `scope-model.md`, and the control-plane plan; later updated to record the shipped first scaffold.
- `crates/mimir-librarian/README.md` updated for submit/sweep/lifecycle/run usage.

## Verification

Fresh local gate passed after the lifecycle, run-skeleton, pre-emit-validation, bounded-retry, durable-commit, supersession-conflict-policy, exact-duplicate-filter, Episodic-retention, configurable valid-at-dedup, polling-watch scheduling, runner-level observability, processor-level observability, Python-prototype retirement, workspace-write-lock, transparent-harness bootstrap-config-helper, quorum proposed-draft emission, recorded quorum fixture smoke, quorum adapter-plan, quorum adapter-run, quorum adapter-run-round, quorum append-status-output, quorum adapter-run-rounds, synthesis hardening, pilot-plan/status/run, pilot-run recovery, pilot-review, pilot-summary, required proposed-draft exit-criteria, service-remote dry-run boundary, and harness status/drafts UX slices:

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
git diff --check
```

`STATUS.md` tracks `648` passing tests after the lifecycle, run-skeleton, pre-emit-validation, bounded-retry, durable-commit, supersession-conflict-policy, exact-duplicate-filter, Episodic-retention, configurable valid-at dedup, polling-watch scheduling, runner-level observability, processor-level observability, Python-prototype retirement, workspace-write-lock, consensus-quorum draft intake, quorum episode/output/adapter-request/result envelopes, independent-first visibility gating, recorded-artifact quorum CLI commands, explicit quorum result synthesis, quorum proposed-draft emission, recorded quorum fixture smoke coverage, native Claude/Codex adapter-plan coverage, bounded adapter-run coverage, adapter-run-round orchestration coverage, append-status-output validation coverage, adapter-run-rounds sequence coverage, pilot-plan/status/run coverage, pilot-run failure reporting and rerun recovery, pilot-review certification, pilot-summary reporting, required proposed-draft exit criteria, and bootstrap/config/session-capsule/rehydration/post-session-capture/native-memory-sweep/bootstrap-artifact/capture-summary/setup-validation/librarian-handoff/process-mode-rehydration/session-checkpoint/agent-specific-checkpoint-ergonomics/native-setup-preflight/setup-agent-installer/setup-aware-bootstrap/setup-agent-hardening/bootstrap-config-helper/remote-sync-boundary/service-remote-status/service-remote-dry-run-contract/remote-conflict-remediation/remote-status-refresh/operator-status/draft-queue harness slices.

## Closing State

The latest continuation is intentionally one coherent librarian + harness chunk after `bbfd3f5`: lifecycle transitions, claim-age stale recovery, one-shot `run`, scratch-pipeline pre-emit validation, bounded LLM validation retry, durable canonical commit for accepted drafts, supersession-conflict policy, exact duplicate filtering across all four memory types, configurable same-day `valid_at` dedup, polling `watch` scheduling, runner-level observability, processor-level observability, Python prototype retirement, workspace write-lock discipline, consensus-quorum draft intake, typed envelope storage, participant output append/load with independent-first visibility gating, recorded-artifact quorum CLI commands, adapter-request contract JSON, native Claude/Codex adapter plans, bounded adapter-run status capture, adapter-run-round orchestration, append-status-output validation, adapter-run-rounds sequencing, explicit result synthesis, proposed-draft emission into the governed draft store, recorded fixture smoke coverage, replayable pilot planning/status/run execution, pilot-run partial failure reporting and complete-gate rerun skipping, pilot-review certification artifacts, pilot-summary reporting, required proposed-draft exit criteria for live pilot manifests, the Claude shim test race fix, transparent harness bootstrap artifacts/rehydration/post-session capture/native-memory sweep/session-checkpoint capture/capture summary/setup validation/process-mode hardening/setup-agent hardening/bootstrap-config helper/remote-sync boundary/remote conflict remediation/status refresh/operator status/draft queue UX, and matching docs/status updates.

## Important Constraints

- Do not re-enable GitHub Actions without operator approval. The 2026-04-27 operator-approved exception restored Actions after additional usage was added.
- Verify locally before pushing.
- No direct pushes to `main`.
- No AI attribution in commits, PRs, or project output.
- Do not let agent outputs bypass the librarian.
- Do not treat quorum majority as truth or erase dissent.

## Latest Engineering Slices

Draft processing state transitions, the one-shot `run` skeleton, and scratch-pipeline pre-emit validation are now wired.

Target behavior:

```text
pending/<id>.json
  -> processing/<id>.json
  -> accepted/<id>.json | skipped/<id>.json | failed/<id>.json | quarantined/<id>.json
```

Implementation requirements:

- Atomic rename-based transitions: complete.
- Crash-safe resume for stale `processing` drafts: complete. Recovery uses processing claim markers, so stale age is based on claim time rather than original draft submission time.
- Typed transition errors: complete.
- Tests for valid transitions, invalid transitions, idempotent recovery, target-overwrite protection, and list/load behavior after moves: complete.
- One-shot runner skeleton: complete. It recovers stale processing drafts, claims pending work, calls an injected processor, and emits a JSON summary.
- CLI `run`: complete for lifecycle-safe bounded validation retry. It uses the retrying LLM processor by default and keeps `--defer` for lifecycle-only dry-runs.
- Pre-emit validation is wired as an in-process scratch `mimir_core::Pipeline` pass and now sits inside the retrying processor.
- Bounded retry loop: complete. The processor invokes the LLM, validates the whole candidate response transactionally, and re-invokes with structured retry hints until success, skip, or retry exhaustion.
- Durable commit: complete. Accepted drafts commit their validated batch through `mimir_core::Store::commit_batch`; store-level pipeline rejections are retryable and do not mark drafts accepted unless classified as deterministic supersession conflicts.
- Supersession-conflict policy: complete. Equal `(s, p, valid_at)` collisions from validation or durable commit skip by default, and `--review-conflicts` writes a provenance-rich review artifact under `drafts/conflicts/` before returning `Quarantined`.
- Episodic retention: complete. `mimir_core::Pipeline` now retains/replays committed `EpiRecord`s via `episodic_records()`.
- Exact duplicate filter: complete for Semantic, Episodic, Procedural, and Inferential records. Exact duplicates skip even in review-conflicts mode; mixed batches keep and commit unique records only.
- Configurable valid-at dedup: complete for otherwise-identical Semantic and Inferential records. Default is a one-day window; exact-only mode is `--dedup-valid-at-window-secs 0` or `DedupPolicy::exact()`.
- Scheduling: complete for one-shot `run`, polling `watch`, and systemd/cron recipes.
- Runner-level observability: complete.
- Processor-level observability: complete for retry/error/record metrics and privacy-safe processor events.
- Python librarian prototype: retired. `research/librarian/run_librarian.py` is deleted; findings remain archived under `research/librarian/`.
- Workspace write-lock discipline: complete for local direct writers. `mimir_core::WorkspaceWriteLock` owns `<canonical-log>.lock`, `mimir-librarian` acquires it before `Store::open`, and `mimir-mcp` ties it to the write-lease lifetime. Expired-lease and release paths drop the lock only while holding the lease mutex, so another process cannot acquire the log while an accepted MCP write is still in flight.
- Transparent harness scaffold: complete for the first process boundary plus bootstrap/config preparation, first-run setup artifacts, setup validation, read-only governed rehydration, post-session draft capture, configured native-memory sweeps, session-checkpoint draft capture, agent-specific checkpoint ergonomics, native setup preflight, setup-agent installer commands, setup-aware bootstrap/capsules, setup-agent dry-run/status hardening, bootstrap config helper, remote recovery metadata surfacing, explicit Git remote sync, service-remote dry-run adapter contracts, capture summaries, and config-driven librarian handoff. `mimir-harness` installs `mimir`, parses Mimir pre-agent flags, preserves user-supplied child argv/stdio, prints a concise `Claude + Mimir` / `Codex + Mimir` preflight banner, propagates exit code, resolves `MIMIR_CONFIG_PATH` or nearest `.mimir/config.toml`, derives `MIMIR_WORKSPACE_PATH` from detected git workspace plus configured storage root, derives or accepts `MIMIR_DRAFTS_DIR`, writes a structured session capsule at `MIMIR_SESSION_CAPSULE_PATH`, exposes `MIMIR_SESSION_DRAFTS_DIR`, `MIMIR_AGENT_GUIDE_PATH`, `MIMIR_AGENT_SETUP_DIR`, and `MIMIR_CHECKPOINT_COMMAND`, writes generated Claude/Codex skill and hook setup artifacts plus `setup-plan.md`, provides `mimir config init` for safe no-overwrite first-run `.mimir/config.toml` creation with optional `[remote]` metadata, provides `mimir remote status|push|pull` with dry-run planning, Mimir-owned Git checkouts, append-only canonical-log prefix checks, copy-only draft JSON mirroring, and service-remote dry-run contract output, provides `mimir hook-context` for native hook context injection, provides `mimir setup-agent status|doctor|install|remove --dry-run` for explicit project/user Claude/Codex skill/hook setup with read-only doctor next actions and reason-coded diagnostics, reports wrapped-agent native setup status in `capsule.native_setup`, writes exact config/setup status/doctor/install/remove commands into `agent-guide.md` and first-run `bootstrap.md`, writes `MIMIR_BOOTSTRAP_GUIDE_PATH` and `MIMIR_CONFIG_TEMPLATE_PATH` artifacts when config is missing, injects Claude context via `--append-system-prompt-file` and Codex context via `-c developer_instructions=...`, exposes setup checks and next actions, reports governed-log presence / rehydrated record count / pending draft count, fills `rehydrated_records` from current committed records when a canonical log already exists, sweeps `[native_memory].claude` / `[native_memory].codex` files after matching agent launches, stages `mimir checkpoint` notes plus direct session-checkpoint files and post-session v2 `agent_export` drafts after the child exits, supports `[librarian].after_capture = "off" | "defer" | "process"`, supports process-mode LLM/retry/timeout/stale/dedup/conflict config, reports `blocked` when process prerequisites are missing, proves a captured draft can become next-launch governed memory, proves Git remote push/pull recovery mirroring, and writes `MIMIR_CAPTURE_SUMMARY_PATH`.

## 2026-04-25 Continuation

Latest local slice: live quorum pilot planning, runner recovery, and review certification.

- `mimir-librarian quorum synthesize-run` status now distinguishes native process status from proposed-result validity with `process_status`, `result_valid`, and `validation_error`.
- A zero-exit Claude/Codex synthesizer that writes incomplete or invalid result JSON now produces a status artifact with `success = false` instead of looking usable.
- `accept-synthesis` accepts either `--result-file` or a successful `--status-file`, then validates the proposed result against the episode before saving: schema/version, episode id, non-empty text fields, finite confidence, duplicate votes, and exactly one participant vote per episode participant.
- Direct `quorum synthesize` uses the same result-field and participant-vote validation before writing `result.json`.
- `mimir-librarian quorum pilot-plan` now writes a replayable manifest for multi-round participant execution, synthesis, status-backed acceptance, and draft submission, including exact argv, expected status/result paths, and optional `--require-proposed-drafts N` exit criteria.
- `mimir-librarian quorum pilot-status --manifest-file PATH` reads that manifest and reports run-rounds, synthesis, acceptance, and draft-submission gates as pending, complete, or failed from recorded artifacts; required proposed-draft counts fail acceptance/submission when the accepted result does not provide enough drafts.
- `mimir-librarian quorum pilot-run --manifest-file PATH` executes the manifest by replaying the recorded gated commands in order, returns the final pilot status, reports partial state on failed steps, exits nonzero for unsuccessful CLI runs, and skips already-complete gates on rerun.
- `mimir-librarian quorum pilot-review --manifest-file PATH` records reviewer, decision, findings, next actions, and the full status snapshot. `--decision pass` requires complete pilot status.
- `mimir-librarian quorum pilot-summary --manifest-file PATH` now reports result/review presence, review decision, proposed draft count, submitted draft count, gates, and next action for the manifest.
- `mimir status` now reports config/bootstrap readiness, workspace/log status, draft counts, remote relation/next action, native setup status, latest capture summary, and one prioritized next action.
- `mimir drafts status|list|show|next|skip|quarantine` now exposes lifecycle counts, state-filtered previews, oldest-draft review, operator skip/quarantine triage, and raw draft/provenance inspection without invoking the librarian.
- A real local Claude/Codex independent-round pilot ran from `/tmp/mimir-live-out.CAYk7N/live-pilot-20260425-001-pilot-plan.json`: the first sandboxed run failed at `run_rounds` with structured status, the approved rerun completed, the accepted result is under `/tmp/mimir-live-quorum.527Yx5/episodes/live-pilot-20260425-001-466a019d687d4fb9/result.json`, and the pass review artifact is `/tmp/mimir-live-out.CAYk7N/live-pilot-20260425-001-pilot-review.json`.
- Live pilot finding: execution/synthesis/acceptance/review gates were proven with real Claude/Codex, but the first accepted synthesis proposed no memory drafts.
- A second real local Claude/Codex independent-round pilot ran from `/tmp/mimir-required-out.V6r9Du/live-pilot-20260425-002-pilot-plan.json` with `--require-proposed-drafts 1`: the run completed `run_rounds`, `run_synthesis`, `accept_synthesis`, and `submit_drafts`; the accepted result is under `/tmp/mimir-required-quorum.I4sHk4/episodes/live-pilot-20260425-002-eac0b1384d607ebd/result.json`; the staged governed draft is `/tmp/mimir-required-drafts.Ruf0tL/pending/64f69c7631b7b9be.json`; and the pass review artifact is `/tmp/mimir-required-out.V6r9Du/live-pilot-20260425-002-pilot-review.json`.
- Added regression coverage for invalid synthesize-run output, status-backed acceptance, failed status rejection, missing participant votes, duplicate participant votes, pilot-plan manifest generation, pending pilot status, complete shim-backed pilot status, required proposed-draft pilot-status failure, pilot-run manifest execution, pilot-run failed-step reporting, pilot-run rerun skipping, required proposed-draft pilot-run failure, pilot-review pass rejection for incomplete status, pilot-review certification artifacts, pilot-summary reporting, harness operator status, and draft queue status/list/show/next/skip/quarantine output.
- `STATUS.md` now tracks 648 passing tests.

## Next Engineering Slice

Implement the service remote transport behind the dry-run adapter contract when the service endpoint contract is ready. Git remotes already have explicit push/pull, checkout/log/draft status, divergent-log refusal, conflict remediation, and freshness semantics; `remote.kind = "service"` now reports unsupported real sync while `push --dry-run` / `pull --dry-run` expose the adapter boundary without network I/O.
