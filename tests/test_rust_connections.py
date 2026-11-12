"""Connections area: native CLI/daemon bytes, events and every table against unchanged Python.

Books are disposable; every child gets its own XDG roots. Network kinds only
reach loopback fakes, and refusals prove the fakes saw no traffic.
"""
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
import re
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import threading
import time
from urllib.parse import parse_qs, urlencode

import pytest

from kassiber.db import open_db
from tests.rust_port.rust_only import without_rust_only_kinds
from tests.rust_sync_env import child_env
from tests.egress_guard import guarded_pythonpath
from tests.test_rust_cln_acquisition import fake_cli, node as cln_node

ROOT = Path(__file__).resolve().parents[1]
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires the kassiber test-harness build")
MANIFEST = ROOT / "crates/store/fixtures/current-schema.json"
PASSPHRASE = "disposable connections passphrase"
REFERENCE = """import os,sys
from pathlib import Path
from kassiber.operator import project
project._owner_lock_root=lambda:Path(os.environ['KASSIBER_RUST_TEST_OWNER_ROOT'])
from kassiber.cli.main import main
raise SystemExit(main(sys.argv[1:]))
"""


def manifest_tables():
    manifest = json.loads(MANIFEST.read_text())
    return sorted(obj[1] for obj in manifest["objects"] if obj[0] == "table")


def connect(root, encrypted=False):
    database = Path(root) / "kassiber.sqlite3"
    if encrypted:
        from kassiber.secrets.sqlcipher import open_encrypted
        return open_encrypted(database, PASSPHRASE, enforce_operator_identity=False)
    return sqlite3.connect(database)


def dump(root, encrypted=False):
    """Every manifest table's rows, order-independent; the book must hold exactly those tables."""
    conn = connect(root, encrypted)
    try:
        present = sorted(name for (name,) in conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'"))
        assert present == manifest_tables()
        return {table: sorted((list(row) for row in conn.execute(f'SELECT * FROM "{table}"')),
                              key=lambda row: json.dumps(row, sort_keys=True, default=repr))
                for table in present}
    finally:
        conn.close()


def environment(tmp_path):
    env = child_env(tmp_path)
    owners = tmp_path / "owners"
    owners.mkdir(mode=0o700, exist_ok=True)
    env.update({
        "KASSIBER_TEST_NO_EGRESS": "1",
        "KASSIBER_RUST_TEST_OWNER_ROOT": str(owners),
        "KASSIBER_HYBRID_RUST_BIN": str(Path(RUST_BIN).resolve()),
        "KASSIBER_HYBRID_RUST_DAEMON": "1",
        "KASSIBER_HYBRID_LOG": str(tmp_path / "served.jsonl"),
        "PYTHONPATH": guarded_pythonpath(),
    })
    # A child must never resolve the account's real app-data root.
    assert str(tmp_path) in env["XDG_DATA_HOME"]
    return env


def run(root, argv, env, *, reference=False, wire=None, passphrase=False):
    prefix = [sys.executable, "-c", REFERENCE] if reference else [sys.executable, str(ROOT / "tests/rust_port/hybrid_cli.py")]
    read_fd = None
    unlock = []
    if passphrase:
        read_fd, write_fd = os.pipe()
        os.write(write_fd, PASSPHRASE.encode())
        os.close(write_fd)
        unlock = ["--db-passphrase-fd", str(read_fd)]
    try:
        return subprocess.run([*prefix, "--data-root", str(root), *unlock, *argv], input=wire, capture_output=True,
                              env=env, cwd=ROOT, timeout=600, pass_fds=() if read_fd is None else (read_fd,))
    finally:
        if read_fd is not None:
            os.close(read_fd)


def served(env):
    path = Path(env["KASSIBER_HYBRID_LOG"])
    return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []


def encrypt(root, env):
    read_fd, write_fd = os.pipe()
    os.write(write_fd, PASSPHRASE.encode())
    os.close(write_fd)
    try:
        result = subprocess.run([sys.executable, "-c", REFERENCE, "--machine", "--data-root", str(root), "secrets", "init",
                                 "--new-passphrase-fd", str(read_fd)], pass_fds=(read_fd,), capture_output=True,
                                env=env, cwd=ROOT, timeout=600)
    finally:
        os.close(read_fd)
    assert result.returncode == 0, result.stdout
    for leftover in Path(root).glob("kassiber.pre-enc*"):
        leftover.unlink()


def wire(requests):
    lines = [json.dumps({"request_id": str(i), "kind": kind, **({} if args is None else {"args": args})})
             for i, (kind, args) in enumerate(requests)]
    lines.append(json.dumps({"request_id": "stop", "kind": "daemon.shutdown"}))
    return ("\n".join(lines) + "\n").encode()


def ready_kinds(stdout):
    return json.loads(stdout.splitlines()[0])["data"]["supported_kinds"]


# --- Lightning node snapshot ----------------------------------------------------------------


@contextmanager
def lnd_node(case):
    """A loopback LND REST fake answering each path with the reference call's response."""
    responses = {}
    for call in case["calls"]:
        path = call["path"] + ("?" + urlencode(call["params"]) if call["params"] else "")
        responses[path] = call["response"]
    hits = []

    class Handler(BaseHTTPRequestHandler):
        def respond(self):
            self.rfile.read(int(self.headers.get("Content-Length", "0")))
            hits.append(f"{self.command} {self.path}")
            body = json.dumps(responses.get(self.path, {"node": {}})).encode()
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        do_GET = respond
        do_POST = respond

        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=lambda: server.serve_forever(poll_interval=0.01))
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}", hits
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


