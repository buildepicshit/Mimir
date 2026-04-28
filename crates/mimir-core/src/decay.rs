//! Deterministic confidence-decay model per
//! `docs/concepts/confidence-decay.md`.
//!
//! Exponential decay: `effective = stored × 2^(-elapsed/half_life)`,
//! computed in integer fixed-point at the `u16` confidence resolution
//! (spec § 5.1 formula, § 13 invariant 2 bit-identity across
//! architectures). A 256-entry lookup table `DECAY_TABLE` covers the
//! fractional exponent with 8 bits of precision; the integer-exponent
//! part is a right-shift. The lookup table values are baked into
//! source (offline-computed `round(2^(-i/256) × 65535)`) so every
//! build produces identical bytes.
//!
//! Surface area:
//!
//! - Core [`effective_confidence`] for non-Procedural, non-Inferential
//!   memories — time decay against a per-`(memory_kind, source_kind)`
//!   half-life.
//! - Pinned / authoritative short-circuit (spec § 13 invariant 3).
//! - v1 default parameter table (spec § 5.2) exposed as
//!   [`DecayConfig::librarian_defaults`]. All fields are `pub` for
//!   runtime user override (spec § 13 invariant 5).
//! - `mimir.toml`-shaped overrides via
//!   [`DecayConfig::from_toml`] / [`DecayConfig::apply_toml`] (spec
//!   § 1 graduation criterion #4) — days-valued integers; `0` encodes
//!   the § 5.3 infinity sentinel; unlisted keys fall back to
//!   librarian defaults; unknown keys are silently ignored for
//!   forward-compatibility.
//!
//! Deferred: Procedural activity weighting (§ 6), Inferential parent-
//! tracking (§ 9 — composes with `inference_methods::InferenceMethod`
//! by the caller).

use thiserror::Error;

use crate::confidence::Confidence;
use crate::memory_kind::MemoryKindTag;
use crate::source_kind::SourceKind;

// -------------------------------------------------------------------
// Lookup table
// -------------------------------------------------------------------

/// `DECAY_TABLE[i] = round(2^(-i/256) × 65535)` for `i` in `[0, 256)`.
/// Baked into source for bit-identical bytes across builds and
/// architectures — any regeneration must produce identical values or
/// the spec § 13 invariant 2 is violated.
#[rustfmt::skip]
const DECAY_TABLE: [u16; 256] = [
    65535, 65358, 65181, 65005, 64829, 64654, 64479, 64305,
    64131, 63957, 63784, 63612, 63440, 63268, 63097, 62927,
    62757, 62587, 62418, 62249, 62081, 61913, 61745, 61578,
    61412, 61246, 61080, 60915, 60750, 60586, 60422, 60259,
    60096, 59933, 59771, 59610, 59449, 59288, 59127, 58968,
    58808, 58649, 58491, 58332, 58175, 58017, 57860, 57704,
    57548, 57392, 57237, 57082, 56928, 56774, 56621, 56468,
    56315, 56163, 56011, 55859, 55708, 55558, 55407, 55258,
    55108, 54959, 54811, 54662, 54515, 54367, 54220, 54074,
    53927, 53781, 53636, 53491, 53346, 53202, 53058, 52915,
    52772, 52629, 52487, 52345, 52203, 52062, 51921, 51781,
    51641, 51501, 51362, 51223, 51085, 50947, 50809, 50671,
    50534, 50398, 50261, 50126, 49990, 49855, 49720, 49586,
    49452, 49318, 49184, 49051, 48919, 48787, 48655, 48523,
    48392, 48261, 48131, 48000, 47871, 47741, 47612, 47483,
    47355, 47227, 47099, 46972, 46845, 46718, 46592, 46466,
    46340, 46215, 46090, 45965, 45841, 45717, 45593, 45470,
    45347, 45225, 45102, 44980, 44859, 44737, 44617, 44496,
    44376, 44256, 44136, 44017, 43898, 43779, 43660, 43542,
    43425, 43307, 43190, 43073, 42957, 42841, 42725, 42609,
    42494, 42379, 42265, 42150, 42036, 41923, 41809, 41696,
    41584, 41471, 41359, 41247, 41136, 41024, 40914, 40803,
    40693, 40583, 40473, 40363, 40254, 40145, 40037, 39929,
    39821, 39713, 39606, 39498, 39392, 39285, 39179, 39073,
    38967, 38862, 38757, 38652, 38548, 38443, 38339, 38236,
    38132, 38029, 37926, 37824, 37722, 37620, 37518, 37416,
    37315, 37214, 37114, 37013, 36913, 36813, 36714, 36615,
    36516, 36417, 36318, 36220, 36122, 36025, 35927, 35830,
    35733, 35637, 35540, 35444, 35348, 35253, 35157, 35062,
    34968, 34873, 34779, 34685, 34591, 34497, 34404, 34311,
    34218, 34126, 34033, 33941, 33850, 33758, 33667, 33576,
    33485, 33394, 33304, 33214, 33124, 33035, 32945, 32856,
];

