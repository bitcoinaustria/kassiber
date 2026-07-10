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
from kassiber.secrets.unlock_store import (
    CLI_REMEMBERED_UNLOCK_SETTING,
    TOUCH_ID_PASSPHRASE_SERVICE,
    delete_remembered_passphrase,
    load_remembered_passphrase,
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

    def tearDown(self):
        keyring.set_keyring(self.original_keyring)

    def test_store_load_delete_round_trip_and_marker(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            data_root.mkdir()
            self.assertTrue(store_remembered_passphrase(data_root, "correct horse"))
            self.assertEqual(load_remembered_passphrase(data_root), "correct horse")
            self.assertEqual(
                remembered_unlock_status(data_root),
                {
                    "platform": PLATFORM_NAME,
                    "available": True,
                    "configured": True,
                    "cli_enabled": False,
                },
            )

            set_cli_remembered_unlock_enabled(data_root, True)
            self.assertTrue(remembered_unlock_status(data_root)["cli_enabled"])
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
            'TOUCH_ID_PASSPHRASE_SERVICE: &str = "Kassiber Database Passphrase"',
            secret_store_source,
        )
        self.assertIn(
            "let normalized = std::fs::canonicalize(&selected).unwrap_or(selected);",
            lib_source,
        )
        self.assertEqual(TOUCH_ID_PASSPHRASE_SERVICE, "Kassiber Database Passphrase")

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
            self.assertEqual(
                remembered_unlock_status(data_root),
                {
                    "platform": PLATFORM_NAME,
                    "available": False,
                    "configured": False,
                    "cli_enabled": False,
                },
            )


class RememberedUnlockCliTests(unittest.TestCase):
    def setUp(self):
        self.original_keyring = keyring.get_keyring()
        self.keyring = MemoryKeyring()
        keyring.set_keyring(self.keyring)

    def tearDown(self):
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
                TOUCH_ID_PASSPHRASE_SERVICE,
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

            payload, returncode, _stderr = _run_cli(data_root, "status")
            self.assertEqual(returncode, 0)
            self.assertEqual(payload["kind"], "status")

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

            payload, returncode, _stderr = _run_cli(data_root, "status")
            self.assertEqual(returncode, 0)

            self.keyring.set_password(
                TOUCH_ID_PASSPHRASE_SERVICE,
                remembered_unlock_account(data_root),
                "stale-passphrase",
            )
            payload, returncode, stderr = _run_cli(
                data_root,
                "status",
                tty=True,
                prompted_passphrase=new_passphrase,
            )
            self.assertEqual(returncode, 0)
            self.assertEqual(payload["kind"], "status")
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
            self.assertFalse(payload["data"]["remembered_unlock"]["cli_enabled"])

            payload, returncode, _stderr = _run_cli(data_root, "status")
            self.assertEqual(returncode, 1)
            self.assertEqual(payload["error"]["code"], "passphrase_required")

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


if __name__ == "__main__":
    unittest.main()
