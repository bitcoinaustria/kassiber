from __future__ import annotations

"""Per-import-run provenance, so one bad file import can be rolled back.

Motivating case: a user onboards an exchange with no predefined importer, the
column plan is wrong (or the export was the wrong file), and the book now holds
garbage rows. Deleting the whole wallet is too blunt when other imports landed in
it, and deleting rows one by one is not a real option.

Two deliberate scoping rules:

1. **Only rows the batch created are linked.** `insert_wallet_records` already
   separates `inserted_records` from `updated_records`; an enrichment update
   touched a transaction that existed *before* this run, so rolling back must not
   delete it. That means a rollback can never remove data an import merely
   annotated.
2. **A rollback is a delete, not an inverse-patch.** A transaction inserted by
   batch A and later enriched by batch B is removed when A is rolled back — the
   row would not exist without A. Everything referencing those transactions
   (tags, attachments, pairs, custody components, journal entries) is removed by
   the schema's `ON DELETE CASCADE`, which is why `plan_rollback` reports the
   counts before anything is confirmed.
"""

import json
import sqlite3
import uuid
from typing import Any, Mapping

from ..errors import AppError
from ..time_utils import now_iso


MAX_LISTED_BATCHES = 200
# The run's transaction ids are staged in a temp table rather than bound as
# parameters. A multi-year exchange export exceeds SQLite's parameter cap
# (32766), and chunking around that would break the collateral count: a row
# referencing two of the run's transactions in different chunks would be counted
# once per chunk. One staged set is both exact and unbounded.
_STAGED_IDS = "_import_rollback_ids"
# Cascade discovery walks the FK graph, so a cycle or a pathological schema
# cannot spin forever. Nothing in the schema is near this.
_MAX_CASCADE_DEPTH = 6


def record_batch(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    *,
    wallet_id: str | None,
    source_format: str,
    source_filename: str | None,
    column_map: Mapping[str, Any] | None,
    outcome: Mapping[str, Any],
) -> str | None:
    """Link the transactions an import run created to a new batch row.

    Returns the batch id, or None when the run created nothing (an import that
    only enriched existing rows leaves no batch to roll back).
    """
    inserted = [
        str(record["transaction_id"])
        for record in outcome.get("inserted_records") or []
        if record.get("transaction_id")
    ]
    if not inserted:
        return None

    batch_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO import_batches(
            id, workspace_id, profile_id, wallet_id, source_format,
            source_filename, column_map_json, imported_at,
            rows_inserted, rows_updated, rows_skipped
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            batch_id,
            profile["workspace_id"],
            profile["id"],
            wallet_id,
            source_format,
            source_filename,
            json.dumps(dict(column_map), sort_keys=True) if column_map else None,
            now_iso(),
            len(inserted),
            int(outcome.get("updated") or 0),
            int(outcome.get("skipped") or 0),
        ),
    )
    conn.executemany(
        "INSERT OR IGNORE INTO import_batch_transactions(batch_id, transaction_id) VALUES(?, ?)",
        [(batch_id, tx_id) for tx_id in inserted],
    )
    return batch_id


def _batch_row(conn: sqlite3.Connection, profile_id: str, batch_ref: str) -> sqlite3.Row:
    row = conn.execute(
        "SELECT * FROM import_batches WHERE profile_id = ? AND id = ?",
        (profile_id, batch_ref),
    ).fetchone()
    if row is None:
        raise AppError(
            f"Import batch not found: {batch_ref}",
            code="not_found",
            hint="Run `imports list` to see this book's import runs.",
            retryable=False,
        )
    return row


def _batch_payload(row: sqlite3.Row, *, remaining: int) -> dict[str, Any]:
    column_map = None
    if row["column_map_json"]:
        try:
            column_map = json.loads(row["column_map_json"])
        except (TypeError, ValueError):
            column_map = None
    return {
        "id": row["id"],
        "wallet_id": row["wallet_id"],
        "source_format": row["source_format"],
        "source_filename": row["source_filename"],
        "column_map": column_map,
        "imported_at": row["imported_at"],
        "rows_inserted": int(row["rows_inserted"] or 0),
        "rows_updated": int(row["rows_updated"] or 0),
        "rows_skipped": int(row["rows_skipped"] or 0),
        # What is still there now. Diverges from rows_inserted once rows are
        # deleted by other means (wallet delete, an earlier partial cleanup), so
        # a rollback never claims it will remove more than it can.
        "rows_present": remaining,
        "rolled_back": remaining == 0,
    }


