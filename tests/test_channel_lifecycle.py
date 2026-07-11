"""Lightning channel-lifecycle netting.

A channel funding tx moves the operator's own BTC into a 2-of-2 they co-control
(not a disposal); a close returns it (not an acquisition). When a separately
synced on-chain wallet records those txs, the tax engine must recognize them via
the derived channel roles and suppress them as non-events — the same machinery
loan collateral lock/release uses.
"""

from __future__ import annotations

import json
import unittest
from decimal import Decimal

from kassiber.core.engines.base import TaxEngineLedgerInputs
from kassiber.core.engines.rp2 import GenericRP2TaxEngine
from kassiber.core.lightning.channel_lifecycle import channel_role_map, channel_transfer_pairs
from kassiber.core.loans import (
    CHANNEL_CLOSE,
    CHANNEL_CLOSE_MISMATCH,
    CHANNEL_OPEN,
    CHANNEL_OPEN_MISMATCH,
)

FUNDING_TXID = "aa" * 32
CLOSING_TXID = "bb" * 32
ONE_BTC = 100_000_000_000  # msat
FEE_MSAT = 100_000_000  # 0.001 BTC


def _trusted_lnd_close_evidence(*sweeps: tuple[str, int]) -> str:
    return json.dumps(
        {
            "_kassiber_provenance": {"import_source": "lnd_adapter"},
            "channel_close_local_sweeps": [
                {
                    "sweep_txid": sweep_txid,
                    "outpoint": f"{CLOSING_TXID}:{vout}",
                }
                for sweep_txid, vout in sweeps
            ],
        }
    )


