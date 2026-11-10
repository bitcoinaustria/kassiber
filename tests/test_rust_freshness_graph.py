import copy
from contextlib import ExitStack
import json
import os
from pathlib import Path
import select
import subprocess
import threading
from unittest.mock import patch

import pytest

from kassiber import daemon_freshness as df
from kassiber.core import freshness, transaction_graph as graph
from tests.rust_freshness_session import Session
from tests.rust_sync_chain import Chain, FOREIGN, serve
from tests.rust_sync_env import child_env
from tests.rust_sync_parity import connect, copy_book, create_book, deterministic_sources, dump_bytes, runtime
from tests.test_rust_wallet_publication import _fixture
from tests.test_rust_wallet_sync_cli import _books, _environment
from tests.rust_fixture_binary import checked


FIXTURE = os.environ.get("KASSIBER_RUST_GRAPH_FIXTURE")
if FIXTURE is None and os.environ.get("KASSIBER_RUST_BIN"):
    FIXTURE = str(checked(Path(os.environ["KASSIBER_RUST_BIN"]).resolve().parent / "examples/graph_fixture",
                          "cargo build -p kassiber-wallets --example graph_fixture --features test-harness --offline --locked"))
pytestmark = pytest.mark.skipif(not FIXTURE, reason="requires native graph fixtures")
NOW = "2030-01-01T00:00:01Z"
CONTEXT = "INSERT INTO settings VALUES('context_workspace','w'); INSERT INTO settings VALUES('context_profile','p');"


def add(conn, txid, raw=None, *, index=0, wallet="a", asset="BTC"):
    conn.execute("""INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,
        occurred_at,direction,amount,asset,raw_json,created_at)
        VALUES(?,'w','p',?,?,?,'fixed','inbound',99000000,?,?,'fixed')""",
        (f"row-{index}", wallet, txid, f"fp-{index}", asset, json.dumps(raw if raw is not None else {"txid": txid})))


def prepared(tmp_path, chain, txids):
    config, _, _ = _fixture("empty")
    python_book, rust_book = (tmp_path / name for name in ("python.sqlite", "rust.sqlite"))
    create_book(python_book, config, extra_sql=CONTEXT)
    with connect(python_book) as conn, patch.object(freshness, "now_iso", return_value=NOW):
        job = freshness.enqueue_job(conn, profile_id="p", job_type="onchain_wallet_history",
            source_key="onchain_wallet:a", source_type="onchain_wallet", source_label="Cold")
        conn.execute("UPDATE freshness_jobs SET id='sync',status='done' WHERE id=?", (job["id"],))
        for index, txid in enumerate(txids):
            add(conn, txid, index=index)
    copy_book(python_book, rust_book)
    return python_book, rust_book


def native(tmp_path, book, backend, prefix="graph", **flags):
    env = child_env(tmp_path)
    if "KASSIBER_NO_EGRESS" in os.environ:
        env["KASSIBER_NO_EGRESS"] = os.environ["KASSIBER_NO_EGRESS"]
    result = subprocess.run([str(Path(FIXTURE).resolve())], input=json.dumps({
        "book": str(book), "backend": backend, "prefix": prefix, **flags}).encode(),
        capture_output=True, timeout=300, env=env)
    assert result.returncode == 0 and result.stderr == b"", result.stderr.decode()
    envelope = json.loads(result.stdout)["envelope"]
    assert envelope["kind"] == "graph.followup", envelope
    return envelope["data"]


class Out:
    def __init__(self):
        self.events = []

    def write(self, event):
        self.events.append(event)


def reference(book, rt, prefix):
    out = Out()
    with connect(book) as conn, ExitStack() as stack:
        stack.enter_context(deterministic_sources(prefix))
        stack.enter_context(patch.object(freshness, "now_iso", return_value=NOW))
        stack.enter_context(patch.object(graph, "now_iso", return_value=NOW))
        stack.enter_context(patch("kassiber.core.wallet_graph_references.monotonic", return_value=0))
        token = df._GRAPH_SYNC_FOLLOWUPS.set([])
        try:
            grant = df._graph_sync_grant(conn, rt, freshness.get_job(conn, "sync"),
                conn.execute("SELECT * FROM wallets WHERE id='a'").fetchone(), None)
        finally:
            df._GRAPH_SYNC_FOLLOWUPS.reset(token)
        assert grant
        df._run_graph_followup(conn, rt, out, grant, threading.Event())
    assert len(out.events) == 1, out.events
    event = out.events[0]
    assert event["kind"] == "ui.freshness.background" and event["event"] is True
    assert "request_id" not in event
    return event["data"]


