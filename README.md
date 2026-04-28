# Mimir

Experimental local-first memory governance for AI agents.

Mimir is built around one rule: agents may propose memory, but they do not write trusted shared memory directly. Session notes, checkpoints, native agent memories, and adapter exports enter as untrusted drafts. The librarian validates, structures, deduplicates, and commits accepted records into an append-only canonical log with provenance.

The project is pre-1.0. The architecture and local implementation are real, but storage details, CLI flags, draft envelopes, and adapter workflows may change before the first stable release.

## What Works Today

| Area | Current state |
|---|---|
| Append-only store | Canonical log, replay, verification, crash recovery, symbol tracking, supersession, and confidence decay are implemented in `mimir-core`. |
| Librarian path | `mimir-librarian` ingests draft envelopes, validates candidate canonical records, filters duplicates, handles conflicts, and commits through the governed store path. |
| Agent harness | `mimir <agent> [agent args...]` preserves the native terminal flow for local agents while adding bootstrap, context, checkpoint, capture, and post-session handoff hooks. |
| Operator controls | `mimir status`, `mimir doctor`, `mimir context`, `mimir drafts ...`, and `mimir memory ...` expose setup, bounded context, draft triage, and read-only memory inspection. |
| MCP | `mimir-mcp` exposes governed local memory tools over stdio MCP. |
| Recovery | Git-backed `mimir remote status|push|pull|drill` mirrors append-only logs and draft JSON for local recovery. |
| Benchmarks | Recovery benchmark fixtures, launch contracts, transcript gates, and score validation live under `benchmarks/recovery`. Public benchmark claims wait for recorded live runs. |
| Codex | `plugins/mimir` is a coherent Codex plugin bundle for the Mimir workflow. It is not a standalone memory skill and does not bypass the librarian. |

## What Is Not Claimed Yet

- Production readiness.
- Stable storage, CLI, API, MCP schema, or wire-format compatibility.
- Hosted service availability.
- Benchmark-proven superiority over other memory systems.
- Direct agent writes into canonical memory.
- Cross-project or operator-wide memory promotion without librarian governance.

## Quickstart

```bash
git clone https://github.com/buildepicshit/Mimir.git
cd Mimir

cargo build --workspace
cargo test --workspace
cargo run -p mimir-harness -- doctor --project-root .
cargo run -p mimir-harness -- rustc --version
```

For a fresh-clone walkthrough, see [`docs/first-run.md`](docs/first-run.md).

## Running Mimir

Build the transparent harness:

```bash
cargo install --locked --path crates/mimir-harness
```

Inspect project readiness:

```bash
mimir doctor --project-root .
mimir status --project-root .
```

Wrap an agent session:

```bash
mimir codex
mimir claude
```

Record an intentional draft memory from a wrapped session:

```bash
mimir checkpoint --title "short title" "memory note"
```

Process captured drafts after a session with the configured per-repo librarian policy. The canonical write boundary stays the librarian path; raw native memories and checkpoint notes are draft evidence until accepted.

## Backup And Restore

Mimir's recovery path is explicit Git-backed BC/DR mirroring:

```bash
mimir remote status
mimir remote push --dry-run
mimir remote push
./scripts/bcdr-drill.sh --dry-run
```

`mimir remote push` mirrors the append-only workspace log and draft JSON files into the configured recovery repository. `mimir remote pull` restores missing or prefix-safe local state after verifying canonical-log integrity. See [`docs/bc-dr-restore.md`](docs/bc-dr-restore.md).

## Documentation

- [`docs/README.md`](docs/README.md) - public documentation index.
- [`STATUS.md`](STATUS.md) - current implementation and release state.
- [`AGENTS.md`](AGENTS.md) - architectural invariants and agent operating manual.
- [`docs/concepts/`](docs/concepts/) - architecture specs.
- [`docs/launch-readiness.md`](docs/launch-readiness.md) - OSS, engineering, and promise sign-off checklist.
- [`docs/launch-posting-plan.md`](docs/launch-posting-plan.md) - launch article, listing, and posting plan.

## Contributing

Useful review areas are Rust correctness, append-only log integrity, security boundaries, recovery benchmark methodology, adapter UX, and documentation clarity.

See [`CONTRIBUTING.md`](CONTRIBUTING.md). All contributors follow [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md). Report security issues per [`SECURITY.md`](SECURITY.md), not as public issues.

## License

Apache-2.0. See [`LICENSE`](LICENSE).

## Studio Context

Mimir is a [BES Studios](https://github.com/buildepicshit) project.
