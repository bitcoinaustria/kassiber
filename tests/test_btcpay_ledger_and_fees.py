"""BTCPay as a balance source: payout fees, batched payouts, payment ledgers, staleness."""

import json
import sqlite3
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from urllib.parse import parse_qs, urlsplit

from kassiber import sync_btcpay
from kassiber.btcpay.discovery import InspectionResult
from kassiber.cli import handlers
from kassiber.core import btcpay_ledger, btcpay_setup, commercial
from kassiber.core import imports as core_imports
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.fingerprints import make_transaction_fingerprint
from kassiber.importers import normalize_btcpay_record
from kassiber.time_utils import now_iso

from tests.test_sync_btcpay_greenfield import _RoutedOpener

TXID = "ab" * 32
HASH_PAID = "cd" * 32
HASH_REFUND = "ef" * 32
PAYOUT_DATE = 1767312000  # 2026-01-02T00:00:00Z


def _first_page(rows):
    return lambda url: rows if parse_qs(urlsplit(url).query).get("skip") == ["0"] else []


def _payout(payout_id, *, state="Completed", sats, eur, txid=TXID, method="BTC-CHAIN", proof=None):
    return {
        "id": payout_id,
        "pullPaymentId": f"pp-{payout_id}",
        "payoutMethodId": method,
        "originalAmount": eur,
        "originalCurrency": "EUR",
        "payoutCurrency": "BTC",
        "payoutAmount": format(sats / 100_000_000, ".8f"),
        "state": state,
        "date": PAYOUT_DATE,
        "paymentProof": proof or {"ProofType": "PayoutTransactionOnChainBlob", "TransactionId": txid},
    }


class _BookTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-btcpay-ledger-")
        self.conn = open_db(str(Path(self.tmp.name) / "data"))
        self.conn.row_factory = sqlite3.Row
        now = now_iso()
        self.conn.execute("INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Shop', ?)", (now,))
        self.conn.execute(
            "INSERT INTO profiles(id, workspace_id, label, fiat_currency, created_at) VALUES('prof', 'ws', 'Main', 'EUR', ?)",
            (now,),
        )
        self.conn.execute(
            "INSERT INTO accounts(id, workspace_id, profile_id, code, label, account_type, created_at) "
            "VALUES('acct', 'ws', 'prof', 'treasury', 'Treasury', 'asset', ?)",
            (now,),
        )
        self.conn.commit()
        self.runtime_config = {
            "env_file": "test.env",
            "backends": {"shop": {"name": "shop", "kind": "btcpay", "url": "https://pay.example.com", "token": "k"}},
        }

    def tearDown(self):
        self.conn.close()
        self.tmp.cleanup()

    @property
    def profile(self):
        return self.conn.execute("SELECT * FROM profiles WHERE id = 'prof'").fetchone()

    @property
    def workspace(self):
        return self.conn.execute("SELECT * FROM workspaces WHERE id = 'ws'").fetchone()

    def _wallet(self, wallet_id, label, config, kind="custom"):
        self.conn.execute(
            "INSERT INTO wallets(id, workspace_id, profile_id, label, kind, config_json, created_at) VALUES(?, 'ws', 'prof', ?, ?, ?, ?)",
            (wallet_id, label, kind, json.dumps(config), now_iso()),
        )
        self.conn.commit()
        return self.conn.execute("SELECT * FROM wallets WHERE id = ?", (wallet_id,)).fetchone()

    def _sync(self, wallet_id, routes):
        wallet = self.conn.execute("SELECT * FROM wallets WHERE id = ?", (wallet_id,)).fetchone()
        opener = _RoutedOpener(routes)
        with patch.object(sync_btcpay, "_backend_http_opener", lambda backend: opener):
            return handlers._sync_btcpay_wallet(self.conn, self.runtime_config, self.profile, wallet)

    def _hooks(self):
        return handlers._commercial_hooks()