// -------------------------------------------------------------------
// Constants
// -------------------------------------------------------------------

/// One day in milliseconds — used for documentation and for converting
/// the spec's day-valued half-lives into `u64` millis.
pub const DAY_MS: u64 = 86_400_000;

/// Sentinel for "no time decay" — the spec § 5.3 infinity encoding.
/// [`effective_confidence`] treats this as `decay_factor = 1`.
pub const NO_DECAY: u64 = 0;

/// A per-class half-life, stored as milliseconds but constructed
/// explicitly in days or millis.
///
/// The agent-facing `mimir.toml` loader and the librarian default
/// table both deal in days; internal decay math is in milliseconds.
/// The newtype keeps the unit explicit at the boundary so a future
/// caller can't accidentally drop a `ClockTime` into a half-life
/// slot (or vice versa).
///
/// The infinity case (`HalfLife::no_decay()` == `HalfLife::ZERO`)
/// maps to the spec § 5.3 encoding — `effective_confidence` returns
/// stored for any memory whose class has `HalfLife::ZERO`.
///
/// # Examples
///
/// ```
/// use mimir_core::decay::{HalfLife, DAY_MS};
/// assert_eq!(HalfLife::from_days(180).as_millis(), 180 * DAY_MS);
/// assert_eq!(HalfLife::from_millis(500).as_millis(), 500);
/// assert!(HalfLife::no_decay().is_no_decay());
/// assert!(!HalfLife::from_days(1).is_no_decay());
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct HalfLife(u64);

impl HalfLife {
    /// The spec § 5.3 infinity case — `decay_factor = 1` always.
    pub const ZERO: Self = Self(NO_DECAY);

    /// Construct from a day count.
    #[must_use]
    pub const fn from_days(days: u64) -> Self {
        Self(days.saturating_mul(DAY_MS))
    }

    /// Construct from a raw millisecond count. `0` encodes
    /// [`Self::no_decay`].
    #[must_use]
    pub const fn from_millis(millis: u64) -> Self {
        Self(millis)
    }

    /// The spec § 5.3 "no time decay" sentinel.
    #[must_use]
    pub const fn no_decay() -> Self {
        Self(NO_DECAY)
    }

    /// Raw millisecond representation — for the internal decay
    /// math. Unit-explicit callers should prefer
    /// `HalfLife`-typed values everywhere else.
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.0
    }

    /// `true` when this half-life encodes the spec § 5.3 infinity
    /// case.
    #[must_use]
    pub const fn is_no_decay(self) -> bool {
        self.0 == NO_DECAY
    }
}

// Max exponent bits before the fractional result underflows u16.
// `decay = frac >> n`; for frac ≤ 65535, `n ≥ 16` produces 0.
const MAX_EXPONENT: u32 = 16;

// Cap `elapsed_ms` so `elapsed_ms * 256` cannot overflow u64.
const ELAPSED_CAP: u64 = u64::MAX / 256;

// -------------------------------------------------------------------
// Core decay primitive
// -------------------------------------------------------------------

/// Deterministic integer decay factor in `u16` fixed-point scale.
///
/// Returns `u16::MAX` (representing 1.0) when `half_life` is
/// [`HalfLife::no_decay`], `0` when `elapsed_ms` saturates the
/// exponent beyond representable precision, and a
/// monotonically-decreasing value in between.
///
/// # Example
///
/// ```
/// use mimir_core::decay::{decay_factor_u16, HalfLife, DAY_MS};
/// // Zero elapsed → full factor (u16::MAX).
/// assert_eq!(decay_factor_u16(0, HalfLife::from_days(180)), u16::MAX);
/// // One half-life elapsed → ≈ 0.5 (u16::MAX / 2, ±1 ULP).
/// let half = decay_factor_u16(180 * DAY_MS, HalfLife::from_days(180));
/// assert!(half.abs_diff(u16::MAX / 2) <= 1);
/// // Infinite half-life → no decay.
/// assert_eq!(
///     decay_factor_u16(1_000 * DAY_MS, HalfLife::no_decay()),
///     u16::MAX,
/// );
/// ```
#[must_use]
pub fn decay_factor_u16(elapsed_ms: u64, half_life: HalfLife) -> u16 {
    let half_life_ms = half_life.as_millis();
    if half_life_ms == NO_DECAY {
        return u16::MAX;
    }
    let elapsed = elapsed_ms.min(ELAPSED_CAP);
    let k_q8 = (elapsed.saturating_mul(256)) / half_life_ms;
    #[allow(clippy::cast_possible_truncation)]
    let n = (k_q8 >> 8) as u32;
    if n >= MAX_EXPONENT {
        return 0;
    }
    let i = (k_q8 & 0xFF) as usize;
    let frac = u32::from(DECAY_TABLE[i]);
    #[allow(clippy::cast_possible_truncation)]
    let result = (frac >> n) as u16;
    result
}

