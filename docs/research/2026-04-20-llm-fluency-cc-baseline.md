# LLM-fluency baseline — Claude Code harness, Sonnet 4.6

> **⚠ Framing update (2026-04-21):** this writeup was produced under the original "agent emits Lisp directly, ≥98% parse-rate is existential for v0.1.0" framing. That framing was superseded the same day by the architecture pivot ([`../../../.claude/.../memory/project_librarian_pivot.md`](../../.claude) / see `project_librarian_pivot` memory): **agents no longer emit canonical Lisp; the librarian does.** The 74% measurement here remains accurate for what it measured — Claude Sonnet 4.6's **native** Lisp-emission fluency with 5-shot prompting — but that metric is no longer a gate on v0.1.0. The relevant successor measurement is **librarian emit + commit rate**, tracked separately under the librarian iterations. This document stands as historical data about base-model Lisp fluency; interpret accordingly.
>
> **Run date:** 2026-04-20
> **Model:** `claude-sonnet-4-6`
> **Harness:** [`research/llm-fluency/run_benchmark_cc.py`](../../research/llm-fluency/run_benchmark_cc.py) — dispatches via `claude -p` CLI, no API key, matches production Claude-Code-over-MCP surface.
> **Corpus:** [`research/llm-fluency/corpus.jsonl`](../../research/llm-fluency/corpus.jsonl), 100 prompts (25 each `sem` / `epi` / `pro` / `query`).
> **Few-shot:** [`research/llm-fluency/few_shot_examples.jsonl`](../../research/llm-fluency/few_shot_examples.jsonl), 5 exemplars.
> **Trials:** 1 per prompt.
> **Wall-clock:** 24 min 36 s (mean 14.8 s per CLI invocation).
> **Exit code:** 1 — missed the (now-obsolete) ≥98% gate.

## Headline

**Overall: 74/100 = 74.0%.** Misses the roadmap's ≥98% parse-rate bar set in [`docs/planning/2026-04-19-roadmap-to-prime-time.md`](../planning/2026-04-19-roadmap-to-prime-time.md) § Phase 3.2.

| Shape | Pass | Total | Rate | Notes |
|---|---|---|---|---|
| `epi` | 24 | 25 | 96.0% | At or near the gate. One unsupported-keyword failure (`:end`). |
| `pro` | 23 | 25 | 92.0% | One Claude-Code safety refusal; one query-keyword leak. |
| `sem` | 23 | 25 | 92.0% | One shape-confusion (`sem` → `query`); one unsupported-keyword (`:until`). |
| **`query`** | **4** | **25** | **16.0%** | **Dominant failure mode.** Not a Claude-fluency issue — a wire-surface discoverability gap. |

Error categories: `ok` 74, `unexpected_token` 20, `lex_error` 6.

## What the query cliff actually is

21 of 25 query prompts fail with `BadKeyword` on one of: `:subj`, `:subject`, `:predicate`, `:object`, `:about`.

The parser accepts [`crates/mimir-core/src/parse.rs`](../../crates/mimir-core/src/parse.rs) L893–915:

```
kind, s, p, o, in_episode, after_episode, before_episode, episode_chain,
as_of, as_committed, include_retired, include_projected,
confidence_threshold, limit, explain_filtered, show_framing,
debug_mode, read_after, timeout_ms
```

Claude Code defaults to `:subject` / `:predicate` / `:object` (full English) or `:subj` (a reasonable mid-length abbreviation). The grammar uses single-char `:s` / `:p` / `:o`. The few-shot corpus never exercises a subject-filtered query — only `(query :kind pro)` appears — so Claude has no worked example of the compressed form. Result: it reaches for its natural vocabulary and the parser rejects every attempt.

This is the benchmark working as designed. The ≥98% gate caught a wire-surface issue *before* v0.1.0 locks it.

Representative failures:

```
qry-01  Find everything Alice knows.
  → (query :subject @alice)
  → unexpected keyword "subject" for form "query"

qry-02  List semantic memories with Bob as the subject.
  → (query :kind sem :subj @bob)
  → unexpected keyword "subj" for form "query"

qry-04  Find all semantic memories about Alice's email.
  → (query :kind sem :subject @alice :predicate @email)
  → unexpected keyword "subject" for form "query"
```

## Non-query failures (5 total, all instructive)

