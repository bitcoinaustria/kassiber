"""Chain form probes compared on the same owned loopback protocol fixtures.

No Python service is patched. The hybrid log proves the whole daemon session
was Rust; each requested kind must be advertised and return a native result.
"""
from __future__ import annotations

from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import socketserver
import subprocess
import sys
import threading

import pytest

from tests import cli_runner
from tests.test_rust_shell_parity import _environment
from tests.rust_fixture_binary import fixture_binary

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires Rust test-harness binary")


def _env(tmp_path):
    # Child failure reports must never contain unrelated parent credentials.
    allowed = {"PATH", "LANG", "LC_ALL", "TZ", "HOME", "USERPROFILE", "LOCALAPPDATA",
               "XDG_DATA_HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "COLUMNS", "LINES",
               "PYTHONIOENCODING", "PYTHONPATH", "KASSIBER_TEST_NO_EGRESS",
               "KASSIBER_DISABLE_UPDATE_CHECK", "KASSIBER_OFFLINE_PREFERENCE_FILE",
               "KASSIBER_AGENT_ACCESS_FILE", "KASSIBER_UPDATE_PREFERENCE_FILE",
               "KASSIBER_UPDATE_CACHE_FILE"}
    return {key: value for key, value in _environment(tmp_path).items() if key in allowed}


@contextmanager
def _http(*, status=200):
    hits = []

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            hits.append((self.path, dict(self.headers)))
            body = b"[]"
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_POST(self):
            request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            hits.append((self.path, request))
            result = {
                "getblockchaininfo": {"chain": "regtest", "blocks": 42, "headers": 42,
                                       "pruned": False, "initialblockdownload": False},
                "getnetworkinfo": {"version": 270000, "connections": 8},
                "listwallets": [], "getbestblockhash": "besthash",
                "getblockfilter": {"filter": "00", "header": "11"},
            }[request["method"]]
            body = json.dumps({"id": request["id"],
                               "result": result if status == 200 else None,
                               "error": None if status == 200 else {
                                   "code": -8, "message": "rescan failed",
                               }}).encode()
            self.send_response(status)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}", hits
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


@contextmanager
def _electrum(*, error=False):
    hits = []

    class Handler(socketserver.StreamRequestHandler):
        def handle(self):
            self.request.settimeout(5)
            while line := self.rfile.readline():
                request = json.loads(line)
                hits.append(request)
                result = ["fixture", "1.6"] if request["method"] == "server.version" else "fixture banner"
                response = {"id": request["id"], "result": result}
                if error and request["method"] != "server.version":
                    response = {"id": request["id"], "error": {
                        "code": -1, "message": "fixture failure",
                    }}
                self.wfile.write(json.dumps(response).encode() + b"\n")

    server = socketserver.ThreadingTCPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"tcp://127.0.0.1:{server.server_address[1]}", hits
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


def _setup(tmp_path, monkeypatch):
    env = _env(tmp_path)
    env["KASSIBER_HYBRID_RUST_BIN"] = str(Path(RUST_BIN).resolve())
    env["KASSIBER_HYBRID_RUST_DAEMON"] = "1"
    env["KASSIBER_HYBRID_LOG"] = str(tmp_path / "served.jsonl")
    root = tmp_path / "data"
    result = cli_runner.run(["--data-root", root, "--machine", "init"],
                            reference=True, env=env, capture_output=True)
    assert result.returncode == 0, result.stdout
    monkeypatch.setenv("KASSIBER_TEST_CLI", json.dumps([
        sys.executable, str(cli_runner.ROOT / "tests/rust_port/hybrid_cli.py")]))
    return root, env


