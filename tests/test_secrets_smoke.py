"""Behaviour pin for the V4.1 SQLCipher / backup work.

Covers the round-trips the plan calls out as table-stakes:

- `escape_passphrase` accepts every weird value the doc names and rejects
  empty / NUL.
- `open_db` keeps the plaintext code path unchanged when no passphrase
  is provided.
- `open_db` round-trips a SQLCipher database when a passphrase is
  provided, while a wrong passphrase surfaces the structured
  `unlock_failed` envelope.
- `migrate_plaintext_to_encrypted` preserves user_version, succeeds on a
  weird passphrase, and produces a file that still opens fresh.
- `change_database_passphrase` rotates and re-verifies.
- `extract_tar_safely` rejects path-traversal, symlinks, hardlinks,
  device nodes, and duplicate paths.
- The full `backup export` -> `backup import` round-trip preserves the
  database content.
"""

from __future__ import annotations

import os
import sqlite3
import tarfile
import tempfile
import unittest
from pathlib import Path

from kassiber.backup.pack import export_backup, import_backup
from kassiber.backup.safe_tar import (
    UnsafeTarMember,
    inspect_tar_members,
)
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.secrets.migration import migrate_plaintext_to_encrypted
from kassiber.secrets.passphrase import change_database_passphrase
from kassiber.secrets.sqlcipher import (
    escape_passphrase,
    looks_like_plaintext_sqlite,
    open_encrypted,
)


WEIRD_PASSPHRASES = [
    "simple",
    "with spaces",
    "single 'quote' inside",
    'double "quote" inside',
    "semicolon;test",
    "backslash\\test",
    "trailing space ",
    "  leading space",
    "₿ emoji 🚀 test",
    "very-long-" + "x" * 256,
]


class EscapePassphraseTests(unittest.TestCase):
    def test_round_trips_weird_inputs(self):
        for value in WEIRD_PASSPHRASES:
            quoted = escape_passphrase(value)
            self.assertTrue(quoted.startswith("'"))
            self.assertTrue(quoted.endswith("'"))
            # Stripping the outer quotes and undoubling inner quotes
            # should reproduce the input exactly.
            inner = quoted[1:-1].replace("''", "'")
            self.assertEqual(inner, value, f"failed for: {value!r}")

    def test_rejects_empty_and_nul(self):
        with self.assertRaises(AppError):
            escape_passphrase("")
        with self.assertRaises(AppError):
            escape_passphrase("contains\x00nul")
        with self.assertRaises(AppError):
            escape_passphrase(None)  # type: ignore[arg-type]


class OpenDbPlaintextTests(unittest.TestCase):
    def test_plaintext_path_unchanged(self):
        with tempfile.TemporaryDirectory() as root:
            conn = open_db(root)
            try:
                conn.execute("CREATE TABLE marker(x INTEGER)")
                conn.execute("INSERT INTO marker VALUES(7)")
                conn.commit()
                self.assertEqual(
                    conn.execute("SELECT x FROM marker").fetchone()[0], 7
                )
            finally:
                conn.close()
            db_path = Path(root) / "kassiber.sqlite3"
            self.assertTrue(looks_like_plaintext_sqlite(db_path))


