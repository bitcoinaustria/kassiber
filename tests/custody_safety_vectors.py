"""Direct Python safety witnesses, outside the raw-only snapshot contract."""
from copy import deepcopy
import json
from pathlib import Path
import sys

from kassiber.core import custody_interpreters as reference
from kassiber.core import ownership_transfers as ownership
from kassiber.core.privacy_hops import privacy_hop_evidence_from_row
from kassiber.transfers import detect_unscoped_transfer_review_ids
from tests.custody_snapshot import to_jsonable
from tests.custody_safety_generator import stamp

PATH = Path(__file__).parent / "fixtures/custody_safety_helpers.json"


def row(id, *, direction="outbound", wallet="wallet-a", txid="a" * 64, confirmed=None, **changes):
    raw = dict(chain="bitcoin", network="main", txid=txid)
    if confirmed is not None:
        raw["status"] = dict(confirmed=confirmed)
    return dict(id=id, workspace_id="workspace", profile_id="profile", wallet_id=wallet,
                asset="BTC", direction=direction, amount=10000, fee=0, external_id=txid,
                occurred_at="2026-01-01T00:00:00Z", created_at="2026-01-01T00:00:00Z",
                raw_json=json.dumps(raw), **changes)


def graph_rows():
    rows = [row("a", confirmed=True), row("b", txid="c" * 64, confirmed=False)]
    for item in rows:
        raw = json.loads(item["raw_json"])
        raw.update(vin=[dict(txid="b" * 64, vout=1, prevout=dict(value_sats=13, scriptpubkey="0014" + "11" * 20))],
                   vout=[dict(n=0, value_sats=10, scriptpubkey="0014" + "22" * 20)])
        item["raw_json"] = json.dumps(raw)
    return rows