def _both(root, env, requests):
    wire = "".join(json.dumps({"request_id": i, "kind": kind, "args": args}) + "\n"
                   for i, (kind, args) in enumerate([*requests, ("daemon.shutdown", {})]))
    options = dict(env=env, input=wire.encode(), capture_output=True, timeout=120)
    python = cli_runner.run(["--data-root", root, "daemon"], reference=True, **options)
    rust = cli_runner.run(["--data-root", root, "daemon"], **options)
    assert python.returncode == rust.returncode == 0, (python.stderr, rust.stderr, rust.stdout)
    assert python.stderr == rust.stderr == b""
    expected, actual = python.stdout.splitlines(), rust.stdout.splitlines()
    assert len(actual) == len(expected) == len(requests) + 2
    ready = json.loads(actual[0])["data"]["supported_kinds"]
    for kind, _ in requests:
        assert kind in ready
    assert actual[1:] == expected[1:]
    calls = [json.loads(line) for line in Path(env["KASSIBER_HYBRID_LOG"]).read_text().splitlines()]
    assert calls and all(row == {"command": "daemon", "served_by": "rust"} for row in calls)


def test_five_probe_kinds_match_python_on_loopback_and_have_no_fallback(tmp_path, monkeypatch):
    root, env = _setup(tmp_path, monkeypatch)
    with _http() as (http, http_hits), _electrum() as (electrum, electrum_hits):
        _both(root, env, [
            ("ui.backends.http.test", {"url": http}),
            ("ui.backends.electrum.test", {"url": electrum}),
            ("ui.backends.bitcoinrpc.test", {"url": http, "username": "user", "password": "secret"}),
            ("ui.backends.btcpay.test", {"server_url": http, "api_key": "secret"}),
            ("ui.backends.detect_core", {}),
        ])
        assert len(http_hits) == 14  # GET, five Core RPCs, store catalog, twice.
        assert len(electrum_hits) == 4  # One handshake and banner per session.
        assert [r["method"] for r in electrum_hits] == ["server.version", "server.banner"] * 2
        for _, request in (row for row in http_hits if isinstance(row[1], dict) and "method" in row[1]):
            assert request["id"] == f"kassiber-{request['method']}"
            assert request["jsonrpc"] == "1.0"


def test_detect_core_uses_one_explicit_local_candidate_without_leaking_cookie(tmp_path, monkeypatch):
    root, env = _setup(tmp_path, monkeypatch)
    bitcoin = tmp_path / ".bitcoin"
    bitcoin.mkdir()
    (bitcoin / ".cookie").write_text("user:fixture-secret")
    with _http() as (http, hits):
        (bitcoin / "bitcoin.conf").write_text(f"[regtest]\nrpcurl={http}\nrpcuser=user\nrpcpassword=fixture-secret\n")
        # Default main cookie would probe a closed real port; keep candidates to
        # the one synthetic regtest basic-auth node on the ephemeral fixture.
        (bitcoin / ".cookie").unlink()
        _both(root, env, [("ui.backends.detect_core", {})])
        assert len(hits) == 10


def test_offline_probes_cannot_touch_loopback(tmp_path, monkeypatch):
    root, env = _setup(tmp_path, monkeypatch)
    (tmp_path / "offline.json").write_text('{"schema_version":1,"enabled":true}')
    with _http() as (http, hits), _electrum() as (electrum, electrum_hits):
        bitcoin = tmp_path / ".bitcoin"
        bitcoin.mkdir()
        (bitcoin / "bitcoin.conf").write_text(
            f"[regtest]\nrpcurl={http}\nrpcuser=user\nrpcpassword=fixture-secret\n")
        # Error text from Python/OpenSSL/socket libraries is not stable across
        # implementations; assert the product refusal and independent no-hit.
        requests = [("ui.backends.http.test", {"url": http}),
                    ("ui.backends.bitcoinrpc.test", {"url": http, "username": "u", "password": "p"}),
                    ("ui.backends.btcpay.test", {"url": http, "api_key": "secret"}),
                    ("ui.backends.electrum.test", {"url": electrum}),
                    ("ui.backends.detect_core", {})]
        wire = "".join(json.dumps({"request_id": i, "kind": kind, "args": args}) + "\n"
                       for i, (kind, args) in enumerate(requests))
        result = cli_runner.run(["--data-root", root, "daemon"], env=env,
                                input=wire.encode(), capture_output=True, timeout=120)
        rows = [json.loads(line) for line in result.stdout.splitlines()][1:]
        assert result.returncode == 0
        assert len(rows) == 5
        assert rows[0]["data"]["ok"] is False
        assert rows[1]["data"]["error"]["code"] == "network_egress_disabled"
        assert rows[2]["error"]["code"] == "network_egress_disabled"
        assert rows[3]["data"]["ok"] is False
        assert rows[4]["data"] == {"candidates": []}
        assert hits == []
        assert electrum_hits == []
        calls = [json.loads(line) for line in Path(env["KASSIBER_HYBRID_LOG"]).read_text().splitlines()]
        assert calls == [{"command": "daemon", "served_by": "rust"}]


