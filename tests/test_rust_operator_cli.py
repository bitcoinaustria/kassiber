"""R6-3 parity: the operator CLI, brokered routing and desktop agent access.

One scenario runs twice on copies of one encrypted book, each side with its
own broker in a disposable 0700 runtime directory:

- **Python:** ``python -m kassiber`` against ``tests/operator_server_fixture.py``,
  and ``python -m kassiber daemon``;
- **Rust:** the hybrid candidate (``tests/rust_port/hybrid_cli.py``) over a
  test-harness ``KASSIBER_RUST_BIN`` against that binary's
  ``--operator-broker-server``, and its daemon (``KASSIBER_HYBRID_RUST_DAEMON``).

Every CLI call's exit code, stdout and stderr, and every daemon record, are
compared byte for byte after declared mappings of what each run generates
afresh: broker generations, lease, project, operation and session ids,
build origins, timestamps, remaining seconds and the copy's path. The hybrid
log must show Rust served every operator leaf and brokered command.

Nothing touches the account's broker endpoint or HOME; agent access uses a
disposable preference file. Ctrl-C (exit 130) is covered by the Rust
end-to-end test ``ctrl_c_cancels_a_queued_brokered_command_with_exit_130``.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from unittest import mock

import pytest

from kassiber.core import accounts as core_accounts
from kassiber.db import open_db, resolve_database_path
from kassiber.errors import AppError
from kassiber.operator.client import BrokerClient
from kassiber.operator.protocol import TEST_RUNTIME_OVERRIDE_ENV
from kassiber.secrets.migration import create_empty_encrypted_database
from kassiber.secrets.sqlcipher import sqlcipher_available

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "tests" / "operator_server_fixture.py"
HYBRID = ROOT / "tests" / "rust_port" / "hybrid_cli.py"
RUST_BIN = Path(os.environ.get("KASSIBER_RUST_BIN", ROOT / "target" / "debug" / "kassiber"))
PASSPHRASE = "correct horse battery staple"

pytestmark = [
    pytest.mark.skipif(os.name == "nt", reason="POSIX broker harness"),
    pytest.mark.skipif(not sqlcipher_available(), reason="SQLCipher is required"),
    pytest.mark.skipif(not RUST_BIN.exists(), reason="build the test-harness binary first"),
]


def _book(directory: Path) -> tuple[str, str]:
    create_empty_encrypted_database(resolve_database_path(directory), PASSPHRASE)
    connection = open_db(directory, passphrase=PASSPHRASE)
    workspace = core_accounts.create_workspace(connection, "Workspace A")
    profile = core_accounts.create_profile(
        connection, workspace["id"], "Book A", "EUR", "FIFO", "generic", 365
    )
    connection.close()
    return workspace["id"], profile["id"]


class Namer:
    """Maps each generated value to a stable name in order of appearance."""

    def __init__(self, root: Path) -> None:
        self.roots = sorted({str(root), str(root.resolve())}, key=len, reverse=True)
        self.names: dict[str, str] = {}

    def name(self, value: str, kind: str) -> str:
        if value not in self.names:
            count = sum(name.startswith(f"<{kind}-") for name in self.names.values())
            self.names[value] = f"<{kind}-{count}>"
        return self.names[value]

    def text(self, text: str) -> str:
        for root in self.roots:
            text = text.replace(root, "<root>")
        text = re.sub(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?Z", "<time>", text)
        text = re.sub(
            r'("(?:idle_)?remaining_seconds": ?)\d+', r'\1"<seconds>"', text
        )
        text = re.sub(
            r"[0-9a-f]{32}\.client\.[0-9a-f]{32}",
            lambda match: self.name(match.group(0), "operation"),
            text,
        )
        text = re.sub(
            r"(?<![0-9a-zA-Z_-])[0-9a-f]{12,32}(?![0-9a-zA-Z_-])",
            lambda match: self.name(match.group(0), f"hex{len(match.group(0))}"),
            text,
        )
        return text

    def record(self, record: object) -> object:
        """A daemon record with its session ids named."""
        if isinstance(record, dict):
            named = {}
            for key, value in record.items():
                if key == "id" and isinstance(value, str):
                    named[key] = self.name(value, "session")
                elif key == "session_id" and isinstance(value, str):
                    named[key] = self.name(value, "session")
                else:
                    named[key] = self.record(value)
            return named
        if isinstance(record, list):
            return [self.record(item) for item in record]
        if isinstance(record, str):
            return self.text(record)
        return record


class Side:
    """One language's CLI, broker and daemon on its own book copy."""

    def __init__(self, name: str, scratch: Path, book: Path) -> None:
        self.name = name
        self.book = book
        self.runtime = scratch / f"rt-{name}"
        self.runtime.mkdir(mode=0o700)
        self.access_file = scratch / f"access-{name}" / "agent-access.json"
        self.log = scratch / f"hybrid-{name}.log"
        self.owners = scratch / f"owners-{name}"
        self.owners.mkdir(mode=0o700)
        # A disposable app-data root: no child resolves the account's.
        self.xdg_data = scratch / f"xdg-{name}"
        self.xdg_data.mkdir(mode=0o700)
        self.broker: subprocess.Popen | None = None
        self.namer = Namer(book)

    @property
    def rust(self) -> bool:
        return self.name == "rust"

    def variables(self) -> dict[str, str]:
        return {
            "KASSIBER_OPERATOR_RUNTIME_DIR": str(self.runtime),
            TEST_RUNTIME_OVERRIDE_ENV: "1",
            "XDG_RUNTIME_DIR": str(self.runtime),
            "KASSIBER_AGENT_ACCESS_FILE": str(self.access_file),
        }

    def environment(self) -> dict[str, str]:
        environment = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith(("KASSIBER_OPERATOR_", "KASSIBER_HYBRID_"))
            and key not in {"KASSIBER_NO_EGRESS", "COLUMNS"}
        }
        environment.update(self.variables())
        environment["PYTHONPATH"] = os.pathsep.join(
            part for part in (str(ROOT), environment.get("PYTHONPATH")) if part
        )
        environment["COLUMNS"] = "100"
        environment["XDG_DATA_HOME"] = str(self.xdg_data)
        if self.rust:
            environment["KASSIBER_RUST_TEST_OPERATOR_SESSION"] = "fixture"
            environment["KASSIBER_RUST_TEST_OWNER_ROOT"] = str(self.owners)
            environment["KASSIBER_HYBRID_RUST_BIN"] = str(RUST_BIN)
            environment["KASSIBER_HYBRID_LOG"] = str(self.log)
        return environment

    def command(self) -> list[str]:
        if self.rust:
            return [sys.executable, str(HYBRID)]
        return [sys.executable, "-m", "kassiber"]

    def cli(self, args: list[str], secrets: dict[int, str] | None = None) -> tuple[int, str, str]:
        """Run the CLI with each `{n}` in `args` naming a pipe that holds
        `secrets[n]`."""
        fds = []
        argv = []
        pipes = {}
        for label, secret in (secrets or {}).items():
            read_fd, write_fd = os.pipe()
            os.write(write_fd, secret.encode())
            os.close(write_fd)
            pipes[label] = read_fd
            fds.append(read_fd)
        for arg in args:
            match = re.fullmatch(r"\{(\d+)\}", arg)
            argv.append(str(pipes[int(match.group(1))]) if match else arg)
        try:
            completed = subprocess.run(
                [*self.command(), *argv],
                stdin=subprocess.DEVNULL,
                capture_output=True,
                pass_fds=fds,
                env=self.environment(),
                timeout=120,
            )
        finally:
            for fd in fds:
                try:
                    os.close(fd)
                except OSError:
                    pass
        return completed.returncode, completed.stdout.decode(), completed.stderr.decode()

    def start_broker(self) -> None:
        command = (
            [str(RUST_BIN), "--operator-broker-server"]
            if self.rust
            else [sys.executable, str(FIXTURE)]
        )
        self.broker = subprocess.Popen(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            env=self.environment(),
        )
        deadline = time.monotonic() + 20
        with self.active():
            while True:
                try:
                    BrokerClient().ping()
                    return
                except (OSError, EOFError, AppError):
                    assert self.broker.poll() is None, "broker exited"
                    assert time.monotonic() < deadline, "broker did not start"
                    time.sleep(0.05)

    def wait_broker_stopped(self) -> None:
        """Until the stopped broker no longer answers; then end its process."""
        deadline = time.monotonic() + 20
        with self.active():
            while True:
                try:
                    BrokerClient().ping()
                except (OSError, EOFError, AppError):
                    break
                assert time.monotonic() < deadline, "broker did not stop"
                time.sleep(0.05)
        self.close()
        self.broker = None

    def close(self) -> None:
        if self.broker is not None and self.broker.poll() is None:
            self.broker.terminate()
            self.broker.wait(timeout=10)

    def active(self):
        return mock.patch.dict(os.environ, self.variables())


