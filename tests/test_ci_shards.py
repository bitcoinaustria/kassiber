from __future__ import annotations

import re
import unittest
from pathlib import Path
from unittest import mock

import pytest
import yaml

from scripts import python_test_shards
from scripts.python_test_shards import (
    ALL_SHARDS,
    CLI_SMOKE_SHARD,
    PARALLEL_SHARD_DISTRIBUTION,
    PREFLIGHT_SHARD,
    RUNTIME_SHARDS,
    discover_test_files,
    shadow_lanes,
    shadow_overlay,
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
            "tests/rust_port/test_inventory.py": PREFLIGHT_SHARD,
            "tests/rust_port/test_contracts.py": PREFLIGHT_SHARD,
            "tests/test_cli_entrypoint_smoke.py": CLI_SMOKE_SHARD,
            "tests/test_proxy.py": "serial-network",
            "tests/test_sync_backends.py": "serial-network",
            "tests/test_cli_chat.py": "serial-daemon",
            "tests/test_daemon_smoke.py": "serial-daemon",
            "tests/test_rust_shell_parity.py": "serial-daemon",
            "tests/test_rust_hybrid_candidate.py": "serial-daemon",
            "tests/test_rust_wallet_discovery.py": "serial-daemon",
            "tests/test_rust_wallet_acquisition.py": "serial-daemon",
            "tests/test_rust_observer_state.py": "serial-daemon",
            "tests/test_rust_wallet_publication.py": "serial-daemon",
            "tests/test_rust_wallet_fetch.py": "serial-daemon",
            "tests/test_rust_connections.py": "serial-daemon",
            "tests/test_rust_wallet_sync_sources.py": "serial-daemon",
            "tests/test_rust_wallet_sync_btcpay.py": "serial-daemon",
            "tests/test_chain_analysis_consent.py": "daemon-cli",
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
        base_jobs = {
            "preflight",
            "rust-binaries",
            "python-tests",
            "tax-shadow",
            "custody-shadow",
            "frontend",
            "cli-smoke",
            "tax-engine",
            "required",
        }
        optional_jobs = {"chain-observers"} & set(jobs)
        self.assertEqual(
            set(jobs),
            base_jobs | optional_jobs,
        )
        for job in ("python-tests", "tax-shadow", "custody-shadow"):
            self.assertEqual(jobs[job]["needs"], ["preflight", "rust-binaries"], job)
        for job in ("rust-binaries", "frontend", "cli-smoke", "tax-engine", *sorted(optional_jobs)):
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
            {"preflight", "rust-binaries", "python-tests", "tax-shadow", "custody-shadow", "frontend", "cli-smoke", "tax-engine"}
            | optional_jobs,
        )

    def test_every_rust_build_uses_the_pinned_toolchain(self):
        pin = (ROOT / "rust-toolchain.toml").read_text(encoding="utf-8")
        channel = re.search(r'(?m)^channel\s*=\s*"(\d+\.\d+\.\d+)"$', pin)
        self.assertIsNotNone(channel, "rust-toolchain.toml must pin an exact release")
        self.assertIn('components = ["rustfmt", "clippy"]', pin)
        major_minor = channel.group(1).rsplit(".", 1)[0]
        for manifest in ("Cargo.toml", "ui-tauri/src-tauri/Cargo.toml"):
            with self.subTest(manifest=manifest):
                text = (ROOT / manifest).read_text(encoding="utf-8")
                self.assertEqual(
                    re.findall(r'(?m)^rust-version\s*=\s*"([^"]+)"', text), [major_minor]
                )
        # src-tauri is its own workspace; Cargo refuses to build it if the root
        # workspace would contain it without listing it.
        root = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
        self.assertRegex(root, r'(?m)^exclude = \["ui-tauri"\]$')
        action = (ROOT / ".github/actions/setup-rust/action.yml").read_text(encoding="utf-8")
        self.assertIn("rust-toolchain.toml", action)
        self.assertIn("--component rustfmt --component clippy", action)
        self.assertIn('rustup default "$channel"', action)

        # Every job that compiles Rust (cargo, maturin through uv sync, or a
        # Tauri build) installs the pinned toolchain first; none picks another.
        compiles = re.compile(r"\bcargo\b|\buv sync\b|\btauri build\b")
        checked = 0
        for path in sorted((ROOT / ".github/workflows").glob("*.yml")):
            text = path.read_text(encoding="utf-8")
            self.assertNotIn("dtolnay/rust-toolchain", text, path.name)
            self.assertNotRegex(text, r"cargo \+|rustup toolchain install|toolchain: stable")
            for job_name, job in yaml.safe_load(text)["jobs"].items():
                steps = job.get("steps") or []
                setups = [
                    index
                    for index, step in enumerate(steps)
                    if step.get("uses") == "./.github/actions/setup-rust"
                ]
                for index, step in enumerate(steps):
                    if compiles.search(str(step.get("run") or "")):
                        checked += 1
                        with self.subTest(workflow=path.name, job=job_name, step=step.get("name")):
                            self.assertTrue(setups and setups[0] < index)
                            # An unconditional setup also covers conditional
                            # builds; a conditional setup must cover the build.
                            if "if" in steps[setups[0]]:
                                self.assertEqual(steps[setups[0]]["if"], step.get("if"))
        self.assertGreater(checked, 10)

        ci = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text())
        runs = "\n".join(
            str(step.get("run") or "") for step in ci["jobs"]["tax-engine"]["steps"]
        )
        self.assertIn("cargo fmt", runs)
        self.assertIn("cargo clippy", runs)
        self.assertIn("-D warnings", runs)
        self.assertIn("--locked", runs)

    def test_platform_workflows_run_when_only_the_tax_engine_changes(self):
        # The engine's binding is built by uv sync on every platform, so a
        # change to it, its workspace, or its crates must still reach the
        # macOS and Windows lanes.
        for name, events in (
            ("prerelease-binaries.yml", ("pull_request",)),
            ("credential-platforms.yml", ("pull_request", "push")),
        ):
            workflow = yaml.safe_load((ROOT / ".github/workflows" / name).read_text())
            # PyYAML reads the bare `on` key as boolean True.
            triggers = workflow[True]
            for event in events:
                with self.subTest(workflow=name, event=event):
                    self.assertIn("tax-engine/**", triggers[event]["paths"])
                    for path in ("Cargo.toml", "Cargo.lock", "crates/**", "rust-toolchain.toml"):
                        self.assertIn(path, triggers[event]["paths"])
                    self.assertIn(".github/actions/setup-rust/**", triggers[event]["paths"])

    def test_dependency_caches_and_failure_artifacts_stay_enabled(self):
        workflow = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text())
        jobs = workflow["jobs"]
        uv_jobs = ["preflight", "python-tests", "tax-shadow", "cli-smoke"]
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
        # Every job that builds Rust restores a Cargo cache; one job per key
        # saves it, so the parallel shards do not race to upload the same key.
        savers: dict[str, list[str]] = {}
        for job_name in [*uv_jobs, "rust-binaries", "tax-engine"]:
            setup = next(
                step
                for step in jobs[job_name]["steps"]
                if step.get("uses") == "./.github/actions/setup-rust"
            )
            key = setup["with"]["cache-key"]
            self.assertEqual(
                key, "rust-workspace" if job_name == "tax-engine" else "python-binding"
            )
            if setup["with"].get("save-cache") == "true":
                savers.setdefault(key, []).append(job_name)
        self.assertEqual(
            savers, {"python-binding": ["preflight"], "rust-workspace": ["tax-engine"]}
        )
        setup_node = next(
            step
            for step in jobs["frontend"]["steps"]
            if str(step.get("uses") or "").startswith("actions/setup-node@")
        )
        self.assertEqual(setup_node["with"]["cache"], "pnpm")
        self.assertEqual(
            setup_node["with"]["cache-dependency-path"], "ui-tauri/pnpm-lock.yaml"
        )
        for job_name in ("preflight", "python-tests", "tax-shadow", "frontend", "cli-smoke"):
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
        # The single pass runs under census, and the overlay reruns once
        # against the oracle.
        pass_line = next(line for line in gate.splitlines() if "-m pytest tests" in line)
        self.assertIn("KASSIBER_TEST_TAX_ENGINE=census", pass_line)
        self.assertEqual(gate.count("python_test_shards.py run-shadow"), 1)

    def test_required_daemon_shard_and_local_gate_run_the_rust_subprocess_oracle(self):
        workflow = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text())
        jobs = workflow["jobs"]
        self.assertIn("python-tests", jobs["required"]["needs"])
        self.assertIn("rust-binaries", jobs["required"]["needs"])
        self.assertEqual(shard_for(Path("tests/test_rust_shell_parity.py")), "serial-daemon")
        self.assertEqual(shard_for(Path("tests/test_rust_reports.py")), "serial-daemon")
        self.assertEqual(shard_for(Path("tests/test_rust_freshness_queue.py")), "serial-daemon")
        self.assertEqual(shard_for(Path("tests/test_rust_lwk_sync_service.py")), "serial-daemon")
        self.assertEqual(shard_for(Path("tests/test_rust_document_ingestion.py")), "serial-daemon")
        # One job builds the harness binary, its example fixtures and the
        # release journal probes; every lane runs them.
        builder = [str(step.get("run") or "") for step in jobs["rust-binaries"]["steps"]]
        self.assertIn("scripts/build-rust-test-binaries.sh build", builder)
        self.assertIn("scripts/build-rust-test-binaries.sh pack rust-test-binaries.tar", builder)
        script = (ROOT / "scripts/build-rust-test-binaries.sh").read_text(encoding="utf-8")
        self.assertIn("cargo build --locked -p kassiber --features test-harness", script)
        for example in ("discovery_fixture", "acquisition_fixture", "observer_state_fixture", "bdk_fixture",
                        "lwk_fixture", "records_fixture", "publication_fixture", "freshness_fixture",
                        "document_fixture"):
            self.assertIn(f"kassiber-wallets {example} test-harness", script)
        for name in ("python-tests", "tax-shadow", "custody-shadow"):
            job = jobs[name]
            self.assertEqual(job["env"]["KASSIBER_RUST_BIN"], "${{ github.workspace }}/target/debug/kassiber", name)
            steps = job["steps"]
            names = [step.get("name") for step in steps]
            self.assertNotIn("continue-on-error", str(steps))
            unpack = names.index("Unpack the Rust test binaries")
            self.assertLess(names.index("Download the Rust test binaries"), unpack)
            self.assertLess(names.index("Set up Rust"), unpack)
            run = next(step["run"] for step in steps if "python_test_shards.py run" in str(step.get("run") or ""))
            self.assertIn('--part "${{ matrix.part }}"', run)
            self.assertLess(unpack, next(index for index, step in enumerate(steps)
                                         if "python_test_shards.py run" in str(step.get("run") or "")))
            # A split lane runs as parts 1..N of N; every other lane whole.
            matrix = job["strategy"]["matrix"]
            self.assertEqual(matrix["part"], ["1/1"])
            split = [entry["shard"] for entry in matrix["exclude"]]
            self.assertEqual(sorted(split), ["core-accounting", "serial-daemon"])
            self.assertEqual({entry["shard"] for entry in matrix["include"]}, set(split))
            for shard in split:
                parts = [entry["part"] for entry in matrix["include"] if entry["shard"] == shard]
                count = len(parts)
                self.assertGreater(count, 1, shard)
                self.assertEqual(parts, [f"{index}/{count}" for index in range(1, count + 1)], shard)

        # The local quality gate builds the same set itself before the single pytest pass.
        gate = (ROOT / "scripts/quality-gate.sh").read_text(encoding="utf-8")
        select = 'export KASSIBER_RUST_BIN="$ROOT/target/debug/kassiber"'
        self.assertIn(select, gate)
        for build in (
            "cargo build -p kassiber --features test-harness --locked",
            "cargo build -p kassiber-backup --example kassiber-backup-probe --features test-hooks --offline --locked",
            "cargo build -p kassiber-store --example kassiber-storage-probe --features test-hooks --offline --locked",
            "cargo build -p kassiber-secrets --example kassiber-secrets-probe --features test-hooks,native-keyring --offline --locked",
            "cargo build -p kassiber-chain --example client_fixture --features test-harness --offline --locked",
            *(f"cargo build -p kassiber-wallets --example {example} --features test-harness --offline --locked"
              for example in ("discovery_fixture", "acquisition_fixture", "observer_state_fixture", "bdk_fixture",
                              "records_fixture", "publication_fixture", "freshness_fixture", "document_fixture")),
        ):
            with self.subTest(build=build):
                self.assertLess(gate.index("run " + build), gate.index(select))
        for check in ("freshness", "bdk", "lwk", "record", "observer", "acquisition", "exchange"):
            self.assertIn(f"run py crates/wallets/scripts/{check}_mutation_check.py --check", gate)
        self.assertLess(gate.index(select), gate.index("-m pytest tests"))


