import json
import sqlite3
import subprocess
import sys
import tempfile
import unittest
import uuid
from pathlib import Path

from kassiber.core.source_funds_coverage import (
    COVERAGE_BUCKETS,
    _classify_transaction,
    compute_coverage,
    coverage_summary_text,
)


ROOT = Path(__file__).resolve().parent.parent


def run_cli(data_root: Path, *args: str):
    result = subprocess.run(
        [
            sys.executable,
            "-m",
            "kassiber",
            "--data-root",
            str(data_root),
            "--machine",
            *args,
        ],
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise AssertionError(
            f"CLI failed with code {result.returncode}: stdout={result.stdout} stderr={result.stderr}"
        )
    payload = json.loads(result.stdout)
    return payload


class CoverageCoreTests(unittest.TestCase):
    """Unit-level tests on the classifier with a hand-seeded sqlite DB.

    Bypasses the CLI to make the algorithm tests fast and deterministic.
    The schema is loaded from kassiber/db.py via open_db() to stay in
    sync with the production schema.
    """

    def setUp(self):
        from kassiber import db as kassiber_db

        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.db_path = Path(self.tmp.name) / "kassiber.sqlite3"
        self.conn = kassiber_db.open_db(self.db_path)
        self.workspace_id = "ws-1"
        self.profile_id = "prof-1"
        self.wallet_id = "wal-1"
        self.account_id = "acct-1"
        now = "2026-04-01T00:00:00Z"
        self.conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES (?, ?, ?)",
            (self.workspace_id, "ws", now),
        )
        self.conn.execute(
            "INSERT INTO profiles(id, workspace_id, label, fiat_currency, tax_country, created_at)"
            " VALUES (?, ?, ?, ?, ?, ?)",
            (self.profile_id, self.workspace_id, "Default", "EUR", "generic", now),
        )
        self.conn.execute(
            "INSERT INTO accounts(id, workspace_id, profile_id, code, label, account_type, created_at)"
            " VALUES (?, ?, ?, ?, ?, ?, ?)",
            (self.account_id, self.workspace_id, self.profile_id, "acct", "acct", "personal", now),
        )
        self.conn.execute(
            "INSERT INTO wallets(id, workspace_id, profile_id, account_id, label, kind, created_at)"
            " VALUES (?, ?, ?, ?, ?, ?, ?)",
            (self.wallet_id, self.workspace_id, self.profile_id, self.account_id, "Target", "personal", now),
        )
        self.conn.commit()

    def _add_inbound_tx(self, external_id: str, amount_msat: int, asset: str = "BTC") -> str:
        tx_id = str(uuid.uuid4())
        self.conn.execute(
            """
            INSERT INTO transactions(id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset, amount, raw_json, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                tx_id,
                self.workspace_id,
                self.profile_id,
                self.wallet_id,
                external_id,
                f"fp-{tx_id}",
                "2026-04-01T09:00:00Z",
                "inbound",
                asset,
                amount_msat,
                "{}",
                "2026-04-01T09:00:00Z",
            ),
        )
        self.conn.commit()
        return tx_id

    def _add_outbound_tx(self, external_id: str, amount_msat: int, asset: str = "BTC") -> str:
        tx_id = str(uuid.uuid4())
        self.conn.execute(
            """
            INSERT INTO transactions(id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset, amount, raw_json, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                tx_id,
                self.workspace_id,
                self.profile_id,
                self.wallet_id,
                external_id,
                f"fp-{tx_id}",
                "2026-03-31T09:00:00Z",
                "outbound",
                asset,
                amount_msat,
                "{}",
                "2026-03-31T09:00:00Z",
            ),
        )
        self.conn.commit()
        return tx_id

    def _add_source(self, source_type: str, asset: str = "BTC", amount_msat: int | None = None) -> str:
        source_id = str(uuid.uuid4())
        self.conn.execute(
            """
            INSERT INTO source_funds_sources(id, workspace_id, profile_id, source_type, label, asset,
                amount, fiat_currency, fiat_value, acquired_at, description, review_state, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                source_id,
                self.workspace_id,
                self.profile_id,
                source_type,
                f"src-{source_type}",
                asset,
                amount_msat,
                "EUR",
                None,
                None,
                None,
                "reviewed",
                "2026-03-30T09:00:00Z",
                "2026-03-30T09:00:00Z",
            ),
        )
        self.conn.commit()
        return source_id

    def _add_link(
        self,
        *,
        to_tx_id: str,
        from_source_id: str | None = None,
        from_tx_id: str | None = None,
        state: str = "reviewed",
        link_type: str = "self_transfer",
        asset: str = "BTC",
        allocation_msat: int = 0,
    ) -> str:
        link_id = str(uuid.uuid4())
        self.conn.execute(
            """
            INSERT INTO source_funds_links(id, workspace_id, profile_id, from_source_id, from_transaction_id,
                to_transaction_id, link_type, state, confidence, method, asset, allocation_amount, from_asset,
                from_allocation_amount, allocation_policy, explanation, uses_chain_observation,
                chain_data_confirmed, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                link_id,
                self.workspace_id,
                self.profile_id,
                from_source_id,
                from_tx_id,
                to_tx_id,
                link_type,
                state,
                "strong",
                "manual",
                asset,
                allocation_msat,
                asset,
                allocation_msat,
                "explicit",
                "",
                0,
                1,
                "2026-04-01T09:00:00Z",
                "2026-04-01T09:00:00Z",
            ),
        )
        self.conn.commit()
        return link_id

    def test_no_links_yields_untraced(self):
        tx = self._add_inbound_tx("solo", 100_000)
        bucket = _classify_transaction(self.conn, self.profile_id, tx)
        self.assertEqual(bucket, "untraced")

    def test_reviewed_to_real_source_is_fully_traced(self):
        tx = self._add_inbound_tx("real-src", 100_000)
        src = self._add_source("fiat_purchase")
        self._add_link(to_tx_id=tx, from_source_id=src, allocation_msat=100_000)
        bucket = _classify_transaction(self.conn, self.profile_id, tx)
        self.assertEqual(bucket, "fully_traced")

    def test_reviewed_to_attestation_only_is_attested(self):
        tx = self._add_inbound_tx("attested", 100_000)
        src = self._add_source("missing_history")
        self._add_link(to_tx_id=tx, from_source_id=src, allocation_msat=100_000)
        bucket = _classify_transaction(self.conn, self.profile_id, tx)
        self.assertEqual(bucket, "attested")

    def test_only_suggestions_is_in_review(self):
        tx = self._add_inbound_tx("pending", 100_000)
        src = self._add_source("fiat_purchase")
        self._add_link(to_tx_id=tx, from_source_id=src, state="suggested", allocation_msat=100_000)
        bucket = _classify_transaction(self.conn, self.profile_id, tx)
        self.assertEqual(bucket, "in_review")

    def test_real_source_wins_over_attestation(self):
        tx = self._add_inbound_tx("mixed", 100_000)
        attest = self._add_source("missing_history")
        real = self._add_source("fiat_purchase")
        self._add_link(to_tx_id=tx, from_source_id=attest, allocation_msat=100_000)
        self._add_link(to_tx_id=tx, from_source_id=real, allocation_msat=100_000)
        bucket = _classify_transaction(self.conn, self.profile_id, tx)
        self.assertEqual(bucket, "fully_traced")

    def test_walks_through_parent_transaction_to_root(self):
        target = self._add_inbound_tx("target", 100_000)
        parent = self._add_inbound_tx("parent", 100_000)
        src = self._add_source("fiat_purchase")
        self._add_link(to_tx_id=target, from_tx_id=parent, allocation_msat=100_000)
        self._add_link(to_tx_id=parent, from_source_id=src, allocation_msat=100_000)
        bucket = _classify_transaction(self.conn, self.profile_id, target)
        self.assertEqual(bucket, "fully_traced")

    def test_cycle_does_not_loop(self):
        a = self._add_inbound_tx("a", 100_000)
        b = self._add_inbound_tx("b", 100_000)
        # a depends on b, b depends on a (no source). Classifier must terminate.
        self._add_link(to_tx_id=a, from_tx_id=b, allocation_msat=100_000)
        self._add_link(to_tx_id=b, from_tx_id=a, allocation_msat=100_000)
        bucket = _classify_transaction(self.conn, self.profile_id, a)
        # No real source reached; classifier returns in_review (not fully_traced).
        self.assertNotEqual(bucket, "fully_traced")

    def test_compute_coverage_buckets_are_exhaustive(self):
        traced = self._add_inbound_tx("traced", 100_000)
        attested = self._add_inbound_tx("attested", 50_000)
        in_review = self._add_inbound_tx("in_review", 25_000)
        untraced = self._add_inbound_tx("untraced", 12_500)
        # Outbound should be ignored:
        self._add_outbound_tx("outbound-noise", 999_999)

        real = self._add_source("fiat_purchase")
        attest = self._add_source("opening_balance_attestation")
        self._add_link(to_tx_id=traced, from_source_id=real, allocation_msat=100_000)
        self._add_link(to_tx_id=attested, from_source_id=attest, allocation_msat=50_000)
        self._add_link(
            to_tx_id=in_review,
            from_source_id=real,
            state="suggested",
            allocation_msat=25_000,
        )

        coverage = compute_coverage(self.conn, self.profile_id)
        totals = coverage["totals"]
        self.assertEqual(totals["tx_count"], 4)
        # Outbound was excluded; sum is 100000 + 50000 + 25000 + 12500.
        self.assertEqual(totals["amount_msat"], 100_000 + 50_000 + 25_000 + 12_500)
        bucket_amounts = {name: totals["buckets"][name]["amount_msat"] for name in COVERAGE_BUCKETS}
        self.assertEqual(bucket_amounts["fully_traced"], 100_000)
        self.assertEqual(bucket_amounts["attested"], 50_000)
        self.assertEqual(bucket_amounts["in_review"], 25_000)
        self.assertEqual(bucket_amounts["untraced"], 12_500)
        # Buckets must be mutually exclusive: tx_counts sum to total tx_count.
        bucket_counts = {name: totals["buckets"][name]["tx_count"] for name in COVERAGE_BUCKETS}
        self.assertEqual(sum(bucket_counts.values()), totals["tx_count"])

    def test_compute_coverage_groups_by_wallet_and_asset(self):
        tx_btc = self._add_inbound_tx("btc-1", 100_000, asset="BTC")
        tx_lbtc = self._add_inbound_tx("lbtc-1", 50_000, asset="L-BTC")
        coverage = compute_coverage(self.conn, self.profile_id)
        assets = {entry["asset"] for entry in coverage["by_asset"]}
        self.assertEqual(assets, {"BTC", "L-BTC"})
        wallet_assets = {(entry["wallet_label"], entry["asset"]) for entry in coverage["by_wallet"]}
        self.assertEqual(wallet_assets, {("Target", "BTC"), ("Target", "L-BTC")})

    def test_summary_text_emits_a_line_per_bucket(self):
        tx = self._add_inbound_tx("solo", 100_000)
        coverage = compute_coverage(self.conn, self.profile_id)
        lines = coverage_summary_text(coverage)
        for bucket in COVERAGE_BUCKETS:
            self.assertTrue(any(bucket in line for line in lines), f"missing bucket {bucket} in output")


class CoverageCliSmokeTest(unittest.TestCase):
    """Smoke test that the CLI subcommand wiring is alive."""

    def test_help_works(self):
        result = subprocess.run(
            [sys.executable, "-m", "kassiber", "source-funds", "coverage", "--help"],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--workspace", result.stdout)
        self.assertIn("--profile", result.stdout)


if __name__ == "__main__":
    unittest.main()
