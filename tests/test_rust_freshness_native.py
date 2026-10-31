import copy
from contextlib import ExitStack
import json
import os
from pathlib import Path
import subprocess
from types import SimpleNamespace
from unittest.mock import patch

import pytest

from kassiber import __version__, daemon_freshness
from kassiber.core import sync as core_sync
from kassiber.envelope import build_envelope
from tests.rust_sync_chain import serve
from tests.rust_sync_env import child_env
from tests.rust_sync_parity import (
    CLOCK_MODULES, connect, copy_book, create_book, deterministic_sources, dump_bytes,
    runtime, seed_reviewed_state,
)
from tests.test_rust_bdk_execution import bdk_request_clock
from tests.test_rust_wallet_publication import _fixture

FIXTURE = os.environ.get("KASSIBER_RUST_FRESHNESS_SYNC_FIXTURE")
if FIXTURE is None and os.environ.get("KASSIBER_RUST_BIN"):
    FIXTURE = str(Path(os.environ["KASSIBER_RUST_BIN"]).resolve().parent / "examples/freshness_sync_fixture")
pytestmark = pytest.mark.skipif(not FIXTURE, reason="requires the native freshness sync fixture")

CONTEXT_SQL = "INSERT INTO settings VALUES('context_workspace','w'); INSERT INTO settings VALUES('context_profile','p');"


def invoke(tmp_path, payload, **extra_env):
    result = subprocess.run([str(Path(FIXTURE).resolve())], input=json.dumps(payload).encode(),
        capture_output=True, timeout=90, env={**child_env(tmp_path), **extra_env})
    assert result.returncode == 0 and result.stderr == b"", result.stderr.decode()
    return json.loads(result.stdout)


def round_trip(tmp_path, chain, protocol, url, python_book, rust_book, index, workspace, force=False):
    initial_chain = copy.deepcopy({key: value for key, value in chain.__dict__.items() if key not in {"lock", "hits"}})
    rt = runtime(protocol, url)
    args = {"rates": False, "journals": False}
    if workspace:
        args["workspace_id"] = "w"
    else:
        args["force_full"] = force
    route = daemon_freshness._workspace_freshness_run_payload if workspace else daemon_freshness._freshness_run_payload
    conn = connect(python_book)
    try:
        with deterministic_sources(f"r{index}"), bdk_request_clock(1_700_000_000), ExitStack() as stack:
            for module in (*CLOCK_MODULES, daemon_freshness):
                stack.enter_context(patch.object(module, "now_iso", return_value="2030-01-01T00:00:01Z"))
            stack.enter_context(patch.object(core_sync, "time", SimpleNamespace(monotonic=lambda: 100.0)))
            expected = build_envelope("freshness.sync", route(conn, rt, args))
    finally:
        conn.close()
    with chain.lock:
        chain.__dict__.update(initial_chain)
    payload = {"book": str(rust_book), "backend": rt["backends"]["fixture"], "prefix": f"r{index}",
               "version": __version__, "force_full": force}
    if workspace:
        payload["workspace_id"] = "w"
    actual = invoke(tmp_path, payload)
    assert json.dumps(actual["envelope"], sort_keys=True) == json.dumps(expected, sort_keys=True)
    assert dump_bytes(rust_book) == dump_bytes(python_book)
    return actual


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("workspace", [False, True])
@pytest.mark.parametrize("fixture", ["empty", "receive-change", "checkpoint", "mempool"])
def test_native_freshness_full_book_execution_matches_python(tmp_path, protocol, workspace, fixture):
    config, chain, rounds = _fixture(fixture)
    python_book, rust_book = tmp_path / "python.sqlite", tmp_path / "rust.sqlite"
    sql = CONTEXT_SQL
    if workspace:
        sql += "INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('q','w','Second','EUR','later');"
    create_book(python_book, config, extra_sql=sql)
    copy_book(python_book, rust_book)
    with serve(chain, protocol) as url:
        for index, mutate in enumerate(rounds):
            if mutate:
                mutate()
            round_trip(tmp_path, chain, protocol, url, python_book, rust_book, index, workspace)


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_native_freshness_forced_reviewed_restart_matches_python(tmp_path, protocol):
    config, chain, rounds = _fixture("checkpoint")
    python_book, rust_book = tmp_path / "python.sqlite", tmp_path / "rust.sqlite"
    create_book(python_book, config, extra_sql=CONTEXT_SQL)
    copy_book(python_book, rust_book)
    with serve(chain, protocol) as url:
        for index, mutate in enumerate(rounds):
            if mutate:
                mutate()
            if index:
                for book in (python_book, rust_book):
                    with connect(book) as conn:
                        seed_reviewed_state(conn)
            round_trip(tmp_path, chain, protocol, url, python_book, rust_book, index, False, force=bool(index))


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("fence", ["missing", "revoked", "foreign", "cancelled", "offline", "no-egress", "enqueue-only"])
def test_native_freshness_pretransport_fences(tmp_path, protocol, fence):
    config, _, _ = _fixture("empty")
    book = tmp_path / "book.sqlite"
    create_book(book, config, extra_sql=CONTEXT_SQL)
    before = dump_bytes(book)
    url = "tcp://127.0.0.1:1" if protocol == "electrum" else "http://127.0.0.1:1"
    actual = invoke(tmp_path, {"book": str(book), "backend": runtime(protocol, url)["backends"]["fixture"],
        "version": __version__, "consent": fence != "missing", "revoke": fence == "revoked",
        "cancel": fence == "cancelled", "foreign": fence == "foreign", "offline": fence == "offline", "run": fence != "enqueue-only"},
        **({"KASSIBER_NO_EGRESS": "1"} if fence == "no-egress" else {}))
    envelope = actual["envelope"]
    if fence in {"missing", "revoked", "foreign", "cancelled"}:
        assert envelope["error"]["code"] == ("cancelled" if fence == "cancelled" else "interaction_required")
        assert dump_bytes(book) == before
    else:
        assert envelope["kind"] == "freshness.sync"
        if fence == "enqueue-only":
            assert envelope["data"]["completed"] == []
        else:
            assert envelope["data"]["completed"][0]["status"] == "error"
            assert envelope["data"]["completed"][0]["error"]["code"] == "network_egress_disabled"