class TaxShadowOverlayContractTest(unittest.TestCase):
    """The tax-shadow overlay reruns, against the frozen oracle, the modules
    that reach the tax engine; it sits across the lanes instead of being one.
    """

    def test_overlay_holds_the_engine_callers_and_not_the_report_only_modules(self):
        overlay = shadow_overlay(ROOT)
        self.assertTrue(overlay)
        self.assertEqual(len(overlay), len(set(overlay)))
        self.assertLessEqual(set(overlay), set(discover_test_files(ROOT)))
        names = {path.stem for path in overlay}
        for module in (
            "test_tax_events",
            "test_review_regressions",
            "test_rp2_ownership_transfers",
            "test_custody_lineage_flagship",
            "test_report_explanation_carry",
            "test_custody_native_transitions",
            "test_rust_review_cases",
        ):
            with self.subTest(module=module):
                self.assertIn(module, names)
        for module in ("test_austrian_spekulation_section", "test_tax_summary_exclusions"):
            with self.subTest(module=module):
                self.assertNotIn(module, names)

    def test_overlay_follows_tests_modules_that_reach_the_engine(self):
        # test_tax_engine_parity calls production only through the harness.
        self.assertIn(Path("tests/test_tax_engine_parity.py"), shadow_overlay(ROOT))

    def test_tax_shadow_job_reruns_each_overlay_lane_under_shadow(self):
        workflow = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text())
        job = workflow["jobs"]["tax-shadow"]
        self.assertEqual(tuple(job["strategy"]["matrix"]["shard"]), shadow_lanes(ROOT))
        runs = "\n".join(str(step.get("run") or "") for step in job["steps"])
        self.assertIn('python_test_shards.py run-shadow "${{ matrix.shard }}" --ci', runs)

    def test_run_shadow_sets_the_mode_and_passes_pytest_arguments_through(self):
        calls = []

        def run_pytest(paths, **options):
            calls.append((tuple(paths), options))
            return 0

        with mock.patch.object(python_test_shards, "_run_pytest", run_pytest):
            python_test_shards.main(["run-shadow", "--", "-q", "--junitxml=all.xml"])
            python_test_shards.main(["run-shadow", "reports-contracts", "--ci", "--", "-q"])
        overlay = shadow_overlay(ROOT)
        lane = tuple(path for path in validate_manifest(ROOT)["reports-contracts"] if path in overlay)
        self.assertEqual(
            calls,
            [
                (overlay, {"distribution": None, "pytest_args": ["-q", "--junitxml=all.xml"], "tax_mode": "shadow"}),
                (lane, {"distribution": "loadscope", "pytest_args": ["-q"], "tax_mode": "shadow"}),
            ],
        )

    def test_python_lanes_run_under_census(self):
        workflow = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text())
        self.assertEqual(
            workflow["jobs"]["python-tests"]["env"]["KASSIBER_TEST_TAX_ENGINE"], "census"
        )

    def test_ci_fails_oracle_checks_without_rp2(self):
        # tests/test_tax_ledger_goldens.py skips its oracle checks without
        # RP2, except under KASSIBER_CI.
        workflow = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text())
        self.assertEqual(workflow["env"]["KASSIBER_CI"], "1")


