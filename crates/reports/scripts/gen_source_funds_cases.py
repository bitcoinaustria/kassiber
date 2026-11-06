import argparse
from decimal import Decimal
import json
from pathlib import Path
import sqlite3
import tempfile
from unittest.mock import patch

from kassiber.core import source_funds
from kassiber.db import open_db
from kassiber.envelope import build_envelope
from crates.reports.scripts.gen_source_funds_reads import BASE, ROOT
from tests.rust_port.book_schema import TABLES

DESTINATION = ROOT / 'crates/reports/tests/data/source_funds_cases.json'
STAMP = '2026-01-02T03:04:05Z'
DEFAULT = dict(
    workspace_id='ws', profile_id='p', target_transaction_id='in',
    target_amount_msat=123456789001, asset='BTC', reveal_mode='standard',
    status='exportable', label='Case Ω', target_external_id='frozen external',
    recipient_id='r-a', recipient_label_snapshot='Original bank',
    recipient_kind_snapshot='bank', recipient_reveal_mode_snapshot='minimal',
    snapshot=dict(
        target=dict(transaction_id='in', required_amount_msat=123456789001),
        source_mix=[dict(asset='BTC', amount_msat=123456789001)],
        attachments=[dict(id='a-a', sha256='hash', stored_relpath='not-read/secret.pdf')],
        diagrams=dict(svg='<svg>Ω</svg>'),
        explain_gates=dict(exportable=True),
    ),
)


def snapshot(value):
    if isinstance(value, dict):
        if set(value) == {'$decimal'}:
            return Decimal(value['$decimal'])
        return {key: snapshot(item) for key, item in value.items()}
    if isinstance(value, list):
        return [snapshot(item) for item in value]
    return value


def outcome(conn, case):
    args = dict(case['args'])
    args['snapshot'] = snapshot(args['snapshot'])
    calls = []
    ids = iter(case.get('ids', ['new-case', 'new-snapshot']))
    def identity():
        calls.append('uuid')
        return next(ids)
    def now():
        calls.append('now')
        return STAMP
    with patch.object(source_funds.uuid, 'uuid4', identity), patch.object(source_funds, '_now', now):
        try:
            result = source_funds.save_case_snapshot(conn, **args)
        except Exception as error:
            conn.rollback()
            return dict(error=dict(code='internal_error', message=str(error)), calls=calls)
    envelope = build_envelope('source-funds.case-snapshot', result)
    return dict(ok=result, calls=calls,
                cli=json.dumps(envelope, indent=2)+'\n',
                daemon=json.dumps(envelope, separators=(',', ':'))+'\n')


