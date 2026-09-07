"""Bounded keyset pages over immutable accounting identities.

Continuation is pinned to book, collection and input revision. Tokens are not
authorization credentials; every request independently verifies book scope.
"""
from __future__ import annotations

import base64
import binascii
import hashlib
import json
from typing import Any, Callable

from kassiber.errors import AppError
from .ledger import require_book

_COLLECTIONS = {
    "evidence": ("gl_evidence", "created_at", False),
    "statements": ("gl_bank_statements", "end_date", True),
    "items": ("gl_open_items", "due_date", False),
    "schedules": ("gl_schedules", "id", False),
}


def _binding(conn: Any, profile_id: str, collection: str) -> tuple[str, int]:
    book = require_book(conn, profile_id)
    table = _COLLECTIONS[collection][0]
    count = conn.execute(f"SELECT COUNT(*) FROM {table} WHERE profile_id=?", (profile_id,)).fetchone()[0]
    encoded = json.dumps([1, profile_id, collection, book["revision"], count], separators=(",", ":"))
    return hashlib.sha256(encoded.encode()).hexdigest(), count


def records_page(conn: Any, profile_id: str, collection: str, *, limit: int = 100,
                 cursor: str | None = None, materialize: Callable[[str], dict]) -> dict:
    if collection not in _COLLECTIONS or type(limit) is not int or not 1 <= limit <= 500:
        raise AppError("Invalid accounting page request", code="accounting_invalid_input")
    binding, count = _binding(conn, profile_id, collection)
    table, field, descending = _COLLECTIONS[collection]
    last = None
    if cursor is not None:
        try:
            if not isinstance(cursor, str) or len(cursor) > 2048:
                raise ValueError()
            decoded = json.loads(base64.b64decode(cursor.encode("ascii"), altchars=b"-_", validate=True))
            if not isinstance(decoded, dict) or set(decoded) != {"version", "profile", "collection", "binding", "last"}:
                raise ValueError()
            if decoded["version"] != 1 or decoded["profile"] != profile_id or decoded["collection"] != collection:
                raise ValueError()
            if decoded["binding"] != binding:
                raise AppError("Accounting records changed; refresh the collection", code="accounting_stale_cursor")
            last = decoded["last"]
            if not isinstance(last, list) or len(last) != 2 or any(not isinstance(value, str) or not value or len(value) > 128 for value in last):
                raise ValueError()
        except (ValueError, TypeError, UnicodeError, binascii.Error, RecursionError) as exc:
            raise AppError("Invalid accounting continuation", code="accounting_invalid_cursor") from exc
    parameters: list[Any] = [profile_id]
    after = ""
    if last is not None:
        operator = "<" if descending else ">"
        after = f" AND ({field}{operator}? OR ({field}=? AND id>?))"
        parameters.extend((last[0], last[0], last[1]))
    rows = conn.execute(f"SELECT id,{field} AS sort_key FROM {table} WHERE profile_id=?{after} "
                        f"ORDER BY {field} {'DESC' if descending else 'ASC'},id ASC LIMIT ?", (*parameters, limit + 1)).fetchall()
    selected = rows[:limit]
    records = [materialize(row["id"]) for row in selected]
    if _binding(conn, profile_id, collection)[0] != binding:
        raise AppError("Accounting records changed while reading; refresh", code="accounting_stale_cursor")
    next_cursor = None
    if len(rows) > limit:
        tail = selected[-1]
        token = {"version": 1, "profile": profile_id, "collection": collection, "binding": binding,
                 "last": [tail["sort_key"], tail["id"]]}
        next_cursor = base64.urlsafe_b64encode(json.dumps(token, separators=(",", ":")).encode()).decode()
    return {collection: records, "next_cursor": next_cursor, "binding": binding, "total_count": count}
