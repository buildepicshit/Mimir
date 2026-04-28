//! Read-only inspection library backing the `mimir-cli` binary.
//!
//! Implements the rendering + verification surfaces of
//! `docs/concepts/decoder-tool-contract.md`. Everything in this crate
//! is read-only — no function here writes to a workspace or appends
//! to a canonical log.
//!
//! Public surface (v1):
//!
//! - [`LispRenderer`] — `CanonicalRecord` → Lisp S-expression text
//!   reconstructing the agent-visible fields of Sem / Epi / Pro / Inf
//!   memory records. Backs the `decode` subcommand.
//! - [`verify`] — integrity check on a `canonical.log` file covering
//!   the framing, opcode, and symbol-reference corruption classes
//!   from `decoder-tool-contract.md` § 6.
//! - [`iso8601_from_millis`] — ms-since-epoch → `YYYY-MM-DDTHH:MM:SSZ`
//!   string inverse of `mimir_core::parse`'s ISO-8601 loader, so
//!   timestamps round-trip bit-perfect through render → re-parse.

use std::path::Path;

use thiserror::Error;

use mimir_core::bind::SymbolTable;
use mimir_core::canonical::{
    decode_all, CanonicalRecord, DecodeError, EpiRecord, InfRecord, ProRecord, SemRecord,
};
use mimir_core::clock::ClockTime;
use mimir_core::confidence::Confidence;
use mimir_core::log::{CanonicalLog, LogBackend, LogError};
use mimir_core::pipeline::Pipeline;
use mimir_core::symbol::SymbolId;
use mimir_core::value::Value;

// -------------------------------------------------------------------
// Render errors
// -------------------------------------------------------------------

/// Errors produced by [`LispRenderer::render_memory`]. None of these
/// are fatal for the caller — the renderer can skip unrenderable
/// records and continue.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RenderError {
    /// A [`SymbolId`] referenced by a record is not allocated in the
    /// supplied symbol table. For a well-formed committed log this
    /// cannot happen (log replay populates the table); in a
    /// corrupted log this surfaces the dangling reference.
    #[error("unknown symbol id {id:?} in {context}")]
    UnknownSymbol {
        /// The offending ID.
        id: SymbolId,
        /// Human-readable slot name (for diagnostics).
        context: &'static str,
    },

    /// The record's opcode is not a memory record (Sem / Epi / Pro /
    /// Inf) — these shapes do not have a write-surface Lisp form.
    /// The caller should `continue` past the record.
    #[error("record is not a write-surface memory")]
    NotAMemory,
}

// -------------------------------------------------------------------
// Renderer
// -------------------------------------------------------------------

/// Renders memory records as Lisp S-expressions using a `SymbolTable`
/// to resolve `SymbolId` → canonical name. Output conforms to
/// `ir-write-surface.md` and re-parses to an equivalent
/// `UnboundForm` (modulo librarian-assigned `memory_id` /
/// `committed_at` / `observed_at` fields which the agent never
/// provides).
pub struct LispRenderer<'a> {
    table: &'a SymbolTable,
}

impl<'a> LispRenderer<'a> {
    /// Construct a renderer bound to a symbol table. The table must
    /// resolve every `SymbolId` the renderer is asked to handle — use
    /// `mimir_core::Store::open` to populate it from a canonical log.
    #[must_use]
    pub fn new(table: &'a SymbolTable) -> Self {
        Self { table }
    }

    /// Render a memory record as a Lisp S-expression. Non-memory
    /// records (`Checkpoint`, `SymbolAlloc`, edge records) return
    /// [`RenderError::NotAMemory`] so the caller can skip them.
    ///
    /// # Errors
    ///
    /// - [`RenderError::NotAMemory`] for non-memory opcodes.
    /// - [`RenderError::UnknownSymbol`] for symbol IDs not in the table.
    pub fn render_memory(&self, record: &CanonicalRecord) -> Result<String, RenderError> {
        match record {
            CanonicalRecord::Sem(r) => self.render_sem(r),
            CanonicalRecord::Epi(r) => self.render_epi(r),
            CanonicalRecord::Pro(r) => self.render_pro(r),
            CanonicalRecord::Inf(r) => self.render_inf(r),
            _ => Err(RenderError::NotAMemory),
        }
    }

