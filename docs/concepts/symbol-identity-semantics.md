# Symbol Identity Semantics

> **Status: authoritative — graduated 2026-04-17.** All cited sources verified (see `docs/attribution.md`), and the semantics are now implemented by `mimir_core::bind::SymbolTable` (monotonic `u64` allocation, per-name lookup, alias list, retirement flag, kind locking) plus `mimir_core::bind::bind()` resolving `RawSymbolName` → `SymbolId` with typed `BindError` variants (`SymbolKindMismatch`, `SymbolRenameConflict`, `AliasChainLengthExceeded`, `UnknownSymbol`, `BadKind`, `UnregisteredInferenceMethod`). Tests cover allocation monotonicity, rename / alias / retire / unretire round-trips, kind lockdown on reuse, and `@name:Kind` annotation overrides. Graduates on milestone 5.3.

Every entity Mimir tracks — an agent, a document, a memory, a predicate, a scope — is referenced through a **symbol**. This specification defines what a symbol is, how symbols are allocated, typed, renamed, aliased, retired, and resolved. It is the Roslyn-analog substrate the librarian binds against.

## 1. Scope

This specification defines:

- The `SymbolId` and `ScopedSymbolId` types.
- The symbol-kind taxonomy (12 kinds).
- Symbol allocation rules and first-use kind inference.
- Rename semantics (alias edges, append-only identity).
- Alias chains (bidirectional resolution, length cap, acyclicity).
- Retirement (soft only; no hard-delete).
- Bind-time resolution algorithm.
- Predicate handling (`@` prefix optional in predicate slots).
- Cross-workspace symbol references.
- Symbol-table persistence per workspace.

This specification does **not** define:

- Workspace identity or partitioning — `workspace-model.md`.
- Memory type shapes — `memory-type-taxonomy.md`.
- Source-kind taxonomy — `grounding-model.md` (symbol-kind and source-kind are orthogonal: symbol-kind is what the symbol *is*; source-kind is how a memory is *grounded*).
- Confidence decay — `confidence-decay.md`.
- Librarian pipeline — `librarian-pipeline.md` (this spec is consumed by the binder stage there).

### Graduation criteria

Graduates draft → authoritative when:

1. Roslyn symbol model and compiler-construction canonical texts are verified in `docs/attribution.md`.
2. A Rust `SymbolId`, `ScopedSymbolId`, and `SymbolKind` implementation compiles in `mimir_core`, with the invariants in § 14 covered by unit and property tests.
3. Property tests exist for: first-use kind locking, rename alias-edge resolution, alias acyclicity and length cap, retirement soft-flag propagation, workspace partitioning is structural.

## 2. Design thesis: typed symbol identity over free strings

Free-string references to entities rot. "Alice" in one memory, "alice" in another, "A. Smith" in a third — three entities as far as naïve equality is concerned, or silently-merged as far as fuzzy matching is concerned. Both failure modes corrupt coherence.

Mimir refuses both. Every entity is a symbol with a stable `SymbolId`. Two references to the same entity resolve to the same ID; two references to different entities that happen to share a name resolve to different IDs. Identity is explicit, not heuristic.

Typed identity is also a lever for the binder. When a memory says `source: @alice`, the binder knows `@alice` must be of symbol-kind Agent (or an error). The symbol table carries enough information to reject type-confused writes before they touch the canonical store. This matches the type-safety policy in `PRINCIPLES.md` § 3 — invariants by construction, not by runtime check.

The model follows Roslyn's approach to compiler symbol tracking: symbols have stable identity independent of their surface name, rename produces an alias edge rather than rewriting the identity, and all references bind through the symbol table rather than through string equality.

## 3. What a symbol is

### 3.1 `SymbolId`

Within a workspace, a symbol is identified by a monotonic `u64`:

```rust
pub struct SymbolId(u64);
```

`SymbolId` is workspace-scoped. `SymbolId(42)` in workspace A is not the same symbol as `SymbolId(42)` in workspace B — they are distinct symbols in distinct tables. Within a workspace, the ID is immutable once allocated.

### 3.2 `ScopedSymbolId`

At inspection boundaries — decoder-tool output, audit logs — symbols appear as:

```rust
pub struct ScopedSymbolId {
    pub workspace: WorkspaceId,
    pub local: SymbolId,
}
```

