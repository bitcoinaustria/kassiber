import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

import pytest

from tests.rust_sync_env import child_env

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("analysis_durable_vectors", ROOT / "crates/wallets/scripts/generate_analysis_durable.py")
REFERENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE)


def test_durable_sources_match_unchanged_python():
    cases = REFERENCE.vectors()
    assert len(cases) >= 15
    assert json.loads(REFERENCE.DEST.read_text()) == cases
    assert all(step["state"]["table_count"] == 112 for case in cases for step in case["expected"])


@pytest.mark.parametrize("offline", [False, True])
def test_durable_native_bytes_all_tables_without_network_consent(tmp_path, offline):
    executable = os.environ.get("KASSIBER_ANALYSIS_DURABLE_BIN")
    if not executable:
        pytest.skip("requires analysis_durable_fixture test-harness build")
    cases = json.loads(REFERENCE.DEST.read_text())
    env = child_env(tmp_path)
    env["KASSIBER_TEST_NO_EGRESS"] = "strict"
    if offline:
        env["KASSIBER_NO_EGRESS"] = "1"
    else:
        env.pop("KASSIBER_NO_EGRESS", None)
    result = subprocess.run([str(Path(executable).resolve())], input=json.dumps(cases).encode(), env=env, capture_output=True, check=True, timeout=120)
    assert result.stderr == b""
    expected = [case["expected"] for case in cases]
    assert result.stdout == REFERENCE.canonical(expected).encode() + b"\n"


def test_durable_mutation_catalog_stays_applicable(tmp_path):
    catalog = ROOT / "crates/wallets/scripts/analysis_durable_mutations.json"
    entries = json.loads(catalog.read_text())
    result = subprocess.run([sys.executable, str(ROOT / "crates/wallets/scripts/record_mutation_check.py"), "--mutations", str(catalog), "--check"], env=child_env(tmp_path), check=True, capture_output=True, timeout=30)
    assert result.stdout == f"{len(entries)} entries apply\n".encode()
