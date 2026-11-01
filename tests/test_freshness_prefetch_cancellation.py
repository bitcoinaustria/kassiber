"""Persisted cancellation fences real loopback prefetch transport."""

from contextlib import contextmanager
from concurrent.futures import ThreadPoolExecutor
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import socketserver
import socket
import sqlite3
import threading

from embit import bip32, script
import pytest

from kassiber import daemon_freshness
from kassiber.cli import handlers
from kassiber.core import sync as core_sync
from kassiber.core import freshness, sync_backends
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
                    control.execute("UPDATE freshness_jobs SET cancel_requested=1 WHERE status IN ('queued','running')")
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


@contextmanager
def cancellation_book(path, *, wallet_count=1, address_count=8):
    conn = open_db(path)
    try:
        conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('w','Set','fixed')")
        conn.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('p','w','Book','EUR','fixed')")
        set_setting(conn, "context_workspace", "w")
        set_setting(conn, "context_profile", "p")
        for index in range(wallet_count):
            root = bip32.HDKey.from_seed(f"cancellation fixture wallet {index}".encode())
            addresses = [script.p2wpkh(root.derive([i]).key.get_public_key()).address() for i in range(address_count)]
            conn.execute(
                "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES(?,'w','p',?,'address',?,'fixed')",
                (f"a{index}", f"Wallet {index}", json.dumps({"addresses": addresses, "backend": "fixture"})),
            )
        conn.commit()
        yield conn
    finally:
        conn.close()


def backend_runtime(protocol, url):
    return {"backends": {"fixture": {"name": "fixture", "kind": protocol, "url": url, "batch_size": 2}},
            "default_backend": "fixture"}


@pytest.mark.parametrize("wallet_count", [1, 3])
@pytest.mark.parametrize("protocol", ["esplora", "electrum"])
@pytest.mark.parametrize("entrypoint", ["freshness", "wallet-sync"])
def test_uncancelled_request_counts(tmp_path, protocol, entrypoint, wallet_count):
    with cancellation_book(tmp_path / "data", wallet_count=wallet_count, address_count=30) as conn:
        hits = {"requests": [], "cancelled_at": -1, "lock": threading.Lock()}
        book = conn.execute("PRAGMA database_list").fetchone()[2]
        with cancellation_backend(protocol, book, hits) as url:
            runtime = backend_runtime(protocol, url)
            if entrypoint == "freshness":
                result = daemon_freshness._freshness_run_payload(conn, runtime, {"rates": False, "journals": False})
            else:
                result = daemon_freshness._wallets_sync_payload(conn, runtime, {"all": True, "process_journals": False}, strict=True)
        assert [job["status"] for job in result["completed"]] == ["done"] * wallet_count
        expected = 154 * wallet_count if protocol == "esplora" else 1 + 92 * wallet_count
        assert len(hits["requests"]) == expected
        print(f"REQUEST_COUNT {entrypoint} {protocol} {wallet_count}: {len(hits['requests'])}")


