"""An exchange withdrawal that names its txid moves basis to the owned wallet.

The platform's withdrawal row and the owned wallet's observed receipt share
one physical txid. When the platform's principal is exactly the receipt's
one owned output, the journal books an own-account transfer with the
``exchange_txid_exact`` evidence. Anything less exact still asks, as before.
"""

from __future__ import annotations

from decimal import Decimal
import json
from pathlib import Path
import tempfile
import unittest
import uuid

from kassiber.cli import handlers
from kassiber.core import accounts, imports as core_imports, wallets as core_wallets
from kassiber.core.book_network import apply_book_network, plan_book_network
from kassiber.core.custody_components import activate_component, create_component
from kassiber.core.custody_review_terms import delete_pair_dismissal, dismiss_pair_candidate
from kassiber.core.exchange_imports import (
    normalize_binance_records,
    normalize_coinbase_records,
    normalize_kraken_records,
)
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.importers import normalize_river_record, normalize_strike_record
from tests.custody_tax_helpers import persist_authoritative_chain_observation


TXID = "7a" * 32
OWNED_ADDRESS = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"
OWNED_SCRIPT = "0014751e76e8199196d454941c45d1b3a323f1433bd6"
SECOND_ADDRESS = "bc1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3qccfmv3"
SECOND_SCRIPT = "00201863143c14c5166804bd19203356da136c985678cd4d27a1b8c6329604903262"
PLATFORM_SCRIPT = "0014" + "11" * 20
PLATFORM_CHANGE_SCRIPT = "0014" + "22" * 20
MSAT = 1_000
# The platform sends 600,000 sat on chain and charges 10,000 sat for it.
PRINCIPAL_SATS = 600_000
FEE_SATS = 10_000


def _btc(sats):
    return f"{Decimal(sats) / Decimal(100_000_000):.8f}"


def strike_records(*, txid=TXID, amount_sats=PRINCIPAL_SATS, fee_sats=FEE_SATS, reference="withdraw"):
    """Strike: ``Amount BTC`` is what leaves on chain, ``Fee BTC`` the charge."""

    rows = [
        {
            "Reference": "buy", "Date & Time (UTC)": "Jan 01 2024 10:00:00",
            "Transaction Type": "Buy", "Amount EUR": "-1000", "Fee EUR": "1",
            "Amount BTC": "0.01000000", "BTC Price": "100000",
        },
        {
            "Reference": reference, "Date & Time (UTC)": "Feb 01 2024 10:00:00",
            "Transaction Type": "Send", "Amount BTC": f"-{_btc(amount_sats)}",
            "Fee BTC": _btc(fee_sats), "BTC Price": "100000",
            "Transaction Hash": txid or "",
        },
    ]
    return [normalize_strike_record(row) for row in rows]


def river_records(*, txid=TXID, amount_sats=PRINCIPAL_SATS, fee_sats=FEE_SATS, reference="RIV-WD-1"):
    """River: ``Sent Amount`` leaves on chain; ``Total Amount`` adds the fee."""

    header = {
        "Sent Amount": "", "Sent Currency": "", "Received Amount": "",
        "Received Currency": "", "Fee Amount": "0", "Fee Currency": "BTC",
        "Bitcoin Price Amount": "100000", "Bitcoin Price Currency": "EUR",
        "Transaction ID": "", "Reference Code": "",
    }
    rows = [
        {
            **header, "Date": "2024-01-01T10:00:00Z", "Reference Code": "RIV-BUY-1",
            "Transaction Type": "Buy", "Sent Amount": "1000.00", "Sent Currency": "EUR",
            "Received Amount": "0.01000000", "Received Currency": "BTC",
            "Fee Amount": "1.00", "Fee Currency": "EUR",
        },
        {
            **header, "Date": "2024-02-01T10:00:00Z", "Reference Code": reference,
            "Transaction Type": "Withdrawal", "Sent Amount": _btc(amount_sats),
            "Sent Currency": "BTC", "Fee Amount": _btc(fee_sats),
            "Total Amount": f"-{_btc(amount_sats + fee_sats)}", "Total Currency": "BTC",
            "Method": "on-chain", "Destination": OWNED_ADDRESS,
            "Transaction ID": txid or "",
        },
    ]
    return [normalize_river_record(row) for row in rows]


