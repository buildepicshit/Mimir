# Write Protocol

> **Status: authoritative 2026-04-18.** Graduated from `citation-verified` on 2026-04-18 backed by `mimir_core::log::CanonicalLog` + `LogBackend` trait (§ 3 file model, § 6 fsync policy), `mimir_core::store::Store` (§ 4 two-phase commit, § 5 rollback semantics, § 10 crash-recovery algorithm), the `SYMBOL_*` canonical record emission (§ 9 symbol durability), and the § 7 failure-mode crash-injection matrix via the `FaultyLog` test backend (rows 3 / 6 / 7 directly; rows 1 / 2 / 5 / 8 via the recovery-on-next-open path already exercised by reopen tests; row 4 collapses to row 3 or row 5 at the recovery boundary and is physically untestable in user-space). Property test `orphan_truncation_is_idempotent` enforces § 10.2; `symbol_table_replay_reproduces_pre_crash_state` enforces § 1 criterion #4's replay-determinism requirement; `checkpoint_is_atomic_commit_boundary` enforces § 12 invariant 1.

This specification defines how a validated batch from the librarian pipeline becomes durable in a workspace's canonical store. It covers the write path, commit boundaries, crash recovery, and the failure-mode taxonomy. Mimir's write protocol is ARIES-inspired (log-replay-based recovery) but simpler — append-only semantics eliminate the Undo and CLR machinery of classical ARIES.

## 1. Scope

This specification defines:

- The write-path flow from a validated pipeline output to durable canonical state.
- The two-phase commit protocol within a single batch (Episode).
- Rollback semantics on crash mid-batch.
- The failure-mode taxonomy and per-failure agent-visible behavior.
- fsync policy and durability guarantees.
- When and how supersession edges apply.
- Symbol-table mutation durability.
- The startup crash-recovery algorithm.

This specification does **not** define:

- Memory type shapes — `memory-type-taxonomy.md`.
- The pipeline stages that produce validated forms — `librarian-pipeline.md`.
- The canonical-form bytecode — `ir-canonical-form.md`.
- The temporal model (bi-temporal clocks, supersession DAG construction) — `temporal-model.md`.
- The read protocol — `read-protocol.md`.
- The agent API contract (synchronous in-process `Store::commit_batch`) — `wire-architecture.md`.

**Out of scope for this protocol.** Concurrent-writer arbitration, SSI rw-antidependency detection, and raw cross-workspace write paths. Each workspace has a single writer; batches are strictly serial and there is nothing to arbitrate. Scope-aware promotion writes are specified separately in `scope-model.md`.

### Graduation criteria

Graduates draft → authoritative when:

1. ARIES (Mohan et al. 1992), LSM-tree (O'Neil et al. 1996), and relevant fsync / durability literature are verified in `docs/attribution.md`.
2. Rust commit and recovery implementations compile in `mimir_core`, with the invariants in § 12 covered by unit, property, and crash-injection tests.
3. Crash-injection tests cover every failure row in § 7 — each fault is injected, the librarian is restarted, and the resulting store state is verified against the "post-recovery state" column.
4. Property tests cover: orphan truncation is idempotent; CHECKPOINT fsync is the atomic commit boundary; symbol-table replay reproduces the exact pre-crash state.

## 2. Design thesis: append-only + CHECKPOINT = simple recovery

Classical database recovery (ARIES-style) handles three concerns: Redo to replay committed changes, Undo to roll back aborted transactions, and Compensation Log Records (CLRs) to avoid double-undo during recovery.

Mimir's append-only canonical store eliminates two of the three:

- **No Undo.** Records are never overwritten. An aborted batch leaves unreferenced records in the log; they are not "rolled back" — they are truncated (since no CHECKPOINT commits them).
- **No CLRs.** CLRs exist to make Undo idempotent. No Undo means no CLR.

What remains is **Redo via log replay**: on startup, scan forward from the last snapshot + CHECKPOINT to rebuild in-memory state. This is the LSM-tree pattern (append log + periodic snapshot + WAL replay).

The commit boundary is the `CHECKPOINT` record per `ir-canonical-form.md` § 6.5. A batch is committed when its CHECKPOINT is durable (fsynced); otherwise the batch's records are orphans, truncated at recovery.

## 3. WAL-as-canonical-log

Mimir does **not** use a separate WAL file. The `canonical.log` is the WAL. Every record is both log entry and canonical truth; there is no translation step, no dual-write.

Rationale:

- Removes the consistency question of "log says one thing, store says another" by collapsing them.
- Eliminates double-fsync overhead that would otherwise hit both WAL and store.
- Matches LSM-tree design where the log *is* the durable source of truth, and snapshots / indices are derived caches.

The derived caches (`symbols.snapshot`, `symbols.wal`, `dag.snapshot`, `dag.wal`, `episodes.log`) are rebuildable from `canonical.log` alone. If any cache is missing or corrupt, it is regenerated. `canonical.log` is the only file whose corruption would lose data.

## 4. Two-phase commit within a batch

A batch reaching the Emit stage has been fully validated. Making it durable is a two-phase sequence:

### 4.1 Phase 1 — append all records

1. Acquire the workspace's single-writer lock (per PRINCIPLES.md architectural boundary #1).
2. For each record produced by Emit:
   - Append the record's bytes to `canonical.log` (standard OS `write()`).
3. No fsync yet. The records are in the OS page cache; they may or may not be on disk.
4. If any write fails (disk full, I/O error), abort:
   - Truncate the log to its pre-batch length (keep in-memory record of that length from Phase 0).
   - Return `StoreError::DiskFull` or similar to the pipeline; pipeline returns to the agent.
   - Release the writer lock.

### 4.2 Phase 2 — append CHECKPOINT, fsync

1. Append the `CHECKPOINT` record (opcode 0x20 per `ir-canonical-form.md` § 6.5). Body: `episode_id`, commit time, `memory_count`.
2. fsync `canonical.log` to disk.
3. On fsync success: the batch is committed. Release the writer lock. Return `Ok(episode_id)` to the agent.
4. On fsync failure: rare, but possible (hardware error, disk full filling up between write and sync). Conservatively treated as **uncommitted** — truncate the log back to pre-batch length, return `StoreError::CommitFailed`, release the lock.

### 4.3 Post-commit derived-state updates

After a successful CHECKPOINT fsync, the librarian updates derived caches **asynchronously** — they do not block the commit:

- DAG in-memory index: apply new supersession edges.
- Symbol table: apply any new SYMBOL_* events.
- Episodes log: append a committed Episode entry.
- Current-state index: update for the batch's memories.

If the librarian crashes between CHECKPOINT fsync and derived-cache update, recovery regenerates the caches from `canonical.log` replay. Correctness is preserved; only cache freshness suffers briefly.

## 5. Rollback semantics

A "rollback" in Mimir is not an Undo operation — it is log truncation.

### 5.1 Mid-batch rollback

If Phase 1 or Phase 2 fails before the CHECKPOINT fsync completes:

1. The librarian records the log's pre-batch length `L_start` (captured before Phase 1).
2. On failure, truncate the log to `L_start` via `ftruncate(fd, L_start)`.
3. Discard any in-memory pipeline state associated with this batch.
4. Return the appropriate `StoreError::*` to the pipeline.

### 5.2 Recovery rollback

On librarian startup, the recovery algorithm (§ 10) truncates any orphan records (records after the last committed CHECKPOINT) automatically. This handles the crash-during-commit case identically to a mid-batch rollback — truncation at the last committed boundary.

### 5.3 No partial commits

There is no mechanism to commit part of a batch. A batch either reaches CHECKPOINT fsync (fully committed) or does not (fully discarded). This matches Episode atomicity from PRINCIPLES.md architectural boundary #6.

## 6. fsync policy

- **fsync at every CHECKPOINT.** One fsync per batch. Batch-internal records are not separately fsynced.
- **No fsync for derived caches.** `symbols.snapshot` and `dag.snapshot` are rewritten atomically via tempfile + rename; the rename itself carries filesystem ordering guarantees but they are always regeneratable from replay, so a lost snapshot is not a durability failure.
- **No periodic fsync.** Durability is Episode-scoped, not time-scoped. Agents expecting durability wait for the commit response.

### 6.1 Durability SLO

Under `PRINCIPLES.md` § 6 (performance targets), p50 write latency target is < 5 ms wire-receive → append-confirmed. That 5 ms budget includes one fsync. Expected breakdown:

- ~1 ms pipeline (lex / parse / bind / semantic / emit).
- ~3 ms fsync on typical SSDs.
- ~1 ms response serialization and transport.

p99 (< 50 ms) accommodates fsync stalls.

### 6.2 `fdatasync` vs `fsync`

v1 uses `fsync` (full metadata + data). `fdatasync` (data only, no metadata) would be faster but risks losing CHECKPOINT location if file size metadata is lost. v1 prioritizes correctness over latency.

## 7. Failure-mode taxonomy

| Failure | Pre-state | Post-recovery state | Agent-visible |
|---|---|---|---|
| Crash before Phase 1 append | Clean | Clean (nothing written) | Write timed out; agent retries |
| Crash during Phase 1 (partial records, no CHECKPOINT) | Records partially in page cache / on disk | Clean (orphans truncated) | Write timed out; agent retries |
| Crash between last record and CHECKPOINT append | All records on disk, no CHECKPOINT | Clean (orphans truncated) | Write timed out; agent retries |
| Crash between CHECKPOINT append and fsync | CHECKPOINT in page cache, unknown on-disk status | Uncommitted if CHECKPOINT not on disk; committed if CHECKPOINT made it. Recovery truncates orphans or respects CHECKPOINT. | Write may have succeeded or failed; agent must check (idempotent retry safe) |
| Crash after CHECKPOINT fsync success | All records + CHECKPOINT durable | Committed | Agent received `Ok(episode_id)` before crash; no retry needed |
| Disk full during Phase 1 | Partial records in page cache | Clean (truncated) | `StoreError::DiskFull`; agent decides retry policy |
| fsync fails (hardware error) | CHECKPOINT in page cache, sync failed | Treated as uncommitted; truncated on recovery | `StoreError::CommitFailed`; agent retries |
| Writer process killed (SIGKILL) mid-batch | In-memory pipeline state lost | Same as crash-during-Phase-1 | Same as crash-during-Phase-1 |
| File-system corruption below the OS layer | Unknowable | Unknowable | Out of scope — filesystem's responsibility |

### 7.1 Idempotent retry

All write operations are idempotent under retry:

- If the agent retries after an uncertain commit (crash between CHECKPOINT write and fsync), the retry produces a new batch with a fresh `episode_id`. If the original batch did commit, the retry may create a "duplicate" — but duplicates are dedup'd at the Semantic stage's DedupProposer (per `librarian-pipeline.md` § 7.1) because they produce byte-identical canonical records.
- Agents should attach a client-provided correlation ID (post-MVP — part of the wire protocol design) so dedup can recognize retries before pipeline work. v1 relies on byte-equality dedup.

## 8. Supersession edge application at CHECKPOINT

Supersession is an edge operation, not a record modification. When a batch includes a `SUPERSEDES` edge:

- The edge record is appended to `canonical.log` like any other record.
- The referenced prior memory's on-disk bytes are **not modified**. Append-only invariant.
- At CHECKPOINT fsync, the edge is committed.
- Post-commit, the in-memory DAG index is updated to reflect the new edge. Reads consulting the current-state index see the prior memory as invalidated.

### 8.1 Batch-level supersession serialization

Batches within a workspace are strictly serial (single-writer per PRINCIPLES.md architectural boundary #1). Each batch's Semantic stage sees the post-commit state of all prior batches, so "concurrent supersession of the same memory" cannot occur. Two back-to-back batches that both target the same current-state memory serialize naturally: the first commits, the second's Semantic stage either extends the supersession chain from the first's result or, if the intent has become incoherent, rejects with a typed error before any record is written.

### 8.2 Retroactive supersession

When a batch supersedes a memory retroactively (new memory's `valid_at` < prior memory's `valid_at` — per `temporal-model.md` § 5.1), the edge is recorded at the new memory's `valid_at`, not at commit time. The DAG reflects the retroactive ordering; queries with `as_of` predates respect the retroactive edge direction.

## 9. Symbol-table mutation durability

### 9.1 Symbol events are regular records

`SYMBOL_ALLOC` / `SYMBOL_RENAME` / `SYMBOL_ALIAS` / `SYMBOL_RETIRE` / `SYMBOL_UNRETIRE` (opcodes 0x30–0x34 per `ir-canonical-form.md` § 6.6) are records in `canonical.log`, appended as part of the batch that caused them. Their durability is tied to the CHECKPOINT that closes the batch.

### 9.2 Derived caches are advisory

`symbols.snapshot` and `symbols.wal` are derived caches. They are rewritten periodically (snapshot) or appended incrementally (WAL) but are **not authoritative** — on any startup they are regenerated from `canonical.log` if they are missing, corrupt, or stale.

### 9.3 Snapshot cadence

Snapshot frequency is a librarian configuration parameter (default: every 10,000 committed batches, or daily, whichever comes first). Snapshots are background operations that do not block writes.

## 10. Crash-recovery algorithm

On librarian startup for a workspace:

```
recover(workspace):
    header = load(workspace/header.bin)
    if header.magic != b"MIMR" or header.format_version != 0x01:
        return StartupError::IncompatibleFormat

    log = open(workspace/canonical.log, read_only=false)
    log_len = file_size(log)

    # Decode forward, tracking the byte offset immediately after the
    # last CHECKPOINT that decoded cleanly.
    last_checkpoint_offset = 0
    cursor = 0
    while cursor < log_len:
        decoded = decode_record(log[cursor:])
        if decoded.ok:
            cursor += decoded.bytes_consumed
            if decoded.record.opcode == CHECKPOINT:
                last_checkpoint_offset = cursor
            continue

        if decoded.error in [Truncated, LengthMismatch]:
            # Torn final frame: recoverable interrupted append.
            break

        # Unknown opcode, reserved sentinel, invalid discriminant,
        # invalid flag bits, body underflow, etc. are corruption, not
        # a crash-shaped orphan tail. Preserve bytes for inspection or
        # remote restore rather than truncating.
        return StartupError::CorruptTail(offset=cursor, source=decoded.error)

    # Truncate valid uncommitted records or a recoverable torn final
    # frame after the last CHECKPOINT.
    if last_checkpoint_offset < log_len:
        truncate(log, last_checkpoint_offset)
        fsync(log)

    # Rebuild in-memory state from the log
    symbol_table = try_load_snapshot(workspace/symbols.snapshot)
    dag = try_load_snapshot(workspace/dag.snapshot)

    if symbol_table is None or dag is None or snapshot_committed_at < last_checkpoint_at:
        # Full replay
        symbol_table, dag = replay_from_start(log)
    else:
        # Incremental replay from snapshot point
        replay_delta(log, from=snapshot_committed_at, state=(symbol_table, dag))

    current_state_index = build_current_state_index(log, dag)

    return LibrarianState { symbol_table, dag, current_state_index, log }
```

### 10.1 Checkpoint scan and tail classification

The current implementation scans forward because v1 framing has no reverse index. It tracks the final clean `CHECKPOINT` boundary and classifies the first decoder stop:

- `Truncated` / `LengthMismatch`: recoverable torn tail; truncate to the last clean `CHECKPOINT`.
- Cleanly decoded records after the last `CHECKPOINT`: valid but uncommitted orphan records; truncate to the last clean `CHECKPOINT`.
- Any other `DecodeError`: non-recoverable corruption; fail open without truncating so `mimir verify` / backup restore can inspect the bytes.

A future reverse index can make the healthy path O(1), but it must preserve this same recovery classification.

### 10.2 Orphan truncation is idempotent

Truncating the same orphan-free log to the same CHECKPOINT-terminated offset is a no-op. Recovery can run multiple times without side effects (important for supervised-restart deployments).

### 10.3 Snapshot-based fast recovery

If `symbols.snapshot` and `dag.snapshot` are both present and both match the last CHECKPOINT's commit time (or an earlier committed one), recovery loads them and replays only the delta. This is the fast path for large workspaces.

If either snapshot is missing or stale, full replay from log start runs. This is the slow path but guaranteed correct.

### 10.4 Target recovery time

Per `PRINCIPLES.md` § 6, cold-start time target is < 2 seconds for a 1M-fact store. On the fast path (snapshot + small delta) this is achievable; on full-replay it may take longer for large logs. Snapshot cadence (§ 9.3) bounds the replay delta.

## 11. Invariants

1. **CHECKPOINT is atomic commit.** A batch is committed iff its `CHECKPOINT` record is durable. Cleanly decoded records after the last durable CHECKPOINT are valid but uncommitted orphans.
2. **Recoverable orphans truncate on recovery.** Recovery restores the log to end exactly after the last durable CHECKPOINT for valid orphan records and torn final frames (`Truncated` / `LengthMismatch`). Non-recoverable decode errors fail open without truncation.
3. **No partial commits.** A batch's records are all in the log or none are. Never some.
4. **Append-only.** No record in `canonical.log` is ever modified after write. Truncation at orphan boundaries is allowed; in-place mutation is not.
5. **Single writer per workspace.** One batch at a time, one writer process (or thread) holding the workspace's writer lock.
6. **Derived caches are regeneratable.** Loss of any cache does not affect correctness; only startup time.
7. **CHECKPOINT carries Episode metadata.** `episode_id`, `memory_count`, and commit time are in the CHECKPOINT record, making the Episode self-describing.
8. **fsync before ack.** The librarian does not return `Ok(episode_id)` to the agent until the CHECKPOINT fsync succeeds.

## 12. Open questions and non-goals for v1

### 12.1 Open questions

**Group commit.** Single-writer semantics mean there are no concurrent writers to batch together at commit time. Group commit is moot under the single-writer invariant.

**Durability levels.** An advanced mode where writes use `O_DIRECT` or platform-specific sync primitives for stricter durability (bypassing OS cache) may be worth exploring for high-assurance deployments. Post-MVP.

**Log rotation / compaction.** `canonical.log` grows monotonically. Compaction (merging old segments while preserving supersession history) is the LSM-tree pattern. v1 does not compact; the log is a single file. Post-MVP LSM-style compaction is a candidate spec.

**Cross-process write-lock detection.** The workspace's single-writer lock needs to be detectable across processes (two librarian instances accidentally targeting the same workspace). v1: advisory file lock via `flock`. Non-cooperating processes can bypass; v1 trusts the deployment to run one librarian per workspace.

**Torn writes.** POSIX does not guarantee write atomicity at the record level — a crash can leave a partially-written record. Record-level CRCs (see `ir-canonical-form.md` § 11.1 open question) would detect this; v1 accepts the small corruption surface and handles it at recovery via length-prefix consistency checks.

### 12.2 Non-goals for v1

- **Multi-writer / concurrent-writer arbitration.** Single-writer per workspace is the thesis. No SSI, no rw-antidependency tracking, no serialization aborts.
- **Two-phase commit across processes.** Single-writer, single-process per workspace.
- **Replication / hot-standby.** Workspaces are machine-local per `workspace-model.md` § 2.
- **Undo / Compensation Log Records.** Append-only forbids — not needed.
- **Point-in-time recovery to a specific CHECKPOINT.** Recovery always recovers to the last durable CHECKPOINT. Intentional earlier-checkpoint recovery (e.g., rolling back an accidental batch) is post-MVP.
- **fsync batching across workspaces.** Each workspace has its own `canonical.log` and its own fsync cadence.

## 13. Primary-source attribution

All entries are verified per `docs/attribution.md`.

- **Mohan et al. 1992, *ARIES: A Transaction Recovery Method Supporting Fine-Granularity Locking and Partial Rollbacks Using Write-Ahead Logging*** (verified, already cited) — canonical reference for log-replay recovery. Mimir uses the Redo-only subset; Undo and CLRs are deliberately omitted because append-only semantics make them unnecessary.
- **O'Neil et al. 1996, *The Log-Structured Merge-Tree (LSM-Tree)*** (verified, already cited) — canonical reference for append-log + snapshot + WAL-reconciliation design. Directly informs § 3 and § 10.
- **POSIX / Linux `fsync` semantics references** (verified, OS docs) — for the durability-boundary claims in § 6. Cited but not load-bearing; the spec's behavior is expressed in terms of "durable after fsync returns success," which matches the POSIX contract.
- **SQLite WAL documentation** ([sqlite.org/wal.html](https://www.sqlite.org/wal.html), pending) — canonical example of a production-grade WAL design with log-replay recovery. Referenced for § 3 and § 10.
