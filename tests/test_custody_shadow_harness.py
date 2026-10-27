"""A custody discrepancy must fail children and sessions that catch errors."""
import json
import os
from pathlib import Path
import subprocess
import sys
import textwrap

import pytest

from tests import custody_engine_compare as compare
from tests import test_tax_shadow_harness as tax_harness

ROOT = Path(__file__).resolve().parents[1]


def child_env(mode="shadow", *, inject=False):
    env = {key: value for key, value in os.environ.items() if not key.startswith("KASSIBER_TEST_CUSTODY_")}
    env[compare.MODE_ENV] = mode
    env["PYTHONPATH"] = os.pathsep.join((str(ROOT / "tests/_child_hooks"), str(ROOT)))
    if inject:
        env[compare.INJECT_ENV] = "evidence"
    return env


@pytest.fixture(scope="module")
def cli_book():
    cls = tax_harness.ChildProcessShadowTest
    cls.setUpClass()
    try:
        yield cls
    finally:
        cls.tearDownClass()


@pytest.mark.parametrize("mode", ["shadow", "census"])
@pytest.mark.parametrize("inject", [False, True])
def test_custody_child_compares_and_records_differences(cli_book, tmp_path, mode, inject):
    records = tmp_path / "records.jsonl"
    env = child_env(mode, inject=inject)
    env[compare.RECORDS_ENV] = str(records)
    env[compare.MODULE_ENV] = __file__
    result = cli_book._kassiber("journals", "process", "--workspace", "Main", "--profile", "Default", env=env)
    assert result.returncode == (1 if inject else 0), result.stdout[-2000:] + result.stderr[-2000:]
    routes = compare.read_records(records)
    assert len(routes) == 1
    assert routes[0]["outcome"] == ("difference" if inject else "result")
    assert routes[0]["compared"] == ["evidence"]
    assert bool(compare.session_report(routes)[1]) == inject


def test_custody_difference_cannot_be_swallowed_by_a_test(tmp_path):
    probe = tmp_path / "test_caught_custody.py"
    probe.write_text(textwrap.dedent('''
        from kassiber.db import open_db
        from kassiber.core.custody_journal import CustodyJournalBuilder
        from tests.test_custody_native_transitions import seed_native_transition_book

        def test_catches_difference(tmp_path):
            conn = open_db(tmp_path)
            try:
                seed_native_transition_book(conn, "claim")
                profile = conn.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()
                try:
                    CustodyJournalBuilder(conn, profile, as_of="2026-01-01T00:00:00Z").build_custody_projection()
                except AssertionError:
                    pass
            finally:
                conn.close()
    '''))
    result = subprocess.run([sys.executable, "-m", "pytest", "-p", "tests.conftest", "-p", "no:cacheprovider", "-q", str(probe)], cwd=tmp_path, env=child_env(inject=True), capture_output=True, text=True, timeout=120)
    assert result.returncode == 1, result.stdout + result.stderr
    assert "1 passed" in result.stdout
    assert "FAILED: custody differences: 1" in result.stdout


def test_unknown_custody_modes_fail_closed():
    with pytest.raises(ValueError):
        compare.install_custody_mode("native")
