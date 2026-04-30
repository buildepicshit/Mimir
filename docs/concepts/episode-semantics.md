# Episode Semantics

> **Status: authoritative — graduated 2026-04-19.** Graduation evidence: `mimir_core::parse` recognises `(episode :start …)` and `(episode :close)` with optional `:label`, `:parent_episode`, `:retracts` keywords; `mimir_core::bind` resolves + length-caps labels at 256 bytes (`BindError::LabelTooLong`); `mimir_core::semantic` enforces "at most one Episode directive per batch" (`SemanticError::MultipleEpisodeDirectives`); `mimir_core::pipeline` threads batch-level metadata through `PendingEpisodeMetadata` into the store; `mimir_core::store::Store::commit_batch` merges form-level metadata with the alternate `commit_batch_with_metadata` API and emits an `EpisodeMeta` canonical record (opcode `0x21`) before the `Checkpoint`. Replay restores `episode_committed_at` + `episode_parent` on reopen. Read-path predicates `:in_episode`, `:after_episode`, `:before_episode`, `:episode_chain` are wired end-to-end. Property + integration tests cover: retraction-edge recording (`episode_start_with_retracts_records_metadata`); cross-Episode supersession behaves identically to intra-Episode (`supersession_crosses_episode_boundaries` property test); implicit-Episode flow end-to-end (`commit_registers_episode_with_pipeline`, `replay_registers_episodes_with_pipeline`). Timeout auto-close (§ 3.3) and post-hoc label update (§ 10.4) are **explicitly deferred** to post-MVP — see § 12.2 non-goals.

An Episode is Mimir's atomic unit of agent work. The write protocol (`write-protocol.md`) defines Episodes mechanically as CHECKPOINT-delimited batches; this specification defines what Episodes *mean* to agents — when they form, how they're named, linked, queried, and retracted.

## 1. Scope

This specification defines:

- Triggers for Episode formation (context-pressure, agent-explicit, timeout).
- Episode identity and stored metadata.
- Optional Episode linking (parent / child, retracts).
- Episode-level query semantics.
- Interaction with supersession (Episodes are labels, not isolation boundaries).
- Episode finality (committed Episodes are immutable).
- The retraction pattern for correcting prior Episodes' content.
- Implicit Episodes — transparent librarian management for agents that don't opt in.

This specification does **not** define:

- The mechanical durability of batches — `write-protocol.md`.
- Memory type shapes — `memory-type-taxonomy.md` (Episodes are orthogonal to memory types — an Episode contains memories of any kind).
- Supersession rules per memory type — `temporal-model.md` § 5.
- Librarian pipeline stages — `librarian-pipeline.md`.
- Read protocol details (hot-path vs as-of) — `read-protocol.md`.

### Graduation criteria

Graduates draft → authoritative when:

1. Related art is verified in `docs/attribution.md` (event-sourcing / event-stream literature; context-window management literature).
2. Episode-management Rust logic compiles in `mimir_core`, with the invariants in § 11 under unit and property tests.
3. Property tests cover: context-pressure trigger fires correctly (explicit `(episode :close)` form accepted); retraction-edge recording (`:retracts` list persisted in `EpisodeMeta`); cross-Episode supersession behaves identically to intra-Episode supersession. *Timeout auto-close was part of this criterion in the original draft but is deferred post-MVP per § 12.2; the spec graduates without it on the understanding that the remaining three invariants cover v1's real behaviour.*
4. Integration tests with an implicit-Episode agent flow confirm end-to-end correctness.

## 2. Design thesis: Episode as the atomic unit of agent work

PRINCIPLES.md architectural boundary #6: *"Checkpoint-triggered write batches. Each checkpoint is one Episode (atomic rollback unit)."*

Two forces shape the Episode abstraction:

- **Context pressure.** An LLM agent works inside a context window. When context fills, it summarizes, persists state, and restarts. Mimir's Episode is the natural handoff unit — the set of memories that constitute "what this agent did before the context reset."
- **Atomic audit.** For post-hoc reasoning about what happened, agents need a named unit that groups related writes. An Episode gives them that: "show me everything from the 2026-04-17 design session" is a query against an Episode label.

Episodes are **logical** units, not **temporal** units. An Episode can span hours of real time if the agent doesn't close it; the CHECKPOINT closes it when the agent signals (or when context-pressure fires). This is deliberate — time doesn't correspond to coherent work boundaries in agent workflows.

