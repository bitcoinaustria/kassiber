"""Pure-function tests for the swap-candidate matcher.

Each test feeds the matcher synthetic dict rows so we pin the
public contract (which fields it reads, what shape the candidates
take) independently of SQLite. The matcher has no I/O — these
exercise the full algorithm end-to-end.
"""

import json
import inspect
import hashlib
import unittest
from dataclasses import replace
from datetime import datetime, timedelta, timezone

from kassiber.core.transfer_matching import (
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
    METHOD_OWNERSHIP_GRAPH,
    POLICY_CARRYING_VALUE,
    POLICY_TAXABLE,
    compute_swap_fee,
    compute_swap_fee_components,
    default_kind_for,
    default_ownership_policy_for,
    fee_threshold_msat,
    finalize_candidate_conflicts,
    suggest_swap_candidates,
)
from kassiber.tax_policy import recommended_pair_policy


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
    raw = base.get("raw_json")
    if isinstance(raw, str):
        try:
            raw_payload = json.loads(raw)
        except (TypeError, ValueError):
            raw_payload = {}
    elif isinstance(raw, dict):
        raw_payload = dict(raw)
    else:
        raw_payload = {}
    external_id = str(base.get("external_id") or "")
    if (
        not raw_payload
        and len(external_id) == 64
        and all(char in "0123456789abcdefABCDEF" for char in external_id)
    ):
        raw_payload["txid"] = external_id
    source = str(base.get("payment_hash_source") or "").lower()
    wallet_kind = str(base.get("wallet_kind") or "").lower()
    direction = str(base.get("direction") or "")
    if source == "lnd" and wallet_kind == "lnd":
        base["kind"] = base.get("kind") or (
            "lnd_pay" if direction == "outbound" else "lnd_invoice"
        )
        raw_payload["_kassiber_provenance"] = {"import_source": "lnd"}
        raw_payload.update({"chain": "lightning", "network": "main"})
    elif source == "core_lightning" and wallet_kind in {"cln", "coreln"}:
        base["kind"] = base.get("kind") or (
            "cln_pay" if direction == "outbound" else "cln_invoice"
        )
        raw_payload["_kassiber_provenance"] = {
            "import_source": "core-lightning"
        }
        raw_payload.update({"chain": "lightning", "network": "main"})
    base["raw_json"] = raw_payload
    return base


_PAY_HASH = "ab" * 32
_TXID_A = "11" * 32
_TXID_B = "22" * 32
_TXID_C = "33" * 32
_LIQUID_ASSET_ID = "6f" * 32
_CLAIM_PREIMAGE = bytes.fromhex("42" * 32)
_CLAIM_HASH = hashlib.sha256(_CLAIM_PREIMAGE).hexdigest()


def _claim_raw(txid, *, liquid=False):
    sha = hashlib.sha256(_CLAIM_PREIMAGE).digest()
    hash160 = hashlib.new("ripemd160", sha).digest()
    redeem_script = (
        bytes([0xA9, 0x14])
        + hash160
        + bytes([0x87, 0x63, 0x21])
        + bytes.fromhex("02" + "11" * 32)
        + bytes([0x67, 0x03, 0x00, 0x00, 0x10, 0xB1, 0x75, 0x21])
        + bytes.fromhex("03" + "22" * 32)
        + bytes([0x68, 0xAC])
    )
    payload = {
        "txid": txid,
        "chain": "liquid" if liquid else "bitcoin",
        "network": "liquidv1" if liquid else "main",
        "vin": [
            {
                "txid": "44" * 32,
                "vout": 0,
                "witness": ["3045", _CLAIM_PREIMAGE.hex(), "01", redeem_script.hex()],
            }
        ],
        "vout": [],
    }
    if liquid:
        payload["component"] = {"asset_id": _LIQUID_ASSET_ID, "asset": "LBTC"}
    return payload


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

    def test_structured_components_do_not_call_bridge_delta_provider_fee(self):
        components = compute_swap_fee_components(
            100,
            80,
            3,
            source_fee_kind="lightning_routing",
        )
        self.assertEqual(components.source_fee_msat, 3)
        self.assertEqual(components.source_fee_kind, "lightning_routing")
        self.assertEqual(components.bridge_delta_msat, 20)
        self.assertEqual(
            components.bridge_delta_kind, "unallocated_bridge_delta"
        )
        self.assertEqual(components.total_msat, 23)


class DefaultKindTests(unittest.TestCase):
    def test_lightning_to_chain_is_reverse_submarine_swap(self):
        self.assertEqual(default_kind_for("BTC", "BTC", "phoenix", "descriptor"), KIND_REVERSE_SUBMARINE_SWAP)
        self.assertEqual(default_kind_for("BTC", "LBTC", "phoenix", "descriptor"), KIND_REVERSE_SUBMARINE_SWAP)

    def test_chain_to_lightning_is_submarine_swap(self):
        self.assertEqual(default_kind_for("BTC", "BTC", "descriptor", "lnd"), KIND_SUBMARINE_SWAP)
        self.assertEqual(default_kind_for("LBTC", "BTC", "descriptor", "phoenix"), KIND_SUBMARINE_SWAP)

    def test_chain_to_chain_btc_to_lbtc_is_peg_in(self):
        self.assertEqual(default_kind_for("BTC", "LBTC", "descriptor", "descriptor"), KIND_PEG_IN)

    def test_chain_to_chain_lbtc_to_btc_is_peg_out(self):
        self.assertEqual(default_kind_for("LBTC", "BTC", "descriptor", "descriptor"), KIND_PEG_OUT)

    def test_wallet_kind_aliases_and_silent_payments_use_canonical_routes(self):
        self.assertEqual(
            default_kind_for("BTC", "LBTC", "core-ln", "descriptor"),
            KIND_REVERSE_SUBMARINE_SWAP,
        )
        self.assertEqual(
            default_kind_for("BTC", "LBTC", "core_lightning", "descriptor"),
            KIND_REVERSE_SUBMARINE_SWAP,
        )
        self.assertEqual(
            default_kind_for("BTC", "LBTC", "silent-payment", "descriptor"),
            KIND_PEG_IN,
        )

    def test_unknown_shape_falls_back_to_manual(self):
        self.assertEqual(default_kind_for("BTC", "BTC", "descriptor", "descriptor"), KIND_MANUAL)


