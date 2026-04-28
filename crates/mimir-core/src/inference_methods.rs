//! Inference-method registry per `docs/concepts/librarian-pipeline.md` § 5.
//!
//! Fourteen named methods, each with:
//!
//! 1. A **parent-count rule** (exactly 1, exactly 2, N ≥ 2, N ≥ 1, etc.).
//! 2. A **deterministic confidence formula** computed over parent confidences.
//! 3. A **staleness predicate** — the condition under which the method's
//!    output must be flagged stale if a parent is superseded.
//!
//! All arithmetic is integer fixed-point at the `u16` confidence
//! resolution (per `ir-canonical-form.md` § 3.1) so output is
//! bit-identical across architectures. Intermediate products use u128
//! to accommodate up to eight 16-bit parents without overflow; methods
//! accepting more than eight parents cap their input at eight and
//! return `InferenceMethodError::TooManyParents` otherwise (this
//! bounded-determinism caveat is flagged in CHANGELOG pending a
//! follow-up log-table implementation for unbounded N).

use thiserror::Error;

use crate::confidence::Confidence;

// -------------------------------------------------------------------
// Method enum
// -------------------------------------------------------------------

/// One of the 14 registered inference methods. Every Inferential memory's
/// `method` field resolves to a symbol whose canonical name maps to one
/// of these variants.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum InferenceMethod {
    /// Pass-through from a single parent. `output.conf = parent.conf`.
    ///
    /// ```
    /// # #![allow(clippy::unwrap_used)]
    /// use mimir_core::inference_methods::InferenceMethod;
    /// use mimir_core::Confidence;
    /// let out = InferenceMethod::DirectLookup
    ///     .compute(&[Confidence::try_from_f32(0.73).unwrap()])
    ///     .unwrap();
    /// assert!((out.as_f32() - 0.73).abs() < 0.001);
    /// ```
    DirectLookup,
    /// Majority vote across an odd number of parents (N ≥ 3). Per the
    /// spec v1 convention (`librarian-pipeline.md` § 5.1) the write
    /// surface carries only voters-in-favor in `derived_from`, so
    /// `votes_for == N` and the formula collapses to `min(parents.conf)`.
    ///
    /// ```
    /// # #![allow(clippy::unwrap_used)]
    /// use mimir_core::inference_methods::InferenceMethod;
    /// use mimir_core::Confidence;
    /// let p = |f| Confidence::try_from_f32(f).unwrap();
    /// let out = InferenceMethod::MajorityVote.compute(&[p(0.9), p(0.6), p(0.7)]).unwrap();
    /// assert!((out.as_f32() - 0.6).abs() < 0.001);
    /// ```
    MajorityVote,
    /// Citation-linked pair of parents.
    /// `output.conf = min(parents.conf) * 0.9`.
    ///
    /// ```
    /// # #![allow(clippy::unwrap_used)]
    /// use mimir_core::inference_methods::InferenceMethod;
    /// use mimir_core::Confidence;
    /// let p = |f| Confidence::try_from_f32(f).unwrap();
    /// let out = InferenceMethod::CitationLink.compute(&[p(1.0), p(1.0)]).unwrap();
    /// assert!((out.as_f32() - 0.9).abs() < 0.001);
    /// ```
    CitationLink,
    /// Analogical mapping from source to target.
    /// `output.conf = product(parents.conf) * 0.7`.
    ///
    /// ```
    /// # #![allow(clippy::unwrap_used)]
    /// use mimir_core::inference_methods::InferenceMethod;
    /// use mimir_core::Confidence;
    /// let p = |f| Confidence::try_from_f32(f).unwrap();
    /// let out = InferenceMethod::AnalogyInference.compute(&[p(1.0), p(1.0)]).unwrap();
    /// assert!((out.as_f32() - 0.7).abs() < 0.001);
    /// ```
    AnalogyInference,
    /// Geometric-mean summarization over N ≥ 2 parents.
    /// `output.conf = geomean(parents.conf) * 0.8`.
    ///
    /// ```
    /// # #![allow(clippy::unwrap_used)]
    /// use mimir_core::inference_methods::InferenceMethod;
    /// use mimir_core::Confidence;
    /// let p = |f| Confidence::try_from_f32(f).unwrap();
    /// // geomean(0.5, 0.5) = 0.5; * 0.8 = 0.4.
    /// let out = InferenceMethod::PatternSummarize.compute(&[p(0.5), p(0.5)]).unwrap();
    /// assert!((out.as_f32() - 0.4).abs() < 0.001);
    /// ```
    PatternSummarize,
    /// Chained architectural derivation.
    /// `output.conf = product(parents.conf)`.
    ///
    /// ```
    /// # #![allow(clippy::unwrap_used)]
    /// use mimir_core::inference_methods::InferenceMethod;
    /// use mimir_core::Confidence;
    /// let p = |f| Confidence::try_from_f32(f).unwrap();
    /// // 0.5 * 0.5 * 0.5 = 0.125.
    /// let out = InferenceMethod::ArchitecturalChain
    ///     .compute(&[p(0.5), p(0.5), p(0.5)]).unwrap();
    /// assert!((out.as_f32() - 0.125).abs() < 0.001);
    /// ```
    ArchitecturalChain,
    /// Dominance / ranking analysis.
    /// `output.conf = min(parents.conf) * 0.6`.
    ///
    /// ```
    /// # #![allow(clippy::unwrap_used)]
    /// use mimir_core::inference_methods::InferenceMethod;
    /// use mimir_core::Confidence;
    /// let p = |f| Confidence::try_from_f32(f).unwrap();
    /// // min(0.9, 0.5) * 0.6 = 0.3.
    /// let out = InferenceMethod::DominanceAnalysis.compute(&[p(0.9), p(0.5)]).unwrap();
    /// assert!((out.as_f32() - 0.3).abs() < 0.001);
    /// ```
    DominanceAnalysis,
    /// Cardinality / count over N ≥ 1 parents.
    /// `output.conf = min(parents.conf) * 0.8`.
    ///
    /// ```
    /// # #![allow(clippy::unwrap_used)]
    /// use mimir_core::inference_methods::InferenceMethod;
    /// use mimir_core::Confidence;
    /// let p = |f| Confidence::try_from_f32(f).unwrap();
    /// let out = InferenceMethod::EntityCount.compute(&[p(0.7)]).unwrap();
    /// assert!((out.as_f32() - 0.56).abs() < 0.001); // 0.7 * 0.8.
    /// ```
    EntityCount,
    /// Interval calculation between two endpoint parents.
    /// `output.conf = min(parents.conf) * 0.9`.
    ///
    /// ```
    /// # #![allow(clippy::unwrap_used)]
    /// use mimir_core::inference_methods::InferenceMethod;
    /// use mimir_core::Confidence;
    /// let p = |f| Confidence::try_from_f32(f).unwrap();
    /// let out = InferenceMethod::IntervalCalc.compute(&[p(0.8), p(0.6)]).unwrap();
    /// assert!((out.as_f32() - 0.54).abs() < 0.001); // min=0.6; *0.9.
    /// ```
    IntervalCalc,
    /// Feedback consolidation over N ≥ 1 parents.
    /// `output.conf = min(parents.conf) * 0.85`.
    ///
    /// ```
    /// # #![allow(clippy::unwrap_used)]
    /// use mimir_core::inference_methods::InferenceMethod;
    /// use mimir_core::Confidence;
    /// let p = |f| Confidence::try_from_f32(f).unwrap();
    /// let out = InferenceMethod::FeedbackConsolidation.compute(&[p(0.6)]).unwrap();
    /// assert!((out.as_f32() - 0.51).abs() < 0.001); // 0.6 * 0.85.
    /// ```
    FeedbackConsolidation,
    /// Qualitative / narrative inference.
    /// `output.conf = min(parents.conf) * 0.5`.
    ///
    /// ```
    /// # #![allow(clippy::unwrap_used)]
    /// use mimir_core::inference_methods::InferenceMethod;
    /// use mimir_core::Confidence;
    /// let p = |f| Confidence::try_from_f32(f).unwrap();
    /// let out = InferenceMethod::QualitativeInference.compute(&[p(0.8)]).unwrap();
    /// assert!((out.as_f32() - 0.4).abs() < 0.001);
    /// ```
    QualitativeInference,
    /// Provenance chain through N ≥ 2 parents.
    /// `output.conf = product(parents.conf)`.
    ///
    /// ```
    /// # #![allow(clippy::unwrap_used)]
    /// use mimir_core::inference_methods::InferenceMethod;
    /// use mimir_core::Confidence;
    /// let p = |f| Confidence::try_from_f32(f).unwrap();
    /// let out = InferenceMethod::ProvenanceChain.compute(&[p(0.9), p(0.8)]).unwrap();
    /// assert!((out.as_f32() - 0.72).abs() < 0.001);
    /// ```
    ProvenanceChain,
    /// Noisy-OR consensus across independent parents.
    /// `output.conf = 1 - product(1 - parents.conf)`.
    ///
    /// ```
    /// # #![allow(clippy::unwrap_used)]
    /// use mimir_core::inference_methods::InferenceMethod;
    /// use mimir_core::Confidence;
    /// let p = |f| Confidence::try_from_f32(f).unwrap();
    /// // 1 - (0.5)(0.5) = 0.75.
    /// let out = InferenceMethod::MultiSourceConsensus.compute(&[p(0.5), p(0.5)]).unwrap();
    /// assert!((out.as_f32() - 0.75).abs() < 0.001);
    /// ```
    MultiSourceConsensus,
    /// Conflict reconciliation over contested peers.
    /// `output.conf = max(parents.conf) * 0.8`.
    ///
    /// ```
    /// # #![allow(clippy::unwrap_used)]
    /// use mimir_core::inference_methods::InferenceMethod;
    /// use mimir_core::Confidence;
    /// let p = |f| Confidence::try_from_f32(f).unwrap();
    /// // max(0.3, 0.9) * 0.8 = 0.72.
    /// let out = InferenceMethod::ConflictReconciliation.compute(&[p(0.3), p(0.9)]).unwrap();
    /// assert!((out.as_f32() - 0.72).abs() < 0.001);
    /// ```
    ConflictReconciliation,
}

