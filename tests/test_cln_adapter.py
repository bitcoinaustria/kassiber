"""Tests for the Core Lightning adapter.

Covers the scaffold contract (registration on import, `fetch_node_snapshot`
shape, private-channel handling), opsec discards (preimages, bolt11, route
hops never reach the snapshot), the routed-vs-invoice double-count
regression (P1 fix #1), and the read-only RPC allowlist.
"""

from __future__ import annotations

import tempfile
import unittest
from collections.abc import Mapping
from pathlib import Path
from types import SimpleNamespace
from typing import Any
from unittest.mock import patch

from kassiber.backends import get_db_backend
from kassiber.core import accounts as core_accounts
from kassiber.core import imports as core_imports
from kassiber.core import wallets as core_wallets
from kassiber.core.lightning import (
    NodeSnapshot,
    build_profitability_report,
    register_adapter,
    resolve_adapter,
)
from kassiber.core.lightning import cln as core_cln
from kassiber.core.lightning.cln import CoreLightningAdapter
from kassiber.core.repo import fetch_wallet_with_account, invalidate_journals
from kassiber.db import open_db
from kassiber.errors import AppError


def _canned_payloads(extra: dict[str, Any] | None = None) -> dict[str, Any]:
    """Canned RPC responses with the kinds of Tier-1 fields adapters must
    discard already present, so the tests can prove they never leak out."""
    payloads: dict[str, Any] = {
        "getinfo": {
            "id": "03" + "cd" * 32,
            "alias": "k-node",
            "version": "v23.11",
            "blockheight": 800_001,
            "network": "bitcoin",
            "num_peers": 2,
        },
        "listfunds": {
            "outputs": [
                {
                    "txid": "aa" * 32,
                    "output": 0,
                    "amount_msat": "200000msat",
                    "status": "confirmed",
                }
            ],
            "channels": [],
        },
        "listpeerchannels": {
            "channels": [
                {
                    "peer_id": "02" + "ab" * 32,
                    "peer_alias": "public-peer",
                    "peer_connected": True,
                    "private": False,
                    "channel_id": "ch-public",
                    "short_channel_id": "742x1x0",
                    "state": "CHANNELD_NORMAL",
                    "total_msat": "1000000000msat",
                    "to_us_msat": "400000000msat",
                    "their_amount_msat": "600000000msat",
                    "opener": "local",
                },
                {
                    "peer_id": "02" + "ef" * 32,
                    "peer_alias": "private-peer",
                    "peer_connected": True,
                    "private": True,
                    "channel_id": "ch-private",
                    "short_channel_id": "742x2x0",
                    "state": "CHANNELD_NORMAL",
                    "total_msat": "500000000msat",
                    "to_us_msat": "500000000msat",
                    "their_amount_msat": "0msat",
                    "opener": "remote",
                },
                {
                    "peer_id": "02" + "ce" * 32,
                    "peer_alias": "closed-coop-peer",
                    "peer_connected": False,
                    "private": False,
                    "channel_id": "ch-closed-coop",
                    "short_channel_id": "742x3x0",
                    "state": "CLOSINGD_COMPLETE",
                    "total_msat": "700000000msat",
                    "to_us_msat": "0msat",
                    "their_amount_msat": "0msat",
                    "opener": "local",
                    "closed_at": 1_700_000_090,
                },
                {
                    "peer_id": "02" + "cf" * 32,
                    "peer_alias": "closed-force-peer",
                    "peer_connected": False,
                    "private": False,
                    "channel_id": "ch-closed-force",
                    "short_channel_id": "742x4x0",
                    "state": "ONCHAIND_OUR_UNILATERAL",
                    "total_msat": "900000000msat",
                    "to_us_msat": "0msat",
                    "their_amount_msat": "0msat",
                    "opener": "local",
                    "closed_at": 1_700_000_100,
                },
            ]
        },
        "listforwards": {
            "forwards": [
                {
                    "in_channel": "111x1x0",
                    "out_channel": "742x1x0",
                    "fee_msat": "2000msat",
                    "in_msat": "52000msat",
                    "out_msat": "50000msat",
                    "status": "settled",
                    "received_time": 1_700_000_020,
                    "resolved_time": 1_700_000_030,
                    # Opsec: erring_node/failure_reason should be dropped.
                    "failcode": 0,
                    "failreason": "OK",
                    "erring_node": "02" + "ba" * 32,
                }
            ]
        },
        "listpays": {
            "pays": [
                {
                    "payment_hash": "22" * 32,
                    "amount_msat": "40000msat",
                    "amount_sent_msat": "40500msat",
                    "status": "complete",
                    "created_at": 1_700_000_040,
                    "completed_at": 1_700_000_041,
                    "destination": "02" + "dd" * 32,
                    # Opsec: preimage, bolt11, route hops must be discarded.
                    "preimage": "1f" * 32,
                    "bolt11": "lnbc1pjexample",
                    "route": [
                        {"id": "02" + "ee" * 32, "channel": "100x1x0"},
                        {"id": "02" + "ff" * 32, "channel": "200x1x0"},
                    ],
                }
            ]
        },
        "listinvoices": {
            "invoices": [
                {
                    "label": "subscription-2025-01",
                    "payment_hash": "33" * 32,
                    "amount_msat": "120000msat",
                    "amount_received_msat": "120000msat",
                    "status": "paid",
                    "paid_at": 1_700_000_050,
                    "description": "Consulting Jan invoice",
                    # Opsec: preimage / bolt11 / payment_secret must be
                    # discarded. Route hints in invoices you paid would
                    # leak someone else's private-channel peers.
                    "payment_preimage": "a1" * 32,
                    "payment_secret": "c1" * 32,
                    "bolt11": "lnbc1pjinvoice",
                    "routes": [
                        [
                            {
                                "pubkey": "02" + "11" * 32,
                                "short_channel_id": "999x1x0",
                            }
                        ]
                    ],
                }
            ]
        },
        "listtransactions": {"transactions": []},
        "bkpr-listincome": {
            "income_events": [
                {
                    "account": "742x1x0",
                    "tag": "routed",
                    "credit_msat": "1500msat",
                    "debit_msat": "0msat",
                    "currency": "bc",
                    "timestamp": 1_700_000_000,
                    "payment_id": "11" * 32,
                },
                {
                    "account": "wallet",
                    "tag": "invoice",
                    "credit_msat": "120000msat",
                    "debit_msat": "0msat",
                    "currency": "bc",
                    "timestamp": 1_700_000_050,
                    "payment_id": "33" * 32,
                    "description": "Consulting Jan invoice",
                },
            ]
        },
        "bkpr-listbalances": {
            "accounts": [
                {
                    "account": "742x1x0",
                    "peer_id": "02" + "ab" * 32,
                    "balances": [{"balance_msat": "400000000msat"}],
                }
            ]
        },
    }
    if extra:
        payloads.update(extra)
    return payloads


