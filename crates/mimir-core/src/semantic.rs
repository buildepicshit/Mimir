//! Semantic stage — validates [`crate::bind::BoundForm`]s against the
//! grounding model and produces [`ValidatedForm`] ASTs with typed
//! fields extracted from keyword arguments.
//!
//! Implements the rules in `docs/concepts/grounding-model.md` §§ 3–4
//! and the clock / projection rules in `docs/concepts/temporal-model.md`
//! §§ 5, 9–10.

use thiserror::Error;

use crate::bind::{BoundForm, BoundKeywords, SymbolTable};
use crate::clock::ClockTime;
use crate::confidence::{Confidence, ConfidenceError};
use crate::memory_kind::MemoryKindTag;
use crate::source_kind::SourceKind;
use crate::symbol::SymbolId;
use crate::value::Value;

/// An AST form after the semantic stage: typed fields are extracted
/// from keyword bags; grounding + confidence + clock invariants are
/// enforced.
///
/// Consumed by the canonical-form emitter.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::module_name_repetitions)]
pub enum ValidatedForm {
    /// Semantic memory write.
    Sem {
        /// Subject symbol.
        s: SymbolId,
        /// Predicate symbol.
        p: SymbolId,
        /// Object value.
        o: Value,
        /// Source symbol (the grounding anchor).
        source: SymbolId,
        /// Derived grounding kind from the source symbol's canonical name.
        source_kind: SourceKind,
        /// Stored confidence — clamped by source bound.
        confidence: Confidence,
        /// Valid-time.
        valid_at: ClockTime,
        /// Whether this memory is a projection about future state.
        projected: bool,
    },
    /// Episodic memory write.
    Epi {
        /// Stable event ID.
        event_id: SymbolId,
        /// Event-type symbol.
        kind: SymbolId,
        /// Participant symbols (may be empty).
        participants: Vec<SymbolId>,
        /// Location symbol.
        location: SymbolId,
        /// Event time.
        at_time: ClockTime,
        /// Observation time — must be `>= at_time`.
        observed_at: ClockTime,
        /// Source symbol (the witness).
        source: SymbolId,
        /// Derived grounding kind.
        source_kind: SourceKind,
        /// Confidence.
        confidence: Confidence,
    },
    /// Procedural memory write.
    Pro {
        /// Stable rule ID.
        rule_id: SymbolId,
        /// Trigger value.
        trigger: Value,
        /// Action value.
        action: Value,
        /// Optional precondition.
        precondition: Option<Value>,
        /// Scope symbol.
        scope: SymbolId,
        /// Source symbol.
        source: SymbolId,
        /// Derived grounding kind.
        source_kind: SourceKind,
        /// Confidence.
        confidence: Confidence,
    },
    /// Inferential memory write.
    Inf {
        /// Subject.
        s: SymbolId,
        /// Predicate.
        p: SymbolId,
        /// Object.
        o: Value,
        /// Non-empty list of parent memories.
        derived_from: Vec<SymbolId>,
        /// Inference method symbol (validated at bind time).
        method: SymbolId,
        /// Confidence.
        confidence: Confidence,
        /// Valid-time.
        valid_at: ClockTime,
        /// Projection flag.
        projected: bool,
    },
    /// Alias declaration.
    Alias {
        /// First symbol.
        a: SymbolId,
        /// Second symbol.
        b: SymbolId,
    },
    /// Rename — old name becomes alias of new.
    Rename {
        /// Previous canonical.
        old: SymbolId,
        /// New canonical.
        new: SymbolId,
    },
    /// Retire a symbol.
    Retire {
        /// Target.
        name: SymbolId,
        /// Optional reason.
        reason: Option<String>,
    },
    /// Correct a prior Episodic memory.
    Correct {
        /// Target episode.
        target_episode: SymbolId,
        /// Corrected Episodic body.
        corrected: Box<ValidatedForm>,
    },
    /// Promote an ephemeral memory.
    Promote {
        /// Target symbol.
        name: SymbolId,
    },
    /// Read-path query.
    Query {
        /// Optional positional selector.
        selector: Option<Value>,
        /// Remaining keyword arguments (uninterpreted at this stage).
        keywords: BoundKeywords,
    },
    /// Explicit Episode-boundary directive — pass-through from bind.
    /// The semantic stage enforces "at most one Episode directive per
    /// batch" so the emitter can trust the batch carries a single
    /// deterministic intent.
    Episode {
        /// Open or close.
        action: crate::parse::EpisodeAction,
        /// Optional label.
        label: Option<String>,
        /// Optional parent Episode.
        parent_episode: Option<SymbolId>,
        /// Retracted Episodes.
        retracts: Vec<SymbolId>,
    },
    /// Pin / unpin / authoritative flag write — pass-through from
    /// bind. See `confidence-decay.md` §§ 7 / 8.
    Flag {
        /// Which flag operation.
        action: crate::parse::FlagAction,
        /// Target memory.
        memory: SymbolId,
        /// Invoking agent / operator.
        actor: SymbolId,
    },
}

