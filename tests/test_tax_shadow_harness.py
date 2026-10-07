"""The tax-shadow harness reaches child processes and fails what it must (plan 20).

`KASSIBER_TEST_TAX_ENGINE=shadow` compares every production ledger build with
the frozen oracle, in the test process and, through `tests/_child_hooks/`, in
the CLI and daemon children a test spawns. A difference
`KASSIBER_TEST_TAX_SHADOW_INJECT_DIFFERENCE` injects must fail a
`journals process` child, and the child's route record must reach the
session, which fails at its end even when a caller swallowed the error.
`census` records the calls without comparing and fails a session whose
engine-reaching module is missing from the overlay.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path
from unittest import mock

from tests import tax_engine_compare as compare

ROOT = Path(__file__).resolve().parents[1]
MODULE = "tests/test_tax_shadow_harness.py"
_SESSION_VARIABLES = (
    "KASSIBER_TEST_TAX_ENGINE",
    compare.RECORDS_ENV,
    compare.MODULE_ENV,
    compare.INJECT_DIFFERENCE_ENV,
)
_BOOK_CSV = (
    "date,txid,direction,asset,amount,fee,fiat_rate,description,kind\n"
    "2026-01-01T10:00:00Z,buy-1,inbound,BTC,0.10000000,0,60000,Buy,buy\n"
    "2026-03-01T10:00:00Z,sell-1,outbound,BTC,0.02000000,0.00001000,70000,Sell,sell\n"
)


def _child_env(mode: str | None = None, *, records: Path | None = None, inject: bool = False) -> dict[str, str]:
    """A child environment with this test's own tax mode, records file and module."""
    env = {key: value for key, value in os.environ.items() if key not in _SESSION_VARIABLES}
    env["PYTHONPATH"] = os.pathsep.join(
        path for path in (str(ROOT / "tests" / "_child_hooks"), str(ROOT), os.environ.get("PYTHONPATH")) if path
    )
    if mode is not None:
        env["KASSIBER_TEST_TAX_ENGINE"] = mode
        env[compare.MODULE_ENV] = MODULE
        if records is not None:
            env[compare.RECORDS_ENV] = str(records)
    if inject:
        env[compare.INJECT_DIFFERENCE_ENV] = "1"
    return env


class ChildProcessShadowTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls._tmp = tempfile.TemporaryDirectory(prefix="kassiber-tax-shadow-harness-")
        cls.tmp_path = Path(cls._tmp.name)
        cls.data_root = cls.tmp_path / "data"
        csv_path = cls.tmp_path / "book.csv"
        csv_path.write_text(_BOOK_CSV, encoding="utf-8")
        for args in (
            ("init",),
            ("workspaces", "create", "Main"),
            ("profiles", "create", "--workspace", "Main", "--fiat-currency", "USD", "--tax-country", "generic", "Default"),
            ("wallets", "create", "--workspace", "Main", "--profile", "Default", "--label", "W1", "--kind", "custom"),
            ("wallets", "import-csv", "--workspace", "Main", "--profile", "Default", "--wallet", "W1", "--file", str(csv_path)),
        ):
            result = cls._kassiber(*args, env=_child_env())
            if result.returncode != 0:
                raise AssertionError(f"{args} failed: {result.stdout[-2000:]} {result.stderr[-2000:]}")

    @classmethod
    def tearDownClass(cls):
        cls._tmp.cleanup()

    @classmethod
    def _kassiber(cls, *args: str, env: dict[str, str], python: tuple[str, ...] = ()) -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, *python, "-m", "kassiber", "--data-root", str(cls.data_root), "--machine", *args],
            cwd=ROOT,
            env=env,
            capture_output=True,
            text=True,
            check=False,
            timeout=300,
        )

    def _process_journals(
        self, mode: str, *, inject: bool = False, python: tuple[str, ...] = ()
    ) -> tuple[subprocess.CompletedProcess, list]:
        with tempfile.TemporaryDirectory(prefix="kassiber-tax-shadow-records-") as scratch:
            records = Path(scratch) / "routes.jsonl"
            records.touch()
            result = self._kassiber(
                "journals", "process", "--workspace", "Main", "--profile", "Default",
                env=_child_env(mode, records=records, inject=inject),
                python=python,
            )
            return result, compare.read_route_records(str(records))

    def test_shadow_child_compares_and_records_its_route(self):
        # The harness imports the oracle adapter only to run it, so finding
        # it in the child's import log shows the child ran the oracle; census
        # records the same route without it.
        for mode, ran_oracle in ((compare.SHADOW, True), (compare.CENSUS, False)):
            with self.subTest(mode=mode):
                result, records = self._process_journals(mode, python=("-X", "importtime"))
                self.assertEqual(result.returncode, 0, result.stdout[-2000:] + result.stderr[-2000:])
                self.assertEqual(json.loads(result.stdout)["data"]["entries_created"], 2)
                self.assertEqual(records, [compare.RouteRecord(MODULE, compare.UNCLASSIFIED, "legacy", "result")])
                imported = re.search(r"^import time:.*\|\s*tests\.tax_oracle\.adapter$", result.stderr, re.MULTILINE)
                self.assertEqual(imported is not None, ran_oracle)

    def test_injected_difference_fails_the_journals_process_child(self):
        result, records = self._process_journals(compare.SHADOW, inject=True)
        self.assertEqual(result.returncode, 1, result.stdout[-2000:] + result.stderr[-2000:])
        message = json.loads(result.stdout)["error"]["message"]
        self.assertIn("production tax engine differs from the oracle in build_ledger_state", message)
        self.assertIn(f"injected by {compare.INJECT_DIFFERENCE_ENV}", message)
        self.assertIn("result['tax_summary']: length", message)
        self.assertEqual(records, [compare.RouteRecord(MODULE, compare.UNCLASSIFIED, "legacy", "difference")])
        _lines, failures = compare.session_report(compare.SHADOW, records)
        self.assertEqual(failures, [f"calls that differed from the oracle: 1, in {MODULE}"])

    def test_census_child_records_without_comparing(self):
        # The injection only alters what shadow compares; census compares nothing.
        result, records = self._process_journals(compare.CENSUS, inject=True)
        self.assertEqual(result.returncode, 0, result.stdout[-2000:] + result.stderr[-2000:])
        self.assertEqual(records, [compare.RouteRecord(MODULE, compare.UNCLASSIFIED, "legacy", "result")])

    def test_a_child_loads_the_harness_only_with_the_engines(self):
        probe = textwrap.dedent(
            """
            import json
            import sys

            import kassiber

            before = "tests.tax_engine_compare" in sys.modules
            import kassiber.core.engines

            loaded = sys.modules.get("tests.tax_engine_compare")
            patch = loaded.PRODUCTION_ENGINE.build_ledger_state.__qualname__ if loaded else None
            print(json.dumps([before, patch]))
            """
        )
        expected = {
            None: [False, None],
            compare.SHADOW: [False, "shadow_build_ledger_state.<locals>.build_ledger_state"],
            compare.CENSUS: [False, "census_build_ledger_state.<locals>.build_ledger_state"],
            # The other modes leave children on production.
            compare.ORACLE: [False, None],
        }
        for mode, outcome in expected.items():
            with self.subTest(mode=mode):
                result = subprocess.run(
                    [sys.executable, "-c", probe],
                    cwd=self.tmp_path,
                    env=_child_env(mode),
                    capture_output=True,
                    text=True,
                    check=False,
                    timeout=300,
                )
                self.assertEqual(result.returncode, 0, result.stderr[-2000:])
                self.assertEqual(json.loads(result.stdout.strip().splitlines()[-1]), outcome)

    def test_a_child_that_imports_the_harness_first_gets_the_mode(self):
        # The harness loads the engines halfway through its own import, so
        # the hook installs the mode once the harness has finished; a child
        # that is not a pytest session has no `conftest.py` to install it.
        probe = textwrap.dedent(
            """
            import json

            import tests.tax_engine_compare as harness

            installed = harness._INSTALLED_MODE[0] if harness._INSTALLED_MODE else None
            print(json.dumps([installed, harness.PRODUCTION_ENGINE.build_ledger_state.__qualname__]))
            """
        )
        expected = {
            None: [None, "GenericRP2TaxEngine.build_ledger_state"],
            compare.SHADOW: [compare.SHADOW, "shadow_build_ledger_state.<locals>.build_ledger_state"],
            compare.CENSUS: [compare.CENSUS, "census_build_ledger_state.<locals>.build_ledger_state"],
            compare.ORACLE: [None, "GenericRP2TaxEngine.build_ledger_state"],
        }
        for mode, outcome in expected.items():
            with self.subTest(mode=mode):
                result = subprocess.run(
                    [sys.executable, "-c", probe],
                    cwd=self.tmp_path,
                    env=_child_env(mode),
                    capture_output=True,
                    text=True,
                    check=False,
                    timeout=300,
                )
                self.assertEqual(result.returncode, 0, result.stderr[-2000:])
                self.assertEqual(json.loads(result.stdout.strip().splitlines()[-1]), outcome)

    def test_a_child_that_imports_the_oracle_first_still_starts(self):
        # The oracle adapter loads the engines package halfway through its
        # own import, so the hook must not need the oracle to install a mode.
        probe = "import tests.tax_oracle.adapter as adapter; print(adapter.OracleTaxEngine.__name__)"
        for mode in compare.RECORDING_MODES:
            with self.subTest(mode=mode):
                result = subprocess.run(
                    [sys.executable, "-c", probe],
                    cwd=self.tmp_path,
                    env=_child_env(mode),
                    capture_output=True,
                    text=True,
                    check=False,
                    timeout=300,
                )
                self.assertEqual(result.returncode, 0, result.stderr[-2000:])
                self.assertEqual(result.stdout.strip(), "OracleTaxEngine")


