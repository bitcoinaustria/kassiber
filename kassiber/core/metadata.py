from __future__ import annotations

import json
import os
import sqlite3
import uuid
from dataclasses import dataclass
from typing import Any, Callable, Mapping

from ..errors import AppError
from ..importers import load_bip329_file
from ..msat import msat_to_btc

DEFAULT_RECORDS_LIMIT = 100
MAX_RECORDS_LIMIT = 1000

ScopeResolver = Callable[[sqlite3.Connection, str | None, str | None], tuple[Mapping[str, Any], Mapping[str, Any]]]
WalletResolver = Callable[[sqlite3.Connection, str, str], Mapping[str, Any]]
TagResolver = Callable[[sqlite3.Connection, str, str], Mapping[str, Any]]
TransactionResolver = Callable[..., Mapping[str, Any]]
NormalizeCode = Callable[[Any], str]
NowIso = Callable[[], str]
InvalidateJournals = Callable[[sqlite3.Connection, str], None]
ParseIsoDateTime = Callable[[str, str], Any]
IsoFormatter = Callable[[Any], str]
EncodeCursor = Callable[[Mapping[str, Any]], str]
DecodeCursor = Callable[[str | None], Mapping[str, str] | None]


@dataclass(frozen=True)
class MetadataHooks:
    resolve_scope: ScopeResolver
    resolve_wallet: WalletResolver
    resolve_tag: TagResolver
    resolve_transaction: TransactionResolver
    normalize_code: NormalizeCode
    now_iso: NowIso
    invalidate_journals: InvalidateJournals
    parse_iso_datetime: ParseIsoDateTime
    iso_z: IsoFormatter
    encode_cursor: EncodeCursor
    decode_cursor: DecodeCursor


def ensure_tag_row(conn, workspace_id, profile_id, code, label, hooks: MetadataHooks):
    normalized_code = hooks.normalize_code(code)
    existing = conn.execute(
        "SELECT * FROM tags WHERE profile_id = ? AND code = ?",
        (profile_id, normalized_code),
    ).fetchone()
    if existing:
        return existing, False
    tag_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO tags(id, workspace_id, profile_id, code, label, created_at)
        VALUES(?, ?, ?, ?, ?, ?)
        """,
        (tag_id, workspace_id, profile_id, normalized_code, label, hooks.now_iso()),
    )
    return conn.execute("SELECT * FROM tags WHERE id = ?", (tag_id,)).fetchone(), True


def set_transaction_note(conn, workspace_ref, profile_ref, tx_ref, note, hooks: MetadataHooks):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    tx = hooks.resolve_transaction(conn, profile["id"], tx_ref)
    conn.execute("UPDATE transactions SET note = ? WHERE id = ?", (note, tx["id"]))
    hooks.invalidate_journals(conn, profile["id"])
    conn.commit()
    return {"transaction_id": tx["id"], "note": note}


def clear_transaction_note(conn, workspace_ref, profile_ref, tx_ref, hooks: MetadataHooks):
    return set_transaction_note(conn, workspace_ref, profile_ref, tx_ref, None, hooks)


def set_transaction_excluded(conn, workspace_ref, profile_ref, tx_ref, excluded, hooks: MetadataHooks):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    tx = hooks.resolve_transaction(conn, profile["id"], tx_ref)
    conn.execute("UPDATE transactions SET excluded = ? WHERE id = ?", (1 if excluded else 0, tx["id"]))
    hooks.invalidate_journals(conn, profile["id"])
    conn.commit()
    return {"transaction_id": tx["id"], "excluded": bool(excluded)}


def create_tag(conn, workspace_ref, profile_ref, code, label, hooks: MetadataHooks):
    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    tag_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO tags(id, workspace_id, profile_id, code, label, created_at)
        VALUES(?, ?, ?, ?, ?, ?)
        """,
        (tag_id, workspace["id"], profile["id"], hooks.normalize_code(code), label, hooks.now_iso()),
    )
    conn.commit()
    return conn.execute("SELECT * FROM tags WHERE id = ?", (tag_id,)).fetchone()


def list_tags(conn, workspace_ref, profile_ref, hooks: MetadataHooks):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        "SELECT id, code, label, created_at FROM tags WHERE profile_id = ? ORDER BY code ASC",
        (profile["id"],),
    ).fetchall()
    return [dict(row) for row in rows]


