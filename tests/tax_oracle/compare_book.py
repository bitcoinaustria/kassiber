"""Compare production with the frozen oracle on a copy of a real book (plan 20).

    uv run --locked python -m tests.tax_oracle.compare_book --data-root <copy> \\
        [--db-passphrase-fd <fd>] [--profile <id or label>]

For each profile of the book, or the one ``--profile`` names, this builds the
finalized custody projection the way journal processing does
(``CustodyJournalBuilder.build_custody_projection``), runs production and the
oracle on it, and prints every difference and result to stdout; refusals and
errors go to stderr. It stores nothing (the connection is read-only once
opened), writes no file outside the copy, and sends nothing anywhere: no
network, no keyring, no remembered unlock, no operator broker. The exit
status is 0 when every profile compared equal, 1 otherwise, and 2 when it
refused to run.

``<copy>`` must be a disposable copy the owner made, because opening it may
migrate its schema. It refuses the default data root, the legacy data roots,
anything inside the default state root or a project registered there, a
directory without a Kassiber database, and a database (or SQLite side file)
that is a symbolic or hard link, which could reach a live book. An encrypted
copy reads its passphrase
from ``--db-passphrase-fd`` only, as ``kassiber --db-passphrase-fd`` does.
"""

from __future__ import annotations

import argparse
import sqlite3
import sys
from pathlib import Path
from typing import Any, Iterable, Optional, Sequence

from kassiber import db as kassiber_db
from kassiber.core.custody_journal import CustodyJournalBuilder
from kassiber.core.engines.base import TaxEngineLedgerInputs
from kassiber.errors import AppError
from kassiber.projects import catalog_path, load_catalog
from kassiber.secrets.prompt import read_passphrase_from_fd
from tests import tax_engine_compare as compare

EXIT_EQUAL = 0
EXIT_DIFFERENT = 1
EXIT_REFUSED = 2
_SQLITE_SIDE_FILES = ("-wal", "-shm", "-journal")


def _database_errors() -> tuple[type[BaseException], ...]:
    # An encrypted copy runs on SQLCipher's driver, whose errors are not
    # sqlite3's.
    errors: list[type[BaseException]] = [sqlite3.DatabaseError]
    try:
        import sqlcipher3
    except ImportError:
        return tuple(errors)
    return (*errors, sqlcipher3.DatabaseError)


def _same_or_inside(path: Path, roots: Iterable[Path]) -> Optional[Path]:
    for root in roots:
        resolved = root.expanduser().resolve()
        if path == resolved or resolved in path.parents:
            return resolved
    return None


def _registered_project_roots(state_roots: Sequence[Path]) -> list[Path]:
    """The project directories the default state roots' catalogs name (read only)."""
    roots = []
    for state_root in state_roots:
        for entry in load_catalog(catalog_path(state_root)).get("projects", ()):
            roots.append(Path(entry["path"]))
    return roots


def refusal(data_root: Path) -> Optional[str]:
    """Why ``data_root`` is not a disposable copy this tool may open, if it is not."""
    copy = data_root.expanduser().resolve()
    # Computed now, not at import, so the environment the tool runs in decides.
    state_roots = [Path(kassiber_db.default_state_root()), Path(kassiber_db.DEFAULT_STATE_ROOT)]
    default_roots = [root / kassiber_db.DEFAULT_DATA_DIRNAME for root in state_roots]
    default_roots += [Path(kassiber_db.resolve_effective_data_root(root)) for root in default_roots]
    default_roots += [Path(kassiber_db.LEGACY_XDG_DATA_ROOT), Path(kassiber_db.LEGACY_DATA_ROOT)]
    if _same_or_inside(copy, default_roots) is not None:
        return f"{copy} is Kassiber's default data root; run on a disposable copy of the book"
    inside = _same_or_inside(copy, state_roots)
    if inside is not None:
        return f"{copy} lies inside Kassiber's default state root {inside}; copy the book elsewhere first"
    project = _same_or_inside(copy, _registered_project_roots(state_roots))
    if project is not None:
        return f"{copy} belongs to the registered project at {project}; copy the book elsewhere first"
    if not copy.is_dir():
        return f"{copy} is not a directory"
    database = kassiber_db.resolve_database_path(copy)
    if not database.is_file() or database.stat().st_size == 0:
        return f"{copy} contains no Kassiber database ({kassiber_db.DEFAULT_DB_FILENAME})"
    # A linked database (or SQLite side file, which is opened by name) would
    # let the checks above pass for the copy while the live file is opened.
    for path in (database, *(database.with_name(database.name + suffix) for suffix in _SQLITE_SIDE_FILES)):
        if path.is_symlink():
            return f"{path} is a symbolic link; copy the book's files instead of linking them"
        if path.exists() and path.stat().st_nlink != 1:
            return f"{path} has other hard links; copy the book's files instead of linking them"
    return None


