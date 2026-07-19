"""Canonical project identity and long-lived owner exclusion."""

from __future__ import annotations

import hashlib
import json
import os
import stat
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import IO

from ..db import resolve_database_path
from ..errors import AppError


OWNER_LOCK_FILENAME = ".operator-owner.lock"
_OWNER_KINDS = frozenset({"broker", "desktop"})


@dataclass(frozen=True)
class CanonicalProject:
    database: Path
    lock_path: Path
    identity: str
    public_id: str


@dataclass
class ProjectOwnerLease:
    project: CanonicalProject
    owner_kind: str
    generation: str
    _handle: IO[bytes]
    _released: bool = False

    def release(self) -> None:
        if self._released:
            return
        self._released = True
        try:
            _unlock_handle(self._handle)
        finally:
            self._handle.close()

    def __enter__(self) -> ProjectOwnerLease:
        return self

    def __exit__(self, *_exc: object) -> None:
        self.release()


def canonical_project(data_root: str | os.PathLike[str]) -> CanonicalProject:
    """Resolve aliases and derive an owner-local, non-path project identity."""

    database = resolve_database_path(data_root).expanduser().resolve(strict=False)
    parent = database.parent.resolve(strict=False)
    if database.exists():
        info = database.stat()
        if not stat.S_ISREG(info.st_mode):
            raise AppError(
                "the project database is not a regular file",
                code="unsafe_project_database",
                retryable=False,
            )
        _require_current_owner(info)
        identity_material = f"file:{info.st_dev}:{info.st_ino}"
    else:
        if parent.exists():
            info = parent.stat()
            if not stat.S_ISDIR(info.st_mode):
                raise AppError(
                    "the project data directory is not a directory",
                    code="unsafe_project_database",
                    retryable=False,
                )
            _require_current_owner(info)
        identity_material = f"path:{database}"
    principal = str(os.getuid()) if hasattr(os, "getuid") else os.environ.get("USERNAME", "")
    identity = hashlib.sha256(
        f"kassiber-operator-v1:{sys.platform}:{principal}:{identity_material}".encode(
            "utf-8"
        )
    ).hexdigest()
    return CanonicalProject(
        database=database,
        lock_path=parent / OWNER_LOCK_FILENAME,
        identity=identity,
        public_id=identity[:16],
    )


def acquire_project_ownership(
    project: CanonicalProject,
    *,
    owner_kind: str,
    generation: str,
) -> ProjectOwnerLease:
    """Acquire the canonical long-lived owner lock without waiting."""

    if owner_kind not in _OWNER_KINDS:
        raise ValueError(f"invalid owner kind: {owner_kind}")
    project.lock_path.parent.mkdir(parents=True, exist_ok=True)
    flags = os.O_RDWR | os.O_CREAT
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        fd = os.open(project.lock_path, flags, 0o600)
    except OSError as exc:
        raise AppError(
            "the project ownership lock is unavailable",
            code="project_owner_lock_unavailable",
            details={"project": project.public_id},
            retryable=True,
        ) from exc
    handle = os.fdopen(fd, "r+b", buffering=0)
    try:
        info = os.fstat(handle.fileno())
        if not stat.S_ISREG(info.st_mode):
            raise AppError(
                "the project ownership lock is unsafe",
                code="unsafe_project_owner_lock",
                details={"project": project.public_id},
                retryable=False,
            )
        _require_current_owner(info)
        if os.name != "nt":
            os.fchmod(handle.fileno(), 0o600)
        if not _try_lock_handle(handle):
            owner = _read_owner_record(handle)
            raise AppError(
                "another long-lived process owns this project",
                code="project_in_use",
                hint="Lock the active broker lease or close the desktop project, then retry.",
                details={
                    "project": project.public_id,
                    "owner": owner.get("owner", "unknown"),
                    "generation": owner.get("generation"),
                },
                retryable=True,
            )
        record = json.dumps(
            {
                "schema_version": 1,
                "owner": owner_kind,
                "generation": generation,
                "pid": os.getpid(),
            },
            sort_keys=True,
        ).encode("utf-8")
        handle.seek(0)
        handle.truncate(0)
        handle.write(record + b"\n")
        return ProjectOwnerLease(project, owner_kind, generation, handle)
    except Exception:
        handle.close()
        raise


def _require_current_owner(info: os.stat_result) -> None:
    if hasattr(os, "getuid") and info.st_uid != os.getuid():
        raise AppError(
            "the project path is owned by another OS user",
            code="unsafe_project_owner",
            retryable=False,
        )


def _try_lock_handle(handle: IO[bytes]) -> bool:
    if os.name == "nt":
        import msvcrt

        try:
            handle.seek(0)
            msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
            return True
        except OSError:
            return False
    import fcntl

    try:
        fcntl.flock(handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        return True
    except BlockingIOError:
        return False


def _unlock_handle(handle: IO[bytes]) -> None:
    if os.name == "nt":
        import msvcrt

        try:
            handle.seek(0)
            msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)
        except OSError:
            pass
        return
    import fcntl

    try:
        fcntl.flock(handle.fileno(), fcntl.LOCK_UN)
    except OSError:
        pass


def _read_owner_record(handle: IO[bytes]) -> dict[str, object]:
    try:
        handle.seek(0)
        raw = handle.read(4096)
        payload = json.loads(raw.decode("utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError, TypeError, ValueError):
        return {}
    if not isinstance(payload, dict):
        return {}
    owner = payload.get("owner")
    generation = payload.get("generation")
    return {
        "owner": owner if owner in _OWNER_KINDS else "unknown",
        "generation": generation if isinstance(generation, str) else None,
    }
