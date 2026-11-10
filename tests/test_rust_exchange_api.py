"""Exchange API sync parity: exact envelopes, request sequences and every
schema table, plaintext and SQLCipher, over signed loopback fakes."""
from contextlib import ExitStack
import copy
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
from unittest.mock import patch
import urllib.parse
import uuid

import pytest

from kassiber.cli import handlers
from kassiber.core import imports, wallets, output_inventory
from kassiber.envelope import build_envelope, build_error_envelope
from kassiber.errors import AppError
from kassiber.secrets import sqlcipher
from tests.rust_port.exchange_api import KEY, Binance, Coinbase, Kraken, serve
from tests.rust_sync_env import child_env
from tests.test_rust_store_migrations import normalized_dump

ROOT = Path(__file__).resolve().parents[1]
TIME = "2026-11-01T12:00:00Z"
PASSPHRASE = "synthetic-exchange-book"
MANIFEST = json.loads((ROOT / "crates/store/fixtures/current-schema.json").read_text())
TABLES = sum(1 for obj in MANIFEST["objects"] if obj[0] == "table")
EXCHANGES = {"kraken": Kraken, "coinbase": Coinbase, "binance": Binance}
THROTTLED = (429, {"Retry-After": "0"}, b"{}")


@pytest.fixture(scope="module")
def driver():
    subprocess.run(["cargo", "build", "--offline", "--locked", "-p", "kassiber-wallets", "--features", "test-harness",
                    "--example", "exchange_fixture"], cwd=ROOT, check=True, timeout=1800)
    return Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")) / "debug/examples/exchange_fixture"


def connect(path, encrypted):
    conn = sqlcipher.require_sqlcipher().connect(str(path)) if encrypted else sqlite3.connect(path)
    if encrypted:
        conn.execute("PRAGMA key=" + sqlcipher.escape_passphrase(PASSPHRASE))
    conn.row_factory = sqlcipher.require_sqlcipher().Row if encrypted else sqlite3.Row
    conn.execute("PRAGMA foreign_keys=ON")
    return conn


def books(tmp_path, encrypted, wallet_kind="custom"):
    paths = [tmp_path / name for name in ("python.sqlite3", "rust.sqlite3")]
    conn = connect(paths[0], encrypted)
    conn.execute("BEGIN")
    for kind in ("table", "index", "view", "trigger"):
        for obj in MANIFEST["objects"]:
            if obj[0] == kind and obj[3]:
                conn.execute(obj[3])
    conn.executescript("""
        INSERT INTO chain_index_clock VALUES(1,0);
        INSERT INTO settings(key,value) VALUES('database_instance_id','synthetic-book');
        INSERT INTO workspaces(id,label,created_at) VALUES('ws','Workspace','fixed');
        INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('p','ws','Book','EUR','fixed');
        INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,asset,created_at) VALUES('treasury','ws','p','treasury','Treasury','asset','BTC','fixed');
        INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('wallet','ws','p','Source','custom','{}','fixed');
    """)
    conn.execute("UPDATE wallets SET kind=?", (wallet_kind,))
    conn.commit()
    assert len(normalized_dump(conn)[1]) == TABLES
    conn.close()
    shutil.copyfile(*paths)
    return paths


def reference(paths, encrypted, exchange, backend, wallet):
    identities = []

    def new_id():
        result = str(uuid.UUID(int=100 + len(identities)))
        identities.append(result)
        return result

    conn = connect(paths[0], encrypted)
    with ExitStack() as stack:
        stack.enter_context(patch.object(wallets.uuid, "uuid4", side_effect=new_id))
        for module in (imports, wallets, handlers, output_inventory):
            stack.enter_context(patch.object(module, "now_iso", return_value=TIME))
        try:
            payload = handlers.import_exchange_api(conn, {"backends": {"fixture": backend}}, "ws", "p", "fixture",
                                                   wallet or None, expected_backend_kind=exchange.kind)
            expected = build_envelope("wallets.sync-" + exchange.kind, payload)
        except AppError as error:
            conn.rollback()
            expected = build_error_envelope(error.code, str(error), details=error.details, hint=error.hint,
                                            retryable=error.retryable)
        except Exception as error:  # noqa: BLE001 - the CLI's internal_error wrapping
            conn.rollback()
            expected = build_error_envelope("internal_error", str(error) or type(error).__name__)
    dump = normalized_dump(conn)
    conn.close()
    return expected, dump, identities


