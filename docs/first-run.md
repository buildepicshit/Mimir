# First-Run Harness Walkthrough

This walkthrough verifies the transparent harness from a fresh checkout without launching a real agent. It uses `true` as a no-op child process so the Mimir wrapper, bootstrap, config, capsule, and draft paths can be checked safely.

Mimir is pre-1.0 active development. Config keys, CLI flags, setup artifacts, and capture behavior may change before the first stable release.

## 1. Build From Source

```bash
git clone https://github.com/buildepicshit/Mimir.git
cd Mimir
cargo build --workspace
cargo install --locked --path crates/mimir-harness
mimir --help
```

The installed binary is named `mimir`. It wraps an agent command as:

```bash
mimir [mimir flags] <agent> [agent args...]
```

Arguments after `<agent>` are passed to the child unchanged.

## 2. Try A No-Op Wrapped Session

From a test project directory:

```bash
mkdir -p /tmp/mimir-demo
cd /tmp/mimir-demo
mimir --project Demo true
```

On a first run without `.mimir/config.toml`, Mimir still launches the child process. The banner should say setup is pending and point to generated session artifacts:

```text
== True + Mimir ==
Mimir first-run setup is pending.
Guide: /tmp/mimir/sessions/<session-id>/agent-guide.md
Native setup artifacts: /tmp/mimir/sessions/<session-id>/setup
```

Check local status:

```bash
mimir doctor --project-root .
mimir status --project-root .
mimir health --project-root .
mimir context --project-root . --limit 8
mimir memory list --project-root . --limit 8
```

Expected shape:

```text
doctor_status=ok
doctor_readiness=action_required
config_status=missing
bootstrap_status=required
health_overall_zone=red
context_status=ok
memory_status=ok
next_action=mimir config init
```

## 3. Create Project Config

Preview config first:

```bash
mimir config init \
  --project-root . \
  --operator <operator-id> \
  --organization <organization-id> \
  --librarian-after-capture defer \
  --dry-run
```

Then write it:

```bash
mimir config init \
  --project-root . \
  --operator <operator-id> \
  --organization <organization-id> \
  --librarian-after-capture defer
```

The default config writes `.mimir/config.toml`, uses `.mimir/state` for local Mimir data, and leaves each repo on rigorous `process` mode unless overridden. `archive_raw` is the lightweight no-LLM drainage mode for repos that prefer fast post-session archival; this walkthrough uses `defer` when it wants to inspect pending draft lifecycle state without committing records.

Check status again:

```bash
mimir status --project-root .
mimir context --project-root . --limit 8
mimir memory list --project-root . --limit 8
```

Expected shape:

```text
config_status=ready
bootstrap_status=ready
data_root=/tmp/mimir-demo/.mimir/state
drafts_dir=/tmp/mimir-demo/.mimir/state/drafts
context_schema=mimir.context.v1
memory_schema=mimir.memory.v1
```

## 4. Inspect Native Agent Setup

Mimir does not silently mutate persistent Claude or Codex settings. It reports setup status and writes installable setup artifacts during wrapped launches.

```bash
mimir setup-agent status --agent codex --scope project --project-root .
mimir setup-agent doctor --agent codex --scope project --project-root .
mimir setup-agent status --agent claude --scope project --project-root .
```

After a real wrapped `mimir codex ...` or `mimir claude ...` launch, the session banner prints the native setup artifact directory. The wrapped child also receives it as `MIMIR_AGENT_SETUP_DIR`. Inspect the generated artifacts, then dry-run install from the printed path:

```bash
mimir setup-agent install \
  --agent codex \
  --scope project \
  --from /tmp/mimir/sessions/<session-id>/setup \
  --dry-run
```

Run without `--dry-run` only after reviewing the generated skill and hook artifacts.

## 5. Run Again With Config

```bash
mimir --project Demo true
```

Expected banner shape:

```text
== True + Mimir ==
Mimir memory wrapper active.
Checkpoint durable session memory with: mimir checkpoint --title "Short title" "Memory note"
Guide: /tmp/mimir/sessions/<session-id>/agent-guide.md
Native setup artifacts: /tmp/mimir/sessions/<session-id>/setup
```

Post-session capture stages draft envelopes under the configured draft store. Check them with:

```bash
mimir drafts status --project-root .
mimir drafts list --state pending --project-root .
```

Drafts remain untrusted until the librarian validates and commits them. The harness is a launch and capture boundary, not a direct canonical-memory writer.

## 6. Launch Real Agents

Once the no-op path behaves as expected, replace `true` with the agent command:

```bash
mimir --project Demo codex --model gpt-5.4
mimir --project Demo claude --r
```

The native terminal UI is preserved. Mimir consumes only its own pre-agent flags and forwards the rest to the child agent.

## Artifact Map

| Artifact | Default location | Purpose |
|---|---|---|
| Project config | `.mimir/config.toml` | Project-local Mimir settings. |
| Data root | `.mimir/state` | Local governed logs, drafts, and recovery state. |
| Draft store | `.mimir/state/drafts` | Pending/processing/terminal draft envelopes. |
| Session capsule | `/tmp/mimir/sessions/<session-id>/capsule.json` | Structured launch metadata and memory status. |
| Agent guide | `/tmp/mimir/sessions/<session-id>/agent-guide.md` | Wrapped-agent instructions for the session. |
| Native setup artifacts | `/tmp/mimir/sessions/<session-id>/setup` | Reviewable Claude/Codex skill and hook setup files. |

## Troubleshooting

- `bootstrap_status=required`: run `mimir config init`.
- `workspace_status=unavailable`: the project has no detectable Git workspace yet, or storage is not configured.
- `native_setup_*_project=missing`: run `mimir setup-agent status`, inspect generated setup artifacts from a wrapped launch, then install explicitly.
- `librarian after_capture=archive_raw` is blocked: configure `storage.data_root` / `drafts.dir` and launch from a detectable Git workspace.
- `librarian after_capture=process` is blocked: use `archive_raw` or `defer` until the draft directory, workspace log path, and selected adapter binary are all available.
