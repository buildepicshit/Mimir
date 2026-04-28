//! As-of query resolver — implements `temporal-model.md` § 7 read
//! semantics over the pipeline's in-memory record history.
//!
//! Query shapes (§ 7.1 – 7.4) collapse into a single bi-temporal
//! form:
//!
//! - `(as_of, as_committed) = (now, now)` — current state (§ 7.1).
//! - `(T, now)` — as-of-valid-time (§ 7.2).
//! - `(now, T_c)` — transaction-time snapshot (§ 7.3).
//! - `(T, T_c)` — retroactive-correction-aware read (§ 7.4).
//!
//! The resolver is a pure projection over `Pipeline::semantic_records`
//! / `Pipeline::procedural_records` plus the supersession DAG. It
//! does **not** mutate the pipeline; multiple readers can call it
//! concurrently against an `Arc<Pipeline>` once that bound lands.
//!
//! Scope (6.4): Semantic and Procedural as-of queries. Episodic
//! queries remain out of scope for the temporal-model graduation.
//! Inferential resolution (§ 5.4) was added in Phase 3.1 of the
//! prime-time roadmap — see [`resolve_inferential`].

use crate::canonical::{InfRecord, ProRecord, SemRecord};
use crate::clock::ClockTime;
use crate::dag::EdgeKind;
use crate::pipeline::Pipeline;
use crate::symbol::SymbolId;

/// Bi-temporal query descriptor. Both fields default to `None`,
/// which means "use the pipeline's current commit watermark" — so an
/// unspecified query is a current-state read.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct TemporalQuery {
    /// Valid-time point: "what was true at this time?"
    /// `None` ⇒ use the pipeline's latest commit as the point.
    pub as_of: Option<ClockTime>,
    /// Transaction-time point: "what did the librarian know by
    /// this time?" `None` ⇒ use the pipeline's latest commit.
    pub as_committed: Option<ClockTime>,
}

impl TemporalQuery {
    /// Current-state read — `(now, now)` per § 7.1.
    #[must_use]
    pub const fn current() -> Self {
        Self {
            as_of: None,
            as_committed: None,
        }
    }

    /// As-of-valid-time read — `(T, now)` per § 7.2.
    #[must_use]
    pub const fn as_of(t: ClockTime) -> Self {
        Self {
            as_of: Some(t),
            as_committed: None,
        }
    }

    /// Transaction-time snapshot — `(now, T_c)` per § 7.3.
    #[must_use]
    pub const fn as_committed(t: ClockTime) -> Self {
        Self {
            as_of: None,
            as_committed: Some(t),
        }
    }

    /// Fully-specified bi-temporal point per § 7.4.
    #[must_use]
    pub const fn bi_temporal(as_of: ClockTime, as_committed: ClockTime) -> Self {
        Self {
            as_of: Some(as_of),
            as_committed: Some(as_committed),
        }
    }
}

/// Resolve the currently-authoritative Semantic memory for `(s, p)`
/// under the temporal query, or `None` if no Semantic memory matches.
///
/// Returns a clone because the resolver is a pure projection — it
/// doesn't hand out references tied to the pipeline's internal
/// vector (which could be invalidated by a subsequent commit).
#[must_use]
pub fn resolve_semantic(
    pipeline: &Pipeline,
    s: SymbolId,
    p: SymbolId,
    query: TemporalQuery,
) -> Option<SemRecord> {
    let (as_of, as_committed) = effective_points(pipeline, query)?;

    // Use the `(s, p) → indices` index so we scan only the history
    // at this specific key — O(k) where k is typically 1–3 — instead
    // of the whole `semantic_records` vec. Per `read-protocol.md`
    // § 3.1 and graduation criterion #4 (p50 < 1ms on a 1M-record
    // warm index).
    let records = pipeline.semantic_records();
    let mut best: Option<&SemRecord> = None;
    for &idx in pipeline.semantic_history_at(s, p) {
        let Some(record) = records.get(idx) else {
            continue;
        };
        if !is_authoritative_sem(pipeline, record, as_of, as_committed) {
            continue;
        }
        best = Some(match best {
            None => record,
            Some(cur) if record.clocks.committed_at > cur.clocks.committed_at => record,
            // Stable tie-break: later committed_at wins. An equal
            // committed_at would collide in memory_id — impossible,
            // because auto-supersession rejects it (§ 5.1 equal
            // valid_at → SemanticSupersessionConflict) — so we
            // preserve the first candidate if this case ever fires.
            Some(cur) => cur,
        });
    }
    best.cloned()
}

