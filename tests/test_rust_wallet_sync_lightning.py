"""Lightning node wallets in `wallets sync` against unchanged Python.

A Core Lightning wallet syncs through a fake `lightning-cli` that forwards
each read method to a loopback node fake (docs/reference/rust-native-wallet-gaps.md,
S7). Output and every manifest table compare with Python's CLI on the same
starting bytes, with generated ids and clocks masked.
"""
from contextlib import contextmanager
import json
from pathlib import Path
import re

import pytest

from tests.test_rust_cln_acquisition import node as cln_node
from tests.test_rust_connections import masked, masked_dump, started_stamp
from tests.test_rust_lightning_service import BIN, ROOT, backend, book

pytestmark = pytest.mark.skipif(not BIN, reason="requires Rust test-harness binary")

PAID = "a1" * 32
PAY = "b2" * 32
FUNDING = "c3" * 32
CLOSING = "d4" * 32

NODE = {
    "getinfo": {"id": "02" + "11" * 32, "alias": "Node", "network": "regtest", "blockheight": 120, "num_peers": 2},
    "listfunds": {"outputs": [{"amount_msat": "5000msat", "status": "confirmed", "address": "bcrt1-private"}]},
    "listpeerchannels": {"channels": [
        {"channel_id": "chan-1", "short_channel_id": "101x1x0", "peer_id": "peer-a", "state": "CHANNELD_NORMAL",
         "total_msat": 2_000_000, "to_us_msat": 1_000_000, "funding": {"txid": FUNDING, "outnum": 0}},
        {"channel_id": "chan-2", "short_channel_id": "102x1x0", "peer_id": "peer-b", "state": "CHANNELD_NORMAL",
         "total_msat": 3_000_000, "to_us_msat": 0, "funding_txid": "e5" * 32, "funding_outnum": 1},
    ]},
    "listforwards": {"forwards": [
        {"in_channel": "102x1x0", "out_channel": "101x1x0", "fee_msat": 1500, "out_msat": 100_000,
         "status": "settled", "resolved_time": 1736463600},
        {"in_channel": "102x1x0", "out_channel": "101x1x0", "fee_msat": "2500msat", "out_msat": "200000msat",
         "status": "settled", "received_time": 1736467200},
        {"in_channel": "102x1x0", "out_channel": "101x1x0", "fee_msat": 9, "status": "failed",
         "received_time": 1736467200},
        {"in_channel": "101x1x0", "out_channel": "102x1x0", "fee_msat": 700, "out_msat": 50_000,
         "status": "settled", "resolved_time": 1736553600},
    ]},
    "listpays": {"pays": [
        {"payment_hash": PAY, "amount_msat": 10_000, "amount_sent_msat": 10_250, "status": "complete",
         "completed_at": 1736463000, "destination": "02" + "22" * 32, "payment_preimage": "private"},
        {"amount_msat": 4000, "amount_sent_msat": 4000, "status": "pending", "created_at": 1736463000},
        {"amount_msat": 3000, "amount_sent_msat": 3100, "status": "complete", "created_at": 1736460000,
         "rebalance": True},
    ]},
    "listinvoices": {"invoices": [
        {"label": "coffee", "payment_hash": PAID.upper(), "amount_received_msat": 25_000, "status": "paid",
         "paid_at": 1736460000, "description": "Coffee", "bolt11": "private"},
        {"label": "unpaid", "amount_msat": 99, "status": "unpaid", "created_at": 1736460000},
        {"label": "labelled", "amount_msat": "1sat", "status": "paid", "paid_at": 1736470000},
    ]},
    "bkpr-listincome": {"income_events": [
        {"account": "chan-1", "tag": "invoice", "credit_msat": 25_000, "debit_msat": 0, "timestamp": 1736460000,
         "payment_id": PAID, "description": "Coffee income", "currency": "bcrt"},
        {"account": "wallet", "tag": "invoice", "credit_msat": 7000, "txid": "f6" * 32, "timestamp": 1736460000},
        {"account": "chan-1", "tag": "routed", "credit_msat": 1500, "timestamp": 1736463600},
    ]},
    "bkpr-listbalances": {"accounts": [
        {"account": "wallet", "balances": [{"balance_msat": 5000, "coin_type": "bcrt"}]},
        {"account": "chan-1", "peer_id": "peer-a", "balances": [{"balance_msat": "1000000msat"}],
         "account_closed": False},
    ]},
    "bkpr-listaccountevents": {"events": [
        {"account": "chan-1", "tag": "channel_open", "outpoint": f"{FUNDING}:0", "credit_msat": 1_000_000,
         "timestamp": 1736400000},
        {"account": "chan-3", "tag": "channel_close", "txid": CLOSING, "debit_msat": 400_000,
         "timestamp": 1736500000},
        {"account": "chan-3", "tag": "channel_open", "txid": "a7" * 32, "credit_msat": 400_000},
    ]},
}


