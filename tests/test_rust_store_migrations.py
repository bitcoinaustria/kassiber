"""R3 storage comparisons. Build the opt-in probe before running this module.

Every database lives in tmp_path; clock/identity are injected before creation,
not masked from results. This module never calls a real keyring or a provider.
"""
from __future__ import annotations

import json
import os
import shutil
from pathlib import Path
import sqlite3
import subprocess
import tempfile
from unittest.mock import patch
import uuid

import pytest

from kassiber import db
from kassiber.secrets import sqlcipher
from kassiber.operator import project as ownership

ROOT = Path(__file__).resolve().parents[1]
IDENTITY = "0123456789ab4def8123456789abcdef"
TIMESTAMP = "2026-10-27T12:34:56.789Z"
PASSPHRASE = "R3 synthetic '🔑"


@pytest.fixture(scope="module", autouse=True)
def _sweep_session_owner_locks(tmp_path_factory):
    root = tmp_path_factory.mktemp("r3-owner-namespace")
    root.chmod(0o700)
    with patch.object(ownership, "_owner_lock_root", return_value=root):
        yield


@pytest.fixture(scope="module")
def storage_probe():
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    binary = target / "debug/examples" / ("kassiber-storage-probe.exe" if os.name == "nt" else "kassiber-storage-probe")
    if not binary.is_file():
        pytest.skip("build cargo example kassiber-storage-probe with --features test-hooks")
    return binary


@pytest.fixture(autouse=True)
def disposable_environment(tmp_path, monkeypatch):
    home = tmp_path / "home"
    home.mkdir()
    for name in ("HOME", "USERPROFILE"):
        monkeypatch.setenv(name, str(home))
    monkeypatch.setenv("XDG_DATA_HOME", str(home / "xdg"))
    monkeypatch.setenv("KASSIBER_NO_EGRESS", "1")
    for name in ("KASSIBER_OPERATOR_CHILD", "KASSIBER_OPERATOR_EXPECTED_DATABASE_IDENTITY"):
        monkeypatch.delenv(name, raising=False)


def invoke(binary, root, *, encrypted=False, op="create", **options):
    request = dict(op=op, data_root=str(root), identity=IDENTITY, timestamp=TIMESTAMP, **options)
    with tempfile.TemporaryFile() as secret:
        if encrypted:
            secret.write(PASSPHRASE.encode())
            secret.seek(0)
            request["fd"] = secret.fileno()
        completed = subprocess.run([str(binary)], input=json.dumps(request) + "\n", text=True,
            capture_output=True, pass_fds=(secret.fileno(),) if encrypted else (), timeout=60)
    payload = json.loads(completed.stdout)
    return payload


def reference_open(root, encrypted=False):
    original = db.secrets_sqlcipher.open_encrypted if encrypted else sqlite3.connect

    def deterministic(*args, **kwargs):
        conn = original(*args, **kwargs)
        conn.create_function("strftime", 2, lambda *_: TIMESTAMP)
        return conn

    target = db.secrets_sqlcipher if encrypted else db.sqlite3
    method = "open_encrypted" if encrypted else "connect"
    with patch.object(target, method, deterministic), patch.object(db.uuid, "uuid4", return_value=uuid.UUID(hex=IDENTITY)):
        return db.open_db(root, passphrase=PASSPHRASE if encrypted else None)


