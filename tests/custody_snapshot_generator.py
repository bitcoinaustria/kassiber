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
SCENARIOS = ("ordinary", "transfer", "fanout", "shared_fee", "contradiction", "samourai", "channel", "loan", "liquid", "excluded_anchor", "missing_anchor", "gap", "receipt_overlap", "zero_delta", "duplicate_fee", "scope_conflict", "exact_integer", "surrogate", "receipt_success", "receipt_deprecated", "receipt_priority", "receipt_label", "receipt_id", "duplicate_slot", "tampered_graph", "tampered_quantity", "historical_overcount", "nonaddress_overcount", "duplicate_fee_keeper")


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
        book.update(book_network=None, owned_index=None, transactions=[], components=[], anchors={}, anchor_wallets=[], revision_headers=[], receipt_sources=[], channel_records=[], loan_legs=[], evidence_snapshots=[], migration_issues=[], gap_reviews=[], pair_dismissals=[], ownership_warnings=[])
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
        if scenario.startswith("receipt_") and scenario != "receipt_overlap":
            shared = "0014" + "ef" * 20
            g = graph([("foreign-input", amount * 3 + fee)], [("wallet-a", amount), ("wallet-b", amount), (shared, amount)])
            for wallet in ("wallet-a", "wallet-b"):
                wallets[wallet]["kind"] = "descriptor"
                if scenario in {"receipt_label", "receipt_id"}:
                    wallets[wallet]["label"] = "Same" if scenario == "receipt_id" else ("Z" if wallet == "wallet-a" else "A")
                own(wallet)
                # A shared match list also exercises actual ownership lookup order.
                index.add_script(shared, own(wallet))
                for script in (scripts[wallet], shared):
                    book["receipt_sources"].append({"profile_id": book["profile"]["id"], "wallet_id": wallet, "wallet_label": wallets[wallet]["label"], "wallet_kind": "descriptor", "chain": "bitcoin", "network": "main", "script_pubkey": script, "source": "sync_target" if scenario == "receipt_priority" and wallet == "wallet-a" else "descriptor_config", "deprecated": scenario == "receipt_deprecated" and wallet == "wallet-a", "active_transaction_count": 1})
                row(wallet, wallet, "inbound", amount * 2, graph=g)
        elif scenario in {"historical_overcount", "nonaddress_overcount"}:
            g = graph([("foreign-input", amount * 2 + fee)], [("wallet-a", amount), ("wallet-b", amount)])
            for wallet in ("wallet-a", "wallet-b"):
                wallets[wallet]["kind"] = "address" if scenario == "historical_overcount" and wallet == "wallet-a" else "descriptor"
                own(wallet)
                book["receipt_sources"].append({"profile_id": book["profile"]["id"], "wallet_id": wallet, "wallet_label": wallets[wallet]["label"], "wallet_kind": wallets[wallet]["kind"], "chain": "bitcoin", "network": "main", "script_pubkey": scripts[wallet], "source": "inventory", "deprecated": False, "active_transaction_count": 1})
                row(wallet, wallet, "inbound", amount * 2, graph=g)
        elif scenario in {"tampered_graph", "tampered_quantity"}:
            own("wallet-a")
            g = graph([("foreign-input", amount + fee)], [("wallet-a", amount)])
            item = row("tampered", "wallet-a", "inbound", amount, graph=g)
            if scenario == "tampered_graph":
                g["vout"][0]["value_sats"] += 1
                item["raw_json"] = json.dumps(g)
            else:
                item["amount"] += 1000
        elif scenario == "duplicate_slot":
            g = graph([("foreign-input", amount + fee)], [("wallet-a", amount)])
            row("alias-a", "wallet-a", "inbound", amount, graph=g)
            other = row("alias-z", "wallet-a", "inbound", amount, graph=g)
            other["occurred_at"] = other["occurred_at"].replace("T00:", "T01:")
        elif scenario == "duplicate_fee_keeper":
            for identifier, wallet in [("fee-a", "wallet-a"), ("fee-z", "wallet-b")]:
                row(identifier, wallet, "outbound", amount, miner_fee=max(fee, 1000), graph={"txid": event}, authority=False)
        elif scenario == "surrogate":
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

