# Canonical IR Form

> **Status: authoritative 2026-04-18; criterion #4 closed 2026-04-19.** Graduated from `citation-verified` on 2026-04-18 backed by the `mimir_core::canonical` encoder/decoder implementation (19 opcodes with the addition of `EpisodeMeta = 0x21` on 2026-04-19; LEB128 varint + ZigZag + fixed-LE primitives, four-clock framing, symbol-event and flag-event records, episode-metadata record). Round-trip invariants are enforced by property tests in `mimir_core/tests/properties.rs`. Criterion #4 (no-panic-on-malformed-input decoder behaviour) met by the `fuzz/` cargo-fuzz target `fuzz_decoder` plus totality properties `decode_record_is_total_over_random_bytes` / `decode_all_is_total_over_random_bytes` / `decoder_rejects_every_prefix_truncation` in `tests/properties.rs` that run on every push. See [#33](https://github.com/buildepicshit/Mimir/issues/33) for cargo-fuzz setup.

The canonical form is Mimir's on-disk bytecode — the agent-native storage format mandated by PRINCIPLES.md architectural boundary #2. Agent writes parsed from the Lisp S-expression surface (per `ir-write-surface.md`) are compiled by the binder/emitter into records in this form, persisted to append-only logs, and read back by the librarian. The canonical form is not human-readable by design; humans inspect through the decoder tool (spec 3.14).

## 1. Scope

This specification defines:

- The opcode table (one byte per record kind).
- The per-record byte-level layout for memory writes, supersession edges, episode markers, and symbol-table events.
- Value-tag encoding for per-slot typed values.
- The on-disk file structure per workspace.
- Format versioning and forward-compatibility rules.

This specification does **not** define:

- The librarian pipeline (lex → parse → bind → semantic → emit) — `librarian-pipeline.md`.
- The IR write surface Lisp grammar — `ir-write-surface.md`.
- Memory type shapes — `memory-type-taxonomy.md`.
- Symbol identity semantics — `symbol-identity-semantics.md`.
- Temporal model — `temporal-model.md`.
- Write protocol (WAL durability, checkpoint atomicity) — `write-protocol.md`.
- Read protocol — `read-protocol.md`.
- Decoder tool contract — `decoder-tool-contract.md`.

### Graduation criteria

Graduates draft → authoritative when:

1. Related art is verified in `docs/attribution.md` (LSM-tree design, SQLite file format reference, Protocol Buffers wire format).
2. A Rust encoder + decoder implementing this spec compiles in `mimir_core`, with the invariants in § 10 covered by unit, property, and round-trip tests.
3. Round-trip tests cover every opcode at every value-tag combination — encode → decode → equality.
4. Fuzz testing (`cargo-fuzz`) runs against the decoder with no panics and deterministic error behavior on corrupted input.

## 2. Design thesis: binary canonical, agent-native

The canonical form exists to be fast, dense, deterministic, and stable across librarian restarts. It is **not** meant to be read with a text editor. That constraint is liberating:

- **Density.** Symbols compress to varint IDs; fixed-width where cache-friendly. A typical Semantic memory fits in ~40 bytes on disk.
- **Determinism.** No floating-point ambiguity in confidence (fixed-point u16). No endianness surprise (LE everywhere). No string-parse heuristics at read time.
- **Parse speed.** Opcode-dispatch on the first byte; length prefix lets the reader skip unknown records; no lookahead.
- **Append-only stability.** The format is additive — opcodes 0x00 and 0xFF are reserved for sentinel / extension continuation so future revisions can append new record types without breaking readers.

Humans who need to inspect the canonical log use the decoder tool (`mimir-cli inspect`), which reads the binary and emits textual forms. This is the mandated path per PRINCIPLES.md architectural boundary #2.

## 3. Conventions

- **Endianness:** little-endian throughout.
- **Integer encoding:**
  - Varint (LEB128, unsigned) for `SymbolId`, record lengths, symbol-table indices.
  - Fixed `u64` LE for `ClockTime` (millis since Unix epoch UTC).
  - Fixed `u16` LE for `Confidence` (see § 3.1).
