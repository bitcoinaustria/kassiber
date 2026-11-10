from contextlib import closing
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import pytest

from tests import cli_runner

ROOT = Path(__file__).resolve().parents[1]
RUST_BIN = os.environ.get('KASSIBER_RUST_BIN')
VECTORS = ROOT / 'crates/wallets/tests/vectors/python_samourai_imports.json'


def cases():
    return json.loads(VECTORS.read_text())


class SamouraiReferenceDriftTest(unittest.TestCase):
    def test_samourai_vectors_match_python(self):
        from crates.wallets.scripts.generate_samourai_vectors import vectors, edge_inputs, EDGE_DEST
        self.assertEqual(cases(), vectors())
        self.assertEqual(json.loads(EDGE_DEST.read_text()), vectors(edge_inputs))


@unittest.skipUnless(RUST_BIN, 'set KASSIBER_RUST_BIN to a test-harness build')
class RustSamouraiImportTest(unittest.TestCase):
    def setUp(self):
        from tests.test_rust_shell_parity import RustRealStoreParityTest
        self.host = RustRealStoreParityTest()
        self.host.setUp()
        self.addCleanup(self.host.tearDown)
        if 'HOME' in os.environ:
            self.host.env['HOME'] = os.environ['HOME']
        else:
            self.host.env.pop('HOME', None)
        self.host.import_run_book()
        self.database = self.host.root / 'kassiber.sqlite3'
        self.file = self.host.home / 'samourai.json'
        self.fixed_time = '2025-01-02T03:04:05Z'

    def write(self, case):
        if case is None:
            self.file.unlink(missing_ok=True)
        else:
            self.file.write_bytes(bytes(case['bytes']) if 'bytes' in case else case['text'].encode())

    def arguments(self, case, label='Samourai', extra=()):
        args = ['wallets', 'import-samourai', '--label', label, '--source-set-file', str(self.file)]
        if case is not None:
            args += ['--network', case['network']]
            if case['gap_limit'] is not None:
                args += ['--gap-limit', str(case['gap_limit'])]
        return [*args, *extra]

    def dump(self, encrypted):
        from kassiber.secrets import sqlcipher
        from tests.test_rust_store_migrations import normalized_dump
        with closing(sqlcipher.open_encrypted(self.database, self.host.PASSPHRASE,
                enforce_operator_identity=False) if encrypted else sqlite3.connect(self.database)) as conn:
            return normalized_dump(conn)

    def wire(self, selected, *, indices=None, check=None):
        """Compare each scenario's raw wire and full books on both codecs.

        ``indices`` name the scenarios by their place in the complete list
        when ``selected`` is a slice of it; ``check`` replaces the unittest
        subtest assertions for pytest callers.
        """
        if check is None:
            def check(encrypted, index, name, wires, books):
                with self.subTest(encrypted=encrypted, scenario=index, name=name):
                    self.assertEqual(wires[1], wires[0])
                    self.assertEqual(books[1], books[0])
        indices = list(range(len(selected))) if indices is None else list(indices)
        for encrypted in [False, True]:
            secret = self.host.PASSPHRASE if encrypted else None
            if encrypted:
                self.host.ok(self.host.call(False, ['secrets', 'init'], secret=secret, new_secret=True))
            baseline = self.database.read_bytes()
            for index, scenario in zip(indices, selected):
                steps = scenario if isinstance(scenario, list) else [(scenario, 'Samourai', ())]
                wires, books = [], []
                for rust in [False, True]:
                    self.database.write_bytes(baseline)
                    outputs, dumps = [], []
                    for step, (case, label, extra) in enumerate(steps):
                        identities = [f'00000000-0000-4000-8000-{step * 100 + i:012}' for i in range(1, 31)]
                        self.host.env.update(KASSIBER_RUST_TEST_IMPORT_IDS=','.join(identities),
                            KASSIBER_RUST_TEST_IMPORT_TIME=self.fixed_time)
                        self.write(case)
                        args = self.arguments(case, label, extra)
                        if rust:
                            result = self.host.call(True, args, secret=secret)
                        else:
                            script = (
                                'import sys; from pathlib import Path; from types import SimpleNamespace; '
                                'from kassiber.operator import project; '
                                'root=Path(sys.argv.pop(1)); project._owner_lock_root=lambda:root; '
                                'from kassiber.core import samourai; '
                                f'ids=iter({identities!r}); samourai.uuid=SimpleNamespace(uuid4=lambda:next(ids)); '
                                f'samourai.now_iso=lambda:{self.fixed_time!r}; '
                                'from kassiber.cli.main import main; raise SystemExit(main(sys.argv[1:]))'
                            )
                            with tempfile.TemporaryFile() as fd, mock.patch.object(cli_runner, 'prefix',
                                    return_value=(sys.executable, '-c', script, str(self.host.owners))):
                                auth = []
                                if secret:
                                    fd.write(secret.encode()); fd.seek(0)
                                    auth = ['--db-passphrase-fd', str(fd.fileno())]
                                result = cli_runner.run(['--machine', *auth, '--data-root', str(self.host.root), *args],
                                    reference=True, cwd=ROOT, env=self.host.env, capture_output=True,
                                    timeout=120, pass_fds=(fd.fileno(),) if secret else ())
                        outputs.append((result.returncode, result.stdout, result.stderr))
                        json.loads(result.stdout)
                        dumps.append(self.dump(encrypted))
                    wires.append(outputs)
                    books.append(dumps)
                check(encrypted, index, steps[0][0]['name'] if steps[0][0] else 'missing', wires, books)
            self.database.write_bytes(baseline)

    def test_coordinator_reimports_overlaps_refusals_and_missing_files_match_wire_and_books(self):
        named = {case['name']: case for case in cases()}
        mixed = named['mixed']
        self.wire([
            [(mixed, 'Group', ('--backend', ' MEMPOOL ')),
             (mixed, 'Group', ()),
             (mixed, 'Overlap', ()),
             (named['duplicate-labels'], 'Duplicates', ()),
             (named['distinct-labels'], 'Distinct', ()),
             (named['late-bad-source'], 'Failure', ()),
             (mixed, '', ()),
             (mixed, 'Missing account', ('--account', 'missing')),
             (mixed, 'Missing profile', ('--profile', 'missing')),
             (None, 'Missing file', ('--profile', 'missing')),
             (named['invalid-json'], 'Bad file', ('--profile', 'missing'))],
        ])

    def hybrid_env(self, log):
        return self.host.env | {
            'KASSIBER_HYBRID_RUST_BIN': str(Path(RUST_BIN).resolve()),
            'KASSIBER_HYBRID_LOG': str(log),
            'KASSIBER_HYBRID_IMPORT_ORACLE': '1',
        }

    def hybrid(self, selected, log, *, subtest):
        """Run each case through the hybrid CLI from the same baseline book."""
        env = self.hybrid_env(log)
        baseline = self.database.read_bytes()
        for case in selected:
            with subtest(name=case['name']):
                self.database.write_bytes(baseline)
                self.write(case)
                result = subprocess.run([sys.executable, str(ROOT / 'tests/rust_port/hybrid_cli.py'),
                    '--machine', '--data-root', str(self.host.root), *self.arguments(case)],
                    env=env, capture_output=True, timeout=120)
                self.assertIn(result.returncode, (0, 1), (result.stdout, result.stderr))
                json.loads(result.stdout)
        self.database.write_bytes(baseline)

    def test_codepoints_match_raw_wire_and_books(self):
        from crates.wallets.scripts.generate_samourai_vectors import EDGE_DEST
        self.wire(json.loads(EDGE_DEST.read_text()))

    def test_codepoints_native_hybrid_routing(self):
        from crates.wallets.scripts.generate_samourai_vectors import EDGE_DEST
        edges = json.loads(EDGE_DEST.read_text())
        baseline = self.database.read_bytes()
        log = self.host.home / 'codepoints-served.jsonl'
        env = self.hybrid_env(log)
        for case in edges:
            self.database.write_bytes(baseline)
            self.write(case)
            result = subprocess.run([sys.executable, str(ROOT / 'tests/rust_port/hybrid_cli.py'),
                '--machine', '--data-root', str(self.host.root), *self.arguments(case)],
                env=env, capture_output=True, timeout=120)
            self.assertIn(result.returncode, (0, 1), (result.stdout, result.stderr))
            json.loads(result.stdout)
        self.assertEqual([json.loads(line) for line in log.read_text().splitlines()],
            [{'command': 'wallets.import-samourai', 'served_by': 'rust'}] * len(edges))


