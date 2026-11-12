"""Frozen Python compiler-boundary witnesses; candidate pairs are test inputs.

Pairs stay outside the schema-1 raw snapshot. Rust recomputes every canonical
observation, reservation and authority bit from that snapshot.
"""
from copy import deepcopy
import json
from pathlib import Path
import sys
from kassiber.core import custody_interpreters as reference
from kassiber.core.custody_compute import compute_components_stage, compute_evidence_stage
from kassiber.transfers import onchain_transfer_scope
from tests.custody_interpretation_generator import recorded_snapshot, samourai_snapshot
from tests.custody_snapshot import to_jsonable

PATH = Path(__file__).parent / "fixtures/custody_pair_cases.json"
SCENARIOS = (
    "ordinary", "source_fallback", "forged_label", "excluded_source", "excluded_target",
    "rowless", "rowless_untrusted", "rowless_reserved", "observed_target", "virtual_channel_source",
    "wrong_direction", "empty_anchor", "chronology", "skew_boundary", "bad_timestamp", "time_overflow",
    "network_mismatch", "cross_rail", "cross_rail_unreviewed", "unlike_exposure",
    "explicit_fee", "implicit_fee", "implausible_fee", "allowed_residual", "fee_floor_boundary",
    "fee_percent_boundary", "fee_percent_above", "request_zero", "request_negative", "request_unicode", "request_invalid",
    "request_float", "request_huge", "request_huge_negative", "request_type_error", "request_false", "request_precedence", "request_fallback", "exact_msat", "metadata",
    "pair_order", "source_cursor", "target_cursor", "same_native_skew", "forged_native_transition",
    "fanout", "fanout_forged", "fanout_duplicate_wallet", "fanout_duplicate_target", "fanout_amount",
    "fanout_nonconserving", "rowless_alias", "virtual_channel_excluded", "rowless_target_alias",
    "proof_exact", "proof_refund", "proof_confidence", "proof_conflict", "proof_method", "proof_source_hash",
    "proof_target_hash", "proof_kind", "proof_policy", "proof_amount", "proof_float_amount", "proof_string_amount",
    "audit_raw_strings", "audit_raw_huge", "audit_bad_amount", "audit_bad_fee", "audit_null_fee", "audit_skipped_bad_amount", "audit_virtual_raw", "fanout_raw_string", "fanout_bad_amount", "fanout_conversion_order", "fanout_wallet_admission_order", "fanout_target_admission_order", "fanout_group_admission_order", "implicit_source_order",
)