/// Resolve the currently-authoritative Inferential memory for
/// `(s, p)` under the temporal query, or `None` if no Inferential
/// matches. Mirrors [`resolve_semantic`] in shape and semantics —
/// per `temporal-model.md` § 5.4 an Inferential re-derivation with
/// the same `(s, p)` and a later `valid_at` supersedes the prior
/// (same rule as Semantic § 5.1).
///
/// Returns a clone for the same reason as [`resolve_semantic`]: the
/// resolver is a pure projection and a subsequent commit may
/// invalidate internal references.
///
/// `StaleParent` edges (§ 5.4's read-time stale-flag overlay) are
/// explicitly NOT consulted here — surfacing them requires a
/// `ReadFlags` bit that `read-protocol.md` § 5 does not yet allocate.
/// That overlay lands with a spec amendment to the flag enum; the
/// resolver's definition of "authoritative" is unchanged by it
/// (stale Inferentials stay authoritative — only the surface flag
/// changes).
#[must_use]
pub fn resolve_inferential(
    pipeline: &Pipeline,
    s: SymbolId,
    p: SymbolId,
    query: TemporalQuery,
) -> Option<InfRecord> {
    let (as_of, as_committed) = effective_points(pipeline, query)?;

    // Same `(s, p) → indices` pattern as Sem — O(k) in the history
    // length at this key rather than O(n) over the whole Inferential
    // record vec.
    let records = pipeline.inferential_records();
    let mut best: Option<&InfRecord> = None;
    for &idx in pipeline.inferential_history_at(s, p) {
        let Some(record) = records.get(idx) else {
            continue;
        };
        if !is_authoritative_inf(pipeline, record, as_of, as_committed) {
            continue;
        }
        best = Some(match best {
            None => record,
            Some(cur) if record.clocks.committed_at > cur.clocks.committed_at => record,
            Some(cur) => cur,
        });
    }
    best.cloned()
}

/// Resolve the currently-authoritative Procedural memory for
/// `rule_id` under the temporal query. Returns `None` if no
/// Procedural with that rule is authoritative at the requested
/// bi-temporal point.
///
/// Scope: keyed by `rule_id` only. The secondary `(trigger, scope)`
/// index from 6.3b is not queried here — v1 read API exposes the
/// primary key.
#[must_use]
pub fn resolve_procedural(
    pipeline: &Pipeline,
    rule_id: SymbolId,
    query: TemporalQuery,
) -> Option<ProRecord> {
    let (as_of, as_committed) = effective_points(pipeline, query)?;

    // `rule_id → indices` index avoids scanning the full history.
    let records = pipeline.procedural_records();
    let mut best: Option<&ProRecord> = None;
    for &idx in pipeline.procedural_history_for(rule_id) {
        let Some(record) = records.get(idx) else {
            continue;
        };
        if !is_authoritative_pro(pipeline, record, as_of, as_committed) {
            continue;
        }
        best = Some(match best {
            None => record,
            Some(cur) if record.clocks.committed_at > cur.clocks.committed_at => record,
            Some(cur) => cur,
        });
    }
    best.cloned()
}

/// Resolve `(as_of, as_committed)` against the pipeline's current
/// commit watermark. Returns `None` if the pipeline hasn't committed
/// anything yet — queries against an empty pipeline trivially return
/// `None` because there are no records to match.
fn effective_points(pipeline: &Pipeline, query: TemporalQuery) -> Option<(ClockTime, ClockTime)> {
    let watermark = pipeline.last_committed_at()?;
    Some((
        query.as_of.unwrap_or(watermark),
        query.as_committed.unwrap_or(watermark),
    ))
}

