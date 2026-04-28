# Transparent Agent Harness

> **Document type:** Planning - accepted product-direction record.
> **Last updated:** 2026-04-28
> **Status:** Initial operator memory CLI shipped. This records the launch-boundary product agreement for the multi-agent memory control-plane mandate. The `mimir-harness` crate installs the `mimir` binary, preserves native child argv/stdio flow, prints a concise `Claude + Mimir` / `Codex + Mimir` preflight banner, discovers first-run/config state, validates setup, writes a structured session capsule with next actions, memory status, native setup status, remote recovery metadata, and a data-only memory boundary, rehydrates current governed records from an existing canonical log with per-record boundary markers, emits first-run setup artifacts when needed, exposes generated Claude/Codex skill and hook setup artifacts, exposes `mimir status`, `mimir health`, `mimir context`, `mimir memory list|show|explain|revoke`, plus `mimir drafts status|list|show|next|skip|quarantine` for operator setup/memory/draft inspection and triage, exposes `mimir config init` for safe first-run config creation, exposes `mimir setup-agent status|doctor|install|remove` for explicit project/user native setup with dry-run previews and reason-coded diagnostics, exposes `mimir remote status|push|pull` for explicit Git-backed recovery mirroring with relation-aware remediation and explicit `status --refresh`, reports service remotes as an unsupported adapter boundary with dry-run contract output, exposes the exact setup commands plus deterministic cold-start rehydration protocol to the wrapped agent, exposes a session-local checkpoint draft inbox plus `mimir checkpoint`, provides `mimir hook-context` for native hook context injection and checkpoint-route validation, stages checkpoint/native-memory/`agent_export` drafts after the child exits behind reason-coded native-memory adapter health checks, and can run the existing librarian path after capture. Harness integration tests prove captured draft -> librarian accept -> canonical log -> next-launch rehydration, local canonical log + draft push/pull through a Git memory remote, divergent-log pull refusal, relation-aware status remediation, draft conflict reporting, explicit status refresh, service-remote status reporting, service-remote dry-run adapter-boundary reporting, operator status, memory health, setup doctor readiness output, bounded context rendering, governed memory list/show/explain/revoke, draft queue inspection, terminal-sanitized draft display, native-memory adapter drift refusal, cold-start protocol guidance for generic launches, and adversarial literal rendering as data.
> **Cross-links:** [`2026-04-24-multi-agent-memory-control-plane.md`](2026-04-24-multi-agent-memory-control-plane.md) | [`2026-04-24-claude-codex-harness-integration-research.md`](2026-04-24-claude-codex-harness-integration-research.md) | [`2026-04-27-progressive-recall-ladder.md`](2026-04-27-progressive-recall-ladder.md) | [`2026-04-27-cold-start-rehydration-protocol.md`](2026-04-27-cold-start-rehydration-protocol.md) | [`../concepts/scope-model.md`](../concepts/scope-model.md) | [`../concepts/consensus-quorum.md`](../concepts/consensus-quorum.md)

## Product rule

Mimir should be transparent to the operator except for better memory quality, recovery, and cross-agent reuse.

The user should keep launching the agent they wanted to use:

```bash
mimir claude --r
mimir codex
mimir copilot --resume
```

`mimir <agent> [agent args...]` launches the normal native agent UI inside a Mimir-managed session envelope. If `claude --r` normally opens Claude's resume selector, then `mimir claude --r` should show that same selector. Mimir wraps the terminal session; it does not replace the child agent's interaction model.

There is no required first-step `mimir setup` command. On first launch, Mimir enters bootstrap mode inside the requested agent session. The agent guides the operator through local store, remote repository or service, operator/org identity, adapter permissions, and any shell or config integration needed. Setup is an agent-guided first-run state, not a separate CLI ceremony.

## Why the harness is the right boundary

MCP, native hooks, client config files, and future SDKs are adapter conveniences. They are not the core trust boundary.

The core product boundary is the process/session launch:

```text
mimir <agent> [agent args...]
  -> detect project/operator/org context
  -> ensure bootstrap/config, or enter agent-guided bootstrap
  -> create a Mimir session id
  -> prepare a scoped memory capsule
  -> expose any available Mimir tools or sidecar paths
  -> launch the native agent through a PTY
  -> preserve terminal behavior and pass-through args
  -> capture exit/session metadata
  -> sweep/checkpoint native memory outputs as raw drafts
  -> hand drafts to the librarian
```

