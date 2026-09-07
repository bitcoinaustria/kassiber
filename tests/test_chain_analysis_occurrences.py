"""Occurrence/instance counterexamples through the real indexed query seam."""
import json
import unittest

from kassiber.core.chain_analysis import analyze_snapshot, build_index, run_analysis
from kassiber.core.chain_analysis.index import digest
from kassiber.core.chain_analysis.projection import index_revision
from kassiber.errors import AppError
from tests.test_chain_analysis import connection, add_tx, txid
from tests.test_chain_analysis_projection import semantic


class OccurrenceIndexTests(unittest.TestCase):
    def setUp(self):
        self.conn = connection()
        self.conn.execute("CREATE TABLE chain_analysis_reference_assertions(grant_id TEXT,profile_id TEXT,domain_id TEXT,chain TEXT,network TEXT,occurrence_id TEXT,txid TEXT,payload_json TEXT,status_json TEXT,active INTEGER,observed_at TEXT,PRIMARY KEY(grant_id,occurrence_id))")

    def tearDown(self):
        self.conn.close()

    def assertion(self, grant, block, number, *, parent=None, parent_occurrence=None, domain=None, network="main"):
        occurrence = f"{block:064x}:0"
        vin = [{"coinbase": "0101"}] if parent is None else [{"txid": txid(parent), "vout": 0, **({"reference_occurrence_id": parent_occurrence, "reference_grant_id": grant} if parent_occurrence else {})}]
        raw = {"txid": txid(number), "vin": vin, "vout": [{"value": 1000, "scriptpubkey": "0014" + f"{number:040x}"}]}
        self.conn.execute("INSERT INTO chain_analysis_reference_assertions VALUES(?,?,?,?,?,?,?,?,?,1,?)", (grant, "p", domain or digest(["bitcoin", network, None]), "bitcoin", network, occurrence, txid(number), json.dumps(raw), json.dumps({"confirmed": True, "block_hash": f"{block:064x}", "block_height": block}), "2026-09-07T12:00:00Z"))
        return occurrence

    def parity(self):
        for observer in ("owner", "public"):
            args = {"observer": observer, "depth": 12}
            self.assertEqual(semantic(run_analysis(self.conn, "p", args)), semantic(analyze_snapshot(build_index(self.conn, "p"), args)))

    def test_duplicate_txids_remain_distinct_and_grants_coalesce_physical_occurrence(self):
        first = self.assertion("one", 10, 1)
        self.parity()
        before = run_analysis(self.conn, "p", {"subject": txid(1), "mode": "trace"})
        self.assertIn(f":occ:{first}:tx:", next(row["id"] for row in before["nodes"] if row["kind"] == "transaction"))
        self.assertion("two", 10, 1)
        self.parity()
        repeated = run_analysis(self.conn, "p", {"subject": txid(1), "mode": "trace"})
        self.assertEqual(repeated["summary"]["transaction_count"], 1)
        second = self.assertion("one", 20, 1)
        self.parity()
        with self.assertRaises(AppError) as caught:
            run_analysis(self.conn, "p", {"subject": txid(1), "mode": "trace"})
        self.assertEqual(caught.exception.code, "subject_ambiguous")
        self.assertion("one", 21, 2, parent=1, parent_occurrence=second)
        self.parity()
        traced = run_analysis(self.conn, "p", {"subject": txid(2), "mode": "trace", "direction": "backward", "depth": 10})
        self.assertTrue(any(f":occ:{second}:tx:" in row["id"] for row in traced["nodes"]))
        self.assertFalse(any(f":occ:{first}:tx:" in row["id"] for row in traced["nodes"]))
        self.conn.execute("UPDATE chain_analysis_reference_assertions SET active=0 WHERE occurrence_id=?", (second,))
        self.parity()
        traced = run_analysis(self.conn, "p", {"subject": txid(2), "mode": "trace", "direction": "backward", "depth": 10})
        self.assertFalse(any(f":occ:{first}:tx:" in row["id"] for row in traced["nodes"]))
        self.assertTrue(traced["coverage"]["missing"])

    def test_unqualified_input_cannot_choose_between_historical_occurrences(self):
        self.assertion("one", 10, 1)
        add_tx(self.conn, 2, [(1, 0)])
        self.parity()
        self.assertion("one", 20, 1)
        self.parity()
        result = run_analysis(self.conn, "p", {"subject": txid(2), "mode": "trace", "direction": "backward", "depth": 10})
        self.assertFalse(any(":occ:" in row["id"] for row in result["nodes"]))
        self.assertTrue(any(row["status"] == "conflicting" for row in result["nodes"]))

    def test_reused_address_spans_outputs_in_one_occurrence_domain(self):
        self.assertion("one", 10, 1)
        self.assertion("one", 11, 2)
        for row in list(self.conn.execute("SELECT rowid,payload_json FROM chain_analysis_reference_assertions")):
            raw = json.loads(row[1])
            raw["vout"][0]["scriptpubkey_address"] = "shared-address"
            self.conn.execute("UPDATE chain_analysis_reference_assertions SET payload_json=? WHERE rowid=?", (json.dumps(raw), row[0]))
        result = run_analysis(self.conn, "p", {"subject": "shared-address", "mode": "trace"})
        self.assertEqual(result["summary"]["transaction_count"], 2)

    def test_occurrence_with_mismatched_block_anchor_is_rejected(self):
        self.assertion("one", 10, 1)
        self.conn.execute("UPDATE chain_analysis_reference_assertions SET status_json='{}'")
        self.parity()
        result = run_analysis(self.conn, "p", {})
        self.assertEqual(result["summary"]["transaction_count"], 0)
        self.assertEqual(result["coverage"]["invalid_observations"], 1)

    def test_bound_regtest_books_cannot_borrow_each_others_cache(self):
        self.conn.execute("CREATE TABLE book_network_bindings(profile_id TEXT PRIMARY KEY,environment_id TEXT,environment TEXT,revision INTEGER,chain_instance_id TEXT,domains_json TEXT,acknowledgements_json TEXT)")
        self.conn.execute("INSERT INTO profiles VALUES('other',NULL,0,0,0)")
        for profile, instance in (("p", "11111111-1111-4111-8111-111111111111"), ("other", "22222222-2222-4222-8222-222222222222")):
            domain = {"domain_id": digest(["bitcoin", "regtest", instance]), "chain": "bitcoin", "network": "regtest", "chain_instance_id": instance}
            self.conn.execute("INSERT INTO book_network_bindings VALUES(?,?,'regtest',1,?,?, '[]')", (profile, profile, instance, json.dumps([domain])))
        for number, instance in ((1, "11111111-1111-4111-8111-111111111111"), (2, "22222222-2222-4222-8222-222222222222"), (3, None)):
            raw = {"txid": txid(number), "chain_instance_id": instance, "vin": [{"coinbase": "01"}], "vout": [{"value": 1000}]}
            self.conn.execute("INSERT INTO transaction_graph_cache VALUES(1,'bitcoin','regtest',?,?,'now','now')", (txid(number), json.dumps(raw)))
        for profile, own, foreign in (("p", 1, 2), ("other", 2, 1)):
            result = run_analysis(self.conn, profile, {"subject": txid(own), "mode": "trace"})
            self.assertIn(":domain:", result["nodes"][0]["id"])
            for number in (foreign, 3):
                with self.assertRaises(AppError):
                    run_analysis(self.conn, profile, {"subject": txid(number), "mode": "trace"})
            self.assertEqual(result["coverage"]["cache_rejected"], 2)


if __name__ == "__main__":
    unittest.main()
