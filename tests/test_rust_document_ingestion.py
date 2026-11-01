"""Python oracle for the unregistered native reviewed-document core."""
from contextlib import closing
import copy
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

from crates.wallets.scripts.generate_document_vectors import SOURCE, SOURCE_HASH, vectors, review_vectors, error
from kassiber.core import document_import as reference
from tests.test_document_import import _hooks
from tests.test_rust_store_migrations import normalized_dump

ROOT = Path(__file__).resolve().parents[1]
DRIVER = Path(os.environ.get('KASSIBER_DOCUMENT_FIXTURE',
    ROOT / 'target/debug/examples' / ('document_fixture.exe' if os.name == 'nt' else 'document_fixture')))
CORPUS = ROOT / 'crates/wallets/tests/vectors/python_document_rows.json'
TIME = '2026-01-02T03:04:05Z'
IDS = [f'00000000-0000-4000-8000-{i:012}' for i in range(1, 1001)]
ATTACHMENT_IDS = [f'00000000-0000-4000-8000-{i:012}' for i in range(2001, 3001)]


def cases():
    return json.loads(CORPUS.read_text())


class DocumentIngestionReferenceTest(unittest.TestCase):
    def test_document_rows_match_frozen_python_reference(self):
        self.assertEqual(cases(), vectors())
        self.assertEqual(json.loads(CORPUS.with_name('python_document_review.json').read_text()), review_vectors())


