"""Independent topology and boundary checks for the local evidence query API."""
import json
import sqlite3
import unittest

from kassiber.core.chain_analysis import analyze_snapshot, build_index, normalize_query, query_index, run_analysis, run_entropy
from kassiber.core.chain_analysis.index import output_node_id, tx_node_id
from kassiber.core.chain_analysis.query import resolve_subject
from kassiber.errors import AppError


NOW = "2026-09-01T12:00:00Z"


def txid(number):
    return f"{number:064x}"


def connection():
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    conn.executescript("""
        CREATE TABLE profiles(id TEXT, last_processed_at TEXT, journal_input_version INTEGER, last_processed_input_version INTEGER, last_processed_tx_count INTEGER);
        INSERT INTO profiles VALUES('p',NULL,0,0,0);
        CREATE TABLE wallets(id TEXT, profile_id TEXT, kind TEXT, config_json TEXT);
        INSERT INTO wallets VALUES('w','p','descriptor','{}');
        CREATE TABLE transactions(id TEXT PRIMARY KEY, profile_id TEXT, wallet_id TEXT, external_id TEXT, amount INTEGER, asset TEXT, raw_json TEXT, occurred_at TEXT, excluded INTEGER, privacy_boundary TEXT);
        CREATE TABLE wallet_utxos(id TEXT, profile_id TEXT, wallet_id TEXT, chain TEXT, network TEXT, txid TEXT, vout INTEGER, amount INTEGER, asset TEXT, address TEXT, script_pubkey TEXT, branch_label TEXT, spent_by TEXT);
        CREATE TABLE transaction_graph_cache(schema_version INTEGER,chain TEXT,network TEXT,txid TEXT,payload_json TEXT,created_at TEXT,updated_at TEXT);
        CREATE TABLE journal_custody_decisions(profile_id TEXT,decision_id TEXT,source_transaction_id TEXT,target_transaction_id TEXT,source_start_msat INTEGER,source_end_msat INTEGER,source_asset TEXT,target_asset TEXT,state TEXT,basis_state TEXT,component_id TEXT);
        CREATE TABLE journal_custody_economic_relations(profile_id TEXT,relation_id TEXT,relation_kind TEXT,source_transaction_id TEXT,target_transaction_id TEXT,source_asset TEXT,target_asset TEXT,source_amount_msat INTEGER,target_amount_msat INTEGER,basis_state TEXT,component_id TEXT);
        CREATE TABLE chain_analysis_observations(profile_id TEXT,chain TEXT,network TEXT,txid TEXT,payload_json TEXT,status_json TEXT,source_name TEXT,observed_at TEXT);
        CREATE TABLE chain_analysis_labels(id TEXT,profile_id TEXT,chain TEXT,network TEXT,subject TEXT,label TEXT,category TEXT,source TEXT,confidence TEXT,cluster_defining INTEGER,revision INTEGER,deleted INTEGER,created_at TEXT,updated_at TEXT);
    """)
    return conn


def add_tx(conn, number, parents=(), *, outputs=(1000,), chain="bitcoin", network="main", record_id=None, wallet="w", boundary=None, extra=None):
    def prevout(parent, number):
        row = conn.execute("SELECT raw_json FROM transactions WHERE external_id=? LIMIT 1", (txid(parent),)).fetchone()
        return json.loads(row[0])["vout"][number] if row else {"value": 1000}
    raw = {"txid": txid(number), "chain": chain, "network": network, "vin": [{"txid": txid(parent), "vout": vout, "prevout": prevout(parent, vout), "sequence": 0xFFFFFFFF} for parent, vout in parents] or [{"coinbase": "0101"}], "vout": [{"value": value, "scriptpubkey": "0014" + f"{number:040x}"} if value is not None else {"valuecommitment": "08" + "11" * 32, "scriptpubkey": "0014" + f"{number:040x}"} for value in outputs], "vsize": 100, **(extra or {})}
    conn.execute("INSERT INTO transactions VALUES(?,?,?,?,?,?,?,?,?,?)", (record_id or str(number), "p", wallet, txid(number), 1000000, "LBTC" if chain == "liquid" else "BTC", json.dumps(raw), NOW, 0, boundary))
    return raw


