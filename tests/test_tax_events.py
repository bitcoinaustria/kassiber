import json
import hashlib
import unittest
from decimal import Decimal

from kassiber.core import pricing
from kassiber.core.engines import GenericRP2TaxEngine
from kassiber.core import quarantine_catalog
from kassiber.core.tax_events import (
    build_tax_quarantine,
    dedupe_quarantines,
)
from kassiber.transfers import detect_unscoped_transfer_review_ids
from tests.custody_tax_helpers import (
    authoritative_chain_observation,
    finalized_tax_inputs,
)
from tests.tax_ledger_generator import (
    chain_txid,
    direct,
    finalized,
    graph_json,
    move,
    pair,
    profile,
    projected,
    row,
)


def _row(
    tx_id,
    wallet_id,
    direction,
    amount,
    *,
    fee=0,
    fiat_rate=None,
    external_id=None,
    raw_json=None,
    payment_hash=None,
    kind=None,
    canonical_external=True,
):
    external_value = external_id or tx_id
    if canonical_external and not (
        len(str(external_value)) == 64
        and all(char in "0123456789abcdefABCDEF" for char in str(external_value))
    ):
        external_value = hashlib.sha256(
            f"tax-event-test:{external_value}".encode()
        ).hexdigest()
    if raw_json is None:
        raw_payload = {"txid": external_value} if canonical_external else {}
    else:
        raw_payload = json.loads(raw_json)
        if (
            canonical_external
            and isinstance(raw_payload, dict)
            and raw_payload.get("txid") == external_id
        ):
            raw_payload["txid"] = external_value
    payment_hash_source = None
    normalized_kind = str(kind or "").lower()
    if payment_hash and normalized_kind.startswith("lnd_"):
        payment_hash_source = "lnd"
        raw_payload["_kassiber_provenance"] = {"import_source": "lnd"}
        raw_payload.update({"chain": "lightning", "network": "main"})
    elif payment_hash and normalized_kind in {
        "cln_invoice",
        "cln_pay",
        "ln_invoice",
        "ln_pay",
    }:
        payment_hash_source = "core_lightning"
        raw_payload["_kassiber_provenance"] = {
            "import_source": "core-lightning"
        }
        raw_payload.update({"chain": "lightning", "network": "main"})
    return {
        "id": tx_id,
        "wallet_id": wallet_id,
        "occurred_at": "2026-01-01T00:00:00Z",
        "direction": direction,
        "asset": "BTC",
        "amount": amount,
        "fee": fee,
        "amount_includes_fee": False,
        "fiat_rate": fiat_rate,
        "fiat_value": None,
        "kind": kind or ("deposit" if direction == "inbound" else "withdrawal"),
        "description": tx_id,
        "note": None,
        "external_id": external_value,
        "payment_hash": payment_hash,
        "payment_hash_source": payment_hash_source,
        "privacy_boundary": None,
        "raw_json": json.dumps(raw_payload),
    }


FIFO = profile("generic", "FIFO")
COARSE_REVIEW = profile("generic", "FIFO", require_coarse_review=1)
MOVING_AVERAGE_AT = profile("at", "moving_average_at")
COARSE = {
    "pricing_quality": pricing.QUALITY_COARSE_FALLBACK,
    "pricing_granularity": "daily",
    "pricing_provider": "kraken",
}


def _ledger(book_profile, inputs):
    return GenericRP2TaxEngine(book_profile).build_ledger_state(inputs)


def _reasons(state):
    return {item["transaction_id"]: item["reason"] for item in state.quarantines}


def _detail(state, transaction_id):
    return json.loads(
        next(
            item["detail_json"]
            for item in state.quarantines
            if item["transaction_id"] == transaction_id
        )
    )


def _entry_types(state, transaction_id):
    return sorted(
        entry["entry_type"]
        for entry in state.entries
        if entry["transaction_id"] == transaction_id
    )


def _description(state, transaction_id, entry_type):
    [text] = [
        entry["description"]
        for entry in state.entries
        if entry["transaction_id"] == transaction_id
        and entry["entry_type"] == entry_type
    ]
    return text


