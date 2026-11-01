from __future__ import annotations

import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).parent))
from generate_analysis_sources import INSTANT, OTHER, TX, cache, database, dataset, evaluate, insert, raw, stored, update

DEST = ROOT / "crates/wallets/tests/data/analysis_legacy.json"
VALUES = [float("nan"), float("inf"), -float("inf"), "\ud800", "\udfff", "x\ud800y", "\ud800\udfff", "�", {"z": "\ud800", "a": float("inf")}, [float("nan"), "\udfff"]]


def acquired(status):
    return insert("chain_analysis_observations", {"profile_id": "p", "chain": "bitcoin", "network": "main", "txid": TX, "payload_json": raw(), "status_json": json.dumps(status), "source_name": "local", "observed_at": INSTANT})


def vectors():
    cases = []

    def add(name, sql, **kwargs):
        for observer in ["owner", "public"]:
            case = {"name": f"{name}-{observer}", "operation": "sources", "sql": sql, "as_of": INSTANT, "observer": observer, **kwargs}
            case["expected"] = evaluate(case)
            cases.append(case)

    for number, value in enumerate(VALUES):
        add(f"ignored-{number}", stored(raw_json=raw(unused=value)))
        add(f"cache-ignored-{number}", cache(payload_json=raw(unused=value)))
        add(f"config-ignored-{number}", stored() + update("wallets", {"config_json": json.dumps({"network": "regtest", "unused": value})}, "id='cold'"))
        for key in ["confirmations", "removed", "privacy_boundary", "version", "locktime", "vsize", "txid", "chain", "network", "chain_instance_id"]:
            add(f"raw-{key}-{number}", stored(raw_json=raw(**{key: value})))
        for key in ["value", "value_sats", "scriptpubkey", "scriptpubkey_address", "n"]:
            payload = json.loads(raw())
            payload["vout"][0][key] = value
            add(f"output-{key}-{number}", stored(raw_json=json.dumps(payload)))
        add(f"source-record-{number}", stored() + dataset(claims=[{"source_record_json": json.dumps({"unused": value, "source": value})}]))
        add(f"status-{number}", acquired({"confirmed": True, "block_height": 10, "unused": value}))
        for key in ["confirmed", "block_height"]:
            add(f"status-{key}-{number}", acquired({key: value}))
    for position, codepoint in enumerate([0xD800, 0xDBFF, 0xDC00, 0xDFFF]):
        char = chr(codepoint)
        add(f"surrogate-key-{position}", stored(raw_json=raw(**{char: {char: [float("nan"), char]}})))
        add(f"surrogate-key-status-{position}", acquired({char: [float("inf"), char]}))
        add(f"surrogate-key-source-record-{position}", stored() + dataset(claims=[{"source_record_json": json.dumps({char: [float("nan"), char]})}]))
        add(f"surrogate-address-{position}", stored(raw_json=raw(vout=[{"value": 10, "scriptpubkey_address": f" \t{char} \n"}])))
        add(f"surrogate-txid-{position}", stored(raw_json=raw(txid=TX[:32] + char + TX[32:])))
        add(f"surrogate-txid-fallback-{position}", stored(raw_json=raw(txid=TX[:32] + char + TX[32:]), external_id=OTHER))
        add(f"surrogate-boundary-{position}", stored(raw_json=raw(privacy_boundary=f"coin{char}join")))
        add(f"surrogate-script-{position}", stored(raw_json=raw(vout=[{"value": 10, "scriptpubkey": f"aa{char}bb"}])))
        add(f"surrogate-coinbase-{position}", stored(raw_json=raw(vin=[{"coinbase": char}])))
    return cases


def main():
    cases = vectors()
    encoded = "[\n" + ",\n".join(json.dumps(case, ensure_ascii=True, separators=(",", ":")) for case in cases) + "\n]\n"
    if "--write" in sys.argv:
        DEST.write_text(encoded)
    elif json.loads(DEST.read_text()) != cases:
        raise SystemExit("analysis legacy vectors drifted")
    print(f"{len(cases)} analysis legacy vectors")


if __name__ == "__main__":
    main()
