//! Canonical bytecode encoder and decoder.
//!
//! Implements the on-disk format specified in
//! `docs/concepts/ir-canonical-form.md`. Every record is framed
//! `[1 byte opcode][varint body length][body]`. Unknown opcodes are
//! errors, not silently skipped — this differs from the spec's
//! forward-compatibility contract, which is reserved for the extension
//! opcode `0xFF` (not implemented in this milestone).

use std::fmt;

use thiserror::Error;

use crate::clock::ClockTime;
use crate::confidence::Confidence;
use crate::symbol::{SymbolId, SymbolKind};
use crate::value::Value;

// -------------------------------------------------------------------
// Opcodes
// -------------------------------------------------------------------

/// Record opcode per `ir-canonical-form.md` § 4.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Opcode {
    /// Semantic memory record.
    Sem = 0x01,
    /// Episodic memory record.
    Epi = 0x02,
    /// Procedural memory record.
    Pro = 0x03,
    /// Inferential memory record.
    Inf = 0x04,
    /// Supersession edge.
    Supersedes = 0x10,
    /// Episodic correction edge.
    Corrects = 0x11,
    /// Inferential stale-parent edge.
    StaleParent = 0x12,
    /// Inferential reconfirmation edge.
    Reconfirms = 0x13,
    /// Episode boundary marker.
    Checkpoint = 0x20,
    /// Episode metadata record (label / parent / retracts). Written
    /// by the store immediately before the `Checkpoint` for any
    /// batch that contains an `(episode :start ...)` form. See
    /// `episode-semantics.md` § 4.2.
    EpisodeMeta = 0x21,
    /// New symbol allocation.
    SymbolAlloc = 0x30,
    /// Rename edge.
    SymbolRename = 0x31,
    /// Alias edge.
    SymbolAlias = 0x32,
    /// Retirement flag set.
    SymbolRetire = 0x33,
    /// Retirement flag cleared.
    SymbolUnretire = 0x34,
    /// Pin flag set (suspends decay).
    Pin = 0x35,
    /// Pin flag cleared.
    Unpin = 0x36,
    /// Operator-authoritative flag set.
    AuthoritativeSet = 0x37,
    /// Operator-authoritative flag cleared.
    AuthoritativeClear = 0x38,
}

impl Opcode {
    fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            0x01 => Self::Sem,
            0x02 => Self::Epi,
            0x03 => Self::Pro,
            0x04 => Self::Inf,
            0x10 => Self::Supersedes,
            0x11 => Self::Corrects,
            0x12 => Self::StaleParent,
            0x13 => Self::Reconfirms,
            0x20 => Self::Checkpoint,
            0x21 => Self::EpisodeMeta,
            0x30 => Self::SymbolAlloc,
            0x31 => Self::SymbolRename,
            0x32 => Self::SymbolAlias,
            0x33 => Self::SymbolRetire,
            0x34 => Self::SymbolUnretire,
            0x35 => Self::Pin,
            0x36 => Self::Unpin,
            0x37 => Self::AuthoritativeSet,
            0x38 => Self::AuthoritativeClear,
            _ => return None,
        })
    }
}

// -------------------------------------------------------------------
// Clocks and flags
// -------------------------------------------------------------------

/// Null sentinel for `invalid_at` — `u64::MAX` per `ir-canonical-form.md`
/// § 3.1. Encoded as 8-byte LE `0xFFFF_FFFF_FFFF_FFFF`; decoded back to
/// `None`.
const NONE_SENTINEL: u64 = u64::MAX;

/// Semantic-memory flags — `projected` only per
/// `ir-canonical-form.md` § 5.1. On the wire: a single byte with
/// bit 0 = `projected`, bit 1 reserved (must be 0).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct SemFlags {
    /// `true` if the memory's `valid_at` is a future projection.
    pub projected: bool,
}

impl SemFlags {
    fn to_u8(self) -> u8 {
        u8::from(self.projected)
    }

    fn try_from_u8(b: u8, offset: usize) -> Result<Self, DecodeError> {
        const ALLOWED_MASK: u8 = 0b0000_0001;
        if b & !ALLOWED_MASK != 0 {
            return Err(DecodeError::InvalidFlagBits {
                byte: b,
                allowed_mask: ALLOWED_MASK,
                offset,
            });
        }

        Ok(Self {
            projected: b & (1 << 0) != 0,
        })
    }
}

/// Inferential-memory flags — carries both `projected` and `stale`
/// (the latter is Inferential-only per `temporal-model.md` § 5.4).
/// On the wire: one byte with bit 0 = `projected`, bit 1 = `stale`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct InfFlags {
    /// `true` if the memory's `valid_at` is a future projection.
    pub projected: bool,
    /// Set when the Inferential was derived from an already-
    /// superseded parent at write time (spec § 5.4). Runtime
    /// staleness (any incoming `StaleParent` edge) is a read-time
    /// overlay, not this flag.
    pub stale: bool,
}

impl InfFlags {
    fn to_u8(self) -> u8 {
        let mut b = 0_u8;
        if self.projected {
            b |= 1 << 0;
        }
        if self.stale {
            b |= 1 << 1;
        }
        b
    }

    fn try_from_u8(b: u8, offset: usize) -> Result<Self, DecodeError> {
        const ALLOWED_MASK: u8 = 0b0000_0011;
        if b & !ALLOWED_MASK != 0 {
            return Err(DecodeError::InvalidFlagBits {
                byte: b,
                allowed_mask: ALLOWED_MASK,
                offset,
            });
        }

        Ok(Self {
            projected: b & (1 << 0) != 0,
            stale: b & (1 << 1) != 0,
        })
    }
}

/// Four clocks plus the projection/stale flags, shared across the four
/// memory record shapes. Episodic adds `at_time` separately.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Clocks {
    /// When the fact becomes true in the world.
    pub valid_at: ClockTime,
    /// When the librarian observed the memory. For non-Episodic
    /// memories this equals `committed_at`.
    pub observed_at: ClockTime,
    /// When the librarian durably committed the record.
    pub committed_at: ClockTime,
    /// When the fact stopped being true. `None` while current.
    pub invalid_at: Option<ClockTime>,
}

// -------------------------------------------------------------------
// Record variants
// -------------------------------------------------------------------

/// Semantic memory record.
#[derive(Clone, Debug, PartialEq)]
pub struct SemRecord {
    /// Memory ID.
    pub memory_id: SymbolId,
    /// Subject.
    pub s: SymbolId,
    /// Predicate.
    pub p: SymbolId,
    /// Object value.
    pub o: Value,
    /// Source.
    pub source: SymbolId,
    /// Stored confidence.
    pub confidence: Confidence,
    /// Four clocks.
    pub clocks: Clocks,
    /// Flags — projected only for Semantic.
    pub flags: SemFlags,
}

/// Episodic memory record. No flags — Episodic neither projects nor
/// stales (the `projected` bit is Semantic / Inferential only, and
/// `stale` is Inferential only). Dropped from the wire as part of
/// the schema bump.
#[derive(Clone, Debug, PartialEq)]
pub struct EpiRecord {
    /// Memory ID.
    pub memory_id: SymbolId,
    /// Event ID.
    pub event_id: SymbolId,
    /// Event-type symbol.
    pub kind: SymbolId,
    /// Participants.
    pub participants: Vec<SymbolId>,
    /// Location.
    pub location: SymbolId,
    /// Event time.
    pub at_time: ClockTime,
    /// Observation time.
    pub observed_at: ClockTime,
    /// Source.
    pub source: SymbolId,
    /// Stored confidence.
    pub confidence: Confidence,
    /// Librarian-assigned commit time.
    pub committed_at: ClockTime,
    /// Supersession time (sentinel when current — Episodic rarely
    /// sets this; the librarian may still record it for consistency).
    pub invalid_at: Option<ClockTime>,
}

/// Procedural memory record. No flags — Procedural doesn't project
/// or stale. Dropped from the wire.
#[derive(Clone, Debug, PartialEq)]
pub struct ProRecord {
    /// Memory ID.
    pub memory_id: SymbolId,
    /// Rule ID.
    pub rule_id: SymbolId,
    /// Trigger.
    pub trigger: Value,
    /// Action.
    pub action: Value,
    /// Optional precondition.
    pub precondition: Option<Value>,
    /// Scope.
    pub scope: SymbolId,
    /// Source.
    pub source: SymbolId,
    /// Stored confidence.
    pub confidence: Confidence,
    /// Four clocks.
    pub clocks: Clocks,
}

/// Inferential memory record.
#[derive(Clone, Debug, PartialEq)]
pub struct InfRecord {
    /// Memory ID.
    pub memory_id: SymbolId,
    /// Subject.
    pub s: SymbolId,
    /// Predicate.
    pub p: SymbolId,
    /// Object.
    pub o: Value,
    /// Parent memories.
    pub derived_from: Vec<SymbolId>,
    /// Inference method.
    pub method: SymbolId,
    /// Stored confidence.
    pub confidence: Confidence,
    /// Four clocks.
    pub clocks: Clocks,
    /// Flags — projection + stale. Inferential is the only record
    /// that carries both.
    pub flags: InfFlags,
}

