"""Shared parsing for stored Bitcoin/Liquid transaction graph shapes.

The ownership, identify, and source-of-funds consumers need different views of
the same local evidence. This module owns the wire-shape compatibility while
callers retain their policy: identification accepts script-only outputs,
ownership derivation can merge partial locally-valued Liquid observations, and
lineage only needs vin outpoints. No network access, secret storage, or
ownership judgment belongs here.
"""

from __future__ import annotations

import json
from decimal import Decimal, InvalidOperation
from typing import Any, Callable, Mapping, Sequence

from ..msat import SATS_PER_BTC

COINBASE_TXID = "0" * 64

# A caller that has local descriptor/blinding context may inject additional
# valued legs for an otherwise confidential Liquid transaction.  The parser
# deliberately does not know how secrets are stored.  Sync normally persists
# the same non-secret result in ``vin[].prevout`` / ``vout[]`` so journal replay
# does not need the resolver (or a network connection).
OnchainValueResolver = Callable[
    [Mapping[str, Any], Mapping[str, Any]], Mapping[str, Any] | None
]


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
    if (
        len(txid) != 64
        or any(char not in "0123456789abcdef" for char in txid)
        or txid == COINBASE_TXID
    ):
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


def input_value_sats(entry: Mapping[str, Any]) -> int | None:
    """Return a stored prevout value, when the importer retained it."""

    prevout = entry.get("prevout")
    if isinstance(prevout, Mapping):
        value = output_value_sats(prevout)
        if value is not None:
            return value
    return output_value_sats(entry)


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


def _asset_fields(entry: Mapping[str, Any]) -> tuple[str | None, str | None]:
    """Return ``(asset_id, asset_code)`` without conflating the two.

    Liquid's consensus asset id is the conservation key.  Human-facing codes
    (``LBTC`` and user-defined aliases) are useful fallbacks, but must never
    make two different 32-byte assets look fungible.
    """

    raw_id = entry.get("asset_id")
    raw_asset = entry.get("asset")
    asset_id = str(raw_id).strip().lower() if raw_id not in (None, "") else None
    asset = str(raw_asset).strip().upper() if raw_asset not in (None, "") else None
    if asset_id is None and asset and len(asset) == 64:
        try:
            bytes.fromhex(asset)
        except ValueError:
            pass
        else:
            asset_id, asset = asset.lower(), None
    return asset_id, asset


def _mapping_tx(
    payload: Mapping[str, Any],
    *,
    chain: str | None = None,
    network: str | None = None,
) -> dict[str, Any] | None:
    """Parse one public/stored tx mapping, retaining partially valued legs."""

    vin = payload.get("vin")
    vout = payload.get("vout")
    if not isinstance(vin, list) or not isinstance(vout, list):
        return None
    parsed_chain = str(payload.get("chain") or chain or "").strip().lower()
    parsed_network = str(payload.get("network") or network or "").strip().lower()
    inputs: list[dict[str, Any]] = []
    for entry in vin:
        if not isinstance(entry, Mapping):
            continue
        outpoint = input_outpoint(entry)
        asset_id, asset = _asset_fields(
            entry.get("prevout")
            if isinstance(entry.get("prevout"), Mapping)
            else entry
        )
        inputs.append(
            {
                "outpoint": f"{outpoint[0]}:{outpoint[1]}" if outpoint else None,
                "script": input_script(entry),
                "value_sats": input_value_sats(entry),
                "asset_id": asset_id,
                "asset": asset,
            }
        )
    outputs: list[dict[str, Any]] = []
    for position, entry in enumerate(vout):
        if not isinstance(entry, Mapping):
            continue
        try:
            output_index = int(entry.get("n", position))
        except (TypeError, ValueError):
            output_index = position
        asset_id, asset = _asset_fields(entry)
        outputs.append(
            {
                "n": output_index,
                "script": output_script(entry),
                "value_sats": output_value_sats(entry),
                "asset_id": asset_id,
                "asset": asset,
                "role": str(entry.get("role") or "") or None,
            }
        )
    component = payload.get("component")
    return {
        "txid": payload.get("txid"),
        "chain": parsed_chain,
        "network": parsed_network,
        "inputs": inputs,
        "outputs": outputs,
        "component": dict(component) if isinstance(component, Mapping) else {},
        "evidence_conflicts": [],
    }


def _liquid_txid(value: Any) -> str | None:
    if isinstance(value, str):
        return value.lower()
    if isinstance(value, (bytes, bytearray)):
        return bytes(value).hex()
    return None


def _liquid_asset_id(value: Any) -> str | None:
    if not isinstance(value, (bytes, bytearray)) or len(value) != 32:
        return None
    # Elements serializes the asset bytes in internal byte order.  Keep this in
    # lockstep with sync_backends.liquid_asset_id_from_bytes.
    return bytes(reversed(bytes(value))).hex()


