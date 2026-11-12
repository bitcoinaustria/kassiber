"""Byte-level desktop backup JSONL oracle, with disposable containers only.

The harness injects OS randomness and the private restore suffix and waits for
the real local-watch database startup. No response fields, separators or
progress records are normalized.
"""
from collections import Counter
from contextlib import contextmanager
import io
import json
import os
from pathlib import Path
import select
import shutil
import subprocess
import sys
import tarfile

import pyrage.passphrase
import pytest

from kassiber.backup.pack import export_backup
from kassiber.core.accounts import create_profile, create_workspace
from kassiber.db import ensure_schema_compat
from kassiber.operator import project as ownership
from unittest.mock import patch

from kassiber.secrets.sqlcipher import open_encrypted, get_row_class
from tests import cli_runner
from tests.test_rust_backup_services import OUTER, _bundle, _manifest, _source
from tests.test_rust_secrets_services import KEY, cli, ok  # noqa: F401
from tests.test_rust_store_migrations import normalized_dump

pytestmark = pytest.mark.skipif(not os.environ.get("KASSIBER_RUST_BIN"), reason="requires the native harness")
TOKEN = "A" * 43
SUFFIX = "OracleRestore"
COUNTS = Counter()


@pytest.fixture
def desktop(cli, tmp_path, monkeypatch):
    state = _source(cli)
    conn = open_encrypted(cli.data / "kassiber.sqlite3", KEY, enforce_operator_identity=False, row_factory=get_row_class())
    workspace = create_workspace(conn, "Incoming set")
    create_profile(conn, workspace["id"], "Incoming book", "EUR", "FIFO", "generic", 365)
    conn.commit()
    incoming = normalized_dump(conn)
    conn.close()
    archive = tmp_path / "archive.kassiber"
    export_backup(cli.data, archive, KEY, backup_passphrase=OUTER)
    conn = open_encrypted(cli.data / "kassiber.sqlite3", KEY, enforce_operator_identity=False, row_factory=get_row_class())
    conn.execute("UPDATE profiles SET label='Target book'")
    # Settle the page layout before the snapshot. Python's compatibility pass
    # drops and recreates current triggers, views and indexes on every open;
    # Rust skips that redundant DDL (c17539b4f). On a freshly converted file
    # the first Python pass can move the schema table onto another page, so
    # the exported archives would differ in size, not in book data. Page
    # layout is not book data; test_export_success_and_io_refusal_match_raw_jsonl
    # compares the exported databases' contents separately.
    ensure_schema_compat(conn)
    conn.commit()
    conn.close()
    snapshot = tmp_path / "snapshot"
    shutil.copytree(state, snapshot)
    # Keep one database inode across both runs: owner refusal envelopes expose
    # its hashed identity. Reset its bytes, without masking that wire field.
    identity = tmp_path / "target-database-identity"
    identity_inode = (cli.data / "kassiber.sqlite3").stat().st_ino
    original_prefix = cli_runner.prefix
    script = '''import sys, os, tempfile, secrets
from pathlib import Path
from kassiber.operator import project
project._owner_lock_root=lambda:Path(os.environ["KASSIBER_RUST_TEST_OWNER_ROOT"])
secrets.token_urlsafe=lambda n:os.environ["KASSIBER_RUST_TEST_BACKUP_TOKEN"]
secrets.token_hex=lambda n:os.environ["KASSIBER_RUST_TEST_SECRETS_TOKEN"]
original=tempfile.mkdtemp
def temporary(suffix=None,prefix=None,dir=None):
    if prefix==".restore-install-":
        path=Path(dir)/(prefix+os.environ["KASSIBER_RUST_TEST_RESTORE_SUFFIX"])
        path.mkdir(mode=0o700)
        return str(path)
    return original(suffix=suffix,prefix=prefix,dir=dir)
tempfile.mkdtemp=temporary
# The real local-watch worker opens a second SQLCipher connection even when
# there is no pending work. Finish that startup before taking a backup's
# database-generation fingerprint, rather than capturing its transient journal.
# Keep the worker and the product fingerprint/restore code unchanged.
import threading
from kassiber import daemon, daemon_chain_analysis_watches as watches
watch_ready=threading.Event()
watch_open=watches.open_db
watch_start=daemon._start_watch_worker
def opened_watch(*args,**kwargs):
    try:
        return watch_open(*args,**kwargs)
    finally:
        watch_ready.set()
def started_watch(ctx,**kwargs):
    previous=getattr(ctx,"watch_worker",None)
    if previous is None or not previous.is_alive():
        watch_ready.clear()
    watch_start(ctx,**kwargs)
    if getattr(ctx,"watch_worker",None) is not previous:
        assert watch_ready.wait(30),"local-watch database startup timed out"
watches.open_db=opened_watch
daemon._start_watch_worker=started_watch
from kassiber.cli.main import main
raise SystemExit(main(sys.argv[1:]))
'''

    def prefix(*, reference=False, python_options=()):
        if reference:
            return (sys.executable, *python_options, "-c", script)
        return original_prefix(reference=False, python_options=python_options)

    monkeypatch.setattr(cli_runner, "prefix", prefix)
    temporary = tmp_path / "temporary"
    temporary.mkdir()
    cli.env.update(KASSIBER_RUST_TEST_BACKUP_TOKEN=TOKEN, KASSIBER_RUST_TEST_SECRETS_TOKEN="ab" * 16,
                   KASSIBER_RUST_TEST_RESTORE_SUFFIX=SUFFIX, KASSIBER_HYBRID_RUST_DAEMON="1", KASSIBER_HYBRID_LOG=str(tmp_path / "served-by.jsonl"), TMPDIR=str(temporary))
    cli.archive, cli.snapshot, cli.incoming = archive, snapshot, incoming

    def reset():
        # Move the original inode out of either live data or retained rollback,
        # then put it back. Both runtimes correctly forbid hard-link aliases.
        original = next(path for path in state.rglob("*") if path.is_file() and path.stat().st_ino == identity_inode)
        original.rename(identity)
        shutil.rmtree(state)
        shutil.copytree(snapshot, state)
        (cli.data / "kassiber.sqlite3").unlink()
        identity.rename(cli.data / "kassiber.sqlite3")
        (cli.data / "kassiber.sqlite3").write_bytes((snapshot / "data/kassiber.sqlite3").read_bytes())

    cli.reset = reset
    yield cli
    # Every successfully routed session advertises all four native kinds.
    assert not list(temporary.glob("kassiber-restore-*"))