def parity(tmp_path, books, rt, prefix="graph"):
    expected = reference(books[0], rt, prefix)
    actual = native(tmp_path, books[1], rt["backends"]["fixture"], prefix)
    (tmp_path / "parity.json").write_text(json.dumps({"actual": actual, "expected": expected}, indent=2))
    assert actual == expected
    assert dump_bytes(books[0]) == dump_bytes(books[1])
    return actual["completed"][0]


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("shape", ["graphless", "partial", "complete", "local", "conflict", "wrong-network", "invalid-scope", "invalid-txid", "invalid-nontext-txid", "removed", "legacy-cache"])
def test_native_graph_reference_full_book_parity(tmp_path, protocol, shape):
    chain = Chain()
    parent = chain.fund(FOREIGN, 100000)
    txid = chain.spend([parent], [(FOREIGN, 99000)])
    books = prepared(tmp_path, chain, [txid])
    for book in books:
        with connect(book) as conn:
            raw = chain.esplora_tx(txid)
            if shape == "partial":
                raw["vin"][0].pop("prevout")
            elif shape == "wrong-network":
                raw = {"txid": txid, "network": "regtest"}
            elif shape == "invalid-scope":
                raw = {"txid": txid, "chain": "liquid"}
            elif shape == "invalid-txid":
                raw = {"txid": "invalid"}
            elif shape == "invalid-nontext-txid":
                raw = {"txid": 123}
            if shape in {"partial", "complete", "wrong-network", "invalid-scope", "invalid-txid", "invalid-nontext-txid"}:
                conn.execute("UPDATE transactions SET raw_json=?", (json.dumps(raw),))
            if shape in {"local", "conflict", "removed"}:
                conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('b','w','p','Other','file','{}','fixed')")
                add(conn, txid, raw, index=1, wallet="b")
                if shape == "conflict":
                    raw = copy.deepcopy(raw)
                    raw["vout"][0]["value"] += 1
                    add(conn, txid, raw, index=2, wallet="b")
                elif shape == "removed":
                    raw["removed"] = True
                    conn.execute("UPDATE transactions SET raw_json=? WHERE wallet_id='b'", (json.dumps(raw),))
            if shape == "legacy-cache":
                raw = {"txid": txid, "vin": [{}], "vout": [{"n": 0, "value": 1}]}
                conn.execute("INSERT INTO transaction_graph_cache VALUES(?,'bitcoin','main',?,?,?,?)",
                             (graph.GRAPH_CACHE_SCHEMA_VERSION, txid, json.dumps(raw), NOW, NOW))
    with serve(chain, protocol) as url:
        rt = runtime(protocol, url)
        job = parity(tmp_path, books, rt)
        expected = 0 if shape in {"complete", "local", "conflict", "invalid-scope", "invalid-txid", "invalid-nontext-txid"} else 1
        assert job["status"] == "done"
        assert job["result"]["attempted"] == expected
        assert job["result"]["cached"] == (0 if shape == "wrong-network" else expected)
        assert not any(key in json.dumps(job) for key in (txid, "after_rowid", "freshness_checkpoint", "descriptor"))
        count = len(chain.hits)
        parity(tmp_path, books, rt, "repeat")
        assert len(chain.hits) == count


@pytest.mark.parametrize("shape", ["graphless", "partial", "complete", "missing-parent"])
def test_native_graph_core_rpc_parity(tmp_path, shape):
    """A wallet synced through Bitcoin Core fills its graph references from
    `getrawtransaction`, as Python's `_fetch_bitcoinrpc_transaction_graph`
    does; a parent the node cannot see (-5) leaves the graph uncached."""
    from tests.rust_core_chain import Node, serve as serve_core

    chain = Chain()
    parent = chain.fund(FOREIGN, 100000)
    second = chain.fund(FOREIGN, 5000)
    # The hidden parent comes first: its -5 must not stop the second.
    txid = chain.spend([parent, second] if shape == "missing-parent" else [parent], [(FOREIGN, 99000)])
    books = prepared(tmp_path, chain, [txid])
    for book in books:
        with connect(book) as conn:
            raw = chain.esplora_tx(txid)
            if shape == "partial":
                raw["vin"][0].pop("prevout")
            if shape in {"partial", "complete"}:
                conn.execute("UPDATE transactions SET raw_json=?", (json.dumps(raw),))
    node = Node(chain)
    if shape == "missing-parent":
        node.chain.visible = (lambda visible: lambda: [t for t in visible() if t != parent[0]])(chain.visible)
    with serve_core(node) as url:
        rt = runtime("bitcoinrpc", url)
        rt["backends"]["fixture"].update({"username": "fixture", "password": "secret"})
        job = parity(tmp_path, books, rt)
        assert job["status"] == "done"
        attempted = 0 if shape == "complete" else 1
        assert job["result"]["attempted"] == attempted
        assert job["result"]["cached"] == (0 if shape == "missing-parent" else attempted)
        calls = len(node.calls)
        parity(tmp_path, books, rt, "repeat")
        if shape != "missing-parent":
            assert len(node.calls) == calls
        else:
            with connect(books[1]) as conn:
                assert conn.execute("SELECT COUNT(*) FROM transaction_graph_cache WHERE txid=?",
                                    (second[0],)).fetchone()[0] == 1


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_native_graph_transaction_limit_cursor_and_cache_parity(tmp_path, protocol):
    chain = Chain()
    parent = chain.fund(FOREIGN, 100000)
    ids = [chain.spend([parent], [(FOREIGN, 99000)], locktime=n) for n in range(52)]
    books = prepared(tmp_path, chain, ids)
    with serve(chain, protocol) as url:
        rt = runtime(protocol, url)
        first = parity(tmp_path, books, rt)
        assert first["result"]["attempted"] == first["result"]["cached"] == 50
        with connect(books[1]) as conn:
            assert json.loads(conn.execute("SELECT checkpoint_json FROM freshness_source_states WHERE source_key='wallet_graph:a'").fetchone()[0]) == {"after_rowid": 50}
        second = parity(tmp_path, books, rt, "remaining")
        assert second["result"]["attempted"] == second["result"]["cached"] == 2
        count = len(chain.hits)
        third = parity(tmp_path, books, rt, "cached")
        assert third["result"]["attempted"] == 0 and third["result"]["skipped"] == 52
        assert len(chain.hits) == count


