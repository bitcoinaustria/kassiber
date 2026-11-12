"""Reproduce Python's unhandled crash on a lone-surrogate UTXO wallet reference.

`build_wallet_utxos_snapshot` binds the stripped reference as a SQLite
parameter, which cannot encode a lone surrogate, so the daemon answers an
unhandled `internal_error` with a traceback instead of `not_found`. Run with
the locked environment from the repository root; the synthetic book lives in a
temporary directory under `TMPDIR`. Python product code is unchanged; the
native port keeps the same code and message (`tests/test_rust_desktop_utxos.py`).
"""
from pathlib import Path
import tempfile

from kassiber.core.ui_snapshot import build_wallet_utxos_snapshot
from kassiber.db import open_db


def main():
    with tempfile.TemporaryDirectory(prefix="kassiber-utxo-repro-") as root:
        with open_db(Path(root) / "data") as conn:
            reproduce(conn)


def reproduce(conn):
    conn.executescript(
        "INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025');"
        "INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at)"
        " VALUES('p','w','Book','EUR','at','2025');"
        "INSERT INTO settings(key,value) VALUES('context_workspace','w'),('context_profile','p');"
    )
    try:
        build_wallet_utxos_snapshot(conn, {}, {"wallet": "\ud800"})
    except UnicodeEncodeError as error:
        print(f"UnicodeEncodeError: {error}")
        return
    raise AssertionError("the reference no longer crashes; update the native port")


if __name__ == "__main__":
    main()
