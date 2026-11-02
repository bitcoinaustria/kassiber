"""R1 parity through real Rust processes and disposable Python-created books.

The example is built offline, so dependencies must first be fetched by setup.
No test selects the real account-home owner namespace or a real keyring item.
"""
from __future__ import annotations

import base64
import gzip
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
from unittest.mock import patch
import uuid

import pytest

from kassiber import db, projects
from kassiber.core.runtime import resolve_runtime_paths
from kassiber.operator import project as ownership
from kassiber.secrets.prompt import read_passphrase_from_fd
from kassiber.errors import AppError

ROOT = Path(__file__).resolve().parents[1]
PASSPHRASE = "Synthetic R1 '🔑\nsecond line"


@pytest.fixture(scope="module", autouse=True)
def _sweep_session_owner_locks(tmp_path_factory):
    """Override the repository janitor fixture, keeping even its sweep in tmp."""
    root = tmp_path_factory.mktemp("r1-owner-namespace")
    root.chmod(0o700)
    with patch.object(ownership, "_owner_lock_root", return_value=root):
        yield


@pytest.fixture(scope="module")
def probe():
    cargo = ["mise", "exec", "rust@1.99.1", "--", "cargo"] if shutil.which("mise") else ["cargo"]
    result = subprocess.run(
        [*cargo, "build", "--locked", "--offline", "--example", "kassiber-store-probe", "--features", "kassiber-secrets/native-keyring"],
        cwd=ROOT, capture_output=True, text=True, timeout=300, env={**os.environ,"MISE_OFFLINE":"1"},
    )
    assert result.returncode == 0, result.stderr
    metadata = subprocess.run([*cargo, "metadata", "--locked", "--offline", "--no-deps", "--format-version", "1"], cwd=ROOT, capture_output=True, text=True, check=True)
    return Path(json.loads(metadata.stdout)["target_directory"]) / "debug" / "examples" / ("kassiber-store-probe.exe" if os.name == "nt" else "kassiber-store-probe")


@pytest.fixture
def isolated(tmp_path, monkeypatch):
    home = tmp_path / "home"
    home.mkdir()
    monkeypatch.setenv("HOME", str(home))
    monkeypatch.setenv("USERPROFILE", str(home))
    monkeypatch.setenv("XDG_DATA_HOME", str(home / "xdg"))
    monkeypatch.setenv("KASSIBER_NO_EGRESS", "1")
    for key in ("KASSIBER_OPERATOR_CHILD", "KASSIBER_OPERATOR_EXPECTED_DATABASE_IDENTITY", "KASSIBER_OPERATOR_EXPECTED_PROJECT_IDENTITY"):
        monkeypatch.delenv(key, raising=False)
    owner_root = tmp_path / "owners"
    owner_root.mkdir(mode=0o700)
    with patch.object(ownership, "_owner_lock_root", return_value=owner_root):
        yield home, owner_root


@pytest.fixture(scope="module")
def installed_cargo():
    # Resolve tooling before function fixtures replace HOME. Never ask mise to
    # discover/install tools from a disposable product-home environment.
    command=["mise","exec","rust@1.99.1","--","rustup","which","cargo"] if shutil.which("mise") else ["rustup","which","--toolchain","1.99.1","cargo"]
    result=subprocess.run(command,cwd=ROOT,text=True,capture_output=True,check=True,timeout=30,env={**os.environ,"MISE_OFFLINE":"1"})
    return Path(result.stdout.strip()),str(os.environ.get("CARGO_HOME",Path.home()/".cargo"))


def _probe(probe, request, *, raw=None, check=True):
    with tempfile.TemporaryFile() as fd:
        if raw is not None:
            fd.write(raw)
            fd.seek(0)
            request = {**request, "fd": fd.fileno()}
        result = subprocess.run([str(probe)], input=json.dumps(request) + "\n", text=True,
                                capture_output=True, pass_fds=(fd.fileno(),) if raw is not None else (), timeout=45)
    payload = json.loads(result.stdout)
    if check:
        assert payload["ok"], payload
    return payload


_CLI = """
import sys
from pathlib import Path
from kassiber.operator import project
root = Path(sys.argv[1])
project._owner_lock_root = lambda: root
from kassiber.cli.main import main
raise SystemExit(main(sys.argv[2:]))
"""


def _cli(root, owner_root, *args, raw=None, new=False, check=True):
    with tempfile.TemporaryFile() as fd:
        if raw is not None:
            fd.write(raw)
            fd.seek(0)
        globals_ = ["--machine", "--data-root", str(root)]
        if raw is not None and not new:
            globals_ += ["--db-passphrase-fd", str(fd.fileno())]
        command = list(args)
        if raw is not None and new:
            command += ["--new-passphrase-fd", str(fd.fileno())]
        result = subprocess.run([sys.executable, "-c", _CLI, str(owner_root), *globals_, *command],
                                capture_output=True, text=True, pass_fds=(fd.fileno(),) if raw is not None else (), timeout=60)
    payload = json.loads(result.stdout)
    if check:
        assert result.returncode == 0, (payload, result.stderr)
    return payload


def _book(tmp_path, owner_root, encrypted=False):
    data = tmp_path / "project" / "data"
    _cli(data, owner_root, "init")
    _cli(data, owner_root, "workspaces", "create", "R1 workspace")
    _cli(data, owner_root, "profiles", "create", "R1 book", "--workspace", "R1 workspace")
    if encrypted:
        _cli(data, owner_root, "secrets", "init", raw=PASSPHRASE.encode(), new=True)
    return data


