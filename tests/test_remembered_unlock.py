"""Remembered-unlock contract tests using an in-memory keyring backend only."""

from __future__ import annotations

from contextlib import redirect_stderr, redirect_stdout
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import keyring
from keyring.backend import KeyringBackend
from keyring.errors import PasswordDeleteError

from kassiber.cli.main import main
from kassiber.db import load_managed_settings
from kassiber.errors import AppError
from kassiber.secrets.unlock_store import (
    CLI_REMEMBERED_PASSPHRASE_SERVICE,
    CLI_REMEMBERED_UNLOCK_SETTING,
    CLI_LEGACY_UNLOCK_QUARANTINED_SETTING,
    DESKTOP_BIOMETRIC_STALE_SETTING,
    DESKTOP_BIOMETRIC_STALE_MARKER_SERVICE,
    LEGACY_SHARED_PASSPHRASE_SERVICE,
    _backend_is_native,
    cli_legacy_unlock_quarantined,
    delete_remembered_passphrase,
    load_remembered_passphrase,
    mark_desktop_biometric_passphrase_stale,
    refresh_remembered_passphrase_after_rotation,
    remembered_unlock_access_policy,
    remembered_unlock_account,
    remembered_unlock_status,
    set_cli_remembered_unlock_enabled,
    store_remembered_passphrase,
)


ROOT = Path(__file__).resolve().parents[1]
PLATFORM_NAME = (
    "macos"
    if sys.platform == "darwin"
    else "windows"
    if sys.platform == "win32"
    else "linux"
    if sys.platform.startswith("linux")
    else "unsupported"
)
ACCESS_POLICY = {
    "macos": "macos_keychain_application_acl",
    "windows": "windows_dpapi_user_scope",
    "linux": "linux_secret_service_session",
    "unsupported": "unsupported",
}[PLATFORM_NAME]


def _fake_backend(module: str, *, children=()):
    backend_type = type("FakeBackend", (), {"priority": 1})
    backend_type.__module__ = module
    backend = backend_type()
    backend.backends = tuple(children)
    return backend


class MemoryKeyring(KeyringBackend):
    priority = 1

    def __init__(self):
        self.values: dict[tuple[str, str], str] = {}
        self.get_calls = 0
        self.fail_reads = False
        self.fail_writes = False
        self.fail_deletes = False

    def get_password(self, service, username):
        self.get_calls += 1
        if self.fail_reads:
            raise RuntimeError("keyring read failed")
        return self.values.get((service, username))

    def set_password(self, service, username, password):
        if self.fail_writes:
            raise RuntimeError("keyring write failed")
        self.values[(service, username)] = password

    def delete_password(self, service, username):
        if self.fail_deletes:
            raise RuntimeError("keyring delete failed")
        key = (service, username)
        if key not in self.values:
            raise PasswordDeleteError("not found")
        del self.values[key]


class TtyStringIO(io.StringIO):
    def __init__(self, *, is_tty: bool):
        super().__init__("")
        self._is_tty = is_tty

    def isatty(self):
        return self._is_tty


def _passphrase_fd(value: str) -> int:
    read_fd, write_fd = os.pipe()
    os.write(write_fd, value.encode("utf-8"))
    os.close(write_fd)
    return read_fd


def _run_cli(data_root: Path, *args: str, tty=False, prompted_passphrase=None):
    stdout = io.StringIO()
    stderr = io.StringIO()
    stdin = TtyStringIO(is_tty=tty)
    prompt_patch = (
        patch("kassiber.core.runtime.prompt_passphrase", return_value=prompted_passphrase)
        if prompted_passphrase is not None
        else patch("kassiber.core.runtime.prompt_passphrase")
    )
    with redirect_stdout(stdout), redirect_stderr(stderr), patch("sys.stdin", stdin), prompt_patch:
        returncode = main(
            [
                "--data-root",
                str(data_root),
                "--machine",
                *args,
            ]
        )
    output = stdout.getvalue().strip()
    if not output:
        raise AssertionError(f"CLI produced no machine envelope; stderr={stderr.getvalue()!r}")
    return json.loads(output), returncode, stderr.getvalue()


