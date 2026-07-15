import tempfile
import unittest

from kassiber.core.ui_snapshot import build_overview_snapshot
from kassiber.db import open_db, set_setting
from kassiber.msat import btc_to_msat


class CustodyQuantityViewTests(unittest.TestCase):
    def test_views_use_observed_wallet_quantity_and_exclude_suspense(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-custody-view-") as root:
            conn = open_db(root)
            self.addCleanup(conn.close)
            now = "2026-01-04T00:00:00Z"
            conn.execute(
                "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Books', ?)",
                (now,),
            )
            conn.execute(
                """
                INSERT INTO profiles(
                    id, workspace_id, label, fiat_currency, gains_algorithm,
                    last_processed_at, last_processed_tx_count, created_at
                ) VALUES('profile', 'ws', 'Main', 'EUR', 'FIFO', ?, 3, ?)
                """,
                (now, now),
            )
            conn.executemany(
                """
                INSERT INTO wallets(
                    id, workspace_id, profile_id, label, kind, config_json,
                    created_at
                ) VALUES(?, 'ws', 'profile', ?, 'descriptor', '{}', ?)
                """,
                (("source", "Old vault", now), ("destination", "New vault", now)),
            )
            transactions = (
                ("fund", "source", "2026-01-01T00:00:00Z", "inbound", "10.0"),
                ("leave", "source", "2026-01-02T00:00:00Z", "outbound", "10.0"),
                ("return", "destination", "2026-01-03T00:00:00Z", "inbound", "9.9"),
            )
            conn.executemany(
                """
                INSERT INTO transactions(
                    id, workspace_id, profile_id, wallet_id, external_id,
                    fingerprint, occurred_at, direction, asset, amount, fee,
                    fiat_currency, fiat_rate, fiat_value, raw_json, created_at
                ) VALUES(?, 'ws', 'profile', ?, ?, ?, ?, ?, 'BTC', ?, 0,
                         'EUR', 50000, 500000, '{}', ?)
                """,
                [
                    (
                        tx_id,
                        wallet_id,
                        tx_id,
                        f"fp:{tx_id}",
                        occurred_at,
                        direction,
                        btc_to_msat(amount),
                        now,
                    )
                    for tx_id, wallet_id, occurred_at, direction, amount in transactions
                ],
            )
            observed = (
                ("observed:fund", "fund", "2026-01-01T00:00:00Z", "source", "10.0"),
                ("observed:leave", "leave", "2026-01-02T00:00:00Z", "source", "-10.0"),
                ("observed:return", "return", "2026-01-03T00:00:00Z", "destination", "9.9"),
            )
            conn.executemany(
                """
                INSERT INTO journal_quantity_postings(
                    posting_id, workspace_id, profile_id, transaction_id,
                    occurred_at, asset, location_kind, location_id,
                    amount_msat, state, created_at
                ) VALUES(?, 'ws', 'profile', ?, ?, 'BTC', 'wallet', ?, ?,
                         'observed', ?)
                """,
                [
                    (
                        posting_id,
                        tx_id,
                        occurred_at,
                        wallet_id,
                        btc_to_msat(amount),
                        now,
                    )
                    for posting_id, tx_id, occurred_at, wallet_id, amount in observed
                ],
            )
            conn.execute(
                """
                INSERT INTO journal_quantity_postings(
                    posting_id, workspace_id, profile_id, transaction_id,
                    occurred_at, asset, location_kind, location_id,
                    amount_msat, state, created_at
                ) VALUES('suspense:leave', 'ws', 'profile', 'leave',
                         '2026-01-02T00:00:00Z', 'BTC', 'custody_suspense',
                         'missing-wallet', ?, 'custody_suspense', ?)
                """,
                (btc_to_msat("0.1"), now),
            )
            conn.execute(
                """
                INSERT INTO journal_quantity_balances(
                    workspace_id, profile_id, location_kind, location_id,
                    asset, amount_msat, created_at
                ) VALUES('ws', 'profile', 'wallet', 'destination', 'BTC', ?, ?)
                """,
                (btc_to_msat("9.9"), now),
            )
            conn.execute(
                """
                INSERT INTO journal_quantity_issues(
                    issue_id, workspace_id, profile_id, issue_type, state,
                    asset, amount_msat, occurred_at, transaction_ids_json,
                    reason, detail_json, blocks_from, created_at
                ) VALUES('gap', 'ws', 'profile', 'unresolved_quantity',
                         'custody_suspense', 'BTC', ?, '2026-01-02T00:00:00Z',
                         '["leave"]', 'missing_wallet', '{}',
                         '2026-01-02T00:00:00Z', ?)
                """,
                (btc_to_msat("0.1"), now),
            )
            conn.execute(
                """
                INSERT INTO rates_cache(pair, timestamp, rate, source, fetched_at)
                VALUES('BTC-EUR', '2026-01-03T00:00:00Z', 50000, 'manual', ?)
                """,
                (now,),
            )
            set_setting(conn, "context_workspace", "ws")
            set_setting(conn, "context_profile", "profile")
            conn.commit()

            overview = build_overview_snapshot(conn)

            balances = {
                item["label"]: item["balance"] for item in overview["connections"]
            }
            self.assertEqual(balances["Old vault"], 0)
            self.assertEqual(balances["New vault"], 9.9)
            activity = {item["id"]: item for item in overview["activityTxs"]}
            self.assertEqual(activity["fund"]["balanceBtc"], 10.0)
            self.assertEqual(activity["leave"]["balanceBtc"], 0.0)
            self.assertEqual(activity["return"]["balanceBtc"], 9.9)
            self.assertEqual(overview["balanceSeries"][-1], 9.9)
            self.assertEqual(overview["portfolioSeries"][-1]["balanceBtc"], 9.9)


if __name__ == "__main__":
    unittest.main()