def trace(index, subject, **kwargs):
    return query_index(index, {"mode": "trace", "subject": subject, "direction": "forward", "depth": 10, **kwargs})


class ChainAnalysisTests(unittest.TestCase):
    def setUp(self):
        self.conn = connection()

    def tearDown(self):
        self.conn.close()

    def test_path_does_not_accept_stale_or_conflicting_target(self):
        add_tx(self.conn, 1)
        add_tx(self.conn, 2, [(1, 0)], extra={"confirmations": -1})
        query = {"mode": "path", "subject": txid(1), "target": txid(2), "direction": "forward", "depth": 10}
        stale = query_index(build_index(self.conn, "p"), query)
        self.assertFalse(stale["paths"])
        self.assertIn("stale", {row["reason"] for row in stale["frontier"]})
        self.assertFalse(stale["coverage"]["complete"])
        self.conn.execute("DELETE FROM transactions WHERE id='2'")
        add_tx(self.conn, 2, [(1, 0)])
        add_tx(self.conn, 2, [(1, 0)], outputs=(500, 400), record_id="conflicting-2")
        conflicting = query_index(build_index(self.conn, "p"), query)
        self.assertFalse(conflicting["paths"])
        self.assertIn("conflicting", {row["reason"] for row in conflicting["frontier"]})
        self.assertFalse(conflicting["coverage"]["complete"])

    def test_topology_is_complete_vin_vout_not_only_wallet_outputs(self):
        add_tx(self.conn, 1, outputs=(600, 400))
        add_tx(self.conn, 2, [(1, 0)], outputs=(500,))
        add_tx(self.conn, 3, [(1, 1)], outputs=(300,))
        index = build_index(self.conn, "p")
        result = trace(index, txid(1))
        # Independent transaction-level oracle: root forks into exactly 2 and3.
        observed = {node["txid"] for node in result["nodes"] if node["kind"] == "transaction"}
        self.assertEqual(observed, {txid(1), txid(2), txid(3)})
        self.assertEqual(sum(edge["kind"] == "creates" for edge in result["edges"]), 4)
        self.assertEqual(sum(edge["kind"] == "spends" for edge in result["edges"]), 2)
        self.assertEqual(index.transaction_facts[tx_node_id("bitcoin", "main", txid(1))]["outputs"][1]["amount_msat"], "400000")
        self.assertIn("not proven unspent", result["coverage"]["unobserved_successors"])

    def test_backward_path_and_longer_alternative(self):
        # 1->2->5 and 1->3->4->5, with disjoint outputs so no double spend.
        add_tx(self.conn, 1, outputs=(1000, 1000))
        add_tx(self.conn, 2, [(1, 0)])
        add_tx(self.conn, 3, [(1, 1)])
        add_tx(self.conn, 4, [(3, 0)])
        add_tx(self.conn, 5, [(2, 0), (4, 0)])
        index = build_index(self.conn, "p")
        result = query_index(index, {"mode": "path", "subject": txid(1), "target": txid(5), "direction": "forward", "depth": 12})
        paths = [[index.nodes[item]["txid"] for item in path["node_ids"] if index.nodes[item]["kind"] == "transaction"] for path in result["paths"]]
        self.assertEqual(paths, [[txid(1), txid(2), txid(5)], [txid(1), txid(3), txid(4), txid(5)]])
        backwards = trace(index, txid(5), direction="backward")
        self.assertEqual({node["txid"] for node in backwards["nodes"] if node["kind"] == "transaction"}, {txid(i) for i in range(1, 6)})

    def test_domains_never_share_outpoints_or_wallet_ownership(self):
        add_tx(self.conn, 1)
        add_tx(self.conn, 2, [(1, 0)], network="regtest")
        self.conn.execute("INSERT INTO wallet_utxos VALUES('u','p','w','bitcoin','main',?,0,1000000,'BTC','address',NULL,'change',NULL)", (txid(1),))
        index = build_index(self.conn, "p")
        result = trace(index, tx_node_id("bitcoin", "main", txid(1)))
        self.assertFalse(any(node.get("txid") == txid(2) for node in result["nodes"]))
        self.assertEqual(index.nodes[output_node_id("bitcoin", "regtest", txid(1), 0)]["wallet_ids"], ())
        with self.assertRaises(AppError) as caught:
            resolve_subject(index, txid(1), {})
        self.assertEqual(caught.exception.code, "subject_ambiguous")

    def test_duplicate_observations_merge_and_preserve_collaboration(self):
        add_tx(self.conn, 2, [(1, 0)], boundary="payjoin")
        add_tx(self.conn, 2, [(1, 0)], record_id="duplicate", wallet="other")
        index = build_index(self.conn, "p")
        node_id = tx_node_id("bitcoin", "main", txid(2))
        self.assertEqual(index.nodes[node_id]["wallet_ids"], ("other", "w"))
        self.assertEqual(len(index.incoming[node_id]), 1)
        self.assertEqual(index.transaction_facts[node_id]["collaboration"]["kind"], "payjoin")
        self.assertEqual(resolve_subject(index, "record:duplicate", {}), (node_id,))

    def test_competing_spends_stop_and_retraction_removes_conflict(self):
        add_tx(self.conn, 1)
        add_tx(self.conn, 2, [(1, 0)])
        add_tx(self.conn, 3, [(1, 0)])
        index = build_index(self.conn, "p")
        result = trace(index, txid(1))
        self.assertTrue(any(item["reason"] == "conflicting" for item in result["frontier"]))
        self.assertFalse(any(node.get("txid") == txid(2) for node in result["nodes"]))
        self.conn.execute("DELETE FROM transactions WHERE id='3'")
        renewed = build_index(self.conn, "p")
        self.assertNotEqual(renewed.snapshot_id, index.snapshot_id)
        self.assertTrue(any(node.get("txid") == txid(2) for node in trace(renewed, txid(1))["nodes"]))

    def test_reorg_flag_is_not_current_reachability(self):
        add_tx(self.conn, 1, extra={"confirmations": -1})
        add_tx(self.conn, 2, [(1, 0)])
        result = trace(build_index(self.conn, "p"), txid(1))
        self.assertEqual(len(result["nodes"]), 1)
        self.assertTrue(result["coverage"]["stale"])

    def test_retained_retracted_spender_does_not_block_live_replacement(self):
        for source, status in (("stored", {"confirmations": -1}), ("stored", {"removed": True}),
                               ("acquired", {"conflicted": True}), ("acquired", {"removed": True})):
            with self.subTest(source=source, status=status):
                conn = connection()
                try:
                    add_tx(conn, 1)
                    old = add_tx(conn, 2, [(1, 0)], outputs=(900,), extra=status if source == "stored" else None)
                    if source == "acquired":
                        conn.execute("DELETE FROM transactions WHERE id='2'")
                        conn.execute("INSERT INTO chain_analysis_observations VALUES('p','bitcoin','main',?,?,?,'node',?)",
                                     (txid(2), json.dumps(old), json.dumps(status), NOW))
                    add_tx(conn, 3, [(1, 0)], outputs=(900,), extra={"confirmations": 6})
                    index = build_index(conn, "p")
                    result = trace(index, txid(1))
                    self.assertEqual({node["txid"] for node in result["nodes"] if node["kind"] == "transaction"}, {txid(1), txid(3)})
                    self.assertFalse(any(row["code"] == "competing_spends" for row in index.findings))
                    old_id = tx_node_id("bitcoin", "main", txid(2))
                    historical = [edge for edge in index.edges.values() if edge["kind"] == "spends" and edge["target"] == old_id]
                    self.assertEqual(len(historical), 1)
                    self.assertEqual(historical[0]["status"], "stale")
                    self.assertEqual(run_entropy(conn, "p", {"subject": txid(3)})["status"], "exact")
                    self.assertEqual(run_entropy(conn, "p", {"subject": txid(2)})["reason"], "incomplete_transaction")
                finally:
                    conn.close()

    def test_live_competing_spends_withhold_current_entropy(self):
        add_tx(self.conn, 1)
        add_tx(self.conn, 2, [(1, 0)], outputs=(900,))
        add_tx(self.conn, 3, [(1, 0)], outputs=(800,))
        index = build_index(self.conn, "p")
        self.assertTrue(any(row["code"] == "competing_spends" for row in index.findings))
        for number in (2, 3):
            with self.subTest(number=number):
                result = run_entropy(self.conn, "p", {"subject": txid(number)})
                self.assertEqual(result["reason"], "incomplete_transaction")
                self.assertIsNone(result["interpretation_count"])
                self.assertEqual(result["deterministic_links"], [])

    def test_confidential_values_are_unknown_and_msat_is_lossless(self):
        add_tx(self.conn, 1, chain="liquid", outputs=(None,))
        add_tx(self.conn, 2, outputs=(9007199254741,))
        index = build_index(self.conn, "p")
        self.assertIsNone(index.nodes[output_node_id("liquid", "liquidv1", txid(1), 0)]["amount_msat"])
        self.assertEqual({node["chain"] for node in trace(index, txid(1), chain="liquid", network="main")["nodes"]}, {"liquid"})
        self.assertEqual(index.nodes[output_node_id("bitcoin", "main", txid(2), 0)]["amount_msat"], "9007199254741000")

    def test_crossrail_all_canonical_records_and_stale_relation_gate(self):
        add_tx(self.conn, 1)
        add_tx(self.conn, 2, chain="liquid")
        add_tx(self.conn, 3, chain="lightning")
        self.conn.execute("INSERT INTO journal_custody_decisions VALUES('p','native','1','2',0,900000,'BTC','LBTC','internal_verified','eligible',NULL)")
        self.conn.execute("INSERT INTO journal_custody_decisions VALUES('p','review','2','3',0,800000,'LBTC','BTC','internal_reviewed','eligible','component')")
        self.conn.execute("UPDATE profiles SET last_processed_at=?,last_processed_tx_count=3", (NOW,))
        index = build_index(self.conn, "p")
        result = trace(index, txid(1))
        custody = [edge for edge in result["edges"] if edge["kind"] == "custody"]
        self.assertEqual({edge["evidence_level"] for edge in custody}, {"native_verified", "reviewed"})
        self.assertTrue(any(node["chain"] == "lightning" for node in result["nodes"]))
        physical = trace(index, txid(1), include_relations=False)
        self.assertEqual({node["chain"] for node in physical["nodes"]}, {"bitcoin"})
        self.conn.execute("UPDATE profiles SET journal_input_version=1")
        stale = trace(build_index(self.conn, "p"), txid(1))
        self.assertFalse(any(node["chain"] == "liquid" for node in stale["nodes"]))
        self.assertTrue(any(item["reason"] == "stale_relation" for item in stale["frontier"]))

    def test_unrelated_global_cache_excluded_from_overview_but_resolvable(self):
        add_tx(self.conn, 1)
        raw = {"txid": txid(9), "vin": [{"coinbase": "01"}], "vout": [{"value": 500}]}
        self.conn.execute("INSERT INTO transaction_graph_cache VALUES(1,'bitcoin','main',?,?,?,?)", (txid(9), json.dumps(raw), NOW, NOW))
        index = build_index(self.conn, "p")
        overview = query_index(index, {})
        self.assertFalse(any(node.get("txid") == txid(9) for node in overview["nodes"]))
        cached = trace(index, txid(9))
        self.assertTrue(all(not node["wallet_ids"] for node in cached["nodes"]))
        self.assertEqual(cached["nodes"][0]["evidence"][0]["source"], "reference_cache")

    def test_cache_domain_payload_mismatch_is_rejected(self):
        raw = {"chain": "liquid", "network": "main", "vin": [], "vout": []}
        self.conn.execute("INSERT INTO transaction_graph_cache VALUES(1,'bitcoin','main',?,?,?,?)", (txid(9), json.dumps(raw), NOW, NOW))
        index = build_index(self.conn, "p")
        self.assertEqual(index.coverage["cache_rejected"], 1)
        self.assertFalse(index.nodes)

    def test_partial_input_graph_is_preserved_without_inventing_outputs(self):
        add_tx(self.conn, 2, [(1, 0)])
        self.conn.execute("UPDATE transactions SET raw_json=? WHERE id='2'", (json.dumps({"vin": [{"txid": txid(1), "vout": 0}]}),))
        index = build_index(self.conn, "p")
        result = trace(index, txid(2), direction="backward")
        self.assertTrue(any(node.get("txid") == txid(1) for node in result["nodes"]))
        self.assertFalse(index.transaction_facts[tx_node_id("bitcoin", "main", txid(2))]["complete"])
        self.assertTrue(any(item["reason"] == "incomplete_transaction" for item in result["frontier"]))

    def test_complete_conflicting_shapes_stop_instead_of_union(self):
        add_tx(self.conn, 2, [(1, 0)])
        add_tx(self.conn, 2, [(3, 0)], record_id="conflict")
        index = build_index(self.conn, "p")
        result = trace(index, txid(2))
        self.assertEqual(len(result["nodes"]), 1)
        self.assertTrue(any(item["code"] == "conflicting_transaction_shape" for item in result["findings"]))

    def test_budget_and_filters_are_explicit_and_zero_io_repeated_queries(self):
        for i in range(1, 1001):
            add_tx(self.conn, i, [(i - 1, 0)] if i > 1 else ())
        index = build_index(self.conn, "p")
        sql = []
        self.conn.set_trace_callback(sql.append)
        for subject in (txid(1), txid(500), txid(900)):
            result = trace(index, subject, depth=50, node_limit=25, edge_limit=50)
            self.assertLessEqual(result["summary"]["node_count"], 25)
            self.assertLessEqual(result["summary"]["inspected_edge_count"], 50)
            self.assertTrue(result["coverage"]["budget_exhausted"])
        self.assertEqual(sql, [])
        filtered = trace(index, txid(1), min_amount_msat="2000000")
        self.assertEqual(filtered["coverage"]["pruning"], [{"filter": "amount", "count": 1}])
        self.assertFalse(filtered["coverage"]["complete"])

    def test_minimal_schema_reports_missing_sources_and_never_fabricates(self):
        empty = sqlite3.connect(":memory:")
        try:
            result = query_index(build_index(empty, "missing"), {})
            self.assertEqual(result["nodes"], [])
            self.assertIn("transactions", result["coverage"]["missing_tables"])
            self.assertFalse(result["coverage"]["complete"])
        finally:
            empty.close()

    def test_index_immutable_and_snapshot_changes_on_label_acquisition_metadata(self):
        raw = add_tx(self.conn, 1)
        index = build_index(self.conn, "p")
        with self.assertRaises(TypeError):
            index.nodes[tx_node_id("bitcoin", "main", txid(1))]["status"] = "changed"
        self.conn.execute("INSERT INTO chain_analysis_labels VALUES('label','p','bitcoin','main',?,'Source','exchange','manual','reviewed',0,1,0,?,?)", (txid(1), NOW, NOW))
        labeled = build_index(self.conn, "p")
        self.assertNotEqual(index.snapshot_id, labeled.snapshot_id)
        self.assertEqual(labeled.labels[0]["node_ids"], (tx_node_id("bitcoin", "main", txid(1)),))
        self.conn.execute("INSERT INTO chain_analysis_observations VALUES('p','bitcoin','main',?,?,?,'local-node',?)", (txid(1), json.dumps(raw), '{"confirmed":true,"block_height":10}', NOW))
        acquired = build_index(self.conn, "p")
        self.assertNotEqual(acquired.snapshot_id, labeled.snapshot_id)
        self.conn.execute("UPDATE chain_analysis_observations SET status_json=?", ('{"confirmed":true,"block_height":10,"block_hash":"replacement"}',))
        self.assertNotEqual(acquired.snapshot_id, build_index(self.conn, "p").snapshot_id)

    def test_argument_contract_rejects_egress_and_precision_loss(self):
        for args in ({"egress": True}, {"depth": True}, {"node_limit": 2001}, {"min_amount_msat": 1}, {"start": "yesterday"}, {"mode": "path", "subject": "one"}):
            with self.subTest(args=args), self.assertRaises(AppError):
                normalize_query(args)


