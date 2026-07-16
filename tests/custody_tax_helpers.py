"""Test adapter for exercising the strict custody-to-tax boundary end to end."""

from __future__ import annotations

from dataclasses import dataclass
import json
import sqlite3
from typing import Any, Mapping, Sequence

from kassiber.core.chain_observer.provenance import (
    AUTHORITY_VERSION,
    canonical_graph_hash,
    canonical_observed_quantity_hash,
    persist_chain_observation_provenance,
)
from kassiber.core.custody_evidence import (
    build_canonical_quantity_input,
    enriched_quantity_rows,
)
from kassiber.core.custody_interpreters import compile_custody_interpreters
from kassiber.core.custody_quantity_runtime import build_canonical_quantity_state
from kassiber.core.custody_tax_projection import compile_finalized_tax_projection
from kassiber.core.engines.base import TaxEngineLedgerInputs
from kassiber.core.loans import CHANNEL_CLOSE, CHANNEL_OPEN


@dataclass(frozen=True)
class CanonicalTaxTestInputs(TaxEngineLedgerInputs):
    """Finalized engine input plus the raw fixtures used to compile it.

    A few regression tests derive a second scenario from a shared fixture.
    Keeping that source material explicitly test-only avoids restoring raw-row
    fields on the production tax-engine boundary.
    """

    source_rows: tuple[Mapping[str, Any], ...] = ()
    source_manual_pair_records: tuple[Mapping[str, Any], ...] = ()


def _raw_mapping(value: Any) -> dict[str, Any]:
    try:
        return json.loads(value or "{}") if isinstance(value, str) else dict(value or {})
    except (TypeError, ValueError, json.JSONDecodeError):
        return {}


def authoritative_chain_observation(
    row: Mapping[str, Any],
    *,
    observer_kind: str = "bitcoinrpc",
    fee_attribution: str = "exact",
) -> dict[str, Any]:
    """Return one in-memory row joined to closed observer provenance.

    Tests must opt in explicitly. Merely placing ``observer=bdk`` or similar
    text in ``raw_json`` is intentionally insufficient, which keeps generic
    importer/provider and fake-marker fixtures non-authoritative.
    """

    item = dict(row)
    raw = _raw_mapping(item.get("raw_json"))
    raw.setdefault("observer", observer_kind)
    if str(raw.get("chain") or "").lower() == "liquid":
        component = raw.get("component")
        component = dict(component) if isinstance(component, Mapping) else {}
        component.setdefault("fee_attribution", fee_attribution)
        raw["component"] = component
    item["raw_json"] = json.dumps(raw, sort_keys=True)
    item["observation_authority_version"] = AUTHORITY_VERSION
    item["observation_graph_hash"] = canonical_graph_hash(item["raw_json"])
    item["observation_quantity_hash"] = canonical_observed_quantity_hash(item)
    item["observation_fee_attribution"] = fee_attribution
    return item


def persist_authoritative_chain_observation(
    conn: sqlite3.Connection,
    transaction_id: str,
    *,
    observer_kind: str = "bitcoinrpc",
    fee_attribution: str = "exact",
) -> None:
    """Persist production-shaped authority for an intentional DB fixture."""

    row = conn.execute(
        "SELECT * FROM transactions WHERE id = ?", (transaction_id,)
    ).fetchone()
    if row is None:
        raise AssertionError(f"missing transaction fixture {transaction_id}")
    authoritative = authoritative_chain_observation(
        row,
        observer_kind=observer_kind,
        fee_attribution=fee_attribution,
    )
    conn.execute(
        "UPDATE transactions SET raw_json = ? WHERE id = ?",
        (authoritative["raw_json"], transaction_id),
    )
    profile = conn.execute(
        "SELECT * FROM profiles WHERE id = ?", (row["profile_id"],)
    ).fetchone()
    wallet = conn.execute(
        "SELECT * FROM wallets WHERE id = ?", (row["wallet_id"],)
    ).fetchone()
    if profile is None or wallet is None:
        raise AssertionError("authoritative fixture requires profile and wallet rows")
    raw = _raw_mapping(authoritative["raw_json"])
    persist_chain_observation_provenance(
        conn,
        profile,
        wallet,
        application_revision=f"test:{transaction_id}",
        chain=str(raw.get("chain") or "bitcoin"),
        network=str(raw.get("network") or "main"),
        entries=(
            {
                "external_id": row["external_id"],
                "asset": row["asset"],
                "direction": row["direction"],
                "observer_ids": [f"test:{observer_kind}"],
                "observer_kinds": [observer_kind],
            },
        ),
    )


