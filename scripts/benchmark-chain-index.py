"""Local synthetic regression probe; no network, persistent book, or wallet sync.

Run from the repository: uv run --locked python scripts/benchmark-chain-index.py
Numbers are single-process unencrypted-memory SQLite measurements, not full-chain
capacity claims. Input generation is excluded; query figures are medians of five
warm runs. Peak RSS is process-cumulative and includes the cold rebuild oracle.
"""
import gc
import json
from pathlib import Path
import resource
import statistics
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from kassiber.core.chain_analysis import analyze_snapshot, build_index, run_analysis
from tests.test_chain_analysis import connection, NOW, txid


def row(number):
    raw = {"txid": txid(number), "chain": "bitcoin", "network": "main", "vsize": 100,
           "vin": [{"txid": txid(number-1), "vout": 0, "prevout": {"value": 1000, "scriptpubkey": "0014" + f"{number-1:040x}"}}] if number > 1 else [{"coinbase": "0101"}],
           "vout": [{"value": 1000, "scriptpubkey": "0014" + f"{number:040x}"}]}
    return str(number), "p", "w", txid(number), 1000000, "BTC", json.dumps(raw), NOW, 0, None


def measured(call):
    start = time.perf_counter()
    result = call()
    return time.perf_counter() - start, result


for count in map(int, sys.argv[1:] or (1000, 10000)):
    conn = connection()
    conn.executemany("INSERT INTO transactions VALUES(?,?,?,?,?,?,?,?,?,?)", (row(i) for i in range(1, count+1)))
    query = {"mode": "trace", "subject": txid(1), "direction": "forward", "depth": 3, "observer": "public"}
    oracle_time, oracle = measured(lambda: analyze_snapshot(build_index(conn, "p"), query))
    cold, initial = measured(lambda: run_analysis(conn, "p", query))
    assert initial["nodes"] == oracle["nodes"] and initial["edges"] == oracle["edges"]
    warm = [measured(lambda: run_analysis(conn, "p", query))[0] for _ in range(5)]
    conn.execute("INSERT INTO transactions VALUES(?,?,?,?,?,?,?,?,?,?)", row(count+1))
    delta, _ = measured(lambda: run_analysis(conn, "p", query))
    print(json.dumps({"transactions": count, "selected_nodes": len(initial["nodes"]), "oracle_seconds": oracle_time, "cold_projection_seconds": cold, "warm_query_median_seconds": statistics.median(warm), "one_transaction_update_seconds": delta, "process_peak_rss_mib": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024**2}), flush=True)
    conn.close()
    del oracle, initial
    gc.collect()
