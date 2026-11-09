"""Keep a book's decisions across "Reset book data" and bind them again.

A reset rebuilds what Kassiber derives or observes (synced transactions,
journals, observer state) but keeps what the user decided. Decisions point at
transactions by row id, and deleting the rows would cascade them away, so the
reset first records each deleted transaction's stable identity and parks every
decision row that references one, verbatim.

The resync then gives a re-observed transaction its old id back: an exact
fingerprint match first, otherwise the only reservation with the same wallet,
external id, asset and direction. Rows keyed by transaction id elsewhere
(custody leg anchors, gap reviews, filed-report dependencies, journal entry
ids) therefore line up again without rewriting. A parked decision is restored
once every transaction it names is back with unchanged facts. One whose
transactions are missing or changed stays parked and holds the transactions
that did come back (`book_reset_decision_held`); it is never applied to other
rows and never silently dropped.
"""

from __future__ import annotations

import json
import sqlite3
from typing import Any, Iterable, Mapping

HOLD_REASON = "book_reset_decision_held"

# The book's transactions, as a subquery taking the profile id.
_TX = "SELECT id FROM transactions WHERE profile_id = ?"

# Decision rows deleted with a profile's transactions, parents first, with the
# columns naming a transaction. Child rows without one follow their parent.
PARKED_TABLES: tuple[tuple[str, str, tuple[str, ...]], ...] = (
    ("transaction_tags", f"transaction_id IN ({_TX})", ("transaction_id",)),
    ("loan_legs", f"transaction_id IN ({_TX})", ("transaction_id",)),
    (
        "transaction_pairs",
        f"out_transaction_id IN ({_TX}) OR in_transaction_id IN ({_TX})",
        ("out_transaction_id", "in_transaction_id"),
    ),
    (
        "transaction_pair_dismissals",
        f"out_transaction_id IN ({_TX}) OR in_transaction_id IN ({_TX})",
        ("out_transaction_id", "in_transaction_id"),
    ),
    ("direct_swap_payouts", f"out_transaction_id IN ({_TX})", ("out_transaction_id",)),
    (
        "custody_component_transaction_memberships",
        f"transaction_id IN ({_TX})",
        ("transaction_id",),
    ),
    ("commercial_links", f"transaction_id IN ({_TX})", ("transaction_id",)),
    ("attachments", f"transaction_id IN ({_TX})", ("transaction_id",)),
    (
        "external_document_attachments",
        f"attachment_id IN (SELECT id FROM attachments WHERE transaction_id IN ({_TX}))",
        (),
    ),
    (
        "source_funds_links",
        f"from_transaction_id IN ({_TX}) OR to_transaction_id IN ({_TX})",
        ("from_transaction_id", "to_transaction_id"),
    ),
    (
        "source_funds_link_attachments",
        "link_id IN (SELECT id FROM source_funds_links WHERE "
        f"from_transaction_id IN ({_TX}) OR to_transaction_id IN ({_TX})) "
        f"OR attachment_id IN (SELECT id FROM attachments WHERE transaction_id IN ({_TX}))",
        (),
    ),
    (
        "source_funds_source_attachments",
        f"attachment_id IN (SELECT id FROM attachments WHERE transaction_id IN ({_TX}))",
        (),
    ),
    ("source_funds_cases", f"target_transaction_id IN ({_TX})", ("target_transaction_id",)),
    (
        "source_funds_snapshots",
        f"case_id IN (SELECT id FROM source_funds_cases WHERE target_transaction_id IN ({_TX}))",
        (),
    ),
    ("transaction_edit_events", f"transaction_id IN ({_TX})", ("transaction_id",)),
    (
        "transaction_edit_fields",
        f"event_id IN (SELECT id FROM transaction_edit_events WHERE transaction_id IN ({_TX}))",
        (),
    ),
)

# Parked rows that bind only together with every row sharing this column.
_GROUPED_TABLES = {"custody_component_transaction_memberships": "component_id"}

# Rows that also die with the transactions but are rebuilt by the resync or
# the next journal run. `custody_component_legs` keeps its anchor through
# ON DELETE SET NULL and is re-pointed when the transaction returns.
DERIVED_TRANSACTION_TABLES = frozenset(
    {
        "chain_observation_provenance",
        "import_batch_transactions",
        "journal_custody_decisions",
        "journal_custody_economic_relations",
        "journal_entries",
        "journal_quantity_postings",
        "journal_quarantines",
    }
)
REANCHORED_TRANSACTION_TABLES = frozenset({"custody_component_legs"})