if __name__ == "__main__":  # pragma: no cover
    unittest.main()


def test_custody_shadow_is_required_and_excludes_only_the_single_build_budget():
    calls = []
    with mock.patch.object(python_test_shards, "_run_pytest", lambda paths, **options: calls.append((paths, options)) or 0):
        python_test_shards.main(["run-custody-shadow", "--", "-q"])
    paths, options = calls[0]
    assert paths == shadow_overlay(ROOT)
    assert options["custody_mode"] == "shadow"
    assert options["distribution"] is None
    assert options["pytest_args"] == ["--deselect", "tests/test_custody_native_reconciliation.py::test_disjoint_reviewed_routes_limit_observation_reads", "-q"]
    workflow = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text())
    assert "custody-shadow" in workflow["jobs"]["required"]["needs"]
    assert tuple(workflow["jobs"]["custody-shadow"]["strategy"]["matrix"]["shard"]) == python_test_shards.shadow_lanes()
    assert "run-custody-shadow" in (ROOT / "scripts/quality-gate.sh").read_text()


def test_custody_shadow_preserves_the_ci_ordering_policy():
    calls = []
    with mock.patch.object(python_test_shards, "_run_pytest", lambda paths, **options: calls.append((paths, options)) or 0):
        for lane in ("core-accounting", "serial-regressions"):
            python_test_shards.main(["run-custody-shadow", lane, "--ci", "--", "-q"])
    for (paths, options), lane in zip(calls, ("core-accounting", "serial-regressions")):
        assert paths == python_test_shards._shadow_paths(lane)
        assert options["distribution"] == python_test_shards.PARALLEL_SHARD_DISTRIBUTION.get(lane)
        assert options["custody_mode"] == "shadow"