    fn render_sem(&self, r: &SemRecord) -> Result<String, RenderError> {
        Ok(format!(
            "(sem @{subject} @{predicate} {object} :src @{source} :c {confidence} :v {valid_at})",
            subject = self.name_of(r.s, "sem.s")?,
            predicate = self.name_of(r.p, "sem.p")?,
            object = self.render_value(&r.o, "sem.o")?,
            source = self.name_of(r.source, "sem.source")?,
            confidence = render_confidence(r.confidence),
            valid_at = iso8601_from_millis(r.clocks.valid_at),
        ))
    }

    fn render_epi(&self, r: &EpiRecord) -> Result<String, RenderError> {
        let mut participants = String::from("(");
        for (i, p) in r.participants.iter().enumerate() {
            if i > 0 {
                participants.push(' ');
            }
            participants.push('@');
            participants.push_str(&self.name_of(*p, "epi.participant")?);
        }
        participants.push(')');
        Ok(format!(
            "(epi @{event_id} @{kind} {participants} @{location} :at {at_time} :obs {observed_at} :src @{source} :c {confidence})",
            event_id = self.name_of(r.event_id, "epi.event_id")?,
            kind = self.name_of(r.kind, "epi.kind")?,
            location = self.name_of(r.location, "epi.location")?,
            at_time = iso8601_from_millis(r.at_time),
            observed_at = iso8601_from_millis(r.observed_at),
            source = self.name_of(r.source, "epi.source")?,
            confidence = render_confidence(r.confidence),
        ))
    }

    fn render_pro(&self, r: &ProRecord) -> Result<String, RenderError> {
        let mut out = format!(
            "(pro @{rule_id} {trigger} {action}",
            rule_id = self.name_of(r.rule_id, "pro.rule_id")?,
            trigger = self.render_value(&r.trigger, "pro.trigger")?,
            action = self.render_value(&r.action, "pro.action")?,
        );
        if let Some(pre) = &r.precondition {
            out.push_str(" :pre ");
            out.push_str(&self.render_value(pre, "pro.precondition")?);
        }
        out.push_str(" :scp @");
        out.push_str(&self.name_of(r.scope, "pro.scope")?);
        out.push_str(" :src @");
        out.push_str(&self.name_of(r.source, "pro.source")?);
        out.push_str(" :c ");
        out.push_str(&render_confidence(r.confidence));
        out.push(')');
        Ok(out)
    }

    fn render_inf(&self, r: &InfRecord) -> Result<String, RenderError> {
        let mut parents = String::from("(");
        for (i, p) in r.derived_from.iter().enumerate() {
            if i > 0 {
                parents.push(' ');
            }
            parents.push('@');
            parents.push_str(&self.name_of(*p, "inf.derived_from")?);
        }
        parents.push(')');
        Ok(format!(
            "(inf @{subject} @{predicate} {object} {parents} @{method} :c {confidence} :v {valid_at})",
            subject = self.name_of(r.s, "inf.s")?,
            predicate = self.name_of(r.p, "inf.p")?,
            object = self.render_value(&r.o, "inf.o")?,
            method = self.name_of(r.method, "inf.method")?,
            confidence = render_confidence(r.confidence),
            valid_at = iso8601_from_millis(r.clocks.valid_at),
        ))
    }

    fn render_value(&self, value: &Value, context: &'static str) -> Result<String, RenderError> {
        Ok(match value {
            Value::Symbol(id) => format!("@{}", self.name_of(*id, context)?),
            Value::Integer(n) => n.to_string(),
            Value::Float(f) => render_float(*f),
            Value::Boolean(b) => if *b { "true" } else { "false" }.to_string(),
            Value::String(s) => render_string_literal(s),
            Value::Timestamp(t) => iso8601_from_millis(*t),
        })
    }

    fn name_of(&self, id: SymbolId, context: &'static str) -> Result<String, RenderError> {
        self.table
            .entry(id)
            .map(|e| e.canonical_name.clone())
            .ok_or(RenderError::UnknownSymbol { id, context })
    }
}

