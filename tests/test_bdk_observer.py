from __future__ import annotations

from collections import defaultdict
import json
import os
from pathlib import Path
import unittest
from unittest import mock

import bdkpython as bdk
from embit import bip32, bip39

from kassiber.core import sync as core_sync
from kassiber.core import sync_backends
from kassiber.core.chain_observer.bdk import (
    BdkObserver,
    _bdk_proxy_url,
    _electrum_confirmation_rebuild_needed,
    _electrum_header_hash,
    bdk_branches_for_identity,
    bdk_compatibility_reason,
)
from kassiber.core.chain_observer.bdk_persistence import (
    BDK_CHANGESET_SCHEMA_VERSION,
    SqlCipherBdkPersistence,
    deserialize_changeset,
    serialize_changeset,
)
from kassiber.core.chain_observer.identity import identities_for_wallet
from kassiber.core.imports import _transaction_merge_updates, normalize_import_record
from kassiber.errors import AppError
from kassiber.wallet_descriptors import load_descriptor_plan
from tests.test_cli_smoke import _sample_descriptor_pair
from tests.test_wallet_descriptors import _account_descriptor, _wsh_multisig


ROOT = Path(__file__).resolve().parents[1]
MNEMONIC = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"


def _descriptor_wallet():
    receive, change, *_ = _sample_descriptor_pair()
    config = {
        "descriptor": receive,
        "change_descriptor": change,
        "chain": "bitcoin",
        "network": "main",
        "gap_limit": 20,
    }
    wallet = {
        "id": "wallet-1",
        "workspace_id": "workspace-1",
        "profile_id": "profile-1",
        "config_json": json.dumps(config),
    }
    return wallet, load_descriptor_plan(config)


