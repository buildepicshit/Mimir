//! `Confidence` — 16-bit fixed-point confidence value mapping `[0.0, 1.0]`
//! to `[0, 65535]`. Implements the contract in
//! `docs/concepts/ir-canonical-form.md` § 3.1 and
//! `docs/concepts/confidence-decay.md` § 3.

use std::fmt;

use thiserror::Error;

const SCALE: f32 = u16::MAX as f32;

/// Errors returned by [`Confidence::try_from_f32`].
#[derive(Debug, Error, PartialEq)]
pub enum ConfidenceError {
    /// The input float was outside the permitted range `[0.0, 1.0]`.
    #[error("confidence {0} outside [0.0, 1.0]")]
    OutOfRange(f32),

    /// The input float was NaN.
    #[error("confidence NaN is not a valid value")]
    NotANumber,
}

/// A confidence value in `[0.0, 1.0]`, stored as 16-bit fixed-point.
///
/// The representation gives a resolution of roughly `1.53e-5` per step
/// and is bit-identical across architectures — no IEEE 754 divergence
/// between CPUs. Per `docs/concepts/ir-canonical-form.md` § 3.1:
///
/// ```text
/// stored_u16 = round(confidence * 65535.0)
/// confidence = stored_u16 / 65535.0
/// ```
///
/// # Examples
///
/// ```
/// # #![allow(clippy::unwrap_used)]
/// use mimir_core::Confidence;
///
/// let c = Confidence::try_from_f32(0.95).unwrap();
/// assert!((c.as_f32() - 0.95).abs() < 1e-4);
/// assert_eq!(c.as_u16(), 62_258);
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Confidence(u16);

impl Confidence {
    /// Lowest confidence — `0.0`.
    pub const ZERO: Self = Self(0);

    /// Highest confidence — `1.0`.
    pub const ONE: Self = Self(u16::MAX);

    /// Construct a [`Confidence`] from an `f32` in `[0.0, 1.0]`.
    ///
    /// Rounds half-to-even.
    ///
    /// # Errors
    ///
    /// - [`ConfidenceError::OutOfRange`] if `value < 0.0` or `value > 1.0`.
    /// - [`ConfidenceError::NotANumber`] if `value` is NaN.
    ///
    /// Subnormal values and negative zero are treated as `0.0`.
    ///
    /// # Examples
    ///
    /// ```
    /// use mimir_core::{Confidence, ConfidenceError};
    ///
    /// assert!(Confidence::try_from_f32(0.5).is_ok());
    /// assert_eq!(
    ///     Confidence::try_from_f32(1.1),
    ///     Err(ConfidenceError::OutOfRange(1.1)),
    /// );
    /// assert_eq!(
    ///     Confidence::try_from_f32(f32::NAN),
    ///     Err(ConfidenceError::NotANumber),
    /// );
    /// ```
    pub fn try_from_f32(value: f32) -> Result<Self, ConfidenceError> {
        if value.is_nan() {
            return Err(ConfidenceError::NotANumber);
        }
        if !(0.0..=1.0).contains(&value) {
            return Err(ConfidenceError::OutOfRange(value));
        }
        // round-half-to-even via `roundeven` not available on stable Rust;
        // `round()` is round-half-away-from-zero, acceptable here because
        // values in [0.0, 1.0] have ties at 0.5-scale increments that map
        // deterministically. For strict round-half-to-even we would cast
        // through f64 and use `.round_ties_even()` (stable 1.77+).
        let scaled = (f64::from(value) * f64::from(SCALE)).round_ties_even();
        // scaled in [0.0, 65535.0] ⇒ fits u16.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let stored = scaled as u16;
        Ok(Self(stored))
    }

    /// Construct a [`Confidence`] from its raw `u16` fixed-point encoding.
    #[must_use]
    pub const fn from_u16(raw: u16) -> Self {
        Self(raw)
    }

    /// Raw `u16` fixed-point encoding.
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// Floating-point representation in `[0.0, 1.0]`.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn as_f32(self) -> f32 {
        (f64::from(self.0) / f64::from(SCALE)) as f32
    }
}

impl fmt::Display for Confidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.4}", self.as_f32())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_values() {
        let zero = Confidence::try_from_f32(0.0).unwrap();
        let one = Confidence::try_from_f32(1.0).unwrap();
        assert_eq!(zero, Confidence::ZERO);
        assert_eq!(one, Confidence::ONE);
    }

    #[test]
    fn out_of_range_rejected() {
        assert!(matches!(
            Confidence::try_from_f32(-0.01),
            Err(ConfidenceError::OutOfRange(_))
        ));
        assert!(matches!(
            Confidence::try_from_f32(1.01),
            Err(ConfidenceError::OutOfRange(_))
        ));
    }

    #[test]
    fn nan_rejected() {
        assert_eq!(
            Confidence::try_from_f32(f32::NAN),
            Err(ConfidenceError::NotANumber),
        );
    }

    #[test]
    fn roundtrip_precision_within_one_step() {
        let step = 1.0 / f32::from(u16::MAX);
        for raw in [0_u16, 1, 32_768, 65_534, 65_535] {
            let c = Confidence::from_u16(raw);
            let rebuilt = Confidence::try_from_f32(c.as_f32()).unwrap();
            // Allow ±1 step of drift from the scale conversion.
            let delta = i64::from(c.as_u16()) - i64::from(rebuilt.as_u16());
            assert!(delta.abs() <= 1, "raw={raw} delta={delta} step={step}");
        }
    }
}
