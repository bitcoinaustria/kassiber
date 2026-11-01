import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

import pytest

from tests.rust_sync_env import child_env
from tests.test_rust_analysis_index import canonical

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("analysis_entropy_vectors", ROOT / "crates/wallets/scripts/generate_analysis_entropy.py")
REFERENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE)


def test_analysis_entropy_vectors_match_unchanged_python():
    vectors = REFERENCE.vectors()
    assert len(vectors["cases"]) >= 100
    assert json.loads(REFERENCE.DEST.read_text()) == vectors
    for case in vectors["cases"]:
        assert canonical(REFERENCE.evaluate(case, vectors["snapshots"])) == canonical(case["expected"]), case["name"]


@pytest.mark.parametrize("offline", [False, True])
def test_analysis_entropy_native_bytes_without_network_consent(tmp_path, offline):
    executable = os.environ.get("KASSIBER_ANALYSIS_ENTROPY_BIN")
    if not executable:
        pytest.skip("requires analysis_entropy_fixture test-harness build")
    vectors = json.loads(REFERENCE.DEST.read_text())
    env = child_env(tmp_path)
    env["KASSIBER_TEST_NO_EGRESS"] = "strict"
    if offline:
        env["KASSIBER_NO_EGRESS"] = "1"
    else:
        env.pop("KASSIBER_NO_EGRESS", None)
    result = subprocess.run([str(Path(executable).resolve())], input=json.dumps(vectors).encode(), env=env, capture_output=True, check=True, timeout=90)
    assert result.stderr == b""
    assert result.stdout == canonical([case["expected"] for case in vectors["cases"]]) + b"\n"


def test_analysis_entropy_mutation_catalog_stays_applicable(tmp_path):
    catalog = ROOT / "crates/wallets/scripts/analysis_entropy_mutations.json"
    entries = json.loads(catalog.read_text())
    result = subprocess.run([sys.executable, str(ROOT / "crates/wallets/scripts/record_mutation_check.py"), "--mutations", str(catalog), "--check"], env=child_env(tmp_path), check=True, capture_output=True, timeout=30)
    assert result.stdout == f"{len(entries)} entries apply\n".encode()
    source = (ROOT / "crates/wallets/src/analysis_index/entropy/tests.rs").read_text()
    assert entries
    assert all(f"fn {name}()" in source for entry in entries for name in entry["expect_failing"])


def test_analysis_entropy_reference_is_stable_across_recorded_hashseeds(tmp_path):
    for seed in ["0", "1", "7", "42"]:
        env = child_env(tmp_path)
        env["PYTHONHASHSEED"] = seed
        result = subprocess.run([sys.executable, str(ROOT / "crates/wallets/scripts/generate_analysis_entropy.py")], env=env, capture_output=True, check=True, timeout=60)
        assert result.stderr == b""
        assert result.stdout.endswith(b" entropy vectors\n")