def roundtrip(driver, tmp_path, paths, encrypted, exchange, backend, wallet, *, seed=False, exact=True):
    if seed:
        for path in paths:
            conn = connect(path, encrypted)
            conn.executescript("""
                UPDATE transactions SET excluded=1;
                INSERT OR IGNORE INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES('review','ws','p','review','Review','fixed');
                INSERT OR IGNORE INTO transaction_tags(transaction_id,tag_id) SELECT id,'review' FROM transactions;
            """)
            conn.commit()
            conn.close()
    exchange.reset()
    expected, expected_dump, identities = reference(paths, encrypted, exchange, backend, wallet)
    requests = list(exchange.requests)
    exchange.reset()
    request = {"book": str(paths[1]), "backend": backend, "wallet": wallet, "ids": identities,
               "timestamp": TIME, "expected": exchange.kind}
    if encrypted:
        request["passphrase"] = PASSPHRASE
    result = subprocess.run([str(driver)], input=json.dumps(request).encode(), capture_output=True,
                            env=child_env(tmp_path), timeout=120)
    assert result.returncode == 0 and result.stderr == b"", result.stderr.decode()
    if exact:
        assert result.stdout.decode() == json.dumps(expected, indent=2) + "\n"
    else:
        assert json.loads(result.stdout)["kind"] == expected["kind"] == "error", result.stdout
    assert exchange.requests == requests
    actual = connect(paths[1], encrypted)
    actual_dump = normalized_dump(actual)
    actual.close()
    assert len(actual_dump[1]) == TABLES
    for table in expected_dump[1]:
        assert actual_dump[1][table] == expected_dump[1][table], table
    assert actual_dump == expected_dump
    return expected, requests


def mutate(exchange):
    if exchange.kind == "kraken":
        exchange.trades["Z"].update(cost="410.00", price="41000.00")
    elif exchange.kind == "coinbase":
        exchange.transactions["btc-wallet"][0]["native_amount"]["amount"] = "-410.00"
    else:
        exchange.histories["/sapi/v1/fiat/payments"]["data"][0]["sourceAmount"] = "-410.5"


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
@pytest.mark.parametrize("wallet", ["", "Source"], ids=["autocreate", "existing"])
@pytest.mark.parametrize("kind", EXCHANGES)
def test_signed_pages_import_dedupe_update_and_review_state_parity(driver, tmp_path, kind, encrypted, wallet):
    # An existing wallet of the provider's kind; Coinbase network objects are
    # payment metadata only there (see the custom-wallet refusal below).
    paths = books(tmp_path, encrypted, kind)
    exchange = EXCHANGES[kind]()
    exchange.faults[0] = THROTTLED
    with serve(exchange) as backend:
        first, requests = roundtrip(driver, tmp_path, paths, encrypted, exchange, backend, wallet)
        assert first["kind"] == "wallets.sync-" + kind, first
        assert first["data"]["imported"] == first["data"]["fetched"] > 0
        assert len({json.dumps(r) for r in requests}) > 2
        exchange.faults.clear()
        second, _ = roundtrip(driver, tmp_path, paths, encrypted, exchange, backend, wallet, seed=True)
        assert second["data"]["imported"] == 0, second
        mutate(exchange)
        third, _ = roundtrip(driver, tmp_path, paths, encrypted, exchange, backend, wallet)
        assert third["data"]["updated"] >= 1, third


def kraken_paging(exchange):
    exchange.page = 3
    exchange.counts["ledger"] = "7"


def kraken_count_garbage(exchange):
    exchange.counts["trades"] = "many"
    exchange.counts["ledger"] = None


