//! Append-only canonical log per `docs/concepts/write-protocol.md`.
//!
//! [`CanonicalLog`] is the single durable file backing a workspace. It
//! exposes only four operations: append, sync (fsync), truncate, and
//! scan for the last CHECKPOINT. The write protocol (see `store.rs`)
//! composes these into two-phase commits.
//!
//! # File format
//!
//! Every canonical log starts with an 8-byte header:
//!
//! ```text
//! offset 0..4 : ASCII magic `MIMR` (4 bytes)
//! offset 4..8 : little-endian u32 format version
//! offset 8..  : record stream (opcode + varint length + body, repeating)
//! ```
//!
//! The header is written eagerly when [`CanonicalLog::open`] is called
//! against an empty (or non-existent) file. On reopen, the header is
//! validated and a non-Mimir or wrong-version file is rejected with
//! [`LogError::IncompatibleFormat`] BEFORE any truncation, append, or
//! recovery logic runs. This closes the destructive-truncate footgun
//! where opening `Store` against a misrouted path would zero an
//! arbitrary file.
//!
//! From the [`LogBackend`] trait's perspective the header is invisible:
//! [`len`](LogBackend::len), [`read_all`](LogBackend::read_all),
//! [`last_checkpoint_end`](LogBackend::last_checkpoint_end), and
//! [`truncate`](LogBackend::truncate) all operate in **logical bytes**
//! (record stream only). [`CanonicalLog`] handles the physical header
//! transparently; in-memory test backends like `FaultyLog` carry no
//! header because they never persist.
//!
//! Engineering notes:
//!
//! - Plain file handle; no mmap, no `O_DIRECT`.
//! - `sync()` is `fsync` (full metadata + data per spec § 6.2).
//! - Orphan detection via forward scan from start. Spec § 10.1 suggests
//!   a backward-scan optimization for healthy logs; deferred until we
//!   have a realistic benchmark.
//!
//! The [`LogBackend`] trait abstracts the four filesystem primitives
//! so [`Store`](crate::store::Store) can be parameterized over a
//! fault-injecting test backend. Production code uses [`CanonicalLog`]
//! (the default).

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::canonical::{decode_record, CanonicalRecord};

/// 4-byte ASCII magic prefix identifying an Mimir canonical log.
pub const LOG_MAGIC: [u8; 4] = *b"MIMR";

/// Current canonical-log format version. Bumped on any wire-format
/// break that the decoder cannot handle transparently.
pub const LOG_FORMAT_VERSION: u32 = 1;

/// Physical byte length of the on-disk header (magic + version).
pub const LOG_HEADER_SIZE: u64 = 8;

/// The filesystem primitives a `Store` needs from its underlying log.
///
/// The production implementation is [`CanonicalLog`]; tests and crash-
/// injection harnesses implement this trait to arm failures on
/// specific operations (see `store::tests::FaultyLog`).
pub trait LogBackend {
    /// Append `bytes` at the current end. No fsync is implied.
    ///
    /// # Errors
    ///
    /// Implementations return [`LogError`] on failure. A failed append
    /// may leave partial bytes written; callers are responsible for
    /// truncating back to their pre-write offset.
    fn append(&mut self, bytes: &[u8]) -> Result<(), LogError>;

    /// Fsync the log. Spec § 6 — data + metadata.
    ///
    /// # Errors
    ///
    /// Returns [`LogError`] on failure. Per spec § 7's `fsync-fails`
    /// row, a sync failure is treated by callers as uncommitted.
    fn sync(&mut self) -> Result<(), LogError>;

    /// Truncate the log to `new_len` bytes (and fsync the truncation).
    ///
    /// # Errors
    ///
    /// - [`LogError::TruncateBeyondEnd`] if `new_len > self.len()`.
    /// - [`LogError::Io`] on other failures.
    fn truncate(&mut self, new_len: u64) -> Result<(), LogError>;

    /// Read the entire log into a buffer.
    ///
    /// # Errors
    ///
    /// Returns [`LogError`] on read failure.
    fn read_all(&mut self) -> Result<Vec<u8>, LogError>;

    /// Byte length of the log.
    fn len(&self) -> u64;

    /// `true` if the log is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Scan forward from offset `0` and return the byte offset after
    /// the last `Checkpoint` record. Returns `0` if no Checkpoint has
    /// committed yet.
    ///
    /// Decode errors past the last good Checkpoint stop the scan —
    /// that matches spec § 10's orphan-truncation contract.
    ///
    /// # Errors
    ///
    /// Returns [`LogError`] on read failure.
    fn last_checkpoint_end(&mut self) -> Result<u64, LogError>;
}

