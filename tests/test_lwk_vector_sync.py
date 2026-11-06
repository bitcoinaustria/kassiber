"""Python LWK observer and `wallets sync` over deterministic Liquid chain states.

The vectors (`crates/wallets/tests/vectors/lwk_chain.json`, from the Rust
`lwk_vectors` example) are real Elements regtest transactions served by a
loopback Electrum fake, so the pinned `lwk` binding scans, unblinds and
projects them exactly as it would against a live server.
"""
from contextlib import contextmanager
import json
from pathlib import Path
import socketserver
import subprocess
import sys
import threading
from unittest.mock import patch

import pytest

from kassiber.cli import handlers
from kassiber.core.chain_observer.contract import ObserverPrepareRequest
from kassiber.core.chain_observer.identity import identities_for_wallet
from kassiber.core.chain_observer.lwk import LwkObserver
from kassiber.core.esplora_history import full_scan_checkpoint
from kassiber.errors import AppError
from kassiber.wallet_descriptors import load_descriptor_plan
from tests.rust_sync_env import child_env
from tests.rust_sync_parity import connect, create_book, deterministic_sources, runtime
from tests.test_lwk_observer import wallet_row

ROOT = Path(__file__).resolve().parents[1]
VECTORS = json.loads((ROOT / "crates/wallets/tests/vectors/lwk_chain.json").read_text())
POLICY = VECTORS["policy_asset"]
TRANSACTIONS = VECTORS["transactions"]


@contextmanager
def electrum(state):
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


def observer_config():
    return {"chain": "liquid", "network": "elementsregtest", "descriptor": VECTORS["descriptor"], "gap_limit": 3}


def prepare(url, *, force_full=False, checkpoint=None):
    config = observer_config()
    identity = identities_for_wallet(wallet_row(config), observer_kind="lwk")[0]
    backend = {"name": "fixture", "kind": "electrum", "url": url}
    observer = LwkObserver(identity=identity, backend=backend, descriptor_plan=load_descriptor_plan(config),
                           policy_asset_id=POLICY, stored_values={})
    with patch("kassiber.egress_policy.require_egress_enabled", lambda *a, **k: None):
        return observer.prepare(
            ObserverPrepareRequest(backend_name="fixture", backend_kind="electrum",
                                   force_full=force_full, checkpoint=checkpoint or {}),
            None,
        )


def records_by_name(update):
    names = {txid: name for name, txid in TRANSACTIONS.items()}
    result = {}
    for record in update["facts"]["transaction_records"]:
        result.setdefault(names[record["txid"]], {})[record["asset"]] = record
    return result


def book(tmp_path):
    path = tmp_path / "book.sqlite"
    create_book(path, {**observer_config(), "backend": "fixture", "policy_asset": POLICY})
    return path


def sync(path, url, *, index=0, force_full=False):
    conn = connect(path)
    try:
        with deterministic_sources(f"r{index}", index * 1000):
            return handlers.sync_wallet(conn, runtime("electrum", url), "w", "p", wallet_ref="a",
                                        sync_all=False, force_full=force_full)
    finally:
        conn.close()


def live_rows(path):
    conn = connect(path)
    try:
        names = {txid: name for name, txid in TRANSACTIONS.items()}
        return {
            (names[row["external_id"]], row["asset"]): (row["direction"], row["amount"], row["fee"])
            for row in conn.execute(
                "SELECT external_id, asset, direction, amount, fee FROM transactions "
                "WHERE wallet_id = 'a' AND excluded = 0"
            )
        }
    finally:
        conn.close()


def test_explicit_wallet_outputs_are_booked_with_their_public_amount():
    # An unblinded output to a wallet script carries a public amount and
    # asset. LWK 0.18.0 leaves such secrets out of its balance and UTXO set,
    # so the receipt and its later spend must still reach the books.
    with electrum({"phase": VECTORS["phases"]["initial"]}) as url:
        update = prepare(url)
    records = records_by_name(update)
    fund = json.loads(records["fund"]["LBTC"]["raw_json"])
    assert fund["component"]["net_sats"] == 150_000
    assert records["fund"]["LBTC"]["amount"] == "0.0015"
    # The explicit receipt is the mixed spend's only owned input.
    mixed = records["mixed"]["LBTC"]
    assert (mixed["direction"], mixed["kind"], mixed["amount"]) == ("outbound", "withdrawal", "0.00021")
    assert mixed["amount_includes_fee"] is True
    assert json.loads(mixed["raw_json"])["component"]["net_sats"] == -21_000


def test_unspent_explicit_wallet_output_is_published_in_the_inventory():
    with electrum({"phase": VECTORS["phases"]["reorg"]}) as url:
        update = prepare(url)
    fund = TRANSACTIONS["fund"]
    explicit = [o for o in update["facts"]["outputs"] if o["txid"] == fund and o["vout"] == 1]
    assert len(explicit) == 1
    assert explicit[0]["amount_sats"] == 50_000 and explicit[0]["asset"] == "LBTC"
    assert explicit[0]["raw"] == {"source": "lwk_wollet", "confidential": False}
    records = records_by_name(update)
    assert "mixed" not in records
    assert records["fund"]["LBTC"]["amount"] == "0.0015"


