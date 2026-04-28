# Claude Desktop integration

> **Status: authoritative** (Phase 2.4 of `docs/planning/2026-04-19-roadmap-to-prime-time.md`).

This document is the canonical setup recipe for the current **Claude Desktop** MCP surface. The 2026-04-24 mandate expands Mimir toward adapter-mediated Claude and Codex ingestion, but this integration remains Claude-specific until `scope-model.md` graduates into implementation; for the CLI, see [`claude-code-hook.md`](claude-code-hook.md).

## Prerequisites

1. **`mimir-mcp` on `PATH`.** Build from source until Phase 4 publishes to crates.io:
   ```bash
   git clone git@github.com:buildepicshit/Mimir.git
   cd Mimir
   cargo install --locked --path crates/mimir-mcp
   # `~/.cargo/bin/mimir-mcp` is on PATH if `~/.cargo/bin` is.
   mimir-mcp --version  # smoke
   ```
2. **A workspace path.** Mimir stores its canonical log at a filesystem path you choose. Convention: one log per project, alongside `.git/`:
   ```bash
   mkdir -p ~/projects/my-project/.mimir
   # The log itself is created on first open by mimir_open_workspace.
   ```

## `claude_desktop_config.json`

Open the config (created by Claude Desktop on first launch):

| OS | Path |
|---|---|
| macOS | `~/Library/Application Support/Claude/claude_desktop_config.json` |
| Windows | `%APPDATA%\Claude\claude_desktop_config.json` |
| Linux | `~/.config/Claude/claude_desktop_config.json` |

Add the `mimir` server:

```json
{
  "mcpServers": {
    "mimir": {
      "command": "mimir-mcp",
      "env": {
        "MIMIR_WORKSPACE_PATH": "/Users/you/projects/my-project/.mimir/canonical.log",
        "MIMIR_MCP_LOG": "info"
      }
    }
  }
}
```

`MIMIR_WORKSPACE_PATH` is **optional**. When set, the server opens the store at boot — read tools (`mimir_read`, `mimir_verify`, `mimir_list_episodes`, `mimir_render_memory`) work immediately. Writes still require an explicit `mimir_open_workspace` call to mint the lease.

When unset, all tools start in the "no workspace open" state; the agent must call `mimir_open_workspace` to bring the workspace online. This is the recommended pattern for multi-workspace setups.

Restart Claude Desktop after editing the config (the file is read once at launch).

## Verification

In a fresh Claude conversation:

> Use the `mimir` MCP server's `mimir_status` tool.

Expected response shape:

```json
{
  "workspace_id": "894be8c00dcbe056",
  "log_path": "/Users/you/projects/my-project/.mimir/canonical.log",
  "store_open": true,
  "lease_held": false,
  "lease_expires_at": null,
  "version": "0.1.0"
}
```

If `store_open` is `false`, either `MIMIR_WORKSPACE_PATH` was not set or the path was unreachable. Check Claude Desktop's **Developer → Open Logs** menu — `mimir-mcp` writes a stderr line on every startup naming the path it tried to open.

## First write

Pattern for the agent to follow on session start:

1. `mimir_open_workspace({ "log_path": "<absolute path>" })` — get back `{ lease_token, lease_expires_at }`.
2. `mimir_write({ "batch": "(episode :start :label \"design-session\")", "lease_token": "<token>" })` — open an episode.
3. ... do work, calling `mimir_write` for each batch of memories ...
4. `mimir_close_episode({ "lease_token": "<token>" })` — close the episode.
5. `mimir_release_workspace({ "lease_token": "<token>" })` — drop the lease at session end (optional — the lease auto-expires after 30 minutes).

The lease's 30-minute default TTL is comfortable for an interactive Claude session. Adjust via `ttl_seconds` on `mimir_open_workspace` (capped at 24h) or by setting `MIMIR_MCP_LEASE_TTL_SECONDS` in the config's `env` block at server startup.

## Logging

`mimir-mcp` writes structured `tracing` logs to **stderr** (Claude Desktop captures these in its dev console; stdout is reserved for the JSON-RPC wire). The default filter is `info`. Crank up via `MIMIR_MCP_LOG`:

```json
"env": {
  "MIMIR_WORKSPACE_PATH": "...",
  "MIMIR_MCP_LOG": "mimir_mcp=debug,mimir_core=info"
}
```

Mimir never logs payload content (memory data) at any level — only structural metadata (record counts, latencies, error categories). See [`docs/observability.md`](../observability.md) for the privacy invariant.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Server doesn't appear in Claude's tool list | Config JSON syntax error, or `mimir-mcp` not on `PATH` | Open dev console; check the parser error or "command not found" line |
| `mimir_status` reports `store_open: false` | `MIMIR_WORKSPACE_PATH` unset or unreachable | Set the env var, or call `mimir_open_workspace` explicitly |
| `mimir_write` returns `lease_held` | Previous session's lease still alive; default TTL is 30 min | Wait for expiry, or call `mimir_release_workspace` with the original token if you have it |
| `mimir_write` returns `lease_expired` | Session has been idle > 30 min | Call `mimir_open_workspace` again to mint a fresh lease |
