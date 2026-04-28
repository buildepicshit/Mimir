# Launch Posting Plan - 2026-04-28

This is the concrete publication plan for Mimir's first public opening. It keeps launch claims bounded to what the repo implements today.

## Channel Order

1. **GitHub public repo** - canonical source of truth. Status: live.
2. **Launch article** - explains the problem, librarian boundary, harness, and dogfood evidence without benchmark overclaiming. Status: [`blog/2026-04-28-agent-memory-compiler-pipeline.md`](blog/2026-04-28-agent-memory-compiler-pipeline.md).
3. **Show HN + X / LinkedIn** - short pointer posts back to GitHub and the article after the article is merged.
4. **Codex plugin bundle** - publish as a coherent Mimir workflow package, not individual skills.
5. **Crates.io / docs.rs alpha** - after package dry-runs and crate README audit; real publish remains owner-gated and irreversible.

Deferred:

- **MCP Registry / MCP directories** - skip for this launch pass until an OCI image, MCPB bundle, or remote service artifact exists.
- **Dev.to or BES Studios blog mirror** - optional article mirror after GitHub + primary posts are live.

## Launch Article

Path: [`blog/2026-04-28-agent-memory-compiler-pipeline.md`](blog/2026-04-28-agent-memory-compiler-pipeline.md)

Title:

> Agent Memory Should Be Governed Like A Compiler Pipeline

Outline:

1. The practical failure mode: agents lose context, contaminate memory across repos, and restart badly.
2. The Mimir thesis: local-first, append-only, librarian-mediated, agent-native canonical memory.
3. Why the librarian boundary matters: drafts are evidence; canonical memory is governed.
4. Why the transparent harness matters: `mimir <agent> [agent args...]` preserves native workflows.
5. What works today: store, librarian, harness, MCP, operator tools, recovery mirror, recovery benchmark harness.
6. What is not claimed: production readiness, stable API, hosted service, benchmark victory.
7. What feedback is wanted: Rust correctness, security review, adapter UX, benchmark methodology, docs clarity.

## Show HN Draft

Title:

`Show HN: Mimir - local-first memory governance for AI agents`

Body:

`Mimir is an experimental Rust project for local-first agent memory governance. Agents submit untrusted memory drafts; a librarian validates and commits accepted records into an append-only canonical log with provenance. The repo includes a transparent harness (`mimir <agent> ...`), local MCP surface, operator inspection commands, Git-backed recovery mirroring, recovery-benchmark scaffolding, and a Codex plugin bundle. It is pre-1.0: no stable API/storage promises and no benchmark victory claims yet. Looking for review on Rust correctness, security boundaries, agent adapter UX, and benchmark methodology.`

## X Draft

`Mimir is public: experimental local-first memory governance for AI agents. Agents propose drafts; the librarian owns canonical writes. Rust core, transparent agent harness, MCP surface, operator controls, Codex plugin bundle, and Git-backed recovery mirror. Pre-1.0, feedback wanted. https://github.com/buildepicshit/Mimir`

## LinkedIn Draft

`We opened Mimir, an experimental local-first memory governance project for AI agents. The core design is simple: agents can propose memory, but trusted shared memory is committed only through a librarian pipeline with validation, provenance, append-only storage, and revocation semantics.`

`The repo includes a Rust core, transparent agent harness, MCP surface, operator inspection tools, Git-backed recovery mirroring, and recovery-benchmark scaffolding. It is deliberately pre-1.0; we are looking for review on correctness, security boundaries, adapter UX, and benchmark methodology.`

## GitHub Repo Settings

Recommended description:

`Experimental local-first memory governance for AI agents.`

Recommended topics:

`agent-memory`, `ai-agents`, `mcp`, `rust`, `local-first`, `memory-governance`, `codex`, `claude`

## Codex Distribution

Do not publish standalone Mimir skills. A single checkpoint skill lacks the setup, doctor, librarian boundary, and post-session processing context.

The public Codex artifact should be the repo plugin bundle at `plugins/mimir`, with the checkpoint skill as one internal component. Required before wider distribution:

- Plugin manifest reviewed.
- README install/verification path documented.
- `mimir doctor` and `mimir setup-agent doctor --agent codex` shown as verification commands.
- Canonical write boundary described as librarian-only.

## Crates.io And docs.rs

Do not publish crates before:

- `cargo publish --dry-run -p mimir-core --allow-dirty` passes.
- `cargo package -p <crate> --allow-dirty` or crate-specific publish dry-runs pass for dependent crates after `mimir-core` is available in the crates.io index.
- Crate READMEs match the root pre-1.0 story.
- `cargo doc --workspace --no-deps` passes.
- Release order is confirmed: `mimir-core` before dependent crates.
- Owner approves the irreversible crates.io publish step.

## MCP Directories

Do not submit to the official MCP Registry or third-party MCP directories until Mimir has a public install artifact they can consume:

- OCI image, or
- MCPB bundle, or
- production remote endpoint.

Until then, GitHub plus the local `mimir-mcp` docs are the canonical MCP distribution surface.

## Posting Checklist

- README and docs index merged.
- `docs/launch-readiness.md` current.
- Local gate green.
- Main CI green after the single cleanup push.
- Launch article committed or published.
- Codex plugin bundle checked as `plugins/mimir`, not individual skills.
- Crates.io alpha release path checked up to dry-run; real publish explicitly approved.
- Public issue lanes seeded for correctness, security, adapter UX, benchmarks, and docs.
