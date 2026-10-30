#!/usr/bin/env python3
"""Unchanged Python authoritative record-insertion oracle (one writer, not full sync)."""
import argparse
from decimal import Decimal
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[3]))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from kassiber.core import imports, repo
from kassiber.core.chain_observer import provenance
from kassiber.core.sync_backends import record_from_bitcoin_esplora_tx
from publication_books import deterministic, dump, error, new_book, snapshot

DEST = Path(__file__).resolve().parents[1] / "tests/vectors/python_insert.json"
OWN = "0014" + "11" * 20
CHANGE = "0014" + "22" * 20
FOREIGN = "0014" + "33" * 20
OTHER_FOREIGN = "0014" + "44" * 20
SEED = """
INSERT INTO workspaces(id,label,created_at) VALUES('w','synthetic','fixed');
INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at,last_processed_at,last_processed_tx_count,journal_input_version,last_processed_input_version)
VALUES('p','w','primary','EUR','fixed','processed',3,4,4),('q','w','other','USD','fixed',NULL,0,0,0);
INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at)
VALUES('a','w','p','Wällét','descriptor','{"chain":"bitcoin","network":"main","descriptor":"synthetic"}','fixed'),
('b','w','p','second','descriptor','{}','fixed'),('c','w','q','foreign','descriptor','{}','fixed');
"""


NETWORK = {"chain": "bitcoin", "network": "mainnet"}


def txid(n):
    return f"{n:02x}" * 32


def tx(n, *, receive=0, spend=0, change=0, confirmed=True, height=5, block_hash=None, time=1_700_000_000,
       observed=1_699_999_000, extra=None):
    vin = [{"txid": txid(200 + n), "vout": 0, "prevout": {"scriptpubkey": OWN if spend else FOREIGN,
            "value": spend or 50_000}, "witness": ["ab" * 70, "cd" * 33]}]
    vout = [{"scriptpubkey": OWN if receive else FOREIGN, "value": receive or 40_000}]
    if change:
        vout.append({"scriptpubkey": CHANGE, "value": change})
    status = ({"confirmed": True, "block_height": height, "block_hash": block_hash or txid(100 + height),
               "block_time": time} if confirmed else {"confirmed": False})
    payload = {"txid": txid(n), "vin": vin, "vout": vout, "status": status, "observed_at": observed,
               "observer": "bdk", "fee": 1_000 if spend else 0}
    payload.update(extra or {})
    return payload


def record(n, **options):
    raw = record_from_bitcoin_esplora_tx(tx(n, **options), {OWN: {}, CHANGE: {}}, "fixture")
    raw["amount"] = str(raw["amount"])
    raw["fee"] = str(raw["fee"])
    return raw


def insert(conn, records, wallet="a"):
    profile = conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone()
    row = conn.execute("SELECT * FROM wallets WHERE id=?", (wallet,)).fetchone()
    if wallet == "missing":
        row = {"id": "missing", "label": "missing"}
    return imports.insert_wallet_records(conn, profile, row, records, "backend:fixture",
        imports.ImportCoordinatorHooks(lambda *args: None, repo.invalidate_journals),
        commit=False, authoritative_chain_observer=True)


def observe(conn, records, wallet="a"):
    """Prior authoritative state: insertion plus closed provenance."""
    outcome = insert(conn, records, wallet)
    entries = provenance.provenance_entries_for_facts(
        ((type("Identity", (), {"id": "observer-1", "observer_kind": "bdk"})(), records),), records)
    provenance.persist_chain_observation_provenance(
        conn, conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone(),
        conn.execute("SELECT * FROM wallets WHERE id=?", (wallet,)).fetchone(),
        application_revision="rev", chain="bitcoin", network="main", entries=entries,
        resolved_records=outcome["_observer_resolved_records"])


def sql(conn, text):
    conn.executescript(text)


def scenario(name, steps, setup=()):
    return {"name": name, "setup": list(setup), "steps": steps}


