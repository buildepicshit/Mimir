//! `ClockTime` — UTC milliseconds-since-Unix-epoch newtype used for every
//! Mimir clock. Implements the contract in `docs/concepts/temporal-model.md`
//! § 9.1.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

/// Sentinel value reserved for "no invalidation" in the canonical form
/// (see `docs/concepts/ir-canonical-form.md` § 3.1). This value is an
/// encoding concern — the public API exposes absence via `Option<ClockTime>`.
pub(crate) const NONE_SENTINEL: u64 = u64::MAX;

/// A point in time, in milliseconds since the Unix epoch, UTC.
///
/// Honors `docs/concepts/temporal-model.md` § 9.1:
///
/// - Millisecond precision only in v1.
/// - UTC exclusively — agent-provided times must be in UTC or the grammar
///   rejects.
/// - `u64` capacity reaches well past year 584,000,000.
///
/// # Examples
///
/// ```
/// # #![allow(clippy::unwrap_used)]
/// use mimir_core::ClockTime;
///
/// let t = ClockTime::try_from_millis(1_713_350_400_000).unwrap();
/// assert_eq!(t.as_millis(), 1_713_350_400_000);
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClockTime(u64);

/// Errors returned when constructing or manipulating a [`ClockTime`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ClockTimeError {
    /// The wall clock appears to predate the Unix epoch. Mimir assumes
    /// positive epoch times; this error is returned only when the host
    /// clock is badly misconfigured.
    #[error("system clock before Unix epoch")]
    BeforeEpoch,

    /// The requested value collides with the canonical-form `None`
    /// sentinel (`u64::MAX`). The Mimir API reserves that value for
    /// "no invalidation" encoding.
    #[error("reserved sentinel value {0} (u64::MAX)")]
    ReservedSentinel(u64),
}

impl ClockTime {
    /// Construct a [`ClockTime`] from raw milliseconds since the epoch.
    ///
    /// # Errors
    ///
    /// Returns [`ClockTimeError::ReservedSentinel`] if `millis == u64::MAX`,
    /// because that value is reserved by the canonical form to encode
    /// `Option::<ClockTime>::None` (per `ir-canonical-form.md` § 3.1).
    ///
    /// # Examples
    ///
    /// ```
    /// use mimir_core::{ClockTime, ClockTimeError};
    ///
    /// assert!(ClockTime::try_from_millis(0).is_ok());
    /// assert_eq!(
    ///     ClockTime::try_from_millis(u64::MAX),
    ///     Err(ClockTimeError::ReservedSentinel(u64::MAX)),
    /// );
    /// ```
    pub const fn try_from_millis(millis: u64) -> Result<Self, ClockTimeError> {
        if millis == NONE_SENTINEL {
            Err(ClockTimeError::ReservedSentinel(millis))
        } else {
            Ok(Self(millis))
        }
    }

    /// Current wall-clock time as milliseconds since the Unix epoch.
    ///
    /// # Errors
    ///
    /// Returns [`ClockTimeError::BeforeEpoch`] if the host clock reports
    /// a time before the Unix epoch (should not happen on sane hosts).
    pub fn now() -> Result<Self, ClockTimeError> {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ClockTimeError::BeforeEpoch)?
            .as_millis();
        // `as_millis` returns u128; values up to 2^64-1 ms cover year
        // 584,000,000, so truncation cannot occur for any real clock.
        let truncated = u64::try_from(millis).unwrap_or(NONE_SENTINEL - 1);
        Self::try_from_millis(truncated)
    }

    /// The underlying millisecond count.
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ClockTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}ms", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentinel_is_rejected() {
        assert!(matches!(
            ClockTime::try_from_millis(u64::MAX),
            Err(ClockTimeError::ReservedSentinel(_))
        ));
    }

    #[test]
    fn ordering_is_numeric() {
        let a = ClockTime::try_from_millis(100).expect("non-sentinel");
        let b = ClockTime::try_from_millis(200).expect("non-sentinel");
        assert!(a < b);
    }

    #[test]
    fn now_is_close_to_epoch_plus_wallclock() {
        let t = ClockTime::now().expect("wall clock sane");
        // Any reasonable wall clock is after 2020-01-01 (1_577_836_800_000 ms).
        assert!(t.as_millis() > 1_577_836_800_000);
    }
}
