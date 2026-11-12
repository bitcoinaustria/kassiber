"""End-to-end coverage for 21bitcoin's txid-less custodial ledger export."""

import json
import tempfile
import unittest
from decimal import Decimal
from pathlib import Path

from kassiber.importers import load_twentyonebitcoin_csv_records
from tests import cli_runner


ROOT = Path(__file__).resolve().parent.parent
HEADER = (
    "id,exchange_name,depot_name,transaction_date,buy_asset,buy_amount,"
    "sell_asset,sell_amount,fee_asset,fee_amount,transaction_type,note,"
    "linked_transaction,btc_price\n"
)


def _run(data_root, *args):
    result = cli_runner.run(
        [
            "--data-root",
            str(data_root),
            "--machine",
            *args,
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if not result.stdout.strip():
        raise AssertionError(
            f"CLI produced no stdout for {args!r}: {result.stderr}"
        )
    return json.loads(result.stdout), result.returncode


class TwentyOneBitcoinImportTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="kassiber-21bitcoin-")
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        self.data_root = self.root / "data"
        self._ok("init")
        self._ok("workspaces", "create", "Main")
        self._ok(
            "profiles",
            "create",
            "--workspace",
            "Main",
            "--fiat-currency",
            "EUR",
            "--tax-country",
            "generic",
            "Custody",
        )

    def _ok(self, *args):
        payload, code = _run(self.data_root, *args)
        self.assertEqual(code, 0, payload)
        return payload

    def _write(self, name, contents):
        path = self.root / name
        path.write_text(contents, encoding="utf-8")
        return path

    def _create_wallet(self, label):
        self._ok(
            "wallets",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Custody",
            "--label",
            label,
            "--kind",
            "custom",
        )

    def _import_csv(self, wallet, path):
        return self._ok(
            "wallets",
            "import-csv",
            "--workspace",
            "Main",
            "--profile",
            "Custody",
            "--wallet",
            wallet,
            "--file",
            str(path),
        )

    def _transactions(self, wallet):
        return self._ok(
            "transactions",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Custody",
            "--wallet",
            wallet,
            "--order",
            "asc",
        )["data"]

    def test_parser_keeps_provider_identity_skips_blank_rows_and_uses_net_sell_proceeds(self):
        export = self._write(
            "parser.csv",
            HEADER
            + "303,21bitcoin,main,06.08.2026 08:35:23,EUR,3112.1,BTC,0.0566936,EUR,47.08,trade,Standard BTC Sell,,55723.62\n"
            + "304,21bitcoin,main,06.08.2026 08:36:12,,,BTC,0.01000000,BTC,0.00001000,withdrawal,L1 BTC Withdrawal,not-a-txid,\n"
            + ",,,,,,,,,,,,,\n",
        )

        records = load_twentyonebitcoin_csv_records(export)

        self.assertEqual(len(records), 2)
        sale, withdrawal = records
        self.assertEqual(sale["txid"], "21bitcoin:303")
        self.assertEqual(sale["fiat_value"], Decimal("3112.1"))
        self.assertEqual(sale["fiat_rate"], Decimal("3112.1") / Decimal("0.0566936"))
        self.assertEqual(withdrawal["txid"], "21bitcoin:304")
        self.assertIsNone(withdrawal["fiat_value"])
        self.assertIsNone(withdrawal["fiat_rate"])

    def test_txidless_withdrawal_is_reviewed_then_carries_basis_to_self_custody(self):
        platform_export = self._write(
            "21bitcoin-withdrawal.csv",
            HEADER
            + "1,21bitcoin,main,01.01.2026 10:00:00,BTC,0.00600000,EUR,300.00,EUR,3.00,trade,First purchase,,50000\n"
            + "2,21bitcoin,main,02.01.2026 10:00:00,BTC,0.00400000,EUR,200.00,EUR,2.00,trade,Second purchase,,50000\n"
            + "3,21bitcoin,main,10.01.2026 12:00:00,,,BTC,0.00600000,BTC,0.00010000,withdrawal,Threshold L1 withdrawal,,\n"
            + ",,,,,,,,,,,,,\n",
        )
        receive_txid = "a" * 64
        self_custody_export = self._write(
            "self-custody-receive.csv",
            "date,txid,direction,asset,amount,fee,description\n"
            f"2026-01-10T12:05:00Z,{receive_txid},inbound,BTC,0.00600000,0,Receive from 21bitcoin\n",
        )
        self._create_wallet("Cold Wallet")
        self._import_csv("Cold Wallet", self_custody_export)

        imported = self._ok(
            "wallets",
            "import-21bitcoin",
            "--workspace",
            "Main",
            "--profile",
            "Custody",
            "--file",
            str(platform_export),
        )["data"]
        self.assertEqual(imported["mode"], "full")
        self.assertEqual(imported["twentyonebitcoin_rows"], 3)
        self.assertEqual(imported["imported"], 3)

        reimported = self._ok(
            "wallets",
            "import-21bitcoin",
            "--workspace",
            "Main",
            "--profile",
            "Custody",
            "--file",
            str(platform_export),
        )["data"]
        self.assertEqual(reimported["imported"], 0)
        self.assertEqual(len(self._transactions("21bitcoin")), 3)

        platform_rows = {
            row["external_id"]: row for row in self._transactions("21bitcoin")
        }
        withdrawal = platform_rows["21bitcoin:3"]
        self.assertIsNone(withdrawal["fiat_rate"])
        self.assertIsNone(withdrawal["fiat_value"])
        receive = self._transactions("Cold Wallet")[0]
        self.assertIsNone(receive["fiat_rate"])
        self.assertIsNone(receive["fiat_value"])

        suggested = self._ok(
            "transfers",
            "suggest",
            "--workspace",
            "Main",
            "--profile",
            "Custody",
            "--candidate-type",
            "transfer",
        )["data"]
        self.assertEqual(suggested["counts"]["total"], 1)
        candidate = suggested["candidates"][0]
        self.assertEqual(candidate["confidence"], "strong")
        self.assertEqual(candidate["method"], "heuristic")
        self.assertEqual(candidate["out_wallet_label"], "21bitcoin")
        self.assertEqual(candidate["in_wallet_label"], "Cold Wallet")

        exact_only = self._ok(
            "transfers",
            "bulk-pair",
            "--workspace",
            "Main",
            "--profile",
            "Custody",
        )["data"]
        self.assertEqual(exact_only["summary"]["count"], 0)

        self._ok(
            "transfers",
            "pair",
            "--workspace",
            "Main",
            "--profile",
            "Custody",
            "--tx-out",
            candidate["out_id"],
            "--tx-in",
            candidate["in_id"],
            "--kind",
            "manual",
            "--policy",
            "carrying-value",
        )
        for timestamp in ("2026-01-10T12:00:00Z", "2026-01-10T12:05:00Z"):
            self._ok("rates", "set", "BTC-EUR", timestamp, "55000")
        journal = self._ok(
            "journals",
            "process",
            "--workspace",
            "Main",
            "--profile",
            "Custody",
        )["data"]
        self.assertEqual(journal["transfers_detected"], 1)
        quarantined = self._ok(
            "journals",
            "quarantined",
            "--workspace",
            "Main",
            "--profile",
            "Custody",
        )["data"]
        self.assertEqual(quarantined, [])

        portfolio = {
            row["wallet"]: row
            for row in self._ok(
                "reports",
                "portfolio-summary",
                "--workspace",
                "Main",
                "--profile",
                "Custody",
            )["data"]
        }
        self.assertAlmostEqual(float(portfolio["21bitcoin"]["quantity"]), 0.0039, places=8)
        self.assertAlmostEqual(float(portfolio["Cold Wallet"]["quantity"]), 0.006, places=8)
        self.assertAlmostEqual(float(portfolio["21bitcoin"]["avg_cost"]), 50500.0, places=2)
        self.assertAlmostEqual(float(portfolio["Cold Wallet"]["avg_cost"]), 50500.0, places=2)
        capital_gains = self._ok(
            "reports",
            "capital-gains",
            "--workspace",
            "Main",
            "--profile",
            "Custody",
        )["data"]
        self.assertEqual(capital_gains, [])

    def test_btc_deposit_is_the_reverse_reviewed_custody_move(self):
        source_export = self._write(
            "source-wallet.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-01-01T10:00:00Z,source-funding,inbound,BTC,0.00300000,0,50000,Funding\n"
            "2026-01-10T12:00:00Z,source-send,outbound,BTC,0.00200000,0.00001000,,Deposit to 21bitcoin\n",
        )
        platform_export = self._write(
            "21bitcoin-deposit.csv",
            HEADER
            + "9,21bitcoin,main,10.01.2026 12:05:00,BTC,0.00200000,,,BTC,0,deposit,BTC Deposit,,\n",
        )
        self._create_wallet("Source Wallet")
        self._import_csv("Source Wallet", source_export)
        self._ok(
            "wallets",
            "import-21bitcoin",
            "--workspace",
            "Main",
            "--profile",
            "Custody",
            "--file",
            str(platform_export),
        )

        deposit = self._transactions("21bitcoin")[0]
        self.assertEqual(deposit["external_id"], "21bitcoin:9")
        self.assertEqual(deposit["kind"], "deposit")
        self.assertIsNone(deposit["fiat_rate"])

        candidate = self._ok(
            "transfers",
            "suggest",
            "--workspace",
            "Main",
            "--profile",
            "Custody",
            "--candidate-type",
            "transfer",
        )["data"]["candidates"][0]
        self.assertEqual(candidate["confidence"], "strong")
        self.assertEqual(candidate["out_wallet_label"], "Source Wallet")
        self.assertEqual(candidate["in_wallet_label"], "21bitcoin")

        self._ok(
            "transfers",
            "pair",
            "--workspace",
            "Main",
            "--profile",
            "Custody",
            "--tx-out",
            candidate["out_id"],
            "--tx-in",
            candidate["in_id"],
            "--kind",
            "manual",
            "--policy",
            "carrying-value",
        )
        for timestamp in ("2026-01-10T12:00:00Z", "2026-01-10T12:05:00Z"):
            self._ok("rates", "set", "BTC-EUR", timestamp, "55000")
        journal = self._ok(
            "journals",
            "process",
            "--workspace",
            "Main",
            "--profile",
            "Custody",
        )["data"]
        self.assertEqual(journal["transfers_detected"], 1, journal)
        quarantined = self._ok(
            "journals",
            "quarantined",
            "--workspace",
            "Main",
            "--profile",
            "Custody",
        )["data"]
        self.assertEqual(quarantined, [])
        portfolio = {
            row["wallet"]: row
            for row in self._ok(
                "reports",
                "portfolio-summary",
                "--workspace",
                "Main",
                "--profile",
                "Custody",
            )["data"]
        }
        self.assertAlmostEqual(float(portfolio["Source Wallet"]["quantity"]), 0.00099, places=8)
        self.assertAlmostEqual(float(portfolio["21bitcoin"]["quantity"]), 0.002, places=8)
        self.assertAlmostEqual(float(portfolio["21bitcoin"]["avg_cost"]), 50000.0, places=2)

    def test_ambiguous_txidless_withdrawal_stays_unpaired(self):
        platform_export = self._write(
            "ambiguous-withdrawal.csv",
            HEADER
            + "5,21bitcoin,main,10.01.2026 12:00:00,,,BTC,0.00100000,BTC,0,withdrawal,L1 withdrawal,,\n",
        )
        self._ok(
            "wallets",
            "import-21bitcoin",
            "--workspace",
            "Main",
            "--profile",
            "Custody",
            "--file",
            str(platform_export),
        )
        for index, minute in ((1, "05"), (2, "10")):
            label = f"Candidate {index}"
            self._create_wallet(label)
            receive = self._write(
                f"candidate-{index}.csv",
                "date,txid,direction,asset,amount,fee,description\n"
                f"2026-01-10T12:{minute}:00Z,{str(index) * 64},inbound,BTC,0.00100000,0,Possible receipt\n",
            )
            self._import_csv(label, receive)

        suggested = self._ok(
            "transfers",
            "suggest",
            "--workspace",
            "Main",
            "--profile",
            "Custody",
            "--candidate-type",
            "transfer",
        )["data"]
        self.assertEqual(suggested["counts"]["total"], 2)
        self.assertTrue(
            all(candidate["conflict_size"] == 2 for candidate in suggested["candidates"])
        )

        bulk = self._ok(
            "transfers",
            "bulk-pair",
            "--workspace",
            "Main",
            "--profile",
            "Custody",
            "--confidence",
            "strong",
        )["data"]
        self.assertEqual(bulk["summary"]["count"], 0)
        self.assertEqual(bulk["summary"]["skipped_conflicts"], 2)
        pairs = self._ok(
            "transfers",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Custody",
        )["data"]
        self.assertEqual(pairs, [])


class TwentyOneBitcoinLegacyReadinessTest(unittest.TestCase):
    def test_legacy_reactivation_invalidates_ready_journals_once(self):
        from contextlib import closing
        from unittest.mock import Mock

        from kassiber.core.imports import ImportCoordinatorHooks, import_file_into_wallet
        from kassiber.core.repo.context import invalidate_journals
        from kassiber.db import open_db

        for priced, excluded in ((False, True), (False, False), (True, True)):
            with self.subTest(priced=priced, excluded=excluded):
                with tempfile.TemporaryDirectory(prefix="kassiber-21bitcoin-readiness-") as root, closing(open_db(Path(root) / "data")) as conn:
                    conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('ws','WS','2025-01-01')")
                    conn.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,tax_long_term_days,gains_algorithm,created_at) VALUES('p','ws','P','EUR','generic',365,'FIFO','2025-01-01')")
                    conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('w','ws','p','21bitcoin','21bitcoin','{}','2025-01-01')")
                    profile = conn.execute("SELECT * FROM profiles").fetchone()
                    wallet = conn.execute("SELECT * FROM wallets").fetchone()
                    source = Path(root) / "history.csv"
                    row = (
                        "1,21bitcoin,main,10.01.2026 12:00:00,BTC,0.002,EUR,100,EUR,1,trade,Buy,,\n"
                        if priced else
                        "1,21bitcoin,main,10.01.2026 12:00:00,,,BTC,0.002,BTC,0.0001,withdrawal,Withdrawal,,\n"
                    )
                    source.write_text(HEADER + row, encoding="utf-8")
                    invalidations = Mock(wraps=invalidate_journals)
                    hooks = ImportCoordinatorHooks(ensure_tag_row=Mock(), invalidate_journals=invalidations)

                    def run():
                        return import_file_into_wallet(
                            conn, profile, wallet, str(source), "21bitcoin_csv", hooks
                        )

                    run()
                    if not priced:
                        self.assertFalse(run()["journal_invalidated"])
                    conn.execute("INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES('legacy','ws','p','21bitcoin-matched','Old','2025-01-01'),('authored','ws','p','keep','Keep','2025-01-01')")
                    conn.execute("INSERT INTO transaction_tags(transaction_id,tag_id) SELECT id,'legacy' FROM transactions")
                    conn.execute("INSERT INTO transaction_tags(transaction_id,tag_id) SELECT id,'authored' FROM transactions")
                    conn.execute("UPDATE transactions SET excluded=?", (int(excluded),))
                    conn.execute("UPDATE profiles SET last_processed_at='2026-01-11T00:00:00Z',last_processed_tx_count=1,last_processed_input_version=journal_input_version,ownership_review_counts_json='{}'")
                    conn.commit()
                    before = conn.execute("SELECT journal_input_version FROM profiles").fetchone()[0]
                    invalidations.reset_mock()
                    result = run()
                    should_invalidate = priced or excluded
                    self.assertEqual(result["journal_invalidated"], should_invalidate)
                    self.assertEqual(result.get("reactivated", 0), int(excluded))
                    self.assertEqual(result["reconciliation_flags_cleared"], 1)
                    self.assertEqual(result["imported"], 0)
                    self.assertNotIn("import_batch_id", result)
                    self.assertEqual(invalidations.call_count, int(should_invalidate))
                    after = conn.execute("SELECT * FROM profiles").fetchone()
                    self.assertEqual(after["journal_input_version"], before + int(should_invalidate))
                    self.assertEqual(after["last_processed_input_version"], before)
                    self.assertEqual(after["last_processed_at"], None if should_invalidate else "2026-01-11T00:00:00Z")
                    self.assertEqual(after["last_processed_tx_count"], 0 if should_invalidate else 1)
                    self.assertEqual(after["ownership_review_counts_json"], None if should_invalidate else "{}")
                    self.assertEqual(conn.execute("SELECT excluded FROM transactions").fetchone()[0], 0)
                    self.assertEqual([row[0] for row in conn.execute("SELECT tag_id FROM transaction_tags")], ["authored"])


if __name__ == "__main__":
    unittest.main()
