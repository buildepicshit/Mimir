# mimir_core

Foundational crate of [Mimir](https://github.com/buildepicshit/Mimir), an experimental pre-1.0 memory governance system for AI agents. `mimir_core` provides the librarian's deterministic core: a compiler-pipeline (lex → parse → bind → semantic → emit) over a Lisp-shaped agent-native IR, plus the bi-temporal append-only canonical store, plus the read-side query engine.

> **Pre-1.0 status.** This crate is part of Mimir's active-development tree. APIs, storage details, and wire-format compatibility may change before v1. Public crates.io releases wait for the first alpha.

## Install

Until the first alpha release, use a workspace or path dependency:

```toml
[dependencies]
mimir-core = { path = "/path/to/Mimir/crates/mimir-core" }
```

## Quickstart

```rust
use mimir_core::{Store, Pipeline};

# fn main() -> anyhow::Result<()> {
let path = tempfile::tempdir()?.path().join("workspace.log");
let mut store = Store::open(&path)?;

// Commit a batch of agent-native memory forms.
let now = mimir_core::ClockTime::from_millis(1_700_000_000_000);
store.commit_batch("(sem @alice :p @likes :o @rust :c 0.95)", now)?;

// Query it back.
let result = store.pipeline_mut().execute_query(
    "(query :kind sem :s @alice :p @likes)",
    now,
)?;
assert_eq!(result.records.len(), 1);
# Ok(())
# }
```

## What's in here

- **Compiler pipeline:** `lex`, `parse`, `bind`, `semantic`, `pipeline::Pipeline::compile_batch` — agent input → typed canonical records.
- **Canonical wire format:** `canonical` — `[opcode][varint length][body]` framing, 18 opcodes, fully self-describing, fuzz-target-covered.
- **Append-only store:** `Store` over a `LogBackend`-abstracted `CanonicalLog`. Two-phase commit with crash-injection-tested rollback.
- **Read-side:** `Pipeline::execute_query` over the in-memory current-state index; p50 ≈ 0.57 µs on a 1 M-memory warm index.
- **Decay:** integer-fixed-point exponential decay via a hand-baked 256-entry lookup table — bit-identical across architectures.
- **Workspace partitioning:** `WorkspaceId` = `hash(git_remote_url)` with an in-process `.git/config` parser. The current implementation keeps raw workspace memories isolated; the draft scope model adds governed promotion above this layer.

## Engineering posture

- `#![forbid(unsafe_code)]` workspace-wide.
- Full test suite: unit + property + doctest + integration + crash-injection + 3 fuzz targets (live counts in [`STATUS.md`](../../STATUS.md) frontmatter).
- 19 `thiserror`-derived error enums — one per subsystem.
- `cargo deny check` gates licenses + advisories + sources on every PR.
- `unwrap_used = "deny"`, `expect_used = "deny"`, `dbg_macro = "deny"` workspace-wide; relaxed only inside `#[cfg(test)]`.

## Specs

The original 14 architecture specs in [`docs/concepts/`](https://github.com/buildepicshit/Mimir/tree/main/docs/concepts) are all `authoritative`; `scope-model.md` is a draft mandate-expansion spec. Every load-bearing claim in this crate cites the spec section that backs it via module-level `//!` docs.

## License

[Apache-2.0](LICENSE).
