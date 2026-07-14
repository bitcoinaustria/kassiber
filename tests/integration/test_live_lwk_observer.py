from __future__ import annotations

import hashlib
import json
import os
import tempfile
import time
import unittest
import uuid
from decimal import Decimal
from pathlib import Path

from kassiber.db import open_db
from tests.integration import regtest_demo
from tests.integration.env import skip_unless_integration
from tests.integration.test_live_bdk_observer import _wait_for_electrum_height, _wait_for_esplora
from tests.integration.test_live_bitcoin_electrum_parity import _rpc, _run, _sync, _transaction_projection, _utxo_projection, _wait_for_electrum


def _liquid_electrum_url() -> str:
    return f"tcp://127.0.0.1:{os.environ.get('KASSIBER_REGTEST_LIQUID_ELECTRUM_PORT', '18545')}"


def _liquid_esplora_url() -> str:
    return f"http://127.0.0.1:{os.environ.get('KASSIBER_REGTEST_LIQUID_MEMPOOL_PORT', '18546')}/api"


def _ct_descriptor(url: str, user: str, password: str, wallet: str) -> str:
    rows = _rpc(url, user, password, "listdescriptors", [False], wallet=wallet)["descriptors"]
    receive = next(
        str(row["desc"]).split("#", 1)[0]
        for row in rows
        if row.get("active") and not row.get("internal") and str(row.get("desc")).startswith("wpkh(")
    )
    master = str(_rpc(url, user, password, "dumpmasterblindingkey", [], wallet=wallet))
    return f"ct(slip77({master}),elwpkh({receive[len('wpkh('):-1]}))"


def _create_book(root: Path, kind: str, endpoint: str, descriptor: str) -> None:
    _run(root, "init")
    _run(root, "workspaces", "create", "LWK")
    _run(root, "profiles", "create", "Default", "--workspace", "LWK", "--tax-country", "generic")
    _run(
        root, "backends", "create", f"{kind}-liquid", "--kind", kind, "--url", endpoint,
        "--chain", "liquid", "--network", "elementsregtest", "--timeout", "30", "--batch-size", "25",
    )
    path = root.parent / f"{kind}-descriptor.txt"
    path.write_text(descriptor + "\n", encoding="utf-8")
    _run(
        root, "wallets", "create", "--workspace", "LWK", "--profile", "Default",
        "--label", "LWK watch", "--kind", "descriptor", "--backend", f"{kind}-liquid",
        "--chain", "liquid", "--network", "elementsregtest", "--descriptor-file", str(path),
        "--gap-limit", "8",
    )


def _state_hash(root: Path) -> tuple[str, int, int]:
    conn = open_db(root)
    try:
        state = conn.execute(
            "SELECT state_json FROM chain_observer_instances WHERE observer_kind='lwk'"
        ).fetchone()
        if state is None:
            raise AssertionError("LWK state missing")
        values = conn.execute(
            "SELECT key,value FROM chain_observer_values ORDER BY key"
        ).fetchall()
        digest = hashlib.sha256(str(state["state_json"]).encode())
        for row in values:
            digest.update(str(row["key"]).encode())
            digest.update(bytes(row["value"]))
        return digest.hexdigest(), len(values), int(
            conn.execute("SELECT COUNT(*) FROM chain_observer_coverage").fetchone()[0]
        )
    finally:
        conn.close()


def _semantic_transactions(root: Path) -> list[dict]:
    rows = _transaction_projection(root)
    for row in rows:
        if row["confirmed_at"] is None:
            row["occurred_at"] = "<mempool>"
    return rows


