"""R6-2 parity: the Rust operator broker against the Python broker.

One scenario of broker actions runs twice, through the same Python client,
once against ``tests/operator_server_fixture.py`` and once against
``KASSIBER_RUST_BIN --operator-broker-server``, each on its own copy of one
encrypted book. Every reply, error, operation record and continuation is
compared after declared mappings of what each run generates afresh (broker
generations, operation ids, lease ids, project ids from file identities,
timestamps, build identities and the copy's path).

The children are compared too: each brokered command's envelope from the
Rust broker equals a direct run of the same Rust binary on a third copy,
and the Python broker's children (the Python CLI) produce the same
envelopes for the commands both CLIs serve.

Every endpoint is a disposable 0700 runtime directory selected through the
gated test override; nothing touches the account's endpoint or HOME.
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
from kassiber.operator.client import BrokerClient, PreparedArguments
from kassiber.operator.protocol import PROTOCOL_VERSION, TEST_RUNTIME_OVERRIDE_ENV, connect
from kassiber.secrets.migration import create_empty_encrypted_database
from kassiber.secrets.sqlcipher import sqlcipher_available

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "tests" / "operator_server_fixture.py"
RUST_BIN = Path(os.environ.get("KASSIBER_RUST_BIN", ROOT / "target" / "debug" / "kassiber"))
PASSPHRASE = "correct horse battery staple"

pytestmark = [
    pytest.mark.skipif(os.name == "nt", reason="POSIX broker harness"),
    pytest.mark.skipif(not sqlcipher_available(), reason="SQLCipher is required"),
    pytest.mark.skipif(not RUST_BIN.exists(), reason="build the test-harness binary first"),
]


class Broker:
    """One broker process on its own runtime directory."""

    def __init__(self, command: list[str], runtime: Path, extra: dict[str, str]) -> None:
        self.runtime = runtime
        environment = os.environ.copy()
        environment.update(self.variables())
        environment.update(extra)
        environment["PYTHONPATH"] = os.pathsep.join(
            part for part in (str(ROOT), environment.get("PYTHONPATH")) if part
        )
        self.process = subprocess.Popen(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            env=environment,
        )
        deadline = time.monotonic() + 20
        while True:
            try:
                with self.active():
                    self.build = BrokerClient().ping()["build"]
                return
            except (OSError, EOFError, AppError):
                if self.process.poll() is not None or time.monotonic() > deadline:
                    raise AssertionError(self.process.stderr.read().decode())
                time.sleep(0.05)

    def variables(self) -> dict[str, str]:
        return {
            "KASSIBER_OPERATOR_RUNTIME_DIR": str(self.runtime),
            TEST_RUNTIME_OVERRIDE_ENV: "1",
            "XDG_RUNTIME_DIR": str(self.runtime),
        }

    def active(self):
        """This broker's endpoint, and its build as the client's own."""

        class Active:
            def __enter__(inner):
                inner.environ = mock.patch.dict(os.environ, self.variables())
                inner.build = mock.patch(
                    "kassiber.operator.client.build_identity",
                    side_effect=lambda: getattr(self, "build", None),
                )
                inner.environ.start()
                inner.build.start()

            def __exit__(inner, *_exc):
                inner.build.stop()
                inner.environ.stop()

        return Active()

    def close(self) -> None:
        if self.process.poll() is None:
            self.process.terminate()
        self.process.wait(timeout=10)
        self.process.stderr.close()


def _book(directory: Path) -> tuple[str, str]:
    create_empty_encrypted_database(resolve_database_path(directory), PASSPHRASE)
    connection = open_db(directory, passphrase=PASSPHRASE)
    workspace = core_accounts.create_workspace(connection, "Workspace A")
    profile = core_accounts.create_profile(
        connection, workspace["id"], "Book A", "EUR", "FIFO", "generic", 365
    )
    connection.close()
    return workspace["id"], profile["id"]


@pytest.fixture
def books():
    with tempfile.TemporaryDirectory(prefix="kb-op-") as scratch:
        scratch = Path(scratch)
        original = scratch / "original"
        original.mkdir()
        scope = _book(original)
        copies = {}
        for name in ("python", "rust", "direct", "other"):
            shutil.copytree(original, scratch / name)
            copies[name] = scratch / name
        runtimes = {}
        for name in ("python", "rust"):
            runtime = scratch / f"rt-{name}"
            runtime.mkdir(mode=0o700)
            runtimes[name] = runtime
        yield copies, runtimes, scope


class Recorder:
    """Normalizes replies: each generated value becomes a stable name."""

    def __init__(self, root: Path) -> None:
        self.root = str(root.resolve())
        self.names: dict[str, str] = {}

    def name(self, kind: str, value: str) -> str:
        key = f"{kind}:{value}"
        if key not in self.names:
            self.names[key] = f"<{kind}-{sum(k.startswith(kind + ':') for k in self.names)}>"
        return self.names[key]

    def text(self, value: str) -> str:
        value = value.replace(self.root, "<root>")
        value = re.sub(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ", "<time>", value)
        return value

    def normalize(self, value, key: str | None = None):
        if key in {"build", "broker_build", "this_build"} and isinstance(value, dict):
            return "<build>"
        if isinstance(value, dict):
            return {k: self.normalize(v, k) for k, v in sorted(value.items())}
        if isinstance(value, list):
            return [self.normalize(item) for item in value]
        if key == "remaining_seconds" and isinstance(value, int):
            return "<remaining>"
        if key in {"stdout", "stderr"} and isinstance(value, str):
            return "<output>" if value else ""
        if isinstance(value, str):
            if key == "generation":
                return self.name("generation", value)
            if key == "lease_id":
                return self.name("lease", value)
            if key in {"project", "lease_project", "command_project"}:
                return self.name("project", value)
            if key == "operation_id":
                return self.name("operation", value)
            return self.text(value)
        return value


def _scenario(broker: Broker, root: Path, other: Path, scope: tuple[str, str]):
    """The actions both brokers answer; returns normalized steps and the
    brokered child outputs by command."""

    recorder = Recorder(root)
    steps: list[tuple[str, object]] = []
    outputs: dict[str, str] = {}
    workspace, profile = scope
    data_root = str(root)

    def record(label: str, action) -> object:
        try:
            result = action()
        except AppError as exc:
            result = {
                "error": {
                    "code": exc.code,
                    "message": str(exc),
                    "hint": exc.hint,
                    "details": exc.details,
                    "retryable": exc.retryable,
                }
            }
        steps.append((label, recorder.normalize(result)))
        return result

    def run(label: str, argv: list[str], *, admin: bytes | None = None, secrets=None):
        prepared = PreparedArguments(argv, dict(secrets or {}))
        accepted = record(
            f"{label}: accepted",
            lambda: BrokerClient().submit(
                data_root,
                prepared,
                admin_authentication=bytearray(admin) if admin is not None else None,
            ),
        )
        if "operation_id" in accepted:
            completed = record(f"{label}: completed", lambda: BrokerClient().wait(accepted["operation_id"]))
            outputs[label] = completed.get("stdout", "")

    def raw(label: str, request: dict) -> None:
        with connect(timeout=2.0, io_timeout=5.0) as channel:
            channel.send_json(request)
            steps.append((label, recorder.normalize(channel.receive_json())))

    with broker.active():
        client = BrokerClient()
        record("ping", client.ping)
        record("status", lambda: client.status(None))
        record("status locked", lambda: client.status(data_root))
        record(
            "unlock wrong passphrase",
            lambda: client.unlock(
                data_root,
                bytearray(b"wrong passphrase"),
                duration_seconds=None,
                capability="operator",
                authentication_method="password",
            ),
        )
        record(
            "unlock",
            lambda: client.unlock(
                data_root,
                bytearray(PASSPHRASE.encode()),
                duration_seconds=3600,
                capability="operator",
                authentication_method="password",
            ),
        )
        record("status unlocked", lambda: client.status(data_root))
        record("status all", lambda: client.status(None))
        run("status", ["--data-root", data_root, "--machine", "status"])
        run(
            "profiles get",
            ["--machine", "profiles", "get", "--workspace", workspace, "--profile", profile],
        )
        run("unscoped", ["--machine", "transactions", "list"])
        run("other project", ["--data-root", str(other), "--machine", "status"])
        run("invalid", ["--definitely-invalid"])
        run("not brokerable", ["--machine", "operator", "status"])
        run(
            "capability",
            ["--machine", "profiles", "set", "--workspace", workspace, "--profile", profile, "--fiat-currency", "USD"],
        )
        run(
            "journals",
            ["--machine", "journals", "process", "--workspace", workspace, "--profile", profile],
        )
        run("admin wrong", ["--machine", "secrets", "verify"], admin=b"wrong passphrase")
        run("admin", ["--machine", "secrets", "verify"], admin=PASSPHRASE.encode())
        run("unclassified", ["--machine", "commands", "describe", "secrets"])
        record("unknown operation", lambda: client._simple_request("operation_status", operation_id=f"{client.ping()['generation']}.x"))
        record("old generation", lambda: client._simple_request("operation_status", operation_id="old.x"))
        record("cancel unknown", lambda: client.cancel("old.x"))
        raw("version mismatch", {"version": PROTOCOL_VERSION - 1, "action": "ping"})
        raw("unknown action", {"version": PROTOCOL_VERSION, "action": "nope"})
        raw("lock condition", {"version": PROTOCOL_VERSION, "action": "lock", "data_root": data_root, "restore_manual": "yes"})
        raw("missing data root", {"version": PROTOCOL_VERSION, "action": "lock"})
        raw(
            "relative directory",
            {
                "version": PROTOCOL_VERSION,
                "action": "submit",
                "data_root": data_root,
                "operation_id": "x",
                "argv": ["status"],
                "working_directory": "relative",
            },
        )
        record("lock kept", lambda: client.lock(data_root, expected_lease_id="nope", restore_manual=True))
        record("lock", lambda: client.lock(data_root, restore_manual=True))
        record("status relocked", lambda: client.status(data_root))
        run("after lock", ["--data-root", data_root, "--machine", "status"])
        record(
            "set mode wrong",
            lambda: client.set_mode(data_root, "brokered", bytearray(b"wrong passphrase")),
        )
        record("set mode", lambda: client.set_mode(data_root, "brokered", bytearray(PASSPHRASE.encode())))
        record("set mode manual", lambda: client.set_mode(data_root, "manual", bytearray(PASSPHRASE.encode())))
    return steps, outputs, recorder


def _direct(root: Path, argv: list[str]) -> str:
    completed = subprocess.run(
        [str(RUST_BIN), "--db-passphrase-fd", "0", *argv],
        input=PASSPHRASE.encode(),
        capture_output=True,
        timeout=120,
        env={k: v for k, v in os.environ.items() if not k.startswith("KASSIBER_OPERATOR_")},
    )
    return completed.stdout.decode()


def test_rust_broker_matches_the_python_broker(books):
    copies, runtimes, scope = books
    python = Broker([sys.executable, str(FIXTURE)], runtimes["python"], {})
    try:
        python_steps, python_outputs, python_names = _scenario(python, copies["python"], copies["other"], scope)
    finally:
        python.close()
    rust = Broker(
        [str(RUST_BIN), "--operator-broker-server"],
        runtimes["rust"],
        {"KASSIBER_RUST_TEST_OPERATOR_SESSION": "fixture"},
    )
    try:
        rust_steps, rust_outputs, rust_names = _scenario(rust, copies["rust"], copies["other"], scope)
    finally:
        rust.close()

    assert [label for label, _ in rust_steps] == [label for label, _ in python_steps]
    codes = {
        label: step["error"]["code"]
        for label, step in python_steps
        if isinstance(step, dict) and isinstance(step.get("error"), dict)
    }
    states = {label: step.get("state") for label, step in python_steps if isinstance(step, dict)}
    # The scenario reaches each refusal and terminal state it means to.
    assert codes["unlock wrong passphrase"] == "unlock_failed"
    assert codes["unscoped: accepted"] == "operator_scope_required"
    assert codes["other project: accepted"] == "operator_project_mismatch"
    assert codes["invalid: accepted"] == "operator_invalid_command"
    assert codes["not brokerable: accepted"] == "operator_command_not_brokerable"
    assert codes["capability: accepted"] == "operator_capability_denied"
    assert codes["admin wrong: accepted"] == "unlock_failed"
    assert codes["after lock: accepted"] == "interaction_required"
    assert codes["set mode wrong"] == "unlock_failed"
    assert states["status: completed"] == states["admin: completed"] == "completed"
    for (label, expected), (_, actual) in zip(python_steps, rust_steps):
        assert actual == expected, label

    # The children: Rust brokered envelopes equal direct Rust runs on a
    # third copy, and the Python CLI's for the leaves both serve.
    workspace, profile = scope
    direct_root = str(copies["direct"])
    direct = {
        "status": _direct(copies["direct"], ["--data-root", direct_root, "--machine", "status"]),
        "profiles get": _direct(
            copies["direct"],
            ["--data-root", direct_root, "--machine", "profiles", "get", "--workspace", workspace, "--profile", profile],
        ),
    }
    direct_names = Recorder(copies["direct"])
    for command, output in direct.items():
        assert json.loads(output)["kind"] in {"status", "profiles.get"}
        assert rust_names.text(rust_outputs[command]) == direct_names.text(output), command
        assert python_names.text(python_outputs[command]) == rust_names.text(rust_outputs[command]), command