class TransferIdentityTest(unittest.TestCase):
    def test_zero_value_inbound_does_not_block_self_transfer(self):
        # A stray 0-value inbound row sharing the txid of a real cold->hot
        # self-transfer must not inflate the inbound count: detect_intra_transfers
        # still pairs the real legs rather than skipping a non-1-out/1-in group.
        from kassiber.transfers import detect_intra_transfers

        out_row = _row("move-out", "wallet-a", "outbound", 50_000_000_000,
                       fee=10_000_000, fiat_rate=60_000, external_id="move-1")
        in_row = _row("move-in", "wallet-b", "inbound", 50_000_000_000,
                      fiat_rate=60_000, external_id="move-1")
        zero_in = _row("zero-in", "wallet-c", "inbound", 0,
                       fiat_rate=60_000, external_id="move-1")
        rows = [out_row, in_row, zero_in]

        pairs, matched = detect_intra_transfers(rows)
        self.assertEqual(len(pairs), 1)
        self.assertEqual(pairs[0]["out"]["id"], "move-out")
        self.assertEqual(pairs[0]["in"]["id"], "move-in")
        self.assertEqual(matched, {"move-out", "move-in"})

    def test_detect_intra_transfers_pairs_lightning_by_payment_hash(self):
        # An own-node LN payment (LND pays a CLN invoice) shares a payment_hash
        # across two owned wallets but has distinct external_ids, so the txid
        # grouping never pairs it. The payment_hash pass must recognize it as a
        # self-transfer so the inbound is not booked as phantom income.
        from kassiber.transfers import detect_intra_transfers

        payment_hash = "ab" * 32
        out_row = _row(
            "lnd:pay:x", "wallet-lnd", "outbound", 1_000_000_000,
            fee=2_000_000, external_id="lnd:pay:x", payment_hash=payment_hash,
            kind="lnd_pay",
        )
        in_row = _row(
            "cln:income:y", "wallet-cln", "inbound", 1_000_000_000,
            external_id="cln:income:y", payment_hash=payment_hash,
            kind="cln_invoice",
        )
        pairs, matched = detect_intra_transfers([out_row, in_row])
        self.assertEqual(len(pairs), 1)
        self.assertEqual(pairs[0]["out"]["id"], "lnd:pay:x")
        self.assertEqual(pairs[0]["in"]["id"], "cln:income:y")
        self.assertEqual(matched, {"lnd:pay:x", "cln:income:y"})

    def test_detect_intra_transfers_rejects_malformed_lightning_payment_hash(self):
        from kassiber.transfers import detect_intra_transfers

        out_row = _row(
            "lnd:pay:bad", "wallet-lnd", "outbound", 1_000_000_000,
            external_id="lnd:pay:bad", payment_hash="shared-import-id",
            kind="lnd_pay",
        )
        in_row = _row(
            "cln:invoice:bad", "wallet-cln", "inbound", 1_000_000_000,
            external_id="cln:invoice:bad", payment_hash="shared-import-id",
            kind="cln_invoice",
        )

        self.assertEqual(detect_intra_transfers([out_row, in_row]), ([], set()))

    def test_detect_intra_transfers_normalizes_lightning_payment_hash_case(self):
        from kassiber.transfers import detect_intra_transfers

        out_row = _row(
            "lnd:pay:case", "wallet-lnd", "outbound", 1_000_000_000,
            external_id="lnd:pay:case", payment_hash="AB" * 32,
            kind="lnd_pay",
        )
        in_row = _row(
            "cln:invoice:case", "wallet-cln", "inbound", 1_000_000_000,
            external_id="cln:invoice:case", payment_hash="ab" * 32,
            kind="cln_invoice",
        )

        pairs, matched = detect_intra_transfers([out_row, in_row])

        self.assertEqual(len(pairs), 1)
        self.assertEqual(matched, {"lnd:pay:case", "cln:invoice:case"})

    def test_detect_intra_transfers_requires_equal_native_lightning_principal(self):
        from kassiber.transfers import detect_intra_transfers

        payment_hash = "bc" * 32
        out_row = _row(
            "lnd:pay:mismatch", "wallet-lnd", "outbound", 10_000_000,
            fee=200_000, external_id="lnd:pay:mismatch",
            payment_hash=payment_hash, kind="lnd_pay",
        )
        in_row = _row(
            "cln:invoice:mismatch", "wallet-cln", "inbound", 8_000_000,
            external_id="cln:invoice:mismatch", payment_hash=payment_hash,
            kind="cln_invoice",
        )

        # The 2,000-sat principal difference is below the generic absolute fee
        # floor, but native node fees are already separate and cannot explain it.
        self.assertEqual(detect_intra_transfers([out_row, in_row]), ([], set()))

    def test_lightning_payment_hash_no_owned_receiver_stays_unpaired(self):
        # A payment to an EXTERNAL node has only an outbound leg; no inbound row
        # shares the hash, so it must NOT pair and stays a real disposal.
        from kassiber.transfers import detect_intra_transfers

        out_row = _row(
            "cln:pay:ext", "wallet-cln", "outbound", 500_000_000,
            fee=1_000_000, external_id="cln:pay:ext", payment_hash="cd" * 32,
            kind="cln_pay",
        )
        pairs, matched = detect_intra_transfers([out_row])
        self.assertEqual(pairs, [])
        self.assertEqual(matched, set())

    def test_lightning_same_wallet_payment_hash_pairs_as_internal_move(self):
        # A circular self-payment through the same owned node has distinct
        # external ids but the same payment hash. Pair it as an internal move so
        # the legs do not become a taxable disposal plus a fresh acquisition.
        from kassiber.transfers import detect_intra_transfers

        payment_hash = "ef" * 32
        out_row = _row(
            "p:out", "wallet-x", "outbound", 100_000_000,
            fee=1_000_000, fiat_rate=60_000,
            external_id="p:out", payment_hash=payment_hash, kind="cln_pay",
        )
        in_row = _row(
            "p:in", "wallet-x", "inbound", 100_000_000,
            external_id="p:in", payment_hash=payment_hash, kind="cln_invoice",
        )
        pairs, matched = detect_intra_transfers([out_row, in_row])
        self.assertEqual(len(pairs), 1)
        self.assertEqual(pairs[0]["out"]["id"], "p:out")
        self.assertEqual(pairs[0]["in"]["id"], "p:in")
        self.assertEqual(matched, {"p:out", "p:in"})
        # The engine books this pair's fee as a transfer fee:
        # LightningPaymentHashEngineTest.test_same_wallet_payment_hash_books_fee_not_sell_buy.

    def test_onchain_payment_hash_rows_do_not_auto_pair_as_internal_move(self):
        from kassiber.transfers import detect_intra_transfers

        payment_hash = "fa" * 32
        out_row = _row(
            "ln-pay", "wallet-a", "outbound", 100_000_000,
            external_id="ln-pay", payment_hash=payment_hash, kind="cln_pay",
        )
        in_row = _row(
            "chain-claim", "wallet-b", "inbound", 99_500_000,
            external_id="chain-claim", payment_hash=payment_hash, kind="deposit",
        )
        in_row["payment_hash_source"] = "chain_script"

        pairs, matched = detect_intra_transfers([out_row, in_row])

        self.assertEqual(pairs, [])
        self.assertEqual(matched, set())

    def test_detect_intra_transfers_folds_mixed_case_txid(self):
        # A txid recorded uppercase in one wallet and lowercase in another is the
        # same on-chain transaction; the grouping must fold case so the pair is
        # detected rather than split into a phantom disposal + acquisition.
        from kassiber.transfers import detect_intra_transfers

        txid = "cd" * 32
        out_row = _row("o", "wallet-a", "outbound", 50_000_000_000,
                       external_id=txid.upper())
        in_row = _row("i", "wallet-b", "inbound", 50_000_000_000,
                      external_id=txid.lower())
        pairs, matched = detect_intra_transfers([out_row, in_row])
        self.assertEqual(len(pairs), 1)
        self.assertEqual(matched, {"o", "i"})

    def test_detect_intra_transfers_rejects_shared_import_id(self):
        from kassiber.transfers import detect_intra_transfers

        out_row = _row(
            "o", "wallet-a", "outbound", 50_000_000_000,
            external_id="provider-batch", canonical_external=False,
        )
        in_row = _row(
            "i", "wallet-b", "inbound", 50_000_000_000,
            external_id="provider-batch", canonical_external=False,
        )
        self.assertEqual(detect_intra_transfers([out_row, in_row]), ([], set()))
        # The custody interpreter holds such legs for review
        # (`custody_interpreters.py`), so they never reach the engine as a pair.
        self.assertEqual(
            detect_unscoped_transfer_review_ids([out_row, in_row]), {"o", "i"}
        )

    def test_unscoped_review_folds_mixed_case_txid(self):
        txid = "cd" * 32
        out_row = _row(
            "o", "wallet-a", "outbound", 50_000_000_000,
            external_id=txid.upper(), canonical_external=False,
        )
        in_row = _row(
            "i", "wallet-b", "inbound", 50_000_000_000,
            external_id=txid.lower(), canonical_external=False,
        )

        self.assertEqual(
            detect_unscoped_transfer_review_ids([out_row, in_row]), {"o", "i"}
        )

    def test_canonical_shaped_provider_fallback_is_not_a_txid(self):
        from kassiber.transfers import detect_intra_transfers

        provider_id = "91" * 32
        raw = json.dumps(
            {"source": "bullbitcoin_wallet_csv", "swap_id": provider_id}
        )
        out_row = _row(
            "o",
            "wallet-a",
            "outbound",
            50_000_000_000,
            external_id=provider_id,
            raw_json=raw,
        )
        in_row = _row(
            "i",
            "wallet-b",
            "inbound",
            50_000_000_000,
            external_id=provider_id,
            raw_json=raw,
        )

        self.assertEqual(detect_intra_transfers([out_row, in_row]), ([], set()))

    def test_graphless_tx_hash_field_is_typed_physical_identity(self):
        from kassiber.transfers import detect_intra_transfers

        txid = "93" * 32
        raw = json.dumps({"Tx Hash": txid, "source": "wallet_csv"})
        out_row = _row(
            "o",
            "wallet-a",
            "outbound",
            50_000_000_000,
            external_id=txid,
            raw_json=raw,
        )
        in_row = _row(
            "i",
            "wallet-b",
            "inbound",
            50_000_000_000,
            external_id=txid,
            raw_json=raw,
        )

        pairs, matched = detect_intra_transfers([out_row, in_row])
        self.assertEqual(len(pairs), 1)
        self.assertEqual(matched, {"o", "i"})

    def test_core_receive_list_is_typed_physical_identity(self):
        from kassiber.transfers import onchain_transfer_scope

        txid = "94" * 32
        row = _row(
            "core-receive",
            "wallet-a",
            "inbound",
            50_000_000_000,
            external_id=txid,
            raw_json=json.dumps(
                [
                    {
                        "txid": txid,
                        "vout": 2,
                        "address": "bcrt1qowned",
                        "category": "receive",
                    }
                ]
            ),
        )
        row["config_json"] = json.dumps(
            {"chain": "bitcoin", "network": "regtest"}
        )

        self.assertEqual(
            onchain_transfer_scope(row),
            ("bitcoin", "regtest", txid, "BTC"),
        )

    def test_core_receive_list_with_conflicting_txid_is_unscoped(self):
        from kassiber.transfers import onchain_transfer_scope

        txid = "95" * 32
        row = _row(
            "core-receive-conflict",
            "wallet-a",
            "inbound",
            50_000_000_000,
            external_id=txid,
            raw_json=json.dumps(
                [
                    {
                        "txid": "96" * 32,
                        "vout": 2,
                        "address": "bcrt1qowned",
                        "category": "receive",
                    }
                ]
            ),
        )
        row["config_json"] = json.dumps(
            {"chain": "bitcoin", "network": "regtest"}
        )

        self.assertIsNone(onchain_transfer_scope(row))

    def test_liquid_scope_uses_consensus_asset_id(self):
        from kassiber.transfers import detect_intra_transfers

        txid = "92" * 32

        def liquid_row(row_id, wallet_id, direction, asset_id):
            row = _row(
                row_id,
                wallet_id,
                direction,
                50_000_000_000,
                external_id=txid,
                raw_json=json.dumps(
                    {
                        "txid": txid,
                        "chain": "liquid",
                        "network": "liquidv1",
                        "component": {"asset_id": asset_id, "asset": "LBTC"},
                    }
                ),
            )
            row["asset"] = "LBTC"
            return row

        out_row = liquid_row("o", "wallet-a", "outbound", "11" * 32)
        in_row = liquid_row("i", "wallet-b", "inbound", "22" * 32)

        self.assertEqual(detect_intra_transfers([out_row, in_row]), ([], set()))

    def test_detect_intra_transfers_requires_same_network(self):
        from kassiber.transfers import detect_intra_transfers

        txid = "de" * 32
        out_row = _row("o", "wallet-a", "outbound", 50_000_000_000, external_id=txid)
        in_row = _row("i", "wallet-b", "inbound", 50_000_000_000, external_id=txid)
        out_row["raw_json"] = json.dumps({"chain": "bitcoin", "network": "regtest"})
        in_row["raw_json"] = json.dumps({"chain": "bitcoin", "network": "main"})

        self.assertEqual(detect_intra_transfers([out_row, in_row]), ([], set()))