class ChannelRoleMapTest(unittest.TestCase):
    def test_funding_outbound_maps_to_channel_open(self) -> None:
        channels = [{"funding_txid": FUNDING_TXID, "closing_txid": CLOSING_TXID}]
        txs = [
            {"id": "open", "external_id": FUNDING_TXID, "external_id_kind": "txid", "direction": "outbound"},
            {"id": "close", "external_id": CLOSING_TXID, "external_id_kind": "txid", "direction": "inbound"},
            {"id": "other", "external_id": "cc" * 32, "external_id_kind": "txid", "direction": "outbound"},
        ]
        roles = channel_role_map(channels, txs)
        self.assertEqual(roles, {"open": CHANNEL_OPEN, "close": CHANNEL_CLOSE})

    def test_funding_outpoint_form_and_case_folding(self) -> None:
        channels = [{"funding_outpoint": f"{FUNDING_TXID}:1"}]
        txs = [{"id": "open", "external_id": FUNDING_TXID.upper(), "external_id_kind": "txid", "direction": "outbound"}]
        self.assertEqual(channel_role_map(channels, txs), {"open": CHANNEL_OPEN})

    def test_direction_guard(self) -> None:
        # A change/receive leg that shares the funding txid but is inbound must
        # NOT be labeled a channel open.
        channels = [{"funding_txid": FUNDING_TXID}]
        txs = [{"id": "change", "external_id": FUNDING_TXID, "external_id_kind": "txid", "direction": "inbound"}]
        self.assertEqual(channel_role_map(channels, txs), {})

    def test_no_channels_is_empty(self) -> None:
        txs = [{"id": "x", "external_id": FUNDING_TXID, "external_id_kind": "txid", "direction": "outbound"}]
        self.assertEqual(channel_role_map([], txs), {})

    def test_noncanonical_channel_ids_never_suppress_or_pair_l1_rows(self) -> None:
        channels = [
            {
                "funding_txid": "provider-funding-id",
                "closing_txid": "provider-closing-id",
                "funding_amount_msat": 100_000_000,
                "close_balance_msat": 100_000_000,
                "wallet_id": "node",
            }
        ]
        txs = [
            {
                "id": "open",
                "external_id": "provider-funding-id",
                "direction": "outbound",
                "wallet_id": "onchain",
                "asset": "BTC",
                "amount": 100_000_000,
                "fee": 0,
                "occurred_at": "2026-01-01T00:00:00Z",
            },
            {
                "id": "close",
                "external_id": "provider-closing-id",
                "direction": "inbound",
                "wallet_id": "onchain",
                "asset": "BTC",
                "amount": 100_000_000,
                "fee": 0,
                "occurred_at": "2026-02-01T00:00:00Z",
            },
        ]
        wallet_refs = {
            "node": {
                "id": "node",
                "label": "Node",
                "wallet_account_id": None,
                "account_code": None,
                "account_label": None,
            }
        }

        self.assertEqual(channel_role_map(channels, txs), {})
        self.assertEqual(channel_transfer_pairs(channels, txs, wallet_refs), [])

    def test_same_txid_on_main_and_regtest_matches_only_the_channel_network(self) -> None:
        channels = [
            {
                "funding_txid": FUNDING_TXID,
                "funding_amount_msat": ONE_BTC,
                "wallet_id": "node",
                "config_json": {"chain": "bitcoin", "network": "main"},
            }
        ]
        txs = [
            {
                "id": "main-open",
                "external_id": FUNDING_TXID,
                "external_id_kind": "txid",
                "direction": "outbound",
                "wallet_id": "onchain",
                "asset": "BTC",
                "amount": ONE_BTC,
                "config_json": {"chain": "bitcoin", "network": "mainnet"},
            },
            {
                "id": "regtest-open",
                "external_id": FUNDING_TXID,
                "external_id_kind": "txid",
                "direction": "outbound",
                "wallet_id": "onchain",
                "asset": "BTC",
                "amount": ONE_BTC,
                "config_json": {"chain": "bitcoin", "network": "regtest"},
            },
        ]

        self.assertEqual(
            channel_role_map(channels, txs),
            {"main-open": CHANNEL_OPEN},
        )
        pairs = channel_transfer_pairs(channels, txs, _wallet_refs())
        self.assertEqual(["main-open"], [pair["out"]["id"] for pair in pairs])

    def test_conflicting_wallet_and_backend_network_metadata_fail_closed(self) -> None:
        channels = [
            {
                "funding_txid": FUNDING_TXID,
                "funding_amount_msat": ONE_BTC,
                "wallet_id": "node",
                "config_json": {"chain": "bitcoin", "network": "regtest"},
                "chain": "bitcoin",
                "network": "main",
            }
        ]
        txs = [
            {
                "id": "open",
                "external_id": FUNDING_TXID,
                "external_id_kind": "txid",
                "direction": "outbound",
                "wallet_id": "onchain",
                "asset": "BTC",
                "amount": ONE_BTC,
                "config_json": {"chain": "bitcoin", "network": "main"},
            }
        ]

        self.assertEqual({}, channel_role_map(channels, txs))
        self.assertEqual([], channel_transfer_pairs(channels, txs, _wallet_refs()))

    def test_adapter_observed_network_scopes_blank_backend_and_wallet(self) -> None:
        channels = [
            {
                "funding_txid": FUNDING_TXID,
                "funding_amount_msat": ONE_BTC,
                "wallet_id": "node",
                "raw_json": {"chain": "bitcoin", "network": "regtest"},
            }
        ]
        txs = [
            {
                "id": "main-open",
                "external_id": FUNDING_TXID,
                "external_id_kind": "txid",
                "direction": "outbound",
                "wallet_id": "onchain",
                "asset": "BTC",
                "amount": ONE_BTC,
                "config_json": {"chain": "bitcoin", "network": "main"},
            },
            {
                "id": "regtest-open",
                "external_id": FUNDING_TXID,
                "external_id_kind": "txid",
                "direction": "outbound",
                "wallet_id": "onchain",
                "asset": "BTC",
                "amount": ONE_BTC,
                "config_json": {"chain": "bitcoin", "network": "regtest"},
            },
        ]

        self.assertEqual(
            {"regtest-open": CHANNEL_OPEN},
            channel_role_map(channels, txs),
        )

    def test_conflicting_observed_and_backend_network_fail_closed(self) -> None:
        channels = [
            {
                "funding_txid": FUNDING_TXID,
                "funding_amount_msat": ONE_BTC,
                "wallet_id": "node",
                "raw_json": {"chain": "bitcoin", "network": "regtest"},
                "chain": "bitcoin",
                "network": "mainnet",
            }
        ]
        txs = [
            {
                "id": "regtest-open",
                "external_id": FUNDING_TXID,
                "external_id_kind": "txid",
                "direction": "outbound",
                "wallet_id": "onchain",
                "asset": "BTC",
                "amount": ONE_BTC,
                "config_json": {"chain": "bitcoin", "network": "regtest"},
            }
        ]

        self.assertEqual({}, channel_role_map(channels, txs))
        self.assertEqual([], channel_transfer_pairs(channels, txs, _wallet_refs()))

    def test_contradictory_raw_graph_and_external_txid_never_match(self) -> None:
        other_txid = "cc" * 32
        channels = [
            {
                "funding_txid": FUNDING_TXID,
                "funding_amount_msat": ONE_BTC,
                "wallet_id": "node",
            }
        ]
        txs = [
            {
                "id": "contradictory-open",
                "external_id": FUNDING_TXID,
                "external_id_kind": "txid",
                "direction": "outbound",
                "wallet_id": "onchain",
                "asset": "BTC",
                "amount": ONE_BTC,
                "raw_json": {
                    "txid": other_txid,
                    "vin": [],
                    "vout": [],
                },
            }
        ]

        self.assertEqual({}, channel_role_map(channels, txs))
        self.assertEqual([], channel_transfer_pairs(channels, txs, _wallet_refs()))

    def test_contradictory_funding_txid_and_outpoint_never_match(self) -> None:
        channels = [
            {
                "funding_txid": FUNDING_TXID,
                "funding_outpoint": f"{'cc' * 32}:0",
                "funding_amount_msat": ONE_BTC,
                "wallet_id": "node",
            }
        ]
        txs = [
            {
                "id": "open",
                "external_id": FUNDING_TXID,
                "external_id_kind": "txid",
                "direction": "outbound",
                "wallet_id": "onchain",
                "asset": "BTC",
                "amount": ONE_BTC,
            }
        ]

        self.assertEqual({}, channel_role_map(channels, txs))
        self.assertEqual([], channel_transfer_pairs(channels, txs, _wallet_refs()))

    def test_close_and_sweep_do_not_cross_networks(self) -> None:
        sweep_txid = "dd" * 32
        channels = [
            {
                "funding_txid": FUNDING_TXID,
                "closing_txid": CLOSING_TXID,
                "funding_amount_msat": ONE_BTC,
                "close_balance_msat": ONE_BTC,
                "wallet_id": "node",
                "config_json": {"chain": "bitcoin", "network": "main"},
            }
        ]
        txs = [
            {
                "id": "main-open",
                "external_id": FUNDING_TXID,
                "external_id_kind": "txid",
                "direction": "outbound",
                "wallet_id": "onchain",
                "asset": "BTC",
                "amount": ONE_BTC,
                "config_json": {"chain": "bitcoin", "network": "main"},
            },
            {
                "id": "regtest-direct-close",
                "external_id": CLOSING_TXID,
                "external_id_kind": "txid",
                "direction": "inbound",
                "wallet_id": "onchain",
                "asset": "BTC",
                "amount": ONE_BTC,
                "config_json": {"chain": "bitcoin", "network": "regtest"},
            },
            {
                "id": "regtest-sweep",
                "external_id": sweep_txid,
                "external_id_kind": "txid",
                "direction": "inbound",
                "wallet_id": "onchain",
                "asset": "BTC",
                "amount": ONE_BTC,
                "config_json": {"chain": "bitcoin", "network": "regtest"},
                "raw_json": {
                    "txid": sweep_txid,
                    "chain": "bitcoin",
                    "network": "regtest",
                    "vin": [{"txid": CLOSING_TXID, "vout": 0}],
                },
            },
        ]

        self.assertEqual(
            {"main-open": CHANNEL_OPEN},
            channel_role_map(channels, txs),
        )
        pairs = channel_transfer_pairs(channels, txs, _wallet_refs())
        self.assertEqual([CHANNEL_OPEN], [pair["kind"] for pair in pairs])

    def test_split_atomic_channel_cannot_link_open_and_close_across_networks(self) -> None:
        channel_id = f"lnd:{FUNDING_TXID}:0"
        channels = [
            {
                "funding_txid": FUNDING_TXID,
                "funding_amount_msat": ONE_BTC,
                "wallet_id": "node",
                "channel_id": channel_id,
                "config_json": {"chain": "bitcoin", "network": "main"},
            },
            {
                "closing_txid": CLOSING_TXID,
                "close_balance_msat": ONE_BTC,
                "wallet_id": "node",
                "channel_id": channel_id,
                "config_json": {"chain": "bitcoin", "network": "regtest"},
            },
        ]
        txs = [
            {
                "id": "main-open",
                "external_id": FUNDING_TXID,
                "external_id_kind": "txid",
                "direction": "outbound",
                "wallet_id": "onchain",
                "asset": "BTC",
                "amount": ONE_BTC,
                "config_json": {"chain": "bitcoin", "network": "main"},
            },
            {
                "id": "regtest-close",
                "external_id": CLOSING_TXID,
                "external_id_kind": "txid",
                "direction": "inbound",
                "wallet_id": "onchain",
                "asset": "BTC",
                "amount": ONE_BTC,
                "config_json": {"chain": "bitcoin", "network": "regtest"},
            },
        ]

        pairs = channel_transfer_pairs(channels, txs, _wallet_refs())
        self.assertEqual([CHANNEL_OPEN], [pair["kind"] for pair in pairs])
        self.assertEqual(
            {
                "main-open": CHANNEL_OPEN,
                "regtest-close": CHANNEL_CLOSE_MISMATCH,
            },
            channel_role_map(channels, txs),
        )

    def test_contradictory_close_and_sweep_graphs_never_suppress(self) -> None:
        sweep_external_txid = "dd" * 32
        channels = [
            {
                "funding_txid": FUNDING_TXID,
                "closing_txid": CLOSING_TXID,
                "funding_amount_msat": ONE_BTC,
                "close_balance_msat": ONE_BTC,
                "wallet_id": "node",
            }
        ]
        txs = [
            {
                "id": "open",
                "external_id": FUNDING_TXID,
                "external_id_kind": "txid",
                "direction": "outbound",
                "wallet_id": "onchain",
                "asset": "BTC",
                "amount": ONE_BTC,
            },
            {
                "id": "contradictory-direct-close",
                "external_id": CLOSING_TXID,
                "external_id_kind": "txid",
                "direction": "inbound",
                "wallet_id": "onchain",
                "asset": "BTC",
                "amount": ONE_BTC,
                "raw_json": {"txid": "cc" * 32, "vin": [], "vout": []},
            },
            {
                "id": "contradictory-sweep",
                "external_id": sweep_external_txid,
                "external_id_kind": "txid",
                "direction": "inbound",
                "wallet_id": "onchain",
                "asset": "BTC",
                "amount": ONE_BTC,
                "raw_json": {
                    "txid": "ee" * 32,
                    "vin": [{"txid": CLOSING_TXID, "vout": 0}],
                },
            },
        ]

        self.assertEqual({"open": CHANNEL_OPEN}, channel_role_map(channels, txs))
        pairs = channel_transfer_pairs(channels, txs, _wallet_refs())
        self.assertEqual([CHANNEL_OPEN], [pair["kind"] for pair in pairs])

    def test_funding_with_external_payment_flags_mismatch(self) -> None:
        # The recorded outflow (channel + external payment) clearly exceeds the
        # funded balance: suppressing the whole row would untax the payment.
        channels = [
            {"funding_txid": FUNDING_TXID, "funding_amount_msat": 100_000_000_000}
        ]
        txs = [
            {
                "id": "open",
                "external_id": FUNDING_TXID,
                "external_id_kind": "txid",
                "direction": "outbound",
                "amount": 130_000_000_000,  # 0.3 BTC beyond the channel
                "fee": 500_000,
            }
        ]
        self.assertEqual(
            channel_role_map(channels, txs), {"open": CHANNEL_OPEN_MISMATCH}
        )

    def test_batched_open_sums_funded_amounts_per_txid(self) -> None:
        # multifundchannel: one funding tx opens N channels — the recorded
        # outflow equals the SUM of the per-channel funded amounts, so
        # first-wins capture would false-positive the mismatch guard.
        channels = [
            {"funding_txid": FUNDING_TXID, "funding_amount_msat": 60_000_000_000},
            {"funding_txid": FUNDING_TXID, "funding_amount_msat": 40_000_000_000},
        ]
        txs = [
            {
                "id": "open",
                "external_id": FUNDING_TXID,
                "external_id_kind": "txid",
                "direction": "outbound",
                "amount": 100_000_000_000,
                "fee": 500_000,
            }
        ]
        self.assertEqual(channel_role_map(channels, txs), {"open": CHANNEL_OPEN})

    def test_funding_amount_within_tolerance_still_opens(self) -> None:
        channels = [
            {"funding_txid": FUNDING_TXID, "funding_amount_msat": 100_000_000_000}
        ]
        txs = [
            {
                "id": "open",
                "external_id": FUNDING_TXID,
                "external_id_kind": "txid",
                "direction": "outbound",
                "amount": 100_000_000_000,
                "fee": 500_000,
            }
        ]
        self.assertEqual(channel_role_map(channels, txs), {"open": CHANNEL_OPEN})

    def test_lnd_amount_bearing_open_rejects_any_principal_residual(self) -> None:
        channels = [
            {
                "funding_txid": FUNDING_TXID,
                "funding_amount_msat": 100_000_000,
                "wallet_id": "node",
                "channel_id": f"lnd:{FUNDING_TXID}:0",
            }
        ]
        txs = [
            {
                "id": "open",
                "external_id": FUNDING_TXID,
                "external_id_kind": "txid",
                "direction": "outbound",
                "wallet_id": "onchain",
                "asset": "BTC",
                "amount": 100_001_000,
                "fee": 0,
                "occurred_at": "2026-01-01T00:00:00Z",
                "description": "batched payment",
                "note": None,
                "kind": "withdrawal",
            }
        ]

        self.assertEqual(
            channel_role_map(channels, txs),
            {"open": CHANNEL_OPEN_MISMATCH},
        )


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
        "node": {
            "id": "node",
            "label": "node",
            "wallet_account_id": "acct-node",
            "account_code": "LN",
            "account_label": "Lightning",
        },
        "node-2": {
            "id": "node-2",
            "label": "node-2",
            "wallet_account_id": "acct-node-2",
            "account_code": "LN2",
            "account_label": "Lightning 2",
        },
    }