def lightning_cases(kind):
    name = "python_cln.json" if kind == "coreln" else "python_lightning.json"
    cases = json.loads((ROOT / "crates/chain/fixtures" / name).read_text())
    good = [c for c in cases if "snapshot" in c and not c.get("failures")]
    picks = [good[0], good[len(good) // 2]] + [c for c in good if c["name"].startswith("snapshot-")][-2:]
    return picks


@contextmanager
def lightning_node(tmp_path, kind, case):
    if kind == "coreln":
        with cln_node(case["data"]) as (url, hits):
            yield {"url": "cln://local", "rpc_file": url, "lightning_cli": str(fake_cli(tmp_path))}, hits
    else:
        with lnd_node(case) as (url, hits):
            yield {"url": url, "token": "aabb"}, hits


def lightning_book(tmp_path, kind, backend):
    root = tmp_path / "data"
    conn = open_db(root)
    conn.executescript((ROOT / "crates/reports/tests/data/book.sql").read_text())
    conn.execute("UPDATE wallets SET kind=?,config_json=? WHERE id='cold'", (kind, json.dumps({"backend": "fake"})))
    extra = {k: v for k, v in backend.items() if k not in {"url", "token"}}
    conn.execute("INSERT INTO backends(name,kind,url,token,timeout,config_json,created_at,updated_at) "
                 "VALUES('fake',?,?,?,2,?,'fixed','fixed')", (kind, backend["url"], backend.get("token", ""), json.dumps(extra)))
    conn.commit()
    conn.close()
    # Let the reference finish its own open-time bookkeeping before comparing.
    open_db(root).close()
    env = environment(tmp_path)
    if kind == "coreln":
        env["KASSIBER_RUST_TEST_CLN_FIXTURE"] = backend["lightning_cli"]
    return root, env


SNAPSHOT_ARGS = [dict(connection="cold", window_days=w) for w in [None, 1, 365, "7", "bad", True, -1, 900]] + [
    dict(wallet="Cold"), dict(label="cold", workspace="Main", profile="Book"), dict(connection="cold", workspace=1),
    dict(connection="missing"), {}, [], None, "text",
]


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
@pytest.mark.parametrize("kind", ["lnd", "coreln"])
@pytest.mark.parametrize("case_index", [0, 1, 2, 3])
def test_node_snapshot_daemon_bytes_all_tables_and_zero_fallback(tmp_path, kind, case_index, encrypted):
    if encrypted and case_index:
        pytest.skip("one encrypted session per adapter")
    case = lightning_cases(kind)[case_index]
    with lightning_node(tmp_path, kind, case) as (backend, hits):
        root, env = lightning_book(tmp_path, kind, backend)
        if encrypted:
            encrypt(root, env)
        before = dump(root, encrypted)
        requests = [("ui.connections.node.snapshot", args) for args in SNAPSHOT_ARGS]
        if encrypted:
            requests = [requests[0], ("daemon.unlock", {"auth_response": {"passphrase_secret": PASSPHRASE}}), *requests,
                        ("daemon.lock", {}), requests[0]]
        sessions = []
        for reference in [True, False]:
            hits.clear()
            result = run(root, ["daemon"], env, reference=reference, wire=wire(requests))
            assert (result.returncode, result.stderr) == (0, b""), result.stderr
            assert dump(root, encrypted) == before
            lines = result.stdout.replace(str(root).encode(), b"ROOT").splitlines()
            sessions.append((lines[1:], list(hits), ready_kinds(result.stdout)))
        assert sessions[1][:2] == sessions[0][:2]
        python_kinds, rust_kinds = sessions[0][2], sessions[1][2]
        assert "ui.connections.node.snapshot" in rust_kinds
        assert [k for k in rust_kinds if k in python_kinds] == [k for k in python_kinds if k in rust_kinds]
        responses = [json.loads(line) for line in sessions[1][0]]
        snapshots = [r for r in responses if r.get("kind") == "ui.connections.node.snapshot"]
        assert snapshots and all(r["data"]["connection"]["id"] == "cold" for r in snapshots)
        assert all("not_implemented" not in line.decode() for line in sessions[1][0])
        assert sessions[1][1], "the snapshot must reach the configured node"
    assert served(env) == [{"command": "daemon", "served_by": "rust"}]


@pytest.mark.parametrize("kind", ["lnd", "coreln"])
@pytest.mark.parametrize("switch", ["offline", "no-egress"])
def test_node_snapshot_refuses_before_any_node_traffic(tmp_path, kind, switch):
    case = lightning_cases(kind)[0]
    with lightning_node(tmp_path, kind, case) as (backend, hits):
        root, env = lightning_book(tmp_path, kind, backend)
        if switch == "offline":
            Path(env["KASSIBER_OFFLINE_PREFERENCE_FILE"]).write_text('{"enabled":true}')
        else:
            env["KASSIBER_NO_EGRESS"] = "1"
        before = dump(root)
        requests = [("ui.connections.node.snapshot", {"connection": "cold"})]
        # Python's CLN subprocess ignores both switches (a documented reference
        # difference); the native process route refuses before spawning it.
        references = [False] if kind == "coreln" else [True, False]
        outputs = []
        for reference in references:
            hits.clear()
            result = run(root, ["daemon"], env, reference=reference, wire=wire(requests))
            assert result.returncode == 0
            outputs.append(result.stdout.splitlines()[1:])
            assert hits == []
        assert outputs[0] == outputs[-1]
        assert json.loads(outputs[-1][0])["error"]["code"] == "network_egress_disabled"
        assert dump(root) == before


def test_node_snapshot_display_reads_make_no_node_traffic(tmp_path):
    """Opening Connections (sources, wallet and backend lists, status) never contacts the node."""
    case = lightning_cases("lnd")[0]
    with lightning_node(tmp_path, "lnd", case) as (backend, hits):
        root, env = lightning_book(tmp_path, "lnd", backend)
        before = dump(root)
        requests = [("status", None), ("ui.connections.sources", {}), ("ui.wallets.list", {}), ("ui.backends.list", {}),
                    ("ui.connections.node.snapshot", {"connection": "missing"})]
        outputs = []
        for reference in [True, False]:
            result = run(root, ["daemon"], env, reference=reference, wire=wire(requests))
            assert (result.returncode, result.stderr) == (0, b"")
            outputs.append(result.stdout.splitlines()[1:])
        assert hits == []
        assert dump(root) == before


# --- Connection sources and static catalogs -----------------------------------------------


def plain_book(tmp_path, encrypted=False):
    root = tmp_path / "data"
    open_db(root).close()
    env = environment(tmp_path)
    if encrypted:
        encrypt(root, env)
    return root, env


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
def test_connection_sources_daemon_bytes_locking_and_all_tables(tmp_path, encrypted):
    root, env = plain_book(tmp_path, encrypted)
    before = dump(root, encrypted)
    requests = [("ui.connections.sources", args) for args in [{}, None, [1], "text", {"unused": True}]]
    if encrypted:
        requests = [requests[0], ("daemon.unlock", {"auth_response": {"passphrase_secret": PASSPHRASE}}), *requests,
                    ("daemon.lock", {}), requests[0]]
    outputs = []
    for reference in [True, False]:
        result = run(root, ["daemon"], env, reference=reference, wire=wire(requests))
        assert (result.returncode, result.stderr) == (0, b""), result.stderr
        assert dump(root, encrypted) == before
        stdout = result.stdout if reference else without_rust_only_kinds(result.stdout)
        outputs.append((stdout.replace(str(root).encode(), b"ROOT").splitlines()[1:], ready_kinds(result.stdout)))
    assert outputs[1][0] == outputs[0][0]
    python_kinds, rust_kinds = outputs[0][1], outputs[1][1]
    assert "ui.connections.sources" in rust_kinds
    assert [k for k in rust_kinds if k in python_kinds] == [k for k in python_kinds if k in rust_kinds]
    first = json.loads(outputs[1][0][2 if encrypted else 0])
    assert first["kind"] == "ui.connections.sources"
    assert first["data"]["source_formats"] == sorted(first["data"]["source_formats"])
    assert served(env) == [{"command": "daemon", "served_by": "rust"}]


@pytest.mark.parametrize("leaf", ["wallets", "backends"])
@pytest.mark.parametrize("flags", [["--machine"], ["--format", "json"], ["--format", "table"], ["--format", "plain"], ["--format", "csv"]])
def test_static_kind_catalogs_match_bytes_and_never_create_a_book(tmp_path, leaf, flags):
    env = environment(tmp_path)
    layouts = []
    for reference in [True, False]:
        root = tmp_path / ("reference" if reference else "native")
        result = run(root, [*flags, leaf, "kinds"], env, reference=reference)
        stdout = result.stdout if reference else without_rust_only_kinds(result.stdout)
        assert reference or b"bitpanda" in result.stdout
        layouts.append((result.returncode, stdout.replace(str(root).encode(), b"ROOT"), result.stderr,
                        sorted(str(p.relative_to(root)) for p in root.rglob("*"))))
        # A read of a static catalog must not create a database.
        assert not (root / "kassiber.sqlite3").exists()
    assert layouts[1] == layouts[0]
    assert served(env) == [{"command": f"{leaf}.kinds", "served_by": "rust"}]


# --- BTCPay: local guides, attachments and reviews ---------------------------------------

SECRET = "synthetic-btcpay-secret"


def btcpay_book(tmp_path, *, encrypted=False, url="http://127.0.0.1:1/prefix/api/v1", config=None, sql=""):
    root = tmp_path / "data"
    conn = open_db(root)
    conn.executescript(f"""
        INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2026-01-01T00:00:00Z');
        INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','w','Book','2026-01-01T00:00:00Z');
        INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,asset,created_at)
        VALUES('acct','w','p','treasury','Treasury','asset','BTC','2026-01-01T00:00:00Z');
        INSERT OR REPLACE INTO settings(key,value) VALUES('context_workspace','w'),('context_profile','p');
    """)
    conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                 "VALUES('a','w','p','Cold','custom',?,'2026-01-01T00:00:00Z')", (json.dumps(config or {}),))
    conn.execute("INSERT INTO backends(name,kind,url,token,created_at,updated_at) "
                 "VALUES('merchant','btcpay',?,?,'fixed','fixed')", (url, SECRET))
    conn.execute("INSERT INTO backends(name,kind,url,created_at,updated_at) "
                 "VALUES('chain','esplora','http://127.0.0.1:1','fixed','fixed')")
    conn.executescript(sql)
    conn.commit()
    conn.close()
    open_db(root).close()
    env = environment(tmp_path)
    if encrypted:
        encrypt(root, env)
    return root, env


def compare_cli(root, env, argv, *, encrypted=False, unchanged=False):
    """Run the reference and the candidate on the same starting bytes; compare output and tables."""
    database = root / "kassiber.sqlite3"
    initial = database.read_bytes()
    before = dump(root, encrypted)
    expected = run(root, ["--machine", *argv], env, reference=True, passphrase=encrypted)
    expected_book = dump(root, encrypted)
    database.write_bytes(initial)
    actual = run(root, ["--machine", *argv], env, passphrase=encrypted)
    assert (actual.returncode, actual.stdout, actual.stderr) == (expected.returncode, expected.stdout, expected.stderr), argv
    assert dump(root, encrypted) == expected_book
    if unchanged or actual.returncode:
        assert expected_book == before
    assert SECRET.encode() not in actual.stdout
    return json.loads(actual.stdout)


@pytest.mark.parametrize("options", [
    [], ["--preset", "wallet_history"], ["--scope", "single"], ["--preset", "wallet_history", "--scope", "all"],
    ["--server-url", "HTTPS://EXAMPLE.invalid:443/shop/stores/s?secret=discard#fragment"],
    ["--server-url", "http://127.0.0.1:1", "--preset", "wallet_history"],
    ["--server-url", "https://merchant.invalid", "--store-id", " one/two ", "--store-id", "三"],
    ["--server-url", "https://merchant.invalid", "--store-id", " ", "--preset", "wallet_history"],
    ["--server-url", "https://[::1]:443/prefix/docs/"], ["--server-url", "missing-scheme.invalid"],
    ["--server-url", "ftp://merchant.invalid"], ["--server-url", "http:///missing"],
    ["--server-url", "https://user:secret@merchant.invalid"], ["--server-url", "http://[broken/"],
    ["--server-url", " "], ["--server-url", ""], ["--backend", ""], ["--server-url", "https://[v1.example]/stores/s"],
    ["--server-url", "https://merchant／invalid/"], ["--preset", "bogus"], ["--scope", "bogus"],
    ["--server-url", "x", "--backend", "y"],
])
def test_key_url_without_book_matches_bytes_and_creates_no_book(tmp_path, options):
    env = environment(tmp_path)
    outputs = []
    for reference in [True, False]:
        root = tmp_path / ("reference" if reference else "native")
        result = run(root, ["--machine", "btcpay", "key-url", *options], env, reference=reference)
        outputs.append((result.returncode, result.stdout.replace(str(root).encode(), b"ROOT"), result.stderr,
                        sorted(str(p.relative_to(root)) for p in root.rglob("*")) if root.exists() else None))
        assert not (root / "kassiber.sqlite3").exists()
    assert outputs[1] == outputs[0]
    assert all(row["served_by"] == "rust" for row in served(env))


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
@pytest.mark.parametrize("options", [["--backend", "merchant"], ["--backend", " MERCHANT "], ["--backend", "chain"],
                                     ["--backend", "missing"], ["--backend", "merchant", "--store-id", "s1"],
                                     ["--backend", "merchant", "--preset", "wallet_history", "--scope", "all"]])
def test_saved_key_url_uses_only_the_saved_url_without_traffic(tmp_path, encrypted, options):
    root, env = btcpay_book(tmp_path, encrypted=encrypted)
    result = compare_cli(root, env, ["btcpay", "key-url", *options], encrypted=encrypted, unchanged=True)
    if result["kind"] == "btcpay.key-url":
        assert result["data"]["server_url"] == "http://127.0.0.1:1/prefix"
    assert served(env) == [{"command": "btcpay.key-url", "served_by": "rust"}]


KEY_GUIDE_ARGS = [
    {}, None, {"backend": "MERCHANT"}, {"backend": " merchant "}, {"backend": "chain"}, {"backend": "missing"},
    {"server_url": "https://example.invalid/api/v1", "preset": "WALLET_HISTORY"},
    {"url": "https://example.invalid/stores/x"}, {"server_url": " ", "url": "https://example.invalid"},
    {"server_url": 1}, {"url": 1}, {"preset": "invalid"}, {"preset": 7},
    {"server_url": "http://127.0.0.1:1", "store_scope": "bad"}, {"store_scope": "bad"},
    {"preset": "invalid", "backend": "missing"}, {"server_url": "https://example.invalid", "backend": 1},
    {"server_url": "https://example.invalid", "backend": "missing"}, {"backend": "merchant", "store_scope": "single"},
    {"server_url": "ftp://x"}, [1], "text",
]


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
def test_key_guide_daemon_bytes_locking_and_all_tables(tmp_path, encrypted):
    root, env = btcpay_book(tmp_path, encrypted=encrypted)
    requests = [("ui.connections.btcpay.key_guide", args) for args in KEY_GUIDE_ARGS]
    if encrypted:
        requests = [requests[0], ("daemon.unlock", {"auth_response": {"passphrase_secret": PASSPHRASE}}), *requests,
                    ("daemon.lock", {}), requests[2]]
    before = dump(root, encrypted)
    outputs = []
    for reference in [True, False]:
        result = run(root, ["daemon"], env, reference=reference, wire=wire(requests))
        assert (result.returncode, result.stderr) == (0, b""), result.stderr
        assert dump(root, encrypted) == before
        assert SECRET.encode() not in result.stdout
        outputs.append(result.stdout.replace(str(root).encode(), b"ROOT").splitlines()[1:])
    assert outputs[1] == outputs[0]
    assert all(b"not_implemented" not in line for line in outputs[1])


@contextmanager
def btcpay_server(routes):
    """A loopback Greenfield fake: `routes[path] = (status, body)` keyed without the query.

    A route may instead be a callable of the parsed query returning `(status, body)`.
    """
    hits = []

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            path, _, query = self.path.partition("?")
            hits.append((self.path, self.headers.get("Authorization")))
            route = routes.get(path, (404, b""))
            status, body = route(parse_qs(query)) if callable(route) else route
            data = body if isinstance(body, bytes) else json.dumps(body).encode()
            self.send_response(status)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=lambda: server.serve_forever(poll_interval=0.01))
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}", hits
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