class DefaultPolicyTests(unittest.TestCase):
    def test_matcher_api_has_no_profile_or_country_policy_input(self):
        self.assertNotIn(
            "tax_country", inspect.signature(suggest_swap_candidates).parameters
        )
        self.assertNotIn(
            "bitcoin_rail_carrying_value",
            inspect.signature(suggest_swap_candidates).parameters,
        )

    def test_non_rail_pair_gets_taxable_without_country_input(self):
        self.assertEqual(
            default_ownership_policy_for("BTC", "USDT"), POLICY_TAXABLE
        )

    def test_bitcoin_rail_pair_gets_carrying_value(self):
        self.assertEqual(
            default_ownership_policy_for("BTC", "LBTC"), POLICY_CARRYING_VALUE
        )
        self.assertEqual(
            default_ownership_policy_for("LBTC", "BTC"), POLICY_CARRYING_VALUE
        )

    def test_profile_can_override_bitcoin_rail_policy_only_after_matching(self):
        self.assertEqual(
            recommended_pair_policy(
                {"tax_country": "generic", "bitcoin_rail_carrying_value": 0},
                "BTC",
                "LBTC",
            ),
            POLICY_TAXABLE,
        )

    def test_country_policy_is_applied_only_after_matching(self):
        out = _row(
            id="country-neutral-out",
            wallet_id="A",
            wallet_kind="lnd",
            payment_hash=_PAY_HASH,
            payment_hash_source="lnd",
            direction="outbound",
            asset="BTC",
        )
        inbound = _row(
            id="country-neutral-in",
            wallet_id="B",
            wallet_kind="custom",
            payment_hash=_PAY_HASH,
            payment_hash_source="chain_script_unique_outpoint",
            direction="inbound",
            asset="USDT",
        )

        candidates = suggest_swap_candidates([out, inbound])

        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].default_policy, POLICY_TAXABLE)
        self.assertEqual(
            recommended_pair_policy(
                {"tax_country": "generic", "bitcoin_rail_carrying_value": 1},
                "BTC",
                "USDT",
            ),
            POLICY_TAXABLE,
        )
        self.assertEqual(
            recommended_pair_policy(
                {"tax_country": "at", "bitcoin_rail_carrying_value": 1},
                "BTC",
                "USDT",
            ),
            POLICY_CARRYING_VALUE,
        )