@pytest.mark.parametrize("change", ["book", "config", "backend"])
def test_native_freshness_revalidates_before_queue_and_transport(tmp_path, change):
    config, _, _ = _fixture("empty")
    book = tmp_path / "book.sqlite"
    create_book(book, config, extra_sql=CONTEXT_SQL)
    backend = runtime("electrum", "tcp://127.0.0.1:1")["backends"]["fixture"]
    payload = {"book": str(book), "version": __version__, "backend": backend}
    if change == "backend":
        payload["changed_backend"] = {**backend, "url": "tcp://127.0.0.1:2"}
    else:
        payload["before_execution_sql"] = ("UPDATE settings SET value='' WHERE key='context_profile'" if change == "book"
                                           else "UPDATE wallets SET config_json='{}'")
    result = invoke(tmp_path, payload)
    assert result["envelope"]["error"]["code"] == "stale_context"
    with connect(book) as conn:
        assert conn.execute("SELECT count(*) FROM freshness_jobs").fetchone()[0] == 0
        assert conn.execute("SELECT count(*) FROM freshness_source_states").fetchone()[0] == 0


@pytest.mark.skipif(not os.environ.get("KASSIBER_RUST_BIN"), reason="requires the guarded Rust daemon")
def test_native_freshness_admitted_book_execution_is_inert_without_run(tmp_path):
    from tests.test_rust_freshness_admission import _book
    _book(tmp_path)
    owners = tmp_path / "owners"
    owners.mkdir(mode=0o700)
    env = {**child_env(tmp_path), "KASSIBER_RUST_TEST_OWNER_ROOT": str(owners),
        "KASSIBER_BACKEND_FIXTURE_KIND": "esplora", "KASSIBER_BACKEND_FIXTURE_URL": "http://127.0.0.1:1",
        "KASSIBER_DEFAULT_BACKEND": "fixture", "KASSIBER_TEST_NO_EGRESS": "1", "KASSIBER_NO_EGRESS": "1"}
    requests = [{"request_id": "execute", "kind": "test.freshness.execute",
                 "args": {"rates": False, "journals": False, "run": False}},
                {"request_id": "stop", "kind": "daemon.shutdown"}]
    result = subprocess.run([str(Path(os.environ["KASSIBER_RUST_BIN"]).resolve()), "--data-root", str(tmp_path), "daemon"],
        input="".join(json.dumps(row) + "\n" for row in requests), text=True, capture_output=True, timeout=30, env=env)
    assert result.returncode == 0 and result.stderr == "", result.stderr
    rows = [json.loads(row) for row in result.stdout.splitlines()]
    assert [row["kind"] for row in rows] == ["daemon.ready", "test.freshness.execute", "daemon.shutdown"]
    assert len(rows[1]["data"]["enqueued"]) == 1
    assert rows[1]["data"]["completed"] == []
    assert "ui.freshness.run" not in rows[0]["data"]["supported_kinds"]