@pytest.mark.parametrize("mode", ["background", "report-read", "duplicate", "prefetch-failure"])
@pytest.mark.parametrize("protocol", ["esplora", "electrum"])
@pytest.mark.parametrize("cancel", [False, True])
def test_isolated_freshness_uses_owner_thread(tmp_path, monkeypatch, mode, protocol, cancel):
    with cancellation_book(tmp_path / "data") as conn:
        classes = dict.fromkeys(freshness.get_policy(conn, "p").source_classes, False)
        classes[freshness.SOURCE_ONCHAIN] = True
        freshness.set_policy(conn, "p", background_enabled=True, report_read_sync=True, source_classes=classes)
        conn.commit()
        before_wallet = dict(conn.execute("SELECT * FROM wallets").fetchone())
        if mode == "prefetch-failure":
            def failed_prefetch(*_args, **_kwargs):
                raise RuntimeError("injected batch prefetch failure")
            monkeypatch.setattr(daemon_freshness, "_prefetch_onchain_freshness_jobs", failed_prefetch)
        elif mode == "duplicate":
            specs = daemon_freshness._freshness_wallet_source_specs(conn, "p", include_rates=False, include_journals=False)
            specs.append({**specs[0], "source_key": specs[0]["source_key"] + ":duplicate"})
            duplicate_jobs = daemon_freshness._enqueue_freshness_jobs(conn, "p", specs)
        hits = {"requests": [], "cancelled_at": None if cancel else -1, "lock": threading.Lock()}
        book = conn.execute("PRAGMA database_list").fetchone()[2]
        with cancellation_backend(protocol, book, hits) as url:
            runtime = backend_runtime(protocol, url)
            if mode == "background":
                class Output:
                    def write(self, _event):
                        pass
                daemon_freshness._freshness_background_tick(conn, runtime, Output())
            elif mode == "report-read":
                daemon_freshness._auto_sync_wallets_if_enabled(conn, runtime, state={})
            elif mode == "duplicate":
                profile = conn.execute("SELECT * FROM profiles").fetchone()
                prefetched = daemon_freshness._prefetch_onchain_freshness_jobs(conn, runtime, profile, duplicate_jobs)
                assert prefetched == {}
                freshness.run_due_jobs(conn, daemon_freshness._freshness_handlers(runtime, prefetched_onchain=prefetched), profile_id="p", limit=2)
            else:
                daemon_freshness._freshness_run_payload(
                    conn, runtime, {"rates": False, "journals": False, "force_full": mode == "duplicate"},
                )
        jobs = [dict(row) for row in conn.execute("SELECT * FROM freshness_jobs")]
        assert len(jobs) == (2 if mode == "duplicate" else 1)
        assert [job["status"] for job in jobs] == ["cancelled" if cancel else "done"] * len(jobs)
        assert hits["requests"]
        if cancel:
            assert hits["cancelled_at"] is not None
            assert len(hits["requests"]) - hits["cancelled_at"] <= 1
            assert dict(conn.execute("SELECT * FROM wallets").fetchone()) == before_wallet
            for table in ("transactions", "chain_observer_instances", "wallet_utxo_refreshes"):
                assert conn.execute(f"SELECT count(*) FROM {table}").fetchone()[0] == 0
            state = conn.execute("SELECT checkpoint_json FROM freshness_source_states").fetchone()
            assert json.loads(state[0]) == {}


@pytest.mark.parametrize("protocol", ["esplora", "electrum"])
@pytest.mark.parametrize("cancel", [False, True])
def test_repair_fetch_progress_uses_owner_thread(tmp_path, protocol, cancel):
    with cancellation_book(tmp_path / "data") as conn:
        job = daemon_freshness._freshness_run_payload(conn, {}, {"rates": False, "journals": False, "run": False})["enqueued"][0]
        wallet = dict(conn.execute("SELECT * FROM wallets").fetchone())
        profile = dict(conn.execute("SELECT * FROM profiles").fetchone())
        owner = threading.get_ident()
        progress_threads = []

        def check():
            freshness._check_cancelled(conn, job["id"])

        def progress(_payload):
            progress_threads.append(threading.get_ident())
            check()

        hits = {"requests": [], "cancelled_at": None if cancel else -1, "lock": threading.Lock()}
        book = conn.execute("PRAGMA database_list").fetchone()[2]
        with cancellation_backend(protocol, book, hits) as url:
            token = core_sync.sync_progress_emitter.set(progress)
            try:
                def fetch():
                    return core_sync.fetch_wallet_backend(backend_runtime(protocol, url), profile, wallet,
                                                          handlers._wallet_sync_hooks(commit=False), force_full=True, check_cancelled=check)
                if cancel:
                    with pytest.raises(AppError) as error:
                        fetch()
                    assert error.value.code == "cancelled"
                    assert len(hits["requests"]) - hits["cancelled_at"] <= 1
                else:
                    assert isinstance(fetch(), core_sync.WalletBackendFetch)
            finally:
                core_sync.sync_progress_emitter.reset(token)
        assert progress_threads and set(progress_threads) == {owner}


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


# Native clients speak newline-delimited JSON-RPC, sometimes as a JSON batch.
BITCOIN_HEADER = ("01000000" + "00" * 32
                  + "3ba3edfd7a7b12b27ac72c3e67768f617fc81bc3888a51323a9fb8aa4b1e5e4a"
                  + "29ab5f49ffff001d1dac2b7c")
LIQUID_HEADER = "01000000" + "00" * 64 + "00000000" * 2 + "0000"


