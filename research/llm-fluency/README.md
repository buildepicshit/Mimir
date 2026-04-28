# LLM-Fluency Benchmark (Phase 3.2)

The wire-surface existential gate. Measures how reliably Claude emits parseable Mimir Lisp for natural-language write / query requests. Exit bar: **≥98% parse-success rate**. If the measured rate is below 98%, the roadmap (`docs/planning/2026-04-19-roadmap-to-prime-time.md` § Phase 3.2) says to **stop, course-correct the wire surface, and re-run** — do not lock the wire format into a published v0.1.0 until this bar is honestly cleared.

## Why this exists

Mimir's core thesis is that a Lisp write surface is more token-efficient and more agent-fluent than markdown / JSON / natural language for agent memory. The tokenizer bake-off (pre-Phase 1) validated the token-cost claim. This benchmark validates the **emit-fluency** claim: if Claude can't reliably produce valid Lisp for ordinary facts, events, rules, and queries, the thesis is falsified and we need a different wire surface.

The v0.1.0 API commits us to this surface publicly. Gating the cut on this measurement prevents a v1.x architecture reckoning.

## Discipline — corpus lockdown

The corpus (`corpus.jsonl`) and few-shot exemplars (`few_shot_examples.jsonl`) are committed artifacts. Locking them in a reviewed PR *before* any run prevents the failure mode where corpus phrasings drift unconsciously toward what Claude emits cleanly — that would silently p-hack the gate.

**If a run surfaces a systematic failure, stop.** Do not edit the corpus. Open an issue with the failing `parse_stderr` distribution from `summary.json` and discuss whether the corpus is unfair or the wire surface itself needs course-correction. The corpus is load-bearing *because* it's locked.

## Contents

| File | Purpose |
|---|---|
| `corpus.jsonl` | 100 prompts: 25 each of `sem` / `epi` / `pro` / `query`. English prompt + ground-truth Lisp. |
| `few_shot_examples.jsonl` | 5 exemplars fed as few-shot context to Claude. Distinct from the corpus (no leakage). |
| `verify_corpus.py` | Sanity check — pipes every ground-truth Lisp through `mimir-cli parse` and reports 100/100 pass. Must run clean before every real benchmark run. |
| `run_benchmark_cc.py` | **Primary harness.** Dispatches each corpus prompt to the `claude` CLI in non-interactive mode (`claude -p --no-session-persistence ...`). Matches the production Claude-Code-over-MCP path. No API key required — uses whatever auth the operator's Claude CLI already has (OAuth / subscription / keychain). |
| `run_benchmark.py` | Independent cross-check harness. Calls the Anthropic messages API directly (SDK path). Requires `ANTHROPIC_API_KEY`. Useful for comparing "raw Claude" behaviour against "Claude-Code-wrapped Claude" on the same corpus. |
| `results/<ts>/` | Run outputs (gitignored). One dir per invocation, timestamped. Subdirectories prefixed `cc-` come from the CC harness; unprefixed come from the SDK harness. |

## Which harness to use

**Use `run_benchmark_cc.py` as the primary measurement.** Mimir's production path is Claude Code over MCP, not direct API calls. The CC harness measures fluency *on the same surface* that Mimir will actually be read-from and written-to in production; the SDK harness measures a surface Mimir does not use.

The SDK harness stays available because an independent cross-check — "does raw Claude do better or worse than Claude Code on the same corpus?" — is a useful datapoint when debugging a shape-specific failure. If both harnesses miss the 98% gate, the wire surface is the problem. If only one misses, the delta points at the surrounding environment (CLAUDE.md / skills / harness behaviour).

## Pre-flight (CC harness — no API key)

1. **Build `mimir-cli`.** Both harnesses shell out to it for parse checking.
   ```bash
   cargo build -p mimir-cli --release
   ```
2. **Sanity-check the corpus.** Should print `parsed : 100/100`.
   ```bash
   python3 research/llm-fluency/verify_corpus.py
   ```
