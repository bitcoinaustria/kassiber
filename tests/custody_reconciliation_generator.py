"""Raw recovered native routes and reviewed shortcuts, with explicit controls."""
from copy import deepcopy
import hashlib
import json

from tests.custody_interpretation_generator import recorded_snapshot
from tests.custody_snapshot_generator import component_snapshot
from tests.custody_safety_generator import stamp

SCENARIOS = (
    "recovered", "same_time", "reverse_txid", "wallet_cycle", "source_fee",
    "intermediate_fee", "residual_suspense", "residual_fee", "extra_activity",
    "outside_activity", "zero_activity", "untrusted_source", "untrusted_middle",
    "inexact_fee", "partial_route", "direct_only", "blocked_middle",
    "protected_middle", "economic_terms", "draft_component", "two_sources",
    "fifo_mismatch", "two_targets", "liquid", "inclusive_fee",
)


def seal(book):
    from kassiber.core.custody_component_facts import ComponentFacts
    from kassiber.core.custody_quantity_store import _canonical_component_snapshots, _commitment_detail_hash
    book["anchors"] = {row["id"]: row for row in book["transactions"]}
    book["anchor_wallets"] = deepcopy(book["wallets"])
    for component in book["components"]:
        h = component["header"]
        h.update(expected_leg_count=len(component["legs"]), expected_allocation_count=len(component["allocations"]), expected_economic_term_count=len(component["economic_terms"]))
        snapshots = _canonical_component_snapshots(ComponentFacts(book), {**h, "legs": component["legs"]})
        committed = sorted((s.quantity_hash, _commitment_detail_hash(s)) for s in snapshots)
        component["commitments"] = [dict(id=f"{h['id']}:commit:{i}", component_id=h["id"], workspace_id=h["workspace_id"], profile_id=h["profile_id"], ordinal=i, quantity_hash=q, detail_hash=d, created_at=h["created_at"]) for i, (q, d) in enumerate(committed)]
        h["expected_evidence_count"] = len(committed)
    book["gap_worklist"]["rows"] = deepcopy(book["transactions"])
    return book