/// Supersession-family edge record. Shared shape for `SUPERSEDES` /
/// `CORRECTS` / `STALE_PARENT` / `RECONFIRMS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EdgeRecord {
    /// Source memory ID.
    pub from: SymbolId,
    /// Target memory ID.
    pub to: SymbolId,
    /// Timestamp the edge was applied.
    pub at: ClockTime,
}

/// Episode boundary marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckpointRecord {
    /// Episode ID.
    pub episode_id: SymbolId,
    /// Commit time of the episode.
    pub at: ClockTime,
    /// Number of memory records that are members of this episode.
    pub memory_count: u64,
}

/// Episode metadata record — extends the mechanical `Checkpoint` with
/// the agent-visible fields from `episode-semantics.md` § 4.2 that
/// the bare `Checkpoint` doesn't carry: optional label, optional
/// parent Episode link, and a (possibly empty) list of retracted
/// Episodes.
///
/// Emitted by the store immediately before the batch's
/// `Checkpoint`. Batches that don't carry an `(episode :start ...)`
/// form have no `EpisodeMeta`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpisodeMetaRecord {
    /// Episode ID this metadata describes (same as the following
    /// `Checkpoint`'s `episode_id`).
    pub episode_id: SymbolId,
    /// Commit time — same as the `Checkpoint`'s `at`.
    pub at: ClockTime,
    /// Optional human-readable label. Capped at 256 bytes per
    /// `episode-semantics.md` § 4.3.
    pub label: Option<String>,
    /// Optional parent Episode.
    pub parent_episode_id: Option<SymbolId>,
    /// Episodes this Episode retracts.
    pub retracts: Vec<SymbolId>,
}

/// Symbol-table event record. Shared shape across `SYMBOL_*` opcodes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolEventRecord {
    /// Symbol ID being affected.
    pub symbol_id: SymbolId,
    /// Canonical or alias name attached by this event (may be empty
    /// for retire/unretire where the spec says the field is ignored).
    pub name: String,
    /// Locked kind for this symbol.
    pub symbol_kind: SymbolKind,
    /// Timestamp of the event.
    pub at: ClockTime,
}

/// Pin / authoritative event record. Shared shape across the four
/// opcodes `PIN`/`UNPIN`/`AUTHORITATIVE_SET`/`AUTHORITATIVE_CLEAR`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlagEventRecord {
    /// Target memory.
    pub memory_id: SymbolId,
    /// Timestamp of the event.
    pub at: ClockTime,
    /// Agent or user who set/cleared the flag.
    pub actor_symbol: SymbolId,
}

/// A canonical-form record — the sum of every encoded record shape.
#[derive(Clone, Debug, PartialEq)]
pub enum CanonicalRecord {
    /// Semantic memory.
    Sem(SemRecord),
    /// Episodic memory.
    Epi(EpiRecord),
    /// Procedural memory.
    Pro(ProRecord),
    /// Inferential memory.
    Inf(InfRecord),
    /// Supersession edge.
    Supersedes(EdgeRecord),
    /// Episodic correction edge.
    Corrects(EdgeRecord),
    /// Inferential stale-parent edge.
    StaleParent(EdgeRecord),
    /// Inferential reconfirmation edge.
    Reconfirms(EdgeRecord),
    /// Episode boundary marker.
    Checkpoint(CheckpointRecord),
    /// Episode metadata (label / parent / retracts).
    EpisodeMeta(EpisodeMetaRecord),
    /// New symbol allocation.
    SymbolAlloc(SymbolEventRecord),
    /// Rename.
    SymbolRename(SymbolEventRecord),
    /// Alias.
    SymbolAlias(SymbolEventRecord),
    /// Retirement flag set.
    SymbolRetire(SymbolEventRecord),
    /// Retirement flag cleared.
    SymbolUnretire(SymbolEventRecord),
    /// Pin flag set.
    Pin(FlagEventRecord),
    /// Pin flag cleared.
    Unpin(FlagEventRecord),
    /// Operator-authoritative flag set.
    AuthoritativeSet(FlagEventRecord),
    /// Operator-authoritative flag cleared.
    AuthoritativeClear(FlagEventRecord),
}

impl CanonicalRecord {
    /// The opcode of this record.
    #[must_use]
    pub fn opcode(&self) -> Opcode {
        match self {
            Self::Sem(_) => Opcode::Sem,
            Self::Epi(_) => Opcode::Epi,
            Self::Pro(_) => Opcode::Pro,
            Self::Inf(_) => Opcode::Inf,
            Self::Supersedes(_) => Opcode::Supersedes,
            Self::Corrects(_) => Opcode::Corrects,
            Self::StaleParent(_) => Opcode::StaleParent,
            Self::Reconfirms(_) => Opcode::Reconfirms,
            Self::Checkpoint(_) => Opcode::Checkpoint,
            Self::EpisodeMeta(_) => Opcode::EpisodeMeta,
            Self::SymbolAlloc(_) => Opcode::SymbolAlloc,
            Self::SymbolRename(_) => Opcode::SymbolRename,
            Self::SymbolAlias(_) => Opcode::SymbolAlias,
            Self::SymbolRetire(_) => Opcode::SymbolRetire,
            Self::SymbolUnretire(_) => Opcode::SymbolUnretire,
            Self::Pin(_) => Opcode::Pin,
            Self::Unpin(_) => Opcode::Unpin,
            Self::AuthoritativeSet(_) => Opcode::AuthoritativeSet,
            Self::AuthoritativeClear(_) => Opcode::AuthoritativeClear,
        }
    }

    /// The librarian-assigned commit time for this record.
    ///
    /// Every canonical record carries a commit time — memory records in
    /// `clocks.committed_at`, edge / symbol / checkpoint / flag records
    /// in the `at` field. This accessor smooths over the field-name
    /// difference so replay and monotonicity checks can read the commit
    /// clock uniformly.
    #[must_use]
    pub fn committed_at(&self) -> ClockTime {
        match self {
            Self::Sem(r) => r.clocks.committed_at,
            Self::Epi(r) => r.committed_at,
            Self::Pro(r) => r.clocks.committed_at,
            Self::Inf(r) => r.clocks.committed_at,
            Self::Supersedes(r)
            | Self::Corrects(r)
            | Self::StaleParent(r)
            | Self::Reconfirms(r) => r.at,
            Self::Checkpoint(r) => r.at,
            Self::EpisodeMeta(r) => r.at,
            Self::SymbolAlloc(r)
            | Self::SymbolRename(r)
            | Self::SymbolAlias(r)
            | Self::SymbolRetire(r)
            | Self::SymbolUnretire(r) => r.at,
            Self::Pin(r)
            | Self::Unpin(r)
            | Self::AuthoritativeSet(r)
            | Self::AuthoritativeClear(r) => r.at,
        }
    }
}

// -------------------------------------------------------------------
// Errors
// -------------------------------------------------------------------

/// Errors produced by [`decode_record`].
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum DecodeError {
    /// The input ended before the record was fully decoded.
    #[error("truncated record at offset {offset}")]
    Truncated {
        /// Byte offset where truncation was detected.
        offset: usize,
    },

    /// The length-prefix said the body extended past the input.
    #[error(
        "length mismatch at offset {offset}: body expects {expected}, only {available} available"
    )]
    LengthMismatch {
        /// Byte offset of the record.
        offset: usize,
        /// Declared body length.
        expected: usize,
        /// Bytes available after the length prefix.
        available: usize,
    },

    /// Opcode byte is not in the registered set.
    #[error("unknown opcode {byte:#04x} at offset {offset}")]
    UnknownOpcode {
        /// The offending byte.
        byte: u8,
        /// Offset of the byte.
        offset: usize,
    },

    /// Value-tag byte is not in `0x01..=0x06`.
    #[error("unknown value tag {tag:#04x} at offset {offset}")]
    UnknownValueTag {
        /// The offending tag.
        tag: u8,
        /// Offset of the tag.
        offset: usize,
    },

    /// A string value was not valid UTF-8.
    #[error("invalid UTF-8 in string payload")]
    InvalidString,

    /// A `ClockTime` field on the wire carried the `u64::MAX` reserved
    /// sentinel, which [`ClockTime`] refuses to construct. Only the
    /// `invalid_at` slot is permitted to be `None`; every other clock
    /// field must be a concrete millisecond value.
    #[error("reserved ClockTime sentinel (u64::MAX) at offset {offset}")]
    ReservedClockSentinel {
        /// Offset of the first sentinel byte.
        offset: usize,
    },

    /// A symbol-kind ordinal byte did not correspond to any variant.
    #[error("unknown symbol-kind ordinal {ordinal} at offset {offset}")]
    UnknownSymbolKind {
        /// The offending byte.
        ordinal: u8,
        /// Offset of the byte.
        offset: usize,
    },

    /// Record body declared more bytes than the body contained.
    #[error("body underflow for opcode {opcode:?} at offset {offset}: consumed {consumed} of {declared}")]
    BodyUnderflow {
        /// The opcode being decoded.
        opcode: Opcode,
        /// Body offset inside the frame.
        offset: usize,
        /// Bytes consumed so far.
        consumed: usize,
        /// Declared body length.
        declared: usize,
    },

    /// Varint decoding overflowed the target type (more than 10 bytes
    /// for `u64`).
    #[error("varint overflow at offset {offset}")]
    VarintOverflow {
        /// Offset of the varint.
        offset: usize,
    },

    /// Varint was well-formed but not encoded with the shortest byte sequence.
    #[error("non-canonical varint at offset {offset}")]
    NonCanonicalVarint {
        /// Offset of the varint.
        offset: usize,
    },

    /// Reserved flag bits were set in a record flag byte.
    #[error("invalid flag byte {byte:#04x} at offset {offset}; allowed mask {allowed_mask:#04x}")]
    InvalidFlagBits {
        /// The offending flag byte.
        byte: u8,
        /// Mask of allowed flag bits for this record kind.
        allowed_mask: u8,
        /// Offset of the flag byte.
        offset: usize,
    },

    /// A field discriminant carried a value outside its canonical set.
    #[error("invalid {field} discriminant {tag:#04x} at offset {offset}")]
    InvalidDiscriminant {
        /// Name of the field being decoded.
        field: &'static str,
        /// The offending discriminant byte.
        tag: u8,
        /// Offset of the discriminant byte.
        offset: usize,
    },
}

