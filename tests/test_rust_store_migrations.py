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
    # Build the probe rather than skipping without it: a missing binary used
    # to skip the whole storage acceptance suite silently in CI and the gate.
    subprocess.run(
        ["cargo", "build", "--locked", "-q", "-p", "kassiber-store",
         "--example", "kassiber-storage-probe", "--features", "test-hooks"],
        cwd=ROOT, check=True, timeout=900,
    )
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    if not target.is_absolute():
        target = ROOT / target
    return target / "debug/examples" / ("kassiber-storage-probe.exe" if os.name == "nt" else "kassiber-storage-probe")


@pytest.fixture(autouse=True)
def disposable_environment(tmp_path, monkeypatch):
    home = tmp_path / "home"
    home.mkdir()
    for name in ("HOME", "USERPROFILE"):
        monkeypatch.setenv(name, str(home))
    monkeypatch.setenv("XDG_DATA_HOME", str(home / "xdg"))
    monkeypatch.setenv("KASSIBER_NO_EGRESS", "1")
    for name in ("KASSIBER_OPERATOR_CHILD", "KASSIBER_OPERATOR_EXPECTED_DATABASE_IDENTITY", "KASSIBER_OPERATOR_EXPECTED_PROJECT_IDENTITY"):
        monkeypatch.delenv(name, raising=False)


def invoke(binary, root, *, encrypted=False, op="create", passphrase=PASSPHRASE, rotate_to=None, **options):
    request = dict(op=op, data_root=str(root), owner_root=str(root.parent / "owners"), identity=IDENTITY, timestamp=TIMESTAMP)
    request.update(options)
    with tempfile.TemporaryFile() as secret, tempfile.TemporaryFile() as new_secret:
        descriptors = []
        if encrypted:
            secret.write(passphrase.encode())
            secret.seek(0)
            request["fd"] = secret.fileno()
            descriptors.append(secret.fileno())
        if rotate_to is not None:
            new_secret.write(rotate_to.encode())
            new_secret.seek(0)
            request["new_fd"] = new_secret.fileno()
            descriptors.append(new_secret.fileno())
        completed = subprocess.run([str(binary)], input=json.dumps(request) + "\n", text=True,
            capture_output=True, pass_fds=tuple(descriptors), timeout=60)
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

    Include internal tables/autoindexes and each value's SQLite storage class.
    Float hex preserves the exact binary value rather than Python numeric ==.
    """
    objects = [tuple(row) for row in conn.execute("SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY type,name")]
    tables = {}
    for (table,) in conn.execute("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"):
        quote = lambda text: '"' + text.replace('"', '""') + '"'
        columns = list(conn.execute(f"PRAGMA table_info({quote(table)})"))
        primary = [row[1] for row in sorted(columns, key=lambda row: row[5]) if row[5]]
        order = list(dict.fromkeys(primary + [row[1] for row in columns]))
        storage_classes = ','.join(f'typeof({quote(row[1])})' for row in columns)
        rows = conn.execute(f"SELECT *,{storage_classes} FROM {quote(table)} ORDER BY {','.join(map(quote, order))}").fetchall()
        typed_rows = [tuple((kind, value.hex() if kind == 'real' else value) for kind,value in zip(row[len(columns):],row[:len(columns)])) for row in rows]
        tables[table] = ([row[1] for row in columns], typed_rows)
    return objects, tables


HISTORICAL_FIXTURES = sorted((ROOT / "tests/fixtures/historical").glob("*.sqlite3.gz.b64"))
LEGACY_PAIR_FIXTURES = [
    ROOT / "tests/fixtures/historical" / name
    for name in ("pre_432_5d232097.sqlite3.gz.b64", "pre_435_16b7bdc1.sqlite3.gz.b64")
]


def decode_historical(root, fixture, encrypted, *, without_legacy_pair=False):
    import base64
    import gzip

    root.mkdir(parents=True)
    database = root / "kassiber.sqlite3"
    database.write_bytes(gzip.decompress(base64.b64decode(fixture.read_bytes())))
    if without_legacy_pair:
        # Explicit test variant: remove only the reviewed compatibility row
        # whose authored consolidation belongs to R2. Keep observations and
        # concurrent authored components exactly as the historical book had them.
        with sqlite3.connect(database) as conn:
            assert conn.execute("SELECT COUNT(*) FROM transaction_pairs").fetchone()[0] == 1
            conn.execute("DELETE FROM transaction_pairs")
    if encrypted:
        cipher = root / "cipher.sqlite3"
        source = sqlcipher.require_sqlcipher().connect(str(database))
        try:
            source.execute("ATTACH DATABASE ? AS encrypted KEY " + sqlcipher.escape_passphrase(PASSPHRASE), (str(cipher),))
            source.execute("PRAGMA encrypted.cipher_compatibility=4")
            source.execute("PRAGMA encrypted.kdf_iter=256000")
            source.execute("PRAGMA encrypted.cipher_page_size=4096")
            source.execute("SELECT sqlcipher_export('encrypted')")
            source.execute("DETACH DATABASE encrypted")
        finally:
            source.close()
        cipher.replace(database)


@pytest.mark.parametrize("fixture", HISTORICAL_FIXTURES, ids=lambda path: path.name)
@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
def test_ordered_historical_migration_exact_dump_and_python_reopen(
    storage_probe, tmp_path, fixture, encrypted,
):
    """Pair-free variants match Python's first pass and unchanged reopen.

    Compare every schema object and typed row without masking or running an
    extra reference pass before comparison.
    """
    python, rust = tmp_path / "python/data", tmp_path / "rust/data"
    for root in (python, rust):
        decode_historical(root, fixture, encrypted, without_legacy_pair=fixture in LEGACY_PAIR_FIXTURES)
    expected_conn = reference_open(python, encrypted)
    expected = normalized_dump(expected_conn)
    expected_conn.close()
    result = invoke(storage_probe, rust, encrypted=encrypted, op="migrate")
    assert result["ok"], result
    actual = sqlcipher.open_encrypted(rust / "kassiber.sqlite3", PASSPHRASE, enforce_operator_identity=False) if encrypted else sqlite3.connect(rust / "kassiber.sqlite3")
    assert normalized_dump(actual) == expected
    assert actual.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    assert actual.execute("PRAGMA foreign_key_check").fetchall() == []
    actual.close()
    reopened = reference_open(rust, encrypted)
    assert normalized_dump(reopened) == expected
    reopened.close()
    assert invoke(storage_probe, rust, encrypted=encrypted, op="migrate")["ok"]
    reopened = reference_open(rust, encrypted)
    assert normalized_dump(reopened) == expected
    reopened.close()


@pytest.mark.parametrize("fixture", LEGACY_PAIR_FIXTURES, ids=lambda path: path.name)
@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
def test_ordered_historical_original_refuses_at_r2_with_exact_prefix(
    storage_probe, tmp_path, fixture, encrypted,
):
    from kassiber.core import custody_authored_migration

    python, rust = tmp_path / "python/data", tmp_path / "rust/data"
    for root in (python, rust):
        decode_historical(root, fixture, encrypted)

    class ReachedR2(Exception):
        pass

    def stop_before_authored_consolidation(conn):
        nonlocal expected
        # Capture the complete prefix at entry to the deferred Python step.
        # No authored consolidation is stubbed into a successful migration.
        conn.commit()
        expected = normalized_dump(conn)
        raise ReachedR2

    expected = None
    with patch.object(custody_authored_migration, "refresh_legacy_authored_components", stop_before_authored_consolidation):
        with pytest.raises(ReachedR2):
            reference_open(python, encrypted)
    assert expected is not None
    result = invoke(storage_probe, rust, encrypted=encrypted, op="migrate")
    assert not result["ok"], result
    assert result["error"]["code"] == "unsupported_schema_layout"
    assert result["error"]["details"] == {
        "migration_step": "refresh_legacy_authored_components",
        "reason": "legacy_authored_custody",
    }
    actual = sqlcipher.open_encrypted(rust / "kassiber.sqlite3", PASSPHRASE, enforce_operator_identity=False) if encrypted else sqlite3.connect(rust / "kassiber.sqlite3")
    assert normalized_dump(actual) == expected
    assert actual.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    assert actual.execute("PRAGMA foreign_key_check").fetchall() == []
    actual.close()
    # The original book is still recoverable by the reference implementation.
    resumed = reference_open(rust, encrypted)
    assert resumed.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    resumed.close()


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
    "custody-fail-closed-holds-v1", "kassiber-tax-engine-v1", "at-acquisition-valuation-support-v1",
    "custody-partial-payout-pricing-v1", "journal-calculation-semantics-adoption-v1",
    "at-outflow-consumed-regime-v2", "all",
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
    if migration in ("at-outflow-consumed-regime-v2", "all"):
        conn.execute("""INSERT INTO journal_entries(id,workspace_id,profile_id,wallet_id,
            transaction_id,entry_type,occurred_at,asset,quantity,created_at)
            VALUES('sale','ws','at','wal','wage','disposal','2024-01-01','BTC',-1,'fixed')""")
        conn.execute("""INSERT INTO schema_migration_audits VALUES(
            'at-fee-consumed-regime-v1','at-fee-consumed-regime-v1',1,'{}','fixed')""")
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


@pytest.mark.parametrize("partial,deleted,included_fee", [
    (True, False, False), (False, False, False), (True, True, False), (False, False, True),
])
def test_partial_payout_semantic_migration_matches_python(storage_probe, tmp_path, partial, deleted, included_fee):
    from kassiber.cli.handlers import create_direct_swap_payout
    from kassiber.core.custody_review_terms import delete_payout_review
    from tests.test_custody_authored_migration import _scope, _tx, NOW

    source = tmp_path / "source/data"
    conn = reference_open(source)
    _scope(conn)
    _tx(conn, "out", "btc", "outbound", "BTC", 100000, NOW)
    if included_fee:
        conn.execute("UPDATE transactions SET amount=110000,fee=10000,amount_includes_fee=1 WHERE id='out'")
    conn.commit()
    payout = create_direct_swap_payout(conn, "Books", "Book", "out", payout_asset="EUR",
                                      payout_amount="1", payout_fiat_value="10",
                                      out_amount="0.0000004" if partial else None)
    if deleted:
        delete_payout_review(conn, "profile", payout["id"])
    conn.execute("UPDATE profiles SET last_processed_at=? WHERE id='profile'", (NOW,))
    conn.execute("DELETE FROM schema_migration_audits WHERE migration_name='custody-partial-payout-pricing-v1'")
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
    actual = sqlite3.connect(rust / "kassiber.sqlite3")
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


@pytest.mark.parametrize("writer", ["python", "rust"])
@pytest.mark.parametrize("auto_vacuum", [0,1,2])
def test_plaintext_encryption_and_rekey_cross_language(storage_probe, tmp_path, writer,auto_vacuum):
    from kassiber.secrets.migration import migrate_plaintext_to_encrypted
    from kassiber.secrets.passphrase import change_database_passphrase

    root = tmp_path / "book/data"
    conn = reference_open(root)
    conn.execute(f"PRAGMA auto_vacuum={auto_vacuum}")
    conn.execute("VACUUM")
    conn.execute("PRAGMA user_version=73")
    conn.execute("INSERT INTO settings VALUES('synthetic_secret','_TOKEN=synthetic_secret')")
    conn.commit()
    before = normalized_dump(conn)
    conn.close()
    if writer == "rust":
        result = invoke(storage_probe, root, encrypted=True, op="encrypt")
        assert result["ok"], result
        assert result["data"]["user_version"] == 73
    else:
        migrate_plaintext_to_encrypted(root / "kassiber.sqlite3", PASSPHRASE)
    assert invoke(storage_probe, root, encrypted=True, op="open")["ok"]
    conn = sqlcipher.open_encrypted(root / "kassiber.sqlite3", PASSPHRASE, enforce_operator_identity=False)
    assert normalized_dump(conn) == before
    assert conn.execute("PRAGMA user_version").fetchone()[0] == 73
    assert conn.execute("PRAGMA auto_vacuum").fetchone()[0] == auto_vacuum
    conn.close()


    backup = sqlite3.connect(root / "kassiber.pre-encryption.sqlite3.bak")
    assert normalized_dump(backup) == before
    backup.close()
    new = "Rotated R3 '🔐"
    if writer == "rust":
        result = invoke(storage_probe, root, encrypted=True, op="rekey", rotate_to=new)
        assert result["ok"], result
    else:
        change_database_passphrase(root / "kassiber.sqlite3", PASSPHRASE, new)
    assert invoke(storage_probe, root, encrypted=True, passphrase=new, op="open")["ok"]
    conn = sqlcipher.open_encrypted(root / "kassiber.sqlite3", new, enforce_operator_identity=False)
    assert normalized_dump(conn) == before
    assert conn.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    assert conn.execute("PRAGMA foreign_key_check").fetchall() == []
    conn.close()


@pytest.mark.parametrize("encrypted",[False,True])
def test_python_backup_api_restores_a_rust_written_book(storage_probe,tmp_path,encrypted):
    source,destination=tmp_path/"source/data",tmp_path/"restored/data"
    assert invoke(storage_probe,source,encrypted=encrypted)["ok"]
    assert invoke(storage_probe,destination,encrypted=encrypted)["ok"]
    connect=lambda root:sqlcipher.open_encrypted(root/"kassiber.sqlite3",PASSPHRASE,enforce_operator_identity=False) if encrypted else sqlite3.connect(root/"kassiber.sqlite3")
    original,restored=connect(source),connect(destination)
    try:
        original.execute("INSERT INTO settings VALUES('r3_backup','exact restored value')");original.commit()
        original.backup(restored)
        assert normalized_dump(restored)==normalized_dump(original)
    finally:original.close();restored.close()
    result=invoke(storage_probe,destination,encrypted=encrypted,op="open")
    assert result["ok"],result


@pytest.mark.parametrize("checkpoint", [
    "encryption.after_export", "encryption.before_verified_sync", "encryption.after_verified_sync",
    "encryption.before_backup_rename", "encryption.after_backup_rename",
    "encryption.before_backup_directory_sync", "encryption.after_backup_directory_sync",
    "encryption.before_publish_rename", "encryption.after_publish_rename",
    "encryption.before_publish_directory_sync", "encryption.after_publish_directory_sync",
])
def test_killed_encryption_resumes_and_preserves_both_copies(storage_probe, tmp_path, checkpoint):
    root = tmp_path / "book/data"
    conn = reference_open(root)
    before = normalized_dump(conn)
    conn.close()
    request = dict(op="encrypt", data_root=str(root), owner_root=str(tmp_path / "owners"), kill_at=checkpoint)
    with tempfile.TemporaryFile() as secret:
        secret.write(PASSPHRASE.encode())
        secret.seek(0)
        request["fd"] = secret.fileno()
        process = subprocess.Popen([str(storage_probe)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, text=True, pass_fds=(secret.fileno(),))
        try:
            process.stdin.write(json.dumps(request) + "\n")
            process.stdin.flush()
            assert json.loads(process.stdout.readline()) == {"checkpoint": checkpoint}
        finally:
            process.kill()
            process.communicate(timeout=10)
    if (root / "kassiber.encrypted.sqlite3").exists():
        result = invoke(storage_probe, root, encrypted=True, op="resume-encryption")
        assert result["ok"], result
    conn = sqlcipher.open_encrypted(root / "kassiber.sqlite3", PASSPHRASE, enforce_operator_identity=False)
    assert normalized_dump(conn) == before
    conn.close()
    backup = sqlite3.connect(root / "kassiber.pre-encryption.sqlite3.bak")
    assert normalized_dump(backup) == before
    backup.close()


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


def test_creation_refuses_a_missing_child_project_binding_before_writing(storage_probe, tmp_path, monkeypatch):
    root = tmp_path / "uncreated/data"
    monkeypatch.setenv("KASSIBER_OPERATOR_CHILD", "1")
    result = invoke(storage_probe, root)
    assert result["error"]["code"] == "operator_project_binding_invalid"
    assert not root.exists()


def historical_msat_starting_point(destination, method):
    """Reuse the reference tests' real legacy DDL before their migrating open."""
    from tests import test_msat_migration as fixture_module
    class CapturedFixture(Exception):
        pass
    calls = 0
    def capture(root, **kwargs):
        nonlocal calls
        calls += 1
        if calls == 2:
            shutil.copytree(root, destination)
            raise CapturedFixture
        return reference_open(Path(root))
    case = fixture_module.MsatMigrationTests(method)
    try:
        with patch.object(fixture_module, "open_db", capture):
            with pytest.raises(CapturedFixture):
                getattr(case, method)()
    finally:
        case.doCleanups()
    assert calls == 2 and (destination / "kassiber.sqlite3").is_file()


