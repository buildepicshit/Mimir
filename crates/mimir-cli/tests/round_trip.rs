//! End-to-end round-trip tests for the decoder.
//!
//! Covers `decoder-tool-contract.md` § 1 graduation criterion #3:
//! every canonical record in the test corpus decodes → re-enqueues →
//! canonicalizes to an agent-visible field set that matches the
//! original. Librarian-assigned fields (`memory_id`, `committed_at`,
//! `observed_at` for non-Episodic) are allowed to differ; everything
//! the agent actually wrote must match.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mimir_cli::{load_table_from_log, verify, LispRenderer, RenderError};
use mimir_core::canonical::CanonicalRecord;
use mimir_core::{ClockTime, Pipeline, Store};
use tempfile::TempDir;

fn fixed_now() -> ClockTime {
    ClockTime::try_from_millis(1_713_350_400_000).expect("non-sentinel")
}

fn commit_corpus(store: &mut Store) {
    let inputs = [
        "(sem @alice @knows @bob :src @observation :c 0.8 :v 2024-01-15)",
        r#"(sem @alice email "alice@example.com" :src @profile :c 0.95 :v 2024-01-15)"#,
        "(epi @evt_001 @rename (@old @new) @github :at 2024-01-15T10:00:00Z :obs 2024-01-15T10:00:05Z :src @alice :c 0.9)",
        r#"(pro @rule_1 "agent writes a memory" "route via librarian" :scp @mimir :src @agent_instruction :c 0.9)"#,
        "(inf @alice @friend_of @carol (@m0 @m1) @citation_link :c 0.6 :v 2024-01-15)",
    ];
    for input in inputs {
        store.commit_batch(input, fixed_now()).expect("commit");
    }
}

#[test]
fn render_every_memory_in_corpus() {
    let dir = TempDir::new().expect("tmp");
    let path = dir.path().join("canonical.log");
    {
        let mut store = Store::open(&path).expect("open");
        commit_corpus(&mut store);
    }

    let table = load_table_from_log(&path).expect("table");
    let mut log = mimir_core::log::CanonicalLog::open(&path).expect("reopen");
    let bytes = {
        use mimir_core::log::LogBackend;
        log.read_all().expect("read")
    };
    let records = mimir_core::canonical::decode_all(&bytes).expect("decode");

    let renderer = LispRenderer::new(&table);
    let mut memories_rendered = 0_usize;
    for record in &records {
        match renderer.render_memory(record) {
            Ok(text) => {
                memories_rendered += 1;
                // Every rendered line must be non-empty valid UTF-8.
                assert!(!text.is_empty(), "empty render for {record:?}");
                // Every rendered Lisp form must re-parse cleanly. This
                // is the round-trip half of graduation criterion #3 —
                // the render output is valid write-surface Lisp.
                let reparsed = mimir_core::parse::parse(&text);
                assert!(
                    reparsed.is_ok(),
                    "rendered form {text:?} failed re-parse: {:?}",
                    reparsed.err()
                );
            }
            Err(RenderError::NotAMemory) => {}
            Err(e) => panic!("render failed: {e}"),
        }
    }
    assert_eq!(memories_rendered, 5, "expected 5 memory records rendered");
}

