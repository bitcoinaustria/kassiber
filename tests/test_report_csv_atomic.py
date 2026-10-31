"""Report CSV artifacts are replaced only once every row is written."""

from __future__ import annotations

import os
import stat
import errno
import subprocess
import sys
import threading
import json

import pytest

from kassiber.core.reports import _write_csv_rows


def _failing_rows():
    yield ["partial"]
    raise OSError("injected write failure")


def test_an_interrupted_write_keeps_the_previous_report(tmp_path):
    path = tmp_path / "report.csv"
    path.write_bytes(b"previous complete report\r\n")

    with pytest.raises(OSError, match="injected write failure"):
        _write_csv_rows(path, _failing_rows())

    assert path.read_bytes() == b"previous complete report\r\n"
    assert sorted(item.name for item in tmp_path.iterdir()) == ["report.csv"]


def test_a_new_report_gets_the_umask_default_mode(tmp_path):
    path = tmp_path / "report.csv"
    previous = os.umask(0o027)
    try:
        _write_csv_rows(path, [["a", "b"], ["1", "2"]])
    finally:
        os.umask(previous)

    assert path.read_bytes() == b"a,b\r\n1,2\r\n"
    assert stat.S_IMODE(path.stat().st_mode) == 0o640


def test_an_existing_report_keeps_its_mode(tmp_path):
    path = tmp_path / "report.csv"
    path.write_bytes(b"old\r\n")
    path.chmod(0o600)

    _write_csv_rows(path, [["new"]])

    assert path.read_bytes() == b"new\r\n"
    assert stat.S_IMODE(path.stat().st_mode) == 0o600


def test_a_final_symlink_is_written_through(tmp_path):
    target = tmp_path / "real.csv"
    target.write_bytes(b"old\r\n")
    link = tmp_path / "link.csv"
    link.symlink_to(target)

    _write_csv_rows(link, [["new"]])

    assert link.is_symlink()
    assert target.read_bytes() == b"new\r\n"


@pytest.mark.parametrize("timing", ["existing-directory", "new-directory", "replaced-file"])
@pytest.mark.parametrize("symlink", [False, True])
def test_csv_directory_error_names_only_the_requested_destination(tmp_path, timing, symlink):
    target = tmp_path / "report.csv"
    requested = target
    if symlink:
        requested = tmp_path / "link.csv"
        requested.symlink_to(target)
    messages = []
    for _ in range(2):
        if timing == "existing-directory":
            target.mkdir()
        elif timing == "replaced-file":
            target.write_bytes(b"old")

        def rows():
            if timing != "existing-directory":
                target.unlink(missing_ok=True)
                target.mkdir()
            yield ["header"]
            yield ["value"]

        with pytest.raises(OSError) as raised:
            _write_csv_rows(requested, rows())
        error = raised.value
        # Windows reports a directory open as PermissionError, while POSIX
        # reports IsADirectoryError. Keep the native category on both.
        with pytest.raises(OSError) as direct:
            requested.open("w").close()
        assert type(error) is type(direct.value)
        assert error.errno == direct.value.errno
        assert error.filename == str(requested)
        assert error.filename2 is None
        messages.append(str(error))
        assert list(target.iterdir()) == []
        assert set(tmp_path.iterdir()) == {target, requested}
        target.rmdir()
    assert messages[0] == messages[1]


@pytest.mark.skipif(os.name != 'posix', reason='POSIX devices and pipes')
def test_csv_character_devices_and_stdout_pipe():
    _write_csv_rows('/dev/null', [['discarded']])
    result = subprocess.run([sys.executable, '-c',
        "from kassiber.core.reports import _write_csv_rows; _write_csv_rows('/dev/stdout', [['a','b']])"], capture_output=True)
    assert result.returncode == 0, result.stderr
    assert result.stdout == b'a,b\r\n'


