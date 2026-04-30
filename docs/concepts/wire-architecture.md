# Wire Architecture

> **Status: authoritative 2026-04-19; scope amended 2026-04-24.** Graduated from `citation-verified` on 2026-04-19 backed by the shipped `mimir_core::store::Store` API (`commit_batch` / `commit_batch_with_metadata` / `pipeline.execute_query`). The deployment-mode scope call (in-process library only vs. out-of-process daemon) was resolved in favour of **in-process only** — see § 2. The async-queue + status-channel surface described in earlier drafts was dropped as unjustified for the workspace-local single-writer target; the sync-commit semantics described here match what `Store::commit_batch` already does. Scope-aware adapters and cross-scope retrieval are future work under `scope-model.md`.

This specification defines the agent ↔ librarian API contract in Mimir's current single-writer, workspace-local, in-process deployment. "Wire" is a historical name — in v1 there is no wire. The librarian links into the agent process as a library; the "wire" is a `Store::commit_batch` call returning `Result<EpisodeId, StoreError>`. What remains here is the API contract, error model, and correlation semantics that hold at that boundary.

## 1. Scope

This specification defines:

- The single supported deployment mode (in-process library).
- The synchronous commit API surface.
- The synchronous query API surface.
- The error model (typed `Result::Err`, no async status channel).
- Correlation via `EpisodeId` across commit, canonical log, and reads.
- Single-writer-per-workspace invariant at the API boundary.

This specification does **not** define:

- Memory type shapes, grounding, temporal model, symbol identity — upstream specs.
- Librarian pipeline stages — `librarian-pipeline.md`.
- Write durability (WAL, CHECKPOINT fsync, recovery) — `write-protocol.md`.
- Read filtering, snapshot isolation, flag bitset — `read-protocol.md`.
- Observability (tracing spans + events) — `docs/observability.md`.

**Out of scope for this protocol.** Multi-writer arbitration. SSI abort flow. Raw cross-workspace connections. Multi-agent task coordination. Out-of-process daemon (`mimird`). Unix socket protocols. Network transport (TCP / gRPC / HTTP). Async queues + status channels. Authentication / encryption / access control. Scope-aware memory governance is specified separately in `scope-model.md`; it must not be smuggled into this API without an explicit spec update.

### Graduation criteria

Graduates draft → authoritative when:

1. The deployment-mode scope call is resolved. **Done 2026-04-19**: in-process only.
2. The API described in § 3 compiles in `mimir_core` and is exercised by integration tests. **Done**: `Store::commit_batch`, `Store::commit_batch_with_metadata`, `Pipeline::execute_query` shipped across 7.x–9.x milestones; round-trip tests in `crates/mimir-cli/tests/round_trip.rs` and per-kind integration tests throughout cover the surface.
3. The correlation contract (§ 6) holds end-to-end. **Done**: `Store::commit_batch` returns `EpisodeId`, the canonical log's matching `CHECKPOINT` record carries the same `episode_id`, and `EpisodeMeta` (opcode `0x21`) preserves it across restart.
4. Single-writer invariant is structural, not policy. **Done**: `Store::commit_batch` takes `&mut self`; Rust's borrow checker prevents concurrent writers at compile time.

## 2. Design thesis: synchronous commit under single-writer

Agent workflows churn writes. Earlier drafts of this spec proposed an async-queue + status-channel protocol modelled on LMAX-disruptor patterns: enqueue-return-immediately, commit-is-silent, errors-arrive-later-tagged-by-`episode_id`. Under the actual target — one Claude instance, one workspace, one writer, in one process — that surface buys nothing and costs a great deal:

- **There is no arbitration work for a queue to do.** A queue exists to serialize writes from multiple clients; we have one client, and the borrow checker already serializes its access to `&mut Store`.
- **Async-error-channel complexity solves the wrong problem.** The async channel exists so an IPC-boundary failure (connection drop, queue full, daemon crash) can be surfaced back to a fire-and-forget caller. An in-process synchronous call returns `Result::Err` directly — no channel plumbing needed, and errors land at the call site where the agent can reason about them with full context.
- **Non-determinism at an IPC boundary is pure cost.** Socket buffering, scheduler slack, reconnect semantics — each is a source of timing variance that `PRINCIPLES.md`'s determinism-first posture actively tries to eliminate.
- **`:read_after @episode_id` collapses to nothing.** Under async, the agent needs to tell the reader "wait until this write commits." Under sync, every call after a successful `commit_batch` return is already after that Episode's `CHECKPOINT` fsync — the primitive is implicit in the call ordering.
- **Operational surface we don't need.** A daemon implies a supervisor, PID file, socket paths, filesystem permissions, restart policy, health checks. For one writer, none of this pays rent.

