import json
import tempfile
import unittest

from kassiber.cli import handlers
from kassiber.core.custody_components import activate_component, create_component
from kassiber.core.custody_journal import project_effective_components
from kassiber.core.engines import TaxEngineLedgerInputs, build_tax_engine
from kassiber.core.tax_events import normalize_tax_asset_inputs
from kassiber.transfers import apply_manual_pairs
from kassiber.db import open_db


PROFILE = {
    "id": "profile-1",
    "workspace_id": "workspace-1",
    "label": "Custody test",
    "tax_country": "generic",
    "fiat_currency": "EUR",
    "tax_long_term_days": 365,
    "gains_algorithm": "FIFO",
    "bitcoin_rail_carrying_value": 1,
}
WALLETS = {
    "wallet-a": {
        "id": "wallet-a",
        "label": "A",
        "kind": "descriptor",
        "wallet_account_id": "account-a",
        "account_code": "a",
        "account_label": "A",
    },
    "wallet-b": {
        "id": "wallet-b",
        "label": "B",
        "kind": "descriptor",
        "wallet_account_id": "account-b",
        "account_code": "b",
        "account_label": "B",
    },
    "wallet-c": {
        "id": "wallet-c",
        "label": "C",
        "kind": "descriptor",
        "wallet_account_id": "account-c",
        "account_code": "c",
        "account_label": "C",
    },
    "wallet-gap": {
        "id": "wallet-gap",
        "label": "Missing historical wallet",
        "kind": "untracked",
        "wallet_account_id": "account-gap",
        "account_code": "gap",
        "account_label": "Gap",
    },
    "wallet-gap-2": {
        "id": "wallet-gap-2",
        "label": "Second missing historical wallet",
        "kind": "untracked",
        "wallet_account_id": "account-gap-2",
        "account_code": "gap-2",
        "account_label": "Gap 2",
    },
    "wallet-d": {
        "id": "wallet-d",
        "label": "D",
        "kind": "descriptor",
        "wallet_account_id": "account-d",
        "account_code": "d",
        "account_label": "D",
    },
    "wallet-e": {
        "id": "wallet-e",
        "label": "E",
        "kind": "descriptor",
        "wallet_account_id": "account-e",
        "account_code": "e",
        "account_label": "E",
    },
}


def row(
    transaction_id,
    wallet_id,
    direction,
    amount,
    *,
    asset="BTC",
    fee=0,
    rate="50000",
    occurred_at="2024-01-01T00:00:00Z",
):
    wallet = WALLETS[wallet_id]
    return {
        "id": transaction_id,
        "workspace_id": PROFILE["workspace_id"],
        "profile_id": PROFILE["id"],
        "wallet_id": wallet_id,
        "wallet_label": wallet["label"],
        "wallet_kind": wallet["kind"],
        "wallet_account_id": wallet["wallet_account_id"],
        "account_code": wallet["account_code"],
        "account_label": wallet["account_label"],
        "external_id": transaction_id,
        "fingerprint": transaction_id,
        "occurred_at": occurred_at,
        "confirmed_at": occurred_at,
        "direction": direction,
        "asset": asset,
        "amount": amount,
        "fee": fee,
        "amount_includes_fee": 0,
        "fiat_currency": "EUR",
        "fiat_rate": None if rate is None else float(rate),
        "fiat_rate_exact": rate,
        "fiat_value": None,
        "fiat_value_exact": None,
        "pricing_quality": "exact",
        "review_status": None,
        "kind": None,
        "description": None,
        "note": None,
        "counterparty": None,
        "excluded": 0,
        "raw_json": "{}",
        "payment_hash": None,
        "payment_hash_source": None,
        "swap_refund_funding_txid": None,
        "privacy_boundary": None,
        "at_regime_override": None,
        "created_at": occurred_at,
    }


def leg(
    leg_id,
    role,
    amount,
    wallet_id=None,
    transaction_id=None,
    *,
    asset="BTC",
    rail="bitcoin",
    occurred_at="2024-01-01T00:00:00Z",
):
    return {
        "id": leg_id,
        "role": role,
        "amount_msat": amount,
        "wallet_id": wallet_id,
        "transaction_id": transaction_id,
        "asset": asset,
        "rail": rail,
        "exposure": "bitcoin",
        "conservation_unit": "msat",
        "occurred_at": occurred_at,
    }


def allocation(index, source, sink, source_amount, sink_amount=None):
    return {
        "id": f"allocation-{index}",
        "ordinal": index,
        "source_leg_id": source,
        "sink_leg_id": sink,
        "source_amount_msat": source_amount,
        "sink_amount_msat": source_amount if sink_amount is None else sink_amount,
    }


def component(legs, allocations, **overrides):
    value = {
        "id": "component-1",
        "effective_state": "active",
        "component_type": "native_transfer",
        "conservation_mode": "quantity",
        "evidence_kind": "transaction_graph",
        "evidence_grade": "exact",
        "conversion_policy": None,
        "notes": None,
        "activated_at": "2024-01-02T00:00:00Z",
        "created_at": "2024-01-02T00:00:00Z",
        "legs": legs,
        "allocations": allocations,
    }
    value.update(overrides)
    return value


