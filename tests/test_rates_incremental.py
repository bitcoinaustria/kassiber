import sqlite3
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path
from unittest.mock import patch

from kassiber.core import rates
from kassiber.db import open_db


def _seed_priced_scope(conn: sqlite3.Connection) -> None:
    conn.execute(
        "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Main', '2026-06-04T00:00:00Z')"
    )
    conn.execute(
        """
        INSERT INTO profiles(id, workspace_id, label, fiat_currency, created_at)
        VALUES('profile', 'ws', 'Book', 'EUR', '2026-06-04T00:00:00Z')
        """
    )
    conn.execute(
        """
        INSERT INTO wallets(id, workspace_id, profile_id, label, kind, config_json, created_at)
        VALUES('wallet', 'ws', 'profile', 'Cold', 'address', '{}', '2026-06-04T00:00:00Z')
        """
    )
    conn.execute(
        """
        INSERT INTO transactions(
            id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
            occurred_at, confirmed_at, direction, asset, amount, fee, fiat_currency,
            kind, description, created_at
        ) VALUES(
            'tx', 'ws', 'profile', 'wallet', 'external', 'fingerprint',
            '2026-01-01T12:34:56Z', '2026-01-01T12:34:56Z',
            'inbound', 'BTC', 100000000000, 0, 'EUR',
            'deposit', 'missing price', '2026-06-04T00:00:00Z'
        )
        """
    )
    conn.commit()


class RatesIncrementalTest(unittest.TestCase):
    def test_second_coinbase_sync_skips_existing_missing_minute(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-rates-incremental-") as tmp:
            conn = open_db(Path(tmp) / "data")
            self.addCleanup(conn.close)
            _seed_priced_scope(conn)
            calls = []

            def fake_candles(pair, start, end, granularity=60):
                calls.append((pair, start, end, granularity))
                candle_open = int(
                    datetime(2026, 1, 1, 12, 33, 0, tzinfo=timezone.utc).timestamp()
                )
                return [
                    [candle_open, "100", "110", "90", "105", "1", 1],
                ]

            with patch("kassiber.core.rates._coinbase_exchange_candles", fake_candles):
                first = rates.sync_rates(conn, pair="BTC-EUR", days=1)
                second = rates.sync_rates(conn, pair="BTC-EUR", days=1)

            self.assertEqual(len(calls), 1)
            self.assertEqual(first[0]["missing_minutes"], 1)
            self.assertEqual(second[0]["missing_minutes"], 0)
            self.assertEqual(second[0]["cached_minutes"], 1)


if __name__ == "__main__":
    unittest.main()
