from __future__ import annotations

import copy
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).parent))
from generate_analysis_index import database, snapshot
from kassiber.core.chain_analysis.index import AnalysisIndex, build_index
from kassiber.core.chain_analysis.query import normalize_query, query_index, resolve_subject
from kassiber.errors import AppError

DEST = ROOT / "crates/wallets/tests/data/analysis_traversal.json"


def evaluate(case, snapshots):
    try:
        if case["operation"] == "normalize":
            return normalize_query(case["args"])
        index = AnalysisIndex(**snapshots[case["snapshot"]])
        if case["operation"] == "resolve":
            return list(resolve_subject(index, case["subject"], normalize_query(case.get("args"))))
        return query_index(index, case.get("args"))
    except AppError as exc:
        return {"error": {"code": exc.code, "message": str(exc), "details": exc.details, "retryable": exc.retryable}}


def captured(large=False):
    conn = database()
    conn.execute("DELETE FROM transactions")
    script = "0014" + "11" * 20
    graph = {1: [], 2: [(1, 0)], 3: [(1, 1)], 4: [(2, 0), (3, 0)], 5: [(4, 0)], 6: [(4, 1)], 7: [(5, 0), (6, 0)]}
    if large:
        graph = {n: [(n - 1, 0)] if n > 1 else [] for n in range(1, 81)}
    for number, parents in graph.items():
        raw = {"txid": f"{number:064x}", "chain": "bitcoin", "network": "main", "version": 2, "locktime": 0,
               "vin": [{"txid": f"{p:064x}", "vout": v, "prevout": {"value": 5000 if v == 0 else 4990, "scriptpubkey": script if v == 0 else "0014" + "22" * 20}} for p, v in parents] or [{"coinbase": "01"}],
               "vout": [{"value": 5000, "scriptpubkey": script}, {"value": 4990, "scriptpubkey": "0014" + "22" * 20}]}
        conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,occurred_at,created_at,raw_json) VALUES(?,'ws','p','cold',?,?,'BTC','inbound',10000000,10000,?,'fixed',?)", (f"obs{number}", f"fp{number}", raw["txid"], f"2026-01-{min(number, 28):02d}T00:00:00Z", json.dumps(raw)))
    result = snapshot(build_index(conn, "p"))
    conn.close()
    return result


