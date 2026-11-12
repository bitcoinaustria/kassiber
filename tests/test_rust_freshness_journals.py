"""Default desktop refresh: ordered events and the complete shared book."""
from contextlib import ExitStack
from dataclasses import replace
import itertools
import json
from pathlib import Path
import shutil
import sys
import time
import uuid
from unittest.mock import patch

import pytest

from kassiber import daemon_freshness as df
from kassiber.cli import handlers
from kassiber.core import custody_journal
from tests import test_rust_freshness_product as product
from tests.test_rust_freshness_product import product_book_templates  # noqa: F401
from tests.test_rust_wallet_publication import _fixture
from tests.rust_sync_chain import serve

pytestmark = product.pytestmark
CLOCK = "2026-10-31T12:00:00Z"


@pytest.fixture(autouse=True)
def default_refresh_sources(monkeypatch):
    original_args = product.arguments
    original_environment = product._environment
    original_refresh = df.refresh_journals_step
    original_build = custody_journal.build_ledger_state
    original_reference = product.reference
    original_round_trip = product.round_trip
    counter = None

    def reference(*args, **kwargs):
        nonlocal counter
        counter = itertools.count(1)
        return original_reference(*args, **kwargs)

    def arguments(kind, **extra):
        args = original_args(kind, **extra)
        key = "process_journals" if kind == product.KINDS[0] else "journals"
        if key not in extra:
            args.pop(key)
        return args

    def refresh(*args, **kwargs):
        class DurableUUID(uuid.UUID):
            def __str__(self):
                return str(uuid.UUID(int=next(counter), version=4))
        def build(*args, **kwargs):
            transient = itertools.count(100_000)
            with patch.object(uuid, "uuid4", side_effect=lambda: uuid.UUID(int=next(transient))):
                return original_build(*args, **kwargs)
        hooks = replace(handlers._metadata_hooks(), now_iso=lambda: CLOCK)
        with ExitStack() as stack:
            stack.enter_context(patch.object(uuid, "uuid4", lambda: DurableUUID(int=0, version=4)))
            stack.enter_context(patch.object(handlers, "_metadata_hooks", return_value=hooks))
            stack.enter_context(patch.object(custody_journal, "build_ledger_state", build))
            for target in (
                "kassiber.core.custody_journal.now_iso",
                "kassiber.core.custody_journal.custody_build_clock",
                "kassiber.core.custody_filed_reports.now_iso",
                "kassiber.core.filed_report_chain.now_iso",
                "kassiber.core.custody_authored_migration.now_iso",
                "kassiber.core.custody_components._now_iso",
                "kassiber.core.custody_review_terms.now_iso",
            ):
                stack.enter_context(patch(target, return_value=CLOCK))
            return original_refresh(*args, **kwargs)

    monkeypatch.setattr(product, "arguments", arguments)
    monkeypatch.setattr(product, "reference", reference)
    monkeypatch.setattr(df, "refresh_journals_step", refresh)
    def environment(*args, **kwargs):
        return {**original_environment(*args, **kwargs),
                "KASSIBER_RUST_TEST_JOURNAL_TIME": CLOCK,
                "KASSIBER_RUST_TEST_JOURNAL_IDS": "sequential",
                "KASSIBER_RUST_TEST_JOURNAL_PYTHON": sys.executable,
                "KASSIBER_RUST_TEST_JOURNAL_REFERENCE": str(Path(__file__).parent / "rust_port/journal_job_reference.py"),
                "KASSIBER_HYBRID_LOG": str(args[0] / "journal-census.jsonl")}
    monkeypatch.setattr(product, "_environment", environment)

    def round_trip(*args, **kwargs):
        original_round_trip(*args, **kwargs)
        assert not (args[0] / "journal-census.jsonl").exists(), "Valid books must need no journal fallback"
    monkeypatch.setattr(product, "round_trip", round_trip)


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("kind", product.KINDS)
@pytest.mark.parametrize("fixture", ["empty", "receive-change", "checkpoint", "reorg", "mempool"])
def test_default_refresh_envelopes_events_and_all_tables(tmp_path, protocol, kind, fixture):
    config, chain, rounds = _fixture(fixture)
    roots, books = product._books(tmp_path, config)
    with serve(chain, protocol) as url:
        for index, mutate in enumerate(rounds):
            if mutate:
                mutate()
            product.round_trip(tmp_path, chain, protocol, url, roots, books, kind, index)


