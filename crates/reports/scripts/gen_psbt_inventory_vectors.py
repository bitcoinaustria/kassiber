"""Unchanged-Python report bytes for the admitted native inventory boundary."""
import argparse
from contextlib import closing
import hashlib
import importlib.util
import json
from pathlib import Path

from kassiber.cli.handlers import _report_hooks
from kassiber.core import reports
from kassiber.errors import AppError

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location("psbt_overlay_vectors", Path(__file__).with_name("gen_psbt_privacy_vectors.py"))
OVERLAY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(OVERLAY)
DEST = ROOT / "crates/reports/tests/data/psbt_inventory.json"
REFUSAL = {"error": {"code": "not_implemented", "message": "PSBT privacy requires a native historical graph for this book",
    "hint": None, "details": {"component": "psbt_privacy_baseline"}, "retryable": False}}


def evaluate(conn, case):
    try:
        payload = reports.report_psbt_privacy(conn, case["args"].get("workspace"), case["args"].get("profile"),
            _report_hooks(), psbt_text=case["psbt"])
        return {"ok": payload, "rows": reports.psbt_privacy_table_rows(payload)}
    except AppError as exc:
        return {"error": dict(code=exc.code, message=str(exc), hint=exc.hint, details=exc.details, retryable=exc.retryable)}
    except RuntimeError as exc:
        return {"error": dict(code="internal_error", message=str(exc), hint=None, details=None, retryable=False)}
    except KeyError as exc:
        return {"reference_exception": {"type": "KeyError", "message": str(exc)}}


