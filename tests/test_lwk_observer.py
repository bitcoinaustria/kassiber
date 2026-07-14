from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

from embit import bip32, bip39

from kassiber.core.chain_observer.identity import identities_for_wallet
from kassiber.core.chain_observer.lwk import (
    LwkObserver,
    _fee_sats_by_asset,
    _lwk_electrum_connection,
    _lwk_esplora_auth_options,
    lwk_compatibility_reason,
    lwk_descriptor_for_plan,
)
from kassiber.core.chain_observer.lwk_persistence import SqlCipherForeignStore, require_lwk
from kassiber.core.chain_observer.store import load_observer_values, persist_observer_state
from kassiber.core import sync as core_sync
from kassiber.core import sync_backends
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.time_utils import now_iso
from kassiber.wallet_descriptors import (
    derive_descriptor_target,
    liquid_blinding_secret,
    load_descriptor_plan,
)


MNEMONIC = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
POLICY_ASSET = "5ac9f65c0efcc4775e0baec4ec03abdde22473cd3cf33c0419ca290e0751b225"
ROOT = Path(__file__).resolve().parents[1]


def descriptor(script: str = "elwpkh", *, path: str = "<0;1>/*") -> str:
    root = bip32.HDKey.from_seed(bip39.mnemonic_to_seed(MNEMONIC))
    xpub = root.derive("m/84h/1h/0h").to_public().to_base58()
    blind = bip32.HDKey.from_seed(b"\x03" * 32).key.wif()
    return f"ct(slip77({blind}),{script}({xpub}/{path}))"


def wallet_row(config: dict) -> dict:
    return {
        "id": "wallet", "workspace_id": "workspace", "profile_id": "profile",
        "config_json": json.dumps(config),
    }