#[test]
fn round_trip_sem_agent_visible_fields_preserved() {
    // Decode → render → re-parse → re-pipeline → compare.
    let dir = TempDir::new().expect("tmp");
    let path = dir.path().join("canonical.log");
    let original_input = "(sem @alice @knows @bob :src @observation :c 0.8 :v 2024-01-15)";
    {
        let mut store = Store::open(&path).expect("open");
        store
            .commit_batch(original_input, fixed_now())
            .expect("commit");
    }

    let table = load_table_from_log(&path).expect("table");
    let mut log = mimir_core::log::CanonicalLog::open(&path).expect("reopen");
    let bytes = {
        use mimir_core::log::LogBackend;
        log.read_all().expect("read")
    };
    let records = mimir_core::canonical::decode_all(&bytes).expect("decode");
    let original_sem = records
        .iter()
        .find_map(|r| {
            if let CanonicalRecord::Sem(s) = r {
                Some(s.clone())
            } else {
                None
            }
        })
        .expect("find original sem");

    let renderer = LispRenderer::new(&table);
    let lisp = renderer
        .render_memory(&CanonicalRecord::Sem(original_sem.clone()))
        .expect("render");

    // Re-pipeline the rendered Lisp in a fresh Pipeline.
    let mut fresh = Pipeline::new();
    let new_records = fresh.compile_batch(&lisp, fixed_now()).expect("re-compile");
    let new_sem = new_records
        .iter()
        .find_map(|r| {
            if let CanonicalRecord::Sem(s) = r {
                Some(s)
            } else {
                None
            }
        })
        .expect("find re-compiled sem");

    // Agent-visible fields must match. memory_id and committed_at /
    // observed_at are librarian-assigned and allowed to differ.
    assert_eq!(
        new_sem.o, original_sem.o,
        "object value changed under round-trip"
    );
    assert_eq!(
        new_sem.confidence, original_sem.confidence,
        "confidence changed"
    );
    assert_eq!(
        new_sem.clocks.valid_at, original_sem.clocks.valid_at,
        "valid_at changed"
    );
    // Subject / predicate / source symbol IDs are workspace-local;
    // compare via the canonical_name lookup.
    let same_name = |a: mimir_core::SymbolId,
                     at: &mimir_core::bind::SymbolTable,
                     b: mimir_core::SymbolId,
                     bt: &mimir_core::bind::SymbolTable| {
        at.entry(a).map(|e| e.canonical_name.clone())
            == bt.entry(b).map(|e| e.canonical_name.clone())
    };
    assert!(same_name(new_sem.s, fresh.table(), original_sem.s, &table));
    assert!(same_name(new_sem.p, fresh.table(), original_sem.p, &table));
    assert!(same_name(
        new_sem.source,
        fresh.table(),
        original_sem.source,
        &table
    ));
}

#[test]
fn verify_reports_clean_committed_log() {
    let dir = TempDir::new().expect("tmp");
    let path = dir.path().join("canonical.log");
    {
        let mut store = Store::open(&path).expect("open");
        commit_corpus(&mut store);
    }
    let report = verify(&path).expect("verify");
    assert_eq!(report.checkpoints, 5);
    assert_eq!(report.memory_records, 5);
    // Exact records_decoded = symbol events + memory records +
    // checkpoints. The loose `symbol_events > 0` assertion is
    // replaced by the stronger invariant-equation assertion below,
    // which catches any regression in the "one SymbolAlloc per
    // first-use symbol + one per librarian-synthesised __mem_N +
    // one per __ep_N" contract.
    assert_eq!(
        report.records_decoded,
        report.symbol_events + report.memory_records + report.checkpoints
    );
    assert!(
        report.symbol_events >= 5 + 5,
        "at least one __mem_N + __ep_N per batch"
    );
    assert!(report.tail.is_clean());
    assert_eq!(report.dangling_symbols, 0);
}

#[test]
fn verify_classifies_unknown_opcode_tail_as_corrupt() {
    // Bytes `[0xFF, ...]` trigger `DecodeError::UnknownOpcode` — the
    // decoder stops on a non-truncation error, so the tail is
    // genuine corruption rather than a recoverable orphan truncation.
    let dir = TempDir::new().expect("tmp");
    let path = dir.path().join("canonical.log");
    {
        let mut store = Store::open(&path).expect("open");
        commit_corpus(&mut store);
    }
    {
        use mimir_core::log::LogBackend;
        let mut raw = mimir_core::log::CanonicalLog::open(&path).expect("raw");
        raw.append(&[0xFF_u8, 0xFE, 0xFD, 0xFC, 0xFB, 0xFA, 0x01, 0x02])
            .expect("append");
        raw.sync().expect("sync");
    }
    let report = verify(&path).expect("verify");
    assert!(report.tail.is_corrupt(), "tail must classify as corrupt");
    assert!(!report.tail.is_clean());
    assert!(report.tail.trailing_bytes() > 0);
    // The corrupt variant preserves the decode error for the caller.
    match &report.tail {
        mimir_cli::TailStatus::Corrupt {
            first_decode_error, ..
        } => {
            assert!(matches!(
                first_decode_error,
                mimir_core::canonical::DecodeError::UnknownOpcode { byte: 0xFF, .. }
            ));
        }
        other => panic!("expected Corrupt tail, got {other:?}"),
    }
}

