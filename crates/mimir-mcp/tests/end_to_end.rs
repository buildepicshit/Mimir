//! End-to-end protocol tests for `mimir-mcp`. Exercises the full
//! `initialize` → `tools/list` → `tools/call(mimir_status)` →
//! response path through `tokio::io::duplex` (in-memory bidirectional
//! pipe), so the protocol surface gets tested without spawning a
//! child process.
//!
//! Phase 2.3 ships nine tools; this file asserts the bedrock
//! protocol-level behaviour shared across all of them:
//!
//! - The protocol handshake completes (no `ServiceError`).
//! - `tools/list` returns exactly the expected set (catches
//!   silent rename / removal / addition).
//! - `tools/call(mimir_status)` returns text content that
//!   round-trips as a [`StatusReport`] JSON value with the
//!   constructed-time fields (workspace null, store closed,
//!   no lease).
//!
//! Per-tool behaviour lives in `tests/read_tools.rs` (`mimir_read`,
//! `mimir_verify`, `mimir_list_episodes`, `mimir_render_memory`)
//! and `tests/write_tools.rs` (`mimir_open_workspace`,
//! `mimir_write`, `mimir_close_episode`,
//! `mimir_release_workspace` + lease state machine). Tool-vs-doc
//! surface drift is enforced by `tests/tool_catalog_drift.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use rmcp::{model::CallToolRequestParams, ClientHandler, ServiceExt};
use tokio::io::duplex;

use mimir_mcp::{MimirServer, StatusReport};

// Default ClientHandler is sufficient for the protocol-path smoke we
// need; rmcp's blanket get_info() advertises the test as an unnamed
// client with default capabilities. No tools are registered on the
// client side, which is correct — the client only initiates calls
// against the server's tool surface.
#[derive(Default, Clone)]
struct NoopClient;

impl ClientHandler for NoopClient {}

async fn spin_up() -> (Arc<rmcp::service::RunningService<rmcp::RoleClient, NoopClient>>,) {
    let (server_io, client_io) = duplex(8 * 1024);

    // Spawn the server side. It owns the server MimirServer and
    // serves over the duplex's server end.
    tokio::spawn(async move {
        let server = MimirServer::new(None, None, None);
        match server.serve(server_io).await {
            Ok(svc) => {
                let _ = svc.waiting().await;
            }
            Err(err) => {
                eprintln!("test server failed: {err:?}");
            }
        }
    });

    // Build the client side, do the protocol handshake.
    let client = NoopClient;
    let service = client
        .serve(client_io)
        .await
        .expect("client handshake failed");
    (Arc::new(service),)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handshake_succeeds() {
    let (_svc,) = spin_up().await;
    // Reaching this line means initialize → initialized round-trip
    // completed without error. ServiceExt::serve drives the
    // handshake.
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tools_list_returns_mimir_status() {
    let (svc,) = spin_up().await;
    let tools = svc
        .list_all_tools()
        .await
        .expect("list_all_tools must succeed");
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(
        tools.len(),
        9,
        "phase 2.3 ships exactly nine tools; got {} ({names:?})",
        tools.len()
    );
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        vec![
            "mimir_close_episode",
            "mimir_list_episodes",
            "mimir_open_workspace",
            "mimir_read",
            "mimir_release_workspace",
            "mimir_render_memory",
            "mimir_status",
            "mimir_verify",
            "mimir_write",
        ]
    );
    let status = tools
        .iter()
        .find(|t| t.name == "mimir_status")
        .expect("mimir_status must be in the tool list");
    let description = status
        .description
        .as_ref()
        .map(AsRef::as_ref)
        .unwrap_or_default();
    assert!(
        !description.is_empty(),
        "mimir_status must carry a non-empty description for client UI"
    );
    assert!(
        description.to_lowercase().contains("workspace"),
        "mimir_status description should mention workspace context — got: {description}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_descriptions_are_runtime_compact() {
    let (svc,) = spin_up().await;
    let tools = svc
        .list_all_tools()
        .await
        .expect("list_all_tools must succeed");

    for tool in &tools {
        let description = tool
            .description
            .as_ref()
            .map(AsRef::as_ref)
            .unwrap_or_default();
        assert!(
            description.len() <= 100,
            "{} description should stay <= 100 chars, got {}: {description}",
            tool.name,
            description.len()
        );
    }

    let status = tools
        .iter()
        .find(|t| t.name == "mimir_status")
        .expect("mimir_status must be in the tool list");
    let status_description = status
        .description
        .as_ref()
        .map(AsRef::as_ref)
        .unwrap_or_default();
    assert!(
        status_description.len() <= 50,
        "mimir_status description should stay <= 50 chars, got {}: {status_description}",
        status_description.len()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn call_mimir_status_returns_parseable_report() {
    let (svc,) = spin_up().await;
    let result = svc
        .call_tool(CallToolRequestParams::new("mimir_status"))
        .await
        .expect("call_tool must succeed");

    assert_ne!(
        result.is_error,
        Some(true),
        "mimir_status must not report tool error; result = {result:?}"
    );

    let content = &result.content;
    assert_eq!(
        content.len(),
        1,
        "mimir_status returns a single text content block; got {}",
        content.len()
    );

    let raw = content[0]
        .as_text()
        .expect("phase 2.1: mimir_status returns text content")
        .text
        .clone();
    assert!(
        !raw.contains('\n'),
        "MCP runtime JSON should be compact, got pretty output: {raw:?}"
    );
    let report: StatusReport =
        serde_json::from_str(&raw).expect("mimir_status text must parse as StatusReport JSON");
    assert_eq!(
        raw,
        serde_json::to_string(&report).expect("serialize parsed report"),
        "runtime JSON should use serde_json::to_string shape"
    );

    // Server constructed with no workspace -> both nullable fields null.
    assert_eq!(report.workspace_id, None);
    assert_eq!(report.log_path, None);
    assert!(
        !report.store_open,
        "no store passed to spin_up — should be closed"
    );
    assert!(!report.lease_held, "no lease minted yet — should be false");
    assert_eq!(report.lease_expires_at, None);
    // Version reported is the mimir-mcp crate version at compile time.
    assert!(
        !report.version.is_empty(),
        "version field must not be empty"
    );
}
