# Claude Code integration

> **Status: authoritative Claude MCP setup recipe.**

This document is the canonical setup recipe for the current **Claude Code** MCP surface. The 2026-04-24 mandate expands Mimir toward adapter-mediated Claude and Codex ingestion, but this integration remains Claude-specific until `scope-model.md` graduates into implementation. For Claude Desktop, see [`claude-desktop-config.md`](claude-desktop-config.md).

## Prerequisites

1. **`mimir-mcp` on `PATH`** — same as Claude Desktop:
   ```bash
   cargo install --locked --path crates/mimir-mcp
   ```
2. **A per-project workspace path** — convention is `<project>/.mimir/canonical.log`.

## Register the server

```bash
cd ~/projects/my-project
claude mcp add mimir mimir-mcp \
  --env MIMIR_WORKSPACE_PATH="$PWD/.mimir/canonical.log" \
  --env MIMIR_MCP_LOG="info"
```

Or edit `~/.claude/mcp.json` directly:

```json
{
  "mcpServers": {
    "mimir": {
      "command": "mimir-mcp",
      "env": {
        "MIMIR_WORKSPACE_PATH": "${workspaceFolder}/.mimir/canonical.log",
        "MIMIR_MCP_LOG": "info"
      }
    }
  }
}
```

`${workspaceFolder}` is expanded by Claude Code at server-spawn time so the same config works across every project you open.

Verify the server is registered:

```bash
claude mcp list
# mimir   running   mimir-mcp   <pid>
```

## Hook recipes

Claude Code's hook system runs shell commands at lifecycle events. Mimir benefits from four:

### `SessionStart` — open the workspace + start an episode

`~/.claude/hooks/session-start.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
LOG_PATH="${PWD}/.mimir/canonical.log"
mkdir -p "$(dirname "$LOG_PATH")"

# Open and capture the lease.
LEASE=$(claude mcp call mimir mimir_open_workspace \
  --json "{\"log_path\":\"${LOG_PATH}\",\"ttl_seconds\":7200}" \
  | jq -r .lease_token)
echo "$LEASE" > /tmp/mimir-lease-${PPID}

# Open an episode named after the current git branch + ISO timestamp.
BRANCH=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo "no-git")
TS=$(date -u +%Y-%m-%dT%H:%M:%SZ)
claude mcp call mimir mimir_write --json \
  "{\"batch\":\"(episode :start :label \\\"${BRANCH}@${TS}\\\")\",\"lease_token\":\"${LEASE}\"}"
```

Register in `~/.claude/settings.json`:

```json
{
  "hooks": {
    "SessionStart": "~/.claude/hooks/session-start.sh"
  }
}
```

The 2-hour `ttl_seconds` covers a long Claude Code session without needing mid-session re-open. (Default is 30 min; capped at 24h.)

### `PreToolUse` — record what's about to happen

`~/.claude/hooks/pre-tool-use.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
LEASE=$(cat /tmp/mimir-lease-${PPID} 2>/dev/null) || exit 0  # silently noop if no session
TOOL_NAME="${CLAUDE_TOOL_NAME:-unknown}"
TOOL_ARGS="${CLAUDE_TOOL_ARGS_JSON:-{}}"

# Lightweight episodic memory: the agent ran tool X with these args.
# `:src @observation` because we directly observed the call.
ESCAPED=$(echo "$TOOL_ARGS" | jq -Rs .)
TS=$(date -u +%Y-%m-%dT%H:%M:%SZ)
claude mcp call mimir mimir_write --json \
  "{\"batch\":\"(epi @${TOOL_NAME} @tool_call () @claude_code :at ${TS} :obs ${TS} :src @observation :c 1.0)\",\"lease_token\":\"${LEASE}\"}" \
  >/dev/null
```

### `PostToolUse` — record the outcome

Symmetric to `PreToolUse` but records the result. Useful for procedural memory ("when X failed with Y, the fix was Z"):

```bash
#!/usr/bin/env bash
set -euo pipefail
LEASE=$(cat /tmp/mimir-lease-${PPID} 2>/dev/null) || exit 0
TOOL_NAME="${CLAUDE_TOOL_NAME:-unknown}"
RESULT="${CLAUDE_TOOL_RESULT:-ok}"
TS=$(date -u +%Y-%m-%dT%H:%M:%SZ)
claude mcp call mimir mimir_write --json \
  "{\"batch\":\"(epi @${TOOL_NAME} @tool_result () @claude_code :at ${TS} :obs ${TS} :src @observation :c 1.0)\",\"lease_token\":\"${LEASE}\"}" \
  >/dev/null
```

### `SessionStop` — close the episode + release the lease

`~/.claude/hooks/session-stop.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
LEASE=$(cat /tmp/mimir-lease-${PPID} 2>/dev/null) || exit 0

claude mcp call mimir mimir_close_episode --json \
  "{\"lease_token\":\"${LEASE}\"}" >/dev/null
claude mcp call mimir mimir_release_workspace --json \
  "{\"lease_token\":\"${LEASE}\"}" >/dev/null
rm -f /tmp/mimir-lease-${PPID}
```

Register the remaining three hooks:

```json
{
  "hooks": {
    "SessionStart": "~/.claude/hooks/session-start.sh",
    "PreToolUse": "~/.claude/hooks/pre-tool-use.sh",
    "PostToolUse": "~/.claude/hooks/post-tool-use.sh",
    "SessionStop": "~/.claude/hooks/session-stop.sh"
  }
}
```

## Verification

```bash
# In a fresh Claude Code session:
claude mcp call mimir mimir_status
# Expect: { "store_open": true, "lease_held": true, "lease_expires_at": "...", ... }

# Replay what the session has captured:
claude mcp call mimir mimir_read --json '{"query":"(query :kind epi :limit 10)"}'
```

## Logging

`mimir-mcp` writes to **stderr** under Claude Code; check the `.claude/logs/mcp-mimir.log` file (or `claude mcp logs mimir` if your version supports it). Privacy invariant: structural metadata only, never payload content. See [`docs/observability.md`](../observability.md).

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Hooks silently no-op | `/tmp/mimir-lease-${PPID}` doesn't exist | `SessionStart` hook didn't run; check `claude mcp logs mimir` |
| `lease_held` on next `SessionStart` | Previous session crashed without `SessionStop` | Wait for TTL (default 30 min) or restart `mimir-mcp` to discard in-memory state |
| `claude mcp call` not found | Older Claude Code version | Upgrade Claude Code, or use `claude tools call` |
| Hook scripts hang | Mimir log file locked | Mimir doesn't lock; check that no other process is reading the log file with an exclusive flock |
