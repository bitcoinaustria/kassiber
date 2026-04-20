import unittest
from decimal import Decimal

from kassiber.core.austrian import (
    AT_SWAP_QUARANTINE_REASON,
    AT_SWAP_TWO_PASS_REASON_CODE,
    REGIME_ALT,
    REGIME_NEU,
    infer_regime_from_timestamp,
    resolve_pool_id,
)
from kassiber.core.tax_events import normalize_tax_asset_inputs


def _row(
    tx_id,
    wallet_id,
    direction,
    amount,
    *,
    occurred_at="2026-01-01T00:00:00Z",
    asset="BTC",
    fee=0,
    fiat_rate=50_000,
    fiat_value=None,
    external_id=None,
):
    return {
        "id": tx_id,
        "wallet_id": wallet_id,
        "occurred_at": occurred_at,
        "direction": direction,
        "asset": asset,
        "amount": amount,
        "fee": fee,
        "fiat_rate": fiat_rate,
        "fiat_value": fiat_value,
        "kind": "deposit" if direction == "inbound" else "withdrawal",
        "description": tx_id,
        "note": None,
        "external_id": external_id or tx_id,
    }


class InferRegimeFromTimestampTest(unittest.TestCase):
    def test_altvermoegen_before_cutoff(self):
        self.assertEqual(
            infer_regime_from_timestamp("2021-01-15T12:00:00+01:00"),
            REGIME_ALT,
        )

    def test_neuvermoegen_at_cutoff(self):
        # Europe/Vienna 2021-03-01 00:00:00 — the cutoff itself is the first Neu moment.
        self.assertEqual(
            infer_regime_from_timestamp("2021-03-01T00:00:00+01:00"),
            REGIME_NEU,
        )

    def test_neuvermoegen_modern(self):
        self.assertEqual(
            infer_regime_from_timestamp("2025-12-01T00:00:00Z"),
            REGIME_NEU,
        )

    def test_handles_z_suffix(self):
        self.assertEqual(
            infer_regime_from_timestamp("2020-06-15T00:00:00Z"),
            REGIME_ALT,
        )


class ResolvePoolIdTest(unittest.TestCase):
    def test_wallet_id_becomes_pool_id(self):
        self.assertEqual(resolve_pool_id("wallet-abc"), "wallet-abc")

    def test_missing_wallet_id_falls_back_to_default(self):
        self.assertEqual(resolve_pool_id(None), "default")
        self.assertEqual(resolve_pool_id(""), "default")


class AustrianNormalizationTest(unittest.TestCase):
    def setUp(self):
        self.at_profile = {
            "id": "profile-at",
            "workspace_id": "ws-1",
            "tax_country": "at",
        }
        self.generic_profile = {
            "id": "profile-gen",
            "workspace_id": "ws-1",
            "tax_country": "generic",
        }
        self.wallet_refs = {
            "wallet-a": {"id": "wallet-a", "label": "Wallet A"},
            "wallet-b": {"id": "wallet-b", "label": "Wallet B"},
        }

    def test_at_inbound_event_gets_neu_regime_and_pool(self):
        rows = [_row("buy-1", "wallet-a", "inbound", 1, occurred_at="2025-06-01T00:00:00Z")]
        result = normalize_tax_asset_inputs(
            self.at_profile, "BTC", rows, self.wallet_refs, []
        )
        self.assertEqual(len(result.events), 1)
        event = result.events[0]
        self.assertEqual(event.at_regime, REGIME_NEU)
        self.assertEqual(event.at_pool, "wallet-a")
        self.assertIsNone(event.at_swap_link)
        self.assertIsNone(event.carried_basis_fiat)

    def test_at_inbound_event_pre_cutoff_gets_alt_regime(self):
        rows = [_row("buy-1", "wallet-a", "inbound", 1, occurred_at="2020-06-01T00:00:00Z")]
        result = normalize_tax_asset_inputs(
            self.at_profile, "BTC", rows, self.wallet_refs, []
        )
        event = result.events[0]
        self.assertEqual(event.at_regime, REGIME_ALT)

    def test_generic_profile_leaves_at_fields_none(self):
        rows = [_row("buy-1", "wallet-a", "inbound", 1, occurred_at="2025-06-01T00:00:00Z")]
        result = normalize_tax_asset_inputs(
            self.generic_profile, "BTC", rows, self.wallet_refs, []
        )
        event = result.events[0]
        self.assertIsNone(event.at_regime)
        self.assertIsNone(event.at_pool)
        self.assertIsNone(event.at_swap_link)

    def test_at_swap_link_tags_inbound_with_carried_basis(self):
        rows = [_row("buy-1", "wallet-a", "inbound", 1, occurred_at="2025-06-01T00:00:00Z")]
        result = normalize_tax_asset_inputs(
            self.at_profile,
            "BTC",
            rows,
            self.wallet_refs,
            [],
            at_swap_link_by_row_id={"buy-1": "swap-42"},
            at_carried_basis_by_row_id={"buy-1": Decimal("40000")},
        )
        event = result.events[0]
        self.assertEqual(event.at_swap_link, "swap-42")
        self.assertEqual(event.carried_basis_fiat, Decimal("40000"))

    def test_at_transfer_gets_pool_from_source_wallet(self):
        rows = [
            _row("out-1", "wallet-a", "outbound", 1, occurred_at="2025-06-01T00:00:00Z", external_id="chain-1"),
            _row("in-1", "wallet-b", "inbound", 1, occurred_at="2025-06-01T00:00:00Z", external_id="chain-1"),
        ]
        pair = {"out": rows[0], "in": rows[1]}
        result = normalize_tax_asset_inputs(
            self.at_profile, "BTC", rows, self.wallet_refs, [pair]
        )
        self.assertEqual(len(result.transfers), 1)
        self.assertEqual(result.transfers[0].at_pool, "wallet-a")