fn render_confidence(c: Confidence) -> String {
    // Five decimal digits exceed u16 resolution (65536 steps) so the
    // parser re-quantizes losslessly.
    format!("{:.5}", c.as_f32())
}

fn render_float(f: f64) -> String {
    if !f.is_finite() {
        // Parser rejects NaN / Inf; the renderer emits "nil" which
        // bind will complain about, surfacing the corruption rather
        // than silently losing it. For v1 corpora these should never
        // appear.
        return "nil".to_string();
    }
    // Use Rust's default Display, which round-trips f64 bit-for-bit.
    // Append ".0" for integer values so the parser sees a float
    // token, not an integer.
    let s = format!("{f}");
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

fn render_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// -------------------------------------------------------------------
// Timestamp rendering — inverse of mimir_core::parse's loader
// -------------------------------------------------------------------

/// ms-since-epoch → `YYYY-MM-DDTHH:MM:SSZ` UTC string. Matches the
/// ISO-8601 format accepted by `mimir_core::parse`'s timestamp
/// loader so the string round-trips bit-perfect.
///
/// Uses the Howard-Hinnant proleptic-Gregorian algorithm
/// (integer-only; no chrono dependency) for the date component.
#[must_use]
#[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
pub fn iso8601_from_millis(clock: ClockTime) -> String {
    // ClockTime is u64 ms-since-epoch; Mimir never represents pre-
    // epoch times (temporal-model.md § 9.1), so the i64 cast is safe
    // for the full representable range.
    let ms = clock.as_millis() as i64;
    let days = ms.div_euclid(86_400_000);
    let time_ms = ms.rem_euclid(86_400_000);
    let (year, month, day) = civil_from_days(days);
    let hour = time_ms / 3_600_000;
    let minute = (time_ms % 3_600_000) / 60_000;
    let second = (time_ms % 60_000) / 1_000;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Howard-Hinnant `civil_from_days`. Given days-since-Unix-epoch,
/// returns `(year, month, day)` in the proleptic Gregorian calendar.
/// Pure integer arithmetic; deterministic across architectures.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::similar_names
)]
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let year_raw = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { year_raw + 1 } else { year_raw };
    (year as i32, m as u32, d as u32)
}

// -------------------------------------------------------------------
// Verify
// -------------------------------------------------------------------

/// Classification of the trailing bytes past the last decodable
/// record, if any.
///
/// Classification is by **decoder behavior**, not by checkpoint-commit
/// boundary: `verify` walks the byte stream and inspects the error (if
/// any) that stopped the walk. A partially-written record whose header
/// fits but whose body is truncated is `OrphanTail`; any structural
/// violation (unknown opcode, reserved sentinel, body underflow after
/// a valid length) is `Corrupt`. Higher layers combine this with
/// checkpoint state to decide recovery strategy.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum TailStatus {
    /// The log decoded cleanly from start to end; no trailing bytes.
    Clean,
    /// Trailing bytes exist and the decoder stopped on a `Truncated`
    /// error — consistent with a crashed-mid-write tail that
    /// `write-protocol.md` § 10 expects to be truncated on next open.
    OrphanTail {
        /// Number of trailing bytes.
        bytes: u64,
    },
    /// Trailing bytes exist and the decoder stopped on a non-
    /// truncation error (unknown opcode, body underflow, reserved
    /// sentinel, etc.) — genuine corruption, not the recoverable
    /// append-was-interrupted pattern.
    Corrupt {
        /// Number of trailing bytes.
        bytes: u64,
        /// The `DecodeError` that stopped the walk.
        first_decode_error: mimir_core::canonical::DecodeError,
    },
}

impl TailStatus {
    /// `true` if the tail is clean (no trailing bytes).
    #[must_use]
    pub const fn is_clean(&self) -> bool {
        matches!(self, Self::Clean)
    }

    /// `true` if the tail indicates genuine corruption rather than a
    /// recoverable truncated write.
    #[must_use]
    pub const fn is_corrupt(&self) -> bool {
        matches!(self, Self::Corrupt { .. })
    }