SOURCES = {
    "strike": ("strike", "strike_csv", strike_records),
    "river": ("river", "river_csv", river_records),
}


def _receipt_graph(txid, outputs, *, inputs=None):
    return {
        "txid": txid,
        "chain": "bitcoin",
        "network": "main",
        "vin": inputs or [
            {
                "txid": "aa" * 32,
                "vout": 0,
                "prevout": {"scriptpubkey": PLATFORM_SCRIPT, "value": 10_000_000},
            }
        ],
        "vout": [
            {"n": n, "scriptpubkey": script, "value": value}
            for n, (script, value) in enumerate(outputs)
        ],
    }


class ExchangeTxidBook:
    """A platform account, owned cold wallets and the receipts they observed."""

    def __init__(self, root):
        self.conn = open_db(Path(root))
        workspace = accounts.create_workspace(self.conn, "W")
        self.workspace_id = workspace["id"]
        profile = accounts.create_profile(
            self.conn, workspace["id"], "P", "EUR", "FIFO", "generic", 365
        )
        self.profile_id = profile["id"]
        self.hooks = handlers._import_coordinator_hooks()

    def close(self):
        self.conn.close()

    def wallet(self, label, kind, config=None):
        created = core_wallets.create_wallet(
            self.conn, self.workspace_id, self.profile_id, label, kind,
            config=config or {},
        )
        return handlers.resolve_wallet(self.conn, self.profile_id, created["id"])

    def platform(self, kind, source, records, *, label=None):
        wallet = self.wallet(label or kind.title(), kind, {"source_format": source})
        profile = handlers.resolve_scope(self.conn, self.workspace_id, self.profile_id)[1]
        core_imports.import_records_into_wallet(
            self.conn, profile, wallet, records, source, self.hooks,
        )
        return wallet

    def cold(self, label="Cold", address=OWNED_ADDRESS):
        return self.wallet(
            label, "address",
            {"addresses": [address], "chain": "bitcoin", "network": "main"},
        )

    def receipt(self, wallet, *, amount_sats=PRINCIPAL_SATS, outputs=None, inputs=None):
        outputs = outputs or [
            (OWNED_SCRIPT, amount_sats), (PLATFORM_CHANGE_SCRIPT, 3_000_000),
        ]
        row_id = f"receipt-{wallet['label']}"
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id,
                external_id_kind, fingerprint, occurred_at, direction, asset,
                amount, fee, fiat_currency, fiat_rate, fiat_rate_exact,
                raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, 'txid', ?, '2024-02-01T10:30:00Z',
                     'inbound', 'BTC', ?, 0, 'EUR', 100000, '100000', ?,
                     '2024-02-01T10:30:00Z')
            """,
            (
                row_id, self.workspace_id, self.profile_id, wallet["id"], TXID,
                f"fp-{row_id}", amount_sats * MSAT,
                json.dumps(_receipt_graph(TXID, outputs, inputs=inputs), sort_keys=True),
            ),
        )
        persist_authoritative_chain_observation(self.conn, row_id)
        self.conn.commit()
        return row_id

    def process(self):
        return handlers.process_journals(self.conn, self.workspace_id, self.profile_id)

    def withdrawal_ids(self):
        return [
            row["id"]
            for row in self.conn.execute(
                "SELECT t.id FROM transactions t JOIN wallets w ON w.id = t.wallet_id "
                "WHERE t.direction = 'outbound' AND w.kind != 'address' ORDER BY t.id"
            )
        ]

    def quarantines(self):
        return {
            row["transaction_id"]: row["reason"]
            for row in self.conn.execute(
                "SELECT transaction_id, reason FROM journal_quarantines WHERE profile_id=?",
                (self.profile_id,),
            )
        }

    def decisions(self):
        return [
            dict(row)
            for row in self.conn.execute(
                "SELECT source_transaction_id, target_transaction_id, state, reason, "
                "source_end_msat - source_start_msat AS amount_msat "
                "FROM journal_custody_decisions WHERE profile_id=? ORDER BY decision_id",
                (self.profile_id,),
            )
        ]

    def entries(self, transaction_id):
        return sorted(
            (row["entry_type"], int(row["quantity"]))
            for row in self.conn.execute(
                "SELECT entry_type, quantity FROM journal_entries WHERE transaction_id=?",
                (transaction_id,),
            )
        )

    def holdings(self):
        return {
            row["label"]: (int(row["quantity"]), Decimal(str(row["cost_basis"])))
            for row in self.conn.execute(
                "SELECT w.label, h.quantity, h.cost_basis FROM journal_wallet_holdings h "
                "JOIN wallets w ON w.id = h.wallet_id WHERE h.profile_id=?",
                (self.profile_id,),
            )
        }

    def suggestions(self):
        return [
            (item["method"], item["confidence"])
            for item in handlers.suggest_transfer_candidates(
                self.conn, self.workspace_id, self.profile_id
            )["candidates"]
        ]


class _SourceCases:
    """The exact match and every held variant, for one platform source."""

    source = ""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.book = ExchangeTxidBook(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.addCleanup(self.book.close)
        self.kind, self.format, self.records = SOURCES[self.source]

    def import_platform(self, **overrides):
        return self.book.platform(self.kind, self.format, self.records(**overrides))

    def assert_held(self, withdrawal, reason="recorded_transfer_authority_conflict"):
        self.assertFalse(
            any(item["reason"] == "exchange_txid_exact" for item in self.book.decisions())
        )
        self.assertEqual(self.book.quarantines().get(withdrawal), reason)

    def test_exact_withdrawal_books_an_automatic_transfer(self):
        self.import_platform()
        receipt = self.book.receipt(self.book.cold())
        self.book.process()
        [withdrawal] = self.book.withdrawal_ids()
        self.assertEqual(self.book.quarantines(), {})
        self.assertEqual(
            self.book.decisions(),
            [{
                "source_transaction_id": withdrawal, "target_transaction_id": receipt,
                "state": "internal_verified", "reason": "exchange_txid_exact",
                "amount_msat": PRINCIPAL_SATS * MSAT,
            }],
        )
        self.assertEqual(
            self.book.entries(withdrawal),
            [
                ("transfer_fee", -FEE_SATS * MSAT),
                ("transfer_out", -(PRINCIPAL_SATS + FEE_SATS) * MSAT),
            ],
        )
        self.assertEqual(self.book.entries(receipt), [("transfer_in", PRINCIPAL_SATS * MSAT)])
        # The cold wallet carries its 0.006 BTC at the purchase's unit cost;
        # the platform keeps 0.0039 BTC after the 0.0001 BTC fee.
        holdings = self.book.holdings()
        self.assertEqual(
            {label: quantity for label, (quantity, _basis) in holdings.items()},
            {"Cold": PRINCIPAL_SATS * MSAT, self.kind.title(): 390_000 * MSAT},
        )
        cold_basis, platform_basis = holdings["Cold"][1], holdings[self.kind.title()][1]
        self.assertGreater(cold_basis, 0)
        self.assertEqual(
            (cold_basis * 390).quantize(Decimal("0.01")),
            (platform_basis * 600).quantize(Decimal("0.01")),
        )

    def test_gross_amount_is_not_the_received_output(self):
        # The platform reports the debit including its fee as the amount.
        self.import_platform(amount_sats=PRINCIPAL_SATS + FEE_SATS)
        self.book.receipt(self.book.cold())
        self.book.process()
        [withdrawal] = self.book.withdrawal_ids()
        self.assert_held(withdrawal)

    def test_missing_txid_still_asks(self):
        self.import_platform(txid=None)
        self.book.receipt(self.book.cold())
        self.book.process()
        self.assertEqual(self.book.decisions(), [])
        self.assertIn(("heuristic", "strong"), self.book.suggestions())

    def test_destination_owned_by_another_wallet_stays_held(self):
        self.import_platform()
        # The receiving wallet does not own the output it reports.
        self.book.receipt(self.book.cold(address=SECOND_ADDRESS))
        self.book.process()
        [withdrawal] = self.book.withdrawal_ids()
        self.assert_held(withdrawal)

    def test_owned_input_stays_held(self):
        self.import_platform()
        cold = self.book.cold()
        self.book.cold("Savings", SECOND_ADDRESS)
        self.book.receipt(
            cold,
            inputs=[{
                "txid": "aa" * 32, "vout": 0,
                "prevout": {"scriptpubkey": SECOND_SCRIPT, "value": 10_000_000},
            }],
        )
        self.book.process()
        [withdrawal] = self.book.withdrawal_ids()
        # A book wallet funded the transaction, so its debit is missing.
        self.assert_held(withdrawal, "ownership_transfer_source_missing")

    def test_two_owned_outputs_stay_held(self):
        self.import_platform(amount_sats=2 * PRINCIPAL_SATS)
        cold = self.book.wallet(
            "Cold", "address",
            {"addresses": [OWNED_ADDRESS, SECOND_ADDRESS], "chain": "bitcoin", "network": "main"},
        )
        self.book.receipt(
            cold, amount_sats=2 * PRINCIPAL_SATS,
            outputs=[(OWNED_SCRIPT, PRINCIPAL_SATS), (SECOND_SCRIPT, PRINCIPAL_SATS)],
        )
        self.book.process()
        [withdrawal] = self.book.withdrawal_ids()
        self.assert_held(withdrawal)

    def test_two_receiving_wallets_stay_held(self):
        self.import_platform()
        outputs = [(OWNED_SCRIPT, PRINCIPAL_SATS // 2), (SECOND_SCRIPT, PRINCIPAL_SATS // 2)]
        self.book.receipt(self.book.cold(), amount_sats=PRINCIPAL_SATS // 2, outputs=outputs)
        self.book.receipt(
            self.book.cold("Savings", SECOND_ADDRESS),
            amount_sats=PRINCIPAL_SATS // 2, outputs=outputs,
        )
        self.book.process()
        self.assertFalse(
            any(item["reason"] == "exchange_txid_exact" for item in self.book.decisions())
        )
        [withdrawal] = self.book.withdrawal_ids()
        self.assertIn(withdrawal, self.book.quarantines())

    def test_two_platform_withdrawals_stay_held(self):
        self.import_platform()
        self.book.platform(
            self.kind, self.format, self.records(reference="duplicate")[1:],
            label="Second account",
        )
        self.book.receipt(self.book.cold())
        self.book.process()
        self.assertEqual(len(self.book.withdrawal_ids()), 2)
        self.assertFalse(
            any(item["reason"] == "exchange_txid_exact" for item in self.book.decisions())
        )

    def test_dismissal_undoes_the_automatic_transfer(self):
        self.import_platform()
        receipt = self.book.receipt(self.book.cold())
        [withdrawal] = self.book.withdrawal_ids()
        dismissal = dismiss_pair_candidate(
            self.book.conn, workspace_id=self.book.workspace_id,
            profile_id=self.book.profile_id, out_transaction_id=withdrawal,
            in_transaction_id=receipt, invalidate=True,
        )
        self.book.process()
        self.assert_held(withdrawal)
        delete_pair_dismissal(
            self.book.conn, profile_id=self.book.profile_id, dismissal_id=dismissal["id"]
        )
        self.book.process()
        self.assertEqual(
            [item["reason"] for item in self.book.decisions()], ["exchange_txid_exact"]
        )


class StrikeExchangeTxidTest(_SourceCases, unittest.TestCase):
    source = "strike"


class RiverExchangeTxidTest(_SourceCases, unittest.TestCase):
    source = "river"


class OtherPlatformWithdrawalTest(unittest.TestCase):
    """API withdrawals the importers cannot scope to a txid still ask."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.book = ExchangeTxidBook(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.addCleanup(self.book.close)

    def _check(self, kind, records):
        self.book.platform(kind, f"{kind}:api", records)
        self.book.receipt(self.book.cold())
        self.book.process()
        self.assertEqual(self.book.decisions(), [])
        self.assertEqual(len(self.book.withdrawal_ids()), 1)

    def test_kraken_ledger_names_no_txid(self):
        self._check("kraken", normalize_kraken_records({"result": {"ledger": {
            "L1": {"type": "withdrawal", "asset": "XXBT", "amount": "-0.006",
                   "fee": "0.0001", "time": 1706781600},
        }}}))

    def test_coinbase_send_keeps_its_network_object(self):
        self._check("coinbase", normalize_coinbase_records({"currency": "BTC", "transactions": [{
            "id": "cb-send", "type": "send", "created_at": "2024-02-01T10:00:00Z",
            "amount": {"amount": "-0.006", "currency": "BTC"},
            "native_amount": {"amount": "-600", "currency": "EUR"},
            "network": {"hash": TXID, "status": "confirmed"},
        }]}))

    def test_binance_withdrawal_is_not_a_matched_platform(self):
        self._check("binance", normalize_binance_records({"withdrawals": [{
            "id": "w1", "coin": "BTC", "amount": "0.006",
            "transactionFee": "0.0001", "status": 6, "txId": TXID,
            "applyTime": 1706781600000,
        }]}))

    def test_binance_withdrawal_with_its_coin_network_imports(self):
        # Binance's withdraw history names the coin's transfer network;
        # "BTC" is Bitcoin on chain, admitted as mainnet evidence.
        self._check("binance", binance_records(network="BTC"))


