"""Bark (BTC-ARK) wallets through BTCPay: booking rules, identities, ledger migration, Lightning overlap.

Greenfield is a path-keyed fake; row shapes follow the Bark plugin's wallet
API (``GreenfieldBarkWalletController`` in plugin 0.13).
"""

import json
import sqlite3
import unittest
from decimal import Decimal
from urllib.parse import parse_qs, urlsplit

from kassiber.btcpay import bark
from kassiber.btcpay.payment_methods import (
    canonical_payment_method_id,
    classify_payment_method,
    require_settlement_payment_method,
    require_wallet_history_payment_method,
)
from kassiber.cli import handlers
from kassiber.core import btcpay_setup, commercial
from kassiber.core.custody_evidence import row_boundary_amounts
from kassiber.errors import AppError
from kassiber.time_utils import now_iso

from tests.test_btcpay_ledger_and_fees import PAYOUT_DATE, _BookTest
from tests.test_sync_btcpay_greenfield import _RoutedOpener  # noqa: F401 - shared fake

T_ARK = "a1" * 32
T_OLD = "a2" * 32
H_ARK_LN = "b1" * 32
H_BTC_LN = "b2" * 32
H_PAYOUT = "b3" * 32
BARK_TX = "/api/v1/stores/S1/payment-methods/BTC-ARK/wallet/transactions"
BARK_WALLET = "/api/v1/stores/S1/payment-methods/BTC-ARK/wallet"
MSAT = 1000


def _row(row_id, kind, amount, *, fee=None, ts=PAYOUT_DATE, status="Confirmed", invoice=None, **extra):
    labels = {"invoice": {"type": "invoice", "text": "invoice", "ref": invoice}} if invoice else {}
    return {
        "id": row_id,
        "kind": kind,
        "amount": amount,
        "fee": fee,
        "status": status,
        "confirmations": 1 if status == "Confirmed" else 0,
        "timestamp": ts,
        "comment": "",
        "labels": labels,
        **extra,
    }


def _paged(rows):
    def route(url):
        query = parse_qs(urlsplit(url).query)
        statuses = query.get("statusFilter") or []
        selected = [row for row in rows if not statuses or row["status"] in statuses]
        skip = int(query.get("skip", ["0"])[0])
        limit = int(query.get("limit", ["100"])[0])
        return selected[skip : skip + limit]

    return route


def _invoice(invoice_id, method, payment_id, value):
    return {
        "id": invoice_id,
        "status": "Settled",
        "currency": "EUR",
        "amount": "10.00",
        "createdTime": PAYOUT_DATE,
        "paymentMethods": [
            {
                "paymentMethodId": method,
                "rate": "50000",
                "payments": [{"id": payment_id, "value": value, "status": "Settled", "receivedDate": PAYOUT_DATE}],
            }
        ],
    }


ROWS = [
    # Newest first, as the plugin lists them.
    _row("9-1:x", "ark-send", "-0.00001000", fee="0"),
    _row("8-1", "refresh", "-0.00000050", fee="0.00000050"),
    _row("7-1", "board", "-0.00000500", fee="0.00000500", transactionHash="c1" * 32),
    _row("6-1", "lightning-receive", "0.00009990", fee="0.00000010", paymentHash=H_ARK_LN, invoice="inv-ln"),
    _row(f"5-1:{T_ARK}:0", "ark-receive", "0.00020000", outpoint=f"{T_ARK}:0", invoice="inv-ark", transactionHash=T_ARK),
    _row(f"3-1:{T_OLD}:1", "ark-receive", "0.00005000", outpoint=f"{T_OLD}:1", movementId=3, invoice="inv-old"),
    _row("tx:" + "d1" * 32, "onchain-receive", "0.00100000", transactionHash="d1" * 32),
    _row("10-1", "ark-receive", "0.00000700", status="Unconfirmed", outpoint=f"{'e1' * 32}:0"),
]
INVOICES = [
    _invoice("inv-ark", "BTC-ARK", f"{T_ARK}-0", "0.0002"),
    _invoice("inv-ln", "BTC-ARK", H_ARK_LN, "0.0001"),
    _invoice("inv-old", "BARK", "inv-old#ark-movement-3", "0.00005"),
]
# Booked effect of every final row: the wallet's confirmed balance, in msat.
CONFIRMED_BALANCE_MSAT = sum(
    int(Decimal(row["amount"]) * 100_000_000) * MSAT for row in ROWS if row["status"] == "Confirmed"
)