def test_native_graph_sanitizes_legacy_reference_fields(tmp_path):
    chain = Chain()
    txid = chain.spend([chain.fund(FOREIGN, 100000)], [(FOREIGN, 99000)])
    books = prepared(tmp_path, chain, [txid])
    original = chain.esplora_tx

    def response(txid):
        raw = original(txid)
        raw["txid"] = txid.upper()
        raw["version"] = str(raw["version"])
        raw["private_backend_field"] = "must not persist"
        outputs = [*raw["vout"], *(entry["prevout"] for entry in raw["vin"])]
        for entry in outputs:
            entry["scriptPubKey"] = {"hex": entry.pop("scriptpubkey"), "address": "synthetic"}
            entry["value_sats"] = str(entry.pop("value"))
            entry["assetId"] = "btc"
            entry["pegoutAddress"] = "synthetic-pegout"
            entry["private_witness"] = ["must not persist"]
        return raw

    chain.esplora_tx = response
    with serve(chain, "esplora") as url:
        result = parity(tmp_path, books, runtime("esplora", url))
        assert result["result"]["cached"] == 1
    with connect(books[1]) as conn:
        payload = conn.execute("SELECT payload_json FROM transaction_graph_cache").fetchone()[0]
        assert "must not persist" not in payload


def test_native_graph_shared_prevout_budget_leaves_refused_row_first(tmp_path):
    chain = Chain()
    parents = [chain.fund(FOREIGN, 1000) for _ in range(251)]
    ids = [chain.spend(parents[:250], [(FOREIGN, 249000)]), chain.spend(parents[250:], [(FOREIGN, 500)])]
    books = prepared(tmp_path, chain, ids)
    with serve(chain, "electrum") as url:
        rt = runtime("electrum", url)
        rt["backends"]["fixture"]["batch_size"] = 250
        job = parity(tmp_path, books, rt)
        assert job["result"] == {"scanned": 2, "attempted": 1, "cached": 1, "skipped": 0, "failed": 0, "prevouts_requested": 250}
        with connect(books[1]) as conn:
            assert json.loads(conn.execute("SELECT checkpoint_json FROM freshness_source_states WHERE source_key='wallet_graph:a'").fetchone()[0]) == {"after_rowid": 1}
        assert parity(tmp_path, books, rt, "next")["result"]["prevouts_requested"] == 1


def test_native_graph_refuses_transaction_over_prevout_limit(tmp_path):
    chain = Chain()
    parents = [chain.fund(FOREIGN, 1000) for _ in range(251)]
    txid = chain.spend(parents, [(FOREIGN, 250000)])
    books = prepared(tmp_path, chain, [txid])
    with serve(chain, "electrum") as url:
        result = parity(tmp_path, books, runtime("electrum", url))
        assert result["result"] == {"scanned": 1, "attempted": 1, "cached": 0, "skipped": 0, "failed": 1, "prevouts_requested": 0}
        lookups = [row for row in chain.hits if row.get("method") == "blockchain.transaction.get"]
        assert len(lookups) == 2 and all(row["params"][0] == txid for row in lookups)