# Independent component corpus: retain the 2,000 evidence books and their
# branch distribution while exercising the next stage with 2,000 more seeds.
COMPONENT_SCENARIOS = (
    "balanced", "no_legs", "missing_source", "missing_destination", "unknown_role",
    "unbalanced", "unit_mismatch", "missing_time", "invalid_time", "chronology",
    "location_continuity", "allocation_required", "multi_allocation", "allocation_invalid",
    "allocation_quantity", "allocation_source_coverage", "allocation_sink_coverage",
    "fee_asset", "fee_wallet", "fee_scope", "fee_orphan",
    "scope_invalid_chain", "scope_rail_chain", "scope_invalid_network", "scope_mismatch",
    "scope_laundering", "cross_scope_laundering", "cross_location_continuity",
    "conversion", "conversion_topology", "conversion_retained", "conversion_review",
    "conversion_policy", "conversion_missing_value", "conversion_no_values",
    "conversion_unbalanced", "conversion_fee_quantity", "conversion_fee_value", "value_only_loss",
    "suspense", "suspense_mode", "suspense_review", "suspense_no_allocations",
    "suspense_anchor", "suspense_missing_time", "suspense_source", "suspense_asset",
    "suspense_quantity", "suspense_network", "suspense_time", "suspense_fee",
    "content_missing", "leg_count", "allocation_count", "term_count",
    "commitment_header", "commitment_count", "commitment_ordinals", "commitment_zero",
    "commitment_no_anchors", "commitment_hash", "commitment_volatile", "commitment_rejected",
    "anchor_missing", "anchor_retracted", "anchor_identity", "anchor_excluded",
    "anchor_source_direction", "anchor_destination_direction", "anchor_loss_direction",
    "anchor_asset", "anchor_wallet", "anchor_time_invalid", "anchor_time_mismatch",
    "anchor_time_warning", "anchor_coverage", "anchor_partial", "anchor_partial_zero",
    "anchor_partial_excess", "anchor_rail", "anchor_chain", "anchor_network",
    "owned_wallet_missing", "retained_location", "transaction_anchor_missing",
    "lineage_conflict", "revision_missing", "revision_scope", "revision_lineage",
    "revision_order", "revision_forward", "revision_valid", "membership_conflict",
    "membership_empty_anchor", "component_order", "children_order", "native_supported",
    "native_partial", "native_contradicted", "native_exact_sink", "native_other_asset",
    "hash_kind", "hash_payment", "hash_payment_source", "hash_refund_txid", "hash_refund_vout",
    "hash_wallet", "hash_direction", "hash_asset", "hash_amount", "hash_fee", "hash_included",
    "hash_event", "integer_semantics", "time_semantics", "scope_enriched_mismatch", "lineage_ties",
    "location_ties", "transactionless_wallet_missing", "anchor_row_time_invalid", "revision_backward",
    "chronology_after_boundary",
    "asset_l_btc_alias", "lightning_chain_default", "lightning_kind_scope", "core_ln_kind_scope", "liquid_kind_conflict",
    "inactive_header",
    "peg_in",
    "peg_out",
    "lightning_receive_lnd",
    "lightning_receive_nwc",
    "lightning_receive_cln",
    "lightning_receive_core_ln",
    "swap_terms",
    "untracked_route",
    "liquid_wallet_omitted_scope",
    "wallet_scope_conflict",
    "conversion_inferred",
    "conversion_inferred_chronology",
    "conversion_ambiguous",
    "native_namespace_gate",
    "native_excluded_rows",
    "lineage_other_profile",
    "authored_source_default",
    "retained_reviewed_notes",
    "liquid_substring_kind",
    "generic_elements_domain",
    "fee_null_domain",
    "missing_time_empty",
    "transaction_id_empty",
    "zero_amount_chronology",
    "authoritative_chronology",
    "asset_upper_semantics", "asset_hash_upper_019b", "asset_hash_upper_0264",
)


# Every scenario is an activation assertion, including negative controls.
EXPECTED_ACTIVE_COMPONENTS = {'asset_l_btc_alias': 1, 'balanced': 1, 'multi_allocation': 1, 'conversion': 1, 'suspense': 1, 'revision_valid': 1, 'children_order': 1, 'native_supported': 1, 'native_exact_sink': 1, 'hash_kind': 1, 'hash_payment': 1, 'hash_payment_source': 1, 'hash_refund_txid': 1, 'hash_refund_vout': 1, 'hash_included': 1, 'hash_event': 1, 'integer_semantics': 1, 'time_semantics': 1, 'peg_in': 1, 'peg_out': 1, 'lightning_receive_lnd': 1, 'lightning_receive_nwc': 1, 'lightning_receive_cln': 1, 'lightning_receive_core_ln': 1, 'swap_terms': 1, 'untracked_route': 2, 'liquid_wallet_omitted_scope': 1, 'conversion_inferred': 1, 'native_excluded_rows': 1, 'lineage_other_profile': 1, 'authored_source_default': 1, 'retained_reviewed_notes': 1, 'liquid_substring_kind': 1, 'fee_null_domain': 1, 'authoritative_chronology': 1, 'asset_hash_upper_019b': 1, 'asset_hash_upper_0264': 1}