class CustodyJournalProjectionTests(unittest.TestCase):
    def test_one_to_many_replaces_anchors_and_allocates_fee_once(self):
        rows = [
            row("out", "wallet-a", "outbound", 90, fee=10),
            row("in-b", "wallet-b", "inbound", 60),
            row("in-c", "wallet-c", "inbound", 30),
        ]
        legs = [
            leg("source", "source", 100, "wallet-a", "out"),
            leg("dest-b", "destination", 60, "wallet-b", "in-b"),
            leg("dest-c", "destination", 30, "wallet-c", "in-c"),
            leg("fee", "fee", 10),
        ]
        allocations = [
            allocation(0, "source", "dest-b", 60),
            allocation(1, "source", "dest-c", 30),
            allocation(2, "source", "fee", 10),
        ]

        projection = project_effective_components(
            PROFILE, rows, WALLETS, (), (component(legs, allocations),)
        )

        self.assertFalse(projection.blockers)
        self.assertEqual({item["id"] for item in projection.rows}.isdisjoint({"out", "in-b", "in-c"}), True)
        self.assertEqual(len(projection.rows), 4)
        self.assertEqual(len(projection.manual_pair_records), 2)
        self.assertEqual(
            sum(int(item["fee"]) for item in projection.rows if item["direction"] == "outbound"),
            10,
        )
        self.assertEqual(
            {item["journal_transaction_id"] for item in projection.rows},
            {"out", "in-b", "in-c"},
        )
        self.assertTrue(
            all(item["component_id"] == "component-1" for item in projection.manual_pair_records)
        )

    def test_transactionless_untracked_wallet_bridges_two_hops(self):
        rows = [
            row("out", "wallet-a", "outbound", 100),
            row("in", "wallet-b", "inbound", 100, occurred_at="2024-03-01T00:00:00Z"),
        ]
        legs = [
            leg("source-a", "source", 100, "wallet-a", "out"),
            leg("gap-in", "retained", 100, "wallet-gap", occurred_at="2024-02-01T00:00:00Z"),
            leg("gap-out", "source", 100, "wallet-gap", occurred_at="2024-02-28T00:00:00Z"),
            leg("dest-b", "destination", 100, "wallet-b", "in", occurred_at="2024-03-01T00:00:00Z"),
        ]
        allocations = [
            allocation(0, "source-a", "gap-in", 100),
            allocation(1, "gap-out", "dest-b", 100),
        ]

        projection = project_effective_components(
            PROFILE, rows, WALLETS, (), (component(legs, allocations, component_type="manual_bridge"),)
        )

        self.assertFalse(projection.blockers)
        self.assertEqual(len(projection.manual_pair_records), 2)
        gap_rows = [item for item in projection.rows if item["wallet_id"] == "wallet-gap"]
        self.assertEqual({item["direction"] for item in gap_rows}, {"inbound", "outbound"})
        self.assertTrue(all(item["journal_transaction_id"] in {"out", "in"} for item in gap_rows))

    def test_external_allocation_remains_taxable_outbound(self):
        rows = [
            row("out", "wallet-a", "outbound", 90, fee=10),
            row("in", "wallet-b", "inbound", 70),
        ]
        legs = [
            leg("source", "source", 100, "wallet-a", "out"),
            leg("dest", "destination", 70, "wallet-b", "in"),
            leg("external", "external", 20),
            leg("fee", "fee", 10),
        ]
        allocations = [
            allocation(0, "source", "dest", 70),
            allocation(1, "source", "external", 20),
            allocation(2, "source", "fee", 10),
        ]

        projection = project_effective_components(
            PROFILE, rows, WALLETS, (), (component(legs, allocations),)
        )

        external_rows = [item for item in projection.rows if item["kind"] == "custody_component_external"]
        self.assertEqual(len(external_rows), 1)
        self.assertEqual(external_rows[0]["direction"], "outbound")
        self.assertEqual(external_rows[0]["amount"], 20)
        self.assertEqual(len(projection.manual_pair_records), 1)
        self.assertEqual(sum(int(item["fee"]) for item in projection.rows), 10)

    def test_same_wallet_refund_component_carries_principal_and_books_only_fee(self):
        rows = [
            row("lockup", "wallet-a", "outbound", 100, rate="50000"),
            row(
                "refund",
                "wallet-a",
                "inbound",
                99,
                rate="50000",
                occurred_at="2024-01-02T00:00:00Z",
            ),
        ]
        legs = [
            leg("source", "source", 100, "wallet-a", "lockup"),
            leg(
                "dest",
                "destination",
                99,
                "wallet-a",
                "refund",
                occurred_at="2024-01-02T00:00:00Z",
            ),
            leg("fee", "fee", 1, asset="BTC", rail="bitcoin"),
        ]
        reviewed = component(
            legs,
            [
                allocation(0, "source", "dest", 99),
                allocation(1, "source", "fee", 1),
            ],
            component_type="refund",
        )

        projection = project_effective_components(
            PROFILE, rows, WALLETS, (), (reviewed,)
        )
        pairs, cross_asset = apply_manual_pairs(
            projection.rows, (), projection.manual_pair_records
        )
        normalized = normalize_tax_asset_inputs(
            PROFILE, "BTC", projection.rows, WALLETS, pairs
        )

        self.assertFalse(projection.blockers)
        self.assertFalse(cross_asset)
        self.assertFalse(normalized.quarantines)
        self.assertEqual(len(normalized.transfers), 1)
        transfer = normalized.transfers[0]
        self.assertEqual(transfer.from_wallet_id, "wallet-a")
        self.assertEqual(transfer.to_wallet_id, "wallet-a")
        self.assertEqual(float(transfer.sent), 0.000000001)
        self.assertEqual(float(transfer.received), 0.00000000099)
        self.assertEqual(float(transfer.fee), 0.00000000001)

    def test_anchor_coverage_mismatch_blocks_even_when_manually_reviewed(self):
        rows = [
            row("out", "wallet-a", "outbound", 100),
            row("in", "wallet-b", "inbound", 90),
        ]
        legs = [
            leg("source", "source", 90, "wallet-a", "out"),
            leg("dest", "destination", 90, "wallet-b", "in"),
        ]
        allocations = [allocation(0, "source", "dest", 90)]
        blocked = project_effective_components(
            PROFILE, rows, WALLETS, (), (component(legs, allocations),)
        )
        self.assertEqual(blocked.blockers[0].code, "custody_component_anchor_coverage_mismatch")
        self.assertFalse(blocked.rows)
        self.assertEqual(
            {"out", "in"},
            {item["id"] for item in blocked.blocked_anchor_rows},
        )
        self.assertTrue(
            all(
                item["custody_component_force_block"]
                == "custody_component_anchor_coverage_mismatch"
                for item in blocked.blocked_anchor_rows
            )
        )
        self.assertEqual({item["transaction_id"] for item in blocked.quarantines}, {"out", "in"})

        reviewed = component(
            legs,
            allocations,
            component_type="manual_bridge",
            evidence_grade="reviewed",
            evidence_kind="manual_reconstruction",
        )
        reviewed_blocked = project_effective_components(
            PROFILE, rows, WALLETS, (), (reviewed,)
        )
        self.assertEqual(
            reviewed_blocked.blockers[0].code,
            "custody_component_anchor_coverage_mismatch",
        )
        self.assertFalse(reviewed_blocked.manual_pair_records)

    def test_authored_active_invalid_component_claims_known_anchors_fail_closed(self):
        rows = [
            row("out", "wallet-a", "outbound", 100),
            row("in", "wallet-b", "inbound", 90),
        ]
        invalid = component(
            [
                leg("source", "source", 100, "wallet-a", "out"),
                leg("dest", "destination", 90, "wallet-b", "in"),
            ],
            [allocation(0, "source", "dest", 90)],
            state="active",
            effective_state="draft",
            validation={
                "activatable": False,
                "issues": [
                    {
                        "code": "unbalanced_quantity",
                        "asset": "BTC",
                        "residual_msat": 10,
                    }
                ],
            },
        )
        raw_pair = {
            "id": "raw-pair",
            "out_transaction_id": "out",
            "in_transaction_id": "in",
        }

        projection = project_effective_components(
            PROFILE, rows, WALLETS, (raw_pair,), (invalid,)
        )

        self.assertFalse(projection.rows)
        self.assertEqual(
            {"out", "in"},
            {item["id"] for item in projection.blocked_anchor_rows},
        )
        self.assertFalse(projection.manual_pair_records)
        self.assertEqual(frozenset({"out", "in"}), projection.claimed_transaction_ids)
        self.assertEqual(1, len(projection.blockers))
        blocker = projection.blockers[0]
        self.assertEqual(
            "custody_component_authored_active_invalid", blocker.code
        )
        self.assertEqual(
            "unbalanced_quantity",
            blocker.details["validation_issues"][0]["code"],
        )
        self.assertIn("supersede", blocker.details["resolution"])
        self.assertEqual(
            {"out", "in"},
            {item["transaction_id"] for item in projection.quarantines},
        )
        detail = json.loads(projection.quarantines[0]["detail_json"])
        self.assertEqual(
            "custody_component_authored_active_invalid",
            detail["blocker_code"],
        )

    def test_conflicting_authored_active_components_block_all_of_both_graphs(self):
        rows = [
            row("out", "wallet-a", "outbound", 100),
            row("in-b", "wallet-b", "inbound", 100),
            row("in-c", "wallet-c", "inbound", 100),
        ]
        validation = {
            "activatable": False,
            "issues": [{"code": "active_transaction_membership_conflict"}],
        }
        left = component(
            [
                leg("left-source", "source", 100, "wallet-a", "out"),
                leg("left-dest", "destination", 100, "wallet-b", "in-b"),
            ],
            [allocation(0, "left-source", "left-dest", 100)],
            id="component-left",
            state="active",
            effective_state="draft",
            validation=validation,
        )
        right = component(
            [
                leg("right-source", "source", 100, "wallet-a", "out"),
                leg("right-dest", "destination", 100, "wallet-c", "in-c"),
            ],
            [allocation(0, "right-source", "right-dest", 100)],
            id="component-right",
            state="active",
            effective_state="draft",
            validation=validation,
        )

        projection = project_effective_components(
            PROFILE, rows, WALLETS, (), (left, right)
        )

        self.assertFalse(projection.rows)
        self.assertEqual(
            {"out", "in-b", "in-c"},
            {item["id"] for item in projection.blocked_anchor_rows},
        )
        self.assertEqual(
            frozenset({"out", "in-b", "in-c"}),
            projection.claimed_transaction_ids,
        )
        self.assertEqual(2, len(projection.blockers))
        self.assertEqual(
            {"custody_component_membership_conflict"},
            {blocker.code for blocker in projection.blockers},
        )
        self.assertEqual(
            {"out", "in-b", "in-c"},
            {item["transaction_id"] for item in projection.quarantines},
        )

    def test_profile_policy_controls_btc_lbtc_carrying_value(self):
        rows = [
            row("out", "wallet-a", "outbound", 100, asset="BTC"),
            row("in", "wallet-b", "inbound", 100, asset="LBTC"),
        ]
        legs = [
            leg("source", "source", 100, "wallet-a", "out", asset="BTC", rail="bitcoin"),
            leg("dest", "destination", 100, "wallet-b", "in", asset="LBTC", rail="liquid"),
        ]
        allocations = [allocation(0, "source", "dest", 100)]
        enabled = project_effective_components(
            PROFILE, rows, WALLETS, (), (component(legs, allocations, component_type="peg"),)
        )
        self.assertEqual(enabled.manual_pair_records[0]["policy"], "carrying-value")

        disabled_profile = {**PROFILE, "bitcoin_rail_carrying_value": 0}
        disabled = project_effective_components(
            disabled_profile, rows, WALLETS, (), (component(legs, allocations, component_type="peg"),)
        )
        self.assertEqual(disabled.manual_pair_records[0]["policy"], "taxable")

    def test_unambiguous_conversion_without_explicit_edge_preserves_both_quantities(self):
        rows = [
            row("out", "wallet-a", "outbound", 100, asset="BTC"),
            row("in", "wallet-b", "inbound", 250, asset="USDT"),
        ]
        legs = [
            {
                **leg(
                    "source",
                    "source",
                    100,
                    "wallet-a",
                    "out",
                    asset="BTC",
                    rail="bitcoin",
                ),
                "valuation_unit": "eur-cent",
                "valuation_amount": 1_000,
            },
            {
                **leg(
                    "dest",
                    "destination",
                    250,
                    "wallet-b",
                    "in",
                    asset="USDT",
                    rail="liquid",
                ),
                "exposure": "usdt",
                "valuation_unit": "eur-cent",
                "valuation_amount": 1_000,
            },
        ]
        converted = component(
            legs,
            [],
            component_type="swap",
            conservation_mode="conversion",
            conversion_policy="taxable",
            conversion_reviewed=True,
        )

        projection = project_effective_components(
            PROFILE, rows, WALLETS, (), (converted,)
        )

        self.assertFalse(projection.blockers)
        outbound = next(
            item for item in projection.rows if item["direction"] == "outbound"
        )
        inbound = next(
            item for item in projection.rows if item["direction"] == "inbound"
        )
        self.assertEqual(outbound["amount"], 100)
        self.assertEqual(inbound["amount"], 250)
        self.assertEqual(outbound["fiat_value_exact"], "10")
        self.assertEqual(inbound["fiat_value_exact"], "10")
        self.assertEqual(projection.manual_pair_records[0]["policy"], "taxable")

    def test_conversion_fee_projection_uses_exact_authored_source_loss(self):
        rows = [
            row("out", "wallet-a", "outbound", 100, asset="BTC"),
            row("in", "wallet-b", "inbound", 250, asset="USDT"),
        ]
        legs = [
            {
                **leg("source", "source", 100, "wallet-a", "out", asset="BTC"),
                "valuation_unit": "eur-cent",
                "valuation_amount": 1_000,
            },
            {
                **leg(
                    "dest",
                    "destination",
                    250,
                    "wallet-b",
                    "in",
                    asset="USDT",
                    rail="liquid",
                ),
                "exposure": "tether-usd",
                "conservation_unit": "asset-quantum",
                "valuation_unit": "eur-cent",
                "valuation_amount": 900,
            },
            {
                **leg("fee", "fee", 10, asset="BTC"),
                "valuation_unit": "eur-cent",
                "valuation_amount": 100,
            },
        ]
        allocations = [
            allocation(0, "source", "dest", 90, 250),
            allocation(1, "source", "fee", 10, 10),
        ]
        reviewed = component(
            legs,
            allocations,
            component_type="swap",
            conservation_mode="conversion",
            conversion_policy="taxable",
            conversion_reviewed=True,
        )

        projection = project_effective_components(
            PROFILE, rows, WALLETS, (), (reviewed,)
        )

        self.assertFalse(projection.blockers)
        outbound = next(
            item for item in projection.rows if item["direction"] == "outbound"
        )
        self.assertEqual(90, outbound["amount"])
        self.assertEqual(10, outbound["fee"])
        self.assertEqual("9", outbound["fiat_value_exact"])

        mismatched = component(
            [*legs[:-1], {**legs[-1], "amount_msat": 5}],
            [
                allocation(0, "source", "dest", 90, 250),
                allocation(1, "source", "fee", 10, 5),
            ],
            component_type="swap",
            conservation_mode="conversion",
            conversion_policy="taxable",
            conversion_reviewed=True,
        )
        blocked = project_effective_components(
            PROFILE, rows, WALLETS, (), (mismatched,)
        )
        self.assertEqual(
            "conversion_fee_quantity_mismatch", blocked.blockers[0].code
        )

    def test_normalizer_blocks_every_component_member_when_one_pair_fails(self):
        rows = [
            row("out", "wallet-a", "outbound", 90, fee=10, rate=None),
            row("in-b", "wallet-b", "inbound", 60, rate=None),
            row("in-c", "wallet-c", "inbound", 30, rate=None),
        ]
        legs = [
            leg("source", "source", 100, "wallet-a", "out"),
            leg("dest-b", "destination", 60, "wallet-b", "in-b"),
            leg("dest-c", "destination", 30, "wallet-c", "in-c"),
            leg("fee", "fee", 10),
        ]
        allocations = [
            allocation(0, "source", "dest-b", 60),
            allocation(1, "source", "dest-c", 30),
            allocation(2, "source", "fee", 10),
        ]
        projection = project_effective_components(
            PROFILE, rows, WALLETS, (), (component(legs, allocations),)
        )
        pairs, cross = apply_manual_pairs(
            projection.rows, (), projection.manual_pair_records
        )
        self.assertFalse(cross)

        normalized = normalize_tax_asset_inputs(
            PROFILE,
            "BTC",
            projection.rows,
            WALLETS,
            pairs,
        )

        self.assertFalse(normalized.events)
        self.assertFalse(normalized.transfers)
        self.assertEqual(
            {item["transaction_id"] for item in normalized.quarantines},
            {"out", "in-b", "in-c"},
        )
        details = [json.loads(item["detail_json"]) for item in normalized.quarantines]
        self.assertTrue(any(detail.get("component_id") == "component-1" for detail in details))

    def test_engine_carries_basis_through_transactionless_wallet(self):
        acquisition = row(
            "buy",
            "wallet-a",
            "inbound",
            100_000_000_000,
            occurred_at="2023-01-01T00:00:00Z",
        )
        source = row(
            "out",
            "wallet-a",
            "outbound",
            100_000_000_000,
            occurred_at="2024-01-01T00:00:00Z",
        )
        destination = row(
            "in",
            "wallet-b",
            "inbound",
            100_000_000_000,
            occurred_at="2024-03-01T00:00:00Z",
        )
        legs = [
            leg(
                "source-a",
                "source",
                100_000_000_000,
                "wallet-a",
                "out",
                occurred_at="2024-01-01T00:00:00Z",
            ),
            leg(
                "gap-in",
                "retained",
                100_000_000_000,
                "wallet-gap",
                occurred_at="2024-02-01T00:00:00Z",
            ),
            leg(
                "gap-out",
                "source",
                100_000_000_000,
                "wallet-gap",
                occurred_at="2024-02-28T00:00:00Z",
            ),
            leg(
                "dest-b",
                "destination",
                100_000_000_000,
                "wallet-b",
                "in",
                occurred_at="2024-03-01T00:00:00Z",
            ),
        ]
        allocations = [
            allocation(0, "source-a", "gap-in", 100_000_000_000),
            allocation(1, "gap-out", "dest-b", 100_000_000_000),
        ]
        reviewed = component(
            legs,
            allocations,
            component_type="manual_bridge",
            evidence_grade="reviewed",
            evidence_kind="manual_reconstruction",
        )

        result = build_tax_engine(PROFILE).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=[acquisition, source, destination],
                wallet_refs_by_id=WALLETS,
                manual_pair_records=(),
                authored_active_custody_components=(reviewed,),
            )
        )

        self.assertFalse(result.quarantines)
        self.assertEqual(
            [entry["entry_type"] for entry in result.entries].count("transfer_out"),
            2,
        )
        self.assertEqual(
            [entry["entry_type"] for entry in result.entries].count("transfer_in"),
            2,
        )
        holdings = {
            wallet_label: totals["quantity"]
            for (_wallet_id, wallet_label, _account, asset), totals in result.wallet_holdings.items()
            if asset == "BTC"
        }
        self.assertEqual(float(holdings.get("B", 0)), 1.0)
        self.assertEqual(float(holdings.get("Missing historical wallet", 0)), 0.0)

    def test_projects_consolidation_and_three_migrations_chronologically(self):
        """Exercise N:1, missing-wallet hops, then 1:N as one custody history."""

        whole = 100_000_000_000
        part_a = 60_000_000_000
        part_b = 40_000_000_000
        half = 50_000_000_000
        rows = [
            row(
                "buy-a",
                "wallet-a",
                "inbound",
                part_a,
                rate="10000",
                occurred_at="2020-01-01T00:00:00Z",
            ),
            row(
                "buy-b",
                "wallet-b",
                "inbound",
                part_b,
                rate="20000",
                occurred_at="2020-02-01T00:00:00Z",
            ),
            row(
                "out-a",
                "wallet-a",
                "outbound",
                part_a,
                occurred_at="2021-01-01T00:00:00Z",
            ),
            row(
                "out-b",
                "wallet-b",
                "outbound",
                part_b,
                occurred_at="2021-01-01T00:00:00Z",
            ),
            row(
                "in-c",
                "wallet-c",
                "inbound",
                whole,
                occurred_at="2021-01-01T00:10:00Z",
            ),
            row(
                "out-c",
                "wallet-c",
                "outbound",
                whole,
                occurred_at="2022-01-01T00:00:00Z",
            ),
            row(
                "in-d",
                "wallet-d",
                "inbound",
                half,
                occurred_at="2024-01-01T00:00:00Z",
            ),
            row(
                "in-e",
                "wallet-e",
                "inbound",
                half,
                occurred_at="2024-01-01T00:00:00Z",
            ),
        ]
        consolidation = component(
            [
                leg(
                    "source-a", "source", part_a, "wallet-a", "out-a",
                    occurred_at="2021-01-01T00:00:00Z",
                ),
                leg(
                    "source-b", "source", part_b, "wallet-b", "out-b",
                    occurred_at="2021-01-01T00:00:00Z",
                ),
                leg(
                    "dest-c", "destination", whole, "wallet-c", "in-c",
                    occurred_at="2021-01-01T00:10:00Z",
                ),
            ],
            [
                allocation(0, "source-a", "dest-c", part_a),
                allocation(1, "source-b", "dest-c", part_b),
            ],
            id="component-consolidation",
        )
        migrations = component(
            [
                leg(
                    "source-c", "source", whole, "wallet-c", "out-c",
                    occurred_at="2022-01-01T00:00:00Z",
                ),
                leg(
                    "gap-1-in",
                    "retained",
                    whole,
                    "wallet-gap",
                    occurred_at="2022-02-01T00:00:00Z",
                ),
                leg(
                    "gap-1-out",
                    "source",
                    whole,
                    "wallet-gap",
                    occurred_at="2022-12-01T00:00:00Z",
                ),
                leg(
                    "gap-2-in",
                    "retained",
                    whole,
                    "wallet-gap-2",
                    occurred_at="2023-01-01T00:00:00Z",
                ),
                leg(
                    "gap-2-out",
                    "source",
                    whole,
                    "wallet-gap-2",
                    occurred_at="2023-12-01T00:00:00Z",
                ),
                leg(
                    "dest-d", "destination", half, "wallet-d", "in-d",
                    occurred_at="2024-01-01T00:00:00Z",
                ),
                leg(
                    "dest-e", "destination", half, "wallet-e", "in-e",
                    occurred_at="2024-01-01T00:00:00Z",
                ),
            ],
            [
                allocation(0, "source-c", "gap-1-in", whole),
                allocation(1, "gap-1-out", "gap-2-in", whole),
                allocation(2, "gap-2-out", "dest-d", half),
                allocation(3, "gap-2-out", "dest-e", half),
            ],
            id="component-migrations",
            component_type="manual_bridge",
            evidence_grade="reviewed",
            evidence_kind="manual_reconstruction",
        )

        projection = project_effective_components(
            PROFILE,
            rows,
            WALLETS,
            (),
            (consolidation, migrations),
        )
        pairs, cross_asset = apply_manual_pairs(
            projection.rows, (), projection.manual_pair_records
        )
        normalized = normalize_tax_asset_inputs(
            PROFILE, "BTC", projection.rows, WALLETS, pairs
        )

        self.assertFalse(projection.blockers)
        self.assertFalse(cross_asset)
        self.assertFalse(normalized.quarantines)
        self.assertEqual(len(normalized.transfers), 6)
        route = {
            (transfer.from_wallet_label, transfer.to_wallet_label)
            for transfer in normalized.transfers
        }
        self.assertEqual(
            route,
            {
                ("A", "C"),
                ("B", "C"),
                ("C", "Missing historical wallet"),
                (
                    "Missing historical wallet",
                    "Second missing historical wallet",
                ),
                ("Second missing historical wallet", "D"),
                ("Second missing historical wallet", "E"),
            },
        )
        chronological = sorted(
            normalized.transfers, key=lambda transfer: transfer.occurred_at
        )
        self.assertEqual(chronological[0].to_wallet_label, "C")
        self.assertEqual(
            chronological[-1].from_wallet_label,
            "Second missing historical wallet",
        )
        consolidation_groups = {
            transfer.group_id
            for transfer in normalized.transfers
            if transfer.to_wallet_label == "C"
        }
        self.assertEqual(len(consolidation_groups), 1)

    def test_engine_carries_basis_across_btc_lbtc_component(self):
        amount = 100_000_000_000
        acquisition = row(
            "buy",
            "wallet-a",
            "inbound",
            amount,
            occurred_at="2023-01-01T00:00:00Z",
        )
        source = row(
            "peg-out",
            "wallet-a",
            "outbound",
            amount,
            asset="BTC",
            occurred_at="2024-01-01T00:00:00Z",
        )
        destination = row(
            "peg-in",
            "wallet-b",
            "inbound",
            amount,
            asset="LBTC",
            occurred_at="2024-01-01T00:10:00Z",
        )
        legs = [
            leg("btc", "source", amount, "wallet-a", "peg-out", asset="BTC", rail="bitcoin"),
            leg("lbtc", "destination", amount, "wallet-b", "peg-in", asset="LBTC", rail="liquid"),
        ]
        reviewed = component(
            legs,
            [allocation(0, "btc", "lbtc", amount)],
            component_type="peg",
        )

        result = build_tax_engine(PROFILE).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=[acquisition, source, destination],
                wallet_refs_by_id=WALLETS,
                manual_pair_records=(),
                authored_active_custody_components=(reviewed,),
            )
        )

        self.assertFalse(result.quarantines)
        lbtc_holdings = sum(
            totals["quantity"]
            for (_wallet_id, _label, _account, asset), totals in result.wallet_holdings.items()
            if asset == "LBTC"
        )
        self.assertEqual(float(lbtc_holdings), 1.0)
        self.assertEqual(result.cross_asset_pairs[0]["component_id"], "component-1")

        at_profile = {
            **PROFILE,
            "tax_country": "at",
            "gains_algorithm": "moving_average_at",
        }
        at_result = build_tax_engine(at_profile).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=[acquisition, source, destination],
                wallet_refs_by_id=WALLETS,
                manual_pair_records=(),
                authored_active_custody_components=(reviewed,),
            )
        )
        self.assertFalse(at_result.quarantines)
        self.assertEqual(
            float(
                sum(
                    totals["quantity"]
                    for (_wallet_id, _label, _account, asset), totals in at_result.wallet_holdings.items()
                    if asset == "LBTC"
                )
            ),
            1.0,
        )

    def test_handler_fail_closes_replicated_invalid_active_component(self):
        with tempfile.TemporaryDirectory() as root:
            conn = open_db(root)
            try:
                conn.execute(
                    "INSERT INTO workspaces(id, label, created_at) "
                    "VALUES('ws', 'ws', '2023-01-01T00:00:00Z')"
                )
                conn.execute(
                    """
                    INSERT INTO profiles(
                        id, workspace_id, label, fiat_currency, tax_country,
                        gains_algorithm, created_at
                    ) VALUES('profile', 'ws', 'main', 'EUR', 'generic', 'FIFO',
                             '2023-01-01T00:00:00Z')
                    """
                )
                for wallet_id in ("a", "b"):
                    conn.execute(
                        """
                        INSERT INTO wallets(
                            id, workspace_id, profile_id, label, kind,
                            config_json, created_at
                        ) VALUES(?, 'ws', 'profile', ?, 'descriptor', '{}',
                                 '2023-01-01T00:00:00Z')
                        """,
                        (wallet_id, wallet_id.upper()),
                    )
                for tx_id, wallet_id, direction, amount, occurred_at in (
                    ("buy", "a", "inbound", 100, "2023-01-01T00:00:00Z"),
                    ("out", "a", "outbound", 100, "2024-01-01T00:00:00Z"),
                    ("in", "b", "inbound", 90, "2024-01-01T00:10:00Z"),
                    (
                        "later-source-spend",
                        "a",
                        "outbound",
                        50,
                        "2025-01-01T00:00:00Z",
                    ),
                ):
                    conn.execute(
                        """
                        INSERT INTO transactions(
                            id, workspace_id, profile_id, wallet_id,
                            fingerprint, occurred_at, direction, asset, amount,
                            fee, fiat_currency, fiat_rate, fiat_rate_exact,
                            raw_json, created_at
                        ) VALUES(?, 'ws', 'profile', ?, ?, ?, ?, 'BTC', ?, 0,
                                 'EUR', 50000, '50000', '{}', ?)
                        """,
                        (
                            tx_id,
                            wallet_id,
                            f"fp-{tx_id}",
                            occurred_at,
                            direction,
                            amount,
                            occurred_at,
                        ),
                    )
                draft = create_component(
                    conn,
                    workspace_id="ws",
                    profile_id="profile",
                    component_type="native_transfer",
                    legs=[
                        {
                            **leg(
                                "source",
                                "source",
                                100,
                                "a",
                                "out",
                                occurred_at="2024-01-01T00:00:00Z",
                            ),
                            "id": "source",
                        },
                        {
                            **leg(
                                "dest",
                                "destination",
                                90,
                                "b",
                                "in",
                                occurred_at="2024-01-01T00:10:00Z",
                            ),
                            "id": "dest",
                        },
                    ],
                    allocations=[allocation(0, "source", "dest", 90)],
                )
                self.assertEqual("draft", draft["effective_state"])
                # Model an active header/partial graph received row-by-row from
                # replication. Normal activation correctly rejects this shape.
                conn.execute(
                    "UPDATE custody_components SET state = 'active', activated_at = ? "
                    "WHERE id = ?",
                    ("2024-01-02T00:00:00Z", draft["id"]),
                )
                profile = conn.execute(
                    "SELECT * FROM profiles WHERE id = 'profile'"
                ).fetchone()

                state = handlers.build_ledger_state(conn, profile)
            finally:
                conn.close()

        self.assertEqual(
            {"buy"}, {entry["transaction_id"] for entry in state["entries"]}
        )
        self.assertEqual(
            {"out", "in", "later-source-spend"},
            {item["transaction_id"] for item in state["quarantines"]},
        )
        for transaction_id in ("out", "in"):
            details = [
                json.loads(quarantine["detail_json"])
                for quarantine in state["quarantines"]
                if quarantine["transaction_id"] == transaction_id
            ]
            actionable = next(
                detail for detail in details if "resolution" in detail
            )
            self.assertEqual(
                "custody_component_authored_active_invalid",
                actionable["blocker_code"],
            )
            self.assertIn("supersede", actionable["resolution"])
        later = next(
            item
            for item in state["quarantines"]
            if item["transaction_id"] == "later-source-spend"
        )
        self.assertEqual("basis_provenance_incomplete", later["reason"])

    def test_handler_consumes_effective_component_from_sqlite(self):
        amount = 100_000_000_000
        with tempfile.TemporaryDirectory() as root:
            conn = open_db(root)
            try:
                conn.execute(
                    "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'ws', '2023-01-01T00:00:00Z')"
                )
                conn.execute(
                    """
                    INSERT INTO profiles(
                        id, workspace_id, label, fiat_currency, tax_country,
                        gains_algorithm, created_at
                    ) VALUES('profile', 'ws', 'main', 'EUR', 'generic', 'FIFO',
                             '2023-01-01T00:00:00Z')
                    """
                )
                for wallet_id, label, kind in (
                    ("a", "A", "descriptor"),
                    ("gap", "Gap", "untracked"),
                    ("b", "B", "descriptor"),
                ):
                    conn.execute(
                        """
                        INSERT INTO wallets(
                            id, workspace_id, profile_id, label, kind,
                            config_json, created_at
                        ) VALUES(?, 'ws', 'profile', ?, ?, '{}',
                                 '2023-01-01T00:00:00Z')
                        """,
                        (wallet_id, label, kind),
                    )
                for tx_id, wallet_id, direction, occurred_at in (
                    ("buy", "a", "inbound", "2023-01-01T00:00:00Z"),
                    ("out", "a", "outbound", "2024-01-01T00:00:00Z"),
                    ("in", "b", "inbound", "2024-03-01T00:00:00Z"),
                ):
                    conn.execute(
                        """
                        INSERT INTO transactions(
                            id, workspace_id, profile_id, wallet_id,
                            fingerprint, occurred_at, direction, asset, amount,
                            fee, fiat_currency, fiat_rate, fiat_rate_exact,
                            kind, raw_json, created_at
                        ) VALUES(?, 'ws', 'profile', ?, ?, ?, ?, 'BTC', ?, 0,
                                 'EUR', 50000, '50000', ?, '{}', ?)
                        """,
                        (
                            tx_id,
                            wallet_id,
                            f"fp-{tx_id}",
                            occurred_at,
                            direction,
                            amount,
                            "deposit" if direction == "inbound" else "withdrawal",
                            occurred_at,
                        ),
                    )
                draft = create_component(
                    conn,
                    workspace_id="ws",
                    profile_id="profile",
                    component_type="manual_bridge",
                    evidence_kind="manual_reconstruction",
                    evidence_grade="reviewed",
                    legs=[
                        {
                            "id": "source-a",
                            "role": "source",
                            "rail": "bitcoin",
                            "chain": "bitcoin",
                            "network": "main",
                            "asset": "BTC",
                            "exposure": "bitcoin",
                            "conservation_unit": "msat",
                            "amount_msat": amount,
                            "transaction_id": "out",
                            "wallet_id": "a",
                        },
                        {
                            "id": "gap-in",
                            "role": "retained",
                            "rail": "untracked",
                            "asset": "BTC",
                            "exposure": "bitcoin",
                            "conservation_unit": "msat",
                            "amount_msat": amount,
                            "occurred_at": "2024-02-01T00:00:00Z",
                            "wallet_id": "gap",
                        },
                        {
                            "id": "gap-out",
                            "role": "source",
                            "rail": "untracked",
                            "asset": "BTC",
                            "exposure": "bitcoin",
                            "conservation_unit": "msat",
                            "amount_msat": amount,
                            "occurred_at": "2024-02-28T00:00:00Z",
                            "wallet_id": "gap",
                        },
                        {
                            "id": "dest-b",
                            "role": "destination",
                            "rail": "bitcoin",
                            "chain": "bitcoin",
                            "network": "main",
                            "asset": "BTC",
                            "exposure": "bitcoin",
                            "conservation_unit": "msat",
                            "amount_msat": amount,
                            "transaction_id": "in",
                            "wallet_id": "b",
                        },
                    ],
                    allocations=[
                        {
                            "source_leg_id": "source-a",
                            "sink_leg_id": "gap-in",
                            "source_amount_msat": amount,
                            "sink_amount_msat": amount,
                        },
                        {
                            "source_leg_id": "gap-out",
                            "sink_leg_id": "dest-b",
                            "source_amount_msat": amount,
                            "sink_amount_msat": amount,
                        },
                    ],
                )
                activate_component(conn, draft["id"])
                profile = conn.execute(
                    "SELECT * FROM profiles WHERE id = 'profile'"
                ).fetchone()
                state = handlers.build_ledger_state(conn, profile)
            finally:
                conn.close()

        self.assertFalse(state["quarantines"])
        self.assertEqual(
            [entry["entry_type"] for entry in state["entries"]].count("transfer_out"),
            2,
        )
        holdings = {
            wallet_label: totals["quantity"]
            for (_wallet_id, wallet_label, _account, asset), totals in state["wallet_holdings"].items()
            if asset == "BTC"
        }
        self.assertEqual(float(holdings.get("B", 0)), 1.0)


if __name__ == "__main__":
    unittest.main()