FAILURES = {
    "kraken": {
        "missing-trade": lambda e: e.trades.clear(),
        "cross-asset": lambda e: e.trades["Z"].update(pair="XBTETH"),
        "empty-quote": lambda e: e.trades["Z"].update(pair="XXBT"),
        "unsupported": lambda e: e.ledgers["Z"].update(type="Rollover"),
        "permission": lambda e: setattr(e, "always", (200, {}, b'{"error":["EGeneral:Permission denied"]}')),
        "error-string": lambda e: setattr(e, "always", (200, {}, b'{"error":"EAPI:Rate limit"}')),
        "error-number": lambda e: setattr(e, "always", (200, {}, b'{"error":5}')),
        "error-string-echo": lambda e: setattr(e, "always", (200, {}, json.dumps({"error": f"EAPI:Invalid key {KEY}"}).encode())),
        "error-list-echo": lambda e: setattr(e, "always", (200, {}, json.dumps({"error": [f"bad {KEY}", e.secret_for_backend()]}).encode())),
        "no-result": lambda e: setattr(e, "always", (200, {}, b'{"error":[],"result":[]}')),
        "page-not-object": lambda e: setattr(e, "always", (200, {}, b'{"error":[],"result":{"trades":[]}}')),
        "count-infinity": lambda e: setattr(e, "always", (200, {}, b'{"error":[],"result":{"trades":{"x":{}},"count":Infinity}}')),
        "paging": kraken_paging,
        "count-garbage": kraken_count_garbage,
    },
    "coinbase": {
        "unsupported": lambda e: e.transactions["btc-wallet"][4].update(amount={"amount": "1"}),
        "trade-without-fiat": lambda e: e.transactions["btc-wallet"][1]["native_amount"].update(currency="USDC"),
        "trade-below-cent": lambda e: e.transactions["btc-wallet"][1]["native_amount"].update(amount="-0.009"),
        "not-object": lambda e: setattr(e, "always", (200, {}, b"[]")),
        "data-not-list": lambda e: setattr(e, "always", (200, {}, b'{"data":{}}')),
        "nan-amount": lambda e: e.transactions["btc-wallet"][2]["amount"].update(amount="NaN"),
        "snan-amount": lambda e: e.transactions["btc-wallet"][2]["amount"].update(amount="sNaN"),
    },
    "binance": {
        "fiat-list": lambda e: e.histories.update({"/sapi/v1/fiat/payments": []}),
        "fiat-currency": lambda e: e.histories["/sapi/v1/fiat/payments"]["data"][0].update(fiatCurrency="USDT"),
        "deposits-number": lambda e: e.histories.update({"/sapi/v1/capital/deposit/hisrec": 7}),
        "deposits-dict": lambda e: e.histories.update({"/sapi/v1/capital/deposit/hisrec": {"code": 1}}),
        "date-string": lambda e: e.histories["/sapi/v1/capital/withdraw/history"][0].update(applyTime="2023-11-14 22:13:20"),
        "dividends-none": lambda e: e.histories.update({"/sapi/v1/asset/assetDividend": {"rows": None}}),
    },
}
COMMON = {
    "throttled": lambda e: setattr(e, "always", THROTTLED),
    "unavailable": lambda e: setattr(e, "always", (503, {"Retry-After": "0"}, b"{}")),
    "server-error": lambda e: setattr(e, "always", (500, {}, b'{"msg":"tx ' + b"ab" * 32 + b' failed"}')),
    "not-json": lambda e: setattr(e, "always", (200, {}, b"{not json")),
    "not-utf8": lambda e: setattr(e, "always", (200, {}, b'{"a":"\xff"}')),
    "bom": lambda e: setattr(e, "always", (200, {}, b'\xef\xbb\xbf{}')),
    "late-throttle": lambda e: e.faults.update({1: THROTTLED, 2: THROTTLED, 3: THROTTLED}),
    # Echoed credentials straddling the 200-character cut, and escaped forms.
    "echo-at-cut": lambda e: setattr(e, "always", (400, {}, b"x" * 195 + KEY.encode() + b" tail")),
    "echo-escaped": lambda e: setattr(e, "always", (401, {}, json.dumps(
        {"msg": f"{urllib.parse.quote(e.secret_for_backend(), safe='')} {json.dumps(KEY)}"}).encode())),
}
BACKENDS = {
    "missing-key": {"token": None},
    "missing-secret": {"auth_header": "  "},
    "wrong-key": {"token": "someone-else", "api_key": "ignored"},
    "secret-fallback": {"auth_header": None, "api_secret": None, "password": "wrong"},
    "kind-mismatch": {"kind": "Binance"},
    "not-exchange": {"kind": "esplora"},
    "bad-timeout": {"timeout": "soon"},
}
CASES = [(kind, name) for kind in EXCHANGES for name in [*FAILURES[kind], *COMMON, *BACKENDS]]


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
@pytest.mark.parametrize(("kind", "case"), CASES, ids=[f"{k}-{c}" for k, c in CASES])
def test_failures_and_edge_pages_keep_exact_envelopes_requests_and_books(driver, tmp_path, kind, case, encrypted):
    paths = books(tmp_path, encrypted)
    exchange = EXCHANGES[kind]()
    script = {**FAILURES[kind], **COMMON}.get(case)
    if script:
        script(exchange)
    with serve(exchange, **BACKENDS.get(case, {})) as backend:
        backend = {key: value for key, value in backend.items() if value is not None}
        if case == "kind-mismatch" and kind == "binance":
            backend["kind"] = "kraken"
        roundtrip(driver, tmp_path, paths, encrypted, exchange, backend, "")