def list_batches(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    wallet_id: str | None = None,
    limit: int = 50,
) -> list[dict[str, Any]]:
    bound = max(1, min(int(limit or 50), MAX_LISTED_BATCHES))
    params: list[Any] = [profile_id]
    where = "b.profile_id = ?"
    if wallet_id:
        where += " AND b.wallet_id = ?"
        params.append(wallet_id)
    rows = conn.execute(
        f"""
        SELECT b.*, (
            SELECT COUNT(*)
            FROM import_batch_transactions bt
            WHERE bt.batch_id = b.id
        ) AS rows_present
        FROM import_batches b
        WHERE {where}
        ORDER BY b.imported_at DESC, b.id DESC
        LIMIT ?
        """,
        (*params, bound),
    ).fetchall()
    return [
        _batch_payload(row, remaining=int(row["rows_present"] or 0)) for row in rows
    ]


def _stage_batch_ids(conn: sqlite3.Connection, batch_id: str) -> int:
    """Stage this run's transaction ids in a temp table. Returns how many."""
    conn.execute(f"DROP TABLE IF EXISTS temp.{_STAGED_IDS}")
    conn.execute(f"CREATE TEMP TABLE {_STAGED_IDS}(id TEXT PRIMARY KEY)")
    conn.execute(
        f"INSERT OR IGNORE INTO temp.{_STAGED_IDS}(id) "
        "SELECT transaction_id FROM import_batch_transactions WHERE batch_id = ?",
        (batch_id,),
    )
    return int(
        conn.execute(f"SELECT COUNT(*) AS n FROM temp.{_STAGED_IDS}").fetchone()["n"]
    )


def _primary_key(conn: sqlite3.Connection, table: str) -> str | None:
    keys = [
        str(row["name"])
        for row in conn.execute(f'PRAGMA table_info("{table}")').fetchall()
        if int(row["pk"] or 0)
    ]
    return keys[0] if len(keys) == 1 else None


def _cascade_predicates(conn: sqlite3.Connection) -> list[tuple[str, str]]:
    """Every (table, predicate) whose matching rows die with the staged ids.

    Discovered from the schema with `PRAGMA foreign_key_list` rather than a
    hand-written list: 17 tables reference `transactions` directly, several
    holding reviewed decisions and authored evidence, and a maintained list would
    drift silently — under-reporting exactly the user work a rollback destroys.

    The walk continues past those tables, because a cascade is transitive: an
    attachment linked to a source-of-funds link dies when the link dies when the
    transaction dies. Each table gets ONE predicate OR-ing all its paths, so a
    row reachable twice (`transaction_pairs` names two transactions, and a run
    usually creates both sides of a pair) counts once, not twice.
    """
    tables = [
        str(row["name"])
        for row in conn.execute(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'"
        ).fetchall()
    ]
    # child table -> [(child column, parent table, parent column)]
    edges: dict[str, list[tuple[str, str, str]]] = {}
    for table in tables:
        for fk in conn.execute(f'PRAGMA foreign_key_list("{table}")').fetchall():
            if str(fk["on_delete"]).upper() != "CASCADE":
                continue  # SET NULL keeps the row; it is not lost work
            parent = str(fk["table"])
            parent_key = fk["to"] or _primary_key(conn, parent)
            if not parent_key:
                continue
            edges.setdefault(table, []).append(
                (str(fk["from"]), parent, str(parent_key))
            )

    resolved: dict[str, str] = {
        "transactions": f"id IN (SELECT id FROM temp.{_STAGED_IDS})"
    }
    # Fixpoint: a row dies if *any* cascade parent row dies, so each round
    # rebuilds every table's predicate from whatever is resolved so far. A table
    # whose parent gains a path in a later round is rebuilt with it; a parent
    # that is never reachable from `transactions` simply contributes nothing.
    for _ in range(_MAX_CASCADE_DEPTH):
        changed = False
        for table, links in edges.items():
            # The run's own link rows are bookkeeping for the thing being
            # deleted, not collateral: reporting "also removed: 3
            # import_batch_transactions" to a user weighing a rollback is noise.
            if table in {"transactions", "import_batch_transactions"}:
                continue
            usable = [link for link in links if link[1] in resolved and link[1] != table]
            if not usable:
                continue
            predicate = " OR ".join(
                f'"{column}" IN (SELECT "{parent_key}" FROM "{parent}" WHERE {resolved[parent]})'
                for column, parent, parent_key in usable
            )
            if resolved.get(table) != predicate:
                resolved[table] = predicate
                changed = True
        if not changed:
            break
    return [(table, predicate) for table, predicate in resolved.items() if table != "transactions"]


def _collateral_counts(conn: sqlite3.Connection, staged: int) -> dict[str, int]:
    """Count rows that cascade away with the staged transactions, by table.

    Surfaced so a rollback is an informed decision: much of this is user work
    (reviewed pairs, custody components, source-of-funds links, attachments,
    tags, metadata history), not import output. One COUNT per reachable table
    over the staged id set, so each row is counted once however many of the
    run's transactions it references.
    """
    if not staged:
        return {}
    counts: dict[str, int] = {}
    for table, predicate in _cascade_predicates(conn):
        row = conn.execute(
            f'SELECT COUNT(*) AS n FROM "{table}" WHERE {predicate}'  # noqa: S608 - identifiers from schema
        ).fetchone()
        if row and int(row["n"] or 0):
            counts[table] = int(row["n"])
    return counts


