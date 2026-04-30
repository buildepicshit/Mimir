# Decoder Tool Contract

> **Status: authoritative 2026-04-18.** Graduated from `citation-verified` on 2026-04-18 backed by the new `mimir-cli` binary crate (depends on `mimir_core` only, no `mimird`). Shipped subcommands: `log`, `decode`, `symbols`, `verify`. `mimir_cli::LispRenderer` reconstructs write-surface Lisp for all four memory record shapes (Sem / Epi / Pro / Inf); `iso8601_from_millis` is the deterministic inverse of the parser's ISO-8601 loader. `mimir_cli::verify` reports records_decoded / checkpoints / memory_records / symbol_events / trailing_bytes / dangling_symbols. Seven integration tests cover § 10 invariants 1 (read-only), 2 (deterministic rendering), 3 (lossless round-trip for agent-visible fields via re-parse + re-pipeline), 4 (streaming integrity), 6 (rendered Lisp is valid per `ir-write-surface.md`). Post-MVP subcommands (`inspect`, `query`, `history`, `episode`, `workspaces`, `config`) are flagged in spec § 3.1 and not shipped in this milestone. v1 goal was to prove the round-trip contract and the corruption-detection contract — both done. **Amended 2026-04-19**: § 9 (live-vs-offline-mode) collapsed to a single filesystem access path, § 10 invariant 5 rewritten, cross-refs to the removed `mimird` daemon dropped — all consequent to `wire-architecture.md`'s graduation to in-process only.

This specification defines the contract of `mimir-cli`, the read-only inspection tool that humans and audit scripts use to observe Mimir's canonical state. Per PRINCIPLES.md architectural boundary #2, the canonical form is not human-readable — inspection always routes through this tool. The decoder is the sanctioned bridge between the agent-native binary format and human / external consumers.

## 1. Scope

This specification defines:

- The `mimir-cli` command surface (subcommands, flags).
- Output formats (Lisp round-trippable, annotated, JSON, text).
- The lossless round-trip guarantee for agent-provided fields.
- Integrity verification (`verify` subcommand).
- Query grammar parity with the agent read API.
- Streaming semantics for large outputs.
- Access mode (direct-log-read).

This specification does **not** define:

- Memory type shapes — `memory-type-taxonomy.md`.
- Canonical bytecode layout — `ir-canonical-form.md` (the decoder *reads* this format; this spec defines the CLI contract).
- The IR write-surface Lisp grammar — `ir-write-surface.md` (the decoder *emits* round-trippable output in this grammar).
- Agent API contract — `wire-architecture.md` (v1 is in-process only; the decoder never touches that API surface — it reads the canonical log directly).
- Read-path internals — `read-protocol.md` (the decoder uses the agent-facing query API).

### Graduation criteria

Graduates draft → authoritative when:

1. Related art is verified in `docs/attribution.md` (Unix tooling philosophy; database inspection utilities like `mysqldump`, `pg_dump`, `sqlite3`).
2. A Rust `mimir-cli` binary compiles as a distinct crate in the workspace, depending on `mimir_core` but not on `mimird`, with the invariants in § 10 covered by integration tests.
3. Round-trip tests: for every canonical record in a test corpus, `decode → re-enqueue → canonicalize` reproduces the agent-visible fields byte-identically.
4. `verify` catches all injected corruptions in the test suite. **v1 scope** (graduation): framing, opcode, symbol reference — each tested via direct byte injection. **Post-MVP**: DAG cycle and snapshot-vs-log divergence, which require the supersession DAG from `temporal-model.md` and the snapshot file format from `write-protocol.md` § 9 respectively; neither underlying feature is built in v1, so the corresponding corruption classes have nothing to inject against. Revisit these rows when the DAG and snapshot surfaces land.

## 2. Design thesis: read-only, high-fidelity, format-choice

The canonical form is binary and agent-optimized (per `ir-canonical-form.md`). Humans who need to audit, debug, or migrate data read via the decoder — never via a hex dump of `canonical.log`. The decoder's job is to:

- **Preserve fidelity.** Every agent-provided field is exposed exactly as stored. Librarian-assigned fields are shown separately so the reader can distinguish agent intent from librarian computation.
- **Support round-trip.** Reconstructing a re-enqueueable Lisp S-expression from a stored record lets operators replay, migrate, or rebuild workspaces.
- **Offer format choice.** Human-skimming, programmatic parsing, and round-trip emission all want different output shapes. One tool, multiple `--format` options.
- **Stay read-only.** The decoder never writes. Write operations on Mimir go through `Store::commit_batch` (`wire-architecture.md` § 3.1) and the librarian pipeline; the decoder is exclusively an inspection path.

