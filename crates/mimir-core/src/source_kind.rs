//! `SourceKind` — the eleven-kind source taxonomy from
//! `docs/concepts/grounding-model.md` § 3.
//!
//! `confidence_bound()` and `admits()` are `const fn` so the taxonomy
//! is compile-time-resolved (per grounding-model.md § 4).

use crate::confidence::Confidence;
use crate::memory_kind::MemoryKindTag;

/// Kind of grounding source attached to a memory.
///
/// Eleven kinds, matching `grounding-model.md` § 3. `#[non_exhaustive]`
/// so additions do not break semver.
///
/// # Examples
///
/// ```
/// use mimir_core::{Confidence, MemoryKindTag, SourceKind};
///
/// let bound = SourceKind::Profile.confidence_bound();
/// assert!(bound < Confidence::ONE);
/// assert!(SourceKind::Profile.admits(MemoryKindTag::Semantic));
/// assert!(!SourceKind::Profile.admits(MemoryKindTag::Procedural));
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SourceKind {
    /// User / entity-provided identity or attribute data.
    Profile,
    /// Directly witnessed by an agent.
    Observation,
    /// Subject reported the fact about themselves.
    SelfReport,
    /// A participant in an event reported it.
    ParticipantReport,
    /// A cited document, URL, paper, or canonical spec.
    Document,
    /// An authoritative registry (package manifest, DNS, filesystem).
    Registry,
    /// A deliberate act of policy-making by a rule-maker.
    Policy,
    /// An instruction from the agent's operator / owner.
    AgentInstruction,
    /// A trusted third-party service or API (not a static document).
    ExternalAuthority,
    /// A transitional marker: the claim has not been primary-source
    /// verified yet.
    PendingVerification,
    /// A fact the librarian itself emitted (timestamps, symbol IDs).
    LibrarianAssignment,
}

impl SourceKind {
    /// Default confidence upper bound for this source kind.
    ///
    /// User-overridable via `mimir.toml` per `confidence-decay.md` § 4.
    /// This method returns the shipped default only.
    #[must_use]
    pub const fn confidence_bound(self) -> Confidence {
        // Fixed-point `u16` values derived per grounding-model.md § 3 table.
        // 1.0 = u16::MAX (65535); 0.95 ≈ 62258; 0.9 ≈ 58982; 0.85 ≈ 55705;
        // 0.6 ≈ 39321.
        match self {
            Self::Observation | Self::Policy | Self::LibrarianAssignment => Confidence::ONE,
            Self::Profile | Self::Registry | Self::AgentInstruction => Confidence::from_u16(62_258),
            Self::SelfReport | Self::Document | Self::ExternalAuthority => {
                Confidence::from_u16(58_982)
            }
            Self::ParticipantReport => Confidence::from_u16(55_705),
            Self::PendingVerification => Confidence::from_u16(39_321),
        }
    }

    /// Whether this source kind is admissible for a given memory kind.
    ///
    /// Matches the `Admits` column in `grounding-model.md` § 3.1 table.
    /// Inferential memories do not use the `source` field (they use
    /// `derived_from` + `method`), so `SourceKind::admits(Inferential)`
    /// is never `true`.
    #[must_use]
    pub const fn admits(self, kind: MemoryKindTag) -> bool {
        // Inferential memories never carry a `source` field.
        if matches!(kind, MemoryKindTag::Inferential) {
            return false;
        }
        match self {
            // Admit Semantic only: Profile, Document, Registry,
            // ExternalAuthority, LibrarianAssignment.
            Self::Profile
            | Self::Document
            | Self::Registry
            | Self::ExternalAuthority
            | Self::LibrarianAssignment => matches!(kind, MemoryKindTag::Semantic),
            Self::Observation | Self::SelfReport => {
                matches!(kind, MemoryKindTag::Semantic | MemoryKindTag::Episodic)
            }
            Self::ParticipantReport => matches!(kind, MemoryKindTag::Episodic),
            Self::Policy => matches!(kind, MemoryKindTag::Procedural),
            Self::AgentInstruction => {
                matches!(kind, MemoryKindTag::Procedural | MemoryKindTag::Semantic)
            }
            Self::PendingVerification => {
                matches!(
                    kind,
                    MemoryKindTag::Semantic | MemoryKindTag::Episodic | MemoryKindTag::Procedural
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inferential_admits_none() {
        for kind in [
            SourceKind::Profile,
            SourceKind::Observation,
            SourceKind::SelfReport,
            SourceKind::ParticipantReport,
            SourceKind::Document,
            SourceKind::Registry,
            SourceKind::Policy,
            SourceKind::AgentInstruction,
            SourceKind::ExternalAuthority,
            SourceKind::PendingVerification,
            SourceKind::LibrarianAssignment,
        ] {
            assert!(
                !kind.admits(MemoryKindTag::Inferential),
                "{kind:?} must not admit Inferential"
            );
        }
    }

    #[test]
    fn policy_admits_only_procedural() {
        assert!(SourceKind::Policy.admits(MemoryKindTag::Procedural));
        assert!(!SourceKind::Policy.admits(MemoryKindTag::Semantic));
        assert!(!SourceKind::Policy.admits(MemoryKindTag::Episodic));
    }

    #[test]
    fn pending_verification_bounded_at_point_six() {
        let c = SourceKind::PendingVerification.confidence_bound();
        assert!((c.as_f32() - 0.6).abs() < 1e-3);
    }

    #[test]
    fn observation_is_fully_confident() {
        assert_eq!(SourceKind::Observation.confidence_bound(), Confidence::ONE);
    }
}
