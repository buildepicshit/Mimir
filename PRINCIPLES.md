# Mimir Engineering Principles

> **Tracked since Phase 2 (PR #2).** This file is the public engineering-practice and product architecture boundary surface. Section-level changes go via PR per `CHANGELOG.md`.

## Framing principle

> Mimir is built to optimize **agent memory usage, not human-developer ergonomics.** Engineering-principle tradeoffs resolve in favor of what makes a better memory system for a Claude-agent consumer — not what is more convenient for the human author.

Application: prefer determinism over speed, type rigor over flexibility, correctness-by-construction over iteration velocity, explicit error modelling over implicit propagation, and boundaries that force invalid states to be unrepresentable rather than detected-and-rejected.

This principle is load-bearing. When a tradeoff is ambiguous, the question to ask is: *which resolution makes Mimir more useful to the agent consuming it?* — not *which is faster to write or easier for a human to skim.*

---

## Architectural Boundaries

1. **Librarian-governed writes.** Agents may propose drafts, but trusted canonical records are committed only through the librarian path.
2. **Agent-native canonical form.** The canonical store is optimized for agent consumption, not direct human editing. Humans inspect it through decoder tools.
3. **Append-only history.** Canonical memory is never overwritten in place. Revocation, correction, supersession, and retirement are represented as new records or edges.
4. **Deterministic core pipeline.** Lexing, parsing, binding, semantic analysis, emission, replay, read routing, and invariant checks are deterministic.
5. **Adapter-mediated surfaces.** Claude, Codex, MCP, hooks, and future clients are adapters around the governed store; none of them bypass the librarian boundary.
6. **Validated write boundary.** Every accepted write crosses parsing, binding, semantic validation, provenance capture, and atomic checkpointing before it becomes trusted memory.
7. **Scoped memory by default.** Raw drafts and local memories remain scoped to their origin until governed promotion assigns broader scope, provenance, trust tier, and revocation semantics.
8. **Consensus is evidence, not truth.** Multi-agent quorum output preserves participant identity, prompts, votes, dissent, and provenance; it does not become canonical truth without the librarian path.

---

## 1. Testing strategy

Mimir's librarian is compiler-shaped (lexer → parser → binder → semantic analyzer → emit). Each pipeline stage has deterministic input→output behavior, which makes testing straightforward — but only if testing discipline is explicit about *what kind* of test answers *what kind* of question.

### Test layers

- **Unit tests** (inline `#[cfg(test)]`). Per-stage component tests. Fast, tight, one behavior per test. Every pipeline stage has unit coverage over happy path, boundary cases, and explicit error paths.
- **Property tests** (`proptest`). Generate valid inputs, assert invariants hold. Core invariants to property-test:
  - Surface-IR round-trip: `parse(emit(canonical)) == canonical` for any valid fact.
  - Symbol-table consistency: after any sequence of allocate / rename / alias / retire, lookups match expected resolutions.
  - Supersession DAG: after any sequence of writes + supersessions, canonical form contains no in-place overwrites.
  - Episode atomicity: any partial-failure injection leaves the store in a pre-checkpoint state, not an intermediate one.
- **Snapshot tests** (`insta`). Canonical-form encodings of a representative corpus of facts. Commits changes to the encoding are explicit and reviewable. Catches unintentional format drift.
- **Integration tests** (`tests/` directory). Full write-read cycles against real filesystem WAL — **no mocks for the store backing**. Hermetic (no network, no external services), but state-holding dependencies are real. Rationale: per user feedback, mocked-DB integration tests hide migration / storage-layer bugs.
- **Fuzz tests** (`cargo-fuzz`, optional). IR parser must handle malformed input deterministically — no panics, no silent acceptance. Fuzz runs in CI nightly, not per-PR.

### Coverage

Prefer property-test breadth over branch coverage percentage. A 60%-branch-covered codebase with strong property tests catches more real bugs than a 95%-covered one with only unit tests of known inputs. No dogmatic percentage threshold; review gates on "is the core invariant tested?" not "is every line covered?"

### What is NOT tested

- Filesystem-level guarantees (fsync durability, etc.) — trusted at the OS boundary; verify only Mimir's use of them.
- External ML model outputs in unit tests (see § 4 Determinism-vs-ML boundary). ML outputs are wrapped in deterministic decisions; test the wrapper, not the model.

---

## 2. Error-handling philosophy

### Errors are data, not strings

All recoverable errors are typed enums with `#[derive(thiserror::Error)]`. Every subsystem owns its error type:

- `mimir_core::parse::ParseError`
- `mimir_core::bind::BindError`
- `mimir_core::store::StoreError`
- `mimir_core::wire::WireError`

Error types carry structured context — affected symbol, byte offset, attempted operation. Agent consumers parse errors by variant; they never regex-match error messages.

### Panics are for bugs, not for user errors

- `panic!`, `unreachable!`, `unwrap()`, `expect()` are forbidden in library crates (`mimir_core`, `engram_store`, `engram_wire`) outside of `#[cfg(test)]`. `clippy::unwrap_used` and `clippy::expect_used` enforce this.
- Panics indicate a broken invariant — a bug. A malformed agent write is *not* a bug, it's a `Result::Err`.
- Panics are allowed in binary crates (`mimird`, `mimir-cli`) for startup-time invariant violations (missing config, corrupted initial state).

### Boundary validation, internal trust

- Validate at: IR parser input, wire deserialization, decoder-tool CLI argv, filesystem reads.
- Internal code (bound IR, canonical form already in memory) trusts its types. Validation inside the librarian pipeline is type-driven, not runtime-checked.

### `anyhow` only in binaries

- Library crates use concrete `Result<T, E>` with thiserror-derived enums. No `Box<dyn Error>` in public library API.
- `anyhow::Error` is allowed only in binary crates for top-level main-function error chaining.

### Agent-facing errors are structured

Agent consumers receive errors as JSON-serializable structured values (over the wire, in the same IR dialect as successful responses). Plaintext-only errors are an anti-pattern: an agent can't reliably route or recover from them.

---

## 3. Type safety policy

### Newtypes for every domain primitive

Bare `u64`, `String`, `f32` are forbidden in domain-meaning positions. Domain types:

- `SymbolId(u64)`
- `EpisodeId(Ulid)`
- `ClockTime(u64)` — represents one of the four clocks, tagged by phantom.
- `Confidence(f32)` — constrained to `0.0..=1.0` at construction.
- `MemoryKind` — enum over `Semantic | Episodic | Procedural | Inferential | Ephemeral`.
- `Grounding` — enum over source types.

Newtypes use `#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]` as applicable, plus explicit `From`/`TryFrom` implementations for conversion boundaries.

### Exhaustive matching

- `match` over finite enums uses explicit variants. No wildcards (`_`) unless the enum is `#[non_exhaustive]` or matches a runtime-open surface.
- `#[deny(unreachable_patterns)]` and `#[deny(unused_variables)]` at library root.

### `#![forbid(unsafe_code)]` at library root

- Core librarian crates forbid `unsafe`. Any future unsafe need goes in a separately-audited module (`unsafe_impl/`) with documented safety invariants and review at introduction.

### No dynamic dispatch on core types

- `dyn Trait` is allowed only for plugin-style extension points with documented justification. Core types (IR nodes, symbol-table entries, canonical-form records) are monomorphic and statically dispatched.

### Validation: once, at the boundary

- Parser input is validated on entry; downstream pipeline stages operate on already-validated types.
- The type system should make it impossible to represent an invalid canonical record. Invariants enforced by construction, not by runtime assertion.

---

## 4. Determinism-vs-ML boundary

Mimir's deterministic-core boundary permits ML only for semantic fuzziness (dedup, synonymy, supersession candidates). This section sharpens that boundary.

### Fully deterministic (no ML permitted)

Every read-hot-path operation, every librarian pipeline stage, every invariant-maintaining operation:

- Lexing, parsing, binding, semantic analysis, emission.
- Canonical encoding / decoding.
- Symbol-table lookup, allocation, rename propagation, alias-chain resolution, retirement flags.
- Temporal clock assignment, bi-temporal edge invalidation, DAG merge.
- Confidence decay (closed-form exponential, parameterized per `(memory-type × grounding × symbol-kind)`).
- Episode atomicity, rollback, and replay.

### ML-permitted, deterministic-wrapped

ML-capable operations *propose*; the librarian deterministically *decides*:

- **Dedup candidate proposal.** ML scores potential duplicates; librarian applies a deterministic merge-or-retain rule based on score × threshold.
- **Synonymy detection.** ML proposes synonym candidates; librarian records each with provenance + confidence as a first-class memory. Never auto-applied silently.
- **Supersession candidate proposal.** ML scores fact pairs; librarian commits supersession only when score exceeds threshold AND agent confidence meets a separate deterministic gate.

### Mandatory properties of ML operations

1. **Determinism across replays.** ML output is reproducible: same input + same model version + same seed → same output. The librarian records model version + seed + input hash alongside any ML-originated decision.
2. **Agent never observes raw ML output.** ML-proposed candidates are always wrapped in a deterministic decision that is itself a first-class memory (visible via the decoder tool, auditable).
3. **Reversibility.** Any ML-originated decision can be reverted by the librarian via the normal supersession mechanism.

### Out of scope for v1

- Pure-LLM reflection-style consolidation (Generative Agents-style). Mimir chooses deterministic graph-rewrite consolidation for v1.
- ML-based retrieval ranking on the read hot path. Hot-path reads are deterministic; ML-ranked semantic search is a Phase ≥5 consideration, and only via an escalation-triggered path.

---

## 5. Logging and observability

### `tracing` is the single interface

- `tracing` for all emission. `log` crate is bridged in at the binary layer via `tracing-log`.
- `tracing-subscriber` configures output: JSON in production, pretty in dev. No direct `println!` / `eprintln!` in library code (allowed only in CLI tools for user-facing output).

### Structured, not formatted

- Events carry fields: `tracing::info!(episode_id = %id, fact_count = facts.len(), "episode committed")`.
- Agent consumers parse log events structurally. String-concatenated messages are anti-pattern.

### Level policy

- `ERROR`: invariant violations, unrecoverable errors, integrity failures. Pageable.
- `WARN`: recoverable anomalies — malformed IR rejected, supersession conflict detected, stale-symbol read escalation.
- `INFO`: lifecycle events — librarian start/stop, episode commit, checkpoint flush, supersession applied.
- `DEBUG`: per-operation traces. Off by default.
- `TRACE`: per-value diagnostics. Only when explicitly requested.

### Correlation

- Every Episode has an `EpisodeId`.
- Every write batch within an Episode carries the Episode ID as a tracing field.
- Every log line within processing that batch tags itself with the Episode ID + batch ordinal.
- Read-side operations tag with a `ReadRequestId` for similar traceability.

### Decoder-tool integration

- Log event schemas are stable: the decoder tool consumes JSON logs to produce post-hoc Episode traces.
- Log event schema changes follow the deprecation policy (§ 11).

### Privacy

- Canonical store may hold arbitrary agent-written content. Logs carry **identifiers only, never values**. Values are accessible via the decoder tool against the canonical store — which is the audited path.

---

## 6. Performance and scale targets (v1, Claude-only)

Targets are **directional until real workload profiling lands**. They guide design decisions ("don't choose an approach that precludes X") but are not hard SLOs.

### Working assumptions

- **Librarian write throughput:** ≥ 1,000 facts/sec sustained on the single-writer pipeline, measured at the wire→canonical-committed boundary.
- **Librarian read throughput:** ≥ 10,000 facts/sec on the hot path.
- **Write latency p50:** < 5 ms wire-receive → append-confirmed (single-fact write, warm librarian).
- **Write latency p99:** < 50 ms.
- **Read latency p50:** < 1 ms for a symbol-bound lookup against warm cache.
- **Memory footprint:** < 500 MB resident for a 1M-fact canonical store. Symbol table in-memory; canonical body paged/mmap'd.
- **Canonical store size:** ≤ 100 bytes/fact average, including symbol-table references and temporal clocks.
- **Cold-start time:** < 2 seconds to open a 1M-fact store and resume both read and write paths.

### What these are not

- Not SLOs — no one is paged on them.
- Not commitments to users — Mimir v1 is single-deployment.
- Not upper bounds — Mimir should be substantially better than these where cheap.

### What these drive

- Storage format choices (canonical bytecode density).
- Symbol-table residency decisions (all in-memory vs partial).
- Concurrency model (single-writer vs optimistic-multi-writer).
- Whether to checkpoint-batch at all (yes, these targets assume batching).

Reassess with real workload data in Phase ≥5.

---

## 7. Code style and tooling

### Tools

| Concern | Tool | Config |
|---|---|---|
| Format | `rustfmt` | default; no custom `rustfmt.toml` unless a concrete conflict |
| Lint | `clippy` | `-D warnings`, `clippy::pedantic` opt-in per-module |
| Type check | `rustc` | `#![deny(warnings)]` at library roots |
| Tests | `cargo test` | plus `cargo test --doc` for doctests |
| Property tests | `proptest` | minimum-case shrinking enabled |
| Snapshot | `insta` | `cargo insta review` workflow |
| Deps audit | `cargo-deny` | license + advisory + source bans |
| Fuzz | `cargo-fuzz` | nightly CI only |
| Doc generation | `cargo doc --no-deps` | CI artifact, not committed |

### Lint additions beyond clippy defaults

- `clippy::unwrap_used` — deny in library crates outside tests.
- `clippy::expect_used` — deny in library crates outside tests.
- `clippy::panic` — warn; allowed with justification comment.
- `clippy::todo` — warn; blocks release.
- `clippy::dbg_macro` — deny in non-test code.
- `clippy::pedantic` — opt in at crate level; opt out per-module where noise > signal.

### File and naming conventions

- `snake_case` for files, functions, variables, modules.
- `UpperCamelCase` for types, traits, enum variants.
- `SCREAMING_SNAKE_CASE` for constants and statics.
- One public type per file where practical in library crates.
- `mod` declarations in `lib.rs` or `mod.rs` only; no in-directory stray mods.
- Import order (rustfmt default): `std`, external crates, workspace crates, `self`/`crate` — separated by blank lines.

### Comment policy

Default: no comments. Add only when the **WHY** is non-obvious:

- Hidden invariants (cross-module contract not visible locally).
- Workarounds for specific bugs (link to issue).
- Surprising behavior a reader would otherwise misread.

Don't explain WHAT — well-named identifiers and types do that. Don't reference the current task, PR, or caller ("used by X", "added for Y flow") — that rots.

Module-level `//!` docs are different and required (see § 9 Documentation standard).

### CI gates

Every PR runs:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo test --doc
cargo deny check
```

Failing any of these blocks merge.

---

## 8. Dependency policy

### Minimize

Every dependency is a review gate. Default is "use std or write it ourselves." Adding a dep requires justification in the PR description: what it solves, what the std/hand-rolled alternative costs, supply-chain risk.

### License audit

`cargo-deny` enforces. Allowed licenses:

- Apache-2.0
- MIT
- BSD-3-Clause
- ISC
- 0BSD
- Unlicense
- CC0-1.0
- Unicode-3.0 — Unicode License v3, OSI-approved, permissive, non-copyleft. Standard license for Unicode data tables in the Rust ecosystem (e.g. `unicode-ident`, a transitive dep of `thiserror` via `proc-macro2`/`syn`). Allowed because avoiding it is infeasible without forking core ecosystem crates.

Anything else requires explicit PR approval and a `cargo-deny` exception entry.

### Supply-chain guard

- `cargo-deny advisories` in CI — no known-vulnerable deps ship.
- `cargo-deny bans` — explicit bans on deps we've evaluated and rejected (recorded with rationale).
- `cargo-deny sources` — only crates.io by default. Git deps require exception.

### Unsafe code in deps

Any dep that uses `unsafe` is reviewed on introduction. Approved unsafe-using deps are listed in this document (see "Approved unsafe deps" below, to be populated when we add them).

### Pinning

- Binary crates: `Cargo.lock` is committed.
- Library crates: `Cargo.lock` committed but semver ranges used in `Cargo.toml`.
- MSRV is declared in `Cargo.toml` via `rust-version`; changes are breaking under pre-1.0.

### Expected foundational deps

When we scaffold the workspace:

- `serde` — derive framework for structured data (error payloads, config, tracing fields, snapshot-test serialization). Wire-format parser/emitter crate choice is a Phase 3 decision — the write surface is Lisp S-expr per the tokenizer bake-off, so the wire serialization crate (hand-rolled, `serde_lexpr`, or similar) is specified in `docs/concepts/wire-architecture.md` when that spec lands.
- `thiserror` — library-crate error derives.
- `anyhow` — binary-crate error chaining only.
- `tracing` + `tracing-subscriber` — logging (JSON output via `tracing-subscriber`'s json layer, unrelated to wire format).
- `tokio` — async runtime, subject to wire-architecture decision.
- `proptest` — property tests.
- `insta` — snapshot tests.
- `ulid` — episode IDs (time-sortable, collision-resistant).

Anything else requires the dep-add review process above.

### Approved unsafe deps

*(Empty — populates as unsafe-using deps are added and approved.)*

---

## 9. Documentation standard

### `rustdoc` on every public item

Every `pub` in a library crate carries a doc comment. No undocumented public API. CI enforces via `#![deny(missing_docs)]` at library roots.

### Doctest coverage

Every public function has at least one doctest showing canonical usage. `cargo test --doc` runs in CI. Doctests double as usage examples and regression tests.

### Module-level overviews

Every `mod` has a `//!` section covering:

1. Purpose — what this module is responsible for.
2. Invariants — what callers can assume.
3. Pipeline position — how this module fits the overall librarian pipeline (lex / parse / bind / semantic / emit / store).

### Architectural invariant pointers

Public items that rely on an `PRINCIPLES.md` invariant or a `docs/concepts/` specification include a doc-comment reference:

```rust
/// Writes a fact to the canonical store.
///
/// Honors the librarian-governed write boundary and
/// `docs/concepts/write-protocol.md` § Episode atomicity.
pub fn commit_fact(...) -> Result<...> { ... }
```

### No generated docs in the repo

`cargo doc --no-deps` output is a CI artifact, not committed. `docs/` holds concept specs and attribution — it is not an API reference mirror.

### Where WHY lives

- Code comments: local non-obvious WHYs only.
- Module `//!` docs: module-level invariants and pipeline position.
- `docs/concepts/`: cross-module architectural WHYs, referenced from code.
- `PRINCIPLES.md`: project-wide invariants, referenced from specs.

---

## 10. Semantic versioning policy

### Pre-release (`0.x.y`)

- Minor bump (`0.X.y+1` → `0.X+1.0`): indicates breaking changes.
- Patch bump (`0.X.Y` → `0.X.Y+1`): additive-compatible or fix-only.
- Breaking changes during `0.x` are **explicit in `CHANGELOG.md`** under a `Breaking` section.

### `1.0.0`

- Reached when the library-crate public API, wire format, and canonical form are stable for external consumers.
- Not a timeline commitment. Reached by deliberate decision, not calendar pressure.

### Post-1.0

- Strict semver. Major = breaking, minor = additive-compatible, patch = fix-only.

### What counts as the API surface

- Library crate `pub` items.
- Wire format (agent ↔ librarian).
- Canonical form (on-disk representation).

### Independent versioning

Wire format and canonical form carry their own version numbers, incremented independently of crate versions. A crate patch bump can accompany a wire-format major bump (if the crate gains support for the new wire version while keeping API stable). Cross-referenced in `CHANGELOG.md` and in the relevant `docs/concepts/` specs.

### MSRV

Declared in `Cargo.toml`'s `rust-version` field. MSRV bumps are breaking under pre-1.0 and minor-bump-worthy post-1.0.

---

## 11. Deprecation policy

### Marking deprecated

```rust
#[deprecated(since = "0.3.0", note = "use `commit_fact_v2` instead; see CHANGELOG.md")]
pub fn commit_fact(...) { ... }
```

Note must point to the migration target.

### Deprecation → removal window

- Post-1.0: minimum **two minor versions** between deprecation and removal.
- Pre-1.0: minimum **one minor version**.
- Removal is a breaking change (major bump post-1.0).

### `CHANGELOG.md` tracking

- Deprecations recorded under `Deprecated` in the release that introduces them.
- Removals recorded under `Removed` in the release that removes them.

### Canonical-form deprecations

Removed canonical opcodes trigger a compatibility layer in the decoder tool for at least one major version post-removal. Rationale: canonical-store migration is a one-way door for user data; the decoder tool must continue reading legacy stores.

### Wire-format deprecations

Follow the same window as library-crate deprecations, but the librarian must advertise supported wire versions at handshake. Agents negotiate via versioned handshake; unsupported wire versions produce a typed `WireError::UnsupportedVersion`.

---

## 12. Release process

Locked once v1 ships. Working assumptions for now:

- **Tagging.** Annotated tags on `main`: `git tag -a vX.Y.Z -m "release X.Y.Z"`. Signed tags required post-1.0.
- **`CHANGELOG.md`.** `[Unreleased]` section becomes `[X.Y.Z] - YYYY-MM-DD` at release cut.
- **Release notes.** Derived from `CHANGELOG.md` — the changelog *is* the release note.
- **Publishing.** Crates.io publication only post-1.0 or as deliberate pre-releases (`0.x.y-alpha.N`). Not during design phase.
- **Binary releases.** TBD. Likely `cargo-dist` + GitHub Releases if the decoder tool or `mimird` daemon warrants prebuilt binaries.
- **Branch model.** Trunk-based. Tags cut off `main`. No long-lived release branches in v1. Revisit if/when external consumers need LTS lines.

---

## Cross-references

- Architectural boundaries: `PRINCIPLES.md` § Architectural Boundaries.
- Current phase and milestone plan: `STATUS.md`.
- Gap analysis (the plan that produced this document): `research/GAP_ANALYSIS.md` (gitignored).
- Architecture specifications: `docs/concepts/` (populated in Phase 3).
- Primary-source attribution: `docs/attribution.md` (populated in Phase 4).

---

`PRINCIPLES.md` is the engineering-practice surface. It grows with the project and is consulted section-by-section by contributors working on the concern at hand. A Rust dev implementing error handling reads § 2; a contributor opening a PR consults § 7 + § 8 + § 9 selectively.
