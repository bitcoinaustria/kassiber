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