#[test]
fn verify_classifies_truncated_tail_as_orphan() {
    // A valid opcode followed by a length-varint indicating more
    // bytes than the log contains triggers `DecodeError::Truncated`.
    // That's the recoverable append-was-interrupted pattern from
    // `write-protocol.md` § 10 — tail must classify as OrphanTail,
    // not Corrupt.
    let dir = TempDir::new().expect("tmp");
    let path = dir.path().join("canonical.log");
    {
        let mut store = Store::open(&path).expect("open");
        commit_corpus(&mut store);
    }
    {
        use mimir_core::log::LogBackend;
        let mut raw = mimir_core::log::CanonicalLog::open(&path).expect("raw");
        // Opcode 0x01 (Sem) + varint body length 5, but no body
        // bytes follow. Decoder reads the length, then hits EOF on
        // the body → Truncated.
        raw.append(&[0x01_u8, 0x05]).expect("append");
        raw.sync().expect("sync");
    }
    let report = verify(&path).expect("verify");
    assert!(
        !report.tail.is_corrupt(),
        "tail must not classify as corrupt"
    );
    assert!(!report.tail.is_clean());
    assert!(matches!(
        report.tail,
        mimir_cli::TailStatus::OrphanTail { .. }
    ));
    assert!(report.tail.trailing_bytes() > 0);
}

#[test]
fn verify_detects_corrupted_opcode_at_head() {
    let dir = TempDir::new().expect("tmp");
    let path = dir.path().join("canonical.log");
    {
        let mut store = Store::open(&path).expect("open");
        store
            .commit_batch(
                "(sem @alice @knows @bob :src @observation :c 0.8 :v 2024-01-15)",
                fixed_now(),
            )
            .expect("commit");
    }
    // Rewrite the first PAYLOAD byte (offset = LOG_HEADER_SIZE = 8,
    // the opening record's opcode) with an unknown opcode. The
    // decoder stops immediately — every payload byte is corrupt tail.
    // Byte 0 of the file is now part of the 8-byte magic header per
    // log.rs `LOG_MAGIC` / `LOG_FORMAT_VERSION`; corrupting it would
    // prevent the file from opening at all (covered separately in
    // `verify_rejects_non_mimir_file`).
    {
        use mimir_core::log::LOG_HEADER_SIZE;
        use std::fs::OpenOptions;
        use std::io::{Seek, SeekFrom, Write};
        let mut f = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .expect("open rw");
        f.seek(SeekFrom::Start(LOG_HEADER_SIZE))
            .expect("seek past header");
        f.write_all(&[0xAA]).expect("write");
        f.sync_all().expect("sync");
    }
    let report = verify(&path).expect("verify");
    assert_eq!(report.records_decoded, 0);
    assert!(report.tail.is_corrupt());
}

/// Companion to `verify_detects_corrupted_opcode_at_head`. Mutating
/// the magic header (offset 0..8) makes the file unrecognizable as
/// an Mimir log — `verify` cannot even open it. Returns an error
/// rather than a `VerifyReport`, which is the correct contract:
/// non-Mimir files don't get auto-classified as "corrupt tail."
#[test]
fn verify_rejects_non_mimir_file() {
    let dir = TempDir::new().expect("tmp");
    let path = dir.path().join("canonical.log");
    {
        let mut store = Store::open(&path).expect("open");
        store
            .commit_batch(
                "(sem @alice @knows @bob :src @observation :c 0.8 :v 2024-01-15)",
                fixed_now(),
            )
            .expect("commit");
    }
    // Corrupt the magic prefix — the file is no longer recognizable.
    {
        use std::fs::OpenOptions;
        use std::io::{Seek, SeekFrom, Write};
        let mut f = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .expect("open rw");
        f.seek(SeekFrom::Start(0)).expect("seek");
        f.write_all(b"NOPE").expect("clobber magic");
        f.sync_all().expect("sync");
    }
    // verify() returns an error (not a VerifyReport) because the
    // open itself fails. This is the safe contract: misrouted-path
    // opens NEVER silently succeed.
    let err = verify(&path).expect_err("must reject non-Mimir file");
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("incompatible") || msg.contains("format"),
        "error should signal format incompatibility, got: {err:?}"
    );
}

