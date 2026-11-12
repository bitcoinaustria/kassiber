"""Reproduce fee attribution across a disclosed privacy boundary without fixing it."""
from pathlib import Path
import tempfile

from kassiber.core import source_funds
from kassiber.daemon import _source_funds_hooks
from kassiber.db import open_db

ROOT = Path(__file__).resolve().parents[3]


def main():
    scratch = ROOT / 'target/r5-sof-reads-2/scratch'
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='fee-privacy-reference-', dir=scratch) as temporary:
        conn = open_db(Path(temporary) / 'data')
        conn.executescript((ROOT / 'crates/reports/tests/data/book.sql').read_text())
        conn.executescript("""
            UPDATE transactions SET privacy_boundary='coinjoin' WHERE id='in';
            INSERT INTO source_funds_sources
                (id,workspace_id,profile_id,source_type,label,asset,amount,review_state,created_at,updated_at)
            VALUES ('root','ws','p','income','Origin evidence','BTC',123456789124,'reviewed','fixed','fixed');
            INSERT INTO source_funds_links
                (id,workspace_id,profile_id,from_source_id,to_transaction_id,link_type,state,confidence,
                 method,asset,allocation_amount,from_asset,from_allocation_amount,allocation_policy,
                 created_at,updated_at)
            VALUES ('privacy','ws','p','root','in','coinjoin','reviewed','strong','manual','BTC',
                    123456789001,'BTC',123456789124,'explicit','fixed','fixed');
        """)
        before = conn.total_changes
        report = source_funds.build_report(
            conn, 'ws', 'p', _source_funds_hooks(), target_transaction_ref='in')
        assert conn.total_changes == before
        assert any(row['code'] == 'privacy_hop_unresolved' for row in report['findings'])
        assert report['simplified_flow']['deferred_privacy_hops'][0]['link_id'] == 'privacy'
        assert report['allocations']['route_difference_explanation'] == dict(
            kind='network_fees', fee_msat=123, fee=1.23e-9, hop_count=1)
        assert not any(node.get('privacy_boundary') for node in report['graph']['nodes'])
        conn.close()
    print('Reference bug reproduced: the report names 123 msat as network fees across its CoinJoin boundary.')


if __name__ == '__main__':
    main()
