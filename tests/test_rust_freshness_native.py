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
    CLOCK_MODULES, connect, copy_book, create_book, deterministic_sources, dump_bytes, fixed_elapsed_ms,
    runtime, seed_reviewed_state,
)
from tests.test_rust_bdk_execution import bdk_request_clock
from tests.test_rust_wallet_publication import _fixture, _wallet
from tests.rust_fixture_binary import checked

FIXTURE = os.environ.get("KASSIBER_RUST_FRESHNESS_SYNC_FIXTURE")
if FIXTURE is None and os.environ.get("KASSIBER_RUST_BIN"):
    FIXTURE = str(checked(Path(os.environ["KASSIBER_RUST_BIN"]).resolve().parent / "examples/freshness_sync_fixture",
                          "cargo build -p kassiber-wallets --example freshness_sync_fixture --features test-harness --offline --locked"))
pytestmark = pytest.mark.skipif(not FIXTURE, reason="requires the native freshness sync fixture")

CONTEXT_SQL = "INSERT INTO settings VALUES('context_workspace','w'); INSERT INTO settings VALUES('context_profile','p');"


def invoke(tmp_path, payload, **extra_env):
    # The debug fixture's widened-gap rescan alone takes ~40 s of CPU; under
    # load its default 60 s request budget runs out and the job ends in
    # `error`. Give it the budget its Rust replays use.
    budget = {"KASSIBER_FRESHNESS_FIXTURE_BUDGET_SECS": "900"}
    result = subprocess.run([str(Path(FIXTURE).resolve())], input=json.dumps(payload).encode(),
        capture_output=True, timeout=900, env={**child_env(tmp_path), **budget, **extra_env})
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
    events = []
    try:
        with deterministic_sources(f"r{index}"), bdk_request_clock(1_700_000_000), ExitStack() as stack:
            for module in (*CLOCK_MODULES, daemon_freshness):
                stack.enter_context(patch.object(module, "now_iso", return_value="2030-01-01T00:00:01Z"))
            stack.enter_context(patch.object(core_sync, "time", SimpleNamespace(monotonic=lambda: 100.0)))
            # Python discovers and fetches a batch's wallets in threads; one
            # worker keeps its progress in wallet order, as Rust's is.
            parallel = core_sync.checked_parallel_map
            stack.enter_context(patch.object(core_sync, "checked_parallel_map",
                                             lambda step, items, check, _workers: parallel(step, items, check, 1)))
            expected = build_envelope("freshness.sync", route(conn, rt, args, progress_observer=events.append))
    finally:
        conn.close()
    with chain.lock:
        chain.__dict__.update(initial_chain)
    payload = {"book": str(rust_book), "backend": rt["backends"]["fixture"], "prefix": f"r{index}",
               "version": __version__, "force_full": force}
    if workspace:
        payload["workspace_id"] = "w"
    actual = invoke(tmp_path, payload)
    (tmp_path / "events.json").write_text(json.dumps({"actual": actual["events"], "expected": events}, indent=2))
    assert json.dumps(fixed_elapsed_ms(actual["envelope"]), sort_keys=True) == json.dumps(fixed_elapsed_ms(expected), sort_keys=True)
    assert json.dumps(fixed_elapsed_ms(actual["events"]), sort_keys=True) == json.dumps(fixed_elapsed_ms(events), sort_keys=True)
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