def _decode_liquid_raw_hex(raw_hex: Any) -> dict[str, Any] | None:
    """Decode public Liquid structure locally; confidential values stay None."""

    if not isinstance(raw_hex, str) or not raw_hex.strip():
        return None
    try:
        # Lazy import keeps the shared parser dependency-light and avoids
        # importing embit for ordinary Bitcoin/CSV journal runs.
        from ..wallet_descriptors import decode_liquid_transaction

        tx = decode_liquid_transaction(raw_hex.strip())
    except Exception:
        return None
    inputs: list[dict[str, Any]] = []
    for vin in getattr(tx, "vin", ()):
        txid = _liquid_txid(getattr(vin, "txid", None))
        vout = getattr(vin, "vout", None)
        outpoint = None
        if txid and isinstance(vout, int) and vout >= 0:
            outpoint = f"{txid}:{vout}"
        inputs.append(
            {
                "outpoint": outpoint,
                "script": None,
                "value_sats": None,
                "asset_id": None,
                "asset": None,
            }
        )
    outputs: list[dict[str, Any]] = []
    for position, output in enumerate(getattr(tx, "vout", ())):
        script_pubkey = getattr(output, "script_pubkey", None)
        script_data = getattr(script_pubkey, "data", None)
        script = script_data.hex() if hasattr(script_data, "hex") else None
        value_sats = None
        asset_id = None
        if not getattr(output, "is_blinded", False):
            explicit_value = getattr(output, "value", None)
            if isinstance(explicit_value, int) and not isinstance(explicit_value, bool):
                value_sats = explicit_value
            asset_id = _liquid_asset_id(getattr(output, "asset", None))
        outputs.append(
            {
                "n": position,
                "script": script,
                "value_sats": value_sats,
                "asset_id": asset_id,
                "asset": None,
                "role": "fee" if script == "" else None,
            }
        )
    return {
        "txid": None,
        "chain": "liquid",
        "network": "",
        "inputs": inputs,
        "outputs": outputs,
        "component": {},
        "evidence_conflicts": [],
    }


def _merge_leg(
    target: dict[str, Any], incoming: Mapping[str, Any], *, label: str, conflicts: list[str]
) -> None:
    for field in ("script", "value_sats", "asset_id", "asset"):
        current = target.get(field)
        value = incoming.get(field)
        if current in (None, "") and value not in (None, ""):
            target[field] = value
        elif value not in (None, "") and current != value:
            conflicts.append(f"{label}.{field}")
    current_role = target.get("role")
    incoming_role = incoming.get("role")
    if current_role in (None, "") and incoming_role not in (None, ""):
        target["role"] = incoming_role
    elif incoming_role not in (None, "") and current_role != incoming_role:
        # Ownership is observation-relative: the source wallet legitimately
        # calls another owned wallet's output ``external`` until the latter
        # contributes its unblinded observation. ``owned`` refines ``external``;
        # it is not contradictory chain evidence. Fee/non-fee role differences
        # remain blockers because an empty-script fee output is objective.
        if {str(current_role), str(incoming_role)} == {"external", "owned"}:
            target["role"] = "owned"
        else:
            conflicts.append(f"{label}.role")


