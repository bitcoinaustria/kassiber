"""Silent-payment wallets in `wallets sync` against unchanged Python.

A silent-payment wallet syncs from its backend's private local scanner file
or, in server-assisted mode, one POST to the backend's scan path on a
loopback fake (docs/reference/rust-native-wallet-gaps.md, S6). Output and
every manifest table compare with Python's CLI on the same starting bytes,
with generated ids and clocks masked; the hybrid log proves Rust served.
"""
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import sqlite3

import pytest

from tests.rust_sync_parity import fixed_elapsed_ms
from tests.test_rust_connections import masked, masked_dump, started_stamp
from tests.test_rust_lightning_service import BIN, book

pytestmark = pytest.mark.skipif(not BIN, reason="requires Rust test-harness binary")

SP_DESCRIPTOR = "sp(spscan1q" + ("q" * 40) + ")"
FINGERPRINT = hashlib.sha256(SP_DESCRIPTOR.encode()).hexdigest()
P2TR = "5120" + ("ab" * 32)
RECEIVE = "11" * 32
SPEND = "22" * 32


def scan(*, complete=True, spent=False, **extra):
    output = {"txid": RECEIVE, "vout": 0, "amount_sats": 50_000, "script_pubkey": P2TR, "silent_payment": True,
              "block_height": 850_100, "block_time": "2026-06-01T12:00:00Z", "confirmations": 6}
    # A spend marks the output through its owned input, not the UTXO row.
    txs = [{"txid": RECEIVE, "block_height": 850_100, "block_time": "2026-06-01T12:00:00Z", "outputs": [output]}]
    if spent:
        txs.append({"txid": SPEND, "block_height": 850_120, "block_time": 1780401600, "fee_sats": 1_000,
                    "inputs": [{"txid": RECEIVE, "vout": 0, "amount_sats": 50_000, "silent_payment": True}],
                    "outputs": [{"amount_sats": 49_000, "script_pubkey": "0014" + ("cd" * 20)}]})
    payload = {"complete": complete, "descriptor_fingerprint": FINGERPRINT,
               "range": {"from_height": 850_000, "to_height": 850_130}, "transactions": txs, "utxos": [output]}
    payload.update(extra)
    return payload


def write_scan(path, payload, mode=0o600):
    path.write_text(payload if isinstance(payload, str) else json.dumps(payload), encoding="utf-8")
    path.chmod(mode)


def sp_book(tmp_path, monkeypatch, backend, **config):
    root, database, env, call = book(tmp_path, monkeypatch, backend.get("kind", "custom"), {"name": "fake", **backend})
    conn = sqlite3.connect(database)
    try:
        settings = {"sp_descriptor": SP_DESCRIPTOR, "backend": "fake", "chain": "bitcoin", "network": "main",
                    "sp_material_format": "bip392-spscan", "sp_scan_mode": "local_index",
                    "sp_scan_start_height": 850_000, "sp_full_history": False, **config}
        conn.execute("UPDATE wallets SET kind='silent-payment',config_json=? WHERE id='cold'", (json.dumps(settings),))
        conn.commit()
    finally:
        conn.close()
    return root, database, env, call


def compare(root, database, env, call, argv, hits=None, replace=()):
    before = database.read_bytes()
    started = started_stamp()
    outcomes = []
    log = Path(env["KASSIBER_HYBRID_LOG"])
    for reference in [True, False]:
        database.write_bytes(before)
        log.unlink(missing_ok=True)
        if hits is not None:
            hits.clear()
        result = call("--machine", *argv, reference=reference)
        stdout = result.stdout.replace(str(root).encode(), b"ROOT")
        for old, new in replace:
            stdout = stdout.replace(old.encode(), new.encode())
        payload = fixed_elapsed_ms(json.loads(masked(stdout, started)))
        outcomes.append((result.returncode, payload, result.stderr, list(hits or []), masked_dump(root, started)))
        if not reference:
            served = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
            assert served and {row["served_by"] for row in served} == {"rust"}, served
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