class BdkDependencyContractTest(unittest.TestCase):
    def _discovery(self, *, backend=None, partial_targets=False):
        wallet, plan = _descriptor_wallet()
        targets = sync_backends._offline_descriptor_targets(plan, {})
        if partial_targets:
            targets = targets[1:]
        resolved_backend = backend or {
            "name": "native",
            "kind": "esplora",
            "url": "https://example.invalid",
        }
        state = core_sync.WalletSyncState(
            chain="bitcoin",
            network="main",
            descriptor_plan=plan,
            policy_asset_id="",
            targets=targets,
            tracked_scripts={
                target["script_pubkey"]: target
                for target in targets
                if target.get("script_pubkey")
            },
            history_cache={},
            checkpoint={},
        )
        return wallet, core_sync.WalletBackendDiscovery(
            backend=resolved_backend,
            sync_state=state,
            kind=str(resolved_backend["kind"]),
            started=0.0,
            force_full=False,
        )

    def test_supported_route_never_calls_compatibility_adapter(self):
        wallet, discovery = self._discovery()
        compatibility = mock.Mock(side_effect=AssertionError("legacy adapter called"))
        prepared = object()
        with mock.patch.object(
            sync_backends,
            "SYNC_BACKEND_ADAPTERS",
            {"esplora": compatibility},
        ), mock.patch(
            "kassiber.core.chain_observer.prepare_observer_update",
            return_value=prepared,
        ):
            fetched = sync_backends.prepare_dependency_observer_fetch(
                mock.Mock(), {}, wallet, discovery
            )
        self.assertEqual(fetched.adapter_meta["observer_route"], "bdk")
        self.assertEqual(fetched.normalized_records, ())
        self.assertEqual(fetched.observer_updates, (prepared,))
        compatibility.assert_not_called()

    def test_bdk_failure_is_not_retried_through_compatibility_adapter(self):
        wallet, discovery = self._discovery()
        compatibility = mock.Mock(side_effect=AssertionError("legacy adapter called"))
        with mock.patch.object(
            sync_backends,
            "SYNC_BACKEND_ADAPTERS",
            {"esplora": compatibility},
        ), mock.patch(
            "kassiber.core.chain_observer.prepare_observer_update",
            side_effect=AppError("native failure", code="backend_sync_failed"),
        ):
            with self.assertRaises(AppError) as raised:
                sync_backends.prepare_dependency_observer_fetch(
                    mock.Mock(), {}, wallet, discovery
                )
        self.assertEqual(raised.exception.code, "backend_sync_failed")
        compatibility.assert_not_called()

    def test_named_compatibility_routes_are_selected_before_network_access(self):
        cases = {
            "custom_ca": (
                {
                    "name": "custom-ca",
                    "kind": "electrum",
                    "url": "ssl://example.invalid:50002",
                    "certificate": "/private/ca.pem",
                },
                False,
            ),
            "source_overlap_partial_descriptor": (
                {
                    "name": "overlap",
                    "kind": "esplora",
                    "url": "https://example.invalid",
                },
                True,
            ),
        }
        for reason, (backend, partial_targets) in cases.items():
            with self.subTest(reason=reason):
                wallet, discovery = self._discovery(
                    backend=backend,
                    partial_targets=partial_targets,
                )
                compatibility = mock.Mock(return_value=([{"txid": "11" * 32}], {}))
                with mock.patch.object(
                    sync_backends,
                    "COMPATIBILITY_SYNC_BACKEND_ADAPTERS",
                    {backend["kind"]: compatibility},
                ), mock.patch(
                    "kassiber.core.chain_observer.prepare_observer_update"
                ) as dependency_prepare:
                    fetched = sync_backends.prepare_dependency_observer_fetch(
                        mock.Mock(), {}, wallet, discovery
                    )
                self.assertEqual(
                    fetched.adapter_meta["observer_compatibility_reason"],
                    reason,
                )
                compatibility.assert_called_once()
                dependency_prepare.assert_not_called()

    def test_supported_descriptor_families_construct_watch_only_bdk_wallets(self):
        root = bip32.HDKey.from_seed(bip39.mnemonic_to_seed(MNEMONIC))
        fingerprint = root.my_fingerprint.hex()
        xpub = root.derive("m/84h/0h/0h").to_public().to_base58()
        receive, change, *_ = _sample_descriptor_pair()
        cases = {
            "bip44": {
                "descriptor": _account_descriptor(44, "pkh(", ")"),
            },
            "bip49": {
                "descriptor": _account_descriptor(49, "sh(wpkh(", "))"),
            },
            "bip84": {
                "descriptor": _account_descriptor(84, "wpkh(", ")"),
            },
            "bip86": {
                "descriptor": _account_descriptor(86, "tr(", ")"),
            },
            "fixed": {
                "descriptor": receive.replace("/0/*", "/0/5"),
                "synthesize_change": False,
            },
            "canonical_multipath": {
                "descriptor": f"wpkh([{fingerprint}/84h/0h/0h]{xpub}/<0;1>/*)",
            },
            "multisig": {
                "descriptor": _wsh_multisig(0),
                "change_descriptor": _wsh_multisig(1),
            },
            "samourai_child": {
                "descriptor": receive,
                "change_descriptor": change,
                "samourai": {
                    "role": "child",
                    "parent_wallet_id": "samourai-parent",
                    "section": "postmix",
                    "script_type": "p2wpkh",
                    "root_path": "m/84'/0'/0'",
                },
            },
        }
        for name, partial in cases.items():
            with self.subTest(name=name):
                config = {
                    "chain": "bitcoin",
                    "network": "main",
                    "gap_limit": 20,
                    **partial,
                }
                plan = load_descriptor_plan(config)
                wallet = {
                    "id": f"wallet-{name}",
                    "workspace_id": "workspace-1",
                    "profile_id": "profile-1",
                    "kind": "descriptor",
                    "config_json": json.dumps(config),
                }
                identities = identities_for_wallet(wallet, observer_kind="bdk")
                self.assertTrue(identities)
                for identity in identities:
                    observer = BdkObserver(
                        identity=identity,
                        backend={
                            "name": "offline-contract",
                            "kind": "esplora",
                            "url": "https://example.invalid",
                        },
                        branches=bdk_branches_for_identity(plan, identity),
                        gap_limit=20,
                    )
                    dependency_wallet, _persister = observer._wallet_from_state(None)
                    self.assertEqual(dependency_wallet.latest_checkpoint().height, 0)

    def test_multi_script_xpub_builds_one_bdk_wallet_per_script_family(self):
        root = bip32.HDKey.from_seed(bip39.mnemonic_to_seed(MNEMONIC))
        xpub = root.derive("m/84h/0h/0h").to_public().to_base58()
        config = {
            "chain": "bitcoin",
            "network": "main",
            "xpub": xpub,
            "script_types": ["p2pkh", "p2sh-p2wpkh", "p2wpkh", "p2tr"],
        }
        plan = load_descriptor_plan(config)
        wallet = {
            "id": "wallet-multi-script",
            "workspace_id": "workspace-1",
            "profile_id": "profile-1",
            "kind": "xpub",
            "config_json": json.dumps(config),
        }
        identities = identities_for_wallet(wallet, observer_kind="bdk")
        self.assertEqual(
            [identity.source_key for identity in identities],
            ["xpub:p2pkh", "xpub:p2sh-p2wpkh", "xpub:p2tr", "xpub:p2wpkh"],
        )
        for identity in identities:
            branches = bdk_branches_for_identity(plan, identity)
            self.assertEqual(len(branches), 2)
            observer = BdkObserver(
                identity=identity,
                backend={
                    "name": "offline-contract",
                    "kind": "electrum",
                    "url": "ssl://example.invalid:50002",
                },
                branches=branches,
                gap_limit=20,
            )
            dependency_wallet, _persister = observer._wallet_from_state(None)
            self.assertEqual(dependency_wallet.latest_checkpoint().height, 0)

    def test_electrum_header_hash_uses_bitcoin_wire_order(self):
        header = bdk.Header(
            version=1,
            prev_blockhash=bdk.BlockHash.from_bytes(bytes(32)),
            merkle_root=bdk.TxMerkleNode.from_bytes(
                bytes.fromhex(
                    "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b"
                )[::-1]
            ),
            time=1231006505,
            bits=486604799,
            nonce=2083236893,
        )
        self.assertEqual(
            _electrum_header_hash(header),
            "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f",
        )

    def test_lagging_backend_fails_before_reorg_rebuild(self):
        observer = object.__new__(BdkObserver)

        observer.backend = {"kind": "esplora"}
        esplora = mock.Mock()
        esplora.get_height.return_value = 9
        with self.assertRaises(AppError) as raised:
            observer._remote_block_hash(esplora, 10)
        self.assertEqual(raised.exception.code, "backend_tip_behind")
        self.assertTrue(raised.exception.retryable)
        self.assertEqual(raised.exception.details["backend_height"], 9)
        esplora.get_block_hash.assert_not_called()

        observer.backend = {"kind": "electrum"}
        electrum = mock.Mock()
        electrum.block_headers_subscribe.return_value.height = 8
        with self.assertRaises(AppError) as raised:
            observer._remote_block_hash(electrum, 10)
        self.assertEqual(raised.exception.code, "backend_tip_behind")
        self.assertTrue(raised.exception.retryable)
        electrum.block_header.assert_not_called()

    def test_electrum_rebuild_is_limited_to_advanced_tip_with_stale_mempool_position(self):
        wallet = mock.Mock()
        wallet.latest_checkpoint.return_value.height = 8
        confirmed = mock.Mock()
        confirmed.chain_position.is_confirmed.return_value = True
        mempool = mock.Mock()
        mempool.chain_position.is_confirmed.return_value = False

        wallet.transactions.return_value = [mempool]
        self.assertTrue(_electrum_confirmation_rebuild_needed(wallet, 7))
        self.assertFalse(_electrum_confirmation_rebuild_needed(wallet, 8))
        self.assertFalse(_electrum_confirmation_rebuild_needed(wallet, None))

        wallet.transactions.return_value = [confirmed]
        self.assertFalse(_electrum_confirmation_rebuild_needed(wallet, 7))

    def test_authoritative_observer_can_demote_confirmation(self):
        existing = defaultdict(
            lambda: None,
            confirmed_at="2026-01-01T00:00:00Z",
            occurred_at="2026-01-01T00:00:00Z",
            fingerprint="old",
            raw_json=json.dumps({"observer": "bdk", "status": {"confirmed": True}}),
            amount=100_000,
            fee=1_000,
        )
        normalized = normalize_import_record(
            {
                "txid": "11" * 32,
                "occurred_at": "2026-01-01T00:00:00Z",
                "confirmed_at": None,
                "direction": "outbound",
                "asset": "BTC",
                "amount": "0.000001",
                "fee": "0.00000001",
                "raw_json": {"observer": "bdk", "status": {"confirmed": False}},
            },
            source_label="backend:test",
        )
        updates = _transaction_merge_updates(existing, normalized, "new")
        self.assertIn("confirmed_at", updates)
        self.assertIsNone(updates["confirmed_at"])

    def test_exact_pin_lock_wheels_and_packager_collection(self):
        pyproject = (ROOT / "pyproject.toml").read_text(encoding="utf-8")
        lock = (ROOT / "uv.lock").read_text(encoding="utf-8")
        workflow = (ROOT / ".github/workflows/prerelease-binaries.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn('"bdkpython==3.0.0"', pyproject)
        self.assertIn('name = "bdkpython"\nversion = "3.0.0"', lock)
        for platform in ("macosx_11_0_arm64", "manylinux_2_28_x86_64", "win_amd64"):
            self.assertIn(f"bdkpython-3.0.0-cp313-cp313-{platform}.whl", lock)
        self.assertGreaterEqual(workflow.count("--collect-submodules bdkpython"), 2)
        self.assertTrue(callable(bdk.Wallet.start_full_scan))
        self.assertTrue(callable(bdk.Persister.custom))

    def test_every_changeset_component_roundtrips_as_stable_json(self):
        receive, change, *_ = _sample_descriptor_pair()
        descriptor = bdk.Descriptor(receive, bdk.NetworkKind.MAIN)
        change_descriptor = bdk.Descriptor(change, bdk.NetworkKind.MAIN)
        txid = bdk.Txid.from_bytes(bytes.fromhex("11" * 32))
        block_hash = bdk.BlockHash.from_bytes(bytes.fromhex("22" * 32))
        outpoint = bdk.OutPoint(txid=txid, vout=3)
        changeset = bdk.ChangeSet.from_aggregate_with_locked_outpoints(
            descriptor,
            change_descriptor,
            bdk.Network.BITCOIN,
            bdk.LocalChainChangeSet(
                changes=[bdk.ChainChange(height=7, hash=block_hash)]
            ),
            bdk.TxGraphChangeSet(
                txs=[],
                txouts={
                    bdk.HashableOutPoint(outpoint): bdk.TxOut(
                        value=bdk.Amount.from_sat(42),
                        script_pubkey=bdk.Script(bytes.fromhex("0014" + "33" * 20)),
                    )
                },
                anchors=[
                    bdk.Anchor(
                        confirmation_block_time=bdk.ConfirmationBlockTime(
                            block_id=bdk.BlockId(height=7, hash=block_hash),
                            confirmation_time=123,
                        ),
                        txid=txid,
                    )
                ],
                last_seen={txid: 124},
                first_seen={txid: 120},
                last_evicted={txid: 125},
            ),
            bdk.IndexerChangeSet(
                last_revealed={
                    bdk.DescriptorId.from_bytes(bytes.fromhex("44" * 32)): 9
                }
            ),
            {bdk.HashableOutPoint(outpoint): True},
        )
        payload = serialize_changeset(changeset)
        encoded = json.dumps(payload, sort_keys=True)
        self.assertNotIn("pickle", encoded.lower())
        self.assertEqual(payload["schema_version"], BDK_CHANGESET_SCHEMA_VERSION)
        self.assertEqual(serialize_changeset(deserialize_changeset(payload)), payload)
        self.assertEqual(SqlCipherBdkPersistence(deserialize_changeset(payload)).payload(), payload)

        invalid = dict(payload)
        invalid["unknown_component"] = []
        with self.assertRaises(AppError) as raised:
            deserialize_changeset(invalid)
        self.assertEqual(raised.exception.code, "observer_state_rebuild_required")
        nested_invalid = json.loads(json.dumps(payload))
        nested_invalid["tx_graph"]["anchors"][0]["future_field"] = 1
        with self.assertRaises(AppError):
            deserialize_changeset(nested_invalid)

    def test_route_capabilities_are_explicit(self):
        _wallet, plan = _descriptor_wallet()
        state = type("State", (), {"chain": "bitcoin", "descriptor_plan": plan})()
        self.assertIsNone(
            bdk_compatibility_reason(
                {"kind": "esplora", "url": "https://mempool.space/api"}, state
            )
        )
        self.assertEqual(
            bdk_compatibility_reason(
                {
                    "kind": "electrum",
                    "url": "ssl://node.example:50002",
                    "certificate": "/private/ca.pem",
                },
                state,
            ),
            "custom_ca",
        )
        address_state = type("State", (), {"chain": "bitcoin", "descriptor_plan": None})()
        self.assertEqual(
            bdk_compatibility_reason({"kind": "electrum"}, address_state), "address_list"
        )

    def test_esplora_custom_ca_fails_closed_instead_of_ignoring_trust(self):
        _wallet, plan = _descriptor_wallet()
        state = type("State", (), {"chain": "bitcoin", "descriptor_plan": plan})()
        with self.assertRaises(AppError) as raised:
            bdk_compatibility_reason(
                {
                    "kind": "esplora",
                    "url": "https://node.example",
                    "certificate": "/private/ca.pem",
                },
                state,
            )
        self.assertEqual(raised.exception.code, "observer_capability_unsupported")
        self.assertEqual(raised.exception.details["capability"], "esplora_custom_ca")

    def test_mempool_backend_alias_selects_bdk(self):
        _wallet, plan = _descriptor_wallet()
        state = type("State", (), {"chain": "bitcoin", "descriptor_plan": plan})()
        self.assertIsNone(
            bdk_compatibility_reason(
                {"kind": "mempool", "url": "https://mempool.example/api"}, state
            )
        )
        self.assertEqual(core_sync.normalize_backend_kind("mempool"), "esplora")

    def test_socks5h_is_translated_for_bdk_without_losing_credentials(self):
        self.assertEqual(
            _bdk_proxy_url("socks5h://user:pass@127.0.0.1:9050"),
            "socks5://user:pass@127.0.0.1:9050",
        )
        self.assertEqual(
            _bdk_proxy_url("socks5://127.0.0.1:9050"),
            "socks5://127.0.0.1:9050",
        )

    def test_address_list_reports_first_class_bitcoin_script_route(self):
        wallet, _discovery = self._discovery()
        backend = {"name": "script", "kind": "esplora", "url": "https://host"}
        state = core_sync.WalletSyncState(
            chain="bitcoin",
            network="main",
            descriptor_plan=None,
            policy_asset_id="",
            targets=({"script_pubkey": "0014" + "11" * 20},),
            tracked_scripts={},
            history_cache={},
            checkpoint={},
        )
        discovery = core_sync.WalletBackendDiscovery(
            backend=backend,
            sync_state=state,
            kind="esplora",
            started=0.0,
            force_full=False,
        )
        compatibility = mock.Mock(return_value=([], {}))
        with mock.patch.object(
            sync_backends,
            "COMPATIBILITY_SYNC_BACKEND_ADAPTERS",
            {"esplora": compatibility},
        ):
            fetched = sync_backends.prepare_dependency_observer_fetch(
                mock.Mock(), {}, wallet, discovery
            )
        self.assertEqual(fetched.adapter_meta["observer_route"], "bitcoin_script")
        self.assertEqual(
            fetched.adapter_meta["observer_compatibility_reason"], "address_list"
        )

    def test_no_egress_fails_before_native_client_construction(self):
        wallet, plan = _descriptor_wallet()
        identity = identities_for_wallet(wallet, observer_kind="bdk")[0]
        observer = BdkObserver(
            identity=identity,
            backend={"name": "native", "kind": "esplora", "url": "https://example.invalid"},
            branches=bdk_branches_for_identity(plan, identity),
            gap_limit=20,
        )
        with mock.patch.dict(os.environ, {"KASSIBER_NO_EGRESS": "1"}), mock.patch.object(
            bdk, "EsploraClient"
        ) as client:
            with self.assertRaises(AppError) as raised:
                observer._client()
        self.assertEqual(raised.exception.code, "network_egress_disabled")
        client.assert_not_called()

    def test_onion_never_connects_directly(self):
        wallet, plan = _descriptor_wallet()
        identity = identities_for_wallet(wallet, observer_kind="bdk")[0]
        observer = BdkObserver(
            identity=identity,
            backend={"name": "onion", "kind": "electrum", "url": "ssl://hiddenservice.onion:50002"},
            branches=bdk_branches_for_identity(plan, identity),
            gap_limit=20,
        )
        with mock.patch.object(bdk, "ElectrumClient") as client:
            with self.assertRaises(AppError) as raised:
                observer._client()
        self.assertEqual(raised.exception.code, "network_proxy_required")
        client.assert_not_called()


if __name__ == "__main__":
    unittest.main()
