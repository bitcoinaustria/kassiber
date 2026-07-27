import json
import sqlite3
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from kassiber import daemon
from kassiber.cli.main import build_parser, dispatch
from kassiber.core import freshness
from kassiber.core.ui_snapshot import (
    _load_swap_report_matcher_rows,
    build_report_blockers_snapshot,
)
from kassiber.db import open_db, set_setting
from kassiber.errors import AppError
from kassiber.msat import btc_to_msat


NOW = "2026-07-03T10:00:00Z"


def _seed_book(conn: sqlite3.Connection, *, tax_country: str = "at") -> None:
    conn.execute(
        "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
        ("ws", "Main", NOW),
    )
    conn.execute(
        """
        INSERT INTO profiles(
            id, workspace_id, label, fiat_currency, tax_country,
            tax_long_term_days, gains_algorithm, journal_input_version,
            last_processed_input_version, last_processed_at,
            last_processed_tx_count, created_at
        ) VALUES(?, ?, ?, ?, ?, 365, 'FIFO', 0, 0, ?, 0, ?)
        """,
        ("pf", "ws", "Main", "EUR", tax_country, NOW, NOW),
    )
    set_setting(conn, "context_workspace", "ws")
    set_setting(conn, "context_profile", "pf")


def _wallet(conn: sqlite3.Connection, wallet_id: str, label: str, kind: str) -> None:
    conn.execute(
        """
        INSERT INTO wallets(
            id, workspace_id, profile_id, label, kind, config_json, created_at
        ) VALUES(?, 'ws', 'pf', ?, ?, '{}', ?)
        """,
        (wallet_id, label, kind, NOW),
    )


def _tx(
    conn: sqlite3.Connection,
    tx_id: str,
    wallet_id: str,
    *,
    direction: str,
    asset: str = "BTC",
    amount_btc: str = "0.01",
    fee_btc: str = "0",
    external_id: str | None = None,
    occurred_at: str = NOW,
    payment_hash: str | None = None,
    payment_hash_source: str | None = None,
    raw_json: dict | None = None,
) -> None:
    amount = btc_to_msat(amount_btc)
    fee = btc_to_msat(fee_btc)
    fiat_rate = 50_000.0
    fiat_value = float(amount) / 100_000_000_000 * fiat_rate
    conn.execute(
        """
        INSERT INTO transactions(
            id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
            occurred_at, confirmed_at, direction, asset, amount, fee,
            fiat_currency, fiat_rate, fiat_value, fiat_price_source,
            kind, description, counterparty, note, excluded, raw_json,
            payment_hash, payment_hash_source, created_at
        ) VALUES(?, 'ws', 'pf', ?, ?, ?, ?, ?, ?, ?, ?, ?, 'EUR', ?, ?, 'manual',
                 'transfer', '', '', '', 0, ?, ?, ?, ?)
        """,
        (
            tx_id,
            wallet_id,
            external_id or tx_id,
            f"{tx_id}-fingerprint",
            occurred_at,
            occurred_at,
            direction,
            asset,
            amount,
            fee,
            fiat_rate,
            fiat_value,
            json.dumps(raw_json or {}, sort_keys=True),
            payment_hash,
            payment_hash_source if payment_hash else None,
            NOW,
        ),
    )


def _mark_processed(conn: sqlite3.Connection) -> None:
    count = conn.execute(
        "SELECT COUNT(*) AS count FROM transactions WHERE profile_id = 'pf' AND excluded = 0"
    ).fetchone()["count"]
    conn.execute(
        """
        UPDATE profiles
        SET last_processed_at = ?, last_processed_tx_count = ?,
            journal_input_version = 0, last_processed_input_version = 0
        WHERE id = 'pf'
        """,
        (NOW, int(count or 0)),
    )
    conn.commit()


