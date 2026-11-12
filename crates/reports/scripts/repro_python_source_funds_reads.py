"""Reproduce stored-list reference quirks without touching an existing book."""
import json
import math
from pathlib import Path
import tempfile

from kassiber.db import open_db
from crates.reports.scripts.gen_source_funds_reads import ROOT, BASE, outcome


def main():
    scratch=ROOT/'target/r5-sof/scratch'
    scratch.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='sof-reference-repro-',dir=scratch) as directory:
        conn=open_db(Path(directory)/'data')
        conn.executescript((ROOT/'crates/reports/tests/data/book.sql').read_text()+BASE)
        result=outcome(conn,'ui.source_funds.evidence.list',{'cursor':'²'})
        assert result['error']['code']=='internal_error'
        assert result['error']['message']=="invalid literal for int() with base 10: '²'"
        result=outcome(conn,'ui.source_funds.evidence.list',{'cursor':'0'*4301})
        assert result['error']['code']=='internal_error'
        assert result['error']['message']=='Exceeds the limit (4300 digits) for integer string conversion: value has 4301 digits; use sys.set_int_max_str_digits() to increase the limit'
        conn.execute("UPDATE source_funds_sources SET amount=1.5 WHERE id='s-a'")
        result=outcome(conn,'source-funds.sources.list',{})
        source=next(row for row in result['ok'] if row['id']=='s-a')
        assert source['amount_msat']==1.5
        assert source['amount']==1e-11
        conn.execute("UPDATE source_funds_sources SET amount=-0.5 WHERE id='s-a'")
        source=next(row for row in outcome(conn,'source-funds.sources.list',{})['ok'] if row['id']=='s-a')
        assert source['amount_msat']==-0.5
        assert source['amount']==0.0 and math.copysign(1.0,source['amount'])==1.0
        result=outcome(conn,'source-funds.cases.list',{})
        assert result['ok'][1]['snapshot_hash']=='saved-hash'
        assert conn.execute("SELECT snapshot_json FROM source_funds_cases WHERE id='c-a'").fetchone()[0]=='{broken'
        conn.execute("UPDATE source_funds_recipients SET label='Renamed' WHERE id='r-a'")
        result=outcome(conn,'source-funds.cases.list',{})
        assert result['ok'][1]['recipient_label']=='Original bank'
        conn.close()
    print(json.dumps({'assertions':11,'reference_quirks':['nondecimal_digit_cursor_internal_error','oversized_cursor_internal_error','fractional_stored_msat_truncated_in_btc_projection','case_listing_does_not_validate_snapshot']}))


if __name__=='__main__':
    main()
