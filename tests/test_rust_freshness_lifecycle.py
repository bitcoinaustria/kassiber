from contextlib import contextmanager
import json
import os
import select
import threading

import pytest

from tests.rust_freshness_session import Session
from tests.rust_sync_chain import serve
from tests.rust_sync_parity import connect, dump_bytes
from tests.test_rust_wallet_publication import _fixture
from tests.test_rust_wallet_sync_cli import _books, _environment


pytestmark = pytest.mark.skipif(not os.environ.get("KASSIBER_RUST_BIN"), reason="requires the guarded Rust daemon")
ARGS = {"rates": False, "journals": False}


class BlockedHits(list):
    def __init__(self, protocol):
        super().__init__()
        self.protocol = protocol
        self.started = threading.Event()
        self.release = threading.Event()
        self.blocked_at = None

    def append(self, request):
        super().append(request)
        fetching = (request.get("method") == "blockchain.scripthash.get_history"
                    if isinstance(request, dict) else request.startswith("/scripthash/"))
        if fetching and self.blocked_at is None:
            self.blocked_at = len(self)
            self.started.set()
            assert self.release.wait(30)


@contextmanager
def running(tmp_path, protocol):
    config, chain, _ = _fixture("receive-change")
    roots, books = _books(tmp_path, config)
    chain.hits = hits = BlockedHits(protocol)
    with serve(chain, protocol) as url, Session(roots[1], _environment(tmp_path, protocol, url)) as session:
        session.send("test.freshness.execute", "sync", ARGS)
        assert hits.started.wait(30)
        try:
            yield session, hits, books[1]
        finally:
            hits.release.set()


def completed(response):
    assert response["kind"] == "test.freshness.execute", response
    jobs = response["data"]["completed"]
    assert len(jobs) == 1
    return jobs[0]


def unpublished(book):
    with connect(book) as conn:
        for table in ("transactions", "chain_observer_instances", "wallet_utxo_refreshes"):
            assert conn.execute(f"SELECT count(*) FROM {table}").fetchone()[0] == 0


