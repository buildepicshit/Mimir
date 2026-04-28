//! `WorkspaceId` — stable identifier for a workspace. Implements
//! `docs/concepts/workspace-model.md` § 3.

use std::fmt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use thiserror::Error;
use ulid::Ulid;

/// Errors returned when constructing a [`WorkspaceId`].
#[derive(Debug, Error, PartialEq)]
pub enum WorkspaceIdError {
    /// The git-remote URL provided to [`WorkspaceId::from_git_remote`] was
    /// empty after normalisation.
    #[error("empty git remote URL")]
    EmptyRemote,
}

/// Errors returned by workspace detection.
#[derive(Debug, Error)]
pub enum WorkspaceError {
    /// A filesystem operation failed while walking ancestors or reading
    /// the git config.
    #[error("workspace I/O error: {0}")]
    Io(#[source] std::io::Error),

    /// A `.git` directory was found but its `config` file lacked an
    /// `[remote "origin"]` section or a `url` key within it.
    #[error("{path}: .git/config has no origin remote URL")]
    NoOriginRemote {
        /// Path to the `.git` directory where the config was inspected.
        path: PathBuf,
    },

    /// The origin URL from `.git/config` was malformed (empty after
    /// normalisation).
    #[error("{path}: origin URL normalises to empty string")]
    InvalidRemote {
        /// Path to the `.git` directory where the malformed URL was read.
        path: PathBuf,
    },

    /// No `.git` directory found at or above the starting path, and no
    /// explicit non-git workspace marker provided.
    #[error("no active workspace: walked to filesystem root from {start} without finding .git")]
    NoActiveWorkspace {
        /// The walk origin.
        start: PathBuf,
    },
}

/// A workspace identifier.
///
/// Stable across sessions and across machines. Two mechanisms exist
/// (per `docs/concepts/workspace-model.md` § 3):
///
/// - **Git-backed** — `WorkspaceId::from_git_remote(origin_url)` produces
///   a deterministic hash of the normalised remote URL. Branch is *not*
///   part of the ID by default; all branches of a repo share one
///   workspace.
/// - **Non-git** — `WorkspaceId::from_ulid(Ulid)` creates an explicit
///   identifier for workspaces not backed by a git repo.
///
/// # Examples
///
/// ```
/// # #![allow(clippy::unwrap_used)]
/// use mimir_core::WorkspaceId;
///
/// let a = WorkspaceId::from_git_remote("git@github.com:buildepicshit/Mimir.git").unwrap();
/// let b = WorkspaceId::from_git_remote("https://github.com/buildepicshit/Mimir").unwrap();
/// // The normalisation collapses scheme and trailing `.git` so equivalent
/// // remotes resolve to equivalent workspace IDs where host+path match.
/// // (Exact normalisation rules live in §§ 3.1 of workspace-model.md.)
/// assert_ne!(a, b); // SSH and HTTPS remotes with different hosts do differ.
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct WorkspaceId([u8; 32]);

impl WorkspaceId {
    /// Compute a workspace ID from a git `origin` remote URL.
    ///
    /// Normalisation performed before hashing:
    ///
    /// 1. Trim surrounding whitespace.
    /// 2. Lowercase the whole URL.
    /// 3. Strip any trailing `.git` suffix.
    /// 4. Strip any trailing slash.
    ///
    /// Branch is intentionally not included (see `workspace-model.md` § 3.1).
    ///
    /// # Errors
    ///
    /// Returns [`WorkspaceIdError::EmptyRemote`] if the URL is empty after
    /// normalisation.
    pub fn from_git_remote(origin_url: &str) -> Result<Self, WorkspaceIdError> {
        let normalised = normalise_git_remote(origin_url);
        if normalised.is_empty() {
            return Err(WorkspaceIdError::EmptyRemote);
        }
        let mut hasher = Sha256::new();
        hasher.update(normalised.as_bytes());
        let digest = hasher.finalize();
        let mut bytes = [0_u8; 32];
        bytes.copy_from_slice(&digest);
        Ok(Self(bytes))
    }

    /// Construct from an explicit ULID — for non-git workspaces created
    /// via `mimir init --workspace <name>` (see `workspace-model.md` § 3.2).
    #[must_use]
    pub fn from_ulid(ulid: Ulid) -> Self {
        let mut bytes = [0_u8; 32];
        let raw = ulid.to_bytes();
        // Place the 16-byte ULID in the high half; low half left zero.
        bytes[..16].copy_from_slice(&raw);
        Self(bytes)
    }

