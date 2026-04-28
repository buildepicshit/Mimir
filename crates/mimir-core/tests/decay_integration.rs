//! Integration tests for `confidence-decay.md` § 1 graduation
//! criterion #4 — "user config overrides (`mimir.toml`) take effect
//! at runtime without librarian restart."
//!
//! The `toml_reload_changes_effective_confidence_without_restart`
//! equivalent also lives as a unit test inside `mimir_core::decay`;
//! this file re-exercises the same contract from across the crate
//! boundary so the graduation criterion is covered by a test that's
//! structurally an integration test (lives in `tests/`, not in the
//! module) in addition to the module-local one.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mimir_core::decay::{effective_confidence, DecayConfig, DecayFlags, HalfLife, DAY_MS};
use mimir_core::{Confidence, MemoryKindTag, SourceKind};

#[test]
fn toml_reload_accelerates_decay_without_restart() {
    // Baseline: 180-day Semantic/Observation default.
    let mut cfg = DecayConfig::librarian_defaults();
    let stored = Confidence::try_from_f32(1.0).expect("in range");
    let elapsed = 30 * DAY_MS;

    let before = effective_confidence(
        stored,
        elapsed,
        MemoryKindTag::Semantic,
        SourceKind::Observation,
        DecayFlags::default(),
        &cfg,
    );

    // Apply a user config that shortens the Semantic/Observation
    // half-life to a single day. No restart — same `cfg` instance,
    // mutated in place by `apply_toml`. The next effective_confidence
    // call must see the shorter half-life.
    let toml_override = r"
        [decay.semantic]
        observation = 1
    ";
    cfg.apply_toml(toml_override).expect("apply_toml");

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
        "reload failed to accelerate decay: before={before:?} after={after:?}"
    );

    // A third reload widening the half-life again must visibly slow
    // decay — proves reload is bidirectional, not a one-way shrink.
    let wider = r"
        [decay.semantic]
        observation = 3650
    ";
    cfg.apply_toml(wider).expect("apply_toml wider");

    let after_wider = effective_confidence(
        stored,
        elapsed,
        MemoryKindTag::Semantic,
        SourceKind::Observation,
        DecayFlags::default(),
        &cfg,
    );
    assert!(
        after_wider > after,
        "second reload failed to slow decay: after={after:?} after_wider={after_wider:?}"
    );
}

#[test]
fn toml_partial_override_preserves_other_defaults() {
    // Overriding ONE key must leave every other default untouched —
    // spec § 5.2 "unlisted keys fall back to librarian defaults".
    let mut cfg = DecayConfig::librarian_defaults();
    let profile_default = cfg.sem_profile;
    let document_default = cfg.sem_document;

    cfg.apply_toml("[decay.semantic]\nobservation = 7")
        .expect("apply");

    // Observation is overridden.
    assert_eq!(cfg.sem_observation, HalfLife::from_days(7));
    // Every other key unchanged.
    assert_eq!(cfg.sem_profile, profile_default);
    assert_eq!(cfg.sem_document, document_default);
}