def _rpc(payloads: dict[str, Any]):
    def call(method: str, _args: Any = None) -> Any:
        return payloads.get(method, {})

    return call


class AdapterRegistrationTest(unittest.TestCase):
    def test_adapter_registers_itself_on_import(self) -> None:
        adapter = resolve_adapter("coreln")
        self.assertIsNotNone(adapter)
        self.assertIsInstance(adapter, CoreLightningAdapter)
        self.assertEqual(adapter.kind, "coreln")


class FetchNodeSnapshotTest(unittest.TestCase):
    def _snapshot(self, payloads: dict[str, Any] | None = None) -> NodeSnapshot:
        used = payloads or _canned_payloads()
        snapshot_blob = core_cln.fetch_core_lightning_snapshot(
            {"kind": "coreln", "name": "cln", "url": "cln://local"},
            rpc_call=_rpc(used),
        )
        return core_cln.build_node_snapshot(snapshot_blob, window_days=30)

    def test_fetch_node_snapshot_returns_typed_snapshot(self) -> None:
        snapshot = self._snapshot()
        self.assertIsInstance(snapshot, NodeSnapshot)
        self.assertEqual(snapshot.alias, "k-node")
        self.assertEqual(snapshot.pubkey, "03" + "cd" * 32)
        self.assertEqual(snapshot.network, "bitcoin")
        self.assertEqual(snapshot.implementation_version, "v23.11")
        self.assertEqual(snapshot.block_height, 800_001)
        self.assertEqual(len(snapshot.channels), 2)
        self.assertEqual(snapshot.invoice_count, 1)
        self.assertEqual(snapshot.paid_invoice_count, 1)
        self.assertEqual(snapshot.payment_count, 1)
        self.assertEqual(snapshot.completed_payment_count, 1)
        self.assertIsNotNone(snapshot.routing)
        self.assertEqual(snapshot.routing.forward_count, 1)

    def test_closed_channels_preserve_close_kind(self) -> None:
        snapshot = self._snapshot()
        self.assertEqual(len(snapshot.closed_channels), 2)
        closed_by_scid = {
            channel.short_channel_id: channel for channel in snapshot.closed_channels
        }

        cooperative = closed_by_scid["742x3x0"]
        self.assertEqual(cooperative.state, "closed")
        self.assertEqual(cooperative.close_kind, "cooperative")
        self.assertEqual(cooperative.closed_at, "2023-11-14T22:14:50Z")

        forced = closed_by_scid["742x4x0"]
        self.assertEqual(forced.state, "force_closed")
        self.assertEqual(forced.close_kind, "force")
        self.assertEqual(forced.closed_at, "2023-11-14T22:15:00Z")

    def test_private_channel_peer_pubkey_is_none(self) -> None:
        snapshot = self._snapshot()
        private_channel = next(
            channel for channel in snapshot.channels if channel.is_private
        )
        self.assertIsNone(private_channel.peer_pubkey)
        public_channel = next(
            channel for channel in snapshot.channels if not channel.is_private
        )
        self.assertEqual(public_channel.peer_pubkey, "02" + "ab" * 32)

    def test_private_channel_without_alias_uses_neutral_label(self) -> None:
        leaked_pubkey = "02" + "ef" * 32
        payloads = _canned_payloads()
        payloads["listpeerchannels"]["channels"][1].pop("peer_alias", None)

        snapshot = self._snapshot(payloads)
        private_channel = next(
            channel for channel in snapshot.channels if channel.is_private
        )

        self.assertIsNone(private_channel.peer_pubkey)
        self.assertEqual(private_channel.peer_alias, "private peer")
        self.assertNotIn(leaked_pubkey, private_channel.peer_alias)

        report = build_profitability_report(
            connection_id="w-1",
            connection_label="Merchant",
            connection_kind="coreln",
            snapshot=snapshot,
        ).to_envelope_payload()
        blob = str(report)
        self.assertNotIn(leaked_pubkey, blob)
        self.assertIn("private peer", blob)

    def test_preimages_bolt11_and_route_hops_never_reach_records(self) -> None:
        # Drive the persistence reshape so we can scan the full curated
        # record set, then assert none of the Tier-1 fields appear.
        snapshot_blob = core_cln.fetch_core_lightning_snapshot(
            {"kind": "coreln", "name": "cln", "url": "cln://local"},
            rpc_call=_rpc(_canned_payloads()),
        )
        records = core_cln.snapshot_records(snapshot_blob, "2026-05-18T12:00:00Z")
        forbidden_substrings = (
            "1f" * 32,  # pay preimage
            "a1" * 32,  # invoice preimage
            "c1" * 32,  # invoice payment_secret
            "lnbc1pjexample",
            "lnbc1pjinvoice",
            "02" + "ba" * 32,  # erring_node
            "02" + "11" * 32,  # invoice route hint pubkey
        )
        serialized = repr(records)
        for needle in forbidden_substrings:
            self.assertNotIn(needle, serialized, msg=f"leaked: {needle}")
        # Forwards in NodeSnapshot must not carry failure_reason either.
        snapshot = core_cln.build_node_snapshot(snapshot_blob, window_days=30)
        for forward in snapshot.forwards:
            self.assertIsNone(forward.failure_reason)

    def test_routed_income_event_does_not_create_wallet_transaction(self) -> None:
        # P1 fix #1: bkpr-listincome `routed` events must not become wallet
        # transactions (they're already in the routing aggregate).
        snapshot_blob = core_cln.fetch_core_lightning_snapshot(
            {"kind": "coreln", "name": "cln", "url": "cln://local"},
            rpc_call=_rpc(_canned_payloads()),
        )
        records = core_cln.snapshot_records(snapshot_blob, "2026-05-18T12:00:00Z")
        import_payloads = [
            payload
            for payload in (core_cln._record_to_import(record) for record in records)
            if payload is not None
        ]
        # The invoice income row and the completed outbound pay are imported as
        # wallet transactions; the routed forwarding event is NOT (it is already
        # in the routing aggregate — the P1 double-count guard).
        kinds = sorted(payload["kind"] for payload in import_payloads)
        self.assertEqual(kinds, ["cln_invoice", "cln_pay"])
        routed_payment_id = "11" * 32
        self.assertNotIn(
            routed_payment_id,
            {payload.get("payment_hash") for payload in import_payloads},
        )
        invoice_payload = next(p for p in import_payloads if p["kind"] == "cln_invoice")
        self.assertEqual(invoice_payload["confirmed_at"], "2023-11-14T22:14:10Z")
        pay_payload = next(p for p in import_payloads if p["kind"] == "cln_pay")
        self.assertEqual(pay_payload["direction"], "outbound")
        self.assertEqual(pay_payload["payment_hash"], "22" * 32)
        # And the routed event should not appear as a forward_day record's
        # source either — it should only contribute to the aggregate.
        forward_day_rows = [r for r in records if r["record_type"] == "forward_day"]
        self.assertEqual(len(forward_day_rows), 1)

    def test_channel_lifecycle_record_carries_funding_txid(self) -> None:
        from types import SimpleNamespace

        channel = {
            "channel_id": "ch-1",
            "short_channel_id": "742x1x0",
            "state": "CHANNELD_NORMAL",
            "peer_connected": True,
            "funding": {"txid": "aa" * 32, "outnum": 0},
            "opened_at": 1_700_000_000,
        }
        records = core_cln._channel_lifecycle_records(SimpleNamespace(channels=[channel]))
        self.assertEqual(len(records), 1)
        rec = records[0]
        self.assertEqual(rec["record_type"], "channel")
        self.assertEqual(rec["txid"], "aa" * 32)
        self.assertEqual(rec["outpoint"], "aa" * 32 + ":0")
        # A channel metadata record is NOT a wallet transaction.
        self.assertIsNone(core_cln._record_to_import(rec))

    def test_outbound_pay_promoted_with_principal_and_routing_fee(self) -> None:
        # The completed listpays row (amount_msat=40000, amount_sent_msat=40500)
        # becomes an outbound cln_pay: principal 40000 msat, routing fee 500 msat.
        from kassiber.msat import msat_to_btc

        snapshot_blob = core_cln.fetch_core_lightning_snapshot(
            {"kind": "coreln", "name": "cln", "url": "cln://local"},
            rpc_call=_rpc(_canned_payloads()),
        )
        records = core_cln.snapshot_records(snapshot_blob, "2026-05-18T12:00:00Z")
        pay_record = next(r for r in records if r["record_type"] == "pay")
        payload = core_cln._record_to_import(pay_record)
        self.assertIsNotNone(payload)
        assert payload is not None
        self.assertEqual(payload["kind"], "cln_pay")
        self.assertEqual(payload["direction"], "outbound")
        self.assertEqual(payload["amount"], msat_to_btc(40_000))
        self.assertEqual(payload["fee"], msat_to_btc(500))
        self.assertEqual(payload["payment_hash"], "22" * 32)
        self.assertEqual(payload["payment_hash_source"], "core_lightning")


