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
    def test_pool_is_global_regardless_of_wallet(self):
        # Single global moving-average pool per asset (§ 2 KryptowährungsVO): the pool id no longer
        # depends on the wallet, so coins acquired in one wallet and sold from another share one pool.
        self.assertEqual(resolve_pool_id("wallet-abc"), "default")
        self.assertEqual(resolve_pool_id("wallet-xyz"), "default")
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

    def test_post_cutoff_sale_from_alt_only_wallet_stays_alt(self):
        rows = [
            _row("buy-alt", "wallet-a", "inbound", 50_000_000, occurred_at="2020-06-01T00:00:00Z"),
            _row("buy-neu", "wallet-b", "inbound", 100_000_000, occurred_at="2024-06-01T00:00:00Z"),
            _row("sell-from-a", "wallet-a", "outbound", 30_000_000, occurred_at="2025-06-01T00:00:00Z"),
        ]
        self.assertEqual(infer_outbound_regimes(rows), {"sell-from-a": REGIME_ALT})

    def test_post_cutoff_sale_from_transfer_funded_wallet_draws_moved_neu_inventory(self):
        # bitcoinaustria/kassiber#213: Neu acquired in wallet-a, moved to wallet-b, then sold from
        # wallet-b. The rp2 cost-basis pool is global, but regime availability still follows wallets
        # and explicit internal transfers.
        rows = [
            _row("buy-alt", "wallet-a", "inbound", 50_000_000, occurred_at="2020-06-01T00:00:00Z"),
            _row("buy-neu", "wallet-a", "inbound", 100_000_000, occurred_at="2024-06-01T00:00:00Z"),
            _row("xfer-out", "wallet-a", "outbound", 100_000_000, occurred_at="2024-07-01T00:00:00Z", external_id="xfer-1"),
            _row("xfer-in", "wallet-b", "inbound", 100_000_000, occurred_at="2024-07-01T00:00:00Z", external_id="xfer-1"),
            _row("sell-from-b", "wallet-b", "outbound", 30_000_000, occurred_at="2025-06-01T00:00:00Z"),
        ]
        intra_pairs = [{"out": rows[2], "in": rows[3]}]
        self.assertEqual(infer_outbound_regimes(rows, intra_pairs), {"sell-from-b": REGIME_NEU})


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
        self.assertEqual(event.at_pool, "default")  # single global per-asset pool, not per-wallet
        self.assertIsNone(event.at_swap_link)

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

    def test_at_swap_link_tags_inbound_without_basis_override(self):
        rows = [_row("buy-1", "wallet-a", "inbound", 1, occurred_at="2025-06-01T00:00:00Z")]
        result = normalize_tax_asset_inputs(
            self.at_profile,
            "BTC",
            rows,
            self.wallet_refs,
            [],
            at_swap_link_by_row_id={"buy-1": "swap-42"},
        )
        event = result.events[0]
        self.assertEqual(event.at_swap_link, "swap-42")
        self.assertEqual(event.fiat_value, event.amount * event.spot_price)

    def test_at_transfer_uses_global_pool(self):
        rows = [
            _row("out-1", "wallet-a", "outbound", 1, occurred_at="2025-06-01T00:00:00Z", external_id="chain-1"),
            _row("in-1", "wallet-b", "inbound", 1, occurred_at="2025-06-01T00:00:00Z", external_id="chain-1"),
        ]
        pair = {"out": rows[0], "in": rows[1]}
        result = normalize_tax_asset_inputs(
            self.at_profile, "BTC", rows, self.wallet_refs, [pair]
        )
        self.assertEqual(len(result.transfers), 1)
        # Transfers carry the single global pool, not the source wallet — so a later sale from the
        # destination wallet resolves against the same Neu pool the coins were acquired in.
        self.assertEqual(result.transfers[0].at_pool, "default")

    def test_at_outbound_post_cutoff_with_only_alt_inventory_gets_alt_regime(self):
        rows = [
            _row("buy-alt", "wallet-a", "inbound", 100_000_000, occurred_at="2020-06-01T00:00:00Z"),
            _row("sell-later", "wallet-a", "outbound", 100_000_000, occurred_at="2025-06-01T00:00:00Z"),
        ]
        result = normalize_tax_asset_inputs(
            self.at_profile, "BTC", rows, self.wallet_refs, []
        )
        self.assertEqual(result.events[1].at_regime, REGIME_ALT)


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