class LedgerGateTest(unittest.TestCase):
    """The normalizer gates a finalized projection still reaches.

    Each case runs ``build_ledger_state`` on a finalized projection: custody's
    own stages compile the rows where they can produce the shape, and the
    projection is built directly where custody holds the shape upstream but an
    authored component could still carry it.
    """

    def test_priced_inbound_books_at_its_value(self):
        # The importer gave only a value; the projection turns it into a rate
        # and the acquisition books at quantity times that rate.
        state = _ledger(
            FIFO,
            finalized(
                FIFO,
                [
                    row(
                        "tx-1", "A", "inbound", 50_000_000_000,
                        "2026-01-01T00:00:00Z", rate=None, fiat_value="30000",
                        kind="buy",
                    )
                ],
            ),
        )

        self.assertEqual(state.quarantines, [])
        [entry] = state.entries
        self.assertEqual(
            (entry["transaction_id"], entry["entry_type"], entry["wallet_id"]),
            ("tx-1", "acquisition", "A"),
        )
        self.assertEqual(entry["quantity"], Decimal("0.5"))
        self.assertEqual(entry["fiat_value"], Decimal("30000"))
        self.assertEqual(entry["unit_cost"], Decimal("60000"))

    def test_recorded_move_fee_books_as_transfer_fee(self):
        # 0.001 BTC moved with a 0.00005 BTC recorded network fee. The fee is
        # far above the max(1%, 2,500 sats) band, but both legs carry the same
        # principal, so nothing unrecognized left the source: the move books
        # and its recorded fee is the transfer fee.
        rows = [
            row("buy", "A", "inbound", 100_000_000_000, "2026-01-01T00:00:00Z", kind="buy"),
            row("move-out", "A", "outbound", 100_000_000, "2026-02-01T00:00:00Z", fee=5_000_000, rate="65000"),
            row("move-in", "B", "inbound", 100_000_000, "2026-02-01T00:10:00Z", rate="65000"),
        ]
        state = _ledger(FIFO, finalized(FIFO, rows, [pair("pair-1", "move-out", "move-in")]))

        self.assertEqual(state.quarantines, [])
        self.assertEqual(_entry_types(state, "move-out"), ["transfer_fee", "transfer_out"])
        self.assertEqual(_entry_types(state, "move-in"), ["transfer_in"])
        fee = next(entry for entry in state.entries if entry["entry_type"] == "transfer_fee")
        self.assertEqual(fee["quantity"], Decimal("-0.00005"))
        [audit] = state.intra_audit
        self.assertEqual(
            (audit["crypto_sent_msat"], audit["crypto_received_msat"], audit["crypto_fee_msat"]),
            (105_000_000, 100_000_000, 5_000_000),
        )
        self.assertEqual(audit["spot_price"], 65000.0)

    def test_unconfirmed_spend_is_held_not_disposed(self):
        # Custody holds an unconfirmed spend before projecting it; a leg an
        # authored component projects still carries the chain evidence, so the
        # engine keeps its own hold. Built directly for that reason.
        txid = chain_txid("pending-out")
        buy = projected("buy", "A", "inbound", 100_000_000_000, "2026-01-01T00:00:00Z", kind="buy")
        pending = projected(
            "pending-out", "A", "outbound", 50_000_000_000, "2026-02-01T00:00:00Z",
            rate="60000", kind="sell", external_id=txid,
            raw_json=graph_json(txid, confirmed=False),
        )
        state = _ledger(FIFO, direct([buy, pending]))

        self.assertEqual(_reasons(state), {"pending-out": "pending_onchain_confirmation"})
        self.assertEqual(_detail(state, "pending-out")["required_for"], "confirmed_chain_history")
        self.assertEqual(_entry_types(state, "pending-out"), [])
        self.assertEqual(_entry_types(state, "buy"), ["acquisition"])

    def test_unpriced_move_fee_holds_both_legs(self):
        # The fee cannot be priced, so the whole move waits rather than book a
        # zero-fee move that would leave the fee quantity spendable in the
        # source. The inbound leg is held too instead of vanishing.
        rows = [
            row("buy", "A", "inbound", 100_000_000_000, "2026-01-01T00:00:00Z", kind="buy"),
            row("move-out", "A", "outbound", 50_000_000_000, "2026-02-01T00:00:00Z", fee=100_000_000, rate=None),
            row("move-in", "B", "inbound", 50_000_000_000, "2026-02-01T00:10:00Z", rate=None),
        ]
        state = _ledger(FIFO, finalized(FIFO, rows, [pair("pair-1", "move-out", "move-in")]))

        self.assertEqual(
            _reasons(state),
            {"move-out": "missing_spot_price", "move-in": "missing_spot_price"},
        )
        primary = _detail(state, "move-out")
        self.assertEqual(primary["required_for"], "transfer_fee")
        self.assertNotIn("paired_leg", primary)
        self.assertIs(_detail(state, "move-in")["paired_leg"], True)
        self.assertEqual(_entry_types(state, "move-out"), [])
        self.assertEqual(_entry_types(state, "move-in"), [])

    def test_blocked_bundle_holds_every_move_in_it(self):
        # Two moves into B form one atomic bundle. The coarse price on the
        # first move's fee needs review, so the second move must not book
        # alone either: its legs are held as blocked by the bundle.
        buy_a = projected("buy-a", "A", "inbound", 60_000_000_000, "2026-01-01T00:00:00Z", kind="buy")
        buy_c = projected("buy-c", "C", "inbound", 40_000_000_000, "2026-01-01T00:00:00Z", kind="buy")
        out_a = projected("out-a", "A", "outbound", 50_000_000_000, "2026-02-01T00:00:00Z", side="move-out", fee=100_000_000, rate="65000", **COARSE)
        in_a = projected("in-b-a", "B", "inbound", 50_000_000_000, "2026-02-01T00:10:00Z", side="move-in", rate="65000")
        out_c = projected("out-c", "C", "outbound", 30_000_000_000, "2026-02-01T00:00:00Z", side="move-out", rate="65000")
        in_c = projected("in-b-c", "B", "inbound", 30_000_000_000, "2026-02-01T00:10:00Z", side="move-in", rate="65000")
        state = _ledger(
            COARSE_REVIEW,
            direct(
                [buy_a, buy_c, out_a, in_a, out_c, in_c],
                [
                    move(out_a, in_a, group_id="grouped-transfer"),
                    move(out_c, in_c, group_id="grouped-transfer"),
                ],
            ),
        )

        self.assertEqual(
            _reasons(state),
            {
                "out-a": "pricing_review_required",
                "in-b-a": "pricing_review_required",
                "out-c": "derived_transfer_group_blocked",
                "in-b-c": "derived_transfer_group_blocked",
            },
        )
        for transaction_id in ("out-c", "in-b-c"):
            detail = _detail(state, transaction_id)
            self.assertEqual(detail["transfer_group_id"], "grouped-transfer")
            self.assertEqual(detail["blocked_by_reason"], "pricing_review_required")
        self.assertEqual(state.intra_audit, [])
        self.assertEqual(
            sorted(entry["entry_type"] for entry in state.entries),
            ["acquisition", "acquisition"],
        )

    def test_held_move_contaminates_later_disposals_by_anchor(self):
        # The move's legs are projection rows with synthetic ids, while its
        # quarantine names the anchor transaction. The lot state is uncertain
        # from the held move on, so a later sale waits; an earlier one books.
        rows = [
            row("buy", "A", "inbound", 100_000_000_000, "2026-01-01T00:00:00Z", kind="buy"),
            row("early-sell", "A", "outbound", 1_000_000_000, "2026-01-15T00:00:00Z", rate="60000", kind="sell"),
            row("move-out", "A", "outbound", 50_000_000_000, "2026-02-01T00:00:00Z", fee=100_000_000, rate="65000", **COARSE),
            row("move-in", "B", "inbound", 50_000_000_000, "2026-02-01T00:10:00Z", rate="65000"),
            row("later-sell", "A", "outbound", 1_000_000_000, "2026-03-01T00:00:00Z", rate="70000", kind="sell"),
        ]
        state = _ledger(
            COARSE_REVIEW,
            finalized(COARSE_REVIEW, rows, [pair("pair-1", "move-out", "move-in")]),
        )

        self.assertEqual(_reasons(state)["move-out"], "pricing_review_required")
        self.assertEqual(_reasons(state)["later-sell"], "basis_provenance_incomplete")
        self.assertEqual(
            _detail(state, "later-sell")["lot_state_uncertain_since"],
            "2026-02-01T00:00:00Z",
        )
        self.assertEqual(_entry_types(state, "early-sell"), ["disposal"])
        self.assertEqual(_entry_types(state, "later-sell"), [])

    def test_privacy_hop_without_provenance_is_held(self):
        # Custody holds such an imported row before projecting it; a projected
        # row can still carry the evidence, so the engine keeps its own hold.
        buy = projected("buy", "A", "inbound", 100_000_000_000, "2026-01-01T00:00:00Z", kind="buy")
        hop = projected(
            "hop-out", "A", "outbound", 100_000_000, "2026-02-01T00:00:00Z",
            rate="60000", privacy_boundary="coinjoin",
            raw_json=json.dumps({"source": "wasabi_gethistory", "islikelycoinjoin": True}),
        )
        state = _ledger(FIFO, direct([buy, hop]))

        self.assertEqual(_reasons(state), {"hop-out": "privacy_hop_unresolved"})
        detail = _detail(state, "hop-out")
        self.assertEqual(detail["privacy_hop"], "coinjoin")
        self.assertEqual(detail["privacy_boundary"], "coinjoin")
        self.assertEqual(detail["direction"], "outbound")
        self.assertEqual(detail["required_for"], "explicit_user_owned_provenance")
        self.assertEqual(_entry_types(state, "hop-out"), [])

    def test_reviewed_coinjoin_move_crosses_the_privacy_boundary(self):
        # Custody marks a reviewed move whose leg carries privacy evidence as
        # a reviewed coinjoin link, and that link books as a transfer.
        rows = [
            row("buy", "A", "inbound", 100_000_000_000, "2026-01-01T00:00:00Z", kind="buy"),
            row("cj-out", "A", "outbound", 50_000_000_000, "2026-02-01T00:00:00Z", fee=100_000_000, rate="65000", privacy_boundary="coinjoin"),
            row("cj-in", "B", "inbound", 50_000_000_000, "2026-02-01T00:10:00Z", rate="65000"),
        ]
        inputs = finalized(FIFO, rows, [pair("pair-1", "cj-out", "cj-in", kind="whirlpool")])
        self.assertEqual(
            [item["kind"] for item in inputs.finalized_tax_projection.intra_pairs],
            ["custody-reviewed-coinjoin"],
        )
        state = _ledger(FIFO, inputs)

        self.assertEqual(state.quarantines, [])
        self.assertEqual(_entry_types(state, "cj-out"), ["transfer_fee", "transfer_out"])
        self.assertEqual(_entry_types(state, "cj-in"), ["transfer_in"])


