# Mimir librarian — archived prototype findings

Archived findings from the 2026-04-20 librarian prototype. The runnable Python script has been retired; production librarian work now lives in the Rust `mimir-librarian` crate.

## Files

| File | Purpose |
|---|---|
| `sample_drafts.jsonl` | 10 hand-crafted prose drafts covering the main shapes: pure observation, pure directive, mixed (sanitisation-critical), event, empty, multi-fact, supersession, ambiguous preferences, security directive, temporal. |
| `real_drafts.jsonl` | 9 real auto-memory drafts used in iteration 2/3 prototype runs. |
| `ITER2_FINDINGS.md` / `ITER3_FINDINGS.md` | Historical prototype measurements and prompt findings. |
| `results/<ts>/` | Per-run output (gitignored): `results.jsonl` (per-draft detail), `summary.json` (aggregate metrics), `run.log` (per-invocation trace). |

## What the retired prototype did

1. Ingests prose drafts.
2. Dispatches each to `claude -p --no-session-persistence --system-prompt <librarian_prompt> <draft>`.
3. Parses the librarian's JSON response — expected shape `{"records": [{"kind", "lisp"}, ...], "notes"}`.
4. Runs each `lisp` string through `mimir-cli parse` to verify syntactic validity.
5. Reports per-draft outcomes and aggregate stats.

## Why it was retired

The Rust `mimir-librarian` crate now owns the production path: scope-aware draft envelopes, `submit` / `sweep` / `run` / `watch`, bounded LLM validation retry, pre-emit validation through `mimir_core::Pipeline`, durable canonical commit, supersession-conflict handling, duplicate filtering, scheduling, and structured observability. Keeping the Python script would leave a stale parallel librarian path.

## First real run — 2026-04-20

Baseline result on `claude-sonnet-4-6`, wrapped-draft envelope, hardened system prompt:

- 9/9 non-empty drafts produced ≥1 parseable record.
- 21/21 emitted records parsed cleanly.
- 1 draft correctly identified as no-durable-content ("hello") and skipped.
- Sanitisation-critical drafts (mixed observation+directive, ambiguous preferences, security directive) all correctly split instruction from memory into separate `pro` records.
- Elapsed: ~163 s for 10 drafts, mean ~16 s per invocation.

Prior iteration — before the system-prompt hardening + `<draft>` envelope — had two drafts where Claude-as-librarian obeyed the content ("Ready." response) instead of structuring it, and one epi draft that emitted without `:obs` (schema gap in the prompt). Both modes fixed in the current prompt.

## Why this matters

The parse-rate concern from [`docs/research/2026-04-20-llm-fluency-cc-baseline.md`](../../docs/research/2026-04-20-llm-fluency-cc-baseline.md) (74% on Sonnet, query form at 16%) was measuring Claude's native emit fluency — a metric that became architecturally irrelevant the moment agents stopped emitting Lisp. The librarian is a purpose-built single-job process; getting its parse rate to 100% is a prompt-engineering problem, not a general-model-fluency problem. The first-pass result (21/21) suggests the approach holds.

## Current implementation

Use `mimir-librarian`:

```bash
cargo run -p mimir-librarian -- submit --text "..."
cargo run -p mimir-librarian -- sweep --path ~/.codex/memories --source-surface codex-memory
cargo run -p mimir-librarian -- run --drafts-dir ~/.mimir/drafts --workspace /path/to/canonical.log
cargo run -p mimir-librarian -- watch --drafts-dir ~/.mimir/drafts --workspace /path/to/canonical.log
```
