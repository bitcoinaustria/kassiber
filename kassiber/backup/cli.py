"""`kassiber backup ...` command handlers."""

from __future__ import annotations

import argparse
from pathlib import Path

from ..envelope import build_envelope
from ..errors import AppError
from ..secrets.prompt import (
    prompt_passphrase,
    prompt_passphrase_with_confirmation,
    read_passphrase_from_fd,
)
from .pack import export_backup, import_backup


def _resolve_backup_passphrase(
    args: argparse.Namespace,
    *,
    label: str,
    confirm: bool = False,
) -> str:
    fd = getattr(args, "backup_passphrase_fd", None)
    if fd is not None:
        return read_passphrase_from_fd(int(fd))
    if confirm:
        return prompt_passphrase_with_confirmation(label, "Confirm backup passphrase: ")
    return prompt_passphrase(label)


def _resolve_db_passphrase(args: argparse.Namespace) -> str:
    fd = getattr(args, "db_passphrase_fd", None)
    if fd is not None:
        return read_passphrase_from_fd(int(fd))
    return prompt_passphrase("Database passphrase: ")


def cmd_backup_export(args: argparse.Namespace) -> dict:
    output = Path(args.file).expanduser()

    db_passphrase = _resolve_db_passphrase(args)

    recipients = args.recipient or None
    backup_passphrase = None
    if not recipients:
        backup_passphrase = _resolve_backup_passphrase(
            args,
            label="Backup passphrase: ",
            confirm=getattr(args, "backup_passphrase_fd", None) is None,
        )

    result = export_backup(
        args.data_root,
        output,
        db_passphrase,
        backup_passphrase=backup_passphrase,
        recipients=recipients,
    )
    return build_envelope(
        "backup.export",
        {
            "output": str(result.output_path),
            "size_bytes": result.output_path.stat().st_size,
            "manifest": result.manifest,
            "age_backend": result.age_backend,
        },
    )


def cmd_backup_import(args: argparse.Namespace) -> dict:
    archive = Path(args.archive).expanduser()
    identity_file = (
        Path(args.identity_file).expanduser() if args.identity_file else None
    )

    backup_passphrase = None
    if identity_file is None:
        backup_passphrase = _resolve_backup_passphrase(
            args,
            label="Backup passphrase: ",
            confirm=False,
        )

    target_data_root = (
        Path(args.target_data_root).expanduser() if args.target_data_root else None
    )

    result = import_backup(
        archive,
        target_data_root or Path(args.data_root),
        backup_passphrase=backup_passphrase,
        identity_file=identity_file,
        move_into_place=bool(args.install),
    )
    return build_envelope(
        "backup.import",
        {
            "archive": str(archive),
            "staging_path": str(result.staging_path),
            "installed_data_root": (
                str(result.installed_data_root)
                if result.installed_data_root
                else None
            ),
            "pre_restore_backup": (
                str(result.pre_restore_backup)
                if result.pre_restore_backup
                else None
            ),
            "manifest": result.manifest,
        },
    )


def add_backup_parser(subparsers) -> argparse.ArgumentParser:
    backup = subparsers.add_parser(
        "backup",
        help="Export or import a `.kassiber` encrypted backup bundle",
    )
    backup_sub = backup.add_subparsers(dest="backup_command", required=True)

    export = backup_sub.add_parser("export", help="Write a `.kassiber` backup file")
    export.add_argument(
        "--file",
        required=True,
        help="Destination `.kassiber` file path (separate from the global --output)",
    )
    export.add_argument(
        "--backup-passphrase-fd",
        type=int,
        default=None,
        metavar="FD",
        help="Read the outer age passphrase from this open file descriptor",
    )
    export.add_argument(
        "--recipient",
        action="append",
        default=None,
        help="Encrypt to an age recipient (e.g. `age1...` or `ssh-ed25519 ...`); may repeat",
    )

    importer = backup_sub.add_parser("import", help="Decrypt and stage a `.kassiber` backup")
    importer.add_argument("archive", help="Path to the `.kassiber` file")
    importer.add_argument(
        "--backup-passphrase-fd",
        type=int,
        default=None,
        metavar="FD",
        help="Read the outer age passphrase from this open file descriptor",
    )
    importer.add_argument(
        "--identity-file",
        default=None,
        help="Path to an age identity file (when the backup was encrypted to recipients)",
    )
    importer.add_argument(
        "--install",
        action="store_true",
        help="Install the decrypted bundle into the target data root after validation",
    )
    importer.add_argument(
        "--target-data-root",
        default=None,
        help="Override the data root for `--install`; defaults to the active --data-root",
    )

    return backup


def dispatch_backup(args: argparse.Namespace) -> dict:
    sub = args.backup_command
    if sub == "export":
        return cmd_backup_export(args)
    if sub == "import":
        return cmd_backup_import(args)
    raise AppError(
        f"unknown backup command: {sub!r}",
        code="unknown_command",
        retryable=False,
    )