# Transaction columns only the user writes.
METADATA_COLUMNS = (
    "note",
    "review_status",
    "taxability_override",
    "at_regime_override",
    "at_category_override",
    "kind_override",
    "excluded",
    "pricing_review_accepted_price",
    "commercial_applied_link_id",
)
# Price columns, kept only when the user (or a reviewed commercial link) set
# the price. A provider or rate-cache price is re-derived after the resync.
PRICING_COLUMNS = (
    "fiat_currency",
    "fiat_rate",
    "fiat_value",
    "fiat_price_source",
    "fiat_rate_exact",
    "fiat_value_exact",
    "pricing_source_kind",
    "pricing_provider",
    "pricing_pair",
    "pricing_timestamp",
    "pricing_fetched_at",
    "pricing_granularity",
    "pricing_method",
    "pricing_external_ref",
    "pricing_quality",
)
_PRICING_EDIT_FIELDS = (
    "fiat_currency",
    "fiat_rate",
    "fiat_value",
    "fiat_price_source",
    "pricing_source_kind",
    "pricing_provider",
    "pricing_pair",
    "pricing_timestamp",
    "pricing_fetched_at",
    "pricing_granularity",
    "pricing_method",
    "pricing_external_ref",
    "pricing_quality",
)
# What a decision asserts about a transaction. A row that comes back with any
# of these changed keeps its id but binds nothing.
FACT_COLUMNS = ("direction", "asset", "amount", "fee", "amount_includes_fee")


def _placeholders(sql: str, profile_id: str) -> tuple[str, ...]:
    return (profile_id,) * sql.count("?")


def _encode_value(value: Any) -> Any:
    if isinstance(value, (bytes, bytearray, memoryview)):
        return {"$hex": bytes(value).hex()}
    return value


def _decode_value(value: Any) -> Any:
    if isinstance(value, dict):
        return bytes.fromhex(value["$hex"])
    return value


def _rows(conn: sqlite3.Connection, sql: str, params: Iterable[Any] = ()):
    cursor = conn.execute(sql, tuple(params))
    columns = [item[0] for item in cursor.description]
    return columns, cursor.fetchall()


def _metadata(conn: sqlite3.Connection, row: Mapping[str, Any]) -> dict[str, Any]:
    kept = {
        column: row[column]
        for column in METADATA_COLUMNS
        if row[column] is not None and not (column == "excluded" and row[column] == 0)
    }
    priced_by_user = (
        row["pricing_source_kind"] == "manual_override"
        or row["fiat_price_source"] == "manual"
        or row["commercial_applied_link_id"] is not None
        or conn.execute(
            "SELECT 1 FROM transaction_edit_fields f "
            "JOIN transaction_edit_events e ON e.id = f.event_id "
            f"WHERE e.transaction_id = ? AND f.field IN ({','.join('?' for _ in _PRICING_EDIT_FIELDS)}) "
            "LIMIT 1",
            (row["id"], *_PRICING_EDIT_FIELDS),
        ).fetchone()
        is not None
    )
    if priced_by_user:
        kept.update({column: row[column] for column in PRICING_COLUMNS})
    return kept


def park_profile(
    conn: sqlite3.Connection,
    workspace_id: str,
    profile_id: str,
    timestamp: str,
) -> dict[str, int]:
    """Record the book's transactions and park their decision rows.

    Runs inside the reset's write transaction, before the transactions are
    deleted. Returns the kept counts by table.
    """
    kept: dict[str, int] = {}
    for table, predicate, transaction_columns in PARKED_TABLES:
        columns, rows = _rows(
            conn,
            f"SELECT * FROM {table} WHERE {predicate} ORDER BY rowid",
            _placeholders(predicate, profile_id),
        )
        # A component's memberships bind together: one restored without the
        # rest would mark an incomplete component effective.
        groups: dict[Any, list[str]] = {}
        if table in _GROUPED_TABLES:
            for row in rows:
                values = dict(zip(columns, row))
                members = groups.setdefault(values[_GROUPED_TABLES[table]], [])
                for column in transaction_columns:
                    if values[column] is not None and str(values[column]) not in members:
                        members.append(str(values[column]))
        for row in rows:
            values = dict(zip(columns, row))
            transaction_ids: list[str] = []
            if table in _GROUPED_TABLES:
                transaction_ids = list(groups[values[_GROUPED_TABLES[table]]])
            for column in transaction_columns:
                value = values.get(column)
                if value is not None and str(value) not in transaction_ids:
                    transaction_ids.append(str(value))
            conn.execute(
                """
                INSERT INTO book_reset_parked_rows(
                    workspace_id, profile_id, table_name, row_json,
                    transaction_ids_json, created_at
                ) VALUES(?, ?, ?, ?, ?, ?)
                """,
                (
                    workspace_id,
                    profile_id,
                    table,
                    json.dumps({key: _encode_value(value) for key, value in values.items()}),
                    json.dumps(transaction_ids),
                    timestamp,
                ),
            )
        kept[table] = len(rows)
    metadata_rows = 0
    for row in conn.execute(
        "SELECT * FROM transactions WHERE profile_id = ? ORDER BY rowid", (profile_id,)
    ).fetchall():
        metadata = _metadata(conn, row)
        if metadata:
            metadata_rows += 1
        # A second reset before the first resync finished replaces the record.
        conn.execute(
            """
            INSERT OR REPLACE INTO book_reset_transactions(
                transaction_id, workspace_id, profile_id, wallet_id, fingerprint,
                external_id, occurred_at, direction, asset, amount, fee,
                amount_includes_fee, metadata_json, state, changed_fields_json,
                created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', '[]', ?)
            """,
            (
                row["id"],
                row["workspace_id"],
                profile_id,
                row["wallet_id"],
                row["fingerprint"],
                row["external_id"],
                row["occurred_at"],
                row["direction"],
                row["asset"],
                row["amount"],
                row["fee"],
                row["amount_includes_fee"],
                json.dumps(metadata, sort_keys=True),
                timestamp,
            ),
        )
    kept["transaction_metadata"] = metadata_rows
    return kept