@pytest.mark.parametrize("kind", EXCHANGES)
def test_redirects_are_refused_without_following_or_book_effects(driver, tmp_path, kind):
    """Python's opener reports `HTTP 302 from backend`; the shared Rust
    transport refuses the redirect itself. Neither follows it or writes."""
    paths = books(tmp_path, False)
    exchange = EXCHANGES[kind]()
    exchange.always = (302, {"Location": "/elsewhere"}, b"")
    with serve(exchange) as backend:
        roundtrip(driver, tmp_path, paths, False, exchange, backend, "", exact=False)
    assert all(request["path"] != "/elsewhere" for request in exchange.requests)


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
def test_coinbase_network_objects_refuse_a_custom_wallet_like_python(driver, tmp_path, encrypted):
    paths = books(tmp_path, encrypted)
    exchange = Coinbase()
    with serve(exchange) as backend:
        expected, _ = roundtrip(driver, tmp_path, paths, encrypted, exchange, backend, "Source")
    assert expected["error"]["code"] == "book_network_mismatch", expected


def binance_rail(exchange, deposit, withdrawal):
    exchange.histories["/sapi/v1/capital/deposit/hisrec"][0]["network"] = deposit
    exchange.histories["/sapi/v1/capital/withdraw/history"][0]["network"] = withdrawal


def bind(paths, encrypted, environment):
    instance = "00000000-0000-0000-0000-000000000001" if environment == "regtest" else None
    for path in paths:
        conn = connect(path, encrypted)
        conn.execute(
            "INSERT INTO book_network_bindings(profile_id,environment_id,environment,revision,chain_instance_id,"
            "domains_json,acknowledgements_json,inventory_digest,created_at) VALUES('p','env',?,1,?,'[]','[]','x','fixed')",
            (environment, instance),
        )
        conn.commit()
        conn.close()


BINANCE_RAILS = {
    # Binance names the coin's transfer rail: "BTC" is Bitcoin on chain.
    "btc-binance-wallet": ("binance", None, "BTC", "BTC", "wallets.sync-binance"),
    "btc-mainnet-book": ("binance", "main", "BTC", "BTC", "wallets.sync-binance"),
    "btc-regtest-book": ("binance", "regtest", "BTC", "BTC", "error"),
    "btc-custom-wallet": ("custom", None, "BTC", "BTC", "error"),
    "lightning": ("binance", None, "BTC", "LIGHTNING", "error"),
    "token-rail": ("binance", None, "BSC", "BTC", "error"),
    "lowercase": ("binance", None, "btc", "BTC", "error"),
}


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
@pytest.mark.parametrize("case", BINANCE_RAILS)
def test_binance_network_rails_admit_and_refuse_like_python(driver, tmp_path, case, encrypted):
    wallet_kind, environment, deposit, withdrawal, kind = BINANCE_RAILS[case]
    paths = books(tmp_path, encrypted, wallet_kind)
    if environment:
        bind(paths, encrypted, environment)
    exchange = Binance()
    binance_rail(exchange, deposit, withdrawal)
    with serve(exchange) as backend:
        expected, _ = roundtrip(driver, tmp_path, paths, encrypted, exchange, backend, "Source")
    assert expected["kind"] == kind, expected
    if kind == "error":
        assert expected["error"]["code"] == "book_network_mismatch", expected
    else:
        assert expected["data"]["imported"] == expected["data"]["fetched"] > 0, expected