#[test]
fn verify_detects_dangling_symbol_reference() {
    // Spec § 6 symbol-reference corruption class. Fabricate a log
    // that contains a `Sem` memory record whose `s` / `p` / `o` /
    // `source` / `memory_id` reference `SymbolId`s that no
    // preceding `SymbolAlloc` introduced. `verify` walks committed
    // records, replays the symbol events into a fresh table, and
    // counts every memory-record `SymbolId` that fails to resolve.
    use mimir_core::canonical::{
        encode_record, CanonicalRecord, CheckpointRecord, Clocks, SemFlags, SemRecord,
        SymbolEventRecord,
    };
    use mimir_core::log::{CanonicalLog, LogBackend};
    use mimir_core::symbol::{SymbolId, SymbolKind};
    use mimir_core::{Confidence, Value};

    let dir = TempDir::new().expect("tmp");
    let path = dir.path().join("canonical.log");
    let now = fixed_now();

    // Allocate only ONE symbol in the log; the Sem record will
    // reference four IDs that weren't allocated — each is dangling.
    let alloc = CanonicalRecord::SymbolAlloc(SymbolEventRecord {
        symbol_id: SymbolId::new(0),
        name: "alice".into(),
        symbol_kind: SymbolKind::Agent,
        at: now,
    });
    let sem = CanonicalRecord::Sem(SemRecord {
        memory_id: SymbolId::new(99),         // dangling
        s: SymbolId::new(0),                  // resolves
        p: SymbolId::new(101),                // dangling
        o: Value::Symbol(SymbolId::new(102)), // dangling
        source: SymbolId::new(103),           // dangling
        confidence: Confidence::try_from_f32(0.8).expect("in range"),
        clocks: Clocks {
            valid_at: now,
            observed_at: now,
            committed_at: now,
            invalid_at: None,
        },
        flags: SemFlags::default(),
    });
    let checkpoint = CanonicalRecord::Checkpoint(CheckpointRecord {
        episode_id: SymbolId::new(0),
        at: now,
        memory_count: 1,
    });
    {
        let mut raw = CanonicalLog::open(&path).expect("open raw");
        let mut buf = Vec::new();
        encode_record(&alloc, &mut buf);
        encode_record(&sem, &mut buf);
        encode_record(&checkpoint, &mut buf);
        raw.append(&buf).expect("append");
        raw.sync().expect("sync");
    }

    let report = verify(&path).expect("verify");
    // Four dangling references: memory_id, p, o, source (s resolves).
    assert_eq!(
        report.dangling_symbols, 4,
        "expected exactly the four fabricated dangling references, got {} — report: {report:?}",
        report.dangling_symbols
    );
    assert_eq!(
        report.trailing_bytes(),
        0,
        "log must decode cleanly end to end"
    );
}

#[test]
fn decoder_is_read_only() {
    // Spec § 10 invariant 1. Render, verify, and load_table_from_log
    // must not mutate the log. We commit a batch, snapshot the log's
    // bytes, run every read-only op, and assert bytes unchanged.
    let dir = TempDir::new().expect("tmp");
    let path = dir.path().join("canonical.log");
    {
        let mut store = Store::open(&path).expect("open");
        commit_corpus(&mut store);
    }
    let before = std::fs::read(&path).expect("read before");

    let table = load_table_from_log(&path).expect("table");
    let _ = verify(&path).expect("verify");
    let mut log = mimir_core::log::CanonicalLog::open(&path).expect("log");
    let bytes = {
        use mimir_core::log::LogBackend;
        log.read_all().expect("read")
    };
    let records = mimir_core::canonical::decode_all(&bytes).expect("decode");
    let renderer = LispRenderer::new(&table);
    for record in &records {
        let _ = renderer.render_memory(record);
    }

    let after = std::fs::read(&path).expect("read after");
    assert_eq!(before, after, "decoder mutated the canonical log");
}

#[test]
fn deterministic_rendering() {
    // Spec § 10 invariant 2: same state + same inputs → byte-
    // identical output. Two independent renders of the same log
    // produce the same strings.
    let dir = TempDir::new().expect("tmp");
    let path = dir.path().join("canonical.log");
    {
        let mut store = Store::open(&path).expect("open");
        commit_corpus(&mut store);
    }
    let table = load_table_from_log(&path).expect("table");
    let mut log = mimir_core::log::CanonicalLog::open(&path).expect("log");
    let bytes = {
        use mimir_core::log::LogBackend;
        log.read_all().expect("read")
    };
    let records = mimir_core::canonical::decode_all(&bytes).expect("decode");
    let renderer = LispRenderer::new(&table);

    let first: Vec<String> = records
        .iter()
        .filter_map(|r| renderer.render_memory(r).ok())
        .collect();
    let second: Vec<String> = records
        .iter()
        .filter_map(|r| renderer.render_memory(r).ok())
        .collect();
    assert_eq!(first, second, "render is not deterministic");
}
