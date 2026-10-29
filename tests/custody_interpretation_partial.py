"""Python builders for retained isolated interpreter vectors.

Live comparisons use the complete interpretation stage. No partial executable
or partial comparison participates in census, shadow or recorded replay.
"""
from __future__ import annotations

import json

from kassiber.core import custody_interpreters as reference
from kassiber.core.custody_compute import compute_components_stage, compute_evidence_stage
from tests.custody_snapshot import to_jsonable



def graph_results(snapshot):
    from kassiber.core.custody_snapshot import decode_owned_index
    from kassiber.core import ownership_transfers as ownership
    from kassiber.core.onchain import parse_ownership_tx
    index = decode_owned_index(snapshot["owned_index"])
    graphs, scopes, lookups = [], [], []
    try:
        for row in snapshot["transactions"]:
            parsed = parse_ownership_tx(row.get("raw_json"))
            scope = ownership._ownership_onchain_scope(row, parsed=parsed, index=index)
            entries = []
            for entry in (parsed or {}).get("inputs", ()):
                txid = str(entry.get("outpoint") or "").split(":", 1)[0]
                entries.append({
                    "outpoint": [vars(m) | {} for m in ownership._lookup_outpoint(index, entry.get("outpoint"), physical_scope=scope[:2] if scope else None)],
                    "txid_wallets": sorted(ownership._lookup_txid_wallets(index, txid, physical_scope=scope[:2] if scope else None)),
                })
                for match in entries[-1]["outpoint"]:
                    for key in ("branch_label", "address_index", "derivation_path"):
                        del match[key]
            graphs.append(parsed)
            scopes.append(scope)
            lookups.append(entries)
        return {"graphs": graphs, "scopes": scopes, "index_lookups": lookups,
                "profile_parsed": ownership._profile_parsed_transactions(snapshot["transactions"], index)}
    except Exception as exc:
        return {"error": type(exc).__name__}


def producer_results(snapshot, observations, excluded, *, consolidation=False):
    from kassiber.core.custody_snapshot import decode_owned_index
    from kassiber.core.ownership_transfers import derive_ownership_transfers, derive_multi_source_consolidations
    from kassiber.transfers import onchain_transfer_scope
    handled = set()
    try:
        derivation = (derive_multi_source_consolidations if consolidation else derive_ownership_transfers)(snapshot["transactions"],
            index=decode_owned_index(snapshot["owned_index"]),
            wallet_refs_by_id={str(w["id"]): w for w in snapshot["wallets"]},
            already_paired_ids=handled)
        claims, blocked, quarantines = reference._pair_claims(derivation.derived_pairs, observations,
            excluded_transaction_ids=set(excluded),
            wallet_refs_by_id={str(w["id"]): w for w in snapshot["wallets"]},
            physical_scopes_by_anchor={reference._anchor_id(row): scope for row in snapshot["transactions"] if (scope := onchain_transfer_scope(row)) is not None})
        return {"derivation": {k: sorted(v) if isinstance(v,set) else v for k,v in vars(derivation).items()},
            "handled_ids": sorted(handled), "claims": claims, "blocked_transaction_ids": blocked, "quarantines": quarantines,
            "native_audits": reference._native_audits_for_rowless_pairs(derivation.derived_pairs, observations, excluded_transaction_ids=set(excluded))}
    except Exception as exc:
        return {"error": type(exc).__name__}