    /// Number of trailing bytes (zero for `Clean`).
    #[must_use]
    pub const fn trailing_bytes(&self) -> u64 {
        match self {
            Self::Clean => 0,
            Self::OrphanTail { bytes } | Self::Corrupt { bytes, .. } => *bytes,
        }
    }
}

/// Result of a `verify` pass.
#[derive(Debug, PartialEq, Eq)]
pub struct VerifyReport {
    /// Number of records successfully decoded.
    pub records_decoded: usize,
    /// Number of `Checkpoint` boundaries found.
    pub checkpoints: usize,
    /// Number of memory records (Sem / Epi / Pro / Inf).
    pub memory_records: usize,
    /// Number of SYMBOL_* records.
    pub symbol_events: usize,
    /// Classification of the tail past the last decoded record —
    /// clean, recoverable orphan-tail truncation, or genuine
    /// corruption (with the underlying `DecodeError` preserved).
    pub tail: TailStatus,
    /// Dangling symbol references found in memory records (the
    /// referenced `SymbolId` had no preceding `SymbolAlloc`).
    pub dangling_symbols: usize,
}

impl VerifyReport {
    /// Convenience accessor — bytes past the last decoded record
    /// regardless of whether they're an orphan tail or corruption.
    #[must_use]
    pub const fn trailing_bytes(&self) -> u64 {
        self.tail.trailing_bytes()
    }
}

