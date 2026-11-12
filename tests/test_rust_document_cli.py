from contextlib import closing, contextmanager
import json
import os
from pathlib import Path
import selectors
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from unittest import mock

from crates.wallets.scripts.generate_document_vectors import base, SOURCE
from tests import test_rust_document_ingestion as core_reference

IDS, ATTACHMENT_IDS, TIME = core_reference.IDS, core_reference.ATTACHMENT_IDS, core_reference.TIME

ROOT = Path(__file__).resolve().parents[1]
BINARY = os.environ.get('KASSIBER_RUST_BIN')
PROMPT = b'Enter comma-separated row numbers to import, or press Enter to cancel: '


@contextmanager
def owned_child(child):
    try:
        yield child
    finally:
        if child.poll() is None:
            child.kill()
            child.wait(timeout=10)
        child.stdout.close()
        child.stderr.close()


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        self.reply({'data': [{'id': 'glm-ocr'}]})

    def do_POST(self):
        self.reply({'output': [{'type': 'message', 'content': [{'type': 'output_text',
            'text': json.dumps({'rows': self.server.rows})}]}]})

    def reply(self, value):
        body = self.rfile.read(int(self.headers.get('Content-Length', 0)))
        self.server.hits.append((self.command, self.path, json.loads(body) if body else None))
        data = json.dumps(value).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)