def wallet_routes():
    tx = "/api/v1/stores/{}/payment-methods/{}/wallet/transactions"
    legacy = "/api/v1/stores/{}/payment-methods/onchain/{}/wallet/transactions"
    return {
        tx.format("ok", "BTC-CHAIN"): (200, []),
        tx.format("ok", "LBTC-CHAIN"): (200, [{"transactionHash": "a" * 64}]),
        legacy.format("legacy", "BTC"): (200, []),
        tx.format("denied", "BTC-CHAIN"): (403, {"code": "missing-permission", "missingPermission": "btcpay.store.canmodifystoresettings:denied"}),
        tx.format("denied-plain", "BTC-CHAIN"): (403, b"forbidden"),
        tx.format("unauthorized", "BTC-CHAIN"): (401, {"code": "unauthenticated"}),
        tx.format("store-gone", "BTC-CHAIN"): (404, {"code": "store-not-found"}),
        tx.format("broken", "BTC-CHAIN"): (500, {"message": "boom " + "x" * 300}),
        tx.format("html", "BTC-CHAIN"): (200, b"<html>login</html>"),
        tx.format("object", "BTC-CHAIN"): (200, {"not": "a list"}),
        tx.format("validation", "BTC-CHAIN"): (422, [{"path": "x", "message": "bad thing"}]),
        tx.format("sp%20ace%2Fslash", "BTC-CHAIN"): (200, []),
    }


BTCPAY_TEST_ARGS = [
    {"backend": "merchant", "store_id": "ok"}, {"backend": "merchant", "store_id": "ok", "payment_method_id": "lbtc"},
    {"backend": "merchant", "store_id": "legacy"}, {"backend": "merchant", "store_id": "denied"},
    {"backend": "merchant", "store_id": "denied-plain"}, {"backend": "merchant", "store_id": "unauthorized"},
    {"backend": "merchant", "store_id": "store-gone"}, {"backend": "merchant", "store_id": "broken"},
    {"backend": "merchant", "store_id": "html"}, {"backend": "merchant", "store_id": "object"},
    {"backend": "merchant", "store_id": "validation"}, {"backend": "merchant", "store_id": "missing-legacy"},
    {"backend": " MERCHANT ", "store_id": " sp ace/slash "},
    {"backend": "merchant", "store_id": "ok", "payment_method_id": "BTC-LN"},
    {"backend": "merchant", "store_id": "ok", "payment_method_id": "XMR"}, {"backend": "merchant"},
    {"backend": "merchant", "store_id": 5}, {"backend": "missing", "store_id": "ok"}, {"backend": "chain", "store_id": "ok"},
    {"backend": "merchant", "store_id": "ok", "server_url": "http://127.0.0.1:1"},
    {"store_id": "ok", "api_key": "inline-key"}, {"store_id": "ok", "server_url": "ftp://x", "api_key": "k"},
    {"store_id": "ok", "server_url": "INLINE", "api_key": "inline-key", "backend_label": "!!!"},
    {"store_id": "ok", "server_url": "INLINE", "token": " inline-key "},
    {}, [], None,
]


def btcpay_session(root, env, requests, url):
    """Reference then candidate over identical starting bytes; returns both outputs."""
    database = root / "kassiber.sqlite3"
    initial = database.read_bytes()
    wire_bytes = wire([(kind, json.loads(json.dumps(args).replace("INLINE", url))) for kind, args in requests])
    outputs = []
    for reference in [True, False]:
        database.write_bytes(initial)
        result = run(root, ["daemon"], env, reference=reference, wire=wire_bytes)
        assert (result.returncode, result.stderr) == (0, b""), result.stderr
        assert SECRET.encode() not in result.stdout and b"inline-key" not in result.stdout
        outputs.append(result.stdout.replace(str(root).encode(), b"ROOT").replace(url.encode(), b"URL").splitlines()[1:])
    return outputs


def test_btcpay_wallet_test_daemon_bytes_requests_and_all_tables(tmp_path):
    with btcpay_server(wallet_routes()) as (url, hits):
        root, env = btcpay_book(tmp_path, url=url + "/prefix-ignored/api/v1".replace("/prefix-ignored", ""))
        before = dump(root)
        requests = [("ui.connections.btcpay.test", args) for args in BTCPAY_TEST_ARGS]
        python_hits, rust_hits = [], []
        database = root / "kassiber.sqlite3"
        initial = database.read_bytes()
        outputs = []
        for reference, log in [(True, python_hits), (False, rust_hits)]:
            database.write_bytes(initial)
            hits.clear()
            wire_bytes = wire([(kind, json.loads(json.dumps(args).replace("INLINE", url))) for kind, args in requests])
            result = run(root, ["daemon"], env, reference=reference, wire=wire_bytes)
            assert (result.returncode, result.stderr) == (0, b""), result.stderr
            assert SECRET.encode() not in result.stdout and b"inline-key" not in result.stdout
            outputs.append(result.stdout.replace(url.encode(), b"URL").splitlines()[1:])
            log.extend(hits)
            assert dump(root) == before
        assert outputs[1] == outputs[0]
        assert [path for path, _ in rust_hits] == [path for path, _ in python_hits]
        assert {auth for _, auth in rust_hits} == {f"token {SECRET}", "token inline-key"}
        responses = [json.loads(line) for line in outputs[1][:len(requests)]]
        assert responses[0]["data"] == {"backend": "merchant", "store_id": "ok", "payment_method_id": "BTC-CHAIN", "ok": True}