def graph_pending(session, request_id):
    session.send("test.freshness.graph.pending", request_id)
    response = session.until(request_id)
    assert response["kind"] == "test.freshness.graph.pending", response
    return response["data"]["grants"]


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("control", ["cancel", "revoke", "lock", "switch"])
def test_live_freshness_private_graph_handoff_and_discard(tmp_path, protocol, control):
    with running(tmp_path, protocol) as (session, hits, book):
        assert graph_pending(session, "before") == 0
        hits.release.set()
        result = session.until("sync", "test.freshness.execute")
        assert completed(result)["status"] == "done"
        assert graph_pending(session, "after") == 1
        before, count = dump_bytes(book), len(hits)
        session.send("status", "idle")
        assert session.until("idle")["kind"] == "status"
        assert graph_pending(session, "idle-graph") == 1
        assert dump_bytes(book) == before and len(hits) == count
        assert "graph_grants" not in json.dumps(session.rows)
        with connect(book) as conn:
            assert conn.execute("SELECT count(*) FROM freshness_jobs WHERE job_type='wallet_graph_references'").fetchone()[0] == 0
            if control == "switch":
                conn.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('q','w','Second','EUR','later')")
        if control in {"cancel", "revoke"}:
            session.send(f"test.freshness.request.{control}", "discard", {"target_request_id": "sync"})
            assert session.until("discard")["data"] == {"cancelled": True, "queued": False}
        else:
            session.send("daemon.lock" if control == "lock" else "ui.profiles.switch", "discard",
                         {"profile_id": "q"} if control == "switch" else None)
            assert session.until("discard")["kind"] != "error"
        assert graph_pending(session, "discarded") == 0
        assert len(hits) == count
        session.finish()


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_live_freshness_graph_handoff_deduplicates_and_never_recovers_authority(tmp_path, protocol):
    config, chain, _ = _fixture("receive-change")
    roots, books = _books(tmp_path, config)
    with serve(chain, protocol) as url:
        env = _environment(tmp_path, protocol, url)
        env.pop("KASSIBER_RUST_TEST_SYNC_PREFIX", None)
        with Session(roots[1], env) as session:
            assert graph_pending(session, "launch") == 0
            for request_id in ("first", "second"):
                session.send("test.freshness.execute", request_id, ARGS)
                assert completed(session.until(request_id, "test.freshness.execute"))["status"] == "done"
                assert graph_pending(session, request_id + "-graph") == 1
            session.send("test.freshness.request.cancel", "old", {"target_request_id": "first"})
            assert session.until("old")["data"] == {"cancelled": False, "queued": False}
            assert graph_pending(session, "current") == 1
            session.finish()
        with connect(books[1]) as conn:
            conn.execute("INSERT INTO freshness_jobs(id,profile_id,job_type,source_key,source_type,source_label,status,phase,priority,created_at,updated_at) VALUES('persisted-graph','p','wallet_graph_references','wallet_graph:a','onchain_wallet','Graph','running','backend_fetch',200,'now','now')")
        count = len(chain.hits)
        with Session(roots[1], env) as session:
            session.send("status", "read")
            assert session.until("read")["kind"] == "status"
            assert graph_pending(session, "restart") == 0
            session.send("test.freshness.execute", "enqueue", {**ARGS, "run": False})
            assert session.until("enqueue", "test.freshness.execute")["data"]["completed"] == []
            assert graph_pending(session, "no-run") == 0
            assert len(chain.hits) == count
            with connect(books[1]) as conn:
                assert conn.execute("SELECT status FROM freshness_jobs WHERE id='persisted-graph'").fetchone()[0] == "running"
            session.finish()


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_live_freshness_encrypted_graph_handoff_drops_on_lock(tmp_path, protocol):
    from kassiber.secrets.migration import migrate_plaintext_to_encrypted
    from kassiber.secrets.sqlcipher import open_encrypted

    key = "Synthetic graph handoff 🔑 passphrase"
    config, chain, _ = _fixture("receive-change")
    roots, books = _books(tmp_path, config)
    migrate_plaintext_to_encrypted(books[1], key)
    with serve(chain, protocol) as url, Session(roots[1], _environment(tmp_path, protocol, url)) as session:
        session.send("daemon.unlock", "unlock", {"auth_response": {"passphrase_secret": key}})
        assert session.until("unlock")["kind"] == "daemon.unlock"
        session.send("test.freshness.execute", "sync", ARGS)
        assert completed(session.until("sync", "test.freshness.execute"))["status"] == "done"
        assert graph_pending(session, "after") == 1
        count = len(chain.hits)
        session.send("daemon.lock", "lock")
        assert session.until("lock")["kind"] == "daemon.lock"
        assert graph_pending(session, "locked") == 0
        session.send("test.freshness.execute", "locked-sync", ARGS)
        assert session.until("locked-sync")["kind"] == "auth_required"
        assert len(chain.hits) == count
        conn = open_encrypted(books[1], key)
        try:
            assert conn.execute("SELECT count(*) FROM freshness_jobs WHERE job_type='wallet_graph_references'").fetchone()[0] == 0
        finally:
            conn.close()
        assert key not in json.dumps(session.rows, ensure_ascii=False)
        session.finish()


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("control", ["cancel", "revoke", "offline", "source-cancel", "pause"])
def test_live_freshness_controls_remain_available_during_fetch(tmp_path, protocol, control):
    with running(tmp_path, protocol) as (session, hits, book):
        session.send("status", "read")
        assert session.until("read")["kind"] == "status"
        session.send("test.freshness.execute", "competing", ARGS)
        assert session.until("competing")["error"]["code"] == "project_operation_in_progress"
        with connect(book) as conn:
            job = conn.execute("SELECT id,source_key FROM freshness_jobs").fetchone()
        if control in {"cancel", "revoke"}:
            session.send(f"test.freshness.request.{control}", "control", {"target_request_id": "sync"})
        elif control == "offline":
            session.send("ui.network.offline.set", "control", {"enabled": True})
        elif control == "source-cancel":
            session.send("test.freshness.cancel", "control", {"job_id": job[0]})
        else:
            session.send("test.freshness.pause", "control", {"source_key": job[1]})
        response = session.until("control")
        assert response["kind"] != "error", response
        if control in {"cancel", "revoke"}:
            assert response["data"] == {"cancelled": True, "queued": False}
        hits.release.set()
        response = session.until("sync", "test.freshness.execute")
        if control == "pause":
            assert response["data"]["completed"] == []
            session.send("test.freshness.resume", "resume", {"source_key": job[1]})
            assert session.until("resume")["data"]["result"]["paused"] is False
        else:
            job = completed(response)
            assert job["status"] == ("cancelled" if control in {"cancel", "source-cancel"} else "error")
            if control in {"revoke", "offline"}:
                assert job["error"]["code"] == ("interaction_required" if control == "revoke" else "network_egress_disabled")
            assert len(hits) <= hits.blocked_at + 1
        unpublished(book)
        assert graph_pending(session, "fenced-graph") == 0
        session.finish()


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("control,args", [
    ("daemon.lock", None), ("daemon.shutdown", None),
    ("ui.profiles.switch", {"profile_id": "q"}), ("ui.profiles.switch", {"profile_id": "r", "workspace_id": "v"}),
])
def test_live_freshness_quiesces_before_lifecycle_acknowledgement(tmp_path, protocol, control, args):
    with running(tmp_path, protocol) as (session, hits, book):
        with connect(book) as conn:
            conn.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('q','w','Second','EUR','later')")
            conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('v','Other','later')")
            conn.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('r','v','Other','EUR','later')")
        session.send(control, "lifecycle", args)
        session.send("status", "barrier")
        while b"\n" in session.buffer or select.select([session.process.stdout], [], [], 0.1)[0]:
            assert session.read().get("request_id") != "lifecycle"
        hits.release.set()
        response = session.until("lifecycle")
        assert response["kind"] == control, response
        sync = [row for row in session.rows if row.get("request_id") == "sync" and row["kind"] == "test.freshness.execute"]
        assert len(sync) == 1
        assert completed(sync[0])["status"] == "cancelled"
        count = len(hits)
        assert count <= hits.blocked_at + 1
        unpublished(book)
        if control == "daemon.shutdown":
            session.process.wait(timeout=30)
            assert session.process.returncode == 0
        else:
            assert session.until("barrier")["kind"] == "status"
            assert len(hits) == count
            session.finish()