def has_state(conn: sqlite3.Connection, profile_id: str) -> bool:
    return (
        conn.execute(
            "SELECT EXISTS(SELECT 1 FROM book_reset_transactions WHERE profile_id = ?) "
            "OR EXISTS(SELECT 1 FROM book_reset_parked_rows WHERE profile_id = ?)",
            (profile_id, profile_id),
        ).fetchone()[0]
        == 1
    )


def discard_profile(conn: sqlite3.Connection, profile_id: str) -> dict[str, int]:
    """Drop every kept record of the book (the wipe-everything reset)."""
    counts = {
        "book_reset_transactions": int(
            conn.execute(
                "SELECT COUNT(*) FROM book_reset_transactions WHERE profile_id = ?", (profile_id,)
            ).fetchone()[0]
        ),
        "book_reset_parked_rows": int(
            conn.execute(
                "SELECT COUNT(*) FROM book_reset_parked_rows WHERE profile_id = ?", (profile_id,)
            ).fetchone()[0]
        ),
    }
    conn.execute("DELETE FROM book_reset_parked_rows WHERE profile_id = ?", (profile_id,))
    conn.execute("DELETE FROM book_reset_transactions WHERE profile_id = ?", (profile_id,))
    return counts


def claim_transaction_id(
    conn: sqlite3.Connection,
    profile_id: str,
    wallet_id: str,
    fingerprint: str,
    external_id: str | None,
    asset: str,
    direction: str,
) -> str | None:
    """Return the reset id an incoming row takes back, if exactly one fits."""
    row = conn.execute(
        """
        SELECT transaction_id FROM book_reset_transactions
        WHERE profile_id = ? AND wallet_id = ? AND fingerprint = ? AND state = 'pending'
          AND NOT EXISTS (SELECT 1 FROM transactions t WHERE t.id = transaction_id)
        """,
        (profile_id, wallet_id, fingerprint),
    ).fetchone()
    if row is not None:
        return str(row[0])
    if not external_id:
        return None
    rows = conn.execute(
        """
        SELECT transaction_id, state FROM book_reset_transactions
        WHERE profile_id = ? AND wallet_id = ? AND external_id = ?
          AND asset = ? AND direction = ?
        """,
        (profile_id, wallet_id, external_id, asset, direction),
    ).fetchall()
    if len(rows) != 1 or rows[0][1] != "pending":
        return None
    transaction_id = str(rows[0][0])
    if conn.execute("SELECT 1 FROM transactions WHERE id = ?", (transaction_id,)).fetchone():
        return None
    return transaction_id


