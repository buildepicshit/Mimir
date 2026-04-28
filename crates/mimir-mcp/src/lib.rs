//! `mimir-mcp` — Mimir MCP (Model Context Protocol) server library.
//!
//! Exposes Mimir's current workspace-local canonical store to
//! **Claude** (Claude Desktop and Claude Code) via stdin/stdout
//! transport. Per Mimir's 2026-04-24 mandate, Claude and Codex are
//! the first target surfaces and future agents integrate through
//! draft/retrieval adapters. This crate remains the Claude MCP surface
//! until scope-aware adapters are specified; other MCP clients may
//! technically connect because the protocol is standard, but they are
//! out of scope for testing, support, and design tuning here. Phase 2
//! of `docs/planning/2026-04-19-roadmap-to-prime-time.md`.
//!
//! The library surface ([`MimirServer`]) is reusable in tests with the
//! in-memory `tokio::io::duplex` transport; the binary at `src/main.rs`
//! is a thin wrapper that wires `MimirServer` to stdio and a
//! tracing-to-stderr subscriber.
//!
//! ## Tools
//!
//! Phase 2.3 ships **nine** tools across three layers:
//!
//! Status:
//! 1. `mimir_status` — server health, store-open + lease-held flags.
//!
//! Read (workspace-required):
//! 2. `mimir_read` — Lisp `(query …)` execution; matched records
//!    rendered as data-marked Lisp payloads.
//! 3. `mimir_verify` — read-only integrity check on a canonical log.
//! 4. `mimir_list_episodes` — paginated episode metadata.
//! 5. `mimir_render_memory` — single-record data-marked render.
//!
//! Write + lifecycle (lease-required):
//! 6. `mimir_open_workspace` — opens a `Store` and mints a write lease.
//! 7. `mimir_write` — commits a Lisp batch.
//! 8. `mimir_close_episode` — convenience wrapper for `(episode :close)`.
//! 9. `mimir_release_workspace` — drops the lease; reads continue to work.
//!
//! Read tools require an opened workspace store (set
//! `MIMIR_WORKSPACE_PATH` at startup, or call `mimir_open_workspace`).
//! Write tools additionally require a valid lease token returned by
//! `mimir_open_workspace`. See `crate::server` module docs for the
//! lease state machine.
//!
//! ## Stability
//!
//! The library API will move freely until `mimir-mcp` cuts a 1.0
//! release alongside the rest of the workspace. The on-wire MCP tool
//! surface (tool names, input schemas, output shapes) is what
//! agent-side prompts depend on; that surface gets a drift gate at
//! Phase 2.4 (`tool_catalog_drift`) and stricter `SemVer` commitments
//! once `v0.1.0-alpha.1` ships.

#![cfg_attr(not(test), forbid(unsafe_code))]

mod server;

pub use server::{
    Clock, CloseEpisodeArgs, EpisodeRow, ListEpisodesArgs, MemoryBoundary, MimirServer,
    OpenWorkspaceArgs, OpenWorkspaceResponse, ReadArgs, ReadResponse, ReleaseWorkspaceArgs,
    ReleaseWorkspaceResponse, RenderMemoryArgs, RenderMemoryResponse, RenderedMemoryRecord,
    StatusReport, SystemClock, VerifyArgs, VerifyReportJson, WriteArgs, WriteResponse,
    DEFAULT_LEASE_TTL_SECONDS, MAX_LEASE_TTL_SECONDS,
};