@unittest.skipUnless(DRIVER.is_file(), 'build the document_fixture test-harness example')
class RustDocumentIngestionTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='document-core-oracle-')
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.data = self.root / 'data'
        self.data.mkdir()
        self.db = self.data / 'kassiber.sqlite3'
        self.source = self.root / 'receipt.png'
        self.source.write_bytes(SOURCE)
        self.attachments = self.root / 'attachments'

    def connect(self, encrypted):
        if encrypted:
            from sqlcipher3 import dbapi2
            conn = dbapi2.connect(self.db)
            conn.execute("PRAGMA key='disposable-test-key'")
            conn.row_factory = dbapi2.Row
        else:
            conn = sqlite3.connect(self.db)
            conn.row_factory = sqlite3.Row
        conn.execute('PRAGMA foreign_keys=ON')
        return conn

    def baseline(self, encrypted):
        self.db.unlink(missing_ok=True)
        with closing(self.connect(encrypted)) as conn:
            for sql in json.loads((ROOT / 'crates/store/fixtures/creation.json').read_text())['statements']:
                conn.execute(sql)
            conn.executescript("""
                INSERT INTO workspaces(id,label,created_at) VALUES('ws','Workspace','2025-01-01');
                INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','ws','Book','2025-01-01');
                INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at)
                    VALUES('wallet','ws','p','Source','custom','{}','2025-01-01');
                INSERT INTO settings(key,value) VALUES('context_workspace','ws'),('context_profile','p');
                UPDATE profiles SET last_processed_at='2025-01-01',last_processed_tx_count=3,journal_input_version=7,fiat_currency='EUR';
            """)
            conn.commit()
        return self.db.read_bytes()

    def state(self, encrypted):
        with closing(self.connect(encrypted)) as conn:
            dump = normalized_dump(conn)
        files = {str(p.relative_to(self.attachments)): p.read_bytes()
            for p in self.attachments.rglob('*') if p.is_file()} if self.attachments.exists() else {}
        return dump, files

    def request(self, rows, encrypted, **kwargs):
        return dict(database=str(self.db), data_root=str(self.data), source=str(self.source),
            rows=rows, encrypted=encrypted, expected_hash=SOURCE_HASH, time=TIME,
            ids=IDS, attachment_ids=ATTACHMENT_IDS, **kwargs)

    def call(self, rust, request):
        if rust:
            result = subprocess.run([DRIVER], input=json.dumps(request), text=True,
                capture_output=True, cwd=ROOT, check=True, timeout=90)
            self.assertEqual(result.stderr, '')
            return result.stdout
        from kassiber.core import imports, attachments
        from kassiber.cli.handlers import resolve_transaction
        with closing(self.connect(request['encrypted'])) as conn:
            profile = conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone()
            wallet = conn.execute("SELECT * FROM wallets WHERE id='wallet'").fetchone()
            hooks = _hooks()
            hooks = reference.DocumentImportHooks(hooks.import_hooks,
                attachments.AttachmentHooks(hooks.attachment_hooks.resolve_scope,
                    resolve_transaction, lambda: TIME))
            ids = iter(request['ids']); attachment_ids = iter(request['attachment_ids'])
            with mock.patch.object(imports, 'uuid', SimpleNamespace(uuid4=lambda: next(ids))), \
                    mock.patch.object(imports, 'now_iso', return_value=TIME), \
                    mock.patch.object(attachments, 'uuid', SimpleNamespace(uuid4=lambda: next(attachment_ids))):
                if request.get('outer_rollback'):
                    conn.execute('BEGIN')
                try:
                    outcome = reference.import_document_draft(conn, source_file=request['source'],
                        data_root=request['data_root'], wallet=wallet, profile=profile,
                        rows=request['rows'], hooks=hooks,
                        include_quarantined=request.get('include_quarantined', False),
                        selected_row_ids=request.get('selected'),
                        expected_source_sha256=request.get('expected_hash'),
                        confidence_threshold=request.get('threshold'),
                        attach_evidence=request.get('attach', True),
                        commit=not request.get('outer_rollback', False))
                    output = {'result': outcome}
                except Exception as exc:
                    output = {'error': error(exc)}
                if request.get('outer_rollback'):
                    conn.rollback()
        return json.dumps(output) + '\n'

    def compare(self, baseline, encrypted, requests, setup=None, database_error=None):
        results, states, wires = [], [], []
        for rust in [False, True]:
            self.db.write_bytes(baseline)
            shutil.rmtree(self.attachments, ignore_errors=True)
            if setup:
                with closing(self.connect(encrypted)) as conn:
                    setup(conn)
                    conn.commit()
            outputs, dumps, raw = [], [], []
            for index, request in enumerate(requests):
                # Different invocations get different UUIDs, including retries.
                request = dict(request, ids=IDS[index * 50:], attachment_ids=ATTACHMENT_IDS[index * 50:])
                wire = self.call(rust, request)
                raw.append(wire)
                outputs.append(json.loads(wire))
                dumps.append(self.state(encrypted))
            results.append(outputs); states.append(dumps); wires.append(raw)
        if database_error:
            # The shared native store intentionally redacts database messages.
            # Pin the difference, never call it byte-for-byte error parity.
            self.assertEqual(results[0], [{'error': error(RuntimeError(database_error))}])
            self.assertEqual(results[1], [{'error': error(RuntimeError('database operation failed'))}])
        else:
            self.assertEqual(results[1], results[0])
            self.assertEqual(wires[1], wires[0])
        self.assertEqual(states[1], states[0])
        return results[1], states[1]

    def test_reference_drafts_match_complete_books_and_managed_bytes_on_both_codecs(self):
        selected = [case for case in cases() if 'rows' in case]
        for encrypted in [False, True]:
            baseline = self.baseline(encrypted)
            for case in selected:
                with self.subTest(encrypted=encrypted, name=case['name']):
                    outputs, _ = self.compare(baseline, encrypted, [self.request(case['rows'], encrypted,
                        threshold=case['threshold'])])
                    if case['name'] == 'ready':
                        self.assertEqual(outputs[0]['result']['imported'], 1)
                        self.assertEqual(len(outputs[0]['result']['attached_evidence']), 1)

    def test_reimports_enrichment_selection_hash_binding_and_rollback_match_reference(self):
        ready = copy.deepcopy(cases()[0]['rows'])
        unpriced = copy.deepcopy(ready)
        unpriced[0]['record'].update(fiat_value=None, fiat_rate=None)
        low = copy.deepcopy(ready); low[0]['confidence'] = .1
        malicious = copy.deepcopy(ready)
        malicious[0]['record']['amount_btc'] = '0.000000000001'
        malicious[0]['import_record']['amount'] = '999'
        enriched = copy.deepcopy(ready); enriched[0]['record']['description'] = 'New description'
        for encrypted in [False, True]:
            baseline = self.baseline(encrypted)
            scenarios = [
                [self.request(unpriced, encrypted), self.request(unpriced, encrypted),
                    self.request(ready, encrypted), self.request(ready, encrypted), self.request(enriched, encrypted)],
                [self.request(ready, encrypted, selected=[])],
                [self.request(low, encrypted)],
                [self.request(low, encrypted, include_quarantined=True)],
                [self.request(malicious, encrypted, include_quarantined=True)],
                [dict(self.request(ready, encrypted), expected_hash='0' * 64)],
                [dict(self.request(ready, encrypted), expected_hash='bad')],
                [self.request(ready, encrypted, attach=False)],
                [self.request(ready, encrypted, outer_rollback=True)],
            ]
            for index, scenario in enumerate(scenarios):
                with self.subTest(encrypted=encrypted, scenario=index):
                    outputs, _ = self.compare(baseline, encrypted, scenario)
                    if index == 0:
                        self.assertEqual(outputs[0]['result']['imported'], 1)
                        self.assertEqual(len(outputs[0]['result']['attached_evidence']), 1)
                        self.assertEqual(outputs[1]['result']['unchanged'], 1)
                        self.assertEqual(outputs[2]['result']['updated'], 1)
                        self.assertEqual(outputs[3]['result']['updated'], 1)
                    if index == 3:
                        self.assertEqual(outputs[0]['result']['imported'], 1)
            second = copy.deepcopy(ready[0]); second['id'] = second['id'][:-3] + '002'
            def fail_second(conn):
                conn.execute("CREATE TRIGGER fail_document_attachment BEFORE INSERT ON attachments "
                    "WHEN (SELECT COUNT(*) FROM attachments)>0 BEGIN SELECT RAISE(ABORT,'late attachment failure'); END")
            outputs, states = self.compare(baseline, encrypted,
                [self.request([ready[0], second], encrypted)], setup=fail_second,
                database_error='late attachment failure')
            self.assertEqual(outputs[0]['error']['message'], 'database operation failed')
            self.assertEqual(states[0][1], {})

    def test_attachment_context_refusals_and_unattached_imports_match_reference(self):
        ready = cases()[0]['rows']
        for encrypted in [False, True]:
            baseline = self.baseline(encrypted)
            def no_context(conn):
                conn.execute("DELETE FROM settings WHERE key LIKE 'context_%'")
            def other_context(conn):
                conn.execute("INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('other','ws','Other','2025-01-01')")
                conn.execute("UPDATE settings SET value='other' WHERE key='context_profile'")
            for setup in [no_context, other_context]:
                with self.subTest(encrypted=encrypted, setup=setup.__name__):
                    outputs, states = self.compare(baseline, encrypted,
                        [self.request(ready, encrypted)], setup=setup)
                    self.assertIn('error', outputs[0])
                    self.assertEqual(states[0][1], {})
                    outputs, _ = self.compare(baseline, encrypted,
                        [self.request(ready, encrypted, attach=False)], setup=setup)
                    self.assertEqual(outputs[0]['result']['imported'], 1)

    def test_local_source_validation_refusals_match_reference_without_writes(self):
        unsupported = self.root / 'receipt.csv'
        unsupported.write_bytes(b'not an OCR source')
        sources = [self.root, unsupported, self.root / 'absent.png', '',
            'https://drive.google.com/file/d/synthetic', 'http://127.0.0.1:1/statement.pdf']
        for encrypted in [False, True]:
            baseline = self.baseline(encrypted)
            for source in sources:
                with self.subTest(encrypted=encrypted, source=str(source)):
                    request = dict(self.request(cases()[0]['rows'], encrypted), source=str(source))
                    outputs, states = self.compare(baseline, encrypted, [request])
                    self.assertIn('error', outputs[0])
                    self.assertEqual(states[0][1], {})
                    with closing(self.connect(encrypted)) as conn:
                        self.assertEqual(conn.execute('SELECT count(*) FROM transactions').fetchone()[0], 0)
                        self.assertEqual(conn.execute('SELECT journal_input_version FROM profiles').fetchone()[0], 7)