def dump(conn):
    tables = [row[0] for row in conn.execute(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
    assert tuple(tables) == TABLES
    return {table: sorted([dict(row) for row in conn.execute(f'SELECT * FROM "{table}"')],
                          key=lambda row: json.dumps(row, sort_keys=True)) for table in tables}


def cases():
    result = []
    def add(name, sql='', ids=None, **fields):
        case = dict(name=name, sql=sql, args=dict(DEFAULT, **fields))
        if ids is not None:
            case['ids'] = ids
        result.append(case)
    add('frozen-report')
    add('blocked-report', status='blocked')
    add('no-recipient', recipient_id=None, recipient_label_snapshot=None,
        recipient_kind_snapshot=None, recipient_reveal_mode_snapshot=None)
    add('empty-recipient', recipient_id='', recipient_label_snapshot='',
        recipient_kind_snapshot='', recipient_reveal_mode_snapshot='')
    add('renamed-recipient', sql="UPDATE source_funds_recipients SET label='Renamed',kind='other',default_reveal_mode='full' WHERE id='r-a';")
    add('deleted-recipient', sql="DELETE FROM source_funds_recipients WHERE id='r-a';")
    for mode in ['minimal', 'labels_only', 'full']:
        add('reveal-'+mode, reveal_mode=mode)
    for label in [None, '', '  Exact\nlabel Ω  ', 'quotation " \\ control\t']:
        add('label-'+repr(label), label=label)
    for external in [None, '', 'untrimmed\nexternal Ω']:
        add('external-'+repr(external), target_external_id=external)
    for amount in [0, 1, -1, 9223372036854775807, -9223372036854775808]:
        add('msat-'+str(amount), target_amount_msat=amount)
    add('liquid-denomination', target_transaction_id='earn', asset='LBTC', target_amount_msat=789001)
    for name, data in [
        ('empty', {}),
        ('order', {'z': 1, 'a': {'z': 2, 'a': 3}, 'array': [3, 2, 1]}),
        ('unicode', {'\uffff': '\ud800\udfff', '𐀀': '€ Ω\n\t\r\b\f \\ "\u0000'}),
        ('numbers', {'negative_zero': -0.0, 'tiny': 1e-100, 'big': 1e100, 'integer': 10**100, 'fraction': 1.00000000001}),
        ('nonfinite', {'nan': float('nan'), 'infinity': float('inf'), 'minus_infinity': -float('inf')}),
        ('decimal', {'nested': [{'$decimal': '123456.1234567890123456789'}, {'$decimal': '-0'}, {'$decimal': '1e1000'}]}),
        ('literal-provenance', {'url': 'https://private.invalid/proof?token=literal', 'stored_relpath': '../never-read', 'ownership': False, 'basis': None}),
    ]:
        add('snapshot-'+name, snapshot=data)
    add('case-id-collision', ids=['c-a', 'new-snapshot'])
    add('snapshot-id-collision', sql="INSERT INTO source_funds_snapshots(id,case_id,snapshot_hash,snapshot_json,created_at) VALUES('new-snapshot','c-a','hash','{}','fixed');")
    for table in ['source_funds_cases', 'source_funds_snapshots']:
        for action in ['ABORT', 'ROLLBACK']:
            add(table+'-'+action, sql=f"CREATE TRIGGER refuse_save BEFORE INSERT ON {table} BEGIN SELECT RAISE({action},'synthetic late refusal'); END;")
    add('foreign-key', target_transaction_id='missing')
    add('deferred-foreign-key', sql='PRAGMA defer_foreign_keys=ON;', target_transaction_id='missing')
    return result


def generate():
    scratch = ROOT / 'target/r5-sof-mutations/scratch'
    scratch.mkdir(parents=True, exist_ok=True)
    result = cases()
    with tempfile.TemporaryDirectory(prefix='case-oracle-', dir=scratch) as directory:
        template = open_db(Path(directory) / 'data')
        template.executescript((ROOT / 'crates/reports/tests/data/book.sql').read_text()+BASE)
        template.commit()
        for case in result:
            conn = sqlite3.connect(':memory:')
            conn.row_factory = sqlite3.Row
            template.backup(conn)
            conn.execute('PRAGMA foreign_keys=ON')
            conn.executescript(case['sql'])
            conn.commit()
            before = dump(conn)
            case['expected'] = outcome(conn, case)
            after = dump(conn)
            written = ['source_funds_cases', 'source_funds_snapshots']
            assert {k: v for k, v in before.items() if k not in written} == {k: v for k, v in after.items() if k not in written}
            if 'error' in case['expected']:
                assert before == after, case['name']
            case['tables'] = {name: after[name] for name in written}
            conn.close()
        template.close()
    return json.dumps(dict(base_sql=(ROOT / 'crates/reports/tests/data/book.sql').read_text()+BASE,
                           cases=result), ensure_ascii=True, indent=2)+'\n'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    generated = generate()
    if args.write:
        DESTINATION.write_text(generated)
    elif DESTINATION.read_text() != generated:
        raise SystemExit('source-funds case vectors differ')
    print(f'{len(json.loads(generated)["cases"])} frozen-case persistence cases verified')


if __name__ == '__main__':
    main()
