"""Same-filesystem restore transaction shared with the Rust store.

Callers close connections and hold both maintenance roles. A durable intent
precedes any live rename. Until its committed replacement is published, restart
rolls back; afterwards restart only finishes cleanup. Recovery is idempotent,
including a second interruption while rolling back. The retained copy is never
removed after commit. The journal contains paths/entry names, never secrets.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import stat
import tempfile
from typing import Callable

from ..errors import AppError

JOURNAL = ".restore-pending.json"
NAMES = ("kassiber.sqlite3", "kassiber.sqlite3-wal", "kassiber.sqlite3-shm",
         "kassiber.sqlite3-journal", "attachments", "config/backends.env",
         "config/settings.json", "exports")
INCOMING = (NAMES[0], *NAMES[4:7])
Observer = Callable[[str], None]


def _noop(_point: str) -> None:
    pass


def restore_targets(data_root: Path) -> dict[str, Path]:
    data_root = Path(data_root).expanduser().resolve()
    state = data_root.parent if data_root.name == "data" else data_root
    return {name: (data_root if name.startswith("kassiber.sqlite3") else state) / name
            for name in NAMES}


def _unsafe() -> AppError:
    return AppError("Restore destination contains a symbolic link", code="unsafe_restore_target")


def _reject_links(path: Path) -> None:
    for part in (path, *path.parents):
        if part.is_symlink():
            raise _unsafe()


def _staging_root(staging: Path) -> Path:
    """Resolve links above staging, such as macOS `/var` -> `/private/var`.

    The system temp root is not Kassiber's to vet; the staging directory
    itself and everything below it still must not be a link.
    """
    staging = Path(staging).expanduser().absolute()
    return staging.parent.resolve() / staging.name


def _sync(path: Path) -> None:
    if os.name != "nt":
        fd = os.open(path, os.O_RDONLY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)


def _tree_sync(path: Path, observe: Observer) -> None:
    for item in sorted(path.rglob("*")):
        _reject_links(item)
        if item.is_file():
            with item.open("rb") as handle:
                os.fsync(handle.fileno())
            observe(f"restore.copy_sync.{item.relative_to(path).as_posix()}")
    for directory in sorted((p for p in path.rglob("*") if p.is_dir()), reverse=True):
        _sync(directory)
        observe(f"restore.copy_dir_sync.{directory.relative_to(path).as_posix()}")
    _sync(path)
    observe("restore.copy_dir_sync.root")


def _publish(state: Path, journal: dict, observe: Observer) -> None:
    with tempfile.NamedTemporaryFile(prefix=".restore-journal-", dir=state, delete=False) as handle:
        temporary = Path(handle.name)
        try:
            handle.write((json.dumps(journal, sort_keys=True) + "\n").encode())
            handle.flush()
            os.fsync(handle.fileno())
            observe(f"restore.{journal['phase']}.journal_sync")
            os.replace(temporary, state / JOURNAL)
            observe(f"restore.{journal['phase']}.journal_replace")
            _sync(state)
            observe(f"restore.{journal['phase']}.journal_dir_sync")
        finally:
            temporary.unlink(missing_ok=True)


def _invalid() -> AppError:
    return AppError("Restore recovery journal is invalid; keep the restore artifacts",
                    code="restore_recovery_required")


def _read(state: Path, data_root: Path) -> dict:
    path = state / JOURNAL
    _reject_links(path)
    info = path.stat()
    if (not stat.S_ISREG(info.st_mode) or info.st_size > 65536
            or (os.name != "nt" and (info.st_uid != os.getuid() or info.st_mode & 0o022))):
        raise _invalid()
    try:
        journal = json.loads(path.read_bytes())
        token = journal["prepared"].removeprefix(".restore-install-")
        if (type(journal["version"]) is not int or journal["version"] != 1 or journal["data_root"] != str(data_root.resolve())
                or not token or not token.isascii() or not all(c.isalnum() or c in "_-" for c in token)
                or journal["prepared"] != ".restore-install-" + token
                or journal["recovery"] not in (None, "pre-restore-" + token)
                or journal["phase"] not in ("pending", "committed")
                or not isinstance(journal["original"], list) or not isinstance(journal["incoming"], list)
                or journal["original"] != [n for n in NAMES if n in journal["original"]]
                or journal["incoming"] != [n for n in INCOMING if n in journal["incoming"]]
                or bool(journal["original"]) != bool(journal["recovery"])):
            raise _invalid()
    except (ValueError, KeyError, TypeError, AttributeError):
        raise _invalid() from None
    for name in (journal["prepared"], journal["recovery"]):
        if name is not None:
            root = state / name
            _reject_links(root)
            for child in root.rglob("*"):
                _reject_links(child)
    return journal


def _remove(path: Path) -> None:
    if path.is_dir():
        shutil.rmtree(path)
    else:
        path.unlink(missing_ok=True)


def _move(source: Path, target: Path, label: str, observe: Observer) -> None:
    target.parent.mkdir(parents=True, exist_ok=True)
    observe(f"restore.before_{label}")
    os.replace(source, target)
    observe(f"restore.after_{label}")
    _sync(source.parent)
    observe(f"restore.source_sync.{label}")
    _sync(target.parent)
    observe(f"restore.target_sync.{label}")


def recover_staged_restore(data_root: Path, *, _observer: Observer = _noop) -> None:
    """Recover under the caller's maintenance leases; never open a database."""
    targets = restore_targets(data_root)
    state = targets["attachments"].parent
    for target in targets.values():
        _reject_links(target)
    if not (state / JOURNAL).exists() and not (state / JOURNAL).is_symlink():
        return
    journal = _read(state, Path(data_root))
    prepared = state / journal["prepared"]
    recovery = state / journal["recovery"] if journal["recovery"] else None
    if journal["phase"] == "pending":
        for name, target in reversed(list(targets.items())):
            saved = recovery / name if recovery else None
            if saved is not None and saved.exists():
                _remove(target)
                _observer(f"restore.rollback_remove.{name}")
                _move(saved, target, f"rollback.{name}", _observer)
            elif name not in journal["original"] and name in journal["incoming"]:
                _remove(target)
                if target.parent.exists():
                    _sync(target.parent)
                _observer(f"restore.rollback_absent.{name}")
            elif name in journal["original"] and not target.exists():
                raise _invalid()
    # Sync every destination before retiring the intent, including replay after
    # an interrupted rollback rename (the saved entry may already be absent).
    for directory in dict.fromkeys(target.parent for target in targets.values()):
        if directory.exists():
            _sync(directory)
    _observer("restore.recovery_dir_sync")
    (state / JOURNAL).unlink()
    _observer("restore.journal_unlink")
    _sync(state)
    _observer("restore.journal_unlink_sync")
    shutil.rmtree(prepared, ignore_errors=True)
    if journal["phase"] == "pending" and recovery is not None:
        shutil.rmtree(recovery)
    _observer("restore.cleanup")