def _private(payload):
    """Native errors omit what Python discloses there: lightning-cli's command
    and stderr, and a malformed amount's value (rust-shell.md, Core Lightning
    report acquisition)."""
    for row in payload.get("data") or []:
        if isinstance(row, dict) and row.get("status") == "error":
            row["details"] = None
            row["message"] = re.sub(r"^(Invalid Core Lightning msat value) '.*'$", r"\1", row["message"])
    return payload


def _compare(root, database, call, hits, argv, url):
    before = database.read_bytes()
    started = started_stamp()
    outcomes = []
    for reference in [True, False]:
        database.write_bytes(before)
        hits.clear()
        log = root.parent / "served.jsonl"
        log.unlink(missing_ok=True)
        result = call("--machine", *argv, reference=reference)
        if not reference:
            assert {json.loads(line)["served_by"] for line in log.read_text().splitlines()} == {"rust"}
        stdout = masked(result.stdout.replace(url.encode(), b"URL").replace(str(root).encode(), b"ROOT"), started)
        outcomes.append((result.returncode, _private(json.loads(stdout)), result.stderr,
                         [[arg for arg in args if not arg.startswith("--rpc-file=")] for args in hits],
                         masked_dump(root, started)))
    for table in outcomes[0][4]:
        if outcomes[1][4][table] != outcomes[0][4][table]:
            want, got = outcomes[0][4][table], outcomes[1][4][table]
            for a, b in zip(want, got):
                if a != b:
                    a, b = json.loads(a), json.loads(b)
                    raise AssertionError((table, [(i, x, y) for i, (x, y) in enumerate(zip(a, b)) if x != y]))
            raise AssertionError((table, len(want), len(got)))
    assert outcomes[1][:4] == outcomes[0][:4]
    return outcomes[1][1]


@pytest.mark.parametrize("selection", [["--wallet", "Cold"], ["--all"]], ids=["wallet", "all"])
def test_core_lightning_wallet_sync_matches_python(tmp_path, monkeypatch, selection):
    with cln_node(NODE) as (url, hits):
        root, database, env, call = book(tmp_path, monkeypatch, "coreln", backend(tmp_path, "coreln", url))
        first = _compare(root, database, call, hits, ["wallets", "sync", *selection], url)
        again = _compare(root, database, call, hits, ["wallets", "sync", *selection], url)
    rows = {row["wallet"]: row for row in first["data"]}
    assert rows["Cold"]["status"] == "synced", rows["Cold"]
    assert rows["Cold"]["transactions"]["imported"] >= 3, rows["Cold"]
    assert {row["wallet"]: row for row in again["data"]}["Cold"]["records_inserted"] == 0
    served = [json.loads(line) for line in Path(env["KASSIBER_HYBRID_LOG"]).read_text().splitlines()]
    assert {row["served_by"] for row in served} == {"rust"}


@pytest.mark.parametrize("failure", ["getinfo", "listinvoices", "bkpr-listaccountevents"])
def test_core_lightning_method_failures_match_python(tmp_path, monkeypatch, failure):
    """`getinfo` must answer; another failed method is recorded and the sync
    is `partial`, which leaves the wallet's synced stamp alone."""
    with cln_node(NODE, failure=failure) as (url, hits):
        root, database, env, call = book(tmp_path, monkeypatch, "coreln", backend(tmp_path, "coreln", url))
        result = _compare(root, database, call, hits, ["wallets", "sync", "--all"], url)
    row = {row["wallet"]: row for row in result["data"]}["Cold"]
    assert row["status"] == ("error" if failure == "getinfo" else "partial"), row


def test_core_lightning_fixture_nodes_match_python(tmp_path, monkeypatch):
    """Reference node fixtures, including invalid amounts the sync refuses."""
    cases = json.loads((ROOT / "crates/chain/fixtures/python_cln.json").read_text())
    for index, case in enumerate([c for c in cases if "expected" in c][::97]):
        case_root = tmp_path / f"case-{index}"
        case_root.mkdir()
        with cln_node(case["data"]) as (url, hits):
            root, database, env, call = book(case_root, monkeypatch, "coreln", backend(case_root, "coreln", url))
            _compare(root, database, call, hits, ["wallets", "sync", "--all"], url)


