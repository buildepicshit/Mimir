//! `mimir-mcp` binary — stdio MCP server entrypoint.
//!
//! Wires [`mimir_mcp::MimirServer`] to stdin/stdout transport and
//! initialises a tracing subscriber that writes to **stderr** (never
//! stdout — stdout carries MCP protocol frames; any non-protocol byte
//! on stdout corrupts the JSON-RPC stream).
//!
//! Two pieces of context are computed at startup:
//!
//! - **Workspace id.** If `current_dir()` is inside a git repository,
//!   we hash the origin URL into a [`WorkspaceId`]. Failures are
//!   silent (`mimir_status.workspace_id` reports `null`).
//! - **Workspace store.** If `MIMIR_WORKSPACE_PATH` is set to a
//!   filesystem path, [`Store::open`] is called against it and the
//!   resulting store is handed to [`MimirServer::new`]. Read tools
//!   work immediately when the env var is set; writes require an
//!   `mimir_open_workspace` call to mint a lease. When the env var
//!   is unset, all tools start in the "no workspace open" state and
//!   the agent must call `mimir_open_workspace` to bring the
//!   workspace online.

use std::io;
use std::path::PathBuf;

use rmcp::{transport::stdio, ServiceExt};
use tracing_subscriber::EnvFilter;

use mimir_core::store::Store;
use mimir_core::workspace::WorkspaceId;
use mimir_mcp::MimirServer;

const USAGE: &str = "\
mimir-mcp — stdio Model Context Protocol server for Mimir.

Usage:
    mimir-mcp
    mimir-mcp --help
    mimir-mcp --version

The server speaks MCP JSON-RPC on stdin/stdout. Logs are emitted to
stderr so stdout remains protocol-only.
";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.as_slice(), [flag] if flag == "-h" || flag == "--help") {
        println!("{USAGE}");
        return Ok(());
    }
    if matches!(args.as_slice(), [flag] if flag == "--version") {
        println!("mimir-mcp {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if let Some(arg) = args.first() {
        return Err(format!("unexpected mimir-mcp argument `{arg}`; see --help").into());
    }

    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_ansi(false)
        .with_env_filter(
            EnvFilter::try_from_env("MIMIR_MCP_LOG").unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "mimir-mcp starting on stdio transport"
    );

    let workspace_id = std::env::current_dir()
        .ok()
        .and_then(|cwd| WorkspaceId::detect_from_path(&cwd).ok());

    if let Some(id) = workspace_id {
        tracing::info!(workspace_id = %id, "detected git workspace");
    } else {
        tracing::info!("no git workspace detected; operating without workspace context");
    }

    let log_path = std::env::var_os("MIMIR_WORKSPACE_PATH").map(PathBuf::from);
    let store = if let Some(path) = &log_path {
        match Store::open(path) {
            Ok(s) => {
                tracing::info!(log_path = %path.display(), "opened workspace store");
                Some(s)
            }
            Err(err) => {
                tracing::error!(
                    log_path = %path.display(),
                    ?err,
                    "failed to open workspace store; read tools will be unavailable"
                );
                None
            }
        }
    } else {
        tracing::info!(
            "MIMIR_WORKSPACE_PATH not set; tools will report no_workspace_open \
             until mimir_open_workspace is called"
        );
        None
    };

    let server = MimirServer::new(workspace_id, log_path, store);

    let service = server.serve(stdio()).await.inspect_err(|err| {
        tracing::error!(?err, "MCP serve failed");
    })?;

    service.waiting().await?;

    tracing::info!("mimir-mcp stopped");
    Ok(())
}