impl InferenceMethod {
    /// The canonical symbol name (with leading `@`) for this method.
    #[must_use]
    pub fn symbol_name(self) -> &'static str {
        match self {
            Self::DirectLookup => "@direct_lookup",
            Self::MajorityVote => "@majority_vote",
            Self::CitationLink => "@citation_link",
            Self::AnalogyInference => "@analogy_inference",
            Self::PatternSummarize => "@pattern_summarize",
            Self::ArchitecturalChain => "@architectural_chain",
            Self::DominanceAnalysis => "@dominance_analysis",
            Self::EntityCount => "@entity_count",
            Self::IntervalCalc => "@interval_calc",
            Self::FeedbackConsolidation => "@feedback_consolidation",
            Self::QualitativeInference => "@qualitative_inference",
            Self::ProvenanceChain => "@provenance_chain",
            Self::MultiSourceConsensus => "@multi_source_consensus",
            Self::ConflictReconciliation => "@conflict_reconciliation",
        }
    }

    /// Resolve a canonical method name (with or without leading `@`) to
    /// a variant, or `None` if the name is not a registered method.
    #[must_use]
    pub fn from_symbol_name(name: &str) -> Option<Self> {
        let bare = name.strip_prefix('@').unwrap_or(name);
        Some(match bare {
            "direct_lookup" => Self::DirectLookup,
            "majority_vote" => Self::MajorityVote,
            "citation_link" => Self::CitationLink,
            "analogy_inference" => Self::AnalogyInference,
            "pattern_summarize" => Self::PatternSummarize,
            "architectural_chain" => Self::ArchitecturalChain,
            "dominance_analysis" => Self::DominanceAnalysis,
            "entity_count" => Self::EntityCount,
            "interval_calc" => Self::IntervalCalc,
            "feedback_consolidation" => Self::FeedbackConsolidation,
            "qualitative_inference" => Self::QualitativeInference,
            "provenance_chain" => Self::ProvenanceChain,
            "multi_source_consensus" => Self::MultiSourceConsensus,
            "conflict_reconciliation" => Self::ConflictReconciliation,
            _ => return None,
        })
    }

    /// Parent-count rule expected by this method.
    #[must_use]
    pub fn parent_count_rule(self) -> ParentCountRule {
        match self {
            Self::DirectLookup => ParentCountRule::Exactly(1),
            Self::MajorityVote => ParentCountRule::AtLeastOdd(3),
            Self::CitationLink | Self::AnalogyInference | Self::IntervalCalc => {
                ParentCountRule::Exactly(2)
            }
            Self::PatternSummarize
            | Self::ArchitecturalChain
            | Self::DominanceAnalysis
            | Self::ProvenanceChain
            | Self::MultiSourceConsensus
            | Self::ConflictReconciliation => ParentCountRule::AtLeast(2),
            Self::EntityCount | Self::FeedbackConsolidation | Self::QualitativeInference => {
                ParentCountRule::AtLeast(1)
            }
        }
    }

    /// Staleness predicate per spec § 5.1.
    #[must_use]
    pub fn staleness_rule(self) -> StalenessRule {
        match self {
            Self::DirectLookup
            | Self::MajorityVote
            | Self::ArchitecturalChain
            | Self::DominanceAnalysis
            | Self::FeedbackConsolidation
            | Self::QualitativeInference
            | Self::ProvenanceChain => StalenessRule::AnyParentSuperseded,
            Self::CitationLink | Self::AnalogyInference | Self::IntervalCalc => {
                StalenessRule::EitherEndpointSuperseded
            }
            Self::PatternSummarize => StalenessRule::OverHalfSuperseded,
            Self::EntityCount => StalenessRule::ParentCountChanges,
            Self::MultiSourceConsensus => StalenessRule::FewerThanTwoRemain,
            Self::ConflictReconciliation => StalenessRule::AnyParentSupersededOrNewConflict,
        }
    }

    /// Compute the output confidence for this method given parent
    /// confidences. See variant docs for each formula.
    ///
    /// # Errors
    ///
    /// - [`InferenceMethodError::WrongParentCount`] if the supplied
    ///   count violates the method's [`ParentCountRule`].
    /// - [`InferenceMethodError::TooManyParents`] if `parents.len() > 8`
    ///   for methods whose formulas require a joint product
    ///   (`ArchitecturalChain`, `ProvenanceChain`, `AnalogyInference`,
    ///   `PatternSummarize`, `MultiSourceConsensus`). The u128
    ///   intermediate accommodates up to eight 16-bit factors; larger N
    ///   is reserved for a log-table follow-up.
    ///
    /// # Example
    ///
    /// ```
    /// # #![allow(clippy::unwrap_used)]
    /// use mimir_core::inference_methods::InferenceMethod;
    /// use mimir_core::Confidence;
    ///
    /// let parents = [Confidence::ONE, Confidence::ONE];
    /// let out = InferenceMethod::CitationLink.compute(&parents).unwrap();
    /// // min(1.0, 1.0) * 0.9 ≈ 0.9; allow ±1 fixed-point step of drift.
    /// let expected = (f32::from(u16::MAX) * 0.9) as u16;
    /// assert!((i32::from(out.as_u16()) - i32::from(expected)).abs() <= 1);
    /// ```
    #[allow(clippy::too_many_lines, clippy::match_same_arms)]
    pub fn compute(self, parents: &[Confidence]) -> Result<Confidence, InferenceMethodError> {
        self.parent_count_rule().validate(self, parents.len())?;

        match self {
            Self::DirectLookup => Ok(parents[0]),

            Self::MajorityVote => Ok(min_conf(parents)),

            Self::CitationLink => {
                let m = min_conf(parents);
                Ok(scale_rational(m, 9, 10))
            }

            Self::AnalogyInference => {
                let prod = product_conf(self, parents)?;
                Ok(scale_rational(prod, 7, 10))
            }

            Self::PatternSummarize => {
                let g = geomean_conf(parents)?;
                Ok(scale_rational(g, 4, 5))
            }

            Self::ArchitecturalChain => product_conf(self, parents),

            Self::DominanceAnalysis => {
                let m = min_conf(parents);
                Ok(scale_rational(m, 3, 5))
            }

            Self::EntityCount => {
                let m = min_conf(parents);
                Ok(scale_rational(m, 4, 5))
            }

            Self::IntervalCalc => {
                let m = min_conf(parents);
                Ok(scale_rational(m, 9, 10))
            }

            Self::FeedbackConsolidation => {
                let m = min_conf(parents);
                Ok(scale_rational(m, 17, 20))
            }

            Self::QualitativeInference => {
                let m = min_conf(parents);
                Ok(scale_rational(m, 1, 2))
            }

            Self::ProvenanceChain => product_conf(self, parents),

            Self::MultiSourceConsensus => {
                // Noisy-OR: 1 - product(1 - p_i). Build a vector of
                // complements and product them.
                if parents.len() > 8 {
                    return Err(InferenceMethodError::TooManyParents {
                        method: self,
                        limit: 8,
                        got: parents.len(),
                    });
                }
                let complements: Vec<Confidence> = parents
                    .iter()
                    .map(|c| Confidence::from_u16(u16::MAX - c.as_u16()))
                    .collect();
                let prod_complement = product_conf(self, &complements)?;
                Ok(Confidence::from_u16(u16::MAX - prod_complement.as_u16()))
            }

            Self::ConflictReconciliation => {
                let m = max_conf(parents);
                Ok(scale_rational(m, 4, 5))
            }
        }
    }
}