def test_local_worker_and_distribution_overrides_leave_the_ci_policy_alone():
    calls = []
    with mock.patch.object(python_test_shards, "_run_pytest", lambda paths, **options: calls.append((paths, options)) or 0):
        python_test_shards.main(["run", "wallets-sync", "--ci", "--", "-q"])
        python_test_shards.main(["run", "wallets-sync", "--ci", "--workers", "16", "--dist", "worksteal", "--", "-q"])
        python_test_shards.main(["run", "daemon-cli", "--ci", "--workers", "16"])
    lane = validate_manifest(ROOT)["wallets-sync"]
    assert calls[0] == (lane, {"distribution": "loadscope", "pytest_args": ["-q"]})
    assert calls[1] == (lane, {"distribution": "worksteal", "pytest_args": ["-q"], "workers": 16})
    assert calls[2][1] == {"distribution": "loadscope", "pytest_args": [], "workers": 16}
    for argv in (["run", "daemon-cli", "--ci", "--dist", "worksteal"],
                 ["run", "serial-daemon", "--ci", "--workers", "16"]):
        with mock.patch.object(python_test_shards, "_run_pytest") as run_pytest:
            with pytest.raises(SystemExit):
                python_test_shards.main(argv)
            run_pytest.assert_not_called()
    with pytest.raises(SystemExit):
        python_test_shards.main(["run", "wallets-sync", "--workers", "16"])


