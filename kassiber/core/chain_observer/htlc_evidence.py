"""Non-secret HTLC facts bound to the existing closed observer authority.

LWK and Core RPC consume witnesses while decoding without adding preimages
to their normalized graphs. A recognized single-input spend can instead retain
its role and exact outpoint;
the observation's graph and quantity commitments bind those facts to the row.
An imported payload with identical fields has no authority on its own.
"""

from __future__ import annotations

import json
from typing import Any, Mapping

from ...transfers import canonical_txid
from ..onchain import stored_tx_mapping
from .provenance import row_has_current_authoritative_observation


def htlc_spend_attestation(*, claim, refund, input_count: int) -> dict[str, Any] | None:
    """Curate already witness-verified adapter results without secret material."""

    if input_count != 1 or bool(claim) == bool(refund):
        return None
    payment_hash, txid, vout = claim if claim else (None, *refund)
    txid = canonical_txid(txid)
    if txid is None or type(vout) is not int or vout < 0:
        return None
    result = {
        "version": 1,
        "role": "claim" if claim else "refund",
        "funding_txid": txid,
        "funding_vout": vout,
        "input_count": 1,
    }
    if payment_hash is not None:
        result["payment_hash"] = payment_hash
    return result


def authoritative_htlc_spend(row: Mapping[str, Any], *, role: str) -> Mapping[str, Any] | None:
    """Read a current adapter attestation agreeing with its committed input."""

    if not row_has_current_authoritative_observation(row):
        return None
    if "observation_observer_kinds_json" not in row.keys():
        return None
    try:
        kinds = json.loads(row["observation_observer_kinds_json"] or "[]")
    except (TypeError, ValueError):
        return None
    if not isinstance(kinds, list):
        return None
    payload = stored_tx_mapping(row["raw_json"])
    if payload is None:
        return None
    observer = payload.get("observer")
    if observer not in {"lwk", "bitcoinrpc"} or observer not in kinds:
        return None
    proof = payload.get("htlc_spend")
    if not isinstance(proof, Mapping) or proof.get("role") != role:
        return None
    if type(proof.get("version")) is not int or proof["version"] != 1:
        return None
    if type(proof.get("input_count")) is not int or proof["input_count"] != 1:
        return None
    vins = payload.get("vin")
    if not isinstance(vins, list) or len(vins) != 1 or not isinstance(vins[0], Mapping):
        return None
    txid = canonical_txid(proof.get("funding_txid"))
    vout = proof.get("funding_vout")
    if txid is None or type(vout) is not int or vout < 0:
        return None
    if canonical_txid(vins[0].get("txid")) != txid:
        return None
    if type(vins[0].get("vout")) is not int or vins[0]["vout"] != vout:
        return None
    return proof