// -------------------------------------------------------------------
// Rules
// -------------------------------------------------------------------

/// Parent-count constraint for a method.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ParentCountRule {
    /// Exactly this many parents.
    Exactly(usize),
    /// At least this many parents (no parity constraint).
    AtLeast(usize),
    /// At least this many parents, and the count must be odd.
    AtLeastOdd(usize),
}

impl ParentCountRule {
    fn validate(self, method: InferenceMethod, n: usize) -> Result<(), InferenceMethodError> {
        let ok = match self {
            Self::Exactly(k) => n == k,
            Self::AtLeast(k) => n >= k,
            Self::AtLeastOdd(k) => n >= k && n % 2 == 1,
        };
        if ok {
            Ok(())
        } else {
            Err(InferenceMethodError::WrongParentCount {
                method,
                rule: self,
                got: n,
            })
        }
    }
}

/// Staleness predicate — when the method's output is flagged stale.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StalenessRule {
    /// Flag stale if *any* parent is superseded.
    AnyParentSuperseded,
    /// Flag stale if *either* of the two endpoint parents is superseded.
    EitherEndpointSuperseded,
    /// Flag stale if more than 50% of parents are superseded.
    OverHalfSuperseded,
    /// Flag stale when the entity count changes (e.g., parent membership
    /// gains / loses an item).
    ParentCountChanges,
    /// Flag stale when fewer than two non-superseded parents remain.
    FewerThanTwoRemain,
    /// Flag stale on any parent supersession OR when a new conflicting
    /// memory lands on the same conflict key.
    AnyParentSupersededOrNewConflict,
}

