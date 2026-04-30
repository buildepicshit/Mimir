# Scope Model

> **Status: draft 2026-04-24.** Drafted after the 2026-04-24 mission expansion from project-local Claude memory toward a multi-agent memory control plane. The mandate is accepted in `PRINCIPLES.md`; this spec is not implemented yet. It supersedes `workspace-model.md` only for the cross-scope promotion model; the current code still implements hard project/workspace partitioning.

Mimir's original workspace model made contamination structurally impossible by forbidding cross-workspace reads, writes, imports, and coordination. That was the right safety baseline for a single-agent memory system.

The expanded mission is broader: Mimir should govern memory across a studio ecosystem where Claude, Codex, MCP clients, and future harnesses can contribute memories without turning shared memory into a contamination path. This scope model preserves the safety intent of hard isolation while adding a controlled promotion path for reusable knowledge. Cross-agent deliberation is specified separately in [`consensus-quorum.md`](consensus-quorum.md); quorum artifacts enter this scope model as drafts or review records, not as direct canonical writes.

## 1. Mission invariant

**Memory is local until governed.** Drafts and raw memories remain isolated at their origin scope. A memory may cross agent, project, operator, or ecosystem boundaries only after librarian validation, explicit scope assignment or promotion, provenance retention, trust classification, and revocable append-only lineage.

This is the active operating mandate. The implementation remains workspace-local until this spec graduates and the code catches up.

Consequences:

- Agents never write shared canonical memory directly.
- Agent-native memory files are ingestion sources, not trusted stores.
- Cross-scope records are new promoted records, not raw imports.
- Promotion is reversible through append-only revocation or supersession edges.
- Runtime recall is scoped by explicit authorization and query shape.

## 2. Relationship to `workspace-model.md`

`workspace-model.md` remains authoritative for the shipped project-local store:

- workspace identity;
- workspace directory layout;
- local symbol table allocation;
- local canonical log partitioning;
- local read/write isolation.

This spec adds scopes above and beside workspaces:

- `agent_local`;
- `project`;
- `operator`;
- `ecosystem`;
- `quarantine`.

`project` is the successor name for the current workspace-level product concept. The implementation can continue using `WorkspaceId` internally while the broader scope model is drafted.

## 3. Scope taxonomy

| Scope | Meaning | Default ingress | Cross-scope visibility |
|---|---|---|---|
| `agent_local` | Memory useful only to one agent identity or surface | Agent native memory sweep | No direct visibility |
| `project` | Memory useful within one repo/workspace | Project draft, MCP write, repo-local status | Visible to that project |
| `operator` | Stable operator preferences and working rules | Operator-authored config, promoted project evidence | Visible to authorized projects/agents |
| `ecosystem` | Reusable, de-identified, high-confidence knowledge | Promoted records only | Visible to authorized projects/agents |
| `quarantine` | Unsafe, conflicting, unresolved, or failed drafts | Librarian rejection path | Review only |

Every canonical record has exactly one owning scope. A retrieval call can fan out across multiple scopes, but only by naming the allowed scopes.

Example:

```text
mimir_rehydrate(project = Mimir, scopes = [project, operator, ecosystem])
```

The retrieval engine combines authorized scope results after reading each physical partition. There is no implicit global memory search.

## 4. Drafts

All raw agent-originated content enters Mimir as a draft. Drafts are untrusted and not canonical.

Minimum draft fields:

```text
Draft {
  id,
  submitted_at,
  source_surface,
  source_agent,
  source_project,
  operator,
  raw_text,
  context_tags,
  provenance_uri,
}
```

Initial draft sources:

- Claude native memory sweep;
- Codex memory sweep;
- explicit MCP submission;
- explicit CLI submission;
- opted-in repo-local handoff/status docs.
- transparent harness capture from `mimir <agent> [agent args...]`.

Draft state machine:

```text
pending -> processing -> accepted | skipped | failed | quarantined
```

Only `accepted` drafts can produce canonical records. `quarantined` drafts are retained for review and cannot be promoted.

## 5. Librarian boundary

The librarian remains the single writer for every canonical scope.

The scope-aware librarian performs:

1. ingestion provenance capture;
2. sanitization;
3. observation/instruction separation;
4. memory type classification;
5. symbol binding inside the origin scope;
6. validation against grounding, confidence, and temporal rules;
7. deduplication and conflict detection;
8. scope assignment;
9. canonical emission;
10. promotion candidate creation when broader scope may be justified.

Agent-authored imperatives do not become durable instructions. They become instruction candidates and require the trust path in section 8.

## 6. Canonical partitions

The physical layout should preserve hard partitioning:

```text
~/.mimir/data/
  agents/<agent_id>/canonical.log
  projects/<workspace_id>/canonical.log
  operators/<operator_id>/canonical.log
  ecosystems/<ecosystem_id>/canonical.log
  quarantine/<scope_id>/canonical.log
```

This keeps the original anti-leak property: a missing predicate in a query cannot accidentally search every memory. Cross-scope retrieval is explicit fan-out over independent stores.

## 7. Promotion

Promotion creates a new canonical record in a broader scope. It does not move or mutate the source record.

Allowed promotion paths:

```text
agent_local -> project
project -> operator
project -> ecosystem
operator -> ecosystem
quarantine -> nowhere
```

Promotion requirements:

- source record has provenance;
- source record is not quarantined;
- target scope is explicit;
- transformation is recorded;
- source links are retained;
- trust tier is assigned;
- revocation/supersession edge can be emitted later.

Additional requirements by target:

