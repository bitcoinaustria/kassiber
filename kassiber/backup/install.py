"""Install a validated backup with same-filesystem staging and rollback.

The caller must close database connections and hold exclusive project maintenance.
Existing database sidecars travel with the recovery copy, never with the new DB.
"""
from __future__ import annotations

import os
from pathlib import Path
import shutil
import tempfile

from ..errors import AppError


def restore_targets(data_root: Path) -> dict[str, Path]:
    data_root = Path(data_root).expanduser().resolve()
    state = data_root.parent if data_root.name == "data" else data_root
    return {
        "kassiber.sqlite3": data_root / "kassiber.sqlite3",
        "kassiber.sqlite3-wal": data_root / "kassiber.sqlite3-wal",
        "kassiber.sqlite3-shm": data_root / "kassiber.sqlite3-shm",
        "attachments": state / "attachments",
        "config/backends.env": state / "config/backends.env",
        "config/settings.json": state / "config/settings.json",
        "exports": state / "exports",
    }


def install_staged_backup(staging: Path, data_root: Path) -> Path | None:
    targets = restore_targets(data_root)
    state = targets["attachments"].parent
    # Do not follow sidecar/destination symlinks outside the previewed container.
    for target in targets.values():
        for part in (target, *target.parents):
            if part.is_symlink():
                raise AppError("Restore destination contains a symbolic link", code="unsafe_restore_target")
    state.mkdir(parents=True, exist_ok=True)
    prepared = Path(tempfile.mkdtemp(prefix=".restore-install-", dir=state))
    recovery: Path | None = None
    moved: list[str] = []
    installed: list[str] = []
    try:
        # Finish potentially failing copies before touching the live tree.
        for name in targets:
            source = staging / name
            if name in {"exports", "kassiber.sqlite3-wal", "kassiber.sqlite3-shm"} or not source.exists():
                continue
            destination = prepared / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            if source.is_dir():
                shutil.copytree(source, destination)
            else:
                shutil.copy2(source, destination)
        for copied in prepared.rglob("*"):
            if copied.is_file():
                with copied.open("rb") as handle:
                    os.fsync(handle.fileno())
        if any(target.exists() for target in targets.values()):
            recovery = Path(tempfile.mkdtemp(prefix="pre-restore-", dir=state))
        try:
            for name, target in targets.items():
                if target.exists():
                    assert recovery is not None
                    saved = recovery / name
                    saved.parent.mkdir(parents=True, exist_ok=True)
                    os.replace(target, saved)
                    moved.append(name)
            for name, target in targets.items():
                source = prepared / name
                if source.exists():
                    target.parent.mkdir(parents=True, exist_ok=True)
                    os.replace(source, target)
                    installed.append(name)
        except Exception as failure:
            try:
                for name in reversed(installed):
                    target = targets[name]
                    if target.is_dir():
                        shutil.rmtree(target)
                    else:
                        target.unlink()
                for name in reversed(moved):
                    os.replace(recovery / name, targets[name])
            except Exception as rollback_failure:
                raise AppError(
                    "Restore failed and automatic recovery could not finish",
                    code="restore_rollback_failed",
                    details={"recovery_path": str(recovery)},
                    hint=f"Keep the recovery copy at {recovery}; restore it before reopening this container.",
                ) from rollback_failure
            raise AppError(
                "Restore could not be installed; the original data was restored",
                code="restore_install_failed",
                hint="Check destination permissions and available space, then preview again.",
            ) from failure
        return recovery
    except OSError as failure:
        raise AppError(
            "Restore preparation failed; the original data is unchanged",
            code="restore_install_failed",
            hint="Check destination permissions and available space, then preview again.",
        ) from failure
    finally:
        shutil.rmtree(prepared, ignore_errors=True)
