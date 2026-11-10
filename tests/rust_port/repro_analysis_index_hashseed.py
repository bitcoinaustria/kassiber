"""Local reference repro: BIP30 ambiguity finding order depends on Python's seed.

Run with --seeds to record variants; no Python product code is changed.
"""
from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
SPEC = importlib.util.spec_from_file_location("analysis_index_vectors", ROOT / "crates/wallets/scripts/generate_analysis_index.py")
REFERENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE)


def reproduce():
    r = REFERENCE
    sql = ""
    for number in range(1, 5):
        txid = f"{number:064x}"
        sql += r.assertion(f"{r.BLOCK}:{number}", txid=txid)
        sql += r.assertion(f"{r.OTHER}:{number}", txid=txid)
        raw = {"txid": txid, "chain": "bitcoin", "network": "main"}
        sql += ("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,occurred_at,created_at,raw_json) VALUES("
            f"'ambiguous-{number}','ws','p','cold','ambiguous-{number}','{txid}','BTC','inbound',1000,0,'fixed','fixed','{json.dumps(raw)}');")
    conn = r.database(sql)
    try:
        index = r.build_index(conn, "p")
        return {"snapshot_id": index.snapshot_id, "finding_node_ids": [list(f["node_ids"]) for f in index.findings if f["code"] == "ambiguous_transaction_occurrence"]}
    finally:
        conn.close()


def variants():
    result = {}
    for seed in range(8):
        env = os.environ.copy()
        env["PYTHONHASHSEED"] = str(seed)
        child = subprocess.run([sys.executable, str(Path(__file__).resolve())], env=env, check=True, capture_output=True, timeout=30)
        result[str(seed)] = json.loads(child.stdout)
    return result


if __name__ == "__main__":
    print(json.dumps(variants() if "--seeds" in sys.argv else reproduce(), sort_keys=True, indent=2))
