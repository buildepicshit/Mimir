# Memory Product Gap Scan - 2026-04-28

This scan compares Mimir against current agent-memory products and frameworks. It is not a claim that Mimir should copy their trust model. The useful question is: which product capabilities would strengthen Mimir while preserving the librarian boundary, append-only lineage, scoped promotion, and data-only retrieval rules?

## Sources

- Letta / MemGPT memory docs: <https://docs.letta.com/guides/agents/memory>, <https://docs.letta.com/guides/agents/architectures/memgpt>
- LangGraph / LangChain memory docs: <https://docs.langchain.com/oss/javascript/langgraph/memory>, <https://docs.langchain.com/oss/python/langchain/long-term-memory>
- Mem0 docs: <https://docs.mem0.ai/core-concepts/memory-types>, <https://docs.mem0.ai/core-concepts/memory-operations/add>, <https://docs.mem0.ai/core-concepts/memory-operations/search>, <https://docs.mem0.ai/open-source/features/graph-memory>
- Zep / Graphiti docs: <https://help.getzep.com/concepts>, <https://www.getzep.com/product/open-source>
- Supermemory MCP docs: <https://supermemory.ai/docs/supermemory-mcp/mcp>
- Pieces Long-Term Memory: <https://pieces.app/features/long-term-memory>
- Engram product page: <https://www.engram.fyi/>

## Feature Gaps Worth Pulling Into Mimir

| Priority | External pattern | Why it matters | Mimir-compatible version |
|---|---|---|---|
| P0 | Operator memory control surface: browse, tag, edit, forget/delete, visibility controls, access logs. Mem0, OpenMemory, Supermemory, and Pieces all emphasize user-visible management. | Public users need confidence that memory is inspectable and controllable. Mimir now has draft triage, decoder tools, and the first operator memory CLI, but no polished operator memory console. | Initial `mimir memory list|show|explain|revoke` shipped. Follow-up: add tags/visibility/access logs plus an audit-oriented TUI or static HTML report. "Delete" remains append-only revocation/tombstone, not physical mutation. |
| P0 | One-command client onboarding across Claude, Cursor, VS Code, Windsurf, etc. Mem0 MCP and Supermemory lead with fast MCP setup. | Mimir's setup is correct but still feels like an engineer's toolkit. Adoption will suffer if external operators cannot get to first memory in minutes. | Initial `mimir setup-agent doctor` shipped for Claude/Codex setup readiness, exact commands, and next action. Follow-up: broaden recipes to Cursor, VS Code, Windsurf, and MCP-client install verification while keeping writes explicit and inspectable. |
| P0 | Context assembly/profile prompt. Zep has a context block; Supermemory has a `/context` prompt; Letta has always-visible memory blocks. | Cold starts should get a compact, predictable "what Mimir knows now" surface without dumping raw memory. | Shipped initial `mimir context`: bounded, source-marked governed records with the data-only boundary, setup/health metadata, pending-draft counts, and metadata-only untrusted supplements. Follow-up: richer conflict/open-work sections and scoped operator rules. |
| P1 | Relationship/graph retrieval. Mem0 Graph Memory and Zep/Graphiti return entity/relationship context, not only vector hits. | Mimir has stable symbols and supersession DAGs, but retrieval is still record/query-shaped. Agents benefit from "who/what/when connected to this?" answers. | Add a relationship read API over symbol links, supersession edges, and inferred relations: `mimir graph neighbors`, `mimir graph timeline`, and MCP equivalents. Do not add opaque graph writes outside the librarian. |
| P1 | Hybrid retrieval with filters, thresholds, reranking, and metadata scopes. Mem0 search exposes metadata filters, top-k, thresholds, and optional rerankers. | Precision matters more than recall spam. Mimir's query language is deterministic but not yet ergonomic for ranked recall. | Add retrieval plans that combine deterministic scope filters, recency/confidence windows, and optional local rerankers. Rerankers may order candidates but must not author canonical facts. |
| P1 | Temporal fact invalidation surfaced directly to consumers. Zep context includes valid/invalid dates; Graphiti emphasizes changing facts and historical context. | Mimir already has bi-temporal clocks and supersession. The gap is product UX: users and agents should see why something is current or stale. | Add `mimir explain <memory-id>` and context output that names superseding records, invalidation edge kind, valid/invalid times, and confidence decay state. |
| P1 | Memory layers by lifetime/scope: conversation/session/user/org/project. Mem0 and LangGraph make this explicit. | Mimir's scope model is strong in design, but promotion mechanics and retrieval fan-out are not yet implemented. | Finish governed promotion/revocation with project/operator/ecosystem tiers and retrieval policies that explicitly state which scopes were searched. |
| P1 | Hot-path vs background memory updates. LangGraph distinguishes updates during the conversation from background/asynchronous memory creation. | Mimir's checkpoint/capture path is safe, but agents need a fast way to propose memory without derailing the task. | Keep checkpoint drafts as hot-path proposals; add a background `mimir capture summarize`/watch mode for transcript or session capsules that proposes drafts after compaction or exit. |
| P1 | Automatic consolidation into higher-level insights. Letta, Mem0, and Engram all market consolidation or self-editing memory. | Mimir has inferential memory types, but productized consolidation policies are still thin. | Add librarian-owned consolidation jobs that propose Inferential records with provenance, review artifacts, dissent/conflict handling, and deterministic validation. |
| P2 | Broad connector ingestion. Pieces captures OS/app context; Supermemory offers content extraction/connectors; Zep ingests messages, JSON, text, and business data. | Mimir is agent-first, but real memory often lives in issue trackers, PRs, chats, docs, and editor history. | Add read-only adapters for GitHub issues/PRs, git history, local editor state, and selected docs. Every connector output enters as untrusted recall or draft evidence. |
| P2 | Time-based workstream timeline. Pieces lets users ask "what was I doing when?" and browse an activity timeline. | Mimir's recovery thesis would be easier to prove with timeline UX. | Add `mimir timeline` over episodes, captures, checkpoints, draft lifecycle events, remote syncs, and setup changes. |
| P2 | Managed service / sync plane. Mem0, Zep, and Supermemory sell hosted persistence and cross-client availability. | OSS local-first is a strength, but multi-machine recovery still needs a polished sync story. | Finish the `remote.kind = "service"` adapter contract only after Git-backed recovery remains stable. Preserve local-first and append-only semantics. |
| P2 | Public benchmark report. Zep, Supermemory, Mem0, and Engram all lean on benchmark evidence. | Mimir should not make benchmark claims until it has transcript-backed results. | Complete the live recovery benchmark with committed transcripts, scoring, and caveats before any performance claim. |

## Immediate Backlog Candidates

1. Audit/report surface over `mimir memory` with tags, visibility, access logs, and tombstone history.
2. Broader client setup recipes for external operators.
3. Relationship/timeline read APIs over existing symbol and supersession data.
4. Richer `mimir context` sections for conflicts, open work, and scoped operator rules.
5. Connector backlog: GitHub PR/issues first, because launch contributors will live there.

## Guardrail

Do not adopt competitor features that let agents silently mutate trusted memory. In Mimir, "self-editing memory" must mean "agent proposes a draft or scratch context update; the librarian governs canonical persistence."
