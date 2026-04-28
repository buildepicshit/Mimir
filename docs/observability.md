# Observability

Status: **authoritative** — backed by `mimir_core` and `mimir-librarian` tracing spans/events and tests.

This document is the contract between Mimir's library instrumentation and any operator / decoder tooling that consumes its logs. Event / span names and field names are stable under `PRINCIPLES.md` § 11 deprecation policy — renaming one is a breaking change.

## 1. Principles

- **`tracing` is the single interface.** No `println!` / `eprintln!` in library code. Binaries initialize a `tracing_subscriber` at startup; embedders install their own.
- **Identifiers only, never values.** Spans and events carry `SymbolId`, `ClockTime`, counts, byte offsets. They MUST NOT carry `Value` payloads, trigger / action / precondition / `o` fields, or Episode labels. Values are inspected via the decoder CLI against the canonical log, not scraped from logs — that path is auditable and the logs stay privacy-safe. A reviewer seeing a log field that carries a user-visible string is a bug.
- **Unconditional.** No feature flags. Disabled-subscriber overhead is near-zero, and conditional instrumentation would require `#[cfg]` on every call site.

## 2. Dependency layout

- `mimir_core` runtime: `tracing` (emission only).
- `mimir_core` dev-deps: `tracing-subscriber` (test capture).
- `mimir-librarian` runtime: `tracing` and `tracing-subscriber` (binary subscriber + library emission).
- `mimir-cli` runtime: `tracing-subscriber` (user-visible log output).

`mimir-cli` installs a stderr subscriber at `main` entry with an `info`-level default that `RUST_LOG` overrides (e.g., `RUST_LOG=mimir=debug`). Library callers pick their own subscriber — `mimir_core` never installs one.

## 3. Spans

### `mimir.pipeline.compile_batch`

Emitted once per `Pipeline::compile_batch` call.

| Field | Type | Recorded | Meaning |
|---|---|---|---|
| `input_len` | `usize` | on entry | Bytes of raw agent input. |
| `record_count` | `usize` | on success | Total canonical records emitted. |
| `memory_count` | `usize` | on success | Count of `Sem` / `Epi` / `Pro` / `Inf` records. |
| `edge_count` | `usize` | on success | Count of `Supersedes` / `Corrects` / `StaleParent` / `Reconfirms` records. |

On error: the span still records timing + `input_len`; count fields stay unset. The typed `PipelineError` is returned to the caller, who decides whether to log it.

### `mimir.commit.batch`

Emitted once per `Store::commit_batch{,_with_metadata}` call; wraps `mimir.pipeline.compile_batch`.

| Field | Type | Recorded | Meaning |
|---|---|---|---|
| `log_offset_before` | `u64` | on entry | Log byte length before the batch. |
| `log_offset_after` | `u64` | on success | Log byte length after the CHECKPOINT. |
| `record_count` | `usize` | on success | Memory-record count in this batch. |
| `episode_id` | `Display(SymbolId)` | on success | Allocated Episode symbol (`__ep_{n}`). |
| `fsync_micros` | `u64` | on success | Wall-clock microseconds spent in Phase 2 fsync. |

### `mimir.librarian.run`

Emitted once per `mimir_librarian::run_once` call.

| Field | Type | Recorded | Meaning |
|---|---|---|---|
| `recovered_processing` | `u64` | on recovery and final update | Stale `processing/` drafts recovered to `pending/`. |
| `pending_seen` | `u64` | after pending scan | Pending drafts seen after recovery. |
| `claimed` | `u64` | as claims succeed and final update | Pending drafts claimed into `processing/`. |
| `accepted` | `u64` | as drafts finish and final update | Drafts accepted by the processor. |
| `skipped` | `u64` | as drafts finish and final update | Drafts intentionally skipped. |
| `failed` | `u64` | as drafts finish and final update | Drafts moved to failed. |
| `quarantined` | `u64` | as drafts finish and final update | Drafts moved to quarantined. |
| `deferred` | `u64` | as drafts finish and final update | Drafts returned to pending without terminal handling. |
| `claim_misses` | `u64` | as claim misses happen and final update | Pending drafts that disappeared before claim, usually from a concurrent runner. |

### `mimir.librarian.process`

Emitted once per `RetryingDraftProcessor::process` call.

