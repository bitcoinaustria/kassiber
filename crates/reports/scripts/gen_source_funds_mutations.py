import argparse
import json
from pathlib import Path
import sqlite3
import tempfile
from unittest.mock import patch

from kassiber import daemon
from kassiber.core import source_funds
from kassiber.db import open_db
from kassiber.envelope import build_envelope, build_error_envelope
from kassiber.errors import AppError
from crates.reports.scripts.gen_source_funds_cases import dump, STAMP
from crates.reports.scripts.gen_source_funds_reads import BASE, ROOT

DESTINATION = ROOT / 'crates/reports/tests/data/source_funds_mutations.json'
DEFAULT = dict(source_type='fiat_purchase', label='Origin Ω', asset='BTC',
               amount='1.23456789001', fiat_value='12.34', fiat_currency=' eur ',
               acquired_at='2024-01-01T02:30:00+02:30', description='Evidence, not tax authority')
WRITTEN = ['source_funds_sources', 'source_funds_source_attachments']


def outcome(conn, case):
    calls = []
    def identity():
        calls.append('uuid')
        return case.get('id', 'new-source')
    def now():
        calls.append('now')
        return STAMP
    with patch.object(source_funds.uuid, 'uuid4', identity), patch.object(source_funds, '_now', now):
        try:
            command, args = case['command'], case['args']
            if command.startswith('ui.'):
                data = daemon._ui_source_funds_payload_from_conn(conn, command, daemon._coerce_args_dict(None, args))
            elif command.endswith('.create'):
                fields = dict(args)
                fields.pop('workspace', None)
                fields.pop('profile', None)
                fields['attachment_ids'] = fields.pop('attachments', [])
                data = source_funds.create_source(conn, args.get('workspace'), args.get('profile'), daemon._source_funds_hooks(), **fields)
            else:
                data = source_funds.attach_source_evidence(conn, args.get('workspace'), args.get('profile'), daemon._source_funds_hooks(), source_ref=args['source'], attachment_id=args['attachment'])
            result = dict(ok=data)
            envelope = build_envelope(command, data)
        except Exception as error:
            conn.rollback()
            if isinstance(error, AppError):
                fields = dict(code=error.code, message=str(error), hint=error.hint, details=error.details, retryable=error.retryable)
            else:
                fields = dict(code='internal_error', message=str(error), hint=None, details=None, retryable=False)
            result = dict(error=fields)
            envelope = build_error_envelope(**fields)
    result['calls'] = calls
    result['cli'] = json.dumps(envelope, indent=2)+'\n'
    result['daemon'] = json.dumps(envelope, separators=(',', ':'))+'\n'
    return result