def cases():
    for seed, case in enumerate(SCENARIOS):
        book = recorded_snapshot(seed, case="bitcoin" if case in {"ordinary", "implicit_fee", "same_native_skew", "rowless", "rowless_alias", "rowless_reserved", "observed_target", "rowless_target_alias", "implicit_source_order"} else "lightning_lnd")
        rows = book["transactions"]
        source = next(r for r in rows if r["direction"] == "outbound")
        target = next(r for r in rows if r["direction"] == "inbound")
        # A channel pair is an existing trusted compiler input; lifecycle proof
        # production is a later slice. All other labels are revalidated here.
        pair = {"out": deepcopy(source), "in": deepcopy(target), "source": "channel_lifecycle"}
        pairs = [pair]
        excluded = []
        if case == "ordinary": pair["source"] = "row_matched"
        if case == "source_fallback": pair.update(source="", pair_source="lightning_payment_hash")
        if case == "forged_label":
            pair["source"] = "row_matched"
        if case in {"excluded_source", "rowless_reserved"}: excluded.append(source["id"])
        if case == "excluded_target": excluded.append(target["id"])
        if case in {"rowless", "rowless_untrusted", "rowless_reserved", "rowless_alias", "observed_target", "rowless_target_alias"}:
            pair["source"] = "ownership_derived"
            if case != "observed_target":
                pair["in"]["id"] = "owned-derive:destination"
            if case in {"rowless_alias", "observed_target"}: pair["in"]["journal_transaction_id"] = source["id"]
            if case == "rowless_target_alias": pair["in"]["journal_transaction_id"] = target["id"]
        if case in {"virtual_channel_source", "virtual_channel_excluded"}:
            pair["out"]["id"] = "channel-source"
            pair["out"]["fee"] = 17
            if case.endswith("excluded"): excluded.append("channel-source")
        if case == "wrong_direction": pair["out"], pair["in"] = pair["in"], pair["out"]
        if case == "empty_anchor": pair["out"]["id"] = ""
        if case in {"chronology", "same_native_skew"}: source["occurred_at"] = "2026-10-28T00:00:00Z"
        if case == "skew_boundary":
            source["occurred_at"] = "2026-01-03T00:00:00.000001Z"; target["occurred_at"] = "2026-01-01T00:00:00Z"
        if case == "bad_timestamp": target["occurred_at"] = "bad"
        if case == "time_overflow": target["occurred_at"] = "9999-12-31T00:00:00Z"
        if case == "network_mismatch": target["config_json"] = '{"network":"regtest"}'
        if case in {"cross_rail", "cross_rail_unreviewed"} or case.startswith("proof_"):
            target.update(wallet_kind="liquid", asset="LBTC", config_json='{"chain":"liquid","network":"liquidv1"}')
            if case == "cross_rail": pair["policy"] = "carrying-value"
        if case == "unlike_exposure": target["asset"] = "OTHER"
        if case in {"explicit_fee", "implicit_fee", "implausible_fee", "allowed_residual"}:
            target["amount"] = source["amount"] // 2
            if case in {"explicit_fee", "implicit_fee"}: pair["out_amount_msat"] = source["amount"]
            if case == "implicit_fee": source["observation_fee_attribution"] = "implicit_wallet_delta"
            if case == "allowed_residual": pair["allow_unclaimed_residual"] = True
        if case.startswith("fee_"):
            source["amount"] = 100_000_000 if case == "fee_floor_boundary" else 900_719_925_474_099
            tolerance = max(source["amount"] // 100, 2_500_000)
            target["amount"] = source["amount"] - tolerance - (case == "fee_percent_above")
        requested = {"request_zero": 0, "request_negative": -1, "request_unicode": "\u2003+１２_３４\u2003", "request_invalid": "1__2", "request_float": 1.9, "request_huge": 10**100, "request_huge_negative": -(10**100), "request_type_error": [], "request_false": False, "request_precedence": 12, "request_fallback": ""}
        if case in requested:
            pair.update(out_amount_msat=requested[case], out_amount=123)
        if case == "exact_msat":
            source["amount"] = target["amount"] = (1 << 53) + 37
        if case == "metadata": pair.update(pair_id="explicit-id", group_id="group", component_id="component", kind="peg-in", policy="carrying-value")
        if case in {"pair_order", "source_cursor", "target_cursor"}:
            pair.update(pair_id="z", out_amount_msat=7)
            second = deepcopy(pair); second.update(pair_id="a", out_amount_msat=13)
            pairs.append(second)
        if case == "forged_native_transition": pair["source"] = "native_htlc_transition"
        if case.startswith("fanout"):
            book = samourai_snapshot(seed, case="fanout")
            source = next(r for r in book["transactions"] if r["direction"] == "outbound")
            targets = [r for r in book["transactions"] if r["direction"] == "inbound"]
            pairs = [{"out": {**deepcopy(source), "id": "recorded-fanout:" + r["id"], "journal_transaction_id": source["id"], "amount": r["amount"]}, "in": deepcopy(r), "source": "recorded_fanout", "group_id": "fanout", "out_amount_msat": r["amount"]} for r in targets]
            # Same-event authority can independently authorize these claims;
            # exact group ids below pin every stronger fan-out shape guard.
            if case == "fanout_forged": pairs[0]["source"] = "row_matched"
            if case in {"fanout_duplicate_wallet", "fanout_wallet_admission_order"}: targets[1]["wallet_id"] = targets[0]["wallet_id"]
            if case == "fanout_duplicate_target": pairs[1]["in"] = deepcopy(pairs[0]["in"])
            if case == "fanout_amount": pairs[0]["out"]["amount"] += 1
            if case == "fanout_nonconserving": source["amount"] += 1
        if case.startswith("audit_"):
            pair["source"] = "channel_lifecycle"
            pair["in"]["id"] = "rowless-destination"
            if case == "audit_virtual_raw":
                pair["out"]["id"] = "virtual-source"
                pair["in"] = deepcopy(target)
        if not case.startswith("fanout"):
            # Keep synthetic ids and authored metadata while updating row facts.
            for item in pairs:
                for side, original in (("out",source),("in",target)):
                    for key in ("amount", "occurred_at", "asset", "wallet_kind", "config_json", "observation_fee_attribution"):
                        item[side][key] = original[key]
        if case == "audit_raw_strings": pair["in"]["amount"] = " +٣_٠٠٠ "; pair["out"]["fee"] = "17"
        if case == "audit_raw_huge": pair["in"]["amount"] = 10**100; pair["out"]["fee"] = 10**101
        if case == "audit_bad_amount": pair["in"]["amount"] = [1]
        if case == "audit_bad_fee": pair["out"]["fee"] = "bad"
        if case == "audit_null_fee": pair["out"]["fee"] = None
        if case == "audit_skipped_bad_amount": pair["out"]["id"] = "missing-source"; pair["in"]["amount"] = [1]; pair["source"] = "ownership_derived"
        if case == "audit_virtual_raw": pair["in"]["amount"] = "123"; pair["out"]["fee"] = "17"
        if case == "fanout_raw_string": pairs[0]["out"]["amount"] = str(pairs[0]["out"]["amount"])
        if case in {"fanout_bad_amount", "fanout_conversion_order"}: pairs[0]["out"]["amount"] = [1]
        if case == "fanout_conversion_order": pairs[0]["out_amount_msat"] = 0
        if case in {"fanout_wallet_admission_order", "fanout_target_admission_order"}: pairs[0]["out"]["amount"] = [1]
        if case == "fanout_target_admission_order": pairs[1]["in"]["id"] = "missing-target"
        if case == "fanout_group_admission_order":
            for item in pairs: item["group_id"] = "z-first"
            pairs[0]["out"]["amount"] = [1]
            other = deepcopy(pairs)
            for item in other: item["group_id"] = "a-later"
            other[0]["out_amount_msat"] = "bad"
            pairs.extend(other)
        if case == "implicit_source_order":
            source["observation_fee_attribution"] = "implicit_wallet_delta"
            target["amount"] = source["amount"] // 2
            pair["in"]["amount"] = target["amount"]
            second_rows = deepcopy(book["transactions"])
            for row in second_rows:
                row["id"] += "-second"
                row["external_id"] = "c" * 64
                raw = json.loads(row["raw_json"])
                raw["txid"] = row["external_id"]
                row["raw_json"] = json.dumps(raw)
            book["transactions"].extend(second_rows)
            pairs.append({"out":deepcopy(next(r for r in second_rows if r["direction"] == "outbound")),"in":deepcopy(next(r for r in second_rows if r["direction"] == "inbound")),"source":"row_matched"})
        for row in book["transactions"]:
            from kassiber.core.chain_observer.provenance import canonical_graph_hash, canonical_observed_quantity_hash
            row.update(observation_graph_hash=canonical_graph_hash(row["raw_json"]), observation_quantity_hash=canonical_observed_quantity_hash(row))
        yield case, book, pairs, excluded


def vectors():
    result = []
    for case, book, pairs, excluded in cases():
        components, _ = compute_components_stage(book)
        observations = reference._observations_by_transaction(compute_evidence_stage(book)["canonical_input"])
        scopes = {reference._anchor_id(r): scope for r in book["transactions"] if (scope := onchain_transfer_scope(r)) is not None}
        if case == "implicit_source_order":
            ordered = sorted(pairs, key=lambda p: observations[reference._anchor_id(p["out"])].quantity_hash, reverse=True)
            for ordinal, item in enumerate(ordered): item["out"]["occurred_at"] = f"2026-01-01T00:00:{ordinal:02}Z"
        proofs = {}
        if case.startswith("proof_"):
            from types import SimpleNamespace
            from kassiber.core.custody_native_transitions import NativeTransitionProof
            pair = pairs[0]
            source, target = observations[pair["out"]["id"]], observations[pair["in"]["id"]]
            pair.update(source="native_htlc_transition", kind="submarine-swap", policy="taxable", out_amount_msat=source.principal_msat)
            # Cross-rail permission must come from proof, independent of tax policy.
            proof = {"candidate": {"confidence":"exact", "conflict_size":1, "method":"payment_hash", "default_kind":"submarine-swap"}, "source_quantity_hash":source.quantity_hash,"target_quantity_hash":target.quantity_hash,"policy":"taxable"}
            if case == "proof_refund": proof["candidate"]["method"] = "htlc_refund"
            if case == "proof_confidence": proof["candidate"]["confidence"] = "strong"
            if case == "proof_conflict": proof["candidate"]["conflict_size"] = 2
            if case == "proof_method": proof["candidate"]["method"] = "heuristic"
            if case == "proof_source_hash": proof["source_quantity_hash"] = "wrong"
            if case == "proof_target_hash": proof["target_quantity_hash"] = "wrong"
            if case == "proof_kind": pair["kind"] = "wrong"
            if case == "proof_policy": pair["policy"] = "wrong"
            if case == "proof_amount": pair["out_amount_msat"] -= 1
            if case == "proof_float_amount": pair["out_amount_msat"] = float(source.principal_msat)
            if case == "proof_string_amount": pair["out_amount_msat"] = str(source.principal_msat)
            proofs[(pair["out"]["id"],pair["in"]["id"])] = proof
        native_proofs = {key: NativeTransitionProof(SimpleNamespace(**p["candidate"]),p["source_quantity_hash"],p["target_quantity_hash"],p["policy"]) for key,p in proofs.items()}
        try:
            claims, blocked, quarantines = reference._pair_claims(pairs, observations, excluded_transaction_ids=set(excluded), wallet_refs_by_id={str(w["id"]): w for w in book["wallets"]}, physical_scopes_by_anchor=scopes, native_transition_proofs=native_proofs)
            expected = {"claims": claims, "blocked_transaction_ids": blocked, "quarantines": quarantines}
        except Exception as exc:
            expected = {"error_code": getattr(exc,"code",type(exc).__name__)}
        def capture(call):
            try: return call()
            except Exception as exc: return {"error_code": getattr(exc, "code", type(exc).__name__)}
        result.append({"case":case,"input":book,"pairs":pairs,"excluded":excluded,"proofs":[{"out_id":key[0],"in_id":key[1],"proof":p} for key,p in proofs.items()],"expected":to_jsonable(expected,redact_config=False),
                       "fanout_groups":capture(lambda: sorted(reference._exact_recorded_fanout_group_ids(pairs,observations,scopes))),
                       "native_audits":to_jsonable(capture(lambda: reference._native_audits_for_rowless_pairs(pairs,observations,excluded_transaction_ids=set(excluded))),redact_config=False)})
    return result


if __name__ == "__main__":
    text = json.dumps(vectors(), indent=2, sort_keys=True) + "\n"
    if "--write" in sys.argv: PATH.write_text(text)
    else: assert PATH.read_text() == text