def _routes(*, rows=ROWS, invoices=INVOICES, served=True, payouts=()):
    routes = {
        "/api/v1/stores/S1/invoices": lambda url: invoices if parse_qs(urlsplit(url).query).get("skip") == ["0"] else [],
        "/api/v1/stores/S1/payouts": list(payouts),
        "/api/v1/stores/S1/pull-payments": [],
    }
    if served is True:
        routes[BARK_WALLET] = {"balance": "0", "confirmedBalance": "0", "unconfirmedBalance": "0", "label": ""}
        routes[BARK_TX] = _paged(rows)
    elif served == "permission":
        routes[BARK_WALLET] = (403, {"code": "missing-permission", "missingPermission": "btcpay.store.canviewwallet"})
    elif served == "unset":
        routes[BARK_WALLET] = (404, {"code": "paymentmethod-not-found", "message": "BTC-ARK is not set up for this store"})
    return routes


class RowBookingTest(unittest.TestCase):
    def _book(self, row):
        return bark.bark_records(row, store_id="S1")

    def _effect(self, records):
        return sum(row_boundary_amounts({**r, "amount": int(r["amount"] * 100_000_000_000), "fee": int(r["fee"] * 100_000_000_000)}).wallet_delta_msat for r in records)

    def test_each_kind_books_its_amount_exactly(self):
        for row in ROWS[:-1]:
            with self.subTest(kind=row["kind"]):
                records = self._book(row)
                self.assertEqual(self._effect(records), int(Decimal(row["amount"]) * 100_000_000) * MSAT)

    def test_receive_fee_is_a_fee_record_and_send_fee_is_split_out(self):
        receive = self._book(ROWS[3])
        self.assertEqual([(r["direction"], r["amount"], r["fee"], r["kind"]) for r in receive], [
            ("inbound", Decimal("0.0001"), Decimal("0"), "deposit"),
            ("outbound", Decimal("0"), Decimal("1E-7"), "fee"),
        ])
        self.assertEqual(receive[0]["payment_hash"], H_ARK_LN)
        self.assertEqual(receive[0]["payment_hash_source"], "btcpay_bark")
        self.assertIsNone(receive[1]["payment_hash"])
        send = self._book(_row("s", "lightning-send", "-0.00003020", fee="0.00000020", paymentHash=H_PAYOUT))
        self.assertEqual([(r["direction"], r["amount"], r["fee"]) for r in send], [("outbound", Decimal("0.00003"), Decimal("2E-7"))])
        refresh = self._book(ROWS[1])
        self.assertEqual([(r["direction"], r["amount"], r["fee"], r["kind"]) for r in refresh], [("outbound", Decimal("0"), Decimal("5E-7"), "fee")])
        # An offboard into the wallet's own on-chain part leaves only its fee.
        offboard = self._book(_row("o", "offboard", "-0.00000300", fee="0.00000300"))
        self.assertEqual(offboard[0]["kind"], "fee")
        self.assertEqual(self._book(_row("z", "exit", "0", fee=None)), [])

    def test_rows_the_fee_cannot_explain_refuse(self):
        cases = {
            "unknown_kind": _row("u", "unknown", "0.0001"),
            "unexpected_inflow": _row("r", "refresh", "0.0001"),
            "unexpected_outflow": _row("i", "ark-receive", "-0.0001"),
            "fee_exceeds_amount": _row("f", "ark-send", "-0.0001", fee="0.0002"),
            "fee_without_amount": _row("e", "exit", "0", fee="0.00000010"),
            "negative_fee": _row("n", "ark-send", "-0.0001", fee="-0.00000001"),
        }
        cases["unexpected_outflow_fee_kind"] = _row("b", "board", "-0.0001", fee="0.00000010")
        for reason, row in cases.items():
            with self.subTest(reason=reason):
                with self.assertRaises(AppError) as ctx:
                    self._book(row)
                self.assertEqual(ctx.exception.code, "bark_row_unsupported")
                self.assertEqual(ctx.exception.details["reason"], reason.replace("_fee_kind", ""))
        for row in (_row("p", "ark-send", "-0.000000000001"), _row("q", "ark-send", "abc"), _row("t", "ark-send", True)):
            with self.subTest(amount=row["amount"]):
                with self.assertRaises(AppError) as ctx:
                    self._book(row)
                self.assertEqual(ctx.exception.code, "protocol_error")

    def test_payment_ids_name_outpoints_hashes_and_movements(self):
        self.assertEqual(bark.payment_keys(f"{T_ARK.upper()}-2"), {"outpoint": f"{T_ARK}:2"})
        self.assertEqual(bark.payment_keys(H_ARK_LN.upper()), {"payment_hash": H_ARK_LN})
        self.assertEqual(bark.payment_keys("inv-9#ark-movement-12"), {"invoice_id": "inv-9", "movement_id": 12})
        for junk in ("", "abc-1", f"{T_ARK}-x", "inv#ark-movement-", "inv#ark-movement-١"):
            self.assertEqual(bark.payment_keys(junk), {}, junk)

    def test_legacy_ids_map_to_btc_ark_and_bark_cannot_enrich_another_wallet(self):
        for legacy in ("BARK", "bark-chain"):
            self.assertEqual(canonical_payment_method_id(legacy), "BTC-ARK")
        info = classify_payment_method("BARK")
        self.assertEqual((info["payment_method_id"], info["ledger_group"], info["label"]), ("BTC-ARK", "BTC-ARK", "Bark (Ark)"))
        self.assertEqual(require_wallet_history_payment_method("bark"), "BTC-ARK")
        with self.assertRaises(AppError) as ctx:
            require_settlement_payment_method("BTC-ARK")
        self.assertEqual(ctx.exception.code, "validation")
        self.assertEqual(require_settlement_payment_method("BTC"), "BTC-CHAIN")