class Session:
    def __init__(self, process, reference):
        self.process, self.reference, self.index = process, reference, 0
        ready = self.read()
        kinds = json.loads(ready)["data"]["supported_kinds"]
        assert all("ui.backup." + kind in kinds for kind in ("export", "preview", "apply", "cancel"))

    def read(self):
        assert select.select([self.process.stdout], [], [], 60)[0], "daemon response timed out"
        line = self.process.stdout.readline()
        assert line, self.process.stderr.read()
        return line

    def ask(self, kind, args=None):
        self.index += 1
        kind = "ui.backup." + kind if kind in ("export", "preview", "apply", "cancel") else kind
        request = {"kind": kind, "request_id": str(self.index)}
        if args is not None:
            request["args"] = args
        self.process.stdin.write((json.dumps(request) + "\n").encode())
        self.process.stdin.flush()
        raw = self.read()
        result = json.loads(raw)
        assert result["request_id"] == str(self.index)  # no extra progress/event records
        if not self.reference:
            COUNTS[kind] += 1
        return raw, result

    def unlock(self):
        return self.ask("daemon.unlock", {"auth_response": {"passphrase_secret": KEY}})

    def preview(self, archive):
        return self.ask("preview", {"path": str(archive), "backup_passphrase_secret": OUTER, "database_passphrase_secret": KEY})


@contextmanager
def session(cli, reference):
    process = cli_runner.popen(["--data-root", str(cli.data), "daemon"], reference=reference, env=cli.env,
                              stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, secret_input=True)
    try:
        instance = Session(process, reference)
        yield instance
        instance.ask("daemon.shutdown")
        process.stdin.close()
        process.wait(timeout=60)
        assert process.returncode == 0
        assert process.stdout.read() == b""
        assert process.stderr.read() == b""
        log = Path(cli.env["KASSIBER_HYBRID_LOG"])
        if not reference and log.exists():
            records = [json.loads(line) for line in log.read_text().splitlines()]
            assert records and all(row == {"command": "daemon", "served_by": "rust"} for row in records)

    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=10)


def assert_same(cli, scenario):
    results = []
    for reference in (True, False):
        cli.reset()
        with session(cli, reference) as instance:
            results.append(scenario(instance))
    assert results[0] == results[1]


