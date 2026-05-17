"""Pure-function tests for the swap-candidate matcher.

Each test feeds the matcher synthetic dict rows so we pin the
public contract (which fields it reads, what shape the candidates
take) independently of SQLite. The matcher has no I/O — these
exercise the full algorithm end-to-end.
"""

import unittest

from kassiber.core.transfer_matching import (
    CONFIDENCE_EXACT,
    CONFIDENCE_STRONG,
    KIND_MANUAL,
    KIND_PEG_IN,
    KIND_PEG_OUT,
    KIND_SUBMARINE_SWAP,
    METHOD_HEURISTIC,
    METHOD_PAYMENT_HASH,
    POLICY_CARRYING_VALUE,
    POLICY_TAXABLE,
    compute_swap_fee,
    default_kind_for,
    default_policy_for,
    fee_threshold_msat,
    suggest_swap_candidates,
)


def _row(**overrides):
    base = {
        "id": "row-id",
        "profile_id": "prof",
        "wallet_id": "wallet-a",
        "wallet_label": "Wallet A",
        "wallet_kind": "descriptor",
        "external_id": "",
        "payment_hash": None,
        "occurred_at": "2026-03-14T17:30:00Z",
        "direction": "outbound",
        "asset": "BTC",
        "amount": 100_000_000_000,  # 1 BTC in msat
        "excluded": 0,
    }
    base.update(overrides)
    return base


_PAY_HASH = "ab" * 32


class FeeThresholdTests(unittest.TestCase):
    def test_percentage_wins_when_amount_large(self):
        # 1 BTC = 100_000_000 sats. 1% = 1_000_000 sats = 1_000_000_000 msat.
        self.assertEqual(
            fee_threshold_msat(out_amount_msat=100_000_000_000, fee_pct_max=0.01, fee_sats_min=2500),
            1_000_000_000,
        )

    def test_absolute_floor_wins_when_amount_small(self):
        # 0.0001 BTC = 10_000 sats. 1% = 100 sats = 100_000 msat.
        # Absolute floor 2500 sats = 2_500_000 msat dominates.
        self.assertEqual(
            fee_threshold_msat(out_amount_msat=10_000_000, fee_pct_max=0.01, fee_sats_min=2500),
            2_500_000,
        )

    def test_zero_amount_yields_floor(self):
        self.assertEqual(
            fee_threshold_msat(0, 0.01, 2500),
            2_500_000,
        )


class ComputeSwapFeeTests(unittest.TestCase):
    def test_positive_fee_when_principal_shrunk(self):
        msat, kind = compute_swap_fee(100, 80)
        self.assertEqual(msat, 20)
        self.assertEqual(kind, "combined")

    def test_negative_fee_when_inbound_exceeds_outbound(self):
        msat, _ = compute_swap_fee(80, 100)
        self.assertEqual(msat, -20)


class DefaultKindTests(unittest.TestCase):
    def test_lightning_to_chain_is_submarine_swap(self):
        self.assertEqual(default_kind_for("BTC", "BTC", "phoenix", "descriptor"), KIND_SUBMARINE_SWAP)
        self.assertEqual(default_kind_for("BTC", "LBTC", "phoenix", "descriptor"), KIND_SUBMARINE_SWAP)

    def test_chain_to_chain_btc_to_lbtc_is_peg_in(self):
        self.assertEqual(default_kind_for("BTC", "LBTC", "descriptor", "descriptor"), KIND_PEG_IN)

    def test_chain_to_chain_lbtc_to_btc_is_peg_out(self):
        self.assertEqual(default_kind_for("LBTC", "BTC", "descriptor", "descriptor"), KIND_PEG_OUT)

    def test_unknown_shape_falls_back_to_manual(self):
        self.assertEqual(default_kind_for("BTC", "BTC", "descriptor", "descriptor"), KIND_MANUAL)


