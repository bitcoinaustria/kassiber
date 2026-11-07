"""Registered CLI wallet sync, including inert hybrid selection and full books."""
import copy
import json
import os
from pathlib import Path
import subprocess
import sys

import pytest

from kassiber.db import open_db
from tests.rust_sync_chain import serve
from tests.rust_sync_env import child_env
from tests.rust_sync_parity import connect, copy_book, create_book, dump_bytes, seed_reviewed_state
from tests.test_rust_wallet_publication import _comparable, _fixture, _sequence

ROOT = Path(__file__).resolve().parents[1]
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
HYBRID = ROOT / "tests/rust_port/hybrid_cli.py"
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires the guarded Rust binary")


def _books(tmp_path, config):
    roots = [tmp_path / name for name in ("python", "rust")]
    for root in roots:
        root.mkdir()
    python_book, rust_book = [root / "kassiber.sqlite3" for root in roots]
    create_book(python_book, config)
    conn = open_db(roots[0])
    conn.execute("INSERT OR REPLACE INTO settings VALUES('context_workspace','w')")
    conn.execute("INSERT OR REPLACE INTO settings VALUES('context_profile','p')")
    conn.commit()
    conn.close()
    copy_book(python_book, rust_book)
    return roots, (python_book, rust_book)


def _environment(tmp_path, protocol="esplora", url="http://127.0.0.1:1", index=0):
    env = child_env(tmp_path)
    owners = tmp_path / "owners"
    owners.mkdir(exist_ok=True, mode=0o700)
    env.update({
        "KASSIBER_BACKEND_FIXTURE_KIND": protocol,
        "KASSIBER_BACKEND_FIXTURE_URL": url,
        "KASSIBER_BACKEND_FIXTURE_BATCH_SIZE": "2",
        "KASSIBER_DEFAULT_BACKEND": "fixture",
        "KASSIBER_RUST_TEST_OWNER_ROOT": str(owners),
        "KASSIBER_RUST_TEST_SYNC_PREFIX": f"r{index}",
        "KASSIBER_RUST_TEST_SYNC_NOW": str(index * 1000),
        "KASSIBER_RUST_TEST_SYNC_START": "1700000000",
        "KASSIBER_HYBRID_RUST_BIN": str(Path(RUST_BIN).resolve()),
        "KASSIBER_HYBRID_LOG": str(tmp_path / "served.jsonl"),
    })
    return env


def _args(root, *, all=False, force_full=False):
    return ["--machine", "--data-root", str(root), "wallets", "sync",
            *(["--all"] if all else ["--wallet", "a"]),
            *(["--force-full"] if force_full else [])]


def _round(tmp_path, chain, protocol, url, roots, books, *, index=0, all=False,
           force_full=False, sql=None, reviewed=False, clear_authority=False):
    for book in books:
        conn = connect(book)
        if reviewed and index:
            seed_reviewed_state(conn, clear_authority=clear_authority and index == 1)
        if sql:
            conn.executescript(sql)
            conn.commit()
        conn.close()
    before = copy.deepcopy({k: v for k, v in chain.__dict__.items() if k not in {"lock", "hits"}})
    env = _environment(tmp_path, protocol, url, index)
    reference = subprocess.run(
        [sys.executable, "-m", "tests.rust_sync_cli_reference",
         *_args(roots[0], all=all, force_full=force_full)],
        capture_output=True, env=env, timeout=90,
    )
    assert reference.returncode == 0, (reference.stdout, reference.stderr)
    with chain.lock:
        chain.__dict__.update(before)
    actual = subprocess.run(
        [sys.executable, str(HYBRID), *_args(roots[1], all=all, force_full=force_full)],
        capture_output=True, env=env, timeout=90,
    )
    assert actual.returncode == 0, (actual.stdout, actual.stderr)
    assert actual.stderr == reference.stderr == b""
    expected, result = json.loads(reference.stdout), json.loads(actual.stdout)
    expected["data"] = _comparable(expected["data"], result["data"])
    assert result == expected
    assert dump_bytes(books[1]) == dump_bytes(books[0])
    log = [json.loads(line) for line in (tmp_path / "served.jsonl").read_text().splitlines()]
    assert log == [{"command": "wallets.sync", "served_by": "rust"}] * (index + 1)


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("all", [False, True], ids=["wallet", "all"])
@pytest.mark.parametrize("fixture", ["empty", "receive-change", "checkpoint", "multi-script", "fixed", "gap-one",
                                     "reorg", "retraction", "mempool", "coinbase"])
def test_sync_cli_fixture_books_match_python(tmp_path, protocol, all, fixture):
    config, chain, rounds = _fixture(fixture)
    roots, books = _books(tmp_path, config)
    with serve(chain, protocol) as url:
        for index, mutate in enumerate(rounds):
            if mutate:
                mutate()
            _round(tmp_path, chain, protocol, url, roots, books, index=index, all=all)


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("sequence", ["reanchor", "reorg-drop", "preflight-race", "mempool-drop-return",
                                      "replacement", "height-change", "force-full", "force-without-state"])