def _row(
    tx_id,
    direction,
    amount_msat,
    occurred_at,
    *,
    external_id=None,
    fee=0,
    wallet_id="onchain",
):
    wallet_ref = _wallet_refs()[wallet_id]
    return {
        "id": tx_id,
        "wallet_id": wallet_id,
        "wallet_label": wallet_ref["label"],
        "wallet_account_id": wallet_ref["wallet_account_id"],
        "account_code": wallet_ref["account_code"],
        "account_label": wallet_ref["account_label"],
        "asset": "BTC",
        "direction": direction,
        "amount": amount_msat,
        "fee": fee,
        "fiat_rate": 50_000,
        "fiat_value": None,
        "kind": "deposit" if direction == "inbound" else "withdrawal",
        "description": tx_id,
        "note": None,
        "external_id": external_id or tx_id,
        "external_id_kind": "txid",
        "occurred_at": occurred_at,
    }


def _run(rows, channel_roles, channel_pairs=()):
    return GenericRP2TaxEngine(_profile()).build_ledger_state(
        TaxEngineLedgerInputs(
            rows=rows,
            wallet_refs_by_id=_wallet_refs(),
            manual_pair_records=[],
            channel_roles=channel_roles,
            channel_transfer_pairs=channel_pairs,
        )
    )


def _btc_quantity(result):
    return sum(
        totals["quantity"]
        for key, totals in result.account_holdings.items()
        if key[3] == "BTC"
    )


def _has_disposal(result):
    return any(
        Decimal(str(row.get("quantity", 0) or 0)) != 0 for row in result.tax_summary
    )