if __name__ == "__main__":
    unittest.main()


class ObserverAndInterpretationRegressionTests(unittest.TestCase):
    def setUp(self):
        self.conn = connection()

    def tearDown(self):
        self.conn.close()

    def test_public_view_removes_private_records_wallet_grouping_labels_and_custody(self):
        from kassiber.core.chain_analysis import run_analysis
        add_tx(self.conn, 1)
        add_tx(self.conn, 2, chain="lightning")
        self.conn.execute("INSERT INTO wallet_utxos VALUES('private-utxo','p','w','bitcoin','main',?,0,1000000,'BTC','private-address',NULL,'change',NULL)", (txid(1),))
        self.conn.execute("INSERT INTO chain_analysis_labels VALUES('private-label','p','bitcoin','main',?,'Private employer','exchange','My bank statement','user_confirmed',1,1,0,?,?)", (txid(1), NOW, NOW))
        owner = run_analysis(self.conn, "p", {"observer": "owner", "include_hypotheses": True})
        self.assertTrue(any(node["kind"] == "record" for node in owner["nodes"]))
        self.assertTrue(owner["exposure"])
        public = run_analysis(self.conn, "p", {"observer": "public", "include_hypotheses": True})
        self.assertTrue(public["nodes"])
        self.assertFalse(any(node["kind"] == "record" for node in public["nodes"]))
        self.assertFalse(any(node.get("wallet_ids") or node.get("transaction_id") for node in public["nodes"]))
        self.assertFalse(public["exposure"])
        self.assertFalse(any(edge["kind"] == "custody" for edge in public["edges"]))
        self.assertNotIn("private-utxo", json.dumps(public))
        self.assertEqual(public["coverage"]["observer_knowledge"], "public_chain_facts")
        disclosed = run_analysis(self.conn, "p", {"observer": "disclosed"})
        self.assertEqual(disclosed["coverage"]["observer_knowledge"], "assumes_all_local_owner_evidence_disclosed")

    def test_public_liquid_unblinded_amounts_do_not_enter_filters_or_analytics(self):
        from kassiber.core.chain_analysis import run_analysis
        from kassiber.core.chain_analysis.index import observer_index
        asset = "a" * 64
        owned = {"scriptpubkey": "0014" + "11" * 20, "value_sats": 1234, "asset_id": asset, "role": "owned"}
        fee = {"scriptpubkey": "", "value_sats": 50, "asset_id": asset, "role": "fee"}
        add_tx(self.conn, 1, chain="liquid", extra={"vout": [owned, fee]})
        self.conn.execute("INSERT INTO wallet_utxos VALUES('unblinded','p','w','liquid','liquidv1',?,0,1234000,?,'lq1private',NULL,'receive',NULL)", (txid(1), asset))
        index = build_index(self.conn, "p")
        oid = output_node_id("liquid", "liquidv1", txid(1), 0)
        fid = output_node_id("liquid", "liquidv1", txid(1), 1)
        public_index = observer_index(index, "public")
        self.assertEqual(index.nodes[oid]["amount_msat"], "1234000")
        self.assertIsNone(public_index.nodes[oid]["amount_msat"])
        self.assertIsNone(public_index.nodes[oid]["asset"])
        self.assertNotIn("address", public_index.nodes[oid])
        self.assertEqual(public_index.nodes[fid]["amount_msat"], "50000")
        self.assertEqual(public_index.nodes[fid]["asset"], asset)
        tf = public_index.transaction_facts[tx_node_id("liquid", "liquidv1", txid(1))]
        self.assertIsNone(tf["outputs"][0]["amount_msat"])
        result = run_analysis(self.conn, "p", {"mode": "trace", "subject": txid(1), "observer": "public", "min_amount_msat": "999999999"})
        self.assertIn(oid, {node["id"] for node in result["nodes"]})  # unknown remains unknown, not pruned using owner's value
        self.assertEqual(result["coverage"]["analytics"]["amount_complete_transaction_count"], 0)

    def test_commitment_backed_values_are_not_public_even_with_numeric_raw_value(self):
        from kassiber.core.chain_analysis.index import observer_index
        add_tx(self.conn, 1, chain="liquid", extra={"vout": [{"value": 1000, "valuecommitment": "08" + "ab" * 32, "asset": "a" * 64, "scriptpubkey": "0014" + "11" * 20}]})
        public = observer_index(build_index(self.conn, "p"), "public")
        output = public.nodes[output_node_id("liquid", "liquidv1", txid(1), 0)]
        self.assertIsNone(output["amount_msat"])
        self.assertIsNone(output["asset"])

    def test_payjoin_exclusion_dominates_later_duplicate_coinjoin_shape_in_both_orders(self):
        from kassiber.core.chain_analysis import run_entropy
        for marker_first in (True, False):
            with self.subTest(marker_first=marker_first):
                self.conn.execute("DELETE FROM transactions")
                add_tx(self.conn, 20, [(i, 0) for i in range(1, 6)], outputs=(900,) * 5, boundary="payjoin" if marker_first else None)
                add_tx(self.conn, 20, [(i, 0) for i in range(1, 6)], outputs=(900,) * 5, record_id="duplicate", boundary=None if marker_first else "payjoin")
                result = run_entropy(self.conn, "p", {"subject": txid(20)})
                self.assertEqual(result["status"], "unsupported")
                self.assertEqual(result["reason"], "joint_payment_or_unknown_collaboration")
                self.assertFalse(result["deterministic_links"])

    def test_script_and_branch_interpretation_changes_bind_snapshot(self):
        add_tx(self.conn, 1)
        self.conn.execute("INSERT INTO wallet_utxos VALUES('u','p','w','bitcoin','main',?,0,1000000,'BTC',NULL,?,'receive',NULL)", (txid(1), "0014" + "22" * 20))
        before = build_index(self.conn, "p")
        self.conn.execute("UPDATE wallet_utxos SET branch_label='change'")
        after = build_index(self.conn, "p")
        self.assertNotEqual(before.snapshot_id, after.snapshot_id)
        self.conn.execute("UPDATE wallet_utxos SET script_pubkey=?", ("0014" + "33" * 20,))
        self.assertNotEqual(after.snapshot_id, build_index(self.conn, "p").snapshot_id)

    def test_conflicting_same_rank_scripts_cannot_create_reuse_clusters(self):
        from kassiber.core.chain_analysis import run_analysis
        add_tx(self.conn, 1, extra={"vout": [{"value": 1000, "scriptpubkey": "0014" + "11" * 20}]})
        add_tx(self.conn, 1, record_id="conflict", extra={"vout": [{"value": 1000, "scriptpubkey": "0014" + "22" * 20}]})
        result = run_analysis(self.conn, "p", {"include_hypotheses": True})
        self.assertTrue(any(row["code"] == "conflicting_output_script" for row in result["findings"]))
        self.assertFalse(result["clusters"])

    def test_snapshot_analysis_is_pure_and_matches_workbench(self):
        add_tx(self.conn, 1)
        add_tx(self.conn, 2, [(1, 0)], outputs=(900,))
        args = {"observer": "public", "include_hypotheses": True}
        expected = run_analysis(self.conn, "p", args)
        index = build_index(self.conn, "p")
        # A frozen snapshot must remain usable after its DB connection ends.
        self.conn.close()
        self.assertEqual(analyze_snapshot(index, args), expected)

    def test_branch_evidence_and_duplicate_ownership_are_canonical_and_private(self):
        from kassiber.core.chain_analysis.index import observer_index
        self.conn.execute("ALTER TABLE wallet_utxos ADD COLUMN branch_index INTEGER")
        add_tx(self.conn, 1)
        self.conn.execute("INSERT INTO wallet_utxos VALUES('u','p','w','bitcoin','main',?,0,1000000,'BTC',NULL,?,'p2tr change',NULL,6)", (txid(1), "0014" + "22" * 20))
        oid = output_node_id("bitcoin", "main", txid(1), 0)
        index = build_index(self.conn, "p")
        self.assertEqual(index.output_facts[oid]["branch_role"], "change")
        self.assertEqual(index.output_facts[oid]["change_evidence"], "imported")
        self.assertTrue(index.output_facts[oid]["ownership_known"])
        public = observer_index(index, "public")
        self.assertFalse(public.output_facts[oid]["ownership_known"])
        self.assertIsNone(public.output_facts[oid]["branch_role"])
        self.assertNotIn("branch_source", public.output_facts[oid])
        self.conn.execute("INSERT INTO wallet_utxos SELECT 'duplicate',profile_id,'another',chain,network,txid,vout,amount,asset,address,script_pubkey,branch_label,spent_by,branch_index FROM wallet_utxos")
        duplicate = build_index(self.conn, "p")
        self.assertTrue(duplicate.output_facts[oid]["ownership_ambiguous"])
        self.assertFalse(duplicate.output_facts[oid]["ownership_known"])
        self.assertEqual(duplicate.output_facts[oid]["branch_role"], "unknown")

    def test_retracted_transactions_do_not_publish_current_structural_findings(self):
        add_tx(self.conn, 1)
        add_tx(self.conn, 2, [(1, 0)], extra={"vin": [{"txid": txid(1), "vout": 0, "sequence": 0xFFFFFFFD}], "confirmations": -1})
        result = run_analysis(self.conn, "p", {"observer": "public"})
        tid = tx_node_id("bitcoin", "main", txid(2))
        self.assertNotIn(tid, {row["subject"] for row in result["transaction_features"]})
        self.assertFalse(any(row["code"] == "explicit_rbf_signal" and tid in row["node_ids"] for row in result["findings"]))


