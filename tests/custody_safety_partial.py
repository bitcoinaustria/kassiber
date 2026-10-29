"""Python reference for the composed pre-arbitration safety checkpoint.

The ported producers include native HTLC production and reservations; native
component reconciliation remains a later slice. Input stays snapshot v1.
"""
import json

from kassiber.core import custody_interpreters as reference
from kassiber.core.privacy_hops import privacy_hop_evidence_from_row


def privacy_result(rows, auto, derived, observations, wallets, excluded, candidates=(), native_ids=()):
    resolved = set(candidates)
    resolved.update(reference._exact_native_pair_ids(auto, observations))
    resolved.update(reference._exact_native_pair_ids(derived, observations))
    resolved.update(
        reference._anchor_id(row) for row in rows
        if row.get("observation_fee_attribution") == "implicit_wallet_delta"
        and (observation := observations.get(reference._anchor_id(row))) is not None
        and observation.authoritative_chain_observation
        and observation.fee_attribution == "exact"
        and observation.principal_msat == 0 and observation.fee_msat > 0
    )
    resolved.update(excluded)
    resolved.update(native_ids)
    quarantines = []
    if any(row.get("privacy_boundary") not in (None, "") for row in rows):
        for row in rows:
            transaction_id = str(row.get("id") or "")
            if transaction_id in resolved or privacy_hop_evidence_from_row(row) is None:
                continue
            detail = dict(wallet=wallets.get(str(row.get("wallet_id") or ""), {}).get("label", row.get("wallet_id")),
                          asset=str(row.get("asset") or "").upper(), direction=row.get("direction"),
                          **(privacy_hop_evidence_from_row(row) or {}))
            quarantines.append(dict(transaction_id=transaction_id, workspace_id=row.get("workspace_id"),
                                    profile_id=row.get("profile_id"), reason="privacy_hop_unresolved",
                                    detail_json=json.dumps(detail, sort_keys=True)))
    return dict(resolved_ids=sorted(resolved), blocked_transaction_ids=sorted({item["transaction_id"] for item in quarantines}), quarantines=quarantines)


