"""Tests for the LND adapter (read-only Lightning scaffold implementation).

Pin the contract the adapter exposes to ``resolve_adapter("lnd")``:

- Registers itself on import.
- ``fetch_node_snapshot`` shapes a canned LND REST response into a
  :class:`NodeSnapshot` with the camelCase JSON the desktop reads.
- Private channels surface with ``peer_pubkey=None`` (opsec policy).
- Preimages, encoded bolt11 strings, route hops, and
  ``failure_source_pubkey`` are stripped before any payload reaches the
  scaffold types (opsec policy).
- The ``_ssl_context`` TLS toggle reads from a DB-resolved backend row
  (P1 regression — the previous implementation looked at a non-existent
  ``backend['config']`` key).
"""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from typing import Any, Mapping
from unittest.mock import patch

from kassiber.core import accounts as core_accounts
from kassiber.core.lightning import (
    NodeSnapshot,
    register_adapter,
    resolve_adapter,
    unregister_adapter,
)
from kassiber.core.lightning import lnd as core_lnd
from kassiber.db import open_db


CANNED_GETINFO = {
    "identity_pubkey": "02" + "aa" * 32,
    "alias": "kassiber-test-node",
    "version": "0.18.0-beta",
    "num_peers": 7,
    "block_height": 800_001,
    "best_header_timestamp": 1_710_000_000,
    "chains": [{"chain": "bitcoin", "network": "mainnet"}],
}

CANNED_BALANCE_BLOCKCHAIN = {"confirmed_balance": "500000"}

CANNED_BALANCE_CHANNELS = {
    "local_balance": {"sat": "700000"},
    "remote_balance": {"sat": "300000"},
}

CANNED_FEES = {
    "channel_fees": [
        {"chan_id": "111", "base_fee_msat": "1000", "fee_per_mil": "200"},
        {"chan_id": "222", "base_fee_msat": "500", "fee_per_mil": "100"},
    ]
}

CANNED_OPEN_CHANNELS = {
    "channels": [
        {
            "active": True,
            "chan_id": "111",
            "channel_point": ("aa" * 32) + ":0",
            "remote_pubkey": "03" + "bb" * 32,
            "peer_alias": "PublicPeer",
            "capacity": "1000000",
            "local_balance": "700000",
            "remote_balance": "300000",
            "commit_fee": "250",
            "private": False,
            "initiator": True,
        },
        {
            "active": True,
            "chan_id": "222",
            "channel_point": ("bb" * 32) + ":1",
            # Even though gossip might surface it, opsec policy says drop
            # it for private channels.
            "remote_pubkey": "03" + "cc" * 32,
            "peer_alias": "PrivatePeer",
            "capacity": "500000",
            "local_balance": "200000",
            "remote_balance": "300000",
            "commit_fee": "150",
            "private": True,
            "initiator": False,
        },
    ]
}

CANNED_CLOSED_CHANNELS = {"channels": []}

CANNED_FORWARDS = {
    "last_offset_index": "1",
    "forwarding_events": [
        {
            "timestamp": "1709999000",
            "timestamp_ns": "1709999000000000000",
            "chan_id_in": "111",
            "chan_id_out": "222",
            "amt_in_msat": "1000000",
            "amt_out_msat": "990000",
            "fee_msat": "10000",
        }
    ],
}

CANNED_PAYMENTS = {
    "last_index_offset": "1",
    "payments": [
        {
            "payment_index": "1",
            "payment_hash": "deadbeef",
            "creation_date": "1709990000",
            "creation_time_ns": "1709990000000000000",
            "status": "SUCCEEDED",
            "value_msat": "1000000",
            "fee_msat": "2000",
            "payment_preimage": "PREIMAGE_THAT_SHOULD_NEVER_LEAK",
            "payment_request": "lnbc10n1pj...ENCODED",
            "htlcs": [
                {
                    "status": "SUCCEEDED",
                    "route": {
                        "total_amt_msat": "1000000",
                        "total_fees_msat": "2000",
                        "hops": [
                            {"chan_id": "111", "pub_key": "PEER1"},
                            {"chan_id": "222", "pub_key": "PEER2"},
                        ],
                    },
                    "failure_source_pubkey": "SOURCE_PUBKEY_SHOULD_BE_DROPPED",
                }
            ],
        }
    ],
}

