from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import time
import unittest

from kassiber.db import open_db, resolve_database_path
from kassiber.operator.client import BrokerClient, PreparedArguments
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
            os.environ["KASSIBER_OPERATOR_RUNTIME_DIR"] = runtime
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

    def test_password_unlock_submit_status_and_lock(self) -> None:
        passphrase = bytearray(b"correct horse battery staple")
        with tempfile.TemporaryDirectory() as tmp, tempfile.TemporaryDirectory() as runtime:
            os.chmod(runtime, 0o700)
            create_empty_encrypted_database(
                resolve_database_path(tmp),
                passphrase.decode(),
            )
            connection = open_db(tmp, passphrase=passphrase.decode())
            connection.close()
            environment = os.environ.copy()
            environment["KASSIBER_OPERATOR_RUNTIME_DIR"] = runtime
            server = subprocess.Popen(
                [sys.executable, "-m", "kassiber.operator.server"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.PIPE,
                env=environment,
            )
            old_runtime = os.environ.get("KASSIBER_OPERATOR_RUNTIME_DIR")
            os.environ["KASSIBER_OPERATOR_RUNTIME_DIR"] = runtime
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
                        ["--data-root", tmp, "--machine", "health"],
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
                self.assertEqual(completed_second["state"], "completed")
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


if __name__ == "__main__":
    unittest.main()