The v1 contract is therefore: **one synchronous function per operation.** `Store::commit_batch` writes; `Pipeline::execute_query` (via `Store::pipeline()`) reads. `Result::Err` is the error channel. `EpisodeId` is the correlation key. That's the wire.

## 3. API surface

All types live in `mimir_core`. Agents link the crate directly.

### 3.1 Commit

```rust
impl Store {
    pub fn commit_batch(
        &mut self,
        input: &str,
        now: ClockTime,
    ) -> Result<EpisodeId, StoreError>;

    pub fn commit_batch_with_metadata(
        &mut self,
        input: &str,
        now: ClockTime,
        metadata: &EpisodeMetadata,
    ) -> Result<EpisodeId, StoreError>;
}
```

Semantics per `write-protocol.md`: parse → bind → semantic → emit → append → fsync, atomic per batch via clone-on-write rollback. On success, the returned `EpisodeId` is the same `SymbolId` that appears in the batch's closing `CHECKPOINT` record and in the `EpisodeMeta` record (when non-empty metadata was supplied). On error, no bytes were added to the log and no pipeline state was mutated.

### 3.2 Query

```rust
impl Pipeline {
    pub fn execute_query(&self, query_source: &str) -> Result<ReadResult, ReadError>;
}
```

Read semantics per `read-protocol.md`: parse → resolve → filter → project through the as-of resolver → return records + framings + flags. `&self`, not `&mut self` — reads don't mutate. Snapshot isolation is automatic: every call sees current committed state; nothing in flight exists under single-writer synchronous commit.

Access from a `Store`: `store.pipeline().execute_query(...)`.

### 3.3 `:read_after` is implicit

Earlier drafts defined `:read_after @episode_id` as a read predicate that blocks until the referenced Episode becomes durable. Under synchronous commit, the predicate is unnecessary: a read issued after `commit_batch` has returned `Ok(episode_id)` is, by construction, issued after that Episode's `CHECKPOINT` was fsynced. The predicate is **not implemented**; `read-protocol.md`'s supported-predicate list omits it.

## 4. Single-writer invariant

`Store::commit_batch` takes `&mut self`. A workspace's `Store` value is owned exclusively by one call site at a time; concurrent writers are impossible because Rust's borrow checker rejects the program that would express them.

If two threads / tasks want to write to the same workspace, they share a `Store` behind a `Mutex` / `RwLock` / channel-and-actor — and in doing so they have explicitly serialized their writes. That matches `PRINCIPLES.md` invariant #1 ("one librarian per workspace, single writer") without any wire-level coordination.

For the Claude target this doesn't even come up: one agent, one `Store`, one call stack.

## 5. Error model

`StoreError` is a thiserror-derived enum tagging the failing stage:

```rust
pub enum StoreError {
    Pipeline(PipelineError),         // parse / bind / semantic / emit rejection
    Log(LogError),                   // filesystem / I/O failure (append / sync / truncate)
    InvalidEpisodeMetadata(...),     // label cap, invalid parent ref, etc.
}
```

Structural guarantees:

- **On `Err`, nothing committed.** The log is byte-identical to its pre-call state. The pipeline's in-memory state is byte-identical to its pre-call state (clone-on-write rollback).
- **On `Err`, no partial-write observable.** No reader can see a half-applied batch; no canonical-log consumer can see records past the last durable `CHECKPOINT`.
- **On `Ok(episode_id)`, durability is guaranteed.** `CHECKPOINT` fsync completed before return. A subsequent `Store::open` on the same path will observe the batch.

`ReadError` is analogous for queries: syntactic / type / predicate errors are typed variants returned synchronously at the call site.

## 6. Correlation via `EpisodeId`

Every committed batch has an `EpisodeId` — a Memory-kind `SymbolId` named `__ep_{n}`. It is the universal correlation key:

| Boundary | Where `EpisodeId` appears |
|---|---|
| Agent → Store | Input does not carry it (librarian allocates). |
| Store → Agent | `commit_batch` return value on success. |
| Canonical log | `CHECKPOINT.episode_id` on the closing record of every committed batch. |
| Canonical log | `EpisodeMeta.episode_id` on the metadata record when metadata is non-empty. |
| Pipeline state | `Pipeline::episode_committed_at` / `episode_parent` / `episode_chain` (`read-protocol.md` § 4.1). |
| Read predicates | `:in_episode @ep` / `:after_episode @ep` / `:before_episode @ep` / `:episode_chain true`. |
| Tracing | `mimir.commit.batch` span's `episode_id` field (`docs/observability.md`). |

