"""Differential and transactional checks for the real incremental query path."""
import json
import random
import unittest
from unittest.mock import patch

from kassiber.core.chain_analysis import analyze_snapshot, build_index, run_analysis
from kassiber.core.chain_analysis.projection import index_revision, read_index, rebuild_index
from tests.test_chain_analysis import connection, add_tx, txid


def semantic(value):
    value = json.loads(json.dumps(value))
    value.pop("snapshot_id", None)
    value["findings"].sort(key=lambda row: row["id"])
    return value


class IncrementalProjectionTests(unittest.TestCase):
    def setUp(self):
        self.conn = connection()

    def tearDown(self):
        self.conn.close()

    def parity(self, subject=None):
        oracle = build_index(self.conn, "p")
        for observer in ("owner", "public"):
            query = {"observer": observer, "depth": 12, "include_hypotheses": True}
            if subject:
                query.update(mode="trace", subject=subject)
            self.assertEqual(semantic(run_analysis(self.conn, "p", query)), semantic(analyze_snapshot(oracle, query)))

    def test_source_replace_delete_missing_prevout_and_competing_spend(self):
        add_tx(self.conn, 1, outputs=(900, 100))
        add_tx(self.conn, 2, [(1, 0)], outputs=(800,))
        self.parity()
        add_tx(self.conn, 3, [(1, 0)], outputs=(700,))
        self.parity()
        raw = json.loads(self.conn.execute("SELECT raw_json FROM transactions WHERE id='2'").fetchone()[0])
        raw["removed"] = True
        self.conn.execute("UPDATE transactions SET raw_json=? WHERE id='2'", (json.dumps(raw),))
        self.parity()
        self.conn.execute("DELETE FROM transactions WHERE id='3'")
        self.parity()
        self.conn.execute("DELETE FROM transactions WHERE id='1'")
        self.parity()

    def test_network_alias_without_chain_preserves_liquid_seeds(self):
        add_tx(self.conn, 1)
        add_tx(self.conn, 2, chain="liquid", network="liquidv1")
        query = {"network": "main", "observer": "public"}
        self.assertEqual(semantic(run_analysis(self.conn, "p", query)), semantic(analyze_snapshot(build_index(self.conn, "p"), query)))

    def test_inventory_ownership_replacement_does_not_leave_private_or_public_aliases(self):
        add_tx(self.conn, 1)
        self.parity()
        self.conn.execute("INSERT INTO wallet_utxos VALUES('u','p','w','bitcoin','main',?,0,1000000,'BTC','private-address',?,'change',NULL)", (txid(1), "0014" + "01" * 20))
        self.parity()
        self.conn.execute("DELETE FROM wallet_utxos")
        self.parity()
        with read_index(self.conn, "p") as view:
            self.assertNotIn("private-address", view.subjects)
            self.assertNotIn("private-address", view.for_observer("public").subjects)

    def test_revision_durable_noop_rebuild_and_source_rollback(self):
        add_tx(self.conn, 1)
        self.conn.commit()
        first = index_revision(self.conn, "p")
        self.conn.commit()
        self.assertEqual(index_revision(self.conn, "p"), first)
        self.assertEqual(rebuild_index(self.conn, "p")["snapshot_id"], first["snapshot_id"])
        self.conn.commit()
        self.conn.execute("BEGIN")
        add_tx(self.conn, 2, [(1, 0)])
        self.assertGreater(index_revision(self.conn, "p")["revision"], first["revision"])
        self.conn.rollback()
        self.assertEqual(index_revision(self.conn, "p"), first)
        self.parity()

    def test_fault_during_publication_rolls_back_and_replays(self):
        add_tx(self.conn, 1)
        self.conn.commit()
        initial = index_revision(self.conn, "p")
        self.conn.commit()
        add_tx(self.conn, 2, [(1, 0)])
        self.conn.commit()
        with patch("kassiber.core.chain_analysis.projection_store._save", side_effect=RuntimeError("interrupted")):
            with self.assertRaisesRegex(RuntimeError, "interrupted"):
                index_revision(self.conn, "p")
        self.assertEqual(self.conn.execute("SELECT revision FROM chain_index_state").fetchone()[0], initial["revision"])
        self.parity()

    def test_warm_small_public_query_does_not_scan_graph_or_parse_sources(self):
        for number in range(1, 100):
            add_tx(self.conn, number, [(number - 1, 0)] if number > 1 else [])
        run_analysis(self.conn, "p", {"subject": txid(1), "mode": "trace", "observer": "public"})
        statements = []
        self.conn.set_trace_callback(statements.append)
        with patch("kassiber.core.chain_analysis.index._Builder.transaction", side_effect=AssertionError("warm query parsed source")):
            result = run_analysis(self.conn, "p", {"subject": txid(1), "mode": "trace", "observer": "public", "depth": 3})
        self.conn.set_trace_callback(None)
        self.assertEqual(len(result["nodes"]), 4)
        self.assertFalse(any("SELECT * FROM transactions" in sql for sql in statements))
        self.assertFalse(any("SELECT id FROM chain_index_nodes" in sql for sql in statements))
        self.assertLess(len(statements), 100)

    def test_randomized_replacement_and_retractions_match_rebuild(self):
        rng = random.Random(8319)
        for number in range(1, 18):
            parent = rng.randrange(1, number) if number > 1 else None
            add_tx(self.conn, number, [(parent, 0)] if parent else [])
            self.parity()
        for number in rng.sample(range(1, 18), 12):
            self.conn.execute("DELETE FROM transactions WHERE id=?", (str(number),))
            self.parity()

    def test_read_view_cannot_escape_snapshot(self):
        add_tx(self.conn, 1)
        with read_index(self.conn, "p") as view:
            self.assertTrue(view.nodes)
        with self.assertRaisesRegex(RuntimeError, "outside its read snapshot"):
            len(view.nodes)

    def test_late_reference_source_install_and_reconciliation_are_visible(self):
        add_tx(self.conn, 1)
        initial = index_revision(self.conn, "p")
        self.conn.execute("CREATE TABLE chain_analysis_reference_assertions(grant_id TEXT,profile_id TEXT,domain_id TEXT,chain TEXT,network TEXT,occurrence_id TEXT,txid TEXT,payload_json TEXT,status_json TEXT,active INTEGER,observed_at TEXT,PRIMARY KEY(grant_id,occurrence_id))")
        raw = {"txid": txid(2), "vin": [{"txid": txid(1), "vout": 0}], "vout": [{"value": 900, "scriptpubkey": "0014" + "22" * 20}]}
        self.conn.execute("INSERT INTO chain_analysis_reference_assertions VALUES('grant','p','domain','bitcoin','main','occurrence',?,?,?,1,'2026-09-07T12:00:00Z')", (txid(2), json.dumps(raw), '{}'))
        self.parity()
        self.assertGreater(index_revision(self.conn, "p")["revision"], initial["revision"])
        self.conn.execute("UPDATE chain_analysis_reference_assertions SET active=2")
        self.parity()
        result = run_analysis(self.conn, "p", {"observer": "public"})
        self.assertEqual(result["coverage"]["reference_reconciling_count"], 1)
        self.assertTrue(result["coverage"]["stale"])
        self.assertFalse(result["coverage"]["complete"])
        self.assertFalse(any(node.get("txid") == txid(2) for node in result["nodes"]))
        self.conn.execute("UPDATE chain_analysis_reference_assertions SET active=1")
        self.parity()


