import tempfile
import unittest
from pathlib import Path

from kassiber.backends import get_db_backend
from kassiber.core import accounts as core_accounts
from kassiber.core import imports as core_imports
from kassiber.core import lightning_cln
from kassiber.core import wallets as core_wallets
from kassiber.core.repo import fetch_wallet_with_account, invalidate_journals
from kassiber.db import open_db
from kassiber.errors import AppError


class CoreLightningSyncTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.data_root = Path(self._tmp.name) / "data"
        self.conn = open_db(self.data_root)
        workspace = core_accounts.create_workspace(self.conn, "Personal")
        self.profile = core_accounts.create_profile(
            self.conn,
            workspace["id"],
            "Main",
            "USD",
            "FIFO",
            "generic",
            365,
        )
        core_accounts.create_backend(
            self.conn,
            "cln",
            "coreln",
            "cln://local",
            token="readonly-rune",
            config={"commando_peer_id": "02" + "ab" * 32},
        )
        created_wallet = core_wallets.create_wallet(
            self.conn,
            workspace["id"],
            self.profile["id"],
            "Routing node",
            "coreln",
            config={"backend": "cln"},
        )
        self.wallet = fetch_wallet_with_account(self.conn, created_wallet["id"])
        self.backend = get_db_backend(self.conn, "cln")

    def tearDown(self):
        self.conn.close()
        self._tmp.cleanup()

    def _hooks(self):
        return core_imports.ImportCoordinatorHooks(
            ensure_tag_row=lambda *_args, **_kwargs: None,
            invalidate_journals=invalidate_journals,
        )

    def test_sync_stores_cln_records_imports_bookkeeper_income_and_reports_profitability(self):
        payloads = {
            "getinfo": {"id": "03" + "cd" * 32, "alias": "k-node"},
            "bkpr-listaccountevents": {
                "events": [
                    {
                        "account": "123x1x0",
                        "type": "channel",
                        "tag": "rebalance_fee",
                        "debit_msat": "3000msat",
                        "credit_msat": "0msat",
                        "timestamp": 1_700_000_010,
                        "is_rebalance": True,
                    }
                ]
            },
            "bkpr-listincome": {
                "income_events": [
                    {
                        "account": "123x1x0",
                        "tag": "routed",
                        "credit_msat": "1500msat",
                        "debit_msat": "0msat",
                        "currency": "bc",
                        "timestamp": 1_700_000_000,
                        "payment_id": "11" * 32,
                    }
                ]
            },
            "bkpr-listbalances": {
                "accounts": [
                    {
                        "account": "123x1x0",
                        "peer_id": "02" + "ef" * 32,
                        "balances": [{"balance_msat": "100000msat"}],
                    }
                ]
            },
            "listfunds": {
                "outputs": [
                    {
                        "txid": "aa" * 32,
                        "output": 0,
                        "amount_msat": "200000msat",
                        "status": "confirmed",
                    }
                ],
                "channels": [
                    {
                        "short_channel_id": "123x1x0",
                        "our_amount_msat": "100000msat",
                        "state": "CHANNELD_NORMAL",
                    }
                ],
            },
            "listforwards": {
                "forwards": [
                    {
                        "in_channel": "111x1x0",
                        "out_channel": "123x1x0",
                        "fee_msat": "2000msat",
                        "out_msat": "50000msat",
                        "status": "settled",
                        "resolved_time": 1_700_000_030,
                    }
                ]
            },
            "listpays": {
                "pays": [
                    {
                        "payment_hash": "22" * 32,
                        "amount_msat": "40000msat",
                        "amount_sent_msat": "40500msat",
                        "status": "complete",
                        "created_at": 1_700_000_040,
                    }
                ]
            },
            "listinvoices": {"invoices": []},
            "listtransactions": {"transactions": [{"hash": "bb" * 32, "blocktime": 1_700_000_050}]},
            "listpeerchannels": {"channels": []},
        }

        def rpc(method, _args=None):
            return payloads[method]

        outcome = lightning_cln.sync_core_lightning_wallet(
            self.conn,
            self.profile,
            self.wallet,
            self.backend,
            self._hooks(),
            rpc_call=rpc,
        )
        self.assertEqual(outcome["status"], "synced")
        self.assertGreaterEqual(outcome["records_fetched"], 7)
        self.assertEqual(outcome["transactions"]["imported"], 1)

        records = self.conn.execute("SELECT record_type FROM lightning_node_records").fetchall()
        self.assertIn("income", {row["record_type"] for row in records})
        self.assertIn("forward", {row["record_type"] for row in records})

        report = lightning_cln.report_lightning_profitability(
            self.conn,
            None,
            None,
            wallet_ref="Routing node",
        )
        self.assertEqual(report["routing_revenue_msat"], 2000)
        self.assertEqual(report["payment_cost_msat"], 500)
        self.assertEqual(report["rebalance_cost_msat"], 3000)
        self.assertEqual(report["net_routing_profit_msat"], -1500)
        self.assertEqual(report["channels"][0]["channel_id"], "123x1x0")

    def test_commando_backend_requires_rune_and_peer_id_together(self):
        with self.assertRaises(AppError) as exc:
            lightning_cln.call_core_lightning(
                {
                    "name": "cln",
                    "kind": "coreln",
                    "url": "cln://commando",
                    "commando_peer_id": "02" + "ab" * 32,
                },
                "getinfo",
            )
        self.assertIn("requires both", str(exc.exception))


if __name__ == "__main__":
    unittest.main()
