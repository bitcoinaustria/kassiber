"""Persisted AI chat sessions, profile-scoped, inside the SQLCipher boundary.

Chat content is mostly *derived from the database* (tool results are views of
transactions, journals, and reports), so the encrypted database is the only
storage location consistent with the documented "the SQLCipher passphrase is
the perimeter" doctrine. Two tables back this module:

    ai_chat_sessions(id, workspace_id, profile_id, title, provider, model,
                     created_at, updated_at)
    ai_chat_messages(id, session_id, seq, role, content, tool_calls_json,
                     provenance_json, finish_reason, created_at)

Policy: the `ai_chat_history` setting is ``auto`` (default), ``on``, or
``off``. ``auto`` persists only when the database file is SQLCipher-encrypted
— writing chat history into a plaintext database is something the user must
ask for explicitly. Clients opt in per request; the daemon never persists a
chat whose request did not carry a persistence intent, so existing clients
keep their current behavior.

Hard exclusions live with the consumers: diagnostics collection and audit
packages must not include these tables, and no AI tool exposes chat history
back to the model.

This module is the thin storage seam — callers manage the connection and
commit boundary; helpers return plain dicts so machine envelopes stay
deterministic.
"""

from __future__ import annotations

import json
import sqlite3
import uuid
from typing import Any, Optional

from ..db import (
    get_setting,
    resolve_database_path,
    resolve_effective_data_root,
    set_setting,
)
from ..errors import AppError
from ..secrets.sqlcipher import looks_like_plaintext_sqlite
from ..time_utils import now_iso


AI_CHAT_HISTORY_SETTING = "ai_chat_history"
AI_CHAT_HISTORY_MODES = ("auto", "on", "off")
SESSION_TITLE_MAX_CHARS = 80


def database_file_is_encrypted(data_root: str) -> bool:
    db_path = resolve_database_path(resolve_effective_data_root(data_root))
    return (
        db_path.exists()
        and db_path.stat().st_size > 0
        and not looks_like_plaintext_sqlite(db_path)
    )


def history_mode(conn: sqlite3.Connection) -> str:
    value = get_setting(conn, AI_CHAT_HISTORY_SETTING)
    return value if value in AI_CHAT_HISTORY_MODES else "auto"


def set_history_mode(conn: sqlite3.Connection, value: str, *, commit: bool = True) -> str:
    if value not in AI_CHAT_HISTORY_MODES:
        raise AppError(
            "chat history mode must be one of auto, on, off",
            code="validation",
            details={"value": value},
        )
    set_setting(conn, AI_CHAT_HISTORY_SETTING, value)
    if commit:
        conn.commit()
    return value


def history_enabled(conn: sqlite3.Connection, *, database_encrypted: bool) -> bool:
    """Resolve the ``auto`` policy: encrypted databases persist, plaintext
    databases stay ephemeral unless the user explicitly set ``on``."""
    mode = history_mode(conn)
    if mode == "on":
        return True
    if mode == "off":
        return False
    return database_encrypted


def session_title_from_prompt(prompt: str) -> str:
    collapsed = " ".join(prompt.split())
    if len(collapsed) <= SESSION_TITLE_MAX_CHARS:
        return collapsed or "Chat"
    return collapsed[: SESSION_TITLE_MAX_CHARS - 1].rstrip() + "…"