def pipeline_results(snapshot, observations, excluded, canonical, touched, *, production_only=False, recorded_pairs=None, reserved=()):
    from kassiber.core.custody_snapshot import decode_owned_index
    from kassiber.core.ownership_transfers import derive_profile_transfers, ProfileTransferDerivation, OwnershipDeriveResult, _rows_after_derivation
    from kassiber.transfers import onchain_transfer_scope
    try:
        rows=snapshot["transactions"]
        recorded=reference.detect_intra_transfers(rows)[0] if recorded_pairs is None else recorded_pairs
        handled=set(touched)|reference._complete_recorded_pair_ids(recorded,observations)|set(reserved)
        eligible=any(event.event_key.native_namespace=="chain" for event in canonical.events) or snapshot["owned_index"] is not None
        normalized=[]
        for row in rows:
            observation=observations.get(reference._anchor_id(row))
            if eligible and row.get("observation_fee_attribution")=="implicit_wallet_delta" and observation is not None and observation.fee_attribution=="exact":
                normalized.append({**row,"amount":observation.amount_msat,"fee":observation.fee_msat,"observation_fee_attribution":"exact","custody_native_fee_normalized":True})
            else:normalized.append(row)
        derived=derive_profile_transfers(normalized,index=decode_owned_index(snapshot["owned_index"]),wallet_refs_by_id={str(w["id"]):w for w in snapshot["wallets"]},already_paired_ids=handled) if eligible else ProfileTransferDerivation(OwnershipDeriveResult(),OwnershipDeriveResult(),OwnershipDeriveResult())
        pairs=list(derived.consolidation.derived_pairs)
        for pair in derived.ownership.derived_pairs:
            pair=dict(pair)
            source=reference._anchor_id(pair.get("out") or {})
            if derived.ownership.out_row_overrides.get(source) is not None:
                pair.setdefault("group_id",f"ownership-event:{source}")
                pair.setdefault("allow_unclaimed_residual",True)
            pairs.append(pair)
        pairs.extend(derived.fanout.derived_pairs)
        members={transaction_id:tuple(sorted(item_id for item_id,_hash in event.observation_aliases)) for event in canonical.events for transaction_id,_hash in event.observation_aliases}
        by_id={str(row.get("id") or ""):row for row in rows}
        quarantines={}
        for blocked in (*derived.consolidation.blocked_sources,*derived.ownership.blocked_sources,*derived.fanout.blocked_sources):
            row=blocked.get("row") or {};blocked_id=reference._row_id(row)
            reason=str(blocked.get("reason") or "ownership_transfer_unresolved")
            detail=dict(blocked.get("detail") or {})
            ids=members.get(blocked_id,(blocked_id,))
            detail.setdefault("atomic_event_transaction_ids",list(ids))
            for transaction_id in ids:
                if transaction_id in excluded:continue
                member=by_id.get(transaction_id,row)
                quarantines[(transaction_id,reason)]={"transaction_id":transaction_id,"workspace_id":member.get("workspace_id"),"profile_id":member.get("profile_id"),"reason":reason,"detail_json":json.dumps(detail,sort_keys=True)}
        if production_only:
            claims,blocked,pair_quarantines=(),(),()
        else:
            claims,blocked,pair_quarantines=reference._pair_claims(pairs,observations,excluded_transaction_ids=set(excluded),wallet_refs_by_id={str(w["id"]):w for w in snapshot["wallets"]},physical_scopes_by_anchor={reference._anchor_id(row):scope for row in rows if (scope:=onchain_transfer_scope(row)) is not None})
        after=_rows_after_derivation(normalized,derived.consolidation,sort_key=None)
        return {"derivation":{name:{k:sorted(v) if isinstance(v,set) else v for k,v in vars(getattr(derived,name)).items()} for name in ("consolidation","ownership","fanout")},"handled_ids":sorted(handled),"derivation_rows":normalized,"rows_after_consolidation":after,"rows_after_fanout":_rows_after_derivation(after,derived.fanout,sort_key=None),"derived_pairs":pairs,"claims":claims,"blocked_transaction_ids":blocked,"quarantines":pair_quarantines,"derivation_quarantines":[quarantines[key] for key in sorted(quarantines)],"native_audits":() if production_only else reference._native_audits_for_rowless_pairs(pairs,observations,excluded_transaction_ids=set(excluded))}
    except Exception as exc:return {"error":type(exc).__name__}


def channel_results(snapshot):
    from kassiber.core.custody_journal import channel_context_from_records
    try:
        roles, pairs = channel_context_from_records(snapshot["channel_records"], snapshot["transactions"], {str(w["id"]): w for w in snapshot["wallets"]})
        return dict(roles=roles, pairs=pairs)
    except Exception as exc:
        return dict(error=type(exc).__name__)


