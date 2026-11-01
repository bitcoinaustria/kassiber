"""Full native PSBT baseline, using the unchanged database graph as oracle."""
import argparse
from contextlib import closing
import hashlib
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).parent))
import gen_psbt_inventory_vectors as inventory

OVERLAY = inventory.OVERLAY
DEST = OVERLAY.DEST.with_name("psbt_history.json")


def vectors():
    result = []

    def add(name, sql, psbt=None, args=None, group="history"):
        case = dict(name=name, group=group, sql=sql, psbt=psbt or OVERLAY.proposal(), args=args or {}, native_history=True)
        with closing(OVERLAY.connection(sql)) as conn:
            before = OVERLAY.tables(conn)
            case["expected"] = inventory.evaluate(conn, case)
            if "reference_exception" in case["expected"]:
                return
            case["table_sha256"] = {name: hashlib.sha256(OVERLAY.canonical(rows).encode()).hexdigest() for name, rows in before.items()}
            assert OVERLAY.tables(conn) == before
        result.append(case)

    for case in json.loads(inventory.DEST.read_text()):
        if case["group"] == "native_inventory" and not case["name"].startswith("script-") and case["name"] != "merged-script-refusal-precedence":
            add("inventory/" + case["name"], case["sql"], case["psbt"], case["args"])
    source = OVERLAY.ROOT / "crates/wallets/tests/data/analysis_sources.json"
    for case in json.loads(source.read_text()):
        if case.get("expected", {}).get("error") or ("DROP TABLE" in case["sql"] or "ALTER TABLE" in case["sql"]):
            continue
        add("source/" + case["name"], OVERLAY.inventory_sql() + case["sql"])
    # Complete historical co-spends, script returns, collaborative and stale
    # boundaries; arbitrary private annotations must not become observer edges.
    def q(value):
        return "'" + str(value).replace("'", "''") + "'"
    txid = OVERLAY.TXIDS[6]
    for count in (1, 2, 4, 6):
        raw = dict(txid=txid, version=2, vin=[dict(txid=OVERLAY.TXIDS[n], vout=0,
            prevout=dict(value=100000, scriptpubkey=OVERLAY.SCRIPTS[n])) for n in range(count)],
            vout=[dict(value=90000, scriptpubkey=OVERLAY.SCRIPTS[2]), dict(value=1000, scriptpubkey=OVERLAY.SCRIPTS[0])])
        for boundary in (None, "coinjoin", "payjoin"):
            for state in ("confirmed", "removed"):
                data = dict(raw)
                if state == "removed": data["status"] = dict(removed=True)
                sql = OVERLAY.inventory_sql() + "UPDATE wallet_utxos SET amount=100000000;"
                sql += "UPDATE transactions SET external_id=" + q(txid) + ",raw_json=" + q(json.dumps(data)) + ",privacy_boundary=" + (q(boundary) if boundary else "NULL") + " WHERE id='in';"
                for anchors in ("none", "source", "counterparty", "both", "excluded", "wrong-network", "unreviewed"):
                    extra = ""
                    if anchors in ("counterparty", "both", "excluded", "wrong-network"):
                        extra += "UPDATE transactions SET counterparty='sentinel-private-counterparty' WHERE id='in';"
                    if anchors in ("source", "both", "excluded", "unreviewed"):
                        extra += "INSERT INTO source_funds_sources(id,workspace_id,profile_id,source_type,label,asset,created_at,updated_at) VALUES('source','ws','p','acquisition','sentinel-private-source','BTC','fixed','fixed');"
                        extra += "INSERT INTO source_funds_links(id,workspace_id,profile_id,from_source_id,to_transaction_id,link_type,state,confidence,asset,created_at,updated_at) VALUES('anchor','ws','p','source','in','source'," + q("proposed" if anchors == "unreviewed" else "reviewed") + ",'reviewed','BTC','fixed','fixed');"
                    if anchors == "excluded": extra += "UPDATE transactions SET excluded=1 WHERE id='in';"
                    if anchors == "wrong-network": extra += "UPDATE transactions SET raw_json=" + q(json.dumps(dict(data,network="test"))) + " WHERE id='in';"
                    add(f"spend/{count}/{boundary}/{state}/{anchors}", sql+extra,
                        OVERLAY.proposal(inputs=(0, 1), outputs=(2, 0)))
    # Exercise native bounds with the same public reuse model and exact limit.
    for count in (501, 1001):
        sql = OVERLAY.inventory_sql() + "DELETE FROM wallet_utxos;"
        for n in range(count):
            tx = f"{n+1:064x}"
            sql += f"INSERT INTO wallet_utxos(id,workspace_id,profile_id,wallet_id,chain,network,asset,amount,txid,vout,outpoint,script_pubkey,confirmation_status,first_seen_at,last_seen_at) VALUES('bound-{n}','ws','p','cold','bitcoin','main','BTC',100000,'{tx}',0,'{tx}:0','0014{'01'*20}','confirmed','fixed','fixed');"
        add(f"bound/{count}",sql)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    encoded = "[\n" + ",\n".join(json.dumps(case, ensure_ascii=True, separators=(",", ":")) for case in vectors()) + "\n]\n"
    if args.check:
        assert DEST.read_text() == encoded
    else:
        DEST.write_text(encoded)


if __name__ == "__main__":
    main()