| Field | Type | Recorded | Meaning |
|---|---|---|---|
| `draft_id` | `Display(DraftId)` | on entry | Scope/provenance-aware content id. |
| `max_attempts` | `u64` | on entry | Initial attempt plus configured retry budget. |
| `attempts` | `u64` | before each LLM call and final update | LLM attempts made for this draft. |
| `retries` | `u64` | when retries are scheduled and final update | Retry prompts scheduled after structured failures. |
| `response_records` | `u64` | after each parsed response and final update | Candidate records returned across parsed LLM responses. |
| `validated_records` | `u64` | after each validation pass and final update | Candidate records that passed pre-emit validation across attempts. |
| `duplicate_records` | `u64` | after duplicate checks and final update | Valid records skipped as already present in the canonical store. |
| `committed_records` | `u64` | after successful durable commit and final update | Unique records committed to the canonical store. |
| `decision` | lifecycle string | on terminal decision | Final processor decision: accepted / skipped / failed / quarantined / deferred. |
| `last_error_stage` | string | on retryable or terminal error | `llm`, `response`, `validation`, `dedup`, or `commit`. |
| `last_error_classification` | string | on retryable or terminal error | `invoke`, `json`, `parse`, `bind`, `semantic`, `emit`, `clock`, `supersession_conflict`, or `validation`. |

## 4. Events

Event names are stable contracts. Renaming requires deprecation.

**Events reflect attempted state, not guaranteed-committed state.** Supersession / cycle-rejection / recovery events fire at the emit-stage or recovery step where the decision is made. The enclosing `mimir.commit.batch` span may still fail at log-append or fsync and roll back; in that case its `log_offset_after` field is never recorded. Consumers MUST correlate events to the outcome of the enclosing span rather than treating an event in isolation as proof the change is durable.

### `mimir.supersession` — level `INFO`

Auto-supersession decision on the emit path. Emitted *once per decision*, before the matching `Supersedes` edge is appended to the record stream.

Semantic variant fields:

| Field | Type | Meaning |
|---|---|---|
| `kind` | `"semantic"` | |
| `direction` | `"forward"` or `"retroactive"` | § 5.1 temporal-model.md |
| `s` | `Display(SymbolId)` | Subject |
| `p` | `Display(SymbolId)` | Predicate |
| `old_memory_id` | `Display(SymbolId)` | Superseded memory |
| `new_memory_id` | `Display(SymbolId)` | Superseding memory |

Procedural variant fields:

| Field | Type | Meaning |
|---|---|---|
| `kind` | `"procedural"` | |
| `rule_id` | `Display(SymbolId)` | Rule identifier |
| `new_memory_id` | `Display(SymbolId)` | Superseding memory |
| `superseded_count` | `usize` | 1 or 2 (`rule_id` match + optional `(trigger, scope)` match) |

Not emitted when no prior memory matched — only actual supersessions raise this event.

### `mimir.dag.cycle_rejected` — level `WARN`