class _BarkBook(_BookTest):
    BARK_CONFIG = {"sync_source": "btcpay", "backend": "shop", "store_id": "S1", "payment_method_id": "BTC-ARK", "chain": "bitcoin"}
    ARK_LEDGER = {
        "sync_source": "btcpay",
        "source_mode": "payments",
        "backend": "shop",
        "store_id": "S1",
        "payment_method_id": "BTC-ARK",
        "payment_method_ids": ["BTC-ARK"],
        "chain": "bitcoin",
    }
    LN_LEDGER = {
        "sync_source": "btcpay",
        "source_mode": "payments",
        "backend": "shop",
        "store_id": "S1",
        "payment_method_id": "BTC-LN",
        "payment_method_ids": ["BTC-LN"],
        "chain": "bitcoin",
    }

    def _rows(self, wallet_id):
        return self.conn.execute(
            "SELECT * FROM transactions WHERE wallet_id = ? ORDER BY external_id", (wallet_id,)
        ).fetchall()

    def _balance(self, wallet_id):
        return sum(
            row_boundary_amounts(dict(row)).wallet_delta_msat
            for row in self.conn.execute("SELECT * FROM transactions WHERE wallet_id = ? AND excluded = 0", (wallet_id,))
        )

    def _snapshot(self):
        tables = [row[0] for row in self.conn.execute("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")]
        return {
            table: sorted(json.dumps(list(row), default=str) for row in self.conn.execute(f"SELECT * FROM {table}"))
            for table in tables
            if table not in {"btcpay_store_sync_states", "btcpay_provenance_records", "commercial_links", "sqlite_sequence"}
        }