impl fmt::Display for Opcode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

// -------------------------------------------------------------------
// Varint + fixed-LE helpers
// -------------------------------------------------------------------

#[allow(clippy::cast_possible_truncation)]
fn encode_varint(mut value: u64, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push(((value as u8) & 0x7F) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn decode_varint(bytes: &[u8], offset: &mut usize) -> Result<u64, DecodeError> {
    let start_offset = *offset;
    let mut result: u64 = 0;
    let mut shift: u32 = 0;
    for i in 0..10 {
        if *offset >= bytes.len() {
            return Err(DecodeError::Truncated { offset: *offset });
        }
        let b = bytes[*offset];
        *offset += 1;
        let part = u64::from(b & 0x7F);
        if i == 9 && part > 1 {
            return Err(DecodeError::VarintOverflow {
                offset: start_offset,
            });
        }
        result |= part.checked_shl(shift).ok_or(DecodeError::VarintOverflow {
            offset: start_offset,
        })?;
        if b & 0x80 == 0 {
            let consumed = *offset - start_offset;
            let mut canonical = Vec::new();
            encode_varint(result, &mut canonical);
            if consumed != canonical.len() {
                return Err(DecodeError::NonCanonicalVarint {
                    offset: start_offset,
                });
            }
            return Ok(result);
        }
        shift += 7;
        // Last allowed byte: 10th for u64.
        if i == 9 && (b & 0x80) != 0 {
            return Err(DecodeError::VarintOverflow {
                offset: start_offset,
            });
        }
    }
    Err(DecodeError::VarintOverflow {
        offset: start_offset,
    })
}

#[allow(clippy::cast_sign_loss)]
fn zigzag_encode(n: i64) -> u64 {
    ((n << 1) ^ (n >> 63)) as u64
}

#[allow(clippy::cast_possible_wrap)]
fn zigzag_decode(u: u64) -> i64 {
    let shifted = (u >> 1) as i64;
    let sign = -((u & 1) as i64);
    shifted ^ sign
}

fn encode_u64_le(value: u64, out: &mut Vec<u8>) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn decode_u64_le(bytes: &[u8], offset: &mut usize) -> Result<u64, DecodeError> {
    if *offset + 8 > bytes.len() {
        return Err(DecodeError::Truncated { offset: *offset });
    }
    let mut buf = [0_u8; 8];
    buf.copy_from_slice(&bytes[*offset..*offset + 8]);
    *offset += 8;
    Ok(u64::from_le_bytes(buf))
}

fn encode_u16_le(value: u16, out: &mut Vec<u8>) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn decode_u16_le(bytes: &[u8], offset: &mut usize) -> Result<u16, DecodeError> {
    if *offset + 2 > bytes.len() {
        return Err(DecodeError::Truncated { offset: *offset });
    }
    let mut buf = [0_u8; 2];
    buf.copy_from_slice(&bytes[*offset..*offset + 2]);
    *offset += 2;
    Ok(u16::from_le_bytes(buf))
}

fn encode_symbol(id: SymbolId, out: &mut Vec<u8>) {
    encode_varint(id.as_u64(), out);
}

fn decode_symbol(bytes: &[u8], offset: &mut usize) -> Result<SymbolId, DecodeError> {
    Ok(SymbolId::new(decode_varint(bytes, offset)?))
}

fn encode_clocktime(ct: ClockTime, out: &mut Vec<u8>) {
    encode_u64_le(ct.as_millis(), out);
}

fn decode_clocktime(bytes: &[u8], offset: &mut usize) -> Result<ClockTime, DecodeError> {
    // Callers that accept the reserved sentinel (`None`) use
    // decode_optional_clocktime instead; a sentinel in a non-Option slot
    // is genuine corruption.
    let sentinel_offset = *offset;
    let raw = decode_u64_le(bytes, offset)?;
    ClockTime::try_from_millis(raw).map_err(|_| DecodeError::ReservedClockSentinel {
        offset: sentinel_offset,
    })
}

fn encode_optional_clocktime(ct: Option<ClockTime>, out: &mut Vec<u8>) {
    match ct {
        Some(t) => encode_u64_le(t.as_millis(), out),
        None => encode_u64_le(NONE_SENTINEL, out),
    }
}

fn decode_optional_clocktime(
    bytes: &[u8],
    offset: &mut usize,
) -> Result<Option<ClockTime>, DecodeError> {
    let sentinel_offset = *offset;
    let raw = decode_u64_le(bytes, offset)?;
    if raw == NONE_SENTINEL {
        Ok(None)
    } else {
        // Sentinel is the `None` case above; any `try_from_millis`
        // failure at a non-sentinel value would be a `ClockTime`
        // invariant change — not reachable in the current codebase,
        // but routed to the correct variant for future-proofing.
        ClockTime::try_from_millis(raw)
            .map(Some)
            .map_err(|_| DecodeError::ReservedClockSentinel {
                offset: sentinel_offset,
            })
    }
}

fn encode_confidence(c: Confidence, out: &mut Vec<u8>) {
    encode_u16_le(c.as_u16(), out);
}

fn decode_confidence(bytes: &[u8], offset: &mut usize) -> Result<Confidence, DecodeError> {
    Ok(Confidence::from_u16(decode_u16_le(bytes, offset)?))
}

// -------------------------------------------------------------------
// Value encoding (tag + body)
// -------------------------------------------------------------------

pub(crate) fn encode_value(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Symbol(id) => {
            out.push(0x01);
            encode_varint(id.as_u64(), out);
        }
        Value::Integer(i) => {
            out.push(0x02);
            encode_varint(zigzag_encode(*i), out);
        }
        Value::Float(f) => {
            out.push(0x03);
            out.extend_from_slice(&f.to_le_bytes());
        }
        Value::Boolean(b) => {
            out.push(0x04);
            out.push(u8::from(*b));
        }
        Value::String(s) => {
            out.push(0x05);
            let bytes = s.as_bytes();
            #[allow(clippy::cast_possible_truncation)]
            let len = bytes.len() as u64;
            encode_varint(len, out);
            out.extend_from_slice(bytes);
        }
        Value::Timestamp(ct) => {
            out.push(0x06);
            encode_u64_le(ct.as_millis(), out);
        }
    }
}

fn decode_value(bytes: &[u8], offset: &mut usize) -> Result<Value, DecodeError> {
    if *offset >= bytes.len() {
        return Err(DecodeError::Truncated { offset: *offset });
    }
    let tag = bytes[*offset];
    let tag_offset = *offset;
    *offset += 1;
    let value = match tag {
        0x01 => Value::Symbol(decode_symbol(bytes, offset)?),
        0x02 => Value::Integer(zigzag_decode(decode_varint(bytes, offset)?)),
        0x03 => {
            if *offset + 8 > bytes.len() {
                return Err(DecodeError::Truncated { offset: *offset });
            }
            let mut buf = [0_u8; 8];
            buf.copy_from_slice(&bytes[*offset..*offset + 8]);
            *offset += 8;
            Value::Float(f64::from_le_bytes(buf))
        }
        0x04 => {
            if *offset >= bytes.len() {
                return Err(DecodeError::Truncated { offset: *offset });
            }
            let b = bytes[*offset] != 0;
            *offset += 1;
            Value::Boolean(b)
        }
        0x05 => {
            let len = usize::try_from(decode_varint(bytes, offset)?)
                .map_err(|_| DecodeError::VarintOverflow { offset: tag_offset })?;
            if *offset + len > bytes.len() {
                return Err(DecodeError::Truncated { offset: *offset });
            }
            let s = std::str::from_utf8(&bytes[*offset..*offset + len])
                .map_err(|_| DecodeError::InvalidString)?
                .to_string();
            *offset += len;
            Value::String(s)
        }
        0x06 => {
            let sentinel_offset = *offset;
            let raw = decode_u64_le(bytes, offset)?;
            Value::Timestamp(ClockTime::try_from_millis(raw).map_err(|_| {
                DecodeError::ReservedClockSentinel {
                    offset: sentinel_offset,
                }
            })?)
        }
        other => {
            return Err(DecodeError::UnknownValueTag {
                tag: other,
                offset: tag_offset,
            });
        }
    };
    Ok(value)
}

// -------------------------------------------------------------------
// SymbolKind ordinal
// -------------------------------------------------------------------

fn symbol_kind_to_u8(kind: SymbolKind) -> u8 {
    match kind {
        SymbolKind::Agent => 0,
        SymbolKind::Document => 1,
        SymbolKind::Registry => 2,
        SymbolKind::Service => 3,
        SymbolKind::Policy => 4,
        SymbolKind::Memory => 5,
        SymbolKind::InferenceMethod => 6,
        SymbolKind::Scope => 7,
        SymbolKind::Predicate => 8,
        SymbolKind::EventType => 9,
        SymbolKind::Workspace => 10,
        SymbolKind::Literal => 11,
    }
}

fn symbol_kind_from_u8(b: u8, offset: usize) -> Result<SymbolKind, DecodeError> {
    Ok(match b {
        0 => SymbolKind::Agent,
        1 => SymbolKind::Document,
        2 => SymbolKind::Registry,
        3 => SymbolKind::Service,
        4 => SymbolKind::Policy,
        5 => SymbolKind::Memory,
        6 => SymbolKind::InferenceMethod,
        7 => SymbolKind::Scope,
        8 => SymbolKind::Predicate,
        9 => SymbolKind::EventType,
        10 => SymbolKind::Workspace,
        11 => SymbolKind::Literal,
        other => {
            return Err(DecodeError::UnknownSymbolKind {
                ordinal: other,
                offset,
            });
        }
    })
}

// -------------------------------------------------------------------
// Record body encode / decode
// -------------------------------------------------------------------

fn encode_clocks(clocks: &Clocks, out: &mut Vec<u8>) {
    encode_clocktime(clocks.valid_at, out);
    encode_clocktime(clocks.observed_at, out);
    encode_clocktime(clocks.committed_at, out);
    encode_optional_clocktime(clocks.invalid_at, out);
}

fn decode_clocks(bytes: &[u8], offset: &mut usize) -> Result<Clocks, DecodeError> {
    let valid_at = decode_clocktime(bytes, offset)?;
    let observed_at = decode_clocktime(bytes, offset)?;
    let committed_at = decode_clocktime(bytes, offset)?;
    let invalid_at = decode_optional_clocktime(bytes, offset)?;
    Ok(Clocks {
        valid_at,
        observed_at,
        committed_at,
        invalid_at,
    })
}

fn encode_body(record: &CanonicalRecord, out: &mut Vec<u8>) {
    match record {
        CanonicalRecord::Sem(r) => encode_sem_body(r, out),
        CanonicalRecord::Epi(r) => encode_epi_body(r, out),
        CanonicalRecord::Pro(r) => encode_pro_body(r, out),
        CanonicalRecord::Inf(r) => encode_inf_body(r, out),
        CanonicalRecord::Supersedes(r)
        | CanonicalRecord::Corrects(r)
        | CanonicalRecord::StaleParent(r)
        | CanonicalRecord::Reconfirms(r) => encode_edge_body(r, out),
        CanonicalRecord::Checkpoint(r) => encode_checkpoint_body(r, out),
        CanonicalRecord::EpisodeMeta(r) => encode_episode_meta_body(r, out),
        CanonicalRecord::SymbolAlloc(r)
        | CanonicalRecord::SymbolRename(r)
        | CanonicalRecord::SymbolAlias(r)
        | CanonicalRecord::SymbolRetire(r)
        | CanonicalRecord::SymbolUnretire(r) => encode_symbol_event_body(r, out),
        CanonicalRecord::Pin(r)
        | CanonicalRecord::Unpin(r)
        | CanonicalRecord::AuthoritativeSet(r)
        | CanonicalRecord::AuthoritativeClear(r) => encode_flag_event_body(r, out),
    }
}

fn encode_sem_body(r: &SemRecord, out: &mut Vec<u8>) {
    encode_symbol(r.memory_id, out);
    encode_symbol(r.s, out);
    encode_symbol(r.p, out);
    encode_value(&r.o, out);
    encode_symbol(r.source, out);
    encode_confidence(r.confidence, out);
    encode_clocks(&r.clocks, out);
    out.push(r.flags.to_u8());
}

fn decode_sem_body(bytes: &[u8], offset: &mut usize) -> Result<SemRecord, DecodeError> {
    let memory_id = decode_symbol(bytes, offset)?;
    let s = decode_symbol(bytes, offset)?;
    let p = decode_symbol(bytes, offset)?;
    let o = decode_value(bytes, offset)?;
    let source = decode_symbol(bytes, offset)?;
    let confidence = decode_confidence(bytes, offset)?;
    let clocks = decode_clocks(bytes, offset)?;
    let flag_offset = *offset;
    let flags = SemFlags::try_from_u8(decode_flag_byte(bytes, offset)?, flag_offset)?;

    Ok(SemRecord {
        memory_id,
        s,
        p,
        o,
        source,
        confidence,
        clocks,
        flags,
    })
}

fn encode_epi_body(r: &EpiRecord, out: &mut Vec<u8>) {
    encode_symbol(r.memory_id, out);
    encode_symbol(r.event_id, out);
    encode_symbol(r.kind, out);
    #[allow(clippy::cast_possible_truncation)]
    encode_varint(r.participants.len() as u64, out);
    for p in &r.participants {
        encode_symbol(*p, out);
    }
    encode_symbol(r.location, out);
    encode_clocktime(r.at_time, out);
    encode_clocktime(r.observed_at, out);
    encode_symbol(r.source, out);
    encode_confidence(r.confidence, out);
    encode_clocktime(r.committed_at, out);
    encode_optional_clocktime(r.invalid_at, out);
    // No flags byte for Episodic — Epi doesn't project or stale.
}

fn decode_epi_body(bytes: &[u8], offset: &mut usize) -> Result<EpiRecord, DecodeError> {
    let memory_id = decode_symbol(bytes, offset)?;
    let event_id = decode_symbol(bytes, offset)?;
    let kind = decode_symbol(bytes, offset)?;
    let count = usize::try_from(decode_varint(bytes, offset)?)
        .map_err(|_| DecodeError::VarintOverflow { offset: *offset })?;
    // Cap allocation by remaining body bytes — each `decode_symbol`
    // consumes at least one byte, so `bytes.len() - *offset` is a sound
    // upper bound on honest counts. Without this cap, an attacker who
    // sets `count` near `usize::MAX` (encoded as a 10-byte varint) would
    // trigger a multi-exabyte `Vec::with_capacity`, aborting the
    // process before the decode loop returns `Truncated`. Closes
    // Security F2 (P2) from the v1.1 fresh assessment.
    let cap = count.min(bytes.len().saturating_sub(*offset));
    let mut participants = Vec::with_capacity(cap);
    for _ in 0..count {
        participants.push(decode_symbol(bytes, offset)?);
    }
    Ok(EpiRecord {
        memory_id,
        event_id,
        kind,
        participants,
        location: decode_symbol(bytes, offset)?,
        at_time: decode_clocktime(bytes, offset)?,
        observed_at: decode_clocktime(bytes, offset)?,
        source: decode_symbol(bytes, offset)?,
        confidence: decode_confidence(bytes, offset)?,
        committed_at: decode_clocktime(bytes, offset)?,
        invalid_at: decode_optional_clocktime(bytes, offset)?,
    })
}

fn encode_pro_body(r: &ProRecord, out: &mut Vec<u8>) {
    encode_symbol(r.memory_id, out);
    encode_symbol(r.rule_id, out);
    encode_value(&r.trigger, out);
    encode_value(&r.action, out);
    match &r.precondition {
        Some(pre) => {
            out.push(0x01);
            encode_value(pre, out);
        }
        None => out.push(0x00),
    }
    encode_symbol(r.scope, out);
    encode_symbol(r.source, out);
    encode_confidence(r.confidence, out);
    encode_clocks(&r.clocks, out);
    // No flags byte for Procedural — Pro doesn't project or stale.
}

fn decode_pro_body(bytes: &[u8], offset: &mut usize) -> Result<ProRecord, DecodeError> {
    let memory_id = decode_symbol(bytes, offset)?;
    let rule_id = decode_symbol(bytes, offset)?;
    let trigger = decode_value(bytes, offset)?;
    let action = decode_value(bytes, offset)?;
    if *offset >= bytes.len() {
        return Err(DecodeError::Truncated { offset: *offset });
    }
    let precondition_offset = *offset;
    let has_pre = bytes[*offset];
    *offset += 1;
    let precondition = match has_pre {
        0 => None,
        1 => Some(decode_value(bytes, offset)?),
        tag => {
            return Err(DecodeError::InvalidDiscriminant {
                field: "procedural precondition",
                tag,
                offset: precondition_offset,
            });
        }
    };
    Ok(ProRecord {
        memory_id,
        rule_id,
        trigger,
        action,
        precondition,
        scope: decode_symbol(bytes, offset)?,
        source: decode_symbol(bytes, offset)?,
        confidence: decode_confidence(bytes, offset)?,
        clocks: decode_clocks(bytes, offset)?,
    })
}

fn encode_inf_body(r: &InfRecord, out: &mut Vec<u8>) {
    encode_symbol(r.memory_id, out);
    encode_symbol(r.s, out);
    encode_symbol(r.p, out);
    encode_value(&r.o, out);
    #[allow(clippy::cast_possible_truncation)]
    encode_varint(r.derived_from.len() as u64, out);
    for parent in &r.derived_from {
        encode_symbol(*parent, out);
    }
    encode_symbol(r.method, out);
    encode_confidence(r.confidence, out);
    encode_clocks(&r.clocks, out);
    out.push(r.flags.to_u8());
}

fn decode_inf_body(bytes: &[u8], offset: &mut usize) -> Result<InfRecord, DecodeError> {
    let memory_id = decode_symbol(bytes, offset)?;
    let s = decode_symbol(bytes, offset)?;
    let p = decode_symbol(bytes, offset)?;
    let o = decode_value(bytes, offset)?;
    let count = usize::try_from(decode_varint(bytes, offset)?)
        .map_err(|_| DecodeError::VarintOverflow { offset: *offset })?;
    // See `decode_epi_body` — same OOM-on-malicious-varint mitigation.
    let cap = count.min(bytes.len().saturating_sub(*offset));
    let mut derived_from = Vec::with_capacity(cap);
    for _ in 0..count {
        derived_from.push(decode_symbol(bytes, offset)?);
    }
    let method = decode_symbol(bytes, offset)?;
    let confidence = decode_confidence(bytes, offset)?;
    let clocks = decode_clocks(bytes, offset)?;
    let flag_offset = *offset;
    let flags = InfFlags::try_from_u8(decode_flag_byte(bytes, offset)?, flag_offset)?;

    Ok(InfRecord {
        memory_id,
        s,
        p,
        o,
        derived_from,
        method,
        confidence,
        clocks,
        flags,
    })
}

fn encode_edge_body(r: &EdgeRecord, out: &mut Vec<u8>) {
    encode_symbol(r.from, out);
    encode_symbol(r.to, out);
    encode_clocktime(r.at, out);
}

fn decode_edge_body(bytes: &[u8], offset: &mut usize) -> Result<EdgeRecord, DecodeError> {
    Ok(EdgeRecord {
        from: decode_symbol(bytes, offset)?,
        to: decode_symbol(bytes, offset)?,
        at: decode_clocktime(bytes, offset)?,
    })
}

fn encode_checkpoint_body(r: &CheckpointRecord, out: &mut Vec<u8>) {
    encode_symbol(r.episode_id, out);
    encode_clocktime(r.at, out);
    encode_varint(r.memory_count, out);
}

fn decode_checkpoint_body(
    bytes: &[u8],
    offset: &mut usize,
) -> Result<CheckpointRecord, DecodeError> {
    Ok(CheckpointRecord {
        episode_id: decode_symbol(bytes, offset)?,
        at: decode_clocktime(bytes, offset)?,
        memory_count: decode_varint(bytes, offset)?,
    })
}

fn encode_episode_meta_body(r: &EpisodeMetaRecord, out: &mut Vec<u8>) {
    encode_symbol(r.episode_id, out);
    encode_clocktime(r.at, out);
    // Label: length-prefixed UTF-8, `0` encodes `None`. A `Some("")`
    // collapses to `None` because episode-semantics.md § 4.3 treats
    // the empty string as meaningless.
    let label_bytes: &[u8] = match r.label.as_deref() {
        Some(s) if !s.is_empty() => s.as_bytes(),
        _ => &[],
    };
    #[allow(clippy::cast_possible_truncation)]
    encode_varint(label_bytes.len() as u64, out);
    out.extend_from_slice(label_bytes);
    // parent_episode: 0 = None, 1 = Some followed by the symbol.
    if let Some(id) = r.parent_episode_id {
        out.push(0x01);
        encode_symbol(id, out);
    } else {
        out.push(0x00);
    }
    // retracts: length-prefixed list of symbol ids.
    #[allow(clippy::cast_possible_truncation)]
    encode_varint(r.retracts.len() as u64, out);
    for id in &r.retracts {
        encode_symbol(*id, out);
    }
}

fn decode_episode_meta_body(
    bytes: &[u8],
    offset: &mut usize,
) -> Result<EpisodeMetaRecord, DecodeError> {
    let episode_id = decode_symbol(bytes, offset)?;
    let at = decode_clocktime(bytes, offset)?;
    let label_len = usize::try_from(decode_varint(bytes, offset)?)
        .map_err(|_| DecodeError::VarintOverflow { offset: *offset })?;
    let label = if label_len == 0 {
        None
    } else {
        if *offset + label_len > bytes.len() {
            return Err(DecodeError::Truncated { offset: *offset });
        }
        let s = std::str::from_utf8(&bytes[*offset..*offset + label_len])
            .map_err(|_| DecodeError::InvalidString)?
            .to_string();
        *offset += label_len;
        Some(s)
    };
    if *offset >= bytes.len() {
        return Err(DecodeError::Truncated { offset: *offset });
    }
    let parent_tag = bytes[*offset];
    *offset += 1;
    let parent_episode_id = match parent_tag {
        0x00 => None,
        0x01 => Some(decode_symbol(bytes, offset)?),
        _ => return Err(DecodeError::InvalidString),
    };
    let retracts_len = usize::try_from(decode_varint(bytes, offset)?)
        .map_err(|_| DecodeError::VarintOverflow { offset: *offset })?;
    // See `decode_epi_body` — same OOM-on-malicious-varint mitigation.
    let retracts_cap = retracts_len.min(bytes.len().saturating_sub(*offset));
    let mut retracts = Vec::with_capacity(retracts_cap);
    for _ in 0..retracts_len {
        retracts.push(decode_symbol(bytes, offset)?);
    }
    Ok(EpisodeMetaRecord {
        episode_id,
        at,
        label,
        parent_episode_id,
        retracts,
    })
}

fn encode_symbol_event_body(r: &SymbolEventRecord, out: &mut Vec<u8>) {
    encode_symbol(r.symbol_id, out);
    let name_bytes = r.name.as_bytes();
    #[allow(clippy::cast_possible_truncation)]
    encode_varint(name_bytes.len() as u64, out);
    out.extend_from_slice(name_bytes);
    out.push(symbol_kind_to_u8(r.symbol_kind));
    encode_clocktime(r.at, out);
}

fn decode_symbol_event_body(
    bytes: &[u8],
    offset: &mut usize,
) -> Result<SymbolEventRecord, DecodeError> {
    let symbol_id = decode_symbol(bytes, offset)?;
    let name_len = usize::try_from(decode_varint(bytes, offset)?)
        .map_err(|_| DecodeError::VarintOverflow { offset: *offset })?;
    if *offset + name_len > bytes.len() {
        return Err(DecodeError::Truncated { offset: *offset });
    }
    let name = std::str::from_utf8(&bytes[*offset..*offset + name_len])
        .map_err(|_| DecodeError::InvalidString)?
        .to_string();
    *offset += name_len;
    if *offset >= bytes.len() {
        return Err(DecodeError::Truncated { offset: *offset });
    }
    let kind_byte = bytes[*offset];
    let kind_offset = *offset;
    *offset += 1;
    let symbol_kind = symbol_kind_from_u8(kind_byte, kind_offset)?;
    let at = decode_clocktime(bytes, offset)?;
    Ok(SymbolEventRecord {
        symbol_id,
        name,
        symbol_kind,
        at,
    })
}

fn encode_flag_event_body(r: &FlagEventRecord, out: &mut Vec<u8>) {
    encode_symbol(r.memory_id, out);
    encode_clocktime(r.at, out);
    encode_symbol(r.actor_symbol, out);
}

fn decode_flag_event_body(
    bytes: &[u8],
    offset: &mut usize,
) -> Result<FlagEventRecord, DecodeError> {
    Ok(FlagEventRecord {
        memory_id: decode_symbol(bytes, offset)?,
        at: decode_clocktime(bytes, offset)?,
        actor_symbol: decode_symbol(bytes, offset)?,
    })
}

fn decode_flag_byte(bytes: &[u8], offset: &mut usize) -> Result<u8, DecodeError> {
    if *offset >= bytes.len() {
        return Err(DecodeError::Truncated { offset: *offset });
    }
    let b = bytes[*offset];
    *offset += 1;
    Ok(b)
}

// -------------------------------------------------------------------
// Public framing API
// -------------------------------------------------------------------

/// Encode a [`CanonicalRecord`] with its framing into the output buffer.
///
/// Framing: `[1 byte opcode][varint body length][body]`.
///
/// # Examples
///
/// ```
/// # #![allow(clippy::unwrap_used)]
/// use mimir_core::canonical::{
///     encode_record, decode_record, CanonicalRecord, CheckpointRecord,
/// };
/// use mimir_core::{ClockTime, SymbolId};
///
/// let record = CanonicalRecord::Checkpoint(CheckpointRecord {
///     episode_id: SymbolId::new(42),
///     at: ClockTime::try_from_millis(1_700_000_000_000).expect("non-sentinel"),
///     memory_count: 3,
/// });
/// let mut bytes = Vec::new();
/// encode_record(&record, &mut bytes);
/// let (decoded, _used) = decode_record(&bytes).unwrap();
/// assert_eq!(decoded, record);
/// ```
pub fn encode_record(record: &CanonicalRecord, out: &mut Vec<u8>) {
    out.push(record.opcode() as u8);
    // Encode the body into a temporary buffer so we can prefix its length.
    let mut body = Vec::new();
    encode_body(record, &mut body);
    #[allow(clippy::cast_possible_truncation)]
    encode_varint(body.len() as u64, out);
    out.extend_from_slice(&body);
}

/// Decode one [`CanonicalRecord`] from the front of `bytes`.
///
/// Returns the decoded record and the total number of bytes consumed
/// (opcode + length varint + body).
///
/// # Errors
///
/// - [`DecodeError::Truncated`] if the input ends inside the record.
/// - [`DecodeError::UnknownOpcode`] if the first byte isn't a known opcode.
/// - [`DecodeError::LengthMismatch`] if the declared body length overruns
///   the input.
/// - [`DecodeError::BodyUnderflow`] if the body encoded less than the
///   declared length.
pub fn decode_record(bytes: &[u8]) -> Result<(CanonicalRecord, usize), DecodeError> {
    if bytes.is_empty() {
        return Err(DecodeError::Truncated { offset: 0 });
    }
    let opcode_byte = bytes[0];
    let opcode = Opcode::from_byte(opcode_byte).ok_or(DecodeError::UnknownOpcode {
        byte: opcode_byte,
        offset: 0,
    })?;
    let mut offset = 1;
    let body_len = usize::try_from(decode_varint(bytes, &mut offset)?)
        .map_err(|_| DecodeError::VarintOverflow { offset: 1 })?;
    let body_start = offset;
    if body_start + body_len > bytes.len() {
        return Err(DecodeError::LengthMismatch {
            offset: 0,
            expected: body_len,
            available: bytes.len() - body_start,
        });
    }
    let body = &bytes[body_start..body_start + body_len];
    let mut body_offset = 0;
    let record = match opcode {
        Opcode::Sem => CanonicalRecord::Sem(decode_sem_body(body, &mut body_offset)?),
        Opcode::Epi => CanonicalRecord::Epi(decode_epi_body(body, &mut body_offset)?),
        Opcode::Pro => CanonicalRecord::Pro(decode_pro_body(body, &mut body_offset)?),
        Opcode::Inf => CanonicalRecord::Inf(decode_inf_body(body, &mut body_offset)?),
        Opcode::Supersedes => {
            CanonicalRecord::Supersedes(decode_edge_body(body, &mut body_offset)?)
        }
        Opcode::Corrects => CanonicalRecord::Corrects(decode_edge_body(body, &mut body_offset)?),
        Opcode::StaleParent => {
            CanonicalRecord::StaleParent(decode_edge_body(body, &mut body_offset)?)
        }
        Opcode::Reconfirms => {
            CanonicalRecord::Reconfirms(decode_edge_body(body, &mut body_offset)?)
        }
        Opcode::Checkpoint => {
            CanonicalRecord::Checkpoint(decode_checkpoint_body(body, &mut body_offset)?)
        }
        Opcode::EpisodeMeta => {
            CanonicalRecord::EpisodeMeta(decode_episode_meta_body(body, &mut body_offset)?)
        }
        Opcode::SymbolAlloc => {
            CanonicalRecord::SymbolAlloc(decode_symbol_event_body(body, &mut body_offset)?)
        }
        Opcode::SymbolRename => {
            CanonicalRecord::SymbolRename(decode_symbol_event_body(body, &mut body_offset)?)
        }
        Opcode::SymbolAlias => {
            CanonicalRecord::SymbolAlias(decode_symbol_event_body(body, &mut body_offset)?)
        }
        Opcode::SymbolRetire => {
            CanonicalRecord::SymbolRetire(decode_symbol_event_body(body, &mut body_offset)?)
        }
        Opcode::SymbolUnretire => {
            CanonicalRecord::SymbolUnretire(decode_symbol_event_body(body, &mut body_offset)?)
        }
        Opcode::Pin => CanonicalRecord::Pin(decode_flag_event_body(body, &mut body_offset)?),
        Opcode::Unpin => CanonicalRecord::Unpin(decode_flag_event_body(body, &mut body_offset)?),
        Opcode::AuthoritativeSet => {
            CanonicalRecord::AuthoritativeSet(decode_flag_event_body(body, &mut body_offset)?)
        }
        Opcode::AuthoritativeClear => {
            CanonicalRecord::AuthoritativeClear(decode_flag_event_body(body, &mut body_offset)?)
        }
    };
    if body_offset != body.len() {
        return Err(DecodeError::BodyUnderflow {
            opcode,
            offset: body_start,
            consumed: body_offset,
            declared: body_len,
        });
    }
    Ok((record, body_start + body_len))
}

/// Decode all records from a byte slice until the slice is exhausted.
///
/// # Errors
///
/// Propagates any [`DecodeError`] from the underlying stream.
pub fn decode_all(bytes: &[u8]) -> Result<Vec<CanonicalRecord>, DecodeError> {
    let mut out = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let (record, used) = decode_record(&bytes[offset..])?;
        out.push(record);
        offset += used;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ct(ms: u64) -> ClockTime {
        ClockTime::try_from_millis(ms).expect("non-sentinel")
    }

    fn clocks() -> Clocks {
        Clocks {
            valid_at: ct(1_700_000_000_000),
            observed_at: ct(1_700_000_001_000),
            committed_at: ct(1_700_000_002_000),
            invalid_at: None,
        }
    }

    fn roundtrip(record: &CanonicalRecord) {
        let mut bytes = Vec::new();
        encode_record(record, &mut bytes);
        let (decoded, used) = decode_record(&bytes).unwrap();
        assert_eq!(&decoded, record);
        assert_eq!(used, bytes.len());
    }

    #[test]
    fn varint_roundtrip_small() {
        for v in [0_u64, 1, 127, 128, 16_383, 16_384, u64::MAX] {
            let mut out = Vec::new();
            encode_varint(v, &mut out);
            let mut offset = 0;
            let decoded = decode_varint(&out, &mut offset).unwrap();
            assert_eq!(decoded, v);
            assert_eq!(offset, out.len());
        }
    }

    #[test]
    fn overlong_varint_encoding_is_rejected() {
        let mut offset = 0;
        let err = decode_varint(&[0x80, 0x00], &mut offset).unwrap_err();
        assert!(matches!(err, DecodeError::NonCanonicalVarint { offset: 0 }));
    }

    #[test]
    fn zigzag_roundtrip() {
        for i in [0_i64, 1, -1, 42, -42, i64::MIN, i64::MAX] {
            assert_eq!(zigzag_decode(zigzag_encode(i)), i);
        }
    }

    #[test]
    fn value_roundtrip_all_tags() {
        let values = [
            Value::Symbol(SymbolId::new(7)),
            Value::Integer(-42),
            Value::Float(1.25),
            Value::Boolean(true),
            Value::String("hello".into()),
            Value::Timestamp(ct(12_345)),
        ];
        for v in values {
            let mut bytes = Vec::new();
            encode_value(&v, &mut bytes);
            let mut offset = 0;
            let decoded = decode_value(&bytes, &mut offset).unwrap();
            assert_eq!(decoded, v);
            assert_eq!(offset, bytes.len());
        }
    }

    #[test]
    fn sem_roundtrip() {
        roundtrip(&CanonicalRecord::Sem(SemRecord {
            memory_id: SymbolId::new(1),
            s: SymbolId::new(2),
            p: SymbolId::new(3),
            o: Value::String("x".into()),
            source: SymbolId::new(4),
            confidence: Confidence::from_u16(62_258),
            clocks: clocks(),
            flags: SemFlags::default(),
        }));
    }

    #[test]
    fn sem_reserved_flag_bits_are_rejected() {
        let mut bytes = Vec::new();
        encode_record(
            &CanonicalRecord::Sem(SemRecord {
                memory_id: SymbolId::new(1),
                s: SymbolId::new(2),
                p: SymbolId::new(3),
                o: Value::String("x".into()),
                source: SymbolId::new(4),
                confidence: Confidence::from_u16(62_258),
                clocks: clocks(),
                flags: SemFlags::default(),
            }),
            &mut bytes,
        );
        *bytes.last_mut().expect("flag byte") = 0b0000_0010;
        let err = decode_record(&bytes).unwrap_err();
        assert!(matches!(
            err,
            DecodeError::InvalidFlagBits {
                byte: 0b0000_0010,
                allowed_mask: 0b0000_0001,
                ..
            }
        ));
    }

    #[test]
    fn epi_roundtrip_with_participants() {
        roundtrip(&CanonicalRecord::Epi(EpiRecord {
            memory_id: SymbolId::new(10),
            event_id: SymbolId::new(11),
            kind: SymbolId::new(12),
            participants: vec![SymbolId::new(13), SymbolId::new(14)],
            location: SymbolId::new(15),
            at_time: ct(1_700_000_000_000),
            observed_at: ct(1_700_000_000_000),
            source: SymbolId::new(16),
            confidence: Confidence::ONE,
            committed_at: ct(1_700_000_005_000),
            invalid_at: None,
        }));
    }

    #[test]
    fn pro_roundtrip_with_precondition() {
        roundtrip(&CanonicalRecord::Pro(ProRecord {
            memory_id: SymbolId::new(20),
            rule_id: SymbolId::new(21),
            trigger: Value::String("agent writing".into()),
            action: Value::String("route via librarian".into()),
            precondition: Some(Value::String("critical".into())),
            scope: SymbolId::new(22),
            source: SymbolId::new(23),
            confidence: Confidence::ONE,
            clocks: clocks(),
        }));
    }

    #[test]
    fn pro_precondition_tag_must_be_zero_or_one() {
        let mut bytes = Vec::new();
        encode_record(
            &CanonicalRecord::Pro(ProRecord {
                memory_id: SymbolId::new(20),
                rule_id: SymbolId::new(21),
                trigger: Value::String("agent writing".into()),
                action: Value::String("route via librarian".into()),
                precondition: Some(Value::String("critical".into())),
                scope: SymbolId::new(22),
                source: SymbolId::new(23),
                confidence: Confidence::ONE,
                clocks: clocks(),
            }),
            &mut bytes,
        );

        // Framing is one-byte opcode plus one-byte body length for
        // this fixture. Body layout starts with memory_id, rule_id,
        // trigger string, action string, then the precondition tag.
        let precondition_tag_offset =
            2 + 1 + 1 + 1 + 1 + "agent writing".len() + 1 + 1 + "route via librarian".len();
        assert_eq!(bytes[precondition_tag_offset], 0x01);
        bytes[precondition_tag_offset] = 0x02;
        let err = decode_record(&bytes).unwrap_err();
        assert!(matches!(
            err,
            DecodeError::InvalidDiscriminant {
                field: "procedural precondition",
                tag: 0x02,
                ..
            }
        ));
    }

    #[test]
    fn pro_roundtrip_without_precondition() {
        roundtrip(&CanonicalRecord::Pro(ProRecord {
            memory_id: SymbolId::new(30),
            rule_id: SymbolId::new(31),
            trigger: Value::String("x".into()),
            action: Value::String("y".into()),
            precondition: None,
            scope: SymbolId::new(32),
            source: SymbolId::new(33),
            confidence: Confidence::from_u16(40_000),
            clocks: clocks(),
        }));
    }

    #[test]
    fn inf_roundtrip_with_stale_flag() {
        roundtrip(&CanonicalRecord::Inf(InfRecord {
            memory_id: SymbolId::new(40),
            s: SymbolId::new(41),
            p: SymbolId::new(42),
            o: Value::Boolean(true),
            derived_from: vec![SymbolId::new(43), SymbolId::new(44), SymbolId::new(45)],
            method: SymbolId::new(46),
            confidence: Confidence::from_u16(50_000),
            clocks: clocks(),
            flags: InfFlags {
                projected: true,
                stale: true,
            },
        }));
    }

    #[test]
    fn inf_reserved_flag_bits_are_rejected() {
        let mut bytes = Vec::new();
        encode_record(
            &CanonicalRecord::Inf(InfRecord {
                memory_id: SymbolId::new(40),
                s: SymbolId::new(41),
                p: SymbolId::new(42),
                o: Value::Boolean(true),
                derived_from: vec![SymbolId::new(43)],
                method: SymbolId::new(46),
                confidence: Confidence::from_u16(50_000),
                clocks: clocks(),
                flags: InfFlags {
                    projected: true,
                    stale: false,
                },
            }),
            &mut bytes,
        );
        *bytes.last_mut().expect("flag byte") = 0b0000_0100;
        let err = decode_record(&bytes).unwrap_err();
        assert!(matches!(
            err,
            DecodeError::InvalidFlagBits {
                byte: 0b0000_0100,
                allowed_mask: 0b0000_0011,
                ..
            }
        ));
    }

    #[test]
    fn edge_records_roundtrip() {
        let edge = EdgeRecord {
            from: SymbolId::new(50),
            to: SymbolId::new(51),
            at: ct(1_700_000_010_000),
        };
        roundtrip(&CanonicalRecord::Supersedes(edge));
        roundtrip(&CanonicalRecord::Corrects(edge));
        roundtrip(&CanonicalRecord::StaleParent(edge));
        roundtrip(&CanonicalRecord::Reconfirms(edge));
    }

    #[test]
    fn checkpoint_roundtrip() {
        roundtrip(&CanonicalRecord::Checkpoint(CheckpointRecord {
            episode_id: SymbolId::new(100),
            at: ct(1_700_000_020_000),
            memory_count: 7,
        }));
    }

    #[test]
    fn episode_meta_roundtrip_minimal() {
        roundtrip(&CanonicalRecord::EpisodeMeta(EpisodeMetaRecord {
            episode_id: SymbolId::new(101),
            at: ct(1_700_000_020_000),
            label: None,
            parent_episode_id: None,
            retracts: Vec::new(),
        }));
    }

    #[test]
    fn episode_meta_roundtrip_full() {
        roundtrip(&CanonicalRecord::EpisodeMeta(EpisodeMetaRecord {
            episode_id: SymbolId::new(102),
            at: ct(1_700_000_021_000),
            label: Some("tokenizer-bakeoff".into()),
            parent_episode_id: Some(SymbolId::new(101)),
            retracts: vec![SymbolId::new(50), SymbolId::new(51)],
        }));
    }

    #[test]
    fn episode_meta_empty_label_decodes_to_none() {
        let mut buf = Vec::new();
        encode_record(
            &CanonicalRecord::EpisodeMeta(EpisodeMetaRecord {
                episode_id: SymbolId::new(103),
                at: ct(1_700_000_022_000),
                label: Some(String::new()),
                parent_episode_id: None,
                retracts: Vec::new(),
            }),
            &mut buf,
        );
        let (decoded, _) = decode_record(&buf).expect("decode");
        let CanonicalRecord::EpisodeMeta(meta) = decoded else {
            panic!("expected EpisodeMeta");
        };
        // Empty string collapses to None on the wire — encoder
        // intentionally normalizes so readers don't see "".
        assert_eq!(meta.label, None);
    }

    #[test]
    fn symbol_event_roundtrip() {
        let rec = SymbolEventRecord {
            symbol_id: SymbolId::new(200),
            name: "alice".into(),
            symbol_kind: SymbolKind::Agent,
            at: ct(1_700_000_030_000),
        };
        roundtrip(&CanonicalRecord::SymbolAlloc(rec.clone()));
        roundtrip(&CanonicalRecord::SymbolRename(rec.clone()));
        roundtrip(&CanonicalRecord::SymbolAlias(rec.clone()));
        roundtrip(&CanonicalRecord::SymbolRetire(rec.clone()));
        roundtrip(&CanonicalRecord::SymbolUnretire(rec));
    }

    #[test]
    fn flag_event_roundtrip() {
        let rec = FlagEventRecord {
            memory_id: SymbolId::new(300),
            at: ct(1_700_000_040_000),
            actor_symbol: SymbolId::new(301),
        };
        roundtrip(&CanonicalRecord::Pin(rec));
        roundtrip(&CanonicalRecord::Unpin(rec));
        roundtrip(&CanonicalRecord::AuthoritativeSet(rec));
        roundtrip(&CanonicalRecord::AuthoritativeClear(rec));
    }

    #[test]
    fn decode_all_multiple_records() {
        let records = vec![
            CanonicalRecord::Checkpoint(CheckpointRecord {
                episode_id: SymbolId::new(1),
                at: ct(1_000),
                memory_count: 0,
            }),
            CanonicalRecord::Supersedes(EdgeRecord {
                from: SymbolId::new(2),
                to: SymbolId::new(3),
                at: ct(2_000),
            }),
            CanonicalRecord::Pin(FlagEventRecord {
                memory_id: SymbolId::new(4),
                at: ct(3_000),
                actor_symbol: SymbolId::new(5),
            }),
        ];
        let mut bytes = Vec::new();
        for r in &records {
            encode_record(r, &mut bytes);
        }
        let decoded = decode_all(&bytes).unwrap();
        assert_eq!(decoded, records);
    }

    #[test]
    fn unknown_opcode_errors() {
        let err = decode_record(&[0x77, 0x00]).unwrap_err();
        assert!(matches!(err, DecodeError::UnknownOpcode { byte: 0x77, .. }));
    }

    #[test]
    fn truncated_input_errors() {
        let err = decode_record(&[]).unwrap_err();
        assert!(matches!(err, DecodeError::Truncated { .. }));
    }

    #[test]
    fn length_mismatch_errors() {
        // Opcode CHECKPOINT then declared body length 50, but only 2 bytes follow.
        let err = decode_record(&[0x20, 50, 0, 0]).unwrap_err();
        assert!(matches!(err, DecodeError::LengthMismatch { .. }));
    }

    #[test]
    fn unknown_value_tag_errors() {
        // Manually craft a SEM body with bad value tag.
        let mut body = Vec::new();
        encode_symbol(SymbolId::new(1), &mut body);
        encode_symbol(SymbolId::new(2), &mut body);
        encode_symbol(SymbolId::new(3), &mut body);
        body.push(0x99); // bogus tag
        let mut framed = Vec::new();
        framed.push(0x01); // SEM opcode
        #[allow(clippy::cast_possible_truncation)]
        encode_varint(body.len() as u64, &mut framed);
        framed.extend_from_slice(&body);
        let err = decode_record(&framed).unwrap_err();
        assert!(matches!(
            err,
            DecodeError::UnknownValueTag { tag: 0x99, .. }
        ));
    }

    #[test]
    fn confidence_fixed_width_two_bytes() {
        let record = CanonicalRecord::Sem(SemRecord {
            memory_id: SymbolId::new(1),
            s: SymbolId::new(2),
            p: SymbolId::new(3),
            o: Value::Integer(0),
            source: SymbolId::new(4),
            confidence: Confidence::from_u16(42),
            clocks: clocks(),
            flags: SemFlags::default(),
        });
        let mut bytes = Vec::new();
        encode_record(&record, &mut bytes);
        let (decoded, _) = decode_record(&bytes).unwrap();
        assert_eq!(decoded, record);
    }

    #[test]
    fn invalid_at_sentinel_is_none() {
        let record = CanonicalRecord::Sem(SemRecord {
            memory_id: SymbolId::new(1),
            s: SymbolId::new(2),
            p: SymbolId::new(3),
            o: Value::Integer(0),
            source: SymbolId::new(4),
            confidence: Confidence::ONE,
            clocks: Clocks {
                valid_at: ct(100),
                observed_at: ct(101),
                committed_at: ct(102),
                invalid_at: None,
            },
            flags: SemFlags::default(),
        });
        roundtrip(&record);
    }

    #[test]
    fn invalid_at_set_roundtrips() {
        let record = CanonicalRecord::Sem(SemRecord {
            memory_id: SymbolId::new(1),
            s: SymbolId::new(2),
            p: SymbolId::new(3),
            o: Value::Integer(0),
            source: SymbolId::new(4),
            confidence: Confidence::ONE,
            clocks: Clocks {
                valid_at: ct(100),
                observed_at: ct(101),
                committed_at: ct(102),
                invalid_at: Some(ct(200)),
            },
            flags: SemFlags::default(),
        });
        roundtrip(&record);
    }

    /// Pre-schema-break Epi/Pro logs carried a trailing `flags` byte
    /// whose `body_len` varint covered that byte. Feeding such a log
    /// to the post-split decoder must fail with `BodyUnderflow`, not
    /// silently drop the extra byte.
    #[test]
    fn legacy_epi_with_trailing_flags_byte_rejected() {
        let new_record = CanonicalRecord::Epi(EpiRecord {
            memory_id: SymbolId::new(1),
            event_id: SymbolId::new(2),
            kind: SymbolId::new(3),
            participants: vec![],
            location: SymbolId::new(4),
            at_time: ct(100),
            observed_at: ct(100),
            source: SymbolId::new(5),
            confidence: Confidence::ONE,
            committed_at: ct(100),
            invalid_at: None,
        });
        let mut new_bytes = Vec::new();
        encode_record(&new_record, &mut new_bytes);

        // Simulate an old-format frame: same body plus a trailing flags
        // byte, with body_len bumped by 1.
        let opcode = new_bytes[0];
        let mut cursor = 1;
        let body_len = decode_varint(&new_bytes, &mut cursor).unwrap();
        let body = &new_bytes[cursor..cursor + usize::try_from(body_len).unwrap()];
        let mut legacy = Vec::new();
        legacy.push(opcode);
        encode_varint(body_len + 1, &mut legacy);
        legacy.extend_from_slice(body);
        legacy.push(0x00); // trailing flags byte

        let err = decode_record(&legacy).unwrap_err();
        assert!(
            matches!(err, DecodeError::BodyUnderflow { .. }),
            "old-format trailing flags byte must be rejected, got {err:?}"
        );
    }

    #[test]
    fn legacy_pro_with_trailing_flags_byte_rejected() {
        let new_record = CanonicalRecord::Pro(ProRecord {
            memory_id: SymbolId::new(20),
            rule_id: SymbolId::new(21),
            trigger: Value::String("x".into()),
            action: Value::String("y".into()),
            precondition: None,
            scope: SymbolId::new(22),
            source: SymbolId::new(23),
            confidence: Confidence::ONE,
            clocks: clocks(),
        });
        let mut new_bytes = Vec::new();
        encode_record(&new_record, &mut new_bytes);

        let opcode = new_bytes[0];
        let mut cursor = 1;
        let body_len = decode_varint(&new_bytes, &mut cursor).unwrap();
        let body = &new_bytes[cursor..cursor + usize::try_from(body_len).unwrap()];
        let mut legacy = Vec::new();
        legacy.push(opcode);
        encode_varint(body_len + 1, &mut legacy);
        legacy.extend_from_slice(body);
        legacy.push(0x00);

        let err = decode_record(&legacy).unwrap_err();
        assert!(
            matches!(err, DecodeError::BodyUnderflow { .. }),
            "old-format trailing flags byte must be rejected, got {err:?}"
        );
    }

    // ---- Security F2 (P2) regression: decoder must not OOM on a
    // crafted record with an attacker-controlled count varint. The
    // pre-fix decoder did `Vec::with_capacity(count)` *before* the
    // decode loop ran, so an attacker who set `count` near
    // `usize::MAX` (10-byte varint) triggered a multi-exabyte
    // allocation that aborts the process. Post-fix: cap is bounded
    // by remaining body bytes, so the loop returns `Truncated`
    // instead.
    //
    // These tests assert the post-fix behaviour: the function returns
    // an `Err` (any variant — `Truncated`, `LengthMismatch`, or
    // `VarintOverflow` are all acceptable) and crucially **does not
    // panic or abort**. Test execution itself proves the OOM is
    // gone: a pre-fix run would have aborted the test process.

    /// Helper: build a frame with a custom body (after `[opcode][varint
    /// body_len]`) for the given opcode. Bypasses [`encode_record`] so
    /// we can craft adversarial bodies the encoder would never produce.
    fn frame(opcode: Opcode, body: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(body.len() + 11);
        out.push(opcode as u8);
        encode_varint(body.len() as u64, &mut out);
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn decode_epi_does_not_oom_on_huge_participant_count() {
        // EpiRecord body shape: memory_id (varint) + event_id (varint) +
        // kind (varint) + participants_count (varint) + participants...
        // We craft a body where the count varint encodes a huge
        // value but no participants follow.
        let mut body = Vec::new();
        encode_varint(1, &mut body); // memory_id
        encode_varint(2, &mut body); // event_id
        encode_varint(3, &mut body); // kind
        encode_varint(u64::MAX, &mut body); // participants_count = u64::MAX
                                            // No participants follow — `decode_symbol` must hit truncation
                                            // immediately.
        let frame = frame(Opcode::Epi, &body);
        let err = decode_record(&frame).expect_err("must reject huge count");
        // The exact variant depends on which check trips first; the
        // load-bearing assertion is that the decoder returns an error
        // instead of OOMing. VarintOverflow on usize::try_from is the
        // most likely path on 64-bit (u64::MAX > usize::MAX is false on
        // 64-bit so we get past that and hit Truncated on the first
        // missing participant); on 32-bit, VarintOverflow on the
        // try_from. Both are acceptable — we just must not panic.
        assert!(
            matches!(
                err,
                DecodeError::Truncated { .. }
                    | DecodeError::VarintOverflow { .. }
                    | DecodeError::BodyUnderflow { .. }
            ),
            "expected typed error, got {err:?}"
        );
    }

    #[test]
    fn decode_inf_does_not_oom_on_huge_derived_from_count() {
        // InfRecord body: memory_id + s + p + o (value) + count + parents...
        let mut body = Vec::new();
        encode_varint(1, &mut body); // memory_id
        encode_varint(2, &mut body); // s
        encode_varint(3, &mut body); // p
                                     // Encode a Symbol value (tag 0x01 + symbol varint) for `o`.
        body.push(0x01);
        encode_varint(4, &mut body);
        encode_varint(u64::MAX, &mut body); // derived_from count
        let frame = frame(Opcode::Inf, &body);
        let err = decode_record(&frame).expect_err("must reject huge count");
        assert!(
            matches!(
                err,
                DecodeError::Truncated { .. }
                    | DecodeError::VarintOverflow { .. }
                    | DecodeError::BodyUnderflow { .. }
            ),
            "expected typed error, got {err:?}"
        );
    }

    #[test]
    fn decode_episode_meta_does_not_oom_on_huge_retracts_count() {
        // EpisodeMeta body: episode_id + at(clocktime) + label_len +
        // label_bytes + parent_tag + [parent?] + retracts_len + retracts...
        let mut body = Vec::new();
        encode_varint(1, &mut body); // episode_id
                                     // ClockTime: fixed-LE u64 (8 bytes).
        body.extend_from_slice(&1_700_000_000_000_u64.to_le_bytes());
        encode_varint(0, &mut body); // label_len = 0 (empty label)
        body.push(0x00); // parent_tag = None
        encode_varint(u64::MAX, &mut body); // retracts_len = u64::MAX
        let frame = frame(Opcode::EpisodeMeta, &body);
        let err = decode_record(&frame).expect_err("must reject huge count");
        assert!(
            matches!(
                err,
                DecodeError::Truncated { .. }
                    | DecodeError::VarintOverflow { .. }
                    | DecodeError::BodyUnderflow { .. }
            ),
            "expected typed error, got {err:?}"
        );
    }
}
