"""Desktop exchange connections through the Rust daemon, end to end.

Runs `KASSIBER_RUST_BIN` (a `--features test-harness` build of
crates/kassiber) against the signed loopback fakes
(`tests/rust_port/exchange_api.py`) on disposable books:

- the daemon's create + sync store exactly the rows the CLI's
  `backends create` + `wallets sync-<kind>` store, for every exchange, on a
  plaintext book and a SQLCipher one;
- no kind but the explicit Test connection and Sync now contacts an
  exchange, and offline mode stops both before any request;
- an echoed key reaches no output, no table but `backends`, and not the
  session's last-sync record.

docs/reference/exchange-desktop.md describes the kinds.
"""
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import threading
import uuid

import pytest

from tests.rust_port.exchange_api import KEY, Binance, Bitpanda, Coinbase, Kraken, serve
from tests.rust_sync_env import child_env
from tests.test_rust_store_migrations import normalized_dump

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
if os.environ.get("CI") and not RUST_BIN:
    raise RuntimeError("CI must set KASSIBER_RUST_BIN; the desktop exchange test cannot skip")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires the kassiber test-harness build")

PASSPHRASE = "synthetic exchange desktop book"
SETUP_TIME = "2026-11-08T10:00:00Z"
TIME = "2026-11-08T12:00:00Z"
WALLET_ID = str(uuid.UUID(int=900))
IDS = [str(uuid.UUID(int=901 + n)) for n in range(64)]
EXCHANGES = {"kraken": Kraken, "coinbase": Coinbase, "binance": Binance, "bitpanda": Bitpanda}
KINDS = ["ui.connections.exchange.status", "ui.connections.exchange.test",
         "ui.connections.exchange.create", "ui.connections.exchange.sync"]


def environment(tmp_path, **extra):
    env = child_env(tmp_path)
    owners = tmp_path / "owners"
    owners.mkdir(mode=0o700, exist_ok=True)
    env.update({"KASSIBER_RUST_TEST_OWNER_ROOT": str(owners),
                "KASSIBER_RUST_TEST_SETUP_TIMESTAMP": SETUP_TIME})
    env.update(extra)
    # A child must never resolve the account's real app-data root.
    assert str(tmp_path) in env["XDG_DATA_HOME"]
    return env


def run(root, env, *args, secrets=(), stdin=None, encrypted=False):
    """One CLI command; `secrets` go through pipes named by `{fd}` placeholders."""
    reads = []
    argv = []
    for arg in args:
        if arg == "{fd}":
            read_fd, write_fd = os.pipe()
            os.write(write_fd, secrets[len(reads)].encode())
            os.close(write_fd)
            reads.append(read_fd)
            arg = str(read_fd)
        argv.append(arg)
    unlock = []
    if encrypted:
        read_fd, write_fd = os.pipe()
        os.write(write_fd, PASSPHRASE.encode())
        os.close(write_fd)
        reads.append(read_fd)
        unlock = ["--db-passphrase-fd", str(read_fd)]
    try:
        result = subprocess.run([RUST_BIN, "--machine", "--data-root", str(root), *unlock, *argv],
                                input=stdin, capture_output=True, text=True, env=env, timeout=300,
                                pass_fds=tuple(reads))
    finally:
        for fd in reads:
            os.close(fd)
    assert KEY not in result.stdout + result.stderr
    return result


def ok(result):
    assert result.returncode == 0, result.stdout + result.stderr
    return json.loads(result.stdout)


