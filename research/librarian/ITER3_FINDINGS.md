# Librarian iteration 3 — binder and semantic constraints in the prompt

Follow-up to iteration 2, which surfaced three distinct constraint failures the iteration-1 parse-check layer did not catch. Iteration 3 teaches the librarian those constraints via the system prompt and re-runs against the same real-drafts corpus.

## Prompt changes (see `run_librarian.py` `LIBRARIAN_SYSTEM_PROMPT`)

New section **"Binder and semantic constraints (CHECKED ON COMMIT)"** with three subsections:

1. **Source × memory-kind admitability** — explicit table mapping each `:src @X` symbol to the memory kinds it admits. `:src @policy` for `pro` only; `:src @observation` / `@profile` / `@document` / `@self_report` etc. per the grounding-model compatibility table in `crates/mimir-core/src/source_kind.rs`.
2. **Symbol-kind first-use locks** — explicit table of which slot in each form locks a symbol as Agent / Predicate / Literal / Memory / EventType / Scope. Rules to avoid reuse conflicts (`@mimir` as `:scp` vs `@mimir` as `sem` subject).
3. **Confidence × source bound** — table of `:c` ceilings per `:src` (Observation/Policy/LibrarianAssignment allow 1.0; Profile/Registry/AgentInstruction cap at 0.95; SelfReport/Document/ExternalAuthority cap at 0.9; ParticipantReport 0.85; PendingVerification 0.6).

No changes to `run_librarian.py` logic; only the `LIBRARIAN_SYSTEM_PROMPT` string.

## Parse-rate on real drafts (iteration 3, second run with confidence bounds added)

- **90 / 90 records parsed cleanly (100%).**
- 9 / 9 drafts produced ≥1 parseable record.
- Per-kind: 51 `sem`, 35 `pro`, 4 `epi`.
- Elapsed ~10 min (mean ~68 s per invocation; longer than iter 2 because the prompt is bigger).

The constraint additions visibly shifted the librarian's output:
- `@mimir` disappears as a sem subject — replaced with `@mimir_project` to avoid collision with `:scp @mimir`.
- `:src @agent_instruction :c 1.0` replaced with `:c 0.95` (respecting the bound).
- `:src @profile :c 1.0` replaced with `:c 0.95`.
- `:src @document :c 1.0` replaced with `:c 0.9`.
- sem records now use `@observation` / `@self_report` / `@profile` / `@document` / `@agent_instruction` per admitability; pro records use `@policy` / `@agent_instruction`.

## Full-batch commit — still not clean, but deeper

Attempting to commit the 90-record batch surfaced a **fifth constraint** not covered by the new prompt:

```
commit_failed: pipeline error: emit error:
  semantic supersession conflict at (s=SymbolId(7), p=SymbolId(107))
  valid_at=1776729600000ms:
  new memory has the same valid_at as existing memory SymbolId(198)
```

The batch had two `(sem @alain @owner_of ...)` records with the same `:v 2026-04-21` — one for `"mimir"`, one for `"bes_studios"`. The emit layer enforces **`(subject, predicate, valid_at)` uniqueness**: you cannot emit two records with the same `(s, p)` at the same `valid_at`; the second is interpreted as a supersession conflict.

## Deduplicated multi-record commit — works

After collapsing the two ownership records into one (`@alain @owns "mimir_and_bes_studios"`), a 9-record subset committed cleanly:

```
{"episode_id": "__ep_0", "committed_at": "2026-04-21T23:18:23Z"}
```

Retrieval round-tripped:

```
(query :s @alain) → 2 records back
(query :kind pro :limit 10) → 3 records back
```

Sanitisation boundary holds on retrieval — the `pro` rules come back as `pro` records with `:scp` set; sem facts come back as sem. A consumer agent sees instruction records and observation records as distinct shapes, never as a prose blob.

## The cumulative iteration map

| Iteration | Parse clean | Full-batch commit clean | Failure categories |
|---|---|---|---|
| 1 — synthetic drafts | 21 / 21 | (not attempted) | (in-prompt only: `:obs` keyword; prompt-injection resistance) |
| 2 — real drafts, existing prompt | 99 / 99 | 0 / 99 | Source × kind ; symbol-kind; (source × confidence bound surfaced on retry) |
| 3 — real drafts, constraint-aware prompt | 90 / 90 | partial | (s, p, valid_at) uniqueness surfaced; 9 / 9 subset succeeds |

Each iteration uncovered new rules; each prompt upgrade raised the parse-rate ceiling without helping the commit rate because the new rules were not yet in the prompt. The pattern:

> Prompt iteration can teach the librarian most static binder/semantic rules. Runtime rules like `(s, p, valid_at)` uniqueness require seeing other records in the batch — harder to encode in a prompt alone.

## Where this goes next

The `(s, p, valid_at)` uniqueness (and likely more constraints still undiscovered) argues for the **pre-emit validator** path from iteration 2's findings, not further prompt expansion:

- Spawn `mimir-mcp` against a scratch workspace per draft.
- For each librarian-emitted record, attempt a single-record write.
- On failure: categorise the error; feed it back to the librarian as "this record failed with <error>; emit a fix."
- Retry N times before giving up on a record.
- Accumulate successful records across the draft; commit the successful subset at the end.

This is substantially more engineering than a prompt change, and it belongs in iteration 4, not here.

## Honest summary of where we are

- **Parse-rate on real drafts with a constraint-aware librarian: 100%.** The librarian produces syntactically valid canonical Lisp for real prose memory content.
- **Single-record and small-batch commits work end-to-end** through the full MCP surface.
- **Arbitrary real-content batches do not yet commit cleanly** because of cross-record constraints (`(s, p, valid_at)` uniqueness, and likely others).
- **The next architectural move is a pre-emit validation loop**, not another prompt version.

## Files

- `run_librarian.py` — prompt updated with three new constraint subsections.
- `ITER3_FINDINGS.md` — this file.
- `results/20260421T194507Z/` — the 90/90 real-drafts run (gitignored).

No change to `sample_drafts.jsonl` or `real_drafts.jsonl`.
