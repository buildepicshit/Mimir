//! Workspace store — the durable write path per
//! `docs/concepts/write-protocol.md`.
//!
//! [`Store`] owns a [`Pipeline`] and the
//! workspace's [`CanonicalLog`], and exposes
//! one public operation: [`Store::commit_batch`]. The commit runs the
//! full pipeline (parse → bind → semantic → emit), appends the emitted
//! records plus a `CHECKPOINT` marker to the log, and fsyncs. Any
//! failure at any phase is rolled back — the in-memory pipeline state
//! and the log both revert to their pre-batch values.
//!
//! Durability surface:
//!
//! - Two-phase commit per spec § 4 (append records + `CHECKPOINT`,
//!   fsync, ack).
//! - Mid-batch rollback via log truncation + pipeline-state restore.
//! - Recovery at open truncates crash-shaped orphan records past the
//!   last committed `CHECKPOINT` (spec § 10) and rejects
//!   non-recoverable corrupt tails without truncating them.
//! - Symbol-table replay: `SYMBOL_*` records emitted by the bind
//!   mutation journal (spec § 3.4) and the librarian-synthesized
//!   `__mem_{n}` / `__ep_{n}` allocations are decoded on `Store::open`
//!   and replayed into the pipeline's `SymbolTable`, restoring
//!   durably-committed state across process restarts. The monotonic
//!   memory and episode counters advance past the highest-numbered
//!   reserved-prefix symbol in the log.
//! - The `LogBackend` trait abstracts the filesystem primitives so
//!   tests can inject faults on `append` / `sync` / `truncate`; see
//!   the `FaultyLog` test backend in this module's tests.
//! - The spec § 7 failure-mode matrix is covered: rows 3 / 6 / 7
//!   directly (orphan memory record without `CHECKPOINT`; disk-full
//!   on append; fsync returns error). Rows 1 / 2 / 5 / 8 collapse to
//!   the recovery-on-next-open path already exercised by the reopen
//!   tests. Row 4 (crash between `CHECKPOINT` append and fsync) is
//!   physically untestable in user-space — its two possible outcomes
//!   collapse to row 3 (bytes not durable → orphan) or row 5 (bytes
//!   durable → committed).

use std::path::Path;

use thiserror::Error;

use crate::canonical::{
    decode_all, decode_record, encode_record, CanonicalRecord, CheckpointRecord, DecodeError,
    EpisodeMetaRecord, SymbolEventRecord,
};
use crate::clock::ClockTime;
use crate::log::{CanonicalLog, LogBackend, LogError};
use crate::pipeline::{Pipeline, PipelineError};
use crate::symbol::{SymbolId, SymbolKind};

/// Identifier for one committed Episode. Wraps the [`SymbolId`] stored
/// in the Episode's `CHECKPOINT` record.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct EpisodeId(SymbolId);

impl EpisodeId {
    /// The underlying symbol ID assigned to this Episode's `CHECKPOINT`
    /// record.
    #[must_use]
    pub const fn as_symbol(self) -> SymbolId {
        self.0
    }
}

/// The workspace store — a [`LogBackend`] plus the `Pipeline` that
/// produces its records. The default backend is [`CanonicalLog`] (real
/// filesystem); tests and crash-injection harnesses parameterize with
/// their own `LogBackend` implementation.
pub struct Store<L: LogBackend = CanonicalLog> {
    log: L,
    pipeline: Pipeline,
    next_episode_counter: u64,
}

impl Store<CanonicalLog> {
    /// Open or create a workspace at `path`. Convenience constructor
    /// that wires a real filesystem-backed [`CanonicalLog`].
    ///
    /// # Errors
    ///
    /// - [`StoreError::Log`] on any filesystem / I/O failure during
    ///   open, scan, or truncate.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let log = CanonicalLog::open(path).map_err(StoreError::Log)?;
        Self::from_backend(log)
    }

    /// Open or create a workspace-partitioned store under a shared
    /// `data_root`. The log lands at
    /// `data_root/<workspace_hex>/canonical.log` per
    /// `workspace-model.md` § 4.2. Parent directories are created on
    /// demand.
    ///
    /// Two `Store`s opened under the same `data_root` but different
    /// [`WorkspaceId`](crate::WorkspaceId) values land in disjoint
    /// directories — per spec § 2 the partition is **structural**, not
    /// policy-enforced.
    ///
    /// # Errors
    ///
    /// - [`StoreError::Log`] on any filesystem / I/O failure.
    pub fn open_in_workspace(
        data_root: impl AsRef<Path>,
        workspace_id: crate::WorkspaceId,
    ) -> Result<Self, StoreError> {
        // Use the full 32-byte hex digest for the directory name —
        // `Display` only shows 8 bytes, which could collide on large
        // workspace counts. The directory name is a filesystem path,
        // not a human-facing identifier.
        use std::fmt::Write;
        let mut hex = String::with_capacity(workspace_id.as_bytes().len() * 2);
        for b in workspace_id.as_bytes() {
            // Writing to a String cannot fail; the result is ignored.
            let _ = write!(hex, "{b:02x}");
        }
        let workspace_dir = data_root.as_ref().join(&hex);
        std::fs::create_dir_all(&workspace_dir)
            .map_err(|e| StoreError::Log(crate::log::LogError::Io(e)))?;
        let log_path = workspace_dir.join("canonical.log");
        let log = CanonicalLog::open(log_path).map_err(StoreError::Log)?;
        Self::from_backend(log)
    }
}

impl<L: LogBackend> Store<L> {
    /// Construct a `Store` over an arbitrary [`LogBackend`]. On open,
    /// crash-shaped orphan bytes past the last durable `CHECKPOINT`
    /// are truncated (spec § 10 recovery step), non-recoverable tail
    /// corruption is rejected without truncation, and `SYMBOL_*`
    /// events from the committed log are replayed into the pipeline's
    /// symbol table so workspace state fully reconstructs across
    /// process restarts.
    ///
    /// # Errors
    ///
    /// - [`StoreError::Log`] on any backend I/O failure.
    /// - [`StoreError::CorruptTail`] if non-recoverable bytes are
    ///   found past the last durable `CHECKPOINT`.
    /// - [`StoreError::Pipeline`] if replay of a `SYMBOL_*` record
    ///   fails (log corruption).
    pub fn from_backend(mut log: L) -> Result<Self, StoreError> {
        let log_len_on_open = log.len();
        let bytes_on_open = log.read_all().map_err(StoreError::Log)?;
        let committed_end = Self::committed_end_for_open(&bytes_on_open)?;
        if committed_end < bytes_on_open.len() {
            let committed_end_u64 =
                u64::try_from(committed_end).map_err(|_| StoreError::Log(LogError::LogOverflow))?;
            let orphan_bytes = log_len_on_open - committed_end_u64;
            log.truncate(committed_end_u64).map_err(StoreError::Log)?;
            tracing::warn!(
                target: "mimir.recovery.orphan_truncated",
                log_len_before = log_len_on_open,
                committed_end = committed_end_u64,
                orphan_bytes,
                "truncated orphan bytes past last CHECKPOINT on open",
            );
        }
        // Replay SYMBOL_* events so the pipeline's table reflects
        // durably-committed state. Counters are advanced past the
        // highest-numbered `__mem_{n}` / `__ep_{n}` symbol seen.
        // Tail recovery already discarded orphan bytes; a decode
        // failure below this point is genuine corruption in the
        // committed region of the log and routes to a distinct error
        // variant so callers can distinguish it from I/O, truncation,
        // or tail-corruption errors.
        let records = decode_all(&bytes_on_open[..committed_end])?;
        let mut pipeline = Pipeline::new();
        let mut next_memory_counter = 0_u64;
        let mut next_episode_counter = 0_u64;
        let mut symbol_alloc_count = 0_u64;
        let mut symbol_mutation_count = 0_u64;
        let mut checkpoint_count = 0_u64;
        for record in records {
            // Restore the monotonic commit watermark from every
            // replayed record's commit time so post-reopen batches
            // keep the `committed_at` monotonicity invariant
            // (temporal-model.md § 9.2 / § 12 #1).
            pipeline.advance_last_committed_at(record.committed_at());
            // Replay supersession edges into the DAG, checking
            // acyclicity (§ 6.2 #1). If an edge appears before the
            // first batch has advanced the DAG, this is trivially OK.
            if let Some(edge) = crate::dag::Edge::try_from_record(&record) {
                pipeline.replay_edge(edge)?;
            }
            // Replay memory records into the supersession-detection
            // indices so post-open batches can auto-supersede (§ 5).
            pipeline.replay_memory_record(&record);
            // Flag events update the pinned / authoritative sets.
            pipeline.replay_flag(&record);
            match record {
                CanonicalRecord::SymbolAlloc(event) => {
                    pipeline
                        .replay_allocate(event.symbol_id, event.name.clone(), event.symbol_kind)
                        .map_err(|e| StoreError::Pipeline(PipelineError::Bind(e)))?;
                    Self::advance_reserved_counter("__mem_", &event.name, &mut next_memory_counter);
                    Self::advance_reserved_counter("__ep_", &event.name, &mut next_episode_counter);
                    symbol_alloc_count += 1;
                }
                CanonicalRecord::SymbolAlias(event) => {
                    pipeline
                        .replay_alias(event.symbol_id, event.name)
                        .map_err(|e| StoreError::Pipeline(PipelineError::Bind(e)))?;
                    symbol_mutation_count += 1;
                }
                CanonicalRecord::SymbolRename(event) => {
                    pipeline
                        .replay_rename(event.symbol_id, event.name)
                        .map_err(|e| StoreError::Pipeline(PipelineError::Bind(e)))?;
                    symbol_mutation_count += 1;
                }
                CanonicalRecord::SymbolRetire(event) => {
                    pipeline
                        .replay_retire(event.symbol_id, event.name)
                        .map_err(|e| StoreError::Pipeline(PipelineError::Bind(e)))?;
                    symbol_mutation_count += 1;
                }
                CanonicalRecord::Checkpoint(cp) => {
                    // Register the replayed Episode with the pipeline
                    // so post-open Episode-scoped reads see it.
                    pipeline.register_episode(cp.episode_id, cp.at);
                    checkpoint_count += 1;
                }
                CanonicalRecord::EpisodeMeta(meta) => {
                    // Restore the Episode index too — register_episode
                    // is idempotent if the following Checkpoint
                    // re-registers with the same clock.
                    pipeline.register_episode(meta.episode_id, meta.at);
                    if let Some(parent) = meta.parent_episode_id {
                        pipeline.register_episode_parent(meta.episode_id, parent);
                    }
                }
                _ => {}
            }
        }
        pipeline.set_next_memory_counter(next_memory_counter);
        // Emit a recovery summary only when there's actually committed
        // state to report — a fresh store should stay silent.
        if symbol_alloc_count > 0 || symbol_mutation_count > 0 || checkpoint_count > 0 {
            tracing::info!(
                target: "mimir.recovery.symbol_replay",
                symbol_alloc_count,
                symbol_mutation_count,
                checkpoint_count,
                next_memory_counter,
                next_episode_counter,
                "replayed committed log into pipeline on open",
            );
        }
        Ok(Self {
            log,
            pipeline,
            next_episode_counter,
        })
    }

