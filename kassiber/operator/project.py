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
    alias_lock_path: Path
    local_lock_path: Path
    identity: str
    public_id: str


@dataclass
class ProjectOwnerLease:
    project: CanonicalProject
    owner_kind: str
    generation: str
    _handles: tuple[IO[bytes], ...]
    _lock_paths: set[Path]
    _released: bool = False

    def release(self) -> None:
        if self._released:
            return
        self._released = True
        # Closing (rather than issuing an explicit unlock) preserves the lock
        # when a worker child inherited a duplicate of the same file object.
        for handle in reversed(self._handles):
            handle.close()

    def duplicate_for_child(self) -> ProjectOwnerChildHandles:
        """Duplicate every held lock into an inheritable child-only handle."""

        duplicates: list[IO[bytes]] = []
        tokens: list[int] = []
        try:
            for handle in self._handles:
                duplicate_fd = os.dup(handle.fileno())
                duplicate = os.fdopen(duplicate_fd, "r+b", buffering=0)
                duplicates.append(duplicate)
                if os.name == "nt":
                    import msvcrt

                    token = int(msvcrt.get_osfhandle(duplicate.fileno()))
                    os.set_handle_inheritable(token, True)
                else:
                    token = duplicate.fileno()
                    os.set_inheritable(token, True)
                tokens.append(token)
            return ProjectOwnerChildHandles(tuple(tokens), tuple(duplicates))
        except Exception:
            for duplicate in duplicates:
                duplicate.close()
            raise

    def add_alias(self, project: CanonicalProject) -> None:
        """Hold every path-local lock for another alias of the same database."""

        if project.identity != self.project.identity:
            raise AppError(
                "the project alias resolves to a different database",
                code="project_owner_mismatch",
                retryable=False,
            )
        for lock_path in (project.alias_lock_path, project.local_lock_path):
            if lock_path not in self._lock_paths:
                self._add_lock(lock_path, project.public_id)

    def _add_lock(self, lock_path: Path, project_id: str) -> None:
        handle = _open_owner_lock(lock_path, project_id)
        try:
            if not _try_lock_handle(handle):
                owner = _read_owner_record(handle)
                raise AppError(
                    "another long-lived process owns this project path",
                    code="project_in_use",
                    details={
                        "project": project_id,
                        "owner": owner.get("owner", "unknown"),
                        "generation": owner.get("generation"),
                    },
                    retryable=True,
                )
            record = json.dumps(
                {
                    "schema_version": 1,
                    "owner": self.owner_kind,
                    "generation": self.generation,
                    "pid": os.getpid(),
                },
                sort_keys=True,
            ).encode("utf-8")
            handle.seek(0)
            handle.truncate(0)
            handle.write(record + b"\n")
            self._handles = (*self._handles, handle)
            self._lock_paths.add(lock_path)
        except Exception:
            try:
                _unlock_handle(handle)
            finally:
                handle.close()
            raise

    def __enter__(self) -> ProjectOwnerLease:
        return self

    def __exit__(self, *_exc: object) -> None:
        self.release()


