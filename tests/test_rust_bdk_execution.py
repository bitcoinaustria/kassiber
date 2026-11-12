"""Native BDK execution/cache oracles; no durable product-sync parity claim."""
from contextlib import contextmanager
import json
import os
from pathlib import Path
import socketserver
import subprocess
import threading
from unittest.mock import patch

import bdkpython as bdk
import pytest
from embit.script import Script
from embit.transaction import Transaction, TransactionOutput

from kassiber.core.chain_observer.bdk import BdkObserver, bdk_branches_for_identity, RECORD_FINGERPRINT_VERSION
from kassiber.core.chain_observer.bdk_persistence import serialize_changeset
from kassiber.core.chain_observer.contract import ObserverPrepareRequest
from kassiber.core.chain_observer.identity import identities_for_wallet
from kassiber.core.chain_observer.store import StoredObserverState
from kassiber.core.sync import WalletSyncState
from kassiber.core import sync_backends as reference
from kassiber.wallet_descriptors import load_descriptor_plan, derive_descriptor_targets
from tests.rust_esplora_chain import block_response
from tests.rust_sync_env import child_env as _env
from tests.test_rust_wallet_acquisition import _rust as acquisition, _server, _transactions
from tests.test_rust_wallet_discovery import _config
from tests.test_rust_bitcoin_records import project
from tests.rust_fixture_binary import fixture_binary

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires native BDK fixture build")
GENESIS = ("01000000" + "00" * 32 + "3ba3edfd7a7b12b27ac72c3e67768f617fc81bc3888a51323a9fb8aa4b1e5e4a" + "29ab5f49ffff001d1dac2b7c")


@contextmanager
def bdk_request_clock(timestamp):
    """Inject the same request clock through BDK's public *_at APIs.

    Widening can cross a wall-clock second on a loaded machine. Freezing only
    the dependency's request clock avoids comparing different first/last-seen
    inputs; no observer, protocol response or output field is replaced.
    """
    with patch.object(bdk.Wallet, "start_full_scan", lambda wallet: wallet.start_full_scan_at(timestamp)), \
         patch.object(bdk.Wallet, "start_sync_with_revealed_spks", lambda wallet: wallet.start_sync_with_revealed_spks_at(timestamp)):
        yield