Design philosophy parallels `sqlite3`, `pg_dump`, `mysqldump`: a small, focused tool that reads the store, prints structured output, and exits. Unix-style composition — `mimir-cli log | grep …`, `mimir-cli symbols --retired | wc -l` — is a first-class use case.

## 3. Command surface

`mimir-cli` ships as a separate Rust binary (`crates/mimir-cli` in the workspace). It depends on `mimir_core` for canonical-form decoding. There is no `mimird` in v1 (`wire-architecture.md`) and therefore no daemon dependency.

### 3.1 Subcommands

The four subcommands below shipped in milestone 5.10 and are the v1 graduation surface. The remainder are post-MVP, pending the underlying librarian features they depend on (query execution, effective-confidence computation in read context, DAG traversal).

| Status | Subcommand | Purpose |
|---|---|---|
| v1 | `mimir-cli log <path>` | Stream canonical records as one-line summaries |
| v1 | `mimir-cli decode <path>` | Emit round-trippable Lisp S-expressions reconstructing memory records (Sem / Epi / Pro / Inf) |
| v1 | `mimir-cli symbols <path>` | List symbols with canonical name, aliases, kind, retirement status |
| v1 | `mimir-cli verify <path>` | Integrity check on `canonical.log`; read-only, reports framing / opcode / symbol-reference corruption diagnostics |
| post-MVP | `mimir-cli inspect @memory_id` | Full detail of one memory: stored + effective confidence, framing, grounding, supersession status (requires read-protocol + confidence-decay effective computation integration) |
| post-MVP | `mimir-cli query "<lisp>"` | Execute a query in the same grammar as agents (requires read-protocol) |
| post-MVP | `mimir-cli history @memory_id` | Supersession chain, rename history, decay trajectory, Episode membership (requires supersession DAG from temporal-model) |
| post-MVP | `mimir-cli episode @episode_id` | All memories in an Episode, chronologically; parent / retracts links (requires episode-semantics) |
| post-MVP | `mimir-cli workspaces` | List workspaces under `~/.mimir/data/` |
| post-MVP | `mimir-cli config` | Show effective workspace configuration (`mimir.toml` + librarian defaults) |

### 3.2 Further post-MVP candidates

- `mimir-cli interactive` — REPL with query history, autocomplete against symbol table.
- `mimir-cli export` / `mimir-cli import` — workspace snapshot for backup / migration.

### 3.3 Global flags

- `--workspace @WS` — target workspace (default: detected from CWD per `workspace-model.md` § 3.3).
- `--format {lisp|annotated|json|text}` — output format (defaults per subcommand).
- `--quiet` / `--verbose` — verbosity.
- `--as-of T` — temporal lens for inspection (defaults to now); maps to the read API's `:as_of` predicate.
- `--as-committed T` — transaction-time lens; maps to `:as_committed`.

## 4. Output formats

### 4.1 `--format lisp` (round-trippable)

Emits pure Lisp S-expressions conforming to `ir-write-surface.md`. Only agent-visible fields included; librarian-assigned fields omitted. Reusable as enqueue input.

Example (`mimir-cli decode @ep_001`):

```
(sem @alain email "alice@example.com" :src @profile :c 0.95 :v 2024-01-15)
(sem @alain role @founder :src @profile :c 0.98 :v 2024-01-01)
```

### 4.2 `--format annotated` (default for most subcommands)

Lisp S-expression plus librarian-assigned fields as line comments. Human-readable but not directly re-enqueueable (comments would be stripped; what remains is the same as `--format lisp`).

Example (`mimir-cli inspect @sem_alain_email`):

```
(sem @alain email "alice@example.com"
     :src @profile :c 0.95 :v 2024-01-15)
  ;; memory_id:        @sem_alain_email (SymbolId(42))
  ;; committed_at:     2024-01-15T10:23:41.502Z
  ;; observed_at:      2024-01-15T10:23:41.502Z
  ;; invalid_at:       None (current)
  ;; effective_conf:   0.93 (decayed from 0.95; half-life @profile = 730 days)
  ;; framing:          Advisory
  ;; episode:          @ep_001 ("profile-import")
  ;; supersession:     none (current state)
  ;; grounding-kind:   Agent [@profile's kind]
```

### 4.3 `--format json`

Structured JSON emission for programmatic consumers. All fields (agent-provided + librarian-assigned + computed) at top level; no annotation comments.

