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
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from unittest import mock

from crates.wallets.scripts.generate_document_vectors import SOURCE, SOURCE_HASH, vectors, review_vectors, error
from kassiber.core import document_import as reference
from tests.test_document_import import _hooks
from tests.test_rust_store_migrations import normalized_dump
from tests.rust_fixture_binary import checked

ROOT = Path(__file__).resolve().parents[1]
DRIVER = checked(os.environ.get('KASSIBER_DOCUMENT_FIXTURE',
    ROOT / 'target/debug/examples' / ('document_fixture.exe' if os.name == 'nt' else 'document_fixture')),
    'cargo build -p kassiber-wallets --example document_fixture --features test-harness --offline --locked')
CORPUS = ROOT / 'crates/wallets/tests/vectors/python_document_rows.json'
TIME = '2026-01-02T03:04:05Z'
IDS = [f'00000000-0000-4000-8000-{i:012}' for i in range(1, 1001)]
ATTACHMENT_IDS = [f'00000000-0000-4000-8000-{i:012}' for i in range(2001, 3001)]


def cases():
    return json.loads(CORPUS.read_text())


class DocumentIngestionReferenceTest(unittest.TestCase):
    def test_document_cli_mutation_results_ignore_payload_words(self):
        from crates.wallets.scripts.document_cli_mutation_check import pytest_outcome
        def result(code, text):
            return pytest_outcome(subprocess.CompletedProcess([], code, text, ''))
        self.assertEqual(result(1, "E AssertionError: quarantined_skipped ERROR\n1 failed in 0.5s\n"), (1, True))
        self.assertEqual(result(0, "1 passed in 0.5s\n"), (0, False))
        for text in ["1 skipped in 0.5s\n", "1 passed, 1 subtests skipped in 0.5s\n",
                     "1 failed, 1 error in 0.5s\n", "no tests ran in 0.5s\n"]:
            self.assertEqual(result(1, text), (2, False))
        self.assertEqual(result(1, "E RuntimeError: fixture failed\n1 failed in 0.5s\n"), (1, False))

    def test_document_cli_mutations_apply_once_and_name_existing_tests(self):
        from crates.wallets.scripts.document_cli_mutation_check import problems
        self.assertEqual(problems(), [])

    def test_document_cli_review_matches_frozen_python_reference(self):
        from crates.wallets.scripts.generate_document_cli_vectors import DEST, vectors as cli_vectors
        self.assertEqual(DEST.read_text(), json.dumps(cli_vectors(), ensure_ascii=True, sort_keys=True, indent=2) + '\n')

    def test_document_rows_match_frozen_python_reference(self):
        self.assertEqual(cases(), vectors())
        self.assertEqual(json.loads(CORPUS.with_name('python_document_review.json').read_text()), review_vectors())
        from crates.wallets.scripts.generate_document_provider_vectors import vectors as provider_vectors
        self.assertEqual(json.loads(CORPUS.with_name('python_document_providers.json').read_text()), provider_vectors())


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
            # Seeded AI providers take the Rust AI clock, pinned like Python's now_iso.
            result = subprocess.run([DRIVER], input=json.dumps(request), text=True,
                capture_output=True, cwd=ROOT, check=True, timeout=90,
                env={**os.environ, 'KASSIBER_RUST_TEST_AI_TIME': TIME})
            self.assertEqual(result.stderr, '')
            return result.stdout
        if request.get('action') == 'preview':
            from kassiber.ai import providers
            with closing(self.connect(request['encrypted'])) as conn, \
                    mock.patch.object(reference, 'now_iso', return_value=TIME), \
                    mock.patch.object(providers, 'now_iso', return_value=TIME):
                try:
                    result = reference.preview_document_import(conn, source_file=request['source'],
                        provider_name=request.get('provider'), model=request.get('model'),
                        confidence_threshold=request.get('threshold'), max_pages=request.get('max_pages'),
                        pages=request.get('pages'), expected_fiat_currency='EUR')
                    output = {'result': result}
                except Exception as exc:
                    output = {'error': error(exc)}
            return json.dumps(output) + '\n'
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

    def test_provider_preview_wire_and_complete_books(self):
        from crates.wallets.scripts.generate_document_vectors import base
        ocr = json.dumps({'rows': [base()]})
        response = json.dumps({'output': [{'type': 'message', 'content': [{'type': 'output_text', 'text': ocr}]}]})
        chat = json.dumps({'choices': [{'message': {'content': ocr}}]})
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass
            def do_GET(self):
                self.respond()
            def do_POST(self):
                self.respond()
            def respond(self):
                body = self.rfile.read(int(self.headers.get('Content-Length', 0)))
                self.server.hits.append((self.command, self.path, body))
                status, payload = self.server.replies[min(self.server.index, len(self.server.replies) - 1)]
                self.server.index += 1
                data = payload.encode()
                self.send_response(status)
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                self.wfile.write(data)
        cases = [
            ('responses', 'auto', False, [(200, response)]),
            ('fallback', 'auto', False, [(404, 'Not found'), (200, chat)]),
            ('chat', 'chat_completions', False, [(200, chat)]),
            ('explicit-responses-no-fallback', 'responses', False, [(404, 'Not found')]),
            ('explicit-retention-choice', 'chat_completions', True, [(200, chat)]),
            ('store-refused', 'auto', False, [(400, '{"error":{"param":"store","message":"unsupported"}}')]),
            ('model-not-route', 'auto', False, [(404, '{"error":{"message":"model not found"}}')]),
            ('auth-refused', 'auto', False, [(401, 'Unauthorized')]),
        ]
        for encrypted in [False, True]:
            for name, wire, omit, replies in cases:
                with self.subTest(encrypted=encrypted, case=name):
                    baseline = self.baseline(encrypted)
                    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
                    server.daemon_threads = True
                    server.replies = [(200, '{"data":[{"id":"glm-ocr"}]}'), *replies]
                    server.hits = []
                    thread = threading.Thread(target=server.serve_forever)
                    thread.start()
                    try:
                        def setup(conn):
                            conn.execute("INSERT INTO settings(key,value) VALUES('database_instance_id','disposable-book')")
                            conn.execute("INSERT INTO ai_providers(name,base_url,kind,api_format,omit_store_field,created_at,updated_at) VALUES('local',?,'local',?,?,'2026-01-01','2026-01-01')",
                                (f'http://127.0.0.1:{server.server_port}/v1', wire, omit))
                            server.index = 0
                        request = self.request([], encrypted, action='preview', provider='local')
                        offline = self.root / 'offline.json'
                        offline.write_text('{"schema_version":1,"enabled":true}')
                        offline.chmod(0o600)
                        with mock.patch.dict(os.environ, {'KASSIBER_NO_EGRESS': '1',
                                'KASSIBER_OFFLINE_PREFERENCE_FILE': str(offline),
                                'HTTP_PROXY': 'http://192.0.2.2:9', 'HTTPS_PROXY': 'http://192.0.2.2:9'}):
                            outputs, states = self.compare(baseline, encrypted, [request], setup=setup)
                        hits = server.hits
                        half = len(hits) // 2
                        self.assertEqual(len(hits), 2 * len(server.replies))
                        for before, after in zip(hits[:half], hits[half:]):
                            self.assertEqual(before[:2], after[:2])
                            self.assertEqual(json.loads(before[2]) if before[2] else None,
                                json.loads(after[2]) if after[2] else None)
                        if name in {'responses', 'fallback', 'chat', 'explicit-retention-choice'}:
                            self.assertEqual(outputs[0]['result']['summary']['ready'], 1)
                    finally:
                        server.shutdown()
                        server.server_close()
                        thread.join()

    @unittest.skipUnless(shutil.which('pdfinfo') and shutil.which('pdftoppm'), 'Poppler OCR tools required')
    def test_pdf_preview_poppler_wire_complete_books_and_cleanup(self):
        from crates.wallets.scripts.generate_document_vectors import base
        def pdf_bytes(pages):
            objects = [b'<< /Type /Catalog /Pages 2 0 R >>',
                ('<< /Type /Pages /Count %d /Kids [%s] >>' % (pages,
                    ' '.join(f'{i + 3} 0 R' for i in range(pages)))).encode()]
            objects += [b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 72] >>'] * pages
            data = bytearray(b'%PDF-1.4\n')
            offsets = [0]
            for index, obj in enumerate(objects, 1):
                offsets.append(len(data))
                data.extend(f'{index} 0 obj\n'.encode() + obj + b'\nendobj\n')
            offset = len(data)
            data.extend(f'xref\n0 {len(offsets)}\n0000000000 65535 f \n'.encode())
            for position in offsets[1:]:
                data.extend(f'{position:010d} 00000 n \n'.encode())
            data.extend(f'trailer\n<< /Size {len(offsets)} /Root 1 0 R >>\nstartxref\n{offset}\n%%EOF\n'.encode())
            return bytes(data)
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass
            def do_GET(self):
                self.reply('{"data":[{"id":"glm-ocr"}]}')
            def do_POST(self):
                self.reply(self.server.ocr)
            def reply(self, payload):
                body = self.rfile.read(int(self.headers.get('Content-Length', 0)))
                self.server.hits.append((self.command, self.path, body))
                payload = payload.encode()
                self.send_response(200)
                self.send_header('Content-Length', str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)
        scratch = self.data / 'ocr-scratch'
        scratch.mkdir(mode=0o700)
        tools = self.root / 'missing-tools'
        tools.mkdir()
        for encrypted in [False, True]:
            for name, total, pages in [('complete', 2, None), ('range', 12, '9-12'),
                    ('selection-required', 9, None), ('missing-ppm', 1, None), ('missing-info', 1, None)]:
                with self.subTest(encrypted=encrypted, case=name):
                    baseline = self.baseline(encrypted)
                    self.source = self.root / 'statement with spaces.pdf'
                    self.source.write_bytes(pdf_bytes(total))
                    row = base()
                    row['source_region'] = {'page': 9 if name == 'range' else 1}
                    ocr = json.dumps({'rows': [row]})
                    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
                    server.daemon_threads = True
                    server.ocr = json.dumps({'output': [{'type': 'message', 'content': [{'type': 'output_text', 'text': ocr}]}]})
                    server.hits = []
                    thread = threading.Thread(target=server.serve_forever)
                    thread.start()
                    try:
                        def setup(conn):
                            conn.execute("INSERT INTO settings(key,value) VALUES('database_instance_id','disposable-book')")
                            conn.execute("INSERT INTO ai_providers(name,base_url,kind,created_at,updated_at) VALUES('local',?,'local','2026-01-01','2026-01-01')", (f'http://127.0.0.1:{server.server_port}/v1',))
                        environment = {'TMPDIR': str(scratch), 'KASSIBER_NO_EGRESS': '1'}
                        if name.startswith('missing-'):
                            (tools / 'pdftoppm').unlink(missing_ok=True)
                            if name == 'missing-info':
                                (tools / 'pdftoppm').symlink_to(shutil.which('pdftoppm'))
                            environment['PATH'] = str(tools)
                        with mock.patch.dict(os.environ, environment), mock.patch.object(tempfile, 'tempdir', str(scratch)):
                            outputs, _ = self.compare(baseline, encrypted,
                                [self.request([], encrypted, action='preview', provider='local', pages=pages)], setup=setup)
                        hits = server.hits
                        half = len(hits) // 2
                        self.assertEqual(half, 2 if name in {'complete', 'range'} else 1)
                        for before, after in zip(hits[:half], hits[half:]):
                            self.assertEqual(before[:2], after[:2])
                            self.assertEqual(json.loads(before[2]) if before[2] else None,
                                json.loads(after[2]) if after[2] else None)
                        self.assertEqual(list(scratch.iterdir()), [])
                        if name in {'complete', 'range'}:
                            result = outputs[0]['result']
                            self.assertEqual(result['source']['kind'], 'pdf')
                            self.assertEqual(result['source']['pdf']['complete'], name == 'complete')
                            self.assertEqual(result['source']['pdf']['rendered_pages'], [1, 2] if name == 'complete' else [9, 10, 11, 12])
                            self.assertEqual(result['summary']['ready'], 1)
                        else:
                            self.assertEqual(outputs[0]['error']['code'],
                                'document_import_pdf_page_selection_required' if name == 'selection-required' else 'document_import_pdf_renderer_missing')
                    finally:
                        server.shutdown()
                        server.server_close()
                        thread.join()

    def test_provider_policy_refusals_match_reference_without_contact(self):
        for encrypted in [False, True]:
            for name in ['disabled', 'remote-unacknowledged', 'remote', 'tee', 'local-remote-host',
                    'cli-target', 'native-missing', 'native-foreign', 'native-valid-unavailable', 'invalid-store']:
                with self.subTest(encrypted=encrypted, case=name):
                    baseline = self.baseline(encrypted)
                    def setup(conn):
                        conn.execute("INSERT INTO settings(key,value) VALUES('database_instance_id','disposable-book')")
                        conn.execute("INSERT INTO ai_providers(name,base_url,kind,created_at,updated_at) VALUES('local','http://127.0.0.1:9/v1','local','2026-01-01','2026-01-01')")
                        if name == 'disabled':
                            conn.execute("UPDATE ai_providers SET enabled=0")
                        if name in {'remote-unacknowledged', 'remote', 'tee'}:
                            conn.execute("UPDATE ai_providers SET kind=?,acknowledged_at=?",
                                ('tee' if name == 'tee' else 'remote', None if name == 'remote-unacknowledged' else TIME))
                        if name == 'local-remote-host':
                            conn.execute("UPDATE ai_providers SET base_url='https://example.invalid/v1'")
                        if name == 'cli-target':
                            conn.execute("UPDATE ai_providers SET base_url='codex-cli://default'")
                        if name.startswith('native-') or name == 'invalid-store':
                            from kassiber.ai.providers import ai_provider_secret_ref_namespace
                            service, account = ai_provider_secret_ref_namespace(conn, 'local')
                            if name == 'native-foreign':
                                service, account = 'foreign', 'foreign'
                            conn.execute("INSERT INTO ai_provider_secret_refs(provider_name,store_id,service,account,state,created_at) VALUES('local',?,?,?,?,?)",
                                ('bad-store' if name == 'invalid-store' else 'linux_secret_service', service, account,
                                 'missing' if name == 'native-missing' else 'ok', TIME))
                    self.compare(baseline, encrypted,
                        [self.request([], encrypted, action='preview', provider='local')], setup=setup)

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
