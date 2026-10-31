"""Persisted cancellation fences real loopback prefetch transport."""

from contextlib import contextmanager
from concurrent.futures import ThreadPoolExecutor
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import socketserver
import sqlite3
import threading

from embit import bip32, script
import pytest

from kassiber import daemon_freshness
from kassiber.core import sync_backends
from kassiber.core.sync_control import sync_transport_check
from kassiber.db import open_db, set_setting
from kassiber.errors import AppError


@contextmanager
def cancellation_backend(protocol, book, hits):
    def record(request):
        with hits["lock"]:
            hits["requests"].append(request)
            fetching = (request == "blockchain.scripthash.get_history" if protocol == "electrum"
                        else request.startswith("/scripthash/"))
            if fetching and hits["cancelled_at"] is None:
                with sqlite3.connect(book) as control:
                    control.execute("UPDATE freshness_jobs SET cancel_requested=1 WHERE status='queued'")
                hits["cancelled_at"] = len(hits["requests"])

    class HttpHandler(BaseHTTPRequestHandler):
        def do_GET(self):
            record(self.path)
            if self.path == "/blocks/tip/hash":
                body = ("11" * 32).encode()
            elif self.path == "/blocks/tip/height":
                body = b"100"
            else:
                value = ({"chain_stats": {"tx_count": 0}, "mempool_stats": {"tx_count": 0}}
                         if len(self.path.split("/")) == 3 else [])
                body = json.dumps(value).encode()
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_args):
            pass

    class ElectrumHandler(socketserver.StreamRequestHandler):
        def handle(self):
            for line in self.rfile:
                request = json.loads(line)
                method = request["method"]
                record(method)
                result = {
                    "server.version": ["fixture", "1.6"],
                    "blockchain.headers.subscribe": {"height": 100, "hex": "00" * 80},
                    "blockchain.scripthash.subscribe": "used",
                    "blockchain.scripthash.get_history": [],
                    "blockchain.scripthash.listunspent": [],
                }[method]
                self.wfile.write(json.dumps({"id": request["id"], "result": result}).encode() + b"\n")
                self.wfile.flush()

    server_type = ThreadingHTTPServer if protocol == "esplora" else socketserver.ThreadingTCPServer
    handler = HttpHandler if protocol == "esplora" else ElectrumHandler
    with server_type(("127.0.0.1", 0), handler) as server:
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        try:
            scheme = "http" if protocol == "esplora" else "tcp"
            yield f"{scheme}://127.0.0.1:{server.server_address[1]}"
        finally:
            server.shutdown()
            thread.join()


@pytest.mark.parametrize("protocol", ["esplora", "electrum"])
@pytest.mark.parametrize("entrypoint", ["freshness", "wallet-sync"])
def test_prefetch_stops_requests_after_saved_cancellation(tmp_path, protocol, entrypoint):
    conn = open_db(tmp_path / "data")
    try:
        conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('w','Set','fixed')")
        conn.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('p','w','Book','EUR','fixed')")
        set_setting(conn, "context_workspace", "w")
        set_setting(conn, "context_profile", "p")
        root = bip32.HDKey.from_seed(b"synthetic cancellation fixture")
        addresses = [script.p2wpkh(root.derive([index]).key.get_public_key()).address() for index in range(8)]
        conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('a','w','p','Wallet','address',?,'fixed')",
                     (json.dumps({"addresses": addresses, "backend": "fixture"}),))
        conn.commit()
        before_wallet = dict(conn.execute("SELECT * FROM wallets").fetchone())
        hits = {"requests": [], "cancelled_at": None, "lock": threading.Lock()}
        book = conn.execute("PRAGMA database_list").fetchone()[2]
        with cancellation_backend(protocol, book, hits) as url:
            runtime = {"backends": {"fixture": {"name": "fixture", "kind": protocol, "url": url, "batch_size": 2}},
                       "default_backend": "fixture"}
            args = {"rates": False, "journals": False}
            route = daemon_freshness._freshness_run_payload
            if entrypoint == "wallet-sync":
                route = daemon_freshness._wallets_sync_payload
                args = {"wallet": "a", "process_journals": False}
            result = route(conn, runtime, args, **({"strict": True} if entrypoint == "wallet-sync" else {}))
        job = dict(conn.execute("SELECT * FROM freshness_jobs").fetchone())
        assert job["status"] == "cancelled"
        assert job["cancel_requested"] == 1
        assert dict(conn.execute("SELECT * FROM wallets").fetchone()) == before_wallet
        for table in ("transactions", "chain_observer_instances", "wallet_utxo_refreshes"):
            assert conn.execute(f"SELECT count(*) FROM {table}").fetchone()[0] == 0
        state = conn.execute("SELECT checkpoint_json FROM freshness_source_states").fetchone()
        assert json.loads(state[0]) == {}
        assert result["completed"][0]["status"] == "cancelled"
        assert hits["cancelled_at"] is not None
        remaining = len(hits["requests"]) - hits["cancelled_at"]
        print(f"{protocol} {entrypoint}: {remaining} requests after cancellation")
        # One other member of the already transmitted two-request batch may finish.
        assert remaining <= 1
    finally:
        conn.close()


@pytest.mark.parametrize("cancel_during_batch", [False, True])
@pytest.mark.parametrize("refusal_code", ["cancelled", "state_not_ready"])
def test_pooled_cancellation_keeps_other_wallet_results(monkeypatch, cancel_during_batch, refusal_code):
    cancelled = threading.Event()
    if not cancel_during_batch:
        cancelled.set()
    wire_batches = []

    class Client:
        def __init__(self, _backend):
            pass

        def __enter__(self):
            return self

        def __exit__(self, *_args):
            pass

        def batch_call(self, requests):
            markers = [params[0] for _method, params in requests]
            wire_batches.append(markers)
            cancelled.set()
            return markers

    monkeypatch.setattr(sync_backends, "ElectrumClient", Client)
    monkeypatch.setattr(sync_backends._ElectrumBatchDispatcher, "_COALESCE_SECONDS", 0.2)
    barrier = threading.Barrier(2)

    def fetch(client, marker):
        def check():
            if marker == "cancelled" and cancelled.is_set():
                raise AppError("Freshness job stopped", code=refusal_code, retryable=False)

        token = sync_transport_check.set(check)
        try:
            barrier.wait(timeout=5)
            return client.batch_call([("example", [marker])] * 2)
        finally:
            sync_transport_check.reset(token)

    with sync_backends.shared_electrum_client_pool() as pool:
        client = pool.client({"kind": "electrum", "url": "tcp://127.0.0.1:1", "batch_size": 1})
        with ThreadPoolExecutor(max_workers=2) as workers:
            good = workers.submit(fetch, client, "good")
            stopped = workers.submit(fetch, client, "cancelled")
            assert good.result(timeout=5) == ["good", "good"]
            with pytest.raises(AppError) as error:
                stopped.result(timeout=5)
            assert error.value.code == refusal_code
    # Cancellation during a batch permits its one transmitted member only.
    assert all("cancelled" not in batch for batch in wire_batches[int(cancel_during_batch):])
