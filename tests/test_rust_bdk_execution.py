"""Native BDK execution/cache oracles; no durable product-sync parity claim."""
from contextlib import contextmanager
import json
import os
from pathlib import Path
import socketserver
import subprocess
import threading

import bdkpython as bdk
import pytest
from embit.script import Script
from embit.transaction import Transaction, TransactionOutput

from kassiber.core.chain_observer.bdk import BdkObserver, bdk_branches_for_identity
from kassiber.core.chain_observer.bdk_persistence import serialize_changeset
from kassiber.core.chain_observer.identity import identities_for_wallet
from kassiber.core.chain_observer.store import StoredObserverState
from kassiber.core.sync import WalletSyncState
from kassiber.core import sync_backends as reference
from kassiber.wallet_descriptors import load_descriptor_plan, derive_descriptor_targets
from tests.test_rust_chain_candidate import _env
from tests.test_rust_wallet_acquisition import _rust as acquisition, _server, _transactions
from tests.test_rust_wallet_discovery import _config

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires native BDK fixture build")
GENESIS = ("01000000" + "00" * 32 + "3ba3edfd7a7b12b27ac72c3e67768f617fc81bc3888a51323a9fb8aa4b1e5e4a" + "29ab5f49ffff001d1dac2b7c")


@contextmanager
def bdk_server(targets, *, empty=False, mempool=False, coinbase=False):
    raw, tx, _, _ = _transactions(targets, coinbase=coinbase)
    old_id = tx["txid"]
    wire_tx = Transaction.parse(bytes.fromhex(raw.pop(old_id)))
    scripts = sorted({t["script_pubkey"] for t in targets})
    wire_tx.vout = [TransactionOutput(99_001 // len(scripts), Script(bytes.fromhex(s))) for s in scripts]
    txid = wire_tx.txid().hex()
    raw[txid] = wire_tx.serialize().hex()
    hits = []
    watched = {reference.scriptpubkey_scripthash(t["script_pubkey"]) for t in targets}

    def header(height):
        if height == 0:
            return GENESIS
        return (b"\x01\x00\x00\x00" + b"\x00" * 32 + bytes.fromhex(txid)[::-1] +
                (1_700_000_000 + height).to_bytes(4, "little") + b"\x00" * 8).hex()

    class Handler(socketserver.StreamRequestHandler):
        def handle(self):
            while line := self.rfile.readline():
                req = json.loads(line)
                hits.append(req)
                method, params = req["method"], req["params"]
                if method == "server.version":
                    result = ["fixture", "1.6"]
                elif method == "blockchain.headers.subscribe":
                    result = {"height": 12, "hex": header(12)}
                elif method == "blockchain.block.header":
                    result = header(params[0])
                elif method == "blockchain.block.headers":
                    start, count = params[:2]
                    result = {"count": min(count, 13-start), "max": 2016,
                              "headers": [header(h) for h in range(start, min(13, start+count))],
                              "hex": "".join(header(h) for h in range(start, min(13, start+count)))}
                elif method == "blockchain.transaction.get":
                    result = raw[params[0]]
                elif method == "blockchain.transaction.get_merkle":
                    result = {"pos": 0, "merkle": [], "block_height": 10}
                elif method == "blockchain.scripthash.get_history":
                    result = ([] if empty or params[0] not in watched else
                              [{"tx_hash": txid, "height": 0 if mempool else 10}])
                else:
                    raise AssertionError(method)
                self.wfile.write(json.dumps({"id": req["id"], "result": result}).encode()+b"\n")

    class Server(socketserver.ThreadingTCPServer):
        daemon_threads = True

    with Server(("127.0.0.1", 0), Handler) as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            yield f"tcp://127.0.0.1:{server.server_address[1]}", hits
        finally:
            server.shutdown()
            thread.join()


def native(tmp_path, backend, branches, gap, *, prior=None, time=1_700_000_000, **options):
    binary = Path(RUST_BIN).resolve().parent / "examples/bdk_fixture"
    assert binary.is_file(), "build kassiber-wallets --examples --features test-harness --offline --locked"
    payload = {"backend": backend, "descriptor": branches[0].descriptor,
               "change_descriptor": next((b.descriptor for b in branches if b.internal), None),
               "gap": gap, "prior": prior, "time": time, "consent": True, **options}
    result = subprocess.run([str(binary)], input=json.dumps(payload).encode(), capture_output=True,
                            check=True, timeout=20, env=_env(tmp_path))
    assert result.stderr == b""
    return json.loads(result.stdout)


@pytest.mark.parametrize("fixture", ["empty", "receive-change", "checkpoint", "multi-script", "fixed", "gap-one", "mempool", "coinbase"])
def test_native_bdk_aggregate_and_restart_match_python_wallet(tmp_path, fixture):
    config = _config("multi-script-p2tr+p2pkh+p2wpkh+p2sh-p2wpkh" if fixture == "multi-script"
                     else "wpkh-fixed" if fixture == "fixed" else "wpkh", 1 if fixture == "gap-one" else 3)
    plan = load_descriptor_plan(config)
    all_targets = [reference.sync_target_from_derived(t) for t in derive_descriptor_targets(plan, start=0, end=7)]
    wallet_row = {"id": "wallet", "workspace_id": "ws", "profile_id": "p", "config_json": json.dumps(config)}
    for identity in identities_for_wallet(wallet_row, observer_kind="bdk"):
        branches = bdk_branches_for_identity(plan, identity)
        indices = {b.branch_index for b in branches}
        targets = [t for t in all_targets if t["branch_index"] in indices
                   and t["address_index"] in ({1, 4} if fixture in {"receive-change", "checkpoint", "multi-script"} else {0})]
        with bdk_server(targets, empty=fixture == "empty", mempool=fixture == "mempool", coinbase=fixture == "coinbase") as (url, _):
            backend = {"name": "fixture", "kind": "electrum", "url": url, "batch_size": 2}
            observer = BdkObserver(identity=identity, backend=backend, branches=branches, gap_limit=plan.gap_limit)
            wallet, persister = observer._wallet_from_state(None)
            client = observer._client()
            wallet.apply_update(observer._full_scan(client, wallet))
            observer._reveal_scan_horizon(wallet)
            wallet.persist(persister)
            expected = serialize_changeset(observer._persistence.aggregate)
            clock = next(iter(expected["tx_graph"]["last_seen"]), {}).get("timestamp", 1_700_000_000)
            actual = native(tmp_path, backend, branches, plan.gap_limit, time=clock)
            assert actual["result"]["bdk_changeset"] == expected
            assert sorted(actual["result"]["canonical_txids"]) == sorted(str(t.transaction.compute_txid()) for t in wallet.transactions())
            state = StoredObserverState(identity=identity, payload={"schema_version": 1, "bdk_changeset": expected}, coverage=())
            wallet, persister = observer._wallet_from_state(state)
            observer._sync_revealed_horizon(observer._client(), wallet)
            wallet.persist(persister)
            restarted = serialize_changeset(observer._persistence.aggregate)
            clock = next(iter(restarted["tx_graph"]["last_seen"]), {}).get("timestamp", clock)
            assert native(tmp_path, backend, branches, plan.gap_limit, prior=expected, time=clock)["result"]["bdk_changeset"] == restarted


@pytest.mark.parametrize("protocol", ["esplora", "electrum"])
@pytest.mark.parametrize("mempool", [False, True])
def test_compatibility_checkpoint_and_cache_reuse_match_python(tmp_path, protocol, mempool):
    plan = load_descriptor_plan(_config("wpkh", 1))
    targets = [reference.sync_target_from_derived(t) for t in derive_descriptor_targets(plan, start=0, end=1)][:1]
    with _server(protocol, targets, mempool=mempool) as (url, _):
        backend = {"name": "fixture", "kind": protocol, "url": url, "batch_size": 2}
        checkpoint, history_cache = {}, {}
        for round_index in range(3):
            state = WalletSyncState(chain="bitcoin", network="main", descriptor_plan=plan, policy_asset_id="",
                                    targets=targets, tracked_scripts={t["script_pubkey"]: t for t in targets},
                                    checkpoint=checkpoint, history_cache=history_cache)
            records, meta = getattr(reference, f"compatibility_{protocol}_sync_adapter")(backend, {}, state)
            actual = acquisition(tmp_path, backend, targets, cached=True, checkpoint=checkpoint, history_cache=history_cache)["result"]
            assert actual["freshness_checkpoint"] == meta["freshness_checkpoint"]
            for key in ("scripts_changed", "scripts_unchanged", "transactions_fetched", "header_cache_hits"):
                if key in meta:
                    assert actual[key] == meta[key]
            assert actual["utxos_skipped_unchanged"] == meta.get("utxos_skipped_unchanged", False)
            if round_index and not mempool:
                assert actual["utxos_skipped_unchanged"] is True
            checkpoint = meta["freshness_checkpoint"]
            for record in records:
                raw = record["raw_json"]
                decoded = json.loads(raw) if isinstance(raw, str) else raw
                # The real history-cache loader supplies the transaction ID.
                decoded["txid"] = record["txid"]
                history_cache[record["txid"]] = json.dumps(decoded)


@pytest.mark.parametrize("protocol", ["esplora", "electrum"])
def test_cached_acquisition_and_native_bdk_require_consent_and_online(tmp_path, protocol):
    config = _config("wpkh", 1)
    plan = load_descriptor_plan(config)
    targets = [reference.sync_target_from_derived(t) for t in derive_descriptor_targets(plan, start=0, end=1)]
    with _server(protocol, targets) as (url, hits):
        backend = {"name": "fixture", "kind": protocol, "url": url, "batch_size": 2}
        for options, code in [({"consent": False}, "interaction_required"), ({"offline": True}, "network_egress_disabled")]:
            assert acquisition(tmp_path, backend, targets, cached=True, checkpoint={}, **options)["error"]["code"] == code
            assert hits == []
            if protocol == "electrum":
                row = {"id": "w", "workspace_id": "ws", "profile_id": "p", "config_json": json.dumps(config)}
                identity = identities_for_wallet(row, observer_kind="bdk")[0]
                branches = bdk_branches_for_identity(plan, identity)
                assert native(tmp_path, backend, branches, plan.gap_limit, **options)["error"]["code"] == code
                assert hits == []
