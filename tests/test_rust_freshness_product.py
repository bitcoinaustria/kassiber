import copy
from contextlib import ExitStack
import json
import os
from pathlib import Path
import threading
from types import SimpleNamespace
from unittest.mock import patch

import pytest

from kassiber import daemon_freshness as df
from kassiber.daemon import SUPPORTED_KINDS
from kassiber.core import freshness, sync as core_sync, transaction_graph
from kassiber.envelope import build_envelope
from tests.rust_freshness_session import Session
from tests.rust_sync_chain import FOREIGN, serve
from tests.rust_sync_parity import connect, copy_book, deterministic_sources, dump_bytes, runtime, seed_reviewed_state
from tests.test_rust_bdk_execution import bdk_request_clock
from tests.test_rust_freshness_graph import Out, add
from tests.test_rust_wallet_publication import _fixture, _sequence
from tests.test_rust_wallet_sync_cli import _books as _seed_books, _environment

pytestmark = pytest.mark.skipif(not os.environ.get("KASSIBER_RUST_BIN"), reason="requires native freshness daemon")
KINDS = ["ui.wallets.sync", "ui.freshness.run", "ui.workspace.freshness.run"]
_TEMPLATES = {}
_TEMPLATE_ROOT = None


@pytest.fixture(scope="module", autouse=True)
def product_book_templates(tmp_path_factory):
    global _TEMPLATE_ROOT
    _TEMPLATE_ROOT = tmp_path_factory.mktemp("product-templates")
    yield
    _TEMPLATES.clear()
    _TEMPLATE_ROOT = None


def _books(tmp_path, config):
    key = json.dumps(config, sort_keys=True)
    if key not in _TEMPLATES:
        template_root = _TEMPLATE_ROOT / str(len(_TEMPLATES))
        template_root.mkdir()
        _, books = _seed_books(template_root, config)
        _TEMPLATES[key] = books[0]
    roots = tuple(tmp_path / name for name in ("python", "rust"))
    books = tuple(root / "kassiber.sqlite3" for root in roots)
    for root, book in zip(roots, books, strict=True):
        root.mkdir()
        copy_book(_TEMPLATES[key], book)
    return roots, books


def arguments(kind, **extra):
    return {**({"process_journals": False} if kind == KINDS[0] else
               {"rates": False, "journals": False, **({"workspace_id": "w"} if kind == KINDS[2] else {})}), **extra}


def reference(book, rt, kind, args, index):
    events, grants = [], []
    with connect(book) as conn, ExitStack() as stack:
        stack.enter_context(deterministic_sources(f"r{index}", now=index * 1000))
        stack.enter_context(bdk_request_clock(1_700_000_000))
        stack.enter_context(patch.object(df, "now_iso", side_effect=lambda: freshness.now_iso()))
        stack.enter_context(patch.object(core_sync, "time", SimpleNamespace(monotonic=lambda: 100.0)))
        token = df._GRAPH_SYNC_FOLLOWUPS.set(grants)
        try:
            routes = {KINDS[0]: df._wallets_sync_payload, KINDS[1]: df._freshness_run_payload,
                      KINDS[2]: df._workspace_freshness_run_payload}
            options = {"strict": False} if kind == KINDS[0] else {}
            value = routes[kind](conn, rt, args, progress_observer=events.append, **options)
        finally:
            df._GRAPH_SYNC_FOLLOWUPS.reset(token)
    rows = [{**build_envelope(kind + ".progress", event), "request_id": "sync"} for event in events]
    rows.append({**build_envelope(kind, value), "request_id": "sync"})
    with connect(book) as conn, deterministic_sources(f"r{index}-graph"), ExitStack() as stack:
        stack.enter_context(patch.object(freshness, "now_iso", return_value="2030-01-01T00:00:01Z"))
        stack.enter_context(patch.object(transaction_graph, "now_iso", return_value="2030-01-01T00:00:01Z"))
        out = Out()
        for grant in grants:
            df._run_graph_followup(conn, rt, out, grant, threading.Event())
        rows.extend(out.events)
    return rows