def vectors():
    snapshots = {"diamond": captured()}
    cases = []
    def add(name, operation="traversal", snapshot="diamond", **kw):
        case = {"name": name, "operation": operation, **kw}
        if operation != "normalize":
            case["snapshot"] = snapshot
        case["expected"] = evaluate(case, snapshots)
        cases.append(case)
    tx = "bitcoin:main:tx:" + f"{1:064x}"
    last = "bitcoin:main:tx:" + f"{7:064x}"
    for mode in ["overview", "trace", "path"]:
        for direction in ["forward", "backward", "both"]:
            for observer in ["owner", "public", "disclosed"]:
                for depth in [1, 4, 12]:
                    args = {"mode": mode, "direction": direction, "observer": observer, "depth": depth}
                    if mode != "overview":
                        args["subject"] = tx
                    if mode == "path":
                        args["target"] = last
                    add(f"{mode}-{direction}-{observer}-{depth}", args=args)
    for subject in [tx, tx.upper(), f"{1:064x}", "tx:" + f"{1:064x}", "out:" + f"{1:064x}:0", "cold", "missing", " obs1 "]:
        add(f"resolve-{subject}", operation="resolve", subject=subject)
    for args in [None, {}, [], "bad", {"x": 1, "z": 2}, *[{k: v} for k, values in {
        "mode": [None, "invalid", "trace", "path"], "direction": [True, "invalid"], "observer": [0, "invalid"],
        "chain": ["lightning", None], "network": ["liquidv1", None], "depth": [True, "4", 0, 51, 1, 50],
        "node_limit": [24, 25, 2000, 2001, 25.0], "edge_limit": [49, 50, 6000, 6001],
        "include_relations": [0, None, "false"], "include_hypotheses": [1, []],
        "subject": ["", "  ", 4, "x" * 1025, "  α  "], "target": [False, " x "],
        "min_amount_msat": [True, 0, "0", " 123 ", "01", "+1", "-1", "1.0", "9" * 200, "9" * 4301],
        "start": ["2026-01-01", "2026-01-01T00:00:00", "2026-02-30T00:00:00Z", "2026-01-01T00:00:00+24:00", "2026-01-01T00:00:00.123456789Z"],
        "end": [None, "2026-01-01T00:00:00+02:00"],
    }.items() for v in values], {"start": "2026-01-02T00:00:00Z", "end": "2026-01-01T00:00:00Z"}]:
        add(f"normalize-{len(cases)}", operation="normalize", args=args)
    for changes in [
        {"node_limit": 25}, {"edge_limit": 50}, {"depth": 1}, {"chain": "liquid"}, {"network": "regtest"},
        {"min_amount_msat": "5000001"}, {"min_amount_msat": "99999999999999999999999999999999"},
        {"start": "2026-01-04T00:00:00Z"}, {"end": "2026-01-02T02:00:00+02:00"},
        {"subject": tx, "target": tx, "mode": "path"}, {"subject": last, "target": tx, "mode": "path", "direction": "forward"},
    ]:
        add(f"filters-{len(cases)}", args={"depth": 50, **changes})
    incomplete = copy.deepcopy(snapshots["diamond"])
    incomplete["transaction_facts"][tx]["complete"] = False
    snapshots["incomplete"] = incomplete
    add("incomplete-transaction", snapshot="incomplete", args={"mode":"trace","subject":tx,"direction":"forward"})
    for status in ["missing", "stale", "conflicting"]:
        changed = copy.deepcopy(snapshots["diamond"])
        changed["nodes"][tx]["status"] = status
        snapshots[status] = changed
        for mode in ["trace", "path"]:
            add(f"{status}-{mode}", snapshot=status, args={"mode": mode, "subject": tx, "target": last, "depth": 50})
    for status in ["fresh", "stale", "conflicting"]:
        changed = copy.deepcopy(snapshots["diamond"])
        edge = {"id": "custody-overlay", "kind": "custody", "source": tx, "target": last, "status": status, "amount_msat": None, "evidence": []}
        changed["edges"][edge["id"]] = edge
        changed["outgoing"].setdefault(tx, []).append(edge["id"])
        changed["incoming"].setdefault(last, []).append(edge["id"])
        snapshots[f"custody-{status}"] = changed
        for include in [True, False]:
            for observer in ["owner", "public"]:
                add(f"custody-{status}-{include}-{observer}", snapshot=f"custody-{status}", args={"mode": "path", "subject": tx, "target": last, "depth": 50, "include_relations": include, "observer": observer})
    for key, value in [("missing_tables", ["absent"]), ("invalid_observations", 1), ("reference_reconciling_count", 1)]:
        changed = copy.deepcopy(snapshots["diamond"])
        changed["coverage"][key] = value
        snapshots[key] = changed
        add(f"coverage-{key}", snapshot=key)
    for suffix, chain, network in [("network", "bitcoin", "regtest"), ("instance", "bitcoin", "main"), ("liquid", "liquid", "liquidv1")]:
        changed = copy.deepcopy(snapshots["diamond"])
        node = copy.deepcopy(changed["nodes"][tx])
        ident = f"{chain}:{network}:domain:other:tx:{1:064x}"
        node.update(id=ident, chain=chain, network=network, address="observed-address")
        changed["nodes"][ident] = node
        changed["nodes"][tx]["address"] = "observed-address"
        changed["subjects"][f"{1:064x}"].append(ident)
        changed["subjects"]["observed-address"] = [tx, ident]
        snapshots[suffix] = changed
        for subject in [f"{1:064x}", "observed-address"]:
            for filters in [{}, {"chain": "bitcoin"}, {"network": "main"}, {"chain": "liquid", "network": "main"}]:
                add(f"ambiguity-{suffix}-{subject}-{filters}", operation="resolve", snapshot=suffix, subject=subject, args=filters)
    snapshots["large"] = captured(large=True)
    for args in [{"node_limit": 25}, {"node_limit": 25, "depth": 50}, {"node_limit": 2000, "edge_limit": 50, "depth": 50}, {"node_limit": 2000, "depth": 50}, {"mode": "trace", "subject": tx, "node_limit": 25, "depth": 50}]:
        add(f"large-{len(cases)}", snapshot="large", args=args)
    conn = database()
    conn.execute("DELETE FROM transactions")
    snapshots["empty"] = snapshot(build_index(conn, "p"))
    conn.close()
    add("empty-complete", snapshot="empty")
    for key, value in [("missing_tables", ["absent"]), ("invalid_observations", 1)]:
        changed = copy.deepcopy(snapshots["empty"])
        changed["coverage"][key] = value
        snapshots["empty-" + key] = changed
        add("empty-" + key, snapshot="empty-" + key)
    return {"snapshots": snapshots, "cases": cases}


def encode(result):
    return "{\n\"snapshots\":{\n" + ",\n".join(json.dumps(k) + ":" + json.dumps(v,ensure_ascii=True,separators=(",",":")) for k,v in result["snapshots"].items()) + "\n},\n\"cases\":[\n" + ",\n".join(json.dumps(c,ensure_ascii=True,separators=(",",":")) for c in result["cases"]) + "\n]\n}\n"


def main():
    result = vectors()
    if "--write" in sys.argv:
        DEST.write_text(encode(result))
    elif json.loads(DEST.read_text()) != result:
        raise SystemExit("analysis traversal vectors drifted")
    print(f"{len(result['cases'])} analysis traversal vectors")


if __name__ == "__main__":
    main()
