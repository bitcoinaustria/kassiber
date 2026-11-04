"""Protocol observations only: no BDK state, retraction authority or book parity.

Python's existing fetch/decode helpers query the same loopback as the native
acquisition driver. The six discovery scenarios feed their actual targets into
this seam. Reorg and disappearance scenarios compare observed facts only.
"""
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import socketserver
import subprocess
import sys
import threading

from embit.script import Script, Witness
from embit.transaction import Transaction, TransactionInput, TransactionOutput
import pytest

from kassiber import __version__
from kassiber.core import sync_backends as reference
from kassiber.envelope import json_ready
from kassiber.wallet_descriptors import load_descriptor_plan, derive_descriptor_targets
from tests.rust_sync_env import child_env as _env
from tests.test_rust_wallet_discovery import _config, _server as discovery_server
from tests.test_rust_bitcoin_records import project
from kassiber.time_utils import timestamp_to_iso

ROOT = Path(__file__).resolve().parents[1]
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires Rust acquisition fixture build")


def _transactions(targets, *, reorg=False, coinbase=False):
    parent = Transaction(vin=[TransactionInput(b"\x00" * 32, 0xffffffff)],
                         vout=[TransactionOutput(100_003, Script(b"\x51"))])
    current = Transaction(vin=[TransactionInput(parent.txid(), 0, sequence=0xfffffffd,
                                               witness=Witness([b"\xab" * 73, b"\xcd" * 33]))],
                          vout=[TransactionOutput(99_001, Script(bytes.fromhex(targets[0]["script_pubkey"])))],
                          locktime=42)
    if coinbase:
        current = Transaction(vin=[TransactionInput(b"\x00" * 32, 0xffffffff, Script(b"\x01\x01"))],
                              vout=current.vout)
    txid = current.txid().hex()
    height = 101 if reorg else 100
    timestamp = 1_700_000_600 if reorg else 1_700_000_000
    raw = {parent.txid().hex(): parent.serialize().hex(), txid: current.serialize().hex()}
    tx = {"txid": txid, "vin": [{"txid": parent.txid().hex(), "vout": 0,
                                "prevout": {"scriptpubkey": "51", "value": 100_003},
                                "witness": ["ab" * 73, "cd" * 33]}],
          "vout": [{"scriptpubkey": targets[0]["script_pubkey"], "value": 99_001}],
          "status": {"confirmed": True, "block_height": height, "block_time": timestamp}}
    if coinbase:
        tx["vin"] = [{"txid": "0" * 64, "vout": 0xffffffff, "is_coinbase": True,
                      "scriptsig": "0101", "sequence": 0xffffffff}]
    return raw, tx, height, timestamp


@contextmanager
def _server(protocol, targets, *, empty=False, reorg=False, mempool=False, coinbase=False):
    hits = []
    raw, tx, height, timestamp = _transactions(targets, reorg=reorg, coinbase=coinbase)
    txid = tx["txid"]
    watched = {reference.scriptpubkey_scripthash(t["script_pubkey"]) for t in targets[:2]}
    if mempool:
        tx["status"] = {"confirmed": False}
        height = 0
    history = [] if empty else [tx]
    header = (b"\x00" * 68 + timestamp.to_bytes(4, "little") + b"\x00" * 8).hex()

    class Http(BaseHTTPRequestHandler):
        def do_GET(self):
            hits.append(self.path)
            parts = self.path.strip("/").split("/")
            if parts == ["blocks", "tip", "hash"]:
                body = b"a" * 64
            elif parts == ["blocks", "tip", "height"]:
                body = b"102"
            else:
                assert parts[0] == "scripthash"
                active = parts[1] in watched and not empty
                if len(parts) == 2:
                    result = {"chain_stats": {"tx_count": int(active and not mempool)},
                              "mempool_stats": {"tx_count": int(active and mempool)}}
                elif parts[2:] == ["txs", "chain"]:
                    result = history if active and not mempool else []
                elif parts[2:] == ["txs", "mempool"]:
                    result = history if active and mempool else []
                else:
                    assert parts[2:] == ["utxo"]
                    result = ([{"txid": txid, "vout": 0, "value": 99_001, "status": tx["status"]}]
                              if active else [])
                body = json.dumps(result).encode()
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_):
            pass

    class Electrum(socketserver.StreamRequestHandler):
        def handle(self):
            self.request.settimeout(5)
            while line := self.rfile.readline():
                request = json.loads(line)
                hits.append(request)
                method, params = request["method"], request["params"]
                if method == "server.version":
                    result = ["fixture", "1.6"]
                elif method == "blockchain.headers.subscribe":
                    result = {"height": 102, "hex": header}
                elif method == "blockchain.block.header":
                    assert params == [height]
                    result = header
                elif method == "blockchain.transaction.get":
                    result = raw[params[0]]
                else:
                    active = params[0] in watched and not empty
                    if method.endswith(".subscribe"):
                        result = "status" if active else None
                    elif method.endswith(".get_history"):
                        result = [{"tx_hash": txid, "height": height}] if active else []
                    else:
                        assert method.endswith(".listunspent")
                        result = [{"tx_hash": txid, "tx_pos": 0, "value": 99_001, "height": height}] if active else []
                self.wfile.write(json.dumps({"id": request["id"], "result": result}).encode() + b"\n")

    server = (ThreadingHTTPServer if protocol == "esplora" else socketserver.ThreadingTCPServer)(
        ("127.0.0.1", 0), Http if protocol == "esplora" else Electrum)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"{'http' if protocol == 'esplora' else 'tcp'}://127.0.0.1:{server.server_address[1]}", hits
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