@pytest.mark.skipif(os.name == "nt", reason="POSIX inherited-fd harness; native Windows runner remains an R1 gate")
@pytest.mark.parametrize("encrypted", [False, True])
def test_python_created_books_rust_reads_and_writes(probe, tmp_path, isolated, encrypted):
    _, owners = isolated
    data = _book(tmp_path, owners, encrypted)
    secret = PASSPHRASE if encrypted else None
    conn = db.open_db(data, passphrase=secret)
    identity = db.database_instance_id(conn)
    expected = {table: conn.execute(f"SELECT count(*) FROM {table}").fetchone()[0] for table in ("workspaces", "profiles", "accounts", "wallets", "transactions")}
    expected_rows = {table: [row[0] for row in conn.execute(f"SELECT label FROM {table} ORDER BY id")] for table in ("workspaces", "profiles")}
    conn.close()
    payload = _probe(probe, {"op": "open", "data_root": str(data), "write_key": "r1_interop_setting", "write_value": "Rust wrote exact text €"}, raw=secret.encode() if secret else None)["data"]
    assert payload["instance_id"] == identity
    assert payload["counts"] == expected
    assert payload["rows"] == expected_rows
    assert payload["encrypted"] is encrypted
    assert payload["journal_mode"] == ("delete" if encrypted else db._journal_settings_for_sqlite_version(db.sqlite3.sqlite_version)[0])
    conn = db.open_db(data, passphrase=secret)
    try:
        assert db.get_setting(conn, "r1_interop_setting") == "Rust wrote exact text €"
        assert conn.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
        assert list(conn.execute("PRAGMA foreign_key_check")) == []
    finally:
        conn.close()


@pytest.mark.parametrize("encrypted", [False, True])
def test_held_python_connection_stays_safe_across_rust_open_and_write(probe, tmp_path, isolated, encrypted):
    data = _book(tmp_path, isolated[1], encrypted)
    secret = PASSPHRASE if encrypted else None
    conn = db.open_db(data, passphrase=secret)
    try:
        before = conn.execute("PRAGMA journal_mode").fetchone()[0]
        result = _probe(probe, {"op":"open", "data_root":str(data), "write_key":"r1_interop_held", "write_value":"Rust"}, raw=secret.encode() if secret else None)
        assert result["data"]["journal_mode"] == before
        assert conn.execute("PRAGMA journal_mode").fetchone()[0] == before
        db.set_setting(conn, "r1_interop_held", "Python")
        conn.commit()
        opened = db.open_db(data, passphrase=secret)
        assert db.get_setting(opened, "r1_interop_held") == "Python"
        opened.close()
        assert conn.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    finally:
        conn.close()