@contextmanager
def native_electrum_backend(book, hits, *, liquid=False):
    header = LIQUID_HEADER if liquid else BITCOIN_HEADER
    sockets = []

    class Handler(socketserver.StreamRequestHandler):
        def handle(self):
            with hits["lock"]:
                hits["connections"] += 1
                connection = hits["connections"]
                sockets.append(self.request)
            for line in self.rfile:
                payload = json.loads(line)
                batch = isinstance(payload, list)
                responses = []
                for request in payload if batch else [payload]:
                    method = request["method"]
                    with hits["lock"]:
                        hits["requests"].append((connection, method))
                        if method == "blockchain.scripthash.get_history" and hits["cancelled_at"] is None:
                            with sqlite3.connect(book) as control:
                                control.execute("UPDATE freshness_jobs SET cancel_requested=1 WHERE status IN ('queued','running')")
                            hits["cancelled_at"] = len(hits["requests"])
                    result = {
                        "server.version": ["fixture", "1.4"],
                        "server.ping": None,
                        "blockchain.headers.subscribe": {"height": 0, "hex": header},
                        "blockchain.block.header": header,
                        "blockchain.block.headers": {"count": 1, "hex": header, "max": 2016},
                        "blockchain.scripthash.subscribe": None,
                        "blockchain.scripthash.get_history": [],
                        "blockchain.scripthash.listunspent": [],
                    }[method]
                    responses.append({"jsonrpc": "2.0", "id": request["id"], "result": result})
                self.wfile.write(json.dumps(responses if batch else responses[0]).encode() + b"\n")
                self.wfile.flush()

    with socketserver.ThreadingTCPServer(("127.0.0.1", 0), Handler) as server:
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        try:
            yield f"tcp://127.0.0.1:{server.server_address[1]}"
        finally:
            server.shutdown()
            # An SDK client can stay referenced by a cancellation traceback.
            # Close our server sockets before joining its handler threads.
            for connection in sockets:
                try:
                    connection.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
            thread.join()


def descriptor_config(observer):
    if observer == "bdk":
        # Two identities prove a cancelled scan cannot open the next client.
        root = bip32.HDKey.from_seed(b"native cancellation fixture")
        return {"chain": "bitcoin", "network": "main", "xpub": root.to_public().to_base58(),
                "script_types": ["p2wpkh", "p2tr"], "gap_limit": 2, "backend": "fixture"}
    from tests.test_lwk_observer import descriptor
    return {"chain": "liquid", "network": "elementsregtest", "descriptor": descriptor(),
            "gap_limit": 2, "backend": "fixture"}


@pytest.mark.parametrize("observer,protocol", [("bdk", "electrum"), ("lwk", "electrum"), ("lwk", "esplora")])
@pytest.mark.parametrize("cancel", [False, True])
def test_descriptor_route_fences_native_steps(tmp_path, monkeypatch, observer, protocol, cancel):
    from kassiber.core.chain_observer.bdk import BdkObserver
    from kassiber.core.chain_observer.lwk import LwkObserver

    with cancellation_book(tmp_path / "data") as conn:
        config = descriptor_config(observer)
        conn.execute("UPDATE wallets SET kind='descriptor', config_json=?", (json.dumps(config),))
        conn.commit()
        before_wallet = dict(conn.execute("SELECT * FROM wallets").fetchone())
        facts_calls = []
        observer_type = BdkObserver if observer == "bdk" else LwkObserver
        original_facts = observer_type._facts
        tip_calls = []
        if observer == "lwk":
            original_client = LwkObserver._client

            class TrackedClient:
                def __init__(self, client):
                    self.client = client

                def __getattr__(self, name):
                    return getattr(self.client, name)

                def tip(self):
                    tip_calls.append(True)
                    return self.client.tip()

            monkeypatch.setattr(LwkObserver, "_client", lambda self, network: TrackedClient(original_client(self, network)))

        def facts(self, *args, **kwargs):
            facts_calls.append(self.identity.id)
            return original_facts(self, *args, **kwargs)

        monkeypatch.setattr(observer_type, "_facts", facts)
        hits = {"requests": [], "cancelled_at": None if cancel else -1,
                "connections": 0, "lock": threading.Lock()}
        book = conn.execute("PRAGMA database_list").fetchone()[2]
        server = cancellation_backend(protocol, book, hits) if protocol == "esplora" else native_electrum_backend(book, hits, liquid=observer == "lwk")
        with server as url:
            result = daemon_freshness._freshness_run_payload(conn, backend_runtime(protocol, url), {"rates": False, "journals": False})
        assert result["completed"][0]["status"] == ("cancelled" if cancel else "done")
        assert hits["requests"]
        if cancel:
            assert hits["cancelled_at"] is not None
            assert facts_calls == []
            assert dict(conn.execute("SELECT * FROM wallets").fetchone()) == before_wallet
            for table in ("transactions", "chain_observer_instances", "chain_observer_values", "wallet_utxo_refreshes"):
                assert conn.execute(f"SELECT count(*) FROM {table}").fetchone()[0] == 0
            if protocol == "electrum":
                assert hits["connections"] == 1
                # Requests internal to the active SDK full scan may finish.
                # No post-scan tip query or second identity may start.
                if observer == "lwk":
                    # full_scan_to_index itself also reads the tip internally.
                    assert tip_calls == []
        elif protocol == "electrum":
            assert len(facts_calls) == (2 if observer == "bdk" else 1)
        else:
            assert facts_calls == []  # Esplora is the named compatibility route.