def test_core_lightning_direct_wallet_sync_events_match_python(tmp_path, monkeypatch):
    """A Core Lightning wallet has no freshness job, so `ui.wallets.sync` syncs
    it directly: the wallet-labelled `backend_fetch` and import progress
    events, the answer and every table match Python's daemon."""
    from tests.test_rust_freshness_sources import _rust_lines

    with cln_node(NODE) as (url, hits):
        root, database, env, call = book(tmp_path, monkeypatch, "coreln", backend(tmp_path, "coreln", url))
        before = database.read_bytes()
        started = started_stamp()
        requests = [("ui.wallets.sync", {"wallet": "Cold", "process_journals": False})]
        wire = b"".join((json.dumps({"request_id": str(i), "kind": kind, "args": args}) + "\n").encode()
                        for i, (kind, args) in enumerate(requests)) + b'{"request_id":"stop","kind":"daemon.shutdown"}\n'
        outcomes = []
        for reference in [True, False]:
            database.write_bytes(before)
            hits.clear()
            if reference:
                result = call("daemon", reference=True, wire=wire)
                assert (result.returncode, result.stderr) == (0, b""), result.stderr
                lines = result.stdout.splitlines()[1:]
            else:
                lines = _rust_lines(root, env, requests)
            rows = [json.loads(masked(line.replace(url.encode(), b"URL"), started)) for line in lines]
            outcomes.append((rows, masked_dump(root, started)))
    assert outcomes[1][0] == outcomes[0][0]
    assert outcomes[1][1] == outcomes[0][1]
    kinds = [row["kind"] for row in outcomes[1][0]]
    assert "ui.wallets.sync.progress" in kinds, kinds


# --- LND -------------------------------------------------------------------------------------

def _b64(hex_text):
    import base64
    return base64.b64encode(bytes.fromhex(hex_text)).decode()


def lnd_data(resolve_ns="1736463000000000000"):
    return {
        "/v1/getinfo": {"identity_pubkey": "03" + "44" * 32, "alias": "LND", "chains": [{"chain": "bitcoin", "network": "regtest"}]},
        "payments": [
            {"payment_hash": "b2" * 32, "value_msat": "10000", "fee_msat": "250", "status": "SUCCEEDED",
             "payment_index": "1", "creation_date": "1736460000",
             "htlcs": [{"status": "SUCCEEDED", "resolve_time_ns": resolve_ns, "route": {"hops": ["private"]}}]},
            {"payment_hash": "c3" * 32, "value_sat": "5", "value": "5", "fee": "1", "status": "SUCCEEDED",
             "payment_index": "2", "creation_time_ns": "1736470000000000000"},
            {"payment_hash": "d4" * 32, "value_msat": "9", "status": "FAILED", "payment_index": "3"},
        ],
        "invoices": [
            {"r_hash": _b64("a1" * 32), "amt_paid_msat": "25000", "settled": True, "settle_date": "1736460000",
             "add_index": "1", "memo": "Coffee", "r_preimage": "private"},
            {"r_hash": _b64("e5" * 32), "value": "7", "state": "SETTLED", "settle_date": "1736461000", "add_index": "2"},
            {"r_hash": "not-a-hash", "value": "3", "state": "OPEN", "add_index": "3"},
        ],
        "forwarding_events": [
            {"timestamp": "1736463600", "chan_id_in": "1", "chan_id_out": "2", "fee_msat": "1500", "amt_in_msat": "101500", "amt_out_msat": "100000"},
            {"timestamp_ns": "1736553600000000000", "chan_id_in": "2", "chan_id_out": "1", "fee": "2", "amt_in": "52", "amt_out": "50"},
        ],
        "/v1/channels": {"channels": [
            {"channel_point": "f6" * 32 + ":0", "chan_id": "1", "initiator": True, "capacity": "200000"},
            {"channel_point": "a7" * 32 + ":1", "chan_id": "2", "initiator": False, "capacity": "300000"},
        ]},
        "/v1/channels/closed": {"channels": [
            {"channel_point": "b8" * 32 + ":0", "chan_id": "3", "open_initiator": "INITIATOR_LOCAL", "capacity": "50000",
             "closing_tx_hash": "c9" * 32, "settled_balance": "40000", "time_locked_balance": "1000",
             "resolutions": [{"outcome": "CLAIMED", "sweep_txid": "da" * 32, "outpoint": "c9" * 32 + ":1"}]},
        ]},
    }