3. **Confirm `claude` is on PATH** and authenticated (test with any trivial invocation, e.g. `claude -p "hello" < /dev/null`).
4. **Preview.** Prints the system prompt + first 3 corpus items; no CLI calls.
   ```bash
   python3 research/llm-fluency/run_benchmark_cc.py --dry-run
   ```

## Pre-flight (SDK harness — API key path, optional cross-check)

1. Build `mimir-cli` and sanity-check the corpus (same as steps 1–2 above).
2. **Install the Anthropic SDK.**
   ```bash
   pip install anthropic
   ```
3. **Set the API key** (this script reads ONLY from `ANTHROPIC_API_KEY`, not from `.env` / keychains / AWS profiles / anywhere else — explicit is the point).
   ```bash
   export ANTHROPIC_API_KEY=sk-ant-...
   ```
4. **Preview.**
   ```bash
   python3 research/llm-fluency/run_benchmark.py --dry-run
   ```

## Running — CC harness (primary)

**Full benchmark on the default model (Sonnet 4.6), 1 trial × 100 prompts = 100 CLI invocations, ~22 min wall-clock.**

```bash
python3 research/llm-fluency/run_benchmark_cc.py
```

**Smoke run (first N prompts only, no pass-rate gate).**

```bash
python3 research/llm-fluency/run_benchmark_cc.py --limit 10
```

**Per-model comparison.**

```bash
python3 research/llm-fluency/run_benchmark_cc.py --model claude-haiku-4-5
python3 research/llm-fluency/run_benchmark_cc.py --model claude-sonnet-4-6
python3 research/llm-fluency/run_benchmark_cc.py --model claude-opus-4-7
```

On a full run (no `--limit`), the script exits `0` if the rate clears the ≥98% bar and `1` if it doesn't. Partial runs (`--limit`) always exit `0` — they report but don't gate.

## Running — SDK harness (cross-check, optional)

**Full benchmark (default: Sonnet 4.6, 3 trials × 100 prompts = 300 API calls, ~$1 on Sonnet pricing).**

```bash
python3 research/llm-fluency/run_benchmark.py
```

**Quick smoke (1 trial, 100 calls, ~$0.30).**

```bash
python3 research/llm-fluency/run_benchmark.py --trials 1
```

Same exit-code contract as the CC harness — `0` on pass, `1` on miss.

## Output shape

```
results/2026-04-20T203012Z/
├── results.jsonl   # one record per (corpus_id, trial)
│                     {"id": "sem-01", "trial": 0, "response_lisp": "...",
│                      "parse_ok": true, "input_tokens": 412, ...}
├── summary.json    # aggregated metrics
│                     {"parse_ok_rate": 0.993, "met_exit_bar": true,
│                      "per_shape_pass_rate": {"sem": 1.0, "epi": 0.97, ...},
│                      "error_categories": {...}}
└── run.log         # per-call parse-check stdin + exit + stderr
```

## Writeup

Once a run clears the bar, the results get promoted to `docs/research/2026-MM-DD-llm-fluency-benchmark.md` with:

1. The run's `summary.json`.
2. The methodology (this README's contents, condensed).
3. Any interesting failure-mode analysis from `results.jsonl`.
4. A decision log: "wire surface verified" or "wire surface needs X".

Results directories (`research/llm-fluency/results/`) stay gitignored — they're re-producible from the committed corpus + harness. Only the writeup ships.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `verify_corpus` reports failures | Corpus edit broke a ground-truth entry | Revert the edit or fix the Lisp. Do NOT silence a failure — it means the benchmark would be measuring against an impossible target. |
| `run_benchmark: ANTHROPIC_API_KEY is not set` | Key isn't exported | `export ANTHROPIC_API_KEY=sk-ant-...` in the same shell. |
| Rate-limit / 429 errors | Anthropic quota tight | Drop to `--trials 1`, or add a sleep between calls (currently 50 ms — increase in the script). |
| Parse rate well below 98% | The real signal this benchmark is for | **Stop.** Check `summary.error_categories` and `run.log` for the dominant failure shape. Open an issue; do not edit the corpus. |
