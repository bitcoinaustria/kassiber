import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

import pytest

from tests.rust_sync_env import child_env
from tests.rust_port.repro_analysis_legacy import reproduce
from tests.test_rust_analysis_index import canonical, tables

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "crates/wallets/scripts"))
SPEC = importlib.util.spec_from_file_location("analysis_legacy_vectors", ROOT / "crates/wallets/scripts/generate_analysis_legacy.py")
REFERENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE)


def test_legacy_json_matches_unchanged_python_and_all_112_tables():
    cases = REFERENCE.vectors()
    assert len(cases) > 400
    frozen = json.loads(REFERENCE.DEST.read_text())
    assert canonical(frozen) == canonical(cases)
    for case in cases:
        conn = REFERENCE.database(case["sql"])
        try:
            before = tables(conn)
            assert canonical(REFERENCE.evaluate(case, conn)) == canonical(case["expected"]), case["name"]
            assert tables(conn) == before
        finally:
            conn.close()


@pytest.mark.parametrize("offline", [False, True])
def test_legacy_json_native_bytes_all_tables_without_network_consent(tmp_path, offline):
    executable = os.environ.get("KASSIBER_ANALYSIS_INDEX_BIN")
    if not executable:
        pytest.skip("requires analysis_index_fixture test-harness build")
    cases = json.loads(REFERENCE.DEST.read_text())
    env = child_env(tmp_path)
    env["KASSIBER_TEST_NO_EGRESS"] = "strict"
    if offline:
        env["KASSIBER_NO_EGRESS"] = "1"
    else:
        env.pop("KASSIBER_NO_EGRESS", None)
    result = subprocess.run([str(Path(executable).resolve())], input=json.dumps(cases).encode(), env=env, capture_output=True, check=True, timeout=120)
    assert result.stderr == b""
    assert result.stdout == canonical([case["expected"] for case in cases]) + b"\n"


def test_legacy_json_mutation_catalog_stays_applicable(tmp_path):
    catalog = ROOT / "crates/wallets/scripts/analysis_legacy_mutations.json"
    entries = json.loads(catalog.read_text())
    result = subprocess.run([sys.executable, str(ROOT / "crates/wallets/scripts/record_mutation_check.py"), "--mutations", str(catalog), "--check"], env=child_env(tmp_path), check=True, capture_output=True, timeout=30)
    assert result.stdout == f"{len(entries)} entries apply\n".encode()
    source = (ROOT / "crates/wallets/src/analysis_index/tests.rs").read_text()
    assert entries
    assert all(f"fn {name}()" in source for entry in entries for name in entry["expect_failing"])


def test_legacy_json_retains_reference_manifest_failure_reproduction():
    assert reproduce() == {"non_finite_manifest": "ValueError", "surrogate_manifest": "UnicodeEncodeError"}