def merge_ownership_txs(items: Sequence[Mapping[str, Any]]) -> dict[str, Any] | None:
    """Merge complementary local observations of one transaction.

    A Liquid source wallet cannot unblind a destination wallet's confidential
    output.  Each wallet can, however, persist the legs it owns.  Merging those
    observations by the caller's already-scoped ``(chain, network, txid)`` key
    reconstructs the profile-owned graph without sharing blinding keys.
    Conflicting evidence is retained as an explicit blocker.
    """

    if not items:
        return None
    first = items[0]
    merged: dict[str, Any] = {
        "txid": first.get("txid"),
        "chain": str(first.get("chain") or ""),
        "network": str(first.get("network") or ""),
        "inputs": [],
        "outputs": [],
        "component": dict(first.get("component") or {}),
        "evidence_conflicts": list(first.get("evidence_conflicts") or ()),
    }
    inputs_by_key: dict[str, dict[str, Any]] = {}
    input_key_by_position: dict[int, str] = {}
    outputs_by_n: dict[int, dict[str, Any]] = {}
    for item in items:
        for field in ("txid", "chain", "network"):
            current = merged.get(field)
            value = item.get(field)
            if current in (None, "") and value not in (None, ""):
                merged[field] = value
            elif value not in (None, "") and current != value:
                merged["evidence_conflicts"].append(field)
        for position, incoming in enumerate(item.get("inputs") or ()):
            if not isinstance(incoming, Mapping):
                continue
            outpoint = str(incoming.get("outpoint") or "").strip()
            positional_key = input_key_by_position.get(position)
            key = outpoint or positional_key or f"position:{position}"
            if outpoint and positional_key and positional_key != outpoint:
                positional = inputs_by_key.pop(positional_key, None)
                if positional is not None:
                    if outpoint not in inputs_by_key:
                        inputs_by_key[outpoint] = positional
                    else:
                        _merge_leg(
                            inputs_by_key[outpoint],
                            positional,
                            label=f"input:{outpoint}",
                            conflicts=merged["evidence_conflicts"],
                        )
                key = outpoint
            input_key_by_position[position] = key
            if key not in inputs_by_key:
                inputs_by_key[key] = dict(incoming)
            else:
                _merge_leg(
                    inputs_by_key[key],
                    incoming,
                    label=f"input:{key}",
                    conflicts=merged["evidence_conflicts"],
                )
            if outpoint:
                inputs_by_key[key]["outpoint"] = outpoint
        for position, incoming in enumerate(item.get("outputs") or ()):
            if not isinstance(incoming, Mapping):
                continue
            try:
                n = int(incoming.get("n", position))
            except (TypeError, ValueError):
                n = position
            if n not in outputs_by_n:
                outputs_by_n[n] = dict(incoming)
                outputs_by_n[n]["n"] = n
            else:
                _merge_leg(
                    outputs_by_n[n],
                    incoming,
                    label=f"output:{n}",
                    conflicts=merged["evidence_conflicts"],
                )
    merged["inputs"] = [inputs_by_key[key] for key in sorted(inputs_by_key)]
    merged["outputs"] = [outputs_by_n[n] for n in sorted(outputs_by_n)]
    merged["evidence_conflicts"] = sorted(set(merged["evidence_conflicts"]))
    chain = str(merged.get("chain") or "").lower()
    if chain == "bitcoin":
        for leg in [*merged["inputs"], *merged["outputs"]]:
            leg["asset_id"] = leg.get("asset_id") or "BTC"
            leg["asset"] = leg.get("asset") or "BTC"
    return merged


def parse_ownership_tx(
    raw_json: Any,
    *,
    value_resolver: OnchainValueResolver | None = None,
) -> dict[str, Any] | None:
    """Parse all available local evidence, allowing confidential unknown legs.

    Unlike :func:`parse_valued_tx`, this is intentionally partial: an external
    confidential output may have a public script but no locally known value.
    Ownership callers can still derive known owned legs and use the source row's
    conserved net outflow for an aggregate external residual.  If an *owned*
    leg lacks value/asset identity, they must block rather than guess.
    """

    outer = stored_tx_mapping(raw_json)
    if outer is None:
        return None
    chain = str(outer.get("chain") or "").strip().lower()
    network = str(outer.get("network") or "").strip().lower()
    component = outer.get("component")
    looks_liquid = chain == "liquid" or (
        isinstance(component, Mapping) and component.get("asset_id")
    )
    observations: list[Mapping[str, Any]] = []
    direct = _mapping_tx(outer, chain=chain, network=network)
    if direct is not None:
        observations.append(direct)
    nested = outer.get("tx")
    if isinstance(nested, Mapping):
        parsed_nested = _mapping_tx(nested, chain=chain, network=network)
        if parsed_nested is not None:
            observations.append(parsed_nested)
    ownership_graph = outer.get("ownership_graph")
    if isinstance(ownership_graph, Mapping):
        parsed_owned = _mapping_tx(ownership_graph, chain=chain, network=network)
        if parsed_owned is not None:
            observations.append(parsed_owned)
    if looks_liquid and outer.get("raw_hex"):
        decoded = _decode_liquid_raw_hex(outer.get("raw_hex"))
        if decoded is not None:
            decoded["network"] = decoded.get("network") or network
            observations.append(decoded)
    if not observations:
        return None
    parsed = merge_ownership_txs(observations)
    if parsed is None:
        return None
    parsed["txid"] = outer.get("txid") or parsed.get("txid")
    parsed["chain"] = chain or parsed.get("chain") or ("liquid" if looks_liquid else "bitcoin")
    parsed["network"] = network or parsed.get("network") or ""
    parsed["component"] = dict(component) if isinstance(component, Mapping) else {}
    if value_resolver is not None:
        resolved = value_resolver(outer, parsed)
        if isinstance(resolved, Mapping):
            parsed = merge_ownership_txs([parsed, resolved]) or parsed
            parsed["component"] = dict(component) if isinstance(component, Mapping) else {}
    if parsed.get("chain") == "bitcoin":
        for leg in [*parsed["inputs"], *parsed["outputs"]]:
            leg["asset_id"] = leg.get("asset_id") or "BTC"
            leg["asset"] = leg.get("asset") or "BTC"
    return parsed