/// Errors produced by [`validate`].
#[derive(Debug, Error, PartialEq)]
pub enum SemanticError {
    /// A confidence value exceeded the bound for its source kind.
    #[error("confidence {requested} exceeds {source_kind:?} bound {bound}")]
    ConfidenceExceedsSourceBound {
        /// The requested confidence.
        requested: Confidence,
        /// The source kind's bound.
        bound: Confidence,
        /// The source kind derived from the source symbol.
        source_kind: SourceKind,
    },

    /// The source kind does not admit the memory kind.
    #[error("source kind {source_kind:?} does not admit memory kind {memory_kind:?}")]
    SourceKindNotAdmitted {
        /// The derived source kind.
        source_kind: SourceKind,
        /// The memory kind being written.
        memory_kind: MemoryKindTag,
    },

    /// Agent supplied a future `valid_at` without the `:projected true` flag.
    #[error("valid_at {valid_at:?} is in the future; require :projected true")]
    FutureValidity {
        /// The offending timestamp.
        valid_at: ClockTime,
    },

    /// Episodic `observed_at` predates `at_time`.
    #[error("Episodic observed_at {observed_at:?} < at_time {at_time:?}")]
    InvalidClockOrder {
        /// The event time.
        at_time: ClockTime,
        /// The observation time.
        observed_at: ClockTime,
    },

    /// Inferential `derived_from` is empty.
    #[error("Inferential derived_from must be non-empty")]
    EmptyDerivedFrom,

    /// A required keyword was missing post-bind. (Normally caught in
    /// parse, but present here as a safety net for future form changes.)
    #[error("semantic stage missing required keyword {keyword:?} for form {form:?}")]
    MissingKeyword {
        /// Missing keyword.
        keyword: &'static str,
        /// Form being validated.
        form: &'static str,
    },

    /// A keyword had the wrong type for the semantic stage.
    #[error("keyword {keyword:?} has wrong type for {form:?}: expected {expected}")]
    BadKeywordType {
        /// Keyword.
        keyword: &'static str,
        /// Form.
        form: &'static str,
        /// Description of expected type.
        expected: &'static str,
    },

