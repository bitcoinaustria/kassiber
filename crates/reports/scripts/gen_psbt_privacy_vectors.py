import argparse
import base64
from contextlib import closing
from dataclasses import asdict, replace
import hashlib
import importlib.util
import json
from pathlib import Path
import random
import sqlite3
from unittest.mock import patch

from embit.script import Script
from embit.transaction import Transaction, TransactionInput, TransactionOutput

from kassiber.cli.handlers import _report_hooks
from kassiber.core import privacy_linkage as linkage, reports
from kassiber.errors import AppError

ROOT = Path(__file__).resolve().parents[3]
DEST = ROOT / "crates/reports/tests/data/psbt_privacy.json"
SEED = ROOT / "crates/reports/tests/data/privacy_hygiene_seed.sql"
# Books built from the frozen seed keep the tables it was cut from.
SEED_TABLE_COUNT = sum(line.startswith("CREATE TABLE ") for line in SEED.read_text().splitlines())
BOOK = SEED.with_name("book.sql")
SPEC = importlib.util.spec_from_file_location("psbt_vectors", ROOT / "crates/wallets/scripts/generate_psbt_vectors.py")
PSBT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PSBT)
TXIDS = [f"{n:02x}" * 32 for n in range(1, 8)]
SCRIPTS = ["0014" + f"{n:02x}" * 20 for n in range(1, 8)]


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True)