class LightningPaymentHashEngineTest(unittest.TestCase):
    def test_persisted_carrying_pair_cannot_cross_bitcoin_networks(self):
        profile = {
            "id": "profile-1",
            "workspace_id": "workspace-1",
            "label": "Default",
            "fiat_currency": "USD",
            "tax_country": "generic",
            "tax_long_term_days": 365,
            "gains_algorithm": "FIFO",
        }
        wallet_refs = {
            wallet_id: {
                "id": wallet_id,
                "label": label,
                "wallet_account_id": "acct-1",
                "account_code": "treasury",
                "account_label": "Treasury",
            }
            for wallet_id, label in (("wallet-a", "Main"), ("wallet-b", "Regtest"))
        }

        def engine_row(tx_id, wallet_id, direction, amount, network, occurred_at):
            txid = hashlib.sha256(tx_id.encode()).hexdigest()
            wallet = wallet_refs[wallet_id]
            return {
                "id": tx_id,
                "workspace_id": "workspace-1",
                "profile_id": "profile-1",
                "wallet_id": wallet_id,
                "wallet_label": wallet["label"],
                "wallet_account_id": "acct-1",
                "account_code": "treasury",
                "account_label": "Treasury",
                "external_id": txid,
                "occurred_at": occurred_at,
                "created_at": occurred_at,
                "direction": direction,
                "asset": "BTC",
                "amount": amount,
                "fee": 0,
                "amount_includes_fee": 0,
                "fiat_currency": "USD",
                "fiat_rate": 40_000.0,
                "fiat_rate_exact": "40000",
                "fiat_value": None,
                "kind": "deposit" if direction == "inbound" else "withdrawal",
                "description": tx_id,
                "note": None,
                "raw_json": json.dumps(
                    {"txid": txid, "chain": "bitcoin", "network": network}
                ),
                "config_json": json.dumps(
                    {"chain": "bitcoin", "network": network}
                ),
                "excluded": 0,
                "payment_hash": None,
                "payment_hash_source": None,
            }

        rows = [
            engine_row(
                "acquisition", "wallet-a", "inbound", 100_000_000_000,
                "main", "2025-01-01T00:00:00Z",
            ),
            engine_row(
                "out", "wallet-a", "outbound", 50_000_000_000,
                "main", "2026-01-01T00:00:00Z",
            ),
            engine_row(
                "in", "wallet-b", "inbound", 50_000_000_000,
                "regtest", "2026-01-01T00:01:00Z",
            ),
        ]
        state = GenericRP2TaxEngine(profile).build_ledger_state(
            finalized_tax_inputs(
                profile,
                rows=rows,
                wallet_refs_by_id=wallet_refs,
                manual_pair_records=[
                    {
                        "id": "legacy-bad-pair",
                        "out_transaction_id": "out",
                        "in_transaction_id": "in",
                        "kind": "manual",
                        "policy": "carrying-value",
                        "out_amount": None,
                    }
                ],
            )
        )

        self.assertIn(
            "custody_quantity_unresolved",
            {item["reason"] for item in state.quarantines},
        )
        self.assertEqual(
            {item["transaction_id"] for item in state.quarantines},
            {"out", "in"},
        )
        self.assertFalse(
            [item for item in state.intra_audit if item.get("out_id") == "out"]
        )
        self.assertFalse(
            [
                item
                for item in state.entries
                if item.get("transaction_id") in {"out", "in"}
            ]
        )

        pair_only_state = GenericRP2TaxEngine(profile).build_ledger_state(
            finalized_tax_inputs(
                profile,
                rows=rows[1:],
                wallet_refs_by_id=wallet_refs,
                manual_pair_records=[
                    {
                        "id": "legacy-bad-pair",
                        "out_transaction_id": "out",
                        "in_transaction_id": "in",
                        "kind": "manual",
                        "policy": "carrying-value",
                        "out_amount": None,
                    }
                ],
            )
        )
        self.assertEqual(pair_only_state.entries, [])
        self.assertEqual(
            {item["transaction_id"] for item in pair_only_state.quarantines},
            {"out", "in"},
        )

        chained_rows = [
            *rows[:2],
            engine_row(
                "bridge", "wallet-a", "inbound", 50_000_000_000,
                "main", "2026-01-01T00:01:00Z",
            ),
            engine_row(
                "bad-destination", "wallet-b", "inbound", 50_000_000_000,
                "regtest", "2026-01-01T00:02:00Z",
            ),
        ]
        chained_state = GenericRP2TaxEngine(profile).build_ledger_state(
            finalized_tax_inputs(
                profile,
                rows=chained_rows,
                wallet_refs_by_id=wallet_refs,
                manual_pair_records=[
                    {
                        "id": "accepted-prefix",
                        "out_transaction_id": "out",
                        "in_transaction_id": "bridge",
                        "kind": "manual",
                        "policy": "carrying-value",
                    },
                    {
                        "id": "rejected-suffix",
                        "out_transaction_id": "bridge",
                        "in_transaction_id": "bad-destination",
                        "kind": "manual",
                        "policy": "carrying-value",
                    },
                ],
            )
        )
        self.assertFalse(
            [
                item
                for item in chained_state.entries
                if item.get("transaction_id")
                in {"out", "bridge", "bad-destination"}
            ]
        )
        self.assertEqual(
            {
                item["transaction_id"]
                for item in chained_state.quarantines
            },
            {"out", "bridge", "bad-destination"},
        )
        self.assertIn(
            "custody_quantity_unresolved",
            {item["reason"] for item in chained_state.quarantines},
        )

    def test_same_wallet_payment_hash_books_fee_not_sell_buy(self):
        profile = {
            "id": "profile-1",
            "workspace_id": "workspace-1",
            "label": "Default",
            "fiat_currency": "USD",
            "tax_country": "generic",
            "tax_long_term_days": 365,
            "gains_algorithm": "FIFO",
        }
        wallet_refs = {
            "wallet-x": {
                "id": "wallet-x",
                "label": "Node",
                "wallet_account_id": "acct-1",
                "account_code": "treasury",
                "account_label": "Treasury",
            }
        }
        payment_hash = "ef" * 32

        def engine_row(tx_id, direction, amount, external_id, *, fee=0, payment_hash=None):
            native_hash_raw = (
                json.dumps(
                    {
                        "_kassiber_provenance": {
                            "import_source": "core-lightning"
                        },
                        "chain": "lightning",
                        "network": "main",
                    }
                )
                if payment_hash
                else "{}"
            )
            return {
                "id": tx_id,
                "workspace_id": "workspace-1",
                "profile_id": "profile-1",
                "wallet_id": "wallet-x",
                "wallet_label": "Node",
                "wallet_account_id": "acct-1",
                "account_code": "treasury",
                "account_label": "Treasury",
                "external_id": external_id,
                "occurred_at": "2026-01-01T00:00:00Z",
                "created_at": "2026-01-01T00:00:00Z",
                "direction": direction,
                "asset": "BTC",
                "amount": amount,
                "fee": fee,
                "fiat_currency": "USD",
                "fiat_rate": 40_000.0,
                "fiat_rate_exact": "40000",
                "fiat_value": None,
                "kind": "ln_pay" if direction == "outbound" else "ln_invoice",
                "description": tx_id,
                "note": None,
                "raw_json": native_hash_raw,
                "excluded": 0,
                "payment_hash": payment_hash,
                "payment_hash_source": "core_lightning" if payment_hash else None,
            }

        state = GenericRP2TaxEngine(profile).build_ledger_state(
            finalized_tax_inputs(
                profile,
                rows=[
                    engine_row("acq", "inbound", 100_000_000_000, "acq"),
                    engine_row(
                        "ln-pay",
                        "outbound",
                        5_000_000_000,
                        "ln-pay-external",
                        fee=1_000_000,
                        payment_hash=payment_hash,
                    ),
                    engine_row(
                        "ln-invoice",
                        "inbound",
                        5_000_000_000,
                        "ln-invoice-external",
                        payment_hash=payment_hash,
                    ),
                ],
                wallet_refs_by_id=wallet_refs,
                manual_pair_records=[],
            )
        )

        self.assertEqual(state.quarantines, [])
        entry_types = [entry["entry_type"] for entry in state.entries]
        self.assertEqual(entry_types.count("acquisition"), 1)
        self.assertIn("transfer_fee", entry_types)
        self.assertIn("transfer_out", entry_types)
        self.assertIn("transfer_in", entry_types)
        self.assertNotIn("disposal", entry_types)
        self.assertEqual(len(state.intra_audit), 1)
        self.assertEqual(state.intra_audit[0]["from_wallet_label"], "Node")
        self.assertEqual(state.intra_audit[0]["to_wallet_label"], "Node")
        self.assertAlmostEqual(state.intra_audit[0]["crypto_fee"], 0.00001)
        holdings = {
            label: float(totals["quantity"])
            for (_, label, _, _), totals in state.wallet_holdings.items()
        }
        self.assertAlmostEqual(holdings["Node"], 0.99999)