def infinite_behind_guard(exchange):
    exchange.transactions["btc-wallet"][1]["amount"]["amount"] = "Infinity"


def infinite_before_price(exchange):
    exchange.ledgers["Z"]["amount"] = "Infinity"
    exchange.trades["Z"]["pair"] = "XXBTZUSD"


@pytest.mark.parametrize(("kind", "script", "code"), [
    ("coinbase", infinite_behind_guard, "internal_error"),
    ("kraken", infinite_before_price, "internal_error"),
], ids=["behind-network-guard", "before-price-currency"])
def test_non_finite_amounts_fail_where_python_converts_them(driver, tmp_path, kind, script, code):
    """Python converts every record's amount to msat before the network
    guard (5a8a786c2), so a non-finite amount fails as Python's OverflowError
    ahead of both the network guard and the row's price-currency check."""
    paths = books(tmp_path, False)
    exchange = EXCHANGES[kind]()
    script(exchange)
    with serve(exchange) as backend:
        expected, _ = roundtrip(driver, tmp_path, paths, False, exchange, backend, "Source")
    assert expected["error"]["code"] == code, expected


def kraken_secret(value):
    return {"auth_header": value}


SECRETS = ["!!" + "c3ludGhldGljLWV4Y2hhbmdlLXNlY3JldA==", "c3ludGhldGljLWV4Y2hhbmdlLXNlY3JldA", "c3ludGhldGljLWV4Y2hhbmdlLXNlY3JldA=x=",
           "ABC", "A===", "c3ludGhldGljLWV4Y2hhbmdlLXNlY3JldA==\n", "é"]


@pytest.mark.parametrize("secret", SECRETS)
def test_kraken_secret_decoding_matches_binascii(driver, tmp_path, secret):
    paths = books(tmp_path, False)
    exchange = Kraken()
    with serve(exchange, auth_header=secret) as backend:
        roundtrip(driver, tmp_path, paths, False, exchange, backend, "Source")


VALUES = ["NaN", "-NaN", "sNaN", "NaN12", "Infinity", "-inf", "1_000", "1__0", "in_f", "١٢", " 0.1 ", "1e400",
          True, False, 0, [], {"a": 1}, 1e300, -0.0, "-0", "abc", "\ud800", 12345678901234567890123456789012345]
TIMES = ["abc", 1e20, -1e20, 253402300800, 253402300799.9999999, -62135596800, -62135596801, 9.3e18, 1e17, 6.7e16,
         "NaN", "sNaN", "Infinity", 0, False, "", None, [], "١"]


def kraken_value(field, value):
    def apply(exchange):
        exchange.ledgers["W"][field] = value
    return apply


def coinbase_value(field, value):
    def apply(exchange):
        tx = exchange.transactions["btc-wallet"][1]
        if field == "time":
            tx["created_at"] = value
        else:
            tx[field]["amount"] = value
    return apply


def binance_value(field, value):
    def apply(exchange):
        row = exchange.histories["/sapi/v1/fiat/payments"]["data"][0]
        row[{"time": "createTime", "amount": "obtainAmount", "fiat": "sourceAmount"}[field]] = value
    return apply


