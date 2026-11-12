#!/usr/bin/env python3
"""Freeze Bitcoin record/composition facts from the unmodified Python reference.

No database or network access. Only the observer-application and policy-coverage
write seams are replaced; Python's actual record composer runs unchanged.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import sys
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[3]))
from kassiber.core import sync, sync_backends
from kassiber.core.chain_observer.contract import ChainFacts
from kassiber.errors import AppError

DEST = Path(__file__).resolve().parents[1] / "tests/vectors/python_bitcoin_records.json"
A, B, C = "0014" + "ab" * 20, "0014" + "cd" * 20, "51"
TXID, PARENT = "ab" * 32, "cd" * 32


def evaluate(case):
    payload = case["input"]
    try:
        if case["operation"] == "compose":
            updates = tuple(SimpleNamespace(
                identity=SimpleNamespace(id=o["observer_id"], observer_kind=o.get("observer_kind", "bdk")),
                update={"facts": {"reused_records": o.get("reused_records", 0)}},
                facts=ChainFacts(transaction_records=tuple(o.get("records", ())),
                    outputs=tuple(o.get("outputs", ())),
                    retracted_external_ids=tuple(o.get("retracted_external_ids", ())),
                    freshness_checkpoint=o.get("checkpoint", {})),
            ) for o in payload.get("observers", ()))
            fetch = sync.WalletBackendFetch(backend={"name": payload.get("backend", "fixture")},
                sync_state=None, normalized_records=tuple(payload.get("adapter_records", ())),
                adapter_meta=payload.get("adapter_metadata", {}), kind="electrum", started=0,
                force_full=False, observer_updates=updates)
            with patch.object(sync, "apply_prepared_observer_update", lambda _, update: update.facts), \
                 patch.object(sync, "record_observer_policy_coverage", lambda *_: None):
                result = sync.apply_fetch_observer_updates(None, fetch)
            metadata = dict(result.adapter_meta)
            provenance = metadata.pop("_chain_observation_provenance", {}).get("entries", [])
            result = {"records": result.normalized_records, "metadata": metadata, "provenance_entries": provenance}
        elif case["operation"] == "electrum":
            result = sync_backends.record_from_electrum_tx(payload["txid"], payload["tx"], payload["occurred_at"],
                payload["scripts"], payload.get("backend", "fixture"), lambda _: None)
        else:
            result = sync_backends.record_from_bitcoin_esplora_tx(payload["tx"], payload["scripts"], payload.get("backend", "fixture"))
        return {"result": json.loads(json.dumps(result, default=str))}
    except AppError as exc:
        return {"error": {"code": exc.code, "message": str(exc), "details": exc.details, "hint": exc.hint}}


def vectors():
    cases = []
    base = {"txid": TXID, "vin": [{"txid": PARENT, "vout": 0, "prevout": {"value": 100_000, "scriptpubkey": A}}],
            "vout": [{"value": 60_000, "scriptpubkey": B}, {"value": 39_000, "scriptpubkey": A}],
            "status": {"confirmed": True, "block_time": 1_700_000_000}}

    def record(name, tx=None, scripts=None, operation="esplora", **extra):
        payload = {"tx": copy.deepcopy(base if tx is None else tx), "scripts": scripts if scripts is not None else [A], **extra}
        cases.append({"name": name, "operation": operation, "input": payload})
        return payload["tx"]

    record("owned-withdrawal")
    record("receive", scripts=[B])
    record("cross-family-consolidation", scripts=[A, B])
    record("graph-scoped-owned-scripts", scripts=[A, B, C])
    record("untracked", scripts=[C])
    record("script-normalization", scripts=[" \t" + A.upper() + "\u001c"])
    record("explicit-fee")["fee"] = 800
    record("negative-fee-clamp")["fee"] = -1
    record("mempool")["status"] = {"confirmed": False}
    tx = record("observed-mempool")
    tx.update(status={"confirmed": False}, observed_at=1_700_000_010)
    record("zero-time")["status"]["block_time"] = 0
    record("block-time-without-confirmed-flag")["status"]["confirmed"] = False
    record("negative-time")["status"]["block_time"] = -1
    record("early-time")["status"]["block_time"] = -62_135_596_800
    record("late-time")["status"]["block_time"] = 253_402_300_799
    tx = record("collaborative-withdrawal")
    tx["vin"].append({"txid": "ef" * 32, "vout": 1, "prevout": {"value": 3000, "scriptpubkey": C}})
    tx = record("collaborative-receive", scripts=[A, B])
    tx["vin"].append({"txid": "ef" * 32, "vout": 1, "prevout": {"value": 3000, "scriptpubkey": C}})
    tx["vout"][0]["value"] = 70_000
    tx = record("equal-collaborative")
    tx["vout"][1]["value"] = 100_000
    tx["vin"].append({"txid": "ef" * 32, "vout": 1})
    tx = record("incomplete-graph", scripts=[A, B, C])
    tx["vin"].append({"txid": "ef" * 32, "vout": 1})
    tx = record("coinbase", scripts=[B])
    tx["vin"] = [{"txid": "0" * 64, "vout": 0xffffffff}]
    tx = record("no-inputs", scripts=[B])
    tx["vin"] = []
    for sats in [0, 1, 10, 99, 100, 101, 1000, 100_000_000, 9_007_199_254_740_993]:
        tx = record(f"exact-receive-{sats}", scripts=[B])
        tx["vin"] = []
        tx["vout"] = [{"scriptpubkey": B, "value": sats}]
    tx = record("legacy-sats-btc-graph")
    tx["vin"][0]["prevout"] = {"scriptPubKey": {"hex": A}, "value_sats": "100_000"}
    tx["vout"] = [{"script_hex": B, "value": "0.0006"}, {"scriptPubKey": A, "value": "0.000390005"}]
    for field, value in [("privacy_boundary", "payjoin"), ("privacyHopKind", "payment-in-coinjoin"),
                         ("likelyCoinJoin", True), ("raw_json", {"privacy_hop": "sweep"}),
                         ("raw_json", '{"isLikelyCoinJoin": "YES"}'), ("privacy_boundary", "invalid")]:
        tx = record(f"privacy-{field}-{value}")
        tx[field] = value
    preimage = bytes(range(32))
    hashlock = hashlib.new("ripemd160", hashlib.sha256(preimage).digest()).digest()
    redeem = b"\xa9\x14" + hashlock + b"\x87\x63\x21" + b"\x02" * 33 + b"\x67\x03\x01\x02\x03\xb1\x75\x21" + b"\x03" * 33 + b"\x68\xac"
    for name, witness in [("claim", ["aa", preimage.hex(), redeem.hex()]),
                          ("claim-reverse", [preimage.hex(), (b"\x82\x01\x20\x88" + redeem).hex()]),
                          ("refund", ["aa", "", redeem.hex()]),
                          ("bad-template", [preimage.hex(), (redeem + b"\x00").hex()]),
                          ("wrong-preimage", ["ff" * 32, redeem.hex()])]:
        tx = record(name)
        tx["vin"][0]["witness"] = witness
        if name == "claim":
            changed = record("claim-unrecognized-witness-field", tx=tx)
            changed["vin"][0]["txinwitness"] = changed["vin"][0].pop("witness")
            for suffix, change in [("mixed", [{"txid": "ef" * 32, "vout": 0}]), ("batched", copy.deepcopy(tx["vin"]))]:
                changed = record(f"claim-{suffix}", tx=tx)
                changed["vin"].extend(change)
            changed = record("claim-invalid-outpoint", tx=tx)
            changed["vin"][0]["vout"] = True
        if name == "refund":
            changed = record("refund-malformed-witness-items", tx=tx)
            changed["vin"][0]["witness"] = ["not hex", redeem.hex()]
            changed = record("refund-batched", tx=tx)
            changed["vin"].append({**copy.deepcopy(tx["vin"][0]), "vout": 1})
            changed = record("refund-duplicate", tx=tx)
            changed["vin"] *= 2
    # Already normalized decoded Electrum graph; the record boundary preserves
    # its wire fields and stamps the current graph version over an older one.
    tx = copy.deepcopy(base)
    tx["_kassiber_electrum_graph"] = {"kind": "bitcoin_electrum", "version": 1}
    for occurred in ["1970-01-01T00:00:00Z", "2023-11-14T22:13:20Z"]:
        record(f"electrum-{occurred}", tx=tx, operation="electrum", txid=TXID, occurred_at=occurred)

    def observer(id, records=(), **kwargs):
        return {"observer_id": id, "observer_kind": "bdk", "records": list(records), **kwargs}

    def compose(name, observers, **kwargs):
        cases.append({"name": name, "operation": "compose", "input": {"observers": observers, **kwargs}})

    family_a = sync_backends.record_from_bitcoin_esplora_tx({**base, "observer": "bdk"}, [A], "fixture")
    family_b = sync_backends.record_from_bitcoin_esplora_tx({**base, "observer": "bdk"}, [B], "fixture")
    family_a, family_b = json.loads(json.dumps([family_a, family_b], default=str))
    compose("cross-family-union", [observer("a", [family_a]), observer("b", [family_b])])
    two_inputs = copy.deepcopy(base)
    two_inputs["vin"].append({"txid":"ef"*32,"vout":1,"prevout":{"value":2000,"scriptpubkey":B}})
    raw_a, raw_b = copy.deepcopy(two_inputs), copy.deepcopy(two_inputs)
    raw_a.update(observer="bdk", observer_owned_scripts=[A])
    raw_b.update(observer="bdk", observer_owned_scripts=[B], distinct_metadata="second")
    raw_a["vin"][1]["prevout"] = None
    raw_b["vin"][0]["prevout"] = None
    compose("complementary-prevouts-and-first-tie", [observer("a", [dict(family_a,raw_json=json.dumps(raw_a))]), observer("b", [dict(family_b,raw_json=json.dumps(raw_b))])])
    raw_sparse = json.loads(family_a["raw_json"])
    raw_sparse["vin"][0]["prevout"] = None
    raw_sparse["distinct_metadata"] = "sparse"
    compose("richest-graph-precedes-sparse", [observer("a",[dict(family_a,raw_json=json.dumps(raw_sparse))]),observer("b",[family_b])])
    compose("provenance-txid-fallback", [observer("b", [dict(family_b,direction="other")]), observer("a", [dict(family_a,direction="other")])])
    compose("cross-observer-veto", [observer("a", retracted_external_ids=[" " + TXID.upper() + " ", PARENT, PARENT]), observer("b", [family_b])])
    compose("empty-output-snapshot", [observer("a")])
    compose("mixed-records-refused", [observer("a")], adapter_records=[family_a])
    compose("mixed-empty-inventory-refused", [observer("a")], adapter_metadata={"utxos": []})
    compose("adapter-only", [], adapter_records=[family_a], adapter_metadata={"utxos": []})
    compose("checkpoints-outputs-reuse", [observer("b", [family_b], outputs=[{"txid":TXID,"vout":1,"value":3}], checkpoint={"highest_used":{"0":3,"1":"bad"}}, reused_records=2),
        observer("a", [family_b], outputs=[{"txid":TXID,"vout":1,"value":4},{"txid":PARENT,"vout":0}], checkpoint={"highest_used":{"0":1,"2":"5"}}, reused_records=1)], adapter_metadata={"freshness_checkpoint":{"existing":True,"highest_used":{"9":10}}})
    opaque = {**family_a, "raw_json": "opaque importer text"}
    compose("opaque-first-record-wins", [observer("a", [opaque]), observer("b", [dict(opaque, amount="999")])])
    external = dict(opaque, external_id=TXID)
    external.pop("txid")
    compose("external-id-key", [observer("a", [external], retracted_external_ids=[TXID,"literal", "literal", "Literal"]),observer("b",[dict(opaque,txid="literal")])])
    compose("non-btc-observer-stays-distinct",[observer("a",[dict(family_a,asset="LBTC")])])
    richer = json.loads(family_a["raw_json"])
    richer["vin"][0]["txid"] = "ef"*32
    compose("inconsistent-inputs-refused", [observer("a", [family_a]), observer("b", [dict(family_b,raw_json=json.dumps(richer))])])
    conflict = json.loads(family_b["raw_json"])
    conflict["vin"][0]["prevout"]["value"] += 1
    compose("conflicting-prevouts-refused", [observer("a", [family_a]), observer("b", [dict(family_b,raw_json=json.dumps(conflict))])])
    # Exact keys have precedence over txid aggregation in provenance, as Python.
    compose("provenance-key-precedence", [observer("b", [family_b]), observer("a", [family_a])])
    for case in cases:
        case["input"] = json.loads(json.dumps(case["input"], default=str))
        case["expected"] = evaluate(case)
    return cases


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    generated = json.dumps(vectors(), indent=2, ensure_ascii=True) + "\n"
    if args.check:
        if DEST.read_text() != generated:
            raise SystemExit("Bitcoin record vectors differ from Python")
    else:
        DEST.write_text(generated)