This lets Mimir work even when the child agent is an opaque terminal app. Rich MCP/tool integrations can improve live recall later, but the first-class product does not depend on every agent client supporting the same plugin model.

## Command semantics

The command shape is:

```text
mimir <agent> [agent arguments...]
```

Rules:

- User-supplied arguments after `<agent>` stay unchanged and keep their relative order.
- Mimir-specific flags, if needed, belong before `<agent>`.
- The shipped first scaffold consumes `--project <name>` before the agent and leaves child-native flags after the agent untouched.
- Known agents may receive Mimir-prepended adapter args that use their native launch surfaces. Today Claude Code receives `--append-system-prompt-file <agent-guide>`, and Codex CLI receives `-c developer_instructions=...`.
- The child process keeps native TTY behavior: raw input, colors, resize events, signals, prompts, and exit codes.
- Known agents use adapter profiles. Unknown agents may still run through a generic wrapper when the executable exists on `PATH`.
- Mimir may print its own short preflight banner before the child process starts. It must not rewrite or filter the child agent's banner/TUI output.
- Persistent skills, hooks, and config edits are generated as inspectable setup artifacts and installed only through explicit operator/agent setup.

Examples:

```bash
mimir claude --r
mimir codex --model gpt-5.4
mimir copilot --resume
mimir --project Mimir claude --r
```

## Implementation status

The current scaffold is intentionally thin:

- workspace crate: `crates/mimir-harness`;
- installed binary: `mimir`;
- launch shape: `mimir [mimir flags] <agent> [agent args...]`;
- terminal behavior: inherited stdin/stdout/stderr and child exit-code propagation;
- bootstrap/config discovery: explicit `MIMIR_CONFIG_PATH` first, then nearest `.mimir/config.toml` found by walking upward from the launch directory;
- session envelope: `MIMIR_HARNESS=1`, `MIMIR_BOOTSTRAP=ready|required`, `MIMIR_SESSION_ID`, `MIMIR_AGENT`, `MIMIR_LIBRARIAN_AFTER_CAPTURE`, `MIMIR_AGENT_GUIDE_PATH`, `MIMIR_AGENT_SETUP_DIR`, `MIMIR_CHECKPOINT_COMMAND`, optional `MIMIR_PROJECT`, optional `MIMIR_CONFIG_PATH`, optional `MIMIR_DATA_ROOT`, optional `MIMIR_WORKSPACE_ID`, optional `MIMIR_WORKSPACE_PATH`, `MIMIR_SESSION_CAPSULE_PATH`, `MIMIR_SESSION_DRAFTS_DIR`, `MIMIR_CAPTURE_SUMMARY_PATH`, and first-run-only `MIMIR_BOOTSTRAP_GUIDE_PATH` / `MIMIR_CONFIG_TEMPLATE_PATH`;
- session capsule: a structured `capsule.json` in the Mimir session directory, currently carrying launch/config/workspace/bootstrap/capture metadata, remote recovery metadata, setup checks, next actions, cold-start memory status, a root `memory_boundary`, warnings, and up to 32 current governed records rendered from the committed canonical log with data-only markers;
- agent-specific context: every prepared launch writes `agent-guide.md`; Claude Code receives it through `--append-system-prompt-file`, while Codex CLI receives a concise `developer_instructions` override pointing at the same guide, `mimir checkpoint`, and setup artifact directory;
- operator status: `mimir status [--project-root <dir>] [--config <file>]` renders a read-only local dashboard with config/bootstrap readiness, workspace/log status, draft counts, remote relation/next action, native setup status, latest capture summary, and one prioritized next action;
- memory health: `mimir health [--project-root <dir>] [--config <file>]` renders a compact readiness zone plus governed-log, pending-draft, capture, remote, native-setup, and recall-telemetry status without raw memory text;
- context assembly: `mimir context [--project-root <dir>] [--config <file>] [--limit <records>]` renders a bounded, data-only context capsule from governed canonical records and metadata-only untrusted supplements. It source-marks governed records, preserves the rehydrated-memory instruction boundary, counts pending drafts without printing raw draft text, and remains read-only when config/logs are missing;
- operator memory controls: `mimir memory list|show|explain|revoke [--project-root <dir>] [--config <file>] [--limit <records>] [--kind sem|epi|pro|inf] [--id <memory-id>] [--reason <text>]` exposes governed canonical records without invoking the librarian for read-only commands. `list`, `show`, and `explain` render canonical Lisp under the data-only boundary; `explain` includes current/stale status, clocks, source, supersession edges, and a revoke command. `revoke` submits a librarian review draft for append-only tombstone/supersession handling and never mutates `canonical.log` directly;
- draft queue status: `mimir drafts status|list|show|next|skip|quarantine [--state <state>] [--project-root <dir>] [--config <file>] [--drafts-dir <dir>]` exposes lifecycle counts, state-filtered previews, oldest-draft review, operator skip/quarantine triage, and raw draft/provenance inspection without invoking the librarian;
- config init helper: `mimir config init [--path <file>|--project-root <dir>] [--data-root <dir>] [--drafts-dir <dir>] [--operator <id>] [--organization <id>] [--remote-url <url>] [--remote-kind git|service] [--remote-branch <branch>] [--librarian-after-capture off|defer|archive_raw|process] [--dry-run]` writes `.mimir/config.toml` safely, refuses overwrites, and lets the wrapped agent preview the exact config before writing;
- remote sync boundary: `mimir remote status|push|pull [--project-root <dir>] [--config <file>] [--dry-run] [--refresh]` is explicit. Launch and capture never sync implicitly. Git status classifies the local-vs-checkout log relation as `missing`, `local_only`, `remote_only`, `synced`, `local_ahead`, `remote_ahead`, or `diverged`, prints the safe next action, and reports copy-only draft conflicts. Plain status reports the current local checkout snapshot; `mimir remote status --refresh` explicitly fetches/pulls the owned checkout first. Git push clones/updates a Mimir-owned checkout under `storage.data_root/remotes/`, mirrors `workspaces/<workspace-hex>/canonical.log` only when the remote log is a prefix of local append-only state, mirrors draft JSON files copy-only under `drafts/<workspace-hex>/<state>/`, commits, and pushes. Git pull restores only missing or prefix-safe logs and copy-only draft files, skipping local-ahead logs and refusing divergent logs or draft content conflicts. Service remotes still do not perform network sync, but `mimir remote push --dry-run` and `mimir remote pull --dry-run` render a machine-readable adapter contract with workspace identity, local log/draft status, append-only log prefix requirements, copy-only draft sync requirements, and the librarian-governed write boundary;
- native setup artifacts: every prepared launch writes `setup/claude/skills/mimir-checkpoint/SKILL.md`, `setup/codex/skills/mimir-checkpoint/SKILL.md`, Claude hook snippets for `SessionStart` plus `PreCompact`, Codex `SessionStart` hook snippets that call `mimir hook-context`, and `setup-plan.md`; these are explicit setup inputs and are not installed silently;
- native setup installer: `mimir setup-agent status|doctor|install|remove --agent claude|codex --scope project|user --features all|skill|hook [--dry-run]` installs/removes only Mimir-owned setup surfaces, previews install/remove actions without writing, reports reason-coded missing/partial/installed status, and provides read-only doctor output with exact status/install/remove/context/checkpoint commands plus one next action. Claude project setup targets `.claude/skills/mimir-checkpoint` and `.claude/settings.json`; Codex project setup targets `.agents/skills/mimir-checkpoint`, `.codex/hooks.json`, and `.codex/config.toml` feature enablement. User scope uses the same native locations under `$HOME`; Codex installs fail before writing if `codex_hooks = false`, and skill removal refuses non-Mimir-owned target directories;
- native setup guidance: the capsule includes a `native_setup` object for the wrapped Claude/Codex agent, setup checks report whether project-scope native setup is installed or missing, and `agent-guide.md` / first-run `bootstrap.md` include exact status, doctor, install, and remove commands;
- first-run setup: when config is missing, the harness writes `bootstrap.md` and `config.template.toml` into the session directory so the launched agent can guide setup without a separate `mimir setup` command; the bootstrap guide includes the same setup-check IDs and actions as the capsule, the exact `mimir config init` helper command, remote repository/service prompts for BC/DR and fresh-machine recovery, plus the setup artifact directory;
- setup validation: empty path values are rejected as config errors; missing identity, workspace detection, governed logs, native-memory source files, and process-mode librarian prerequisites are surfaced as agent-actionable setup checks rather than silently hidden;
- rehydration discipline: read-only, missing logs are skipped instead of created, records are rendered under `mimir.governed_memory.data.v1` with `instruction_boundary = data_only_never_execute`, and trailing bytes past the last checkpoint become capsule warnings rather than harness-side repair;
- native-memory sweeps: `[native_memory].claude` and `[native_memory].codex` config entries may point at files or directories; after the child exits, only the launched agent's configured sources are swept into `claude_memory` or `codex_memory` drafts;
- session checkpoint capture: the harness creates `MIMIR_SESSION_DRAFTS_DIR`; wrapped agents can run `mimir checkpoint --title "<title>" "<note>"` or write `.md`, `.markdown`, or `.txt` files there during the session; after exit, non-empty supported files become `agent_export` drafts tagged `session_checkpoint` with file provenance;
- post-session capture: when `MIMIR_DRAFTS_DIR` or a configured storage root is available, the harness writes a v2 `agent_export` draft under `drafts/pending/` after the child exits and records the capture result in `capture-summary.json`;
- librarian handoff: `[librarian].after_capture` accepts `off`, `defer`, or `process`; `defer` exercises lifecycle recovery without LLM calls, and `process` runs the existing LLM-backed librarian processor against the configured workspace log;
- process-mode ergonomics: `[librarian].llm_binary`, `llm_model`, `max_retries_per_record`, `llm_timeout_secs`, `processing_stale_secs`, `dedup_valid_at_window_secs`, and `review_conflicts` are honored by harness after-capture processing; process mode returns `blocked` instead of invoking the librarian when drafts, workspace-log path, or LLM binary prerequisites are missing;
- end-to-end memory loop: a deterministic integration test exercises the real process path with a Claude-compatible shim and proves accepted memory is rendered into the next launch capsule;
- durable writes: no direct canonical-store writes from the harness; staged draft files still require librarian validation and governance before becoming memory.