- **Null ClockTime:** `u64::MAX` (`0xFFFF_FFFF_FFFF_FFFF`) is the sentinel for `invalid_at = None`. Year ~584M-from-epoch is unambiguous.
- **Strings:** varint length prefix + UTF-8 bytes. No null terminator.

### 3.1 Confidence encoding

```
stored_u16 = round(confidence * 65535.0)
confidence = stored_u16 / 65535.0
```

Resolution: ~1.53e-5 per step. Deterministic across architectures (no IEEE 754 rounding divergence). Bound enforcement (per `grounding-model.md`) applies to the stored value.

### 3.2 Value tags

Each typed value slot (e.g., `Semantic.o`, `Procedural.trigger`) carries a one-byte tag followed by a body.

| Tag | Kind | Body |
|---|---|---|
| `0x01` | Symbol | varint SymbolId |
| `0x02` | Integer | varint ZigZag-encoded i64 |
| `0x03` | Float | 8 bytes IEEE 754 binary64 LE |
| `0x04` | Boolean | 1 byte (`0x00` = false, `0x01` = true) |
| `0x05` | String | varint length + UTF-8 bytes |
| `0x06` | Timestamp | fixed u64 LE |

Tags 0x00 and 0xFF are reserved for sentinel / extension continuation.

### 3.3 Framing

Every record is:

```
[1 byte opcode][varint length][body]
```

`length` is the byte count of `body` only (not including the opcode or length bytes). Streaming readers parse opcode, read varint length, slice the body, and dispatch on opcode. Unknown opcodes are skipped (length tells the reader how many bytes to advance).

This framing enables:

- Forward-compatible log readers (unknown records skipped cleanly).
- Efficient seek-scan for specific opcodes (no full-record parse needed to skip).
- Safe corruption detection — a record whose length exceeds remaining-file-size is flagged per § 11.2.

## 4. Opcode table