/// The append-only canonical log file.
#[derive(Debug)]
pub struct CanonicalLog {
    file: File,
    path: PathBuf,
    len: u64,
}

impl CanonicalLog {
    /// Open or create the canonical log at `path`. File is opened in
    /// read+write+append-friendly mode; existing content is preserved.
    ///
    /// On a freshly-created (or pre-existing zero-byte) file, the
    /// 8-byte header (`MIMR` + format version `1` LE) is written and
    /// fsync'd before this returns. On a file that already has bytes,
    /// the first 8 bytes are validated against the expected header and
    /// rejected with [`LogError::IncompatibleFormat`] on mismatch —
    /// **before** any truncation, append, or replay path runs. This
    /// guards against the destructive-truncate footgun where pointing
    /// `Store::open` at a misrouted path would silently zero the file.
    ///
    /// # Errors
    ///
    /// - [`LogError::Io`] if the file cannot be created or opened.
    /// - [`LogError::IncompatibleFormat`] if the file's first bytes are
    ///   not a valid Mimir log header (truncated, wrong magic, or
    ///   wrong format version).
    pub fn open(path: impl AsRef<Path>) -> Result<Self, LogError> {
        let path = path.as_ref().to_path_buf();
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(LogError::Io)?;
        let physical_len = file.metadata().map_err(LogError::Io)?.len();

        if physical_len == 0 {
            // Fresh file: write the header eagerly + fsync so a crash
            // between open and the first append still leaves a
            // recognizable Mimir log.
            file.seek(SeekFrom::Start(0)).map_err(LogError::Io)?;
            let mut header = [0_u8; 8];
            header[0..4].copy_from_slice(&LOG_MAGIC);
            header[4..8].copy_from_slice(&LOG_FORMAT_VERSION.to_le_bytes());
            file.write_all(&header).map_err(LogError::Io)?;
            file.sync_all().map_err(LogError::Io)?;
            return Ok(Self { file, path, len: 0 });
        }

        if physical_len < LOG_HEADER_SIZE {
            return Err(LogError::IncompatibleFormat {
                reason: format!(
                    "file is {physical_len} bytes; expected at least \
                     {LOG_HEADER_SIZE}-byte Mimir header"
                ),
            });
        }

        // Read + validate header.
        file.seek(SeekFrom::Start(0)).map_err(LogError::Io)?;
        let mut header = [0_u8; 8];
        file.read_exact(&mut header).map_err(LogError::Io)?;
        if header[0..4] != LOG_MAGIC {
            return Err(LogError::IncompatibleFormat {
                reason: format!(
                    "magic mismatch: got {:?}, expected {:?} ({:?})",
                    &header[0..4],
                    LOG_MAGIC,
                    std::str::from_utf8(&LOG_MAGIC).unwrap_or("?"),
                ),
            });
        }
        let version = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        if version != LOG_FORMAT_VERSION {
            return Err(LogError::IncompatibleFormat {
                reason: format!(
                    "format version {version} not supported \
                     (this build supports version {LOG_FORMAT_VERSION})"
                ),
            });
        }

        // Logical length excludes the header; payload starts at offset
        // LOG_HEADER_SIZE.
        let len = physical_len - LOG_HEADER_SIZE;
        Ok(Self { file, path, len })
    }

    /// The filesystem path of this log.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl LogBackend for CanonicalLog {
    fn append(&mut self, bytes: &[u8]) -> Result<(), LogError> {
        self.file.seek(SeekFrom::End(0)).map_err(LogError::Io)?;
        self.file.write_all(bytes).map_err(LogError::Io)?;
        self.len = self
            .len
            .checked_add(bytes.len() as u64)
            .ok_or(LogError::LogOverflow)?;
        Ok(())
    }

    fn sync(&mut self) -> Result<(), LogError> {
        self.file.sync_all().map_err(LogError::Io)
    }

    fn truncate(&mut self, new_len: u64) -> Result<(), LogError> {
        if new_len > self.len {
            return Err(LogError::TruncateBeyondEnd {
                requested: new_len,
                current: self.len,
            });
        }
        // Logical truncation: physical file length is `header + new_len`.
        // The header is never touched, so a `truncate(0)` rollback still
        // leaves a valid (empty-payload) Mimir log on disk.
        let physical_new_len = LOG_HEADER_SIZE
            .checked_add(new_len)
            .ok_or(LogError::LogOverflow)?;
        self.file.set_len(physical_new_len).map_err(LogError::Io)?;
        self.file.sync_all().map_err(LogError::Io)?;
        self.len = new_len;
        Ok(())
    }

