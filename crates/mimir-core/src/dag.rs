//! Supersession DAG — in-memory representation of the four edge kinds
//! defined by `docs/concepts/temporal-model.md` § 6.
//!
//! The DAG is a workspace-scoped graph of memory IDs connected by
//! typed edges:
//!
//! - `Supersedes` — closes the target's validity (§ 5.1, § 5.2).
//! - `Corrects` — Episodic-only correction link; target's validity is
//!   preserved (§ 5.3).
//! - `StaleParent` — Inferential-only stale-flag link when a parent
//!   was superseded (§ 5.4).
//! - `Reconfirms` — Inferential re-derivation that confirms the
//!   original conclusion (§ 5.4).
//!
//! Invariants enforced here:
//!
//! 1. **Acyclicity.** The union of all edges forms a DAG. A write
//!    whose edge would close a cycle is rejected with
//!    [`DagError::SupersessionCycle`].
//! 2. **No self-edges.** An edge `from == to` is rejected with
//!    [`DagError::SelfEdge`]. Supersession of `M` by `M` has no
//!    meaningful semantics.
//!
//! Invariants deferred to the bind layer (milestone 6.3):
//!
//! - **Source precedes target in transaction time** (§ 6.2 invariant
//!   #2). The DAG does not store memory metadata; the caller
//!   (auto-supersession logic in `bind`) already has `committed_at`
//!   for both endpoints and checks this before calling `add_edge`.
//! - **Supersedes closes validity** (§ 6.2 invariant #3). Setting the
//!   target's `invalid_at` is a memory-record mutation, not a DAG
//!   concern.

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::canonical::CanonicalRecord;
use crate::clock::ClockTime;
use crate::symbol::SymbolId;

/// Kind of supersession edge.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    /// `from supersedes to` — target's validity closes at this edge.
    Supersedes,
    /// `from corrects to` — Episodic-only; target stays valid.
    Corrects,
    /// `from has-stale-parent to` — Inferential-only; flags `from`
    /// stale when `to` is superseded.
    StaleParent,
    /// `from reconfirms to` — Inferential re-derivation confirming
    /// the original conclusion.
    Reconfirms,
}

/// One edge in the supersession DAG.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Edge {
    /// Edge kind.
    pub kind: EdgeKind,
    /// Source memory.
    pub from: SymbolId,
    /// Target memory.
    pub to: SymbolId,
    /// Timestamp the edge was applied.
    pub at: ClockTime,
}

impl Edge {
    /// Build an [`Edge`] from a canonical edge-family record.
    /// Returns `None` for non-edge variants.
    #[must_use]
    pub fn try_from_record(record: &CanonicalRecord) -> Option<Self> {
        let (kind, r) = match record {
            CanonicalRecord::Supersedes(r) => (EdgeKind::Supersedes, r),
            CanonicalRecord::Corrects(r) => (EdgeKind::Corrects, r),
            CanonicalRecord::StaleParent(r) => (EdgeKind::StaleParent, r),
            CanonicalRecord::Reconfirms(r) => (EdgeKind::Reconfirms, r),
            _ => return None,
        };
        Some(Self {
            kind,
            from: r.from,
            to: r.to,
            at: r.at,
        })
    }
}

/// Errors produced by [`SupersessionDag`] mutations.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum DagError {
    /// The edge would close a cycle in the DAG. Per
    /// `temporal-model.md` § 6.2 invariant #1, cycles are forbidden.
    #[error("supersession cycle: {kind:?} edge {from:?} -> {to:?} would close a cycle")]
    SupersessionCycle {
        /// Source of the rejected edge.
        from: SymbolId,
        /// Target of the rejected edge.
        to: SymbolId,
        /// Kind of the rejected edge.
        kind: EdgeKind,
    },

    /// The edge's endpoints are identical (`from == to`). Supersession
    /// of a memory by itself has no meaningful semantics.
    #[error("self-edge is forbidden: {kind:?} edge on {memory:?}")]
    SelfEdge {
        /// The memory that appeared on both sides.
        memory: SymbolId,
        /// Edge kind.
        kind: EdgeKind,
    },
}

/// Workspace-scoped supersession graph.
///
/// Holds the four edge kinds in a single indexed structure: a flat
/// list of edges plus outgoing / incoming adjacency indices keyed by
/// memory ID. Acyclicity is enforced at every `add_edge` call via a
/// forward-reachability DFS from the proposed target.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SupersessionDag {
    edges: Vec<Edge>,
    outgoing: BTreeMap<SymbolId, Vec<usize>>,
    incoming: BTreeMap<SymbolId, Vec<usize>>,
}

