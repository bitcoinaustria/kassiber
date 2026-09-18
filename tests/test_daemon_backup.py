"""Desktop backups use the real SQLCipher/age engine and scoped confirmation."""
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock

import pytest

from kassiber import daemon_backup as backup
from kassiber.backup import install
from kassiber.core.accounts import create_profile, create_workspace
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.operator.project import acquire_project_ownership, canonical_project


@pytest.fixture
def desktop(tmp_path, monkeypatch):
    from kassiber.operator import project
    owner_root = tmp_path / "owner-locks"
    owner_root.mkdir(mode=0o700)
    monkeypatch.setattr(project, "_owner_lock_root", lambda: owner_root)
    pytest.importorskip("sqlcipher3")
    pytest.importorskip("pyrage")
    root = tmp_path / "original" / "data"
    conn = open_db(root, passphrase="database-secret")
    workspace = create_workspace(conn, "Original set")
    create_profile(conn, workspace["id"], "Original book", "EUR", "FIFO", "generic", 365)
    conn.commit()
    attachments = root.parent / "attachments"
    attachments.mkdir(exist_ok=True)
    (attachments / "invoice.txt").write_text("retained evidence")
    ctx = SimpleNamespace(conn=conn, data_root=str(root), db_passphrase="database-secret", project_id="original", backup_sessions=backup.BackupSessions(), owner=None)

    def close(guard):
        guard()
        ctx.conn.execute("PRAGMA wal_checkpoint(TRUNCATE)")
        ctx.conn.close()
        ctx.conn = None
        ctx.db_passphrase = None

    def ensure():
        if ctx.owner is not None:
            return
        ctx.owner = acquire_project_ownership(canonical_project(ctx.data_root), owner_kind="desktop", generation="test-backup")

    def release():
        ctx.owner.release()
        ctx.owner = None

    callbacks = dict(close_database=close, ensure_owner=ensure, release_owner=release, invalidate_credentials=Mock())
    yield ctx, callbacks, tmp_path / "archive.kassiber"
    ctx.backup_sessions.clear()
    if ctx.conn is not None:
        ctx.conn.close()
    if ctx.owner is not None:
        ctx.owner.release()


def call(desktop, kind, **args):
    ctx, callbacks, _ = desktop
    return backup.handle_backup(ctx, "ui.backup." + kind, args, **callbacks)


def exported(desktop):
    _, _, archive = desktop
    call(desktop, "export", path=str(archive), backup_passphrase_secret="outer-secret")
    return archive


def preview(desktop, archive):
    return call(desktop, "preview", path=str(archive), backup_passphrase_secret="outer-secret", database_passphrase_secret="database-secret")


def test_native_roundtrip_preview_scope_and_retained_recovery(desktop):
    ctx, callbacks, _ = desktop
    archive = exported(desktop)
    assert b"retained evidence" not in archive.read_bytes()
    ctx.conn.execute("UPDATE profiles SET label='Changed book'")
    ctx.conn.commit()
    shown = preview(desktop, archive)
    assert shown["incoming"]["books"] == ["Original book"]
    assert shown["target"]["books"] == ["Changed book"]
    assert shown["attachments_files"] == 1
    assert shown["target_data_root"] == str(Path(ctx.data_root).resolve())
    assert "outer-secret" not in str(shown)
    assert "database-secret" not in str(shown)
    assert not hasattr(ctx.backup_sessions.preview, "passphrase")
    stage = ctx.backup_sessions.preview.staging
    # The selected source is now irrelevant: apply uses the validated snapshot.
    archive.write_bytes(b"source was replaced after preview")
    with pytest.raises(AppError, match="Confirm"):
        call(desktop, "apply", token=shown["token"])
    result = call(desktop, "apply", token=shown["token"], confirm="RESTORE")
    assert result["restored"] and result["locked"] and ctx.conn is None
    assert not stage.exists()
    callbacks["invalidate_credentials"].assert_called_once()
    restored = open_db(ctx.data_root, passphrase="database-secret")
    assert restored.execute("SELECT label FROM profiles").fetchone()[0] == "Original book"
    restored.close()
    recovery = Path(result["pre_restore_backup"])
    old = open_db(recovery, passphrase="database-secret")
    assert old.execute("SELECT label FROM profiles").fetchone()[0] == "Changed book"
    old.close()
    assert (Path(ctx.data_root).parent / "attachments/invoice.txt").read_text() == "retained evidence"