| Opcode | Name | Purpose |
|---|---|---|
| `0x01` | `SEM` | Semantic memory |
| `0x02` | `EPI` | Episodic memory |
| `0x03` | `PRO` | Procedural memory |
| `0x04` | `INF` | Inferential memory |
| `0x10` | `SUPERSEDES` | supersession edge (sets prior memory's `invalid_at`) |
| `0x11` | `CORRECTS` | Episodic correction edge (no `invalid_at` set) |
| `0x12` | `STALE_PARENT` | Inferential stale-parent edge |
| `0x13` | `RECONFIRMS` | Inferential reconfirmation edge |
| `0x20` | `CHECKPOINT` | Episode boundary marker (atomic commit unit) |
| `0x30` | `SYMBOL_ALLOC` | new symbol allocation |
| `0x31` | `SYMBOL_RENAME` | rename edge |
| `0x32` | `SYMBOL_ALIAS` | alias edge |
| `0x33` | `SYMBOL_RETIRE` | retirement flag |
| `0x34` | `SYMBOL_UNRETIRE` | unretirement |
| `0x35` | `PIN` | agent-invokable pin (suspends decay per `confidence-decay.md` § 7) |
| `0x36` | `UNPIN` | clear pin flag |
| `0x37` | `AUTHORITATIVE_SET` | operator-authoritative flag set (per `confidence-decay.md` § 8) |
| `0x38` | `AUTHORITATIVE_CLEAR` | operator-authoritative flag cleared |
| `0xFF` | extension | continuation marker for future single-byte-overflow opcodes |

Reserved: `0x00` (sentinel / padding). All other unlisted opcodes are invalid; encountering one is a `DecodeError::UnknownOpcode`.

## 5. Per-memory-type record layout

Each record below assumes the framing of § 3.3: opcode byte, then varint length, then the body shown. Field order in the body is fixed.

### 5.1 `SEM` (0x01)

```
varint memory_id         (SymbolId — Memory-kind)
varint s_symbol          (Symbol)
varint p_symbol          (Symbol, Predicate-kind)
[1-byte tag + body] o    (Value; see § 3.2)
varint source_symbol     (Symbol)
u16 confidence           (fixed-point; § 3.1)
u64 valid_at             (ClockTime)
u64 observed_at          (ClockTime)
u64 committed_at         (ClockTime)
u64 invalid_at           (ClockTime; u64::MAX = None)
1 byte flags             (bit 0: projected; bits 1–7: reserved)
```

### 5.2 `EPI` (0x02)

```
varint memory_id
varint event_id_symbol     (Symbol, Memory-kind)
varint kind_symbol         (Symbol, EventType-kind)
varint participants_count
  repeat participants_count:
    varint participant_symbol
varint location_symbol     (Symbol)
u64 at_time
u64 observed_at
varint source_symbol
u16 confidence
u64 committed_at
u64 invalid_at             (always u64::MAX for Episodic; reserved for future use)
```

Episodic records carry no `flags` byte: `projected` and `stale` are not
meaningful for Episodes (observations are not projections, and
staleness applies only to Inferentials). Any future Epi-only flag gets
its own explicit byte at that point — see § 5.5.

### 5.3 `PRO` (0x03)

```
varint memory_id
varint rule_id_symbol
[1-byte tag + body] trigger   (Value)
[1-byte tag + body] action    (Value)
1 byte has_precondition       (0x00 = None, 0x01 = Some)
if has_precondition:
  [1-byte tag + body] precondition (Value)
varint scope_symbol
varint source_symbol
u16 confidence
u64 valid_at
u64 observed_at
u64 committed_at
u64 invalid_at
```

Procedural records carry no `flags` byte. `projected` on a rule has no
defined semantics (rules are not hypotheticals), and `stale` applies
only to Inferentials. See § 5.5.

### 5.4 `INF` (0x04)

```
varint memory_id
varint s_symbol
varint p_symbol
[1-byte tag + body] o         (Value)
varint derived_from_count
  repeat derived_from_count:
    varint parent_memory_symbol (Symbol, Memory-kind)
varint method_symbol          (Symbol, InferenceMethod-kind)
u16 confidence
u64 valid_at
u64 observed_at
u64 committed_at
u64 invalid_at
1 byte flags                  (bit 0: projected; bit 1: stale; bits 2–7: reserved)
```

### 5.5 Per-kind flags are kind-specific

Flag layouts are defined per memory kind; there is no shared
`MemoryFlags` byte across kinds. Kinds that have no applicable flag
carry no flags byte at all:

| Kind | Flags byte | Bits |
|------|------------|------|
| `SEM` | yes | `bit 0 = projected` |
| `EPI` | no  | — |
| `PRO` | no  | — |
| `INF` | yes | `bit 0 = projected`, `bit 1 = stale` |

Rationale: a single shared flag byte conflates flags that only apply to
some kinds (e.g., `stale` is meaningful only for Inferentials per
`temporal-model.md` § 5.4), pushing invariant enforcement out of the
type system and into readers. Per-kind flags let the type system reject
invalid combinations at construction time and keep the wire format
exactly as wide as each kind needs.

A kind that later grows a flag adds its own byte explicitly (with an
opcode bump if the change is incompatible). This is a schema break from
the earlier shared-byte layout — any on-disk logs written before this
split must be re-encoded before replay.

## 6. Operation records

Operation records are separate from memory records. They populate the supersession DAG and the symbol table. Episodic events for operations (`@rename`, `@alias`, `@retire` — see `symbol-identity-semantics.md`) are emitted *as well*, written as normal `EPI` records; the operation records here are the librarian-internal state mutations, not the audit events.

### 6.1 `SUPERSEDES` (0x10)

```
varint from_memory_id    (the superseding memory)
varint to_memory_id      (the superseded memory)
u64 at                   (timestamp at which the supersession edge was applied)
```

Effect on canonical state: `to_memory`'s `invalid_at` is updated to `at`.

### 6.2 `CORRECTS` (0x11)

```
varint from_memory_id    (the correction)
varint to_memory_id      (the corrected Episodic)
u64 at
```

Does **not** update `invalid_at`. Both memories remain current; the edge is audit-only. Decoder tool displays linked pairs.

### 6.3 `STALE_PARENT` (0x12)

```
varint from_memory_id    (the Inferential that was marked stale)
varint to_memory_id      (the superseded parent)
u64 at
```

Effect: `from_memory`'s stale flag is set (bit 1 of the `INF.flags` byte).

### 6.4 `RECONFIRMS` (0x13)

```
varint from_memory_id    (the reconfirming Inferential)
varint to_memory_id      (the previously-stale Inferential)
u64 at
```

Effect: `to_memory`'s stale flag is cleared.

### 6.5 `CHECKPOINT` (0x20)

```
varint episode_id        (SymbolId — Memory-kind, newly allocated for the Episode)
u64 at                   (commit time of the Episode)
varint memory_count      (count of memory records included in this Episode)
```

Emitted at the end of an Episode's batch. The librarian uses CHECKPOINT markers to atomically commit / roll back Episodes per `write-protocol.md`. Records between two CHECKPOINTs are members of the same Episode.

### 6.6 Symbol-table events (0x30–0x34)

All symbol-table events take the form:

```
varint symbol_id
varint name_length
name_length UTF-8 bytes  (the canonical or alias name)
1 byte symbol_kind       (ordinal into the SymbolKind enum per `symbol-identity-semantics.md` § 4)
u64 at                   (timestamp)
```

Specific semantics:

- `SYMBOL_ALLOC` (0x30): allocation of a new symbol with given name + kind.
- `SYMBOL_RENAME` (0x31): declares this `name` as the new canonical for `symbol_id`; previous canonical becomes an alias.
- `SYMBOL_ALIAS` (0x32): adds `name` as an alias of `symbol_id`.
- `SYMBOL_RETIRE` (0x33): marks `symbol_id` retired; `name` field is ignored (kept for format uniformity).
- `SYMBOL_UNRETIRE` (0x34): clears retirement flag on `symbol_id`.

The librarian replays these events in order to reconstruct the symbol table on startup (§ 7.2).

### 6.7 Pin and authoritative events (0x35–0x38)

Records suspending decay on individual memories per `confidence-decay.md` §§ 7–8. All four take the same shape:

```
varint memory_id           (the target memory's SymbolId — Memory-kind)
u64 at                     (timestamp)
varint actor_symbol        (agent or user who set/cleared the flag; SymbolId — Agent-kind)
```

Specific semantics:

- `PIN` (0x35): sets the memory's `pinned` flag. Agent-invokable. While pinned, `effective = stored` (no time decay, no activity weighting). Emits a companion Episodic `@pin` memory per `symbol-identity-semantics.md`-style audit convention.
- `UNPIN` (0x36): clears the `pinned` flag. Decay resumes from the memory's original `valid_at`, not from unpin time (per `confidence-decay.md` § 7.2).
- `AUTHORITATIVE_SET` (0x37): sets the memory's `operator_authoritative` flag. User-applied only (via `mimir-cli`). Rejected at bind if the request comes through the agent write surface. Gives the memory `Framing::Authoritative { set_by: OperatorAuthoritative }` on reads that request framing.
- `AUTHORITATIVE_CLEAR` (0x38): clears the `operator_authoritative` flag.

These events replay alongside supersession edges and symbol-table mutations during recovery (per `write-protocol.md` § 10) to reconstruct per-memory flag state. The derived caches in `~/.mimir/data/<workspace>/` may maintain a compact per-memory flag bitset as an optimization; the canonical log is the source of truth.

## 7. On-disk file structure

Per `workspace-model.md` § 4.2, each workspace owns a directory:

```
~/.mimir/data/<workspace_id>/
    header.bin
    canonical.log
    symbols.snapshot
    symbols.wal
    dag.snapshot
    dag.wal
    episodes.log
```

### 7.1 `header.bin`

Fixed-size file, 16 bytes:

```
offset 0:  4 bytes magic       = b"MIMR"
offset 8:  1 byte format_version = 0x01
offset 9:  1 byte reserved       = 0x00
offset 10: 6 bytes reserved      = 0x00 …
```

A librarian that opens a workspace whose `header.bin` does not match the expected magic or whose `format_version` it does not support refuses to start and emits `StartupError::IncompatibleFormat`.

### 7.2 `canonical.log`

Append-only stream of records per § 3.3 framing. Source of truth. All other files in the workspace are derivable from this log by full replay.

Records appear in commit order. `CHECKPOINT` records delimit Episodes; memory / operation records belong to the most recent preceding checkpoint-less batch.

### 7.3 `symbols.snapshot`

Periodic snapshot of the workspace's symbol table at a known commit point. Format:

```
u64 next_symbol_id
varint entry_count
  repeat entry_count:
    varint symbol_id
    varint canonical_name_length
    N bytes UTF-8 canonical_name
    1 byte symbol_kind
    varint alias_count
      repeat alias_count:
        varint alias_length
        M bytes UTF-8 alias
    1 byte retired           (0x00 = no, 0x01 = yes)
    u64 created_at
    u64 retired_at           (u64::MAX if not retired)
u64 snapshot_committed_at    (matches a CHECKPOINT in canonical.log)
```

### 7.4 `symbols.wal`

WAL accumulating `SYMBOL_ALLOC` / `SYMBOL_RENAME` / `SYMBOL_ALIAS` / `SYMBOL_RETIRE` / `SYMBOL_UNRETIRE` events since the last snapshot. On startup: load snapshot, replay WAL, reach current state.

Records use the same framing and encoding as in `canonical.log` § 6.6.

### 7.5 `dag.snapshot`

Snapshot of the supersession DAG (edges from § 6.1–6.4) at a known commit point.

```
varint edge_count
  repeat edge_count:
    1 byte edge_kind         (matches opcode: 0x10/0x11/0x12/0x13)
    varint from_memory_id
    varint to_memory_id
    u64 at
u64 snapshot_committed_at    (matches a CHECKPOINT in canonical.log)
```

### 7.6 `dag.wal`

Edges added since the last DAG snapshot. Same framing as in `canonical.log` § 6.1–6.4.

### 7.7 `episodes.log`

Per-Episode metadata for fast queries by episode_id:

```
Episode records:
    varint episode_id
    u64 started_at
    u64 committed_at         (u64::MAX if rolled back)
    varint member_memory_count
    bytes to the start offset in canonical.log (u64)
    bytes to the end offset in canonical.log (u64)
    1 byte status            (0x01 = committed, 0x02 = rolled_back)
```

Rollback records explain how `write-protocol.md`'s rollback machinery references an uncommitted Episode.

## 8. Format versioning

### 8.1 Version byte in `header.bin`

`format_version = 0x01` for v1. Incremented on any breaking change to record layouts.

### 8.2 Forward compatibility: opcode 0xFF

Opcode `0xFF` is reserved as an **extension continuation marker**. Its body layout:

```
varint extension_id
varint ext_length
ext_length bytes body
```

Extension IDs are registered centrally. A librarian that encounters a `0xFF` record with an unknown `extension_id` may skip it (length prefix tells it how many bytes to advance) and continue reading. This lets future single-byte-overflow record types ship without bumping `format_version`.

### 8.3 Backward compatibility

A librarian reading a file with a *lower* `format_version` than its current is expected to handle it — each version-bump defines the upgrade path. v1 has no older version.

### 8.4 No silent upgrades

Write operations never implicitly upgrade a lower-version file. An `mimir-cli workspace migrate --target-version <N>` command explicitly rewrites the canonical log at the new version (post-MVP; not in v1).

## 9. Record size expectations

Approximate sizes for typical memory records (varint-encoded with small-ID assumption):

| Record | Typical size |
|---|---|
| SEM with small symbols + short string `o` | ~32–48 bytes |
| EPI with 2 participants | ~48–64 bytes |
| PRO with short trigger + action | ~48–64 bytes |
| INF with 3 parents | ~40–56 bytes |

These sit comfortably under the ~16 KB per-memory soft cap proposed in `workspace-model.md` § 8.1 (open question). A 1M-memory store is expected to be under 100 MB on disk — matches the ~500 MB resident memory target in `PRINCIPLES.md` § 6.

## 10. Invariants

1. **Framing consistency.** Every record is `[opcode][varint len][body]`, `body` is exactly `len` bytes. Violations are `DecodeError::TruncatedRecord` or `DecodeError::LengthMismatch`.
2. **Opcode validity.** Opcodes are in the registered set or `0xFF`; otherwise `DecodeError::UnknownOpcode`.
3. **Value tag validity.** Value tags are in `0x01–0x06`; otherwise `DecodeError::UnknownValueTag`.
4. **Confidence range.** Decoded confidence `u16` converts to a `f32` in `0.0..=1.0`.
5. **ClockTime sentinel.** `u64::MAX` in an `invalid_at` position is `None`; anywhere else it is a literal timestamp (~year 584M).
6. **String UTF-8.** String bodies are valid UTF-8; invalid UTF-8 is `DecodeError::InvalidString`.
7. **Endianness.** All multi-byte fixed-width fields are little-endian.
8. **Symbol references bind.** Every `SymbolId` referenced in a record resolves to an entry in the workspace's current symbol table at decode time. Unresolved: `DecodeError::DanglingSymbol`.
9. **Header integrity.** Workspace directories without a valid `header.bin` refuse to load.
10. **Append-only.** Records are never modified in place in `canonical.log`. Snapshots (`symbols.snapshot`, `dag.snapshot`) are rewritten atomically via tempfile + rename, never patched.

## 11. Open questions and non-goals for v1

### 11.1 Open questions

**Log rotation.** `canonical.log` grows monotonically. When does the librarian split into multiple log files? Candidate: at every snapshot, close the current log and start a new one. Defer to `librarian-pipeline.md` § compaction.

**Compression.** v1 uses no compression. LSM-style block compression (zstd / lz4) is a clear post-MVP optimization. Candidate trigger: canonical log size > 1 GB per workspace.

**Checksum per record.** Framing is size-delimited; a corrupt length byte can stampede reads. Should each record carry a CRC32 or xxhash checksum? Cost: +4 bytes/record. Benefit: detectable corruption. Defer until workload shows the need.

**Multi-workspace shared dictionary.** Cross-workspace import (`workspace-model.md` § 5.3) currently copies records verbatim, re-allocating foreign symbols in the receiving workspace's table. A shared-dictionary encoding for the importing workspace could avoid duplicate symbol allocation. Post-MVP federation question.

**Streaming writers.** v1 assumes the librarian is a single-writer process (PRINCIPLES.md architectural boundary #1); the canonical log is exclusive-write. Multi-process writer coordination via file locks or a coordinator is out of scope.

### 11.2 Non-goals for v1

- **Textual canonical form.** The canonical form is binary. A text-based canonical log is a non-goal.
- **Random-access mutation.** The canonical log is append-only; no in-place edits.
- **Forward-compatible arbitrary opcode semantics.** Opcode 0xFF handles extensions; unknown non-`0xFF` opcodes are errors, not silently-skipped.
- **Cross-platform binary compatibility.** v1 targets little-endian CPUs (all currently-supported Claude-deployment targets are LE). Big-endian compatibility is post-MVP.
- **Encryption at rest.** Plaintext on disk. Disk encryption is an OS-layer concern, not an Mimir concern.
- **Time-series compression.** No delta encoding across sequential memories. Defer.

## 12. Primary-source attribution

All entries are verified per `docs/attribution.md`. The canonical form's design is not load-bearing on literature — it follows established binary-format patterns — but prior art is worth citing.

- **LSM-Tree** (O'Neil et al. 1996, already pending) — append-only log + periodic snapshot + WAL-reconciliation pattern. Directly informs § 7 file structure.
- **SQLite file format reference** ([sqlite.org/fileformat2.html](https://www.sqlite.org/fileformat2.html), pending) — canonical example of a stable, versioned, self-describing binary file format. Mimir's `header.bin` + format-version byte follows this pattern.
- **Protocol Buffers wire format** ([protobuf.dev/programming-guides/encoding/](https://protobuf.dev/programming-guides/encoding/), pending) — varint encoding, field-tag conventions. Mimir's opcode + value-tag + varint approach parallels protobuf's tag + wire-type + value.
- **Google LEB128 / ZigZag encoding references** (verified) — standard varint encoding for unsigned (LEB128) and signed (ZigZag) integers.