class OpenDbEncryptedTests(unittest.TestCase):
    def test_encrypted_round_trip(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            data_root.mkdir()
            # Seed plaintext, then migrate, then re-open with passphrase.
            seed = open_db(str(data_root))
            seed.execute("CREATE TABLE marker(x INTEGER)")
            seed.execute("INSERT INTO marker VALUES(42)")
            seed.commit()
            seed.close()

            migrate_plaintext_to_encrypted(
                Path(data_root) / "kassiber.sqlite3", "tracer-pass-12345"
            )
            conn = open_db(str(data_root), passphrase="tracer-pass-12345")
            try:
                self.assertEqual(
                    conn.execute("SELECT x FROM marker").fetchone()[0], 42
                )
            finally:
                conn.close()

    def test_wrong_passphrase_raises_unlock_failed(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            data_root.mkdir()
            seed = open_db(str(data_root))
            seed.execute("CREATE TABLE marker(x INTEGER)")
            seed.execute("INSERT INTO marker VALUES(1)")
            seed.commit()
            seed.close()
            migrate_plaintext_to_encrypted(
                Path(data_root) / "kassiber.sqlite3", "real-passphrase-12345"
            )
            with self.assertRaises(AppError) as ctx:
                open_db(str(data_root), passphrase="wrong-passphrase").close()
            self.assertEqual(ctx.exception.code, "unlock_failed")

    def test_passphrase_required_when_db_is_encrypted(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            data_root.mkdir()
            seed = open_db(str(data_root))
            seed.execute("CREATE TABLE marker(x INTEGER)")
            seed.commit()
            seed.close()
            migrate_plaintext_to_encrypted(
                Path(data_root) / "kassiber.sqlite3", "passphrase-here"
            )
            with self.assertRaises(AppError) as ctx:
                open_db(str(data_root)).close()
            self.assertEqual(ctx.exception.code, "passphrase_required")


class MigrationTests(unittest.TestCase):
    def test_round_trip_preserves_user_version(self):
        with tempfile.TemporaryDirectory() as root:
            db_path = Path(root) / "kassiber.sqlite3"
            seed = sqlite3.connect(str(db_path))
            seed.execute("PRAGMA user_version = 17")
            seed.execute("CREATE TABLE x(a)")
            seed.execute("INSERT INTO x VALUES('value')")
            seed.commit()
            seed.close()

            result = migrate_plaintext_to_encrypted(db_path, "weird 'pass\";rule")
            self.assertEqual(result.plaintext_user_version, 17)
            self.assertEqual(result.integrity_check, "ok")
            self.assertTrue(result.credential_marker_clean)
            self.assertTrue(result.backup_path.exists())

            conn = open_encrypted(db_path, "weird 'pass\";rule")
            try:
                self.assertEqual(conn.execute("PRAGMA user_version").fetchone()[0], 17)
                self.assertEqual(conn.execute("SELECT a FROM x").fetchone()[0], "value")
            finally:
                conn.close()


class ChangePassphraseTests(unittest.TestCase):
    def test_rotation(self):
        with tempfile.TemporaryDirectory() as root:
            db_path = Path(root) / "kassiber.sqlite3"
            seed = sqlite3.connect(str(db_path))
            seed.execute("CREATE TABLE x(a)")
            seed.execute("INSERT INTO x VALUES('abc')")
            seed.commit()
            seed.close()

            migrate_plaintext_to_encrypted(db_path, "first-pass")
            change_database_passphrase(db_path, "first-pass", "second-pass")

            with self.assertRaises(AppError):
                open_encrypted(db_path, "first-pass").close()
            conn = open_encrypted(db_path, "second-pass")
            try:
                self.assertEqual(conn.execute("SELECT a FROM x").fetchone()[0], "abc")
            finally:
                conn.close()


def _make_tarinfo(name: str, *, type_: bytes = tarfile.REGTYPE, size: int = 0):
    info = tarfile.TarInfo(name=name)
    info.type = type_
    info.size = size
    return info


class SafeTarTests(unittest.TestCase):
    def test_rejects_absolute_path(self):
        members = [_make_tarinfo("/etc/passwd")]
        with self.assertRaises(UnsafeTarMember):
            inspect_tar_members(members)

    def test_rejects_traversal(self):
        members = [_make_tarinfo("../../../escape")]
        with self.assertRaises(UnsafeTarMember):
            inspect_tar_members(members)

    def test_rejects_symlink(self):
        members = [_make_tarinfo("attachments/link", type_=tarfile.SYMTYPE)]
        with self.assertRaises(UnsafeTarMember):
            inspect_tar_members(members)

    def test_rejects_hardlink(self):
        members = [_make_tarinfo("attachments/hl", type_=tarfile.LNKTYPE)]
        with self.assertRaises(UnsafeTarMember):
            inspect_tar_members(members)

    def test_rejects_device_node(self):
        members = [_make_tarinfo("attachments/dev", type_=tarfile.CHRTYPE)]
        with self.assertRaises(UnsafeTarMember):
            inspect_tar_members(members)

    def test_rejects_duplicate(self):
        members = [_make_tarinfo("manifest.json"), _make_tarinfo("manifest.json")]
        with self.assertRaises(UnsafeTarMember):
            inspect_tar_members(members)

    def test_rejects_unexpected_top_level(self):
        members = [_make_tarinfo("rogue/file")]
        with self.assertRaises(UnsafeTarMember):
            inspect_tar_members(members)

    def test_rejects_oversized(self):
        members = [_make_tarinfo("attachments/big", size=10)]
        with self.assertRaises(UnsafeTarMember):
            inspect_tar_members(members, max_member_bytes=5)


class BackupRoundTripTests(unittest.TestCase):
    def test_export_then_import(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            data_root.mkdir()
            seed = open_db(str(data_root))
            seed.execute("CREATE TABLE marker(x INTEGER)")
            seed.execute("INSERT INTO marker VALUES(99)")
            seed.commit()
            seed.close()
            migrate_plaintext_to_encrypted(data_root / "kassiber.sqlite3", "db-pass")

            backup_path = Path(root) / "snap.kassiber"
            export_backup(
                str(data_root),
                backup_path,
                "db-pass",
                backup_passphrase="outer-pass",
            )
            self.assertTrue(backup_path.exists())
            self.assertGreater(backup_path.stat().st_size, 1024)

            restore_root = Path(root) / "restore"
            (restore_root / "data").mkdir(parents=True)
            result = import_backup(
                backup_path,
                restore_root / "data",
                backup_passphrase="outer-pass",
                move_into_place=True,
            )
            self.assertEqual(result.manifest["schema_version"], 1)
            self.assertIsNone(result.pre_restore_backup)  # clean target
            db_path = restore_root / "data" / "kassiber.sqlite3"
            self.assertTrue(db_path.exists())
            conn = open_encrypted(db_path, "db-pass")
            try:
                self.assertEqual(
                    conn.execute("SELECT x FROM marker").fetchone()[0], 99
                )
            finally:
                conn.close()

    def test_install_preserves_existing_data_root(self):
        """Installing over a populated data root must move the old files
        aside into a `pre-restore-...` directory rather than nuking them."""

        with tempfile.TemporaryDirectory() as root:
            # Build a backup from `data_root_a`.
            data_root_a = Path(root) / "a" / "data"
            data_root_a.mkdir(parents=True)
            seed_a = open_db(str(data_root_a))
            seed_a.execute("CREATE TABLE marker(label TEXT)")
            seed_a.execute("INSERT INTO marker VALUES('imported')")
            seed_a.commit()
            seed_a.close()
            migrate_plaintext_to_encrypted(
                data_root_a / "kassiber.sqlite3", "imported-pass"
            )
            backup_path = Path(root) / "snap.kassiber"
            export_backup(
                str(data_root_a),
                backup_path,
                "imported-pass",
                backup_passphrase="outer-pass",
            )

            # Prepare a different populated target (`data_root_b`).
            data_root_b = Path(root) / "b" / "data"
            data_root_b.mkdir(parents=True)
            seed_b = open_db(str(data_root_b))
            seed_b.execute("CREATE TABLE marker(label TEXT)")
            seed_b.execute("INSERT INTO marker VALUES('local')")
            seed_b.commit()
            seed_b.close()
            migrate_plaintext_to_encrypted(
                data_root_b / "kassiber.sqlite3", "local-pass"
            )
            attachments_root_b = data_root_b.parent / "attachments"
            attachments_root_b.mkdir()
            (attachments_root_b / "keepme.txt").write_text("local-attachment", encoding="utf-8")

            result = import_backup(
                backup_path,
                data_root_b,
                backup_passphrase="outer-pass",
                move_into_place=True,
            )
            self.assertIsNotNone(result.pre_restore_backup)
            self.assertTrue(result.pre_restore_backup.exists())
            # Pre-existing local DB is preserved.
            preserved_db = result.pre_restore_backup / "kassiber.sqlite3"
            self.assertTrue(preserved_db.exists())
            local_conn = open_encrypted(preserved_db, "local-pass")
            try:
                self.assertEqual(
                    local_conn.execute("SELECT label FROM marker").fetchone()[0],
                    "local",
                )
            finally:
                local_conn.close()
            # Pre-existing attachments are preserved.
            self.assertEqual(
                (result.pre_restore_backup / "attachments" / "keepme.txt").read_text(),
                "local-attachment",
            )
            # The newly installed DB carries the imported data.
            new_conn = open_encrypted(data_root_b / "kassiber.sqlite3", "imported-pass")
            try:
                self.assertEqual(
                    new_conn.execute("SELECT label FROM marker").fetchone()[0],
                    "imported",
                )
            finally:
                new_conn.close()


class CredentialLeakageGuardTests(unittest.TestCase):
    """Migration must raise if the encrypted output ever leaks markers."""

    def test_encrypted_file_is_clean_of_markers(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            data_root.mkdir()
            seed = sqlite3.connect(str(data_root / "kassiber.sqlite3"))
            seed.execute("CREATE TABLE secrets(x TEXT)")
            seed.execute("INSERT INTO secrets VALUES('xprv9YYYY')")
            seed.execute("INSERT INTO secrets VALUES('Authorization: Basic XXX')")
            seed.commit()
            seed.close()
            # The migration must succeed: the input contains markers but the
            # encrypted output should not. If this ever flips to raise we
            # want to know immediately.
            result = migrate_plaintext_to_encrypted(
                data_root / "kassiber.sqlite3", "tracer-pass"
            )
            self.assertTrue(result.credential_marker_clean)
            raw = (data_root / "kassiber.sqlite3").read_bytes()
            self.assertNotIn(b"xprv", raw)
            self.assertNotIn(b"Authorization:", raw)

    def test_migration_aborts_when_markers_leak(self):
        """Synthetic regression: if the scanner ever sees a marker, raise."""

        from kassiber.secrets import migration

        with tempfile.TemporaryDirectory() as root:
            db_path = Path(root) / "kassiber.sqlite3"
            seed = sqlite3.connect(str(db_path))
            seed.execute("CREATE TABLE x(a)")
            seed.execute("INSERT INTO x VALUES('benign')")
            seed.commit()
            seed.close()
            sentinel = b"_TOKEN=" + os.urandom(8).hex().encode()
            markers = (sentinel,)

            class _PlantPlaintext:
                """Force a marker hit by writing the sentinel into the
                output file before the scanner runs."""

                def __init__(self, original):
                    self.original = original

                def __call__(self, path, _markers):
                    # Append our sentinel to the encrypted file so the
                    # scanner is guaranteed to find it.
                    with open(path, "ab") as handle:
                        handle.write(sentinel)
                    return self.original(path, _markers)

            real_scan = migration._scan_for_markers
            migration._scan_for_markers = _PlantPlaintext(real_scan)
            try:
                with self.assertRaises(AppError) as ctx:
                    migrate_plaintext_to_encrypted(
                        db_path, "another-pass", credential_markers=markers
                    )
                self.assertEqual(ctx.exception.code, "migration_leaks_plaintext")
            finally:
                migration._scan_for_markers = real_scan


class CredentialMigrationTests(unittest.TestCase):
    """Move plaintext secret entries from `backends.env` into the encrypted DB.

    Covers the V4.1 follow-up that closes the "tokens stay plaintext on
    disk" gap: secret-shaped dotenv keys (token, password, auth_header,
    username + RPC aliases) are lifted into the SQLCipher `backends`
    table, and the dotenv is rewritten with non-secret rows preserved.
    """

    def _seed_backend(self, data_root: Path, name: str, kind: str, url: str) -> None:
        from kassiber.backends import create_db_backend

        conn = open_db(str(data_root), passphrase="tracer-pass-12345")
        try:
            create_db_backend(conn, name, kind, url)
        finally:
            conn.close()

    def test_scan_separates_secrets_from_urls(self):
        from kassiber.secrets.credentials import scan_dotenv_for_secrets

        with tempfile.TemporaryDirectory() as root:
            env_file = Path(root) / "backends.env"
            env_file.write_text(
                "\n".join(
                    [
                        "# bootstrap config",
                        "KASSIBER_BACKEND_BTCPAY_KIND=btcpay",
                        "KASSIBER_BACKEND_BTCPAY_URL=https://btcpay.example.com",
                        "KASSIBER_BACKEND_BTCPAY_TOKEN=tok-xyz",
                        "KASSIBER_BACKEND_CORE_KIND=bitcoinrpc",
                        "KASSIBER_BACKEND_CORE_URL=http://127.0.0.1:8332",
                        "KASSIBER_BACKEND_CORE_RPCUSER=alice",
                        "KASSIBER_BACKEND_CORE_RPCPASSWORD=hunter2",
                        "KASSIBER_DEFAULT_BACKEND=btcpay",
                    ]
                ),
                encoding="utf-8",
            )
            findings = scan_dotenv_for_secrets(env_file)
            keys = {(f["backend"], f["field"]) for f in findings}
            self.assertEqual(
                keys,
                {
                    ("btcpay", "token"),
                    ("core", "username"),
                    ("core", "password"),
                },
                "URLs, kinds, and KASSIBER_DEFAULT_BACKEND must not be flagged",
            )

    def test_scan_handles_missing_file(self):
        from kassiber.secrets.credentials import scan_dotenv_for_secrets

        with tempfile.TemporaryDirectory() as root:
            self.assertEqual(
                scan_dotenv_for_secrets(Path(root) / "absent.env"),
                [],
            )

    def test_migration_lifts_token_and_strips_dotenv(self):
        from kassiber.secrets.credentials import migrate_dotenv_credentials

        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            data_root.mkdir()
            seed = open_db(str(data_root))
            seed.close()
            migrate_plaintext_to_encrypted(
                data_root / "kassiber.sqlite3", "tracer-pass-12345"
            )
            self._seed_backend(
                data_root, "btcpay", "btcpay", "https://btcpay.example.com"
            )

            env_file = Path(root) / "backends.env"
            env_file.write_text(
                "\n".join(
                    [
                        "# kassiber backend bootstrap",
                        "KASSIBER_BACKEND_BTCPAY_KIND=btcpay",
                        "KASSIBER_BACKEND_BTCPAY_URL=https://btcpay.example.com",
                        "KASSIBER_BACKEND_BTCPAY_TOKEN=tok-xyz-123",
                        "",
                    ]
                ),
                encoding="utf-8",
            )

            conn = open_db(str(data_root), passphrase="tracer-pass-12345")
            try:
                result = migrate_dotenv_credentials(conn, env_file)
            finally:
                conn.close()

            self.assertEqual(len(result["migrated"]), 1)
            self.assertEqual(result["skipped"], [])
            self.assertTrue(result["rewritten"])
            self.assertIsNotNone(result["backup_path"])

            sanitized = env_file.read_text(encoding="utf-8")
            self.assertIn("KASSIBER_BACKEND_BTCPAY_KIND=btcpay", sanitized)
            self.assertIn("KASSIBER_BACKEND_BTCPAY_URL=https://btcpay.example.com", sanitized)
            self.assertNotIn("TOKEN", sanitized)

            from kassiber.backends import get_db_backend

            conn = open_db(str(data_root), passphrase="tracer-pass-12345")
            try:
                row = get_db_backend(conn, "btcpay")
                self.assertEqual(row["token"], "tok-xyz-123")
            finally:
                conn.close()

            self.assertIn(
                "TOKEN=tok-xyz-123",
                Path(result["backup_path"]).read_text(encoding="utf-8"),
            )

    def test_migration_skips_unknown_backend(self):
        from kassiber.secrets.credentials import migrate_dotenv_credentials

        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            data_root.mkdir()
            seed = open_db(str(data_root))
            seed.close()
            migrate_plaintext_to_encrypted(
                data_root / "kassiber.sqlite3", "tracer-pass-12345"
            )

            env_file = Path(root) / "backends.env"
            env_file.write_text(
                "KASSIBER_BACKEND_GHOST_TOKEN=tok-orphan\n",
                encoding="utf-8",
            )
            conn = open_db(str(data_root), passphrase="tracer-pass-12345")
            try:
                result = migrate_dotenv_credentials(conn, env_file)
            finally:
                conn.close()

            self.assertEqual(result["migrated"], [])
            self.assertEqual(len(result["skipped"]), 1)
            self.assertEqual(result["skipped"][0]["reason"], "backend_not_in_db")
            self.assertIn(
                "KASSIBER_BACKEND_GHOST_TOKEN=tok-orphan",
                env_file.read_text(encoding="utf-8"),
            )

    def test_username_and_password_lift_into_config_json(self):
        from kassiber.backends import get_db_backend
        from kassiber.secrets.credentials import migrate_dotenv_credentials

        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            data_root.mkdir()
            seed = open_db(str(data_root))
            seed.close()
            migrate_plaintext_to_encrypted(
                data_root / "kassiber.sqlite3", "tracer-pass-12345"
            )
            self._seed_backend(
                data_root, "core", "bitcoinrpc", "http://127.0.0.1:8332"
            )

            env_file = Path(root) / "backends.env"
            env_file.write_text(
                "\n".join(
                    [
                        "KASSIBER_BACKEND_CORE_RPCUSER=alice",
                        "KASSIBER_BACKEND_CORE_RPCPASSWORD=hunter2",
                    ]
                ),
                encoding="utf-8",
            )
            conn = open_db(str(data_root), passphrase="tracer-pass-12345")
            try:
                migrate_dotenv_credentials(conn, env_file)
                row = get_db_backend(conn, "core")
            finally:
                conn.close()
            self.assertEqual(row.get("username"), "alice")
            self.assertEqual(row.get("password"), "hunter2")
            self.assertNotIn("RPCUSER", env_file.read_text(encoding="utf-8"))
            self.assertNotIn("RPCPASSWORD", env_file.read_text(encoding="utf-8"))


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
