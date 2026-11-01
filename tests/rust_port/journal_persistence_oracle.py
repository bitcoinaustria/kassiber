"""Complete, unmasked database-dump oracle at the R4-12 boundary.

Only disposable synthetic books belong here. SQL, rowids, stored scalar types,
UUIDs and clocks are retained; this is private test data, not diagnostics.
"""
from contextlib import ExitStack
import itertools
import json
import math
from pathlib import Path
import struct
import tempfile
import uuid
from unittest.mock import patch

from kassiber.core import custody_journal, custody_filed_reports, filed_report_chain
from tests.rust_port.journal_render_oracle import capture_build
from tests.test_journal_storage_format import _book, _profile

PATH = Path(__file__).resolve().parents[2] / 'crates/journals/tests/data/persistence.json'
CLOCK = '2026-10-31T12:00:00Z'


def encode_scalar(value):
    if isinstance(value, bytes):
        return {'$bytes': value.hex()}
    if isinstance(value, float) and not math.isfinite(value):
        return {'$real_bits': struct.pack('>d', value).hex()}
    return value


def decode_scalar(value):
    if isinstance(value, dict) and '$bytes' in value:
        return bytes.fromhex(value['$bytes'])
    if isinstance(value, dict) and '$real_bits' in value:
        return struct.unpack('>d', bytes.fromhex(value['$real_bits']))[0]
    return value


def dump(conn):
    tables = {}
    for (table, sql) in conn.execute("SELECT name,sql FROM sqlite_master WHERE type='table' ORDER BY name").fetchall():
        quoted = '"' + table.replace('"', '""') + '"'
        columns = [row[1] for row in conn.execute(f'PRAGMA table_info({quoted})')]
        if 'WITHOUT ROWID' not in sql.upper(): columns.insert(0, '_rowid_')
        query = ','.join('"' + c.replace('"', '""') + '"' for c in columns)
        rows = [[encode_scalar(v) for v in r]
                for r in conn.execute(f'SELECT {query} FROM {quoted}')]
        rows.sort(key=lambda row: json.dumps(row, sort_keys=True))
        tables[table] = dict(columns=columns, rows=rows)
    return tables


def capture(conn, name, profile=None, custody=None):
    profile = _profile(conn) if profile is None else profile
    conn.execute('BEGIN IMMEDIATE')
    builder = custody_journal.CustodyJournalBuilder(conn, profile, as_of=CLOCK)
    if custody is not None:
        builder.build_custody_projection = lambda: custody
    with capture_build(builder) as (record, _):
        state = builder.build()
    before = dump(conn)
    ids = []
    counter = itertools.count(1)
    def identity():
        ident = uuid.UUID(int=next(counter),version=4)
        ids.append(str(ident))
        return ident
    with ExitStack() as stack:
        stack.enter_context(patch.object(uuid, 'uuid4', identity))
        for module in (custody_journal,custody_filed_reports,filed_report_chain):
            stack.enter_context(patch.object(module,'now_iso',lambda:CLOCK))
        custody_journal.store_ledger_state(conn,profile,state,created_at=CLOCK)
    conn.commit()
    record.pop('_custody',None)
    return dict(name=name,record=record,before=before,expected=dump(conn),created_at=CLOCK,ids=ids)


def vectors():
    output=[]
    with tempfile.TemporaryDirectory(prefix='persistence-oracle-') as scratch:
        for mode in ('fresh','unchanged','appended','changed','mixed-stamps','clock-repeated','blocked','cipher'):
            with patch.object(custody_journal,'now_iso',lambda:'2026-01-01T00:00:00Z'):
                conn=_book(Path(scratch)/mode,passphrase='synthetic-test-only' if mode=='cipher' else None)
            if mode=='fresh':
                conn.execute('DELETE FROM journal_entries')
                conn.execute('DELETE FROM journal_quantity_postings')
            elif mode=='appended':
                conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,fee,kind,fiat_currency,fiat_rate,fiat_rate_exact,pricing_source_kind,pricing_quality,raw_json,created_at) VALUES('append','w','p','wallet','append','2024-05-01T00:00:00Z','inbound','BTC',1000000000,0,'buy','EUR',31000,'31000','manual','exact','{}','2020')")
            elif mode=='changed':
                conn.execute("UPDATE transactions SET fiat_rate=10001,fiat_rate_exact='10001' WHERE id='a'")
            elif mode=='mixed-stamps':
                conn.execute("UPDATE journal_entries SET occurred_at='2024-03-01T00:00:00Z',created_at=id")
            elif mode=='clock-repeated':
                conn.execute('UPDATE profiles SET last_processed_at=?',(CLOCK,))
            elif mode=='blocked':
                conn.execute("UPDATE transactions SET kind='unknown',fiat_rate=NULL,fiat_rate_exact=NULL WHERE id='a'")
            conn.execute('UPDATE profiles SET journal_input_version=journal_input_version+1')
            conn.commit()
            output.append(capture(conn,mode))
            conn.close()
        for mode in ('no-change','filed-change','saved-change','blocked-impact','scoped-impact','austrian-impact'):
            conn=_book(Path(scratch)/mode,country='AT' if mode=='austrian-impact' else 'generic')
            scope=dict(wallet_ids=['wallet'],occurred_at_start='2024-02-01T00:00:00Z',occurred_at_end='2024-03-01T00:00:00Z') if mode=='scoped-impact' else {}
            summaries=custody_filed_reports.current_report_summaries(conn,'p',period_start_year=2024,period_end_year=2024,report_scope=scope)
            snapshot=custody_filed_reports.create_filed_report_snapshot(
                conn,workspace_id='w',profile_id='p',report_kind='capital-gains',
                report_state='saved' if mode=='saved-change' else 'filed',
                period_start_year=2024,period_end_year=2024,content_sha256='ab'*32,
                classification_summary=summaries['classification_summary'],gain_summary=summaries['gain_summary'],
                report_scope=scope,created_at='2025-01-01T00:00:00Z')
            before=summaries if mode=='no-change' else dict(classification_summary={},gain_summary={})
            conn.execute('''INSERT INTO custody_filed_report_impacts(
                id,workspace_id,profile_id,filed_report_snapshot_id,component_id,review_id,gap_id,
                affected_period_start_year,affected_period_end_year,before_classification_summary_json,
                after_classification_summary_json,before_gain_summary_json,after_gain_summary_json,amendment_warning,created_at)
                VALUES('impact','w','p',?,'component','review','gap',2024,2024,?,'{}',?,'{}','Review','old')''',
                (snapshot['id'],json.dumps(before['classification_summary']),json.dumps(before['gain_summary'])))
            if mode=='blocked-impact':
                conn.execute("UPDATE transactions SET kind='unknown',fiat_rate=NULL,fiat_rate_exact=NULL WHERE id='a'")
            conn.execute('UPDATE profiles SET journal_input_version=journal_input_version+1')
            conn.commit()
            output.append(capture(conn,mode))
            conn.close()
    return output


if __name__=='__main__':
    PATH.write_text(json.dumps(vectors(),ensure_ascii=True,separators=(',',':'))+'\n')
