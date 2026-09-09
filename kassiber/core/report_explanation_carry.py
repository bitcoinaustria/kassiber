"""Read retained RP2 calculations behind synthetic carrying-value acquisitions.

These are whole outgoing calculations, not reconstructed allocations of their
historical lots to the later sale. Canonical eligible relations are the only
links; raw txids, equal amounts and same-asset pools confer no authority.
"""
from __future__ import annotations

import json
from decimal import Decimal, InvalidOperation, localcontext

_FIELDS = ("cost_basis_exact", "proceeds_exact", "gain_loss_exact")
_MAX_DEPTH = 8
_MAX_RECORDS = 100


def retained_calculation(row):
    """Validate retained contributions without running or emulating the engine."""
    if not row["calculation_json"] or any(row[field] is None for field in _FIELDS):
        return "engine_detail_unavailable", None
    try:
        calculation = json.loads(row["calculation_json"])
        fragments = calculation["fragments"]
        with localcontext() as context:
            context.prec = 32  # Existing RP2 calculation precision.
            reconciled = all(
                sum((Decimal(item[field]) for item in fragments), Decimal(0)) == Decimal(row[field])
                for field in _FIELDS
            )
        reconciled &= sum(item["quantity_msat"] for item in fragments) == abs(int(row["quantity"]))
        if reconciled:
            return "available", calculation
    except (ValueError, TypeError, KeyError, InvalidOperation):
        pass
    return "engine_detail_mismatch", None


def attach_inherited_basis(conn, profile_id, calculation):
    """Enrich RAM-only lot evidence inside the caller's pinned read snapshot."""
    remaining = _MAX_RECORDS
    retained = {}
    truncated = False

    def visit(current, ancestors, depth):
        nonlocal remaining, truncated
        for fragment in current["fragments"]:
            lot = fragment.get("lot")
            if not lot or lot.get("pricing", {}).get("pricing_method") != "carrying_value":
                continue
            detail = {"status": "engine_detail_unavailable", "relations": [], "source_calculations": []}
            lot["inherited_basis"] = detail
            target = lot.get("transaction_id")
            if target in ancestors or depth >= _MAX_DEPTH or remaining <= 0:
                detail["status"] = "inherited_detail_truncated"
                truncated = True
                continue
            relations = conn.execute("""
                SELECT id AS decision_id, relation_kind AS custody_state,
                       kind AS reason, policy, basis_state, component_id,
                       out_transaction_id AS source_transaction_id,
                       in_transaction_id AS target_transaction_id,
                       out_asset AS source_asset, in_asset AS target_asset,
                       CAST(out_amount AS TEXT) AS source_quantity_msat_exact,
                       CAST(in_amount AS TEXT) AS target_quantity_msat_exact,
                       CAST(swap_fee_msat AS TEXT) AS swap_fee_msat_exact,
                       occurred_at, target_occurred_at
                FROM journal_custody_projection_relations
                WHERE profile_id = ? AND in_transaction_id = ? AND in_asset = ?
                  AND policy = 'carrying-value' AND basis_state = 'eligible'
                ORDER BY occurred_at, id LIMIT ?
            """, (profile_id, target, lot["asset"], remaining + 1)).fetchall()
            if len(relations) > remaining:
                detail["status"] = "inherited_detail_truncated"
                truncated = True
            sources = set()
            for relation in relations[:remaining]:
                if remaining <= 0:
                    detail["status"] = "inherited_detail_truncated"
                    truncated = True
                    break
                remaining -= 1
                value = dict(relation)
                detail["relations"].append(value)
                retained[value["decision_id"]] = value
                source_key = (value["source_transaction_id"], value["source_asset"])
                if source_key in sources:
                    continue
                sources.add(source_key)
                rows = conn.execute("""
                    SELECT * FROM journal_entries
                    WHERE profile_id = ? AND transaction_id = ? AND asset = ?
                      AND entry_type = 'disposal'
                    ORDER BY occurred_at, id LIMIT ?
                """, (profile_id, *source_key, remaining + 1)).fetchall()
                if len(rows) > remaining:
                    detail["status"] = "inherited_detail_truncated"
                    truncated = True
                for row in rows[:remaining]:
                    if remaining <= 0:
                        detail["status"] = "inherited_detail_truncated"
                        truncated = True
                        break
                    remaining -= 1
                    status, inherited = retained_calculation(row)
                    source = {
                        "entry_id": row["id"], "transaction_id": row["transaction_id"],
                        "asset": row["asset"], "scope": "whole_source_disposal",
                        "status": status, "calculation": inherited,
                        "totals": {**{field: row[field] for field in _FIELDS},
                                   "quantity_msat_exact": str(abs(int(row["quantity"])))},
                    }
                    detail["source_calculations"].append(source)
                    if inherited is not None:
                        visit(inherited, ancestors | {target}, depth + 1)
                if not rows:
                    detail["source_calculations"].append({
                        "transaction_id": source_key[0], "asset": source_key[1],
                        "scope": "whole_source_disposal", "status": "engine_detail_unavailable",
                        "calculation": None, "totals": None,
                    })
            if detail["status"] != "inherited_detail_truncated" and detail["source_calculations"]:
                statuses = {source["status"] for source in detail["source_calculations"]}
                detail["status"] = "available" if statuses == {"available"} else (
                    "engine_detail_mismatch" if "engine_detail_mismatch" in statuses else "engine_detail_unavailable"
                )

    visit(calculation, set(), 0)
    return list(retained.values()), truncated
