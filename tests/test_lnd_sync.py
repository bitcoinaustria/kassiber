import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from kassiber.core import accounts as core_accounts
from kassiber.core import lnd as core_lnd
from kassiber.db import open_db


class _FakeLndClient:
    requests = []

    def __init__(self, backend):
        self.backend = backend

    def get(self, path, *, params=None):
        self.requests.append(("GET", path, params, self.backend.get("token")))
        payloads = {
            "/v1/getinfo": {
                "identity_pubkey": "02" + "aa" * 32,
                "alias": "routing-node",
                "synced_to_chain": True,
                "synced_to_graph": True,
            },
            "/v1/channels": {
                "channels": [
                    {
                        "active": True,
                        "chan_id": "123",
                        "channel_point": "fundingtx:0",
                        "remote_pubkey": "03" + "bb" * 32,
                        "capacity": "1000000",
                        "local_balance": "700000",
                        "remote_balance": "300000",
                        "commit_fee": "250",
                    }
                ]
            },
            "/v1/channels/closed": {"channels": []},
            "/v1/payments": {
                "last_index_offset": "7",
                "payments": [
                    {
                        "payment_index": "7",
                        "payment_hash": "hash-1",
                        "creation_date": "1700000010",
                        "status": "SUCCEEDED",
                        "value_msat": "1000000",
                        "fee_msat": "2000",
                    }
                ],
            },
            "/v1/invoices": {
                "last_index_offset": "9",
                "invoices": [
                    {
                        "add_index": "9",
                        "r_hash": "invoice-hash",
                        "creation_date": "1700000020",
                        "settle_date": "1700000030",
                        "settled": True,
                        "value_msat": "3000000",
                        "amt_paid_msat": "3000000",
                        "memo": "settled invoice",
                    }
                ],
            },
            "/v1/transactions": {
                "transactions": [
                    {
                        "tx_hash": "fundingtx",
                        "time_stamp": "1700000000",
                        "block_height": 800000,
                        "amount": "-1000000",
                        "total_fees": "300",
                    }
                ]
            },
            "/v1/balance/blockchain": {"confirmed_balance": "500000"},
            "/v1/balance/channels": {
                "local_balance": {"sat": "700000"},
                "remote_balance": {"sat": "300000"},
            },
            "/v1/fees": {
                "day_fee_sum": "10",
                "week_fee_sum": "20",
                "month_fee_sum": "30",
            },
        }
        return payloads.get(path, {})

    def post(self, path, payload=None):
        self.requests.append(("POST", path, payload, self.backend.get("token")))
        if path == "/v1/switch":
            return {
                "last_offset_index": "5",
                "forwarding_events": [
                    {
                        "timestamp": "1700000040",
                        "chan_id_in": "111",
                        "chan_id_out": "123",
                        "amt_in_msat": "1000000",
                        "amt_out_msat": "990000",
                        "fee_msat": "10000",
                    }
                ],
            }
        return {}


class LndSyncTest(unittest.TestCase):
    def test_lnd_sync_is_idempotent_and_powers_profitability_report(self):
        with tempfile.TemporaryDirectory() as tmp:
            data_root = Path(tmp) / "data"
            conn = open_db(data_root)
            workspace = core_accounts.create_workspace(conn, "Main")
            profile = core_accounts.create_profile(
                conn,
                "Main",
                "Default",
                "USD",
                "FIFO",
                "generic",
                365,
            )
            redacted = core_accounts.create_backend(
                conn,
                "node",
                "lnd",
                "https://127.0.0.1:8080",
                chain="bitcoin",
                network="main",
                token="00aa",
                config={"certificate": "/tmp/tls.cert"},
            )
            self.assertTrue(redacted["has_token"])
            self.assertTrue(redacted["has_certificate"])
            self.assertNotIn("00aa", json.dumps(redacted))
            backend = {
                "name": "node",
                "kind": "lnd",
                "url": "https://127.0.0.1:8080",
                "token": "00aa",
                "config": {"certificate": "/tmp/tls.cert"},
            }

            _FakeLndClient.requests = []
            with patch("kassiber.core.lnd.LndRestClient", _FakeLndClient):
                first = core_lnd.sync_lnd_backend(conn, workspace, profile, backend)
                second = core_lnd.sync_lnd_backend(conn, workspace, profile, backend)

            self.assertEqual(first["datasets"]["forwards"], 1)
            self.assertEqual(second["datasets"]["payments"], 1)
            status = core_lnd.lnd_status(conn, profile, "node")
            self.assertEqual(status["counts"]["forwards"], 1)
            self.assertEqual(status["counts"]["payments"], 1)
            self.assertEqual(status["counts"]["channels"], 1)

            report = core_lnd.lnd_profitability_report(conn, profile, "node")
            self.assertEqual(report["summary"]["routing_fees_earned_msat"], 10000)
            self.assertEqual(report["summary"]["payment_fees_paid_msat"], 2000)
            self.assertEqual(report["summary"]["wallet_fees_paid_msat"], 300000)
            self.assertEqual(len(report["channels"]), 1)
            self.assertTrue(all(req[3] == "00aa" for req in _FakeLndClient.requests))

            csv_path = Path(tmp) / "lightning.csv"
            exported = core_lnd.export_lnd_profitability_csv(conn, profile, str(csv_path), backend_name="node")
            self.assertEqual(exported["rows"], 3)
            self.assertIn("routing_fee", csv_path.read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
