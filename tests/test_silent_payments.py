import json
import os
import tempfile
import unittest
from decimal import Decimal
from pathlib import Path

from kassiber.envelope import json_ready
from kassiber.core import accounts as core_accounts
from kassiber.core import output_inventory as core_output_inventory
from kassiber.core import silent_payments
from kassiber.core import sync_backends
from kassiber.core import wallets as core_wallets
from kassiber.core.imports import ImportCoordinatorHooks, import_records_into_wallet
from kassiber.core.sync import WalletSyncHooks, WalletSyncState, sync_wallet_from_backend
from kassiber.db import open_db
from kassiber.diagnostics import sanitize_text
from kassiber.errors import AppError
from kassiber.log_ring import LogRing
from kassiber.redaction import redact_secret_text


SP_DESCRIPTOR = "sp(spscan1q" + ("q" * 40) + ")"
P2TR_SCRIPT = "5120" + ("ab" * 32)


def _import_ui_snapshot():
    try:
        from kassiber.core import ui_snapshot
    except ModuleNotFoundError as exc:
        if exc.name == "embit":
            raise unittest.SkipTest("embit dependency is not installed") from exc
        raise
    return ui_snapshot


def _book(conn):
    workspace = core_accounts.create_workspace(conn, "Main")
    profile = core_accounts.create_profile(
        conn,
        workspace["id"],
        "Book",
        "EUR",
        "FIFO",
        "generic",
        365,
    )
    return workspace, profile


def _sp_config(**overrides):
    config = {
        "sp_descriptor": SP_DESCRIPTOR,
        "backend": "sp-local",
        "chain": "bitcoin",
        "network": "main",
        "sp_scan_start_height": 850_000,
    }
    config.update(overrides)
    return config


def _runtime(scan_file: Path, *, silent_payments_enabled=True):
    backend = {
        "name": "sp-local",
        "kind": "custom",
        "url": "local://silent-payments",
        "chain": "bitcoin",
        "network": "main",
        "silent_payment_scan_file": str(scan_file),
        "source": "test",
    }
    if silent_payments_enabled is not None:
        backend["silent_payments"] = silent_payments_enabled
    return {
        "default_backend": "sp-local",
        "env_file": "test",
        "backends": {"sp-local": backend},
    }


def _import_hooks():
    return ImportCoordinatorHooks(
        ensure_tag_row=lambda conn, workspace_id, profile_id, slug, label: ({"id": slug}, False),
        invalidate_journals=lambda conn, profile_id: None,
    )


def _resolve_backend(runtime_config, name):
    backend_name = (name or runtime_config["default_backend"]).strip().lower()
    return runtime_config["backends"][backend_name]


def _resolve_sync_state(backend, wallet):
    config = json.loads(wallet["config_json"] or "{}")
    plan = silent_payments.build_plan(config)
    kind = str(backend.get("kind") or "")
    silent_payments.validate_backend_capability(backend, plan, kind=kind)
    checkpoint = wallet.get("_freshness_checkpoint") if isinstance(wallet, dict) else None
    return WalletSyncState(
        chain=plan.chain,
        network=plan.network,
        descriptor_plan=plan,
        policy_asset_id="",
        targets=[silent_payments.sync_target(plan)],
        tracked_scripts={},
        history_cache={},
        checkpoint=dict(checkpoint or {}),
    )


def _sp_adapter(backend, wallet, sync_state):
    payload = sync_backends._silent_payment_scan_payload(
        backend,
        wallet,
        sync_state.descriptor_plan,
    )
    return silent_payments.normalize_scan_payload(
        payload,
        backend_name=str(backend["name"]),
        backend_kind=str(backend["kind"]),
        plan=sync_state.descriptor_plan,
        checkpoint=sync_state.checkpoint,
        wallet_id=wallet["id"],
        wallet_label=wallet["label"],
    )


