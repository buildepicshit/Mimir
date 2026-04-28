//! `MemoryKind` — the four canonical memory types from
//! `docs/concepts/memory-type-taxonomy.md` § 5.
//!
//! Each variant is a distinct struct with distinct fields, lifecycle,
//! and decay profile. Variants are frozen at the spec's graduation;
//! adding a fifth type is a breaking change per `PRINCIPLES.md` § 10.

use crate::clock::ClockTime;
use crate::confidence::Confidence;
use crate::symbol::SymbolId;
use crate::value::Value;

/// A Semantic memory — a general fact about the world.
///
/// Canonical examples: entity attributes, relationships, category
/// memberships. See `memory-type-taxonomy.md` § 3.1.
#[derive(Clone, Debug, PartialEq)]
pub struct Semantic {
    /// Subject.
    pub s: SymbolId,
    /// Predicate.
    pub p: SymbolId,
    /// Object — may be a symbol, literal value, or timestamp.
    pub o: Value,
    /// Grounding source.
    pub source: SymbolId,
    /// Stored confidence at write time.
    pub confidence: Confidence,
    /// When the fact became true in the world.
    pub valid_at: ClockTime,
}

/// An Episodic memory — an event at a point in time.
///
/// See `memory-type-taxonomy.md` § 3.2. `participants` is ontic (actors
/// in the event); `source` is epistemic (who witnessed or reported).
#[derive(Clone, Debug, PartialEq)]
pub struct Episodic {
    /// Stable memory ID for this event.
    pub event_id: SymbolId,
    /// Event-type tag (e.g. `@rename`, `@commit`, `@discussion`).
    pub kind: SymbolId,
    /// Actors in the event (ontic).
    pub participants: Vec<SymbolId>,
    /// Where the event occurred.
    pub location: SymbolId,
    /// When the event occurred.
    pub at_time: ClockTime,
    /// When the recording agent observed the event — distinct from
    /// `at_time`.
    pub observed_at: ClockTime,
    /// Witness / reporter (epistemic).
    pub source: SymbolId,
    /// Stored confidence at write time.
    pub confidence: Confidence,
}

/// A Procedural memory — a trigger-action rule that directs future
/// behavior.
///
/// See `memory-type-taxonomy.md` § 3.3.
#[derive(Clone, Debug, PartialEq)]
pub struct Procedural {
    /// Stable memory ID for this rule.
    pub rule_id: SymbolId,
    /// Trigger condition — typically a string describing the match.
    pub trigger: Value,
    /// Action to take on match.
    pub action: Value,
    /// Optional additional gating on the trigger.
    pub precondition: Option<Value>,
    /// Scope in which this rule applies (e.g. `@mimir_repo`).
    pub scope: SymbolId,
    /// Grounding source.
    pub source: SymbolId,
    /// Stored confidence at write time.
    pub confidence: Confidence,
}

/// An Inferential memory — a fact derived from other memories rather
/// than from an external source.
///
/// See `memory-type-taxonomy.md` § 3.4. `derived_from` is non-empty;
/// `method` resolves to a registered inference-method symbol.
#[derive(Clone, Debug, PartialEq)]
pub struct Inferential {
    /// Subject.
    pub s: SymbolId,
    /// Predicate.
    pub p: SymbolId,
    /// Object.
    pub o: Value,
    /// Parent memory IDs that this derivation depends on. Must be
    /// non-empty.
    pub derived_from: Vec<SymbolId>,
    /// Registered inference method (resolves to a Symbol of kind
    /// `InferenceMethod`).
    pub method: SymbolId,
    /// Stored confidence at derivation time.
    pub confidence: Confidence,
    /// When the derived claim is taken to hold true.
    pub valid_at: ClockTime,
}

/// A canonical memory kind.
///
/// Four variants, frozen per `memory-type-taxonomy.md` § 5. The enum is
/// **not** `#[non_exhaustive]`; adding a fifth type is a breaking
/// change under semver (`PRINCIPLES.md` § 10).
#[derive(Clone, Debug, PartialEq)]
pub enum MemoryKind {
    /// General facts about the world.
    Semantic(Semantic),
    /// Events at a point in time.
    Episodic(Episodic),
    /// Rules that direct future behavior.
    Procedural(Procedural),
    /// Facts derived from other memories.
    Inferential(Inferential),
}

/// A compact tag for the memory kind without the variant body.
///
/// Used in admission-check APIs like [`crate::SourceKind::admits`]
/// where only the tag is needed, not the full record.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum MemoryKindTag {
    /// Tag for [`Semantic`].
    Semantic,
    /// Tag for [`Episodic`].
    Episodic,
    /// Tag for [`Procedural`].
    Procedural,
    /// Tag for [`Inferential`].
    Inferential,
}

impl MemoryKind {
    /// Compact tag for this memory kind.
    #[must_use]
    pub const fn tag(&self) -> MemoryKindTag {
        match self {
            Self::Semantic(_) => MemoryKindTag::Semantic,
            Self::Episodic(_) => MemoryKindTag::Episodic,
            Self::Procedural(_) => MemoryKindTag::Procedural,
            Self::Inferential(_) => MemoryKindTag::Inferential,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock(millis: u64) -> ClockTime {
        ClockTime::try_from_millis(millis).expect("non-sentinel")
    }

    fn conf(raw: u16) -> Confidence {
        Confidence::from_u16(raw)
    }

    #[test]
    fn tag_reflects_variant() {
        let sem = MemoryKind::Semantic(Semantic {
            s: SymbolId::new(1),
            p: SymbolId::new(2),
            o: Value::String("x".into()),
            source: SymbolId::new(3),
            confidence: conf(62_258),
            valid_at: clock(1_000),
        });
        assert_eq!(sem.tag(), MemoryKindTag::Semantic);

        let epi = MemoryKind::Episodic(Episodic {
            event_id: SymbolId::new(10),
            kind: SymbolId::new(11),
            participants: vec![SymbolId::new(12)],
            location: SymbolId::new(13),
            at_time: clock(1_000),
            observed_at: clock(1_000),
            source: SymbolId::new(14),
            confidence: conf(u16::MAX),
        });
        assert_eq!(epi.tag(), MemoryKindTag::Episodic);

        let pro = MemoryKind::Procedural(Procedural {
            rule_id: SymbolId::new(20),
            trigger: Value::String("agent about to write".into()),
            action: Value::String("route via librarian".into()),
            precondition: None,
            scope: SymbolId::new(21),
            source: SymbolId::new(22),
            confidence: conf(u16::MAX),
        });
        assert_eq!(pro.tag(), MemoryKindTag::Procedural);

        let inf = MemoryKind::Inferential(Inferential {
            s: SymbolId::new(30),
            p: SymbolId::new(31),
            o: Value::Boolean(true),
            derived_from: vec![SymbolId::new(32), SymbolId::new(33)],
            method: SymbolId::new(34),
            confidence: conf(50_000),
            valid_at: clock(2_000),
        });
        assert_eq!(inf.tag(), MemoryKindTag::Inferential);
    }
}