@pytest.mark.parametrize("switch", ["offline", "no-egress"])
def test_btcpay_wallet_test_refuses_before_any_traffic(tmp_path, switch):
    with btcpay_server(wallet_routes()) as (url, hits):
        root, env = btcpay_book(tmp_path, url=url)
        if switch == "offline":
            Path(env["KASSIBER_OFFLINE_PREFERENCE_FILE"]).write_text('{"enabled":true}')
        else:
            env["KASSIBER_NO_EGRESS"] = "1"
        requests = [("ui.connections.btcpay.test", {"backend": "merchant", "store_id": "ok"})]
        outputs = btcpay_session(root, env, requests, url)
        assert hits == []
        assert outputs[1] == outputs[0]
        assert json.loads(outputs[1][0])["error"]["code"] == "network_egress_disabled"


# --- BTCPay provenance sync --------------------------------------------------------------

UUID = re.compile(rb"[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}")
STAMP = re.compile(rb"20[2-9][0-9]-[01][0-9]-[0-3][0-9]T[0-2][0-9]:[0-5][0-9]:[0-5][0-9]Z")


def masked(data, started):
    """Generated ids and clocks read at or after `started` become placeholders."""
    data = UUID.sub(b"<uuid>", data)
    return STAMP.sub(lambda m: b"<now>" if m.group(0) >= started else m.group(0), data)


AGE = re.compile(rb'"age_seconds":\s?(\d+)')


def settle_ages(expected, actual, started):
    """Sync-state ages read the clock: the candidate's may exceed the reference's by the time both runs took."""
    slack = int(time.monotonic() - started) + 1
    want, got = [int(m) for m in AGE.findall(expected)], [int(m) for m in AGE.findall(actual)]
    assert len(want) == len(got) and all(0 <= b - a <= slack for a, b in zip(want, got)), (want, got, slack)
    return AGE.sub(b'"age_seconds":"<age>"', expected), AGE.sub(b'"age_seconds":"<age>"', actual)


def masked_dump(root, started, encrypted=False):
    tables = dump(root, encrypted)
    return {name: sorted(masked(json.dumps(row, default=repr).encode(), started) for row in rows)
            for name, rows in tables.items()}


def started_stamp():
    from datetime import datetime, timezone
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z").encode()


PAY_TXID = "ab" * 32
PAYOUT_TXID = "cd" * 32


def provenance_routes():
    store = "/api/v1/stores/shop"
    invoices = [
        {"id": "inv-paid", "status": "Settled", "currency": "EUR", "amount": "30.00", "createdTime": 1700000000,
         "metadata": {"orderId": "PAY_REQUEST_req1", "itemDesc": ""},
         "paymentMethods": [{"paymentMethodId": "BTC-CHAIN", "rate": "30000",
                             "payments": [{"id": PAY_TXID + "-0", "value": "0.001", "receivedDate": 1700000100,
                                           "status": "Settled"}]}]},
        {"id": "inv-hydrate", "status": "Processing", "currency": "USD", "amount": "5",
         "metadata": {"orderUrl": "https://shop.example/apps/App1/pos", "posData": {"cart": [{"title": "Tea"}]}}},
        {"id": "inv-new", "status": "New", "additionalStatus": "None", "currency": "BTC", "amount": "0.1"},
        {"id": "inv-ln", "status": "Settled", "currency": "EUR", "amount": "1",
         "paymentMethods": [{"paymentMethodId": "BTC-LN", "payments": [{"id": "ef" * 32, "value": "0.00002"}]}]},
    ]
    return {
        store + "/invoices": (200, invoices),
        store + "/invoices/inv-hydrate/payment-methods": (200, [
            {"paymentMethodId": "BTC_LightningLike", "payments": [{"id": "12" * 32, "value": "0.0001", "receivedDate": "2024-01-02T00:00:00Z"}]}]),
        store + "/payment-requests": (200, [{"id": "req1", "title": "Consulting retainer"}]),
        store + "/payouts": (200, [
            {"id": "po1", "payoutMethodId": "BTC-CHAIN", "state": "Completed", "date": 1700000500,
             "payoutAmount": "0.0005", "payoutCurrency": "BTC", "originalCurrency": "EUR", "originalAmount": "15",
             "paymentProof": {"proofType": "PayoutTransactionOnChainBlob", "transactionId": PAYOUT_TXID},
             "destination": "bc1qdest", "pullPaymentId": "pp1"},
            {"id": "po2", "payoutMethodId": "BTC-LN", "state": "AwaitingPayment", "destination": "lnbc1xyz"}]),
        store + "/pull-payments": (200, [{"id": "pp1", "name": "Refund 5ZbLqJLTwm9ZbDtZzkB3Yp"}]),
        "/api/v1/stores/denied/invoices": (403, {"code": "missing-permission", "missingPermission": "btcpay.store.canviewinvoices:denied"}),
        "/api/v1/stores/nopayouts/invoices": (200, []),
        "/api/v1/stores/nopayouts/payouts": (403, {"code": "missing-permission"}),
    }


def provenance_book(tmp_path, url, encrypted=False):
    sql = f"""
        INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,occurred_at,direction,asset,amount,fee,created_at)
        VALUES('tx-in','w','p','a','fp-in','{PAY_TXID}','2023-11-14T22:15:00Z','inbound','BTC',100000000,0,'2026-01-01T00:00:00Z'),
              ('tx-out','w','p','a','fp-out','{PAYOUT_TXID}','2023-11-14T22:21:40Z','outbound','BTC',50000000,0,'2026-01-01T00:00:00Z');
    """
    return btcpay_book(tmp_path, encrypted=encrypted, url=url, sql=sql)


SYNC_ARGS = [
    {"backend": "merchant", "store_id": "shop"}, {"backend": "MERCHANT", "store_id": "shop", "page_size": "2"},
    {"backend": "merchant", "store_id": "denied"}, {"backend": "merchant", "store_id": "nopayouts", "page_size": 1},
    {"backend": "merchant", "store_id": "shop", "page_size": 0}, {"backend": "chain", "store_id": "shop"},
    {"backend": "missing", "store_id": "shop"}, {"backend": "", "store_id": "shop"}, {"backend": "merchant"},
    {"backend": "merchant", "store_id": 5}, {"backend": "merchant", "store_id": "shop", "page_size": True},
]


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
def test_provenance_sync_daemon_bytes_requests_and_all_tables(tmp_path, encrypted):
    with btcpay_server(provenance_routes()) as (url, hits):
        root, env = provenance_book(tmp_path, url, encrypted)
        requests = [("ui.btcpay.provenance.sync", args) for args in SYNC_ARGS]
        requests += [("ui.btcpay.provenance.list", {}), ("ui.btcpay.provenance.links", {})]
        if encrypted:
            requests = [("daemon.unlock", {"auth_response": {"passphrase_secret": PASSPHRASE}}), *requests]
        database = root / "kassiber.sqlite3"
        initial = database.read_bytes()
        started = started_stamp()
        outcomes = []
        for reference in [True, False]:
            database.write_bytes(initial)
            hits.clear()
            result = run(root, ["daemon"], env, reference=reference, wire=wire(requests))
            assert (result.returncode, result.stderr) == (0, b""), result.stderr
            assert SECRET.encode() not in result.stdout
            outcomes.append((masked(result.stdout.replace(str(root).encode(), b"ROOT"), started).splitlines()[1:],
                             [path for path, _ in hits], masked_dump(root, started, encrypted)))
        # Records with equal times list in random-id order; compare them as sets.
        def settled(lines):
            out = []
            for line in lines:
                record = json.loads(line)
                data = record.get("data")
                if isinstance(data, dict) and set(data) & {"records", "links"}:
                    key = "records" if "records" in data else "links"
                    data[key] = sorted(data[key], key=lambda row: json.dumps(row, sort_keys=True))
                out.append(json.dumps(record, sort_keys=True))
            return out
        assert settled(outcomes[1][0]) == settled(outcomes[0][0])
        assert outcomes[1][1] == outcomes[0][1]
        assert outcomes[1][2] == outcomes[0][2]
        tables = outcomes[1][2]
        assert len(tables["btcpay_provenance_records"]) >= 6
        assert any(b"denied" in row for row in tables["btcpay_store_sync_states"])
        assert tables["commercial_links"], "suggestions link the paid invoice and the payout"


