"""Native LWK execution oracles against Python's pinned lwk 0.18.0 binding.

Both observers scan the same deterministic Elements regtest chain states
(`crates/wallets/tests/vectors/lwk_chain.json`, from the `lwk_vectors`
example) through one loopback Electrum fake. The fake answers JSON-array
batches (Python's rust-electrum-client) and newline batches (Kassiber's own
client) alike. Prepared updates must match exactly; LWK's opaque update bytes
serialize hash-map order, so they are compared through a canonical decode and
by restarting each observer from the other's stored values.
"""
from contextlib import contextmanager
import json
import os
from pathlib import Path
import socketserver
import subprocess
import threading
from unittest.mock import patch

import pytest

from kassiber.core.chain_observer.contract import ObserverPrepareRequest
from kassiber.core.chain_observer.identity import identities_for_wallet
from kassiber.core.chain_observer.lwk import (
    LwkObserver,
    lwk_compatibility_reason,
    lwk_descriptor_for_plan,
)
from kassiber.core.chain_observer.store import CoveragePoint, StoredObserverState
from kassiber.errors import AppError
from kassiber.wallet_descriptors import load_descriptor_plan
from tests.rust_sync_env import child_env
from tests.test_lwk_observer import descriptor, wallet_row
from tests.rust_fixture_binary import fixture_binary

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires native LWK fixture build")
ROOT = Path(__file__).resolve().parents[1]
VECTORS = json.loads((ROOT / "crates/wallets/tests/vectors/lwk_chain.json").read_text())
POLICY = VECTORS["policy_asset"]


@contextmanager
def electrum(state, hits=None):
    """Serve `state["phase"]`; switching it models a later chain view."""

    class Handler(socketserver.StreamRequestHandler):
        def handle(self):
            while line := self.rfile.readline():
                request = json.loads(line)
                batch = isinstance(request, list)
                replies = []
                for item in request if batch else [request]:
                    phase = state["phase"]
                    method, params = item["method"], item.get("params", [])
                    if hits is not None:
                        hits.append(method)
                    if method == "server.version":
                        result = ["fixture", "1.4"]
                    elif method == "blockchain.headers.subscribe":
                        result = {"height": phase["tip"], "hex": phase["headers"][str(phase["tip"])]}
                    elif method == "blockchain.block.header":
                        result = phase["headers"][str(params[0])]
                    elif method == "blockchain.scripthash.get_history":
                        result = phase["history"].get(params[0], [])
                    elif method == "blockchain.transaction.get":
                        result = phase["transactions"][params[0]]
                    else:
                        raise AssertionError(method)
                    replies.append({"jsonrpc": "2.0", "id": item["id"], "result": result})
                self.wfile.write(json.dumps(replies if batch else replies[0]).encode() + b"\n")

    class Server(socketserver.ThreadingTCPServer):
        daemon_threads = True

    with Server(("127.0.0.1", 0), Handler) as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            yield f"tcp://127.0.0.1:{server.server_address[1]}"
        finally:
            server.shutdown()
            thread.join()


def config(text=None, **extra):
    return {"chain": "liquid", "network": "elementsregtest", "descriptor": text or VECTORS["descriptor"],
            "gap_limit": 3, **extra}


def identity(wallet_config):
    return identities_for_wallet(wallet_row(wallet_config), observer_kind="lwk")[0]


def python(backend, wallet_config, *, values=None, prior=None, force_full=False, checkpoint=None):
    plan = load_descriptor_plan(wallet_config)
    observer = LwkObserver(identity=identity(wallet_config), backend=backend, descriptor_plan=plan,
                           policy_asset_id=POLICY, stored_values=values or {})
    with patch("kassiber.egress_policy.require_egress_enabled", lambda *a, **k: None):
        update = observer.prepare(
            ObserverPrepareRequest(backend_name=backend["name"], backend_kind="electrum",
                                   force_full=force_full, checkpoint=checkpoint or {}),
            prior,
        )
    return json.loads(json.dumps(update)), observer.store.snapshot()


def native(tmp_path, payload):
    binary = fixture_binary("lwk_fixture")
    result = subprocess.run([str(binary)], input=json.dumps(payload).encode(), capture_output=True,
                            check=True, timeout=60, env=child_env(tmp_path))
    assert result.stderr == b""
    return json.loads(result.stdout)