def tables(conn):
    names = [r[0] for r in conn.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
    assert len(names) == SEED_TABLE_COUNT
    return {name: sorted([dict(r) for r in conn.execute(f'SELECT * FROM "{name}"')], key=canonical) for name in names}


def connection(sql=""):
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    conn.executescript(SEED.read_text() + BOOK.read_text() + sql)
    return conn


def inventory_sql():
    rows = []
    for n in range(6):
        values = dict(id=f"coin-{n}", workspace_id="ws", profile_id="p", wallet_id="cold" if n % 2 == 0 else "hot",
            chain="bitcoin", network="main", asset="BTC", amount=100_000_000 + n,
            txid=TXIDS[n], vout=0, outpoint=TXIDS[n] + ":0", confirmation_status="confirmed", confirmations=5,
            script_pubkey=SCRIPTS[n], address=f"sentinel-private-address-{n}",
            branch_label="receive" if n % 3 == 0 else "change" if n % 3 == 1 else None,
            branch_index=n % 2, address_index=12345 + n, first_seen_at="fixed", last_seen_at="fixed",
            raw_json='{"secret":"sentinel-private-raw"}')
        encoded = ["NULL" if v is None else str(v) if isinstance(v, int) else "'" + v.replace("'", "''") + "'" for v in values.values()]
        rows.append(f"INSERT INTO wallet_utxos({','.join(values)}) VALUES({','.join(encoded)});")
    return "\n".join(rows)


def proposal(inputs=(0, 1), outputs=(2,), *, sequence=0xfffffffd, signed=False, amount=50000, v2=False):
    if v2:
        g = [(b"\xfb", (2).to_bytes(4, "little")), (b"\2", (2).to_bytes(4, "little")),
             (b"\4", PSBT.compact(len(inputs))), (b"\5", PSBT.compact(len(outputs)))]
        ins = [[(b"\x0e", bytes.fromhex(TXIDS[n])[::-1]), (b"\x0f", bytes(4)), (b"\x10", sequence.to_bytes(4,"little"))] for n in inputs]
        outs = [[(b"\3", amount.to_bytes(8,"little")), (b"\4", bytes.fromhex(SCRIPTS[n]))] for n in outputs]
    else:
        tx = Transaction(vin=[TransactionInput(bytes.fromhex(TXIDS[n]), 0, sequence=sequence) for n in inputs],
            vout=[TransactionOutput(amount, Script(bytes.fromhex(SCRIPTS[n]))) for n in outputs])
        g, ins, outs = [(b"\0",tx.serialize())], [[] for _ in inputs], [[] for _ in outputs]
    if signed and ins:
        ins[0].append((b"\7", b"sentinel-private-final-script"))
    g.append((b"\xfcsentinel-private-key",b"sentinel-private-value"))
    return base64.b64encode(PSBT.encode(g, ins, outs)).decode()


def evaluate(conn, case, baseline):
    try:
        with patch.object(linkage, "build_privacy_linkage_graph", return_value=baseline):
            payload = reports.report_psbt_privacy(conn, case["args"].get("workspace"), case["args"].get("profile"), _report_hooks(), psbt_text=case["psbt"])
        return {"ok": payload, "rows": reports.psbt_privacy_table_rows(payload)}
    except AppError as exc:
        return {"error": dict(code=exc.code, message=str(exc), hint=exc.hint, details=exc.details, retryable=exc.retryable)}
    except RuntimeError as exc:
        return {"error": dict(code="internal_error", message=str(exc), hint=None, details=None, retryable=False)}


def baseline_input(graph):
    node_fields = {"node_id", "txid", "vout", "wallet_id", "amount_msat", "script_key", "branch_role", "branch_evidence_level", "change_evidence"}
    return dict(
        nodes={key:{k:v for k,v in asdict(node).items() if k in node_fields} for key,node in graph.nodes.items()},
        edges=[dict(from_node_id=edge.from_node_id,to_node_id=edge.to_node_id,observer_linkage=edge.observer_linkage) for edge in graph.edges],
        adversary_views=[dict(tier=view.tier,label=view.label,clusters=[dict(cluster_id=c.cluster_id) for c in view.clusters],
            summary=dict(exposed_cluster_count=view.summary.get("exposed_cluster_count",0)),model_assumptions=[{} for _ in view.model_assumptions]) for view in graph.adversary_views],
        limitations=[dict(code=row["code"],evidence=row.get("evidence",{})) for row in graph.limitations],
    )


def vectors():
    result = []
    sql = inventory_sql()

    def add(name, group="inventory", *, extra="", text=None, baseline=None, args=None):
        with closing(connection(sql + extra)) as conn:
            args = args or {}
            profile = "other" if args.get("profile") == "other" else "p"
            graph = baseline or linkage.build_privacy_linkage_graph(conn, profile)
            case = dict(name=name, group=group, sql=sql + extra, psbt=text or proposal(), args=args,
                baseline_profile=profile, baseline=baseline_input(graph))
            before = tables(conn)
            case["table_sha256"] = {name:hashlib.sha256(canonical(rows).encode()).hexdigest() for name,rows in before.items()}
            case["expected"] = evaluate(conn, case, graph)
            assert tables(conn) == before
            result.append(case)

    add("unlinked-inputs")
    for inputs in ((), (0,), (6,), (0, 6), (0, 1, 2), (6, 5, 4, 3, 2, 1, 0)):
        for outputs in ((), (0,), (1,), (2,), (6,), (0, 1, 2, 3, 4, 5, 6), (0, 0)):
            add(f"inventory-{inputs}-{outputs}", text=proposal(inputs, outputs))
    for label, extra in (
        ("empty", "DELETE FROM wallet_utxos;"),
        ("other-profile", "UPDATE wallet_utxos SET profile_id='other',wallet_id='other';"),
        ("mixed-networks", "UPDATE wallet_utxos SET network='test' WHERE id='coin-1';"),
        ("non-bitcoin", "UPDATE wallet_utxos SET chain='liquid',asset='LBTC';"),
        ("no-script", "UPDATE wallet_utxos SET script_pubkey=NULL;"),
        ("unknown-role", "UPDATE wallet_utxos SET branch_label=NULL,branch_index=NULL;"),
        ("same-script", "UPDATE wallet_utxos SET script_pubkey='" + SCRIPTS[0] + "';"),
        ("role-conflict", "UPDATE wallet_utxos SET script_pubkey='" + SCRIPTS[0] + "',branch_label=NULL,branch_index=NULL WHERE id='coin-1';"),
        ("ambiguous-owner", "UPDATE wallet_utxos SET txid='" + TXIDS[0] + "',outpoint='" + TXIDS[0] + ":0' WHERE id='coin-1';"),
        ("subsat", "UPDATE wallet_utxos SET amount=49999999;"),
        ("zero", "UPDATE wallet_utxos SET amount=0;"),
        ("large", "UPDATE wallet_utxos SET amount=9223372036854775807;"),
    ):
        add("admission-" + label, extra=extra, text=proposal((0,), (0,)))
    for value in (0, 1, 99999, 100000, 100001, 2100000000000000):
        add(f"fee-{value}", text=proposal((0,), (6,), amount=value))
    for sequence in (0, 1, 0xfffffffd, 0xfffffffe, 0xffffffff):
        for version in (False, True):
            add(f"sequence-{sequence}-v2-{version}", "features", text=proposal(sequence=sequence, v2=version))
    add("signed", "features", text=proposal(signed=True))
    add("op-return", "features", text=base64.b64encode(PSBT.encode([(b"\0", Transaction(vin=[TransactionInput(bytes.fromhex(TXIDS[0]),0)],vout=[TransactionOutput(0,Script(b"\x6a\x01\x41"))]).serialize())],[[]],[[]])).decode())
    with closing(connection(sql)) as conn:
        graph = linkage.build_privacy_linkage_graph(conn, "p")
    for stopped in ((), ("node_limit",), ("hypothesis_budget",)):
        for exhausted in (False, True):
            bounded = replace(graph,limitations=graph.limitations + ({"code":"shared_analysis_bounded","evidence":{"stopped_reasons":stopped,"budget_exhausted":exhausted}},))
            add(f"bounded-{stopped}-{exhausted}","linkage",baseline=bounded)
    rng = random.Random(21)
    nodes = list(graph.nodes.values())
    assert len(nodes) == 6
    for n in range(80):
        owned = {node.node_id:replace(node, branch_role=rng.choice(("receive","change","unknown")),
            branch_evidence_level=rng.choice(("exact","derived","unknown")),
            change_evidence=rng.choice(("ground_truth","imported","heuristic","unavailable")),
            script_key=rng.choice(SCRIPTS[:4])) for node in nodes}
        edges = tuple(linkage.PrivacyLinkageEdge(str(i),"common_input","common_input",nodes[a].node_id,nodes[b].node_id,"derived","shared_chain_analysis",None,0,False,0,{},rng.choice((False,True))) for i,(a,b) in enumerate((rng.sample(range(6),2) for _ in range(rng.randrange(7)))))
        provisional = replace(graph,nodes=owned,edges=edges)
        components,_ = linkage._build_inference_components(owned,edges)
        views = tuple(replace(view,clusters=tuple(linkage.AdversaryInferenceCluster(component.component_id,component.node_ids,component.wallet_ids,component.edge_ids,(),(),"derived","supported_by_local_ground_truth","fixture") for component in components if rng.choice((False,True)))) for view in graph.adversary_views)
        add(f"conditional-overlay-{n}","linkage",baseline=replace(provisional,adversary_views=views), text=proposal(tuple(rng.sample(range(7),rng.randrange(1,8))),tuple(rng.choices(range(7),k=rng.randrange(1,5)))))
    for args in ({"workspace":"Main","profile":"Book"},{"profile":"other"},{"workspace":"missing"},{"profile":"missing"}):
        add("scope-" + canonical(args),"scope",args=args)
    add("scope-ambiguous", "scope", args={"profile":"Book"}, extra="INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at) VALUES('duplicate','ws','book','EUR','generic','fixed');")
    add("scope-before-decode", "scope", args={"profile":"missing"}, text="invalid")
    for text in ("invalid", "", "cHNidP8=", "70736274ff", base64.b64encode(PSBT.encode([(b"\0",b"")],[],[])).decode()):
        add("decode-" + repr(text),"errors",text=text if text else " ")
    g,i,o=PSBT.sample()
    i[0] = [(b"\0",b"")]
    add("embedded-reference-bug", "errors",text=base64.b64encode(PSBT.encode(g,i,o)).decode())
    return json.loads(json.dumps(result))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    data = vectors()
    if args.check:
        assert json.loads(DEST.read_text()) == data
    else:
        DEST.write_text(json.dumps(data,indent=2,sort_keys=True) + "\n")
    print(f"{len(data)} PSBT overlay vectors")


if __name__ == "__main__":
    main()