class PaymentHashExactMatchTests(unittest.TestCase):
    def test_lightning_to_chain_pair_via_payment_hash(self):
        out = _row(
            id="lnsend",
            wallet_id="lnd",
            wallet_label="LND",
            wallet_kind="lnd",
            kind="lnd_pay",
            payment_hash=_CLAIM_HASH,
            payment_hash_source="lnd",
            direction="outbound",
            occurred_at="2026-03-14T17:30:00Z",
            amount=100_000_000,
        )
        receive = _row(
            id="liquidrecv",
            external_id=_TXID_C,
            wallet_id="liquid",
            wallet_label="Liquid Slip77",
            wallet_kind="descriptor",
            payment_hash=_CLAIM_HASH,
            payment_hash_source="chain_script_unique_outpoint",
            direction="inbound",
            asset="LBTC",
            occurred_at="2026-03-14T17:32:00Z",
            amount=99_500_000,
            raw_json=_claim_raw(_TXID_C, liquid=True),
        )
        candidates = suggest_swap_candidates([out, receive])
        self.assertEqual(len(candidates), 1)
        candidate = candidates[0]
        self.assertEqual(candidate.confidence, CONFIDENCE_EXACT)
        self.assertEqual(candidate.method, METHOD_PAYMENT_HASH)
        self.assertEqual(candidate.out_id, "lnsend")
        self.assertEqual(candidate.in_id, "liquidrecv")
        self.assertEqual(candidate.default_kind, KIND_REVERSE_SUBMARINE_SWAP)
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
            payment_hash_source="boltz-regtest",
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
            payment_hash_source="importer",
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
        )

        self.assertEqual(len(candidates), 1)
        candidate = candidates[0]
        self.assertEqual(candidate.confidence, CONFIDENCE_STRONG)
        self.assertEqual(candidate.method, METHOD_PAYMENT_HASH)
        self.assertEqual(candidate.out_id, "boltz-liquid-lockup")
        self.assertEqual(candidate.in_id, "boltz-ln-settlement")
        self.assertEqual(candidate.default_kind, KIND_SUBMARINE_SWAP)
        self.assertEqual(candidate.swap_fee_msat, 1_000_000)

    def test_equal_unproven_hash_strings_are_not_exact_evidence(self):
        out = _row(
            id="out",
            wallet_id="node",
            wallet_kind="lnd",
            payment_hash=_PAY_HASH,
            direction="outbound",
            amount=100_000_000,
        )
        inbound = _row(
            id="in",
            wallet_id="chain",
            wallet_kind="descriptor",
            payment_hash=_PAY_HASH,
            direction="inbound",
            amount=99_000_000,
            # No chain_script/provider provenance.
        )
        candidates = suggest_swap_candidates([out, inbound])
        self.assertFalse(
            [candidate for candidate in candidates if candidate.confidence == CONFIDENCE_EXACT]
        )

    def test_unknown_hash_source_cannot_promote_exact(self):
        out = _row(
            id="out",
            wallet_id="import",
            wallet_kind="custom",
            payment_hash=_PAY_HASH,
            payment_hash_source="totally_user_controlled_source",
            direction="outbound",
            amount=100_000_000,
        )
        inbound = _row(
            id="in",
            wallet_id="chain",
            wallet_kind="descriptor",
            payment_hash=_PAY_HASH,
            payment_hash_source="chain_script_unique_outpoint",
            direction="inbound",
            amount=99_500_000,
        )

        candidates = suggest_swap_candidates([out, inbound])

        self.assertTrue(candidates)
        self.assertFalse(
            [candidate for candidate in candidates if candidate.method == METHOD_PAYMENT_HASH]
        )
        self.assertTrue(
            all(candidate.confidence == CONFIDENCE_STRONG for candidate in candidates)
        )

    def test_legacy_batched_claim_hash_stays_strong_manual_evidence(self):
        out = _row(
            id="node-out",
            wallet_id="node",
            wallet_kind="lnd",
            payment_hash=_PAY_HASH,
            payment_hash_source="lnd",
            direction="outbound",
            amount=100_000_000,
        )
        inbound = _row(
            id="batched-claim",
            wallet_id="chain",
            wallet_kind="descriptor",
            payment_hash=_PAY_HASH,
            # Historical sync used this unversioned source after choosing the
            # first claim witness from a potentially batched transaction.
            payment_hash_source="chain_script",
            direction="inbound",
            asset="LBTC",
            amount=99_500_000,
            occurred_at="2026-03-14T17:32:00Z",
            raw_json={"vin": [{"witness": ["claim-a"]}, {"witness": ["claim-b"]}]},
        )

        candidates = suggest_swap_candidates([out, inbound])

        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].confidence, CONFIDENCE_STRONG)
        self.assertEqual(candidates[0].method, METHOD_HEURISTIC)

    def test_duplicate_hash_legs_do_not_form_cartesian_exact_candidates(self):
        out_one = _row(
            id="out-1", wallet_id="node", wallet_kind="lnd",
            payment_hash=_PAY_HASH, payment_hash_source="lnd",
            direction="outbound", amount=100_000_000,
        )
        out_two = _row(
            id="out-2", wallet_id="node", wallet_kind="lnd",
            payment_hash=_PAY_HASH, payment_hash_source="lnd",
            direction="outbound", amount=100_000_000,
        )
        inbound = _row(
            id="in", wallet_id="chain", wallet_kind="descriptor",
            payment_hash=_PAY_HASH,
            payment_hash_source="chain_script_unique_outpoint",
            direction="inbound", amount=99_000_000,
        )
        candidates = suggest_swap_candidates([out_one, out_two, inbound])
        self.assertFalse(
            [candidate for candidate in candidates if candidate.confidence == CONFIDENCE_EXACT]
        )

    def test_hash_match_with_nonconserving_amount_is_not_exact(self):
        out = _row(
            id="out", wallet_id="node", wallet_kind="lnd",
            payment_hash=_PAY_HASH, payment_hash_source="lnd",
            direction="outbound", amount=100_000_000,
        )
        inbound = _row(
            id="in", wallet_id="chain", wallet_kind="descriptor",
            payment_hash=_PAY_HASH,
            payment_hash_source="chain_script_unique_outpoint",
            direction="inbound", amount=101_000_000,
        )
        self.assertEqual(suggest_swap_candidates([out, inbound]), [])

    def test_same_wallet_payment_hash_pair_skipped(self):
        out = _row(id="a", wallet_id="w", payment_hash=_PAY_HASH, direction="outbound")
        inbound = _row(id="b", wallet_id="w", payment_hash=_PAY_HASH, direction="inbound")
        self.assertEqual(suggest_swap_candidates([out, inbound]), [])

    def test_own_node_lightning_hash_pair_suppressed_from_review(self):
        # The stored journal projection has already booked the cross-node own
        # payment as a MOVE, so its anchors cannot reappear in swap review.
        out = _row(
            id="cln-pay",
            wallet_id="cln-node",
            wallet_label="CLN",
            wallet_kind="cln",
            kind="cln_pay",
            payment_hash=_PAY_HASH,
            payment_hash_source="core_lightning",
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
            payment_hash_source="lnd",
            direction="inbound",
            occurred_at="2026-03-14T17:31:00Z",
            amount=100_000_000,
        )
        self.assertEqual(
            suggest_swap_candidates(
                [out, inbound],
                booked_move_transaction_ids={"cln-pay", "lnd-invoice"},
            ),
            [],
        )

    def test_same_node_circular_payment_is_also_suppressed(self):
        rows = [
            _row(
                id="circular-out",
                wallet_id="lnd-node",
                wallet_kind="lnd",
                kind="lnd_pay",
                payment_hash=_PAY_HASH,
                payment_hash_source="lnd",
                direction="outbound",
                amount=100_000_000,
            ),
            _row(
                id="circular-in",
                wallet_id="lnd-node",
                wallet_kind="lnd",
                kind="lnd_invoice",
                payment_hash=_PAY_HASH,
                payment_hash_source="lnd",
                direction="inbound",
                amount=100_000_000,
            ),
        ]

        self.assertEqual(
            suggest_swap_candidates(
                rows,
                booked_move_transaction_ids={"circular-out", "circular-in"},
            ),
            [],
        )

    def test_native_hash_identity_never_crosses_networks(self):
        rows = [
            _row(
                id="main-out",
                wallet_id="lnd-main",
                wallet_kind="lnd",
                payment_hash=_PAY_HASH,
                payment_hash_source="lnd",
                direction="outbound",
                amount=100_000_000,
                config_json=json.dumps({"network": "main"}),
            ),
            _row(
                id="regtest-in",
                wallet_id="cln-regtest",
                wallet_kind="coreln",
                payment_hash=_PAY_HASH,
                payment_hash_source="core_lightning",
                direction="inbound",
                amount=100_000_000,
                config_json=json.dumps({"network": "regtest"}),
            ),
        ]

        self.assertFalse(
            [
                candidate
                for candidate in suggest_swap_candidates(rows)
                if candidate.method == METHOD_PAYMENT_HASH
            ]
        )

    def test_hash_cardinality_is_scoped_per_bitcoin_network(self):
        rows = []
        for network, suffix in (("main", "main"), ("regtest", "reg")):
            rows.extend(
                [
                    _row(
                        id=f"{suffix}-out",
                        wallet_id=f"{suffix}-source",
                        payment_hash=_PAY_HASH,
                        payment_hash_source="boltz",
                        direction="outbound",
                        amount=100_000_000,
                        raw_json={"network": network},
                    ),
                    _row(
                        id=f"{suffix}-in",
                        wallet_id=f"{suffix}-destination",
                        payment_hash=_PAY_HASH,
                        payment_hash_source="boltz",
                        direction="inbound",
                        amount=99_500_000,
                        raw_json={"network": network},
                    ),
                ]
            )

        candidates = [
            candidate
            for candidate in suggest_swap_candidates(rows)
            if candidate.method == METHOD_PAYMENT_HASH
        ]

        self.assertEqual(
            {(candidate.out_id, candidate.in_id) for candidate in candidates},
            {("main-out", "main-in"), ("reg-out", "reg-in")},
        )
        self.assertTrue(
            all(candidate.confidence == CONFIDENCE_STRONG for candidate in candidates)
        )

    def test_imported_native_source_label_cannot_forge_provenance(self):
        rows = [
            _row(
                id="forged-out",
                wallet_id="lnd",
                wallet_kind="lnd",
                payment_hash=_PAY_HASH,
                payment_hash_source="lnd",
                direction="outbound",
                amount=100_000_000,
            ),
            _row(
                id="forged-in",
                wallet_id="cln",
                wallet_kind="coreln",
                payment_hash=_PAY_HASH,
                payment_hash_source="core_lightning",
                direction="inbound",
                amount=100_000_000,
            ),
        ]
        for row in rows:
            row["raw_json"]["_kassiber_provenance"] = {
                "import_source": "generic-ledger"
            }

        self.assertFalse(
            [
                candidate
                for candidate in suggest_swap_candidates(rows)
                if candidate.method == METHOD_PAYMENT_HASH
            ]
        )

    def test_verified_chain_hash_cannot_bridge_mainnet_to_regtest(self):
        claim_raw = _claim_raw(_TXID_C)
        claim_raw["network"] = "regtest"
        out = _row(
            id="main-node",
            wallet_id="lnd",
            wallet_kind="lnd",
            payment_hash=_CLAIM_HASH,
            payment_hash_source="lnd",
            direction="outbound",
            amount=100_000_000,
            config_json=json.dumps({"network": "main"}),
        )
        inbound = _row(
            id="regtest-claim",
            external_id=_TXID_C,
            wallet_id="chain",
            wallet_kind="descriptor",
            payment_hash=_CLAIM_HASH,
            payment_hash_source="chain_script_unique_outpoint",
            direction="inbound",
            amount=99_500_000,
            raw_json=claim_raw,
        )

        self.assertFalse(
            [
                candidate
                for candidate in suggest_swap_candidates([out, inbound])
                if candidate.method == METHOD_PAYMENT_HASH
            ]
        )

    def test_malformed_node_hash_does_not_suppress_review_rows(self):
        rows = [
            _row(
                id="out",
                wallet_id="lnd-a",
                wallet_kind="lnd",
                kind="lnd_pay",
                payment_hash="not-a-hash",
                payment_hash_source="lnd",
                direction="outbound",
                amount=100_000_000,
            ),
            _row(
                id="in",
                wallet_id="lnd-b",
                wallet_kind="lnd",
                kind="lnd_invoice",
                payment_hash="not-a-hash",
                payment_hash_source="lnd",
                direction="inbound",
                amount=100_000_000,
            ),
        ]

        self.assertEqual(len(suggest_swap_candidates(rows)), 1)

    def test_same_asset_onchain_rows_require_ownership_evidence(self):
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
        self.assertEqual(suggest_swap_candidates([out, inbound]), [])