class TransferGateEngineTest(unittest.TestCase):
    def test_blocked_plain_transfer_quarantines_destination_leg(self):
        profile = {
            "id": "profile-1",
            "workspace_id": "workspace-1",
            "label": "Default",
            "fiat_currency": "USD",
            "tax_country": "generic",
            "tax_long_term_days": 365,
            "gains_algorithm": "FIFO",
        }
        wallet_refs = {
            "wallet-a": {
                "id": "wallet-a",
                "label": "Wallet A",
                "wallet_account_id": "acct-a",
                "account_code": "A",
                "account_label": "Account A",
            },
            "wallet-b": {
                "id": "wallet-b",
                "label": "Wallet B",
                "wallet_account_id": "acct-b",
                "account_code": "B",
                "account_label": "Account B",
            },
        }

        def engine_row(tx_id, wallet_id, direction):
            ref = wallet_refs[wallet_id]
            return authoritative_chain_observation({
                "id": tx_id,
                "workspace_id": "workspace-1",
                "profile_id": "profile-1",
                "wallet_id": wallet_id,
                "wallet_label": ref["label"],
                "wallet_account_id": ref["wallet_account_id"],
                "account_code": ref["account_code"],
                "account_label": ref["account_label"],
                "external_id": "44" * 32,
                "occurred_at": "2026-01-01T00:00:00Z",
                "created_at": "2026-01-01T00:00:00Z",
                "direction": direction,
                "asset": "BTC",
                "amount": 100_000_000_000,
                "fee": 0,
                "fiat_currency": "USD",
                "fiat_rate": 40_000.0,
                "fiat_rate_exact": "40000",
                "fiat_value": None,
                "kind": "transfer",
                "description": tx_id,
                "note": None,
                "raw_json": json.dumps({"txid": "44" * 32}),
                "excluded": 0,
                "payment_hash": None,
            })

        state = GenericRP2TaxEngine(profile).build_ledger_state(
            finalized_tax_inputs(
                profile,
                rows=[
                    engine_row("move-out", "wallet-a", "outbound"),
                    engine_row("move-in", "wallet-b", "inbound"),
                ],
                wallet_refs_by_id=wallet_refs,
                manual_pair_records=[],
            )
        )

        reasons_by_id = {q["transaction_id"]: q["reason"] for q in state.quarantines}
        self.assertEqual(
            reasons_by_id,
            {
                "move-out": "insufficient_lots",
                "move-in": "derived_transfer_group_blocked",
            },
        )
        partner_detail = json.loads(
            next(
                q["detail_json"]
                for q in state.quarantines
                if q["transaction_id"] == "move-in"
            )
        )
        self.assertTrue(partner_detail["paired_leg"])
        self.assertEqual(partner_detail["blocked_by_reason"], "insufficient_lots")