def _sync_hooks():
    return WalletSyncHooks(
        import_file=lambda *args, **kwargs: (_ for _ in ()).throw(AssertionError("not used")),
        insert_records=lambda conn, profile, wallet, records, source_label: import_records_into_wallet(
            conn,
            profile,
            wallet,
            records,
            source_label,
            _import_hooks(),
        ),
        resolve_backend=_resolve_backend,
        resolve_sync_state=_resolve_sync_state,
        normalize_addresses=core_wallets.normalize_addresses,
        backend_adapters={"custom": _sp_adapter},
        update_output_inventory=core_output_inventory.update_wallet_output_inventory,
    )


def _scanner_payload(*, complete=True, spent=False):
    receive_txid = "11" * 32
    spend_txid = "22" * 32
    output = {
        "txid": receive_txid,
        "vout": 0,
        "amount_sats": 50_000,
        "script_pubkey": P2TR_SCRIPT,
        "silent_payment": True,
        "block_height": 850_100,
        "block_time": "2026-06-01T12:00:00Z",
        "confirmations": 6,
    }
    if spent:
        output["spent_by"] = spend_txid
    txs = [
        {
            "txid": receive_txid,
            "block_height": 850_100,
            "block_time": "2026-06-01T12:00:00Z",
            "outputs": [output],
        }
    ]
    if spent:
        txs.append(
            {
                "txid": spend_txid,
                "block_height": 850_120,
                "block_time": "2026-06-02T12:00:00Z",
                "fee_sats": 1_000,
                "inputs": [
                    {
                        "txid": receive_txid,
                        "vout": 0,
                        "amount_sats": 50_000,
                        "silent_payment": True,
                    }
                ],
                "outputs": [
                    {
                        "amount_sats": 49_000,
                        "script_pubkey": "0014" + ("cd" * 20),
                    }
                ],
            }
        )
    return {
        "complete": complete,
        "descriptor_fingerprint": silent_payments.descriptor_fingerprint(SP_DESCRIPTOR),
        "range": {"from_height": 850_000, "to_height": 850_130},
        "transactions": txs,
        "utxos": [output],
    }


def _write_private_scanner_payload(path: Path, payload):
    path.write_text(json.dumps(payload), encoding="utf-8")
    if os.name == "posix":
        path.chmod(0o600)


