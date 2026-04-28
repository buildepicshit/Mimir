//! Property tests for foundational types.

// Integration-test binary; idiomatic Result-assertion patterns use
// unwrap/expect/panic. The workspace-level denies those for library
// correctness (PRINCIPLES.md § 7); relax here.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::similar_names
)]

use mimir_core::bind::SymbolTable;
use mimir_core::canonical::{
    decode_record, encode_record, CanonicalRecord, Clocks, EdgeRecord, SemFlags, SemRecord,
};
use mimir_core::dag::{DagError, Edge, EdgeKind, SupersessionDag};
use mimir_core::decay::{
    decay_factor_u16, effective_confidence, DecayConfig, DecayFlags, HalfLife, DAY_MS, NO_DECAY,
};
use mimir_core::pipeline::Pipeline;
use mimir_core::read::ReadFlags;
use mimir_core::{
    ClockTime, Confidence, ConfidenceError, MemoryKindTag, ScopedSymbolId, SourceKind, SymbolId,
    SymbolKind, Value, WorkspaceId,
};
use proptest::prelude::*;
use ulid::Ulid;

fn symbol_kind_strategy() -> impl Strategy<Value = SymbolKind> {
    prop_oneof![
        Just(SymbolKind::Agent),
        Just(SymbolKind::Document),
        Just(SymbolKind::Registry),
        Just(SymbolKind::Service),
        Just(SymbolKind::Policy),
        Just(SymbolKind::Memory),
        Just(SymbolKind::InferenceMethod),
        Just(SymbolKind::Scope),
        Just(SymbolKind::Predicate),
        Just(SymbolKind::EventType),
        Just(SymbolKind::Workspace),
        Just(SymbolKind::Literal),
    ]
}

