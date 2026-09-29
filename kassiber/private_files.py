"""Small owner-only files for machine-wide preferences.

The update-check consent, offline mode and agent access each keep a tiny JSON
document under the state root's config directory. They share one writer and
one fail-closed reader so none of them can drift into following a symlink or
reading an oversized file.
"""

from __future__ import annotations

import os
import stat
import tempfile
from pathlib import Path


def atomic_write_private(destination: Path, text: str) -> None:
    """Atomically replace `destination` with owner-only (0600) UTF-8 content."""

    destination.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    fd, temporary_name = tempfile.mkstemp(
        prefix=f".{destination.name}.",
        suffix=".tmp",
        dir=destination.parent,
    )
    temporary = Path(temporary_name)
    try:
        try:
            os.fchmod(fd, 0o600)
        except (AttributeError, OSError):
            # mkstemp already creates the file owner-only; this is hardening.
            pass
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            handle.write(text)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, destination)
    finally:
        try:
            temporary.unlink()
        except OSError:
            # Cleanup must not hide the original write/replace failure.
            pass


def read_small_private_file(path: Path, limit: int) -> bytes | None:
    """Read a regular, non-symlinked file of at most `limit` bytes, or None.

    Shared fail-closed reader for the consent file and similar small local
    contracts: symlinks, special files, and oversized content all read as
    absent rather than raising.
    """

    try:
        if stat.S_ISLNK(os.lstat(path).st_mode):
            return None
    except OSError:
        return None
    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NONBLOCK", 0)
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        descriptor = os.open(path, flags)
    except OSError:
        return None
    try:
        if not stat.S_ISREG(os.fstat(descriptor).st_mode):
            return None
        with os.fdopen(descriptor, "rb") as handle:
            descriptor = -1
            raw = handle.read(limit + 1)
    except OSError:
        return None
    finally:
        if descriptor >= 0:
            try:
                os.close(descriptor)
            except OSError:
                # Best-effort cleanup after the read path has already failed.
                pass
    return raw if len(raw) <= limit else None
