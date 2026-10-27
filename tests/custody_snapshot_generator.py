"""Seeded synthetic raw custody books. No Python verdicts become Rust inputs."""
from __future__ import annotations
from copy import deepcopy
from functools import lru_cache
import hashlib
import json
import random
import tempfile

from kassiber.core.custody_snapshot import load_custody_snapshot, encode_owned_index
from kassiber.core.chain_observer.provenance import canonical_graph_hash, canonical_observed_quantity_hash
from kassiber.core.ownership import OwnedIndex, OwnedMatch
from kassiber.db import open_db
from tests.custody_engine_compare import permuted_snapshot as permuted

SEED = 20261027
AS_OF = "2026-10-27T12:00:00Z"
SCENARIOS = ("ordinary", "transfer", "fanout", "shared_fee", "contradiction", "samourai", "channel", "loan", "liquid", "excluded_anchor", "missing_anchor", "gap", "receipt_overlap", "zero_delta", "duplicate_fee", "scope_conflict", "exact_integer", "surrogate")


@lru_cache(maxsize=1)
def templates():
    from tests import test_rp2_ownership_transfers as fixtures
    with tempfile.TemporaryDirectory(prefix="kassiber-custody-generator-") as scratch:
        conn = open_db(scratch)
        try:
            helper = fixtures.OwnershipDeriverHandlerTest()
            helper._seed(conn)
            helper._tx(conn, tx_id="template", wallet_id="wallet-a", direction="inbound", amount=100000, external_id="template", raw_json="{}")
            profile = conn.execute("SELECT * FROM profiles WHERE id='profile-1'").fetchone()
            base = load_custody_snapshot(conn, profile, as_of=AS_OF)
        finally:
            conn.close()
    from tests import test_custody_components as components
    book = components.CustodyComponentApiTests()
    book.setUp()
    try:
        components.activate_component(book.conn, book._balanced_component()["id"])
        # Authored ids are normally UUID4; fix every reference before snapshot
        # generation without rewriting the stored economics or oracle outputs.
        profile = book.conn.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()
        component = load_custody_snapshot(book.conn, profile, as_of=AS_OF)
        mapping = {}
        for item in component["components"]:
            for record in (item["header"], *item["legs"], *item["allocations"], *item["economic_terms"], *item["commitments"]):
                value = record["id"]
                mapping.setdefault(value, f"generated-authored-{len(mapping)}")
            lineage = item["header"]["lineage_id"]
            mapping.setdefault(lineage, f"generated-authored-{len(mapping)}")
        def remap(value):
            if isinstance(value, str):
                return mapping.get(value, value)
            if isinstance(value, list):
                return [remap(item) for item in value]
            if isinstance(value, dict):
                return {mapping.get(key, key): remap(item) for key, item in value.items()}
            return value
        component = remap(component)
        # Persisted timestamps are facts; synthetic template instants are fixed.
        def fix_times(value):
            if isinstance(value, list):
                for item in value:
                    fix_times(item)
            elif isinstance(value, dict):
                for key in ("activated_at", "created_at", "updated_at"):
                    if value.get(key):
                        value[key] = "2026-01-01T00:00:00Z"
                for item in value.values():
                    fix_times(item)
        fix_times(component)
    finally:
        book.tearDown()
    return base, component