    fn committed_end_for_open(bytes: &[u8]) -> Result<usize, StoreError> {
        let mut pos = 0_usize;
        let mut last_checkpoint_end = 0_usize;
        while pos < bytes.len() {
            match decode_record(&bytes[pos..]) {
                Ok((record, consumed)) => {
                    pos += consumed;
                    if matches!(record, CanonicalRecord::Checkpoint(_)) {
                        last_checkpoint_end = pos;
                    }
                }
                Err(source) if Self::is_recoverable_tail_decode_error(&source) => {
                    return Ok(last_checkpoint_end);
                }
                Err(source) => {
                    let offset =
                        u64::try_from(pos).map_err(|_| StoreError::Log(LogError::LogOverflow))?;
                    return Err(StoreError::CorruptTail { offset, source });
                }
            }
        }
        Ok(last_checkpoint_end)
    }

    const fn is_recoverable_tail_decode_error(error: &DecodeError) -> bool {
        matches!(
            error,
            DecodeError::Truncated { .. } | DecodeError::LengthMismatch { .. }
        )
    }

    fn advance_reserved_counter(prefix: &str, name: &str, counter: &mut u64) {
        if let Some(suffix) = name.strip_prefix(prefix) {
            if let Ok(n) = suffix.parse::<u64>() {
                if n + 1 > *counter {
                    *counter = n + 1;
                }
            }
        }
    }

    /// Committed log length in bytes.
    #[must_use]
    pub fn log_len(&self) -> u64 {
        self.log.len()
    }

    /// Read-only view of the underlying pipeline. Used by callers
    /// that want to issue read-path queries (`execute_query`) or
    /// inspect pipeline state without owning the whole store.
    #[must_use]
    pub fn pipeline(&self) -> &Pipeline {
        &self.pipeline
    }

    /// Mutable view of the pipeline. Exposed so tests and
    /// downstream callers can call `execute_query` (which needs
    /// `&self`, not `&mut self`, but the mut accessor keeps the
    /// door open for future read-path methods that do require
    /// exclusive borrow).
    pub fn pipeline_mut(&mut self) -> &mut Pipeline {
        &mut self.pipeline
    }

    /// Compile a batch of agent input and commit it atomically.
    ///
    /// The two phases run under the workspace's single-writer invariant:
    ///
    /// 1. Pipeline compiles the input into a `Vec<CanonicalRecord>`. On
    ///    pipeline error the pipeline's in-memory state is already
    ///    auto-rolled-back (per `Pipeline::compile_batch`'s clone-on-
    ///    write contract) and no log bytes have been written.
    /// 2. Records + a `CHECKPOINT` marker are appended to the log.
    /// 3. The log is fsynced. On success the batch is durable and the
    ///    new Episode ID is returned; on fsync failure the log is
    ///    truncated to its pre-batch offset and the pipeline's
    ///    in-memory state is restored from a snapshot taken before
    ///    step 1.
    ///
    /// # Errors
    ///
    /// - [`StoreError::Pipeline`] if parse / bind / semantic / emit
    ///   rejected the batch. In-memory state is unchanged; log is
    ///   untouched.
    /// - [`StoreError::Log`] if the append / sync / truncate sequence
    ///   failed at any step. In-memory pipeline state is restored to
    ///   its pre-batch snapshot; log is truncated back to pre-batch.
    pub fn commit_batch(&mut self, input: &str, now: ClockTime) -> Result<EpisodeId, StoreError> {
        self.commit_batch_with_metadata(input, now, &EpisodeMetadata::default())
    }

    /// Commit a batch and attach agent-visible Episode metadata
    /// (label, `parent_episode`, retracts) per `episode-semantics.md`
    /// § 4.2 / § 5. Same commit semantics as [`Self::commit_batch`];
    /// when `metadata` is non-empty, an `EpisodeMeta` canonical
    /// record is emitted immediately before the `CHECKPOINT`.
    ///
    /// # Errors
    ///
    /// Same as [`Self::commit_batch`]. If `metadata.label` exceeds
    /// the 256-byte cap (spec § 4.3) the commit fails with a
    /// [`StoreError::InvalidEpisodeMetadata`] before any log writes.
    pub fn commit_batch_with_metadata(
        &mut self,
        input: &str,
        now: ClockTime,
        metadata: &EpisodeMetadata,
    ) -> Result<EpisodeId, StoreError> {
        // observability.md: `mimir.commit.batch` span wraps the full
        // commit. Fields recorded after each phase so timing stays
        // attached even on error paths.
        let span = tracing::info_span!(
            "mimir.commit.batch",
            log_offset_before = self.log.len(),
            log_offset_after = tracing::field::Empty,
            record_count = tracing::field::Empty,
            episode_id = tracing::field::Empty,
            fsync_micros = tracing::field::Empty,
        );
        let _enter = span.enter();

        metadata.validate()?;
        let pipeline_snapshot = self.pipeline.clone();
        let episode_counter_snapshot = self.next_episode_counter;
        let log_len_before = self.log.len();

        // Phase 0: compile. compile_batch's internal clone-on-write
        // means a pipeline error leaves self.pipeline untouched; a
        // successful compile auto-applies the working state.
        let records = self.pipeline.compile_batch(input, now)?;

        // If the batch carried an `(episode :start …)` form, the
        // pipeline captured its metadata. Merge with the explicit
        // `metadata` arg — form-level metadata wins on conflict
        // because the agent wrote it directly into the batch.
        let pending = self.pipeline.take_pending_episode_metadata();
        let mut resolved_meta = metadata.clone();
        if let Some(p) = pending {
            if p.label.is_some() {
                resolved_meta.label = p.label;
            }
            if p.parent_episode.is_some() {
                resolved_meta.parent_episode = p.parent_episode;
            }
            if !p.retracts.is_empty() {
                resolved_meta.retracts = p.retracts;
            }
            // Re-validate since form-level label may exceed cap
            // (bind already checks, but defence-in-depth).
            resolved_meta.validate()?;
        }

        // The pipeline monotonically advances `committed_at` past any
        // previous batch (temporal-model.md § 9.2). The checkpoint and
        // episode-alloc records must use that same advanced clock —
        // stamping them with raw wall-clock `now` would violate the
        // per-workspace monotonicity invariant on a regressed clock.
        let effective_now = self.pipeline.last_committed_at().unwrap_or(now);

        // Phase 1: append each record plus a closing CHECKPOINT.
        let episode_id = self
            .pipeline
            .allocate_episode_symbol(self.next_episode_counter)
            .map_err(|e| {
                // Compile succeeded and mutated the pipeline; roll back.
                self.pipeline = pipeline_snapshot.clone();
                self.next_episode_counter = episode_counter_snapshot;
                StoreError::Pipeline(PipelineError::Emit(e))
            })?;
        self.next_episode_counter += 1;

        let checkpoint = CheckpointRecord {
            episode_id,
            at: effective_now,
            memory_count: memory_record_count(&records),
        };

        // Emit a SymbolAlloc record for the synthesized __ep_{n}
        // episode symbol so replay can reconstruct it. This sits
        // between the pipeline's journal-derived SymbolAlloc records
        // and the memory records; replay treats it like any other
        // SymbolAlloc.
        let episode_alloc = CanonicalRecord::SymbolAlloc(SymbolEventRecord {
            symbol_id: episode_id,
            name: format!("__ep_{episode_counter_snapshot}"),
            symbol_kind: SymbolKind::Memory,
            at: effective_now,
        });

        let episode_meta = resolved_meta.to_record(episode_id, effective_now);

        let mut buf = Vec::new();
        encode_record(&episode_alloc, &mut buf);
        for r in &records {
            encode_record(r, &mut buf);
        }
        if let Some(ref meta_rec) = episode_meta {
            encode_record(&CanonicalRecord::EpisodeMeta(meta_rec.clone()), &mut buf);
        }
        encode_record(&CanonicalRecord::Checkpoint(checkpoint), &mut buf);

        if let Err(e) = self.log.append(&buf) {
            self.rollback(&pipeline_snapshot, episode_counter_snapshot, log_len_before)?;
            return Err(StoreError::Log(e));
        }

        // Phase 2: fsync. Per spec § 7, an fsync failure is treated as
        // uncommitted — roll back log + pipeline.
        let fsync_start = std::time::Instant::now();
        if let Err(e) = self.log.sync() {
            self.rollback(&pipeline_snapshot, episode_counter_snapshot, log_len_before)?;
            return Err(StoreError::Log(e));
        }
        let fsync_micros = u64::try_from(fsync_start.elapsed().as_micros()).unwrap_or(u64::MAX);

        // Post-commit: register the Episode's metadata with the
        // pipeline so Episode-scoped reads (`read-protocol.md`
        // § 4.1) can resolve `:in_episode` / `:after_episode` /
        // `:before_episode` against this commit's clock.
        self.pipeline.register_episode(episode_id, effective_now);
        if let Some(ref meta_rec) = episode_meta {
            if let Some(parent) = meta_rec.parent_episode_id {
                self.pipeline.register_episode_parent(episode_id, parent);
            }
        }

        span.record("log_offset_after", self.log.len());
        span.record("record_count", records.len());
        span.record("episode_id", tracing::field::display(episode_id));
        span.record("fsync_micros", fsync_micros);

        Ok(EpisodeId(episode_id))
    }

