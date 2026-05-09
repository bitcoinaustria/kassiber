import json
import sqlite3
import subprocess
import sys
import tempfile
import unittest
import uuid
from pathlib import Path

from kassiber.core.source_funds import SourceFundsHooks
from kassiber.core.source_funds_coverage import (
    COVERAGE_BUCKETS,
    _classify_transaction,
    compute_coverage,
    coverage_summary_text,
)


def _stub_resolve_scope(workspace_id: str, profile_id: str):
    def resolve(conn, workspace_ref, profile_ref):
        workspace_row = conn.execute(
            "SELECT id, label FROM workspaces WHERE id = ?",
            (workspace_id,),
        ).fetchone()
        profile_row = conn.execute(
            "SELECT id, label FROM profiles WHERE id = ?",
            (profile_id,),
        ).fetchone()
        return (
            {"id": workspace_row["id"], "label": workspace_row["label"]},
            {"id": profile_row["id"], "label": profile_row["label"]},
        )

    return resolve


def _stub_resolve_transaction(conn, profile_id, ref):
    if not ref:
        from kassiber.errors import AppError

        raise AppError("transaction ref required", code="validation")
    row = conn.execute(
        "SELECT * FROM transactions WHERE profile_id = ? AND id = ?",
        (profile_id, ref),
    ).fetchone()
    if row is None:
        row = conn.execute(
            "SELECT * FROM transactions WHERE profile_id = ? AND external_id = ?",
            (profile_id, ref),
        ).fetchone()
    if row is None:
        from kassiber.errors import AppError

        raise AppError(f"transaction '{ref}' not found", code="not_found")
    return row


def _stub_format_table(headers, rows, widths, *, align_right=None):
    return [" ".join(headers)] + [" ".join(str(c) for c in row) for row in rows]


def _stub_write_text_pdf(path, title, lines):
    return {"file": path, "title": title, "line_count": len(lines)}


