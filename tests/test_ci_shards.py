from __future__ import annotations

import unittest
from pathlib import Path

import yaml

from scripts.python_test_shards import (
    ALL_SHARDS,
    CLI_SMOKE_SHARD,
    PARALLEL_SHARD_DISTRIBUTION,
    PREFLIGHT_SHARD,
    RUNTIME_SHARDS,
    discover_test_files,
    shard_for,
    shard_manifest,
    validate_manifest,
)


ROOT = Path(__file__).resolve().parent.parent


class PythonShardContractTest(unittest.TestCase):
    def test_every_python_test_module_has_exactly_one_nonempty_lane(self):
        manifest = validate_manifest(ROOT)
        discovered = discover_test_files(ROOT)
        assigned = [path for shard in ALL_SHARDS for path in manifest[shard]]
        self.assertEqual(sorted(assigned), sorted(discovered))
        self.assertEqual(len(assigned), len(set(assigned)))
        self.assertTrue(all(manifest[shard] for shard in ALL_SHARDS))

    def test_new_modules_fail_open_to_core_without_becoming_uncollected(self):
        self.assertEqual(shard_for(Path("tests/test_future_feature.py")), "core-accounting")

    def test_sensitive_and_specialized_modules_stay_out_of_xdist(self):
        expected = {
            "tests/test_ci_shards.py": PREFLIGHT_SHARD,
            "tests/test_cli_entrypoint_smoke.py": CLI_SMOKE_SHARD,
            "tests/test_proxy.py": "serial-network",
            "tests/test_sync_backends.py": "serial-network",
            "tests/test_sync_replication_s5.py": "serial-network",
            "tests/test_cli_chat.py": "serial-daemon",
            "tests/test_daemon_smoke.py": "serial-daemon",
            "tests/test_review_regressions.py": "serial-regressions",
            "tests/test_remembered_unlock.py": "serial-integration",
            "tests/test_wasabi_import.py": "serial-integration",
            "tests/integration/test_live_bdk_observer.py": "serial-integration",
        }
        for path, shard in expected.items():
            with self.subTest(path=path):
                self.assertEqual(shard_for(Path(path)), shard)

    def test_only_measured_safe_shards_enable_xdist(self):
        self.assertTrue(
            all(mode == "loadscope" for mode in PARALLEL_SHARD_DISTRIBUTION.values())
        )
        for shard in (
            "serial-network",
            "serial-daemon",
            "serial-regressions",
            "serial-integration",
        ):
            self.assertNotIn(shard, PARALLEL_SHARD_DISTRIBUTION)

    def test_ci_jobs_preserve_fail_fast_dependencies_and_aggregate_check(self):
        workflow = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text())
        self.assertTrue(workflow["concurrency"]["cancel-in-progress"])
        jobs = workflow["jobs"]
        base_jobs = {"preflight", "python-tests", "frontend", "cli-smoke", "required"}
        optional_jobs = {"chain-observers"} & set(jobs)
        self.assertEqual(
            set(jobs),
            base_jobs | optional_jobs,
        )
        self.assertEqual(jobs["python-tests"]["needs"], "preflight")
        for job in ("frontend", "cli-smoke", *sorted(optional_jobs)):
            self.assertNotIn("needs", jobs[job])
        preflight_steps = {step.get("name") for step in jobs["preflight"]["steps"]}
        self.assertFalse(
            preflight_steps
            & {"Set up Node.js", "Install pnpm", "Desktop UI typecheck", "Desktop UI lint"}
        )
        frontend_steps = {step.get("name") for step in jobs["frontend"]["steps"]}
        self.assertTrue(
            {"Desktop UI typecheck", "Desktop UI lint", "Run Vitest"}.issubset(frontend_steps)
        )
        matrix = jobs["python-tests"]["strategy"]["matrix"]["shard"]
        self.assertEqual(tuple(matrix), RUNTIME_SHARDS)
        required_needs = set(jobs["required"]["needs"])
        self.assertEqual(
            required_needs,
            {"preflight", "python-tests", "frontend", "cli-smoke"} | optional_jobs,
        )

    def test_dependency_caches_and_failure_artifacts_stay_enabled(self):
        workflow = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text())
        jobs = workflow["jobs"]
        uv_jobs = ["preflight", "python-tests", "cli-smoke"]
        if "chain-observers" in jobs:
            uv_jobs.append("chain-observers")
        for job_name in uv_jobs:
            uv_steps = [
                step
                for step in jobs[job_name]["steps"]
                if str(step.get("uses") or "").startswith("astral-sh/setup-uv@")
            ]
            self.assertEqual(len(uv_steps), 1, job_name)
            self.assertTrue(uv_steps[0]["with"]["enable-cache"], job_name)
            self.assertEqual(uv_steps[0]["with"]["cache-dependency-glob"], "uv.lock")
        setup_node = next(
            step
            for step in jobs["frontend"]["steps"]
            if str(step.get("uses") or "").startswith("actions/setup-node@")
        )
        self.assertEqual(setup_node["with"]["cache"], "pnpm")
        self.assertEqual(
            setup_node["with"]["cache-dependency-path"], "ui-tauri/pnpm-lock.yaml"
        )
        for job_name in ("preflight", "python-tests", "frontend", "cli-smoke"):
            upload = next(
                step
                for step in jobs[job_name]["steps"]
                if str(step.get("uses") or "").startswith("actions/upload-artifact@")
            )
            self.assertEqual(upload["if"], "always()")

    def test_local_quality_gate_runs_python_tests_once(self):
        gate = (ROOT / "scripts/quality-gate.sh").read_text(encoding="utf-8")
        self.assertNotIn("-m unittest", gate)
        self.assertEqual(gate.count("-m pytest tests"), 1)


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