def settle_claim(conn: sqlite3.Connection, transaction_id: str) -> str:
    """Bind a re-inserted row's own metadata, or mark its facts changed.

    Returns ``bound``, ``bound_with_metadata`` or ``changed``.
    """
    reservation = conn.execute(
        "SELECT * FROM book_reset_transactions WHERE transaction_id = ?", (transaction_id,)
    ).fetchone()
    current = conn.execute(
        f"SELECT profile_id, {', '.join(FACT_COLUMNS)} FROM transactions WHERE id = ?",
        (transaction_id,),
    ).fetchone()
    changed = [column for column in FACT_COLUMNS if reservation[column] != current[column]]
    if changed:
        conn.execute(
            "UPDATE book_reset_transactions SET state = 'changed', changed_fields_json = ? "
            "WHERE transaction_id = ?",
            (json.dumps(changed), transaction_id),
        )
        return "changed"
    metadata = json.loads(reservation["metadata_json"] or "{}")
    if metadata:
        columns = sorted(metadata)
        conn.execute(
            f"UPDATE transactions SET {', '.join(f'{column} = ?' for column in columns)} WHERE id = ?",
            (*(metadata[column] for column in columns), transaction_id),
        )
    # The leg kept its anchor when the reset nulled its transaction.
    conn.execute(
        "UPDATE custody_component_legs SET transaction_id = anchor_transaction_id "
        "WHERE profile_id = ? AND transaction_id IS NULL AND anchor_transaction_id = ?",
        (current["profile_id"], transaction_id),
    )
    conn.execute("DELETE FROM book_reset_transactions WHERE transaction_id = ?", (transaction_id,))
    return "bound_with_metadata" if metadata else "bound"


def _transaction_states(conn: sqlite3.Connection, transaction_ids: Iterable[str]) -> dict[str, str]:
    states: dict[str, str] = {}
    for transaction_id in transaction_ids:
        present = conn.execute(
            "SELECT 1 FROM transactions WHERE id = ?", (transaction_id,)
        ).fetchone() is not None
        reserved = conn.execute(
            "SELECT 1 FROM book_reset_transactions WHERE transaction_id = ?", (transaction_id,)
        ).fetchone() is not None
        states[transaction_id] = (
            "missing" if not present else "changed" if reserved else "bound"
        )
    return states


def _foreign_keys(conn: sqlite3.Connection, table: str) -> list[tuple[str, list[tuple[str, str]]]]:
    groups: dict[int, tuple[str, list[tuple[str, str]]]] = {}
    for fk in conn.execute(f'PRAGMA foreign_key_list("{table}")').fetchall():
        parent = str(fk[2])
        groups.setdefault(int(fk[0]), (parent, []))[1].append((str(fk[3]), str(fk[4] or "id")))
    return [groups[key] for key in sorted(groups)]


def _table_columns(conn: sqlite3.Connection, table: str) -> list[str]:
    return [str(row[1]) for row in conn.execute(f'PRAGMA table_info("{table}")').fetchall()]


def rebind_parked(conn: sqlite3.Connection, profile_id: str) -> int:
    """Restore every parked row whose transactions and parents are back."""
    restored = 0
    schema: dict[str, tuple[list[str], list[tuple[str, list[tuple[str, str]]]]]] = {}
    progress = True
    while progress:
        progress = False
        for parked in conn.execute(
            "SELECT seq, table_name, row_json, transaction_ids_json FROM book_reset_parked_rows "
            "WHERE profile_id = ? ORDER BY seq",
            (profile_id,),
        ).fetchall():
            transaction_ids = json.loads(parked["transaction_ids_json"])
            states = _transaction_states(conn, transaction_ids)
            if any(state != "bound" for state in states.values()):
                continue
            table = str(parked["table_name"])
            if table not in schema:
                schema[table] = (_table_columns(conn, table), _foreign_keys(conn, table))
            columns, foreign_keys = schema[table]
            values = {key: _decode_value(value) for key, value in json.loads(parked["row_json"]).items()}
            if not columns or not _parents_present(conn, values, foreign_keys):
                continue
            insert_columns = [column for column in columns if column in values]
            conn.execute("SAVEPOINT book_reset_rebind")
            try:
                conn.execute(
                    f"INSERT INTO {table}({', '.join(insert_columns)}) "
                    f"VALUES({', '.join('?' for _ in insert_columns)})",
                    tuple(values[column] for column in insert_columns),
                )
            except sqlite3.IntegrityError:
                # Something now occupies the decision's place (a newer pair or
                # mark): keep it parked and held rather than override either.
                conn.execute("ROLLBACK TO SAVEPOINT book_reset_rebind")
                conn.execute("RELEASE SAVEPOINT book_reset_rebind")
                continue
            conn.execute("RELEASE SAVEPOINT book_reset_rebind")
            conn.execute("DELETE FROM book_reset_parked_rows WHERE seq = ?", (parked["seq"],))
            restored += 1
            progress = True
    return restored


def _parents_present(
    conn: sqlite3.Connection,
    values: Mapping[str, Any],
    foreign_keys: list[tuple[str, list[tuple[str, str]]]],
) -> bool:
    for parent, pairs in foreign_keys:
        if any(values.get(child) is None for child, _ in pairs):
            continue
        predicate = " AND ".join(f'"{column}" = ?' for _, column in pairs)
        if conn.execute(
            f'SELECT 1 FROM "{parent}" WHERE {predicate} LIMIT 1',
            tuple(values[child] for child, _ in pairs),
        ).fetchone() is None:
            return False
    return True