def daemon(root, env, requests, encrypted=False):
    """One daemon session; returns the ready record and each reply by request.

    Each request is sent once the previous one has answered, as the desktop
    awaits each reply. A user-requested sync (`ui.wallets.sync`) runs on the
    daemon's freshness worker, and shutdown, like lock or a book switch,
    cancels and joins that worker by design (docs/reference/rust-shell.md), so
    a shutdown written while it still runs would cancel the very sync the test
    asserts on.
    """
    lines = []
    if encrypted:
        lines.append({"request_id": "unlock", "kind": "daemon.unlock",
                      "args": {"auth_response": {"passphrase_secret": PASSPHRASE}}})
    lines += [{"request_id": str(i), "kind": kind, **({} if args is None else {"args": args})}
              for i, (kind, args) in enumerate(requests)]
    lines.append({"request_id": "stop", "kind": "daemon.shutdown"})
    process = subprocess.Popen([RUST_BIN, "--machine", "--data-root", str(root), "daemon"],
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               text=True, env=env)
    output, errors = queue.Queue(), []

    def drain():
        for raw in process.stdout:
            output.put(raw)
        output.put(None)

    readers = [threading.Thread(target=drain, daemon=True),
               threading.Thread(target=lambda: errors.append(process.stderr.read()), daemon=True)]
    for reader in readers:
        reader.start()
    raw_lines, records = [], []

    def record():
        raw = output.get(timeout=600)
        assert raw is not None, "the daemon closed its output early: " + "".join(raw_lines)
        raw_lines.append(raw)
        records.append(json.loads(raw))
        return records[-1]

    try:
        record()  # the ready record
        for line in lines:
            process.stdin.write(json.dumps(line) + "\n")
            process.stdin.flush()
            while True:
                reply = record()
                if reply.get("request_id") == line["request_id"] and not reply["kind"].endswith(".progress"):
                    break
        process.stdin.close()
        returncode = process.wait(timeout=600)
        for reader in readers:
            reader.join(timeout=600)
        while (raw := output.get(timeout=600)) is not None:
            raw_lines.append(raw)
            records.append(json.loads(raw))
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
    stdout, stderr = "".join(raw_lines), "".join(errors)
    assert returncode == 0, stdout + stderr
    assert KEY not in stdout + stderr
    replies = {item.get("request_id"): item for item in records[1:]}
    if encrypted:
        assert replies["unlock"]["kind"] == "daemon.unlock", replies["unlock"]
    return records[0], [replies[str(i)] for i in range(len(requests))]


def new_book(tmp_path, encrypted=False):
    root = tmp_path / "base" / "data"
    env = environment(tmp_path)
    ok(run(root, env, "init"))
    ok(run(root, env, "workspaces", "create", "Main"))
    ok(run(root, env, "profiles", "create", "Book", "--fiat-currency", "EUR"))
    if encrypted:
        ok(run(root, env, "secrets", "init", "--new-passphrase-fd", "{fd}", secrets=[PASSPHRASE]))
        for leftover in root.glob("kassiber.pre-enc*"):
            leftover.unlink()
    return root


def copy_book(base, tmp_path, name):
    root = tmp_path / name / "data"
    shutil.copytree(base, root)
    return root


def dump(root, encrypted=False):
    database = root / "kassiber.sqlite3"
    if encrypted:
        from kassiber.secrets.sqlcipher import open_encrypted
        conn = open_encrypted(database, PASSPHRASE, enforce_operator_identity=False)
    else:
        import sqlite3
        conn = sqlite3.connect(database)
    try:
        return normalized_dump(conn)[1]
    finally:
        conn.close()


def credentials(exchange, backend):
    args = {"exchange": exchange.kind, "api_key": backend["token"], "url": backend["url"]}
    if exchange.kind != "bitpanda":
        args["api_secret"] = backend["auth_header"]
    return args


def cli_connect(root, env, exchange, backend, encrypted=False):
    """`backends create` with the key on stdin and the secret on a descriptor."""
    args = ["backends", "create", exchange.kind, "--kind", exchange.kind, "--url", backend["url"],
            "--token-stdin"]
    secrets = []
    if exchange.kind != "bitpanda":
        args += ["--auth-header-fd", "{fd}"]
        secrets.append(backend["auth_header"])
    return ok(run(root, env, *args, secrets=secrets, stdin=backend["token"] + "\n", encrypted=encrypted))


CASES = [("kraken", False), ("coinbase", False), ("binance", False), ("bitpanda", False),
         ("kraken", True), ("bitpanda", True)]


