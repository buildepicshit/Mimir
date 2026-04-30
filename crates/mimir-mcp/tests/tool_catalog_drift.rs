//! Drift gates for the Phase 2.4 tool catalog.
//!
//! Two invariants enforced:
//!
//! 1. **Tool surface ↔ README sync.** Every tool registered with the
//!    rmcp `ToolRouter` must appear in `crates/mimir-mcp/README.md`'s
//!    tool table; every tool name in the README's tool table must
//!    exist in the registry. Catches the failure mode where a tool is
//!    added/renamed/removed without a corresponding README update.
//!
//! 2. **Supported-client scope.** No file under `crates/mimir-mcp/` or
//!    `docs/integrations/` may advertise an unsupported MCP client
//!    (Cursor, Cline, Continue, Windsurf, Copilot, etc.) as a targeted
//!    client. The 2026-04-24 mandate allows future adapter-mediated
//!    surfaces, but this crate is still the Claude MCP surface until a
//!    new adapter is specified. This test keeps client-surface changes
//!    explicit in code review.
//!
//! When either test fails, the failure message names the file and
//! the specific drift so the fix is unambiguous.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::manual_assert
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rmcp::{ClientHandler, ServiceExt};
use tokio::io::duplex;

use mimir_mcp::MimirServer;

#[derive(Default, Clone)]
struct NoopClient;

impl ClientHandler for NoopClient {}

/// Walk up from `CARGO_MANIFEST_DIR` to find the repo root.
fn find_repo_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if dir.join("Cargo.toml").exists() && dir.join("LICENSE").exists() {
            if let Ok(text) = std::fs::read_to_string(dir.join("Cargo.toml")) {
                if text.contains("[workspace]") {
                    return dir;
                }
            }
        }
        if !dir.pop() {
            panic!("could not find Mimir repo root walking up from CARGO_MANIFEST_DIR");
        }
    }
}

/// Spin up an `MimirServer` and ask it for the live tool list.
/// This is the source of truth — drift checks compare other artifacts
/// to *this*, not to a hardcoded list.
async fn fetch_registered_tools() -> Vec<String> {
    let (server_io, client_io) = duplex(8 * 1024);
    tokio::spawn(async move {
        let server = MimirServer::new(None, None, None);
        match server.serve(server_io).await {
            Ok(svc) => {
                let _ = svc.waiting().await;
            }
            Err(err) => eprintln!("test server failed: {err:?}"),
        }
    });
    let svc = Arc::new(
        NoopClient
            .serve(client_io)
            .await
            .expect("client handshake failed"),
    );
    let tools = svc.list_all_tools().await.expect("list_all_tools");
    tools.into_iter().map(|t| t.name.into_owned()).collect()
}

// ----------------------------------------------------------------
// Test 1 — README ↔ tool registry are in sync
// ----------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn readme_tool_table_matches_registry() {
    let registered: Vec<String> = {
        let mut v = fetch_registered_tools().await;
        v.sort_unstable();
        v
    };

    let repo_root = find_repo_root();
    let readme_path = repo_root.join("crates/mimir-mcp/README.md");
    let readme = std::fs::read_to_string(&readme_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", readme_path.display()));

    // Pull every backtick-wrapped `mimir_*` identifier out of the
    // README — those are the tool references the table renders. We
    // dedup and intersect with the registered set.
    let mut readme_mentions: Vec<String> = Vec::new();
    let mut search = readme.as_str();
    while let Some(start) = search.find("`mimir_") {
        let after = &search[start + 1..]; // skip opening backtick
        let Some(end) = after.find('`') else { break };
        let name = &after[..end];
        // Only catalog tool-name-shaped strings (snake_case
        // mimir_<word>{...}); skip e.g. `mimir_core::Pipeline::…`.
        if name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            readme_mentions.push(name.to_string());
        }
        search = &after[end + 1..];
    }
    readme_mentions.sort_unstable();
    readme_mentions.dedup();

    // Direction A: every registered tool must appear in the README.
    let missing_from_readme: Vec<&String> = registered
        .iter()
        .filter(|t| !readme_mentions.contains(t))
        .collect();
    assert!(
        missing_from_readme.is_empty(),
        "tool surface drift: these tools are registered with the rmcp \
         ToolRouter but are NOT mentioned in crates/mimir-mcp/README.md — \
         add a row to the README's `## Tools` table for each:\n  - {}\n\n\
         Registered: {registered:?}\nReadme mentions: {readme_mentions:?}",
        missing_from_readme
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  - ")
    );

    // Direction B: every README-mentioned tool name must exist in the
    // registry. (Filters out generic words like `mimir_status_field` —
    // we already constrained to snake_case above.)
    let extra_in_readme: Vec<&String> = readme_mentions
        .iter()
        .filter(|t| !registered.contains(t))
        .collect();
    assert!(
        extra_in_readme.is_empty(),
        "README references tool names that don't exist in the rmcp ToolRouter — \
         either rename the tool, drop the README mention, or this is a typo:\n  - {}\n\n\
         Registered: {registered:?}\nReadme mentions: {readme_mentions:?}",
        extra_in_readme
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  - ")
    );
}