def python_results(snapshot):
    components, _ = compute_components_stage(snapshot)
    canonical = compute_evidence_stage(snapshot)["canonical_input"]
    observations = reference._observations_by_transaction(canonical)
    excluded = sorted({
        str(leg.get("anchor_transaction_id") or leg.get("transaction_id"))
        for component in components for leg in component.get("legs", ())
        if leg.get("anchor_transaction_id") or leg.get("transaction_id")
    })
    present = any(row.get("config_text_mentions_samourai", "samourai" in str(row.get("config_json") or "").lower()) for row in snapshot["transactions"])
    pairs, fees, candidates, touched = reference._samourai_privacy_pairs(snapshot["transactions"], observations) if present else ([], [], set(), set())
    claims, blocked, quarantines = reference._pair_claims(pairs, observations,
        excluded_transaction_ids=set(excluded), wallet_refs_by_id={}, physical_scopes_by_anchor={})
    assert not blocked and not quarantines  # Samourai pairs use only same-event proof.
    unverified = sorted(candidates - touched - set(excluded))
    rows_by_id = {str(row.get("id") or ""): row for row in snapshot["transactions"]}
    quarantines = [{
        "transaction_id": transaction_id,
        "workspace_id": rows_by_id.get(transaction_id, {}).get("workspace_id"),
        "profile_id": rows_by_id.get(transaction_id, {}).get("profile_id"),
        "reason": "samourai_native_event_unverified",
        "detail_json": json.dumps({"required_for": "authoritative_same_event_observation", "resolution": "sync every imported Samourai source with a chain observer"}, sort_keys=True),
    } for transaction_id in unverified]
    from kassiber.transfers import bitcoin_network_domain, is_lightning_payment_hash_row, onchain_transfer_scope
    recorded_pairs, matched = reference.detect_intra_transfers(snapshot["transactions"])
    recorded_claims, recorded_blocked, recorded_quarantines = reference._pair_claims(
        recorded_pairs, observations, excluded_transaction_ids=set(excluded),
        wallet_refs_by_id={str(wallet["id"]): wallet for wallet in snapshot["wallets"]},
        physical_scopes_by_anchor={reference._anchor_id(row): scope for row in snapshot["transactions"] if (scope := onchain_transfer_scope(row)) is not None})
    from kassiber.core.ownership_transfers import derive_recorded_fanout_transfers
    handled = reference._complete_recorded_pair_ids(recorded_pairs, observations)
    fanout = derive_recorded_fanout_transfers(snapshot["transactions"], already_paired_ids=handled)
    fanout_claims, fanout_blocked, fanout_quarantines = reference._pair_claims(
        fanout.derived_pairs, observations, excluded_transaction_ids=set(excluded),
        wallet_refs_by_id={str(wallet["id"]): wallet for wallet in snapshot["wallets"]},
        physical_scopes_by_anchor={reference._anchor_id(row): scope for row in snapshot["transactions"] if (scope := onchain_transfer_scope(row)) is not None})
    from tests.custody_safety_partial import safety_results
    from tests.custody_native_partial import native_results
    return to_jsonable({"native": native_results(snapshot, observations), "channels": channel_results(snapshot), "safety": safety_results(snapshot, observations, excluded, canonical), "graph": graph_results(snapshot), "pipeline": pipeline_results(snapshot, observations, excluded, canonical, touched), "consolidation": producer_results(snapshot, observations, excluded, consolidation=True), "ownership": producer_results(snapshot, observations, excluded), "fanout": {
        "derivation": {key: sorted(value) if isinstance(value, set) else value for key, value in vars(fanout).items()},
        "handled_ids": sorted(handled), "claims": fanout_claims,
        "blocked_transaction_ids": fanout_blocked, "quarantines": fanout_quarantines,
        "native_audits": reference._native_audits_for_rowless_pairs(fanout.derived_pairs, observations, excluded_transaction_ids=set(excluded)),
    }, "recorded": {
        "pairs": recorded_pairs, "matched_ids": sorted(matched), "claims": recorded_claims,
        "blocked_transaction_ids": recorded_blocked, "quarantines": recorded_quarantines,
        "exact_native_pair_ids": sorted(reference._exact_native_pair_ids(recorded_pairs, observations)),
        "complete_recorded_pair_ids": sorted(reference._complete_recorded_pair_ids(recorded_pairs, observations)),
        "native_audits": reference._native_audits_for_rowless_pairs(recorded_pairs, observations, excluded_transaction_ids=set(excluded)),
        "physical_scopes": [onchain_transfer_scope(row) for row in snapshot["transactions"]],
        "network_domains": [bitcoin_network_domain(row) for row in snapshot["transactions"]],
        "lightning_rows": [is_lightning_payment_hash_row(row) for row in snapshot["transactions"]],
    }, "samourai": {
        "pairs": pairs, "fee_claims": fees, "candidate_ids": sorted(candidates), "touched_ids": sorted(touched),
        "claims": (*claims, *fees), "blocked_transaction_ids": unverified, "quarantines": quarantines,
    }, "shared": {
        "observations_by_transaction": observations,
        "excluded_transaction_ids": excluded,
        "row_anchor_ids": [reference._anchor_id(row) for row in snapshot["transactions"]],
    }}, redact_config=False)