def normalized_dump(conn):
    """All schema objects and values; table rows use PK order, then all columns.

    Internal autoindex DDL is represented by its owning table's SQL; SQLite's
    implementation-generated autoindex names aren't a format commitment.
    """
    objects = [tuple(row) for row in conn.execute("SELECT type,name,tbl_name,sql FROM sqlite_master WHERE name NOT GLOB 'sqlite_*' ORDER BY type,name")]
    tables = {}
    for (table,) in conn.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT GLOB 'sqlite_*' ORDER BY name"):
        quote = lambda text: '"' + text.replace('"', '""') + '"'
        columns = list(conn.execute(f"PRAGMA table_info({quote(table)})"))
        primary = [row[1] for row in sorted(columns, key=lambda row: row[5]) if row[5]]
        order = list(dict.fromkeys(primary + [row[1] for row in columns]))
        rows = conn.execute(f"SELECT * FROM {quote(table)} ORDER BY {','.join(map(quote, order))}").fetchall()
        tables[table] = ([row[1] for row in columns], [tuple(row) for row in rows])
    return objects, tables


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
def test_new_book_exact_schema_rows_and_python_reopen(storage_probe, tmp_path, encrypted):
    python = tmp_path / "python/data"
    rust = tmp_path / "rust/data"
    reference = reference_open(python, encrypted)
    expected = normalized_dump(reference)
    reference.close()
    result = invoke(storage_probe, rust, encrypted=encrypted)
    assert result["ok"], result
    database = rust / "kassiber.sqlite3"
    actual = sqlcipher.open_encrypted(database, PASSPHRASE, enforce_operator_identity=False) if encrypted else sqlite3.connect(database)
    assert normalized_dump(actual) == expected
    assert actual.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    assert actual.execute("PRAGMA foreign_key_check").fetchall() == []
    actual.close()
    reopened = reference_open(rust, encrypted)
    assert normalized_dump(reopened) == expected
    reopened.close()
    assert invoke(storage_probe, rust, encrypted=encrypted, op="open")["ok"]
    assert invoke(storage_probe, python, encrypted=encrypted, op="open")["ok"]


def test_creation_transcript_cannot_drift_from_python():
    result = subprocess.run([str(ROOT / ".venv/bin/python"), str(ROOT / "crates/store/tools/export_creation.py"), "--check"], cwd=ROOT, text=True, capture_output=True, timeout=60)
    assert result.returncode == 0, result.stderr
    result = subprocess.run([str(ROOT / ".venv/bin/python"), str(ROOT / "crates/store/tools/export_semantics.py"), "--check"], cwd=ROOT, text=True, capture_output=True, timeout=60)
    assert result.returncode == 0, result.stderr


@pytest.mark.parametrize("migration", [
    "wages-acquisition-semantics-v1", "rp2-pool-basis-semantics-v1",
    "custody-fail-closed-holds-v1", "kassiber-tax-engine-v1", "at-acquisition-valuation-support-v1", "all",
])
def test_semantic_audits_profile_changes_and_idempotency_match_python(storage_probe, tmp_path, migration):
    source = tmp_path / "source/data"
    conn = reference_open(source)
    conn.executescript("""
        INSERT INTO workspaces VALUES('ws','Synthetic','fixed');
        INSERT INTO profiles(id,workspace_id,label,tax_country,gains_algorithm,last_processed_at,created_at)
            VALUES('at','ws','AT','at','FIFO','fixed','fixed'),
                  ('pool','ws','Pool','generic','moving_average','fixed','fixed'),
                  ('fifo','ws','FIFO','generic','FIFO','fixed','fixed');
        INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('wal','ws','at','Wallet','custom','fixed');
        INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,kind,created_at)
            VALUES('wage','ws','at','wal','wage','fixed','inbound','BTC',1,' WAGES ','fixed'),
                  ('airdrop','ws','at','wal','airdrop','fixed','inbound','BTC',1,'airdrop','fixed');
    """)
    if migration == "all":
        conn.execute("DELETE FROM schema_migration_audits")
    else:
        conn.execute("DELETE FROM schema_migration_audits WHERE migration_name=?", (migration,))
    conn.commit()
    conn.close()
    python, rust = tmp_path / "python/data", tmp_path / "rust/data"
    shutil.copytree(source, python)
    shutil.copytree(source, rust)
    expected = reference_open(python)
    result = invoke(storage_probe, rust, op="semantics")
    assert result["ok"], result
    actual = sqlite3.connect(rust / "kassiber.sqlite3")
    assert normalized_dump(actual) == normalized_dump(expected)
    before = normalized_dump(actual)
    actual.close()
    assert invoke(storage_probe, rust, op="semantics")["ok"]
    actual = reference_open(rust)
    assert normalized_dump(actual) == before
    actual.close()
    expected.close()


