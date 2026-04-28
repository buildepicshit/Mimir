# mimir-harness

Transparent launch harness for [Mimir](https://github.com/buildepicshit/Mimir).

> **Pre-1.0 status.** This crate is part of Mimir's active-development tree. Config keys, native setup artifacts, capture behavior, and CLI flags may change before v1. Public crates.io releases wait for the first alpha.

Until the first alpha release, install the `mimir` binary from the repository root:

```bash
cargo install --locked --path crates/mimir-harness
```

Examples:

```bash
mimir claude --r
mimir codex --model gpt-5.4
mimir --project Mimir claude --r
```

For a fresh-clone walkthrough that verifies the wrapper with a no-op child process before launching a real agent, see [`docs/first-run.md`](../../docs/first-run.md).

The implementation is intentionally thin: Mimir consumes only its own pre-agent flags, preserves every user-supplied child argument, prints a short `Claude + Mimir` / `Codex + Mimir` preflight banner, launches the native agent with inherited terminal streams, propagates the child exit code, discovers bootstrap/config state, validates the launch setup, and writes a structured session capsule before launch. Known agents receive small launch-time context through their native surfaces: Claude Code gets `--append-system-prompt-file`, while Codex CLI gets `-c developer_instructions=...`. Each launch also writes native setup artifacts under `MIMIR_AGENT_SETUP_DIR`: Claude and Codex checkpoint skills plus hook snippets that call `mimir hook-context`. Those artifacts are inspectable and opt-in; Mimir does not silently mutate persistent agent settings. `mimir status` gives operators and wrapped agents a read-only project dashboard, `mimir health` gives a compact memory-readiness view, `mimir context` renders a bounded data-only context capsule from governed records plus metadata-only untrusted supplements, `mimir memory list|show|explain|revoke` exposes governed canonical records and append-only revocation requests, and `mimir drafts status|list|show|next|skip|quarantine` exposes the local draft queue without invoking the librarian. `mimir config init` gives the wrapped agent a safe first-run helper for `.mimir/config.toml`, including optional remote recovery metadata. `mimir remote status|push|pull|drill` provides an explicit Git-backed BC/DR mirror and restore-drill boundary for governed logs and draft files; projects can opt into `remote.auto_push_after_capture = true` to run the same verified push path after capture and librarian handoff, while pull remains explicit. `mimir setup-agent status|doctor|install|remove` provides the explicit one-time setup path for project/user Claude and Codex skills/hooks, including dry-run previews, readiness diagnostics, reason-coded status output, and protection around non-Mimir-owned skill directories. The capsule/first-run guides include the exact config and native setup commands for the wrapped agent. When a configured canonical log already exists, the capsule, `mimir context`, and `mimir memory` include current governed records rendered as Lisp with a data-only instruction boundary. During the session, the wrapped agent can run `mimir checkpoint` or write intentional memory checkpoint notes into `MIMIR_SESSION_DRAFTS_DIR`; after exit, the harness stages those notes, configured native-memory files, and a raw `agent_export` draft, then optionally hands pending drafts to the librarian. Process-mode handoff now has a tested end-to-end loop: a captured draft can be accepted into the canonical log and rehydrated into the next wrapped launch.

## Environment Contract

Wrapped agents receive:

- `MIMIR_HARNESS=1`
- `MIMIR_BOOTSTRAP=ready|required`
- `MIMIR_SESSION_ID=<generated session id>`
- `MIMIR_AGENT=<agent executable>`
- `MIMIR_PROJECT=<project>` when `--project` is supplied
- `MIMIR_SESSION_CAPSULE_PATH=<path>` pointing at the JSON launch capsule
- `MIMIR_SESSION_DRAFTS_DIR=<path>` pointing at a session-local checkpoint draft inbox
- `MIMIR_AGENT_GUIDE_PATH=<path>` pointing at the wrapped-agent memory guide
- `MIMIR_AGENT_SETUP_DIR=<path>` pointing at generated native setup artifacts for Claude/Codex
- `MIMIR_CHECKPOINT_COMMAND=mimir checkpoint`
- `MIMIR_CAPTURE_SUMMARY_PATH=<path>` pointing at the post-child capture summary
- `MIMIR_CONFIG_PATH=<path>` when config was discovered
- `MIMIR_DATA_ROOT=<path>` when configured
- `MIMIR_DRAFTS_DIR=<path>` when configured or derived from the configured storage root
- `MIMIR_WORKSPACE_ID=<id>` and `MIMIR_WORKSPACE_PATH=<canonical.log>` when a git workspace and data root are both available
- `MIMIR_BOOTSTRAP_GUIDE_PATH=<path>` and `MIMIR_CONFIG_TEMPLATE_PATH=<path>` when first-run setup is required

Config discovery checks `MIMIR_CONFIG_PATH` first, then walks upward from the launch directory for `.mimir/config.toml`. Empty configured path values are rejected because they cannot produce a stable memory boundary. If no storage root is configured, Mimir sets `MIMIR_BOOTSTRAP=required`, writes an agent-facing bootstrap guide plus a TOML template into the session directory, exposes a `mimir config init` command, and still launches the requested agent so first-run setup can happen inside the normal agent session.

`capsule.json` includes launch metadata, setup checks, next actions, native setup status, cold-start memory status, warnings, and rehydrated records. Setup checks cover config discovery, storage/drafts availability, remote recovery metadata, operator/org identity, workspace detection, governed-log availability, native Claude/Codex setup status, native-memory source availability for the launched agent, and process-mode librarian readiness. Memory status reports whether a governed log was present, how many governed records were rendered into the capsule, and how many pending draft files are waiting for librarian processing when a pending directory exists. The generated agent guide includes the cold-start rehydration protocol: current workspace instructions first, `mimir health`, governed capsule records, open-work metadata, untrusted adapter supplements, warnings, then a budgeted provenance-preserving summary.

## Operator Status

The operator status and draft review surface summarizes local setup without launching an agent or processing drafts:

```bash
mimir status
mimir status --project-root /path/to/project
mimir doctor
mimir health
mimir context --limit 12
mimir memory list --limit 20
mimir memory explain --id @__mem_0
mimir memory revoke --id @__mem_0 --reason "incorrect or obsolete"
mimir drafts status
mimir drafts list --state pending
mimir drafts next
mimir drafts show <draft-id>
mimir drafts skip <draft-id> --reason "duplicate"
mimir drafts quarantine <draft-id> --reason "unsafe or ambiguous"
```

`mimir status` reports config/bootstrap readiness, operator/org identity, workspace id, governed log path and presence, draft queue counts, remote relation/next action when configured, project native setup status for Claude/Codex, latest capture summary presence, and one prioritized `next_action`. `mimir doctor` is the first-run readiness front door: it reports the same metadata plus a prioritized action list covering config, workspace identity, draft backlog, native Claude/Codex setup, librarian mode, remote sync, and capture summaries without printing raw draft text. `mimir health` reports a compact `green` / `amber` / `red` readiness zone plus the fields needed for the progressive recall ladder, including pending draft age and telemetry availability; it does not print raw memory text. `mimir context` reports the same safety boundary and then renders up to `--limit` governed records from the committed canonical log as source-marked `context_record` lines; pending drafts appear only as counts and metadata-only untrusted supplements. `mimir memory list` renders bounded governed records with optional `--kind sem|epi|pro|inf`; `show` renders one canonical Lisp payload by memory id; `explain` adds current/stale status, clocks, source, supersession edges, and the matching revoke command; `revoke` stages a librarian review draft and leaves `canonical.log` untouched. `mimir drafts status` prints lifecycle counts across `pending`, `processing`, `accepted`, `skipped`, `failed`, and `quarantined`; `list` gives one-line metadata and terminal-sanitized previews for a state; `next` prints the oldest submitted draft in a state for immediate review; `skip` and `quarantine` move a draft to terminal review states with a reason artifact; `show` prints one draft's provenance metadata and terminal-sanitized raw text for operator review. Use `--config <file>` or `--drafts-dir <dir>` to inspect non-default locations.

## Config Bootstrap

Wrapped agents can create project config safely during first-run bootstrap:

```bash
mimir config init --operator hasnobeef --organization buildepicshit --remote-url git@github.com:org/mimir-memory.git --dry-run
mimir config init --operator hasnobeef --organization buildepicshit --remote-url git@github.com:org/mimir-memory.git
```

By default this writes `.mimir/config.toml` with `data_root = "state"`, which resolves to `.mimir/state` because config paths are relative to the config file. Use `--path <file>` or `--project-root <dir>` to target another location, `--data-root <dir>` or `--drafts-dir <dir>` for storage overrides, and `--librarian-after-capture off|defer|archive_raw|process` for capture behavior. The command refuses to overwrite an existing config; `--dry-run` prints the TOML without writing.

The optional `[remote]` section records the intended shared memory repository or service for BC/DR and fresh-machine recovery. Git remotes can now be used explicitly through `mimir remote`; service remotes report an unsupported adapter boundary and support dry-run contract rendering until a service adapter lands. Set `auto_push_after_capture = true` only when the operator wants every wrapped-session capture to run a verified Git push after the librarian handoff.

## Remote Sync

Manual remote movement is explicit:

```bash
mimir remote status
mimir remote status --refresh
mimir remote push --dry-run
mimir remote push
mimir remote pull
mimir remote drill --dry-run
mimir remote drill --destructive
```

`mimir remote status` reports the configured Git target, branch, derived Mimir-owned checkout under `storage.data_root/remotes/`, local workspace log status, draft counts, and the explicit push/pull commands. By default it classifies the local checkout snapshot and prints `status_snapshot=local_checkout`; `mimir remote status --refresh` explicitly fetches/pulls the owned checkout first and prints `status_snapshot=refreshed_checkout`. Status classifies the local-vs-checkout log relation as `missing`, `local_only`, `remote_only`, `synced`, `local_ahead`, `remote_ahead`, or `diverged`, then prints `next_action` and `remediation` lines. `--project-root <dir>` and `--config <file>` target a different project or config.

`mimir remote push` clones or updates the configured Git remote into the Mimir-owned checkout, mirrors the current workspace `canonical.log` under `workspaces/<workspace-hex>/canonical.log`, mirrors draft JSON files under `drafts/<workspace-hex>/<state>/`, commits changes, and pushes the configured branch. Canonical logs are append-only checked: push refuses if the remote log is not a prefix of the local log. Push verifies the source and mirrored log before publishing.

`mimir remote pull` updates the same checkout and restores safe state locally. It copies a remote log only when the local log is missing or a prefix of the remote log, skips when local is already ahead, and refuses divergent logs. Pull verifies the remote source and local restored/skipped log before reporting success. Draft mirroring is copy-only; identical files are skipped and same-name/different-content files are conflicts. Remote sync does not validate or promote drafts, and it does not bypass the librarian.

`mimir remote drill --destructive` runs the BC/DR proof: delete the local workspace `canonical.log`, restore via `mimir remote pull`, verify the restored log, reopen the store, and execute `(query :limit 1)` as a sanity query. `mimir remote drill --dry-run` prints the deletion target and restore/verify steps without changing local state. `scripts/bcdr-drill.sh` is the repository wrapper for the same command. See [`docs/bc-dr-restore.md`](../../docs/bc-dr-restore.md).

For `remote.kind = "service"`, `mimir remote push --dry-run` and `mimir remote pull --dry-run` do not perform network I/O. They render the future service-adapter contract: workspace identity, local log and draft status, `service_operation`, append-only log prefix requirements, copy-only draft sync requirements, and the librarian-governed write boundary. Real service push/pull still returns an unsupported-adapter error.

Optional automatic backup is configured in `[remote]`:

```toml
[remote]
kind = "git"
url = "git@github.com:org/mimir-memory.git"
branch = "main"
auto_push_after_capture = true
```

Auto-push runs after post-session capture and after the configured librarian handoff (`off`, `defer`, `archive_raw`, or `process`) has returned. It calls the same verified `mimir remote push` implementation, records `remote_backup` in `capture-summary.json`, and converts failures into capture warnings so the child agent exit code remains authoritative. It never pulls remote state and never writes canonical memory directly.

Conflict remediation is intentionally conservative. `local_only` and `local_ahead` mean run `mimir remote push`; `remote_only` and `remote_ahead` mean run `mimir remote pull`; `synced` means no movement is needed. `diverged` means preserve both `canonical.log` files, decode both histories, and resolve through the librarian instead of overwriting append-only state. `draft_conflicts > 0` means a same-name draft JSON differs across local and remote checkout state; rename or quarantine one side before push/pull because draft sync is copy-only.

Optional native-memory sweeps are configured in `.mimir/config.toml`:

```toml
[native_memory]
claude = ["../.claude/projects/mimir/memory"]
codex = ["/home/me/.codex/memories/mimir.md"]
```

Only sources matching the launched agent are swept. Directory sources recurse through `.md`, `.markdown`, and `.txt`; empty files and missing roots are skipped. Swept files become `claude_memory` or `codex_memory` draft envelopes and still require librarian validation.

Before sweeping, each configured native-memory source receives a reason-coded adapter health check. Supported files/directories are swept, missing sources are counted without failing the wrapped session, and drifted sources such as unsupported file formats are skipped before any data is ingested. Drift appears in `capture-summary.json` as `native_memory.drifted_sources` plus per-source `adapter_health`; recover by correcting the `[native_memory]` path or adding an explicit adapter for that storage format.

After-capture librarian handoff is configured separately:

```toml
[librarian]
after_capture = "process" # off | defer | archive_raw | process
llm_binary = "claude"
llm_model = "claude-sonnet-4-6"
max_retries_per_record = 3
llm_timeout_secs = 120
processing_stale_secs = 3600
dedup_valid_at_window_secs = 86400
review_conflicts = false
```

`off` records only capture output. `defer` runs the librarian lifecycle safely without invoking an LLM, recovering stale `processing/` drafts and returning captured drafts to `pending/`. `process` is the rigorous LLM-backed default for repos that want structured post-session processing through `mimir-librarian`. `archive_raw` is a per-repo lightweight option that drains drafts without an LLM by committing raw pending-verification evidence plus provenance records through the normal append-only store. Archive mode is blocked before invocation when the draft directory or workspace log path is unavailable; process mode also requires the configured LLM binary. `after_capture`, `llm_binary`, and `llm_model` can be overridden per launch with `MIMIR_LIBRARIAN_AFTER_CAPTURE`, `MIMIR_LIBRARIAN_LLM_BINARY`, and `MIMIR_LIBRARIAN_LLM_MODEL`.

Capsule rehydration is read-only. Missing canonical logs are reported as cold-start status instead of being created, and trailing bytes past the last committed checkpoint are ignored with a capsule warning rather than repaired by the harness.

Intentional checkpoint capture uses `mimir checkpoint` and `MIMIR_SESSION_DRAFTS_DIR`. Wrapped agents may run:

```bash
mimir checkpoint --title "Short title" "Memory note for the librarian."
```

They may also write `.md`, `.markdown`, or `.txt` files directly under `MIMIR_SESSION_DRAFTS_DIR`. `mimir checkpoint --list` prints the current supported checkpoint files in the session inbox. After the child exits, non-empty supported files are submitted as v2 `agent_export` draft envelopes tagged `session_checkpoint` with file provenance. Unsupported files are ignored and counted in the capture summary.

Native hooks should call `mimir hook-context`, which is quiet outside wrapped sessions and prints concise Mimir context inside wrapped sessions. The helper now validates whether `MIMIR_SESSION_DRAFTS_DIR` is available before presenting the checkpoint route and reminds agents to checkpoint durable decisions before compaction or long handoff. Generated Claude hooks include both `SessionStart` and `PreCompact`; generated Codex hooks validate the checkpoint route at `SessionStart`, with `mimir checkpoint` remaining the explicit pre-compaction capture path. Hooks add context only; they do not write trusted memory.

## Native Setup

Generated setup artifacts can be installed explicitly:

```bash
mimir setup-agent status --agent claude --scope project
mimir setup-agent doctor --agent claude --scope project
mimir setup-agent install --agent claude --scope project --from "$MIMIR_AGENT_SETUP_DIR" --dry-run
mimir setup-agent install --agent claude --scope project --from "$MIMIR_AGENT_SETUP_DIR"
mimir setup-agent remove --agent claude --scope project
```

Use `--agent claude|codex`, `--scope project|user`, `--features all|skill|hook`, and optional `--dry-run`. Project scope writes Claude skills to `.claude/skills/mimir-checkpoint`, Claude hooks to `.claude/settings.json`, Codex skills to `.agents/skills/mimir-checkpoint`, and Codex hooks to `.codex/hooks.json` with `.codex/config.toml` feature enablement. User scope writes the same agent-native locations under `$HOME`. Status output includes `reason=` fields for missing, partial, installed, or conflicting surfaces. `doctor` is read-only and adds `doctor_status`, status/install/remove/context/checkpoint commands, and one `next_action` so setup can be verified in one command before an external operator launches a real agent. Installs preflight explicit `codex_hooks = false` before writing any setup surface, and removal refuses to delete a `mimir-checkpoint` skill directory whose `SKILL.md` is not Mimir-owned. The installer owns only the `mimir-checkpoint` skill directory and `mimir hook-context` hook entry.

Post-session capture also writes one v2 draft envelope under `drafts/pending/` with `source_surface = "agent_export"`. That draft contains only session metadata and provenance; it does not capture the child transcript. `capture-summary.json` records native sweep counts, session-checkpoint counts, missing source counts, staged post-session draft path, librarian handoff status (`skipped`, `blocked`, `deferred`, `processed`, or `failed`), remote backup status when configured, run counters when the librarian executes, and non-fatal capture warnings. Durable memory writes still flow through the librarian-governed draft path. The harness is the process boundary, not a direct canonical-store writer.

## License

[Apache-2.0](../../LICENSE).