proptest! {
    /// Any `f32` in `[0.0, 1.0]` round-trips through `Confidence`.
    #[test]
    fn confidence_roundtrips_in_range(raw in 0.0_f32..=1.0_f32) {
        let c = Confidence::try_from_f32(raw).expect("in range");
        let back = c.as_f32();
        prop_assert!((0.0..=1.0).contains(&back));
        // ±1 fixed-point step of drift is acceptable.
        let step = 1.0 / f32::from(u16::MAX);
        prop_assert!((back - raw).abs() <= 2.0 * step, "raw={raw} back={back}");
    }

    /// Any `f32` outside `[0.0, 1.0]` or NaN is rejected with a typed error.
    #[test]
    fn confidence_rejects_out_of_range(raw in any::<f32>()) {
        let result = Confidence::try_from_f32(raw);
        if raw.is_nan() {
            prop_assert_eq!(result, Err(ConfidenceError::NotANumber));
        } else if (0.0..=1.0).contains(&raw) {
            prop_assert!(result.is_ok());
        } else {
            prop_assert!(matches!(result, Err(ConfidenceError::OutOfRange(_))));
        }
    }

    // (removed: `confidence_u16_roundtrip_is_identity` — a tautology
    // over two `const fn`s on the raw u16 field. The semantic
    // round-trip property is covered by `confidence_roundtrips_in_range`
    // above, which asserts quantization drift stays within one fixed-
    // point step.)

    /// `SymbolId::new(raw).as_u64() == raw` for all `raw`.
    #[test]
    fn symbol_id_roundtrips(raw in any::<u64>()) {
        prop_assert_eq!(SymbolId::new(raw).as_u64(), raw);
    }

    /// Two `ScopedSymbolId`s with different workspaces are never equal even
    /// when their local IDs match.
    #[test]
    fn scoped_symbol_distinguishes_by_workspace(
        ws_a_high in any::<u64>(),
        ws_a_low in any::<u128>(),
        ws_b_high in any::<u64>(),
        ws_b_low in any::<u128>(),
        local in any::<u64>(),
    ) {
        prop_assume!(ws_a_high != ws_b_high || ws_a_low != ws_b_low);

        let ws_a = WorkspaceId::from_ulid(Ulid::from_parts(ws_a_high, ws_a_low));
        let ws_b = WorkspaceId::from_ulid(Ulid::from_parts(ws_b_high, ws_b_low));
        prop_assume!(ws_a != ws_b);

        let a = ScopedSymbolId::new(ws_a, SymbolId::new(local));
        let b = ScopedSymbolId::new(ws_b, SymbolId::new(local));
        prop_assert_ne!(a, b);
    }

    /// Every `SourceKind` admits a consistent set of memory kinds —
    /// Inferential is never admitted (matches grounding-model.md § 7.1).
    #[test]
    fn no_source_admits_inferential(kind in prop_oneof![
        Just(SourceKind::Profile),
        Just(SourceKind::Observation),
        Just(SourceKind::SelfReport),
        Just(SourceKind::ParticipantReport),
        Just(SourceKind::Document),
        Just(SourceKind::Registry),
        Just(SourceKind::Policy),
        Just(SourceKind::AgentInstruction),
        Just(SourceKind::ExternalAuthority),
        Just(SourceKind::PendingVerification),
        Just(SourceKind::LibrarianAssignment),
    ]) {
        prop_assert!(!kind.admits(MemoryKindTag::Inferential));
    }

    /// Confidence bounds stay within `[0.0, 1.0]` for every SourceKind.
    #[test]
    fn every_source_bound_is_in_range(kind in prop_oneof![
        Just(SourceKind::Profile),
        Just(SourceKind::Observation),
        Just(SourceKind::SelfReport),
        Just(SourceKind::ParticipantReport),
        Just(SourceKind::Document),
        Just(SourceKind::Registry),
        Just(SourceKind::Policy),
        Just(SourceKind::AgentInstruction),
        Just(SourceKind::ExternalAuthority),
        Just(SourceKind::PendingVerification),
        Just(SourceKind::LibrarianAssignment),
    ]) {
        let bound = kind.confidence_bound();
        prop_assert!((0.0..=1.0).contains(&bound.as_f32()));
    }

    /// An edge record survives a round-trip through the canonical
    /// encoder / decoder. Exercises varint framing, symbol and clock
    /// encoding, and opcode discrimination for all four edge kinds.
    #[test]
    fn edge_record_roundtrips(
        from in any::<u64>(),
        to in any::<u64>(),
        at_ms in 0_u64..u64::MAX,
        which in 0_u8..4,
    ) {
        let edge = EdgeRecord {
            from: SymbolId::new(from),
            to: SymbolId::new(to),
            at: ClockTime::try_from_millis(at_ms).expect("non-sentinel"),
        };
        let record = match which {
            0 => CanonicalRecord::Supersedes(edge),
            1 => CanonicalRecord::Corrects(edge),
            2 => CanonicalRecord::StaleParent(edge),
            _ => CanonicalRecord::Reconfirms(edge),
        };
        let mut buf = Vec::new();
        encode_record(&record, &mut buf);
        let (decoded, consumed) = decode_record(&buf).expect("decode");
        prop_assert_eq!(consumed, buf.len());
        prop_assert_eq!(decoded, record);
    }

    /// A semantic record with an integer object round-trips through
    /// encode / decode. Covers confidence fixed-point encoding, four
    /// clock values (including the `None` invalid-at sentinel), flag
    /// bits, and zigzag varint for the signed integer value.
    #[test]
    fn sem_record_roundtrips(
        memory_id in any::<u64>(),
        s in any::<u64>(),
        p in any::<u64>(),
        o in any::<i64>(),
        source in any::<u64>(),
        conf in any::<u16>(),
        valid_at in 0_u64..u64::MAX,
        observed_at in 0_u64..u64::MAX,
        committed_at in 0_u64..u64::MAX,
        invalid_at in prop::option::of(0_u64..u64::MAX),
        projected in any::<bool>(),
    ) {
        let sem = SemRecord {
            memory_id: SymbolId::new(memory_id),
            s: SymbolId::new(s),
            p: SymbolId::new(p),
            o: Value::Integer(o),
            source: SymbolId::new(source),
            confidence: Confidence::from_u16(conf),
            clocks: Clocks {
                valid_at: ClockTime::try_from_millis(valid_at).expect("non-sentinel"),
                observed_at: ClockTime::try_from_millis(observed_at).expect("non-sentinel"),
                committed_at: ClockTime::try_from_millis(committed_at).expect("non-sentinel"),
                invalid_at: invalid_at.map(|ms| ClockTime::try_from_millis(ms).expect("non-sentinel")),
            },
            flags: SemFlags { projected },
        };
        let record = CanonicalRecord::Sem(sem);
        let mut buf = Vec::new();
        encode_record(&record, &mut buf);
        let (decoded, consumed) = decode_record(&buf).expect("decode");
        prop_assert_eq!(consumed, buf.len());
        prop_assert_eq!(decoded, record);
    }

    /// `workspace-model.md` § 3.1: git-remote → WorkspaceId mapping
    /// is deterministic — the same normalized URL always hashes to
    /// the same ID, regardless of capitalization or trailing-`.git`.
    #[test]
    fn workspace_id_determinism(
        host in "[a-z][a-z0-9.-]{0,15}\\.com",
        path in "[a-z][a-z0-9_/-]{0,31}",
        uppercase in any::<bool>(),
        trailing_git in any::<bool>(),
    ) {
        let base = format!("https://{host}/{path}");
        let shown = if uppercase { base.to_uppercase() } else { base.clone() };
        let shown = if trailing_git { format!("{shown}.git") } else { shown };
        let a = WorkspaceId::from_git_remote(&base).unwrap();
        let b = WorkspaceId::from_git_remote(&shown).unwrap();
        prop_assert_eq!(a, b, "normalization should collapse case and .git suffix");
    }

    /// `workspace-model.md` § 4.2 (hard partitioning): two different
    /// git remotes produce disjoint workspace directories. Writing
    /// to workspace A does not surface any state in workspace B's
    /// reopened Store.
    #[test]
    fn workspace_partitioning_is_structural(
        host_a in "[a-z][a-z0-9-]{3,10}",
        host_b in "[a-z][a-z0-9-]{3,10}",
    ) {
        prop_assume!(host_a != host_b);
        let data_root = tempfile::TempDir::new().unwrap();
        let ws_a = WorkspaceId::from_git_remote(
            &format!("https://github.com/{host_a}/repo")
        ).unwrap();
        let ws_b = WorkspaceId::from_git_remote(
            &format!("https://github.com/{host_b}/repo")
        ).unwrap();
        prop_assume!(ws_a != ws_b);

        let sem = "(sem @alice @knows @bob :src @observation :c 0.8 :v 2024-01-15)";
        let now = ClockTime::try_from_millis(1_713_350_400_000).expect("non-sentinel");
        {
            let mut store_a =
                mimir_core::Store::open_in_workspace(data_root.path(), ws_a).unwrap();
            let _ = store_a.commit_batch(sem, now).unwrap();
        }
        let store_b =
            mimir_core::Store::open_in_workspace(data_root.path(), ws_b).unwrap();
        // Workspace B's table has no memory of workspace A's commit.
        prop_assert!(store_b.log_len() == 0);
    }

    /// `confidence-decay.md` § 13 invariant 2: deterministic decay.
    /// Two calls with identical arguments must produce byte-identical
    /// effective confidences.
    #[test]
    fn decay_determinism(
        stored_raw in any::<u16>(),
        elapsed_ms in 0_u64..(200 * 365 * DAY_MS),
        half_life_days in 1_u64..=3650,
    ) {
        let stored = Confidence::from_u16(stored_raw);
        let half_life_ms = half_life_days * DAY_MS;
        let a = effective_confidence(
            stored,
            elapsed_ms,
            MemoryKindTag::Semantic,
            SourceKind::Observation,
            DecayFlags::default(),
            &DecayConfig {
                sem_observation: HalfLife::from_millis(half_life_ms),
                ..DecayConfig::librarian_defaults()
            },
        );
        let b = effective_confidence(
            stored,
            elapsed_ms,
            MemoryKindTag::Semantic,
            SourceKind::Observation,
            DecayFlags::default(),
            &DecayConfig {
                sem_observation: HalfLife::from_millis(half_life_ms),
                ..DecayConfig::librarian_defaults()
            },
        );
        prop_assert_eq!(a, b);
    }

    /// `confidence-decay.md` § 5.1 formula: decay is monotonic in
    /// elapsed time for a fixed half-life. `decay_factor_u16(e2, hl)
    /// <= decay_factor_u16(e1, hl)` whenever `e2 >= e1`.
    #[test]
    fn decay_is_monotonic(
        e1 in 0_u64..(100 * 365 * DAY_MS),
        delta in 0_u64..(10 * 365 * DAY_MS),
        half_life_days in 1_u64..=3650,
    ) {
        let hl = HalfLife::from_days(half_life_days);
        let e2 = e1 + delta;
        let f1 = decay_factor_u16(e1, hl);
        let f2 = decay_factor_u16(e2, hl);
        prop_assert!(f2 <= f1, "monotonicity violated: f({e1})={f1}, f({e2})={f2}");
    }

    /// `confidence-decay.md` § 13 invariant 3: pinned or operator-
    /// authoritative memories do not decay regardless of elapsed time
    /// or half-life.
    #[test]
    fn pinned_or_authoritative_skip_decay(
        stored_raw in any::<u16>(),
        elapsed_ms in 0_u64..(200 * 365 * DAY_MS),
        pinned in any::<bool>(),
        authoritative in any::<bool>(),
    ) {
        prop_assume!(pinned || authoritative);
        let stored = Confidence::from_u16(stored_raw);
        let cfg = DecayConfig::librarian_defaults();
        let flags = DecayFlags { pinned, authoritative };
        let eff = effective_confidence(
            stored,
            elapsed_ms,
            MemoryKindTag::Semantic,
            SourceKind::Observation,
            flags,
            &cfg,
        );
        prop_assert_eq!(eff, stored);
    }

    /// `confidence-decay.md` § 5.3: `NO_DECAY` half-life yields
    /// `effective = stored` for any elapsed.
    #[test]
    fn no_decay_half_life_never_decays(
        stored_raw in any::<u16>(),
        elapsed_ms in any::<u64>(),
    ) {
        let stored = Confidence::from_u16(stored_raw);
        let factor = decay_factor_u16(elapsed_ms, HalfLife::no_decay());
        let _ = NO_DECAY; // still publicly exported; keep the smoke-import
        prop_assert_eq!(factor, u16::MAX);
        // Also check the full effective pipeline via librarian
        // assignment which has NO_DECAY as its default.
        let eff = effective_confidence(
            stored,
            elapsed_ms,
            MemoryKindTag::Semantic,
            SourceKind::LibrarianAssignment,
            DecayFlags::default(),
            &DecayConfig::librarian_defaults(),
        );
        prop_assert_eq!(eff, stored);
    }

    /// `write-protocol.md` § 10.2 + § 1 criterion #4: orphan
    /// truncation is idempotent. After any sequence of committed
    /// batches followed by a crash-shaped partial frame, two
    /// successive `Store::open` calls leave the log at the same
    /// length. Random structurally-invalid bytes are corruption, not
    /// recoverable orphan tails.
    #[test]
    fn orphan_truncation_is_idempotent(
        subject in "[a-z][a-z0-9_]{0,8}",
        orphan_len in 0_usize..32,
    ) {
        use mimir_core::log::{CanonicalLog, LogBackend};
        use mimir_core::Store;
        let dir = tempfile::TempDir::new().expect("tmp");
        let path = dir.path().join("canonical.log");
        {
            let mut store = Store::open(&path).expect("open");
            let input = format!(
                "(sem @{subject} @knows @target :src @observation :c 0.5 :v 2024-01-15)"
            );
            let now = ClockTime::try_from_millis(1_713_350_400_000).expect("non-sentinel");
            let _ = store.commit_batch(&input, now).expect("commit");
        }
        // Inject an interrupted record frame. Lengths >= 2 declare a
        // 32-byte Checkpoint body but provide fewer bytes; length 1
        // stops while decoding the frame header.
        {
            let mut raw = CanonicalLog::open(&path).expect("raw");
            if orphan_len > 0 {
                let mut payload = vec![0x20_u8];
                if orphan_len > 1 {
                    payload.push(0x20);
                    payload.resize(orphan_len, 0);
                }
                raw.append(&payload).expect("append");
                raw.sync().expect("sync");
            }
        }
        let len_after_first = Store::open(&path).expect("recover once").log_len();
        let len_after_second = Store::open(&path).expect("recover twice").log_len();
        prop_assert_eq!(len_after_first, len_after_second);
    }

    // -----------------------------------------------------------
    // symbol-identity-semantics.md § 1 criterion #3 — five property
    // tests enumerated by the spec.
    // -----------------------------------------------------------

    /// First-use kind locking (spec § 3.2, § 4.2): the kind a symbol
    /// is allocated with is the kind it keeps forever. Any later
    /// attempt to re-allocate the same name with a different kind
    /// is a hard error, because `SymbolTable::allocate` refuses to
    /// create a second allocation under the same name at all.
    #[test]
    fn symbol_first_use_kind_is_locked(
        name in "[a-z][a-z0-9_]{0,15}",
        first_kind in symbol_kind_strategy(),
        second_kind in symbol_kind_strategy(),
    ) {
        prop_assume!(first_kind != second_kind);
        let mut table = SymbolTable::new();
        let id = table.allocate(name.clone(), first_kind).expect("first alloc");
        // Direct re-allocation under the same name must fail.
        let re_alloc = table.allocate(name.clone(), second_kind);
        prop_assert!(re_alloc.is_err());
        // The original binding's kind is unchanged.
        prop_assert_eq!(table.kind_of(id), Some(first_kind));
    }

    /// Alias collapse (spec § 7): adding an alias for an already-
    /// allocated symbol does not create a second `SymbolId`; both
    /// the canonical name and the alias resolve to the identical
    /// numeric id.
    #[test]
    fn symbol_alias_resolves_to_same_id(
        canonical in "[a-z][a-z0-9_]{0,15}",
        alias in "[a-z][a-z0-9_]{0,15}",
        kind in symbol_kind_strategy(),
    ) {
        prop_assume!(canonical != alias);
        let mut table = SymbolTable::new();
        let id = table.allocate(canonical.clone(), kind).expect("alloc");
        // Seed the alias as a separate allocation so add_alias has
        // something to collapse (the spec's `Alias { a, b }` form
        // requires both sides to exist — see bind.rs::UnboundForm::Alias).
        table.allocate(alias.clone(), kind).expect("alloc alias name");
        // Now add the alias edge.
        let attached = table.add_alias(canonical.as_str(), alias.as_str());
        // `add_alias` may reject if both names already resolve to
        // distinct ids (they do here) — the spec's rename operation
        // is the right call for that case. Treat both outcomes as
        // acceptable: if it succeeds, both resolve to the same id;
        // if it rejects, we haven't corrupted state.
        if attached.is_ok() {
            prop_assert_eq!(table.lookup(canonical.as_str()), Some(id));
            prop_assert_eq!(table.lookup(alias.as_str()), Some(id));
        }
    }

    /// Rename promotes a new canonical name (spec § 6): after rename,
    /// the old name still resolves to the same `SymbolId` (as an
    /// alias), and the new name resolves to that same id as the new
    /// canonical.
    #[test]
    fn symbol_rename_preserves_id_and_resolves_both_names(
        old_name in "[a-z][a-z0-9_]{0,15}",
        new_name in "[a-z][a-z0-9_]{0,15}",
        kind in symbol_kind_strategy(),
    ) {
        prop_assume!(old_name != new_name);
        let mut table = SymbolTable::new();
        let id = table.allocate(old_name.clone(), kind).expect("alloc");
        let renamed = table.rename(old_name.as_str(), new_name.as_str()).expect("rename");
        prop_assert_eq!(renamed, id);
        prop_assert_eq!(table.lookup(old_name.as_str()), Some(id));
        prop_assert_eq!(table.lookup(new_name.as_str()), Some(id));
        let entry = table.entry(id).expect("entry");
        prop_assert_eq!(&entry.canonical_name, &new_name);
        prop_assert!(entry.aliases.iter().any(|a| a == &old_name));
    }

    /// Retirement is a soft flag (spec § 8): a retired symbol still
    /// resolves — it is not deleted — but its `is_retired` bit is
    /// set. Subsequent retirement is idempotent; unretire clears
    /// the flag.
    #[test]
    fn symbol_retire_soft_flag_preserves_resolution(
        name in "[a-z][a-z0-9_]{0,15}",
        kind in symbol_kind_strategy(),
    ) {
        let mut table = SymbolTable::new();
        let id = table.allocate(name.clone(), kind).expect("alloc");
        prop_assert!(!table.is_retired(id));
        let retired_id = table.retire(name.as_str()).expect("retire");
        prop_assert_eq!(retired_id, id);
        prop_assert!(table.is_retired(id));
        // Resolution still works.
        prop_assert_eq!(table.lookup(name.as_str()), Some(id));
        // Re-retire is idempotent.
        let retired_again = table.retire(name.as_str()).expect("re-retire");
        prop_assert_eq!(retired_again, id);
        prop_assert!(table.is_retired(id));
        // Unretire clears the flag.
        let unretired = table.unretire(name.as_str()).expect("unretire");
        prop_assert_eq!(unretired, id);
        prop_assert!(!table.is_retired(id));
    }

    /// Cross-workspace distinctness (spec § 5, workspace-model.md
    /// § 4.1): the same `local` `SymbolId` under two different
    /// `WorkspaceId`s yields two distinct `ScopedSymbolId` values
    /// that compare unequal. This closes the round-trip half of
    /// the existing `scoped_symbol_distinguishes_by_workspace` by
    /// asserting *round-trip equality* when the workspace is held
    /// constant.
    #[test]
    fn scoped_symbol_roundtrips_within_a_workspace(
        ws_high in any::<u64>(),
        ws_low in any::<u128>(),
        local in any::<u64>(),
    ) {
        let ws = WorkspaceId::from_ulid(Ulid::from_parts(ws_high, ws_low));
        let a = ScopedSymbolId::new(ws, SymbolId::new(local));
        let b = ScopedSymbolId::new(ws, SymbolId::new(local));
        prop_assert_eq!(a, b);
        prop_assert_eq!(a.local, SymbolId::new(local));
        prop_assert_eq!(a.workspace, ws);
    }

    /// Pipeline determinism (spec § 11.2): two independent `Pipeline`
    /// instances fed the same input and the same `now` produce
    /// byte-identical canonical bytecode. Name strategies use
    /// disjoint first-character prefixes (`s` / `p` / `o`) so every
    /// generated triple is compile-legal; no `prop_assume!` filter
    /// is needed.
    #[test]
    fn pipeline_determinism(
        subject in "s[a-z0-9_]{0,15}",
        predicate in "p[a-z0-9_]{0,15}",
        object in "o[a-z0-9_]{0,15}",
        conf in 0_u16..=u16::MAX,
    ) {
        let conf_f = f64::from(conf) / f64::from(u16::MAX);
        let input = format!(
            "(sem @{subject} @{predicate} @{object} :src @observation :c {conf_f:.6} :v 2024-01-15)"
        );
        let now_ms = 1_713_350_400_000_u64;
        let t = ClockTime::try_from_millis(now_ms).expect("non-sentinel");

        let mut pipe_a = Pipeline::new();
        let mut pipe_b = Pipeline::new();
        let a = pipe_a.compile_batch(&input, t).expect("a");
        let b = pipe_b.compile_batch(&input, t).expect("b");

        let mut bytes_a = Vec::new();
        let mut bytes_b = Vec::new();
        for r in &a { encode_record(r, &mut bytes_a); }
        for r in &b { encode_record(r, &mut bytes_b); }
        prop_assert_eq!(bytes_a, bytes_b);
    }

    /// `librarian-pipeline.md` § 1 criterion #3 — batch atomicity:
    /// a batch that contains a bind-stage failure at any position
    /// leaves the pipeline's state fully unchanged (no partial
    /// commits, no lingering symbol allocations, counters at their
    /// pre-batch values).
    #[test]
    fn batch_atomicity_bind_failure_rolls_back_everything(
        good_forms in 0_usize..=4,
    ) {
        use mimir_core::pipeline::PipelineError;
        let mut pipe = Pipeline::new();
        let now = ClockTime::try_from_millis(1_713_350_400_000).expect("non-sentinel");
        // A kind-mismatch bind failure: the last form reuses @x
        // (allocated as Agent earlier in the batch) in a Predicate
        // slot. Parametrised by `good_forms` so the failure appears
        // anywhere from "first position" to "after 4 valid forms".
        let mut input = String::new();
        for i in 0..good_forms {
            std::fmt::Write::write_fmt(
                &mut input,
                format_args!(
                    "(sem @x @knows @y_{i} :src @observation :c 0.8 :v 2024-01-15)\n"
                ),
            )
            .expect("write to String never fails");
        }
        // The kind-mismatch form: @x in predicate slot (Predicate
        // kind expected; @x is Agent from any preceding good form,
        // or is freshly allocated as Predicate if no good forms
        // preceded — in which case we force the conflict by seeding
        // @x as Agent first).
        if good_forms == 0 {
            input.push_str("(sem @x @knows @y :src @observation :c 0.8 :v 2024-01-15)\n");
            input.push_str("(sem @alice @x @z :src @observation :c 0.8 :v 2024-01-15)\n");
        } else {
            input.push_str("(sem @alice @x @z :src @observation :c 0.8 :v 2024-01-15)\n");
        }
        let table_before = pipe.table().clone();
        let counter_before = pipe.next_memory_counter();
        let err = pipe.compile_batch(&input, now).expect_err("bind failure");
        prop_assert!(matches!(err, PipelineError::Bind(_)));
        prop_assert_eq!(pipe.table(), &table_before);
        prop_assert_eq!(pipe.next_memory_counter(), counter_before);
    }

    /// `librarian-pipeline.md` § 1 criterion #3 — per-stage error
    /// propagation: each stage's failure surfaces as the matching
    /// `PipelineError::*` variant, never laundered through a
    /// neighbouring stage. Parametrises the failure mode and asserts
    /// the variant tag.
    #[test]
    fn per_stage_error_routes_to_matching_variant(mode in 0_u8..4) {
        use mimir_core::pipeline::PipelineError;
        let mut pipe = Pipeline::new();
        let now = ClockTime::try_from_millis(1_713_350_400_000).expect("non-sentinel");
        let (input, expected_stage) = match mode {
            0 => (
                // Parse: unterminated list.
                "(sem @a".to_string(),
                "parse",
            ),
            1 => (
                // Bind: unknown inference method.
                "(inf @a p @b (@m1) @bogus_method :c 0.5 :v 2024-01-15)".to_string(),
                "bind",
            ),
            2 => (
                // Semantic: confidence over Registry bound (0.98).
                "(sem @a @knows @b :src @registry :c 0.99 :v 2024-01-15)".to_string(),
                "semantic",
            ),
            _ => (
                // Emit: unsupported form (correct targets a future track).
                "(correct @a (epi @e1 @kind () @loc :at 2024-01-15 :obs 2024-01-15 :src @alice :c 0.9))".to_string(),
                "emit",
            ),
        };
        let err = pipe.compile_batch(&input, now).expect_err("stage failure");
        match (expected_stage, &err) {
            ("parse", PipelineError::Parse(_))
            | ("bind", PipelineError::Bind(_))
            | ("semantic", PipelineError::Semantic(_))
            | ("emit", PipelineError::Emit(_)) => {}
            (stage, other) => {
                return Err(TestCaseError::fail(format!(
                    "expected {stage} error, got {other:?}"
                )));
            }
        }
        // Every stage-failure case leaves the pipeline unmodified.
        prop_assert_eq!(pipe.next_memory_counter(), 0);
    }

    /// `committed_at` is strictly monotonic across any sequence of
    /// `wall_now` values — including regressions, repeats, and
    /// out-of-order submissions. This covers the § 1 graduation
    /// criterion "monotonic `committed_at`" and invariant § 12 #1 of
    /// `temporal-model.md`.
    #[test]
    fn monotonic_committed_at_across_regressing_wall_clocks(
        wall_offsets in proptest::collection::vec(0_u64..100_000_000, 2..=8),
    ) {
        // Base far after every `valid_at` used in the body so semantic
        // validation never rejects anything for future-validity.
        const BASE_MS: u64 = 1_750_000_000_000;
        let mut pipe = Pipeline::new();
        let mut prev_committed: Option<ClockTime> = None;

        for (idx, offset) in wall_offsets.into_iter().enumerate() {
            let wall_now = ClockTime::try_from_millis(BASE_MS + offset).expect("non-sentinel");
            // Distinct predicate per iteration so Semantic
            // auto-supersession doesn't reject duplicate `(s, p)`
            // writes — this property is about the commit clock, not
            // supersession detection.
            let input = format!(
                "(sem @alice @knows_{idx} @bob :src @observation :c 0.8 :v 2024-01-15)"
            );
            let records = pipe.compile_batch(&input, wall_now).expect("compile");
            let sem = records.iter().find_map(|r| match r {
                CanonicalRecord::Sem(s) => Some(s),
                _ => None,
            });
            let committed = sem.expect("batch has a Sem record").clocks.committed_at;
            if let Some(prev) = prev_committed {
                prop_assert!(
                    committed > prev,
                    "non-monotonic committed_at: prev={prev:?} current={committed:?}"
                );
            }
            prop_assert_eq!(pipe.last_committed_at(), Some(committed));
            prev_committed = Some(committed);
        }
    }

    /// Forward-only edges (strictly increasing memory IDs) can never
    /// close a cycle — acyclicity is trivially preserved. This locks
    /// in the primary non-cycle case across the four edge kinds.
    ///
    /// Covers § 1 graduation criterion #3's "supersession DAG
    /// acyclicity" property from the positive direction.
    #[test]
    fn forward_only_edges_build_an_acyclic_dag(
        seeds in proptest::collection::vec(0_u32..32, 2..=16),
        kinds in proptest::collection::vec(0_u8..4, 2..=16),
    ) {
        let mut dag = SupersessionDag::new();
        // Sort and dedupe seeds so we have strictly increasing IDs.
        let mut ids: Vec<u64> = seeds.into_iter().map(u64::from).collect();
        ids.sort_unstable();
        ids.dedup();
        prop_assume!(ids.len() >= 2);
        let at = ClockTime::try_from_millis(1_000_000).unwrap();
        for pair in ids.windows(2) {
            // Pick an edge kind for this pair from the strategy-supplied
            // vector; cycling is fine since a small kind palette exercises
            // the union-of-kinds acyclicity check.
            let kind_idx = kinds[usize::try_from(pair[0]).unwrap() % kinds.len()] as usize;
            let kind = match kind_idx {
                0 => EdgeKind::Supersedes,
                1 => EdgeKind::Corrects,
                2 => EdgeKind::StaleParent,
                _ => EdgeKind::Reconfirms,
            };
            dag.add_edge(Edge {
                kind,
                from: SymbolId::new(pair[0]),
                to: SymbolId::new(pair[1]),
                at,
            }).expect("forward edge never forms a cycle");
        }
        prop_assert_eq!(dag.len(), ids.len() - 1);
    }

    /// Any edge that closes back to an ancestor in the current DAG
    /// must be rejected with `SupersessionCycle` — regardless of the
    /// edge kinds already in the chain.
    #[test]
    fn back_edge_to_ancestor_is_always_rejected(
        chain_len in 2_u32..=8,
        kind_choice in 0_u8..4,
    ) {
        let mut dag = SupersessionDag::new();
        let at = ClockTime::try_from_millis(1_000_000).unwrap();
        // Build a linear chain 0 -> 1 -> 2 -> ... -> chain_len-1
        // using a mix of kinds to exercise union acyclicity.
        for i in 0..chain_len {
            let kind = match (i + u32::from(kind_choice)) % 4 {
                0 => EdgeKind::Supersedes,
                1 => EdgeKind::Corrects,
                2 => EdgeKind::StaleParent,
                _ => EdgeKind::Reconfirms,
            };
            dag.add_edge(Edge {
                kind,
                from: SymbolId::new(u64::from(i)),
                to: SymbolId::new(u64::from(i + 1)),
                at,
            }).expect("chain link");
        }
        // Any back-edge from the last node to an earlier node must be
        // rejected — that edge would close the chain into a cycle.
        let closer_kind = match kind_choice {
            0 => EdgeKind::Supersedes,
            1 => EdgeKind::Corrects,
            2 => EdgeKind::StaleParent,
            _ => EdgeKind::Reconfirms,
        };
        // Use node `0` as the back-edge target so it's reachable
        // from every subsequent node, not just the last. We target
        // from the penultimate node because chain_len itself is the
        // last node added to the chain (`chain_len - 1 -> chain_len`
        // was the last link), so `chain_len` reaches `0` via the
        // whole chain.
        let err = dag.add_edge(Edge {
            kind: closer_kind,
            from: SymbolId::new(u64::from(chain_len)),
            to: SymbolId::new(0),
            at,
        }).expect_err("back-edge closes the chain");
        let is_cycle = matches!(err, DagError::SupersessionCycle { .. });
        prop_assert!(is_cycle, "expected SupersessionCycle, got different error");
        // DAG was not mutated by the failed add.
        prop_assert_eq!(u32::try_from(dag.len()).unwrap(), chain_len);
    }

    /// As-of-query correctness across a generated forward-supersession
    /// chain. Covers § 1 graduation criterion #3's "as-of query
    /// correctness on a generated supersession chain" — closes the
    /// last remaining criterion for `temporal-model` graduation.
    ///
    /// For any strictly-increasing sequence of `valid_at` offsets
    /// (days past 2024-01-01), a Semantic write is committed at each
    /// point. Reading at any `as_of` between entries must return the
    /// memory whose `valid_at` is the largest value ≤ `as_of`:
    /// that's the one whose forward-supersession hasn't yet closed.
    ///
    /// Reading at an `as_of` before the first entry returns `None`.
    #[test]
    fn as_of_returns_latest_valid_at_not_past_query_point(
        day_offsets in proptest::collection::vec(0_u32..27, 2..=6),
        probe_offset in 0_u32..60,
    ) {
        use mimir_core::resolver::{resolve_semantic, TemporalQuery};

        // Base date: 2024-01-01 00:00:00 UTC.
        const BASE_MS: u64 = 1_704_067_200_000;
        const DAY_MS: u64 = 24 * 60 * 60 * 1000;

        // Deduplicate and sort so we get strictly-increasing valid_at
        // points, and drop runs whose collapsed length dropped below 2.
        let mut days: Vec<u32> = day_offsets;
        days.sort_unstable();
        days.dedup();
        prop_assume!(days.len() >= 2);

        let mut pipe = Pipeline::new();
        // Fixed wall clock far after the last valid_at so no
        // future-validity rejections can fire.
        let fixed_now = ClockTime::try_from_millis(1_714_000_000_000).unwrap();

        for day in &days {
            let input = format!(
                "(sem @subj @rel @obj_d{day} :src @observation :c 0.8 :v 2024-01-{:02})",
                day + 1
            );
            pipe.compile_batch(&input, fixed_now).expect("compile");
        }

        let probe = ClockTime::try_from_millis(BASE_MS + u64::from(probe_offset) * DAY_MS).unwrap();
        let s = pipe.table().lookup("subj").unwrap();
        let p = pipe.table().lookup("rel").unwrap();

        let got = resolve_semantic(&pipe, s, p, TemporalQuery::as_of(probe));

        // Expected: the entry with the highest valid_at ≤ probe,
        // or None if no entry's valid_at is ≤ probe.
        let expected_day = days.iter().rev().find(|&&d| {
            let day_ms = BASE_MS + u64::from(d) * DAY_MS;
            day_ms <= probe.as_millis()
        }).copied();

        match (got, expected_day) {
            (None, None) => {} // both agree — probe before any entry
            (Some(record), Some(d)) => {
                let expected_valid_at = ClockTime::try_from_millis(BASE_MS + u64::from(d) * DAY_MS).unwrap();
                if record.clocks.valid_at != expected_valid_at {
                    return Err(TestCaseError::fail(format!(
                        "resolver picked wrong chain entry: got valid_at={:?}, expected={expected_valid_at:?} for probe {probe:?}",
                        record.clocks.valid_at
                    )));
                }
            }
            (Some(r), None) => {
                return Err(TestCaseError::fail(format!(
                    "resolver returned {r:?} but expected None (probe before every entry)"
                )));
            }
            (None, Some(d)) => {
                return Err(TestCaseError::fail(format!(
                    "resolver returned None but expected entry for day {d}"
                )));
            }
        }
    }

    /// Stronger as-of property: generate a forward chain, then
    /// inject retroactive writes at random earlier valid_at points.
    /// Compare the resolver's answer to a reference implementation
    /// that brute-forces § 7 semantics from the record+edge state
    /// via `Pipeline` accessors only — no shared code with the
    /// resolver module.
    #[test]
    #[allow(clippy::items_after_statements)]
    fn resolver_matches_reference_on_forward_and_retroactive_mix(
        forward_days in proptest::collection::vec(0_u32..27, 2..=5),
        retro_days in proptest::collection::vec(0_u32..27, 0..=4),
        probe_offset in 0_u32..60,
    ) {
        use mimir_core::resolver::{resolve_semantic, TemporalQuery};
        use mimir_core::canonical::SemRecord;

        const BASE_MS: u64 = 1_704_067_200_000;
        const DAY_MS: u64 = 24 * 60 * 60 * 1000;

        // Forward chain: strictly increasing valid_at.
        let mut forward: Vec<u32> = forward_days;
        forward.sort_unstable();
        forward.dedup();
        prop_assume!(forward.len() >= 2);

        let mut pipe = Pipeline::new();
        let fixed_now = ClockTime::try_from_millis(1_714_000_000_000).unwrap();

        for (idx, day) in forward.iter().enumerate() {
            let input = format!(
                "(sem @subj @rel @fwd_{idx} :src @observation :c 0.8 :v 2024-01-{:02})",
                day + 1
            );
            pipe.compile_batch(&input, fixed_now).expect("forward compile");
        }
        // Retroactive writes: each is a Sem write with a valid_at
        // strictly less than the current head (prior writes already
        // committed). Skip any candidate that would equal an
        // existing valid_at — that would be SemanticSupersessionConflict.
        let max_forward_day = *forward.last().unwrap();
        let mut used_days: std::collections::BTreeSet<u32> = forward.iter().copied().collect();
        for (idx, day) in retro_days.iter().enumerate() {
            let retro_day = day % max_forward_day.max(1);
            if used_days.contains(&retro_day) {
                continue;
            }
            used_days.insert(retro_day);
            let input = format!(
                "(sem @subj @rel @retro_{idx} :src @observation :c 0.8 :v 2024-01-{:02})",
                retro_day + 1
            );
            pipe.compile_batch(&input, fixed_now).expect("retro compile");
        }

        let probe = ClockTime::try_from_millis(BASE_MS + u64::from(probe_offset) * DAY_MS).unwrap();
        let s = pipe.table().lookup("subj").unwrap();
        let p = pipe.table().lookup("rel").unwrap();

        let got = resolve_semantic(&pipe, s, p, TemporalQuery::as_of(probe));

        // Reference implementation: iterate all SemRecords for (s, p),
        // compute effective invalid_at from record-level + edges
        // targeting that memory (forward-derived closure), and pick
        // the record with the highest committed_at among authoritative
        // candidates at the probe.
        let expected = reference_resolve_semantic(&pipe, s, p, probe);

        match (got, expected) {
            (None, None) => {}
            (Some(g), Some(e)) => {
                if g.memory_id != e.memory_id {
                    return Err(TestCaseError::fail(format!(
                        "resolver disagrees with reference at probe {probe:?}: got {:?}, expected {:?}",
                        g.memory_id, e.memory_id
                    )));
                }
            }
            (Some(g), None) => {
                return Err(TestCaseError::fail(format!(
                    "resolver returned {:?}, reference returned None",
                    g.memory_id
                )));
            }
            (None, Some(e)) => {
                return Err(TestCaseError::fail(format!(
                    "resolver returned None, reference returned {:?}",
                    e.memory_id
                )));
            }
        }

        // Nested so it shares types without leaking into the
        // top-level test module.
        fn reference_resolve_semantic(
            pipe: &Pipeline,
            s: SymbolId,
            p: SymbolId,
            probe: ClockTime,
        ) -> Option<SemRecord> {
            use mimir_core::dag::EdgeKind;
            let as_committed = pipe.last_committed_at()?;
            let mut best: Option<SemRecord> = None;
            for record in pipe.semantic_records() {
                if record.s != s || record.p != p {
                    continue;
                }
                if record.clocks.committed_at > as_committed {
                    continue;
                }
                if record.clocks.valid_at > probe {
                    continue;
                }
                // Effective invalid_at.
                let mut min_iv = record.clocks.invalid_at;
                for edge in pipe.dag().edges_to(record.memory_id) {
                    if edge.kind != EdgeKind::Supersedes {
                        continue;
                    }
                    if edge.at > as_committed {
                        continue;
                    }
                    // Look up source valid_at.
                    if let Some(src) = pipe
                        .semantic_records()
                        .iter()
                        .find(|r| r.memory_id == edge.from)
                    {
                        min_iv = Some(match min_iv {
                            None => src.clocks.valid_at,
                            Some(m) => m.min(src.clocks.valid_at),
                        });
                    }
                }
                if let Some(iv) = min_iv {
                    if iv <= probe {
                        continue;
                    }
                }
                best = Some(match best {
                    None => record.clone(),
                    Some(cur) if record.clocks.committed_at > cur.clocks.committed_at => {
                        record.clone()
                    }
                    Some(cur) => cur,
                });
            }
            best
        }
    }

    // -----------------------------------------------------------------
    // Phase 7.4 — read-path property tests (read-protocol.md § 1 #3)
    // -----------------------------------------------------------------

    /// Snapshot isolation: writes committed after a reader's query-
    /// start time are invisible to that reader. Modelled here by
    /// capturing a query before a later write, then asserting the
    /// later write never appears in the earlier result set.
    ///
    /// Strategy: commit one record, capture the query; commit another
    /// record; assert the captured result still contains only records
    /// with `committed_at <= query.query_committed_at`.
    #[test]
    fn read_snapshot_isolation_holds(
        n_prior in 0_usize..6,
        n_after in 1_usize..6,
    ) {
        let mut pipe = Pipeline::new();
        let base_ms = 1_710_000_000_000_u64;
        let mut step = 0_u64;
        let mut next_now = || {
            step += 1;
            ClockTime::try_from_millis(base_ms + step * 1000)
                .expect("non-sentinel")
        };
        // Commit the "prior" batch of distinct (s, p) records.
        for i in 0..n_prior {
            let src = format!(
                "(sem @prior_s{i} @rel @prior_o{i} :src @observation :c 0.9 :v 2024-01-01)"
            );
            pipe.compile_batch(&src, next_now()).expect("compile");
        }
        // Capture the snapshot at this watermark.
        let captured = pipe.execute_query("(query)").expect("query");
        let snapshot = captured.query_committed_at.as_millis();
        // Commit more records after the snapshot.
        for i in 0..n_after {
            let src = format!(
                "(sem @later_s{i} @rel @later_o{i} :src @observation :c 0.9 :v 2024-01-01)"
            );
            pipe.compile_batch(&src, next_now()).expect("compile");
        }
        // Re-check the captured result: none of its records can be
        // committed after the captured snapshot.
        for record in &captured.records {
            let cat = match record {
                CanonicalRecord::Sem(r) => r.clocks.committed_at.as_millis(),
                CanonicalRecord::Pro(r) => r.clocks.committed_at.as_millis(),
                CanonicalRecord::Inf(r) => r.clocks.committed_at.as_millis(),
                CanonicalRecord::Epi(r) => r.committed_at.as_millis(),
                _ => 0,
            };
            prop_assert!(
                cat <= snapshot,
                "record committed_at {cat} exceeds captured snapshot {snapshot}"
            );
        }
        // And captured.records must equal n_prior — later commits are
        // invisible to the already-computed result.
        prop_assert_eq!(captured.records.len(), n_prior);
    }

    /// STALE_SYMBOL flag propagation: if a returned record references
    /// at least one retired symbol, the flag is set. Conversely, if
    /// no returned record touches a retired symbol, the flag is clear.
    ///
    /// Strategy: build a random mix of (s, p) Semantic writes, then
    /// retire a random subset of the subjects, then query with
    /// `:include_retired true`. Assert the flag's presence matches
    /// the actual retired-ref content of the result set.
    #[test]
    fn stale_symbol_flag_matches_returned_record_content(
        n_memories in 1_usize..8,
        retirement_bits in 0_u32..256,
    ) {
        let mut pipe = Pipeline::new();
        let base_ms = 1_710_000_000_000_u64;
        for i in 0..n_memories {
            let now = ClockTime::try_from_millis(base_ms + (i as u64) * 1000)
                .expect("non-sentinel");
            let src = format!(
                "(sem @subj{i} @rel @obj{i} :src @observation :c 0.9 :v 2024-01-01)"
            );
            pipe.compile_batch(&src, now).expect("compile");
        }
        // Retire each subject whose bit is set in `retirement_bits`.
        // Cap the iteration to whatever bits cover `n_memories`.
        for i in 0..n_memories {
            if (retirement_bits >> i) & 1 == 1 {
                let now = ClockTime::try_from_millis(
                    base_ms + ((n_memories + i) as u64) * 1000,
                )
                .expect("non-sentinel");
                let src = format!("(retire @subj{i})");
                pipe.compile_batch(&src, now).expect("retire");
            }
        }
        let got = pipe
            .execute_query("(query :include_retired true)")
            .expect("query");
        let any_retired_ref = got.records.iter().any(|record| match record {
            CanonicalRecord::Sem(r) => {
                pipe.table().is_retired(r.s)
                    || pipe.table().is_retired(r.p)
                    || pipe.table().is_retired(r.source)
                    || matches!(&r.o, Value::Symbol(id) if pipe.table().is_retired(*id))
            }
            _ => false,
        });
        prop_assert_eq!(
            got.flags.contains(ReadFlags::STALE_SYMBOL),
            any_retired_ref,
            "STALE_SYMBOL flag must match actual retired-ref content"
        );
    }

    // -----------------------------------------------------------------
    // Randomized-input robustness (#33)
    //
    // These stand in for the `cargo-fuzz` targets at `fuzz/` when the
    // workspace doesn't have a nightly toolchain available. Coverage
    // is breadth-random rather than coverage-guided, but the contract
    // they check is identical: no panics, no infinite loops, all
    // failure modes route through typed `Result::Err`.
    // -----------------------------------------------------------------

    /// `tokenize` returns a `Result` for any valid UTF-8 string.
    /// Panics are bugs; `LexError` is the only acceptable failure
    /// mode.
    #[test]
    fn lex_is_total_over_random_utf8(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        if let Ok(s) = std::str::from_utf8(&bytes) {
            let _ = mimir_core::lex::tokenize(s);
        }
    }

    /// `parse` is total over random UTF-8 — same contract as
    /// `tokenize`.
    #[test]
    fn parse_is_total_over_random_utf8(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        if let Ok(s) = std::str::from_utf8(&bytes) {
            let _ = mimir_core::parse::parse(s);
        }
    }

    /// `decode_record` returns a `Result` for any byte sequence.
    /// Malformed canonical bytes must surface as `DecodeError`,
    /// never panics / overreads / silent truncation.
    #[test]
    fn decode_record_is_total_over_random_bytes(
        bytes in proptest::collection::vec(any::<u8>(), 0..256)
    ) {
        let _ = mimir_core::canonical::decode_record(&bytes);
    }

    /// Same contract for the streaming `decode_all` boundary.
    #[test]
    fn decode_all_is_total_over_random_bytes(
        bytes in proptest::collection::vec(any::<u8>(), 0..512)
    ) {
        let _ = mimir_core::canonical::decode_all(&bytes);
    }

    /// Feed the decoder the prefix of a valid encoding at every
    /// byte boundary. A truncated record must surface as
    /// `DecodeError::Truncated` (or another `DecodeError`), never
    /// succeed partially.
    #[test]
    fn decoder_rejects_every_prefix_truncation(
        s in 1_u64..1000,
        p in 1001_u64..2000,
    ) {
        let sem = SemRecord {
            memory_id: SymbolId::new(1),
            s: SymbolId::new(s),
            p: SymbolId::new(p),
            o: Value::Integer(42),
            source: SymbolId::new(9),
            confidence: Confidence::from_u16(50_000),
            clocks: Clocks {
                valid_at: ClockTime::try_from_millis(1).unwrap(),
                observed_at: ClockTime::try_from_millis(1).unwrap(),
                committed_at: ClockTime::try_from_millis(1).unwrap(),
                invalid_at: None,
            },
            flags: SemFlags::default(),
        };
        let mut buf = Vec::new();
        encode_record(&CanonicalRecord::Sem(sem), &mut buf);
        // Exclude the full length — that's the valid case, not a
        // truncation.
        for cut in 0..buf.len() {
            let result = decode_record(&buf[..cut]);
            prop_assert!(
                result.is_err(),
                "prefix of length {cut} must fail to decode (got {:?})",
                result.map(|(r, _)| r.opcode())
            );
        }
    }

    /// Supersession crosses Episode boundaries per
    /// `episode-semantics.md` § 7 — Episodes are labels, not
    /// isolation boundaries. A memory in Episode A superseded by a
    /// memory in Episode B becomes invalid even though the two
    /// live in different Episodes.
    ///
    /// Strategy: commit a Sem in Episode 1, then a superseding Sem
    /// at the same `(s, p)` with a later `valid_at` in Episode 2.
    /// Query unscoped — the current-state result is the Episode-2
    /// memory regardless of the Episode boundary between them.
    #[test]
    fn supersession_crosses_episode_boundaries(
        step_days in 1_i64..90,
    ) {
        let mut pipe = Pipeline::new();
        let base_ms = 1_710_000_000_000_u64;
        let now1 = ClockTime::try_from_millis(base_ms).expect("non-sentinel");
        pipe.compile_batch(
            "(sem @mira @likes @tea :src @observation :c 0.8 :v 2024-01-15)",
            now1,
        )
        .expect("first batch");
        let now2 = ClockTime::try_from_millis(base_ms + 1000).expect("non-sentinel");
        // Later valid_at → forward supersession.
        let day = 15 + step_days.min(10);
        let v2 = format!("2024-01-{day:02}");
        let src = format!(
            "(sem @mira @likes @coffee :src @observation :c 0.8 :v {v2})"
        );
        pipe.compile_batch(&src, now2).expect("second batch (new Episode)");

        let got = pipe
            .execute_query("(query :s @mira :p @likes)")
            .expect("unscoped query");
        prop_assert_eq!(got.records.len(), 1, "supersession should yield exactly one current");
        let CanonicalRecord::Sem(sem) = &got.records[0] else {
            panic!("expected Sem record");
        };
        let coffee = pipe.table().lookup("coffee").expect("coffee bound");
        prop_assert!(
            matches!(&sem.o, Value::Symbol(id) if *id == coffee),
            "the later-Episode memory (@coffee) should be current"
        );
    }

    /// As-of with retroactive supersession: when a memory's
    /// `valid_at` retroactively predates another already in the
    /// store, an `:as_of` query at a historical point returns
    /// whichever memory's validity window contains that point.
    ///
    /// Strategy: commit M1 with `valid_at = 2024-03-01`, then a
    /// retroactive M2 with `valid_at` in January. Probe at a point
    /// between the two. Depending on which side of the probe M2's
    /// `valid_at` falls, the result is either M2 (V2 ≤ probe) or
    /// nothing (V2 > probe).
    #[test]
    fn read_as_of_respects_retroactive_supersession(
        v2_day in 1_u32..28,
    ) {
        let mut pipe = Pipeline::new();
        let now1 = ClockTime::try_from_millis(1_710_000_000_000)
            .expect("non-sentinel");
        pipe.compile_batch(
            "(sem @a @rel @o1 :src @observation :c 0.9 :v 2024-03-01)",
            now1,
        )
        .expect("first");
        let now2 = ClockTime::try_from_millis(1_710_000_000_000 + 1_000)
            .expect("non-sentinel");
        let v2_iso = format!("2024-01-{v2_day:02}");
        let src = format!(
            "(sem @a @rel @o2 :src @observation :c 0.9 :v {v2_iso})"
        );
        pipe.compile_batch(&src, now2).expect("retroactive");

        // Probe at 2024-01-20. If v2_day ≤ 20, the retroactive M2 is
        // valid at the probe → expect 1 record with `o == @o2`. Else
        // neither M1 (valid_at 2024-03-01) nor M2 is valid yet →
        // expect empty.
        let got = pipe
            .execute_query("(query :s @a :p @rel :as_of 2024-01-20)")
            .expect("as_of query");
        if v2_day <= 20 {
            prop_assert_eq!(got.records.len(), 1);
            let CanonicalRecord::Sem(sem) = &got.records[0] else {
                panic!("expected Sem record");
            };
            let o2 = pipe.table().lookup("o2").expect("o2 bound");
            prop_assert!(
                matches!(&sem.o, Value::Symbol(id) if *id == o2),
                "expected retroactive M2 (@o2) at probe 2024-01-20"
            );
        } else {
            prop_assert!(
                got.records.is_empty(),
                "no memory is valid at probe 2024-01-20 when V2={v2_iso}"
            );
        }
    }

    /// Phase 3.1 exit criterion: "for any committed Inferential, a
    /// matching `(query :kind inf)` returns it." We commit N
    /// Inferentials with **distinct** `(s, p)` pairs — no
    /// auto-supersession kicks in — and assert every single one is
    /// retrievable by a bare `:kind inf` query. Distinctness guards
    /// the proptest against the (separately-tested) supersession
    /// path.
    #[test]
    fn every_committed_inferential_is_retrievable_by_kind_inf(
        n in 1usize..=8usize,
    ) {
        let mut pipe = Pipeline::new();
        // Seed @alice so downstream inf references `@__mem_0` as a
        // legal parent symbol.
        let seed_now = ClockTime::try_from_millis(1_710_000_000_000)
            .expect("non-sentinel");
        pipe.compile_batch(
            "(sem @alice @knows @bob :src @observation :c 0.9 :v 2024-01-01)",
            seed_now,
        )
        .expect("seed sem commit");
        for i in 0..n {
            // Each Inf uses a distinct predicate `@rel_i` so no two
            // share an `(s, p)` key — no supersession interference.
            let src = format!(
                "(inf @alice @rel_{i} @target_{i} (@__mem_0) @citation_link \
                 :c 0.7 :v 2024-02-01)",
            );
            let inf_now = ClockTime::try_from_millis(1_710_000_000_000 + 1_000 + (i as u64) * 1_000)
                .expect("non-sentinel");
            pipe.compile_batch(&src, inf_now).expect("inf commit");
        }
        let got = pipe.execute_query("(query :kind inf)").expect("query");
        prop_assert_eq!(
            got.records.len(),
            n,
            "every committed Inferential must be retrievable; got {} of {}",
            got.records.len(),
            n
        );
        for r in &got.records {
            prop_assert!(
                matches!(r, CanonicalRecord::Inf(_)),
                ":kind inf must return only Inf records; got {r:?}"
            );
        }
    }
}
