from __future__ import annotations

import json
import copy
import os
from pathlib import Path
import random
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).parent))
from generate_analysis_index import BLOCK, DOMAIN, OTHER, SEED_TABLE_COUNT, TX, assertion, database, snapshot
from kassiber.core.chain_analysis.confirmations import ConfirmationOverlay
from kassiber.core.chain_analysis.index import _Builder, observer_index
from kassiber.core.chain_analysis.occurrences import OccurrenceResolver
from generate_coinjoin_shapes import cases as coinjoin_shape_cases

DEST = ROOT / "crates/wallets/tests/data/analysis_ingestion.json"
IDENT = f"bitcoin:main:domain:{DOMAIN}:occ:{BLOCK}:0:tx:{TX}"


def q(value):
    return "NULL" if value is None else "'" + str(value).replace("'", "''") + "'"


def anchored(*, grant="g", status=None, tip=110, tip_hash=OTHER, at="2026-01-01T00:00:00Z", code=None, **kwargs):
    if status is None:
        status = {"confirmed": True, "block_height": 100, "block_hash": BLOCK}
    return assertion(f"{BLOCK}:0", grant=grant, status=status, **kwargs) + (
        "UPDATE chain_analysis_acquisition_grants SET verified_tip_height=" + q(tip)
        + ",verified_tip_hash=" + q(tip_hash) + ",verified_tip_at=" + q(at)
        + ",last_code=" + q(code) + " WHERE id=" + q(grant) + ";"
    )


def evaluate(case, conn=None):
    own = conn is None
    conn = database(case.get("sql", "")) if own else conn
    before = table_rows(conn) if own else None
    try:
        if case["operation"] == "ingestion":
            builder = _Builder("p", occurrences=OccurrenceResolver(conn, "p"))
            for request in case["requests"]:
                row = copy.deepcopy(request["row"])
                if request["source"] == "transaction":
                    builder.transaction(row, cached=request.get("cached", False))
                elif request["source"] == "inventory":
                    builder.inventory(row)
                elif request["source"] == "reference":
                    builder.reference(row, acquired=request.get("acquired", False))
                elif request["source"] == "relations":
                    builder.relations(row["decisions"], row["economics"], row["fresh"])
            return snapshot(observer_index(builder.finish(), case.get("observer", "owner")))
        overlay = ConfirmationOverlay(conn, "p")
        return [overlay.apply(node) for node in case["nodes"]]
    finally:
        if own:
            assert table_rows(conn) == before
            conn.close()


