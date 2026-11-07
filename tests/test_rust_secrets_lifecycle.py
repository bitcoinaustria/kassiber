"""CLI/daemon secrets parity on disposable books; native stores are gated.

The ordinary cases give both processes an unreachable private bus address.
They never consult the account's actual credential store. That address only
isolates Linux's Secret Service: on macOS and Windows both runtimes would
write and delete real login-keychain or Credential Manager items, so the
module runs on Linux only (`tests/test_keyring_isolation_guard.py`).
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import select

import pytest

from kassiber.db import open_db
from kassiber.secrets import sqlcipher
from tests import cli_runner
from tests.test_rust_secrets_services import KEY, RUST_BIN, ROOT, cli as base_cli, ok, same
from tests.test_rust_store_migrations import normalized_dump
from tests.test_rust_shell_parity import _environment

pytestmark = [
    pytest.mark.skipif(sys.platform != "linux", reason="an unreachable D-Bus address isolates only Linux's keyring; elsewhere this would use the account's real credential store"),
    pytest.mark.skipif(not RUST_BIN, reason="set KASSIBER_RUST_BIN to a test-harness build"),
]
NEW = "Synthetic replacement 🔐"
TOKEN = "ab" * 16
TIME = "2026-01-02T03:04:05Z"


@pytest.fixture
def cli(base_cli, monkeypatch, tmp_path):
    # Exported only into children by the existing runner fixture; startup must
    # not autolaunch or discover the account's bus when testing availability.
    original = cli_runner.run
    original_prefix = cli_runner.prefix
    def prefix(*, reference=False, python_options=()):
        result = original_prefix(reference=reference, python_options=python_options)
        if reference:
            result = list(result)
            index = result.index("-c") + 1
            result[index] = result[index].replace("from kassiber.cli.main import main;", "from kassiber.secrets import unlock_store, credentials; from kassiber import backends; import secrets; secrets.token_hex=lambda n:'abababababababababababababababab'; credentials._now_stamp=lambda:'20260102T030405Z'; backends.now_iso=lambda:'2026-01-02T03:04:05Z'; from kassiber.cli.main import main;")
            result = tuple(result)
        return result
    def run(*args, **kwargs):
        env = dict(kwargs.get("env", {}))
        env.update(DBUS_SESSION_BUS_ADDRESS=f"unix:path={tmp_path}/absent-bus", KASSIBER_RUST_TEST_SECRETS_TOKEN=TOKEN, KASSIBER_RUST_TEST_SECRETS_TIME=TIME)
        env.update(getattr(base_cli, "native_env", {}))
        kwargs["env"] = env
        return original(*args, **kwargs)
    monkeypatch.setattr(cli_runner, "prefix", prefix)
    monkeypatch.setattr(cli_runner, "run", run)
    return base_cli


def encrypted_book(cli):
    ok(cli("secrets", "status"))
    conn = open_db(cli.data, passphrase=KEY, require_existing_schema=False)
    conn.close()
    return cli.data / "kassiber.sqlite3"


def test_status_classification_dotenv_and_policy_match_without_secret_disclosure(cli):
    same(cli, "secrets", "status")
    database = encrypted_book(cli)
    env = cli.data.parent / "config/backends.env"
    env.write_text("# synthetic\nKASSIBER_BACKEND_CORE_KIND=bitcoinrpc\nKASSIBER_BACKEND_CORE_URL=http://127.0.0.1:8332\nKASSIBER_BACKEND_CORE_PASSWORD=do-not-print\nKASSIBER_BACKEND_CORE_COOKIEFILE=/cookie\n")
    result = same(cli, "secrets", "status")
    assert b"do-not-print" not in result.stdout + result.stderr
    assert ok(result)["encrypted"]
    before = database.read_bytes()
    same(cli, "secrets", "status")
    assert database.read_bytes() == before


def test_new_command_refusals_and_authentication_order_match(cli):
    for command in ("change-passphrase", "remember-unlock", "migrate-credentials"):
        same(cli, "secrets", command)
    encrypted_book(cli)
    for command in ("change-passphrase", "remember-unlock", "forget-unlock"):
        same(cli, "secrets", command)
    same(cli, "--db-passphrase-fd", cli_runner.InputFd(KEY.encode()), "secrets", "change-passphrase", "--new-passphrase-fd", cli_runner.InputFd(b"short"))
    # No credentials means migration does not require passphrase input.
    same(cli, "secrets", "migrate-credentials")


def test_rotation_matches_reference_and_preserves_complete_logical_book(cli):
    database = encrypted_book(cli)
    before = database.read_bytes()
    conn = sqlcipher.open_encrypted(database, KEY)
    dump = normalized_dump(conn)
    conn.close()
    args = ("--db-passphrase-fd", cli_runner.InputFd(KEY.encode()), "secrets", "change-passphrase", "--new-passphrase-fd", cli_runner.InputFd(NEW.encode()))
    python = cli(*args, reference=True)
    database.write_bytes(before)
    rust = cli(*args)
    assert (rust.returncode, rust.stdout, rust.stderr) == (python.returncode, python.stdout, python.stderr)
    conn = sqlcipher.open_encrypted(database, NEW)
    assert normalized_dump(conn) == dump
    conn.close()
    same(cli, "--db-passphrase-fd", cli_runner.InputFd(NEW.encode()), "secrets", "verify")


def test_unavailable_remember_and_authenticated_forget_match_without_native_items(cli):
    encrypted_book(cli)
    same(cli, "secrets", "remember-unlock", "--passphrase-fd", cli_runner.InputFd(KEY.encode()))
    result = same(cli, "--db-passphrase-fd", cli_runner.InputFd(KEY.encode()), "secrets", "forget-unlock")
    assert ok(result)["remembered_unlock"]["cli_enabled"] is False
    assert b"plaintext fallback" not in result.stderr


def seed_credentials(cli, body):
    database = encrypted_book(cli)
    conn = sqlcipher.open_encrypted(database, KEY)
    conn.execute("INSERT INTO backends(name,kind,url,config_json,created_at,updated_at) VALUES('core','bitcoinrpc','http://127.0.0.1:8332','{\"preserved\": 7}',?,?)", (TIME, TIME))
    conn.commit()
    conn.close()
    env = cli.data.parent / "config/backends.env"
    env.write_text(body)
    return database, env


def test_credential_migration_matches_wire_bytes_and_every_table(cli):
    body = "# keep\nKASSIBER_BACKEND_CORE_KIND=bitcoinrpc\nKASSIBER_BACKEND_CORE_URL=http://127.0.0.1:8332\nKASSIBER_BACKEND_GONE_KIND=custom\nKASSIBER_BACKEND_GONE_URL=http://127.0.0.1:8333\nKASSIBER_BACKEND_CORE_RPCUSER=' user '\nSATBOOKS_BACKEND_CORE_RPCPASSWORD=' pw '\nKASSIBER_BACKEND_CORE_TOKEN=exact-token\nKASSIBER_BACKEND_GONE_TOKEN=retained\nKASSIBER_BACKEND_CORE_COOKIEFILE=/cookie\n"
    database, env = seed_credentials(cli, body)
    same(cli, "secrets", "migrate-credentials", "--dry-run")
    same(cli, "secrets", "migrate-credentials")  # refuses input before changes
    original = database.read_bytes()
    args = ("--db-passphrase-fd", cli_runner.InputFd(KEY.encode()), "secrets", "migrate-credentials")
    python = cli(*args, reference=True)
    conn = sqlcipher.open_encrypted(database, KEY)
    expected = normalized_dump(conn)
    conn.close()
    rewritten = env.read_bytes()
    database.write_bytes(original)
    env.write_text(body)
    rust = cli(*args)
    assert (rust.returncode, rust.stdout, rust.stderr) == (python.returncode, python.stdout, python.stderr)
    assert env.read_bytes() == rewritten
    conn = sqlcipher.open_encrypted(database, KEY)
    assert normalized_dump(conn) == expected
    conn.close()
    assert b"exact-token" not in rust.stdout + rust.stderr
    assert Path(ok(rust)["backup_path"]).read_text() == body
    same(cli, *args)


def test_duplicate_aliases_refuse_before_any_credential_move(cli):
    database, env = seed_credentials(cli, "KASSIBER_BACKEND_CORE_KIND=bitcoinrpc\nKASSIBER_BACKEND_CORE_URL=http://127.0.0.1:8332\nKASSIBER_BACKEND_CORE_PASSWORD=first\nSATBOOKS_BACKEND_CORE_RPC_PASSWORD=second\n")
    before = database.read_bytes(), env.read_bytes()
    result = same(cli, "--db-passphrase-fd", cli_runner.InputFd(KEY.encode()), "secrets", "migrate-credentials")
    assert result.returncode == 1
    assert (database.read_bytes(), env.read_bytes()) == before


def test_daemon_rotation_and_forget_kinds_match_reference_bytes(cli, tmp_path):
    database = encrypted_book(cli)
    original = database.read_bytes()
    settings = cli.data.parent / "config/settings.json"
    original_settings = settings.read_bytes()
    requests = [
        {"kind": "ui.secrets.forget_cli_unlock"},
        {"kind": "ui.secrets.change_passphrase", "args": {}},
        {"kind": "ui.secrets.change_passphrase", "args": {"auth_response": {"passphrase_secret": KEY}}},
        {"kind": "ui.secrets.change_passphrase", "args": {"auth_response": {"passphrase_secret": KEY}, "new_passphrase_secret": "short"}},
        {"kind": "ui.secrets.change_passphrase", "args": {"auth_response": {"passphrase_secret": KEY}, "new_passphrase_secret": "1234567890"}},
        {"kind": "ui.secrets.change_passphrase", "args": {"auth_response": {"passphrase_secret": KEY}, "new_passphrase_secret": NEW}},
        {"kind": "daemon.lock"},
        {"kind": "daemon.unlock", "args": {"auth_response": {"passphrase_secret": NEW}}},
        {"kind": "ui.secrets.forget_cli_unlock"},
        {"kind": "daemon.shutdown"},
    ]
    raw = "".join(json.dumps(dict(request_id=str(i), **request)) + "\n" for i, request in enumerate(requests)).encode()
    env = _environment(tmp_path / "home")
    env.update(KASSIBER_RUST_TEST_OWNER_ROOT=str(cli.owners), KASSIBER_TEST_NO_EGRESS="strict", KASSIBER_NO_EGRESS="1", KASSIBER_HYBRID_RUST_DAEMON="1")
    results = []
    for reference in (True, False):
        database.write_bytes(original)
        settings.write_bytes(original_settings)
        result = cli_runner.run(["--data-root", str(cli.data), "daemon"], reference=reference, env=env, input=raw, capture_output=True, timeout=90, secret_input=True)
        assert result.returncode == 0, (result.stdout, result.stderr)
        assert result.stderr == b""
        results.append(result.stdout.splitlines(keepends=True))
    assert results[0][1:] == results[1][1:]
    supported = json.loads(results[1][0])["data"]["supported_kinds"]
    assert {"ui.secrets.change_passphrase", "ui.secrets.forget_cli_unlock"} <= set(supported)
    assert json.loads(results[1][6])["kind"] == "ui.secrets.change_passphrase"


@pytest.fixture(scope="module")
def secrets_probe():
    binary = ROOT / "target/debug/examples/kassiber-secrets-probe"
    if not binary.is_file():
        pytest.skip("build kassiber-secrets-probe with test-hooks,native-keyring")
    return binary


def killed_probe(binary, cli, checkpoint, operation, env=None):
    descriptors = []
    request = dict(op=operation, data_root=str(cli.data), owners=str(cli.owners), kill_at=checkpoint)
    for field, value in (("fd", KEY), ("new_fd", NEW)):
        read, write = os.pipe()
        os.write(write, value.encode())
        os.close(write)
        descriptors.append(read)
        request[field] = read
    try:
        process = subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, pass_fds=descriptors, env=env or {**os.environ, "DBUS_SESSION_BUS_ADDRESS": f"unix:path={cli.data.parent}/absent-bus"})
    finally:
        for fd in descriptors:
            os.close(fd)
    try:
        process.stdin.write((json.dumps(request) + "\n").encode())
        process.stdin.flush()
        assert select.select([process.stdout], [], [], 30)[0], "probe did not reach checkpoint"
        assert json.loads(process.stdout.readline()) == {"checkpoint": checkpoint}
    finally:
        process.kill()
        process.communicate(timeout=10)


CREDENTIAL_POINTS = ["credentials.after_row_commit", "credentials.after_backup_sync", "credentials.after_backup_publish", "credentials.after_sanitized_sync", "credentials.before_sanitized_publish", "credentials.after_sanitized_publish", "credentials.after_directory_sync"]
ROTATION_POINTS = ["rotation.after_operator_guard", "rekey.before_rekey", "rekey.after_rekey", "rekey.after_verified_sync"]
if sys.platform == "darwin":
    ROTATION_POINTS.insert(0, "rotation.after_desktop_guard")


@pytest.mark.parametrize("reference", [True, False], ids=["python-recovery", "rust-recovery"])
@pytest.mark.parametrize("checkpoint", CREDENTIAL_POINTS)
def test_killed_credential_move_resumes_in_either_runtime(cli, secrets_probe, checkpoint, reference):
    body = "KASSIBER_BACKEND_CORE_KIND=bitcoinrpc\nKASSIBER_BACKEND_CORE_URL=http://127.0.0.1:8332\nKASSIBER_BACKEND_CORE_TOKEN=synthetic-credential\n"
    database, env = seed_credentials(cli, body)
    killed_probe(secrets_probe, cli, checkpoint, "credentials")
    result = cli("--db-passphrase-fd", cli_runner.InputFd(KEY.encode()), "secrets", "migrate-credentials", reference=reference)
    ok(result)
    assert "synthetic-credential" not in env.read_text()
    conn = sqlcipher.open_encrypted(database, KEY)
    assert conn.execute("SELECT token FROM backends WHERE name='core'").fetchone()[0] == "synthetic-credential"
    assert conn.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    conn.close()


@pytest.mark.parametrize("reference", [True, False], ids=["python-recovery", "rust-recovery"])
@pytest.mark.parametrize("checkpoint", ROTATION_POINTS)
def test_killed_rotation_keeps_guards_armed_and_resumes_in_either_runtime(cli, secrets_probe, checkpoint, reference):
    database = encrypted_book(cli)
    killed_probe(secrets_probe, cli, checkpoint, "rotate")
    settings = json.loads((cli.data.parent / "config/settings.json").read_text())
    assert len(settings["operator_native_auth_generation"]) == 32
    key = NEW if checkpoint in ("rekey.after_rekey", "rekey.after_verified_sync") else KEY
    ok(cli("--db-passphrase-fd", cli_runner.InputFd(key.encode()), "secrets", "verify", reference=reference))
    if key == KEY:
        ok(cli("--db-passphrase-fd", cli_runner.InputFd(KEY.encode()), "secrets", "change-passphrase", "--new-passphrase-fd", cli_runner.InputFd(NEW.encode()), reference=reference))
    conn = sqlcipher.open_encrypted(database, NEW)
    assert conn.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    assert conn.execute("PRAGMA foreign_key_check").fetchall() == []
    conn.close()


@pytest.fixture(scope="module")
def disposable_keyring(tmp_path_factory):
    if os.environ.get("KASSIBER_TEST_DISPOSABLE_KEYRING") != "1":
        pytest.skip("set KASSIBER_TEST_DISPOSABLE_KEYRING=1 to launch a fresh Linux bus and keyring")
    if sys.platform != "linux" or not all(shutil.which(tool) for tool in ("dbus-run-session", "gnome-keyring-daemon")):
        pytest.skip("requires Linux dbus-run-session and gnome-keyring-daemon")
    root = tmp_path_factory.mktemp("disposable-native-keyring")
    home = root / "home"
    home.mkdir()
    env = _environment(home)
    env.pop("DBUS_SESSION_BUS_ADDRESS", None)
    env["XDG_RUNTIME_DIR"] = str(root / "run")
    Path(env["XDG_RUNTIME_DIR"]).mkdir(mode=0o700)
    script = r'''
import json, os, subprocess, sys, time
import secretstorage
daemon = subprocess.Popen(["gnome-keyring-daemon", "--foreground", "--unlock", "--components=secrets"], stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
daemon.stdin.write(b"\n"); daemon.stdin.close()
try:
    for attempt in range(40):
        try:
            connection = secretstorage.dbus_init()
            collection = secretstorage.get_default_collection(connection)
            if not collection.is_locked():
                break
        except Exception:
            pass
        time.sleep(0.1)
    else:
        raise RuntimeError("disposable Secret Service could not be unlocked")
    print(json.dumps({"DBUS_SESSION_BUS_ADDRESS": os.environ["DBUS_SESSION_BUS_ADDRESS"]}), flush=True)
    sys.stdin.readline()
finally:
    daemon.terminate()
    daemon.wait(timeout=5)
'''
    process = subprocess.Popen(["dbus-run-session", "--", sys.executable, "-c", script], env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        assert select.select([process.stdout], [], [], 10)[0], "disposable bus startup timed out"
        line = process.stdout.readline()
        if not line:
            error = process.stderr.read().decode(errors="replace")
            pytest.skip(f"sandbox cannot launch a disposable Secret Service: {error[-500:]}")
        yield {**env, **json.loads(line)}
    finally:
        if process.poll() is None:
            process.communicate(b"stop\n", timeout=10)


def test_disposable_keyring_python_and_rust_enrollment_rotation_and_revocation(cli, disposable_keyring):
    cli.native_env = disposable_keyring
    encrypted_book(cli)
    # Reference enrolls the exact Python item, Rust uses it on the same book.
    ok(cli("secrets", "remember-unlock", "--passphrase-fd", cli_runner.InputFd(KEY.encode()), reference=True))
    assert ok(same(cli, "secrets", "status"))["remembered_unlock"]["configured"]
    ok(cli("--db-passphrase-fd", cli_runner.InputFd(KEY.encode()), "secrets", "change-passphrase", "--new-passphrase-fd", cli_runner.InputFd(NEW.encode())))
    assert ok(same(cli, "secrets", "status"))["remembered_unlock"]["configured"]
    ok(cli("--db-passphrase-fd", cli_runner.InputFd(NEW.encode()), "secrets", "forget-unlock", reference=True))
    ok(cli("secrets", "remember-unlock", "--passphrase-fd", cli_runner.InputFd(NEW.encode())))
    ok(cli("--db-passphrase-fd", cli_runner.InputFd(NEW.encode()), "secrets", "forget-unlock"))
    assert not ok(same(cli, "secrets", "status"))["remembered_unlock"]["configured"]


NATIVE_POINTS = [*( ("remember", p) for p in ["remember.after_item_write", "remember.after_prior_delete", "remember.after_legacy_cli_delete", "remember.after_legacy_shared_delete", "remember.after_policy_publish"]), *( ("forget", p) for p in ["forget.after_item_delete", "forget.after_legacy_cli_delete", "forget.after_legacy_shared_delete", "forget.after_policy_publish"]), ("rotate", "rotation.after_item_refresh")]
NATIVE_POINTS += [("rotate-failed-refresh", p) for p in ["rotation.after_item_delete", "rotation.after_legacy_cli_delete", "rotation.after_legacy_shared_delete", "rotation.after_policy_disable"]]


@pytest.mark.parametrize("reference", [True, False], ids=["python-recovery", "rust-recovery"])
@pytest.mark.parametrize(("operation", "checkpoint"), NATIVE_POINTS)
def test_disposable_keyring_interruption_resumes_in_either_runtime(cli, secrets_probe, disposable_keyring, operation, checkpoint, reference):
    cli.native_env = disposable_keyring
    encrypted_book(cli)
    if operation != "remember":
        ok(cli("secrets", "remember-unlock", "--passphrase-fd", cli_runner.InputFd(KEY.encode()), reference=True))
    killed_probe(secrets_probe, cli, checkpoint, operation, env=disposable_keyring)
    key = NEW if operation.startswith("rotate") else KEY
    if operation == "remember":
        ok(cli("secrets", "remember-unlock", "--passphrase-fd", cli_runner.InputFd(key.encode()), reference=reference))
    else:
        ok(cli("--db-passphrase-fd", cli_runner.InputFd(key.encode()), "secrets", "forget-unlock", reference=reference))
    # Both runtimes can open the resulting book; policy never grants ownership
    # to an unpublished enrollment, even if a killed writer left an orphan item.
    for runtime in (True, False):
        ok(cli("--db-passphrase-fd", cli_runner.InputFd(key.encode()), "secrets", "verify", reference=runtime))