def candidate(session, book, rt, kind, args, index=0, *, chain):
    before = dump_bytes(book)
    before_hits = len(chain.hits)
    session.send("test.freshness.select", "select", {"kind": kind, "args": args, "request_id": "sync"})
    selection = session.until("select")
    assert dump_bytes(book) == before
    assert len(chain.hits) == before_hits
    supported = selection["data"]["supported"]
    served_by = "rust" if supported else "python"
    session.served = [*getattr(session, "served", []), {"command": kind, "served_by": served_by}]
    if not supported:
        rows = reference(book, rt, kind, args, index)
        return served_by, rows[-1], rows
    session.send(kind, "sync", args)
    return served_by, session.until("sync", kind), None


def round_trip(tmp_path, chain, protocol, url, roots, books, kind, index=0, **extra):
    args = arguments(kind, **extra)
    initial = copy.deepcopy({key: value for key, value in chain.__dict__.items() if key not in {"lock", "hits"}})
    expected = reference(books[0], runtime(protocol, url), kind,
                         {key: value for key, value in args.items() if key != "expected_scope"}, index)
    with chain.lock:
        chain.__dict__.update(initial)
    env = _environment(tmp_path, protocol, url, index)
    env["KASSIBER_RUST_TEST_GRAPH_PREFIX"] = f"r{index}-graph"
    before_hits = len(chain.hits)
    with Session(roots[1], env) as session:
        ready = session.rows[0]["data"]["supported_kinds"]
        assert kind in ready
        assert [item for item in ready if not item.startswith("test.")] == [item for item in SUPPORTED_KINDS if item in ready]
        session.send("status", "open")
        assert session.until("open")["kind"] == "status"
        assert len(chain.hits) == before_hits
        served_by, terminal, _ = candidate(session, books[1], runtime(protocol, url), kind, args, index, chain=chain)
        assert served_by == "rust", session.served
        assert terminal["kind"] == kind, terminal
        graph_events = sum(row["kind"] == "ui.freshness.background" for row in expected)
        while sum(row["kind"] == "ui.freshness.background" for row in session.rows) < graph_events:
            session.read()
        session.finish()
        actual = [row for row in session.rows if row.get("request_id") == "sync" or row.get("event")]
    (tmp_path / "product-parity.json").write_text(json.dumps({"actual": actual, "expected": expected}, indent=2))
    assert json.dumps(actual, sort_keys=True) == json.dumps(expected, sort_keys=True)
    assert dump_bytes(books[1]) == dump_bytes(books[0])
    assert session.served == [{"command": kind, "served_by": "rust"}]
    record = {"command": kind, "served_by": served_by, "fallbacks": 0}
    witness = Path(os.environ.get("KASSIBER_RUST_PRODUCT_WITNESS", tmp_path / "product-served.jsonl"))
    with witness.open("a") as output:
        output.write(json.dumps(record) + "\n")


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("kind", KINDS)
@pytest.mark.parametrize("fixture", ["empty", "receive-change", "checkpoint", "multi-script", "fixed", "gap-one",
                                      "reorg", "retraction", "mempool", "coinbase"])
def test_product_sync_envelopes_events_and_all_tables(tmp_path, protocol, kind, fixture):
    config, chain, rounds = _fixture(fixture)
    roots, books = _books(tmp_path, config)
    with serve(chain, protocol) as url:
        for index, mutate in enumerate(rounds):
            if mutate:
                mutate()
            round_trip(tmp_path, chain, protocol, url, roots, books, kind, index)


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("kind", KINDS)
@pytest.mark.parametrize("sequence", ["reanchor", "reorg-drop", "preflight-race", "mempool-drop-return",
                                       "replacement", "height-change", "force-full", "force-without-state"])