class ChannelLifecycleEngineTest(unittest.TestCase):
    def test_channel_open_is_suppressed_not_a_disposal(self) -> None:
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z"),
            _row("fund", "outbound", ONE_BTC, "2025-06-01T00:00:00Z", external_id=FUNDING_TXID),
        ]
        # Baseline: with no channel role, the funding outbound books as a disposal.
        baseline = _run(rows, {})
        self.assertEqual(_btc_quantity(baseline), Decimal("0"))
        self.assertTrue(_has_disposal(baseline))

        # Recognized as a channel open: suppressed — the coin stays owned.
        roles = channel_role_map([{"funding_txid": FUNDING_TXID}], rows)
        tagged = _run(rows, roles)
        self.assertEqual(_btc_quantity(tagged), Decimal("1"))
        self.assertFalse(_has_disposal(tagged))

    def test_channel_open_books_miner_fee_without_disposing_principal(self) -> None:
        rows = [
            _row("buy", "inbound", ONE_BTC + FEE_MSAT, "2025-05-01T00:00:00Z"),
            _row(
                "fund",
                "outbound",
                ONE_BTC,
                "2025-06-01T00:00:00Z",
                external_id=FUNDING_TXID,
                fee=FEE_MSAT,
            ),
        ]
        roles = channel_role_map([{"funding_txid": FUNDING_TXID}], rows)
        result = _run(rows, roles)

        # The channel capacity stays owned, but the L1 miner fee left the pool.
        self.assertEqual(_btc_quantity(result), Decimal("1"))
        self.assertFalse(_has_disposal(result))
        fee_entries = [row for row in result.entries if row["entry_type"] == "fee"]
        self.assertEqual(len(fee_entries), 1)
        self.assertEqual(Decimal(str(fee_entries[0]["quantity"])), Decimal("-0.001"))

    def test_austrian_channel_open_fee_uses_alt_lot_when_only_alt_is_available(
        self,
    ) -> None:
        rows = [
            _row("buy", "inbound", ONE_BTC + FEE_MSAT, "2021-02-01T00:00:00Z"),
            _row(
                "fund",
                "outbound",
                ONE_BTC,
                "2025-06-01T00:00:00Z",
                external_id=FUNDING_TXID,
                fee=FEE_MSAT,
            ),
            _row(
                "sell",
                "outbound",
                ONE_BTC // 2,
                "2025-06-02T00:00:00Z",
                external_id="cc" * 32,
            ),
        ]
        roles = channel_role_map([{"funding_txid": FUNDING_TXID}], rows)
        result = _run(rows, roles)

        self.assertEqual(result.quarantines, [])
        self.assertEqual(_btc_quantity(result), Decimal("0.5"))
        fee_entries = [row for row in result.entries if row["entry_type"] == "fee"]
        self.assertEqual(len(fee_entries), 1)
        self.assertEqual(Decimal(str(fee_entries[0]["quantity"])), Decimal("-0.001"))

    def test_channel_open_pair_credits_node_wallet_capacity(self) -> None:
        rows = [
            _row("buy", "inbound", ONE_BTC + FEE_MSAT, "2025-05-01T00:00:00Z"),
            _row(
                "fund",
                "outbound",
                ONE_BTC,
                "2025-06-01T00:00:00Z",
                external_id=FUNDING_TXID,
                fee=FEE_MSAT,
            ),
            _row(
                "node-pay",
                "outbound",
                ONE_BTC // 2,
                "2025-06-02T00:00:00Z",
                wallet_id="node",
            ),
        ]
        channel_rows = [{"funding_txid": FUNDING_TXID, "wallet_id": "node"}]
        result = _run(
            rows,
            channel_role_map(channel_rows, rows),
            channel_transfer_pairs(channel_rows, rows, _wallet_refs()),
        )

        self.assertEqual(result.quarantines, [])
        wallet_quantities = {
            key[1]: totals["quantity"] for key, totals in result.wallet_holdings.items()
        }
        self.assertEqual(wallet_quantities.get("onchain", Decimal("0")), Decimal("0"))
        self.assertEqual(wallet_quantities["node"], Decimal("0.5"))
        transfer_audit = [
            row
            for row in result.intra_audit
            if row.get("pairing_source") == "channel_lifecycle"
        ]
        self.assertEqual(len(transfer_audit), 1)
        self.assertEqual(transfer_audit[0]["from_wallet_id"], "onchain")
        self.assertEqual(transfer_audit[0]["to_wallet_id"], "node")

    def test_open_then_close_round_trip_is_net_zero(self) -> None:
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z"),
            _row("fund", "outbound", ONE_BTC, "2025-06-01T00:00:00Z", external_id=FUNDING_TXID),
            _row("close", "inbound", ONE_BTC, "2025-07-01T00:00:00Z", external_id=CLOSING_TXID),
        ]
        roles = channel_role_map(
            [{"funding_txid": FUNDING_TXID, "closing_txid": CLOSING_TXID}], rows
        )
        result = _run(rows, roles)
        # Both suppressed: exactly the original 1 BTC, no disposal, no second
        # acquisition at market price.
        self.assertEqual(_btc_quantity(result), Decimal("1"))
        self.assertFalse(_has_disposal(result))

    def test_channel_pairs_move_capacity_back_on_close(self) -> None:
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z"),
            _row("fund", "outbound", ONE_BTC, "2025-06-01T00:00:00Z", external_id=FUNDING_TXID),
            _row("close", "inbound", ONE_BTC, "2025-07-01T00:00:00Z", external_id=CLOSING_TXID),
        ]
        channel_rows = [
            {
                "funding_txid": FUNDING_TXID,
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node",
            }
        ]
        result = _run(
            rows,
            channel_role_map(channel_rows, rows),
            channel_transfer_pairs(channel_rows, rows, _wallet_refs()),
        )

        self.assertEqual(result.quarantines, [])
        self.assertFalse(_has_disposal(result))
        wallet_quantities = {
            key[1]: totals["quantity"] for key, totals in result.wallet_holdings.items()
        }
        self.assertEqual(wallet_quantities["onchain"], Decimal("1"))
        self.assertEqual(wallet_quantities.get("node", Decimal("0")), Decimal("0"))

    def test_split_persisted_channel_rows_link_close_to_open_by_channel_id(self) -> None:
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z"),
            _row("fund", "outbound", ONE_BTC, "2025-06-01T00:00:00Z", external_id=FUNDING_TXID),
            _row("close", "inbound", ONE_BTC, "2025-07-01T00:00:00Z", external_id=CLOSING_TXID),
        ]
        channel_rows = [
            {
                "funding_txid": FUNDING_TXID,
                "wallet_id": "node",
                "channel_id": "chan-1",
            },
            {
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node",
                "channel_id": "chan-1",
            },
        ]
        pairs = channel_transfer_pairs(channel_rows, rows, _wallet_refs())
        self.assertEqual([pair["kind"] for pair in pairs], [CHANNEL_OPEN, CHANNEL_CLOSE])

        result = _run(rows, channel_role_map(channel_rows, rows), pairs)

        self.assertEqual(result.quarantines, [])
        wallet_quantities = {
            key[1]: totals["quantity"] for key, totals in result.wallet_holdings.items()
        }
        self.assertEqual(wallet_quantities["onchain"], Decimal("1"))
        self.assertEqual(wallet_quantities.get("node", Decimal("0")), Decimal("0"))

    def test_multifund_accounts_share_one_open_move_and_link_each_close(self) -> None:
        # CLN persists one open record per bookkeeper account even though
        # multifundchannel gives all of them the same funding txid. Lifecycle
        # validation sums the per-channel contributions for the whole L1 row,
        # but materializes exactly one atomic wallet->node MOVE. A later close
        # for channel B must still find B's funding record.
        sixty = 60_000_000_000
        forty = 40_000_000_000
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z"),
            _row(
                "fund",
                "outbound",
                ONE_BTC,
                "2025-06-01T00:00:00Z",
                external_id=FUNDING_TXID,
            ),
            _row(
                "close-b",
                "inbound",
                forty,
                "2025-07-01T00:00:00Z",
                external_id=CLOSING_TXID,
            ),
        ]
        channel_rows = [
            {
                "funding_txid": FUNDING_TXID,
                "funding_amount_msat": sixty,
                "wallet_id": "node",
                "channel_id": "channel-a",
            },
            {
                "funding_txid": FUNDING_TXID,
                "funding_amount_msat": forty,
                "wallet_id": "node",
                "channel_id": "channel-b",
            },
            {
                "closing_txid": CLOSING_TXID,
                "close_balance_msat": forty,
                "wallet_id": "node",
                "channel_id": "channel-b",
            },
        ]

        pairs = channel_transfer_pairs(channel_rows, rows, _wallet_refs())
        self.assertEqual(
            [CHANNEL_OPEN, CHANNEL_CLOSE], [pair["kind"] for pair in pairs]
        )
        self.assertEqual(
            1, sum(pair["kind"] == CHANNEL_OPEN for pair in pairs)
        )
        roles = channel_role_map(channel_rows, rows)
        self.assertEqual(
            {"fund": CHANNEL_OPEN, "close-b": CHANNEL_CLOSE}, roles
        )

        result = _run(rows, roles, pairs)
        self.assertEqual([], result.quarantines)
        self.assertFalse(_has_disposal(result))
        wallet_quantities = {
            key[1]: totals["quantity"]
            for key, totals in result.wallet_holdings.items()
        }
        self.assertEqual(Decimal("0.4"), wallet_quantities["onchain"])
        self.assertEqual(Decimal("0.6"), wallet_quantities["node"])

    def test_close_only_window_does_not_synthesize_node_outflow(self) -> None:
        rows = [
            _row("close", "inbound", ONE_BTC, "2025-07-01T00:00:00Z", external_id=CLOSING_TXID),
        ]
        channel_rows = [
            {
                "funding_txid": FUNDING_TXID,
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node",
            }
        ]
        roles = channel_role_map(channel_rows, rows)
        self.assertEqual(roles["close"], CHANNEL_CLOSE)
        self.assertEqual(channel_transfer_pairs(channel_rows, rows, _wallet_refs()), [])

        result = _run(rows, roles, [])
        self.assertEqual(result.quarantines, [])
        self.assertFalse(_has_disposal(result))
        self.assertEqual(_btc_quantity(result), Decimal("0"))

    def test_ambiguous_channel_open_owner_does_not_pick_first_wallet(self) -> None:
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z"),
            _row("fund", "outbound", ONE_BTC, "2025-06-01T00:00:00Z", external_id=FUNDING_TXID),
        ]
        channel_rows = [
            {"funding_txid": FUNDING_TXID, "wallet_id": "node"},
            {"funding_txid": FUNDING_TXID, "wallet_id": "node-2"},
        ]

        result = _run(
            rows,
            channel_role_map(channel_rows, rows),
            channel_transfer_pairs(channel_rows, rows, _wallet_refs()),
        )

        self.assertEqual(result.quarantines, [])
        self.assertFalse(_has_disposal(result))
        wallet_quantities = {
            key[1]: totals["quantity"] for key, totals in result.wallet_holdings.items()
        }
        self.assertEqual(wallet_quantities["onchain"], Decimal("1"))
        self.assertEqual(wallet_quantities.get("node", Decimal("0")), Decimal("0"))
        self.assertEqual(wallet_quantities.get("node-2", Decimal("0")), Decimal("0"))
        self.assertFalse(
            any(
                row.get("pairing_source") == "channel_lifecycle"
                for row in result.intra_audit
            )
        )

    def test_ambiguous_channel_close_owner_does_not_pick_first_wallet(self) -> None:
        other_funding_txid = "cc" * 32
        rows = [
            _row("buy", "inbound", 2 * ONE_BTC, "2025-05-01T00:00:00Z"),
            _row("fund-1", "outbound", ONE_BTC, "2025-06-01T00:00:00Z", external_id=FUNDING_TXID),
            _row("fund-2", "outbound", ONE_BTC, "2025-06-02T00:00:00Z", external_id=other_funding_txid),
            _row("close", "inbound", 2 * ONE_BTC, "2025-07-01T00:00:00Z", external_id=CLOSING_TXID),
        ]
        channel_rows = [
            {
                "funding_txid": FUNDING_TXID,
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node",
            },
            {
                "funding_txid": other_funding_txid,
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node-2",
            },
        ]
        pairs = channel_transfer_pairs(channel_rows, rows, _wallet_refs())

        self.assertEqual([p["kind"] for p in pairs], [CHANNEL_OPEN, CHANNEL_OPEN])

        result = _run(rows, channel_role_map(channel_rows, rows), pairs)
        self.assertEqual(result.quarantines, [])
        wallet_quantities = {
            key[1]: totals["quantity"] for key, totals in result.wallet_holdings.items()
        }
        self.assertEqual(wallet_quantities.get("onchain", Decimal("0")), Decimal("0"))
        self.assertEqual(wallet_quantities.get("node", Decimal("0")), Decimal("1"))
        self.assertEqual(wallet_quantities.get("node-2", Decimal("0")), Decimal("1"))
        close_pairs = [
            row
            for row in result.intra_audit
            if row.get("loan_role") == CHANNEL_CLOSE
        ]
        self.assertEqual(close_pairs, [])

    def test_force_close_sweep_round_trip_is_net_zero(self) -> None:
        # A force-close pays the wallet via a separate timelocked SWEEP tx: its
        # own txid never equals the recorded closing txid, but its inputs spend
        # the commitment tx. Without the vin match the open stays suppressed
        # while the sweep books a fresh market-priced acquisition — channel
        # capacity double-counted plus a phantom basis reset.
        sweep_txid = "dd" * 32
        sweep_raw = json.dumps(
            {
                "txid": sweep_txid,
                "vin": [{"txid": CLOSING_TXID, "vout": 0}],
            }
        )
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z"),
            _row("fund", "outbound", ONE_BTC, "2025-06-01T00:00:00Z", external_id=FUNDING_TXID),
            _row("sweep", "inbound", ONE_BTC, "2025-08-01T00:00:00Z", external_id=sweep_txid),
        ]
        rows[2]["raw_json"] = sweep_raw
        channel_rows = [
            {
                "funding_txid": FUNDING_TXID,
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node",
                "raw_json": _trusted_lnd_close_evidence((sweep_txid, 0)),
            }
        ]
        roles = channel_role_map(channel_rows, rows)
        self.assertEqual(roles["sweep"], CHANNEL_CLOSE)
        result = _run(
            rows,
            roles,
            channel_transfer_pairs(channel_rows, rows, _wallet_refs()),
        )

        self.assertEqual(result.quarantines, [])
        self.assertFalse(_has_disposal(result))
        self.assertEqual(_btc_quantity(result), Decimal("1"))
        wallet_quantities = {
            key[1]: totals["quantity"] for key, totals in result.wallet_holdings.items()
        }
        self.assertEqual(wallet_quantities["onchain"], Decimal("1"))
        self.assertEqual(wallet_quantities.get("node", Decimal("0")), Decimal("0"))

    def test_unique_peer_output_payment_is_not_synthesized_as_force_close(self) -> None:
        # Candidate cardinality is not ownership evidence. A peer may spend its
        # commitment output in a later payment to us; accepting the only vin
        # match silently suppresses that taxable receipt as a channel close.
        peer_payment_txid = "dd" * 32
        rows = [
            _row(
                "fund",
                "outbound",
                ONE_BTC,
                "2025-06-01T00:00:00Z",
                external_id=FUNDING_TXID,
            ),
            _row(
                "peer-pay",
                "inbound",
                ONE_BTC,
                "2025-08-01T00:00:00Z",
                external_id=peer_payment_txid,
            ),
        ]
        rows[1]["raw_json"] = json.dumps(
            {
                "txid": peer_payment_txid,
                "vin": [{"txid": CLOSING_TXID, "vout": 1}],
            }
        )
        channel_rows = [
            {
                "funding_txid": FUNDING_TXID,
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node",
                "close_balance_msat": ONE_BTC,
            }
        ]

        roles = channel_role_map(channel_rows, rows)
        pairs = channel_transfer_pairs(channel_rows, rows, _wallet_refs())

        self.assertEqual(roles["peer-pay"], CHANNEL_CLOSE_MISMATCH)
        self.assertFalse([pair for pair in pairs if pair["kind"] == CHANNEL_CLOSE])

        result = _run(rows, roles, pairs)
        self.assertTrue(
            any(
                item["transaction_id"] == "peer-pay"
                and item["reason"] == "channel_close_unresolved"
                for item in result.quarantines
            )
        )


    def test_close_fee_gap_books_as_move_fee_disposal(self) -> None:
        # The settled channel balance at close (bkpr debit) exceeds the
        # on-chain receipt by the commitment fee. Without booking that gap the
        # node wallet keeps a phantom residual forever and the fee is never
        # taxed.
        received = ONE_BTC - 100_000_000  # 0.999 BTC
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z"),
            _row("fund", "outbound", ONE_BTC, "2025-06-01T00:00:00Z", external_id=FUNDING_TXID),
            _row("close", "inbound", received, "2025-07-01T00:00:00Z", external_id=CLOSING_TXID),
        ]
        channel_rows = [
            {
                "funding_txid": FUNDING_TXID,
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node",
                "close_balance_msat": ONE_BTC,
            }
        ]
        pairs = channel_transfer_pairs(channel_rows, rows, _wallet_refs())
        close_pair = next(p for p in pairs if p["kind"] == CHANNEL_CLOSE)
        self.assertEqual(int(close_pair["out"]["fee"]), 100_000_000)

        result = _run(rows, channel_role_map(channel_rows, rows), pairs)
        self.assertEqual(result.quarantines, [])
        # The close fee books as a real fee disposal on the MOVE.
        fee_entries = [
            entry for entry in result.entries if entry["entry_type"] == "transfer_fee"
        ]
        self.assertEqual(len(fee_entries), 1)
        self.assertEqual(Decimal(str(fee_entries[0]["quantity"])), Decimal("-0.001"))
        wallet_quantities = {
            key[1]: totals["quantity"] for key, totals in result.wallet_holdings.items()
        }
        self.assertEqual(wallet_quantities["onchain"], Decimal("0.999"))
        # No phantom residual stranded in the node wallet.
        self.assertEqual(wallet_quantities.get("node", Decimal("0")), Decimal("0"))


    def test_channel_close_carries_alt_availability_back_to_onchain_wallet(self) -> None:
        received = ONE_BTC - FEE_MSAT
        rows = [
            _row("buy", "inbound", ONE_BTC, "2021-02-01T00:00:00Z"),
            _row("fund", "outbound", ONE_BTC, "2025-06-01T00:00:00Z", external_id=FUNDING_TXID),
            _row("close", "inbound", received, "2025-07-01T00:00:00Z", external_id=CLOSING_TXID),
            _row("sell", "outbound", received // 2, "2025-08-01T00:00:00Z"),
        ]
        channel_rows = [
            {
                "funding_txid": FUNDING_TXID,
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node",
                "close_balance_msat": ONE_BTC,
            }
        ]
        result = _run(
            rows,
            channel_role_map(channel_rows, rows),
            channel_transfer_pairs(channel_rows, rows, _wallet_refs()),
        )

        self.assertEqual(result.quarantines, [])
        close_fee = [
            entry
            for entry in result.entries
            if entry["transaction_id"] == "close"
            and entry["entry_type"] == "transfer_fee"
        ]
        self.assertEqual(len(close_fee), 1)
        self.assertIn("at_regime=alt", close_fee[0]["description"])
        sale = next(
            entry for entry in result.entries
            if entry["transaction_id"] == "sell"
            and entry["entry_type"] == "disposal"
        )
        self.assertEqual(sale["at_category"], "alt_taxfree")


    def test_funding_with_external_payment_quarantines_for_split(self) -> None:
        # 1.0 BTC funded into the channel, but the funding tx spent 1.3 BTC —
        # 0.3 went to an external recipient. Suppression would untax the
        # payment; standalone booking would dispose the owned capacity. The
        # engine must quarantine the row for an explicit split review.
        rows = [
            _row("buy", "inbound", 2 * ONE_BTC, "2025-05-01T00:00:00Z"),
            _row(
                "fund",
                "outbound",
                ONE_BTC + 30_000_000_000,
                "2025-06-01T00:00:00Z",
                external_id=FUNDING_TXID,
            ),
        ]
        channel_rows = [
            {
                "funding_txid": FUNDING_TXID,
                "wallet_id": "node",
                "funding_amount_msat": ONE_BTC,
            }
        ]
        result = _run(
            rows,
            channel_role_map(channel_rows, rows),
            channel_transfer_pairs(channel_rows, rows, _wallet_refs()),
        )
        reasons = [q["reason"] for q in result.quarantines]
        self.assertEqual(reasons, ["channel_open_unresolved"])
        # Neither a disposal of the whole outflow nor a capacity MOVE books.
        entry_types = [entry["entry_type"] for entry in result.entries]
        self.assertNotIn("transfer_out", entry_types)
        self.assertFalse(_has_disposal(result))


    def test_implausible_close_gap_quarantines_instead_of_booking_fee(self) -> None:
        # The synthesized close pair clones the receipt row, so the generic
        # transfer-fee implausibility guard (out.amount - in.amount == 0) can
        # never fire for it — a mis-captured close balance (unsynced sweep,
        # HTLC value lost to the peer) would book an UNBOUNDED silent fee.
        # The lifecycle ceiling must quarantine instead.
        received = ONE_BTC - 10_000_000_000  # 0.9 BTC: 10% gap, 10x tolerance
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z"),
            _row("fund", "outbound", ONE_BTC, "2025-06-01T00:00:00Z", external_id=FUNDING_TXID),
            _row("close", "inbound", received, "2025-07-01T00:00:00Z", external_id=CLOSING_TXID),
        ]
        channel_rows = [
            {
                "funding_txid": FUNDING_TXID,
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node",
                "close_balance_msat": ONE_BTC,
            }
        ]
        roles = channel_role_map(channel_rows, rows)
        self.assertEqual(roles["close"], CHANNEL_CLOSE_MISMATCH)
        pairs = channel_transfer_pairs(channel_rows, rows, _wallet_refs())
        self.assertFalse(any(p["kind"] == CHANNEL_CLOSE for p in pairs))

        result = _run(rows, roles, pairs)
        reasons = [q["reason"] for q in result.quarantines]
        self.assertIn("channel_close_unresolved", reasons)
        # No silent 0.1 BTC "fee" disposal books.
        fee_entries = [
            entry for entry in result.entries if entry["entry_type"] == "transfer_fee"
        ]
        self.assertEqual(fee_entries, [])