A proposed supersession / correction / stale-parent / reconfirms edge was rejected because it would close a cycle (`temporal-model.md` § 6.2 #1).

| Field | Type | Meaning |
|---|---|---|
| `from` | `Display(SymbolId)` | Edge origin memory |
| `to` | `Display(SymbolId)` | Edge target memory |
| `edge_kind` | `Debug(EdgeKind)` | `Supersedes` / `Corrects` / `StaleParent` / `Reconfirms` |

### `mimir.recovery.orphan_truncated` — level `WARN`

`Store::from_backend` found recoverable bytes past the last committed CHECKPOINT and truncated them (`write-protocol.md` § 10): either cleanly decoded but uncommitted records, or a torn final frame (`Truncated` / `LengthMismatch`). A healthy shutdown produces no orphans, so every occurrence of this event indicates a prior crash / incomplete commit. Non-recoverable tail corruption fails open as `StoreError::CorruptTail` and does not emit this truncation event.

| Field | Type | Meaning |
|---|---|---|
| `log_len_before` | `u64` | Log byte length at open time |
| `committed_end` | `u64` | Offset of the last committed CHECKPOINT |
| `orphan_bytes` | `u64` | Bytes truncated (`log_len_before - committed_end`) |

### `mimir.recovery.symbol_replay` — level `INFO`

Summary of committed-log replay during `Store::from_backend`. Emitted only when the store actually replayed state; opening a fresh empty store stays silent.

| Field | Type | Meaning |
|---|---|---|
| `symbol_alloc_count` | `u64` | `SymbolAlloc` records replayed |
| `symbol_mutation_count` | `u64` | `SymbolAlias` + `SymbolRename` + `SymbolRetire` records replayed |
| `checkpoint_count` | `u64` | CHECKPOINTs replayed (= committed-batch count) |
| `next_memory_counter` | `u64` | Post-replay `__mem_{n}` counter |
| `next_episode_counter` | `u64` | Post-replay `__ep_{n}` counter |

### `mimir.librarian.draft_processed` — level `INFO`

One event per draft that the runner successfully moves out of `processing/`.

| Field | Type | Meaning |
|---|---|---|
| `draft_id` | `Display(DraftId)` | Scope/provenance-aware content id. |
| `decision` | `"accepted"` / `"skipped"` / `"failed"` / `"quarantined"` / `"deferred"` | Processor lifecycle decision. |
| `final_state` | `"accepted"` / `"skipped"` / `"failed"` / `"quarantined"` / `"pending"` | Filesystem lifecycle state after movement. |

This event intentionally omits raw draft text, source file paths, LLM response text, and canonical Lisp payloads.

### `mimir.librarian.retry.scheduled` — level `INFO`

One event per retry prompt scheduled by the processor.

| Field | Type | Meaning |
|---|---|---|
| `draft_id` | `Display(DraftId)` | Scope/provenance-aware content id. |
| `attempt` | `u32` | Failed attempt number. |
| `next_attempt` | `u32` | Next attempt number to invoke. |
| `stage` | string | `response`, `validation`, `dedup`, or `commit`. |
| `classification` | string | Structured retry class; same vocabulary as `last_error_classification`. |

### `mimir.librarian.retry.exhausted` — level `WARN`

One event when a draft consumes its retry budget and fails.

| Field | Type | Meaning |
|---|---|---|
| `draft_id` | `Display(DraftId)` | Scope/provenance-aware content id. |
| `attempts` | `u32` | Attempts consumed. |
| `stage` | string | Stage that produced the terminal retry hint. |
| `classification` | string | Structured retry class; same vocabulary as `last_error_classification`. |

### `mimir.librarian.duplicate.skipped` — level `INFO`

One event when candidate records are intentionally skipped because matching canonical records already exist.

| Field | Type | Meaning |
|---|---|---|
| `draft_id` | `Display(DraftId)` | Scope/provenance-aware content id. |
| `duplicate_count` | `u64` | Candidate records skipped as duplicates. |

### `mimir.librarian.archive_raw.accepted` — level `INFO`

One event when the deterministic raw-archive processor commits a draft as governed pending-verification evidence.

| Field | Type | Meaning |
|---|---|---|
| `draft_id` | `Display(DraftId)` | Scope/provenance-aware content id. |
| `committed_records` | `u64` | Raw-evidence and provenance records committed for this draft. |
| `duplicate_count` | `u64` | Candidate archive records skipped as duplicates before commit. |

### `mimir.librarian.archive_raw.duplicate` — level `INFO`

One event when the deterministic raw-archive processor finds that every generated archive record already exists.

| Field | Type | Meaning |
|---|---|---|
| `draft_id` | `Display(DraftId)` | Scope/provenance-aware content id. |
| `duplicate_count` | `u64` | Candidate archive records skipped as duplicates. |

### `mimir.librarian.supersession_conflict` — level `WARN`

One event when a deterministic equal-key supersession conflict branches out of retry repair.

| Field | Type | Meaning |
|---|---|---|
| `draft_id` | `Display(DraftId)` | Scope/provenance-aware content id. |
| `attempt` | `u32` | Attempt that produced the conflict. |
| `policy` | `"skip"` / `"review"` | Conflict policy applied, without logging review directory paths. |

Processor events intentionally omit raw draft text, source file paths, raw LLM response text, retry prompt text, validation error strings, and canonical Lisp payloads.

## 5. Levels

| Level | Use |
|---|---|
| `INFO` | Commit boundaries; auto-supersession; recovery summaries; draft/run/process summaries; scheduled retries |
| `WARN` | DAG cycle rejection; orphan truncation; retry exhaustion; supersession-conflict quarantine/skip decisions |
| `DEBUG` | (reserved — not currently emitted) |
| `TRACE` | (reserved — not currently emitted) |

## 6. Deferred surface

The following are intentionally **not** instrumented in v1. They are surfaced here so future work starts from the known gap list.

- **Decay evaluation spans.** `decay::effective_confidence` is called from the read hot path, sometimes thousands of times per query. Instrumenting it unconditionally would be heavy; sampling policy is its own design surface. Deferred.
- **OTLP / `tracing-opentelemetry` coupling.** v1 emits only to whatever subscriber the operator installs. OTLP export is a bin-layer concern and is deferred until there's a deployment that needs it.
- **`ReadRequestId` correlation on read-path operations** (`PRINCIPLES.md` § 5). Issue #30's done-when covers write-entry points only; read-path tracing is a follow-up.
- **Per-form emit spans** and **stage-transition TRACE events**. The `mimir.pipeline.compile_batch` span covers the whole batch; per-form spans would blow up cardinality without clear consumer value.
- **Schema evolution tooling.** Once a stable set of consumers exists, a JSON-schema validator for event payloads will live under the decoder CLI.

## 7. Testing

Instrumentation is covered by tests that install a `tracing_subscriber::Layer`, capture structured fields, and assert on those fields — never on formatted strings. See `crates/mimir_core/tests/observability.rs` and `crates/mimir-librarian/src/runner.rs`.