def run_native(tmp_path, backend, wallet_config, *, values=None, prior=None, force_full=False,
               checkpoint=None, **options):
    payload = {"op": "run", "config": wallet_config, "backend": backend, "network": "elementsregtest",
               "policy_asset": POLICY, "branch_keys": list(identity(wallet_config).branch_keys),
               "values": {k: bytes(v).hex() for k, v in (values or {}).items()},
               "prior": prior.payload if prior else None,
               "prior_coverage": [
                   {"branch_key": p.branch_key, "highest_used": p.highest_used, "scanned_to": p.scanned_to}
                   for p in (prior.coverage if prior else ())
               ],
               "checkpoint": checkpoint or {}, "force_full": force_full, "consent": True, **options}
    return native(tmp_path, payload)


def canonical(tmp_path, values):
    out = native(tmp_path, {"op": "canonical", "config": config(),
                            "values": {k: bytes(v).hex() for k, v in values.items()}})
    return out["result"]


def stored(update, wallet_config):
    """The `StoredObserverState` a successful apply would persist."""
    coverage = tuple(CoveragePoint(**{**point, "details": dict(point["details"])})
                     for point in update["facts"]["coverage"])
    return StoredObserverState(identity=identity(wallet_config), payload=dict(update["state"]), coverage=coverage)


@pytest.mark.parametrize("phase", ["empty", "initial", "reorg", "lagging"])
def test_fresh_scan_matches_python_update_and_store(tmp_path, phase):
    wallet_config = config()
    state = {"phase": VECTORS["phases"][phase]}
    with electrum(state) as url:
        backend = {"name": "fixture", "kind": "electrum", "url": url}
        expected, values = python(backend, wallet_config)
        actual = run_native(tmp_path, backend, wallet_config)["result"]
    assert actual["update"] == expected
    assert sorted(actual["values"]) == sorted(values) == ["000000000000"]
    assert actual["canonical_values"] == canonical(tmp_path, values)


def test_vectors_exercise_every_accounting_shape(tmp_path):
    # Guard the oracle itself: each scenario must still reach its own branch.
    with electrum({"phase": VECTORS["phases"]["initial"]}) as url:
        update, _ = python({"name": "fixture", "kind": "electrum", "url": url}, config())
    names = {txid: name for name, txid in VECTORS["transactions"].items()}
    by_tx = {}
    for record in update["facts"]["transaction_records"]:
        raw = json.loads(record["raw_json"])
        by_tx.setdefault(names[record["txid"]], []).append((record, raw))
    assert set(by_tx) == {"fund", "spend", "mixed", "claim", "refund", "asset_send", "mempool"}
    # A wrongly blinded output to our script is never unblinded or booked.
    assert "wrong_key" not in by_tx
    assert {r["asset"] for r, _ in by_tx["fund"]} == {"LBTC", VECTORS["issued_asset"]}
    assert [r["kind"] for r, _ in by_tx["asset_send"]] == ["fee", "withdrawal"]
    mixed_record, mixed_raw = by_tx["mixed"][0]
    assert mixed_raw["vin"][0]["prevout"]["role"] == "owned"
    assert "prevout" not in mixed_raw["vin"][1]
    assert by_tx["claim"][0][0]["payment_hash_source"] == "chain_script_unique_outpoint"
    assert by_tx["claim"][0][1]["htlc_spend"]["role"] == "claim"
    assert by_tx["refund"][0][1]["htlc_spend"]["role"] == "refund"
    assert by_tx["refund"][0][0]["swap_refund_funding_vout"] == 1
    assert by_tx["mempool"][0][0]["confirmed_at"] is None
    spend_record, spend_raw = by_tx["spend"][0]
    assert spend_record["fee"] == "0.000005"
    assert spend_raw["component"]["fee_attribution"] == "exact"
    statuses = {o["confirmation_status"] for o in update["facts"]["outputs"]}
    assert statuses == {"confirmed", "mempool"}