class AtCrossAssetSwapEngineTest(unittest.TestCase):
    """End-to-end engine-level handling of AT cross-asset swap pairs.

    The engine is exercised via its private classifier because the full
    rp2 integration requires a sqlite-backed profile. The classifier is
    the seam where v1's quarantine decision is made.
    """

    def setUp(self):
        from kassiber.core.engines.rp2 import GenericRP2TaxEngine

        self.GenericRP2TaxEngine = GenericRP2TaxEngine

    def _make_row(self, tx_id, asset, direction, amount, occurred_at):
        return {
            "id": tx_id,
            "asset": asset,
            "direction": direction,
            "amount": amount,
            "occurred_at": occurred_at,
            "fee": 0,
            "fiat_rate": 50000 if asset == "BTC" else 3000,
            "fiat_value": None,
            "wallet_id": "wallet-a",
            "kind": "deposit" if direction == "inbound" else "withdrawal",
            "description": tx_id,
            "note": None,
            "external_id": tx_id,
        }

    def test_neu_cross_asset_swap_gets_quarantined(self):
        profile = {"id": "p1", "workspace_id": "w1", "tax_country": "at"}
        engine = self.GenericRP2TaxEngine(profile)
        rows = [
            self._make_row("out-1", "BTC", "outbound", 1, "2025-06-01T00:00:00Z"),
            self._make_row("in-1", "LBTC", "inbound", 1, "2025-06-01T00:00:00Z"),
        ]
        pairs = [
            {
                "pair_id": "mp-1",
                "kind": "swap",
                "policy": "taxable",
                "out_id": "out-1",
                "in_id": "in-1",
                "out_asset": "BTC",
                "in_asset": "LBTC",
            }
        ]
        swap_map, quarantined, quarantines = engine._classify_at_cross_asset_pairs(pairs, rows)
        self.assertEqual(quarantined, {"out-1", "in-1"})
        self.assertEqual(len(quarantines), 2)
        for q in quarantines:
            self.assertEqual(q["reason"], AT_SWAP_QUARANTINE_REASON)
            import json

            detail = json.loads(q["detail_json"])
            self.assertEqual(detail["reason_code"], AT_SWAP_TWO_PASS_REASON_CODE)
            self.assertEqual(detail["at_swap_link"], "mp-1")
            self.assertEqual(detail["outgoing_asset"], "BTC")
            self.assertEqual(detail["incoming_asset"], "LBTC")

    def test_alt_cross_asset_swap_realizes_without_quarantine(self):
        profile = {"id": "p1", "workspace_id": "w1", "tax_country": "at"}
        engine = self.GenericRP2TaxEngine(profile)
        rows = [
            self._make_row("out-1", "BTC", "outbound", 1, "2019-06-01T00:00:00Z"),
            self._make_row("in-1", "LBTC", "inbound", 1, "2019-06-01T00:00:00Z"),
        ]
        pairs = [
            {
                "pair_id": "mp-1",
                "kind": "swap",
                "policy": "taxable",
                "out_id": "out-1",
                "in_id": "in-1",
                "out_asset": "BTC",
                "in_asset": "LBTC",
            }
        ]
        swap_map, quarantined, quarantines = engine._classify_at_cross_asset_pairs(pairs, rows)
        self.assertEqual(swap_map, {})
        self.assertEqual(quarantined, set())
        self.assertEqual(quarantines, [])

    def test_non_at_profile_leaves_cross_asset_pairs_untouched(self):
        profile = {"id": "p1", "workspace_id": "w1", "tax_country": "generic"}
        engine = self.GenericRP2TaxEngine(profile)
        rows = [
            self._make_row("out-1", "BTC", "outbound", 1, "2025-06-01T00:00:00Z"),
            self._make_row("in-1", "LBTC", "inbound", 1, "2025-06-01T00:00:00Z"),
        ]
        pairs = [
            {
                "pair_id": "mp-1",
                "kind": "swap",
                "policy": "taxable",
                "out_id": "out-1",
                "in_id": "in-1",
                "out_asset": "BTC",
                "in_asset": "LBTC",
            }
        ]
        swap_map, quarantined, quarantines = engine._classify_at_cross_asset_pairs(pairs, rows)
        self.assertEqual(quarantined, set())
        self.assertEqual(quarantines, [])


if __name__ == "__main__":
    unittest.main()
