#!/usr/bin/env python3
"""Freeze unchanged Python occurrence and observer behavior, on disposable data."""
from __future__ import annotations

from dataclasses import fields
import json
from pathlib import Path
import sqlite3
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core.chain_analysis.index import AnalysisIndex, build_index, digest, observer_index, thaw
from kassiber.core.chain_analysis.occurrences import OccurrenceResolver, parse_transaction_identity

DEST = ROOT / "crates/wallets/tests/data/analysis_index.json"
SEED = ROOT / "crates/reports/tests/data/privacy_hygiene_seed.sql"
BOOK = ROOT / "crates/reports/tests/data/book.sql"
# Books built from the frozen seed keep the tables it was cut from.
SEED_TABLE_COUNT = sum(line.startswith("CREATE TABLE ") for line in SEED.read_text().splitlines())
TX = "a" * 64
BLOCK = "b" * 64
OTHER = "c" * 64
DOMAIN = digest(["bitcoin", "main", None])


def database(sql=""):
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA foreign_keys=ON")
    conn.executescript(SEED.read_text())
    conn.executescript(BOOK.read_text())
    conn.executescript(sql)
    return conn


def snapshot(index):
    return {f.name: thaw(getattr(index, f.name)) for f in fields(AnalysisIndex)}


def evaluate(case, conn=None):
    if case["operation"] == "observer":
        index = AnalysisIndex(**case["snapshot"])
        for audience in case["observers"]:
            index = observer_index(index, audience)
        return snapshot(index)
    if case["operation"] == "parse":
        parsed = parse_transaction_identity(case["subject"], chain=case.get("chain"), network=case.get("network"))
        return list(parsed) if parsed is not None else None
    own = conn is None
    conn = database(case.get("sql", "")) if own else conn
    try:
        resolver = OccurrenceResolver(conn, "p")
        result = []
        for request in case["requests"]:
            row = request.get("row", {})
            chain, network = request.get("chain", "bitcoin"), request.get("network", "main")
            source = resolver.source(row, chain, network)
            ident = resolver.transaction_id(chain, network, request.get("txid", TX), source,
                occurrence=request.get("occurrence"), resolve=request.get("resolve", True))
            result.append({"source": {f.name: getattr(source, f.name) for f in fields(source)},
                "accepted": resolver.accepts(row, unscoped=request.get("unscoped", False)),
                "id": ident, "ambiguous": sorted(resolver.ambiguous)})
        return result
    finally:
        if own:
            conn.close()


def assertion(occurrence, *, domain=DOMAIN, txid=TX, profile="p", active=1, status=None, grant="grant"):
    if status is None:
        status = {"block_hash": occurrence.split(":")[0]}
    raw = json.dumps(status) if not isinstance(status, str) else status
    def q(v):
        return "'" + str(v).replace("'", "''") + "'"
    return ("INSERT OR IGNORE INTO chain_analysis_acquisition_grants(id,profile_id,spec_json,binding_json,source_fingerprint,database_identity,status,next_run_at,expires_at,created_at) VALUES("
        + ",".join(q(v) for v in [grant, profile, "{}", "{}", "source", "book", "active", 0, 0, "fixed"]) + ");"
        + "INSERT INTO chain_analysis_reference_assertions(profile_id,grant_id,domain_id,occurrence_id,chain,network,txid,payload_json,status_json,observed_at,active) VALUES("
        + ",".join(q(v) for v in [profile, grant, domain, occurrence, "bitcoin", "main", txid, "{}", raw, "2026-01-01T00:00:00Z", active]) + ");")