def test_explicit_wallet_outputs_are_booked_and_held_in_both_observers(tmp_path):
    # LWK 0.18.0 leaves explicit (unblinded) secrets out of its balance and
    # UTXO set; both observers add the owned explicit amounts back (Python
    # fix 56f928ace), so the receipt is booked and its spend is a debit.
    fund, mixed = VECTORS["transactions"]["fund"], VECTORS["transactions"]["mixed"]
    for phase in ("initial", "reorg"):
        with electrum({"phase": VECTORS["phases"][phase]}) as url:
            backend = {"name": "fixture", "kind": "electrum", "url": url}
            update, _ = python(backend, config())
            actual = run_native(tmp_path, backend, config())["result"]["update"]
        assert actual == update
        records = update["facts"]["transaction_records"]
        lbtc = next(r for r in records if r["txid"] == fund and r["asset"] == "LBTC")
        assert lbtc["amount"] == "0.0015"
        assert json.loads(lbtc["raw_json"])["component"]["net_sats"] == 150_000
        explicit = [o for o in update["facts"]["outputs"] if o["txid"] == fund and o["vout"] == 1]
        if phase == "initial":
            spend = next(r for r in records if r["txid"] == mixed)
            assert (spend["direction"], spend["amount"], spend["amount_includes_fee"]) == ("outbound", "0.00021", True)
            assert explicit == []
        else:
            # The reorg drops the mixed spend: the explicit output is held again.
            assert len(explicit) == 1
            assert explicit[0]["amount_sats"] == 50_000
            assert explicit[0]["raw"] == {"source": "lwk_wollet", "confidential": False}


def test_outputs_are_in_outpoint_order_in_both_observers(tmp_path):
    with electrum({"phase": VECTORS["phases"]["initial"]}) as url:
        update, _ = python({"name": "fixture", "kind": "electrum", "url": url}, config())
    points = [(o["txid"], o["vout"]) for o in update["facts"]["outputs"]]
    assert points == sorted(points) and len(points) > 1


@pytest.mark.parametrize("later", ["initial", "reorg"])
@pytest.mark.parametrize("store_from", ["python", "native"])
def test_restart_from_either_store_matches_python(tmp_path, later, store_from):
    wallet_config = config()
    state = {"phase": VECTORS["phases"]["initial"]}
    with electrum(state) as url:
        backend = {"name": "fixture", "kind": "electrum", "url": url}
        first, python_values = python(backend, wallet_config)
        native_first = run_native(tmp_path, backend, wallet_config)["result"]
        values = python_values if store_from == "python" else {
            k: bytes.fromhex(v) for k, v in native_first["values"].items()}
        prior = stored(first, wallet_config)
        state["phase"] = VECTORS["phases"][later]
        expected, expected_values = python(backend, wallet_config, values=values, prior=prior,
                                           checkpoint=first["facts"]["freshness_checkpoint"])
        actual = run_native(tmp_path, backend, wallet_config, values=values, prior=prior,
                            checkpoint=first["facts"]["freshness_checkpoint"])["result"]
    assert actual["update"] == expected
    assert actual["canonical_values"] == canonical(tmp_path, expected_values)
    if later == "reorg":
        assert expected["facts"]["retracted_external_ids"] == [VECTORS["transactions"]["mixed"]]
        assert sorted(expected_values) == ["000000000000", "000000000001"]
    else:
        assert expected["facts"]["retracted_external_ids"] == []


def test_forced_rebuild_uses_checkpoint_baseline_and_fresh_store(tmp_path):
    wallet_config = config()
    state = {"phase": VECTORS["phases"]["initial"]}
    with electrum(state) as url:
        backend = {"name": "fixture", "kind": "electrum", "url": url}
        first, _ = python(backend, wallet_config)
        checkpoint = first["facts"]["freshness_checkpoint"]
        state["phase"] = VECTORS["phases"]["reorg"]
        # A rebuild ignores stored opaque values and an incompatible payload.
        incompatible = StoredObserverState(identity=identity(wallet_config),
                                           payload={"schema_version": 9}, coverage=())
        expected, values = python(backend, wallet_config, force_full=True, checkpoint=checkpoint)
        actual = run_native(tmp_path, backend, wallet_config, force_full=True, checkpoint=checkpoint,
                            prior=incompatible)["result"]
        python_incompatible, _ = python(backend, wallet_config, force_full=True, checkpoint=checkpoint,
                                        prior=incompatible)
    assert actual["update"] == expected == python_incompatible
    assert expected["facts"]["retracted_external_ids"] == [VECTORS["transactions"]["mixed"]]
    assert actual["canonical_values"] == canonical(tmp_path, values)


