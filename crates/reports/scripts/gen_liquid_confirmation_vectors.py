#!/usr/bin/env python3
"""Pin Liquid parse-success and conflict-only identities against current Python.

Synthetic public bytes only. This corpus records the reference parser's
leniency as well as ordinary explicit/confidential/issuance/witness shapes.
"""
import json
from pathlib import Path
import sys

from kassiber.core.onchain import _decode_liquid_raw_hex
from kassiber.core.ownership_transfers import _physical_conflict_scope, _parse_onchain_tx

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / "crates/reports/tests/data/liquid_confirmations.json"


def raw_transactions():
    vin = bytes.fromhex("11" * 32 + "00000000" + "00" + "ffffffff")
    asset = bytes.fromhex("01" + "22" * 32)
    explicit = asset + bytes.fromhex("01" + "0000000000000064" + "00" + "0151")
    confidential = bytes.fromhex("0a" + "22" * 32 + "08" + "33" * 32 + "02" + "44" * 32 + "0151")
    issuance = vin[:32] + bytes.fromhex("00000080") + vin[36:] + bytes(64) + bytes.fromhex("01000000000000000a00")
    commitment = vin[:32] + bytes.fromhex("00000080") + vin[36:] + bytes(64) + bytes.fromhex("09" + "33" * 32 + "01" + "00" * 8)
    return {
        "empty": bytes.fromhex("02000000000000"),
        "explicit": bytes.fromhex("020000000001") + vin + b"\x01" + explicit + bytes(4),
        "confidential": bytes.fromhex("020000000001") + vin + b"\x01" + confidential + bytes(4),
        "issuance": bytes.fromhex("020000000001") + issuance + b"\x01" + explicit + bytes(4),
        "commitment": bytes.fromhex("020000000001") + commitment + b"\x01" + explicit + bytes(4),
        "coinbase": bytes.fromhex("020000000001") + vin[:32] + bytes.fromhex("ffffffff") + vin[36:] + b"\x01" + explicit + bytes(4),
        "pegin": bytes.fromhex("020000000001") + vin[:32] + bytes.fromhex("00000040") + vin[36:] + b"\x01" + explicit + bytes(4),
        "witness": bytes.fromhex("020000000101") + vin + b"\x01" + explicit + bytes(4) + bytes.fromhex("016101620102aabb0101cc01640165"),
        "short-proof": bytes.fromhex("02000000010001") + explicit + bytes(4) + bytes.fromhex("0009aabb"),
        "proof-max-length": bytes.fromhex("02000000010001") + explicit + bytes(4) + bytes.fromhex("00ff" + "ff" * 7 + "7f"),
        "proof-overflow-length": bytes.fromhex("02000000010001") + explicit + bytes(4) + bytes.fromhex("00ff" + "ff" * 8),
        "proof-truncated-compact": bytes.fromhex("02000000010001") + explicit + bytes(4) + bytes.fromhex("00ff"),
        "noncanonical-counts": bytes.fromhex("0200000000fd0000fe00000000"),
        "truncated-compact": bytes.fromhex("020000000000ff"),
    }


def build():
    inputs = []
    for name, raw in raw_transactions().items():
        inputs.append((name, raw.hex()))
        inputs.extend((f"{name}-prefix-{i}", raw[:i].hex()) for i in range(len(raw)))
        inputs.extend((f"{name}-suffix-{i}", (raw + bytes(i)).hex()) for i in range(1, 4))
        for index in range(len(raw)):
            for byte in (0, 1, 0xfd, 0xff):
                edited = raw[:index] + bytes([byte]) + raw[index + 1:]
                if edited != raw:
                    inputs.append((f"{name}-edit-{index}-{byte}", edited.hex()))
    inputs.extend((f"text-{i}", text) for i, text in enumerate((
        "", "0200", "zz", "0", "0x02000000000000", "02 000000000000", "02000000000000\n", "\u200302000000000000\u2003",
        "０２００００００００００００", "02000000000000é", "02000000000000".upper(),
    )))
    result = []
    for name, raw_hex in inputs:
        raw = dict(chain="liquid", network="liquidv1", raw_hex=raw_hex)
        row = dict(id="hex", asset="LBTC", external_id="b" * 64, config_json="{}", raw_json=raw)
        scope = _physical_conflict_scope(row, _parse_onchain_tx(raw, allow_partial=True))
        result.append(dict(name=name, row=row, decodable=_decode_liquid_raw_hex(raw_hex) is not None,
                           scope=list(scope[:3]) if scope else None))
    # Stored graph precedence and identity/domain ambiguity are independent of
    # decode success. A raw txid is never substituted with a hash of the bytes.
    raw_hex = raw_transactions()["explicit"].hex()
    for name, edits in (
        ("external-conflict", dict(raw_json=dict(txid="a" * 64))),
        ("invalid-txid", dict(raw_json=dict(txid="not-a-txid"))),
        ("raw-txid", dict(external_id="", raw_json=dict(txid="a" * 64))),
        ("missing-txid", dict(external_id="")),
        ("missing-network", dict(raw_json=dict(network=""))),
        ("network-conflict", dict(config_json='{"chain":"liquid","network":"liquidtestnet"}')),
        ("issued-asset", dict(asset="USD")),
        ("wrong-family", dict(asset="BTC")),
        ("graph-invalid-hex", dict(raw_json=dict(raw_hex="0200", vin=[], vout=[]))),
        ("nested-graph-invalid-hex", dict(raw_json=dict(raw_hex="0200", tx=dict(vin=[], vout=[], txid="b" * 64)))),
        ("non-string-hex", dict(raw_json=dict(raw_hex=True))),
    ):
        raw = dict(chain="liquid", network="liquidv1", raw_hex=raw_hex)
        raw.update(edits.pop("raw_json", {}))
        row = dict(id="hex", asset="LBTC", external_id="b" * 64, config_json="{}", raw_json=raw)
        row.update(edits)
        scope = _physical_conflict_scope(row, _parse_onchain_tx(raw, allow_partial=True))
        result.append(dict(name=name, row=row, scope=list(scope[:3]) if scope else None))
    return json.dumps(result, ensure_ascii=True, separators=(",", ":")) + "\n"


if __name__ == "__main__":
    content = build()
    if "--check" in sys.argv:
        assert OUT.read_text() == content, "Liquid confirmation vectors drifted"
    else:
        OUT.write_text(content)
