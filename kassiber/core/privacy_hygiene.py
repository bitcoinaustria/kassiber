from __future__ import annotations

import json
import sqlite3
from collections import Counter, defaultdict
from typing import Any, Mapping

from ..errors import AppError
from .repo import current_context_snapshot


BASE_SCORE = 70
MAX_RETURNED_TRANSACTIONS = 200
DEFAULT_RETURNED_TRANSACTIONS = 50
EVIDENCE_LEVELS = {"ground_truth", "reviewed", "imported", "heuristic", "unavailable"}
USER_ATTRIBUTION = "user_wallet"
COUNTERPARTY_ATTRIBUTION = "counterparty"
LOCAL_DATA_ATTRIBUTION = "local_data"
INBOUND_COUNTERPARTY_FINDINGS = {
    "change_position_fingerprint",
    "change_type_fingerprint",
    "common_input_ownership",
    "rbf_signal",
    "round_fee_rate",
    "round_output_amount",
    "script_type_mix",
    "unnecessary_input_heuristic",
    "wallet_fingerprint_locktime",
    "wallet_fingerprint_version",
    "wallet_fingerprint_witness",
}
ROUND_BTC_DENOMINATIONS_SATS = {
    100_000,
    200_000,
    500_000,
    1_000_000,
    2_000_000,
    5_000_000,
    10_000_000,
    20_000_000,
    25_000_000,
    50_000_000,
    100_000_000,
    200_000_000,
    500_000_000,
    1_000_000_000,
}
MEANINGFUL_ROUND_AMOUNT_FLOOR_SATS = 10_000
FINDING_SEVERITY_RANK = {
    "positive": 0,
    "info": 1,
    "low": 2,
    "medium": 3,
    "high": 4,
    "critical": 5,
}
FINDING_METADATA = {
    "address_reuse": {
        "evidence_level": "ground_truth",
        "remediation": "Review the reused receive-address context and keep the affected rows labelled; Kassiber does not change wallet state from this tell.",
    },
    "change_position_fingerprint": {
        "evidence_level": "heuristic",
        "remediation": "Treat this as an advisory change-shape tell and confirm intent from first-party wallet records before drawing conclusions.",
    },
    "change_type_fingerprint": {
        "evidence_level": "heuristic",
        "remediation": "Review the script-type mismatch as context only; no tax, balance, or transfer state is changed.",
    },
    "coin_anonymity_evidence": {
        "evidence_level": "imported",
        "remediation": "Keep the imported wallet evidence attached and treat it as boundary context, not proof of a complete privacy graph.",
    },
    "coinjoin_pattern": {
        "evidence_level": "heuristic",
        "remediation": "Document the CoinJoin boundary locally and do not infer unrelated participant ownership from this tell.",
    },
    "common_input_ownership": {
        "evidence_level": "heuristic",
        "remediation": "Use this as an advisory common-input tell only; confirm ownership with first-party wallet evidence before relying on it.",
    },
    "dust_utxo_exposure": {
        "evidence_level": "ground_truth",
        "remediation": "Label or review small-output provenance when it matters; Kassiber leaves balances and UTXO state untouched.",
    },
    "large_utxo_set": {
        "evidence_level": "ground_truth",
        "remediation": "Use the count as review context for the wallet inventory; no spending or consolidation advice is implied.",
    },
    "legacy_address_type": {
        "evidence_level": "ground_truth",
        "remediation": "Record this as an older script-type observation; it is not an accounting action or a privacy verdict.",
    },
    "op_return_metadata": {
        "evidence_level": "ground_truth",
        "remediation": "Review the transaction context knowing permanent metadata exists; Kassiber does not inspect or enrich external entities.",
    },
    "payjoin_boundary": {
        "evidence_level": "imported",
        "remediation": "Keep the PayJoin boundary documented locally; Kassiber does not cross it or infer the counterparty graph.",
    },
    "rbf_signal": {
        "evidence_level": "ground_truth",
        "remediation": "Treat replace-by-fee signalling as transaction-shape context only.",
    },
    "round_fee_rate": {
        "evidence_level": "heuristic",
        "remediation": "Treat the rounded fee rate as a weak wallet-shape tell and verify with first-party records if it matters.",
    },
    "round_output_amount": {
        "evidence_level": "heuristic",
        "remediation": "Keep the round amount as payment-shape context; attach invoice or counterparty records if the distinction matters.",
    },
    "script_type_mix": {
        "evidence_level": "heuristic",
        "remediation": "Review mixed script families as a local transaction-shape tell, not as proof of ownership.",
    },
    "taproot_usage": {
        "evidence_level": "ground_truth",
        "remediation": "Treat Taproot usage as a local script-type observation; it is not a standing privacy guarantee.",
    },
    "transaction_coverage_gap": {
        "evidence_level": "unavailable",
        "remediation": "Import or sync local vin/vout detail if you need this transaction analysed; otherwise keep the gap explicit.",
    },
    "transaction_not_analysable": {
        "evidence_level": "unavailable",
        "remediation": "Leave this transaction as unknown unless local first-party transaction graph data is imported or synced.",
    },
    "unnecessary_input_heuristic": {
        "evidence_level": "heuristic",
        "remediation": "Treat this as an advisory input-shape tell and review the source-wallet record before relying on it.",
    },
    "wallet_fingerprint_locktime": {
        "evidence_level": "heuristic",
        "remediation": "Use the locktime pattern as weak wallet-fingerprint context only.",
    },
    "wallet_fingerprint_version": {
        "evidence_level": "heuristic",
        "remediation": "Use the version pattern as weak wallet-fingerprint context only.",
    },
    "wallet_fingerprint_witness": {
        "evidence_level": "heuristic",
        "remediation": "Use the witness-shape pattern as weak wallet-fingerprint context only.",
    },
}