@pytest.mark.parametrize("observer,protocol", [("bdk", "electrum"), ("lwk", "electrum"), ("lwk", "esplora")])
def test_native_cancellation_prevents_connection(tmp_path, observer, protocol):
    from kassiber.core.chain_observer.bdk import BdkObserver, bdk_branches_for_identity
    from kassiber.core.chain_observer.identity import identities_for_wallet
    from kassiber.core.chain_observer.lwk import LwkObserver, lwk_network
    from kassiber.wallet_descriptors import load_descriptor_plan, default_policy_asset_id

    config = descriptor_config(observer)
    wallet = {"id": "a", "workspace_id": "w", "profile_id": "p", "config_json": json.dumps(config)}
    identity = identities_for_wallet(wallet, observer_kind=observer)[0]
    plan = load_descriptor_plan(config)
    hits = {"requests": [], "cancelled_at": -1, "connections": 0, "lock": threading.Lock()}
    server = cancellation_backend(protocol, None, hits) if protocol == "esplora" else native_electrum_backend(None, hits, liquid=observer == "lwk")
    with server as url:
        backend = backend_runtime(protocol, url)["backends"]["fixture"]
        if observer == "bdk":
            instance = BdkObserver(identity=identity, backend=backend, branches=bdk_branches_for_identity(plan, identity), gap_limit=2)
            client = instance._client
        else:
            asset = default_policy_asset_id("elementsregtest")
            instance = LwkObserver(identity=identity, backend=backend, descriptor_plan=plan, policy_asset_id=asset, stored_values={})
            client = lambda: instance._client(lwk_network(identity.network, asset))

        def cancelled():
            raise AppError("Saved cancellation", code="cancelled", retryable=False)

        token = sync_transport_check.set(cancelled)
        try:
            with pytest.raises(AppError) as error:
                client()
            assert error.value.code == "cancelled"
        finally:
            sync_transport_check.reset(token)
    assert hits["requests"] == []
    assert hits["connections"] == 0


def test_bdk_cancellation_between_header_reads(monkeypatch):
    from types import SimpleNamespace
    from unittest.mock import Mock
    from tests.test_bdk_observer import BdkDependencyContractTest

    observer = BdkDependencyContractTest()._observer()
    observer.backend_kind = "electrum"
    stopped = threading.Event()
    client = Mock()

    def tip():
        stopped.set()
        return SimpleNamespace(height=0)

    client.block_headers_subscribe.side_effect = tip
    client.block_header.return_value = BITCOIN_HEADER

    def check():
        if stopped.is_set():
            raise AppError("Saved cancellation", code="cancelled")

    token = sync_transport_check.set(check)
    try:
        with pytest.raises(AppError) as error:
            observer._remote_block_hash(client, 0)
        assert error.value.code == "cancelled"
    finally:
        sync_transport_check.reset(token)
    client.block_header.assert_not_called()


def test_bdk_cancellation_stops_incremental_widening(monkeypatch):
    from unittest.mock import Mock
    from tests.test_bdk_observer import BdkDependencyContractTest

    observer = BdkDependencyContractTest()._observer()
    stopped = threading.Event()
    client = Mock()
    wallet = Mock()
    horizons = []

    def horizon(_wallet):
        horizons.append(True)
        return True

    def sync(_client, _wallet):
        stopped.set()
        return object()

    def check():
        if stopped.is_set():
            raise AppError("Saved cancellation", code="cancelled")

    monkeypatch.setattr(observer, "_reveal_scan_horizon", horizon)
    monkeypatch.setattr(observer, "_incremental_sync", sync)
    token = sync_transport_check.set(check)
    try:
        with pytest.raises(AppError) as error:
            observer._sync_revealed_horizon(client, wallet)
        assert error.value.code == "cancelled"
    finally:
        sync_transport_check.reset(token)
    assert len(horizons) == 1
