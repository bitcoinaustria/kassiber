"""An owned Liquid rotation journals as a move in both runtimes, byte for byte.

The book is the one `test_liquid_rotation_authority` seeds: two owned Liquid
wallets, one same-txid rotation, observer provenance on every row. The
rewritten variant carries the top-level identity the old open-time backfill
added; the native runtime restores it exactly as Python does.
"""
import json
import sqlite3

import pytest

from kassiber.db import open_db
from tests.test_liquid_rotation_authority import POLICY_ASSET, LiquidRotationAuthorityTest
from tests.test_rust_connections import RUST_BIN, btcpay_book, served
from tests.test_rust_wallet_sync_sources import compare_cli

pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires the kassiber test-harness build")


def rotation_book(tmp_path, *, rewritten):
    root, env = btcpay_book(tmp_path)
    conn = open_db(root)
    try:
        LiquidRotationAuthorityTest()._seed(conn)
        if rewritten:
            for row in conn.execute("SELECT id, raw_json FROM transactions WHERE profile_id = 'profile'").fetchall():
                raw = {**json.loads(row["raw_json"]), "asset": "LBTC", "asset_id": POLICY_ASSET}
                conn.execute("UPDATE transactions SET raw_json = ? WHERE id = ?",
                             (json.dumps(raw, sort_keys=True), row["id"]))
        conn.commit()
    finally:
        conn.close()
    return root, env


@pytest.mark.parametrize("rewritten", [False, True], ids=["committed", "rewritten"])
def test_owned_liquid_rotation_journals_as_a_move_like_python(tmp_path, rewritten):
    root, env = rotation_book(tmp_path, rewritten=rewritten)
    result, _ = compare_cli(root, env, ["journals", "process", "--workspace", "Books", "--profile", "Book"])
    assert result["data"]["quarantined"] == 0, result
    # The compared book now holds the native runtime's result.
    conn = sqlite3.connect(root / "kassiber.sqlite3")
    try:
        entries = set(conn.execute("SELECT transaction_id, entry_type FROM journal_entries"))
        raw = json.loads(conn.execute("SELECT raw_json FROM transactions WHERE id = 'rotation-out'").fetchone()[0])
    finally:
        conn.close()
    assert ("rotation-out", "transfer_out") in entries and ("rotation-in", "transfer_in") in entries, entries
    assert ("rotation-out", "disposal") not in entries and ("rotation-in", "acquisition") not in entries
    assert "asset_id" not in raw
    assert {entry["served_by"] for entry in served(env)} == {"rust"}