def _rust(tmp_path, backend, targets, **options):
    binary = Path(RUST_BIN).resolve().parent / "examples/acquisition_fixture"
    assert binary.is_file(), "build kassiber-wallets --example acquisition_fixture --features test-harness --offline"
    result = subprocess.run([str(binary)], input=json.dumps({
        "backend": backend, "targets": targets, "version": __version__, "consent": True, **options,
    }).encode(), capture_output=True, timeout=15, check=True, env=_env(tmp_path))
    assert result.stderr == b""
    return json.loads(result.stdout)


def _reference(backend, targets):
    scripts, transactions, headers, block_headers = [], {}, {}, {}
    if backend["kind"] == "esplora":
        for target in targets:
            path = f"scripthash/{reference.scriptpubkey_scripthash(target['script_pubkey'])}"
            before = reference.http_get_json(reference.append_url_path(backend["url"], path))
            history = reference.fetch_esplora_scripthash_transactions(backend["url"], target["script_pubkey"])
            after = reference.http_get_json(reference.append_url_path(backend["url"], path))
            utxos = reference.fetch_esplora_scripthash_utxos(backend["url"], target["script_pubkey"])
            transactions.update((tx["txid"], tx) for tx in history)
            scripts.append(dict(target=target, before=before, after=after, history=history, utxos=utxos))
        tip = reference._esplora_tip_height(backend["url"])
        positions = [{"txid": txid, "confirmed": tx["status"]["confirmed"],
                      "block_height": reference._block_height_from_status(tx["status"]),
                      "block_time": tx["status"].get("block_time"),
                      "confirmations": reference._confirmations_from_heights(reference._block_height_from_status(tx["status"]), tip)}
                     for txid, tx in sorted(transactions.items())]
    else:
        hashes = [reference.scriptpubkey_scripthash(t["script_pubkey"]) for t in targets]
        with reference.ElectrumClient(backend) as client:
            def many(method, params):
                return reference.electrum_call_many(client, [(method, p) for p in params], backend["batch_size"])

            statuses = many("blockchain.scripthash.subscribe", [[h] for h in hashes])
            histories = many("blockchain.scripthash.get_history", [[h] for h in hashes])
            unspent = many("blockchain.scripthash.listunspent", [[h] for h in hashes])
            txids, heights = {}, set()
            for target, status, history, utxos in zip(targets, statuses, histories, unspent):
                txids.update((item["tx_hash"], item) for item in history)
                heights.update(h for item in history + utxos
                               if (h := reference._positive_electrum_height(item.get("height"))) is not None)
                scripts.append(dict(target=target, status=status, history=history, utxos=utxos))
            for txid, raw in zip(sorted(txids), many("blockchain.transaction.get", [[t] for t in sorted(txids)])):
                transactions[txid] = reference.decode_raw_transaction(raw)
            previous = sorted({item["txid"] for tx in transactions.values() for item in tx["vin"]
                               if not reference._is_bitcoin_coinbase_input(item)} - transactions.keys())
            for txid, raw in zip(previous, many("blockchain.transaction.get", [[t] for t in previous])):
                transactions[txid] = reference.decode_raw_transaction(raw)
            for txid in sorted(txids):
                reference._normalize_electrum_bitcoin_graph_for_storage(transactions[txid], transactions.__getitem__)
            block_headers = dict(zip(map(str, sorted(heights)), many("blockchain.block.header", [[h] for h in sorted(heights)])))
            headers = {h: reference.block_header_timestamp(raw) for h, raw in block_headers.items()}
            tip = reference._electrum_tip_height(client)
            positions = [{"txid": txid, "confirmed": (h := reference._positive_electrum_height(item["height"])) is not None,
                          "block_height": h, "block_time": headers.get(str(h)),
                          "confirmations": reference._confirmations_from_heights(h, tip)}
                         for txid, item in sorted(txids.items())]
    return json_ready(dict(scripts=scripts, transactions=transactions, headers=headers, block_headers=block_headers, tip_height=tip, positions=positions))