def build_privacy_hygiene_snapshot(
    conn: sqlite3.Connection,
    args: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    """Build the local Phase-1 privacy-hygiene snapshot.

    The scorer is intentionally read-only over data Kassiber already has:
    ``transactions.raw_json`` and the durable ``wallet_utxos`` inventory. It
    never asks a backend for missing prevouts, so graphless imports and current
    Bitcoin Core detail rows become explicit coverage states instead of quiet
    false confidence.
    """

    options = _coerce_args(args)
    unknown = sorted(set(options) - {"wallet", "transaction", "limit"})
    if unknown:
        raise AppError(
            "ui.privacy_hygiene.snapshot received unsupported fields",
            code="validation",
            details={"unknown": unknown},
            retryable=False,
        )

    context = current_context_snapshot(conn)
    profile_id = str(context.get("profile_id") or "")
    if not profile_id:
        return _empty_snapshot(context)

    wallet = _resolve_wallet(conn, profile_id, options.get("wallet"))
    wallet_id = wallet["id"] if wallet is not None else None
    transaction_ref = _string_or_none(options.get("transaction"))
    returned_limit = _coerce_limit(options.get("limit"))

    profile = conn.execute(
        "SELECT id, label FROM profiles WHERE id = ?",
        (profile_id,),
    ).fetchone()
    wallets = _load_wallets(conn, profile_id, wallet_id=wallet_id)
    wallet_by_id = {row["id"]: row for row in wallets}
    inventory = _load_inventory(conn, profile_id, wallet_id=wallet_id)
    inventory_index = _inventory_indexes(inventory, wallet_by_id)
    transactions = _load_transactions(
        conn,
        profile_id,
        wallet_id=wallet_id,
        transaction_ref=transaction_ref,
    )
    if transaction_ref is not None and not transactions:
        raise AppError(
            "Transaction not found for privacy-hygiene scan",
            code="not_found",
            details={"transaction": transaction_ref},
            retryable=False,
        )

    tx_results = [
        _score_transaction(row, inventory_index)
        for row in transactions
    ]
    wallet_results = _score_wallets(wallets, inventory, tx_results)
    returned_transactions = _returned_transactions(tx_results, returned_limit)
    aggregate_findings = _aggregate_findings(
        [finding for wallet_row in wallet_results for finding in wallet_row["findings"]]
        + [finding for tx in tx_results for finding in tx["findings"]]
    )
    summary = _summary(wallet_results, tx_results, aggregate_findings)

    return {
        "profile": (
            {"id": profile["id"], "label": profile["label"]}
            if profile is not None
            else {"id": profile_id, "label": context.get("profile_label") or ""}
        ),
        "scope": {
            "wallet": (
                {"id": wallet["id"], "label": wallet["label"]}
                if wallet is not None
                else None
            ),
            "transaction": transaction_ref,
        },
        "summary": summary,
        "coverage": _coverage(wallets, inventory, tx_results),
        "wallets": [_public_wallet_result(row) for row in wallet_results],
        "transactions": returned_transactions,
        "findings": aggregate_findings[:12],
        "meta": {
            "engine": "privacy_hygiene_phase1",
            "local_only": True,
            "egress": "none",
            "scope": "single_transaction_and_address_level",
            "phase2_deferred": [
                "cross_wallet_cluster_reconstruction",
                "peel_chain_detection",
                "boltzmann_entropy",
                "coin_selection_fingerprint",
                "psbt_prebroadcast_check",
            ],
        },
    }


def _empty_snapshot(context: Mapping[str, Any]) -> dict[str, Any]:
    return {
        "profile": None,
        "scope": {"wallet": None, "transaction": None},
        "summary": {
            "state": "no_active_profile",
            "wallet_count": 0,
            "transaction_count": 0,
            "risk_weight": 0,
            "risk_count": 0,
            "unknown_count": 0,
            "risk_level": "none",
            "finding_counts": _empty_finding_counts(),
            "top_findings": [],
        },
        "coverage": {
            "wallet_count": 0,
            "wallets_with_inventory": 0,
            "inventory_outputs": 0,
            "active_utxos": 0,
            "transaction_total": 0,
            "transaction_full": 0,
            "transaction_partial": 0,
            "transaction_not_analysable": 0,
            "transaction_scored": 0,
        },
        "wallets": [],
        "transactions": [],
        "findings": [],
        "meta": {
            "engine": "privacy_hygiene_phase1",
            "local_only": True,
            "egress": "none",
            "scope": "single_transaction_and_address_level",
            "context": {
                "workspace": context.get("workspace_label") or None,
                "profile": context.get("profile_label") or None,
            },
        },
    }


def _coerce_args(args: Mapping[str, Any] | None) -> dict[str, Any]:
    if args is None:
        return {}
    if isinstance(args, Mapping):
        return dict(args)
    raise AppError(
        "ui.privacy_hygiene.snapshot args must be an object",
        code="validation",
        details={"type": type(args).__name__},
        retryable=False,
    )


def _coerce_limit(raw: Any) -> int:
    if raw in (None, ""):
        return DEFAULT_RETURNED_TRANSACTIONS
    try:
        limit = int(raw)
    except (TypeError, ValueError) as exc:
        raise AppError(
            "Privacy-hygiene transaction limit must be an integer",
            code="validation",
            details={"limit": raw},
            retryable=False,
        ) from exc
    if limit < 1:
        raise AppError(
            "Privacy-hygiene transaction limit must be positive",
            code="validation",
            details={"limit": raw},
            retryable=False,
        )
    return min(limit, MAX_RETURNED_TRANSACTIONS)


def _resolve_wallet(
    conn: sqlite3.Connection,
    profile_id: str,
    value: Any,
) -> sqlite3.Row | None:
    wallet_ref = _string_or_none(value)
    if wallet_ref is None:
        return None
    row = conn.execute(
        """
        SELECT id, label, kind
        FROM wallets
        WHERE profile_id = ?
          AND (id = ? OR label = ?)
        ORDER BY CASE WHEN id = ? THEN 0 ELSE 1 END, label ASC
        LIMIT 1
        """,
        (profile_id, wallet_ref, wallet_ref, wallet_ref),
    ).fetchone()
    if row is None:
        raise AppError(
            "Wallet not found for privacy-hygiene scan",
            code="not_found",
            details={"wallet": wallet_ref},
            retryable=False,
        )
    return row


def _load_wallets(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    wallet_id: str | None,
) -> list[sqlite3.Row]:
    params: list[Any] = [profile_id]
    where_wallet = ""
    if wallet_id is not None:
        where_wallet = "AND id = ?"
        params.append(wallet_id)
    return conn.execute(
        f"""
        SELECT id, label, kind
        FROM wallets
        WHERE profile_id = ?
          {where_wallet}
        ORDER BY label ASC, id ASC
        """,
        params,
    ).fetchall()


def _load_inventory(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    wallet_id: str | None,
) -> list[sqlite3.Row]:
    params: list[Any] = [profile_id]
    where_wallet = ""
    if wallet_id is not None:
        where_wallet = "AND u.wallet_id = ?"
        params.append(wallet_id)
    return conn.execute(
        f"""
        SELECT u.*, w.label AS wallet_label, w.kind AS wallet_kind
        FROM wallet_utxos u
        JOIN wallets w ON w.id = u.wallet_id
        WHERE u.profile_id = ?
          {where_wallet}
        ORDER BY u.wallet_id ASC, u.txid ASC, u.vout ASC
        """,
        params,
    ).fetchall()


def _load_transactions(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    wallet_id: str | None,
    transaction_ref: str | None,
) -> list[sqlite3.Row]:
    params: list[Any] = [profile_id]
    filters = ["t.profile_id = ?", "t.excluded = 0"]
    if wallet_id is not None:
        filters.append("t.wallet_id = ?")
        params.append(wallet_id)
    if transaction_ref is not None:
        filters.append("(t.id = ? OR lower(t.external_id) = lower(?))")
        params.extend([transaction_ref, transaction_ref])
    return conn.execute(
        f"""
        SELECT
            t.*,
            w.label AS wallet_label,
            w.kind AS wallet_kind
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        WHERE {" AND ".join(filters)}
        ORDER BY t.occurred_at DESC, t.created_at DESC, t.id DESC
        """,
        params,
    ).fetchall()


def _inventory_indexes(
    rows: list[sqlite3.Row],
    wallet_by_id: Mapping[str, sqlite3.Row],
) -> dict[str, Any]:
    outpoints: dict[tuple[str, int], dict[str, Any]] = {}
    scripts: dict[str, dict[str, Any]] = {}
    wallet_inventory: dict[str, list[sqlite3.Row]] = defaultdict(list)
    for row in rows:
        wallet_inventory[row["wallet_id"]].append(row)
        txid = _txid_or_none(row["txid"])
        try:
            vout = int(row["vout"])
        except (TypeError, ValueError):
            vout = -1
        if txid and vout >= 0:
            key = (txid, vout)
            if key in outpoints:
                outpoints[key]["ambiguous"] = True
            else:
                outpoints[key] = _inventory_owner(row, wallet_by_id)
        script = _hex_or_none(row["script_pubkey"])
        if script:
            owner = _inventory_owner(row, wallet_by_id)
            existing = scripts.get(script)
            if existing is None:
                scripts[script] = owner
            elif existing.get("wallet_id") != owner.get("wallet_id"):
                scripts[script] = {
                    "wallet_id": None,
                    "wallet_label": "",
                    "amount_sats": 0,
                    "address_type": (
                        existing["address_type"]
                        if existing["address_type"] == owner["address_type"]
                        else "unknown"
                    ),
                    "ambiguous": True,
                }
    return {
        "outpoints": outpoints,
        "scripts": scripts,
        "wallet_inventory": wallet_inventory,
    }


def _inventory_owner(
    row: sqlite3.Row,
    wallet_by_id: Mapping[str, sqlite3.Row],
) -> dict[str, Any]:
    wallet = wallet_by_id.get(row["wallet_id"])
    amount_msat = _int_or_none(row["amount"]) or 0
    address = _string_or_none(row["address"])
    script = _hex_or_none(row["script_pubkey"])
    return {
        "wallet_id": row["wallet_id"],
        "wallet_label": row["wallet_label"] or (wallet["label"] if wallet else ""),
        "amount_sats": max(0, amount_msat // 1000),
        "address_type": _script_type(address, script),
        "ambiguous": False,
    }


def _score_transaction(
    row: sqlite3.Row,
    inventory_index: Mapping[str, Any],
) -> dict[str, Any]:
    raw = _json_obj(row["raw_json"])
    parsed = _parse_local_transaction(row, raw, inventory_index)
    if parsed["support"] == "none":
        findings = [
            _finding(
                "transaction_not_analysable",
                "info",
                0,
                scope="transaction",
                count=1,
                details={"reason": parsed["reason"]},
            )
        ]
        return _transaction_result(row, None, "not_analysable", parsed, findings)

    findings: list[dict[str, Any]] = []
    coinjoin = _coinjoin_signal(row, raw, parsed)
    if coinjoin:
        findings.append(
            _finding(
                "coinjoin_pattern",
                "positive",
                coinjoin["impact"],
                scope="transaction",
                count=coinjoin["participant_count"],
                details={"pattern": coinjoin["pattern"]},
                evidence_level=coinjoin["evidence_level"],
                attribution=LOCAL_DATA_ATTRIBUTION,
            )
        )

    if _payjoin_boundary(row):
        findings.append(
            _finding(
                "payjoin_boundary",
                "positive",
                8,
                scope="transaction",
                count=1,
                details={},
                evidence_level=_privacy_boundary_evidence_level(row),
                attribution=LOCAL_DATA_ATTRIBUTION,
            )
        )

    if not coinjoin:
        findings.extend(_common_input_findings(parsed))
        findings.extend(_round_output_findings(parsed))
        findings.extend(_unnecessary_input_findings(parsed))

    findings.extend(_fee_findings(parsed))
    findings.extend(_script_type_findings(parsed))
    findings.extend(_change_fingerprint_findings(parsed))
    findings.extend(_wallet_fingerprint_findings(parsed))
    findings.extend(_metadata_findings(parsed))
    findings.extend(_taproot_findings(parsed))
    findings = _apply_direction_attribution(row, findings)

    score = _score_from_findings(findings)
    state = "full" if parsed["support"] == "full" else "partial"
    return _transaction_result(row, score, state, parsed, findings)


def _transaction_result(
    row: sqlite3.Row,
    score: int | None,
    state: str,
    parsed: Mapping[str, Any],
    findings: list[dict[str, Any]],
) -> dict[str, Any]:
    return {
        "id": row["id"],
        "external_id": row["external_id"] or "",
        "wallet_id": row["wallet_id"],
        "wallet_label": row["wallet_label"] or "",
        "occurred_at": row["occurred_at"],
        "direction": row["direction"],
        "asset": row["asset"],
        "score": score,
        "state": state,
        "support": {
            "level": parsed["support"],
            "reason": parsed["reason"],
            "input_count": parsed["input_count"],
            "output_count": parsed["output_count"],
            "known_input_values": parsed["known_input_values"],
            "known_output_values": parsed["known_output_values"],
        },
        "finding_counts": _finding_counts(findings),
        "findings": sorted(
            findings,
            key=lambda item: (
                -FINDING_SEVERITY_RANK.get(item["severity"], 0),
                item["impact"],
                item["code"],
            ),
        ),
    }


def _parse_local_transaction(
    row: sqlite3.Row,
    raw: Mapping[str, Any],
    inventory_index: Mapping[str, Any],
) -> dict[str, Any]:
    vin = raw.get("vin")
    vout = raw.get("vout")
    if not isinstance(vin, list) or not isinstance(vout, list):
        nested = raw.get("tx")
        if isinstance(nested, Mapping):
            vin = nested.get("vin")
            vout = nested.get("vout")
            raw = nested
    if not isinstance(vin, list) or not isinstance(vout, list):
        return _empty_parsed("none", "missing_vin_vout")

    current_txid = _txid_or_none(raw.get("txid")) or _txid_or_none(row["external_id"])
    inputs = []
    for index, entry in enumerate(vin):
        if not isinstance(entry, Mapping):
            continue
        txid = _txid_or_none(entry.get("txid"))
        vout_index = _int_or_none(entry.get("vout"))
        outpoint = (txid, vout_index) if txid is not None and vout_index is not None else None
        owner = (
            inventory_index["outpoints"].get(outpoint)
            if outpoint is not None
            else None
        )
        prevout = entry.get("prevout") if isinstance(entry.get("prevout"), Mapping) else {}
        script = _hex_or_none(prevout.get("scriptpubkey") or prevout.get("script_hex"))
        address = _string_or_none(
            prevout.get("scriptpubkey_address") or prevout.get("address")
        )
        value_sats = _value_sats_or_none(prevout.get("value_sats"))
        if value_sats is None:
            value_sats = _value_sats_or_none(prevout.get("value"))
        if value_sats is None and owner is not None:
            value_sats = owner["amount_sats"]
        script_type = _script_type(address, script)
        if script_type == "unknown" and owner is not None:
            script_type = owner["address_type"]
        inputs.append(
            {
                "index": index,
                "txid": txid,
                "vout": vout_index,
                "value_sats": value_sats,
                "script_type": script_type,
                "owner": owner,
                "sequence": _int_or_none(entry.get("sequence")),
                "witness_items": _witness_item_count(entry),
            }
        )

    outputs = []
    ambiguous_ownership = 0
    for index, entry in enumerate(vout):
        if not isinstance(entry, Mapping):
            continue
        n = _int_or_none(entry.get("n"))
        if n is None:
            n = index
        script = _hex_or_none(entry.get("scriptpubkey") or entry.get("script_hex"))
        address = _string_or_none(entry.get("scriptpubkey_address") or entry.get("address"))
        value_sats = _value_sats_or_none(entry.get("value_sats"))
        if value_sats is None:
            value_sats = _value_sats_or_none(entry.get("value"))
        owner = None
        if current_txid is not None:
            owner = inventory_index["outpoints"].get((current_txid, n))
        if owner is None and script is not None:
            owner = inventory_index["scripts"].get(script)
            if owner is not None and owner.get("ambiguous"):
                ambiguous_ownership += 1
                owner = None
        outputs.append(
            {
                "index": n,
                "value_sats": value_sats,
                "script_type": _script_type(address, script),
                "owner": owner,
                "op_return": _is_op_return(entry, script),
            }
        )

    known_input_values = sum(1 for item in inputs if item["value_sats"] is not None)
    known_output_values = sum(1 for item in outputs if item["value_sats"] is not None)
    support = "full"
    reason = None
    if not inputs and not outputs:
        return _empty_parsed("none", "empty_vin_vout")
    if known_input_values < len(inputs) or known_output_values < len(outputs):
        support = "partial"
        reason = "missing_prevout_or_output_values"
    if ambiguous_ownership:
        support = "partial"
        if reason is None:
            reason = "ambiguous_inventory_owner"

    return {
        "support": support,
        "reason": reason,
        "input_count": len(inputs),
        "output_count": len(outputs),
        "known_input_values": known_input_values,
        "known_output_values": known_output_values,
        "ambiguous_ownership": ambiguous_ownership,
        "inputs": inputs,
        "outputs": outputs,
        "version": _int_or_none(raw.get("version")),
        "locktime": _int_or_none(raw.get("locktime") or raw.get("lock_time")),
        "fee_sats": _fee_sats(row, raw, inputs, outputs),
        "vsize": _int_or_none(raw.get("vsize")) or _int_or_none(raw.get("virtual_size")),
    }


def _empty_parsed(support: str, reason: str) -> dict[str, Any]:
    return {
        "support": support,
        "reason": reason,
        "input_count": 0,
        "output_count": 0,
        "known_input_values": 0,
        "known_output_values": 0,
        "inputs": [],
        "outputs": [],
        "version": None,
        "locktime": None,
        "fee_sats": None,
        "vsize": None,
    }


def _coinjoin_signal(
    row: sqlite3.Row,
    raw: Mapping[str, Any],
    parsed: Mapping[str, Any],
) -> dict[str, Any] | None:
    outputs = [item for item in parsed["outputs"] if not item["op_return"]]
    value_counts = Counter(
        item["value_sats"]
        for item in outputs
        if item["value_sats"] is not None and item["value_sats"] > 0
    )
    most_common_count = value_counts.most_common(1)[0][1] if value_counts else 0
    boundary = str(row["privacy_boundary"] or "").strip().lower()
    raw_likely = bool(raw.get("islikelycoinjoin") or raw.get("is_likely_coinjoin"))
    if boundary == "coinjoin" or raw_likely:
        return {
            "pattern": "reviewed_or_imported_coinjoin",
            "participant_count": max(most_common_count, 1),
            "impact": 24 if most_common_count >= 5 else 18,
            "evidence_level": (
                _privacy_boundary_evidence_level(row)
                if boundary == "coinjoin"
                else "imported"
            ),
        }
    if 5 <= len(outputs) <= 12 and most_common_count >= 5:
        return {
            "pattern": "equal_output_coinjoin",
            "participant_count": most_common_count,
            "impact": 18,
            "evidence_level": "heuristic",
        }
    if parsed["input_count"] >= 20 and len(outputs) >= 20 and most_common_count >= 5:
        return {
            "pattern": "large_equal_output_coinjoin",
            "participant_count": most_common_count,
            "impact": 20,
            "evidence_level": "heuristic",
        }
    return None


def _payjoin_boundary(row: sqlite3.Row) -> bool:
    return str(row["privacy_boundary"] or "").strip().lower() == "payjoin"


def _privacy_boundary_evidence_level(row: sqlite3.Row) -> str:
    status = str(row["review_status"] or "").strip().lower()
    if status in {"accepted", "completed", "reviewed"}:
        return "reviewed"
    return "imported"


def _apply_direction_attribution(
    row: sqlite3.Row,
    findings: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    direction = str(row["direction"] or "").strip().lower()
    if direction != "inbound":
        return findings
    adjusted: list[dict[str, Any]] = []
    for finding in findings:
        if (
            finding["code"] in INBOUND_COUNTERPARTY_FINDINGS
            and int(finding.get("impact") or 0) < 0
        ):
            item = dict(finding)
            item["attribution"] = COUNTERPARTY_ATTRIBUTION
            item["impact"] = 0
            item["remediation"] = (
                "This tell is on the inbound payer side; keep it as context and do not lower the receiving wallet's risk from it."
            )
            adjusted.append(item)
        else:
            adjusted.append(finding)
    return adjusted


def _common_input_findings(parsed: Mapping[str, Any]) -> list[dict[str, Any]]:
    inputs = parsed["inputs"]
    unique_outpoints = {
        (item["txid"], item["vout"])
        for item in inputs
        if item["txid"] is not None and item["vout"] is not None
    }
    if len(unique_outpoints) <= 1:
        return []
    count = len(unique_outpoints)
    if count >= 50:
        impact = -45
        severity = "critical"
    elif count >= 20:
        impact = -35
        severity = "critical"
    elif count >= 10:
        impact = -25
        severity = "high"
    elif count >= 5:
        impact = -15
        severity = "high"
    else:
        impact = -3 * count
        severity = "medium"
    return [
        _finding(
            "common_input_ownership",
            severity,
            impact,
            scope="transaction",
            count=count,
            details={"input_count": count},
        )
    ]


def _round_output_findings(parsed: Mapping[str, Any]) -> list[dict[str, Any]]:
    round_outputs = [
        item for item in parsed["outputs"]
        if not item["op_return"] and _is_round_amount(item["value_sats"])
    ]
    if not round_outputs:
        return []
    count = len(round_outputs)
    impact = -min(count * 8, 20)
    return [
        _finding(
            "round_output_amount",
            "medium" if count < 3 else "high",
            impact,
            scope="transaction",
            count=count,
            details={"round_outputs": count},
        )
    ]


def _unnecessary_input_findings(parsed: Mapping[str, Any]) -> list[dict[str, Any]]:
    inputs = [item for item in parsed["inputs"] if item["value_sats"] is not None]
    spendable_outputs = [
        item for item in parsed["outputs"]
        if not item["op_return"] and item["value_sats"] is not None
    ]
    if len(inputs) < 2 or len(spendable_outputs) != 2:
        return []
    fee_sats = parsed["fee_sats"] or 0
    largest_input = max(item["value_sats"] for item in inputs)
    total_input = sum(item["value_sats"] for item in inputs)
    if total_input <= largest_input:
        return []
    candidates = [
        item for item in spendable_outputs
        if item["value_sats"] + fee_sats <= largest_input
    ]
    if not candidates:
        return []
    return [
        _finding(
            "unnecessary_input_heuristic",
            "medium",
            -8,
            scope="transaction",
            count=len(candidates),
            details={"candidate_outputs": len(candidates)},
        )
    ]


def _fee_findings(parsed: Mapping[str, Any]) -> list[dict[str, Any]]:
    findings: list[dict[str, Any]] = []
    fee_sats = parsed["fee_sats"]
    vsize = parsed["vsize"]
    if fee_sats is not None and vsize and vsize > 0:
        fee_rate = fee_sats / vsize
        rounded = int(round(fee_rate))
        if abs(fee_rate - rounded) < 0.01 and rounded in {1, 2, 3, 5, 10, 15, 20, 25, 50, 100}:
            findings.append(
                _finding(
                    "round_fee_rate",
                    "low",
                    -3,
                    scope="transaction",
                    count=1,
                    details={"sat_vb": rounded},
                )
            )
    sequences = [
        item["sequence"] for item in parsed["inputs"] if item["sequence"] is not None
    ]
    if sequences and any(sequence < 0xFFFFFFFE for sequence in sequences):
        findings.append(
            _finding(
                "rbf_signal",
                "low",
                -2,
                scope="transaction",
                count=sum(1 for sequence in sequences if sequence < 0xFFFFFFFE),
                details={},
            )
        )
    return findings


def _script_type_findings(parsed: Mapping[str, Any]) -> list[dict[str, Any]]:
    input_types = _meaningful_types(item["script_type"] for item in parsed["inputs"])
    output_types = _meaningful_types(
        item["script_type"] for item in parsed["outputs"] if not item["op_return"]
    )
    all_types = input_types | output_types
    if len(all_types) <= 1:
        return []
    return [
        _finding(
            "script_type_mix",
            "medium",
            -8,
            scope="transaction",
            count=len(all_types),
            details={"type_count": len(all_types)},
        )
    ]


def _change_fingerprint_findings(parsed: Mapping[str, Any]) -> list[dict[str, Any]]:
    findings: list[dict[str, Any]] = []
    spendable_outputs = [item for item in parsed["outputs"] if not item["op_return"]]
    input_types = _meaningful_types(item["script_type"] for item in parsed["inputs"])
    output_types = _meaningful_types(item["script_type"] for item in spendable_outputs)
    if len(spendable_outputs) == 2 and len(input_types) == 1 and len(output_types) > 1:
        input_type = next(iter(input_types))
        matching_outputs = [
            item for item in spendable_outputs if item["script_type"] == input_type
        ]
        if len(matching_outputs) == 1:
            findings.append(
                _finding(
                    "change_type_fingerprint",
                    "medium",
                    -8,
                    scope="transaction",
                    count=1,
                    details={},
                )
            )
    owned_outputs = [item for item in spendable_outputs if item["owner"] is not None]
    if len(spendable_outputs) >= 2 and len(owned_outputs) == 1:
        owned_position = owned_outputs[0]["index"]
        output_positions = [item["index"] for item in spendable_outputs]
        if owned_position in {min(output_positions), max(output_positions)}:
            findings.append(
                _finding(
                    "change_position_fingerprint",
                    "low",
                    -4,
                    scope="transaction",
                    count=1,
                    details={
                        "position": (
                            "first"
                            if owned_position == min(output_positions)
                            else "last"
                        )
                    },
                )
            )
    return findings


def _wallet_fingerprint_findings(parsed: Mapping[str, Any]) -> list[dict[str, Any]]:
    findings: list[dict[str, Any]] = []
    locktime = parsed["locktime"]
    if locktime not in (None, 0):
        findings.append(
            _finding(
                "wallet_fingerprint_locktime",
                "low",
                -3,
                scope="transaction",
                count=1,
                details={},
            )
        )
    version = parsed["version"]
    if version is not None and version not in {1, 2}:
        findings.append(
            _finding(
                "wallet_fingerprint_version",
                "low",
                -3,
                scope="transaction",
                count=1,
                details={"version": version},
            )
        )
    witness_counts = [
        item["witness_items"]
        for item in parsed["inputs"]
        if item["witness_items"] is not None and item["witness_items"] > 0
    ]
    if len(witness_counts) >= 2 and len(set(witness_counts)) == 1:
        findings.append(
            _finding(
                "wallet_fingerprint_witness",
                "low",
                -2,
                scope="transaction",
                count=len(witness_counts),
                details={"witness_items": witness_counts[0]},
            )
        )
    return findings


def _metadata_findings(parsed: Mapping[str, Any]) -> list[dict[str, Any]]:
    op_returns = [item for item in parsed["outputs"] if item["op_return"]]
    if not op_returns:
        return []
    return [
        _finding(
            "op_return_metadata",
            "high",
            -10,
            scope="transaction",
            count=len(op_returns),
            details={"outputs": len(op_returns)},
        )
    ]


def _taproot_findings(parsed: Mapping[str, Any]) -> list[dict[str, Any]]:
    spendable_outputs = [item for item in parsed["outputs"] if not item["op_return"]]
    if not spendable_outputs:
        return []
    taproot_outputs = [
        item for item in spendable_outputs if item["script_type"] == "p2tr"
    ]
    if not taproot_outputs:
        return []
    ratio = len(taproot_outputs) / len(spendable_outputs)
    if ratio < 0.5:
        return []
    return [
        _finding(
            "taproot_usage",
            "positive",
            4 if ratio < 1 else 6,
            scope="transaction",
            count=len(taproot_outputs),
            details={"taproot_outputs": len(taproot_outputs)},
        )
    ]


def _score_wallets(
    wallets: list[sqlite3.Row],
    inventory: list[sqlite3.Row],
    tx_results: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    inventory_by_wallet: dict[str, list[sqlite3.Row]] = defaultdict(list)
    tx_by_wallet: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in inventory:
        inventory_by_wallet[row["wallet_id"]].append(row)
    for tx in tx_results:
        tx_by_wallet[tx["wallet_id"]].append(tx)

    results = []
    for wallet in wallets:
        wallet_inventory = inventory_by_wallet.get(wallet["id"], [])
        wallet_transactions = tx_by_wallet.get(wallet["id"], [])
        address_findings = _address_level_findings(wallet_inventory)
        transaction_findings = [
            finding
            for tx in wallet_transactions
            for finding in tx["findings"]
        ]
        tx_scores = [
            tx["score"] for tx in wallet_transactions if tx["score"] is not None
        ]
        if tx_scores:
            base = round(sum(tx_scores) / len(tx_scores))
            score = _clamp_score(base + sum(item["impact"] for item in address_findings))
            state = "partial" if any(tx["state"] != "full" for tx in wallet_transactions) else "full"
        elif wallet_inventory:
            score = _score_from_findings(address_findings)
            state = "address_only"
        else:
            score = None
            state = "not_enough_data"
        wallet_findings = address_findings + _wallet_transaction_findings(wallet_transactions)
        findings = sorted(
            wallet_findings,
            key=lambda item: (
                -FINDING_SEVERITY_RANK.get(item["severity"], 0),
                item["impact"],
                item["code"],
            ),
        )
        risk_findings = sorted(
            wallet_findings + transaction_findings,
            key=lambda item: (
                -FINDING_SEVERITY_RANK.get(item["severity"], 0),
                item["impact"],
                item["code"],
            ),
        )
        results.append(
            {
                "id": wallet["id"],
                "label": wallet["label"],
                "kind": wallet["kind"],
                "score": score,
                "state": state,
                "transaction_count": len(wallet_transactions),
                "scored_transaction_count": len(tx_scores),
                "inventory_output_count": len(wallet_inventory),
                "active_utxo_count": sum(
                    1 for row in wallet_inventory if not _string_or_none(row["spent_at"])
                ),
                "findings": findings,
                "risk_findings": risk_findings,
                "address": _address_summary(wallet_inventory),
            }
        )
    return results


def _address_level_findings(rows: list[sqlite3.Row]) -> list[dict[str, Any]]:
    findings: list[dict[str, Any]] = []
    by_address: dict[str, list[sqlite3.Row]] = defaultdict(list)
    active_rows = []
    script_types = Counter()
    anonymity_scores = []
    for row in rows:
        address = _string_or_none(row["address"])
        script_type = _script_type(address, _hex_or_none(row["script_pubkey"]))
        if script_type != "unknown":
            script_types[script_type] += 1
        if address is not None:
            by_address[address].append(row)
        if not _string_or_none(row["spent_at"]):
            active_rows.append(row)
        anonymity_score = _int_or_none(row["anonymity_score"])
        if anonymity_score is not None:
            anonymity_scores.append(anonymity_score)

    reused = [
        address_rows for address_rows in by_address.values()
        if len({f"{row['txid']}:{row['vout']}" for row in address_rows}) > 1
    ]
    if reused:
        reused_outputs = sum(len(group) for group in reused)
        findings.append(
            _finding(
                "address_reuse",
                "critical",
                -min(12 + (len(reused) * 6), 30),
                scope="wallet",
                count=len(reused),
                details={"outputs": reused_outputs},
            )
        )

    dust_count = sum(1 for row in active_rows if int(row["amount"] or 0) // 1000 < 1000)
    if dust_count:
        findings.append(
            _finding(
                "dust_utxo_exposure",
                "medium",
                -min(4 + dust_count * 2, 12),
                scope="wallet",
                count=dust_count,
                details={"active_dust_utxos": dust_count},
            )
        )

    active_count = len(active_rows)
    if active_count >= 50:
        findings.append(
            _finding(
                "large_utxo_set",
                "high",
                -15,
                scope="wallet",
                count=active_count,
                details={"active_utxos": active_count},
            )
        )
    elif active_count >= 20:
        findings.append(
            _finding(
                "large_utxo_set",
                "medium",
                -8,
                scope="wallet",
                count=active_count,
                details={"active_utxos": active_count},
            )
        )

    legacy_count = script_types["p2pkh"] + script_types["p2sh"]
    if legacy_count:
        findings.append(
            _finding(
                "legacy_address_type",
                "medium",
                -min(legacy_count * 2, 10),
                scope="wallet",
                count=legacy_count,
                details={"legacy_outputs": legacy_count},
            )
        )

    taproot_count = script_types["p2tr"]
    if taproot_count and taproot_count >= max(1, sum(script_types.values()) // 2):
        findings.append(
            _finding(
                "taproot_usage",
                "positive",
                5,
                scope="wallet",
                count=taproot_count,
                details={"taproot_outputs": taproot_count},
            )
        )

    if anonymity_scores:
        high_scores = [score for score in anonymity_scores if score >= 20]
        if high_scores:
            findings.append(
                _finding(
                    "coin_anonymity_evidence",
                    "positive",
                    min(10, 4 + len(high_scores)),
                    scope="wallet",
                    count=len(high_scores),
                    details={"outputs": len(high_scores)},
                )
            )
    return findings


def _wallet_transaction_findings(
    wallet_transactions: list[dict[str, Any]]
) -> list[dict[str, Any]]:
    unanalysable = sum(1 for tx in wallet_transactions if tx["state"] == "not_analysable")
    if not wallet_transactions or unanalysable == 0:
        return []
    return [
        _finding(
            "transaction_coverage_gap",
            "info",
            0,
            scope="wallet",
            count=unanalysable,
            details={"transactions": unanalysable},
        )
    ]


def _address_summary(rows: list[sqlite3.Row]) -> dict[str, Any]:
    active_rows = [row for row in rows if not _string_or_none(row["spent_at"])]
    address_counts = Counter(
        address for row in rows
        if (address := _string_or_none(row["address"])) is not None
    )
    script_types = Counter(
        script_type for row in rows
        if (
            script_type := _script_type(
                _string_or_none(row["address"]),
                _hex_or_none(row["script_pubkey"]),
            )
        ) != "unknown"
    )
    reused_count = sum(1 for count in address_counts.values() if count > 1)
    return {
        "known_address_count": len(address_counts),
        "reused_address_count": reused_count,
        "active_utxo_count": len(active_rows),
        "dust_utxo_count": sum(
            1 for row in active_rows if int(row["amount"] or 0) // 1000 < 1000
        ),
        "script_type_counts": dict(sorted(script_types.items())),
    }


def _summary(
    wallet_results: list[dict[str, Any]],
    tx_results: list[dict[str, Any]],
    aggregate_findings: list[dict[str, Any]],
) -> dict[str, Any]:
    scores = [wallet["score"] for wallet in wallet_results if wallet["score"] is not None]
    if not wallet_results:
        state = "no_wallets"
    elif not scores:
        state = "not_enough_data"
    elif any(wallet["state"] != "full" for wallet in wallet_results):
        state = "partial"
    else:
        state = "full"
    all_findings = [
        finding for wallet in wallet_results for finding in wallet["findings"]
    ] + [finding for tx in tx_results for finding in tx["findings"]]
    risk_weight = _risk_weight(all_findings)
    return {
        "state": state,
        "wallet_count": len(wallet_results),
        "transaction_count": len(tx_results),
        "risk_weight": risk_weight,
        "risk_count": _risk_count(all_findings),
        "unknown_count": _unknown_count(all_findings),
        "risk_level": _risk_level(risk_weight),
        "finding_counts": _finding_counts(all_findings),
        "top_findings": aggregate_findings[:5],
    }


def _coverage(
    wallets: list[sqlite3.Row],
    inventory: list[sqlite3.Row],
    tx_results: list[dict[str, Any]],
) -> dict[str, Any]:
    wallet_inventory = {row["wallet_id"] for row in inventory}
    return {
        "wallet_count": len(wallets),
        "wallets_with_inventory": len(wallet_inventory),
        "inventory_outputs": len(inventory),
        "active_utxos": sum(1 for row in inventory if not _string_or_none(row["spent_at"])),
        "transaction_total": len(tx_results),
        "transaction_full": sum(1 for tx in tx_results if tx["state"] == "full"),
        "transaction_partial": sum(1 for tx in tx_results if tx["state"] == "partial"),
        "transaction_not_analysable": sum(
            1 for tx in tx_results if tx["state"] == "not_analysable"
        ),
        "transaction_scored": sum(1 for tx in tx_results if tx["score"] is not None),
    }


def _returned_transactions(
    tx_results: list[dict[str, Any]],
    limit: int,
) -> list[dict[str, Any]]:
    ranked = sorted(
        tx_results,
        key=lambda tx: (
            tx["score"] is None,
            tx["score"] if tx["score"] is not None else 101,
            -max(
                (
                    FINDING_SEVERITY_RANK.get(item["severity"], 0)
                    for item in tx["findings"]
                ),
                default=0,
            ),
            tx["occurred_at"],
        ),
    )
    return [_public_transaction_result(tx) for tx in ranked[:limit]]


def _public_wallet_result(row: Mapping[str, Any]) -> dict[str, Any]:
    risk_findings = list(row.get("risk_findings") or row["findings"])
    risk_weight = _risk_weight(risk_findings)
    return {
        "id": row["id"],
        "label": row["label"],
        "kind": row["kind"],
        "state": row["state"],
        "transaction_count": row["transaction_count"],
        "scored_transaction_count": row["scored_transaction_count"],
        "inventory_output_count": row["inventory_output_count"],
        "active_utxo_count": row["active_utxo_count"],
        "address": row["address"],
        "risk_weight": risk_weight,
        "risk_count": _risk_count(risk_findings),
        "unknown_count": _unknown_count(risk_findings),
        "risk_level": _risk_level(risk_weight),
        "finding_counts": _finding_counts(risk_findings),
        "top_findings": _aggregate_findings(risk_findings)[:5],
    }


def _public_transaction_result(tx: Mapping[str, Any]) -> dict[str, Any]:
    return {
        "id": tx["id"],
        "external_id": tx["external_id"],
        "wallet_id": tx["wallet_id"],
        "wallet_label": tx["wallet_label"],
        "occurred_at": tx["occurred_at"],
        "direction": tx["direction"],
        "asset": tx["asset"],
        "state": tx["state"],
        "support": tx["support"],
        "risk_weight": _risk_weight(tx["findings"]),
        "risk_count": _risk_count(tx["findings"]),
        "unknown_count": _unknown_count(tx["findings"]),
        "risk_level": _risk_level(_risk_weight(tx["findings"])),
        "finding_counts": tx["finding_counts"],
        "top_findings": _aggregate_findings(tx["findings"])[:8],
    }


def _aggregate_findings(findings: list[Mapping[str, Any]]) -> list[dict[str, Any]]:
    grouped: dict[str, dict[str, Any]] = {}
    for finding in findings:
        code = str(finding["code"])
        current = grouped.setdefault(
            code,
            {
                "code": code,
                "severity": finding["severity"],
                "scope": finding["scope"],
                "count": 0,
                "impact": 0,
                "occurrences": 0,
                "evidence_level": finding["evidence_level"],
                "remediation": finding["remediation"],
                "attribution": finding["attribution"],
                "details": {},
            },
        )
        current["count"] += int(finding.get("count") or 0)
        current["impact"] += int(finding.get("impact") or 0)
        current["occurrences"] += 1
        if FINDING_SEVERITY_RANK.get(
            str(finding["severity"]), 0
        ) > FINDING_SEVERITY_RANK.get(str(current["severity"]), 0):
            current["severity"] = finding["severity"]
            current["evidence_level"] = finding["evidence_level"]
            current["remediation"] = finding["remediation"]
            current["attribution"] = finding["attribution"]
        if not current["details"]:
            current["details"] = dict(finding.get("details") or {})
    return sorted(
        grouped.values(),
        key=lambda item: (
            -FINDING_SEVERITY_RANK.get(item["severity"], 0),
            item["impact"],
            -item["occurrences"],
            item["code"],
        ),
    )


def _finding(
    code: str,
    severity: str,
    impact: int,
    *,
    scope: str,
    count: int,
    details: Mapping[str, Any],
    evidence_level: str | None = None,
    remediation: str | None = None,
    attribution: str = USER_ATTRIBUTION,
) -> dict[str, Any]:
    metadata = FINDING_METADATA.get(code, {})
    resolved_evidence = evidence_level or str(
        metadata.get("evidence_level") or "heuristic"
    )
    if resolved_evidence not in EVIDENCE_LEVELS:
        resolved_evidence = "heuristic"
    resolved_remediation = remediation or str(
        metadata.get("remediation")
        or "Review this local tell manually; Kassiber does not mutate accounting state from privacy heuristics."
    )
    return {
        "code": code,
        "severity": severity,
        "impact": impact,
        "scope": scope,
        "count": count,
        "evidence_level": resolved_evidence,
        "remediation": resolved_remediation,
        "attribution": attribution,
        "details": dict(details),
    }


def _risk_weight(findings: list[Mapping[str, Any]]) -> int:
    return sum(
        max(0, -int(finding.get("impact") or 0))
        for finding in findings
        if finding.get("attribution") == USER_ATTRIBUTION
    )


def _risk_count(findings: list[Mapping[str, Any]]) -> int:
    return sum(
        1
        for finding in findings
        if finding.get("attribution") == USER_ATTRIBUTION
        and int(finding.get("impact") or 0) < 0
    )


def _unknown_count(findings: list[Mapping[str, Any]]) -> int:
    return sum(1 for finding in findings if finding.get("evidence_level") == "unavailable")


def _risk_level(risk_weight: int) -> str:
    if risk_weight >= 45:
        return "critical"
    if risk_weight >= 25:
        return "high"
    if risk_weight >= 10:
        return "medium"
    if risk_weight > 0:
        return "low"
    return "none"


def _finding_counts(findings: list[Mapping[str, Any]]) -> dict[str, int]:
    counts = _empty_finding_counts()
    for finding in findings:
        severity = str(finding.get("severity") or "info")
        if severity not in counts:
            severity = "info"
        counts[severity] += 1
    return counts


def _empty_finding_counts() -> dict[str, int]:
    return {
        "positive": 0,
        "info": 0,
        "low": 0,
        "medium": 0,
        "high": 0,
        "critical": 0,
    }


def _score_from_findings(findings: list[Mapping[str, Any]]) -> int:
    return _clamp_score(BASE_SCORE + sum(int(item["impact"]) for item in findings))


def _clamp_score(score: int) -> int:
    return max(0, min(100, int(round(score))))


def _fee_sats(
    row: sqlite3.Row,
    raw: Mapping[str, Any],
    inputs: list[Mapping[str, Any]],
    outputs: list[Mapping[str, Any]],
) -> int | None:
    explicit = _value_sats_or_none(raw.get("fee_sats"))
    if explicit is None:
        explicit = _value_sats_or_none(raw.get("fee"))
    if explicit is not None:
        return max(0, explicit)
    input_values = [item["value_sats"] for item in inputs if item["value_sats"] is not None]
    output_values = [
        item["value_sats"] for item in outputs
        if item["value_sats"] is not None and not item["op_return"]
    ]
    if len(input_values) == len(inputs) and len(output_values) == len([item for item in outputs if not item["op_return"]]):
        computed = sum(input_values) - sum(output_values)
        if computed >= 0:
            return computed
    row_fee = _int_or_none(row["fee"])
    return None if row_fee is None else max(0, row_fee // 1000)


def _is_round_amount(value_sats: Any) -> bool:
    value = _int_or_none(value_sats)
    if value is None or value < MEANINGFUL_ROUND_AMOUNT_FLOOR_SATS:
        return False
    if value in ROUND_BTC_DENOMINATIONS_SATS:
        return True
    return value % 100_000 == 0 or value % 1_000_000 == 0


def _meaningful_types(values: Any) -> set[str]:
    return {
        value for value in values
        if value not in {"", "unknown", "op_return", "fee", "confidential"}
    }


def _script_type(address: str | None, script: str | None) -> str:
    normalized_script = (script or "").strip().lower()
    if normalized_script.startswith("6a"):
        return "op_return"
    if normalized_script.startswith("5120") and len(normalized_script) == 68:
        return "p2tr"
    if normalized_script.startswith("0014"):
        return "p2wpkh"
    if normalized_script.startswith("0020"):
        return "p2wsh"
    if normalized_script.startswith("a914") and normalized_script.endswith("87"):
        return "p2sh"
    if normalized_script.startswith("76a914") and normalized_script.endswith("88ac"):
        return "p2pkh"
    normalized_address = (address or "").strip().lower()
    if normalized_address.startswith(("bc1p", "tb1p", "bcrt1p")):
        return "p2tr"
    if normalized_address.startswith(("bc1q", "tb1q", "bcrt1q")):
        return "segwit"
    if normalized_address.startswith(("1", "m", "n")):
        return "p2pkh"
    if normalized_address.startswith(("3", "2")):
        return "p2sh"
    if normalized_address.startswith(("ex1", "tex1", "el1")):
        return "confidential"
    return "unknown"


def _is_op_return(entry: Mapping[str, Any], script: str | None) -> bool:
    script_type = str(entry.get("scriptpubkey_type") or entry.get("type") or "").lower()
    return script_type == "op_return" or _script_type(None, script) == "op_return"


def _witness_item_count(entry: Mapping[str, Any]) -> int | None:
    witness = entry.get("witness") or entry.get("txinwitness")
    if isinstance(witness, list):
        return len(witness)
    scriptsig = entry.get("scriptsig") or entry.get("scriptSig")
    if scriptsig:
        return 0
    return None


def _json_obj(value: Any) -> dict[str, Any]:
    if isinstance(value, Mapping):
        return dict(value)
    try:
        parsed = json.loads(value or "{}")
    except (TypeError, ValueError, json.JSONDecodeError):
        return {}
    return dict(parsed) if isinstance(parsed, Mapping) else {}


def _value_sats_or_none(value: Any) -> int | None:
    if value is None or value == "" or isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value
    if isinstance(value, float):
        return int(round(value * 100_000_000))
    if isinstance(value, str):
        text = value.strip()
        if not text:
            return None
        try:
            if "." in text:
                return int(round(float(text) * 100_000_000))
            return int(text)
        except ValueError:
            return None
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _int_or_none(value: Any) -> int | None:
    if value is None or value == "" or isinstance(value, bool):
        return None
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _string_or_none(value: Any) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _txid_or_none(value: Any) -> str | None:
    text = _string_or_none(value)
    if text is None:
        return None
    normalized = text.lower()
    if len(normalized) != 64:
        return None
    try:
        bytes.fromhex(normalized)
    except ValueError:
        return None
    return normalized


def _hex_or_none(value: Any) -> str | None:
    text = _string_or_none(value)
    if text is None:
        return None
    normalized = text.lower()
    if len(normalized) % 2 != 0:
        return None
    try:
        bytes.fromhex(normalized)
    except ValueError:
        return None
    return normalized