@pytest.mark.parametrize("options", [
    ["--backend", "merchant", "--store-id", "shop"], ["--backend", "merchant", "--store-id", "shop", "--no-payouts"],
    ["--backend", "merchant", "--store-id", "shop", "--page-size", "1"], ["--backend", "merchant"],
    ["--backend", "merchant", "--store-id", "denied"], ["--backend", "merchant", "--store-id", "shop", "--page-size", "-1"],
    ["--backend", "merchant", "--store-id", "shop", "--all"], ["--store-id", "shop"],
])
def test_provenance_sync_cli_bytes_and_all_tables(tmp_path, options):
    with btcpay_server(provenance_routes()) as (url, hits):
        root, env = provenance_book(tmp_path, url)
        database = root / "kassiber.sqlite3"
        initial = database.read_bytes()
        started = started_stamp()
        outcomes = []
        for reference in [True, False]:
            database.write_bytes(initial)
            hits.clear()
            result = run(root, ["--machine", "btcpay", "provenance", "sync", *options], env, reference=reference)
            outcomes.append((result.returncode, masked(result.stdout, started), result.stderr,
                             [path for path, _ in hits], masked_dump(root, started)))
        assert outcomes[1] == outcomes[0]
        assert served(env) == [{"command": "btcpay.provenance.sync", "served_by": "rust"}]


@pytest.mark.parametrize("switch", ["offline", "no-egress"])
def test_provenance_sync_refuses_before_traffic_and_records_the_attempt(tmp_path, switch):
    with btcpay_server(provenance_routes()) as (url, hits):
        root, env = provenance_book(tmp_path, url)
        if switch == "offline":
            Path(env["KASSIBER_OFFLINE_PREFERENCE_FILE"]).write_text('{"enabled":true}')
        else:
            env["KASSIBER_NO_EGRESS"] = "1"
        database = root / "kassiber.sqlite3"
        initial = database.read_bytes()
        started = started_stamp()
        outcomes = []
        for reference in [True, False]:
            database.write_bytes(initial)
            result = run(root, ["daemon"], env, reference=reference,
                         wire=wire([("ui.btcpay.provenance.sync", {"backend": "merchant", "store_id": "shop"})]))
            outcomes.append((masked(result.stdout, started).splitlines()[1:], masked_dump(root, started)))
        assert hits == []
        assert outcomes[1] == outcomes[0]
        assert json.loads(outcomes[1][0][0])["error"]["code"] == "network_egress_disabled"


# --- BTCPay discovery and setup plans -----------------------------------------------------

OWNED_A = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
OWNED_B = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"


def discovery_routes(*, key_permissions=("btcpay.store.canviewstoresettings",), version="2.0.4"):
    preview = (200, {"addresses": [{"address": OWNED_A}, {"address": OWNED_B}, {"no": "address"}]})
    return {
        "/api/v1/server/info": (200, {"version": version, "fullySynched": False, "supportedPaymentMethods": ["BTC", "BTC-LightningNetwork", ""]}),
        "/api/v1/api-keys/current": (200, {"label": "Kassiber", "permissions": list(key_permissions), "apiKey": SECRET}),
        "/api/v1/stores": (200, [{"id": "s1", "name": "Shop", "defaultCurrency": "EUR"}, {"id": "s2", "name": "Cafe"},
                                 {"id": "arch", "name": "Old", "archived": True}, "junk"]),
        "/api/v1/stores/s1/payment-methods": (200, [{"paymentMethodId": "BTC-CHAIN", "enabled": True},
                                                     {"paymentMethodId": "BTC-LN", "enabled": True},
                                                     {"paymentMethodId": "XMR-CHAIN"}, {"paymentMethodId": "BTC-Boltz"}]),
        "/api/v1/stores/s1/payment-methods/BTC-CHAIN/wallet/preview": preview,
        "/api/v1/stores/s2/payment-methods": (200, {"BTC": True, "LBTC-CHAIN": {"enabled": True}, "DOGE": False}),
        "/api/v1/stores/s2/payment-methods/BTC-CHAIN/wallet/preview": preview,
        "/api/v1/stores/s2/payment-methods/LBTC-CHAIN/wallet/preview": (403, {"code": "missing-permission"}),
        "/api/v1/stores/s2/payment-methods/onchain/LBTC/preview": (200, {"addresses": []}),
    }


def discovery_book(tmp_path, url, *, encrypted=False):
    sql = f"""
        INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at)
        VALUES('cold','w','p','Shop cold','address','{{"addresses":["{OWNED_A}","{OWNED_B}"],"chain":"bitcoin","network":"main"}}','2026-01-01T00:00:00Z'),
              ('ln','w','p','Node','lnd','{{}}','2026-01-01T00:00:00Z');
        INSERT INTO backends(name,kind,url,token,created_at,updated_at) VALUES('sibling','btcpay','{url}/','other-key','fixed','fixed');
        INSERT INTO btcpay_account_routes(id,workspace_id,profile_id,backend_name,store_id,payment_method_id,action,label,created_at,updated_at)
        VALUES('route-1','w','p','merchant','s1','BTC-LN','provenance_only','Shop','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z');
        INSERT INTO btcpay_store_sync_states(profile_id,backend_name,store_id,last_attempt_at,last_success_at,updated_at)
        VALUES('p','merchant','s1','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z');
    """
    return btcpay_book(tmp_path, encrypted=encrypted, url=url, sql=sql)


DISCOVER_ARGS = [
    {"backend": "merchant"}, {"backend": "Sibling"}, {"backend": "missing"}, {"backend": "chain"},
    {"server_url": "INLINE", "api_key": "inline-key", "instance_label": "New shop"}, {"server_url": "INLINE"},
    {"backend": "merchant", "api_key": "x"}, {}, [],
]


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
@pytest.mark.parametrize("variant", ["read_only", "wallet_history", "legacy_unknown_key",
                                     "btcpay24_read_only", "btcpay24_store_only_key"])
def test_btcpay_discover_daemon_bytes_requests_and_all_tables(tmp_path, encrypted, variant):
    if encrypted and variant != "read_only":
        pytest.skip("one encrypted session")
    routes = discovery_routes() if variant == "read_only" else discovery_routes(
        key_permissions=("btcpay.store.canmodifystoresettings:s1", "btcpay.store.canviewinvoices:s2"))
    # BTCPay 2.4 guards the wallet preview with `canviewwallet` (issue #692):
    # a 2.3 read-only key loses it, the new read-only key keeps it.
    if variant == "btcpay24_read_only":
        routes = discovery_routes(version="2.4.7", key_permissions=(
            "btcpay.store.canviewstoresettings", "btcpay.store.canviewwallet"))
    if variant == "btcpay24_store_only_key":
        routes = discovery_routes(version="2.4.7")
    if variant == "legacy_unknown_key":
        routes = discovery_routes(version="1.12.5")
        routes["/api/v1/api-keys/current"] = (404, b"")
        routes["/api/v1/stores/s1/payment-methods"] = (403, b"")
    with btcpay_server(routes) as (url, hits):
        root, env = discovery_book(tmp_path, url, encrypted=encrypted)
        requests = [("ui.connections.btcpay.discover", json.loads(json.dumps(a).replace("INLINE", url))) for a in DISCOVER_ARGS]
        if encrypted:
            requests = [("daemon.unlock", {"auth_response": {"passphrase_secret": PASSPHRASE}}), *requests]
        database = root / "kassiber.sqlite3"
        initial = database.read_bytes()
        before = dump(root, encrypted)
        clock = time.monotonic()
        outcomes = []
        for reference in [True, False]:
            database.write_bytes(initial)
            hits.clear()
            result = run(root, ["daemon"], env, reference=reference, wire=wire(requests))
            assert (result.returncode, result.stderr) == (0, b""), result.stderr
            assert SECRET.encode() not in result.stdout and b"inline-key" not in result.stdout
            assert OWNED_A.encode() not in result.stdout, "preview addresses never leave the inspection"
            outcomes.append((result.stdout.replace(url.encode(), b"URL").replace(str(root).encode(), b"ROOT"),
                             sorted(path for path, _ in hits)))
            assert dump(root, encrypted) == before
        expected, actual = settle_ages(outcomes[0][0], outcomes[1][0], clock)
        outcomes = [(expected.splitlines()[1:], outcomes[0][1]), (actual.splitlines()[1:], outcomes[1][1])]
        assert outcomes[1][0] == outcomes[0][0]
        assert outcomes[1][1] == outcomes[0][1]
        first = json.loads(outcomes[1][0][1 if encrypted else 0])
        assert first["kind"] == "ui.connections.btcpay.discover", first
        if variant.startswith("btcpay24"):
            methods = {m["payment_method_id"]: m for m in first["data"]["payment_methods"] if m["store_id"] == "s1"}
            assert first["data"]["key_upgrade"]["preset"] == "read_only", first["data"]["key_upgrade"]
            preview = methods["BTC-CHAIN"]["preview_error"], methods["BTC-CHAIN"]["network"]
            assert preview == ((None, "main") if variant == "btcpay24_read_only" else ("missing_permission", None)), methods


@pytest.mark.parametrize("options", [["--backend", "merchant"], ["--backend", "sibling", "--profile", "Book"],
                                     ["--backend", "chain"], ["--backend", "missing"], ["--backend", "merchant", "--profile", "missing"]])
