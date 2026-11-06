"""Check an imported transaction against its locally stored chain graph.

Conservation could otherwise only be checked against imported rows: an
importer that dropped an output, or booked a wallet's side of a transaction
with the wrong amount, would leave nothing to compare with. This module reads
the raw graph Kassiber already stored for the transaction (its own observation,
or a transaction reference stored earlier) and compares three things:

- conservation: inputs minus outputs equals the recorded network fee;
- legs: every wallet of this book that owns an input or output of the graph
  has an imported row for the txid whose wallet delta matches what it owns;
- the reverse: every row of the txid belongs to a wallet that owns a leg.

It never contacts the network, never writes, and never returns addresses,
scripts, descriptors or derivation paths. Ownership comes from the book's
local ownership index, which cannot prove that an unmatched output belongs to
someone else; such an output is ``owned: false`` (not matched), not external.
"""

from __future__ import annotations

import sqlite3
from typing import Any, Mapping, Sequence

from ..errors import AppError
from .chain_analysis.ownership import branch_evidence
from .custody_evidence import row_boundary_amounts
from .ownership_transfers import _norm_chain_network
from . import transaction_graph as graph_core

SATS_TO_MSAT = 1000
MAX_NODES_PER_SIDE = 100
MAX_ROWS = 50

VERIFY_DEFINITIONS = {
    "status": "ok: every check passed; mismatch: at least one check failed; partial: no failure but some check could not run; unverifiable: no stored graph to check against.",
    "conservation": "Sum of input values minus sum of output values against the fee the importer recorded, in msat; a source that reports no fee had it derived from the same graph, which then only proves the graph is self-consistent.",
    "legs": "Per wallet of this book owning a graph input or output: owned outputs minus owned inputs (expected_delta_msat) against the imported row's wallet delta (inbound amount, or minus amount and fee). missing_imported_leg means no row of that wallet carries this txid.",
    "unexplained_rows": "Rows with this txid whose wallet owns no graph leg; expected for an exchange or custodial record of the same transfer, a mismatch for a wallet this book observes on chain.",
    "keychain": "receive or change from the owning wallet's derivation branch; unknown when the ownership evidence names no branch (address lists, unlabeled inventory).",
}


def _sats(value: Any) -> int | None:
    return int(value) if isinstance(value, int) and not isinstance(value, bool) else None


def _graph_source(enriched: Mapping[str, Any]) -> str:
    """Where the compared graph came from; a reference is not an observation."""

    warning = enriched.get("_graphLookupWarning")
    code = warning.get("code") if isinstance(warning, Mapping) else None
    if code == "local_transaction_reference":
        return "local_reference"
    if code == "cached_reference_graph":
        return "cached_public_reference"
    return "stored_observation"


def _keychain(matches: Sequence[Any]) -> str:
    roles = {
        evidence["branch_role"]
        for evidence in (branch_evidence(match.branch_label, None) for match in matches)
        if evidence["branch_evidence_level"] == "exact"
    }
    return next(iter(roles)) if len(roles) == 1 else "unknown"


def _observed_wallet_ids(owned_index: Any) -> set[str]:
    wallet_ids: set[str] = set()
    for attribute in ("by_script", "by_address", "by_outpoint"):
        for matches in (getattr(owned_index, attribute, None) or {}).values():
            for match in matches if isinstance(matches, list) else [matches]:
                wallet_ids.add(str(match.wallet_id))
    return wallet_ids


def _txid_rows(
    conn: sqlite3.Connection,
    profile_id: str,
    txid: str,
    chain_network: tuple[str, str],
) -> list[Mapping[str, Any]]:
    rows = conn.execute(
        """
        SELECT t.*, w.label AS wallet_label, w.kind AS wallet_kind,
               w.config_json AS wallet_config_json
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        WHERE t.profile_id = ? AND lower(t.external_id) = ?
        ORDER BY t.occurred_at, t.created_at, t.id
        """,
        (profile_id, txid.lower()),
    ).fetchall()
    return [
        row
        for row in rows
        if _norm_chain_network(*graph_core._row_chain_network(row)) == chain_network
    ]


def _node_view(node: Mapping[str, Any], matches: Sequence[Any]) -> dict[str, Any]:
    wallet_ids = sorted({str(match.wallet_id) for match in matches})
    labels = sorted({str(match.wallet_label) for match in matches})
    value_sats = _sats(node.get("valueSats"))
    view: dict[str, Any] = {
        "index": node.get("index"),
        "amount_msat": value_sats * SATS_TO_MSAT if value_sats is not None else None,
        "value_state": node.get("valueState"),
        "owned": len(wallet_ids) == 1,
        "ambiguous": len(wallet_ids) > 1,
        "wallet_id": wallet_ids[0] if len(wallet_ids) == 1 else None,
        "wallet": labels[0] if len(labels) == 1 else (", ".join(labels) or None),
        "keychain": _keychain(matches) if wallet_ids else None,
    }
    return view