def _drop_staged(conn: sqlite3.Connection) -> None:
    conn.execute(f"DROP TABLE IF EXISTS temp.{_STAGED_IDS}")


def plan_rollback(
    conn: sqlite3.Connection,
    profile_id: str,
    batch_ref: str,
) -> dict[str, Any]:
    """Report exactly what rolling this batch back would remove. Pure."""
    row = _batch_row(conn, profile_id, batch_ref)
    try:
        staged = _stage_batch_ids(conn, row["id"])
        also_removed = _collateral_counts(conn, staged)
    finally:
        _drop_staged(conn)
    return {
        "batch": _batch_payload(row, remaining=staged),
        "transactions_to_delete": staged,
        "also_removed": also_removed,
        "journals_invalidated": bool(staged),
    }


def rollback_batch(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    batch_ref: str,
    *,
    invalidate_journals,
    commit: bool = True,
) -> dict[str, Any]:
    """Delete every transaction this import run created, then drop the batch.

    Callers must confirm first — this removes real rows, and anything attached to
    them goes too (see `plan_rollback`).
    """
    row = _batch_row(conn, profile["id"], batch_ref)
    try:
        staged = _stage_batch_ids(conn, row["id"])
        also_removed = _collateral_counts(conn, staged)
        conn.execute(
            f"DELETE FROM transactions WHERE id IN (SELECT id FROM temp.{_STAGED_IDS})"
        )
    finally:
        _drop_staged(conn)
    conn.execute("DELETE FROM import_batches WHERE id = ?", (row["id"],))
    if staged:
        invalidate_journals(conn, profile["id"])
    if commit:
        conn.commit()
    return {
        "id": row["id"],
        "source_format": row["source_format"],
        "source_filename": row["source_filename"],
        "wallet_id": row["wallet_id"],
        "rolled_back": True,
        "transactions_deleted": staged,
        "also_removed": also_removed,
        "journals_invalidated": bool(staged),
    }


def observed_assets(conn: sqlite3.Connection, profile_id: str) -> dict[str, list[str]]:
    """Assets actually present per wallet, for callers that must not guess.

    Wallet config states an intended chain at setup time; a file import states
    nothing. The rows themselves are the only honest answer to "what is in this
    connection", which is what the UI's asset icon needs.
    """
    rows = conn.execute(
        """
        SELECT wallet_id, asset, COUNT(*) AS n
        FROM transactions
        WHERE profile_id = ? AND wallet_id IS NOT NULL AND asset IS NOT NULL AND asset != ''
        GROUP BY wallet_id, asset
        ORDER BY wallet_id, n DESC, asset ASC
        """,
        (profile_id,),
    ).fetchall()
    grouped: dict[str, list[str]] = {}
    for row in rows:
        grouped.setdefault(str(row["wallet_id"]), []).append(
            str(row["asset"]).upper()
        )
    return grouped