@contextmanager
def bdk_server(targets, *, empty=False, mempool=False, coinbase=False, state=None):
    state = state if state is not None else {}
    state.setdefault("empty", empty)
    state.setdefault("mempool", mempool)
    state.setdefault("height", 10)
    state.setdefault("tip", 12)
    state.setdefault("epoch", 0)
    raw, tx, _, _ = _transactions(targets, coinbase=coinbase)
    old_id = tx["txid"]
    wire_tx = Transaction.parse(bytes.fromhex(raw.pop(old_id)))
    scripts = sorted({t["script_pubkey"] for t in targets})
    wire_tx.vout = [TransactionOutput(99_001 // len(scripts), Script(bytes.fromhex(s))) for s in scripts]
    txid = wire_tx.txid().hex()
    raw[txid] = wire_tx.serialize().hex()
    replacement = Transaction.parse(wire_tx.serialize())
    replacement.vout[0].value -= 1
    replacement_id = replacement.txid().hex()
    raw[replacement_id] = replacement.serialize().hex()
    hits = []
    watched = {reference.scriptpubkey_scripthash(t["script_pubkey"]) for t in targets}

    def header(height):
        if height == 0:
            return GENESIS
        return (b"\x01\x00\x00\x00" + b"\x00" * 32 + bytes.fromhex(txid)[::-1] +
                (1_700_000_000 + height + state["epoch"] * 100).to_bytes(4, "little") + b"\x00" * 8).hex()

    class Handler(socketserver.StreamRequestHandler):
        def handle(self):
            while line := self.rfile.readline():
                req = json.loads(line)
                hits.append(req)
                method, params = req["method"], req["params"]
                if method == "server.version":
                    result = ["fixture", "1.6"]
                elif method == "blockchain.headers.subscribe":
                    result = {"height": state["tip"], "hex": header(state["tip"])}
                elif method == "blockchain.block.header":
                    result = header(params[0])
                    if state.pop("race_on_header", False):
                        state["epoch"] += 1
                elif method == "blockchain.block.headers":
                    start, count = params[:2]
                    end = min(state["tip"] + 1, start + count)
                    result = {"count": end - start, "max": 2016,
                              "headers": [header(h) for h in range(start, end)],
                              "hex": "".join(header(h) for h in range(start, end))}
                elif method == "blockchain.transaction.get":
                    result = raw[params[0]]
                elif method == "blockchain.transaction.get_merkle":
                    result = {"pos": 0, "merkle": [], "block_height": state["height"]}
                elif method == "blockchain.scripthash.get_history":
                    result = ([] if state["empty"] or params[0] not in watched else
                              [{"tx_hash": replacement_id if state.get("replacement") else txid,
                                "height": 0 if state["mempool"] else state["height"]}])
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


def same_requests(python_hits, rust_hits):
    """The same backend requests, in any order: Python reads scripts on worker
    threads. Electrum request ids are per-connection counters."""
    def key(hit):
        return hit if isinstance(hit, str) else json.dumps([hit["method"], hit["params"]])
    assert sorted(map(key, rust_hits)) == sorted(map(key, python_hits))


def native(tmp_path, backend, branches, gap, *, prior=None, time=1_700_000_000, **options):
    binary = fixture_binary("bdk_fixture")
    payload = {"backend": backend, "descriptor": branches[0].descriptor,
               "change_descriptor": next((b.descriptor for b in branches if b.internal), None),
               "gap": gap, "prior": prior, "time": time, "consent": True,
               "fingerprint_version": RECORD_FINGERPRINT_VERSION, **options}
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
            facts = observer._facts(wallet, None)
            assert actual["result"]["records"] == list(facts.transaction_records)
            assert actual["result"]["record_fingerprints"] == observer._record_fingerprints
            state = StoredObserverState(identity=identity, payload={"schema_version": 1, "bdk_changeset": expected}, coverage=())
            wallet, persister = observer._wallet_from_state(state)
            observer._sync_revealed_horizon(observer._client(), wallet)
            wallet.persist(persister)
            restarted = serialize_changeset(observer._persistence.aggregate)
            clock = next(iter(restarted["tx_graph"]["last_seen"]), {}).get("timestamp", clock)
            actual = native(tmp_path, backend, branches, plan.gap_limit, prior=expected, time=clock)["result"]
            assert actual["bdk_changeset"] == restarted
            facts = observer._facts(wallet, state)
            assert actual["records"] == list(facts.transaction_records)
            assert actual["record_fingerprints"] == observer._record_fingerprints


@pytest.mark.parametrize("protocol", ["esplora", "electrum"])
@pytest.mark.parametrize("mempool", [False, True])
def test_compatibility_checkpoint_and_cache_reuse_match_python(tmp_path, protocol, mempool):
    plan = load_descriptor_plan(_config("wpkh", 1))
    targets = [reference.sync_target_from_derived(t) for t in derive_descriptor_targets(plan, start=0, end=1)][:1]
    with _server(protocol, targets, mempool=mempool) as (url, hits):
        backend = {"name": "fixture", "kind": protocol, "url": url, "batch_size": 2}
        checkpoint, history_cache = {}, {}
        for round_index in range(3):
            state = WalletSyncState(chain="bitcoin", network="main", descriptor_plan=plan, policy_asset_id="",
                                    targets=targets, tracked_scripts={t["script_pubkey"]: t for t in targets},
                                    checkpoint=checkpoint, history_cache=history_cache)
            start = len(hits)
            records, meta = getattr(reference, f"compatibility_{protocol}_sync_adapter")(backend, {}, state)
            python_hits, start = hits[start:], len(hits)
            actual = acquisition(tmp_path, backend, targets, cached=True, checkpoint=checkpoint, history_cache=history_cache)["result"]
            same_requests(python_hits, hits[start:])
            cases = [{"operation":protocol,"input":{"tx":actual["transactions"][r["txid"]],
                "scripts":[t["script_pubkey"] for t in targets],"txid":r["txid"],"occurred_at":r["occurred_at"]}}
                for r in records]
            assert project(tmp_path,cases) == json.loads(json.dumps([{"result":r} for r in records],default=str))
            assert actual["freshness_checkpoint"] == meta["freshness_checkpoint"]
            for key in ("scripts_changed", "scripts_unchanged", "transactions_fetched", "header_cache_hits"):
                if key in meta:
                    assert actual[key] == meta[key]
            if protocol == "esplora":
                assert actual["esplora_history_complete"] == meta["esplora_history_complete"]
                assert actual["observer_retracted_external_ids"] == meta["observer_retracted_external_ids"]
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


@pytest.mark.parametrize("sequence", ["reanchor", "reorg-drop", "preflight-race", "mempool-drop-return", "replacement", "height-change", "force-full", "force-without-state"])
def test_native_canonical_and_retraction_sequences_match_python_prepare(tmp_path, sequence):
    plan = load_descriptor_plan(_config("wpkh", 3))
    row = {"id": "wallet", "workspace_id": "ws", "profile_id": "p", "config_json": json.dumps(_config("wpkh", 3))}
    identity = identities_for_wallet(row, observer_kind="bdk")[0]
    branches = bdk_branches_for_identity(plan, identity)
    targets = [reference.sync_target_from_derived(t) for t in derive_descriptor_targets(plan, start=0, end=1)]
    remote = {"mempool": sequence in {"mempool-drop-return", "replacement", "force-full", "force-without-state"}}
    with bdk_server(targets, state=remote) as (url, _):
        backend = {"name": "fixture", "kind": "electrum", "url": url, "batch_size": 2}
        prior, checkpoint = None, {}
        for round_index in range(3):
            force_full = round_index == 1 and sequence in {"force-full", "force-without-state"}
            if round_index == 1:
                if sequence in {"reanchor", "reorg-drop"}:
                    remote["epoch"] += 1
                if sequence == "reanchor":
                    remote["height"] = 9
                if sequence in {"reorg-drop", "mempool-drop-return", "force-full", "force-without-state"}:
                    remote["empty"] = True
                if sequence == "height-change":
                    remote["height"] = 9
                if sequence == "replacement":
                    remote["replacement"] = True
                if sequence == "preflight-race":
                    remote["race_on_header"] = True
            elif round_index == 2:
                remote["empty"] = False
            # Force-full can recover an unreadable dependency aggregate using
            # this identity's checkpoint. Other identities cannot supply it.
            supplied_prior = None if force_full and sequence == "force-without-state" else prior
            observer = BdkObserver(identity=identity, backend=backend, branches=branches, gap_limit=plan.gap_limit)
            request = ObserverPrepareRequest(backend_name="fixture", backend_kind="electrum", force_full=force_full,
                                             checkpoint={"observer_instances": {identity.id: checkpoint, "other": {"canonical_txids": ["f" * 64], "tip_height": 1000}}})
            clock = 1_700_000_000 + round_index
            with bdk_request_clock(clock):
                expected = observer.prepare(request, supplied_prior)
            baseline = (supplied_prior.payload if supplied_prior else checkpoint if force_full else {}).get("canonical_txids", [])
            if sequence == "preflight-race" and round_index == 1:
                # Replay the same chain change for the independent native
                # request, after its old-tip check and before its suffix scan.
                remote["epoch"] -= 1
                remote["race_on_header"] = True
            actual = native(tmp_path, backend, branches, plan.gap_limit,
                            prior=supplied_prior.payload["bdk_changeset"] if supplied_prior else None,
                            time=clock, force_full=force_full, canonical_txids=baseline,
                            checkpoint_tip=checkpoint.get("tip_height"))
            result = actual["result"]
            assert result["bdk_changeset"] == expected["state"]["bdk_changeset"]
            assert result["canonical_txids"] == expected["state"]["canonical_txids"]
            assert result["retracted_external_ids"] == expected["facts"]["retracted_external_ids"]
            assert result["records"] == expected["facts"]["transaction_records"]
            assert result["record_fingerprints"] == expected["state"]["record_fingerprints"]
            assert result["rebuilt"] == (round_index == 1 and sequence in {"reanchor", "reorg-drop"})
            if round_index == 1 and sequence in {"reorg-drop", "mempool-drop-return", "replacement", "force-full", "force-without-state"}:
                assert result["retracted_external_ids"]
            prior = StoredObserverState(identity=identity, payload=expected["state"], coverage=())
            checkpoint = expected["facts"]["freshness_checkpoint"]


@pytest.mark.parametrize("force_full", [False, True])
def test_native_rebuild_preflight_and_consent_fences_match_python(tmp_path, force_full):
    plan = load_descriptor_plan(_config("wpkh", 3))
    row = {"id": "wallet", "workspace_id": "ws", "profile_id": "p", "config_json": json.dumps(_config("wpkh", 3))}
    identity = identities_for_wallet(row, observer_kind="bdk")[0]
    branches = bdk_branches_for_identity(plan, identity)
    targets = [reference.sync_target_from_derived(t) for t in derive_descriptor_targets(plan, start=0, end=1)]
    remote = {}
    with bdk_server(targets, state=remote) as (url, hits):
        backend = {"name": "fixture", "kind": "electrum", "url": url, "batch_size": 2}
        observer = BdkObserver(identity=identity, backend=backend, branches=branches, gap_limit=plan.gap_limit)
        expected = observer.prepare(ObserverPrepareRequest("fixture", "electrum"), None)
        prior = expected["state"]["bdk_changeset"]
        options = {"prior": prior, "force_full": force_full, "canonical_txids": expected["state"]["canonical_txids"], "checkpoint_tip": 12}
        hits.clear()
        for flags, code in [({"consent": False}, "interaction_required"), ({"offline": True}, "network_egress_disabled")]:
            assert native(tmp_path, backend, branches, 3, **options, **flags)["error"]["code"] == code
            assert hits == []
        remote["tip"] = 11
        error = native(tmp_path, backend, branches, 3, **options)["error"]
        assert error["code"] == "backend_tip_behind"
        assert not any(hit["method"] == "blockchain.scripthash.get_history" for hit in hits)


@contextmanager
def esplora_round_server(targets, remote):
    from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

    _, tx, _, _ = _transactions(targets)
    tx["vout"] = [{"scriptpubkey": target["script_pubkey"], "value": 99_001} for target in targets]
    remote["original"] = tx["txid"]
    hits = []

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            hits.append(self.path)
            parts = self.path.strip("/").split("/")
            status = 200
            # One block is added each round: a proven extension, not a reorg.
            block = block_response(parts, 102 + remote.get("round", 0))
            if block is not None:
                status, body = block
            else:
                assert parts[0] == "scripthash"
                rows = [] if remote.get("empty") else [{**tx, "txid": remote.get("txid", tx["txid"])}]
                if len(parts) == 2:
                    result = {"chain_stats": {"tx_count": len(rows) + int(remote.get("partial", False)),
                                               "funded_txo_sum": remote.get("round", 0)},
                              "mempool_stats": {"tx_count": 0}}
                elif parts[2:] == ["txs", "chain"]:
                    result = rows
                elif parts[2:] in (["txs", "mempool"], ["utxo"]):
                    result = []
                else:
                    raise AssertionError(self.path)
                body = json.dumps(result).encode()
            self.send_response(status)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_):
            pass

    with ThreadingHTTPServer(("127.0.0.1", 0), Handler) as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            yield f"http://127.0.0.1:{server.server_address[1]}", hits
        finally:
            server.shutdown()
            thread.join()


@pytest.mark.parametrize("sequence", ["empty", "partial", "shrink", "force-full", "source-change", "page-limit"])
def test_esplora_absence_sequences_match_python_source_bound_completeness(tmp_path, sequence):
    from kassiber.core.esplora_history import full_scan_checkpoint

    plan = load_descriptor_plan(_config("wpkh", 1))
    targets = [reference.sync_target_from_derived(t) for t in derive_descriptor_targets(plan, start=0, end=1)]
    remote = {}
    with esplora_round_server(targets, remote) as (url, hits):
        backend = {"name": "fixture", "kind": "esplora", "url": url, "batch_size": 2}
        checkpoint = {}
        for round_index in range(3):
            remote["round"] = round_index
            remote["empty"] = round_index > 0
            selected = targets[:1] if sequence == "shrink" and round_index == 1 else targets
            if sequence == "partial" and round_index == 1:
                remote.update(empty=False, partial=True, txid="b" * 64)
            else:
                remote["partial"] = False
            if sequence == "page-limit" and round_index == 0:
                backend["maxpages"] = 1
            else:
                backend.pop("maxpages", None)
            if sequence == "source-change" and round_index == 1:
                backend["token"] = "synthetic-fixture"
            if sequence == "force-full" and round_index == 1:
                checkpoint = full_scan_checkpoint(checkpoint)
            # A durable absence baseline is still inert without exact consent.
            if round_index == 1:
                hits.clear()
                for flags, code in [({"consent": False}, "interaction_required"), ({"offline": True}, "network_egress_disabled")]:
                    assert acquisition(tmp_path, backend, selected, cached=True, checkpoint=checkpoint, **flags)["error"]["code"] == code
                    assert hits == []
            state = WalletSyncState(chain="bitcoin", network="main", descriptor_plan=plan, policy_asset_id="",
                                    targets=selected, tracked_scripts={t["script_pubkey"]: t for t in selected}, checkpoint=checkpoint, history_cache={})
            start = len(hits)
            _, expected = reference.compatibility_esplora_sync_adapter(backend, {}, state)
            python_hits, start = hits[start:], len(hits)
            actual = acquisition(tmp_path, backend, selected, cached=True, checkpoint=checkpoint)["result"]
            same_requests(python_hits, hits[start:])
            for key in ("freshness_checkpoint", "esplora_history_complete", "observer_retracted_external_ids"):
                assert actual[key] == expected[key]
            if round_index == 1 and sequence in {"partial", "shrink", "source-change"}:
                assert actual["observer_retracted_external_ids"] == []
            elif round_index == 1:
                assert actual["observer_retracted_external_ids"] == [remote["original"]]
            if round_index == 2 and sequence in {"partial", "shrink"}:
                expected_ids = [remote["original"], "b" * 64] if sequence == "partial" else [remote["original"]]
                assert actual["observer_retracted_external_ids"] == sorted(expected_ids)
            checkpoint = expected["freshness_checkpoint"]


@pytest.mark.parametrize("protocol", ["esplora", "electrum"])
def test_compatibility_tip_changes_refresh_evidence_without_refetching_valid_graphs(tmp_path, protocol):
    from tests.rust_sync_chain import Chain, serve

    plan = load_descriptor_plan(_config("wpkh", 1))
    targets = [reference.sync_target_from_derived(t)
               for t in derive_descriptor_targets(plan, start=0, end=1)][:1]
    chain = Chain()
    txid, _ = chain.fund(targets[0]["script_pubkey"], 20_000, height=14)
    checkpoint, history_cache = {}, {}
    observed_times = []
    with serve(chain, protocol) as url:
        backend = {"name": "fixture", "kind": protocol, "url": url, "batch_size": 2}
        for index in range(4):
            if index == 2:
                # The subscription status, history heights and Esplora statistics
                # stay identical while the confirming block identity/time changes.
                chain.reorg(14)
            if index == 3:
                # Pre-upgrade/unknown anchors cannot authorize cache reuse.
                checkpoint.pop("esplora_tip_hash" if protocol == "esplora" else "electrum_tip_fingerprint", None)
            state = WalletSyncState(chain="bitcoin", network="main", descriptor_plan=plan, policy_asset_id="",
                                    targets=targets, tracked_scripts={t["script_pubkey"]: t for t in targets},
                                    checkpoint=checkpoint, history_cache=history_cache)
            records, meta = getattr(reference, f"compatibility_{protocol}_sync_adapter")(backend, {}, state)
            native_start = len(chain.hits)
            actual = acquisition(tmp_path, backend, targets, cached=True, checkpoint=checkpoint,
                                 history_cache=history_cache)["result"]
            assert actual["freshness_checkpoint"] == meta["freshness_checkpoint"]
            for key in ("scripts_changed", "scripts_unchanged", "transactions_fetched", "header_cache_hits"):
                if key in meta:
                    assert actual[key] == meta[key]
            assert actual["utxos_skipped_unchanged"] == meta.get("utxos_skipped_unchanged", False)
            assert actual["utxos_skipped_unchanged"] == (index == 1)
            if index in {2, 3} and protocol == "electrum":
                fetched = [hit["params"][0] for hit in chain.hits[native_start:]
                           if hit["method"] == "blockchain.transaction.get"]
                assert txid not in fetched  # The foreign parent is not in the persisted history cache.
                assert actual["transactions_fetched"] == len(fetched)
                assert actual["header_cache_hits"] == 0
            for record in records:
                payload = {"tx": actual["transactions"][record["txid"]],
                           "scripts": [t["script_pubkey"] for t in targets],
                           "txid": record["txid"], "occurred_at": record["occurred_at"]}
                assert project(tmp_path, [{"operation": protocol, "input": payload}]) == json.loads(
                    json.dumps([{"result": record}], default=str))
                observed_times.append(record["occurred_at"])
                decoded = json.loads(record["raw_json"]) if isinstance(record["raw_json"], str) else record["raw_json"]
                decoded["txid"] = record["txid"]
                history_cache[record["txid"]] = json.dumps(decoded)
            checkpoint = meta["freshness_checkpoint"]
    assert txid in history_cache
    assert observed_times[0] != observed_times[-1]