@pytest.mark.skipif(os.name != 'posix', reason='POSIX device paths')
@pytest.mark.parametrize('target', ['/dev/stdout', '/dev/null'])
def test_cli_csv_export_to_device(tmp_path, target):
    from kassiber.db import open_db
    from tests.test_custody_authored_migration import _scope, _tx, NOW
    conn = open_db(tmp_path)
    _scope(conn)
    _tx(conn, 'out', 'btc', 'outbound', 'BTC', 100000, NOW)
    conn.commit()
    conn.close()
    result = subprocess.run([sys.executable, '-m', 'kassiber', '--machine', '--data-root', str(tmp_path),
        'transactions', 'export', '--workspace', 'Books', '--profile', 'Book', '--export-format', 'csv', '--file', target],
        text=True, capture_output=True)
    assert result.returncode == 0, result.stdout + result.stderr
    if target == '/dev/stdout':
        assert 'Kassiber Transactions - Book' in result.stdout


@pytest.mark.skipif(os.name != 'posix', reason='POSIX devices and pipes')
@pytest.mark.parametrize('target', ['/dev/stdout', '/dev/null', 'fifo'])
def test_cli_reports_export_csv_skips_non_regular_artifact_registration(tmp_path, target):
    """Exercise reports export-csv, whose saved-artifact hook follows writing."""
    from kassiber.cli.handlers import process_journals
    from kassiber.db import open_db
    from tests.test_custody_authored_migration import _scope, _tx, NOW

    root = tmp_path / 'book'
    conn = open_db(root)
    try:
        _scope(conn)
        _tx(conn, 'buy', 'btc', 'inbound', 'BTC', 100000, NOW)
        conn.execute("UPDATE transactions SET fiat_rate=50000, fiat_rate_exact='50000'")
        conn.commit()
        process_journals(conn, 'Books', 'Book')
    finally:
        conn.close()
    received = []
    reader = None
    if target == 'fifo':
        fifo = tmp_path / 'report.pipe'
        os.mkfifo(fifo)
        target = str(fifo)
        def read():
            with fifo.open('rb') as stream:
                received.append(stream.read())
        reader = threading.Thread(target=read, daemon=True)
        reader.start()
    result = subprocess.run([sys.executable, '-m', 'kassiber', '--machine', '--data-root', str(root),
        'reports', 'export-csv', '--workspace', 'Books', '--profile', 'Book', '--file', target],
        text=True, capture_output=True, timeout=30)
    if reader:
        reader.join(timeout=5)
        assert not reader.is_alive()
        assert b'Kassiber Report - Book' in received[0]
        assert stat.S_ISFIFO(fifo.stat().st_mode)
    assert result.returncode == 0, result.stdout + result.stderr
    # stdout may contain the CSV followed by the ordinary machine envelope.
    payload = json.loads(result.stdout[result.stdout.rfind('\n{') + 1:])
    assert payload['data']['report_snapshot']['status'] == 'skipped'
    assert payload['data']['report_snapshot']['reason'] == 'non_regular_target'
    assert 'registration skipped' in payload['data']['report_snapshot']['message']
    if target == '/dev/stdout':
        assert 'Kassiber Report - Book' in result.stdout
    conn = open_db(root)
    try:
        assert conn.execute('SELECT COUNT(*) FROM filed_report_snapshots').fetchone()[0] == 0
    finally:
        conn.close()


@pytest.mark.skipif(not hasattr(os, 'mkfifo'), reason='requires named pipes')
def test_csv_named_pipe(tmp_path):
    fifo = tmp_path / 'report.pipe'
    os.mkfifo(fifo)
    received = []
    def read():
        with fifo.open('rb') as stream:
            received.append(stream.read())
    reader = threading.Thread(target=read, daemon=True)
    reader.start()
    try:
        _write_csv_rows(fifo, [['a', 'b']])
    finally:
        # Also unblock the reader when a pre-fix staged replacement bypasses it.
        if reader.is_alive():
            try:
                descriptor = os.open(fifo, os.O_WRONLY | os.O_NONBLOCK)
            except OSError as exc:
                assert exc.errno == errno.ENXIO
            else:
                os.close(descriptor)
        reader.join(timeout=5)
    assert stat.S_ISFIFO(fifo.stat().st_mode)
    assert received == [b'a,b\r\n']


def test_csv_hard_links_see_in_place_updates(tmp_path):
    target = tmp_path / 'report.csv'
    target.write_bytes(b'old')
    link = tmp_path / 'linked.csv'
    os.link(target, link)
    _write_csv_rows(target, [['new']])
    assert target.stat().st_ino == link.stat().st_ino
    assert link.read_bytes() == b'new\r\n'