def binance_records(*, network, deposit=False):
    row = {
        "id": "w1", "coin": "BTC", "network": network, "amount": "0.006",
        "transactionFee": "0.0001", "status": 6, "txId": TXID,
        "applyTime": 1706781600000,
    }
    return normalize_binance_records({"deposits" if deposit else "withdrawals": [row]})


class BinanceNetworkAdmissionTest(unittest.TestCase):
    """Binance's ``network`` names a transfer rail, not a book environment."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.book = ExchangeTxidBook(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.addCleanup(self.book.close)

    def imported(self):
        return self.book.conn.execute(
            "SELECT COUNT(*) FROM transactions WHERE profile_id=?", (self.book.profile_id,)
        ).fetchone()[0]

    def refused(self, records, kind="binance", label=None):
        with self.assertRaises(AppError) as caught:
            self.book.platform(kind, f"{kind}:api", records, label=label)
        self.assertEqual(self.imported(), 0)
        return caught.exception

    def bind(self, environment):
        args = {
            "environment": environment,
            "chain_instance_id": str(uuid.uuid4()) if environment == "regtest" else None,
            "declared_wallet_ids": [],
        }
        plan = plan_book_network(self.book.conn, self.book.profile_id, args)
        apply_book_network(self.book.conn, self.book.profile_id, {**args, "plan_id": plan["plan_id"]})

    def test_btc_network_deposits_and_withdrawals_import_into_a_mainnet_book(self):
        self.bind("main")
        self.book.platform("binance", "binance:api", [
            *binance_records(network="BTC", deposit=True),
            *binance_records(network="BTC"),
        ])
        self.assertEqual(self.imported(), 2)
        [raw] = {
            row[0] for row in self.book.conn.execute(
                "SELECT json_extract(raw_json, '$.network') FROM transactions"
            )
        }
        self.assertEqual(raw, "BTC")

    def test_btc_network_is_mainnet_evidence_and_refused_in_a_regtest_book(self):
        self.bind("regtest")
        error = self.refused(binance_records(network="BTC"))
        self.assertEqual(error.code, "book_network_mismatch")
        self.assertEqual(str(error), "Observation belongs to a different book environment")

    def test_other_binance_networks_are_still_refused(self):
        for network in ("LIGHTNING", "BSC", "BEP20", "ETH", "btc", " BTC"):
            with self.subTest(network=network):
                error = self.refused(binance_records(network=network), label=repr(network))
                self.assertEqual(error.code, "book_network_mismatch")

    def test_the_btc_code_is_not_liquid_evidence(self):
        [record] = binance_records(network="BTC")
        record = {**record, "asset": "LBTC"}
        error = self.refused([record])
        self.assertEqual(str(error), "Observation contradicts its configured network")

    def test_the_btc_code_is_only_read_for_binance_wallets(self):
        error = self.refused(binance_records(network="BTC"), kind="custom")
        self.assertEqual(str(error), "Observation contradicts its configured network")


if __name__ == "__main__":
    unittest.main()
