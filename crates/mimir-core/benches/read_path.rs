//! Read-path load tests for `read-protocol.md` graduation criterion #4:
//! p50 < 1 ms for a single-predicate Semantic lookup against a
//! warm 1M-memory index.
//!
//! The bench bypasses `compile_batch` during setup (parsing 1M forms
//! would take minutes) and populates the pipeline via the same
//! `replay_allocate` / `replay_memory_record` path that `Store::open`
//! uses for log replay. Measurement is on the query path only.

#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use mimir_core::canonical::{CanonicalRecord, Clocks, SemFlags, SemRecord};
use mimir_core::pipeline::Pipeline;
use mimir_core::{ClockTime, Confidence, SymbolKind, Value};

const ONE_MILLION: usize = 1_000_000;

/// Per-record timestamp offsets — all constant so record construction
/// stays fast. Decay is not exercised by this bench (the hot path is
/// the index lookup + resolver tie-break).
const BASE_VALID_AT_MS: u64 = 1_700_000_000_000;
const BASE_COMMITTED_AT_MS: u64 = 1_710_000_000_000;

fn build_warm_pipeline(n: usize) -> Pipeline {
    let mut pipe = Pipeline::new();
    let confidence = Confidence::try_from_f32(0.9).unwrap();
    let valid_at = ClockTime::try_from_millis(BASE_VALID_AT_MS).unwrap();

    // Two symbols per record (a distinct subject + predicate) plus
    // one shared source symbol. IDs start at 0; we track them by hand
    // since `replay_allocate` sets the ID we pass.
    let source_id = mimir_core::SymbolId::new(0);
    pipe.replay_allocate(source_id, "observation".into(), SymbolKind::Agent)
        .unwrap();
    // Share a single object symbol — tests don't need distinct objects.
    let object_id = mimir_core::SymbolId::new(1);
    pipe.replay_allocate(object_id, "obj".into(), SymbolKind::Literal)
        .unwrap();

    // Allocate n subjects + n predicates, IDs [2 .. 2 + 2n).
    for i in 0..n {
        let s_id = mimir_core::SymbolId::new(2 + (2 * i) as u64);
        let p_id = mimir_core::SymbolId::new(2 + (2 * i + 1) as u64);
        pipe.replay_allocate(s_id, format!("s{i}"), SymbolKind::Agent)
            .unwrap();
        pipe.replay_allocate(p_id, format!("p{i}"), SymbolKind::Predicate)
            .unwrap();
    }

    // Replay n Semantic records. Each uses a distinct (s_i, p_i) so
    // no supersession conflicts arise; every record stays current.
    let mut last_committed = ClockTime::try_from_millis(BASE_COMMITTED_AT_MS).unwrap();
    for i in 0..n {
        let s = mimir_core::SymbolId::new(2 + (2 * i) as u64);
        let p = mimir_core::SymbolId::new(2 + (2 * i + 1) as u64);
        let memory_id = mimir_core::SymbolId::new((2 + 2 * n + i) as u64);
        last_committed = ClockTime::try_from_millis(BASE_COMMITTED_AT_MS + i as u64).unwrap();
        let record = CanonicalRecord::Sem(SemRecord {
            memory_id,
            s,
            p,
            o: Value::Symbol(object_id),
            source: source_id,
            confidence,
            clocks: Clocks {
                valid_at,
                observed_at: last_committed,
                committed_at: last_committed,
                invalid_at: None,
            },
            flags: SemFlags::default(),
        });
        pipe.replay_memory_record(&record);
    }

    // Replay path mirrors `Store::open`, which also has to advance the
    // pipeline's commit watermark so the read path can compute a
    // sensible default `query_committed_at`. Without this the resolver
    // sees `last_committed_at = None` and treats the empty-pipeline
    // branch.
    pipe.advance_last_committed_at(last_committed);

    pipe
}

fn bench_sp_lookup(c: &mut Criterion) {
    let pipe = build_warm_pipeline(ONE_MILLION);
    // Verify one query returns one record before measurement — fail
    // fast if the setup is broken.
    let sanity = pipe
        .execute_query("(query :s @s500000 :p @p500000)")
        .unwrap();
    assert_eq!(sanity.records.len(), 1, "bench setup is broken");

    // Rotate across a handful of distinct `(s, p)` pairs so the bench
    // doesn't just hit a hot cache line.
    let probes: Vec<String> = (0..16)
        .map(|i| {
            let j = i * (ONE_MILLION / 16);
            format!("(query :s @s{j} :p @p{j})")
        })
        .collect();

    let mut group = c.benchmark_group("read_path_1m");
    group.throughput(Throughput::Elements(1));
    group.bench_function("single_predicate_sem_lookup", |b| {
        let mut idx = 0_usize;
        b.iter(|| {
            let q = &probes[idx % probes.len()];
            let got = pipe.execute_query(q).unwrap();
            idx += 1;
            // `std::hint::black_box` so the optimizer can't hoist.
            std::hint::black_box(got);
        });
    });
    group.finish();
}

criterion_group!(benches, bench_sp_lookup);
criterion_main!(benches);