// -------------------------------------------------------------------
// Errors
// -------------------------------------------------------------------

/// Errors from [`InferenceMethod::compute`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum InferenceMethodError {
    /// Parent count did not match the method's [`ParentCountRule`].
    #[error("method {method:?} requires {rule:?} parents, got {got}")]
    WrongParentCount {
        /// The method.
        method: InferenceMethod,
        /// The rule that was violated.
        rule: ParentCountRule,
        /// Actual parent count supplied.
        got: usize,
    },

    /// Supplied parent count exceeds the u128 product capacity. Bounded
    /// at 8 for methods that compute a joint product across all
    /// parents; relaxing this requires a log-table implementation.
    #[error("method {method:?} supports at most {limit} parents, got {got}")]
    TooManyParents {
        /// The method.
        method: InferenceMethod,
        /// Maximum supported parent count.
        limit: usize,
        /// Actual parent count supplied.
        got: usize,
    },
}

// -------------------------------------------------------------------
// Fixed-point arithmetic helpers
// -------------------------------------------------------------------

/// Minimum confidence. Spec requires a non-empty list; callers are
/// responsible for upstream validation.
fn min_conf(parents: &[Confidence]) -> Confidence {
    parents.iter().min().copied().unwrap_or(Confidence::ZERO)
}