def _field(row: Mapping[str, Any], key: str, default: Any = None) -> Any:
    if hasattr(row, "keys") and key not in row.keys():
        return default
    if hasattr(row, "get"):
        return row.get(key, default)
    return row[key]


def finalized_tax_inputs(
    profile: Mapping[str, Any],
    *,
    rows: Sequence[Mapping[str, Any]],
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]],
    manual_pair_records: Sequence[Mapping[str, Any]] = (),
    owned_index: Any = None,
    channel_roles: Mapping[str, str] | None = None,
    channel_transfer_pairs: Sequence[Mapping[str, Any]] = (),
    loan_legs: Sequence[Mapping[str, Any]] = (),
    direct_payout_records: Sequence[Mapping[str, Any]] = (),
) -> CanonicalTaxTestInputs:
    """Compile raw test fixtures through the same pre-tax custody stages.

    Engine tests used to inject imported transaction rows directly into RP2.
    Keeping this adapter in the test tree preserves their behavioral coverage
    without reopening that production bypass.
    """

    # Match the production handler boundary: canonical quantity receives the
    # enriched/fee-normalized view, while interpreters and projection retain the
    # imported rows whose provenance was persisted at insertion time.
    prepared_rows = [dict(row) for row in rows]
    rows_by_id = {str(_field(row, "id") or ""): row for row in prepared_rows}
    enriched_direct_payout_records = []
    for record in direct_payout_records:
        enriched = dict(record)
        source = rows_by_id.get(
            str(_field(record, "out_transaction_id") or ""), {}
        )
        enriched.setdefault("out_asset", _field(source, "asset"))
        enriched_direct_payout_records.append(enriched)

    canonical = build_canonical_quantity_input(enriched_quantity_rows(prepared_rows))
    interpreters = compile_custody_interpreters(
        prepared_rows,
        canonical,
        wallet_refs_by_id=wallet_refs_by_id,
        manual_pair_records=manual_pair_records,
        owned_index=owned_index,
        channel_transfer_pairs=channel_transfer_pairs,
        channel_roles=channel_roles,
        loan_legs=loan_legs,
        direct_payout_records=enriched_direct_payout_records,
    )
    complete_channel_transaction_ids = {
        str(transaction_id)
        for transaction_id, role in (channel_roles or {}).items()
        if role in {CHANNEL_OPEN, CHANNEL_CLOSE}
    }
    ignored_gap_transaction_ids = {
        str(record[key])
        for record in manual_pair_records
        for key in ("out_transaction_id", "in_transaction_id")
        if record.get(key) not in (None, "")
    }
    ignored_gap_transaction_ids.update(
        str(leg["transaction_id"])
        for leg in loan_legs
        if leg.get("transaction_id") not in (None, "")
    )
    ignored_gap_transaction_ids.update(
        str(record["out_transaction_id"])
        for record in enriched_direct_payout_records
        if record.get("out_transaction_id") not in (None, "")
    )
    ignored_gap_transaction_ids.update(complete_channel_transaction_ids)
    state = build_canonical_quantity_state(
        prepared_rows,
        interpreter_claims=interpreters.claims,
        native_evidence=interpreters.native_audits,
        direct_payout_records=enriched_direct_payout_records,
        interpreter_blockers=interpreters.blocking_quarantines,
        ignored_gap_transaction_ids=(
            *ignored_gap_transaction_ids,
            *interpreters.blocked_transaction_ids,
        ),
    )
    projection = compile_finalized_tax_projection(
        profile,
        prepared_rows,
        state,
        non_event_transaction_ids=(
            *interpreters.non_event_transaction_ids,
            *complete_channel_transaction_ids,
        ),
        blocked_transaction_ids=interpreters.blocked_transaction_ids,
        direct_payout_conflict_transaction_ids=(
            interpreters.direct_payout_conflict_transaction_ids
        ),
        interpreter_quarantines=interpreters.quarantines,
        direct_payout_records=enriched_direct_payout_records,
        reviewed_cross_asset_pairs=interpreters.cross_asset_pairs,
    )
    return CanonicalTaxTestInputs(
        finalized_tax_projection=projection,
        wallet_refs_by_id=wallet_refs_by_id,
        direct_payout_records=enriched_direct_payout_records,
        source_rows=tuple(prepared_rows),
        source_manual_pair_records=tuple(manual_pair_records),
    )


__all__ = [
    "CanonicalTaxTestInputs",
    "authoritative_chain_observation",
    "finalized_tax_inputs",
    "persist_authoritative_chain_observation",
]