def add_tag_to_transaction(conn, workspace_ref, profile_ref, tx_ref, tag_ref, hooks: MetadataHooks):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    tx = hooks.resolve_transaction(conn, profile["id"], tx_ref)
    tag = hooks.resolve_tag(conn, profile["id"], tag_ref)
    conn.execute(
        "INSERT OR IGNORE INTO transaction_tags(transaction_id, tag_id) VALUES(?, ?)",
        (tx["id"], tag["id"]),
    )
    hooks.invalidate_journals(conn, profile["id"])
    conn.commit()
    return {"transaction_id": tx["id"], "tag": tag["code"], "status": "added"}


def remove_tag_from_transaction(conn, workspace_ref, profile_ref, tx_ref, tag_ref, hooks: MetadataHooks):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    tx = hooks.resolve_transaction(conn, profile["id"], tx_ref)
    tag = hooks.resolve_tag(conn, profile["id"], tag_ref)
    conn.execute(
        "DELETE FROM transaction_tags WHERE transaction_id = ? AND tag_id = ?",
        (tx["id"], tag["id"]),
    )
    hooks.invalidate_journals(conn, profile["id"])
    conn.commit()
    return {"transaction_id": tx["id"], "tag": tag["code"], "status": "removed"}


def _tags_for_transaction(conn, tx_id):
    rows = conn.execute(
        """
        SELECT t.code, t.label
        FROM transaction_tags tt
        JOIN tags t ON t.id = tt.tag_id
        WHERE tt.transaction_id = ?
        ORDER BY t.code ASC
        """,
        (tx_id,),
    ).fetchall()
    return [{"code": row["code"], "label": row["label"]} for row in rows]


def get_transaction_record(conn, workspace_ref, profile_ref, tx_ref, hooks: MetadataHooks):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    tx = hooks.resolve_transaction(conn, profile["id"], tx_ref)
    wallet = conn.execute(
        "SELECT id, label FROM wallets WHERE id = ?",
        (tx["wallet_id"],),
    ).fetchone()
    return {
        "transaction_id": tx["id"],
        "external_id": tx["external_id"] or "",
        "occurred_at": tx["occurred_at"],
        "direction": tx["direction"],
        "asset": tx["asset"],
        "amount": float(msat_to_btc(tx["amount"])),
        "amount_msat": int(tx["amount"]),
        "fee": float(msat_to_btc(tx["fee"])),
        "fee_msat": int(tx["fee"]),
        "counterparty": tx["counterparty"] or "",
        "wallet_id": wallet["id"] if wallet else "",
        "wallet_label": wallet["label"] if wallet else "",
        "note": tx["note"] or "",
        "excluded": bool(tx["excluded"]),
        "tags": _tags_for_transaction(conn, tx["id"]),
    }