class Daemon:
    """One `kassiber daemon` session on the side's book."""

    def __init__(self, side: Side) -> None:
        environment = side.environment()
        if side.rust:
            environment["KASSIBER_HYBRID_RUST_DAEMON"] = "1"
        self.process = subprocess.Popen(
            [*side.command(), "--data-root", str(side.book), "daemon"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            env=environment,
            text=True,
        )
        ready = json.loads(self.process.stdout.readline())
        assert ready["kind"] == "daemon.ready"
        self.count = 0

    def request(self, kind: str, args: object) -> dict:
        self.count += 1
        request_id = f"r{self.count}"
        self.process.stdin.write(
            json.dumps({"request_id": request_id, "kind": kind, "args": args}) + "\n"
        )
        self.process.stdin.flush()
        while True:
            record = json.loads(self.process.stdout.readline())
            if record.get("request_id") == request_id:
                return record

    def close(self) -> None:
        if self.process.poll() is None:
            self.request("daemon.shutdown", {})
            self.process.wait(timeout=20)


def _cli_scenario(side: Side, scope: tuple[str, str]) -> list[tuple[str, int, str, str]]:
    workspace, profile = scope
    root = str(side.book)
    steps: list[tuple[str, int, str, str]] = []
    operation_ids: list[str] = []

    def run(label: str, args: list[str], secrets: dict[int, str] | None = None) -> tuple[int, str, str]:
        code, stdout, stderr = side.cli(["--data-root", root, *args], secrets)
        for line in stderr.splitlines():
            if line.startswith('{"kind":"operator.operation.accepted"'):
                operation_ids.append(json.loads(line)["data"]["operation_id"])
        steps.append((label, code, side.namer.text(stdout), side.namer.text(stderr)))
        return code, stdout, stderr

    pw = {3: PASSPHRASE}
    # No broker: reads and refusals start none.
    run("status stopped", ["--machine", "operator", "status"])
    run("status table", ["operator", "status"])
    run("status all stopped", ["--machine", "operator", "status", "--all"])
    run("lock stopped", ["--machine", "operator", "lock"])
    run("stop stopped", ["--machine", "operator", "stop"])
    run("operation stopped", ["--machine", "operator", "operation", "status", "old.x"])
    run("unlock machine", ["--machine", "operator", "unlock"])
    run("unlock touch id", ["--machine", "operator", "unlock", "--auth", "touch-id"])
    run("touch-id status", ["--machine", "operator", "touch-id", "status"])
    run("touch-id status table", ["operator", "touch-id", "status"])
    run("touch-id enroll machine", ["--machine", "operator", "touch-id", "enroll"])
    run("touch-id forget machine", ["--machine", "operator", "touch-id", "forget"])
    run("mode machine", ["--machine", "operator", "mode", "brokered"])
    run("unlock bad duration", ["--machine", "operator", "unlock", "--passphrase-fd", "{3}", "--duration", "5s"], pw)
    run("unlock short duration", ["--machine", "operator", "unlock", "--passphrase-fd", "{3}", "--duration", "0m"], pw)
    run("unlock table", ["operator", "unlock"])
    assert not any(side.runtime.iterdir()), "a refusal started a broker"

    side.start_broker()
    run("status running", ["--machine", "operator", "status"])
    run("mode wrong", ["--machine", "operator", "mode", "brokered", "--passphrase-fd", "{3}"], {3: "wrong passphrase"})
    run("mode brokered", ["--machine", "operator", "mode", "brokered", "--passphrase-fd", "{3}"], pw)
    run("touch-id status bound", ["--machine", "operator", "touch-id", "status"])
    run("touch-id enroll unsigned", ["--machine", "operator", "touch-id", "enroll", "--passphrase-fd", "{3}"], pw)
    run("touch-id forget unsigned", ["--machine", "operator", "touch-id", "forget", "--passphrase-fd", "{3}"], pw)
    run("no lease", ["--machine", "status"])
    run("no lease table", ["status"])
    run(
        "unlock",
        ["--machine", "operator", "unlock", "--passphrase-fd", "{3}", "--capability", "read", "--duration", "30m"],
        pw,
    )
    run("unlock huge duration", ["--machine", "operator", "unlock", "--passphrase-fd", "{3}", "--duration", "99999999999999999999d"], pw)
    run("status unlocked", ["--machine", "operator", "status"])
    run("status all", ["--machine", "operator", "status", "--all"])
    run("brokered status", ["--machine", "status"])
    run("brokered status table", ["status"])
    run("brokered db fd", ["--machine", "--db-passphrase-fd", "{3}", "status"], pw)
    run("brokered profiles", ["--machine", "profiles", "get", "--workspace", workspace, "--profile", profile])
    run("brokered unscoped", ["--machine", "transactions", "list"])
    run("brokered capability", ["--machine", "context", "set", "--workspace", workspace, "--profile", profile])
    run("admin machine", ["--machine", "secrets", "verify"])
    run("admin wrong", ["--machine", "--operator-auth-fd", "{3}", "secrets", "verify"], {3: "wrong passphrase"})
    run("admin", ["--machine", "--operator-auth-fd", "{3}", "secrets", "verify"], pw)
    run("operation status", ["--machine", "operator", "operation", "status", operation_ids[0]])
    run("operation cancel", ["--machine", "operator", "operation", "cancel", operation_ids[0]])
    run("operation unknown", ["--machine", "operator", "operation", "status", "old.x"])
    run("stop busy", ["--machine", "operator", "stop"])
    run("lock", ["--machine", "operator", "lock"])
    run("after lock", ["--machine", "status"])
    run("mode manual", ["--machine", "operator", "mode", "manual", "--passphrase-fd", "{3}"], pw)
    run("lock table", ["operator", "lock"])
    run("stop", ["--machine", "operator", "stop"])
    side.wait_broker_stopped()
    run("stopped again", ["--machine", "operator", "stop"])
    return steps


def _daemon_scenario(side: Side) -> list[tuple[str, object]]:
    steps: list[tuple[str, object]] = []
    daemon = Daemon(side)
    root = str(side.book)

    def ask(label: str, kind: str, args: object = None) -> dict:
        record = daemon.request(kind, {} if args is None else args)
        steps.append((label, side.namer.record(record)))
        return record

    try:
        ask("status locked", "ui.agent_access.status", {"refresh": True})
        ask("unlock off", "ui.agent_access.unlock")
        ask("configure nothing", "ui.agent_access.configure")
        ask("configure non-boolean", "ui.agent_access.configure", {"mcp_enabled": 1})
        ask("configure unknown", "ui.agent_access.configure", {"mcp_enabled": True, "b": 1, "a": 2})
        ask("configure on", "ui.agent_access.configure", {"mcp_enabled": True})
        ask("unlock without a book", "ui.agent_access.unlock")
        ask("status arguments", "ui.agent_access.status", {"refresh": "yes"})
        ask("unlock arguments", "ui.agent_access.unlock", {"capability": "admin"})
        ask("args not an object", "ui.agent_access.status", [1])
        ask("args falsy", "ui.agent_access.status", [])
        record = daemon.request("daemon.unlock", {"auth_response": {"passphrase_secret": PASSPHRASE}})
        assert record["kind"] == "daemon.unlock", record
        ask("status needed", "ui.agent_access.status")
        ask("pairing without a session", "ui.agent_access.pairing", {"session_id": "s1", "allow": True})
        side.start_broker()
        ask("unlock", "ui.agent_access.unlock")
        ask("unlock again", "ui.agent_access.unlock")
        ask("refresh", "ui.agent_access.status", {"refresh": True})
        with side.active():
            lease = BrokerClient().status(root)
            assert lease["agent_scope"] is True and lease["capability"] == "read", lease
            agent = BrokerClient().open_agent_session(root)
        try:
            ask("refresh waiting", "ui.agent_access.status", {"refresh": True})
            ask("pairing arguments", "ui.agent_access.pairing", {"session_id": "", "allow": True})
            ask("pairing", "ui.agent_access.pairing", {"session_id": agent.session_id, "allow": True})
        finally:
            agent.close()
        ask("lock", "ui.agent_access.lock")
        with side.active():
            assert BrokerClient().status(root)["lease"] == "locked"
        ask("unlock for configure", "ui.agent_access.unlock")
        ask("configure off", "ui.agent_access.configure", {"mcp_enabled": False})
        with side.active():
            assert BrokerClient().status(root)["lease"] == "locked"
        ask("configure ai off", "ui.agent_access.configure", {"mcp_enabled": True, "ai_features_enabled": False})
        ask("unlock ai off", "ui.agent_access.unlock")
    finally:
        daemon.close()
        side.close()
    return steps


@pytest.fixture
def sides():
    with tempfile.TemporaryDirectory(prefix="kb-r63-", dir=os.environ.get("KASSIBER_TEST_SCRATCH")) as scratch:
        scratch = Path(scratch)
        original = scratch / "original"
        original.mkdir()
        scope = _book(original)
        made = {}
        for name in ("python", "rust"):
            # Equal path lengths keep table columns comparable.
            book = scratch / f"book-{name[:2]}"
            shutil.copytree(original, book)
            made[name] = Side(name, scratch, book)
        try:
            yield made, scope, scratch
        finally:
            for side in made.values():
                side.close()


def _app_data_untouched(made: dict) -> None:
    """With explicit data roots, no child writes an app-data root."""
    for side in made.values():
        assert not any(side.xdg_data.rglob("*")), sorted(side.xdg_data.rglob("*"))


def _compare(python: list, rust: list) -> None:
    assert [step[0] for step in rust] == [step[0] for step in python]
    for expected, actual in zip(python, rust):
        assert actual == expected, expected[0]


def test_operator_cli_matches_python_through_both_brokers(sides):
    made, scope, _ = sides
    python = _cli_scenario(made["python"], scope)
    rust = _cli_scenario(made["rust"], scope)

    by_label = {label: (code, stdout, stderr) for label, code, stdout, stderr in python}
    # The scenario reaches each state it means to.
    assert json.loads(by_label["status stopped"][1])["data"]["broker"] == "stopped"
    assert json.loads(by_label["unlock machine"][1])["error"]["code"] == "interaction_required"
    unavailable = {"available": False, "configured": False, "reason": "native_auth_unavailable"}
    assert json.loads(by_label["touch-id status"][1])["data"] == unavailable
    assert json.loads(by_label["touch-id status bound"][1])["data"] == unavailable
    assert json.loads(by_label["touch-id enroll machine"][1])["error"]["code"] == "interaction_required"
    assert json.loads(by_label["touch-id enroll unsigned"][1])["error"]["code"] == "native_auth_unavailable"
    assert json.loads(by_label["no lease"][1])["error"]["details"]["reason"] == "operator_lease_required"
    assert json.loads(by_label["unlock"][1])["data"]["lease"] == "unlocked"
    assert json.loads(by_label["unlock huge duration"][1])["error"]["code"] == "operator_invalid_duration"
    assert json.loads(by_label["brokered status"][1])["kind"] == "status"
    assert by_label["brokered status"][2].startswith('{"kind":"operator.operation.accepted"')
    assert by_label["brokered status table"][2].startswith("operator operation accepted: ")
    assert json.loads(by_label["brokered unscoped"][1])["error"]["code"] == "operator_scope_required"
    assert json.loads(by_label["brokered capability"][1])["error"]["code"] == "operator_capability_denied"
    assert json.loads(by_label["admin machine"][1])["error"]["details"]["reason"] == "fresh_local_authentication"
    assert json.loads(by_label["admin"][1])["kind"] == "secrets.verify"
    assert json.loads(by_label["stop"][1])["data"]["broker"] == "stopping"
    _compare(python, rust)
    _app_data_untouched(made)

    # Rust served every call; none fell back to Python.
    served = [json.loads(line) for line in made["rust"].log.read_text().splitlines()]
    assert {record["served_by"] for record in served} == {"rust"}, served
    assert {
        "operator.status",
        "operator.unlock",
        "operator.lock",
        "operator.stop",
        "operator.mode",
        "operator.operation.status",
        "operator.operation.cancel",
        "operator.touch-id.status",
        "operator.touch-id.enroll",
        "operator.touch-id.forget",
        "status",
        "secrets.verify",
    } <= {record["command"] for record in served}


def test_agent_access_daemon_kinds_match_python_through_both_brokers(sides):
    made, _, _ = sides
    python = _daemon_scenario(made["python"])
    rust = _daemon_scenario(made["rust"])

    by_label = dict(python)
    assert by_label["unlock off"]["error"]["code"] == "mcp_disabled"
    assert by_label["unlock"]["data"]["session"]["active"] is True
    waiting = by_label["refresh waiting"]["data"]["session"]["agents"]
    assert [(agent["pid"], agent["state"]) for agent in waiting] == [(os.getpid(), "pending")]
    assert by_label["pairing"]["data"]["session"]["agents"][0]["state"] == "allowed"
    assert by_label["lock"]["data"]["session"]["active"] is False
    assert by_label["unlock ai off"]["error"]["code"] == "ai_features_disabled"
    _compare(python, rust)
    _app_data_untouched(made)
    assert made["python"].access_file.read_bytes() == made["rust"].access_file.read_bytes()

    served = [json.loads(line) for line in made["rust"].log.read_text().splitlines()]
    assert {(record["command"], record["served_by"]) for record in served} == {("daemon", "rust")}