class AdapterContractTest(unittest.TestCase):
    def test_adapter_rejects_missing_backend(self) -> None:
        adapter = CoreLightningAdapter()
        with self.assertRaises(AppError) as ctx:
            adapter.fetch_node_snapshot({"id": "w-1"}, None)
        self.assertEqual(ctx.exception.code, "validation")


class RpcAllowlistTest(unittest.TestCase):
    def test_pay_close_withdraw_are_rejected_by_allowlist(self) -> None:
        backend = {"kind": "coreln", "name": "cln", "url": "cln://local"}
        for method in ("pay", "close", "withdraw", "fundchannel", "delpay"):
            with self.assertRaises(AppError) as ctx:
                core_cln.call_core_lightning(backend, method)
            self.assertEqual(ctx.exception.code, "validation")

    def test_allowlist_contains_only_read_methods(self) -> None:
        for method in core_cln.CLN_ALLOWED_METHODS:
            self.assertTrue(
                method.startswith("get")
                or method.startswith("list")
                or method.startswith("bkpr-"),
                msg=f"non-read method allowed: {method}",
            )

    def test_commando_rune_is_passed_without_shell_placeholder_and_redacted(self) -> None:
        backend = {
            "kind": "coreln",
            "name": "cln",
            "url": "cln://commando",
            "token": "secret-rune-value",
            "commando_peer_id": "02" + "ab" * 32,
        }
        seen = {}

        def _fake_run(command, **_kwargs):
            seen["command"] = command
            return SimpleNamespace(returncode=1, stdout="", stderr="denied")

        with patch("kassiber.core.lightning.cln.subprocess.run", side_effect=_fake_run):
            with self.assertRaises(AppError) as ctx:
                core_cln.call_core_lightning(backend, "getinfo")

        command = seen["command"]
        self.assertNotIn("--commando-rune=${LIGHTNING_RUNE}", command)
        self.assertIn("--commando-rune=secret-rune-value", command)
        self.assertNotIn("secret-rune-value", str(ctx.exception.details))
        self.assertIn("<commando rune redacted>", str(ctx.exception.details))


