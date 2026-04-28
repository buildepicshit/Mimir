# Render-surface audit (Delivery Plan D.1)

> **Document type:** Planning — agent-native render-surface audit + decision log.
> **Last updated:** 2026-04-20
> **Status:** Scouting complete. Recommendations proposed; actual tightening lands as follow-up PRs after the current benchmark/harness priorities; Actions capacity was restored on 2026-04-27 but quota discipline still applies.
> **Cross-links:** [`2026-04-20-mission-scope-and-recovery-benchmark.md`](2026-04-20-mission-scope-and-recovery-benchmark.md) § 7 · [`2026-04-20-delivery-plan.md`](2026-04-20-delivery-plan.md) § D.1 · [`../../AGENTS.md`](../../AGENTS.md)

## Purpose

Delivery Plan item **D.1** requires every agent-facing render site to be inspected for agent-native-ness and a decision logged on whether to tighten the current form. This document is that inspection + decision log.

Agent-native = token-dense, structured, no narrative filler, no whitespace-for-human-eyes. Human-readable is confined to the observability surface (`mimir-cli log / decode / symbols / verify` outputs, STATUS.md, docs, debug dumps). The two surfaces are not the same; prose on the runtime surface is a sanitisation risk and a token tax. See the scope doc § 7.

## Scope

**In scope:** the 9 MCP tool response shapes exposed by `mimir-mcp` (listed in [`crates/mimir-mcp/src/lib.rs`](../../crates/mimir-mcp/src/lib.rs)) — these are what flow into an agent's context window at runtime.

**Out of scope:** `mimir-cli` subcommand output, `STATUS.md`, `CHANGELOG.md`, doc prose, tracing-span output, panic messages. These are observability — humans are the consumer.

## Methodology

For each of the 9 tools, record:

1. **Response shape** — Rust struct + field types.
2. **Content source** — where the content originates (canonical log / replay / in-memory state / hardcoded).
3. **Format** — JSON structure, Lisp rendering, or plain text.
4. **Agent-native score** — a rough 1–10 judgement of token-density + structure vs. narrative / whitespace overhead.
5. **File:line citations** for the handler, response type, and renderer.

Scoring is subjective — a tool graded 9/10 has no obvious runtime-surface waste; 6–7/10 has targeted tightening opportunities; <6 would be a design problem (none observed).

## Inventory

| Tool | Response shape | Format | Agent-native | Tightening candidate? |
|---|---|---|---|---|
| `mimir_status` | `StatusReport` (7 fields) | JSON (pretty) | 7/10 | Yes — verbose tool description |
| `mimir_read` | `ReadResponse` (6 fields, `Vec<String>` Lisp records) | JSON (pretty) + compact Lisp per record | 8/10 | Minor — JSON pretty-print whitespace |
| `mimir_verify` | `VerifyReportJson` (7 fields) | JSON (pretty) | 6/10 | Yes — narrative embedded in `tail_status` |
| `mimir_list_episodes` | `Vec<EpisodeRow>` (3 fields each) | JSON (pretty) | 9/10 | No |
| `mimir_render_memory` | plain text (single Lisp S-expr) | Compact Lisp, no wrapper | 9/10 | No |
| `mimir_open_workspace` | `OpenWorkspaceResponse` (4 fields) | JSON (pretty) | 8/10 | Minor — pretty-print |
| `mimir_write` | `WriteResponse` (2 fields) | JSON (pretty) | 9/10 | No |
| `mimir_close_episode` | `WriteResponse` (2 fields, same as write) | JSON (pretty) | 9/10 | No |
| `mimir_release_workspace` | `ReleaseWorkspaceResponse` (1 field) | JSON (pretty) | 9/10 | No |

**Summary:** 7 of 9 tools score ≥ 8/10. Two tools (`mimir_status`, `mimir_verify`) and three cross-cutting patterns account for essentially all of the tightening opportunity.

## Detailed findings (non-9/10 tools only)

### `mimir_status` — 7/10