/// Errors produced by [`verify`] and [`load_table_from_log`].
#[derive(Debug, Error)]
pub enum VerifyError {
    /// Underlying I/O failure.
    #[error("verify I/O: {0}")]
    Log(#[from] LogError),

    /// Committed canonical bytes failed to decode. Distinct from the
    /// trailing-tail-truncation case, which `verify` reports via
    /// [`VerifyReport::trailing_bytes`]. This is genuine corruption in
    /// the durable region of the log.
    #[error("committed canonical bytes failed to decode: {source}")]
    CorruptCommittedLog {
        /// Underlying decoder error.
        #[from]
        source: mimir_core::canonical::DecodeError,
    },

    /// Replay of a `SYMBOL_*` record failed while rebuilding the
    /// table — duplicate allocations with distinct names, aliases
    /// pointing at unallocated symbols, etc.
    #[error("symbol-replay conflict during load: {source}")]
    SymbolReplay {
        /// Underlying bind error.
        #[from]
        source: mimir_core::bind::BindError,
    },

    /// `last_checkpoint_end` returned a byte offset that does not fit
    /// in `usize` on this target. Only possible on 32-bit hosts with
    /// multi-gigabyte logs, but we surface it rather than silently
    /// widening.
    #[error("committed log offset {offset} exceeds usize on this target")]
    CommittedEndOverflow {
        /// The u64 offset.
        offset: u64,
    },
}

/// Read-only integrity check on a canonical log.
///
/// Checks:
/// 1. Every record decodes cleanly until a trailing-bytes boundary.
/// 2. Every memory record's `SymbolId` references resolve against
///    the symbol table reconstructed by replaying the log's
///    `SYMBOL_*` events.
///
/// The report distinguishes "clean log + trailing orphans" (a
/// recoverable state per `write-protocol.md` § 10) from "dangling
/// symbol reference in committed data" (a corruption signal).
///
/// # Errors
///
/// - [`VerifyError::Log`] on filesystem read failure.
pub fn verify(log_path: &Path) -> Result<VerifyReport, VerifyError> {
    let mut log = CanonicalLog::open(log_path)?;
    let bytes = log.read_all()?;
    let total_len = bytes.len() as u64;

    // Walk records; stop on first decode error (matches Store recovery
    // semantics). Track the final decoded-offset for trailing_bytes.
    let mut pos: usize = 0;
    let mut records_decoded = 0_usize;
    let mut checkpoints = 0_usize;
    let mut memory_records = 0_usize;
    let mut symbol_events = 0_usize;

    // Reconstruct a SymbolTable as we walk so we can validate symbol
    // references in memory records.
    let mut table = SymbolTable::new();

    let mut first_stop_error: Option<mimir_core::canonical::DecodeError> = None;
    while pos < bytes.len() {
        let remaining = &bytes[pos..];
        match mimir_core::canonical::decode_record(remaining) {
            Ok((record, consumed)) => {
                pos += consumed;
                records_decoded += 1;
                apply_for_verify(
                    &record,
                    &mut table,
                    &mut checkpoints,
                    &mut memory_records,
                    &mut symbol_events,
                );
            }
            Err(e) => {
                first_stop_error = Some(e);
                break;
            }
        }
    }

    // Second pass: dangling-symbol detection. decode_all covers all
    // records we could read; we traverse memory records and check that
    // every SymbolId has an entry.
    let dangling_symbols = count_dangling_symbols(&bytes[..pos], &table);

    let trailing = total_len - pos as u64;
    // Classify the tail. `Truncated` and `LengthMismatch` correspond
    // to the "append was interrupted mid-record" pattern (`write-
    // protocol.md` § 10 — the writer crashed before fsync completed
    // and the final record's bytes are short). Every other
    // `DecodeError` variant indicates structurally-wrong bytes that
    // no healthy write path could have produced.
    let tail = match (first_stop_error, trailing) {
        (None, 0) => TailStatus::Clean,
        (None, bytes) => TailStatus::OrphanTail { bytes },
        (Some(DecodeError::Truncated { .. } | DecodeError::LengthMismatch { .. }), bytes) => {
            TailStatus::OrphanTail { bytes }
        }
        (Some(err), bytes) => TailStatus::Corrupt {
            bytes,
            first_decode_error: err,
        },
    };

    Ok(VerifyReport {
        records_decoded,
        checkpoints,
        memory_records,
        symbol_events,
        tail,
        dangling_symbols,
    })
}

fn apply_for_verify(
    record: &CanonicalRecord,
    table: &mut SymbolTable,
    checkpoints: &mut usize,
    memory_records: &mut usize,
    symbol_events: &mut usize,
) {
    match record {
        CanonicalRecord::SymbolAlloc(e) => {
            *symbol_events += 1;
            // Replay allocate; ignore conflicts (e.g. duplicate
            // allocations) — verify reports them via dangling /
            // trailing counters, not via mutation errors.
            let _ = table.replay_allocate(e.symbol_id, e.name.clone(), e.symbol_kind);
        }
        CanonicalRecord::SymbolAlias(e) => {
            *symbol_events += 1;
            let _ = table.replay_alias(e.symbol_id, e.name.clone());
        }
        CanonicalRecord::SymbolRename(e) => {
            *symbol_events += 1;
            let _ = table.replay_rename(e.symbol_id, e.name.clone());
        }
        CanonicalRecord::SymbolRetire(e) => {
            *symbol_events += 1;
            let _ = table.replay_retire(e.symbol_id, e.name.clone());
        }
        CanonicalRecord::Checkpoint(_) => {
            *checkpoints += 1;
        }
        CanonicalRecord::Sem(_)
        | CanonicalRecord::Epi(_)
        | CanonicalRecord::Pro(_)
        | CanonicalRecord::Inf(_) => {
            *memory_records += 1;
        }
        _ => {}
    }
}

fn count_dangling_symbols(bytes: &[u8], table: &SymbolTable) -> usize {
    let Ok(records) = decode_all(bytes) else {
        return 0;
    };
    let mut dangling = 0_usize;
    for record in records {
        match record {
            CanonicalRecord::Sem(r) => {
                for id in [r.s, r.p, r.source, r.memory_id] {
                    if table.entry(id).is_none() {
                        dangling += 1;
                    }
                }
                if let Value::Symbol(id) = r.o {
                    if table.entry(id).is_none() {
                        dangling += 1;
                    }
                }
            }
            CanonicalRecord::Epi(r) => {
                for id in [r.event_id, r.kind, r.location, r.source, r.memory_id] {
                    if table.entry(id).is_none() {
                        dangling += 1;
                    }
                }
                for p in &r.participants {
                    if table.entry(*p).is_none() {
                        dangling += 1;
                    }
                }
            }
            CanonicalRecord::Pro(r) => {
                for id in [r.rule_id, r.scope, r.source, r.memory_id] {
                    if table.entry(id).is_none() {
                        dangling += 1;
                    }
                }
            }
            CanonicalRecord::Inf(r) => {
                for id in [r.s, r.p, r.method, r.memory_id] {
                    if table.entry(id).is_none() {
                        dangling += 1;
                    }
                }
                for p in &r.derived_from {
                    if table.entry(*p).is_none() {
                        dangling += 1;
                    }
                }
            }
            _ => {}
        }
    }
    dangling
}

// -------------------------------------------------------------------
// Helpers for the binary — decode a log + return its reconstructed
// pipeline table. Thin wrapper around Store::open.
// -------------------------------------------------------------------

/// Open a canonical log read-only and return the reconstructed
/// [`SymbolTable`] (via a temporary [`Pipeline`])
/// so the renderer and verify helpers can share a table.
///
/// # Errors
///
/// - [`VerifyError::Log`] on filesystem read failure.
pub fn load_table_from_log(log_path: &Path) -> Result<SymbolTable, VerifyError> {
    let mut log = CanonicalLog::open(log_path)?;
    let bytes = log.read_all()?;
    let committed_end = log.last_checkpoint_end()?;
    // Only replay committed data to avoid decoding corrupted tails.
    let committed_end =
        usize::try_from(committed_end).map_err(|_| VerifyError::CommittedEndOverflow {
            offset: committed_end,
        })?;
    let records = decode_all(&bytes[..committed_end])?;
    let mut pipeline = Pipeline::new();
    for record in records {
        match record {
            CanonicalRecord::SymbolAlloc(e) => {
                pipeline.replay_allocate(e.symbol_id, e.name, e.symbol_kind)?;
            }
            CanonicalRecord::SymbolAlias(e) => {
                pipeline.replay_alias(e.symbol_id, e.name)?;
            }
            CanonicalRecord::SymbolRename(e) => {
                pipeline.replay_rename(e.symbol_id, e.name)?;
            }
            CanonicalRecord::SymbolRetire(e) => {
                pipeline.replay_retire(e.symbol_id, e.name)?;
            }
            _ => {}
        }
    }
    Ok(pipeline.table().clone())
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn iso8601_renders_unix_epoch_zero() {
        assert_eq!(
            iso8601_from_millis(ClockTime::try_from_millis(0).expect("non-sentinel")),
            "1970-01-01T00:00:00Z"
        );
    }

    #[test]
    fn iso8601_renders_y2k() {
        // 2000-01-01T00:00:00Z = 946,684,800,000 ms.
        assert_eq!(
            iso8601_from_millis(ClockTime::try_from_millis(946_684_800_000).expect("non-sentinel")),
            "2000-01-01T00:00:00Z"
        );
    }

    #[test]
    fn iso8601_renders_known_timestamp() {
        // 2024-01-15T00:00:00Z = 1_705_276_800_000 ms.
        assert_eq!(
            iso8601_from_millis(
                ClockTime::try_from_millis(1_705_276_800_000).expect("non-sentinel")
            ),
            "2024-01-15T00:00:00Z"
        );
    }

    #[test]
    fn render_float_adds_fractional_for_integers() {
        assert_eq!(render_float(3.0), "3.0");
        assert_eq!(render_float(0.0), "0.0");
    }

    #[test]
    fn render_float_preserves_fractional() {
        assert_eq!(render_float(0.5), "0.5");
    }

    #[test]
    fn render_string_literal_escapes_special_chars() {
        assert_eq!(render_string_literal("hi"), r#""hi""#);
        assert_eq!(render_string_literal("a\"b"), r#""a\"b""#);
        assert_eq!(render_string_literal("x\nn"), r#""x\nn""#);
    }

    #[test]
    fn render_confidence_gives_stable_decimal() {
        let c = Confidence::try_from_f32(0.8).unwrap();
        // 0.8 * 65535 ≈ 52428 → as f32 ≈ 0.79999...; five decimals.
        let s = render_confidence(c);
        assert!(s.starts_with("0.7999") || s.starts_with("0.8000"));
        assert_eq!(s.chars().filter(|c| *c == '.').count(), 1);
    }
}