impl SupersessionDag {
    /// Construct an empty DAG.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an edge to the DAG.
    ///
    /// # Errors
    ///
    /// - [`DagError::SelfEdge`] if `edge.from == edge.to`.
    /// - [`DagError::SupersessionCycle`] if the edge would close a
    ///   cycle (i.e. `edge.to` already reaches `edge.from` via the
    ///   existing edge set of any kind).
    pub fn add_edge(&mut self, edge: Edge) -> Result<(), DagError> {
        if edge.from == edge.to {
            return Err(DagError::SelfEdge {
                memory: edge.from,
                kind: edge.kind,
            });
        }
        if self.reaches(edge.to, edge.from) {
            tracing::warn!(
                target: "mimir.dag.cycle_rejected",
                from = %edge.from,
                to = %edge.to,
                edge_kind = ?edge.kind,
                "supersession cycle rejected",
            );
            return Err(DagError::SupersessionCycle {
                from: edge.from,
                to: edge.to,
                kind: edge.kind,
            });
        }
        let idx = self.edges.len();
        self.outgoing.entry(edge.from).or_default().push(idx);
        self.incoming.entry(edge.to).or_default().push(idx);
        self.edges.push(edge);
        Ok(())
    }

    /// Forward reachability: does any path from `start` reach `target`
    /// by following `from -> to` edges? Linear in DAG size.
    fn reaches(&self, start: SymbolId, target: SymbolId) -> bool {
        if start == target {
            return true;
        }
        let mut visited = BTreeSet::new();
        let mut stack = vec![start];
        while let Some(node) = stack.pop() {
            if !visited.insert(node) {
                continue;
            }
            if let Some(indices) = self.outgoing.get(&node) {
                for &i in indices {
                    let next = self.edges[i].to;
                    if next == target {
                        return true;
                    }
                    stack.push(next);
                }
            }
        }
        false
    }

