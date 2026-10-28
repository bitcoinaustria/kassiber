"""Raw, independently observed owned-wallet fan-outs on Bitcoin and Liquid."""
from copy import deepcopy
import json
import random

from kassiber.core.chain_observer.provenance import canonical_graph_hash, canonical_observed_quantity_hash
from tests.custody_interpretation_generator import recorded_snapshot

FANOUT_SCENARIOS = (
    "bitcoin", "liquid_main", "liquid_test", "liquid_regtest", "liquid_asset_id", "bitcoin_graph", "liquid_graph",
    "three_destinations", "equal_amount_ties", "amount_order", "provider_ids",
    "multiple_events", "exact_msat", "value_pricing", "exact_value_pricing",
    "existing_rate", "existing_exact_rate", "zero_value", "negative_value",
    "missing_value", "zero_exact_value_fallback", "zero_rate", "whitespace_value",
    "unicode_value", "nan_value", "nan_payload", "infinite_value", "negative_infinite_value",
    "untrusted_source", "untrusted_target", "tampered_quantity", "fee_inclusive",
    "partial_total", "over_total", "duplicate_wallet", "same_wallet",
    "one_destination", "multiple_sources", "zero_source", "zero_destination",
    "negative_destination", "unknown_direction", "asset_separation", "network_separation",
    "synthetic_source", "synthetic_destination", "reviewed_reservation",
)