VECTORS = (
    [("kraken", f"amount-{i}", kraken_value("amount", v)) for i, v in enumerate(VALUES)]
    + [("kraken", f"fee-{i}", kraken_value("fee", v)) for i, v in enumerate(VALUES)]
    + [("kraken", f"time-{i}", kraken_value("time", v)) for i, v in enumerate(TIMES)]
    + [("coinbase", f"amount-{i}", coinbase_value("amount", v)) for i, v in enumerate(VALUES)]
    + [("coinbase", f"native-{i}", coinbase_value("native_amount", v)) for i, v in enumerate(VALUES)]
    + [("coinbase", f"time-{i}", coinbase_value("time", v)) for i, v in enumerate(TIMES)]
    + [("binance", f"amount-{i}", binance_value("amount", v)) for i, v in enumerate(VALUES)]
    + [("binance", f"fiat-{i}", binance_value("fiat", v)) for i, v in enumerate(VALUES)]
    + [("binance", f"time-{i}", binance_value("time", v)) for i, v in enumerate(TIMES)]
)


@pytest.mark.parametrize(("kind", "name", "script"), VECTORS, ids=[f"{k}-{n}" for k, n, _ in VECTORS])
def test_hostile_field_values_match_python_exactly(driver, tmp_path, kind, name, script):
    paths = books(tmp_path, False, kind)
    exchange = EXCHANGES[kind]()
    script(exchange)
    with serve(exchange) as backend:
        roundtrip(driver, tmp_path, paths, False, exchange, backend, "Source")


@pytest.fixture(scope="module")
def cli_binary():
    subprocess.run(["cargo", "build", "--offline", "--locked", "-p", "kassiber", "--features", "test-harness"],
                   cwd=ROOT, check=True, timeout=3600)
    return Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")) / "debug/kassiber"


@pytest.mark.parametrize("kind", EXCHANGES)
def test_registered_cli_serves_every_exchange_natively_with_identical_books(cli_binary, tmp_path, kind):
    from kassiber import __version__
    from kassiber.db import open_db, set_setting
    from kassiber.core import accounts
    exchange = EXCHANGES[kind]()
    with serve(exchange) as backend:
        roots = [tmp_path / name for name in ("python", "rust")]
        for root in roots:
            root.mkdir()
        conn = open_db(roots[0])
        conn.executescript("""
            INSERT INTO workspaces(id,label,created_at) VALUES('ws','Workspace','fixed');
            INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('p','ws','Book','EUR','fixed');
        """)
        accounts.ensure_default_accounts(conn, "ws", "p")
        accounts.create_backend(conn, "fixture", kind, backend["url"], token=backend["token"],
                                auth_header=backend["auth_header"])
        set_setting(conn, "app_version", __version__)
        set_setting(conn, "context_workspace", "ws")
        set_setting(conn, "context_profile", "p")
        conn.commit()
        conn.close()
        shutil.copytree(roots[0], roots[1], dirs_exist_ok=True)
        paths = [root / "kassiber.sqlite3" for root in roots]
        log = tmp_path / "served.jsonl"
        for attempt in range(2):
            expected, expected_dump, identities = reference(paths, False, exchange, copy.deepcopy(backend), "")
            requests = list(exchange.requests)
            exchange.reset()
            env = child_env(tmp_path)
            env.update({"KASSIBER_RUST_TEST_IMPORT_IDS": ",".join(identities) or "unused",
                        "KASSIBER_RUST_TEST_IMPORT_TIME": TIME,
                        "KASSIBER_RUST_TEST_OWNER_ROOT": str(tmp_path / "owners"), "KASSIBER_HYBRID_RUST_BIN": str(cli_binary),
                        "KASSIBER_HYBRID_LOG": str(log)})
            command = [sys.executable, str(ROOT / "tests/rust_port/hybrid_cli.py"), "--machine", "--data-root",
                       str(roots[1]), "wallets", "sync-" + kind, "--workspace", "ws", "--profile", "p", "--backend", "fixture"]
            result = subprocess.run(command, capture_output=True, env=env, timeout=120)
            assert result.returncode == 0 and result.stderr == b"", result.stderr.decode()
            assert result.stdout.decode() == json.dumps(expected, indent=2) + "\n"
            assert expected["data"]["imported"] == (expected["data"]["fetched"] if attempt == 0 else 0)
            assert exchange.requests == requests
            exchange.reset()
            actual = sqlite3.connect(paths[1])
            assert normalized_dump(actual) == expected_dump
            actual.close()
        served = [json.loads(line) for line in log.read_text().splitlines()]
        assert served == [{"command": "wallets.sync-" + kind, "served_by": "rust"}] * 2