    /// Restore pipeline + episode-counter snapshot and truncate the log
    /// back to `log_len_before`. Helper used by `commit_batch` on any
    /// Phase 1 / Phase 2 failure.
    fn rollback(
        &mut self,
        pipeline_snapshot: &Pipeline,
        episode_counter_snapshot: u64,
        log_len_before: u64,
    ) -> Result<(), StoreError> {
        self.pipeline = pipeline_snapshot.clone();
        self.next_episode_counter = episode_counter_snapshot;
        // Best-effort log truncation. If this fails too, the log has
        // orphan bytes past log_len_before; recovery on the next open
        // will truncate them via last_checkpoint_end(). Propagate the
        // secondary error for diagnosability.
        if self.log.len() > log_len_before {
            self.log.truncate(log_len_before).map_err(StoreError::Log)?;
        }
        Ok(())
    }
}

fn memory_record_count(records: &[CanonicalRecord]) -> u64 {
    records
        .iter()
        .filter(|record| {
            matches!(
                record,
                CanonicalRecord::Sem(_)
                    | CanonicalRecord::Epi(_)
                    | CanonicalRecord::Pro(_)
                    | CanonicalRecord::Inf(_)
            )
        })
        .count() as u64
}

/// Errors produced by [`Store`].
#[derive(Debug, Error)]
pub enum StoreError {
    /// A pipeline stage (parse / bind / semantic / emit) rejected the
    /// batch. In-memory state and log are both untouched.
    #[error("pipeline error: {0}")]
    Pipeline(#[from] PipelineError),

    /// A filesystem / I/O failure during append, sync, or truncate.
    /// On commit-time failures the pipeline and log are rolled back to
    /// their pre-batch state before this error is returned.
    #[error("log error: {0}")]
    Log(#[from] LogError),

    /// Non-recoverable bytes were found after the last durable
    /// `CHECKPOINT` during `Store::open`. Unlike crash-shaped orphan
    /// tails (`Truncated` / `LengthMismatch`) or valid uncommitted
    /// records, these bytes are preserved for inspection or restore
    /// rather than silently truncated.
    #[error("corrupt canonical log tail at offset {offset}: {source}")]
    CorruptTail {
        /// Logical byte offset where corrupt tail decoding failed.
        offset: u64,
        /// The underlying [`DecodeError`] from `canonical::decode_record`.
        source: DecodeError,
    },

    /// The committed portion of the log (bytes before the last
    /// `CHECKPOINT` fsync) failed to decode during `Store::open`. This
    /// is distinct from tail recovery and indicates genuine
    /// corruption in the durable store.
    #[error("committed canonical log failed to decode: {source}")]
    CorruptCommittedLog {
        /// The underlying [`DecodeError`] from `canonical::decode_all`.
        #[from]
        source: crate::canonical::DecodeError,
    },

    /// A supersession edge replayed from the committed log failed its
    /// acyclicity check. The on-disk edges are expected to satisfy
    /// `temporal-model.md` § 6.2 invariant #1; surfacing as a typed
    /// error on open keeps silent invariant violations out of the
    /// reopened store.
    #[error("supersession DAG replay failed: {source}")]
    DagReplay {
        /// The underlying [`DagError`](crate::dag::DagError).
        #[from]
        source: crate::dag::DagError,
    },

    /// Supplied [`EpisodeMetadata`] violates a
    /// `episode-semantics.md` constraint — e.g. a `label` exceeding
    /// the 256-byte cap (§ 4.3).
    #[error("invalid episode metadata: {reason}")]
    InvalidEpisodeMetadata {
        /// Human-readable description of the failed constraint.
        reason: &'static str,
    },
}

/// Agent-supplied Episode metadata. Passed into
/// [`Store::commit_batch_with_metadata`] to attach a label / parent /
/// retracts to the next committed Episode. See
/// `episode-semantics.md` § 3.2 / § 4.2 / § 5.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EpisodeMetadata {
    /// Optional human-readable label.
    pub label: Option<String>,
    /// Optional parent Episode.
    pub parent_episode: Option<SymbolId>,
    /// Episodes this Episode retracts.
    pub retracts: Vec<SymbolId>,
}

impl EpisodeMetadata {
    /// Spec § 4.3 cap.
    pub const MAX_LABEL_BYTES: usize = 256;

    /// True if no metadata is attached; the commit path skips
    /// emitting an `EpisodeMeta` record in this case.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.label.as_deref().is_none_or(str::is_empty)
            && self.parent_episode.is_none()
            && self.retracts.is_empty()
    }

    /// Spec § 4.3: labels cap at 256 bytes.
    fn validate(&self) -> Result<(), StoreError> {
        if let Some(label) = self.label.as_deref() {
            if label.len() > Self::MAX_LABEL_BYTES {
                return Err(StoreError::InvalidEpisodeMetadata {
                    reason: "label exceeds 256-byte cap",
                });
            }
        }
        Ok(())
    }

