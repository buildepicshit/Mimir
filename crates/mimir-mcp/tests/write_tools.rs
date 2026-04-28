//! End-to-end tests for the Phase 2.3 write tools and the workspace
//! lease state machine. Each test starts a server with no workspace
//! open (the production default), then drives the open → write →
//! release lifecycle through `tokio::io::duplex`.
//!
//! Coverage:
//!
//! - `mimir_open_workspace` happy path returns a valid lease token,
//!   ISO-8601 expiry, and (optional) workspace id.
//! - `mimir_write` happy path (a single sem record) returns the
//!   auto-generated episode id and ISO-8601 commit time.
//! - `mimir_write` rejected with `no_lease` when no lease is held.
//! - `mimir_write` rejected with `lease_token_mismatch` when the
//!   token is wrong.
//! - `mimir_open_workspace` rejected with `lease_held` while a
//!   non-expired lease is alive.
//! - `mimir_open_workspace` rejected with `workspace_lock_held` when
//!   another process already holds the shared workspace write lock.
//! - `mimir_release_workspace` releases the lease so a fresh open
//!   succeeds, but the store stays open so reads still work.
//! - `mimir_release_workspace` rejects mismatched tokens.
//! - `mimir_close_episode` happy path commits `(episode :close)`.
//! - `mimir_status` reports `lease_held: true` after open and
//!   `lease_held: false` after release.
//! - Lease TTL bounds: `ttl_seconds: 0` rejected, `ttl_seconds`
//!   above the cap rejected.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use rmcp::{
    model::{CallToolRequestParams, CallToolResult},
    ClientHandler, ServiceExt,
};
use serde_json::Value as JsonValue;
use tempfile::TempDir;
use tokio::io::duplex;

use mimir_core::WorkspaceWriteLock;
use mimir_mcp::{
    Clock, MimirServer, OpenWorkspaceResponse, ReleaseWorkspaceResponse, StatusReport,
    WriteResponse,
};

#[derive(Default, Clone)]
struct NoopClient;

impl ClientHandler for NoopClient {}

/// Test-only virtual clock: holds a `SystemTime` that tests advance
/// explicitly. Makes lease-expiry assertions deterministic (no
/// `tokio::time::sleep` against the real wall clock), which closes
/// the clock-injection follow-up filed as issue #2 alongside the
/// 2026-04-20 post-cutover cleanup PR.
#[derive(Debug, Clone)]
struct TestClock {
    current: Arc<Mutex<SystemTime>>,
}

impl TestClock {
    fn new(start: SystemTime) -> Self {
        Self {
            current: Arc::new(Mutex::new(start)),
        }
    }

    fn advance(&self, by: Duration) {
        let mut t = self.current.lock().expect("TestClock poisoned");
        *t += by;
    }
}

impl Clock for TestClock {
    fn now(&self) -> SystemTime {
        *self.current.lock().expect("TestClock poisoned")
    }
}

async fn spin_up_empty() -> (
    Arc<rmcp::service::RunningService<rmcp::RoleClient, NoopClient>>,
    TempDir,
    PathBuf,
) {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let log_path = tempdir.path().join("canonical.log");
    let (server_io, client_io) = duplex(16 * 1024);
    tokio::spawn(async move {
        let server = MimirServer::new(None, None, None);
        match server.serve(server_io).await {
            Ok(svc) => {
                let _ = svc.waiting().await;
            }
            Err(err) => eprintln!("test server failed: {err:?}"),
        }
    });
    let client = NoopClient;
    let service = client
        .serve(client_io)
        .await
        .expect("client handshake failed");
    (Arc::new(service), tempdir, log_path)
}

