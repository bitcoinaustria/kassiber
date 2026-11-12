"""Python CoinJoin shape results for the Rust ports (wallets, reports, journals).

Covers the hand-written shapes in tests/coinjoin_shape_vectors.py plus leg
spellings that exercise each port's value and script parsers. Usage:
``generate_coinjoin_shapes.py`` checks, ``--write`` rewrites.
"""
from __future__ import annotations

import copy
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core import privacy_hygiene as reference
from tests.coinjoin_shape_vectors import distinct, script, tx, vectors as shapes

DEST = ROOT / "crates/wallets/tests/data/coinjoin_shapes.json"


def spellings():
    base = next(vector["raw"] for vector in shapes() if vector["name"] == "wabisabi_four_per_denomination")
    gap = next(vector["raw"] for vector in shapes() if vector["name"] == "gap_5in_13out")
    rows = []

    def add(name, raw):
        rows.append({"name": name, "raw": raw})

    for name, change in (
        ("prevout-not-a-mapping", lambda raw: raw["vin"][3].update(prevout="text", value=raw["vin"][3]["prevout"]["value"])),
        ("entry-value-beside-prevout", lambda raw: raw["vin"][2].update(value=1) or raw["vin"][2]["prevout"].pop("value")),
        ("prevout-value-sats", lambda raw: raw["vin"][1]["prevout"].update(value_sats=str(raw["vin"][1]["prevout"].pop("value")))),
        ("prevout-btc-string", lambda raw: raw["vin"][0]["prevout"].update(value="0.0003")),
        ("output-btc-float", lambda raw: [entry.update(value=entry["value"] / 100_000_000) for entry in raw["vout"]]),
        ("output-script-uppercase", lambda raw: raw["vout"][5].update(scriptpubkey=raw["vout"][5]["scriptpubkey"].upper())),
        ("script-uppercase-reuses-input", lambda raw: raw["vout"][5].update(scriptpubkey=raw["vin"][5]["prevout"]["scriptpubkey"].upper())),
        ("script-spaced-bytes", lambda raw: raw["vout"][6].update(scriptpubkey=" ".join(raw["vout"][6]["scriptpubkey"][i:i + 2] for i in range(0, 44, 2)))),
        ("script-double-spaced-bytes", lambda raw: raw["vout"][6].update(scriptpubkey="  ".join(raw["vout"][6]["scriptpubkey"][i:i + 2] for i in range(0, 44, 2)))),
        ("script-vertical-tab", lambda raw: raw["vin"][7]["prevout"].update(scriptpubkey="00\x0b\x0b" + raw["vin"][7]["prevout"]["scriptpubkey"][2:])),
        ("script-split-byte", lambda raw: raw["vin"][7]["prevout"].update(scriptpubkey="0 014" + raw["vin"][7]["prevout"]["scriptpubkey"][4:])),
        ("script-odd-length", lambda raw: raw["vin"][8]["prevout"].update(scriptpubkey=raw["vin"][8]["prevout"]["scriptpubkey"] + "0")),
        ("script-not-hex", lambda raw: raw["vout"][9].update(scriptpubkey="zz" * 22)),
        ("script-nested-hex", lambda raw: raw["vout"][10].update(scriptPubKey={"hex": raw["vout"][10].pop("scriptpubkey")})),
        ("script-nested-string", lambda raw: raw["vin"][9]["prevout"].update(scriptPubKey=raw["vin"][9]["prevout"].pop("scriptpubkey"))),
        ("script-hex-key", lambda raw: raw["vin"][10]["prevout"].update(script_hex=raw["vin"][10]["prevout"].pop("scriptpubkey"))),
        ("script-pubkey-key-only", lambda raw: raw["vin"][11]["prevout"].update(script_pubkey=raw["vin"][11]["prevout"].pop("scriptpubkey"))),
        ("script-empty", lambda raw: raw["vout"][11].update(scriptpubkey="")),
        ("input-not-a-mapping", lambda raw: raw["vin"].append("junk")),
        ("output-not-a-mapping", lambda raw: raw["vout"].append("junk")),
        ("output-op-return-by-script", lambda raw: raw["vout"].append({"value": 0, "scriptpubkey": "6a00"})),
        ("output-op-return-script-pubkey-key", lambda raw: raw["vout"].append({"value": 0, "script_pubkey": "6a00"})),
        ("output-op-return-spaced", lambda raw: raw["vout"].append({"value": 0, "scriptpubkey": "6a  00"})),
        ("output-zero-values", lambda raw: [entry.update(value=0) for entry in raw["vout"][:4]]),
        ("output-five-zero-values", lambda raw: [entry.update(value=0) for entry in raw["vout"][:5]]),
        ("input-negative-value", lambda raw: raw["vin"][4]["prevout"].update(value=-5)),
        ("value-bool", lambda raw: raw["vin"][4]["prevout"].update(value=True)),
    ):
        raw = copy.deepcopy(base)
        change(raw)
        add(name, raw)
    add("nested-tx", {"tx": copy.deepcopy(base)})
    add("gap-with-btc-strings", dict(gap, vout=[dict(entry, value=f"{entry['value'] / 100_000_000:.8f}") for entry in gap["vout"]]))
    add("equal-values-as-sats-strings", tx(900, distinct(6, 9_000), [], output_scripts=[]) | {"vout": [{"n": index, "value_sats": "500000", "scriptpubkey": script("b", index, 900)} for index in range(14)]})
    return rows


def evaluate(raw):
    evidence = reference.collaborative_transaction_evidence({}, raw)
    row = {"chain": "bitcoin", "network": "main", "asset": "BTC", "raw_json": json.dumps(raw),
           "external_id": None, "privacy_boundary": None, "review_status": None, "fee": None}
    parsed = reference._parse_local_transaction(row, raw, {"outpoints": {}, "scripts": {}})
    signal = reference._coinjoin_signal(row, raw, parsed) if parsed["support"] != "none" else None
    return {"collaboration": evidence, "hygiene": signal}


def cases():
    rows = [{"name": vector["name"], "raw": vector["raw"]} for vector in shapes()] + spellings()
    for row in rows:
        row.update(evaluate(row["raw"]))
    return rows


def main():
    rows = cases()
    sources = {row["collaboration"]["source"] for row in rows if row["collaboration"]}
    assert sources == {"equal_output_coinjoin", "large_equal_output_coinjoin", "distinct_value_coinjoin"}, sources
    encoded = "[\n" + ",\n".join(json.dumps(row, ensure_ascii=True, separators=(",", ":")) for row in rows) + "\n]\n"
    if "--write" in sys.argv:
        DEST.write_text(encoded)
    elif DEST.read_text() != encoded:
        raise SystemExit("coinjoin shape vectors drifted")
    print(f"{len(rows)} coinjoin shape vectors, {sum(row['collaboration'] is not None for row in rows)} marked")


if __name__ == "__main__":
    main()
