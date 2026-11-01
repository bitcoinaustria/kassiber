from __future__ import annotations

import copy
import json
import os
from pathlib import Path
import random
import subprocess
import sys
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).parent))
from generate_analysis_index import BLOCK, DOMAIN, OTHER, TX, assertion, database, snapshot
from generate_analysis_ingestion import anchored, q, table_rows, transaction
from kassiber.core.chain_analysis.index import build_index, observer_index
from kassiber.errors import AppError

DEST = ROOT / "crates/wallets/tests/data/analysis_sources.json"
INSTANT = "2026-01-01T00:00:00Z"


def update(table, values, where):
    return f"UPDATE {table} SET " + ",".join(f"{key}={q(value)}" for key, value in values.items()) + f" WHERE {where};"


def insert(table, values):
    return f"INSERT INTO {table}(" + ",".join(values) + ") VALUES(" + ",".join(q(v) for v in values.values()) + ");"


def raw(**kwargs):
    value = copy.deepcopy(transaction()["row"]["raw_json"])
    value.update(kwargs)
    return json.dumps(value)


def stored(**kwargs):
    return update("transactions", {"raw_json": raw(), **kwargs}, "id='in'")


def cache(**kwargs):
    return insert("transaction_graph_cache", {"schema_version": 1, "chain": "bitcoin", "network": "main", "txid": TX, "payload_json": raw(), "created_at": "fixed", "updated_at": "fixed", **kwargs})


def inventory(**kwargs):
    return insert("wallet_utxos", {"id": "utxo", "workspace_id": "ws", "profile_id": "p", "wallet_id": "cold", "chain": "bitcoin", "network": "main", "txid": TX, "vout": 0, "outpoint": f"{TX}:0", "amount": 10000, "asset": "BTC", "confirmation_status": "confirmed", "first_seen_at": "fixed", "last_seen_at": "fixed", "script_pubkey": "0014" + "22" * 20, "branch_index": 1, **kwargs})


def label(**kwargs):
    return insert("chain_analysis_labels", {"id": "label", "profile_id": "p", "chain": "bitcoin", "network": "main", "subject": TX, "label": "Private label", "category": "service", "source": "Local note", "confidence": "reviewed", "created_at": "fixed", "updated_at": "fixed", **kwargs})


def dataset(*, ident="pack", manifest=None, claims=None, **kwargs):
    manifest = {"dataset_key": ident, "name": "Evidence 雪", "version": "one", "chain": "bitcoin", "network": "main", "source": "Declared source", "license": "Local fixture", "attribution_method": "declared", "visibility": kwargs.get("visibility", "public"), "valid_from": None, "valid_until": None, **(manifest or {})}
    sql = insert("chain_analysis_datasets", {"id": ident, "profile_id": "p", "dataset_key": ident, "version": "one", "chain": "bitcoin", "network": "main", "visibility": "public", "status": "active", "manifest_json": json.dumps(manifest), "content_sha256": "fixture", "created_at": "fixed", "updated_at": "fixed", **kwargs})
    if claims is None:
        claims = [{"subject": f"tx:{TX}"}, {"subject": f"out:{TX}:0"}, {"subject": "script:0014" + "22" * 20}]
    for number, claim in enumerate(claims, 1):
        sql += insert("chain_analysis_dataset_claims", {"dataset_id": ident, "record_number": number, "subject": f"tx:{TX}", "label": "Declared entity", "category": "service", "source_record_json": '{"retained":"雪"}', **claim})
    return sql


def evaluate(case, conn=None):
    own = conn is None
    conn = database(case.get("sql", "")) if own else conn
    before = table_rows(conn)
    try:
        with patch("kassiber.core.chain_analysis_datasets.now_iso", return_value=case.get("as_of", INSTANT)):
            try:
                index = build_index(conn, case.get("profile", "p"), observer=case.get("dataset_observer", "owner"))
                result = snapshot(observer_index(index, case.get("observer", "owner")))
                if case.get("summary"):
                    return {"snapshot_id": result["snapshot_id"], "coverage": result["coverage"], "label_count": len(result["labels"]), "first_label": result["labels"][0], "last_label": result["labels"][-1]}
                return result
            except AppError as error:
                return {"error": error.code}
    finally:
        assert table_rows(conn) == before
        if own:
            conn.close()