@pytest.mark.parametrize("failure", ["wrong_archive_password", "wrong_database_password", "invalid_archive"])
def test_preview_failure_preserves_live_container(desktop, failure):
    ctx, _, _ = desktop
    archive = exported(desktop)
    args = dict(path=str(archive), backup_passphrase_secret="outer-secret", database_passphrase_secret="database-secret")
    if failure == "invalid_archive":
        archive.write_bytes(b"invalid")
    elif failure == "wrong_archive_password":
        args["backup_passphrase_secret"] = "wrong"
    else:
        args["database_passphrase_secret"] = "wrong"
    with pytest.raises(AppError) as exc:
        call(desktop, "preview", **args)
    assert exc.value.code == "invalid_backup"
    assert ctx.backup_sessions.preview is None
    assert ctx.conn.execute("SELECT label FROM profiles").fetchone()[0] == "Original book"


@pytest.mark.parametrize("change", ["write", "switch", "expiry", "staging"])
def test_apply_rejects_stale_or_tampered_preview(desktop, change):
    ctx, _, _ = desktop
    shown = preview(desktop, exported(desktop))
    stage = ctx.backup_sessions.preview.staging
    if change == "write":
        ctx.conn.execute("UPDATE profiles SET label='Changed'")
        ctx.conn.commit()
    elif change == "switch":
        ctx.project_id = "another-container"
    elif change == "expiry":
        ctx.backup_sessions.preview.expires_at = 0
    else:
        (stage / "attachments/invoice.txt").write_text("tampered")
    with pytest.raises(AppError) as exc:
        call(desktop, "apply", token=shown["token"], confirm="RESTORE")
    assert exc.value.code == "stale_backup_preview"
    assert ctx.conn is not None
    assert not stage.exists()


def test_cancel_removes_staging_without_changing_book(desktop):
    ctx, _, _ = desktop
    shown = preview(desktop, exported(desktop))
    stage = ctx.backup_sessions.preview.staging
    call(desktop, "cancel", token=shown["token"])
    assert not stage.parent.exists()
    assert ctx.conn.execute("SELECT label FROM profiles").fetchone()[0] == "Original book"


def test_write_while_quiescing_is_rejected_before_close(desktop):
    ctx, callbacks, _ = desktop
    shown = preview(desktop, exported(desktop))
    close = callbacks["close_database"]

    def racing_close(guard):
        ctx.conn.execute("UPDATE profiles SET label='Late write'")
        ctx.conn.commit()
        close(guard)

    callbacks["close_database"] = racing_close
    with pytest.raises(AppError) as exc:
        call(desktop, "apply", token=shown["token"], confirm="RESTORE")
    assert exc.value.code == "stale_backup_preview"
    assert ctx.conn.execute("SELECT label FROM profiles").fetchone()[0] == "Late write"


def test_export_destination_failure_preserves_existing_file(desktop, monkeypatch):
    _, _, archive = desktop
    archive.write_bytes(b"previous backup")
    from kassiber.backup import pack
    monkeypatch.setattr(pack, "encrypt_age_stream", Mock(side_effect=OSError("destination full")))
    with pytest.raises(AppError, match="could not be saved"):
        call(desktop, "export", path=str(archive), backup_passphrase_secret="outer-secret")
    assert archive.read_bytes() == b"previous backup"
    assert not list(archive.parent.glob(".archive.kassiber.*"))


def test_export_cannot_replace_active_container_files(desktop):
    ctx, _, _ = desktop
    with pytest.raises(AppError) as exc:
        call(desktop, "export", path=str(Path(ctx.data_root) / "backup.kassiber"), backup_passphrase_secret="outer-secret")
    assert exc.value.code == "invalid_backup_destination"


def test_install_failure_rolls_back_database_attachments_and_exports(desktop, monkeypatch):
    ctx, _, _ = desktop
    root = Path(ctx.data_root).parent
    (root / "exports").mkdir()
    (root / "exports/report.csv").write_text("old report")
    shown = preview(desktop, exported(desktop))
    replace = install.os.replace

    def fail_install(source, target):
        if ".restore-install-" in str(source) and Path(target).name == "attachments":
            raise OSError("injected install failure")
        replace(source, target)

    monkeypatch.setattr(install.os, "replace", fail_install)
    with pytest.raises(AppError) as exc:
        call(desktop, "apply", token=shown["token"], confirm="RESTORE")
    assert exc.value.code == "restore_install_failed"
    assert exc.value.details["locked"] is True
    assert ctx.conn is None
    restored = open_db(ctx.data_root, passphrase="database-secret")
    assert restored.execute("SELECT label FROM profiles").fetchone()[0] == "Original book"
    restored.close()
    assert (root / "attachments/invoice.txt").read_text() == "retained evidence"
    assert (root / "exports/report.csv").read_text() == "old report"