class SessionEndTest(unittest.TestCase):
    """Whole pytest sessions, with `tests/conftest.py` loaded as a plugin."""

    TEST = textwrap.dedent(
        """
        from tests.custody_tax_helpers import finalized_tax_inputs
        from tests.tax_engine_compare import PRODUCTION_ENGINE
        from tests.tax_ledger_generator import WALLET_REFS, ledger_history as _ledger_history, profile as _adapter_profile


        def test_builds_a_ledger_and_swallows_assertions():
            profile = _adapter_profile("generic", "FIFO")
            rows, pairs = _ledger_history()
            inputs = finalized_tax_inputs(profile, rows=rows, wallet_refs_by_id=WALLET_REFS, manual_pair_records=pairs)
            try:
                PRODUCTION_ENGINE(profile).build_ledger_state(inputs)
            except AssertionError:
                pass
        """
    )

    def _session(self, mode: str, *, inject: bool = False, test: str = TEST) -> subprocess.CompletedProcess:
        with tempfile.TemporaryDirectory(prefix="kassiber-tax-shadow-session-") as scratch:
            test_file = Path(scratch) / "test_shadow_session_probe.py"
            test_file.write_text(test, encoding="utf-8")
            env = _child_env(inject=inject)
            env["KASSIBER_TEST_TAX_ENGINE"] = mode
            return subprocess.run(
                [sys.executable, "-m", "pytest", "-p", "tests.conftest", "-p", "no:cacheprovider", "-q", str(test_file)],
                cwd=scratch,
                env=env,
                capture_output=True,
                text=True,
                check=False,
                timeout=600,
            )

    def test_shadow_session_prints_its_route_counts(self):
        result = self._session(compare.SHADOW)
        self.assertEqual(result.returncode, 0, result.stdout[-4000:] + result.stderr[-2000:])
        self.assertIn("1 passed", result.stdout)
        self.assertIn("tax engine shadow", result.stdout)
        self.assertIn("production calls compared with the oracle: 1", result.stdout)
        self.assertIn("generic: 0 v2, 0 legacy", result.stdout)
        self.assertIn("unclassified: 0 v2, 1 legacy", result.stdout)

    def test_a_swallowed_difference_still_fails_the_session(self):
        result = self._session(compare.SHADOW, inject=True)
        self.assertEqual(result.returncode, 1, result.stdout[-4000:] + result.stderr[-2000:])
        self.assertIn("1 passed", result.stdout)
        self.assertIn("FAILED: calls that differed from the oracle: 1, in /", result.stdout)

    def test_a_test_that_stubs_the_engine_runs_uncompared(self):
        # Both tests see the injected difference; only the unmarked one is compared.
        stubbed = self.TEST + textwrap.dedent(
            """

            import pytest


            @pytest.mark.stubs_tax_engine
            def test_stubs_the_engine():
                profile = _adapter_profile("generic", "FIFO")
                rows, pairs = _ledger_history()
                inputs = finalized_tax_inputs(profile, rows=rows, wallet_refs_by_id=WALLET_REFS, manual_pair_records=pairs)
                assert PRODUCTION_ENGINE(profile).build_ledger_state(inputs).entries
            """
        )
        result = self._session(compare.SHADOW, inject=True, test=stubbed)
        self.assertEqual(result.returncode, 1, result.stdout[-4000:] + result.stderr[-2000:])
        self.assertIn("2 passed", result.stdout)
        self.assertIn("production calls compared with the oracle: 1", result.stdout)
        self.assertIn("unclassified: 0 v2, 2 legacy", result.stdout)
        self.assertIn("calls not compared because the test stubs the engine: 1, in /", result.stdout)
        self.assertIn("FAILED: calls that differed from the oracle: 1, in /", result.stdout)

    def test_census_fails_an_engine_caller_outside_the_overlay(self):
        result = self._session(compare.CENSUS)
        self.assertEqual(result.returncode, 1, result.stdout[-4000:] + result.stderr[-2000:])
        self.assertIn("1 passed", result.stdout)
        self.assertIn("production calls: 1, from test modules: 1", result.stdout)
        self.assertIn("are not in the shadow overlay (scripts/python_test_shards.py): /", result.stdout)
        self.assertIn("test_shadow_session_probe.py", result.stdout)


