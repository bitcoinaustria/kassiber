from types import SimpleNamespace
import threading

import pytest

from kassiber import daemon_accounting_documents as jobs
from kassiber.core.accounting import document_text, evidence
from kassiber.errors import AppError
from tests.test_accounting_integration import book  # noqa: F401


@pytest.fixture
def prepared(book):
    conn, profile, root = book
    source = evidence.retain_evidence(conn, profile, content=b"A retained synthetic document",
        name="Synthetic", media_type="text/plain")
    ctx = SimpleNamespace(conn=conn, data_root=str(root), ownership_generation="original")
    manager = jobs.DocumentJobs()
    args = {"profile_id": profile, "payload": {"evidence_id": source["id"]}}
    output = []
    out = SimpleNamespace(write=output.append)
    return ctx, manager, args, out, output


def test_worker_returns_scoped_result_and_only_main_thread_persists(prepared, monkeypatch):
    ctx, manager, args, out, output = prepared
    main_ident = threading.get_ident()
    original = document_text.extract_bytes
    def parser(*a, **kw):
        assert threading.get_ident() != main_ident
        return original(*a, **kw)
    monkeypatch.setattr(document_text, "extract_bytes", parser)
    manager.start(ctx, "extract-one", args)
    job = manager.jobs["extract-one"]
    assert job.done.wait(2)
    assert ctx.conn.execute("SELECT COUNT(*) FROM gl_evidence_extractions").fetchone()[0] == 0
    manager.poll(ctx, out)
    assert output[0]["kind"] == "ui.accounting.document_extract"
    assert output[0]["request_id"] == "extract-one"
    assert output[0]["data"]["profile_id"] == args["profile_id"]
    assert output[0]["data"]["pages"] == ["A retained synthetic document"]
    assert not manager.jobs
    assert job.result is None


@pytest.mark.parametrize("change", ["cancel", "lock", "project", "generation"])
def test_cancel_and_scope_change_prevent_persistence_and_clear_results(prepared, monkeypatch, change):
    ctx, manager, args, out, output = prepared
    started = threading.Event()
    def parser(content, media_type, *, cancel):
        started.set()
        assert cancel.wait(2), "parser cancellation was not delivered"
        # Even a parser returning plaintext after cancellation cannot retain it.
        return (["Must not persist"], "synthetic", "1")
    monkeypatch.setattr(document_text, "extract_bytes", parser)
    manager.start(ctx, "extract-one", args)
    job = manager.jobs["extract-one"]
    assert started.wait(2)
    original = ctx.conn
    if change == "cancel":
        manager.cancel(ctx, args)
    elif change == "lock":
        ctx.conn = None
    elif change == "project":
        ctx.data_root = "different-project"
    else:
        ctx.ownership_generation = "different-generation"
    manager.poll(ctx, out)
    assert job.done.wait(2)
    manager.poll(ctx, out)
    assert output[0]["error"]["code"] == "accounting_document_cancelled"
    assert "Must not persist" not in str(output)
    assert original.execute("SELECT COUNT(*) FROM gl_evidence_extractions").fetchone()[0] == 0
    assert job.result is None and not manager.jobs


def test_only_one_worker_and_wrong_book_cannot_cancel(prepared, monkeypatch):
    ctx, manager, args, out, output = prepared
    def parser(content, media_type, *, cancel):
        cancel.wait(2)
        raise AppError("synthetic", code="accounting_document_cancelled")
    monkeypatch.setattr(document_text, "extract_bytes", parser)
    manager.start(ctx, "one", args)
    with pytest.raises(AppError) as exc:
        manager.start(ctx, "two", args)
    assert exc.value.code == "accounting_document_busy"
    with pytest.raises(AppError):
        manager.cancel(ctx, dict(args, profile_id="other"))
    assert not manager.jobs["one"].cancel.is_set()
    manager.cancel_all()
    assert manager.jobs["one"].done.wait(2)
    manager.poll(ctx, out)


def test_parser_failure_is_generic_and_does_not_log_document_text(prepared, monkeypatch):
    ctx, manager, args, out, output = prepared
    def parser(*a, **kw):
        raise RuntimeError("secret document bytes must never be a diagnostic")
    monkeypatch.setattr(document_text, "extract_bytes", parser)
    manager.start(ctx, "one", args)
    assert manager.jobs["one"].done.wait(2)
    manager.poll(ctx, out)
    assert output[0]["error"]["code"] == "accounting_document_parse_failed"
    assert "secret document" not in str(output)


def test_malformed_request_never_launches_worker(prepared):
    ctx, manager, args, out, output = prepared
    for malformed in ({}, {**args, "path": "/no/file"}, dict(args, profile_id="other")):
        with pytest.raises(AppError):
            manager.start(ctx, "one", malformed)
    assert not manager.jobs


@pytest.mark.parametrize('identity', [None, [], {}, True, 42, '', 'x\0y', 'x' * 201])
def test_malformed_evidence_ids_fail_typed_before_worker_or_cancel(prepared, identity):
    ctx, manager, args, out, output = prepared
    invalid = {**args, 'payload': {'evidence_id': identity}}
    with pytest.raises(AppError):
        manager.start(ctx, 'one', invalid)
    with pytest.raises(AppError):
        manager.cancel(ctx, invalid)
    assert not manager.jobs