class BuildTaxQuarantineTest(unittest.TestCase):
    profile = {"id": "p", "workspace_id": "w"}

    def test_uses_real_id_for_synthetic_rows(self):
        # A synthetic engine-only row (e.g. a direct-payout or cross-split leg)
        # must quarantine against its real tx so the journal_quarantines FK to
        # transactions(id) holds — otherwise the whole `journals process` aborts.
        row = {"id": "direct-payout:abc:out", "journal_transaction_id": "real-tx"}
        q = build_tax_quarantine(self.profile, row, "reason", {})
        self.assertEqual(q["transaction_id"], "real-tx")

    def test_uses_own_id_for_real_rows(self):
        q = build_tax_quarantine(self.profile, {"id": "real-tx"}, "reason", {})
        self.assertEqual(q["transaction_id"], "real-tx")


class DedupeQuarantinesTest(unittest.TestCase):
    profile = {"id": "p", "workspace_id": "w"}

    def _q(self, tx_id, reason, detail):
        # build through the real builder so detail_json serialization matches prod
        return build_tax_quarantine(self.profile, {"id": tx_id}, reason, detail)

    def test_distinct_reasons_for_same_tx_merge_into_one_row(self):
        # Two engine legs (e.g. a direct-payout synthetic leg AND another drop)
        # that map back to the same real tx would otherwise collide on
        # journal_quarantines' PRIMARY KEY and abort the whole run. They must
        # collapse to ONE row, with the later reason preserved under detail.
        out = dedupe_quarantines(
            [
                self._q("real-tx", "missing_cost_basis", {"required": 1.0}),
                self._q("real-tx", "basis_provenance_incomplete", {"since": "x"}),
            ]
        )
        self.assertEqual(len(out), 1)
        self.assertEqual(out[0]["transaction_id"], "real-tx")
        self.assertEqual(out[0]["reason"], "missing_cost_basis")
        detail = json.loads(out[0]["detail_json"])
        self.assertEqual(detail["required"], 1.0)
        self.assertEqual(len(detail["additional_reasons"]), 1)
        self.assertEqual(
            detail["additional_reasons"][0]["reason"], "basis_provenance_incomplete"
        )
        self.assertEqual(detail["additional_reasons"][0]["detail"]["since"], "x")

    def test_exact_duplicate_for_same_tx_is_dropped_silently(self):
        out = dedupe_quarantines(
            [
                self._q("real-tx", "missing_cost_basis", {"required": 1.0}),
                self._q("real-tx", "missing_cost_basis", {"required": 1.0}),
            ]
        )
        self.assertEqual(len(out), 1)
        self.assertNotIn("additional_reasons", json.loads(out[0]["detail_json"]))

    def test_distinct_transactions_preserved_in_order(self):
        out = dedupe_quarantines(
            [
                self._q("tx-b", "r1", {}),
                self._q("tx-a", "r2", {}),
            ]
        )
        self.assertEqual([q["transaction_id"] for q in out], ["tx-b", "tx-a"])

    def test_root_cause_outranks_an_earlier_downstream_reason(self):
        # The projection emits a gap hold's own basis barrier before the hold
        # (alphabetical order). The stored reason must name the custody gap,
        # not the barrier every later row in the pool shares.
        out = dedupe_quarantines(
            [
                self._q("tx", "custody_basis_barrier", {"root_transaction_ids": ["tx"]}),
                self._q(
                    "tx",
                    "custody_quantity_unresolved",
                    {"blocker_code": "custody_gap_review_required"},
                ),
                self._q("tx", "missing_spot_price", {"asset": "BTC"}),
            ]
        )
        self.assertEqual(len(out), 1)
        self.assertEqual(out[0]["reason"], "custody_quantity_unresolved")
        detail = json.loads(out[0]["detail_json"])
        self.assertEqual(detail["blocker_code"], "custody_gap_review_required")
        self.assertEqual(
            [item["reason"] for item in detail["additional_reasons"]],
            ["custody_basis_barrier", "missing_spot_price"],
        )
        self.assertEqual(
            quarantine_catalog.primary_reasons(
                [
                    self._q("tx", "custody_basis_barrier", {}),
                    self._q("tx", "custody_quantity_unresolved", {}),
                    self._q("tx", "missing_spot_price", {}),
                ]
            ),
            {"tx": "custody_quantity_unresolved"},
        )

    def test_downstream_only_rows_keep_their_first_reason(self):
        out = dedupe_quarantines(
            [
                self._q("tx", "custody_basis_barrier", {"root_transaction_ids": ["root"]}),
                self._q("tx", "transfer_pair_dependency_blocked", {}),
            ]
        )
        self.assertEqual(out[0]["reason"], "custody_basis_barrier")


