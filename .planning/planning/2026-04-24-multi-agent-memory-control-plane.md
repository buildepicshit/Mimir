# Multi-agent memory control plane

> **Document type:** Planning - accepted mandate-expansion record. Production invariants are formalized in `AGENTS.md`; this document explains the product and implementation path.
> **Last updated:** 2026-04-24
> **Status:** Direction accepted for mandate update. This document records the scope expansion that is formalized in [`../concepts/scope-model.md`](../concepts/scope-model.md) and [`../concepts/consensus-quorum.md`](../concepts/consensus-quorum.md); implementation still requires staged spec and code work.
> **Cross-links:** [`../../AGENTS.md`](../../AGENTS.md) invariants #1, #7, and #8 | [`../concepts/workspace-model.md`](../concepts/workspace-model.md) | [`2026-04-20-mission-scope-and-recovery-benchmark.md`](2026-04-20-mission-scope-and-recovery-benchmark.md) | [`2026-04-21-rolls-royce-engineering-plan.md`](2026-04-21-rolls-royce-engineering-plan.md)

## TL;DR

Mimir should not try to become a Hermes-style general-purpose agent harness first. The stronger move is to become the memory governance layer underneath agent harnesses: a backend service that ingests raw memories from Claude, Codex, MCP clients, CLI sweeps, and future agent platforms; cleans and validates them through the librarian; routes them into scoped stores; exposes rehydration and recall surfaces that agents can use safely; and coordinates governed cross-agent deliberation when a problem benefits from multiple model/persona perspectives.

The proposed product shape is:

1. Agents keep their own native memory mechanisms.
2. Mimir imports those memories as untrusted drafts.
3. The librarian separates observations from instructions, validates structure, deduplicates, detects conflict, and emits canonical records.
4. A scope router files records as agent-local, project, operator, ecosystem, or quarantine.
5. Cross-project/ecosystem recall is allowed only through explicit promotion with provenance, auditability, and revocation.
6. A quorum broker can ask Claude, Codex, and future adapters to deliberate, critique, vote, and emit a structured result whose memory candidates still enter through the draft/librarian path.

The primary user entry point is a transparent agent harness:

```bash
mimir claude --r
mimir codex
mimir copilot --resume
```

The command should preserve the native agent experience while wrapping the session with Mimir bootstrap, rehydration, capture, and governance. There is no required separate `mimir setup` ceremony; first-run setup happens inside the requested agent session. See [`2026-04-24-transparent-agent-harness.md`](2026-04-24-transparent-agent-harness.md).

The replacement invariant is:

> Raw workspace memories are isolated by default. Cross-project or ecosystem memory exists only through explicit librarian promotion, with provenance, scope, trust tier, and revocation.

That is a deliberate replacement for the previous absolute workspace-isolation rule, not an implementation detail.

## Why this is a real scope change

The current authoritative model says workspaces never share memories, never read from each other, never import from each other, and never coordinate. That rule was correct for the original contamination-prevention thesis.

The new target is broader: a multi-agent ecosystem where Claude, Codex, and later other agents can contribute memories to a common backend while avoiding memory rot, prompt injection, stale project assumptions, and instruction/memory mixing. That cannot be implemented as a small extension to the current workspace model. It needs a new scope model that preserves the safety intent of isolation while adding controlled promotion paths.

This document is the planning bridge behind the 2026-04-24 mandate update in `AGENTS.md`, the draft `scope-model.md`, and the draft `consensus-quorum.md`.

## External patterns reviewed

These notes are based on primary project sources checked on 2026-04-24.

### Hermes Agent

