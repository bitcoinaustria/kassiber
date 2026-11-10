from contextlib import closing
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys

import pytest

from tests.integration.env import no_egress_guard
from tests.rust_sync_env import child_env

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("legacy_privacy_vectors", ROOT / "crates/reports/scripts/gen_legacy_privacy_vectors.py")
REFERENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE)


def test_legacy_privacy_vectors_cover_every_scoring_rule_and_match_python():
    with no_egress_guard(enabled=True,allow_loopback=False):
        cases = REFERENCE.vectors()
    assert len(cases) >= 560
    assert json.loads(REFERENCE.DEST.read_text()) == cases
    assert json.loads(REFERENCE.METADATA.read_text()) == REFERENCE.metadata()
    codes = {finding["code"] for case in cases for probe in case["probes"] for finding in probe["findings"]}
    assert codes >= {"coinjoin_pattern","payjoin_boundary","common_input_ownership","round_output_amount","unnecessary_input_heuristic","round_fee_rate","rbf_signal","script_type_mix","change_type_fingerprint","change_position_fingerprint","op_return_metadata","taproot_usage","transaction_not_analysable","transaction_coverage_gap","unusual_transaction_version","ineffective_absolute_locktime","relative_lock_constraints","consistent_low_r_encoding","nondefault_sighash"}
    wallet_codes = {f["code"] for c in cases for w in c["expected"].get("ok",{}).get("wallets",[]) for f in w["top_findings"]}
    assert wallet_codes >= {"address_reuse","dust_utxo_exposure","large_utxo_set","legacy_address_type","taproot_usage","coin_anonymity_evidence"}


def test_legacy_privacy_mutations_have_named_killers_and_apply(tmp_path):
    catalog = ROOT / "crates/reports/scripts/legacy_privacy_mutations.json"
    result = subprocess.run([sys.executable,str(ROOT / "crates/reports/scripts/legacy_privacy_mutation_check.py"),"--check"],env=child_env(tmp_path),capture_output=True,check=True,timeout=30)
    entries = json.loads(catalog.read_text())
    assert result.stdout == f"{len(entries)} entries apply\n".encode()
    source = (ROOT / "crates/reports/tests/legacy_privacy.rs").read_text()
    for entry in entries:
        assert all(f"fn {name}()" in source for name in entry["expect_failing"])


@pytest.mark.parametrize("offline",[False,True])
def test_legacy_privacy_native_bytes_and_all_tables_without_consent(tmp_path,offline):
    executable = os.environ.get("KASSIBER_LEGACY_PRIVACY_BIN")
    if not executable:
        pytest.skip("requires legacy_privacy_fixture test-harness build")
    cases = json.loads(REFERENCE.DEST.read_text())
    env = child_env(tmp_path)
    env["KASSIBER_TEST_NO_EGRESS"] = "strict"
    if offline:
        env["KASSIBER_NO_EGRESS"] = "1"
    for case in cases:
        database = tmp_path / "book.sqlite"
        with closing(sqlite3.connect(database)) as conn:
            conn.row_factory = sqlite3.Row
            conn.executescript("PRAGMA foreign_keys=ON; PRAGMA synchronous=OFF; PRAGMA journal_mode=MEMORY;" + REFERENCE.SEED.read_text() + REFERENCE.BOOK.read_text() + case["sql"])
            before = REFERENCE.tables(conn)
            request = dict(database=str(database),args_json=case["args_json"])
            child = subprocess.run([str(Path(executable).resolve())],input=(json.dumps(request)+"\n").encode(),capture_output=True,env=env,check=True,timeout=30)
            expected = dict(result=case["expected"],probes=case["probes"])
            assert child.stdout == (REFERENCE.canonical(expected)+"\n").encode(),case["name"]
            assert child.stderr == b"",case["name"]
            assert REFERENCE.tables(conn) == before,case["name"]
        database.unlink()


def test_legacy_privacy_reference_order_is_independent_of_hashseed(tmp_path):
    script = """
import hashlib,json,sys
from contextlib import closing
from tests.test_rust_legacy_privacy import REFERENCE
from tests.integration.env import no_egress_guard
results=[]
with no_egress_guard(enabled=True,allow_loopback=False):
    with closing(REFERENCE.connection()) as conn:
        for case in json.loads(REFERENCE.DEST.read_text()):
            conn.executescript('SAVEPOINT seed;' + case['sql'])
            result=REFERENCE.evaluate(conn,json.loads(case['args_json']))
            assert result == case['expected'],case['name']
            results.append(dict(result=result,probes=REFERENCE.probes(conn)))
            conn.execute('ROLLBACK TO seed')
            conn.execute('RELEASE seed')
print(hashlib.sha256(REFERENCE.canonical(results).encode()).hexdigest())
"""
    outputs = {}
    for seed in ("0","1","2","7","21","42"):
        env = child_env(tmp_path)
        env["PYTHONHASHSEED"] = seed
        child = subprocess.run([sys.executable,"-c",script],env=env,capture_output=True,check=True,timeout=180)
        assert child.stderr == b""
        outputs[seed] = child.stdout.decode().strip()
    assert len(set(outputs.values())) == 1,outputs
    print(json.dumps({"python_hashseed_sha256":outputs},sort_keys=True))