```json
{
  "memory_id": "@sem_alain_email",
  "kind": "semantic",
  "s": "@alain", "p": "email", "o": "alice@example.com",
  "source": "@profile",
  "stored_confidence": 0.95,
  "effective_confidence": 0.93,
  "valid_at": "2024-01-15T00:00:00Z",
  "committed_at": "2024-01-15T10:23:41.502Z",
  "observed_at": "2024-01-15T10:23:41.502Z",
  "invalid_at": null,
  "framing": "Advisory",
  "episode_id": "@ep_001",
  "supersession": { "chain": [] }
}
```

JSON is stable: v1 fields are specified; additions appear under a `"v2"` or similar nested key to preserve forward-compatibility for scripts.

### 4.4 `--format text`

Tabular text for terminal skimming. No guarantees about line breaking or column alignment beyond "fits in a typical terminal width."

## 5. Lossless round-trip guarantee

### 5.1 Contract

For any committed Episode `@E` with agent-written forms `F_1, F_2, ..., F_n`:

```
mimir-cli decode @E → L_1, L_2, ..., L_n  (Lisp S-expressions)
```

Such that re-committing `L_1, ..., L_n` via `Store::commit_batch` (`wire-architecture.md` § 3.1) would compile to canonical records with agent-visible fields **byte-identical** to the original `F_i`'s canonical records.

### 5.2 What's preserved

- `s`, `p`, `o` for Semantic / Inferential.
- `event_id`, `kind`, `participants`, `location` for Episodic.
- `rule_id`, `trigger`, `action`, `precondition` for Procedural.
- `derived_from`, `method` for Inferential.
- `source`, `confidence` (stored), `valid_at` (where agent-provided).
- `:projected` flag when set.
- Query predicates (for query forms).
- Symbol declarations with `:Kind` annotations when the symbol was first introduced with one.

### 5.3 What's not preserved in `--format lisp`

Librarian-assigned fields are not re-enqueueable:

- `committed_at` — assigned fresh on re-enqueue.
- `observed_at` for non-Episodic memories — assigned = `committed_at` fresh.
- `invalid_at` — recomputed from current DAG state on re-enqueue.
- Effective confidence — recomputed per current decay config.
- Supersession edges, Episode-level metadata (parent, retracts, status) — reconstructed by the new re-enqueue, not copied.

These appear in `--format annotated` and `--format json` but are elided from `--format lisp`.

### 5.4 Why this matters

Lossless round-trip enables:

- **Workspace replay** for testing or staging builds.
- **Migration** between Mimir versions with format-version bumps (export with old decoder, import with new librarian).
- **Audit reproducibility** — re-running the original enqueues deterministically reaches the same canonical state (given the same workspace config).

## 6. Integrity verification

`mimir-cli verify` checks:

1. **Framing integrity.** Every record's `[opcode][varint length][body]` framing is consistent; body size matches length; no torn writes mid-record.
2. **Opcode validity.** Every opcode is in the registered set per `ir-canonical-form.md` § 4.
3. **Symbol references resolve.** Every `SymbolId` referenced in a record resolves to an entry in the symbol table.
4. **DAG acyclicity.** Supersession edges form a DAG; no cycles.
5. **Supersession consistency.** Every `SUPERSEDES` edge has a valid `from` and `to` memory; `to.invalid_at` matches `from.valid_at` (or `from.committed_at` for Procedural).
6. **CHECKPOINT completeness.** Every batch of records between two CHECKPOINTs has a closing CHECKPOINT; no orphan records remain (recovery should have truncated them).
7. **Snapshot-vs-log consistency.** Symbol-table and DAG snapshots reproduce the same state as log replay from the snapshot's commit point.
8. **Clock monotonicity.** `committed_at` values are monotonically non-decreasing per workspace.

Reports warnings (`W:`) and errors (`E:`) with line numbers / offsets. Exits 0 if clean, 1 if warnings, 2 if errors. Never modifies state.

## 7. Query grammar parity with agents

`mimir-cli query "<lisp>"` accepts the exact same query grammar as agents (`read-protocol.md` § 4). Identical semantics — same predicates, same flags, same tolerances. Users (who may be operators diagnosing workflows) see what the agent sees.

Example:

```
$ mimir-cli query '(query :s @alain :kind semantic :debug_mode true)'
```

Returns the same result shape an agent would receive, rendered per `--format`.

## 8. Streaming for large outputs