def test_btcpay_inspect_cli_bytes_and_all_tables(tmp_path, options):
    with btcpay_server(discovery_routes()) as (url, hits):
        root, env = discovery_book(tmp_path, url)
        outcomes = []
        database = root / "kassiber.sqlite3"
        initial = database.read_bytes()
        before = dump(root)
        clock = time.monotonic()
        for reference in [True, False]:
            database.write_bytes(initial)
            hits.clear()
            result = run(root, ["--machine", "btcpay", "inspect", *options], env, reference=reference)
            outcomes.append((result.returncode, result.stdout.replace(url.encode(), b"URL"), result.stderr, sorted(p for p, _ in hits)))
            assert dump(root) == before
        expected, actual = settle_ages(outcomes[0][1], outcomes[1][1], clock)
        outcomes = [(o[0], text, *o[2:]) for o, text in zip(outcomes, [expected, actual])]
        assert outcomes[1] == outcomes[0]
        assert served(env) == [{"command": "btcpay.inspect", "served_by": "rust"}]


# --- BTCPay setup: create, attach, setup and sync --all ------------------------------------

def setup_routes(**discovery):
    routes = discovery_routes(**discovery)
    for store in ["s1", "s2"]:
        routes[f"/api/v1/stores/{store}/invoices"] = (200, [
            {"id": f"{store}-inv", "status": "Settled", "currency": "EUR", "amount": "10", "createdTime": 1700000000,
             "paymentMethods": [{"paymentMethodId": "BTC-CHAIN", "payments": [{"id": PAY_TXID + "-1", "value": "0.0002"}]}]}])
        routes[f"/api/v1/stores/{store}/payouts"] = (200, [])
    routes["/api/v1/stores/s2/payouts"] = (403, {"code": "missing-permission"})
    return routes


def settle(root, encrypted):
    """Open the book once as the next session would, applying open-time repairs."""
    open_db(root, passphrase=PASSPHRASE if encrypted else None).close()


def compare_session(root, env, requests, url, *, encrypted=False, derived=None, settled=False):
    """`derived` rewrites named tables' masked rows before comparing, for values derived from random ids.
    `settled` compares each book as its next open sees it."""
    database = root / "kassiber.sqlite3"
    initial = database.read_bytes()
    started = started_stamp()
    clock = time.monotonic()
    outcomes = []
    for reference in [True, False]:
        database.write_bytes(initial)
        result = run(root, ["daemon"], env, reference=reference, wire=wire(requests))
        assert (result.returncode, result.stderr) == (0, b""), result.stderr
        assert SECRET.encode() not in result.stdout and b"inline-key" not in result.stdout
        lines = masked(result.stdout.replace(url.encode(), b"URL").replace(str(root).encode(), b"ROOT"), started)
        if settled:
            settle(root, encrypted)
        tables = masked_dump(root, started, encrypted)
        for table, rewrite in (derived or {}).items():
            tables[table] = sorted(rewrite(row) for row in tables[table])
        outcomes.append((lines, tables))
    expected, actual = settle_ages(outcomes[0][0], outcomes[1][0], clock)
    assert actual.splitlines()[1:] == expected.splitlines()[1:]
    assert outcomes[1][1] == outcomes[0][1]
    return [json.loads(line) for line in actual.splitlines()[1:]], outcomes[1][1]


ACCOUNT_ROUTES = [
    {"store_id": "s1", "payment_method_id": "BTC-CHAIN", "action": "map", "wallet": "Shop cold"},
    {"store_id": "s1", "payment_method_id": "BTC-LN", "action": "ledger", "network": "main"},
    {"store_id": "s2", "payment_method_id": "BTC", "action": "provenance", "store_name": "Cafe"},
    {"store_id": "s2", "payment_method_id": "lbtc", "action": "wallet_source", "wallet_fingerprint": "btcpay-wallet:abc"},
    {"store_id": "s2", "payment_method_id": "XMR-CHAIN", "action": "skip"},
]

CREATE_ARGS = [
    {"label": "Shop", "backend": "merchant", "store_id": "s1"},
    {"label": "Shop", "backend": "merchant", "store_id": "s1"},
    {"label": "Multi", "backend": "merchant", "store_id": "s1", "payment_method_ids": ["BTC", "lbtc", "btc-chain"]},
    {"label": "Bad", "backend": "merchant", "store_id": "s1", "payment_method_ids": ["BTC-LN"]},
    {"label": "Bad", "backend": "merchant", "store_id": "s1", "payment_method_ids": "BTC"},
    {"label": "Bad", "backend": "merchant", "store_id": "s1", "payment_method_ids": [" "]},
    {"label": "Inline", "server_url": "INLINE/stores/x", "api_key": "inline-key", "store_id": "s1", "backend_label": "New Shop", "network": "TEST"},
    {"label": "Inline 2", "server_url": "INLINE", "api_key": "inline-key", "store_id": "s1", "backend_label": "New Shop"},
    {"label": "Inline 3", "server_url": "INLINE", "api_key": "inline-key", "store_id": "s1", "backend_label": "Other"},
    {"label": "Inline 4", "server_url": "INLINE", "api_key": "k2", "store_id": "s1", "backend_label": "Third", "network": "mars"},
    {"label": "Map", "mode": "map_existing", "backend": "merchant", "store_id": "s1", "target_wallet": "Shop cold"},
    {"label": "Map", "mode": "provenance", "backend": "merchant", "store_id": "s2",
     "routes": [{"wallet": "Shop cold", "payment_method_id": "lbtc"}, {"target_wallet": "Shop cold"}, {"wallet": "Shop cold"}]},
    {"label": "Map", "mode": "existing_wallets", "backend": "merchant", "store_id": "s1", "target_wallet": "missing"},
    {"label": "Map", "mode": "existing_wallets", "backend": "merchant", "store_id": "s1", "routes": [{"payment_method_id": "BTC"}]},
    {"label": "Map", "mode": "existing_wallets", "backend": "merchant", "store_id": "s1"},
    {"label": "Acct", "mode": "account", "backend": "merchant", "routes": ACCOUNT_ROUTES},
    {"label": "Acct", "mode": "account", "backend": "merchant", "routes": ACCOUNT_ROUTES, "sync_provenance": False},
    {"label": "Acct", "mode": "account_wizard", "backend": "merchant", "routes": [{"store_id": "s1", "action": "bogus"}]},
    {"label": "Acct", "mode": "account", "routes": [{"store_id": "s1", "action": "skip"}], "server_url": "INLINE", "api_key": "inline-key"},
    {"label": "Acct", "mode": "account", "backend": "merchant", "routes": []},
    {"label": "Acct", "mode": "nonsense", "backend": "merchant"}, {"mode": "account"}, {}, [],
]


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
def test_btcpay_create_daemon_bytes_and_all_tables(tmp_path, encrypted):
    with btcpay_server(setup_routes()) as (url, hits):
        root, env = discovery_book(tmp_path, url, encrypted=encrypted)
        requests = [("ui.connections.btcpay.create", json.loads(json.dumps(a).replace("INLINE", url))) for a in CREATE_ARGS]
        requests.append(("ui.backends.list", {}))
        if encrypted:
            requests = [("daemon.unlock", {"auth_response": {"passphrase_secret": PASSPHRASE}}), *requests]
        responses, tables = compare_session(root, env, requests, url, encrypted=encrypted)
        kinds = [r.get("kind") for r in responses]
        assert kinds.count("ui.connections.btcpay.create") >= 5, kinds


@pytest.mark.parametrize("options", [
    ["--wallet", "Shop cold", "--backend", "merchant", "--store-id", " s1 "],
    ["--wallet", "Shop cold", "--backend", "MERCHANT", "--store-id", "s1", "--payment-method-id", "lbtc"],
    ["--wallet", "Shop cold", "--backend", "merchant", "--store-id", "s1", "--payment-method-id", "BTC-LN"],
    ["--wallet", "Shop cold", "--backend", "chain", "--store-id", "s1"],
    ["--wallet", "missing", "--backend", "merchant", "--store-id", "s1"],
    ["--wallet", "Shop cold", "--backend", "missing", "--store-id", "s1"],
    ["--wallet", "Shop cold", "--backend", "merchant", "--store-id", " "],
])
def test_attach_btcpay_cli_bytes_and_all_tables(tmp_path, options):
    with btcpay_server(setup_routes()) as (url, hits):
        root, env = discovery_book(tmp_path, url)
        compare_cli_masked(root, env, ["wallets", "attach-btcpay", *options], url)
        assert hits == []
        assert served(env) == [{"command": "wallets.attach-btcpay", "served_by": "rust"}]


def compare_cli_masked(root, env, argv, url):
    database = root / "kassiber.sqlite3"
    initial = database.read_bytes()
    started = started_stamp()
    clock = time.monotonic()
    outcomes = []
    for reference in [True, False]:
        database.write_bytes(initial)
        result = run(root, ["--machine", *argv], env, reference=reference)
        outcomes.append((result.returncode, masked(result.stdout.replace(url.encode(), b"URL"), started),
                         result.stderr, masked_dump(root, started)))
        assert SECRET.encode() not in result.stdout
    expected, actual = settle_ages(outcomes[0][1], outcomes[1][1], clock)
    outcomes = [(o[0], text, *o[2:]) for o, text in zip(outcomes, [expected, actual])]
    assert outcomes[1] == outcomes[0]
    return json.loads(outcomes[1][1])


