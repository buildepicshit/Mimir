#!/usr/bin/env python3
"""Phase 3.2 LLM-fluency benchmark harness — the existential-gate runner.

Drives the corpus through Claude and measures parse-success rate on
the resulting Lisp. The ≥98% exit bar is set by
`docs/planning/2026-04-19-roadmap-to-prime-time.md` § Phase 3.2: if
Claude can't natively emit parseable Mimir Lisp at that rate, the
wire surface needs course-correction before v0.1.0 locks it into a
published API.

## Lockdown discipline

The corpus, few-shot exemplars, and prompting strategy are committed
artifacts. This script does NOT mutate them. If a run surfaces an
unexpected failure pattern (e.g. 40% of pro-* fail with the same
predicate shape), the correct response is to stop, open an issue,
discuss whether the corpus is unfair or the wire surface is wrong —
not to edit the corpus and re-run. Corpus p-hacking invalidates the
gate.

## Usage

    pip install anthropic
    export ANTHROPIC_API_KEY=sk-ant-...
    python3 run_benchmark.py                     # default: Sonnet 4.6, 3 trials per prompt
    python3 run_benchmark.py --model opus-4-7    # override model
    python3 run_benchmark.py --trials 1          # quick smoke (100 calls instead of 300)
    python3 run_benchmark.py --dry-run           # print prompts + exit; no API calls

## Cost estimate

300 calls × ~500 input tokens × Sonnet 4.6 pricing ≈ $1. Opus 4.7
is roughly 5× that. See `--dry-run` before any real run to inspect
the prompts and budget.

## Output

results/<timestamp>/
    results.jsonl    — one record per (corpus_id, trial) with the raw
                       response and parse-check outcome
    summary.json     — aggregated metrics (per-shape pass rates,
                       overall rate, error distribution)
    run.log          — stdin to `mimir-cli parse` for every call +
                       its exit code + stderr

Summarize / visualize via `summarize.py` (separate script, reads
results.jsonl only).
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path

try:
    import anthropic  # type: ignore[import-not-found]
except ImportError:
    anthropic = None  # deferred: only imported when running (not for --dry-run)


HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parents[1]
DEFAULT_CORPUS = HERE / "corpus.jsonl"
DEFAULT_FEWSHOT = HERE / "few_shot_examples.jsonl"
RESULTS_ROOT = HERE / "results"

# Default model. Sonnet 4.6 is the roadmap's "current default" choice:
# balanced cost + parse-rate expectation. Override with --model if a
# per-model comparison run is desired.
DEFAULT_MODEL = "claude-sonnet-4-6"

# Per-prompt trial count. 3 trials × 100 prompts = 300 API calls.
DEFAULT_TRIALS = 3

# Anthropic messages API: max tokens to request in the response. The
# corpus entries are all single-line Lisp forms; 256 is comfortably
# above the longest epi form.
MAX_OUTPUT_TOKENS = 256

# System-prompt text. Kept terse + instruction-heavy to minimize
# preamble tokens and eliminate chitchat in the response.
SYSTEM_PROMPT = (
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
            sys.exit(f"run_benchmark: --bin does not exist: {p}")
        return p
    for profile in ("release", "debug"):
        candidate = REPO_ROOT / "target" / profile / "mimir-cli"
        if candidate.is_file():
            return candidate
    sys.exit(
        "run_benchmark: no built mimir-cli found.\n"
        "  Build: `cargo build -p mimir-cli --release` (recommended) or "
        "`cargo build -p mimir-cli`."
    )


def build_messages(few_shot: list[dict], prompt_en: str) -> list[dict]:
    """Construct the messages array for the Anthropic messages API.

    Few-shot examples are threaded as alternating user / assistant turns,
    which Claude handles natively and which matches the production
    integration pattern (agents see worked examples in the hook docs).
    """
    messages: list[dict] = []
    for ex in few_shot:
        messages.append({"role": "user", "content": ex["prompt_en"]})
        messages.append({"role": "assistant", "content": ex["lisp"]})
    messages.append({"role": "user", "content": prompt_en})
    return messages


def parse_check(bin_path: Path, lisp: str) -> tuple[int, str]:
    result = subprocess.run(
        [str(bin_path), "parse"],
        input=lisp,
        capture_output=True,
        text=True,
        check=False,
    )
    return result.returncode, result.stderr.strip()


def extract_lisp(content_blocks: list[dict]) -> str:
    """Concatenate the text content blocks into a single string."""
    pieces: list[str] = []
    for block in content_blocks:
        # SDK returns TextBlock-like objects; unify on dict access.
        if isinstance(block, dict):
            if block.get("type") == "text":
                pieces.append(block.get("text", ""))
        else:
            if getattr(block, "type", None) == "text":
                pieces.append(getattr(block, "text", ""))
    return "".join(pieces).strip()


def run(
    corpus: list[dict],
    few_shot: list[dict],
    client,
    model: str,
    trials: int,
    bin_path: Path,
    out_dir: Path,
) -> dict:
    results_path = out_dir / "results.jsonl"
    log_path = out_dir / "run.log"
    per_shape: Counter = Counter()
    per_shape_ok: Counter = Counter()
    error_categories: Counter = Counter()
    total = 0
    ok = 0

    with results_path.open("w", encoding="utf-8") as results_f, log_path.open(
        "w", encoding="utf-8"
    ) as log_f:
        for entry in corpus:
            prompt_en = entry["prompt_en"]
            shape = entry["shape"]
            for trial in range(trials):
                total += 1
                per_shape[shape] += 1
                messages = build_messages(few_shot, prompt_en)
                try:
                    response = client.messages.create(
                        model=model,
                        max_tokens=MAX_OUTPUT_TOKENS,
                        system=SYSTEM_PROMPT,
                        messages=messages,
                    )
                except Exception as e:  # noqa: BLE001  — surface the raw failure into results
                    error_categories["api_error"] += 1
                    record = {
                        "id": entry["id"],
                        "shape": shape,
                        "trial": trial,
                        "error": f"api_error: {e}",
                        "parse_ok": False,
                    }
                    results_f.write(json.dumps(record) + "\n")
                    log_f.write(f"[{entry['id']}#{trial}] api_error: {e}\n")
                    continue

                lisp = extract_lisp(
                    response.content if not isinstance(response.content, list) else response.content  # type: ignore[arg-type]
                )
                parse_code, parse_stderr = parse_check(bin_path, lisp)
                parse_ok = parse_code == 0

                record = {
                    "id": entry["id"],
                    "shape": shape,
                    "trial": trial,
                    "prompt_en": prompt_en,
                    "response_lisp": lisp,
                    "parse_ok": parse_ok,
                    "parse_exit_code": parse_code,
                    "parse_stderr": parse_stderr if not parse_ok else "",
                    "input_tokens": getattr(response.usage, "input_tokens", None),
                    "output_tokens": getattr(response.usage, "output_tokens", None),
                }
                results_f.write(json.dumps(record) + "\n")
                log_f.write(
                    f"[{entry['id']}#{trial}] parse_ok={parse_ok} exit={parse_code}\n"
                    f"  lisp: {lisp}\n"
                    f"  stderr: {parse_stderr}\n"
                )

                if parse_ok:
                    ok += 1
                    per_shape_ok[shape] += 1
                else:
                    # Categorize by the parser error prefix (e.g. "parse error: unexpected...")
                    category = parse_stderr.split(":", 2)[0:2]
                    key = ":".join(c.strip() for c in category) or "other"
                    error_categories[key] += 1

                # Gentle pacing to avoid a rate-limit storm on a long
                # run. Anthropic Sonnet's default rate limit is high
                # enough that this is more courtesy than necessity.
                time.sleep(0.05)

    summary = {
        "model": model,
        "trials_per_prompt": trials,
        "total_calls": total,
        "parse_ok": ok,
        "parse_fail": total - ok,
        "parse_ok_rate": ok / total if total else 0.0,
        "per_shape_pass_rate": {
            shape: per_shape_ok[shape] / count
            for shape, count in per_shape.items()
            if count
        },
        "error_categories": dict(error_categories),
        "exit_bar": 0.98,
        "met_exit_bar": (ok / total if total else 0.0) >= 0.98,
    }
    with (out_dir / "summary.json").open("w", encoding="utf-8") as f:
        json.dump(summary, f, indent=2, sort_keys=True)
    return summary


def dry_run(corpus: list[dict], few_shot: list[dict]) -> None:
    sample = corpus[0]
    messages = build_messages(few_shot, sample["prompt_en"])
    print("=== System prompt ===")
    print(SYSTEM_PROMPT)
    print("\n=== Few-shot turns (5 examples) ===")
    for msg in messages[:-1]:
        print(f"[{msg['role']}] {msg['content']}")
    print("\n=== Query turn (first corpus entry) ===")
    print(f"[user] {sample['prompt_en']}")
    print(f"\n=== Expected ground truth (not sent) ===")
    print(sample["ground_truth_lisp"])
    print(f"\nCorpus size   : {len(corpus)} prompts")
    print(f"Per-call cost : estimate ~$0.003 (Sonnet 4.6)")
    print(f"Total (3x)    : estimate ~$0.90")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--model", default=DEFAULT_MODEL, help=f"Anthropic model (default: {DEFAULT_MODEL})")
    ap.add_argument("--trials", type=int, default=DEFAULT_TRIALS, help=f"Trials per prompt (default: {DEFAULT_TRIALS})")
    ap.add_argument("--corpus", default=str(DEFAULT_CORPUS))
    ap.add_argument("--fewshot", default=str(DEFAULT_FEWSHOT))
    ap.add_argument("--bin", help="Path to mimir-cli binary (auto-detected if omitted)")
    ap.add_argument("--dry-run", action="store_true", help="Print a sample prompt + exit; no API calls")
    args = ap.parse_args()

    corpus = load_jsonl(Path(args.corpus))
    few_shot = load_jsonl(Path(args.fewshot))
    if not corpus:
        sys.exit("run_benchmark: corpus is empty")
    if not few_shot:
        sys.exit("run_benchmark: few-shot file is empty")

    if args.dry_run:
        dry_run(corpus, few_shot)
        return 0

    if anthropic is None:
        sys.exit(
            "run_benchmark: the `anthropic` SDK is not installed. Run\n"
            "    pip install anthropic\n"
            "then re-invoke."
        )
    if not os.environ.get("ANTHROPIC_API_KEY"):
        sys.exit(
            "run_benchmark: ANTHROPIC_API_KEY is not set.\n"
            "    export ANTHROPIC_API_KEY=sk-ant-...\n"
            "then re-invoke. This script will NOT read keys from any other source."
        )

    bin_path = locate_bin(args.bin)

    timestamp = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H%M%SZ")
    out_dir = RESULTS_ROOT / timestamp
    out_dir.mkdir(parents=True, exist_ok=True)

    client = anthropic.Anthropic()
    print(f"model     : {args.model}")
    print(f"trials    : {args.trials} per prompt")
    print(f"corpus    : {len(corpus)} prompts")
    print(f"total     : {len(corpus) * args.trials} API calls")
    print(f"output    : {out_dir}")
    print(f"bin       : {bin_path}")
    print()

    start = time.monotonic()
    summary = run(corpus, few_shot, client, args.model, args.trials, bin_path, out_dir)
    elapsed = time.monotonic() - start

    print(f"\n=== Summary ({elapsed:.1f}s) ===")
    print(json.dumps(summary, indent=2, sort_keys=True))
    exit_bar_met = summary["met_exit_bar"]
    print(f"\nResult: exit bar ≥0.98 {'MET' if exit_bar_met else 'NOT MET'}.")
    if not exit_bar_met:
        print(
            "The wire surface may need course-correction before v0.1.0. "
            "Do NOT edit the corpus. Open an issue with the `parse_stderr` "
            "distribution from summary.json and discuss.",
            file=sys.stderr,
        )
    return 0 if exit_bar_met else 1


if __name__ == "__main__":
    raise SystemExit(main())