def snapshot(seed, *, case=None):
    case = case or SCENARIOS[seed % len(SCENARIOS)]
    book = recorded_snapshot(seed, case="liquid_main" if case == "liquid" else "bitcoin")
    template = component_snapshot(0, case="balanced")["components"][0]
    source = next(r for r in book["transactions"] if r["direction"] == "outbound")
    target = next(r for r in book["transactions"] if r["direction"] == "inbound")
    quantity = 1_000_000 + (seed % 100) * 1000
    t1, t2 = "2026-01-01T00:00:00Z", "2026-01-02T00:00:00Z"
    first, second = "22" * 32, "11" * 32
    if case == "reverse_txid": first, second = second, first
    if case in {"same_time", "reverse_txid", "wallet_cycle"}: t2 = t1
    destination_wallet = "wallet-a" if case == "wallet_cycle" else "wallet-c"
    source.update(id="route-out", wallet_id="wallet-a", amount=quantity, fee=0, occurred_at=t1)
    middle_in = dict(target, id="middle-in", wallet_id="wallet-b", amount=quantity, fee=0, occurred_at=t1)
    middle_out = dict(source, id="middle-out", wallet_id="wallet-b", occurred_at=t2)
    target.update(id="route-in", wallet_id=destination_wallet, amount=quantity, fee=0, occurred_at=t2)
    book["transactions"] = [source, middle_in, middle_out, target]
    for row, txid, parent in ((source, first, "ff" * 32), (middle_in, first, "ff" * 32), (middle_out, second, first), (target, second, first)):
        raw = json.loads(row["raw_json"])
        raw.update(txid=txid, vin=[dict(txid=parent, vout=0)], status=dict(confirmed=True))
        row.update(external_id=txid, raw_json=json.dumps(raw))
    if case in {"source_fee", "inclusive_fee"}:
        source["fee"] = 1000
        if case == "inclusive_fee": source.update(amount=quantity+1000, amount_includes_fee=1)
    if case in {"intermediate_fee", "residual_suspense", "residual_fee"}:
        middle_out.update(amount=quantity-1000, fee=1000)
        target["amount"] = quantity-1000
    if case == "partial_route": middle_out["amount"] = target["amount"] = quantity-1000
    if case in {"extra_activity", "outside_activity", "zero_activity"}:
        extra = dict(middle_in, id="unrelated", amount=0 if case == "zero_activity" else 1, occurred_at="2025-01-01T00:00:00Z" if case == "outside_activity" else t1, external_id="33" * 32)
        raw = json.loads(extra["raw_json"]); raw["txid"] = extra["external_id"]; extra["raw_json"] = json.dumps(raw)
        book["transactions"].append(extra)
    if case in {"untrusted_source", "untrusted_middle"}: (source if case.endswith("source") else middle_out)["observation_authority_version"] = None
    if case == "inexact_fee": middle_out["observation_fee_attribution"] = "unknown"
    if case == "blocked_middle":
        raw = json.loads(middle_out["raw_json"]); raw["status"] = dict(confirmed=False); middle_out["raw_json"] = json.dumps(raw)
        raw = json.loads(target["raw_json"]); raw["status"] = dict(confirmed=False); target["raw_json"] = json.dumps(raw)
    if case == "direct_only": book["transactions"] = [source, middle_in]; target = middle_in
    c = deepcopy(template)
    h = c["header"]
    h.update(id="reviewed-route", lineage_id="reviewed-route", workspace_id=source["workspace_id"], profile_id=source["profile_id"], evidence_kind="manual_claim", evidence_grade="reviewed", conservation_mode="quantity", revision=1)
    leg_template = c["legs"][0]
    def leg(i, role, row, n, **kw):
        raw = json.loads(source["raw_json"])
        return dict(leg_template, id=f"leg:{i}", component_id=h["id"], workspace_id=h["workspace_id"], profile_id=h["profile_id"], ordinal=i, role=role, transaction_id=row["id"] if row else None, anchor_transaction_id=row["id"] if row else None, wallet_id=row["wallet_id"] if row else None, rail="liquid" if case == "liquid" else "bitcoin", chain=raw["chain"], network=raw["network"], asset=source["asset"], amount_msat=n, occurred_at=row["occurred_at"] if row else t2, **kw)
    c["legs"] = [leg(0, "source", source, quantity+source["fee"]), leg(1, "destination", target, target["amount"])]
    if source["fee"]: c["legs"].append(leg(2, "fee", source, source["fee"]))
    if case in {"intermediate_fee", "residual_suspense", "residual_fee", "partial_route"}:
        c["legs"].append(leg(2, "fee" if case in {"intermediate_fee", "residual_fee"} else "suspense", None, 1000, notes="reviewed_residual:network_fee" if case in {"intermediate_fee", "residual_fee"} else None))
    c["allocations"] = [dict(id=f"allocation:{i}", component_id=h["id"], workspace_id=h["workspace_id"], profile_id=h["profile_id"], created_at=h["created_at"], ordinal=i, source_leg_id="leg:0", sink_leg_id=sink["id"], source_amount_msat=sink["amount_msat"], sink_amount_msat=sink["amount_msat"]) for i, sink in enumerate(c["legs"][1:])]
    c["economic_terms"] = []
    book["components"] = [c]
    if case == "draft_component": h["state"] = "draft"
    if case == "economic_terms": c["economic_terms"] = [dict(id="term", component_id=h["id"], workspace_id=h["workspace_id"], profile_id=h["profile_id"], ordinal=0, term_kind="transaction_pair", source_leg_id="leg:0", target_leg_id="leg:1", review_kind="manual", tax_policy="reviewed", created_at=h["created_at"])]
    if case == "protected_middle":
        other = deepcopy(c)
        other["header"].update(id="protect-middle", lineage_id="protect-middle")
        other["legs"] = [leg(9, "source", middle_out, middle_out["amount"])]
        other["legs"][0]["component_id"] = "protect-middle"
        other["allocations"] = []
        book["components"].append(other)
    if case in {"two_sources", "fifo_mismatch", "two_targets"}:
        # Independent routes sharing the same intermediary credit pool exercise
        # seed insertion order and exact allocation identity, not only totals.
        extra_source = dict(source, id="extra-out", wallet_id="wallet-d", occurred_at=t1)
        extra_in = dict(middle_in, id="extra-middle", external_id="44" * 32)
        raw = json.loads(extra_in["raw_json"]); raw["txid"] = extra_in["external_id"]; extra_in["raw_json"] = json.dumps(raw)
        extra_source.update(external_id=extra_in["external_id"], raw_json=extra_in["raw_json"])
        middle_out["amount"] *= 2; target["amount"] *= 2
        book["transactions"].extend([extra_source, extra_in])
        c["legs"][1]["amount_msat"] *= 2
        c["legs"].append(leg(3, "source", extra_source, quantity))
        c["allocations"].append(dict(c["allocations"][0], id="allocation:extra", ordinal=1, source_leg_id="leg:3"))
        if case in {"fifo_mismatch", "two_targets"}:
            extra_target = dict(target, id="extra-target", wallet_id="wallet-e", amount=quantity)
            target["amount"] = quantity
            book["transactions"].append(extra_target)
            c["legs"][1]["amount_msat"] = quantity
            c["legs"].append(leg(4, "destination", extra_target, quantity))
            c["allocations"][1]["sink_leg_id"] = "leg:4"
            if case == "two_targets":
                for allocation in c["allocations"]: allocation["sink_leg_id"] = "leg:4" if allocation["sink_leg_id"] == "leg:1" else "leg:1"
    wallet_template = book["wallets"][0]
    book["wallets"] = [dict(wallet_template, id=wallet, label=wallet, config_json=source["config_json"]) for wallet in sorted({row["wallet_id"] for row in book["transactions"]})]
    book.update(owned_index=None, receipt_sources=[], rate_rows=[], revision_headers=[])
    for leg in c["legs"]:
        if leg["role"] == "suspense": leg["occurred_at"] = t1
    stamp(book)
    return seal(book)
