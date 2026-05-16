import json
import sqlite3
import tempfile
import unittest
from pathlib import Path

from kassiber.core import commercial
from kassiber.db import open_db
from kassiber.msat import btc_to_msat
from kassiber.sync_btcpay import fetch_btcpay_invoice_provenance
from kassiber.time_utils import now_iso


class _Response:
    def __init__(self, payload):
        self.payload = payload

    def __enter__(self):
        return self

    def __exit__(self, *args):
        return False

    def read(self):
        return json.dumps(self.payload).encode("utf-8")


class _Opener:
    def __init__(self, pages):
        self.pages = list(pages)
        self.urls = []

    def open(self, request, timeout=None):
        self.urls.append(request.full_url)
        return _Response(self.pages.pop(0))


def _hooks():
    def resolve_scope(conn, workspace_ref=None, profile_ref=None):
        workspace = conn.execute("SELECT * FROM workspaces LIMIT 1").fetchone()
        profile = conn.execute("SELECT * FROM profiles LIMIT 1").fetchone()
        return workspace, profile

    def resolve_transaction(conn, profile_id, tx_ref):
        return conn.execute(
            "SELECT * FROM transactions WHERE profile_id = ? AND id = ?",
            (profile_id, tx_ref),
        ).fetchone()

    def invalidate_journals(conn, profile_id):
        conn.execute(
            "UPDATE profiles SET journal_input_version = journal_input_version + 1 WHERE id = ?",
            (profile_id,),
        )

    return commercial.CommercialHooks(
        resolve_scope=resolve_scope,
        resolve_transaction=resolve_transaction,
        invalidate_journals=invalidate_journals,
    )


class BtcpayCommercialProvenanceTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-btcpay-commercial-")
        self.data_root = Path(self.tmp.name) / "data"
        self.conn = open_db(str(self.data_root))
        self.conn.row_factory = sqlite3.Row
        now = now_iso()
        self.conn.execute("INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Default', ?)", (now,))
        self.conn.execute(
            """
            INSERT INTO profiles(id, workspace_id, label, fiat_currency, created_at)
            VALUES('prof', 'ws', 'Business', 'EUR', ?)
            """,
            (now,),
        )
        self.conn.execute(
            """
            INSERT INTO wallets(id, workspace_id, profile_id, label, kind, created_at)
            VALUES('wallet', 'ws', 'prof', 'BTCPay', 'custom', ?)
            """,
            (now,),
        )
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset, amount, fee, fiat_currency,
                kind, raw_json, created_at
            ) VALUES(
                'tx', 'ws', 'prof', 'wallet',
                'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                'fp-commercial-1', '2026-01-01T12:00:00Z', 'inbound',
                'BTC', ?, 0, 'EUR', 'deposit', '{}', ?
            )
            """,
            (btcpay_to_msat("0.01000000"), now),
        )
        self.conn.commit()

    def tearDown(self):
        self.conn.close()
        self.tmp.cleanup()

    def test_fetch_invoice_provenance_normalizes_invoice_and_payment(self):
        opener = _Opener(
            [
                [
                    {
                        "id": "inv-1",
                        "orderId": "order-1",
                        "status": "Settled",
                        "createdTime": 1760000000,
                        "currency": "EUR",
                        "amount": "500.00",
                        "payments": [
                            {
                                "id": "pay-1",
                                "paymentMethod": "BTC-CHAIN",
                                "value": "0.01000000",
                                "rate": "50000.00",
                                "receivedDate": 1760000060,
                                "transactionId": "a" * 64,
                            }
                        ],
                    }
                ]
            ]
        )
        backend = {"url": "https://btcpay.example", "token": "secret", "timeout": 5}

        invoices = fetch_btcpay_invoice_provenance(
            backend,
            "store-1",
            page_size=100,
            opener=opener,
        )

        self.assertEqual(len(invoices), 1)
        self.assertEqual(invoices[0]["invoice_id"], "inv-1")
        self.assertEqual(invoices[0]["payments"][0]["txid"], "a" * 64)
        self.assertIn("/api/v1/stores/store-1/invoices?", opener.urls[0])

    def test_reviewed_link_applies_btcpay_price_and_commercial_kind(self):
        workspace, profile = _hooks().resolve_scope(self.conn)
        commercial.upsert_btcpay_provenance(
            self.conn,
            workspace,
            profile,
            backend_name="btcpay-prod",
            invoices=[
                {
                    "store_id": "store-1",
                    "invoice_id": "inv-1",
                    "order_id": "order-1",
                    "status": "Settled",
                    "created_at": "2026-01-01T11:59:00Z",
                    "currency": "EUR",
                    "amount": "500.00",
                    "invoice": {"id": "inv-1"},
                    "payments": [
                        {
                            "payment_id": "pay-1",
                            "payment_method_id": "BTC-CHAIN",
                            "status": "Settled",
                            "received_at": "2026-01-01T12:00:00Z",
                            "amount": "0.01000000",
                            "rate": "50000.00",
                            "txid": "a" * 64,
                            "invoice_currency": "EUR",
                            "invoice_amount": "500.00",
                            "payment": {"id": "pay-1"},
                        }
                    ],
                }
            ],
        )
        document = commercial.create_document(
            self.conn,
            None,
            None,
            _hooks(),
            document_type="invoice",
            label="Invoice 2026-001",
            external_ref="inv-1",
            fiat_currency="EUR",
            fiat_value="500.00",
        )

        suggested = commercial.suggest_links(self.conn, None, None, _hooks())
        self.assertGreaterEqual(suggested["created"], 2)
        combined = [row for row in suggested["suggestions"] if row["document_id"] == document["id"] and row["transaction_id"] == "tx"]
        self.assertEqual(len(combined), 1)
        link_id = combined[0]["id"]

        reviewed = commercial.review_link(
            self.conn,
            None,
            None,
            link_id,
            _hooks(),
            state="reviewed",
            commercial_kind="income",
        )

        self.assertTrue(reviewed["applied_to_transaction"])
        tx = self.conn.execute(
            """
            SELECT kind, pricing_source_kind, pricing_quality, pricing_external_ref,
                   fiat_value_exact, fiat_rate_exact
            FROM transactions WHERE id = 'tx'
            """
        ).fetchone()
        self.assertEqual(tx["kind"], "income")
        self.assertEqual(tx["pricing_source_kind"], "btcpay_payment")
        self.assertEqual(tx["pricing_quality"], "exact")
        self.assertEqual(tx["fiat_value_exact"], "500.00")
        self.assertEqual(tx["fiat_rate_exact"], "50000.00")
        self.assertIn("invoice:inv-1:payment:pay-1", tx["pricing_external_ref"])
        profile_row = self.conn.execute("SELECT journal_input_version FROM profiles WHERE id = 'prof'").fetchone()
        self.assertEqual(profile_row["journal_input_version"], 1)
        subledger = commercial.build_reviewed_subledger_rows(self.conn, None, None, _hooks())
        self.assertEqual(subledger[0]["document_label"], "Invoice 2026-001")
        self.assertEqual(subledger[0]["invoice_id"], "inv-1")

    def test_external_document_evidence_reuses_attachment_store_without_transaction(self):
        document = commercial.create_document(
            self.conn,
            None,
            None,
            _hooks(),
            document_type="invoice",
            label="Invoice 2026-001",
            external_ref="inv-1",
            fiat_currency="EUR",
            fiat_value="500.00",
        )
        evidence = Path(self.tmp.name) / "invoice.txt"
        evidence.write_text("invoice evidence\n", encoding="utf-8")

        result = commercial.attach_document_evidence(
            self.conn,
            str(self.data_root),
            None,
            None,
            document["id"],
            _hooks(),
            file_path=str(evidence),
        )

        attachment = self.conn.execute(
            "SELECT transaction_id, stored_relpath FROM attachments WHERE id = ?",
            (result["attachment_id"],),
        ).fetchone()
        self.assertIsNone(attachment["transaction_id"])
        self.assertTrue(attachment["stored_relpath"])
        linked = self.conn.execute(
            """
            SELECT 1 FROM external_document_attachments
            WHERE document_id = ? AND attachment_id = ?
            """,
            (document["id"], result["attachment_id"]),
        ).fetchone()
        self.assertIsNotNone(linked)


def btcpay_to_msat(value):
    return btc_to_msat(value)


if __name__ == "__main__":
    unittest.main()