def safety_results(snapshot, observations, excluded, canonical):
    from tests.custody_interpretation_partial import pipeline_results
    from kassiber.transfers import onchain_transfer_scope
    try:
        rows = snapshot["transactions"]
        from kassiber.core.custody_journal import channel_context_from_records
        wallets = {str(wallet["id"]): wallet for wallet in snapshot["wallets"]}
        channel_roles, channel_pairs = channel_context_from_records(snapshot["channel_records"], rows, wallets)
        by_id = {str(row.get("id") or ""): row for row in rows}
        channel_quarantines = [dict(transaction_id=id, workspace_id=by_id.get(id, {}).get("workspace_id"), profile_id=by_id.get(id, {}).get("profile_id"), reason="channel_open_unresolved" if role == "channel_open_mismatch" else "channel_close_unresolved", detail_json=json.dumps(dict(channel_role=role, required_for="complete_channel_lifecycle"), sort_keys=True)) for id, role in sorted(channel_roles.items()) if role in {"channel_open_mismatch", "channel_close_mismatch"}]
        channel_blocked = {item["transaction_id"] for item in channel_quarantines}
        present = any(row.get("config_text_mentions_samourai", "samourai" in str(row.get("config_json") or "").lower()) for row in rows)
        samourai, fees, candidates, touched = reference._samourai_privacy_pairs(rows, observations) if present else ([], [], set(), set())
        has_chain = any(event.event_key.native_namespace == "chain" for event in canonical.events)
        auto = reference.detect_intra_transfers(rows)[0] if has_chain or any(row.get("payment_hash") not in (None, "") for row in rows) else []
        auto_original = auto
        from kassiber.core.custody_native_transitions import compile_native_transitions, has_native_transition_evidence
        occupied = set(excluded) | touched | set(channel_roles) | reference._complete_recorded_pair_ids(auto, observations)
        occupied.update(str(leg["transaction_id"]) for leg in snapshot["loan_legs"] if leg.get("transaction_id"))
        occupied.update(id for pair in channel_pairs for id in reference._pair_anchor_ids(pair))
        native_pairs, proofs, native_quarantines = compile_native_transitions(rows, observations, profile=snapshot["profile"], occupied_ids=occupied, dismissals=snapshot["pair_dismissals"], as_of=snapshot["as_of"]) if has_native_transition_evidence(rows) else ([], {}, ())
        native_ids = {id for key in proofs for id in key}
        native_blocked = {item["transaction_id"] for item in native_quarantines}
        prepared = pipeline_results(snapshot, observations, excluded, canonical, touched, production_only=True, recorded_pairs=auto, reserved=native_ids|native_blocked)
        if "error" in prepared:
            return prepared
        derived = [*prepared["derived_pairs"], *channel_pairs]
        privacy = privacy_result(rows, auto, derived, observations, wallets, excluded, candidates, native_ids)
        derivation_blocked = {item["transaction_id"] for item in prepared["derivation_quarantines"]}
        derivation_touched = derivation_blocked | {id for pair in derived for id in reference._pair_anchor_ids(pair)}
        auto = [pair for pair in auto if not reference._pair_anchor_ids(pair) & derivation_touched]
        pair_inputs = [*auto, *samourai, *derived, *native_pairs]
        unverified = candidates - touched - set(excluded)
        samourai_quarantines = [dict(transaction_id=id, workspace_id=by_id.get(id, {}).get("workspace_id"), profile_id=by_id.get(id, {}).get("profile_id"), reason="samourai_native_event_unverified", detail_json=json.dumps(dict(required_for="authoritative_same_event_observation", resolution="sync every imported Samourai source with a chain observer"), sort_keys=True)) for id in sorted(unverified)]
        other_blocked = channel_blocked | derivation_blocked | set(privacy["blocked_transaction_ids"]) | unverified | native_blocked
        resolved = occupied | touched | native_ids
        resolved.update(channel_roles)
        resolved.update(str(leg.get("transaction_id")) for leg in snapshot["loan_legs"] if leg.get("transaction_id"))
        resolved.update(id for pair in channel_pairs for id in reference._pair_anchor_ids(pair))
        non_events = sorted({str(leg.get("transaction_id") or "") for leg in snapshot["loan_legs"] if leg.get("transaction_id") not in (None, "")} | {id for id, role in channel_roles.items() if role in {"channel_open", "channel_close"}})
        holds = reference._raw_observation_holds(rows, prepared["derivation_rows"], pair_inputs,
            wallet_refs_by_id=wallets, has_chain_events=any(event.event_key.native_namespace == "chain" for event in canonical.events),
            excluded_ids=set(excluded), blocked_ids=other_blocked, resolved_ids=resolved)
        kept = reference._without_held_pairs(pair_inputs, set(holds.blocked_ids))
        derived = reference._without_held_pairs(derived, set(holds.blocked_ids))
        claims, blocked, quarantines = reference._pair_claims(kept, observations, excluded_transaction_ids=set(excluded),
            wallet_refs_by_id=wallets, native_transition_proofs=proofs, physical_scopes_by_anchor={reference._anchor_id(row): scope for row in rows if (scope := onchain_transfer_scope(row)) is not None})
        from tests.custody_native_partial import candidate_projection
        native_trace=dict(occupied_ids=sorted(occupied),pairs=native_pairs,proofs=[dict(key=key,proof=dict(candidate=candidate_projection(p.candidate),source_quantity_hash=p.source_quantity_hash,target_quantity_hash=p.target_quantity_hash,policy=p.policy)) for key,p in proofs.items()],quarantines=native_quarantines)
        return dict(native_transitions=native_trace,handled_ids=prepared['handled_ids'],non_event_transaction_ids=non_events, privacy=privacy, holds={key: sorted(value) if isinstance(value, frozenset) else value for key, value in vars(holds).items()}, pair_inputs_before_holds=pair_inputs, pair_inputs=kept, derived_pairs=derived,
                    claims=(*claims, *fees), native_audits=reference._native_audits_for_rowless_pairs(derived, observations, excluded_transaction_ids=set(excluded)),
                    blocked_transaction_ids=sorted(set(blocked) | other_blocked | set(holds.blocked_ids)),
                    quarantines=(*prepared["derivation_quarantines"], *quarantines, *channel_quarantines, *privacy["quarantines"], *samourai_quarantines, *native_quarantines, *holds.quarantines),
                    transient_blocked_transaction_ids=sorted(holds.transient_ids - set(blocked) - other_blocked))
    except Exception as exc:
        return {"error": type(exc).__name__}