def list_transaction_records(
    conn,
    workspace_ref,
    profile_ref,
    hooks: MetadataHooks,
    wallet=None,
    tag=None,
    has_note=None,
    excluded=None,
    start=None,
    end=None,
    cursor=None,
    limit=None,
):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    effective_limit = limit if limit is not None else DEFAULT_RECORDS_LIMIT
    if effective_limit <= 0:
        raise AppError("--limit must be positive", code="validation")
    if effective_limit > MAX_RECORDS_LIMIT:
        raise AppError(
            f"--limit cannot exceed {MAX_RECORDS_LIMIT}",
            code="validation",
            hint=f"Use cursor-based pagination instead of larger limits; max page size is {MAX_RECORDS_LIMIT}.",
        )

    where = ["t.profile_id = ?"]
    params = [profile["id"]]
    start_ts = hooks.iso_z(hooks.parse_iso_datetime(start, "start")) if start else None
    end_ts = hooks.iso_z(hooks.parse_iso_datetime(end, "end")) if end else None

    if wallet:
        wallet_row = hooks.resolve_wallet(conn, profile["id"], wallet)
        where.append("t.wallet_id = ?")
        params.append(wallet_row["id"])
    if tag:
        tag_row = hooks.resolve_tag(conn, profile["id"], tag)
        where.append("EXISTS (SELECT 1 FROM transaction_tags tt WHERE tt.transaction_id = t.id AND tt.tag_id = ?)")
        params.append(tag_row["id"])
    if has_note is True:
        where.append("t.note IS NOT NULL AND t.note != ''")
    elif has_note is False:
        where.append("(t.note IS NULL OR t.note = '')")
    if excluded is True:
        where.append("t.excluded = 1")
    elif excluded is False:
        where.append("t.excluded = 0")
    if start_ts:
        where.append("t.occurred_at >= ?")
        params.append(start_ts)
    if end_ts:
        where.append("t.occurred_at <= ?")
        params.append(end_ts)

    cursor_data = hooks.decode_cursor(cursor)
    if cursor_data:
        where.append(
            "(t.occurred_at < ? OR "
            "(t.occurred_at = ? AND t.created_at < ?) OR "
            "(t.occurred_at = ? AND t.created_at = ? AND t.id < ?))"
        )
        params.extend(
            [
                cursor_data["occurred_at"],
                cursor_data["occurred_at"],
                cursor_data["created_at"],
                cursor_data["occurred_at"],
                cursor_data["created_at"],
                cursor_data["id"],
            ]
        )

    query = f"""
        SELECT
            t.id,
            t.occurred_at,
            t.created_at,
            t.external_id,
            t.direction,
            t.asset,
            t.amount,
            t.fee,
            t.counterparty,
            t.note,
            t.excluded,
            w.id AS wallet_id,
            w.label AS wallet_label
        FROM transactions t
        LEFT JOIN wallets w ON w.id = t.wallet_id
        WHERE {' AND '.join(where)}
        ORDER BY t.occurred_at DESC, t.created_at DESC, t.id DESC
        LIMIT ?
    """
    params.append(effective_limit + 1)
    rows = conn.execute(query, params).fetchall()

    has_more = len(rows) > effective_limit
    page = rows[:effective_limit]
    records = []
    for row in page:
        records.append(
            {
                "transaction_id": row["id"],
                "external_id": row["external_id"] or "",
                "occurred_at": row["occurred_at"],
                "direction": row["direction"],
                "asset": row["asset"],
                "amount": float(msat_to_btc(row["amount"])),
                "amount_msat": int(row["amount"]),
                "fee": float(msat_to_btc(row["fee"])),
                "fee_msat": int(row["fee"]),
                "counterparty": row["counterparty"] or "",
                "wallet_id": row["wallet_id"] or "",
                "wallet_label": row["wallet_label"] or "",
                "note": row["note"] or "",
                "excluded": bool(row["excluded"]),
                "tags": _tags_for_transaction(conn, row["id"]),
            }
        )
    next_cursor = hooks.encode_cursor(page[-1]) if has_more and page else None
    return {
        "records": records,
        "next_cursor": next_cursor,
        "has_more": has_more,
        "limit": effective_limit,
    }


