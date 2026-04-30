# Mimir Documentation

This is the public documentation index for Mimir. Planning notes remain available for provenance, but the links below are the intended starting points for users and contributors.

## Start Here

- [`../README.md`](../README.md) - product overview, quickstart, and current public claims.
- [`../STATUS.md`](../STATUS.md) - current implementation, release, and launch state.
- [`first-run.md`](first-run.md) - fresh-clone walkthrough.
- [`bc-dr-restore.md`](bc-dr-restore.md) - Git-backed backup and restore flow.
- [`launch-readiness.md`](launch-readiness.md) - OSS readiness, engineering quality, and promise audit.
- [`blog/2026-04-28-agent-memory-compiler-pipeline.md`](blog/2026-04-28-agent-memory-compiler-pipeline.md) - public launch article.

## Architecture

- [`concepts/README.md`](concepts/README.md) - architecture spec index.
- [`concepts/librarian-pipeline.md`](concepts/librarian-pipeline.md) - librarian compiler pipeline.
- [`concepts/ir-write-surface.md`](concepts/ir-write-surface.md) - canonical write grammar.
- [`concepts/write-protocol.md`](concepts/write-protocol.md) - append-only write protocol.
- [`concepts/read-protocol.md`](concepts/read-protocol.md) - governed read protocol.
- [`concepts/scope-model.md`](concepts/scope-model.md) - scoped memory and governed promotion.
- [`concepts/consensus-quorum.md`](concepts/consensus-quorum.md) - quorum evidence model.

## Operations

- [`observability.md`](observability.md) - tracing events and privacy boundaries.
- [`sanitisation.md`](sanitisation.md) - draft sanitisation expectations.
- [`integrations/claude-code-hook.md`](integrations/claude-code-hook.md) - Claude hook integration.
- [`integrations/claude-desktop-config.md`](integrations/claude-desktop-config.md) - Claude Desktop MCP setup.

## Launch Execution

Historical planning notes are kept outside the public docs tree. Current release state is tracked in [`../STATUS.md`](../STATUS.md), readiness evidence in [`launch-readiness.md`](launch-readiness.md), and release mechanics in [`../RELEASING.md`](../RELEASING.md).
