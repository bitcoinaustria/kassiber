"""Pure-function tests for the swap-candidate matcher.

Each test feeds the matcher synthetic dict rows so we pin the
public contract (which fields it reads, what shape the candidates
take) independently of SQLite. The matcher has no I/O — these
exercise the full algorithm end-to-end.
"""

import unittest
from datetime import datetime, timedelta, timezone

from kassiber.core.transfer_matching import (
    _deterministic_self_transfer_ids,
    CONFIDENCE_EXACT,
    CONFIDENCE_STRONG,
    KIND_CHAIN_SWAP,
    KIND_MANUAL,
    KIND_PEG_IN,
    KIND_PEG_OUT,
    KIND_REVERSE_SUBMARINE_SWAP,
    KIND_SUBMARINE_SWAP,
    KIND_SWAP_REFUND,
    METHOD_HEURISTIC,
    METHOD_HTLC_REFUND,
    METHOD_PAYMENT_HASH,
    METHOD_PROVIDER_SWAP_ID,
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
        "raw_json": "{}",
        "occurred_at": "2026-03-14T17:30:00Z",
        "direction": "outbound",
        "asset": "BTC",
        "amount": 100_000_000_000,  # 1 BTC in msat
        "fee": 0,
        "amount_includes_fee": 0,
        "kind": "",
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

    def test_outbound_fee_component_included_when_separate(self):
        msat, kind = compute_swap_fee(100, 80, 3)
        self.assertEqual(msat, 23)
        self.assertEqual(kind, "combined")


class DefaultKindTests(unittest.TestCase):
    def test_lightning_to_chain_is_submarine_swap(self):
        self.assertEqual(default_kind_for("BTC", "BTC", "phoenix", "descriptor"), KIND_SUBMARINE_SWAP)
        self.assertEqual(default_kind_for("BTC", "LBTC", "phoenix", "descriptor"), KIND_SUBMARINE_SWAP)

    def test_chain_to_chain_btc_to_lbtc_is_peg_in(self):
        self.assertEqual(default_kind_for("BTC", "LBTC", "descriptor", "descriptor"), KIND_PEG_IN)

    def test_chain_to_chain_lbtc_to_btc_is_peg_out(self):
        self.assertEqual(default_kind_for("LBTC", "BTC", "descriptor", "descriptor"), KIND_PEG_OUT)

    def test_wallet_kind_aliases_and_silent_payments_use_canonical_routes(self):
        self.assertEqual(
            default_kind_for("BTC", "LBTC", "core-ln", "descriptor"),
            KIND_SUBMARINE_SWAP,
        )
        self.assertEqual(
            default_kind_for("BTC", "LBTC", "silent-payment", "descriptor"),
            KIND_PEG_IN,
        )

    def test_unknown_shape_falls_back_to_manual(self):
        self.assertEqual(default_kind_for("BTC", "BTC", "descriptor", "descriptor"), KIND_MANUAL)


class DefaultPolicyTests(unittest.TestCase):
    def test_at_profile_gets_carrying_value(self):
        self.assertEqual(default_policy_for("at"), POLICY_CARRYING_VALUE)
        self.assertEqual(default_policy_for("AT"), POLICY_CARRYING_VALUE)

    def test_generic_profile_gets_taxable(self):
        self.assertEqual(default_policy_for("generic"), POLICY_TAXABLE)
        self.assertEqual(default_policy_for(None), POLICY_TAXABLE)

    def test_generic_bitcoin_rail_pair_gets_carrying_value(self):
        self.assertEqual(default_policy_for("generic", "BTC", "LBTC"), POLICY_CARRYING_VALUE)
        self.assertEqual(default_policy_for(None, "LBTC", "BTC"), POLICY_CARRYING_VALUE)

    def test_generic_bitcoin_rail_pair_can_default_to_taxable(self):
        self.assertEqual(
            default_policy_for(
                "generic",
                "BTC",
                "LBTC",
                bitcoin_rail_carrying_value=False,
            ),
            POLICY_TAXABLE,
        )


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

    def test_liquid_to_lightning_boltz_submarine_pair_via_payment_hash(self):
        lockup = _row(
            id="boltz-liquid-lockup",
            external_id="liquid-lockup-txid",
            wallet_id="liquid",
            wallet_label="Liquid on-chain",
            wallet_kind="custom",
            payment_hash=_PAY_HASH,
            direction="outbound",
            asset="LBTC",
            occurred_at="2026-03-14T17:30:00Z",
            amount=100_000_000,
        )
        invoice = _row(
            id="boltz-ln-settlement",
            external_id="phoenix-invoice-id",
            wallet_id="phoenix",
            wallet_label="Phoenix",
            wallet_kind="phoenix",
            payment_hash=_PAY_HASH,
            direction="inbound",
            asset="BTC",
            occurred_at="2026-03-14T17:32:00Z",
            amount=99_000_000,
        )
        ordinary_payment = _row(
            id="ordinary-liquid-payment",
            external_id="liquid-payment-txid",
            wallet_id="liquid",
            wallet_label="Liquid on-chain",
            wallet_kind="custom",
            payment_hash=None,
            direction="outbound",
            asset="LBTC",
            occurred_at="2026-03-14T17:31:00Z",
            amount=42_000_000,
        )

        candidates = suggest_swap_candidates(
            [lockup, invoice, ordinary_payment],
            tax_country="at",
        )

        self.assertEqual(len(candidates), 1)
        candidate = candidates[0]
        self.assertEqual(candidate.confidence, CONFIDENCE_EXACT)
        self.assertEqual(candidate.method, METHOD_PAYMENT_HASH)
        self.assertEqual(candidate.out_id, "boltz-liquid-lockup")
        self.assertEqual(candidate.in_id, "boltz-ln-settlement")
        self.assertEqual(candidate.default_kind, KIND_SUBMARINE_SWAP)
        self.assertEqual(candidate.swap_fee_msat, 1_000_000)

    def test_same_wallet_payment_hash_pair_skipped(self):
        out = _row(id="a", wallet_id="w", payment_hash=_PAY_HASH, direction="outbound")
        inbound = _row(id="b", wallet_id="w", payment_hash=_PAY_HASH, direction="inbound")
        self.assertEqual(suggest_swap_candidates([out, inbound]), [])

    def test_own_node_lightning_hash_pair_suppressed_from_review(self):
        # The journal nets a cross-node own payment (CLN pay -> LND invoice,
        # same hash, same asset) as a MOVE, so the matcher must not surface it
        # as an exact payment_hash swap candidate — lockstep with
        # transfers.detect_intra_transfers' Lightning hash pass.
        out = _row(
            id="cln-pay",
            wallet_id="cln-node",
            wallet_label="CLN",
            wallet_kind="cln",
            kind="cln_pay",
            payment_hash=_PAY_HASH,
            direction="outbound",
            amount=100_000_000,
            fee=100_000,
        )
        inbound = _row(
            id="lnd-invoice",
            wallet_id="lnd-node",
            wallet_label="LND",
            wallet_kind="lnd",
            kind="lnd_invoice",
            payment_hash=_PAY_HASH,
            direction="inbound",
            occurred_at="2026-03-14T17:31:00Z",
            amount=100_000_000,
        )
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


class ProviderEvidenceExactMatchTests(unittest.TestCase):
    def test_bull_chain_swap_pairs_by_redacted_swap_id(self):
        raw = {
            "source": "bullbitcoin_wallet_csv",
            "type": "chain_swap",
            "status": "completed",
            "swap_id": "swap-chain",
            "send_network": "bitcoin",
            "receive_network": "liquid",
            "send_txid": "bull-chain-send",
            "receive_txid": "bull-chain-recv",
        }
        out = _row(
            id="btc-out",
            external_id="bull-chain-send",
            wallet_id="bull-btc",
            wallet_kind="bullbitcoin",
            direction="outbound",
            asset="BTC",
            amount=1_000_000_000,
            fee=500_000,
            raw_json=raw,
        )
        inbound = _row(
            id="lbtc-in",
            external_id="bull-chain-recv",
            wallet_id="bull-liquid",
            wallet_kind="bullbitcoin",
            direction="inbound",
            asset="LBTC",
            amount=990_000_000,
            raw_json=raw,
        )

        candidates = suggest_swap_candidates([out, inbound], tax_country="at")

        self.assertEqual(len(candidates), 1)
        candidate = candidates[0]
        self.assertEqual(candidate.confidence, CONFIDENCE_EXACT)
        self.assertEqual(candidate.method, METHOD_PROVIDER_SWAP_ID)
        self.assertEqual(candidate.default_kind, KIND_CHAIN_SWAP)
        self.assertEqual(candidate.default_policy, POLICY_CARRYING_VALUE)
        self.assertEqual(candidate.swap_fee_msat, 10_500_000)
        self.assertEqual(candidate.evidence_provider, "bullbitcoin")
        self.assertEqual(candidate.evidence_id, "swap-chain")
        self.assertEqual(candidate.evidence_status, "completed")

    def test_provider_evidence_route_txids_must_match_when_present(self):
        out = _row(
            id="btc-out",
            external_id="unrelated-send",
            wallet_id="bull-btc",
            wallet_kind="bullbitcoin",
            direction="outbound",
            asset="BTC",
            raw_json={
                "source": "bullbitcoin_wallet_csv",
                "type": "chain_swap",
                "swap_id": "swap-chain",
                "send_txid": "bull-chain-send",
                "receive_txid": "bull-chain-recv",
            },
        )
        inbound = _row(
            id="lbtc-in",
            external_id="bull-chain-recv",
            wallet_id="bull-liquid",
            wallet_kind="bullbitcoin",
            direction="inbound",
            asset="LBTC",
            amount=99_900_000_000,
            raw_json={
                "source": "bullbitcoin_wallet_csv",
                "type": "chain_swap",
                "swap_id": "swap-chain",
                "send_txid": "bull-chain-send",
                "receive_txid": "bull-chain-recv",
            },
        )

        self.assertEqual(suggest_swap_candidates([out, inbound], tax_country="at"), [])

    def test_refunded_provider_status_overrides_chain_swap_flow(self):
        raw = {
            "source": "bullbitcoin_wallet_csv",
            "type": "chain_swap",
            "status": "refunded",
            "swap_id": "swap-refund",
            "send_txid": "bull-refund-lockup",
            "receive_txid": "bull-refund-return",
        }
        out = _row(
            id="btc-out",
            external_id="bull-refund-lockup",
            wallet_id="bull-btc",
            wallet_kind="bullbitcoin",
            direction="outbound",
            asset="BTC",
            amount=1_000_000_000,
            fee=500_000,
            raw_json=raw,
        )
        inbound = _row(
            id="btc-in",
            external_id="bull-refund-return",
            wallet_id="bull-btc",
            wallet_kind="bullbitcoin",
            direction="inbound",
            asset="BTC",
            amount=998_000_000,
            raw_json=raw,
        )

        candidates = suggest_swap_candidates([out, inbound], tax_country="generic")

        self.assertEqual(len(candidates), 1)
        candidate = candidates[0]
        self.assertEqual(candidate.method, METHOD_PROVIDER_SWAP_ID)
        self.assertEqual(candidate.confidence, CONFIDENCE_EXACT)
        self.assertEqual(candidate.default_kind, KIND_SWAP_REFUND)
        self.assertEqual(candidate.default_policy, POLICY_CARRYING_VALUE)
        self.assertEqual(candidate.evidence_provider, "bullbitcoin")
        self.assertEqual(candidate.evidence_id, "swap-refund")

    def test_provider_id_without_source_marker_is_not_exact(self):
        out = _row(
            id="o",
            wallet_id="custom-a",
            wallet_kind="custom",
            direction="outbound",
            asset="BTC",
            amount=100_000_000,
            raw_json={"swap_id": "ambiguous"},
        )
        inbound = _row(
            id="i",
            wallet_id="custom-b",
            wallet_kind="custom",
            direction="inbound",
            asset="LBTC",
            amount=99_900_000,
            raw_json={"swap_id": "ambiguous"},
            occurred_at="2026-03-14T17:32:00Z",
        )

        self.assertEqual(suggest_swap_candidates([out, inbound], tax_country="at"), [])

    def test_free_text_provider_hint_with_generic_id_is_not_exact(self):
        for field in ("counterparty", "service"):
            with self.subTest(field=field):
                raw = {field: "Boltz support", "id": "free-text-id", "flow": "chain"}
                out = _row(
                    id=f"o-{field}",
                    wallet_id="custom-a",
                    wallet_kind="custom",
                    direction="outbound",
                    asset="BTC",
                    amount=100_000_000,
                    raw_json=raw,
                )
                inbound = _row(
                    id=f"i-{field}",
                    wallet_id="custom-b",
                    wallet_kind="custom",
                    direction="inbound",
                    asset="LBTC",
                    amount=99_900_000,
                    raw_json=raw,
                    occurred_at="2026-03-14T17:32:00Z",
                )

                self.assertEqual(
                    suggest_swap_candidates([out, inbound], tax_country="at"),
                    [],
                )

    def test_reverse_submarine_provider_evidence_sets_specific_kind(self):
        raw = {"provider": "boltz", "swap_id": "r1", "flow": "reverse-submarine"}
        out = _row(
            id="ln-out",
            wallet_id="phoenix",
            wallet_kind="phoenix",
            direction="outbound",
            asset="BTC",
            amount=50_000_000,
            raw_json=raw,
        )
        inbound = _row(
            id="lbtc-in",
            wallet_id="liquid",
            wallet_kind="descriptor",
            direction="inbound",
            asset="LBTC",
            amount=49_500_000,
            raw_json=raw,
        )
        candidates = suggest_swap_candidates([out, inbound], tax_country="at")
        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].method, METHOD_PROVIDER_SWAP_ID)
        self.assertEqual(candidates[0].default_kind, KIND_REVERSE_SUBMARINE_SWAP)

    def test_boltz_native_id_field_is_allowed_with_provider_marker(self):
        raw = {
            "provider": "boltz",
            "id": "boltz-chain-id",
            "flow": "chain",
            "version": "2",
            "taproot": True,
            "cooperative": True,
            "spend_path": "key",
        }
        out = _row(
            id="btc-out",
            wallet_id="btc",
            wallet_kind="descriptor",
            direction="outbound",
            asset="BTC",
            amount=100_000_000,
            raw_json=raw,
        )
        inbound = _row(
            id="lbtc-in",
            wallet_id="liquid",
            wallet_kind="descriptor",
            direction="inbound",
            asset="LBTC",
            amount=99_500_000,
            raw_json=raw,
        )
        candidates = suggest_swap_candidates([out, inbound], tax_country="at")
        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].method, METHOD_PROVIDER_SWAP_ID)
        self.assertEqual(candidates[0].default_kind, KIND_CHAIN_SWAP)
        self.assertEqual(candidates[0].evidence_id, "boltz-chain-id")
        self.assertEqual(candidates[0].evidence_version, "2")
        self.assertEqual(candidates[0].evidence_taproot, "True")
        self.assertEqual(candidates[0].evidence_cooperative, "True")
        self.assertEqual(candidates[0].evidence_spend_path, "key")


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

    def test_normal_self_transfer_claimed_as_deterministic(self):
        # delta 0.001 BTC on a 1.001 BTC transfer is well under the
        # max(1%, 2500 sats) ceiling -> a clean self-transfer, claimed.
        out = _row(id="o", external_id="txid", wallet_id="cold",
                   direction="outbound", asset="BTC", amount=100_100_000_000)
        inbound = _row(id="i", external_id="txid", wallet_id="hot",
                       direction="inbound", asset="BTC", amount=100_000_000_000)
        ids = _deterministic_self_transfer_ids([out, inbound])
        self.assertEqual(ids, {"o", "i"})

    def test_mixed_case_txid_still_claimed_as_deterministic(self):
        # Bitcoin txids are case-insensitive hex; two wallets recording the same
        # self-transfer with opposite casing must still group as one proven
        # self-transfer (in lockstep with detect_intra_transfers grouping), so
        # the clean move is not surfaced as a swap candidate.
        txid = "ab" * 32
        out = _row(id="o", external_id=txid.upper(), wallet_id="cold",
                   direction="outbound", asset="BTC", amount=100_100_000_000)
        inbound = _row(id="i", external_id=txid.lower(), wallet_id="hot",
                       direction="inbound", asset="BTC", amount=100_000_000_000)
        ids = _deterministic_self_transfer_ids([out, inbound])
        self.assertEqual(ids, {"o", "i"})

    def test_zero_value_inbound_does_not_surface_self_transfer_as_swap(self):
        # Keep this in lockstep with transfers.detect_intra_transfers: a stray
        # zero-value inbound placeholder sharing the txid must not stop the
        # deterministic self-transfer prefilter from claiming the real 1-out/1-in
        # move. Otherwise the heuristic path re-surfaces an ordinary cold->hot
        # move as a strong carrying-value swap candidate.
        txid = "11" * 32
        out = _row(id="o", external_id=txid, wallet_id="cold",
                   direction="outbound", asset="BTC", amount=100_100_000_000)
        inbound = _row(id="i", external_id=txid, wallet_id="hot",
                       direction="inbound", asset="BTC", amount=100_000_000_000)
        zero_inbound = _row(id="z", external_id=txid, wallet_id="csv",
                            direction="inbound", asset="BTC", amount=0)

        rows = [out, inbound, zero_inbound]
        self.assertEqual(_deterministic_self_transfer_ids(rows), {"o", "i"})
        self.assertEqual(suggest_swap_candidates(rows, tax_country="at"), [])

    def test_fee_inclusive_leg_claimed_as_deterministic(self):
        # A BTCPay outbound folds the miner fee into `amount` (amount_includes_fee),
        # so the out/in gap is the fee, not an implausible residual. It must be
        # claimed as a proven self-transfer (suppressed from swap review), in
        # lockstep with the journal's fee-inclusive transfer guard — even when the
        # gap exceeds the standard max(1%, 2500 sats) ceiling.
        out = _row(id="o", external_id="txid", wallet_id="btcpay",
                   direction="outbound", asset="BTC", amount=103_000_000,
                   amount_includes_fee=1)
        inbound = _row(id="i", external_id="txid", wallet_id="hot",
                       direction="inbound", asset="BTC", amount=100_000_000)
        self.assertEqual(_deterministic_self_transfer_ids([out, inbound]), {"o", "i"})

        # Control: the identical gap on a node-backed (recipient-only) outbound is
        # NOT claimed — it stays eligible for swap review.
        out_node = _row(id="o2", external_id="txid2", wallet_id="cold",
                        direction="outbound", asset="BTC", amount=103_000_000)
        in_node = _row(id="i2", external_id="txid2", wallet_id="hot",
                       direction="inbound", asset="BTC", amount=100_000_000)
        self.assertEqual(_deterministic_self_transfer_ids([out_node, in_node]), set())

    def test_implausible_fee_self_transfer_not_claimed_as_deterministic(self):
        # The id=47 split-peg shape: a ~41x-tolerance implied fee means the
        # outbound fanned out to an unrecognized recipient, so it must NOT be
        # claimed as a proven self-transfer (kept eligible for swap review,
        # in lockstep with the transfer_fee_implausible tax quarantine).
        out = _row(id="o", external_id="txid", wallet_id="cold",
                   direction="outbound", asset="BTC", amount=4_702_253_000)
        inbound = _row(id="i", external_id="txid", wallet_id="hot",
                       direction="inbound", asset="BTC", amount=2_750_000_000)
        ids = _deterministic_self_transfer_ids([out, inbound])
        self.assertEqual(ids, set())

    def test_cross_asset_heuristic_without_recognized_route_skipped(self):
        # LBTC->BTC across two custodial-exchange wallets (neither a chain nor a
        # Lightning kind) is not a recognized peg/submarine route, so the
        # time+amount heuristic must NOT surface it as a strong candidate (would
        # otherwise be weldable into a basis-corrupting carrying-value pair).
        out = _row(id="o", external_id="", wallet_id="w1", wallet_kind="strike",
                   direction="outbound", asset="LBTC", amount=100_000_000)
        inbound = _row(id="i", external_id="", wallet_id="w2", wallet_kind="river",
                       direction="inbound", asset="BTC", amount=99_900_000,
                       occurred_at="2026-03-14T17:31:00Z")
        self.assertEqual(suggest_swap_candidates([out, inbound], tax_country="at"), [])

    def test_custom_wallet_cross_asset_peg_is_not_inferred(self):
        # `custom` is too broad: it can mean custodians, exchanges, CSV-only
        # sources, or self-custody wallets. Do not infer a carrying-value peg
        # from asset shape alone.
        out = _row(id="o", external_id="", wallet_id="w1", wallet_kind="custom",
                   direction="outbound", asset="BTC", amount=100_000_000)
        inbound = _row(id="i", external_id="", wallet_id="w2", wallet_kind="custom",
                       direction="inbound", asset="LBTC", amount=99_900_000,
                       occurred_at="2026-03-14T17:31:00Z")
        self.assertEqual(suggest_swap_candidates([out, inbound], tax_country="at"), [])

    def test_deterministic_self_transfer_ignores_caller_fee_tolerance(self):
        # Caller tolerances widen heuristic generation only. Deterministic
        # suppression must keep the journal's fixed defaults or a loose review
        # flag can hide a pair the journal still quarantines.
        out = _row(id="o", external_id="txid", wallet_id="cold",
                   direction="outbound", asset="BTC", amount=4_702_253_000)
        inbound = _row(id="i", external_id="txid", wallet_id="hot",
                       direction="inbound", asset="BTC", amount=2_750_000_000)
        self.assertEqual(_deterministic_self_transfer_ids([out, inbound]), set())
        self.assertEqual(
            _deterministic_self_transfer_ids([out, inbound], fee_pct_max=0.5),
            set(),
        )

    def test_conserving_same_txid_fanout_is_suppressed_as_one_group(self):
        out = _row(
            id="o", external_id="fanout", wallet_id="cold",
            direction="outbound", asset="BTC", amount=100_000_000,
        )
        large_in = _row(
            id="i-large", external_id="fanout", wallet_id="hot",
            direction="inbound", asset="BTC", amount=99_500_000,
        )
        small_in = _row(
            id="i-small", external_id="fanout", wallet_id="savings",
            direction="inbound", asset="BTC", amount=500_000,
        )
        rows = [out, large_in, small_in]
        self.assertEqual(
            _deterministic_self_transfer_ids(rows),
            {"o", "i-large", "i-small"},
        )
        self.assertEqual(suggest_swap_candidates(rows), [])

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

    def test_zero_amount_inbound_rejected(self):
        # A zero-amount inbound row sits within the absolute fee floor of any
        # small outbound; it must never become a heuristic candidate.
        out = _row(id="o", wallet_id="A", direction="outbound", amount=2_000_000, asset="LBTC")  # 2000 sats
        inbound = _row(id="i", wallet_id="B", direction="inbound", amount=0, asset="BTC")
        self.assertEqual(suggest_swap_candidates([out, inbound], tax_country="at"), [])

    def test_negative_amount_inbound_rejected(self):
        out = _row(id="o", wallet_id="A", direction="outbound", amount=2_000_000, asset="LBTC")
        inbound = _row(id="i", wallet_id="B", direction="inbound", amount=-1_000_000, asset="BTC")
        self.assertEqual(suggest_swap_candidates([out, inbound], tax_country="at"), [])

    def test_window_slice_matches_naive_all_pairs_reference(self):
        # Outbounds and inbounds scattered across weeks, most far outside
        # the 24h window of any outbound. Pins the time-sorted bisect slice
        # against a naive all-pairs evaluation of the same predicates so the
        # windowing can never drift from the documented contract, including
        # the inclusive boundary at exactly ±window.
        window = 24 * 3600
        base = datetime(2026, 3, 1, 12, 0, 0, tzinfo=timezone.utc)

        def at(offset_seconds):
            return (base + timedelta(seconds=offset_seconds)).isoformat().replace("+00:00", "Z")

        outs = [
            _row(id="o-day0", wallet_id="A", direction="outbound", asset="LBTC",
                 amount=100_000_000_000, occurred_at=at(0)),
            _row(id="o-day10", wallet_id="A", direction="outbound", asset="LBTC",
                 amount=50_000_000_000, occurred_at=at(10 * 86_400)),
            _row(id="o-day30", wallet_id="B", direction="outbound", asset="LBTC",
                 amount=20_000_000_000, occurred_at=at(30 * 86_400)),
        ]
        ins = [
            # Near o-day0: inside window and tolerance.
            _row(id="i-near0", wallet_id="B", direction="inbound", asset="BTC",
                 amount=99_900_000_000, occurred_at=at(2 * 3600)),
            # Exactly at the +window boundary of o-day0: inclusive, matches.
            _row(id="i-edge0", wallet_id="C", direction="inbound", asset="BTC",
                 amount=99_950_000_000, occurred_at=at(window)),
            # One second past the boundary: rejected.
            _row(id="i-past0", wallet_id="C", direction="inbound", asset="BTC",
                 amount=99_950_000_000, occurred_at=at(window + 1)),
            # Near o-day10, inside window; amount fits only o-day10.
            _row(id="i-near10", wallet_id="C", direction="inbound", asset="BTC",
                 amount=49_900_000_000, occurred_at=at(10 * 86_400 - 3600)),
            # Far outside every outbound's window despite a matching amount.
            _row(id="i-far", wallet_id="C", direction="inbound", asset="BTC",
                 amount=99_900_000_000, occurred_at=at(60 * 86_400)),
            # Inside o-day30's window but the amount delta exceeds tolerance.
            _row(id="i-wrong-size30", wallet_id="C", direction="inbound", asset="BTC",
                 amount=10_000_000_000, occurred_at=at(30 * 86_400 + 3600)),
        ]

        candidates = suggest_swap_candidates(
            [*outs, *ins], time_window_seconds=window, tax_country="at"
        )
        matched = {(c.out_id, c.in_id) for c in candidates}

        expected = set()
        for out in outs:
            out_seconds = datetime.fromisoformat(out["occurred_at"].replace("Z", "+00:00")).timestamp()
            threshold = fee_threshold_msat(out["amount"], 0.01, 2500)
            for inbound in ins:
                in_seconds = datetime.fromisoformat(inbound["occurred_at"].replace("Z", "+00:00")).timestamp()
                if out["wallet_id"] == inbound["wallet_id"]:
                    continue
                if abs(in_seconds - out_seconds) > window:
                    continue
                delta = out["amount"] - inbound["amount"]
                if delta < 0 or delta > threshold:
                    continue
                expected.add((out["id"], inbound["id"]))

        self.assertEqual(matched, expected)
        self.assertIn(("o-day0", "i-near0"), matched)
        self.assertIn(("o-day0", "i-edge0"), matched)
        self.assertIn(("o-day10", "i-near10"), matched)
        self.assertNotIn(("o-day0", "i-past0"), matched)
        self.assertTrue(all(in_id != "i-far" for _, in_id in matched))
        self.assertTrue(all(in_id != "i-wrong-size30" for _, in_id in matched))


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
        self.assertEqual([c.conflict_size for c in candidates], [2, 2])

    def test_solo_candidate_gets_conflict_size_one(self):
        out = _row(id="o", wallet_id="A", asset="LBTC", direction="outbound", amount=100_000_000)
        inbound = _row(id="i", wallet_id="B", asset="BTC", direction="inbound",
                       amount=99_500_000, occurred_at="2026-03-14T17:32:00Z")
        candidates = suggest_swap_candidates([out, inbound], tax_country="at")
        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].conflict_size, 1)

    def test_cross_type_conflict_keeps_size_across_interpretations(self):
        # One outbound BTC leg that matches both a same-asset inbound
        # (transfer interpretation) and a cross-asset inbound (swap
        # interpretation). Both candidates carry conflict_size=2 so a
        # filtered swap-only or transfer-only view cannot make either
        # look solo.
        out = _row(id="o", wallet_id="A", asset="BTC", direction="outbound", amount=100_000_000_000)
        transfer_in = _row(id="i-btc", wallet_id="B", asset="BTC", direction="inbound",
                           amount=99_900_000_000, occurred_at="2026-03-14T17:40:00Z")
        swap_in = _row(id="i-lbtc", wallet_id="C", asset="LBTC", direction="inbound",
                       amount=99_800_000_000, occurred_at="2026-03-14T17:45:00Z")
        candidates = suggest_swap_candidates([out, transfer_in, swap_in], tax_country="at")
        self.assertEqual(len(candidates), 2)
        self.assertEqual({c.in_id for c in candidates}, {"i-btc", "i-lbtc"})
        self.assertEqual([c.conflict_size for c in candidates], [2, 2])

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


