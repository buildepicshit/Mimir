//! `LibrarianConfig` — runtime configuration for a librarian run.
//!
//! Fields are public (plain struct) so operators can construct the
//! config from CLI flags, a config file, or a test harness without
//! forcing a builder API. All fields have documented defaults via
//! [`LibrarianConfig::default`].

use std::path::PathBuf;
use std::time::Duration;

use crate::{
    DEFAULT_DEDUP_VALID_AT_WINDOW_SECS, DEFAULT_LLM_TIMEOUT_SECS, DEFAULT_MAX_RETRIES_PER_RECORD,
    DEFAULT_PROCESSING_STALE_SECS,
};

/// Runtime configuration for a single librarian invocation.
///
/// `Default::default()` produces a config suitable for the
/// operator's typical personal workspace — `drafts_dir` under
/// `~/.mimir/drafts/`, 3 retries per record, 120 s LLM timeout,
/// conflicts skipped with a warning.
#[derive(Debug, Clone)]
pub struct LibrarianConfig {
    /// Root directory containing draft lifecycle sub-directories
    /// (`pending/`, `processing/`, `accepted/`, `skipped/`,
    /// `failed/`, `quarantined/`).
    pub drafts_dir: PathBuf,

    /// Path of the Mimir canonical log to commit against.
    pub workspace_log: PathBuf,

    /// Maximum number of retries per record before giving up and
    /// moving the draft to `failed/`.
    pub max_retries_per_record: u32,

    /// Per-invocation timeout for the LLM call.
    pub llm_timeout: Duration,

    /// Age after which an in-flight `processing/` draft is considered
    /// stale and recovered to `pending/` at run start.
    pub processing_stale_after: Duration,

    /// Duplicate-detection window for Semantic and Inferential
    /// `valid_at` clocks. Otherwise-identical candidate records within
    /// this absolute window are skipped before commit.
    pub dedup_valid_at_window: Duration,

    /// When `true`, `(s, p, valid_at)` supersession conflicts are
    /// dropped into `drafts_dir/conflicts/` for operator review
    /// instead of skipped with a warning. Maps to the D.3 policy
    /// from the 2026-04-21 Category 1 conversation.
    pub review_conflicts: bool,
}

impl LibrarianConfig {
    /// Construct a config from explicit paths; all other fields
    /// default.
    #[must_use]
    pub fn new(drafts_dir: PathBuf, workspace_log: PathBuf) -> Self {
        Self {
            drafts_dir,
            workspace_log,
            max_retries_per_record: DEFAULT_MAX_RETRIES_PER_RECORD,
            llm_timeout: Duration::from_secs(DEFAULT_LLM_TIMEOUT_SECS),
            processing_stale_after: Duration::from_secs(DEFAULT_PROCESSING_STALE_SECS),
            dedup_valid_at_window: Duration::from_secs(DEFAULT_DEDUP_VALID_AT_WINDOW_SECS),
            review_conflicts: false,
        }
    }
}

impl Default for LibrarianConfig {
    /// A config pointing at the operator's home-dir conventional
    /// locations: `~/.mimir/drafts/` and `~/.mimir/canonical.log`.
    ///
    /// If `HOME` is unset (unusual), falls back to `./drafts/` and
    /// `./canonical.log` relative to the current working directory;
    /// callers that care about the exact path should construct the
    /// config explicitly rather than rely on `Default`.
    fn default() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let (drafts_dir, workspace_log) = if let Some(h) = home {
            (h.join(".mimir/drafts"), h.join(".mimir/canonical.log"))
        } else {
            (PathBuf::from("./drafts"), PathBuf::from("./canonical.log"))
        };
        Self::new(drafts_dir, workspace_log)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_expected_retry_budget() {
        let cfg = LibrarianConfig::default();
        assert_eq!(cfg.max_retries_per_record, DEFAULT_MAX_RETRIES_PER_RECORD);
        assert_eq!(
            cfg.llm_timeout,
            Duration::from_secs(DEFAULT_LLM_TIMEOUT_SECS)
        );
        assert_eq!(
            cfg.processing_stale_after,
            Duration::from_secs(DEFAULT_PROCESSING_STALE_SECS)
        );
        assert_eq!(
            cfg.dedup_valid_at_window,
            Duration::from_secs(DEFAULT_DEDUP_VALID_AT_WINDOW_SECS)
        );
        assert!(!cfg.review_conflicts);
    }

    #[test]
    fn new_uses_explicit_paths() {
        let cfg =
            LibrarianConfig::new(PathBuf::from("/tmp/drafts"), PathBuf::from("/tmp/log.mimr"));
        assert_eq!(cfg.drafts_dir, PathBuf::from("/tmp/drafts"));
        assert_eq!(cfg.workspace_log, PathBuf::from("/tmp/log.mimr"));
        assert_eq!(
            cfg.dedup_valid_at_window,
            Duration::from_secs(DEFAULT_DEDUP_VALID_AT_WINDOW_SECS)
        );
    }
}