def demo() -> None:
    """Self-check: rollback must remove created rows and spare enriched ones."""
    # `open_db` needs a real data root, and the wired path is covered by the CLI
    # and daemon tests. Here we assert the link/scoping logic against a minimal
    # hand-built schema that mirrors the real cascades.
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    conn.executescript(
        """
        PRAGMA foreign_keys = ON;
        CREATE TABLE workspaces(id TEXT PRIMARY KEY);
        CREATE TABLE profiles(id TEXT PRIMARY KEY);
        CREATE TABLE wallets(id TEXT PRIMARY KEY);
        CREATE TABLE transactions(
            id TEXT PRIMARY KEY,
            profile_id TEXT,
            wallet_id TEXT REFERENCES wallets(id) ON DELETE CASCADE,
            asset TEXT
        );
        CREATE TABLE import_batches(
            id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
            wallet_id TEXT REFERENCES wallets(id) ON DELETE CASCADE,
            source_format TEXT NOT NULL,
            source_filename TEXT,
            column_map_json TEXT,
            imported_at TEXT NOT NULL,
            rows_inserted INTEGER NOT NULL DEFAULT 0,
            rows_updated INTEGER NOT NULL DEFAULT 0,
            rows_skipped INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE import_batch_transactions(
            batch_id TEXT NOT NULL REFERENCES import_batches(id) ON DELETE CASCADE,
            transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
            PRIMARY KEY (batch_id, transaction_id)
        );
        INSERT INTO workspaces VALUES('w');
        INSERT INTO profiles VALUES('p');
        INSERT INTO wallets VALUES('wal');
        -- 'old' predates the import; 'new1'/'new2' are created by it.
        INSERT INTO transactions VALUES('old','p','wal','BTC');
        INSERT INTO transactions VALUES('new1','p','wal','LBTC');
        INSERT INTO transactions VALUES('new2','p','wal','LBTC');
        -- Authored work that cascades with a deleted transaction. Discovery is
        -- schema-driven, so this proves the PRAGMA walk finds it.
        CREATE TABLE transaction_tags(
            transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
            tag TEXT
        );
        -- SET NULL keeps the row, so it must NOT be counted as lost work.
        CREATE TABLE loose_notes(
            transaction_id TEXT REFERENCES transactions(id) ON DELETE SET NULL,
            note TEXT
        );
        -- Names TWO transactions, like transaction_pairs / source_funds_links.
        -- A run usually creates both sides, and the row dies once.
        CREATE TABLE transaction_pairs(
            id TEXT PRIMARY KEY,
            in_transaction_id TEXT REFERENCES transactions(id) ON DELETE CASCADE,
            out_transaction_id TEXT REFERENCES transactions(id) ON DELETE CASCADE
        );
        -- Cascades off the pair, not off the transaction: a transitive death
        -- the walk has to follow or the plan under-reports authored evidence.
        CREATE TABLE pair_attachments(
            pair_id TEXT NOT NULL REFERENCES transaction_pairs(id) ON DELETE CASCADE,
            filename TEXT
        );
        INSERT INTO transaction_tags VALUES('new1','reviewed');
        INSERT INTO transaction_tags VALUES('new2','reviewed');
        INSERT INTO loose_notes VALUES('new1','keep me');
        INSERT INTO transaction_pairs VALUES('pair1','new1','new2');
        INSERT INTO pair_attachments VALUES('pair1','receipt.pdf');
        """
    )
    profile = {"id": "p", "workspace_id": "w"}

    batch_id = record_batch(
        conn,
        profile,
        wallet_id="wal",
        source_format="generic_ledger",
        source_filename="xyz.csv",
        column_map={"date": "Ausfuehrung", "amount": "Stueck"},
        outcome={
            "inserted_records": [
                {"transaction_id": "new1"},
                {"transaction_id": "new2"},
            ],
            # An enrichment update to a pre-existing row must not be linked.
            "updated_records": [{"transaction_id": "old"}],
            "updated": 1,
            "skipped": 1,
        },
    )
    assert batch_id, "a run that inserted rows must produce a batch"

    listed = list_batches(conn, "p")
    assert len(listed) == 1, listed
    assert listed[0]["rows_inserted"] == 2 and listed[0]["rows_present"] == 2
    assert listed[0]["column_map"]["date"] == "Ausfuehrung"
    assert listed[0]["rolled_back"] is False

    plan = plan_rollback(conn, "p", batch_id)
    assert plan["transactions_to_delete"] == 2, plan
    # Cascading user work is reported, discovered from the schema itself...
    assert plan["also_removed"].get("transaction_tags") == 2, plan
    # ...while a SET NULL reference is not "removed" and must not be listed.
    assert "loose_notes" not in plan["also_removed"], plan
    # A row naming two of the run's transactions dies once, so it counts once.
    assert plan["also_removed"].get("transaction_pairs") == 1, plan
    # ...and what dies with *that* row is disclosed too, or the plan claims less
    # than the rollback destroys.
    assert plan["also_removed"].get("pair_attachments") == 1, plan

    invalidated = []
    result = rollback_batch(
        conn,
        profile,
        batch_id,
        invalidate_journals=lambda c, pid: invalidated.append(pid),
    )
    assert result["transactions_deleted"] == 2, result
    assert invalidated == ["p"], "journals must be invalidated after a rollback"

    surviving = {row["id"] for row in conn.execute("SELECT id FROM transactions")}
    assert surviving == {"old"}, f"enriched pre-existing row was deleted: {surviving}"
    assert list_batches(conn, "p") == [], "batch row should be gone"
    assert not conn.execute("SELECT 1 FROM import_batch_transactions").fetchall()

    # An import that only enriched existing rows has nothing to roll back.
    assert (
        record_batch(
            conn,
            profile,
            wallet_id="wal",
            source_format="generic_ledger",
            source_filename="again.csv",
            column_map=None,
            outcome={"inserted_records": [], "updated_records": [{"transaction_id": "old"}]},
        )
        is None
    )

    # Observed assets come from rows, not config.
    conn.execute("INSERT INTO transactions VALUES('l1','p','wal','LBTC')")
    assert observed_assets(conn, "p")["wal"] == ["BTC", "LBTC"]

    try:
        plan_rollback(conn, "p", "nope")
    except AppError as exc:
        assert exc.code == "not_found"
    else:
        raise AssertionError("unknown batch must not silently succeed")

    print("import_batches demo OK")


if __name__ == "__main__":
    demo()