def test_csv_long_target_name_uses_short_staged_name(tmp_path):
    target = tmp_path / ('x' * 250)
    _write_csv_rows(target, [['new']])
    assert target.read_bytes() == b'new\r\n'
    assert list(tmp_path.iterdir()) == [target]


def test_csv_writable_file_falls_back_when_sibling_cannot_be_created(tmp_path, monkeypatch):
    target = tmp_path / 'report.csv'
    target.write_bytes(b'old')
    real_open = os.open
    def refuse_staging(path, flags, *args, **kwargs):
        if flags & os.O_CREAT:
            raise PermissionError(errno.EACCES, 'directory is read-only')
        return real_open(path, flags, *args, **kwargs)
    monkeypatch.setattr(os, 'open', refuse_staging)
    _write_csv_rows(target, [['new']])
    assert target.read_bytes() == b'new\r\n'
    assert list(tmp_path.iterdir()) == [target]


@pytest.mark.parametrize('error_number', [errno.ENOSPC, getattr(errno, 'EDQUOT', errno.ENOSPC)])
def test_csv_storage_exhaustion_never_truncates_previous_export(tmp_path, monkeypatch, error_number):
    """csv_more.py: staging exhaustion must not fall back to open('w')."""
    target = tmp_path / 'report.csv'
    target.write_bytes(b'previous complete report\r\n')
    target.chmod(0o640)
    real_open = os.open
    def refuse_staging(path, flags, *args, **kwargs):
        if flags & os.O_CREAT:
            raise OSError(error_number, 'injected storage exhaustion')
        return real_open(path, flags, *args, **kwargs)
    monkeypatch.setattr(os, 'open', refuse_staging)
    consumed = []
    def rows():
        consumed.append(True)
        yield ['partial']
        raise OSError(error_number, 'write also exhausted storage')
    with pytest.raises(OSError) as error:
        _write_csv_rows(target, rows())
    assert error.value.errno == error_number
    assert target.read_bytes() == b'previous complete report\r\n'
    assert stat.S_IMODE(target.stat().st_mode) == 0o640
    assert not consumed
    assert list(tmp_path.iterdir()) == [target]


def test_csv_fsync_orders_file_replace_directory(tmp_path, monkeypatch):
    target = tmp_path / 'report.csv'
    events = []
    replace = os.replace
    def sync(fd):
        events.append('directory' if stat.S_ISDIR(os.fstat(fd).st_mode) else 'file')
    def install(source, destination):
        events.append('replace')
        replace(source, destination)
    monkeypatch.setattr(os, 'fsync', sync)
    monkeypatch.setattr(os, 'replace', install)
    _write_csv_rows(target, [['new']])
    assert events == ['file', 'replace', 'directory']


@pytest.mark.parametrize('failure', ['fdopen', 'chmod', 'fsync', 'replace'])
def test_csv_cleans_stage_on_exception(tmp_path, monkeypatch, failure):
    target = tmp_path / 'report.csv'
    target.write_bytes(b'old')
    def fail(*args, **kwargs):
        raise OSError(errno.EIO, 'injected stage failure')
    monkeypatch.setattr(os, failure, fail)
    with pytest.raises(OSError, match='injected stage failure'):
        _write_csv_rows(target, [['new']])
    assert target.read_bytes() == b'old'
    assert list(tmp_path.iterdir()) == [target]


@pytest.mark.parametrize('error_number', [errno.EINVAL, errno.EIO])
def test_csv_directory_fsync_handles_platform_limits_and_reports_io_errors(tmp_path, monkeypatch, error_number):
    target = tmp_path / 'report.csv'
    def sync(fd):
        if stat.S_ISDIR(os.fstat(fd).st_mode):
            raise OSError(error_number, 'directory fsync')
    monkeypatch.setattr(os, 'fsync', sync)
    if error_number == errno.EIO:
        with pytest.raises(OSError, match='directory fsync'):
            _write_csv_rows(target, [['new']])
    else:
        _write_csv_rows(target, [['new']])
    assert target.read_bytes() == b'new\r\n'
    assert list(tmp_path.iterdir()) == [target]
