from __future__ import annotations

import socket
import tempfile
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path
from unittest.mock import patch

import openpyxl

from kassiber import backends as backend_config
from kassiber.core import accounts as core_accounts
from kassiber.core import imports as core_imports
from kassiber.core import output_inventory as core_output_inventory
from kassiber.core import rates as core_rates
from kassiber.core import reports as core_reports
from kassiber.core import sync as core_sync
from kassiber.core import sync_backends as core_sync_backends
from kassiber.core import wallets as core_wallets
from kassiber.core.repo import invalidate_journals
from kassiber.cli.handlers import _report_hooks as cli_report_hooks
from kassiber.cli.handlers import process_journals
from kassiber.db import open_db

from tests.integration.env import no_egress_guard
from tests.integration import regtest_demo
from tests.integration.tapes import BitcoinRpcTape, RecordedTape, TapeMiss


ROOT = Path(__file__).resolve().parent.parent
TAPE = ROOT / "tests" / "fixtures" / "regtest_tapes" / "bitcoin_core_address_baseline.json"
REGTEST_ADDRESS = "bcrt1qs758ursh4q9z627kt3pp5yysm78ddny6txaqgw"


def _import_hooks() -> core_imports.ImportCoordinatorHooks:
    return core_imports.ImportCoordinatorHooks(
        ensure_tag_row=lambda *args: None,
        invalidate_journals=invalidate_journals,
    )


