#!/usr/bin/env python3
"""Phase 3.2 LLM-fluency benchmark — Claude Code variant.

Same corpus + few-shot + parse-check pipeline as `run_benchmark.py`,
but dispatches to the `claude` CLI in non-interactive mode
(`claude -p --no-session-persistence --system-prompt ... <prompt>`)
instead of calling the Anthropic messages API directly.

Why this exists: the production path for Mimir is Claude Code over
MCP, not direct API calls. Measuring fluency via Claude Code is the
surface-matched measurement; the SDK harness (`run_benchmark.py`)
remains available as an independent cross-check.

Requires: `claude` CLI on PATH (Claude Code). No `ANTHROPIC_API_KEY`
is needed — the CLI uses whatever auth the operator already has
(OAuth / subscription / keychain, per the CLI's normal path).

Usage:
    python3 run_benchmark_cc.py                        # full 100 prompts, 1 trial
    python3 run_benchmark_cc.py --limit 10             # first 10 prompts only (smoke)
    python3 run_benchmark_cc.py --model claude-opus-4-7
    python3 run_benchmark_cc.py --dry-run              # print prompts and exit

Output: same shape as run_benchmark.py — `results/<ts>/` containing
`results.jsonl`, `summary.json`, `run.log`.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path


HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parents[1]
DEFAULT_CORPUS = HERE / "corpus.jsonl"
DEFAULT_FEWSHOT = HERE / "few_shot_examples.jsonl"
RESULTS_ROOT = HERE / "results"

DEFAULT_MODEL = "claude-sonnet-4-6"
# Per-invocation timeout. A single claude -p call has been observed at
# 10-20s for short prompts; 90s is a comfortable ceiling that still
# catches runaway requests.
INVOCATION_TIMEOUT_S = 90

SYSTEM_PROMPT_HEAD = (
    "You are an agent emitting Mimir Lisp write-surface forms. Mimir is an "
    "agent-first memory system with a canonical Lisp write surface. "
    "Given a natural-language request, emit a SINGLE canonical Lisp form "
    "(sem / epi / pro / query) that represents it. "
    "Output ONLY the Lisp form — no prose, no backticks, no explanation, no "
    "leading/trailing whitespace beyond the form itself."
)


def load_jsonl(path: Path) -> list[dict]:
    with path.open(encoding="utf-8") as f:
        return [json.loads(line) for line in f if line.strip()]


def locate_bin(explicit: str | None) -> Path:
    if explicit:
        p = Path(explicit).resolve()
        if not p.is_file():
            sys.exit(f"run_benchmark_cc: --bin does not exist: {p}")
        return p
    for profile in ("release", "debug"):
        candidate = REPO_ROOT / "target" / profile / "mimir-cli"
        if candidate.is_file():
            return candidate
    sys.exit(
        "run_benchmark_cc: no built mimir-cli found.\n"
        "  Build: `cargo build -p mimir-cli --release` or `cargo build -p mimir-cli`."
    )


def build_system_prompt(few_shot: list[dict]) -> str:
    lines = [SYSTEM_PROMPT_HEAD, "", "Examples:"]
    for ex in few_shot:
        lines.append(f"Request: {ex['prompt_en']}")
        lines.append(f"Response: {ex['lisp']}")
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def invoke_claude(model: str, system_prompt: str, prompt_en: str) -> tuple[str, int, str]:
    """Run `claude -p` once; return (stdout, return_code, stderr_tail).

    stderr tail is truncated to keep log lines bounded.
    """
    cmd = [
        "claude",
        "-p",
        "--no-session-persistence",
        "--model",
        model,
        "--system-prompt",
        system_prompt,
        prompt_en,
    ]
    try:
        result = subprocess.run(
            cmd,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            timeout=INVOCATION_TIMEOUT_S,
            check=False,
        )
    except subprocess.TimeoutExpired:
        return "", 124, "timeout"
    stderr_tail = (result.stderr or "").strip()[-400:]
    return (result.stdout or "").strip(), result.returncode, stderr_tail


def parse_check(bin_path: Path, lisp: str) -> tuple[int, str]:
    result = subprocess.run(
        [str(bin_path), "parse"],
        input=lisp,
        capture_output=True,
        text=True,
        check=False,
    )
    return result.returncode, result.stderr.strip()


def classify_parse_error(stderr: str) -> str:
    """Coarse error-category bucket for summary stats."""
    if not stderr:
        return "ok"
    s = stderr.lower()
    if "lex" in s:
        return "lex_error"
    if "unexpected" in s:
        return "unexpected_token"
    if "eof" in s:
        return "unexpected_eof"
    return "other_parse_error"


def run(args) -> int:
    corpus_all = load_jsonl(args.corpus)
    few_shot = load_jsonl(args.few_shot)

    corpus = corpus_all if args.limit is None else corpus_all[: args.limit]

    system_prompt = build_system_prompt(few_shot)

    if args.dry_run:
        print("=== SYSTEM PROMPT ===")
        print(system_prompt)
        print("=== CORPUS (first 3) ===")
        for item in corpus[:3]:
            print(json.dumps(item, indent=2))
        print(f"=== Would invoke claude -p for {len(corpus)} prompts ===")
        return 0

    bin_path = locate_bin(args.bin)

    timestamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    out_dir = RESULTS_ROOT / f"cc-{timestamp}"
    out_dir.mkdir(parents=True, exist_ok=True)

    results_path = out_dir / "results.jsonl"
    log_path = out_dir / "run.log"
    summary_path = out_dir / "summary.json"

    per_shape: Counter = Counter()
    per_shape_ok: Counter = Counter()
    error_categories: Counter = Counter()

    start = time.time()
    ok = 0
    total = 0

    with results_path.open("w", encoding="utf-8") as res_f, log_path.open(
        "w", encoding="utf-8"
    ) as log_f:
        for i, item in enumerate(corpus, 1):
            prompt_en = item["prompt_en"]
            shape = item["shape"]
            corpus_id = item["id"]

            t0 = time.time()
            stdout, rc, stderr_tail = invoke_claude(args.model, system_prompt, prompt_en)
            invoke_s = time.time() - t0

            if rc != 0 or not stdout:
                parse_rc = -1
                parse_err = f"claude_invoke_failed rc={rc} err={stderr_tail}"
                error_category = "claude_failed"
            else:
                parse_rc, parse_err = parse_check(bin_path, stdout)
                error_category = classify_parse_error(parse_err) if parse_rc != 0 else "ok"

            passed = rc == 0 and parse_rc == 0
            if passed:
                ok += 1
                per_shape_ok[shape] += 1

            per_shape[shape] += 1
            error_categories[error_category] += 1
            total += 1

            res_f.write(
                json.dumps(
                    {
                        "corpus_id": corpus_id,
                        "shape": shape,
                        "prompt_en": prompt_en,
                        "response": stdout,
                        "claude_rc": rc,
                        "parse_rc": parse_rc,
                        "parse_err": parse_err,
                        "passed": passed,
                        "invoke_s": round(invoke_s, 2),
                    }
                )
                + "\n"
            )
            res_f.flush()

            log_f.write(
                f"[{i}/{len(corpus)}] {corpus_id} shape={shape} "
                f"invoke_s={invoke_s:.1f} passed={passed} "
                f"parse_rc={parse_rc}\n"
            )
            if not passed:
                log_f.write(f"  prompt: {prompt_en}\n")
                log_f.write(f"  response: {stdout!r}\n")
                log_f.write(f"  parse_err: {parse_err}\n")
            log_f.flush()

            print(
                f"[{i:3}/{len(corpus)}] {corpus_id:10} "
                f"{shape:5} {'PASS' if passed else 'FAIL':4} "
                f"({invoke_s:5.1f}s)"
            )

    elapsed = time.time() - start
    overall_rate = ok / total if total else 0.0
    per_shape_rates = {
        s: (per_shape_ok[s] / per_shape[s] if per_shape[s] else 0.0) for s in per_shape
    }

    summary = {
        "timestamp_utc": timestamp,
        "model": args.model,
        "harness": "claude-code-cli",
        "corpus_size": len(corpus),
        "total": total,
        "ok": ok,
        "overall_rate": overall_rate,
        "per_shape_totals": dict(per_shape),
        "per_shape_ok": dict(per_shape_ok),
        "per_shape_rates": per_shape_rates,
        "error_categories": dict(error_categories),
        "elapsed_s": round(elapsed, 1),
        "mean_invoke_s": round(elapsed / total, 2) if total else 0.0,
    }
    summary_path.write_text(json.dumps(summary, indent=2))

    print()
    print(f"=== SUMMARY ({out_dir.name}) ===")
    print(f"Overall: {ok}/{total} = {overall_rate:.1%}")
    for s, r in sorted(per_shape_rates.items()):
        print(f"  {s:5} {per_shape_ok[s]}/{per_shape[s]} = {r:.1%}")
    print(f"Errors: {dict(error_categories)}")
    print(f"Elapsed: {elapsed:.1f}s (mean {elapsed/total:.1f}s per invocation)")

    # Exit 1 if we ran the full corpus and missed the 98% gate.
    # Partial runs (--limit) don't gate on the rate, only report.
    if args.limit is None and overall_rate < 0.98:
        return 1
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description="Claude-Code-based LLM-fluency benchmark.")
    ap.add_argument("--corpus", type=Path, default=DEFAULT_CORPUS)
    ap.add_argument("--few-shot", type=Path, default=DEFAULT_FEWSHOT)
    ap.add_argument("--model", default=DEFAULT_MODEL)
    ap.add_argument("--limit", type=int, default=None, help="Run only the first N corpus items")
    ap.add_argument("--dry-run", action="store_true", help="Print prompts and exit")
    ap.add_argument("--bin", help="Path to mimir-cli binary (default: target/{release,debug}/mimir-cli)")
    args = ap.parse_args()
    return run(args)


if __name__ == "__main__":
    sys.exit(main())