def test_probe_validation_matches_python_before_any_loopback_hit(tmp_path, monkeypatch):
    root, env = _setup(tmp_path, monkeypatch)
    with _http() as (http, hits):
        _both(root, env, [
            ("ui.backends.http.test", {}),
            ("ui.backends.http.test", {"url": http, "proxy": False}),
            ("ui.backends.electrum.test", {"url": 3}),
            ("ui.backends.bitcoinrpc.test", {"url": http, "config": []}),
            ("ui.backends.bitcoinrpc.test", {"url": http, "birthday": "bad"}),
            ("ui.backends.btcpay.test", {"url": http, "timeout": False}),
        ])
        assert hits == []


def test_direct_clients_match_python_on_identical_loopback_fixtures(tmp_path):
    from kassiber.core.sync_backends import ElectrumClient, bitcoinrpc_call, http_get_json

    binary = fixture_binary("client_fixture", crate="kassiber-chain")
    env = _env(tmp_path)

    def rust(protocol, backend, *, expect_error=False, **args):
        result = subprocess.run([str(binary)], env=env, input=json.dumps({
            "protocol": protocol, "backend": backend, **args,
        }).encode(), capture_output=True, check=True, timeout=10)
        assert result.stderr == b""
        return json.loads(result.stdout)["error" if expect_error else "result"]

    with _http() as (http, http_hits), _electrum() as (electrum, electrum_hits):
        backend = {"name": "fixture", "url": http, "username": "user", "password": "secret"}
        assert rust("esplora", backend, path="blocks") == http_get_json(f"{http}/blocks")
        assert rust("core", backend, method="getblockchaininfo") == bitcoinrpc_call(backend, "getblockchaininfo")
        backend = {"name": "fixture", "url": electrum, "batch_size": 2}
        requests = [["server.banner", []], ["server.banner", []]]
        with ElectrumClient(backend) as client:
            expected = client.batch_call(requests)
        assert rust("electrum", backend, requests=requests) == expected
        assert len(http_hits) == 4
        # Compare the actual wire requests, including Core method/id/params.
        assert http_hits[2] == http_hits[3]
        assert electrum_hits[:3] == electrum_hits[3:]

    from kassiber.errors import AppError

    with _http(status=401) as (http, _):
        with pytest.raises(AppError) as error:
            http_get_json(f"{http}/blocks")
        assert rust("esplora", {"url": http}, path="blocks", expect_error=True) == {
            "code": error.value.code, "message": str(error.value),
        }
    with _http(status=500) as (http, _):
        backend = {"name": "fixture", "url": http, "username": "u", "password": "p"}
        with pytest.raises(AppError) as error:
            bitcoinrpc_call(backend, "getblockchaininfo")
        assert rust("core", backend, method="getblockchaininfo", expect_error=True) == {
            "code": error.value.code, "message": str(error.value),
        }
    with _electrum(error=True) as (electrum, _):
        backend = {"name": "fixture", "url": electrum}
        with ElectrumClient(backend) as client, pytest.raises(AppError) as error:
            client.call("server.banner")
        assert rust("electrum", backend, requests=[["server.banner", []]], expect_error=True) == {
            "code": error.value.code, "message": str(error.value),
        }
