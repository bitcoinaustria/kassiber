"""Seeded raw safety books; every scenario has explicit two-engine controls."""
from copy import deepcopy
import hashlib
import json

from kassiber.core.chain_observer.provenance import canonical_graph_hash, canonical_observed_quantity_hash
from tests.custody_interpretation_generator import recorded_snapshot
from tests.custody_pipeline_generator import pipeline_snapshot

BOUNDARIES = ("coinjoin", "collaborative", "payjoin", "payment_in_coinjoin", "sweep")
SAFETY_SCENARIOS = (
    "pending_pair", "pending_fanout", "pending_consolidation", "pending_ownership",
    "replaced_spend", "conflict_none_confirmed", "conflict_both_confirmed",
    "unscoped_provider", "blank_identity", "unresolved_fanout", "unresolved_consolidation", "unresolved_many_many",
    *("privacy_" + name for name in BOUNDARIES),
    "privacy_exact_pair", "privacy_fanout", "privacy_consolidation", "privacy_untrusted_pair", "privacy_rowless",
    "confirmed_pair", "status_string", "legacy_missing_status", "privacy_unknown",
    "closed_newer_pending", "closed_newer_confirmed", "closed_equal_disagreement", "closed_invalid_timestamp",
    "liquid_pending", "liquid_privacy_pair", "samourai_unverified", "samourai_verified",
    "normalized_fee_only", "privacy_reviewed", "dependency_pending", "dependency_privacy", "lightning_chain_no_hash",
)


def stamp(book):
    for row in book["transactions"]:
        row["observation_graph_hash"] = canonical_graph_hash(row["raw_json"])
        row["observation_quantity_hash"] = canonical_observed_quantity_hash(row)
    book["transactions"].sort(key=lambda row: (row["occurred_at"], row["created_at"], row["id"]))
    book["gap_worklist"]["rows"] = deepcopy(book["transactions"])
    return book