class SwapCandidateReportBlockerTests(unittest.TestCase):
    def _with_conn(self):
        tmp = tempfile.TemporaryDirectory(prefix="kassiber-swap-blocker-")
        self.addCleanup(tmp.cleanup)
        return open_db(Path(tmp.name) / "data")

    def test_report_matcher_rows_include_wallet_network_config(self):
        conn = self._with_conn()
        try:
            _seed_book(conn)
            _wallet(conn, "node", "Regtest node", "lnd")
            conn.execute(
                "UPDATE wallets SET config_json = ? WHERE id = 'node'",
                (json.dumps({"chain": "lightning", "network": "regtest"}),),
            )
            _tx(conn, "payment", "node", direction="outbound")

            rows = _load_swap_report_matcher_rows(conn, "pf")

            self.assertEqual(
                json.loads(rows[0]["config_json"]),
                {"chain": "lightning", "network": "regtest"},
            )
        finally:
            conn.close()

    def test_header_only_active_custody_component_blocks_reports(self):
        conn = self._with_conn()
        try:
            _seed_book(conn, tax_country="generic")
            conn.execute(
                """
                INSERT INTO custody_components(
                    id, lineage_id, workspace_id, profile_id, revision,
                    component_type, conservation_mode, state, activated_at,
                    created_at
                ) VALUES(
                    'partial-component', 'partial-lineage', 'ws', 'pf', 1,
                    'manual_bridge', 'quantity', 'active', ?, ?
                )
                """,
                (NOW, NOW),
            )
            conn.commit()

            snapshot = build_report_blockers_snapshot(conn)

            blocker = next(
                item
                for item in snapshot["blockers"]
                if item["id"] == "custody_component_integrity"
            )
            self.assertEqual(blocker["severity"], "blocking")
            self.assertEqual(
                blocker["components"][0]["id"], "partial-component"
            )
            self.assertEqual(blocker["components"][0]["known_anchor_count"], 0)
            self.assertIn(
                "no_legs", blocker["components"][0]["issue_codes"]
            )
        finally:
            conn.close()

    def test_persisted_freshness_failure_blocks_final_exports_without_leaking_error_text(self):
        tmp = tempfile.TemporaryDirectory(prefix="kassiber-freshness-blocker-")
        self.addCleanup(tmp.cleanup)
        data_root = Path(tmp.name) / "data"
        conn = open_db(data_root)
        self.addCleanup(conn.close)
        _seed_book(conn)
        freshness.upsert_source_state(
            conn,
            profile_id="pf",
            source_key="onchain:wallet",
            source_type=freshness.SOURCE_ONCHAIN,
            source_label="Bull-Onchain",
            status="failed",
            stale_reason="observer_projection_conflict",
            blocking_reports=True,
            last_error_code="observer_projection_conflict",
            last_error_message="secret backend https://node.invalid/token",
            progress={"response_preview": "private progress"},
            checkpoint={"descriptor": "private checkpoint"},
        )
        conn.commit()

        snapshot = build_report_blockers_snapshot(conn)

        blocker = next(
            item for item in snapshot["blockers"] if item["id"] == "sync_failed"
        )
        self.assertFalse(snapshot["ready"])
        self.assertEqual(blocker["daemon_kind"], "ui.freshness.run")
        self.assertEqual(blocker["counts"], {"sources": 1})
        self.assertEqual(
            blocker["sources"],
            [
                {
                    "source_type": freshness.SOURCE_ONCHAIN,
                    "source_label": "Bull-Onchain",
                    "status": freshness.STATUS_BLOCKING_REPORTS,
                    "stale_reason": "observer_projection_conflict",
                    "last_error_code": "observer_projection_conflict",
                }
            ],
        )
        encoded = json.dumps(snapshot, sort_keys=True)
        self.assertNotIn("node.invalid", encoded)
        self.assertNotIn("private progress", encoded)
        self.assertNotIn("private checkpoint", encoded)

        with self.assertRaises(AppError) as raised:
            freshness.require_report_freshness(conn, "pf")
        self.assertEqual(raised.exception.code, "report_freshness_blocked")
        self.assertFalse(raised.exception.retryable)
        self.assertNotIn("node.invalid", json.dumps(raised.exception.details))

        managed_path = Path(tmp.name) / "should-not-exist.pdf"
        with (
            mock.patch(
                "kassiber.daemon._managed_report_export_path",
                return_value=managed_path,
            ) as path_mock,
            mock.patch(
                "kassiber.daemon.core_reports.export_pdf_report"
            ) as report_export,
        ):
            with self.assertRaises(AppError) as daemon_error:
                daemon._ui_report_export_payload_from_conn(
                    conn,
                    str(data_root),
                    "ui.reports.export_pdf",
                    {},
                )
        self.assertEqual(daemon_error.exception.code, "report_freshness_blocked")
        path_mock.assert_not_called()
        report_export.assert_not_called()
        self.assertFalse(managed_path.exists())

        cli_path = Path(tmp.name) / "cli-should-not-exist.pdf"
        cli_args = build_parser().parse_args(
            [
                "reports",
                "export-pdf",
                "--workspace",
                "Main",
                "--profile",
                "Main",
                "--file",
                str(cli_path),
            ]
        )
        with mock.patch(
            "kassiber.cli.main.core_reports.export_pdf_report"
        ) as cli_export:
            with self.assertRaises(AppError) as cli_error:
                dispatch(conn, cli_args)
        self.assertEqual(cli_error.exception.code, "report_freshness_blocked")
        cli_export.assert_not_called()
        self.assertFalse(cli_path.exists())

        raw_path = Path(tmp.name) / "transactions.csv"
        with (
            mock.patch(
                "kassiber.daemon._managed_report_export_path",
                return_value=raw_path,
            ),
            mock.patch(
                "kassiber.daemon.core_reports.export_transactions_csv_report",
                return_value={"file": str(raw_path), "rows": 0},
            ) as transaction_export,
        ):
            raw_payload = daemon._ui_report_export_payload_from_conn(
                conn,
                str(data_root),
                "ui.transactions.export_csv",
                {},
            )
        self.assertEqual(raw_payload["scope"], "transactions")
        transaction_export.assert_called_once()

        audit_path = Path(tmp.name) / "audit-package"
        with (
            mock.patch(
                "kassiber.daemon._managed_report_export_path",
                return_value=audit_path,
            ),
            mock.patch(
                "kassiber.daemon.core_audit_package.export_audit_package",
                return_value={"directory": str(audit_path)},
            ) as audit_export,
        ):
            audit_payload = daemon._ui_report_export_payload_from_conn(
                conn,
                str(data_root),
                "ui.reports.export_audit_package",
                {},
            )
        self.assertEqual(audit_payload["directory"], str(audit_path))
        audit_export.assert_called_once()

        freshness.upsert_source_state(
            conn,
            profile_id="pf",
            source_key="onchain:wallet",
            source_type=freshness.SOURCE_ONCHAIN,
            source_label="Bull-Onchain",
            status=freshness.STATUS_FRESH,
            blocking_reports=False,
        )
        conn.commit()
        refreshed = build_report_blockers_snapshot(conn)
        self.assertNotIn(
            "sync_failed",
            [item["id"] for item in refreshed["blockers"]],
        )
        freshness.require_report_freshness(conn, "pf")

    def test_route_only_provider_candidate_stays_strong_and_blocks_reports(self):
        conn = self._with_conn()
        try:
            _seed_book(conn)
            _wallet(conn, "btc", "Bull Bitcoin", "bullbitcoin")
            _wallet(conn, "liquid", "Bull Liquid", "bullbitcoin")
            raw = {
                "source": "bullbitcoin_wallet_csv",
                "type": "chain_swap",
                "status": "completed",
                "swap_id": "swap-chain",
                "send_txid": "bull-chain-send",
                "receive_txid": "bull-chain-recv",
            }
            _tx(
                conn,
                "out",
                "btc",
                direction="outbound",
                asset="BTC",
                amount_btc="0.01000000",
                fee_btc="0.00000500",
                external_id="bull-chain-send",
                raw_json=raw,
            )
            _tx(
                conn,
                "in",
                "liquid",
                direction="inbound",
                asset="LBTC",
                amount_btc="0.00990000",
                external_id="bull-chain-recv",
                raw_json=raw,
            )
            _mark_processed(conn)

            payload = build_report_blockers_snapshot(conn)

            blocker = next(item for item in payload["blockers"] if item["id"] == "unreviewed_swap_candidates")
            self.assertFalse(payload["ready"])
            self.assertEqual(blocker["counts"], {"total": 1, "exact": 0, "strong": 1})
            self.assertEqual(blocker["routes"][0]["method"], "provider_swap_id")
            self.assertEqual(blocker["routes"][0]["default_kind"], "chain-swap")
            self.assertNotIn("swap-chain", json.dumps(blocker))
        finally:
            conn.close()

    def test_ordinary_unmatched_outbound_does_not_block_reports(self):
        conn = self._with_conn()
        try:
            _seed_book(conn)
            _wallet(conn, "wallet", "Spending", "descriptor")
            _tx(conn, "payment", "wallet", direction="outbound", amount_btc="0.01000000")
            _mark_processed(conn)

            payload = build_report_blockers_snapshot(conn)

            self.assertNotIn("unreviewed_swap_candidates", [item["id"] for item in payload["blockers"]])
        finally:
            conn.close()

    def test_same_asset_manual_heuristic_candidate_does_not_block_reports(self):
        conn = self._with_conn()
        try:
            _seed_book(conn)
            _wallet(conn, "a", "Wallet A", "descriptor")
            _wallet(conn, "b", "Wallet B", "descriptor")
            _tx(conn, "out", "a", direction="outbound", amount_btc="0.01000000")
            _tx(
                conn,
                "in",
                "b",
                direction="inbound",
                amount_btc="0.00999000",
                occurred_at="2026-07-03T10:05:00Z",
            )
            _mark_processed(conn)

            payload = build_report_blockers_snapshot(conn)

            self.assertNotIn("unreviewed_swap_candidates", [item["id"] for item in payload["blockers"]])
        finally:
            conn.close()

    def test_cross_asset_strong_candidate_blocks_reports(self):
        conn = self._with_conn()
        try:
            _seed_book(conn)
            _wallet(conn, "btc", "Bitcoin", "descriptor")
            _wallet(conn, "liquid", "Liquid", "descriptor")
            _tx(conn, "out", "btc", direction="outbound", asset="BTC", amount_btc="0.01000000")
            _tx(
                conn,
                "in",
                "liquid",
                direction="inbound",
                asset="LBTC",
                amount_btc="0.00999000",
                occurred_at="2026-07-03T10:05:00Z",
            )
            _mark_processed(conn)

            payload = build_report_blockers_snapshot(conn)

            blocker = next(item for item in payload["blockers"] if item["id"] == "unreviewed_swap_candidates")
            self.assertEqual(blocker["counts"], {"total": 1, "exact": 0, "strong": 1})
            self.assertEqual(blocker["routes"][0]["default_kind"], "peg-in")
        finally:
            conn.close()

    def test_chain_script_only_hash_still_blocks_but_is_not_exact(self):
        conn = self._with_conn()
        try:
            _seed_book(conn)
            _wallet(conn, "node", "Node", "lnd")
            _wallet(conn, "chain", "Chain", "descriptor")
            payment_hash = "ab" * 32
            _tx(
                conn,
                "out",
                "node",
                direction="outbound",
                amount_btc="0.01000000",
                payment_hash=payment_hash,
                payment_hash_source="chain_script",
            )
            _tx(
                conn,
                "in",
                "chain",
                direction="inbound",
                amount_btc="0.00999000",
                occurred_at="2026-07-03T10:05:00Z",
                payment_hash=payment_hash,
                payment_hash_source="chain_script",
            )
            _mark_processed(conn)

            payload = build_report_blockers_snapshot(conn)

            blocker = next(item for item in payload["blockers"] if item["id"] == "unreviewed_swap_candidates")
            self.assertEqual(blocker["counts"], {"total": 1, "exact": 0, "strong": 1})
            self.assertEqual(blocker["routes"][0]["method"], "heuristic")
        finally:
            conn.close()


if __name__ == "__main__":
    unittest.main()