@pytest.mark.parametrize("options", [
    ["--backend", "merchant", "--label", "Shop", "--recommended"],
    ["--backend", "merchant", "--label", "Shop", "--recommended", "--dry-run"],
    ["--backend", "merchant", "--label", "Shop", "--recommended", "--no-sync"],
    ["--backend", "merchant", "--label", "Shop", "--route", "s1=existing_wallet@Shop cold", "--route", "s2:LBTC=provenance_only"],
    ["--backend", "merchant", "--label", "Shop", "--route", "s1:BTC-LN=ledger"],
    ["--backend", "merchant", "--label", "Shop", "--route", "s1=wallet_source"],
    ["--backend", "merchant", "--label", "Shop", "--route", "bad"],
    ["--backend", "merchant", "--label", "Shop", "--route", "zz=skip"],
    ["--backend", "merchant", "--label", "Shop"], ["--backend", "chain", "--label", "Shop", "--recommended"],
])
def test_btcpay_setup_cli_bytes_and_all_tables(tmp_path, options):
    with btcpay_server(setup_routes()) as (url, hits):
        root, env = discovery_book(tmp_path, url)
        compare_cli_masked(root, env, ["btcpay", "setup", *options], url)
        assert served(env) == [{"command": "btcpay.setup", "served_by": "rust"}]


@pytest.mark.parametrize("key", [("btcpay.store.canviewstoresettings",),
                                 ("btcpay.store.canviewstoresettings", "btcpay.store.canviewwallet")],
                         ids=["store_only", "read_only"])
@pytest.mark.parametrize("options", [
    ["--backend", "merchant", "--label", "Shop", "--recommended", "--dry-run"],
    ["--backend", "merchant", "--label", "Shop", "--route", "s1=wallet_source", "--no-sync"],
])
def test_btcpay_24_setup_cli_bytes_and_all_tables(tmp_path, key, options):
    """On BTCPay 2.4 the read-only key reads wallet history (`canviewwallet`);
    a key with only `canviewstoresettings` is offered the read-only upgrade."""
    with btcpay_server(setup_routes(version="2.4.7", key_permissions=key)) as (url, hits):
        root, env = discovery_book(tmp_path, url)
        compare_cli_masked(root, env, ["btcpay", "setup", *options], url)
        assert served(env) == [{"command": "btcpay.setup", "served_by": "rust"}]


@pytest.mark.parametrize("options", [["--all"], ["--all", "--no-payouts", "--page-size", "1"], ["--all", "--backend", "merchant"]])
def test_provenance_sync_all_cli_bytes_and_all_tables(tmp_path, options):
    with btcpay_server(setup_routes()) as (url, hits):
        root, env = discovery_book(tmp_path, url)
        compare_cli_masked(root, env, ["btcpay", "provenance", "sync", *options], url)


# --- Bull Bitcoin wallet connections --------------------------------------------------------

def bull_args(tmp_path):
    real = tmp_path / "exports" / "bull.csv"
    real.parent.mkdir(exist_ok=True)
    link = tmp_path / "exports" / "link.csv"
    if not link.exists():
        link.symlink_to(real)
    return [
        {"label": "Bull", "source_file": str(link)},
        {"label": "Bull", "source_file": str(real)},
        {"label": "Bull BTC", "file": str(tmp_path / "exports" / "missing" / ".." / "bull.csv"), "network": "BTC"},
        {"label": "Bull LN", "source_file": str(real), "networks": ["ln", "LIGHTNING", "lbtc"]},
        {"label": "Bull X", "source_file": str(real), "networks": []},
        {"label": "Bull X", "source_file": str(real), "networks": "bitcoin"},
        {"label": "Bull X", "source_file": str(real), "networks": ["doge"]},
        {"label": "Bull X", "source_file": str(real), "networks": [5]},
        {"label": "Map", "source_file": str(real), "mode": "map_existing", "target_wallet": "Cold", "network": "liquid"},
        {"label": "Map", "source_file": str(link), "mode": "existing_wallets",
         "routes": [{"wallet": "Cold", "network": "bitcoin"}, {"target_wallet": "Cold", "network": "btc"}]},
        {"label": "Map", "source_file": str(real), "mode": "provenance", "routes": [{"wallet": "missing", "network": "bitcoin"}]},
        {"label": "Map", "source_file": str(real), "mode": "provenance", "routes": [{"network": "bitcoin"}]},
        {"label": "Map", "source_file": str(real), "mode": "provenance", "routes": [{"wallet": "Cold"}]},
        {"label": "Map", "source_file": str(real), "mode": "provenance"},
        {"label": "Map", "source_file": str(real), "mode": "wizard"},
        {"label": "No file"}, {"source_file": str(real)}, {"label": "Rel", "source_file": "relative/bull.csv"}, [],
    ]


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
def test_bull_wallet_create_daemon_bytes_and_all_tables(tmp_path, encrypted):
    root, env = btcpay_book(tmp_path, encrypted=encrypted)
    requests = [("ui.connections.bullbitcoin_wallet.create", args) for args in bull_args(tmp_path)]
    if encrypted:
        requests = [("daemon.unlock", {"auth_response": {"passphrase_secret": PASSPHRASE}}), *requests]
    responses, tables = compare_session(root, env, requests, "http://unused.invalid", encrypted=encrypted)
    assert [r.get("kind") for r in responses].count("ui.connections.bullbitcoin_wallet.create") >= 5
    assert not (tmp_path / "exports" / "bull.csv").exists(), "the export is never opened or created"


@pytest.mark.parametrize("options", [
    ["--file", "EXPORT", "--network", "liquid"], ["--file", "EXPORT/../x/./bull.csv", "--network", "bitcoin"],
    ["--file", "relative/bull.csv", "--network", "lightning"], ["--file", "~/bull.csv", "--network", "bitcoin"],
    ["--file", "EXPORT", "--network", "liquid", "--profile", "missing"], ["--file", "EXPORT", "--network", "doge"],
])
@pytest.mark.parametrize("config", [{}, {"bullbitcoin_wallet_exports": [{"source_file": "/a/../b.csv", "network": "LBTC"}]},
                                    {"bullbitcoin_wallet_exports": "bad"}])
def test_attach_bull_wallet_cli_bytes_and_all_tables(tmp_path, options, config):
    root, env = btcpay_book(tmp_path, config=config)
    export = str(tmp_path / "exports" / "bull.csv")
    argv = ["wallets", "attach-bullbitcoin-wallet", "--wallet", "Cold", *[o.replace("EXPORT", export) for o in options]]
    compare_cli_masked(root, env, argv, "http://unused.invalid")
    command = "<usage>" if "doge" in options else "wallets.attach-bullbitcoin-wallet"
    assert served(env) == [{"command": command, "served_by": "rust"}]


BULL_HEADER = ("date,type,direction,amount_sats,amount_btc,fee_sats,status,txid,network,address,swap_id,preimage,"
               "total_swap_fees_sats,send_network,receive_network,send_txid,receive_txid")
PREIMAGE = "5a" * 32
BULL_ROWS = [
    "2026-01-02T10:00:00Z,onchain,incoming,150000,0.0015,200,confirmed,{tx1},bitcoin,bc1q,,,,,,,",
    "2026-01-03T10:00:00Z,liquid,outgoing,-5000,,1.0,completed,{tx2},,,,,,,,,",
    "2026-01-04T10:00:00Z,lightning_receive,Received,2100,,,completed,,,,swap-ln-1," + PREIMAGE + ",,,,,",
    "2026-01-05T10:00:00Z,lightning_send,sent,3000,,0,pending,{hash},,,, ,,,,,",
    "2026-01-06T10:00:00Z,chain_swap,inbound,40000,,150,completed,,,,swap-2,,300,liquid,BTC,,{tx3}",
    "2026-01-07T10:00:00Z,payjoin_send,outbound,1E+3,,,confirmed,{tx4},,,,,,,,,",
    "2026-01-08T10:00:00Z,onchain,incoming,999,,,failed,{tx5},bitcoin,,,,,,,,",
    "2026-01-09T10:00:00Z,onchain,self,999,,,confirmed,{tx6},bitcoin,,,,,,,,",
    "2026-01-10T10:00:00Z,,receive,77,,,,,LN,,,,,,,,",
]