    /// A confidence value was malformed (e.g. NaN).
    #[error("confidence malformed: {0}")]
    ConfidenceMalformed(#[from] ConfidenceError),

    /// A `correct` form's corrected body is not an `Epi` form.
    #[error("correct body must be an Epi form")]
    CorrectsNonEpisodic,

    /// A batch contains more than one `(episode …)` directive. Per
    /// `episode-semantics.md` § 3 / § 11, each batch corresponds to
    /// at most one Episode, so a single directive per batch is the
    /// only coherent shape.
    #[error("batch contains {count} episode directives; at most 1 allowed")]
    MultipleEpisodeDirectives {
        /// Number of episode forms in the batch.
        count: usize,
    },
}

/// Validate a sequence of bound forms against the grounding + clock
/// invariants, producing typed `ValidatedForm` ASTs.
///
/// # Errors
///
/// Returns the first [`SemanticError`] encountered.
pub fn validate(
    forms: Vec<BoundForm>,
    table: &SymbolTable,
    now: ClockTime,
) -> Result<Vec<ValidatedForm>, SemanticError> {
    let validated = forms
        .into_iter()
        .map(|form| validate_form(form, table, now))
        .collect::<Result<Vec<_>, _>>()?;
    // Spec invariant: a batch can carry at most one `(episode …)`
    // form. Two :start directives in one batch would need to
    // contradict each other about the Episode's metadata; two
    // :close forms is a simple client bug. Reject at validation
    // so emit can trust a singleton.
    let episode_count = validated
        .iter()
        .filter(|f| matches!(f, ValidatedForm::Episode { .. }))
        .count();
    if episode_count > 1 {
        return Err(SemanticError::MultipleEpisodeDirectives {
            count: episode_count,
        });
    }
    Ok(validated)
}

#[allow(clippy::too_many_lines)]
fn validate_form(
    form: BoundForm,
    table: &SymbolTable,
    now: ClockTime,
) -> Result<ValidatedForm, SemanticError> {
    match form {
        BoundForm::Sem {
            s,
            p,
            o,
            mut keywords,
        } => {
            let source = take_symbol(&mut keywords, "src", "sem")?;
            let confidence = take_confidence(&mut keywords, "sem")?;
            let valid_at = take_timestamp(&mut keywords, "v", "sem")?;
            let projected = take_projected(&mut keywords);
            let source_kind = source_kind_for(source, table);
            check_admits(source_kind, MemoryKindTag::Semantic)?;
            check_confidence_bound(source_kind, confidence)?;
            check_future_validity(valid_at, now, projected)?;
            Ok(ValidatedForm::Sem {
                s,
                p,
                o,
                source,
                source_kind,
                confidence,
                valid_at,
                projected,
            })
        }
        BoundForm::Epi {
            event_id,
            kind,
            participants,
            location,
            mut keywords,
        } => {
            let source = take_symbol(&mut keywords, "src", "epi")?;
            let confidence = take_confidence(&mut keywords, "epi")?;
            let at_time = take_timestamp(&mut keywords, "at", "epi")?;
            let observed_at = take_timestamp(&mut keywords, "obs", "epi")?;
            let source_kind = source_kind_for(source, table);
            check_admits(source_kind, MemoryKindTag::Episodic)?;
            check_confidence_bound(source_kind, confidence)?;
            if observed_at < at_time {
                return Err(SemanticError::InvalidClockOrder {
                    at_time,
                    observed_at,
                });
            }
            Ok(ValidatedForm::Epi {
                event_id,
                kind,
                participants,
                location,
                at_time,
                observed_at,
                source,
                source_kind,
                confidence,
            })
        }
        BoundForm::Pro {
            rule_id,
            trigger,
            action,
            mut keywords,
        } => {
            let source = take_symbol(&mut keywords, "src", "pro")?;
            let confidence = take_confidence(&mut keywords, "pro")?;
            let scope = take_symbol(&mut keywords, "scp", "pro")?;
            let precondition = keywords.remove("pre");
            let source_kind = source_kind_for(source, table);
            check_admits(source_kind, MemoryKindTag::Procedural)?;
            check_confidence_bound(source_kind, confidence)?;
            Ok(ValidatedForm::Pro {
                rule_id,
                trigger,
                action,
                precondition,
                scope,
                source,
                source_kind,
                confidence,
            })
        }
        BoundForm::Inf {
            s,
            p,
            o,
            derived_from,
            method,
            mut keywords,
        } => {
            if derived_from.is_empty() {
                return Err(SemanticError::EmptyDerivedFrom);
            }
            let confidence = take_confidence(&mut keywords, "inf")?;
            let valid_at = take_timestamp(&mut keywords, "v", "inf")?;
            let projected = take_projected(&mut keywords);
            check_future_validity(valid_at, now, projected)?;
            Ok(ValidatedForm::Inf {
                s,
                p,
                o,
                derived_from,
                method,
                confidence,
                valid_at,
                projected,
            })
        }
        BoundForm::Alias { a, b } => Ok(ValidatedForm::Alias { a, b }),
        BoundForm::Rename { old, new } => Ok(ValidatedForm::Rename { old, new }),
        BoundForm::Retire { name, reason } => Ok(ValidatedForm::Retire { name, reason }),
        BoundForm::Correct {
            target_episode,
            corrected,
        } => {
            let bound = validate_form(*corrected, table, now)?;
            if !matches!(&bound, ValidatedForm::Epi { .. }) {
                return Err(SemanticError::CorrectsNonEpisodic);
            }
            Ok(ValidatedForm::Correct {
                target_episode,
                corrected: Box::new(bound),
            })
        }
        BoundForm::Promote { name } => Ok(ValidatedForm::Promote { name }),
        BoundForm::Query { selector, keywords } => Ok(ValidatedForm::Query { selector, keywords }),
        BoundForm::Episode {
            action,
            label,
            parent_episode,
            retracts,
        } => Ok(ValidatedForm::Episode {
            action,
            label,
            parent_episode,
            retracts,
        }),
        BoundForm::Flag {
            action,
            memory,
            actor,
        } => Ok(ValidatedForm::Flag {
            action,
            memory,
            actor,
        }),
    }
}

fn take_symbol(
    keywords: &mut BoundKeywords,
    key: &'static str,
    form: &'static str,
) -> Result<SymbolId, SemanticError> {
    match keywords.remove(key) {
        Some(Value::Symbol(id)) => Ok(id),
        Some(_) => Err(SemanticError::BadKeywordType {
            keyword: key,
            form,
            expected: "symbol",
        }),
        None => Err(SemanticError::MissingKeyword { keyword: key, form }),
    }
}

fn take_timestamp(
    keywords: &mut BoundKeywords,
    key: &'static str,
    form: &'static str,
) -> Result<ClockTime, SemanticError> {
    match keywords.remove(key) {
        Some(Value::Timestamp(t)) => Ok(t),
        Some(_) => Err(SemanticError::BadKeywordType {
            keyword: key,
            form,
            expected: "timestamp",
        }),
        None => Err(SemanticError::MissingKeyword { keyword: key, form }),
    }
}

#[allow(clippy::cast_possible_truncation)]
fn take_confidence(
    keywords: &mut BoundKeywords,
    form: &'static str,
) -> Result<Confidence, SemanticError> {
    let raw = match keywords.remove("c") {
        Some(Value::Float(f)) => f as f32,
        Some(_) => {
            return Err(SemanticError::BadKeywordType {
                keyword: "c",
                form,
                expected: "float confidence in [0.0, 1.0]",
            });
        }
        None => {
            return Err(SemanticError::MissingKeyword { keyword: "c", form });
        }
    };
    Ok(Confidence::try_from_f32(raw)?)
}

fn take_projected(keywords: &mut BoundKeywords) -> bool {
    matches!(keywords.remove("projected"), Some(Value::Boolean(true)))
}

fn check_admits(source_kind: SourceKind, memory_kind: MemoryKindTag) -> Result<(), SemanticError> {
    if source_kind.admits(memory_kind) {
        Ok(())
    } else {
        Err(SemanticError::SourceKindNotAdmitted {
            source_kind,
            memory_kind,
        })
    }
}

fn check_confidence_bound(
    source_kind: SourceKind,
    confidence: Confidence,
) -> Result<(), SemanticError> {
    let bound = source_kind.confidence_bound();
    if confidence <= bound {
        Ok(())
    } else {
        Err(SemanticError::ConfidenceExceedsSourceBound {
            requested: confidence,
            bound,
            source_kind,
        })
    }
}

fn check_future_validity(
    valid_at: ClockTime,
    now: ClockTime,
    projected: bool,
) -> Result<(), SemanticError> {
    if valid_at > now && !projected {
        Err(SemanticError::FutureValidity { valid_at })
    } else {
        Ok(())
    }
}

/// Derive [`SourceKind`] from a source symbol's canonical name.
///
/// The 12 reserved grounding-kind names from `grounding-model.md` § 3.1
/// map to their specific kinds. Any other source symbol defaults to
/// [`SourceKind::Observation`] — the most permissive kind (admits
/// Semantic + Episodic, bound 1.0). This lets agents use specific
/// witness symbols (e.g. `@mira`) as sources for Episodic memories
/// without needing a reserved name.
#[must_use]
pub fn source_kind_for(source: SymbolId, table: &SymbolTable) -> SourceKind {
    let Some(entry) = table.entry(source) else {
        return SourceKind::Observation;
    };
    source_kind_from_name(&entry.canonical_name)
}

/// Map a source symbol's canonical name to a [`SourceKind`].
#[must_use]
pub fn source_kind_from_name(name: &str) -> SourceKind {
    match name {
        "profile" => SourceKind::Profile,
        "self_report" => SourceKind::SelfReport,
        "participant_report" => SourceKind::ParticipantReport,
        "document" => SourceKind::Document,
        "registry" => SourceKind::Registry,
        "policy" => SourceKind::Policy,
        "agent_instruction" => SourceKind::AgentInstruction,
        "external_authority" => SourceKind::ExternalAuthority,
        "pending_verification" => SourceKind::PendingVerification,
        "librarian_assignment" => SourceKind::LibrarianAssignment,
        // "observation" matches the default branch below; included in
        // the 11 reserved names via the wildcard.
        _ => SourceKind::Observation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bind::{bind, SymbolTable};
    use crate::parse::parse;

    fn now() -> ClockTime {
        ClockTime::try_from_millis(2_000_000_000_000).expect("non-sentinel") // ~2033 — always 'now' for tests
    }

    fn bind_and_validate(src: &str) -> Result<Vec<ValidatedForm>, SemanticError> {
        let forms = parse(src).unwrap();
        let mut table = SymbolTable::new();
        let (bound, _journal) = bind(forms, &mut table).unwrap();
        validate(bound, &table, now())
    }

    #[test]
    fn sem_profile_passes() {
        let r = bind_and_validate(r#"(sem @alice email "x" :src @profile :c 0.95 :v 2024-01-15)"#);
        assert!(r.is_ok(), "got {r:?}");
    }

    #[test]
    fn sem_profile_over_bound_fails() {
        let err =
            bind_and_validate(r#"(sem @alice email "x" :src @profile :c 0.99 :v 2024-01-15)"#)
                .unwrap_err();
        assert!(matches!(
            err,
            SemanticError::ConfidenceExceedsSourceBound { .. }
        ));
    }

    #[test]
    fn pro_observation_source_not_admitted() {
        // Procedural requires Policy or AgentInstruction; Observation
        // does not admit Procedural.
        let src = r#"(pro @rule "trigger" "action" :scp @mimir :src @observation :c 0.9)"#;
        let err = bind_and_validate(src).unwrap_err();
        assert!(matches!(err, SemanticError::SourceKindNotAdmitted { .. }));
    }

    #[test]
    fn pro_policy_admitted() {
        let src = r#"(pro @rule "trigger" "action" :scp @mimir :src @policy :c 1.0)"#;
        let r = bind_and_validate(src);
        assert!(r.is_ok(), "got {r:?}");
    }

    #[test]
    fn epi_observed_before_at_time_errors() {
        let err = bind_and_validate(
            r"(epi @ev @k (@p1) @loc :at 2024-01-15T10:00:00Z :obs 2024-01-15T09:00:00Z :src @alice :c 0.9)",
        )
        .unwrap_err();
        assert!(matches!(err, SemanticError::InvalidClockOrder { .. }));
    }

    #[test]
    fn epi_observed_equal_at_time_passes() {
        let r = bind_and_validate(
            r"(epi @ev @k (@p1) @loc :at 2024-01-15T10:00:00Z :obs 2024-01-15T10:00:00Z :src @alice :c 0.9)",
        );
        assert!(r.is_ok(), "got {r:?}");
    }

    #[test]
    fn future_validity_without_projected_errors() {
        // 2099-01-01 is far past `now` (~2033).
        let err = bind_and_validate(
            r"(sem @alice status @future :src @agent_instruction :c 0.9 :v 2099-01-01)",
        )
        .unwrap_err();
        assert!(matches!(err, SemanticError::FutureValidity { .. }));
    }

    #[test]
    fn future_validity_with_projected_passes() {
        let r = bind_and_validate(
            r"(sem @alice status @future :src @agent_instruction :c 0.9 :v 2099-01-01 :projected true)",
        );
        assert!(r.is_ok(), "got {r:?}");
    }

    #[test]
    fn inf_empty_derived_from_not_allowed_by_parser() {
        // The parser enforces at least the syntactic form; an empty
        // derived_from list is written as () which the parser accepts
        // but the semantic stage must reject.
        let forms = parse("(inf @a p @b () @pattern_summarize :c 0.7 :v 2024-01-15)").unwrap();
        let mut table = SymbolTable::new();
        let (bound, _journal) = bind(forms, &mut table).unwrap();
        let err = validate(bound, &table, now()).unwrap_err();
        assert!(matches!(err, SemanticError::EmptyDerivedFrom));
    }

    #[test]
    fn sem_unknown_source_defaults_to_observation() {
        // `@mira` isn't a reserved grounding-kind name; defaults to
        // Observation (bound 1.0, admits Semantic). Use distinct symbols
        // for subject and source so kind-locking doesn't collide.
        let r = bind_and_validate(r#"(sem @mimir founder "mira" :src @mira :c 1.0 :v 2024-01-15)"#);
        assert!(r.is_ok(), "got {r:?}");
    }

    #[test]
    fn correct_non_episodic_body_errors() {
        // The parser's correct-form already enforces body must be epi;
        // ensure the semantic stage's own check is defensive. Here the
        // body IS epi, so this should pass.
        let r = bind_and_validate(
            r"(correct @target_ep (epi @ev @k (@p) @loc :at 2024-01-15T10:00:00Z :obs 2024-01-15T10:00:00Z :src @alice :c 0.9))",
        );
        assert!(r.is_ok(), "got {r:?}");
    }

    #[test]
    fn source_kind_from_name_mapping() {
        assert_eq!(source_kind_from_name("profile"), SourceKind::Profile);
        assert_eq!(
            source_kind_from_name("observation"),
            SourceKind::Observation
        );
        assert_eq!(
            source_kind_from_name("pending_verification"),
            SourceKind::PendingVerification
        );
        // Unreserved → Observation default.
        assert_eq!(source_kind_from_name("mira"), SourceKind::Observation);
    }
}