def snapshot(seed: int):
    rng = random.Random(SEED + seed)
    scenario = SCENARIOS[seed % len(SCENARIOS)]
    base, component = templates()
    if scenario in {"excluded_anchor", "missing_anchor"}:
        book = deepcopy(component)
        book["book_network"] = None
        book["transactions"] = [row for row in book["transactions"] if row["id"] != "out"]
        if scenario == "excluded_anchor":
            book["anchors"]["out"]["excluded"] = 1
        else:
            book["anchors"]["out"] = None
            for item in book["components"]:
                for leg in item["legs"]:
                    if leg["transaction_id"] == "out":
                        leg["transaction_id"] = None
        book["profile"]["journal_input_version"] = seed
    else:
        book = deepcopy(base)
        template = book["transactions"][0]
        book.update(book_network=None, owned_index=None, transactions=[], components=[], anchors={}, anchor_wallets=[], revision_headers=[], receipt_sources=[], channel_records=[], loan_legs=[], evidence_snapshots=[], migration_issues=[], dismissed_gap_fingerprints={}, pair_dismissals=[], ownership_warnings=[])
        wallet_c = {**book["wallets"][1], "id": "wallet-c", "label": "Savings"}
        book["wallets"].append(wallet_c)
        wallets = {wallet["id"]: wallet for wallet in book["wallets"]}
        amount = rng.randint(1, 10**10) * 1000
        fee = rng.randint(0, 10000) * 1000
        event = hashlib.sha256(f"custody-generator:{seed}".encode()).hexdigest()
        previous = hashlib.sha256(f"custody-generator:prev:{seed}".encode()).hexdigest()
        scripts = {wallet: "0014" + hashlib.sha256(wallet.encode()).hexdigest()[:40] for wallet in wallets}
        def row(identifier, wallet, direction, value, *, miner_fee=0, asset="BTC", kind=None, graph=None, authority=True, includes=False):
            ref = wallets[wallet]
            item = {**template, "id": identifier, "fingerprint": f"generated-{seed}-{identifier}", "external_id": graph.get("txid", event) if graph is not None else identifier, "external_id_kind": "txid" if graph is not None else None, "wallet_id": wallet, "wallet_label": ref["label"], "wallet_kind": ref["kind"], "config_json": ref["config_json"], "direction": direction, "amount": value, "fee": miner_fee, "amount_includes_fee": int(includes), "asset": asset, "kind": kind, "occurred_at": f"2026-01-{seed % 28 + 1:02d}T00:00:00Z", "created_at": "2026-01-01T00:00:00Z", "raw_json": json.dumps(graph if graph is not None else {"generated": seed, "float": rng.choice([0.1, -0.0, 1e-25, 1e20]), "unicode": "é 钱包"}), "observation_authority_version": None, "observation_graph_hash": None, "observation_quantity_hash": None, "observation_application_revision": None, "observation_fee_attribution": None}
            if graph is not None and authority:
                item.update(observation_authority_version=1, observation_graph_hash=canonical_graph_hash(item["raw_json"]), observation_quantity_hash=canonical_observed_quantity_hash(item), observation_application_revision="generated", observation_fee_attribution="exact")
            book["transactions"].append(item)
            return item
        def graph(inputs, outputs):
            return {"chain": "bitcoin", "network": "main", "txid": event, "vin": [{"txid": previous, "vout": i, "prevout": {"scriptpubkey": scripts.get(w, w), "value_sats": value // 1000}} for i, (w, value) in enumerate(inputs)], "vout": [{"n": i, "scriptpubkey": scripts.get(w, w), "value_sats": value // 1000} for i, (w, value) in enumerate(outputs)]}
        index = OwnedIndex()
        def own(wallet, *, chain="bitcoin", network="main"):
            match = OwnedMatch(wallet, wallets[wallet]["label"], "treasury", chain, network, "", None, None, "inventory", wallets[wallet]["kind"])
            index.add_script(scripts[wallet], match)
            return match
        if scenario == "surrogate":
            item = row("surrogate", "wallet-a", "inbound", amount)
            item["raw_json"] = r'{"value":"\ud800"}'
        elif scenario == "exact_integer":
            amount = (1 << 53) + rng.randint(1, 1000000)
            row("large-buy", "wallet-a", "inbound", amount, kind="buy")
            row("large-sell", "wallet-a", "outbound", amount - 1, miner_fee=1, kind="sell")
        elif scenario == "ordinary":
            row("buy", "wallet-a", "inbound", amount * 2, kind="buy")
            row("sell", "wallet-a", "outbound", amount, miner_fee=fee, kind="sell", includes=bool(seed % 2))
        elif scenario in {"transfer", "fanout", "samourai", "duplicate_fee"}:
            for wallet in wallets:
                wallets[wallet].update(kind="descriptor", config_json=json.dumps({"chain": "bitcoin", "network": "main"}))
                own(wallet)
            if scenario == "samourai":
                for i, wallet in enumerate(wallets.values()):
                    wallet["config_json"] = json.dumps({"chain": "bitcoin", "network": "main", "samourai": {"role": "child", "group_id": "generated", "section": ["deposit", "premix", "postmix"][i]}})
                    wallet["config_text_mentions_samourai"] = True
            if scenario == "duplicate_fee":
                g = graph([("wallet-a", amount + fee), ("wallet-c", amount + fee)], [("wallet-b", amount * 2 + fee)])
                row("out-a", "wallet-a", "outbound", amount, miner_fee=fee, graph=g)
                row("out-c", "wallet-c", "outbound", amount, miner_fee=fee, graph=g)
                row("in", "wallet-b", "inbound", amount * 2 + fee, graph=g)
            else:
                destinations = [("wallet-b", amount)] if scenario != "fanout" else [("wallet-b", amount // 2000 * 1000), ("wallet-c", amount - amount // 2000 * 1000)]
                g = graph([("wallet-a", amount + fee)], destinations)
                row("out", "wallet-a", "outbound", amount, miner_fee=fee, graph=g)
                for i, (wallet, value) in enumerate(destinations):
                    if value:
                        row(f"in-{i}", wallet, "inbound", value, graph=g)
        elif scenario == "shared_fee":
            a = amount + fee + 1000
            g = graph([("wallet-a", a), ("wallet-b", a)], [("external-script", a * 2 - fee)])
            g["fee"] = fee // 1000
            for wallet in ("wallet-a", "wallet-b"):
                own(wallet)
                observation = {**g, "observer_owned_scripts": [scripts[wallet]]}
                item = row(wallet, wallet, "outbound", a, graph=observation)
                item["observation_fee_attribution"] = "implicit_wallet_delta"
        elif scenario == "receipt_overlap":
            g = graph([("foreign-input", amount * 2 + fee)], [("wallet-a", amount), ("wallet-b", amount)])
            for wallet, kind, source in (("wallet-a", "descriptor", "descriptor_config"), ("wallet-b", "address", "address_list")):
                wallets[wallet]["kind"] = kind
                own(wallet)
                for script in (scripts["wallet-a"], scripts["wallet-b"]):
                    book["receipt_sources"].append({"profile_id": book["profile"]["id"], "wallet_id": wallet, "wallet_label": wallets[wallet]["label"], "wallet_kind": kind, "chain": "bitcoin", "network": "main", "script_pubkey": script, "source": source, "deprecated": False, "active_transaction_count": 1})
                row(wallet, wallet, "inbound", amount * 2, graph=g)
            # Both overlapping wallets need a positive disjoint entitlement.
            book["receipt_sources"] = [s for s in book["receipt_sources"] if not (s["wallet_id"] == "wallet-a" and s["script_pubkey"] == scripts["wallet-b"])]
        elif scenario == "zero_delta":
            own("wallet-a")
            g = graph([("wallet-a", amount)], [("wallet-a", amount)])
            item = row("zero", "wallet-a", "outbound", 0, graph=g)
            if (seed // len(SCENARIOS)) % 3 == 1:
                g.pop("chain")
                item["wallet_kind"] = "lnd"
                item["raw_json"] = json.dumps(g)
                item["observation_graph_hash"] = canonical_graph_hash(item["raw_json"])
            elif (seed // len(SCENARIOS)) % 3 == 2:
                item["asset"] = "USDT"
                g["component"] = {"asset": "BTC"}
                item["raw_json"] = json.dumps(g)
                item["observation_graph_hash"] = canonical_graph_hash(item["raw_json"])
                item["observation_quantity_hash"] = canonical_observed_quantity_hash(item)
        elif scenario == "channel":
            wallets["wallet-b"].update(kind="coreln", config_json=json.dumps({"chain": "bitcoin", "network": "main"}))
            own("wallet-a")
            g = graph([("wallet-a", amount + fee)], [("channel-script", amount)])
            row("funding", "wallet-a", "outbound", amount, miner_fee=fee, graph=g)
            close_id = hashlib.sha256(f"custody-generator:close:{seed}".encode()).hexdigest()
            close_fee = min(fee, amount // 2)
            closing = graph([("channel-script", amount)], [("wallet-a", amount - close_fee)])
            closing["txid"] = close_id
            closing["vin"][0]["txid"] = event
            closed = row("closing", "wallet-a", "inbound", amount - close_fee, graph=closing)
            closed["occurred_at"] = closed["occurred_at"].replace("T00:", "T01:")
            book["channel_records"] = [{"txid": event if tag == "channel_open" else close_id, "outpoint": event + ":0", "tag": tag, "wallet_id": "wallet-b", "channel_id": "generated-channel", "amount_msat": amount, "raw_json": json.dumps({"local_funding_msat": amount, "chain": "bitcoin", "network": "main"}), "config_json": wallets["wallet-b"]["config_json"], "chain": "bitcoin", "network": "main"} for tag in ("channel_open", "channel_close")]
        elif scenario == "loan":
            row("loan", "wallet-a", "inbound", amount)
            row("repay", "wallet-a", "outbound", amount, miner_fee=fee)
            book["loan_legs"] = [{"transaction_id": "loan", "role": "loan_principal_received"}, {"transaction_id": "repay", "role": "loan_principal_repaid"}]
        elif scenario == "liquid":
            for wallet in wallets.values():
                wallet.update(kind="liquid-electrum", config_json=json.dumps({"chain": "liquid", "network": "elementsregtest"}))
            g = {"chain": "liquid", "network": "elementsregtest", "txid": event, "component": {"asset_id": "11" * 32, "asset": "LBTC"}}
            row("liquid-out", "wallet-a", "outbound", amount, miner_fee=fee, asset="LBTC", graph=g)
            row("liquid-in", "wallet-b", "inbound", amount, asset="LBTC", graph=g)
        elif scenario == "contradiction":
            g = graph([("wallet-a", amount + fee)], [("wallet-b", amount)])
            row("copy-a", "wallet-b", "inbound", amount, graph=g, authority=False)
            row("copy-b", "wallet-b", "inbound", amount + 1000, graph=g, authority=False)
        elif scenario == "scope_conflict":
            item = row("unsupported", "wallet-a", "inbound", amount)
            item["config_json"] = json.dumps({"chain": "future-chain", "network": "unknown"})
        else:
            item = row("gap-out", "wallet-a", "outbound", amount * 10, miner_fee=fee)
            item["privacy_boundary"] = "whirlpool"
            returned = row("gap-return", "wallet-b", "inbound", amount * 99 // 10)
            returned["occurred_at"] = returned["occurred_at"].replace("T00:", "T01:")
        book["owned_index"] = encode_owned_index(index, book["transactions"]) if any(row["observation_authority_version"] is not None for row in book["transactions"]) else None
    book["as_of"] = AS_OF
    book["transactions"].sort(key=lambda row: (row["occurred_at"], row["created_at"], row["id"]))
    book["rate_rows"] = [{key: row.get(key) for key in ("asset", "fiat_rate", "fiat_value", "fiat_rate_exact", "fiat_value_exact", "amount")} for row in reversed(book["transactions"])]
    book["gap_worklist"] = {"large_book": False, "rows": [{**row, "wallet_config_json": row["config_json"]} for row in book["transactions"]]}
    return book