def _conflict(label, wallet, amount, occurred_at, *, confirmed, side=None, **fields):
    """A spend of the shared conflict prevout, as a projection row."""
    txid = chain_txid(label)
    return projected(
        label, wallet, "outbound", amount, occurred_at, side=side, rate="60000",
        external_id=txid,
        raw_json=graph_json(
            txid,
            confirmed=confirmed,
            spends=[(chain_txid("conflict-prevout"), 0)],
        ),
        **fields,
    )


class ConflictPairInteractionTest(unittest.TestCase):
    """A spend that lost a conflict over its input is held, never booked.

    Two transactions spend the same prevout; the confirmed one wins and books
    (it comes first, so the held loser cannot taint it). The losing spend is
    held for review whether it stands alone or is one leg of a move, and the
    move's other leg then books on its own.
    """

    def test_conflict_loser_move_leg_is_held_and_its_partner_books_alone(self):
        buy = projected("buy", "A", "inbound", 30_000_000_000, "2026-01-01T00:00:00Z", kind="buy")
        loser_out = _conflict("loser-out", "A", 10_000_000_000, "2026-02-01T00:05:00Z", confirmed=False, side="move-out", fee=5_000)
        partner_in = projected("partner-in", "B", "inbound", 10_000_000_000, "2026-02-01T00:10:00Z", side="move-in", rate="60000")
        winner = _conflict("winner-out", "A", 5_000_000_000, "2026-02-01T00:00:00Z", confirmed=True, kind="sell")
        state = _ledger(FIFO, direct([buy, loser_out, partner_in, winner], [move(loser_out, partner_in)]))

        self.assertEqual(_reasons(state)["loser-out"], "conflicting_spend")
        self.assertEqual(_entry_types(state, "loser-out"), [])
        self.assertNotIn("winner-out", _reasons(state))
        self.assertEqual(_entry_types(state, "winner-out"), ["disposal"])
        self.assertEqual(_entry_types(state, "partner-in"), ["acquisition"])
        self.assertEqual(state.intra_audit, [])

    def test_conflict_loser_sale_is_held_not_disposed(self):
        buy = projected("buy", "A", "inbound", 30_000_000_000, "2026-01-01T00:00:00Z", kind="buy")
        loser = _conflict("loser-sale", "A", 10_000_000_000, "2026-02-01T00:05:00Z", confirmed=False, kind="sell")
        winner = _conflict("winner-out", "A", 5_000_000_000, "2026-02-01T00:00:00Z", confirmed=True, kind="sell")
        state = _ledger(FIFO, direct([buy, loser, winner]))

        self.assertEqual(_reasons(state)["loser-sale"], "conflicting_spend")
        self.assertEqual(_entry_types(state, "loser-sale"), [])
        self.assertNotIn("winner-out", _reasons(state))
        self.assertEqual(_entry_types(state, "winner-out"), ["disposal"])


def _with_id(item, row_id):
    return {**item, "id": row_id}