// -------------------------------------------------------------------
// Decay flags (pinning / authoritative)
// -------------------------------------------------------------------

/// Runtime flags that suspend decay per spec §§ 7–8.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct DecayFlags {
    /// Agent-invokable pin — `effective = stored`.
    pub pinned: bool,
    /// Operator-declared authoritative — `effective = stored` and
    /// distinct surfacing semantics at read time (see
    /// `read-protocol.md` amendment).
    pub authoritative: bool,
}

impl DecayFlags {
    /// `true` if either flag suspends decay.
    #[must_use]
    pub const fn suspends_decay(self) -> bool {
        self.pinned || self.authoritative
    }
}

// -------------------------------------------------------------------
// DecayConfig
// -------------------------------------------------------------------

/// Per-`(memory-kind, source-kind)` decay parameter table with the
/// v1 librarian defaults from spec § 5.2. Fields are `pub` so callers
/// can override at runtime without restart (spec § 13 invariant 5);
/// [`apply_toml`](Self::apply_toml) reads `mimir.toml`-shaped
/// overrides in days.
///
/// Half-lives are stored in **milliseconds**; [`NO_DECAY`] encodes the
/// spec § 5.3 infinity case (`librarian_assignment`, Procedural
/// time-decay).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecayConfig {
    /// Semantic × `@profile`.
    pub sem_profile: HalfLife,
    /// Semantic × `@observation`.
    pub sem_observation: HalfLife,
    /// Semantic × `@self_report`.
    pub sem_self_report: HalfLife,
    /// Semantic × `@participant_report` (no explicit default — mirrors
    /// `@self_report` for unlisted pairs).
    pub sem_participant_report: HalfLife,
    /// Semantic × `@document`.
    pub sem_document: HalfLife,
    /// Semantic × `@registry`.
    pub sem_registry: HalfLife,
    /// Semantic × `@policy` (no explicit default; mirrors
    /// `@agent_instruction`).
    pub sem_policy: HalfLife,
    /// Semantic × `@external_authority`.
    pub sem_external_authority: HalfLife,
    /// Semantic × `@agent_instruction`.
    pub sem_agent_instruction: HalfLife,
    /// Semantic × `@librarian_assignment` — no decay.
    pub sem_librarian_assignment: HalfLife,
    /// Semantic × `@pending_verification`.
    pub sem_pending_verification: HalfLife,
    /// Episodic × `@observation`.
    pub epi_observation: HalfLife,
    /// Episodic × `@self_report`.
    pub epi_self_report: HalfLife,
    /// Episodic × `@participant_report`.
    pub epi_participant_report: HalfLife,
    /// Procedural — any source. [`HalfLife::no_decay`] (spec § 6
    /// activity-weighted instead — not implemented in 5.8).
    pub pro_any: HalfLife,
}

impl DecayConfig {
    /// v1 default parameters per spec § 5.2. User overrides happen by
    /// mutating the struct in-place.
    #[must_use]
    pub const fn librarian_defaults() -> Self {
        Self {
            sem_profile: HalfLife::from_days(730),
            sem_observation: HalfLife::from_days(180),
            sem_self_report: HalfLife::from_days(90),
            sem_participant_report: HalfLife::from_days(90),
            sem_document: HalfLife::from_days(365),
            sem_registry: HalfLife::from_days(90),
            sem_policy: HalfLife::from_days(730),
            sem_external_authority: HalfLife::from_days(180),
            sem_agent_instruction: HalfLife::from_days(730),
            sem_librarian_assignment: HalfLife::no_decay(),
            sem_pending_verification: HalfLife::from_days(30),
            epi_observation: HalfLife::from_days(90),
            epi_self_report: HalfLife::from_days(30),
            epi_participant_report: HalfLife::from_days(60),
            pro_any: HalfLife::no_decay(),
        }
    }

