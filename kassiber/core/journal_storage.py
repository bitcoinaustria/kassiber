"""Stable identities and changed-row writes for the two large journal tables.

The caller owns the journal transaction. Other derived journal tables keep
full replacement. See the R4 storage-format contract for coexistence rules.
"""
from __future__ import annotations

import hashlib
import json
import math
import sqlite3
import uuid
from collections.abc import Sequence
from contextlib import contextmanager
from datetime import timedelta
from typing import Any

from ..time_utils import parse_iso_datetime_or_none

ENTRY_COLUMNS = (
    "id", "workspace_id", "profile_id", "transaction_id", "wallet_id", "account_id",
    "occurred_at", "entry_type", "asset", "quantity", "fiat_value", "unit_cost",
    "cost_basis", "proceeds", "gain_loss", "fiat_value_exact", "unit_cost_exact",
    "cost_basis_exact", "proceeds_exact", "gain_loss_exact", "calculation_json",
    "calculation_compact", "pricing_source_kind", "pricing_quality", "description",
    "at_category", "at_kennzahl", "capital_gains_type", "created_at",
)
POSTING_COLUMNS = (
    "posting_id", "workspace_id", "profile_id", "transaction_id", "observation_hash",
    "occurred_at", "asset", "location_kind", "location_id", "amount_msat", "state",
    "created_at",
)


def journal_processed_at(candidate: str, previous: str | None) -> str:
    """Keep explanation references distinct when the build clock repeats.

    Row creation times still use the supplied build clock. Only the profile's
    rebuild marker needs a microsecond successor when that clock has not
    advanced; the source input version and all accounting facts stay intact.
    """
    current = parse_iso_datetime_or_none(candidate)
    prior = parse_iso_datetime_or_none(previous)
    if current is not None and prior is not None and current <= prior:
        return (prior + timedelta(microseconds=1)).isoformat(timespec="microseconds").replace("+00:00", "Z")
    return candidate


def journal_entry_id(
    profile_id: str, transaction_id: str, wallet_id: str, entry_type: str,
    asset: str, ordinal: int,
) -> str:
    """UUIDv8: SHA-256 of the versioned compact ASCII JSON identity tuple."""
    encoded = json.dumps(
        ["kassiber-journal-entry-v1", profile_id, transaction_id, wallet_id,
         entry_type, asset, ordinal],
        ensure_ascii=True, separators=(",", ":"),
    ).encode("ascii")
    digest = bytearray(hashlib.sha256(encoded).digest()[:16])
    digest[6] = (digest[6] & 0x0f) | 0x80
    digest[8] = (digest[8] & 0x3f) | 0x80
    return str(uuid.UUID(bytes=bytes(digest)))


def _same_values(old, new, real_positions):
    # created_at (last) records the last write, not the last build. Tuple
    # equality alone loses the sign of zero on a REAL column.
    return old[:-1] == new[:-1] and all(
        old[index] != 0 or math.copysign(1, old[index]) == math.copysign(1, new[index])
        for index in real_positions
    )


def _replace_rows(conn, profile_id, rows, *, table, columns, real_positions=(), tie_position=None):
    names = ", ".join(columns)
    existing = {
        row[0]: tuple(row)
        for row in conn.execute(
            f"SELECT {names} FROM {table} WHERE profile_id = ?", (profile_id,)
        ).fetchall()
    }
    wanted = {row[0] for row in rows}
    if len(wanted) != len(rows):
        # Full replacement rejected duplicate primary keys too. A duplicate
        # must not disappear merely because both copies match a retained row.
        raise sqlite3.IntegrityError(f"duplicate {table} identifier")
    changed = {
        row[0] for row in rows
        if row[0] not in existing or not _same_values(existing[row[0]], row, real_positions)
    }
    if tie_position is not None:
        dirty_times = {row[tie_position] for row in rows if row[0] in changed}
        kept_stamps = {}
        for row in rows:
            old = existing.get(row[0])
            if old is not None:
                kept_stamps.setdefault(row[tie_position], set()).add(old[-1])
        dirty_times.update(time for time, stamps in kept_stamps.items() if len(stamps) > 1)
        changed = {row[0] for row in rows if row[tie_position] in dirty_times}
    conn.executemany(
        f"DELETE FROM {table} WHERE profile_id = ? AND {columns[0]} = ?",
        [(profile_id, identifier) for identifier in existing if identifier not in wanted or identifier in changed],
    )
    conn.executemany(
        f"INSERT INTO {table}({names}) VALUES({','.join('?' for _ in columns)})",
        [row for row in rows if row[0] in changed],
    )


def replace_journal_entries(conn: sqlite3.Connection, profile_id: str, rows: Sequence[tuple[Any, ...]]) -> None:
    """Write changed timestamp groups in engine order; retain all other rows."""
    _replace_rows(conn, profile_id, rows, table="journal_entries", columns=ENTRY_COLUMNS,
                  real_positions=(10, 11, 12, 13, 14), tie_position=6)


def replace_quantity_postings(conn: sqlite3.Connection, profile_id: str, rows: Sequence[tuple[Any, ...]]) -> None:
    """Write only changed postings in projection order, with no timestamp tie rule."""
    _replace_rows(conn, profile_id, rows, table="journal_quantity_postings", columns=POSTING_COLUMNS)


@contextmanager
def journal_write_cache(conn: sqlite3.Connection):
    """Use 128 MiB while rebuilding; restore the connection's exact cache setting.

    Journal mode, synchronous level and explicit spill settings are untouched. The
    caller owns its transaction, including any savepoints and commit policy.
    """
    previous = conn.execute("PRAGMA cache_size").fetchone()[0]
    conn.execute("PRAGMA cache_size = -131072")
    try:
        yield
    finally:
        conn.execute(f"PRAGMA cache_size = {int(previous)}")