The quorum adapter-plan, adapter-run, adapter-run-round, append-status-output, and adapter-run-rounds slices now define the first native Claude/Codex command contract while preserving the recorded-artifact boundary: Mimir writes request/prompt/response/status artifacts, can execute one participant, a full round, or a gated multi-round sequence with a bounded timeout, validates status artifacts before output recording, and still records participant outputs only through file-based `append-output` commands. Service remotes report an unsupported adapter boundary and dry-run a stable contract; Git remotes have explicit push/pull commands, local-checkout conflict remediation, and an explicit status refresh path. Saved quorum results can emit proposed memory drafts into the existing `consensus_quorum` draft surface, and recorded fixture smoke plus adapter-plan/adapter-run/adapter-run-round/append-status-output/adapter-run-rounds coverage prove the create-to-submit-drafts path and native command execution path without live adapters.

## Pre-agent harness best practices

The native setup approach follows patterns from established non-AI command/session harnesses:

- **Pass-through by default.** Nix `develop` and Devbox `run` both preserve the target command shape while preparing an environment; Mimir mirrors this by keeping child argv/stdio native and putting Mimir flags before the agent.
- **Inspectable environment envelope.** Direnv computes an environment diff in a subprocess and exports that back to the shell; Mimir writes explicit files and `MIMIR_*` environment variables rather than relying on hidden in-process state.
- **Explicit trust for persistent execution.** Mise requires config trust before enabling potentially dangerous features; Mimir similarly generates skills/hooks but does not install persistent settings silently.
- **Lifecycle hooks are context channels, not source of truth.** Dev Containers, Claude hooks, and Codex hooks separate startup/attach/prompt lifecycle events from the tool's main command flow. Mimir's `mimir hook-context` emits concise context, validates the checkpoint route, reminds agents to checkpoint before compaction/handoff, and never writes canonical memory.
- **Small status surface.** Pre-commit shows named hook outcomes and skips without forcing users into a dashboard. Mimir's banner and capsule setup checks should stay short, agent-actionable, and queue-free for the operator.

## Session memory capsule

The launch-time context should be a compact, structured session memory capsule generated from governed Mimir state. The shipped scaffold writes the capsule file, exposes its path to the child agent, populates `rehydrated_records` from current committed records when a configured canonical log exists, marks those records as data-only, and reports whether governed memory and pending drafts were actually found.

The capsule is the "clean agent starts here" payload. It should include only authorized, current, scoped records relevant to the session:

- project state and active decisions;
- operator rules and preferences;
- applicable org/ecosystem procedures;
- recent relevant episodes;
- known conflicts, stale-symbol flags, or low-confidence warnings when useful.