def local(tmp_path, kind="custom", **extra):
    target = tmp_path / "scan.json"
    url = "local://silent-payments" if kind == "custom" else "http://127.0.0.1:9/"
    return target, {"kind": kind, "url": url, "chain": "bitcoin", "network": "main",
                    "silent_payment_scan_file": str(target), "silent_payments": True, **extra}


@pytest.mark.parametrize("kind", ["custom", "esplora", "electrum", "bitcoinrpc"])
def test_local_scanner_receive_spend_and_partial_match_python(tmp_path, monkeypatch, kind):
    """A complete receive, the same scan again, a spend (custom backends are
    never authoritative; Esplora, Electrum and Core RPC are, for a complete
    timestamped scan) and then a partial scan that must not spend missing
    outputs."""
    target, backend = local(tmp_path, kind)
    if kind == "electrum":
        backend["url"] = "tcp://127.0.0.1:9"
    root, database, env, call = sp_book(tmp_path, monkeypatch, backend)
    write_scan(target, scan())
    first = compare(root, database, env, call, ["wallets", "sync", "--wallet", "Cold"])
    row = first["data"][0]
    assert row["status"] == "synced" and row["sync_mode"] == "silent_payment", row
    assert row["output_inventory"]["active"] == 1, row
    compare(root, database, env, call, ["wallets", "sync", "--wallet", "Cold"])
    write_scan(target, scan(spent=True))
    spent = compare(root, database, env, call, ["wallets", "sync", "--all"])
    assert {r["wallet"]: r for r in spent["data"]}["Cold"]["records_fetched"] == 2, spent
    write_scan(target, scan(complete="false", transactions=[], utxos=[]))
    partial = compare(root, database, env, call, ["wallets", "sync", "--wallet", "Cold"])
    row = partial["data"][0]
    assert row["partial_success"] is True and row["utxos_skipped_partial"] is True, row


def _observed(database):
    conn = sqlite3.connect(database)
    try:
        return conn.execute("SELECT COUNT(*) FROM chain_observation_provenance").fetchone()[0]
    finally:
        conn.close()


@pytest.mark.parametrize("shape", ["incomplete", "untimestamped"])
def test_core_rpc_scans_are_authoritative_only_when_complete(tmp_path, monkeypatch, shape):
    """Python's ordinary adapter path granted every Core RPC scan observer
    authority; since round 14 an incomplete or untimestamped scan publishes
    without it there, as on the compatibility route, and a complete
    timestamped scan still publishes with it."""
    target, backend = local(tmp_path, "bitcoinrpc")
    root, database, env, call = sp_book(tmp_path, monkeypatch, backend)
    payload = scan()
    if shape == "incomplete":
        payload["complete"] = False
    else:
        for tx in payload["transactions"]:
            tx.pop("block_time")
    write_scan(target, payload)
    row = compare(root, database, env, call, ["wallets", "sync", "--wallet", "Cold"])["data"][0]
    assert row["status"] == "synced", row
    assert _observed(database) == 0
    write_scan(target, scan())
    compare(root, database, env, call, ["wallets", "sync", "--wallet", "Cold"])
    assert _observed(database) > 0


@pytest.mark.parametrize("case", ["unconfirmed", "range", "full-history", "date", "raw-echo", "amount", "label",
                                  "spend-only"])
