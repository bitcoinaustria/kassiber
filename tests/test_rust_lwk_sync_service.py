"""Native Liquid wallet sync against Python's complete CLI wallet-sync handler.

Both runtimes sync the same disposable book (plaintext and SQLCipher) from
the LWK chain-state vectors over one loopback Electrum fake. Every table must
match. `chain_observer_values` holds LWK's opaque update bytes, which encode
hash-map iteration order; those rows compare through LWK's own decode.
"""
import copy
import json
import os
from pathlib import Path
import sqlite3
import subprocess

import pytest
from sqlcipher3 import dbapi2 as sqlcipher

from kassiber import __version__
from kassiber.cli import handlers
from kassiber.envelope import build_envelope, build_error_envelope
from kassiber.errors import AppError
from tests.rust_sync_env import child_env
from tests.rust_sync_parity import connect, copy_book, create_book, deterministic_sources, runtime
from tests.test_rust_lwk_execution import VECTORS, POLICY, canonical, electrum
from tests.rust_sync_parity import fixed_elapsed_ms, fixed_elapsed_ms_text
from tests.rust_fixture_binary import fixture_binary

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires the guarded Rust sync service fixture")
TRANSACTIONS = VECTORS["transactions"]


def wallet_config(**extra):
    return {"chain": "liquid", "network": "elementsregtest", "descriptor": VECTORS["descriptor"],
            "gap_limit": 3, "backend": "fixture", "policy_asset": POLICY, **extra}


KEY = "lwk-sync-fixture"


def open_book(path, key=None):
    codec = sqlcipher if key else sqlite3
    conn = codec.connect(str(path))
    if key:
        conn.execute(f"PRAGMA key='{key}'")
    conn.row_factory = codec.Row
    conn.execute("PRAGMA foreign_keys=ON")
    return conn