def test_native_graph_row_limit_and_private_cursor_parity(tmp_path):
    chain = Chain()
    books = prepared(tmp_path, chain, ["invalid"] * 1002)
    with serve(chain, "esplora") as url:
        rt = runtime("esplora", url)
        first = parity(tmp_path, books, rt)
        assert first["result"]["scanned"] == first["result"]["skipped"] == 1000
        with connect(books[1]) as conn:
            assert json.loads(conn.execute("SELECT checkpoint_json FROM freshness_source_states WHERE source_key='wallet_graph:a'").fetchone()[0]) == {"after_rowid": 1000}
        second = parity(tmp_path, books, rt, "remaining")
        assert second["result"]["scanned"] == second["result"]["skipped"] == 2
        assert chain.hits == []


def test_native_graph_local_inspection_limit_fails_closed(tmp_path):
    chain = Chain()
    txid = chain.spend([chain.fund(FOREIGN, 100000)], [(FOREIGN, 99000)])
    books = prepared(tmp_path, chain, [txid])
    for book in books:
        with connect(book) as conn:
            conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('b','w','p','Other','file','{}','fixed')")
            for index in range(1, 257):
                add(conn, txid, chain.esplora_tx(txid), index=index, wallet="b")
    with serve(chain, "esplora") as url:
        result = parity(tmp_path, books, runtime("esplora", url))
        assert result["result"]["skipped"] == 1
        assert chain.hits == []


@pytest.mark.parametrize("mismatch", ["current", "parent"])
def test_electrum_mismatched_transaction_is_refused_like_python(tmp_path, mismatch):
    # Since 10719a7d5 the reference refuses Electrum bytes that do not hash to
    # the requested current or parent txid, as the native graph always did.
    chain = Chain()
    parent = chain.fund(FOREIGN, 100000)
    requested = chain.spend([parent], [(FOREIGN, 99000)])
    other = chain.spend([parent], [(FOREIGN, 98000)], locktime=1)
    books = prepared(tmp_path, chain, [requested])
    if mismatch == "current":
        chain.txs[requested] = chain.txs[other]
    else:
        other_parent = chain.fund(FOREIGN, 110000)
        chain.txs[parent[0]] = chain.txs[other_parent[0]]
    with serve(chain, "electrum") as url:
        result = parity(tmp_path, books, runtime("electrum", url))
    assert result["result"]["failed"] == 1 and result["result"]["cached"] == 0
    for book in books:
        with connect(book) as conn:
            assert conn.execute("SELECT count(*) FROM transaction_graph_cache").fetchone()[0] == 0


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_native_graph_failures_advance_cursor_without_backend_fallback(tmp_path, protocol):
    chain = Chain()
    books = prepared(tmp_path, chain, [f"{index:064x}" for index in range(1, 52)])
    with serve(chain, protocol) as url:
        rt = runtime(protocol, url)
        rt["backends"]["other"] = {"name": "other", "kind": "esplora", "url": "http://127.0.0.1:1"}
        first = parity(tmp_path, books, rt)
        assert first["result"]["failed"] == first["result"]["attempted"] == 50
        second = parity(tmp_path, books, rt, "next")
        assert second["result"]["failed"] == second["result"]["attempted"] == 1


@pytest.mark.parametrize("shape", ["observation", "removed-observation", "liquid", "confidential", "foreign-cache", "mismatched-cache"])
def test_native_graph_local_scope_and_cache_guards(tmp_path, shape):
    chain = Chain()
    txid = chain.spend([chain.fund(FOREIGN, 100000)], [(FOREIGN, 99000)])
    books = prepared(tmp_path, chain, [txid])
    for book in books:
        with connect(book) as conn:
            raw = chain.esplora_tx(txid)
            if shape in {"observation", "removed-observation"}:
                conn.execute("INSERT INTO chain_analysis_observations(profile_id,chain,network,txid,payload_json,status_json,source_name,observed_at) VALUES('p','bitcoin','main',?,?,?,'fixture','fixed')",
                    (txid, json.dumps(raw), json.dumps({"removed": shape == "removed-observation"})))
            elif shape == "liquid":
                conn.execute("UPDATE transactions SET asset='LBTC',raw_json=?", (json.dumps(raw),))
                conn.execute("UPDATE wallets SET config_json=json_set(config_json,'$.chain','liquid','$.network','liquidv1')")
            elif shape == "confidential":
                raw["vout"][0]["value_state"] = "confidential"
                conn.execute("UPDATE transactions SET raw_json=?", (json.dumps(raw),))
            else:
                cached = graph._sanitize_graph_lookup_raw(raw, "bitcoin", txid)
                if shape == "mismatched-cache":
                    cached["txid"] = "f" * 64
                # The current cache version, so only the guard under test refuses it.
                conn.execute("INSERT INTO transaction_graph_cache VALUES(?,'bitcoin',?,?,?,?,?)",
                    (graph.GRAPH_CACHE_SCHEMA_VERSION, "test" if shape == "foreign-cache" else "main", txid,
                     json.dumps(cached), NOW, NOW))
    with serve(chain, "esplora") as url:
        result = parity(tmp_path, books, runtime("esplora", url))
        expected = 0 if shape in {"observation", "liquid", "confidential"} else 1
        assert result["result"]["attempted"] == expected


