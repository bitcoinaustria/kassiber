"""Leftover storage from the removed general ledger (plan 19).

Books created by earlier versions may still contain the ledger's ``gl_*``
tables and the triggers it installed on ``external_documents``. Empty
leftovers are dropped when the database opens. Tables that still hold rows
stay intact: their foreign keys and document triggers keep protecting that
data, so book deletion, reset, and partition fail closed here until the user
runs the explicit purge.
"""

from __future__ import annotations

from typing import Any, Iterable

from ..errors import AppError

PURGE_COMMAND = "kassiber maintenance purge-legacy-ledger"
_PREFIX_SQL = r"name LIKE 'gl\_%' ESCAPE '\'"


def _objects(conn: Any) -> tuple[list[str], list[str]]:
    rows = conn.execute(
        f"SELECT type, name FROM sqlite_master WHERE type IN ('table', 'trigger') AND {_PREFIX_SQL}"
    ).fetchall()
    tables = sorted(str(row[1]) for row in rows if row[0] == "table")
    triggers = sorted(str(row[1]) for row in rows if row[0] == "trigger")
    return tables, triggers


def _quote(name: str) -> str:
    return '"' + name.replace('"', '""') + '"'


def _has_profile_column(conn: Any, table: str) -> bool:
    return any(row[1] == "profile_id" for row in conn.execute(f"PRAGMA table_info({_quote(table)})"))


def populated_tables(conn: Any, *, profile_ids: Iterable[str] | None = None) -> list[str]:
    """Legacy tables with rows, optionally only rows owned by ``profile_ids``."""

    tables, _ = _objects(conn)
    scope = None if profile_ids is None else sorted({str(value) for value in profile_ids})
    if scope == []:
        return []
    found = []
    for table in tables:
        # A table without a book column cannot be scoped, so it counts for all.
        if scope is None or not _has_profile_column(conn, table):
            row = conn.execute(f"SELECT 1 FROM {_quote(table)} LIMIT 1").fetchone()
        else:
            marks = ",".join("?" for _ in scope)
            row = conn.execute(
                f"SELECT 1 FROM {_quote(table)} WHERE profile_id IN ({marks}) LIMIT 1", scope
            ).fetchone()
        if row is not None:
            found.append(table)
    return found


def require_absent(conn: Any, *, profile_ids: Iterable[str] | None, action: str) -> None:
    """Fail closed before a core path would collide with retained ledger rows."""

    if populated_tables(conn, profile_ids=profile_ids):
        raise AppError(
            f"This book holds archived general-ledger data from an earlier Kassiber version, so {action} is blocked.",
            code="legacy_ledger_present",
            hint=(
                "General accounting was removed. Back up the project first, then run "
                f"`{PURGE_COMMAND}` to review and permanently remove the archived ledger data."
            ),
            retryable=False,
        )


def _drop_all(conn: Any, tables: list[str], triggers: list[str]) -> None:
    conn.execute("SAVEPOINT legacy_ledger_drop")
    try:
        # Dropping the triggers first removes the ledger's retention guards,
        # including the ones on external_documents.
        for trigger in triggers:
            conn.execute(f"DROP TRIGGER IF EXISTS {_quote(trigger)}")
        conn.execute("PRAGMA defer_foreign_keys = ON")
        remaining = set(tables)
        while remaining:
            referenced = {
                str(row[2])
                for table in remaining
                for row in conn.execute(f"PRAGMA foreign_key_list({_quote(table)})")
                if row[2] != table
            }
            # Children first; a reference cycle falls back to deferred checks.
            batch = sorted(remaining - referenced) or sorted(remaining)
            for table in batch:
                conn.execute(f"DROP TABLE IF EXISTS {_quote(table)}")
            remaining.difference_update(batch)
    except BaseException:
        conn.execute("ROLLBACK TO SAVEPOINT legacy_ledger_drop")
        conn.execute("RELEASE SAVEPOINT legacy_ledger_drop")
        raise
    conn.execute("RELEASE SAVEPOINT legacy_ledger_drop")


def retire_if_empty(conn: Any) -> bool:
    """Open-time migration: drop leftovers only when no ledger row exists.

    The probe is read-only, so a database without leftovers, or one that
    keeps populated tables, is not written on open.
    """

    tables, triggers = _objects(conn)
    if not tables and not triggers:
        return False
    if populated_tables(conn):
        return False
    _drop_all(conn, tables, triggers)
    return True


def purge_plan(conn: Any) -> dict[str, Any]:
    tables, triggers = _objects(conn)
    counts = {
        table: int(conn.execute(f"SELECT COUNT(*) FROM {_quote(table)}").fetchone()[0])
        for table in tables
    }
    return {
        "tables": [{"table": table, "rows": rows} for table, rows in counts.items() if rows],
        "table_count": len(tables),
        "trigger_count": len(triggers),
        "row_count": sum(counts.values()),
    }


def purge(conn: Any) -> dict[str, Any]:
    """User-confirmed removal of every leftover ledger table and trigger."""

    plan = purge_plan(conn)
    tables, triggers = _objects(conn)
    if tables or triggers:
        _drop_all(conn, tables, triggers)
    conn.commit()
    return {**plan, "applied": True}