/// A Semantic record is authoritative at `(as_of, as_committed)` iff:
///
/// 1. `record.committed_at ≤ as_committed` — the librarian knew about
///    this record by the transaction-time point.
/// 2. `record.valid_at ≤ as_of` — the record's validity started by
///    the valid-time point.
/// 3. `effective_invalid_at(record, as_committed) > as_of` OR None —
///    the record's validity hadn't ended by the valid-time point,
///    considering both retroactive record-level closures (§ 5.1
///    backward case) and forward-case closures derived from
///    `Supersedes` edges committed by `as_committed`.
fn is_authoritative_sem(
    pipeline: &Pipeline,
    record: &SemRecord,
    as_of: ClockTime,
    as_committed: ClockTime,
) -> bool {
    if record.clocks.committed_at > as_committed {
        return false;
    }
    if record.clocks.valid_at > as_of {
        return false;
    }
    let effective_invalid = effective_invalid_at_sem(pipeline, record, as_committed);
    match effective_invalid {
        None => true,
        Some(iv) => iv > as_of,
    }
}

/// An Inferential record is authoritative at `(as_of, as_committed)`.
/// Same shape as Semantic: must be committed by the transaction-time
/// point, valid-time started by the valid-time point, and not closed
/// by own `invalid_at` or a `Supersedes` edge committed by
/// `as_committed`. Per temporal-model.md § 6.2 only `Supersedes`
/// edges close validity — `StaleParent` does not, so the stale flag
/// is orthogonal to authoritativeness.
fn is_authoritative_inf(
    pipeline: &Pipeline,
    record: &InfRecord,
    as_of: ClockTime,
    as_committed: ClockTime,
) -> bool {
    if record.clocks.committed_at > as_committed {
        return false;
    }
    if record.clocks.valid_at > as_of {
        return false;
    }
    let effective_invalid = effective_invalid_at_inf(pipeline, record, as_committed);
    match effective_invalid {
        None => true,
        Some(iv) => iv > as_of,
    }
}

fn effective_invalid_at_inf(
    pipeline: &Pipeline,
    record: &InfRecord,
    as_committed: ClockTime,
) -> Option<ClockTime> {
    let mut candidates: Vec<ClockTime> = Vec::new();
    if let Some(iv) = record.clocks.invalid_at {
        candidates.push(iv);
    }
    collect_edge_closures(pipeline, record.memory_id, as_committed, &mut candidates);
    candidates.into_iter().min()
}

/// A Procedural record is authoritative at `(as_of, as_committed)`.
/// Same shape as Semantic; `valid_at = committed_at` by spec § 4.3,
/// so a Pro record's own `invalid_at` is always `None` (Pro has no
/// retroactive case) and closures come entirely from `Supersedes`
/// edges.
fn is_authoritative_pro(
    pipeline: &Pipeline,
    record: &ProRecord,
    as_of: ClockTime,
    as_committed: ClockTime,
) -> bool {
    if record.clocks.committed_at > as_committed {
        return false;
    }
    if record.clocks.valid_at > as_of {
        return false;
    }
    let effective_invalid = effective_invalid_at_pro(pipeline, record, as_committed);
    match effective_invalid {
        None => true,
        Some(iv) => iv > as_of,
    }
}

/// Compute a Semantic record's effective `invalid_at` as observed at
/// transaction time `as_committed`, combining:
///
/// - The record's own `invalid_at` field (set at write time for
///   retroactive corrections per § 5.1 backward case).
/// - The earliest `Supersedes`-edge-derived closure from any forward
///   supersession known by `as_committed`: for each incoming
///   Supersedes edge `e` with `e.at ≤ as_committed`, look up
///   `e.from`'s `valid_at` and take the minimum.
///
/// Returns the minimum of all sources, or `None` if none apply.
fn effective_invalid_at_sem(
    pipeline: &Pipeline,
    record: &SemRecord,
    as_committed: ClockTime,
) -> Option<ClockTime> {
    let mut candidates: Vec<ClockTime> = Vec::new();
    if let Some(iv) = record.clocks.invalid_at {
        candidates.push(iv);
    }
    collect_edge_closures(pipeline, record.memory_id, as_committed, &mut candidates);
    candidates.into_iter().min()
}