def recover_pending_restore(data_root: Path) -> None:
    """Startup admission: take both roles before recovery or any layout writes."""
    targets = restore_targets(data_root)
    marker = targets["attachments"].parent / JOURNAL
    if marker.exists() or marker.is_symlink():
        from ..operator.project import exclusive_project_maintenance
        with exclusive_project_maintenance(data_root, active_owner_kind=None):
            recover_staged_restore(data_root)


def install_staged_backup(staging: Path, data_root: Path, *, _observer: Observer = _noop) -> Path | None:
    targets = restore_targets(data_root)
    state = targets["attachments"].parent
    for target in targets.values():
        _reject_links(target)
    staging = _staging_root(staging)
    _reject_links(staging)
    recover_staged_restore(data_root, _observer=_observer)
    state.mkdir(parents=True, exist_ok=True)
    prepared = Path(tempfile.mkdtemp(prefix=".restore-install-", dir=state))
    recovery = None
    published = False
    committed = False
    try:
        _observer("restore.prepared")
        incoming = []
        for name in INCOMING:
            source = staging / name
            if not source.exists():
                continue
            destination = prepared / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            _reject_links(source)
            if source.is_dir():
                # Validated tar staging cannot contain links. Refuse changed
                # staging rather than letting copytree follow a new link.
                for child in source.rglob("*"):
                    _reject_links(child)
                shutil.copytree(source, destination)
            else:
                shutil.copy2(source, destination)
            incoming.append(name)
        _tree_sync(prepared, _observer)
        for directory in dict.fromkeys(target.parent for target in targets.values()):
            directory.mkdir(parents=True, exist_ok=True)
            _sync(directory)
        _sync(state)
        _observer("restore.destinations_dir_sync")
        original = [name for name, target in targets.items() if target.exists()]
        if original:
            recovery = state / ("pre-restore-" + prepared.name.removeprefix(".restore-install-"))
            recovery.mkdir(mode=0o700)
            # Parents of every possible recovery target must also be durable.
            (recovery / "config").mkdir(mode=0o700)
            _sync(recovery / "config")
            _sync(recovery)
        _sync(state)
        _observer("restore.artifacts_dir_sync")
        journal = dict(version=1, data_root=str(Path(data_root).resolve()), prepared=prepared.name,
                       recovery=recovery.name if recovery else None, original=original,
                       incoming=incoming, phase="pending")
        _publish(state, journal, _observer)
        published = True
        for name in original:
            _move(targets[name], recovery / name, f"save.{name}", _observer)
        for name in incoming:
            _move(prepared / name, targets[name], f"install.{name}", _observer)
        journal["phase"] = "committed"
        _publish(state, journal, _observer)
        committed = True
        recover_staged_restore(data_root, _observer=_observer)
        return recovery
    except OSError as failure:
        # A failed directory sync may follow a successful journal rename. Read
        # the authoritative phase instead of relying on in-memory progress.
        if (state / JOURNAL).exists():
            try:
                committed = _read(state, Path(data_root))["phase"] == "committed"
                recover_staged_restore(data_root)
            except Exception as rollback_failure:
                raise AppError("Restore failed and automatic recovery could not finish",
                               code="restore_rollback_failed", details={"recovery_path": str(recovery)},
                               hint=f"Keep the recovery copy at {recovery}; restore it before reopening this container.") from rollback_failure
        if committed:
            return recovery
        raise AppError("Restore could not be installed; the original data was restored" if published else
                       "Restore preparation failed; the original data is unchanged",
                       code="restore_install_failed",
                       hint="Check destination permissions and available space, then preview again.") from failure
    finally:
        # Never destroy a live intent's prepared tree during an interrupted or
        # failed recovery. Before intent, these are private disposable copies.
        if not (state / JOURNAL).exists():
            shutil.rmtree(prepared, ignore_errors=True)
            if not committed and recovery is not None:
                shutil.rmtree(recovery, ignore_errors=True)