def _build_hooks(workspace_id: str, profile_id: str) -> SourceFundsHooks:
    return SourceFundsHooks(
        resolve_scope=_stub_resolve_scope(workspace_id, profile_id),
        resolve_transaction=_stub_resolve_transaction,
        write_text_pdf=_stub_write_text_pdf,
        format_table=_stub_format_table,
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

    def _add_inbound_tx(
        self,
        external_id: str,
        amount_msat: int,
        asset: str = "BTC",
        *,
        occurred_at: str = "2026-04-01T09:00:00Z",
    ) -> str:
        tx_id = str(uuid.uuid4())
        self.conn.execute(
            """
            INSERT INTO transactions(id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset, amount, fiat_currency, fiat_rate, fiat_value, raw_json, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                tx_id,
                self.workspace_id,
                self.profile_id,
                self.wallet_id,
                external_id,
                f"fp-{tx_id}",
                occurred_at,
                "inbound",
                asset,
                amount_msat,
                "EUR",
                50000.0,
                float(amount_msat) / 1e11 * 50000.0,
                "{}",
                occurred_at,
            ),
        )
        self.conn.commit()
        return tx_id

    def _add_outbound_tx(self, external_id: str, amount_msat: int, asset: str = "BTC") -> str:
        tx_id = str(uuid.uuid4())
        self.conn.execute(
            """
            INSERT INTO transactions(id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset, amount, fiat_currency, fiat_rate, fiat_value, raw_json, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
                "EUR",
                50000.0,
                float(amount_msat) / 1e11 * 50000.0,
                "{}",
                "2026-03-31T09:00:00Z",
            ),
        )
        self.conn.commit()
        return tx_id

    def _add_source(
        self,
        source_type: str,
        asset: str = "BTC",
        amount_msat: int | None = None,
        *,
        acquired_at: str | None = "2026-03-15T00:00:00Z",
    ) -> str:
        source_id = str(uuid.uuid4())
        # Attestation source types stay undated by convention.
        from kassiber.core.source_funds import ATTESTATION_SOURCE_TYPES
        effective_acquired = None if source_type in ATTESTATION_SOURCE_TYPES else acquired_at
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
                effective_acquired,
                None,
                "reviewed",
                "2026-03-10T09:00:00Z",
                "2026-03-10T09:00:00Z",
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
        allocation_policy: str = "explicit",
        uses_chain_observation: bool = False,
        chain_data_confirmed: bool = True,
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
                allocation_policy,
                "",
                1 if uses_chain_observation else 0,
                1 if chain_data_confirmed else 0,
                "2026-04-01T09:00:00Z",
                "2026-04-01T09:00:00Z",
            ),
        )
        self.conn.commit()
        return link_id

    def _classify(self, tx_id: str) -> str:
        return _classify_transaction(
            self.conn,
            self.workspace_id,
            self.profile_id,
            self._build_hooks(),
            self.profile_id,
            tx_id,
        )

    def _coverage(self) -> dict:
        return compute_coverage(self.conn, self.workspace_id, self.profile_id, self._build_hooks())

    def _build_hooks(self):
        return _build_hooks(self.workspace_id, self.profile_id)

    def test_no_links_yields_untraced(self):
        tx = self._add_inbound_tx("solo", 100_000)
        self.assertEqual(self._classify(tx), "untraced")

    def test_reviewed_to_real_source_is_fully_traced(self):
        tx = self._add_inbound_tx("real-src", 100_000)
        src = self._add_source("fiat_purchase", amount_msat=100_000)
        self._add_link(to_tx_id=tx, from_source_id=src, allocation_msat=100_000)
        self.assertEqual(self._classify(tx), "fully_traced")

    def test_reviewed_to_attestation_only_is_attested(self):
        tx = self._add_inbound_tx("attested", 100_000)
        src = self._add_source("missing_history")
        self._add_link(to_tx_id=tx, from_source_id=src, allocation_msat=100_000)
        self.assertEqual(self._classify(tx), "attested")

    def test_only_suggestions_is_in_review(self):
        tx = self._add_inbound_tx("pending", 100_000)
        src = self._add_source("fiat_purchase", amount_msat=100_000)
        self._add_link(to_tx_id=tx, from_source_id=src, state="suggested", allocation_msat=100_000)
        self.assertEqual(self._classify(tx), "in_review")

    def test_partial_reviewed_allocation_is_in_review_not_fully_traced(self):
        tx = self._add_inbound_tx("partial", 100_000)
        src = self._add_source("fiat_purchase", amount_msat=100_000)
        self._add_link(to_tx_id=tx, from_source_id=src, allocation_msat=1_000)
        self.assertEqual(self._classify(tx), "in_review")

    def test_overallocated_reviewed_link_is_in_review(self):
        tx = self._add_inbound_tx("over", 100_000)
        src = self._add_source("fiat_purchase", amount_msat=100_000)
        self._add_link(to_tx_id=tx, from_source_id=src, allocation_msat=200_000)
        self.assertEqual(self._classify(tx), "in_review")

    def test_two_partial_reviewed_links_summing_to_full_is_fully_traced(self):
        tx = self._add_inbound_tx("split", 100_000)
        src_a = self._add_source("fiat_purchase", amount_msat=60_000)
        src_b = self._add_source("exchange_withdrawal", amount_msat=40_000)
        self._add_link(to_tx_id=tx, from_source_id=src_a, allocation_msat=60_000)
        self._add_link(to_tx_id=tx, from_source_id=src_b, allocation_msat=40_000)
        self.assertEqual(self._classify(tx), "fully_traced")

    def test_mixed_real_and_attestation_is_attested(self):
        tx = self._add_inbound_tx("mixed", 100_000)
        attest = self._add_source("missing_history")
        real = self._add_source("fiat_purchase", amount_msat=50_000)
        self._add_link(to_tx_id=tx, from_source_id=attest, allocation_msat=50_000)
        self._add_link(to_tx_id=tx, from_source_id=real, allocation_msat=50_000)
        self.assertEqual(self._classify(tx), "attested")

    def test_walks_through_parent_transaction_to_root(self):
        target = self._add_inbound_tx("target", 100_000, occurred_at="2026-04-02T09:00:00Z")
        parent = self._add_inbound_tx("parent", 100_000, occurred_at="2026-04-01T09:00:00Z")
        src = self._add_source("fiat_purchase", amount_msat=100_000)
        self._add_link(to_tx_id=target, from_tx_id=parent, allocation_msat=100_000)
        self._add_link(to_tx_id=parent, from_source_id=src, allocation_msat=100_000)
        self.assertEqual(self._classify(target), "fully_traced")

    def test_parent_with_partial_coverage_propagates_in_review(self):
        target = self._add_inbound_tx("target", 100_000, occurred_at="2026-04-02T09:00:00Z")
        parent = self._add_inbound_tx("parent", 100_000, occurred_at="2026-04-01T09:00:00Z")
        src = self._add_source("fiat_purchase", amount_msat=100_000)
        self._add_link(to_tx_id=target, from_tx_id=parent, allocation_msat=100_000)
        # parent only partially covered upstream
        self._add_link(to_tx_id=parent, from_source_id=src, allocation_msat=10_000)
        self.assertEqual(self._classify(target), "in_review")

    def test_heuristic_allocation_policy_is_in_review(self):
        # Even with full coverage, allocation_policy != 'explicit' must
        # not classify as fully_traced. Coverage now mirrors the export
        # gate's ambiguous_allocation predicate.
        tx = self._add_inbound_tx("heuristic", 100_000)
        src = self._add_source("fiat_purchase", amount_msat=100_000)
        self._add_link(
            to_tx_id=tx,
            from_source_id=src,
            allocation_msat=100_000,
            allocation_policy="heuristic",
        )
        self.assertEqual(self._classify(tx), "in_review")

    def test_chain_observation_unconfirmed_is_in_review(self):
        target = self._add_inbound_tx("target", 100_000, occurred_at="2026-04-02T09:00:00Z")
        parent = self._add_inbound_tx("parent", 100_000, occurred_at="2026-04-01T09:00:00Z")
        src = self._add_source("fiat_purchase", amount_msat=100_000)
        self._add_link(
            to_tx_id=target,
            from_tx_id=parent,
            allocation_msat=100_000,
            uses_chain_observation=True,
            chain_data_confirmed=False,
        )
        self._add_link(to_tx_id=parent, from_source_id=src, allocation_msat=100_000)
        self.assertEqual(self._classify(target), "in_review")

    def test_unreviewed_suggestion_alongside_reviewed_is_in_review(self):
        tx = self._add_inbound_tx("with-suggestion", 100_000)
        src = self._add_source("fiat_purchase", amount_msat=100_000)
        self._add_link(to_tx_id=tx, from_source_id=src, allocation_msat=100_000)
        # Pending suggestion on the same target tx should block via unreviewed_link.
        another_src = self._add_source("exchange_withdrawal", amount_msat=100_000)
        self._add_link(
            to_tx_id=tx,
            from_source_id=another_src,
            allocation_msat=50_000,
            state="suggested",
        )
        self.assertEqual(self._classify(tx), "in_review")

    def test_chronology_violation_is_in_review(self):
        target = self._add_inbound_tx("target", 100_000, occurred_at="2026-04-01T09:00:00Z")
        # Parent dated AFTER target.
        parent = self._add_inbound_tx("future-parent", 100_000, occurred_at="2026-05-01T09:00:00Z")
        src = self._add_source("fiat_purchase", amount_msat=100_000)
        self._add_link(to_tx_id=target, from_tx_id=parent, allocation_msat=100_000)
        self._add_link(to_tx_id=parent, from_source_id=src, allocation_msat=100_000)
        self.assertEqual(self._classify(target), "in_review")

    def test_source_overallocation_is_in_review(self):
        tx = self._add_inbound_tx("over-source", 100_000)
        # Source has only 50k available, link claims 100k.
        src = self._add_source("fiat_purchase", amount_msat=50_000)
        self._add_link(to_tx_id=tx, from_source_id=src, allocation_msat=100_000)
        self.assertEqual(self._classify(tx), "in_review")

    def test_cycle_classifies_as_in_review_not_fully_traced(self):
        a = self._add_inbound_tx("a", 100_000)
        b = self._add_inbound_tx("b", 100_000)
        self._add_link(to_tx_id=a, from_tx_id=b, allocation_msat=100_000)
        self._add_link(to_tx_id=b, from_tx_id=a, allocation_msat=100_000)
        self.assertEqual(self._classify(a), "in_review")

    def test_compute_coverage_buckets_are_exhaustive(self):
        traced = self._add_inbound_tx("traced", 100_000)
        attested = self._add_inbound_tx("attested", 50_000)
        in_review = self._add_inbound_tx("in_review", 25_000)
        self._add_inbound_tx("untraced", 12_500)
        # Outbound should be ignored:
        self._add_outbound_tx("outbound-noise", 999_999)

        real = self._add_source("fiat_purchase", amount_msat=100_000)
        attest = self._add_source("opening_balance_attestation")
        self._add_link(to_tx_id=traced, from_source_id=real, allocation_msat=100_000)
        self._add_link(to_tx_id=attested, from_source_id=attest, allocation_msat=50_000)
        self._add_link(
            to_tx_id=in_review,
            from_source_id=real,
            state="suggested",
            allocation_msat=25_000,
        )

        coverage = self._coverage()
        totals = coverage["totals"]
        self.assertEqual(totals["tx_count"], 4)
        self.assertEqual(totals["amount_msat"], 100_000 + 50_000 + 25_000 + 12_500)
        bucket_amounts = {name: totals["buckets"][name]["amount_msat"] for name in COVERAGE_BUCKETS}
        self.assertEqual(bucket_amounts["fully_traced"], 100_000)
        self.assertEqual(bucket_amounts["attested"], 50_000)
        self.assertEqual(bucket_amounts["in_review"], 25_000)
        self.assertEqual(bucket_amounts["untraced"], 12_500)
        bucket_counts = {name: totals["buckets"][name]["tx_count"] for name in COVERAGE_BUCKETS}
        self.assertEqual(sum(bucket_counts.values()), totals["tx_count"])

    def test_compute_coverage_groups_by_wallet_and_asset(self):
        self._add_inbound_tx("btc-1", 100_000, asset="BTC")
        self._add_inbound_tx("lbtc-1", 50_000, asset="L-BTC")
        coverage = self._coverage()
        assets = {entry["asset"] for entry in coverage["by_asset"]}
        self.assertEqual(assets, {"BTC", "L-BTC"})
        wallet_assets = {(entry["wallet_label"], entry["asset"]) for entry in coverage["by_wallet"]}
        self.assertEqual(wallet_assets, {("Target", "BTC"), ("Target", "L-BTC")})

    def test_summary_text_emits_a_line_per_bucket(self):
        self._add_inbound_tx("solo", 100_000)
        coverage = self._coverage()
        lines = coverage_summary_text(coverage)
        for bucket in COVERAGE_BUCKETS:
            self.assertTrue(any(bucket in line for line in lines), f"missing bucket {bucket} in output")

    def test_default_max_depth_matches_build_report(self):
        """Coverage and build_report must agree on the default depth limit.

        If they disagree, a 9-16 hop chain shows fully_traced under
        coverage but path_truncated under build_report - false readiness.
        """
        from kassiber.core.source_funds_coverage import DEFAULT_MAX_DEPTH

        # build_report's default in source_funds.build_report is 8.
        self.assertEqual(DEFAULT_MAX_DEPTH, 8)

    def test_path_just_beyond_default_depth_classifies_as_in_review(self):
        """A chain longer than the default max-depth must classify as
        in_review (matches the path_truncated blocker the export gate
        would emit). Builds a 9-hop chain (target -> 8 parents) so depth
        crosses the limit before reaching a real source."""
        # Build chain target <- p1 <- p2 <- ... <- p9 <- src
        prev = self._add_inbound_tx("target-deep", 100_000, occurred_at="2026-04-10T09:00:00Z")
        target_id = prev
        for hop in range(1, 10):
            day = 10 - hop
            parent = self._add_inbound_tx(
                f"hop-{hop}",
                100_000,
                occurred_at=f"2026-04-{day:02d}T09:00:00Z",
            )
            self._add_link(to_tx_id=prev, from_tx_id=parent, allocation_msat=100_000)
            prev = parent
        # Real source at the top of the chain:
        src = self._add_source("fiat_purchase", amount_msat=100_000, acquired_at="2026-03-25T00:00:00Z")
        self._add_link(to_tx_id=prev, from_source_id=src, allocation_msat=100_000)
        # With default depth (8), the deepest hops cannot reach the source:
        self.assertEqual(self._classify(target_id), "in_review")

    def test_truncation_flags_when_inbound_count_exceeds_cap(self):
        """When a profile has more inbound rows than max_transactions,
        the coverage envelope must surface the truncation flag and the
        unclassified totals so the UI can prompt for explicit recompute."""
        for i in range(3):
            self._add_inbound_tx(f"tx-{i}", 100_000)
        coverage = compute_coverage(
            self.conn,
            self.workspace_id,
            self.profile_id,
            self._build_hooks(),
            max_transactions=2,
        )
        self.assertTrue(coverage["truncation"]["truncated"])
        self.assertEqual(coverage["truncation"]["inbound_total_count"], 3)
        self.assertEqual(coverage["truncation"]["not_classified_count"], 1)
        self.assertEqual(coverage["truncation"]["not_classified_msat"], 100_000)
        # Only the first two are classified; totals reflect that.
        self.assertEqual(coverage["totals"]["tx_count"], 2)

    def test_no_truncation_flag_when_under_cap(self):
        for i in range(2):
            self._add_inbound_tx(f"tx-{i}", 100_000)
        coverage = self._coverage()
        self.assertFalse(coverage["truncation"]["truncated"])
        self.assertEqual(coverage["truncation"]["not_classified_count"], 0)
        self.assertEqual(coverage["totals"]["tx_count"], 2)

    def test_summary_text_announces_truncation(self):
        for i in range(3):
            self._add_inbound_tx(f"tx-{i}", 100_000)
        coverage = compute_coverage(
            self.conn,
            self.workspace_id,
            self.profile_id,
            self._build_hooks(),
            max_transactions=1,
        )
        lines = coverage_summary_text(coverage)
        joined = "\n".join(lines)
        self.assertIn("truncated", joined.lower())


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
