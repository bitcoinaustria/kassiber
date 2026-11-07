"""Check: a list or object `sync` is a typed `ui.maintenance.run` refusal.

`_maintenance_run_payload` tested `sync_mode not in {"never", "if_enabled",
"always"}` before its typed validation, so an unhashable value raised a bare
TypeError and the daemon answered `internal_error` with a traceback on
stderr. It now checks the type first and refuses with `validation`, as Rust
does. Run with TMPDIR under the worktree's target/.
"""
from pathlib import Path
import tempfile

from kassiber.daemon_freshness import _maintenance_run_payload
from kassiber.db import open_db
from kassiber.errors import AppError


def main():
    with tempfile.TemporaryDirectory(prefix="maintenance-sync-repro-") as directory:
        conn = open_db(Path(directory) / "data")
        try:
            for value in ([], {}):
                try:
                    _maintenance_run_payload(conn, {}, {"sync": value}, state={})
                except AppError as error:
                    assert error.code == "validation", error.code
                    print(f"fixed: {value!r} refused with {error.code}")
                else:
                    raise SystemExit("expected a validation refusal")
        finally:
            conn.close()


if __name__ == "__main__":
    main()
