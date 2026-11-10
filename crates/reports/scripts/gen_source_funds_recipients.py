import argparse
import json
from pathlib import Path
import sqlite3
import tempfile
from unittest.mock import patch

from kassiber import daemon
from kassiber.core import source_funds_recipients as recipients
from kassiber.db import open_db
from kassiber.envelope import build_envelope, build_error_envelope
from kassiber.errors import AppError
from crates.reports.scripts.gen_source_funds_cases import dump, STAMP
from crates.reports.scripts.gen_source_funds_reads import BASE, ROOT

DESTINATION = ROOT / 'crates/reports/tests/data/source_funds_recipients.json'


def outcome(conn, case):
    calls = []
    def identity():
        calls.append('uuid')
        return case['id']
    def now():
        calls.append('now')
        return STAMP
    with patch.object(recipients.uuid, 'uuid4', identity), patch.object(recipients, 'now_iso', now):
        try:
            command, args = case['command'], case['args']
            if command.startswith('ui.'):
                data = daemon._ui_source_funds_payload_from_conn(conn, command, daemon._coerce_args_dict(None, args))
            else:
                workspace, profile = daemon._source_funds_hooks().resolve_scope(conn, args.get('workspace'), args.get('profile'))
                if command.endswith('.create'):
                    fields = {key: args.get(key) for key in ['label', 'kind', 'default_reveal_mode', 'notes']}
                    data = recipients.create_recipient(conn, workspace['id'], profile['id'], **fields)
                else:
                    recipient = recipients.resolve_recipient(conn, profile['id'], args.get('recipient'))
                    if command.endswith('.delete'):
                        data = recipients.delete_recipient(conn, profile['id'], recipient['id'])
                    else:
                        fields = {key: args.get(key) for key in ['label', 'kind', 'default_reveal_mode', 'notes']}
                        data = recipients.update_recipient(conn, profile['id'], recipient['id'], **fields)
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
    def add(name, action='create', desktop=True, sql='', identity='new-recipient', **fields):
        args = dict(label='  Disclosure Ω  ', kind=' Bank ', default_reveal_mode='standard', notes='  Exact notes Ω \n') if action == 'create' else dict(recipient='r-a')
        args.update(fields)
        result.append(dict(name=name, command=f'ui.source_funds.recipients.{action}' if desktop else f'source-funds.recipients.{action}', args=args, sql=sql, id=identity))
    add('desktop-create')
    add('cli-create', desktop=False)
    for key, values in dict(
        label=['', ' \t\u001c ', None, False, 42, 'alpha', 'ALPHA', 'Alpha', 'A retired recipient'],
        kind=list(recipients.RECIPIENT_KINDS)+[' Tax-Authority ', '', 'bad', None, False, 42, [], {}],
        default_reveal_mode=list(recipients.REVEAL_MODES)+[' MINIMAL ', '', ' ', 'bad', None, False, 42],
        notes=[None, '', ' \u001c ', ' keep\nverbatim Ω ', False, 42, [], {}],
    ).items():
        for index, item in enumerate(values):
            add(f'create-{key}-{index}', **{key:item})
    add('create-other-profile', desktop=False, profile='Other')
    add('create-ignores-desktop-scope', profile='missing', workspace='missing')
    add('create-no-context', sql="DELETE FROM settings WHERE key='context_profile';")
    add('create-id-collision', identity='r-z')
    add('create-validation-order', label='', kind='bad', default_reveal_mode='bad')
    for action in ['update', 'delete']:
        add(f'desktop-{action}', action)
        add(f'cli-{action}', action, desktop=False)
        for index, ref in enumerate(['alpha', 'ALPHA', 'Alpha', ' r-a ', 'r-i', 'A retired recipient', 'r-other', 'Other', 'missing', '', ' ', None, False, 42]):
            add(f'{action}-ref-{index}', action, recipient=ref)
        add(f'{action}-id-precedence', action, recipient='r-a', sql="UPDATE source_funds_recipients SET label='r-a' WHERE id='r-z';")
        add(f'{action}-other-profile', action, desktop=False, recipient='r-other', profile='Other')
        add(f'{action}-ignores-scope', action, recipient='r-a', profile='Other')
        add(f'{action}-scope-before-arguments', action, recipient='', sql="DELETE FROM settings WHERE key='context_profile';")
    for key, values in dict(
        label=['', ' \u001c ', None, False, 42, 'ALPHA', ' New label Ω '],
        kind=[' Tax-Authority ', '', None, False, 42, 'bad'],
        default_reveal_mode=[' FULL ', '', ' ', None, False, 42, 'bad'],
        notes=[None, '', ' \u001c ', ' New\nnotes Ω ', False, 42, [], {}],
    ).items():
        for index, item in enumerate(values):
            add(f'update-{key}-{index}', 'update', **{key:item})
    add('update-inactive-keeps-inactive', 'update', recipient='r-i', label='Retired renamed')
    add('update-preserves-note-whitespace', 'update', sql="UPDATE source_funds_recipients SET notes='  Existing\nnotes  ' WHERE id='r-a';")
    for action in ['create', 'update', 'delete']:
        verb = 'INSERT' if action == 'create' else 'UPDATE'
        for refusal in ['ABORT', 'ROLLBACK']:
            add(f'{action}-{refusal}', action, sql=f"CREATE TRIGGER refuse_write BEFORE {verb} ON source_funds_recipients BEGIN SELECT RAISE({refusal},'synthetic late refusal'); END;")
        add(f'{action}-deferred-commit', action, sql=f"CREATE TABLE commit_failure(id TEXT REFERENCES profiles(id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER refuse_commit AFTER {verb} ON source_funds_recipients BEGIN INSERT INTO commit_failure VALUES('missing'); END;")
        for index, args in enumerate([None, [], False, 42, 'wrong']):
            result.append(dict(name=f'{action}-args-{index}', command=f'ui.source_funds.recipients.{action}', args=args, sql='', id='new-recipient'))
    return result


def generate():
    scratch = ROOT / 'target/r5-sof-mutations-2/scratch'
    scratch.mkdir(parents=True, exist_ok=True)
    result = cases()
    with tempfile.TemporaryDirectory(prefix='recipient-oracle-', dir=scratch) as directory:
        template = open_db(Path(directory) / 'data')
        template.executescript((ROOT / 'crates/reports/tests/data/book.sql').read_text()+BASE)
        template.commit()
        schema = '\n'.join(template.iterdump())
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
                assert {k:v for k,v in before.items() if k != 'source_funds_recipients'} == {k:v for k,v in after.items() if k != 'source_funds_recipients'}, case['name']
                if 'error' in case['expected']:
                    assert before == after, case['name']
            case['tables'] = dict(source_funds_recipients=[dict(row) for row in conn.execute('SELECT * FROM source_funds_recipients ORDER BY rowid')])
            conn.close()
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
        raise SystemExit('recipient mutation vectors differ')
    print(f'{len(json.loads(generated)["cases"])} recipient mutation cases verified')


if __name__ == '__main__':
    main()