class ATSwapOverSellQuarantineTest(unittest.TestCase):
    """An Austrian carrying-value swap whose disposed leg has insufficient
    quantity must be quarantined as a PAIR before it can be promoted to an
    at_swap_link.

    Regression for the report-abort path: marking such a leg lets it bypass the
    single-asset quantity gate on the second prepare pass and reach compute_tax,
    where rp2's per-account BalanceSet raises an uncatchable "balance went
    negative" that aborts the whole multi-asset report instead of quarantining
    the one offending pair. (Real-world trigger: a self-custody round-trip — e.g.
    BTC funding a friend's multisig that later returns — that the user paired as
    a BTC↔L-BTC carrying-value swap.)
    """

    def _profile(self):
        return {
            "id": "p1",
            "workspace_id": "w1",
            "label": "BA",
            "tax_country": "at",
            "gains_algorithm": "moving_average_at",
        }

    def _wallet_refs(self):
        return {
            "onchain": {
                "id": "onchain",
                "label": "onchain",
                "wallet_account_id": "acct-1",
                "account_code": "A",
                "account_label": "Account A",
            },
            "liquid": {
                "id": "liquid",
                "label": "liquid",
                "wallet_account_id": "acct-1",
                "account_code": "A",
                "account_label": "Account A",
            },
        }

    def _row(self, tx_id, wallet_id, direction, asset, amount_msat, occurred_at):
        return {
            "id": tx_id,
            "wallet_id": wallet_id,
            "wallet_label": wallet_id,
            "asset": asset,
            "direction": direction,
            "amount": amount_msat,
            "fee": 0,
            "fiat_rate": 50_000,
            "fiat_value": None,
            "kind": "deposit" if direction == "inbound" else "withdrawal",
            "description": tx_id,
            "note": None,
            "external_id": tx_id,
            "occurred_at": occurred_at,
        }

    def test_oversold_swap_leg_quarantines_pair_without_marking(self):
        from kassiber.core.austrian import AT_SWAP_QUARANTINE_REASON
        from kassiber.core.engines.rp2 import (
            _prepare_assets,
            _rp2_configuration,
            _select_at_cross_asset_swap_links,
        )

        profile = self._profile()
        wallet_refs = self._wallet_refs()
        # The BTC account holds only 0.0001 BTC but the swap disposes 0.0005 BTC —
        # a local over-sell. The L-BTC acquisition leg is otherwise valid.
        btc_in = self._row("btc-in", "onchain", "inbound", "BTC", 10_000_000, "2025-05-01T00:00:00Z")
        btc_out = self._row("swap-out", "onchain", "outbound", "BTC", 50_000_000, "2025-06-01T00:00:00Z")
        lbtc_in = self._row("swap-in", "liquid", "inbound", "L-BTC", 50_000_000, "2025-06-01T00:00:00Z")
        rows = [btc_in, btc_out, lbtc_in]
        rows_by_asset = {"BTC": [btc_in, btc_out], "L-BTC": [lbtc_in]}

        with _rp2_configuration(profile, ["onchain", "liquid"], ["BTC", "L-BTC"]) as configuration:
            prepared_by_asset = _prepare_assets(profile, rows_by_asset, wallet_refs, {}, configuration)
            # Phase 1 (no swap links yet) must quarantine the over-sold BTC leg.
            phase1_reasons = {
                str(q["transaction_id"]): q["reason"]
                for _, prepared in prepared_by_asset
                for q in prepared.quarantines
            }
            self.assertEqual(phase1_reasons.get("swap-out"), "insufficient_lots")

            pair = {
                "out_id": "swap-out",
                "in_id": "swap-in",
                "out_asset": "BTC",
                "in_asset": "L-BTC",
                "policy": "carrying-value",
                "pair_id": "swap-1",
            }
            swap_link_by_row_id, quarantined_row_ids, swap_quarantines = _select_at_cross_asset_swap_links(
                profile, [pair], rows, prepared_by_asset
            )

        # The over-sold pair is NOT promoted to an at_swap_link (which would
        # bypass the quantity gate and abort the whole report)...
        self.assertNotIn("swap-out", swap_link_by_row_id)
        self.assertNotIn("swap-in", swap_link_by_row_id)
        # ...both legs are excluded from the compute pass (so the surviving leg
        # cannot orphan the cross-asset validator)...
        self.assertEqual(quarantined_row_ids, {"swap-out", "swap-in"})
        # ...and the pair surfaces for review under the swap-carry reason, with
        # the original phase-1 cause preserved in the detail.
        self.assertEqual(len(swap_quarantines), 2)
        self.assertEqual(
            {str(q["transaction_id"]) for q in swap_quarantines},
            {"swap-out", "swap-in"},
        )
        for quarantine in swap_quarantines:
            self.assertEqual(quarantine["reason"], AT_SWAP_QUARANTINE_REASON)
            self.assertEqual(json.loads(quarantine["detail_json"])["reason_code"], "insufficient_lots")


if __name__ == "__main__":
    unittest.main()