# The two corpus-wide comparisons run in slices, so xdist spreads the 518
# cases instead of one worker replaying them all. Each slice builds the same
# book as the unittest class and replays its cases in order from it; the
# per-case subtests still name every case.
SLICES = 16


@pytest.fixture
def samourai():
    if not RUST_BIN:
        pytest.skip('set KASSIBER_RUST_BIN to a test-harness build')
    test = RustSamouraiImportTest()
    test.setUp()
    try:
        yield test
    finally:
        test.doCleanups()


@pytest.mark.parametrize('part', range(SLICES), ids=lambda part: f'slice{part:02}')
def test_reference_files_match_raw_cli_wire_and_full_books_on_both_codecs(samourai, subtests, part):
    def check(encrypted, index, name, wires, books):
        with subtests.test(encrypted=encrypted, scenario=index, name=name):
            assert wires[1] == wires[0]
            assert books[1] == books[0]
    indices = range(part, len(cases()), SLICES)
    samourai.wire(cases()[part::SLICES], indices=indices, check=check)


@pytest.mark.parametrize('part', range(SLICES), ids=lambda part: f'slice{part:02}')
def test_hybrid_reference_files_have_zero_importer_fallbacks(samourai, subtests, part):
    selected = cases()[part::SLICES]
    log = samourai.host.home / 'served.jsonl'
    samourai.hybrid(selected, log, subtest=subtests.test)
    rows = [json.loads(line) for line in log.read_text().splitlines()]
    assert rows == [{'command': 'wallets.import-samourai', 'served_by': 'rust'}] * len(selected)


def test_hybrid_coordinator_calls_have_zero_importer_fallbacks(samourai):
    log = samourai.host.home / 'served.jsonl'
    env = samourai.hybrid_env(log)
    mixed = next(case for case in cases() if case['name'] == 'mixed')
    samourai.write(mixed)
    for label, extra in [('Group', ()), ('Group', ()), ('Overlap', ()),
                         ('Account', ('--account', 'missing')), ('Profile', ('--profile', 'missing'))]:
        result = subprocess.run([sys.executable, str(ROOT / 'tests/rust_port/hybrid_cli.py'),
            '--machine', '--data-root', str(samourai.host.root), *samourai.arguments(mixed, label, extra)],
            env=env, capture_output=True, timeout=120)
        assert result.returncode in (0, 1), (result.stdout, result.stderr)
        json.loads(result.stdout)
    rows = [json.loads(line) for line in log.read_text().splitlines()]
    assert rows == [{'command': 'wallets.import-samourai', 'served_by': 'rust'}] * 5