def vectors():
    cases = []
    def add(name, sql="", variants=False, **kwargs):
        case = {"name": name, "operation": "sources", "sql": sql, **kwargs}
        if variants:
            results = []
            for seed in range(8):
                env = os.environ.copy()
                env["PYTHONHASHSEED"] = str(seed)
                child = subprocess.run([sys.executable, str(Path(__file__).resolve()), "--evaluate"], input=json.dumps(case).encode(), env=env, capture_output=True, check=True, timeout=30)
                results.append(json.loads(child.stdout))
            case["variants"] = results
            case["expected"] = results[0]
        else:
            case["expected"] = evaluate(case)
        cases.append(case)

    add("seeded-book")
    add("other-profile", profile="other")
    add("absent-profile", profile="absent")
    add("stored", stored())
    add("excluded-is-still-an-observation", stored(excluded=1))
    add("duplicate-transaction", stored() + update("transactions", {"raw_json": raw(), "external_id": TX}, "id='out'"))
    for kind in ["phoenix", "coreln", "bitcoin", "address", "exchange"]:
        add(f"wallet-kind-{kind}", stored() + update("wallets", {"kind": kind}, "id='cold'"))
    for index, config in enumerate(["broken", "[]", '{"network":"regtest"}', '{"network":"testnet"}', '{"network":"bad"}', '{"chain_instance_id":"lab"}', '{"network":"regtest","chain_instance_id":"lab"}']):
        add(f"wallet-config-{index}", stored() + update("wallets", {"config_json": config}, "id='cold'"))
    for name, values in [("ordinary", {}), ("stale-schema", {"schema_version": 0}), ("bad-payload", {"payload_json": "broken"}), ("wrong-txid", {"payload_json": raw(txid=OTHER)}), ("other-network", {"network": "regtest"}), ("other-chain", {"chain": "liquid", "network": "liquidv1"})]:
        add(f"cache-{name}", cache(**values))
    add("cache-stored-inventory", stored() + cache() + inventory(amount=8000))
    add("inventory-only", inventory())
    add("inventory-other-profile", inventory(profile_id="other", wallet_id="other"))
    add("inventory-ambiguous-wallet", inventory() + inventory(id="utxo-two", wallet_id="hot"))
    add("cache-is-global", cache(), profile="other")
    for index, payload in enumerate([raw(), "broken", "[]", "{}", raw(txid=OTHER)]):
        sql = insert("chain_analysis_observations", {"profile_id": "p", "chain": "bitcoin", "network": "main", "txid": TX, "payload_json": payload, "source_name": "Local source", "observed_at": INSTANT})
        add(f"legacy-acquired-{index}", sql)
    for active in [0, 1, 2]:
        add(f"assertion-active-{active}", stored() + assertion(f"{BLOCK}:0", active=active))
    for name, sql in [("anchored", anchored()), ("wrong-domain", assertion(f"{BLOCK}:0", domain="foreign")), ("other-profile", assertion(f"{BLOCK}:0", profile="other"))]:
        add(f"assertion-{name}", stored() + sql)
    sql = ""
    for number, record in enumerate(["in", "out", "earn", "excluded"]):
        txid = f"{number + 1:064x}"
        sql += update("transactions", {"raw_json": raw(txid=txid), "asset": "BTC"}, f"id={q(record)}")
        sql += assertion(f"{BLOCK}:{number}", txid=txid) + assertion(f"{OTHER}:{number}", txid=txid)
    add("ambiguous-hashseed-variants", sql, variants=True)
    for index, values in enumerate([{}, {"deleted": 1}, {"network": "mainnet"}, {"network": "testnet"}, {"network": "bad"}, {"chain": "invalid"}, {"chain": "liquid", "network": "liquidv1"}, {"subject": f"{TX}:0"}, {"subject": "no-match"}, {"profile_id": "other"}, {"cluster_defining": 1}]):
        add(f"authored-label-{index}", stored() + label(**values))
    add("authored-label-order", stored() + label(id="z") + label(id="a"))
    for index, values in enumerate([{"last_processed_at": None}, {"journal_input_version": 8}, {"last_processed_input_version": 0}, {"last_processed_tx_count": 5}, {"last_processed_tx_count": 4}]):
        add(f"custody-freshness-{index}", stored() + update("profiles", values, "id='p'"))
    add("custody-excluded-count", stored(excluded=1) + update("profiles", {"last_processed_tx_count": 3}, "id='p'"))
    for table in ["wallets", "transactions", "transaction_graph_cache", "wallet_utxos", "chain_analysis_observations", "chain_analysis_reference_assertions", "chain_analysis_labels", "chain_analysis_datasets", "journal_custody_decisions", "journal_custody_economic_relations", "profiles", "book_network_bindings", "chain_analysis_acquisition_grants"]:
        add(f"missing-{table}", f"ALTER TABLE {table} RENAME TO unavailable_{table};")
    add("datasets-empty", stored())
    add("dataset-matches", stored() + dataset())
    add("dataset-private", stored() + dataset(visibility="private"))
    add("dataset-conflicting-claims", stored() + dataset() + dataset(ident="private", visibility="private", claims=[{"label": "Different entity"}]))
    for status in ["staging", "superseded", "revoked", "failed"]:
        add(f"dataset-status-{status}", stored() + dataset(status=status))
    for key in ["valid_from", "valid_until"]:
        for suffix, instant in [("before", "2025-01-01T00:00:00Z"), ("equal", INSTANT), ("after", "2027-01-01T00:00:00Z")]:
            add(f"dataset-manifest-{key}-{suffix}", stored() + dataset(manifest={key: instant}))
            add(f"dataset-claim-{key}-{suffix}", stored() + dataset(claims=[{key: instant}]))
    add("dataset-source-override", stored() + dataset(claims=[{"source_record_json": '{"source":null,"attribution_method":"exact"}'}]))
    add("dataset-other-domain", stored() + dataset(network="regtest"))
    add("dataset-other-profile", stored() + dataset(profile_id="other"))
    add("dataset-missing-subject", stored() + dataset(claims=[{"subject": f"tx:{BLOCK}"}]))
    add("dataset-aliases", stored() + label(subject=f"tx:{TX}") + label(id="out", subject=f"out:{TX}:0") + dataset())
    add("script-too-large", stored(raw_json=raw(vout=[{"value": 1, "scriptpubkey": "51" * 10001}])))
    add("script-at-limit", stored(raw_json=raw(vout=[{"value": 1, "scriptpubkey": "51" * 10000}])) + dataset(claims=[{"subject": "script:" + "51" * 10000}]))
    add("script-with-whitespace", stored(raw_json=raw(vout=[{"value": 1, "scriptpubkey": "aa  bb"}])))
    add("outpoint-outside-dataset-range", stored(raw_json=raw(vout=[{"value": 1, "n": 4294967296}])))
    add("dataset-chunked-lookups", stored(raw_json=raw(vout=[{"value": 1} for _ in range(205)])) + dataset(claims=[{"subject": f"out:{TX}:{n}"} for n in range(205)]))
    add("dataset-chunk-boundary", stored(raw_json=raw(vout=[{"value": 1} for _ in range(196)])) + dataset(claims=[{"subject": f"out:{TX}:{n}"} for n in range(196)]))
    assert cases[-1]["expected"]["coverage"]["datasets"]["subject_count"] == 200
    assert cases[-1]["expected"]["coverage"]["datasets"]["lookup_queries"] == 1
    sql = stored()
    for ident, visibility in [("a-private", "private"), ("z-public", "public")]:
        sql += dataset(ident=ident, visibility=visibility, claims=[])
        sql += f"WITH RECURSIVE numbers(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM numbers WHERE i<5001) INSERT INTO chain_analysis_dataset_claims(dataset_id,record_number,subject,label,category,source_record_json) SELECT '{ident}',i,'tx:{TX}','Declared entity','service','{{}}' FROM numbers;"
    add("dataset-independent-visibility-limits", sql, summary=True)
    add("dataset-public-independent-limit", sql, summary=True, observer="public")
    rng = random.Random(2106)
    for number in range(12):
        sql = stored(raw_json=raw(confirmations=rng.choice([0, 1, 3])))
        if rng.choice([True, False]):
            sql += cache()
        if rng.choice([True, False]):
            sql += inventory(amount=rng.choice([8000, 10000]))
        sql += dataset(visibility=rng.choice(["public", "private"]))
        sql += update("profiles", {"journal_input_version": rng.choice([7, 8])}, "id='p'")
        add(f"mixed-{number}", sql)
    for case in list(cases):
        if case["name"] in {"seeded-book", "cache-stored-inventory", "assertion-anchored", "dataset-private", "dataset-conflicting-claims", "custody-freshness-1", "authored-label-0"}:
            add("public-" + case["name"], case["sql"], observer="public")
    for observer in ["public", "disclosed", "invalid"]:
        add("dataset-observer-" + observer, stored() + dataset(visibility="private") + dataset(ident="public"), dataset_observer=observer)
    return cases


def main():
    cases = vectors()
    encoded = "[\n" + ",\n".join(json.dumps(case, ensure_ascii=True, separators=(",", ":")) for case in cases) + "\n]\n"
    if "--write" in sys.argv:
        DEST.write_text(encoded)
    elif json.loads(DEST.read_text()) != cases:
        raise SystemExit("analysis source vectors drifted")
    print(f"{len(cases)} analysis source vectors")


if __name__ == "__main__":
    if "--evaluate" in sys.argv:
        print(json.dumps(evaluate(json.load(sys.stdin)), sort_keys=True))
    else:
        main()
