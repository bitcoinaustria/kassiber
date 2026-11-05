"""R6-1 parity: the Rust operator protocol, endpoint and client against Python.

- The checked-in corpus (``crates/operator/tests/data/protocol_corpus.json``)
  is regenerated from ``kassiber/operator/protocol.py`` and ``client.py`` and
  must not drift; ``crates/operator/tests/corpus.rs`` replays it in Rust.
- Python's ``BrokerClient.ping``/``status`` talk to a Rust listener, and the
  Rust client talks to the Python broker from ``tests/operator_server_fixture.py``.
  Neither needs a same-build proof, so mixed builds can meet here.
- The mutation catalog stays applicable.

Every endpoint lives in a disposable 0700 runtime directory selected through
the gated test override; no test touches the account home's endpoint. The
Rust examples are test-harness builds and are built offline.
"""

from __future__ import annotations

import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from unittest import mock

import pytest

from kassiber.operator.client import BrokerClient
from kassiber.operator.protocol import BrokerChannel, TEST_RUNTIME_OVERRIDE_ENV

ROOT = Path(__file__).resolve().parents[1]
GENERATOR = ROOT / "crates" / "operator" / "scripts" / "generate_protocol_corpus.py"
CORPUS = ROOT / "crates" / "operator" / "tests" / "data" / "protocol_corpus.json"
MUTATIONS = ROOT / "crates" / "operator" / "scripts" / "mutations.json"
CHECKER = ROOT / "crates" / "operator" / "scripts" / "mutation_check.py"
FIXTURE = ROOT / "tests" / "operator_server_fixture.py"


def _load_generator():
    spec = importlib.util.spec_from_file_location("operator_protocol_corpus", GENERATOR)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def test_protocol_corpus_matches_the_python_reference():
    generator = _load_generator()
    assert CORPUS.read_text(encoding="utf-8") == generator.render()
    corpus = json.loads(CORPUS.read_text(encoding="utf-8"))
    assert len(corpus["client"]) >= 28
    assert len(corpus["decode_errors"]) >= 10


def test_mutation_catalog_stays_applicable():
    entries = json.loads(MUTATIONS.read_text(encoding="utf-8"))
    result = subprocess.run(
        [sys.executable, str(CHECKER), "--check"],
        check=True,
        capture_output=True,
        text=True,
        timeout=60,
    )
    assert result.stdout == f"{len(entries)} entries apply\n"
    sources = "".join(
        path.read_text(encoding="utf-8")
        # R6-3's CLI and agent-access recipes name tests in cli, setup and daemon.
        for crate in ("operator", "ipc", "ownership", "cli", "setup", "daemon")
        for path in (ROOT / "crates" / crate).rglob("*.rs")
    )
    for entry in entries:
        for name in entry["expect_failing"]:
            assert f"fn {name}(" in sources, name