fn max_conf(parents: &[Confidence]) -> Confidence {
    parents.iter().max().copied().unwrap_or(Confidence::ZERO)
}

/// Fixed-point product in u16 scale. `a * b / u16::MAX`, round-to-nearest.
/// Implementation: `((a as u64 * b as u64) + u16::MAX / 2) / u16::MAX`.
fn mul_conf(a: Confidence, b: Confidence) -> Confidence {
    let a64 = u64::from(a.as_u16());
    let b64 = u64::from(b.as_u16());
    let max = u64::from(u16::MAX);
    let raw = (a64 * b64 + max / 2) / max;
    // raw ≤ u16::MAX because a, b ≤ u16::MAX and (a*b)/max ≤ max.
    #[allow(clippy::cast_possible_truncation)]
    Confidence::from_u16(raw as u16)
}

/// Product of a non-empty slice of confidences. Bounded at 8 parents to
/// keep the u128 intermediate below overflow; `method` is taken as a
/// parameter so the typed error reports the correct caller.
fn product_conf(
    method: InferenceMethod,
    parents: &[Confidence],
) -> Result<Confidence, InferenceMethodError> {
    if parents.len() > 8 {
        return Err(InferenceMethodError::TooManyParents {
            method,
            limit: 8,
            got: parents.len(),
        });
    }
    let mut acc = Confidence::ONE;
    for p in parents {
        acc = mul_conf(acc, *p);
    }
    Ok(acc)
}

/// Scale a confidence by a rational `num/den`, round-to-nearest.
/// Precondition: `num ≤ den ≤ u16::MAX` so the result stays in range.
fn scale_rational(c: Confidence, num: u64, den: u64) -> Confidence {
    debug_assert!(num <= den);
    let v = u64::from(c.as_u16());
    let raw = (v * num + den / 2) / den;
    #[allow(clippy::cast_possible_truncation)]
    Confidence::from_u16(raw as u16)
}

/// Geometric mean of u16 confidences. Integer Newton's method on u128
/// product. Bounded at N ≤ 8 parents so the u128 product cannot overflow
/// (`65535^8 ≈ 3.2e38 < 3.4e38 = u128::MAX`). Spec § 5.1 permits unbounded
/// N; lifting this cap waits on a log-table implementation.
fn geomean_conf(parents: &[Confidence]) -> Result<Confidence, InferenceMethodError> {
    if parents.len() > 8 {
        return Err(InferenceMethodError::TooManyParents {
            method: InferenceMethod::PatternSummarize,
            limit: 8,
            got: parents.len(),
        });
    }
    if parents.iter().any(|c| c.as_u16() == 0) {
        return Ok(Confidence::ZERO);
    }
    // With the `p/65535` scaling convention, the geomean of u16 values
    // simplifies to `prod(p_i)^(1/n)` — the outer 65535 scaling cancels.
    let n = parents.len();
    let mut prod: u128 = 1;
    for p in parents {
        prod *= u128::from(p.as_u16());
    }
    #[allow(clippy::cast_possible_truncation)]
    let root = integer_nth_root_u128(prod, n as u32) as u16;
    Ok(Confidence::from_u16(root))
}