class AtomicNodeLifecycleTest(unittest.TestCase):
    def test_amountless_coreln_open_cannot_suppress_a_copayment(self) -> None:
        rows = [
            _row(
                "fund",
                "outbound",
                ONE_BTC,
                "2025-06-01T00:00:00Z",
                external_id=FUNDING_TXID,
            )
        ]
        channels = [
            {
                "funding_txid": FUNDING_TXID,
                "wallet_id": "node",
                "channel_id": "coreln:channel-a",
                # listpeerchannels can prove the outpoint before bookkeeper
                # evidence proves our local contribution. The tx may also pay
                # someone external, so txid-only evidence is not a MOVE.
                "funding_amount_msat": 0,
            }
        ]

        self.assertEqual(
            CHANNEL_OPEN_MISMATCH,
            channel_role_map(channels, rows)["fund"],
        )
        self.assertEqual([], channel_transfer_pairs(channels, rows, _wallet_refs()))

    def test_incomplete_open_quarantines_instead_of_suppressing_l1(self) -> None:
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z"),
            _row(
                "fund",
                "outbound",
                ONE_BTC,
                "2025-06-01T00:00:00Z",
                external_id=FUNDING_TXID,
            ),
        ]
        channels = [
            {
                "funding_txid": FUNDING_TXID,
                "wallet_id": "node",
                "channel_id": f"lnd:{FUNDING_TXID}:0",
                "funding_amount_msat": -1,
            }
        ]
        self.assertEqual(
            channel_role_map(channels, rows)["fund"], CHANNEL_OPEN_MISMATCH
        )
        self.assertEqual(channel_transfer_pairs(channels, rows, _wallet_refs()), [])

    def test_complete_open_gets_role_only_with_compensating_move(self) -> None:
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z"),
            _row(
                "fund",
                "outbound",
                ONE_BTC,
                "2025-06-01T00:00:00Z",
                external_id=FUNDING_TXID,
            ),
        ]
        channels = [
            {
                "funding_txid": FUNDING_TXID,
                "wallet_id": "node",
                "channel_id": f"lnd:{FUNDING_TXID}:0",
                "funding_amount_msat": ONE_BTC,
            }
        ]
        pairs = channel_transfer_pairs(channels, rows, _wallet_refs())
        self.assertEqual([pair["kind"] for pair in pairs], [CHANNEL_OPEN])
        self.assertEqual(channel_role_map(channels, rows)["fund"], CHANNEL_OPEN)

    def test_close_only_history_is_incomplete_not_suppressed(self) -> None:
        rows = [
            _row(
                "close",
                "inbound",
                ONE_BTC,
                "2025-07-01T00:00:00Z",
                external_id=CLOSING_TXID,
            )
        ]
        channel_id = f"lnd:{FUNDING_TXID}:0"
        channels = [
            {
                "funding_txid": FUNDING_TXID,
                "wallet_id": "node",
                "channel_id": channel_id,
                "funding_amount_msat": ONE_BTC,
            },
            {
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node",
                "channel_id": channel_id,
                "close_balance_msat": ONE_BTC,
            },
        ]
        self.assertEqual(channel_transfer_pairs(channels, rows, _wallet_refs()), [])
        self.assertEqual(
            channel_role_map(channels, rows)["close"], CHANNEL_CLOSE_MISMATCH
        )

    def test_complete_round_trip_constructs_both_moves_atomically(self) -> None:
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z"),
            _row(
                "fund",
                "outbound",
                ONE_BTC,
                "2025-06-01T00:00:00Z",
                external_id=FUNDING_TXID,
            ),
            _row(
                "close",
                "inbound",
                ONE_BTC,
                "2025-07-01T00:00:00Z",
                external_id=CLOSING_TXID,
            ),
        ]
        channel_id = f"lnd:{FUNDING_TXID}:0"
        channels = [
            {
                "funding_txid": FUNDING_TXID,
                "wallet_id": "node",
                "channel_id": channel_id,
                "funding_amount_msat": ONE_BTC,
            },
            {
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node",
                "channel_id": channel_id,
                "close_balance_msat": ONE_BTC,
            },
        ]
        pairs = channel_transfer_pairs(channels, rows, _wallet_refs())
        self.assertEqual(
            [pair["kind"] for pair in pairs], [CHANNEL_OPEN, CHANNEL_CLOSE]
        )
        self.assertEqual(
            channel_role_map(channels, rows),
            {"fund": CHANNEL_OPEN, "close": CHANNEL_CLOSE},
        )


