"""Directory trees as content-addressed snapshots, captured and restored unchanged."""

from __future__ import annotations

import hashlib
import os
import stat
from collections.abc import Iterable
from pathlib import Path


_CHUNK = 1 << 20


class BlobStore:
    """File contents by sha256, written once with owner-only permissions."""

    def __init__(self, directory: Path):
        self.directory = Path(directory)

    def path(self, digest: str) -> Path:
        return self.directory / digest

    def has(self, digest: str) -> bool:
        return self.path(digest).is_file()

    def put_bytes(self, data: bytes) -> str:
        digest = hashlib.sha256(data).hexdigest()
        if not self.has(digest):
            self._write(digest, data)
        return digest

    def put_file(self, source: Path) -> tuple[str, int]:
        data = Path(source).read_bytes()
        return self.put_bytes(data), len(data)

    def get(self, digest: str) -> bytes:
        return self.path(digest).read_bytes()

    def delete(self, digest: str) -> None:
        try:
            self.path(digest).unlink()
        except FileNotFoundError:
            pass

    def _write(self, digest: str, data: bytes) -> None:
        self.directory.mkdir(parents=True, exist_ok=True)
        partial = self.directory / f".{digest}.partial"
        fd = os.open(partial, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, "wb") as handle:
            handle.write(data)
        os.replace(partial, self.path(digest))


def file_digest(path: Path) -> tuple[str, int]:
    digest = hashlib.sha256()
    size = 0
    with open(path, "rb") as handle:
        while chunk := handle.read(_CHUNK):
            digest.update(chunk)
            size += len(chunk)
    return digest.hexdigest(), size


def capture_tree(root: Path, store: BlobStore | None = None, *, exclude: Iterable[Path] = ()) -> dict:
    """The tree under `root`: every directory, file, and symbolic link.

    Links are recorded, never followed. With a store, file contents go into
    it; without one only their digests are taken, which is enough to notice
    a change.
    """
    root = Path(root)
    excluded = {Path(path) for path in exclude}
    try:
        root_stat = os.lstat(root)
    except FileNotFoundError:
        return {"exists": False}
    if stat.S_ISLNK(root_stat.st_mode) or not stat.S_ISDIR(root_stat.st_mode):
        return {"exists": True, "kind": "not_a_directory"}
    entries: dict[str, dict] = {}
    for current, dirnames, filenames in os.walk(root, followlinks=False):
        current_path = Path(current)
        dirnames[:] = sorted(name for name in dirnames if current_path / name not in excluded)
        for name in dirnames:
            path = current_path / name
            info = os.lstat(path)
            rel = path.relative_to(root).as_posix()
            if stat.S_ISLNK(info.st_mode):
                entries[rel] = {"type": "symlink", "target": os.readlink(path)}
            else:
                entries[rel] = {"type": "dir", "mode": stat.S_IMODE(info.st_mode)}
        for name in sorted(filenames):
            path = current_path / name
            if path in excluded:
                continue
            info = os.lstat(path)
            rel = path.relative_to(root).as_posix()
            if stat.S_ISLNK(info.st_mode):
                entries[rel] = {"type": "symlink", "target": os.readlink(path)}
            elif stat.S_ISREG(info.st_mode):
                if store is None:
                    digest, size = file_digest(path)
                else:
                    digest, size = store.put_file(path)
                entries[rel] = {
                    "type": "file",
                    "mode": stat.S_IMODE(info.st_mode),
                    "size": size,
                    "sha256": digest,
                }
            else:
                entries[rel] = {"type": "special"}
    return {"exists": True, "mode": stat.S_IMODE(root_stat.st_mode), "entries": dict(sorted(entries.items()))}


def tree_blobs(state: dict | None) -> set[str]:
    if not state or not state.get("entries"):
        return set()
    return {entry["sha256"] for entry in state["entries"].values() if entry.get("type") == "file"}


def restore_tree(state: dict, store: BlobStore, destination: Path) -> None:
    """Recreate a captured tree at `destination`, which must be absent or an empty directory."""
    destination = Path(destination)
    if not state.get("exists"):
        if destination.is_dir() and not any(destination.iterdir()):
            destination.rmdir()
        return
    if state.get("kind") == "not_a_directory":
        raise ValueError(f"captured root was not a directory: {destination}")
    if destination.is_dir():
        if any(destination.iterdir()):
            raise FileExistsError(destination)
    else:
        destination.mkdir(parents=True)
    entries = state["entries"]
    for rel, entry in entries.items():
        path = destination / rel
        kind = entry["type"]
        if kind == "dir":
            path.mkdir(parents=True, exist_ok=True)
        elif kind == "file":
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(store.get(entry["sha256"]))
            os.chmod(path, entry["mode"])
        elif kind == "symlink":
            path.parent.mkdir(parents=True, exist_ok=True)
            os.symlink(entry["target"], path)
        else:
            raise ValueError(f"cannot restore a special file: {rel}")
    # Directory modes last, so a read-only directory can still be filled.
    for rel, entry in sorted(entries.items(), reverse=True):
        if entry["type"] == "dir":
            os.chmod(destination / rel, entry["mode"])
    os.chmod(destination, state["mode"])
