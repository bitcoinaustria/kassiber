"""Immutable-index prerequisite oracle; Mirror product routing remains closed."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

import pytest

from tests.rust_sync_env import child_env
from tests.rust_port.repro_analysis_index_hashseed import variants
from tests.rust_port.book_schema import SEED_TABLES

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("analysis_index_vectors", ROOT / "crates/wallets/scripts/generate_analysis_index.py")
REFERENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE)
SEEDS = ROOT / "crates/wallets/tests/data/analysis_index_hashseeds.json"


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def tables(conn, schema=SEED_TABLES):
    """Every table's rows; seed-built books keep the frozen seed's table count
    (a case may rename one, as `missing-wallets` does)."""
    names = [r[0] for r in conn.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
    assert len(names) == len(schema)
    return {name: sorted((tuple(r) for r in conn.execute(f'SELECT * FROM "{name}"')), key=repr) for name in names}


def test_analysis_index_frozen_vectors_match_unchanged_python_and_all_tables():
    cases = REFERENCE.vectors()
    assert len(cases) == 195
    assert json.loads(REFERENCE.DEST.read_text()) == cases
    for case in cases:
        conn = REFERENCE.database(case.get("sql", ""))
        try:
            before = tables(conn)
            assert canonical(REFERENCE.evaluate(case, conn)) == canonical(case["expected"]), case["name"]
            assert tables(conn) == before
        finally:
            conn.close()


def test_analysis_index_reference_hashseed_variants_remain_recorded():
    recorded = json.loads(SEEDS.read_text())
    assert variants() == recorded
    assert len({canonical(v["finding_node_ids"]) for v in recorded.values()}) > 1
    assert len({v["snapshot_id"] for v in recorded.values()}) == 1


def test_analysis_index_mutation_catalog_stays_applicable(tmp_path):
    catalog = ROOT / "crates/wallets/scripts/analysis_index_mutations.json"
    entries = json.loads(catalog.read_text())
    result = subprocess.run([sys.executable, str(ROOT / "crates/wallets/scripts/record_mutation_check.py"), "--mutations", str(catalog), "--check"], env=child_env(tmp_path), check=True, capture_output=True, timeout=30)
    assert result.stdout == f"{len(entries)} entries apply\n".encode()
    source = (ROOT / "crates/wallets/src/analysis_index/tests.rs").read_text()
    assert entries
    assert all(f"fn {name}()" in source for entry in entries for name in entry["expect_failing"])


@pytest.mark.parametrize("offline", [False, True])
def test_analysis_index_native_bytes_all_tables_without_network_consent(tmp_path, offline):
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
    result = subprocess.run([str(Path(executable).resolve())], input=json.dumps(cases).encode(), env=env, capture_output=True, check=True, timeout=90)
    assert result.stderr == b""
    # The native driver additionally compares all 114 tables around every case.
    assert result.stdout == canonical([c["expected"] for c in cases]) + b"\n"