def helpers():
    vectors = []

    def add(case, op, **facts):
        vector = dict(case=case, op=op, **deepcopy(facts))
        try:
            if op == "detectors":
                rows = facts["rows"]
                result = dict(conflicts=ownership.detect_conflicting_spend_ids(rows), pending=ownership.detect_pending_onchain_ids(rows),
                              fanout=ownership.detect_unresolved_owned_fanout_ids(rows, covered_ids=set(facts.get("covered", ()))),
                              unscoped=detect_unscoped_transfer_review_ids(rows))
            elif op == "holds":
                result = vars(reference._raw_observation_holds(facts["rows"], facts["derivation_rows"], facts["pairs"],
                    wallet_refs_by_id=facts["wallets"], has_chain_events=facts["has_chain_events"],
                    excluded_ids=set(facts["excluded"]), blocked_ids=set(facts["blocked"]), resolved_ids=set(facts["resolved"])))
            elif op == "filter":
                result = reference._without_held_pairs(facts["pairs"], set(facts["held"]))
            elif op == "conflict_scope":
                result = ownership._physical_conflict_scope(facts["row"], facts["parsed"])
            elif op == "outpoint":
                result = ownership._canonical_outpoint(facts["value"])
            elif op == "privacy_resolution":
                from types import SimpleNamespace
                from tests.custody_safety_partial import privacy_result
                result = privacy_result(facts["rows"], [], [], {key: SimpleNamespace(**value) for key, value in facts["observations"].items()}, facts["wallets"], facts["excluded"], facts["candidates"], facts["native_ids"])
            elif op == "privacy_evidence":
                evidence = privacy_hop_evidence_from_row(facts["row"])
                result = None if evidence is None else json.dumps(evidence, sort_keys=True)
            result = {key: sorted(value) if isinstance(value, (set, frozenset)) else value for key, value in result.items()} if isinstance(result, dict) else result
            vector["expected"] = to_jsonable(result, redact_config=False)
        except Exception as exc:
            vector["expected"] = dict(error=type(exc).__name__)
        vectors.append(vector)

    # Confirmation chronology and conflicts, including import-only losers.
    for case in ("winner", "none", "both", "inbound_graph", "synthetic", "graphless_sibling", "network_separation", "chain_separation", "invalid_network", "contradictory_network", "unknown_asset", "liquid_missing_asset", "bad_txid", "conflicting_txid", "no_txid", "no_graph", "no_inputs", "same_txid", "negative_index", "huge_index", "unicode_index", "missing_outpoint"):
        rows = graph_rows()
        a, b = rows
        if case in {"none", "both"}:
            for item in rows:
                raw = json.loads(item["raw_json"]); raw["status"]["confirmed"] = case == "both"; item["raw_json"] = json.dumps(raw)
        if case == "inbound_graph": b["direction"] = "inbound"
        if case == "synthetic":
            b.update(id="owned-derive:b", external_id="synthetic-output")
        if case == "graphless_sibling": rows.append(row("sibling", txid="c" * 64, direction="inbound", wallet="wallet-b"))
        if case in {"network_separation", "chain_separation", "invalid_network", "contradictory_network", "unknown_asset", "liquid_missing_asset", "bad_txid", "conflicting_txid", "no_txid", "no_graph", "no_inputs", "same_txid", "negative_index", "huge_index", "unicode_index", "missing_outpoint"}:
            raw = json.loads(b["raw_json"])
            if case == "network_separation": raw["network"] = "regtest"
            if case in {"chain_separation", "liquid_missing_asset"}:
                for item in (rows if case == "liquid_missing_asset" else [b]):
                    item["asset"] = "LBTC"
                    liquid = json.loads(item["raw_json"]); liquid.update(chain="liquid", network="liquid"); item["raw_json"] = json.dumps(liquid)
                raw = json.loads(b["raw_json"])
            if case == "invalid_network": raw["network"] = "unsupported"
            if case == "contradictory_network": b["config_json"] = '{"network":"regtest"}'
            if case == "unknown_asset": b["asset"] = "TOKEN"
            if case == "bad_txid": raw["txid"] = "provider:bad"; b["external_id"] = "provider:bad"
            if case == "conflicting_txid": b["external_id"] = "d" * 64
            if case == "no_txid": raw.pop("txid"); b["external_id"] = "provider:bad"
            if case == "no_graph": raw = {}
            if case == "no_inputs": raw.pop("vin")
            if case == "same_txid": raw["txid"] = "a" * 64; b["external_id"] = "a" * 64
            if case in {"negative_index", "huge_index", "unicode_index"}:
                for item in rows:
                    other = json.loads(item["raw_json"]); other["vin"][0]["vout"] = {"negative_index": -1, "huge_index": 2**100, "unicode_index": "١"}[case]; item["raw_json"] = json.dumps(other)
                raw = json.loads(b["raw_json"])
            if case == "missing_outpoint": raw["vin"][0].pop("txid")
            b["raw_json"] = json.dumps(raw)
        add("conflict_" + case, "detectors", rows=rows, covered=[])

    for case in ("explicit_false", "explicit_true", "missing_status", "status_string", "status_int", "legacy_confirmed", "nested_only", "nested_sibling", "closed_latest_false", "closed_latest_true", "closed_tie", "closed_naive", "closed_offset", "closed_invalid", "closed_overflow", "stale_graph", "stale_quantity", "unsupported_authority", "malformed_authority", "null_payload", "array_payload", "bad_json", "synthetic", "custody_tax"):
        a = row("a", confirmed=False)
        rows = [a, row("b", direction="inbound", wallet="wallet-b")]
        raw = json.loads(a["raw_json"])
        if case == "explicit_true": raw["status"]["confirmed"] = True
        if case == "missing_status": raw.pop("status")
        if case == "status_string": raw["status"]["confirmed"] = "false"
        if case == "status_int": raw["status"]["confirmed"] = 0
        if case == "legacy_confirmed": rows[1]["confirmed_at"] = "stored-confirmed"
        if case.startswith("nested_"):
            raw = {"tx": raw}
            if case == "nested_sibling": rows[1]["raw_json"] = json.dumps(dict(txid="a" * 64, status=dict(confirmed=False)))
        a["raw_json"] = json.dumps(raw)
        if case.startswith("closed_") or case in {"stale_graph", "stale_quantity", "unsupported_authority", "malformed_authority"}:
            a["observation_authority_version"] = 1
            a["observation_observed_at"] = "2026-01-01T00:00:00.000001Z"
            b = rows[1]
            b.update(observation_authority_version=1, observation_observed_at="2026-01-01T00:00:00Z")
            b["raw_json"] = json.dumps(dict(chain="bitcoin", network="main", txid="a" * 64, status=dict(confirmed=True)))
            if case == "closed_latest_true": a["observation_observed_at"] = "2025-12-31T23:00:00Z"
            if case == "closed_tie": b["observation_observed_at"] = a["observation_observed_at"]
            if case == "closed_naive": a["observation_observed_at"] = "2026-01-01T00:00:00.000001"
            if case == "closed_offset": a["observation_observed_at"] = "2026-01-01T01:00:00.000001+01:00"
            if case == "closed_invalid": a["observation_observed_at"] = "not-a-time"
            if case == "closed_overflow": a["observation_observed_at"] = "0001-01-01T00:00:00+01:00"
            stamp(dict(transactions=rows, gap_worklist={}))
            if case == "stale_graph": a["observation_graph_hash"] = "stale"
            if case == "stale_quantity": a["observation_quantity_hash"] = "stale"
            if case == "unsupported_authority": a["observation_authority_version"] = 2
            if case == "malformed_authority": a["observation_authority_version"] = []
        if case in {"null_payload", "array_payload", "bad_json"}: a["raw_json"] = {"null_payload":"null", "array_payload":"[]", "bad_json":"{"}[case]
        if case == "synthetic": a["id"] = "cross-split:a"
        if case == "custody_tax": a["id"] = "custody-tax:a"
        add("pending_" + case, "detectors", rows=rows, covered=[])

    # Covered subsets, graph guard and fee-only sources change fan-out safety.
    for case in ("fanout", "consolidation", "many_many", "one_one", "same_wallet", "all_covered", "part_covered", "graph_uncovered", "graph_part", "graph_all", "zero_covered_source", "zero_uncovered_source", "zero_unhandled", "negative_source", "zero_receipt", "unknown_direction", "synthetic", "custody_tax", "no_scope", "huge_amount", "bad_amount", "null_amount", "false_amount", "duplicate_id"):
        rows = [row("a"), row("b", direction="inbound", wallet="wallet-b"), row("c", direction="inbound", wallet="wallet-c")]
        covered = []
        if case in {"consolidation", "many_many"}: rows.append(row("d", wallet="wallet-c"))
        if case == "consolidation": rows.pop(2)
        if case == "one_one": rows.pop()
        if case == "same_wallet":
            for item in rows: item["wallet_id"] = "wallet-a"
        if case in {"all_covered", "graph_all"}: covered = [item["id"] for item in rows]
        if case in {"part_covered", "graph_part"}: covered = ["a"]
        if case.startswith("graph_"):
            raw = json.loads(graph_rows()[0]["raw_json"]); rows[0]["raw_json"] = json.dumps(raw)
        if case.startswith("zero_") and case != "zero_receipt":
            rows.append(row("z")); rows[-1]["amount"] = 0
            if case == "zero_covered_source": covered = ["a"]
            if case == "zero_uncovered_source": covered = ["b"]
        if case in {"negative_source", "huge_amount", "bad_amount", "null_amount", "false_amount"}: rows[0]["amount"] = {"negative_source": -1, "huge_amount": 2**200, "bad_amount": {}, "null_amount": None, "false_amount": False}[case]
        if case == "zero_receipt": rows[1]["amount"] = 0
        if case == "unknown_direction": rows[0]["direction"] = "other"
        if case == "synthetic": rows[0]["id"] = "recorded-fanout:a"
        if case == "custody_tax": rows[0]["id"] = "custody-tax:a"
        if case == "no_scope":
            for item in rows: item.update(raw_json="{}", external_id="provider")
        if case == "duplicate_id": rows[2]["id"] = "b"
        add("fanout_" + case, "detectors", rows=rows, covered=covered)

    for case in ("provider", "blank", "whitespace", "same_wallet", "asset_case", "asset_split", "txid_case", "provider_case", "zero_out", "zero_in", "missing_direction", "synthetic", "custody_tax", "typed_id", "huge_amount", "bad_string", "bad_list"):
        rows = [row("a"), row("b", direction="inbound", wallet="wallet-b"), row("z", direction="other")]
        for item in rows: item.update(raw_json="{}", external_id="provider", asset="fß")
        if case in {"blank", "whitespace"}:
            for item in rows: item["external_id"] = "" if case == "blank" else " \t\u2003"
        if case == "same_wallet": rows[1]["wallet_id"] = "wallet-a"
        if case == "asset_case": rows[1]["asset"] = " FSS "
        if case == "asset_split": rows[1]["asset"] = "BTC"
        if case == "txid_case":
            for item in rows: item["external_id"] = "a" * 64
            rows[1]["external_id"] = "A" * 64
        if case == "provider_case": rows[1]["external_id"] = "PROVIDER"
        if case == "zero_out": rows[0]["amount"] = 0
        if case == "zero_in": rows[1]["amount"] = 0
        if case == "missing_direction": rows[0].pop("direction")
        if case in {"synthetic", "custody_tax"}: rows[0]["id"] = ("owned-derive:" if case == "synthetic" else "custody-tax:") + "a"
        if case == "typed_id":
            for item in rows: item["external_id"] = [False, 1, "label"]
        if case in {"huge_amount", "bad_string", "bad_list"}: rows[0]["amount"] = {"huge_amount": 2**150, "bad_string": "not-an-int", "bad_list": [1]}[case]
        add("unscoped_" + case, "detectors", rows=rows, covered=[])

    def hold(case, rows, pairs=(), **changes):
        facts = dict(rows=rows, derivation_rows=rows, pairs=list(pairs), wallets={"wallet-a":dict(label="Ålice"), "wallet-b":dict(label=None)},
                     has_chain_events=True, excluded=[], blocked=[], resolved=[])
        facts.update(changes)
        add(case, "holds", **facts)

    for reason, rows in (("pending", [row("a", confirmed=False)]), ("conflict", graph_rows()),
                         ("fanout", [row("a"), row("b", direction="inbound", wallet="wallet-b"), row("c", direction="inbound", wallet="wallet-c")]),
                         ("unscoped", [dict(row("a"), raw_json="{}", external_id="provider"), dict(row("b", direction="inbound", wallet="wallet-b"), raw_json="{}", external_id="provider")])):
        hold("hold_" + reason, rows)
        for change in ("excluded", "blocked", "resolved"):
            hold("hold_" + reason + "_" + change, rows, **{change:[item["id"] for item in rows]})
        hold("hold_" + reason + "_no_chain", rows, has_chain_events=False)
    a = row("a", confirmed=False); b = row("b", txid="c" * 64); c = row("c", txid="d" * 64); d = row("d", txid="e" * 64)
    pair = lambda out, incoming, **extra: dict(out=out, **{"in":incoming}, **extra)
    for case, pairs in (
        ("dependency_direct", [pair(a,b)]), ("dependency_chain", [pair(c,d),pair(b,c),pair(a,b)]),
        ("dependency_group", [pair(a,b,group_id="g"),pair(c,d,group_id="g")]),
        ("dependency_ungrouped", [pair(a,b),pair(c,d)]),
        ("dependency_numeric_group", [pair(a,b,group_id=3),pair(c,d,group_id="3")]),
        ("dependency_false_group", [pair(a,b,group_id=False),pair(c,d,group_id="False")]),
        ("dependency_anchor", [pair(dict(a,id="synthetic",journal_transaction_id="a"),b)]),
        ("dependency_duplicate", [pair(a,b),pair(a,b)]),
        ("dependency_cycle", [pair(a,b),pair(b,c),pair(c,a)]),
    ):
        hold(case, [a,b,c,d], pairs)
        add(case + "_filter", "filter", pairs=pairs, held=["a"])
    for exception in ("excluded", "blocked"):
        hold("dependency_" + exception, [a,b,c,d], [pair(a,b),pair(b,c)], **{exception:["b"]})
    hold("dependency_missing", [a,c], [pair(a,b),pair(b,c)])
    permanent = row("permanent", txid="f" * 64)
    p2 = row("permanent-in", txid="f" * 64, direction="inbound", wallet="wallet-b")
    p3 = row("permanent-extra", txid="f" * 64, direction="inbound", wallet="wallet-c")
    e = row("e", txid="1" * 64)
    hold("dependency_mixed", [a,b,c,d,e,permanent,p2,p3],
         [pair(a,b,group_id="g"),pair(permanent,c,group_id="g"),pair(d,e,group_id="g")])
    normalized = [row("na"),row("nb",wallet="wallet-b"),row("nc",direction="inbound",wallet="wallet-c")]
    hold("hold_normalized_fee_only", normalized, [], derivation_rows=[dict(normalized[0],amount=0),*normalized[1:]], resolved=["na"])
    hold("hold_last_row", [a,dict(a,workspace_id="latest",profile_id="latest",asset="fß",wallet_id="missing")])
    ur = [dict(row("ua"),raw_json="{}",external_id="provider"), dict(row("ub",direction="inbound",wallet="wallet-b"),raw_json="{}",external_id="provider")]
    hold("hold_candidate_covered", ur, [pair(*ur)])
    hold("hold_pending_ids_kept", [a], excluded=["a"])
    add("filter_empty_anchor", "filter", pairs=[pair({}, {})], held=[""])

    for boundary in (None,""," \t ",False,0,[],{},"COINJOIN","collaborative","PayJoin","payment-in-coinjoin","sweep","unknown","ſweep"):
        add("privacy_boundary_" + repr(boundary), "privacy_evidence", row=dict(privacy_boundary=boundary,raw_json='{"source":"export"}'))
    for key in ("islikelycoinjoin","is_likely_coinjoin","isLikelyCoinJoin","likely_coinjoin","likelyCoinJoin"):
        for value in (True,False,None,""," YES ","on",1,1.0,[],{},"unknown"):
            add("privacy_boolish_" + key + repr(value), "privacy_evidence", row=dict(privacy_boundary="payjoin",raw_json=json.dumps({key:value})))
    for case, raw in (("mapping",dict(source=[False,"quoted"])),("null","null"),("array","[]"),("invalid","{"),("nonfinite",'{"source":NaN,"islikelycoinjoin":Infinity}'),("surrogate",'{"source":"\\ud800"}'),("zero",'{"source":0}')):
        add("privacy_payload_" + case,"privacy_evidence",row=dict(privacy_boundary="coinjoin",raw_json=raw))
    for case in ("invalid_graph_txid", "external_fallback", "unknown_asset", "unsupported_chain", "liquid_missing_network", "whitespace_network", "lightning_family", "different_chain", "bitcoin_missing_network", "liquid_network", "graph_txid_only", "no_txid", "no_parsed"):
        item = dict(row("custody-tax:a"), raw_json="{}")
        parsed = dict(txid="c" * 64)
        item["external_id"] = "c" * 64
        if case == "invalid_graph_txid": parsed["txid"] = "invalid"
        if case == "external_fallback": parsed.clear()
        if case == "unknown_asset": item["asset"] = "TOKEN"
        if case == "unsupported_chain": item["raw_json"] = '{"chain":"unsupported"}'
        if case == "liquid_missing_network": item["asset"] = "LBTC"
        if case == "whitespace_network": item["raw_json"] = '{"network":" "}'
        if case == "lightning_family": item["raw_json"] = '{"chain":"core-lightning"}'
        if case == "different_chain": item["raw_json"] = '{"chain":"liquid"}'
        if case == "liquid_network": item.update(asset="L-BTC",raw_json='{"chain":"liquid","network":"liquidtestnet"}')
        if case == "graph_txid_only": item["external_id"] = "provider"
        if case == "no_txid": parsed.clear(); item["external_id"] = "provider"
        if case == "no_parsed": parsed = None
        add("scope_"+case,"conflict_scope",row=item,parsed=parsed)
    for value in (None,"", "not-an-outpoint", "b"*64+":1", "B"*64+":0001", "b"*64+":-1", "b"*64+":"+str(2**150), "b"*64+":١", "b"*64+":+1", "b"*64+":1.0", "b"*64+":", "invalid:1"):
        add("outpoint_"+repr(value),"outpoint",value=value)
    for case in ("fee_only", "not_implicit", "no_observation", "untrusted", "inexact", "principal", "zero_fee", "negative_fee", "reviewed", "native", "samourai", "wallet_default", "wallet_null", "anchor"):
        item = dict(row("a"), privacy_boundary="coinjoin", observation_fee_attribution="implicit_wallet_delta")
        observations = {"a":dict(authoritative_chain_observation=True,fee_attribution="exact",principal_msat=0,amount_msat=0,amount_includes_fee=False,direction="outbound",fee_msat=25)}
        facts = dict(rows=[item],observations=observations,wallets={"wallet-a":dict(label="Ålice")},excluded=[],candidates=[],native_ids=[])
        if case == "not_implicit": item["observation_fee_attribution"] = "exact"
        if case == "no_observation": observations.clear()
        if case == "untrusted": observations["a"]["authoritative_chain_observation"] = False
        if case == "inexact": observations["a"]["fee_attribution"] = "implicit_wallet_delta"
        if case == "principal": observations["a"].update(principal_msat=1,amount_msat=1)
        if case == "zero_fee": observations["a"]["fee_msat"] = 0
        if case == "negative_fee": observations["a"]["fee_msat"] = -1
        if case in {"reviewed", "native", "samourai"}:
            observations.clear();facts[{"reviewed":"excluded","native":"native_ids","samourai":"candidates"}[case]] = ["a"]
        if case == "wallet_default": observations.clear(); facts["wallets"] = {}
        if case == "wallet_null": observations.clear(); facts["wallets"] = {"wallet-a":dict(label=None)}
        if case == "anchor": item["journal_transaction_id"] = "original"; observations["original"] = observations.pop("a")
        add("privacy_resolution_"+case, "privacy_resolution", **facts)
    return vectors


if __name__ == "__main__":
    text = json.dumps(helpers(), indent=2, ensure_ascii=True, sort_keys=True) + "\n"
    if "--write" in sys.argv: PATH.write_text(text)
    else: assert PATH.read_text() == text