class SilentPaymentsTests(unittest.TestCase):
    def _db(self):
        tmp = tempfile.TemporaryDirectory(prefix="kassiber-sp-")
        self.addCleanup(tmp.cleanup)
        conn = open_db(Path(tmp.name) / "data")
        self.addCleanup(conn.close)
        return conn

    def test_watch_only_wallet_creation_redacts_material(self):
        conn = self._db()
        workspace, profile = _book(conn)

        created = core_wallets.create_wallet(
            conn,
            workspace["id"],
            profile["id"],
            "SP receive",
            "silent-payment",
            config=_sp_config(),
        )

        self.assertEqual(created["kind"], "silent-payment")
        self.assertEqual(created["config"]["sp_descriptor"], "[redacted]")
        self.assertEqual(created["silent_payment"]["material_format"], "bip392-spscan")
        self.assertNotIn(SP_DESCRIPTOR, json.dumps(created, sort_keys=True))

        revealed = core_wallets.reveal_wallet_secrets(
            conn,
            workspace["id"],
            profile["id"],
            created["id"],
        )
        self.assertEqual(revealed["sp_descriptor"], SP_DESCRIPTOR)

    def test_rejects_spending_material_and_requires_scan_start(self):
        checksummed = silent_payments.validate_watch_only_descriptor(
            SP_DESCRIPTOR + "#abcd1234",
            network="main",
        )
        self.assertEqual(checksummed["sp_descriptor"], SP_DESCRIPTOR)

        signet_two_key = silent_payments.validate_watch_only_descriptor(
            "sp(tprv" + ("q" * 40) + ",tpub" + ("q" * 40) + ")",
            network="signet",
        )
        self.assertEqual(signet_two_key["sp_material_format"], "bip392-two-key-watch-only")

        with self.assertRaises(AppError) as spending:
            silent_payments.validate_watch_only_descriptor("sp(spspend1q" + ("q" * 40) + ")")
        self.assertEqual(spending.exception.code, "validation")

        with self.assertRaises(AppError) as private_spend:
            silent_payments.validate_watch_only_descriptor(
                "sp(K" + ("a" * 40) + ",L" + ("b" * 40) + ")"
            )
        self.assertEqual(private_spend.exception.code, "validation")

        with self.assertRaises(AppError) as wrapped_private_spend:
            silent_payments.validate_watch_only_descriptor(
                "sp(K" + ("a" * 40) + ",musig(K" + ("b" * 40) + ",03" + ("c" * 32) + "))"
            )
        self.assertEqual(wrapped_private_spend.exception.code, "validation")

        with self.assertRaises(AppError) as network_mismatch:
            silent_payments.validate_watch_only_descriptor(
                "sp(tprv" + ("q" * 40) + ",03" + ("c" * 32) + ")",
                network="main",
            )
        self.assertEqual(network_mismatch.exception.code, "validation")

        with self.assertRaises(AppError) as missing_start:
            silent_payments.validate_wallet_config(
                {
                    "sp_descriptor": SP_DESCRIPTOR,
                    "backend": "sp-local",
                    "chain": "bitcoin",
                    "network": "main",
                }
            )
        self.assertEqual(missing_start.exception.code, "silent_payment_scan_start_required")

        with self.assertRaises(AppError) as full_history:
            silent_payments.validate_wallet_config(
                {
                    "sp_descriptor": SP_DESCRIPTOR,
                    "backend": "sp-local",
                    "chain": "bitcoin",
                    "network": "main",
                    "sp_full_history": True,
                }
            )
        self.assertEqual(full_history.exception.code, "silent_payment_full_history_warning_required")

        with self.assertRaises(AppError) as server_warning:
            silent_payments.validate_wallet_config(
                _sp_config(sp_scan_mode="server-assisted")
            )
        self.assertEqual(server_warning.exception.code, "silent_payment_server_warning_required")

    def test_full_history_ignores_stale_scan_start_fields(self):
        validated = silent_payments.validate_wallet_config(
            _sp_config(
                sp_scan_start_height=850_000,
                sp_scan_start_date="2026-06-01T12:00:00Z",
                sp_full_history=True,
                sp_acknowledge_full_history_warning=True,
            )
        )
        self.assertNotIn("sp_scan_start_height", validated)
        self.assertNotIn("sp_scan_start_date", validated)

        plan = silent_payments.build_plan(validated)
        self.assertIsNone(plan.start_height)
        self.assertIsNone(plan.start_date)
        records, meta = silent_payments.normalize_scan_payload(
            {
                "complete": True,
                "descriptor_fingerprint": plan.descriptor_fingerprint,
                "range": {"from_height": 0},
                "transactions": [],
                "utxos": [],
            },
            backend_name="sp-local",
            backend_kind="custom",
            plan=plan,
        )
        self.assertEqual(records, [])
        self.assertTrue(meta["silent_payment_scan_complete"])

    def test_full_history_accepts_genesis_date_range(self):
        plan = silent_payments.build_plan(
            _sp_config(
                sp_full_history=True,
                sp_acknowledge_full_history_warning=True,
            )
        )

        _records, meta = silent_payments.normalize_scan_payload(
            {
                "complete": True,
                "descriptor_fingerprint": plan.descriptor_fingerprint,
                "range": {"from_date": "2009-01-03T00:00:00Z"},
                "transactions": [],
                "utxos": [],
            },
            backend_name="sp-local",
            backend_kind="custom",
            plan=plan,
        )

        self.assertTrue(meta["silent_payment_scan_complete"])

    def test_wallet_label_alone_is_not_a_scanner_binding(self):
        plan = silent_payments.build_plan(_sp_config())

        with self.assertRaises(AppError) as error:
            silent_payments.normalize_scan_payload(
                {
                    "complete": True,
                    "wallet": {"wallet_label": "SP receive"},
                    "range": {"from_height": 850_000, "to_height": 850_130},
                    "transactions": [],
                    "utxos": [],
                },
                backend_name="sp-local",
                backend_kind="custom",
                plan=plan,
                wallet_label="SP receive",
            )

        self.assertEqual(error.exception.code, "silent_payment_scan_wallet_mismatch")

    def test_unsupported_backend_is_explicit_not_zero_balance(self):
        plan = silent_payments.build_plan(_sp_config())
        backend = {
            "name": "ordinary-electrum",
            "kind": "electrum",
            "url": "ssl://example.invalid:50002",
            "chain": "bitcoin",
            "network": "main",
            "source": "test",
        }

        with self.assertRaises(AppError) as error:
            silent_payments.validate_backend_capability(backend, plan, kind="electrum")

        self.assertEqual(error.exception.code, "silent_payment_backend_unsupported")

    def test_server_assisted_rejects_electrum_transport(self):
        plan = silent_payments.build_plan(
            _sp_config(
                sp_scan_mode="server-assisted",
                sp_acknowledge_server_warning=True,
            )
        )
        backend = {
            "name": "electrum-sp",
            "kind": "electrum",
            "url": "ssl://electrum.example:50002",
            "chain": "bitcoin",
            "network": "main",
            "silent_payments": True,
            "silent_payment_scan_path": "/silent-payments/scan",
            "source": "test",
        }

        with self.assertRaises(AppError) as error:
            silent_payments.validate_backend_capability(backend, plan, kind="electrum")

        self.assertEqual(error.exception.code, "silent_payment_backend_unsupported")

    def test_backend_list_exposes_safe_silent_payment_capability(self):
        scan_dir = tempfile.TemporaryDirectory(prefix="kassiber-sp-scan-")
        self.addCleanup(scan_dir.cleanup)
        rows = core_accounts.list_backends(_runtime(Path(scan_dir.name) / "scan.json"))

        self.assertEqual(rows[0]["silent_payments"], True)
        self.assertNotIn("silent_payment_scan_file", rows[0])

        legacy_rows = core_accounts.list_backends(
            _runtime(Path(scan_dir.name) / "legacy-scan.json", silent_payments_enabled=None)
        )
        self.assertEqual(legacy_rows[0]["silent_payments"], True)

        scan_path_backend = _runtime(
            Path(scan_dir.name) / "path-only-scan.json",
            silent_payments_enabled=None,
        )["backends"]["sp-local"]
        scan_path_backend.pop("silent_payment_scan_file")
        scan_path_backend["silent_payment_scan_path"] = "/silent-payments/scan"
        self.assertTrue(silent_payments.backend_supports_silent_payments(scan_path_backend))
        silent_payments.validate_backend_capability(
            scan_path_backend,
            silent_payments.build_plan(_sp_config()),
            kind="custom",
        )

        disabled_backend = _runtime(
            Path(scan_dir.name) / "disabled-scan.json",
            silent_payments_enabled=False,
        )["backends"]["sp-local"]
        disabled_rows = core_accounts.list_backends(
            _runtime(Path(scan_dir.name) / "disabled-scan.json", silent_payments_enabled=False)
        )
        self.assertEqual(disabled_rows[0]["silent_payments"], False)
        self.assertFalse(silent_payments.backend_supports_silent_payments(disabled_backend))
        with self.assertRaises(AppError) as disabled_error:
            silent_payments.validate_backend_capability(
                disabled_backend,
                silent_payments.build_plan(_sp_config()),
                kind="custom",
            )
        self.assertEqual(disabled_error.exception.code, "silent_payment_backend_unsupported")

    @unittest.skipUnless(os.name == "posix", "POSIX file mode checks are required")
    def test_scanner_file_requires_private_permissions(self):
        conn = self._db()
        workspace, profile = _book(conn)
        scan_dir = tempfile.TemporaryDirectory(prefix="kassiber-sp-scan-")
        self.addCleanup(scan_dir.cleanup)
        scan_file = Path(scan_dir.name) / "scan.json"
        scan_file.write_text(json.dumps(_scanner_payload()), encoding="utf-8")
        scan_file.chmod(0o644)
        created = core_wallets.create_wallet(
            conn,
            workspace["id"],
            profile["id"],
            "SP receive",
            "silent-payment",
            config=_sp_config(),
        )
        wallet = conn.execute("SELECT * FROM wallets WHERE id = ?", (created["id"],)).fetchone()

        with self.assertRaises(AppError) as error:
            sync_wallet_from_backend(conn, _runtime(scan_file), profile, wallet, _sync_hooks())

        self.assertEqual(error.exception.code, "silent_payment_scanner_unavailable")
        self.assertFalse(error.exception.retryable)
        self.assertIn("0600", error.exception.hint or "")
        self.assertEqual(error.exception.details.get("mode"), "0o644")

    def test_scanner_payload_imports_transactions_utxos_and_is_idempotent(self):
        conn = self._db()
        workspace, profile = _book(conn)
        scan_dir = tempfile.TemporaryDirectory(prefix="kassiber-sp-scan-")
        self.addCleanup(scan_dir.cleanup)
        scan_file = Path(scan_dir.name) / "scan.json"
        _write_private_scanner_payload(scan_file, _scanner_payload())
        created = core_wallets.create_wallet(
            conn,
            workspace["id"],
            profile["id"],
            "SP receive",
            "silent-payment",
            config=_sp_config(),
        )
        wallet = conn.execute("SELECT * FROM wallets WHERE id = ?", (created["id"],)).fetchone()

        runtime = _runtime(scan_file)
        result = sync_wallet_from_backend(conn, runtime, profile, wallet, _sync_hooks())
        self.assertEqual(result["sync_mode"], "silent_payment")
        self.assertEqual(result["records_fetched"], 1)
        self.assertEqual(result["output_inventory"]["active"], 1)

        tx_rows = conn.execute("SELECT direction, amount, fee FROM transactions ORDER BY occurred_at").fetchall()
        self.assertEqual([(row["direction"], row["amount"], row["fee"]) for row in tx_rows], [("inbound", 50_000_000, 0)])
        utxo = conn.execute("SELECT amount, script_pubkey, spent_at FROM wallet_utxos").fetchone()
        self.assertEqual(utxo["amount"], 50_000_000)
        self.assertEqual(utxo["script_pubkey"], P2TR_SCRIPT)
        self.assertIsNone(utxo["spent_at"])

        sync_wallet_from_backend(conn, runtime, profile, wallet, _sync_hooks())
        self.assertEqual(conn.execute("SELECT COUNT(*) FROM transactions").fetchone()[0], 1)
        self.assertEqual(conn.execute("SELECT COUNT(*) FROM wallet_utxos").fetchone()[0], 1)

        _write_private_scanner_payload(scan_file, _scanner_payload(spent=True))
        result = sync_wallet_from_backend(conn, runtime, profile, wallet, _sync_hooks())
        self.assertEqual(result["records_fetched"], 2)
        self.assertEqual(result["output_inventory"]["active"], 0)
        tx_rows = conn.execute("SELECT direction, amount, fee FROM transactions ORDER BY occurred_at").fetchall()
        self.assertEqual(
            [(row["direction"], row["amount"], row["fee"]) for row in tx_rows],
            [("inbound", 50_000_000, 0), ("outbound", 49_000_000, 1_000_000)],
        )
        utxo = conn.execute("SELECT spent_by, spent_at FROM wallet_utxos").fetchone()
        self.assertEqual(utxo["spent_by"], "22" * 32)
        self.assertIsNotNone(utxo["spent_at"])

    def test_scanner_payload_requires_owned_output_markers(self):
        plan = silent_payments.build_plan(_sp_config())
        records, meta = silent_payments.normalize_scan_payload(
            {
                "complete": True,
                "descriptor_fingerprint": plan.descriptor_fingerprint,
                "transactions": [
                    {
                        "txid": "33" * 32,
                        "block_time": "2026-06-01T12:00:00Z",
                        "outputs": [
                            {
                                "vout": 0,
                                "amount_sats": 50_000,
                                "script_pubkey": P2TR_SCRIPT,
                                "silent_payment": True,
                            },
                            {
                                "vout": 1,
                                "amount_sats": 70_000,
                                "script_pubkey": "5120" + ("cd" * 32),
                            },
                        ],
                    }
                ],
            },
            backend_name="sp-local",
            backend_kind="custom",
            plan=plan,
        )

        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["amount"], Decimal("0.0005"))
        self.assertEqual(meta["silent_payment_outputs_seen"], 1)

    def test_scanner_raw_echoes_are_redacted_before_persistence(self):
        plan = silent_payments.build_plan(_sp_config())
        records, meta = silent_payments.normalize_scan_payload(
            {
                "complete": True,
                "descriptor_fingerprint": plan.descriptor_fingerprint,
                "range": {"request_echo": SP_DESCRIPTOR},
                "transactions": [
                    {
                        "txid": "44" * 32,
                        "block_time": "2026-06-01T12:00:00Z",
                        "request_echo": SP_DESCRIPTOR,
                        "outputs": [
                            {
                                "vout": 0,
                                "amount_sats": 50_000,
                                "script_pubkey": P2TR_SCRIPT,
                                "silent_payment": True,
                                "raw": {
                                    "scanDescriptor": SP_DESCRIPTOR,
                                    "silentPaymentScanKey": "K" + ("a" * 40),
                                    "scanKey": "L" + ("b" * 40),
                                    "note": "keep",
                                },
                            },
                        ],
                    }
                ],
            },
            backend_name="sp-local",
            backend_kind="custom",
            plan=plan,
        )

        persisted = json.dumps(json_ready({"records": records, "meta": meta}), sort_keys=True)
        self.assertNotIn(SP_DESCRIPTOR, persisted)
        self.assertNotIn("K" + ("a" * 40), persisted)
        self.assertNotIn("L" + ("b" * 40), persisted)
        self.assertIn("[redacted-silent-payment-descriptor]", persisted)
        raw = json.loads(records[0]["raw_json"])
        output_raw = raw["outputs"][0]["raw"]
        self.assertEqual(output_raw["silentPaymentScanKey"], "[redacted]")
        self.assertEqual(output_raw["scanKey"], "[redacted]")
        self.assertIn("keep", persisted)

    def test_confirmations_mark_utxos_confirmed_without_block_height(self):
        plan = silent_payments.build_plan(_sp_config())
        records, meta = silent_payments.normalize_scan_payload(
            {
                "complete": True,
                "descriptor_fingerprint": plan.descriptor_fingerprint,
                "range": {"from_height": 850_000, "to_height": 850_130},
                "transactions": [
                    {
                        "txid": "66" * 32,
                        "block_time": "2026-06-01T12:00:00Z",
                        "outputs": [
                            {
                                "vout": 0,
                                "amount_sats": 50_000,
                                "script_pubkey": P2TR_SCRIPT,
                                "silent_payment": True,
                                "confirmations": 3,
                            },
                        ],
                    }
                ],
                "utxos": [
                    {
                        "txid": "66" * 32,
                        "vout": 0,
                        "amount_sats": 50_000,
                        "script_pubkey": P2TR_SCRIPT,
                        "confirmations": 3,
                    },
                ],
            },
            backend_name="sp-local",
            backend_kind="custom",
            plan=plan,
        )

        self.assertEqual(len(records), 1)
        self.assertEqual(meta["utxos"][0]["confirmation_status"], "confirmed")
        self.assertEqual(meta["utxos"][0]["confirmations"], 3)

    def test_server_scan_mode_uses_scan_path_when_local_file_is_configured(self):
        scan_dir = tempfile.TemporaryDirectory(prefix="kassiber-sp-scan-")
        self.addCleanup(scan_dir.cleanup)
        scan_file = Path(scan_dir.name) / "stale-scan.json"
        _write_private_scanner_payload(scan_file, {"stale": True})
        runtime = _runtime(scan_file)
        backend = runtime["backends"]["sp-local"]
        backend["url"] = "https://sp.example.test"
        backend["silent_payment_scan_path"] = "/silent-payments/scan"
        config = _sp_config(
            sp_scan_mode="server-assisted",
            sp_acknowledge_server_warning=True,
        )
        wallet = {"config_json": json.dumps(config)}
        plan = silent_payments.build_plan(config)
        calls = []

        def fake_post_json(url, payload, **kwargs):
            calls.append((url, payload, kwargs))
            return {
                "complete": True,
                "descriptor_fingerprint": plan.descriptor_fingerprint,
                "range": {"from_height": 850_000, "to_height": 850_130},
                "transactions": [],
                "utxos": [],
            }

        original_post_json = sync_backends.http_post_json
        sync_backends.http_post_json = fake_post_json
        self.addCleanup(lambda: setattr(sync_backends, "http_post_json", original_post_json))

        payload = sync_backends._silent_payment_scan_payload(backend, wallet, plan)

        self.assertTrue(payload["complete"])
        self.assertEqual(calls[0][0], "https://sp.example.test/silent-payments/scan")
        self.assertEqual(calls[0][1]["descriptor"], SP_DESCRIPTOR)

    def test_scan_range_must_cover_wallet_birthday(self):
        plan = silent_payments.build_plan(_sp_config(sp_scan_start_height=850_000))
        records, meta = silent_payments.normalize_scan_payload(
            {
                "complete": True,
                "descriptor_fingerprint": plan.descriptor_fingerprint,
                "range": {"from_height": 850_050, "to_height": 850_130},
                "transactions": [],
                "utxos": [],
            },
            backend_name="sp-local",
            backend_kind="custom",
            plan=plan,
        )

        self.assertEqual(records, [])
        self.assertTrue(meta["partial_success"])
        self.assertTrue(meta["blocking_reports"])
        self.assertEqual(meta["silent_payment_degraded_reason"], "scan_range_incomplete")

    def test_spend_transactions_require_explicit_fee(self):
        plan = silent_payments.build_plan(_sp_config())
        with self.assertRaises(AppError) as missing_fee:
            silent_payments.normalize_scan_payload(
                {
                    "complete": True,
                    "descriptor_fingerprint": plan.descriptor_fingerprint,
                    "range": {"from_height": 850_000, "to_height": 850_130},
                    "transactions": [
                        {
                            "txid": "55" * 32,
                            "block_time": "2026-06-02T12:00:00Z",
                            "inputs": [
                                {
                                    "txid": "11" * 32,
                                    "vout": 0,
                                    "amount_sats": 50_000,
                                    "silent_payment": True,
                                }
                            ],
                        }
                    ],
                },
                backend_name="sp-local",
                backend_kind="custom",
                plan=plan,
            )
        self.assertEqual(missing_fee.exception.code, "validation")

    def test_partial_scan_blocks_report_readiness(self):
        conn = self._db()
        workspace, profile = _book(conn)
        scan_dir = tempfile.TemporaryDirectory(prefix="kassiber-sp-scan-")
        self.addCleanup(scan_dir.cleanup)
        scan_file = Path(scan_dir.name) / "scan.json"
        _write_private_scanner_payload(scan_file, _scanner_payload(complete=False))
        created = core_wallets.create_wallet(
            conn,
            workspace["id"],
            profile["id"],
            "SP receive",
            "silent-payment",
            config=_sp_config(),
        )
        wallet = conn.execute("SELECT * FROM wallets WHERE id = ?", (created["id"],)).fetchone()

        result = sync_wallet_from_backend(conn, _runtime(scan_file), profile, wallet, _sync_hooks())
        self.assertTrue(result["partial_success"])
        self.assertTrue(result["blocking_reports"])
        self.assertEqual(result["silent_payment_degraded_reason"], "scan_incomplete")
        self.assertFalse(result["freshness_checkpoint"]["silent_payment"]["scan_complete"])

        self.assertNotIn("output_inventory", result)
        self.assertEqual(result["utxos_skipped_partial"], True)

        ui_snapshot = _import_ui_snapshot()
        source_key = "onchain_wallet:" + wallet["id"]
        conn.execute(
            """
            INSERT INTO freshness_source_states(
                profile_id, source_key, source_type, source_label, status,
                stale_reason, blocking_reports, last_success_at, last_phase,
                progress_json, checkpoint_json, updated_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                profile["id"],
                source_key,
                "onchain_wallet",
                wallet["label"],
                "blocking_reports",
                result["silent_payment_degraded_reason"],
                1,
                "2026-06-01T12:00:00Z",
                "done",
                json.dumps({"phase": "done"}),
                json.dumps(result["freshness_checkpoint"], sort_keys=True),
                "2026-06-01T12:00:00Z",
            ),
        )
        conn.commit()
        blockers = ui_snapshot.build_report_blockers_snapshot(conn)["blockers"]
        self.assertTrue(any(blocker.get("code") == "silent_payment_scan_incomplete" for blocker in blockers))

    def test_partial_scan_does_not_mark_missing_utxos_spent(self):
        conn = self._db()
        workspace, profile = _book(conn)
        scan_dir = tempfile.TemporaryDirectory(prefix="kassiber-sp-scan-")
        self.addCleanup(scan_dir.cleanup)
        scan_file = Path(scan_dir.name) / "scan.json"
        _write_private_scanner_payload(scan_file, _scanner_payload())
        created = core_wallets.create_wallet(
            conn,
            workspace["id"],
            profile["id"],
            "SP receive",
            "silent-payment",
            config=_sp_config(),
        )
        wallet = conn.execute("SELECT * FROM wallets WHERE id = ?", (created["id"],)).fetchone()
        runtime = _runtime(scan_file)

        sync_wallet_from_backend(conn, runtime, profile, wallet, _sync_hooks())
        self.assertIsNone(conn.execute("SELECT spent_at FROM wallet_utxos").fetchone()["spent_at"])

        _write_private_scanner_payload(
            scan_file,
            {
                "complete": "false",
                "descriptor_fingerprint": silent_payments.descriptor_fingerprint(SP_DESCRIPTOR),
                "range": {"from_height": 850_000, "to_height": 850_050},
                "transactions": [],
                "utxos": [],
            },
        )
        result = sync_wallet_from_backend(conn, runtime, profile, wallet, _sync_hooks())
        self.assertTrue(result["partial_success"])
        self.assertEqual(result["utxos_skipped_partial"], True)
        self.assertNotIn("output_inventory", result)
        self.assertIsNone(conn.execute("SELECT spent_at FROM wallet_utxos").fetchone()["spent_at"])

    def test_ai_utxo_redaction_drops_script_and_derivation_metadata(self):
        ui_snapshot = _import_ui_snapshot()

        redacted = ui_snapshot._wallet_utxo_row_for_ai(
            {
                "txid": "11" * 32,
                "vout": 0,
                "address": "bc1qexample",
                "address_label": "Silent Payment",
                "script_pubkey": P2TR_SCRIPT,
                "branch_label": "silent-payment",
                "branch_index": 0,
                "address_index": 42,
                "derivation_path": "m/352h/0h/0h/0/42",
                "derivation_paths": ["m/352h/0h/0h/0/42"],
                "amount": 50_000_000,
            }
        )

        for key in (
            "address",
            "address_label",
            "script_pubkey",
            "branch_label",
            "branch_index",
            "address_index",
            "derivation_path",
            "derivation_paths",
        ):
            self.assertNotIn(key, redacted)
        self.assertEqual(redacted["amount"], 50_000_000)

    def test_redaction_covers_logs_diagnostics_and_public_text(self):
        text = f"descriptor={SP_DESCRIPTOR} spscan1q{'q' * 40} spspend1q{'q' * 40}"

        redacted = redact_secret_text(text)
        sanitized = sanitize_text(text)
        ring = LogRing(max_records=10)
        ring.append("info", "test", "test.py", 1, text)
        log_text = json.dumps(ring.snapshot(), sort_keys=True)

        self.assertNotIn("spscan1q", redacted)
        self.assertNotIn("spspend1q", redacted)
        self.assertNotIn("sp(spscan", redacted)
        self.assertNotIn("spscan1q", sanitized)
        self.assertNotIn("spspend1q", sanitized)
        self.assertNotIn("sp(spscan", sanitized)
        self.assertNotIn("spscan1q", log_text)
        self.assertNotIn("spspend1q", log_text)
        self.assertNotIn("sp(spscan", log_text)


if __name__ == "__main__":
    unittest.main()