@pytest.fixture(scope="module")
def examples():
    cargo = ["mise", "exec", "rust@1.99.1", "--", "cargo"] if shutil.which("mise") else ["cargo"]
    result = subprocess.run(
        [*cargo, "build", "--locked", "--offline", "-p", "kassiber-operator", "--features", "test-harness", "--examples"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        timeout=600,
        env={**os.environ, "MISE_OFFLINE": "1"},
    )
    assert result.returncode == 0, result.stderr
    metadata = subprocess.run(
        [*cargo, "metadata", "--locked", "--offline", "--no-deps", "--format-version", "1"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    directory = Path(json.loads(metadata.stdout)["target_directory"]) / "debug" / "examples"
    suffix = ".exe" if os.name == "nt" else ""
    return {name: directory / f"{name}{suffix}" for name in ("operator_test_listener", "operator_test_client")}


@pytest.fixture
def runtime():
    # AF_UNIX paths stop at 104/108 bytes; keep the directory short.
    with tempfile.TemporaryDirectory(prefix="kb-op-") as directory:
        os.chmod(directory, 0o700)
        environment = {
            "KASSIBER_OPERATOR_RUNTIME_DIR": directory,
            TEST_RUNTIME_OVERRIDE_ENV: "1",
            "XDG_RUNTIME_DIR": directory,
            "KASSIBER_TEST_NO_EGRESS": "strict",
        }
        with mock.patch.dict(os.environ, environment):
            yield Path(directory)


def _python_frame(payload: dict) -> bytes:
    class Capture:
        sent = bytearray()

        def send_all(self, data: bytes) -> None:
            self.sent.extend(data)

    capture = Capture()
    BrokerChannel(capture).send_json(payload)
    return bytes(capture.sent)


@pytest.mark.skipif(os.name == "nt", reason="POSIX socket harness; the Windows pipe path is in crates/operator/tests/windows_pipe.rs")
def test_python_client_reaches_a_rust_listener(examples, runtime):
    listener = subprocess.Popen(
        [str(examples["operator_test_listener"]), "3"],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=os.environ.copy(),
    )
    try:
        assert listener.stdout.readline() == "ready\n", listener.stderr.read()
        assert (runtime / "operator-v1.sock").exists()
        client = BrokerClient()
        ping = client.ping()
        assert ping["broker"] == "running"
        assert ping["caller_context"] is True
        build = ping["build"]
        assert list(build) == sorted(["version", "channel", "commit", "frozen", "origin"])
        assert build["frozen"] is False
        assert len(build["origin"]) == 12
        assert client.status(None) == {
            "broker": "running",
            "generation": "rust-test-listener",
            "leases": [],
            "broker_build": build,
        }
        # A request in another protocol version gets the version refusal.
        from kassiber.operator.protocol import connect

        with connect() as channel:
            channel.send_json({"version": 2, "action": "ping"})
            reply = channel.receive_json()
        assert reply["ok"] is False and reply["error"]["code"] == "operator_protocol_version_mismatch"
        received = [listener.stdout.readline().strip() for _ in range(3)]
        assert received == [
            _python_frame({"version": 3, "action": "ping"}).hex(),
            _python_frame({"version": 3, "action": "status", "data_root": None}).hex(),
            _python_frame({"version": 2, "action": "ping"}).hex(),
        ]
        assert listener.wait(timeout=10) == 0
        # The listener removed only its own endpoint and left the election.
        assert not (runtime / "operator-v1.sock").exists()
        assert (runtime / "operator-v1.start.lock").exists()
    finally:
        if listener.poll() is None:
            listener.kill()
        listener.communicate(timeout=10)


@pytest.mark.skipif(os.name == "nt", reason="POSIX broker fixture")
def test_rust_client_reaches_the_python_broker(examples, runtime, tmp_path):
    environment = os.environ.copy()
    environment["PYTHONPATH"] = os.pathsep.join(
        part for part in (str(ROOT), environment.get("PYTHONPATH")) if part
    )
    broker = subprocess.Popen(
        [sys.executable, str(FIXTURE)],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        env=environment,
    )
    data_root = tmp_path / "book"
    data_root.mkdir()
    try:
        client = BrokerClient()
        deadline = time.monotonic() + 20
        while True:
            try:
                expected_ping = client.ping()
                break
            except (OSError, EOFError):
                assert broker.poll() is None, broker.stderr.read()
                assert time.monotonic() < deadline
                time.sleep(0.05)
        expected = [
            {"value": expected_ping},
            {"value": client.status(None)},
            {"value": client.status(str(data_root))},
        ]
        result = subprocess.run(
            [str(examples["operator_test_client"]), str(data_root)],
            capture_output=True,
            text=True,
            timeout=30,
            env=environment,
        )
        assert result.returncode == 0, result.stderr
        assert result.stderr == ""
        lines = result.stdout.splitlines()
        assert [json.loads(line) for line in lines] == expected
        # The lines are the Rust encoder's compact sorted bytes of the same data.
        assert lines == [json.dumps(item, separators=(",", ":"), sort_keys=True) for item in expected]
        # Same endpoint, another build: the Rust client never claims Python's.
        assert expected_ping["build"]["frozen"] is False
    finally:
        broker.terminate()
        broker.wait(timeout=10)
        broker.stderr.close()


@pytest.mark.skipif(os.name == "nt", reason="POSIX socket harness")
def test_rust_client_without_a_broker_reports_it_stopped(examples, runtime):
    result = subprocess.run(
        [str(examples["operator_test_client"])],
        capture_output=True,
        text=True,
        timeout=30,
        env=os.environ.copy(),
    )
    assert result.returncode == 0, result.stderr
    assert [json.loads(line) for line in result.stdout.splitlines()] == [
        {"error": {"class": "FileNotFoundError"}},
        {"value": {"broker": "stopped", "lease": "locked"}},
    ]
    # Nothing was started and no endpoint was created.
    assert sorted(path.name for path in runtime.iterdir()) == []
