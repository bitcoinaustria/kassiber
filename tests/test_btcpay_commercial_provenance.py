import json
import sqlite3
import tempfile
import unittest
from pathlib import Path

from kassiber.core import commercial
from kassiber.db import _migrate_nullable_attachment_transactions, open_db
from kassiber.errors import AppError
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

    def _upsert_invoice_payment(self, *, raw_payment=None, asset="BTC"):
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
                            "payment_method_id": f"{asset}-CHAIN",
                            "status": "Settled",
                            "received_at": "2026-01-01T12:00:00Z",
                            "amount": "0.01000000",
                            "rate": "50000.00",
                            "txid": "a" * 64,
                            "invoice_currency": "EUR",
                            "invoice_amount": "500.00",
                            "payment": raw_payment or {"id": "pay-1"},
                        }
                    ],
                }
            ],
        )

    def _create_matching_document(self):
        return commercial.create_document(
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

    def _suggested_transaction_link_id(self):
        suggested = commercial.suggest_links(self.conn, None, None, _hooks())
        links = [
            row
            for row in suggested["suggestions"]
            if row["link_type"] == "btcpay_payment_transaction"
            and row["transaction_id"] == "tx"
        ]
        self.assertEqual(len(links), 1)
        return links[0]["id"]

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

    def test_rejecting_reviewed_link_restores_transaction_snapshot(self):
        self._upsert_invoice_payment()
        self._create_matching_document()
        link_id = self._suggested_transaction_link_id()

        commercial.review_link(
            self.conn,
            None,
            None,
            link_id,
            _hooks(),
            state="reviewed",
            commercial_kind="income",
        )
        reverted = commercial.review_link(
            self.conn,
            None,
            None,
            link_id,
            _hooks(),
            state="rejected",
        )

        self.assertTrue(reverted["restored_transaction"])
        tx = self.conn.execute(
            """
            SELECT kind, pricing_source_kind, fiat_value_exact, commercial_applied_link_id
            FROM transactions WHERE id = 'tx'
            """
        ).fetchone()
        self.assertEqual(tx["kind"], "deposit")
        self.assertIsNone(tx["pricing_source_kind"])
        self.assertIsNone(tx["fiat_value_exact"])
        self.assertIsNone(tx["commercial_applied_link_id"])
        profile_row = self.conn.execute("SELECT journal_input_version FROM profiles WHERE id = 'prof'").fetchone()
        self.assertEqual(profile_row["journal_input_version"], 2)

    def test_review_freezes_btcpay_raw_snapshot_across_resync(self):
        self._upsert_invoice_payment(raw_payment={"id": "pay-1", "rate": "50000.00"})
        link_id = self._suggested_transaction_link_id()

        reviewed = commercial.review_link(
            self.conn,
            None,
            None,
            link_id,
            _hooks(),
            state="reviewed",
            commercial_kind="income",
        )
        frozen_hash = reviewed["reviewed_record_snapshot_sha256"]
        self._upsert_invoice_payment(raw_payment={"id": "pay-1", "rate": "51000.00"})

        link = commercial.list_links(self.conn, None, None, _hooks(), state="reviewed")[0]
        self.assertEqual(link["reviewed_record_snapshot_sha256"], frozen_hash)
        stored = self.conn.execute(
            "SELECT reviewed_record_snapshot_json FROM commercial_links WHERE id = ?",
            (link_id,),
        ).fetchone()
        self.assertIn("50000.00", stored["reviewed_record_snapshot_json"])

    def test_ambiguous_payment_match_is_not_suggested_or_reviewable(self):
        now = now_iso()
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset, amount, fee, fiat_currency,
                kind, raw_json, created_at
            ) VALUES(
                'tx-duplicate', 'ws', 'prof', 'wallet',
                'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                'fp-commercial-duplicate', '2026-01-01T12:00:01Z', 'inbound',
                'BTC', ?, 0, 'EUR', 'deposit', '{}', ?
            )
            """,
            (btcpay_to_msat("0.01000000"), now),
        )
        self.conn.commit()
        self._upsert_invoice_payment()

        suggested = commercial.suggest_links(self.conn, None, None, _hooks())
        self.assertEqual(
            [row for row in suggested["suggestions"] if row["link_type"] == "btcpay_payment_transaction"],
            [],
        )
        workspace, profile = _hooks().resolve_scope(self.conn)
        record = self.conn.execute(
            "SELECT id FROM btcpay_provenance_records WHERE record_type = 'payment'"
        ).fetchone()
        link = commercial._upsert_link(
            self.conn,
            workspace,
            profile,
            btcpay_record_id=record["id"],
            document_id=None,
            transaction_id="tx",
            link_type="btcpay_payment_transaction",
            state="suggested",
            confidence="weak",
            method="test",
            allocation_amount=btcpay_to_msat("0.01000000"),
            allocation_fiat_exact="500.00",
            reconciliation_state="unreviewed",
            commercial_kind=None,
            notes=None,
            now=now,
        )["link"]
        with self.assertRaises(AppError) as raised:
            commercial.review_link(
                self.conn,
                None,
                None,
                link["id"],
                _hooks(),
                state="reviewed",
                commercial_kind="income",
            )
        self.assertEqual(raised.exception.code, "ambiguous")

    def test_review_rejects_asset_direction_currency_and_transfer_pair_mismatch(self):
        self._upsert_invoice_payment(asset="LBTC")
        link_id = self._suggested_transaction_link_id()
        with self.assertRaises(AppError):
            commercial.review_link(
                self.conn,
                None,
                None,
                link_id,
                _hooks(),
                state="reviewed",
                commercial_kind="income",
            )

        self.conn.execute("UPDATE btcpay_provenance_records SET asset = 'BTC', fiat_currency = 'USD'")
        with self.assertRaises(AppError):
            commercial.review_link(
                self.conn,
                None,
                None,
                link_id,
                _hooks(),
                state="reviewed",
                commercial_kind="income",
            )

        self.conn.execute("UPDATE btcpay_provenance_records SET fiat_currency = 'EUR'")
        with self.assertRaises(AppError):
            commercial.review_link(
                self.conn,
                None,
                None,
                link_id,
                _hooks(),
                state="reviewed",
                commercial_kind="expense",
            )

        now = now_iso()
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset, amount, fee, fiat_currency,
                kind, raw_json, created_at
            ) VALUES(
                'tx-out', 'ws', 'prof', 'wallet', 'out-ref', 'fp-out',
                '2026-01-01T12:00:02Z', 'outbound',
                'BTC', ?, 0, 'EUR', 'withdrawal', '{}', ?
            )
            """,
            (-btcpay_to_msat("0.01000000"), now),
        )
        self.conn.execute(
            """
            INSERT INTO transaction_pairs(
                id, workspace_id, profile_id, out_transaction_id, in_transaction_id,
                kind, policy, created_at
            ) VALUES('pair-1', 'ws', 'prof', 'tx-out', 'tx', 'manual', 'carrying-value', ?)
            """,
            (now,),
        )
        self.conn.commit()
        with self.assertRaises(AppError):
            commercial.review_link(
                self.conn,
                None,
                None,
                link_id,
                _hooks(),
                state="reviewed",
                commercial_kind="income",
            )

    def test_payment_transaction_link_is_not_duplicated_when_document_later_matches(self):
        self._upsert_invoice_payment()
        first = commercial.suggest_links(self.conn, None, None, _hooks())
        self.assertEqual(
            len([row for row in first["suggestions"] if row["link_type"] == "btcpay_payment_transaction"]),
            1,
        )
        document = self._create_matching_document()
        second = commercial.suggest_links(self.conn, None, None, _hooks())

        links = commercial.list_links(self.conn, None, None, _hooks())
        payment_links = [row for row in links if row["link_type"] == "btcpay_payment_transaction"]
        self.assertEqual(len(payment_links), 1)
        self.assertEqual(payment_links[0]["document_id"], document["id"])
        self.assertEqual(
            len([row for row in second["suggestions"] if row["link_type"] == "btcpay_payment_transaction"]),
            1,
        )

    def test_document_reference_resolution_rejects_ambiguous_labels_and_duplicate_refs(self):
        commercial.create_document(
            self.conn,
            None,
            None,
            _hooks(),
            document_type="invoice",
            label="Invoice duplicate",
        )
        commercial.create_document(
            self.conn,
            None,
            None,
            _hooks(),
            document_type="receipt",
            label="Invoice duplicate",
        )

        with self.assertRaises(AppError) as raised:
            commercial.attach_document_evidence(
                self.conn,
                str(self.data_root),
                None,
                None,
                "Invoice duplicate",
                _hooks(),
                url="https://example.test/invoice",
            )
        self.assertEqual(raised.exception.code, "ambiguous")

        commercial.create_document(
            self.conn,
            None,
            None,
            _hooks(),
            document_type="invoice",
            label="Invoice unique",
            external_ref="unique-ref",
        )
        with self.assertRaises(AppError) as duplicate_ref:
            commercial.create_document(
                self.conn,
                None,
                None,
                _hooks(),
                document_type="invoice",
                label="Invoice unique 2",
                external_ref="unique-ref",
            )
        self.assertEqual(duplicate_ref.exception.code, "conflict")

    def test_nullable_attachment_migration_recovers_stranded_legacy_table(self):
        document = self._create_matching_document()
        result = commercial.attach_document_evidence(
            self.conn,
            str(self.data_root),
            None,
            None,
            document["id"],
            _hooks(),
            url="https://example.test/invoice",
        )
        self.conn.execute("ALTER TABLE attachments RENAME TO attachments_legacy_notnull_tx")
        self.conn.execute(
            """
            CREATE TABLE attachments (
                id TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
                transaction_id TEXT REFERENCES transactions(id) ON DELETE CASCADE,
                attachment_type TEXT NOT NULL,
                label TEXT NOT NULL,
                original_filename TEXT,
                stored_relpath TEXT,
                source_url TEXT,
                media_type TEXT,
                size_bytes INTEGER,
                sha256 TEXT,
                created_at TEXT NOT NULL
            )
            """
        )
        self.conn.commit()

        _migrate_nullable_attachment_transactions(self.conn)

        attachment = self.conn.execute(
            "SELECT source_url FROM attachments WHERE id = ?",
            (result["attachment_id"],),
        ).fetchone()
        self.assertEqual(attachment["source_url"], "https://example.test/invoice")
        legacy = self.conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='attachments_legacy_notnull_tx'"
        ).fetchone()
        self.assertIsNone(legacy)

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
