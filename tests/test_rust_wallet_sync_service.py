"""Native wallet service against Python's complete CLI wallet-sync handler."""
import copy
import json
import os
from pathlib import Path
import subprocess

import pytest

from kassiber import __version__
from kassiber.cli import handlers
from kassiber.envelope import build_envelope
from tests.rust_sync_chain import serve
from tests.rust_sync_env import child_env
from tests.rust_sync_parity import (
    connect, copy_book, create_book, deterministic_sources, dump_bytes, runtime, seed_reviewed_state,
)
from tests.test_rust_bdk_execution import bdk_request_clock
from tests.test_rust_wallet_publication import _fixture, _sequence
from tests.rust_sync_parity import fixed_elapsed_ms
from tests.rust_fixture_binary import fixture_binary

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires the guarded Rust sync service fixture")


def _round(tmp_path, chain, protocol, url, python_book, rust_book, *, index=0,
           force_full=False, sql=None, reviewed=False, clear_authority=False, sync_all=False):
    for book in (python_book, rust_book):
        conn = connect(book)
        if reviewed and index:
            seed_reviewed_state(conn, clear_authority=clear_authority and index == 1)
        if sql:
            conn.executescript(sql)
            conn.commit()
        conn.close()
    initial_chain = copy.deepcopy({k: v for k, v in chain.__dict__.items() if k not in {"lock", "hits"}})
    rt = runtime(protocol, url)
    conn = connect(python_book)
    try:
        with deterministic_sources(f"r{index}", index * 1000), bdk_request_clock(1_700_000_000):
            expected = build_envelope("wallets.sync", handlers.sync_wallet(
                conn, rt, "w", "p", wallet_ref=None if sync_all else "a",
                sync_all=sync_all, force_full=force_full))
    finally:
        conn.close()
    with chain.lock:
        chain.__dict__.update(initial_chain)
    payload = {"book": str(rust_book), "profile_id": "p", "wallet": None if sync_all else "a", "all": sync_all,
               "backend": rt["backends"]["fixture"], "force_full": force_full, "version": __version__,
               "prefix": f"r{index}", "now": index * 1000, "start_time": 1_700_000_000}
    binary = fixture_binary("sync_fixture")
    result = subprocess.run([str(binary)], input=json.dumps(payload).encode(), capture_output=True,
                            timeout=90, env=child_env(tmp_path))
    assert result.returncode == 0 and result.stderr == b"", result.stderr.decode()
    actual = json.loads(result.stdout)
    expected, actual = fixed_elapsed_ms(expected), fixed_elapsed_ms(actual)
    assert actual == expected, (expected, actual)
    assert dump_bytes(rust_book) == dump_bytes(python_book)


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("sync_all", [False, True], ids=["wallet", "all"])
@pytest.mark.parametrize("fixture", ["empty", "receive-change", "checkpoint", "multi-script", "fixed", "gap-one",
                                     "reorg", "retraction", "mempool", "coinbase"])
def test_sync_service_fixture_books_match_python(tmp_path, protocol, sync_all, fixture):
    config, chain, rounds = _fixture(fixture)
    python_book, rust_book = tmp_path / "python.sqlite", tmp_path / "rust.sqlite"
    create_book(python_book, config)
    copy_book(python_book, rust_book)
    with serve(chain, protocol) as url:
        for index, mutate in enumerate(rounds):
            if mutate:
                mutate()
            _round(tmp_path, chain, protocol, url, python_book, rust_book, index=index, sync_all=sync_all)


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("sequence", ["reanchor", "reorg-drop", "preflight-race", "mempool-drop-return",
                                      "replacement", "height-change", "force-full", "force-without-state"])
def test_sync_service_sequence_books_match_python_across_restarts(tmp_path, protocol, sequence):
    config, chain, rounds, state = _sequence(sequence)
    python_book, rust_book = tmp_path / "python.sqlite", tmp_path / "rust.sqlite"
    create_book(python_book, config)
    copy_book(python_book, rust_book)
    with serve(chain, protocol) as url:
        for index, mutate in enumerate(rounds):
            if mutate:
                mutate()
            _round(tmp_path, chain, protocol, url, python_book, rust_book, index=index,
                   force_full=bool(state["force"]), sql=state["sql"])


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("fixture", ["checkpoint", "reorg", "retraction", "mempool", "authority"])
def test_sync_service_preserves_reviewed_books(tmp_path, protocol, fixture):
    config, chain, rounds = _fixture("checkpoint" if fixture == "authority" else fixture)
    python_book, rust_book = tmp_path / "python.sqlite", tmp_path / "rust.sqlite"
    create_book(python_book, config)
    copy_book(python_book, rust_book)
    with serve(chain, protocol) as url:
        for index, mutate in enumerate(rounds):
            if mutate:
                mutate()
            _round(tmp_path, chain, protocol, url, python_book, rust_book, index=index,
                   reviewed=True, clear_authority=fixture == "authority")


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("fence", ["missing", "revoked", "cancelled", "no-egress"])
def test_sync_service_process_fences_leave_full_books_unchanged(tmp_path, protocol, fence):
    config, _, _ = _fixture("empty")
    book = tmp_path / "book.sqlite"
    create_book(book, config)
    before = dump_bytes(book)
    url = "tcp://127.0.0.1:1" if protocol == "electrum" else "http://127.0.0.1:1"
    payload = {"book": str(book), "profile_id": "p", "wallet": "a", "all": False,
               "backend": runtime(protocol, url)["backends"]["fixture"], "version": __version__,
               "consent": fence != "missing", "revoke": fence == "revoked", "cancel": fence == "cancelled"}
    env = child_env(tmp_path)
    if fence == "no-egress":
        env["KASSIBER_NO_EGRESS"] = "1"
    result = subprocess.run([str(fixture_binary("sync_fixture"))],
                            input=json.dumps(payload).encode(), capture_output=True, timeout=15, env=env)
    assert result.returncode == 0 and result.stderr == b"", result.stderr.decode()
    actual = json.loads(result.stdout)
    assert actual["error"]["code"] == {"missing": "interaction_required", "revoked": "interaction_required",
                                        "cancelled": "cancelled", "no-egress": "network_egress_disabled"}[fence]
    assert dump_bytes(book) == before


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_sync_service_uses_active_default_backend(tmp_path, protocol):
    config, chain, _ = _fixture("receive-change")
    config.pop("backend", None)
    python_book, rust_book = tmp_path / "python.sqlite", tmp_path / "rust.sqlite"
    create_book(python_book, config)
    copy_book(python_book, rust_book)
    with serve(chain, protocol) as url:
        _round(tmp_path, chain, protocol, url, python_book, rust_book)
