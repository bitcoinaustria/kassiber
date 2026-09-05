"""Fast production-import replay for the Docker exchange lifecycle oracle."""
import csv
from pathlib import Path
import tempfile
import unittest

from tests.integration.regtest_demo import run_cli
from tests.integration.regtest_exchange_cases import reconcile_exports, write_exports, price_native_fixture_rows, EXPECTED
from kassiber.importers import load_strike_csv_records


class RegtestExchangeCasesTest(unittest.TestCase):
    def test_export_pages_preserve_execution_cost_and_distinct_account_fee(self):
        with tempfile.TemporaryDirectory() as tmp:
            files = write_exports(Path(tmp), txids={"withdrawal":"a"*64,"deposit":"b"*64}, times={
                "buy_time":"2020-01-01T00:00:00Z", "withdrawal_time":"2020-01-02T00:00:00Z",
                "deposit_time":"2020-01-03T00:00:00Z", "sell_time":"2020-01-04T00:00:00Z",
            })
            self.assertEqual(len(load_strike_csv_records(files["incomplete"])), 3)
            records = load_strike_csv_records(files["complete"])
            self.assertEqual(str(records[0]["fiat_value"]), "1001")
            self.assertEqual(str(records[1]["fee"]), "0.00010000")
            self.assertEqual(str(records[3]["fiat_value"]), "149")

    def test_real_cli_partial_full_and_repeated_export_matches_independent_oracle(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            run_cli(root, "init")
            run_cli(root, "workspaces", "create", "Exchange tests")
            profile = run_cli(root, "profiles", "create", "Exchange lifecycle", "--workspace", "Exchange tests",
                              "--fiat-currency", "EUR", "--tax-country", "generic", "--gains-algorithm", "FIFO")["data"]
            scope = ("--workspace", "Exchange tests", "--profile", profile["id"])
            wallet = run_cli(root, "wallets", "create", *scope, "--label", "Native replay", "--kind", "custom")["data"]
            txids = {"withdrawal":"a"*64,"deposit":"b"*64}
            times = {"buy_time":"2020-01-01T00:00:00Z", "withdrawal_time":"2020-01-02T00:00:00Z",
                     "deposit_time":"2020-01-03T00:00:00Z", "sell_time":"2020-01-04T00:00:00Z"}
            # Fast lane imports observations explicitly. Docker runner supplies
            # these two rows through actual Core-backed watch-only sync instead.
            source = root / "native-replay.csv"
            with source.open("w", newline="") as stream:
                writer = csv.writer(stream)
                writer.writerow(["txid","occurred_at","direction","asset","amount","fee"])
                writer.writerow([txids["withdrawal"], times["withdrawal_time"], "inbound", "BTC", "0.006", "0"])
                writer.writerow([txids["deposit"], times["deposit_time"], "outbound", "BTC", "0.002", "0.00001"])
            run_cli(root, "wallets", "import-csv", *scope, "--wallet", wallet["id"], "--file", str(source))
            price_native_fixture_rows(root, profile, wallet)
            result = reconcile_exports(data_root=root, artifact_dir=root/"evidence", workspace="Exchange tests",
                                       profile=profile, native_wallet=wallet, txids=txids, times=times, cli=run_cli)
            self.assertTrue(any(row["reason"] == "insufficient_lots" for row in result["incomplete_export_cases"]), result["incomplete_export_cases"])
            self.assertEqual(result["actual"]["combined_quantity_msat"], EXPECTED["combined_quantity_msat"])
            self.assertEqual(result["actual"]["excluded_transactions"], 0)
            self.assertEqual(result["imports"]["repeat"]["imported"], 0)
            self.assertTrue(all(row["imported"] == 0 for row in result["imports"]["file_sync"]))