def test_scan_payload_shapes_match_python(tmp_path, monkeypatch, case):
    target, backend = local(tmp_path, "esplora")
    config = {}
    payload = scan()
    if case == "unconfirmed":
        for tx in payload["transactions"]:
            tx.pop("block_time")
        payload["utxos"][0].pop("block_time")
        payload["utxos"][0]["confirmations"] = 0
        payload["utxos"][0].pop("block_height")
    elif case == "range":
        payload["range"] = {"start_height": 850_001}
    elif case == "full-history":
        config = {"sp_full_history": True, "sp_acknowledge_full_history_warning": True}
        payload["range"] = {"from_date": "2009-01-03T00:00:00Z"}
    elif case == "date":
        config = {"sp_scan_start_date": "2026-01-01T00:00:00Z"}
        payload["range"] = {"from_height": 850_000, "start_time": "2025-12-31T00:00:00+00:00"}
    elif case == "raw-echo":
        payload["range"]["request_echo"] = SP_DESCRIPTOR
        payload["transactions"][0]["outputs"][0]["raw"] = {"scanDescriptor": SP_DESCRIPTOR, "note": "kept"}
        payload["utxos"][0]["raw"] = {"scanDescriptor": SP_DESCRIPTOR, "note": "kept"}
    elif case == "amount":
        # The scan's UTXO row is the transaction's output object.
        output = payload["transactions"][0]["outputs"][0]
        output.pop("amount_sats")
        output["amount"] = "0.00050000"
        payload["degraded_reason"] = "operator_note"
    elif case == "label":
        payload.pop("descriptor_fingerprint")
        payload["wallet"] = {"wallet_id": "cold", "wallet_label": "Cold"}
    elif case == "spend-only":
        # A spend whose receive the scan omits: the negative balance takes
        # the widened repair scan before publishing.
        payload = scan(spent=True)
        payload["transactions"] = payload["transactions"][1:]
        payload["utxos"] = []
        # More than the wallet's stored balance, so a deficit appears.
        payload["transactions"][0]["inputs"][0]["amount_sats"] = 200_000_000
        payload["transactions"][0]["outputs"][0]["amount_sats"] = 199_999_000
    root, database, env, call = sp_book(tmp_path, monkeypatch, backend, **config)
    write_scan(target, payload)
    compare(root, database, env, call, ["wallets", "sync", "--wallet", "Cold"])


@pytest.mark.parametrize("case", ["mode", "missing", "symlink", "directory", "json", "mismatch", "unbound",
                                  "label-mismatch", "txid", "fee", "owned", "script", "negative", "payload",
                                  "incapable", "no-scanner", "network", "kind"])
def test_scanner_failures_match_python(tmp_path, monkeypatch, case):
    target, backend = local(tmp_path)
    payload = scan()
    mode = 0o600
    if case == "mode":
        mode = 0o644
    elif case == "json":
        payload = "{not json"
    elif case == "mismatch":
        payload["descriptor_fingerprint"] = "00" * 32
    elif case == "unbound":
        payload.pop("descriptor_fingerprint")
    elif case == "label-mismatch":
        payload["wallet_label"] = "Hot"
    elif case == "txid":
        payload["transactions"][0]["txid"] = "zz"
    elif case == "fee":
        payload = scan(spent=True)
        payload["transactions"][1].pop("fee_sats")
    elif case == "owned":
        payload["transactions"][0]["outputs"][0]["silent_payment"] = False
    elif case == "script":
        payload["utxos"][0]["script_pubkey"] = "0014" + "cd" * 20
    elif case == "negative":
        payload["utxos"][0]["amount_sats"] = -1
    elif case == "payload":
        payload = [1, 2]
    elif case == "incapable":
        backend["silent_payments"] = False
    elif case == "no-scanner":
        backend.pop("silent_payment_scan_file")
        backend["sp_capable"] = "yes"
    elif case == "network":
        backend["network"] = "testnet"
    elif case == "kind":
        backend["kind"] = "lnd"
    root, database, env, call = sp_book(tmp_path, monkeypatch, backend)
    if case == "symlink":
        real = tmp_path / "real.json"
        write_scan(real, payload)
        target.symlink_to(real)
    elif case == "directory":
        target.mkdir()
    elif case != "missing":
        write_scan(target, payload, mode)
    result = compare(root, database, env, call, ["wallets", "sync", "--all"], replace=[(str(tmp_path), "TMP")])
    rows = {r["wallet"]: r for r in result["data"]}
    assert rows["Cold"]["status"] == "error", rows


@contextmanager
def scanner(responses):
    from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
    import threading
    hits = []

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
            hits.append((self.path, self.headers.get("Content-Type"), self.headers.get("Accept"), body.decode()))
            status, payload = responses[min(len(hits), len(responses)) - 1]
            response = payload if isinstance(payload, bytes) else json.dumps(payload).encode()
            self.send_response(status)
            self.send_header("Content-Length", str(len(response)))
            self.end_headers()
            self.wfile.write(response)

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


