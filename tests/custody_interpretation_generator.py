"""Seeded raw facts reaching successful and blocked interpreter paths."""
from copy import deepcopy
import hashlib
import json
import random

from kassiber.core.chain_observer.provenance import canonical_graph_hash, canonical_observed_quantity_hash
from tests.custody_snapshot_generator import snapshot, SCENARIOS

SAMOURAI_SCENARIOS = (
    "tx0", "tx0_fee", "mixing", "remix", "fanout", "badbank",
    "untrusted_source", "untrusted_target", "unknown_fee", "implicit_delta",
    "oversize", "wrong_asset", "group_mismatch", "multiple_sources",
    "invalid_role", "empty_group", "empty_section", "bool_group", "dict_group",
    "list_group", "nonprintable_group", "numeric_group", "false_presence_gate",
    "decoded_config", "mixing_residual", "fee_inclusive", "tampered_hash",
    "sorted_targets", "duplicate_target", "reviewed_reservation", "multiple_groups",
    "post_unicode14_group", "implicit_mix_residual", "exact_msat",
)


def samourai_snapshot(seed, *, case=None):
    case = case or SAMOURAI_SCENARIOS[seed % len(SAMOURAI_SCENARIOS)]
    rng = random.Random(20261027 + seed)
    book = deepcopy(snapshot(SCENARIOS.index("fanout" if case in {"fanout", "sorted_targets", "duplicate_target"} else "samourai")))
    rows = book["transactions"]
    source = next(row for row in rows if row["direction"] == "outbound")
    targets = [row for row in rows if row["direction"] == "inbound"]
    amount = rng.randint(100, 10**8) * 1000
    if case == "exact_msat": amount = (1 << 53) + 23
    source.update(amount=amount, fee=3000)
    for i, target in enumerate(targets):
        target.update(amount=amount // len(targets))
        if case == "sorted_targets":
            target["id"] = ["z-receive", "a-receive"][i]
    if len(targets) > 1:
        targets[-1]["amount"] += amount - sum(row["amount"] for row in targets)
    residual = max(1, amount // 100)
    if case in {"tx0_fee", "unknown_fee", "implicit_delta", "mixing_residual", "implicit_mix_residual", "fee_inclusive"}:
        source["amount"] += residual
    if case == "fee_inclusive":
        source.update(amount=source["amount"] + source["fee"], amount_includes_fee=1)
    if case == "oversize":
        targets[0]["amount"] = amount + 1
    if case == "wrong_asset":
        targets[0]["asset"] = "OTHER"
    if case == "multiple_sources":
        rows.append({**source, "id": "second-source", "wallet_id": "wallet-c", "wallet_label": "Savings", "fee": 0})
    if case == "duplicate_target":
        rows.append({**targets[0], "id": "duplicate-target"})
    group = {
        "bool_group": True,
        "dict_group": {"z": [True, "'quoted'"], "a": None},
        "list_group": ["钱包", False, -1.25],
        "nonprintable_group": "\x1c group\u200b\n ' \" \U000e0001",
        "post_unicode14_group": ["\U0001e4d0\U0001e030\x1c\u200b"],
        "numeric_group": 17.0,
        "float_overflow": 17.0,
        "empty_group": [],
    }.get(case, "generated")
    for row in rows:
        section = "deposit" if row["direction"] == "outbound" else "premix"
        if case in {"mixing", "mixing_residual", "implicit_mix_residual"}:
            section = "premix" if row["direction"] == "outbound" else "postmix"
        elif case == "remix":
            section = "postmix"
        elif case == "badbank" and row["direction"] == "inbound":
            section = "badbank"
        if case == "empty_section": section = "\u2003\x1f"
        metadata = {"role": "parent" if case == "invalid_role" else "child", "group_id": group, "section": "\u2003" + section.upper() + "\x1c"}
        if case == "group_mismatch" and row["direction"] == "inbound": metadata["group_id"] = "other"
        config = {"chain": "bitcoin", "network": "main", "samourai": metadata}
        row.update(config_json=config if case == "decoded_config" else json.dumps(config), config_text_mentions_samourai=case != "false_presence_gate")
        if case == "float_overflow": row["config_json"] = row["config_json"].replace('"group_id": 17.0', '"group_id": 1e400')
        # A closed graph need not be complete ownership proof. This preserves
        # implicit attribution and avoids accidentally exercising only exact fees.
        graph = {"chain": "bitcoin", "network": "main", "txid": hashlib.sha256(f"samourai:{seed}".encode()).hexdigest()}
        row.update(raw_json=json.dumps(graph), external_id=graph["txid"], external_id_kind="txid",
                   observation_fee_attribution="implicit_wallet_delta" if case in {"implicit_delta", "implicit_mix_residual"} else "unknown" if case == "unknown_fee" else "exact")
        row.update(observation_graph_hash=canonical_graph_hash(row["raw_json"]), observation_quantity_hash=canonical_observed_quantity_hash(row))
        if (case == "untrusted_source" and row["direction"] == "outbound") or (case == "untrusted_target" and row["direction"] == "inbound"):
            row["observation_authority_version"] = None
        if case == "tampered_hash" and row["direction"] == "inbound": row["observation_quantity_hash"] = "0" * 64
    # Receipt reconciliation cannot be fabricated from a Python quantity verdict.
    # Multiple targets are separate owned scripts in a shared raw graph.
    book["owned_index"] = None
    if len(targets) > 1:
        scripts = {row["wallet_id"]: "0014" + hashlib.sha256(row["wallet_id"].encode()).hexdigest()[:40] for row in targets}
        graph["vout"] = [{"n": i, "scriptpubkey": scripts[row["wallet_id"]], "value_sats": row["amount"] // 1000} for i, row in enumerate(targets)]
        # Use sat-exact target totals for these fan-out books.
        for row in targets: row["amount"] = row["amount"] // 1000 * 1000
        source["amount"] = sum(row["amount"] for row in targets)
        book["receipt_sources"] = [{"wallet_id": row["wallet_id"], "profile_id": book["profile"]["id"], "wallet_label": row["wallet_label"], "chain": "bitcoin", "network": "main", "source": "inventory", "wallet_kind": row["wallet_kind"], "script_pubkey": scripts[row["wallet_id"]], "deprecated": False} for row in targets]
        for row in rows:
            row["raw_json"] = json.dumps(graph)
            row.update(observation_graph_hash=canonical_graph_hash(row["raw_json"]), observation_quantity_hash=canonical_observed_quantity_hash(row))
    if case == "multiple_groups":
        second = deepcopy(rows)
        for row in second:
            row["id"] += "-second"
            config = json.loads(row["config_json"])
            config["samourai"]["group_id"] = "aaa\u200b"
            row["config_json"] = json.dumps(config)
            row["external_id"] = "f" * 64
            row["raw_json"] = json.dumps({"chain": "bitcoin", "network": "main", "txid": "f" * 64})
            row.update(observation_graph_hash=canonical_graph_hash(row["raw_json"]), observation_quantity_hash=canonical_observed_quantity_hash(row))
        rows.extend(second)
    if case == "reviewed_reservation":
        # Reuse the stored raw authored header/children and rebind its exact
        # anchors; materialization recomputes draft/active status and reservations.
        from tests.custody_snapshot_generator import component_snapshot
        authored = component_snapshot(0)
        book["profile"] = authored["profile"]
        book.update(components=authored["components"], anchors=authored["anchors"], anchor_wallets=authored["anchor_wallets"], revision_headers=authored["revision_headers"])
        rows[0]["id"] = "out" if rows[0]["direction"] == "outbound" else "in"
    book["transactions"].sort(key=lambda row: (row["occurred_at"], row["created_at"], row["id"]))
    book["gap_worklist"]["rows"] = deepcopy(rows)
    return book


RECORDED_SCENARIOS = (
    "bitcoin", "bitcoin_upper_txid", "bitcoin_typed_external", "bitcoin_timestamp_skew",
    "bitcoin_fee_inclusive", "bitcoin_implicit_residual", "bitcoin_partial", "exact_msat",
    "liquid_main", "liquid_test", "liquid_regtest", "liquid_asset_id",
    "lightning_lnd", "lightning_cln", "lightning_circular", "lightning_hash_case",
    "untrusted_source", "untrusted_target", "tampered_quantity", "lightning_chronology",
    "untrusted_both", "same_wallet", "zero_inbound", "negative_outbound", "multiple_sources",
    "multiple_targets", "different_txid", "network_separation", "lightning_shortfall",
    "lightning_forged_source", "lightning_forged_lnd", "lightning_bad_hash", "lightning_wrong_kind", "lightning_chain_hash", "lightning_duplicate",
    "lightning_no_network", "lightning_asset_case", "liquid_missing_identity",
    "liquid_asset_separation", "synthetic_id", "reviewed_reservation", "multiple_events",
    "nonpositive_placeholder", "nonfinite_graph_identity", "lightning_skew_boundary", "lightning_invalid_timestamp",
)


def recorded_snapshot(seed, *, case=None):
    case = case or RECORDED_SCENARIOS[seed % len(RECORDED_SCENARIOS)]
    rng = random.Random(20261028 + seed)
    book = samourai_snapshot(seed, case="tx0")
    rows = book["transactions"]
    source = next(row for row in rows if row["direction"] == "outbound")
    target = next(row for row in rows if row["direction"] == "inbound")
    amount = rng.randint(10**4, 10**10) * 1000
    if case == "exact_msat": amount = (1 << 53) + 47
    source.update(amount=amount, fee=3000, amount_includes_fee=0)
    target.update(amount=amount, fee=0)
    for row in rows:
        row.update(config_json='{"chain":"bitcoin","network":"main"}', config_text_mentions_samourai=False,
                   observation_fee_attribution="exact", privacy_boundary=None, journal_transaction_id=None)
        raw = json.loads(row["raw_json"])
        raw.pop("vin", None)
        raw.pop("vout", None)
        row["raw_json"] = json.dumps(raw)
    if case == "bitcoin_upper_txid": target["external_id"] = source["external_id"].upper()
    if case == "bitcoin_typed_external":
        for row in rows: row.update(raw_json="{}", external_id_kind="transaction_hash", observation_authority_version=None)
    if case == "bitcoin_timestamp_skew": source["occurred_at"] = "2026-03-10T00:00:00Z"
    if case == "bitcoin_fee_inclusive": source.update(amount=amount + 3000, amount_includes_fee=1)
    if case in {"bitcoin_partial", "bitcoin_implicit_residual"}:
        target["amount"] = amount // 2
        if case.endswith("residual"): source["observation_fee_attribution"] = "implicit_wallet_delta"
    if case in {"untrusted_source", "untrusted_both"}: source["observation_authority_version"] = None
    if case in {"untrusted_target", "untrusted_both"}: target["observation_authority_version"] = None
    if case == "same_wallet": target["wallet_id"] = source["wallet_id"]
    if case == "zero_inbound": target["amount"] = 0
    if case == "negative_outbound": source["amount"] = -1
    if case == "multiple_sources": rows.append({**source, "id": "another-source", "wallet_id": "wallet-c"})
    if case == "multiple_targets": rows.append({**target, "id": "another-target"})
    if case == "different_txid":
        target["external_id"] = "f" * 64
        target["raw_json"] = json.dumps({"chain": "bitcoin", "network": "main", "txid": "f" * 64})
    if case == "network_separation":
        target["config_json"] = '{"chain":"bitcoin","network":"regtest"}'
        raw = json.loads(target["raw_json"]); raw["network"] = "regtest"; target["raw_json"] = json.dumps(raw)
    if case.startswith("liquid_"):
        network = {"liquid_test": "liquidtestnet", "liquid_regtest": "elementsregtest"}.get(case, "liquidv1")
        for row in rows:
            row.update(asset="LBTC", config_json=json.dumps({"chain": "liquid", "network": network}), wallet_kind="liquid")
            raw = {"chain": "liquid", "network": network, "txid": source["external_id"], "component": {"asset_id": "a" * 64, "asset": "LBTC"}}
            if case == "liquid_missing_identity": raw.pop("component")
            if case == "liquid_asset_separation" and row is target: raw["component"]["asset_id"] = "b" * 64
            if case == "liquid_asset_id": row["asset"] = "a" * 64; raw["component"]["asset"] = "a" * 64
            row["raw_json"] = json.dumps(raw)
    if case.startswith("lightning_"):
        for row in rows:
            receive = row["direction"] == "inbound"
            cln = case == "lightning_cln" or receive
            row.update(wallet_kind="coreln" if cln else "lnd", payment_hash="b" * 64,
                       payment_hash_source="core_lightning" if cln else "lnd",
                       kind=("cln_invoice" if receive else "cln_pay") if cln else ("lnd_invoice" if receive else "lnd_pay"),
                       external_id="node:receive" if receive else "node:pay", external_id_kind=None,
                       raw_json=json.dumps({"_kassiber_provenance": {"import_source": "core-lightning" if cln else "lnd"}}),
                       config_json='{"network":"main"}', observation_authority_version=None)
        if case == "lightning_circular": target["wallet_id"] = source["wallet_id"]
        if case == "lightning_hash_case": target["payment_hash"] = "\u2003" + "B" * 64 + "\x1c"
        if case == "lightning_chronology": source["occurred_at"] = "2026-10-28T00:00:00Z"
        if case == "lightning_skew_boundary":
            source["occurred_at"] = "2026-01-03T00:00:00.000001+00:00"
            target["occurred_at"] = "2026-01-01T00:00:00.000001Z"
        if case == "lightning_invalid_timestamp": target["occurred_at"] = "not-a-date"
        if case == "lightning_shortfall": target["amount"] -= 1
        if case == "lightning_forged_source": target["raw_json"] = "{}"
        if case == "lightning_forged_lnd": source["raw_json"] = "{}"
        if case == "lightning_bad_hash": target["payment_hash"] = "x" * 64
        if case == "lightning_wrong_kind": target["kind"] = "chain_receipt"
        if case == "lightning_chain_hash": target["payment_hash_source"] = "chain_script_unique_outpoint"
        if case == "lightning_duplicate": rows.append({**target, "id": "duplicate-node-receipt"})
        if case == "lightning_no_network":
            for row in rows: row["config_json"] = "{}"
        if case == "lightning_asset_case": target["asset"] = "btc"
    if case == "synthetic_id": source["id"] = "owned-derive:source"
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
    if case == "nonpositive_placeholder": rows.append({**target, "id": "placeholder", "amount": 0})
    if case == "nonfinite_graph_identity":
        for row in rows: row["raw_json"] = '{"chain":"bitcoin","network":"main","txid":1e400}'
    for row in rows:
        row.update(observation_graph_hash=canonical_graph_hash(row["raw_json"]), observation_quantity_hash=canonical_observed_quantity_hash(row))
    if case == "tampered_quantity": target["observation_quantity_hash"] = "0" * 64
    book["transactions"].sort(key=lambda r: (r["occurred_at"], r["created_at"], r["id"]))
    book["gap_worklist"]["rows"] = deepcopy(rows)
    return book