def parse_vin_outpoints(raw_json: Any) -> list[tuple[str, int]]:
    parsed = parse_ownership_tx(raw_json)
    if parsed is None:
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
    result: list[tuple[str, int]] = []
    for entry in parsed["inputs"]:
        outpoint = str(entry.get("outpoint") or "")
        if not outpoint or ":" not in outpoint:
            continue
        txid, vout = outpoint.rsplit(":", 1)
        try:
            result.append((txid, int(vout)))
        except ValueError:
            continue
    return result


def parse_identification_legs(
    raw_json: Any,
    *,
    chain: str = "bitcoin",
    network: str = "",
) -> dict[str, Any] | None:
    parsed = parse_ownership_tx(raw_json)
    if parsed is None:
        return None
    inputs = [
        {"outpoint": entry.get("outpoint"), "script": entry.get("script")}
        for entry in parsed["inputs"]
    ]
    outputs = [
        {"n": entry.get("n", position), "script": entry.get("script")}
        for position, entry in enumerate(parsed["outputs"])
    ]
    return {
        "inputs": inputs,
        "outputs": outputs,
        "chain": parsed.get("chain") or chain or "bitcoin",
        "network": parsed.get("network") or network,
        "source": "local_tx",
    }


def parse_valued_tx(
    raw_json: Any,
    *,
    value_resolver: OnchainValueResolver | None = None,
) -> dict[str, Any] | None:
    parsed = parse_ownership_tx(raw_json, value_resolver=value_resolver)
    if parsed is None:
        return None
    for entry in parsed["outputs"]:
        if entry.get("value_sats") is None:
            return None
    return parsed


def exact_onchain_fee_msat(raw_json: Any, *, asset: str | None = None) -> int | None:
    """Return an exactly evidenced network fee, otherwise ``None``.

    Bitcoin's fee is derivable only when every input prevout value and every
    output value is present in the stored transaction graph.  A wallet-row
    amount delta is deliberately *not* evidence: it can also contain a payment,
    swap lockup, or an owned output from a wallet that was never imported.

    Liquid exposes its fee as an explicit empty-script output.  Accept that
    output only when its asset can be tied to the row's component; confidential
    or asset-ambiguous observations stay unresolved.  The function performs no
    ownership inference and never contacts a backend.
    """

    parsed = parse_ownership_tx(raw_json)
    return exact_onchain_fee_msat_from_parsed(parsed, asset=asset)


def exact_onchain_fee_msat_from_observations(
    raw_json_values: Sequence[Any], *, asset: str | None = None
) -> int | None:
    """Merge complementary stored observations, then derive an exact fee."""

    observations = [
        parsed
        for raw_json in raw_json_values
        if (parsed := parse_ownership_tx(raw_json)) is not None
    ]
    parsed = merge_ownership_txs(observations)
    return exact_onchain_fee_msat_from_parsed(parsed, asset=asset)


def exact_onchain_fee_msat_from_parsed(
    parsed: Mapping[str, Any] | None, *, asset: str | None = None
) -> int | None:
    """Derive an exact fee from one already-scoped, possibly merged graph."""

    if parsed is None or parsed.get("evidence_conflicts"):
        return None
    chain = str(parsed.get("chain") or "").strip().lower()
    if chain == "bitcoin":
        inputs = list(parsed.get("inputs") or ())
        outputs = list(parsed.get("outputs") or ())
        if not inputs or not outputs:
            return None
        if any(entry.get("value_sats") is None for entry in (*inputs, *outputs)):
            return None
        fee_sats = sum(int(entry["value_sats"]) for entry in inputs) - sum(
            int(entry["value_sats"]) for entry in outputs
        )
        return fee_sats * 1000 if fee_sats >= 0 else None

    if chain != "liquid":
        return None
    component = parsed.get("component")
    component = component if isinstance(component, Mapping) else {}
    component_asset_id = str(component.get("asset_id") or "").strip().lower()
    component_asset = str(component.get("asset") or asset or "").strip().upper()
    fee_outputs = [
        entry
        for entry in parsed.get("outputs") or ()
        if entry.get("role") == "fee"
    ]
    if not fee_outputs or any(entry.get("value_sats") is None for entry in fee_outputs):
        return None
    matching: list[Mapping[str, Any]] = []
    for entry in fee_outputs:
        entry_asset_id = str(entry.get("asset_id") or "").strip().lower()
        entry_asset = str(entry.get("asset") or "").strip().upper()
        if component_asset_id and entry_asset_id:
            if component_asset_id == entry_asset_id:
                matching.append(entry)
            continue
        if component_asset and entry_asset and component_asset == entry_asset:
            matching.append(entry)
    if len(matching) != 1:
        return None
    fee_sats = int(matching[0]["value_sats"])
    return fee_sats * 1000 if fee_sats >= 0 else None
