"""Remove the owner-lock records a test run left behind, the product's way.

The namespace is per account, so other test runs, a running desktop and the
CLI share it. A group is swept only under its admission lock, exactly as
`_prune_owner_lock_key` does: an acquirer holds that lock from opening a
record until it has locked it, so no record is unlinked under a process that
is about to lock it. A group whose admission is busy is skipped, a record
someone holds stays, and the `.admission` file itself is never removed, since
an acquirer may already have it open. What makes this a session sweep rather
than the product's week-old prune is only the selection: groups touched since
the session started.
"""

from __future__ import annotations

from pathlib import Path

from kassiber.operator import project as project_module

ADMISSION_SUFFIX = ".admission"


def sweep_owner_locks(root: Path, started: float) -> list[Path]:
    """Unlink unheld records of groups touched since `started`; return them."""

    groups: dict[Path, list[Path]] = {}
    for path in root.glob("*.lock*"):
        key = root / f"{path.name.split('.lock', 1)[0]}.lock"
        groups.setdefault(key, []).append(path)
    removed: list[Path] = []
    for key, paths in sorted(groups.items()):
        try:
            if all(path.stat().st_mtime < started for path in paths):
                continue
        except OSError:
            continue
        removed.extend(_sweep_group(key, paths))
    return removed


def _sweep_group(key: Path, paths: list[Path]) -> list[Path]:
    try:
        admission = project_module._open_owner_lock(
            project_module._owner_admission_lock_path(key), "sweep"
        )
    except Exception:
        return []
    removed = []
    try:
        if not project_module._try_lock_handle(admission):
            return []
        for path in sorted(paths):
            if path.name.endswith(ADMISSION_SUFFIX):
                continue
            try:
                handle = project_module._open_owner_lock(path, "sweep")
            except Exception:
                continue
            dead = False
            try:
                dead = project_module._try_lock_handle(handle)
                if dead:
                    project_module._unlock_handle(handle)
            finally:
                handle.close()
            if dead:
                try:
                    path.unlink(missing_ok=True)
                except OSError:
                    # One file that cannot be removed must not abort the sweep.
                    continue
                removed.append(path)
    finally:
        project_module._unlock_handle(admission)
        admission.close()
    return removed