@skip_unless_integration
class LiveLwkObserverTest(unittest.TestCase):
    def test_lwk_esplora_electrum_multi_asset_restart_noop_and_reorg(self):
        url = os.environ.get("KASSIBER_REGTEST_ELEMENTS_URL", "http://127.0.0.1:18547")
        user = os.environ.get("KASSIBER_REGTEST_RPC_USER", "kassiber")
        password = os.environ.get("KASSIBER_REGTEST_RPC_PASSWORD", "kassiber")
        run_id = uuid.uuid4().hex[:12]
        faucet, owner = f"lwk-faucet-{run_id}", f"lwk-owner-{run_id}"
        created = [faucet, owner]
        try:
            for wallet in created:
                _rpc(url, user, password, "createwallet", [wallet, False, False, "", False, True, True])
            mine_conf = _rpc(url, user, password, "getnewaddress", ["mine"], wallet=faucet)
            mine = regtest_demo._unconfidential_address(url, user, password, faucet, mine_conf)
            receive = _rpc(url, user, password, "getnewaddress", ["receive"], wallet=owner)
            descriptor = _ct_descriptor(url, user, password, owner)
            _rpc(url, user, password, "generatetoaddress", [101, mine])
            initial = str(_rpc(url, user, password, "sendtoaddress", [receive, 2.0], wallet=faucet))
            _rpc(url, user, password, "generatetoaddress", [1, mine])
            height = int(_rpc(url, user, password, "getblockcount"))
            electrum, esplora = _liquid_electrum_url(), _liquid_esplora_url()
            _wait_for_electrum(electrum, min_height=height, txids=[initial])
            _wait_for_esplora(esplora, initial, confirmed=True)

            with tempfile.TemporaryDirectory(prefix="kassiber-live-lwk-") as tmp:
                roots = {"electrum": Path(tmp) / "electrum", "esplora": Path(tmp) / "esplora"}
                _create_book(roots["electrum"], "electrum", electrum, descriptor)
                _create_book(roots["esplora"], "esplora", esplora, descriptor)
                first_hashes = {}
                for kind, root in roots.items():
                    result = _sync(root, "LWK watch")
                    self.assertEqual(result["observer_route"], "lwk", result)
                    first_hashes[kind], value_count, coverage = _state_hash(root)
                    self.assertGreater(value_count, 0)
                    self.assertEqual(coverage, 2)
                    _sync(root, "LWK watch")
                    self.assertEqual(_state_hash(root)[0], first_hashes[kind])
                self.assertEqual(_semantic_transactions(roots["electrum"]), _semantic_transactions(roots["esplora"]))
                self.assertEqual(_utxo_projection(roots["electrum"]), _utxo_projection(roots["esplora"]))

                external = _rpc(url, user, password, "getnewaddress", ["LWK external"], wallet=faucet)
                spend_tx = str(
                    _rpc(
                        url,
                        user,
                        password,
                        "sendtoaddress",
                        [external, 0.4],
                        wallet=owner,
                    )
                )
                _wait_for_electrum(electrum, min_height=height, txids=[spend_tx])
                _wait_for_esplora(esplora, spend_tx, confirmed=False)
                for root in roots.values():
                    _sync(root, "LWK watch")
                self.assertEqual(
                    _semantic_transactions(roots["electrum"]),
                    _semantic_transactions(roots["esplora"]),
                )
                spend = next(
                    row
                    for row in _transaction_projection(roots["electrum"])
                    if row["external_id"] == spend_tx
                )
                self.assertEqual(spend["direction"], "outbound")
                self.assertIsNone(spend["confirmed_at"])
                self.assertGreater(Decimal(str(spend["fee"])), Decimal(0))
                _rpc(url, user, password, "generatetoaddress", [1, mine])
                height = int(_rpc(url, user, password, "getblockcount"))
                _wait_for_electrum(electrum, min_height=height, txids=[spend_tx])
                _wait_for_esplora(esplora, spend_tx, confirmed=True)
                for root in roots.values():
                    _sync(root, "LWK watch")

                issued = _rpc(url, user, password, "issueasset", [5, 0, False], wallet=faucet)
                asset = str(issued["asset"])
                _rpc(url, user, password, "generatetoaddress", [1, mine])
                height = int(_rpc(url, user, password, "getblockcount"))
                asset_tx = str(_rpc(
                    url, user, password, "sendtoaddress",
                    [receive, 1.25, "", "", False, False, 1, "UNSET", False, asset],
                    wallet=faucet,
                ))
                _wait_for_electrum(electrum, min_height=height, txids=[asset_tx])
                _wait_for_esplora(esplora, asset_tx, confirmed=False)
                for root in roots.values():
                    _sync(root, "LWK watch")
                rows = _semantic_transactions(roots["electrum"])
                self.assertEqual(rows, _semantic_transactions(roots["esplora"]))
                self.assertIn(asset, {row["asset"] for row in rows})

                block = _rpc(url, user, password, "generatetoaddress", [1, mine])[0]
                height = int(_rpc(url, user, password, "getblockcount"))
                _wait_for_electrum(electrum, min_height=height, txids=[asset_tx])
                _wait_for_esplora(esplora, asset_tx, confirmed=True)
                for root in roots.values():
                    _sync(root, "LWK watch")
                self.assertIsNotNone(next(row for row in _semantic_transactions(roots["electrum"]) if row["external_id"] == asset_tx)["confirmed_at"])

                _rpc(url, user, password, "invalidateblock", [block])
                _wait_for_electrum_height(electrum, height - 1)
                _wait_for_esplora(esplora, asset_tx, confirmed=False)
                for root in roots.values():
                    _sync(root, "LWK watch")
                self.assertIsNone(next(row for row in _semantic_transactions(roots["electrum"]) if row["external_id"] == asset_tx)["confirmed_at"])
                _rpc(url, user, password, "reconsiderblock", [block])
        finally:
            for wallet in reversed(created):
                try:
                    _rpc(url, user, password, "unloadwallet", [wallet])
                except Exception:
                    # Teardown must not mask the live observer assertion result.
                    pass


if __name__ == "__main__":
    unittest.main()