class ProjectionDatasetTests(unittest.TestCase):
    def test_label_expiry_advances_revision_without_source_write(self):
        from tests.test_chain_analysis_datasets import manifest, binary, TXID
        from tests.test_privacy_hygiene import PrivacyHygieneTests
        from kassiber.core import chain_analysis_datasets as datasets
        book = PrivacyHygieneTests()
        book.setUp()
        try:
            conn = book.conn
            book._insert_transaction(tx_id="watched", external_id=TXID, raw_json={"txid": TXID, "vin": [{"coinbase": "01"}], "vout": [{"value": 1000, "scriptpubkey": "0014" + "11" * 20}]})
            conn.commit()
            datasets.import_dataset(conn, "pf", manifest(), binary({"subject": TXID, "label": "Earlier", "valid_until": "2026-09-08"}, {"subject": TXID, "label": "Later", "valid_from": "2026-09-08"}), format="jsonl")
            query = {"mode": "trace", "subject": TXID, "observer": "public", "include_hypotheses": True}
            def at(instant):
                with patch("kassiber.core.chain_analysis.projection_store.now_iso", return_value=instant), patch.object(datasets, "now_iso", return_value=instant):
                    result = run_analysis(conn, "pf", query)
                    revision = index_revision(conn, "pf")
                    oracle = analyze_snapshot(build_index(conn, "pf"), query)
                    self.assertEqual(semantic(result), semantic(oracle))
                    return result, revision
            before, first = at("2026-09-07T12:00:00Z")
            clock = conn.execute("SELECT revision FROM chain_index_clock").fetchone()[0]
            self.assertEqual(first["next_expiry"], "2026-09-08T00:00:00Z")
            after, second = at("2026-09-08T00:00:00Z")
            self.assertEqual(conn.execute("SELECT revision FROM chain_index_clock").fetchone()[0], clock)
            self.assertGreater(second["revision"], first["revision"])
            self.assertNotEqual(before["exposure"], after["exposure"])
            self.assertEqual(at("2026-09-08T00:00:01Z")[1], second)
        finally:
            book.tearDown()


if __name__ == "__main__":
    unittest.main()