[Hermes Agent](https://github.com/NousResearch/hermes-agent) is relevant because it is a persistent personal agent surface, not just a chat wrapper. Its README describes multi-platform gateways, model-provider choice, agent-curated memory, skills, session search, scheduled automation, subagents, and terminal/cloud execution surfaces.

Takeaway for Mimir: Hermes is a useful product-shape reference, but competing directly with a full agent harness would dilute Mimir's core advantage. The useful idea is not "build all of Hermes"; it is "memory must live behind persistent agent surfaces and become more useful over time."

### CopilotKit / CoAgents

[CopilotKit](https://github.com/CopilotKit/CopilotKit) and its CoAgents material are relevant for application state and human oversight. The current public material emphasizes generative UI, shared state between app and agent, and human-in-the-loop flows. The `useAgent`/`useCoAgent` model makes agent state inspectable and steerable from an app.

Takeaway for Mimir: CoAgents points toward the operator review/workbench layer. Mimir's review queue, promotions, conflicts, and instruction extraction should be visible and steerable, not hidden inside batch jobs.

### Synapse Protocol

[Synapse](https://synapse.md/) is the closest public prior-art shape for multi-agent shared memory. It describes append-only entries, namespaces, priority delivery, role-based authority, consolidation, and git-native audit trails.

Takeaway for Mimir: Synapse validates the need for shared memory primitives, but Mimir should stay stricter: raw entries are drafts, not durable truth; human-readable markdown is an ingestion or observability surface, not canonical storage; promotion is a compiler/librarian operation, not just a namespace write.

### Model Context Protocol

The [MCP specification](https://modelcontextprotocol.io/specification/draft) defines hosts, clients, servers, resources, prompts, tools, and capability negotiation. Its [tools spec](https://modelcontextprotocol.io/specification/draft/server/tools) treats tools as model-invocable and explicitly calls out human review and trust/safety for tool invocation.

Takeaway for Mimir: MCP is a useful connector for agent surfaces, but it is not itself a memory governance model or the required adoption path. Mimir should expose MCP tools for draft submission, recall, review, promotion, and status where a client supports them, while keeping the authority model inside Mimir and the first-class product boundary at the launch/session harness.

## Product thesis

Mimir becomes the memory control plane for the studio's agent ecosystem.

It should answer six operational problems:

1. **Persistence:** memory survives agent sessions, context compaction, machine moves, and native-client memory bugs.
2. **Cleanliness:** raw prose memories get normalized, deduplicated, superseded, and retired over time.
3. **Instruction separation:** facts, observations, operator preferences, and durable instructions are stored and rendered as different things.
4. **Scoped sharing:** agents can access the right memory at the right scope without cross-project contamination.
5. **Governance:** every accepted memory has provenance, trust tier, scope, and revocation semantics.
6. **Deliberation:** hard questions can be sent to a governed cross-agent, cross-model quorum with explicit personas, preserved dissent, and auditable synthesis.

The system does not need to run in real time. Scheduled sweeps and checkpoint-triggered ingestion are acceptable. The hot path is session startup and explicit recall, not every token.

## Proposed architecture

### 1. Draft ingestion

All agent-originated content enters as a draft. Drafts are untrusted.

Initial ingestion sources:

- Claude native memory sweep.
- Codex memory sweep from `$CODEX_HOME/memories`.
- Explicit MCP submission.
- Explicit CLI submission.
- Repo-local handoff docs or status files, when opted in.
- Session-wrapper capture from `mimir <agent> [agent args...]`.

Future ingestion sources:

- Hermes-style persistent agent exports.
- CoAgents/app-state snapshots.
- Other MCP memory services.
- Hosted or local agent harnesses.

Draft schema should stay small:

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

### 2. Librarian service

The librarian remains the single writer. It receives drafts and produces validated canonical records.

Responsibilities:

- Sanitize raw prose.
- Separate observation from instruction.
- Classify memory type.
- Bind symbols.
- Detect duplicates and conflicts.
- Detect supersession.
- Validate confidence and source bounds.
- Emit canonical records.
- Route failed or suspicious drafts to quarantine.

Instruction extraction is a first-class concern. Agent-written statements like "we should always do X" are not automatically durable instructions. They become instruction candidates. Operator-authored instructions can enter with higher trust, but still need structure and scope.

### 3. Scope router

The scope router decides where a validated record lives.

Proposed scopes:

| Scope | Meaning | Default write source |
|---|---|---|
| `agent_local` | Useful only to one agent identity or surface | Raw agent self-memory |
| `project` | Useful inside one repo/workspace | Project-specific decisions and constraints |
| `operator` | Applies to the operator across projects | Stable operator preferences and workflow rules |
| `ecosystem` | Applies broadly across agents/projects | Promoted, de-identified, high-confidence knowledge |
| `quarantine` | Unsafe, conflicting, or unresolved | Suspicious drafts and failed validation |

The existing workspace model maps mostly to `project`. The proposed change is to add higher scopes without letting raw project memory leak upward by default.

### 4. Canonical storage

Keep the hard-partition instinct. Do not collapse everything into one shared table with a scope column and hope every query remembers to filter correctly.

Candidate physical layout:

```text
~/.mimir/data/
  projects/<workspace_id>/canonical.log
  agents/<agent_id>/canonical.log
  operators/<operator_id>/canonical.log
  ecosystem/<ecosystem_id>/canonical.log
  quarantine/<scope_id>/canonical.log
```

Cross-scope retrieval is then an explicit fan-out over allowed stores:

```text
recall(project = Mimir, scopes = [project, operator, ecosystem])
```

The retrieval engine combines results after scope authorization, not before.

### 5. Promotion and distillation

Promotion is the mechanism that safely turns narrow memories into broader ones.

Rules:

- No draft promotes directly to `ecosystem`.
- `project -> operator` requires either explicit operator authorship or repeated evidence across projects.
- `project -> ecosystem` requires de-identification, conflict check, and approval.
- Instruction promotion requires human approval unless the source is already an operator-authored control file.
- Every promoted memory keeps links to source records.
- Every promoted memory can be superseded or revoked without deleting history.

Distillation can run asynchronously. A nightly job is enough for v1 of this direction.

### 6. Retrieval and rehydration

Agents need two retrieval modes:

- **Cold-start rehydration:** project state, operator rules, active decisions, recent feedback, and applicable procedures.
- **On-demand recall:** targeted lookup by current task, symbols, or memory kind.

The agent-facing surface should be compact and structured. Human-readable summaries belong in CLI or review UI, not in the runtime payload.

Example MCP tools:

```text
mimir_submit_draft
mimir_rehydrate
mimir_recall
mimir_review_queue
mimir_promote
mimir_revoke
mimir_scope_status
mimir_quorum_create
mimir_quorum_status
mimir_quorum_result
```

### 7. Review and governance

The control plane needs a review surface, even if the first version is CLI-only.

Review objects:

- Quarantined drafts.
- Instruction candidates.
- Cross-scope promotion candidates.
- Conflicts between current memories.
- Revocation candidates.
- Low-confidence or stale records.

Governance metadata:

- Source agent and surface.
- Project/workspace.
- Operator identity.
- Source URI or file path.
- Ingestion run ID.
- Trust tier.
- Scope.
- Promotion lineage.
- Supersession and revocation edges.

### 8. Consensus quorum

Consensus quorum is the controlled version of cross-agent discussion. It lets a requester ask Claude, Codex, and future adapters to reason over the same problem from explicit personas, then critique each other and produce a structured synthesis.

The minimum useful protocol is:

```text
request
  -> independent first pass
  -> critique round
  -> revision / hold-position round
  -> vote round
  -> synthesis with dissent
  -> draft submission or archive
```

Quorum outputs are not canonical memory. They are deliberation artifacts that can create memory drafts, instruction candidates, conflict reports, or promotion suggestions.

First personas:

| Persona | Role |
|---|---|
| `architect` | System invariants and long-term shape |
| `implementation_engineer` | Concrete code path and testability |
| `skeptic` | Failure modes and weak assumptions |
| `research_verifier` | Source quality and claim boundaries |
| `product_operator` | Workflow usefulness and review burden |

Safety rules:

- No participant writes canonical memory.
- No majority vote is treated as truth.
- Dissent is preserved in the result.
- One model playing many personas is not reported as cross-model agreement.
- Any proposed memory enters the normal draft/librarian path with provenance.

## Trust model

Trust should be explicit and structural.

Proposed tiers:

| Tier | Meaning |
|---|---|
| `raw` | Captured but not validated |
| `validated` | Passed librarian validation inside original scope |
| `accepted` | Committed to canonical store for original scope |
| `candidate` | Proposed for broader scope or instruction status |
| `promoted` | Accepted into broader scope |
| `revoked` | Superseded, corrected, or explicitly withdrawn |

Agent-authored content starts as `raw`. It can become `accepted` after validation. It cannot become durable operator instruction or ecosystem memory without a promotion path.

## Safety rules

1. **Single writer remains non-negotiable.** All canonical writes still go through the librarian.
2. **Append-only remains non-negotiable.** Correction and revocation are edges, not overwrites.
3. **Raw cross-project sharing is forbidden.** Only promoted records cross scope.
4. **Instructions are not observations.** Instruction extraction creates candidates and requires tighter trust rules.
5. **Runtime output is data.** Retrieved memories must be rendered as data records, not prose instructions.
6. **Quarantine is a first-class destination.** Failed memories are retained for review, not silently discarded.
7. **Provenance is mandatory.** A memory without provenance cannot be promoted.
8. **Quorum is evidence, not truth.** A consensus result can propose memory or action, but it does not bypass librarian validation or operator approval.

## Implementation path

This should land in stages.

### Stage 0 - Mandate and spec reset

- Record that the new invariant replaces absolute workspace isolation in the operating mandate.
- Write a formal `scope-model.md` draft.
- Write a formal `consensus-quorum.md` draft.
- Mark `workspace-model.md` as still authoritative for v1 local project stores, but superseded by `scope-model.md` for cross-scope promotion.

### Stage 1 - Drafts v2

- Generalize drafts beyond Claude.
- Add `source_surface`, `source_agent`, `source_project`, and `operator`.
- Add Codex memory sweep.
- Keep all drafts project-local until routed.

### Stage 2 - Production librarian

- Finish the pre-emit validator and bounded retry loop.
- Add instruction/observation separation as a typed output.
- Add quarantine and review records.

### Stage 3 - Scope router

- Add scope metadata and allowed-scope fan-out.
- Preserve hard physical partitions.
- Add project + operator retrieval first.

### Stage 4 - Promotion queue

- Implement promotion candidates.
- Require approval for operator/ecosystem scope and all instruction candidates.
- Add supersession/revocation edges for promoted memories.

### Stage 5 - Transparent harness and agent adapters

- Transparent `mimir <agent> [agent args...]` harness with pass-through child args and PTY supervision.
- First-run bootstrap inside the requested agent session, not a separate setup command.
- Claude and Codex adapter profiles first.
- Codex/Claude native memory import/export discipline.
- MCP tools, hooks, and native config only as optional adapter conveniences.
- Optional REST API only if a concrete harness needs it.

### Stage 6 - Consensus quorum

- Define quorum episode/result envelopes.
- Add CLI-backed create/append/synthesize flow.
- Wire Claude and Codex adapter participation through recorded outputs first.
- Submit quorum memory candidates into the draft store, not the canonical store.

### Stage 7 - Review UI

- Start with CLI.
- Add TUI or web workbench only when review volume justifies it.
- CoAgents-style state visibility is the reference pattern for this layer, not for canonical storage.

## What not to build first

- Do not build a full Hermes competitor before the memory control plane works.
- Do not create real-time multi-agent chat or task orchestration before governed memory intake works.
- Do not let agents write directly into shared memory namespaces.
- Do not make markdown the canonical format for shared memory.
- Do not promote memories across projects by default.
- Do not treat an agent's self-written preferences as operator instructions.
- Do not treat a quorum majority as truth or erase minority objections.
- Do not make a separate setup command the adoption path; setup should be an agent-guided first-run state inside `mimir <agent>`.
- Do not make MCP or native plugin configuration the required foundation; the launch harness is the first integration boundary.

## Open questions

1. What is the exact operator identity model on a single-user machine?
2. Should `operator` scope be local-only, syncable, or both?
3. How much human approval is required for project-to-operator promotion?
4. Does ecosystem memory mean "BES Studios only" or "public reusable knowledge"?
5. Should Codex and Claude have separate `agent_local` stores or share one per surface?
6. What is the minimum review UI that the operator will actually use?
7. How should native agent memories be written back, if at all?
8. What gets encrypted locally by default?
9. What is the backup/restore story for operator and ecosystem stores?
10. Which local adapter contract can ask Claude and Codex to participate in quorum episodes without brittle UI automation?

## Near-term recommendation

The next concrete engineering move is not a broad rewrite. It is:

1. Keep `scope-model.md` moving from draft to implementable spec.
2. Keep `consensus-quorum.md` aligned with the same draft/librarian boundary.
3. Keep the transparent harness direction recorded in [`2026-04-24-transparent-agent-harness.md`](2026-04-24-transparent-agent-harness.md), but do not let it distract from memory correctness.
4. Resume Category 1 from the Rolls Royce plan with the new draft schema in mind.
5. Add Codex and Claude ingestion as the first two real input surfaces.

That gives Mimir a path from the current solid core to a multi-agent memory ecosystem without losing the compiler/librarian discipline that makes it worth building.