CANNED_INVOICES = {
    "last_index_offset": "1",
    "invoices": [
        {
            "add_index": "1",
            "r_hash": "INVOICE_R_HASH",
            "r_preimage": "INVOICE_PREIMAGE_NEVER_LEAK",
            "payment_request": "lnbc10n1pj...ENCODED",
            "payment_addr": "PAYMENT_SECRET_NEVER_LEAK",
            "creation_date": "1709980000",
            "settle_date": "1709980050",
            "settled": True,
            "value_msat": "3000000",
            "amt_paid_msat": "3000000",
            "memo": "Consulting invoice",
            "route_hints": [{"hop_hints": [{"chan_id": "private-hint"}]}],
        }
    ],
}


class _FakeLndRestClient:
    """Captures requests and serves canned responses for the adapter tests."""

    def __init__(self, backend: Mapping[str, Any]):
        self.backend = backend
        self.requests: list[tuple[str, str, Any]] = []
        self.responses: dict[tuple[str, str], dict[str, Any]] = {
            ("GET", "/v1/getinfo"): dict(CANNED_GETINFO),
            ("GET", "/v1/balance/blockchain"): dict(CANNED_BALANCE_BLOCKCHAIN),
            ("GET", "/v1/balance/channels"): dict(CANNED_BALANCE_CHANNELS),
            ("GET", "/v1/fees"): dict(CANNED_FEES),
            ("GET", "/v1/channels"): dict(CANNED_OPEN_CHANNELS),
            ("GET", "/v1/channels/closed"): dict(CANNED_CLOSED_CHANNELS),
            ("GET", "/v1/payments"): dict(CANNED_PAYMENTS),
            ("GET", "/v1/invoices"): dict(CANNED_INVOICES),
            ("POST", "/v1/switch"): dict(CANNED_FORWARDS),
        }

    # ---- shared instance store so the test can inspect post-construction
    instances: list["_FakeLndRestClient"] = []

    def __init_subclass__(cls, **kw):
        super().__init_subclass__(**kw)

    def _record(self, method: str, path: str, payload: Any) -> dict[str, Any]:
        self.requests.append((method, path, payload))
        return dict(self.responses.get((method, path), {}))

    def get(
        self, path: str, *, params: Mapping[str, Any] | None = None
    ) -> dict[str, Any]:
        return self._record("GET", path, dict(params or {}))

    def post(
        self, path: str, payload: Mapping[str, Any] | None = None
    ) -> dict[str, Any]:
        return self._record("POST", path, dict(payload or {}))


def _make_fake_client_factory() -> tuple[
    list[_FakeLndRestClient], type[_FakeLndRestClient]
]:
    """Return a (instances, factory) pair that records every constructed client."""

    instances: list[_FakeLndRestClient] = []

    class _Factory(_FakeLndRestClient):
        def __init__(self, backend: Mapping[str, Any]):
            super().__init__(backend)
            instances.append(self)

    return instances, _Factory


BACKEND = {
    "name": "node",
    "kind": "lnd",
    "url": "https://127.0.0.1:8080",
    "token": "00aa",
    "certificate": "/tmp/tls.cert",
}


class LndAdapterRegistrationTest(unittest.TestCase):
    """Confirm the side-effect import wires the adapter into the shared
    registry. Other suites in the project intentionally swap the registry
    entry, so this test re-imports the module to recreate the
    registration before asserting — that's the contract we ship: simply
    importing :mod:`kassiber.core.lightning.lnd` registers the
    adapter."""

    def setUp(self) -> None:
        import importlib

        importlib.reload(core_lnd)

    def test_registers_itself_under_kind_lnd_on_import(self) -> None:
        adapter = resolve_adapter("lnd")
        self.assertIsNotNone(adapter)
        self.assertEqual(adapter.kind, "lnd")

    def test_registered_adapter_is_LndAdapter_instance(self) -> None:
        adapter = resolve_adapter("lnd")
        self.assertIsInstance(adapter, core_lnd.LndAdapter)