def table_rows(conn):
    names = [r[0] for r in conn.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
    assert len(names) == SEED_TABLE_COUNT
    return {name: sorted((tuple(r) for r in conn.execute(f'SELECT * FROM "{name}"')), key=repr) for name in names}


def vectors():
    cases = []
    def add(name, sql="", nodes=None):
        if nodes is None:
            nodes = [{"id": IDENT, "confirmations": 999, "status": "conflicting", "wallet_ids": ["private"]},
                {"id": IDENT.replace(":tx:", ":out:") + ":0", "confirmations": 999}, {"id": IDENT, "confirmations": 0}]
        case = {"name": name, "group": "confirmations", "operation": "confirmations", "sql": sql, "nodes": nodes}
        case["expected"] = evaluate(case)
        cases.append(case)

    add("no-sources")
    add("anchored", anchored())
    add("depth-one", anchored(tip=100, tip_hash=BLOCK))
    for name, kwargs in [
        ("inactive", {"active": 0}), ("reconciling", {"active": 2}),
        ("other-profile", {"profile": "other"}), ("other-domain", {"domain": "foreign"}),
        ("other-transaction", {"txid": OTHER}),
        *[(f"tip-{i}", {"tip": v}) for i, v in enumerate([None, -1, 99, "invalid", 100.5, 9223372036854775807])],
        *[(f"hash-{i}", {"tip_hash": v}) for i, v in enumerate([None, "", "g" * 64, OTHER.upper(), "f" * 63, OTHER + "\n"])],
        ("same-height-wrong-hash", {"tip": 100}),
        *[(f"code-{i}", {"code": v}) for i, v in enumerate(["chain_analysis_stale", "reorg_reconciling", "other", ""])],
        *[(f"time-{i}", {"at": v}) for i, v in enumerate([None, "", "invalid", "2026-02-30T00:00:00Z", "2026-01-01", "2026-01-01T00:00:00", "20260101T000000Z", "2026-W01-4T00:00:00Z", "2026-01-01T00:00:00.123456Z", " 2026-01-01T01:00:00+01:00 ", "2026-01-01T00:00:00+00:00:00.5"])],
    ]:
        add(name, anchored(**kwargs))
    base = {"confirmed": True, "block_height": 100, "block_hash": BLOCK}
    for field, values in {
        "confirmed": [False, None, 1, "true"], "removed": [True, False, 1],
        "conflicted": [True, False, 1], "block_hash": [None, OTHER, BLOCK.upper()],
        "block_height": [None, -1, True, 100.0, "100", 0, 111, 10**50],
    }.items():
        for i, value in enumerate(values):
            add(f"status-{field}-{i}", anchored(status={**base, field: value}))
    for i, status in enumerate(["broken", "null", "[]", "{}"]):
        add(f"invalid-status-{i}", anchored(status=status))
    for i, at in enumerate(["2026-01-01T00:00:01Z", "2025-12-31T23:59:59Z", "2026-01-01T01:00:00+01:00"]):
        add(f"newest-{i}", anchored(grant="old") + anchored(grant="new", at=at, tip=120))
    add("tie-same-count", anchored(grant="a") + anchored(grant="b", at="2026-01-01T01:00:00+01:00"))
    add("tie-different-height", anchored(grant="a") + anchored(grant="b", status={**base, "block_height": 101}, tip=111))
    add("newest-invalid-does-not-suppress", anchored(grant="a") + anchored(grant="b", at="2027-01-01T00:00:00Z", code="chain_analysis_stale"))
    for field in ["verified_tip_height", "verified_tip_hash", "verified_tip_at", "last_code", "status"]:
        add(f"missing-column-{field}", anchored() + f"ALTER TABLE chain_analysis_acquisition_grants RENAME COLUMN {field} TO legacy_{field};")
    for i, ident in enumerate([IDENT + "\n", IDENT.replace(":0:tx:", ":00:tx:"), IDENT.replace(BLOCK, BLOCK.upper()), IDENT.replace(":domain:" + DOMAIN + ":occ:" + BLOCK + ":0", ""), IDENT.replace("domain:" + DOMAIN + ":", ""), IDENT.replace(":tx:", ":record:"), IDENT.replace(DOMAIN, "foreign"), IDENT.replace(TX, OTHER), IDENT.replace(":tx:", ":out:") + ":7"]):
        add(f"identity-{i}", anchored(), [{"id": ident, "confirmations": 13}])
    cases.extend(ingestion_vectors())
    return cases


def transaction(raw=None, **kwargs):
    row = {"id": "record", "wallet_id": "cold", "asset": "BTC", "amount": 123, "direction": "inbound", "occurred_at": "private-time", "raw_json": {"txid": TX, "vin": [{"txid": OTHER, "vout": 0, "prevout": {"value": 12, "scriptpubkey": "0014" + "11" * 20}}], "vout": [{"value": 10, "scriptpubkey": "0014" + "22" * 20}], "version": 2, "locktime": 0}}
    if raw is not None:
        row["raw_json"] = raw
    row.update(kwargs)
    return {"source": "transaction", "row": row}


def inventory(**kwargs):
    return {"source": "inventory", "row": {"id": "utxo", "wallet_id": "cold", "chain": "bitcoin", "network": "main", "txid": TX, "vout": 0, "amount": 10000, "asset": "BTC", "branch_index": 1, "script_pubkey": "0014" + "22" * 20, **kwargs}}


def ingestion_vectors():
    cases = []
    def add(name, requests, sql="", variants=False):
        case = {"name": name, "group": "ingestion", "operation": "ingestion", "requests": copy.deepcopy(requests), "sql": sql}
        if variants:
            results = []
            for seed in range(8):
                env = os.environ.copy()
                env["PYTHONHASHSEED"] = str(seed)
                result = subprocess.run([sys.executable, str(Path(__file__).resolve()), "--evaluate"], input=json.dumps(case).encode(), env=env, capture_output=True, check=True, timeout=30)
                results.append(json.loads(result.stdout))
            case["variants"] = results
            case["expected"] = results[0]
        else:
            case["expected"] = evaluate(case)
        cases.append(case)

    add("empty-builder", [])
    add("ordinary-spend", [transaction()])
    add("ordinary-cached", [{**transaction(), "cached": True}])
    for i, raw in enumerate(["broken", "[]", {"tx": {"txid": TX, "vin": [{"coinbase": "01"}], "vout": [{"value": 1}]}}, {}, {"txid": TX, "confirmations": -1}, {"txid": TX, "removed": True}, {"txid": TX, "status": {"confirmed": False}}, {"txid": OTHER}]):
        add(f"raw-{i}", [transaction(raw, external_id=TX)])
    for key, values in {
        "asset": ["LBTC", "USD", None], "chain": ["bitcoin", "liquid", "lightning", "future"],
        "network": ["mainnet", "testnet", "regtest", "signet", "bad"],
        "wallet_kind": ["phoenix", "coreln"], "amount": [-1, True, 1.0, "0001", " 1 ", 10**40],
    }.items():
        for i, value in enumerate(values):
            add(f"scope-{key}-{i}", [transaction({}, **{key: value})])
    for i, value in enumerate([0, 1, 100, -2, True, "3", 3.0, None]):
        raw = transaction()["row"]["raw_json"]
        raw["confirmations"] = value
        add(f"confirmations-{i}", [transaction(raw)])
    for i, value in enumerate([None, "", -1, "1", "0.000000015", "100_000", 1.5, True, 10**35]):
        raw = transaction()["row"]["raw_json"]
        raw["vout"] = [{"value": value}]
        add(f"output-amount-{i}", [transaction(raw)])
    for i, output in enumerate([
        {"value_sats": 1.9}, {"value_sats": "1.9", "value": 10}, {"scriptPubKey": {"hex": "AA", "addresses": [" first "]}},
        {"script_hex": "aa bb", "address": 123}, {"scriptpubkey": "bad"}, {"scriptpubkey": "", "valuecommitment": "08"},
        {"n": "01", "value": 5}, {"n": True, "value": 5}, {"n": 2, "value": 5},
        {"scriptPubKey": "6A00", "value": 0}, {"scriptpubkey": "  "}, "invalid", {},
    ]):
        raw = transaction()["row"]["raw_json"]
        raw["vout"] = [output]
        add(f"output-shape-{i}", [transaction(raw)])
    for i, item in enumerate([None, {}, {"coinbase": "01"}, {"is_coinbase": True}, {"txid": "0" * 64, "vout": 0}, {"txid": OTHER, "vout": True}, {"txid": OTHER, "vout": 1.9}, {"txid": OTHER, "vout": "01"}, {"txid": OTHER, "vout": -1}, {"is_pegin": True, "txid": OTHER, "vout": 0}, {"is_peg_in": True}]):
        raw = transaction()["row"]["raw_json"]
        raw["vin"] = [item]
        add(f"input-shape-{i}", [transaction(raw)])
    for kind in [None, "coinjoin", "payjoin", "collaborative", "payment_in_coinjoin", "sweep", "invalid", " "]:
        for reviewed in [False, True]:
            add(f"privacy-{kind}-{reviewed}", [transaction(privacy_boundary=kind, review_status="reviewed" if reviewed else "unreviewed")])
    for key in ["privacy_boundary_kind", "privacyHop", "isLikelyCoinJoin"]:
        raw = transaction()["row"]["raw_json"]
        raw[key] = True if key == "isLikelyCoinJoin" else "payjoin"
        add(f"imported-{key}", [transaction(raw)])
    for inputs, outputs in [(5, 5), (4, 5), (5, 13), (20, 20)]:
        raw = transaction()["row"]["raw_json"]
        raw["vin"] *= inputs
        raw["vout"] *= outputs
        add(f"collaborative-shape-{inputs}-{outputs}", [transaction(raw)])
    for field, value in [("value", 8), ("asset", "foreign"), ("address", "another"), ("scriptpubkey", "51")]:
        original = transaction()
        original["row"]["raw_json"]["vout"][0]["address"] = "address"
        changed = copy.deepcopy(original)
        changed["row"]["id"] = "other-record"
        changed["row"]["raw_json"]["vout"][0][field] = value
        if field == "asset":
            for request in [original, changed]:
                request["row"]["raw_json"].update(chain="liquid", network="liquidv1")
        add(f"equal-rank-conflict-{field}", [original, changed, copy.deepcopy(original)])
        add(f"cached-cannot-conflict-{field}", [original, {**changed, "cached": True}])
    for name, change in [
        ("complete-other-shape", {"vin": [{"txid": BLOCK, "vout": 0}], "vout": [{"value": 10}]}),
        ("partial-extra-output", {"vin": [], "vout": [{"n": 3, "value": 10}]}),
        ("partial-extra-input", {"vin": [{"txid": BLOCK, "vout": 0}]}),
        ("duplicate-input", {"vin": [{"txid": OTHER, "vout": 0}] * 2}),
        ("duplicate-output", {"vout": [{"n": 0, "value": 10}] * 2}),
        ("feature-disagreement", {"version": 1}),
        ("confirmation-conflict", {"confirmations": 8}),
    ]:
        first = transaction()
        first["row"]["raw_json"]["confirmations"] = 3
        second = copy.deepcopy(first)
        second["row"]["id"] = "second"
        second["row"]["raw_json"].update(change)
        add(name, [first, second])
    first = transaction()
    second = copy.deepcopy(first)
    second["row"]["raw_json"]["txid"] = BLOCK
    second["row"]["id"] = "replacement"
    add("competing-spends", [first, second])
    first["row"]["raw_json"]["removed"] = True
    add("retracted-spender", [first, second])
    add("inventory", [inventory()])
    add("inventory-precedence", [transaction(), inventory(amount=8000, confirmations=3)])
    add("inventory-before-transaction", [inventory(amount=8000), transaction()])
    add("ambiguous-inventory", [inventory(), inventory(wallet_id="hot", id="utxo-two")])
    add("duplicate-same-wallet-inventory", [inventory(), inventory()])
    add("inventory-confirmation-conflict", [inventory(confirmations=1), inventory(confirmations=2)])
    add("inventory-spent", [inventory(spent_by=BLOCK), transaction()])
    add("inventory-spent-twice", [inventory(spent_by=BLOCK), inventory(spent_by=OTHER)])
    for field, values in {"branch_label": ["change", "p2wpkh-change", "receive", "p2sh-p2wpkh external", "deposit", "internal", "unknown"], "branch_index": [None, True, "01", " 1 ", -1, 0, 2, 7, 8], "amount": [True, 1.5, -1, "12", 10**45], "chain": ["future", "lightning"], "txid": ["bad", None], "vout": [True, -1, "01"]}.items():
        for i, value in enumerate(values):
            add(f"inventory-{field}-{i}", [inventory(**{field: value})])
    for role in [None, "owned", "fee", "external"]:
        for committed in [False, True]:
            raw = transaction()["row"]["raw_json"]
            raw.update(chain="liquid", network="liquidv1")
            for output in [raw["vout"][0], raw["vin"][0]["prevout"]]:
                output.update(asset="asset", role=role)
                if committed:
                    output.update(valuecommitment="08", assetcommitment="0a")
            add(f"liquid-{role}-{committed}", [transaction(raw)])
    payload = transaction()["row"]["raw_json"]
    ref = {"source": "reference", "row": {"chain": "bitcoin", "network": "main", "txid": TX, "schema_version": 1, "payload_json": payload}}
    add("reference-cache", [ref])
    add("reference-precedence", [transaction(), ref])
    for key, values in {"payload_json": ["broken", "[]", {}, {"txid": OTHER}, {"chain": "liquid"}], "schema_version": [0, True, "1", 2], "chain": ["future", "lightning"], "txid": ["bad", None]}.items():
        for i, value in enumerate(values):
            for acquired in [False, True]:
                changed = copy.deepcopy(ref)
                changed["acquired"] = acquired
                changed["row"][key] = value
                add(f"reference-{key}-{i}-{acquired}", [changed])
    for removed in [False, True]:
        acquired = copy.deepcopy(ref)
        acquired.update(acquired=True)
        acquired["row"].update(grant_id="g", occurrence_id=f"{BLOCK}:0", domain_id=DOMAIN, observed_at="private-time", source_name="private-source", status_json={"confirmed": True, "block_hash": BLOCK, "block_height": 100, "removed": removed})
        add(f"acquired-{removed}", [acquired], anchored())
        acquired["row"]["status_json"]["block_hash"] = OTHER
        add(f"acquired-wrong-anchor-{removed}", [acquired], anchored())
    for fresh in [False, True]:
        for economics in [False, True]:
            relation = {"source_transaction_id": "record", "target_transaction_id": "target", "source_start_msat": 1, "source_end_msat": 4, "source_amount_msat": 3, "target_amount_msat": 2, "source_asset": "BTC", "target_asset": "LBTC", "decision_id": "decision", "relation_id": "relation", "component_id": "component", "state": "internal_verified", "relation_kind": "carry", "basis_state": "reviewed"}
            rows = [transaction(), transaction({}, id="target", wallet_kind="phoenix")]
            overlay = {"source": "relations", "row": {"fresh": fresh, "decisions": [] if economics else [relation], "economics": [relation] if economics else []}}
            add(f"relations-{fresh}-{economics}", [*rows, overlay])
            add(f"relations-missing-target-{fresh}-{economics}", [rows[0], overlay])
            add(f"relations-missing-source-{fresh}-{economics}", [overlay])
    decision = {"source_transaction_id": "record", "target_transaction_id": "target", "source_start_msat": 1, "source_end_msat": 4, "source_asset": "BTC", "decision_id": "shared", "state": "internal_verified", "relation_kind": "carry"}
    economic = {**decision, "relation_id": "shared", "source_amount_msat": 9, "source_asset": "LBTC", "state": "reviewed", "relation_kind": "conversion"}
    first = {"source": "relations", "row": {"fresh": True, "decisions": [decision], "economics": []}}
    second = {"source": "relations", "row": {"fresh": False, "decisions": [], "economics": [economic]}}
    for missing in [False, True]:
        rows = [transaction()] if missing else [transaction(), transaction({}, id="target", wallet_kind="phoenix")]
        add(f"relations-repeated-{missing}", [*rows, first, second])
        add(f"relations-table-collision-{missing}", [*rows, {"source": "relations", "row": {"fresh": True, "decisions": [decision], "economics": [economic]}}])
    sql = ""
    rows = []
    for number in range(1, 5):
        txid = f"{number:064x}"
        sql += assertion(f"{BLOCK}:{number}", txid=txid) + assertion(f"{OTHER}:{number}", txid=txid)
        rows.append(transaction({"txid": txid}, id=f"ambiguous-{number}"))
    add("ambiguous-hashseed-variants", rows, sql, variants=True)
    for hint in [None, f"{OTHER}:3", f"{OTHER}:03"]:
        acquired = copy.deepcopy(ref)
        acquired.update(acquired=True)
        acquired["row"].update(grant_id="g", domain_id=DOMAIN, occurrence_id=f"{BLOCK}:0", status_json={"block_hash": BLOCK})
        acquired["row"]["payload_json"]["vin"][0]["reference_occurrence_id"] = hint
        sql = assertion(f"{OTHER}:3", txid=OTHER)
        add(f"historical-reference-parent-{hint}", [acquired], sql)
    binding = "INSERT INTO book_network_bindings(profile_id,environment_id,environment,revision,domains_json,acknowledgements_json,inventory_digest,created_at) VALUES('p','env','main',1," + q(json.dumps([{"chain": "bitcoin", "network": "main", "domain_id": DOMAIN}])) + ",'[]','fixed','fixed');"
    for i, request in enumerate([transaction(), transaction(network="regtest"), inventory(network="regtest"), {**ref, "acquired": True, "row": {**ref["row"], "domain_id": "foreign"}}]):
        add(f"bound-source-{i}", [request], binding)
    rng = random.Random(2105)
    for number in range(30):
        requests = []
        for i in range(rng.randrange(1, 6)):
            row = transaction(id=f"random-{i}", wallet_id=rng.choice(["cold", "hot", None]), privacy_boundary=rng.choice([None, "coinjoin", "payjoin"]))
            raw = row["row"]["raw_json"]
            raw["txid"] = rng.choice([TX, OTHER, BLOCK])
            raw["version"] = rng.choice([1, 2])
            raw["vin"][0]["txid"] = rng.choice([TX, OTHER, BLOCK])
            raw["vout"][0]["value"] = rng.choice([None, 0, 5, 10])
            raw["confirmations"] = rng.choice([-1, 0, 1, 8])
            raw["vout"][0]["address"] = rng.choice(["one", "two", None])
            requests.append(row)
        add(f"seeded-merge-{number}", requests)
    for vector in coinjoin_shape_cases():
        add(f"coinjoin-shape-{vector['name']}", [transaction(vector["raw"])])
    for case in list(cases):
        if case["name"].startswith("coinjoin-shape-") or case["name"] in {"ordinary-spend", "competing-spends", "retracted-spender", "inventory-precedence", "inventory-spent-twice", "relations-True-False", "relations-False-True", "acquired-False"} or case["name"].startswith("liquid-"):
            public = copy.deepcopy(case)
            public.update(name="public-" + case["name"], observer="public")
            public["expected"] = evaluate(public)
            cases.append(public)
    return cases


def main():
    cases = vectors()
    encoded = "[\n" + ",\n".join(json.dumps(case, ensure_ascii=True, separators=(",", ":")) for case in cases) + "\n]\n"
    if "--write" in sys.argv:
        DEST.write_text(encoded)
    elif json.loads(DEST.read_text()) != cases:
        raise SystemExit("analysis ingestion vectors drifted")
    print(f"{len(cases)} analysis ingestion vectors")


if __name__ == "__main__":
    if "--evaluate" in sys.argv:
        print(json.dumps(evaluate(json.load(sys.stdin)), sort_keys=True))
    else:
        main()