def verify_transaction(
    conn: sqlite3.Connection,
    profile_id: str,
    transaction_ref: str,
    *,
    semantics_cache: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Verify one transaction against its stored graph. Local only."""

    ref = str(transaction_ref or "").strip()
    if not ref:
        raise AppError(
            "Transaction verification requires a transaction id or txid",
            code="validation",
            retryable=False,
        )
    row = graph_core._fetch_transaction(conn, profile_id, ref)
    if row is None:
        raise AppError(
            "Transaction was not found in the active book",
            code="not_found",
            details={"transaction": ref},
            retryable=False,
        )
    row_raw = graph_core._json_obj(graph_core._row_get(row, "raw_json"))
    # No runtime config and no public lookup: local observation, stored
    # references and cached references only.
    enriched = graph_core._enrich_graph_raw(
        conn, row, row_raw, None, allow_public_lookup=False
    )
    graph = graph_core._parse_graph(
        row,
        enriched,
        local_outpoint_amounts=graph_core._local_wallet_outpoint_amounts(
            conn, profile_id, row, enriched
        ),
    )
    chain, network = graph_core._row_chain_network(row)
    txid = graph_core._txid_from_row(row)
    payload: dict[str, Any] = {
        "transaction_id": str(row["id"]),
        "txid": txid,
        "chain": chain,
        "network": network,
        "asset": row["asset"],
        "network_contacted": False,
        "definitions": dict(VERIFY_DEFINITIONS),
    }
    if graph["supportLevel"] == "graphless" or not txid:
        return {
            **payload,
            "status": "unverifiable",
            "reasons": [graph.get("unsupportedReason") or "no_txid"],
            "message": (
                "No raw transaction graph is stored for this row, so it can only be "
                "compared with other imported rows. Refreshing the wallet from its "
                "chain source or opening the transaction graph with a reference "
                "lookup stores one."
            ),
            "conservation": None,
            "legs": None,
            "unexplained_rows": None,
            "inputs": [],
            "outputs": [],
        }

    reasons: list[str] = []
    statuses: list[str] = []
    payload["graph_source"] = _graph_source(enriched)

    # Conservation: inputs - outputs = fee.
    inputs = graph["inputs"]
    outputs = graph["outputs"]
    input_values = [_sats(node.get("valueSats")) for node in inputs]
    output_values = [_sats(node.get("valueSats")) for node in outputs]
    explicit_fee = _sats(row_raw.get("fee")) if "vin" in row_raw else None
    fee_source = "observer_record" if explicit_fee is not None else None
    if explicit_fee is None:
        explicit_fee = _sats(enriched.get("fee"))
        fee_source = "reference_graph" if explicit_fee is not None else None
    if explicit_fee is None and isinstance(graph.get("fee"), Mapping) and chain == "liquid":
        explicit_fee = _sats(graph["fee"].get("valueSats"))
        fee_source = "fee_output" if explicit_fee is not None else None
    conservation: dict[str, Any] = {
        "inputs_msat": None,
        "outputs_msat": None,
        "fee_expected_msat": None,
        "fee_recorded_msat": explicit_fee * SATS_TO_MSAT if explicit_fee is not None else None,
        "fee_source": fee_source,
    }
    if any(node.get("_coinbase") or node.get("_pegin") for node in inputs):
        conservation.update(status="not_applicable", reason="newly_issued_or_pegged_in")
    elif not inputs or any(value is None for value in input_values + output_values):
        conservation.update(
            status="unverifiable",
            reason=graph.get("unsupportedReason") or "graph_values_missing",
        )
    else:
        total_in = sum(input_values)
        total_out = sum(output_values)
        expected_fee = total_in - total_out
        conservation.update(
            inputs_msat=total_in * SATS_TO_MSAT,
            outputs_msat=total_out * SATS_TO_MSAT,
            fee_expected_msat=expected_fee * SATS_TO_MSAT,
        )
        if expected_fee < 0:
            conservation.update(status="mismatch", reason="outputs_exceed_inputs")
        elif explicit_fee is None:
            conservation.update(status="unverifiable", reason="no_recorded_fee")
        elif expected_fee != explicit_fee:
            conservation.update(status="mismatch", reason="fee_differs")
        else:
            conservation.update(status="ok", reason=None)
    statuses.append(conservation["status"])
    if conservation.get("reason"):
        reasons.append(f"conservation:{conservation['reason']}")
    payload["conservation"] = conservation

    bundle = graph_core._load_profile_semantics(conn, profile_id, cache=semantics_cache)
    owned_index = bundle.owned_index
    chain_network = _norm_chain_network(chain, network)
    input_views: list[dict[str, Any]] = []
    output_views: list[dict[str, Any]] = []
    per_wallet: dict[str, dict[str, Any]] = {}
    ambiguous_nodes = 0

    def account(node: Mapping[str, Any], matches: Sequence[Any], side: str) -> dict[str, Any]:
        nonlocal ambiguous_nodes
        view = _node_view(node, matches)
        if view["ambiguous"]:
            ambiguous_nodes += 1
        elif view["owned"]:
            leg = per_wallet.setdefault(
                view["wallet_id"],
                {
                    "wallet_id": view["wallet_id"],
                    "wallet": view["wallet"],
                    "owned_inputs_msat": 0,
                    "owned_outputs_msat": 0,
                    "owned_input_count": 0,
                    "owned_output_count": 0,
                    "values_complete": True,
                },
            )
            leg[f"owned_{side}_count"] += 1
            if view["amount_msat"] is None:
                leg["values_complete"] = False
            else:
                leg[f"owned_{side}s_msat"] += view["amount_msat"]
        return view

    for node in inputs:
        matches = (
            graph_core._node_matches(node, owned_index, chain_network)
            if owned_index is not None
            else []
        )
        input_views.append(account(node, matches, "input"))
    for node in outputs:
        if graph_core._is_unspendable(node.get("_script")) or node.get("_pegoutAddress"):
            view = _node_view(node, [])
            view["unspendable"] = True
            output_views.append(view)
            continue
        matches = (
            graph_core._node_matches(node, owned_index, chain_network)
            if owned_index is not None
            else []
        )
        output_views.append(account(node, matches, "output"))

    rows = _txid_rows(conn, profile_id, txid, chain_network)
    rows_by_wallet: dict[str, list[Mapping[str, Any]]] = {}
    for candidate in rows:
        rows_by_wallet.setdefault(str(candidate["wallet_id"]), []).append(candidate)

    def row_view(candidate: Mapping[str, Any]) -> dict[str, Any]:
        return {
            "transaction_id": str(candidate["id"]),
            "direction": candidate["direction"],
            "recorded_delta_msat": row_boundary_amounts(candidate).wallet_delta_msat,
            "excluded": bool(candidate["excluded"]),
        }

    legs: list[dict[str, Any]] | None = None
    unexplained: list[dict[str, Any]] | None = None
    if owned_index is None:
        statuses.append("unverifiable")
        reasons.append("legs:ownership_index_unavailable")
    else:
        legs = []
        for wallet_id in sorted(per_wallet, key=lambda key: (per_wallet[key]["wallet"] or "", key)):
            leg = per_wallet[wallet_id]
            complete = leg.pop("values_complete")
            expected = (
                leg["owned_outputs_msat"] - leg["owned_inputs_msat"] if complete else None
            )
            wallet_rows = [row_view(candidate) for candidate in rows_by_wallet.get(wallet_id, [])]
            active = [item for item in wallet_rows if not item["excluded"]]
            if not wallet_rows:
                status, reason = "mismatch", "missing_imported_leg"
            elif not active:
                status, reason = "excluded", "imported_leg_excluded"
            elif len(active) > 1:
                status, reason = "mismatch", "several_imported_legs"
            elif expected is None:
                status, reason = "unverifiable", "owned_values_hidden"
            elif active[0]["recorded_delta_msat"] != expected:
                status, reason = "mismatch", "amount_differs"
            else:
                status, reason = "ok", None
            legs.append(
                {
                    **leg,
                    "expected_delta_msat": expected,
                    "rows": wallet_rows,
                    "status": status,
                    "reason": reason,
                }
            )
            statuses.append("ok" if status == "excluded" else status)
            if reason and status != "excluded":
                reasons.append(f"leg:{reason}")
        observed_wallets = _observed_wallet_ids(owned_index)
        unexplained = []
        for wallet_id, wallet_rows in rows_by_wallet.items():
            if wallet_id in per_wallet:
                continue
            observed = wallet_id in observed_wallets
            for candidate in wallet_rows:
                item = {
                    **row_view(candidate),
                    "wallet_id": wallet_id,
                    "wallet": candidate["wallet_label"],
                    "wallet_kind": candidate["wallet_kind"],
                    "observed_wallet": observed,
                    "status": (
                        "excluded"
                        if candidate["excluded"]
                        else "mismatch"
                        if observed
                        else "external_record"
                    ),
                }
                unexplained.append(item)
                if item["status"] == "mismatch":
                    statuses.append("mismatch")
                    reasons.append("leg:row_without_owned_graph_leg")
        if ambiguous_nodes:
            statuses.append("unverifiable")
            reasons.append("legs:ambiguous_ownership")

    if "mismatch" in statuses:
        status = "mismatch"
    elif all(item in {"ok", "not_applicable"} for item in statuses):
        status = "ok"
    else:
        status = "partial"
    return {
        **payload,
        "status": status,
        "reasons": sorted(set(reasons)),
        "legs": legs,
        "unexplained_rows": unexplained[:MAX_ROWS] if unexplained is not None else None,
        "input_count": len(input_views),
        "output_count": len(output_views),
        "nodes_truncated": len(input_views) > MAX_NODES_PER_SIDE
        or len(output_views) > MAX_NODES_PER_SIDE,
        "inputs": input_views[:MAX_NODES_PER_SIDE],
        "outputs": output_views[:MAX_NODES_PER_SIDE],
    }


__all__ = ["VERIFY_DEFINITIONS", "verify_transaction"]
