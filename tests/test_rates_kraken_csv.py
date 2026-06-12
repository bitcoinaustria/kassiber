import json
import sqlite3
import subprocess
import sys
import tempfile
import unittest
import zipfile
from datetime import datetime, timezone
from pathlib import Path
from unittest.mock import patch

from kassiber.backends import create_db_backend
from kassiber.core import rates as core_rates
from kassiber.core import pricing
from kassiber.core.rates import get_cached_rate_at_or_before
from kassiber.core.ui_snapshot import build_transactions_snapshot
from kassiber.daemon import (
    _rates_kraken_csv_import_payload,
    _rates_latest_payload,
    _rates_rebuild_payload,
)
from kassiber.db import open_db, set_setting
from kassiber.errors import AppError


ROOT = Path(__file__).resolve().parent.parent
FIXTURES = Path(__file__).resolve().parent / "fixtures"
XBTEUR_FIXTURE = FIXTURES / "XBTEUR_1.csv"
BUNDLED_KRAKEN_BTC_DAILY = (
    ROOT / "kassiber" / "data" / "rates" / "kraken" / "btc_daily"
)

XBTUSD_CSV = """1714521600,65000.00,65010.00,64990.00,65005.50,0.5000,10
1714521660,65005.50,65020.00,65000.00,65012.25,0.2500,4
"""

XBTUSD_DAILY_CSV = """1714521600,65000.00,65100.00,64900.00,65050.00,1.5000,20
"""


class _MemoryResourceFile:
    def __init__(self, name, content):
        self.name = name
        self._content = content

    def is_file(self):
        return True

    def read_bytes(self):
        return self._content