class SessionReportTest(unittest.TestCase):
    def _records(self, *rows: tuple) -> list:
        return [compare.RouteRecord(*row) for row in rows]

    def test_shadow_counts_routes_per_capability(self):
        records = self._records(
            ("tests/test_a.py", "generic", "legacy", "result"),
            ("tests/test_a.py", "generic", "v2", "result"),
            ("tests/test_b.py", "at_swap", "legacy", "error"),
            ("tests/test_b.py", compare.UNCLASSIFIED, "legacy", "result"),
        )
        lines, failures = compare.session_report(compare.SHADOW, records, routed=frozenset())
        self.assertEqual(
            lines,
            [
                "production calls compared with the oracle: 4",
                "generic: 1 v2, 1 legacy",
                "generic_carry: 0 v2, 0 legacy",
                "at: 0 v2, 0 legacy",
                "at_swap: 0 v2, 1 legacy",
                "unclassified: 0 v2, 1 legacy",
            ],
        )
        self.assertEqual(failures, [])

    def test_shadow_fails_a_routed_capability_on_the_legacy_adapter(self):
        records = self._records(
            ("tests/test_a.py", "generic", "v2", "result"),
            ("tests/test_a.py", "at", "legacy", "result"),
            ("tests/test_a.py", "at", "legacy", "error"),
        )
        _lines, failures = compare.session_report(compare.SHADOW, records, routed=frozenset({"generic"}))
        self.assertEqual(failures, [])
        _lines, failures = compare.session_report(compare.SHADOW, records, routed=frozenset({"generic", "at"}))
        # A routed capability that never reached v2 fails the same way.
        self.assertEqual(failures, ["2 of 2 calls of routed capability 'at' went to the legacy adapter"])

    def test_shadow_lists_uncompared_calls_without_failing(self):
        records = self._records(
            ("tests/test_a.py", compare.UNCLASSIFIED, "legacy", "uncompared"),
            ("tests/test_a.py", compare.UNCLASSIFIED, "legacy", "result"),
        )
        lines, failures = compare.session_report(compare.SHADOW, records)
        self.assertEqual(lines[0], "production calls compared with the oracle: 1")
        self.assertEqual(lines[-1], "calls not compared because the test stubs the engine: 1, in tests/test_a.py")
        self.assertEqual(failures, [])

    def test_shadow_fails_on_any_difference(self):
        records = self._records(
            ("tests/test_b.py", "generic", "legacy", "difference"),
            ("tests/test_a.py", "generic", "legacy", "result"),
            (None, "generic", "legacy", "difference"),
        )
        _lines, failures = compare.session_report(compare.SHADOW, records)
        self.assertEqual(failures, ["calls that differed from the oracle: 2, in outside any test, tests/test_b.py"])

    def test_census_fails_modules_missing_from_the_overlay_and_unplaced_calls(self):
        records = self._records(
            ("tests/test_a.py", compare.UNCLASSIFIED, "legacy", "result"),
            ("tests/test_b.py", compare.UNCLASSIFIED, "legacy", "error"),
        )
        lines, failures = compare.session_report(compare.CENSUS, records, overlay=["tests/test_a.py", "tests/test_b.py"])
        self.assertEqual((lines, failures), (["production calls: 2, from test modules: 2"], []))
        records.append(compare.RouteRecord("tests/test_c.py", compare.UNCLASSIFIED, "legacy", "result"))
        records.append(compare.RouteRecord(None, compare.UNCLASSIFIED, "legacy", "result"))
        _lines, failures = compare.session_report(compare.CENSUS, records, overlay=["tests/test_a.py", "tests/test_b.py"])
        self.assertEqual(
            failures,
            [
                "these test modules reach the tax engine but are not in the shadow overlay"
                " (scripts/python_test_shards.py): tests/test_c.py",
                "production calls outside any test, which the census cannot place: 1",
            ],
        )

    def test_records_hold_only_the_route(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-tax-shadow-records-") as scratch:
            records = Path(scratch) / "routes.jsonl"
            with mock.patch.dict(os.environ, {compare.RECORDS_ENV: str(records), compare.MODULE_ENV: MODULE}):
                compare.record_route("generic", "v2", "result")
            with mock.patch.dict(os.environ, {compare.RECORDS_ENV: str(records)}):
                os.environ.pop(compare.MODULE_ENV, None)
                compare.record_route("at", "legacy", "error")
            lines = records.read_text(encoding="utf-8").splitlines()
            self.assertEqual(
                [json.loads(line) for line in lines],
                [
                    {"capability": "generic", "module": MODULE, "outcome": "result", "route": "v2"},
                    {"capability": "at", "module": None, "outcome": "error", "route": "legacy"},
                ],
            )
            self.assertEqual(
                compare.read_route_records(str(records)),
                [
                    compare.RouteRecord(MODULE, "generic", "v2", "result"),
                    compare.RouteRecord(None, "at", "legacy", "error"),
                ],
            )
        with mock.patch.dict(os.environ):
            os.environ.pop(compare.RECORDS_ENV, None)
            compare.record_route("generic", "v2", "result")  # no records file: nothing to write

    def test_only_the_known_modes_install(self):
        self.assertEqual(compare.TAX_MODES, ("shadow", "census", "oracle", "legacy-native"))
        self.assertEqual(compare.RECORDING_MODES, ("shadow", "census"))
        for mode in ("rp2", "native", ""):
            with self.subTest(mode=mode), self.assertRaises(ValueError):
                compare._tax_mode_patch(mode)

    def test_only_the_listed_tests_stub_the_engine(self):
        # A `stubs_tax_engine` test runs uncompared under shadow, so adding
        # one must be a deliberate change to this list.
        harness = {"tests/conftest.py", "tests/tax_engine_compare.py", MODULE}
        users = {}
        for path in sorted((ROOT / "tests").rglob("*.py")):
            name = path.relative_to(ROOT).as_posix()
            count = path.read_text(encoding="utf-8").count("stubs_tax_engine")
            if count and name not in harness:
                users[name] = count
        # The spy replaces the phase-3 adapter's preparation and RP2
        # configuration. A spy that only observes needs no marker.
        self.assertEqual(users, {"tests/test_custody_pre_tax_projection.py": 1})


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