def test_parts_split_a_lane_into_disjoint_complete_runs():
    import os
    import subprocess
    import sys

    def collected(part=None):
        env = dict(os.environ)
        env.pop("KASSIBER_TEST_PART", None)
        if part:
            env["KASSIBER_TEST_PART"] = part
        result = subprocess.run(
            [sys.executable, "-m", "pytest", "--collect-only", "-q", "-p", "no:cacheprovider",
             "tests/test_ci_shards.py"],
            cwd=ROOT, env=env, capture_output=True, text=True, timeout=120,
        )
        assert result.returncode == 0, result.stdout + result.stderr
        return [line for line in result.stdout.splitlines() if "::" in line]

    whole = collected()
    parts = [collected(f"{index}/3") for index in (1, 2, 3)]
    assert sorted(node for part in parts for node in part) == sorted(whole)
    assert all(parts), "every part keeps some of these tests"
    with pytest.raises(SystemExit):
        python_test_shards.main(["run", "serial-daemon", "--part", "4/3"])
    calls = []
    with mock.patch.object(python_test_shards, "_run_pytest", lambda paths, **options: calls.append(os.environ.get("KASSIBER_TEST_PART")) or 0):
        with mock.patch.dict(os.environ):
            python_test_shards.main(["run-shadow", "serial-daemon", "--ci", "--part", "2/8"])
    assert calls == ["2/8"]