@pytest.mark.parametrize(("kind", "encrypted"), CASES, ids=[f"{k}-{'sqlcipher' if e else 'sqlite'}" for k, e in CASES])
def test_desktop_create_and_sync_store_the_cli_rows(tmp_path, kind, encrypted):
    base = new_book(tmp_path, encrypted)
    exchange = EXCHANGES[kind]()
    with serve(exchange) as backend:
        # The CLI: the backend, then the sync, which creates the provider
        # wallet with its first identity.
        cli_root = copy_book(base, tmp_path, "cli")
        cli_env = environment(tmp_path, KASSIBER_RUST_TEST_IMPORT_IDS=",".join([WALLET_ID, *IDS]),
                              KASSIBER_RUST_TEST_IMPORT_TIME=TIME)
        cli_connect(cli_root, cli_env, exchange, backend, encrypted)
        cli_sync = ok(run(cli_root, cli_env, "wallets", "sync-" + kind, "--backend", kind, encrypted=encrypted))
        cli_requests = list(exchange.requests)
        exchange.reset()

        # The desktop: the connection creates that wallet, then Sync now
        # imports with the identities that follow.
        desktop_root = copy_book(base, tmp_path, "desktop")
        desktop_env = environment(tmp_path, KASSIBER_RUST_TEST_WALLET_ID=WALLET_ID,
                                  KASSIBER_RUST_TEST_WALLET_TIME=TIME,
                                  KASSIBER_RUST_TEST_IMPORT_IDS=",".join(IDS),
                                  KASSIBER_RUST_TEST_IMPORT_TIME=TIME)
        _, (created, synced, status) = daemon(desktop_root, desktop_env, [
            ("ui.connections.exchange.create", credentials(exchange, backend)),
            ("ui.connections.exchange.sync", {"wallet": kind.capitalize()}),
            ("ui.connections.exchange.status", None),
        ], encrypted)
        desktop_requests = list(exchange.requests)

    assert created["kind"] == "ui.connections.exchange.create", created
    assert created["data"]["wallet"] == {"id": WALLET_ID, "label": exchange.kind.capitalize(), "kind": kind}
    assert created["data"]["backend"]["name"] == kind
    assert synced["kind"] == "ui.connections.exchange.sync", synced
    assert synced["data"] == cli_sync["data"]
    assert synced["data"]["imported"] > 0
    assert desktop_requests == cli_requests
    assert dump(desktop_root, encrypted) == dump(cli_root, encrypted)
    [connection] = status["data"]["connections"]
    assert connection["wallet_id"] == WALLET_ID and connection["backend"] == kind
    assert connection["has_api_key"] is True
    assert connection["has_api_secret"] is (None if kind == "bitpanda" else True)
    assert connection["last_sync"]["status"] == "ok"
    assert connection["last_sync"]["imported"] == synced["data"]["imported"]


def test_only_the_explicit_test_and_sync_contact_the_exchange(tmp_path):
    root = new_book(tmp_path)
    env = environment(tmp_path)
    exchange = Kraken()
    with serve(exchange) as backend:
        ready, replies = daemon(root, env, [
            ("status", None),
            ("ui.connections.exchange.create", credentials(exchange, backend)),
            ("ui.connections.exchange.status", None),
            ("ui.connections.exchange.status", {"wallet": "Kraken"}),
            ("ui.wallets.list", None),
            ("ui.overview.snapshot", None),
            ("ui.backends.list", None),
            # Exchange wallets stay sourceless for the ordinary refresh.
            ("ui.wallets.sync", {"wallet": "Kraken"}),
        ])
        assert exchange.requests == []
        assert set(KINDS) <= set(ready["data"]["supported_kinds"])
        assert [reply["kind"] for reply in replies] == [
            "status", "ui.connections.exchange.create", "ui.connections.exchange.status",
            "ui.connections.exchange.status", "ui.wallets.list", "ui.overview.snapshot", "ui.backends.list",
            "ui.wallets.sync"]
        assert replies[2]["data"]["connections"][0]["last_sync"] is None

        # A new session after launch: still nothing until the user asks.
        daemon(root, env, [("status", None), ("ui.connections.exchange.status", None)])
        assert exchange.requests == []

        # Offline mode stops both explicit actions before any request.
        ok(run(root, env, "offline", "on"))
        _, (tested, synced, status) = daemon(root, env, [
            ("ui.connections.exchange.test", {"backend": "kraken"}),
            ("ui.connections.exchange.sync", {"wallet": "Kraken"}),
            ("ui.connections.exchange.status", {"wallet": "Kraken"}),
        ])
        assert tested["error"]["code"] == "network_egress_disabled", tested
        assert synced["error"]["code"] == "network_egress_disabled", synced
        assert status["data"]["connections"][0]["last_sync"]["error"]["code"] == "network_egress_disabled"
        assert exchange.requests == []
        ok(run(root, env, "offline", "off"))

        # The explicit check reads the first page of both histories only.
        _, (tested,) = daemon(root, env, [("ui.connections.exchange.test", {"backend": "kraken"})])
        assert tested["data"] == {"exchange": "kraken", "ok": True, "read_only": None}, tested
        assert [(r["path"], r["params"]) for r in exchange.requests] == [
            ("/0/private/TradesHistory", [("ofs", "0")]), ("/0/private/Ledgers", [("ofs", "0")])]
        exchange.reset()

        # A pasted key is checked without being stored.
        before = dump(root)
        _, (pasted,) = daemon(root, env, [("ui.connections.exchange.test", credentials(exchange, backend))])
        assert pasted["data"]["ok"] is True
        assert dump(root) == before


