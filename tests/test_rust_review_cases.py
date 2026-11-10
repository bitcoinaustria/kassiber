"""Keep unregistered native review primitives tied to the current Python service."""
import os
from pathlib import Path
import subprocess
import sys

import pytest

ROOT = Path(__file__).resolve().parents[1]

# The exporters stub the ledger build (recorded ids, clocks, injected
# failures) to reproduce their vectors, so under the tax shadow their
# children run production alone, like the `stubs_tax_engine` tests do
# in-process; census still counts their calls.
pytestmark = pytest.mark.stubs_tax_engine


def _child_env():
    env = dict(os.environ)
    if env.get("KASSIBER_TEST_TAX_ENGINE", "").strip().lower() == "shadow":
        del env["KASSIBER_TEST_TAX_ENGINE"]
    return env


def test_review_cases_replay_current_python_builds_cursors_pairs_and_waiting_roots():
    result = subprocess.run(
        [sys.executable, str(ROOT / "crates/transfers/scripts/export_review_cases.py"), "--check"],
        cwd=ROOT, env=_child_env(), text=True, capture_output=True, timeout=120,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def test_review_operation_normalization_and_digests_match_current_python():
    result = subprocess.run(
        [sys.executable, str(ROOT / "crates/transfers/scripts/export_review_operations.py"), "--check"],
        cwd=ROOT, env=_child_env(), text=True, capture_output=True, timeout=120,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def test_review_economic_and_policy_effects_match_current_python():
    result = subprocess.run(
        [sys.executable, str(ROOT / "crates/transfers/scripts/export_review_effects.py"), "--check"],
        cwd=ROOT, env=_child_env(), text=True, capture_output=True, timeout=120,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def test_review_artifact_content_and_admission_match_current_python():
    result = subprocess.run(
        [sys.executable, str(ROOT / "crates/transfers/scripts/export_review_artifacts.py"), "--check"],
        cwd=ROOT, env=_child_env(), text=True, capture_output=True, timeout=120,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def test_review_plans_and_recomputing_validation_match_current_python():
    result = subprocess.run(
        [sys.executable, str(ROOT / "crates/transfers/scripts/export_review_plans.py"), "--check"],
        cwd=ROOT, env=_child_env(), text=True, capture_output=True, timeout=120,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def test_review_atomic_apply_receipts_and_complete_rows_match_current_python():
    result = subprocess.run(
        [sys.executable, str(ROOT / "crates/transfers/scripts/export_review_apply.py"), "--check"],
        cwd=ROOT, env=_child_env(), text=True, capture_output=True, timeout=120,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def test_review_input_requests_match_current_python():
    result = subprocess.run(
        [sys.executable, str(ROOT / "crates/transfers/scripts/export_review_inputs.py"), "--check"],
        cwd=ROOT, env=_child_env(), text=True, capture_output=True, timeout=120,
    )
    assert result.returncode == 0, result.stdout + result.stderr