def test_product_sync_restart_sequences(tmp_path, protocol, kind, sequence):
    config, chain, rounds, state = _sequence(sequence)
    roots, books = _books(tmp_path, config)
    with serve(chain, protocol) as url:
        for index, mutate in enumerate(rounds):
            if mutate:
                mutate()
            for book in books:
                with connect(book) as conn:
                    if state["sql"]:
                        conn.executescript(state["sql"])
            extra = {"force_full": bool(state["force"])} if kind != KINDS[2] else {}
            round_trip(tmp_path, chain, protocol, url, roots, books, kind, index, **extra)


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("kind", KINDS)
@pytest.mark.parametrize("fixture", ["checkpoint", "reorg", "retraction", "mempool", "authority"])
def test_product_sync_reviewed_books(tmp_path, protocol, kind, fixture):
    config, chain, rounds = _fixture("checkpoint" if fixture == "authority" else fixture)
    roots, books = _books(tmp_path, config)
    with serve(chain, protocol) as url:
        for index, mutate in enumerate(rounds):
            if mutate:
                mutate()
            if index:
                for book in books:
                    with connect(book) as conn:
                        seed_reviewed_state(conn, clear_authority=fixture == "authority" and index == 1)
            round_trip(tmp_path, chain, protocol, url, roots, books, kind, index)


@pytest.mark.parametrize("kind", KINDS)
@pytest.mark.parametrize("shape", ["rates", "unported", "empty-id", "null-id",
                                    "number-id", "boolean-id", "array-id", "object-id", "surrogate-id", "missing-id"])
def test_product_selection_refuses_whole_invocation_without_effects(tmp_path, kind, shape):
    config, chain, _ = _fixture("empty")
    roots, books = _books(tmp_path, config)
    args, identity = arguments(kind), "sync"
    if shape == "rates":
        if kind != KINDS[0]:
            args["rates"] = True
            with connect(books[1]) as conn:
                conn.execute("INSERT INTO settings VALUES('freshness.policy.profile.p',?)",
                             (json.dumps({"source_classes": {"market_rates": True}}),))
    elif shape == "unported":
        with connect(books[1]) as conn:
            conn.execute("UPDATE wallets SET kind='lnd'")
    elif shape != "missing-id":
        identity = {"empty-id": "", "null-id": None, "number-id": 7, "boolean-id": True,
                    "array-id": [], "object-id": {}, "surrogate-id": "\ud800"}[shape]
    backend_kind = "bitcoinrpc" if kind == KINDS[0] and shape == "rates" else "esplora"
    with serve(chain, "esplora") as url, Session(roots[1], _environment(tmp_path, backend_kind, url)) as session:
        session.send("status", "open")
        assert session.until("open")["kind"] == "status"
        before = dump_bytes(books[1])
        selection = {"kind": kind, "args": args, "request_id": identity}
        if shape == "missing-id":
            selection["request"] = {}
        session.send("test.freshness.select", "select", selection)
        assert session.until("select")["data"] == {"supported": False}
        assert dump_bytes(books[1]) == before
        session.finish()
        assert chain.hits == []


@pytest.mark.parametrize("kind", KINDS)
def test_product_selection_does_not_initialize_a_book(tmp_path, kind):
    root = tmp_path / "fresh"
    with Session(root, _environment(tmp_path)) as session:
        before = {str(path.relative_to(root)): path.read_bytes() for path in root.rglob("*") if path.is_file()}
        for args in (arguments(kind), {}):
            session.send("test.freshness.select", "select", {"kind": kind, "args": args, "request_id": "sync"})
            assert session.until("select")["data"] == {"supported": False}
            assert {str(path.relative_to(root)): path.read_bytes() for path in root.rglob("*") if path.is_file()} == before
        assert not (root / "kassiber.sqlite3").exists()
        session.finish()


@pytest.mark.parametrize("shape", ["paused", "no-source"])
def test_product_wallet_selection_refuses_direct_sync_reference_path(tmp_path, shape):
    config, chain, _ = _fixture("empty")
    roots, books = _books(tmp_path, config)
    with connect(books[1]) as conn:
        if shape == "paused":
            freshness.upsert_source_state(conn, profile_id="p", source_key="onchain_wallet:a",
                                          source_type="onchain_wallet", source_label="Cold",
                                          status=freshness.STATUS_PAUSED, paused=True)
        else:
            conn.execute("UPDATE wallets SET config_json='{}'")
    with serve(chain, "esplora") as url, Session(roots[1], _environment(tmp_path, "esplora", url)) as session:
        session.send("status", "open")
        assert session.until("open")["kind"] == "status"
        before = dump_bytes(books[1])
        session.send("test.freshness.select", "select", {"kind": KINDS[0], "args": arguments(KINDS[0])})
        assert session.until("select")["data"] == {"supported": False}
        session.send(KINDS[0], "direct", arguments(KINDS[0]))
        assert session.until("direct", KINDS[0])["error"]["code"] == "native_fallback_required"
        assert dump_bytes(books[1]) == before
        assert chain.hits == []
        session.finish()


