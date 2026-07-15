from __future__ import annotations

import json
import tempfile
import unittest
from argparse import Namespace
from pathlib import Path
from unittest.mock import patch

from kassiber.cli import handlers as cli_handlers
from kassiber.core import audit_package
from kassiber.core import maintenance as core_maintenance
from kassiber.core import sync as core_sync
from kassiber.core import wallets as core_wallets
from kassiber.core.chain_observer import (
    PRIVATE_OBSERVER_TABLES,
    ChainFacts,
    ChainObserver,
    CoveragePoint,
    ObserverApplication,
    ObserverIdentity,
    ObserverPrepareRequest,
    apply_prepared_observer_update,
    delete_wallet_observer_state,
    discard_prepared_observer_update,
    identities_for_wallet,
    identities_for_wallets,
    load_observer_state,
    persist_observer_state,
    prepare_observer_update,
)
from kassiber.core.sync_replication.schema_allowlist import SYNC_TABLES
from kassiber.core.ui_snapshot import build_wallets_list_snapshot
from kassiber.db import open_db, set_setting
from kassiber.diagnostics import collect_public_diagnostics
from kassiber.errors import AppError
from kassiber.secrets.sqlcipher import sqlcipher_available
from kassiber.time_utils import now_iso


class FakeObserver:
    def __init__(
        self,
        conn,
        *,
        marker="observer-private-marker",
        outputs=({"txid": "aa" * 32, "vout": 0},),
    ):
        self.conn = conn
        self.marker = marker
        self.outputs = outputs
        self.events = []
        self.discarded = False

    def prepare(self, request, prior_state):
        self.events.append(("prepare", self.conn.in_transaction))
        generation = int((prior_state.payload if prior_state else {}).get("generation", 0)) + 1
        return {
            "generation": generation,
            "backend_kind": request.backend_kind,
            "force_full": request.force_full,
        }

    def apply(self, prepared_update, prior_state):
        self.events.append(("apply", self.conn.in_transaction))
        generation = int(prepared_update["generation"])
        return ObserverApplication(
            state={
                "encoding": "fake-observer-json-v1",
                "generation": generation,
                "private": self.marker,
            },
            facts=ChainFacts(
                transaction_records=({"external_id": f"tx-{generation}"},),
                retracted_external_ids=("old-tx",),
                outputs=self.outputs,
                coverage=(
                    CoveragePoint(
                        "receive",
                        scanned_to=40,
                        highest_used=3,
                        details={"dependency": "fake"},
                    ),
                    CoveragePoint("change", scanned_to=20, highest_used=None),
                ),
                freshness_checkpoint={"fake_generation": generation},
            ),
        )

    def discard(self):
        self.events.append(("discard", self.conn.in_transaction))
        self.discarded = True


class ChainObserverContractTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="kassiber-observer-")
        self.addCleanup(self.temp.cleanup)
        self.data_root = Path(self.temp.name) / "data"
        self.conn = open_db(self.data_root)
        self.addCleanup(self.conn.close)
        self.workspace_id = "observer-workspace"
        self.profile_id = "observer-profile"
        self.wallet_id = "observer-wallet"
        timestamp = now_iso()
        self.conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            (self.workspace_id, "Observer workspace", timestamp),
        )
        self.conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, created_at
            ) VALUES(?, ?, ?, 'EUR', 'generic', 365, 'FIFO', ?)
            """,
            (self.profile_id, self.workspace_id, "Observer profile", timestamp),
        )
        self.conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES(?, ?, ?, ?, 'address', '{}', ?)
            """,
            (self.wallet_id, self.workspace_id, self.profile_id, "Observer wallet", timestamp),
        )
        set_setting(self.conn, "context_workspace", self.workspace_id)
        set_setting(self.conn, "context_profile", self.profile_id)
        self.conn.commit()
        self.identity = ObserverIdentity(
            id="observer-instance",
            workspace_id=self.workspace_id,
            profile_id=self.profile_id,
            logical_wallet_id=self.wallet_id,
            source_wallet_id=self.wallet_id,
            source_key="descriptor:default",
            observer_kind="fake",
            chain="bitcoin",
            network="regtest",
            branch_keys=("receive", "change"),
        )

    def _prepare(self, observer=None):
        observer = observer or FakeObserver(self.conn)
        prepared = prepare_observer_update(
            self.conn,
            self.identity,
            observer,
            ObserverPrepareRequest(
                backend_name="regtest",
                backend_kind="fake",
            ),
        )
        return observer, prepared

    def _apply_and_commit(self, prepared):
        self.conn.execute("SAVEPOINT observer_apply")
        facts = apply_prepared_observer_update(self.conn, prepared)
        self.conn.execute("RELEASE SAVEPOINT observer_apply")
        self.conn.commit()
        return facts

    def test_fake_observer_fetches_outside_and_applies_inside_transaction(self):
        observer, prepared = self._prepare()
        self.assertEqual(observer.events, [("prepare", False)])

        facts = self._apply_and_commit(prepared)

        self.assertEqual(observer.events[-1], ("apply", True))
        self.assertEqual(facts.freshness_checkpoint, {"fake_generation": 1})
        stored = load_observer_state(self.conn, self.identity)
        self.assertEqual(stored.payload["generation"], 1)
        self.assertEqual(
            [(point.branch_key, point.scanned_to) for point in stored.coverage],
            [("change", 20), ("receive", 40)],
        )

    def test_used_index_must_be_inside_exclusive_coverage_boundary(self):
        observer = FakeObserver(self.conn)
        _observer, prepared = self._prepare(observer)
        original_apply = observer.apply

        def invalid_apply(prepared_update, prior_state):
            application = original_apply(prepared_update, prior_state)
            return ObserverApplication(
                state=application.state,
                facts=ChainFacts(
                    coverage=(
                        CoveragePoint(
                            "receive",
                            scanned_to=3,
                            highest_used=3,
                        ),
                    ),
                ),
            )

        observer.apply = invalid_apply
        self.conn.execute("SAVEPOINT invalid_coverage")
        with self.assertRaises(AppError) as raised:
            apply_prepared_observer_update(self.conn, prepared)
        self.conn.execute("ROLLBACK TO SAVEPOINT invalid_coverage")
        self.conn.execute("RELEASE SAVEPOINT invalid_coverage")

        self.assertEqual(raised.exception.code, "observer_state_invalid")

    def test_version_one_coverage_requires_rebuild_under_exclusive_semantics(self):
        _observer, prepared = self._prepare()
        self._apply_and_commit(prepared)
        self.conn.execute(
            "UPDATE chain_observer_coverage SET coverage_version = 1 WHERE observer_id = ?",
            (self.identity.id,),
        )
        self.conn.commit()

        with self.assertRaises(AppError) as raised:
            load_observer_state(self.conn, self.identity)

        self.assertEqual(raised.exception.code, "observer_state_rebuild_required")
        self.assertEqual(raised.exception.details["representation"], "coverage")

    def test_observer_boundary_exposes_no_spending_or_broadcast_capability(self):
        methods = {
            name
            for name, value in ChainObserver.__dict__.items()
            if callable(value) and not name.startswith("_")
        }
        self.assertEqual(methods, {"prepare", "apply", "discard"})
        forbidden = {"address", "build", "coin_select", "psbt", "pset", "sign", "broadcast"}
        self.assertTrue(methods.isdisjoint(forbidden))

    def test_prepare_inside_and_apply_outside_transactions_fail_closed(self):
        self.conn.execute("SAVEPOINT invalid_prepare")
        with self.assertRaises(AppError) as prepare_error:
            self._prepare()
        self.assertEqual(prepare_error.exception.code, "observer_prepare_in_transaction")
        self.conn.execute("ROLLBACK TO SAVEPOINT invalid_prepare")
        self.conn.execute("RELEASE SAVEPOINT invalid_prepare")

        _observer, prepared = self._prepare()
        with self.assertRaises(AppError) as apply_error:
            apply_prepared_observer_update(self.conn, prepared)
        self.assertEqual(apply_error.exception.code, "observer_apply_outside_transaction")

    def test_rollback_discards_database_and_request_local_observer(self):
        observer, prepared = self._prepare()
        self.conn.execute("SAVEPOINT failed_wallet_refresh")
        apply_prepared_observer_update(self.conn, prepared)
        self.conn.execute("ROLLBACK TO SAVEPOINT failed_wallet_refresh")
        self.conn.execute("RELEASE SAVEPOINT failed_wallet_refresh")
        discard_prepared_observer_update(prepared)

        self.assertIsNone(load_observer_state(self.conn, self.identity))
        self.assertTrue(observer.discarded)
        self.assertEqual(observer.events[-1][0], "discard")

    def test_wallet_coordinator_rolls_back_and_discards_applied_observer(self):
        self.conn.execute(
            "UPDATE wallets SET config_json = ? WHERE id = ?",
            (
                json.dumps(
                    {
                        "descriptor": "fake-contract-descriptor",
                        "chain": "bitcoin",
                        "network": "regtest",
                    }
                ),
                self.wallet_id,
            ),
        )
        self.conn.commit()
        wallet = self.conn.execute(
            "SELECT * FROM wallets WHERE id = ?", (self.wallet_id,)
        ).fetchone()
        profile = self.conn.execute(
            "SELECT * FROM profiles WHERE id = ?", (self.profile_id,)
        ).fetchone()
        observer, prepared = self._prepare()
        sync_state = core_sync.WalletSyncState(
            chain="bitcoin",
            network="regtest",
            descriptor_plan=None,
            policy_asset_id="",
            targets=({"script_pubkey": "0014" + "00" * 20},),
            tracked_scripts={},
            history_cache={},
        )
        fetch = core_sync.WalletBackendFetch(
            backend={"name": "fake", "kind": "fake", "url": "http://127.0.0.1"},
            sync_state=sync_state,
            normalized_records=(),
            adapter_meta={},
            kind="fake",
            started=0.0,
            force_full=False,
            observer_updates=(prepared,),
        )

        def fail_after_observer(stage):
            if stage == core_sync.APPLY_STAGE_OBSERVER_PERSISTENCE:
                raise RuntimeError("fault after observer persistence")

        hooks = core_sync.WalletSyncHooks(
            import_file=lambda *_args: {},
            insert_records=lambda *_args, **_kwargs: {"inserted": 0},
            resolve_backend=lambda *_args: fetch.backend,
            resolve_sync_state=lambda *_args: sync_state,
            normalize_addresses=lambda _value: (),
            backend_adapters={},
            after_apply_stage=fail_after_observer,
        )
        with patch.object(
            core_sync.source_overlap,
            "filter_sync_state_for_canonical_owner",
            side_effect=lambda _conn, _profile, _wallet, state: state,
        ):
            with self.assertRaisesRegex(RuntimeError, "fault after observer"):
                cli_handlers._apply_wallet_sync_atomically(
                    self.conn,
                    {},
                    profile,
                    wallet,
                    hooks,
                    prefetched={self.wallet_id: fetch},
                )

        self.assertIsNone(load_observer_state(self.conn, self.identity))
        self.assertTrue(observer.discarded)

    def test_sync_marks_only_prepared_observer_records_as_authoritative(self):
        _observer, prepared = self._prepare()
        wallet = self.conn.execute(
            "SELECT * FROM wallets WHERE id = ?", (self.wallet_id,)
        ).fetchone()
        profile = self.conn.execute(
            "SELECT * FROM profiles WHERE id = ?", (self.profile_id,)
        ).fetchone()
        sync_state = core_sync.WalletSyncState(
            chain="bitcoin",
            network="regtest",
            descriptor_plan=None,
            policy_asset_id="",
            targets=(),
            tracked_scripts={},
            history_cache={},
        )
        fetch = core_sync.WalletBackendFetch(
            backend={"name": "fake", "kind": "fake", "url": "http://127.0.0.1"},
            sync_state=sync_state,
            normalized_records=(),
            adapter_meta={},
            kind="fake",
            started=0.0,
            force_full=False,
            observer_updates=(prepared,),
        )
        insert_calls = []

        def insert_records(*_args, **kwargs):
            insert_calls.append(kwargs)
            return {"imported": 0, "skipped": 0}

        hooks = core_sync.WalletSyncHooks(
            import_file=lambda *_args: {},
            insert_records=insert_records,
            resolve_backend=lambda *_args: fetch.backend,
            resolve_sync_state=lambda *_args: sync_state,
            normalize_addresses=lambda _value: (),
            backend_adapters={},
        )
        self.conn.execute("SAVEPOINT observer_authority")
        with patch.object(
            core_sync.source_overlap,
            "filter_sync_state_for_canonical_owner",
            side_effect=lambda _conn, _profile, _wallet, state: state,
        ):
            core_sync.sync_wallet_from_backend(
                self.conn,
                {},
                profile,
                wallet,
                hooks,
                prefetched=fetch,
            )
        self.conn.execute("ROLLBACK TO SAVEPOINT observer_authority")
        self.conn.execute("RELEASE SAVEPOINT observer_authority")

        self.assertEqual(insert_calls, [{"authoritative_chain_observer": True}])

    def test_sync_fetch_projects_facts_and_rejects_shadow_projection(self):
        observer, prepared = self._prepare()
        sync_state = core_sync.WalletSyncState(
            chain="bitcoin",
            network="regtest",
            descriptor_plan=None,
            policy_asset_id="",
            targets=(),
            tracked_scripts={},
            history_cache={},
        )
        fetch = core_sync.WalletBackendFetch(
            backend={"name": "fake", "kind": "fake", "url": "http://127.0.0.1"},
            sync_state=sync_state,
            normalized_records=(),
            adapter_meta={},
            kind="fake",
            started=0.0,
            force_full=False,
            observer_updates=(prepared,),
        )
        self.conn.execute("SAVEPOINT observer_projection")
        projected = core_sync.apply_fetch_observer_updates(self.conn, fetch)
        self.assertEqual(projected.normalized_records[0]["external_id"], "tx-1")
        self.assertEqual(projected.adapter_meta["observer_retracted_external_ids"], ["old-tx"])
        self.assertEqual(
            projected.adapter_meta["freshness_checkpoint"],
            {
                "observer_instances": {
                    self.identity.id: {"fake_generation": 1}
                }
            },
        )
        self.conn.execute("ROLLBACK TO SAVEPOINT observer_projection")
        self.conn.execute("RELEASE SAVEPOINT observer_projection")
        core_sync.discard_fetch_observer_updates(fetch)
        self.assertTrue(observer.discarded)

        second_observer, second_prepared = self._prepare(FakeObserver(self.conn, marker="second"))
        conflicting = core_sync.WalletBackendFetch(
            backend=fetch.backend,
            sync_state=sync_state,
            normalized_records=({"external_id": "legacy-shadow"},),
            adapter_meta={},
            kind="fake",
            started=0.0,
            force_full=False,
            observer_updates=(second_prepared,),
        )
        self.conn.execute("SAVEPOINT observer_conflict")
        with self.assertRaises(AppError) as conflict:
            core_sync.apply_fetch_observer_updates(self.conn, conflicting)
        self.assertEqual(conflict.exception.code, "observer_projection_conflict")
        self.conn.execute("ROLLBACK TO SAVEPOINT observer_conflict")
        self.conn.execute("RELEASE SAVEPOINT observer_conflict")
        core_sync.discard_fetch_observer_updates(conflicting)
        self.assertTrue(second_observer.discarded)

    def test_empty_observer_output_snapshot_remains_authoritative(self):
        _observer, prepared = self._prepare(FakeObserver(self.conn, outputs=()))
        sync_state = core_sync.WalletSyncState(
            chain="bitcoin",
            network="regtest",
            descriptor_plan=None,
            policy_asset_id="",
            targets=(),
            tracked_scripts={},
            history_cache={},
        )
        fetch = core_sync.WalletBackendFetch(
            backend={"name": "fake", "kind": "fake"},
            sync_state=sync_state,
            normalized_records=(),
            adapter_meta={},
            kind="fake",
            started=0.0,
            force_full=False,
            observer_updates=(prepared,),
        )
        self.conn.execute("SAVEPOINT empty_observer_outputs")
        projected = core_sync.apply_fetch_observer_updates(self.conn, fetch)
        self.assertEqual(projected.adapter_meta["utxos"], [])
        self.conn.execute("ROLLBACK TO SAVEPOINT empty_observer_outputs")
        self.conn.execute("RELEASE SAVEPOINT empty_observer_outputs")

    def test_multi_observer_shared_transaction_is_normalized_once(self):
        script_a = "0014" + "11" * 20
        script_b = "0014" + "22" * 20
        txid = "33" * 32
        raw_base = {
            "txid": txid,
            "vin": [
                {
                    "txid": "44" * 32,
                    "vout": 0,
                    "prevout": {"scriptpubkey": script_a, "value": 10_000},
                    "witness": [],
                },
                {
                    "txid": "55" * 32,
                    "vout": 1,
                    "prevout": {"scriptpubkey": script_b, "value": 20_000},
                    "witness": [],
                },
            ],
            "vout": [{"scriptpubkey": script_b, "value": 29_000}],
            "fee": 1_000,
            "status": {"confirmed": False},
            "observer": "bdk",
        }

        class StaticBdkObserver:
            def __init__(self, owned_script, direction):
                self.owned_script = owned_script
                self.direction = direction

            def prepare(self, _request, _prior_state):
                return {"ready": True}

            def apply(self, _update, _prior_state):
                raw = json.loads(json.dumps(raw_base))
                for vin in raw["vin"]:
                    if vin["prevout"]["scriptpubkey"] != self.owned_script:
                        vin["prevout"] = None
                raw["observer_owned_scripts"] = [self.owned_script]
                return ObserverApplication(
                    state={"schema_version": 1, "owned": self.owned_script},
                    facts=ChainFacts(
                        transaction_records=(
                            {
                                "txid": txid,
                                "asset": "BTC",
                                "direction": self.direction,
                                "amount": "0.00009",
                                "fee": "0.00001",
                                "raw_json": json.dumps(raw, sort_keys=True),
                            },
                        ),
                    ),
                )

            def discard(self):
                return None

        second_identity = ObserverIdentity(
            id="observer-instance-second",
            workspace_id=self.workspace_id,
            profile_id=self.profile_id,
            logical_wallet_id=self.wallet_id,
            source_wallet_id=self.wallet_id,
            source_key="xpub:p2tr",
            observer_kind="bdk",
            chain="bitcoin",
            network="regtest",
            branch_keys=("receive",),
        )
        first = prepare_observer_update(
            self.conn,
            self.identity,
            StaticBdkObserver(script_a, "outbound"),
            ObserverPrepareRequest("regtest", "electrum"),
        )
        second = prepare_observer_update(
            self.conn,
            second_identity,
            StaticBdkObserver(script_b, "inbound"),
            ObserverPrepareRequest("regtest", "electrum"),
        )
        sync_state = core_sync.WalletSyncState(
            chain="bitcoin",
            network="regtest",
            descriptor_plan=None,
            policy_asset_id="",
            targets=(),
            tracked_scripts={},
            history_cache={},
        )
        fetch = core_sync.WalletBackendFetch(
            backend={"name": "regtest", "kind": "electrum", "url": "tcp://127.0.0.1"},
            sync_state=sync_state,
            normalized_records=(),
            adapter_meta={},
            kind="electrum",
            started=0.0,
            force_full=False,
            observer_updates=(first, second),
        )
        self.conn.execute("SAVEPOINT shared_bdk_transaction")
        projected = core_sync.apply_fetch_observer_updates(self.conn, fetch)
        self.assertEqual(len(projected.normalized_records), 1)
        record = projected.normalized_records[0]
        self.assertEqual(record["direction"], "outbound")
        self.assertEqual(str(record["amount"]), "0")
        self.assertEqual(str(record["fee"]), "0.00001")
        merged_raw = json.loads(record["raw_json"])
        self.assertTrue(all(vin.get("prevout") for vin in merged_raw["vin"]))
        self.conn.execute("ROLLBACK TO SAVEPOINT shared_bdk_transaction")
        self.conn.execute("RELEASE SAVEPOINT shared_bdk_transaction")

    def test_unknown_version_and_non_json_state_require_safe_rebuild(self):
        _observer, prepared = self._prepare()
        self._apply_and_commit(prepared)
        self.conn.execute(
            "UPDATE chain_observer_instances SET state_version = 99 WHERE id = ?",
            (self.identity.id,),
        )
        self.conn.commit()
        with self.assertRaises(AppError) as error:
            load_observer_state(self.conn, self.identity)
        self.assertEqual(error.exception.code, "observer_state_rebuild_required")
        self.assertNotIn("observer-private-marker", str(error.exception.details))

        observer = FakeObserver(self.conn)
        prepared = prepare_observer_update(
            self.conn,
            self.identity,
            observer,
            ObserverPrepareRequest(
                backend_name="regtest",
                backend_kind="fake",
                force_full=True,
            ),
        )
        self.assertEqual(
            self.conn.execute(
                "SELECT state_version FROM chain_observer_instances WHERE id = ?",
                (self.identity.id,),
            ).fetchone()[0],
            99,
        )
        self._apply_and_commit(prepared)
        rebuilt = load_observer_state(self.conn, self.identity)
        self.assertEqual(rebuilt.payload["generation"], 1)
        self.assertEqual(
            self.conn.execute(
                "SELECT state_version FROM chain_observer_instances WHERE id = ?",
                (self.identity.id,),
            ).fetchone()[0],
            1,
        )

        self.conn.execute("SAVEPOINT invalid_state")
        with self.assertRaises(AppError) as invalid:
            persist_observer_state(
                self.conn,
                self.identity,
                {"not_json": b"bytes-are-not-a-state-format"},
                (),
            )
        self.assertEqual(invalid.exception.code, "observer_state_invalid")
        self.conn.execute("ROLLBACK TO SAVEPOINT invalid_state")
        self.conn.execute("RELEASE SAVEPOINT invalid_state")

    def test_rebuilding_observer_state_preserves_authored_evidence_and_review(self):
        _observer, prepared = self._prepare()
        self._apply_and_commit(prepared)
        timestamp = now_iso()
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id,
                fingerprint, occurred_at, direction, asset, amount, fee,
                note, raw_json, created_at
            ) VALUES(
                'authored-tx', ?, ?, ?, 'authored-external',
                'authored-fingerprint', ?, 'inbound', 'BTC', 1000, 0,
                'keep this note', '{}', ?
            )
            """,
            (self.workspace_id, self.profile_id, self.wallet_id, timestamp, timestamp),
        )
        self.conn.execute(
            """
            INSERT INTO attachments(
                id, workspace_id, profile_id, transaction_id,
                attachment_type, label, created_at
            ) VALUES('authored-attachment', ?, ?, 'authored-tx', 'url', 'keep evidence', ?)
            """,
            (self.workspace_id, self.profile_id, timestamp),
        )
        self.conn.execute(
            """
            INSERT INTO transaction_edit_events(
                id, workspace_id, profile_id, transaction_id, wallet_id,
                source, reason, changed_at
            ) VALUES(
                'authored-history', ?, ?, 'authored-tx', ?,
                'user', 'keep review history', ?
            )
            """,
            (self.workspace_id, self.profile_id, self.wallet_id, timestamp),
        )
        self.conn.execute(
            """
            INSERT INTO custody_components(
                id, lineage_id, workspace_id, profile_id, revision,
                component_type, notes, created_at
            ) VALUES(
                'authored-component', 'authored-lineage', ?, ?, 1,
                'transfer', 'keep custody interpretation', ?
            )
            """,
            (self.workspace_id, self.profile_id, timestamp),
        )
        delete_wallet_observer_state(self.conn, self.wallet_id)
        self.conn.commit()

        self.assertIsNone(load_observer_state(self.conn, self.identity))
        for table, row_id in (
            ("transactions", "authored-tx"),
            ("attachments", "authored-attachment"),
            ("transaction_edit_events", "authored-history"),
            ("custody_components", "authored-component"),
        ):
            self.assertEqual(
                self.conn.execute(
                    f"SELECT COUNT(*) FROM {table} WHERE id = ?", (row_id,)
                ).fetchone()[0],
                1,
            )

    def test_wallet_delete_and_profile_reset_remove_observer_rows(self):
        _observer, prepared = self._prepare()
        self._apply_and_commit(prepared)
        reset = core_maintenance.reset_current_profile_data(
            self.conn,
            str(self.data_root),
        )
        self.assertEqual(reset["removed"]["chain_observer_instances"], 1)
        self.assertEqual(reset["removed"]["chain_observer_coverage"], 2)
        self.assertEqual(
            self.conn.execute("SELECT COUNT(*) FROM wallets WHERE id = ?", (self.wallet_id,)).fetchone()[0],
            1,
        )
        self.assertIsNone(load_observer_state(self.conn, self.identity))

        _observer, prepared = self._prepare()
        self._apply_and_commit(prepared)
        core_wallets.delete_wallet(
            self.conn,
            self.workspace_id,
            self.profile_id,
            self.wallet_id,
        )
        self.assertEqual(
            self.conn.execute("SELECT COUNT(*) FROM chain_observer_instances").fetchone()[0],
            0,
        )

    def test_public_ai_diagnostics_audit_and_replication_surfaces_exclude_state(self):
        marker = "observer-state-must-never-egress"
        observer, prepared = self._prepare(FakeObserver(self.conn, marker=marker))
        self._apply_and_commit(prepared)
        self.assertFalse(observer.discarded)

        diagnostics = collect_public_diagnostics(
            self.conn,
            Namespace(command="diagnostics", diagnostics_command="collect"),
        )
        wallet_snapshot = build_wallets_list_snapshot(self.conn)
        combined = json.dumps(
            {"diagnostics": diagnostics, "wallets": wallet_snapshot},
            sort_keys=True,
        )
        self.assertNotIn(marker, combined)
        self.assertIn(
            "chain observer state and derivation coverage",
            diagnostics["privacy_contract"]["omits"],
        )
        self.assertIn(
            "chain observer state and derivation coverage",
            audit_package.SENSITIVE_MATERIAL_EXCLUSIONS,
        )
        replicated_tables = {spec.table for spec in SYNC_TABLES}
        self.assertTrue(PRIVATE_OBSERVER_TABLES.isdisjoint(replicated_tables))


class ObserverIdentityTest(unittest.TestCase):
    def _wallet(self, wallet_id, config, *, kind="xpub"):
        return {
            "id": wallet_id,
            "workspace_id": "workspace",
            "profile_id": "profile",
            "kind": kind,
            "config_json": json.dumps(config),
        }

    def test_multi_script_instances_are_stable_and_order_independent(self):
        first = self._wallet(
            "multi-script",
            {
                "chain": "bitcoin",
                "network": "main",
                "xpub": "public-material-not-used-in-identity",
                "script_types": ["p2wpkh", "p2tr"],
            },
        )
        reordered = self._wallet(
            "multi-script",
            {
                "chain": "bitcoin",
                "network": "main",
                "xpub": "different-public-material-still-not-an-id-input",
                "script_types": ["p2tr", "p2wpkh"],
            },
        )
        first_identities = identities_for_wallet(first)
        second_identities = identities_for_wallet(reordered)
        self.assertEqual([item.source_key for item in first_identities], ["xpub:p2tr", "xpub:p2wpkh"])
        self.assertEqual([item.id for item in first_identities], [item.id for item in second_identities])
        self.assertTrue(all(item.branch_keys == ("receive", "change") for item in first_identities))

    def test_samourai_sources_map_to_one_logical_parent_deterministically(self):
        sections = ("deposit", "badbank", "premix", "postmix", "ricochet")
        children = [
            self._wallet(
                f"child-{section}",
                {
                    "chain": "bitcoin",
                    "network": "main",
                    "descriptor": f"descriptor-{section}",
                    "change_descriptor": f"change-{section}",
                    "samourai": {
                        "role": "child",
                        "parent_wallet_id": "samourai-parent",
                        "section": section,
                        "script_type": "p2wpkh",
                        "root_path": f"m/84'/0'/{sections.index(section)}'",
                    },
                },
                kind="descriptor",
            )
            for section in sections
        ]
        identities = identities_for_wallets(tuple(reversed(children)))
        self.assertEqual(len(identities), 5)
        self.assertEqual({item.logical_wallet_id for item in identities}, {"samourai-parent"})
        self.assertEqual({item.source_wallet_id for item in identities}, {f"child-{section}" for section in sections})
        self.assertEqual(
            [item.id for item in identities],
            [item.id for item in identities_for_wallets(children)],
        )


@unittest.skipUnless(sqlcipher_available(), "SQLCipher driver unavailable")
class ObserverSqlcipherRestartTest(unittest.TestCase):
    def test_observer_state_survives_encrypted_restart(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-observer-cipher-") as tmp:
            root = Path(tmp) / "data"
            conn = open_db(root, passphrase="observer-passphrase")
            timestamp = now_iso()
            conn.execute(
                "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'WS', ?)",
                (timestamp,),
            )
            conn.execute(
                """
                INSERT INTO profiles(
                    id, workspace_id, label, fiat_currency, tax_country,
                    tax_long_term_days, gains_algorithm, created_at
                ) VALUES('pf', 'ws', 'PF', 'EUR', 'generic', 365, 'FIFO', ?)
                """,
                (timestamp,),
            )
            conn.execute(
                """
                INSERT INTO wallets(
                    id, workspace_id, profile_id, label, kind, config_json, created_at
                ) VALUES('wallet', 'ws', 'pf', 'Wallet', 'address', '{}', ?)
                """,
                (timestamp,),
            )
            conn.commit()
            identity = ObserverIdentity(
                id="cipher-observer",
                workspace_id="ws",
                profile_id="pf",
                logical_wallet_id="wallet",
                source_wallet_id="wallet",
                source_key="descriptor:default",
                observer_kind="fake",
                chain="bitcoin",
                network="regtest",
                branch_keys=("receive",),
            )
            conn.execute("SAVEPOINT observer_cipher")
            persist_observer_state(
                conn,
                identity,
                {"encoding": "fake-v1", "generation": 7},
                (CoveragePoint("receive", scanned_to=25, highest_used=2),),
            )
            conn.execute("RELEASE SAVEPOINT observer_cipher")
            conn.commit()
            self.assertTrue(
                {path.name for path in root.iterdir()}.issubset(
                    {
                        "kassiber.sqlite3",
                        "kassiber.sqlite3-wal",
                        "kassiber.sqlite3-shm",
                    }
                )
            )
            conn.close()

            reopened = open_db(root, passphrase="observer-passphrase")
            try:
                stored = load_observer_state(reopened, identity)
                self.assertEqual(stored.payload["generation"], 7)
                self.assertEqual(stored.coverage[0].scanned_to, 25)
            finally:
                reopened.close()


if __name__ == "__main__":
    unittest.main()
