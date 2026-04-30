---
project: Mimir
phase: post-v0.1.0 alpha cleanup
last_updated: 2026-04-30
version: 0.1.0
release_tags: v0.1.0
ci_state: "main green on 2026-04-28 after PR #11; current branch verification is recorded in PR evidence"
blockers: []
---

# Mimir Status

Mimir is an experimental local-first memory governance system for AI agents. It is public as pre-1.0 active development, not as a production service or stable API.

## Current State

| Area | Status |
|---|---|
| Core store | Append-only canonical log, replay, decoder verification, crash recovery, symbol identity, supersession, temporal model, and confidence decay are implemented. |
| Librarian | Draft ingestion, validation, duplicate filtering, conflict policy, raw archive mode, LLM-backed processing, observability, and workspace write locking are implemented. |
| Harness | `mimir <agent> [agent args...]` preserves native agent stdio/argv and adds bootstrap, context, checkpoint drafts, capture, native setup artifacts, and post-session handoff. |
| Operator tools | `mimir status`, `mimir health`, `mimir doctor`, `mimir context`, `mimir drafts ...`, and `mimir memory ...` are implemented. |
| Adapters | Claude and Codex setup paths are implemented; Copilot session-store recall is read-only and submits checkpoint drafts through the librarian draft path. |
| MCP | `mimir-mcp` exposes local governed memory tools over stdio MCP. |
| Recovery | Git-backed remote push/pull/drill flows verify append-only log integrity and copy draft JSON without mutating canonical history. |
| Benchmarks | Recovery benchmark fixtures and validation harness live under `benchmarks/recovery`; live benchmark claims are not made yet. |
| Release | Workspace version is `0.1.0`; tag `v0.1.0` points to commit `315d791`, and GitHub Release `v0.1.0` was published on 2026-04-28 with platform archives and checksums. |

## Architectural Boundaries

- Agents can propose memory drafts; they do not write trusted shared memory directly.
- The librarian is the canonical writer.
- Canonical memory is append-only; revocation and supersession are represented as new records or edges, not in-place mutation.
- Raw native memories remain untrusted evidence until processed or archived through the configured per-repo librarian policy.
- Cross-project, operator-level, or ecosystem-level reuse requires governed promotion with provenance and revocation semantics.
- Consensus quorum output is evidence, not automatic truth.

## Launch Work Order

1. Keep public documentation links aligned with files that exist in the repo.
2. Keep README and docs index cleanup batched with other public admin updates.
3. Keep OSS readiness, engineering quality, and promise-audit status current.
4. Keep future release communication and listing work separate from implementation status.
5. Verify locally before pushing: build, tests, fmt, clippy, targeted benchmark checks, docs/package checks where relevant.
6. Use one batched commit and one push after local green.

## Public Claims Allowed Now

- Mimir is an experimental memory governance/control-plane project for AI agents.
- It has a local append-only store, librarian-governed draft path, transparent harness, MCP surface, operator inspection tools, and recovery mirroring.
- It is designed around scoped memory, provenance, validation, and append-only lineage.

## Claims Not Allowed Yet

- Production-ready.
- Stable API, CLI, MCP schema, storage format, or wire format.
- Hosted service.
- Benchmark-proven recovery advantage.
- Direct agent memory writes into canonical storage.
- Automatic cross-project memory sharing without librarian-governed promotion.

## References

- [`README.md`](README.md) - public entry point.
- [`docs/README.md`](docs/README.md) - documentation index.
- [`docs/launch-readiness.md`](docs/launch-readiness.md) - current launch checklist.
- [`RELEASING.md`](RELEASING.md) - release runbook.
- [`docs/concepts/`](docs/concepts/) - architecture specs.
- [`PRINCIPLES.md`](PRINCIPLES.md) - engineering principles and architectural boundaries.