def test_partial_duplicate_cannot_extend_a_complete_transaction_in_either_order():
    from kassiber.core.chain_analysis import run_entropy
    for partial_first in (True, False):
        conn = connection()
        try:
            order = (True, False) if partial_first else (False, True)
            for partial in order:
                add_tx(conn, 2, [(3 if partial else 1, 0)], outputs=(500,), record_id="partial" if partial else "full", extra={"vout": [{}]} if partial else None)
            index = build_index(conn, "p")
            node = tx_node_id("bitcoin", "main", txid(2))
            assert index.nodes[node]["status"] == "conflicting"
            assert not index.transaction_facts[node]["complete"]
            result = run_entropy(conn, "p", {"subject": txid(2)})
            assert result["status"] == "unsupported"
            assert not result["deterministic_links"]
        finally:
            conn.close()


def test_compatible_partial_observation_can_enrich_complete_shape():
    conn = connection()
    try:
        add_tx(conn, 2, [(1, 0)], outputs=(500,))
        add_tx(conn, 2, [(1, 0)], record_id="partial", extra={"vout": [{}]})
        index = build_index(conn, "p")
        node = tx_node_id("bitcoin", "main", txid(2))
        assert index.nodes[node]["status"] == "observed"
        assert index.transaction_facts[node]["complete"]
        assert len(index.transaction_facts[node]["inputs"]) == 1
    finally:
        conn.close()