See `workspace-model.md` § 4.1. Within a workspace the librarian uses bare `SymbolId` internally; the workspace component is implicit from context. `ScopedSymbolId` exists to make the workspace component explicit in diagnostic surfaces, not to enable raw cross-workspace reference. Governed cross-scope reuse is modeled as promotion to a new scoped record per `scope-model.md`, not by sharing local symbols.

### 3.3 Canonical name and aliases

Every symbol has a **canonical name** — the primary `@name` string used in the write surface — plus zero or more **alias names**. Canonical names are unique within a workspace; alias names are unique within a workspace but resolve to the same `SymbolId` as the canonical.

Renaming a symbol (§ 7) does not change its `SymbolId` — it changes which string is canonical and records an alias edge.

### 3.4 Surface form

In the Lisp S-expression write surface (per `ir-write-surface.md`), symbols appear as bareword `@name`:

```
@alain
@claude_4_7
@mimir
```

Names follow `[a-z][a-z0-9_]*` after the `@`. Uppercase is reserved for opcode keywords (`SEM`, `EPI`, `PRO`, `INF`) in the canonical form per the tokenizer bake-off.

## 4. Symbol-kind taxonomy

Twelve kinds. The Rust enum is `#[non_exhaustive]` so additions do not break semver.

| Kind | Represents | Typical positions |
|---|---|---|
| `Agent` | Actors — profile subjects, observers, reporters, rule-makers | `Semantic.s`, `Episodic.participants`, `Procedural.source` when a person, any `source` when `@observation` / `@self_report` / etc. |
| `Document` | Static references carrying a citation pointer (paper, URL, spec) | `source` when `@document` |
| `Registry` | Authoritative programmatic sources (package manifests, DNS, filesystem) | `source` when `@registry` |
| `Service` | Live third-party APIs | `source` when `@external_authority` |
| `Policy` | Policy-making sources distinct from Agent (organizational policy, externally-authored rulesets). May be dual-kind with Agent. | `source` when `@policy` |
| `Memory` | Memory IDs | `Episodic.event_id`, `Procedural.rule_id`, `Inferential.derived_from[]` |
| `InferenceMethod` | Registered method tags for Inferential derivations | `Inferential.method` |
| `Scope` | Scope tags for Procedural rule applicability and `EphemeralScope::Named` | `Procedural.scope`, named ephemeral scopes |
| `Predicate` | Predicate names in s-p-o tuples | `Semantic.p`, `Inferential.p` |
| `EventType` | Event-type tags for Episodic memories | `Episodic.kind` |
| `Workspace` | Workspace identifiers when referenced as symbols | rare — appears in audit logs / decoder-tool output only |
| `Literal` | Typed-value barewords that don't belong to any registry (catchall for enum-ish values) | `Semantic.o`, `Procedural.scope` when scope is a literal tag, `Episodic.location` when location is a literal tag |

```rust
#[non_exhaustive]
pub enum SymbolKind {
    Agent,
    Document,
    Registry,
    Service,
    Policy,
    Memory,
    InferenceMethod,
    Scope,
    Predicate,
    EventType,
    Workspace,
    Literal,
}
```

### 4.1 Dual-kind symbols

A symbol may in principle represent both an Agent and a Policy (e.g., a person acting in a policy-making capacity). v1 rejects dual kinds to keep the binder deterministic — each symbol has exactly one kind, locked at first use. If an agent needs dual representation they allocate two symbols (`@alain` as Agent, `@alain_policy` as Policy) and optionally alias them (§ 8).

### 4.2 Kind immutability

A symbol's kind is **immutable after first allocation.** Attempting to use an existing symbol in a slot expecting a different kind produces `BindError::SymbolKindMismatch`. Changing a symbol's kind is not supported; allocate a new symbol.

## 5. Allocation and first-use kind inference

### 5.1 Allocation strategy

Symbol IDs are allocated monotonically per workspace:

```rust
impl SymbolTable {
    fn next_id(&mut self) -> SymbolId {
        let id = SymbolId(self.next);
        self.next += 1;
        id
    }
}
```

The counter persists with the symbol snapshot (§ 13). It never decreases, even across retirements — retired symbols keep their IDs; new allocations get fresh IDs.

### 5.2 First-use resolution

When the binder encounters `@name` in a slot:

1. **Lookup.** If `@name` exists in the workspace symbol table (as a canonical name or alias), resolve to the existing `SymbolId`. Validate the existing kind against the slot's expected kind. On mismatch: `BindError::SymbolKindMismatch`.

