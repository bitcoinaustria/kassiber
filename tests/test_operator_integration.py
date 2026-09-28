from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

from kassiber.db import open_db, resolve_database_path
from kassiber.core import accounts as core_accounts
from kassiber.errors import AppError
from kassiber.operator.client import BrokerClient, PreparedArguments
from kassiber.operator.protocol import TEST_RUNTIME_OVERRIDE_ENV
from kassiber.secrets.migration import create_empty_encrypted_database
from kassiber.secrets.sqlcipher import sqlcipher_available


BROKER_START_TIMEOUT_SECONDS = 20.0
BROKER_SERVER_COMMAND = [
    sys.executable,
    str(Path(__file__).with_name("operator_server_fixture.py")),
]
SOURCE_ROOT = Path(__file__).resolve().parents[1]


def _broker_environment(runtime: str) -> dict[str, str]:
    environment = os.environ.copy()
    environment["KASSIBER_OPERATOR_RUNTIME_DIR"] = runtime
    environment[TEST_RUNTIME_OVERRIDE_ENV] = "1"
    environment["XDG_RUNTIME_DIR"] = runtime
    existing_pythonpath = environment.get("PYTHONPATH")
    environment["PYTHONPATH"] = os.pathsep.join(
        part
        for part in (str(SOURCE_ROOT), existing_pythonpath)
        if part
    )
    return environment


def _wait_for_broker(
    client: BrokerClient,
    processes: list[subprocess.Popen[bytes]],
) -> None:
    deadline = time.monotonic() + BROKER_START_TIMEOUT_SECONDS
    last_error: BaseException | None = None
    while time.monotonic() < deadline:
        try:
            client.ping()
            return
        except (OSError, EOFError, AppError) as exc:
            last_error = exc
        process_states = [process.poll() for process in processes]
        if all(state is not None for state in process_states):
            raise AssertionError(
                f"operator broker exited before rendezvous: {process_states}"
            ) from last_error
        time.sleep(0.05)
    process_states = [process.poll() for process in processes]
    raise AssertionError(
        f"operator broker did not rendezvous within "
        f"{BROKER_START_TIMEOUT_SECONDS:g}s: {process_states}"
    ) from last_error