class PayoutFeeTest(_BookTest):
    def _routes(self, *, payouts, wallet_rows):
        return {
            "/api/v1/stores/S1/invoices": _first_page([]),
            "/api/v1/stores/S1/payouts": payouts,
            "/api/v1/stores/S1/pull-payments": [],
            "/api/v1/stores/S1/payment-methods/BTC-CHAIN/wallet/transactions": _first_page(wallet_rows),
        }

    def setUp(self):
        super().setUp()
        self._wallet("w", "Shop wallet", {"sync_source": "btcpay", "backend": "shop", "store_id": "S1", "payment_method_id": "BTC-CHAIN"})
        # One send paying two payouts: 10,000 + 3,611 sats plus a 282-sat fee.
        self.wallet_rows = [{"transactionHash": TXID, "amount": "-0.00013893", "timestamp": PAYOUT_DATE, "confirmations": 3}]

    def test_fee_is_split_out_of_a_payout_send(self):
        outcome = self._sync(
            "w",
            self._routes(
                payouts=[_payout("po-1", sats=10_000, eur="5.00"), _payout("po-2", sats=3_611, eur="1.80")],
                wallet_rows=self.wallet_rows,
            ),
        )
        tx = self.conn.execute("SELECT * FROM transactions WHERE wallet_id = 'w'").fetchone()
        self.assertEqual((tx["amount"], tx["fee"], tx["amount_includes_fee"]), (13_893_000, 282_000, 1))
        self.assertEqual(outcome["payout_fees"], {"separated": 1, "backfilled": 0})
        self.assertEqual(outcome["invoice_provenance"]["suggestions"]["total"], 2)

    def test_fee_is_backfilled_when_payouts_become_known_later(self):
        # The send was imported before its payouts were visible (an older
        # Kassiber, or a key that could not read payouts yet).
        routes = self._routes(payouts=[], wallet_rows=self.wallet_rows)
        self._sync("w", routes)
        first = self.conn.execute("SELECT * FROM transactions WHERE wallet_id = 'w'").fetchone()
        self.assertEqual(first["fee"], 0)

        routes["/api/v1/stores/S1/payouts"] = [_payout("po-1", sats=10_000, eur="5.00"), _payout("po-2", sats=3_611, eur="1.80")]
        outcome = self._sync("w", routes)
        rows = self.conn.execute("SELECT * FROM transactions WHERE wallet_id = 'w'").fetchall()
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["fee"], 282_000)
        self.assertEqual(outcome["payout_fees"]["backfilled"], 1)

        # The backfilled fingerprint is the one a fresh import of the same row computes.
        record = normalize_btcpay_record(
            {"TransactionId": TXID, "Timestamp": rows[0]["occurred_at"], "Currency": "BTC", "Amount": "-0.00013893"}
        )
        btcpay_ledger.apply_payout_fees([record], btcpay_ledger.payout_totals_by_txid(self.conn, "prof", asset="BTC"))
        normalized = core_imports.normalize_import_record(record)
        self.assertEqual(
            rows[0]["fingerprint"],
            make_transaction_fingerprint(
                "w", TXID, normalized["occurred_at"], "outbound", "BTC", normalized["amount"], normalized["fee"]
            ),
        )

    def test_a_legacy_row_without_the_fee_flag_is_not_duplicated(self):
        normalized = core_imports.normalize_import_record(
            normalize_btcpay_record(
                {"TransactionId": TXID, "Timestamp": "2026-01-02T00:00:00Z", "Currency": "BTC", "Amount": "-0.00013893"}
            )
        )
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint, occurred_at, confirmed_at,
                direction, asset, amount, fee, amount_includes_fee, kind, raw_json, created_at
            ) VALUES('legacy', 'ws', 'prof', 'w', ?, ?, '2026-01-02T00:00:00Z', '2026-01-02T00:00:00Z',
                     'outbound', 'BTC', 13893000, 0, 0, 'withdrawal', '{}', ?)
            """,
            (
                TXID,
                make_transaction_fingerprint("w", TXID, normalized["occurred_at"], "outbound", "BTC", normalized["amount"], normalized["fee"]),
                now_iso(),
            ),
        )
        self.conn.commit()
        self._sync(
            "w",
            self._routes(payouts=[_payout("po-1", sats=10_000, eur="5.00"), _payout("po-2", sats=3_611, eur="1.80")], wallet_rows=self.wallet_rows),
        )
        rows = self.conn.execute("SELECT id, fee FROM transactions WHERE wallet_id = 'w'").fetchall()
        self.assertEqual([(row["id"], row["fee"]) for row in rows], [("legacy", 0)])

    def test_in_progress_payouts_count_towards_the_batch(self):
        outcome = self._sync(
            "w",
            self._routes(
                payouts=[_payout("po-1", sats=10_000, eur="5.00"), _payout("po-2", state="InProgress", sats=3_611, eur="1.80")],
                wallet_rows=self.wallet_rows,
            ),
        )
        self.assertEqual(outcome["payout_fees"]["separated"], 1)
        tx = self.conn.execute("SELECT fee FROM transactions WHERE wallet_id = 'w'").fetchone()
        self.assertEqual(tx["fee"], 282_000)

    def test_unexplained_remainders_stay_in_the_send(self):
        self.assertIsNone(btcpay_ledger.derived_payout_fee_msat(10_000_000, 12_000_000))
        self.assertIsNone(btcpay_ledger.derived_payout_fee_msat(1_000_000_000, 700_000_000))
        self.assertIsNone(btcpay_ledger.derived_payout_fee_msat(1_000_000, 400_000))
        self.assertEqual(btcpay_ledger.derived_payout_fee_msat(13_893_000, 13_611_000), 282_000)


class BatchPayoutReviewTest(_BookTest):
    def setUp(self):
        super().setUp()
        self._wallet("w", "Shop wallet", {"sync_source": "btcpay", "backend": "shop", "store_id": "S1", "payment_method_id": "BTC-CHAIN"})
        self.payouts = [_payout("po-1", sats=10_000, eur="5.00"), _payout("po-2", sats=3_611, eur="1.80")]
        self._sync(
            "w",
            {
                "/api/v1/stores/S1/invoices": _first_page([]),
                "/api/v1/stores/S1/payouts": self.payouts,
                "/api/v1/stores/S1/pull-payments": [],
                "/api/v1/stores/S1/payment-methods/BTC-CHAIN/wallet/transactions": _first_page(
                    [{"transactionHash": TXID, "amount": "-0.00013893", "timestamp": PAYOUT_DATE, "confirmations": 3}]
                ),
            },
        )
        self.tx_id = self.conn.execute("SELECT id FROM transactions WHERE wallet_id = 'w'").fetchone()["id"]
        self.links = {
            row["payment_id"]: row["id"]
            for row in self.conn.execute(
                """
                SELECT cl.id, p.payment_id FROM commercial_links cl
                JOIN btcpay_provenance_records p ON p.id = cl.btcpay_record_id
                WHERE cl.transaction_id = ?
                """,
                (self.tx_id,),
            ).fetchall()
        }

    def test_one_review_confirms_and_prices_the_whole_batch(self):
        context = commercial.get_transaction_commercial_context(self.conn, "ws", "prof", self.tx_id, self._hooks())
        self.assertEqual(context["payout_batch"]["size"], 2)
        self.assertEqual(context["payout_batch"]["fiat_value_exact"], "6.80")

        reviewed = commercial.review_link(
            self.conn, "ws", "prof", self.links["po-2"], self._hooks(), state="reviewed", commercial_kind="expense"
        )
        self.assertEqual(reviewed["batch_link_ids"], [self.links["po-1"]])
        tx = self.conn.execute("SELECT * FROM transactions WHERE id = ?", (self.tx_id,)).fetchone()
        self.assertEqual((tx["kind"], tx["fiat_value_exact"], tx["pricing_granularity"]), ("expense", "6.80", "payout_batch"))
        states = {row["id"]: row["state"] for row in self.conn.execute("SELECT id, state FROM commercial_links")}
        self.assertEqual({states[self.links["po-1"]], states[self.links["po-2"]]}, {"reviewed"})

        # Reopening either payout reopens the batch and restores the transaction.
        commercial.review_link(self.conn, "ws", "prof", self.links["po-1"], self._hooks(), state="suggested")
        tx = self.conn.execute("SELECT * FROM transactions WHERE id = ?", (self.tx_id,)).fetchone()
        self.assertEqual((tx["kind"], tx["commercial_applied_link_id"]), ("withdrawal", None))
        states = {row["id"]: row["state"] for row in self.conn.execute("SELECT id, state FROM commercial_links")}
        self.assertEqual({states[self.links["po-1"]], states[self.links["po-2"]]}, {"suggested"})

    def test_a_reviewed_batch_is_reopened_before_single_payouts_are_rejected(self):
        commercial.review_link(self.conn, "ws", "prof", self.links["po-2"], self._hooks(), state="reviewed", commercial_kind="expense")
        for link_id in (self.links["po-1"], self.links["po-2"]):
            with self.assertRaises(AppError):
                commercial.review_link(self.conn, "ws", "prof", link_id, self._hooks(), state="rejected")
        commercial.review_link(self.conn, "ws", "prof", self.links["po-1"], self._hooks(), state="suggested")
        rejected = commercial.review_link(self.conn, "ws", "prof", self.links["po-1"], self._hooks(), state="rejected")
        self.assertEqual(rejected["state"], "rejected")
        states = {row["id"]: row["state"] for row in self.conn.execute("SELECT id, state FROM commercial_links")}
        self.assertEqual(states[self.links["po-2"]], "suggested")

    def test_batch_with_mixed_currencies_is_refused(self):
        self.conn.execute(
            "UPDATE btcpay_provenance_records SET fiat_currency = 'USD' WHERE payment_id = 'po-1'"
        )
        self.conn.commit()
        with self.assertRaises(AppError) as ctx:
            commercial.review_link(
                self.conn, "ws", "prof", self.links["po-2"], self._hooks(), state="reviewed", commercial_kind="expense"
            )
        self.assertEqual(ctx.exception.code, "validation")


class PaymentLedgerTest(_BookTest):
    def setUp(self):
        super().setUp()
        self._wallet(
            "ln",
            "Shop Lightning",
            {
                "sync_source": "btcpay",
                "source_mode": "payments",
                "backend": "shop",
                "store_id": "S1",
                "payment_method_id": "BTC-LN",
                "payment_method_ids": ["BTC-LN", "BTC-LNURL"],
                "chain": "bitcoin",
            },
        )
        invoices = [
            {
                "id": "inv-paid",
                "status": "Settled",
                "currency": "EUR",
                "amount": "25.00",
                "createdTime": PAYOUT_DATE,
                "metadata": {"orderId": "A-1"},
                "paymentMethods": [
                    {
                        "paymentMethodId": "BTC-LN",
                        "rate": "50000",
                        "payments": [{"id": HASH_PAID, "value": "0.0005", "status": "Settled", "receivedDate": PAYOUT_DATE}],
                    }
                ],
            },
            {
                "id": "inv-open",
                "status": "Processing",
                "currency": "EUR",
                "amount": "10.00",
                "createdTime": PAYOUT_DATE,
                "paymentMethods": [
                    {
                        "paymentMethodId": "BTC-LNURL",
                        "payments": [{"id": "11" * 32, "value": "0.0002", "status": "Processing", "receivedDate": PAYOUT_DATE}],
                    }
                ],
            },
            {
                "id": "inv-chain",
                "status": "Settled",
                "currency": "EUR",
                "amount": "5.00",
                "createdTime": PAYOUT_DATE,
                "paymentMethods": [
                    {
                        "paymentMethodId": "BTC-CHAIN",
                        "payments": [{"id": f"{TXID}-0", "value": "0.0001", "status": "Settled", "receivedDate": PAYOUT_DATE}],
                    }
                ],
            },
        ]
        refund = _payout(
            "po-ln",
            sats=20_000,
            eur="10.00",
            method="BTC-LN",
            proof={"proofType": "PayoutLightningBlob", "paymentHash": HASH_REFUND},
        )
        self.routes = {
            "/api/v1/stores/S1/invoices": _first_page(invoices),
            "/api/v1/stores/S1/payouts": [refund],
            "/api/v1/stores/S1/pull-payments": [],
        }

    def test_ledger_books_settled_payments_and_completed_payouts(self):
        outcome = self._sync("ln", self.routes)
        rows = self.conn.execute(
            "SELECT direction, amount, fee, payment_hash, payment_hash_source, external_id FROM transactions WHERE wallet_id = 'ln' ORDER BY direction"
        ).fetchall()
        self.assertEqual(
            [(row["direction"], row["amount"], row["payment_hash"]) for row in rows],
            [("inbound", 50_000_000, HASH_PAID), ("outbound", 20_000_000, HASH_REFUND)],
        )
        self.assertEqual({row["payment_hash_source"] for row in rows}, {"btcpay"})
        self.assertTrue(all(row["external_id"].startswith("btcpay:S1:") for row in rows))
        self.assertEqual(outcome["ledger"]["pending"], 1)
        self.assertEqual(outcome["source_mode"], "payments")

        # The same payment links exactly and reviews with BTCPay's own price.
        payment_link = self.conn.execute(
            """
            SELECT cl.id, cl.confidence FROM commercial_links cl
            JOIN btcpay_provenance_records p ON p.id = cl.btcpay_record_id
            WHERE p.payment_id = ?
            """,
            (HASH_PAID,),
        ).fetchone()
        self.assertEqual(payment_link["confidence"], "exact")
        commercial.review_link(self.conn, "ws", "prof", payment_link["id"], self._hooks(), state="reviewed", commercial_kind="income")
        tx = self.conn.execute("SELECT kind, fiat_value_exact, pricing_source_kind FROM transactions WHERE payment_hash = ?", (HASH_PAID,)).fetchone()
        self.assertEqual((tx["kind"], tx["fiat_value_exact"], tx["pricing_source_kind"]), ("income", "25.00", "btcpay_payment"))

        # A rerun updates rows in place.
        self._sync("ln", self.routes)
        self.assertEqual(self.conn.execute("SELECT COUNT(*) FROM transactions WHERE wallet_id = 'ln'").fetchone()[0], 2)

    def test_bitcoin_priced_invoices_never_become_a_fiat_price(self):
        routes = dict(self.routes)
        routes["/api/v1/stores/S1/invoices"] = _first_page(
            [
                {
                    "id": "inv-sats",
                    "status": "Settled",
                    "currency": "BTC",
                    "amount": "0.0005",
                    "createdTime": PAYOUT_DATE,
                    "metadata": {"orderUrl": "https://pay.example.com/apps/app1/pos"},
                    "paymentMethods": [
                        {
                            "paymentMethodId": "BTC-LN",
                            "rate": "1",
                            "payments": [{"id": HASH_PAID, "value": "0.0005", "status": "Settled", "receivedDate": PAYOUT_DATE}],
                        }
                    ],
                }
            ]
        )
        self._sync("ln", routes)
        payment = self.conn.execute(
            "SELECT fiat_currency, fiat_value_exact, fiat_rate_exact FROM btcpay_provenance_records WHERE payment_id = ?",
            (HASH_PAID,),
        ).fetchone()
        self.assertEqual(tuple(payment), (None, None, None))
        link_id = self.conn.execute(
            "SELECT cl.id FROM commercial_links cl JOIN btcpay_provenance_records p ON p.id = cl.btcpay_record_id WHERE p.payment_id = ?",
            (HASH_PAID,),
        ).fetchone()["id"]
        # Rows stored by older versions still carry the bitcoin "currency".
        self.conn.execute("UPDATE btcpay_provenance_records SET fiat_currency = 'BTC', fiat_value_exact = '0.0005' WHERE payment_id = ?", (HASH_PAID,))
        self.conn.commit()
        commercial.review_link(self.conn, "ws", "prof", link_id, self._hooks(), state="reviewed", commercial_kind="income")
        tx = self.conn.execute(
            "SELECT kind, fiat_currency, fiat_value_exact, pricing_source_kind FROM transactions WHERE payment_hash = ?", (HASH_PAID,)
        ).fetchone()
        self.assertEqual(tx["kind"], "income")
        self.assertNotEqual(tx["fiat_currency"], "BTC")
        self.assertNotEqual(tx["pricing_source_kind"], "btcpay_payment")

    def test_payments_a_connected_node_already_books_are_held_back(self):
        self._wallet("node", "Shop node", {"chain": "bitcoin"}, kind="lnd")
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint, occurred_at, direction,
                asset, amount, fee, kind, raw_json, payment_hash, payment_hash_source, created_at
            ) VALUES('node-in', 'ws', 'prof', 'node', 'lnd:invoice:x', 'fp-node', '2026-01-02T00:00:00Z', 'inbound',
                     'BTC', 50000000, 0, 'lnd_invoice', '{}', ?, 'lnd', ?)
            """,
            (HASH_PAID, now_iso()),
        )
        self.conn.commit()
        outcome = self._sync("ln", self.routes)
        self.assertEqual(outcome["ledger"]["held_tracked_elsewhere"], 1)
        hashes = {row[0] for row in self.conn.execute("SELECT payment_hash FROM transactions WHERE wallet_id = 'ln'")}
        self.assertEqual(hashes, {HASH_REFUND})

    def test_ledger_sync_fails_when_btcpay_cannot_be_read(self):
        routes = dict(self.routes)
        routes["/api/v1/stores/S1/invoices"] = (403, {"code": "missing-permission", "missingPermission": "btcpay.store.canviewinvoices"})
        with self.assertRaises(AppError):
            self._sync("ln", routes)
        state = commercial.btcpay_store_sync_states(self.conn, "prof", [("shop", "S1")])[("shop", "S1")]
        self.assertTrue(state["stale"])
        self.assertEqual(state["last_error_code"], "auth_error")


    def test_ledger_refuses_to_book_without_payouts(self):
        routes = dict(self.routes)
        routes["/api/v1/stores/S1/payouts"] = (403, {"code": "missing-permission", "missingPermission": "btcpay.store.canviewpayouts"})
        with self.assertRaises(AppError) as ctx:
            self._sync("ln", routes)
        self.assertEqual(ctx.exception.code, "auth_error")
        self.assertEqual(self.conn.execute("SELECT COUNT(*) FROM transactions WHERE wallet_id = 'ln'").fetchone()[0], 0)

    def test_ledger_stops_when_a_connected_node_duplicates_booked_payments(self):
        self._sync("ln", self.routes)
        self._wallet("node", "Shop node", {"chain": "bitcoin"}, kind="lnd")
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint, occurred_at, direction,
                asset, amount, fee, kind, raw_json, payment_hash, payment_hash_source, created_at
            ) VALUES('node-in', 'ws', 'prof', 'node', 'lnd:invoice:x', 'fp-node', '2026-01-02T00:00:00Z', 'inbound',
                     'BTC', 50000000, 0, 'lnd_invoice', '{}', ?, 'lnd', ?)
            """,
            (HASH_PAID, now_iso()),
        )
        self.conn.commit()
        with self.assertRaises(AppError) as ctx:
            self._sync("ln", self.routes)
        self.assertEqual(ctx.exception.code, "conflict")
        self.assertEqual(ctx.exception.details["wallets"], ["Shop node"])

        # Excluding the ledger's copy resolves it; the node's row then links alone.
        self.conn.execute("UPDATE transactions SET excluded = 1 WHERE wallet_id = 'ln' AND payment_hash = ?", (HASH_PAID,))
        self.conn.commit()
        self._sync("ln", self.routes)
        record = self.conn.execute("SELECT * FROM btcpay_provenance_records WHERE payment_id = ?", (HASH_PAID,)).fetchone()
        matches = commercial._matching_transactions_for_record(self.conn, "prof", record)
        self.assertEqual([row["id"] for row in matches], ["node-in"])


class StoreSyncStateTest(_BookTest):
    def test_success_clears_the_error_and_age_marks_staleness(self):
        commercial.record_btcpay_store_sync(self.conn, "prof", backend_name="shop", store_id="S1", error_code="unavailable", error_message="down")
        state = commercial.btcpay_store_sync_states(self.conn, "prof", [("shop", "S1"), ("shop", "S2")])
        self.assertTrue(state[("shop", "S1")]["never_synced"])
        self.assertTrue(state[("shop", "S2")]["stale"])

        commercial.record_btcpay_store_sync(self.conn, "prof", backend_name="shop", store_id="S1", invoices_seen=3)
        fresh = commercial.btcpay_store_sync_states(self.conn, "prof", [("shop", "S1")])[("shop", "S1")]
        self.assertFalse(fresh["stale"])
        self.assertIsNone(fresh["last_error_code"])

        self.conn.execute("UPDATE btcpay_store_sync_states SET last_success_at = '2020-01-01T00:00:00Z'")
        old = commercial.btcpay_store_sync_states(self.conn, "prof", [("shop", "S1")])[("shop", "S1")]
        self.assertTrue(old["stale"])


class LedgerPlanTest(_BookTest):
    def _inspection(self):
        methods = []
        for method_id, rail, ledger_group in (
            ("BTC-LN", "lightning", "lightning"),
            ("BTC-LNURL", "lnurl", "lightning"),
            ("BTC-ARK", "plugin", "BTC-ARK"),
            ("ARKADE", "plugin", None),
        ):
            methods.append(
                {
                    "store_id": "S1",
                    "payment_method_id": method_id,
                    "enabled": True,
                    "sync_supported": False,
                    "rail": rail,
                    "currency": method_id.split("-")[0],
                    "chain": "bitcoin" if ledger_group else None,
                    "bitcoin_asset": bool(ledger_group),
                    "ledger_group": ledger_group,
                }
            )
        public = {
            "stores": [{"id": "S1", "name": "Café", "capabilities": {"invoices": True, "payouts": True, "wallet_history": False}}],
            "payment_methods": methods,
            "shared_wallets": [],
        }
        return InspectionResult(public=public, private={"previews": {}}, grant=None)

    def _plan(self, inspection=None):
        return btcpay_setup.plan_btcpay_setup(
            self.conn,
            "prof",
            inspection or self._inspection(),
            backend_name="shop",
            runtime_config=self.runtime_config,
            server_url="https://pay.example.com",
        )

    def test_unwatchable_rails_become_one_ledger_per_rail(self):
        plan = self._plan()
        recommendations = {m["payment_method_id"]: m["recommendation"]["action"] for m in plan["payment_methods"]}
        self.assertEqual(
            recommendations,
            {"BTC-LN": "payment_ledger", "BTC-LNURL": "payment_ledger", "BTC-ARK": "payment_ledger", "ARKADE": "skip"},
        )
        routes = btcpay_setup.normalize_setup_routes(btcpay_setup.routes_from_plan(plan))
        applied = btcpay_setup.apply_setup_routes(
            self.conn, self.workspace, self.profile, backend_name="shop", routes=routes, label="Café"
        )
        ledgers = {wallet["label"]: wallet["config"] for wallet in applied["payment_ledgers"]}
        self.assertEqual(sorted(ledgers), ["Café - Café - BTC-ARK (plugin)", "Café - Café - Lightning"])
        self.assertEqual(ledgers["Café - Café - Lightning"]["payment_method_ids"], ["BTC-LN", "BTC-LNURL"])
        self.assertEqual(applied["provenance_store_ids"], ["S1"])

        replanned = self._plan()
        by_method = {m["payment_method_id"]: m for m in replanned["payment_methods"]}
        self.assertEqual(by_method["BTC-LN"]["recommendation"]["reason"], "already_configured")
        # A configured store that was never refreshed is reported as stale.
        self.assertIn("stale_store_data", [warning["code"] for warning in replanned["warnings"]])
        self.assertTrue(replanned["stores"][0]["sync_state"]["never_synced"])

    def test_a_connected_lightning_node_keeps_invoices_as_provenance(self):
        self._wallet("node", "Shop node", {"chain": "bitcoin"}, kind="lnd")
        methods = {m["payment_method_id"]: m for m in self._plan()["payment_methods"]}
        self.assertEqual(methods["BTC-LN"]["recommendation"]["action"], "provenance_only")
        self.assertEqual(methods["BTC-LN"]["recommendation"]["reason"], "lightning_settles_elsewhere")
        self.assertTrue(methods["BTC-LN"]["actions"]["payment_ledger"]["available"])

    def test_a_key_without_payouts_cannot_book_a_ledger(self):
        inspection = self._inspection()
        inspection.public["stores"][0]["capabilities"]["payouts"] = False
        methods = {m["payment_method_id"]: m for m in self._plan(inspection)["payment_methods"]}
        self.assertFalse(methods["BTC-LN"]["actions"]["payment_ledger"]["available"])
        self.assertEqual(methods["BTC-LN"]["actions"]["payment_ledger"]["reason"], "payouts_permission_missing")
        self.assertEqual(methods["BTC-LN"]["recommendation"]["action"], "provenance_only")

    def test_a_configured_ledger_cannot_be_switched_while_it_books(self):
        routes = btcpay_setup.normalize_setup_routes(btcpay_setup.routes_from_plan(self._plan()))
        btcpay_setup.apply_setup_routes(self.conn, self.workspace, self.profile, backend_name="shop", routes=routes, label="Café")
        methods = {m["payment_method_id"]: m for m in self._plan()["payment_methods"]}
        actions = methods["BTC-LN"]["actions"]
        self.assertTrue(actions["payment_ledger"]["available"])
        self.assertEqual(
            {name for name, entry in actions.items() if not entry["available"] and entry["reason"] == "configured_balance_source"},
            {"provenance_only", "skip"},
        )
        for action in ("provenance_only", "skip"):
            with self.assertRaises(AppError) as ctx:
                btcpay_setup.validate_setup_routes(
                    self.conn,
                    "prof",
                    [{"store_id": "S1", "payment_method_id": "BTC-LN", "action": action}],
                    backend_name="shop",
                    runtime_config=self.runtime_config,
                    server_url="https://pay.example.com",
                )
            self.assertEqual(ctx.exception.details["reason"], "configured_balance_source")

    def test_on_chain_methods_cannot_be_booked_as_a_ledger(self):
        with self.assertRaises(AppError):
            btcpay_setup.normalize_setup_routes([{"store_id": "S1", "payment_method_id": "BTC-CHAIN", "action": "payment_ledger"}])


if __name__ == "__main__":
    unittest.main()