@pytest.mark.parametrize("fence", ["early-cancel", "missing", "revoked", "offline", "no-egress", "stale-scope"])
def test_live_freshness_preconsent_and_early_request_fences(tmp_path, fence):
    config, chain, _ = _fixture("empty")
    roots, books = _books(tmp_path, config)
    with serve(chain, "esplora") as url:
        env = _environment(tmp_path, "esplora", url)
        if fence == "no-egress":
            env["KASSIBER_NO_EGRESS"] = "1"
        with Session(roots[1], env) as session:
            session.send("status", "read")
            assert session.until("read")["kind"] == "status"
            before = dump_bytes(books[1])
            if fence == "early-cancel":
                session.send("test.freshness.request.cancel", "cancel", {"target_request_id": "sync"})
                assert session.until("cancel")["data"] == {"cancelled": True, "queued": True}
            if fence == "offline":
                session.send("ui.network.offline.set", "offline", {"enabled": True})
                session.until("offline")
            args = {**ARGS, **({"consent": False} if fence == "missing" else {}),
                    **({"revoke": True} if fence == "revoked" else {}),
                    **({"expected_scope": {"workspace_id": "w", "profile_id": "other"}} if fence == "stale-scope" else {})}
            session.send("test.freshness.execute", "sync", args)
            response = session.until("sync", "test.freshness.execute")
            if fence in {"early-cancel", "missing", "revoked", "stale-scope"}:
                assert response["error"]["code"] == {"early-cancel": "cancelled", "stale-scope": "stale_context"}.get(fence, "interaction_required")
                assert dump_bytes(books[1]) == before
            else:
                assert completed(response)["error"]["code"] == "network_egress_disabled"
            assert chain.hits == []
            assert graph_pending(session, "fenced-graph") == 0
            session.finish()


