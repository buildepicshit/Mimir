# Librarian iteration 2 — real drafts + MCP write path

Follow-up to iteration 1 (`run_librarian.py` + synthetic `sample_drafts.jsonl`, 21/21 parse rate). Iteration 2 swaps synthetic drafts for real auto-memory content and exercises the actual MCP write path, not just parse-check.

## What changed

- **`real_drafts.jsonl`** — 9 prose drafts extracted from the operator's Claude auto-memory directory (`~/.claude/projects/.../memory/*.md`, frontmatter stripped). Sizes range ~800–3500 chars each. Represents real lived memory content, not synthetic cases.
- **No script changes** to `run_librarian.py`. Same harness, different input.
- **MCP write exercise** — manually drove `mimir_open_workspace → mimir_write → mimir_read → mimir_release_workspace` against a fresh workspace at `/tmp/mimir-librarian-iter2.log` using the librarian's output as the batch.

## Parse-rate result on real drafts

- 9 / 9 drafts produced at least one parseable record.
- 99 / 99 emitted records parsed cleanly (**100%**).
- Per-kind: 62 `sem`, 35 `pro`, 2 `epi`.
- Elapsed ~6 min (mean ~42 s per invocation; slower than synthetic because real drafts are denser).

Sanitisation discipline held. Mixed drafts (e.g. `feedback_ci_quota.md` — "CI quota is a hard rule; here's the full local gate to run before every push") correctly split into separate `sem` observations ("budget burned through 2026-04-20") and `pro` rules ("run the full cargo gate before push"). No directive was blobbed into an observation or vice versa.

## MCP-write findings — parse clean ≠ commit clean

Attempted to commit the full 99-record batch. The first three attempts each failed with a different kind / source / semantic constraint:

### 1. Symbol-kind consistency (binder error)

```
commit_failed: pipeline error: bind error:
  symbol kind mismatch for "mimir": expected Agent, locked as Scope
```

`@mimir` appeared early in the batch as `:scp @mimir` in `pro` records — the binder locked its symbol kind as `Scope`. Later records used `(sem @mimir ...)` where `@mimir` is the subject (Agent kind). First-use wins; the later records can't re-type the symbol.

### 2. Object-vs-subject position (binder error)

```
commit_failed: pipeline error: bind error:
  symbol kind mismatch for "bes_studios": expected Agent, locked as Literal
```

`@bes_studios` appeared first as the *object* of a sem (`(sem @alain @owner_of @bes_studios ...)`), locking it as `Literal`. Later records used it as a *subject* (`(sem @bes_studios @github_handle ...)`), which expects `Agent` kind.

### 3. Source × memory-kind matrix (semantic error)

```
commit_failed: pipeline error: semantic error:
  source kind Policy does not admit memory kind Semantic
```

`:src @policy` is not admitted for `sem` memories — there is a source × memory-kind compatibility matrix. Several of the librarian's outputs used `:src @policy` on `sem` records, inherited from the few-shot examples.

## Single-record commit works

After whittling down to one conservative record:

```
(sem @alain_user @prefers_response_style "terse" :src @observation :c 0.9 :v 2026-04-21)
```

Commit succeeded cleanly:

```json
{"episode_id": "__ep_0", "committed_at": "2026-04-21T17:13:40Z"}
```

`mimir_read :s @alain_user` returned the record round-tripped. Write path works end-to-end at the MCP surface.

## What iteration 2 actually proves

- The `mimir_open_workspace → mimir_write → mimir_read → mimir_release_workspace` MCP loop is functional.
- Parse-check via `mimir-cli parse` is **necessary but not sufficient**. It verifies syntax; the binder + semantic layers have additional constraints:
  - **Symbol-kind first-use-wins** across the batch.
  - **Object-vs-subject position pins symbol kind.**
  - **Source × memory-kind compatibility matrix.**
- The librarian system prompt (iteration 1) teaches syntactic correctness only. To hit high commit-rate on real drafts, iteration 3 must extend the prompt to cover the binder + semantic constraints, or the pre-emit step must validate against them.

## Next iteration (iteration 3)

Two directions, not prescribed:

1. **Extend the librarian prompt** with explicit rules: (a) use distinct symbols for scope vs subject vs object; (b) prefer string literals for object slots rather than named symbols; (c) choose `:src` values compatible with each memory kind (document the matrix in the prompt). Re-run on real drafts; measure commit-rate.

2. **Add a pre-emit validation layer** on the Python side — spawn `mimir-mcp` in a scratch workspace, attempt each record, categorize failures, feed them back to the librarian as "your record failed the binder; retry with this constraint." This is heavier but more robust.

Both are real paths; both belong in iteration 3, not here.

## Files

- `real_drafts.jsonl` — 9 drafts, real auto-memory content.
- `results/20260421T164836Z/` — 99/99 parse-rate run (gitignored, reproducible).
- `ITER2_FINDINGS.md` — this file.

## How to reproduce the MCP write exercise

1. Start a Mimir MCP server (or use the operator's configured one).
2. Open a fresh workspace at a disposable path: `mimir_open_workspace(log_path="/tmp/mimir-librarian-iter2.log")` → get lease token.
3. Take any single-record line from a results.jsonl file, pass as `batch` to `mimir_write` with the lease token.
4. Attempt to commit a multi-record batch to observe the kind / source constraints firsthand.
5. `mimir_release_workspace` when done.
