"""Reproduce hash-seed-dependent order in Python's complete source-funds report.

Eight reviewed parents have the same disclosed external id and event time.
The simplified-flow level sorts set members by a key that omits row identity,
so its order (and the later review fingerprint) depends on the Python process.
This reproduction changes no Python product code and uses disposable books.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from kassiber.core.source_funds import build_report
from kassiber.daemon import _source_funds_hooks
from kassiber.db import open_db
from crates.reports.scripts.gen_source_funds_projection import ROOT, tables
from crates.reports.scripts.gen_source_funds_reads import BASE


def sample():
    scratch = ROOT/'target/r5-sof-reads-3/scratch'
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='flow-order-repro-', dir=scratch) as temporary:
        conn = open_db(Path(temporary)/'data')
        conn.executescript((ROOT/'crates/reports/tests/data/book.sql').read_text()+BASE)
        conn.execute('DELETE FROM source_funds_links')
        conn.execute("UPDATE transactions SET amount=8000,occurred_at='2025-01-01T00:00:00Z' WHERE id='in'")
        for index in range(8):
            tx_id = f'parent-{index}'
            conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,asset,direction,amount,fee,fiat_value,occurred_at,created_at) VALUES(?,'ws','p','cold','shared-external-id',?,'BTC','inbound',1000,0,1,'2024-01-01T00:00:00Z','fixed')", (tx_id,tx_id))
            conn.execute("INSERT INTO source_funds_links(id,workspace_id,profile_id,from_transaction_id,to_transaction_id,link_type,state,confidence,method,asset,allocation_amount,from_asset,from_allocation_amount,allocation_policy,created_at,updated_at) VALUES(?,'ws','p',?,'in','self_transfer','reviewed','exact','manual','BTC',1000,'BTC',1000,'explicit','fixed','fixed')", ('edge-'+str(index),tx_id))
            conn.execute("INSERT INTO source_funds_links(id,workspace_id,profile_id,from_source_id,to_transaction_id,link_type,state,confidence,method,asset,allocation_amount,from_asset,from_allocation_amount,allocation_policy,created_at,updated_at) VALUES(?,'ws','p','s-a',?,'manual_source','reviewed','exact','manual','BTC',1000,'BTC',1000,'explicit','fixed','fixed')", ('root-'+str(index),tx_id))
        conn.commit()
        before = tables(conn)
        report = build_report(conn, 'ws', 'p', _source_funds_hooks(), target_transaction_ref='in')
        assert report['explain_gates']['exportable']
        assert tables(conn) == before
        level = next(level for level in report['simplified_flow']['levels'] if level['distance_to_target']==1)
        assert len({(node['label'],node['occurred_at'],node['node_type']) for node in level['nodes']}) == 1
        result = [node['transaction_id'] for node in level['nodes']]
        conn.close()
        return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--child', action='store_true')
    args = parser.parse_args()
    if args.child:
        print(json.dumps(sample()))
        return
    orders = []
    for seed in ('1','2'):
        result = subprocess.run([sys.executable, '-m', 'crates.reports.scripts.repro_python_source_funds_flow_order', '--child'], env={**os.environ, 'PYTHONHASHSEED':seed}, capture_output=True, text=True, check=True, timeout=60)
        assert not result.stderr
        orders.append(json.loads(result.stdout))
    assert orders[0] != orders[1], orders
    assert sorted(orders[0]) == sorted(orders[1])
    print(json.dumps(dict(reference_bug='simplified_flow_tie_order', seeds=[1,2], orders=orders, tables_unchanged=114), indent=2))


if __name__ == '__main__':
    main()