| Target | Requirement |
|---|---|
| `project` | Same project or explicit operator approval |
| `operator` | Operator-authored source or repeated evidence plus operator approval |
| `ecosystem` | De-identification, conflict check, and operator approval |

No raw draft can promote directly to `operator` or `ecosystem`.

## 8. Trust tiers

Trust is structural metadata, not prose.

| Tier | Meaning |
|---|---|
| `raw` | Captured but not validated |
| `validated` | Passed librarian validation in origin scope |
| `accepted` | Committed to canonical storage in origin scope |
| `candidate` | Proposed for broader scope or durable instruction status |
| `promoted` | Accepted into broader scope |
| `revoked` | Superseded, corrected, or explicitly withdrawn |

Only `accepted` and `promoted` records are visible on normal recall paths. `raw`, `validated`, and `candidate` belong to ingestion/review surfaces. `revoked` remains inspectable through audit tools but is not treated as current memory.

## 9. Instruction extraction

Mimir separates memory from instructions.

Examples:

- "The build failed because `ld.bfd` segfaulted" is an observation.
- "Use `mold` for this repo" is a project procedure candidate.
- "Always verify locally before pushing" is an operator or project instruction candidate depending on provenance.

Instruction candidates require:

- source provenance;
- author classification;
- scope decision;
- conflict check against existing instructions;
- operator approval unless the source is already an operator-controlled configuration surface.

Agent-authored instructions default to `candidate`, never `promoted`.

## 10. Retrieval

Retrieval must be scoped and explicit.

Required retrieval modes:

- `rehydrate`: cold-start project/operator/ecosystem context;
- `recall`: targeted memory search;
- `review`: candidate/quarantine/conflict queues;
- `audit`: provenance, promotion lineage, revocation history.

Runtime output is data. Human-readable narration belongs in CLI, review UI, or docs, not in agent recall payloads.

## 11. Agent launch boundary

The expected product entry point is a transparent launch harness:

```text
mimir <agent> [agent args...]
```

The harness preserves the native agent interface while wrapping the session with scoped rehydration, sidecar recall/draft tools when available, and post-session or checkpoint capture. Arguments after `<agent>` are pass-through by default, so `mimir claude --r` behaves like `claude --r` inside a Mimir-governed session envelope.

First-run setup is not a separate prerequisite command. If Mimir is unconfigured, the harness enters bootstrap mode inside the requested agent session and lets that agent guide the operator through configuration.

MCP tools, hooks, and native client config are adapter conveniences. They can improve live recall or capture, but the scope and trust boundary remains the harness plus librarian path: retrieved memory is scoped data, and captured memory is an untrusted draft until accepted.

## 12. Quorum artifacts

Consensus quorum episodes are governed deliberation artifacts. They may produce memory candidates, instruction candidates, conflict reports, or promotion suggestions, but they do not directly commit memory.

When a quorum result enters the memory pipeline, it is a draft with:

- a provenance URI pointing to the quorum episode;
- source identity that distinguishes the quorum broker from participant agents;
- participant/model/persona metadata in the episode store;
- explicit consensus level and preserved dissent;
- target project/scope metadata when applicable.

The librarian decides whether quorum-derived drafts are accepted, skipped, failed, quarantined, or promoted. A strong quorum can raise confidence in a candidate; it cannot replace validation, approval, supersession checks, or revocation rules.

## 13. Safety invariants

1. **Single writer.** Every canonical write crosses the librarian.
2. **Append-only.** Promotion, revocation, and supersession emit new records or edges.
3. **Default isolation.** Raw memories are visible only at origin scope.
4. **Explicit fan-out.** Cross-scope reads list scopes intentionally.
5. **No direct shared writes.** Agents cannot append to operator or ecosystem canonical stores.
6. **No instruction laundering.** Agent-authored imperatives cannot become durable instructions without review.
7. **Mandatory provenance.** Records without provenance cannot promote.
8. **Quarantine containment.** Quarantined drafts never appear in normal recall.
9. **Structured runtime rendering.** Retrieved memory is rendered as data, not executable prompt prose.
10. **Quorum containment.** Quorum outputs are evidence artifacts until accepted through the normal draft/librarian path.

## 13. Open implementation questions

1. Does `operator` scope live only on one machine or sync across machines?
2. What concrete identifier names an operator on a local machine?
3. Are Claude and Codex separate `agent_local` scopes or source surfaces under one operator?
4. What is the first review UI: CLI, TUI, web, or MCP tool?
5. What is the minimum approval ceremony for project-to-operator promotion?
6. What local encryption is required for operator and ecosystem stores?
7. How does backup/restore compose across project, operator, and ecosystem partitions?
8. Which scope transitions need benchmark coverage before public claims?
9. Which quorum result fields are copied into draft metadata versus retained only in the quorum episode store?

## 14. Graduation criteria

This spec can graduate from draft when:

1. `ScopeId`, `ScopeKind`, and `TrustTier` compile in `mimir-core`.
2. Draft ingestion carries source surface, source agent, source project, operator, and provenance.
3. The librarian can commit to at least `project` and `operator` partitions without direct agent writes.
4. Retrieval requires explicit allowed scopes and has tests proving no implicit cross-scope leakage.
5. Promotion creates a new append-only canonical record with source lineage.
6. Revocation or supersession of a promoted record is query-visible.
7. Quarantine records are retained for review but excluded from normal recall.
8. Primary-source attribution for MCP, Hermes, CoAgents, and Synapse is recorded in `docs/attribution.md` where used for load-bearing comparisons.
9. Quorum-derived drafts can enter the draft store with provenance while normal recall excludes unaccepted quorum artifacts.