class LndAdapterFetchSnapshotTest(unittest.TestCase):
    def setUp(self) -> None:
        # Snapshot the adapter registered by import-time `register_adapter`
        # so monkey-patching the client doesn't leak across tests.
        self._original_adapter = resolve_adapter("lnd")

    def tearDown(self) -> None:
        if self._original_adapter is not None:
            register_adapter("lnd", self._original_adapter)

    def test_fetch_node_snapshot_returns_NodeSnapshot_with_camel_case_payload(
        self,
    ) -> None:
        adapter = core_lnd.LndAdapter()
        instances, factory = _make_fake_client_factory()
        with patch.object(core_lnd, "LndRestClient", factory):
            snapshot = adapter.fetch_node_snapshot(
                {"id": "w-1", "label": "Home", "kind": "lnd"},
                dict(BACKEND),
                window_days=30,
            )
        self.assertIsInstance(snapshot, NodeSnapshot)
        self.assertEqual(snapshot.alias, "kassiber-test-node")
        self.assertEqual(snapshot.pubkey, CANNED_GETINFO["identity_pubkey"])
        self.assertEqual(snapshot.network, "mainnet")
        self.assertEqual(snapshot.peer_count, 7)
        self.assertEqual(snapshot.onchain_balance_sat, 500_000)
        self.assertEqual(snapshot.total_local_balance_sat, 700_000)
        self.assertEqual(snapshot.total_remote_balance_sat, 300_000)
        self.assertEqual(len(snapshot.channels), 2)
        self.assertEqual(len(snapshot.closed_channels), 0)
        self.assertEqual(len(snapshot.forwards), 1)
        self.assertEqual(snapshot.invoice_count, 1)
        self.assertEqual(snapshot.paid_invoice_count, 1)
        self.assertEqual(snapshot.expired_invoice_count, 0)
        self.assertEqual(snapshot.payment_count, 1)
        self.assertEqual(snapshot.completed_payment_count, 1)
        self.assertEqual(snapshot.failed_payment_count, 0)
        self.assertIsNotNone(snapshot.routing)
        self.assertEqual(snapshot.routing.routing_revenue_sat, 10)  # 10000 msat
        self.assertEqual(snapshot.routing.payment_count, 1)
        # Adapter requested the canned endpoints
        request_paths = {(req[0], req[1]) for req in instances[0].requests}
        for required in (
            ("GET", "/v1/getinfo"),
            ("GET", "/v1/channels"),
            ("GET", "/v1/channels/closed"),
            ("GET", "/v1/balance/blockchain"),
            ("GET", "/v1/balance/channels"),
            ("GET", "/v1/fees"),
            ("POST", "/v1/switch"),
            ("GET", "/v1/payments"),
            ("GET", "/v1/invoices"),
        ):
            self.assertIn(required, request_paths)

    def test_private_channel_drops_peer_pubkey(self) -> None:
        adapter = core_lnd.LndAdapter()
        _, factory = _make_fake_client_factory()
        with patch.object(core_lnd, "LndRestClient", factory):
            snapshot = adapter.fetch_node_snapshot(
                {"id": "w-1", "label": "Home", "kind": "lnd"},
                dict(BACKEND),
            )
        by_id = {channel.id: channel for channel in snapshot.channels}
        public = by_id["111"]
        private = by_id["222"]
        self.assertFalse(public.is_private)
        self.assertEqual(public.peer_pubkey, CANNED_OPEN_CHANNELS["channels"][0]["remote_pubkey"])
        self.assertTrue(private.is_private)
        # Opsec policy: private channel surfaces peer_pubkey=None.
        self.assertIsNone(private.peer_pubkey)

    def test_private_channel_with_pubkey_alias_falls_back_to_neutral_label(
        self,
    ) -> None:
        """Regression for the Codex H-3 finding: when LND omits
        ``peer_alias`` for a private channel, the previous fallback
        ``row.get("peer_alias") or remote_pubkey or "unknown"`` serialized
        the peer pubkey under ``peerAlias``, bypassing the ``peer_pubkey``
        opsec guard. The fallback for private channels must be a neutral
        placeholder."""

        leaked_pubkey = "03" + "cc" * 32
        row = {
            "active": True,
            "chan_id": "999",
            "channel_point": ("ff" * 32) + ":0",
            "remote_pubkey": leaked_pubkey,
            # LND omits the alias (or returns an empty string) — this
            # is the leak scenario the fix targets.
            "peer_alias": "",
            "capacity": "500000",
            "local_balance": "200000",
            "remote_balance": "300000",
            "private": True,
            "initiator": False,
        }
        channel = core_lnd._map_channel(row, closed=False)
        # Both the structured pubkey field and the alias must not leak
        # the remote pubkey for a private channel.
        self.assertIsNone(channel.peer_pubkey)
        self.assertNotIn(leaked_pubkey, channel.peer_alias)
        self.assertEqual(channel.peer_alias, "private peer")
        # Defense in depth: serialize through the scaffold's dict shape
        # and confirm the pubkey is nowhere in the payload — even a
        # transitive leak through `peerAlias` would surface here.
        from kassiber.core.lightning import snapshot_to_dict
        from kassiber.core.lightning.types import (
            NodeRoutingSnapshot,
            NodeSnapshot,
        )

        snapshot = NodeSnapshot(
            alias="leaky-node",
            pubkey="02" + "aa" * 32,
            network="mainnet",
            implementation_version=None,
            peer_count=0,
            block_height=None,
            onchain_balance_sat=0,
            total_local_balance_sat=0,
            total_remote_balance_sat=0,
            total_capacity_sat=0,
            channels=(channel,),
            closed_channels=(),
            routing=NodeRoutingSnapshot(
                window_label="Last 30 days",
                routing_revenue_sat=0,
                payment_cost_sat=0,
                rebalance_cost_sat=0,
                onchain_cost_sat=0,
                net_profit_sat=0,
                forward_count=0,
                payment_count=0,
                rebalance_count=0,
            ),
            forwards=(),
        )
        self.assertNotIn(leaked_pubkey, json.dumps(snapshot_to_dict(snapshot)))

    def test_private_channel_can_use_graph_alias_without_pubkey_leak(
        self,
    ) -> None:
        remote_pubkey = "03" + "dd" * 32
        row = {
            "active": True,
            "chan_id": "333",
            "channel_point": ("ee" * 32) + ":0",
            "remote_pubkey": remote_pubkey,
            "peer_alias": "",
            "capacity": "500000",
            "local_balance": "200000",
            "remote_balance": "300000",
            "private": True,
            "initiator": False,
        }
        client = _FakeLndRestClient(BACKEND)
        client.responses[("GET", f"/v1/graph/node/{remote_pubkey}")] = {
            "node": {"alias": "KnownPrivatePeer"}
        }

        aliases = core_lnd._peer_alias_lookup(client, [row])
        channel = core_lnd._map_channel(
            row,
            closed=False,
            peer_alias_lookup=aliases,
        )

        self.assertEqual(channel.peer_alias, "KnownPrivatePeer")
        self.assertIsNone(channel.peer_pubkey)
        self.assertNotIn(remote_pubkey, channel.peer_alias)

    def test_private_channel_with_pubkey_shaped_alias_keeps_alias(self) -> None:
        """If LND surfaces an alias for a private channel — even one that
        happens to look like a pubkey — we keep it. The opsec rule is to
        not leak the pubkey via FALLBACK, not to second-guess the alias
        the peer chose to expose. ``peer_pubkey`` is still ``None``."""

        # An alias that happens to resemble a hex pubkey. The adapter
        # treats it as the peer's chosen identity, NOT a leak source.
        pubkey_shaped_alias = "03" + "ee" * 32
        row = {
            "active": True,
            "chan_id": "888",
            "channel_point": ("ee" * 32) + ":2",
            "remote_pubkey": "03" + "dd" * 32,
            "peer_alias": pubkey_shaped_alias,
            "capacity": "400000",
            "local_balance": "100000",
            "remote_balance": "300000",
            "private": True,
            "initiator": True,
        }
        channel = core_lnd._map_channel(row, closed=False)
        self.assertIsNone(channel.peer_pubkey)
        self.assertEqual(channel.peer_alias, pubkey_shaped_alias)

    def test_payment_sanitization_drops_preimage_bolt11_and_failure_source(
        self,
    ) -> None:
        adapter = core_lnd.LndAdapter()
        _, factory = _make_fake_client_factory()
        with patch.object(core_lnd, "LndRestClient", factory):
            snapshot = adapter.fetch_node_snapshot(
                {"id": "w-1", "label": "Home", "kind": "lnd"},
                dict(BACKEND),
            )
        # NodeSnapshot has no preimage / payment_request fields by
        # construction. Re-serialize through json and check the text
        # contains none of the secret markers.
        from kassiber.core.lightning import snapshot_to_dict

        blob = json.dumps(snapshot_to_dict(snapshot))
        self.assertNotIn("PREIMAGE_THAT_SHOULD_NEVER_LEAK", blob)
        self.assertNotIn("ENCODED", blob)
        self.assertNotIn("SOURCE_PUBKEY_SHOULD_BE_DROPPED", blob)
        # And the per-test sanitizer drops the same fields off raw rows.
        cleaned = core_lnd._sanitize_payment(CANNED_PAYMENTS["payments"][0])
        self.assertNotIn("payment_preimage", cleaned)
        self.assertNotIn("payment_request", cleaned)
        self.assertNotIn("failure_source_pubkey", cleaned)
        first_htlc = cleaned["htlcs"][0]
        self.assertNotIn("hops", first_htlc["route"])
        self.assertNotIn("failure_source_pubkey", first_htlc)

    def test_invoice_sanitization_drops_preimage_bolt11_and_route_hints(
        self,
    ) -> None:
        adapter = core_lnd.LndAdapter()
        _, factory = _make_fake_client_factory()
        with patch.object(core_lnd, "LndRestClient", factory):
            adapter.fetch_node_snapshot(
                {"id": "w-1", "label": "Home", "kind": "lnd"},
                dict(BACKEND),
            )
        cleaned = core_lnd._sanitize_invoice(CANNED_INVOICES["invoices"][0])
        self.assertNotIn("r_preimage", cleaned)
        self.assertNotIn("payment_request", cleaned)
        self.assertNotIn("payment_addr", cleaned)
        self.assertNotIn("route_hints", cleaned)
        # The decoded fields that DO have tax value survive.
        self.assertEqual(cleaned["r_hash"], "INVOICE_R_HASH")
        self.assertEqual(cleaned["value_msat"], "3000000")
        self.assertEqual(cleaned["memo"], "Consulting invoice")


