import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

import pytest

from tests.rust_sync_env import child_env
from tests.rust_port.book_schema import SEED_TABLES

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("analysis_durable_vectors", ROOT / "crates/wallets/scripts/generate_analysis_durable.py")
REFERENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE)
SYNC_SPEC = importlib.util.spec_from_file_location("analysis_sync_vectors", ROOT / "crates/wallets/scripts/generate_analysis_synchronize.py")
SYNC = importlib.util.module_from_spec(SYNC_SPEC)
SYNC_SPEC.loader.exec_module(SYNC)


def test_durable_sources_match_unchanged_python():
    cases = REFERENCE.vectors()
    assert len(cases) >= 15
    assert json.loads(REFERENCE.DEST.read_text()) == cases
    assert all(step["state"]["table_count"] == len(SEED_TABLES) for case in cases for step in case["expected"])


def test_incremental_synchronization_matches_unchanged_python():
    cases = SYNC.vectors()
    assert len(cases) >= 35
    assert json.loads(SYNC.DEST.read_text()) == cases
    assert all(step["state"]["table_count"] == len(SEED_TABLES) for case in cases for action, step in zip(case["actions"], case["expected"]) if action["operation"] == "synchronize")
    assert all(len(SEED_TABLES) - 1 <= step["state"]["table_count"] <= len(SEED_TABLES) for case in cases for step in case["expected"])


@pytest.mark.parametrize("offline", [False, True])
def test_durable_native_bytes_all_tables_without_network_consent(tmp_path, offline):
    executable = os.environ.get("KASSIBER_ANALYSIS_DURABLE_BIN")
    if not executable:
        pytest.skip("requires analysis_durable_fixture test-harness build")
    cases = json.loads(REFERENCE.DEST.read_text()) + json.loads(SYNC.DEST.read_text())
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
    for name in ("analysis_durable_mutations.json", "analysis_synchronize_mutations.json", "analysis_recovery_mutations.json"):
        catalog = ROOT / "crates/wallets/scripts" / name
        entries = json.loads(catalog.read_text())
        result = subprocess.run([sys.executable, str(ROOT / "crates/wallets/scripts/record_mutation_check.py"), "--mutations", str(catalog), "--check"], env=child_env(tmp_path), check=True, capture_output=True, timeout=30)
        assert result.stdout == f"{len(entries)} entries apply\n".encode()
