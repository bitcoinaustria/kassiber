"""Exact bounded acquisition review effects from the canonical ledger output."""
from decimal import Decimal
import json
from ..db import database_instance_id


def _decimal_text(value):
    text = format(value, "f")
    return text.rstrip("0").rstrip(".") if "." in text else text


def require_acquisition_target(conn, profile_id, transaction):
    from ..errors import AppError
    from .custody_authored_migration import find_active_review_for_transaction
    if transaction["direction"] != "inbound":
        raise AppError("Acquisition review requires an inbound transaction", code="validation")
    if find_active_review_for_transaction(conn, profile_id=profile_id, transaction_id=transaction["id"]):
        raise AppError("This transaction belongs to an active custody review",
                       code="acquisition_custody_review_required",
                       hint="Reopen or supersede that custody interpretation before classifying the acquisition.")


def acquisition_effects(state, conn, profile):
    current = conn.execute("SELECT * FROM profiles WHERE id = ?", (profile["id"],)).fetchone()
    totals = {}
    for entry in state["entries"]:
        asset = entry["asset"]
        row = totals.setdefault(asset, {key: Decimal(0) for key in
            ("acquisition_basis", "income", "disposal_basis", "proceeds", "gain_loss")})
        if entry["entry_type"] == "acquisition":
            row["acquisition_basis"] += entry["fiat_value"] or Decimal(0)
        elif entry["entry_type"] == "income":
            row["income"] += entry["fiat_value"] or Decimal(0)
        elif entry["entry_type"] in {"disposal", "fee", "transfer_fee"}:
            for target, source in (("disposal_basis", "cost_basis"), ("proceeds", "proceeds"), ("gain_loss", "gain_loss")):
                row[target] += entry.get(source) or Decimal(0)
    return {
        "acquisition_context": {
            "database_instance_id": database_instance_id(conn),
            "profile_policy": {key: current[key] for key in current.keys()
                               if key.startswith("tax_") or key in ("gains_algorithm", "fiat_currency", "cost_basis_pool_scope", "require_coarse_review", "bitcoin_rail_carrying_value")},
        },
        "accounting_totals": [{"asset": asset, **{key: _decimal_text(value) for key, value in values.items()}}
                              for asset, values in sorted(totals.items())],
    }


VALUATION_MIGRATION = "at-acquisition-valuation-support-v1"


def migrate_valuation_support(conn):
    """Invalidate pre-contract Austrian journals once, retaining their evidence."""
    if conn.execute("SELECT 1 FROM schema_migration_audits WHERE migration_name=?", (VALUATION_MIGRATION,)).fetchone():
        return False
    affected = """
        lower(trim(tax_country)) = 'at'
        AND (last_processed_at IS NOT NULL OR EXISTS (
            SELECT 1 FROM journal_entries WHERE profile_id = profiles.id))
        AND EXISTS (SELECT 1 FROM transactions t WHERE t.profile_id = profiles.id
            AND t.direction = 'inbound' AND coalesce(t.excluded,0) = 0
            AND replace(replace(lower(trim(coalesce(nullif(t.kind_override,''),t.kind,''))),'-','_'),' ','_')
                IN ('airdrop','hardfork','hard_fork'))
    """
    count = conn.execute(f"SELECT count(*) FROM profiles WHERE {affected}").fetchone()[0]
    inserted = conn.execute(
        "INSERT OR IGNORE INTO schema_migration_audits(id,migration_name,schema_version,impact_json,created_at) "
        "VALUES(?,?,1,?,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        (VALUATION_MIGRATION, VALUATION_MIGRATION,
         json.dumps({"schema_version": 1, "affected_profile_count": count,
                     "explanation": "Rebuild Austrian airdrop/hardfork journals with the explicit unsupported-valuation barrier."})),
    )
    if not inserted.rowcount:
        return False
    conn.execute(f"UPDATE profiles SET last_processed_at=NULL,last_processed_tx_count=0, "
                 f"journal_input_version=journal_input_version+1,ownership_review_counts_json=NULL WHERE {affected}")
    return True
