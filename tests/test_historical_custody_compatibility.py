from __future__ import annotations

import base64
import gzip
import json
import tempfile
import unittest
from pathlib import Path

from kassiber.core.custody_components import list_components
from kassiber.core.custody_gaps import load_gap_candidates
from kassiber.db import open_db


FIXTURE_ROOT = Path(__file__).parent / "fixtures" / "historical"


def _open_fixture(name: str):
    encoded = (FIXTURE_ROOT / name).read_bytes()
    database = gzip.decompress(base64.b64decode(encoded))
    temporary = tempfile.TemporaryDirectory(prefix="kassiber-historical-compat-")
    data_root = Path(temporary.name) / "data"
    data_root.mkdir(parents=True)
    (data_root / "kassiber.sqlite3").write_bytes(database)
    return temporary, open_db(data_root)


class HistoricalCustodyCompatibilityTests(unittest.TestCase):
    def _assert_migrated_fixture(self, fixture_name: str, schema_ref: str) -> None:
        temporary, conn = _open_fixture(fixture_name)
        self.addCleanup(temporary.cleanup)
        self.addCleanup(conn.close)

        self.assertEqual(conn.execute("PRAGMA integrity_check").fetchone()[0], "ok")
        self.assertEqual(
            conn.execute(
                "SELECT value FROM settings WHERE key = 'historical_fixture_ref'"
            ).fetchone()[0],
            schema_ref,
        )

        pair = conn.execute(
            "SELECT * FROM transaction_pairs WHERE id = 'manual-pair'"
        ).fetchone()
        self.assertEqual(pair["kind"], "manual")
        self.assertEqual(pair["pair_source"], "user")
        self.assertEqual(pair["out_amount"], 100_000_000_000)
        self.assertIsNone(pair["component_id"])

        authored_active = list_components(conn, profile_id="pf", state="active")
        self.assertEqual(
            {item["id"] for item in authored_active},
            {"component-replica-a", "component-replica-b"},
        )
        self.assertEqual({item["effective_state"] for item in authored_active}, {"draft"})
        for item in authored_active:
            issue_codes = {issue["code"] for issue in item["validation"]["issues"]}
            self.assertIn("active_lineage_conflict", issue_codes)
            self.assertIn("component_evidence_commitment_invalid", issue_codes)
        self.assertEqual(
            list_components(conn, profile_id="pf", effective_only=True),
            [],
        )

        samourai_config = json.loads(
            conn.execute(
                "SELECT config_json FROM wallets WHERE id = 'sam-postmix'"
            ).fetchone()[0]
        )
        self.assertEqual(samourai_config["samourai"]["section"], "postmix")
        self.assertTrue(samourai_config["samourai"]["whirlpool"])
        self.assertNotIn("descriptor", samourai_config)
        self.assertEqual(
            conn.execute(
                "SELECT COUNT(*) FROM wallet_policy_epochs WHERE wallet_id = 'sam-postmix'"
            ).fetchone()[0],
            0,
        )

        candidates, _rows = load_gap_candidates(conn, "pf")
        missing_whirlpool = next(
            candidate
            for candidate in candidates
            if candidate.source_ids == ("whirlpool-out",)
            and candidate.return_ids == ("whirlpool-return",)
        )
        self.assertEqual(missing_whirlpool.source_total_msat, 1_000_000_000_000)
        self.assertEqual(missing_whirlpool.return_total_msat, 990_000_000_000)
        self.assertIn("unresolved_residual", missing_whirlpool.reason_codes)

        self.assertEqual(
            conn.execute("SELECT COUNT(*) FROM filed_report_snapshots").fetchone()[0],
            0,
        )
        self.assertEqual(
            conn.execute(
                "SELECT COUNT(*) FROM journal_entries WHERE id = 'legacy-journal'"
            ).fetchone()[0],
            1,
        )
        self.assertEqual(
            conn.execute(
                "SELECT COUNT(*) FROM journal_tax_summary WHERE id = 'legacy-tax-summary'"
            ).fetchone()[0],
            1,
        )

        self.assertEqual(conn.execute("SELECT COUNT(*) FROM sync_events").fetchone()[0], 2)
        conflict = conn.execute(
            "SELECT * FROM sync_conflicts WHERE id = 'custody-conflict'"
        ).fetchone()
        self.assertEqual(conflict["status"], "open")
        self.assertEqual(conflict["entity_table"], "custody_components")

    def test_pre_432_database_upgrades_without_reinterpreting_custody(self):
        self._assert_migrated_fixture(
            "pre_432_5d232097.sqlite3.gz.b64",
            "5d232097",
        )


if __name__ == "__main__":
    unittest.main()