    /// Convert to a canonical `EpisodeMetaRecord` for the given
    /// Episode and commit time. Returns `None` when
    /// [`Self::is_empty`] — no metadata record is emitted for bare
    /// (implicit-Episode) commits.
    fn to_record(&self, episode_id: SymbolId, at: ClockTime) -> Option<EpisodeMetaRecord> {
        if self.is_empty() {
            return None;
        }
        Some(EpisodeMetaRecord {
            episode_id,
            at,
            label: self.label.clone().filter(|s| !s.is_empty()),
            parent_episode_id: self.parent_episode,
            retracts: self.retracts.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::{decode_all, decode_record, CanonicalRecord};
    use crate::read::{Framing, FramingSource, ReadFlags};
    use tempfile::TempDir;

    const SEM_OK: &str = "(sem @alice @knows @bob :src @observation :c 0.8 :v 2024-01-15)";
    const SEM_OK_2: &str = "(sem @alice @likes @carol :src @observation :c 0.7 :v 2024-01-16)";

    fn fixed_now() -> ClockTime {
        ClockTime::try_from_millis(1_713_350_400_000).expect("non-sentinel")
    }

    fn open_fresh(dir: &TempDir) -> Store {
        Store::open(dir.path().join("canonical.log")).expect("open")
    }

    // ----------------------------------------------------------
    // FaultyLog — in-memory LogBackend with armable failure hooks.
    // ----------------------------------------------------------

    #[derive(Default)]
    struct FaultyLog {
        bytes: Vec<u8>,
        fail_next_append: Option<std::io::ErrorKind>,
        fail_next_sync: Option<std::io::ErrorKind>,
        fail_next_truncate: Option<std::io::ErrorKind>,
    }

    impl FaultyLog {
        fn new() -> Self {
            Self::default()
        }

        fn arm_append_failure(&mut self, kind: std::io::ErrorKind) {
            self.fail_next_append = Some(kind);
        }

        fn arm_sync_failure(&mut self, kind: std::io::ErrorKind) {
            self.fail_next_sync = Some(kind);
        }

        fn arm_truncate_failure(&mut self, kind: std::io::ErrorKind) {
            self.fail_next_truncate = Some(kind);
        }
    }

    impl LogBackend for FaultyLog {
        fn append(&mut self, bytes: &[u8]) -> Result<(), LogError> {
            if let Some(kind) = self.fail_next_append.take() {
                return Err(LogError::Io(std::io::Error::from(kind)));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(())
        }

        fn sync(&mut self) -> Result<(), LogError> {
            if let Some(kind) = self.fail_next_sync.take() {
                return Err(LogError::Io(std::io::Error::from(kind)));
            }
            Ok(())
        }

        fn truncate(&mut self, new_len: u64) -> Result<(), LogError> {
            if let Some(kind) = self.fail_next_truncate.take() {
                return Err(LogError::Io(std::io::Error::from(kind)));
            }
            let current = self.bytes.len() as u64;
            if new_len > current {
                return Err(LogError::TruncateBeyondEnd {
                    requested: new_len,
                    current,
                });
            }
            let new_len_usize = usize::try_from(new_len).unwrap_or(self.bytes.len());
            self.bytes.truncate(new_len_usize);
            Ok(())
        }

        fn read_all(&mut self) -> Result<Vec<u8>, LogError> {
            Ok(self.bytes.clone())
        }

        fn len(&self) -> u64 {
            self.bytes.len() as u64
        }

        fn last_checkpoint_end(&mut self) -> Result<u64, LogError> {
            let mut pos: usize = 0;
            let mut last_checkpoint_end: u64 = 0;
            while pos < self.bytes.len() {
                match decode_record(&self.bytes[pos..]) {
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

    #[test]
    fn commit_single_batch_persists_records_and_checkpoint() {
        let dir = TempDir::new().expect("tmp");
        let mut store = open_fresh(&dir);
        let _ = store.commit_batch(SEM_OK, fixed_now()).expect("commit");

        // Log content: [SymbolAlloc...][Sem][Checkpoint]. Last record
        // must be a Checkpoint; exactly one memory record (Sem).
        let bytes = store.log.read_all().expect("read");
        let records = decode_all(&bytes).expect("decode");
        assert!(matches!(
            records.last(),
            Some(CanonicalRecord::Checkpoint(_))
        ));
        let checkpoint = records
            .iter()
            .find_map(|r| match r {
                CanonicalRecord::Checkpoint(c) => Some(c),
                _ => None,
            })
            .expect("checkpoint");
        assert_eq!(
            checkpoint.memory_count, 1,
            "checkpoint memory_count must count memory records, not symbol events"
        );
        let mem_count = records
            .iter()
            .filter(|r| matches!(r, CanonicalRecord::Sem(_)))
            .count();
        assert_eq!(mem_count, 1);
    }

    #[test]
    fn commit_registers_episode_with_pipeline() {
        let dir = TempDir::new().expect("tmp");
        let mut store = open_fresh(&dir);
        let first = store.commit_batch(SEM_OK, fixed_now()).expect("first");
        let second = store.commit_batch(SEM_OK_2, fixed_now()).expect("second");

        // Both Episodes must be registered with the Pipeline for
        // post-commit `:in_episode` / `:after_episode` reads to
        // resolve. Query via the real `(query ...)` form.
        let got1 = store
            .pipeline_mut()
            .execute_query("(query :in_episode @__ep_0)")
            .expect("q1");
        assert_eq!(got1.records.len(), 1, "first Episode holds SEM_OK");

        let got2 = store
            .pipeline_mut()
            .execute_query("(query :after_episode @__ep_0)")
            .expect("q2");
        assert_eq!(got2.records.len(), 1, "SEM_OK_2 commits after __ep_0");

        // Episode IDs are sequential synthesized symbols.
        assert_ne!(first, second);
    }

    #[test]
    fn replay_registers_episodes_with_pipeline() {
        // Round-trip through `Store::open` — reopening should
        // restore the episodes-by-committed-at index so post-reopen
        // Episode-scoped reads still work.
        let dir = TempDir::new().expect("tmp");
        {
            let mut store = open_fresh(&dir);
            store.commit_batch(SEM_OK, fixed_now()).expect("first");
        }
        let mut reopened = open_fresh(&dir);
        let got = reopened
            .pipeline_mut()
            .execute_query("(query :in_episode @__ep_0)")
            .expect("query");
        assert_eq!(
            got.records.len(),
            1,
            "replay must re-register Episodes with the pipeline"
        );
    }

    #[test]
    fn commit_with_metadata_emits_episode_meta_record() {
        let dir = TempDir::new().expect("tmp");
        let mut store = open_fresh(&dir);
        let first = store.commit_batch(SEM_OK, fixed_now()).expect("first");
        let meta = EpisodeMetadata {
            label: Some("design-session".into()),
            parent_episode: Some(first.0),
            retracts: Vec::new(),
        };
        store
            .commit_batch_with_metadata(SEM_OK_2, fixed_now(), &meta)
            .expect("second with metadata");

        let bytes = store.log.read_all().expect("read");
        let records = decode_all(&bytes).expect("decode");
        let meta_count = records
            .iter()
            .filter(|r| matches!(r, CanonicalRecord::EpisodeMeta(_)))
            .count();
        assert_eq!(
            meta_count, 1,
            "only the metadata-carrying commit should emit an EpisodeMeta"
        );
        // Find the metadata record and inspect it.
        let meta_rec = records
            .iter()
            .find_map(|r| match r {
                CanonicalRecord::EpisodeMeta(m) => Some(m),
                _ => None,
            })
            .expect("EpisodeMeta present");
        assert_eq!(meta_rec.label.as_deref(), Some("design-session"));
        assert_eq!(meta_rec.parent_episode_id, Some(first.0));
    }

    #[test]
    fn episode_chain_walks_parent_links_after_replay() {
        let dir = TempDir::new().expect("tmp");
        // Commit three Episodes linked parent → child → grandchild.
        let (first, second, third);
        {
            let mut store = open_fresh(&dir);
            first = store.commit_batch(SEM_OK, fixed_now()).expect("first");
            second = store
                .commit_batch_with_metadata(
                    "(sem @alice @likes @carol :src @observation :c 0.7 :v 2024-01-16)",
                    fixed_now(),
                    &EpisodeMetadata {
                        label: None,
                        parent_episode: Some(first.0),
                        retracts: Vec::new(),
                    },
                )
                .expect("second");
            third = store
                .commit_batch_with_metadata(
                    "(sem @charlie @knows @dana :src @observation :c 0.7 :v 2024-01-17)",
                    fixed_now(),
                    &EpisodeMetadata {
                        label: None,
                        parent_episode: Some(second.0),
                        retracts: Vec::new(),
                    },
                )
                .expect("third");
        }

        // Reopen and query `:episode_chain @third` — should return
        // memories from all three Episodes (third walks back to
        // second walks back to first).
        let mut reopened = open_fresh(&dir);
        let _ = (first, second, third);
        let got = reopened
            .pipeline_mut()
            .execute_query("(query :episode_chain @__ep_2)")
            .expect("query");
        assert_eq!(
            got.records.len(),
            3,
            "episode_chain over three linked Episodes returns all three memories"
        );
    }

    #[test]
    fn label_exceeding_cap_rejects() {
        let dir = TempDir::new().expect("tmp");
        let mut store = open_fresh(&dir);
        let bad_label = "x".repeat(EpisodeMetadata::MAX_LABEL_BYTES + 1);
        let err = store
            .commit_batch_with_metadata(
                SEM_OK,
                fixed_now(),
                &EpisodeMetadata {
                    label: Some(bad_label),
                    parent_episode: None,
                    retracts: Vec::new(),
                },
            )
            .expect_err("label too long");
        assert!(matches!(
            err,
            StoreError::InvalidEpisodeMetadata { reason } if reason.contains("256")
        ));
    }

    #[test]
    fn episode_start_form_writes_episode_meta_end_to_end() {
        // `(episode :start :label …)` in the batch produces an
        // `EpisodeMeta` record in the log.
        let dir = TempDir::new().expect("tmp");
        let mut store = open_fresh(&dir);
        let input = r#"(episode :start :label "design-session")
                       (sem @alice @knows @bob :src @observation :c 0.8 :v 2024-01-15)"#;
        store.commit_batch(input, fixed_now()).expect("commit");

        let bytes = store.log.read_all().expect("read");
        let records = decode_all(&bytes).expect("decode");
        let meta = records
            .iter()
            .find_map(|r| match r {
                CanonicalRecord::EpisodeMeta(m) => Some(m),
                _ => None,
            })
            .expect("EpisodeMeta present");
        assert_eq!(meta.label.as_deref(), Some("design-session"));
    }

    #[test]
    fn episode_close_form_is_accepted_no_op() {
        // `(episode :close)` parses valid and commits without
        // emitting an EpisodeMeta record (no metadata to carry).
        let dir = TempDir::new().expect("tmp");
        let mut store = open_fresh(&dir);
        let input = "(sem @alice @knows @bob :src @observation :c 0.8 :v 2024-01-15)
                       (episode :close)";
        store.commit_batch(input, fixed_now()).expect("commit");

        let bytes = store.log.read_all().expect("read");
        let records = decode_all(&bytes).expect("decode");
        let meta_count = records
            .iter()
            .filter(|r| matches!(r, CanonicalRecord::EpisodeMeta(_)))
            .count();
        assert_eq!(
            meta_count, 0,
            ":close alone carries no metadata; no EpisodeMeta record"
        );
    }

    #[test]
    fn episode_start_with_parent_links_chain() {
        let dir = TempDir::new().expect("tmp");
        let mut store = open_fresh(&dir);
        let first = store
            .commit_batch(
                r#"(episode :start :label "parent")
                   (sem @alice @knows @bob :src @observation :c 0.8 :v 2024-01-15)"#,
                fixed_now(),
            )
            .expect("parent");
        // Second batch references the first Episode via the
        // write-surface form.
        let second_input = "(episode :start :parent_episode @__ep_0)\n\
             (sem @alice @likes @carol :src @observation :c 0.7 :v 2024-01-16)";
        store
            .commit_batch(second_input, fixed_now())
            .expect("child");

        // `:episode_chain @__ep_1` should return records from both
        // Episodes.
        let got = store
            .pipeline_mut()
            .execute_query("(query :episode_chain @__ep_1)")
            .expect("query");
        assert_eq!(
            got.records.len(),
            2,
            "chain walk returns both linked Episodes (got {first:?})"
        );
    }

    #[test]
    fn episode_start_with_retracts_records_metadata() {
        let dir = TempDir::new().expect("tmp");
        let mut store = open_fresh(&dir);
        let _bad = store
            .commit_batch(SEM_OK, fixed_now())
            .expect("bad episode");
        // Next batch retracts the first Episode via the write surface.
        // Distinct valid_at avoids the equal-valid_at auto-supersession
        // conflict (spec § 5.1 — two memories at the same `(s, p)` can't
        // share valid_at under the single-writer invariant).
        let input = r#"(episode :start :label "correction" :retracts (@__ep_0))
                       (sem @alice @knows @charlie :src @observation :c 0.95 :v 2024-01-16)"#;
        store.commit_batch(input, fixed_now()).expect("correction");

        let bytes = store.log.read_all().expect("read");
        let records = decode_all(&bytes).expect("decode");
        let meta = records
            .iter()
            .find_map(|r| match r {
                CanonicalRecord::EpisodeMeta(m) => Some(m),
                _ => None,
            })
            .expect("EpisodeMeta present on the correction batch");
        assert_eq!(meta.retracts.len(), 1);
        assert_eq!(meta.label.as_deref(), Some("correction"));
    }

    #[test]
    fn two_episode_directives_in_one_batch_reject() {
        let dir = TempDir::new().expect("tmp");
        let mut store = open_fresh(&dir);
        let input = r#"(episode :start :label "a")
                       (episode :start :label "b")
                       (sem @alice @knows @bob :src @observation :c 0.8 :v 2024-01-15)"#;
        let err = store
            .commit_batch(input, fixed_now())
            .expect_err("multiple episode directives must reject");
        assert!(matches!(
            err,
            StoreError::Pipeline(PipelineError::Semantic(
                crate::semantic::SemanticError::MultipleEpisodeDirectives { count: 2 }
            ))
        ));
    }

    #[test]
    fn pin_suspends_decay_and_flags_authoritative() {
        // Ancient Sem that would normally decay below 0.5 effective;
        // pinning it should lift effective back to stored and surface
        // Framing::Authoritative { set_by: AgentPinned }.
        let dir = TempDir::new().expect("tmp");
        let mut store = open_fresh(&dir);
        let old_sem = "(sem @mira @saw @kilroy :src @observation :c 0.8 :v 2023-12-01)";
        let _ = store.commit_batch(old_sem, fixed_now()).expect("old sem");

        // Before pin — LOW_CONFIDENCE should fire (decay applies).
        let before = store
            .pipeline_mut()
            .execute_query("(query)")
            .expect("before");
        assert!(
            before.flags.contains(ReadFlags::LOW_CONFIDENCE),
            "decayed stored 0.8 should be < 0.5 before pin"
        );

        // Pin the memory via the write surface.
        store
            .commit_batch("(pin @__mem_0 :actor @mira)", fixed_now())
            .expect("pin");

        // After pin — decay suspended, flag clears, framing surfaces as Authoritative.
        let after = store
            .pipeline_mut()
            .execute_query("(query :show_framing true)")
            .expect("after");
        assert!(
            !after.flags.contains(ReadFlags::LOW_CONFIDENCE),
            "pin must suspend decay"
        );
        assert_eq!(after.framings.len(), 1);
        assert_eq!(
            after.framings[0],
            Framing::Authoritative {
                set_by: FramingSource::AgentPinned
            }
        );
    }

    #[test]
    fn unpin_restores_decay() {
        let dir = TempDir::new().expect("tmp");
        let mut store = open_fresh(&dir);
        let old_sem = "(sem @mira @saw @kilroy :src @observation :c 0.8 :v 2023-12-01)";
        store.commit_batch(old_sem, fixed_now()).expect("old sem");
        store
            .commit_batch("(pin @__mem_0 :actor @mira)", fixed_now())
            .expect("pin");
        store
            .commit_batch("(unpin @__mem_0 :actor @mira)", fixed_now())
            .expect("unpin");

        let got = store
            .pipeline_mut()
            .execute_query("(query)")
            .expect("query");
        assert!(
            got.flags.contains(ReadFlags::LOW_CONFIDENCE),
            "unpin should restore decay"
        );
    }

    #[test]
    fn authoritative_set_surfaces_operator_framing() {
        let dir = TempDir::new().expect("tmp");
        let mut store = open_fresh(&dir);
        let sem = "(sem @alice @knows @bob :src @observation :c 0.8 :v 2024-01-15)";
        store.commit_batch(sem, fixed_now()).expect("sem");
        store
            .commit_batch("(authoritative_set @__mem_0 :actor @operator)", fixed_now())
            .expect("auth-set");

        let got = store
            .pipeline_mut()
            .execute_query("(query :show_framing true)")
            .expect("query");
        assert_eq!(got.framings.len(), 1);
        assert_eq!(
            got.framings[0],
            Framing::Authoritative {
                set_by: FramingSource::OperatorAuthoritative
            }
        );
    }

    #[test]
    fn authoritative_clear_resumes_decay() {
        let dir = TempDir::new().expect("tmp");
        let mut store = open_fresh(&dir);
        let old_sem = "(sem @mira @saw @kilroy :src @observation :c 0.8 :v 2023-12-01)";
        store.commit_batch(old_sem, fixed_now()).expect("sem");
        store
            .commit_batch("(authoritative_set @__mem_0 :actor @operator)", fixed_now())
            .expect("set");
        store
            .commit_batch(
                "(authoritative_clear @__mem_0 :actor @operator)",
                fixed_now(),
            )
            .expect("clear");

        let got = store
            .pipeline_mut()
            .execute_query("(query)")
            .expect("query");
        assert!(
            got.flags.contains(ReadFlags::LOW_CONFIDENCE),
            "clear should restore decay"
        );
    }

    #[test]
    fn pin_replay_survives_reopen() {
        let dir = TempDir::new().expect("tmp");
        let old_sem = "(sem @mira @saw @kilroy :src @observation :c 0.8 :v 2023-12-01)";
        {
            let mut store = open_fresh(&dir);
            store.commit_batch(old_sem, fixed_now()).expect("sem");
            store
                .commit_batch("(pin @__mem_0 :actor @mira)", fixed_now())
                .expect("pin");
        }
        let mut reopened = open_fresh(&dir);
        let got = reopened
            .pipeline_mut()
            .execute_query("(query :show_framing true)")
            .expect("reopened query");
        // Pin state must survive replay.
        assert_eq!(got.framings.len(), 1);
        assert_eq!(
            got.framings[0],
            Framing::Authoritative {
                set_by: FramingSource::AgentPinned
            }
        );
    }

    #[test]
    fn multiple_commits_accumulate_in_log() {
        let dir = TempDir::new().expect("tmp");
        let mut store = open_fresh(&dir);
        let _ = store.commit_batch(SEM_OK, fixed_now()).expect("first");
        let input2 = "(sem @alice @likes @carol :src @observation :c 0.7 :v 2024-01-16)";
        let _ = store.commit_batch(input2, fixed_now()).expect("second");

        let bytes = store.log.read_all().expect("read");
        let records = decode_all(&bytes).expect("decode");
        // Two checkpoints and two Sems, intermingled with SymbolAlloc
        // records at the start of each batch.
        let checkpoints = records
            .iter()
            .filter(|r| matches!(r, CanonicalRecord::Checkpoint(_)))
            .count();
        assert_eq!(checkpoints, 2);
        let sems = records
            .iter()
            .filter(|r| matches!(r, CanonicalRecord::Sem(_)))
            .count();
        assert_eq!(sems, 2);
    }

    #[test]
    fn pipeline_error_does_not_write_log() {
        let dir = TempDir::new().expect("tmp");
        let mut store = open_fresh(&dir);
        let err = store
            .commit_batch("(sem @a", fixed_now())
            .expect_err("malformed");
        assert!(matches!(err, StoreError::Pipeline(_)));
        assert_eq!(store.log.len(), 0);
    }

    #[test]
    fn commits_assign_distinct_episode_ids() {
        let dir = TempDir::new().expect("tmp");
        let mut store = open_fresh(&dir);
        let a = store.commit_batch(SEM_OK, fixed_now()).expect("a");
        let input2 = "(sem @alice @likes @carol :src @observation :c 0.7 :v 2024-01-16)";
        let b = store.commit_batch(input2, fixed_now()).expect("b");
        assert_ne!(a.as_symbol(), b.as_symbol());
    }

    #[test]
    fn reopen_truncates_orphans_past_last_checkpoint() {
        let dir = TempDir::new().expect("tmp");
        let path = dir.path().join("canonical.log");
        let committed_len;
        {
            let mut store = Store::open(&path).expect("open");
            let _ = store.commit_batch(SEM_OK, fixed_now()).expect("commit");
            committed_len = store.log.len();
        }
        // Simulate a crash mid-batch: append orphan bytes that are
        // neither a valid record nor terminated by a CHECKPOINT.
        {
            let mut raw = CanonicalLog::open(&path).expect("reopen raw");
            raw.append(&[0x01, 0x42, 0xFF, 0xFF]).expect("append");
            raw.sync().expect("sync");
            assert!(raw.len() > committed_len);
        }
        // Reopening the store must truncate the orphan bytes.
        let store = Store::open(&path).expect("reopen store");
        assert_eq!(store.log.len(), committed_len);
    }

    #[test]
    fn reopen_on_empty_workspace_is_clean() {
        let dir = TempDir::new().expect("tmp");
        let store = Store::open(dir.path().join("canonical.log")).expect("open");
        assert_eq!(store.log_len(), 0);
    }

    #[test]
    fn episode_allocation_collision_restores_pipeline_state() {
        // Covers the commit-path rollback branch where
        // `allocate_episode_symbol` fails after the pipeline already
        // auto-applied its compile mutations. We force a collision by
        // rewinding the episode counter to a value whose `__ep_{n}`
        // name is already in the table from a prior commit.
        let dir = TempDir::new().expect("tmp");
        let mut store = open_fresh(&dir);
        let _ = store.commit_batch(SEM_OK, fixed_now()).expect("first");
        assert_eq!(store.next_episode_counter, 1);
        let snapshot = store.pipeline.clone();
        let log_len_after_first = store.log.len();

        // Force the collision.
        store.next_episode_counter = 0;

        let input2 = "(sem @alice @likes @carol :src @observation :c 0.7 :v 2024-01-16)";
        let err = store
            .commit_batch(input2, fixed_now())
            .expect_err("collision");
        assert!(matches!(err, StoreError::Pipeline(_)));

        // Rollback verification: pipeline + counter + log all restored
        // to their pre-second-commit state. In particular the pipeline
        // must NOT contain the new @carol symbol that compile_batch
        // allocated before the episode-collision fired.
        assert_eq!(store.next_episode_counter, 0);
        assert_eq!(store.pipeline, snapshot);
        assert_eq!(store.log.len(), log_len_after_first);
    }

    #[test]
    fn reopen_restores_symbol_table_from_log() {
        let dir = TempDir::new().expect("tmp");
        let path = dir.path().join("canonical.log");
        let alice_id;
        {
            let mut store = Store::open(&path).expect("open");
            let _ = store.commit_batch(SEM_OK, fixed_now()).expect("commit");
            alice_id = store
                .pipeline
                .table()
                .lookup("alice")
                .expect("alice allocated");
        }
        // Reopen: replay must restore the table such that @alice is
        // still allocated with the SAME SymbolId.
        let store = Store::open(&path).expect("reopen");
        assert_eq!(store.pipeline.table().lookup("alice"), Some(alice_id));
        assert!(store.pipeline.table().lookup("knows").is_some());
        assert!(store.pipeline.table().lookup("bob").is_some());
    }

    #[test]
    fn reopen_restores_table_from_epi_batch() {
        let dir = TempDir::new().expect("tmp");
        let path = dir.path().join("canonical.log");
        let evt_id;
        let alice_id;
        {
            let mut store = Store::open(&path).expect("open");
            let input = "(epi @evt_001 @rename (@old @new) @github \
                         :at 2024-01-15T10:00:00Z :obs 2024-01-15T10:00:05Z \
                         :src @alice :c 0.9)";
            let _ = store.commit_batch(input, fixed_now()).expect("commit");
            evt_id = store
                .pipeline
                .table()
                .lookup("evt_001")
                .expect("event id allocated");
            alice_id = store
                .pipeline
                .table()
                .lookup("alice")
                .expect("witness allocated");
        }
        let store = Store::open(&path).expect("reopen");
        assert_eq!(store.pipeline.table().lookup("evt_001"), Some(evt_id));
        assert_eq!(store.pipeline.table().lookup("alice"), Some(alice_id));
        assert!(store.pipeline.table().lookup("old").is_some());
        assert!(store.pipeline.table().lookup("new").is_some());
        assert!(store.pipeline.table().lookup("github").is_some());
        assert_eq!(store.pipeline.episodic_records().len(), 1);
        assert_eq!(store.pipeline.episodic_records()[0].event_id, evt_id);
        assert_eq!(store.pipeline.episodic_records()[0].source, alice_id);
    }

    #[test]
    fn reopen_restores_table_from_pro_batch() {
        let dir = TempDir::new().expect("tmp");
        let path = dir.path().join("canonical.log");
        let rule_id;
        {
            let mut store = Store::open(&path).expect("open");
            let input = r#"(pro @rule_1 "trigger text" "action text" :scp @mimir :src @agent_instruction :c 0.9)"#;
            let _ = store.commit_batch(input, fixed_now()).expect("commit");
            rule_id = store
                .pipeline
                .table()
                .lookup("rule_1")
                .expect("rule allocated");
        }
        let store = Store::open(&path).expect("reopen");
        assert_eq!(store.pipeline.table().lookup("rule_1"), Some(rule_id));
        assert!(store.pipeline.table().lookup("mimir").is_some());
        assert!(store.pipeline.table().lookup("agent_instruction").is_some());
    }

    #[test]
    fn reopen_restores_table_from_inf_batch() {
        let dir = TempDir::new().expect("tmp");
        let path = dir.path().join("canonical.log");
        let method_id;
        {
            let mut store = Store::open(&path).expect("open");
            let input = "(inf @alice @friend_of @carol (@m0 @m1) @citation_link \
                         :c 0.6 :v 2024-01-15)";
            let _ = store.commit_batch(input, fixed_now()).expect("commit");
            method_id = store
                .pipeline
                .table()
                .lookup("citation_link")
                .expect("method allocated");
        }
        let store = Store::open(&path).expect("reopen");
        assert_eq!(
            store.pipeline.table().lookup("citation_link"),
            Some(method_id)
        );
        for name in ["alice", "friend_of", "carol", "m0", "m1"] {
            assert!(
                store.pipeline.table().lookup(name).is_some(),
                "{name} lost on reopen"
            );
        }
    }

    #[test]
    fn reopen_advances_memory_and_episode_counters() {
        let dir = TempDir::new().expect("tmp");
        let path = dir.path().join("canonical.log");
        {
            let mut store = Store::open(&path).expect("open");
            let _ = store.commit_batch(SEM_OK, fixed_now()).expect("commit");
        }
        let mut store = Store::open(&path).expect("reopen");
        assert_eq!(store.next_episode_counter, 1);
        // A follow-up commit must not collide on __mem_0 or __ep_0 —
        // replay advanced both counters past their pre-crash values.
        let input2 = "(sem @alice @likes @carol :src @observation :c 0.7 :v 2024-01-16)";
        let _ = store.commit_batch(input2, fixed_now()).expect("second");
    }

    #[test]
    fn checkpoint_and_episode_alloc_use_monotonic_clock_under_regressed_wall_clock() {
        // Store-side contract: the CHECKPOINT record and the
        // synthetic __ep_{n} SymbolAlloc both carry `effective_now`
        // (the pipeline's monotonic-enforced clock) in their `at`
        // field, not the raw wall clock passed into `commit_batch`.
        // Without this, a regressed wall clock would let the
        // checkpoint's `at` sit below the prior batch's committed_at,
        // violating the per-workspace monotonicity invariant.
        let dir = TempDir::new().expect("tmp");
        let path = dir.path().join("canonical.log");
        let high = ClockTime::try_from_millis(2_000_000_000_000).expect("non-sentinel");
        let regressed = ClockTime::try_from_millis(1_800_000_000_000).expect("non-sentinel");
        {
            let mut store = Store::open(&path).expect("open");
            let _ = store.commit_batch(SEM_OK, high).expect("high");
            // Distinct predicate so auto-supersession doesn't
            // interfere — this test is about clock monotonicity, not
            // (s, p) supersession detection.
            let _ = store
                .commit_batch(
                    "(sem @alice @likes @dan :src @observation :c 0.8 :v 2024-01-15)",
                    regressed,
                )
                .expect("regressed");
        }

        // Decode the log and pull the second batch's checkpoint and
        // __ep_1 alloc. Both must sit at `high + 1` — the monotonic
        // bump — not at `regressed` (which is < `high`).
        // Skip the 8-byte canonical-log header (magic + format version,
        // see `log::LOG_HEADER_SIZE`) before decoding the record stream.
        let raw = std::fs::read(&path).expect("read log");
        let header_size = usize::try_from(crate::log::LOG_HEADER_SIZE).expect("header fits");
        let bytes = &raw[header_size..];
        let records = decode_all(bytes).expect("decode");

        // Find the __ep_1 SymbolAlloc.
        let ep1_alloc = records
            .iter()
            .find(|r| matches!(r, CanonicalRecord::SymbolAlloc(ev) if ev.name == "__ep_1"))
            .expect("__ep_1 alloc present");
        let CanonicalRecord::SymbolAlloc(ep1) = ep1_alloc else {
            unreachable!();
        };
        let expected = ClockTime::try_from_millis(high.as_millis() + 1).expect("non-sentinel");
        assert_eq!(ep1.at, expected, "__ep_1 alloc must use monotonic clock");

        // There are two checkpoints — the second (last) corresponds
        // to the regressed batch.
        let checkpoints: Vec<_> = records
            .iter()
            .filter_map(|r| match r {
                CanonicalRecord::Checkpoint(c) => Some(c),
                _ => None,
            })
            .collect();
        assert_eq!(checkpoints.len(), 2, "two batches → two checkpoints");
        assert_eq!(
            checkpoints[1].at, expected,
            "second checkpoint.at must use monotonic clock, not regressed wall clock"
        );
    }

    #[test]
    fn reopen_restores_monotonic_commit_watermark() {
        // temporal-model.md § 9.2 / § 12 #1: committed_at must be
        // strictly monotonic per workspace even across reopen. On
        // open, the pipeline's watermark is restored from the
        // highest `committed_at` seen in the log — so a follow-up
        // batch submitted with a regressed wall clock is still
        // bumped past the last durably-committed record.
        let dir = TempDir::new().expect("tmp");
        let path = dir.path().join("canonical.log");
        let high = ClockTime::try_from_millis(2_000_000_000_000).expect("non-sentinel");
        {
            let mut store = Store::open(&path).expect("open");
            let _ = store.commit_batch(SEM_OK, high).expect("commit at high");
            assert_eq!(store.pipeline.last_committed_at(), Some(high));
        }
        // Reopen and check the watermark survived.
        let mut store = Store::open(&path).expect("reopen");
        assert_eq!(store.pipeline.last_committed_at(), Some(high));

        // Commit with a regressed wall clock; pipeline must bump
        // past `high`. `low` sits after 2024-01-15 so semantic does
        // not reject the form for future-validity.
        let low = ClockTime::try_from_millis(1_800_000_000_000).expect("non-sentinel");
        let _ = store
            .commit_batch(
                "(sem @alice @likes @dan :src @observation :c 0.8 :v 2024-01-15)",
                low,
            )
            .expect("regressed commit");
        let watermark = store
            .pipeline
            .last_committed_at()
            .expect("watermark set after commit");
        assert_eq!(watermark.as_millis(), high.as_millis() + 1);
    }

    #[test]
    fn reopen_replays_rename_and_retire() {
        let dir = TempDir::new().expect("tmp");
        let path = dir.path().join("canonical.log");
        {
            let mut store = Store::open(&path).expect("open");
            let _ = store.commit_batch(SEM_OK, fixed_now()).expect("first");
            let _ = store
                .commit_batch("(rename @alice @alice_v2)", fixed_now())
                .expect("rename");
            let _ = store
                .commit_batch("(retire @bob)", fixed_now())
                .expect("retire");
        }
        let store = Store::open(&path).expect("reopen");
        let alice_id = store
            .pipeline
            .table()
            .lookup("alice_v2")
            .expect("canonical rotated");
        assert_eq!(
            store
                .pipeline
                .table()
                .entry(alice_id)
                .expect("entry")
                .canonical_name,
            "alice_v2"
        );
        let bob_id = store.pipeline.table().lookup("bob").expect("bob");
        assert!(store.pipeline.table().is_retired(bob_id));
    }

    // ----------------------------------------------------------
    // Crash-injection matrix per write-protocol.md § 7.
    // ----------------------------------------------------------

    #[test]
    fn row_3_orphan_memory_record_without_checkpoint_truncated_on_reopen() {
        // Spec § 7 row: "Crash between last record and CHECKPOINT
        // append". Simulate by committing one batch (durable), then
        // appending an orphan memory record whose batch never reached
        // CHECKPOINT. Reopen must truncate to the last CHECKPOINT.
        let dir = TempDir::new().expect("tmp");
        let path = dir.path().join("canonical.log");
        let committed_len;
        {
            let mut store = Store::open(&path).expect("open");
            let _ = store.commit_batch(SEM_OK, fixed_now()).expect("commit");
            committed_len = store.log_len();
        }
        // Append a valid but uncommitted SymbolAlloc — represents an
        // in-progress batch that crashed before Phase 2.
        {
            let mut raw = CanonicalLog::open(&path).expect("raw");
            let fake_alloc = CanonicalRecord::SymbolAlloc(SymbolEventRecord {
                symbol_id: SymbolId::new(999),
                name: "orphan_symbol".into(),
                symbol_kind: SymbolKind::Literal,
                at: fixed_now(),
            });
            let mut buf = Vec::new();
            encode_record(&fake_alloc, &mut buf);
            raw.append(&buf).expect("append");
            raw.sync().expect("sync");
            assert!(raw.len() > committed_len);
        }
        let store = Store::open(&path).expect("reopen");
        assert_eq!(store.log_len(), committed_len);
        // The orphaned symbol must NOT appear in the reconstructed table.
        assert!(store.pipeline.table().lookup("orphan_symbol").is_none());
    }

    #[test]
    fn row_6_append_failure_rolls_back_pipeline_and_log() {
        // Spec § 7 row: "Disk full during Phase 1". Inject a
        // StorageFull on the next append and assert full rollback.
        let mut store = Store::from_backend(FaultyLog::new()).expect("open");
        let pre_commit_pipeline = store.pipeline.clone();
        store
            .log
            .arm_append_failure(std::io::ErrorKind::StorageFull);

        let err = store
            .commit_batch(SEM_OK, fixed_now())
            .expect_err("append failure");
        assert!(matches!(err, StoreError::Log(_)));
        // Log, pipeline, and episode counter are all restored.
        assert_eq!(store.log.len(), 0);
        assert_eq!(store.pipeline, pre_commit_pipeline);
        assert_eq!(store.next_episode_counter, 0);
    }

    #[test]
    fn row_7_sync_failure_rolls_back_pipeline_and_log() {
        // Spec § 7 row: "fsync fails (hardware error)". Inject an IO
        // error on the next sync and assert full rollback. The log
        // bytes were appended but must be truncated back.
        let mut store = Store::from_backend(FaultyLog::new()).expect("open");
        let pre_commit_pipeline = store.pipeline.clone();
        store.log.arm_sync_failure(std::io::ErrorKind::Other);

        let err = store
            .commit_batch(SEM_OK, fixed_now())
            .expect_err("sync failure");
        assert!(matches!(err, StoreError::Log(_)));
        // The appended bytes have been truncated back.
        assert_eq!(store.log.len(), 0);
        assert_eq!(store.pipeline, pre_commit_pipeline);
        assert_eq!(store.next_episode_counter, 0);
    }

    #[test]
    fn rollback_truncate_failure_still_surfaces_an_error() {
        // Compound-failure path: sync fails, triggering rollback;
        // rollback's truncate also fails. The store must surface
        // *an* error (currently the secondary truncate error, with
        // the primary sync error lost — documented diagnosability
        // limitation noted in the `rollback` helper's "best-effort"
        // comment). This test proves the path is reachable and the
        // caller is not silently given Ok.
        let mut store = Store::from_backend(FaultyLog::new()).expect("open");
        let _ = store.commit_batch(SEM_OK, fixed_now()).expect("first");
        let pre_second_pipeline = store.pipeline.clone();
        let len_after_first = store.log.len();

        store.log.arm_sync_failure(std::io::ErrorKind::Other);
        store
            .log
            .arm_truncate_failure(std::io::ErrorKind::PermissionDenied);

        let err = store
            .commit_batch(SEM_OK_2, fixed_now())
            .expect_err("compound failure");
        assert!(matches!(err, StoreError::Log(_)));
        // Rollback could not truncate the log, so the bytes appended
        // for the second batch remain beyond `len_after_first` — a
        // reopen's `last_checkpoint_end` scan will truncate them.
        assert!(store.log.len() >= len_after_first);
        // Pipeline and counter were restored *before* the truncate
        // attempt, so their snapshot semantics hold even when
        // truncate fails.
        assert_eq!(store.pipeline, pre_second_pipeline);
        assert_eq!(store.next_episode_counter, 1);
    }

    #[test]
    fn rollback_preserves_earlier_committed_bytes() {
        // Variant of rows 6/7: after one successful commit, a failure
        // on the second commit truncates back to the first commit's
        // length — not all the way to zero.
        let mut store = Store::from_backend(FaultyLog::new()).expect("open");
        let _ = store.commit_batch(SEM_OK, fixed_now()).expect("first");
        let len_after_first = store.log.len();
        assert!(len_after_first > 0);

        store.log.arm_sync_failure(std::io::ErrorKind::Other);
        let err = store
            .commit_batch(SEM_OK_2, fixed_now())
            .expect_err("sync failure");
        assert!(matches!(err, StoreError::Log(_)));
        assert_eq!(store.log.len(), len_after_first);
    }

    #[test]
    fn orphan_truncation_is_idempotent() {
        // Spec § 1 graduation criterion #4 + § 10.2: truncating an
        // already-orphan-free log to the same committed offset is a
        // no-op. Running recovery multiple times must converge.
        let dir = TempDir::new().expect("tmp");
        let path = dir.path().join("canonical.log");
        {
            let mut store = Store::open(&path).expect("open");
            let _ = store.commit_batch(SEM_OK, fixed_now()).expect("first");
            let _ = store.commit_batch(SEM_OK_2, fixed_now()).expect("second");
        }
        // Inject a crash-shaped partial record tail.
        {
            let mut raw = CanonicalLog::open(&path).expect("raw");
            raw.append(&[0x01_u8]).expect("append partial frame");
            raw.sync().expect("sync");
        }
        // First recovery truncates.
        let len_after_first_recovery = {
            let store = Store::open(&path).expect("recover once");
            store.log_len()
        };
        // Second recovery is a no-op — same length.
        let store = Store::open(&path).expect("recover twice");
        assert_eq!(store.log_len(), len_after_first_recovery);
    }

    #[test]
    fn reopen_rejects_corrupt_tail_after_last_checkpoint() {
        let dir = TempDir::new().expect("tmp");
        let path = dir.path().join("canonical.log");
        let committed_len;
        {
            let mut store = Store::open(&path).expect("open");
            let _ = store.commit_batch(SEM_OK, fixed_now()).expect("commit");
            committed_len = store.log_len();
        }

        {
            let mut raw = CanonicalLog::open(&path).expect("raw");
            raw.append(&[0x05_u8; 7]).expect("append corrupt tail");
            raw.sync().expect("sync");
            assert!(raw.len() > committed_len);
        }

        let Err(err) = Store::open(&path) else {
            panic!("corrupt tail must not truncate");
        };
        assert!(
            matches!(err, StoreError::CorruptTail { .. }),
            "expected corrupt-tail error, got {err:?}"
        );

        let raw = CanonicalLog::open(&path).expect("raw reopen");
        assert!(
            raw.len() > committed_len,
            "corrupt tail must be preserved for inspection"
        );
    }

    #[test]
    fn reopen_rejects_corrupt_log_without_checkpoint() {
        let dir = TempDir::new().expect("tmp");
        let path = dir.path().join("canonical.log");
        {
            let mut raw = CanonicalLog::open(&path).expect("raw");
            raw.append(&[0x05_u8]).expect("append corrupt log");
            raw.sync().expect("sync");
        }

        let Err(err) = Store::open(&path) else {
            panic!("corrupt checkpoint-free log must not truncate to empty");
        };
        assert!(
            matches!(
                err,
                StoreError::CorruptTail {
                    offset: 0,
                    source: DecodeError::UnknownOpcode { .. }
                }
            ),
            "expected corrupt-tail unknown-opcode error, got {err:?}"
        );

        let raw = CanonicalLog::open(&path).expect("raw reopen");
        assert_eq!(raw.len(), 1, "corrupt bytes must be preserved");
    }

    #[test]
    fn symbol_table_replay_reproduces_pre_crash_state() {
        // Spec § 1 graduation criterion #4: symbol-table replay
        // reproduces the exact pre-crash state. Commit a diverse
        // batch (alloc + rename + retire), capture every state field
        // — `SymbolTable`, `next_episode_counter`, and
        // `next_memory_counter` — then close and reopen. The replayed
        // triple must be byte-equal to the pre-close triple; asserting
        // all three ensures "exact state" isn't tested via a proxy.
        let dir = TempDir::new().expect("tmp");
        let path = dir.path().join("canonical.log");
        let table_before;
        let counter_before;
        let memory_counter_before;
        {
            let mut store = Store::open(&path).expect("open");
            let _ = store.commit_batch(SEM_OK, fixed_now()).expect("commit");
            let _ = store
                .commit_batch("(rename @alice @alice_v2)", fixed_now())
                .expect("rename");
            let _ = store
                .commit_batch("(retire @bob)", fixed_now())
                .expect("retire");
            table_before = store.pipeline.table().clone();
            counter_before = store.next_episode_counter;
            memory_counter_before = store.pipeline.next_memory_counter();
        }
        let store = Store::open(&path).expect("reopen");
        assert_eq!(store.pipeline.table(), &table_before);
        assert_eq!(store.next_episode_counter, counter_before);
        // `next_memory_counter` must advance past every `__mem_{n}`
        // seen in the log so a follow-up commit doesn't collide with
        // a pre-crash memory-id allocation.
        assert_eq!(store.pipeline.next_memory_counter(), memory_counter_before);
    }

    #[test]
    fn checkpoint_is_atomic_commit_boundary() {
        // Spec § 1 graduation criterion #4 + § 12 invariant 1:
        // truncating the log to just before a Checkpoint makes the
        // batch uncommitted. After truncation to any Checkpoint
        // boundary, Store::open must treat the post-checkpoint bytes
        // as orphans.
        let dir = TempDir::new().expect("tmp");
        let path = dir.path().join("canonical.log");
        let len_after_first;
        {
            let mut store = Store::open(&path).expect("open");
            let _ = store.commit_batch(SEM_OK, fixed_now()).expect("first");
            len_after_first = store.log_len();
            let _ = store.commit_batch(SEM_OK_2, fixed_now()).expect("second");
        }
        // Truncate to the first Checkpoint's end — simulates the
        // second batch's Checkpoint never having been durable.
        {
            let mut raw = CanonicalLog::open(&path).expect("raw");
            raw.truncate(len_after_first).expect("truncate");
        }
        let store = Store::open(&path).expect("reopen");
        // The second batch is fully discarded: @carol not present.
        assert!(store.pipeline.table().lookup("carol").is_none());
        // But the first batch's state is intact: @alice still there.
        assert!(store.pipeline.table().lookup("alice").is_some());
    }

    // ----- workspace partitioning -----

    #[test]
    fn open_in_workspace_creates_partitioned_directory() {
        // `workspace-model.md` § 4.2: different workspaces under the
        // same data_root land in different on-disk directories and
        // share no state.
        use crate::WorkspaceId;
        let data_root = TempDir::new().expect("tmp");
        let ws_a = WorkspaceId::from_git_remote("https://github.com/foo/mimir").unwrap();
        let ws_b = WorkspaceId::from_git_remote("https://github.com/bar/mimir").unwrap();
        assert_ne!(ws_a, ws_b);

        {
            let mut store_a = Store::open_in_workspace(data_root.path(), ws_a).expect("open ws a");
            let _ = store_a.commit_batch(SEM_OK, fixed_now()).expect("commit a");
        }
        {
            let mut store_b = Store::open_in_workspace(data_root.path(), ws_b).expect("open ws b");
            // Workspace B's Store has no knowledge of workspace A's
            // commit — the table is fresh.
            assert!(store_b.pipeline.table().lookup("alice").is_none());
            let _ = store_b.commit_batch(SEM_OK, fixed_now()).expect("commit b");
        }
        // Reopen workspace A; its state is intact and independent of B.
        let store_a_again = Store::open_in_workspace(data_root.path(), ws_a).expect("reopen ws a");
        assert!(store_a_again.pipeline.table().lookup("alice").is_some());
    }

    #[test]
    fn reopen_restores_procedural_supersession_index() {
        // 6.3b contract: the Procedural index (rule_id +
        // (trigger, scope)) is rebuilt from the log at open, so a
        // post-reopen Pro write with the same rule_id or same
        // (trigger, scope) correctly auto-supersedes the pre-reopen
        // memory.
        let dir = TempDir::new().expect("tmp");
        let path = dir.path().join("canonical.log");
        let pro_seed = r#"(pro @rule_route "agent_write" "route_via_librarian"
            :scp @mimir :src @policy :c 1.0)"#;
        {
            let mut store = Store::open(&path).expect("open");
            let _ = store.commit_batch(pro_seed, fixed_now()).expect("seed");
        }
        let mut store = Store::open(&path).expect("reopen");
        // Post-reopen write with the same rule_id — must auto-supersede.
        let records = store
            .pipeline
            .compile_batch(
                r#"(pro @rule_route "other_trigger" "other_action"
                :scp @other_scope :src @policy :c 0.9)"#,
                fixed_now(),
            )
            .expect("post-reopen compile");
        let edges: Vec<_> = records
            .iter()
            .filter(|r| matches!(r, CanonicalRecord::Supersedes(_)))
            .collect();
        assert_eq!(
            edges.len(),
            1,
            "post-reopen same-rule_id write must auto-supersede"
        );
    }

    #[test]
    fn reopen_restores_supersession_index_so_post_reopen_auto_supersedes() {
        // 6.3a contract: the supersession-detection index is rebuilt
        // from the log at open, so a post-reopen batch at the same
        // (s, p) with a later valid_at correctly auto-supersedes the
        // pre-reopen memory. Without index replay, the new batch would
        // see an empty index and emit no Supersedes edge.
        let dir = TempDir::new().expect("tmp");
        let path = dir.path().join("canonical.log");
        {
            let mut store = Store::open(&path).expect("open");
            let _ = store.commit_batch(SEM_OK, fixed_now()).expect("seed");
        }
        // Reopen and commit a later-valid_at write at the same (s, p).
        let mut store = Store::open(&path).expect("reopen");
        let records = store
            .pipeline
            .compile_batch(
                "(sem @alice @knows @mallory :src @observation :c 0.8 :v 2024-03-01)",
                fixed_now(),
            )
            .expect("post-reopen compile");
        let edges: Vec<_> = records
            .iter()
            .filter(|r| matches!(r, CanonicalRecord::Supersedes(_)))
            .collect();
        assert_eq!(
            edges.len(),
            1,
            "post-reopen forward write must auto-supersede the pre-reopen memory"
        );
    }

    #[test]
    fn reopen_on_fully_committed_log_preserves_length() {
        let dir = TempDir::new().expect("tmp");
        let path = dir.path().join("canonical.log");
        let committed_len;
        {
            let mut store = Store::open(&path).expect("open");
            let _ = store.commit_batch(SEM_OK, fixed_now()).expect("commit");
            committed_len = store.log.len();
        }
        let store = Store::open(&path).expect("reopen");
        assert_eq!(store.log.len(), committed_len);
    }

    /// Append raw bytes bypassing the normal commit path, then close
    /// with a `CHECKPOINT` so recovery treats the run as committed.
    fn fabricate_committed_segment<L: LogBackend>(log: &mut L, records: &[CanonicalRecord]) {
        let mut buf = Vec::new();
        for r in records {
            encode_record(r, &mut buf);
        }
        log.append(&buf).expect("append");
        log.sync().expect("sync");
    }

    #[test]
    fn reopen_replays_supersession_edges_into_dag() {
        // 6.2's replay contract: edge records (`SUPERSEDES` /
        // `CORRECTS` / `STALE_PARENT` / `RECONFIRMS`) appearing before
        // the last durable `CHECKPOINT` are replayed into
        // `Pipeline::dag` with full acyclicity enforcement.
        use crate::canonical::{CheckpointRecord, EdgeRecord};
        use crate::dag::EdgeKind;

        let mut log = FaultyLog::new();
        let ep0 = SymbolId::new(100);
        let m1 = SymbolId::new(101);
        let m2 = SymbolId::new(102);
        let m3 = SymbolId::new(103);
        let ts = fixed_now();

        let records = vec![
            // Synthetic episode-alloc + three memory IDs as Memory-kind symbols
            // so replay has the referenced IDs in the symbol table (not
            // required by the DAG but realistic).
            CanonicalRecord::SymbolAlloc(SymbolEventRecord {
                symbol_id: ep0,
                name: "__ep_0".into(),
                symbol_kind: SymbolKind::Memory,
                at: ts,
            }),
            CanonicalRecord::SymbolAlloc(SymbolEventRecord {
                symbol_id: m1,
                name: "__mem_0".into(),
                symbol_kind: SymbolKind::Memory,
                at: ts,
            }),
            CanonicalRecord::SymbolAlloc(SymbolEventRecord {
                symbol_id: m2,
                name: "__mem_1".into(),
                symbol_kind: SymbolKind::Memory,
                at: ts,
            }),
            CanonicalRecord::SymbolAlloc(SymbolEventRecord {
                symbol_id: m3,
                name: "__mem_2".into(),
                symbol_kind: SymbolKind::Memory,
                at: ts,
            }),
            // Two edges, acyclic: m1 -> m2, m2 -> m3.
            CanonicalRecord::Supersedes(EdgeRecord {
                from: m1,
                to: m2,
                at: ts,
            }),
            CanonicalRecord::Corrects(EdgeRecord {
                from: m2,
                to: m3,
                at: ts,
            }),
            CanonicalRecord::Checkpoint(CheckpointRecord {
                episode_id: ep0,
                at: ts,
                memory_count: 0,
            }),
        ];
        fabricate_committed_segment(&mut log, &records);

        let store = Store::from_backend(log).expect("open");
        assert_eq!(store.pipeline.dag().len(), 2);
        let edges: Vec<_> = store.pipeline.dag().edges().to_vec();
        assert_eq!(edges[0].kind, EdgeKind::Supersedes);
        assert_eq!(edges[0].from, m1);
        assert_eq!(edges[0].to, m2);
        assert_eq!(edges[1].kind, EdgeKind::Corrects);
    }

    #[test]
    fn reopen_surfaces_dag_replay_error_on_cyclic_edges() {
        // A log whose edges close a cycle must fail open with
        // `StoreError::DagReplay`, not a silent invariant break.
        use crate::canonical::{CheckpointRecord, EdgeRecord};

        let mut log = FaultyLog::new();
        let ep0 = SymbolId::new(200);
        let m1 = SymbolId::new(201);
        let m2 = SymbolId::new(202);
        let ts = fixed_now();

        let records = vec![
            CanonicalRecord::SymbolAlloc(SymbolEventRecord {
                symbol_id: ep0,
                name: "__ep_0".into(),
                symbol_kind: SymbolKind::Memory,
                at: ts,
            }),
            // Cycle: m1 -> m2, m2 -> m1.
            CanonicalRecord::Supersedes(EdgeRecord {
                from: m1,
                to: m2,
                at: ts,
            }),
            CanonicalRecord::Supersedes(EdgeRecord {
                from: m2,
                to: m1,
                at: ts,
            }),
            CanonicalRecord::Checkpoint(CheckpointRecord {
                episode_id: ep0,
                at: ts,
                memory_count: 0,
            }),
        ];
        fabricate_committed_segment(&mut log, &records);

        let Err(err) = Store::from_backend(log) else {
            panic!("cyclic edges must not replay cleanly");
        };
        assert!(
            matches!(err, StoreError::DagReplay { .. }),
            "expected DagReplay, got {err:?}"
        );
    }
}