    /// Look up the half-life for a given memory kind / source kind
    /// pair. Returns `None` for pairs that `SourceKind::admits`
    /// rejects (the caller should validate upstream) or for
    /// Inferential memories, which decay via their parents rather than
    /// a per-pair half-life.
    #[must_use]
    #[allow(clippy::match_same_arms)]
    pub const fn half_life_for(
        &self,
        memory_kind: MemoryKindTag,
        source_kind: SourceKind,
    ) -> Option<HalfLife> {
        match (memory_kind, source_kind) {
            (MemoryKindTag::Semantic, SourceKind::Profile) => Some(self.sem_profile),
            (MemoryKindTag::Semantic, SourceKind::Observation) => Some(self.sem_observation),
            (MemoryKindTag::Semantic, SourceKind::SelfReport) => Some(self.sem_self_report),
            (MemoryKindTag::Semantic, SourceKind::ParticipantReport) => {
                Some(self.sem_participant_report)
            }
            (MemoryKindTag::Semantic, SourceKind::Document) => Some(self.sem_document),
            (MemoryKindTag::Semantic, SourceKind::Registry) => Some(self.sem_registry),
            (MemoryKindTag::Semantic, SourceKind::Policy) => Some(self.sem_policy),
            (MemoryKindTag::Semantic, SourceKind::ExternalAuthority) => {
                Some(self.sem_external_authority)
            }
            (MemoryKindTag::Semantic, SourceKind::AgentInstruction) => {
                Some(self.sem_agent_instruction)
            }
            (MemoryKindTag::Semantic, SourceKind::LibrarianAssignment) => {
                Some(self.sem_librarian_assignment)
            }
            (MemoryKindTag::Semantic, SourceKind::PendingVerification) => {
                Some(self.sem_pending_verification)
            }
            (MemoryKindTag::Episodic, SourceKind::Observation) => Some(self.epi_observation),
            (MemoryKindTag::Episodic, SourceKind::SelfReport) => Some(self.epi_self_report),
            (MemoryKindTag::Episodic, SourceKind::ParticipantReport) => {
                Some(self.epi_participant_report)
            }
            (MemoryKindTag::Procedural, _) => Some(self.pro_any),
            // Inferential decays via parents (spec § 9) — caller must
            // recompute from current parent effective confidences.
            (MemoryKindTag::Inferential, _) => None,
            // Episodic paired with a non-Episodic-admitted source —
            // validation should have rejected upstream; we fall through
            // to None so the caller's bug path is explicit.
            (MemoryKindTag::Episodic, _) => None,
        }
    }
}

impl Default for DecayConfig {
    fn default() -> Self {
        Self::librarian_defaults()
    }
}

// -------------------------------------------------------------------
// TOML loading (spec § 5.2, § 13 invariant 5, graduation criterion #4)
// -------------------------------------------------------------------

/// Errors produced by [`DecayConfig::apply_toml`] and
/// [`DecayConfig::from_toml`].
#[derive(Debug, Error)]
pub enum DecayConfigError {
    /// The TOML input failed to parse. Wraps `toml::de::Error`
    /// directly so callers can route on its structured
    /// `span()` / `message()` without re-parsing a string.
    #[error("toml parse error: {0}")]
    Parse(#[from] toml::de::Error),
    /// A section was the wrong TOML value type (e.g. `decay` as an
    /// integer instead of a table).
    #[error("{path}: expected table")]
    ExpectedTable {
        /// Dotted path to the offending key.
        path: &'static str,
    },
    /// A leaf value was the wrong type — half-lives must be
    /// non-negative integers (days).
    #[error("{path}: expected non-negative integer (days)")]
    ExpectedNonNegInteger {
        /// Dotted path to the offending key.
        path: &'static str,
    },
    /// A recognized key carried an unknown value (e.g. a negative
    /// integer, or a floating-point where integer was expected).
    #[error("{path}: value {value} is not a valid half-life (days ≥ 0)")]
    InvalidDays {
        /// Dotted path to the offending key.
        path: &'static str,
        /// The offending value as it appeared in the TOML.
        value: i64,
    },
}

impl DecayConfig {
    /// Parse `mimir.toml`-shaped overrides on top of the v1 librarian
    /// defaults (spec § 5.2). Accepted TOML shape:
    ///
    /// ```toml
    /// [decay.semantic]
    /// profile = 730            # days; `0` encodes NO_DECAY (spec § 5.3)
    /// observation = 180
    /// # any of the other SourceKind keys under the `[decay.semantic]`
    /// # table; unlisted keys fall back to librarian defaults
    ///
    /// [decay.episodic]
    /// observation = 90
    /// self_report = 30
    /// participant_report = 60
    ///
    /// [decay.procedural]
    /// any = 0                  # v1 uses activity weighting instead
    /// ```
    ///
    /// Unknown TOML keys are silently ignored per the spec's
    /// "unlisted keys fall back" convention; the v1 goal is
    /// tolerance of future extensions.
    ///
    /// # Errors
    ///
    /// - [`DecayConfigError::Parse`] if the TOML doesn't parse.
    /// - [`DecayConfigError::ExpectedTable`] if `decay` or a known
    ///   subsection is not a TOML table.
    /// - [`DecayConfigError::ExpectedNonNegInteger`] if a recognized
    ///   leaf key is not an integer.
    /// - [`DecayConfigError::InvalidDays`] if the integer is negative.
    pub fn from_toml(toml_str: &str) -> Result<Self, DecayConfigError> {
        let mut cfg = Self::librarian_defaults();
        cfg.apply_toml(toml_str)?;
        Ok(cfg)
    }

