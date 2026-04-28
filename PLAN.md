# Mimir — Execution Plan to Feature Complete

> Written 2026-04-18. Single-writer-per-workspace, Claude-targeted. Every item here is work against the post-scope-reduction specs in `docs/concepts/` (see PR #38).

## Step 0 — Merge open PRs ✓
- [x] PR #37 — phase/7.1-hot-path-reader
- [x] PR #38 — spec/scope-reduction-single-agent-claude

## Step 1 — Spec-sync code cleanup ✓ (PR #39)
All cross-workspace / SSI dead surfaces removed; code now matches the amended specs.

## Step 2 — Read-protocol graduation ✓ (PRs #40–#43)

### 7.2 — Filter predicates + flags + framing ✓ (PR #40)
### 7.3 — Decay integration into read ✓ (PR #41)
### 7.4 — Property tests ✓ (PR #42)
### 7.5 — Current-state index + load test + graduation ✓ (PR #43)
- Current-state index (`Pipeline::semantic_by_sp_history`, `procedural_by_rule_history`) — O(k) lookup
- Criterion bench: p50 ≈ 0.57 µs on 1 M-memory warm index (criterion #4 cleared by ~1,750×)
- `read-protocol.md` banner: `authoritative` as of 2026-04-18

## Step 3 — Episode-semantics graduation (in progress)

### 8.1 — Episode-scoped read predicates ✓ (PR #44)
`:in_episode`, `:after_episode`, `:before_episode` wired against the existing mechanical-Episode model. `Pipeline::register_episode` + accessor.

### 8.2 — `EpisodeMeta` canonical record + `:episode_chain` ✓ (PR #45)
New `EpisodeMeta = 0x21` opcode carries label / `parent_episode_id` / retracts. `Store::commit_batch_with_metadata` is the agent-facing entry point; replay restores the index. Reader wires `:episode_chain @E` walking parent links.

### 8.3 — Write-surface explicit Episode forms + graduation ✓
- [x] `(episode :start :label "x" :parent_episode @E :retracts (@E1 @E2))` form — parser + binder + semantic validation
- [x] `(episode :close)` form (no-op on the single-compile_batch-per-Episode model; parses valid)
- [x] Retraction-pattern persisted in `EpisodeMeta`; property test covers cross-Episode supersession
- [x] `episode-semantics.md` banner flipped to authoritative
- **Timeout auto-close** (§ 3.3) and **post-hoc label update** (§ 10.4) explicitly amended to post-MVP — see § 12.2 non-goals in the amended spec.

## Step 4 — Pin / Authoritative write surface
Canonical opcodes (`0x35`–`0x38`) exist per `ir-canonical-form.md`; the write surface and semantic enforcement don't.

- [ ] Write-surface forms: `(pin @mem)`, `(unpin @mem)`, `(authoritative-set @mem)`, `(authoritative-clear @mem)`
- [ ] Parser + binder + semantic validation
- [ ] `Framing::Authoritative { set_by }` populated at read time from pin/auth state
- [ ] Audit Episodic emissions per `confidence-decay.md` § 8.2

## Step 5 — Tracked deferred issues
All five are single-workspace-valid.

- [x] **#33** Fuzz harness (lex + canonical decoder) — do first; it's a safety net
- [x] **#29** Inferential staling (`temporal-model.md` § 5.4): reverse-parent index + StaleParent edge emission (write-path); stale-flag read-time overlay deferred until Inferential resolver lands
- [x] **#31** `HalfLife` newtype refactor
- [x] **#32** `MemoryFlags` split per-kind — kind-specific `SemFlags` / `InfFlags`, Epi/Pro drop the flags byte entirely (wire-format schema break; `ir-canonical-form.md` § 5.5 documents layout)
- [x] **#30** Tracing spans + observability — `engram.pipeline.compile_batch` + `engram.commit.batch` spans, `engram.supersession` / `engram.dag.cycle_rejected` / `engram.recovery.*` events, `docs/observability.md` locks the contract, `mimir-cli` ships a default stderr subscriber

## Step 6 — Wire-architecture graduation
- [x] Scope resolved (2026-04-19): **in-process library only**. Daemon mode, async queue, status channel, `:read_after` predicate, socket protocols, network transport all dropped as out-of-scope under Claude-single-writer. See `wire-architecture.md` § 2 design thesis + § 10 non-goals for the first-principles reasoning.
- [x] Implementation was already built — `Store::commit_batch` / `Pipeline::execute_query` satisfy the graduated spec as-is. No new code required.
- [x] Integration tests: existing `Store` round-trip / replay tests + `crates/mimir-cli/tests/round_trip.rs` cover § 9 invariants (sync commit, err-is-noop, ok-is-durable, EpisodeId correlation).
- [x] `wire-architecture.md` banner flipped to authoritative; consequent amendments to `decoder-tool-contract.md`, `librarian-pipeline.md`, `read-protocol.md`, `write-protocol.md`, `ir-write-surface.md`, `workspace-model.md`, `docs/concepts/README.md`, `docs/attribution.md`.

## Done state
- All 14 remaining specs authoritative (the 15th, `multi-agent-coherence.md`, was deleted in the scope reduction)
- Workspace isolation structural, single-writer enforced
- Every originally-specified feature minus multi-agent is implemented
- Zero tracked deferred issues
- CI green; property + load tests cover every invariant