/// Like [`spin_up_empty`] but constructs the server with an injected
/// [`TestClock`], returned alongside so the test can advance the
/// virtual clock in-process. Used by the lease-expiry test, which
/// needs to observe expired-lease behaviour without a wall-clock
/// sleep.
async fn spin_up_empty_with_clock(
    clock: TestClock,
) -> (
    Arc<rmcp::service::RunningService<rmcp::RoleClient, NoopClient>>,
    TempDir,
    PathBuf,
) {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let log_path = tempdir.path().join("canonical.log");
    let (server_io, client_io) = duplex(16 * 1024);
    let clock_arc: Arc<dyn Clock> = Arc::new(clock);
    tokio::spawn(async move {
        let server = MimirServer::with_clock(None, None, None, clock_arc);
        match server.serve(server_io).await {
            Ok(svc) => {
                let _ = svc.waiting().await;
            }
            Err(err) => eprintln!("test server failed: {err:?}"),
        }
    });
    let client = NoopClient;
    let service = client
        .serve(client_io)
        .await
        .expect("client handshake failed");
    (Arc::new(service), tempdir, log_path)
}

fn parse_text<T: serde::de::DeserializeOwned>(result: &CallToolResult) -> T {
    let raw = result.content[0]
        .as_text()
        .expect("text content")
        .text
        .clone();
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse JSON: {e}\nraw: {raw}"))
}