class BarkWalletImportTest(_BarkBook):
    def setUp(self):
        super().setUp()
        self._wallet("bark", "Shop Bark", self.BARK_CONFIG)

    def test_the_wallet_books_every_final_row_and_links_invoice_payments(self):
        outcome = self._sync("bark", _routes())
        self.assertEqual(self._balance("bark"), CONFIRMED_BALANCE_MSAT)
        ids = {row["external_id"] for row in self._rows("bark")}
        self.assertIn(f"btcpay:S1:invoice:inv-ark:payment:{T_ARK}-0", ids)
        self.assertIn(f"btcpay:S1:invoice:inv-ln:payment:{H_ARK_LN}", ids)
        self.assertIn("btcpay:S1:invoice:inv-old:payment:inv-old#ark-movement-3", ids)
        self.assertIn("btcpay-bark:S1:6-1:fee", ids)
        self.assertIn("btcpay-bark:S1:9-1:x", ids)
        self.assertNotIn("btcpay-bark:S1:10-1", ids)
        self.assertEqual(outcome["bark"]["linked_to_payments"], 3)
        links = {
            row["payment_id"]: row["confidence"]
            for row in self.conn.execute(
                "SELECT p.payment_id, cl.confidence FROM commercial_links cl JOIN btcpay_provenance_records p ON p.id = cl.btcpay_record_id"
            )
        }
        self.assertEqual(links, {f"{T_ARK}-0": "exact", H_ARK_LN: "exact", "inv-old#ark-movement-3": "exact"})

        before = self._snapshot()
        again = self._sync("bark", _routes())
        self.assertEqual(again["imported"], 0)
        self.assertEqual(self._snapshot()["transactions"], before["transactions"])

    def test_freshness_progress_cannot_end_the_import_savepoint(self):
        # A freshness job persists progress and commits on this connection.
        from kassiber.core import sync as core_sync

        token = core_sync.sync_progress_emitter.set(lambda event: self.conn.commit())
        try:
            self._sync("bark", _routes())
        finally:
            core_sync.sync_progress_emitter.reset(token)
        self.assertEqual(self._balance("bark"), CONFIRMED_BALANCE_MSAT)

    def test_a_row_booked_before_its_invoice_moves_to_the_payment_key(self):
        self._sync("bark", _routes(invoices=[]))
        early = self.conn.execute("SELECT id FROM transactions WHERE external_id = ?", (f"btcpay-bark:S1:5-1:{T_ARK}:0",)).fetchone()
        self.assertIsNotNone(early)
        self.conn.execute("UPDATE transactions SET note = 'kept' WHERE id = ?", (early["id"],))
        self.conn.commit()
        outcome = self._sync("bark", _routes())
        self.assertEqual(outcome["bark"]["identities_rekeyed"], 3)
        row = self.conn.execute("SELECT external_id, note FROM transactions WHERE id = ?", (early["id"],)).fetchone()
        self.assertEqual(tuple(row), (f"btcpay:S1:invoice:inv-ark:payment:{T_ARK}-0", "kept"))
        self.assertEqual(self._balance("bark"), CONFIRMED_BALANCE_MSAT)

    def test_a_payment_two_rows_could_be_stays_unlinked(self):
        rows = [
            _row("5-1:a", "ark-receive", "0.0001", movementId=3, invoice="inv-old", outpoint=f"{'f1' * 32}:0"),
            _row("5-1:b", "ark-receive", "0.0001", movementId=3, invoice="inv-old", outpoint=f"{'f2' * 32}:0"),
        ]
        outcome = self._sync("bark", _routes(rows=rows))
        self.assertEqual(outcome["bark"]["ambiguous_payments"], 2)
        self.assertEqual({row["external_id"] for row in self._rows("bark")}, {"btcpay-bark:S1:5-1:a", "btcpay-bark:S1:5-1:b"})

    def test_an_unknown_kind_refuses_the_whole_sync(self):
        rows = [*ROWS, _row("11-1", "teleport", "0.0001")]
        with self.assertRaises(AppError) as ctx:
            self._sync("bark", _routes(rows=rows))
        self.assertEqual(ctx.exception.code, "bark_row_unsupported")
        self.assertEqual(self._rows("bark"), [])