def test_book_synced_without_explicit_outputs_is_repaired_by_the_next_ordinary_sync(tmp_path):
    path = book(tmp_path)
    with electrum({"phase": VECTORS["phases"]["lbtc_initial"]}) as url:
        # Model a book written before the fix: explicit secrets ignored.
        with patch("kassiber.core.chain_observer.lwk._explicit_owned_deltas", return_value={}):
            sync(path, url)
        assert live_rows(path)[("lbtc_mixed", "LBTC")][0] == "inbound"
        outcome = sync(path, url, index=1)
    assert outcome[0]["status"] == "synced"
    rows = live_rows(path)
    assert rows[("lbtc_fund", "LBTC")] == ("inbound", 150_000_000, 0)
    assert rows[("lbtc_mixed", "LBTC")] == ("outbound", 21_000_000, 0)
    assert sum(amount if direction == "inbound" else -amount - fee
               for direction, amount, fee in rows.values()) == 93_000_000


def test_forced_rebuild_refuses_a_backend_behind_the_book(tmp_path):
    path = book(tmp_path)
    state = {"phase": VECTORS["phases"]["lbtc_initial"]}
    with electrum(state) as url:
        sync(path, url)
        before = live_rows(path)
        state["phase"] = VECTORS["phases"]["lbtc_lagging"]
        with pytest.raises(AppError) as raised:
            sync(path, url, index=1, force_full=True)
    assert raised.value.code == "backend_tip_behind"
    assert raised.value.details["persisted_height"] == VECTORS["phases"]["lbtc_initial"]["tip"]
    assert live_rows(path) == before


def test_full_scan_checkpoint_keeps_only_each_observer_rebuild_baseline():
    checkpoint = {
        "highest_used": {"0": 9},
        "esplora_tip_hash": "aa",
        "observer_instances": {
            "lwk:b": {"observer": "lwk", "tip_height": 6, "tip_hash": "bb", "highest_used": {"0": 4},
                      "canonical_txids": ["11"]},
            "bdk:a": {"tip_height": 7},
            "broken": "not a mapping",
        },
    }
    retained = full_scan_checkpoint(checkpoint)
    assert retained == {"observer_instances": {"bdk:a": {"tip_height": 7},
                                               "lwk:b": {"tip_height": 6, "canonical_txids": ["11"]}}}
    assert full_scan_checkpoint(retained) == retained
    assert full_scan_checkpoint({"highest_used": {"0": 1}}) == {}


def test_rows_a_lagging_forced_rebuild_retracted_return_on_the_next_sync(tmp_path):
    path = book(tmp_path)
    state = {"phase": VECTORS["phases"]["lbtc_initial"]}
    with electrum(state) as url:
        sync(path, url)
        before = live_rows(path)
        # Model a book damaged before the fix: the lag guard never fired.
        state["phase"] = VECTORS["phases"]["lbtc_lagging"]
        with patch("kassiber.core.chain_observer.lwk._require_lwk_tip_not_behind"):
            damaged = sync(path, url, index=1, force_full=True)
        assert damaged[0]["retracted"] == 4
        assert {name for name, _asset in live_rows(path)} == {"lbtc_fund", "lbtc_spend"}
        state["phase"] = VECTORS["phases"]["lbtc_initial"]
        repaired = sync(path, url, index=2)
    assert repaired[0]["status"] == "synced"
    assert live_rows(path) == before


def test_multi_asset_history_does_not_force_a_repair_rescan(tmp_path):
    path = book(tmp_path)
    with electrum({"phase": VECTORS["phases"]["initial"]}) as url:
        first = sync(path, url)
        second = sync(path, url, index=1)
    for outcome in (first, second):
        assert outcome[0]["status"] == "synced"
        assert not outcome[0].get("negative_balance_rescan", {}).get("triggered")
    rows = live_rows(path)
    assert rows[("fund", VECTORS["issued_asset"])] == ("inbound", 7_000_000, 0)
    assert rows[("fund", "LBTC")] == ("inbound", 150_000_000, 0)


_PROBE = """
import json, sys
sys.path.insert(0, sys.argv[1])
from tests.test_lwk_vector_sync import prepare
update = prepare(sys.argv[2])
print(json.dumps(update, sort_keys=True))
"""


def test_output_order_is_the_same_in_every_process(tmp_path):
    # LWK lists UTXOs in hash-map order, independent of PYTHONHASHSEED.
    env = {**child_env(tmp_path), "PYTHONHASHSEED": "0"}
    updates = set()
    with electrum({"phase": VECTORS["phases"]["initial"]}) as url:
        for _ in range(8):
            result = subprocess.run([sys.executable, "-c", _PROBE, str(ROOT), url], capture_output=True,
                                    text=True, timeout=120, env=env, cwd=tmp_path, check=True)
            updates.add(result.stdout.strip().splitlines()[-1])
    assert len(updates) == 1
    order = [[o["txid"], o["vout"]] for o in json.loads(updates.pop())["facts"]["outputs"]]
    assert order == sorted(order) and len(order) == 4
