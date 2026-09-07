"""Current verified-tip overlays advance watches without rebuilding history."""
import json
import unittest
from unittest.mock import patch

from kassiber.core.chain_analysis import analyze_snapshot, build_index, run_analysis
from kassiber.core.chain_analysis.projection import index_revision
from tests.test_chain_analysis import txid
from tests import test_chain_analysis_occurrences as fixtures
from tests.test_chain_analysis_projection import semantic


class VerifiedTipTests(unittest.TestCase):
    assertion = fixtures.OccurrenceIndexTests.assertion
    tearDown = fixtures.OccurrenceIndexTests.tearDown
    def setUp(self):
        fixtures.OccurrenceIndexTests.setUp(self)
        self.conn.execute("""CREATE TABLE chain_analysis_acquisition_grants(id TEXT PRIMARY KEY,profile_id TEXT,
            status TEXT,verified_tip_height INTEGER,verified_tip_hash TEXT,verified_tip_at TEXT,last_code TEXT,
            requests_used INTEGER,lease_token TEXT)""")
        self.conn.execute("INSERT INTO chain_analysis_acquisition_grants VALUES('one','p','active',12,?,'2026-09-07T12:00:00Z','caught_up',0,NULL)", (txid(12),))

    def result(self):
        result = run_analysis(self.conn, "p", {"subject": txid(1), "mode": "trace", "observer": "public"})
        self.assertEqual(semantic(result), semantic(analyze_snapshot(build_index(self.conn, "p"), {"subject": txid(1), "mode": "trace", "observer": "public"})))
        return result

    def test_tip_overlay_updates_tx_and_outputs_without_rewriting_history(self):
        self.assertion("one", 10, 1)
        first = self.result()
        self.assertEqual({node["confirmations"] for node in first["nodes"]}, {3})
        before_sources = [tuple(row) for row in self.conn.execute("SELECT * FROM chain_analysis_reference_assertions")]
        before_nodes = [tuple(row) for row in self.conn.execute("SELECT * FROM chain_index_nodes")]
        self.conn.execute("UPDATE chain_analysis_acquisition_grants SET verified_tip_height=15,verified_tip_hash=?,verified_tip_at='2026-09-07T13:00:00Z'", (txid(15),))
        with patch("kassiber.core.chain_analysis.projection_store._assemble", side_effect=AssertionError("tip rebuilt graph")), patch("kassiber.core.chain_analysis.projection_store._labels", side_effect=AssertionError("tip rematched labels")):
            after = self.result()
        self.assertNotEqual(first["snapshot_id"], after["snapshot_id"])
        self.assertEqual({node["confirmations"] for node in after["nodes"]}, {6})
        self.assertEqual({node["confirmation_observed_at"] for node in after["nodes"]}, {"2026-09-07T13:00:00Z"})
        self.assertEqual(before_sources, [tuple(row) for row in self.conn.execute("SELECT * FROM chain_analysis_reference_assertions")])
        self.assertEqual(before_nodes, [tuple(row) for row in self.conn.execute("SELECT * FROM chain_index_nodes")])
        self.assertNotIn("verified_tip_hash", json.dumps(after))
        self.assertNotIn("grant_id", json.dumps(after))
        revision = index_revision(self.conn, "p")
        self.conn.execute("UPDATE chain_analysis_acquisition_grants SET lease_token='private-lease',requests_used=99")
        self.assertEqual(index_revision(self.conn, "p"), revision)

    def test_stale_or_reconciling_source_never_reuses_persisted_count(self):
        self.assertion("one", 10, 1)
        self.assertEqual(self.result()["nodes"][0]["confirmations"], 3)
        for code in ("chain_analysis_stale", "reorg_reconciling"):
            self.conn.execute("UPDATE chain_analysis_acquisition_grants SET last_code=?", (code,))
            self.assertTrue(all(node["confirmations"] is None for node in self.result()["nodes"]))
        self.conn.execute("UPDATE chain_analysis_acquisition_grants SET last_code='caught_up',verified_tip_height=NULL")
        self.assertTrue(all(node["confirmations"] is None for node in self.result()["nodes"]))

    def test_newest_verified_source_and_equal_time_disagreement(self):
        self.assertion("one", 10, 1)
        self.assertion("two", 10, 1)
        self.conn.execute("INSERT INTO chain_analysis_acquisition_grants VALUES('two','p','active',20,?,'2026-09-07T13:00:00Z','caught_up',0,NULL)", (txid(20),))
        self.assertEqual(self.result()["nodes"][0]["confirmations"], 11)
        self.conn.execute("UPDATE chain_analysis_acquisition_grants SET verified_tip_at='2026-09-07T13:00:00Z' WHERE id='one'")
        self.assertTrue(all(node["confirmations"] is None for node in self.result()["nodes"]))
        self.conn.execute("UPDATE chain_analysis_acquisition_grants SET last_code='chain_analysis_stale' WHERE id='one'")
        self.assertEqual(self.result()["nodes"][0]["confirmations"], 11)


if __name__ == '__main__':
    unittest.main()