class LightningPaymentHashSuppressionTests(unittest.TestCase):
    def test_node_sourced_payment_hash_suppressed_even_with_large_route_fee(self):
        out = _row(
            id="ln-out",
            wallet_id="node-a",
            wallet_kind="lnd",
            kind="lnd_payment",
            payment_hash=_PAY_HASH,
            payment_hash_source="lnd",
            direction="outbound",
            amount=100_000_000,
        )
        inbound = _row(
            id="ln-in",
            wallet_id="node-b",
            wallet_kind="coreln",
            kind="cln_invoice",
            payment_hash=_PAY_HASH,
            payment_hash_source="core_lightning",
            direction="inbound",
            amount=50_000_000,
        )

        self.assertEqual(suggest_swap_candidates([out, inbound], tax_country="at"), [])


class RefundLinkMatchingTests(unittest.TestCase):
    def test_same_wallet_refund_paired_by_funding_link(self):
        lockup = _row(
            id="lockup",
            wallet_id="wallet-a",
            external_id="lockup-txid",
            direction="outbound",
            amount=10_000_000,
            occurred_at="2026-03-01T09:00:00Z",
        )
        refund = _row(
            id="refund",
            wallet_id="wallet-a",  # refund returns to the funding wallet
            external_id="refund-txid",
            swap_refund_funding_txid="lockup-txid",
            direction="inbound",
            amount=9_950_000,
            occurred_at="2026-03-05T09:00:00Z",  # well past the 24h window
        )
        candidates = suggest_swap_candidates([lockup, refund])
        self.assertEqual(len(candidates), 1)
        candidate = candidates[0]
        self.assertEqual(candidate.out_id, "lockup")
        self.assertEqual(candidate.in_id, "refund")
        self.assertEqual(candidate.confidence, CONFIDENCE_EXACT)
        self.assertEqual(candidate.method, METHOD_HTLC_REFUND)
        self.assertEqual(candidate.default_kind, KIND_SWAP_REFUND)
        self.assertEqual(candidate.default_policy, POLICY_CARRYING_VALUE)
        self.assertEqual(candidate.swap_fee_msat, 50_000)

    def test_refund_link_with_no_matching_funding_leg_is_unmatched(self):
        # Same-wallet refund whose funding leg isn't present (and same-wallet, so
        # the heuristic can't rescue it either) stays unmatched.
        refund = _row(
            id="refund",
            wallet_id="wallet-a",
            external_id="refund-txid",
            swap_refund_funding_txid="missing-lockup",
            direction="inbound",
            amount=9_950_000,
        )
        self.assertEqual(suggest_swap_candidates([refund]), [])

    def test_refund_link_beats_heuristic_for_cross_wallet_refund(self):
        # A different-wallet refund inside the window would also match the
        # heuristic; the deterministic link must win and emit exactly one
        # exact candidate, not a duplicate strong one.
        lockup = _row(
            id="lockup",
            wallet_id="wallet-a",
            external_id="lockup-txid",
            direction="outbound",
            amount=10_000_000,
            occurred_at="2026-03-01T09:00:00Z",
        )
        refund = _row(
            id="refund",
            wallet_id="wallet-b",
            external_id="refund-txid",
            swap_refund_funding_txid="lockup-txid",
            direction="inbound",
            amount=9_950_000,
            occurred_at="2026-03-01T15:00:00Z",
        )
        candidates = suggest_swap_candidates([lockup, refund])
        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].method, METHOD_HTLC_REFUND)
        self.assertEqual(candidates[0].confidence, CONFIDENCE_EXACT)

    def test_refund_link_matches_case_insensitively(self):
        # txids are hex; sync lowercases the funding link but external_id is
        # stored verbatim, so the join must not be case-sensitive.
        mixed_txid = "AABB" + "cc" * 30  # 64 chars, mixed case
        lockup = _row(
            id="lockup",
            wallet_id="wallet-a",
            external_id=mixed_txid,
            direction="outbound",
            amount=10_000_000,
        )
        refund = _row(
            id="refund",
            wallet_id="wallet-a",
            external_id="refund-txid",
            swap_refund_funding_txid=mixed_txid.lower(),
            direction="inbound",
            amount=9_950_000,
        )
        candidates = suggest_swap_candidates([lockup, refund])
        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].method, METHOD_HTLC_REFUND)

    def test_cross_asset_funding_link_not_paired(self):
        # The same-asset guard: a refund returns the asset that was locked, so a
        # link that points at a different-asset outbound is not a swap refund.
        lockup = _row(
            id="lockup",
            wallet_id="wallet-a",
            external_id="lockup-txid",
            direction="outbound",
            asset="LBTC",
            amount=10_000_000,
        )
        refund = _row(
            id="refund",
            wallet_id="wallet-a",
            external_id="refund-txid",
            swap_refund_funding_txid="lockup-txid",
            direction="inbound",
            asset="BTC",
            amount=9_950_000,
        )
        self.assertEqual(suggest_swap_candidates([lockup, refund]), [])


if __name__ == "__main__":
    unittest.main()