def test_shutdown_joins_parser_without_persistence_or_db_access(prepared, monkeypatch):
    ctx, manager, args, out, output = prepared
    started = threading.Event()
    def parser(content, media_type, *, cancel):
        started.set()
        assert cancel.wait(2)
        return (['Must be discarded during shutdown'], 'fixture', '1')
    monkeypatch.setattr(document_text, 'extract_bytes', parser)
    manager.start(ctx, 'one', args)
    job = manager.jobs['one']
    assert started.wait(2)
    ctx.conn = None  # Shutdown must not need a usable book/context.
    manager.shutdown()
    assert job.thread is not None and not job.thread.is_alive()
    assert job.result is None and not manager.jobs and not output


def test_shutdown_timeout_is_explicit_not_success(prepared, monkeypatch):
    ctx, manager, args, out, output = prepared
    release = threading.Event()
    monkeypatch.setattr(document_text, 'extract_bytes', lambda *a, **kw: (release.wait(2), 'fixture', '1'))
    manager.start(ctx, 'one', args)
    try:
        with pytest.raises(AppError) as error:
            manager.shutdown(timeout=.01)
        assert error.value.code == 'accounting_document_shutdown_timeout'
        assert manager.jobs['one'].cancel.is_set()
    finally:
        release.set()
        manager.shutdown()


@pytest.mark.skipif(__import__('os').name != 'posix', reason='Verified POSIX process-group worker')
def test_real_parser_child_reaped_before_shutdown_returns(prepared, monkeypatch):
    import subprocess
    import sys
    ctx, manager, args, out, output = prepared
    launched = threading.Event()
    children = []
    original = subprocess.Popen
    def launch(*args, **kwargs):
        process = original(*args, **kwargs)
        children.append(process)
        launched.set()
        return process
    monkeypatch.setattr(document_text, '_worker_command', lambda *_: [sys.executable, '-c', 'import time; time.sleep(20)'])
    monkeypatch.setattr(document_text.subprocess, 'Popen', launch)
    monkeypatch.setattr(document_text, 'extract_bytes', lambda content, media_type, *, cancel:
        (*document_text._run_worker('text', [], content, cancel, timeout=30), 'fixture'))
    manager.start(ctx, 'one', args)
    assert launched.wait(2)
    try:
        manager.shutdown()
        assert children[0].poll() is not None
        assert not manager.jobs and not output
        assert ctx.conn.execute('SELECT COUNT(*) FROM gl_evidence_extractions').fetchone()[0] == 0
    finally:
        if children[0].poll() is None:
            children[0].kill()
            children[0].wait()


def test_already_cancelled_worker_never_spawns(monkeypatch):
    cancel = threading.Event()
    cancel.set()
    monkeypatch.setattr(document_text.subprocess, 'Popen', lambda *a, **kw: pytest.fail('cancelled worker spawned'))
    with pytest.raises(AppError) as error:
        document_text._run_worker('text', [], b'synthetic', cancel, timeout=30)
    assert error.value.code == 'accounting_document_cancelled'


@pytest.mark.skipif(__import__('os').name != 'posix', reason='Verified POSIX process-group worker')
def test_cancel_during_popen_never_sends_document_bytes(monkeypatch):
    from unittest.mock import Mock
    cancel = threading.Event()
    process = Mock(pid=1234)
    def launch(*a, **kw):
        cancel.set()
        return process
    monkeypatch.setattr(document_text.subprocess, 'Popen', launch)
    kill = Mock()
    monkeypatch.setattr(document_text.os, 'killpg', kill)
    with pytest.raises(AppError) as error:
        document_text._run_worker('text', [], b'sensitive synthetic document', cancel, timeout=30)
    assert error.value.code == 'accounting_document_cancelled'
    kill.assert_called_once()
    process.communicate.assert_called_once_with()  # Reaping only, never input bytes.


@pytest.mark.skipif(__import__('os').name != 'posix', reason='Verified POSIX process-group worker')
def test_normal_parent_exit_after_shutdown_leaves_no_parser_child():
    import os
    import signal
    import subprocess
    import sys
    script = '''import subprocess, sys, threading
from kassiber import daemon_accounting_documents as documents
from kassiber.core.accounting import document_text
manager = documents.DocumentJobs()
job = documents._Job('shutdown-test', (), {})
manager.jobs[job.request_id] = job
started = threading.Event()
original = subprocess.Popen
def launch(*args, **kwargs):
    child = original(*args, **kwargs)
    print(child.pid, flush=True)
    started.set()
    return child
document_text._worker_command = lambda *_: [sys.executable, '-c', 'import time; time.sleep(20)']
document_text.subprocess.Popen = launch
def work():
    try: document_text._run_worker('text', [], b'synthetic', job.cancel, timeout=30)
    except Exception: pass
    finally: job.done.set()
job.thread = threading.Thread(target=work, daemon=True)
job.thread.start()
assert started.wait(2)
manager.shutdown()
'''
    result = subprocess.run([sys.executable, '-c', script], capture_output=True, text=True, timeout=5)
    pid = int(result.stdout.strip())
    try:
        assert result.returncode == 0, result.stderr
        with pytest.raises(ProcessLookupError):
            os.kill(pid, 0)
    finally:
        try:
            os.killpg(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