@pytest.mark.parametrize("method", [
    "test_legacy_transaction_rebuild_preserves_columns_added_after_real_schema",
    "test_legacy_derived_journal_quantities_migrate_to_msat",
])
def test_msat_historical_starting_points_match_python(storage_probe, tmp_path, method):
    source = tmp_path / "source/data"
    historical_msat_starting_point(source, method)
    python, rust = tmp_path / "python/data", tmp_path / "rust/data"
    shutil.copytree(source, python)
    shutil.copytree(source, rust)
    expected = reference_open(python)
    result = invoke(storage_probe, rust, op="msat")
    assert result["ok"], result
    actual = sqlite3.connect(rust / "kassiber.sqlite3")
    assert normalized_dump(actual) == normalized_dump(expected)
    assert actual.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    assert actual.execute("PRAGMA foreign_key_check").fetchall() == []
    before = normalized_dump(actual)
    actual.close()
    assert invoke(storage_probe, rust, op="msat")["ok"]
    actual = reference_open(rust)
    assert normalized_dump(actual) == before
    actual.close()
    expected.close()


def test_compatibility_sql_contract_cannot_drift():
    result = subprocess.run([str(ROOT / ".venv/bin/python"), str(ROOT / "crates/store/tools/export_compat.py"), "--check"], cwd=ROOT, text=True, capture_output=True, timeout=60)
    assert result.returncode == 0, result.stderr