fn effective_invalid_at_pro(
    pipeline: &Pipeline,
    record: &ProRecord,
    as_committed: ClockTime,
) -> Option<ClockTime> {
    let mut candidates: Vec<ClockTime> = Vec::new();
    if let Some(iv) = record.clocks.invalid_at {
        candidates.push(iv);
    }
    collect_edge_closures(pipeline, record.memory_id, as_committed, &mut candidates);
    candidates.into_iter().min()
}

/// For every `Supersedes` edge targeting `target_memory` with
/// `edge.at ≤ as_committed`, push the source memory's `valid_at`
/// into `out`. Per invariant § 6.2 #4, a Supersedes edge closes the
/// target's validity at the source's `valid_at`.
///
/// Sources are looked up in the pipeline's Sem and Pro histories.
/// An edge whose source isn't found is skipped silently — this
/// shouldn't happen against a well-formed log, but skipping is safer
/// than panicking.
fn collect_edge_closures(
    pipeline: &Pipeline,
    target_memory: SymbolId,
    as_committed: ClockTime,
    out: &mut Vec<ClockTime>,
) {
    for edge in pipeline.dag().edges_to(target_memory) {
        if edge.kind != EdgeKind::Supersedes {
            continue;
        }
        if edge.at > as_committed {
            continue;
        }
        if let Some(source_valid_at) = lookup_source_valid_at(pipeline, edge.from) {
            out.push(source_valid_at);
        }
    }
}