@pytest.mark.parametrize("case", ["complete", "partial", "unconfigured", "electrum", "status", "invalid"])
def test_server_assisted_scan_matches_python(tmp_path, monkeypatch, case):
    """One POST of the wallet's scan request to the backend's scan path, even
    with a local file configured; nothing reaches the scanner otherwise."""
    responses = {"complete": [(200, scan())], "partial": [(200, scan(complete=False))],
                 "status": [(404, {"error": "nope"})], "invalid": [(200, b"[")]}.get(case, [(200, scan())])
    with scanner(responses) as (url, hits):
        target, backend = local(tmp_path, "esplora", sp_scan_path="/sp/scan")
        backend["url"] = url + "/api"
        if case == "unconfigured":
            backend.pop("sp_scan_path")
        elif case == "electrum":
            backend.update(kind="electrum", url="tcp://127.0.0.1:9")
        write_scan(target, {"descriptor_fingerprint": "00" * 32})
        root, database, env, call = sp_book(tmp_path, monkeypatch, backend, sp_scan_mode="server_assisted",
                                            sp_acknowledge_server_warning=True)
        compare(root, database, env, call, ["wallets", "sync", "--wallet", "Cold"], hits=hits,
                replace=[(url, "URL")])
        if case in {"unconfigured", "electrum"}:
            assert hits == []
        else:
            assert hits and hits[0][0] == "/api/sp/scan", hits


@pytest.mark.parametrize("mode", ["custom", "esplora", "server"])
def test_offline_mode_matches_python(tmp_path, monkeypatch, mode):
    """Offline mode refuses a scan that needs the network before any request;
    a local scanner file needs none."""
    with scanner([(200, scan())]) as (url, hits):
        target, backend = local(tmp_path, "custom" if mode == "custom" else "esplora")
        config = {}
        if mode == "server":
            backend.update(url=url, sp_scan_path="/scan")
            config = {"sp_scan_mode": "server_assisted", "sp_acknowledge_server_warning": True}
        write_scan(target, scan())
        root, database, env, call = sp_book(tmp_path, monkeypatch, backend, **config)
        (tmp_path / "offline.json").write_text(json.dumps({"schema_version": 1, "enabled": True}))
        compare(root, database, env, call, ["wallets", "sync", "--wallet", "Cold"], hits=hits,
                replace=[(url, "URL")])
        assert hits == []


@pytest.mark.parametrize("kind", ["custom", "esplora"])
def test_direct_wallet_sync_events_match_python(tmp_path, monkeypatch, kind):
    """The desktop's `ui.wallets.sync` reaches a silent-payment wallet only
    through the direct sync; its progress events and rows match Python's."""
    from tests.test_rust_freshness_sources import _rust_lines

    target, backend = local(tmp_path, kind)
    root, database, env, call = sp_book(tmp_path, monkeypatch, backend)
    write_scan(target, scan(spent=True))
    before = database.read_bytes()
    started = started_stamp()
    requests = [("ui.wallets.sync", {"wallet": "Cold", "process_journals": False})]
    wire = b"".join((json.dumps({"request_id": str(i), "kind": kind, "args": args}) + "\n").encode()
                    for i, (kind, args) in enumerate(requests)) + b'{"request_id":"stop","kind":"daemon.shutdown"}\n'
    outcomes = []
    for reference in [True, False]:
        database.write_bytes(before)
        if reference:
            result = call("daemon", reference=True, wire=wire)
            assert (result.returncode, result.stderr) == (0, b""), result.stderr
            lines = result.stdout.splitlines()[1:]
        else:
            lines = _rust_lines(root, env, requests)
        rows = [fixed_elapsed_ms(json.loads(masked(line, started))) for line in lines]
        outcomes.append((rows, masked_dump(root, started)))
    assert outcomes[1] == outcomes[0]