@pytest.mark.parametrize("resume", ["python", "rust"])
@pytest.mark.parametrize("checkpoint", ["schema.after_file_create", "schema.after_statement.200", "schema.before_commit", "schema.after_commit"])
def test_killed_creation_resumes_without_partial_schema(storage_probe, tmp_path, resume, checkpoint):
    root = tmp_path / "interrupted/data"
    request = dict(op="create", data_root=str(root), identity=IDENTITY, timestamp=TIMESTAMP, kill_at=checkpoint)
    process = subprocess.Popen([str(storage_probe)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        process.stdin.write(json.dumps(request) + "\n")
        process.stdin.flush()
        assert json.loads(process.stdout.readline()) == {"checkpoint": checkpoint}
    finally:
        process.kill()
        process.communicate(timeout=10)
    if resume == "rust" and checkpoint != "schema.after_commit":
        result = invoke(storage_probe, root)
        assert result["ok"], result
    conn = reference_open(root)
    control = reference_open(tmp_path / "control/data")
    assert normalized_dump(conn) == normalized_dump(control)
    conn.close()
    control.close()


@pytest.mark.parametrize("with_indexes", [False, True])
def test_device_sync_removal_audit_and_rows_equal_python(storage_probe, tmp_path, with_indexes):
    source = tmp_path / "source/data"
    conn = reference_open(source)
    for table in db.RETIRED_DEVICE_SYNC_TABLES:
        conn.execute(f"CREATE TABLE {table}(id TEXT, status TEXT)")
    for table, rows in (("sync_books", [("book", "open")]), ("sync_events", [("event-a", "open"), ("event-b", "closed")]), ("sync_conflicts", [("conflict-a", "open"), ("conflict-b", "closed")]), ("sync_transports", [("transport", "open")])):
        conn.executemany(f"INSERT INTO {table} VALUES(?,?)", rows)
    if with_indexes:
        for name, definition in (("sync_event_id", "TEXT"), ("sync_replica_id", "TEXT"), ("sync_replica_seq", "INTEGER")):
            conn.execute(f"ALTER TABLE transaction_edit_events ADD COLUMN {name} {definition}")
        conn.execute("CREATE INDEX idx_transaction_edit_events_sync_event ON transaction_edit_events(sync_event_id)")
        conn.execute("CREATE INDEX idx_transaction_edit_events_sync_replica_seq ON transaction_edit_events(sync_replica_id,sync_replica_seq)")
    conn.commit()
    conn.close()
    python, rust = tmp_path / "python/data", tmp_path / "rust/data"
    shutil.copytree(source, python)
    shutil.copytree(source, rust)
    expected = reference_open(python)
    result = invoke(storage_probe, rust, op="retired")
    assert result["ok"], result
    actual = sqlite3.connect(rust / "kassiber.sqlite3")
    assert normalized_dump(actual) == normalized_dump(expected)
    before = normalized_dump(actual)
    actual.close()
    assert invoke(storage_probe, rust, op="retired")["ok"]
    actual = reference_open(rust)
    assert normalized_dump(actual) == before
    actual.close()
    expected.close()


@pytest.mark.parametrize("populated", [False, True])
def test_removed_ledger_retention_matches_python(storage_probe, tmp_path, populated):
    from tests.test_legacy_ledger import LEGACY_DDL
    source = tmp_path / "source/data"
    conn = reference_open(source)
    conn.executescript("""
        INSERT INTO workspaces VALUES('ws','Synthetic','fixed');
        INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('pf','ws','Synthetic','fixed');
        INSERT INTO external_documents(id,workspace_id,profile_id,document_type,label,created_at,updated_at)
        VALUES('doc','ws','pf','invoice','Synthetic','fixed','fixed');
    """)
    conn.executescript(LEGACY_DDL)
    if populated:
        conn.executescript("""
            INSERT INTO gl_books VALUES('pf','EUR','fixed');
            INSERT INTO gl_entries VALUES('entry','pf','Synthetic');
            INSERT INTO gl_lines VALUES('entry','pf',1);
            INSERT INTO gl_evidence VALUES('evidence','pf',x'00','doc');
        """)
    conn.close()
    python, rust = tmp_path / "python/data", tmp_path / "rust/data"
    shutil.copytree(source, python)
    shutil.copytree(source, rust)
    expected = reference_open(python)
    result = invoke(storage_probe, rust, op="retired")
    assert result["ok"], result
    actual = sqlite3.connect(rust / "kassiber.sqlite3")
    assert normalized_dump(actual) == normalized_dump(expected)
    assert actual.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    if populated:
        with pytest.raises(sqlite3.IntegrityError, match="accounting_evidence_retained"):
            actual.execute("DELETE FROM external_documents WHERE id='doc'")
    actual.close()
    expected.close()


def test_retired_storage_contract_cannot_drift_from_python():
    result = subprocess.run([str(ROOT / ".venv/bin/python"), str(ROOT / "crates/store/tools/export_retired.py"), "--check"], cwd=ROOT, text=True, capture_output=True, timeout=60)
    assert result.returncode == 0, result.stderr


def root_snapshot(home):
    import stat
    result = {}
    for path in sorted(home.rglob("*")):
        info = path.lstat()
        relative = str(path.relative_to(home))
        if path.is_symlink():
            result[relative] = ("symlink", os.readlink(path))
        elif stat.S_ISREG(info.st_mode):
            result[relative] = ("file", path.read_bytes())
        elif stat.S_ISDIR(info.st_mode):
            result[relative] = ("directory",)
        else:
            result[relative] = ("special", stat.S_IFMT(info.st_mode))
    return result


def legacy_root_fixture(home, case="normal"):
    legacy = home / ".kassiber"
    preferred = home / "xdg/kassiber"
    (legacy / "bin").mkdir(parents=True, mode=0o700)
    (legacy / "bin/launcher").write_bytes(b"fixed synthetic launcher")
    (legacy / "run").mkdir(mode=0o700)
    if case == "bin-only":
        return legacy, preferred
    conn = reference_open(legacy / "data")
    conn.close()
    (legacy / "config").mkdir(mode=0o700)
    catalog = {"schema_version": 1, "projects": [
        {"id": "default", "name": "Synthetic", "path": str(legacy / "projects/default"), "encrypted": False, "last_opened_at": None},
        {"id": "external", "name": "External", "path": str(home / "external"), "encrypted": False, "last_opened_at": None},
    ], "selected_project_id": "default", "extra": "preserved"}
    (legacy / "config/projects.json").write_text(json.dumps(catalog, indent=2, sort_keys=True) + "\n")
    (legacy / "config/projects.json").chmod(0o600)
    (legacy / "unknown-local-state").write_bytes(b"keep unknown state")
    if case == "native-conflict":
        preferred.mkdir(parents=True)
        (preferred / "independent").write_bytes(b"never merge")
    elif case == "legacy-symlink":
        (legacy / "data/link").symlink_to("kassiber.sqlite3")
    elif case == "fixed-bin-symlink":
        shutil.rmtree(legacy / "bin")
        (home / "external-bin").mkdir()
        (legacy / "bin").symlink_to(home / "external-bin", target_is_directory=True)
    elif case == "fifo":
        os.mkfifo(legacy / "retained-fifo")
    return legacy, preferred


@pytest.mark.skipif(os.name == "nt", reason="POSIX legacy-root differential fixture")
@pytest.mark.parametrize("case", ["normal", "native-conflict", "legacy-symlink", "bin-only", "fixed-bin-symlink", "fifo"])
def test_legacy_root_move_matches_python_without_copying_state(storage_probe, tmp_path, case):
    home = tmp_path / "scenario/home"
    legacy, preferred = legacy_root_fixture(home, case)
    seed = tmp_path / "seed"
    shutil.copytree(home, seed, symlinks=True, ignore=lambda directory, names: [name for name in names if name == "retained-fifo"])
    # FIFO has no file contents to copy; recreate the same synthetic entry.
    if case == "fifo":
        os.mkfifo(seed / ".kassiber/retained-fifo")
    environment = {"XDG_DATA_HOME": str(home / "xdg")}
    expected_path = db.migrate_hidden_home_state_root_if_needed(home=home, environ=environment, platform="linux")
    expected = root_snapshot(home)
    shutil.rmtree(home)
    shutil.copytree(seed, home, symlinks=True, ignore=lambda directory, names: [name for name in names if name == "retained-fifo"])
    if case == "fifo":
        os.mkfifo(legacy / "retained-fifo")
    database = legacy / "data/kassiber.sqlite3"
    identity = database.stat().st_ino if database.exists() else None
    result = invoke(storage_probe, home / "data", op="state-root", home=str(home), platform="linux", environ=environment)
    assert result["ok"], result
    assert result["data"]["state_root"] == str(expected_path)
    assert root_snapshot(home) == expected
    if identity is not None:
        assert (Path(result["data"]["state_root"]) / "data/kassiber.sqlite3").stat().st_ino == identity


@pytest.mark.skipif(os.name == "nt", reason="POSIX flock interop")
def test_active_python_owner_blocks_legacy_root_cutover(storage_probe, tmp_path):
    home = tmp_path / "scenario/home"
    legacy, preferred = legacy_root_fixture(home)
    environment = {"XDG_DATA_HOME": str(home / "xdg")}
    project = ownership.canonical_project(legacy / "data")
    lease = ownership.acquire_project_ownership(project, owner_kind="desktop", generation="synthetic-root-test")
    try:
        result = invoke(storage_probe, home / "data", op="state-root", home=str(home), platform="linux", environ=environment)
        assert result["ok"], result
        assert result["data"]["state_root"] == str(legacy)
        assert not preferred.exists()
        assert not (legacy / ".state-root-migration.json").exists()
    finally:
        lease.release()
    result = invoke(storage_probe, home / "data", op="state-root", home=str(home), platform="linux", environ=environment)
    assert result["ok"], result
    assert result["data"]["state_root"] == str(preferred)


@pytest.mark.skipif(os.name == "nt", reason="POSIX killed-child root fixture")
@pytest.mark.parametrize("resume", ["python", "rust"])
@pytest.mark.parametrize("checkpoint", ["proof.file.after_sync", "state-root.before_rename", "state-root.after_rename", "state-root.after_parent_sync", "state-root.after_restore.bin", "state-root.after_restore.run", "catalog.file.after_replace"])
def test_killed_legacy_root_migration_resumes_in_either_runtime(storage_probe, tmp_path, resume, checkpoint):
    home = tmp_path / "scenario/home"
    legacy, preferred = legacy_root_fixture(home)
    original = legacy / "data/kassiber.sqlite3"
    identity, content = original.stat().st_ino, original.read_bytes()
    environment = {"XDG_DATA_HOME": str(home / "xdg")}
    request = dict(op="state-root", data_root=str(home / "data"), home=str(home), platform="linux", environ=environment, kill_at=checkpoint)
    process = subprocess.Popen([str(storage_probe)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        process.stdin.write(json.dumps(request) + "\n")
        process.stdin.flush()
        assert json.loads(process.stdout.readline()) == {"checkpoint": checkpoint}
    finally:
        process.kill()
        process.communicate(timeout=10)
    if resume == "python":
        assert db.migrate_hidden_home_state_root_if_needed(home=home, environ=environment, platform="linux") == preferred
    else:
        result = invoke(storage_probe, home / "data", op="state-root", home=str(home), platform="linux", environ=environment)
        assert result["ok"], result
        assert result["data"]["state_root"] == str(preferred)
    database = preferred / "data/kassiber.sqlite3"
    assert database.read_bytes() == content
    assert database.stat().st_ino == identity
    assert json.loads((preferred / ".state-root-migration.json").read_text()) == {"schema_version": 1, "migrated_from_hidden_home": True}
    assert (legacy / "bin/launcher").read_bytes() == b"fixed synthetic launcher"
