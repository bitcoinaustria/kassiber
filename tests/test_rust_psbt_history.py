"""Native historical PSBT baseline against unchanged Python, with strict no-egress."""
from contextlib import closing
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys

import pytest

from tests.rust_sync_env import child_env

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("psbt_history_vectors", ROOT / "crates/reports/scripts/gen_psbt_history_vectors.py")
REFERENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE)


def test_psbt_history_corpus_matches_unchanged_python():
    assert json.loads(REFERENCE.DEST.read_text()) == REFERENCE.vectors()


@pytest.mark.parametrize("offline", [False, True])
def test_psbt_native_history_bytes_all_tables_without_consent(tmp_path, offline):
    executable = os.environ.get("KASSIBER_PSBT_PRIVACY_BIN")
    if not executable:
        pytest.skip("requires psbt_privacy_fixture test-harness build")
    cases = REFERENCE.vectors()
    env = child_env(tmp_path)
    env["KASSIBER_TEST_NO_EGRESS"] = "strict"
    if offline:
        env["KASSIBER_NO_EGRESS"] = "1"
    for index, case in enumerate(cases):
        database = tmp_path / f"book-{index}.sqlite"
        with closing(sqlite3.connect(database)) as conn:
            conn.row_factory = sqlite3.Row
            conn.executescript("PRAGMA synchronous=OFF; PRAGMA journal_mode=MEMORY;" + REFERENCE.OVERLAY.SEED.read_text() + REFERENCE.OVERLAY.BOOK.read_text() + case["sql"])
            before = REFERENCE.OVERLAY.tables(conn)
            child = subprocess.run([str(Path(executable).resolve())], input=(json.dumps(dict(case, database=str(database))) + "\n").encode(),
                capture_output=True, env=env, check=True, timeout=30)
            assert child.stdout == (REFERENCE.OVERLAY.canonical(case["expected"]) + "\n").encode(), case["name"]
            assert child.stderr == b""
            assert b"sentinel-private-" not in child.stdout
            for (text,) in conn.execute("SELECT script_pubkey FROM wallet_utxos"):
                if text and len(text) >= 20:
                    assert text.encode() not in child.stdout, case["name"]
            if len(case["psbt"]) > 30:
                assert case["psbt"].encode() not in child.stdout
            assert REFERENCE.OVERLAY.tables(conn) == before, case["name"]
        database.unlink()