// ----------------------------------------------------------------
// Test 2 — supported-client scope is enforced across mimir-mcp + docs
// ----------------------------------------------------------------

#[test]
fn mimir_mcp_surface_does_not_advertise_unsupported_clients() {
    let repo_root = find_repo_root();

    // Files under these paths must not promote off-spec MCP clients.
    // Either the file doesn't exist (skipped) or it must not contain
    // any of the forbidden phrases.
    let scoped_dirs = [
        repo_root.join("crates/mimir-mcp"),
        repo_root.join("docs/integrations"),
    ];

    // Forbidden phrases — case-insensitive substring match. Each
    // phrase is the way an unwary contributor might mention an
    // off-spec client. The phrase set is the load-bearing part of
    // this test; review it together with the supported-client boundary if
    // either changes.
    //
    // NOTE: bare client names ("Cursor", "Cline") are intentionally
    // NOT in the forbidden list — they may legitimately appear in
    // discussion of broad client behavior. The forbidden phrases
    // below specifically catch *targeting* / *supporting* Mimir on
    // those clients.
    let forbidden_phrases: &[&str] = &[
        "any mcp-compatible client",
        "any other mcp-compatible client",
        "every mcp-compatible client",
        "claude desktop, cursor",
        "claude desktop, cline",
        ", cursor, cline",
        ", cursor, continue",
        ", cline, continue",
        "windsurf, claude code",
        "windsurf, copilot",
        ".cursor/mcp.json",
        // Phase 5 marketplace surfaces tied to off-spec clients:
        "in-client directories: cursor",
        "in-client directories: cline",
    ];

    let mut violations: Vec<String> = Vec::new();
    for dir in &scoped_dirs {
        if !dir.exists() {
            continue;
        }
        walk_files(dir, &mut |path| {
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
            // Skip target/build artifacts + binary files.
            if path.components().any(|c| c.as_os_str() == "target") {
                return;
            }
            if !matches!(ext, "rs" | "md" | "toml" | "yml" | "yaml" | "json") {
                return;
            }
            let Ok(text) = std::fs::read_to_string(path) else {
                return;
            };
            // SELF-EXCLUSION: this test file enumerates the forbidden
            // phrases as *string literals*, which would otherwise
            // trip the test against itself. Skip.
            if path.ends_with("tool_catalog_drift.rs") {
                return;
            }
            let lowered = text.to_lowercase();
            for phrase in forbidden_phrases {
                if lowered.contains(phrase) {
                    violations.push(format!(
                        "{}: contains forbidden phrase {phrase:?}",
                        path.strip_prefix(&repo_root).unwrap_or(path).display()
                    ));
                }
            }
        });
    }

    assert!(
        violations.is_empty(),
        "unsupported client-surface violation — crates/mimir-mcp and docs/integrations \
         are currently the Claude MCP surface. The following files promote clients \
         that do not yet have a specified adapter. Either drop the unsupported mention, \
         or add the adapter/spec work and update the forbidden-phrase list in this test \
         so the scope change surfaces in code review.\n\nViolations ({} total):\n  - {}",
        violations.len(),
        violations.join("\n  - ")
    );
}

fn walk_files(dir: &Path, visit: &mut impl FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_files(&path, visit);
        } else {
            visit(&path);
        }
    }
}