    /// All edges in insertion order.
    #[must_use]
    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// Edge count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.edges.len()
    }

    /// `true` if the DAG has no edges.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    /// Iterator over edges leaving `memory` (i.e. where
    /// `edge.from == memory`). Returned in insertion order.
    pub fn edges_from(&self, memory: SymbolId) -> impl Iterator<Item = &Edge> {
        self.outgoing
            .get(&memory)
            .into_iter()
            .flat_map(move |idxs| idxs.iter().map(move |&i| &self.edges[i]))
    }

    /// Iterator over edges entering `memory` (i.e. where
    /// `edge.to == memory`). Returned in insertion order.
    pub fn edges_to(&self, memory: SymbolId) -> impl Iterator<Item = &Edge> {
        self.incoming
            .get(&memory)
            .into_iter()
            .flat_map(move |idxs| idxs.iter().map(move |&i| &self.edges[i]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(n: u64) -> SymbolId {
        SymbolId::new(n)
    }

    fn at(millis: u64) -> ClockTime {
        ClockTime::try_from_millis(millis).expect("non-sentinel")
    }

    fn edge(kind: EdgeKind, from: u64, to: u64) -> Edge {
        Edge {
            kind,
            from: m(from),
            to: m(to),
            at: at(1_000_000),
        }
    }

    #[test]
    fn empty_dag_has_zero_edges() {
        let dag = SupersessionDag::new();
        assert!(dag.is_empty());
        assert_eq!(dag.len(), 0);
        assert_eq!(dag.edges(), &[]);
        assert!(dag.edges_from(m(0)).next().is_none());
        assert!(dag.edges_to(m(0)).next().is_none());
    }

    #[test]
    fn single_edge_adds_and_indexes() {
        let mut dag = SupersessionDag::new();
        let e = edge(EdgeKind::Supersedes, 1, 2);
        dag.add_edge(e).expect("add");
        assert_eq!(dag.len(), 1);
        assert_eq!(dag.edges(), &[e]);
        let out: Vec<_> = dag.edges_from(m(1)).copied().collect();
        assert_eq!(out, vec![e]);
        let inc: Vec<_> = dag.edges_to(m(2)).copied().collect();
        assert_eq!(inc, vec![e]);
        // Reverse queries are empty.
        assert!(dag.edges_to(m(1)).next().is_none());
        assert!(dag.edges_from(m(2)).next().is_none());
    }

    #[test]
    fn self_edge_is_rejected() {
        let mut dag = SupersessionDag::new();
        let err = dag
            .add_edge(edge(EdgeKind::Supersedes, 7, 7))
            .expect_err("self edge");
        assert_eq!(
            err,
            DagError::SelfEdge {
                memory: m(7),
                kind: EdgeKind::Supersedes
            }
        );
        assert!(dag.is_empty(), "failed add must not mutate");
    }

    #[test]
    fn direct_cycle_is_rejected() {
        // 1 -> 2, then 2 -> 1 would form a 2-cycle.
        let mut dag = SupersessionDag::new();
        dag.add_edge(edge(EdgeKind::Supersedes, 1, 2))
            .expect("first");
        let err = dag
            .add_edge(edge(EdgeKind::Supersedes, 2, 1))
            .expect_err("cycle");
        assert!(matches!(
            err,
            DagError::SupersessionCycle {
                from, to, kind: EdgeKind::Supersedes
            } if from == m(2) && to == m(1)
        ));
        assert_eq!(dag.len(), 1, "failed add must not mutate");
    }

    #[test]
    fn indirect_cycle_across_kinds_is_rejected() {
        // 1 -Supersedes-> 2 -Corrects-> 3 -StaleParent-> 1 is a cycle
        // across three edge kinds. Acyclicity is checked over the
        // union of all kinds.
        let mut dag = SupersessionDag::new();
        dag.add_edge(edge(EdgeKind::Supersedes, 1, 2)).expect("a");
        dag.add_edge(edge(EdgeKind::Corrects, 2, 3)).expect("b");
        let err = dag
            .add_edge(edge(EdgeKind::StaleParent, 3, 1))
            .expect_err("3-cycle across kinds");
        assert!(matches!(err, DagError::SupersessionCycle { .. }));
        assert_eq!(dag.len(), 2);
    }

    #[test]
    fn dag_shaped_additions_all_succeed() {
        // 1 -> 2, 1 -> 3, 2 -> 4, 3 -> 4 is a diamond: not a cycle.
        let mut dag = SupersessionDag::new();
        dag.add_edge(edge(EdgeKind::Supersedes, 1, 2)).expect("12");
        dag.add_edge(edge(EdgeKind::Supersedes, 1, 3)).expect("13");
        dag.add_edge(edge(EdgeKind::Supersedes, 2, 4)).expect("24");
        dag.add_edge(edge(EdgeKind::Supersedes, 3, 4)).expect("34");
        assert_eq!(dag.len(), 4);
        // Two edges enter 4; two edges leave 1.
        assert_eq!(dag.edges_to(m(4)).count(), 2);
        assert_eq!(dag.edges_from(m(1)).count(), 2);
    }

    #[test]
    fn edge_try_from_record_matches_every_edge_variant() {
        use crate::canonical::{CanonicalRecord, EdgeRecord};

        let r = EdgeRecord {
            from: m(10),
            to: m(11),
            at: at(42),
        };
        let cases = [
            (CanonicalRecord::Supersedes(r), EdgeKind::Supersedes),
            (CanonicalRecord::Corrects(r), EdgeKind::Corrects),
            (CanonicalRecord::StaleParent(r), EdgeKind::StaleParent),
            (CanonicalRecord::Reconfirms(r), EdgeKind::Reconfirms),
        ];
        for (record, expected_kind) in cases {
            let e = Edge::try_from_record(&record).expect("edge variant");
            assert_eq!(e.kind, expected_kind);
            assert_eq!(e.from, m(10));
            assert_eq!(e.to, m(11));
            assert_eq!(e.at, at(42));
        }
    }

    #[test]
    fn edge_try_from_record_rejects_non_edge_variants() {
        use crate::canonical::{CanonicalRecord, CheckpointRecord};
        let cp = CanonicalRecord::Checkpoint(CheckpointRecord {
            episode_id: m(1),
            at: at(1),
            memory_count: 0,
        });
        assert!(Edge::try_from_record(&cp).is_none());
    }

    #[test]
    fn failed_cycle_add_does_not_corrupt_indices() {
        // Specifically check that the outgoing/incoming indices are
        // not mutated on a rejected add — otherwise subsequent queries
        // would dangle past the end of `edges`.
        let mut dag = SupersessionDag::new();
        dag.add_edge(edge(EdgeKind::Supersedes, 1, 2)).expect("a");
        let _ = dag
            .add_edge(edge(EdgeKind::Supersedes, 2, 1))
            .expect_err("cycle");
        assert_eq!(dag.len(), 1);
        assert_eq!(dag.edges_from(m(1)).count(), 1);
        assert_eq!(dag.edges_from(m(2)).count(), 0);
        assert_eq!(dag.edges_to(m(1)).count(), 0);
        assert_eq!(dag.edges_to(m(2)).count(), 1);
    }
}
