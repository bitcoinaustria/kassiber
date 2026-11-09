"""Pure custody computation over versioned raw facts (no SQL, I/O or clock)."""
from __future__ import annotations
from dataclasses import fields, replace
from . import custody_components, custody_authored_migration, custody_quantity_runtime
from . import custody_interpreters, custody_tax_projection, custody_gaps, loans, ownership_transfers
from .custody_evidence import enriched_quantity_rows, build_canonical_quantity_input
from .custody_component_facts import ComponentFacts
from .custody_snapshot import decode_owned_index, require_snapshot_accounting
from ..tax_policy import require_tax_processing_supported

STAGES = ("components", "evidence", "interpretation", "gaps", "quantity", "finalized_projection")


def compute_evidence_stage(snapshot, *, rows=None, owned_index=None):
    from .source_overlap import ProfileSourceIndex, SourceScript, reconcile_receipt_quantities
    rows = snapshot["transactions"] if rows is None else rows
    if owned_index is None:
        owned_index = decode_owned_index(snapshot["owned_index"])
    non_events = tuple(sorted(ownership_transfers.native_zero_delta_ids(rows, owned_index)))
    enriched = enriched_quantity_rows(rows, evidence_only=True)
    reconciliations = reconcile_receipt_quantities(
        None, str(snapshot["profile"]["id"]), rows, owned_index=owned_index,
        profile_source_index=ProfileSourceIndex(str(snapshot["profile"]["id"]), tuple(SourceScript(**item) for item in snapshot["receipt_sources"])),
    )
    canonical = build_canonical_quantity_input(enriched, non_event_transaction_ids=non_events, receipt_reconciliations=reconciliations)
    return {"enriched_rows": enriched, "canonical_input": canonical, "receipt_reconciliations": reconciliations, "native_non_event_ids": non_events}