def test_desktop_refusals_and_cancel_match_raw_jsonl(desktop, tmp_path):
    def scenario(s):
        records = []
        for kind in ("export", "preview", "apply"):
            records.append(s.ask(kind)[0])
        records.append(s.ask("cancel", {"token": "missing"})[0])
        records.append(s.unlock()[0])
        for kind in ("export", "preview", "apply", "cancel"):
            records.append(s.ask(kind, ["malformed args"])[0])
        for path in (None, 2, "relative.kassiber", str(tmp_path / "bad.tar"), str(desktop.data.parent / "inside.kassiber")):
            records.append(s.ask("export", {"path": path})[0])
        records.append(s.ask("export", {"path": str(tmp_path / "valid.kassiber")})[0])
        for args in ({}, {"path": "relative"}, {"path": str(desktop.archive)},
                     {"path": str(tmp_path / "missing"), "backup_passphrase_secret": OUTER},
                     {"path": str(desktop.archive), "backup_passphrase_secret": OUTER},
                     {"path": str(desktop.archive), "backup_passphrase_secret": "\ud800", "database_passphrase_secret": KEY},
                     {"path": str(desktop.archive), "backup_passphrase_secret": "wrong", "database_passphrase_secret": KEY},
                     {"path": str(desktop.archive), "backup_passphrase_secret": OUTER, "database_passphrase_secret": "wrong"}):
            records.append(s.ask("preview", args)[0])
        records.append(s.ask("apply", {"confirm": "restore"})[0])
        records.append(s.ask("apply", {"confirm": "RESTORE", "token": TOKEN})[0])
        return records
    assert_same(desktop, scenario)


def test_preview_summary_cancel_and_lock_lifecycle_match_raw_jsonl(desktop):
    def scenario(s):
        records = [s.unlock()[0]]
        raw, shown = s.preview(desktop.archive)
        records.append(raw)
        data = shown["data"]
        assert data["target"]["books"] == ["Target book"]
        assert data["incoming"]["books"] == ["Incoming book"]
        assert data["attachments_files"] == 1
        assert set(data) == {"token", "target_data_root", "target", "incoming", "attachments_files", "replaces", "unavailable_secrets", "expires_in_seconds"}
        records.append(s.ask("cancel", {"token": "wrong"})[0])
        records.append(s.ask("apply", {"token": TOKEN})[0])
        records.append(s.ask("cancel", {"token": TOKEN})[0])
        records.append(s.ask("apply", {"token": TOKEN, "confirm": "RESTORE"})[0])
        records.append(s.preview(desktop.archive)[0])
        records.append(s.ask("daemon.lock")[0])
        records.append(s.unlock()[0])
        records.append(s.ask("apply", {"token": TOKEN, "confirm": "RESTORE"})[0])
        return records
    assert_same(desktop, scenario)


@pytest.mark.parametrize("change", ["database", "attachment", "staging", "source", "none"])
def test_apply_generation_guards_and_safe_restore_match_raw_jsonl(desktop, change):
    archive_bytes = desktop.archive.read_bytes()
    def scenario(s):
        desktop.archive.write_bytes(archive_bytes)
        records = [s.unlock()[0], s.preview(desktop.archive)[0]]
        if change == "database":
            conn = open_encrypted(desktop.data / "kassiber.sqlite3", KEY, enforce_operator_identity=False, row_factory=get_row_class())
            conn.execute("UPDATE profiles SET label='Concurrent change'")
            conn.commit()
            conn.close()
        elif change == "attachment":
            (desktop.data.parent / "attachments/nested/₿.bin").write_bytes(b"changed evidence")
        elif change == "staging":
            stages = list(Path(desktop.env["TMPDIR"]).glob("kassiber-restore-*/stage"))
            assert len(stages) == 1
            (stages[0] / "attachments/nested/₿.bin").write_bytes(b"tampered snapshot")
        elif change == "source":
            desktop.archive.write_bytes(b"source no longer matters")
        raw, result = s.ask("apply", {"token": TOKEN, "confirm": "RESTORE"})
        records.append(raw)
        if change in ("none", "source"):
            assert result["data"]["restored"] is True
            assert result["data"]["locked"] is True
            assert result["data"]["warning"] is None
            conn = open_encrypted(desktop.data / "kassiber.sqlite3", KEY, enforce_operator_identity=False, row_factory=get_row_class())
            assert normalized_dump(conn) == desktop.incoming
            conn.close()
            saved = Path(result["data"]["pre_restore_backup"])
            assert saved.name == "pre-restore-" + SUFFIX
            assert (saved / "exports/excluded.txt").read_text() == "excluded export"
            assert not (desktop.data.parent / "exports").exists()
            settings = json.loads((desktop.data.parent / "config/settings.json").read_text())
            assert settings["operator_unlock_mode"] == "manual"
            assert settings["cli_legacy_unlock_quarantined"] is True
            assert settings["operator_native_auth_generation"] == "ab" * 16
            records.append(s.ask("export", {"path": str(desktop.archive)})[0])
        else:
            assert result["error"]["code"] == "stale_backup_preview"
        return records
    assert_same(desktop, scenario)


