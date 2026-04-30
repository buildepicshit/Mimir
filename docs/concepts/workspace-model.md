# Workspace Model

> **Status: authoritative 2026-04-18; scope amended 2026-04-24.** Graduated from `citation-verified` on 2026-04-18 backed by `mimir_core::workspace::WorkspaceId` (SHA-256 of normalised origin URL per § 3.1, `from_ulid` for explicit non-git workspaces per § 3.2, `detect_from_path` implementing the § 3.3 ancestor walk + `.git/config` origin lookup), `mimir_core::store::Store::open_in_workspace` (disjoint `data_root/<hash>/canonical.log` directories per § 4.2, structurally preventing cross-workspace contamination), and property tests `workspace_id_determinism` + `workspace_partitioning_is_structural` covering graduation criterion #3. `.git/config` parsing is in-process (no `git` subprocess dep) and tolerant of quoted/unquoted section headers and standard `#`/`;` comments. This spec remains authoritative for project-local partitioning. Cross-scope promotion and multi-agent memory governance are now owned by draft [`scope-model.md`](scope-model.md).

Mimir partitions memory by Workspace. A Workspace is a project / repo / named-context that owns a dedicated symbol table, a dedicated canonical store, and a dedicated write stream. Cross-workspace contamination is structurally impossible, not policy-enforced.

## 1. Scope

This specification defines:

- What a Workspace is in Mimir.
- How workspaces are identified (git-backed and non-git).
- Partitioning of symbol tables, canonical stores, write streams, and ephemeral scopes per workspace.
- Read scope (workspace-local only).
- Write-scope enforcement.

This specification does **not** define:

