"""R5 encryption boundaries; synthetic descriptors and disposable books only.

The reference launcher injects its ownership namespace before CLI startup.
No test in this module constructs a native keyring or contacts a session bus.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import select
import sqlite3
import subprocess
import sys
from unittest.mock import patch

import pytest

from kassiber.secrets import sqlcipher
from kassiber.secrets.migration import create_empty_encrypted_database
from kassiber.operator import project as ownership
from tests import cli_runner
from tests.test_rust_shell_parity import _environment
from tests.test_rust_store_migrations import normalized_dump
from tests.rust_fixture_binary import checked

ROOT = Path(__file__).resolve().parents[1]
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
KEY = "Synthetic R5 '🔑 with spaces "
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="set KASSIBER_RUST_BIN to a test-harness build")


@pytest.fixture
def cli(tmp_path, monkeypatch):
    home = tmp_path / "home"
    home.mkdir()
    owners = tmp_path / "owners"
    owners.mkdir(mode=0o700)
    env = _environment(home)
    env.update(KASSIBER_RUST_TEST_OWNER_ROOT=str(owners), KASSIBER_TEST_NO_EGRESS="strict", KASSIBER_NO_EGRESS="1")
    data = tmp_path / "book/data"
    original = cli_runner.prefix
    script = (
        "import sys; from pathlib import Path; "
        "from kassiber.operator import project; "
        "owners=Path(sys.argv.pop(1)); project._owner_lock_root=lambda:owners; "
        "from kassiber.cli.main import main; raise SystemExit(main(sys.argv[1:]))"
    )

    def prefix(*, reference=False, python_options=()):
        if reference:
            return (sys.executable, *python_options, "-c", script, str(owners))
        if cli_runner.candidate_prefix() is not None:
            return original(reference=False, python_options=python_options)
        return (str(Path(RUST_BIN).resolve()),)

    monkeypatch.setattr(cli_runner, "prefix", prefix)

    def call(*args, reference=False, root=None):
        return cli_runner.run(["--machine", "--data-root", str(root or data), *args], reference=reference,
                              env=env, capture_output=True, timeout=90, stdin=subprocess.DEVNULL)

    call.data = data
    call.owners = owners
    call.env = env
    return call


def ok(result):
    assert result.returncode == 0, (result.stdout, result.stderr)
    assert result.stderr == b""
    return json.loads(result.stdout)["data"]


def same(cli, *args):
    python, rust = cli(*args, reference=True), cli(*args)
    assert (rust.returncode, rust.stdout, rust.stderr) == (python.returncode, python.stdout, python.stderr)
    return rust


def test_empty_init_and_verify_match_reference_bytes_without_schema_bootstrap(cli):
    args = ("secrets", "init", "--new-passphrase-fd", cli_runner.InputFd(KEY.encode()))
    python = cli(*args, reference=True)
    assert ok(python)["mode"] == "created"
    database = cli.data / "kassiber.sqlite3"
    assert ok(same(cli, "--db-passphrase-fd", cli_runner.InputFd(KEY.encode()), "secrets", "verify"))["sqlite_master_rows"] == 0
    database.unlink()
    rust = cli(*args)
    assert (rust.returncode, rust.stdout, rust.stderr) == (python.returncode, python.stdout, python.stderr)
    assert not sqlcipher.looks_like_plaintext_sqlite(database)
    before = database.read_bytes()
    result = same(cli, "--db-passphrase-fd", cli_runner.InputFd(KEY.encode()), "secrets", "verify")
    assert ok(result)["sqlite_master_rows"] == 0
    assert database.read_bytes() == before
    same(cli, "secrets", "init")  # already encrypted, before asking for input
    same(cli, "secrets", "init-resume")
    assert not (cli.data / "kassiber.encrypted.sqlite3").exists()


def test_plaintext_migration_preserves_every_row_pragmas_and_rollback_copy(cli):
    cli.data.mkdir(parents=True)
    database = cli.data / "kassiber.sqlite3"
    conn = sqlite3.connect(database)
    conn.executescript("PRAGMA auto_vacuum=2; CREATE TABLE historical(value, other); PRAGMA user_version=37;")
    conn.executemany("INSERT INTO historical VALUES(?,?)", [("_TOKEN=synthetic marker", b"\x00\xff"), ("₿", 9223372036854775807), (None, 0.125)])
    conn.commit()
    before = normalized_dump(conn)
    conn.close()
    result = ok(cli("secrets", "init", "--new-passphrase-fd", cli_runner.InputFd(KEY.encode())))
    assert result["mode"] == "migrated"
    assert result["user_version"] == 37
    assert result["auto_vacuum"] == 2
    encrypted = sqlcipher.open_encrypted(database, KEY, enforce_operator_identity=False)
    try:
        assert normalized_dump(encrypted) == before
        assert encrypted.execute("PRAGMA foreign_key_check").fetchall() == []
    finally:
        encrypted.close()
    backup = sqlite3.connect(result["backup"])
    try:
        assert normalized_dump(backup) == before
    finally:
        backup.close()
    same(cli, "--db-passphrase-fd", cli_runner.InputFd(KEY.encode()), "secrets", "verify")
    same(cli, "secrets", "init-resume")
    assert b"_TOKEN=" not in database.read_bytes()


def test_inspection_missing_plaintext_and_input_refusals_match_reference(cli):
    same(cli, "secrets", "init-resume")
    same(cli, "secrets", "verify")
    same(cli, "secrets", "init")
    for secret in ("short", "🔑" * 7):
        same(cli, "secrets", "init", "--new-passphrase-fd", cli_runner.InputFd(secret.encode()))
    assert not (cli.data / "kassiber.sqlite3").exists()
    sqlite3.connect(cli.data / "kassiber.sqlite3").execute("CREATE TABLE marker(value)").connection.close()
    same(cli, "secrets", "verify")
    for name in ("kassiber.encrypted.sqlite3", "kassiber.pre-encryption.sqlite3.bak"):
        (cli.data / name).write_bytes(b"retained interrupted artifact")
    same(cli, "secrets", "init-resume")


@pytest.mark.parametrize("raw", [b"", b"has\x00nul", b"bad\xffutf8", b"\n"])
def test_invalid_descriptor_input_never_creates_a_database_or_prints_secret(cli, raw):
    result = cli("secrets", "init", "--new-passphrase-fd", cli_runner.InputFd(raw))
    assert result.returncode == 1
    assert json.loads(result.stdout)["error"]["code"] == "invalid_passphrase"
    assert not (cli.data / "kassiber.sqlite3").exists()
    assert not (cli.data / "kassiber.encrypted.sqlite3").exists()
    assert b"has" not in result.stdout + result.stderr
    assert b"bad" not in result.stdout + result.stderr


@pytest.mark.parametrize("owner", ["desktop", "broker"])
@pytest.mark.parametrize("plaintext", [False, True])
def test_init_refuses_either_live_owner_without_changing_the_database(cli, owner, plaintext):
    cli.data.mkdir(parents=True)
    database = cli.data / "kassiber.sqlite3"
    if plaintext:
        conn = sqlite3.connect(database)
        conn.execute("CREATE TABLE retained(value)")
        conn.close()
    before = database.read_bytes() if database.exists() else None
    with patch.object(ownership, "_owner_lock_root", return_value=cli.owners):
        project = ownership.canonical_project(str(cli.data))
        with ownership.acquire_project_ownership(project, owner_kind=owner, generation="r5-test-owner"):
            for reference in (True, False):
                result = cli("secrets", "init", "--new-passphrase-fd", cli_runner.InputFd(KEY.encode()), reference=reference)
                assert result.returncode == 1, (result.stdout, result.stderr)
                assert json.loads(result.stdout)["error"]["code"] == "project_in_use"
                assert (database.read_bytes() if database.exists() else None) == before
                assert not (cli.data / "kassiber.encrypted.sqlite3").exists()


@pytest.fixture(scope="module")
def storage_probe():
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    binary = target / "debug/examples/kassiber-storage-probe"
    if not binary.is_file():
        pytest.skip("build the test-hooks kassiber-storage-probe example")
    return checked(binary, "cargo build -p kassiber-store --example kassiber-storage-probe --features test-hooks --offline --locked")


EMPTY_BOUNDARIES = [
    "empty.before_candidate_create", "empty.after_candidate_create",
    "empty.after_candidate_close", "empty.before_candidate_sync", "empty.after_candidate_sync",
    "empty.after_candidate_file_sync", "empty.before_candidate_directory_sync",
    "empty.before_placeholder_remove", "empty.before_placeholder_directory_sync", "empty.after_placeholder_remove",
    "empty.before_publish_rename", "empty.after_publish_rename", "empty.before_publish_directory_sync", "empty.after_publish_directory_sync",
]


def probe(binary, root, **request):
    read, write = os.pipe()
    try:
        os.write(write, KEY.encode())
    finally:
        os.close(write)
    try:
        return subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, text=True, pass_fds=(read,)), dict(data_root=str(root), fd=read, **request)
    finally:
        os.close(read)


def rust_verify(binary, root, database):
    process, request = probe(binary, root, op="verify-encrypted-file", database=str(database))
    output, error = process.communicate(json.dumps(request) + "\n", timeout=30)
    assert process.returncode == 0, (output, error)
    payload = json.loads(output)
    assert payload["ok"], payload
    assert payload["data"]["sqlite_master_rows"] == 0


@pytest.mark.parametrize("resume", ["python", "rust"])
@pytest.mark.parametrize(("placeholder", "checkpoint"), [
    (placeholder, checkpoint) for placeholder in (False, True) for checkpoint in EMPTY_BOUNDARIES
    if placeholder or "placeholder" not in checkpoint
])
def test_killed_empty_publication_is_inspectable_and_recoverable_by_either_runtime(storage_probe, tmp_path, resume, placeholder, checkpoint):
    root = tmp_path / "data"
    root.mkdir()
    database = root / "kassiber.sqlite3"
    if placeholder:
        database.write_bytes(b"")
    process, request = probe(storage_probe, root, op="create-empty-encrypted", kill_at=checkpoint)
    try:
        process.stdin.write(json.dumps(request) + "\n")
        process.stdin.flush()
        assert select.select([process.stdout], [], [], 30)[0], "probe did not reach checkpoint"
        assert json.loads(process.stdout.readline()) == {"checkpoint": checkpoint}
    finally:
        process.kill()
        process.communicate(timeout=10)
    candidate = root / "kassiber.encrypted.sqlite3"
    if candidate.exists() and candidate.stat().st_size:
        if resume == "rust":
            rust_verify(storage_probe, root, candidate)
        else:
            conn = sqlcipher.open_encrypted(candidate, KEY, enforce_operator_identity=False)
            assert conn.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
            assert conn.execute("SELECT count(*) FROM sqlite_master").fetchone()[0] == 0
            conn.close()
        # Python's init-resume contract explicitly leaves verified candidate
        # publication to the operator. Both readers above prove that recovery.
        candidate.replace(database)
    elif not database.exists() or not database.stat().st_size:
        candidate.unlink(missing_ok=True)  # only an incomplete zero-byte candidate
        if resume == "rust":
            process, request = probe(storage_probe, root, op="create-empty-encrypted")
            output, error = process.communicate(json.dumps(request) + "\n", timeout=30)
            assert process.returncode == 0, (output, error)
            assert json.loads(output)["ok"], output
        else:
            create_empty_encrypted_database(database, KEY)
    rust_verify(storage_probe, root, database)
    conn = sqlcipher.open_encrypted(database, KEY, enforce_operator_identity=False)
    assert conn.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    assert conn.execute("PRAGMA foreign_key_check").fetchall() == []
    conn.close()
    assert not candidate.exists()