def import_bip329_labels(conn, workspace_ref, profile_ref, file_path, hooks: MetadataHooks, wallet_ref=None):
    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    wallet = hooks.resolve_wallet(conn, profile["id"], wallet_ref) if wallet_ref else None
    records = load_bip329_file(file_path)
    imported = 0
    updated = 0
    transaction_tags_added = 0
    transaction_tags_created = 0
    for record in records:
        existing = conn.execute(
            """
            SELECT id
            FROM bip329_labels
            WHERE profile_id = ?
              AND COALESCE(wallet_id, '') = ?
              AND record_type = ?
              AND ref = ?
              AND COALESCE(label, '') = ?
              AND COALESCE(origin, '') = ?
            LIMIT 1
            """,
            (
                profile["id"],
                wallet["id"] if wallet else "",
                record["type"],
                record["ref"],
                record["label"] or "",
                record["origin"] or "",
            ),
        ).fetchone()
        if existing:
            conn.execute(
                """
                UPDATE bip329_labels
                SET spendable = ?, data_json = ?
                WHERE id = ?
                """,
                (
                    None if record["spendable"] is None else (1 if record["spendable"] else 0),
                    json.dumps(record["data"], sort_keys=True),
                    existing["id"],
                ),
            )
            updated += 1
        else:
            conn.execute(
                """
                INSERT INTO bip329_labels(
                    id, workspace_id, profile_id, wallet_id, record_type, ref,
                    label, origin, spendable, data_json, created_at
                ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    str(uuid.uuid4()),
                    workspace["id"],
                    profile["id"],
                    wallet["id"] if wallet else None,
                    record["type"],
                    record["ref"],
                    record["label"],
                    record["origin"],
                    None if record["spendable"] is None else (1 if record["spendable"] else 0),
                    json.dumps(record["data"], sort_keys=True),
                    hooks.now_iso(),
                ),
            )
            imported += 1
        if record["type"] == "tx" and record["label"]:
            query = """
                SELECT id
                FROM transactions
                WHERE profile_id = ? AND external_id = ?
            """
            params = [profile["id"], record["ref"]]
            if wallet:
                query += " AND wallet_id = ?"
                params.append(wallet["id"])
            tx_rows = conn.execute(query, params).fetchall()
            for tx in tx_rows:
                tag, created = ensure_tag_row(
                    conn,
                    profile["workspace_id"],
                    profile["id"],
                    record["label"],
                    record["label"],
                    hooks,
                )
                if created:
                    transaction_tags_created += 1
                before = conn.total_changes
                conn.execute(
                    "INSERT OR IGNORE INTO transaction_tags(transaction_id, tag_id) VALUES(?, ?)",
                    (tx["id"], tag["id"]),
                )
                if conn.total_changes > before:
                    transaction_tags_added += 1
    if records:
        hooks.invalidate_journals(conn, profile["id"])
    conn.commit()
    return {
        "file": os.path.abspath(file_path),
        "imported": imported,
        "updated": updated,
        "records": len(records),
        "transaction_tags_created": transaction_tags_created,
        "transaction_tags_added": transaction_tags_added,
    }


def list_bip329_labels(conn, workspace_ref, profile_ref, hooks: MetadataHooks, wallet_ref=None, limit=None):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    effective_limit = limit if limit is not None else DEFAULT_RECORDS_LIMIT
    if effective_limit <= 0:
        raise AppError("--limit must be positive", code="validation")
    if effective_limit > MAX_RECORDS_LIMIT:
        raise AppError(
            f"--limit cannot exceed {MAX_RECORDS_LIMIT}",
            code="validation",
            hint=f"Use a smaller --limit; max page size is {MAX_RECORDS_LIMIT}.",
        )
    wallet = hooks.resolve_wallet(conn, profile["id"], wallet_ref) if wallet_ref else None
    wallet_clause = "AND wallet_id = ?" if wallet else ""
    params = [profile["id"]]
    if wallet:
        params.append(wallet["id"])
    params.append(effective_limit)
    rows = conn.execute(
        f"""
        SELECT
            record_type AS type,
            ref,
            COALESCE(label, '') AS label,
            COALESCE(origin, '') AS origin,
            CASE
                WHEN spendable IS NULL THEN ''
                WHEN spendable = 1 THEN 'true'
                ELSE 'false'
            END AS spendable,
            created_at
        FROM bip329_labels
        WHERE profile_id = ? {wallet_clause}
        ORDER BY created_at DESC
        LIMIT ?
        """,
        params,
    ).fetchall()
    return [dict(row) for row in rows]


def export_bip329_labels(conn, workspace_ref, profile_ref, file_path, hooks: MetadataHooks, wallet_ref=None):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    wallet = hooks.resolve_wallet(conn, profile["id"], wallet_ref) if wallet_ref else None
    wallet_clause = "AND wallet_id = ?" if wallet else ""
    params = [profile["id"]]
    if wallet:
        params.append(wallet["id"])
    rows = conn.execute(
        f"""
        SELECT record_type, ref, label, origin, spendable, data_json
        FROM bip329_labels
        WHERE profile_id = ? {wallet_clause}
        ORDER BY created_at ASC
        """,
        params,
    ).fetchall()
    output_lines = []
    for row in rows:
        payload = {"type": row["record_type"], "ref": row["ref"]}
        if row["label"] is not None:
            payload["label"] = row["label"]
        if row["origin"] is not None:
            payload["origin"] = row["origin"]
        if row["spendable"] is not None:
            payload["spendable"] = bool(row["spendable"])
        payload.update(json.loads(row["data_json"] or "{}"))
        output_lines.append(json.dumps(payload, ensure_ascii=True))
    export_path = os.path.abspath(file_path)
    with open(export_path, "w", encoding="utf-8") as handle:
        if output_lines:
            handle.write("\n".join(output_lines) + "\n")
    return {
        "file": export_path,
        "exported": len(output_lines),
    }


__all__ = [
    "DEFAULT_RECORDS_LIMIT",
    "MAX_RECORDS_LIMIT",
    "MetadataHooks",
    "add_tag_to_transaction",
    "clear_transaction_note",
    "create_tag",
    "ensure_tag_row",
    "export_bip329_labels",
    "get_transaction_record",
    "import_bip329_labels",
    "list_bip329_labels",
    "list_tags",
    "list_transaction_records",
    "remove_tag_from_transaction",
    "set_transaction_excluded",
    "set_transaction_note",
]
