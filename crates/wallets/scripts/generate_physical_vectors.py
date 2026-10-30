#!/usr/bin/env python3
"""Unchanged Python physical-scope and confirmation-state oracle for publication."""
import argparse
from datetime import datetime, timedelta, timezone
import json
from pathlib import Path
import random
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[3]))
from kassiber import transfers
from kassiber.core import ownership_transfers as ownership
from kassiber.core.chain_observer.provenance import (
    canonical_graph_hash,
    canonical_observed_quantity_hash,
)

DEST = Path(__file__).resolve().parents[1] / "tests/vectors/python_physical.json"
TXID = "ab" * 32
OTHER = "cd" * 32
ASSET_ID = "6f" * 32
EPOCH = datetime(1, 1, 1, tzinfo=timezone.utc)


def graph(txid, **extra):
    payload = {"vin": [{"txid": OTHER, "vout": 1, "prevout": {"scriptpubkey": "0014aa", "value": 5}}],
               "vout": [{"scriptpubkey": "0014bb", "value": 4}]}
    if txid is not None:
        payload["txid"] = txid
    payload.update(extra)
    return payload


RAWS = [
    None, "", "{}", "not json", "[1,2]", "null",
    json.dumps(graph(TXID, status={"confirmed": True, "block_height": 5})),
    json.dumps(graph(TXID, status={"confirmed": False})),
    json.dumps(graph(TXID.upper(), status={"confirmed": "yes"})),
    json.dumps(graph(OTHER, status={"confirmed": True})),
    json.dumps(graph("provider-label")),
    json.dumps(graph(None, status={"confirmed": True})),
    json.dumps(graph(TXID, chain="bitcoin", network="testnet", status={"confirmed": True})),
    json.dumps(graph(TXID, chain="liquid", network="liquidv1", component={"asset_id": ASSET_ID, "asset": "LBTC"})),
    json.dumps(graph(TXID, chain="liquid", network="liquidv1")),
    json.dumps(graph(TXID, chain="dogecoin")),
    json.dumps(graph(TXID, network="bogus")),
    json.dumps(graph(TXID, network="Main ")),
    json.dumps({"tx": graph(TXID, status={"confirmed": True})}),
    json.dumps({"tx": graph(TXID), "txid": OTHER}),
    json.dumps({"ownership_graph": graph(None), "txid": TXID}),
    json.dumps({"TxHash": TXID, "source": "csv"}),
    json.dumps({"Transaction ID": TXID.upper()}),
    json.dumps({"swap_id": TXID}),
    json.dumps({"external_id_kind": "txid"}),
    json.dumps([{"txid": TXID, "vout": 0, "category": "receive"}]),
    json.dumps([{"txid": OTHER}]),
    json.dumps({"vin": [], "vout": [], "status": {"confirmed": True}}),
    json.dumps({"vin": "x", "vout": []}),
    json.dumps({"txid": 7, "vin": [], "vout": []}),
    json.dumps({"txid": "", "vin": [], "vout": [], "tx": {"txid": TXID, "vin": [], "vout": []}}),
    json.dumps({"txid": None, "ownership_graph": {"txid": TXID, "vin": [], "vout": []}}),
    json.dumps({"vin": [], "vout": [], "chain": "bitcoin", "network": "regtest", "status": {"confirmed": False}}),
    json.dumps({"chain": "lnd", "network": "mainnet"}),
    json.dumps({"chain": "liquid", "network": "testnet"}),
    json.dumps({"chain": "weird"}),
    json.dumps({"bitcoin_network": "signet"}),
    json.dumps({"network": "main", "chain_network": "test"}),
    "{\"vin\": [{\"txid\": \"" + OTHER + "\", \"vout\": Infinity}], \"vout\": []}",
    "{\"vin\": [{\"txid\": \"bad\", \"vout\": Infinity}], \"vout\": [], \"txid\": \"" + TXID + "\"}",
]
CONFIGS = [None, "", "{}", "garbage", json.dumps({"chain": "bitcoin", "network": "testnet"}),
           json.dumps({"chain": "liquid", "network": "liquidv1"}), json.dumps({"network": "bogus"}),
           json.dumps({"chain": "core-lightning"}), json.dumps({"chain": "bitcoin", "network": "main"})]
TIMES = [None, "", "2024-01-01T00:00:00Z", "2024-01-01T00:00:00.5+01:00", "2024-01-02 03:04:05",
         "not a time", "2024-01-01T00:00:00Z ", 20240101]