@pytest.mark.parametrize("fence", ["stop", "revoke", "offline", "wallet", "context", "paused", "cancelled"])
def test_native_graph_pretransport_fences_are_inert(tmp_path, monkeypatch, fence):
    chain = Chain()
    txid = chain.spend([chain.fund(FOREIGN, 100000)], [(FOREIGN, 99000)])
    books = prepared(tmp_path, chain, [txid])
    flags = {}
    if fence in {"stop", "revoke"}:
        flags[fence] = True
    elif fence == "offline":
        monkeypatch.setenv("KASSIBER_NO_EGRESS", "1")
    else:
        flags["before_sql"] = {
            "wallet": "UPDATE wallets SET config_json='{}'",
            "context": "UPDATE settings SET value='changed' WHERE key='context_profile'",
            "paused": "UPDATE freshness_source_states SET paused=1",
            "cancelled": "UPDATE freshness_jobs SET cancel_requested=1",
        }[fence]
    with serve(chain, "esplora") as url:
        before = dump_bytes(books[1])
        assert native(tmp_path, books[1], runtime("esplora", url)["backends"]["fixture"], **flags) is None
        assert chain.hits == []
        with connect(books[1]) as conn:
            assert conn.execute("SELECT count(*) FROM transaction_graph_cache").fetchone()[0] == 0
            assert conn.execute("SELECT count(*) FROM freshness_jobs WHERE job_type='wallet_graph_references'").fetchone()[0] == 0
        if "before_sql" not in flags:
            assert dump_bytes(books[1]) == before


class BlockGraph(list):
    def __init__(self, protocol, predicate=lambda _: True):
        super().__init__()
        self.protocol = protocol
        self.started = threading.Event()
        self.release = threading.Event()
        self.predicate = predicate

    def append(self, request):
        super().append(request)
        if not self.started.is_set() and self.predicate(request):
            self.started.set()
            assert self.release.wait(30)