def test_a_binance_key_that_can_trade_is_refused_by_the_check(tmp_path):
    root = new_book(tmp_path)
    env = environment(tmp_path)
    exchange = Binance()
    exchange.restrictions = {**Binance.restrictions, "enableSpotAndMarginTrading": True}
    with serve(exchange) as backend:
        _, (refused,) = daemon(root, env, [("ui.connections.exchange.test", credentials(exchange, backend))])
    assert refused["error"]["code"] == "auth_error", refused
    assert refused["error"]["details"] == {"permissions": ["enableSpotAndMarginTrading"]}
    assert [r["path"] for r in exchange.requests] == ["/sapi/v1/account/apiRestrictions"]


def test_an_echoed_key_is_withheld_from_output_tables_and_status(tmp_path):
    root = new_book(tmp_path)
    env = environment(tmp_path)
    exchange = Bitpanda()
    with serve(exchange) as backend:
        daemon(root, env, [("ui.connections.exchange.create", credentials(exchange, backend))])
        exchange.faults[1] = (401, {}, json.dumps({"error": {"code": f"invalid key {KEY}"}}).encode())
        _, (synced, status) = daemon(root, env, [
            ("ui.connections.exchange.sync", {"wallet": "Bitpanda"}),
            ("ui.connections.exchange.status", None),
        ])
    assert synced["error"]["code"] == "auth_error", synced
    assert synced["error"]["details"]["bitpanda_error"] == "invalid key <redacted>"
    error = status["data"]["connections"][0]["last_sync"]["error"]
    assert error == synced["error"]
    for table, (_, rows) in dump(root).items():
        if table != "backends":
            assert KEY not in json.dumps(rows, default=str), table


def test_requests_are_checked_before_any_work(tmp_path):
    root = new_book(tmp_path)
    env = environment(tmp_path)
    exchange = Kraken()
    with serve(exchange) as backend:
        before = dump(root)
        _, replies = daemon(root, env, [
            ("ui.connections.exchange.create", {**credentials(exchange, backend), "token": "x"}),
            ("ui.connections.exchange.create", {"exchange": "kraken", "api_key": KEY}),
            ("ui.connections.exchange.create", {"exchange": "esplora", "api_key": KEY, "api_secret": "s"}),
            ("ui.connections.exchange.test", {"backend": "kraken", "api_key": KEY}),
            ("ui.connections.exchange.sync", {}),
            ("ui.connections.exchange.sync", {"wallet": "nope"}),
        ])
        assert exchange.requests == []
        assert dump(root) == before
    codes = [reply["error"]["code"] for reply in replies]
    assert codes == ["validation", "validation", "validation", "validation", "validation", "not_found"], replies
    assert replies[0]["error"]["details"] == {"unsupported": ["token"]}

    # A second connection to the same exchange gets its own backend; a label
    # that is taken is refused without storing a key.
    with serve(Kraken()) as backend:
        _, (first, second, taken) = daemon(root, env, [
            ("ui.connections.exchange.create", credentials(exchange, backend)),
            ("ui.connections.exchange.create", {**credentials(exchange, backend), "label": "Kraken 2"}),
            ("ui.connections.exchange.create", credentials(exchange, backend)),
        ])
    assert (first["data"]["backend"]["name"], second["data"]["backend"]["name"]) == ("kraken", "kraken-2")
    assert taken["error"]["code"] == "conflict", taken
    columns, rows = dump(root)["backends"]
    names = [row[columns.index("name")][1] for row in rows]
    assert [name for name in names if name.startswith("kraken")] == ["kraken", "kraken-2"]