/// Integer `n`-th root of `x` via Newton's method. Returns `floor(x^(1/n))`.
fn integer_nth_root_u128(x: u128, n: u32) -> u128 {
    if x < 2 {
        return x;
    }
    if n <= 1 {
        return x;
    }
    // Initial estimate: 2^(bits(x) / n) is at-least the true root.
    let bits = 128 - x.leading_zeros();
    let shift = (bits / n).min(127);
    let mut y: u128 = 1u128 << shift;
    // Ensure upper bound.
    while y.checked_pow(n).is_none_or(|v| v < x) {
        y = y.saturating_mul(2);
        if y >= 1u128 << 127 {
            break;
        }
    }
    // Newton's iteration.
    loop {
        let y_pow = checked_pow_u128(y, n - 1);
        if y_pow == 0 {
            break;
        }
        let y_new = ((u128::from(n) - 1) * y + x / y_pow) / u128::from(n);
        if y_new >= y {
            break;
        }
        y = y_new;
    }
    // Fine-tune down to floor.
    while y > 0 && y.checked_pow(n).is_none_or(|v| v > x) {
        y -= 1;
    }
    y
}

/// `y^n` saturating to 0 on overflow (0 is a sentinel for "too big to
/// divide by"); Newton's iteration handles the sentinel by breaking.
fn checked_pow_u128(y: u128, n: u32) -> u128 {
    y.checked_pow(n).unwrap_or(0)
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

    // ±1 fixed-point step tolerance for round-trip comparisons.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn approx(a: Confidence, expected_f: f32) {
        let expected = (f32::from(u16::MAX) * expected_f).round() as i32;
        let actual = i32::from(a.as_u16());
        assert!(
            (actual - expected).abs() <= 1,
            "expected ≈{expected_f} (u16={expected}), got {} (u16={actual})",
            a.as_f32()
        );
    }

    // ----- symbol-name round-trip -----

    #[test]
    fn every_variant_roundtrips_through_symbol_name() {
        let variants = [
            InferenceMethod::DirectLookup,
            InferenceMethod::MajorityVote,
            InferenceMethod::CitationLink,
            InferenceMethod::AnalogyInference,
            InferenceMethod::PatternSummarize,
            InferenceMethod::ArchitecturalChain,
            InferenceMethod::DominanceAnalysis,
            InferenceMethod::EntityCount,
            InferenceMethod::IntervalCalc,
            InferenceMethod::FeedbackConsolidation,
            InferenceMethod::QualitativeInference,
            InferenceMethod::ProvenanceChain,
            InferenceMethod::MultiSourceConsensus,
            InferenceMethod::ConflictReconciliation,
        ];
        assert_eq!(variants.len(), 14);
        for m in variants {
            let name = m.symbol_name();
            let back = InferenceMethod::from_symbol_name(name).expect("known");
            assert_eq!(m, back);
        }
    }

    #[test]
    fn from_symbol_name_accepts_without_at_prefix() {
        assert_eq!(
            InferenceMethod::from_symbol_name("direct_lookup"),
            Some(InferenceMethod::DirectLookup)
        );
        assert_eq!(
            InferenceMethod::from_symbol_name("@direct_lookup"),
            Some(InferenceMethod::DirectLookup)
        );
    }

    #[test]
    fn unknown_name_returns_none() {
        assert!(InferenceMethod::from_symbol_name("@my_custom_method").is_none());
    }

    // ----- parent-count rules -----

    #[test]
    fn direct_lookup_requires_exactly_one() {
        let method = InferenceMethod::DirectLookup;
        assert!(method.compute(&[c(0.5)]).is_ok());
        assert!(matches!(
            method.compute(&[]).unwrap_err(),
            InferenceMethodError::WrongParentCount { .. }
        ));
        assert!(matches!(
            method.compute(&[c(0.5), c(0.6)]).unwrap_err(),
            InferenceMethodError::WrongParentCount { .. }
        ));
    }

    #[test]
    fn majority_vote_rejects_even_n() {
        let method = InferenceMethod::MajorityVote;
        assert!(method.compute(&[c(0.5), c(0.6), c(0.7)]).is_ok());
        assert!(matches!(
            method.compute(&[c(0.5), c(0.6)]).unwrap_err(),
            InferenceMethodError::WrongParentCount { .. }
        ));
        assert!(matches!(
            method.compute(&[c(0.5)]).unwrap_err(),
            InferenceMethodError::WrongParentCount { .. }
        ));
    }

    #[test]
    fn exactly_two_methods_reject_other_counts() {
        for m in [
            InferenceMethod::CitationLink,
            InferenceMethod::AnalogyInference,
            InferenceMethod::IntervalCalc,
        ] {
            assert!(m.compute(&[c(0.5), c(0.6)]).is_ok());
            assert!(matches!(
                m.compute(&[c(0.5)]).unwrap_err(),
                InferenceMethodError::WrongParentCount { .. }
            ));
            assert!(matches!(
                m.compute(&[c(0.5), c(0.6), c(0.7)]).unwrap_err(),
                InferenceMethodError::WrongParentCount { .. }
            ));
        }
    }

    // ----- formula behavior -----

    #[test]
    fn direct_lookup_is_identity() {
        let out = InferenceMethod::DirectLookup
            .compute(&[c(0.75)])
            .expect("ok");
        approx(out, 0.75);
    }

    #[test]
    fn citation_link_scales_by_0_9() {
        let out = InferenceMethod::CitationLink
            .compute(&[c(1.0), c(1.0)])
            .expect("ok");
        approx(out, 0.9);
    }

    #[test]
    fn citation_link_uses_min() {
        let out = InferenceMethod::CitationLink
            .compute(&[c(0.8), c(0.5)])
            .expect("ok");
        approx(out, 0.5 * 0.9);
    }

    #[test]
    fn analogy_inference_scales_product_by_0_7() {
        let out = InferenceMethod::AnalogyInference
            .compute(&[c(1.0), c(1.0)])
            .expect("ok");
        approx(out, 0.7);
        let out = InferenceMethod::AnalogyInference
            .compute(&[c(0.5), c(0.5)])
            .expect("ok");
        approx(out, 0.25 * 0.7);
    }

    #[test]
    fn architectural_chain_is_raw_product() {
        let out = InferenceMethod::ArchitecturalChain
            .compute(&[c(0.5), c(0.5), c(0.5)])
            .expect("ok");
        approx(out, 0.125);
    }

    #[test]
    fn dominance_analysis_scales_min_by_0_6() {
        let out = InferenceMethod::DominanceAnalysis
            .compute(&[c(0.9), c(0.5)])
            .expect("ok");
        approx(out, 0.5 * 0.6);
    }

    #[test]
    fn entity_count_scales_min_by_0_8() {
        let out = InferenceMethod::EntityCount.compute(&[c(0.7)]).expect("ok");
        approx(out, 0.7 * 0.8);
    }

    #[test]
    fn interval_calc_scales_min_by_0_9() {
        let out = InferenceMethod::IntervalCalc
            .compute(&[c(0.8), c(0.6)])
            .expect("ok");
        approx(out, 0.6 * 0.9);
    }

    #[test]
    fn feedback_consolidation_scales_min_by_0_85() {
        let out = InferenceMethod::FeedbackConsolidation
            .compute(&[c(0.6)])
            .expect("ok");
        approx(out, 0.6 * 0.85);
    }

    #[test]
    fn qualitative_inference_scales_min_by_0_5() {
        let out = InferenceMethod::QualitativeInference
            .compute(&[c(0.8)])
            .expect("ok");
        approx(out, 0.8 * 0.5);
    }

    #[test]
    fn provenance_chain_is_raw_product() {
        let out = InferenceMethod::ProvenanceChain
            .compute(&[c(0.9), c(0.8), c(0.7)])
            .expect("ok");
        approx(out, 0.9 * 0.8 * 0.7);
    }

    #[test]
    fn multi_source_consensus_noisy_or_raises_confidence() {
        // Two independent 0.5 parents: 1 - (0.5 * 0.5) = 0.75.
        let out = InferenceMethod::MultiSourceConsensus
            .compute(&[c(0.5), c(0.5)])
            .expect("ok");
        approx(out, 0.75);
    }

    #[test]
    fn multi_source_consensus_saturates_at_one_with_strong_parents() {
        let out = InferenceMethod::MultiSourceConsensus
            .compute(&[c(1.0), c(1.0)])
            .expect("ok");
        approx(out, 1.0);
    }

    #[test]
    fn conflict_reconciliation_scales_max_by_0_8() {
        let out = InferenceMethod::ConflictReconciliation
            .compute(&[c(0.3), c(0.9)])
            .expect("ok");
        approx(out, 0.9 * 0.8);
    }

    #[test]
    fn pattern_summarize_geomean_of_uniform_is_identity_scaled() {
        // geomean(0.5, 0.5, 0.5, 0.5) = 0.5; * 0.8 = 0.4.
        let out = InferenceMethod::PatternSummarize
            .compute(&[c(0.5), c(0.5), c(0.5), c(0.5)])
            .expect("ok");
        approx(out, 0.4);
    }

    #[test]
    fn pattern_summarize_of_two_is_sqrt_product_scaled() {
        // geomean(0.25, 1.0) = 0.5; * 0.8 = 0.4.
        let out = InferenceMethod::PatternSummarize
            .compute(&[c(0.25), c(1.0)])
            .expect("ok");
        approx(out, 0.4);
    }

    // ----- range + monotonicity -----

    #[test]
    fn every_method_output_stays_in_range_for_saturated_input() {
        // Build a valid parent list for each method and assert the
        // output is a valid confidence (trivially in [0, 1]).
        let parents_for = |m: InferenceMethod| -> Vec<Confidence> {
            let k = match m.parent_count_rule() {
                ParentCountRule::Exactly(k)
                | ParentCountRule::AtLeast(k)
                | ParentCountRule::AtLeastOdd(k) => k,
            };
            vec![c(1.0); k]
        };
        let variants = [
            InferenceMethod::DirectLookup,
            InferenceMethod::MajorityVote,
            InferenceMethod::CitationLink,
            InferenceMethod::AnalogyInference,
            InferenceMethod::PatternSummarize,
            InferenceMethod::ArchitecturalChain,
            InferenceMethod::DominanceAnalysis,
            InferenceMethod::EntityCount,
            InferenceMethod::IntervalCalc,
            InferenceMethod::FeedbackConsolidation,
            InferenceMethod::QualitativeInference,
            InferenceMethod::ProvenanceChain,
            InferenceMethod::MultiSourceConsensus,
            InferenceMethod::ConflictReconciliation,
        ];
        for m in variants {
            let parents = parents_for(m);
            let out = m.compute(&parents).expect("compute");
            let v = out.as_f32();
            assert!(
                (0.0..=1.0).contains(&v),
                "method {m:?} output {v} out of range"
            );
        }
    }

    #[test]
    fn too_many_parents_errors_for_product_methods() {
        let nine = vec![c(0.5); 9];
        for m in [
            InferenceMethod::AnalogyInference,
            InferenceMethod::ArchitecturalChain,
            InferenceMethod::ProvenanceChain,
            InferenceMethod::MultiSourceConsensus,
            InferenceMethod::PatternSummarize,
        ] {
            let parents = match m.parent_count_rule() {
                ParentCountRule::Exactly(2) => continue, // AnalogyInference: fixed at 2.
                _ => nine.clone(),
            };
            let err = m.compute(&parents).expect_err("overflow");
            // Error must identify the actual calling method — not a
            // hardcoded placeholder (regression guard).
            let InferenceMethodError::TooManyParents { method, .. } = err else {
                panic!("expected TooManyParents, got {err:?}");
            };
            assert_eq!(method, m);
        }
    }

    // ----- integer Nth root sanity -----

    #[test]
    fn integer_nth_root_matches_known_values() {
        assert_eq!(integer_nth_root_u128(0, 2), 0);
        assert_eq!(integer_nth_root_u128(1, 2), 1);
        assert_eq!(integer_nth_root_u128(4, 2), 2);
        assert_eq!(integer_nth_root_u128(9, 2), 3);
        assert_eq!(integer_nth_root_u128(27, 3), 3);
        assert_eq!(integer_nth_root_u128(1000, 3), 10);
        // Non-exact: floor(sqrt(10)) = 3, floor(cbrt(28)) = 3.
        assert_eq!(integer_nth_root_u128(10, 2), 3);
        assert_eq!(integer_nth_root_u128(28, 3), 3);
    }

    // ----- staleness rules -----

    #[test]
    fn staleness_rules_match_spec() {
        use StalenessRule::*;
        assert_eq!(
            InferenceMethod::DirectLookup.staleness_rule(),
            AnyParentSuperseded
        );
        assert_eq!(
            InferenceMethod::PatternSummarize.staleness_rule(),
            OverHalfSuperseded
        );
        assert_eq!(
            InferenceMethod::MultiSourceConsensus.staleness_rule(),
            FewerThanTwoRemain
        );
        assert_eq!(
            InferenceMethod::EntityCount.staleness_rule(),
            ParentCountChanges
        );
        assert_eq!(
            InferenceMethod::ConflictReconciliation.staleness_rule(),
            AnyParentSupersededOrNewConflict
        );
        assert_eq!(
            InferenceMethod::CitationLink.staleness_rule(),
            EitherEndpointSuperseded
        );
    }
}
