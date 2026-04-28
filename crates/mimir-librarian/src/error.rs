//! `LibrarianError` — the typed error taxonomy for the librarian.
//!
//! Every externally-observable failure mode has a dedicated variant
//! so the caller (CLI / test / embedder) can match on the specific
//! failure and the retry loop (follow-up PR) can classify-then-hint.
//!
//! The [`LibrarianError::NotYetImplemented`] variant marks the
//! boundary between the skeleton landing in this PR and the
//! follow-up PRs that wire up real processing. Every method that
//! currently returns this variant names its follow-up sub-task so
//! the remaining work is traceable from `grep`.

use crate::drafts::{DraftId, DraftState};
use thiserror::Error;

/// Every externally-observable failure mode of the librarian.
///
/// Variants are stable public API; new variants may be added in
/// future minor releases (enum is `#[non_exhaustive]`).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LibrarianError {
    /// The `claude -p` subprocess failed to spawn or exited with a
    /// non-zero status code before producing a response.
    #[error("llm invocation failed: {message}")]
    LlmInvocationFailed {
        /// Human-readable context (stderr tail, exit code, or spawn error).
        message: String,
    },

    /// The LLM returned output that could not be parsed as the
    /// expected JSON shape (`{records: [...], notes: "..."}`).
    #[error("llm produced non-JSON response: {parse_err}")]
    LlmNonJsonResponse {
        /// First 400 chars of the raw LLM response, for debugging.
        raw: String,
        /// The `serde_json` error message.
        parse_err: String,
    },

    /// A candidate record still failed `mimir_core::Pipeline` validation
    /// after the full retry budget was exhausted. The wrapped error is
    /// from the final attempt.
    #[error("validation failed after {attempts} retries: {source}")]
    ValidationFailedAfterRetry {
        /// Retry count consumed before giving up.
        attempts: u32,
        /// The pipeline error from the final attempt.
        #[source]
        source: mimir_core::PipelineError,
    },

    /// A candidate record failed pre-emit validation.
    #[error("validation rejected candidate: {source}")]
    ValidationRejected {
        /// The pipeline error produced by the scratch validation pass.
        #[source]
        source: mimir_core::PipelineError,
    },

    /// The validator could not acquire a wall-clock timestamp.
    #[error("validation clock unavailable: {message}")]
    ValidationClock {
        /// Clock error details.
        message: String,
    },

    /// The durable canonical store could not be opened.
    #[error("store open failed: {source}")]
    StoreOpen {
        /// The underlying store error.
        #[source]
        source: mimir_core::StoreError,
    },

    /// The workspace write lock could not be acquired.
    #[error("workspace write lock unavailable: {source}")]
    WorkspaceLock {
        /// Underlying lock acquisition error.
        #[source]
        source: mimir_core::WorkspaceLockError,
    },

    /// The durable canonical store rejected or failed a commit.
    #[error("store commit failed: {source}")]
    StoreCommit {
        /// The underlying store error.
        #[source]
        source: mimir_core::StoreError,
    },

    /// Could not acquire the Mimir workspace lease. Future-proofing
    /// for E.1 (MCP-client mode); the E.2 skeleton variant opens
    /// `Store` directly and does not hit this path.
    #[error("could not acquire workspace lease: {message}")]
    LeaseContest {
        /// Human-readable context (existing-lease expiry, holder, etc.).
        message: String,
    },

    /// An I/O error occurred reading or moving a draft file.
    #[error("draft i/o error: {0}")]
    DraftIo(#[from] std::io::Error),

    /// A draft JSON envelope could not be encoded or decoded.
    #[error("draft JSON error: {0}")]
    DraftJson(#[from] serde_json::Error),

    /// A draft file used an unsupported schema version.
    #[error("unsupported draft schema version: {version}")]
    UnsupportedDraftSchema {
        /// Schema version read from the draft file.
        version: u32,
    },

    /// A draft file's declared ID did not match its content and provenance.
    #[error("draft id mismatch: declared {declared}, computed {computed}")]
    DraftIdMismatch {
        /// ID declared in the draft file.
        declared: String,
        /// ID recomputed from raw text and provenance.
        computed: String,
    },

    /// A draft transition was requested outside the supported
    /// lifecycle graph.
    #[error("invalid draft transition: {from:?} -> {to:?}")]
    InvalidDraftTransition {
        /// Source state requested by the caller.
        from: DraftState,
        /// Target state requested by the caller.
        to: DraftState,
    },

    /// The source draft file for a transition was not found.
    #[error("draft {id} not found in {state:?}")]
    DraftNotFound {
        /// State directory that was expected to contain the draft.
        state: DraftState,
        /// Draft ID requested by the caller.
        id: DraftId,
    },

    /// The target draft file for a transition already exists.
    #[error("draft {id} already exists in {state:?}")]
    DraftAlreadyExists {
        /// Target state directory.
        state: DraftState,
        /// Draft ID requested by the caller.
        id: DraftId,
    },

    /// A quorum participant output already exists for this episode,
    /// round, and participant.
    #[error(
        "quorum output already exists for episode {episode_id}, round {round}, participant {participant_id}"
    )]
    QuorumOutputAlreadyExists {
        /// Quorum episode id.
        episode_id: String,
        /// Quorum round name.
        round: String,
        /// Participant id.
        participant_id: String,
    },

    /// A quorum operation would violate the deliberation protocol.
    #[error("quorum protocol violation for episode {episode_id}: {message}")]
    QuorumProtocolViolation {
        /// Quorum episode id.
        episode_id: String,
        /// Human-readable protocol rule that was violated.
        message: String,
    },

    /// Retry budget exceeded for a single record.
    #[error("retry budget exhausted after {attempts} attempts")]
    RetryBudgetExhausted {
        /// Retry count consumed.
        attempts: u32,
    },

    /// The librarian was asked to do something not yet implemented.
    ///
    /// Present only during the Category 1 skeleton phase. Each
    /// instance names the specific follow-up PR sub-task responsible
    /// for removing the variant at that call site.
    #[error("not yet implemented: {component} (see crate README § 'Roadmap within Category 1')")]
    NotYetImplemented {
        /// Name of the component / method that is not yet wired up.
        component: &'static str,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_informative_for_not_yet_implemented() {
        let err = LibrarianError::NotYetImplemented {
            component: "LlmInvoker::invoke",
        };
        let text = err.to_string();
        assert!(text.contains("LlmInvoker::invoke"));
        assert!(text.contains("not yet implemented"));
    }

    #[test]
    fn draft_io_wraps_std_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing draft");
        let wrapped: LibrarianError = io_err.into();
        assert!(
            matches!(wrapped, LibrarianError::DraftIo(_)),
            "expected DraftIo, got something else",
        );
    }

    #[test]
    fn retry_budget_exhausted_reports_attempts() {
        let err = LibrarianError::RetryBudgetExhausted { attempts: 3 };
        assert!(err.to_string().contains("3 attempts"));
    }
}