class RememberedUnlockStoreTests(unittest.TestCase):
    def setUp(self):
        self.original_keyring = keyring.get_keyring()
        self.keyring = MemoryKeyring()
        keyring.set_keyring(self.keyring)
        self.native_backend = patch(
            "kassiber.secrets.unlock_store._backend_is_native",
            return_value=True,
        )
        self.native_backend.start()

    def tearDown(self):
        self.native_backend.stop()
        keyring.set_keyring(self.original_keyring)

    def test_store_load_delete_round_trip_and_marker(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            data_root.mkdir()
            self.assertTrue(store_remembered_passphrase(data_root, "correct horse"))
            self.assertIsNone(load_remembered_passphrase(data_root))
            self.assertEqual(
                remembered_unlock_status(data_root),
                {
                    "platform": PLATFORM_NAME,
                    "access_policy": ACCESS_POLICY,
                    "available": True,
                    "configured": False,
                    "cli_enabled": False,
                    "legacy_quarantined": False,
                },
            )

            set_cli_remembered_unlock_enabled(data_root, True)
            self.assertEqual(load_remembered_passphrase(data_root), "correct horse")
            status = remembered_unlock_status(data_root)
            self.assertTrue(status["cli_enabled"])
            self.assertTrue(status["configured"])
            settings = json.loads(
                (Path(root) / "config" / "settings.json").read_text(encoding="utf-8")
            )
            self.assertIs(settings[CLI_REMEMBERED_UNLOCK_SETTING], True)

            self.assertTrue(delete_remembered_passphrase(data_root))
            self.assertIsNone(load_remembered_passphrase(data_root))
            self.assertTrue(delete_remembered_passphrase(data_root))

    def test_account_derivation_matches_desktop_contract(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            data_root.mkdir()
            self.assertEqual(remembered_unlock_account(data_root), str(data_root.resolve()))
        secret_store_source = (ROOT / "ui-tauri/src-tauri/src/secret_store.rs").read_text(
            encoding="utf-8"
        )
        lib_source = (ROOT / "ui-tauri/src-tauri/src/lib.rs").read_text(encoding="utf-8")
        self.assertIn(
            'LEGACY_SHARED_PASSPHRASE_SERVICE: &str = "Kassiber Database Passphrase"',
            secret_store_source,
        )
        self.assertIn(
            'CLI_REMEMBERED_PASSPHRASE_SERVICE: &str = "Kassiber CLI Database Passphrase"',
            secret_store_source,
        )
        self.assertIn(
            '"Kassiber Desktop Biometric Invalidated"',
            secret_store_source,
        )
        self.assertIn(
            "fn touch_id_scope_for_selected(selected: PathBuf) -> TouchIdScope",
            lib_source,
        )
        self.assertEqual(
            LEGACY_SHARED_PASSPHRASE_SERVICE,
            "Kassiber Database Passphrase",
        )
        self.assertEqual(
            CLI_REMEMBERED_PASSPHRASE_SERVICE,
            "Kassiber CLI Database Passphrase",
        )

    def test_enabled_cli_migrates_and_removes_legacy_shared_item(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            data_root.mkdir()
            account = remembered_unlock_account(data_root)
            self.keyring.set_password(
                LEGACY_SHARED_PASSPHRASE_SERVICE,
                account,
                "legacy-passphrase",
            )
            set_cli_remembered_unlock_enabled(data_root, True)

            self.assertEqual(load_remembered_passphrase(data_root), "legacy-passphrase")
            self.assertEqual(
                self.keyring.get_password(CLI_REMEMBERED_PASSPHRASE_SERVICE, account),
                "legacy-passphrase",
            )
            self.assertIsNone(
                self.keyring.get_password(LEGACY_SHARED_PASSPHRASE_SERVICE, account)
            )

    def test_cli_revocation_never_deletes_desktop_biometric_entry(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            data_root.mkdir()
            account = remembered_unlock_account(data_root)
            desktop_service = "Kassiber Desktop Biometric Passphrase"
            self.keyring.set_password(desktop_service, account, "desktop-secret")
            self.assertTrue(store_remembered_passphrase(data_root, "cli-secret"))
            set_cli_remembered_unlock_enabled(data_root, True)

            self.assertTrue(delete_remembered_passphrase(data_root))
            set_cli_remembered_unlock_enabled(data_root, False)
            self.assertEqual(
                self.keyring.get_password(desktop_service, account),
                "desktop-secret",
            )

    def test_backend_errors_degrade_without_raising(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            data_root.mkdir()
            self.keyring.fail_reads = True
            self.keyring.fail_writes = True
            self.keyring.fail_deletes = True
            self.assertIsNone(load_remembered_passphrase(data_root))
            self.assertFalse(store_remembered_passphrase(data_root, "secret"))
            self.assertFalse(delete_remembered_passphrase(data_root))
            set_cli_remembered_unlock_enabled(data_root, True)
            self.assertEqual(
                remembered_unlock_status(data_root),
                {
                    "platform": PLATFORM_NAME,
                    "access_policy": ACCESS_POLICY,
                    "available": False,
                    "configured": False,
                    "cli_enabled": True,
                    "legacy_quarantined": False,
                },
            )

    def test_non_native_backend_is_rejected_without_access(self):
        with tempfile.TemporaryDirectory() as root, patch(
            "kassiber.secrets.unlock_store._backend_is_native",
            return_value=False,
        ):
            data_root = Path(root) / "data"
            data_root.mkdir()
            self.assertFalse(store_remembered_passphrase(data_root, "secret"))
            self.assertIsNone(load_remembered_passphrase(data_root))
            self.assertFalse(delete_remembered_passphrase(data_root))
            self.assertEqual(self.keyring.get_calls, 0)

    def test_platform_policy_accepts_only_the_expected_native_backend(self):
        platform_modules = {
            "macos": "keyring.backends.macOS",
            "windows": "keyring.backends.Windows",
            "linux": "keyring.backends.SecretService",
        }
        for platform, expected_module in platform_modules.items():
            with self.subTest(platform=platform), patch(
                "kassiber.secrets.unlock_store._platform_name",
                return_value=platform,
            ):
                self.assertTrue(_backend_is_native(_fake_backend(expected_module)))
                for other_module in platform_modules.values():
                    if other_module != expected_module:
                        self.assertFalse(_backend_is_native(_fake_backend(other_module)))
                self.assertFalse(_backend_is_native(_fake_backend("keyring.backends.file")))

    def test_native_chainer_rejects_mixed_or_empty_backend_sets(self):
        native = _fake_backend("keyring.backends.Windows")
        file_backend = _fake_backend("keyring.backends.file")
        with patch(
            "kassiber.secrets.unlock_store._platform_name",
            return_value="windows",
        ), patch(
            "kassiber.secrets.unlock_store._backend_is_native",
            side_effect=_backend_is_native,
        ):
            self.assertTrue(
                _backend_is_native(
                    _fake_backend("keyring.backends.chainer", children=(native,))
                )
            )
            self.assertFalse(
                _backend_is_native(
                    _fake_backend(
                        "keyring.backends.chainer",
                        children=(native, file_backend),
                    )
                )
            )
            self.assertFalse(_backend_is_native(_fake_backend("keyring.backends.chainer")))

    def test_access_policy_codes_are_stable_for_every_supported_platform(self):
        expected = {
            "macos": "macos_keychain_application_acl",
            "windows": "windows_dpapi_user_scope",
            "linux": "linux_secret_service_session",
            "unsupported": "unsupported",
        }
        for platform, policy in expected.items():
            with self.subTest(platform=platform), patch(
                "kassiber.secrets.unlock_store._platform_name",
                return_value=platform,
            ):
                self.assertEqual(remembered_unlock_access_policy(), policy)

    def test_desktop_stale_guard_uses_cross_process_managed_settings(self):
        with tempfile.TemporaryDirectory() as root, patch(
            "kassiber.secrets.unlock_store._platform_name",
            return_value="macos",
        ):
            data_root = Path(root) / "data"
            data_root.mkdir()
            generation = mark_desktop_biometric_passphrase_stale(data_root)
            self.assertIsInstance(generation, str)
            self.assertEqual(
                load_managed_settings(data_root)[DESKTOP_BIOMETRIC_STALE_SETTING],
                generation,
            )

    def test_password_delete_error_is_failure_when_credential_remains(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            data_root.mkdir()
            account = remembered_unlock_account(data_root)
            self.keyring.set_password(
                CLI_REMEMBERED_PASSPHRASE_SERVICE,
                account,
                "still-present",
            )
            with patch.object(
                self.keyring,
                "delete_password",
                side_effect=PasswordDeleteError("access denied"),
            ):
                self.assertFalse(delete_remembered_passphrase(data_root))
            self.assertEqual(
                self.keyring.get_password(CLI_REMEMBERED_PASSPHRASE_SERVICE, account),
                "still-present",
            )

    def test_nominal_delete_is_failure_when_credential_remains(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            data_root.mkdir()
            account = remembered_unlock_account(data_root)
            self.keyring.set_password(
                CLI_REMEMBERED_PASSPHRASE_SERVICE,
                account,
                "still-present",
            )
            with patch.object(self.keyring, "delete_password", return_value=None):
                self.assertFalse(delete_remembered_passphrase(data_root))
            self.assertEqual(
                self.keyring.get_password(CLI_REMEMBERED_PASSPHRASE_SERVICE, account),
                "still-present",
            )


class RememberedUnlockCliTests(unittest.TestCase):
    def setUp(self):
        self.original_keyring = keyring.get_keyring()
        self.keyring = MemoryKeyring()
        keyring.set_keyring(self.keyring)
        self.native_backend = patch(
            "kassiber.secrets.unlock_store._backend_is_native",
            return_value=True,
        )
        self.native_backend.start()

    def tearDown(self):
        self.native_backend.stop()
        keyring.set_keyring(self.original_keyring)

    def test_enroll_unlock_rotate_stale_fallback_and_forget(self):
        old_passphrase = "old-passphrase-123"
        new_passphrase = "new-passphrase-456"
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"

            init_fd = _passphrase_fd(old_passphrase)
            payload, returncode, _stderr = _run_cli(
                data_root,
                "secrets",
                "init",
                "--new-passphrase-fd",
                str(init_fd),
            )
            self.assertEqual(returncode, 0)
            self.assertEqual(payload["kind"], "secrets.init")

            # Desktop enrollment alone must not opt the CLI into credential reads.
            self.keyring.set_password(
                LEGACY_SHARED_PASSPHRASE_SERVICE,
                remembered_unlock_account(data_root),
                old_passphrase,
            )
            reads_before = self.keyring.get_calls
            payload, returncode, _stderr = _run_cli(data_root, "status")
            self.assertEqual(returncode, 1)
            self.assertEqual(payload["error"]["code"], "passphrase_required")
            self.assertEqual(self.keyring.get_calls, reads_before)

            enroll_fd = _passphrase_fd(old_passphrase)
            payload, returncode, _stderr = _run_cli(
                data_root,
                "secrets",
                "remember-unlock",
                "--passphrase-fd",
                str(enroll_fd),
            )
            self.assertEqual(returncode, 0)
            self.assertEqual(payload["kind"], "secrets.remember-unlock")
            self.assertTrue(payload["data"]["remembered_unlock"]["cli_enabled"])
            self.assertIsNone(
                self.keyring.get_password(
                    LEGACY_SHARED_PASSPHRASE_SERVICE,
                    remembered_unlock_account(data_root),
                )
            )

            payload, returncode, _stderr = _run_cli(data_root, "secrets", "status")
            self.assertEqual(returncode, 0)
            self.assertEqual(payload["kind"], "secrets.status")
            self.assertEqual(
                set(payload["data"]["remembered_unlock"]),
                {
                    "platform",
                    "access_policy",
                    "available",
                    "configured",
                    "cli_enabled",
                    "legacy_quarantined",
                },
            )

            payload, returncode, _stderr = _run_cli(data_root, "status")
            self.assertEqual(returncode, 0)
            self.assertEqual(payload["kind"], "status")

            backup_fd = _passphrase_fd("outer-backup-passphrase")
            backup_path = Path(root) / "remembered-unlock.kassiber"
            payload, returncode, _stderr = _run_cli(
                data_root,
                "backup",
                "export",
                "--file",
                str(backup_path),
                "--backup-passphrase-fd",
                str(backup_fd),
            )
            self.assertEqual(returncode, 0)
            self.assertEqual(payload["kind"], "backup.export")
            self.assertTrue(backup_path.is_file())

            current_fd = _passphrase_fd(old_passphrase)
            new_fd = _passphrase_fd(new_passphrase)
            payload, returncode, _stderr = _run_cli(
                data_root,
                "--db-passphrase-fd",
                str(current_fd),
                "secrets",
                "change-passphrase",
                "--new-passphrase-fd",
                str(new_fd),
            )
            self.assertEqual(returncode, 0)
            self.assertEqual(payload["kind"], "secrets.change-passphrase")
            self.assertEqual(load_remembered_passphrase(data_root), new_passphrase)
            self.assertEqual(
                payload["data"]["desktop_biometric_invalidated"],
                PLATFORM_NAME == "macos",
            )
            self.assertIsNone(
                self.keyring.get_password(
                    DESKTOP_BIOMETRIC_STALE_MARKER_SERVICE,
                    remembered_unlock_account(data_root),
                )
            )
            stale_generation = load_managed_settings(data_root).get(
                DESKTOP_BIOMETRIC_STALE_SETTING
            )
            if PLATFORM_NAME == "macos":
                self.assertIsInstance(stale_generation, str)
                self.assertEqual(
                    payload["data"]["desktop_biometric_stale_generation"],
                    stale_generation,
                )
            else:
                self.assertIsNone(stale_generation)

            payload, returncode, _stderr = _run_cli(data_root, "status")
            self.assertEqual(returncode, 0)

            self.keyring.set_password(
                CLI_REMEMBERED_PASSPHRASE_SERVICE,
                remembered_unlock_account(data_root),
                "stale-passphrase",
            )
            payload, returncode, stderr = _run_cli(
                data_root,
                "status",
                tty=True,
                prompted_passphrase=new_passphrase,
            )
            self.assertEqual(returncode, 1)
            self.assertEqual(payload["error"]["code"], "passphrase_required")
            self.assertIn("remembered_unlock_stale", stderr)

            # An explicit fd always wins, so the stale store is not consulted.
            explicit_fd = _passphrase_fd(new_passphrase)
            payload, returncode, stderr = _run_cli(
                data_root,
                "--db-passphrase-fd",
                str(explicit_fd),
                "status",
            )
            self.assertEqual(returncode, 0)
            self.assertNotIn("remembered_unlock_stale", stderr)

            payload, returncode, _stderr = _run_cli(
                data_root,
                "secrets",
                "forget-unlock",
            )
            self.assertEqual(returncode, 0)
            self.assertEqual(payload["kind"], "secrets.forget-unlock")
            self.assertTrue(payload["data"]["cli_marker_cleared"])
            self.assertFalse(payload["data"]["remembered_unlock"]["cli_enabled"])
            self.assertIsNone(
                self.keyring.get_password(
                    LEGACY_SHARED_PASSPHRASE_SERVICE,
                    remembered_unlock_account(data_root),
                )
            )

            payload, returncode, _stderr = _run_cli(data_root, "status")
            self.assertEqual(returncode, 1)
            self.assertEqual(payload["error"]["code"], "passphrase_required")

    def test_enrollment_rolls_back_when_legacy_cleanup_fails(self):
        passphrase = "legacy-cleanup-failure-passphrase"
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            init_fd = _passphrase_fd(passphrase)
            payload, returncode, _stderr = _run_cli(
                data_root,
                "secrets",
                "init",
                "--new-passphrase-fd",
                str(init_fd),
            )
            self.assertEqual(returncode, 0)

            enroll_fd = _passphrase_fd(passphrase)
            with patch(
                "kassiber.secrets.cli.delete_legacy_shared_passphrase",
                return_value=False,
            ):
                payload, returncode, _stderr = _run_cli(
                    data_root,
                    "secrets",
                    "remember-unlock",
                    "--passphrase-fd",
                    str(enroll_fd),
                )

            self.assertEqual(returncode, 1)
            self.assertEqual(
                payload["error"]["code"],
                "remembered_unlock_legacy_cleanup_failed",
            )
            self.assertFalse(remembered_unlock_status(data_root)["cli_enabled"])
            self.assertIsNone(
                self.keyring.get_password(
                    CLI_REMEMBERED_PASSPHRASE_SERVICE,
                    remembered_unlock_account(data_root),
                )
            )

            payload, returncode, _stderr = _run_cli(data_root, "status")
            self.assertEqual(returncode, 1)
            self.assertEqual(payload["error"]["code"], "passphrase_required")

    def test_enrollment_deletes_credential_when_marker_write_fails(self):
        passphrase = "marker-write-failure-passphrase"
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            init_fd = _passphrase_fd(passphrase)
            _run_cli(
                data_root,
                "secrets",
                "init",
                "--new-passphrase-fd",
                str(init_fd),
            )

            enroll_fd = _passphrase_fd(passphrase)
            with patch(
                "kassiber.secrets.cli.set_cli_remembered_unlock_enabled",
                side_effect=OSError("settings are read-only"),
            ):
                payload, returncode, _stderr = _run_cli(
                    data_root,
                    "secrets",
                    "remember-unlock",
                    "--passphrase-fd",
                    str(enroll_fd),
                )

            self.assertEqual(returncode, 1)
            self.assertEqual(
                payload["error"]["code"],
                "remembered_unlock_settings_failed",
            )
            self.assertTrue(payload["error"]["details"]["credential_deleted"])
            self.assertIsNone(load_remembered_passphrase(data_root))

    def test_forget_attempts_credential_delete_when_marker_clear_fails(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            data_root.mkdir()
            self.assertTrue(store_remembered_passphrase(data_root, "secret"))
            set_cli_remembered_unlock_enabled(data_root, True)

            with patch(
                "kassiber.secrets.cli.set_cli_unlock_state",
                side_effect=OSError("settings are read-only"),
            ):
                payload, returncode, _stderr = _run_cli(
                    data_root,
                    "secrets",
                    "forget-unlock",
                )

            self.assertEqual(returncode, 1)
            self.assertEqual(
                payload["error"]["code"],
                "remembered_unlock_settings_failed",
            )
            self.assertFalse(payload["error"]["details"]["cli_marker_cleared"])
            self.assertTrue(payload["error"]["details"]["credential_deleted"])
            self.assertIsNone(load_remembered_passphrase(data_root))

    def test_forget_disables_cli_and_warns_when_credential_delete_fails(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            data_root.mkdir()
            account = remembered_unlock_account(data_root)
            self.assertTrue(store_remembered_passphrase(data_root, "secret"))
            set_cli_remembered_unlock_enabled(data_root, True)

            with patch(
                "kassiber.secrets.cli.delete_remembered_passphrase",
                return_value=False,
            ):
                payload, returncode, _stderr = _run_cli(
                    data_root,
                    "secrets",
                    "forget-unlock",
                )

            self.assertEqual(returncode, 0)
            self.assertTrue(payload["data"]["cli_marker_cleared"])
            self.assertFalse(payload["data"]["credential_deleted"])
            self.assertIn("warning", payload["data"])
            self.assertFalse(payload["data"]["remembered_unlock"]["cli_enabled"])
            self.assertEqual(
                self.keyring.get_password(CLI_REMEMBERED_PASSPHRASE_SERVICE, account),
                "secret",
            )

    def test_forget_deletes_cli_owned_legacy_credential_before_clearing_marker(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            data_root.mkdir()
            account = remembered_unlock_account(data_root)
            self.keyring.set_password(
                LEGACY_SHARED_PASSPHRASE_SERVICE,
                account,
                "legacy-secret",
            )
            set_cli_remembered_unlock_enabled(data_root, True)

            payload, returncode, _stderr = _run_cli(
                data_root,
                "secrets",
                "forget-unlock",
            )

            self.assertEqual(returncode, 0)
            self.assertTrue(payload["data"]["legacy_credential_deleted"])
            self.assertFalse(payload["data"]["remembered_unlock"]["cli_enabled"])
            self.assertIsNone(
                self.keyring.get_password(LEGACY_SHARED_PASSPHRASE_SERVICE, account)
            )

    def test_forget_quarantines_owned_legacy_when_delete_fails(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            data_root.mkdir()
            set_cli_remembered_unlock_enabled(data_root, True)

            with patch(
                "kassiber.secrets.cli.delete_legacy_shared_passphrase",
                return_value=False,
            ):
                payload, returncode, _stderr = _run_cli(
                    data_root,
                    "secrets",
                    "forget-unlock",
                )

            self.assertEqual(returncode, 1)
            self.assertEqual(
                payload["error"]["code"],
                "remembered_unlock_legacy_cleanup_failed",
            )
            status = remembered_unlock_status(data_root)
            self.assertFalse(status["cli_enabled"])
            self.assertTrue(status["legacy_quarantined"])
            self.assertTrue(cli_legacy_unlock_quarantined(data_root))
            self.assertIsNone(load_remembered_passphrase(data_root))

    def test_rotation_store_failure_disables_cli_copy_but_keeps_new_db_key(self):
        old_passphrase = "rotation-old-pass"
        new_passphrase = "rotation-new-pass"
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            init_fd = _passphrase_fd(old_passphrase)
            _run_cli(
                data_root,
                "secrets",
                "init",
                "--new-passphrase-fd",
                str(init_fd),
            )
            self.assertTrue(store_remembered_passphrase(data_root, old_passphrase))
            set_cli_remembered_unlock_enabled(data_root, True)
            self.keyring.fail_writes = True

            current_fd = _passphrase_fd(old_passphrase)
            new_fd = _passphrase_fd(new_passphrase)
            payload, returncode, stderr = _run_cli(
                data_root,
                "--db-passphrase-fd",
                str(current_fd),
                "secrets",
                "change-passphrase",
                "--new-passphrase-fd",
                str(new_fd),
            )
            self.assertEqual(returncode, 0)
            self.assertEqual(
                payload["data"]["remembered_unlock_warning"]["code"],
                "remembered_unlock_update_failed",
            )
            self.assertIn("remembered_unlock_update_failed", stderr)
            self.assertFalse(remembered_unlock_status(data_root)["cli_enabled"])

            verify_fd = _passphrase_fd(new_passphrase)
            payload, returncode, _stderr = _run_cli(
                data_root,
                "--db-passphrase-fd",
                str(verify_fd),
                "status",
            )
            self.assertEqual(returncode, 0)
            self.assertEqual(payload["kind"], "status")

    def test_ambiguous_rotation_failure_keeps_desktop_stale_generation(self):
        old_passphrase = "rotation-old-pass"
        new_passphrase = "rotation-new-pass"
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            init_fd = _passphrase_fd(old_passphrase)
            _run_cli(
                data_root,
                "secrets",
                "init",
                "--new-passphrase-fd",
                str(init_fd),
            )

            current_fd = _passphrase_fd(old_passphrase)
            new_fd = _passphrase_fd(new_passphrase)
            with patch(
                "kassiber.secrets.unlock_store._platform_name",
                return_value="macos",
            ), patch(
                "kassiber.secrets.cli.change_database_passphrase",
                side_effect=AppError(
                    "verification failed after rekey",
                    code="rekey_verification_failed",
                ),
            ):
                payload, returncode, _stderr = _run_cli(
                    data_root,
                    "--db-passphrase-fd",
                    str(current_fd),
                    "secrets",
                    "change-passphrase",
                    "--new-passphrase-fd",
                    str(new_fd),
                )

            self.assertEqual(returncode, 1)
            self.assertEqual(payload["error"]["code"], "rekey_verification_failed")
            self.assertIsInstance(
                load_managed_settings(data_root).get(
                    DESKTOP_BIOMETRIC_STALE_SETTING
                ),
                str,
            )

    def test_rotation_refresh_quarantines_legacy_cleanup_failure(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            data_root.mkdir()
            set_cli_remembered_unlock_enabled(data_root, True)
            with patch(
                "kassiber.secrets.unlock_store.store_remembered_passphrase",
                return_value=False,
            ), patch(
                "kassiber.secrets.unlock_store.delete_remembered_passphrase",
                return_value=True,
            ), patch(
                "kassiber.secrets.unlock_store.delete_legacy_shared_passphrase",
                return_value=False,
            ):
                warning = refresh_remembered_passphrase_after_rotation(
                    data_root,
                    "new-passphrase",
                )

            self.assertIsNotNone(warning)
            self.assertFalse(warning["legacy_credential_deleted"])
            self.assertTrue(warning["cli_marker_cleared"])
            self.assertTrue(warning["legacy_quarantined"])
            status = remembered_unlock_status(data_root)
            self.assertFalse(status["cli_enabled"])
            self.assertTrue(status["legacy_quarantined"])
            self.assertIsNone(load_remembered_passphrase(data_root))
            self.assertIs(
                load_managed_settings(data_root).get(
                    CLI_LEGACY_UNLOCK_QUARANTINED_SETTING
                ),
                True,
            )


if __name__ == "__main__":
    unittest.main()