@unittest.skipIf(os.name == "nt", "Unix broker process integration")
@unittest.skipUnless(sqlcipher_available(), "SQLCipher is required")
class OperatorIntegrationTest(unittest.TestCase):
    def test_simultaneous_startup_elects_one_broker(self) -> None:
        with tempfile.TemporaryDirectory() as runtime:
            os.chmod(runtime, 0o700)
            environment = _broker_environment(runtime)
            processes = [
                subprocess.Popen(
                    BROKER_SERVER_COMMAND,
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
                _wait_for_broker(BrokerClient(), processes)
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
            environment = _broker_environment(runtime)
            server = subprocess.Popen(
                BROKER_SERVER_COMMAND,
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
                _wait_for_broker(client, [server])
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

    def test_desktop_grant_serves_mcp_and_ends_with_the_desktop_session(self) -> None:
        import threading
        from types import SimpleNamespace

        from kassiber import daemon, daemon_agent_session
        from kassiber.agent_access import PATH_ENV, set_agent_access
        from kassiber.mcp import cli as mcp_cli
        from kassiber.operator.modes import effective_unlock_mode

        passphrase = "correct horse battery staple"
        with (
            tempfile.TemporaryDirectory() as tmp,
            tempfile.TemporaryDirectory() as runtime,
            tempfile.TemporaryDirectory() as config,
        ):
            os.chmod(runtime, 0o700)
            create_empty_encrypted_database(resolve_database_path(tmp), passphrase)
            connection = open_db(tmp, passphrase=passphrase)
            workspace = core_accounts.create_workspace(connection, "Workspace A")
            profile = core_accounts.create_profile(
                connection, workspace["id"], "Book A", "EUR", "FIFO", "generic", 365
            )
            access_file = str(Path(config) / "agent-access.json")
            environment = _broker_environment(runtime)
            # Broker children read the same agent preference as the desktop.
            environment[PATH_ENV] = access_file
            server = subprocess.Popen(
                BROKER_SERVER_COMMAND,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.PIPE,
                env=environment,
            )
            saved = {
                key: os.environ.get(key)
                for key in ("KASSIBER_OPERATOR_RUNTIME_DIR", TEST_RUNTIME_OVERRIDE_ENV, PATH_ENV)
            }
            os.environ["KASSIBER_OPERATOR_RUNTIME_DIR"] = runtime
            os.environ[TEST_RUNTIME_OVERRIDE_ENV] = "1"
            os.environ[PATH_ENV] = access_file
            # What the desktop daemon holds after its own unlock.
            ctx = SimpleNamespace(
                conn=connection, db_passphrase=passphrase, data_root=tmp, agent_lease=None
            )
            provider = mcp_cli.BookToolProvider(
                data_root=tmp,
                project=None,
                env_file=None,
                workspace=workspace["id"],
                profile=profile["id"],
            )
            try:
                _wait_for_broker(BrokerClient(), [server])
                set_agent_access(mcp_enabled=True)
                before = provider.call_tool("status", {}, threading.Event())
                self.assertEqual(
                    before.structured["error"]["details"]["reason"], "database_passphrase"
                )

                state = daemon_agent_session.unlock(ctx)

                self.assertTrue(state["active"])
                # The broker reported the mode its grant replaced.
                self.assertTrue(ctx.agent_lease.restore_manual)
                lease = BrokerClient().status(tmp)
                self.assertEqual(lease["capability"], "read")
                self.assertEqual(
                    lease["duration_seconds"],
                    daemon_agent_session.DESKTOP_AGENT_LEASE_SECONDS,
                )
                self.assertEqual(effective_unlock_mode(tmp), "brokered")
                during = provider.call_tool("status", {}, threading.Event())
                self.assertFalse(during.is_error, during.structured)
                self.assertEqual(during.structured["kind"], "status")
                self.assertEqual(during.structured["book"]["profile"], "Book A")

                # The desktop locking itself ends the agents' lease too.
                daemon._clear_unlocked_passphrase(ctx)

                self.assertEqual(BrokerClient().status(tmp)["lease"], "locked")
                self.assertEqual(effective_unlock_mode(tmp), "manual")
                after = provider.call_tool("status", {}, threading.Event())
                self.assertEqual(
                    after.structured["error"]["details"]["reason"], "database_passphrase"
                )
            finally:
                connection.close()
                server.terminate()
                server.wait(timeout=5)
                if server.stderr is not None:
                    server.stderr.close()
                for key, value in saved.items():
                    if value is None:
                        os.environ.pop(key, None)
                    else:
                        os.environ[key] = value

    def test_desktop_grant_never_replaces_or_ends_a_terminal_lease(self) -> None:
        from types import SimpleNamespace

        from kassiber import daemon_agent_session
        from kassiber.agent_access import PATH_ENV, set_agent_access
        from kassiber.operator.modes import effective_unlock_mode

        passphrase = "correct horse battery staple"
        with (
            tempfile.TemporaryDirectory() as tmp,
            tempfile.TemporaryDirectory() as runtime,
            tempfile.TemporaryDirectory() as config,
        ):
            os.chmod(runtime, 0o700)
            create_empty_encrypted_database(resolve_database_path(tmp), passphrase)
            connection = open_db(tmp, passphrase=passphrase)
            access_file = str(Path(config) / "agent-access.json")
            server = subprocess.Popen(
                BROKER_SERVER_COMMAND,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.PIPE,
                env=_broker_environment(runtime),
            )
            saved = {
                key: os.environ.get(key)
                for key in ("KASSIBER_OPERATOR_RUNTIME_DIR", TEST_RUNTIME_OVERRIDE_ENV, PATH_ENV)
            }
            os.environ["KASSIBER_OPERATOR_RUNTIME_DIR"] = runtime
            os.environ[TEST_RUNTIME_OVERRIDE_ENV] = "1"
            os.environ[PATH_ENV] = access_file
            ctx = SimpleNamespace(
                conn=connection, db_passphrase=passphrase, data_root=tmp, agent_lease=None
            )
            terminal = BrokerClient()
            try:
                _wait_for_broker(terminal, [server])
                set_agent_access(mcp_enabled=True)
                terminal.unlock(
                    tmp,
                    bytearray(passphrase.encode()),
                    duration_seconds=None,
                    capability="accounting_decisions",
                    authentication_method="password",
                )

                # A terminal session exists: the desktop must not narrow it.
                state = daemon_agent_session.unlock(ctx)

                self.assertTrue(state["existing_lease"])
                self.assertIsNone(ctx.agent_lease)
                self.assertEqual(terminal.status(tmp)["capability"], "accounting_decisions")

                # The desktop grants once the book is free; then the terminal
                # unlocks again, replacing the desktop's lease with its own.
                terminal.lock(tmp)
                daemon_agent_session.unlock(ctx)
                self.assertEqual(terminal.status(tmp)["capability"], "read")
                terminal.unlock(
                    tmp,
                    bytearray(passphrase.encode()),
                    duration_seconds=None,
                    capability="operator",
                    authentication_method="password",
                )

                daemon_agent_session.end_lease(ctx)

                # The desktop's session ended; the terminal's did not, and its
                # book stays brokered.
                status = terminal.status(tmp)
                self.assertEqual(status["lease"], "unlocked")
                self.assertEqual(status["capability"], "operator")
                self.assertEqual(effective_unlock_mode(tmp), "brokered")
            finally:
                connection.close()
                server.terminate()
                server.wait(timeout=5)
                if server.stderr is not None:
                    server.stderr.close()
                for key, value in saved.items():
                    if value is None:
                        os.environ.pop(key, None)
                    else:
                        os.environ[key] = value

    def test_desktop_cleanup_leaves_a_mode_chosen_since(self) -> None:
        from types import SimpleNamespace

        from kassiber import daemon_agent_session
        from kassiber.agent_access import PATH_ENV, set_agent_access
        from kassiber.db import database_instance_id
        from kassiber.operator.modes import effective_unlock_mode, set_unlock_mode

        passphrase = "correct horse battery staple"
        with (
            tempfile.TemporaryDirectory() as tmp,
            tempfile.TemporaryDirectory() as runtime,
            tempfile.TemporaryDirectory() as config,
        ):
            os.chmod(runtime, 0o700)
            create_empty_encrypted_database(resolve_database_path(tmp), passphrase)
            connection = open_db(tmp, passphrase=passphrase)
            server = subprocess.Popen(
                BROKER_SERVER_COMMAND,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.PIPE,
                env=_broker_environment(runtime),
            )
            saved = {
                key: os.environ.get(key)
                for key in ("KASSIBER_OPERATOR_RUNTIME_DIR", TEST_RUNTIME_OVERRIDE_ENV, PATH_ENV)
            }
            os.environ["KASSIBER_OPERATOR_RUNTIME_DIR"] = runtime
            os.environ[TEST_RUNTIME_OVERRIDE_ENV] = "1"
            os.environ[PATH_ENV] = str(Path(config) / "agent-access.json")
            ctx = SimpleNamespace(
                conn=connection, db_passphrase=passphrase, data_root=tmp, agent_lease=None
            )
            terminal = BrokerClient()
            try:
                _wait_for_broker(terminal, [server])
                set_agent_access(mcp_enabled=True)
                daemon_agent_session.unlock(ctx)
                # The desktop's lease ends elsewhere, and the user then picks
                # another mode on purpose.
                terminal.lock(tmp)
                set_unlock_mode(
                    tmp, "unattended", database_identity=database_instance_id(connection)
                )

                daemon_agent_session.end_lease(ctx)

                self.assertIsNone(ctx.agent_lease)
                self.assertEqual(effective_unlock_mode(tmp), "unattended")
            finally:
                connection.close()
                server.terminate()
                server.wait(timeout=5)
                if server.stderr is not None:
                    server.stderr.close()
                for key, value in saved.items():
                    if value is None:
                        os.environ.pop(key, None)
                    else:
                        os.environ[key] = value

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

            environment = _broker_environment(runtime)
            server = subprocess.Popen(
                BROKER_SERVER_COMMAND,
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
                _wait_for_broker(client, [server])
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