Across restart: `Store::open`'s replay path reconstructs the pipeline's episode index from the log's `Checkpoint` + `EpisodeMeta` records, so Episode-scoped reads against post-reopen state are bit-identical to pre-crash state.

## 7. Batching

A single call to `commit_batch` receives one UTF-8 `input` string. That string may contain one or more write-surface forms (`ir-write-surface.md`); together they constitute **one Episode**. Batch semantics (`librarian-pipeline.md`):

- All forms in the batch commit together or none do (clone-on-write atomicity).
- The batch's `committed_at` clock is computed once per batch and stamped on every record in it.
- The batch's closing `CHECKPOINT` is the atomic durability boundary (`write-protocol.md` § 3).

Splitting a conceptual operation across multiple `commit_batch` calls gives up atomicity between them. If the agent needs atomicity across N forms, they go in one call.

## 8. Deployment

`Store::open(path)` or `Store::open_in_workspace(data_root, workspace_id)` returns a `Store<CanonicalLog>` backed by a local filesystem log. For tests, `Store::from_backend(backend)` accepts any `LogBackend` impl (`FaultyLog` is used to exercise the `write-protocol.md` § 7 failure matrix).

No daemon. No socket. No network. No multi-process deployment mode is part of v1. A future out-of-process split (if ever motivated) would be a new spec with its own graduation criteria; `wire-architecture.md` would not be the home for it.

## 9. Invariants

1. **Synchronous commit.** `commit_batch` blocks until the batch is fully rolled back or fully durable.
2. **Err-means-no-op.** On any `StoreError`, log + pipeline state are byte-identical to pre-call.
3. **Ok-means-durable.** On `Ok(episode_id)`, the batch's `CHECKPOINT` has been fsynced.
4. **Single writer, structural.** `&mut Store` enforces it at compile time; no runtime coordination needed.
5. **EpisodeId correlation end-to-end.** The `SymbolId` returned from `commit_batch` is the same ID in the log's `CHECKPOINT`, in `EpisodeMeta`, in the pipeline's Episode index, and in every read predicate that references it.
6. **One batch = one Episode.** Every `commit_batch` call produces exactly one `CHECKPOINT`.
7. **No hidden wire.** There is no queue, no async channel, no socket, no frame protocol. The only failure modes an agent sees are the ones `StoreError` variants enumerate.

## 10. Non-goals for v1

The following have durable post-v1 positions on the roadmap but are **not** part of the wire-architecture surface and will not be added to this spec:

- **Out-of-process daemon.** A daemon is motivated only by multi-process deployments or by surviving agent crashes. Neither applies to single-Claude, single-writer, single-process.
- **Network transport.** Cross-machine deployments imply authentication, encryption, framing, and cross-clock correlation — a substantially different architecture.
- **Async queue + status channel.** See § 2 design thesis. A persistent queue would only make sense alongside the daemon, which we've scoped out.
- **Read-push notifications.** No server-sent-events; reads are pull-only via `execute_query`.
- **Multi-writer / SSI.** Explicitly forbidden by `PRINCIPLES.md` invariant #1 and user-level scope.
- **Cross-workspace wire connections.** A `Store` attaches to exactly one workspace.
- **Authentication, authorization, encryption.** Filesystem-permission-gated log + in-process linkage = no adversarial boundary to defend in v1.

If any of these become genuinely needed, they will be introduced in a new spec with its own graduation criteria. They will **not** be retrofitted here, because they are not wire-architecture concerns under the single-writer in-process target.

## 11. Primary-source attribution

Only two lines of prior art are load-bearing for the spec in its graduated form:

- **Rust's `&mut` aliasing rule** (The Rust Reference, Ownership chapter) — the borrow checker is how the single-writer invariant becomes structural rather than policy. Any multi-reader-single-writer shared state in safe Rust satisfies this invariant; `Store` is a specific instance.
- **Synchronous fsync-based durability** (POSIX `fsync(2)`; PostgreSQL WAL-based commit; SQLite's rollback journal) — the idea that a synchronous commit returning success implies an fsynced durable state is standard practice across serious transactional systems. `write-protocol.md` § 3 is where this spec's durability claim is actually discharged.

The LMAX Disruptor pattern, actor-model IPC, Kafka / NATS / Redis wire patterns, and Unix-domain-socket framing conventions that earlier drafts cited are retained only as background reading for whoever eventually writes the post-v1 daemon spec, if that ever happens; they are not attribution sources for the v1 wire architecture as built.