class ProviderEvidenceExactMatchTests(unittest.TestCase):
    def test_bull_chain_swap_pairs_by_redacted_swap_id(self):
        raw = {
            "source": "bullbitcoin_wallet_csv",
            "type": "chain_swap",
            "status": "completed",
            "swap_id": "swap-chain",
            "send_network": "bitcoin",
            "receive_network": "liquid",
            "send_txid": _TXID_A,
            "receive_txid": _TXID_B,
            "send_amount_msat": 1_000_000_000,
            "receive_amount_msat": 990_000_000,
        }
        out = _row(
            id="btc-out",
            external_id=_TXID_A,
            wallet_id="bull-btc",
            wallet_kind="bullbitcoin",
            direction="outbound",
            asset="BTC",
            amount=1_000_000_000,
            fee=500_000,
            raw_json={**raw, "chain": "bitcoin", "network": "main"},
        )
        inbound = _row(
            id="lbtc-in",
            external_id=_TXID_B,
            wallet_id="bull-liquid",
            wallet_kind="bullbitcoin",
            direction="inbound",
            asset="LBTC",
            amount=990_000_000,
            raw_json={
                **raw,
                "chain": "liquid",
                "network": "liquidv1",
                "component": {
                    "asset_id": _LIQUID_ASSET_ID,
                    "asset": "LBTC",
                },
            },
        )

        candidates = suggest_swap_candidates([out, inbound])

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
            external_id=_TXID_C,
            wallet_id="bull-btc",
            wallet_kind="bullbitcoin",
            direction="outbound",
            asset="BTC",
            raw_json={
                "source": "bullbitcoin_wallet_csv",
                "type": "chain_swap",
                "swap_id": "swap-chain",
                "send_txid": _TXID_A,
                "receive_txid": _TXID_B,
            },
        )
        inbound = _row(
            id="lbtc-in",
            external_id=_TXID_B,
            wallet_id="bull-liquid",
            wallet_kind="bullbitcoin",
            direction="inbound",
            asset="LBTC",
            amount=99_900_000_000,
            raw_json={
                "source": "bullbitcoin_wallet_csv",
                "type": "chain_swap",
                "swap_id": "swap-chain",
                "send_txid": _TXID_A,
                "receive_txid": _TXID_B,
            },
        )

        self.assertEqual(suggest_swap_candidates([out, inbound]), [])

    def test_route_txids_without_explicit_row_amounts_stay_strong(self):
        raw = {
            "provider": "boltz",
            "swap_id": "route-only",
            "flow": "chain",
            "send_txid": _TXID_A,
            "receive_txid": _TXID_B,
        }
        out = _row(
            id="out", external_id=_TXID_A, direction="outbound",
            wallet_id="btc", asset="BTC", amount=100_000_000, raw_json=raw,
        )
        inbound = _row(
            id="in", external_id=_TXID_B, direction="inbound",
            wallet_id="liquid", asset="LBTC", amount=99_500_000, raw_json=raw,
        )

        candidate = suggest_swap_candidates([out, inbound])[0]

        self.assertEqual(candidate.method, METHOD_PROVIDER_SWAP_ID)
        self.assertEqual(candidate.confidence, CONFIDENCE_STRONG)

    def test_contradictory_provider_routes_never_become_exact(self):
        out = _row(
            id="out",
            external_id=_TXID_A,
            wallet_id="btc",
            direction="outbound",
            amount=100_000_000,
            raw_json={
                "provider": "boltz",
                "swap_id": "route-conflict",
                "flow": "chain",
                "send_txid": _TXID_A,
                "receive_txid": _TXID_B,
                "send_amount_msat": 100_000_000,
                "receive_amount_msat": 99_500_000,
            },
        )
        inbound = _row(
            id="in",
            external_id=_TXID_B,
            wallet_id="liquid",
            direction="inbound",
            asset="LBTC",
            amount=99_500_000,
            raw_json={
                "provider": "boltz",
                "swap_id": "route-conflict",
                "flow": "chain",
                "send_txid": _TXID_C,
                "receive_txid": "44" * 32,
                "send_amount_msat": 100_000_000,
                "receive_amount_msat": 99_500_000,
            },
        )

        candidate = suggest_swap_candidates([out, inbound])[0]

        self.assertEqual(candidate.method, METHOD_PROVIDER_SWAP_ID)
        self.assertEqual(candidate.confidence, CONFIDENCE_STRONG)

    def test_strong_provider_hint_does_not_add_onchain_heuristic_sibling(self):
        provider = {
            "provider": "boltz",
            "swap_id": "ambiguous-provider-route",
            "flow": "chain",
        }
        out = _row(
            id="out",
            wallet_id="source",
            direction="outbound",
            amount=100_000_000,
            raw_json=provider,
        )
        provider_in = _row(
            id="provider-in",
            wallet_id="provider-destination",
            direction="inbound",
            amount=99_500_000,
            raw_json=provider,
        )
        other_in = _row(
            id="other-in",
            wallet_id="other-destination",
            direction="inbound",
            amount=99_500_000,
        )

        candidates = suggest_swap_candidates([out, provider_in, other_in])

        self.assertEqual({candidate.in_id for candidate in candidates}, {"provider-in"})
        self.assertEqual(candidates[0].conflict_size, 1)
        self.assertEqual(candidates[0].method, METHOD_PROVIDER_SWAP_ID)

    def test_duplicate_provider_rows_never_become_exact(self):
        raw = {
            "provider": "boltz",
            "swap_id": "duplicate",
            "flow": "chain",
            "send_txid": _TXID_A,
            "receive_txid": _TXID_B,
            "send_amount_msat": 100_000_000,
            "receive_amount_msat": 99_500_000,
        }
        outs = [
            _row(
                id=f"out-{index}", external_id=_TXID_A, direction="outbound",
                wallet_id=f"btc-{index}", asset="BTC", amount=100_000_000,
                raw_json=raw,
            )
            for index in range(2)
        ]
        inbound = _row(
            id="in", external_id=_TXID_B, direction="inbound",
            wallet_id="liquid", asset="LBTC", amount=99_500_000, raw_json=raw,
        )

        candidates = suggest_swap_candidates([*outs, inbound])

        self.assertEqual(len(candidates), 2)
        self.assertTrue(all(item.confidence == CONFIDENCE_STRONG for item in candidates))
        self.assertTrue(all(item.conflict_size == 2 for item in candidates))

    def test_exact_evidence_filter_cannot_make_provider_key_falsely_unique(self):
        provider = {
            "provider": "boltz",
            "swap_id": "filtered-duplicate",
            "flow": "chain",
            "send_txid": _TXID_A,
            "receive_txid": _TXID_B,
            "send_amount_msat": 100_000_000,
            "receive_amount_msat": 99_500_000,
        }
        hash_out = _row(
            id="hash-out",
            external_id=_TXID_A,
            wallet_id="node",
            wallet_kind="lnd",
            payment_hash=_PAY_HASH,
            payment_hash_source="lnd",
            direction="outbound",
            amount=100_000_000,
            raw_json=provider,
        )
        duplicate_out = _row(
            id="provider-out",
            external_id=_TXID_A,
            wallet_id="other-source",
            direction="outbound",
            amount=100_000_000,
            raw_json=provider,
        )
        provider_in = _row(
            id="provider-in",
            external_id=_TXID_B,
            wallet_id="liquid",
            direction="inbound",
            asset="LBTC",
            amount=99_500_000,
            raw_json=provider,
        )
        hash_in = _row(
            id="hash-in",
            external_id=_TXID_C,
            wallet_id="chain",
            wallet_kind="descriptor",
            payment_hash=_PAY_HASH,
            payment_hash_source="chain_script_unique_outpoint",
            direction="inbound",
            amount=99_500_000,
        )

        candidates = suggest_swap_candidates(
            [hash_out, duplicate_out, provider_in, hash_in]
        )
        provider_candidate = next(
            candidate
            for candidate in candidates
            if candidate.out_id == "provider-out"
            and candidate.in_id == "provider-in"
        )

        self.assertEqual(provider_candidate.method, METHOD_PROVIDER_SWAP_ID)
        self.assertEqual(provider_candidate.confidence, CONFIDENCE_STRONG)

    def test_fractional_provider_amounts_never_become_exact(self):
        raw = {
            "provider": "boltz",
            "swap_id": "fractional",
            "flow": "chain",
            "send_txid": _TXID_A,
            "receive_txid": _TXID_B,
            "send_amount_msat": 100_000_000.9,
            "receive_amount_msat": 99_500_000.9,
        }
        out = _row(
            id="out", external_id=_TXID_A, direction="outbound",
            wallet_id="btc", asset="BTC", amount=100_000_000, raw_json=raw,
        )
        inbound = _row(
            id="in", external_id=_TXID_B, direction="inbound",
            wallet_id="liquid", asset="LBTC", amount=99_500_000, raw_json=raw,
        )

        candidate = suggest_swap_candidates([out, inbound])[0]

        self.assertEqual(candidate.method, METHOD_PROVIDER_SWAP_ID)
        self.assertEqual(candidate.confidence, CONFIDENCE_STRONG)

    def test_refunded_provider_status_overrides_chain_swap_flow(self):
        raw = {
            "source": "bullbitcoin_wallet_csv",
            "type": "chain_swap",
            "status": "refunded",
            "swap_id": "swap-refund",
            "send_txid": _TXID_A,
            "receive_txid": _TXID_B,
            "send_amount_msat": 1_000_000_000,
            "receive_amount_msat": 998_000_000,
        }
        out = _row(
            id="btc-out",
            external_id=_TXID_A,
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
            external_id=_TXID_B,
            wallet_id="bull-btc",
            wallet_kind="bullbitcoin",
            direction="inbound",
            asset="BTC",
            amount=998_000_000,
            raw_json=raw,
        )

        candidates = suggest_swap_candidates([out, inbound])

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

        self.assertEqual(suggest_swap_candidates([out, inbound]), [])

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
                    suggest_swap_candidates([out, inbound]),
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
        candidates = suggest_swap_candidates([out, inbound])
        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].method, METHOD_PROVIDER_SWAP_ID)
        self.assertEqual(candidates[0].confidence, CONFIDENCE_STRONG)
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
        candidates = suggest_swap_candidates([out, inbound])
        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].method, METHOD_PROVIDER_SWAP_ID)
        self.assertEqual(candidates[0].confidence, CONFIDENCE_STRONG)
        self.assertEqual(candidates[0].default_kind, KIND_CHAIN_SWAP)
        self.assertEqual(candidates[0].evidence_id, "boltz-chain-id")
        self.assertEqual(candidates[0].evidence_version, "2")
        self.assertEqual(candidates[0].evidence_taproot, "True")
        self.assertEqual(candidates[0].evidence_cooperative, "True")
        self.assertEqual(candidates[0].evidence_spend_path, "key")


