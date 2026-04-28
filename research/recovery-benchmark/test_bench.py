#!/usr/bin/env python3
"""Self-tests for the recovery benchmark harness."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
BENCH = REPO_ROOT / "bench"


def run_bench(*args: str, env: dict[str, str] | None = None) -> dict:
    result = subprocess.run(
        [sys.executable, str(BENCH), *args],
        cwd=REPO_ROOT,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise AssertionError(
            f"bench {' '.join(args)} failed with {result.returncode}\n"
            f"stdout:\n{result.stdout}\n"
            f"stderr:\n{result.stderr}"
        )
    return json.loads(result.stdout)


def run_bench_failure(
    *args: str,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        [sys.executable, str(BENCH), *args],
        cwd=REPO_ROOT,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode == 0:
        raise AssertionError(
            f"bench {' '.join(args)} unexpectedly succeeded\n"
            f"stdout:\n{result.stdout}\n"
            f"stderr:\n{result.stderr}"
        )
    return result


def fill_scores(scores_path: Path) -> None:
    scores = json.loads(scores_path.read_text(encoding="utf-8"))
    for baseline_id, baseline in scores["baselines"].items():
        baseline["time_to_productive_minutes"] = 3 if baseline_id == "D" else 8
        baseline["fact_correctness"] = {
            "correct": 20 if baseline_id == "D" else 12,
            "prompted": 0,
            "wrong": 0,
            "silent": 0 if baseline_id == "D" else 8,
        }
        baseline["decision_assertions"] = {
            "hallucinations": 0 if baseline_id == "D" else 1,
            "total": 5,
        }
        baseline["staleness"] = {
            "stale_occurrences": 0 if baseline_id == "D" else 2
        }
        baseline["rehydration_tokens"] = {
            "input": 1200 if baseline_id == "D" else 2400,
            "output": 800 if baseline_id == "D" else 1400,
        }
    scores_path.write_text(
        json.dumps(scores, indent=2, sort_keys=True),
        encoding="utf-8",
    )


def materialize_required_inputs(result_dir: Path, materialized_root: Path) -> None:
    for baseline_id in ("B", "C", "D"):
        inputs_path = (
            result_dir / "environments" / baseline_id / "materialized-inputs.json"
        )
        inputs = json.loads(inputs_path.read_text(encoding="utf-8"))
        for item in inputs["inputs"]:
            materialized_path = materialized_root / baseline_id / item["id"]
            if item["kind"] == "directory":
                materialized_path.mkdir(parents=True)
            else:
                materialized_path.parent.mkdir(parents=True)
                materialized_path.write_text("materialized\n", encoding="utf-8")
            item["path"] = str(materialized_path)
            item["verified"] = True
        inputs_path.write_text(
            json.dumps(inputs, indent=2, sort_keys=True),
            encoding="utf-8",
        )


def fill_transcripts(result_dir: Path) -> None:
    cold_start_prompt = "I'm back. What were we working on, and what's the next step?"
    work_instruction_test = (
        "Identify the next concrete unblock for the recovery benchmark work, "
        "and state why a real pilot must not run yet."
    )
    for baseline_id in ("A", "B", "C", "D"):
        (result_dir / f"transcript-{baseline_id}.md").write_text(
            "\n".join(
                [
                    f"# Captured transcript {baseline_id}",
                    "",
                    "## Cold-start prompt",
                    "",
                    cold_start_prompt,
                    "",
                    "## Transcript",
                    "",
                    "Operator issued the cold-start prompt.",
                    "Agent recovered state and answered the work-instruction test.",
                    "",
                    "## Work-instruction test",
                    "",
                    work_instruction_test,
                    "",
                ]
            ),
            encoding="utf-8",
        )


def write_fake_live_runners(bin_dir: Path) -> None:
    runner = bin_dir / "fake_agent.py"
    runner.write_text(
        "\n".join(
            [
                "#!/usr/bin/env python3",
                "from __future__ import annotations",
                "import sys",
                "payload = sys.stdin.read()",
                "print('FAKE_AGENT_BEGIN')",
                "print(payload)",
                "print('FAKE_AGENT_READY')",
            ]
        )
        + "\n",
        encoding="utf-8",
    )
    runner.chmod(0o755)
    claude = bin_dir / "claude"
    claude.write_text(
        f"#!/bin/sh\nexec {sys.executable} {runner} \"$@\"\n",
        encoding="utf-8",
    )
    claude.chmod(0o755)
    mimir = bin_dir / "mimir"
    mimir.write_text(
        f"#!/bin/sh\nshift\nexec {sys.executable} {runner} \"$@\"\n",
        encoding="utf-8",
    )
    mimir.chmod(0o755)


class RecoveryBenchTests(unittest.TestCase):
    def test_list_scenarios_reports_structured_fixture(self) -> None:
        payload = run_bench("recovery", "--list", "--format", "json")
        self.assertEqual(payload["mode"], "list")
        self.assertIn(
            "01-example-session-context-loss",
            [scenario["id"] for scenario in payload["scenarios"]],
        )

    def test_list_rejects_invalid_ground_truth_shape(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            scenario_path = (
                REPO_ROOT
                / "research"
                / "recovery-benchmark"
                / "scenarios"
                / "01-example-session-context-loss.json"
            )
            scenario = json.loads(scenario_path.read_text(encoding="utf-8"))
            del scenario["ground_truth"][0]["source_refs"]
            temp_path = Path(tmp) / "01-example-session-context-loss.json"
            temp_path.write_text(
                json.dumps(scenario, indent=2, sort_keys=True),
                encoding="utf-8",
            )

            result = run_bench_failure(
                "recovery",
                "--list",
                "--scenarios-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertIn(
                "ground_truth[0].source_refs must be a non-empty list of strings",
                result.stderr,
            )

    def test_list_rejects_duplicate_ground_truth_ids(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            scenario_path = (
                REPO_ROOT
                / "research"
                / "recovery-benchmark"
                / "scenarios"
                / "01-example-session-context-loss.json"
            )
            scenario = json.loads(scenario_path.read_text(encoding="utf-8"))
            scenario["ground_truth"][1]["id"] = scenario["ground_truth"][0]["id"]
            temp_path = Path(tmp) / "01-example-session-context-loss.json"
            temp_path.write_text(
                json.dumps(scenario, indent=2, sort_keys=True),
                encoding="utf-8",
            )

            result = run_bench_failure(
                "recovery",
                "--list",
                "--scenarios-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertIn(
                "ground_truth ids must be unique: GT01",
                result.stderr,
            )

    def test_dry_run_renders_expected_artifacts_without_live_execution(self) -> None:
        payload = run_bench(
            "recovery",
            "--scenario",
            "01-example-session-context-loss",
            "--dry-run",
            "--format",
            "json",
        )
        self.assertEqual(payload["mode"], "dry_run")
        self.assertFalse(payload["live_execution"])
        self.assertEqual(payload["ground_truth_count"], 20)
        self.assertEqual(payload["load_bearing_decision_count"], 5)
        self.assertEqual(payload["staleness_test_count"], 3)
        self.assertEqual(
            [baseline["id"] for baseline in payload["baselines"]],
            ["A", "B", "C", "D"],
        )
        self.assertIn(
            "research/recovery-benchmark/results/01-example-session-context-loss/scorecard.md",
            payload["expected_artifacts"],
        )

    def test_init_results_writes_scorecard_and_placeholders(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            payload = run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--init-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            self.assertEqual(payload["mode"], "init_results")
            self.assertFalse(payload["live_execution"])
            self.assertEqual(payload["created_count"], 8)
            self.assertEqual(payload["skipped_count"], 0)

            result_dir = Path(tmp) / "01-example-session-context-loss"
            scorecard = result_dir / "scorecard.md"
            scores = result_dir / "scores.json"
            transcript_a = result_dir / "transcript-A.md"
            run_plan = result_dir / "run-plan.json"

            self.assertTrue(scorecard.is_file())
            self.assertTrue(scores.is_file())
            self.assertTrue(transcript_a.is_file())
            self.assertTrue(run_plan.is_file())
            self.assertIn("GT11", scorecard.read_text(encoding="utf-8"))
            self.assertIn(
                "load_bearing_decision",
                scorecard.read_text(encoding="utf-8"),
            )
            self.assertIn(
                "Cold-start prompt",
                transcript_a.read_text(encoding="utf-8"),
            )
            self.assertEqual(
                json.loads(run_plan.read_text(encoding="utf-8"))["scenario_id"],
                "01-example-session-context-loss",
            )
            self.assertEqual(
                json.loads(scores.read_text(encoding="utf-8"))["schema_version"],
                1,
            )

    def test_init_results_does_not_clobber_operator_files(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--init-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            transcript_a = (
                Path(tmp) / "01-example-session-context-loss" / "transcript-A.md"
            )
            transcript_a.write_text("operator transcript\n", encoding="utf-8")

            payload = run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--init-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertEqual(payload["created_count"], 0)
            self.assertEqual(payload["skipped_count"], 8)
            self.assertEqual(
                transcript_a.read_text(encoding="utf-8"),
                "operator transcript\n",
            )

    def test_validate_transcripts_reports_placeholders(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--init-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            payload = run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--validate-transcripts",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertEqual(payload["mode"], "validate_transcripts")
            self.assertFalse(payload["ready_for_scoring"])
            self.assertEqual(payload["captured_count"], 0)
            self.assertEqual(payload["placeholder_count"], 4)
            self.assertEqual(payload["missing_count"], 0)
            baseline_a = next(
                row for row in payload["baselines"] if row["baseline_id"] == "A"
            )
            self.assertFalse(baseline_a["captured"])
            self.assertEqual(baseline_a["problem"], "placeholder")

    def test_validate_transcripts_accepts_captured_files(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--init-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            result_dir = Path(tmp) / "01-example-session-context-loss"
            fill_transcripts(result_dir)

            payload = run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--validate-transcripts",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertTrue(payload["ready_for_scoring"])
            self.assertEqual(payload["captured_count"], 4)
            self.assertEqual(payload["placeholder_count"], 0)
            self.assertEqual(payload["missing_count"], 0)
            self.assertTrue(all(row["captured"] for row in payload["baselines"]))

    def test_validate_transcripts_rejects_missing_prompt_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--init-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            result_dir = Path(tmp) / "01-example-session-context-loss"
            fill_transcripts(result_dir)
            (result_dir / "transcript-D.md").write_text(
                "\n".join(
                    [
                        "# Captured transcript D",
                        "",
                        "This is non-placeholder text, but not the actual prompt.",
                        "",
                    ]
                ),
                encoding="utf-8",
            )

            payload = run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--validate-transcripts",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            baseline_d = next(
                row for row in payload["baselines"] if row["baseline_id"] == "D"
            )
            self.assertFalse(baseline_d["captured"])
            self.assertEqual(baseline_d["problem"], "missing_prompt_evidence")

    def test_prepare_envs_writes_baseline_manifests(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            payload = run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--prepare-envs",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertEqual(payload["mode"], "prepare_envs")
            self.assertFalse(payload["live_execution"])
            self.assertEqual(payload["created_count"], 24)
            self.assertEqual(payload["skipped_count"], 0)

            env_dir = (
                Path(tmp)
                / "01-example-session-context-loss"
                / "environments"
                / "D"
            )
            manifest = json.loads(
                (env_dir / "manifest.json").read_text(encoding="utf-8")
            )
            self.assertEqual(manifest["schema_version"], 1)
            self.assertEqual(manifest["baseline_id"], "D")
            self.assertFalse(manifest["live_execution"])
            self.assertEqual(
                manifest["transcript_path"],
                str(
                    Path(tmp)
                    / "01-example-session-context-loss"
                    / "transcript-D.md"
                ),
            )
            self.assertIn(
                "I'm back",
                (env_dir / "cold-start-prompt.txt").read_text(encoding="utf-8"),
            )
            self.assertIn(
                "Canonical Mimir log",
                (env_dir / "preserved-state.md").read_text(encoding="utf-8"),
            )
            inputs = json.loads(
                (env_dir / "materialized-inputs.json").read_text(encoding="utf-8")
            )
            self.assertEqual(inputs["baseline_id"], "D")
            self.assertEqual(
                [item["id"] for item in inputs["inputs"]],
                ["canonical_mimir_log"],
            )

    def test_prepare_envs_does_not_clobber_operator_notes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--prepare-envs",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            readme = (
                Path(tmp)
                / "01-example-session-context-loss"
                / "environments"
                / "A"
                / "README.md"
            )
            readme.write_text("operator setup notes\n", encoding="utf-8")

            payload = run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--prepare-envs",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertEqual(payload["created_count"], 0)
            self.assertEqual(payload["skipped_count"], 24)
            self.assertEqual(
                readme.read_text(encoding="utf-8"),
                "operator setup notes\n",
            )

    def test_validate_envs_reports_missing_preserved_inputs(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--prepare-envs",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            payload = run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--validate-envs",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertEqual(payload["mode"], "validate_envs")
            self.assertEqual(payload["ready_count"], 1)
            self.assertEqual(payload["blocked_count"], 3)
            baseline_a = next(
                row for row in payload["baselines"] if row["baseline_id"] == "A"
            )
            baseline_d = next(
                row for row in payload["baselines"] if row["baseline_id"] == "D"
            )
            self.assertTrue(baseline_a["ready_for_launch"])
            self.assertFalse(baseline_d["ready_for_launch"])
            self.assertIn("canonical_mimir_log", baseline_d["missing_inputs"])

    def test_validate_envs_accepts_verified_materialized_inputs(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--prepare-envs",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            result_dir = Path(tmp) / "01-example-session-context-loss"
            materialize_required_inputs(result_dir, Path(tmp) / "materialized")

            payload = run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--validate-envs",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertEqual(payload["ready_count"], 4)
            self.assertEqual(payload["blocked_count"], 0)
            self.assertTrue(
                all(row["ready_for_launch"] for row in payload["baselines"])
            )

    def test_validate_envs_reports_tampered_required_contract(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--prepare-envs",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            inputs_path = (
                Path(tmp)
                / "01-example-session-context-loss"
                / "environments"
                / "D"
                / "materialized-inputs.json"
            )
            inputs = json.loads(inputs_path.read_text(encoding="utf-8"))
            inputs["inputs"] = []
            inputs_path.write_text(
                json.dumps(inputs, indent=2, sort_keys=True),
                encoding="utf-8",
            )

            payload = run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--validate-envs",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            baseline_d = next(
                row for row in payload["baselines"] if row["baseline_id"] == "D"
            )
            self.assertFalse(baseline_d["ready_for_launch"])
            self.assertIn("canonical_mimir_log", baseline_d["missing_inputs"])

    def test_validate_envs_rejects_wrong_materialized_input_kind(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--prepare-envs",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            inputs_path = (
                Path(tmp)
                / "01-example-session-context-loss"
                / "environments"
                / "B"
                / "materialized-inputs.json"
            )
            wrong_path = Path(tmp) / "claude-memory-file"
            wrong_path.write_text("not a directory\n", encoding="utf-8")
            inputs = json.loads(inputs_path.read_text(encoding="utf-8"))
            inputs["inputs"][0]["path"] = str(wrong_path)
            inputs["inputs"][0]["verified"] = True
            inputs_path.write_text(
                json.dumps(inputs, indent=2, sort_keys=True),
                encoding="utf-8",
            )

            payload = run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--validate-envs",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            baseline_b = next(
                row for row in payload["baselines"] if row["baseline_id"] == "B"
            )
            self.assertFalse(baseline_b["ready_for_launch"])
            self.assertEqual(baseline_b["missing_inputs"], [])
            self.assertEqual(
                baseline_b["invalid_inputs"][0]["id"],
                "claude_markdown_memory_dir",
            )
            self.assertEqual(
                baseline_b["invalid_inputs"][0]["problem"],
                "expected directory",
            )

    def test_launch_plan_blocks_unready_baselines_without_execution(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--prepare-envs",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            payload = run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--launch-plan",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertEqual(payload["mode"], "launch_plan")
            self.assertFalse(payload["live_execution"])
            self.assertFalse(payload["ready_to_launch"])
            self.assertEqual(payload["launchable_count"], 1)
            self.assertEqual(payload["blocked_count"], 3)
            baseline_a = next(
                row for row in payload["baselines"] if row["baseline_id"] == "A"
            )
            baseline_d = next(
                row for row in payload["baselines"] if row["baseline_id"] == "D"
            )
            self.assertIsNotNone(baseline_a["launch_contract"])
            self.assertEqual(baseline_a["launch_contract"]["argv"], ["claude"])
            self.assertIsNone(baseline_d["launch_contract"])
            self.assertIn("canonical_mimir_log", baseline_d["blocked_reasons"])

    def test_launch_plan_emits_contracts_for_ready_environments(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--prepare-envs",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            result_dir = Path(tmp) / "01-example-session-context-loss"
            materialize_required_inputs(result_dir, Path(tmp) / "materialized")

            payload = run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--launch-plan",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertTrue(payload["ready_to_launch"])
            self.assertEqual(payload["launchable_count"], 4)
            baseline_d = next(
                row for row in payload["baselines"] if row["baseline_id"] == "D"
            )
            contract = baseline_d["launch_contract"]
            self.assertEqual(contract["schema_version"], 1)
            self.assertEqual(contract["runner"], "mimir_wrapped_claude")
            self.assertEqual(contract["argv"], ["mimir", "claude"])
            self.assertEqual(contract["cutoff_minutes"], 10)
            self.assertTrue(contract["transcript_path"].endswith("transcript-D.md"))
            self.assertEqual(
                [item["id"] for item in contract["preserved_inputs"]],
                ["canonical_mimir_log"],
            )

    def test_write_launch_contracts_blocks_partial_readiness(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--prepare-envs",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            payload = run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--write-launch-contracts",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            result_dir = Path(tmp) / "01-example-session-context-loss"
            self.assertEqual(payload["mode"], "write_launch_contracts")
            self.assertFalse(payload["ready_to_launch"])
            self.assertEqual(payload["created_count"], 0)
            self.assertEqual(payload["skipped_count"], 0)
            self.assertEqual(payload["blocked_count"], 3)
            self.assertFalse(
                (
                    result_dir
                    / "environments"
                    / "A"
                    / "launch-contract.json"
                ).exists()
            )

    def test_write_launch_contracts_materializes_ready_contracts(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--prepare-envs",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            result_dir = Path(tmp) / "01-example-session-context-loss"
            materialize_required_inputs(result_dir, Path(tmp) / "materialized")

            payload = run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--write-launch-contracts",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertTrue(payload["ready_to_launch"])
            self.assertEqual(payload["created_count"], 4)
            self.assertEqual(payload["skipped_count"], 0)
            self.assertEqual(payload["blocked_count"], 0)
            contract_path = (
                result_dir / "environments" / "D" / "launch-contract.json"
            )
            self.assertTrue(contract_path.is_file())
            contract = json.loads(contract_path.read_text(encoding="utf-8"))
            self.assertEqual(contract["runner"], "mimir_wrapped_claude")
            self.assertEqual(contract["argv"], ["mimir", "claude"])
            self.assertEqual(
                contract["transcript_path"],
                str(result_dir / "transcript-D.md"),
            )
            contract_path.write_text("operator launch notes\n", encoding="utf-8")

            rerun = run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--write-launch-contracts",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertEqual(rerun["created_count"], 0)
            self.assertEqual(rerun["skipped_count"], 4)
            self.assertEqual(
                contract_path.read_text(encoding="utf-8"),
                "operator launch notes\n",
            )

    def test_validate_launch_contracts_reports_missing_contracts(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--prepare-envs",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            result_dir = Path(tmp) / "01-example-session-context-loss"
            materialize_required_inputs(result_dir, Path(tmp) / "materialized")

            payload = run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--validate-launch-contracts",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertEqual(payload["mode"], "validate_launch_contracts")
            self.assertFalse(payload["ready_for_execution"])
            self.assertEqual(payload["valid_count"], 0)
            self.assertEqual(payload["problem_count"], 4)
            self.assertEqual(payload["missing_count"], 4)
            baseline_d = next(
                row for row in payload["baselines"] if row["baseline_id"] == "D"
            )
            self.assertFalse(baseline_d["valid"])
            self.assertEqual(baseline_d["problem"], "missing")

    def test_validate_launch_contracts_accepts_materialized_contracts(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--prepare-envs",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            result_dir = Path(tmp) / "01-example-session-context-loss"
            materialize_required_inputs(result_dir, Path(tmp) / "materialized")
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--write-launch-contracts",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            payload = run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--validate-launch-contracts",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertTrue(payload["ready_for_execution"])
            self.assertEqual(payload["valid_count"], 4)
            self.assertEqual(payload["problem_count"], 0)
            self.assertEqual(payload["missing_count"], 0)
            self.assertTrue(all(row["valid"] for row in payload["baselines"]))

    def test_validate_launch_contracts_rejects_tampered_contract(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--prepare-envs",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            result_dir = Path(tmp) / "01-example-session-context-loss"
            materialize_required_inputs(result_dir, Path(tmp) / "materialized")
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--write-launch-contracts",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            contract_path = (
                result_dir / "environments" / "D" / "launch-contract.json"
            )
            contract = json.loads(contract_path.read_text(encoding="utf-8"))
            contract["runner"] = "native_claude"
            contract_path.write_text(
                json.dumps(contract, indent=2, sort_keys=True) + "\n",
                encoding="utf-8",
            )

            payload = run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--validate-launch-contracts",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertFalse(payload["ready_for_execution"])
            self.assertEqual(payload["valid_count"], 3)
            self.assertEqual(payload["problem_count"], 1)
            baseline_d = next(
                row for row in payload["baselines"] if row["baseline_id"] == "D"
            )
            self.assertEqual(baseline_d["problem"], "stale_or_tampered")

    def test_validate_launch_contracts_rejects_tampered_prompt_files(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--prepare-envs",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            result_dir = Path(tmp) / "01-example-session-context-loss"
            materialize_required_inputs(result_dir, Path(tmp) / "materialized")
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--write-launch-contracts",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            prompt_path = (
                result_dir / "environments" / "D" / "cold-start-prompt.txt"
            )
            prompt_path.write_text("stale prompt\n", encoding="utf-8")

            payload = run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--validate-launch-contracts",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertFalse(payload["ready_for_execution"])
            self.assertEqual(payload["valid_count"], 3)
            self.assertEqual(payload["problem_count"], 1)
            baseline_d = next(
                row for row in payload["baselines"] if row["baseline_id"] == "D"
            )
            self.assertEqual(
                baseline_d["problem"],
                "cold_start_prompt_stale_or_tampered",
            )

    def test_execute_launch_contracts_requires_explicit_approval(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--prepare-envs",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            result_dir = Path(tmp) / "01-example-session-context-loss"
            materialize_required_inputs(result_dir, Path(tmp) / "materialized")
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--write-launch-contracts",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            result = run_bench_failure(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--execute-launch-contracts",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertIn(
                "--execute-launch-contracts requires --approve-live-execution",
                result.stderr,
            )

    def test_execute_launch_contracts_captures_transcripts_and_scores_mechanics(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--init-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--prepare-envs",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            result_dir = Path(tmp) / "01-example-session-context-loss"
            materialize_required_inputs(result_dir, Path(tmp) / "materialized")
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--write-launch-contracts",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            bin_dir = Path(tmp) / "bin"
            bin_dir.mkdir()
            write_fake_live_runners(bin_dir)
            env = dict(os.environ)
            env["PATH"] = str(bin_dir) + os.pathsep + env["PATH"]

            payload = run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--execute-launch-contracts",
                "--approve-live-execution",
                "01-example-session-context-loss",
                "--results-dir",
                tmp,
                "--format",
                "json",
                env=env,
            )

            self.assertEqual(payload["mode"], "execute_launch_contracts")
            self.assertTrue(payload["live_execution"])
            self.assertEqual(payload["executed_count"], 4)
            self.assertEqual(payload["failed_count"], 0)
            self.assertEqual(payload["mechanical_scores_updated_count"], 12)
            self.assertTrue(payload["transcripts_ready"])
            self.assertTrue((result_dir / "live-run.json").is_file())
            transcript_d = (result_dir / "transcript-D.md").read_text(
                encoding="utf-8"
            )
            self.assertIn("## Cold-start prompt", transcript_d)
            self.assertIn("FAKE_AGENT_READY", transcript_d)
            self.assertIn("Identify the next concrete unblock", transcript_d)
            scores = json.loads((result_dir / "scores.json").read_text("utf-8"))
            self.assertEqual(scores["baselines"]["D"]["time_to_productive_minutes"], 0)
            self.assertGreater(
                scores["baselines"]["D"]["rehydration_tokens"]["input"],
                0,
            )
            self.assertGreater(
                scores["baselines"]["D"]["rehydration_tokens"]["output"],
                0,
            )

    def test_execute_launch_contracts_refuses_existing_captured_transcript(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--init-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--prepare-envs",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            result_dir = Path(tmp) / "01-example-session-context-loss"
            materialize_required_inputs(result_dir, Path(tmp) / "materialized")
            fill_transcripts(result_dir)
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--write-launch-contracts",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            result = run_bench_failure(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--execute-launch-contracts",
                "--approve-live-execution",
                "01-example-session-context-loss",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertIn("refusing to overwrite captured transcript", result.stderr)

    def test_execute_launch_contracts_records_spawn_failure(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--init-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--prepare-envs",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            result_dir = Path(tmp) / "01-example-session-context-loss"
            materialize_required_inputs(result_dir, Path(tmp) / "materialized")
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--write-launch-contracts",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            empty_bin = Path(tmp) / "empty-bin"
            empty_bin.mkdir()
            env = dict(os.environ)
            env["PATH"] = str(empty_bin)

            result = run_bench_failure(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--execute-launch-contracts",
                "--approve-live-execution",
                "01-example-session-context-loss",
                "--results-dir",
                tmp,
                "--format",
                "json",
                env=env,
            )

            self.assertIn("live execution failed for baseline(s)", result.stderr)
            transcript_a = (result_dir / "transcript-A.md").read_text(
                encoding="utf-8"
            )
            self.assertIn("spawn failed:", transcript_a)
            self.assertIn("## Captured stderr", transcript_a)

    def test_score_results_reports_incomplete_template(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--init-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            payload = run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--score-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertEqual(payload["mode"], "score_results")
            self.assertFalse(payload["complete"])
            self.assertEqual(payload["missing_count"], 40)
            self.assertFalse(payload["transcripts_ready"])
            self.assertEqual(payload["transcript_problem_count"], 4)
            self.assertIn(
                "baselines.A.time_to_productive_minutes",
                payload["missing_fields"],
            )

    def test_score_results_requires_captured_transcripts(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--init-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            scores_path = (
                Path(tmp) / "01-example-session-context-loss" / "scores.json"
            )
            fill_scores(scores_path)

            payload = run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--score-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertFalse(payload["complete"])
            self.assertEqual(payload["missing_count"], 0)
            self.assertFalse(payload["transcripts_ready"])
            self.assertEqual(payload["transcript_problem_count"], 4)
            self.assertEqual(payload["baseline_summaries"], [])

    def test_score_results_summarizes_complete_scores(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--init-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            scores_path = (
                Path(tmp) / "01-example-session-context-loss" / "scores.json"
            )
            fill_scores(scores_path)
            fill_transcripts(Path(tmp) / "01-example-session-context-loss")

            payload = run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--score-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertTrue(payload["complete"])
            self.assertEqual(payload["missing_count"], 0)
            self.assertTrue(payload["transcripts_ready"])
            self.assertEqual(payload["transcript_problem_count"], 0)
            baseline_d = next(
                summary
                for summary in payload["baseline_summaries"]
                if summary["id"] == "D"
            )
            self.assertEqual(baseline_d["fact_unprompted_correct_pct"], 1.0)
            self.assertEqual(baseline_d["rehydration_token_total"], 2000)

    def test_score_results_rejects_time_after_cutoff(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--init-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            result_dir = Path(tmp) / "01-example-session-context-loss"
            scores_path = result_dir / "scores.json"
            fill_scores(scores_path)
            fill_transcripts(result_dir)
            scores = json.loads(scores_path.read_text(encoding="utf-8"))
            scores["baselines"]["D"]["time_to_productive_minutes"] = 11
            scores_path.write_text(
                json.dumps(scores, indent=2, sort_keys=True),
                encoding="utf-8",
            )

            result = run_bench_failure(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--score-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertIn(
                "baselines.D.time_to_productive_minutes exceeds cutoff_minutes",
                result.stderr,
            )

    def test_score_results_rejects_staleness_above_probe_count(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--init-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            result_dir = Path(tmp) / "01-example-session-context-loss"
            scores_path = result_dir / "scores.json"
            fill_scores(scores_path)
            fill_transcripts(result_dir)
            scores = json.loads(scores_path.read_text(encoding="utf-8"))
            scores["baselines"]["D"]["staleness"]["stale_occurrences"] = 4
            scores_path.write_text(
                json.dumps(scores, indent=2, sort_keys=True),
                encoding="utf-8",
            )

            result = run_bench_failure(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--score-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertIn(
                "baselines.D.staleness.stale_occurrences exceeds staleness_tests",
                result.stderr,
            )

    def test_score_results_rejects_zero_decision_total(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--init-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            result_dir = Path(tmp) / "01-example-session-context-loss"
            scores_path = result_dir / "scores.json"
            fill_scores(scores_path)
            fill_transcripts(result_dir)
            scores = json.loads(scores_path.read_text(encoding="utf-8"))
            scores["baselines"]["D"]["decision_assertions"]["total"] = 0
            scores_path.write_text(
                json.dumps(scores, indent=2, sort_keys=True),
                encoding="utf-8",
            )

            result = run_bench_failure(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--score-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertIn(
                "baselines.D.decision_assertions.total must be greater than zero",
                result.stderr,
            )

    def test_score_results_rejects_boolean_integer_fields(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--init-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            result_dir = Path(tmp) / "01-example-session-context-loss"
            scores_path = result_dir / "scores.json"
            fill_scores(scores_path)
            fill_transcripts(result_dir)
            scores = json.loads(scores_path.read_text(encoding="utf-8"))
            scores["baselines"]["D"]["rehydration_tokens"]["input"] = True
            scores_path.write_text(
                json.dumps(scores, indent=2, sort_keys=True),
                encoding="utf-8",
            )

            result = run_bench_failure(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--score-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertIn(
                "baselines.D.rehydration_tokens.input must be a non-negative integer",
                result.stderr,
            )

    def test_summary_results_reports_incomplete_scenario_scores(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--init-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            payload = run_bench(
                "recovery",
                "--summary-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertEqual(payload["mode"], "summary_results")
            self.assertEqual(payload["scenario_count"], 4)
            self.assertEqual(payload["complete_count"], 0)
            self.assertEqual(payload["incomplete_count"], 4)
            scenario = next(
                row
                for row in payload["scenarios"]
                if row["scenario_id"] == "01-example-session-context-loss"
            )
            self.assertFalse(scenario["complete"])
            self.assertEqual(scenario["missing_count"], 40)
            self.assertEqual(scenario["transcript_problem_count"], 4)
            self.assertFalse(payload["benchmark_verdict"]["complete"])
            self.assertFalse(payload["benchmark_verdict"]["benchmark_win"])

    def test_summary_results_reports_missing_scenario_scores(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            payload = run_bench(
                "recovery",
                "--summary-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertEqual(payload["mode"], "summary_results")
            self.assertEqual(payload["scenario_count"], 4)
            self.assertEqual(payload["complete_count"], 0)
            self.assertEqual(payload["incomplete_count"], 4)
            self.assertEqual(payload["missing_score_file_count"], 4)
            scenario = payload["scenarios"][0]
            self.assertFalse(scenario["complete"])
            self.assertFalse(scenario["score_file_present"])
            self.assertEqual(scenario["score_problem"], "missing")
            self.assertEqual(scenario["transcript_problem_count"], 4)
            self.assertEqual(scenario["baseline_summaries"], [])
            self.assertFalse(payload["benchmark_verdict"]["complete"])
            self.assertFalse(payload["benchmark_verdict"]["benchmark_win"])

    def test_summary_results_aggregates_complete_scores(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--init-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            scores_path = (
                Path(tmp) / "01-example-session-context-loss" / "scores.json"
            )
            fill_scores(scores_path)
            fill_transcripts(Path(tmp) / "01-example-session-context-loss")

            payload = run_bench(
                "recovery",
                "--summary-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            self.assertEqual(payload["scenario_count"], 4)
            self.assertEqual(payload["complete_count"], 1)
            self.assertEqual(payload["incomplete_count"], 3)
            self.assertEqual(payload["missing_score_file_count"], 3)
            scenario = next(
                row
                for row in payload["scenarios"]
                if row["scenario_id"] == "01-example-session-context-loss"
            )
            self.assertTrue(scenario["complete"])
            self.assertTrue(scenario["score_file_present"])
            self.assertTrue(scenario["transcripts_ready"])
            baseline_d = next(
                summary
                for summary in scenario["baseline_summaries"]
                if summary["id"] == "D"
            )
            self.assertEqual(baseline_d["rehydration_token_total"], 2000)
            self.assertTrue(scenario["scenario_verdict"]["mimir_win"])
            self.assertEqual(scenario["scenario_verdict"]["d_metric_wins"], 5)
            self.assertTrue(
                scenario["scenario_verdict"]["guardrails"][
                    "hallucinations_not_worse_than_b"
                ]
            )
            self.assertTrue(
                scenario["scenario_verdict"]["guardrails"]["staleness_better_than_b"]
            )
            self.assertEqual(payload["benchmark_verdict"]["mimir_wins"], 1)
            self.assertEqual(payload["benchmark_verdict"]["required_wins"], 1)
            self.assertFalse(payload["benchmark_verdict"]["complete"])
            self.assertFalse(payload["benchmark_verdict"]["benchmark_win"])

    def test_summary_results_enforces_staleness_guardrail(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--init-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            scores_path = (
                Path(tmp) / "01-example-session-context-loss" / "scores.json"
            )
            fill_scores(scores_path)
            fill_transcripts(Path(tmp) / "01-example-session-context-loss")
            scores = json.loads(scores_path.read_text(encoding="utf-8"))
            scores["baselines"]["D"]["staleness"]["stale_occurrences"] = scores[
                "baselines"
            ]["B"]["staleness"]["stale_occurrences"]
            scores_path.write_text(
                json.dumps(scores, indent=2, sort_keys=True),
                encoding="utf-8",
            )

            payload = run_bench(
                "recovery",
                "--summary-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            scenario = payload["scenarios"][0]
            verdict = scenario["scenario_verdict"]
            self.assertEqual(verdict["d_metric_wins"], 4)
            self.assertFalse(verdict["guardrails"]["staleness_better_than_b"])
            self.assertFalse(verdict["mimir_win"])
            self.assertEqual(payload["benchmark_verdict"]["mimir_wins"], 0)
            self.assertFalse(payload["benchmark_verdict"]["benchmark_win"])

    def test_summary_results_enforces_hallucination_guardrail(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run_bench(
                "recovery",
                "--scenario",
                "01-example-session-context-loss",
                "--init-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )
            scores_path = (
                Path(tmp) / "01-example-session-context-loss" / "scores.json"
            )
            fill_scores(scores_path)
            fill_transcripts(Path(tmp) / "01-example-session-context-loss")
            scores = json.loads(scores_path.read_text(encoding="utf-8"))
            scores["baselines"]["D"]["decision_assertions"]["hallucinations"] = (
                scores["baselines"]["B"]["decision_assertions"]["hallucinations"] + 1
            )
            scores_path.write_text(
                json.dumps(scores, indent=2, sort_keys=True),
                encoding="utf-8",
            )

            payload = run_bench(
                "recovery",
                "--summary-results",
                "--results-dir",
                tmp,
                "--format",
                "json",
            )

            verdict = payload["scenarios"][0]["scenario_verdict"]
            self.assertEqual(verdict["d_metric_wins"], 4)
            self.assertFalse(
                verdict["guardrails"]["hallucinations_not_worse_than_b"]
            )
            self.assertFalse(verdict["mimir_win"])


if __name__ == "__main__":
    unittest.main()
