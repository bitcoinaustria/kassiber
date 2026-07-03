import json
import sqlite3
import tempfile
import unittest
from pathlib import Path

from kassiber.core.ui_snapshot import build_report_blockers_snapshot
from kassiber.db import open_db, set_setting
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
            "importer" if payment_hash else None,
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

    def test_provider_evidence_swap_candidate_blocks_reports(self):
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
            self.assertEqual(blocker["counts"], {"total": 1, "exact": 1, "strong": 0})
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


if __name__ == "__main__":
    unittest.main()