class LwkDescriptorContractTest(unittest.TestCase):
    def test_fee_normalization_reads_explicit_elements_fee_outputs(self):
        class Output:
            def __init__(self, *, fee, value=None, asset=None):
                self._fee = fee
                self._value = value
                self._asset = asset

            def is_fee(self):
                return self._fee

            def value(self):
                return self._value

            def asset(self):
                return self._asset

        outputs = [
            Output(fee=False, value=10, asset="asset"),
            Output(fee=True, value=700, asset="policy"),
            Output(fee=True, value=300, asset="policy"),
            Output(fee=True, value=None, asset="policy"),
        ]

        self.assertEqual(_fee_sats_by_asset(outputs), {"policy": 1000})

    def _discovery(self, *, backend=None, partial=False, force_full=False):
        config = {"chain": "liquid", "network": "elementsregtest", "descriptor": descriptor(), "gap_limit": 20}
        plan = load_descriptor_plan(config)
        targets = sync_backends._offline_descriptor_targets(plan, {})
        if partial:
            targets = targets[1:]
        wallet = wallet_row(config)
        resolved = backend or {"name": "native", "kind": "esplora", "url": "http://127.0.0.1:3002"}
        state = core_sync.WalletSyncState(
            chain="liquid", network="elementsregtest", descriptor_plan=plan,
            policy_asset_id=POLICY_ASSET, targets=targets,
            tracked_scripts={item["script_pubkey"]: item for item in targets},
            history_cache={}, checkpoint={},
        )
        return wallet, core_sync.WalletBackendDiscovery(
            backend=resolved, sync_state=state, kind=resolved["kind"],
            started=0.0, force_full=force_full,
        )

    def test_dependency_version_is_exact(self):
        self.assertEqual(require_lwk().__name__, "lwk")

    def test_exact_pin_platform_wheels_and_packager_collection(self):
        pyproject = (ROOT / "pyproject.toml").read_text(encoding="utf-8")
        lock = (ROOT / "uv.lock").read_text(encoding="utf-8")
        workflow = (ROOT / ".github/workflows/prerelease-binaries.yml").read_text(encoding="utf-8")
        self.assertIn('"lwk==0.18.0"', pyproject)
        self.assertIn('name = "lwk"\nversion = "0.18.0"', lock)
        for platform in (
            "macosx_11_0_arm64", "manylinux_2_17_x86_64.manylinux2014_x86_64", "win_amd64",
        ):
            self.assertIn(f"lwk-0.18.0-py3-none-{platform}.whl", lock)
        self.assertGreaterEqual(workflow.count("--collect-submodules lwk"), 2)

    def test_canonical_multipath_and_fixed_descriptors_execute_upstream(self):
        for raw in (descriptor(), descriptor(path="0/7")):
            plan = load_descriptor_plan(
                {"chain": "liquid", "network": "elementsregtest", "descriptor": raw,
                 "synthesize_change": False}
            )
            parsed = lwk_descriptor_for_plan(plan)
            self.assertTrue(str(parsed).startswith("ct(slip77("))

    def test_slip77_multisig_and_liquid_taproot_execute_upstream(self):
        root_a = bip32.HDKey.from_seed(bip39.mnemonic_to_seed(MNEMONIC))
        root_b = bip32.HDKey.from_seed(b"\x02" * 32)
        blind = bip32.HDKey.from_seed(b"\x03" * 32).key.wif()
        keys = [
            root.derive("m/48h/1h/0h/2h").to_public().to_base58() + "/0/*"
            for root in (root_a, root_b)
        ]
        raws = (
            f"ct(slip77({blind}),elwsh(sortedmulti(2,{keys[0]},{keys[1]})))",
            descriptor(script="eltr", path="0/*"),
        )
        for raw in raws:
            plan = load_descriptor_plan(
                {"chain": "liquid", "network": "elementsregtest", "descriptor": raw,
                 "synthesize_change": False}
            )
            self.assertIn("ct(", str(lwk_descriptor_for_plan(plan)))

    def test_actual_legacy_p2sh_view_key_support_is_executable(self):
        lwk = require_lwk()
        keys = (
            "026a04ab98d9e4774ad806e302dddeb63bea16b5cb5f223ee77478e861bb583eb3",
            "0268680737c76dabb801cb2204f57dbe4e4579e4f710cd67dc1b4227592c81e9b5",
            "02b95c249d84f417e3e395a127425428b540671cc15881eb828c17b722a53fc599",
        )
        parsed = lwk.WolletDescriptor(
            f"ct({'11' * 32},elsh(multi(2,{','.join(keys)})))"
        )
        self.assertIn("elsh(multi", str(parsed))

    def test_transport_and_descriptor_compatibility_is_preflighted(self):
        plan = load_descriptor_plan(
            {"chain": "liquid", "network": "elementsregtest", "descriptor": descriptor()}
        )
        state = SimpleNamespace(chain="liquid", descriptor_plan=plan)
        self.assertIsNone(lwk_compatibility_reason(
            {"kind": "esplora", "url": "http://127.0.0.1:3002"}, state
        ))
        self.assertEqual(lwk_compatibility_reason(
            {"kind": "esplora", "url": "http://example.onion", "proxy": "socks5://127.0.0.1:9050"}, state
        ), "proxy_transport")
        self.assertEqual(lwk_compatibility_reason(
            {"kind": "electrum", "url": "ssl://host:50002", "certificate": "ca.pem"}, state
        ), "custom_ca")

    def test_esplora_auth_is_passed_to_lwk_instead_of_compatibility(self):
        plan = load_descriptor_plan(
            {"chain": "liquid", "network": "elementsregtest", "descriptor": descriptor()}
        )
        state = SimpleNamespace(chain="liquid", descriptor_plan=plan)
        backend = {
            "kind": "liquid-esplora",
            "url": "https://example.invalid",
            "auth_header": "Bearer secret",
            "token": "api-key",
        }
        self.assertIsNone(lwk_compatibility_reason(backend, state))
        fake_lwk = SimpleNamespace(
            TokenProvider=SimpleNamespace(STATIC=lambda value: ("static", value))
        )
        self.assertEqual(
            _lwk_esplora_auth_options(fake_lwk, backend),
            {
                "headers": {"Authorization": "Bearer secret"},
                "token_provider": ("static", "api-key"),
            },
        )

    def test_esplora_custom_ca_fails_closed_before_compatibility(self):
        plan = load_descriptor_plan(
            {"chain": "liquid", "network": "elementsregtest", "descriptor": descriptor()}
        )
        state = SimpleNamespace(chain="liquid", descriptor_plan=plan)
        with self.assertRaises(AppError) as raised:
            lwk_compatibility_reason(
                {"kind": "esplora", "url": "https://host", "certificate": "ca.pem"},
                state,
            )
        self.assertEqual(raised.exception.code, "observer_capability_unsupported")
        self.assertEqual(raised.exception.details["capability"], "esplora_custom_ca")

    def test_explicit_electrum_constructor_honors_tls_validation(self):
        self.assertEqual(
            _lwk_electrum_connection({"url": "ssl://node.example:50002"}),
            ("node.example:50002", True, True),
        )
        self.assertEqual(
            _lwk_electrum_connection({"url": "tcp://node.example:50001"}),
            ("node.example:50001", False, False),
        )

    def test_pinned_lwk_insecure_tls_stays_on_compatibility(self):
        plan = load_descriptor_plan(
            {"chain": "liquid", "network": "elementsregtest", "descriptor": descriptor()}
        )
        state = SimpleNamespace(chain="liquid", descriptor_plan=plan)
        self.assertEqual(
            lwk_compatibility_reason(
                {
                    "kind": "electrum",
                    "url": "ssl://node.example:50002",
                    "insecure": True,
                },
                state,
            ),
            "insecure_tls",
        )

    def test_native_client_receives_auth_and_explicit_tls_policy(self):
        network = object()
        builder = object()
        esplora_client = object()
        fake_lwk = SimpleNamespace(
            TokenProvider=SimpleNamespace(STATIC=Mock(return_value="static-token")),
            EsploraClientBuilder=Mock(return_value=builder),
            EsploraClient=SimpleNamespace(from_builder=Mock(return_value=esplora_client)),
            ElectrumClient=Mock(return_value="electrum-client"),
        )
        observer = object.__new__(LwkObserver)
        observer.backend = {
            "name": "auth",
            "kind": "liquid-esplora",
            "url": "https://example.invalid/api",
            "auth_header": "Bearer secret",
            "token": "api-key",
            "batch_size": 4,
            "timeout": 12,
        }
        with patch(
            "kassiber.core.chain_observer.lwk.require_lwk", return_value=fake_lwk
        ), patch(
            "kassiber.core.chain_observer.lwk._truthy_env", return_value=False
        ):
            self.assertIs(observer._client(network), esplora_client)
        fake_lwk.EsploraClientBuilder.assert_called_once_with(
            base_url="https://example.invalid/api",
            network=network,
            concurrency=4,
            timeout=12,
            headers={"Authorization": "Bearer secret"},
            token_provider="static-token",
        )

        observer.backend = {
            "name": "tls",
            "kind": "electrum",
            "url": "ssl://node.example:50002",
        }
        with patch(
            "kassiber.core.chain_observer.lwk.require_lwk", return_value=fake_lwk
        ), patch(
            "kassiber.core.chain_observer.lwk._truthy_env", return_value=False
        ):
            self.assertEqual(observer._client(network), "electrum-client")
        fake_lwk.ElectrumClient.assert_called_once_with(
            "node.example:50002", True, True
        )

    def test_structurally_equivalent_separate_change_is_canonicalized(self):
        plan = load_descriptor_plan(
            {
                "chain": "liquid",
                "network": "elementsregtest",
                "descriptor": descriptor(path="0/*"),
                "change_descriptor": descriptor(path="1/*"),
                "synthesize_change": False,
            }
        )
        state = SimpleNamespace(chain="liquid", descriptor_plan=plan)
        self.assertIsNone(
            lwk_compatibility_reason(
                {"kind": "esplora", "url": "http://127.0.0.1:3002"}, state
            )
        )
        parsed = lwk_descriptor_for_plan(plan)
        self.assertIn("/<0;1>/*", str(parsed))
        lwk = require_lwk()
        for branch_index, chain in ((0, lwk.Chain.EXTERNAL), (1, lwk.Chain.INTERNAL)):
            for index in (0, 1, 7, 100):
                expected = derive_descriptor_target(plan, branch_index, index)
                actual_script = parsed.script_pubkey(chain, index)
                self.assertEqual(actual_script.to_bytes().hex(), expected.script_pubkey)
                expected_blinding, _target = liquid_blinding_secret(
                    plan, branch_index, index
                )
                actual_blinding = parsed.derive_blinding_key(actual_script)
                self.assertIsNotNone(actual_blinding)
                self.assertEqual(actual_blinding.bytes(), expected_blinding)

    def test_different_change_policy_stays_on_compatibility_route(self):
        plan = load_descriptor_plan(
            {
                "chain": "liquid",
                "network": "elementsregtest",
                "descriptor": descriptor(path="0/*"),
                "change_descriptor": descriptor(script="eltr", path="1/*"),
                "synthesize_change": False,
            }
        )
        state = SimpleNamespace(chain="liquid", descriptor_plan=plan)
        self.assertEqual(
            lwk_compatibility_reason(
                {"kind": "esplora", "url": "http://127.0.0.1:3002"}, state
            ),
            "separate_change_descriptor",
        )

    def test_separate_multisig_change_canonicalizes_all_key_branches(self):
        root_a = bip32.HDKey.from_seed(bip39.mnemonic_to_seed(MNEMONIC))
        root_b = bip32.HDKey.from_seed(b"\x02" * 32)
        blind = bip32.HDKey.from_seed(b"\x03" * 32).key.wif()

        def multisig(branch: int) -> str:
            keys = [
                root.derive("m/48h/1h/0h/2h").to_public().to_base58()
                + f"/{branch}/*"
                for root in (root_a, root_b)
            ]
            return f"ct(slip77({blind}),elwsh(sortedmulti(2,{keys[0]},{keys[1]})))"

        plan = load_descriptor_plan(
            {
                "chain": "liquid",
                "network": "elementsregtest",
                "descriptor": multisig(0),
                "change_descriptor": multisig(1),
                "synthesize_change": False,
            }
        )
        parsed = lwk_descriptor_for_plan(plan)
        self.assertEqual(str(parsed).count("/<0;1>/*"), 2)
        lwk = require_lwk()
        for branch_index, chain in ((0, lwk.Chain.EXTERNAL), (1, lwk.Chain.INTERNAL)):
            for index in (0, 9, 100):
                expected = derive_descriptor_target(plan, branch_index, index)
                self.assertEqual(
                    parsed.script_pubkey(chain, index).to_bytes().hex(),
                    expected.script_pubkey,
                )

    def test_reversed_noncanonical_multipath_is_not_relabelled(self):
        plan = load_descriptor_plan(
            {
                "chain": "liquid",
                "network": "elementsregtest",
                "descriptor": descriptor(path="<1;0>/*"),
            }
        )
        state = SimpleNamespace(chain="liquid", descriptor_plan=plan)
        self.assertEqual(
            lwk_compatibility_reason(
                {"kind": "esplora", "url": "http://127.0.0.1:3002"}, state
            ),
            "descriptor_unsupported",
        )

    def test_supported_route_is_lwk_only_and_runtime_failure_never_falls_back(self):
        wallet, discovery = self._discovery()
        compatibility = unittest.mock.Mock(side_effect=AssertionError("embit observer called"))
        prepared = object()
        with patch.object(sync_backends, "COMPATIBILITY_SYNC_BACKEND_ADAPTERS", {"esplora": compatibility}), patch(
            "kassiber.core.chain_observer.prepare_observer_update", return_value=prepared,
        ), patch("kassiber.core.chain_observer.store.load_observer_values", return_value={}):
            fetched = sync_backends.prepare_dependency_observer_fetch(unittest.mock.Mock(), {}, wallet, discovery)
        self.assertEqual(fetched.adapter_meta["observer_route"], "lwk")
        self.assertEqual(fetched.observer_updates, (prepared,))
        compatibility.assert_not_called()

        with patch.object(sync_backends, "COMPATIBILITY_SYNC_BACKEND_ADAPTERS", {"esplora": compatibility}), patch(
            "kassiber.core.chain_observer.prepare_observer_update",
            side_effect=AppError("native failed", code="backend_sync_failed"),
        ), patch("kassiber.core.chain_observer.store.load_observer_values", return_value={}):
            with self.assertRaises(AppError):
                sync_backends.prepare_dependency_observer_fetch(unittest.mock.Mock(), {}, wallet, discovery)
        compatibility.assert_not_called()

    def test_overlap_and_proxy_choose_named_compatibility_before_lwk(self):
        for expected, backend, partial in (
            ("source_overlap_partial_descriptor", {"name": "overlap", "kind": "esplora", "url": "http://host"}, True),
            ("proxy_transport", {"name": "tor", "kind": "esplora", "url": "http://hidden.onion", "proxy": "socks5://127.0.0.1:9050"}, False),
        ):
            wallet, discovery = self._discovery(backend=backend, partial=partial)
            compatibility = unittest.mock.Mock(return_value=([], {}))
            with patch.object(sync_backends, "COMPATIBILITY_SYNC_BACKEND_ADAPTERS", {"esplora": compatibility}), patch(
                "kassiber.core.chain_observer.prepare_observer_update"
            ) as native:
                fetched = sync_backends.prepare_dependency_observer_fetch(unittest.mock.Mock(), {}, wallet, discovery)
            self.assertEqual(fetched.adapter_meta["observer_compatibility_reason"], expected)
            compatibility.assert_called_once()
            native.assert_not_called()

    def test_forced_refresh_rebuilds_incompatible_opaque_store_in_memory(self):
        wallet, discovery = self._discovery(force_full=True)
        prepared = object()
        with patch(
            "kassiber.core.chain_observer.store.load_observer_values",
            side_effect=AppError(
                "newer opaque namespace",
                code="observer_state_rebuild_required",
            ),
        ) as load_values, patch(
            "kassiber.core.chain_observer.prepare_observer_update",
            return_value=prepared,
        ) as prepare:
            fetched = sync_backends.prepare_dependency_observer_fetch(
                unittest.mock.Mock(), {}, wallet, discovery
            )

        self.assertEqual(fetched.observer_updates, (prepared,))
        observer = prepare.call_args.args[2]
        self.assertEqual(observer.store.snapshot(), {})
        load_values.assert_not_called()


class LwkForeignStoreTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="kassiber-lwk-")
        self.addCleanup(self.temp.cleanup)
        self.conn = open_db(Path(self.temp.name) / "data")
        self.addCleanup(self.conn.close)
        timestamp = now_iso()
        self.conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('workspace','W',?)", (timestamp,))
        self.conn.execute(
            "INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,tax_long_term_days,gains_algorithm,created_at) VALUES('profile','workspace','P','EUR','generic',365,'FIFO',?)",
            (timestamp,),
        )
        config = {"chain": "liquid", "network": "elementsregtest", "descriptor": descriptor()}
        self.conn.execute(
            "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('wallet','workspace','profile','L','descriptor',?,?)",
            (json.dumps(config), timestamp),
        )
        self.conn.commit()
        self.identity = identities_for_wallet(wallet_row(config), observer_kind="lwk")[0]

    def test_foreign_store_roundtrips_opaque_bytes_only_on_apply_savepoint(self):
        lwk = require_lwk()
        store = SqlCipherForeignStore(self.identity, {})
        helper = lwk.LwkTestStore(lwk.ForeignStoreLink(store))
        helper.write("Liquid:Tx:test", b"\x00\xffopaque")
        self.assertEqual(helper.read("Liquid:Tx:test"), b"\x00\xffopaque")
        self.assertEqual(load_observer_values(self.conn, self.identity), {})

        self.conn.execute("SAVEPOINT lwk_apply")
        persist_observer_state(self.conn, self.identity, {"schema_version": 1}, ())
        store.persist(self.conn)
        self.conn.execute("RELEASE SAVEPOINT lwk_apply")
        self.conn.commit()
        self.assertEqual(load_observer_values(self.conn, self.identity), {"Liquid:Tx:test": b"\x00\xffopaque"})

    def test_rollback_and_discard_never_apply_mutated_lwk_state(self):
        store = SqlCipherForeignStore(self.identity, {})
        store.put("mutated", b"state")
        self.conn.execute("SAVEPOINT cancelled")
        persist_observer_state(self.conn, self.identity, {"schema_version": 1}, ())
        store.persist(self.conn)
        self.conn.execute("ROLLBACK TO SAVEPOINT cancelled")
        self.conn.execute("RELEASE SAVEPOINT cancelled")
        store.discard()
        self.assertEqual(load_observer_values(self.conn, self.identity), {})

    def test_no_egress_blocks_before_native_client_construction(self):
        config = {"chain": "liquid", "network": "elementsregtest", "descriptor": descriptor()}
        plan = load_descriptor_plan(config)
        observer = LwkObserver(
            identity=self.identity,
            backend={"name": "liquid", "kind": "esplora", "url": "https://secret.example/api"},
            descriptor_plan=plan, policy_asset_id=POLICY_ASSET, stored_values={},
        )
        with patch.dict("os.environ", {"KASSIBER_NO_EGRESS": "1"}), patch.object(
            require_lwk().EsploraClient, "from_builder"
        ) as native:
            with self.assertRaises(AppError) as error:
                observer._client(require_lwk().Network.regtest(POLICY_ASSET))
        self.assertEqual(error.exception.code, "network_egress_disabled")
        native.assert_not_called()


if __name__ == "__main__":
    unittest.main()