class ProviderEvidenceAdversarialTests(unittest.TestCase):
    def _raw(self, txid, *, send_txid=_TXID_A, receive_txid=_TXID_B, **extra):
        return {
            "txid": txid,
            "chain": "bitcoin",
            "network": "main",
            "provider": "boltz",
            "swap_id": "same-swap",
            "flow": "chain-swap",
            "send_txid": send_txid,
            "receive_txid": receive_txid,
            "send_amount_msat": 100_000_000,
            "receive_amount_msat": 99_500_000,
            **extra,
        }

    def _rows(self, out_raw=None, in_raw=None):
        return [
            _row(
                id="out",
                external_id=_TXID_A,
                wallet_id="A",
                direction="outbound",
                amount=100_000_000,
                raw_json=out_raw or self._raw(_TXID_A),
            ),
            _row(
                id="in",
                external_id=_TXID_B,
                wallet_id="B",
                direction="inbound",
                amount=99_500_000,
                raw_json=in_raw or self._raw(_TXID_B),
            ),
        ]

    def test_provider_route_must_agree_with_canonical_graph_scope(self):
        candidates = suggest_swap_candidates(
            self._rows(out_raw=self._raw(_TXID_C))
        )

        self.assertTrue(candidates)
        self.assertFalse(
            [candidate for candidate in candidates if candidate.confidence == "exact"]
        )

    def test_contradictory_provider_route_aliases_never_become_exact(self):
        out_raw = self._raw(_TXID_A, lockup_txid=_TXID_C, claim_txid=_TXID_C)
        in_raw = self._raw(_TXID_B, lockup_txid=_TXID_C, claim_txid=_TXID_C)

        candidates = suggest_swap_candidates(self._rows(out_raw, in_raw))

        provider_candidates = [
            candidate
            for candidate in candidates
            if candidate.method == METHOD_PROVIDER_SWAP_ID
        ]
        self.assertEqual(len(provider_candidates), 1)
        self.assertEqual(provider_candidates[0].confidence, CONFIDENCE_STRONG)

    def test_contradictory_provider_identity_aliases_never_become_exact(self):
        out_raw = self._raw(_TXID_A, boltz_id="different-swap")
        in_raw = self._raw(_TXID_B, boltz_id="different-swap")

        candidates = suggest_swap_candidates(self._rows(out_raw, in_raw))

        provider_candidates = [
            candidate
            for candidate in candidates
            if candidate.method == METHOD_PROVIDER_SWAP_ID
        ]
        self.assertEqual(len(provider_candidates), 1)
        self.assertEqual(provider_candidates[0].confidence, CONFIDENCE_STRONG)

    def test_contradictory_provider_flow_aliases_never_become_exact(self):
        out_raw = self._raw(_TXID_A, type="reverse-submarine")
        in_raw = self._raw(_TXID_B, type="reverse-submarine")

        provider_candidates = [
            candidate
            for candidate in suggest_swap_candidates(self._rows(out_raw, in_raw))
            if candidate.method == METHOD_PROVIDER_SWAP_ID
        ]

        self.assertEqual(len(provider_candidates), 1)
        self.assertEqual(provider_candidates[0].confidence, CONFIDENCE_STRONG)

    def test_contradictory_provider_status_aliases_never_become_exact(self):
        out_raw = self._raw(_TXID_A, status="completed", state="refunded")
        in_raw = self._raw(_TXID_B, status="completed", state="refunded")

        provider_candidates = [
            candidate
            for candidate in suggest_swap_candidates(self._rows(out_raw, in_raw))
            if candidate.method == METHOD_PROVIDER_SWAP_ID
        ]

        self.assertEqual(len(provider_candidates), 1)
        self.assertEqual(provider_candidates[0].confidence, CONFIDENCE_STRONG)

    def test_active_pair_does_not_make_duplicate_provider_key_unique(self):
        rows = []
        for suffix in ("1", "2"):
            out, inbound = self._rows()
            out = {**out, "id": f"out-{suffix}"}
            inbound = {**inbound, "id": f"in-{suffix}"}
            rows.extend((out, inbound))

        candidates = suggest_swap_candidates(
            rows,
            pair_records=[
                {
                    "out_transaction_id": "out-1",
                    "in_transaction_id": "in-1",
                    "deleted_at": None,
                }
            ],
        )

        remaining = [
            candidate
            for candidate in candidates
            if candidate.out_id == "out-2" and candidate.in_id == "in-2"
        ]
        self.assertEqual(len(remaining), 1)
        self.assertEqual(remaining[0].confidence, CONFIDENCE_STRONG)


