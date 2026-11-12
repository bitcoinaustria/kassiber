import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

import pytest

from tests.rust_sync_env import child_env
from tests.test_rust_analysis_index import canonical, tables
from tests.rust_port.repro_analysis_source_script import reproduce

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "crates/wallets/scripts"))
SPEC = importlib.util.spec_from_file_location("analysis_source_vectors", ROOT / "crates/wallets/scripts/generate_analysis_sources.py")
REFERENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE)


def test_sql_sources_match_unchanged_python_and_all_112_tables(tmp_path):
    cases = REFERENCE.vectors()
    assert len(cases) > 100
    assert json.loads(REFERENCE.DEST.read_text()) == cases
    for case in cases:
        conn = REFERENCE.database(case.get("sql", ""))
        try:
            before = tables(conn)
            variants = case.get("variants", [case["expected"]])
            if "variants" in case:
                env = child_env(tmp_path)
                env["PYTHONHASHSEED"] = "0"
                child = subprocess.run([sys.executable, str(ROOT / "crates/wallets/scripts/generate_analysis_sources.py"), "--evaluate"], input=json.dumps(case).encode(), env=env, capture_output=True, check=True, timeout=30)
                actual = json.loads(child.stdout)
            else:
                actual = REFERENCE.evaluate(case, conn)
            assert canonical(actual) in [canonical(value) for value in variants], case["name"]
            assert tables(conn) == before
        finally:
            conn.close()


@pytest.mark.parametrize("offline", [False, True])
def test_sql_sources_native_bytes_all_tables_without_network_consent(tmp_path, offline):
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
    values = json.loads(result.stdout)
    assert len(values) == len(cases)
    matched = []
    for case, actual in zip(cases, values):
        variants = case.get("variants", [case["expected"]])
        match = next((value for value in variants if canonical(value) == canonical(actual)), None)
        assert match is not None, case["name"]
        matched.append(match)
    assert result.stdout == canonical(matched) + b"\n"


def test_sql_sources_mutation_catalog_stays_applicable(tmp_path):
    catalog = ROOT / "crates/wallets/scripts/analysis_sources_mutations.json"
    entries = json.loads(catalog.read_text())
    result = subprocess.run([sys.executable, str(ROOT / "crates/wallets/scripts/record_mutation_check.py"), "--mutations", str(catalog), "--check"], env=child_env(tmp_path), check=True, capture_output=True, timeout=30)
    assert result.stdout == f"{len(entries)} entries apply\n".encode()
    source = (ROOT / "crates/wallets/src/analysis_index/tests.rs").read_text()
    assert entries
    assert all(f"fn {name}()" in source for entry in entries for name in entry["expect_failing"])


def test_sql_sources_retain_reference_script_reproduction():
    assert reproduce() == {"with_dataset_table": {"error": "validation"}, "without_dataset_table": "accepted"}
