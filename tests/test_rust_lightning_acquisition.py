from pathlib import Path
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
import subprocess
import sys
import threading

import pytest

from kassiber import __version__

ROOT = Path(__file__).resolve().parents[1]
BIN = os.environ.get("KASSIBER_LIGHTNING_FIXTURE_BIN")


@contextmanager
def node(calls):
    requests = []

    class Handler(BaseHTTPRequestHandler):
        def respond(self):
            body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
            index = len(requests)
            requests.append({"method": self.command, "path": self.path,
                             "body": json.loads(body) if body else None,
                             "macaroon": self.headers.get("Grpc-Metadata-macaroon"),
                             "agent": self.headers.get("User-Agent")})
            response = json.dumps(calls[min(index, len(calls) - 1)]["response"]).encode()
            self.send_response(200)
            self.send_header("Content-Length", str(len(response)))
            self.end_headers()
            self.wfile.write(response)

        do_GET = respond
        do_POST = respond

        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=lambda: server.serve_forever(poll_interval=0.01))
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}", requests
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


def environment(root):
    env = {k: v for k, v in os.environ.items() if not k.startswith("KASSIBER_")}
    env.update(
        KASSIBER_OFFLINE_PREFERENCE_FILE=str(root / "offline.json"),
        KASSIBER_TEST_NO_EGRESS="1",
        XDG_DATA_HOME=str(root / "data"), XDG_CONFIG_HOME=str(root / "config"),
        XDG_STATE_HOME=str(root / "state"),
    )
    return env


PYTHON_READ = """
from dataclasses import asdict
import json, sys
from kassiber.core.lightning.lnd import LndAdapter
data = json.load(sys.stdin)
snapshot = LndAdapter().fetch_node_snapshot({}, data['backend'], window_days=data['window'])
print(json.dumps({'channels': [{'id': c.id, 'peer_alias': c.peer_alias,
    'capacity_sat': c.capacity_sat, 'earned_routing_sat': c.earned_routing_sat}
    for c in snapshot.channels], 'routing': asdict(snapshot.routing) if snapshot.routing else None}))
"""


@pytest.mark.skipif(not BIN, reason="requires native Lightning acquisition fixture")
def test_lnd_report_real_transport_processes_match_python_without_fallback(tmp_path):
    cases = json.loads((ROOT / "crates/chain/fixtures/python_lightning.json").read_text())
    selected = [c for c in cases if "error" not in c][::7]
    assert len(selected) == 12
    for case in selected:
        outputs, requests = [], []
        for native in [False, True]:
            with node(case["calls"]) as (url, hits):
                data = {"backend": {"kind": "lnd", "name": "fake", "url": url,
                                    "token": "ABCD", "timeout": 2},
                        "window": case["window"], "version": __version__}
                result = subprocess.run(
                    [BIN] if native else [sys.executable, "-c", PYTHON_READ],
                    cwd=ROOT, env=environment(tmp_path), input=json.dumps(data),
                    text=True, capture_output=True, check=True, timeout=15,
                )
                output = json.loads(result.stdout)
                if native:
                    assert output["attempts"] > 0
                    output = output["response"]["result"]
                outputs.append(output)
                requests.append(hits.copy())
        assert outputs == [case["expected"], case["expected"]], case["name"]
        assert requests[0] == requests[1], case["name"]


@pytest.mark.skipif(not BIN, reason="requires native Lightning acquisition fixture")
def test_lnd_report_process_product_no_egress_offline_and_missing_consent_have_zero_hits(tmp_path):
    with node([{"response": {}}]) as (url, hits):
        for fields, extra, expected in [
            ({"consent": False}, {}, "interaction_required"),
            ({"offline": True}, {}, "network_egress_disabled"),
            ({}, {"KASSIBER_NO_EGRESS": "1"}, "network_egress_disabled"),
        ]:
            data = {"backend": {"kind": "lnd", "name": "fake", "url": url,
                                "token": "abcd"}, **fields}
            result = subprocess.run([BIN], cwd=ROOT, env={**environment(tmp_path), **extra},
                                    input=json.dumps(data), text=True, capture_output=True,
                                    check=True, timeout=15)
            output = json.loads(result.stdout)
            assert output["response"]["error"]["code"] == expected
            assert output["attempts"] == output["records"] == 0
        assert hits == []


def test_lnd_report_acquisition_vectors_match_unchanged_python():
    subprocess.run(
        [sys.executable, str(ROOT / "crates/chain/scripts/gen_lightning_vectors.py"), "--check"],
        cwd=ROOT, check=True, capture_output=True, text=True, timeout=180,
    )


def test_lnd_reference_error_disclosure_remains_reproducible():
    result = subprocess.run(
        [sys.executable, str(ROOT / "crates/chain/scripts/repro_python_lnd_errors.py")],
        cwd=ROOT, check=True, capture_output=True, text=True, timeout=30,
    )
    assert json.loads(result.stdout) == {
        "http_error_discloses_response": True,
        "json_error_discloses_response": True,
    }