@unittest.skipUnless(BINARY and os.name == 'posix', 'set KASSIBER_RUST_BIN; terminal acceptance requires POSIX')
class RustDocumentCliTest(unittest.TestCase):
    connect = core_reference.RustDocumentIngestionTest.connect
    state = core_reference.RustDocumentIngestionTest.state
    core_baseline = core_reference.RustDocumentIngestionTest.baseline

    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix='document-cli-')
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.data = self.root / 'data'
        self.data.mkdir()
        self.db = self.data / 'kassiber.sqlite3'
        self.source = self.root / 'receipt.png'
        self.source.write_bytes(SOURCE)
        self.attachments = self.root / 'attachments'
        owners = self.root / 'owners'
        owners.mkdir(mode=0o700)
        self.log = self.root / 'served.jsonl'
        self.offline = self.root / 'offline.json'
        self.offline.write_text('{"schema_version":1,"enabled":true}')
        self.offline.chmod(0o600)
        environment = {
            'KASSIBER_RUST_TEST_OWNER_ROOT': str(owners),
            'XDG_DATA_HOME': str(self.root / 'xdg-data'),
            'XDG_CONFIG_HOME': str(self.root / 'xdg-config'),
            'XDG_STATE_HOME': str(self.root / 'xdg-state'),
            'KASSIBER_DISABLE_UPDATE_CHECK': '1',
            'KASSIBER_OFFLINE_PREFERENCE_FILE': str(self.offline),
            'KASSIBER_AGENT_ACCESS_FILE': str(self.root / 'agents.json'),
            'KASSIBER_UPDATE_PREFERENCE_FILE': str(self.root / 'updates.json'),
            'KASSIBER_UPDATE_CACHE_FILE': str(self.root / 'update-cache.json'),
            'KASSIBER_HYBRID_RUST_BIN': str(Path(BINARY).resolve()),
            'KASSIBER_HYBRID_LOG': str(self.log),
            'KASSIBER_RUST_TEST_DOCUMENT_TIME': TIME,
            'KASSIBER_RUST_TEST_DOCUMENT_ATTACHMENT_IDS': ','.join(ATTACHMENT_IDS),
            'KASSIBER_RUST_TEST_IMPORT_TIME': TIME,
            # Resolving the OCR provider seeds the built-in rows (#698) at this time.
            'KASSIBER_RUST_TEST_AI_TIME': TIME,
            'KASSIBER_RUST_TEST_IMPORT_IDS': ','.join(IDS),
            'KASSIBER_NO_EGRESS': '1',
            'HTTP_PROXY': 'http://192.0.2.2:9',
            'HTTPS_PROXY': 'http://192.0.2.2:9',
        }
        patcher = mock.patch.dict(os.environ, environment)
        patcher.start()
        self.addCleanup(patcher.stop)
        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.server.rows = [base()]
        self.server.hits = []
        self.thread = threading.Thread(target=self.server.serve_forever)
        self.thread.start()
        self.addCleanup(self.stop_server)

    def stop_server(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()

    def baseline(self, encrypted):
        self.core_baseline(encrypted)
        result = self.cli('rust', ['init'], encrypted, machine=True)
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        with closing(self.connect(encrypted)) as conn:
            conn.execute("INSERT INTO settings(key,value) VALUES('ai_providers_seeded','1') ON CONFLICT(key) DO UPDATE SET value='1'")
            conn.execute("INSERT INTO ai_providers(name,base_url,kind,created_at,updated_at) VALUES('local',?,'local','2026-01-01','2026-01-01')", (f'http://127.0.0.1:{self.server.server_port}/v1',))
            conn.commit()
        return self.db.read_bytes()

    def cli(self, runtime, args, encrypted, *, machine=False, interactive=False, response=b'1\n', mutate=None, offset=0, output_format='json', non_interactive=False):
        import pty
        import termios
        command = {'python': [sys.executable, '-m', 'tests.rust_port.document_cli_reference'],
            'rust': [str(Path(BINARY).resolve())],
            'hybrid': [sys.executable, str(ROOT / 'tests/rust_port/hybrid_cli.py')]}[runtime]
        argv = ['--machine'] if machine else (['--format', output_format] if output_format else [])
        argv += ['--data-root', str(self.data)]
        if non_interactive:
            argv += ['--non-interactive']
        descriptors = []
        if encrypted:
            read, write = os.pipe()
            os.write(write, b'disposable-test-key')
            os.close(write)
            argv += ['--db-passphrase-fd', str(read)]
            descriptors.append(read)
        environment = {**os.environ, 'KASSIBER_RUST_TEST_IMPORT_IDS': ','.join(IDS[offset:]),
            'KASSIBER_RUST_TEST_DOCUMENT_ATTACHMENT_IDS': ','.join(ATTACHMENT_IDS[offset:])}
        master = slave = None
        try:
            if interactive:
                master, slave = pty.openpty()
                attributes = termios.tcgetattr(slave)
                attributes[3] &= ~termios.ECHO
                termios.tcsetattr(slave, termios.TCSANOW, attributes)
                child = subprocess.Popen([*command, *argv, *args], stdin=slave, stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE, pass_fds=descriptors, cwd=ROOT, env=environment)
                with owned_child(child):
                    prefix = bytearray()
                    stderr = bytearray()
                    with selectors.DefaultSelector() as poll:
                        poll.register(child.stdout, selectors.EVENT_READ, prefix)
                        poll.register(child.stderr, selectors.EVENT_READ, stderr)
                        deadline = time.monotonic() + 90
                        while PROMPT not in prefix:
                            self.assertLess(time.monotonic(), deadline, (prefix, stderr))
                            events = poll.select(1)
                            if child.poll() is not None and not events:
                                break
                            for key, _ in events:
                                block = os.read(key.fileobj.fileno(), 65536)
                                if block:
                                    key.data.extend(block)
                                else:
                                    poll.unregister(key.fileobj)
                            if not poll.get_map():
                                break
                    if PROMPT in prefix:
                        if mutate:
                            mutate()
                        if response == 'interrupt':
                            child.send_signal(signal.SIGINT)
                        else:
                            os.write(master, response or b'\x04')
                    out, err = child.communicate(timeout=90)
                    return subprocess.CompletedProcess(args, child.returncode, bytes(prefix) + out, bytes(stderr) + err)
            return subprocess.run([*command, *argv, *args], input=b'', capture_output=True,
                pass_fds=descriptors, cwd=ROOT, env=environment, timeout=90)
        finally:
            for descriptor in [*descriptors, master, slave]:
                if descriptor is not None:
                    os.close(descriptor)

    def compare(self, encrypted, baseline, commands, *, rows=None, setup=None, deviation=False, source_bytes=SOURCE):
        outcomes, books, hits = [], [], []
        for runtime in ['python', 'rust', 'hybrid']:
            self.db.write_bytes(baseline)
            shutil.rmtree(self.attachments, ignore_errors=True)
            self.source.write_bytes(source_bytes)
            self.server.rows = rows if rows is not None else [base()]
            self.server.hits.clear()
            if setup:
                with closing(self.connect(encrypted)) as conn:
                    setup(conn)
                    conn.commit()
            results, states = [], []
            for index, command in enumerate(commands):
                args, options = command
                result = self.cli(runtime, args, encrypted, offset=index * 50, **options)
                results.append((result.returncode, result.stdout, result.stderr))
                if path := os.environ.get('KASSIBER_DOCUMENT_CLI_COUNTS'):
                    with open(path, 'a') as handle:
                        handle.write(json.dumps(dict(command='.'.join(args[:2]), runtime=runtime,
                            encrypted=encrypted, returncode=result.returncode)) + '\n')
                states.append(self.state(encrypted))
            outcomes.append(results)
            books.append(states)
            hits.append(list(self.server.hits))
        if not deviation:
            for actual, expected in zip(outcomes[1], outcomes[0]):
                self.assertEqual(actual, expected, f'Native: {actual!r}\nPython: {expected!r}')
        else:
            self.assertEqual(outcomes[1], [(code, out.replace(b'private-source-value', b'database operation failed'), err)
                for code, out, err in outcomes[0]])
        self.assertEqual(outcomes[2], outcomes[1])
        if books[1] != books[0]:
            for index, (actual, expected) in enumerate(zip(books[1], books[0])):
                if actual != expected:
                    changed = {name: (actual[0][1][name], expected[0][1][name])
                        for name in actual[0][1] if actual[0][1][name] != expected[0][1][name]}
                    self.fail(f'Book mismatch at {index}: {changed!r}; replies: {outcomes!r}')
        self.assertEqual(books[2], books[0])
        self.assertEqual(hits[1:], [hits[0], hits[0]])
        served = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertTrue(served)
        self.assertTrue(all(row['served_by'] == 'rust' for row in served), served)
        return outcomes[1], books[1], hits[1]

    def args(self, action='import-document', *extra):
        return ['wallets', action, *([] if action == 'preview-document' else ['--wallet', 'Source']),
            '--file', str(self.source), '--provider', 'local', *extra]

    def test_document_cli_preview_review_reimport_and_zero_fallback(self):
        for encrypted in [False, True]:
            with self.subTest(encrypted=encrypted):
                baseline = self.baseline(encrypted)
                commands = [(self.args('preview-document'), {'machine': True}),
                    (self.args(), {'interactive': True}), (self.args(), {'interactive': True})]
                results, books, hits = self.compare(encrypted, baseline, commands)
                self.assertEqual([r[0] for r in results], [0, 0, 0])
                self.assertEqual(len(books[-1][1]), 2)
                self.assertEqual(len(hits), 6)

    def test_document_cli_cancellation_selection_and_confidence_gates(self):
        for encrypted in [False, True]:
            baseline = self.baseline(encrypted)
            for name, rows, response, extra, code in [
                ('empty', [base()], b'\n', [], 1),
                ('eof', [base()], b'', [], 1),
                ('duplicate', [base(), {**base(), 'amount_btc': '0.02'}], b'2,1,2\n', [], 0),
                ('unicode', [base()], '１\n'.encode(), [], 0),
                ('invalid', [base()], b'1,\n', [], 1),
                ('unselectable', [base()], b'2\n', [], 1),
                ('confidence', [{**base(), 'confidence': 0.5}], b'1\n', [], 1),
                ('override', [{**base(), 'confidence': 0.5}], b'1\n', ['--include-quarantined'], 0),
                ('structural', [{**base(), 'amount_btc': '0.000000000015'}], b'1\n', ['--include-quarantined'], 1),
                ('default-fee', [{**base(), 'fee_btc': None}], b'1\n', [], 0),
                ('threshold', [{**base(), 'confidence': 0.5}], b'1\n', ['--confidence-threshold', '0.3'], 0),
                ('unicode-evidence', [{**base(), 'counterparty': 'Desk é', 'description': 'café😀'}], b'1\n', [], 0),
                ('surrogate-evidence', [{**base(), 'counterparty': 'Desk\ud800', 'description': 'café😀\udfff'}], b'1\n', [], 1),
            ]:
                with self.subTest(encrypted=encrypted, case=name):
                    results, _, _ = self.compare(encrypted, baseline,
                        [(self.args('import-document', *extra), {'interactive': True, 'response': response})], rows=rows)
                    self.assertEqual(results[0][0], code)

    def test_document_cli_imports_only_selected_rows(self):
        for encrypted in [False, True]:
            baseline = self.baseline(encrypted)
            results, books, _ = self.compare(encrypted, baseline,
                [(self.args(), {'interactive': True, 'response': b'2\n'})],
                rows=[base(), {**base(), 'amount_btc': '0.02'}])
            self.assertEqual(results[0][0], 0)
            self.assertEqual(len(books[0][1]), 1)

    def test_document_cli_human_formats_match_python(self):
        for encrypted in [False, True]:
            baseline = self.baseline(encrypted)
            commands = [(self.args(), {'interactive': True, 'output_format': format_})
                for format_ in [None, 'plain', 'csv']]
            results, books, _ = self.compare(encrypted, baseline, commands)
            self.assertEqual([r[0] for r in results], [0, 0, 0])
            self.assertEqual(len(books[-1][1]), 3)

    def test_document_cli_sigint_cancels_without_writes(self):
        for encrypted in [False, True]:
            baseline = self.baseline(encrypted)
            results, books, _ = self.compare(encrypted, baseline,
                [(self.args(), {'interactive': True, 'response': 'interrupt'})])
            self.assertEqual(results[0][0], 1)
            self.assertIn(b'document_import_review_cancelled', results[0][1])
            self.assertEqual(books[0][1], {})

    def test_document_cli_refuses_before_provider_contact_and_resolves_scope_first(self):
        for encrypted in [False, True]:
            baseline = self.baseline(encrypted)
            commands = [(self.args(), {'machine': True}), (self.args(), {}),
                (self.args(), {'interactive': True, 'non_interactive': True}),
                (self.args('import-document', '--wallet', 'missing'), {'machine': True}),
                (self.args('preview-document', '--profile', 'missing'), {'machine': True}),
                (self.args('preview-document', '--file', str(self.root / 'absent.png')), {'machine': True}),
                (self.args('preview-document', '--provider', 'missing'), {'machine': True})]
            results, _, hits = self.compare(encrypted, baseline, commands)
            self.assertEqual([r[0] for r in results], [1] * len(commands))
            self.assertEqual(hits, [])

    def test_document_cli_changed_source_and_atomic_database_failure(self):
        for encrypted in [False, True]:
            baseline = self.baseline(encrypted)
            results, books, _ = self.compare(encrypted, baseline, [(self.args(),
                {'interactive': True, 'mutate': lambda: self.source.write_bytes(b'changed')})])
            self.assertEqual(results[0][0], 1)
            self.assertIn(b'document_import_source_changed', results[0][1])
            self.assertEqual(books[0][1], {})

            def fail():
                with closing(self.connect(encrypted)) as conn:
                    conn.executescript("CREATE TRIGGER deny_evidence BEFORE INSERT ON attachments BEGIN SELECT RAISE(ABORT,'private-source-value'); END;")
                    conn.commit()
            results, books, _ = self.compare(encrypted, baseline,
                [(self.args(), {'interactive': True, 'mutate': fail})], deviation=True)
            self.assertEqual(results[0][0], 1)
            self.assertIn(b'database operation failed', results[0][1])
            self.assertNotIn(b'private-source-value', results[0][1])
            self.assertEqual(books[0][1], {})

    @unittest.skipUnless(shutil.which('pdfinfo') and shutil.which('pdftoppm'), 'Poppler required')
    def test_document_cli_pdf_options_and_full_book_parity(self):
        from io import BytesIO
        from reportlab.pdfgen import canvas
        for encrypted in [False, True]:
            baseline = self.baseline(encrypted)
            self.source = self.root / 'statement.pdf'
            output = BytesIO()
            document = canvas.Canvas(output, pagesize=(72, 72), invariant=1)
            for _ in range(10):
                document.showPage()
            document.save()
            rows = [{**base(), 'source_region': {'page': 9}}]
            results, books, _ = self.compare(encrypted, baseline,
                [(self.args('preview-document', '--pages', '9-10', '--max-pages', '2'), {'machine': True}),
                 (self.args('import-document', '--pages', '9-10', '--max-pages', '2'), {'interactive': True})],
                rows=rows, source_bytes=output.getvalue())
            self.assertEqual([r[0] for r in results], [0, 0])
            self.assertEqual(len(books[-1][1]), 1)
            self.source = self.root / 'receipt.png'