@pytest.mark.parametrize("kind", KINDS[:2])
def test_reference_all_false_without_wallet_repro_and_native_refusal(tmp_path, kind):
    config, chain, _ = _fixture("empty")
    roots, books = _books(tmp_path, config)
    args = arguments(kind, all=False)
    route = df._wallets_sync_payload if kind == KINDS[0] else df._freshness_run_payload
    with connect(books[0]) as conn, pytest.raises(AttributeError, match="strip"):
        route(conn, {}, args, **({"strict": False} if kind == KINDS[0] else {}))
    with serve(chain, "esplora") as url, Session(roots[1], _environment(tmp_path, "esplora", url)) as session:
        session.send("status", "open")
        assert session.until("open")["kind"] == "status"
        before = dump_bytes(books[1])
        session.send("test.freshness.select", "select", {"kind": kind, "args": args})
        assert session.until("select")["data"] == {"supported": False}
        assert dump_bytes(books[1]) == before
        assert chain.hits == []
        session.finish()


VALIDATION_CASES = [
    (KINDS[0], {"wallet": False}), (KINDS[0], {"wallet": " "}),
    (KINDS[0], {"wallet": "a", "all": True}), (KINDS[0], {"all": 1}),
    (KINDS[0], {"force_full": 1}), (KINDS[0], {"process_journals": 0}),
    (KINDS[1], {"wallet": False}), (KINDS[1], {"wallet": " "}),
    (KINDS[1], {"force_full": 1}), (KINDS[1], {"auto_pair": 1}),
    (KINDS[1], {"unknown": 1}), (KINDS[2], {"unknown": 1}),
    (KINDS[2], {"workspace_id": None}), (KINDS[2], {"workspace_id": " "}),
]


@pytest.mark.parametrize("kind,extra", VALIDATION_CASES)
def test_product_argument_errors_match_reference_without_effects(tmp_path, kind, extra):
    from kassiber.envelope import build_error_envelope
    from kassiber.errors import AppError

    config, chain, _ = _fixture("empty")
    roots, books = _books(tmp_path, config)
    args = arguments(kind, **extra)
    route = {KINDS[0]: df._wallets_sync_payload, KINDS[1]: df._freshness_run_payload,
             KINDS[2]: df._workspace_freshness_run_payload}[kind]
    with connect(books[0]) as conn, pytest.raises(AppError) as error:
        route(conn, {}, args, **({"strict": False} if kind == KINDS[0] else {}))
    error = error.value
    expected = {**build_error_envelope(error.code, str(error), details=error.details,
                                      hint=error.hint, retryable=error.retryable), "request_id": "sync"}
    with Session(roots[1], _environment(tmp_path)) as session:
        before = dump_bytes(books[1])
        session.send(kind, "sync", args)
        assert session.until("sync") == expected
        session.finish()
        assert dump_bytes(books[1]) == before


@pytest.mark.parametrize("kind", KINDS)
@pytest.mark.parametrize("stored", ["bad", "[]", "null", '"text"'])
def test_product_unreadable_wallet_config_is_a_typed_refusal(tmp_path, kind, stored):
    # tests/rust_port/repro_ai_wallets_sync_bad_config.py: the planner refuses
    # the whole sync with invalid_wallet_config, naming the wallet.
    from kassiber.envelope import build_error_envelope
    from kassiber.errors import AppError

    config, chain, _ = _fixture("empty")
    roots, books = _books(tmp_path, config)
    for book in books:
        with connect(book) as conn:
            conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                         "SELECT 'bad',workspace_id,profile_id,'Bad',kind,?,created_at FROM wallets WHERE id='a'",
                         (stored,))
    args = arguments(kind)
    route = {KINDS[0]: df._wallets_sync_payload, KINDS[1]: df._freshness_run_payload,
             KINDS[2]: df._workspace_freshness_run_payload}[kind]
    with connect(books[0]) as conn, pytest.raises(AppError) as error:
        route(conn, {}, args, **({"strict": False} if kind == KINDS[0] else {}))
    error = error.value
    assert (error.code, error.details) == ("invalid_wallet_config", {"wallet": "Bad"})
    expected = {**build_error_envelope(error.code, str(error), details=error.details,
                                      hint=error.hint, retryable=error.retryable), "request_id": "sync"}
    with Session(roots[1], _environment(tmp_path)) as session:
        before = dump_bytes(books[1])
        session.send(kind, "sync", args)
        assert session.until("sync") == expected
        session.finish()
        assert dump_bytes(books[1]) == before
    assert dump_bytes(books[1]) == dump_bytes(books[0])
    assert chain.hits == []