def test_lagging_backend_refuses_forced_rebuild_like_python(tmp_path):
    wallet_config = config()
    with electrum({"phase": VECTORS["phases"]["lagging"]}) as url:
        backend = {"name": "fixture", "kind": "electrum", "url": url}
        with pytest.raises(AppError) as raised:
            python(backend, wallet_config, force_full=True, checkpoint={"tip_height": 6})
        actual = run_native(tmp_path, backend, wallet_config, force_full=True,
                            checkpoint={"tip_height": 6})["error"]
        # An ordinary refresh has no lag guard in either observer.
        ordinary, _ = python(backend, wallet_config, checkpoint={"tip_height": 6})
        assert run_native(tmp_path, backend, wallet_config,
                          checkpoint={"tip_height": 6})["result"]["update"] == ordinary
    assert actual["code"] == raised.value.code == "backend_tip_behind"
    assert actual["message"] == str(raised.value)
    assert actual["hint"] == raised.value.hint
    assert actual["details"] == raised.value.details
    assert actual["retryable"] is raised.value.retryable is True


def test_incompatible_state_requires_rebuild_before_any_request(tmp_path):
    wallet_config = config()
    hits = []
    incompatible = StoredObserverState(identity=identity(wallet_config), payload={"schema_version": 2}, coverage=())
    with electrum({"phase": VECTORS["phases"]["initial"]}, hits) as url:
        backend = {"name": "fixture", "kind": "electrum", "url": url}
        with pytest.raises(AppError) as raised:
            python(backend, wallet_config, prior=incompatible)
        python_hits = list(hits)
        actual = run_native(tmp_path, backend, wallet_config, prior=incompatible)["error"]
    assert actual["code"] == raised.value.code == "observer_state_rebuild_required"
    assert python_hits == [] and hits == []


@pytest.mark.parametrize("highest", [None, {"0": 6}, {"1": "7", "0": -1}, {"0": True}])
def test_scan_horizon_follows_prior_coverage_and_checkpoint(tmp_path, highest):
    wallet_config = config()
    state = {"phase": VECTORS["phases"]["initial"]}
    with electrum(state) as url:
        backend = {"name": "fixture", "kind": "electrum", "url": url}
        first, values = python(backend, wallet_config)
        prior = stored(first, wallet_config)
        checkpoint = {**first["facts"]["freshness_checkpoint"]}
        if highest is not None:
            checkpoint["highest_used"] = highest
        expected, _ = python(backend, wallet_config, values=values, prior=prior, checkpoint=checkpoint)
        actual = run_native(tmp_path, backend, wallet_config, values=values, prior=prior,
                            checkpoint=checkpoint)["result"]
    assert actual["update"] == expected


def test_corrupt_store_fails_closed_without_partial_state(tmp_path):
    wallet_config = config()
    hits = []
    with electrum({"phase": VECTORS["phases"]["initial"]}, hits) as url:
        backend = {"name": "fixture", "kind": "electrum", "url": url}
        with pytest.raises(AppError) as raised:
            python(backend, wallet_config, values={"000000000000": b"not an update"})
        actual = run_native(tmp_path, backend, wallet_config, values={"000000000000": b"not an update"})
    assert actual["error"]["code"] == raised.value.code == "backend_sync_failed"
    assert actual["error"]["message"] == str(raised.value)
    assert actual["error"]["hint"] == raised.value.hint
    assert hits == []


@pytest.mark.parametrize("fence", ["offline", "consent", "cancelled"])
def test_fences_stop_before_any_request(tmp_path, fence):
    hits = []
    with electrum({"phase": VECTORS["phases"]["initial"]}, hits) as url:
        backend = {"name": "fixture", "kind": "electrum", "url": url}
        options = {"offline": fence == "offline", "consent": fence != "consent",
                   "cancelled": fence == "cancelled"}
        payload = {"op": "run", "config": config(), "backend": backend, "network": "elementsregtest",
                   "policy_asset": POLICY, "branch_keys": ["receive", "change"], **options}
        actual = native(tmp_path, payload)["error"]
    assert actual["code"] == {"offline": "network_egress_disabled", "consent": "interaction_required",
                              "cancelled": "cancelled"}[fence]
    assert hits == []


