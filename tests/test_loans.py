import tempfile
import unittest
from decimal import Decimal

from kassiber.core import loans as core_loans
from kassiber.core.engines.base import TaxEngineLedgerInputs
from kassiber.core.engines.rp2 import GenericRP2TaxEngine
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.time_utils import now_iso


def _profile():
    return {
        "id": "p1",
        "workspace_id": "w1",
        "label": "BA",
        "tax_country": "at",
        "gains_algorithm": "moving_average_at",
    }


def _wallet_refs():
    return {
        "onchain": {
            "id": "onchain",
            "label": "onchain",
            "wallet_account_id": "acct-1",
            "account_code": "A",
            "account_label": "Account A",
        },
    }


def _row(tx_id, direction, amount_msat, occurred_at, *, fiat_rate=50_000, wallet_id="onchain"):
    return {
        "id": tx_id,
        "wallet_id": wallet_id,
        "wallet_label": wallet_id,
        "asset": "BTC",
        "direction": direction,
        "amount": amount_msat,
        "fee": 0,
        "fiat_rate": fiat_rate,
        "fiat_value": None,
        "kind": "deposit" if direction == "inbound" else "withdrawal",
        "description": tx_id,
        "note": None,
        "external_id": tx_id,
        "occurred_at": occurred_at,
    }


def _run(rows, loan_legs=()):
    return GenericRP2TaxEngine(_profile()).build_ledger_state(
        TaxEngineLedgerInputs(
            rows=rows,
            wallet_refs_by_id=_wallet_refs(),
            manual_pair_records=[],
            loan_legs=list(loan_legs),
        )
    )


def _btc_quantity(result):
    return sum(
        totals["quantity"]
        for key, totals in result.account_holdings.items()
        if key[3] == "BTC"
    )


def _has_disposal(result):
    # A realized disposal shows up as a tax_summary row with a non-zero quantity.
    return any(
        Decimal(str(row.get("quantity", 0) or 0)) != 0
        for row in result.tax_summary
    )


ONE_BTC = 100_000_000_000  # msat


class LoanTaxClassificationTest(unittest.TestCase):
    """The leg role drives classification: a collateral lock/release is a
    non-event (coins stay in the owned pool, encumbered), a liquidation books the
    one real disposal. Driven through the engine end to end."""

    def test_collateral_lock_is_suppressed_not_a_disposal(self):
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z"),
            _row("lock", "outbound", ONE_BTC, "2025-06-01T00:00:00Z"),
        ]
        # Baseline: with no loan leg, the outbound is booked as a disposal.
        baseline = _run(rows)
        self.assertEqual(_btc_quantity(baseline), Decimal("0"))
        self.assertTrue(_has_disposal(baseline))

        # Tagged as a collateral lock: suppressed — the coin stays owned.
        tagged = _run(rows, [{"transaction_id": "lock", "role": "collateral_lock"}])
        self.assertEqual(_btc_quantity(tagged), Decimal("1"))
        self.assertFalse(_has_disposal(tagged))

    def test_lock_and_release_round_trip_is_net_zero(self):
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z"),
            _row("lock", "outbound", ONE_BTC, "2025-06-01T00:00:00Z"),
            _row("release", "inbound", ONE_BTC, "2025-07-01T00:00:00Z"),
        ]
        result = _run(
            rows,
            [
                {"transaction_id": "lock", "role": "collateral_lock"},
                {"transaction_id": "release", "role": "collateral_release"},
            ],
        )
        # Lock and release both suppressed: still exactly the original 1 BTC, and
        # NO disposal and NO second acquisition were booked.
        self.assertEqual(_btc_quantity(result), Decimal("1"))
        self.assertFalse(_has_disposal(result))

    def test_liquidation_books_a_disposal(self):
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z", fiat_rate=50_000),
            _row("seize", "outbound", ONE_BTC, "2025-06-01T00:00:00Z", fiat_rate=60_000),
        ]
        # A liquidation falls through to the normal disposal path (NOT suppressed).
        result = _run(rows, [{"transaction_id": "seize", "role": "liquidation"}])
        self.assertEqual(_btc_quantity(result), Decimal("0"))
        self.assertTrue(_has_disposal(result))

    def test_altvermoegen_survives_lock_release_round_trip(self):
        # Pre-2021 Altvermögen coin, round-tripped through a loan escrow, then sold
        # post-cutoff. The release must NOT create a fresh Neu lot; the sale draws
        # the original Alt lot, so its basis is preserved (no quarantine, holdings
        # net to zero after the real sale).
        rows = [
            _row("buy-2020", "inbound", ONE_BTC, "2020-05-01T00:00:00Z", fiat_rate=10_000),
            _row("lock", "outbound", ONE_BTC, "2021-06-01T00:00:00Z"),
            _row("release", "inbound", ONE_BTC, "2021-07-01T00:00:00Z"),
            _row("sell", "outbound", ONE_BTC, "2026-01-01T00:00:00Z", fiat_rate=80_000),
        ]
        result = _run(
            rows,
            [
                {"transaction_id": "lock", "role": "collateral_lock"},
                {"transaction_id": "release", "role": "collateral_release"},
            ],
        )
        # The real sale disposed the coin; nothing left, and nothing quarantined
        # (a broken round-trip would strand the lock as a disposal or quarantine
        # the sale for missing basis).
        self.assertEqual(_btc_quantity(result), Decimal("0"))
        self.assertEqual(len(result.quarantines), 0)
        self.assertTrue(_has_disposal(result))


class LoanCrudTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.conn = open_db(self._tmp.name)
        self.conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES('w1', 'Main', ?)",
            (now_iso(),),
        )
        self.conn.execute(
            "INSERT INTO profiles(id, workspace_id, label, fiat_currency, tax_country, created_at) "
            "VALUES('p1', 'w1', 'Book', 'EUR', 'at', ?)",
            (now_iso(),),
        )
        self.conn.execute(
            "INSERT INTO wallets(id, workspace_id, profile_id, label, kind, created_at) "
            "VALUES('wal1', 'w1', 'p1', 'onchain', 'descriptor', ?)",
            (now_iso(),),
        )
        self.conn.execute(
            "INSERT INTO transactions(id, workspace_id, profile_id, wallet_id, fingerprint, asset, "
            "direction, amount, fee, occurred_at, kind, created_at) "
            "VALUES('tx1', 'w1', 'p1', 'wal1', 'fp1', 'BTC', 'outbound', 100000000000, 0, ?, 'withdrawal', ?)",
            (now_iso(), now_iso()),
        )
        self.conn.commit()

    def tearDown(self):
        self.conn.close()
        self._tmp.cleanup()

    def test_create_loan_with_preset_seeds_custody(self):
        loan = core_loans.create_loan(self.conn, "w1", "p1", preset_id="firefish")
        self.assertEqual(loan["custody_type"], "non_custodial_presigned")
        self.assertEqual(loan["control_mechanism"], "presigned_only")
        self.assertEqual(loan["preset_label"], "Firefish")
        self.assertEqual(loan["status"], "open")

    def test_explicit_kwargs_override_preset(self):
        loan = core_loans.create_loan(
            self.conn, "w1", "p1", preset_id="firefish", custody_type="custodial_segregated"
        )
        self.assertEqual(loan["custody_type"], "custodial_segregated")

    def test_invalid_custody_type_rejected(self):
        with self.assertRaises(AppError) as ctx:
            core_loans.create_loan(self.conn, "w1", "p1", custody_type="nonsense")
        self.assertEqual(ctx.exception.code, "validation")

    def test_invalid_leg_role_rejected(self):
        loan = core_loans.create_loan(self.conn, "w1", "p1")
        with self.assertRaises(AppError) as ctx:
            core_loans.create_loan_leg(self.conn, "w1", "p1", loan["id"], role="bogus", transaction_id="tx1")
        self.assertEqual(ctx.exception.code, "validation")

    def test_onchain_role_requires_transaction_id(self):
        loan = core_loans.create_loan(self.conn, "w1", "p1")
        with self.assertRaises(AppError) as ctx:
            core_loans.create_loan_leg(self.conn, "w1", "p1", loan["id"], role="collateral_lock")
        self.assertEqual(ctx.exception.code, "validation")

    def test_offchain_role_allows_null_transaction_id(self):
        loan = core_loans.create_loan(self.conn, "w1", "p1")
        leg = core_loans.create_loan_leg(
            self.conn, "w1", "p1", loan["id"], role="principal_draw", on_chain_present=False
        )
        self.assertEqual(leg["role"], "principal_draw")
        self.assertIsNone(leg["transaction_id"])

    def test_duplicate_transaction_leg_conflicts(self):
        loan = core_loans.create_loan(self.conn, "w1", "p1")
        core_loans.create_loan_leg(self.conn, "w1", "p1", loan["id"], role="collateral_lock", transaction_id="tx1")
        with self.assertRaises(AppError) as ctx:
            core_loans.create_loan_leg(self.conn, "w1", "p1", loan["id"], role="liquidation", transaction_id="tx1")
        self.assertEqual(ctx.exception.code, "conflict")

    def test_role_map_only_includes_active_onchain_legs(self):
        loan = core_loans.create_loan(self.conn, "w1", "p1")
        core_loans.create_loan_leg(self.conn, "w1", "p1", loan["id"], role="collateral_lock", transaction_id="tx1")
        core_loans.create_loan_leg(self.conn, "w1", "p1", loan["id"], role="principal_draw")
        role_map = core_loans.load_loan_leg_role_map(self.conn, "p1")
        self.assertEqual(role_map, {"tx1": "collateral_lock"})

    def test_action_items_flag_missing_lock_and_rehyp(self):
        core_loans.create_loan(self.conn, "w1", "p1", custody_type="custodial_rehypothecated", rehypothecation="allowed")
        actions = {item["action"] for item in core_loans.loan_action_items(self.conn, "p1")}
        self.assertIn("needs_lock", actions)
        self.assertIn("rehyp_review", actions)

    def test_healthy_paired_loan_has_no_standing_status(self):
        # signal-not-reassurance: a lock + close-out leaves no action chip.
        loan = core_loans.create_loan(self.conn, "w1", "p1", custody_type="non_custodial_multisig", status="repaid")
        core_loans.create_loan_leg(self.conn, "w1", "p1", loan["id"], role="collateral_lock", transaction_id="tx1")
        actions = core_loans.loan_action_items(self.conn, "p1")
        self.assertEqual(actions, [])


if __name__ == "__main__":
    unittest.main()
