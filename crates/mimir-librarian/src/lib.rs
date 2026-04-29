//! `mimir-librarian` — Mimir librarian. Ingests prose memory drafts,
//! sanitises them (separates observations from directives), structures
//! them into canonical Mimir Lisp, and commits them to the canonical
//! log via the in-process `mimir_core::Pipeline`.
//!
//! Category 1 of the 2026-04-21 Rolls Royce engineering plan
//! for librarian-governed draft processing.
//!
//! # Status
//!
//! **Draft-ingestion foundation.** The scope-aware draft envelope,
//! filesystem draft store, `submit` command, explicit file/directory
//! `sweep` command, rename-based lifecycle transitions, one-shot
//! `run` lifecycle skeleton, scratch-pipeline pre-emit validation,
//! bounded LLM validation retry with durable commit, and shared
//! workspace write-lock acquisition are wired. The remaining Category
//! 1 work still lands one concern at a time per the build discipline
//! documented in the Rolls Royce plan.
//!
//! See the crate `README.md` for the architectural decisions from
//! the 2026-04-21 Category 1 design conversation and the roadmap
//! of follow-up PRs that fill the skeleton in.
//!
//! # Architecture sketch
//!
//! ```text
//! ┌───────────────┐   ┌───────────────┐   ┌─────────────────────┐
//! │ Drafts surface│──▶│  LlmInvoker   │──▶│ PreEmitValidator    │
//! │ (filesystem)  │   │ (adapter CLI) │   │ (mimir_core Pipeline│
//! │               │   │               │   │  clone-on-write)    │
//! └───────────────┘   └───────────────┘   └─────────┬───────────┘
//!                                                   │
//!                                                   ▼
//!                                         ┌──────────────────────┐
//!                                         │   mimir_core::Store  │
//!                                         │      commit_batch    │
//!                                         └──────────────────────┘
//! ```
//!
//! The draft lifecycle, LLM invocation, bounded validation retry,
//! validator box, `Store::commit_batch` durable commit,
//! supersession-conflict policy, exact duplicate filtering across all
//! four memory types, and configurable same-day `valid_at` dedup for
//! Semantic / Inferential records are wired. The binary also exposes a
//! polling `watch` scheduler, and librarian-specific observability is
//! wired for runner and processor paths. Store commits acquire the
//! shared `mimir_core::WorkspaceWriteLock`, so direct librarian runs
//! and MCP write sessions exclude each other without introducing an
//! MCP dependency. The earlier Python prototype is retired and is no
//! longer shipped in the public tree.

#![cfg_attr(not(test), forbid(unsafe_code))]

mod config;
mod drafts;
mod error;
mod llm;
mod processor;
mod quorum;
mod runner;
#[cfg(test)]
mod test_tracing;
mod validator;

pub use config::LibrarianConfig;
pub use drafts::{
    Draft, DraftId, DraftMetadata, DraftSource, DraftSourceSurface, DraftState, DraftStore,
    DraftTransition, DRAFT_SCHEMA_VERSION,
};
pub use error::LibrarianError;
pub use llm::{ClaudeCliInvoker, CodexCliInvoker, CopilotCliInvoker, LlmAdapter, LlmInvoker};
pub use processor::{
    DedupPolicy, RawArchiveDraftProcessor, RetryingDraftProcessor, SupersessionConflictPolicy,
};
pub use quorum::{
    ConsensusLevel, DecisionStatus, ParticipantVote, QuorumAdapterRequest, QuorumEpisode,
    QuorumEpisodeState, QuorumParticipant, QuorumParticipantOutput, QuorumResult, QuorumRound,
    QuorumStore, VoteChoice, QUORUM_SCHEMA_VERSION,
};
pub use runner::{
    run_once, DeferredDraftProcessor, DraftProcessingDecision, DraftProcessor, DraftRunItem,
    DraftRunSummary, DEFAULT_PROCESSING_STALE_SECS,
};
pub use validator::PreEmitValidator;

/// Canonical location of the librarian's system prompt.
///
/// Embedded at compile time via [`include_str!`] so the prompt is
/// trivially versionable and reviewable as a text file.
pub const SYSTEM_PROMPT: &str = include_str!("prompts/system_prompt.md");

/// Default maximum number of per-record retries when a candidate
/// record fails pre-emit validation.
///
/// 3 retries matches the bounded-retry discipline in the Rolls Royce
/// plan § Category 1 acceptance criteria. Configurable per-run via
/// [`LibrarianConfig::max_retries_per_record`].
pub const DEFAULT_MAX_RETRIES_PER_RECORD: u32 = 3;

/// Default `claude -p` invocation timeout, in seconds.
///
/// 120 s is comfortably above observed 10–30 s typical and the
/// 50 s upper outlier observed during the iteration-3 run on real
/// drafts.
pub const DEFAULT_LLM_TIMEOUT_SECS: u64 = 120;

/// Default duplicate-detection `valid_at` window, in seconds.
///
/// Semantic and Inferential candidates with otherwise-identical
/// canonical fields inside this window are skipped before commit.
pub const DEFAULT_DEDUP_VALID_AT_WINDOW_SECS: u64 = 86_400;
