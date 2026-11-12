"""Flush files, directories and SQLite commits all the way to the device.

On macOS `fsync(2)` only hands data to the drive, whose cache may still lose
it on power failure; `fcntl(F_FULLFSYNC)` also flushes that cache. Rust's
`File::sync_all` already uses it there, and the Rust store sets the same two
SQLite pragmas, so both runtimes give a book the same durability.

`_platform()` is the seam tests replace to exercise the macOS branch on Linux.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

try:
    import fcntl
except ImportError:  # Windows
    fcntl = None


def _platform() -> str:
    return sys.platform


def full_fsync(fd: int) -> None:
    """Flush `fd` to stable storage: `F_FULLFSYNC` on macOS, else `fsync`.

    Like SQLite, fall back to `fsync` where the file system refuses
    `F_FULLFSYNC` (some network and FUSE volumes) instead of failing the write.
    """

    if _platform() == "darwin" and fcntl is not None:
        try:
            fcntl.fcntl(fd, fcntl.F_FULLFSYNC)
            return
        except OSError:
            pass
    os.fsync(fd)


def sync_directory(path: str | os.PathLike) -> None:
    """Make a rename or create inside `path` durable. No-op on Windows."""

    if os.name == "nt":
        return
    fd = os.open(Path(path), os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
    try:
        full_fsync(fd)
    finally:
        os.close(fd)


def sync_file(path: str | os.PathLike) -> None:
    """Flush an already written file by path.

    Windows flushes (`_commit`, i.e. `FlushFileBuffers`) need a handle with
    write access and fail with EBADF on a read-only one; elsewhere a
    read-only descriptor is enough.
    """

    access = os.O_RDWR if _platform() == "win32" else os.O_RDONLY
    fd = os.open(Path(path), access | getattr(os, "O_BINARY", 0))
    try:
        full_fsync(fd)
    finally:
        os.close(fd)


def sync_tree(root: str | os.PathLike) -> None:
    """Flush every regular file, then every directory bottom-up, under `root`."""

    root = Path(root)
    directories = [root]
    for current, dirnames, filenames in os.walk(root):
        base = Path(current)
        for name in filenames:
            path = base / name
            if path.is_file() and not path.is_symlink():
                sync_file(path)
        directories.extend(base / name for name in dirnames if not (base / name).is_symlink())
    for directory in sorted(directories, key=lambda item: len(item.parts), reverse=True):
        sync_directory(directory)


SQLITE_FULLFSYNC_PRAGMAS = ("PRAGMA fullfsync = ON", "PRAGMA checkpoint_fullfsync = ON")


def apply_sqlite_durability(conn) -> None:
    """Make SQLite commits and WAL checkpoints use `F_FULLFSYNC` on macOS.

    Per connection and covering attached databases; stored bytes are unchanged.
    Other platforms keep SQLite's defaults, where the pragmas do nothing.
    """

    if _platform() == "darwin":
        for statement in SQLITE_FULLFSYNC_PRAGMAS:
            conn.execute(statement)