def fanout_snapshot(seed, *, case=None):
    case = case or FANOUT_SCENARIOS[seed % len(FANOUT_SCENARIOS)]
    base = case if case.startswith("liquid_") and case != "liquid_graph" else "liquid_main" if case == "liquid_graph" else "bitcoin"
    book = recorded_snapshot(seed, case=base)
    rng = random.Random(20261029 + seed)
    rows = book["transactions"]
    source = next(row for row in rows if row["direction"] == "outbound")
    target = next(row for row in rows if row["direction"] == "inbound")
    targets = [target, {**target, "id": "third-receipt", "wallet_id": "wallet-c", "wallet_label": "Savings"}]
    rows.append(targets[-1])
    if case == "three_destinations":
        targets.append({**target, "id": "fourth-receipt", "wallet_id": "wallet-d", "wallet_label": "Vault"})
        rows.append(targets[-1])
        book["wallets"].append({**book["wallets"][0], "id": "wallet-d", "label": "Vault"})
    for i, row in enumerate(targets):
        row["amount"] = rng.randint(10_000, 10**9) * 1000 + i * 1000
    if case == "equal_amount_ties":
        for row, name in zip(targets, ("z-receipt", "a-receipt")):
            row.update(amount=123_456_000, id=name)
    if case == "amount_order":
        targets[0].update(amount=200_000_000, id="a-receipt")
        targets[1].update(amount=100_000_000, id="z-receipt")
    if case == "exact_msat":
        targets[0]["amount"] = (1 << 53) + 123
        targets[1]["amount"] = (1 << 53) + 124
    source.update(amount=sum(row["amount"] for row in targets), fee=3000)
    if case in {"bitcoin_graph", "liquid_graph"}:
        graph = json.loads(source["raw_json"])
        asset = {"asset_id": "a" * 64, "asset": "LBTC"} if case == "liquid_graph" else {}
        graph["vin"] = [{"txid": "b" * 64, "vout": 1, "prevout": {"scriptpubkey": "0014" + "1" * 40, "value_sats": source["amount"] // 1000 + 3, **asset}}]
        graph["vout"] = [{"n": i, "scriptpubkey": "0014" + str(i + 2) * 40, "value_sats": row["amount"] // 1000, **asset} for i, row in enumerate(targets)]
        if case == "liquid_graph": graph["vout"].append({"n": len(targets), "scriptpubkey": "", "value_sats": 3, "role": "fee", **asset})
        graph["fee"] = 3
        book["receipt_sources"] = [{"wallet_id": row["wallet_id"], "profile_id": book["profile"]["id"], "wallet_label": row["wallet_label"], "chain": graph["chain"], "network": graph["network"], "source": "inventory", "wallet_kind": row["wallet_kind"], "script_pubkey": output["scriptpubkey"], "deprecated": False} for row, output in zip(targets, graph["vout"])]
        for row in rows: row["raw_json"] = json.dumps(graph)
    for row in rows:
        row.update(fiat_value=None, fiat_value_exact=None, fiat_rate=None, fiat_rate_exact=None)
    if case in {"value_pricing", "exact_value_pricing", "existing_rate", "existing_exact_rate", "zero_value", "negative_value", "zero_exact_value_fallback", "zero_rate"}:
        source["fiat_value"] = "123.45678901234567890123456789012345"
    if case == "exact_value_pricing": source["fiat_value_exact"] = "987.65432109876543210987654321098765"
    if case == "existing_rate": source["fiat_rate"] = "12345.67"
    if case == "existing_exact_rate": source["fiat_rate_exact"] = "98765.4321"
    if case == "zero_rate": source["fiat_rate"] = 0
    if case == "zero_value": source["fiat_value"] = "0"
    if case == "negative_value": source["fiat_value"] = "-123.45000"
    if case == "zero_exact_value_fallback": source.update(fiat_value_exact=0, fiat_value="321.123")
    if case == "whitespace_value": source["fiat_value"] = "\u2003 +1_23.4500\u00a0"
    if case == "unicode_value": source["fiat_value"] = "١٢٣.٤٥٠٠"
    if case == "nan_value": source["fiat_value"] = "NaN"
    if case == "nan_payload": source["fiat_value"] = "-NaN٠٠١٢"
    if case == "infinite_value": source["fiat_value"] = "  +iN_FiNiTy\t"
    if case == "negative_infinite_value": source["fiat_value"] = "-Inf"
    if case == "untrusted_source": source["observation_authority_version"] = None
    if case == "untrusted_target": targets[0]["observation_authority_version"] = None
    if case == "fee_inclusive": source["amount_includes_fee"] = 1
    if case == "partial_total": source["amount"] += 1000
    if case == "over_total": source["amount"] -= 1000
    if case == "duplicate_wallet": targets[1]["wallet_id"] = targets[0]["wallet_id"]
    if case == "same_wallet": targets[1]["wallet_id"] = source["wallet_id"]
    if case == "one_destination": rows.remove(targets[1])
    if case == "multiple_sources": rows.append({**source, "id": "second-source", "wallet_id": "wallet-d"})
    if case == "zero_source": source["amount"] = 0
    if case in {"zero_destination", "negative_destination"}:
        targets[0]["amount"] = 0 if case == "zero_destination" else -1000
        source["amount"] = sum(row["amount"] for row in targets)
    if case == "unknown_direction": targets[1]["direction"] = "unknown"
    if case == "asset_separation": targets[1]["asset"] = "OTHER"
    if case == "network_separation":
        targets[1]["config_json"] = '{"chain":"bitcoin","network":"regtest"}'
        raw = json.loads(targets[1]["raw_json"]); raw["network"] = "regtest"; targets[1]["raw_json"] = json.dumps(raw)
    if case == "synthetic_source": source["id"] = "recorded-fanout:already-split"
    if case == "synthetic_destination": targets[1]["id"] = "owned-derive:already-split"
    if case == "provider_ids":
        for row in rows: row["external_id"] = "provider-record:" + row["id"]
    if case == "reviewed_reservation":
        from tests.custody_snapshot_generator import component_snapshot
        authored = component_snapshot(0)
        for key in ("profile", "components", "anchors", "anchor_wallets", "revision_headers"): book[key] = authored[key]
        source["id"] = "out"
    if case == "multiple_events":
        extra = deepcopy(rows)
        for row in extra:
            row["id"] = "aaa-" + row["id"]
            row["external_id"] = "d" * 64
            raw = json.loads(row["raw_json"]); raw["txid"] = "d" * 64; row["raw_json"] = json.dumps(raw)
        rows.extend(extra)
    for row in rows:
        row.update(observation_graph_hash=canonical_graph_hash(row["raw_json"]), observation_quantity_hash=canonical_observed_quantity_hash(row))
    if case == "tampered_quantity": targets[0]["observation_quantity_hash"] = "0" * 64
    rows.sort(key=lambda row: (row["occurred_at"], row["created_at"], row["id"]))
    book["gap_worklist"]["rows"] = deepcopy(rows)
    return book