- **`sem-10`** — "The database is currently in read-only mode." → `(query :mode :read-only)`. Two layers of failure: shape confusion (sem vs query) and a hyphenated keyword (`:read-only`) that the lexer parses as `:read - only`. Prompt is ambiguous — "read-only mode" reads as a state declaration (sem) or a query-mode predicate (query) depending on frame.
- **`sem-16`** — "Dan is on vacation until 2024-05-30." → emitted an `epi` with `:until`. Claude modelled a vacation as an event with an end date; the `epi` form's time keyword is `:at` not `:until`. Shape choice defensible; keyword unknown.
- **`epi-16`** — weekly standup from 09:00 to 09:30. Emitted `:at` + `:end`. The `epi` form accepts `:at` but not `:end`. Claude naturally reaches for "start + end" terminology; the schema only provides a start.
- **`pro-09`** — "When a session starts, open the workspace and begin a new episode." Claude refused the prompt: *"I need permission to use the Mimir MCP tools..."* — a Claude Code safety check firing because the prompt's imperative language looked like a tool-invocation request. The system prompt ("emit a single canonical Lisp form") didn't override it. Environment artefact of the CC harness, not a model-fluency fail.
- **`pro-13`** — JSON-format data-return rule. Emitted a `query` with `:subj` / `:pred`. Shape confusion + query-keyword leak.

## Interpretation

Claude is **fluent on write forms** (`sem`, `epi`, `pro` all ≥ 92%). The wire surface for writes is discoverable via the 5 few-shot exemplars alone. Write-side v0.1.0 is probably close to the 98% bar with modest corpus / few-shot tightening.

Claude is **not fluent on the query form** (16%), but the miss is concentrated in two issues, both fixable:

1. **Query keyword vocabulary.** The grammar uses single-char `:s / :p / :o`. Claude reaches for `:subject / :predicate / :object` (or `:subj / :pred / :obj`). Three design responses, not ranked here:
   - Expand the parser to accept aliases (`:subj → :s`, `:subject → :s`, etc.). Cheap, backward-compatible, lossy-in-token-density.
   - Change the canonical form to the longer keywords and drop the single-char form. Breaking change to the wire surface; the parser is barely v0.1.0-alpha, so cost is mostly writing the migration.
   - Keep the single-char form and rely on few-shot examples / a Mimir Skill to teach it. Lowest parser change, highest in-session token cost per agent-briefing.
2. **Query few-shot coverage.** The committed few-shot has one query exemplar: `(query :kind pro)`. It never shows a subject-filtered or predicate-filtered query. Any solution path above benefits from few-shot coverage of the subject/predicate-filtered query cases. This is NOT the corpus (locked); it is the **few-shot file**, which is a separate artefact and therefore amendable without p-hacking the corpus.

The write-side failures (`sem-10`, `sem-16`, `epi-16`, `pro-13`) cluster around **keywords for temporal / state / end-of-duration semantics** that Claude naturally expects (`:until`, `:end`, `:mode`) but the schema doesn't expose. Whether to expand the schema or to accept these as "won't fix, close-enough pass rate with current corpus" is a design decision downstream of the query question.

## What this does NOT say

- It does **not** say Claude is bad at emitting Mimir Lisp. On write forms Claude is ≥ 92% fluent with only five exemplars. That's a strong baseline.
- It does **not** say the corpus is wrong. The corpus is locked; running the benchmark *is* the point of locking it. The signal goes to the grammar or the few-shot, not to the corpus.
- It does **not** gate downstream work by itself. Deliverable C (Mode 1 client integration) and D (public-readiness pass) can proceed in parallel; they don't depend on the query-form decision being made first.

## Reproducing this run

```
cargo build -p mimir-cli --release
python3 research/llm-fluency/verify_corpus.py         # must print 100/100
python3 research/llm-fluency/run_benchmark_cc.py
```

Output at `research/llm-fluency/results/cc-<timestamp>/`. Results directories are gitignored; the committed artefact is this writeup.

## Next steps (for operator decision, not executed here)

1. **Run Opus 4.7 baseline.** Same harness, different model. ~25 min more wall-clock. If Opus meets 98% with no grammar changes, the issue is model-specific and resolves with a Skill briefing. If Opus also fails on query, the grammar or few-shot needs work.
2. **Decide the query-keyword question.** The three options above. Do not edit the corpus; the decision belongs at the grammar or few-shot layer.
3. **If few-shot is the chosen fix**, add 2–3 query exemplars that exercise `:s` / `:p` / `:o` explicitly, re-run CC + SDK harnesses, verify the delta.
4. **If grammar expansion is the chosen fix**, decide whether to accept keyword aliases or switch canonical forms entirely. Either way, tests to cover the expanded surface.

This writeup lands as the honest baseline. Course-correction is a separate PR.