class LedgerMigrationTest(_BarkBook):
    def setUp(self):
        super().setUp()
        self._wallet("ark", "Shop Bark payments", self.ARK_LEDGER)

    def _ledger_first(self):
        outcome = self._sync("ark", _routes(served=False))
        self.assertEqual(outcome["source_mode"], "payments")
        self.assertEqual(outcome["bark_wallet_history"], "not_served")
        rows = self._rows("ark")
        self.assertEqual(len(rows), 3)
        return {row["external_id"]: row["id"] for row in rows}

    def test_older_plugins_keep_the_ledger(self):
        self._ledger_first()
        for served, status in ((False, "not_served"), ("unset", "not_served"), ("permission", "missing_permission")):
            outcome = self._sync("ark", _routes(served=served))
            self.assertEqual((outcome["source_mode"], outcome["bark_wallet_history"]), ("payments", status))
        config = json.loads(self.conn.execute("SELECT config_json FROM wallets WHERE id = 'ark'").fetchone()[0])
        self.assertEqual(config["source_mode"], "payments")

    def test_the_next_sync_switches_and_keeps_every_booked_payment(self):
        booked = self._ledger_first()
        self.conn.execute("UPDATE transactions SET note = 'reviewed' WHERE wallet_id = 'ark'")
        self.conn.commit()
        stale = self.conn.execute("SELECT * FROM wallets WHERE id = 'ark'").fetchone()
        outcome = self._sync("ark", _routes())
        self.assertTrue(outcome["bark"]["migrated_from_payment_ledger"])
        self.assertEqual(outcome["bark"]["ledger_rows_kept"], 3)
        config = json.loads(self.conn.execute("SELECT config_json FROM wallets WHERE id = 'ark'").fetchone()[0])
        self.assertNotIn("source_mode", config)
        self.assertNotIn("payment_method_ids", config)
        self.assertEqual(config["payment_method_id"], "BTC-ARK")
        after = {row["external_id"]: row for row in self._rows("ark")}
        for external_id, transaction_id in booked.items():
            self.assertEqual((after[external_id]["id"], after[external_id]["note"]), (transaction_id, "reviewed"))
        self.assertEqual(self._balance("ark"), CONFIRMED_BALANCE_MSAT)
        self.assertEqual(json.loads(after[f"btcpay:S1:invoice:inv-ln:payment:{H_ARK_LN}"]["raw_json"])["source"], "btcpay_bark_wallet")
        # The switch survives the sync's own bookkeeping (last_synced_at).
        handlers._mark_wallet_synced(self.conn, stale)
        self.assertNotIn("source_mode", json.loads(self.conn.execute("SELECT config_json FROM wallets WHERE id = 'ark'").fetchone()[0]))
        # Afterwards it syncs as a wallet import, without rows twice.
        self._sync("ark", _routes())
        self.assertEqual(self._balance("ark"), CONFIRMED_BALANCE_MSAT)

    def test_a_payment_without_a_bark_row_refuses_and_changes_nothing(self):
        invoices = [*INVOICES, _invoice("inv-lost", "BTC-ARK", f"{'99' * 32}-0", "0.0003")]
        self._sync("ark", _routes(served=False, invoices=invoices))
        before = self._snapshot()
        with self.assertRaises(AppError) as ctx:
            self._sync("ark", _routes(invoices=invoices))
        self.assertEqual(ctx.exception.code, "bark_migration_conflict")
        self.assertEqual(ctx.exception.details["reason"], "unmatched_ledger_rows")
        self.assertEqual(
            [(p["external_id"], p["reason"]) for p in ctx.exception.details["problems"]],
            [(f"btcpay:S1:invoice:inv-lost:payment:{'99' * 32}-0", "unmatched")],
        )
        self.assertEqual(self._snapshot(), before)

        # Excluding the payment the wallet never held lets the switch go ahead.
        self.conn.execute("UPDATE transactions SET excluded = 1 WHERE external_id LIKE '%inv-lost%'")
        self.conn.commit()
        self.assertTrue(self._sync("ark", _routes(invoices=invoices))["bark"]["migrated_from_payment_ledger"])

    def test_a_payment_of_another_amount_refuses(self):
        self._ledger_first()
        rows = [dict(row) for row in ROWS]
        rows[4]["amount"] = "0.00019000"
        with self.assertRaises(AppError) as ctx:
            self._sync("ark", _routes(rows=rows))
        self.assertEqual(ctx.exception.details["problems"][0]["reason"], "amount_mismatch")