The response body is fine — `StatusReport` is seven scalar fields ([`crates/mimir-mcp/src/server.rs:123–148`](../../crates/mimir-mcp/src/server.rs#L123)). The friction is on the **tool-invocation surface**: the `#[tool(description = "...")]` macro carries ~250 characters of narrative prose about what the fields mean ([`crates/mimir-mcp/src/server.rs:415–416`](../../crates/mimir-mcp/src/server.rs#L415)). Tool descriptions flow into the agent's system prompt at MCP-handshake time; per-invocation this is amortized, but on long sessions (or repeated cold-starts, which is our BC/DR scenario) the description text is paying a recurring tax. Better: a sub-50-char description + detail in the JSON Schema `description` fields per input parameter, which most clients surface lazily only when the agent asks.

### `mimir_verify` — 6/10

Six of the seven fields are integers. The seventh, `tail_status`, is a `String` that encodes a three-state enum plus a narrative error message in the corrupt case ([`crates/mimir-mcp/src/server.rs:883–922`](../../crates/mimir-mcp/src/server.rs#L883), [`crates/mimir-cli/src/lib.rs:305–369`](../../crates/mimir-cli/src/lib.rs#L305)). On corrupted logs the narrative can run 200+ chars (full `DecodeError` Display expansion). That narrative belongs on the observability surface (the `mimir-cli verify` text output), not in the runtime response.

Recommended shape:

```
tail_type:  "clean" | "orphan_tail" | "corrupt"
tail_bytes: u64                        // already present; rename from trailing_bytes
tail_error: Option<String>             // only populated when tail_type == "corrupt"; short code, not prose
```

An agent wanting the full error can call an observability endpoint or read the CLI output; agents on the happy path pay zero narrative tokens.

## Cross-cutting tightening opportunities

Three patterns are not tool-specific but apply across the surface:

### C.1 — Narrative error strings in validation responses

Several error returns are human-reader-friendly prose, 90–115 chars each: e.g. `"no_workspace_open: no workspace is open; call mimir_open_workspace or restart the server with MIMIR_WORKSPACE_PATH set"` ([`crates/mimir-mcp/src/server.rs:828–873`](../../crates/mimir-mcp/src/server.rs#L828)). Every failed tool call pays this tax. The error *code* prefix is the load-bearing part; the explanatory prose is documentation.

**Recommended:** keep short, stable error codes (`no_workspace_open`, `lease_expired`, `lease_token_mismatch`, etc.) and move explanatory prose to the tool-level docs / a single reference section in the MCP server instructions. Agents that need recovery guidance can be briefed in their system prompt once, not per-error.

### C.2 — Tool-description verbosity

Each `#[tool(description = "...")]` carries 200–500 chars of prose ([`crates/mimir-mcp/src/server.rs:415, 452, 502, 531, 578, 617, 711, 747, 789`](../../crates/mimir-mcp/src/server.rs#L415)). `mimir_read` is the worst (~450 chars listing every query predicate inline). Descriptions sit in the agent's tool registry for the session's lifetime.

**Recommended:** compress each to ≤100 chars ("what does this tool do, in one sentence") and move syntactic detail to input-schema field descriptions and a single authoritative doc link (e.g., `docs/concepts/read-protocol.md` for `mimir_read`). Agents fetch detail lazily when needed, not ambiently.

### C.3 — JSON pretty-printing on the runtime surface

All JSON responses go through `serde_json::to_string_pretty()` ([`crates/mimir-mcp/src/server.rs:945`](../../crates/mimir-mcp/src/server.rs#L945)). Pretty-printing adds 2–3 bytes of whitespace per field (newlines + indentation). For large `mimir_read` results the cumulative overhead is non-trivial — a 100-record batch can carry ~200 bytes of pure whitespace.

**Recommended:** switch to `serde_json::to_string()` (compact) for all MCP tool responses. Agents parse identical bytes either way; human debugging goes through the observability surface. **Caveat:** confirm no supported MCP surface depends on pretty-printed JSON for line-based logging. Quick one-line audit before the change.

## Decision log

For each opportunity, the current position:

| # | Opportunity | Decision | Rationale |
|---|---|---|---|
| `mimir_status` | Shrink tool description | **Accept tightening** (follow-up PR) | Amortized cost; low risk; ≤ 100 chars is enforceable via a lint. |
| `mimir_verify` | Split narrative out of `tail_status` | **Accept tightening** (follow-up PR) | Touches a response shape and is therefore a wire-compat change — land before `v0.1.0-alpha.1` (Deliverable D.6) so no alpha consumers drift. |
| C.1 | Short error codes, prose → docs | **Accept tightening** (follow-up PR) | Error codes are already the stable contract; prose was convenience. |
| C.2 | Tool-description compression | **Accept tightening** (follow-up PR) | Largest per-session token saving; trivial diff. |
| C.3 | Compact JSON | **Accept pending quick compat audit** | Low risk for the supported MCP surface, but worth a grep across tests + known client patterns first. |
| Lisp rendering (`mimir_read`, `mimir_render_memory`) | Tighten further? | **Current form stays** | Already compact Lisp (not pretty-printed). Any tighter would either change the wire surface (breaks round-trip) or compress whitespace in ways the parser already tolerates but that cost comprehensibility. |

Net decision: the runtime surface is largely agent-native already. The audit identifies six concrete tightening opportunities with a conservative estimated token saving of ~300–500 tokens per session (≈ 3–5% of a 10k-token session). All six are suitable for a single follow-up engineering PR once CI capacity returns.

## Out of scope

- **Lisp rendering redesign.** Whether to replace Lisp with a denser binary / structural form for the runtime surface is a separate wire-surface spec decision and out of scope for D.1. The current Lisp rendering is already compact (single-line per record) and round-trips through the parser.
- **Recovery-digest format.** If a future recovery-digest tool lands (see scope doc § 4), its render surface gets audited at that time. Not retrospective work.
- **Observability surface.** The `mimir-cli` inspector outputs, STATUS.md rendering, and debug dumps keep their human-readable form intentionally. This audit does not touch them.

## Follow-up tracking

The six tightening items (`mimir_status` description, `mimir_verify` tail split, C.1 error codes, C.2 tool-description compression, C.3 compact JSON, and the C.3 compat audit itself) belong in a single follow-up engineering PR, sequenced after CI capacity returns. Tracked under Delivery Plan item D.1 completion; [`2026-04-20-delivery-plan.md`](2026-04-20-delivery-plan.md) will tick its D.1 checkbox when that PR lands.