@pytest.mark.parametrize("kind", product.KINDS[1:])
@pytest.mark.parametrize("run", [False, True])
def test_default_refresh_enqueue_and_limit(tmp_path, kind, run):
    config, chain, _ = _fixture("empty")
    roots, books = product._books(tmp_path, config)
    with serve(chain, "esplora") as url:
        product.round_trip(tmp_path, chain, "esplora", url, roots, books, kind, run=run, limit=1)
        if not run:
            assert chain.hits == []


def test_default_refresh_current_after_repeat(tmp_path):
    from tests.rust_sync_parity import connect
    config, chain, _ = _fixture("empty")
    roots, books = product._books(tmp_path, config)
    with serve(chain, "esplora") as url:
        for index in range(2):
            product.round_trip(tmp_path, chain, "esplora", url, roots, books, product.KINDS[0], index)
    with connect(books[1]) as conn:
        row = conn.execute("SELECT result_json FROM freshness_jobs WHERE job_type='journal_refresh' "
                           "ORDER BY created_at DESC LIMIT 1").fetchone()
        assert json.loads(row[0])["rebuilt"] is False


@pytest.mark.parametrize("kind", product.KINDS)
@pytest.mark.parametrize("value", [None, True])
@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_default_refresh_null_and_explicit_journal_selection(tmp_path, kind, value, protocol):
    config, chain, _ = _fixture("empty")
    roots, books = product._books(tmp_path, config)
    key = "process_journals" if kind == product.KINDS[0] else "journals"
    with serve(chain, protocol) as url:
        product.round_trip(tmp_path, chain, protocol, url, roots, books, kind, **{key: value})


@pytest.mark.parametrize("kind", product.KINDS)
@pytest.mark.parametrize("fence", ["early-cancel", "offline", "no-egress", "stale-scope", "malformed-scope"])
def test_default_refresh_consent_fences(tmp_path, kind, fence):
    product.test_product_consent_fences_before_transport(tmp_path, kind, fence)


@pytest.mark.parametrize("kind", product.KINDS)
@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_default_refresh_graph_publication_after_journals(tmp_path, kind, protocol):
    from tests.rust_sync_parity import connect
    config, chain, _ = _fixture("empty")
    txid, _ = chain.fund(product.FOREIGN, 100000)
    roots, books = product._books(tmp_path, config)
    for book in books:
        with connect(book) as conn:
            product.add(conn, txid)
            conn.execute("UPDATE transactions SET occurred_at='2025-01-01T12:00:00Z'")
    with serve(chain, protocol) as url:
        product.round_trip(tmp_path, chain, protocol, url, roots, books, kind)
    with connect(books[1]) as conn:
        assert conn.execute("SELECT COUNT(*) FROM transaction_graph_cache WHERE txid=?", (txid,)).fetchone()[0] == 1


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_default_refresh_workspace_context_and_book_order(tmp_path, protocol):
    product.test_product_workspace_preserves_active_context_and_book_order(tmp_path, protocol)


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("kind", product.KINDS)
def test_default_refresh_reviewed_books_and_restarts(tmp_path, protocol, kind):
    product.test_product_sync_reviewed_books(tmp_path, protocol, kind, "authority")


@pytest.mark.parametrize("kind", product.KINDS)
@pytest.mark.parametrize("shape", ["disabled", "paused", "queued-pairing"])
def test_default_refresh_policy_and_coalesced_pairing(tmp_path, kind, shape):
    from kassiber.core import freshness
    from tests.rust_sync_parity import connect, deterministic_sources
    config, chain, _ = _fixture("empty")
    roots, books = product._books(tmp_path, config)
    for book in books:
        with connect(book) as conn, deterministic_sources("journal-seed"):
            if shape == "disabled":
                conn.execute("INSERT INTO settings VALUES('freshness.policy.profile.p',?)",
                             (json.dumps({"source_classes": {"journals": False}}),))
            elif shape == "paused":
                freshness.upsert_source_state(conn, profile_id="p", source_key="journals:p",
                    source_type="journals", source_label="Journal refresh", status="paused", paused=True)
            else:
                freshness.enqueue_job(conn, profile_id="p", job_type="journal_refresh",
                    source_type="journals", source_key="journals:p", source_label="Journal refresh",
                    payload={"auto_pair": True}, priority=80)
    with serve(chain, "esplora") as url:
        product.round_trip(tmp_path, chain, "esplora", url, roots, books, kind)