class DefaultPolicyTests(unittest.TestCase):
    def test_at_profile_gets_carrying_value(self):
        self.assertEqual(default_policy_for("at"), POLICY_CARRYING_VALUE)
        self.assertEqual(default_policy_for("AT"), POLICY_CARRYING_VALUE)

    def test_generic_profile_gets_taxable(self):
        self.assertEqual(default_policy_for("generic"), POLICY_TAXABLE)
        self.assertEqual(default_policy_for(None), POLICY_TAXABLE)


class PaymentHashExactMatchTests(unittest.TestCase):
    def test_lightning_to_chain_pair_via_payment_hash(self):
        out = _row(
            id="lnsend",
            wallet_id="phoenix",
            wallet_label="Phoenix",
            wallet_kind="phoenix",
            payment_hash=_PAY_HASH,
            direction="outbound",
            occurred_at="2026-03-14T17:30:00Z",
            amount=100_000_000,
        )
        receive = _row(
            id="liquidrecv",
            wallet_id="liquid",
            wallet_label="Liquid Slip77",
            wallet_kind="descriptor",
            payment_hash=_PAY_HASH,
            direction="inbound",
            asset="LBTC",
            occurred_at="2026-03-14T17:32:00Z",
            amount=99_500_000,
        )
        candidates = suggest_swap_candidates([out, receive], tax_country="at")
        self.assertEqual(len(candidates), 1)
        candidate = candidates[0]
        self.assertEqual(candidate.confidence, CONFIDENCE_EXACT)
        self.assertEqual(candidate.method, METHOD_PAYMENT_HASH)
        self.assertEqual(candidate.out_id, "lnsend")
        self.assertEqual(candidate.in_id, "liquidrecv")
        self.assertEqual(candidate.default_kind, KIND_SUBMARINE_SWAP)
        self.assertEqual(candidate.default_policy, POLICY_CARRYING_VALUE)
        self.assertEqual(candidate.swap_fee_msat, 500_000)

    def test_same_wallet_payment_hash_pair_skipped(self):
        out = _row(id="a", wallet_id="w", payment_hash=_PAY_HASH, direction="outbound")
        inbound = _row(id="b", wallet_id="w", payment_hash=_PAY_HASH, direction="inbound")
        self.assertEqual(suggest_swap_candidates([out, inbound]), [])

    def test_same_asset_transfer_defaults_to_carrying_value_for_generic_profile(self):
        out = _row(
            id="cold-out",
            wallet_id="cold",
            wallet_label="Cold",
            direction="outbound",
            asset="BTC",
            amount=100_000_000_000,
        )
        inbound = _row(
            id="hot-in",
            wallet_id="hot",
            wallet_label="Hot",
            direction="inbound",
            asset="BTC",
            occurred_at="2026-03-14T17:32:00Z",
            amount=99_990_000_000,
        )
        candidates = suggest_swap_candidates([out, inbound], tax_country="generic")
        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].default_policy, POLICY_CARRYING_VALUE)