class LndAdapterSslContextTest(unittest.TestCase):
    """Regression for the P1 fix: `_ssl_context` must read TLS settings
    via :func:`backend_value` so DB-resolved backend rows (where
    `_backend_row_to_dict` flattens `config_json` to top-level keys) are
    honored. The previous implementation looked at `backend['config']`
    which is only present on synthetic dicts."""

    def test_ssl_context_honors_insecure_flag_on_db_resolved_backend(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            data_root = Path(tmp) / "data"
            conn = open_db(data_root)
            core_accounts.create_workspace(conn, "Main")
            core_accounts.create_profile(
                conn,
                "Main",
                "Default",
                "USD",
                "FIFO",
                "generic",
                365,
            )
            core_accounts.create_backend(
                conn,
                "node",
                "lnd",
                "https://127.0.0.1:8080",
                chain="bitcoin",
                network="main",
                token="00aa",
                config={"insecure": True},
            )
            from kassiber.backends import get_db_backend

            backend_row = get_db_backend(conn, "node")
            # The DB row flattens config_json: `insecure` should be a
            # top-level key. This is the precondition the fix targets.
            self.assertTrue(
                backend_row.get("insecure"),
                "Precondition: DB-resolved backend exposes insecure at top level",
            )
            self.assertNotIn(
                "config",
                backend_row,
                "Precondition: DB-resolved backend has no nested 'config' dict",
            )
            context = core_lnd._ssl_context(backend_row)
            # An unverified context still exists; the test only needs to
            # confirm we actually picked up the flag and didn't fall
            # through to the default verifier.
            self.assertIsNotNone(context)
            self.assertFalse(context.check_hostname)

    def test_ssl_context_returns_none_when_no_cert_and_not_insecure(
        self,
    ) -> None:
        backend = {
            "name": "node",
            "kind": "lnd",
            "url": "https://127.0.0.1:8080",
            "token": "00aa",
        }
        self.assertIsNone(core_lnd._ssl_context(backend))


class LndAdapterBackendValidationTest(unittest.TestCase):
    def test_missing_backend_raises_config_error(self) -> None:
        adapter = core_lnd.LndAdapter()
        from kassiber.errors import AppError

        with self.assertRaises(AppError) as ctx:
            adapter.fetch_node_snapshot(
                {"id": "w-1", "label": "Home", "kind": "lnd"},
                None,
            )
        self.assertEqual(ctx.exception.code, "config_error")

    def test_wrong_backend_kind_raises_validation(self) -> None:
        adapter = core_lnd.LndAdapter()
        from kassiber.errors import AppError

        with self.assertRaises(AppError) as ctx:
            adapter.fetch_node_snapshot(
                {"id": "w-1", "label": "Home", "kind": "lnd"},
                {"name": "node", "kind": "btcpay"},
            )
        self.assertEqual(ctx.exception.code, "validation")


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