@pytest.mark.parametrize("workspace", [False, True])
@pytest.mark.parametrize("duplicate", [False, True], ids=["distinct", "duplicate"])
def test_native_freshness_several_wallets_match_python(tmp_path, workspace, duplicate):
    # Electrum only: the base's Esplora acquisition lacks Python's history
    # anchors (docs/reference/rust-native-wallet-gaps.md).
    config, chain, _ = _fixture("receive-change")
    other = dict(config) if duplicate else _wallet("tr", 3)[0]
    python_book, rust_book = tmp_path / "python.sqlite", tmp_path / "rust.sqlite"
    sql = CONTEXT_SQL + ("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                         f"VALUES('b','w','p','Hot','descriptor','{json.dumps(other)}','fixed');")
    create_book(python_book, config, extra_sql=sql)
    copy_book(python_book, rust_book)
    with serve(chain, "electrum") as url:
        for index in range(2):
            round_trip(tmp_path, chain, "electrum", url, python_book, rust_book, index, workspace)


@pytest.mark.parametrize("workspace", [False, True])
def test_native_freshness_negative_balance_repair_matches_python(tmp_path, workspace):
    # An unfunded stored outbound makes Python refetch at the widened gap
    # before publishing (docs/reference/rust-native-wallet-gaps.md, R1).
    config, chain, _ = _fixture("receive-change")
    python_book, rust_book = tmp_path / "python.sqlite", tmp_path / "rust.sqlite"
    sql = CONTEXT_SQL + ("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,"
                         "occurred_at,direction,asset,amount,fee,raw_json,created_at) VALUES('manual','w','p','a',"
                         "'manual-1','f','2023-01-01T00:00:00Z','outbound','BTC',100000000000,0,'{}','fixed');")
    create_book(python_book, config, extra_sql=sql)
    copy_book(python_book, rust_book)
    with serve(chain, "electrum") as url:
        actual = round_trip(tmp_path, chain, "electrum", url, python_book, rust_book, 0, workspace)
    assert "negative_balance_rescan" in json.dumps(actual["envelope"])


def test_native_freshness_electrum_address_list_matches_python(tmp_path):
    # Python's compatibility Electrum observer (gap E1) under a freshness
    # job, its backend_fetch progress events included.
    from kassiber.wallet_descriptors import derive_descriptor_targets, load_descriptor_plan

    config, chain, _ = _fixture("receive-change")
    targets = derive_descriptor_targets(load_descriptor_plan(config), start=0, end=3)
    addresses = {"addresses": [t.address for t in targets], "chain": "bitcoin", "network": "main",
                 "backend": "fixture"}
    python_book, rust_book = tmp_path / "python.sqlite", tmp_path / "rust.sqlite"
    create_book(python_book, addresses, extra_sql=CONTEXT_SQL)
    copy_book(python_book, rust_book)
    with serve(chain, "electrum") as url:
        actual = round_trip(tmp_path, chain, "electrum", url, python_book, rust_book, 0, False)
    assert '"known_txids"' in json.dumps(actual["envelope"])


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_native_freshness_rescan_after_apply_matches_python(tmp_path, protocol):
    # An address list has no gap to widen, so the deficit an unfunded stored
    # outbound leaves after applying re-syncs the wallet inside the job's
    # publication savepoint (docs/reference/rust-native-wallet-gaps.md, C7).
    from kassiber.wallet_descriptors import derive_descriptor_targets, load_descriptor_plan

    config, chain, _ = _fixture("receive-change")
    target = derive_descriptor_targets(load_descriptor_plan(config), start=0, end=1)[0]
    addresses = {"addresses": [target.address], "chain": "bitcoin", "network": "main", "backend": "fixture"}
    python_book, rust_book = tmp_path / "python.sqlite", tmp_path / "rust.sqlite"
    sql = CONTEXT_SQL + ("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,"
                         "occurred_at,direction,asset,amount,fee,raw_json,created_at) VALUES('manual','w','p','a',"
                         "'manual-1','f','2023-01-01T00:00:00Z','outbound','BTC',100000000000,0,'{}','fixed');")
    create_book(python_book, addresses, extra_sql=sql)
    copy_book(python_book, rust_book)
    with serve(chain, protocol) as url:
        actual = round_trip(tmp_path, chain, protocol, url, python_book, rust_book, 0, False)
    assert '"rescan_gap_limit": null' in json.dumps(actual["envelope"])


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
    from tests.rust_freshness_session import Session
    with Session(tmp_path, env) as session:
        session.send("test.freshness.execute", "execute", {"rates": False, "journals": False, "run": False})
        session.until("execute", "test.freshness.execute")
        session.finish()
        rows = session.rows
    assert [row["kind"] for row in rows] == ["daemon.ready", "test.freshness.execute", "daemon.shutdown"]
    assert len(rows[1]["data"]["enqueued"]) == 1
    assert rows[1]["data"]["completed"] == []
    assert "ui.freshness.run" in rows[0]["data"]["supported_kinds"]


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("control", ["cancel", "book", "config", "backend"])
@pytest.mark.parametrize("workspace", [False, True])
def test_native_freshness_controls_between_fetch_batches(tmp_path, protocol, control, workspace):
    config, chain, _ = _fixture("empty")
    book = tmp_path / "book.sqlite"
    create_book(book, config, extra_sql=CONTEXT_SQL)

    class ControlledHits(list):
        changed_at = None

        def append(self, request):
            super().append(request)
            fetching = (request.get("method") == "blockchain.scripthash.get_history" if isinstance(request, dict)
                        else request.startswith("/scripthash/"))
            if fetching and self.changed_at is None:
                self.changed_at = len(self)
                with connect(book) as conn:
                    if control == "cancel":
                        conn.execute("UPDATE freshness_jobs SET cancel_requested=1 WHERE status IN ('queued','running')")
                    elif control == "book":
                        conn.execute("UPDATE settings SET value='' WHERE key='context_profile'")
                    elif control == "config":
                        conn.execute("UPDATE wallets SET config_json='{}'")
                    else:
                        conn.execute("UPDATE backends SET url='http://127.0.0.1:1' WHERE name='fixture'")

    chain.hits = hits = ControlledHits()
    with serve(chain, protocol) as url:
        if control == "backend":
            with connect(book) as conn:
                conn.execute("INSERT INTO backends(name,kind,url,batch_size,created_at,updated_at) VALUES('fixture',?,?,2,'fixed','fixed')",
                             (protocol, url))
        result = invoke(tmp_path, {"book": str(book), "backend": runtime(protocol, url)["backends"]["fixture"],
                                  "version": __version__, "saved_backend_url": control == "backend",
                                  **({"workspace_id": "w"} if workspace else {})})
    assert hits.changed_at is not None
    # The request already on the wire may finish, including its second batch
    # member. No later batch or publication may start.
    assert len(hits) <= hits.changed_at + 1
    data = result["envelope"]["data"]
    completed = (data["books"][0] if workspace else data)["completed"]
    assert len(completed) == 1
    assert completed[0]["status"] == ("cancelled" if control == "cancel" else "error")
    if control != "cancel":
        assert completed[0]["error"]["code"] == "stale_context"
    with connect(book) as conn:
        for table in ("transactions", "chain_observer_instances", "wallet_utxo_refreshes"):
            assert conn.execute(f"SELECT count(*) FROM {table}").fetchone()[0] == 0


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("control", ["revoke", "cancel", "offline", "closed"])
@pytest.mark.parametrize("phase", ["discovery", "decode_enrich"])
def test_native_freshness_progress_authority_fences(tmp_path, protocol, control, phase):
    config, chain, _ = _fixture("receive-change")
    book = tmp_path / "book.sqlite"
    create_book(book, config, extra_sql=CONTEXT_SQL)
    with serve(chain, protocol) as url:
        result = invoke(tmp_path, {"book": str(book), "backend": runtime(protocol, url)["backends"]["fixture"],
            "version": __version__, "at_phase": phase, "during_progress": control})
    assert any(event["phase"] == phase for event in result["events"])
    job = result["envelope"]["data"]["completed"][0]
    assert job["status"] == ("cancelled" if control in {"cancel", "closed"} else "error")
    if control in {"revoke", "offline"}:
        assert job["error"]["code"] == ("interaction_required" if control == "revoke" else "network_egress_disabled")
    if phase == "discovery":
        assert chain.hits == []
    with connect(book) as conn:
        assert conn.execute("SELECT count(*) FROM transactions").fetchone()[0] == 0
        assert conn.execute("SELECT count(*) FROM chain_observer_instances").fetchone()[0] == 0


@pytest.mark.skipif(not os.environ.get("KASSIBER_RUST_BIN"), reason="requires the guarded Rust daemon")
@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_native_freshness_live_stream_matches_python(tmp_path, protocol):
    from kassiber.core import freshness
    from tests.test_rust_wallet_sync_cli import _books, _environment

    config, chain, _ = _fixture("receive-change")
    roots, books = _books(tmp_path, config)
    before = copy.deepcopy({key: value for key, value in chain.__dict__.items() if key not in {"lock", "hits"}})
    expected_events = []
    with serve(chain, protocol) as url:
        conn = connect(books[0])
        try:
            with deterministic_sources("r0"), bdk_request_clock(1_700_000_000), \
                    patch.object(daemon_freshness, "now_iso", side_effect=lambda: freshness.now_iso()), \
                    patch.object(core_sync, "time", SimpleNamespace(monotonic=lambda: 100.0)):
                expected = daemon_freshness._freshness_run_payload(conn, runtime(protocol, url),
                    {"rates": False, "journals": False}, progress_observer=expected_events.append)
        finally:
            conn.close()
        with chain.lock:
            chain.__dict__.update(before)
        env = _environment(tmp_path, protocol, url)
        from tests.rust_freshness_session import Session
        with Session(roots[1], env) as session:
            session.send("test.freshness.execute", "sync", {"rates": False, "journals": False})
            session.until("sync", "test.freshness.execute")
            session.finish()
            rows = session.rows
    assert rows[0]["kind"] == "daemon.ready"
    assert rows[-1]["kind"] == "daemon.shutdown"
    assert fixed_elapsed_ms(rows[1:-2]) == fixed_elapsed_ms(
        [{**build_envelope("test.freshness.execute.progress", event), "request_id": "sync"} for event in expected_events])
    assert fixed_elapsed_ms(rows[-2]) == fixed_elapsed_ms(
        {**build_envelope("test.freshness.execute", expected), "request_id": "sync"})
    assert dump_bytes(books[1]) == dump_bytes(books[0])
    assert "ui.freshness.run" in rows[0]["data"]["supported_kinds"]


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_reference_prefetch_cancellation_repro_and_native_terminal_parity(tmp_path, protocol):
    """Both runtimes stop prefetch after persisted job cancellation."""
    config, chain, _ = _fixture("empty")
    python_book, rust_book = tmp_path / "python.sqlite", tmp_path / "rust.sqlite"
    create_book(python_book, config, extra_sql=CONTEXT_SQL)
    copy_book(python_book, rust_book)

    class CancelHits(list):
        cancelled_at = None

        def __init__(self, book):
            super().__init__()
            self.book = book

        def append(self, request):
            super().append(request)
            fetching = (request.get("method") == "blockchain.scripthash.get_history" if isinstance(request, dict)
                        else request.startswith("/scripthash/"))
            if fetching and self.cancelled_at is None:
                self.cancelled_at = len(self)
                with connect(self.book) as conn:
                    conn.execute("UPDATE freshness_jobs SET cancel_requested=1 WHERE status='queued'")

    with serve(chain, protocol) as url:
        chain.hits = reference_hits = CancelHits(python_book)
        conn = connect(python_book)
        try:
            with deterministic_sources("job"), bdk_request_clock(1_700_000_000), ExitStack() as stack:
                for module in (*CLOCK_MODULES, daemon_freshness):
                    stack.enter_context(patch.object(module, "now_iso", return_value="2030-01-01T00:00:01Z"))
                stack.enter_context(patch.object(core_sync, "time", SimpleNamespace(monotonic=lambda: 100.0)))
                expected = build_envelope("freshness.sync", daemon_freshness._freshness_run_payload(conn,
                    runtime(protocol, url), {"rates": False, "journals": False}))
        finally:
            conn.close()
        chain.hits = native_hits = CancelHits(rust_book)
        actual = invoke(tmp_path, {"book": str(rust_book), "backend": runtime(protocol, url)["backends"]["fixture"],
                                  "version": __version__})
    assert fixed_elapsed_ms(actual["envelope"]) == fixed_elapsed_ms(expected)
    assert dump_bytes(rust_book) == dump_bytes(python_book)
    assert expected["data"]["completed"][0]["status"] == "cancelled"
    assert reference_hits.cancelled_at is not None and native_hits.cancelled_at is not None
    assert len(native_hits) - native_hits.cancelled_at <= 1
    remaining = reference_hits[reference_hits.cancelled_at:]
    if protocol == "electrum":
        assert len(remaining) <= 7
        assert all(request["method"] == "blockchain.scripthash.get_history" for request in remaining)
    else:
        assert len(remaining) <= 1