def dump(tmp_path, path, key=None):
    conn = open_book(path, key)
    try:
        tables = [name for (name,) in conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
        result = {}
        for table in tables:
            cursor = conn.execute(f'SELECT * FROM "{table}"')
            columns = [c[0] for c in cursor.description]
            rows = []
            for row in cursor:
                row = list(row)
                if table == "chain_observer_values":
                    value = columns.index("value")
                    decoded = canonical(tmp_path, {row[columns.index("key")]: row[value]})
                    row[value] = decoded
                # A stored outcome's wall-clock elapsed_ms is checked and fixed.
                rows.append([fixed_elapsed_ms_text(cell) if isinstance(cell, str) and '"elapsed_ms"' in cell else cell
                             for cell in row])
            result[table] = sorted(rows, key=lambda r: json.dumps(r, sort_keys=True))
        return result
    finally:
        conn.close()


def _round(tmp_path, state, url, python_book, rust_book, *, index=0, force_full=False, key=None):
    rt = runtime("electrum", url)
    conn = open_book(python_book, key)
    try:
        with deterministic_sources(f"r{index}", index * 1000):
            try:
                expected = build_envelope("wallets.sync", handlers.sync_wallet(
                    conn, rt, "w", "p", wallet_ref="a", sync_all=False, force_full=force_full))
            except AppError as exc:
                expected = build_error_envelope(exc.code, str(exc), exc.details, exc.hint, exc.retryable)
    finally:
        conn.close()
    payload = {"book": str(rust_book), "profile_id": "p", "wallet": "a", "all": False,
               "backend": rt["backends"]["fixture"], "force_full": force_full, "version": __version__,
               "prefix": f"r{index}", "now": index * 1000, "start_time": 1_700_000_000, "key": key}
    binary = fixture_binary("sync_fixture")
    result = subprocess.run([str(binary)], input=json.dumps(payload).encode(), capture_output=True,
                            timeout=120, env=child_env(tmp_path))
    assert result.returncode == 0 and result.stderr == b"", result.stderr.decode()
    actual = json.loads(result.stdout)
    expected, actual = fixed_elapsed_ms(expected), fixed_elapsed_ms(actual)
    if actual != expected:
        for left, right in zip(expected.get("data", []), actual.get("data", [])):
            for name in sorted(set(left) | set(right)):
                if left.get(name) != right.get(name):
                    print("OUTCOME", name, json.dumps(left.get(name))[:600], "|", json.dumps(right.get(name))[:600])
    assert actual == expected, (expected, actual)
    rust, python = dump(tmp_path, rust_book, key), dump(tmp_path, python_book, key)
    for table in sorted(set(rust) | set(python)):
        if rust.get(table) != python.get(table):
            print("TABLE", table, len(rust.get(table) or []), len(python.get(table) or []))
            for a, b in zip(rust.get(table) or [], python.get(table) or []):
                if a != b:
                    print("  rust  ", json.dumps(a, default=str)[:900])
                    print("  python", json.dumps(b, default=str)[:900])
                    break
    assert rust == python
    return expected


def _books(tmp_path, config, key=None):
    python_book, rust_book = tmp_path / "python.sqlite", tmp_path / "rust.sqlite"
    create_book(python_book, config)
    if key is not None:
        plain = sqlcipher.connect(str(python_book))
        plain.execute(f"ATTACH DATABASE '{tmp_path / 'encrypted.sqlite'}' AS encrypted KEY '{key}'")
        plain.execute("SELECT sqlcipher_export('encrypted')")
        plain.execute("DETACH DATABASE encrypted")
        plain.close()
        (tmp_path / "encrypted.sqlite").replace(python_book)
        assert open_book(python_book, key).execute("SELECT COUNT(*) FROM wallets").fetchone()[0] == 1
    copy_book(python_book, rust_book)
    return python_book, rust_book


# Full-book parity runs on the L-BTC-only chain: on the multi-asset chain
# Python's prospective overlay keeps one record per txid and always widens
# to a forced repair scan, which the native route refuses instead.
SEQUENCES = {
    "fresh": ["lbtc_initial"],
    "empty-then-funded": ["empty", "lbtc_initial"],
    "restart": ["lbtc_initial", "lbtc_initial"],
    "reorg": ["lbtc_initial", "lbtc_reorg"],
    "reorg-restart": ["lbtc_initial", "lbtc_reorg", "lbtc_reorg"],
}


@pytest.mark.parametrize("sequence", sorted(SEQUENCES))
def test_liquid_sync_books_match_python(tmp_path, sequence):
    python_book, rust_book = _books(tmp_path, wallet_config())
    state = {"phase": None}
    with electrum(state) as url:
        for index, phase in enumerate(SEQUENCES[sequence]):
            state["phase"] = VECTORS["phases"][phase]
            outcome = _round(tmp_path, state, url, python_book, rust_book, index=index)
    data = outcome["data"][0]
    assert data["status"] == "synced"
    assert data["observer_route"] == "lwk"
    if sequence.startswith("reorg"):
        assert data.get("retracted", 0) == (1 if sequence == "reorg" else 0)
        conn = connect(python_book)
        retracted = {row[0] for row in conn.execute(
            "SELECT external_id FROM transaction_retractions")} if conn.execute(
            "SELECT 1 FROM sqlite_master WHERE name='transaction_retractions'").fetchone() else set()
        conn.close()
        assert not retracted or TRANSACTIONS["lbtc_mixed"] in retracted


def test_liquid_force_full_rebuild_matches_python(tmp_path):
    python_book, rust_book = _books(tmp_path, wallet_config())
    state = {"phase": VECTORS["phases"]["lbtc_initial"]}
    with electrum(state) as url:
        _round(tmp_path, state, url, python_book, rust_book, index=0)
        state["phase"] = VECTORS["phases"]["lbtc_reorg"]
        _round(tmp_path, state, url, python_book, rust_book, index=1, force_full=True)
        before = dump(tmp_path, rust_book)
        state["phase"] = VECTORS["phases"]["lbtc_lagging"]
        lagging = _round(tmp_path, state, url, python_book, rust_book, index=2, force_full=True)
    # `full_scan_checkpoint` keeps the persisted tip (Python fix d35dbb6d3):
    # a backend behind the book is refused and nothing is retracted.
    assert lagging["error"]["code"] == "backend_tip_behind"
    assert lagging["error"]["details"]["persisted_height"] == VECTORS["phases"]["lbtc_reorg"]["tip"]
    assert dump(tmp_path, rust_book) == before


def test_bitcoin_force_full_rebuild_refuses_a_lagging_backend_like_python(tmp_path):
    from tests.rust_sync_chain import serve
    from tests.test_rust_bdk_execution import bdk_request_clock
    from tests.test_rust_wallet_publication import _fixture

    config, chain, _ = _fixture("checkpoint")
    python_book, rust_book = tmp_path / "python.sqlite", tmp_path / "rust.sqlite"
    create_book(python_book, config)
    copy_book(python_book, rust_book)
    with serve(chain, "electrum") as url:
        rt = runtime("electrum", url)
        outcomes = []
        for index, force_full in enumerate([False, True]):
            if index:
                chain.tip = 5
            conn = open_book(python_book)
            try:
                with deterministic_sources(f"r{index}", index * 1000), bdk_request_clock(1_700_000_000):
                    try:
                        expected = build_envelope("wallets.sync", handlers.sync_wallet(
                            conn, rt, "w", "p", wallet_ref="a", sync_all=False, force_full=force_full))
                    except AppError as exc:
                        expected = build_error_envelope(exc.code, str(exc), exc.details, exc.hint, exc.retryable)
            finally:
                conn.close()
            actual = _native(tmp_path, rust_book, rt["backends"]["fixture"], force_full=force_full,
                             prefix=f"r{index}", now=index * 1000, start_time=1_700_000_000)
            expected, actual = fixed_elapsed_ms(expected), fixed_elapsed_ms(actual)
            assert actual == expected
            assert dump(tmp_path, rust_book) == dump(tmp_path, python_book)
            outcomes.append(expected)
    assert outcomes[0]["data"][0]["status"] == "synced"
    assert outcomes[1]["error"]["code"] == "backend_tip_behind"


def test_liquid_sqlcipher_books_match_python(tmp_path):
    key = KEY
    python_book, rust_book = _books(tmp_path, wallet_config(), key)
    state = {"phase": VECTORS["phases"]["lbtc_initial"]}
    with electrum(state) as url:
        _round(tmp_path, state, url, python_book, rust_book, index=0, key=key)
        state["phase"] = VECTORS["phases"]["lbtc_reorg"]
        _round(tmp_path, state, url, python_book, rust_book, index=1, key=key)


def test_liquid_store_written_by_python_restores_natively(tmp_path):
    # Python syncs first; the native runtime then refreshes the same book.
    python_book, rust_book = _books(tmp_path, wallet_config())
    state = {"phase": VECTORS["phases"]["lbtc_initial"]}
    with electrum(state) as url:
        _round(tmp_path, state, url, python_book, rust_book, index=0)
        copy_book(python_book, rust_book)
        state["phase"] = VECTORS["phases"]["lbtc_reorg"]
        _round(tmp_path, state, url, python_book, rust_book, index=1)


def _native(tmp_path, book, backend, **extra):
    payload = {"book": str(book), "profile_id": "p", "wallet": "a", "all": False, "backend": backend,
               "version": __version__, "prefix": "r0", "now": 0, **extra}
    result = subprocess.run([str(fixture_binary("sync_fixture"))],
                            input=json.dumps(payload).encode(), capture_output=True, timeout=120,
                            env=child_env(tmp_path))
    assert result.returncode == 0 and result.stderr == b"", result.stderr.decode()
    return json.loads(result.stdout)


def test_multi_asset_history_syncs_natively_without_a_repair_scan(tmp_path):
    # The prospective overlay is keyed by (txid, asset) on both sides
    # (Python fix 005d5dfd6): an issued-asset record no longer hides the
    # L-BTC receipt of the same transaction, so nothing widens or refuses.
    python_book, rust_book = _books(tmp_path, wallet_config())
    state = {"phase": None}
    with electrum(state) as url:
        for index, phase in enumerate(["initial", "reorg", "reorg"]):
            state["phase"] = VECTORS["phases"][phase]
            outcome = _round(tmp_path, state, url, python_book, rust_book, index=index)
            data = outcome["data"][0]
            assert data["status"] == "synced"
            assert "negative_balance_rescan" not in data
            assert data["gap_limit"] == 3


# Custom timeouts, proxies and Esplora take Python's Liquid compatibility
# observer natively (tests/test_rust_liquid_compatibility.py).
@pytest.mark.parametrize("case", ["public_view", "unnamed_backend"])
def test_unported_liquid_routes_refuse_before_any_request(tmp_path, case):
    config = wallet_config()
    backend_extra = {}
    if case == "public_view":
        config["descriptor"] = config["descriptor"].replace(
            config["descriptor"].split("slip77(")[1].split(")")[0], "02" + "11" * 32).replace("slip77(", "").replace(
            "02" + "11" * 32 + ")", "02" + "11" * 32, 1)
    elif case == "unnamed_backend":
        config.pop("backend")
    python_book, rust_book = _books(tmp_path, config)
    hits = []
    with electrum({"phase": VECTORS["phases"]["initial"]}, hits) as url:
        rt = runtime("electrum", url)
        backend = {**rt["backends"]["fixture"], **backend_extra}
        payload = {"book": str(rust_book), "profile_id": "p", "wallet": "a", "all": False, "backend": backend,
                   "version": __version__, "prefix": "r0", "now": 0}
        result = subprocess.run([str(fixture_binary("sync_fixture"))],
                                input=json.dumps(payload).encode(), capture_output=True, timeout=60,
                                env=child_env(tmp_path))
    assert result.returncode == 0 and result.stderr == b"", result.stderr.decode()
    actual = json.loads(result.stdout)
    assert hits == []
    assert actual["error"]["code"] in {"app_error", "native_fallback_required", "validation"}


@pytest.mark.parametrize("workspace", [False, True], ids=["profile", "workspace"])
def test_liquid_freshness_run_matches_python(tmp_path, workspace):
    """The native freshness planner takes Liquid wallet sources too."""
    from contextlib import ExitStack
    from types import SimpleNamespace
    from unittest.mock import patch

    from kassiber import daemon_freshness
    from kassiber.core import sync as core_sync
    from tests.rust_sync_parity import CLOCK_MODULES
    from tests.test_rust_freshness_native import CONTEXT_SQL, FIXTURE

    python_book, rust_book = tmp_path / "python.sqlite", tmp_path / "rust.sqlite"
    create_book(python_book, wallet_config(), extra_sql=CONTEXT_SQL)
    copy_book(python_book, rust_book)
    state = {"phase": None}
    with electrum(state) as url:
        rt = runtime("electrum", url)
        for index, phase in enumerate(["lbtc_initial", "lbtc_reorg"]):
            state["phase"] = VECTORS["phases"][phase]
            args = {"rates": False, "journals": False}
            if workspace:
                args["workspace_id"] = "w"
            route = (daemon_freshness._workspace_freshness_run_payload if workspace
                     else daemon_freshness._freshness_run_payload)
            conn = open_book(python_book)
            events = []
            try:
                with deterministic_sources(f"r{index}"), ExitStack() as stack:
                    for module in (*CLOCK_MODULES, daemon_freshness):
                        stack.enter_context(patch.object(module, "now_iso", return_value="2030-01-01T00:00:01Z"))
                    stack.enter_context(patch.object(core_sync, "time", SimpleNamespace(monotonic=lambda: 100.0)))
                    expected = build_envelope("freshness.sync", route(conn, rt, args, progress_observer=events.append))
            finally:
                conn.close()
            payload = {"book": str(rust_book), "backend": rt["backends"]["fixture"], "prefix": f"r{index}",
                       "version": __version__, "force_full": False}
            if workspace:
                payload["workspace_id"] = "w"
            result = subprocess.run([str(Path(FIXTURE).resolve())], input=json.dumps(payload).encode(),
                                    capture_output=True, timeout=120, env=child_env(tmp_path))
            assert result.returncode == 0 and result.stderr == b"", result.stderr.decode()
            actual = json.loads(result.stdout)
            assert json.dumps(fixed_elapsed_ms(actual["envelope"]), sort_keys=True) == json.dumps(fixed_elapsed_ms(expected), sort_keys=True)
            assert json.dumps(fixed_elapsed_ms(actual["events"]), sort_keys=True) == json.dumps(fixed_elapsed_ms(events), sort_keys=True)
            assert dump(tmp_path, rust_book) == dump(tmp_path, python_book)
    data = expected["data"]
    completed = [job for book in data["books"] for job in book["completed"]] if workspace else data["completed"]
    assert completed and all(job["status"] != "error" for job in completed)


def test_liquid_negative_balance_repair_matches_python(tmp_path):
    # An unfunded stored L-BTC outbound is a prospective deficit, so Python
    # rebuilds the LWK observer at the widened gap (500) before publishing;
    # the native route runs the same repair (rust-native-wallet-gaps.md, R1).
    python_book, rust_book = _books(tmp_path, wallet_config())
    for book in (python_book, rust_book):
        conn = connect(book)
        conn.execute(
            "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,"
            "direction,asset,amount,fee,raw_json,created_at) VALUES('manual','w','p','a','manual-1','f',"
            "'2023-01-01T00:00:00Z','outbound','LBTC',100000000000000,0,'{}','fixed')")
        conn.commit()
        conn.close()
    state = {"phase": VECTORS["phases"]["lbtc_initial"]}
    with electrum(state) as url:
        outcome = _round(tmp_path, state, url, python_book, rust_book)
    rescan = outcome["data"][0]["negative_balance_rescan"]
    assert (rescan["triggered"], rescan["rescan_gap_limit"]) == (True, 500)