def vectors():
    result = []

    def add(name, sql, psbt=None, args=None, refusal=False):
        case = dict(name=name, group="admission_refusal" if refusal else "native_inventory", sql=sql,
            args=args or {}, psbt=psbt or OVERLAY.proposal(), native_inventory=True)
        with closing(OVERLAY.connection(sql)) as conn:
            before = OVERLAY.tables(conn)
            case["table_sha256"] = {name: hashlib.sha256(OVERLAY.canonical(rows).encode()).hexdigest() for name, rows in before.items()}
            case["python_reference"] = evaluate(conn, case)
            case["expected"] = REFUSAL if refusal else case["python_reference"]
            assert OVERLAY.tables(conn) == before, name
        result.append(case)

    for case in OVERLAY.vectors():
        if case["name"].startswith(("conditional-overlay-", "bounded-")):
            continue
        add(case["name"], case["sql"], case["psbt"], case["args"])
    inventory = OVERLAY.inventory_sql()
    for label in ("P2SH-P2WPKH internal", "p2tr external", "p2wpkh change", "p2pkh deposit", "change_receive", "receive change",
                  "changeé", "change²", "change\U0001e4d0", "\x1c INTERNAL \x1f", "\u0130nternal", "", None):
        encoded = "NULL" if label is None else "'" + label.replace("'", "''") + "'"
        add("branch-label-" + repr(label), inventory + f"UPDATE wallet_utxos SET branch_label={encoded};")
    for index in (-1, 0, 1, 2, 3, 4, 5, 6, 7, 8, 2147483647, None):
        add(f"numeric-branch-{index}", inventory + f"UPDATE wallet_utxos SET branch_label=NULL,branch_index={'NULL' if index is None else index};")
    for script in ("", "0014AA", "6a0141", "6a", "aa  bb", "aa bb", "aa\tbb", " aA  bB ", "xyz", "０１", " "):
        add("script-" + repr(script), inventory + "UPDATE wallet_utxos SET script_pubkey='" + script + "';")
    for extra, label in (
        ("UPDATE wallet_utxos SET txid='bad' WHERE id='coin-0';", "invalid-txid"),
        ("UPDATE wallet_utxos SET txid=upper(txid),network='mainnet';", "normalized-domain"),
        ("UPDATE wallet_utxos SET asset='OTHER';", "bitcoin-index-asset"),
        ("UPDATE wallet_utxos SET amount=-1;", "unavailable-amount"),
        ("UPDATE wallet_utxos SET wallet_id='';", "missing-owner"),
        ("UPDATE wallet_utxos SET txid='" + OVERLAY.TXIDS[0] + "',amount=123 WHERE id='coin-1';", "conflicting-amount"),
        ("UPDATE wallet_utxos SET txid='" + "ab" * 32 + "' WHERE id='coin-0'; UPDATE wallet_utxos SET txid='" + "AB" * 32 + "',amount=100000000,script_pubkey='" + OVERLAY.SCRIPTS[0] + "',address='sentinel-private-address-0' WHERE id='coin-2';", "duplicate-normalized-outpoint-same-owner"),
        ("UPDATE wallet_utxos SET script_pubkey='" + OVERLAY.SCRIPTS[0] + "'; UPDATE wallet_utxos SET txid='" + OVERLAY.TXIDS[0] + "' WHERE id='coin-1';", "ambiguous-reuse-intermediate"),
        ("UPDATE wallet_utxos SET vout=-1;", "invalid-vout"),
        ("UPDATE wallet_utxos SET chain='future';", "invalid-domain"),
        ("UPDATE wallet_utxos SET raw_json='{\"chain\":\"liquid\",\"network\":\"liquidv1\"}';", "raw-domain-precedence"),
        ("UPDATE wallet_utxos SET network='future';", "invalid-network"),
        ("UPDATE wallet_utxos SET chain='liquid',network='liquidv1',asset='LBTC',script_pubkey='aa  bb';", "liquid-attribution-script"),
        ("UPDATE wallet_utxos SET script_pubkey='aa  bb'; UPDATE wallet_utxos SET txid='" + OVERLAY.TXIDS[0] + "',script_pubkey='aabb' WHERE id='coin-1'; UPDATE wallet_utxos SET script_pubkey='aabb' WHERE id!='coin-0';", "merged-script-refusal-precedence"),
    ):
        add(label, inventory + extra, psbt=OVERLAY.proposal(outputs=(0,)) if label == "duplicate-normalized-outpoint-same-owner" else None)
    source_rows = [
        ("stored-transaction", "UPDATE transactions SET external_id='" + OVERLAY.TXIDS[0] + "' WHERE id='in';"),
        ("excluded-transaction", "UPDATE transactions SET external_id='" + OVERLAY.TXIDS[0] + "' WHERE id='excluded';"),
        ("raw-transaction", "UPDATE transactions SET raw_json='{\"tx\":{\"txid\":\"" + OVERLAY.TXIDS[0] + "\"}}' WHERE id='in';"),
        ("reference-cache", "INSERT INTO transaction_graph_cache VALUES(1,'bitcoin','main','" + OVERLAY.TXIDS[0] + "','{}','fixed','fixed');"),
        ("acquisition", "INSERT INTO chain_analysis_observations VALUES('p','bitcoin','main','" + OVERLAY.TXIDS[0] + "','{}','{}','sentinel-private-source','fixed');"),
        ("assertion", "INSERT INTO chain_analysis_reference_assertions VALUES('grant','p','domain','bitcoin','main','','" + OVERLAY.TXIDS[0] + "','{}','{}',1,'fixed');"),
        ("inactive-assertion", "INSERT INTO chain_analysis_reference_assertions VALUES('grant','p','domain','bitcoin','main','','" + OVERLAY.TXIDS[0] + "','{}','{}',0,'fixed');"),
        ("dataset", "INSERT INTO chain_analysis_datasets(id,profile_id,dataset_key,version,chain,network,visibility,status,manifest_json,created_at,updated_at) VALUES('pack','p','key','1','bitcoin','main','public','active','{\"valid_from\":null,\"valid_until\":null}','fixed','fixed');"),
        ("corrupt-dataset-manifest", "INSERT INTO chain_analysis_datasets(id,profile_id,dataset_key,version,chain,network,visibility,status,manifest_json,created_at,updated_at) VALUES('pack','p','key','1','bitcoin','main','public','active','{}','fixed','fixed');"),
        ("book-binding", "INSERT INTO book_network_bindings VALUES('p','env','main',1,NULL,'[]','{}','digest','fixed');"),
        ("spent-inventory", "UPDATE wallet_utxos SET spent_by='" + OVERLAY.TXIDS[6] + "';"),
        ("chain-instance", "UPDATE wallet_utxos SET raw_json='{\"chain_instance_id\":\"sentinel-private-instance\"}';"),
        ("wide-vout", "UPDATE wallet_utxos SET vout=4294967296;"),
        ("raw-surrogate", "UPDATE wallet_utxos SET raw_json='{\"chain_instance_id\":\"\\ud800\"}';"),
    ]
    for name, extra in source_rows:
        add("refuse-" + name, inventory + extra, refusal=True)
    history = inventory + source_rows[0][1]
    add("decode-before-source-admission", history, psbt="invalid")
    add("scope-before-source-admission", history, args={"profile": "missing"})
    # A source in another profile must not invalidate the selected snapshot;
    # the unscoped reference cache above deliberately has no such exemption.
    add("other-profile-history", inventory + "UPDATE transactions SET external_id='" + OVERLAY.TXIDS[0] + "',profile_id='other';")
    # Bound work before query/rule truncation could alter a historical delta.
    for count in (400, 401):
        sql = inventory + "DELETE FROM wallet_utxos;"
        for n in range(count):
            txid = f"{n + 1:064x}"
            sql += "INSERT INTO wallet_utxos(id,workspace_id,profile_id,wallet_id,chain,network,asset,amount,txid,vout,outpoint,confirmation_status,first_seen_at,last_seen_at) VALUES(" + f"'bound-{n}','ws','p','cold','bitcoin','main','BTC',100000,'{txid}',0,'{txid}:0','confirmed','fixed','fixed');"
        add(f"row-bound-{count}", sql, refusal=count > 400)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    encoded = json.dumps(vectors(), ensure_ascii=True, indent=2) + "\n"
    if args.check:
        if DEST.read_text() != encoded:
            raise SystemExit("PSBT inventory vectors differ")
    else:
        DEST.write_text(encoded)


if __name__ == "__main__":
    main()
