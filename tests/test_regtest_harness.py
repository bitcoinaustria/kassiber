from __future__ import annotations

import json
import socket
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import patch

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

                backend = {
                    "name": "core-regtest",
                    "kind": "bitcoinrpc",
                    "url": "http://127.0.0.1:18443",
                    "chain": "bitcoin",
                    "network": "regtest",
                    "wallet": "kassiber-wallet-1",
                    "username": "user",
                    "password": "pass",
                }
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
                    resolve_backend=lambda runtime_config, backend_name: backend,
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
                        {},
                        profile_row,
                        wallet_row,
                        hooks,
                    )

                self.assertEqual(outcome["backend_kind"], "bitcoinrpc")
                self.assertEqual(outcome["records_fetched"], 2)
                self.assertEqual(outcome["imported"], 2)
                self.assertEqual(outcome["output_inventory"]["observed"], 1)
                self.assertEqual(outcome["output_inventory"]["active"], 1)
                self.assertIn("bitcoinrpc_last_block", outcome["freshness_checkpoint"])

                tx_rows = conn.execute(
                    "SELECT external_id, direction, amount, fee FROM transactions ORDER BY occurred_at"
                ).fetchall()
                self.assertEqual(len(tx_rows), 2)
                self.assertEqual(tx_rows[0]["direction"], "inbound")
                self.assertEqual(tx_rows[1]["direction"], "outbound")

                core_rates.set_manual_rate(conn, "BTC-EUR", "2023-11-14T22:13:20Z", "35000")
                core_rates.set_manual_rate(conn, "BTC-EUR", "2023-11-14T22:23:20Z", "36000")

                report_hooks = cli_report_hooks()
                journal = process_journals(conn, workspace["id"], profile["id"])
                self.assertGreaterEqual(journal["entries_created"], 1)

                summary = core_reports.report_summary(conn, workspace["id"], profile["id"], report_hooks)
                self.assertEqual(summary["metrics"]["assets_in_scope"], 1)
                self.assertGreaterEqual(summary["metrics"]["journal_entries"], 1)

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

                with zipfile.ZipFile(export_path) as archive:
                    names = set(archive.namelist())
                    workbook_xml = archive.read("xl/workbook.xml").decode("utf-8")
                    shared_strings = archive.read("xl/sharedStrings.xml").decode("utf-8")
                self.assertIn("xl/worksheets/sheet1.xml", names)
                self.assertIn('name="Verify"', workbook_xml)
                self.assertIn('name="Control"', workbook_xml)
                self.assertIn("Kassiber", shared_strings)

                # Second run proves replay + DB import idempotency for duplicate backend rows.
                tape_rpc_repeat = BitcoinRpcTape(RecordedTape.load(TAPE))
                with no_egress_guard(enabled=True), patch(
                    "kassiber.core.sync_backends.bitcoinrpc_call",
                    tape_rpc_repeat.call,
                ):
                    repeat = core_sync.sync_wallet_from_backend(
                        conn,
                        {},
                        profile_row,
                        wallet_row,
                        hooks,
                    )
                self.assertEqual(repeat["imported"], 0)
                self.assertEqual(repeat["skipped"], 2)
                count = conn.execute("SELECT COUNT(*) AS count FROM transactions").fetchone()["count"]
                self.assertEqual(count, 2)
            finally:
                conn.close()


if __name__ == "__main__":
    unittest.main()
