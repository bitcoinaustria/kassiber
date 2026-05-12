import json
import sqlite3
import subprocess
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

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
        self.assertEqual(summary["first_timestamp"], "2024-05-01T00:00:00Z")
        self.assertEqual(summary["last_timestamp"], "2024-05-01T00:09:00Z")

        conn = self._connect()
        row = conn.execute(
            """
            SELECT pair, timestamp, rate_exact, source, granularity, method,
                   open_rate_exact, high_rate_exact, low_rate_exact,
                   close_rate_exact, volume_exact, trades
            FROM rates_cache
            WHERE timestamp = ?
            """,
            ("2024-05-01T00:03:00Z",),
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


if __name__ == "__main__":
    unittest.main()
