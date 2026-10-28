"""Independent interpreter slices; test-only and raw snapshot in both engines.

This does not declare interpretation implemented. Until composition is ported,
each slice runs in isolation before the other interpreters reserve or hold rows.
"""
from __future__ import annotations

from functools import lru_cache
import json
import os
from pathlib import Path
import subprocess

from kassiber.core import custody_interpreters as reference
from kassiber.core.custody_compute import compute_components_stage, compute_evidence_stage
from tests.custody_snapshot import to_jsonable

ROOT = Path(__file__).resolve().parents[1]


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


def ownership_results(snapshot, observations, excluded):
    from kassiber.core.custody_snapshot import decode_owned_index
    from kassiber.core.ownership_transfers import derive_ownership_transfers
    from kassiber.transfers import onchain_transfer_scope
    handled = set()
    try:
        derivation = derive_ownership_transfers(snapshot["transactions"],
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
    return to_jsonable({"graph": graph_results(snapshot), "ownership": ownership_results(snapshot, observations, excluded), "fanout": {
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


@lru_cache(maxsize=1)
def executable():
    subprocess.run(["cargo", "build", "--locked", "-q", "-p", "kassiber-custody", "--example", "interpretation_partial"], cwd=ROOT, check=True, timeout=300)
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    if not target.is_absolute():
        target = ROOT / target
    return target / "debug/examples" / ("interpretation_partial.exe" if os.name == "nt" else "interpretation_partial")


def rust_results(snapshot):
    result = subprocess.run([str(executable())], input=json.dumps(snapshot) + "\n", capture_output=True, text=True, check=True, timeout=60)
    value = json.loads(result.stdout)
    if "partial_error" in value:
        raise AssertionError("Rust partial interpretation refused the reference snapshot")
    return value


def compare(snapshot):
    from tests.custody_engine_compare import assert_same
    expected, actual = python_results(snapshot), rust_results(snapshot)
    assert_same(expected, actual, "partial interpretation")
    return tuple(expected)