`mimir-cli log` streams output as records are decoded — no full-file buffering. This makes `mimir-cli log | less` and `mimir-cli log --from T | grep …` natural and low-memory even for gigabyte logs.

Filters:

- `--from T` / `--to T` — time-range bound by `committed_at`.
- `--opcode OP` — restrict to records with matching opcode.
- `--kind K` — restrict to records of memory type K.
- `--episode @E` — restrict to an Episode's records.

`query`, `inspect`, and `history` are not streaming (result sets are bounded and small per operation); they buffer before printing to produce consistently-aligned output.

## 9. Access mode

`mimir-cli` reads `canonical.log` (and, when they land, snapshot files) directly from the workspace's data directory. There is no live / daemon mode: `wire-architecture.md` scopes v1 deployment to in-process only, so there is no daemon to connect to and no live-librarian read path distinct from file reads.

Consequences:

- `mimir-cli` works on any workspace the invoking user has filesystem access to: a currently-open workspace, an archived one, or a post-mortem snapshot. No coordination with a running agent is needed.
- If an agent is actively writing to the workspace while `mimir-cli` reads, the CLI sees a consistent prefix of the log at open time (filesystem read), but will not see writes that committed after that open. Re-run to pick up newer state.
- Read-only by construction: the binary never opens the log for write; the underlying `CanonicalLog` backend is opened in append-only mode, and `mimir-cli` never uses the append path.

## 10. Invariants

1. **Read-only.** `mimir-cli` never calls `Store::commit_batch`, never appends to `canonical.log`, never mutates snapshots. Any invocation that would require a write fails.
2. **Deterministic rendering.** Given the same workspace state and same `--format`, output is byte-identical across runs.
3. **Lossless round-trip for agent-visible fields.** Per § 5.1.
4. **Streaming integrity.** `log` output lines correspond 1:1 to canonical records — no duplication, no elision, in canonical-order.
5. **Filesystem-only access.** `mimir-cli` opens `canonical.log` (and, when they land, snapshot files) directly. No sockets, no processes, no daemon — matches `wire-architecture.md`'s in-process-only scope.
6. **Format correctness.** `--format lisp` output is valid per `ir-write-surface.md`. `--format json` output is valid JSON. `--format annotated` is valid Lisp with `;` comments per standard Scheme syntax.

## 11. Open questions and non-goals for v1

### 11.1 Open questions

**Interactive REPL.** `mimir-cli interactive` is flagged post-MVP (§ 3.2). Value is high for exploratory debugging; complexity (history, autocomplete, multi-query session) warrants its own design. Defer.

**Export / import for workspace migration.** `mimir-cli export` dumping a workspace + `mimir-cli import` reconstructing one is distinct from `decode` (which is Episode-scoped) and distinct from `verify` (which is integrity-only). Candidates for `workspace-export.md` follow-up spec.

**Rendering large blob values.** A quoted-string `o` slot could in principle contain megabytes. v1 truncates at 4 KB for `annotated` / `text`; full value available via `--no-truncate`. Post-MVP: better inline rendering, diff-friendly chunking.

### 11.2 Non-goals for v1

- **Write operations.** `mimir-cli` is exclusively read-only.
- **GUI.** Terminal-only in v1.
- **Performance monitoring / metrics export.** The observability surface (`docs/observability.md`) is the agent-side tracing channel; the decoder does not scrape it.
- **Remote workspaces over network.** Machine-local only, matching `workspace-model.md` § 2.
- **Schema evolution tooling.** Format-version migrations are a distinct concern; post-MVP.
- **Plugins / extensions.** No custom output formatters; no query-language macros; no scripting hooks in v1.

## 12. Primary-source attribution

All entries are verified per `docs/attribution.md`.

- **Unix tooling philosophy** (McIlroy, Pike et al., pending) — composable single-purpose tools read stdin, write stdout, exit with status. `mimir-cli` structure follows this lineage.
- **`sqlite3` CLI reference** ([sqlite.org/cli.html](https://www.sqlite.org/cli.html), pending) — canonical example of a small embedded-database inspection tool; model for `mimir-cli`'s subcommand structure and `--format` options.
- **`pg_dump` / `pg_restore` design** (PostgreSQL documentation, pending) — prior art for round-trippable textual dump of a structured store. Mimir's `decode` and the post-MVP `export` follow this pattern.
- **JSON Lines / ND-JSON** ([jsonlines.org](https://jsonlines.org/), pending) — streaming structured output per record. Mimir's `--format json` on streaming subcommands emits one JSON object per line, matching this convention.