class _MemoryResourceDir:
    def __init__(self, children):
        self._children = children

    def iterdir(self):
        return iter(self._children)

    def __str__(self):
        return "memory://kassiber/data/rates/kraken/btc_daily"


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

    def _seed_mempool_backend(self, conn):
        create_db_backend(
            conn,
            "own-mempool",
            "mempool",
            "http://127.0.0.1:3006/api",
            chain="bitcoin",
            network="main",
            timeout=17,
            tor_proxy="socks5h://127.0.0.1:9050",
            commit=False,
        )
        set_setting(conn, "default_backend", "own-mempool")
        conn.commit()

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
        self.assertEqual(payload["data"][0]["skipped_files"], 2)
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

    def test_daily_kraken_csv_ingests_with_daily_granularity(self):
        archive_dir = self.tmp_path / "btc_daily"
        archive_dir.mkdir()
        (archive_dir / "XBTUSD_1440.csv").write_text(
            XBTUSD_DAILY_CSV,
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
            "BTC/USD",
        )

        self.assertEqual(len(payload["data"]), 1)
        summary = payload["data"][0]
        self.assertEqual(summary["pair"], "BTC-USD")
        self.assertEqual(summary["samples"], 1)
        self.assertEqual(summary["granularity"], "daily")
        self.assertEqual(summary["first_timestamp"], "2024-05-02T00:00:00Z")

        conn = self._connect()
        row = conn.execute(
            """
            SELECT timestamp, rate_exact, granularity, open_rate_exact,
                   high_rate_exact, low_rate_exact, close_rate_exact,
                   volume_exact, trades
            FROM rates_cache
            WHERE pair = 'BTC-USD'
            """,
        ).fetchone()
        self.assertEqual(row["timestamp"], "2024-05-02T00:00:00Z")
        self.assertEqual(row["rate_exact"], "65050.00")
        self.assertEqual(row["granularity"], "daily")
        self.assertEqual(row["open_rate_exact"], "65000.00")
        self.assertEqual(row["high_rate_exact"], "65100.00")
        self.assertEqual(row["low_rate_exact"], "64900.00")
        self.assertEqual(row["close_rate_exact"], "65050.00")
        self.assertEqual(row["volume_exact"], "1.5000")
        self.assertEqual(row["trades"], 20)
        self.assertIsNone(
            get_cached_rate_at_or_before(
                conn,
                "BTC-USD",
                "2024-05-01T12:00:00Z",
            )
        )
        cached_rate = get_cached_rate_at_or_before(
            conn,
            "BTC-USD",
            "2024-05-02T12:00:00Z",
        )
        self.assertIsNotNone(cached_rate)
        self.assertEqual(cached_rate["timestamp"], "2024-05-02T00:00:00Z")
        self.assertEqual(cached_rate["rate_exact"], "65050.00")

    def test_kraken_zip_prefers_minute_candles_when_daily_is_also_present(self):
        archive = self.tmp_path / "Kraken_OHLCVT.zip"
        with zipfile.ZipFile(archive, "w") as zf:
            zf.writestr("XBTUSD_1.csv", XBTUSD_CSV)
            zf.writestr("XBTUSD_1440.csv", XBTUSD_DAILY_CSV)

        payload = self._run_json(
            "rates",
            "sync",
            "--source",
            "kraken-csv",
            "--path",
            str(archive),
            "--pair",
            "BTC/USD",
        )

        self.assertEqual(len(payload["data"]), 1)
        summary = payload["data"][0]
        self.assertEqual(summary["samples"], 2)
        self.assertEqual(summary["granularity"], "minute")
        self.assertEqual(summary["skipped_files"], 1)

        conn = self._connect()
        rows = conn.execute(
            """
            SELECT timestamp, granularity, rate_exact
            FROM rates_cache
            WHERE pair = 'BTC-USD'
            ORDER BY timestamp
            """
        ).fetchall()
        self.assertEqual(
            [
                (row["timestamp"], row["granularity"], row["rate_exact"])
                for row in rows
            ],
            [
                ("2024-05-01T00:01:00Z", "minute", "65005.50"),
                ("2024-05-01T00:02:00Z", "minute", "65012.25"),
            ],
        )

    def test_kraken_csv_counts_skipped_files_before_selected_member_pass(self):
        archive = self.tmp_path / "Kraken_OHLCVT.zip"
        with zipfile.ZipFile(archive, "w") as zf:
            zf.writestr("XBTUSD_1.csv", XBTUSD_CSV)
            zf.writestr("XBTUSD_1440.csv", XBTUSD_DAILY_CSV)
            zf.writestr("XBTEUR_1.csv", XBTEUR_FIXTURE.read_text(encoding="utf-8"))
            zf.writestr("ETHUSD_1.csv", XBTUSD_CSV)
            zf.writestr("XBTJPY_1.csv", XBTUSD_CSV)
            zf.writestr("README.txt", "not a csv")

        payload = self._run_json(
            "rates",
            "sync",
            "--source",
            "kraken-csv",
            "--path",
            str(archive),
            "--pair",
            "BTC/USD",
        )

        self.assertEqual(len(payload["data"]), 1)
        summary = payload["data"][0]
        self.assertEqual(summary["pair"], "BTC-USD")
        self.assertEqual(summary["granularity"], "minute")
        self.assertEqual(summary["samples"], 2)
        self.assertEqual(summary["skipped_files"], 5)

    def test_kraken_csv_skips_duplicate_normalized_pair_interval_members(self):
        archive = self.tmp_path / "Kraken_OHLCVT.zip"
        with zipfile.ZipFile(archive, "w") as zf:
            zf.writestr("XBTUSD_1440.csv", XBTUSD_DAILY_CSV)
            zf.writestr("XXBTZUSD_1440.csv", "1714521600,66000,66100,65900,66050,2,21\n")

        payload = self._run_json(
            "rates",
            "sync",
            "--source",
            "kraken-csv",
            "--path",
            str(archive),
            "--pair",
            "BTC/USD",
        )

        self.assertEqual(len(payload["data"]), 1)
        summary = payload["data"][0]
        self.assertEqual(summary["pair"], "BTC-USD")
        self.assertEqual(summary["granularity"], "daily")
        self.assertEqual(summary["samples"], 1)
        self.assertEqual(summary["skipped_files"], 1)
        conn = self._connect()
        row = conn.execute(
            "SELECT COUNT(*) AS count, rate_exact FROM rates_cache WHERE pair = 'BTC-USD'"
        ).fetchone()
        self.assertEqual(row["count"], 1)
        self.assertEqual(row["rate_exact"], "65050.00")

    def test_bundled_kraken_btc_daily_rates_import_eur_and_usd_only(self):
        self.assertEqual(
            sorted(path.name for path in BUNDLED_KRAKEN_BTC_DAILY.glob("*.csv")),
            ["XBTEUR_1440.csv", "XBTUSD_1440.csv"],
        )

        payload = self._run_json(
            "rates",
            "sync",
            "--source",
            "kraken-csv",
            "--path",
            str(BUNDLED_KRAKEN_BTC_DAILY),
        )

        self.assertEqual(len(payload["data"]), 2)
        summaries = {summary["pair"]: summary for summary in payload["data"]}
        self.assertEqual(sorted(summaries), ["BTC-EUR", "BTC-USD"])
        self.assertEqual(summaries["BTC-EUR"]["samples"], 4581)
        self.assertEqual(summaries["BTC-EUR"]["granularity"], "daily")
        self.assertEqual(
            summaries["BTC-EUR"]["first_timestamp"],
            "2013-09-11T00:00:00Z",
        )
        self.assertEqual(
            summaries["BTC-EUR"]["last_timestamp"],
            "2026-04-01T00:00:00Z",
        )
        self.assertEqual(summaries["BTC-USD"]["samples"], 4548)
        self.assertEqual(summaries["BTC-USD"]["granularity"], "daily")
        self.assertEqual(
            summaries["BTC-USD"]["first_timestamp"],
            "2013-10-07T00:00:00Z",
        )
        self.assertEqual(
            summaries["BTC-USD"]["last_timestamp"],
            "2026-04-01T00:00:00Z",
        )

        conn = self._connect()
        rows = conn.execute(
            """
            SELECT pair, COUNT(*) AS count, MIN(timestamp) AS first_timestamp,
                   MAX(timestamp) AS last_timestamp
            FROM rates_cache
            GROUP BY pair
            ORDER BY pair
            """
        ).fetchall()
        self.assertEqual(
            [
                (
                    row["pair"],
                    row["count"],
                    row["first_timestamp"],
                    row["last_timestamp"],
                )
                for row in rows
            ],
            [
                (
                    "BTC-EUR",
                    4581,
                    "2013-09-11T00:00:00Z",
                    "2026-04-01T00:00:00Z",
                ),
                (
                    "BTC-USD",
                    4548,
                    "2013-10-07T00:00:00Z",
                    "2026-04-01T00:00:00Z",
                ),
            ],
        )

    def test_desktop_daemon_imports_bundled_kraken_btc_daily_seed(self):
        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)

        payload = _rates_kraken_csv_import_payload(
            conn,
            {"operation": "full", "use_bundled": True},
        )

        self.assertTrue(payload["bundled"])
        self.assertEqual(payload["source"], "kraken-csv")
        self.assertEqual(payload["operation"], "full")
        self.assertEqual(payload["totals"]["pairs"], 2)
        self.assertEqual(payload["totals"]["samples"], 9129)
        summaries = {row["pair"]: row for row in payload["summary"]}
        self.assertEqual(summaries["BTC-EUR"]["samples"], 4581)
        self.assertEqual(summaries["BTC-USD"]["samples"], 4548)
        self.assertEqual(summaries["BTC-EUR"]["granularity"], "daily")
        self.assertEqual(summaries["BTC-USD"]["granularity"], "daily")

    def test_bundled_kraken_daily_seed_is_idempotent(self):
        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)

        _, first = core_rates.ensure_bundled_kraken_btc_daily_seed(conn)
        _, second = core_rates.ensure_bundled_kraken_btc_daily_seed(conn)

        first_summaries = {row["pair"]: row for row in first}
        second_summaries = {row["pair"]: row for row in second}
        self.assertFalse(first_summaries["BTC-EUR"]["already_seeded"])
        self.assertFalse(first_summaries["BTC-USD"]["already_seeded"])
        self.assertEqual(first_summaries["BTC-EUR"]["samples"], 4581)
        self.assertEqual(first_summaries["BTC-USD"]["samples"], 4548)
        self.assertTrue(second_summaries["BTC-EUR"]["already_seeded"])
        self.assertTrue(second_summaries["BTC-USD"]["already_seeded"])
        self.assertEqual(second_summaries["BTC-EUR"]["samples"], 0)
        self.assertEqual(second_summaries["BTC-USD"]["samples"], 0)

        rows = conn.execute(
            """
            SELECT pair, COUNT(*) AS count
            FROM rates_cache
            WHERE source = 'kraken-csv'
            GROUP BY pair
            ORDER BY pair
            """
        ).fetchall()
        self.assertEqual(
            [(row["pair"], row["count"]) for row in rows],
            [("BTC-EUR", 4581), ("BTC-USD", 4548)],
        )

    def test_bundled_kraken_import_supports_non_filesystem_resources(self):
        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)
        fake_resource = _MemoryResourceDir(
            [
                _MemoryResourceFile(
                    "XBTUSD_1440.csv",
                    XBTUSD_DAILY_CSV.encode("utf-8"),
                )
            ]
        )

        with patch.object(
            core_rates,
            "bundled_kraken_btc_daily_path",
            return_value=fake_resource,
        ):
            archive_path, summary = core_rates.sync_bundled_kraken_btc_daily(
                conn,
                pair="BTC-USD",
            )

        self.assertEqual(
            archive_path,
            "memory://kassiber/data/rates/kraken/btc_daily",
        )
        self.assertEqual(len(summary), 1)
        self.assertEqual(summary[0]["pair"], "BTC-USD")
        self.assertEqual(summary[0]["granularity"], "daily")
        self.assertEqual(summary[0]["samples"], 1)
        row = conn.execute(
            "SELECT COUNT(*) AS count FROM rates_cache WHERE pair = 'BTC-USD'"
        ).fetchone()
        self.assertEqual(row["count"], 1)

    def test_transaction_snapshot_exposes_rate_cache_pricing_provenance(self):
        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)
        self._seed_transaction_needing_rate(conn)
        set_setting(conn, "context_workspace", "workspace-1")
        set_setting(conn, "context_profile", "profile-1")
        conn.execute(
            """
            UPDATE transactions
            SET fiat_rate = 65050.0,
                fiat_value = 65.05,
                fiat_price_source = ?,
                fiat_rate_exact = '65050.00',
                fiat_value_exact = '65.05',
                pricing_source_kind = ?,
                pricing_provider = ?,
                pricing_pair = 'BTC-EUR',
                pricing_timestamp = '2024-05-02T00:00:00Z',
                pricing_fetched_at = '2026-05-24T00:00:00Z',
                pricing_granularity = 'daily',
                pricing_method = 'ohlcvt_csv',
                pricing_quality = ?
            WHERE id = 'tx-1'
            """,
            (
                pricing.LEGACY_SOURCE_RATES_CACHE,
                pricing.SOURCE_FMV_PROVIDER,
                core_rates.RATE_SOURCE_KRAKEN_CSV,
                pricing.QUALITY_COARSE_FALLBACK,
            ),
        )
        conn.commit()

        tx = build_transactions_snapshot(conn, {"limit": 5})["txs"][0]
        self.assertEqual(tx["pricingSourceKind"], pricing.SOURCE_FMV_PROVIDER)
        self.assertEqual(tx["pricingQuality"], pricing.QUALITY_COARSE_FALLBACK)
        self.assertEqual(tx["pricingProvider"], core_rates.RATE_SOURCE_KRAKEN_CSV)
        self.assertEqual(tx["pricingPair"], "BTC-EUR")
        self.assertEqual(tx["pricingTimestamp"], "2024-05-02T00:00:00Z")
        self.assertEqual(tx["pricingFetchedAt"], "2026-05-24T00:00:00Z")
        self.assertEqual(tx["pricingGranularity"], "daily")
        self.assertEqual(tx["pricingMethod"], "ohlcvt_csv")

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

    def test_coinbase_missing_minute_windows_use_coarse_utc_blocks(self):
        windows = core_rates._coinbase_windows_for_close_minutes(
            [
                "2024-05-01T10:00:00Z",
                "2024-05-01T10:01:00Z",
                "2024-05-01T12:34:00Z",
            ],
            now=datetime(2024, 5, 1, 20, 0, tzinfo=timezone.utc),
        )

        self.assertEqual(
            [(start.isoformat(), end.isoformat()) for start, end in windows],
            [
                ("2024-05-01T05:00:00+00:00", "2024-05-01T10:00:00+00:00"),
                ("2024-05-01T10:00:00+00:00", "2024-05-01T15:00:00+00:00"),
            ],
        )

    def test_coinbase_latest_rate_sync_fetches_only_recent_quote(self):
        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)
        calls = []

        def fake_coinbase_rows(pair, start, end, granularity=60):
            calls.append((pair, start, end, granularity))
            return [
                [
                    1714566720,
                    "59980.00",
                    "60000.00",
                    "59990.00",
                    "59995.00",
                    "0.40",
                ],
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
            summary = core_rates.sync_latest_rates(conn, pair="BTC-EUR")

        self.assertEqual(summary[0]["mode"], "latest_quote")
        self.assertEqual(summary[0]["samples"], 1)
        self.assertEqual(len(calls), 1)
        pair, start, end, granularity = calls[0]
        self.assertEqual(pair, "BTC-EUR")
        self.assertEqual(granularity, 60)
        self.assertLessEqual((end - start).total_seconds(), 5 * 60)
        row = conn.execute(
            """
            SELECT timestamp, rate_exact, open_rate_exact, close_rate_exact
            FROM rates_cache
            WHERE pair = 'BTC-EUR' AND source = 'coinbase-exchange'
            """
        ).fetchone()
        self.assertEqual(row["timestamp"], "2024-05-01T12:34:00Z")
        self.assertEqual(row["rate_exact"], "60010.00")
        self.assertEqual(row["open_rate_exact"], "60000.00")
        self.assertEqual(row["close_rate_exact"], "60010.00")

    def test_coingecko_latest_rate_sync_uses_live_provider_setting(self):
        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)

        with patch.object(
            core_rates,
            "fetch_rates_coingecko",
            return_value=[
                ("2024-05-01T12:30:00Z", 59990.0),
                ("2024-05-01T12:35:00Z", 60010.5),
            ],
        ) as fetch:
            summary = core_rates.sync_latest_rates(
                conn,
                pair="BTC-EUR",
                source=core_rates.RATE_SOURCE_COINGECKO,
            )

        fetch.assert_called_once_with("BTC-EUR", days=1)
        self.assertEqual(summary[0]["source"], core_rates.RATE_SOURCE_COINGECKO)
        self.assertEqual(summary[0]["mode"], "latest_quote")
        self.assertEqual(summary[0]["samples"], 1)
        self.assertEqual(summary[0]["granularity"], "five_minute")
        row = conn.execute(
            """
            SELECT timestamp, rate_exact, source, granularity, method
            FROM rates_cache
            WHERE pair = 'BTC-EUR' AND source = 'coingecko'
            """
        ).fetchone()
        self.assertEqual(row["timestamp"], "2024-05-01T12:35:00Z")
        self.assertEqual(row["rate_exact"], "60010.5")
        self.assertEqual(row["source"], core_rates.RATE_SOURCE_COINGECKO)
        self.assertEqual(row["granularity"], "five_minute")
        self.assertEqual(row["method"], "market_chart")

    def test_mempool_latest_rate_sync_uses_configured_backend_and_proxy(self):
        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)
        self._seed_mempool_backend(conn)
        calls = []

        def fake_http(url, timeout=30, proxy_url=None):
            calls.append((url, timeout, proxy_url))
            return {"EUR": "60010.50", "time": 1714566900}

        with patch.object(core_rates, "http_get_json", side_effect=fake_http):
            summary = core_rates.sync_latest_rates(
                conn,
                pair="BTC-EUR",
                source=core_rates.RATE_SOURCE_MEMPOOL,
            )

        self.assertEqual(
            calls,
            [
                (
                    "http://127.0.0.1:3006/api/v1/prices",
                    17,
                    "socks5h://127.0.0.1:9050",
                )
            ],
        )
        self.assertEqual(summary[0]["source"], core_rates.RATE_SOURCE_MEMPOOL)
        self.assertEqual(summary[0]["mode"], "latest_quote")
        self.assertEqual(summary[0]["samples"], 1)
        row = conn.execute(
            """
            SELECT timestamp, rate_exact, source, granularity, method
            FROM rates_cache
            WHERE pair = 'BTC-EUR' AND source = 'mempool'
            """
        ).fetchone()
        self.assertEqual(row["timestamp"], "2024-05-01T12:35:00Z")
        self.assertEqual(row["rate_exact"], "60010.50")
        self.assertEqual(row["granularity"], "latest")
        self.assertEqual(row["method"], "mempool_prices")

    def test_mempool_sync_fetches_missing_transaction_minute_from_backend(self):
        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)
        self._seed_mempool_backend(conn)
        self._seed_transaction_needing_rate(conn)
        calls = []

        def fake_http(url, timeout=30, proxy_url=None):
            calls.append((url, timeout, proxy_url))
            return {"prices": [{"time": 1714566840, "EUR": "60010.25"}]}

        with patch.object(core_rates, "http_get_json", side_effect=fake_http):
            summary = core_rates.sync_rates(
                conn,
                pair="BTC-EUR",
                source=core_rates.RATE_SOURCE_MEMPOOL,
                days=1,
            )

        self.assertEqual(len(calls), 1)
        url, timeout, proxy_url = calls[0]
        self.assertEqual(timeout, 17)
        self.assertEqual(proxy_url, "socks5h://127.0.0.1:9050")
        self.assertTrue(
            url.startswith("http://127.0.0.1:3006/api/v1/historical-price?")
        )
        self.assertIn("currency=EUR", url)
        self.assertIn("timestamp=1714566840", url)
        self.assertEqual(summary[0]["source"], core_rates.RATE_SOURCE_MEMPOOL)
        self.assertEqual(summary[0]["mode"], "transaction_need")
        self.assertEqual(summary[0]["needed_minutes"], 1)
        self.assertEqual(summary[0]["missing_minutes"], 1)
        row = conn.execute(
            """
            SELECT timestamp, rate_exact, source, granularity, method
            FROM rates_cache
            WHERE pair = 'BTC-EUR' AND source = 'mempool'
            """
        ).fetchone()
        self.assertEqual(row["timestamp"], "2024-05-01T12:34:00Z")
        self.assertEqual(row["rate_exact"], "60010.25")
        self.assertEqual(row["granularity"], "daily")
        self.assertEqual(row["method"], "historical_price")

    def test_desktop_latest_payload_preserves_historical_rate_cache(self):
        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)
        core_rates.upsert_rate(
            conn,
            "BTC-EUR",
            "2024-05-01T12:34:00Z",
            "59900.00",
            core_rates.RATE_SOURCE_COINBASE_EXCHANGE,
            fetched_at="2024-05-01T12:35:00Z",
            granularity="minute",
            method="product_candles",
        )
        conn.commit()

        def fake_coinbase_rows(pair, start, end, granularity=60):
            return [
                [
                    1714653240,
                    "60980.00",
                    "61040.00",
                    "60990.00",
                    "61010.00",
                    "0.55",
                ],
            ]

        with patch.object(
            core_rates,
            "_coinbase_exchange_candles",
            side_effect=fake_coinbase_rows,
        ):
            payload = _rates_latest_payload(conn, {"pair": "BTC-EUR"})

        self.assertEqual(payload["source"], core_rates.RATE_SOURCE_COINBASE_EXCHANGE)
        self.assertEqual(payload["pair"], "BTC-EUR")
        self.assertEqual(payload["latest"][0]["mode"], "latest_quote")
        self.assertEqual(payload["marketRate"]["rate"], 61010.0)
        self.assertEqual(payload["marketRate"]["timestamp"], "2024-05-02T12:35:00Z")
        rows = conn.execute(
            """
            SELECT timestamp, rate_exact
            FROM rates_cache
            WHERE pair = 'BTC-EUR' AND source = 'coinbase-exchange'
            ORDER BY timestamp ASC
            """
        ).fetchall()
        self.assertEqual(
            [(row["timestamp"], row["rate_exact"]) for row in rows],
            [
                ("2024-05-01T12:34:00Z", "59900.00"),
                ("2024-05-02T12:35:00Z", "61010.00"),
            ],
        )

    def test_desktop_latest_payload_defaults_to_configured_market_provider(self):
        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)
        core_rates.set_market_rate_provider(
            conn,
            core_rates.RATE_SOURCE_COINGECKO,
            commit=True,
        )

        with patch.object(
            core_rates,
            "fetch_rates_coingecko",
            return_value=[
                ("2024-05-01T12:30:00Z", 59990.0),
                ("2024-05-01T12:35:00Z", 60010.5),
            ],
        ) as fetch:
            payload = _rates_latest_payload(conn, {"pair": "BTC-EUR"})

        fetch.assert_called_once_with("BTC-EUR", days=1)
        self.assertEqual(payload["source"], core_rates.RATE_SOURCE_COINGECKO)
        self.assertEqual(payload["latest"][0]["source"], core_rates.RATE_SOURCE_COINGECKO)
        self.assertEqual(payload["marketRate"]["source"], core_rates.RATE_SOURCE_COINGECKO)
        self.assertEqual(payload["marketRate"]["rate"], 60010.5)

    def test_desktop_rebuild_defaults_to_configured_market_rate_provider(self):
        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)
        core_rates.set_market_rate_provider(
            conn,
            core_rates.RATE_SOURCE_COINGECKO,
            commit=True,
        )

        with patch.object(
            core_rates,
            "fetch_rates_coingecko",
            return_value=[("2024-05-01T12:35:00Z", 60010.5)],
        ) as fetch:
            payload = _rates_rebuild_payload(
                conn,
                {
                    "pair": "BTC-EUR",
                    "days": 1,
                    "reprice_transactions": False,
                },
            )

        fetch.assert_called_once_with("BTC-EUR", days=1)
        self.assertEqual(payload["source"], core_rates.RATE_SOURCE_COINGECKO)
        self.assertEqual(payload["sync"][0]["source"], core_rates.RATE_SOURCE_COINGECKO)

    def test_desktop_rebuild_without_pair_defaults_to_active_profile_btc_pair(self):
        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)
        self._seed_transaction_needing_rate(conn, profile_fiat="EUR")
        set_setting(conn, "context_workspace", "workspace-1")
        set_setting(conn, "context_profile", "profile-1")
        core_rates.set_market_rate_provider(
            conn,
            core_rates.RATE_SOURCE_COINGECKO,
            commit=True,
        )

        with patch.object(
            core_rates,
            "fetch_rates_coingecko",
            return_value=[("2024-05-01T12:35:00Z", 60010.5)],
        ) as fetch:
            payload = _rates_rebuild_payload(
                conn,
                {
                    "days": 1,
                    "reprice_transactions": False,
                },
            )

        fetch.assert_called_once_with("BTC-EUR", days=1)
        self.assertEqual(payload["pair"], "BTC-EUR")
        self.assertEqual(payload["source"], core_rates.RATE_SOURCE_COINGECKO)

    def test_coinbase_background_sync_skips_warm_cache_when_idle(self):
        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)

        with patch.object(core_rates, "_coinbase_exchange_candles") as fetch:
            summary = core_rates.sync_rates(
                conn,
                pair="BTC-EUR",
                days=1,
                warm_cache_when_idle=False,
            )

        fetch.assert_not_called()
        self.assertEqual(summary[0]["mode"], "idle_no_missing_minutes")
        self.assertEqual(summary[0]["samples"], 0)

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
        self.assertEqual(start.isoformat(), "2024-05-01T10:00:00+00:00")
        self.assertEqual(end.isoformat(), "2024-05-01T15:00:00+00:00")

        rate_row = conn.execute(
            """
            SELECT timestamp, rate_exact
            FROM rates_cache
            WHERE pair = 'BTC-EUR' AND source = 'coinbase-exchange'
            """
        ).fetchone()
        self.assertEqual(rate_row["timestamp"], "2024-05-01T12:34:00Z")
        self.assertEqual(rate_row["rate_exact"], "60010.00")

    def test_coinbase_sync_treats_zero_transaction_price_as_missing(self):
        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)
        self._seed_transaction_needing_rate(conn)
        conn.execute(
            """
            UPDATE transactions
            SET fiat_rate = 0, fiat_value = 0
            WHERE id = 'tx-1'
            """
        )
        conn.commit()

        def fake_coinbase_rows(pair, start, end, granularity=60):
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
        self.assertEqual(eur_summary["needed_minutes"], 1)
        self.assertEqual(eur_summary["missing_minutes"], 1)
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

    def test_rebuild_rates_cache_clears_provider_rows_and_reprices(self):
        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)
        self._seed_transaction_needing_rate(conn)
        core_rates.upsert_rate(
            conn,
            "BTC-EUR",
            "2024-05-01T12:34:00Z",
            "59900.00",
            core_rates.RATE_SOURCE_COINBASE_EXCHANGE,
            fetched_at="2024-05-01T00:00:00Z",
            granularity="minute",
            method="product_candles",
        )
        core_rates.upsert_rate(
            conn,
            "BTC-EUR",
            "2024-05-01T12:33:00Z",
            "60100.00",
            "manual",
            fetched_at="2024-05-01T00:00:00Z",
            granularity="exact",
            method="manual",
        )
        conn.execute(
            """
            INSERT INTO rates_checked_minutes(
                pair, timestamp, source, checked_at, granularity, method
            )
            VALUES (?, ?, ?, ?, ?, ?)
            """,
            (
                "BTC-EUR",
                "2024-05-01T12:34:00Z",
                core_rates.RATE_SOURCE_COINBASE_EXCHANGE,
                "2024-05-01T00:00:00Z",
                "minute",
                "product_candles",
            ),
        )
        conn.execute(
            """
            UPDATE transactions
            SET fiat_rate = ?, fiat_value = ?, fiat_price_source = ?,
                fiat_rate_exact = ?, fiat_value_exact = ?,
                pricing_source_kind = ?, pricing_provider = ?, pricing_pair = ?,
                pricing_timestamp = ?, pricing_fetched_at = ?,
                pricing_granularity = ?, pricing_method = ?,
                pricing_quality = ?
            WHERE id = ?
            """,
            (
                59900.0,
                59900.0,
                pricing.LEGACY_SOURCE_RATES_CACHE,
                "59900.00",
                "59900.00",
                pricing.SOURCE_FMV_PROVIDER,
                core_rates.RATE_SOURCE_COINBASE_EXCHANGE,
                "BTC-EUR",
                "2024-05-01T12:34:00Z",
                "2024-05-01T00:00:00Z",
                "minute",
                "product_candles",
                pricing.QUALITY_PROVIDER_SAMPLE,
                "tx-1",
            ),
        )
        conn.commit()

        def fake_coinbase_rows(pair, start, end, granularity=60):
            return [
                [
                    1714566780,
                    "60000.00",
                    "60040.00",
                    "60010.00",
                    "60030.00",
                    "0.75",
                ],
            ]

        with patch.object(
            core_rates,
            "_coinbase_exchange_candles",
            side_effect=fake_coinbase_rows,
        ):
            rebuilt = core_rates.rebuild_rates_cache(
                conn,
                pair="BTC-EUR",
                source=core_rates.RATE_SOURCE_COINBASE_EXCHANGE,
                reprice_transactions=True,
                profile_id="profile-1",
            )

        self.assertEqual(rebuilt["deleted"]["rates"], 1)
        self.assertEqual(rebuilt["deleted"]["checked_minutes"], 1)
        self.assertEqual(rebuilt["deleted"]["transaction_prices"], 1)
        self.assertEqual(rebuilt["sync"][0]["samples"], 1)
        rows = conn.execute(
            """
            SELECT source, rate_exact
            FROM rates_cache
            WHERE pair = 'BTC-EUR'
            ORDER BY source
            """
        ).fetchall()
        self.assertEqual(
            [(row["source"], row["rate_exact"]) for row in rows],
            [
                ("coinbase-exchange", "60030.00"),
                ("manual", "60100.00"),
            ],
        )
        checked = conn.execute("SELECT COUNT(*) FROM rates_checked_minutes").fetchone()[0]
        self.assertEqual(checked, 300)
        tx = conn.execute(
            """
            SELECT fiat_rate, fiat_value, pricing_source_kind
            FROM transactions
            WHERE id = 'tx-1'
            """
        ).fetchone()
        self.assertIsNone(tx["fiat_rate"])
        self.assertIsNone(tx["fiat_value"])
        self.assertIsNone(tx["pricing_source_kind"])

    def test_rebuild_rates_cache_rolls_back_when_provider_fetch_fails(self):
        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)
        self._seed_transaction_needing_rate(conn)
        core_rates.upsert_rate(
            conn,
            "BTC-EUR",
            "2024-05-01T12:34:00Z",
            "59900.00",
            core_rates.RATE_SOURCE_COINBASE_EXCHANGE,
            fetched_at="2024-05-01T00:00:00Z",
            granularity="minute",
            method="product_candles",
        )
        conn.execute(
            """
            INSERT INTO rates_checked_minutes(
                pair, timestamp, source, checked_at, granularity, method
            )
            VALUES (?, ?, ?, ?, ?, ?)
            """,
            (
                "BTC-EUR",
                "2024-05-01T12:34:00Z",
                core_rates.RATE_SOURCE_COINBASE_EXCHANGE,
                "2024-05-01T00:00:00Z",
                "minute",
                "product_candles",
            ),
        )
        conn.execute(
            """
            UPDATE transactions
            SET fiat_rate = ?, fiat_value = ?, fiat_price_source = ?,
                fiat_rate_exact = ?, fiat_value_exact = ?,
                pricing_source_kind = ?, pricing_provider = ?, pricing_pair = ?,
                pricing_timestamp = ?, pricing_fetched_at = ?,
                pricing_granularity = ?, pricing_method = ?,
                pricing_quality = ?
            WHERE id = ?
            """,
            (
                59900.0,
                59900.0,
                pricing.LEGACY_SOURCE_RATES_CACHE,
                "59900.00",
                "59900.00",
                pricing.SOURCE_FMV_PROVIDER,
                core_rates.RATE_SOURCE_COINBASE_EXCHANGE,
                "BTC-EUR",
                "2024-05-01T12:34:00Z",
                "2024-05-01T00:00:00Z",
                "minute",
                "product_candles",
                pricing.QUALITY_PROVIDER_SAMPLE,
                "tx-1",
            ),
        )
        conn.commit()

        with self.assertRaises(AppError):
            core_rates.rebuild_rates_cache(
                conn,
                pair="BTC-EUR",
                days=0,
                source=core_rates.RATE_SOURCE_COINBASE_EXCHANGE,
                reprice_transactions=True,
                profile_id="profile-1",
            )
        with patch.object(
            core_rates,
            "_coinbase_exchange_candles",
            side_effect=RuntimeError("provider down"),
        ):
            with self.assertRaises(RuntimeError):
                core_rates.rebuild_rates_cache(
                    conn,
                    pair="BTC-EUR",
                    source=core_rates.RATE_SOURCE_COINBASE_EXCHANGE,
                    reprice_transactions=True,
                    profile_id="profile-1",
                )

        rate_count = conn.execute(
            """
            SELECT COUNT(*) FROM rates_cache
            WHERE source = 'coinbase-exchange' AND rate_exact = '59900.00'
            """
        ).fetchone()[0]
        checked_count = conn.execute(
            "SELECT COUNT(*) FROM rates_checked_minutes"
        ).fetchone()[0]
        tx = conn.execute(
            """
            SELECT fiat_rate_exact, pricing_source_kind, pricing_provider
            FROM transactions
            WHERE id = 'tx-1'
            """
        ).fetchone()
        self.assertEqual(rate_count, 1)
        self.assertEqual(checked_count, 1)
        self.assertEqual(tx["fiat_rate_exact"], "59900.00")
        self.assertEqual(tx["pricing_source_kind"], pricing.SOURCE_FMV_PROVIDER)
        self.assertEqual(tx["pricing_provider"], "coinbase-exchange")

    def test_desktop_daemon_rebuild_reprices_active_profile(self):
        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)
        self._seed_transaction_needing_rate(conn)
        set_setting(conn, "context_workspace", "workspace-1")
        set_setting(conn, "context_profile", "profile-1")
        core_rates.upsert_rate(
            conn,
            "BTC-EUR",
            "2024-05-01T12:34:00Z",
            "59900.00",
            core_rates.RATE_SOURCE_COINBASE_EXCHANGE,
            fetched_at="2024-05-01T00:00:00Z",
            granularity="minute",
            method="product_candles",
        )
        conn.execute(
            """
            UPDATE transactions
            SET fiat_rate = ?, fiat_value = ?, fiat_price_source = ?,
                fiat_rate_exact = ?, fiat_value_exact = ?
            WHERE id = ?
            """,
            (
                59900.0,
                59900.0,
                pricing.LEGACY_SOURCE_RATES_CACHE,
                "59900.00",
                "59900.00",
                "tx-1",
            ),
        )
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, confirmed_at, direction, asset, amount, fee,
                fiat_currency, fiat_rate, fiat_value, fiat_price_source,
                fiat_rate_exact, fiat_value_exact, pricing_source_kind,
                pricing_provider, pricing_pair, pricing_timestamp,
                pricing_fetched_at, pricing_granularity, pricing_method,
                pricing_quality, raw_json, created_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "tx-2",
                "workspace-1",
                "profile-1",
                "wallet-1",
                "kraken-priced",
                "fingerprint-kraken-priced",
                "2024-05-01T12:40:00Z",
                None,
                "in",
                "BTC",
                200_000_000,
                0,
                None,
                61000.0,
                122000.0,
                pricing.LEGACY_SOURCE_RATES_CACHE,
                "61000.00",
                "122000.00",
                pricing.SOURCE_FMV_PROVIDER,
                core_rates.RATE_SOURCE_KRAKEN_CSV,
                "BTC-EUR",
                "2024-05-01T12:40:00Z",
                "2024-05-01T00:00:00Z",
                "minute",
                "ohlcvt_csv",
                pricing.QUALITY_PROVIDER_SAMPLE,
                "{}",
                "2024-05-01T00:00:00Z",
            ),
        )
        conn.commit()

        def fake_coinbase_rows(pair, start, end, granularity=60):
            return [
                [
                    1714566780,
                    "60000.00",
                    "60040.00",
                    "60010.00",
                    "60030.00",
                    "0.75",
                ],
            ]

        with patch.object(
            core_rates,
            "_coinbase_exchange_candles",
            side_effect=fake_coinbase_rows,
        ):
            payload = _rates_rebuild_payload(
                conn,
                {
                    "source": core_rates.RATE_SOURCE_COINBASE_EXCHANGE,
                    "pair": "BTC-EUR",
                    "reprice_transactions": True,
                },
            )

        self.assertEqual(payload["source"], "coinbase-exchange")
        self.assertEqual(payload["deleted"]["transaction_prices"], 1)
        self.assertEqual(payload["reprice"], {"auto_priced": 1})
        self.assertTrue(payload["journals"]["ok"])
        self.assertIsNotNone(payload["journals"]["result"])
        tx = conn.execute(
            """
            SELECT fiat_rate_exact, fiat_value_exact, pricing_source_kind,
                   pricing_provider
            FROM transactions
            WHERE id = 'tx-1'
            """
        ).fetchone()
        self.assertEqual(tx["fiat_rate_exact"], "60030.00")
        self.assertEqual(tx["fiat_value_exact"], "60.03000")
        self.assertEqual(tx["pricing_source_kind"], pricing.SOURCE_FMV_PROVIDER)
        self.assertEqual(tx["pricing_provider"], "coinbase-exchange")
        preserved = conn.execute(
            """
            SELECT fiat_rate_exact, fiat_value_exact, pricing_source_kind,
                   pricing_provider
            FROM transactions
            WHERE id = 'tx-2'
            """
        ).fetchone()
        self.assertEqual(preserved["fiat_rate_exact"], "61000.00")
        self.assertEqual(preserved["fiat_value_exact"], "122000.00")
        self.assertEqual(
            preserved["pricing_source_kind"],
            pricing.SOURCE_FMV_PROVIDER,
        )
        self.assertEqual(preserved["pricing_provider"], "kraken-csv")

    def test_desktop_daemon_rebuild_returns_journal_error_after_price_sync(self):
        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)
        self._seed_transaction_needing_rate(conn)
        set_setting(conn, "context_workspace", "workspace-1")
        set_setting(conn, "context_profile", "profile-1")

        def fake_coinbase_rows(pair, start, end, granularity=60):
            return [
                [
                    1714566780,
                    "60000.00",
                    "60040.00",
                    "60010.00",
                    "60030.00",
                    "0.75",
                ],
            ]

        with (
            patch.object(
                core_rates,
                "_coinbase_exchange_candles",
                side_effect=fake_coinbase_rows,
            ),
            patch(
                "kassiber.daemon.process_journals",
                side_effect=AppError("negative balance", code="app_error"),
            ),
        ):
            payload = _rates_rebuild_payload(
                conn,
                {
                    "source": core_rates.RATE_SOURCE_COINBASE_EXCHANGE,
                    "pair": "BTC-EUR",
                    "reprice_transactions": True,
                },
            )

        self.assertEqual(payload["source"], "coinbase-exchange")
        self.assertEqual(payload["reprice"], {"auto_priced": 1})
        self.assertEqual(payload["journals"]["ok"], False)
        self.assertEqual(payload["journals"]["error"]["message"], "negative balance")
        tx = conn.execute(
            """
            SELECT fiat_rate_exact, fiat_value_exact, pricing_source_kind,
                   pricing_provider
            FROM transactions
            WHERE id = 'tx-1'
            """
        ).fetchone()
        self.assertEqual(tx["fiat_rate_exact"], "60030.00")
        self.assertEqual(tx["fiat_value_exact"], "60.03000")
        self.assertEqual(tx["pricing_source_kind"], pricing.SOURCE_FMV_PROVIDER)
        self.assertEqual(tx["pricing_provider"], "coinbase-exchange")
        profile = conn.execute(
            """
            SELECT last_processed_at, journal_input_version,
                   last_processed_input_version
            FROM profiles
            WHERE id = 'profile-1'
            """
        ).fetchone()
        self.assertIsNone(profile["last_processed_at"])
        self.assertEqual(profile["journal_input_version"], 1)
        self.assertEqual(profile["last_processed_input_version"], 0)

    def test_journal_processing_rolls_back_auto_pricing_on_ledger_error(self):
        from kassiber.cli import handlers

        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)
        self._seed_transaction_needing_rate(conn)
        core_rates.upsert_rate(
            conn,
            "BTC-EUR",
            "2024-05-01T12:34:00Z",
            "59900.00",
            core_rates.RATE_SOURCE_COINBASE_EXCHANGE,
            fetched_at="2024-05-01T00:00:00Z",
            granularity="minute",
            method="product_candles",
        )
        conn.commit()

        with patch.object(
            handlers,
            "build_ledger_state",
            side_effect=AppError("negative balance", code="app_error"),
        ):
            with self.assertRaises(AppError):
                handlers.process_journals(conn, None, None)

        tx = conn.execute(
            """
            SELECT fiat_rate_exact, fiat_value_exact, pricing_source_kind
            FROM transactions
            WHERE id = 'tx-1'
            """
        ).fetchone()
        self.assertIsNone(tx["fiat_rate_exact"])
        self.assertIsNone(tx["fiat_value_exact"])
        self.assertIsNone(tx["pricing_source_kind"])
        self.assertFalse(conn.in_transaction)


if __name__ == "__main__":
    unittest.main()
