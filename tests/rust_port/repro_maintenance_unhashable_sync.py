"""Disposable repro: a list or object `sync` escapes `ui.maintenance.run`.

`_maintenance_run_payload` tests `sync_mode not in {"never", "if_enabled",
"always"}` before its typed validation, so an unhashable value raises a bare
TypeError and the daemon answers `internal_error` with a traceback on stderr.
Rust returns the same envelope without the traceback. Python product code is
unchanged. Run with TMPDIR under the worktree's target/.
"""
from pathlib import Path
import tempfile

from kassiber.daemon_freshness import _maintenance_run_payload
from kassiber.db import open_db


def main():
    with tempfile.TemporaryDirectory(prefix="maintenance-sync-repro-") as directory:
        conn = open_db(Path(directory) / "data")
        try:
            for value in ([], {}):
                try:
                    _maintenance_run_payload(conn, {}, {"sync": value}, state={})
                except TypeError as error:
                    print(f"Python reference bug reproduced: {type(error).__name__}: {error}")
                else:
                    raise SystemExit("expected an unhandled TypeError")
        finally:
            conn.close()


if __name__ == "__main__":
    main()