def test_real_daemon_dispatch_closes_workers_and_leaves_restored_database_locked(desktop, monkeypatch):
    import io
    import queue
    import threading
    from kassiber import daemon
    from kassiber.secrets.auth_backoff import AuthAttemptBackoff

    source, _, archive = desktop
    exported(desktop)
    ctx = daemon.DaemonContext(
        conn=source.conn, data_root=source.data_root, runtime_config={},
        active_ai_chats=daemon.ActiveAiChats(), main_thread_tasks=queue.Queue(),
        auth_backoff=AuthAttemptBackoff(str(Path(source.data_root).parent / "config/auth.json")),
        input_lines=queue.Queue(), deferred_input_lines=[], out=io.StringIO(),
        freshness_stop_event=threading.Event(), db_passphrase="database-secret",
    )
    stopped = []
    monkeypatch.setattr(daemon, "_stop_watch_worker", lambda *_args, **_kwargs: stopped.append("watches"))
    monkeypatch.setattr(daemon, "_stop_freshness_background_worker", lambda *_args, **_kwargs: stopped.append("freshness"))
    monkeypatch.setattr(daemon, "mark_desktop_biometric_passphrase_stale", Mock())
    monkeypatch.setattr(daemon, "invalidate_operator_native_auth", Mock())
    monkeypatch.setattr(daemon, "disable_remembered_unlock", Mock())
    reply, _ = daemon.handle_request(ctx, {"kind": "ui.backup.preview", "request_id": "preview", "args": {"path": str(archive), "backup_passphrase_secret": "outer-secret", "database_passphrase_secret": "database-secret"}}, ctx.out)
    assert reply["kind"] == "ui.backup.preview"
    reply, _ = daemon.handle_request(ctx, {"kind": "ui.backup.apply", "request_id": "apply", "args": {"token": reply["data"]["token"], "confirm": "RESTORE"}}, ctx.out)
    assert reply["request_id"] == "apply"
    assert reply["data"]["restored"] and reply["data"]["locked"]
    assert reply["data"]["warning"] is None
    assert stopped == ["watches", "freshness"]
    assert ctx.conn is None and ctx.db_passphrase is None and ctx.project_owner is None
    source.conn = None


def test_restore_target_symlink_is_rejected_without_modifying_target(tmp_path):
    root = tmp_path / "container"
    root.mkdir()
    target = root / "data"
    target.mkdir()
    (target / "kassiber.sqlite3").write_bytes(b"original")
    external = tmp_path / "outside"
    external.mkdir()
    (root / "attachments").symlink_to(external, target_is_directory=True)
    stage = tmp_path / "stage"
    stage.mkdir()
    (stage / "kassiber.sqlite3").write_bytes(b"replacement")
    with pytest.raises(AppError) as exc:
        install.install_staged_backup(stage, target)
    assert exc.value.code == "unsafe_restore_target"
    assert (target / "kassiber.sqlite3").read_bytes() == b"original"
    assert not list(external.iterdir())


def test_restore_waits_for_worker_connections_even_after_receipts_are_cleared(desktop, monkeypatch):
    import threading
    from kassiber.core.chain_analysis_runtime import AnalysisJobs
    from kassiber.secrets.sqlcipher import open_encrypted

    ctx, _, _ = desktop
    shown = preview(desktop, exported(desktop))
    jobs = AnalysisJobs()
    monkeypatch.setattr(backup, "JOBS", jobs)
    entered, release, closed = threading.Event(), threading.Event(), threading.Event()

    def compute(progress, cancelled):
        conn = open_encrypted(Path(ctx.data_root) / "kassiber.sqlite3", "database-secret",
                              enforce_operator_identity=False)
        try:
            conn.execute("SELECT COUNT(*) FROM profiles").fetchone()
            entered.set()
            assert release.wait(10)
            return {"status": "cancelled" if cancelled() else "completed"}
        finally:
            conn.close()
            closed.set()

    jobs.start(b"original", {}, compute)
    try:
        assert entered.wait(5)
        jobs.clear()
        # A cleared receipt cannot hide its live SQLCipher connection.
        with pytest.raises(AppError) as error:
            call(desktop, "apply", token=shown["token"], confirm="RESTORE")
        assert error.value.code == "project_in_use"
        assert not closed.is_set() and ctx.conn is not None
        assert ctx.backup_sessions.preview is not None
        assert ctx.conn.execute("SELECT label FROM profiles").fetchone()[0] == "Original book"
    finally:
        release.set()
        with jobs.quiesce():
            assert closed.is_set()
            with pytest.raises(AppError) as error:
                jobs.start(b"new", {}, compute)
            assert error.value.code == "chain_analysis_busy"
    shown = preview(desktop, exported(desktop))
    result = call(desktop, "apply", token=shown["token"], confirm="RESTORE")
    assert result["restored"] and closed.is_set()