def test_sync_cli_sequences_match_python_across_restarts(tmp_path, protocol, sequence):
    config, chain, rounds, state = _sequence(sequence)
    roots, books = _books(tmp_path, config)
    with serve(chain, protocol) as url:
        for index, mutate in enumerate(rounds):
            if mutate:
                mutate()
            _round(tmp_path, chain, protocol, url, roots, books, index=index,
                   force_full=bool(state["force"]), sql=state["sql"])


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("fixture", ["checkpoint", "reorg", "retraction", "mempool", "authority"])
def test_sync_cli_reviewed_books_match_python(tmp_path, protocol, fixture):
    config, chain, rounds = _fixture("checkpoint" if fixture == "authority" else fixture)
    roots, books = _books(tmp_path, config)
    with serve(chain, protocol) as url:
        for index, mutate in enumerate(rounds):
            if mutate:
                mutate()
            _round(tmp_path, chain, protocol, url, roots, books, index=index,
                   reviewed=True, clear_authority=fixture == "authority")


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_sync_cli_active_default_matches_python(tmp_path, protocol):
    config, chain, _ = _fixture("receive-change")
    config.pop("backend", None)
    roots, books = _books(tmp_path, config)
    with serve(chain, protocol) as url:
        _round(tmp_path, chain, protocol, url, roots, books)


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
def test_sync_cli_saved_custom_backend_matches_python(tmp_path, protocol):
    config, chain, _ = _fixture("receive-change")
    config["backend"] = "saved"
    roots, books = _books(tmp_path, config)
    with serve(chain, protocol) as url:
        for book in books:
            conn = connect(book)
            conn.execute("INSERT INTO backends(name,kind,url,batch_size,created_at,updated_at) "
                         "VALUES('saved',?,?,2,'fixed','fixed')", (protocol, url))
            conn.commit()
            conn.close()
        _round(tmp_path, chain, protocol, url, roots, books)


@pytest.mark.parametrize("all", [False, True], ids=["wallet", "all"])
def test_sync_cli_source_less_wallets_match_python_without_traffic(tmp_path, all):
    _, chain, _ = _fixture("empty")
    roots, books = _books(tmp_path, {})
    if all:
        for book in books:
            conn = connect(book)
            conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                         "VALUES('b','w','p','Alpha','descriptor','{}','fixed')")
            conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                         "VALUES('c','w','p','Retired','descriptor','{\"deprecated\":true}','fixed')")
            conn.commit()
            conn.close()
    with serve(chain, "esplora") as url:
        _round(tmp_path, chain, "esplora", url, roots, books, all=all)
        assert not chain.hits


def test_sync_cli_probe_does_not_bootstrap(tmp_path):
    root = tmp_path / "missing"
    env = _environment(tmp_path)
    env["KASSIBER_RUST_TEST_PROBE"] = "1"
    result = subprocess.run([RUST_BIN, *_args(root)], capture_output=True, env=env, timeout=10)
    assert result.returncode == 0 and result.stdout == b"wallets.sync\n" and not result.stderr
    assert not root.exists()


@pytest.mark.parametrize("protocol", ["electrum", "esplora"])
@pytest.mark.parametrize("fence", ["no-egress", "offline"])
def test_sync_cli_native_failure_never_falls_back(tmp_path, protocol, fence):
    config, chain, _ = _fixture("empty")
    roots, books = _books(tmp_path, config)
    before = dump_bytes(books[1])
    with serve(chain, protocol) as url:
        env = _environment(tmp_path, protocol, url)
        if fence == "no-egress":
            env["KASSIBER_NO_EGRESS"] = "1"
        else:
            Path(env["KASSIBER_OFFLINE_PREFERENCE_FILE"]).write_text(
                '{"schema_version": 1, "enabled": true}')
        reference = subprocess.run([sys.executable, "-m", "tests.rust_sync_cli_reference",
                                    *_args(roots[0])], capture_output=True, env=env, timeout=30)
        assert reference.returncode == 1
        result = subprocess.run([sys.executable, str(HYBRID), *_args(roots[1])],
                                capture_output=True, env=env, timeout=30)
        assert result.returncode == 1
        assert json.loads(result.stdout)["error"]["code"] == "network_egress_disabled"
        assert json.loads(result.stdout) == json.loads(reference.stdout)
        assert not chain.hits
    assert dump_bytes(books[1]) == before
    assert json.loads((tmp_path / "served.jsonl").read_text()) == {
        "command": "wallets.sync", "served_by": "rust"}