@pytest.mark.parametrize("encrypted", [False, True])
def test_held_rust_connection_allows_python_open_and_write(probe, tmp_path, isolated, encrypted):
    data = _book(tmp_path, isolated[1], encrypted)
    secret = PASSPHRASE if encrypted else None
    with tempfile.TemporaryFile() as fd:
        request = {"op":"open-hold", "data_root":str(data)}
        if secret:
            fd.write(secret.encode()); fd.seek(0); request["fd"] = fd.fileno()
        process = subprocess.Popen([str(probe)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, pass_fds=(fd.fileno(),) if secret else ())
        try:
            process.stdin.write(json.dumps(request) + "\n"); process.stdin.flush()
            assert json.loads(process.stdout.readline())["data"]["holding"]
            conn = db.open_db(data, passphrase=secret)
            db.set_setting(conn,"r1_interop_held","Python while Rust holds")
            conn.commit(); conn.close()
            process.stdin.write("release\n"); process.stdin.flush()
            out, err = process.communicate(timeout=45)
            assert process.returncode == 0, err
            assert json.loads(out)["ok"]
        finally:
            if process.poll() is None: process.kill(); process.communicate(timeout=10)


@pytest.mark.parametrize("fixture", sorted((ROOT / "tests/fixtures/historical").glob("*.sqlite3.gz.b64")), ids=lambda p: p.name)
def test_historical_books_refused_without_mutating_bytes(probe, tmp_path, isolated, fixture):
    data = tmp_path / "data"
    data.mkdir()
    path = data / "kassiber.sqlite3"
    path.write_bytes(gzip.decompress(base64.b64decode(fixture.read_bytes())))
    before = path.read_bytes()
    result = _probe(probe, {"op": "open", "data_root": str(data)}, check=False)
    assert result["error"]["code"] == "requires_python_migration"
    assert path.read_bytes() == before
    conn = db.open_db(data)
    conn.close()
    assert path.read_bytes() != before  # Python actually had work to do.


@pytest.mark.parametrize("fixture", sorted((ROOT / "tests/fixtures/historical").glob("*.sqlite3.gz.b64")), ids=lambda p:p.name)
def test_python_migrated_historical_layouts_are_accepted(probe, tmp_path, isolated, fixture):
    data = tmp_path / "data"
    data.mkdir()
    (data / "kassiber.sqlite3").write_bytes(gzip.decompress(base64.b64decode(fixture.read_bytes())))
    for _ in range(2):
        conn = db.open_db(data); conn.close()
    result = _probe(probe, {"op":"open", "data_root":str(data)}, check=False)
    assert result["ok"], result


@pytest.mark.parametrize("mutation", ["DROP TRIGGER chain_index_track_transactions_insert", "DROP INDEX idx_chain_index_outgoing", "DELETE FROM schema_migration_audits WHERE migration_name='kassiber-tax-engine-v1'", "DELETE FROM settings WHERE key='database_instance_id'", "CREATE TABLE future_schema(value INTEGER)"])
def test_schema_or_audit_drift_refused(probe, tmp_path, isolated, mutation):
    data = _book(tmp_path, isolated[1])
    conn = db.open_db(data)
    conn.execute(mutation)
    conn.commit()
    conn.close()
    before = (data / "kassiber.sqlite3").read_bytes()
    result=_probe(probe, {"op": "open", "data_root": str(data)}, check=False)
    assert result["error"]["code"] == ("database_busy" if mutation.startswith(("DROP TRIGGER","DROP INDEX")) else "requires_python_migration")
    assert (data / "kassiber.sqlite3").read_bytes() == before


@pytest.mark.parametrize("object_type,name", [("VIEW","journal_custody_projection_relations"),("TRIGGER","chain_index_track_transactions_insert"),("INDEX","idx_chain_index_outgoing")])
def test_python_open_schema_replacement_window_is_retryable(probe,tmp_path,isolated,object_type,name):
    data=_book(tmp_path,isolated[1]);conn=db.open_db(data)
    sql=conn.execute("SELECT sql FROM sqlite_master WHERE name=?",(name,)).fetchone()[0]
    conn.execute(f'DROP {object_type} "{name}"');conn.commit()
    result=_probe(probe,{"op":"open","data_root":str(data)},check=False)
    assert result["error"]["code"]=="database_busy",result
    assert result["error"]["retryable"] is True
    conn.execute(sql);conn.commit();conn.close()
    assert _probe(probe,{"op":"open","data_root":str(data)})["ok"]


@pytest.mark.skipif(os.name == "nt", reason="POSIX inherited-fd harness")
@pytest.mark.parametrize("raw", [b"normal\r\n", b"normal\n\n", b"", b"\r\n", b"\xff", b"nul\x00", b"x" * 8192, b"x" * 8193])
def test_passphrase_fd_matches_python(probe, isolated, raw):
    with tempfile.TemporaryFile() as fd:
        fd.write(raw)
        fd.seek(0)
        duplicated = os.dup(fd.fileno())
        try:
            expected = read_passphrase_from_fd(duplicated)
        except AppError as error:
            expected = error.code
            failed = True
        else:
            failed = False
        with pytest.raises(OSError):
            os.fstat(duplicated)
    result = _probe(probe, {"op": "fd", "expected": expected}, raw=raw, check=False)
    if failed:
        assert result["error"]["code"] == expected
    else:
        assert result["data"]["matches"] is True
        assert result["data"]["length"] == len(expected.encode())


@pytest.mark.skipif(os.name == "nt", reason="POSIX inherited-fd harness")
def test_unlock_failures_and_database_binding(probe, tmp_path, isolated):
    data = _book(tmp_path, isolated[1])
    assert _probe(probe, {"op": "open", "data_root": str(data)}, raw=b"irrelevant", check=False)["error"]["code"] == "plaintext_database"
    assert _probe(probe, {"op": "open", "data_root": str(data), "expected_id": "f" * 32}, check=False)["error"]["code"] == "operator_project_replaced"
    _cli(data, isolated[1], "secrets", "init", raw=PASSPHRASE.encode(), new=True)
    error = _probe(probe, {"op": "open", "data_root": str(data)}, check=False)["error"]
    assert error["code"] == "interaction_required"
    assert error["details"]["reason"] == "database_passphrase"
    assert _probe(probe, {"op": "open", "data_root": str(data)}, raw=b"wrong", check=False)["error"]["code"] == "unlock_failed"


@pytest.mark.parametrize("xdg", [None, "", "relative", "absolute"])
@pytest.mark.parametrize("legacy", ["absent", "bin-run", "state", "satbooks", "flat-kassiber"])
def test_root_resolvers_match_python(probe, tmp_path, isolated, xdg, legacy):
    home, _ = isolated
    environ = {} if xdg is None else {"XDG_DATA_HOME": str(home / "custom") if xdg == "absolute" else xdg}
    if legacy in ("bin-run", "state"):
        (home / ".kassiber/bin").mkdir(parents=True)
        (home / ".kassiber/run").mkdir()
        if legacy == "state":
            (home / ".kassiber/unknown-state").write_text("synthetic")
    if legacy in ("satbooks", "flat-kassiber"):
        flat = home / ".local/share" / ("satbooks" if legacy == "satbooks" else "kassiber")
        flat.mkdir(parents=True)
        (flat / "satbooks.sqlite3").write_bytes(b"synthetic")
    default = db.default_state_root(platform="linux", environ=environ, home=home)
    native = db.native_state_root(platform="linux", environ=environ, home=home)
    with patch.multiple(db, DEFAULT_DATA_ROOT=str(default / "data"), LEGACY_XDG_DATA_ROOT=str(home / ".local/share/kassiber"), LEGACY_DATA_ROOT=str(home / ".local/share/satbooks")):
        for requested in (default / "data", tmp_path / "explicit/data", tmp_path / "flat"):
            data = db.resolve_effective_data_root(requested)
            expected = {"native": str(native), "default": str(default), "data_root": str(data), "state_root": str(db.resolve_effective_state_root(requested)), "database": str(db.resolve_database_path(data))}
            actual = _probe(probe, {"op": "roots", "platform": "linux", "home": str(home), "environ": environ, "data_root": str(requested)})["data"]
            assert actual == expected


@pytest.mark.parametrize("platform,python_platform", [("linux", "linux"), ("macos", "darwin"), ("windows", "win32")])
def test_native_platform_path_vectors(probe, isolated, platform, python_platform):
    home, _ = isolated
    for environ in ({}, {"LOCALAPPDATA": str(home / "local")}, {"LOCALAPPDATA": "relative"}):
        result = _probe(probe, {"op": "roots", "platform": platform, "home": str(home), "environ": environ, "data_root": str(home / "explicit")})["data"]
        assert result["native"] == str(db.native_state_root(platform=python_platform, environ=environ, home=home))


@pytest.mark.parametrize("selection", ["explicit", "project", "selected", "both"])
def test_runtime_paths_match_python_without_writes(probe, tmp_path, isolated, selection):
    home, _ = isolated
    state = db.native_state_root(platform=sys.platform, environ=os.environ, home=home)
    projects.create_project("Synthetic project", project_id="synthetic", state_root=state)
    project_data = state / "projects/synthetic/data"
    db.ensure_settings_file(project_data, project_data.parent / "config/backends.env")
    # A legacy env file must outrank the absent preferred file.
    (project_data.parent / ".env").write_text("# synthetic\n")
    kwargs = {"data_root": str(project_data)} if selection == "explicit" else {"project": "SYNTHETIC"} if selection == "project" else {"project": "synthetic", "data_root": str(project_data)} if selection == "both" else {}
    before = sorted((str(p), p.read_bytes()) for p in state.rglob("*") if p.is_file())
    result = _probe(probe, {"op": "resolve", "platform": "linux", "home": str(home), "environ": {"XDG_DATA_HOME": os.environ["XDG_DATA_HOME"]}, "options": kwargs}, check=False)
    if selection == "both":
        with pytest.raises(AppError) as error:
            resolve_runtime_paths(**kwargs)
        assert result["error"]["code"] == error.value.code
    else:
        # Avoid mark_project_opened; this is the same resolver the runtime uses.
        assert result["data"] == resolve_runtime_paths(**kwargs).__dict__
    assert before == sorted((str(p), p.read_bytes()) for p in state.rglob("*") if p.is_file())


def test_uninitialized_resolution_creates_nothing(probe, isolated):
    home, _ = isolated
    request = {"op": "resolve", "platform": "linux", "home": str(home), "environ": {}, "options": {}}
    assert _probe(probe, request, check=False)["error"]["code"] == "not_initialized"
    assert list(home.iterdir()) == []
    (home / ".kassiber").mkdir()
    (home / ".kassiber/state").write_text("synthetic")
    assert _probe(probe, request, check=False)["error"]["code"] == "requires_python_migration"
    assert not (home / ".local").exists()


def test_canonical_file_identity_matches_python(probe, tmp_path, isolated):
    data = _book(tmp_path, isolated[1])
    expected = ownership.canonical_project(data)
    actual = _probe(probe, {"op": "identity", "data_root": str(data), "owner_root": str(isolated[1])})["data"]
    assert actual == {key: str(getattr(expected, key)) for key in ("identity", "database", "lock_path", "alias_lock_path", "local_lock_path")}


@pytest.mark.skipif(os.name == "nt", reason="POSIX inherited-fd CLI harness")
def test_rust_owner_blocks_python_cli_and_python_owner_blocks_rust(probe, tmp_path, isolated):
    _, owners = isolated
    data = _book(tmp_path, owners)
    request = {"op": "lock-hold", "data_root": str(data), "owner_root": str(owners), "role": "desktop"}
    process = subprocess.Popen([str(probe)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        process.stdin.write(json.dumps(request) + "\n")
        process.stdin.flush()
        assert json.loads(process.stdout.readline())["data"]["holding"] is True
        response = _cli(data, owners, "secrets", "init", raw=PASSPHRASE.encode(), new=True, check=False)
        assert response["error"]["code"] == "project_in_use"
        assert response["error"]["retryable"] is True
        # Opposite role remains compatible for the same identity.
        _probe(probe, {**request, "op": "lock-once", "role": "broker"})
    finally:
        process.communicate("release\n", timeout=10)
    project = ownership.canonical_project(data)
    with ownership.acquire_project_ownership(project, owner_kind="desktop", generation="python-r1-synthetic"):
        response = _probe(probe, {**request, "op": "lock-once"}, check=False)
        assert response["error"]["code"] == "project_in_use"
        assert response["error"]["details"]["pid"] == os.getpid()
        _probe(probe, {**request, "op": "lock-once", "role": "broker"})
    _probe(probe, {**request, "op": "lock-once"})


@pytest.mark.skipif(os.name == "nt", reason="POSIX inherited-fd launch harness")
@pytest.mark.parametrize("role", ["broker", "desktop"])
def test_rust_launched_child_holds_inherited_owner_locks_against_python(probe, tmp_path, isolated, role):
    # R6-1: the broker hands its children owner-lock duplicates. Once the Rust
    # parent released its own lease, the child's inherited copies alone must
    # still exclude a Python owner of the same role, and only them.
    _, owners = isolated
    data = _book(tmp_path, owners)
    request = {"op": "lock-child", "data_root": str(data), "owner_root": str(owners), "role": role}
    process = subprocess.Popen([str(probe)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    project = ownership.canonical_project(data)
    try:
        process.stdin.write(json.dumps(request) + "\n")
        process.stdin.flush()
        holding = json.loads(process.stdout.readline())["data"]
        assert holding["holding"] is True
        with pytest.raises(AppError) as error:
            ownership.acquire_project_ownership(project, owner_kind=role, generation="python-r6-synthetic")
        assert error.value.code == "project_in_use"
        assert error.value.details["owner"] == role
        # The record names the Rust parent that wrote it, not the child.
        assert error.value.details["pid"] == process.pid
        assert f"pid {process.pid}" in error.value.hint
        other = "desktop" if role == "broker" else "broker"
        ownership.acquire_project_ownership(project, owner_kind=other, generation="python-r6-other").release()
    finally:
        out, err = process.communicate("release\n", timeout=30)
    assert process.returncode == 0, err
    assert json.loads(out)["data"] == {"released": True, "child_exit": 0}
    ownership.acquire_project_ownership(project, owner_kind=role, generation="python-r6-after").release()


def test_rust_owner_records_are_python_bytes(probe, tmp_path, isolated):
    _, owners = isolated
    data = _book(tmp_path, owners)
    request = {"op": "lock-hold", "data_root": str(data), "owner_root": str(owners), "role": "broker"}
    process = subprocess.Popen([str(probe)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    project = ownership.canonical_project(data)
    try:
        process.stdin.write(json.dumps(request) + "\n")
        process.stdin.flush()
        assert json.loads(process.stdout.readline())["data"]["holding"] is True
        record = {"schema_version": 2, "owner": "broker", "generation": "r1-synthetic-probe", "identity": project.identity, "pid": process.pid}
        expected = (json.dumps(record, sort_keys=True) + "\n").encode("utf-8")
        for lock_path in (project.lock_path, project.alias_lock_path, project.local_lock_path):
            assert ownership._owner_kind_lock_path(lock_path, "broker").read_bytes() == expected
            # The shared compatibility locks carry no record.
            assert lock_path.read_bytes() == b""
    finally:
        process.communicate("release\n", timeout=30)


@pytest.mark.skipif(sys.platform != "linux", reason="Linux Secret Service integration")
def test_python_secret_service_item_rust_reads_updates_in_place(probe, isolated):
    if not os.environ.get("DBUS_SESSION_BUS_ADDRESS"):
        pytest.skip("no Secret Service session")
    import secretstorage
    from keyring.backends.SecretService import Keyring
    try:
        connection = secretstorage.dbus_init()
        collection = secretstorage.get_default_collection(connection)
        if collection.is_locked():
            pytest.skip("Secret Service collection is locked")
    except Exception:
        pytest.skip("unlocked Secret Service collection is unavailable")
    service = f"Kassiber R1 synthetic {uuid.uuid4().hex}"
    account = hashlib.sha256(uuid.uuid4().bytes).hexdigest()
    backend = Keyring()
    try:
        backend.set_password(service, account, "Python R1 '🔑")
        matches = list(collection.search_items({"service": service, "username": account}))
        assert len(matches) == 1
        before = matches[0].get_attributes()
        object_path = matches[0].item_path
        result = _probe(probe, {"op": "keyring-read", "service": service, "account": account})
        assert result["data"]["matches"] is True
        assert backend.get_password(service, account) == "Rust R1 '🔐"
        after = list(collection.search_items({"service": service, "username": account}))
        assert after[0].item_path == object_path
        assert after[0].get_attributes() == before
    finally:
        backend.delete_password(service, account)
    assert backend.get_password(service, account) is None


def test_schema_manifest_is_current_python_stop_state(isolated):
    result = subprocess.run([sys.executable, str(ROOT / "crates/store/tools/export_schema.py"), "--check"], cwd=ROOT, capture_output=True, text=True, timeout=30)
    assert result.returncode == 0, result.stderr


@pytest.mark.skipif(os.name == "nt", reason="POSIX inherited-fd harness")
@pytest.mark.parametrize("encrypted", [False, True])
def test_hot_backup_round_trip_to_python(probe, tmp_path, isolated, encrypted):
    source = _book(tmp_path / "source", isolated[1], encrypted)
    destination = _book(tmp_path / "destination", isolated[1], encrypted)
    raw = PASSPHRASE.encode() if encrypted else None
    result = _probe(probe, {"op": "backup", "data_root": str(source), "destination": str(destination)}, raw=raw)["data"]
    conn = db.open_db(destination, passphrase=PASSPHRASE if encrypted else None)
    try:
        assert db.database_instance_id(conn) == result["instance_id"]
        assert conn.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
        assert conn.execute("SELECT count(*) FROM profiles").fetchone()[0] > 0
    finally:
        conn.close()
    _probe(probe, {"op": "open", "data_root": str(destination)}, raw=raw)


def test_busy_writer_maps_to_retryable_database_busy(probe, tmp_path, isolated):
    data = _book(tmp_path, isolated[1])
    conn = db.open_db(data)
    conn.execute("BEGIN IMMEDIATE")
    try:
        result = _probe(probe, {"op": "write-busy", "data_root": str(data)}, check=False)
        assert result["error"]["code"] == "database_busy"
        assert result["error"]["retryable"] is True
    finally:
        conn.rollback()
        conn.close()


@pytest.mark.skipif(os.name == "nt", reason="POSIX inherited-fd harness")
@pytest.mark.parametrize("case", ["valid", "manual", "null-mode", "brokered", "quarantined", "missing-binding", "wrong-project", "wrong-database", "invalid-enrollment", "missing-enrollment", "stale-secret"])
def test_remembered_unlock_binding_order_and_cache(probe, tmp_path, isolated, case):
    from kassiber.secrets.unlock_store import remembered_unlock_account
    data = _book(tmp_path, isolated[1], True)
    conn = db.open_db(data, passphrase=PASSPHRASE)
    identity = db.database_instance_id(conn)
    conn.close()
    enrollment = "e" * 32
    account = remembered_unlock_account(data, database_identity=identity, enrollment_id=enrollment)
    settings = {"operator_unlock_mode": "unattended", "cli_remembered_unlock": True, "operator_policy_project_identity": ownership.canonical_project(data).identity, "operator_policy_database_identity": identity, "cli_remembered_unlock_enrollment": enrollment}
    if case in ("manual", "brokered"):
        settings["operator_unlock_mode"] = case
    elif case == "null-mode":
        settings["operator_unlock_mode"] = None
    elif case == "quarantined":
        settings["cli_legacy_unlock_quarantined"] = True
    elif case == "missing-binding":
        settings.pop("operator_policy_project_identity")
    elif case == "wrong-project":
        settings["operator_policy_project_identity"] = "0" * 64
    elif case == "wrong-database":
        settings["operator_policy_database_identity"] = "f" * 32
        account = remembered_unlock_account(data, database_identity="f" * 32, enrollment_id=enrollment)
    elif case == "invalid-enrollment":
        settings["cli_remembered_unlock_enrollment"] = "invalid"
    elif case == "missing-enrollment":
        settings.pop("cli_remembered_unlock_enrollment")
    db.update_managed_settings(data, updates=settings)
    result = _probe(probe, {"op": "policy-unlock", "data_root": str(data), "account": account}, raw=b"stale synthetic secret" if case == "stale-secret" else PASSPHRASE.encode(), check=False)
    if case == "valid":
        assert result["data"] == {"instance_id": identity, "keyring_reads": 1}
    else:
        error = result["error"]
        assert error["code"] == ("operator_project_replaced" if case == "wrong-database" else "interaction_required")
        assert error["details"]["keyring_reads"] == (1 if case in ("wrong-database", "stale-secret") else 0)
        assert error["details"]["stale_notices"] == (1 if case == "stale-secret" else 0)
        if case in ("invalid-enrollment", "missing-enrollment", "null-mode"):
            from kassiber.core.runtime import _open_db_with_resolved_passphrase
            with patch("kassiber.secrets.unlock_store._native_keyring_available", return_value=True), patch("kassiber.secrets.unlock_store.keyring.get_password") as lookup:
                with pytest.raises(AppError) as reference:
                    _open_db_with_resolved_passphrase(data, None, allow_prompt=False)
            assert error["code"] == reference.value.code == "interaction_required"
            assert error["hint"] == reference.value.hint
            lookup.assert_not_called()


@pytest.mark.parametrize("legacy_data", ["ownership-history", "liquid-payload"])
def test_current_schema_with_pending_semantic_backfill_is_refused(probe, tmp_path, isolated, legacy_data):
    data = _book(tmp_path, isolated[1])
    conn = db.open_db(data)
    profile = conn.execute("SELECT id,workspace_id FROM profiles LIMIT 1").fetchone()
    config = {"ownership_history": []} if legacy_data == "ownership-history" else {"chain": "liquid", "network": "main"}
    conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('r1-synthetic-wallet',?,?,'Synthetic wallet','manual',?,'synthetic')", (profile["workspace_id"], profile["id"], json.dumps(config)))
    if legacy_data == "liquid-payload":
        conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,created_at) VALUES('r1-synthetic-tx',?,?,'r1-synthetic-wallet',?,'r1-synthetic-fingerprint','2026-01-01','inbound','LBTC',9007199254740993,'synthetic')", (profile["workspace_id"], profile["id"], "a" * 64))
    conn.commit()
    conn.close()
    path = data / "kassiber.sqlite3"
    before = path.read_bytes()
    result = _probe(probe, {"op": "open", "data_root": str(data)}, check=False)
    assert result["error"]["code"] == "requires_python_migration"
    assert path.read_bytes() == before
    conn = db.open_db(data)
    if legacy_data == "liquid-payload":
        row = conn.execute("SELECT amount,raw_json FROM transactions WHERE id='r1-synthetic-tx'").fetchone()
        assert row["amount"] == 9007199254740993
        assert json.loads(row["raw_json"])["network"] == "liquidv1"
    conn.close()
    _probe(probe, {"op": "open", "data_root": str(data)})


@pytest.mark.parametrize("case", ["null-history","mixed-case-history","empty-payment-hash","nested-payment-hash","non-policy-asset","non-liquid-lbtc","contradictory-liquid","missing-clock","unrecoverable-dismissal"])
def test_semantic_gate_accepts_rows_python_does_not_backfill(probe, tmp_path, isolated, case):
    from tests.test_rust_store_migrations import normalized_dump
    data = _book(tmp_path, isolated[1])
    conn = db.open_db(data)
    profile = conn.execute("SELECT id,workspace_id FROM profiles LIMIT 1").fetchone()
    config = {"ownership_history":None} if case == "null-history" else {"Ownership_History":[]} if case == "mixed-case-history" else {"chain":"liquid"} if case in ("non-policy-asset","contradictory-liquid") else {}
    conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('synthetic',?,?,'Synthetic','manual',?,'fixed')", (profile["workspace_id"],profile["id"],json.dumps(config)))
    if case in ("empty-payment-hash","nested-payment-hash","non-policy-asset","non-liquid-lbtc","contradictory-liquid"):
        raw = {"payment_hash":""} if case=="empty-payment-hash" else {"nested":{"payment_hash":"a"*64}} if case=="nested-payment-hash" else {"chain":"bitcoin","txid":"a"*64} if case=="contradictory-liquid" else {}
        asset = "c"*64 if case=="non-policy-asset" else "LBTC" if case in ("non-liquid-lbtc","contradictory-liquid") else "BTC"
        conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,raw_json,created_at) VALUES('synthetic-tx',?,?,'synthetic',?,'synthetic-fp','fixed','inbound',?,1,?,'fixed')",(profile["workspace_id"],profile["id"],"a"*64,asset,json.dumps(raw)))
    if case=="missing-clock":conn.execute("DELETE FROM chain_index_clock")
    if case=="unrecoverable-dismissal":
        conn.execute("INSERT INTO custody_gap_reviews(id,workspace_id,profile_id,gap_id,revision,candidate_fingerprint,action,created_at) VALUES('unrecoverable',?,?,'no-current-candidate',1,'not-a-current-fingerprint','dismissed','fixed')",(profile["workspace_id"],profile["id"]))
    conn.commit()
    before=normalized_dump(conn)
    conn.close()
    result=_probe(probe,{"op":"open","data_root":str(data)},check=False)
    assert result["ok"],result
    conn=db.open_db(data)
    assert normalized_dump(conn)==before
    conn.close()


@pytest.mark.parametrize("extension", ['NaN', 'Infinity', '1e400', '"\\ud800"'])
def test_python_json_extensions_cannot_hide_pending_review_relations(probe, tmp_path, isolated, extension):
    data=_book(tmp_path,isolated[1])
    conn=db.open_db(data)
    profile=conn.execute("SELECT id,workspace_id FROM profiles LIMIT 1").fetchone()
    snapshot='{"source_ids":["missing-source"],"extension":'+extension+'}'
    assert json.loads(snapshot)["source_ids"]==["missing-source"]
    conn.execute("INSERT INTO custody_gap_reviews(id,workspace_id,profile_id,gap_id,revision,candidate_fingerprint,action,snapshot_json,created_at) VALUES('review',?,?,'synthetic',1,'synthetic','resolved',?,'fixed')",(profile["workspace_id"],profile["id"],snapshot))
    conn.commit();conn.close()
    result=_probe(probe,{"op":"open","data_root":str(data)},check=False)
    assert result["error"]["code"]=="unsupported_schema_layout",result
    conn=db.open_db(data)
    assert conn.execute("SELECT transaction_id FROM custody_gap_review_transactions WHERE review_id='review'").fetchone()[0]=="missing-source"
    conn.close()


@pytest.mark.parametrize("asset,chain", [("LBTC-","liquid"),("L-B-T-C","liquid"),("LBTC","\x1cliquid\x1f")])
def test_liquid_legacy_spelling_and_python_strip_require_backfill(probe,tmp_path,isolated,asset,chain):
    data=_book(tmp_path,isolated[1]);conn=db.open_db(data)
    profile=conn.execute("SELECT id,workspace_id FROM profiles LIMIT 1").fetchone()
    conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('synthetic',?,?,'Synthetic','manual',?,'fixed')",(profile["workspace_id"],profile["id"],json.dumps({"chain":chain})))
    conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,created_at) VALUES('synthetic-tx',?,?,'synthetic',?,'synthetic-fp','fixed','inbound',?,1,'fixed')",(profile["workspace_id"],profile["id"],"a"*64,asset))
    conn.commit();conn.close()
    result=_probe(probe,{"op":"open","data_root":str(data)},check=False)
    assert result["error"]["code"]=="requires_python_migration",result
    conn=db.open_db(data)
    assert json.loads(conn.execute("SELECT raw_json FROM transactions WHERE id='synthetic-tx'").fetchone()[0])["chain"]=="liquid"
    conn.close()
    assert _probe(probe,{"op":"open","data_root":str(data)})["ok"]


def test_explicit_missing_runtime_layout_returns_not_initialized(probe, isolated):
    home, _ = isolated
    request = {"op": "resolve", "platform": "linux", "home": str(home), "environ": {}, "options": {"data_root": str(home / "missing/data")}}
    assert _probe(probe, request, check=False)["error"]["code"] == "not_initialized"
    assert list(home.iterdir()) == []


@pytest.mark.parametrize("case", ["valid","missing-project","wrong-project","outside-child"])
def test_operator_child_project_binding_precedes_keying(probe,tmp_path,isolated,monkeypatch,case):
    from kassiber.cli.main import _verify_operator_child_project
    from types import SimpleNamespace
    data=_book(tmp_path,isolated[1],True)
    conn=db.open_db(data,passphrase=PASSPHRASE)
    database_identity=db.database_instance_id(conn);conn.close()
    project=ownership.canonical_project(data)
    monkeypatch.setenv("KASSIBER_OPERATOR_EXPECTED_DATABASE_IDENTITY",database_identity)
    if case!="outside-child":monkeypatch.setenv("KASSIBER_OPERATOR_CHILD","1")
    if case!="missing-project":monkeypatch.setenv("KASSIBER_OPERATOR_EXPECTED_PROJECT_IDENTITY",project.identity if case!="wrong-project" else "0"*64)
    result=_probe(probe,{"op":"open","data_root":str(data)},raw=PASSPHRASE.encode() if case=="valid" else b"wrong key must not be tried",check=False)
    args=SimpleNamespace(data_root=str(data),env_file=None,project=None)
    if case=="valid":
        _verify_operator_child_project(args)
        assert result["ok"],result
    else:
        with pytest.raises(AppError) as reference:_verify_operator_child_project(args)
        assert result["error"]["code"]==reference.value.code


@pytest.mark.parametrize("requested", ["data", "./data/", "./flat//data/", "", "//synthetic///nested/data"])
def test_relative_and_lexical_paths_match_python(probe, isolated, requested):
    home, _ = isolated
    actual = _probe(probe, {"op": "roots", "platform": "linux", "home": str(home), "environ": {}, "data_root": requested})["data"]
    effective = db.resolve_effective_data_root(requested)
    assert actual["data_root"] == str(effective)
    assert actual["state_root"] == str(db.resolve_effective_state_root(requested))
    assert actual["database"] == str(db.resolve_database_path(effective))


def test_relative_runtime_paths_match_python(probe, tmp_path, isolated, monkeypatch):
    data = _book(tmp_path, isolated[1])
    monkeypatch.chdir(data.parent)
    home = isolated[0]
    actual = _probe(probe, {"op": "resolve", "platform": "linux", "home": str(home), "environ": {"XDG_DATA_HOME": os.environ["XDG_DATA_HOME"]}, "options": {"data_root": "./data/"}})["data"]
    assert actual == resolve_runtime_paths(data_root="./data/").__dict__


@pytest.mark.skipif(os.name=="nt",reason="POSIX HOME semantics")
def test_empty_home_environment_matches_python_root(probe,isolated,monkeypatch):
    monkeypatch.setenv("HOME","")
    expected=str(Path.home())
    result=_probe(probe,{"op":"environment-home"})
    assert result["data"]["home"]==expected=="/"


@pytest.mark.skipif(sys.platform not in ("win32","darwin"),reason="native case normalization acceptance gate")
def test_native_case_aliases_keep_python_project_identity(probe,tmp_path,isolated):
    data=tmp_path/"MixedCaseBook"/"Data"
    conn=db.open_db(data);conn.close()
    alias=data.parent.with_name(data.parent.name.swapcase())/data.name.swapcase()
    if not alias.is_dir():pytest.skip("filesystem is case sensitive")
    expected=ownership.canonical_project(data)
    for path in (data,alias):
        actual=_probe(probe,{"op":"identity","data_root":str(path),"owner_root":str(isolated[1])})["data"]
        assert actual["identity"]==expected.identity
        assert actual["database"]==str(expected.database)


@pytest.mark.skipif(os.name!="nt",reason="native Windows inherited OS handle acceptance gate")
def test_windows_broker_secret_uses_inherited_os_handle(probe,isolated):
    import msvcrt
    with tempfile.TemporaryFile() as secret:
        secret.write(b"Synthetic Windows handle\r\n");secret.seek(0)
        handle=msvcrt.get_osfhandle(secret.fileno())
        os.set_handle_inheritable(handle,True)
        startup=subprocess.STARTUPINFO(lpAttributeList={"handle_list":[handle]})
        try:
            result=subprocess.run([str(probe)],input=json.dumps({"op":"inherited-handle","handle":handle,"expected":"Synthetic Windows handle"})+"\n",text=True,capture_output=True,startupinfo=startup,timeout=30)
        finally:os.set_handle_inheritable(handle,False)
    assert result.returncode==0,result.stderr
    assert json.loads(result.stdout)["data"]=={"length":24,"matches":True}


def test_core_secret_build_does_not_link_a_native_async_runtime(isolated,installed_cargo):
    cargo,cargo_home=installed_cargo
    environment={**os.environ,"CARGO_HOME":cargo_home,"PATH":str(cargo.parent)+os.pathsep+os.environ["PATH"]}
    result=subprocess.run([str(cargo),"tree","-p","kassiber-secrets","--no-default-features","--locked","--offline","--prefix","none","--format","{p}","--edges","normal"],cwd=ROOT,text=True,capture_output=True,env=environment,timeout=30)
    assert result.returncode==0,result.stderr
    packages={line.split()[0] for line in result.stdout.splitlines()}
    assert not packages & {"tokio","zbus","secret-service","zbus-secret-service-keyring-store","keyring-core"}


@pytest.mark.parametrize("whitespace", [" ", "\t", "\n", "\r", "\v", "\f"])
def test_payment_hash_admission_matches_python_fromhex_whitespace(probe, tmp_path, isolated, whitespace):
    data = _book(tmp_path, isolated[1])
    conn = db.open_db(data)
    profile = conn.execute("SELECT id,workspace_id FROM profiles LIMIT 1").fetchone()
    candidate = "ab"*15 + whitespace*2 + "cd"*16
    assert len(candidate) == 64 and len(bytes.fromhex(candidate)) == 31
    conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('synthetic',?,?,'Synthetic','manual','fixed')", (profile["workspace_id"],profile["id"]))
    conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,raw_json,created_at) VALUES('synthetic-tx',?,?,'synthetic','synthetic-fp','fixed','inbound','BTC',1,?,'fixed')",(profile["workspace_id"],profile["id"],json.dumps({"payment_hash":candidate})))
    conn.commit()
    conn.close()
    result = _probe(probe,{"op":"open","data_root":str(data)},check=False)
    assert result["error"]["code"] == "requires_python_migration", result
    conn = db.open_db(data)
    assert conn.execute("SELECT payment_hash FROM transactions WHERE id='synthetic-tx'").fetchone()[0] == candidate
    conn.close()
    assert _probe(probe,{"op":"open","data_root":str(data)})["ok"]


@pytest.mark.parametrize("case", ["large-integer-txid", "collection-policy-asset"])
def test_lossy_json_values_cannot_hide_liquid_backfills(probe, tmp_path, isolated, case):
    data = _book(tmp_path, isolated[1])
    conn = db.open_db(data)
    profile = conn.execute("SELECT id,workspace_id FROM profiles LIMIT 1").fetchone()
    config = {"chain":"liquid"}
    raw = {"txid":int("1"*64)} if case == "large-integer-txid" else {"txid":"a"*64}
    if case == "collection-policy-asset":
        config["policy_asset"] = ["a"*56, ""]
        assert len(str(config["policy_asset"])) == 64
    conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('synthetic',?,?,'Synthetic','manual',?,'fixed')", (profile["workspace_id"],profile["id"],json.dumps(config)))
    conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,raw_json,created_at) VALUES('synthetic-tx',?,?,'synthetic','synthetic-fp','fixed','inbound','LBTC',1,?,'fixed')",(profile["workspace_id"],profile["id"],json.dumps(raw)))
    conn.commit()
    conn.close()
    result = _probe(probe,{"op":"open","data_root":str(data)},check=False)
    assert result["error"]["code"] == "unsupported_schema_layout", result
    conn = db.open_db(data)
    migrated = json.loads(conn.execute("SELECT raw_json FROM transactions WHERE id='synthetic-tx'").fetchone()[0])
    assert migrated["chain"] == "liquid"
    assert migrated["network"] == "liquidv1"
    conn.close()
