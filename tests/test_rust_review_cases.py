"""Keep the unregistered native case reader tied to the current Python service."""
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