class MultiSweepCloseTest(unittest.TestCase):
    def test_multi_sweep_close_books_one_fee_for_the_group(self) -> None:
        # A force close pays back in several legs (to_local sweep + HTLC
        # sweep). The single close fee is balance - SUM(legs); per-leg
        # evaluation would book each other leg's amount as a "fee" once each
        # and double-debit the node wallet.
        sweep_one = json.dumps(
            {
                "txid": "dd" * 32,
                "vin": [{"txid": CLOSING_TXID, "vout": 0}],
            }
        )
        sweep_two = json.dumps(
            {
                "txid": "ee" * 32,
                "vin": [{"txid": CLOSING_TXID, "vout": 1}],
            }
        )
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z"),
            _row("fund", "outbound", ONE_BTC, "2025-06-01T00:00:00Z", external_id=FUNDING_TXID),
            _row("sweep-1", "inbound", 60_000_000_000, "2025-08-01T00:00:00Z", external_id="dd" * 32),
            _row("sweep-2", "inbound", 39_900_000_000, "2025-08-02T00:00:00Z", external_id="ee" * 32),
        ]
        rows[2]["raw_json"] = sweep_one
        rows[3]["raw_json"] = sweep_two
        channel_rows = [
            {
                "funding_txid": FUNDING_TXID,
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node",
                "close_balance_msat": ONE_BTC,
                "raw_json": _trusted_lnd_close_evidence(
                    ("dd" * 32, 0), ("ee" * 32, 1)
                ),
            }
        ]
        roles = channel_role_map(channel_rows, rows)
        self.assertEqual(roles["sweep-1"], CHANNEL_CLOSE)
        self.assertEqual(roles["sweep-2"], CHANNEL_CLOSE)
        pairs = channel_transfer_pairs(channel_rows, rows, _wallet_refs())
        close_fees = sorted(
            int(p["out"]["fee"]) for p in pairs if p["kind"] == CHANNEL_CLOSE
        )
        # One group fee (0.001 BTC), on one leg only.
        self.assertEqual(close_fees, [0, 100_000_000])

        result = _run(rows, roles, pairs)
        self.assertEqual(result.quarantines, [])
        wallet_quantities = {
            key[1]: totals["quantity"] for key, totals in result.wallet_holdings.items()
        }
        self.assertEqual(wallet_quantities["onchain"], Decimal("0.999"))
        self.assertEqual(wallet_quantities.get("node", Decimal("0")), Decimal("0"))

    def test_vin_match_after_full_recovery_is_not_a_close_leg(self) -> None:
        # The peer's to_remote output pays them directly from the commitment
        # tx; if they later pay US spending that output, our inbound must not
        # be reclassified as a close leg once the close is fully accounted —
        # that would suppress taxable income.
        payout = json.dumps(
            {
                "txid": "dd" * 32,
                "vin": [{"txid": CLOSING_TXID, "vout": 0}],
            }
        )
        peer_payment = json.dumps(
            {"txid": "ee" * 32, "vin": [{"txid": CLOSING_TXID, "vout": 1}]}
        )
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z"),
            _row("fund", "outbound", ONE_BTC, "2025-06-01T00:00:00Z", external_id=FUNDING_TXID),
            # Our full settled balance came back in the first sweep.
            _row("sweep", "inbound", ONE_BTC, "2025-08-01T00:00:00Z", external_id="dd" * 32),
            # Later inbound funded by the peer's swept commitment output.
            _row("peer-pay", "inbound", 30_000_000_000, "2025-09-01T00:00:00Z", external_id="ee" * 32),
        ]
        rows[2]["raw_json"] = payout
        rows[3]["raw_json"] = peer_payment
        channel_rows = [
            {
                "funding_txid": FUNDING_TXID,
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node",
                "close_balance_msat": ONE_BTC,
                "raw_json": _trusted_lnd_close_evidence(("dd" * 32, 0)),
            }
        ]
        roles = channel_role_map(channel_rows, rows)
        self.assertEqual(roles.get("sweep"), CHANNEL_CLOSE)
        self.assertNotIn("peer-pay", roles)

    def test_competing_vin_matches_without_local_outpoint_evidence_fail_closed(self) -> None:
        # The earlier receipt spends the peer's commitment output; the later
        # receipt is our actual local sweep. Their amounts happen to sum to the
        # captured close balance, so chronological first-fit used to suppress
        # both as non-taxable close receipts.
        peer_payment = json.dumps(
            {"txid": "dd" * 32, "vin": [{"txid": CLOSING_TXID, "vout": 1}]}
        )
        local_sweep = json.dumps(
            {"txid": "ee" * 32, "vin": [{"txid": CLOSING_TXID, "vout": 0}]}
        )
        rows = [
            _row(
                "fund", "outbound", ONE_BTC, "2025-06-01T00:00:00Z",
                external_id=FUNDING_TXID,
            ),
            _row(
                "peer-pay", "inbound", 30_000_000_000,
                "2025-07-01T00:00:00Z", external_id="dd" * 32,
            ),
            _row(
                "local-sweep", "inbound", 70_000_000_000,
                "2025-08-01T00:00:00Z", external_id="ee" * 32,
            ),
        ]
        rows[1]["raw_json"] = peer_payment
        rows[2]["raw_json"] = local_sweep
        channel_rows = [
            {
                "funding_txid": FUNDING_TXID,
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node",
                "close_balance_msat": ONE_BTC,
            }
        ]

        roles = channel_role_map(channel_rows, rows)
        pairs = channel_transfer_pairs(channel_rows, rows, _wallet_refs())

        self.assertEqual(roles["peer-pay"], CHANNEL_CLOSE_MISMATCH)
        self.assertEqual(roles["local-sweep"], CHANNEL_CLOSE_MISMATCH)
        self.assertFalse([pair for pair in pairs if pair["kind"] == CHANNEL_CLOSE])

    def test_user_transaction_marker_cannot_attest_a_local_sweep(self) -> None:
        sweep_txid = "ee" * 32
        rows = [
            _row(
                "local-sweep",
                "inbound",
                70_000_000_000,
                "2025-08-01T00:00:00Z",
                external_id=sweep_txid,
            )
        ]
        rows[0]["raw_json"] = json.dumps(
            {
                "txid": sweep_txid,
                "vin": [{"txid": CLOSING_TXID, "vout": 0}],
                "channel_close_local_outpoint": f"{CLOSING_TXID}:0",
            }
        )
        channel_rows = [
            {
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node",
                "close_balance_msat": 70_000_000_000,
            }
        ]

        self.assertEqual(
            channel_role_map(channel_rows, rows)["local-sweep"],
            CHANNEL_CLOSE_MISMATCH,
        )

    def test_competing_vin_matches_select_only_exact_local_outpoint(self) -> None:
        peer_payment = json.dumps(
            {"txid": "dd" * 32, "vin": [{"txid": CLOSING_TXID, "vout": 1}]}
        )
        local_sweep = json.dumps(
            {
                "txid": "ee" * 32,
                "vin": [{"txid": CLOSING_TXID, "vout": 0}],
            }
        )
        rows = [
            _row(
                "fund",
                "outbound",
                ONE_BTC,
                "2025-06-01T00:00:00Z",
                external_id=FUNDING_TXID,
            ),
            _row(
                "peer-pay",
                "inbound",
                30_000_000_000,
                "2025-07-01T00:00:00Z",
                external_id="dd" * 32,
            ),
            _row(
                "local-sweep",
                "inbound",
                70_000_000_000,
                "2025-08-01T00:00:00Z",
                external_id="ee" * 32,
            ),
        ]
        rows[1]["raw_json"] = peer_payment
        rows[2]["raw_json"] = local_sweep
        channel_rows = [
            {
                "funding_txid": FUNDING_TXID,
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node",
                "close_balance_msat": 70_000_000_000,
                "raw_json": _trusted_lnd_close_evidence(("ee" * 32, 0)),
            }
        ]

        roles = channel_role_map(channel_rows, rows)
        pairs = channel_transfer_pairs(channel_rows, rows, _wallet_refs())

        self.assertNotIn("peer-pay", roles)
        self.assertEqual(CHANNEL_CLOSE, roles["local-sweep"])
        close_pairs = [pair for pair in pairs if pair["kind"] == CHANNEL_CLOSE]
        self.assertEqual(["local-sweep"], [pair["in"]["id"] for pair in close_pairs])

    def test_duplicate_claims_of_one_local_outpoint_fail_closed(self) -> None:
        duplicate_marker = f"{CLOSING_TXID}:0"
        sweep_one = json.dumps(
            {
                "txid": "dd" * 32,
                "vin": [{"txid": CLOSING_TXID, "vout": 0}],
                "channel_close_local_outpoint": duplicate_marker,
            }
        )
        sweep_two = json.dumps(
            {
                "txid": "ee" * 32,
                "vin": [{"txid": CLOSING_TXID, "vout": 0}],
                "channel_close_local_outpoint": duplicate_marker,
            }
        )
        rows = [
            _row(
                "fund",
                "outbound",
                ONE_BTC,
                "2025-06-01T00:00:00Z",
                external_id=FUNDING_TXID,
            ),
            _row(
                "sweep-1",
                "inbound",
                40_000_000_000,
                "2025-07-01T00:00:00Z",
                external_id="dd" * 32,
            ),
            _row(
                "sweep-2",
                "inbound",
                60_000_000_000,
                "2025-08-01T00:00:00Z",
                external_id="ee" * 32,
            ),
        ]
        rows[1]["raw_json"] = sweep_one
        rows[2]["raw_json"] = sweep_two
        channel_rows = [
            {
                "funding_txid": FUNDING_TXID,
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node",
                "close_balance_msat": ONE_BTC,
            }
        ]

        roles = channel_role_map(channel_rows, rows)
        pairs = channel_transfer_pairs(channel_rows, rows, _wallet_refs())

        self.assertEqual(CHANNEL_CLOSE_MISMATCH, roles["sweep-1"])
        self.assertEqual(CHANNEL_CLOSE_MISMATCH, roles["sweep-2"])
        self.assertFalse([pair for pair in pairs if pair["kind"] == CHANNEL_CLOSE])


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