def test_native_graph_cancels_between_electrum_prevout_batches(tmp_path):
    config, chain, _ = _fixture("empty")
    parents = [chain.fund(FOREIGN, 100000) for _ in range(5)]
    txid = chain.spend(parents, [(FOREIGN, 499000)])
    roots, books = _books(tmp_path, config)
    with serve(chain, "electrum") as url:
        env = _environment(tmp_path, "electrum", url)
        env.pop("KASSIBER_RUST_TEST_SYNC_PREFIX", None)
        with Session(roots[1], env) as session:
            session.send("test.freshness.execute", "sync", {"rates": False, "journals": False})
            assert session.until("sync", "test.freshness.execute")["data"]["completed"][0]["status"] == "done"
            with connect(books[1]) as conn:
                add(conn, txid)
            chain.hits = hits = BlockGraph("electrum", lambda r: r.get("method") == "blockchain.transaction.get" and r["params"][0] != txid)
            try:
                session.send("test.freshness.graph.start", "start")
                assert session.until("start")["kind"] == "test.freshness.graph.start"
                assert hits.started.wait(30)
                session.send("test.freshness.request.cancel", "cancel", {"target_request_id": "sync"})
                assert session.until("cancel")["data"]["cancelled"] is True
                hits.release.set()
                while not any(r["kind"] == "ui.freshness.background" for r in session.rows):
                    session.read()
                lookups = [r for r in hits if r.get("method") == "blockchain.transaction.get"]
                assert len(lookups) == 3
                with connect(books[1]) as conn:
                    assert conn.execute("SELECT count(*) FROM transaction_graph_cache").fetchone()[0] == 0
                session.finish()
            finally:
                hits.release.set()


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("automatic", [False, True])
def test_native_graph_enabled_worker_waits_for_terminal_handoff(tmp_path, protocol, automatic):
    config, chain, _ = _fixture("receive-change")
    roots, books = _books(tmp_path, config)
    with serve(chain, protocol) as url:
        env = _environment(tmp_path, protocol, url)
        env.pop("KASSIBER_RUST_TEST_SYNC_PREFIX", None)
        with Session(roots[1], env) as session:
            if not automatic:
                session.send("test.freshness.graph.start", "start")
                assert session.until("start")["kind"] == "test.freshness.graph.start"
            assert chain.hits == []
            kind = "test.freshness.product.execute" if automatic else "test.freshness.execute"
            session.send(kind, "sync", {"rates": False, "journals": False})
            terminal = session.until("sync", "ui.freshness.run" if automatic else kind)
            assert terminal["data"]["completed"][0]["status"] == "done"
            assert not any(row["kind"] == "ui.freshness.background" for row in session.rows)
            while not any(row["kind"] == "ui.freshness.background" for row in session.rows):
                session.read()
            event = next(row for row in session.rows if row["kind"] == "ui.freshness.background")
            assert event["event"] is True and "request_id" not in event
            assert event["data"]["completed"][0]["status"] == "done"
            assert event["data"]["completed"][0]["result"]["attempted"] == 0
            count = len(chain.hits)
            session.send("test.freshness.graph.start", "again")
            assert session.until("again")["kind"] == "test.freshness.graph.start"
            session.finish()
            assert len(chain.hits) == count
            with connect(books[1]) as conn:
                assert conn.execute("SELECT count(*) FROM freshness_jobs WHERE job_type='wallet_graph_references'").fetchone()[0] == 1


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("control", ["cancel", "revoke", "offline", "source-cancel", "source-pause", "graph-cancel", "graph-pause", "lock", "switch", "restore", "shutdown", "eof"])
def test_native_graph_worker_controls_and_owned_connection(tmp_path, protocol, control):
    config, chain, _ = _fixture("receive-change")
    roots, books = _books(tmp_path, config)
    with serve(chain, protocol) as url:
        env = _environment(tmp_path, protocol, url)
        env.pop("KASSIBER_RUST_TEST_SYNC_PREFIX", None)
        with Session(roots[1], env) as session:
            session.send("test.freshness.execute", "sync", {"rates": False, "journals": False})
            assert session.until("sync", "test.freshness.execute")["data"]["completed"][0]["status"] == "done"
            with connect(books[1]) as conn:
                conn.execute("UPDATE transactions SET raw_json=json_object('txid',external_id)")
                if control == "switch":
                    conn.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('q','w','Other','EUR','fixed')")
            chain.hits = hits = BlockGraph(protocol)
            try:
                session.send("test.freshness.graph.start", "start")
                assert session.until("start")["kind"] == "test.freshness.graph.start"
                assert hits.started.wait(30)
                session.send("status", "status")
                assert session.until("status")["kind"] == "status"
                if control == "eof":
                    session.process.stdin.close()
                elif control in {"cancel", "revoke"}:
                    session.send(f"test.freshness.request.{control}", "control", {"target_request_id": "sync"})
                elif control == "offline":
                    session.send("ui.network.offline.set", "control", {"enabled": True})
                elif control in {"source-cancel", "source-pause", "graph-cancel", "graph-pause"}:
                    graph_source = control.startswith("graph-")
                    action = control.rsplit("-", 1)[-1]
                    with connect(books[1]) as conn:
                        job = conn.execute("SELECT id,source_key FROM freshness_jobs WHERE job_type=?",
                            ("wallet_graph_references" if graph_source else "onchain_wallet_history",)).fetchone()
                    args = {"job_id": job[0]} if action == "cancel" else {"source_key": job[1]}
                    session.send(f"test.freshness.{action}", "control", args)
                elif control == "switch":
                    session.send("ui.profiles.switch", "control", {"profile_id": "q"})
                elif control == "restore":
                    session.send("ui.backup.apply", "control", {"token": "missing", "confirm": "RESTORE"})
                else:
                    session.send(f"daemon.{control}", "control")
                if control in {"cancel", "revoke", "offline", "source-cancel", "source-pause", "graph-cancel", "graph-pause"}:
                    assert session.until("control")["kind"] != "error"
                elif control != "eof":
                    while b"\n" in session.buffer or select.select([session.process.stdout], [], [], 0.1)[0]:
                        assert session.read().get("request_id") != "control"
                hits.release.set()
                if control in {"lock", "switch", "shutdown", "restore"}:
                    assert (session.until("control")["kind"] == "error") == (control == "restore")
                while not any(r["kind"] == "ui.freshness.background" for r in session.rows):
                    session.read()
                event = next(r for r in session.rows if r["kind"] == "ui.freshness.background")
                assert event["event"] is True and "request_id" not in event
                assert event["data"]["completed"][0]["status"] in ({"done"} if control == "source-cancel" else {"cancelled", "error"})
                if control.endswith("pause"):
                    session.send("test.freshness.resume", "resume", {"source_key": job[1]})
                    assert session.until("resume")["data"]["result"]["paused"] is False
                count = len(hits)
                with connect(books[1]) as conn:
                    assert bool(conn.execute("SELECT count(*) FROM transaction_graph_cache").fetchone()[0]) == (control == "source-cancel")
                    assert conn.execute("SELECT count(*) FROM freshness_jobs WHERE job_type='wallet_graph_references' AND status='running'").fetchone()[0] == 0
                if control in {"shutdown", "eof"}:
                    session.process.wait(timeout=30)
                    assert session.process.returncode == 0
                else:
                    session.finish()
                assert len(hits) == count
            finally:
                hits.release.set()


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_native_graph_worker_passive_paths_and_restart_have_no_authority(tmp_path, protocol):
    config, chain, _ = _fixture("receive-change")
    roots, books = _books(tmp_path, config)
    with connect(books[1]) as conn:
        conn.execute("INSERT INTO freshness_jobs(id,profile_id,job_type,source_key,source_type,source_label,status,phase,priority,created_at,updated_at) VALUES('old-graph','p','wallet_graph_references','wallet_graph:a','onchain_wallet','Graph','running','backend_fetch',200,'fixed','fixed')")
    with serve(chain, protocol) as url:
        env = _environment(tmp_path, protocol, url)
        for _ in range(2):
            with Session(roots[1], env) as session:
                for index in range(3):
                    session.send("test.freshness.graph.start", f"start-{index}")
                    assert session.until(f"start-{index}")["kind"] == "test.freshness.graph.start"
                    session.send("status", f"read-{index}")
                    assert session.until(f"read-{index}")["kind"] == "status"
                session.finish()
            assert chain.hits == []
            with connect(books[1]) as conn:
                assert conn.execute("SELECT status FROM freshness_jobs WHERE id='old-graph'").fetchone()[0] == "running"