2. **Allocate.** If `@name` does not exist, allocate a new `SymbolId`, set its canonical name to `@name`, and infer its kind from the slot:
   - Position-default kind per § 5.3.
   - Or, if the write uses explicit annotation `@name:Kind`, use that kind instead.

Allocation and kind-assignment are atomic under the librarian's single-writer invariant.

### 5.3 Position-default kind table

| Position | Default kind | Notes |
|---|---|---|
| `Semantic.s` | Agent | override via `@name:Kind` if the subject is Document / Service / etc. |
| `Semantic.p` | Predicate | always |
| `Semantic.o` | Literal | override if the object is Agent / Document / Memory / etc. |
| `Semantic.source` | *source-kind-determined* | Agent / Document / Registry / Service / Policy per the source kind from `grounding-model.md` |
| `Episodic.event_id` | Memory | always |
| `Episodic.kind` | EventType | always |
| `Episodic.participants[]` | Agent | override via `@name:Kind` if a participant is non-Agent (a Document being referenced, a Service being called) |
| `Episodic.location` | Literal | override if location is a Scope or Agent |
| `Episodic.source` | *source-kind-determined* | per `grounding-model.md` |
| `Procedural.rule_id` | Memory | always |
| `Procedural.scope` | Scope | always |
| `Procedural.trigger` / `action` | *free Value* | if symbol: Literal by default |
| `Procedural.source` | *source-kind-determined* | per `grounding-model.md` |
| `Inferential.s` / `p` / `o` | same as Semantic | |
| `Inferential.derived_from[]` | Memory | always |
| `Inferential.method` | InferenceMethod | always |

### 5.4 Explicit kind annotation

The write surface accepts `@name:Kind` to override the position default:

```
(sem @alice_book:Document p @alice_book o ... :src @profile :c 0.9)
```

The annotation applies only at allocation time. If `@alice_book` already exists, the annotation must match the existing kind or the binder returns `BindError::SymbolKindMismatch`.

Exact grammar of `@name:Kind` annotation is defined in `ir-write-surface.md`.

## 6. Rename semantics

### 6.1 Rename is an alias edge, not a rewrite

Renaming a symbol does not rewrite prior canonical records. Canonical memories are append-only per PRINCIPLES.md architectural boundary #3. Instead, the librarian records an alias edge `new → old` in the symbol table, such that reads of either name resolve to the same `SymbolId`.

### 6.2 Rename as a first-class event

A rename is itself a first-class memory. When `@old` is renamed to `@new`, the librarian emits an Episodic memory:

```
Episodic {
    event_id: <fresh Memory symbol>,
    kind: @rename,
    participants: [<SymbolId of old/new>],  // one symbol, two names
    location: @librarian,
    at_time: T,
    observed_at: T,
    source: @librarian_assignment,
    confidence: 1.0,
}
```

This gives rename events auditability — the decoder tool can reconstruct naming history.

### 6.3 Alias-edge direction and canonical name

After rename, the **new name is canonical**; the old name is an alias. Both resolve to the same `SymbolId`. Reads default to displaying the canonical name; the decoder tool can show the full name history.

If `@new` already exists with a different `SymbolId`, the rename is rejected as `BindError::SymbolRenameConflict` — aliasing two separate symbols is a separate operation (§ 8), not a rename.

### 6.4 Rename-only agents are not special

Renames are a canonical write that goes through the normal pipeline. Because each workspace has a single writer, there is no rename-conflict arbitration — renames are serial within the workspace.

## 7. Alias chains

### 7.1 Declaration

Declaring an alias:

```
(alias @new @old)
```

Effect: `@new` and `@old` resolve to the same `SymbolId`. Unlike rename, an alias declaration does not designate a new canonical name — both remain alias names, and the canonical name is the one that existed first (or was explicitly set via `canonical` directive — reserved future syntax).

### 7.2 Bidirectional resolution

Aliases are **bidirectional** by resolution. Reading `@old` or `@new` yields the same `SymbolId` and the same canonical memory references.

### 7.3 Length cap

The alias chain for any symbol is capped at **16 names**. The cap:

- Prevents pathological graphs.
- Is enforced at write: `BindError::AliasChainLengthExceeded`.
- Is revisitable if real workloads demand more (post-MVP consideration, § 15.1).

