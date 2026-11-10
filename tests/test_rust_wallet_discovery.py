"""Compare only the compatibility discovery helper, never claim product sync.

Both implementations query the same owned loopback fixture without patching
Python service code. Ordinary Electrum remains the separate BDK observer route.
This deliberately compares no book persistence or transaction/reorg behavior.
"""
from collections import Counter
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import socketserver
import subprocess
import threading

import pytest

from kassiber import __version__
from kassiber.core.sync import sync_progress_emitter
from kassiber.core.sync_backends import (
    discover_compatibility_descriptor_targets,
    scriptpubkey_scripthash,
)
from kassiber.wallet_descriptors import derive_descriptor_targets, load_descriptor_plan
from tests.rust_sync_env import child_env as _env
from tests.rust_fixture_binary import fixture_binary

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires Rust discovery fixture build")
VECTORS = Path(__file__).resolve().parents[1] / "crates/wallets/tests/vectors/python_descriptors.json"


def _config(name, gap=3):
    config = next(row["config"] for row in json.loads(VECTORS.read_text())["plans"]
                  if row["name"] == name).copy()
    config["gap_limit"] = gap
    return config


@contextmanager
def _server(protocol, used):
    hits = []

    class Http(BaseHTTPRequestHandler):
        def do_GET(self):
            hits.append(self.path)
            script = self.path.removeprefix("/scripthash/")
            payload = {"chain_stats": {"tx_count": int(script in used)},
                       "mempool_stats": {"tx_count": 0}}
            body = json.dumps(payload).encode()
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_):
            pass

    class Electrum(socketserver.StreamRequestHandler):
        def handle(self):
            self.request.settimeout(5)
            while line := self.rfile.readline():
                request = json.loads(line)
                hits.append(request)
                if request["method"] == "server.version":
                    result = ["fixture", "1.6"]
                else:
                    assert request["method"] == "blockchain.scripthash.subscribe"
                    result = "nonempty-status" if request["params"][0] in used else None
                self.wfile.write(json.dumps({"id": request["id"], "result": result}).encode() + b"\n")

    server = (ThreadingHTTPServer if protocol == "esplora" else socketserver.ThreadingTCPServer)(
        ("127.0.0.1", 0), Http if protocol == "esplora" else Electrum)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    scheme = "http" if protocol == "esplora" else "tcp"
    try:
        yield f"{scheme}://127.0.0.1:{server.server_address[1]}", hits
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


def _rust(tmp_path, config, backend, checkpoint, **options):
    binary = fixture_binary("discovery_fixture")
    result = subprocess.run([str(binary)], input=json.dumps({
        "wallet": config, "backend": backend, "checkpoint": checkpoint,
        "version": __version__, "consent": True, **options,
    }).encode(), capture_output=True, timeout=15, check=True, env=_env(tmp_path))
    assert result.stderr == b""
    return json.loads(result.stdout)


@pytest.mark.parametrize("protocol", ["esplora", "electrum"])
@pytest.mark.parametrize("fixture", ["empty", "receive-change", "checkpoint", "multi-script", "fixed", "gap-one"])
def test_compatibility_discovery_matches_python_targets_progress_and_queries(tmp_path, protocol, fixture):
    config = _config("multi-script-p2tr+p2pkh+p2wpkh+p2sh-p2wpkh" if fixture == "multi-script"
                     else "wpkh-fixed" if fixture == "fixed" else "wpkh", 1 if fixture == "gap-one" else 3)
    plan = load_descriptor_plan(config)
    all_targets = list(derive_descriptor_targets(plan, start=0, end=12))
    used = {scriptpubkey_scripthash(target.script_pubkey) for target in all_targets
            if fixture in {"receive-change", "checkpoint", "multi-script"}
            and target.address_index in {1, 4}}
    checkpoint = {}
    if fixture == "checkpoint":
        checkpoint = {"highest_used": {"0": "٤", "1": 4.9},
                      "electrum_scripthash_statuses": {}, "esplora_scripthashes": {}}
        for target in all_targets:
            if target.address_index in {1, 4}:
                script = scriptpubkey_scripthash(target.script_pubkey)
                checkpoint["esplora_scripthashes"][script] = {"tx_count": 1}
                checkpoint["electrum_scripthash_statuses"][script] = "previous-status"
    with _server(protocol, used) as (url, hits):
        backend = {"name": "fixture", "kind": protocol, "url": url, "batch_size": 2}
        events = []
        token = sync_progress_emitter.set(lambda payload: events.append(dict(payload)))
        try:
            expected = discover_compatibility_descriptor_targets(backend, plan, protocol, checkpoint)
        finally:
            sync_progress_emitter.reset(token)
        reference_queries = hits.copy()
        hits.clear()
        actual = _rust(tmp_path, config, backend, checkpoint)
        # Canonicalized preparation dump, not a normalized book dump.
        assert json.dumps(actual, sort_keys=True) == json.dumps({"result": expected, "progress": events}, sort_keys=True)
        assert Counter(json.dumps(row, sort_keys=True) for row in hits) == Counter(
            json.dumps(row, sort_keys=True) for row in reference_queries)


@pytest.mark.parametrize("protocol", ["esplora", "electrum"])
def test_discovery_driver_refuses_without_consent_offline_and_after_cancellation(tmp_path, protocol):
    with _server(protocol, set()) as (url, hits):
        backend = {"name": "fixture", "kind": protocol, "url": url, "batch_size": 2}
        for options, code in [({"consent": False}, "interaction_required"),
                              ({"offline": True}, "network_egress_disabled")]:
            result = _rust(tmp_path, _config("wpkh"), backend, {}, **options)
            assert result["error"]["code"] == code
            assert hits == []
        result = _rust(tmp_path, _config("wpkh"), backend, {}, cancel_after_progress=True)
        assert result["error"]["code"] == "cancelled"
        assert len(result["progress"]) == 1
        assert len(hits) == (2 if protocol == "esplora" else 3)
