import json
import unittest
from unittest.mock import patch

from kassiber.core import transaction_graph as graph
from tests import test_transaction_graph as fixtures


class LocalReferenceGraphTests(unittest.TestCase):
    def setUp(self):
        self.book = fixtures.TransactionGraphTest()
        self.book.setUp()
        self.addCleanup(self.book.tearDown)
        self.conn = self.book.conn
        self.txid = "a" * 64
        self.raw = {
            "txid": self.txid,
            "vin": [{"txid": "b" * 64, "vout": 0, "prevout": {
                "value": 100000, "scriptpubkey": fixtures.SCRIPT_A}}],
            "vout": [{"value": 99000, "scriptpubkey": fixtures.SCRIPT_B}],
        }
        self.conn.execute("UPDATE wallets SET config_json = ?", (json.dumps({"chain": "bitcoin", "network": "main"}),))
        self.book._tx("exchange", "wallet-a", "outbound", 99000000, self.txid, {})

    def observe(self, raw=None, *, network="main", status=None):
        self.conn.execute(
            "INSERT INTO chain_analysis_observations(profile_id,chain,network,txid,payload_json,status_json,source_name,observed_at) VALUES(?,?,?,?,?,?,?,?)",
            ("profile-1", "bitcoin", network, self.txid, json.dumps(raw or self.raw), json.dumps(status or {}), "authorized-local-node", fixtures.NOW),
        )

    def snapshot(self, **kwargs):
        with patch.object(graph, "_fetch_reference_graph_from_backend", side_effect=AssertionError("unexpected egress")):
            return self.book._graph("exchange", **kwargs)

    def test_reuses_same_profile_same_network_wallet_graph(self):
        self.book._tx("observed", "wallet-b", "inbound", 99000000, self.txid, self.raw)
        result = self.snapshot()
        self.assertEqual(result["supportLevel"], "full")
        self.assertEqual(result["transaction"]["id"], "exchange")
        self.assertEqual(result["outputs"][0]["valueSats"], 99000)

    def test_reuses_previously_authorized_local_acquisition(self):
        self.observe()
        before = self.conn.total_changes
        result = self.snapshot()
        self.assertEqual(result["supportLevel"], "full")
        self.assertEqual(self.conn.total_changes, before)
        self.assertEqual(self.conn.execute("SELECT count(*) FROM transaction_graph_cache").fetchone()[0], 0)

    def test_reference_payload_does_not_leak_private_source_fields(self):
        self.observe({**self.raw, "descriptor": "private-source-descriptor", "token": "private-source-token"})
        encoded = json.dumps(self.snapshot())
        self.assertNotIn("private-source-descriptor", encoded)
        self.assertNotIn("private-source-token", encoded)

    def test_other_profile_reference_is_not_reused(self):
        self.conn.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,tax_long_term_days,gains_algorithm,created_at) VALUES('other','ws-1','Other','EUR','generic',365,'FIFO',?)", (fixtures.NOW,))
        self.observe()
        self.conn.execute("UPDATE chain_analysis_observations SET profile_id='other'")
        self.assertEqual(self.snapshot()["supportLevel"], "graphless")

    def test_wrong_network_wallet_reference_is_not_reused(self):
        self.conn.execute("UPDATE wallets SET config_json=? WHERE id='wallet-b'", (json.dumps({"chain": "bitcoin", "network": "regtest"}),))
        self.book._tx("observed", "wallet-b", "inbound", 99000000, self.txid, self.raw)
        self.assertEqual(self.snapshot()["supportLevel"], "graphless")

    def test_legacy_source_is_not_relabelled_to_focused_regtest_network(self):
        self.conn.execute("UPDATE wallets SET config_json='{}' WHERE id='wallet-b'")
        self.conn.execute("UPDATE wallets SET config_json=? WHERE id='wallet-a'", (json.dumps({"chain": "bitcoin", "network": "regtest"}),))
        self.book._tx("observed", "wallet-b", "inbound", 99000000, self.txid, self.raw)
        self.assertEqual(self.snapshot()["supportLevel"], "graphless")

    def test_partial_reference_does_not_displace_known_output_value(self):
        current = {**self.raw, "vin": [{"txid": "b" * 64, "vout": 0}]}
        self.conn.execute("UPDATE transactions SET raw_json=? WHERE id='exchange'", (json.dumps(current),))
        self.observe({**self.raw, "vout": [{"scriptpubkey": fixtures.SCRIPT_B, "asset": "BTC"}]})
        result = self.snapshot()
        self.assertEqual(result["supportLevel"], "partial")
        self.assertEqual(result["outputs"][0]["valueSats"], 99000)

    def test_capacity_limit_does_not_choose_an_unchecked_source(self):
        for index in range(3):
            self.book._tx(f"observed-{index}", "wallet-b", "inbound", 99000000, self.txid, self.raw)
        with patch("kassiber.core.transaction_references.MAX_LOCAL_REFERENCE_ROWS", 2):
            result = self.snapshot()
        self.assertEqual(result["supportLevel"], "graphless")
        self.assertIn("local_reference_conflict", {item["code"] for item in result["warnings"]})

    def test_wrong_network_and_stale_observations_are_not_reused(self):
        self.observe(network="regtest")
        self.assertEqual(self.snapshot()["supportLevel"], "graphless")
        self.observe(status={"removed": True})
        self.assertEqual(self.snapshot()["supportLevel"], "graphless")

    def test_conflicting_local_graphs_are_not_selected_arbitrarily(self):
        self.book._tx("observed", "wallet-b", "inbound", 99000000, self.txid, self.raw)
        changed = {**self.raw, "vout": [{"value": 98000, "scriptpubkey": fixtures.SCRIPT_B}]}
        self.observe(changed)
        self.assertEqual(self.snapshot()["supportLevel"], "graphless")

    def test_raw_network_scope_cannot_read_mainnet_cache(self):
        self.conn.execute("UPDATE wallets SET config_json='{}' WHERE id='wallet-a'")
        self.conn.execute("UPDATE transactions SET raw_json=? WHERE id='exchange'", (json.dumps({"chain": "bitcoin", "network": "regtest"}),))
        graph._store_graph_lookup_cache(self.conn, "bitcoin", "main", self.txid, self.raw)
        result = self.snapshot()
        self.assertEqual(result["transaction"]["network"], "regtest")
        self.assertEqual(result["supportLevel"], "graphless")

    def test_contradictory_network_prevents_cache_and_approved_lookup(self):
        self.conn.execute("UPDATE transactions SET raw_json=? WHERE id='exchange'", (json.dumps({"network": "regtest"}),))
        graph._store_graph_lookup_cache(self.conn, "bitcoin", "main", self.txid, self.raw)
        result = self.snapshot(allow_public_lookup=True)
        self.assertEqual(result["supportLevel"], "graphless")
        self.assertEqual(result["transaction"]["network"], "unknown")

    def test_network_alias_reuses_only_correct_reference(self):
        self.conn.execute("UPDATE wallets SET config_json='{}' WHERE id='wallet-a'")
        self.conn.execute("UPDATE transactions SET raw_json=? WHERE id='exchange'", (json.dumps({"bitcoin_network": "regtest"}),))
        self.observe(network="regtest")
        result = self.snapshot()
        self.assertEqual(result["transaction"]["network"], "regtest")
        self.assertEqual(result["supportLevel"], "full")

    def test_liquid_regtest_reference_uses_canonical_domain_and_keeps_foreign_values_off_axis(self):
        self.conn.execute("UPDATE wallets SET config_json=? WHERE id='wallet-a'", (json.dumps({"chain": "liquid", "network": "elementsregtest"}),))
        self.conn.execute("UPDATE transactions SET asset='LBTC' WHERE id='exchange'")
        self.observe({**self.raw, "vout": [{**self.raw["vout"][0], "asset": "USDt"}]})
        self.conn.execute("UPDATE chain_analysis_observations SET chain='liquid', network='elementsregtest'")
        result = self.snapshot()
        self.assertEqual(result["transaction"]["network"], "elementsregtest")
        self.assertNotEqual(result["supportLevel"], "graphless")
        self.assertIsNone(result["outputs"][0].get("valueSats"))

    def test_partial_local_reference_does_not_block_approved_lookup(self):
        self.observe({**self.raw, "vin": [{"txid": "b" * 64, "vout": 0}]})
        with patch.object(graph, "_graph_lookup_backends", return_value=[{"name": "approved"}]), patch.object(graph, "_fetch_reference_graph_from_backend", return_value=self.raw) as fetch:
            result = self.book._graph("exchange", allow_public_lookup=True)
        self.assertEqual(result["supportLevel"], "full")
        fetch.assert_called_once()

    def test_partial_reference_does_not_present_exchange_fee_as_miner_fee(self):
        self.conn.execute("UPDATE transactions SET fee=2000000 WHERE id='exchange'")
        self.observe({**self.raw, "vin": [{"txid": "b" * 64, "vout": 0}]})
        result = self.snapshot()
        self.assertEqual(result["supportLevel"], "partial")
        self.assertIsNone(result["fee"])
        self.assertEqual(result["transaction"]["feeMsat"], 2000000)

    def test_partial_own_graph_cannot_use_participant_fee_as_total_miner_fee(self):
        partial = {**self.raw, "vin": [{"txid": "b" * 64, "vout": 0}]}
        self.conn.execute("UPDATE transactions SET raw_json=?, fee=2000000 WHERE id='exchange'", (json.dumps(partial),))
        result = self.snapshot()
        self.assertEqual(result["supportLevel"], "partial")
        self.assertIsNone(result["fee"])

    def test_inconsistent_output_index_cannot_replace_current_graph(self):
        partial = {**self.raw, "vin": [{"txid": "b" * 64, "vout": 0}]}
        self.conn.execute("UPDATE transactions SET raw_json=? WHERE id='exchange'", (json.dumps(partial),))
        self.observe({**self.raw, "vout": [{**self.raw["vout"][0], "n": 7}]})
        self.assertEqual(self.snapshot()["supportLevel"], "partial")

    def test_existing_partial_graph_survives_conflicting_reference(self):
        partial = {**self.raw, "vin": [{"txid": "b" * 64, "vout": 0}]}
        self.conn.execute("UPDATE transactions SET raw_json=? WHERE id='exchange'", (json.dumps(partial),))
        self.observe({**self.raw, "vout": [{"value": 98000, "scriptpubkey": fixtures.SCRIPT_B}]})
        result = self.snapshot()
        self.assertEqual(result["supportLevel"], "partial")
        self.assertEqual(result["outputs"][0]["valueSats"], 99000)