def cases():
    result = []
    def create(name, desktop=True, sql='', identity=None, **fields):
        case = dict(name=name, sql=sql, command='ui.source_funds.sources.create' if desktop else 'source-funds.sources.create', args=dict(DEFAULT, **fields))
        if identity is not None:
            case['id'] = identity
        result.append(case)
    def attach(name, desktop=True, sql='', **fields):
        args = dict(source='s-z', **({'attachment_id':'a-a'} if desktop else {'attachment':'a-a'}))
        args.update(fields)
        result.append(dict(name=name, sql=sql, command='ui.source_funds.sources.attach' if desktop else 'source-funds.sources.attach', args=args))
    create('desktop-create')
    create('cli-create', desktop=False, attachments=['a-a', 'a-z', 'a-a', 'a-orphan'])
    create('desktop-attachments', attachment_ids=['a-z', 'a-a', 'a-a', 'a-orphan'])
    create('desktop-singular', attachment_id='a-a')
    create('desktop-explicit-empty', attachment_ids=[], attachment_id='missing')
    for field, values in dict(
        source_type=list(source_funds.SOURCE_TYPES)+[' Opening-Balance-Attestation ', 'bad', '', None, False, 9],
        label=['  Exact\nlabel Ω  ', '', ' \t\u001c ', None, False, 0, 42],
        asset=[' l-btc ', 'LbTc', 'A'*64, 'custom Ω', '', None, False],
        amount=[None, '', '0', '-0', '0.00000000001', '0.0000000000049', '0.000000000005', '-0.0000000000049', '-0.000000000005', '-1', '92233720.36854775807', '92233720.36854775808', '1.234567890015', 'garbage', 'banana', 'Infinityx', 'NaN', 'Infinity', True, False, {}, []],
        fiat_value=[None, '', '0', '-0', '-12.5', '1e-13', '9e-14', '1e309', 'NaN', 'bad', True, False],
        fiat_currency=[None, '', ' \u001c ', ' usd ', 42, False],
        acquired_at=[None, '', '2024-01-01', '2024-01-01T00:00:00.987654Z', '1970-01-01T00:00:00Z', 'wrong', False, 0],
        description=[None, '', 42, False, [], {}],
        attachment_ids=[None, [], ['a-a', 'missing'], ['a-other'], [None], [False], 42, 'a-a'],
    ).items():
        for index, value in enumerate(values):
            create(f'{field}-{index}', **{field:value})
    create('default-currency', fiat_currency=None)
    create('missing-profile', sql="DELETE FROM settings WHERE key='context_profile';")
    create('ignore-desktop-scope', workspace='missing', profile='missing')
    create('cli-other-scope', desktop=False, profile='Other', fiat_currency=None)
    create('cli-id-collision', desktop=False, identity='s-a')
    create('deleted-before-return', sql="CREATE TRIGGER delete_source AFTER INSERT ON source_funds_sources BEGIN DELETE FROM source_funds_sources WHERE id=NEW.id; END;")
    for field, value in [('source_type', 'bad'), ('fiat_value', 'bad'), ('acquired_at', 'bad'), ('label', ''), ('description', [])]:
        create('overflow-order-'+field, amount='1e100', **{field:value})
    create('invalid-order', amount='wrong', source_type='bad', label='')
    for table in WRITTEN:
        for action in ['ABORT', 'ROLLBACK']:
            create(f'{table}-{action}', sql=f"CREATE TRIGGER refuse_write BEFORE INSERT ON {table} BEGIN SELECT RAISE({action},'synthetic late refusal'); END;", attachment_ids=['a-a'])
    create('deferred-commit', sql="CREATE TABLE commit_failure(id TEXT REFERENCES profiles(id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER refuse_commit AFTER INSERT ON source_funds_source_attachments BEGIN INSERT INTO commit_failure VALUES('missing'); END;", attachment_ids=['a-a'])
    attach('desktop-attach')
    attach('cli-attach', desktop=False)
    attach('repeat-attachment', source='s-a')
    attach('source-label', source='Unknown')
    attach('trimmed-desktop', source=' s-z ', attachment_id=' a-a ')
    attach('literal-cli', desktop=False, source=' s-z ')
    attach('other-book-source', source='s-other')
    attach('other-book-attachment', attachment_id='a-other')
    attach('ambiguous-source', source='Unknown', sql="UPDATE source_funds_sources SET label='UNKNOWN' WHERE id='s-a';")
    attach('id-and-label-ambiguous', source='s-z', sql="UPDATE source_funds_sources SET label='s-z' WHERE id='s-a';")
    for key in ['source', 'attachment_id']:
        for index, item in enumerate([None, '', ' \t ', 42, False]):
            attach(f'attach-{key}-{index}', **{key:item})
    for action in ['ABORT', 'ROLLBACK']:
        attach('attach-'+action, sql=f"CREATE TRIGGER refuse_attach BEFORE INSERT ON source_funds_source_attachments BEGIN SELECT RAISE({action},'synthetic late refusal'); END;")
    for command in ['ui.source_funds.sources.create', 'ui.source_funds.sources.attach']:
        for index, args in enumerate([None, [], False, 1, 'wrong']):
            result.append(dict(name=f'{command}-args-{index}', command=command, args=args, sql=''))
    return result


def generate():
    scratch = ROOT / 'target/r5-sof-mutations-2/scratch'
    scratch.mkdir(parents=True, exist_ok=True)
    result = cases()
    with tempfile.TemporaryDirectory(prefix='source-oracle-', dir=scratch) as directory:
        template = open_db(Path(directory) / 'data')
        template.executescript((ROOT / 'crates/reports/tests/data/book.sql').read_text()+BASE)
        template.commit()
        for case in result:
            conn = sqlite3.connect(':memory:')
            conn.row_factory = sqlite3.Row
            template.backup(conn)
            conn.execute('PRAGMA foreign_keys=ON')
            conn.executescript(case['sql'])
            before = dump(conn) if 'CREATE TABLE' not in case['sql'] else None
            case['expected'] = outcome(conn, case)
            if before is not None:
                after = dump(conn)
                assert {k:v for k,v in before.items() if k not in WRITTEN} == {k:v for k,v in after.items() if k not in WRITTEN}, case['name']
                if 'error' in case['expected']:
                    assert before == after, case['name']
            case['tables'] = {table: [dict(row) for row in conn.execute(f'SELECT * FROM {table} ORDER BY rowid')] for table in WRITTEN}
            conn.close()
        schema = "\n".join(template.iterdump())
        template.close()
        from sqlcipher3 import dbapi2 as cipher
        for case in result:
            conn = cipher.connect(':memory:')
            conn.row_factory = cipher.Row
            conn.executescript(schema)
            conn.execute('PRAGMA foreign_keys=ON')
            conn.executescript(case['sql'])
            expected = outcome(conn, case)
            if expected != case['expected']:
                case['expected_cipher'] = expected
            conn.close()
    return json.dumps(dict(base_sql=(ROOT / 'crates/reports/tests/data/book.sql').read_text()+BASE, cases=result), ensure_ascii=True, indent=2)+'\n'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    generated = generate()
    if args.write:
        DESTINATION.write_text(generated)
    elif DESTINATION.read_text() != generated:
        raise SystemExit('source mutation vectors differ')
    print(f'{len(json.loads(generated)["cases"])} source mutation cases verified')


if __name__ == '__main__':
    main()