class AustrianSelfTransferRegimeTest(unittest.TestCase):
    """Alt/Neu regimes of Austrian self-transfers, as the journal records them.

    A move's fee is a disposal; its ``transfer_fee`` description starts with
    the fee's regime. Both legs' descriptions carry the regime and how much
    Alt left the source and reached the destination (``at_alt_out`` and
    ``at_alt_in``, in msat).
    """

    def _move_fee_description(self, neu_acquisition_id, *, neu_first):
        # An Alt lot (2020) and a Neu acquisition at the same instant as a
        # fee-bearing self-transfer. The projection row ids decide whether
        # the Neu acquisition sorts before or after the move by id alone;
        # ``neu_first`` lists it before or after the move's legs.
        alt = projected("alt", "A", "inbound", 30_000_000_000, "2020-06-01T00:00:00Z", rate="10000", kind="buy")
        neu = _with_id(
            projected("neu", "A", "inbound", 40_000_000_000, "2025-02-01T00:00:00Z", rate="60000", kind="buy"),
            f"custody-tax:{neu_acquisition_id}:0:40000000000:inbound",
        )
        out_row = _with_id(
            projected("move-out", "A", "outbound", 50_000_000_000, "2025-02-01T00:00:00Z", side="move-out", fee=100_000_000, rate="60000"),
            "custody-tax:mmm-move:0:50000000000:move-out",
        )
        in_row = _with_id(
            projected("move-in", "B", "inbound", 50_000_000_000, "2025-02-01T00:00:00Z", side="move-in", rate="60000"),
            "custody-tax:mmm-move:0:50000000000:move-in",
        )
        rows = [alt, neu, out_row, in_row] if neu_first else [alt, out_row, in_row, neu]
        state = _ledger(MOVING_AVERAGE_AT, direct(rows, [move(out_row, in_row)]))
        self.assertEqual(state.quarantines, [])
        return _description(state, "move-out", "transfer_fee")

    def test_self_transfer_fee_regime_is_order_independent(self):
        # Economically identical books must not differ by the Neu acquisition's
        # id or its place in the projection. The move post-dates the cutoff
        # and Neu inventory exists, so the fee is Neu every way.
        for neu_acquisition_id in ("aaa-neu", "zzz-neu"):
            for neu_first in (True, False):
                with self.subTest(neu_acquisition_id=neu_acquisition_id, neu_first=neu_first):
                    description = self._move_fee_description(neu_acquisition_id, neu_first=neu_first)
                    self.assertTrue(description.startswith("at_regime=neu "), description)

    def test_self_transfer_fee_honors_regime_override(self):
        rows = [
            row("alt", "A", "inbound", 30_000_000_000, "2020-06-01T00:00:00Z", rate="10000", kind="buy"),
            row("neu", "A", "inbound", 40_000_000_000, "2025-02-01T00:00:00Z", rate="60000", kind="buy"),
            row("move-out", "A", "outbound", 50_000_000_000, "2025-02-01T00:00:00Z", fee=100_000_000, rate="60000", at_regime_override="alt"),
            row("move-in", "B", "inbound", 50_000_000_000, "2025-02-01T00:10:00Z", rate="60000"),
        ]
        state = _ledger(
            MOVING_AVERAGE_AT,
            finalized(MOVING_AVERAGE_AT, rows, [pair("pair-1", "move-out", "move-in")]),
        )

        self.assertEqual(
            _description(state, "move-out", "transfer_fee"),
            "at_regime=alt at_pool=default move-out via Cold",
        )

    def test_conflict_loser_pair_excluded_from_regime_inference(self):
        # A move whose out leg lost a conflict is not booked, so its partner
        # books as a plain Neu acquisition in B. Regime inference must treat
        # it the same way: had the move carried A's Alt coins into B, the
        # later move out of B would draw Alt.
        alt_buy = projected("alt-buy", "A", "inbound", 50_000_000_000, "2020-06-01T00:00:00Z", rate="8000", kind="buy")
        loser_out = _conflict("loser-out", "A", 10_000_000_000, "2024-06-01T00:00:00Z", confirmed=False, side="move-out")
        partner_in = projected("partner-in", "B", "inbound", 10_000_000_000, "2024-06-01T00:10:00Z", side="move-in", rate="60000")
        winner = _conflict("winner-out", "A", 1_000_000_000, "2024-06-01T00:05:00Z", confirmed=True, kind="sell")
        later_out = projected("later-out", "B", "outbound", 5_000_000_000, "2025-06-01T00:00:00Z", side="move-out", fee=10_000_000, rate="90000")
        later_in = projected("later-in", "C", "inbound", 5_000_000_000, "2025-06-01T00:10:00Z", side="move-in", rate="90000")
        state = _ledger(
            MOVING_AVERAGE_AT,
            direct(
                [alt_buy, loser_out, partner_in, winner, later_out, later_in],
                [move(loser_out, partner_in), move(later_out, later_in)],
            ),
        )

        self.assertEqual(_reasons(state)["loser-out"], "conflicting_spend")
        self.assertEqual(
            _description(state, "partner-in", "acquisition"),
            "at_regime=neu at_pool=default partner-in via Hot",
        )
        self.assertTrue(_description(state, "later-out", "transfer_fee").startswith("at_regime=neu "))
        self.assertEqual(
            _description(state, "later-in", "transfer_in"),
            "at_regime=neu at_alt_out=0 at_alt_in=0 at_pool=default Transfer Hot -> Exchange",
        )

    def test_move_from_alt_only_wallet_carries_alt_to_destination(self):
        # After the cutoff a move from a wallet holding only Alt coins carries
        # Alt: its legs record the Alt quantity, and a later sale from the
        # destination is Alt.
        rows = [
            row("alt", "A", "inbound", 100_000_000_000, "2020-06-01T00:00:00Z", rate="10000", kind="buy"),
            row("move-out", "A", "outbound", 50_000_000_000, "2025-02-01T00:00:00Z", rate="60000"),
            row("move-in", "B", "inbound", 50_000_000_000, "2025-02-01T00:10:00Z", rate="60000"),
            row("sell", "B", "outbound", 10_000_000_000, "2025-03-01T00:00:00Z", rate="60000", kind="sell"),
        ]
        state = _ledger(
            MOVING_AVERAGE_AT,
            finalized(MOVING_AVERAGE_AT, rows, [pair("pair-1", "move-out", "move-in")]),
        )

        self.assertEqual(state.quarantines, [])
        for transaction_id, entry_type in (("move-out", "transfer_out"), ("move-in", "transfer_in")):
            self.assertEqual(
                _description(state, transaction_id, entry_type),
                "at_regime=alt at_alt_out=50000000000 at_alt_in=50000000000 at_pool=default Transfer Cold -> Hot",
            )
        self.assertEqual(
            _description(state, "sell", "disposal"),
            "at_regime=alt at_pool=default sell via Hot",
        )


class AustrianRegimeElectionAuditTest(unittest.TestCase):
    """Mixed-holding Neu-first is a KryptowährungsVO designation, not a forced
    outcome — the disposal's description must say so; forced/override regimes
    must not."""

    def _sale_description(self, *acquisitions, **sale_fields):
        rows = [
            *acquisitions,
            row("sell", "A", "outbound", 10_000_000_000, "2025-06-01T00:00:00Z", rate="60000", kind="sell", **sale_fields),
        ]
        state = _ledger(MOVING_AVERAGE_AT, finalized(MOVING_AVERAGE_AT, rows))
        self.assertEqual(state.quarantines, [])
        return _description(state, "sell", "disposal")

    @staticmethod
    def _alt_acquisition():
        return row("alt-in", "A", "inbound", 30_000_000_000, "2020-06-01T00:00:00Z", rate="10000", kind="buy")

    @staticmethod
    def _neu_acquisition():
        return row("neu-in", "A", "inbound", 40_000_000_000, "2025-02-01T00:00:00Z", rate="60000", kind="buy")

    def test_mixed_holdings_disposal_records_wahlrecht_election(self):
        self.assertEqual(
            self._sale_description(self._alt_acquisition(), self._neu_acquisition()),
            "at_regime=neu at_regime_basis=wahlrecht at_pool=default sell via Cold",
        )

    def test_pure_alt_wallet_disposal_is_forced_not_election(self):
        self.assertEqual(
            self._sale_description(self._alt_acquisition()),
            "at_regime=alt at_pool=default sell via Cold",
        )

    def test_pure_neu_wallet_disposal_is_forced_not_election(self):
        self.assertEqual(
            self._sale_description(self._neu_acquisition()),
            "at_regime=neu at_pool=default sell via Cold",
        )

    def test_explicit_override_suppresses_election_marker(self):
        self.assertEqual(
            self._sale_description(
                self._alt_acquisition(),
                self._neu_acquisition(),
                at_regime_override="alt",
            ),
            "at_regime=alt at_pool=default sell via Cold",
        )


if __name__ == "__main__":
    unittest.main()