def scenarios():
    receive = record(1, receive=90_000)
    send = record(2, spend=100_000, change=9_000)
    mempool = record(3, receive=7_000, confirmed=False)
    csv_raw = json.dumps({"TxHash": txid(1).upper(), "source": "csv", "privacy_boundary": "coinjoin"})
    upgraded_raw = json.loads(receive["raw_json"])
    upgraded_raw.pop("observer_owned_scripts")
    superset_raw = json.loads(receive["raw_json"])
    superset_raw["observer_owned_scripts"] = sorted([OWN, CHANGE])
    versioned = json.loads(receive["raw_json"])
    versioned["ownership_graph_version"] = 1
    enriched = dict(receive, raw_json=json.dumps({**versioned, "ownership_graph_version": 1,
        "vout": [*versioned["vout"], {"scriptpubkey": OTHER_FOREIGN, "value": 1, "role": "external"}]}, sort_keys=True))
    hashed = dict(record(4, receive=10_000), payment_hash="AA" * 32, payment_hash_source="chain_script_unique_outpoint",
                  swap_refund_funding_txid=" " + txid(9).upper(), swap_refund_funding_vout="3",
                  counterparty="Shop", description=None)
    return [
        scenario("canonical-upper-hex", [[record(11, receive=90_000)]],
                 [("observe", [record(11, receive=90_000)]),
                  ("sql", "UPDATE transactions SET external_id=UPPER(external_id)")]),
        scenario("fresh-mixed", [[receive, send, mempool, record(5, receive=1, extra={"islikelycoinjoin": True}),
                                  dict(record(6, receive=2), kind=None, description="custom")]]),
        scenario("reobserve-unchanged", [[receive, send]], [("observe", [receive, send])]),
        scenario("confirm-later", [[record(3, receive=7_000, time=1_700_000_500)]], [("observe", [mempool])]),
        scenario("reorg-demote", [[record(1, receive=90_000, confirmed=False)]], [("observe", [receive])]),
        scenario("block-refresh-same-time", [[record(1, receive=90_000, block_hash=txid(77))]], [("observe", [receive])]),
        scenario("block-refresh-identity", [[record(1, receive=90_000, block_hash=txid(77), extra=NETWORK)]],
                 [("observe", [record(1, receive=90_000, extra=NETWORK)])]),
        scenario("block-refresh-other-instance", [[record(1, receive=90_000, block_hash=txid(77), extra={**NETWORK, "chain_instance_id": "x"})]],
                 [("observe", [record(1, receive=90_000, extra=NETWORK)])]),
        scenario("height-and-time-change", [[record(1, receive=90_000, height=6, time=1_700_000_900)]],
                 [("observe", [receive])]),
        scenario("rates-cache-price-cleared", [[record(3, receive=7_000)]], [("observe", [mempool]),
                 ("sql", "UPDATE transactions SET fiat_rate=30000.5,fiat_value=2.1,fiat_rate_exact='30000.5',fiat_value_exact='2.1',fiat_price_source='rates_cache',pricing_source_kind='fmv_provider',pricing_quality='exact'")]),
        scenario("manual-price-kept", [[record(3, receive=7_000)]], [("observe", [mempool]),
                 ("sql", "UPDATE transactions SET fiat_rate=30000.5,fiat_price_source='manual',pricing_source_kind='manual_override'")]),
        scenario("amount-change-recomputes", [[record(1, receive=95_000)]], [("observe", [receive]),
                 ("sql", "UPDATE transactions SET fiat_rate=30000.25,fiat_value=27.0,fiat_rate_exact=NULL,fiat_value_exact='27'")]),
        scenario("amount-change-clears-value", [[record(1, receive=95_000)]], [("observe", [receive]),
                 ("sql", "UPDATE transactions SET fiat_value=27.0,fiat_value_exact='27'")]),
        scenario("direction-reinterpretation", [[record(1, spend=100_000, receive=90_000)]], [("observe", [receive])]),
        scenario("supersede-obsolete", [[record(1, spend=100_000, receive=90_000)]], [("observe", [receive]),
                 ("sql", "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES('legacy-out','w','p','a','" + txid(1) + "','fp-legacy','2023-11-14T22:13:20Z','outbound','BTC',10000000,0,'0000')")]),
        scenario("supersede-authored-refused", [[record(1, spend=100_000, receive=90_000)]], [("observe", [receive]),
                 ("sql", "UPDATE transactions SET note='mine'; INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES('legacy-out','w','p','a','" + txid(1) + "','fp-legacy','2023-11-14T22:13:20Z','outbound','BTC',10000000,0,'0000')")]),
        scenario("supersede-tagged-refused", [[record(1, spend=100_000, receive=90_000)]], [("observe", [receive]),
                 ("sql", "INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES('tag','w','p','t','t','fixed'); INSERT INTO transaction_tags(transaction_id,tag_id) SELECT id,'tag' FROM transactions; INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES('legacy-out','w','p','a','" + txid(1) + "','fp-legacy','2023-11-14T22:13:20Z','outbound','BTC',10000000,0,'0000')")]),
        scenario("kind-override-contradicts", [[record(1, spend=100_000, receive=90_000)]], [("observe", [receive]),
                 ("sql", "UPDATE transactions SET kind_override='deposit'")]),
        scenario("kind-override-agrees", [[record(1, spend=100_000, receive=90_000)]], [("observe", [receive]),
                 ("sql", "UPDATE transactions SET kind_override='spend'")]),
        scenario("excluded-direction-owns", [[record(1, spend=100_000, receive=90_000)]], [("observe", [receive]),
                 ("sql", "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,excluded,created_at) VALUES('excluded-out','w','p','a','" + txid(1) + "','fp-x','2023-11-14T22:13:20Z','outbound','BTC',1,0,1,'0000')")]),
        scenario("multiple-active", [[receive]], [("observe", [receive]),
                 ("sql", "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES('twin','w','p','a','" + txid(1).upper() + "','fp-twin','2023-11-14T22:13:20Z','inbound','BTC',1,0,'0000')")]),
        scenario("multiple-excluded", [[receive]], [
                 ("sql", "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,excluded,created_at) VALUES('x1','w','p','a','" + txid(1) + "','fp-x1','2023-11-14T22:13:20Z','inbound','BTC',1,0,1,'0000'),('x2','w','p','a','" + txid(1) + "','fp-x2','2023-11-14T22:13:20Z','inbound','BTC',1,0,1,'0001')")]),
        scenario("single-excluded-kept-excluded", [[receive]], [
                 ("sql", "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,excluded,created_at) VALUES('x1','w','p','a','" + txid(1) + "','fp-x1','2023-11-14T22:13:20Z','inbound','BTC',1,0,1,'0000')")]),
        scenario("csv-row-promoted", [[receive]], [
                 ("sql", "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,description,kind,privacy_boundary,raw_json,fiat_rate,fiat_value,fiat_price_source,pricing_source_kind,created_at) VALUES('csv','w','p','a','" + txid(1).upper() + "','fp-csv','2023-11-14T22:13:20Z','inbound','BTC',89000000,0,'Imported','buy','coinjoin','" + csv_raw + "',30000,26.7,'import','generic_import','0000')")]),
        scenario("unknown-occurrence", [[receive]], [
                 ("sql", "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,external_id_kind,fingerprint,occurred_at,confirmed_at,direction,asset,amount,fee,created_at) VALUES('u','w','p','a','" + txid(1) + "','txid','fp-u','1970-01-01T00:00:00Z','2023-11-14T22:13:20Z','inbound','BTC',90000000,0,'0000')")]),
        scenario("attestation-upgrade", [[receive]], [("observe", [dict(receive, raw_json=json.dumps(upgraded_raw, sort_keys=True))])]),
        scenario("attestation-rescope", [[receive]], [("observe", [dict(receive, raw_json=json.dumps(superset_raw, sort_keys=True))])]),
        scenario("graph-version-upgrade", [[dict(receive, raw_json=json.dumps(versioned, sort_keys=True))]], [("observe", [receive])]),
        scenario("graph-strict-enrichment", [[enriched]], [("observe", [dict(receive, raw_json=json.dumps(versioned, sort_keys=True))])]),
        scenario("hash-and-refund-fill", [[hashed]], [("observe", [dict(hashed, payment_hash=None, payment_hash_source=None, swap_refund_funding_txid=None, counterparty=None, description=None)])]),
        scenario("hash-source-promotion", [[hashed]], [("observe", [dict(hashed, payment_hash_source="chain_script")])]),
        scenario("refund-vout-fill", [[hashed]], [("observe", [dict(hashed, swap_refund_funding_vout=None)])]),
        scenario("privacy-import-authored", [[record(1, receive=90_000)]], [
                 ("sql", "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,privacy_boundary,raw_json,created_at) VALUES('pb','w','p','a','" + txid(1) + "','fp-pb','2023-11-14T22:13:20Z','inbound','BTC',90000000,0,'payjoin','{\"source\":\"x\",\"privacy_hop\":\"payjoin\"}','0000')")]),
        scenario("non-canonical-txid", [[dict(receive, txid="provider-id")], [dict(receive, txid="provider-id")]]),
        scenario("opposite-directions", [[receive, record(1, spend=100_000, receive=90_000)]]),
        scenario("network-binding-refused", [[receive]], [
                 ("sql", "INSERT INTO book_network_bindings(profile_id,environment_id,environment,chain_instance_id,domains_json,acknowledgements_json,revision,inventory_digest,created_at) VALUES('p','env','regtest','00000000-0000-4000-8000-000000000000','[]','[]',1,'digest','fixed')")]),
        scenario("foreign-wallet", [[receive]], [], ) | {"wallet": "c"},
        scenario("other-wallet-twin-untouched", [[receive]], [("observe_b", [receive])]),
        scenario("invalid-privacy-boundary", [[dict(receive, privacy_boundary="bogus")]]),
        scenario("unsupported-direction", [[dict(receive, direction="sideways")]]),
        scenario("bad-timestamp", [[dict(receive, occurred_at="yesterday")]]),
        scenario("negative-amount", [[dict(receive, amount="-0.00090000", direction=None)]]),
        scenario("empty", [[]]),
    ]