POOLS = {
    "id": lambda index: [f"row-{index}", f"owned-derive:{index}", f"custody-tax:{index}", f"cross-split:{index}"],
    "wallet_id": lambda index: ["w1", "w2"],
    "external_id": lambda index: [TXID, TXID.upper(), f" {TXID} ", OTHER, "swap-123", None, ""],
    "external_id_kind": lambda index: [None, "txid", " TXID ", "provider", ""],
    "asset": lambda index: ["BTC", "btc", "LBTC", "L-BTC", "ETH", None, ASSET_ID],
    "confirmed_at": lambda index: [None, "", "2024-01-01T00:00:00Z"],
    "raw_json": lambda index: RAWS,
    "wallet_kind": lambda index: ["descriptor", "lnd", "core_lightning", None, "address"],
    "wallet_config_json": lambda index: CONFIGS,
    "observation_observed_at": lambda index: TIMES,
    "observation_authority_version": lambda index: [None, 1, "1", 1.0, 2, "x", True],
}
COHERENT = [json.dumps(graph(TXID, status={"confirmed": True, "block_height": 5})),
            json.dumps(graph(TXID, status={"confirmed": False})),
            json.dumps(graph(TXID, chain="bitcoin", network="main", status={"confirmed": True})),
            json.dumps({"tx": graph(TXID, status={"confirmed": False})})]


def row(rng, index):
    coherent = rng.random() < 0.6
    value = {
        "id": f"row-{index}",
        "wallet_id": rng.choice(["w1", "w2"]),
        "external_id": TXID,
        "external_id_kind": "txid",
        "asset": "BTC",
        "direction": rng.choice(["inbound", "outbound"]),
        "amount": rng.choice([0, 1000, 99000]),
        "fee": rng.choice([0, 7]),
        "amount_includes_fee": rng.choice([0, 1]),
        "confirmed_at": rng.choice([None, "2024-01-01T00:00:00Z"]),
        "raw_json": rng.choice(COHERENT),
        "wallet_kind": "descriptor",
        "wallet_config_json": json.dumps({"chain": "bitcoin", "network": "main"}),
        "observation_observed_at": rng.choice(TIMES[2:5]),
        "observation_authority_version": 1,
    }
    keys = sorted(POOLS)
    for key in rng.sample(keys, rng.randint(0, 2) if coherent else rng.randint(3, len(keys))):
        value[key] = rng.choice(POOLS[key](index))
    if rng.random() < 0.15:
        value["config_json"] = rng.choice(CONFIGS)
    raw = value["raw_json"]
    good_graph = rng.random() < 0.85
    value["observation_graph_hash"] = canonical_graph_hash(raw) if good_graph else "0" * 64
    value["observation_quantity_hash"] = (
        canonical_observed_quantity_hash(value) if rng.random() < 0.85 else "1" * 64
    )
    return value


def call(function, *args):
    try:
        return {"result": function(*args)}
    except Exception as exc:  # The vector records which inputs Python refuses.
        return {"error": type(exc).__name__}


def observation(value):
    result = ownership.capture_confirmation_observation(value)
    if result is None:
        return None
    return [list(result.physical_scope), (result.observed_at - EPOCH) // timedelta(microseconds=1), result.confirmed]


def conflict(value):
    parsed = ownership._parse_onchain_tx(value.get("raw_json"), allow_partial=True)
    scope = ownership._physical_conflict_scope(value, parsed)
    return None if scope is None else list(scope[:3])


def states(rows):
    return sorted([list(key), value] for key, value in ownership._current_confirmation_states(rows).items())


def vectors():
    rng = random.Random(21)
    rows = [row(rng, index) for index in range(900)]
    cases = []
    chain_conflict = {"id": "explicit-chain-conflict", "wallet_id": "w1", "wallet_kind": "descriptor",
                      "external_id": TXID, "external_id_kind": "txid", "asset": "BTC",
                      "raw_json": json.dumps(graph(TXID, chain="bitcoin")),
                      "wallet_config_json": json.dumps({"chain": "liquid"})}
    for value in [*rows, chain_conflict]:
        scope = call(transfers.onchain_transfer_scope, value)
        cases.append({
            "row": value,
            "transfer_scope": {"result": list(scope["result"])} if scope.get("result") else scope,
            "domain": call(lambda v: list(transfers.bitcoin_network_domain_evidence(v)), value),
            "conflict": call(conflict, value),
            "observation": call(observation, value),
        })
    groups = []
    for _ in range(300):
        group = rng.sample(rows, rng.randint(1, 8))
        groups.append({"rows": [rows.index(item) for item in group], "states": call(states, group)})
    return {"generator": "crates/wallets/scripts/generate_physical_vectors.py", "cases": cases, "groups": groups}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    text = json.dumps(vectors(), indent=1, sort_keys=True, ensure_ascii=False) + "\n"
    if args.check:
        if DEST.read_text(encoding="utf-8") != text:
            raise SystemExit(f"{DEST} is stale")
        return
    DEST.write_text(text, encoding="utf-8")


if __name__ == "__main__":
    main()