class LightningOverlapTest(_BarkBook):
    LN_ROWS = [_row("20-1", "lightning-receive", "0.00030000", fee="0", paymentHash=H_BTC_LN, invoice="inv-bln")]
    LN_INVOICES = [_invoice("inv-bln", "BTC-LN", H_BTC_LN, "0.0003")]

    def setUp(self):
        super().setUp()
        self._wallet("ln", "Shop Lightning", self.LN_LEDGER)
        self._wallet("bark", "Shop Bark", self.BARK_CONFIG)

    def _routes(self):
        return _routes(rows=self.LN_ROWS, invoices=self.LN_INVOICES)

    def test_the_bark_wallet_takes_a_payment_the_lightning_ledger_booked(self):
        self._sync("ln", self._routes())
        (ledger_row,) = self._rows("ln")
        outcome = self._sync("bark", self._routes())
        self.assertEqual(outcome["bark"]["lightning_ledger_rows_moved"], 1)
        self.assertEqual(self._rows("ln"), [])
        (moved,) = self._rows("bark")
        self.assertEqual((moved["id"], moved["payment_hash"]), (ledger_row["id"], H_BTC_LN))
        held = self._sync("ln", self._routes())
        self.assertEqual(held["ledger"]["held_tracked_elsewhere"], 1)
        self.assertEqual(self._rows("ln"), [])
        self.assertEqual(self._balance("bark") + self._balance("ln"), 30_000_000)

    def test_a_ledger_synced_after_the_bark_wallet_holds_the_payment_back(self):
        self._sync("bark", self._routes())
        held = self._sync("ln", self._routes())
        self.assertEqual(held["ledger"]["held_tracked_elsewhere"], 1)
        self.assertEqual(self._balance("bark") + self._balance("ln"), 30_000_000)

    def test_another_wallet_booking_the_payment_refuses(self):
        self._wallet("node", "Shop node", {"chain": "bitcoin"}, kind="lnd")
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint, occurred_at, direction,
                asset, amount, fee, kind, raw_json, payment_hash, payment_hash_source, created_at
            ) VALUES('node-in', 'ws', 'prof', 'node', 'lnd:invoice:x', 'fp-node', '2026-01-02T00:00:00Z', 'inbound',
                     'BTC', 30000000, 0, 'lnd_invoice', '{}', ?, 'lnd', ?)
            """,
            (H_BTC_LN, now_iso()),
        )
        self.conn.commit()
        before = self._snapshot()
        with self.assertRaises(AppError) as ctx:
            self._sync("bark", self._routes())
        self.assertEqual((ctx.exception.code, ctx.exception.details["reason"]), ("bark_migration_conflict", "booked_by_other_wallet"))
        self.assertEqual(self._snapshot(), before)

    def test_a_reviewed_pair_keeps_the_ledger_row_where_it_is(self):
        self._sync("ln", self._routes())
        (ledger_row,) = self._rows("ln")
        self._wallet("other", "Other", {"chain": "bitcoin"})
        self.conn.execute(
            """
            INSERT INTO transactions(id, workspace_id, profile_id, wallet_id, external_id, fingerprint, occurred_at,
                direction, asset, amount, fee, kind, raw_json, created_at)
            VALUES('out', 'ws', 'prof', 'other', 'o', 'fp-o', '2026-01-02T00:00:00Z', 'outbound', 'BTC', 30000000, 0,
                   'withdrawal', '{}', ?)
            """,
            (now_iso(),),
        )
        self.conn.execute(
            "INSERT INTO transaction_pairs(id, workspace_id, profile_id, out_transaction_id, in_transaction_id, created_at) "
            "VALUES('pair', 'ws', 'prof', 'out', ?, ?)",
            (ledger_row["id"], now_iso()),
        )
        self.conn.commit()
        with self.assertRaises(AppError) as ctx:
            self._sync("bark", self._routes())
        self.assertEqual(ctx.exception.details["reason"], "bound_by_review")
        self.assertEqual(ctx.exception.details["tables"], ["transaction_pairs"])
        self.conn.execute("UPDATE transaction_pairs SET deleted_at = ?", (now_iso(),))
        self.conn.commit()
        self.assertEqual(self._sync("bark", self._routes())["bark"]["lightning_ledger_rows_moved"], 1)


class BarkSetupPlanTest(_BarkBook):
    def _plan(self, *, available, history=True):
        method = {
            "store_id": "S1",
            "payment_method_id": "BTC-ARK",
            "enabled": True,
            "sync_supported": True,
            "rail": "plugin",
            "currency": "BTC",
            "chain": "bitcoin",
            "bitcoin_asset": True,
            "ledger_group": "BTC-ARK",
            "wallet_history_available": available,
        }
        public = {
            "stores": [{"id": "S1", "name": "Café", "capabilities": {"invoices": True, "payouts": True, "wallet_history": history}}],
            "payment_methods": [method],
            "shared_wallets": [],
        }
        from kassiber.btcpay.discovery import InspectionResult

        plan = btcpay_setup.plan_btcpay_setup(
            self.conn,
            "prof",
            InspectionResult(public=public, private={"previews": {}}, grant=None),
            backend_name="shop",
            runtime_config=self.runtime_config,
            server_url="https://pay.example.com",
        )
        return plan["payment_methods"][0]

    def test_a_plugin_serving_wallet_history_is_imported_as_a_wallet(self):
        method = self._plan(available=True)
        self.assertEqual(method["recommendation"]["reason"], "import_bark_wallet_history")
        self.assertTrue(method["actions"]["wallet_source"]["available"])
        self.assertFalse(method["actions"]["existing_wallet"]["available"])
        routes = btcpay_setup.normalize_setup_routes([{"store_id": "S1", "payment_method_id": "BTC-ARK", "action": "wallet_source"}])
        applied = btcpay_setup.apply_setup_routes(self.conn, self.workspace, self.profile, backend_name="shop", routes=routes, label="Café")
        self.assertEqual(applied["wallet_sources"][0]["config"]["payment_method_id"], "BTC-ARK")

    def test_older_plugins_and_read_only_keys_book_a_ledger(self):
        for available, history, reason in (
            (False, True, "bark_wallet_history_unavailable"),
            (None, True, "bark_wallet_history_unavailable"),
            (False, False, "wallet_history_permission_missing"),
        ):
            with self.subTest(available=available, history=history):
                method = self._plan(available=available, history=history)
                self.assertEqual(method["recommendation"]["action"], "payment_ledger")
                self.assertEqual(method["recommendation"]["reason"], "bark_payment_ledger")
                self.assertEqual(method["actions"]["wallet_source"]["reason"], reason)

    def test_bark_cannot_be_mapped_onto_another_wallet(self):
        with self.assertRaises(AppError):
            btcpay_setup.normalize_setup_routes(
                [{"store_id": "S1", "payment_method_id": "BTC-ARK", "action": "existing_wallet", "wallet": "Cold"}]
            )


class DiscoveryProbeTest(unittest.TestCase):
    def _inspect(self, wallet_route, *, permissions=("btcpay.store.canviewstoresettings", "btcpay.store.canviewwallet")):
        from kassiber.btcpay.discovery import inspect_btcpay_connection
        from tests.test_sync_btcpay_greenfield import _client

        routes = {
            "/api/v1/server/info": {"version": "2.4.7", "fullySynched": True, "supportedPaymentMethods": ["BTC-CHAIN", "BTC-ARK"]},
            "/api/v1/api-keys/current": {"apiKey": "secret-key", "label": "Kassiber", "permissions": list(permissions)},
            "/api/v1/stores": [{"id": "S1", "name": "Bark shop", "defaultCurrency": "EUR"}],
            "/api/v1/stores/S1/payment-methods": [{"paymentMethodId": "BTC-ARK", "enabled": True}],
        }
        if wallet_route is not None:
            routes[BARK_WALLET] = wallet_route
        opener = _RoutedOpener(routes)
        public = inspect_btcpay_connection(_client(opener), max_workers=1).public
        (method,) = public["payment_methods"]
        return method, [url for url in opener.urls if "/BTC-ARK/" in url]

    def test_the_wallet_probe_tells_plugin_versions_apart(self):
        cases = (
            ({"balance": "0.001"}, True, None),
            (None, False, "not_served"),
            ((404, {"code": "paymentmethod-not-found"}), False, "not_served"),
            ((403, {"code": "missing-permission"}), False, "missing_permission"),
            ((503, {"code": "not-available"}), None, "unavailable"),
        )
        for route, available, error in cases:
            with self.subTest(route=route):
                method, probes = self._inspect(route)
                self.assertEqual((method["wallet_history_available"], method["wallet_history_error"]), (available, error))
                self.assertEqual((method["label"], method["settlement"], method["sync_supported"]), ("Bark (Ark)", "bark", True))
                # BTCPay's 503 is retried; every other answer is final.
                self.assertEqual(len(probes) > 1, error == "unavailable")

    def test_a_key_that_cannot_view_wallets_is_not_probed(self):
        method, probes = self._inspect({"balance": "0"}, permissions=("btcpay.store.canviewstoresettings",))
        self.assertEqual((method["wallet_history_available"], method["wallet_history_error"], probes), (False, None, []))


class CommercialMatchTest(_BarkBook):
    def test_bark_rows_link_exactly_by_provenance_key(self):
        self._wallet("bark", "Shop Bark", self.BARK_CONFIG)
        self._sync("bark", _routes())
        record = self.conn.execute("SELECT * FROM btcpay_provenance_records WHERE payment_id = ?", (f"{T_ARK}-0",)).fetchone()
        (match,) = commercial._matching_transactions_for_record(self.conn, "prof", record)
        self.assertEqual(self.conn.execute("SELECT external_id FROM transactions WHERE id = ?", (match["id"],)).fetchone()[0], record["stable_key"])


if __name__ == "__main__":
    unittest.main()