@pytest.mark.parametrize("kind", KINDS[1:])
@pytest.mark.parametrize("run", [False, True])
def test_product_enqueue_only_and_run_limit(tmp_path, kind, run):
    config, chain, _ = _fixture("receive-change")
    roots, books = _books(tmp_path, config)
    with serve(chain, "esplora") as url:
        round_trip(tmp_path, chain, "esplora", url, roots, books, kind, run=run, limit=1)
        if not run:
            assert chain.hits == []


@pytest.mark.parametrize("kind", KINDS[1:])
def test_product_hybrid_serves_defaults_without_reference_enqueue(tmp_path, kind):
    config, chain, _ = _fixture("empty")
    roots, books = _books(tmp_path, config)
    args = arguments(kind, run=False)
    args.pop("journals")
    with serve(chain, "esplora") as url:
        rt = runtime("esplora", url)
        expected = reference(books[0], rt, kind, args, 0)
        with Session(roots[1], _environment(tmp_path, "esplora", url)) as session:
            session.send("status", "open")
            assert session.until("open")["kind"] == "status"
            served_by, _, actual = candidate(session, books[1], rt, kind, args, chain=chain)
            assert served_by == "rust"
            actual = [row for row in session.rows if row.get("request_id") == "sync"]
            assert actual == expected
            assert session.served == [{"command": kind, "served_by": "rust"}]
            assert dump_bytes(books[1]) == dump_bytes(books[0])
            assert chain.hits == []
            session.finish()


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_product_workspace_preserves_active_context_and_book_order(tmp_path, protocol):
    config, chain, _ = _fixture("receive-change")
    roots, books = _books(tmp_path, config)
    for book in books:
        with connect(book) as conn:
            conn.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('q','w','Other','EUR','aaa')")
            conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                         "SELECT 'b','w','q','Other',kind,config_json,created_at FROM wallets WHERE id='a'")
    with serve(chain, protocol) as url:
        round_trip(tmp_path, chain, protocol, url, roots, books, KINDS[2])


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("kind", KINDS)
def test_product_automatic_graph_lookup_matches_reference(tmp_path, protocol, kind):
    config, chain, _ = _fixture("empty")
    txid, _ = chain.fund(FOREIGN, 100000)
    roots, books = _books(tmp_path, config)
    for book in books:
        with connect(book) as conn:
            add(conn, txid)
    with serve(chain, protocol) as url:
        round_trip(tmp_path, chain, protocol, url, roots, books, kind)
    with connect(books[1]) as conn:
        assert conn.execute("SELECT count(*) FROM transaction_graph_cache WHERE txid=?", (txid,)).fetchone()[0] == 1


@pytest.mark.parametrize("kind", KINDS)
def test_product_matching_scope_is_removed_before_argument_validation(tmp_path, kind):
    config, chain, _ = _fixture("empty")
    roots, books = _books(tmp_path, config)
    with serve(chain, "esplora") as url:
        round_trip(tmp_path, chain, "esplora", url, roots, books, kind,
                   expected_scope={"workspace_id": "w", "profile_id": "p"})


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("kind,extra", [
    (KINDS[0], {"wallet": " a ", "all": False, "force_full": None, "ignored": True}),
    (KINDS[1], {"wallet": " a ", "all": False, "force_full": None, "auto_pair": True}),
    (KINDS[2], {"workspace_id": " w "}),
])
def test_product_argument_coercions_match_reference(tmp_path, protocol, kind, extra):
    config, chain, _ = _fixture("empty")
    roots, books = _books(tmp_path, config)
    with serve(chain, protocol) as url:
        round_trip(tmp_path, chain, protocol, url, roots, books, kind, **extra)


