"""Shared parsing for stored Bitcoin/Liquid transaction graph shapes.

The ownership, identify, and source-of-funds consumers need different views of
the same local evidence. This module owns the wire-shape compatibility while
callers retain their policy: identification accepts script-only outputs,
ownership derivation requires valued outputs, and lineage only needs vin
outpoints. No network access or ownership judgment belongs here.
"""

from __future__ import annotations

import json
from decimal import Decimal, InvalidOperation
from typing import Any, Mapping

from ..msat import SATS_PER_BTC

COINBASE_TXID = "0" * 64


def stored_tx_mapping(raw_json: Any, *, allow_nested: bool = False) -> Mapping[str, Any] | None:
    if isinstance(raw_json, Mapping):
        raw = raw_json
    else:
        try:
            raw = json.loads(raw_json or "{}")
        except (TypeError, ValueError, json.JSONDecodeError):
            return None
    if not isinstance(raw, Mapping):
        return None
    if allow_nested and isinstance(raw.get("tx"), Mapping):
        return raw["tx"]
    return raw


def input_outpoint(entry: Mapping[str, Any]) -> tuple[str, int] | None:
    txid = str(entry.get("txid") or "").strip().lower()
    if not txid or txid == COINBASE_TXID:
        return None
    try:
        vout = int(entry.get("vout"))
    except (TypeError, ValueError):
        return None
    if vout < 0:
        return None
    return txid, vout


def input_script(entry: Mapping[str, Any]) -> Any:
    prevout = entry.get("prevout")
    return prevout.get("scriptpubkey") if isinstance(prevout, Mapping) else None


def output_script(entry: Mapping[str, Any]) -> Any:
    script = entry.get("scriptpubkey")
    return entry.get("script_hex") if script is None else script


def output_value_sats(entry: Mapping[str, Any]) -> int | None:
    explicit_sats = entry.get("value_sats") is not None
    value = entry.get("value_sats") if explicit_sats else entry.get("value")
    if value is None or value == "" or isinstance(value, bool):
        return None
    try:
        if explicit_sats or isinstance(value, int):
            return int(value)
        if isinstance(value, str) and "." not in value:
            return int(value)
        return int((Decimal(str(value)) * SATS_PER_BTC).to_integral_value())
    except (TypeError, ValueError, ArithmeticError, InvalidOperation):
        return None


def parse_vin_outpoints(raw_json: Any) -> list[tuple[str, int]]:
    payload = stored_tx_mapping(raw_json, allow_nested=True)
    vin = payload.get("vin") if payload is not None else None
    if not isinstance(vin, list):
        return []
    return [
        outpoint
        for entry in vin
        if isinstance(entry, Mapping)
        and (outpoint := input_outpoint(entry)) is not None
    ]


def parse_identification_legs(
    raw_json: Any,
    *,
    chain: str = "bitcoin",
    network: str = "",
) -> dict[str, Any] | None:
    payload = stored_tx_mapping(raw_json)
    if payload is None:
        return None
    vin = payload.get("vin")
    vout = payload.get("vout")
    if not isinstance(vin, list) or not isinstance(vout, list):
        return None
    inputs = []
    for entry in vin:
        if not isinstance(entry, Mapping):
            continue
        outpoint = input_outpoint(entry)
        inputs.append(
            {
                "outpoint": f"{outpoint[0]}:{outpoint[1]}" if outpoint else None,
                "script": input_script(entry),
            }
        )
    outputs = []
    for position, entry in enumerate(vout):
        if not isinstance(entry, Mapping):
            continue
        outputs.append(
            {
                "n": entry.get("n", position),
                "script": output_script(entry),
            }
        )
    return {
        "inputs": inputs,
        "outputs": outputs,
        "chain": chain or "bitcoin",
        "network": network,
        "source": "local_tx",
    }


def parse_valued_tx(raw_json: Any) -> dict[str, Any] | None:
    payload = stored_tx_mapping(raw_json)
    if payload is None:
        return None
    vin = payload.get("vin")
    vout = payload.get("vout")
    if not isinstance(vin, list) or not isinstance(vout, list):
        return None
    inputs: list[dict[str, Any]] = []
    for entry in vin:
        if not isinstance(entry, Mapping):
            return None
        outpoint = input_outpoint(entry)
        inputs.append(
            {
                "outpoint": f"{outpoint[0]}:{outpoint[1]}" if outpoint else None,
                "script": input_script(entry),
            }
        )
    outputs: list[dict[str, Any]] = []
    for position, entry in enumerate(vout):
        if not isinstance(entry, Mapping):
            return None
        value_sats = output_value_sats(entry)
        if value_sats is None:
            return None
        try:
            output_index = int(entry.get("n", position))
        except (TypeError, ValueError):
            output_index = position
        outputs.append(
            {
                "n": output_index,
                "script": output_script(entry),
                "value_sats": value_sats,
            }
        )
    return {"txid": payload.get("txid"), "inputs": inputs, "outputs": outputs}