@pytest.mark.parametrize("protocol", ["esplora", "electrum"])
@pytest.mark.parametrize("fixture", ["empty", "receive-change", "checkpoint", "multi-script", "fixed", "gap-one", "reorg", "retraction", "mempool", "coinbase"])
def test_acquisition_matches_python_protocol_facts(tmp_path, protocol, fixture):
    config = _config("multi-script-p2tr+p2pkh+p2wpkh+p2sh-p2wpkh" if fixture == "multi-script"
                     else "wpkh-fixed" if fixture == "fixed" else "wpkh", 1 if fixture == "gap-one" else 3)
    plan = load_descriptor_plan(config)
    used = {reference.scriptpubkey_scripthash(t.script_pubkey)
            for t in derive_descriptor_targets(plan, start=0, end=12)
            if fixture not in {"empty", "retraction"} and t.address_index in {1, 4}}
    checkpoint = {"highest_used": {"0": "٤", "1": 4.9}} if fixture == "checkpoint" else {}
    with discovery_server(protocol, used) as (url, _):
        targets = reference.discover_compatibility_descriptor_targets(
            {"name": "fixture", "kind": protocol, "url": url, "batch_size": 2}, plan, protocol, checkpoint)["targets"]
    with _server(protocol, targets, empty=fixture in {"empty", "retraction"},
                 reorg=fixture == "reorg", mempool=fixture == "mempool", coinbase=fixture == "coinbase") as (url, _):
        backend = {"name": "fixture", "kind": protocol, "url": url, "batch_size": 2}
        expected = _reference(backend, targets)
        actual = _rust(tmp_path, backend, targets)
        assert actual == {"result": expected}
        cases = []
        records = []
        for position in expected["positions"]:
            txid = position["txid"]
            payload = {"tx": actual["result"]["transactions"][txid], "scripts": [t["script_pubkey"] for t in targets]}
            if protocol == "electrum":
                occurred = timestamp_to_iso(position["block_time"])
                payload.update(txid=txid, occurred_at=occurred)
                record = reference.record_from_electrum_tx(txid, expected["transactions"][txid], occurred,
                    payload["scripts"], "fixture", expected["transactions"].__getitem__)
            else:
                record = reference.record_from_bitcoin_esplora_tx(expected["transactions"][txid], payload["scripts"], "fixture")
            cases.append({"operation":protocol,"input":payload})
            records.append({"result":record})
        assert project(tmp_path,cases) == json.loads(json.dumps(records,default=str))
        # No acquisition result grants absence or observer persistence authority.
        assert "retracted_external_ids" not in actual["result"]
        assert "freshness_checkpoint" not in actual["result"]
        if fixture == "coinbase":
            assert len(actual["result"]["transactions"]) == 1
            graph = next(iter(actual["result"]["transactions"].values()))
            assert graph["vin"][0]["txid"] == "0" * 64
            assert "prevout" not in graph["vin"][0]


@pytest.mark.parametrize("protocol", ["esplora", "electrum"])
def test_acquisition_no_traffic_before_consent_or_while_offline(tmp_path, protocol):
    targets = [reference.sync_target_from_derived(t) for t in derive_descriptor_targets(load_descriptor_plan(_config("wpkh")), start=0, end=1)]
    with _server(protocol, targets) as (url, hits):
        backend = {"name": "fixture", "kind": protocol, "url": url, "batch_size": 2}
        for options, code in [({"consent": False}, "interaction_required"), ({"offline": True}, "network_egress_disabled")]:
            assert _rust(tmp_path, backend, targets, **options)["error"]["code"] == code
            assert hits == []


def test_acquisition_product_defaults_match_python_without_wallets(tmp_path):
    # With no wallet to fetch, the plan holds only the local journal refresh,
    # which runs natively since R4-15. Each route gets its own daemon so the
    # documented busy answer for a pipelined second refresh cannot apply.
    env = {**_env(tmp_path), "KASSIBER_NO_EGRESS": "1", "KASSIBER_TEST_NO_EGRESS": "1"}
    owners = tmp_path / "owners"
    owners.mkdir(mode=0o700)
    env["KASSIBER_RUST_TEST_OWNER_ROOT"] = str(owners)
    runtimes = {"native": [str(Path(RUST_BIN).resolve())], "python": [sys.executable, "-m", "kassiber"]}
    kinds = ["ui.wallets.sync", "ui.freshness.run", "ui.workspace.freshness.run"]
    for i, kind in enumerate(kinds):
        request = {"kind": kind, "request_id": str(i),
                   "args": {"workspace_id": "unused"} if kind == kinds[2] else {"all": True}}
        lines = {}
        for name, command in runtimes.items():
            argv = [*command, "--machine", "--data-root", str(tmp_path / f"{name}-{i}")]
            subprocess.run([*argv, "init"], env=env, capture_output=True, check=True, timeout=60)
            daemon = subprocess.Popen([*argv, "daemon"], env=env, cwd=ROOT, text=True,
                                      stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
            try:
                ready = json.loads(daemon.stdout.readline())
                if name == "native":
                    assert kind in ready["data"]["supported_kinds"]
                daemon.stdin.write(json.dumps(request) + "\n")
                daemon.stdin.flush()
                # Shutdown cancels a running native refresh, so wait for the
                # final answer; progress events share the request id.
                for line in daemon.stdout:
                    if json.loads(line).get("kind") in (kind, "error"):
                        lines[name] = line
                        break
                daemon.stdin.write(json.dumps({"kind": "daemon.shutdown"}) + "\n")
                daemon.stdin.close()
                assert daemon.wait(timeout=60) == 0
            finally:
                if daemon.poll() is None:
                    daemon.kill()
                    daemon.wait()
        assert lines["native"] == lines["python"], kind
