"""Retained public-feature oracle; product Mirror/PSBT routing remains unported."""
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys

import pytest

from tests.rust_sync_env import child_env
from tests.rust_port.book_schema import SEED_TABLES

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("analysis_feature_vectors", ROOT / "crates/wallets/scripts/generate_analysis_features.py")
REFERENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE)


def test_public_feature_frozen_vectors_match_unchanged_python():
    cases = REFERENCE.vectors()
    assert len(cases) >= 1300
    assert json.loads(REFERENCE.DEST.read_text()) == cases
    assert {case["group"] for case in cases} == {"script", "signature", "extract", "normalize", "rules"}


def test_public_feature_mutation_catalog_stays_applicable(tmp_path):
    catalog = ROOT / "crates/wallets/scripts/analysis_features_mutations.json"
    result = subprocess.run([sys.executable, str(ROOT / "crates/wallets/scripts/record_mutation_check.py"),
        "--mutations", str(catalog), "--check"], env=child_env(tmp_path), capture_output=True, check=True, timeout=30)
    entries = json.loads(catalog.read_text())
    assert result.stdout == f"{len(entries)} entries apply\n".encode()
    source = (ROOT / "crates/wallets/src/analysis_features/tests.rs").read_text()
    assert entries
    for entry in entries:
        assert all(f"fn {name}()" in source for name in entry["expect_failing"])


@pytest.mark.parametrize("offline", [False, True])
def test_public_features_native_bytes_all_tables_without_network_consent(tmp_path, offline):
    executable = os.environ.get("KASSIBER_FEATURES_BIN")
    if not executable:
        pytest.skip("requires analysis_features_fixture test-harness build")
    cases = REFERENCE.vectors()
    conn = sqlite3.connect(":memory:")
    conn.executescript((ROOT / "crates/reports/tests/data/privacy_hygiene_seed.sql").read_text())
    conn.executescript((ROOT / "crates/reports/tests/data/book.sql").read_text())

    def tables():
        names = [r[0] for r in conn.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
        assert names == list(SEED_TABLES)
        return {name: sorted(conn.execute(f'SELECT * FROM "{name}"').fetchall(), key=repr) for name in names}

    before = tables()
    env = child_env(tmp_path)
    env["KASSIBER_TEST_NO_EGRESS"] = "strict"
    if offline:
        env["KASSIBER_NO_EGRESS"] = "1"
    result = subprocess.run([str(Path(executable).resolve())], input=json.dumps(cases).encode(),
        env=env, capture_output=True, check=True, timeout=30)
    assert result.stderr == b""
    actual = json.loads(result.stdout)
    expected = [REFERENCE.evaluate(case) for case in cases]
    assert len(actual) == len(expected) == len(cases)
    for case, a, b in zip(cases, actual, expected):
        assert json.dumps(a, sort_keys=True, separators=(",", ":")).encode() == json.dumps(b, sort_keys=True, separators=(",", ":")).encode(), case["name"]
    assert tables() == before
    conn.close()