def vectors():
    cases = []
    def add(group, name, **kwargs):
        case = {"name": name, "group": group, **kwargs}
        case["expected"] = evaluate(case)
        cases.append(case)
    invalid = ["", f"{BLOCK}:00", f"{BLOCK}:-1", f"{BLOCK}:01", f"{BLOCK.upper()}:0", f"{BLOCK}:0\n", f"{BLOCK}:١", f"{BLOCK}:1:0"]
    for value in [None, f"{BLOCK}:0", f"{BLOCK}:1", f"{BLOCK}:123456789012345678901234567890", *invalid]:
        add("occurrences", f"explicit-{value}", operation="occurrences", requests=[{"occurrence": value}])
    for name, sql in [
        ("none", ""), ("single", assertion(f"{BLOCK}:0")),
        ("duplicates", assertion(f"{BLOCK}:0", grant="one") + assertion(f"{BLOCK}:0", grant="two")),
        ("bip30", assertion(f"{BLOCK}:0") + assertion(f"{OTHER}:9")),
        ("inactive", assertion(f"{BLOCK}:0", active=0)),
        ("other-profile", assertion(f"{BLOCK}:0", profile="other")),
        ("other-domain", assertion(f"{BLOCK}:0", domain="foreign")),
        ("other-txid", assertion(f"{BLOCK}:0", txid=OTHER)),
        ("wrong-anchor", assertion(f"{BLOCK}:0", status={"block_hash": OTHER})),
        ("bad-json", assertion(f"{BLOCK}:0", status="broken")),
        *[(f"invalid-{i}", assertion(v)) for i, v in enumerate(invalid)],
    ]:
        add("occurrences", f"candidates-{name}", operation="occurrences", sql=sql, requests=[{}, {}, {"resolve": False}, {"occurrence": f"{OTHER}:7"}])
    for location in ["row", "raw_json", "wallet_config_json"]:
        for instance in [None, "lab-one", "lab-two", "ä/雪"]:
            row = {"chain_instance_id": instance} if location == "row" else {location: {"chain_instance_id": instance}}
            add("occurrences", f"instance-{location}-{instance}", operation="occurrences", requests=[{"row": row}])
    for row in [
        {"_index_domain_id": "explicit"}, {"domain_id": "ignored-for-source"},
        {"_index_reference": True, "_index_occurrence_id": f"{BLOCK}:0"},
        {"_index_reference": 1, "_index_occurrence_id": f"{BLOCK}:01"},
        {"raw_json": '{"chain_instance_id":"encoded"}'},
        {"raw_json": "broken", "wallet_config_json": {"chain_instance_id": "config"}},
    ]:
        add("occurrences", f"source-{len(cases)}", operation="occurrences", requests=[{"row": row}])
    for chain, network in [("bitcoin", "main"), ("bitcoin", "test"), ("bitcoin", "regtest"), ("liquid", "liquidv1"), ("liquid", "elementsregtest")]:
        add("occurrences", f"domain-{chain}-{network}", operation="occurrences", requests=[{"chain": chain, "network": network}])
    for environment, instance in [("main", None), ("regtest", "lab")]:
        domains = [{"chain": "bitcoin", "network": environment, "domain_id": digest(["bitcoin", environment, instance])}]
        binding = "INSERT INTO book_network_bindings(profile_id,environment_id,environment,revision,chain_instance_id,domains_json,acknowledgements_json,inventory_digest,created_at) VALUES('p','env','%s',1,%s,'%s','[]','fixed','fixed');" % (environment, "NULL" if instance is None else "'lab'", json.dumps(domains))
        requests = []
        for network in ["main", "regtest", "test", None]:
            for inst in [None, "lab", "foreign"]:
                for unscoped in [False, True]:
                    requests.append({"row": {"chain": "bitcoin", "network": network, "chain_instance_id": inst}, "network": network or "main", "unscoped": unscoped})
        for domain in ["foreign", domains[0]["domain_id"]]:
            requests.append({"row": {"chain": "bitcoin", "network": environment, "_index_domain_id": domain, "chain_instance_id": instance}, "network": environment})
        add("occurrences", f"bound-{environment}", operation="occurrences", sql=binding, requests=requests)
    subjects = [TX, TX.upper(), "", f"bitcoin:main:tx:{TX}", f"liquid:liquidv1:tx:{TX}", f"bitcoin:main:domain:d:tx:{TX}",
        f"bitcoin:main:domain:d:occ:{BLOCK}:0:tx:{TX}", f"bitcoin:main:domain:d:occ:{BLOCK}:01:tx:{TX}",
        f"bitcoin:main:domain:d:occ:{BLOCK}:0:out:{TX}:0", f"bitcoin:main:tx:{TX}\n", f"lightning:main:tx:{TX}"]
    for subject in subjects:
        for chain, network in [(None, None), ("bitcoin", "main"), ("liquid", "test"), ("", "main")]:
            add("parse", f"parse-{len(cases)}", operation="parse", subject=subject, chain=chain, network=network)
    for chain, network in [("bitcoin", "main"), ("bitcoin", "regtest"), ("liquid", "liquidv1")]:
        for role in [None, "owned", "external", "fee"]:
            for committed in [False, True]:
                conn = database()
                for number in [1, 2]:
                    output = {"value": 1000 - number, "scriptpubkey": "0014" + "11" * 20, "address": "private-confidential" if chain == "liquid" else "public-address", "asset": "asset"}
                    if role:
                        output["role"] = role
                    if committed:
                        output["valuecommitment"] = "08" + "11" * 32
                    raw = {"txid": f"{number:064x}", "chain": chain, "network": network, "version": 2, "locktime": 0, "vin": [{"coinbase": "01"}] if number == 1 else [{"txid": f"{1:064x}", "vout": 0, "prevout": output}], "vout": [output]}
                    conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,occurred_at,created_at,raw_json) VALUES(?,'ws','p','cold',?,?,?,'inbound',1000000,0,'private-time','fixed',?)", (f"obs{number}", f"fp{number}", raw["txid"], "BTC" if chain == "bitcoin" else "LBTC", json.dumps(raw)))
                owner = snapshot(build_index(conn, "p"))
                conn.close()
                # Overlays exercise public dataset partitions and private labels
                # independently of matching, which is a later port boundary.
                owner["labels"] = [{"id": "private-label", "subject": "secret", "label": "secret", "visibility": "public"}, {"id": "public-label", "dataset_id": "public-pack", "visibility": "public"}, {"id": "private-pack", "dataset_id": "private-pack", "visibility": "private"}]
                owner["coverage"]["datasets"] = {"matched_claims": 8, "visibility_coverage": {"public": {"matched_claims": 2}}}
                visible = [k for k, v in owner["nodes"].items() if v["kind"] == "transaction"]
                hidden = next(k for k, v in owner["nodes"].items() if v["kind"] == "record")
                owner["edges"]["custody-private"] = {"id": "custody-private", "kind": "custody", "source": visible[0], "target": hidden, "evidence": []}
                owner["edges"]["custody-visible-ends"] = {"id": "custody-visible-ends", "kind": "custody", "source": visible[0], "target": visible[1], "evidence": []}
                owner["nodes"]["private-protocol"] = {"id": "private-protocol", "kind": "transaction", "chain": "lightning", "network": "main", "wallet_ids": ["cold"], "evidence": []}
                owner["profile_seeds"].append("private-protocol")
                owner["findings"].extend([
                    {"id": "mixed", "node_ids": [hidden, *visible], "edge_ids": ["custody-private", *list(owner["edges"])[:1]], "evidence": [{"source": "stored_transaction", "reference": "private-record", "level": "observed", "source_name": "private-endpoint"}]},
                    {"id": "global", "node_ids": [], "evidence": []},
                    {"id": "hidden", "node_ids": [hidden], "evidence": []},
                ])
                for ident, fact in owner["transaction_facts"].items():
                    fact["collaboration"] = {"kind": "payjoin", "source": "private_review", "evidence_level": "reviewed"} if ident == visible[0] else {"kind": "coinjoin", "source": "equal_output_coinjoin", "evidence_level": "heuristic"}
                    if chain == "liquid":
                        fact["fee_msat"] = "999"
                for fact in owner["output_facts"].values():
                    fact.update(ownership_known=True, branch_role="change", private_annotation="private")
                for observers in [["owner"], ["public"], ["public", "owner"], ["public", "public"]]:
                    add("observer", f"observer-{chain}-{network}-{role}-{committed}-{'-'.join(observers)}", operation="observer", snapshot=owner, observers=observers)
    return cases


def main():
    cases = vectors()
    encoded = "[\n" + ",\n".join(json.dumps(case, ensure_ascii=True, separators=(",", ":")) for case in cases) + "\n]\n"
    if "--write" in sys.argv:
        DEST.write_text(encoded)
    elif json.loads(DEST.read_text()) != cases:
        raise SystemExit("analysis index vectors drifted")
    print(f"{len(cases)} analysis index vectors")


if __name__ == "__main__":
    main()
