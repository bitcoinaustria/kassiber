import hashlib
import json
import os
from pathlib import Path
import re
import sqlite3

import pytest

from crates.reports.scripts.gen_source_funds_cases import dump
from kassiber import daemon
from kassiber.cli.command_registry import describe_command_catalog
from kassiber.cli.main import build_parser
from kassiber.db import open_db
from tests.test_rust_report_exports import _book, ROOT, RUST_BIN
from tests.test_rust_source_funds_diagnostics import _compare


# Every source-funds route is native; none stays Python.
PDF_COMMANDS = set()
PDF_KINDS = set()
NATIVE_COMMANDS = {
    'reports.source-funds', 'reports.export-source-funds-pdf', 'reports.export-source-funds-bundle',
    'source-funds.sources.list', 'source-funds.sources.create', 'source-funds.sources.attach',
    'source-funds.links.list', 'source-funds.links.create', 'source-funds.links.review',
    'source-funds.links.attach', 'source-funds.links.bulk-review',
    'source-funds.recipients.list', 'source-funds.recipients.create',
    'source-funds.recipients.update', 'source-funds.recipients.delete',
    'source-funds.cases.list', 'source-funds.review-context', 'source-funds.request-input',
    'source-funds.coverage', 'source-funds.suggest', 'source-funds.assemble',
}
NATIVE_KINDS = {
    'ui.source_funds.' + name.removeprefix('source-funds.').replace('-', '_')
    for name in NATIVE_COMMANDS if name.startswith('source-funds.')
} | {'ui.source_funds.preview', 'ui.source_funds.cases.save', 'ui.source_funds.evidence.list',
     'ui.source_funds.export_pdf', 'ui.source_funds.export_bundle'}


def catalog():
    return [c for c in describe_command_catalog(build_parser())['commands']
            if c['kind'].startswith('source-funds.') or 'source-funds' in c['kind']]


def test_source_funds_route_inventory_accounts_for_native_and_python_owners():
    assert len(NATIVE_COMMANDS) == 21
    assert len(NATIVE_KINDS) == 23
    assert {c['kind'] for c in catalog()} == NATIVE_COMMANDS | PDF_COMMANDS
    assert {k for k in daemon.SUPPORTED_KINDS if k.startswith('ui.source_funds.')} == NATIVE_KINDS | PDF_KINDS
    for path, expected in [
        ('crates/cli/src/describe.rs', NATIVE_COMMANDS),
        ('crates/cli/src/reports.rs', NATIVE_COMMANDS),
        ('crates/daemon/src/reports.rs', NATIVE_KINDS),
    ]:
        constants = (ROOT / path).read_text().split('];', 1)[0]
        found = set(re.findall(r'"((?:source-funds\.|reports\.[^"\n]*source-funds|ui\.source_funds\.)[^"\n]*)"', constants))
        assert found == expected, path


@pytest.mark.skipif(not RUST_BIN, reason='requires native test-harness binary')
@pytest.mark.parametrize('encrypted', [False, True], ids=['sqlite', 'sqlcipher'])
def test_source_funds_every_route_preserves_all_tables_hashes_and_declared_owner(tmp_path, monkeypatch, encrypted):
    root, database, env, call = _book(tmp_path, monkeypatch, 'generic', 'FIFO')
    conn = open_db(root)
    conn.execute('PRAGMA wal_checkpoint(TRUNCATE)')
    conn.execute('PRAGMA journal_mode=DELETE')
    conn.close()
    if encrypted:
        from sqlcipher3 import dbapi2
        conn = dbapi2.connect(database)
        output = tmp_path / 'encrypted.sqlite'
        conn.execute('ATTACH DATABASE ? AS encrypted KEY ?', (str(output), 'synthetic-routes-passphrase'))
        conn.execute("SELECT sqlcipher_export('encrypted')")
        conn.execute('DETACH DATABASE encrypted')
        conn.close()
        os.replace(output, database)

    def state():
        if encrypted:
            from sqlcipher3 import dbapi2 as driver
        else:
            driver = sqlite3
        connection = driver.connect(database)
        connection.row_factory = driver.Row
        if encrypted:
            connection.execute("PRAGMA key='synthetic-routes-passphrase'")
        result = dump(connection)
        connection.close()
        assert len(result) == 112
        return result

    from tests import cli_runner
    flags = ['--db-passphrase-fd', cli_runner.InputFd(b'synthetic-routes-passphrase')] if encrypted else []
    evidence = root / 'attachments/not-read/secret.pdf'
    evidence.parent.mkdir(parents=True)
    evidence.write_bytes(b'route audit evidence\0\xff')
    before_hash = hashlib.sha256(evidence.read_bytes()).hexdigest()
    before = state()
    expected_routes = []
    for command in catalog():
        argv = ['--machine', *command['path'], '--profile', 'missing-audit-profile']
        for argument in command['arguments']:
            if argument['required']:
                argv.extend([argument['flags'][0], str((argument['choices'] or ['missing-audit-record'])[0])])
        native, reference = call(*flags, *argv), call(*flags, *argv, reference=True)
        _compare(native, reference, daemon=False, debug=False)
        assert state() == before
        owner = 'rust' if command['kind'] in NATIVE_COMMANDS else 'python'
        expected_routes.append(dict(command=command['kind'], served_by=owner))
        if command['kind'] == 'reports.source-funds':
            native, reference = call(*flags, *argv, '--save-case'), call(*flags, *argv, '--save-case', reference=True)
            _compare(native, reference, daemon=False, debug=False)
            assert state() == before
            expected_routes.append(dict(command=command['kind'], served_by='rust'))
    args = dict(target_transaction='missing-audit-record')
    requests = [dict(request_id=kind, kind=kind, args=args) for kind in sorted(NATIVE_KINDS)]
    requests.append(dict(request_id='done', kind='daemon.shutdown'))
    wire = ''.join(json.dumps(r) + '\n' for r in requests).encode()
    native, reference = call(*flags, 'daemon', wire=wire), call(*flags, 'daemon', wire=wire, reference=True)
    _compare(native, reference, daemon=True, debug=False)
    assert state() == before
    expected_routes.append(dict(command='daemon', served_by='rust'))
    responses = [json.loads(line) for line in native.stdout.splitlines()]
    assert {r['request_id'] for r in responses if r.get('request_id') in NATIVE_KINDS} == NATIVE_KINDS
    for kind in sorted(PDF_KINDS):
        wire = (json.dumps(dict(request_id='pdf', kind=kind, args=dict(case='missing-audit-case'))) + '\n' + json.dumps(dict(request_id='done', kind='daemon.shutdown')) + '\n').encode()
        result = cli_runner.run(['--data-root', root, *flags, 'daemon'], env={**env, 'KASSIBER_HYBRID_RUST_DAEMON': '0'}, input=wire, capture_output=True, timeout=180)
        assert result.returncode == 0, result.stderr
        replies = [json.loads(line) for line in result.stdout.splitlines() if json.loads(line).get('request_id') == 'pdf']
        assert len(replies) == 1
        assert replies[0].get('error', {}).get('code') != 'unknown_kind'
        expected_routes.append(dict(command='daemon', served_by='python'))
        assert state() == before
    served = [json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert served == expected_routes
    assert hashlib.sha256(evidence.read_bytes()).hexdigest() == before_hash
    (tmp_path / 'route-audit.json').write_text(json.dumps(dict(cli=expected_routes, daemon_native=sorted(NATIVE_KINDS), daemon_python=sorted(PDF_KINDS)), indent=2) + '\n')