@pytest.mark.parametrize("hybrid", [False, True])
@pytest.mark.parametrize("shape", ["build", "admission", "timestamp", "priced-timestamp", "fault", "after-first-commit"])
def test_journal_refusal_census_and_hybrid_only_replay(tmp_path, hybrid, shape):
    from kassiber.db import open_db
    from tests.rust_port.journal_refresh_oracle import book as pairing_book
    from tests.rust_freshness_session import Session
    from tests.rust_sync_parity import connect
    from tests.test_rust_journal_commands import seed
    roots = (tmp_path / "python", tmp_path / "rust")
    if shape != "after-first-commit":
        seed(roots[0], shape={"build": "build-refusal", "admission": "admission-refusal",
                             "timestamp": "changed", "priced-timestamp": "changed", "fault": "changed"}[shape])
        if shape in {"timestamp", "priced-timestamp"}:
            with connect(roots[0] / "kassiber.sqlite3") as conn:
                conn.execute("UPDATE transactions SET occurred_at='fixed'")
                if shape == "timestamp":
                    conn.execute("UPDATE transactions SET fiat_rate=NULL,fiat_rate_exact=NULL,fiat_value=NULL,fiat_value_exact=NULL")
    else:
        roots[0].mkdir()
        with open_db(roots[0]) as conn:
            pairing_book("exact", connection=conn)
            conn.execute("INSERT OR REPLACE INTO settings VALUES('context_workspace','w'),('context_profile','p')")
        conn.close()
    shutil.copytree(roots[0], roots[1])
    log = tmp_path / "census.jsonl"
    env = product._environment(tmp_path)
    env.pop("KASSIBER_RUST_TEST_JOURNAL_PYTHON")
    env.pop("KASSIBER_RUST_TEST_JOURNAL_REFERENCE")
    env["KASSIBER_HYBRID_LOG"] = str(log)
    if shape == "after-first-commit":
        env["KASSIBER_RUST_TEST_JOURNAL_FAIL_SECOND_BUILD"] = "1"
    if shape == "fault":
        env["KASSIBER_RUST_TEST_JOURNAL_FAIL_AFTER_WRITE"] = "1"
    if hybrid:
        env["KASSIBER_RUST_TEST_JOURNAL_PYTHON"] = sys.executable
        env["KASSIBER_RUST_TEST_JOURNAL_REFERENCE"] = str(Path(__file__).parent / "rust_port/journal_job_reference.py")
    with Session(roots[1], env) as session:
        args = {"rates": False, "auto_pair": shape == "after-first-commit"}
        session.send(product.KINDS[1], "refresh", args)
        result = session.until("refresh", product.KINDS[1])
        session.finish()
    (tmp_path / "journal-refusal.json").write_text(json.dumps(result, indent=2))
    assert result["kind"] == product.KINDS[1], result
    records = [json.loads(line) for line in log.read_text().splitlines()]
    refusal, *replay = records
    assert refusal == {"command": "freshness.journal_refresh", "event": "refusal",
        "refusal_shape": {"build": "building:AppError", "admission": "preparing:ValueError",
                          "timestamp": "pricing:AppError",
                          "priced-timestamp": "building:EngineValueError",
                          "fault": "storing:DatabaseError",
                          "after-first-commit": "storing:DatabaseError"}[shape],
        "replay_safe": shape != "after-first-commit"}
    assert replay == ([{"command": "freshness.journal_refresh", "served_by": "python"}]
                      if hybrid and shape != "after-first-commit" else [])
    assert result["data"]["journals"]["status"] == ("processed" if shape == "fault" and hybrid else "failed"), result


