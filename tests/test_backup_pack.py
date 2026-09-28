import json
import shutil
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import argparse

from kassiber.backup import cli as backup_cli
from kassiber.backup import pack as backup_pack
from kassiber.backup.age_cli import AgeBackend
from kassiber.backup.pack import import_backup
from kassiber.errors import AppError
from kassiber.operator.project import acquire_project_ownership, canonical_project


def _write_file_tar_member(tar: tarfile.TarFile, name: str, payload: bytes) -> None:
    info = tarfile.TarInfo(name)
    info.size = len(payload)
    tar.addfile(info, fileobj=backup_pack.BytesIO(payload))


def _manifest(*, attachments_files: int) -> dict:
    return {
        "schema_version": backup_pack.MANIFEST_SCHEMA_VERSION,
        "entries": {
            "database": backup_pack.BACKUP_DB_NAME,
            "attachments_files": attachments_files,
            "backends_env": False,
            "settings_json": False,
        },
        "secret_refs": {"ai_provider_refs": []},
    }


def _write_bundle(
    path: Path,
    manifest: dict,
    *,
    attachments: dict[str, bytes] | None = None,
) -> None:
    with tarfile.open(path, "w") as tar:
        _write_file_tar_member(tar, backup_pack.BACKUP_DB_NAME, b"db")
        for relpath, payload in sorted((attachments or {}).items()):
            _write_file_tar_member(
                tar,
                f"{backup_pack.BACKUP_ATTACHMENTS_DIR}/{relpath}",
                payload,
            )
        _write_file_tar_member(
            tar,
            backup_pack.BACKUP_MANIFEST_NAME,
            json.dumps(manifest).encode("utf-8"),
        )


def _copy_decrypted_tar(source, destination, **kwargs) -> None:
    del kwargs
    shutil.copyfileobj(source, destination)


class BackupPackManifestValidationTests(unittest.TestCase):
    def test_import_rejects_missing_declared_attachments(self):
        with tempfile.TemporaryDirectory() as root:
            archive = Path(root) / "tampered.kassiber"
            _write_bundle(archive, _manifest(attachments_files=1))

            with patch.object(
                backup_pack,
                "decrypt_age_stream",
                side_effect=_copy_decrypted_tar,
            ):
                with self.assertRaises(AppError) as ctx:
                    import_backup(
                        archive,
                        Path(root) / "target" / "data",
                        backup_passphrase="outer-pass",
                        age_backend=AgeBackend("fake"),
                    )

            self.assertEqual(ctx.exception.code, "invalid_backup")
            self.assertEqual(ctx.exception.details, {"declared": 1, "actual": 0})

    def test_restore_moves_existing_exports_aside_but_does_not_restore_exports(self):
        with tempfile.TemporaryDirectory() as root:
            archive = Path(root) / "snap.kassiber"
            _write_bundle(archive, _manifest(attachments_files=0))
            target_state = Path(root) / "target"
            target_data = target_state / "data"
            target_data.mkdir(parents=True)
            (target_data / backup_pack.BACKUP_DB_NAME).write_bytes(b"old-db")
            exports_root = target_state / "exports"
            exports_root.mkdir()
            (exports_root / "old-report.csv").write_text(
                "local export",
                encoding="utf-8",
            )

            with patch.object(
                backup_pack,
                "decrypt_age_stream",
                side_effect=_copy_decrypted_tar,
            ):
                result = import_backup(
                    archive,
                    target_data,
                    backup_passphrase="outer-pass",
                    age_backend=AgeBackend("fake"),
                    move_into_place=True,
                )

            self.assertIsNotNone(result.pre_restore_backup)
            self.assertFalse(exports_root.exists())
            self.assertTrue(
                (result.pre_restore_backup / "exports" / "old-report.csv").exists()
            )
            self.assertFalse((target_state / "exports" / "old-report.csv").exists())

    def test_restore_keeps_a_hot_rollback_journal_with_the_old_database(self):
        with tempfile.TemporaryDirectory() as root:
            archive = Path(root) / "snap.kassiber"
            _write_bundle(archive, _manifest(attachments_files=0))
            target_data = Path(root) / "target" / "data"
            target_data.mkdir(parents=True)
            (target_data / backup_pack.BACKUP_DB_NAME).write_bytes(b"old-db")
            journal = target_data / f"{backup_pack.BACKUP_DB_NAME}-journal"
            journal.write_bytes(b"old-journal")

            with patch.object(
                backup_pack,
                "decrypt_age_stream",
                side_effect=_copy_decrypted_tar,
            ):
                result = import_backup(
                    archive,
                    target_data,
                    backup_passphrase="outer-pass",
                    age_backend=AgeBackend("fake"),
                    move_into_place=True,
                )

            self.assertFalse(journal.exists())
            self.assertEqual(
                (result.pre_restore_backup / journal.name).read_bytes(),
                b"old-journal",
            )


