import json
import unittest
from decimal import Decimal

from kassiber.core.austrian import (
    AT_CATEGORY_TO_KENNZAHL,
    AT_NEU_CUTOFF,
    REGIME_ALT,
    REGIME_NEU,
    infer_outbound_regimes,
    infer_regime_from_timestamp,
    kennzahl_for_disposal_category,
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


class InferOutboundRegimesTest(unittest.TestCase):
    def test_post_cutoff_sale_with_only_alt_inventory_falls_back_to_alt(self):
        rows = [
            _row("buy-alt", "wallet-a", "inbound", 100_000_000, occurred_at="2020-06-01T00:00:00Z"),
            _row("sell-later", "wallet-a", "outbound", 100_000_000, occurred_at="2025-06-01T00:00:00Z"),
        ]
        self.assertEqual(infer_outbound_regimes(rows), {"sell-later": REGIME_ALT})

    def test_post_cutoff_sale_with_neu_inventory_stays_neu(self):
        rows = [
            _row("buy-alt", "wallet-a", "inbound", 50_000_000, occurred_at="2020-06-01T00:00:00Z"),
            _row("buy-neu", "wallet-a", "inbound", 100_000_000, occurred_at="2024-06-01T00:00:00Z"),
            _row("sell-later", "wallet-a", "outbound", 30_000_000, occurred_at="2025-06-01T00:00:00Z"),
        ]
        self.assertEqual(infer_outbound_regimes(rows), {"sell-later": REGIME_NEU})


class AustrianKennzahlMappingTest(unittest.TestCase):
    def test_neu_cutoff_matches_rp2_fork_contract(self):
        from rp2.plugin.country.at import AT_NEU_CUTOFF as RP2_AT_NEU_CUTOFF

        self.assertEqual(AT_NEU_CUTOFF, RP2_AT_NEU_CUTOFF)

    def test_kennzahl_mapping_covers_every_rp2_category(self):
        from rp2.plugin.country.at import AtDisposalCategory

        self.assertEqual(
            set(AT_CATEGORY_TO_KENNZAHL),
            {category.value for category in AtDisposalCategory},
        )

    def test_maps_known_categories(self):
        self.assertEqual(kennzahl_for_disposal_category("income_general"), 172)
        self.assertEqual(kennzahl_for_disposal_category("income_capital_yield"), 172)
        self.assertEqual(kennzahl_for_disposal_category("neu_gain"), 174)
        self.assertEqual(kennzahl_for_disposal_category("neu_loss"), 176)
        self.assertEqual(kennzahl_for_disposal_category("alt_spekulation"), 801)

    def test_returns_none_for_non_reported_categories(self):
        self.assertIsNone(kennzahl_for_disposal_category("neu_swap"))
        self.assertIsNone(kennzahl_for_disposal_category("alt_taxfree"))
        self.assertIsNone(kennzahl_for_disposal_category(None))


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

    def test_at_outbound_post_cutoff_with_only_alt_inventory_gets_alt_regime(self):
        rows = [
            _row("buy-alt", "wallet-a", "inbound", 100_000_000, occurred_at="2020-06-01T00:00:00Z"),
            _row("sell-later", "wallet-a", "outbound", 100_000_000, occurred_at="2025-06-01T00:00:00Z"),
        ]
        result = normalize_tax_asset_inputs(
            self.at_profile, "BTC", rows, self.wallet_refs, []
        )
        self.assertEqual(result.events[1].at_regime, REGIME_ALT)


class AtCrossAssetSwapEngineTest(unittest.TestCase):
    """End-to-end engine-level handling of AT cross-asset swap pairs.

    The engine is exercised via its private pre-pass because the full
    rp2 integration requires a sqlite-backed profile. The pre-pass is
    the seam where swap carry annotations and fallback quarantines are decided.
    """

    def setUp(self):
        from kassiber.core.engines.rp2 import GenericRP2TaxEngine

        self.GenericRP2TaxEngine = GenericRP2TaxEngine

    def _make_row(self, tx_id, asset, direction, amount, occurred_at, *, wallet_id="wallet-a", fiat_rate=None):
        return {
            "id": tx_id,
            "asset": asset,
            "direction": direction,
            "amount": amount,
            "occurred_at": occurred_at,
            "fee": 0,
            "fiat_rate": (50000 if asset == "BTC" else 3000) if fiat_rate is None else fiat_rate,
            "fiat_value": None,
            "wallet_id": wallet_id,
            "kind": "deposit" if direction == "inbound" else "withdrawal",
            "description": tx_id,
            "note": None,
            "external_id": tx_id,
        }

    def test_neu_cross_asset_swap_gets_annotations_and_carried_basis(self):
        profile = {"id": "p1", "workspace_id": "w1", "tax_country": "at"}
        engine = self.GenericRP2TaxEngine(profile)
        rows = [
            self._make_row("buy-1", "BTC", "inbound", 100_000_000_000, "2025-05-01T00:00:00Z"),
            self._make_row("out-1", "BTC", "outbound", 50_000_000_000, "2025-06-01T00:00:00Z"),
            self._make_row("in-1", "LBTC", "inbound", 50_000_000_000, "2025-06-01T00:00:00Z", wallet_id="wallet-b", fiat_rate=50_000),
        ]
        pairs = [
            {
                "pair_id": "mp-1",
                "kind": "swap",
                "policy": "carrying-value",
                "out_id": "out-1",
                "in_id": "in-1",
                "out_asset": "BTC",
                "in_asset": "LBTC",
            }
        ]
        regime_map, swap_map, carried_map, quarantined, quarantines = engine._annotate_at_cross_asset_pairs(pairs, rows, [])
        self.assertEqual(regime_map, {"out-1": REGIME_NEU})
        self.assertEqual(
            swap_map,
            {
                "out-1": "mp-1",
                "in-1": "mp-1",
            },
        )
        self.assertEqual(carried_map, {"in-1": Decimal("25000.0")})
        self.assertEqual(quarantined, set())
        self.assertEqual(quarantines, [])

    def test_alt_cross_asset_swap_realizes_without_quarantine(self):
        profile = {"id": "p1", "workspace_id": "w1", "tax_country": "at"}
        engine = self.GenericRP2TaxEngine(profile)
        rows = [
            self._make_row("buy-alt", "BTC", "inbound", 1, "2020-06-01T00:00:00Z"),
            self._make_row("out-1", "BTC", "outbound", 1, "2025-06-01T00:00:00Z"),
            self._make_row("in-1", "LBTC", "inbound", 1, "2025-06-01T00:00:00Z", wallet_id="wallet-b"),
        ]
        pairs = [
            {
                "pair_id": "mp-1",
                "kind": "swap",
                "policy": "carrying-value",
                "out_id": "out-1",
                "in_id": "in-1",
                "out_asset": "BTC",
                "in_asset": "LBTC",
            }
        ]
        regime_map, swap_map, carried_map, quarantined, quarantines = engine._annotate_at_cross_asset_pairs(pairs, rows, [])
        self.assertEqual(regime_map, {"out-1": REGIME_ALT})
        self.assertEqual(swap_map, {})
        self.assertEqual(carried_map, {})
        self.assertEqual(quarantined, set())
        self.assertEqual(quarantines, [])

    def test_reverse_direction_neu_cross_asset_swap_is_supported(self):
        profile = {"id": "p1", "workspace_id": "w1", "tax_country": "at"}
        engine = self.GenericRP2TaxEngine(profile)
        rows = [
            self._make_row("buy-1", "LBTC", "inbound", 100_000_000_000, "2025-05-01T00:00:00Z", fiat_rate=50_000),
            self._make_row("out-1", "LBTC", "outbound", 50_000_000_000, "2025-06-01T00:00:00Z", fiat_rate=50_000),
            self._make_row("in-1", "BTC", "inbound", 50_000_000_000, "2025-06-01T00:00:00Z", wallet_id="wallet-b"),
        ]
        pairs = [
            {
                "pair_id": "mp-1",
                "kind": "swap",
                "policy": "carrying-value",
                "out_id": "out-1",
                "in_id": "in-1",
                "out_asset": "LBTC",
                "in_asset": "BTC",
            }
        ]
        regime_map, swap_map, carried_map, quarantined, quarantines = engine._annotate_at_cross_asset_pairs(pairs, rows, [])
        self.assertEqual(regime_map, {"out-1": REGIME_NEU})
        self.assertEqual(
            swap_map,
            {
                "out-1": "mp-1",
                "in-1": "mp-1",
            },
        )
        self.assertEqual(carried_map, {"in-1": Decimal("25000.0")})
        self.assertEqual(quarantined, set())
        self.assertEqual(quarantines, [])

    def test_non_at_profile_leaves_cross_asset_pairs_untouched(self):
        profile = {"id": "p1", "workspace_id": "w1", "tax_country": "generic"}
        engine = self.GenericRP2TaxEngine(profile)
        rows = [
            self._make_row("out-1", "BTC", "outbound", 1, "2025-06-01T00:00:00Z"),
            self._make_row("in-1", "LBTC", "inbound", 1, "2025-06-01T00:00:00Z", wallet_id="wallet-b"),
        ]
        pairs = [
            {
                "pair_id": "mp-1",
                "kind": "swap",
                "policy": "carrying-value",
                "out_id": "out-1",
                "in_id": "in-1",
                "out_asset": "BTC",
                "in_asset": "LBTC",
            }
        ]
        regime_map, swap_map, carried_map, quarantined, quarantines = engine._annotate_at_cross_asset_pairs(pairs, rows, [])
        self.assertEqual(regime_map, {})
        self.assertEqual(swap_map, {})
        self.assertEqual(carried_map, {})
        self.assertEqual(quarantined, set())
        self.assertEqual(quarantines, [])

    def test_taxable_cross_asset_pair_stays_unannotated_for_at(self):
        profile = {"id": "p1", "workspace_id": "w1", "tax_country": "at"}
        engine = self.GenericRP2TaxEngine(profile)
        rows = [
            self._make_row("buy-1", "BTC", "inbound", 100_000_000_000, "2025-05-01T00:00:00Z"),
            self._make_row("out-1", "BTC", "outbound", 50_000_000_000, "2025-06-01T00:00:00Z"),
            self._make_row("in-1", "LBTC", "inbound", 50_000_000_000, "2025-06-01T00:00:00Z", wallet_id="wallet-b", fiat_rate=50_000),
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
        regime_map, swap_map, carried_map, quarantined, quarantines = engine._annotate_at_cross_asset_pairs(pairs, rows, [])
        self.assertEqual(regime_map, {"out-1": REGIME_NEU})
        self.assertEqual(swap_map, {})
        self.assertEqual(carried_map, {})
        self.assertEqual(quarantined, set())
        self.assertEqual(quarantines, [])

    def test_transfer_then_swap_uses_destination_wallet_pool(self):
        profile = {"id": "p1", "workspace_id": "w1", "tax_country": "at"}
        engine = self.GenericRP2TaxEngine(profile)
        rows = [
            self._make_row("buy-1", "BTC", "inbound", 100_000_000_000, "2025-01-01T00:00:00Z"),
            self._make_row("move-out", "BTC", "outbound", 50_000_000_000, "2025-02-01T00:00:00Z"),
            self._make_row("move-in", "BTC", "inbound", 50_000_000_000, "2025-02-01T00:00:00Z", wallet_id="wallet-b"),
            self._make_row("swap-out", "BTC", "outbound", 50_000_000_000, "2025-03-01T00:00:00Z", wallet_id="wallet-b", fiat_rate=60_000),
            self._make_row("swap-in", "LBTC", "inbound", 50_000_000_000, "2025-03-01T00:00:00Z", wallet_id="wallet-c", fiat_rate=60_000),
        ]
        pairs = [
            {
                "pair_id": "mp-1",
                "kind": "swap",
                "policy": "carrying-value",
                "out_id": "swap-out",
                "in_id": "swap-in",
                "out_asset": "BTC",
                "in_asset": "LBTC",
            }
        ]
        intra_pairs = [{"out": rows[1], "in": rows[2]}]
        regime_map, swap_map, carried_map, quarantined, quarantines = engine._annotate_at_cross_asset_pairs(
            pairs,
            rows,
            intra_pairs,
        )
        self.assertEqual(regime_map["swap-out"], REGIME_NEU)
        self.assertEqual(swap_map, {"swap-out": "mp-1", "swap-in": "mp-1"})
        self.assertEqual(carried_map, {"swap-in": Decimal("25000.0")})
        self.assertEqual(quarantined, set())
        self.assertEqual(quarantines, [])

    def test_same_timestamp_swap_chain_is_topologically_resolved(self):
        profile = {"id": "p1", "workspace_id": "w1", "tax_country": "at"}
        engine = self.GenericRP2TaxEngine(profile)
        rows = [
            self._make_row("buy-1", "BTC", "inbound", 100_000_000_000, "2025-01-01T00:00:00Z"),
            self._make_row("m-btc-out", "BTC", "outbound", 100_000_000_000, "2025-03-01T00:00:00Z", fiat_rate=60_000),
            self._make_row("z-lbtc-in", "LBTC", "inbound", 100_000_000_000, "2025-03-01T00:00:00Z", wallet_id="wallet-b", fiat_rate=60_000),
            self._make_row("a-lbtc-out", "LBTC", "outbound", 100_000_000_000, "2025-03-01T00:00:00Z", wallet_id="wallet-b", fiat_rate=60_000),
            self._make_row("b-xyz-in", "XYZ", "inbound", 100_000_000_000, "2025-03-01T00:00:00Z", wallet_id="wallet-c", fiat_rate=60_000),
        ]
        pairs = [
            {
                "pair_id": "mp-1",
                "kind": "swap",
                "policy": "carrying-value",
                "out_id": "m-btc-out",
                "in_id": "z-lbtc-in",
                "out_asset": "BTC",
                "in_asset": "LBTC",
            },
            {
                "pair_id": "mp-2",
                "kind": "swap",
                "policy": "carrying-value",
                "out_id": "a-lbtc-out",
                "in_id": "b-xyz-in",
                "out_asset": "LBTC",
                "in_asset": "XYZ",
            },
        ]
        regime_map, swap_map, carried_map, quarantined, quarantines = engine._annotate_at_cross_asset_pairs(pairs, rows, [])
        self.assertEqual(regime_map["m-btc-out"], REGIME_NEU)
        self.assertEqual(regime_map["a-lbtc-out"], REGIME_NEU)
        self.assertEqual(
            swap_map,
            {
                "m-btc-out": "mp-1",
                "z-lbtc-in": "mp-1",
                "a-lbtc-out": "mp-2",
                "b-xyz-in": "mp-2",
            },
        )
        self.assertEqual(
            carried_map,
            {
                "z-lbtc-in": Decimal("50000.0"),
                "b-xyz-in": Decimal("50000.0"),
            },
        )
        self.assertEqual(quarantined, set())
        self.assertEqual(quarantines, [])

    def test_missing_pool_average_quarantines_pair(self):
        profile = {"id": "p1", "workspace_id": "w1", "tax_country": "at"}
        engine = self.GenericRP2TaxEngine(profile)
        rows = [
            self._make_row("out-1", "BTC", "outbound", 50_000_000_000, "2025-06-01T00:00:00Z", fiat_rate=50_000),
            self._make_row("in-1", "LBTC", "inbound", 50_000_000_000, "2025-06-01T00:00:00Z", wallet_id="wallet-b", fiat_rate=50_000),
        ]
        pairs = [
            {
                "pair_id": "mp-1",
                "kind": "swap",
                "policy": "carrying-value",
                "out_id": "out-1",
                "in_id": "in-1",
                "out_asset": "BTC",
                "in_asset": "LBTC",
            }
        ]
        regime_map, swap_map, carried_map, quarantined, quarantines = engine._annotate_at_cross_asset_pairs(pairs, rows, [])
        self.assertEqual(regime_map, {"out-1": REGIME_NEU})
        self.assertEqual(swap_map, {})
        self.assertEqual(carried_map, {})
        self.assertEqual(quarantined, {"out-1", "in-1"})
        self.assertEqual(len(quarantines), 2)
        for quarantine in quarantines:
            detail = json.loads(quarantine["detail_json"])
            self.assertEqual(detail["reason_code"], "missing_pool_average")

    def test_missing_spot_price_quarantines_pair(self):
        profile = {"id": "p1", "workspace_id": "w1", "tax_country": "at"}
        engine = self.GenericRP2TaxEngine(profile)
        rows = [
            self._make_row("buy-1", "BTC", "inbound", 100_000_000_000, "2025-05-01T00:00:00Z"),
            self._make_row("out-1", "BTC", "outbound", 50_000_000_000, "2025-06-01T00:00:00Z", fiat_rate=0),
            self._make_row("in-1", "LBTC", "inbound", 50_000_000_000, "2025-06-01T00:00:00Z", wallet_id="wallet-b", fiat_rate=50_000),
        ]
        pairs = [
            {
                "pair_id": "mp-1",
                "kind": "swap",
                "policy": "carrying-value",
                "out_id": "out-1",
                "in_id": "in-1",
                "out_asset": "BTC",
                "in_asset": "LBTC",
            }
        ]
        regime_map, swap_map, carried_map, quarantined, quarantines = engine._annotate_at_cross_asset_pairs(pairs, rows, [])
        self.assertEqual(regime_map, {"out-1": REGIME_NEU})
        self.assertEqual(swap_map, {})
        self.assertEqual(carried_map, {})
        self.assertEqual(quarantined, {"out-1", "in-1"})
        self.assertEqual(len(quarantines), 2)
        for quarantine in quarantines:
            detail = json.loads(quarantine["detail_json"])
            self.assertEqual(detail["reason_code"], "missing_spot_price")


class ATCrossAssetValidationWiringTest(unittest.TestCase):
    """Pin that ``GenericRP2TaxEngine.build_ledger_state`` runs the country's cross-asset
    validator between the parse and compute phases — the new backstop that catches
    orphan ``at_swap_link`` markers Kassiber's annotator structurally cannot detect
    (a paired leg that was never imported can't be annotated).
    """

    def setUp(self):
        from kassiber.core.engines.base import TaxEngineLedgerInputs
        from kassiber.core.engines.rp2 import GenericRP2TaxEngine

        self.GenericRP2TaxEngine = GenericRP2TaxEngine
        self.TaxEngineLedgerInputs = TaxEngineLedgerInputs

    def _profile(self):
        return {
            "id": "p1",
            "workspace_id": "w1",
            "label": "holder1",
            "tax_country": "at",
            "gains_algorithm": "moving_average_at",
        }

    def _wallet_refs(self):
        return {
            "wallet-a": {
                "id": "wallet-a",
                "label": "wallet-a",
                "wallet_account_id": "acct-1",
                "account_code": "A",
                "account_label": "Account A",
            },
            "wallet-b": {
                "id": "wallet-b",
                "label": "wallet-b",
                "wallet_account_id": "acct-1",
                "account_code": "A",
                "account_label": "Account A",
            },
        }

    def _inbound_row(self, tx_id, wallet_id, asset, amount, occurred_at, *, fiat_rate=50_000):
        return {
            "id": tx_id,
            "wallet_id": wallet_id,
            "wallet_label": wallet_id,
            "asset": asset,
            "direction": "inbound",
            "amount": amount,
            "fee": 0,
            "fiat_rate": fiat_rate,
            "fiat_value": None,
            "kind": "deposit",
            "description": tx_id,
            "note": None,
            "external_id": tx_id,
            "occurred_at": occurred_at,
        }

    def _build_inputs(self):
        rows = [
            self._inbound_row("buy-btc", "wallet-a", "BTC", 100_000_000_000, "2025-05-01T00:00:00Z"),
            self._inbound_row("buy-eur", "wallet-b", "ETH", 1_000_000_000_000_000_000, "2025-05-02T00:00:00Z", fiat_rate=3_000),
        ]
        return self.TaxEngineLedgerInputs(
            rows=rows,
            wallet_refs_by_id=self._wallet_refs(),
            manual_pair_records=[],
        )

    def test_validator_is_called_once_with_every_non_empty_input_data(self):
        # Two assets with non-empty inventory → validator sees two InputData objects, once.
        engine = self.GenericRP2TaxEngine(self._profile())
        inputs = self._build_inputs()

        # Spy by patching the AT country's method at class level. build_tax_policy builds a
        # fresh `rp2.plugin.country.at.AT` on every call, so patching the class affects the
        # instance used by `_rp2_configuration`.
        from rp2.plugin.country.at import AT

        calls: list[list[object]] = []
        original = AT.validate_input_data

        def spy(self, input_data_list):
            calls.append(list(input_data_list))
            return original(self, input_data_list)

        AT.validate_input_data = spy  # type: ignore[assignment]
        try:
            engine.build_ledger_state(inputs)
        finally:
            AT.validate_input_data = original  # type: ignore[assignment]

        self.assertEqual(len(calls), 1, "validator must be called exactly once per build_ledger_state")
        self.assertEqual(len(calls[0]), 2, "validator must receive one InputData per non-empty asset")
        seen_assets = {getattr(input_data, "asset", None) for input_data in calls[0]}
        self.assertEqual(seen_assets, {"BTC", "ETH"})

    def test_validator_failure_surfaces_as_apperror_with_code(self):
        from kassiber.errors import AppError
        from rp2.plugin.country.at import AT
        from rp2.rp2_error import RP2ValueError

        engine = self.GenericRP2TaxEngine(self._profile())
        inputs = self._build_inputs()

        def failing(self, input_data_list):
            raise RP2ValueError("Unpaired `at_swap_link=orphan` marker")

        original = AT.validate_input_data
        AT.validate_input_data = failing  # type: ignore[assignment]
        try:
            with self.assertRaises(AppError) as ctx:
                engine.build_ledger_state(inputs)
        finally:
            AT.validate_input_data = original  # type: ignore[assignment]

        self.assertEqual(ctx.exception.code, "rp2_input_validation")
        self.assertIn("at_swap_link=orphan", str(ctx.exception))

    def test_missing_validator_surfaces_as_unsupported_apperror(self):
        # Protects developers who updated pyproject.toml but haven't re-synced: `AT` from a
        # stale rp2 pin has no `validate_input_data`. The compat guard must raise a clear
        # upgrade hint, not a generic `rp2_input_validation` wrapped AttributeError.
        from kassiber.errors import AppError
        from rp2.abstract_country import AbstractCountry
        from rp2.plugin.country.at import AT

        engine = self.GenericRP2TaxEngine(self._profile())
        inputs = self._build_inputs()

        original_at = AT.__dict__.get("validate_input_data")
        original_abstract = AbstractCountry.__dict__.get("validate_input_data")
        if original_at is not None:
            delattr(AT, "validate_input_data")
        if original_abstract is not None:
            delattr(AbstractCountry, "validate_input_data")
        try:
            with self.assertRaises(AppError) as ctx:
                engine.build_ledger_state(inputs)
        finally:
            if original_abstract is not None:
                AbstractCountry.validate_input_data = original_abstract  # type: ignore[assignment]
            if original_at is not None:
                AT.validate_input_data = original_at  # type: ignore[assignment]

        self.assertEqual(ctx.exception.code, "unsupported")
        self.assertIn("PR #4", str(ctx.exception))


if __name__ == "__main__":
    unittest.main()
