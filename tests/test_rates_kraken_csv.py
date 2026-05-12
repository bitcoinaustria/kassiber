import json
import sqlite3
import subprocess
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import patch

from kassiber.core import rates as core_rates
from kassiber.core.rates import get_cached_rate_at_or_before
from kassiber.daemon import _rates_kraken_csv_import_payload
from kassiber.db import open_db


ROOT = Path(__file__).resolve().parent.parent
FIXTURES = Path(__file__).resolve().parent / "fixtures"
XBTEUR_FIXTURE = FIXTURES / "XBTEUR_1.csv"

XBTUSD_CSV = """1714521600,65000.00,65010.00,64990.00,65005.50,0.5000,10
1714521660,65005.50,65020.00,65000.00,65012.25,0.2500,4
"""


class KrakenCsvRatesTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="kassiber-kraken-rates-")
        self.tmp_path = Path(self._tmp.name)
        self.data_root = self.tmp_path / "data"

    def tearDown(self):
        self._tmp.cleanup()

    def _run_json(self, *args):
        cmd = [
            sys.executable,
            "-m",
            "kassiber",
            "--data-root",
            str(self.data_root),
            "--machine",
            *args,
        ]
        result = subprocess.run(
            cmd,
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        stdout = result.stdout.strip()
        self.assertTrue(stdout, msg=f"No stdout for {args!r}; stderr={result.stderr!r}")
        payload = json.loads(stdout)
        self.assertEqual(result.returncode, 0, msg=f"{payload!r}; stderr={result.stderr!r}")
        self.assertEqual(payload.get("schema_version"), 1)
        return payload

    def _connect(self):
        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        self.addCleanup(conn.close)
        return conn

    def _seed_transaction_needing_rate(
        self,
        conn,
        *,
        profile_fiat="EUR",
        tx_fiat=None,
        occurred_at="2024-05-01T12:34:56Z",
        confirmed_at=None,
        asset="BTC",
        external_id="needs-rate",
    ):
        created_at = "2024-05-01T00:00:00Z"
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES (?, ?, ?)",
            ("workspace-1", "Main", created_at),
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, created_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "profile-1",
                "workspace-1",
                "Default",
                profile_fiat,
                "generic",
                365,
                "FIFO",
                created_at,
            ),
        )
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "wallet-1",
                "workspace-1",
                "profile-1",
                "Cold",
                "manual",
                "{}",
                created_at,
            ),
        )
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, confirmed_at, direction, asset, amount, fee,
                fiat_currency, raw_json, created_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "tx-1",
                "workspace-1",
                "profile-1",
                "wallet-1",
                external_id,
                f"fingerprint-{external_id}",
                occurred_at,
                confirmed_at,
                "in",
                asset,
                100_000_000,
                0,
                tx_fiat,
                "{}",
                created_at,
            ),
        )
        conn.commit()

    def test_ingests_kraken_csv_and_stores_ohlcvt(self):
        payload = self._run_json(
            "rates",
            "sync",
            "--source",
            "kraken-csv",
            "--path",
            str(XBTEUR_FIXTURE),
        )

        self.assertEqual(payload["kind"], "rates.sync")
        self.assertEqual(len(payload["data"]), 1)
        summary = payload["data"][0]
        self.assertEqual(summary["pair"], "BTC-EUR")
        self.assertEqual(summary["source"], "kraken-csv")
        self.assertEqual(summary["samples"], 10)
        self.assertEqual(summary["first_timestamp"], "2024-05-01T00:01:00Z")
        self.assertEqual(summary["last_timestamp"], "2024-05-01T00:10:00Z")

        conn = self._connect()
        row = conn.execute(
            """
            SELECT pair, timestamp, rate_exact, source, granularity, method,
                   open_rate_exact, high_rate_exact, low_rate_exact,
                   close_rate_exact, volume_exact, trades
            FROM rates_cache
            WHERE timestamp = ?
            """,
            ("2024-05-01T00:04:00Z",),
        ).fetchone()
        self.assertEqual(row["pair"], "BTC-EUR")
        self.assertEqual(row["source"], "kraken-csv")
        self.assertEqual(row["granularity"], "minute")
        self.assertEqual(row["method"], "ohlcvt_csv")
        self.assertEqual(row["rate_exact"], "60018.75")
        self.assertEqual(row["open_rate_exact"], "60020.00")
        self.assertEqual(row["high_rate_exact"], "60020.00")
        self.assertEqual(row["low_rate_exact"], "60018.75")
        self.assertEqual(row["close_rate_exact"], "60018.75")
        self.assertEqual(row["volume_exact"], "0")
        self.assertEqual(row["trades"], 0)

        cached = get_cached_rate_at_or_before(
            conn,
            "BTC-EUR",
            "2024-05-01T00:03:30Z",
        )
        self.assertEqual(cached["timestamp"], "2024-05-01T00:03:00Z")
        self.assertEqual(cached["rate_exact"], "60020.00")

    def test_reingest_is_idempotent(self):
        for _ in range(2):
            self._run_json(
                "rates",
                "sync",
                "--source",
                "kraken-csv",
                "--path",
                str(XBTEUR_FIXTURE),
            )

        conn = self._connect()
        count = conn.execute(
            "SELECT COUNT(*) FROM rates_cache WHERE source = 'kraken-csv'"
        ).fetchone()[0]
        self.assertEqual(count, 10)

    def test_desktop_daemon_import_payload_supports_incremental_archives(self):
        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)

        payload = _rates_kraken_csv_import_payload(
            conn,
            {"path": str(XBTEUR_FIXTURE), "operation": "incremental"},
        )
        self.assertEqual(payload["source"], "kraken-csv")
        self.assertEqual(payload["operation"], "incremental")
        self.assertEqual(payload["totals"]["pairs"], 1)
        self.assertEqual(payload["totals"]["samples"], 10)
        self.assertEqual(payload["summary"][0]["pair"], "BTC-EUR")

        _rates_kraken_csv_import_payload(
            conn,
            {"path": str(XBTEUR_FIXTURE), "operation": "incremental"},
        )
        count = conn.execute(
            "SELECT COUNT(*) FROM rates_cache WHERE source = 'kraken-csv'"
        ).fetchone()[0]
        self.assertEqual(count, 10)

    def test_zip_pair_filter_ingests_only_matching_pair(self):
        archive = self.tmp_path / "Kraken_OHLCVT.zip"
        with zipfile.ZipFile(archive, "w") as zf:
            zf.writestr("XBTEUR_1.csv", XBTEUR_FIXTURE.read_text(encoding="utf-8"))
            zf.writestr("XBTUSD_1.csv", XBTUSD_CSV)
            zf.writestr("XBTEUR_5.csv", "1714521600,1,1,1,1,1,1\n")

        payload = self._run_json(
            "rates",
            "sync",
            "--source",
            "kraken-csv",
            "--path",
            str(archive),
            "--pair",
            "BTC/EUR",
        )

        self.assertEqual(len(payload["data"]), 1)
        self.assertEqual(payload["data"][0]["pair"], "BTC-EUR")
        conn = self._connect()
        pairs = [
            row["pair"]
            for row in conn.execute(
                "SELECT DISTINCT pair FROM rates_cache ORDER BY pair"
            ).fetchall()
        ]
        self.assertEqual(pairs, ["BTC-EUR"])
        count = conn.execute("SELECT COUNT(*) FROM rates_cache").fetchone()[0]
        self.assertEqual(count, 10)

    def test_directory_pair_filter_ingests_extracted_archive(self):
        archive_dir = self.tmp_path / "master_q4"
        archive_dir.mkdir()
        (archive_dir / "XBTEUR_1.csv").write_text(
            XBTEUR_FIXTURE.read_text(encoding="utf-8"),
            encoding="utf-8",
        )
        (archive_dir / "XBTUSD_1.csv").write_text(XBTUSD_CSV, encoding="utf-8")
        (archive_dir / "BTCUSD_Daily_OHLC.csv").write_text(
            "time,open,high,low,close,volume\n",
            encoding="utf-8",
        )

        payload = self._run_json(
            "rates",
            "sync",
            "--source",
            "kraken-csv",
            "--path",
            str(archive_dir),
            "--pair",
            "BTC/EUR",
        )

        self.assertEqual(len(payload["data"]), 1)
        self.assertEqual(payload["data"][0]["pair"], "BTC-EUR")
        conn = self._connect()
        pairs = [
            row["pair"]
            for row in conn.execute(
                "SELECT DISTINCT pair FROM rates_cache ORDER BY pair"
            ).fetchall()
        ]
        self.assertEqual(pairs, ["BTC-EUR"])

    def test_default_sync_uses_coinbase_exchange_and_maps_lhoc_tuple(self):
        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)

        def fake_coinbase_rows(pair, start, end, granularity=60):
            self.assertEqual(pair, "BTC-EUR")
            self.assertEqual(granularity, 60)
            return [
                [
                    1714521660,
                    "60001.00",
                    "60040.00",
                    "60010.00",
                    "60030.00",
                    "0.75",
                ],
                [
                    1714521600,
                    "59990.00",
                    "60020.00",
                    "60000.00",
                    "60010.00",
                    "0.50",
                ],
            ]

        with patch.object(
            core_rates,
            "_coinbase_exchange_candles",
            side_effect=fake_coinbase_rows,
        ):
            summary = core_rates.sync_rates(conn, pair="BTC-EUR", days=1)

        self.assertEqual(summary[0]["source"], "coinbase-exchange")
        self.assertEqual(summary[0]["method"], "product_candles")
        rows = conn.execute(
            """
            SELECT timestamp, open_rate_exact, high_rate_exact, low_rate_exact,
                   close_rate_exact, volume_exact, trades
            FROM rates_cache
            WHERE source = 'coinbase-exchange'
            ORDER BY timestamp
            """
        ).fetchall()
        self.assertEqual(
            [row["timestamp"] for row in rows],
            [
                "2024-05-01T00:01:00Z",
                "2024-05-01T00:02:00Z",
            ],
        )
        self.assertEqual(rows[0]["open_rate_exact"], "60000.00")
        self.assertEqual(rows[0]["high_rate_exact"], "60020.00")
        self.assertEqual(rows[0]["low_rate_exact"], "59990.00")
        self.assertEqual(rows[0]["close_rate_exact"], "60010.00")
        self.assertEqual(rows[0]["volume_exact"], "0.50")
        self.assertIsNone(rows[0]["trades"])

    def test_coinbase_exchange_fetch_windows_at_300_minutes(self):
        windows = []

        def fake_coinbase_rows(pair, start, end, granularity=60):
            windows.append((start, end))
            return []

        with patch.object(
            core_rates,
            "_coinbase_exchange_candles",
            side_effect=fake_coinbase_rows,
        ):
            core_rates.fetch_rates_coinbase_exchange("BTC-EUR", days=1, granularity=60)

        self.assertEqual(len(windows), 5)
        self.assertTrue(
            all((end - start).total_seconds() <= 300 * 60 for start, end in windows)
        )

    def test_coinbase_sync_fetches_only_missing_transaction_window(self):
        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)
        self._seed_transaction_needing_rate(conn)
        windows = []

        def fake_coinbase_rows(pair, start, end, granularity=60):
            windows.append((pair, start, end, granularity))
            return [
                [
                    1714566780,
                    "59990.00",
                    "60020.00",
                    "60000.00",
                    "60010.00",
                    "0.50",
                ],
            ]

        with patch.object(
            core_rates,
            "_coinbase_exchange_candles",
            side_effect=fake_coinbase_rows,
        ):
            summary = core_rates.sync_rates(conn, days=1)

        eur_summary = next(row for row in summary if row["pair"] == "BTC-EUR")
        usd_summary = next(row for row in summary if row["pair"] == "BTC-USD")
        self.assertEqual(eur_summary["mode"], "transaction_need")
        self.assertEqual(eur_summary["needed_minutes"], 1)
        self.assertEqual(eur_summary["missing_minutes"], 1)
        self.assertEqual(eur_summary["windows"], 1)
        self.assertEqual(eur_summary["checked_minutes"], 300)
        self.assertEqual(usd_summary["mode"], "transaction_need")
        self.assertEqual(usd_summary["needed_minutes"], 0)
        self.assertEqual(usd_summary["windows"], 0)
        self.assertEqual(len(windows), 1)
        pair, start, end, granularity = windows[0]
        self.assertEqual(pair, "BTC-EUR")
        self.assertEqual(granularity, 60)
        self.assertEqual(start.isoformat(), "2024-05-01T12:33:00+00:00")
        self.assertEqual(end.isoformat(), "2024-05-01T17:33:00+00:00")

        rate_row = conn.execute(
            """
            SELECT timestamp, rate_exact
            FROM rates_cache
            WHERE pair = 'BTC-EUR' AND source = 'coinbase-exchange'
            """
        ).fetchone()
        self.assertEqual(rate_row["timestamp"], "2024-05-01T12:34:00Z")
        self.assertEqual(rate_row["rate_exact"], "60010.00")

    def test_coinbase_sync_does_not_refetch_checked_sparse_minute(self):
        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)
        self._seed_transaction_needing_rate(conn)
        calls = []

        def fake_coinbase_rows(pair, start, end, granularity=60):
            calls.append((pair, start, end, granularity))
            return []

        with patch.object(
            core_rates,
            "_coinbase_exchange_candles",
            side_effect=fake_coinbase_rows,
        ):
            first = core_rates.sync_rates(conn, pair="BTC-EUR", days=1)
            second = core_rates.sync_rates(conn, pair="BTC-EUR", days=1)

        self.assertEqual(len(calls), 1)
        self.assertEqual(first[0]["missing_minutes"], 1)
        self.assertEqual(first[0]["checked_minutes"], 300)
        self.assertEqual(second[0]["needed_minutes"], 1)
        self.assertEqual(second[0]["already_checked_minutes"], 1)
        self.assertEqual(second[0]["missing_minutes"], 0)
        self.assertEqual(second[0]["windows"], 0)

        checked_count = conn.execute(
            """
            SELECT COUNT(*)
            FROM rates_checked_minutes
            WHERE pair = 'BTC-EUR' AND source = 'coinbase-exchange'
            """
        ).fetchone()[0]
        self.assertEqual(checked_count, 300)

    def test_coinbase_sync_skips_transaction_minute_already_cached(self):
        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)
        self._seed_transaction_needing_rate(conn)
        core_rates.upsert_rate(
            conn,
            "BTC-EUR",
            "2024-05-01T12:34:00Z",
            "60000.00",
            core_rates.RATE_SOURCE_KRAKEN_CSV,
            fetched_at="2024-05-01T00:00:00Z",
            granularity="minute",
            method="ohlcvt_csv",
        )
        conn.commit()

        with patch.object(core_rates, "_coinbase_exchange_candles") as fetch:
            summary = core_rates.sync_rates(conn, pair="BTC-EUR", days=1)

        fetch.assert_not_called()
        self.assertEqual(summary[0]["needed_minutes"], 1)
        self.assertEqual(summary[0]["cached_minutes"], 1)
        self.assertEqual(summary[0]["missing_minutes"], 0)
        self.assertEqual(summary[0]["windows"], 0)


if __name__ == "__main__":
    unittest.main()