def run(case):
    conn = new_book()
    conn.executescript(SEED)
    with deterministic(imports, provenance, prefix="setup"):
        for kind, payload in case["setup"]:
            if kind == "sql":
                sql(conn, payload)
            else:
                observe(conn, payload, "b" if kind == "observe_b" else "a")
    conn.commit()
    steps = []
    with deterministic(imports, provenance):
        for records in case["steps"]:
            before = dump(conn)
            conn.execute("SAVEPOINT step")
            try:
                outcome = {"result": insert(conn, records, case.get("wallet", "a"))}
                conn.execute("RELEASE step")
            except Exception as exc:  # The vector records Python's refusal.
                conn.execute("ROLLBACK TO step")
                conn.execute("RELEASE step")
                outcome = {"error": error(exc)}
            steps.append({"records": records, "before": before, "outcome": outcome, "after": snapshot(conn)})
    return {"name": case["name"], "wallet": case.get("wallet", "a"), "steps": steps}


def vectors():
    return {"generator": "crates/wallets/scripts/generate_insert_vectors.py",
            "cases": [run(case) for case in scenarios()]}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    text = json.dumps(vectors(), indent=1, sort_keys=True, ensure_ascii=False, default=str) + "\n"
    if args.check:
        if DEST.read_text(encoding="utf-8") != text:
            raise SystemExit(f"{DEST} is stale")
        return
    DEST.write_text(text, encoding="utf-8")


if __name__ == "__main__":
    main()
