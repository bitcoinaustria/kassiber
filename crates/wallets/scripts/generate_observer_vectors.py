#!/usr/bin/env python3
"""Freeze pure observer state/reuse values from the unmodified Python oracle."""
import argparse
import copy
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[3]))

from kassiber import __version__
from kassiber.core.chain_observer.bdk import _record_fingerprint
from kassiber.core.chain_observer.bdk_persistence import deserialize_changeset, serialize_changeset
from kassiber.core.chain_observer.identity import observer_instance_id
from kassiber.core.chain_observer.provenance import canonical_graph_hash, canonical_observed_quantity_hash
from tests.test_bdk_observer import _descriptor_wallet
from tests.test_rust_wallet_acquisition import _transactions

DEST = Path(__file__).resolve().parents[1] / "tests/vectors/python_observer_state.json"


def vectors():
    wallet, _ = _descriptor_wallet()
    config = json.loads(wallet["config_json"])
    empty = {"schema_version": 1, "descriptor": config["descriptor"],
             "change_descriptor": config["change_descriptor"], "network": "BITCOIN",
             "local_chain": [], "tx_graph": {"txs": [], "txouts": [], "anchors": [],
             "last_seen": [], "first_seen": [], "last_evicted": []}, "indexer": [], "locked_outpoints": []}
    raw, tx, _, _ = _transactions([{"script_pubkey": "0014" + "aa" * 20}])
    txid = tx["txid"]
    # These hashes intentionally are NOT byte-symmetric: serialized order must
    # survive instead of accidentally converting through display-order txids.
    a, b = bytes(range(32)).hex(), bytes(range(32, 64)).hex()
    populated = copy.deepcopy(empty)
    populated["local_chain"] = [{"height": 101, "hash": b}, {"height": 0, "hash": a}, {"height": 99, "hash": None}]
    graph = populated["tx_graph"]
    graph["txs"] = list(raw.values())[::-1]
    graph["txouts"] = [{"txid": b, "vout": 2, "value_sats": 9007199254740993, "script_pubkey": "51"},
                       {"txid": a, "vout": 0, "value_sats": 100003, "script_pubkey": "0014" + "aa" * 20}]
    graph["anchors"] = [{"txid": b, "block_height": 101, "block_hash": a, "confirmation_time": 1700000001},
                        {"txid": a, "block_height": 100, "block_hash": b, "confirmation_time": 1700000000}]
    for name in ("last_seen", "first_seen", "last_evicted"):
        graph[name] = [{"txid": b, "timestamp": 1700000010}, {"txid": a, "timestamp": 1700000000}]
    populated["indexer"] = [{"descriptor_id": b, "last_revealed": 20}, {"descriptor_id": a, "last_revealed": 19}]
    populated["locked_outpoints"] = [{"txid": b, "vout": 2, "locked": False}, {"txid": a, "vout": 0, "locked": True}]
    coercions = copy.deepcopy(populated)
    coercions["schema_version"] = 1.0
    coercions["local_chain"][0]["height"] = "１０１"
    coercions["indexer"][0]["last_revealed"] = 20.75
    coercions["tx_graph"]["txouts"][0]["value_sats"] = "9_007_199_254_740_993"
    cases = [("empty", empty), ("all-components", populated), ("python-integers", coercions)]
    single = copy.deepcopy(empty)
    single["change_descriptor"] = None
    cases.append(("single", single))
    cases.append(("absent", {**empty, "descriptor": None, "change_descriptor": None, "network": None}))
    record = {"txid": txid, "vin": tx["vin"], "vout": tx["vout"],
              "position": {"status": {"confirmed": True, "block_height": 100, "block_hash": a,
                                      "block_time": 1700000000}, "confirmation_status": "confirmed",
                           "block_height": 100, "block_time": 1700000000, "confirmations": 3},
              "observed_at": 1700000000, "owned_scripts": ["0014" + "aa" * 20], "backend": "Fülcrum 😺"}
    fingerprints = []
    for name, changes in [("confirmed", {}), ("depth", {"position": {**record["position"], "confirmations": 300}}),
                          ("first-seen", {"observed_at": 1700000001}), ("owned", {"owned_scripts": []}),
                          ("backend", {"backend": "other"}), ("mempool", {"position": {"status": {"confirmed": False}, "confirmations": 0}})]:
        value = {**copy.deepcopy(record), **changes}
        fingerprints.append({"name": name, "version": f"1:{__version__}", "record": value,
                             "expected": _record_fingerprint(value["txid"], value["vin"], value["vout"], value["position"],
                                                             value["observed_at"], value["owned_scripts"], value["backend"])})
    quantity = {"wallet_id": "wallet 😺", "external_id": "\u001c" + txid.upper() + " ", "direction": " Inbound ",
                "asset": " btc ", "amount": 9007199254740993, "fee": 1001, "amount_includes_fee": 1}
    quantities = [quantity, {**quantity, "asset": "L-btc", "amount": -1001, "amount_includes_fee": False},
                  {**quantity, "asset": b.upper(), "fee": 0}]
    raw_graphs = [json.dumps(tx), json.dumps({"unicode": "😺\u001cß", "floats": [1e-7, 1.0], "large": 9007199254740993}),
                  "not-json", "", "[1, null, true]", '{"duplicate":0,"duplicate":2}', '{"n":NaN,"i":Infinity}']
    return {"changesets": [{"name": n, "input": p, "expected": serialize_changeset(deserialize_changeset(p))} for n, p in cases],
            "fingerprints": fingerprints,
            "quantities": [{"input": p, "expected": canonical_observed_quantity_hash(p)} for p in quantities],
            "graphs": [{"input": p, "expected": canonical_graph_hash(p)} for p in raw_graphs],
            "identity": {"logical_wallet": "wallet 😺", "source_wallet": "source", "source_key": "xpub:p2wpkh",
                         "expected": observer_instance_id("wallet 😺", "source", "xpub:p2wpkh")}}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    encoded = json.dumps(vectors(), ensure_ascii=False, indent=2) + "\n"
    if args.check:
        if DEST.read_text() != encoded:
            raise SystemExit("observer state vectors have drifted")
    else:
        DEST.write_text(encoded)