class HeuristicMatchTests(unittest.TestCase):
    def test_same_txid_self_transfer_skipped_before_heuristic(self):
        out = _row(
            id="cold-out",
            external_id="same-chain-txid",
            wallet_id="cold",
            wallet_label="Cold",
            direction="outbound",
            asset="BTC",
            amount=100_100_000_000,
        )
        inbound = _row(
            id="hot-in",
            external_id="same-chain-txid",
            wallet_id="hot",
            wallet_label="Hot",
            direction="inbound",
            asset="BTC",
            occurred_at="2026-03-14T17:32:00Z",
            amount=100_000_000_000,
        )
        self.assertEqual(suggest_swap_candidates([out, inbound], tax_country="at"), [])

    def test_same_txid_cross_asset_not_treated_as_self_transfer(self):
        out = _row(
            id="btc-out",
            external_id="shared-provider-id",
            wallet_id="onchain",
            direction="outbound",
            asset="BTC",
            amount=100_000_000,
        )
        inbound = _row(
            id="liquid-in",
            external_id="shared-provider-id",
            wallet_id="liquid",
            direction="inbound",
            asset="LBTC",
            occurred_at="2026-03-14T17:32:00Z",
            amount=99_500_000,
        )
        candidates = suggest_swap_candidates([out, inbound], tax_country="at")
        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].default_kind, KIND_PEG_IN)

    def test_pegout_within_window_paired(self):
        out = _row(
            id="lbtc-out",
            wallet_id="liquid",
            wallet_label="Liquid",
            wallet_kind="descriptor",
            asset="LBTC",
            direction="outbound",
            occurred_at="2026-03-14T17:30:00Z",
            amount=124_262_750_000,  # 0.12426275 BTC msat
        )
        inbound = _row(
            id="btc-in",
            wallet_id="onchain",
            wallet_label="On-chain",
            wallet_kind="descriptor",
            asset="BTC",
            direction="inbound",
            occurred_at="2026-03-14T17:32:00Z",
            amount=124_132_980_000,  # 0.12413298 BTC msat
        )
        candidates = suggest_swap_candidates([out, inbound], tax_country="at")
        self.assertEqual(len(candidates), 1)
        candidate = candidates[0]
        self.assertEqual(candidate.confidence, CONFIDENCE_STRONG)
        self.assertEqual(candidate.method, METHOD_HEURISTIC)
        self.assertEqual(candidate.default_kind, KIND_PEG_OUT)
        # 0.12426275 - 0.12413298 = 0.00012977 BTC = 12_977_000 msat
        self.assertEqual(candidate.swap_fee_msat, 129_770_000)
        # 1% of 0.12426275 BTC = ~0.00124262 BTC > 0.00012977 BTC → within threshold.

    def test_fee_outside_tolerance_rejected(self):
        out = _row(id="o", wallet_id="A", direction="outbound", amount=100_000_000_000)  # 1 BTC
        inbound = _row(id="i", wallet_id="B", direction="inbound", amount=50_000_000_000)  # 0.5 BTC
        self.assertEqual(suggest_swap_candidates([out, inbound], tax_country="at"), [])

    def test_outside_time_window_rejected(self):
        out = _row(
            id="o",
            wallet_id="A",
            direction="outbound",
            asset="LBTC",
            occurred_at="2026-03-14T00:00:00Z",
        )
        inbound = _row(
            id="i",
            wallet_id="B",
            direction="inbound",
            asset="BTC",
            occurred_at="2026-03-16T00:00:00Z",
            amount=99_500_000_000,
        )
        self.assertEqual(
            suggest_swap_candidates([out, inbound], time_window_seconds=24 * 3600, tax_country="at"),
            [],
        )

    def test_inbound_larger_than_outbound_rejected(self):
        out = _row(id="o", wallet_id="A", direction="outbound", amount=100, asset="LBTC")
        inbound = _row(id="i", wallet_id="B", direction="inbound", amount=200, asset="BTC")
        self.assertEqual(suggest_swap_candidates([out, inbound], tax_country="at"), [])

    def test_absolute_fee_floor_admits_small_swap(self):
        out = _row(id="o", wallet_id="A", direction="outbound", amount=10_000_000, asset="LBTC")  # 0.0001 BTC
        inbound = _row(id="i", wallet_id="B", direction="inbound", amount=8_000_000, asset="BTC")  # 0.00008 BTC
        # 1% of 10_000_000 msat = 100_000 msat = 100 sats. Floor 2500 sats = 2_500_000 msat.
        # Delta is 2_000_000 msat = 2_000 sats, below floor → admitted.
        candidates = suggest_swap_candidates([out, inbound], tax_country="at")
        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].confidence, CONFIDENCE_STRONG)