def test_live_freshness_refuses_replay_and_foreign_request_cancellation(tmp_path):
    with running(tmp_path, "esplora") as (session, hits, book):
        session.send("test.freshness.request.cancel", "stale", {"target_request_id": "sync",
            "expected_scope": {"workspace_id": "w", "profile_id": "other"}})
        assert session.until("stale")["error"]["code"] == "stale_context"
        session.send("test.freshness.request.cancel", "foreign", {"target_request_id": "other"})
        assert session.until("foreign")["data"] == {"cancelled": True, "queued": True}
        session.send("test.freshness.execute", "sync", ARGS)
        assert session.until("sync")["error"]["code"] == "interaction_required"
        hits.release.set()
        assert completed(session.until("sync", "test.freshness.execute"))["status"] == "done"
        before, count = dump_bytes(book), len(hits)
        session.send("test.freshness.execute", "sync", ARGS)
        assert session.until("sync")["error"]["code"] == "interaction_required"
        assert len(hits) == count and dump_bytes(book) == before
        session.finish()


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_live_freshness_encrypted_request_connection_and_lock_cleanup(tmp_path, protocol):
    from kassiber.secrets.migration import migrate_plaintext_to_encrypted
    from kassiber.secrets.sqlcipher import open_encrypted

    key = "Synthetic lifecycle 🔑 passphrase"
    config, chain, _ = _fixture("receive-change")
    roots, books = _books(tmp_path, config)
    migrate_plaintext_to_encrypted(books[1], key)
    chain.hits = hits = BlockedHits(protocol)
    with serve(chain, protocol) as url, Session(roots[1], _environment(tmp_path, protocol, url)) as session:
        session.send("test.freshness.execute", "locked", ARGS)
        assert session.until("locked")["kind"] == "auth_required"
        assert hits == []
        session.send("daemon.unlock", "unlock", {"auth_response": {"passphrase_secret": key}})
        assert session.until("unlock")["kind"] == "daemon.unlock"
        session.send("test.freshness.execute", "sync", ARGS)
        assert hits.started.wait(30)
        try:
            session.send("daemon.lock", "lock")
            while b"\n" in session.buffer or select.select([session.process.stdout], [], [], 0.1)[0]:
                assert session.read().get("request_id") != "lock"
            hits.release.set()
            assert session.until("lock")["kind"] == "daemon.lock"
            sync = next(row for row in session.rows if row["kind"] == "test.freshness.execute")
            assert completed(sync)["status"] == "cancelled"
            session.send("test.freshness.execute", "locked-again", ARGS)
            assert session.until("locked-again")["kind"] == "auth_required"
            conn = open_encrypted(books[1], key)
            try:
                assert conn.execute("SELECT count(*) FROM transactions").fetchone()[0] == 0
                assert conn.execute("SELECT count(*) FROM freshness_jobs WHERE status='cancelled'").fetchone()[0] == 1
            finally:
                conn.close()
            assert key not in json.dumps(session.rows, ensure_ascii=False)
            session.finish()
        finally:
            hits.release.set()


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_live_freshness_eof_joins_the_request_before_exit(tmp_path, protocol):
    with running(tmp_path, protocol) as (session, hits, book):
        session.process.stdin.close()
        while b"\n" in session.buffer or select.select([session.process.stdout], [], [], 0.1)[0]:
            row = session.read()
            assert row["kind"] == "test.freshness.execute.progress"
        hits.release.set()
        assert completed(session.until("sync", "test.freshness.execute"))["status"] == "cancelled"
        session.process.wait(timeout=30)
        assert session.process.returncode == 0
        assert len(hits) <= hits.blocked_at + 1
        unpublished(book)


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_live_freshness_closed_output_stops_and_joins_the_request(tmp_path, protocol):
    with running(tmp_path, protocol) as (session, hits, book):
        session.process.stdout.close()
        hits.release.set()
        session.process.wait(timeout=30)
        assert session.process.returncode == 1
        unpublished(book)


@pytest.mark.parametrize("action", ["cancel", "pause", "resume"])
@pytest.mark.parametrize("source", ["wallet", "graph"])
def test_reference_source_control_checkpoint_repro_and_native_parity(tmp_path, action, source):
    from kassiber import daemon_freshness
    from kassiber.envelope import build_envelope
    from tests.rust_sync_parity import deterministic_sources

    config, _, _ = _fixture("empty")
    roots, books = _books(tmp_path, config)
    with Session(roots[1], _environment(tmp_path)) as session:
        session.send("test.freshness.execute", "enqueue", {**ARGS, "run": False})
        session.until("enqueue", "test.freshness.execute")
        reference = tmp_path / "reference.sqlite"
        checkpoint = {"script_membership": {"synthetic": ["private-script-member"]}}
        with connect(books[1]) as current, connect(reference) as snapshot:
            if source == "graph":
                current.execute("UPDATE freshness_jobs SET source_key='wallet_graph:a',job_type='wallet_graph_references'")
                current.execute("UPDATE freshness_source_states SET source_key='wallet_graph:a'")
            current.execute("UPDATE freshness_source_states SET checkpoint_json=?", (json.dumps(checkpoint),))
            current.commit()
            current.backup(snapshot)
        with connect(reference) as conn:
            job = conn.execute("SELECT id,source_key FROM freshness_jobs").fetchone()
            args = {"job_id": job[0]} if action == "cancel" else {"source_key": job[1]}
            with deterministic_sources("r0"):
                expected = daemon_freshness._freshness_control_payload(conn, args, action=action)
            assert "private-script-member" not in json.dumps(expected)
        kind = f"test.freshness.{action}"
        session.send(kind, "control", args)
        actual = session.until("control")
        assert "private-script-member" not in json.dumps(actual)
        assert json.dumps(actual, sort_keys=True) == json.dumps(
            {**build_envelope(kind, expected), "request_id": "control"}, sort_keys=True)
        assert dump_bytes(books[1]) == dump_bytes(reference)
        session.finish()