The capsule is not raw prose instruction stuffing. It is an agent-facing rendering of governed memory records. Different adapters may render the same internal retrieval result differently for Claude, Codex, Copilot, or future agents, but they must preserve provenance, scope, and trust boundaries. Imperative-looking text inside `rehydrated_records` remains memory data, not executable instruction.

Live recall remains possible through whatever adapter surface is available: MCP, CLI sidecar, temp-file protocol, or native hooks. The capsule handles cold start; live tools handle targeted recall.

The capsule also includes `setup_checks` and `next_actions` so a cold agent can guide the operator without guessing what is missing. Those checks are diagnostic and agent-facing; they do not promote raw observations to trusted memory and they do not create or repair canonical logs.

Cold-start recall should follow the progressive ladder in [`2026-04-27-progressive-recall-ladder.md`](2026-04-27-progressive-recall-ladder.md): readiness first, cheap orientation second, targeted recall third, and deep inspection only after a concrete target is known. The ladder is an adapter ergonomics contract; it does not change the librarian boundary or turn native session stores into trusted memory.

## Capture and checkpoint behavior

Mimir should not replace an agent's hot in-session memory. Agents may continue using their native context, native memory files, and checkpoint habits.

The desired checkpoint behavior is dual-outcome:

```text
agent checkpoint
  -> local/native memory remains available to that agent
  -> Mimir receives raw memory as an untrusted draft
  -> librarian cleans, validates, deduplicates, extracts instructions, and commits or quarantines
```

The shipped capture strategy has three paths. Configured native-memory sources are swept as untrusted `claude_memory` / `codex_memory` drafts using file provenance and `mimir_harness` / `native_memory_sweep` tags. The session-local `MIMIR_SESSION_DRAFTS_DIR` inbox lets wrapped agents write intentional checkpoint notes with `mimir checkpoint` or direct `.md`, `.markdown`, or `.txt` files; those files become untrusted `agent_export` drafts tagged `mimir_harness` / `session_checkpoint`. The harness also writes a v2 post-session metadata draft with `source_surface = "agent_export"`, source agent, project/operator when known, capsule provenance, and explicit `mimir_harness` / `post_session` tags. The raw post-session draft records session metadata and clearly states that no child transcript was captured.

After capture, the harness can hand pending drafts to the librarian according to `[librarian].after_capture`: `off` records only the capture result, `defer` performs safe lifecycle handoff without LLM calls, and `process` runs the LLM-backed `RetryingDraftProcessor` through the same `run_once` path as `mimir-librarian run`. Process mode uses the configured LLM binary/model and fails closed with `blocked` status when required local prerequisites are missing. Handoff results are recorded in `capture-summary.json`.

More invasive transcript capture or live hooks should wait until they are justified by reliability and privacy requirements.

## Agent-operated governance

The operator should not have to babysit a queue.

Drafts, processing states, promotion candidates, conflicts, and quarantined records are governance work items that agents can inspect through Mimir. A human approval moment is required only when a trust boundary is crossed or the agent needs an operator decision:

- durable operator instruction;
- org/ecosystem promotion;
- conflict resolution;
- revocation or supersession of important memory;
- suspicious or prompt-injection-like content;
- remote sync or permission changes.

The normal human interaction should be agent-mediated: "There are promotion candidates and one conflict; should I inspect them?" not "open a dashboard before you can work."

## Relationship to consensus quorum

The harness shape generalizes cleanly to future cross-agent discussion:

```bash
mimir quorum ...
```

or an agent-mediated request from within a wrapped session.

Consensus quorum still follows the existing rule: deliberation outputs are evidence drafts or review artifacts, not canonical memory. The wrapper makes it easier to enlist Claude, Codex, Copilot, and future adapters later, but quorum is staged after governed memory intake is useful.

## Near-term implementation priority

This document does not move the immediate implementation target away from the librarian.

The build order remains:

1. Finish scope-aware draft processing.
2. Wire bounded LLM retry and pre-emit validation.
3. Commit accepted drafts to canonical storage safely.
4. Add reliable recovery and rehydration.
5. Extend the shipped thin `mimir <agent>` harness with agent-guided bootstrap ergonomics.
6. Expand adapters, live tools, and quorum only after the memory core is dependable.

The harness should stay thin until memory quality is clearly better than local markdown for recovery, cross-agent transfer, and organization-wide skill/instruction distillation.