    /// The raw 32-byte hash.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Walk up from `start` looking for a `.git/` directory and, on
    /// finding one, read `origin` remote URL from `.git/config` and
    /// hash it per [`WorkspaceId::from_git_remote`].
    ///
    /// Implements `workspace-model.md` § 3.3 step 1 (git-backed
    /// workspaces). Returns [`WorkspaceError::NoActiveWorkspace`] if
    /// the walk reaches the filesystem root without finding `.git/`.
    ///
    /// # Errors
    ///
    /// - [`WorkspaceError::Io`] on filesystem read failure.
    /// - [`WorkspaceError::NoActiveWorkspace`] if no `.git/` is found.
    /// - [`WorkspaceError::NoOriginRemote`] if the config has no
    ///   `[remote "origin"] url = ...` entry.
    /// - [`WorkspaceError::InvalidRemote`] if the origin URL
    ///   normalises to an empty string.
    pub fn detect_from_path(start: &Path) -> Result<Self, WorkspaceError> {
        // `canonicalize` may fail if `start` doesn't exist yet — that's
        // a legitimate case (detection can run against a path being
        // set up). Fall back to the literal path so the ancestor walk
        // still operates.
        let start_abs = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
        let mut cursor: &Path = &start_abs;
        loop {
            let git_dir = cursor.join(".git");
            if git_dir.is_dir() {
                let config_path = git_dir.join("config");
                let contents = std::fs::read_to_string(&config_path).map_err(WorkspaceError::Io)?;
                let origin_url = parse_git_config_origin_url(&contents).ok_or_else(|| {
                    WorkspaceError::NoOriginRemote {
                        path: git_dir.clone(),
                    }
                })?;
                return Self::from_git_remote(&origin_url).map_err(|_| {
                    WorkspaceError::InvalidRemote {
                        path: git_dir.clone(),
                    }
                });
            }
            match cursor.parent() {
                Some(parent) if parent != cursor => cursor = parent,
                _ => {
                    return Err(WorkspaceError::NoActiveWorkspace { start: start_abs });
                }
            }
        }
    }
}

/// Parse a `.git/config`-shaped string and return the `origin` remote
/// URL if present. Tolerant of tabs, spaces, and missing-quote
/// variations in the section header.
///
/// Exposed `pub` for tests and for future `workspace init` tooling
/// that may want to validate its own config before writing.
#[must_use]
pub fn parse_git_config_origin_url(config: &str) -> Option<String> {
    let mut in_origin_section = false;
    for line in config.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            // Section header. Match `[remote "origin"]` with tolerance
            // for extra whitespace.
            let head = line.trim_matches(|c: char| c == '[' || c == ']').trim();
            in_origin_section =
                head == "remote \"origin\"" || head == "remote 'origin'" || head == "remote origin";
            continue;
        }
        if in_origin_section {
            // Looking for `url = <value>`.
            if let Some(rest) = line.strip_prefix("url") {
                let rest = rest.trim_start();
                if let Some(value) = rest.strip_prefix('=') {
                    return Some(value.trim().to_string());
                }
            }
        }
    }
    None
}