@pytest.mark.skipif(os.name == "nt", reason="POSIX multi-process descriptor harness")
@pytest.mark.parametrize("encrypted", [False, True])
def test_concurrent_rust_and_python_writers_preserve_every_increment(storage_probe, tmp_path, encrypted):
    from contextlib import ExitStack
    root = tmp_path / "concurrent/data"
    conn = reference_open(root, encrypted)
    conn.execute("INSERT INTO settings VALUES('r3_counter','0')")
    conn.commit()
    children = []
    with ExitStack() as resources:
        try:
            for _ in range(3):
                request = dict(op="increment", data_root=str(root), count=20)
                descriptors = ()
                if encrypted:
                    secret = resources.enter_context(tempfile.TemporaryFile())
                    secret.write(PASSPHRASE.encode())
                    secret.seek(0)
                    request["fd"] = secret.fileno()
                    descriptors = (secret.fileno(),)
                child = subprocess.Popen([str(storage_probe)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, pass_fds=descriptors)
                child.stdin.write(json.dumps(request) + "\n")
                child.stdin.flush()
                children.append(child)
            for _ in range(20):
                conn.execute("BEGIN IMMEDIATE")
                conn.execute("UPDATE settings SET value=CAST(value AS INTEGER)+1 WHERE key='r3_counter'")
                conn.commit()
            for child in children:
                out, err = child.communicate(timeout=60)
                assert child.returncode == 0, err
                assert json.loads(out)["data"]["writes"] == 20
        finally:
            for child in children:
                if child.poll() is None:
                    child.kill()
                    child.communicate(timeout=10)
    assert conn.execute("SELECT value FROM settings WHERE key='r3_counter'").fetchone()[0] == "80"
    assert conn.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    assert conn.execute("PRAGMA foreign_key_check").fetchall() == []
    conn.close()


@pytest.mark.skipif(os.name == "nt", reason="POSIX held-writer descriptor harness")
@pytest.mark.parametrize("encrypted", [False, True])
def test_python_busy_retry_after_a_held_rust_write(storage_probe, tmp_path, encrypted):
    root = tmp_path / "busy/data"
    conn = reference_open(root, encrypted)
    conn.execute("INSERT INTO settings VALUES('r3_counter','0')")
    conn.commit()
    conn.execute("PRAGMA busy_timeout=10")
    with tempfile.TemporaryFile() as secret:
        request = dict(op="write-hold", data_root=str(root))
        if encrypted:
            secret.write(PASSPHRASE.encode())
            secret.seek(0)
            request["fd"] = secret.fileno()
        child = subprocess.Popen([str(storage_probe)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, pass_fds=(secret.fileno(),) if encrypted else ())
        try:
            child.stdin.write(json.dumps(request) + "\n")
            child.stdin.flush()
            assert json.loads(child.stdout.readline()) == {"checkpoint": "writer.holding"}
            with pytest.raises((sqlite3.OperationalError, sqlcipher.require_sqlcipher().OperationalError)) as error:
                conn.execute("BEGIN IMMEDIATE")
            assert sqlcipher.is_lock_contention(error.value)
            child.stdin.write("release\n")
            child.stdin.flush()
            out, err = child.communicate(timeout=60)
            assert child.returncode == 0, err
            assert json.loads(out)["ok"]
        finally:
            if child.poll() is None:
                child.kill()
                child.communicate(timeout=10)
    conn.execute("BEGIN IMMEDIATE")
    conn.execute("UPDATE settings SET value=CAST(value AS INTEGER)+1 WHERE key='r3_counter'")
    conn.commit()
    assert conn.execute("SELECT value FROM settings WHERE key='r3_counter'").fetchone()[0] == "2"
    assert conn.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    conn.close()


@pytest.mark.parametrize("all_missing", [False, True])
def test_graph_schema_installed_at_open_matches_python(storage_probe, tmp_path, all_missing):
    source = tmp_path / "source/data"
    conn = reference_open(source)
    conn.execute("PRAGMA foreign_keys=OFF")
    tables = [row[0] for row in conn.execute("SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'chain_index_%'")]
    for name in tables if all_missing else ["chain_index_edges"]:
        conn.execute(f'DROP TABLE "{name}"')
    for (name,) in conn.execute("SELECT name FROM sqlite_master WHERE type='trigger' AND name LIKE 'chain_index_track_%'").fetchall():
        conn.execute(f'DROP TRIGGER "{name}"')
    conn.commit()
    conn.close()
    python, rust = tmp_path / "python/data", tmp_path / "rust/data"
    shutil.copytree(source, python)
    shutil.copytree(source, rust)
    expected = reference_open(python)
    result = invoke(storage_probe, rust, op="graph-install")
    assert result["ok"], result
    actual = sqlite3.connect(rust / "kassiber.sqlite3")
    assert normalized_dump(actual) == normalized_dump(expected)
    assert actual.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    assert actual.execute("PRAGMA foreign_key_check").fetchall() == []
    actual.close()
    expected.close()


SCHEMA_STEPS = {
    "swap-schema": "_ensure_swap_matching_schema",
    "liquid": "_backfill_liquid_asset_codes",
    "bip329": "_ensure_bip329_wallet_agnostic_schema",
    "payment-hash": "_backfill_payment_hash_from_raw_json",
    "swap-fees": "_clear_stale_same_asset_swap_fees",
    "inbox": "ensure_inbox_sources",
    "gap-inputs": None,
    "leg-roles": "_ensure_custody_leg_role_schema",
    "relation-schema": "_ensure_custody_gap_review_transaction_schema",
    "review-relations": "_backfill_custody_gap_review_transactions",
    "evidence": "_backfill_local_custody_evidence_commitments",
    "attachments": "_migrate_attachment_table_shape",
    "pair-uniques": "_migrate_legacy_transaction_pairs_uniques",
    "recipient-unique": "_drop_legacy_source_funds_recipients_unique",
    "projection-shapes": "_ensure_custody_projection_table_shapes",
    "economic-notes": "_ensure_custody_economic_term_review_notes",
    "immutability": "_ensure_custody_revision_immutability_triggers",
    "direct-payout": "_ensure_direct_swap_payout_schema",
    "commercial": "_ensure_commercial_reconciliation_schema",
    "freshness": "_ensure_freshness_schema",
    "ai-refs": "_ensure_ai_provider_secret_refs_schema",
    "ai-checks": "_ensure_ai_provider_checks_schema",
    "ai-audit": "_ensure_custody_ai_audit_consent_schema",
    "graph-cache": "_ensure_transaction_graph_cache_schema",
    "projection-view": "_ensure_custody_projection_relation_view",
    "legacy-freeze": "_ensure_legacy_custody_write_freeze_triggers",
    "source-target": "_backfill_source_funds_target_external_id",
}


def reference_schema_step(conn, step):
    if step == "inbox":
        from kassiber.chain_analysis_watches_schema import ensure_inbox_sources
        ensure_inbox_sources(conn)
    elif step == "durable-evidence":
        import ast
        import inspect
        body = ast.parse(inspect.getsource(db.ensure_schema_compat)).body[0].body
        start = next(i for i,node in enumerate(body) if isinstance(node,ast.Assign) and isinstance(node.targets[0],ast.Name) and node.targets[0].id == "legacy_leg_columns")
        end = next(i for i,node in enumerate(body) if isinstance(node,ast.Expr) and isinstance(node.value,ast.Call) and isinstance(node.value.func,ast.Name) and node.value.func.id == "_ensure_custody_leg_role_schema")
        function = ast.FunctionDef(name="durable_step",args=ast.arguments(posonlyargs=[],args=[ast.arg(arg="conn")],kwonlyargs=[],kw_defaults=[],defaults=[]),body=body[start:end],decorator_list=[])
        scope = dict(vars(db))
        exec(compile(ast.fix_missing_locations(ast.Module(body=[function],type_ignores=[])),"<reference durable migration>","exec"),scope)
        scope["durable_step"](conn)
    elif step == "gap-inputs":
        import ast
        import inspect
        # Run the unchanged reference's exact cache-retirement block. It has
        # no separate helper; do not approximate its version conversion here.
        tree = ast.parse(inspect.getsource(db.ensure_schema_compat))
        body = tree.body[0].body
        start = next(index for index,node in enumerate(body) if isinstance(node,ast.Expr) and isinstance(node.value,ast.Call) and node.value.args and isinstance(node.value.args[0],ast.Constant) and node.value.args[0].value == "DROP TABLE IF EXISTS custody_gap_candidate_snapshots")
        end = next(index for index,node in enumerate(body[start:],start) if isinstance(node,ast.Expr) and isinstance(node.value,ast.Call) and isinstance(node.value.func,ast.Name) and node.value.func.id == "ensure_column")
        function = ast.FunctionDef(name="cache_step", args=ast.arguments(posonlyargs=[],args=[ast.arg(arg="conn")],kwonlyargs=[],kw_defaults=[],defaults=[]),body=body[start:end],decorator_list=[])
        module = ast.fix_missing_locations(ast.Module(body=[function],type_ignores=[]))
        scope = {"json":json}
        exec(compile(module,"<reference cache migration>","exec"),scope)
        scope["cache_step"](conn)
    else:
        getattr(db, SCHEMA_STEPS[step])(conn)


def compare_schema_step(storage_probe, tmp_path, source, step):
    python, rust = tmp_path / "python/data", tmp_path / "rust/data"
    shutil.copytree(source, python)
    shutil.copytree(source, rust)
    expected = sqlite3.connect(python / "kassiber.sqlite3")
    expected.row_factory = sqlite3.Row
    expected.execute("PRAGMA foreign_keys=ON")
    expected.create_function("strftime",2,lambda *_:TIMESTAMP)
    reference_schema_step(expected, step)
    expected.commit()
    result = invoke(storage_probe, rust, op="schema-step", step=step)
    assert result["ok"], result
    actual = sqlite3.connect(rust / "kassiber.sqlite3")
    assert normalized_dump(actual) == normalized_dump(expected)
    assert actual.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    # Some legacy fixtures intentionally contain old child FKs. Compare all
    # violations; do not mask them or pretend unrelated repairs ran.
    assert actual.execute("PRAGMA foreign_key_check").fetchall() == [tuple(row) for row in expected.execute("PRAGMA foreign_key_check")]
    before = normalized_dump(actual)
    actual.close()
    assert invoke(storage_probe, rust, op="schema-step", step=step)["ok"]
    actual = sqlite3.connect(rust / "kassiber.sqlite3")
    assert normalized_dump(actual) == before
    actual.row_factory = sqlite3.Row
    actual.execute("PRAGMA foreign_keys=ON")
    reference_schema_step(actual, step)
    actual.commit()
    assert normalized_dump(actual) == before
    actual.close()
    expected.close()


@pytest.mark.parametrize("step", SCHEMA_STEPS)
def test_schema_steps_match_python_on_current_books(storage_probe, tmp_path, step):
    source = tmp_path / "source/data"
    reference_open(source).close()
    compare_schema_step(storage_probe, tmp_path, source, step)


def attachment_starting_point(source, method):
    from tests import test_btcpay_commercial_provenance as fixtures
    source.mkdir(parents=True)
    class CapturedFixture(Exception):
        pass
    def capture(conn):
        conn.commit()
        with sqlite3.connect(source / "kassiber.sqlite3") as target:
            conn.backup(target)
        raise CapturedFixture
    case = fixtures.BtcpayCommercialProvenanceTest(method)
    case.setUp()
    try:
        with patch.object(fixtures, "_migrate_attachment_table_shape", capture):
            with pytest.raises(CapturedFixture):
                getattr(case, method)()
    finally:
        case.tearDown()
        case.doCleanups()


@pytest.mark.parametrize("method", [
    "test_nullable_attachment_migration_recovers_stranded_legacy_table",
    "test_attachment_migration_removes_copied_provenance_foreign_keys",
])
def test_attachment_reference_fixtures_match_python(storage_probe, tmp_path, method):
    source = tmp_path / "source/data"
    attachment_starting_point(source, method)
    compare_schema_step(storage_probe, tmp_path, source, "attachments")


@pytest.mark.parametrize("step", ["pair-uniques", "recipient-unique", "projection-shapes", "economic-notes", "commercial"])
def test_legacy_schema_repairs_match_python(storage_probe, tmp_path, step):
    source = tmp_path / "source/data"
    conn = reference_open(source)
    if step == "pair-uniques":
        from tests.test_schema_swap_matching import LegacyUniqueMigrationTests
        LegacyUniqueMigrationTests()._downgrade_transaction_pairs_to_legacy(conn)
    elif step == "recipient-unique":
        conn.execute("PRAGMA foreign_keys=OFF")
        ddl = conn.execute("SELECT sql FROM sqlite_master WHERE name='source_funds_recipients'").fetchone()[0]
        conn.execute("DROP TABLE source_funds_recipients")
        conn.execute(ddl.rstrip().rsplit(")", 1)[0] + ", UNIQUE (profile_id, label))")
    elif step == "projection-shapes":
        definitions = json.loads((ROOT / "crates/store/fixtures/compat.json").read_text())["functions"]["_ensure_custody_projection_table_shapes"]["constants"]["semantic_columns"]
        for table, names in definitions.items():
            for name in names:
                conn.execute(f'ALTER TABLE "{table}" ADD COLUMN "{name}" TEXT')
    elif step == "economic-notes":
        import ast
        import inspect
        from tests.test_custody_authored_migration import test_review_notes_schema_migration_backfills_and_refreezes_terms
        # Reuse the exact historical test schema and its authored pair/payout
        # rows, rather than making a fresh-schema approximation.
        conn.close()
        (source / "kassiber.sqlite3").unlink()
        conn = sqlite3.connect(source / "kassiber.sqlite3")
        tree = ast.parse(inspect.getsource(test_review_notes_schema_migration_backfills_and_refreezes_terms))
        script = next(node.args[0].value for node in ast.walk(tree) if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute) and node.func.attr == "executescript")
        conn.executescript(script)
    elif step == "commercial":
        conn.executescript("""
            INSERT INTO workspaces VALUES('ws','Synthetic','fixed');
            INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('pf','ws','Synthetic','fixed');
            DROP INDEX idx_external_documents_profile_external_ref_unique;
            INSERT INTO external_documents(id,workspace_id,profile_id,document_type,label,external_ref,created_at,updated_at)
                VALUES('a','ws','pf','invoice','First','duplicate','fixed','fixed'),('b','ws','pf','invoice','Second','duplicate','fixed','fixed');
        """)
    conn.commit()
    conn.close()
    compare_schema_step(storage_probe, tmp_path, source, step)


def narrow_ai_audit(conn):
    """Give a current book's custody AI audit #654's consent CHECK and two rows."""
    import re
    objects = {name: sql for name, sql in conn.execute(
        "SELECT name, sql FROM sqlite_master WHERE tbl_name='custody_ai_assistance_audits' AND sql IS NOT NULL")}
    table = objects.pop("custody_ai_assistance_audits")
    narrow = re.sub(r"'mode_auto', 'mode_full_access',\s*'mcp_host', ", "", table)
    assert narrow != table and "'mcp_host'" not in narrow
    conn.commit()
    conn.execute("PRAGMA foreign_keys=OFF")
    conn.execute("DROP TABLE custody_ai_assistance_audits")
    for sql in [narrow, *objects.values()]:
        conn.execute(sql)
    conn.execute("PRAGMA foreign_keys=ON")
    conn.executescript("""
        INSERT INTO workspaces VALUES('ws','Synthetic','fixed');
        INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('pf','ws','Synthetic','fixed');
    """)
    for ordinal, decision in enumerate(("allow_once", "deny")):
        conn.execute(
            "INSERT INTO custody_ai_assistance_audits(id,workspace_id,profile_id,tool_name,daemon_kind,call_id,provider_kind,model,"
            "facts_sha256,model_proposal_json,consent_decision,consent_requested_at,consent_decided_at,execution_status,created_at) "
            "VALUES(?,'ws','pf','tool','kind',?,'local','m',?,'{\"€\":1}',?,'t','t','executed',?)",
            (f"audit-{ordinal}", f"call-{ordinal}", f"{ordinal:064x}", decision, f"2025-01-0{ordinal + 1}"))
    conn.commit()


def ai_audit_starting_point(source):
    conn = reference_open(source)
    narrow_ai_audit(conn)
    conn.close()


def test_custody_ai_audit_consent_widening_matches_python(storage_probe, tmp_path):
    source = tmp_path / "source/data"
    ai_audit_starting_point(source)
    compare_schema_step(storage_probe, tmp_path, source, "ai-audit")
    conn = sqlite3.connect(tmp_path / "rust/data/kassiber.sqlite3")
    assert "'mcp_host'" in conn.execute("SELECT sql FROM sqlite_master WHERE name='custody_ai_assistance_audits'").fetchone()[0]
    with pytest.raises(sqlite3.IntegrityError, match="custody_ai_assistance_immutable"):
        conn.execute("UPDATE custody_ai_assistance_audits SET model='x'")
    conn.close()


@pytest.mark.skipif(os.name == "nt", reason="POSIX killed-child schema fixture")
@pytest.mark.parametrize("resume", ["python", "rust"])
@pytest.mark.parametrize("checkpoint", ["ai-audit.after_rename", "ai-audit.after_copy", "ai-audit.after_restore_objects", "repair.before_commit", "repair.after_commit"])
def test_killed_ai_audit_rebuild_keeps_every_record(storage_probe, tmp_path, resume, checkpoint):
    source = tmp_path / "source/data"
    ai_audit_starting_point(source)
    control, root = tmp_path / "control/data", tmp_path / "interrupted/data"
    shutil.copytree(source, control)
    shutil.copytree(source, root)
    expected = sqlite3.connect(control / "kassiber.sqlite3")
    expected.row_factory = sqlite3.Row
    expected.execute("PRAGMA foreign_keys=ON")
    db._ensure_custody_ai_audit_consent_schema(expected)
    child = subprocess.Popen([str(storage_probe)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        child.stdin.write(json.dumps(dict(op="schema-step", step="ai-audit", data_root=str(root), kill_at=checkpoint)) + "\n")
        child.stdin.flush()
        assert json.loads(child.stdout.readline()) == {"checkpoint": checkpoint}
    finally:
        child.kill()
        child.communicate(timeout=10)
    if resume == "rust":
        assert invoke(storage_probe, root, op="schema-step", step="ai-audit")["ok"]
    actual = sqlite3.connect(root / "kassiber.sqlite3")
    actual.row_factory = sqlite3.Row
    actual.execute("PRAGMA foreign_keys=ON")
    if resume == "python":
        db._ensure_custody_ai_audit_consent_schema(actual)
    assert normalized_dump(actual) == normalized_dump(expected)
    assert actual.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    actual.close()
    expected.close()


def populate_main_sync_storage(conn):
    """Rows in every storage object main added after #654."""
    conn.executescript("""
        INSERT INTO workspaces VALUES('ws','Synthetic','fixed');
        INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('pf','ws','Synthetic','fixed');
        INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('wal','ws','pf','Wallet','manual','fixed');
        INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,
            fiat_currency,fiat_rate,pricing_quality,pricing_review_accepted_price,created_at)
            VALUES('tx','ws','pf','wal','tx','2025-01-01T00:00:00Z','inbound','BTC',100000,'EUR','51234.5','coarse','51234.5','fixed');
        INSERT INTO ai_providers(name,display_name,base_url,kind,agent_access,created_at,updated_at)
            VALUES('synthetic','Synthetic','http://127.0.0.1:9/v1','local','restricted','fixed','fixed');
        INSERT INTO ai_provider_checks(provider_name,check_kind,base_url,checked_at,stale,error_json,value_json)
            VALUES('synthetic','models','http://127.0.0.1:9/v1','fixed',1,NULL,'{"models":["m"]}');
    """)
    for ordinal, decision in enumerate(("mode_auto", "mode_full_access", "mcp_host")):
        conn.execute(
            "INSERT INTO custody_ai_assistance_audits(id,workspace_id,profile_id,tool_name,daemon_kind,call_id,provider_kind,model,"
            "facts_sha256,consent_decision,consent_requested_at,consent_decided_at,execution_status,created_at) "
            "VALUES(?,'ws','pf','tool','kind',?,'local','m',?,?,'t','t','executed','t')",
            (f"audit-{ordinal}", f"call-{ordinal}", f"{ordinal:064x}", decision))
    conn.commit()


def open_raw(root, encrypted):
    database = root / "kassiber.sqlite3"
    return sqlcipher.open_encrypted(database, PASSPHRASE, enforce_operator_identity=False) if encrypted else sqlite3.connect(database)


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
def test_current_python_book_with_main_sync_storage_opens_in_rust(storage_probe, tmp_path, encrypted):
    """A book the current Python created and wrote is admitted unchanged."""
    root = tmp_path / "python/data"
    conn = reference_open(root, encrypted)
    populate_main_sync_storage(conn)
    conn.close()
    conn = reference_open(root, encrypted)
    expected = normalized_dump(conn)
    conn.close()
    for op in ("open", "migrate", "open"):
        result = invoke(storage_probe, root, encrypted=encrypted, op=op)
        assert result["ok"], result
        conn = open_raw(root, encrypted)
        assert normalized_dump(conn) == expected, op
        conn.close()
    conn = reference_open(root, encrypted)
    assert normalized_dump(conn) == expected
    conn.close()


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
def test_rust_book_with_main_sync_storage_reopens_without_a_migration_loop(storage_probe, tmp_path, encrypted):
    """Rust-created, Python-written, then alternately reopened: nothing changes."""
    root = tmp_path / "rust/data"
    assert invoke(storage_probe, root, encrypted=encrypted)["ok"]
    conn = reference_open(root, encrypted)
    populate_main_sync_storage(conn)
    expected = normalized_dump(conn)
    conn.close()
    for side in ("rust", "python", "rust", "python"):
        if side == "rust":
            assert invoke(storage_probe, root, encrypted=encrypted, op="migrate")["ok"]
            assert invoke(storage_probe, root, encrypted=encrypted, op="open")["ok"]
            conn = open_raw(root, encrypted)
        else:
            conn = reference_open(root, encrypted)
        assert normalized_dump(conn) == expected, side
        conn.close()


NEWER_LAYOUTS = {
    "table": "CREATE TABLE future_storage(value INTEGER)",
    "column": "ALTER TABLE transactions ADD COLUMN future_column TEXT",
    "index": "CREATE INDEX idx_future_transactions ON transactions(asset)",
    "audit": "INSERT INTO schema_migration_audits VALUES('future-migration-v1','future-migration-v1',1,'{}','fixed')",
    "trigger": "CREATE TRIGGER trg_future BEFORE INSERT ON ai_provider_checks BEGIN SELECT 1; END",
}


@pytest.mark.parametrize("layout", NEWER_LAYOUTS)
@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
def test_newer_unknown_layouts_are_refused_by_rust_without_writing(storage_probe, tmp_path, encrypted, layout):
    """Rust refuses a layout it does not know, in both open modes, byte for byte.

    The Python reference does not refuse these: its open is additive and
    ignores objects and audits it does not know. That asymmetry is deliberate
    on the Rust side (fail closed rather than run on an unknown schema); see
    docs/reference/database-compatibility.md.
    """
    root = tmp_path / "book/data"
    conn = reference_open(root, encrypted)
    conn.execute(NEWER_LAYOUTS[layout])
    conn.commit()
    conn.close()
    database = root / "kassiber.sqlite3"
    before = database.read_bytes()
    for op in ("open", "open-migrating"):
        result = invoke(storage_probe, root, encrypted=encrypted, op=op)
        assert not result["ok"] and result["error"]["code"] == "requires_python_migration", (op, result)
        assert database.read_bytes() == before, op
    conn = reference_open(root, encrypted)
    assert conn.execute("SELECT count(*) FROM settings WHERE key='database_instance_id'").fetchone()[0] == 1
    conn.close()


@pytest.mark.skipif(os.name == "nt", reason="POSIX killed-child schema fixture")
@pytest.mark.parametrize("resume", ["python", "rust"])
@pytest.mark.parametrize("checkpoint", ["attachment.before_rename", "attachment.after_rename", "attachment.after_create", "attachment.after_copy", "attachment.after_drop", "repair.before_commit", "repair.after_commit"])
def test_killed_attachment_rebuild_preserves_authored_links(storage_probe, tmp_path, resume, checkpoint):
    source = tmp_path / "source/data"
    attachment_starting_point(source, "test_attachment_migration_removes_copied_provenance_foreign_keys")
    control, root = tmp_path / "control/data", tmp_path / "interrupted/data"
    shutil.copytree(source, control)
    shutil.copytree(source, root)
    expected = sqlite3.connect(control / "kassiber.sqlite3")
    expected.row_factory = sqlite3.Row
    expected.execute("PRAGMA foreign_keys=ON")
    db._migrate_attachment_table_shape(expected)
    expected.commit()
    child = subprocess.Popen([str(storage_probe)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        child.stdin.write(json.dumps(dict(op="schema-step", step="attachments", data_root=str(root), kill_at=checkpoint)) + "\n")
        child.stdin.flush()
        assert json.loads(child.stdout.readline()) == {"checkpoint": checkpoint}
    finally:
        child.kill()
        child.communicate(timeout=10)
    if resume == "rust":
        assert invoke(storage_probe, root, op="schema-step", step="attachments")["ok"]
    actual = sqlite3.connect(root / "kassiber.sqlite3")
    actual.row_factory = sqlite3.Row
    actual.execute("PRAGMA foreign_keys=ON")
    if resume == "python":
        db._migrate_attachment_table_shape(actual)
        actual.commit()
    assert normalized_dump(actual) == normalized_dump(expected)
    assert actual.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    assert actual.execute("PRAGMA foreign_key_check").fetchall() == []
    actual.close()
    expected.close()


@pytest.mark.parametrize("raw", [
    '{"x":NaN,"y":Infinity,"z":-Infinity,"overflow":1e400}',
    '{"a":1,"a":2,"b":-0.0,"c":-0,"d":1.234567890123456789e-280}',
    '{"surrogate":"\\ud800","pair":"\\ud83d\\ude00","low":"\\udc00","broken":"\\ud800\\u0041"}',
    '{"escaped":"\\b\\f\\n\\r\\t\\/\\\\\\\"","unicode":"€😀"}',
    '[[],{},true,false,null,0,1e-400,-1e400,123456789012345678901234567890]',
    ' {"large":' + '9' * 4300 + '} ',
])
def test_persisted_python_json_roundtrip_matches_reference(storage_probe, tmp_path, raw):
    expected = json.dumps(json.loads(raw), sort_keys=True)
    result = invoke(storage_probe, tmp_path, op="json-roundtrip", raw=raw)
    assert result["ok"], result
    assert result["data"]["encoded"] == expected


def review_schema_starting_point(source, half_migrated=False):
    from tests.test_custody_review_scope_migration import CustodyReviewScopeMigrationTests
    source.mkdir(parents=True)
    case = CustodyReviewScopeMigrationTests()
    case.setUp()
    try:
        case._insert_legacy_review_on(case.conn, "review", snapshot={"source_ids": ["out"]})
        case._install_v1_relation_table("review")
        if half_migrated:
            case.conn.execute("ALTER TABLE custody_gap_review_transactions RENAME TO custody_gap_review_transactions_v1")
            db._create_custody_gap_review_transaction_table(case.conn)
        case.conn.commit()
        with sqlite3.connect(source / "kassiber.sqlite3") as target:
            case.conn.backup(target)
    finally:
        case.tearDown()
        case.doCleanups()


@pytest.mark.parametrize("half_migrated", [False, True])
def test_legacy_custody_relation_table_matches_python(storage_probe, tmp_path, half_migrated):
    source = tmp_path / "source/data"
    review_schema_starting_point(source, half_migrated)
    compare_schema_step(storage_probe, tmp_path, source, "relation-schema")


@pytest.mark.parametrize("snapshot", [
    '{"source_ids":["out","missing","out",null,1,""],"return_ids":["return"]}',
    '{"source_ids":["out"],"extra":NaN,"overflow":1e400,"surrogate":"\\ud800"}',
    '{"source_ids":null,"nested":{"source_ids":["out"]}}',
    '{bad-json',
])
def test_custody_review_snapshot_backfill_matches_python(storage_probe, tmp_path, snapshot):
    from tests.test_custody_review_scope_migration import CustodyReviewScopeMigrationTests
    source = tmp_path / "source/data"
    source.mkdir(parents=True)
    case = CustodyReviewScopeMigrationTests()
    case.setUp()
    try:
        case.conn.execute("""INSERT INTO custody_gap_reviews(id,workspace_id,profile_id,gap_id,revision,candidate_fingerprint,action,event_kind,component_id,authored_source,reason,snapshot_json,created_at) VALUES('review','ws','profile','legacy-gap',1,?,'dismissed','review_decision',NULL,'user','legacy',?,'fixed')""", ("a"*64,snapshot))
        case.conn.commit()
        with sqlite3.connect(source / "kassiber.sqlite3") as target:
            case.conn.backup(target)
    finally:
        case.tearDown()
        case.doCleanups()
    compare_schema_step(storage_probe, tmp_path, source, "review-relations")


def evidence_starting_point(source):
    from tests.test_custody_migration_acceptance import _build_current_fixture
    conn = _build_current_fixture(source)
    # Model the pre-commitment headers while retaining the activation's own
    # evidence snapshots. Never construct commitments from current tx rows.
    conn.execute("DROP TRIGGER trg_custody_component_revision_immutable")
    conn.execute("DROP TRIGGER trg_custody_component_evidence_revision_delete_immutable")
    conn.execute("DELETE FROM custody_component_evidence_commitments")
    conn.execute("UPDATE custody_components SET expected_evidence_count=NULL")
    conn.commit()
    conn.close()


def test_authored_evidence_fixture_backfill_matches_python(storage_probe, tmp_path):
    source = tmp_path / "source/data"
    evidence_starting_point(source)
    compare_schema_step(storage_probe, tmp_path, source, "evidence")


@pytest.mark.parametrize("raw", [
    {"ownership_history":[{"descriptor":"synthetic-retired","chain":"elements","network":"regtest","scan_to_index":"+2_5","ownership_scan_to_index":4},{"descriptor":"synthetic-retired","chain":"elements","network":"regtest","ownership_scan_to_index":25}],"label":"ä"},
    {"ownership_history":[{"xpub":"synthetic-retired","chain":"unknown","network":"unknown","gap_limit":False,"addresses":[],"samourai":{}}]},
    {"ownership_history":[None,[],{},"ignored"],"chain":"bitcoin"},
    {"ownership_history":{},"chain":"bitcoin"},
])
def test_inline_ownership_history_matches_python(storage_probe,tmp_path,raw):
    source=tmp_path/"source/data"
    conn=reference_open(source)
    conn.executescript("INSERT INTO workspaces VALUES('ws','Synthetic','fixed'); INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('pf','ws','Book','fixed');")
    conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('wal','ws','pf','Wallet','descriptor',?,'fixed')",(json.dumps(raw,ensure_ascii=False),))
    conn.commit()
    conn.close()
    python,rust=tmp_path/"python/data",tmp_path/"rust/data"
    shutil.copytree(source,python)
    shutil.copytree(source,rust)
    expected=sqlite3.connect(python/"kassiber.sqlite3")
    expected.row_factory=sqlite3.Row
    with patch.object(db.uuid,"uuid4",return_value=uuid.UUID(hex=IDENTITY)):
        db._migrate_inline_ownership_history(expected)
    expected.commit()
    result=invoke(storage_probe,rust,op="ownership-history")
    assert result["ok"],result
    actual=sqlite3.connect(rust/"kassiber.sqlite3")
    before=normalized_dump(actual)
    assert before==normalized_dump(expected)
    actual.close()
    assert invoke(storage_probe,rust,op="ownership-history")["ok"]
    actual=reference_open(rust)
    assert normalized_dump(actual)==before
    actual.close()
    expected.close()


@pytest.mark.parametrize("amount,fee", [("1.2300","0.00000000001"),("-1.2300","-0.00000000001"),("1.23000000000000000000000000000000001",None),("1e-11",0),("invalid",[]),(True,False)])
@pytest.mark.parametrize("collision",[False,True])
def test_liquid_fingerprint_and_identity_match_python(storage_probe,tmp_path,amount,fee,collision):
    from kassiber.wallet_descriptors import LIQUID_POLICY_ASSET_IDS
    source=tmp_path/"source/data"
    conn=reference_open(source)
    conn.executescript("INSERT INTO workspaces VALUES('ws','Synthetic','fixed'); INSERT INTO profiles(id,workspace_id,label,created_at,last_processed_at) VALUES('pf','ws','Book','fixed','fixed'); INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('wal','ws','pf','Wallet','descriptor','{\"chain\":\"liquid\",\"network\":\"regtest\"}','fixed');")
    raw=json.dumps(dict(amount=amount,fee=fee,extra=1.0))
    conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,fee,external_id,raw_json,created_at) VALUES('tx','ws','pf','wal','old-fp','fixed','inbound',?,123000000000,1,?,?,'fixed')",(LIQUID_POLICY_ASSET_IDS['main'],'ab'*32,raw))
    if collision:
        row=conn.execute("SELECT * FROM transactions WHERE id='tx'").fetchone()
        fingerprint=db._backfilled_transaction_fingerprint(row,"LBTC")
        conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,created_at) VALUES('collision','ws','pf','wal',?,'fixed','inbound','BTC',1,'fixed')",(fingerprint,))
    conn.commit()
    conn.close()
    compare_schema_step(storage_probe,tmp_path,source,"liquid")


def test_durable_anchor_group_and_exact_audit_match_python(storage_probe,tmp_path):
    source=tmp_path/"source/data"
    evidence_starting_point(source)
    compare_schema_step(storage_probe,tmp_path,source,"durable-evidence")


@pytest.mark.parametrize("checkpoint",["durable-evidence.after_anchors","durable-evidence.after_headers","evidence.after_insert","evidence.after_component","durable-evidence.after_audit","durable-evidence.before_commit","durable-evidence.after_commit"])
def test_durable_evidence_failure_is_atomic(storage_probe,tmp_path,checkpoint):
    source=tmp_path/"source/data"
    evidence_starting_point(source)
    with sqlite3.connect(source/"kassiber.sqlite3") as conn:
        before=normalized_dump(conn)
    result=invoke(storage_probe,source,op="schema-step",step="durable-evidence",fail_at=checkpoint)
    assert not result["ok"],result
    if checkpoint!="durable-evidence.after_commit":
        with sqlite3.connect(source/"kassiber.sqlite3") as conn:
            assert normalized_dump(conn)==before
    compare_schema_step(storage_probe,tmp_path,source,"durable-evidence")


@pytest.mark.parametrize("value",[None,"", "ABCDEF0123456789abcdef0123456789", "0"*31, "1"*32, b"1"*32, 7])
def test_missing_and_invalid_database_identities_match_python(storage_probe,tmp_path,value):
    root=tmp_path/"book/data"
    conn=reference_open(root)
    if value is None:
        conn.execute("DELETE FROM settings WHERE key='database_instance_id'")
    else:
        conn.execute("UPDATE settings SET value=? WHERE key='database_instance_id'",(value,))
    conn.commit()
    with patch.object(db.uuid,"uuid4",return_value=uuid.UUID(hex=IDENTITY)):
        expected=db.ensure_database_instance_id(conn)
    conn.close()
    # Restore the same starting value before the Rust operation.
    with sqlite3.connect(root/"kassiber.sqlite3") as conn:
        if value is None:
            conn.execute("DELETE FROM settings WHERE key='database_instance_id'")
        else:
            conn.execute("UPDATE settings SET value=? WHERE key='database_instance_id'",(value,))
    result=invoke(storage_probe,root,op="database-identity")
    assert result["ok"],result
    conn=reference_open(root)
    assert db.database_instance_id(conn)==expected
    conn.close()


@pytest.mark.parametrize("fixture",sorted((ROOT/"tests/fixtures/historical").glob("*.sqlite3.gz.b64")),ids=lambda path:path.name)
def test_historical_durable_group_from_pre_anchor_schema(storage_probe,tmp_path,fixture):
    import base64
    import gzip
    root=tmp_path/"original/data"
    root.mkdir(parents=True)
    (root/"kassiber.sqlite3").write_bytes(gzip.decompress(base64.b64decode(fixture.read_bytes())))
    source=tmp_path/"source/data"
    source.mkdir(parents=True)
    class CapturedFixture(Exception):
        pass
    original=db._ensure_legacy_custody_write_freeze_triggers
    def capture(conn):
        original(conn)
        conn.commit()
        with sqlite3.connect(source/"kassiber.sqlite3") as target:
            conn.backup(target)
        raise CapturedFixture
    with patch.object(db,"_ensure_legacy_custody_write_freeze_triggers",capture):
        with pytest.raises(CapturedFixture):
            reference_open(root)
    compare_schema_step(storage_probe,tmp_path,source,"durable-evidence")


@pytest.mark.parametrize("step,checkpoint",[
    ("liquid","liquid.after_asset"),("liquid","liquid.after_identity"),
    ("liquid","liquid.before_commit"),("liquid","liquid.after_commit"),
    ("ownership-history","ownership-history.after_epoch"),
    ("ownership-history","ownership-history.after_config"),
    ("ownership-history","ownership-history.before_commit"),
    ("ownership-history","ownership-history.after_commit"),
])
def test_ownership_and_liquid_failure_boundaries(storage_probe,tmp_path,step,checkpoint):
    from kassiber.wallet_descriptors import LIQUID_POLICY_ASSET_IDS
    root=tmp_path/"source/data"
    conn=reference_open(root)
    conn.executescript("INSERT INTO workspaces VALUES('ws','Synthetic','fixed'); INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('pf','ws','Book','fixed');")
    config=json.dumps(dict(chain="liquid",network="regtest",ownership_history=[dict(descriptor="synthetic-retired")]))
    conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('wal','ws','pf','Wallet','descriptor',?,'fixed')",(config,))
    conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,fee,external_id,created_at) VALUES('tx','ws','pf','wal','fp','fixed','inbound',?,1,0,?,'fixed')",(LIQUID_POLICY_ASSET_IDS['main'],'ab'*32))
    conn.commit()
    before=normalized_dump(conn)
    conn.close()
    result=invoke(storage_probe,root,op="schema-step",step=step,fail_at=checkpoint)
    assert not result["ok"],result
    if not checkpoint.endswith("after_commit"):
        with sqlite3.connect(root/"kassiber.sqlite3") as conn:
            assert normalized_dump(conn)==before


@pytest.mark.skipif(os.name=="nt",reason="POSIX killed-child migration harness")
@pytest.mark.parametrize("resume",["python","rust"])
@pytest.mark.parametrize("checkpoint",[
    "durable-evidence.after_occurred_at","durable-evidence.after_anchors",
    "durable-evidence.after_headers","evidence.after_insert","evidence.after_component",
    "durable-evidence.after_audit","durable-evidence.before_commit","durable-evidence.after_commit",
])
def test_killed_durable_evidence_group_preserves_exact_audit(storage_probe,tmp_path,resume,checkpoint):
    import select
    root=tmp_path/"source/data"
    evidence_starting_point(root)
    control=tmp_path/"control/data"
    shutil.copytree(root,control)
    expected=sqlite3.connect(control/"kassiber.sqlite3")
    expected.row_factory=sqlite3.Row
    expected.create_function("strftime",2,lambda *_:TIMESTAMP)
    reference_schema_step(expected,"durable-evidence")
    expected.commit()
    request=dict(op="schema-step",step="durable-evidence",data_root=str(root),timestamp=TIMESTAMP,kill_at=checkpoint)
    child=subprocess.Popen([str(storage_probe)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    try:
        child.stdin.write(json.dumps(request)+"\n")
        child.stdin.flush()
        assert select.select([child.stdout],[],[],15)[0],"migration checkpoint timed out"
        assert json.loads(child.stdout.readline())==dict(checkpoint=checkpoint)
    finally:
        child.kill()
        child.communicate(timeout=10)
    if resume=="rust":
        result=invoke(storage_probe,root,op="schema-step",step="durable-evidence")
        assert result["ok"],result
    actual=sqlite3.connect(root/"kassiber.sqlite3")
    actual.row_factory=sqlite3.Row
    actual.create_function("strftime",2,lambda *_:TIMESTAMP)
    if resume=="python":
        reference_schema_step(actual,"durable-evidence")
        actual.commit()
    assert normalized_dump(actual)==normalized_dump(expected)
    assert actual.execute("PRAGMA integrity_check").fetchone()[0]=="ok"
    assert actual.execute("PRAGMA foreign_key_check").fetchall()==[]
    actual.close()
    expected.close()


@pytest.mark.skipif(os.name == "nt", reason="POSIX killed-child custody fixture")
@pytest.mark.parametrize("resume", ["python", "rust"])
@pytest.mark.parametrize("step,checkpoint", [
    ("relation-schema", "review-schema.after_drop_objects"),
    ("relation-schema", "review-schema.after_rename"),
    ("relation-schema", "review-schema.after_copy"),
    ("relation-schema", "review-schema.before_commit"),
    ("relation-schema", "review-schema.after_commit"),
    ("evidence", "evidence.after_insert"),
    ("evidence", "evidence.after_component"),
    ("evidence", "evidence.before_commit"),
    ("evidence", "evidence.after_commit"),
    ("leg-roles", "leg-roles.after_drop_objects"),
    ("leg-roles", "leg-roles.after_rename"),
    ("leg-roles", "leg-roles.after_create"),
    ("leg-roles", "leg-roles.after_copy"),
    ("leg-roles", "leg-roles.after_restore_objects"),
    ("leg-roles", "repair.before_commit"),
    ("leg-roles", "repair.after_commit"),
])
def test_killed_custody_backfill_resumes_without_losing_authored_history(storage_probe, tmp_path, resume, step, checkpoint):
    source = tmp_path / "source/data"
    if step == "relation-schema":
        review_schema_starting_point(source)
    elif step == "leg-roles":
        historical_pre_role_starting_point(source, ROOT / "tests/fixtures/historical/pre_432_5d232097.sqlite3.gz.b64")
    else:
        evidence_starting_point(source)
    control, root = tmp_path / "control/data", tmp_path / "interrupted/data"
    shutil.copytree(source, control)
    shutil.copytree(source, root)
    expected = sqlite3.connect(control / "kassiber.sqlite3")
    expected.row_factory = sqlite3.Row
    expected.execute("PRAGMA foreign_keys=ON")
    reference_schema_step(expected, step)
    expected.commit()
    child = subprocess.Popen([str(storage_probe)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        child.stdin.write(json.dumps(dict(op="schema-step", step=step, data_root=str(root), kill_at=checkpoint)) + "\n")
        child.stdin.flush()
        assert json.loads(child.stdout.readline()) == {"checkpoint": checkpoint}
    finally:
        child.kill()
        child.communicate(timeout=10)
    if resume == "rust":
        assert invoke(storage_probe, root, op="schema-step", step=step)["ok"]
    actual = sqlite3.connect(root / "kassiber.sqlite3")
    actual.row_factory = sqlite3.Row
    actual.execute("PRAGMA foreign_keys=ON")
    if resume == "python":
        reference_schema_step(actual, step)
        actual.commit()
    assert normalized_dump(actual) == normalized_dump(expected)
    assert actual.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    assert actual.execute("PRAGMA foreign_key_check").fetchall() == []
    actual.close()
    expected.close()


def historical_pre_role_starting_point(destination, fixture):
    import base64
    import gzip
    destination.mkdir(parents=True)
    with tempfile.TemporaryDirectory(prefix="kassiber-r3-pre-role-") as directory:
        root = Path(directory) / "data"
        root.mkdir()
        (root / "kassiber.sqlite3").write_bytes(gzip.decompress(base64.b64decode(fixture.read_bytes())))
        class CapturedFixture(Exception):
            pass
        def capture(conn):
            conn.commit()
            with sqlite3.connect(destination / "kassiber.sqlite3") as target:
                conn.backup(target)
            raise CapturedFixture
        with patch.object(db, "_ensure_custody_leg_role_schema", capture):
            with pytest.raises(CapturedFixture):
                reference_open(root)


@pytest.mark.parametrize("fixture", sorted((ROOT / "tests/fixtures/historical").glob("*.sqlite3.gz.b64")), ids=lambda path:path.name)
def test_historical_custody_leg_rebuild_matches_python(storage_probe, tmp_path, fixture):
    source = tmp_path / "source/data"
    historical_pre_role_starting_point(source, fixture)
    compare_schema_step(storage_probe, tmp_path, source, "leg-roles")


def minimal_backfill_scope(conn):
    conn.executescript("""
        INSERT INTO workspaces VALUES('ws','Synthetic','fixed');
        INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('pf','ws','Synthetic','fixed');
        INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('wal','ws','pf','Wallet','custom','fixed'),('second','ws','pf','Second','custom','fixed');
    """)


def test_payment_hash_backfill_uses_exact_python_write_conditions(storage_probe, tmp_path):
    source = tmp_path / "source/data"
    conn = reference_open(source)
    minimal_backfill_scope(conn)
    payloads = [
        {"payment_hash":"ab"*32}, {"payment_hash":"\x1c"+"CD"*32+"\x1f"},
        {"payment_hash":""}, {"nested":{"payment_hash":"ab"*32}},
        {"payment_hash":"ab"*30+"    "}, {"payment_hash":"a b"+"f"*61},
        {"payment_hash":None}, {"payment_hash":"g"*64},
        {"payment_hash":"ef"*32,"extra":float("nan"),"surrogate":"\ud800"},
    ]
    for index,payload in enumerate(payloads):
        conn.execute("""INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,raw_json,created_at) VALUES(?,'ws','pf','wal',?,'fixed','inbound','BTC',1,?,'fixed')""", (str(index),str(index),json.dumps(payload)))
    conn.commit()
    conn.close()
    compare_schema_step(storage_probe, tmp_path, source, "payment-hash")


def test_bip329_consolidation_preserves_latest_authored_fields_and_json(storage_probe, tmp_path):
    source = tmp_path / "source/data"
    conn = reference_open(source)
    minimal_backfill_scope(conn)
    conn.execute("DROP INDEX idx_bip329_labels_profile_object")
    for identifier,wallet,label,origin,spendable,payload,created in [
        ("first","wal","Old","original",1,'{"retained":true,"replaced":1}',"a"),
        ("last","second","Latest",None,0,'{"replaced":2,"extra":NaN,"surrogate":"\\ud800"}',"b"),
    ]:
        conn.execute("""INSERT INTO bip329_labels(id,workspace_id,profile_id,wallet_id,record_type,ref,label,origin,spendable,data_json,created_at) VALUES(?,'ws','pf',?,'tx','synthetic',?,?,?,?,?)""", (identifier,wallet,label,origin,spendable,payload,created))
    conn.commit()
    conn.close()
    compare_schema_step(storage_probe, tmp_path, source, "bip329")


@pytest.mark.parametrize("step",["swap-fees","swap-schema"])
@pytest.mark.parametrize("kind,asset_in", [("manual","l-btc"),("peg-in","LBTC"),("\x1cMANUAL\x1f","LBTC"),("manual","BTC")])
def test_stale_pair_fees_match_python_normalization(storage_probe,tmp_path,kind,asset_in,step):
    source=tmp_path/"source/data"
    conn=reference_open(source)
    minimal_backfill_scope(conn)
    for identity,direction,asset in [("out","outbound","LBTC"),("in","inbound",asset_in)]:
        conn.execute("""INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,created_at) VALUES(?,'ws','pf','wal',?,'fixed',?,?,1,'fixed')""",(identity,identity,direction,asset))
    conn.execute("""INSERT INTO transaction_pairs(id,workspace_id,profile_id,out_transaction_id,in_transaction_id,kind,swap_fee_msat,swap_fee_kind,created_at) VALUES('pair','ws','pf','out','in',?,7,'synthetic','fixed')""",(kind,))
    conn.commit()
    conn.close()
    compare_schema_step(storage_probe,tmp_path,source,step)


def test_swap_orchestration_repairs_legacy_unique_table_and_payment_hash(storage_probe,tmp_path):
    from tests.test_schema_swap_matching import LegacyUniqueMigrationTests
    source=tmp_path/"source/data"
    conn=reference_open(source)
    minimal_backfill_scope(conn)
    LegacyUniqueMigrationTests()._downgrade_transaction_pairs_to_legacy(conn)
    for identity,direction in [("out","outbound"),("in","inbound")]:
        conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,raw_json,created_at) VALUES(?,'ws','pf','wal',?,'fixed',?,'BTC',1,?,'fixed')",(identity,identity,direction,json.dumps(dict(payment_hash="ab"*32))))
    conn.execute("INSERT INTO transaction_pairs(id,workspace_id,profile_id,out_transaction_id,in_transaction_id,kind,policy,notes,created_at) VALUES('pair','ws','pf','out','in','manual','carrying-value','reviewed','fixed')")
    conn.commit()
    conn.close()
    compare_schema_step(storage_probe,tmp_path,source,"swap-schema")


@pytest.mark.parametrize("version", [[1,2,7], [], "42", [" +１２_３ "], ["1.0"], "invalid", [True]])
def test_gap_input_cache_retirement_matches_python(storage_probe,tmp_path,version):
    source=tmp_path/"source/data"
    conn=reference_open(source)
    minimal_backfill_scope(conn)
    conn.executescript("""CREATE TABLE custody_gap_candidate_projections(profile_id TEXT,producer_kind TEXT,version_json TEXT,ignored_ids_json TEXT,accounting_ignored_ids_json TEXT,created_at TEXT);CREATE TABLE custody_gap_candidate_snapshots(id TEXT);CREATE TABLE custody_gap_projection_rows(id TEXT);CREATE TABLE custody_gap_candidate_boundaries(id TEXT);CREATE TABLE custody_gap_candidates(id TEXT);""")
    for producer,raw,ignored in [("journal","[1]",'["old"]'),("journal",json.dumps(version),'["retained"]'),("display","[99]",'["ignored"]')]:
        conn.execute("INSERT INTO custody_gap_candidate_projections VALUES('pf',?,?,?,?, 'fixed')",(producer,raw,ignored,'["accounting"]'))
    conn.commit()
    conn.close()
    compare_schema_step(storage_probe,tmp_path,source,"gap-inputs")


def test_inbox_rebuild_preserves_public_rowid_cursors(storage_probe,tmp_path):
    source=tmp_path/"source/data"
    conn=reference_open(source)
    minimal_backfill_scope(conn)
    conn.execute("DROP TRIGGER trg_report_chain_impacts_delete_inbox")
    ddl=conn.execute("SELECT sql FROM sqlite_master WHERE name='chain_analysis_watch_inbox'").fetchone()[0]
    conn.execute("DROP TABLE chain_analysis_watch_inbox")
    conn.execute(ddl.replace("watch_id TEXT REFERENCES","watch_id TEXT NOT NULL REFERENCES"))
    conn.execute("""INSERT INTO chain_analysis_watches(id,profile_id,book_id,domain_json,definition_json,baseline_json,checkpoint,checked_at,created_at) VALUES('watch','pf','book','{}','{}','{}','fixed','fixed','fixed')""")
    for rowid,sequence in [(7,1),(123,2)]:
        conn.execute("""INSERT INTO chain_analysis_watch_inbox(rowid,id,watch_id,profile_id,sequence,code,observation_json,created_at) VALUES(?,?,'watch','pf',?,'synthetic','{}','fixed')""",(rowid,str(rowid),sequence))
    conn.executescript(db.FILED_REPORT_CHAIN_SCHEMA)
    conn.commit()
    conn.close()
    compare_schema_step(storage_probe,tmp_path,source,"inbox")
    conn=sqlite3.connect(tmp_path/"rust/data/kassiber.sqlite3")
    assert conn.execute("SELECT rowid FROM chain_analysis_watch_inbox ORDER BY rowid").fetchall()==[(7,),(123,)]
    conn.close()


def test_normalized_dump_includes_internal_objects_and_storage_classes():
    integer, real = sqlite3.connect(":memory:"), sqlite3.connect(":memory:")
    try:
        for conn in (integer, real):
            conn.executescript("CREATE TABLE values_without_affinity(value);CREATE TABLE sequence_owner(id INTEGER PRIMARY KEY AUTOINCREMENT);INSERT INTO sequence_owner DEFAULT VALUES;")
        integer.execute("INSERT INTO values_without_affinity VALUES(1)")
        real.execute("INSERT INTO values_without_affinity VALUES(1.0)")
        assert normalized_dump(integer) != normalized_dump(real)
        assert "sqlite_sequence" in normalized_dump(integer)[1]
    finally:
        integer.close()
        real.close()


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
@pytest.mark.parametrize("first", ["python", "rust"])
def test_transaction_page_index_upgrade(storage_probe, tmp_path, encrypted, first):
    root = tmp_path / "page/data"
    conn = reference_open(root, encrypted)
    conn.executescript("""
        INSERT INTO workspaces VALUES('page-ws','Page','fixed');
        INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('page-pf','page-ws','Page','fixed');
        INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at)
            VALUES('page-wallet','page-ws','page-pf','Page','manual','fixed');
        INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,created_at)
            VALUES('page-tx','page-ws','page-pf','page-wallet','page-tx','2025-01-01','inbound','BTC',9007199254740997,'fixed');
    """)
    conn.close()
    conn = reference_open(root, encrypted)
    conn.execute("DROP INDEX idx_transactions_profile_page")
    conn.commit()
    before = normalized_dump(conn)[1:]
    conn.close()
    if first == "rust":
        result = invoke(storage_probe, root, encrypted=encrypted, op="migrate")
        assert result["ok"], result
    conn = reference_open(root, encrypted)
    assert normalized_dump(conn)[1:] == before
    expected = normalized_dump(conn)
    conn.close()
    assert invoke(storage_probe, root, encrypted=encrypted, op="migrate")["ok"]
    conn = reference_open(root, encrypted)
    assert normalized_dump(conn) == expected
    conn.close()


@pytest.mark.parametrize("encrypted", [False, True])
@pytest.mark.parametrize("checkpoint", ["transaction_page_index.before", "transaction_page_index.after"])
def test_transaction_page_index_rollback(storage_probe, tmp_path, encrypted, checkpoint):
    root = tmp_path / "page/data"
    conn = reference_open(root, encrypted)
    conn.close()
    conn = reference_open(root, encrypted)
    conn.execute("DROP INDEX idx_transactions_profile_page")
    conn.commit()
    before = normalized_dump(conn)
    conn.close()
    result = invoke(storage_probe, root, encrypted=encrypted, op="migrate", fail_at=checkpoint)
    assert not result["ok"]
    conn = sqlcipher.open_encrypted(root / "kassiber.sqlite3", PASSPHRASE, enforce_operator_identity=False) if encrypted else sqlite3.connect(root / "kassiber.sqlite3")
    assert normalized_dump(conn) == before
    conn.close()
    assert invoke(storage_probe, root, encrypted=encrypted, op="migrate")["ok"]


@pytest.mark.parametrize("resume", ["python", "rust"])
@pytest.mark.parametrize("encrypted", [False, True])
def test_transaction_page_index_killed_creation(storage_probe, tmp_path, resume, encrypted):
    root = tmp_path / "page/data"
    conn = reference_open(root, encrypted)
    conn.close()
    conn = reference_open(root, encrypted)
    conn.execute("DROP INDEX idx_transactions_profile_page")
    conn.commit()
    before = normalized_dump(conn)
    conn.close()
    # invoke's secret descriptor cannot outlive its call; use the same explicit
    # temporary descriptor protocol while the child waits at the checkpoint.
    with tempfile.TemporaryFile() as secret:
        secret.write(PASSPHRASE.encode())
        secret.seek(0)
        request = dict(op="migrate", data_root=str(root), owner_root=str(root.parent / "owners"), identity=IDENTITY,
                       timestamp=TIMESTAMP, kill_at="transaction_page_index.after")
        if encrypted:
            request["fd"] = secret.fileno()
        process = subprocess.Popen([str(storage_probe)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, text=True, pass_fds=(secret.fileno(),) if encrypted else ())
        try:
            process.stdin.write(json.dumps(request) + "\n")
            process.stdin.flush()
            assert json.loads(process.stdout.readline()) == {"checkpoint": "transaction_page_index.after"}
        finally:
            process.kill()
            process.communicate(timeout=10)
    conn = sqlcipher.open_encrypted(root / "kassiber.sqlite3", PASSPHRASE, enforce_operator_identity=False) if encrypted else sqlite3.connect(root / "kassiber.sqlite3")
    assert normalized_dump(conn) == before
    conn.close()
    if resume == "rust":
        assert invoke(storage_probe, root, encrypted=encrypted, op="migrate")["ok"]
    conn = reference_open(root, encrypted)
    assert conn.execute("SELECT count(*) FROM sqlite_master WHERE name='idx_transactions_profile_page'").fetchone()[0] == 1
    conn.close()


@pytest.mark.parametrize("failure", ["interrupt", "busy", "full"])
def test_python_transaction_page_index_storage_errors(tmp_path, failure):
    path = tmp_path / "errors.sqlite3"
    conn = sqlite3.connect(path, timeout=0)
    conn.executescript("CREATE TABLE transactions(profile_id,occurred_at,created_at,id,wallet_id);"
                       "INSERT INTO transactions SELECT value,value,value,value,value FROM json_each('[1,2,3,4,5]');")
    cookie = conn.execute("PRAGMA schema_version").fetchone()[0]
    other = sqlite3.connect(path, timeout=0)
    if failure == "busy":
        other.execute("BEGIN IMMEDIATE")
    elif failure == "full":
        conn.execute(f"PRAGMA max_page_count={conn.execute('PRAGMA page_count').fetchone()[0]}")
    else:
        def interrupt_creation(sql):
            if sql.startswith("CREATE INDEX"):
                def stop():
                    conn.set_progress_handler(None, 0)
                    return 1
                conn.set_progress_handler(stop, 1)
        conn.set_trace_callback(interrupt_creation)
    try:
        with pytest.raises(sqlite3.OperationalError, match={"busy": "locked", "full": "full", "interrupt": "interrupted"}[failure]):
            db._ensure_transaction_page_index(conn)
    finally:
        conn.set_progress_handler(None, 0)
        conn.set_trace_callback(None)
        other.rollback()
        other.close()
    assert conn.execute("PRAGMA schema_version").fetchone()[0] == cookie
    assert conn.execute("SELECT count(*) FROM sqlite_master WHERE type='index'").fetchone()[0] == 0
    assert not conn.in_transaction
    conn.close()


def test_python_transaction_page_index_definition_and_cookie(tmp_path):
    conn = sqlite3.connect(tmp_path / "definition.sqlite3")
    conn.execute("CREATE TABLE transactions(profile_id,occurred_at,created_at,id,wallet_id)")
    cookie = conn.execute("PRAGMA schema_version").fetchone()[0]
    db._ensure_transaction_page_index(conn)
    rootpage = conn.execute("SELECT rootpage FROM sqlite_master WHERE name='idx_transactions_profile_page'").fetchone()[0]
    db._ensure_transaction_page_index(conn)
    assert conn.execute("PRAGMA schema_version").fetchone()[0] == cookie + 1
    assert conn.execute("SELECT rootpage FROM sqlite_master WHERE name='idx_transactions_profile_page'").fetchone()[0] == rootpage
    conn.execute("DROP INDEX idx_transactions_profile_page")
    conn.execute("CREATE INDEX idx_transactions_profile_page ON transactions(id)")
    with pytest.raises(sqlite3.DatabaseError, match="unexpected transaction page index definition"):
        db._ensure_transaction_page_index(conn)
    assert conn.execute("PRAGMA index_info(idx_transactions_profile_page)").fetchone()[2] == "id"
    conn.close()


@pytest.mark.parametrize("fixture", HISTORICAL_FIXTURES, ids=lambda path: path.name)
@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
def test_transaction_page_index_original_historical_python_first(storage_probe, tmp_path, fixture, encrypted):
    root = tmp_path / "original/data"
    decode_historical(root, fixture, encrypted)
    conn = reference_open(root, encrypted)
    expected = normalized_dump(conn)
    assert conn.execute("PRAGMA index_info(idx_transactions_profile_page)").fetchall()
    conn.close()
    assert invoke(storage_probe, root, encrypted=encrypted, op="migrate")["ok"]
    conn = reference_open(root, encrypted)
    assert normalized_dump(conn) == expected
    conn.close()
