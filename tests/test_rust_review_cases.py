"""Keep unregistered native review primitives tied to the current Python service."""
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def test_review_cases_replay_current_python_builds_cursors_pairs_and_waiting_roots():
    result = subprocess.run(
        [sys.executable, str(ROOT / "crates/transfers/scripts/export_review_cases.py"), "--check"],
        cwd=ROOT, text=True, capture_output=True, timeout=120,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def test_review_operation_normalization_and_digests_match_current_python():
    result = subprocess.run(
        [sys.executable, str(ROOT / "crates/transfers/scripts/export_review_operations.py"), "--check"],
        cwd=ROOT, text=True, capture_output=True, timeout=120,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def test_review_economic_and_policy_effects_match_current_python():
    result = subprocess.run(
        [sys.executable, str(ROOT / "crates/transfers/scripts/export_review_effects.py"), "--check"],
        cwd=ROOT, text=True, capture_output=True, timeout=120,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def test_review_artifact_content_and_admission_match_current_python():
    result = subprocess.run(
        [sys.executable, str(ROOT / "crates/transfers/scripts/export_review_artifacts.py"), "--check"],
        cwd=ROOT, text=True, capture_output=True, timeout=120,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def test_review_plans_and_recomputing_validation_match_current_python():
    result = subprocess.run(
        [sys.executable, str(ROOT / "crates/transfers/scripts/export_review_plans.py"), "--check"],
        cwd=ROOT, text=True, capture_output=True, timeout=120,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def test_review_atomic_apply_receipts_and_complete_rows_match_current_python():
    result = subprocess.run(
        [sys.executable, str(ROOT / "crates/transfers/scripts/export_review_apply.py"), "--check"],
        cwd=ROOT, text=True, capture_output=True, timeout=120,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def test_review_input_requests_match_current_python():
    result = subprocess.run(
        [sys.executable, str(ROOT / "crates/transfers/scripts/export_review_inputs.py"), "--check"],
        cwd=ROOT, text=True, capture_output=True, timeout=120,
    )
    assert result.returncode == 0, result.stdout + result.stderr