def test_native_graph_request_identity_and_replay_are_fenced(tmp_path):
    config, chain, _ = _fixture("receive-change")
    roots, books = _books(tmp_path, config)
    with serve(chain, "esplora") as url:
        env = _environment(tmp_path, "esplora", url)
        env.pop("KASSIBER_RUST_TEST_SYNC_PREFIX", None)
        with Session(roots[1], env) as session:
            session.send("test.freshness.execute", "sync", {"rates": False, "journals": False})
            assert session.until("sync", "test.freshness.execute")["data"]["completed"][0]["status"] == "done"
            with connect(books[1]) as conn:
                conn.execute("UPDATE transactions SET raw_json=json_object('txid',external_id)")
            chain.hits = hits = BlockGraph("esplora")
            try:
                session.send("test.freshness.graph.start", "start")
                assert session.until("start")["kind"] == "test.freshness.graph.start"
                assert hits.started.wait(30)
                session.send("test.freshness.request.cancel", "foreign", {"target_request_id": "another-request"})
                assert session.until("foreign")["data"] == {"cancelled": True, "queued": True}
                hits.release.set()
                while not any(row["kind"] == "ui.freshness.background" for row in session.rows):
                    session.read()
                event = next(row for row in session.rows if row["kind"] == "ui.freshness.background")
                assert event["data"]["completed"][0]["status"] == "done"
                count = len(hits)
                session.send("test.freshness.execute", "sync", {"rates": False, "journals": False})
                assert session.until("sync")["error"]["code"] == "interaction_required"
                session.finish()
                assert len(hits) == count
                with connect(books[1]) as conn:
                    assert conn.execute("SELECT count(*) FROM freshness_jobs WHERE job_type='onchain_wallet_history'").fetchone()[0] == 1
                    assert conn.execute("SELECT count(*) FROM freshness_jobs WHERE job_type='wallet_graph_references'").fetchone()[0] == 1
            finally:
                hits.release.set()