@contextmanager
def lnd_node(state):
    from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
    import threading
    from urllib.parse import parse_qs, urlsplit
    hits = []

    class Handler(BaseHTTPRequestHandler):
        def respond(self):
            body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
            hits.append((self.command, self.path, json.loads(body) if body else None))
            data = state["data"]
            parts = urlsplit(self.path)
            query = {k: v[0] for k, v in parse_qs(parts.query).items()}
            if parts.path in {"/v1/payments", "/v1/invoices"}:
                key = parts.path.rsplit("/", 1)[1]
                first = query.get("index_offset") == "0"
                payload = {key: data[key] if first else [], "last_index_offset": str(len(data[key])) if first else "0"}
            elif parts.path == "/v1/switch":
                first = json.loads(body)["index_offset"] == "0"
                events = data["forwarding_events"]
                payload = {"forwarding_events": events if first else [], "last_offset_index": len(events) if first else 0}
            else:
                payload = data[parts.path]
            response = json.dumps(payload).encode()
            self.send_response(200)
            self.send_header("Content-Length", str(len(response)))
            self.end_headers()
            self.wfile.write(response)

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


@pytest.mark.parametrize("selection", [["--wallet", "Cold"], ["--all"]], ids=["wallet", "all"])
def test_lnd_wallet_sync_matches_python(tmp_path, monkeypatch, selection):
    """Settled invoices, succeeded payments and daily routing fees become
    wallet rows and channels lifecycle evidence; a later sync whose payment
    HTLC settled at another time corrects that payment's date (live
    settlement authority) in both implementations."""
    state = {"data": lnd_data()}
    with lnd_node(state) as (url, hits):
        root, database, env, call = book(tmp_path, monkeypatch, "lnd", backend(tmp_path, "lnd", url))
        first = _compare_lnd(root, database, call, hits, ["wallets", "sync", *selection], url)
        state["data"] = lnd_data(resolve_ns="1736466600000000000")
        _compare_lnd(root, database, call, hits, ["wallets", "sync", *selection], url)
    import sqlite3
    conn = sqlite3.connect(database)
    try:
        dates = conn.execute("SELECT occurred_at, confirmed_at FROM transactions WHERE external_id LIKE 'lnd:pay:b2%'").fetchall()
    finally:
        conn.close()
    assert dates == [("2025-01-09T23:50:00Z", "2025-01-09T23:50:00Z")], dates
    row = {row["wallet"]: row for row in first["data"]}["Cold"]
    assert row["status"] == "synced" and row["transactions"]["imported"] >= 4, row


def _compare_lnd(root, database, call, hits, argv, url):
    before = database.read_bytes()
    started = started_stamp()
    outcomes = []
    for reference in [True, False]:
        database.write_bytes(before)
        hits.clear()
        log = root.parent / "served.jsonl"
        log.unlink(missing_ok=True)
        result = call("--machine", *argv, reference=reference)
        if not reference:
            assert {json.loads(line)["served_by"] for line in log.read_text().splitlines()} == {"rust"}
        stdout = masked(result.stdout.replace(url.encode(), b"URL"), started)
        outcomes.append((result.returncode, _private(json.loads(stdout)), result.stderr, list(hits),
                         masked_dump(root, started)))
    for table in outcomes[0][4]:
        if outcomes[1][4][table] != outcomes[0][4][table]:
            want, got = outcomes[0][4][table], outcomes[1][4][table]
            for a, b in zip(want, got):
                if a != b:
                    a, b = json.loads(a), json.loads(b)
                    raise AssertionError((table, [(i, x, y) for i, (x, y) in enumerate(zip(a, b)) if x != y]))
            raise AssertionError((table, len(want), len(got)))
    assert outcomes[1][:4] == outcomes[0][:4]
    return outcomes[1][1]


def test_lnd_direct_wallet_sync_events_match_python(tmp_path, monkeypatch):
    from tests.test_rust_freshness_sources import _rust_lines

    state = {"data": lnd_data()}
    with lnd_node(state) as (url, hits):
        root, database, env, call = book(tmp_path, monkeypatch, "lnd", backend(tmp_path, "lnd", url))
        before = database.read_bytes()
        started = started_stamp()
        requests = [("ui.wallets.sync", {"wallet": "Cold", "process_journals": False})]
        wire = b"".join((json.dumps({"request_id": str(i), "kind": kind, "args": args}) + "\n").encode()
                        for i, (kind, args) in enumerate(requests)) + b'{"request_id":"stop","kind":"daemon.shutdown"}\n'
        outcomes = []
        for reference in [True, False]:
            database.write_bytes(before)
            hits.clear()
            if reference:
                result = call("daemon", reference=True, wire=wire)
                assert (result.returncode, result.stderr) == (0, b""), result.stderr
                lines = result.stdout.splitlines()[1:]
            else:
                lines = _rust_lines(root, env, requests)
            rows = [json.loads(masked(line.replace(url.encode(), b"URL"), started)) for line in lines]
            outcomes.append((rows, list(hits), masked_dump(root, started)))
    assert outcomes[1] == outcomes[0]