def compute_custody_decisions(snapshot, *, stage_observer=None, owned_index_observer=None):
    from .custody_journal import (CustodyJournalDecisions, duplicate_label_warnings, rates_from_transaction_rows, channel_context_from_records)
    if snapshot["schema_version"] != 1 or snapshot["operation"] != "custody":
        raise ValueError("unsupported custody snapshot schema or operation")
    def emit(stage, result):
        if stage_observer is not None:
            stage_observer(stage, result)
    require_snapshot_accounting(snapshot)
    profile = snapshot["profile"]
    profile_id = str(profile["id"])
    as_of = snapshot["as_of"]
    require_tax_processing_supported(profile)
    rows = [{**row, "_closed_confirmation_observation": ownership_transfers.capture_confirmation_observation(row)} for row in snapshot["transactions"]]
    rates = rates_from_transaction_rows(snapshot["rate_rows"])
    wallet_refs_by_id = {wallet["id"]: {key: value for key, value in wallet.items() if key not in {"config_json", "config_text_mentions_samourai"}} for wallet in snapshot["wallets"]}
    warnings = duplicate_label_warnings(wallet_refs_by_id)
    warnings.extend({"code": "ownership_index", "message": str(message)} for message in snapshot["ownership_warnings"])
    owned_index = decode_owned_index(snapshot["owned_index"])
    if owned_index_observer is not None:
        owned_index = owned_index_observer(owned_index)
    loan_legs = snapshot["loan_legs"]
    channel_roles, channel_transfer_pairs = channel_context_from_records(snapshot["channel_records"], rows, wallet_refs_by_id)
    active_components, component_blockers = compute_components_stage(snapshot)
    migration_quarantines = (
        *custody_authored_migration.migration_quarantines_from_rows(snapshot["migration_issues"]),
        *snapshot.get("import_duplicate_quarantines", ()),
        *snapshot.get("reset_hold_quarantines", ()),
    )
    evidence_snapshots = {}
    for row in snapshot["evidence_snapshots"]:
        evidence_snapshots.setdefault(str(row["subject_id"]), []).append(dict(row))
    evidence_snapshots = {key: tuple(value) for key, value in evidence_snapshots.items()}
    from .custody_gap_reviews import dismissed_fingerprints_from_rows
    dismissed_fingerprints = dismissed_fingerprints_from_rows(snapshot["gap_reviews"])
    emit("components", {"components": active_components, "blockers": component_blockers})
    ignored_gap_ids: set[str] = set()
    ignored_gap_ids.update(
        str(leg["transaction_id"])
        for leg in loan_legs
        if leg["transaction_id"] not in (None, "")
    )
    ignored_gap_ids.update(str(item) for item in channel_roles)
    for pair in channel_transfer_pairs:
        for side in ("out", "in"):
            row = pair.get(side) or {}
            transaction_id = row.get("journal_transaction_id") or row.get("id")
            if transaction_id not in (None, ""):
                ignored_gap_ids.add(str(transaction_id))

    component_transaction_ids = tuple(
        sorted(
            {
                str(leg.get("anchor_transaction_id") or leg.get("transaction_id"))
                for component in active_components
                for leg in component.get("legs", ())
                if leg.get("anchor_transaction_id") or leg.get("transaction_id")
            }
        )
    )
    evidence = compute_evidence_stage(snapshot, rows=rows, owned_index=owned_index)
    native_non_event_ids = set(evidence["native_non_event_ids"])
    enriched_rows = evidence["enriched_rows"]
    canonical_input = evidence["canonical_input"]
    emit("evidence", evidence)
    swap_dismissals = snapshot["pair_dismissals"]
    interpretation = custody_interpreters.compile_custody_interpreters(
        rows,
        canonical_input,
        profile=profile,
        swap_dismissals=swap_dismissals,
        wallet_refs_by_id=wallet_refs_by_id,
        owned_index=owned_index,
        channel_transfer_pairs=channel_transfer_pairs,
        channel_roles=channel_roles,
        loan_legs=loan_legs,
        component_transaction_ids=component_transaction_ids,
        components=active_components,
        as_of=as_of,
    )
    if native_non_event_ids:
        interpretation = replace(
            interpretation,
            non_event_transaction_ids=tuple(sorted({
                *interpretation.non_event_transaction_ids, *native_non_event_ids,
            })),
        )
    if migration_quarantines:
        interpretation = replace(
            interpretation,
            blocked_transaction_ids=tuple(
                sorted(
                    {
                        *interpretation.blocked_transaction_ids,
                        *(
                            str(item["transaction_id"])
                            for item in migration_quarantines
                        ),
                    }
                )
            ),
            quarantines=(
                *interpretation.quarantines,
                *migration_quarantines,
            ),
        )
    projection_ignored_ids = {
        *ignored_gap_ids,
        *component_transaction_ids,
    }
    observations_by_hash = {
        item.quantity_hash: item for item in canonical_input.observations
    }
    for claim in interpretation.claims:
        # Gap discovery must retain unmatched/suspense sources so the
        # review queue can explain a missing-wallet hypothesis. Only an
        # authoritative target-bearing custody edge suppresses both
        # boundaries from the shared candidate population.
        if claim.target is None:
            continue
        source = observations_by_hash.get(claim.source.observation_hash)
        if source is not None:
            projection_ignored_ids.add(source.transaction_id)
        target = observations_by_hash.get(claim.target.observation_hash)
        if target is not None:
            projection_ignored_ids.add(target.transaction_id)
    projection_ignored_transaction_ids = tuple(
        sorted(projection_ignored_ids)
    )
    runtime_ignored_transaction_ids = tuple(
        sorted(
            {
                *projection_ignored_ids,
                *interpretation.blocked_transaction_ids,
            }
        )
    )

    emit("interpretation", interpretation)
    gap_search_result, _gap_legs = custody_gaps.compute_gap_search_result(
        snapshot["gap_worklist"]["rows"],
        large_book=snapshot["gap_worklist"]["large_book"],
        ignored_transaction_ids=projection_ignored_transaction_ids,
        accounting_ignored_transaction_ids=runtime_ignored_transaction_ids,
    )
    emit("gaps", gap_search_result)
    quantity_state = custody_quantity_runtime.build_canonical_quantity_state(
        enriched_rows,
        canonical_input=canonical_input,
        interpreter_claims=interpretation.claims,
        effective_components=[
            component for component in active_components
            if str(component["id"]) not in interpretation.native_reconciled_component_ids
        ],
        native_evidence=interpretation.native_audits,
        interpreter_blockers=interpretation.blocking_quarantines,
        ignored_gap_transaction_ids=runtime_ignored_transaction_ids,
        component_evidence_snapshots=evidence_snapshots,
        dismissed_gap_fingerprints=dismissed_fingerprints,
        gap_search_result=gap_search_result,
    )
    emit("quantity", quantity_state)
    direct_payout_records = list(quantity_state.reviewed_direct_payouts)
    custody_transfers = custody_quantity_runtime.canonical_internal_transfer_rows(
        quantity_state,
        wallet_refs_by_id,
    )
    channel_non_event_ids = tuple(
        sorted(
            str(transaction_id)
            for transaction_id, role in channel_roles.items()
            if str(role) in {loans.CHANNEL_OPEN, loans.CHANNEL_CLOSE}
        )
    )
    return CustodyJournalDecisions(
        rows=rows,
        direct_payout_records=direct_payout_records,
        rates=rates,
        wallet_refs_by_id=wallet_refs_by_id,
        warnings=warnings,
        owned_index=owned_index,
        loan_legs=loan_legs,
        channel_roles=channel_roles,
        active_components=active_components,
        component_blockers=component_blockers,
        interpretation=interpretation,
        quantity_state=quantity_state,
        custody_transfers=custody_transfers,
        channel_non_event_ids=channel_non_event_ids,
        gap_ignored_transaction_ids=projection_ignored_transaction_ids,
        gap_accounting_ignored_transaction_ids=runtime_ignored_transaction_ids,
        as_of=as_of,
    )



def compute_custody_projection(snapshot, *, stage_observer=None, owned_index_observer=None):
    from .custody_journal import CustodyJournalDecisions, CustodyJournalProjection
    decisions = compute_custody_decisions(snapshot, stage_observer=stage_observer, owned_index_observer=owned_index_observer)
    finalized = custody_tax_projection.compile_finalized_tax_projection(
        snapshot["profile"], decisions.rows, decisions.quantity_state,
        non_event_transaction_ids=(*decisions.interpretation.non_event_transaction_ids, *decisions.channel_non_event_ids),
        blocked_transaction_ids=decisions.interpretation.blocked_transaction_ids,
        interpreter_quarantines=decisions.interpretation.quarantines,
        direct_payout_records=decisions.direct_payout_records,
    )
    if stage_observer is not None:
        stage_observer("finalized_projection", finalized)
    return CustodyJournalProjection(**{field.name: getattr(decisions, field.name) for field in fields(CustodyJournalDecisions)}, finalized_tax_projection=finalized)


def compute_components_stage(snapshot):
    from .custody_journal import component_integrity_blockers
    facts = ComponentFacts(snapshot)
    profile_id = str(snapshot["profile"]["id"])
    components = list(custody_components.iter_authored_active_components(facts, profile_id=profile_id, include_local_evidence=False))
    return components, component_integrity_blockers(facts, profile_id, components=components)