@pytest.mark.parametrize("replay", [False, True, "cancel"], ids=["native", "replay", "cancel"])
def test_encrypted_journal_job_and_hybrid_key_handoff(tmp_path, replay):
    from tests.rust_freshness_session import Session
    from tests.test_rust_journal_commands import KEY, book_dump, seed
    root = seed(tmp_path / "encrypted", cipher=True, shape="changed")
    before = book_dump(root, cipher=True) if replay == "cancel" else None
    env = product._environment(tmp_path)
    log = tmp_path / "journal-census.jsonl"
    if replay:
        env["KASSIBER_RUST_TEST_JOURNAL_FAIL_AFTER_WRITE"] = "1"
    else:
        env.pop("KASSIBER_RUST_TEST_JOURNAL_PYTHON")
        env.pop("KASSIBER_RUST_TEST_JOURNAL_REFERENCE")
    with Session(root, env) as session:
        session.send("daemon.unlock", "unlock", {"auth_response": {"passphrase_secret": KEY}})
        assert session.until("unlock")["kind"] == "daemon.unlock"
        session.send(product.KINDS[1], "refresh", {"rates": False})
        if replay == "cancel":
            deadline = time.monotonic() + 120
            while not log.exists() or '"served_by":"python"' not in log.read_text():
                assert time.monotonic() < deadline
                time.sleep(0.01)
            session.send("test.freshness.request.cancel", "cancel", {"target_request_id": "refresh"})
            assert session.until("cancel")["data"]["cancelled"]
        result = session.until("refresh", product.KINDS[1])
        cancelled = replay == "cancel"
        assert result["data"]["journals"]["status"] == ("cancelled" if cancelled else "processed"), result
        session.send("daemon.lock", "lock")
        assert session.until("lock")["kind"] == "daemon.lock"
        session.finish()
        assert KEY not in json.dumps(session.rows)
    rows = book_dump(root, cipher=True)
    profile = dict(zip(rows["profiles"]["columns"], rows["profiles"]["rows"][0], strict=True))
    if cancelled:
        assert profile["last_processed_at"] == "2020-01-01T00:00:00Z"
        domain = lambda tables: {name: value for name, value in tables.items()
                                 if name not in {"freshness_jobs", "freshness_source_states"}}
        assert domain(rows) == domain(before)
    else:
        assert profile["last_processed_at"] is not None
        assert profile["last_processed_input_version"] == profile["journal_input_version"]
    jobs = [dict(zip(rows["freshness_jobs"]["columns"], row, strict=True))
            for row in rows["freshness_jobs"]["rows"]]
    assert [(job["job_type"], job["status"]) for job in jobs] == [
        ("journal_refresh", "cancelled" if cancelled else "done")]
    if replay:
        records = [json.loads(line) for line in log.read_text().splitlines()]
        assert records == [
            {"command": "freshness.journal_refresh", "event": "refusal",
             "refusal_shape": "storing:DatabaseError", "replay_safe": True},
            {"command": "freshness.journal_refresh", "served_by": "python"},
        ]
    else:
        assert not log.exists()


@pytest.mark.parametrize("scenario", ["exact", "rules", "unpaired", "ownership", "booked"])
@pytest.mark.parametrize("auto_pair", [False, True])
def test_journal_job_pairing_and_caller_authority(tmp_path, scenario, auto_pair):
    from kassiber.db import open_db
    from tests.rust_port.journal_refresh_oracle import book as pairing_book
    from tests.rust_sync_parity import connect, copy_book
    config, chain, _ = _fixture("empty")
    roots, books = product._books(tmp_path, config)
    with open_db(roots[0]) as conn:
        pairing_book(scenario, connection=conn)
        conn.execute("INSERT OR REPLACE INTO settings VALUES('context_workspace','w'),('context_profile','p')")
    conn.close()
    copy_book(books[0], books[1])
    with serve(chain, "esplora") as url:
        product.round_trip(tmp_path, chain, "esplora", url, roots, books, product.KINDS[1], auto_pair=auto_pair)
    assert chain.hits == []


@pytest.mark.parametrize("control", ["cancel", "revoke", "switch", "lock"])
def test_default_refresh_slot_and_cancel(tmp_path, control):
    from tests.rust_freshness_session import Session
    from tests.rust_sync_parity import connect
    config, _, _ = _fixture("empty")
    roots, books = product._books(tmp_path, config)
    with connect(books[1]) as conn:
        conn.execute("DELETE FROM wallets")
        conn.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) "
                     "VALUES('q','w','Other','EUR','later')")
    env = {**product._environment(tmp_path), "KASSIBER_RUST_TEST_JOURNAL_WAIT_MS": "5000"}
    with Session(roots[1], env) as session:
        session.send(product.KINDS[1], "refresh", {"rates": False})
        while session.read().get("data", {}).get("phase") != "journal_refresh":
            pass
        session.send(product.KINDS[1], "competing", {"rates": False})
        assert session.until("competing")["error"]["code"] == "project_operation_in_progress"
        session.send("test.freshness.select", "select", {"kind": product.KINDS[1], "args": {"rates": False}})
        assert session.until("select")["error"]["code"] == "project_operation_in_progress"
        if control in {"cancel", "revoke"}:
            session.send(f"test.freshness.request.{control}", "control", {"target_request_id": "refresh"})
            assert session.until("control")["kind"] == f"test.freshness.request.{control}"
            session.until("refresh", product.KINDS[1])
        else:
            session.send("ui.profiles.switch" if control == "switch" else "daemon.lock", "control",
                         {"profile_id": "q"} if control == "switch" else {})
            assert session.until("control")["kind"] == ("ui.profiles.switch" if control == "switch" else "daemon.lock")
        with connect(books[1]) as conn:
            assert conn.execute("SELECT last_processed_at FROM profiles WHERE id='p'").fetchone()[0] is None
            assert conn.execute("SELECT COUNT(*) FROM freshness_jobs WHERE status='running'").fetchone()[0] == 0
        session.finish()