def bull_export(path, rows=None, header=BULL_HEADER):
    values = {f"tx{i}": f"{i:x}" * 64 for i in range(1, 7)} | {"hash": "AB" * 32}
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join([header, *[row.format(**values) for row in (BULL_ROWS if rows is None else rows)]]) + "\n")
    return str(path)


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
def test_bull_wallet_csv_import_daemon_bytes_and_all_tables(tmp_path, encrypted):
    exports = tmp_path / "exports"
    good = bull_export(exports / "bull.csv")
    bad = {
        "columns": bull_export(exports / "columns.csv", header=BULL_HEADER.replace(",receive_txid", "")),
        "fraction": bull_export(exports / "fraction.csv", ["2026-01-02,onchain,incoming,1.5,,,confirmed,a,bitcoin,,,,,,,,"]),
        "empty": bull_export(exports / "empty.csv", ["2026-01-02,onchain,incoming,,,,confirmed,a,bitcoin,,,,,,,,"]),
        "nan": bull_export(exports / "nan.csv", ["2026-01-02,onchain,incoming,NaN,,,confirmed,a,bitcoin,,,,,,,,"]),
        "direction": bull_export(exports / "direction.csv", ["2026-01-02,onchain,sideways,1,,,confirmed,a,bitcoin,,,,,,,,"]),
        "network": bull_export(exports / "network.csv", ["2026-01-02,mystery,incoming,1,,,confirmed,a,,,,,,,,,"]),
        "blank": bull_export(exports / "blank.csv", []),
    }
    # Fixed wallet ids keep transaction fingerprints, and the journal rows
    # derived from them, identical across the two runs.
    wallets = [("x", "Odd", {"bullbitcoin_wallet_network": "doge"})] + [
        (network, f"Bull - {network.title()}", {"source_file": good, "source_format": "bullbitcoin_wallet_csv",
                                                 "bullbitcoin_wallet_network": network,
                                                 **({"chain": network} if network != "lightning" else {})})
        for network in ["bitcoin", "liquid", "lightning"]]
    sql = "".join("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES"
                  f"('{wallet_id}','w','p','{label}','bullbitcoin','{json.dumps(config)}','2026-01-01T00:00:00Z');"
                  for wallet_id, label, config in wallets)
    root, env = btcpay_book(tmp_path, encrypted=encrypted, sql=sql)

    def importing(wallet, source):
        return ("ui.wallets.import_file", {"wallet": wallet, "source_file": source, "source_format": "bullbitcoin_wallet_csv"})

    requests = [
        *[importing(f"Bull - {name}", good) for name in ["Bitcoin", "Lightning"]],
        importing("Bull - Bitcoin", good),
        importing("Cold", good),
        importing("Odd", good),
        *[importing("Cold", path) for path in bad.values()],
        importing("Bull - Liquid", good),
    ]
    unlock = [("daemon.unlock", {"auth_response": {"passphrase_secret": PASSPHRASE}})] if encrypted else []
    imports = []
    # An import ends its daemon session, so each one runs on the book the last left.
    # Quantity issue ids hash the imported rows' random transaction ids.
    derived = {
        "journal_quantity_issues": lambda row: re.sub(rb'^\["[0-9a-f]{64}"', b'["<issue>"', row),
    }
    if encrypted:
        # Unlocking starts Python's local-evidence worker, whose own open_db
        # lands before or after the import commits; when after, its open-time
        # repairs advance the derived index clock and the gap-input version
        # once more than the next open alone does. Rust starts no such worker,
        # so these counters (and only these) depend on that race.
        derived.update({
            "chain_index_clock": lambda row: re.sub(rb'^\[1, \d+\]$', b'[1, "<revision>"]', row),
            "chain_index_dirty": lambda row: re.sub(rb', -?\d+\]$', b', "<revision>"]', row),
            "journal_custody_gap_inputs": lambda row: re.sub(rb'^(\["[^"]*", "[^"]*", )\d+', rb'\1"<version>"', row),
        })
    # The Liquid rows need Python's open-time identity backfill, which both
    # stores run on their next open. In an unlocked Python session the chain
    # analysis watch worker reopens the book at a timing-dependent point, so
    # each session's book is compared as the next open sees it.
    for request in requests:
        responses, tables = compare_session(root, env, [*unlock, request], "http://unused.invalid",
                                            encrypted=encrypted, derived=derived, settled=True)
        # The import's response precedes the daemon's shutdown event.
        imports.append(responses[len(unlock)])
    # Lightning rows and the unscoped wallet's Liquid rows contradict the book's network.
    assert [r["data"]["bullbitcoin_wallet_network"] for r in (imports[0], imports[2], imports[-1])] == ["bitcoin", "bitcoin", "liquid"]
    assert imports[0]["data"]["imported"] > 0 and imports[2]["data"]["imported"] == 0 and imports[-1]["data"]["imported"] > 0
    assert [imports[i]["error"]["code"] for i in (1, 3)] == ["book_network_mismatch"] * 2
    assert imports[4]["error"]["message"] == "Unsupported Bull Bitcoin wallet network 'doge'", imports[4]
    assert not any(PREIMAGE.encode() in row for rows in tables.values() for row in rows)


# --- BTCPay provenance review -------------------------------------------------------------

CUSTODY_SQL = """
    INSERT INTO custody_components(id,lineage_id,workspace_id,profile_id,revision,component_type,state,created_at)
    VALUES('cc','cc','w','p',1,'transfer','active','2026-01-01T00:00:00Z');
    INSERT INTO custody_component_legs(id,component_id,workspace_id,profile_id,ordinal,role,rail,asset,exposure,conservation_unit,amount_msat,transaction_id,anchor_transaction_id,created_at)
    VALUES('leg','cc','w','p',0,'source','bitcoin','BTC','btc','msat',1,'tx-in','tx-in','2026-01-01T00:00:00Z');
"""


def reviewed_book(tmp_path, *, encrypted=False, custody=False):
    """Sync and suggest once with the reference, then return the shared starting book and its link ids."""
    routes = provenance_routes()
    routes["/api/v1/stores/shop/payouts"][1].append(
        {"id": "po3", "payoutMethodId": "BTC-CHAIN", "state": "Completed", "date": 1700000600, "payoutAmount": "0.0002",
         "payoutCurrency": "BTC", "originalCurrency": "EUR", "originalAmount": "6.5",
         "paymentProof": {"proofType": "PayoutTransactionOnChainBlob", "transactionId": PAYOUT_TXID}})
    with btcpay_server(routes) as (url, _):
        root, env = provenance_book(tmp_path, url)
        result = run(root, ["daemon"], env, reference=True, wire=wire([
            ("ui.btcpay.provenance.sync", {"backend": "merchant", "store_id": "shop"}), ("ui.btcpay.provenance.suggest", {})]))
        assert result.returncode == 0, result.stderr
    conn = sqlite3.connect(root / "kassiber.sqlite3")
    try:
        if custody:
            conn.executescript(CUSTODY_SQL)
            conn.commit()
        links = [row[0] for row in conn.execute(
            "SELECT cl.id FROM commercial_links cl JOIN btcpay_provenance_records p ON p.id=cl.btcpay_record_id "
            "ORDER BY p.stable_key, cl.transaction_id, cl.link_type")]
    finally:
        conn.close()
    assert len(links) >= 3
    if encrypted:
        encrypt(root, env)
    return root, env, links


def review_requests(links):
    review = "ui.btcpay.provenance.review"
    requests = [
        (review, {"link": "missing", "state": "reviewed"}), (review, {"state": "reviewed"}), (review, {"link": links[0]}),
        (review, {"link": links[0], "state": "bogus"}), (review, {"link": links[0], "state": "reviewed", "commercial_kind": ""}),
        (review, {"link": links[0], "state": "reviewed", "reconciliation_state": "bogus"}), (review, {"link": 5, "state": "reviewed"}),
    ]
    for kind, state, extra in [("income", "reviewed", {"notes": "first"}), ("refund", "reviewed", {}),
                               ("none", "reviewed", {"reconciliation_state": "Mis-match"}), (None, "rejected", {}),
                               ("expense", "reviewed", {"notes": 5}), (None, "suggested", {}), ("transfer", "reviewed", {})]:
        for link in links:
            requests.append((review, {"link": link, "state": state, **({} if kind is None else {"commercial_kind": kind}), **extra}))
    return [*requests, ("ui.btcpay.provenance.links", {})]


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
@pytest.mark.parametrize("custody", [False, True], ids=["free", "custody_review"])
def test_provenance_review_daemon_bytes_and_all_tables(tmp_path, encrypted, custody):
    root, env, links = reviewed_book(tmp_path, encrypted=encrypted, custody=custody)
    requests = review_requests(links)
    if encrypted:
        requests = [("daemon.unlock", {"auth_response": {"passphrase_secret": PASSPHRASE}}), *requests]
    responses, tables = compare_session(root, env, requests, "http://unused.invalid", encrypted=encrypted)
    reviews = responses[len(responses) - len(review_requests(links)):-1]
    assert any(r.get("data", {}).get("applied_to_transaction") for r in reviews)
    assert any(r.get("data", {}).get("batch_link_ids") for r in reviews)
    blocked = [r for r in reviews if "active custody review" in r.get("error", {}).get("message", "")]
    assert bool(blocked) == custody, blocked


@pytest.mark.parametrize("options", [
    ["--state", "reviewed", "--commercial-kind", "income"], ["--state", "reviewed", "--reconciliation-state", "mismatch", "--notes", "n"],
    ["--state", "rejected"], ["--state", "reviewed", "--commercial-kind", "none", "--profile", "Book"],
    ["--state", "reviewed", "--profile", "missing"],
])
def test_provenance_review_cli_bytes_and_all_tables(tmp_path, options):
    root, env, links = reviewed_book(tmp_path)
    for link in links:
        compare_cli_masked(root, env, ["btcpay", "provenance", "review", "--link", link, *options], "http://unused.invalid")
    assert {entry["served_by"] for entry in served(env)} == {"rust"}