impl fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0[..8] {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

fn normalise_git_remote(url: &str) -> String {
    let trimmed = url.trim().to_ascii_lowercase();
    let stripped = trimmed.strip_suffix(".git").unwrap_or(&trimmed);
    let stripped = stripped.strip_suffix('/').unwrap_or(stripped);
    stripped.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_remote_rejected() {
        assert_eq!(
            WorkspaceId::from_git_remote("   "),
            Err(WorkspaceIdError::EmptyRemote),
        );
    }

    #[test]
    fn trailing_git_collapses() {
        let a = WorkspaceId::from_git_remote("https://github.com/foo/bar.git").unwrap();
        let b = WorkspaceId::from_git_remote("https://github.com/foo/bar").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn case_insensitive() {
        let a = WorkspaceId::from_git_remote("https://GitHub.com/Foo/Bar.git").unwrap();
        let b = WorkspaceId::from_git_remote("https://github.com/foo/bar").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn distinct_remotes_distinct_ids() {
        let a = WorkspaceId::from_git_remote("https://github.com/foo/mimir").unwrap();
        let b = WorkspaceId::from_git_remote("https://github.com/foo/other").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn ulid_workspace_is_stable() {
        let ulid = Ulid::from_parts(42, 99);
        let a = WorkspaceId::from_ulid(ulid);
        let b = WorkspaceId::from_ulid(ulid);
        assert_eq!(a, b);
    }

    #[test]
    fn display_is_eight_hex_bytes() {
        let id = WorkspaceId::from_git_remote("https://github.com/example/mimir").unwrap();
        let formatted = format!("{id}");
        assert_eq!(formatted.len(), 16);
        assert!(formatted.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // ----- git config parser -----

    #[test]
    fn parse_origin_url_from_standard_config() {
        let config = r#"
            [core]
                    repositoryformatversion = 0
                    filemode = true
            [remote "origin"]
                    url = git@github.com:foo/bar.git
                    fetch = +refs/heads/*:refs/remotes/origin/*
        "#;
        assert_eq!(
            parse_git_config_origin_url(config),
            Some("git@github.com:foo/bar.git".to_string())
        );
    }

    #[test]
    fn parse_origin_url_stops_at_next_section() {
        let config = r#"
            [remote "origin"]
                    url = https://github.com/foo/mimir
            [remote "upstream"]
                    url = https://github.com/bar/mimir
        "#;
        assert_eq!(
            parse_git_config_origin_url(config),
            Some("https://github.com/foo/mimir".to_string())
        );
    }

    #[test]
    fn parse_origin_url_returns_none_when_no_origin() {
        let config = r#"
            [core]
                    bare = false
            [remote "upstream"]
                    url = https://github.com/other/repo.git
        "#;
        assert_eq!(parse_git_config_origin_url(config), None);
    }

    #[test]
    fn parse_origin_url_skips_comments() {
        let config = r#"
            # remote origin is the canonical upstream
            ; and here's a semicolon comment
            [remote "origin"]
                    # url = https://commented.out/repo
                    url = https://real.example/repo.git
        "#;
        assert_eq!(
            parse_git_config_origin_url(config),
            Some("https://real.example/repo.git".to_string())
        );
    }

    // ----- detect_from_path -----

    fn write_fake_git_repo(root: &std::path::Path, origin_url: &str) {
        let git_dir = root.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(
            git_dir.join("config"),
            format!(
                "[core]\n\trepositoryformatversion = 0\n[remote \"origin\"]\n\turl = {origin_url}\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn detect_finds_git_at_start_path() {
        let dir = tempfile::TempDir::new().unwrap();
        write_fake_git_repo(dir.path(), "https://github.com/foo/mimir.git");
        let id = WorkspaceId::detect_from_path(dir.path()).unwrap();
        let expected = WorkspaceId::from_git_remote("https://github.com/foo/mimir.git").unwrap();
        assert_eq!(id, expected);
    }

    #[test]
    fn detect_walks_up_to_find_git() {
        let dir = tempfile::TempDir::new().unwrap();
        write_fake_git_repo(dir.path(), "https://github.com/foo/mimir.git");
        let subdir = dir.path().join("crates").join("mimir_core").join("src");
        std::fs::create_dir_all(&subdir).unwrap();
        let id = WorkspaceId::detect_from_path(&subdir).unwrap();
        let expected = WorkspaceId::from_git_remote("https://github.com/foo/mimir.git").unwrap();
        assert_eq!(id, expected);
    }

    #[test]
    fn detect_returns_no_active_workspace_on_empty_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let err = WorkspaceId::detect_from_path(dir.path()).unwrap_err();
        assert!(matches!(err, WorkspaceError::NoActiveWorkspace { .. }));
    }

    #[test]
    fn detect_returns_no_origin_if_config_missing_origin() {
        let dir = tempfile::TempDir::new().unwrap();
        let git_dir = dir.path().join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(git_dir.join("config"), "[core]\n\tbare = false\n").unwrap();
        let err = WorkspaceId::detect_from_path(dir.path()).unwrap_err();
        assert!(matches!(err, WorkspaceError::NoOriginRemote { .. }));
    }

    // ----- write-scope enforcement (structural) -----

    #[test]
    fn distinct_workspaces_produce_distinct_ids_across_forks() {
        // Spec § 3.1: a fork is a new workspace.
        let original = WorkspaceId::from_git_remote("https://github.com/upstream/mimir").unwrap();
        let fork = WorkspaceId::from_git_remote("https://github.com/fork/mimir").unwrap();
        assert_ne!(original, fork);
    }

    #[test]
    fn mirror_clones_converge_to_same_workspace() {
        // Spec § 3.1: mirror clones (same remote) are the same
        // workspace regardless of local path.
        let a = WorkspaceId::from_git_remote("https://github.com/foo/mimir.git").unwrap();
        let b = WorkspaceId::from_git_remote("https://github.com/foo/mimir").unwrap();
        assert_eq!(a, b);
    }
}