- Memory type shapes — `memory-type-taxonomy.md`.
- The source-type taxonomy — `grounding-model.md` (which operates within a workspace's scope).
- Symbol ID allocation and lifecycle — `symbol-identity-semantics.md` (which operates under this spec's workspace partition).
- On-disk canonical form layout — `ir-canonical-form.md` (which implements the per-workspace partition).

**Out of scope for this spec.** Cross-workspace reads, cross-workspace writes, cross-workspace import, multi-agent coordination, and workspace federation are not defined by the workspace-local implementation model. The 2026-04-24 mandate introduces governed cross-scope promotion in [`scope-model.md`](scope-model.md); that mechanism is explicit promotion across scopes, not raw workspace sharing.

### Graduation criteria

Graduates draft → authoritative when:

1. Related art is verified in `docs/attribution.md` (candidates: Postgres schema isolation, SQLite `ATTACH DATABASE`, git worktree isolation, Claude Code project directory convention).
2. A Rust `WorkspaceId` newtype plus detection logic compiles with the invariants in § 6.
3. Property tests cover write-scope enforcement and read-default-scope behavior.

## 2. Design thesis: workspaces prevent contamination structurally

Agent memory drift across unrelated projects is catastrophic. An agent working on project A must never silently pull facts from project B — even if symbols happen to share a name (two different `@alain` references, two different `@mimir` references in different projects).

Mimir handles this by **hard partitioning**. Each project is a workspace; each workspace owns:

- A dedicated symbol table (so `@alain` in workspace A is a *distinct symbol* from `@alain` in workspace B, not an alias).
- A dedicated canonical store (separate append log, separate symbol-table snapshot, separate supersession DAG).
- A dedicated write stream (no shared queue between workspaces).

The librarian enforces the partition at bind time. A write without an active workspace is rejected. A read operates only against the active workspace — there is no cross-workspace read API.

### Why hard partition over shared-backend-with-tenant-id

Tenant-id-as-column in a shared store is a leaky abstraction. Every query must carry a tenant predicate; one missing predicate leaks cross-tenant data. Hard partitioning means contamination is structurally impossible — there is no shared backend to query incorrectly.

This matches the operational pattern Mimir was conceived from. Claude Code scopes repository instructions and memory directories to per-project hashes (`~/.claude/projects/<hash>/memory/`), and Claude Code's memory is machine-local by design — never synchronized across machines ([Claude Code memory docs](https://code.claude.com/docs/en/memory.md)). Mimir inherits the machine-local default; cross-machine replication is a post-MVP candidate (§ 8.2). Mimir formalizes the per-project convention into a first-class architectural boundary.

## 3. Workspace identity

A workspace is identified by a stable `WorkspaceId`.

### 3.1 Git-backed workspaces (default)

When the runtime environment sits inside a git repository:

```
WorkspaceId = hash(canonical_git_remote)
```

Where `canonical_git_remote` is the lowercased, trailing-`.git`-stripped URL of the `origin` remote.

**Branch is not part of `WorkspaceId` by default.** Rationale: most development workflows want branch-agnostic memory — a feature branch should inherit the repo's canonical memories. Branch-scoped memory for cases like long-lived maintenance branches is available via an explicit named workspace (§ 3.2).

Consequences:

- A fork is a new workspace (different remote).
- Mirror clones are the same workspace (same remote).
- All branches of a repo share the workspace's memory by default.
- Multi-branch isolation requires explicit configuration.

**Alignment note.** Claude Code's convention uses the local git repository path for its `<project>` hash in `~/.claude/projects/<project>/memory/`. Mimir uses the normalized `origin` remote URL instead. The Mimir approach is stricter (mirror clones in different local paths converge to the same workspace, which matches developer intent) and safer (no accidental sharing across unrelated local checkouts that happen to share a path prefix). Both approaches deliver per-project isolation; Mimir's is remote-identity-scoped rather than local-path-scoped.

### 3.2 Non-git workspaces (explicit)

When no git repository is found at or above CWD, a workspace is initialized explicitly:

```
mimir-cli init --workspace <name>
```

This allocates a stable `WorkspaceId(Ulid)` and records it in a workspace manifest. The manifest location is a librarian-pipeline config detail (`librarian-pipeline.md`); candidate: `.mimir/workspace.toml` in the project root, or `~/.mimir/workspaces/<name>.toml` for user-global.

Named workspaces are also the mechanism for the edge case above: a repo that wants branch-scoped memory creates explicit named workspaces per branch and chooses which one to activate.

### 3.3 Workspace detection

When `Store::open_in_workspace` / `Store::open` is called (or, equivalently, when a CLI tool like `mimir-cli` opens a log), the active workspace is resolved in order:

1. Walk up from CWD looking for `.git/`. If found, read the `origin` remote and compute `WorkspaceId` per § 3.1.
2. Walk up from CWD looking for a `.mimir/` marker. If found, read the named workspace from its manifest.
3. Otherwise, require explicit `mimir-cli init` or an `--workspace=<id>` flag. Writes without an active workspace are rejected with `WorkspaceError::NoActiveWorkspace`.

## 4. Partitioning

### 4.1 Symbol table per workspace

Each workspace has a dedicated symbol table. Symbol allocation, rename, alias, and retirement are workspace-scoped. A symbol resolved in workspace A is not visible in workspace B.

Symbol identity across workspace boundaries uses a scoped wrapper:

```rust
pub struct ScopedSymbolId {
    pub workspace: WorkspaceId,
    pub local: SymbolId,
}
```

Within a workspace the librarian uses bare `SymbolId` internally; the `workspace` component is implicit from context. The `ScopedSymbolId` form is exposed only at inspection boundaries (decoder-tool output, audit logs) to make the workspace component explicit in diagnostic surfaces — not for cross-workspace reference.

Full semantics in `symbol-identity-semantics.md`.

### 4.2 Canonical store per workspace

Physical partitioning: one canonical store per workspace. Each store is an independent append-only log plus symbol-table snapshot plus supersession DAG.

Implementation shape (binding in `ir-canonical-form.md` and `librarian-pipeline.md`):

```
~/.mimir/data/
    <workspace_id>/
        canonical.log
        symbols.snapshot
        dag.snapshot
        episodes.log
```

Different workspaces share no files, no symbol IDs, and no canonical state.

### 4.3 Write stream per workspace

Each workspace has its own write queue and WAL stream. Different Claude instances each attach to their own workspace; their write streams are independent. The "librarian is single-writer" invariant (PRINCIPLES.md § Architectural Boundaries #1) is enforced per-workspace — one writer, one workspace.

### 4.4 Ephemeral scope interaction

Ephemeral memories nest inside workspace scope. An ephemeral memory created in workspace A has scope `Session(session_id)` *in workspace A*; it is invisible from workspace B regardless of session overlap. Ephemeral → canonical promotion is within-workspace only; there is no cross-workspace promotion.

## 5. Read scope

Every read operates against the active workspace and only the active workspace. There is no API to read from another workspace, and there is no import / copy / federation path. A Claude instance sees exactly the memories its own workspace contains; if it needs knowledge from a different context, that context is owned by a different Claude in a different workspace, and the correct answer is not to share it.

This is the load-bearing isolation mechanism on the read side. An agent cannot accidentally — or deliberately — see another workspace's memories.

## 6. Write-scope enforcement

At bind time, every write is tagged with the active workspace:

```rust
pub struct BoundWrite {
    pub workspace: WorkspaceId,
    pub memory: MemoryKind,
    // other fields per write-protocol.md
}
```

Enforcement rules:

- If the librarian receives a write with no active workspace, it returns `WorkspaceError::NoActiveWorkspace`.
- If the librarian receives a write whose declared target workspace does not match the active one (e.g., through a stale `--workspace=Y` flag while connected to workspace X), it returns `WorkspaceError::WorkspaceMismatch`. Crossing workspaces requires reconnection, not a flag override in-flight.
- Workspace ID values in `BoundWrite.memory`'s `ScopedSymbolId` fields must equal `BoundWrite.workspace`. Any foreign workspace reference in a write's symbol list is rejected.

## 7. Interactions with other specs

- **Symbol identity:** `ScopedSymbolId` + per-workspace allocation — `symbol-identity-semantics.md`.
- **Memory types:** every memory's `Symbol` fields are implicitly workspace-scoped via `ScopedSymbolId` — `memory-type-taxonomy.md`.
- **Grounding:** sources resolve within the active workspace — `grounding-model.md`.
- **Canonical form:** on-disk layout per workspace — `ir-canonical-form.md`.
- **Read protocol:** workspace-local read behavior — `read-protocol.md`.
- **Write protocol:** workspace tagging at bind + enforcement rules — `write-protocol.md`.

## 8. Open questions and non-goals for v1

### 8.1 Open questions

**Workspace config format.** Where workspace metadata lives — repo-root `.mimir/workspace.toml` (for git-backed, travels with the repo) vs user-global `~/.mimir/workspaces/<id>.toml` (local) vs both. Decide in `librarian-pipeline.md`.

**Git remote renames.** If the `origin` URL changes (org rename, host migration), `WorkspaceId` changes under § 3.1. Workspace memories do not auto-migrate. A `mimir-cli workspace relink --from <old> --to <new>` tool is post-MVP (§ 8.2).

**Ephemeral across workspace switches.** If an agent switches active workspaces mid-session, what happens to ephemeral memories scoped to the prior workspace? Default: evict on scope end — the session has effectively ended from the new workspace's perspective. Revisit if real workflows demand hand-off semantics.

**Branch-scoped-by-default cases.** The branch-agnostic default (§ 3.1) is right for most workflows, but exceptions exist (security-sensitive maintenance branches with different access policies). The explicit-named-workspace escape hatch covers this; if exceptions become common, promote branch scoping to a config flag.

**Per-memory size budget.** Anthropic's Managed Agents API caps individual memories at ~100 KB ([Managed Agents memory docs](https://platform.claude.com/docs/en/managed-agents/memory.md)) — encouraging a many-small-memories pattern rather than a few large ones. Mimir currently imposes no per-memory byte budget. Decide in `canonical-form.md` or `librarian-pipeline.md`: cap per memory (and at what value), or leave uncapped and rely on supersession / decay to keep individual records small. Default recommendation: enforce a librarian-configurable cap (initial value ~16 KB per memory for the canonical form; agents that need more structure decompose into multiple linked memories).

### 8.2 Non-goals for the workspace-local implementation

- **Raw cross-workspace reads, writes, imports, consolidation, federation.** None of these exist in the workspace-local implementation. A workspace is a sealed unit unless a later scope-aware promotion flow emits a new record in another scope.
- **Access control.** Since there is no cross-workspace surface, there is no access-control surface either. The filesystem boundary on `~/.mimir/data/<workspace_id>/` is the only access control.
- **Cross-machine synchronization / replication.** Workspaces are machine-local by default (§ 2). A sync/replication protocol for multi-machine deployments is post-MVP. Candidate spec: `workspace-sync.md`.
- **Workspace versioning / branching as a primitive.** A workspace's canonical history is linear; forking a workspace (like a git branch with its own divergent state) is not a v1 primitive. Use separate named workspaces if this is needed.
- **Git-remote relink tooling.** Renaming a git remote leaves the old `WorkspaceId` orphaned. A relink utility is post-MVP.
- **Per-deployment canonical output artifacts.** Compiling a workspace's canonical store into a deployable fleet-agent instruction bundle is a post-MVP feature candidate. Captured here so the concept is not lost; candidate spec name `deployment-compilation.md`.

## 9. Primary-source attribution

All entries are verified per `docs/attribution.md`. None is load-bearing for the partitioning decisions above — those derive from Mimir's architectural principles (hard partitioning over shared-backend) and the operational precedent in Claude Code.

- **Claude Code memory model** ([`https://code.claude.com/docs/en/memory.md`](https://code.claude.com/docs/en/memory.md), pending) — repository instruction scoping, `~/.claude/projects/<hash>/memory/` per-project isolation, and machine-local-by-default stance directly informed this design. Verified against § 2, § 3.1, and the machine-local inheritance in § 8.2.
- **Anthropic Managed Agents API memory stores** ([`https://platform.claude.com/docs/en/managed-agents/memory.md`](https://platform.claude.com/docs/en/managed-agents/memory.md), pending) — explicit memory-store attachment with `read_only` vs `read_write` access control, per-memory size caps (~100 KB), optimistic concurrency via content hashes. Cited for the post-MVP access-control direction in § 8.2 and the per-memory size-budget open question in § 8.1.
- **Anthropic Managed Agents API sessions** ([`https://platform.claude.com/docs/en/managed-agents/sessions.md`](https://platform.claude.com/docs/en/managed-agents/sessions.md), pending) — session-level vs durable resource distinction. Referenced for the ephemeral-vs-canonical distinction in `memory-type-taxonomy.md`.
- **Postgres schema-based tenant isolation** (verified) — the shared-backend-with-tenant-predicate pattern. Mimir diverges (hard partitioning over shared-backend); cited as prior art for why Mimir chose the other path.
- **SQLite `ATTACH DATABASE`** (verified) — lightweight per-workspace database file model; similar spirit to Mimir's per-workspace canonical store.
- **Git worktree isolation** (verified) — each worktree has independent HEAD, index, stash on a shared object database; analogue for how workspaces could one day share an object store while keeping state partitioned.