    /// Apply TOML-encoded overrides in place. See [`from_toml`](Self::from_toml)
    /// for the accepted schema.
    ///
    /// # Errors
    ///
    /// See [`from_toml`](Self::from_toml).
    pub fn apply_toml(&mut self, toml_str: &str) -> Result<(), DecayConfigError> {
        let root: toml::Table = toml_str.parse()?;
        let Some(decay) = root.get("decay") else {
            return Ok(());
        };
        let toml::Value::Table(decay) = decay else {
            return Err(DecayConfigError::ExpectedTable { path: "decay" });
        };
        if let Some(section) = decay.get("semantic") {
            let toml::Value::Table(sem) = section else {
                return Err(DecayConfigError::ExpectedTable {
                    path: "decay.semantic",
                });
            };
            apply_section(
                sem,
                "decay.semantic",
                &mut [
                    ("profile", &mut self.sem_profile),
                    ("observation", &mut self.sem_observation),
                    ("self_report", &mut self.sem_self_report),
                    ("participant_report", &mut self.sem_participant_report),
                    ("document", &mut self.sem_document),
                    ("registry", &mut self.sem_registry),
                    ("policy", &mut self.sem_policy),
                    ("external_authority", &mut self.sem_external_authority),
                    ("agent_instruction", &mut self.sem_agent_instruction),
                    ("librarian_assignment", &mut self.sem_librarian_assignment),
                    ("pending_verification", &mut self.sem_pending_verification),
                ],
            )?;
        }
        if let Some(section) = decay.get("episodic") {
            let toml::Value::Table(epi) = section else {
                return Err(DecayConfigError::ExpectedTable {
                    path: "decay.episodic",
                });
            };
            apply_section(
                epi,
                "decay.episodic",
                &mut [
                    ("observation", &mut self.epi_observation),
                    ("self_report", &mut self.epi_self_report),
                    ("participant_report", &mut self.epi_participant_report),
                ],
            )?;
        }
        if let Some(section) = decay.get("procedural") {
            let toml::Value::Table(pro) = section else {
                return Err(DecayConfigError::ExpectedTable {
                    path: "decay.procedural",
                });
            };
            apply_section(pro, "decay.procedural", &mut [("any", &mut self.pro_any)])?;
        }
        Ok(())
    }
}

/// Apply a set of `(key, &mut HalfLife)` slots against a TOML
/// subsection. Values are read as integers (days) and wrapped in a
/// `HalfLife::from_days`; `0` encodes [`HalfLife::no_decay`].
fn apply_section(
    section: &toml::Table,
    section_path: &'static str,
    slots: &mut [(&'static str, &mut HalfLife)],
) -> Result<(), DecayConfigError> {
    for (key, slot) in slots {
        let Some(value) = section.get(*key) else {
            continue;
        };
        let toml::Value::Integer(days) = value else {
            // Build a dotted path — but we can't easily allocate a
            // &'static str per (section, key) pair without leaking or
            // pulling in a string-ID map. Use the section path; the
            // caller gets a close-enough pointer.
            return Err(DecayConfigError::ExpectedNonNegInteger { path: section_path });
        };
        if *days < 0 {
            return Err(DecayConfigError::InvalidDays {
                path: section_path,
                value: *days,
            });
        }
        #[allow(clippy::cast_sign_loss)]
        let days_u64 = *days as u64;
        **slot = HalfLife::from_days(days_u64);
    }
    Ok(())
}

// -------------------------------------------------------------------
// Effective-confidence computation
// -------------------------------------------------------------------

/// Compute the effective confidence for a non-Inferential memory at
/// read time. Pinned or operator-authoritative memories short-circuit
/// to `stored` (spec § 13 invariant 3).
///
/// `elapsed_ms` is `now - valid_at` for Semantic/Inferential or
/// `now - committed_at` for Procedural (where `valid_at == committed_at`)
/// — the caller supplies the already-differenced value so this function
/// doesn't reach into `ClockTime`.
///
/// # Example
///
/// ```
/// use mimir_core::decay::{effective_confidence, DecayConfig, DecayFlags, DAY_MS};
/// use mimir_core::{Confidence, MemoryKindTag, SourceKind};
/// let cfg = DecayConfig::librarian_defaults();
/// let stored = Confidence::try_from_f32(0.8).unwrap();
/// let one_hl = 180 * DAY_MS; // one Semantic@observation half-life
/// let effective = effective_confidence(
///     stored,
///     one_hl,
///     MemoryKindTag::Semantic,
///     SourceKind::Observation,
///     DecayFlags::default(),
///     &cfg,
/// );
/// // After one half-life, effective is approximately half of stored.
/// let target = stored.as_f32() * 0.5;
/// assert!((effective.as_f32() - target).abs() < 0.01);
/// ```
#[must_use]
pub fn effective_confidence(
    stored: Confidence,
    elapsed_ms: u64,
    memory_kind: MemoryKindTag,
    source_kind: SourceKind,
    flags: DecayFlags,
    config: &DecayConfig,
) -> Confidence {
    if flags.suspends_decay() {
        return stored;
    }
    // Inferential: caller is responsible for supplying the effective
    // confidence derived from its parents. If Inferential reaches this
    // function by accident, decay is a no-op rather than crashing —
    // the caller's test is expected to catch the misuse.
    let Some(half_life) = config.half_life_for(memory_kind, source_kind) else {
        return stored;
    };
    let factor = decay_factor_u16(elapsed_ms, half_life);
    let product = u32::from(stored.as_u16()) * u32::from(factor);
    // round-to-nearest division by u16::MAX.
    let scaled = (product + u32::from(u16::MAX) / 2) / u32::from(u16::MAX);
    #[allow(clippy::cast_possible_truncation)]
    Confidence::from_u16(scaled as u16)
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn c(f: f32) -> Confidence {
        Confidence::try_from_f32(f).expect("in range")
    }

    // ----- Table + decay_factor_u16 -----

    #[test]
    fn table_first_entry_is_unit_factor() {
        assert_eq!(DECAY_TABLE[0], u16::MAX);
    }

    #[test]
    fn table_is_strictly_monotonically_decreasing() {
        for i in 1..256 {
            assert!(
                DECAY_TABLE[i] < DECAY_TABLE[i - 1],
                "non-monotonic at index {i}: {} >= {}",
                DECAY_TABLE[i],
                DECAY_TABLE[i - 1]
            );
        }
    }

    #[test]
    fn no_decay_half_life_returns_unit() {
        assert_eq!(decay_factor_u16(1_000_000, HalfLife::no_decay()), u16::MAX);
        assert_eq!(decay_factor_u16(u64::MAX, HalfLife::no_decay()), u16::MAX);
    }

    #[test]
    fn zero_elapsed_returns_unit() {
        assert_eq!(decay_factor_u16(0, HalfLife::from_days(180)), u16::MAX);
    }

    #[test]
    fn one_half_life_returns_approximately_half() {
        let factor = decay_factor_u16(180 * DAY_MS, HalfLife::from_days(180));
        assert!(factor.abs_diff(u16::MAX / 2) <= 1);
    }

    #[test]
    fn sixteen_half_lives_saturate_to_zero() {
        // 2^(-16) × u16::MAX ≈ 1.0, but our table's integer
        // right-shift produces 0 at n == 16.
        let factor = decay_factor_u16(16 * 180 * DAY_MS, HalfLife::from_days(180));
        assert_eq!(factor, 0);
    }

    #[test]
    fn elapsed_near_u64_max_saturates_to_zero_not_panics() {
        // Exercises the `ELAPSED_CAP` saturating_mul guard — without
        // the cap, `elapsed.saturating_mul(256)` in `decay_factor_u16`
        // could overflow the u64 intermediate. Result must be 0
        // (fully decayed), not a panic or wrapped value.
        let factor = decay_factor_u16(u64::MAX, HalfLife::from_millis(1));
        assert_eq!(factor, 0);
        // Sentinel-adjacent values land in the same regime.
        let factor = decay_factor_u16(u64::MAX - 1, HalfLife::from_days(180));
        assert_eq!(factor, 0);
    }

    #[test]
    fn half_life_of_one_millisecond_is_the_tightest_divisor() {
        // Smallest legal half-life — no overflow, no divide-by-zero,
        // result saturates to 0 once elapsed crosses 16 half-lives.
        let one_ms = HalfLife::from_millis(1);
        assert_eq!(decay_factor_u16(0, one_ms), u16::MAX);
        // One half-life elapsed at this boundary: DECAY_TABLE[0] >> 1.
        assert_eq!(decay_factor_u16(1, one_ms), u16::MAX >> 1);
        // 16 ms is 16 half-lives; hits the MAX_EXPONENT saturation.
        assert_eq!(decay_factor_u16(16, one_ms), 0);
    }

    #[test]
    fn decay_is_monotonic_in_elapsed() {
        // For a fixed half-life, longer elapsed → smaller or equal
        // factor. Sampled check (property test elsewhere).
        let hl = HalfLife::from_days(180);
        let mut prev = u16::MAX;
        for days in (0_u64..=1800).step_by(7) {
            let f = decay_factor_u16(days * DAY_MS, hl);
            assert!(f <= prev, "non-monotonic at day {days}");
            prev = f;
        }
    }

    // ----- effective_confidence -----

    #[test]
    fn pinned_short_circuits_to_stored() {
        let cfg = DecayConfig::librarian_defaults();
        let stored = c(0.8);
        let eff = effective_confidence(
            stored,
            10 * 365 * DAY_MS,
            MemoryKindTag::Semantic,
            SourceKind::Observation,
            DecayFlags {
                pinned: true,
                authoritative: false,
            },
            &cfg,
        );
        assert_eq!(eff, stored);
    }

    #[test]
    fn authoritative_short_circuits_to_stored() {
        let cfg = DecayConfig::librarian_defaults();
        let stored = c(0.8);
        let eff = effective_confidence(
            stored,
            10 * 365 * DAY_MS,
            MemoryKindTag::Semantic,
            SourceKind::Observation,
            DecayFlags {
                pinned: false,
                authoritative: true,
            },
            &cfg,
        );
        assert_eq!(eff, stored);
    }

    #[test]
    fn librarian_assignment_never_decays() {
        let cfg = DecayConfig::librarian_defaults();
        let stored = c(1.0);
        let eff = effective_confidence(
            stored,
            100 * 365 * DAY_MS,
            MemoryKindTag::Semantic,
            SourceKind::LibrarianAssignment,
            DecayFlags::default(),
            &cfg,
        );
        assert_eq!(eff, stored);
    }

    #[test]
    fn procedural_time_decay_is_disabled() {
        // Procedural uses activity weighting (§ 6) not time decay;
        // v1 stores `NO_DECAY` for the half-life.
        let cfg = DecayConfig::librarian_defaults();
        let stored = c(0.9);
        let eff = effective_confidence(
            stored,
            10 * 365 * DAY_MS,
            MemoryKindTag::Procedural,
            SourceKind::AgentInstruction,
            DecayFlags::default(),
            &cfg,
        );
        assert_eq!(eff, stored);
    }

    #[test]
    fn inferential_is_passthrough_at_this_layer() {
        // Inferential doesn't time-decay here — it's recomputed from
        // parent effective confidences by the caller. If it reaches
        // this function, we return stored unchanged rather than
        // crashing (defensive).
        let cfg = DecayConfig::librarian_defaults();
        let stored = c(0.7);
        let eff = effective_confidence(
            stored,
            10 * 365 * DAY_MS,
            MemoryKindTag::Inferential,
            SourceKind::Observation,
            DecayFlags::default(),
            &cfg,
        );
        assert_eq!(eff, stored);
    }

    #[test]
    fn one_half_life_halves_stored_confidence() {
        let cfg = DecayConfig::librarian_defaults();
        let stored = c(0.8);
        let eff = effective_confidence(
            stored,
            180 * DAY_MS,
            MemoryKindTag::Semantic,
            SourceKind::Observation,
            DecayFlags::default(),
            &cfg,
        );
        // ±1 fixed-point step of drift acceptable.
        let target = i32::from(stored.as_u16()) / 2;
        let actual = i32::from(eff.as_u16());
        assert!(
            (actual - target).abs() <= 1,
            "expected ≈{target}, got {actual}"
        );
    }

    #[test]
    fn defaults_match_spec_table() {
        let cfg = DecayConfig::librarian_defaults();
        // Spot-check the three distinctive rows: profile (730d),
        // pending_verification (30d), librarian_assignment (∞).
        assert_eq!(cfg.sem_profile, HalfLife::from_days(730));
        assert_eq!(cfg.sem_pending_verification, HalfLife::from_days(30));
        assert_eq!(cfg.sem_librarian_assignment, HalfLife::no_decay());
        assert_eq!(cfg.epi_self_report, HalfLife::from_days(30));
        assert_eq!(cfg.pro_any, HalfLife::no_decay());
    }

    // ----- TOML loader -----

    #[test]
    fn toml_empty_input_preserves_defaults() {
        let cfg = DecayConfig::from_toml("").expect("parse");
        assert_eq!(cfg, DecayConfig::librarian_defaults());
    }

    #[test]
    fn toml_overrides_semantic_half_lives() {
        let toml = r"
            [decay.semantic]
            profile = 30
            observation = 365
        ";
        let cfg = DecayConfig::from_toml(toml).expect("parse");
        assert_eq!(cfg.sem_profile, HalfLife::from_days(30));
        assert_eq!(cfg.sem_observation, HalfLife::from_days(365));
        // Non-overridden keys preserved.
        assert_eq!(cfg.sem_document, HalfLife::from_days(365)); // default
    }

    #[test]
    fn toml_zero_encodes_no_decay() {
        // Spec § 5.3 — 0 in mimir.toml = ∞ (NO_DECAY internally).
        let toml = r"
            [decay.semantic]
            librarian_assignment = 0
            profile = 0
        ";
        let cfg = DecayConfig::from_toml(toml).expect("parse");
        assert_eq!(cfg.sem_librarian_assignment, HalfLife::no_decay());
        assert_eq!(cfg.sem_profile, HalfLife::no_decay());
    }

    #[test]
    fn toml_unknown_keys_are_ignored() {
        let toml = r"
            [decay.semantic]
            profile = 30
            future_source_kind = 42  # not in the v1 registry — must be ignored

            [decay.not_a_real_section]
            key = 1
        ";
        let cfg = DecayConfig::from_toml(toml).expect("parse");
        assert_eq!(cfg.sem_profile, HalfLife::from_days(30));
    }

    #[test]
    fn toml_rejects_negative_days() {
        let toml = r"
            [decay.semantic]
            profile = -1
        ";
        let err = DecayConfig::from_toml(toml).expect_err("negative");
        assert!(matches!(err, DecayConfigError::InvalidDays { .. }));
    }

    #[test]
    fn toml_rejects_non_integer_values() {
        let toml = r#"
            [decay.semantic]
            profile = "thirty"
        "#;
        let err = DecayConfig::from_toml(toml).expect_err("string");
        assert!(matches!(
            err,
            DecayConfigError::ExpectedNonNegInteger { .. }
        ));
    }

    #[test]
    fn toml_rejects_wrong_section_type() {
        let toml = r"
            decay = 42
        ";
        let err = DecayConfig::from_toml(toml).expect_err("not a table");
        assert!(matches!(
            err,
            DecayConfigError::ExpectedTable { path: "decay" }
        ));
    }

    #[test]
    fn toml_overrides_episodic_and_procedural() {
        let toml = r"
            [decay.episodic]
            observation = 7
            self_report = 3
            participant_report = 14

            [decay.procedural]
            any = 365
        ";
        let cfg = DecayConfig::from_toml(toml).expect("parse");
        assert_eq!(cfg.epi_observation, HalfLife::from_days(7));
        assert_eq!(cfg.epi_self_report, HalfLife::from_days(3));
        assert_eq!(cfg.epi_participant_report, HalfLife::from_days(14));
        assert_eq!(cfg.pro_any, HalfLife::from_days(365));
    }

    #[test]
    fn apply_toml_is_additive() {
        // Multiple calls stack; later values win.
        let mut cfg = DecayConfig::librarian_defaults();
        cfg.apply_toml("[decay.semantic]\nprofile = 30")
            .expect("first");
        assert_eq!(cfg.sem_profile, HalfLife::from_days(30));
        cfg.apply_toml("[decay.semantic]\nobservation = 7")
            .expect("second");
        // First override preserved; second override applied.
        assert_eq!(cfg.sem_profile, HalfLife::from_days(30));
        assert_eq!(cfg.sem_observation, HalfLife::from_days(7));
    }

    #[test]
    fn toml_reload_changes_effective_confidence_without_restart() {
        // Spec § 1 graduation criterion #4 + § 13 invariant 5: user
        // config overrides take effect at runtime.
        let mut cfg = DecayConfig::librarian_defaults();
        let stored = c(1.0);
        let elapsed = 30 * DAY_MS;

        let before = effective_confidence(
            stored,
            elapsed,
            MemoryKindTag::Semantic,
            SourceKind::Observation,
            DecayFlags::default(),
            &cfg,
        );

        // Simulate an mimir.toml reload that shortens the half-life
        // dramatically. The same (stored, elapsed) must now produce a
        // lower effective confidence.
        cfg.apply_toml("[decay.semantic]\nobservation = 1")
            .expect("reload");
        let after = effective_confidence(
            stored,
            elapsed,
            MemoryKindTag::Semantic,
            SourceKind::Observation,
            DecayFlags::default(),
            &cfg,
        );
        assert!(
            after < before,
            "reload did not accelerate decay: before={before:?} after={after:?}"
        );
    }

    #[test]
    fn user_override_takes_effect_at_runtime() {
        // Spec § 13 invariant 5 ("user sovereignty") — overriding a
        // half-life in-memory changes subsequent effective-confidence
        // computations without any re-initialization.
        let mut cfg = DecayConfig::librarian_defaults();
        let stored = c(1.0);
        // Baseline: 180 days Semantic@observation → approximately half.
        let baseline = effective_confidence(
            stored,
            180 * DAY_MS,
            MemoryKindTag::Semantic,
            SourceKind::Observation,
            DecayFlags::default(),
            &cfg,
        );
        // Override the half-life to 90 days — 180 days is now 2 HLs
        // (decay factor ≈ 0.25) rather than 1 HL.
        cfg.sem_observation = HalfLife::from_days(90);
        let overridden = effective_confidence(
            stored,
            180 * DAY_MS,
            MemoryKindTag::Semantic,
            SourceKind::Observation,
            DecayFlags::default(),
            &cfg,
        );
        assert!(
            overridden < baseline,
            "override should accelerate decay; baseline={baseline:?} overridden={overridden:?}"
        );
    }
}