    fn read_all(&mut self) -> Result<Vec<u8>, LogError> {
        // Skip the header — callers see only the logical record stream.
        self.file
            .seek(SeekFrom::Start(LOG_HEADER_SIZE))
            .map_err(LogError::Io)?;
        let capacity = usize::try_from(self.len).unwrap_or(usize::MAX);
        let mut buf = Vec::with_capacity(capacity);
        self.file.read_to_end(&mut buf).map_err(LogError::Io)?;
        Ok(buf)
    }

    fn len(&self) -> u64 {
        self.len
    }

    fn last_checkpoint_end(&mut self) -> Result<u64, LogError> {
        let bytes = self.read_all()?;
        let mut pos: usize = 0;
        let mut last_checkpoint_end: u64 = 0;
        while pos < bytes.len() {
            match decode_record(&bytes[pos..]) {
                Ok((record, consumed)) => {
                    pos += consumed;
                    if matches!(record, CanonicalRecord::Checkpoint(_)) {
                        last_checkpoint_end = pos as u64;
                    }
                }
                Err(_) => break,
            }
        }
        Ok(last_checkpoint_end)
    }
}

/// Errors produced by [`CanonicalLog`].
#[derive(Debug, Error)]
pub enum LogError {
    /// Underlying filesystem / I/O error.
    #[error("log I/O error: {0}")]
    Io(#[source] std::io::Error),

    /// The log's byte length would exceed `u64::MAX`. A single-workspace
    /// log is not expected to hit this — included for completeness.
    #[error("log length would overflow u64")]
    LogOverflow,

    /// Truncation target is beyond the current log length.
    #[error("truncate target {requested} exceeds current length {current}")]
    TruncateBeyondEnd {
        /// Requested truncation offset.
        requested: u64,
        /// Current log length.
        current: u64,
    },

    /// The file at the supplied path is not a recognizable Mimir
    /// canonical log. Either the magic prefix doesn't match, the
    /// declared format version isn't supported by this build, or the
    /// file is too short to carry the 8-byte header. Surfaced
    /// **before** any truncation or recovery logic so misrouted-path
    /// opens cannot silently destroy data.
    #[error("incompatible canonical-log format: {reason}")]
    IncompatibleFormat {
        /// Human-readable diagnostic; never include the actual bytes
        /// of any payload past the header (no PII / value leakage).
        reason: String,
    },
}

impl PartialEq for LogError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Io(a), Self::Io(b)) => a.kind() == b.kind(),
            (Self::LogOverflow, Self::LogOverflow) => true,
            (
                Self::TruncateBeyondEnd {
                    requested: ra,
                    current: ca,
                },
                Self::TruncateBeyondEnd {
                    requested: rb,
                    current: cb,
                },
            ) => ra == rb && ca == cb,
            (Self::IncompatibleFormat { reason: ra }, Self::IncompatibleFormat { reason: rb }) => {
                ra == rb
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::{encode_record, CheckpointRecord};
    use crate::clock::ClockTime;
    use crate::symbol::SymbolId;
    use std::fs;
    use tempfile::TempDir;

    fn checkpoint_bytes(seed: u64) -> Vec<u8> {
        let mut buf = Vec::new();
        encode_record(
            &CanonicalRecord::Checkpoint(CheckpointRecord {
                episode_id: SymbolId::new(seed),
                at: ClockTime::try_from_millis(seed * 1000).expect("non-sentinel"),
                memory_count: 1,
            }),
            &mut buf,
        );
        buf
    }

    #[test]
    fn open_creates_empty_log_and_writes_header() {
        let tmp = TempDir::new().expect("tmp");
        let path = tmp.path().join("canonical.log");
        let log = CanonicalLog::open(&path).expect("open");
        assert!(
            log.is_empty(),
            "logical length is 0 (header is transparent)"
        );
        assert_eq!(log.len(), 0);
        // Physical: 8-byte header was written + fsync'd.
        let physical = fs::metadata(&path).expect("stat").len();
        assert_eq!(physical, LOG_HEADER_SIZE);
        let raw = fs::read(&path).expect("read raw");
        assert_eq!(&raw[0..4], &LOG_MAGIC, "magic prefix written");
        assert_eq!(
            u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]),
            LOG_FORMAT_VERSION,
            "format version written LE"
        );
    }

    #[test]
    fn open_reopens_existing_log_preserving_length() {
        let tmp = TempDir::new().expect("tmp");
        let path = tmp.path().join("canonical.log");
        let payload = checkpoint_bytes(1);
        {
            let mut log = CanonicalLog::open(&path).expect("open");
            log.append(&payload).expect("append");
            log.sync().expect("sync");
        }
        let log = CanonicalLog::open(&path).expect("reopen");
        assert_eq!(log.len(), payload.len() as u64);
    }

    /// **Audit finding F1 (P1, 2026-04-19 fresh assessment):** opening
    /// `Store` (which composes `CanonicalLog::open`) against a
    /// misrouted path silently truncated the file to zero, because
    /// `last_checkpoint_end` returned `0` for a non-Mimir file and
    /// `Store::from_backend` truncated to that offset. The magic
    /// header makes this impossible: the open call rejects the file
    /// with `IncompatibleFormat` before any truncation runs, so the
    /// non-Mimir file is preserved byte-for-byte.
    #[test]
    fn open_refuses_to_initialize_non_mimir_file() {
        let tmp = TempDir::new().expect("tmp");
        let path = tmp.path().join("not-a-mimir-log.cfg");
        let original: &[u8] = b"some_other_format=hello\nimportant_data=42\n";
        fs::write(&path, original).expect("write fixture");

        let err = CanonicalLog::open(&path).expect_err("must reject non-Mimir file");
        assert!(
            matches!(err, LogError::IncompatibleFormat { .. }),
            "expected IncompatibleFormat, got {err:?}"
        );

        // CRITICAL invariant: the file was NOT modified by the open
        // attempt. If this assertion ever regresses, the destructive-
        // truncate footgun is back.
        let after = fs::read(&path).expect("read post-open");
        assert_eq!(
            after, original,
            "non-Mimir file must be preserved byte-for-byte on rejected open"
        );
    }

    #[test]
    fn open_refuses_truncated_header() {
        let tmp = TempDir::new().expect("tmp");
        let path = tmp.path().join("canonical.log");
        // 5 bytes — less than the 8-byte header, so we can't even read
        // the version field. Has the magic prefix to rule out
        // confusion with the magic-mismatch case.
        fs::write(&path, b"MIMR\x01").expect("write fixture");
        let err = CanonicalLog::open(&path).expect_err("must reject truncated header");
        assert!(
            matches!(err, LogError::IncompatibleFormat { .. }),
            "expected IncompatibleFormat, got {err:?}"
        );
        // Bytes preserved.
        assert_eq!(fs::read(&path).expect("read"), b"MIMR\x01");
    }

    #[test]
    fn open_refuses_wrong_magic() {
        let tmp = TempDir::new().expect("tmp");
        let path = tmp.path().join("canonical.log");
        // Right size, wrong magic. Plausible scenario: an older
        // Mimir-related tool that wrote a different prefix.
        fs::write(&path, b"WICK\x01\x00\x00\x00").expect("write fixture");
        let err = CanonicalLog::open(&path).expect_err("must reject wrong magic");
        assert!(
            matches!(err, LogError::IncompatibleFormat { .. }),
            "expected IncompatibleFormat, got {err:?}"
        );
    }

    #[test]
    fn open_refuses_unsupported_format_version() {
        let tmp = TempDir::new().expect("tmp");
        let path = tmp.path().join("canonical.log");
        // Right magic, far-future version.
        let mut header = Vec::with_capacity(8);
        header.extend_from_slice(&LOG_MAGIC);
        header.extend_from_slice(&999_u32.to_le_bytes());
        fs::write(&path, &header).expect("write fixture");
        let err = CanonicalLog::open(&path).expect_err("must reject unsupported version");
        match err {
            LogError::IncompatibleFormat { reason } => {
                assert!(
                    reason.contains("999"),
                    "diagnostic should name the bad version, got: {reason}"
                );
            }
            other => panic!("expected IncompatibleFormat, got {other:?}"),
        }
    }

    #[test]
    fn open_idempotent_against_reopen() {
        // Opening a v1 log we just created must succeed without
        // corrupting the header.
        let tmp = TempDir::new().expect("tmp");
        let path = tmp.path().join("canonical.log");
        let _first = CanonicalLog::open(&path).expect("first open");
        // Capture physical bytes after the first open.
        let raw1 = fs::read(&path).expect("read 1");
        // Reopen.
        let _second = CanonicalLog::open(&path).expect("reopen");
        let raw2 = fs::read(&path).expect("read 2");
        assert_eq!(raw1, raw2, "reopen does not mutate the header");
        assert_eq!(
            raw1.len(),
            usize::try_from(LOG_HEADER_SIZE).expect("header fits")
        );
    }

    #[test]
    fn append_sync_roundtrip_preserves_bytes() {
        let tmp = TempDir::new().expect("tmp");
        let mut log = CanonicalLog::open(tmp.path().join("canonical.log")).expect("open");
        let payload = checkpoint_bytes(42);
        log.append(&payload).expect("append");
        log.sync().expect("sync");
        let read = log.read_all().expect("read");
        assert_eq!(read, payload);
    }

    #[test]
    fn truncate_shrinks_log() {
        let tmp = TempDir::new().expect("tmp");
        let mut log = CanonicalLog::open(tmp.path().join("canonical.log")).expect("open");
        let first = checkpoint_bytes(1);
        let second = checkpoint_bytes(2);
        log.append(&first).expect("append 1");
        log.append(&second).expect("append 2");
        log.sync().expect("sync");
        log.truncate(first.len() as u64).expect("truncate");
        assert_eq!(log.len(), first.len() as u64);
        let read = log.read_all().expect("read");
        assert_eq!(read, first);
    }

    #[test]
    fn truncate_beyond_end_errors() {
        let tmp = TempDir::new().expect("tmp");
        let mut log = CanonicalLog::open(tmp.path().join("canonical.log")).expect("open");
        let err = log.truncate(100).expect_err("beyond");
        assert!(matches!(
            err,
            LogError::TruncateBeyondEnd {
                requested: 100,
                current: 0
            }
        ));
    }

    #[test]
    fn truncate_to_zero_clears_the_log() {
        let tmp = TempDir::new().expect("tmp");
        let mut log = CanonicalLog::open(tmp.path().join("canonical.log")).expect("open");
        let payload = checkpoint_bytes(1);
        log.append(&payload).expect("append");
        log.sync().expect("sync");
        assert!(log.len() > 0);
        log.truncate(0).expect("truncate to zero");
        assert_eq!(log.len(), 0);
        assert!(log.is_empty());
        assert!(log.read_all().expect("read").is_empty());
    }

    #[test]
    fn truncate_to_current_length_is_a_noop() {
        let tmp = TempDir::new().expect("tmp");
        let mut log = CanonicalLog::open(tmp.path().join("canonical.log")).expect("open");
        let payload = checkpoint_bytes(1);
        log.append(&payload).expect("append");
        log.sync().expect("sync");
        let before = log.len();
        log.truncate(before).expect("truncate to current len");
        assert_eq!(log.len(), before);
        assert_eq!(log.read_all().expect("read"), payload);
    }

    #[test]
    fn last_checkpoint_end_returns_zero_for_empty_log() {
        let tmp = TempDir::new().expect("tmp");
        let mut log = CanonicalLog::open(tmp.path().join("canonical.log")).expect("open");
        assert_eq!(log.last_checkpoint_end().expect("scan"), 0);
    }

    #[test]
    fn last_checkpoint_end_finds_the_final_checkpoint() {
        let tmp = TempDir::new().expect("tmp");
        let mut log = CanonicalLog::open(tmp.path().join("canonical.log")).expect("open");
        let cp_a = checkpoint_bytes(1);
        let cp_b = checkpoint_bytes(2);
        log.append(&cp_a).expect("append a");
        log.append(&cp_b).expect("append b");
        log.sync().expect("sync");
        let end = log.last_checkpoint_end().expect("scan");
        assert_eq!(end, (cp_a.len() + cp_b.len()) as u64);
    }

    #[test]
    fn last_checkpoint_end_stops_at_corruption() {
        let tmp = TempDir::new().expect("tmp");
        let mut log = CanonicalLog::open(tmp.path().join("canonical.log")).expect("open");
        let cp = checkpoint_bytes(1);
        log.append(&cp).expect("append");
        // Half-written orphan: a bare opcode byte with nothing following.
        log.append(&[0x01_u8]).expect("append garbage");
        log.sync().expect("sync");
        let end = log.last_checkpoint_end().expect("scan");
        // Scan stops at the committed CHECKPOINT boundary.
        assert_eq!(end, cp.len() as u64);
    }
}