class BackupImportCliExclusionTests(unittest.TestCase):
    def _args(self, root: Path, *, install: bool) -> argparse.Namespace:
        return argparse.Namespace(
            archive=str(root / "snap.kassiber"),
            identity_file=None,
            target_data_root=None,
            data_root=str(root / "data"),
            install=install,
        )

    def test_install_refuses_a_project_the_desktop_owns(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            data = root / "data"
            data.mkdir()
            (data / backup_pack.BACKUP_DB_NAME).write_bytes(b"live-db")
            owner = acquire_project_ownership(
                canonical_project(data), owner_kind="desktop", generation="test"
            )
            try:
                with patch.object(
                    backup_cli, "_resolve_backup_passphrase", return_value="pass"
                ), patch.object(backup_cli, "import_backup") as importer:
                    with self.assertRaises(AppError) as ctx:
                        backup_cli.cmd_backup_import(self._args(root, install=True))
            finally:
                owner.release()

            self.assertEqual(ctx.exception.code, "project_in_use")
            importer.assert_not_called()
            self.assertEqual((data / backup_pack.BACKUP_DB_NAME).read_bytes(), b"live-db")

    def test_install_takes_exclusive_access_before_a_database_exists(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            data = root / "data"
            data.mkdir()
            owner = acquire_project_ownership(
                canonical_project(data), owner_kind="broker", generation="test"
            )
            try:
                with patch.object(
                    backup_cli, "_resolve_backup_passphrase", return_value="pass"
                ), patch.object(backup_cli, "import_backup") as importer:
                    with self.assertRaises(AppError) as ctx:
                        backup_cli.cmd_backup_import(self._args(root, install=True))
            finally:
                owner.release()

            self.assertEqual(ctx.exception.code, "project_in_use")
            importer.assert_not_called()

    def test_install_into_a_new_directory_creates_and_holds_it(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            data = root / "data"
            installed = backup_pack.BackupImportResult(
                staging_path=None, installed_data_root=data, manifest={}
            )

            def import_while_held(*_args, **_kwargs):
                # A desktop opening the new project now must be refused.
                with self.assertRaises(AppError) as ctx:
                    acquire_project_ownership(
                        canonical_project(data), owner_kind="desktop", generation="late"
                    )
                self.assertEqual(ctx.exception.code, "project_in_use")
                return installed

            with patch.object(
                backup_cli, "_resolve_backup_passphrase", return_value="pass"
            ), patch.object(backup_cli, "import_backup", side_effect=import_while_held):
                result = backup_cli.cmd_backup_import(self._args(root, install=True))

            self.assertEqual(result["installed_data_root"], str(data))

    def test_staging_only_import_does_not_need_exclusive_access(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            data = root / "data"
            data.mkdir()
            (data / backup_pack.BACKUP_DB_NAME).write_bytes(b"live-db")
            owner = acquire_project_ownership(
                canonical_project(data), owner_kind="desktop", generation="test"
            )
            staged = backup_pack.BackupImportResult(
                staging_path=root / "staged",
                installed_data_root=None,
                manifest={},
            )
            try:
                with patch.object(
                    backup_cli, "_resolve_backup_passphrase", return_value="pass"
                ), patch.object(backup_cli, "import_backup", return_value=staged) as importer:
                    result = backup_cli.cmd_backup_import(self._args(root, install=False))
            finally:
                owner.release()

            importer.assert_called_once()
            self.assertEqual(result["staging_path"], str(root / "staged"))


if __name__ == "__main__":
    unittest.main()