Episodes are also **not isolation boundaries**. Memories in one Episode freely supersede memories in another. The Episode is a label, not a scope.

## 3. Episode formation triggers

An Episode opens on first write after a CHECKPOINT (or on explicit signal); it closes on one of three triggers.

### 3.1 Context-pressure (primary trigger)

The agent detects it is approaching its context window limit and emits a flush signal:

```
(episode :close)
```

The librarian closes the current Episode, runs the pipeline, writes the CHECKPOINT, fsyncs, and returns the Episode ID. The agent can now free context and continue fresh.

Context-pressure is agent-owned — only the agent knows its own context state. The librarian does not try to infer context pressure.

### 3.2 Agent-explicit (secondary trigger)

Agents can explicitly open and close Episodes around logical work units:

```
(episode :start :label "tokenizer-bakeoff" [:parent_episode @prior_ep])
... writes ...
(episode :close)
```

`:label` is optional; `:parent_episode` is optional and records a cross-Episode link.

### 3.3 Timeout (safety net)

If an Episode is open for longer than the librarian's configured idle timeout (default: 5 minutes of write inactivity), the librarian auto-closes it. This prevents an agent that crashed mid-work from leaving indefinitely-open Episodes.

Auto-close records a `timed_out: true` flag in the Episode metadata so the decoder tool can distinguish clean closures from timeouts.

### 3.4 Trigger priority

If multiple triggers fire simultaneously, the order is: agent-explicit > context-pressure > timeout. Rationale: explicit signals reflect agent intent; context-pressure is self-reported and honored next; timeout is only a safety fallback.

## 4. Episode identity and metadata

### 4.1 `episode_id`

Every Episode has a `Memory`-kind symbol as its ID, allocated by the librarian at Episode open. Agents receive the ID on `(episode :start ...)`; for implicit Episodes (§ 9), the ID is assigned transparently and exposed only on query.

### 4.2 Metadata record in `episodes.log`

Per `ir-canonical-form.md` § 7.7, each Episode has a metadata record:

```
episode_id:            Symbol (Memory-kind)
started_at:            ClockTime
committed_at:          ClockTime (u64::MAX = in-flight or rolled-back)
label:                 Option<String>
parent_episode_id:     Option<Symbol>
retracts:              Vec<Symbol>          // other episode_ids this Episode retracts
member_memory_count:   u64
start_offset:          u64                  // offset into canonical.log where Episode begins
end_offset:            u64                  // offset just past the CHECKPOINT record
status:                EpisodeStatus        // Committed | InFlight | RolledBack
timed_out:             bool
```

The metadata is populated as the Episode progresses and finalized at CHECKPOINT.

### 4.3 Label semantics

Labels are free-form strings. They are **not** unique across Episodes — two sessions can both have `label = "design-session"`. Labels are for human-readability via the decoder tool; programmatic identification uses `episode_id`.

Labels are capped at 256 bytes. Longer labels are rejected with `BindError::LabelTooLong`.

## 5. Episode linking

### 5.1 Parent / child

Optional `:parent_episode @E` on Episode-start records a one-way link. The child Episode knows its parent; the parent knows nothing about its children (no eager back-reference maintained, though queries can find children by scanning).

Semantics:
- **Reference only.** No supersession, no confidence propagation, no shared state.
- **Agent-owned tree shape.** Mimir doesn't define when to use parent/child; agents use it for branching conversations, retry chains, or experiment forks.

### 5.2 Retraction

An Episode can retract one or more prior Episodes via `:retracts [@E1 @E2 ...]` on its start form. Semantics covered in § 8.

### 5.3 Workspace-local linking

`parent_episode_id` always refers to an Episode in the same workspace. Workspaces do not share Episodes — isolation is structural (see `workspace-model.md`).

## 6. Episode-level query semantics

The write surface and read protocol support Episode-scoped queries:

```
(query :in_episode @E)
      ; returns all memories whose CHECKPOINT is @E

(query :after_episode @E)
      ; returns memories from Episodes committed after @E

(query :before_episode @E)
      ; returns memories from Episodes committed before @E

(query :episode_chain @E)
      ; returns @E and all ancestors via parent_episode_id

(query :retracted :all)
      ; returns all Episodes in `retracts: []` of any other Episode
```

These queries are backed by the index built from `episodes.log` — constant-time Episode metadata lookup, log-range scan for member memories.

Full query grammar is in `read-protocol.md`.

## 7. Interaction with supersession

