# Agent Memory Should Be Governed Like A Compiler Pipeline

Mimir is public today as an experimental local-first memory governance system for AI agents.

The project starts from a practical failure mode: agent sessions often end with useful context scattered across transcripts, scratch notes, repo-local files, and native client memory. On the next run, the agent either starts cold, reloads too much untrusted prose, or mixes facts from one project into another. That is not a memory system. It is a pile of notes with unclear authority.

Mimir's thesis is simple: agents may propose memory, but trusted shared memory should be committed through a governed write path.

## The Librarian Boundary

Mimir treats incoming memories as drafts. Checkpoints, native agent memories, session exports, and adapter submissions enter as untrusted evidence. The librarian owns the canonical write boundary: it validates candidate records, applies deterministic checks, detects duplicates and conflicts, preserves provenance, and commits accepted records into an append-only log.

That boundary is the product. It keeps a raw agent-authored note from becoming a durable instruction just because it appeared in a transcript. It also keeps memory isolated by scope until it has enough provenance and governance to move elsewhere.

## Why A Compiler Pipeline

The architecture borrows from compiler design because the failure modes look familiar:

- syntax matters because malformed memory should not commit;
- binding matters because names need stable identity;
- semantic checks matter because later records can supersede earlier ones;
- emit matters because the canonical store should be append-only and replayable;
- diagnostics matter because operators need to understand what was accepted, rejected, deferred, or revoked.

Mimir is not trying to make memory more human-readable at the storage layer. The canonical store is agent-oriented. Human operability comes from tools that inspect, explain, and verify the governed log without making the log itself a notes folder.

## The Transparent Harness

The main user surface is intended to stay boring:

```bash
mimir codex
mimir claude
```

The harness preserves the native agent terminal flow while adding project bootstrap, bounded context rehydration, checkpoint capture, native-memory sweep, and post-session librarian handoff. The agent keeps working in the tool it already uses; Mimir owns the durable memory boundary around that session.

## What Works Today

The public repo currently includes:

- a Rust append-only core with replay, verification, symbol tracking, supersession, and confidence decay;
- a librarian path for draft ingestion, duplicate filtering, conflict handling, raw archival, and governed commit;
- a transparent harness for wrapped agent sessions;
- operator commands for status, doctor checks, bounded context, draft triage, and read-only memory inspection;
- a local stdio MCP surface;
- Git-backed recovery mirroring and restore drills;
- recovery benchmark scaffolding and launch contracts;
- a coherent Codex plugin bundle under `plugins/mimir`.

The project has been dogfooded locally during its public-launch cleanup, including session checkpoints, wrapper health checks, and post-session draft capture.

## What We Are Not Claiming Yet

Mimir is pre-1.0. The implementation is real, but storage details, CLI flags, draft envelopes, MCP schemas, and adapter workflows may change before a stable release.

We are not claiming production readiness, hosted service availability, stable APIs, or benchmark-proven superiority over other memory systems. Public benchmark claims wait for recorded live runs.

## What We Want Reviewed

The useful review lanes are concrete:

- Rust correctness and append-only log integrity;
- security boundaries around untrusted memory text;
- librarian validation and conflict behavior;
- agent adapter UX for Codex, Claude, and future local clients;
- recovery benchmark methodology;
- documentation clarity for first-run users.

The repo is open at <https://github.com/buildepicshit/Mimir>.