class HeuristicMatchTests(unittest.TestCase):
    def test_same_txid_onchain_rows_do_not_become_heuristic_candidates(self):
        out = _row(
            id="cold-out",
            external_id=_TXID_A,
            wallet_id="cold",
            wallet_label="Cold",
            direction="outbound",
            asset="BTC",
            amount=100_100_000_000,
        )
        inbound = _row(
            id="hot-in",
            external_id=_TXID_A,
            wallet_id="hot",
            wallet_label="Hot",
            direction="inbound",
            asset="BTC",
            occurred_at="2026-03-14T17:32:00Z",
            amount=100_000_000_000,
        )
        self.assertEqual(suggest_swap_candidates([out, inbound]), [])

    def test_different_onchain_txids_do_not_match_by_time_and_amount(self):
        out = _row(
            id="later-payment",
            external_id=_TXID_A,
            wallet_id="cold",
            wallet_kind="descriptor",
            direction="outbound",
            asset="BTC",
            occurred_at="2023-02-01T04:41:00Z",
            amount=15_025_943_000,
            fee=652_000,
        )
        inbound = _row(
            id="earlier-funding",
            external_id=_TXID_B,
            wallet_id="hot",
            wallet_kind="descriptor",
            direction="inbound",
            asset="BTC",
            occurred_at="2023-02-01T02:15:00Z",
            amount=14_964_523_000,
        )

        self.assertEqual(suggest_swap_candidates([out, inbound]), [])

    def test_same_asset_chain_to_lightning_remains_a_heuristic_candidate(self):
        out = _row(
            id="chain-lockup",
            wallet_id="chain",
            wallet_kind="descriptor",
            direction="outbound",
            asset="BTC",
            amount=100_000_000,
        )
        inbound = _row(
            id="lightning-settlement",
            wallet_id="node",
            wallet_kind="lnd",
            direction="inbound",
            asset="BTC",
            occurred_at="2026-03-14T17:32:00Z",
            amount=99_500_000,
        )

        candidates = suggest_swap_candidates([out, inbound])

        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].method, METHOD_HEURISTIC)
        self.assertEqual(candidates[0].default_kind, KIND_SUBMARINE_SWAP)

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
        candidates = suggest_swap_candidates([out, inbound])
        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].default_kind, KIND_PEG_IN)

    def test_booked_move_anchors_suppress_swap_candidates(self):
        out = _row(
            id="o",
            external_id=_TXID_A,
            wallet_id="cold",
            direction="outbound",
            asset="BTC",
            amount=100_000_000_000,
            fee=100_000_000,
        )
        inbound = _row(
            id="i",
            external_id=_TXID_A,
            wallet_id="hot",
            direction="inbound",
            asset="BTC",
            amount=100_000_000_000,
        )
        rows = [out, inbound]
        self.assertEqual(
            suggest_swap_candidates(rows, booked_move_transaction_ids={"o", "i"}),
            [],
        )

    def test_zero_value_placeholder_does_not_restore_onchain_heuristic(self):
        txid = "11" * 32
        out = _row(id="o", external_id=txid, wallet_id="cold",
                   direction="outbound", asset="BTC", amount=100_100_000_000)
        inbound = _row(id="i", external_id=txid, wallet_id="hot",
                       direction="inbound", asset="BTC", amount=100_000_000_000)
        zero_inbound = _row(id="z", external_id=txid, wallet_id="csv",
                            direction="inbound", asset="BTC", amount=0)

        rows = [out, inbound, zero_inbound]
        self.assertEqual(suggest_swap_candidates(rows), [])

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
        self.assertEqual(suggest_swap_candidates([out, inbound]), [])

    def test_custom_wallet_cross_asset_peg_is_not_inferred(self):
        # `custom` is too broad: it can mean custodians, exchanges, CSV-only
        # sources, or self-custody wallets. Do not infer a carrying-value peg
        # from asset shape alone.
        out = _row(id="o", external_id="", wallet_id="w1", wallet_kind="custom",
                   direction="outbound", asset="BTC", amount=100_000_000)
        inbound = _row(id="i", external_id="", wallet_id="w2", wallet_kind="custom",
                       direction="inbound", asset="LBTC", amount=99_900_000,
                       occurred_at="2026-03-14T17:31:00Z")
        self.assertEqual(suggest_swap_candidates([out, inbound]), [])

    def test_all_booked_fanout_anchors_are_suppressed(self):
        out = _row(
            id="o", external_id=_TXID_A, wallet_id="cold",
            direction="outbound", asset="BTC", amount=100_000_000,
        )
        large_in = _row(
            id="i-large", external_id=_TXID_A, wallet_id="hot",
            direction="inbound", asset="BTC", amount=99_500_000,
        )
        small_in = _row(
            id="i-small", external_id=_TXID_A, wallet_id="savings",
            direction="inbound", asset="BTC", amount=500_000,
        )
        rows = [out, large_in, small_in]
        self.assertEqual(
            suggest_swap_candidates(
                rows,
                booked_move_transaction_ids={"o", "i-large", "i-small"},
            ),
            [],
        )

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
        candidates = suggest_swap_candidates([out, inbound])
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
        self.assertEqual(suggest_swap_candidates([out, inbound]), [])

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
            suggest_swap_candidates([out, inbound], time_window_seconds=24 * 3600),
            [],
        )

    def test_inbound_larger_than_outbound_rejected(self):
        out = _row(id="o", wallet_id="A", direction="outbound", amount=100, asset="LBTC")
        inbound = _row(id="i", wallet_id="B", direction="inbound", amount=200, asset="BTC")
        self.assertEqual(suggest_swap_candidates([out, inbound]), [])

    def test_absolute_fee_floor_admits_small_swap(self):
        out = _row(id="o", wallet_id="A", direction="outbound", amount=10_000_000, asset="LBTC")  # 0.0001 BTC
        inbound = _row(id="i", wallet_id="B", direction="inbound", amount=8_000_000, asset="BTC")  # 0.00008 BTC
        # 1% of 10_000_000 msat = 100_000 msat = 100 sats. Floor 2500 sats = 2_500_000 msat.
        # Delta is 2_000_000 msat = 2_000 sats, below floor → admitted.
        candidates = suggest_swap_candidates([out, inbound])
        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].confidence, CONFIDENCE_STRONG)

    def test_zero_amount_inbound_rejected(self):
        # A zero-amount inbound row sits within the absolute fee floor of any
        # small outbound; it must never become a heuristic candidate.
        out = _row(id="o", wallet_id="A", direction="outbound", amount=2_000_000, asset="LBTC")  # 2000 sats
        inbound = _row(id="i", wallet_id="B", direction="inbound", amount=0, asset="BTC")
        self.assertEqual(suggest_swap_candidates([out, inbound]), [])

    def test_negative_amount_inbound_rejected(self):
        out = _row(id="o", wallet_id="A", direction="outbound", amount=2_000_000, asset="LBTC")
        inbound = _row(id="i", wallet_id="B", direction="inbound", amount=-1_000_000, asset="BTC")
        self.assertEqual(suggest_swap_candidates([out, inbound]), [])

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
            [*outs, *ins], time_window_seconds=window
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
    def test_ownership_review_edge_blocks_conflicting_exact_auto_pair(self):
        exact = suggest_swap_candidates(
            [
                _row(
                    id="out",
                    wallet_id="node",
                    wallet_kind="lnd",
                    payment_hash=_CLAIM_HASH,
                    payment_hash_source="lnd",
                    direction="outbound",
                    amount=100_000_000,
                ),
                _row(
                    id="exact-in",
                    external_id=_TXID_C,
                    wallet_id="chain",
                    payment_hash=_CLAIM_HASH,
                    payment_hash_source="chain_script_unique_outpoint",
                    direction="inbound",
                    asset="LBTC",
                    amount=99_500_000,
                    raw_json=_claim_raw(_TXID_C, liquid=True),
                ),
            ]
        )[0]
        ownership_review = replace(
            exact,
            in_id="ownership-in",
            in_wallet_id="other-owned-wallet",
            confidence=CONFIDENCE_STRONG,
            method=METHOD_OWNERSHIP_GRAPH,
            conflict_set_id="",
            conflict_size=1,
        )

        clustered = finalize_candidate_conflicts([exact, ownership_review])

        self.assertEqual(len(clustered), 2)
        self.assertTrue(all(candidate.conflict_size == 2 for candidate in clustered))

    def test_strong_provider_edge_is_not_hidden_by_exact_hash_edge(self):
        out = _row(
            id="out",
            wallet_id="node",
            wallet_kind="lnd",
            payment_hash=_CLAIM_HASH,
            payment_hash_source="lnd",
            direction="outbound",
            amount=100_000_000,
        )
        out["raw_json"].update(
            {"provider": "boltz", "swap_id": "ambiguous", "flow": "chain-swap"}
        )
        exact_in = _row(
            id="exact-in",
            external_id=_TXID_C,
            wallet_id="chain",
            payment_hash=_CLAIM_HASH,
            payment_hash_source="chain_script_unique_outpoint",
            direction="inbound",
            asset="LBTC",
            amount=99_500_000,
            raw_json=_claim_raw(_TXID_C, liquid=True),
        )
        provider_in = _row(
            id="provider-in",
            wallet_id="provider-wallet",
            direction="inbound",
            asset="LBTC",
            amount=99_400_000,
            raw_json={
                "provider": "boltz",
                "swap_id": "ambiguous",
                "flow": "chain-swap",
            },
        )

        candidates = suggest_swap_candidates([out, exact_in, provider_in])

        self.assertEqual(
            {(candidate.method, candidate.in_id) for candidate in candidates},
            {
                (METHOD_PAYMENT_HASH, "exact-in"),
                (METHOD_PROVIDER_SWAP_ID, "provider-in"),
            },
        )
        self.assertTrue(all(candidate.conflict_size == 2 for candidate in candidates))

    def test_two_heuristic_candidates_share_leg_get_same_cluster_id(self):
        out = _row(id="o", wallet_id="A", asset="LBTC", direction="outbound", amount=124_262_750_000)
        in1 = _row(id="i1", wallet_id="B", asset="BTC", direction="inbound",
                   amount=124_132_980_000, occurred_at="2026-03-14T17:32:00Z")
        in2 = _row(id="i2", wallet_id="C", asset="BTC", direction="inbound",
                   amount=124_132_980_000, occurred_at="2026-03-14T17:33:00Z")
        candidates = suggest_swap_candidates([out, in1, in2])
        self.assertEqual(len(candidates), 2)
        self.assertEqual(candidates[0].conflict_set_id, candidates[1].conflict_set_id)
        self.assertEqual([c.conflict_size for c in candidates], [2, 2])

    def test_solo_candidate_gets_conflict_size_one(self):
        out = _row(id="o", wallet_id="A", asset="LBTC", direction="outbound", amount=100_000_000)
        inbound = _row(id="i", wallet_id="B", asset="BTC", direction="inbound",
                       amount=99_500_000, occurred_at="2026-03-14T17:32:00Z")
        candidates = suggest_swap_candidates([out, inbound])
        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].conflict_size, 1)

    def test_onchain_heuristic_does_not_create_cross_type_conflict(self):
        # The same-asset on-chain row is not a candidate. It therefore cannot
        # manufacture a conflict around the legitimate cross-asset review.
        out = _row(id="o", wallet_id="A", asset="BTC", direction="outbound", amount=100_000_000_000)
        transfer_in = _row(id="i-btc", wallet_id="B", asset="BTC", direction="inbound",
                           amount=99_900_000_000, occurred_at="2026-03-14T17:40:00Z")
        swap_in = _row(id="i-lbtc", wallet_id="C", asset="LBTC", direction="inbound",
                       amount=99_800_000_000, occurred_at="2026-03-14T17:45:00Z")
        candidates = suggest_swap_candidates([out, transfer_in, swap_in])
        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].in_id, "i-lbtc")
        self.assertEqual(candidates[0].conflict_size, 1)

    def test_exact_dominates_heuristic_with_overlap(self):
        # Exact (payment_hash) and heuristic candidates that share the same
        # outbound leg: exact wins, heuristic drops out.
        out = _row(
            id="o",
            wallet_id="A",
            wallet_kind="lnd",
            kind="lnd_pay",
            payment_hash=_CLAIM_HASH,
            payment_hash_source="lnd",
            direction="outbound",
            amount=100_000_000,
        )
        exact_in = _row(
            id="exact_in",
            external_id=_TXID_C,
            wallet_id="B",
            wallet_kind="descriptor",
            payment_hash=_CLAIM_HASH,
            payment_hash_source="chain_script_unique_outpoint",
            direction="inbound",
            asset="LBTC",
            occurred_at="2026-03-14T17:32:00Z",
            amount=99_500_000,
            raw_json=_claim_raw(_TXID_C, liquid=True),
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
        candidates = suggest_swap_candidates([out, exact_in, heuristic_in])
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
                payment_hash_source="lnd",
                direction="outbound",
                amount=100_000_000,
            ),
            _row(
                id="i",
                wallet_id="B",
                wallet_kind="descriptor",
                payment_hash=_PAY_HASH,
                payment_hash_source="chain_script_unique_outpoint",
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
        )
        self.assertEqual(candidates, [])

    def test_soft_deleted_pair_does_not_skip(self):
        candidates = suggest_swap_candidates(
            self._legs(),
            pair_records=[
                {"out_transaction_id": "o", "in_transaction_id": "i", "deleted_at": "2026-04-01T00:00:00Z"}
            ],
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
            now_iso="2026-06-01T00:00:00Z",
        )
        self.assertEqual(len(candidates), 1)

    def test_dismissed_exact_link_does_not_consume_leg_before_heuristic(self):
        out, exact_in = self._legs()
        heuristic_in = _row(
            id="heuristic-in",
            wallet_id="C",
            wallet_kind="descriptor",
            direction="inbound",
            asset="LBTC",
            occurred_at="2026-03-14T17:33:00Z",
            amount=99_400_000,
        )

        candidates = suggest_swap_candidates(
            [out, exact_in, heuristic_in],
            dismissals=[
                {
                    "out_transaction_id": "o",
                    "in_transaction_id": "i",
                    "expires_at": "2099-01-01T00:00:00Z",
                }
            ],
            now_iso="2026-06-01T00:00:00Z",
        )

        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].in_id, "heuristic-in")
        self.assertEqual(candidates[0].method, METHOD_HEURISTIC)


