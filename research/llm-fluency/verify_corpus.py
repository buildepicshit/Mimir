#!/usr/bin/env python3
"""Verify the Phase 3.2 LLM-fluency corpus parses cleanly via `mimir-cli parse`.

This is NOT a benchmark. It's a sanity check that every ground-truth
Lisp answer in corpus.jsonl is syntactically valid Mimir. It must run
to 100/100 pass BEFORE the benchmark PR merges — the corpus is the
load-bearing measurement input for the wire-surface existential gate,
and a corpus with unparseable ground-truth entries would silently
inflate Claude's parse-failure rate.

Usage:
    python3 verify_corpus.py                    # uses ../../target/{release,debug}/mimir-cli
    python3 verify_corpus.py --bin <path>       # point at a pre-built binary
    python3 verify_corpus.py --corpus <path>    # override corpus.jsonl path

Exit codes:
    0 — every entry parses cleanly
    1 — one or more entries fail to parse; failures printed to stderr
    2 — usage / filesystem error

The script runs `mimir-cli parse` in a subprocess per entry (piping
ground_truth_lisp on stdin). This is O(100 × ~15ms) ≈ 1.5 s total, so
fast enough that there's no need to batch.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path


DEFAULT_CORPUS = Path(__file__).resolve().parent / "corpus.jsonl"
REPO_ROOT = Path(__file__).resolve().parents[2]


def locate_bin(explicit: str | None) -> Path:
    if explicit:
        path = Path(explicit).resolve()
        if not path.is_file():
            sys.exit(f"verify_corpus: --bin path does not exist: {path}")
        return path
    for profile in ("release", "debug"):
        candidate = REPO_ROOT / "target" / profile / "mimir-cli"
        if candidate.is_file():
            return candidate
    sys.exit(
        "verify_corpus: no built mimir-cli found under target/release or target/debug.\n"
        f"  Searched: {REPO_ROOT / 'target' / 'release' / 'mimir-cli'}\n"
        f"            {REPO_ROOT / 'target' / 'debug' / 'mimir-cli'}\n"
        "  Build first: `cargo build -p mimir-cli` (or `--release`)."
    )


def run_parse(bin_path: Path, lisp: str) -> tuple[int, str]:
    """Return (exit_code, stderr)."""
    result = subprocess.run(
        [str(bin_path), "parse"],
        input=lisp,
        capture_output=True,
        text=True,
        check=False,
    )
    return result.returncode, result.stderr


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--bin", help="Path to mimir-cli binary")
    ap.add_argument(
        "--corpus",
        default=str(DEFAULT_CORPUS),
        help=f"Path to corpus.jsonl (default: {DEFAULT_CORPUS})",
    )
    args = ap.parse_args()

    bin_path = locate_bin(args.bin)
    corpus_path = Path(args.corpus).resolve()
    if not corpus_path.is_file():
        sys.exit(f"verify_corpus: corpus does not exist: {corpus_path}")

    print(f"binary : {bin_path}")
    print(f"corpus : {corpus_path}")

    entries: list[dict] = []
    with corpus_path.open(encoding="utf-8") as f:
        for lineno, raw in enumerate(f, start=1):
            raw = raw.strip()
            if not raw:
                continue
            try:
                entries.append(json.loads(raw))
            except json.JSONDecodeError as e:
                sys.exit(f"verify_corpus: corpus.jsonl line {lineno}: {e}")

    # Shape distribution sanity check — the spec split is 25/25/25/25.
    shape_counts: dict[str, int] = {}
    for e in entries:
        shape_counts[e["shape"]] = shape_counts.get(e["shape"], 0) + 1
    expected = {"sem": 25, "epi": 25, "pro": 25, "query": 25}
    if shape_counts != expected:
        print(f"WARN: shape distribution {shape_counts} != expected {expected}", file=sys.stderr)

    passed = 0
    failures: list[tuple[str, str, int, str]] = []
    for entry in entries:
        code, stderr = run_parse(bin_path, entry["ground_truth_lisp"])
        if code == 0:
            passed += 1
        else:
            failures.append((entry["id"], entry["ground_truth_lisp"], code, stderr.strip()))

    total = len(entries)
    print(f"\nparsed : {passed}/{total}")
    if failures:
        print(f"failed : {len(failures)}", file=sys.stderr)
        for id_, lisp, code, stderr in failures:
            print(f"\n  [{id_}] exit={code}", file=sys.stderr)
            print(f"    lisp   : {lisp}", file=sys.stderr)
            print(f"    stderr : {stderr}", file=sys.stderr)
        return 1
    print("all ground-truth entries parse cleanly — corpus is internally well-formed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