def create_session(
    conn: sqlite3.Connection,
    workspace_id: str,
    profile_id: str,
    *,
    title: str,
    provider: Optional[str] = None,
    model: Optional[str] = None,
    commit: bool = True,
) -> dict:
    session_id = str(uuid.uuid4())
    timestamp = now_iso()
    conn.execute(
        """
        INSERT INTO ai_chat_sessions(id, workspace_id, profile_id, title,
                                     provider, model, created_at, updated_at)
        VALUES(?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (session_id, workspace_id, profile_id, title, provider, model, timestamp, timestamp),
    )
    if commit:
        conn.commit()
    return get_session(conn, profile_id, session_id, include_messages=False)


def append_exchange(
    conn: sqlite3.Connection,
    profile_id: str,
    session_id: str,
    *,
    user_content: str,
    assistant_content: str,
    tool_calls: Optional[list[dict[str, Any]]] = None,
    provenance: Optional[dict[str, Any]] = None,
    finish_reason: Optional[str] = None,
    provider: Optional[str] = None,
    model: Optional[str] = None,
    commit: bool = True,
) -> dict:
    """Append one user/assistant exchange to an existing session."""
    session = _session_row(conn, profile_id, session_id)
    timestamp = now_iso()
    next_seq = (
        conn.execute(
            "SELECT COALESCE(MAX(seq), -1) + 1 FROM ai_chat_messages WHERE session_id = ?",
            (session_id,),
        ).fetchone()[0]
    )
    conn.execute(
        """
        INSERT INTO ai_chat_messages(id, session_id, seq, role, content,
                                     tool_calls_json, provenance_json,
                                     finish_reason, created_at)
        VALUES(?, ?, ?, 'user', ?, NULL, NULL, NULL, ?)
        """,
        (str(uuid.uuid4()), session_id, next_seq, user_content, timestamp),
    )
    conn.execute(
        """
        INSERT INTO ai_chat_messages(id, session_id, seq, role, content,
                                     tool_calls_json, provenance_json,
                                     finish_reason, created_at)
        VALUES(?, ?, ?, 'assistant', ?, ?, ?, ?, ?)
        """,
        (
            str(uuid.uuid4()),
            session_id,
            next_seq + 1,
            assistant_content,
            json.dumps(tool_calls, sort_keys=True) if tool_calls else None,
            json.dumps(provenance, sort_keys=True) if provenance else None,
            finish_reason,
            timestamp,
        ),
    )
    conn.execute(
        """
        UPDATE ai_chat_sessions
        SET updated_at = ?,
            provider = COALESCE(?, provider),
            model = COALESCE(?, model)
        WHERE id = ?
        """,
        (timestamp, provider, model, session_id),
    )
    if commit:
        conn.commit()
    return {"session_id": session["id"], "appended": 2}


def list_sessions(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    limit: int = 50,
) -> list[dict]:
    rows = conn.execute(
        """
        SELECT s.*, COUNT(m.id) AS message_count
        FROM ai_chat_sessions s
        LEFT JOIN ai_chat_messages m ON m.session_id = s.id
        WHERE s.profile_id = ?
        GROUP BY s.id
        ORDER BY s.updated_at DESC, s.id
        LIMIT ?
        """,
        (profile_id, max(1, int(limit))),
    ).fetchall()
    return [_session_to_dict(row) for row in rows]


def get_session(
    conn: sqlite3.Connection,
    profile_id: str,
    session_id: str,
    *,
    include_messages: bool = True,
) -> dict:
    session = _session_to_dict(_session_row(conn, profile_id, session_id))
    if include_messages:
        message_rows = conn.execute(
            "SELECT * FROM ai_chat_messages WHERE session_id = ? ORDER BY seq",
            (session_id,),
        ).fetchall()
        session["messages"] = [_message_to_dict(row) for row in message_rows]
        session["message_count"] = len(session["messages"])
    return session


def delete_session(
    conn: sqlite3.Connection,
    profile_id: str,
    session_id: str,
    *,
    commit: bool = True,
) -> dict:
    session = _session_row(conn, profile_id, session_id)
    conn.execute("DELETE FROM ai_chat_messages WHERE session_id = ?", (session_id,))
    conn.execute("DELETE FROM ai_chat_sessions WHERE id = ?", (session_id,))
    if commit:
        conn.commit()
    return {"deleted": session["id"], "title": session["title"]}


def clear_sessions(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    commit: bool = True,
) -> dict:
    count = conn.execute(
        "SELECT COUNT(*) FROM ai_chat_sessions WHERE profile_id = ?",
        (profile_id,),
    ).fetchone()[0]
    conn.execute(
        """
        DELETE FROM ai_chat_messages WHERE session_id IN (
            SELECT id FROM ai_chat_sessions WHERE profile_id = ?
        )
        """,
        (profile_id,),
    )
    conn.execute("DELETE FROM ai_chat_sessions WHERE profile_id = ?", (profile_id,))
    if commit:
        conn.commit()
    return {"deleted": count}


def _session_row(
    conn: sqlite3.Connection, profile_id: str, session_id: str
) -> sqlite3.Row:
    row = conn.execute(
        "SELECT * FROM ai_chat_sessions WHERE id = ? AND profile_id = ?",
        (session_id, profile_id),
    ).fetchone()
    if row is None:
        raise AppError(
            "chat session not found for the active profile",
            code="not_found",
            details={"session_id": session_id},
        )
    return row


def _session_to_dict(row: sqlite3.Row) -> dict:
    payload = {
        "id": row["id"],
        "workspace_id": row["workspace_id"],
        "profile_id": row["profile_id"],
        "title": row["title"],
        "provider": row["provider"],
        "model": row["model"],
        "created_at": row["created_at"],
        "updated_at": row["updated_at"],
    }
    keys = row.keys()
    if "message_count" in keys:
        payload["message_count"] = row["message_count"]
    return payload


def _message_to_dict(row: sqlite3.Row) -> dict:
    return {
        "id": row["id"],
        "seq": row["seq"],
        "role": row["role"],
        "content": row["content"],
        "tool_calls": json.loads(row["tool_calls_json"]) if row["tool_calls_json"] else None,
        "provenance": json.loads(row["provenance_json"]) if row["provenance_json"] else None,
        "finish_reason": row["finish_reason"],
        "created_at": row["created_at"],
    }