async fn open_workspace(
    svc: &rmcp::service::RunningService<rmcp::RoleClient, NoopClient>,
    log_path: &Path,
    ttl_seconds: Option<u64>,
) -> Result<OpenWorkspaceResponse, String> {
    let mut args = serde_json::json!({"log_path": log_path.to_string_lossy()});
    if let Some(t) = ttl_seconds {
        args["ttl_seconds"] = serde_json::json!(t);
    }
    let result = svc
        .call_tool(
            CallToolRequestParams::new("mimir_open_workspace")
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await;
    match result {
        Ok(r) if r.is_error == Some(true) => {
            Err(r.content[0].as_text().expect("text").text.clone())
        }
        Ok(r) => Ok(parse_text::<OpenWorkspaceResponse>(&r)),
        Err(err) => Err(format!("{err}")),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_workspace_returns_valid_lease() {
    let (svc, _tempdir, log_path) = spin_up_empty().await;
    let resp = open_workspace(&svc, &log_path, None)
        .await
        .expect("open_workspace must succeed");
    assert_eq!(resp.lease_token.len(), 32, "32-char hex token");
    assert!(
        resp.lease_token
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "lowercase hex"
    );
    assert!(!resp.lease_expires_at.is_empty());
    assert_eq!(resp.log_path, log_path.to_string_lossy());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_workspace_rejects_external_workspace_lock() {
    let (svc, _tempdir, log_path) = spin_up_empty().await;
    let _lock = WorkspaceWriteLock::acquire_for_log(&log_path).expect("hold external lock");

    let err = open_workspace(&svc, &log_path, None)
        .await
        .expect_err("external lock must reject open");

    assert!(
        err.contains("workspace_lock_held"),
        "expected workspace_lock_held; got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_commits_a_batch_and_returns_episode_id() {
    let (svc, _tempdir, log_path) = spin_up_empty().await;
    let lease = open_workspace(&svc, &log_path, None).await.expect("open");
    let result = svc
        .call_tool(
            CallToolRequestParams::new("mimir_write").with_arguments(
                serde_json::json!({
                    "batch": "(sem @alice @knows @bob :src @observation :c 0.9 :v 2024-01-01)",
                    "lease_token": lease.lease_token,
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await
        .expect("call_tool");
    assert_ne!(
        result.is_error,
        Some(true),
        "mimir_write must succeed; got {result:?}"
    );
    let resp: WriteResponse = parse_text(&result);
    assert!(resp.episode_id.starts_with("__ep_"));
    assert!(!resp.committed_at.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_rejected_without_lease() {
    let (svc, _tempdir, log_path) = spin_up_empty().await;
    open_workspace(&svc, &log_path, None).await.expect("open");
    // Wrong token (no lease minted in some other code path; we
    // mint one above to ensure store is open, but pass a bogus
    // token here).
    let result = svc
        .call_tool(
            CallToolRequestParams::new("mimir_write").with_arguments(
                serde_json::json!({
                    "batch": "(sem @alice @knows @bob :src @observation :c 0.9 :v 2024-01-01)",
                    "lease_token": "0".repeat(32),
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await;
    let err = match result {
        Ok(r) => {
            assert_eq!(r.is_error, Some(true));
            r.content[0].as_text().expect("text").text.clone()
        }
        Err(e) => format!("{e}"),
    };
    assert!(
        err.contains("lease_token_mismatch"),
        "expected lease_token_mismatch; got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_rejected_with_no_lease_when_none_minted() {
    // Open workspace via the read-side env var path: construct a
    // server with Some(Store) but no lease. We fake this by opening
    // and immediately releasing the lease, leaving the store open.
    let (svc, _tempdir, log_path) = spin_up_empty().await;
    let lease = open_workspace(&svc, &log_path, None).await.expect("open");
    let _ = svc
        .call_tool(
            CallToolRequestParams::new("mimir_release_workspace").with_arguments(
                serde_json::json!({"lease_token": lease.lease_token})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .expect("release");

    let result = svc
        .call_tool(
            CallToolRequestParams::new("mimir_write").with_arguments(
                serde_json::json!({
                    "batch": "(sem @alice @knows @bob :src @observation :c 0.9 :v 2024-01-01)",
                    "lease_token": "0".repeat(32),
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await;
    let err = match result {
        Ok(r) => {
            assert_eq!(r.is_error, Some(true));
            r.content[0].as_text().expect("text").text.clone()
        }
        Err(e) => format!("{e}"),
    };
    assert!(err.contains("no_lease"), "expected no_lease; got: {err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn second_open_returns_lease_held_while_first_alive() {
    let (svc, _tempdir, log_path) = spin_up_empty().await;
    open_workspace(&svc, &log_path, None)
        .await
        .expect("first open");
    let err = open_workspace(&svc, &log_path, None)
        .await
        .expect_err("second open must fail");
    assert!(
        err.contains("lease_held"),
        "expected lease_held; got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn release_then_reopen_succeeds_and_store_stays_open_for_reads() {
    let (svc, _tempdir, log_path) = spin_up_empty().await;
    let lease = open_workspace(&svc, &log_path, None).await.expect("open");

    let release_result = svc
        .call_tool(
            CallToolRequestParams::new("mimir_release_workspace").with_arguments(
                serde_json::json!({"lease_token": lease.lease_token})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .expect("release call");
    assert_ne!(release_result.is_error, Some(true));
    let release_resp: ReleaseWorkspaceResponse = parse_text(&release_result);
    assert!(release_resp.released);

    // Store still open after release — reads work.
    let status = svc
        .call_tool(CallToolRequestParams::new("mimir_status"))
        .await
        .expect("status call");
    let report: StatusReport = parse_text(&status);
    assert!(report.store_open, "store stays open after release");
    assert!(!report.lease_held, "lease dropped");

    // Fresh open succeeds.
    let _ = open_workspace(&svc, &log_path, None)
        .await
        .expect("fresh open after release");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn release_rejects_mismatched_token() {
    let (svc, _tempdir, log_path) = spin_up_empty().await;
    open_workspace(&svc, &log_path, None).await.expect("open");

    let result = svc
        .call_tool(
            CallToolRequestParams::new("mimir_release_workspace").with_arguments(
                serde_json::json!({"lease_token": "0".repeat(32)})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await;
    let err = match result {
        Ok(r) => {
            assert_eq!(r.is_error, Some(true));
            r.content[0].as_text().expect("text").text.clone()
        }
        Err(e) => format!("{e}"),
    };
    assert!(
        err.contains("lease_token_mismatch"),
        "expected lease_token_mismatch; got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_episode_commits_close_form() {
    let (svc, _tempdir, log_path) = spin_up_empty().await;
    let lease = open_workspace(&svc, &log_path, None).await.expect("open");

    // Open an episode first by writing a sem with an episode-start.
    let _ = svc
        .call_tool(
            CallToolRequestParams::new("mimir_write").with_arguments(
                serde_json::json!({
                    "batch": "(episode :start :label \"design-session\")\n(sem @alice @knows @bob :src @observation :c 0.9 :v 2024-01-01)",
                    "lease_token": lease.lease_token.clone(),
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await
        .expect("write")
        ;

    let close_result = svc
        .call_tool(
            CallToolRequestParams::new("mimir_close_episode").with_arguments(
                serde_json::json!({
                    "lease_token": lease.lease_token.clone(),
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await
        .expect("close call");
    assert_ne!(
        close_result.is_error,
        Some(true),
        "close must succeed; got {close_result:?}"
    );
    let resp: WriteResponse = parse_text(&close_result);
    assert!(resp.episode_id.starts_with("__ep_"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_reflects_lease_state_changes() {
    let (svc, _tempdir, log_path) = spin_up_empty().await;

    let pre = svc
        .call_tool(CallToolRequestParams::new("mimir_status"))
        .await
        .expect("status");
    let pre_report: StatusReport = parse_text(&pre);
    assert!(!pre_report.store_open);
    assert!(!pre_report.lease_held);

    let lease = open_workspace(&svc, &log_path, None).await.expect("open");

    let mid = svc
        .call_tool(CallToolRequestParams::new("mimir_status"))
        .await
        .expect("status");
    let mid_report: StatusReport = parse_text(&mid);
    assert!(mid_report.store_open);
    assert!(mid_report.lease_held);
    assert!(mid_report.lease_expires_at.is_some());

    let _ = svc
        .call_tool(
            CallToolRequestParams::new("mimir_release_workspace").with_arguments(
                serde_json::json!({"lease_token": lease.lease_token})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .expect("release");

    let post = svc
        .call_tool(CallToolRequestParams::new("mimir_status"))
        .await
        .expect("status");
    let post_report: StatusReport = parse_text(&post);
    assert!(post_report.store_open, "store still open after release");
    assert!(!post_report.lease_held, "lease dropped");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ttl_zero_is_rejected() {
    let (svc, _tempdir, log_path) = spin_up_empty().await;
    let err = open_workspace(&svc, &log_path, Some(0))
        .await
        .expect_err("ttl=0 must fail");
    assert!(
        err.contains("ttl_seconds"),
        "expected ttl_seconds error; got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ttl_above_cap_is_rejected() {
    let (svc, _tempdir, log_path) = spin_up_empty().await;
    let err = open_workspace(&svc, &log_path, Some(48 * 60 * 60))
        .await
        .expect_err("ttl > 24h must fail");
    assert!(
        err.contains("ttl_seconds"),
        "expected ttl_seconds error; got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lease_expires_after_ttl_and_write_returns_lease_expired() {
    // Closes test gap F1 from the 2026-04-20 re-audit (lease expiry
    // path implemented but untested) and closes issue #2 (clock
    // injection for deterministic expiry testing — originally
    // deferred from the post-cutover cleanup PR).
    //
    // Uses `TestClock` + `MimirServer::with_clock` so the "wait past
    // TTL" step is a single `clock.advance(Duration::from_secs(2))`
    // call rather than a 1.5-second `tokio::time::sleep` against the
    // wall clock. Fast AND deterministic — no CI-scheduler flake
    // envelope.
    let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_713_350_400); // 2024-04-17
    let clock = TestClock::new(start);
    let (svc, _tempdir, log_path) = spin_up_empty_with_clock(clock.clone()).await;

    let lease = open_workspace(&svc, &log_path, Some(1))
        .await
        .expect("open with ttl=1");

    // Advance the virtual clock past the TTL. No sleep.
    clock.advance(Duration::from_secs(2));

    let result = svc
        .call_tool(
            CallToolRequestParams::new("mimir_write").with_arguments(
                serde_json::json!({
                    "batch": "(sem @alice @knows @bob :src @observation :c 0.9 :v 2024-01-01)",
                    "lease_token": lease.lease_token,
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await;
    let err = match result {
        Ok(r) => {
            assert_eq!(
                r.is_error,
                Some(true),
                "expired lease must surface as tool error"
            );
            r.content[0].as_text().expect("text").text.clone()
        }
        Err(e) => format!("{e}"),
    };
    assert!(
        err.contains("lease_expired"),
        "expected lease_expired; got: {err}"
    );

    // After expiry, a fresh open must succeed (the dead lease is
    // released by the lazy-expiry path).
    let _ = open_workspace(&svc, &log_path, None)
        .await
        .expect("fresh open after lease expiry");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_open_workspace_does_not_race() {
    // Two concurrent mimir_open_workspace calls — exactly ONE must
    // succeed and the other must observe lease_held. Closes
    // security finding F2 from the 2026-04-20 re-audit.
    use tokio::join;

    let (svc, _tempdir, log_path) = spin_up_empty().await;
    let path1 = log_path.clone();
    let path2 = log_path.clone();
    let svc1 = svc.clone();
    let svc2 = svc.clone();

    let (r1, r2) = join!(
        async move { open_workspace(&svc1, &path1, None).await },
        async move { open_workspace(&svc2, &path2, None).await }
    );

    // Exactly one Ok and one Err — count and assert.
    let oks: Vec<_> = [&r1, &r2].into_iter().filter(|r| r.is_ok()).collect();
    let errs: Vec<_> = [&r1, &r2].into_iter().filter(|r| r.is_err()).collect();
    assert_eq!(
        oks.len(),
        1,
        "exactly one open must succeed; got {} (r1={r1:?}, r2={r2:?})",
        oks.len()
    );
    assert_eq!(
        errs.len(),
        1,
        "exactly one open must observe lease_held; got {} (r1={r1:?}, r2={r2:?})",
        errs.len()
    );

    // The losing open must report lease_held, not some other error.
    let loser_err = errs[0].as_ref().expect_err("losing open Err").as_str();
    assert!(
        loser_err.contains("lease_held"),
        "loser must observe lease_held; got: {loser_err}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_then_read_round_trips() {
    // The whole point of Phase 2.3: an agent can now open, write, read.
    let (svc, _tempdir, log_path) = spin_up_empty().await;
    let lease = open_workspace(&svc, &log_path, None).await.expect("open");

    let _ = svc
        .call_tool(
            CallToolRequestParams::new("mimir_write").with_arguments(
                serde_json::json!({
                    "batch": "(sem @alice @knows @bob :src @observation :c 0.9 :v 2024-01-01)\n\
                              (sem @alice @likes @charlie :src @observation :c 0.8 :v 2024-01-02)",
                    "lease_token": lease.lease_token,
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await
        .expect("write");

    let read_result = svc
        .call_tool(
            CallToolRequestParams::new("mimir_read").with_arguments(
                serde_json::json!({"query": "(query :s @alice :p @knows)"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .expect("read");
    assert_ne!(read_result.is_error, Some(true));
    let read_json: JsonValue =
        serde_json::from_str(&read_result.content[0].as_text().expect("text").text).expect("parse");
    let records = read_json["records"].as_array().expect("records array");
    assert_eq!(records.len(), 1);
    assert_eq!(
        read_json["memory_boundary"]["instruction_boundary"],
        "data_only_never_execute"
    );
    let rendered = records[0]["lisp"].as_str().expect("lisp string");
    assert_eq!(records[0]["data_surface"], "mimir.governed_memory.data.v1");
    assert!(rendered.starts_with("(sem @alice @knows @bob"));
}