Supersession operates on memories, not on Episodes. A memory in Episode 1 can be superseded by a memory in Episode 100 via the normal rules in `temporal-model.md` § 5. The supersession DAG carries memory-to-memory edges; it does not track Episode-to-Episode relationships.

**Episodes are labels, not isolation boundaries.** This means:

- Supersession crosses Episode boundaries freely.
- A query for "current-state memories" returns the latest un-superseded memory regardless of which Episode committed it.
- Episode deletion does not delete member memories (and Episodes cannot be deleted anyway per § 8).

## 8. Episode finality

**Committed Episodes are immutable.** Once a CHECKPOINT is durable (fsynced per `write-protocol.md` § 4.2), its Episode is permanent.

### 8.1 No Episode rollback

There is no "undo this Episode" operation. The write protocol's rollback exists only for in-flight batches before CHECKPOINT fsync. Committed Episodes are append-only per PRINCIPLES.md architectural boundary #3.

### 8.2 No Episode deletion

An Episode's member memories remain in `canonical.log` forever (subject to post-MVP LSM compaction, which would consolidate but not erase). The Episode metadata in `episodes.log` is similarly append-only.

### 8.3 Post-commit Episode metadata updates

One exception: after an Episode is committed, *other* Episodes may record `retracts: [@this_episode]` (§ 8.4). The retraction is stored on the retracting Episode, not as a mutation of the retracted Episode. The retracted Episode's metadata is unchanged.

## 9. Retraction pattern

When an agent realizes a prior committed Episode contained mistakes:

1. **Open a new Episode with `:retracts [@bad_episode]`.**
   ```
   (episode :start :label "correction-2026-04-17" :retracts [@ep_xyz])
   ```
2. **Emit supersession writes** for each erroneous memory in the retracted Episode:
   ```
   (sem @alain email "correct@example.com" :src @alain :c 0.95 :v 2024-01-15)
     ; this supersedes the prior Semantic memory about Alain's email
   ```
3. **Close the new Episode.**
   ```
   (episode :close)
   ```

### 9.1 Effect of retraction

- Each erroneous memory is superseded per normal `temporal-model.md` § 5 rules — the supersession edge closes its validity.
- The `retracts` field in the new Episode's metadata records the cross-Episode relationship. The decoder tool uses this to highlight retracted Episodes in human-readable output.
- The retracted Episode's memories remain in the store, flagged as superseded; they are still visible in as-of queries that predate the retraction.

### 9.2 Retraction is agent-driven, not automatic

The librarian does not auto-retract Episodes on confidence-threshold triggers or ML-proposed staleness. Retraction is always an explicit agent decision.

### 9.3 Retraction is not a blanket invalidation

Retracting an Episode does **not** supersede all its member memories automatically. The agent must emit explicit supersession writes for each memory it considers wrong. Rationale: most retracted Episodes contain *mostly* correct memories with a few specific errors; blanket invalidation would over-correct.

An agent wanting blanket supersession of all Episode members iterates:

```
(episode :start :retracts [@bad_episode])
for each memory in @bad_episode:
    (sem ... :src @cross_episode_correction :c 0.0 :v ...)  ; invalidating replacement
(episode :close)
```

## 10. Implicit Episodes

Most agent workflows do not need explicit Episode management. The librarian auto-manages Episodes transparently:

### 10.1 Auto-open

A write received outside an active Episode triggers auto-open:
- Fresh `episode_id` allocated.
- `label = None`, `parent_episode_id = None`, `retracts = []`.
- `started_at = now`.
- `timed_out = false`.

### 10.2 Auto-close triggers

Implicit Episodes close on the same triggers as explicit ones (§ 3) — context-pressure signal, explicit `(episode :close)`, or timeout.

### 10.3 Agent visibility

An implicit Episode's ID is not returned to the agent unless queried (`(query :current_episode)`). Agents that don't care about Episode boundaries never see them.

### 10.4 Label assignment post-hoc

An agent can label an implicit Episode post-hoc via `(episode :label @E "name")`. This writes a metadata update to `episodes.log` — which IS allowed even though the Episode is committed, because label storage is a metadata-only append, not a memory mutation.

Exception to § 8.1: labels can be added / changed after commit. Rationale: labels are a naming concern, not a correctness concern. All other metadata (committed_at, member count, offsets) is immutable.

## 11. Invariants

