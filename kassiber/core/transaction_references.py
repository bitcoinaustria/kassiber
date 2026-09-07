"""Bounded, profile-scoped reuse of local transaction references; never egresses.

Physical transaction facts do not carry the source wallet's ownership or custody
authority. Consumers must apply their own annotations after reading a reference.
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Mapping

from ..transfers import bitcoin_network_domain_evidence
from ..wallet_descriptors import normalize_network
from .custody_evidence import resolve_protocol_scope
from .onchain import input_outpoint, output_script, output_value_sats, stored_tx_mapping

MAX_LOCAL_REFERENCE_ROWS = 256


def reference_scope(row: Mapping[str, Any], *, default_chain: str | None = None,
                    default_network: str | None = None) -> tuple[str, str] | None:
    """Canonical physical chain scope, rejecting contradictory explicit evidence."""
    domain, valid = bitcoin_network_domain_evidence(row)
    if not valid:
        return None
    scoped = dict(row)
    raw = stored_tx_mapping(scoped.get("raw_json")) or {}
    config = stored_tx_mapping(scoped.get("config_json") or scoped.get("wallet_config_json")) or {}
    if default_chain and not (raw.get("chain") or config.get("chain") or scoped.get("chain")):
        scoped["chain"] = default_chain
    try:
        scope = resolve_protocol_scope(scoped)
        if domain is not None:
            network = ({"main": "liquidv1", "test": "liquidtestnet", "regtest": "regtest"}.get(domain)
                       if scope.base_chain == "liquid" else domain)
            if network is None:
                return None
        else:
            network = scope.network if raw.get("network") or config.get("network") or scoped.get("network") else default_network or scope.network
        return scope.base_chain, normalize_network(scope.base_chain, network)
    except ValueError:
        return None


@dataclass(frozen=True)
class LocalTransactionReference:
    payload: Mapping[str, Any] | None = None
    conflict: bool = False


def _facts(payload: Mapping[str, Any]) -> dict[str, Any] | None:
    """Compare known physical facts across the supported wire representations."""
    vin, vout = payload.get("vin"), payload.get("vout")
    if not isinstance(vin, list) or not isinstance(vout, list) or not vin or not vout:
        return None
    facts: dict[str, Any] = {"input_count": len(vin), "output_count": len(vout)}
    for side, entries in (("in", vin), ("out", vout)):
        for index, entry in enumerate(entries):
            if not isinstance(entry, Mapping):
                return None
            if side == "out" and entry.get("n", index) != index:
                return None
            if side == "in":
                outpoint = input_outpoint(entry)
                if outpoint is None and "coinbase" not in entry:
                    return None
                facts[f"in:{index}:outpoint"] = outpoint or "coinbase"
                entry = entry.get("prevout") or {}
                if not isinstance(entry, Mapping):
                    return None
            for name, value in (
                ("script", output_script(entry)),
                ("value", output_value_sats(entry)),
                ("asset", entry.get("asset")),
                ("valuecommitment", entry.get("valuecommitment")),
                ("assetcommitment", entry.get("assetcommitment")),
            ):
                if value is not None:
                    facts[f"{side}:{index}:{name}"] = value
    return facts


def load_local_transaction_reference(conn, *, profile_id: str, chain: str,
                                     network: str, txid: str,
                                     current: Mapping[str, Any]) -> LocalTransactionReference:
    """Return a compatible local graph, refusing conflicting or truncated sources."""
    if chain not in {"bitcoin", "liquid"}:
        return LocalTransactionReference()
    rows = conn.execute(
        """SELECT t.raw_json, t.external_id, t.asset, w.kind AS wallet_kind,
                  w.config_json AS wallet_config_json
           FROM transactions t JOIN wallets w ON w.id = t.wallet_id
           WHERE t.profile_id = ? AND t.excluded = 0
             AND (lower(t.external_id) = ? OR
                  lower(CASE WHEN json_valid(t.raw_json) THEN json_extract(t.raw_json, '$.txid') END) = ?)
           ORDER BY t.id LIMIT ?""",
        (profile_id, txid, txid, MAX_LOCAL_REFERENCE_ROWS + 1),
    ).fetchall()
    if len(rows) > MAX_LOCAL_REFERENCE_ROWS:
        return LocalTransactionReference(conflict=True)
    candidates = []
    for row in rows:
        row = dict(row)
        payload = stored_tx_mapping(row.get("raw_json"), allow_nested=True)
        if payload is not None:
            candidates.append((row, payload, {}))
    observation = conn.execute(
        """SELECT payload_json, status_json FROM chain_analysis_observations
           WHERE profile_id = ? AND chain = ? AND network = ? AND txid = ?""",
        (profile_id, chain, network, txid),
    ).fetchone()
    if observation is not None:
        payload = stored_tx_mapping(observation["payload_json"])
        if payload is not None:
            candidates.append(({"chain": chain, "network": network, "raw_json": payload},
                               payload, stored_tx_mapping(observation["status_json"]) or {}))
    current_facts = _facts(current) or {}
    combined = dict(current_facts)
    best, best_size = None, len(combined)
    for row, payload, status in candidates:
        raw_id = str(payload.get("txid") or txid).lower()
        external = str(row.get("external_id") or txid).lower()
        if raw_id != txid or (len(external) == 64 and external != txid):
            continue
        scope = reference_scope(row)
        payload_scope = reference_scope({"chain": chain, "network": network, "raw_json": payload})
        if scope != payload_scope or scope != (chain, network):
            continue
        if status.get("removed") is True or status.get("conflicted") is True or payload.get("removed") is True:
            continue
        confirmations = payload.get("confirmations")
        if isinstance(confirmations, (int, float)) and confirmations < 0:
            continue
        facts = _facts(payload)
        if facts is None:
            continue
        if any(key in combined and combined[key] != value for key, value in facts.items()):
            return LocalTransactionReference(conflict=True)
        combined.update(facts)
        if len(facts) > best_size and current_facts.keys() <= facts.keys():
            best, best_size = payload, len(facts)
    return LocalTransactionReference(payload=best)