class RegtestHarnessTest(unittest.TestCase):
    def test_no_egress_guard_blocks_only_non_loopback_connects(self):
        calls = []

        def fake_connect(_self, address):
            calls.append(("connect", address))
            return None

        def fake_connect_ex(_self, address):
            calls.append(("connect_ex", address))
            return 0

        with patch("socket.socket.connect", fake_connect), patch(
            "socket.socket.connect_ex",
            fake_connect_ex,
        ):
            with no_egress_guard(enabled=True):
                with self.assertRaises(AssertionError):
                    socket.socket.connect(object(), ("198.51.100.1", 443))
                socket.socket.connect(object(), ("127.0.0.1", 18443))
                socket.socket.connect_ex(object(), ("localhost", 18443))

        self.assertEqual(
            calls,
            [
                ("connect", ("127.0.0.1", 18443)),
                ("connect_ex", ("localhost", 18443)),
            ],
        )

    def test_recorded_tape_has_provenance_and_completeness_gate(self):
        tape = RecordedTape.load(TAPE)

        self.assertEqual(tape.provenance["kassiber_issue"], 312)
        self.assertEqual(tape.provenance["backend_kind"], "bitcoinrpc")
        with self.assertRaises(TapeMiss):
            tape.lookup("missing interaction")

    def test_full_accounting_demo_manifest_covers_expected_workflows(self):
        scenario = regtest_demo.load_scenario()

        self.assertEqual(scenario["id"], "full-accounting-v1")
        wallet_keys = {wallet["key"] for wallet in scenario["wallets"]}
        self.assertEqual(wallet_keys, {"treasury", "cold", "spending", "merchant"})
        operation_kinds = {operation["kind"] for operation in scenario["operations"]}
        self.assertTrue(
            {
                "payment",
                "self_transfer",
                "coinjoin_shape",
                "payjoin_shape",
                "loan_collateral_lock",
                "loan_collateral_release",
                "loan_principal_received",
                "loan_principal_repaid",
            }.issubset(operation_kinds)
        )
        self.assertGreaterEqual(scenario["expected"]["min_transactions"], 950)
        self.assertGreaterEqual(scenario["expected"]["min_active_transactions"], 940)
        base_time = datetime.fromisoformat(scenario["base_time"].replace("Z", "+00:00"))
        stress = scenario["stress"]
        self.assertTrue(stress["enabled"])
        self.assertGreaterEqual(stress["cycles"], 190)
        stress_span_days = stress["cycles"] * stress["days_between_cycles"]
        self.assertGreaterEqual(stress_span_days, 365 * 7)
        self.assertLess(base_time, datetime(2020, 1, 1, tzinfo=timezone.utc))
        self.assertLess(base_time + timedelta(days=stress_span_days), datetime(2026, 7, 1, tzinfo=timezone.utc))
        self.assertEqual(scenario["expected"]["collaborative_excluded"], 5)
        self.assertEqual(scenario["expected"]["min_transfer_pairs"], 2)
        self.assertEqual(scenario["expected"]["loan_marks"], 4)
        self.assertIn("full-report.xlsx", scenario["expected"]["export_files"])

    def test_full_accounting_demo_manifest_validation_fails_closed(self):
        scenario = regtest_demo.load_scenario()
        scenario.pop("operations")

        with self.assertRaisesRegex(ValueError, "operations"):
            regtest_demo.validate_scenario(scenario)

    def test_core_rpc_tape_replays_through_sync_journal_report_and_xlsx(self):
        tape_rpc = BitcoinRpcTape(RecordedTape.load(TAPE))
        with tempfile.TemporaryDirectory() as tmp:
            data_root = Path(tmp) / "data"
            export_path = Path(tmp) / "report.xlsx"
            conn = open_db(data_root)
            try:
                workspace = core_accounts.create_workspace(conn, "Regtest")
                profile = core_accounts.create_profile(
                    conn,
                    workspace["id"],
                    "Replay",
                    "EUR",
                    "FIFO",
                    "generic",
                    365,
                )
                core_accounts.create_backend(
                    conn,
                    "core-regtest",
                    "bitcoinrpc",
                    "http://127.0.0.1:18443",
                    chain="bitcoin",
                    network="regtest",
                    timeout=30,
                    config={"wallet": "kassiber-wallet-1", "username": "user", "password": "pass"},
                )
                wallet = core_wallets.create_wallet(
                    conn,
                    workspace["id"],
                    profile["id"],
                    "Core replay",
                    "address",
                    None,
                    {
                        "backend": "core-regtest",
                        "chain": "bitcoin",
                        "network": "regtest",
                        "addresses": [REGTEST_ADDRESS],
                    },
                )

                runtime_config = backend_config.merge_db_backends(
                    conn,
                    {
                        "env_file": str(data_root / "unused.env"),
                        "default_backend": "core-regtest",
                        "bootstrap_default_backend": "core-regtest",
                        "backends": {},
                        "bootstrap_backends": {},
                        "dotenv_backends": [],
                        "process_env_overrides": {"backends": {}, "default_backend": False},
                    },
                )
                profile_row = conn.execute("SELECT * FROM profiles WHERE id = ?", (profile["id"],)).fetchone()
                wallet_row = conn.execute("SELECT * FROM wallets WHERE id = ?", (wallet["id"],)).fetchone()
                hooks = core_sync.WalletSyncHooks(
                    import_file=lambda *args: {},
                    insert_records=lambda db, prof, wal, records, source_label: core_imports.insert_wallet_records(
                        db,
                        prof,
                        wal,
                        records,
                        source_label,
                        _import_hooks(),
                    ),
                    resolve_backend=backend_config.resolve_backend,
                    resolve_sync_state=core_sync_backends.resolve_wallet_sync_targets,
                    normalize_addresses=core_wallets.normalize_addresses,
                    backend_adapters={"bitcoinrpc": core_sync_backends.bitcoinrpc_sync_adapter},
                    update_output_inventory=lambda db, prof, wal, be, state, outputs: core_output_inventory.update_wallet_output_inventory(
                        db,
                        prof,
                        wal,
                        be,
                        state,
                        outputs,
                    ),
                )

                with no_egress_guard(enabled=True), patch(
                    "kassiber.core.sync_backends.bitcoinrpc_call",
                    tape_rpc.call,
                ):
                    outcome = core_sync.sync_wallet_from_backend(
                        conn,
                        runtime_config,
                        profile_row,
                        wallet_row,
                        hooks,
                    )
                    self.assertEqual(outcome["backend_kind"], "bitcoinrpc")
                    self.assertEqual(outcome["records_fetched"], 2)
                    self.assertEqual(outcome["imported"], 2)
                    self.assertEqual(outcome["bitcoinrpc_sync_mode"], "full_scan")
                    self.assertEqual(outcome["output_inventory"]["observed"], 1)
                    self.assertEqual(outcome["output_inventory"]["active"], 1)
                    self.assertIn("bitcoinrpc_last_block", outcome["freshness_checkpoint"])

                    tx_rows = conn.execute(
                        """
                        SELECT external_id, direction, amount, fee
                        FROM transactions
                        ORDER BY occurred_at
                        """
                    ).fetchall()
                    self.assertEqual(len(tx_rows), 2)
                    self.assertEqual(tx_rows[0]["direction"], "inbound")
                    self.assertEqual(tx_rows[0]["amount"], 25_000_000_000)
                    self.assertEqual(tx_rows[1]["direction"], "outbound")
                    self.assertEqual(tx_rows[1]["amount"], 5_000_000_000)
                    self.assertEqual(tx_rows[1]["fee"], 1_000_000)

                    utxo = conn.execute(
                        """
                        SELECT wu.txid, wu.vout, wu.amount, wu.block_height, wu.block_time, t.id AS transaction_id
                        FROM wallet_utxos wu
                        LEFT JOIN transactions t
                          ON t.profile_id = wu.profile_id
                         AND t.external_id = wu.txid
                        """
                    ).fetchone()
                    self.assertIsNotNone(utxo)
                    self.assertEqual(
                        utxo["txid"],
                        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    )
                    self.assertEqual(utxo["vout"], 0)
                    self.assertEqual(utxo["amount"], 19_999_000_000)
                    self.assertEqual(utxo["block_height"], 102)
                    self.assertEqual(utxo["block_time"], "2023-11-14T22:23:20Z")
                    self.assertIsNotNone(utxo["transaction_id"])

                    core_rates.set_manual_rate(conn, "BTC-EUR", "2023-11-14T22:13:20Z", "35000")
                    core_rates.set_manual_rate(conn, "BTC-EUR", "2023-11-14T22:23:20Z", "36000")

                    report_hooks = cli_report_hooks()
                    journal = process_journals(conn, workspace["id"], profile["id"])
                    self.assertEqual(journal["entries_created"], 2)
                    self.assertEqual(journal["quarantined"], 0)
                    self.assertEqual(journal["auto_priced"], 2)
                    self.assertEqual(journal["processed_transactions"], 2)

                    priced = conn.execute(
                        """
                        SELECT external_id, fiat_rate, fiat_value, pricing_source_kind, pricing_quality
                        FROM transactions
                        ORDER BY occurred_at
                        """
                    ).fetchall()
                    self.assertEqual([row["fiat_value"] for row in priced], [8750.0, 1800.0])
                    self.assertEqual({row["pricing_source_kind"] for row in priced}, {"manual_rate_cache"})
                    self.assertEqual({row["pricing_quality"] for row in priced}, {"exact"})

                    journal_rows = conn.execute(
                        """
                        SELECT entry_type, asset, quantity, fiat_value, cost_basis, proceeds, gain_loss
                        FROM journal_entries
                        ORDER BY occurred_at, entry_type
                        """
                    ).fetchall()
                    self.assertEqual(len(journal_rows), 2)
                    self.assertEqual(journal_rows[0]["entry_type"], "acquisition")
                    self.assertEqual(journal_rows[0]["quantity"], 25_000_000_000)
                    self.assertEqual(journal_rows[0]["fiat_value"], 8750.0)
                    self.assertEqual(journal_rows[1]["entry_type"], "disposal")
                    self.assertEqual(journal_rows[1]["quantity"], -5_001_000_000)
                    self.assertEqual(journal_rows[1]["proceeds"], 1800.0)
                    self.assertAlmostEqual(journal_rows[1]["cost_basis"], 1750.35, places=2)
                    self.assertAlmostEqual(journal_rows[1]["gain_loss"], 49.65, places=2)

                    summary = core_reports.report_summary(conn, workspace["id"], profile["id"], report_hooks)
                    self.assertEqual(summary["metrics"]["assets_in_scope"], 1)
                    self.assertEqual(summary["metrics"]["journal_entries"], 2)
                    self.assertEqual(summary["metrics"]["priced_transactions"], 2)
                    self.assertEqual(summary["metrics"]["quarantines"], 0)
                    self.assertAlmostEqual(summary["holdings"]["cost_basis"], 6999.65, places=2)
                    self.assertAlmostEqual(summary["realized"]["gain_loss"], 49.65, places=2)

                    export = core_reports.export_xlsx_report(
                        conn,
                        workspace["id"],
                        profile["id"],
                        str(export_path),
                        report_hooks,
                        verify=True,
                    )
                    self.assertEqual(export["file"], str(export_path))
                    self.assertTrue(export_path.exists())
                    self.assertTrue(export["verified"])

                    workbook = openpyxl.load_workbook(export_path, data_only=False, read_only=True)
                    self.assertIn("Verify", workbook.sheetnames)
                    self.assertIn("Control", workbook.sheetnames)
                    self.assertIn("Acquisitions", workbook.sheetnames)
                    self.assertIn("Disposals", workbook.sheetnames)
                    self.assertEqual(workbook["Verify"]["A2"].value, "Verification status")
                    self.assertIn("ALL CHECKS OK", workbook["Verify"]["B2"].value)
                    self.assertEqual(workbook["Acquisitions"]["C3"].value, tx_rows[0]["external_id"])
                    self.assertEqual(workbook["Acquisitions"]["F3"].value, 25_000_000_000)
                    self.assertEqual(workbook["Acquisitions"]["H3"].value, 8750)
                    self.assertEqual(workbook["Disposals"]["C3"].value, tx_rows[1]["external_id"])
                    self.assertEqual(workbook["Disposals"]["F3"].value, 5_001_000_000)
                    self.assertEqual(workbook["Disposals"]["K3"].value, 49.65)
                    self.assertIn('"OK"', workbook["Disposals"]["L3"].value)
                    self.assertEqual(workbook["Control"]["A3"].value, "BTC")
                    self.assertEqual(workbook["Control"]["F3"].value, 0.19999)
                    self.assertEqual(workbook["Control"]["I3"].value, 6999.65)
                    self.assertIn('"OK"', workbook["Control"]["G3"].value)
                    workbook.close()

                    # Second run feeds the first checkpoint back into Core RPC and
                    # proves the incremental listsinceblock path plus DB idempotency.
                    repeat_wallet_row = conn.execute(
                        "SELECT * FROM wallets WHERE id = ?", (wallet["id"],)
                    ).fetchone()
                    repeat = core_sync.sync_wallet_from_backend(
                        conn,
                        runtime_config,
                        profile_row,
                        repeat_wallet_row,
                        hooks,
                        checkpoint=outcome["freshness_checkpoint"],
                    )
                self.assertEqual(repeat["imported"], 0)
                self.assertEqual(repeat["records_fetched"], 0)
                self.assertEqual(repeat["bitcoinrpc_sync_mode"], "sinceblock")
                count = conn.execute("SELECT COUNT(*) AS count FROM transactions").fetchone()["count"]
                self.assertEqual(count, 2)
                self.assertEqual(tape_rpc.unused_interactions(), [])
            finally:
                conn.close()


if __name__ == "__main__":
    unittest.main()