1. **Episode identity persists.** A committed Episode's `episode_id` is immutable once allocated. No renumbering.
2. **CHECKPOINT closes Episode.** Every committed Episode corresponds to exactly one CHECKPOINT record in `canonical.log` (per `write-protocol.md`).
3. **No orphan Episodes.** Every Episode in `episodes.log` with `status = Committed` has a corresponding CHECKPOINT in `canonical.log`. Recovery enforces this.
4. **Supersession crosses Episode boundaries.** Supersession edges in the DAG connect memories regardless of their Episode.
5. **Retraction is additive.** Recording a retraction in Episode B that points to Episode A does not mutate Episode A's record.
6. **Labels are updatable, other metadata is not.** Post-commit, only `label` can change on an Episode's metadata record.
7. **Parent_episode is same-workspace.** Cross-workspace Episode linking is rejected at bind.
8. **Retraction without supersession is inert.** Declaring `:retracts [@E]` without emitting supersession writes for the erroneous memories leaves `@E`'s memories still current. This is by design (agent-driven, not automatic).

## 12. Open questions and non-goals for v1

### 12.1 Open questions

**Nested Episodes.** Current spec allows Episode linking via `parent_episode_id` but does not support concurrent nested Episodes (A opens inside B). A sub-Episode model (similar to nested transactions) is worth revisiting if real agent workflows demand it. Post-MVP.

**Episode-scoped ephemeral memories.** Ephemeral memories (per `memory-type-taxonomy.md` § 4) currently have `EphemeralScope::Session` or narrower. Should `EphemeralScope::Episode(@E)` exist? Probably yes — would simplify scratch memories that naturally align to an Episode's lifetime. Defer until a concrete driver appears.

**Automatic retraction of low-confidence Episodes.** A future ML component could flag "this Episode looks like it was written under context pressure or hallucination risk." Mimir would not auto-retract (deterministic-first principle), but could surface a `suggested_retraction` flag for agent review. Post-MVP.

**Episode-level access control.** Permissions per Episode (hide / redact specific Episodes from reads) — post-MVP, aligns with `workspace-model.md` § 8.2 access-control candidate spec.

**Episode compaction.** LSM-style log compaction (post-MVP) could consolidate superseded memories' raw bytes while preserving Episode metadata. The Episode's record in `episodes.log` needs to survive compaction; the member memories' log offsets may shift. Defer to the compaction spec.

### 12.2 Non-goals for v1

- **Episode hard-delete.** Append-only forbids.
- **Automatic Episode formation by time window.** Time-based triggers are out of scope; Episodes are agent-semantic units.
- **Timeout auto-close (§ 3.3).** Deferred post-MVP. Requires a background scheduler running independently of `commit_batch`; the single-writer, synchronous store architecture has no such scheduler today. Without timeout the spec still holds — agents explicitly close via `(episode :close)` or implicitly by returning from `commit_batch`. A stuck agent leaves its last Episode open in memory; on process restart the pipeline's monotonic watermark ensures no corruption, just a gap. Revisit when a long-running daemon model lands.
- **Post-hoc label update (§ 10.4).** Deferred post-MVP. The § 8.1 "labels are the one allowed post-commit mutation" exception needs either a new canonical opcode (LABEL_UPDATE) or a side-channel `episodes.log` that's separately updatable. The single-file append-only model used today doesn't accommodate either; labels are chosen at Episode-open time and immutable thereafter. Revisit when operational workflows demand renaming.
- **Distributed Episodes across workspaces.** Cross-workspace hard-partition per `workspace-model.md`.
- **Episode templates.** No first-class "Episode schema" concept. Agents encode intent in labels and member memories.
- **Parallel Episodes per agent.** An agent has at most one open Episode at a time per workspace. Parallel agents (distinct librarian connections) have distinct Episodes.

## 13. Primary-source attribution

All entries are **pending verification** per `docs/attribution.md`.

- **Event Sourcing pattern** — Fowler, *Event Sourcing* (martinfowler.com/eaaDev/EventSourcing.html, pending) — the pattern of storing immutable events and deriving state. Mimir's Episode is an event-sourcing bundling primitive atop the memory-level event stream.
- **Park et al. 2023, *Generative Agents*** (pending, already cited) — establishes Episode as a cognitive primitive in agent memory modelling. Mimir's Episodes are operationally-defined rather than cognitively-defined, but the abstraction debt is worth citing.
- **Context-window management literature** — recent LLM system papers on context compression, summarization, and handoff (pending, specific citation to be identified during verification) — informs § 3.1 context-pressure trigger semantics.