class ExcludedRowsTests(unittest.TestCase):
    def test_excluded_rows_ignored(self):
        out = _row(id="o", wallet_id="A", payment_hash=_PAY_HASH, direction="outbound", excluded=1)
        inbound = _row(id="i", wallet_id="B", payment_hash=_PAY_HASH, direction="inbound", asset="LBTC")
        self.assertEqual(suggest_swap_candidates([out, inbound]), [])


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

        self.assertEqual(suggest_swap_candidates([out, inbound]), [])

class RefundLinkMatchingTests(unittest.TestCase):
    def test_historical_raw_witness_recovers_exact_refund_outpoint(self):
        funding_txid = "ab" * 32
        redeem_script = (
            "a914e81bfa71da56f187cce1319ee773dabf56988e9587632102"
            + "11" * 32
            + "6703000010b1752103"
            + "22" * 32
            + "68ac"
        )
        lockup = _row(
            id="historic-lockup",
            wallet_id="wallet-a",
            external_id=funding_txid,
            direction="outbound",
            amount=10_000_000,
            raw_json=json.dumps(
                {
                    "txid": funding_txid,
                    "vout": [{"n": 2, "value_sats": 10_000}],
                }
            ),
        )
        refund = _row(
            id="historic-refund",
            wallet_id="wallet-a",
            external_id="cd" * 32,
            direction="inbound",
            amount=9_950_000,
            raw_json=json.dumps(
                {
                    "txid": "cd" * 32,
                    "vin": [
                        {
                            "txid": funding_txid,
                            "vout": 2,
                            "witness": ["3045", "", redeem_script],
                        }
                    ]
                }
            ),
        )

        candidates = suggest_swap_candidates([lockup, refund])

        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].method, METHOD_HTLC_REFUND)
        self.assertEqual(candidates[0].confidence, CONFIDENCE_EXACT)
        self.assertEqual(candidates[0].evidence_id, f"{funding_txid}:2")

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
        self.assertEqual(candidate.confidence, CONFIDENCE_STRONG)
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
        # refund candidate. Legacy txid-only evidence stays manual-review
        # strength until an exact output index is available.
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
        self.assertEqual(candidates[0].evidence_id, "lockup-txid")
        self.assertEqual(candidates[0].evidence_spend_path, "timeout")
        self.assertEqual(candidates[0].confidence, CONFIDENCE_STRONG)

    def test_refund_link_does_not_absorb_other_lockups_as_a_fake_fee(self):
        lockup_batch = _row(
            id="lockup-batch",
            wallet_id="wallet-a",
            external_id="lockup-batch-txid",
            direction="outbound",
            amount=30_000_000,
        )
        one_refund = _row(
            id="one-refund",
            wallet_id="wallet-a",
            external_id="refund-txid",
            swap_refund_funding_outpoint="lockup-batch-txid:2",
            direction="inbound",
            amount=9_950_000,
        )

        candidates = suggest_swap_candidates([lockup_batch, one_refund])

        self.assertEqual(candidates, [])

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
        self.assertEqual(candidates[0].evidence_id, mixed_txid.lower())
        self.assertEqual(candidates[0].evidence_spend_path, "timeout")

    def test_refund_link_outpoint_field_without_witness_stays_strong(self):
        lockup = _row(
            id="lockup",
            wallet_id="wallet-a",
            external_id="lockup-txid",
            direction="outbound",
            amount=10_000_000,
            raw_json=json.dumps(
                {"vout": [{"n": 2, "value": 10_000}]}
            ),
        )
        refund = _row(
            id="refund",
            wallet_id="wallet-a",
            external_id="refund-txid",
            swap_refund_funding_outpoint="lockup-txid:2",
            direction="inbound",
            amount=9_950_000,
        )
        candidates = suggest_swap_candidates([lockup, refund])
        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].method, METHOD_HTLC_REFUND)
        self.assertEqual(candidates[0].confidence, CONFIDENCE_STRONG)
        self.assertEqual(candidates[0].evidence_id, "lockup-txid:2")
        self.assertEqual(candidates[0].evidence_spend_path, "timeout")

    def test_canonical_outpoint_without_refund_witness_is_not_exact(self):
        funding_txid = "55" * 32
        lockup = _row(
            id="lockup",
            wallet_id="wallet-a",
            external_id=funding_txid,
            direction="outbound",
            amount=10_000_000,
            raw_json={"txid": funding_txid, "vout": [{"n": 2, "value_sats": 10_000}]},
        )
        refund = _row(
            id="refund",
            wallet_id="wallet-a",
            external_id="66" * 32,
            swap_refund_funding_outpoint=f"{funding_txid}:2",
            direction="inbound",
            amount=9_950_000,
        )

        candidate = suggest_swap_candidates([lockup, refund])[0]

        self.assertEqual(candidate.method, METHOD_HTLC_REFUND)
        self.assertEqual(candidate.confidence, CONFIDENCE_STRONG)

    def test_duplicate_funding_rows_do_not_become_exact_refund_pairs(self):
        lockups = [
            _row(
                id=f"lockup-{index}",
                wallet_id="wallet-a",
                external_id="lockup-txid",
                direction="outbound",
                amount=10_000_000,
            )
            for index in range(2)
        ]
        refund = _row(
            id="refund",
            wallet_id="wallet-a",
            external_id="refund-txid",
            swap_refund_funding_txid="lockup-txid",
            direction="inbound",
            amount=9_950_000,
        )
        candidates = suggest_swap_candidates([*lockups, refund])
        self.assertFalse(
            [candidate for candidate in candidates if candidate.method == METHOD_HTLC_REFUND]
        )

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