def test_plaintext_incoming_and_invalid_archives_match_raw_jsonl(desktop, tmp_path):
    plain = tmp_path / "plain.kassiber"
    import sqlite3
    db = tmp_path / "plain.db"
    conn = sqlite3.connect(db)
    conn.execute("CREATE TABLE profiles(label TEXT)")
    conn.close()
    _bundle(plain, [("kassiber.sqlite3", db.read_bytes())], _manifest())
    malformed = tmp_path / "malformed.kassiber"
    malformed.write_bytes(b"not an age envelope")
    def scenario(s):
        return [s.unlock()[0], s.preview(plain)[0], s.preview(malformed)[0]]
    assert_same(desktop, scenario)


def _exported_dump(archive, scratch):
    """The normalized dump of the database inside an exported archive."""
    plain = pyrage.passphrase.decrypt(archive.read_bytes(), OUTER)
    with tarfile.open(fileobj=io.BytesIO(plain)) as tar:
        scratch.write_bytes(tar.extractfile("kassiber.sqlite3").read())
    conn = open_encrypted(scratch, KEY, enforce_operator_identity=False, row_factory=get_row_class())
    try:
        return normalized_dump(conn)
    finally:
        conn.close()
        scratch.unlink()


def test_export_success_and_io_refusal_match_raw_jsonl(desktop, tmp_path):
    output = tmp_path / "export.kassiber"
    blocked = tmp_path / "blocked"
    blocked.write_text("regular file")
    dumps = {}
    def scenario(s):
        records = [s.unlock()[0]]
        records.append(s.ask("export", {"path": str(output), "backup_passphrase_secret": OUTER})[0])
        dumps[s.reference] = _exported_dump(output, tmp_path / "exported.sqlite3")
        records.append(s.ask("export", {"path": str(blocked / "out.kassiber"), "backup_passphrase_secret": OUTER})[0])
        return records
    assert_same(desktop, scenario)
    # Equal sizes could hide a data difference; compare the exported contents.
    assert dumps[True] == dumps[False]



def test_other_owner_refusal_retains_preview_until_safe_apply(desktop):
    def scenario(s):
        records = [s.unlock()[0], s.preview(desktop.archive)[0]]
        with patch.object(ownership, "_owner_lock_root", lambda: desktop.owners):
            lease = ownership.acquire_project_ownership(ownership.canonical_project(desktop.data), owner_kind="broker", generation="oracle-other-owner")
            try:
                raw, result = s.ask("apply", {"token": TOKEN, "confirm": "RESTORE"})
                assert result["error"]["code"] == "project_in_use"
                records.append(raw)
            finally:
                lease.release()
        records.append(s.ask("apply", {"token": TOKEN, "confirm": "RESTORE"})[0])
        return records
    assert_same(desktop, scenario)


def test_plaintext_export_refusal_precedes_outer_passphrase(cli, tmp_path):
    ok(cli("init", reference=True))
    cli.env.update(KASSIBER_HYBRID_RUST_DAEMON="1", KASSIBER_HYBRID_LOG=str(tmp_path / "served-by.jsonl"))
    results = []
    for reference in (True, False):
        with session(cli, reference) as s:
            records = [s.ask("status")[0]]
            raw, result = s.ask("export", {"path": str(tmp_path / "plain.kassiber")})
            assert result["error"]["code"] == "plaintext_database"
            records.append(raw)
            results.append(records)
    assert results[0] == results[1]

@pytest.fixture(scope="module", autouse=True)
def served_counts():
    yield
    print("desktop backup Rust-served counts:", json.dumps(dict(sorted(COUNTS.items()))))
