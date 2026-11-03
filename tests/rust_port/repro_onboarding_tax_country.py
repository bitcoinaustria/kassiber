"""Disposable repro: onboarding with an unknown tax country is unhandled.

`ui.onboarding.complete` passes `tax_country` to `create_profile`, whose
`normalize_tax_country` raises a bare ValueError before the typed
`validation` check, so the daemon answers `internal_error` and prints a
traceback. Rust returns the same envelope without the traceback. Python
product code is unchanged. Run with TMPDIR under the worktree's target/.
"""
from pathlib import Path
import tempfile

from kassiber.core import accounts
from kassiber.db import open_db


def main():
    with tempfile.TemporaryDirectory(prefix="onboarding-country-repro-") as directory:
        conn = open_db(Path(directory) / "data")
        workspace = accounts.create_workspace(conn, "Main")
        try:
            accounts.create_profile(conn, workspace["id"], "Book", "EUR", None, "xx", 365)
        except ValueError as error:
            print(f"Python reference bug reproduced: {type(error).__name__}: {error}")
        else:
            raise SystemExit("expected an unhandled ValueError")
        finally:
            conn.close()


if __name__ == "__main__":
    main()