def safety_snapshot(seed, *, case=None):
    case = case or SAFETY_SCENARIOS[seed % len(SAFETY_SCENARIOS)]
    if case.startswith("dependency_") or case == "lightning_chain_no_hash":
        book = recorded_snapshot(seed, case="lightning_lnd")
        source = next(row for row in book["transactions"] if row["direction"] == "outbound")
        raw = json.loads(source["raw_json"])
        raw.update(txid=hashlib.sha256(f"pending-rail:{seed}".encode()).hexdigest(), chain="bitcoin", network="main", status={"confirmed":False})
        raw.update(vin=[dict(txid="e" * 64, vout=0, prevout=dict(scriptpubkey="0014" + "11" * 20, value_sats=1000))], vout=[dict(n=0, scriptpubkey="0014" + "22" * 20, value_sats=997)])
        source["raw_json"] = json.dumps(raw)
        if case == "lightning_chain_no_hash":
            source["raw_json"] = json.dumps(dict(raw, chain="lightning", status={"confirmed":True}))
            for row in book["transactions"]: row["payment_hash"] = None
        if case == "dependency_privacy":
            next(row for row in book["transactions"] if row["direction"] == "inbound")["privacy_boundary"] = "coinjoin"
    elif case.endswith(("fanout", "consolidation", "ownership")) and case.split("_", 1)[0] in {"pending", "privacy"}:
        book = pipeline_snapshot(seed, case="bitcoin_" + case.split("_", 1)[1])
    elif case in {"samourai_unverified", "samourai_verified"}:
        from tests.custody_interpretation_generator import samourai_snapshot
        book = samourai_snapshot(seed, case="untrusted_source" if case.endswith("unverified") else "tx0")
        for row in book["transactions"]:
            row["privacy_boundary"] = "coinjoin"
    elif case == "privacy_reviewed":
        from tests.custody_snapshot_generator import component_snapshot
        book = component_snapshot(seed, case="balanced")
        for row in book["transactions"]:
            row["privacy_boundary"] = "sweep"
    elif case == "normalized_fee_only":
        from tests.custody_graph_vectors import match
        book = pipeline_snapshot(seed, case="bitcoin_consolidation")
        rows = book["transactions"]
        raw = json.loads(rows[0]["raw_json"])
        scripts = {matches[0]["wallet_id"]: script for script, matches in book["owned_index"]["by_script"].items()}
        scripts["wallet-d"] = "0014" + "99" * 20
        book["wallets"].append(dict(book["wallets"][0], id="wallet-d", label="Fee destination"))
        book["owned_index"]["by_script"][scripts["wallet-d"]] = [match("wallet-d")]
        raw["vin"] = [dict(txid=f"{index+1:02x}" * 32, vout=0, prevout=dict(scriptpubkey=scripts[wallet], value_sats=1000)) for index, wallet in enumerate(("wallet-a", "wallet-b", "wallet-c"))]
        raw["vout"] = [dict(n=0, scriptpubkey=scripts["wallet-d"], value_sats=1500)]
        raw["fee"] = 1500
        destination = deepcopy(next(row for row in rows if row["direction"] == "inbound"))
        for row in rows:
            row.update(direction="outbound", amount=1000000, fee=0)
        destination.update(id="fee-destination", wallet_id="wallet-d", direction="inbound", amount=1500000, fee=0)
        rows.append(destination)
        for row in rows:
            row.update(privacy_boundary="collaborative", observation_fee_attribution="implicit_wallet_delta")
            row["raw_json"] = json.dumps(dict(raw, observer_owned_scripts=[scripts[row["wallet_id"]]]))
        book["receipt_sources"] = []
    elif case in {"replaced_spend", "conflict_none_confirmed", "conflict_both_confirmed"}:
        book = recorded_snapshot(seed, case="bitcoin")
        original = deepcopy(book["transactions"])
        for index, row in enumerate(original):
            row["id"] = "winner:" + row["id"]
            raw = json.loads(row["raw_json"])
            raw.update(status={"confirmed": case != "conflict_none_confirmed"}, vin=[dict(txid="b" * 64, vout=1, prevout=dict(scriptpubkey="0014" + "11" * 20, value_sats=10000))], vout=[dict(n=0, scriptpubkey="0014" + "22" * 20, value_sats=9997)])
            row["raw_json"] = json.dumps(raw)
        losers = deepcopy(original)
        txid = hashlib.sha256(f"replacement:{seed}".encode()).hexdigest()
        for row in losers:
            row["id"] = row["id"].replace("winner:", "loser:")
            row["external_id"] = txid
            raw = json.loads(row["raw_json"])
            raw.update(txid=txid, status={"confirmed": case == "conflict_both_confirmed"})
            row["raw_json"] = json.dumps(raw)
        book["transactions"] = original + losers
    elif case == "privacy_rowless":
        book = pipeline_snapshot(seed, case="bitcoin_ownership")
    else:
        book = recorded_snapshot(seed, case="liquid_main" if case.startswith("liquid_") else "bitcoin")

    if case in {"unscoped_provider", "blank_identity"} or case.startswith("privacy_") and case[8:] in BOUNDARIES or case == "privacy_unknown":
        for row in book["transactions"]:
            row.update(raw_json="{}", external_id="" if case == "blank_identity" else f"import:{seed}", external_id_kind=None, observation_authority_version=None)
        if case.startswith("privacy_"):
            book["transactions"] = [book["transactions"][0]]
            book["transactions"][0]["privacy_boundary"] = case[8:]
            book["transactions"][0]["raw_json"] = json.dumps(dict(source="synthetic-export", likelyCoinJoin=" YES "))
    if case.startswith("unresolved_"):
        for row in book["transactions"]:
            row["observation_authority_version"] = None
        source = next(row for row in book["transactions"] if row["direction"] == "outbound")
        receipt = next(row for row in book["transactions"] if row["direction"] == "inbound")
        receipt["amount"] -= 7000
        if case.endswith(("fanout", "many_many")):
            book["transactions"].append(dict(receipt, id="extra-receipt", wallet_id="wallet-c", amount=1))
        if case.endswith(("consolidation", "many_many")):
            book["transactions"].append(dict(source, id="extra-source", wallet_id="wallet-c", amount=3))
    if case.startswith("pending_") or case == "liquid_pending":
        for row in book["transactions"]:
            raw = json.loads(row["raw_json"]); raw["status"] = {"confirmed": False}; row["raw_json"] = json.dumps(raw)
    if case in {"confirmed_pair", "status_string", "legacy_missing_status"}:
        for row in book["transactions"]:
            raw = json.loads(row["raw_json"])
            if case != "legacy_missing_status": raw["status"] = {"confirmed": True if case == "confirmed_pair" else "false"}
            row["raw_json"] = json.dumps(raw)
    if case in {"privacy_exact_pair", "privacy_fanout", "privacy_consolidation", "privacy_untrusted_pair", "privacy_rowless", "liquid_privacy_pair"}:
        for row in book["transactions"]: row["privacy_boundary"] = " PayJoin "
        if case == "privacy_untrusted_pair": book["transactions"][0]["observation_authority_version"] = None
    if case.startswith("closed_"):
        incoming = next(row for row in book["transactions"] if row["direction"] == "inbound")
        outgoing = next(row for row in book["transactions"] if row["direction"] == "outbound")
        for row, confirmed, instant in ((outgoing, True, "2026-01-01T01:00:00+01:00"), (incoming, case == "closed_newer_confirmed", "2026-01-01T00:00:00.000001Z")):
            raw = json.loads(row["raw_json"]); raw["status"] = {"confirmed": confirmed}; row["raw_json"] = json.dumps(raw)
            row["observation_observed_at"] = "invalid" if case == "closed_invalid_timestamp" else ("2026-01-01" if case == "closed_equal_disagreement" else instant)
        if case == "closed_newer_confirmed":
            raw = json.loads(outgoing["raw_json"]); raw["status"]["confirmed"] = False; outgoing["raw_json"] = json.dumps(raw)
    return stamp(book)