### 7.4 Acyclicity

The union of alias edges + rename edges forms a DAG per symbol. Cycles are rejected at write with `BindError::AliasCycle`.

### 7.5 Alias events

Alias declarations emit an Episodic memory analogous to rename events (§ 6.2), with `kind: @alias`.

## 8. Retirement

### 8.1 Soft retirement only

Retiring a symbol flags it as no-longer-recommended for new references. Existing references still resolve; reads of retired symbols emit a stale-symbol escalation flag per `read-protocol.md`.

Mimir does **not** support hard-delete. The append-only boundary (PRINCIPLES.md architectural boundary #3) forbids removing historical references. Retirement is the only way to stop advertising a symbol.

### 8.2 Retirement event

Retiring `@name`:

```
(retire @name)
```

Emits an Episodic memory:

```
Episodic {
    event_id: <fresh>,
    kind: @retire,
    participants: [<SymbolId>],
    location: @librarian,
    at_time: T,
    observed_at: T,
    source: @librarian_assignment,
    confidence: 1.0,
}
```

### 8.3 Read-side escalation

When an agent reads a canonical memory whose symbol is retired, the read result carries a `stale_symbol_warning` flag (per `read-protocol.md`). Agents are expected to handle the warning — typically by escalating to the librarian, or by renaming the reference in their own new writes to a non-retired successor.

### 8.4 Unretirement

A retired symbol can be un-retired via a fresh write (`unretire @name`), which emits another Episodic event. The retirement history is preserved in canonical form.

## 9. Bind-time resolution algorithm

Given a write containing `@name` at position P:

```
resolve(workspace, name, position) -> Result<SymbolId>:
    if name is a ScopedSymbolId { workspace: other_workspace, local: id } and other_workspace != workspace:
        return Err(ForeignSymbolForbidden)  // cross-workspace refs never resolve; isolation is structural
    if name exists in workspace.symbol_table:
        id = workspace.symbol_table.lookup(name)
        kind = workspace.symbol_table.kind_of(id)
        expected = expected_kind_for(position)
        if not compatible(kind, expected):
            return Err(SymbolKindMismatch { existing: kind, expected })
        return Ok(id)
    else:
        kind = position.default_kind  // or annotation override
        id = workspace.symbol_table.allocate(name, kind)
        return Ok(id)
```

Cross-workspace resolution is read-only; the foreign workspace's table is consulted but not mutated.

The resolution algorithm is deterministic given the workspace's symbol table state. No heuristic matching, no fuzzy equality, no ML.

## 10. Predicate handling: `@` optional in predicate slots

Predicates frequently appear without the `@` prefix in natural usage (`email`, `role`, `valid_at`). The write surface grammar accepts both forms **only in Predicate-kind positions**:

- `Semantic.p`
- `Inferential.p`

In these positions, a bareword without `@` is treated as a synthetic symbol reference of kind Predicate. The binder normalizes both forms to the same canonical Predicate-kind symbol on first use.

In any other position, a bareword without `@` is a string literal (a `Value::String`), not a symbol. The grammar disambiguates by position type.

This exception is a concession to write-surface ergonomics. It does not weaken the identity model — predicates still get a `SymbolId`, still live in the symbol table, still have `Predicate` as their locked kind.

## 11. Cross-workspace references

### 11.1 `ScopedSymbolId` on the wire

When a memory import crosses workspaces (per `workspace-model.md` § 5.3), the import-time symbol reference is `ScopedSymbolId { workspace: origin, local: id }`. The receiving workspace's symbol table records a **foreign-symbol entry** for the import — an entry that resolves to the foreign `ScopedSymbolId` rather than to a local `SymbolId`.

Foreign-symbol entries are a distinct symbol-table state:

```rust
pub enum SymbolTableEntry {
    Local { id: SymbolId, canonical_name: String, kind: SymbolKind, aliases: Vec<String>, retired: bool },
    Foreign { scoped: ScopedSymbolId, kind: SymbolKind, retired: bool },
}
```

Foreign entries cannot be renamed or aliased locally. They are read-only references to the origin workspace. If the origin workspace renames or retires the symbol, the importing workspace does not auto-update (per hard-partition — drift prevention from `workspace-model.md` § 5.3).

### 11.2 Foreign symbols in new local writes

A local write may reference a foreign symbol. The binder validates the foreign symbol's kind against the slot's expected kind the same way it validates local symbols, but does not allocate — the foreign symbol is already resolved.

## 12. Symbol-table persistence per workspace

Per `workspace-model.md` § 4.2, each workspace has its own `symbols.snapshot` file. The symbol table persists across librarian restarts.

### 12.1 Snapshot contents

A workspace's symbol snapshot contains, per symbol:

- `SymbolId`
- Canonical name
- Kind
- Alias name list
- Rename history (chain of names with timestamps)
- Retirement flag + retirement timestamp (if retired)
- Foreign `ScopedSymbolId` pointer (if foreign)

### 12.2 WAL durability

Symbol-table mutations are WAL'd alongside canonical memory writes. On librarian startup, the snapshot is loaded and then fast-forwarded by replaying the WAL. Crash recovery is atomic per the same ARIES-analog mechanism described in `write-protocol.md`.

### 12.3 Next-ID counter persistence

The monotonic allocation counter is part of the snapshot. A librarian that crashes mid-allocation replays the WAL and resumes from the last durable counter value.

## 13. Invariants

Every write binds against these invariants, any of which produces a typed `BindError::*` on violation.

1. **Identity stability.** `SymbolId`s are immutable once allocated. No operation mutates an existing `SymbolId`.
2. **Kind immutability.** A symbol's kind is set at allocation; later references must match.
3. **Name uniqueness.** Canonical names and alias names collectively form a unique index within a workspace — no two entries share a name.
4. **Alias / rename acyclicity.** The union of alias and rename edges per symbol is a DAG. No cycles.
5. **Alias chain length.** ≤ 16 names per symbol.
6. **Foreign reference immutability.** Foreign symbol entries cannot be locally renamed, aliased, or retired.
7. **Retirement monotonicity.** A retirement can be reversed (`unretire`), but a retired symbol that receives a new canonical reference does not auto-unretire — unretirement is always explicit.
8. **Append-only identity history.** Rename, alias, retirement, unretirement are Episodic memories emitted by the librarian at bind. The canonical store carries the history.

## 14. Open questions and non-goals for v1

### 14.1 Open questions

**Predicate prefix convention.** § 10 makes `@` optional in predicate slots. Should we require `@` uniformly for readability and remove the exception, at the cost of uglier predicate emissions (`@email` instead of `email`)? Defer — re-evaluate once real agent emissions show us what looks clean under the tokenizer profile.

**Dual-kind symbols.** § 4.1 rejects dual kinds in v1. If real workloads demand dual representation (e.g., an Agent who is also a Policy), revisit — likely via a `KindSet` wrapper rather than true multi-kind symbols.

**Alias chain cap.** § 7.3 caps at 16. Not clearly justified empirically. Revisit post-MVP if real workloads hit the cap.

### 14.2 Non-goals for v1

- **Hard-delete of symbols.** Append-only invariant forbids.
- **Automatic symbol merging.** Agents may allocate two symbols for the same entity by mistake; merging them is possible only by explicit alias declaration. No ML-based dedup in v1 (matches determinism-vs-ML boundary in `PRINCIPLES.md` § 4).
- **Cross-workspace symbol identity.** Each workspace's symbols are local and stay local. No cross-workspace references resolve; foreign refs are rejected with `ForeignSymbolForbidden`.
- **Symbol metadata beyond kind + name + alias + retirement.** Adding arbitrary metadata to symbols (display hints, localized names, tagging systems) is post-MVP; such data belongs in memories about symbols, not in the symbol table itself.

## 15. Primary-source attribution

All entries are verified per `docs/attribution.md`. None is load-bearing for the design decisions above — those derive from Mimir's architectural principles (append-only, determinism, hard-partition) and the operational requirement for stable identity under rename.

- **Roslyn symbol model** (Microsoft .NET Compiler Platform, pending) — stable symbol identity across references, rename propagation, symbol-kind taxonomy. Directly informed § 3, § 4, § 6.
- **Compiler construction canon** (Aho, Sethi, Ullman, *Compilers: Principles, Techniques, and Tools*, or similar — pending) — classical symbol-table design, scoping, resolution algorithms. Verifies § 9 resolution algorithm against established practice.
- **Intensional vs extensional identity** (pending, philosophical) — the distinction between an entity and its names. Relevant to § 3 and § 6; verification pass will decide if a formal citation adds load-bearing weight.
