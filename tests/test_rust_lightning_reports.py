"""Retain Python report-kernel oracles and native mutation coverage."""
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def test_lightning_report_kernel_vectors_match_unchanged_python():
    subprocess.run([sys.executable, str(ROOT / "crates/reports/scripts/gen_lightning_vectors.py"), "--check"],
                   cwd=ROOT, check=True, capture_output=True, text=True, timeout=120)


def test_lightning_report_kernel_reference_quirks_remain_reproducible():
    subprocess.run([sys.executable, str(ROOT / "crates/reports/scripts/repro_python_lightning.py")],
                   cwd=ROOT, check=True, capture_output=True, text=True, timeout=180)


def test_lightning_service_mutation_catalog_applies():
    subprocess.run([sys.executable, str(ROOT / "crates/kassiber/scripts/lightning_mutation_check.py"), "--check"],
                   cwd=ROOT, check=True, capture_output=True, text=True, timeout=60)
