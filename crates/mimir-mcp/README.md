# mimir-mcp

[Model Context Protocol](https://modelcontextprotocol.io/) server for [Mimir](https://github.com/buildepicshit/Mimir). Makes the current workspace-local canonical store reachable from Claude — both **Claude Desktop** (via `claude_desktop_config.json`) and **Claude Code** (via `claude mcp add` or hooks).

> **Pre-1.0 status.** This crate is part of Mimir's active-development tree. MCP tool schemas and setup recipes may change before v1. Public crates.io releases wait for the first alpha.

> **Current MCP surface.** Mimir's 2026-04-24 mandate targets adapter-mediated agent surfaces: Claude and Codex first, future agents through draft/retrieval adapters. This crate remains the Claude MCP surface until `scope-model.md` graduates into implementation. Other clients may technically connect because MCP is standard, but they are not supported by this crate until an adapter is explicitly specified.

> **Current tool status.** Full read + write surface live (9 tools). See [`docs/README.md`](https://github.com/buildepicshit/Mimir/blob/main/docs/README.md) for current public documentation.

## Install

Until the first alpha release, build from the repository root:

```bash
cargo install --locked --path crates/mimir-mcp
```

## Configure

### Claude Desktop

```json
{
  "mcpServers": {
    "mimir": {
      "command": "mimir-mcp"
    }
  }
}
```

(`~/Library/Application Support/Claude/claude_desktop_config.json` on macOS, `%APPDATA%\Claude\claude_desktop_config.json` on Windows.)

### Claude Code (CLI)

```bash
claude mcp add mimir mimir-mcp
```

Detailed setup recipes (including `PreToolUse` / `PostToolUse` / `SessionStart` / `SessionStop` hooks) live at [`docs/integrations/claude-code-hook.md`](../../docs/integrations/claude-code-hook.md). Claude Desktop config details are at [`docs/integrations/claude-desktop-config.md`](../../docs/integrations/claude-desktop-config.md).

## Tools (Phase 2.3 — full read + write surface)

| Name | Effect | Inputs | Output |
|---|---|---|---|
| `mimir_status` | Server health: workspace id, log path, store-open + lease-held flags. | — | JSON `{ workspace_id, log_path, store_open, lease_held, lease_expires_at, version }` |
| `mimir_read` | Execute a Lisp `(query …)` form against the open workspace. Returns matched records rendered as Lisp + read-protocol flags + snapshot watermarks. | `query: String` | JSON `ReadResponse` |
| `mimir_verify` | Read-only integrity check on a canonical log: decode count, checkpoints, dangling-symbol report, `tail_type`, and corrupt-tail `tail_error`. | `log_path?: String` | JSON `VerifyReportJson` |
| `mimir_list_episodes` | Paginated session history, `committed_at`-ordered. | `limit?: usize`, `offset?: usize` | JSON `Vec<EpisodeRow>` |
| `mimir_render_memory` | Render the single record matched by a query as Lisp; errors on multi-match. | `query: String` | text |
| `mimir_open_workspace` | Open or create a canonical log; mint a 30-min write lease. Errors with `lease_held` if one is already alive. | `log_path: String`, `ttl_seconds?: u64` | JSON `OpenWorkspaceResponse` |
| `mimir_write` | Commit a Lisp batch. Lease-required. | `batch: String`, `lease_token: String` | JSON `WriteResponse` |
| `mimir_close_episode` | Convenience wrapper for `(episode :close)`. Lease-required. | `lease_token: String` | JSON `WriteResponse` |
| `mimir_release_workspace` | Drop the lease. Store stays open for reads. | `lease_token: String` | JSON `ReleaseWorkspaceResponse` |

## Logging

`mimir-mcp` writes structured `tracing` logs to **stderr** (stdout is reserved for the MCP JSON-RPC wire). Filter via the `MIMIR_MCP_LOG` environment variable using [`EnvFilter`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/struct.EnvFilter.html) syntax:

```bash
MIMIR_MCP_LOG=mimir_mcp=debug,mimir_core=info mimir-mcp
```

Default filter is `info`. The server never logs payload content (memory data) at any level — only structural metadata (record counts, latencies, error categories). See [`docs/observability.md`](https://github.com/buildepicshit/Mimir/blob/main/docs/observability.md) for the privacy invariant.

## Library use

`mimir-mcp` is also a library so it can be embedded in tests and custom transports:

```rust,no_run
use mimir_mcp::MimirServer;
use rmcp::{ServiceExt, transport::stdio};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Args: workspace_id, log_path, store. Passing None/None/None
    // starts the server in the "no workspace open" state; callers
    // bring a workspace online with `mimir_open_workspace`.
    let server = MimirServer::new(None, None, None);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
```

Integration tests in `tests/end_to_end.rs` use `tokio::io::duplex` for in-memory protocol round-trips.

## License

Apache-2.0 — see [LICENSE](LICENSE).
