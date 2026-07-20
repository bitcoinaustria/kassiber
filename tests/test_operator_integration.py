from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import time
import unittest

from kassiber.db import open_db, resolve_database_path
from kassiber.core import accounts as core_accounts
from kassiber.operator.client import BrokerClient, PreparedArguments
from kassiber.operator.protocol import TEST_RUNTIME_OVERRIDE_ENV
from kassiber.secrets.migration import create_empty_encrypted_database
from kassiber.secrets.sqlcipher import sqlcipher_available


@unittest.skipIf(os.name == "nt", "Unix broker process integration")
@unittest.skipUnless(sqlcipher_available(), "SQLCipher is required")
class OperatorIntegrationTest(unittest.TestCase):
    def test_simultaneous_startup_elects_one_broker(self) -> None:
        with tempfile.TemporaryDirectory() as runtime:
            os.chmod(runtime, 0o700)
            environment = os.environ.copy()
            environment["KASSIBER_OPERATOR_RUNTIME_DIR"] = runtime
            environment[TEST_RUNTIME_OVERRIDE_ENV] = "1"
            environment["XDG_RUNTIME_DIR"] = runtime
            processes = [
                subprocess.Popen(
                    [sys.executable, "-m", "kassiber.operator.server"],
                    stdin=subprocess.DEVNULL,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.PIPE,
                    env=environment,
                )
                for _ in range(2)
            ]
            old_runtime = os.environ.get("KASSIBER_OPERATOR_RUNTIME_DIR")
            old_test_gate = os.environ.get(TEST_RUNTIME_OVERRIDE_ENV)
            os.environ["KASSIBER_OPERATOR_RUNTIME_DIR"] = runtime
            os.environ[TEST_RUNTIME_OVERRIDE_ENV] = "1"
            try:
                deadline = time.monotonic() + 5
                while True:
                    try:
                        BrokerClient().ping()
                        break
                    except OSError:
                        if time.monotonic() >= deadline:
                            raise
                        time.sleep(0.05)
                time.sleep(0.2)
                self.assertEqual(sum(process.poll() is None for process in processes), 1)
            finally:
                for process in processes:
                    if process.poll() is None:
                        process.terminate()
                    process.wait(timeout=5)
                    if process.stderr is not None:
                        process.stderr.close()
                if old_runtime is None:
                    os.environ.pop("KASSIBER_OPERATOR_RUNTIME_DIR", None)
                else:
                    os.environ["KASSIBER_OPERATOR_RUNTIME_DIR"] = old_runtime
                if old_test_gate is None:
                    os.environ.pop(TEST_RUNTIME_OVERRIDE_ENV, None)
                else:
                    os.environ[TEST_RUNTIME_OVERRIDE_ENV] = old_test_gate

    def test_password_unlock_submit_status_and_lock(self) -> None:
        passphrase = bytearray(b"correct horse battery staple")
        with tempfile.TemporaryDirectory() as tmp, tempfile.TemporaryDirectory() as runtime:
            os.chmod(runtime, 0o700)
            create_empty_encrypted_database(
                resolve_database_path(tmp),
                passphrase.decode(),
            )
            connection = open_db(tmp, passphrase=passphrase.decode())
            workspace = core_accounts.create_workspace(connection, "Workspace A")
            profile = core_accounts.create_profile(
                connection,
                workspace["id"],
                "Book A",
                "EUR",
                "FIFO",
                "generic",
                365,
            )
            connection.close()
            environment = os.environ.copy()
            environment["KASSIBER_OPERATOR_RUNTIME_DIR"] = runtime
            environment[TEST_RUNTIME_OVERRIDE_ENV] = "1"
            environment["XDG_RUNTIME_DIR"] = runtime
            server = subprocess.Popen(
                [sys.executable, "-m", "kassiber.operator.server"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.PIPE,
                env=environment,
            )
            old_runtime = os.environ.get("KASSIBER_OPERATOR_RUNTIME_DIR")
            old_test_gate = os.environ.get(TEST_RUNTIME_OVERRIDE_ENV)
            os.environ["KASSIBER_OPERATOR_RUNTIME_DIR"] = runtime
            os.environ[TEST_RUNTIME_OVERRIDE_ENV] = "1"
            client = BrokerClient()
            try:
                deadline = time.monotonic() + 5
                while True:
                    try:
                        client.ping()
                        break
                    except OSError:
                        if time.monotonic() >= deadline:
                            raise
                        time.sleep(0.05)
                unlocked = client.unlock(
                    tmp,
                    passphrase,
                    duration_seconds=None,
                    capability="accounting_decisions",
                    authentication_method="password",
                )
                self.assertEqual(unlocked["lease"], "unlocked")
                self.assertEqual(unlocked["authentication_method"], "password")
                self.assertNotIn(tmp, repr(unlocked))
                second_client = BrokerClient()
                accepted = client.submit(
                    tmp,
                    PreparedArguments(
                        ["--data-root", tmp, "--machine", "status"],
                        {},
                    ),
                    admin_authentication=None,
                )
                accepted_second = second_client.submit(
                    tmp,
                    PreparedArguments(
                        [
                            "--data-root",
                            tmp,
                            "--machine",
                            "health",
                            "--workspace",
                            workspace["id"],
                            "--profile",
                            profile["id"],
                        ],
                        {},
                    ),
                    admin_authentication=None,
                )
                completed = client.wait(accepted["operation_id"])
                completed_second = second_client.wait(
                    accepted_second["operation_id"]
                )
                self.assertEqual(completed["state"], "completed")
                self.assertEqual(json.loads(completed["stdout"])["kind"], "status")
                self.assertEqual(
                    completed_second["state"],
                    "completed",
                    completed_second,
                )
                self.assertEqual(
                    json.loads(completed_second["stdout"])["kind"],
                    "health",
                )
                locked = client.lock(tmp)
                self.assertTrue(locked["locked"])
                self.assertEqual(client.status(tmp)["lease"], "locked")
            finally:
                server.terminate()
                server.wait(timeout=5)
                if server.stderr is not None:
                    server.stderr.close()
                if old_runtime is None:
                    os.environ.pop("KASSIBER_OPERATOR_RUNTIME_DIR", None)
                else:
                    os.environ["KASSIBER_OPERATOR_RUNTIME_DIR"] = old_runtime
                if old_test_gate is None:
                    os.environ.pop(TEST_RUNTIME_OVERRIDE_ENV, None)
                else:
                    os.environ[TEST_RUNTIME_OVERRIDE_ENV] = old_test_gate

    def test_two_projects_and_multiple_books_remain_independent(self) -> None:
        first_passphrase = bytearray(b"first project passphrase")
        second_passphrase = bytearray(b"second project passphrase")
        with (
            tempfile.TemporaryDirectory() as first,
            tempfile.TemporaryDirectory() as second,
            tempfile.TemporaryDirectory() as runtime,
        ):
            os.chmod(runtime, 0o700)
            scopes: list[tuple[str, str]] = []
            for root, passphrase, labels in (
                (first, first_passphrase, ("Workspace A", "Book A")),
                (first, first_passphrase, ("Workspace B", "Book B")),
                (second, second_passphrase, ("Workspace C", "Book C")),
            ):
                database = resolve_database_path(root)
                if not database.exists():
                    create_empty_encrypted_database(database, passphrase.decode())
                connection = open_db(root, passphrase=passphrase.decode())
                workspace = core_accounts.create_workspace(connection, labels[0])
                profile = core_accounts.create_profile(
                    connection,
                    workspace["id"],
                    labels[1],
                    "EUR",
                    "FIFO",
                    "generic",
                    365,
                )
                scopes.append((workspace["id"], profile["id"]))
                connection.close()

            environment = os.environ.copy()
            environment["KASSIBER_OPERATOR_RUNTIME_DIR"] = runtime
            environment[TEST_RUNTIME_OVERRIDE_ENV] = "1"
            environment["XDG_RUNTIME_DIR"] = runtime
            server = subprocess.Popen(
                [sys.executable, "-m", "kassiber.operator.server"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.PIPE,
                env=environment,
            )
            old_runtime = os.environ.get("KASSIBER_OPERATOR_RUNTIME_DIR")
            old_test_gate = os.environ.get(TEST_RUNTIME_OVERRIDE_ENV)
            os.environ["KASSIBER_OPERATOR_RUNTIME_DIR"] = runtime
            os.environ[TEST_RUNTIME_OVERRIDE_ENV] = "1"
            client = BrokerClient()
            try:
                deadline = time.monotonic() + 5
                while True:
                    try:
                        client.ping()
                        break
                    except OSError:
                        if time.monotonic() >= deadline:
                            raise
                        time.sleep(0.05)
                first_status = client.unlock(
                    first,
                    first_passphrase,
                    duration_seconds=None,
                    capability="accounting_decisions",
                    authentication_method="password",
                )
                second_status = client.unlock(
                    second,
                    second_passphrase,
                    duration_seconds=None,
                    capability="accounting_decisions",
                    authentication_method="password",
                )
                self.assertNotEqual(first_status["project"], second_status["project"])

                accepted = []
                for root, (workspace_id, profile_id) in (
                    (first, scopes[0]),
                    (first, scopes[1]),
                    (second, scopes[2]),
                ):
                    accepted.append(
                        client.submit(
                            root,
                            PreparedArguments(
                                [
                                    "--data-root",
                                    root,
                                    "--machine",
                                    "profiles",
                                    "get",
                                    "--workspace",
                                    workspace_id,
                                    "--profile",
                                    profile_id,
                                ],
                                {},
                            ),
                            admin_authentication=None,
                        )
                    )
                completed = [
                    client.wait(item["operation_id"])
                    for item in accepted
                ]
                self.assertTrue(
                    all(item["state"] == "completed" for item in completed),
                    completed,
                )
                returned_profiles = [
                    json.loads(item["stdout"])["data"]["id"]
                    for item in completed
                ]
                self.assertEqual(returned_profiles, [scope[1] for scope in scopes])

                client.lock(first)
                self.assertEqual(client.status(first)["lease"], "locked")
                self.assertEqual(client.status(second)["lease"], "unlocked")
                client.lock(second)
            finally:
                server.terminate()
                server.wait(timeout=5)
                if server.stderr is not None:
                    server.stderr.close()
                if old_runtime is None:
                    os.environ.pop("KASSIBER_OPERATOR_RUNTIME_DIR", None)
                else:
                    os.environ["KASSIBER_OPERATOR_RUNTIME_DIR"] = old_runtime
                if old_test_gate is None:
                    os.environ.pop(TEST_RUNTIME_OVERRIDE_ENV, None)
                else:
                    os.environ[TEST_RUNTIME_OVERRIDE_ENV] = old_test_gate


if __name__ == "__main__":
    unittest.main()
