//! Cross-process workspace write lock.
//!
//! The append-only log assumes one writer per workspace. This module
//! provides a small filesystem lockfile guard that higher-level write
//! surfaces can share before opening or writing a canonical log.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

static NEXT_LOCK_ID: AtomicU64 = AtomicU64::new(1);

/// Exclusive write guard for one canonical log path.
///
/// The guard creates `<canonical-log>.lock` using `create_new(true)`,
/// so acquisition is atomic across processes on local filesystems. The
/// lockfile is removed when the guard drops. If a process crashes, the
/// file can remain behind; operators should inspect and remove that
/// stale file deliberately rather than have Mimir guess liveness.
#[derive(Debug)]
pub struct WorkspaceWriteLock {
    path: PathBuf,
    lock_id: String,
    _file: File,
}

impl WorkspaceWriteLock {
    /// Acquire the lock associated with `log_path`.
    ///
    /// # Errors
    ///
    /// Returns [`WorkspaceLockError::AlreadyHeld`] when another
    /// holder's lockfile already exists, or
    /// [`WorkspaceLockError::Io`] for filesystem failures creating,
    /// writing, or syncing the lockfile.
    pub fn acquire_for_log(log_path: impl AsRef<Path>) -> Result<Self, WorkspaceLockError> {
        Self::acquire_for_log_with_owner(log_path, default_owner())
    }

    /// Acquire the lock associated with `log_path` and write an
    /// operator-visible owner string into the lockfile.
    ///
    /// # Errors
    ///
    /// Same as [`Self::acquire_for_log`].
    pub fn acquire_for_log_with_owner(
        log_path: impl AsRef<Path>,
        owner: impl AsRef<str>,
    ) -> Result<Self, WorkspaceLockError> {
        let log_path = log_path.as_ref();
        let path = lock_path_for_log(log_path);
        if let Some(parent) = parent_to_create(&path) {
            fs::create_dir_all(parent).map_err(|source| WorkspaceLockError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(WorkspaceLockError::AlreadyHeld { path });
            }
            Err(source) => {
                return Err(WorkspaceLockError::Io { path, source });
            }
        };

        let metadata = LockMetadata::new(log_path, owner.as_ref());
        write_lock_metadata(&mut file, &metadata).map_err(|source| {
            let _ = fs::remove_file(&path);
            WorkspaceLockError::Io {
                path: path.clone(),
                source,
            }
        })?;
        file.sync_all().map_err(|source| {
            let _ = fs::remove_file(&path);
            WorkspaceLockError::Io {
                path: path.clone(),
                source,
            }
        })?;

        Ok(Self {
            path,
            lock_id: metadata.lock_id,
            _file: file,
        })
    }

    /// Filesystem path of the held lockfile.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for WorkspaceWriteLock {
    fn drop(&mut self) {
        if lock_file_still_owned(&self.path, &self.lock_id) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Error returned when a workspace write lock cannot be acquired.
#[derive(Debug, Error)]
pub enum WorkspaceLockError {
    /// A lockfile already exists for this canonical log.
    #[error("workspace write lock already held: {path}")]
    AlreadyHeld {
        /// Existing lockfile path.
        path: PathBuf,
    },

    /// Filesystem failure while creating, writing, syncing, or
    /// cleaning up a lockfile.
    #[error("workspace write lock i/o failed at {path}: {source}")]
    Io {
        /// Path involved in the failing operation.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
}

/// Return the lockfile path associated with `log_path`.
///
/// `canonical.log` maps to `canonical.log.lock`; paths without a file
/// name use `.mimir-workspace.lock` inside that directory.
#[must_use]
pub fn lock_path_for_log(log_path: impl AsRef<Path>) -> PathBuf {
    let log_path = log_path.as_ref();
    let mut file_name = log_path
        .file_name()
        .map_or_else(|| OsString::from(".mimir-workspace"), OsString::from);
    file_name.push(".lock");
    match log_path.parent() {
        Some(parent) => parent.join(file_name),
        None => PathBuf::from(file_name),
    }
}

struct LockMetadata {
    lock_id: String,
    owner: String,
    pid: u32,
    acquired_at_ms: u128,
    log_path: PathBuf,
}

impl LockMetadata {
    fn new(log_path: &Path, owner: &str) -> Self {
        let acquired_at_ms = unix_time_millis();
        let pid = std::process::id();
        let sequence = NEXT_LOCK_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            lock_id: format!("{pid}-{acquired_at_ms}-{sequence}"),
            owner: owner.to_string(),
            pid,
            acquired_at_ms,
            log_path: log_path.to_path_buf(),
        }
    }
}

fn write_lock_metadata(file: &mut File, metadata: &LockMetadata) -> Result<(), std::io::Error> {
    writeln!(file, "lock_id={}", metadata.lock_id)?;
    writeln!(file, "owner={}", metadata.owner)?;
    writeln!(file, "pid={}", metadata.pid)?;
    writeln!(file, "acquired_at_ms={}", metadata.acquired_at_ms)?;
    writeln!(file, "log_path={}", metadata.log_path.display())?;
    Ok(())
}

fn unix_time_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}

fn parent_to_create(path: &Path) -> Option<&Path> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
}

fn lock_file_still_owned(path: &Path, lock_id: &str) -> bool {
    let Ok(contents) = fs::read_to_string(path) else {
        return false;
    };
    let expected = format!("lock_id={lock_id}");
    contents.lines().any(|line| line == expected)
}

fn default_owner() -> String {
    std::env::args()
        .next()
        .unwrap_or_else(|| "mimir".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_path_appends_lock_suffix() {
        let path = lock_path_for_log("/tmp/canonical.log");
        assert_eq!(path, PathBuf::from("/tmp/canonical.log.lock"));
    }

    #[test]
    fn relative_lock_path_has_no_parent_to_create() {
        assert_eq!(parent_to_create(Path::new("canonical.log.lock")), None);
    }
}