@dataclass
class ProjectOwnerChildHandles:
    """Parent-side duplicates that are inherited by one worker child."""

    tokens: tuple[int, ...]
    _handles: tuple[IO[bytes], ...]
    _closed: bool = False

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        first_error: Exception | None = None
        for handle in self._handles:
            try:
                handle.close()
            except Exception as exc:
                if first_error is None:
                    first_error = exc
        if first_error is not None:
            raise first_error


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
        _require_windows_path_owner(database)
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
            _require_windows_path_owner(parent)
        identity_material = f"path:{database}"
    if hasattr(os, "getuid"):
        principal = str(os.getuid())
    else:
        from .protocol import _windows_current_sid

        principal = _windows_current_sid()
    identity = hashlib.sha256(
        f"kassiber-operator-v1:{sys.platform}:{principal}:{identity_material}".encode(
            "utf-8"
        )
    ).hexdigest()
    lock_root = _owner_lock_root()
    alias_digest = hashlib.sha256(str(database).encode("utf-8")).hexdigest()
    return CanonicalProject(
        database=database,
        lock_path=lock_root / f"identity-{identity}.lock",
        alias_lock_path=lock_root / f"path-{alias_digest}.lock",
        local_lock_path=parent / OWNER_LOCK_FILENAME,
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
    handles: list[IO[bytes]] = []
    try:
        # Identity preserves ownership across moves; the global path lock and
        # project-local lock prevent replacement or differing runtime-directory
        # selections from creating a second owner at the admitted project.
        lock_paths = (
            project.lock_path,
            project.alias_lock_path,
            project.local_lock_path,
        )
        for lock_path in dict.fromkeys(lock_paths):
            handle = _open_owner_lock(lock_path, project.public_id)
            handles.append(handle)
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
        for handle in handles:
            handle.seek(0)
            handle.truncate(0)
            handle.write(record + b"\n")
        return ProjectOwnerLease(
            project,
            owner_kind,
            generation,
            tuple(handles),
            set(lock_paths),
        )
    except Exception:
        for handle in reversed(handles):
            try:
                _unlock_handle(handle)
            finally:
                handle.close()
        raise


def _owner_lock_root() -> Path:
    # Ownership exclusion is a security invariant, so it must not follow the
    # configurable broker endpoint/test rendezvous. The project-local lock is
    # the primary cross-environment guard; this stable per-user namespace also
    # preserves inode identity across path moves and hard-link aliases.
    if os.name == "nt":
        root = _windows_local_appdata() / "Kassiber" / "run" / "owners"
    else:
        import pwd

        # Resolve the account home from the user database rather than HOME or
        # XDG variables so every normal process for this UID rendezvouses in a
        # persistent namespace that tmpfile cleanup cannot unlink mid-lease.
        account_home = Path(pwd.getpwuid(os.getuid()).pw_dir)
        root = account_home / ".kassiber" / "run" / "operator-owners"
    if root.is_symlink():
        raise AppError(
            "the project owner lock directory may not be a symlink",
            code="unsafe_project_owner_lock",
            retryable=False,
        )
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    info = root.stat()
    if not stat.S_ISDIR(info.st_mode):
        raise AppError(
            "the project owner lock path is not a directory",
            code="unsafe_project_owner_lock",
            retryable=False,
        )
    _require_current_owner(info)
    if os.name != "nt":
        os.chmod(root, 0o700)
    return root.resolve(strict=True)


def _windows_local_appdata() -> Path:
    """Resolve Local AppData through the shell API, not caller environment."""

    import ctypes

    buffer = ctypes.create_unicode_buffer(32768)
    result = ctypes.windll.shell32.SHGetFolderPathW(
        None,
        0x001C,  # CSIDL_LOCAL_APPDATA
        None,
        0,
        buffer,
    )
    if result != 0 or not buffer.value:
        raise AppError(
            "the stable project ownership directory is unavailable",
            code="project_owner_lock_unavailable",
            retryable=True,
        )
    return Path(buffer.value)


def _open_owner_lock(lock_path: Path, project_id: str) -> IO[bytes]:
    if os.name == "nt":
        return _open_windows_owner_lock(lock_path, project_id)
    flags = os.O_RDWR | os.O_CREAT
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        fd = os.open(lock_path, flags, 0o600)
    except OSError as exc:
        raise AppError(
            "the project ownership lock is unavailable",
            code="project_owner_lock_unavailable",
            details={"project": project_id},
            retryable=True,
        ) from exc
    handle = os.fdopen(fd, "r+b", buffering=0)
    try:
        info = os.fstat(handle.fileno())
        if not stat.S_ISREG(info.st_mode):
            raise AppError(
                "the project ownership lock is unsafe",
                code="unsafe_project_owner_lock",
                details={"project": project_id},
                retryable=False,
            )
        _require_current_owner(info)
        _require_windows_path_owner(lock_path)
        if os.name != "nt":
            os.fchmod(handle.fileno(), 0o600)
        return handle
    except Exception:
        handle.close()
        raise


def _open_windows_owner_lock(lock_path: Path, project_id: str) -> IO[bytes]:
    """Open with share-mode zero so inherited duplicates preserve exclusion."""

    import ctypes
    import msvcrt
    from ctypes import wintypes

    create_file = ctypes.WinDLL("kernel32", use_last_error=True).CreateFileW
    create_file.argtypes = [
        wintypes.LPCWSTR,
        wintypes.DWORD,
        wintypes.DWORD,
        wintypes.LPVOID,
        wintypes.DWORD,
        wintypes.DWORD,
        wintypes.HANDLE,
    ]
    create_file.restype = wintypes.HANDLE
    handle = create_file(
        str(lock_path),
        0x80000000 | 0x40000000,  # GENERIC_READ | GENERIC_WRITE
        0,  # no sharing: duplicate/inherited handles keep this reservation
        None,
        4,  # OPEN_ALWAYS
        0x80,  # FILE_ATTRIBUTE_NORMAL
        None,
    )
    invalid_handle = ctypes.c_void_p(-1).value
    if handle == invalid_handle:
        error = ctypes.get_last_error()
        if error in {32, 33}:  # ERROR_SHARING_VIOLATION / ERROR_LOCK_VIOLATION
            raise AppError(
                "another long-lived process owns this project",
                code="project_in_use",
                details={"project": project_id, "owner": "unknown"},
                retryable=True,
            )
        raise AppError(
            "the project ownership lock is unavailable",
            code="project_owner_lock_unavailable",
            details={"project": project_id},
            retryable=True,
        )
    try:
        fd = msvcrt.open_osfhandle(int(handle), os.O_RDWR)
    except Exception:
        ctypes.WinDLL("kernel32", use_last_error=True).CloseHandle(handle)
        raise
    file_handle = os.fdopen(fd, "r+b", buffering=0)
    try:
        info = os.fstat(file_handle.fileno())
        if not stat.S_ISREG(info.st_mode):
            raise AppError(
                "the project ownership lock is unsafe",
                code="unsafe_project_owner_lock",
                details={"project": project_id},
                retryable=False,
            )
        _require_windows_path_owner(lock_path)
        return file_handle
    except Exception:
        file_handle.close()
        raise


def _require_current_owner(info: os.stat_result) -> None:
    if hasattr(os, "getuid") and info.st_uid != os.getuid():
        raise AppError(
            "the project path is owned by another OS user",
            code="unsafe_project_owner",
            retryable=False,
        )


def _require_windows_path_owner(path: Path) -> None:
    if os.name != "nt":
        return
    from .protocol import windows_path_owned_by_current_user

    try:
        owned = windows_path_owned_by_current_user(str(path))
    except OSError as exc:
        raise AppError(
            "the project path owner could not be verified",
            code="unsafe_project_owner",
            retryable=False,
        ) from exc
    if not owned:
        raise AppError(
            "the project path is owned by another OS user",
            code="unsafe_project_owner",
            retryable=False,
        )


def _try_lock_handle(handle: IO[bytes]) -> bool:
    if os.name == "nt":
        # Share-mode zero was acquired atomically by _open_windows_owner_lock.
        return True
    import fcntl

    try:
        fcntl.flock(handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        return True
    except BlockingIOError:
        return False


def _unlock_handle(handle: IO[bytes]) -> None:
    if os.name == "nt":
        # Closing the last duplicate releases the share-mode reservation.
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