@pytest.mark.parametrize("saved", [False, True])
def test_sync_cli_unsupported_capability_handoff_has_no_traffic(tmp_path, saved):
    config, chain, _ = _fixture("empty")
    roots, books = _books(tmp_path, config)
    with serve(chain, "esplora") as url:
        env = _environment(tmp_path, "bitcoinrpc", url)
        if saved:
            conn = connect(books[1])
            conn.execute("INSERT INTO backends(name,kind,url,created_at,updated_at) "
                         "VALUES('saved','bitcoinrpc',?,'fixed','fixed')", (url,))
            config["backend"] = "saved"
            conn.execute("UPDATE wallets SET config_json=?", (json.dumps(config),))
            conn.commit()
            conn.close()
            env["KASSIBER_BACKEND_FIXTURE_KIND"] = "esplora"
        env["KASSIBER_RUST_TEST_SYNC_HANDOFF"] = "1"
        before = dump_bytes(books[1])
        result = subprocess.run([RUST_BIN, *_args(roots[1])], capture_output=True, env=env, timeout=30)
        assert result.returncode == 4 and not result.stdout and not result.stderr
        assert not chain.hits
        assert dump_bytes(books[1]) == before


def test_sync_cli_unsupported_hybrid_selection_falls_back_before_traffic(tmp_path):
    config, chain, _ = _fixture("empty")
    roots, books = _books(tmp_path, config)
    with serve(chain, "esplora") as url:
        env = _environment(tmp_path, "bitcoinrpc", url)
        env["KASSIBER_NO_EGRESS"] = "1"
        before = dump_bytes(books[1])
        result = subprocess.run([sys.executable, str(HYBRID), *_args(roots[1])],
                                capture_output=True, env=env, timeout=30)
        assert result.returncode == 1
        assert json.loads((tmp_path / "served.jsonl").read_text()) == {
            "command": "wallets.sync", "served_by": "python"}
        assert not chain.hits
        assert dump_bytes(books[1]) == before


@pytest.mark.parametrize("unsupported", [False, True])
def test_sync_cli_capability_errors_are_not_handoff(tmp_path, unsupported):
    config, _, _ = _fixture("empty")
    roots, _ = _books(tmp_path, config)
    env = _environment(tmp_path, "bitcoinrpc" if unsupported else "esplora")
    env["KASSIBER_RUST_TEST_SYNC_HANDOFF"] = "1"
    result = subprocess.run([RUST_BIN, *_args(roots[1]), "--all"],
                            capture_output=True, env=env, timeout=30)
    assert result.returncode == 1
    assert json.loads(result.stdout)["error"]["code"] == "validation"


def test_sync_cli_handoff_requires_the_test_only_opt_in(tmp_path):
    config, _, _ = _fixture("empty")
    roots, _ = _books(tmp_path, config)
    result = subprocess.run([RUST_BIN, *_args(roots[1])], capture_output=True,
                            env=_environment(tmp_path, "bitcoinrpc"), timeout=30)
    assert result.returncode == 1
    assert json.loads(result.stdout)["error"]["code"] == "native_fallback_required"


def test_sync_cli_missing_capability_implementation_fails_closed(tmp_path):
    env = _environment(tmp_path)
    env["KASSIBER_RUST_TEST_FAKE_STORE"] = "{}"
    env["KASSIBER_RUST_TEST_SYNC_HANDOFF"] = "1"
    result = subprocess.run([RUST_BIN, *_args(tmp_path / "absent")],
                            capture_output=True, env=env, timeout=30)
    assert result.returncode == 1
    error = json.loads(result.stdout)["error"]
    assert error["code"] == "not_implemented"
    assert "wallet sync capability" in error["message"]


@pytest.mark.parametrize("stored", ["bad", "[]", "null"])
def test_sync_cli_unreadable_wallet_config_matches_python(tmp_path, stored):
    # tests/rust_port/repro_ai_wallets_sync_bad_config.py: planning refuses with
    # invalid_wallet_config naming the wallet, before any source or traffic.
    # Python's --wallet path still crashes before planning
    # (tests/rust_port/repro_cli_wallet_sync_bad_config_explicit.py).
    config, chain, _ = _fixture("empty")
    roots, books = _books(tmp_path, config)
    for book in books:
        conn = connect(book)
        conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                     "VALUES('bad','w','p','Bad','descriptor',?,'fixed')", (stored,))
        conn.commit()
        conn.close()
    args = ["--all"]
    with serve(chain, "esplora") as url:
        env = _environment(tmp_path, "esplora", url)
        reference = subprocess.run([sys.executable, "-m", "tests.rust_sync_cli_reference",
                                    *_args(roots[0])[:-2], *args], capture_output=True, env=env, timeout=90)
        result = subprocess.run([sys.executable, str(HYBRID), *_args(roots[1])[:-2], *args],
                                capture_output=True, env=env, timeout=90)
        assert (result.returncode, result.stdout, result.stderr) == (
            reference.returncode, reference.stdout, reference.stderr)
        error = json.loads(result.stdout)["error"]
        assert (error["code"], error["details"]) == ("invalid_wallet_config", {"wallet": "Bad"})
        assert not chain.hits
    assert dump_bytes(books[1]) == dump_bytes(books[0])
    assert json.loads((tmp_path / "served.jsonl").read_text()) == {
        "command": "wallets.sync", "served_by": "rust"}