def _profiles(conn: Any, ref: Optional[str]) -> list[Any]:
    rows = conn.execute("SELECT * FROM profiles ORDER BY workspace_id ASC, label ASC, id ASC").fetchall()
    if ref is None:
        return list(rows)
    matches = [row for row in rows if row["id"] == ref] or [
        row for row in rows if str(row["label"]).lower() == ref.strip().lower()
    ]
    if len(matches) != 1:
        found = ", ".join(row["id"] for row in matches) or "none"
        raise AppError(f"--profile {ref!r} must name exactly one profile (matches: {found})", code="validation")
    return matches


def _compare_profile(conn: Any, profile: Any) -> bool:
    """Print one profile's comparison; whether production equals the oracle."""
    name = f"profile {profile['label']!r} ({profile['id']})"
    try:
        custody = CustodyJournalBuilder(conn, profile).build_custody_projection()
    except (AppError, *_database_errors()) as exc:
        # A write attempt on the read-only connection lands here too.
        print(f"{name}: not compared, its custody projection does not build: {exc}")
        return False
    inputs = TaxEngineLedgerInputs(
        finalized_tax_projection=custody.finalized_tax_projection,
        wallet_refs_by_id=custody.wallet_refs_by_id,
        direct_payout_records=custody.direct_payout_records,
    )
    projection = inputs.finalized_tax_projection
    oracle = compare.run_oracle(profile, inputs)
    production = compare.run_production(profile, inputs)
    differences = compare.ledger_differences(oracle, production)
    shape = f"{len(projection.rows)} projected rows, {len(projection.intra_pairs)} moves, {len(projection.cross_asset_pairs)} cross-asset pairs"
    if not differences:
        error = oracle[1]
        outcome = f"both fail: {type(error).__name__}: {error}" if error is not None else f"{len(oracle[0].entries)} entries"
        print(f"{name}: {shape}: production equals the oracle ({outcome})")
        return True
    print(f"{name}: {shape}: production differs from the oracle:")
    for difference in differences:
        print(f"  {difference}")
    return False


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(
        prog="python -m tests.tax_oracle.compare_book",
        description="Compare production's tax ledger with the frozen oracle on a disposable copy of a book.",
    )
    parser.add_argument("--data-root", required=True, type=Path, help="a disposable copy of a Kassiber data directory")
    parser.add_argument("--db-passphrase-fd", type=int, help="read an encrypted copy's passphrase from this file descriptor")
    parser.add_argument("--profile", help="compare only this profile (id or label)")
    args = parser.parse_args(argv)

    reason = refusal(args.data_root)
    if reason is not None:
        print(f"refusing to run: {reason}", file=sys.stderr)
        return EXIT_REFUSED
    copy = args.data_root.expanduser().resolve()
    try:
        passphrase = None if args.db_passphrase_fd is None else read_passphrase_from_fd(args.db_passphrase_fd)
        conn = kassiber_db.open_db(copy, passphrase=passphrase, require_existing_schema=True)
    except AppError as exc:
        print(f"cannot open {copy}: {exc}", file=sys.stderr)
        return EXIT_REFUSED
    try:
        # open_db may have migrated the copy; from here on nothing is written.
        conn.execute("PRAGMA query_only = ON")
        profiles = _profiles(conn, args.profile)
        if not profiles:
            print("the book has no profiles")
            return EXIT_EQUAL
        equal = [_compare_profile(conn, profile) for profile in profiles]
    except AppError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return EXIT_REFUSED
    finally:
        conn.close()
    return EXIT_EQUAL if all(equal) else EXIT_DIFFERENT


if __name__ == "__main__":
    sys.exit(main())
