"""R4-13's unregistered prepared-book pipeline against live Python processing."""
from contextlib import ExitStack
import itertools
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import uuid
from unittest.mock import patch

import pytest

from kassiber.cli import handlers
from kassiber.core import custody_journal, custody_filed_reports, filed_report_chain
from tests.rust_port.journal_persistence_oracle import CLOCK, dump
from tests.test_journal_storage_format import _book

ROOT = Path(__file__).resolve().parents[1]
BINARY = Path(os.environ.get("KASSIBER_TEST_JOURNAL_PROCESS_BIN", ROOT / "target/debug/examples/journal-process"))
pytestmark = pytest.mark.skipif(not BINARY.is_file(), reason="requires journal-process test-harness example")


@pytest.fixture
def prepared_book_root():
    parent = ROOT / 'target/r4-13/tests'
    parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='prepared-', dir=parent) as root:
        yield Path(root)


def scalar_exact(a, b):
    if type(a) is not type(b):
        return False
    if isinstance(a, float):
        return struct.pack('!d', a) == struct.pack('!d', b)
    if isinstance(a, list):
        return len(a) == len(b) and all(scalar_exact(x, y) for x, y in zip(a, b))
    if isinstance(a, dict):
        return a.keys() == b.keys() and all(scalar_exact(a[k], b[k]) for k in a)
    return a == b


@pytest.mark.parametrize("cipher", [False, True])
@pytest.mark.parametrize("country", ["generic", "at"])
@pytest.mark.parametrize("mode", ["unchanged", "changed", "blocked", "empty"])
def test_native_prepared_pipeline_matches_live_payload_progress_and_complete_dump(prepared_book_root, cipher, country, mode):
    tmp_path = prepared_book_root
    # The native probe explicitly accepts disposable books under target/ only.
    assert tmp_path.resolve().is_relative_to(ROOT / 'target')
    key = 'synthetic-r4-13-only' if cipher else None
    conn = _book(tmp_path / 'python', country=country, passphrase=key)
    try:
        if mode == 'changed':
            conn.execute("UPDATE transactions SET fiat_rate=12345,fiat_rate_exact='12345' WHERE id='a'")
        elif mode == 'blocked':
            conn.execute("UPDATE transactions SET fiat_rate=NULL,fiat_rate_exact=NULL WHERE id='a'")
        elif mode == 'empty':
            conn.execute('DELETE FROM transactions')
        conn.commit()
        before = dump(conn)
        profile = dict(conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone())
        path = Path(conn.execute('PRAGMA database_list').fetchone()[2])
        conn.execute('PRAGMA wal_checkpoint(TRUNCATE)').fetchall()
        native = tmp_path / 'native.sqlite'
        shutil.copy2(path, native)
        ids = []
        counter = itertools.count(1)

        def identity():
            value = uuid.UUID(int=next(counter), version=4)
            ids.append(str(value))
            return value

        original_store = custody_journal.store_ledger_state
        original_build = custody_journal.build_ledger_state

        def store(*args, **kwargs):
            with patch.object(uuid, 'uuid4', identity):
                return original_store(*args, **kwargs)

        def build(*args, **kwargs):
            # This corpus qualifies for the explicit R4-10 no-op seam only if
            # real Python repair, refresh and pricing left every scalar intact.
            assert scalar_exact(dump(conn), before)
            return original_build(*args, **kwargs)

        phases = []
        with ExitStack() as stack:
            for module in (custody_journal, custody_filed_reports, filed_report_chain):
                stack.enter_context(patch.object(module, 'now_iso', lambda: CLOCK))
            stack.enter_context(patch.object(custody_journal, 'store_ledger_state', store))
            stack.enter_context(patch.object(custody_journal, 'build_ledger_state', build))
            expected = handlers.process_journals(conn, 'w', 'p', progress_observer=phases.append)
        request = dict(database=str(native), passphrase=key, prepared_noop=True,
                       profile=profile, clock=CLOCK, ids=ids, dump=True)
        result = subprocess.run([str(BINARY)], input=json.dumps(request) + '\n', text=True,
                                capture_output=True, check=True, timeout=120)
        assert not result.stderr
        actual = json.loads(result.stdout)
        assert actual['ok'], actual
        assert actual['payload'] == expected
        assert [{'phase': p['phase']} for p in actual['phases']] == phases
        expected_dump = dump(conn)
        assert actual['dump'].keys() == expected_dump.keys()
        for table in expected_dump:
            assert scalar_exact(actual['dump'][table], expected_dump[table]), table
    finally:
        conn.close()