@pytest.mark.parametrize("kind", KINDS)
@pytest.mark.parametrize("fence", ["early-cancel", "offline", "no-egress", "stale-scope", "malformed-scope"])
def test_product_consent_fences_before_transport(tmp_path, kind, fence):
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
            args = arguments(kind)
            if fence == "early-cancel":
                session.send("test.freshness.request.cancel", "cancel", {"target_request_id": "sync"})
                assert session.until("cancel")["data"]["queued"] is True
            elif fence == "offline":
                session.send("ui.network.offline.set", "offline", {"enabled": True})
                assert session.until("offline")["kind"] == "ui.network.offline.set"
            elif fence.endswith("scope"):
                args["expected_scope"] = ({"workspace_id": "w", "profile_id": "other"}
                                          if fence == "stale-scope" else {})
            session.send(kind, "sync", args)
            result = session.until("sync", kind)
            if fence in {"offline", "no-egress"}:
                data = result["data"]["books"][0] if kind == KINDS[2] else result["data"]
                assert data["completed"][0]["error"]["code"] == "network_egress_disabled"
            else:
                assert result["error"]["code"] == {"early-cancel": "cancelled", "stale-scope": "stale_context",
                                                     "malformed-scope": "validation"}[fence]
                assert dump_bytes(books[1]) == before
            assert chain.hits == []
            session.finish()


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("kind", KINDS)
@pytest.mark.parametrize("control", ["cancel", "pause"])
def test_product_source_controls_remain_available_during_fetch(tmp_path, protocol, kind, control):
    from tests.test_rust_freshness_lifecycle import BlockedHits, unpublished

    config, chain, _ = _fixture("receive-change")
    roots, books = _books(tmp_path, config)
    class ReferenceControl(list):
        def append(self, item):
            first = not self
            super().append(item)
            if first:
                with connect(books[0]) as conn, deterministic_sources("r0"):
                    job = conn.execute("SELECT id,source_key FROM freshness_jobs").fetchone()
                    if control == "cancel":
                        freshness.cancel_job(conn, job[0])
                    else:
                        freshness.pause_source(conn, "p", job[1])
                    conn.commit()

    chain.hits = ReferenceControl()
    with serve(chain, protocol) as url:
        expected = reference(books[0], runtime(protocol, url), kind, arguments(kind), 0)
        chain.hits = hits = BlockedHits(protocol)
        with Session(roots[1], _environment(tmp_path, protocol, url)) as session:
            session.send(kind, "sync", arguments(kind))
            assert hits.started.wait(30)
            try:
                before, before_hits = dump_bytes(books[1]), len(hits)
                session.send("test.freshness.select", "busy", {"kind": kind, "args": arguments(kind)})
                assert session.until("busy")["error"]["code"] == "project_operation_in_progress"
                session.send(kind, "competing", arguments(kind))
                assert session.until("competing")["error"]["code"] == "project_operation_in_progress"
                assert dump_bytes(books[1]) == before
                assert len(hits) == before_hits
                with connect(books[1]) as conn:
                    job = conn.execute("SELECT id,source_key FROM freshness_jobs").fetchone()
                args = {"job_id": job[0]} if control == "cancel" else {"source_key": job[1]}
                session.send(f"ui.freshness.{control}", "control", args)
                assert session.until("control")["kind"] == f"ui.freshness.{control}"
                hits.release.set()
                session.until("sync", kind)
                actual = [row for row in session.rows if row.get("request_id") == "sync" or row.get("event")]
                (tmp_path / "product-control-parity.json").write_text(json.dumps({"actual": actual, "expected": expected}, indent=2))
                assert actual == expected
                assert dump_bytes(books[1]) == dump_bytes(books[0])
                if control == "pause":
                    session.send("ui.freshness.resume", "resume", {"source_key": job[1]})
                    assert session.until("resume")["data"]["result"]["paused"] is False
                if control == "cancel" or kind != KINDS[2]:
                    unpublished(books[1])
                session.finish()
            finally:
                hits.release.set()
