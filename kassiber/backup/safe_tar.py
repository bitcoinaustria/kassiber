"""Strict tar member inspection for `.kassiber` import.

The Python stdlib `tarfile` docs are explicit: `extractall()` is unsafe
on untrusted archives. We do our own member validation that rejects any
of:

- absolute paths,
- relative paths that escape the staging directory,
- symlinks and hardlinks,
- device nodes, FIFOs, and sockets,
- duplicate paths,
- entries whose declared sizes exceed a configurable per-file cap.

Allowed top-level entries are restricted to the manifest, the bundled
SQLCipher database, the attachments tree, and the config tree. Any
other top-level path causes a hard error.
"""

from __future__ import annotations

import os
import tarfile
from pathlib import Path
from typing import Iterable

from ..errors import AppError


_DEFAULT_ALLOWED_TOP_LEVEL = (
    "manifest.json",
    "kassiber.sqlite3",
    "attachments",
    "config",
)

# 4 GiB per file is enough headroom for a full attachments tree without
# letting a malicious archive trigger an out-of-disk DoS without warning.
_DEFAULT_MAX_MEMBER_BYTES = 4 * 1024 * 1024 * 1024


class UnsafeTarMember(Exception):
    """Raised when a tar member fails validation."""

    def __init__(self, *, name: str, reason: str) -> None:
        super().__init__(f"unsafe tar member {name!r}: {reason}")
        self.name = name
        self.reason = reason


def _normalize_member_path(name: str) -> str:
    if name.startswith("/"):
        raise UnsafeTarMember(name=name, reason="absolute path")
    if "\\" in name:
        raise UnsafeTarMember(name=name, reason="backslash in path (Windows-style)")
    if "\x00" in name:
        raise UnsafeTarMember(name=name, reason="NUL byte in path")
    parts = []
    for part in name.split("/"):
        if part == "" or part == ".":
            continue
        if part == "..":
            raise UnsafeTarMember(name=name, reason="parent-directory traversal")
        parts.append(part)
    if not parts:
        raise UnsafeTarMember(name=name, reason="empty path after normalization")
    return "/".join(parts)


def inspect_tar_members(
    members: Iterable[tarfile.TarInfo],
    *,
    allowed_top_level: Iterable[str] = _DEFAULT_ALLOWED_TOP_LEVEL,
    max_member_bytes: int = _DEFAULT_MAX_MEMBER_BYTES,
) -> list[tarfile.TarInfo]:
    """Return a sanitized copy of `members`, raising on any unsafe entry."""

    seen: set[str] = set()
    allowed = set(allowed_top_level)
    sanitized: list[tarfile.TarInfo] = []

    for info in members:
        normalized = _normalize_member_path(info.name)
        top = normalized.split("/", 1)[0]
        if top not in allowed:
            raise UnsafeTarMember(
                name=info.name,
                reason=f"top-level entry {top!r} not in {sorted(allowed)}",
            )

        if info.issym():
            raise UnsafeTarMember(name=info.name, reason="symlink")
        if info.islnk():
            raise UnsafeTarMember(name=info.name, reason="hardlink")
        if info.ischr() or info.isblk():
            raise UnsafeTarMember(name=info.name, reason="device node")
        if info.isfifo():
            raise UnsafeTarMember(name=info.name, reason="FIFO")
        if not (info.isfile() or info.isdir()):
            raise UnsafeTarMember(
                name=info.name,
                reason=f"unsupported member type {info.type!r}",
            )

        if info.isfile() and info.size > max_member_bytes:
            raise UnsafeTarMember(
                name=info.name,
                reason=(
                    f"declared size {info.size} exceeds limit {max_member_bytes}"
                ),
            )

        if normalized in seen:
            raise UnsafeTarMember(name=info.name, reason="duplicate path")
        seen.add(normalized)

        # Replace the tarinfo's name with the normalized one so the
        # extractor cannot be tricked by stray "./" prefixes.
        clean = tarfile.TarInfo(name=normalized)
        clean.size = info.size
        clean.mtime = info.mtime
        clean.mode = info.mode & 0o777
        clean.type = info.type
        clean.uid = 0
        clean.gid = 0
        clean.uname = ""
        clean.gname = ""
        sanitized.append(clean)

    return sanitized


def extract_tar_safely(
    tar: tarfile.TarFile,
    destination: Path,
    *,
    allowed_top_level: Iterable[str] = _DEFAULT_ALLOWED_TOP_LEVEL,
    max_member_bytes: int = _DEFAULT_MAX_MEMBER_BYTES,
) -> list[Path]:
    """Extract `tar` into `destination`, applying full member validation.

    Returns the list of extracted file/dir paths. The caller owns the
    `destination` directory and is responsible for atomically moving it
    into place once validation succeeds.
    """

    destination = Path(destination).expanduser().resolve()
    destination.mkdir(parents=True, exist_ok=True)
    members = tar.getmembers()
    try:
        sanitized = inspect_tar_members(
            members,
            allowed_top_level=allowed_top_level,
            max_member_bytes=max_member_bytes,
        )
    except UnsafeTarMember as exc:
        raise AppError(
            f"backup contains an unsafe tar member: {exc.reason}",
            code="unsafe_backup_member",
            details={"member": exc.name, "reason": exc.reason},
            retryable=False,
        ) from None

    extracted: list[Path] = []
    for original, clean in zip(members, sanitized):
        target = destination / clean.name
        if clean.isdir():
            target.mkdir(parents=True, exist_ok=True)
        else:
            target.parent.mkdir(parents=True, exist_ok=True)
            extracted_file = tar.extractfile(original)
            if extracted_file is None:
                raise AppError(
                    f"could not stream backup member {original.name!r}",
                    code="backup_stream_error",
                    retryable=False,
                )
            with extracted_file as src, open(target, "wb") as dst:
                while True:
                    chunk = src.read(64 * 1024)
                    if not chunk:
                        break
                    dst.write(chunk)
            os.chmod(target, clean.mode or 0o600)
        extracted.append(target)
    return extracted