class PersistenceIdempotenceTest(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.conn = open_db(Path(self._tmp.name) / "data")
        workspace = core_accounts.create_workspace(self.conn, "Personal")
        self.profile = core_accounts.create_profile(
            self.conn,
            workspace["id"],
            "Main",
            "USD",
            "FIFO",
            "generic",
            365,
        )
        core_accounts.create_backend(
            self.conn,
            "cln",
            "coreln",
            "cln://local",
            token="readonly-rune",
            config={"commando_peer_id": "02" + "ab" * 32},
        )
        created_wallet = core_wallets.create_wallet(
            self.conn,
            workspace["id"],
            self.profile["id"],
            "Routing node",
            "coreln",
            config={"backend": "cln"},
        )
        self.wallet = fetch_wallet_with_account(self.conn, created_wallet["id"])
        self.backend = get_db_backend(self.conn, "cln")

    def tearDown(self) -> None:
        self.conn.close()
        self._tmp.cleanup()

    def _hooks(self) -> core_imports.ImportCoordinatorHooks:
        return core_imports.ImportCoordinatorHooks(
            ensure_tag_row=lambda *_args, **_kwargs: None,
            invalidate_journals=invalidate_journals,
        )

    def test_balance_snapshot_is_daily_bucketed(self) -> None:
        # P1 fix #2: two syncs on the same day must produce one balance row
        # per account, not a fresh one per sync.
        rpc = _rpc(_canned_payloads())
        for _ in range(2):
            core_cln.sync_core_lightning_wallet(
                self.conn,
                self.profile,
                self.wallet,
                self.backend,
                self._hooks(),
                rpc_call=rpc,
            )
        rows = self.conn.execute(
            "SELECT COUNT(*) AS c FROM lightning_node_records"
            " WHERE record_type = 'balance_snapshot'"
        ).fetchone()
        self.assertEqual(rows["c"], 1)

    def test_no_raw_rpc_payload_persisted(self) -> None:
        rpc = _rpc(_canned_payloads())
        core_cln.sync_core_lightning_wallet(
            self.conn,
            self.profile,
            self.wallet,
            self.backend,
            self._hooks(),
            rpc_call=rpc,
        )
        # The opsec policy requires that raw RPC payloads never land on disk.
        rows = self.conn.execute(
            "SELECT raw_json FROM lightning_node_records"
        ).fetchall()
        self.assertTrue(rows, "expected at least one row")
        for row in rows:
            self.assertEqual(row["raw_json"], "{}")


class RebalanceCostFormulaTest(unittest.TestCase):
    def test_rebalance_fee_event_does_not_double_count_principal(self) -> None:
        # P1 fix #3: a rebalance_fee bookkeeper event's `amount_msat` IS the
        # fee. Summing principal + fee would double-count. The routing
        # summary should treat it as a single fee value.
        payloads = _canned_payloads(
            {
                "bkpr-listincome": {
                    "income_events": [
                        {
                            "account": "742x1x0",
                            "tag": "rebalance_fee",
                            "debit_msat": "3000msat",
                            "credit_msat": "0msat",
                            "timestamp": 1_700_000_010,
                        }
                    ]
                }
            }
        )
        snapshot_blob = core_cln.fetch_core_lightning_snapshot(
            {"kind": "coreln", "name": "cln", "url": "cln://local"},
            rpc_call=_rpc(payloads),
        )
        snapshot = core_cln.build_node_snapshot(snapshot_blob, window_days=30)
        self.assertIsNotNone(snapshot.routing)
        # 3000 msat = 3 sat (truncated). Principal is NOT added.
        self.assertEqual(snapshot.routing.rebalance_cost_sat, 3)

    def test_rebalance_fee_in_both_listpays_and_bookkeeper_counts_once(self) -> None:
        # M-2: prior implementation summed `fee` from completed
        # rebalance-tagged `listpays` rows AND ``bkpr-listincome
        # rebalance_fee`` events, double-counting the same 3-sat fee. The
        # bookkeeper view is canonical; listpays only contributes the count.
        payloads = _canned_payloads(
            {
                "listpays": {
                    "pays": [
                        {
                            "payment_hash": "55" * 32,
                            "amount_msat": "100000msat",
                            "amount_sent_msat": "103000msat",
                            "status": "complete",
                            "rebalance": True,
                            "created_at": 1_700_000_080,
                            "completed_at": 1_700_000_081,
                            "destination": "02" + "44" * 32,
                        }
                    ]
                },
                "bkpr-listincome": {
                    "income_events": [
                        {
                            "account": "742x1x0",
                            "tag": "rebalance_fee",
                            "debit_msat": "3000msat",
                            "credit_msat": "0msat",
                            "timestamp": 1_700_000_081,
                            "payment_id": "55" * 32,
                        }
                    ]
                },
            }
        )
        snapshot_blob = core_cln.fetch_core_lightning_snapshot(
            {"kind": "coreln", "name": "cln", "url": "cln://local"},
            rpc_call=_rpc(payloads),
        )
        snapshot = core_cln.build_node_snapshot(snapshot_blob, window_days=30)
        self.assertIsNotNone(snapshot.routing)
        # 3000 msat -> 3 sat, counted ONCE (not 6 like the pre-fix bug).
        self.assertEqual(snapshot.routing.rebalance_cost_sat, 3)
        self.assertEqual(snapshot.routing.rebalance_count, 1)


def _walk(value: Any):
    """Recursive iterator over every primitive value reachable from ``value``."""
    if isinstance(value, Mapping):
        for key, item in value.items():
            yield from _walk(key)
            yield from _walk(item)
    elif isinstance(value, (list, tuple, set, frozenset)):
        for item in value:
            yield from _walk(item)
    else:
        yield value


def _snapshot_to_walkable(snapshot: core_cln.CoreLightningSnapshot) -> dict[str, Any]:
    # frozen dataclass -> dict so the recursive walker sees every field.
    from dataclasses import asdict

    return asdict(snapshot)


class SnapshotObjectSanitizationTest(unittest.TestCase):
    """H-4: the snapshot object itself must never carry Tier-1 fields, even
    before the reshape helpers run. Earlier revisions stored raw RPC payloads
    on ``method_payloads`` and relied on reshape to strip sensitive fields —
    a future debug dump or accidental persistence path could leak them. The
    fetcher now sanitizes at the transport boundary."""

    def _build_snapshot(self) -> core_cln.CoreLightningSnapshot:
        return core_cln.fetch_core_lightning_snapshot(
            {"kind": "coreln", "name": "cln", "url": "cln://local"},
            rpc_call=_rpc(_canned_payloads()),
        )

    def test_snapshot_object_contains_no_raw_sensitive_fields(self) -> None:
        snapshot = self._build_snapshot()
        forbidden_strings = (
            "1f" * 32,  # pay preimage
            "a1" * 32,  # invoice preimage (payment_preimage)
            "c1" * 32,  # invoice payment_secret
            "lnbc1pjexample",  # pay bolt11
            "lnbc1pjinvoice",  # invoice bolt11
            "02" + "ba" * 32,  # erring_node
            "02" + "11" * 32,  # invoice route-hint pubkey
            "02" + "ee" * 32,  # listpays route hop pubkey
            "02" + "ff" * 32,  # listpays route hop pubkey
            "100x1x0",  # route hop channel
            "200x1x0",  # route hop channel
            "999x1x0",  # invoice route hint short channel id
        )
        forbidden_keys = {
            "payment_preimage",
            "preimage",
            "payment_secret",
            "bolt11",
            "route",
            "routes",
            "route_hints",
            "erring_node",
            "failcode",
            "failreason",
            "failure_reason",
            "failure_source_pubkey",
        }
        walkable = _snapshot_to_walkable(snapshot)

        leaked_keys: list[str] = []

        def _scan_keys(value: Any) -> None:
            if isinstance(value, Mapping):
                for key, item in value.items():
                    if isinstance(key, str) and key.lower() in forbidden_keys:
                        leaked_keys.append(key)
                    _scan_keys(item)
            elif isinstance(value, (list, tuple, set, frozenset)):
                for item in value:
                    _scan_keys(item)

        _scan_keys(walkable)
        self.assertEqual(leaked_keys, [], msg=f"forbidden keys present: {leaked_keys}")

        values = list(_walk(walkable))
        text_values = [value for value in values if isinstance(value, str)]
        serialized_blob = "\n".join(text_values)
        for needle in forbidden_strings:
            self.assertNotIn(
                needle,
                serialized_blob,
                msg=f"snapshot leaked sensitive substring {needle!r}",
            )

    def test_snapshot_typed_collections_replace_method_payloads(self) -> None:
        # The migration away from ``method_payloads`` is what makes the
        # leak-by-construction impossible. Pin the API shape so a future
        # refactor that reintroduces raw RPC blobs trips this test.
        snapshot = self._build_snapshot()
        self.assertFalse(hasattr(snapshot, "method_payloads"))
        self.assertIsInstance(snapshot.channels, tuple)
        self.assertIsInstance(snapshot.forwards, tuple)
        self.assertIsInstance(snapshot.pays, tuple)
        self.assertIsInstance(snapshot.invoices, tuple)
        self.assertIsInstance(snapshot.income_events, tuple)
        self.assertIsInstance(snapshot.balance_accounts, tuple)
        self.assertIsInstance(snapshot.funds_outputs, tuple)


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
