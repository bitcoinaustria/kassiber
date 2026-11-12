"""Realistic raw lifecycle/loan books with independently pinned scenario counts."""
from copy import deepcopy
import hashlib
import json

from tests.custody_snapshot_generator import snapshot, SCENARIOS
from tests.custody_safety_generator import stamp

CHANNEL_SCENARIOS = (
    "open_close", "strict_lnd", "strict_coreln", "batched", "force_trusted",
    "force_untrusted", "force_multiple", "force_duplicate", "peer_payment",
    "fee_tie", "missing_funding", "open_mismatch", "close_mismatch", "strict_zero",
    "owner_conflict", "network_conflict", "graph_conflict", "pending_open", "privacy_close",
    "splice_tag", "loan_collateral", "loan_principal", "loan_all", "loan_pending", "mixed_recorded", "mixed_pipeline", "mixed_samourai",
    "loan_wrong_direction", "loan_unknown_role", "loan_empty_ids", "strict_owner_conflict", "multiple_close_scopes",
)


def channel_snapshot(seed, *, case=None):
    case = case or CHANNEL_SCENARIOS[seed % len(CHANNEL_SCENARIOS)]
    book = deepcopy(snapshot(SCENARIOS.index("channel")))
    template = book["transactions"][0]
    amount = 500_000_000 + seed * 1000  # five million sat channel, exact msat
    fee = 200_000 + seed
    fund, close, sweep = [hashlib.sha256(f"{seed}:{name}".encode()).hexdigest() for name in ("fund", "close", "sweep")]
    book.update(transactions=[], channel_records=[], loan_legs=[], owned_index=None, receipt_sources=[])
    node = next(w for w in book["wallets"] if w["id"] == "wallet-b")
    node.update(kind="lnd", config_json=json.dumps(dict(chain="bitcoin", network="main")))
    channel_id = "lnd:channel" if case == "strict_lnd" else "coreln:channel" if case == "strict_coreln" else "legacy-channel"

    def row(id, direction, value, txid, *, prev=None, index=0):
        graph = dict(chain="bitcoin", network="main", txid=txid, status=dict(confirmed=True), vin=[dict(txid=prev or hashlib.sha256((id + ":prev").encode()).hexdigest(), vout=index)], vout=[])
        item = dict(template, id=id, external_id=txid, direction=direction, amount=value, fee=fee if direction == "outbound" else 0,
                    wallet_id="wallet-a", wallet_label="Onchain", wallet_kind="bitcoin-core", config_json=node["config_json"],
                    occurred_at="2026-01-01T00:00:00Z" if id == "funding" else "2026-02-01T00:00:00Z", raw_json=json.dumps(graph))
        book["transactions"].append(item)
        return item

    def record(tag, value, *, wallet="wallet-b", txid=None, cid=None):
        item = dict(txid=txid or (close if tag == "channel_close" else fund), outpoint=fund + ":0", tag=tag,
                    wallet_id=wallet, channel_id=cid or channel_id, amount_msat=value, raw_json="{}", config_json=node["config_json"], chain="bitcoin", network="main")
        book["channel_records"].append(item)
        return item

    opening = row("funding", "outbound", amount, fund)
    closing = row("closing", "inbound", amount - fee, close, prev=fund)
    record("channel_open", amount)
    close_record = record("channel_close", amount)
    if case == "batched":
        for rec in book["channel_records"]: rec["amount_msat"] = amount // 2
        for rec in deepcopy(book["channel_records"]):
            rec.update(channel_id="second-channel", amount_msat=amount - amount // 2)
            book["channel_records"].append(rec)
    if case.startswith("force_") or case in {"peer_payment", "fee_tie"}:
        closing["amount"] = amount // 2
        swept = row("sweep", "inbound", amount - closing["amount"] - fee, sweep, prev=close, index=1)
        attest = [dict(sweep_txid=sweep, outpoint=close + ":1")]
        if case == "force_untrusted": attest = []
        if case in {"force_multiple", "force_duplicate"}:
            swept["amount"] //= 2
            second_txid = hashlib.sha256((sweep + "second").encode()).hexdigest()
            second = row("sweep-2", "inbound", amount - closing["amount"] - swept["amount"] - fee, second_txid, prev=close, index=1 if case == "force_duplicate" else 2)
            attest.append(dict(sweep_txid=second_txid, outpoint=close + (":1" if case == "force_duplicate" else ":2")))
        if case == "peer_payment":
            closing["amount"] = amount
            swept["amount"] = 5000
            attest = []
        if case == "fee_tie":
            closing["amount"] = (amount - 200_000) // 2
            swept["amount"] = closing["amount"]
        close_record["raw_json"] = json.dumps(dict(_kassiber_provenance=dict(import_source="lnd_adapter"), channel_close_local_sweeps=attest))
    if case == "missing_funding": book["transactions"].remove(opening)
    if case == "open_mismatch": opening["amount"] += amount
    if case == "close_mismatch": closing["amount"] //= 2
    if case == "strict_zero":
        for rec in book["channel_records"]: rec.update(channel_id="lnd:zero", amount_msat=0)
    if case in {"owner_conflict", "strict_owner_conflict"}:
        if case == "strict_owner_conflict":
            for rec in book["channel_records"]: rec["channel_id"] = "lnd:conflict"
        for rec in deepcopy(book["channel_records"]):
            rec["wallet_id"] = "wallet-c"
            book["channel_records"].append(rec)
    if case == "network_conflict": close_record["network"] = "regtest"
    if case == "graph_conflict":
        raw = json.loads(closing["raw_json"]); raw["txid"] = "c" * 64; closing["raw_json"] = json.dumps(raw)
    if case == "pending_open":
        raw = json.loads(opening["raw_json"]); raw["status"]["confirmed"] = False; opening["raw_json"] = json.dumps(raw)
    if case == "privacy_close": closing["privacy_boundary"] = "coinjoin"
    if case == "splice_tag": book["channel_records"][0]["tag"] = "channel_splice"
    if case.startswith("loan_"):
        book["channel_records"] = []
        book["transactions"] = []
        roles = {"loan_collateral": ("collateral_lock", "collateral_release"), "loan_principal": ("loan_principal_received", "loan_principal_repaid"),
                 "loan_all": ("collateral_lock", "collateral_release", "loan_principal_received", "loan_principal_repaid"), "loan_pending": ("collateral_lock",), "loan_wrong_direction": ("collateral_lock", "loan_principal_received"), "loan_unknown_role": ("unknown",), "loan_empty_ids": ("collateral_lock", "collateral_release")}[case]
        for index, role in enumerate(roles):
            direction = "outbound" if role in {"collateral_lock", "loan_principal_repaid"} else "inbound"
            item = row(role, direction, amount, hashlib.sha256(f"{seed}:{role}".encode()).hexdigest())
            book["loan_legs"].append(dict(transaction_id=role, role=role))
            if case == "loan_wrong_direction": item["direction"] = "inbound" if direction == "outbound" else "outbound"
            if case == "loan_pending":
                raw = json.loads(item["raw_json"]); raw["status"]["confirmed"] = False; item["raw_json"] = json.dumps(raw)
    if case == "loan_principal":
        for index,item in enumerate(book["transactions"]):
            item.update(external_id="provider:loan-facility",external_id_kind=None,raw_json="{}",wallet_id="wallet-a" if index==0 else "wallet-c")
    if case == "loan_empty_ids":
        book["loan_legs"].extend(dict(transaction_id=id,role="collateral_lock") for id in (None,"",False,0))
    if case.startswith("mixed_"):
        from tests.custody_interpretation_generator import recorded_snapshot, samourai_snapshot
        from tests.custody_pipeline_generator import pipeline_snapshot
        other = {"mixed_recorded": lambda:recorded_snapshot(seed,case="bitcoin"), "mixed_pipeline":lambda:pipeline_snapshot(seed,case="bitcoin_mixed"), "mixed_samourai":lambda:samourai_snapshot(seed,case="tx0")}[case]()
        book["transactions"].extend(other["transactions"])
        book["owned_index"] = other["owned_index"]
        book["receipt_sources"] = other["receipt_sources"]
        known={w["id"] for w in book["wallets"]}
        book["wallets"].extend(w for w in other["wallets"] if w["id"] not in known)
    if case == "multiple_close_scopes":
        second=deepcopy(book)
        mapping={fund:hashlib.sha256((fund+"2").encode()).hexdigest(),close:hashlib.sha256((close+"2").encode()).hexdigest()}
        for rec in second["channel_records"]:
            rec.update(txid=mapping[rec["txid"]], outpoint=mapping[fund]+":0", channel_id="second")
        for item in second["transactions"]:
            item.update(id="second-"+item["id"],external_id=mapping[item["external_id"]])
            raw=json.loads(item["raw_json"]);raw["txid"]=mapping[raw["txid"]]
            for vin in raw["vin"]:vin["txid"]=mapping.get(vin["txid"],hashlib.sha256((vin["txid"]+"second").encode()).hexdigest())
            item["raw_json"]=json.dumps(raw)
        book["channel_records"].extend(second["channel_records"])
        book["transactions"].extend(second["transactions"])
    if case == "role_reservation":
        book["transactions"]=[opening]
        opening.update(amount=400_000_000,raw_json="{}")
        for i,wallet in enumerate(("wallet-b","wallet-c")):
            book["transactions"].append(dict(opening,id=f"close-{i}",wallet_id=wallet,direction="inbound",amount=250_000_000,fee=0))
        for rec in book["channel_records"]:rec.update(wallet_id="missing",txid=fund,amount_msat=400_000_000 if rec["tag"]=="channel_open" else 500_000_000)
    return stamp(book)
