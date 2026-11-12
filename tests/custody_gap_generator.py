"""Seeded realistic missing-wallet books and bounded-worklist controls."""
from copy import deepcopy
from datetime import datetime,timedelta,timezone
import json
import random

from tests.custody_snapshot_generator import snapshot as baseline

SCENARIOS=("missing_wallet","whirlpool_policy","whirlpool_tx0","untyped","fee","inclusive_fee",
           "excess","split_sources","split_returns","postmix_fan_in","multiple_wallets","separate_eras",
           "long_horizon","external_return","external_source","equal_competitors","clear_margin",
           "cross_network","cross_chain","source_capacity","return_capacity","era_capacity",
           "large_structured","large_untyped","large_empty","large_no_returns","loan_ignored",
           "blocked_competitor","excluded","same_time","unicode_labels","missing_wallet_and_disposal")


def snapshot(seed,*,case=None):
    case=SCENARIOS[seed%len(SCENARIOS)] if case is None else case
    rng=random.Random(20261029+seed)
    book=deepcopy(baseline(0))
    template=book["transactions"][0]
    book.update(transactions=[],owned_index=None,rate_rows=[],components=[],anchors={},anchor_wallets=[],revision_headers=[],
                loan_legs=[],channel_records=[],migration_issues=[],gap_reviews=[],evidence_snapshots=[],pair_dismissals=[],receipt_sources=[])
    book["profile"]["journal_input_version"]=seed
    amount=rng.randrange(1,20)*100_000_000_000
    returned=amount*99//100
    def add(identifier,inbound=False,value=None,**fields):
        wallet="wallet-b" if inbound else "wallet-a"
        ref=next(w for w in book["wallets"] if w["id"]==wallet)
        row=dict(template,id=identifier,external_id=identifier,fingerprint=f"gap-{seed}-{identifier}",external_id_kind=None,
                 wallet_id=wallet,wallet_label=ref["label"],wallet_kind="descriptor",config_json="{}",wallet_config_json="{}",
                 direction="inbound" if inbound else "outbound",asset="BTC",amount=(returned if inbound else amount) if value is None else value,
                 occurred_at="2021-01-01T00:00:00Z" if inbound else "2020-01-01T00:00:00Z",created_at="2026-01-01T00:00:00Z",
                 fee=0,amount_includes_fee=0,kind="",kind_override=None,privacy_boundary=None,raw_json="{}",excluded=0,
                 observation_authority_version=None,observation_graph_hash=None,observation_quantity_hash=None)
        row.update(fields);book["transactions"].append(row);return row
    source=add("source",privacy_boundary="coinjoin")
    dest=add("return",True)
    large=False
    if case=="whirlpool_policy":
        source.update(privacy_boundary=None,wallet_config_json=json.dumps(dict(samourai=dict(role="child",section="postmix",group_id="old-policy"))))
    elif case=="whirlpool_tx0": source.update(privacy_boundary=None,kind="whirlpool_tx0")
    elif case in {"untyped","large_untyped"}: source["privacy_boundary"]=None
    elif case=="fee": source["fee"]=amount//1000
    elif case=="inclusive_fee": source.update(amount=amount+amount//1000,fee=amount//1000,amount_includes_fee=1)
    elif case=="excess": dest["amount"]=amount*11//10
    elif case=="split_sources":
        source["amount"]=amount*6//10;add("source-2",value=amount*4//10,occurred_at="2020-02-01T00:00:00Z")
    elif case in {"split_returns","postmix_fan_in","multiple_wallets","separate_eras","return_capacity","era_capacity","large_structured"}:
        count={"split_returns":3,"postmix_fan_in":24,"multiple_wallets":3,"separate_eras":2,"return_capacity":513,"era_capacity":257,"large_structured":24}[case]
        book["transactions"].remove(dest)
        for i in range(count):
            date=(datetime(2021,1,1,tzinfo=timezone.utc)+timedelta(days=i if case!="separate_eras" else 200*i)).isoformat().replace("+00:00","Z")
            value=returned//count+(returned%count if i==count-1 else 0)
            add(f"return-{i:03}",True,value=value,occurred_at=date,wallet_id=f"destination-{i}" if case=="multiple_wallets" else "wallet-b")
        large=case=="large_structured"
    elif case=="long_horizon":dest["occurred_at"]="2040-01-01T00:00:00Z"
    elif case=="external_return":dest["kind_override"]="income"
    elif case=="external_source":source["kind_override"]="sale"
    elif case=="equal_competitors":add("competitor",True)
    elif case=="clear_margin":add("competitor",True,value=amount*8//10)
    elif case=="cross_network":dest["raw_json"]='{"network":"signet"}'
    elif case=="cross_chain":dest["raw_json"]='{"chain":"lightning"}'
    elif case=="source_capacity":
        for i in range(88):add(f"source-{i:03}",value=amount*(i+2),occurred_at="2019-01-01T00:00:00Z")
    elif case in {"large_empty","large_no_returns"}:
        large=True;book["transactions"]=[] if case=="large_empty" else [source]
    elif case=="loan_ignored":book["loan_legs"]=[dict(transaction_id="source",role="loan_principal_out")]
    elif case=="blocked_competitor":
        add("competitor",value=amount,privacy_boundary="coinjoin",occurred_at="2020-02-01T00:00:00Z")
        book["migration_issues"]=[dict(id="migration",workspace_id=source["workspace_id"],profile_id=source["profile_id"],legacy_table="pairs",legacy_source_id="old",issue_code="ambiguous",transaction_ids_json='["source"]',details_json='{}')]
    elif case=="excluded":source["excluded"]=1
    elif case=="same_time":dest["occurred_at"]=source["occurred_at"]
    elif case=="unicode_labels":source.update(wallet_label="Samourai 🐍");dest.update(wallet_label="Wien é")
    elif case=="missing_wallet_and_disposal":add("later-disposal",value=amount//10,wallet_id="wallet-b",occurred_at="2022-01-01T00:00:00Z")
    if case=="large_untyped":large=True
    # These typed missing-wallet signals are not privacy quarantines. They
    # exercise the accounting population and its capacity-only source holds.
    if case in {"source_capacity","return_capacity","era_capacity","large_structured","blocked_competitor","clear_margin"}:
        source.update(privacy_boundary=None,kind="whirlpool_tx0")
        for item in book["transactions"]:
            if item["id"]=="competitor" and item["direction"]=="outbound":
                item.update(privacy_boundary=None,kind="whirlpool_tx0")
    book["transactions"].sort(key=lambda r:(r["occurred_at"],r["created_at"],r["id"]))
    book["gap_worklist"]={"large_book":large,"rows":deepcopy(book["transactions"])}
    return book


def signature(result):
    if not isinstance(result,dict):
        from tests.custody_snapshot import to_jsonable
        result=to_jsonable(result,redact_config=False)
    return ",".join(map(str,(len(result["candidates"]),len(result["accounting_candidates"]),result["promotion_eligible_count"],
                              len(result["blocking_source_ids"]),int(result["search_complete"]),result["limit_kind"],result["candidate_count"])))
