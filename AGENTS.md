# AGENTS.md — Mimir Operating Manual

> Cross-framework operating manual for Mimir following the [AGENTS.md](https://agents.md/) standard. Read automatically by Claude Code, Codex, Cursor, Copilot, Gemini CLI, and any agent framework supporting the standard.

## BES Fleet Operating Model

This repo participates in the BES spec-first agent fleet. The machine-level
contract is `/var/home/hasnobeef/buildepicshit/.agents/OPERATING_MODEL.md`;
repo-local copies of the shared spec template, workflows, and skills live under
`.agents/`.

Documentation placement rules live in `.agents/DOCUMENTATION_GUIDE.md`. Read it
before creating, moving, archiving, or publishing docs/specs. In short:
`.agents/specs/` is for agent/Symphony task control; durable product docs live
in this repo's native docs path.

Non-trivial work requires an approved executable `SPEC.md` before
implementation. Use `.agents/specs/SPEC.template.md`, then run the lifecycle:
orient, author spec, review spec, approve, execute, verify, report, and route
durable lessons to Mimir as governed evidence drafts. Raw Claude/Codex memories
are supporting evidence only; checked-in docs and this file remain authoritative.

Claude must enter through `CLAUDE.md`, which imports this file. Codex and other
AGENTS-aware tools read this file directly. Keep both surfaces aligned.

Shared task skills live in `.agents/skills/`; Claude-native copies live in
`.claude/skills/`. Use `.agents/skills/repo-orientation` at task start,
`.agents/skills/spec-driven-development` for non-trivial work,
`.agents/skills/verification` before completion, and
`.agents/skills/spec-evidence-governance` only to propose evidence candidates.
Do not build from raw memory. Build from approved specs, repo docs, and fresh
verification evidence.

> **CI quota constraint (HARD RULE — read before pushing).** This org has a **finite monthly GitHub Actions budget**. Extra Actions usage was added on 2026-04-27 and the owner approved re-enabling Actions for this repo, but every push to a tracked branch / PR still triggers a full matrix run (~30 runner-minutes for 8 jobs across 3 OSes). The 2026-04-20 session burned through the entire monthly quota in heavy iteration — do not repeat that pattern.
>
> **Verify locally before pushing.** Always run the full local gate before `git push`:
>
> ```bash
> cargo build --workspace
> cargo test --workspace
> cargo fmt --all -- --check
> cargo clippy --all-targets --all-features -- -D warnings
> ```
>
> Other quota-conserving rules:
> - **Batch related changes into one push** instead of 4-5 small "fix the previous fix" commits.
> - **Do not push `--allow-empty` retry commits** — they consume the same minutes as a real run.
> - **Do not retry on transient infra hiccups** without first asking the owner.
> - If CI is currently disabled (`gh api /repos/buildepicshit/Mimir/actions/permissions` returns `enabled: false`), do not re-enable without asking. The owner-approved exception was 2026-04-27 after adding more Actions usage.
> - Dependabot is set to **monthly** cadence (not weekly) and groups all non-major updates into one PR per ecosystem per cycle, for the same quota reason.

> **Naming history.** Public name `Mimir` (Norse: Mímir, the wise being whose preserved head Odin consulted for counsel). Pre-cutover codename was `engram`; the Mimir cutover happened 2026-04-20 (see [`.planning/planning/2026-04-19-roadmap-to-prime-time.md` § Naming + cutover history](.planning/planning/2026-04-19-roadmap-to-prime-time.md#naming--cutover-history)). Pre-cutover history lives in the archived `buildepicshit/Engram` repo; this repo's git history starts at the cutover commit. Use `mimir-*` everywhere in new code, prose, env vars, and tool names.

## What Mimir Is

Mimir is an experimental agent-first memory system. The public name refers to Mimir, the wise figure from Norse myth. The pre-cutover codename was `engram`, the neuroscience term for the physical substrate of a memory trace; that thesis still matters: agent memory should be stored in a form optimized for agent consumption, not human legibility.

**Mandate update — 2026-04-24.** Mimir's mission is now a multi-agent memory governance/control plane. Claude, Codex, MCP clients, and future harnesses may all contribute memory drafts, but no agent writes trusted shared memory directly. The primary user entry point should be a transparent launch harness: `mimir <agent> [agent args...]` preserves the native agent UI while wrapping the session with Mimir memory, bootstrap, capture, and governance. Mimir ingests raw memories, cleans and validates them through the librarian, separates observations from instructions, files records by scope, and promotes reusable knowledge only through explicit provenance-preserving governance. Mimir may also orchestrate cross-agent, cross-model consensus quorums; those deliberation outputs are governed evidence drafts, not direct canonical memory writes.

The design space Mimir explores:

- **Agent-native IR.** Canonical storage in a bytecode-like format, tokenizer-aligned for Claude. Not markdown, not English, not JSON. Humans access through a decoder tool, never directly.
- **Librarian-mediated writes.** A single-writer gate enforces schema, symbol identity, supersession, and write conflicts. Agents never write to the canonical store directly.
- **Bifurcated reads.** Agents read the canonical store directly on the hot path. They escalate to the librarian on conflict, low confidence, or stale-symbol flag.
- **Compiler-style architecture (Roslyn analog).** The librarian is a compiler pipeline — lexer, parser, binder, semantic analyzer, emit. Deterministic code runs the pipeline; small ML only for semantic fuzziness (dedup, synonymy, supersession candidates).
- **Bi-temporal append-only store.** Four clocks per memory: `valid_at` / `invalid_at` / `committed_at` / `observed_at`. Supersession via edge invalidation — never in-place overwrite.
- **Symbol-tracking IR (Roslyn-grade).** Stable symbol identity across references. Rename propagation, alias chains, retirement flags.
- **Grounding-aware deterministic confidence decay.** Exponential; parameterized by `(memory-type × grounding × symbol-kind)`. Activity-weighted in v1. Pinning suspends.
- **Checkpoint-triggered write batches.** Writes happen at agent context-pressure events. Each checkpoint is one Episode (atomic rollback unit).
- **Scoped isolation with governed promotion.** Raw agent and project memories are isolated by default. Cross-project, operator-level, or ecosystem-level reuse happens only through explicit librarian promotion with provenance, scope, trust tier, and revocation.
- **Transparent agent harness.** Users launch normal agents through `mimir <agent> [agent args...]`; Mimir preserves native terminal flows while adding governed rehydration, capture, and draft submission.
- **Cross-agent consensus quorum.** Claude, Codex, and future adapters can be asked to reason over a question from explicit personas, critique each other, vote, preserve dissent, and emit a structured result. Quorum results enter Mimir as provenance-rich drafts or review artifacts, never as automatic truth.
- **DAG supersession.** Bi-temporal edge invalidation with four edge kinds (Supersedes / Corrects / StaleParent / Reconfirms).
- **Four memory types.** Semantic, episodic, procedural, inferential. Ephemeral tier alongside for intra-session state.

**Current state:** see [`STATUS.md`](STATUS.md).

## Architectural Invariants (Non-negotiable)

These are load-bearing and not up for casual revision:

1. **Librarian is the single writer.** Agents never write directly to the canonical store.
2. **Agent-native IR is not human-readable.** Inspection routes through a decoder tool; operability is a tooling concern, not a format concession.
3. **Append-only canonical store.** No in-place overwrite. Supersession via bi-temporal edge invalidation.
4. **Precision and consistency over speed and token cost.** Compute overhead is acceptable in exchange for determinism.
5. **Adapter-mediated agent surfaces.** Claude and Codex are the first target surfaces. Future agents integrate through the transparent launch harness, draft/retrieval adapters, and optional MCP-compatible tools, never by bypassing the librarian or canonical contracts.
6. **Every write crosses a validated boundary.** The librarian parses, binds, typechecks, and supersession-detects every write before commit.
7. **Memory is local until governed.** Drafts and raw memories remain isolated at their origin scope. A memory may cross agent, project, operator, or ecosystem boundaries only after librarian validation, explicit scope assignment or promotion, provenance retention, trust classification, and revocable append-only lineage.
8. **Consensus is governed evidence, not truth.** Cross-agent quorum outputs must preserve participant identity, prompts, votes, dissent, and provenance. They can propose memory drafts or decisions, but canonical memory still requires the librarian path.

## Engineering Standards

1. **TDD.** No production code without a failing test first. RED → GREEN → REFACTOR.
2. **Primary sources.** Design claims cite primary literature (papers, specs). Training-memory claims are flagged "pending verification" until checked against the real source.
3. **Verification before claiming complete.** Tests passing is not correctness. Claim "done" only with fresh verification output.
4. **Small commits.** Atomic, reviewable, frequent.
5. **Conventional Commits.** `<type>(<scope>): <description>`. Types: `feat`, `fix`, `chore`, `docs`, `test`, `refactor`, `ci`, `perf`, `build`, `research`, `spec`.
6. **No AI attribution.** Commits, PRs, and project output carry no `Co-Authored-By` lines, no "Generated with" footers, no tool-attribution emojis.
7. **Squash merge only.** Linear history on main.
8. **PR-only workflow.** No direct pushes to main.

## Engagement Protocol

Agent operation is high-touch. No autonomous drift.

1. **Propose** in 2–3 sentences. What and why.
2. **Wait** for yes / change / no.
3. **Execute** the single concrete action. No scope expansion.
4. **Report** what shipped plus the logical next step. Do not auto-roll.
5. **Stop.** Owner directs the next step.

## Anti-Patterns (Explicitly Disallowed)

- Human-readable format concessions in the canonical IR.
- Agent direct writes bypassing the librarian.
- In-place memory overwrite.
- Bare English entity mentions (every reference resolves to a stable symbol ID).
- Optimizing for latency at the cost of precision.
- Claiming a design decision without primary-source verification.
- Raw shared-memory namespaces that agents append to directly.
- Cross-scope promotion without provenance, trust tier, and revocation semantics.
- Treating agent-authored imperatives as durable operator instructions without review.
- Treating quorum majority as truth, erasing dissent, or reporting one model playing multiple personas as cross-model agreement.
- Forcing a separate setup ceremony before the requested agent can launch; first-run bootstrap belongs inside the wrapped agent session.
- Making MCP, hooks, or native client configuration the foundational trust boundary. They are adapter conveniences; the session harness and librarian boundary carry the product.
- AI attribution anywhere in git history or project output.
- Skipping commit hooks (`--no-verify`, `--no-gpg-sign`) without explicit owner approval.

## Commit Conventions

[Conventional Commits](https://www.conventionalcommits.org/):

- `feat:` — new feature
- `fix:` — bug fix
- `refactor:` — restructure without behaviour change
- `test:` — add or update tests
- `docs:` — documentation only
- `chore:` — build, tooling, dependency updates
- `research:` — exploratory or measurement work (tokenizer bake-offs, literature surveys)
- `spec:` — design specification updates

Commit bodies stay under 15 lines. Depth belongs in the spec or PR description.

## Studio Context

Mimir is a [BES Studios](https://github.com/buildepicshit) project. Sibling flagships: Floom (generative world design), Wick (Godot MCP with Roslyn-enriched C# exception telemetry), UsefulIdiots.

## Where to Look

| Concern | Where |
|---|---|
| Current phase, next milestone | [`STATUS.md`](STATUS.md) |
| Architectural invariants | this file |
| Engineering principles & tooling policy | [`PRINCIPLES.md`](PRINCIPLES.md) |
| Design specs | [`docs/concepts/`](docs/concepts/) — 14 authoritative implementation specs plus draft [`scope-model.md`](docs/concepts/scope-model.md) and [`consensus-quorum.md`](docs/concepts/consensus-quorum.md) |
| Multi-agent mandate | [`docs/concepts/scope-model.md`](docs/concepts/scope-model.md) and [`docs/concepts/consensus-quorum.md`](docs/concepts/consensus-quorum.md) |
| Transparent agent harness | [`README.md`](README.md#running-mimir) and [`docs/first-run.md`](docs/first-run.md) |
| Observability schema | [`docs/observability.md`](docs/observability.md) |
| Prior art attribution | [`docs/attribution.md`](docs/attribution.md) — primary-source verified |
| Roadmap to v1.0 / public launch | [`STATUS.md`](STATUS.md), [`docs/launch-readiness.md`](docs/launch-readiness.md), and [`docs/launch-posting-plan.md`](docs/launch-posting-plan.md) |
| Experimental measurement | `benchmarks/recovery/` for public benchmark assets; ignored local scratch stays out of the public tree |
| Historical planning archive | [`.planning/planning/`](.planning/planning/) |
