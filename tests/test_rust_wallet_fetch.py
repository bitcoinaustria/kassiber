"""Native preparation and publication against unchanged Python full books."""
import copy
import json
import os
import subprocess

import pytest

from kassiber import __version__
from tests.rust_fixture_binary import fixture_binary
from tests.rust_sync_chain import serve
from tests.rust_sync_env import child_env
from tests.rust_sync_parity import (
    connect, copy_book, create_book, deserialize_fetch, dump, dump_bytes,
    python_apply, python_prefetch, runtime, seed_reviewed_state, serialize_fetch,
)
from tests.test_rust_bdk_execution import bdk_request_clock
from tests.test_rust_wallet_publication import _fixture, _sequence
from tests.rust_sync_parity import fixed_elapsed_ms

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires the guarded Rust publication fixture")


def _round(tmp_path, chain, protocol, url, python_book, rust_book, *, index=0,
           force_full=False, sql=None, reviewed=False, clear_authority=False):
    for book in (python_book, rust_book):
        conn = connect(book)
        if reviewed and index:
            seed_reviewed_state(conn, clear_authority=clear_authority and index == 1)
        if sql:
            conn.executescript(sql)
            conn.commit()
        conn.close()
    # A preflight-race fixture mutates its backend during preparation. Each
    # implementation must see the same starting backend and the same race.
    initial_chain = copy.deepcopy({k:v for k,v in chain.__dict__.items() if k not in {"lock","hits"}})
    rt = runtime(protocol, url)
    conn = connect(python_book)
    try:
        with bdk_request_clock(1_700_000_000):
            fetch, checkpoints = python_prefetch(conn, rt, force_full=force_full)
        data = serialize_fetch(fetch)
        assert "error" not in data, data
        expected = python_apply(conn, rt, deserialize_fetch(data), checkpoints,
                                force_full=force_full, prefix=f"r{index}", now=index * 1000)
    finally:
        conn.close()
    with chain.lock:
        chain.__dict__.update(initial_chain)
    conn = connect(rust_book)
    captured = conn.execute("SELECT config_json FROM wallets WHERE id='a'").fetchone()[0]
    conn.close()
    payload = {"book": str(rust_book), "profile_id": "p", "wallet_id": "a", "captured_config": captured,
               "native_backend": rt["backends"]["fixture"], "force_full": force_full, "version": __version__,
               "start_time": 1_700_000_000, "prefix": f"r{index}", "now": index * 1000}
    binary = fixture_binary("publication_fixture")
    result = subprocess.run([str(binary)], input=json.dumps(payload).encode(), capture_output=True,
                            timeout=90, env=child_env(tmp_path))
    assert result.returncode == 0 and result.stderr == b"", result.stderr.decode()
    actual = json.loads(result.stdout)
    actual, data = fixed_elapsed_ms(actual), fixed_elapsed_ms(data)
    # Persist mismatches only in disposable worktree scratch for inspection.
    if actual.get("fetch") != data:
        (tmp_path / "python-fetch.json").write_text(json.dumps(data, indent=2, sort_keys=True))
        (tmp_path / "rust-fetch.json").write_text(json.dumps(actual.get("fetch"), indent=2, sort_keys=True))
    assert actual.get("fetch") == data
    assert actual.get("result") == fixed_elapsed_ms(expected.get("result", [])), (expected, actual)
    assert dump_bytes(rust_book) == dump_bytes(python_book)
    return actual


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("fixture", ["empty", "receive-change", "checkpoint", "multi-script", "fixed", "gap-one",
                                     "reorg", "retraction", "mempool", "coinbase"])
def test_native_fetch_fixture_books_match_python(tmp_path, protocol, fixture):
    config, chain, rounds = _fixture(fixture)
    python_book, rust_book = tmp_path / "python.sqlite", tmp_path / "rust.sqlite"
    create_book(python_book, config)
    copy_book(python_book, rust_book)
    with serve(chain, protocol) as url:
        for index, mutate in enumerate(rounds):
            if mutate:
                mutate()
            _round(tmp_path, chain, protocol, url, python_book, rust_book, index=index)
    if fixture == "retraction":
        assert not dump(rust_book)["transactions"]


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("sequence", ["reanchor", "reorg-drop", "preflight-race", "mempool-drop-return",
                                      "replacement", "height-change", "force-full", "force-without-state"])
def test_native_fetch_sequence_books_match_python_across_restarts(tmp_path, protocol, sequence):
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
def test_native_fetch_preserves_saved_reports_and_current_journals(tmp_path, protocol, fixture):
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
    if fixture in {"retraction", "mempool", "reorg"}:
        assert dump(rust_book)["filed_report_chain_impacts"]
