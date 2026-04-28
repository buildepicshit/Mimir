# Mimir

Experimental, pre-1.0 memory governance for AI agents.

Mimir started as an experiment to see whether we could reduce a few recurring pains in day-to-day agent work: cross-agent memory management, memory contamination between contexts, and losing useful working memory during restarts or system failures. The core thesis: when agents write and read their own memory, the canonical storage format should be optimized for agent consumption, not human legibility, with separate decoder and observability tools for human audit.

The name refers to Mimir (Norse: Mímir), the wise being Odin consulted for counsel. The pre-cutover codename was `engram`; the thesis still targets durable agent memory traces, but public code and docs now use Mimir throughout.

Mimir is being published early so other engineers, researchers, and agent-tool builders can inspect the architecture while it is still malleable. It is not production-ready: APIs, storage details, CLI behavior, and harness workflows may change before the first stable release. The intent is honest active development, not a polished v1 claim.

## Public status

| Area | State |
|---|---|
| Core append-only store | Implemented and covered by local unit, property, integration, and crash-injection tests. |
| Librarian-mediated writes | Implemented for validated draft processing; agents still do not write trusted shared memory directly. |
| MCP surface | Implemented for governed read/write tooling against the librarian boundary. |
| Transparent harness | Implemented for local wrapped Claude/Codex sessions; Copilot is now an official adapter target with read-only session-store recall and governed draft submission through `mimir-librarian copilot`. |
| Recovery benchmark | Scenario corpus, dry-run planner, environment validation, launch contracts, explicitly approved live execution, transcript gates, and score validation are in place; benchmark claims wait for recorded live runs. |
| Releases | No stable release yet. Treat `main` as pre-1.0 active development. |

The current public-readiness checklist is tracked in [`docs/planning/2026-04-27-public-readiness.md`](docs/planning/2026-04-27-public-readiness.md).

The original 14 architecture specs are `authoritative`; the newer scope-model and consensus-quorum specs are still `draft` while the multi-agent control-plane mandate is being productized. The 2026-04-24 mandate expands Mimir into a multi-agent memory governance/control plane with scoped promotion, consensus quorum artifacts, and a transparent launch harness (`mimir <agent> [agent args...]`). See [`STATUS.md`](STATUS.md) for the current phase snapshot, [`AGENTS.md`](AGENTS.md) for architectural invariants and the agent operating manual, and [`docs/planning/2026-04-24-transparent-agent-harness.md`](docs/planning/2026-04-24-transparent-agent-harness.md) for the launch-boundary direction.

The 2026-04-20 architecture pivot remains central: agents write prose, the librarian sanitizes and structures it into canonical Lisp before committing, and the canonical storage surface stays internal. Agents and human inspectors use MCP or `mimir-cli`; direct canonical writes are out of bounds.

## Quickstart

```bash
git clone https://github.com/buildepicshit/Mimir.git
cd Mimir
cargo build --workspace --all-features
cargo test --workspace --all-features
cargo run -p mimir-cli -- --help
cargo run -p mimir-harness -- --help
cargo run -p mimir-harness -- doctor --project-root .
```

For a safe fresh-clone walkthrough that verifies the harness with a no-op child process, see [`docs/first-run.md`](docs/first-run.md).

For Codex users, the repo includes a draft Mimir plugin bundle at [`plugins/mimir`](plugins/mimir). It is a workflow package, not a standalone skill: the bundled skill points Codex at `mimir doctor`, explicit setup inspection, checkpoint draft submission, and governed read-only context while preserving the librarian as the only canonical writer.

## Backup And Restore

Mimir's primary recovery path is explicit Git-backed BC/DR mirroring:

```bash
mimir remote status
mimir remote push --dry-run
mimir remote push
./scripts/bcdr-drill.sh --dry-run
./scripts/bcdr-drill.sh --destructive
```

`mimir remote push` mirrors the append-only workspace log and draft JSON
files into the configured recovery repository. `mimir remote pull`
restores missing or prefix-safe local state. Push and pull verify
canonical-log integrity before and after sync. The drill deletes the local
workspace log, restores it from the remote, verifies integrity, and runs a
read-path sanity query. Projects can opt into
`remote.auto_push_after_capture = true` to run the same verified push path
after wrapped-session capture and librarian handoff. See
[`docs/bc-dr-restore.md`](docs/bc-dr-restore.md).

## Why this exists

Running many agents across many projects, each instance's memory drifts across sessions, restarts, tools, and contexts that should never mix. System failures and cold starts can also erase useful working context at exactly the wrong time. The dominant cost is timeline, not tokens or latency: an agent acting on stale, missing, or contaminated context burns developer hours. Mimir is an attempt to make memory local until governed, then reusable through librarian validation, provenance, scoped promotion, and recovery paths that are better than isolated markdown files.

## Where help is useful

- Rust correctness review for the append-only log, replay, write-locking, and crash-recovery paths.
- Security and threat-model review around prompt/data boundaries, native agent setup, subprocess handling, and recovery remotes.
- Benchmark methodology review for the recovery scenarios, scoring rubric, transcript evidence gates, and future live pilots.
- Adapter and harness UX feedback from people who regularly run Claude, Codex, MCP clients, or other agent tools.
- Documentation cleanup where the internal design history is too dense for a new contributor.

## Technology

- **Librarian implementation:** [Rust](https://www.rust-lang.org). Chosen for compiler-shaped workload fit, deterministic performance without GC surprises, and alignment with the modern agent-memory ecosystem (LanceDB, Turbopuffer, qdrant).
- **Write-surface IR (v0, working assumption):** Lisp S-expression — picked empirically via the tokenizer bake-off on a 50-fact corpus.
- **License:** [Apache-2.0](LICENSE).

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). All contributors — human and agent — follow [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md). Report security issues per [`SECURITY.md`](SECURITY.md).

## Studio context

Mimir is a [BES Studios](https://github.com/buildepicshit) project. Sibling flagships: Floom, Wick, UsefulIdiots.