def component_snapshot(seed: int, *, case: str | None = None):
    """Authored raw rows, including interrupted imports and restored conflicts."""
    from kassiber.core.custody_component_facts import ComponentFacts
    from kassiber.core.custody_quantity_store import _canonical_component_snapshots, _commitment_detail_hash
    case = COMPONENT_SCENARIOS[seed % len(COMPONENT_SCENARIOS)] if case is None else case
    book = deepcopy(templates()[1])
    book["book_network"] = None
    book["profile"]["journal_input_version"] = seed
    # Anchor reads and the active population contain the same stored facts.
    rows = {r["id"]: r for r in book["transactions"]}
    book["anchors"] = {key: rows.get(key, value) for key, value in book["anchors"].items()}
    c = book["components"][0]
    h = c["header"]
    source, dest, other_dest, fee = c["legs"]

    def new_leg(role, amount, *, wallet="btc", when="2026-01-01T00:00:00Z", **kw):
        leg = {**source, "id": f"{h['id']}:leg:{len(c['legs'])}", "ordinal": len(c["legs"]), "role": role, "amount_msat": amount, "transaction_id": None, "anchor_transaction_id": None, "wallet_id": wallet, "occurred_at": when, **kw}
        c["legs"].append(leg)
        return leg

    def explicit():
        c["allocations"] = [
            {key: source[key] for key in ("component_id", "workspace_id", "profile_id", "created_at")}
            | {"id": f"{h['id']}:allocation:{i}", "ordinal": i, "source_leg_id": source["id"], "sink_leg_id": sink["id"], "source_amount_msat": sink["amount_msat"], "sink_amount_msat": sink["amount_msat"]}
            for i, sink in enumerate(c["legs"][1:])
        ]

    def seal(item=None):
        item = c if item is None else item
        header = item["header"]
        # These are author-written commitments, not validation outputs. Like
        # activation, they bind canonical quantity and decision-material facts.
        snapshots = _canonical_component_snapshots(ComponentFacts(book), {**header, "legs": item["legs"]})
        committed = sorted((s.quantity_hash, _commitment_detail_hash(s)) for s in snapshots)
        item["commitments"] = [{"id": f"{header['id']}:commitment:{i}", "component_id": h["id"], "workspace_id": h["workspace_id"], "profile_id": h["profile_id"], "ordinal": i, "quantity_hash": q, "detail_hash": d, "created_at": h["created_at"]} for i, (q, d) in enumerate(committed)]
        header["expected_evidence_count"] = len(committed)

    def clone_component(identifier):
        original = h["id"]
        result = deepcopy(c)
        def remap(v):
            if isinstance(v, str):
                return identifier + v[len(original):] if v == original or v.startswith(original + ":") else v
            if isinstance(v, dict):
                return {k: remap(x) for k, x in v.items()}
            if isinstance(v, list):
                return [remap(x) for x in v]
            return v
        result = remap(result)
        # Historical UUID leg ids do not share the component prefix.
        ids = {leg["id"]: f"{identifier}:child:{i}" for i, leg in enumerate(result["legs"])}
        for leg in result["legs"]:
            leg["id"] = ids[leg["id"]]
        for a in result["allocations"]:
            for key in ("source_leg_id", "sink_leg_id"):
                a[key] = ids[a[key]]
        book["components"].append(result)
        return result

    if seed:
        scale = random.Random(SEED + seed).randint(1, 10**8)
        for row in rows.values():
            for key in ("amount", "fee"):
                row[key] *= scale
        for leg in c["legs"]:
            leg["amount_msat"] *= scale
        seal()

    if case.startswith("suspense"):
        h["evidence_grade"] = "reviewed"
        other_dest["amount_msat"] -= 1
        rows["in-2"]["amount"] = other_dest["amount_msat"]
        rows["out"].update(fee=fee["amount_msat"], amount_includes_fee=1)
        residual = new_leg("suspense", 1, wallet=None)
        explicit()
        seal()
        if case == "suspense_mode": h["conservation_mode"] = "conversion"
        if case == "suspense_review": h["evidence_grade"] = "exact"
        if case == "suspense_no_allocations": c["allocations"] = []
        if case == "suspense_anchor": residual.update(wallet_id="btc", transaction_id="in-2", anchor_transaction_id="in-2")
        if case == "suspense_missing_time": residual["occurred_at"] = None
        if case == "suspense_source": source.update(transaction_id=None, anchor_transaction_id=None)
        if case == "suspense_asset": residual["asset"] = "LBTC"
        if case == "suspense_quantity": c["allocations"][-1]["sink_amount_msat"] = 2
        if case == "suspense_network": residual["network"] = "main"
        if case == "suspense_time": residual["occurred_at"] = "2026-01-02T00:00:00Z"
        if case == "suspense_fee": rows["out"]["fee"] = 2
    elif case.startswith("conversion") or case == "value_only_loss":
        h.update(conservation_mode="conversion", conversion_policy="reviewed", conversion_reviewed=1)
        c["legs"].remove(other_dest)
        source["amount_msat"] = 100
        dest["amount_msat"] = rows["in-1"]["amount"] = 90
        fee["amount_msat"] = 10
        for leg in c["legs"]:
            leg.update(valuation_unit="EUR-cent", valuation_amount=leg["amount_msat"] * 2)
        rows["out"].update(amount=100, fee=10, amount_includes_fee=1)
        explicit()
        seal()
        if case == "conversion_topology": new_leg("destination", 1, valuation_unit="EUR-cent", valuation_amount=0)
        if case == "conversion_retained":
            retained = new_leg("retained", 1, valuation_unit="EUR-cent", valuation_amount=0)
            explicit()
            c["allocations"][-1]["source_amount_msat"] = 2
        if case == "conversion_review": h["conversion_reviewed"] = 0
        if case == "conversion_policy": h["conversion_policy"] = "\x1c\u0085"
        if case == "conversion_missing_value": dest["valuation_amount"] = None
        if case == "conversion_no_values":
            for leg in c["legs"]: leg.update(valuation_unit=None, valuation_amount=None)
        if case == "conversion_unbalanced": dest["valuation_amount"] += 1
        if case == "conversion_fee_quantity": c["allocations"][-1]["sink_amount_msat"] = 11
        if case == "conversion_fee_value": fee["valuation_amount"] += 1
        if case == "value_only_loss": fee["amount_msat"] = 0
    elif case in {"scope_laundering", "cross_scope_laundering", "cross_location_continuity"}:
        placeholder = {**book["anchor_wallets"][0], "id": "untracked", "kind": "untracked", "config_json": "{}"}
        book["anchor_wallets"].append(placeholder)
        dest.update(wallet_id="untracked", transaction_id=None, anchor_transaction_id=None, rail="untracked", chain=None, network=None)
        other_dest["amount_msat"] = 0
        dest["amount_msat"] = 99
        intermediate = new_leg("source", 99, wallet="untracked", rail="untracked", chain=None, network=None, when="2026-01-04T00:00:00Z")
        output = new_leg("destination", 99, wallet="btc", network="main", when="2026-01-04T00:00:00Z")
        if case != "scope_laundering":
            other = clone_component("route-b")
            c["legs"] = c["legs"][:-2]
            other["legs"] = other["legs"][-2:]
            other["header"]["lineage_id"] = "route-b"
            if case == "cross_location_continuity":
                other["legs"][0]["amount_msat"] = 100
                other["legs"][1].update(amount_msat=100, network="regtest")
        explicit()
    elif case in {"location_continuity", "location_ties"}:
        new_leg("source", 101, when="2026-01-04T00:00:00Z")
        if case == "location_ties":
            # Canonical event ties receive before spending; tied spends retain
            # ordinal then input order, which decides the named failing source.
            c["legs"][-1]["occurred_at"] = "2025-12-30T00:00:00Z"
            new_leg("source", 1, when="2025-12-30T00:00:00Z")
        new_leg("retained", 101, wallet="elsewhere", when="2026-01-04T00:00:00Z")
    elif case in {"allocation_required", "multi_allocation", "fee_orphan"}:
        second = new_leg("source", 60, transaction_id="other-out", anchor_transaction_id="other-out")
        dest["amount_msat"] += 60
        if case != "allocation_required":
            c["legs"].remove(second)
            explicit()
            c["legs"].append(second)
            if case == "multi_allocation":
                rows["other-out"].update(amount=60, fee=0, amount_includes_fee=0)
                book["anchors"]["other-out"] = rows["other-out"]
                rows["in-1"]["amount"] = dest["amount_msat"]
                c["allocations"][0]["source_amount_msat"] = c["allocations"][0]["sink_amount_msat"] = dest["amount_msat"] - 60
                c["allocations"].append({**c["allocations"][0], "id": "second-source", "ordinal": 4, "source_leg_id": second["id"], "source_amount_msat": 60, "sink_amount_msat": 60})
                seal()
            else:
                c["allocations"][-1]["source_leg_id"] = second["id"]
    elif case.startswith("allocation_") or case in {"fee_asset", "fee_wallet", "fee_scope", "children_order"}:
        explicit()
        if case == "allocation_count": h["expected_allocation_count"] = 4
        if case == "allocation_invalid": c["allocations"][0]["source_leg_id"] = dest["id"]
        if case == "allocation_quantity": c["allocations"][0]["sink_amount_msat"] += 1
        if case == "allocation_source_coverage": c["allocations"][0]["source_amount_msat"] -= 1
        if case == "allocation_sink_coverage": c["allocations"][0]["sink_leg_id"] = other_dest["id"]
        if case == "fee_asset": fee["asset"] = "ETH"
        if case == "fee_wallet": fee["wallet_id"] = "other"
        if case == "fee_scope": fee["rail"] = "lightning"
        if case == "children_order":
            for row in c["legs"] + c["allocations"]: row["ordinal"] = 0
            c["legs"].sort(key=lambda row: row["id"])
            c["allocations"].sort(key=lambda row: row["id"])
    elif case.startswith("native_"):
        # Equal protocol-qualified events; no closed authority is granted in
        # the native-support read, just as in Python's historical selector.
        event = "ab" * 32
        for row in rows.values():
            row.update(external_id=event, external_id_kind="txid", raw_json=json.dumps({"chain": "bitcoin", "network": "regtest", "txid": event}))
        # Distinct wallet/event slots are required for the two receipts.
        second_wallet = {**book["anchor_wallets"][0], "id": "btc-second"}
        book["anchor_wallets"].append(second_wallet)
        rows["in-2"]["wallet_id"] = other_dest["wallet_id"] = "btc-second"
        rows["other-out"]["external_id"] = None
        rows["other-out"]["raw_json"] = "{}"
        rows["out"]["amount"] = source["amount_msat"] = 99
        rows["in-1"]["amount"] = dest["amount_msat"] = 60
        rows["in-2"]["amount"] = other_dest["amount_msat"] = 39
        fee["amount_msat"] = 0
        if case == "native_partial":
            rows["out"]["amount"] = source["amount_msat"] = 100
            for leg in (dest, other_dest): leg.update(transaction_id=None, anchor_transaction_id=None)
        if case == "native_contradicted": rows["in-2"]["amount"] = 40
        if case == "native_exact_sink":
            rows["out"]["amount"] = source["amount_msat"] = 100
            fee["amount_msat"] = 1
            explicit()
        if case == "native_other_asset":
            for row in (rows["in-1"], rows["in-2"]): row["asset"] = "USDT"
        seal()
    elif case.startswith("lineage_") or case.startswith("membership_") or case == "component_order":
        other = clone_component("aaa-other" if case == "component_order" else "other")
        if case in {"lineage_conflict", "lineage_ties"}:
            other["header"].update(revision=2, lineage_id=h["lineage_id"])
            if case == "lineage_ties":
                third = clone_component("aaa-revision")
                third["header"].update(revision=2, lineage_id=h["lineage_id"])
        else: other["header"]["lineage_id"] = other["header"]["id"]
        if case == "membership_empty_anchor":
            source["anchor_transaction_id"] = ""
            other["legs"][0]["anchor_transaction_id"] = ""
        if case == "component_order": other["header"]["revision"] = h["revision"] = 2
    elif case.startswith("revision_"):
        h["revision"] = 2
        target = {**h, "id": "previous", "revision": 1, "state": "superseded"}
        h["supersedes_component_id"] = target["id"]
        if case != "revision_missing": book["revision_headers"].append(target)
        if case == "revision_scope": target["profile_id"] = "other-profile"
        if case == "revision_lineage": target["lineage_id"] = "other-lineage"
        if case == "revision_order": target["revision"] = 2
        if case == "revision_backward": target["revision"] = 3
        if case == "revision_forward":
            h["supersedes_component_id"] = None
            h["superseded_by_component_id"] = target["id"]
    elif case.startswith("hash_"):
        field = {"hash_kind": "kind", "hash_payment": "payment_hash", "hash_payment_source": "payment_hash_source", "hash_refund_txid": "swap_refund_funding_txid", "hash_refund_vout": "swap_refund_funding_vout", "hash_wallet": "wallet_id", "hash_direction": "direction", "hash_asset": "asset", "hash_amount": "amount", "hash_fee": "fee", "hash_included": "amount_includes_fee", "hash_event": "external_id"}[case]
        value = {"swap_refund_funding_vout": 7, "amount": 101, "fee": 1, "amount_includes_fee": 1, "direction": "inbound", "asset": "LBTC"}.get(field, "changed:é 钱包")
        rows["out"][field] = value
        seal()
    elif case.startswith("asset_hash_upper_"):
        asset = chr(int(case.rsplit("_", 1)[1], 16))
        for row in rows.values(): row["asset"] = asset
        for leg in c["legs"]: leg["asset"] = asset
        seal()
    elif case == "integer_semantics":
        # CPython int accepts Unicode decimal digits and single underscores.
        for leg in c["legs"]:
            leg["amount_msat"] = " +" + "_".join(str(leg["amount_msat"]).translate(str.maketrans("0123456789", "٠١٢٣٤٥٦٧٨٩"))) + "\u0085"
    elif case == "time_semantics":
        source["occurred_at"] = "2026-W01-4T00:00:00+00:00"
        dest["occurred_at"] = "20260101T010000,1234567+01:00:00.123456"
        other_dest["occurred_at"] = "2026-01-01🐍00:00:00"
    else:
        if case == "no_legs": c["legs"] = []
        if case == "missing_source": source["role"] = "external"
        if case == "missing_destination": dest["role"] = other_dest["role"] = "external"
        if case == "unknown_role": dest["role"] = "future"
        if case == "unbalanced": dest["amount_msat"] += 1
        if case == "unit_mismatch": dest["exposure"] = "other"
        if case == "missing_time": dest.update(transaction_id=None, anchor_transaction_id=None, occurred_at=None)
        if case == "invalid_time": dest["occurred_at"] = "2026-02-30"
        if case == "chronology": source["occurred_at"] = "2026-01-03T00:00:00Z"
        if case == "chronology_after_boundary": source["occurred_at"] = "2026-01-03T00:00:00.000001Z"
        if case == "scope_invalid_chain": dest["chain"] = "future"
        if case == "scope_rail_chain": dest["chain"] = "liquid"
        if case == "scope_invalid_network": dest["network"] = "unknown"
        if case == "scope_mismatch": dest["network"] = "main"
        if case == "content_missing":
            h.update(expected_leg_count=None, expected_allocation_count=None, expected_economic_term_count=None)
        if case == "leg_count": h["expected_leg_count"] = 5
        if case == "allocation_count": h["expected_allocation_count"] = 1
        if case == "term_count": h["expected_economic_term_count"] = 1
        if case == "commitment_header": h["expected_evidence_count"] = None
        if case == "commitment_count": c["commitments"].pop()
        if case == "commitment_ordinals":
            for i, commitment in enumerate(c["commitments"]): commitment["ordinal"] = i + 1
        if case == "commitment_zero": c["commitments"] = []; h["expected_evidence_count"] = 0
        if case == "commitment_no_anchors":
            for leg in c["legs"]: leg.update(transaction_id=None, anchor_transaction_id=None)
        if case == "commitment_hash": c["commitments"][0]["detail_hash"] = "f" * 64
        if case == "commitment_volatile":
            # Pin event identity before refining its fallback fingerprint.
            # Otherwise changing the fingerprint would correctly change the
            # event key and never exercise the volatile detail exclusions.
            for row in rows.values():
                txid = hashlib.sha256(row["id"].encode()).hexdigest()
                row.update(external_id=txid, external_id_kind="txid", raw_json=json.dumps({"txid": txid, "chain": "bitcoin", "network": "regtest"}))
            seal()
            for row in rows.values():
                raw = json.loads(row["raw_json"])
                raw["new"] = "raw detail"
                row.update(fingerprint="refined", occurred_at="2026-01-02T00:00:00Z", confirmed_at="2026-01-03T00:00:00Z", raw_json=json.dumps(raw))
        if case == "commitment_rejected": rows["out"]["amount"] = -1
        if case == "anchor_missing": book["anchors"]["out"] = None
        if case == "anchor_retracted": source["transaction_id"] = None
        if case == "anchor_identity": source["anchor_transaction_id"] = "in-1"
        if case == "anchor_excluded": rows["out"]["excluded"] = 1
        if case == "anchor_source_direction": rows["out"]["direction"] = "inbound"
        if case == "anchor_destination_direction": rows["in-1"]["direction"] = "outbound"
        if case == "anchor_loss_direction": fee.update(transaction_id="in-1", anchor_transaction_id="in-1")
        if case == "anchor_asset": source["asset"] = "ETH"
        if case == "asset_upper_semantics":
            source["asset"] = "\u019b"
            rows["out"]["asset"] = "\ua7dc"
        if case == "anchor_wallet": source["wallet_id"] = "other"
        if case == "scope_enriched_mismatch":
            for leg in c["legs"]: leg.update(chain=None, network=None)
            rows["out"]["config_json"] = '{"chain":"bitcoin","network":"regtest"}'
            rows["in-1"]["config_json"] = '{"chain":"bitcoin","network":"main"}'
        if case == "transactionless_wallet_missing": source.update(transaction_id=None, anchor_transaction_id=None, wallet_id=None, occurred_at=None)
        if case == "anchor_row_time_invalid": rows["out"]["occurred_at"] = "invalid"
        if case == "anchor_time_invalid": source["occurred_at"] = "invalid"
        if case in {"anchor_time_mismatch", "anchor_time_warning"}: source["occurred_at"] = "2026-01-01T01:00:00Z"
        if case == "anchor_time_warning":
            row = rows["out"]
            row.update(observation_authority_version=1, observation_graph_hash=canonical_graph_hash(row["raw_json"]), observation_quantity_hash=canonical_observed_quantity_hash(row), observation_application_revision="refined")
        if case == "anchor_coverage": source["amount_msat"] = 101
        if case.startswith("anchor_partial"):
            c["economic_terms"] = [{"source_leg_id": source["id"], "target_leg_id": dest["id"], "reviewed_source_amount_msat": 50, "swap_fee_msat": 0, "payout_amount_msat": None}]
            source["amount_msat"] = {"anchor_partial": 50, "anchor_partial_zero": 0, "anchor_partial_excess": 101}[case]
        if case == "anchor_rail": source["rail"] = "lightning"
        if case == "anchor_chain": source["chain"] = "liquid"
        if case == "anchor_network":
            rows["out"]["config_json"] = '{"chain":"bitcoin","network":"regtest"}'
            source["network"] = "main"
        if case == "owned_wallet_missing": source["wallet_id"] = None
        if case == "retained_location": dest.update(role="retained", rail="untracked", wallet_id=None, transaction_id=None, anchor_transaction_id=None, location_ref="private:location")
        if case == "transaction_anchor_missing":
            for leg in c["legs"]: leg.update(transaction_id=None, anchor_transaction_id=None)
            c["commitments"] = []; h["expected_evidence_count"] = 0
    # Positive rail/topology controls and the independently reviewed X branches.
    def wallet(identifier, kind, config):
        existing = next((w for w in book["anchor_wallets"] if w["id"] == identifier), None)
        item = {**book["anchor_wallets"][0], "id": identifier, "kind": kind, "config_json": json.dumps(config)}
        if existing is None: book["anchor_wallets"].append(item)
        else: existing.update(item)
        return item

    def anchored_scope(leg, identifier, kind, rail, chain, network, asset="BTC"):
        w = wallet(identifier, kind, {"network": network} | ({"chain": chain} if chain else {}))
        leg.update(wallet_id=identifier, rail=rail, chain=chain, network=network, asset=asset)
        tx = leg["transaction_id"]
        if tx:
            rows[tx].update(wallet_id=identifier, wallet_kind=kind, config_json=w["config_json"], asset=asset, raw_json="{}", external_id=None)

    if case in {"peg_in", "peg_out", "asset_l_btc_alias"}:
        h.update(component_type="peg_in" if case == "asset_l_btc_alias" else case, evidence_grade="reviewed")
        liquid = [dest, other_dest] if case != "peg_out" else [source, fee]
        for leg in liquid: anchored_scope(leg, "liquid", "liquid-electrum", "liquid", "liquid", "elementsregtest", "LBTC")
        if case == "asset_l_btc_alias": dest["asset"] = "l-btc"
        seal()
    if case in {"lightning_chain_default", "lightning_kind_scope", "core_ln_kind_scope"}:
        anchored_scope(dest, "lnd", "core-ln" if case == "core_ln_kind_scope" else "lnd", "lightning", None, "regtest")
        if case == "lightning_chain_default": dest["chain"] = "liquid"
        else: dest["rail"] = "bitcoin"
        seal()
    if case.startswith("lightning_receive_"):
        k = case.removeprefix("lightning_receive_").replace("_", "-")
        for leg in [dest, other_dest]:
            anchored_scope(leg, "ln-" + leg["id"], k, "lightning" if k == "core-ln" else k, None, "regtest")
        seal()
    if case == "swap_terms":
        h.update(component_type="swap", evidence_grade="reviewed", conservation_mode="conversion", conversion_policy="reviewed", conversion_reviewed=1)
        c["legs"].remove(other_dest)
        source["amount_msat"] = 100
        dest["amount_msat"] = 250
        fee["amount_msat"] = 10
        rows["out"].update(amount=110, fee=10, amount_includes_fee=1)
        rows["in-1"]["amount"] = 250
        anchored_scope(dest, "liquid", "liquid-electrum", "liquid", "liquid", "elementsregtest", "USDT")
        dest.update(exposure="tether-usd", conservation_unit="asset-quantum")
        for leg, value in [(source, 1000), (dest, 900), (fee, 100)]: leg.update(valuation_unit="eur-cent", valuation_amount=value)
        explicit()
        c["allocations"][0]["source_amount_msat"] = 90
        c["economic_terms"] = [{"id": "swap-terms", "ordinal": 0, "source_leg_id": source["id"], "target_leg_id": dest["id"], "reviewed_source_amount_msat": 100, "swap_fee_msat": 10, "payout_amount_msat": 250}]
        seal()
    if case == "untracked_route":
        wallet("untracked", "untracked", {})
        c["legs"].remove(other_dest)
        dest.update(role="retained", amount_msat=source["amount_msat"]-fee["amount_msat"], wallet_id="untracked", transaction_id=None, anchor_transaction_id=None, rail="untracked", chain=None, network=None)
        explicit()
        other = clone_component("route-b")
        other["header"].update(lineage_id="route-b")
        other["legs"] = [dict(source, id="route-b:source", component_id="route-b", transaction_id=None, anchor_transaction_id=None, wallet_id="untracked", rail="untracked", chain=None, network=None, amount_msat=dest["amount_msat"], occurred_at="2026-01-04T00:00:00Z"), dict(dest, id="route-b:dest", component_id="route-b", role="destination", wallet_id="btc", rail="bitcoin", chain="bitcoin", network="regtest", transaction_id="in-1", anchor_transaction_id="in-1", occurred_at="2026-01-04T00:00:00Z")]
        other["allocations"] = []
        rows["in-1"].update(amount=dest["amount_msat"], occurred_at="2026-01-04T00:00:00Z")
        seal(); seal(other)
    if case in {"liquid_wallet_omitted_scope", "wallet_scope_conflict"}:
        for leg in [source, fee]: anchored_scope(leg, "liquid", "liquid-electrum", "liquid", "liquid", "elementsregtest", "LBTC")
        for leg in [dest, other_dest]:
            leg.update(transaction_id=None, anchor_transaction_id=None, wallet_id="manual-liquid", asset="LBTC", rail="", chain=None, network=None)
        wallet("manual-liquid", "liquid-electrum", {"chain":"liquid", "network": "liquidv1" if case == "wallet_scope_conflict" else "elementsregtest"})
        seal()
    if case in {"conversion_inferred", "conversion_inferred_chronology"}:
        c["legs"].remove(fee)
        source["amount_msat"] = rows["out"]["amount"] = 90
        rows["out"].update(fee=0, amount_includes_fee=0)
        source["valuation_amount"] = dest["valuation_amount"] = 180
        c["allocations"] = []
        if case.endswith("chronology"): source["occurred_at"] = "2026-01-04T00:00:00Z"
        seal()
    if case == "conversion_ambiguous":
        c["allocations"] = []
        source["occurred_at"] = "2026-01-04T00:00:00Z"
    if case == "native_namespace_gate":
        for row in rows.values(): row.update(wallet_kind="manual", config_json='{"network":"regtest"}', external_id=None, native_namespace="adapter", native_event_id="shared", raw_json='{"chain":"bitcoin","network":"regtest"}')
        rows["in-2"]["amount"] = 1000
        for leg in c["legs"]: leg.update(rail="bitcoin", chain="bitcoin", network="regtest")
        seal()
    if case == "native_excluded_rows":
        excluded = dict(rows["in-1"], id="excluded-in", wallet_id="excluded-wallet", amount=1000, excluded=1)
        book["transactions"].append(excluded)
    if case == "lineage_other_profile":
        book["components"].pop()  # Baseline lineage branch cloned a competing header.
        book["revision_headers"].append({**h, "id":"other-profile", "profile_id":"elsewhere"})
    if case == "authored_source_default": h["authored_source"] = None
    if case == "retained_reviewed_notes":
        dest.update(role="retained", rail="untracked", wallet_id=None, transaction_id=None, anchor_transaction_id=None, location_ref=None, notes="reviewed_residual:retained_custody")
        seal()
    if case in {"liquid_substring_kind", "liquid_kind_conflict"}:
        for leg in c["legs"]: anchored_scope(leg, "custom-liquid", "custom-liquid-node", "liquid", None, None)
        if case == "liquid_kind_conflict": source["rail"] = "bitcoin"
        seal()
    if case == "generic_elements_domain":
        for leg in c["legs"]: leg.update(transaction_id=None, anchor_transaction_id=None, rail="future", chain="future", network="elements")
        dest.update(rail="bitcoin", chain="bitcoin", network="main")
        seal()
    if case == "fee_null_domain": source["network"] = None
    if case == "missing_time_empty": dest.update(transaction_id=None, anchor_transaction_id=None, occurred_at="")
    if case == "transaction_id_empty": dest.update(transaction_id="", anchor_transaction_id="")
    if case == "zero_amount_chronology":
        zero = new_leg("source", 0, when="2026-01-05T00:00:00Z")
        c["allocations"] = [{"id":"zero-edge", "ordinal":0, "component_id":h["id"], "workspace_id":h["workspace_id"], "profile_id":h["profile_id"], "created_at":h["created_at"], "source_leg_id":zero["id"], "sink_leg_id":dest["id"], "source_amount_msat":0, "sink_amount_msat":0}]
    if case == "authoritative_chronology":
        row = rows["out"]
        row.update(occurred_at="2026-01-04T00:00:00Z", observation_authority_version=1, observation_graph_hash=canonical_graph_hash(row["raw_json"]), observation_quantity_hash=canonical_observed_quantity_hash(row), observation_application_revision="refined")
    if case == "inactive_header": h["state"] = "superseded"
    # Child row order is the stored ordinal/id order, not insertion order.
    for item in book["components"]:
        for field in ("legs", "allocations", "economic_terms", "commitments"):
            item[field].sort(key=lambda r: (r.get("ordinal", 0), r.get("id", "")))
        if case not in {"content_missing", "leg_count", "allocation_count", "term_count", "no_legs"}:
            item["header"].update(expected_leg_count=len(item["legs"]), expected_allocation_count=len(item["allocations"]), expected_economic_term_count=len(item["economic_terms"]))
    book["transactions"].sort(key=lambda r: (r["occurred_at"], r["created_at"], r["id"]))
    from kassiber.core.custody_compute import compute_components_stage
    components, _ = compute_components_stage(book)
    active = sum(item["effective_state"] == "active" for item in components)
    assert active == EXPECTED_ACTIVE_COMPONENTS.get(case, 0), (case, seed, active, components)
    return book