class ConflictClusteringTests(unittest.TestCase):
    def test_two_heuristic_candidates_share_leg_get_same_cluster_id(self):
        out = _row(id="o", wallet_id="A", asset="LBTC", direction="outbound", amount=124_262_750_000)
        in1 = _row(id="i1", wallet_id="B", asset="BTC", direction="inbound",
                   amount=124_132_980_000, occurred_at="2026-03-14T17:32:00Z")
        in2 = _row(id="i2", wallet_id="C", asset="BTC", direction="inbound",
                   amount=124_132_980_000, occurred_at="2026-03-14T17:33:00Z")
        candidates = suggest_swap_candidates([out, in1, in2], tax_country="at")
        self.assertEqual(len(candidates), 2)
        self.assertEqual(candidates[0].conflict_set_id, candidates[1].conflict_set_id)

    def test_exact_dominates_heuristic_with_overlap(self):
        # Exact (payment_hash) and heuristic candidates that share the same
        # outbound leg: exact wins, heuristic drops out.
        out = _row(
            id="o",
            wallet_id="A",
            wallet_kind="phoenix",
            payment_hash=_PAY_HASH,
            direction="outbound",
            amount=100_000_000,
        )
        exact_in = _row(
            id="exact_in",
            wallet_id="B",
            wallet_kind="descriptor",
            payment_hash=_PAY_HASH,
            direction="inbound",
            asset="LBTC",
            occurred_at="2026-03-14T17:32:00Z",
            amount=99_500_000,
        )
        heuristic_in = _row(
            id="heuristic_in",
            wallet_id="C",
            wallet_kind="descriptor",
            direction="inbound",
            asset="LBTC",
            occurred_at="2026-03-14T17:33:00Z",
            amount=99_400_000,
        )
        candidates = suggest_swap_candidates([out, exact_in, heuristic_in], tax_country="at")
        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].in_id, "exact_in")
        self.assertEqual(candidates[0].confidence, CONFIDENCE_EXACT)


class PairAndDismissalSuppressionTests(unittest.TestCase):
    def _legs(self):
        return [
            _row(
                id="o",
                wallet_id="A",
                wallet_kind="phoenix",
                payment_hash=_PAY_HASH,
                direction="outbound",
                amount=100_000_000,
            ),
            _row(
                id="i",
                wallet_id="B",
                wallet_kind="descriptor",
                payment_hash=_PAY_HASH,
                direction="inbound",
                asset="LBTC",
                occurred_at="2026-03-14T17:32:00Z",
                amount=99_500_000,
            ),
        ]

    def test_active_pair_record_skips_pairing(self):
        candidates = suggest_swap_candidates(
            self._legs(),
            pair_records=[{"out_transaction_id": "o", "in_transaction_id": "i", "deleted_at": None}],
            tax_country="at",
        )
        self.assertEqual(candidates, [])

    def test_soft_deleted_pair_does_not_skip(self):
        candidates = suggest_swap_candidates(
            self._legs(),
            pair_records=[
                {"out_transaction_id": "o", "in_transaction_id": "i", "deleted_at": "2026-04-01T00:00:00Z"}
            ],
            tax_country="at",
        )
        self.assertEqual(len(candidates), 1)

    def test_active_dismissal_drops_candidate(self):
        candidates = suggest_swap_candidates(
            self._legs(),
            dismissals=[
                {
                    "out_transaction_id": "o",
                    "in_transaction_id": "i",
                    "expires_at": "2099-01-01T00:00:00Z",
                }
            ],
            tax_country="at",
            now_iso="2026-06-01T00:00:00Z",
        )
        self.assertEqual(candidates, [])

    def test_expired_dismissal_re_surfaces_candidate(self):
        candidates = suggest_swap_candidates(
            self._legs(),
            dismissals=[
                {
                    "out_transaction_id": "o",
                    "in_transaction_id": "i",
                    "expires_at": "2025-01-01T00:00:00Z",
                }
            ],
            tax_country="at",
            now_iso="2026-06-01T00:00:00Z",
        )
        self.assertEqual(len(candidates), 1)


class ExcludedRowsTests(unittest.TestCase):
    def test_excluded_rows_ignored(self):
        out = _row(id="o", wallet_id="A", payment_hash=_PAY_HASH, direction="outbound", excluded=1)
        inbound = _row(id="i", wallet_id="B", payment_hash=_PAY_HASH, direction="inbound", asset="LBTC")
        self.assertEqual(suggest_swap_candidates([out, inbound], tax_country="at"), [])


if __name__ == "__main__":
    unittest.main()