/// Find the `valid_at` of a memory by ID, searching Sem, Pro, then
/// Inf histories. Linear in total record count — acceptable for v1;
/// a `memory_id -> record` index is an obvious optimization.
fn lookup_source_valid_at(pipeline: &Pipeline, memory_id: SymbolId) -> Option<ClockTime> {
    for r in pipeline.semantic_records() {
        if r.memory_id == memory_id {
            return Some(r.clocks.valid_at);
        }
    }
    for r in pipeline.procedural_records() {
        if r.memory_id == memory_id {
            return Some(r.clocks.valid_at);
        }
    }
    for r in pipeline.inferential_records() {
        if r.memory_id == memory_id {
            return Some(r.clocks.valid_at);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(v: u64) -> ClockTime {
        ClockTime::try_from_millis(v).expect("non-sentinel")
    }

    fn now() -> ClockTime {
        ms(1_713_350_400_000)
    }

    fn compile(pipe: &mut Pipeline, src: &str) {
        pipe.compile_batch(src, now()).expect("compile");
    }

    fn alice_knows(pipe: &Pipeline) -> (SymbolId, SymbolId) {
        let s = pipe.table().lookup("alice").expect("alice");
        let p = pipe.table().lookup("knows").expect("knows");
        (s, p)
    }

    #[test]
    fn empty_pipeline_resolves_to_none() {
        let pipe = Pipeline::new();
        let q = TemporalQuery::current();
        // Can't even construct s/p symbols — pass in fabricated ones
        // to prove the resolver handles an empty state.
        let got = resolve_semantic(&pipe, SymbolId::new(0), SymbolId::new(1), q);
        assert!(got.is_none());
    }

    #[test]
    fn current_read_returns_latest_forward_supersessor() {
        let mut pipe = Pipeline::new();
        compile(
            &mut pipe,
            "(sem @alice @knows @bob :src @observation :c 0.8 :v 2024-01-15)",
        );
        compile(
            &mut pipe,
            "(sem @alice @knows @carol :src @observation :c 0.8 :v 2024-03-15)",
        );
        let (s, p) = alice_knows(&pipe);
        let got = resolve_semantic(&pipe, s, p, TemporalQuery::current())
            .expect("has authoritative record");
        // @carol's valid_at is later — the head of the forward chain.
        let carol = pipe.table().lookup("carol").expect("carol");
        assert!(matches!(&got.o, crate::Value::Symbol(id) if *id == carol));
    }

    #[test]
    fn as_of_past_valid_time_returns_earlier_record() {
        let mut pipe = Pipeline::new();
        compile(
            &mut pipe,
            "(sem @alice @knows @bob :src @observation :c 0.8 :v 2024-01-15)",
        );
        compile(
            &mut pipe,
            "(sem @alice @knows @carol :src @observation :c 0.8 :v 2024-03-15)",
        );
        let (s, p) = alice_knows(&pipe);

        // Query at 2024-02-15 — between the two valid_at points.
        // @bob was valid, @carol not yet.
        let between = ms(1_707_955_200_000); // 2024-02-15
        let got = resolve_semantic(&pipe, s, p, TemporalQuery::as_of(between))
            .expect("bob valid at 2024-02-15");
        let bob = pipe.table().lookup("bob").expect("bob");
        assert!(matches!(&got.o, crate::Value::Symbol(id) if *id == bob));

        // Query at 2024-01-01 — before either record's valid_at.
        // Nothing authoritative yet.
        let before = ms(1_704_067_200_000); // 2024-01-01
        assert!(resolve_semantic(&pipe, s, p, TemporalQuery::as_of(before)).is_none());
    }

    #[test]
    fn retroactive_record_wins_over_earlier_forward_record_in_overlap() {
        // Three writes (all with valid_at before `now()` = 2024-04-17):
        //   M1: valid_at 2024-01-15 (forward-superseded by M2)
        //   M2: valid_at 2024-04-01 (current)
        //   M3: retroactive at valid_at 2024-03-15, invalid_at=M2's
        // Query at 2024-03-20 — M3 and M1 both overlap:
        //   M1: valid 2024-01-15, edge-closed at M2's 2024-04-01 → still valid at 2024-03-20
        //   M3: valid 2024-03-15 to 2024-04-01 → valid
        // Tie-break by committed_at: M3 > M1, so M3 wins.
        let mut pipe = Pipeline::new();
        compile(
            &mut pipe,
            "(sem @alice @knows @bob :src @observation :c 0.8 :v 2024-01-15)",
        );
        compile(
            &mut pipe,
            "(sem @alice @knows @carol :src @observation :c 0.8 :v 2024-04-01)",
        );
        compile(
            &mut pipe,
            "(sem @alice @knows @dan :src @observation :c 0.8 :v 2024-03-15)",
        );
        let (s, p) = alice_knows(&pipe);
        let mar_20 = ms(1_710_892_800_000); // 2024-03-20
        let got = resolve_semantic(&pipe, s, p, TemporalQuery::as_of(mar_20))
            .expect("dan valid at 2024-03-20");
        let dan = pipe.table().lookup("dan").expect("dan");
        assert!(matches!(&got.o, crate::Value::Symbol(id) if *id == dan));
    }

    #[test]
    fn as_committed_hides_records_committed_after_snapshot() {
        // Two writes; query with as_committed between them — the
        // second write shouldn't be visible.
        let mut pipe = Pipeline::new();
        let t1 = ms(1_713_350_400_000);
        let t2 = ms(1_713_350_500_000);
        pipe.compile_batch(
            "(sem @alice @knows @bob :src @observation :c 0.8 :v 2024-01-15)",
            t1,
        )
        .expect("t1");
        pipe.compile_batch(
            "(sem @alice @knows @carol :src @observation :c 0.8 :v 2024-03-15)",
            t2,
        )
        .expect("t2");
        let (s, p) = alice_knows(&pipe);

        // Current read: sees t2's carol write.
        let now_got = resolve_semantic(&pipe, s, p, TemporalQuery::current()).expect("current");
        let carol = pipe.table().lookup("carol").expect("carol");
        assert!(matches!(&now_got.o, crate::Value::Symbol(id) if *id == carol));

        // as_committed between t1 and t2: t2's write invisible.
        let between = ms(t1.as_millis() + 1);
        let got = resolve_semantic(&pipe, s, p, TemporalQuery::as_committed(between))
            .expect("t1 visible, t2 not");
        let bob = pipe.table().lookup("bob").expect("bob");
        assert!(matches!(&got.o, crate::Value::Symbol(id) if *id == bob));
    }

    #[test]
    fn procedural_current_read_follows_supersession_chain() {
        let mut pipe = Pipeline::new();
        compile(
            &mut pipe,
            r#"(pro @rule_x "t_a" "act_1" :scp @mimir :src @policy :c 1.0)"#,
        );
        compile(
            &mut pipe,
            r#"(pro @rule_x "t_b" "act_2" :scp @other :src @policy :c 1.0)"#,
        );
        let rule = pipe.table().lookup("rule_x").expect("rule_x");
        let got = resolve_procedural(&pipe, rule, TemporalQuery::current()).expect("current pro");
        // Second commit's action wins.
        assert!(matches!(&got.action, crate::Value::String(s) if s == "act_2"));
    }

    #[test]
    fn procedural_as_committed_returns_older_version() {
        let mut pipe = Pipeline::new();
        let t1 = ms(1_713_350_400_000);
        let t2 = ms(1_713_350_500_000);
        pipe.compile_batch(
            r#"(pro @rule_x "t_a" "act_1" :scp @mimir :src @policy :c 1.0)"#,
            t1,
        )
        .expect("t1");
        pipe.compile_batch(
            r#"(pro @rule_x "t_b" "act_2" :scp @other :src @policy :c 1.0)"#,
            t2,
        )
        .expect("t2");
        let rule = pipe.table().lookup("rule_x").expect("rule_x");

        let got =
            resolve_procedural(&pipe, rule, TemporalQuery::as_committed(t1)).expect("t1-era pro");
        assert!(matches!(&got.action, crate::Value::String(s) if s == "act_1"));
    }

    #[test]
    fn bi_temporal_read_returns_pre_correction_view() {
        // Spec § 7.4: at as_committed before the correction, the
        // pre-correction view. Build a chain with a retroactive
        // correction and verify the bi-temporal read ignores it
        // when as_committed predates the correction's commit.
        let mut pipe = Pipeline::new();
        let t1 = ms(1_713_350_400_000);
        let t2 = ms(1_713_350_500_000);
        pipe.compile_batch(
            "(sem @alice @knows @bob :src @observation :c 0.8 :v 2024-01-15)",
            t1,
        )
        .expect("t1 forward base");
        pipe.compile_batch(
            "(sem @alice @knows @carol :src @observation :c 0.8 :v 2024-04-01)",
            t1,
        )
        .expect("t1 forward super");
        // Retroactive correction committed at t2 only.
        pipe.compile_batch(
            "(sem @alice @knows @dan :src @observation :c 0.8 :v 2024-03-15)",
            t2,
        )
        .expect("t2 retroactive");
        let (s, p) = alice_knows(&pipe);
        let mar_20 = ms(1_710_892_800_000); // 2024-03-20

        // Post-correction bi-temporal read at 2024-03-20:
        // retroactive dan wins.
        let post = resolve_semantic(&pipe, s, p, TemporalQuery::bi_temporal(mar_20, t2))
            .expect("post-correction");
        let dan = pipe.table().lookup("dan").expect("dan");
        assert!(matches!(&post.o, crate::Value::Symbol(id) if *id == dan));

        // Pre-correction bi-temporal read at 2024-03-20 with
        // as_committed right after t1 — dan not yet committed. M1
        // (bob) is the pre-correction truth at 2024-03-20 because M2
        // (carol, valid_at=2024-04-01) doesn't start until after.
        // Use t1 + 2 so monotonic-enforced committed_at bumps for the
        // two t1-batches land below this snapshot.
        let pre = resolve_semantic(
            &pipe,
            s,
            p,
            TemporalQuery::bi_temporal(mar_20, ms(t1.as_millis() + 2)),
        )
        .expect("pre-correction");
        let bob = pipe.table().lookup("bob").expect("bob");
        assert!(matches!(&pre.o, crate::Value::Symbol(id) if *id == bob));
    }
}