DESCRIPTORS = {
    "multipath": descriptor(),
    "taproot": descriptor("eltr"),
    "nested": descriptor("elsh(wpkh").replace("/*))", "/*)))"),
    "fixed": descriptor(path="0/5"),
    "receive_only": descriptor(path="0/*"),
}


def _separate(change_path):
    return config(descriptor(path="0/*"), change_descriptor=descriptor(path=change_path), synthesize_change=False)


@pytest.mark.parametrize("name,wallet_config", [
    *[(name, config(text)) for name, text in DESCRIPTORS.items()],
    ("separate_equivalent", _separate("1/*")),
    ("separate_different", _separate("2/*")),
])
def test_descriptor_translation_and_route_reasons_match_python(tmp_path, name, wallet_config):
    plan = load_descriptor_plan(wallet_config)
    try:
        expected_text = str(lwk_descriptor_for_plan(plan))
    except Exception:
        expected_text = None
    backends = [
        {"name": "e", "kind": "electrum", "url": "tcp://127.0.0.1:1"},
        {"name": "e", "kind": "electrum", "url": "ssl://liquid.example:995"},
        {"name": "e", "kind": "electrum", "url": "tcp://abc.onion:50001"},
        {"name": "e", "kind": "electrum", "url": "tcp://x:1", "proxy": "socks5h://127.0.0.1:9050"},
        {"name": "e", "kind": "electrum", "url": "tcp://x:1", "certificate": "/ca.pem"},
        {"name": "e", "kind": "electrum", "url": "ssl://x:1", "insecure": "yes"},
        {"name": "e", "kind": "electrum", "url": "ssl://x:1", "insecure": "no"},
        {"name": "e", "kind": "electrum", "url": "ssl://x:1", "insecure": "maybe"},
        {"name": "e", "kind": "electrum", "url": "tcp://x:1", "timeout": "30"},
        {"name": "e", "kind": "electrum", "url": "tcp://x:1", "timeout": 60},
        {"name": "e", "kind": "electrum", "url": "tcp://x:1", "timeout": "soon"},
        {"name": "e", "kind": "esplora", "url": "https://x"},
        {"name": "e", "kind": "liquid-esplora", "url": "https://x", "insecure": True},
        {"name": "e", "kind": "mempool", "url": "https://x", "certificate": "/ca.pem"},
        {"name": "e", "kind": "bitcoinrpc", "url": "http://x"},
    ]
    expected_reasons = []
    for backend in backends:
        sync_state = type("State", (), {"chain": "liquid", "descriptor_plan": plan})()
        try:
            expected_reasons.append(lwk_compatibility_reason(backend, sync_state))
        except AppError as exc:
            expected_reasons.append({"error": exc.code, "message": str(exc)})
    actual = native(tmp_path, {"op": "descriptor", "config": wallet_config, "backends": backends})["result"]
    assert actual["wollet"] == expected_text
    assert actual["reasons"] == expected_reasons
    if name in {"multipath", "taproot", "nested", "fixed", "separate_equivalent"}:
        assert expected_text is not None and expected_reasons[0] is None
    if name == "separate_different":
        assert expected_reasons[0] == "separate_change_descriptor"


@pytest.mark.parametrize("force_full", [False, True])
def test_request_sequence_matches_python_apart_from_version_negotiation(tmp_path, force_full):
    wallet_config = config()
    checkpoint = {"tip_height": 6} if force_full else {}
    python_hits, native_hits = [], []
    with electrum({"phase": VECTORS["phases"]["initial"]}, python_hits) as url:
        python({"name": "fixture", "kind": "electrum", "url": url}, wallet_config,
               force_full=force_full, checkpoint=checkpoint)
    with electrum({"phase": VECTORS["phases"]["initial"]}, native_hits) as url:
        run_native(tmp_path, {"name": "fixture", "kind": "electrum", "url": url}, wallet_config,
                   force_full=force_full, checkpoint=checkpoint)
    assert native_hits[0] == "server.version"
    assert native_hits[1:] == python_hits
    subscribes = 4 if force_full else 3
    assert python_hits.count("blockchain.headers.subscribe") == subscribes