@pytest.mark.parametrize("failure", ["open", "run"])
def test_native_graph_worker_failure_events_are_redacted(tmp_path, failure):
    config, chain, _ = _fixture("empty")
    roots, books = _books(tmp_path, config)
    with serve(chain, "esplora") as url:
        env = _environment(tmp_path, "esplora", url)
        with Session(roots[1], env) as session:
            session.send("test.freshness.execute", "sync", {"rates": False, "journals": False})
            assert session.until("sync", "test.freshness.execute")["data"]["completed"][0]["status"] == "done"
            chain.hits.clear()
            displaced = books[1].with_suffix(".displaced")
            if failure == "open":
                books[1].rename(displaced)
                books[1].mkdir()
            try:
                session.send("test.freshness.graph.start", "start")
                assert session.until("start")["kind"] == "test.freshness.graph.start"
                while not any(row["kind"] == "ui.freshness.worker" for row in session.rows):
                    session.read()
                event = next(row for row in session.rows if row["kind"] == "ui.freshness.worker")
                assert event["event"] is True and "request_id" not in event
                assert event["data"] == {
                    "status": "error",
                    "code": "freshness_background_open_failed" if failure == "open" else "freshness_background_error",
                    "message": "Graph worker could not open its book" if failure == "open" else "Graph reference worker failed",
                    "retryable": True,
                }
                assert str(books[1]) not in json.dumps(event) and url not in json.dumps(event)
                assert not any(row["kind"] == "ui.freshness.background" for row in session.rows)
                assert chain.hits == []
            finally:
                if failure == "open":
                    books[1].rmdir()
                    displaced.rename(books[1])
            session.finish()


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_native_graph_closed_output_joins_worker_before_exit(tmp_path, protocol):
    config, chain, _ = _fixture("receive-change")
    roots, books = _books(tmp_path, config)
    with serve(chain, protocol) as url:
        env = _environment(tmp_path, protocol, url)
        env.pop("KASSIBER_RUST_TEST_SYNC_PREFIX", None)
        with Session(roots[1], env) as session:
            session.send("test.freshness.execute", "sync", {"rates": False, "journals": False})
            assert session.until("sync", "test.freshness.execute")["data"]["completed"][0]["status"] == "done"
            with connect(books[1]) as conn:
                conn.execute("UPDATE transactions SET raw_json=json_object('txid',external_id)")
            chain.hits = hits = BlockGraph(protocol)
            try:
                session.send("test.freshness.graph.start", "start")
                assert session.until("start")["kind"] == "test.freshness.graph.start"
                assert hits.started.wait(30)
                session.process.stdout.close()
                hits.release.set()
                session.process.wait(timeout=30)
                assert session.process.returncode != 0
                assert session.process.stderr.read() == b""
                count = len(hits)
                with connect(books[1]) as conn:
                    assert conn.execute("SELECT count(*) FROM freshness_jobs WHERE status='running'").fetchone()[0] == 0
                assert len(hits) == count
            finally:
                hits.release.set()


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("stop", [False, True, "rotate"], ids=["complete", "lock", "rotate"])
def test_native_graph_encrypted_worker_handoff_and_cleanup(tmp_path, protocol, stop):
    from kassiber.secrets.migration import migrate_plaintext_to_encrypted
    from kassiber.secrets.sqlcipher import open_encrypted

    key = "Synthetic graph worker 🔑 passphrase"
    config, chain, _ = _fixture("receive-change")
    roots, books = _books(tmp_path, config)
    migrate_plaintext_to_encrypted(books[1], key)
    with serve(chain, protocol) as url:
        env = _environment(tmp_path, protocol, url)
        env.pop("KASSIBER_RUST_TEST_SYNC_PREFIX", None)
        if stop == "rotate":
            env["KASSIBER_RUST_TEST_GRAPH_STOP_WITNESS"] = "1"
        with Session(roots[1], env) as session:
            session.send("daemon.unlock", "unlock", {"auth_response": {"passphrase_secret": key}})
            assert session.until("unlock")["kind"] == "daemon.unlock"
            session.send("test.freshness.execute", "sync", {"rates": False, "journals": False})
            assert session.until("sync", "test.freshness.execute")["data"]["completed"][0]["status"] == "done"
            conn = open_encrypted(books[1], key)
            try:
                conn.execute("UPDATE transactions SET raw_json=json_object('txid',external_id)")
                conn.commit()
            finally:
                conn.close()
            chain.hits = hits = BlockGraph(protocol)
            try:
                session.send("test.freshness.graph.start", "start")
                assert session.until("start")["kind"] == "test.freshness.graph.start"
                assert hits.started.wait(30)
                if stop:
                    if stop == "rotate":
                        session.send("ui.secrets.change_passphrase", "lock", {
                            "auth_response": {"passphrase_secret": key},
                            "new_passphrase_secret": "Synthetic replacement 🔑 passphrase"})
                    else:
                        session.send("daemon.lock", "lock")
                    if stop == "rotate":
                        while not any(row["kind"] == "test.freshness.graph.stopping" for row in session.rows):
                            assert session.read().get("request_id") != "lock"
                    while b"\n" in session.buffer or select.select([session.process.stdout], [], [], 0.1)[0]:
                        assert session.read().get("request_id") != "lock"
                hits.release.set()
                if stop:
                    assert session.until("lock")["kind"] == ("ui.secrets.change_passphrase" if stop == "rotate" else "daemon.lock")
                while not any(r["kind"] == "ui.freshness.background" for r in session.rows):
                    session.read()
                event = next(r for r in session.rows if r["kind"] == "ui.freshness.background")
                assert event["data"]["completed"][0]["status"] == ("cancelled" if stop else "done")
                count = len(hits)
                if not stop:
                    session.send("daemon.lock", "lock")
                    assert session.until("lock")["kind"] == "daemon.lock"
                conn = open_encrypted(books[1], "Synthetic replacement 🔑 passphrase" if stop == "rotate" else key)
                try:
                    cached = conn.execute("SELECT count(*) FROM transaction_graph_cache").fetchone()[0]
                    assert cached == 0 if stop else cached > 0
                finally:
                    conn.close()
                session.send("test.freshness.graph.start", "locked-start")
                assert session.until("locked-start")["kind"] == "test.freshness.graph.start"
                session.finish()
                assert len(hits) == count
                assert key not in json.dumps(session.rows, ensure_ascii=False)
                assert "Synthetic replacement 🔑 passphrase" not in json.dumps(session.rows, ensure_ascii=False)
            finally:
                hits.release.set()
