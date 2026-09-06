"""Actual mined spends against Core31, including saved-case change detection."""
from __future__ import annotations

import json
import os
import tempfile
import time
import unittest
import uuid

from kassiber.core.chain_analysis_acquisition import plan_acquisition, apply_acquisition
from kassiber.core.chain_analysis import run_analysis
from kassiber.core.chain_analysis_cases import save_case, compare_case
from kassiber.db import open_db
from tests.integration.env import skip_unless_integration
from tests.integration.test_live_bitcoin_core_regtest import _rpc


@skip_unless_integration
class LiveChainAnalysisTest(unittest.TestCase):
    def test_mined_intermediate_wallet_and_later_spend_are_real_paths(self):
        url = os.environ["KASSIBER_REGTEST_CORE_URL"]
        username = os.environ["KASSIBER_REGTEST_RPC_USER"]
        password = os.environ["KASSIBER_REGTEST_RPC_PASSWORD"]
        rpc = lambda method, params=None, wallet=None: _rpc(url, username, password, method, params, wallet)
        assert rpc("getblockchaininfo")["chain"] == "regtest"
        self.assertIn("txospenderindex", rpc("getindexinfo"))
        names = [f"chain-analysis-{uuid.uuid4().hex[:12]}-{i}" for i in range(3)]
        try:
            for name in names:
                rpc("createwallet", [name])
            miner, middle, destination = names
            mining_address = rpc("getnewaddress", wallet=miner)
            rpc("generatetoaddress", [101, mining_address])
            receive_address = rpc("getnewaddress", wallet=middle)
            funding = rpc("sendtoaddress", [receive_address, 1], miner)
            rpc("generatetoaddress", [1, mining_address])
            final_address = rpc("getnewaddress", wallet=destination)
            spend = rpc("sendtoaddress", [final_address, 0.7], middle)
            rpc("generatetoaddress", [1, mining_address])
            self._wait_index(rpc)
            with tempfile.TemporaryDirectory() as directory:
                conn = open_db(directory)
                try:
                    now = "2026-09-06T12:00:00Z"
                    conn.execute("INSERT INTO workspaces VALUES('ws','Live investigation',?)", (now,))
                    conn.execute("INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','ws','Regtest',?)", (now,))
                    conn.execute("INSERT INTO backends(name,kind,chain,network,url,config_json,created_at,updated_at) VALUES('node','bitcoinrpc','bitcoin','regtest',?,?,?,?)", (url, json.dumps({"username": username, "password": password}), now, now))
                    conn.commit()
                    args = {"backend": "node", "subject": funding, "chain": "bitcoin", "network": "regtest", "direction": "forward", "depth": 3, "max_transactions": 10}
                    plan = plan_acquisition(conn, "p", args)
                    observed = apply_acquisition(conn, "p", {"plan": plan})
                    self.assertTrue(observed["complete"], observed)
                    self.assertEqual(set(observed["transaction_ids"]), {funding, spend})
                    self.assertLessEqual(observed["request_count"], plan["effects"]["max_requests"])
                    query = {"mode": "trace", "subject": funding, "chain": "bitcoin", "network": "regtest", "direction": "forward", "depth": 10}
                    result = run_analysis(conn, "p", query)
                    self.assertEqual({node["txid"] for node in result["nodes"] if node["kind"] == "transaction"}, {funding, spend})
                    self.assertTrue(any(edge["kind"] == "spends" for edge in result["edges"]))
                    # Public observation acquisition creates no watch-only
                    # wallet, imported accounting row or custody decision.
                    self.assertEqual(conn.execute("SELECT COUNT(*) FROM transactions").fetchone()[0], 0)
                    self.assertEqual(conn.execute("SELECT COUNT(*) FROM wallets").fetchone()[0], 0)
                    case = save_case(conn, "p", {"title": "Before destination moves", "query": query, "expected_snapshot_id": result["snapshot_id"]})
                    next_address = rpc("getnewaddress", wallet=miner)
                    onward = rpc("sendtoaddress", [next_address, 0.5], destination)
                    rpc("generatetoaddress", [1, mining_address])
                    self._wait_index(rpc)
                    apply_acquisition(conn, "p", {"plan": plan_acquisition(conn, "p", args)})
                    comparison = compare_case(conn, "p", {"id": case["id"]})
                    self.assertTrue(comparison["changed"])
                    self.assertIn(onward, {node.get("txid") for node in comparison["added_nodes"]})
                    self.assertNotIn("password", json.dumps(run_analysis(conn, "p", query)))
                finally:
                    conn.close()
        finally:
            for name in reversed(names):
                try:
                    rpc("unloadwallet", [name])
                except Exception:
                    pass

    def _wait_index(self, rpc):
        for _ in range(100):
            rows = rpc("getindexinfo")
            tip = rpc("getblockcount")
            if all(rows.get(name, {}).get("synced") and rows[name].get("best_block_height", -1) >= tip for name in ("txindex", "txospenderindex")):
                return
            time.sleep(0.1)
        self.fail("Core transaction/spender indexes did not reach the mined tip")