def _held(conn: sqlite3.Connection, profile_id: str):
    """Unbound decisions by the returned transactions they hold, plus counts.

    A decision is held once any transaction it names is back (bound or with
    changed facts) and it still cannot be restored; until then it waits.
    """
    holds: dict[str, dict[str, Any]] = {}
    held = waiting = 0

    def entry(transaction_id: str) -> dict[str, Any]:
        return holds.setdefault(
            transaction_id,
            {"held_decisions": {}, "metadata_held": False, "missing": set(), "changed": set()},
        )

    for parked in conn.execute(
        "SELECT table_name, transaction_ids_json FROM book_reset_parked_rows "
        "WHERE profile_id = ? ORDER BY seq",
        (profile_id,),
    ).fetchall():
        states = _transaction_states(conn, json.loads(parked["transaction_ids_json"]))
        present = [key for key, state in states.items() if state != "missing"]
        if not present:
            waiting += 1
            continue
        held += 1
        table = str(parked["table_name"])
        for transaction_id in present:
            item = entry(transaction_id)
            item["held_decisions"][table] = item["held_decisions"].get(table, 0) + 1
            item["missing"].update(key for key, state in states.items() if state == "missing")
            item["changed"].update(key for key, state in states.items() if state == "changed")
    for reservation in conn.execute(
        "SELECT transaction_id, state FROM book_reset_transactions "
        "WHERE profile_id = ? AND metadata_json != '{}' ORDER BY transaction_id",
        (profile_id,),
    ).fetchall():
        if reservation["state"] == "changed":
            held += 1
            item = entry(str(reservation["transaction_id"]))
            item["metadata_held"] = True
            item["changed"].add(str(reservation["transaction_id"]))
        else:
            waiting += 1
    return holds, held, waiting


def counts(conn: sqlite3.Connection, profile_id: str) -> dict[str, int]:
    """Kept decisions not yet restored: held by a returned row, or waiting."""
    _, held, waiting = _held(conn, profile_id)
    return {"held": held, "waiting": waiting}


def _identity(conn: sqlite3.Connection, transaction_id: str) -> dict[str, Any]:
    row = conn.execute(
        "SELECT wallet_id, external_id, asset, direction FROM book_reset_transactions "
        "WHERE transaction_id = ?",
        (transaction_id,),
    ).fetchone()
    identity: dict[str, Any] = {"transaction_id": transaction_id}
    if row is not None:
        identity.update(
            wallet_id=row["wallet_id"],
            external_id=row["external_id"],
            asset=row["asset"],
            direction=row["direction"],
        )
    return identity


def held_quarantines(conn: sqlite3.Connection, profile_id: str) -> tuple[dict[str, Any], ...]:
    """Hold every returned transaction a kept decision could not bind to."""
    holds, _, _ = _held(conn, profile_id)
    quarantines = []
    for transaction_id in sorted(holds):
        item = holds[transaction_id]
        workspace = conn.execute(
            "SELECT workspace_id FROM transactions WHERE id = ?", (transaction_id,)
        ).fetchone()
        changed = []
        for changed_id in sorted(item["changed"]):
            fields = conn.execute(
                "SELECT changed_fields_json FROM book_reset_transactions WHERE transaction_id = ?",
                (changed_id,),
            ).fetchone()
            changed.append(
                {"transaction_id": changed_id, "fields": json.loads(fields[0]) if fields else []}
            )
        detail = {
            "held_decisions": dict(sorted(item["held_decisions"].items())),
            "metadata_held": item["metadata_held"],
            "missing_transactions": [_identity(conn, key) for key in sorted(item["missing"])],
            "changed_transactions": changed,
        }
        quarantines.append(
            {
                "transaction_id": transaction_id,
                "workspace_id": workspace["workspace_id"] if workspace else None,
                "profile_id": profile_id,
                "reason": HOLD_REASON,
                "detail_json": json.dumps(detail, sort_keys=True),
            }
        )
    return tuple(quarantines)


def parked_attachment_relpaths(conn: sqlite3.Connection) -> set[str]:
    """Managed files of parked attachments, which must survive file cleanup."""
    paths = set()
    for (row_json,) in conn.execute(
        "SELECT row_json FROM book_reset_parked_rows WHERE table_name = 'attachments'"
    ).fetchall():
        relpath = json.loads(row_json).get("stored_relpath")
        if relpath:
            paths.add(str(relpath))
    return paths
