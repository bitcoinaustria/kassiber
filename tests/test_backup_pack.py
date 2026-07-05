import json
import shutil
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from kassiber.backup import pack as backup_pack
from kassiber.backup.age_cli import AgeBackend
from kassiber.backup.pack import import_backup
from kassiber.errors import AppError


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


if __name__ == "__main__":
    unittest.main()
