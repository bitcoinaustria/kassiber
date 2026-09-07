"""Line-backed open items and evidence-backed immutable manual schedules.

Schedules are reviewed inputs, never an implicit tax/depreciation calculator.
"""
from __future__ import annotations

import json
import hashlib
import uuid
from typing import Any

from kassiber.errors import AppError
from .ledger import atomic, require_book, strict_minor
from .bank import iso_date, posted_line, signed_minor, require_open_interval, reconcile_statement
from .evidence import bounded_text, require_evidence

SCHEDULE_KINDS = frozenset({"asset", "depreciation", "accrual", "tax", "restricted_fund", "valuation", "carryforward"})


def ensure_schema(conn: Any) -> None:
    for statement in (
        """CREATE TABLE IF NOT EXISTS gl_open_items (
          id TEXT PRIMARY KEY,profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE RESTRICT,
          direction TEXT NOT NULL CHECK(direction IN ('receivable','payable')),document_ref TEXT NOT NULL,
          origin_line_id TEXT NOT NULL REFERENCES gl_lines(id),evidence_id TEXT NOT NULL,
          amount_minor INTEGER NOT NULL CHECK(typeof(amount_minor)='integer' AND amount_minor>0),due_date TEXT NOT NULL,
          UNIQUE(profile_id,origin_line_id),UNIQUE(profile_id,id),
          FOREIGN KEY(profile_id,evidence_id) REFERENCES gl_evidence(profile_id,id))""",
        """CREATE TABLE IF NOT EXISTS gl_open_item_allocations (
          id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,item_id TEXT NOT NULL,line_id TEXT NOT NULL REFERENCES gl_lines(id),
          amount_minor INTEGER NOT NULL CHECK(typeof(amount_minor)='integer' AND amount_minor>0),idempotency_key TEXT NOT NULL,
          UNIQUE(profile_id,idempotency_key),FOREIGN KEY(profile_id,item_id) REFERENCES gl_open_items(profile_id,id))""",
        """CREATE TABLE IF NOT EXISTS gl_open_item_revisions (
          id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,item_id TEXT NOT NULL,
          revision INTEGER NOT NULL CHECK(typeof(revision)='integer' AND revision>0),
          previous_digest TEXT NOT NULL,payload_digest TEXT NOT NULL,
          document_ref TEXT NOT NULL,due_date TEXT NOT NULL,effective_date TEXT NOT NULL,
          evidence_id TEXT NOT NULL,reason TEXT NOT NULL,idempotency_key TEXT NOT NULL,
          created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
          UNIQUE(item_id,revision),UNIQUE(profile_id,idempotency_key),
          FOREIGN KEY(profile_id,item_id) REFERENCES gl_open_items(profile_id,id),
          FOREIGN KEY(profile_id,evidence_id) REFERENCES gl_evidence(profile_id,id))""",
        """CREATE TABLE IF NOT EXISTS gl_schedules (
          id TEXT PRIMARY KEY,profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE RESTRICT,
          kind TEXT NOT NULL,label TEXT NOT NULL,UNIQUE(profile_id,id))""",
        """CREATE TABLE IF NOT EXISTS gl_schedule_revisions (
          id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,schedule_id TEXT NOT NULL,
          revision INTEGER NOT NULL CHECK(typeof(revision)='integer' AND revision>0),
          effective_date TEXT NOT NULL,evidence_id TEXT NOT NULL,entry_id TEXT REFERENCES gl_entries(id),
          payload_json TEXT NOT NULL,reason TEXT NOT NULL,
          created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
          UNIQUE(schedule_id,revision),FOREIGN KEY(profile_id,schedule_id) REFERENCES gl_schedules(profile_id,id),
          FOREIGN KEY(profile_id,evidence_id) REFERENCES gl_evidence(profile_id,id))""",
    ):
        conn.execute(statement)
    for table in ("gl_open_items", "gl_open_item_allocations", "gl_open_item_revisions", "gl_schedules", "gl_schedule_revisions"):
        for action in ("UPDATE", "DELETE"):
            conn.execute(f"""CREATE TRIGGER IF NOT EXISTS {table}_no_{action.lower()}
                BEFORE {action} ON {table} BEGIN SELECT RAISE(ABORT,'accounting_schedule_retained'); END""")
    for table, conflict in (
        ("gl_open_items", "id=NEW.id OR (profile_id=NEW.profile_id AND origin_line_id=NEW.origin_line_id)"),
        ("gl_open_item_allocations", "id=NEW.id OR (profile_id=NEW.profile_id AND idempotency_key=NEW.idempotency_key)"),
        ("gl_open_item_revisions", "id=NEW.id OR (item_id=NEW.item_id AND revision=NEW.revision) OR (profile_id=NEW.profile_id AND idempotency_key=NEW.idempotency_key)"),
        ("gl_schedules", "id=NEW.id"),
        ("gl_schedule_revisions", "id=NEW.id OR (schedule_id=NEW.schedule_id AND revision=NEW.revision)"),
    ):
        conn.execute(f"""CREATE TRIGGER IF NOT EXISTS {table}_no_replace BEFORE INSERT ON {table}
            WHEN EXISTS (SELECT 1 FROM {table} WHERE {conflict})
            BEGIN SELECT RAISE(ABORT,'accounting_schedule_retained'); END""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_open_item_origin_scope BEFORE INSERT ON gl_open_items
        WHEN NOT EXISTS (SELECT 1 FROM gl_lines l JOIN gl_entries e ON e.id=l.entry_id AND e.profile_id=l.profile_id
          WHERE l.id=NEW.origin_line_id AND l.profile_id=NEW.profile_id AND e.status='posted'
          AND NOT EXISTS (SELECT 1 FROM gl_entries reversal WHERE reversal.reversal_of=e.id AND reversal.status='posted')
          AND ((NEW.direction='receivable' AND l.account_kind='asset' AND l.debit_minor=NEW.amount_minor)
            OR (NEW.direction='payable' AND l.account_kind='liability' AND l.credit_minor=NEW.amount_minor)))
        BEGIN SELECT RAISE(ABORT,'accounting_schedule_scope'); END""")
    conn.execute('DROP TRIGGER IF EXISTS gl_open_item_active_document')
    conn.execute("""CREATE TRIGGER gl_open_item_active_document BEFORE INSERT ON gl_open_items
        WHEN EXISTS (SELECT 1 FROM gl_open_items i WHERE i.profile_id=NEW.profile_id
          AND i.direction=NEW.direction AND COALESCE((SELECT r.document_ref FROM gl_open_item_revisions r
            WHERE r.item_id=i.id AND r.profile_id=i.profile_id ORDER BY r.revision DESC LIMIT 1),i.document_ref)=NEW.document_ref AND NOT EXISTS
          (SELECT 1 FROM gl_open_item_voids v WHERE v.item_id=i.id))
        BEGIN SELECT RAISE(ABORT,'accounting_open_item_duplicate'); END""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_open_item_revision_scope BEFORE INSERT ON gl_open_item_revisions
        WHEN NOT EXISTS (SELECT 1 FROM gl_open_items i JOIN gl_lines l ON l.id=i.origin_line_id AND l.profile_id=i.profile_id
          JOIN gl_entries e ON e.id=l.entry_id AND e.profile_id=l.profile_id WHERE i.id=NEW.item_id AND i.profile_id=NEW.profile_id
          AND e.entry_date<=NEW.effective_date AND NOT EXISTS(SELECT 1 FROM gl_open_item_voids v WHERE v.item_id=i.id))
        OR NEW.revision!=COALESCE((SELECT MAX(revision)+1 FROM gl_open_item_revisions WHERE item_id=NEW.item_id),1)
        OR EXISTS(SELECT 1 FROM gl_open_item_revisions r WHERE r.item_id=NEW.item_id AND
          (r.effective_date>NEW.effective_date OR (r.revision=NEW.revision-1 AND r.payload_digest!=NEW.previous_digest)))
        OR EXISTS(SELECT 1 FROM gl_periods p WHERE p.profile_id=NEW.profile_id AND p.state!='open' AND p.end_date>=NEW.effective_date)
        OR EXISTS(SELECT 1 FROM gl_open_items i WHERE i.profile_id=NEW.profile_id AND i.id!=NEW.item_id
          AND i.direction=(SELECT direction FROM gl_open_items WHERE id=NEW.item_id)
          AND COALESCE((SELECT r.document_ref FROM gl_open_item_revisions r WHERE r.item_id=i.id AND r.profile_id=i.profile_id
              ORDER BY r.revision DESC LIMIT 1),i.document_ref)=NEW.document_ref
          AND NOT EXISTS(SELECT 1 FROM gl_open_item_voids v WHERE v.item_id=i.id))
        BEGIN SELECT RAISE(ABORT,'accounting_item_revision_integrity'); END""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_settlement_scope BEFORE INSERT ON gl_open_item_allocations
        WHEN NOT EXISTS (SELECT 1 FROM gl_open_items i JOIN gl_lines o ON o.id=i.origin_line_id
          AND o.profile_id=i.profile_id JOIN gl_lines l ON l.id=NEW.line_id AND l.profile_id=i.profile_id
          JOIN gl_entries e ON e.id=l.entry_id AND e.profile_id=l.profile_id
          WHERE i.id=NEW.item_id AND i.profile_id=NEW.profile_id AND e.status='posted'
            AND NOT EXISTS (SELECT 1 FROM gl_entries reversal WHERE reversal.reversal_of=e.id AND reversal.status='posted')
            AND l.account_code=o.account_code AND ((i.direction='receivable' AND l.credit_minor>0)
              OR (i.direction='payable' AND l.debit_minor>0)) AND NOT EXISTS
            (SELECT 1 FROM gl_open_item_voids v WHERE v.item_id=i.id))
        BEGIN SELECT RAISE(ABORT,'accounting_schedule_scope'); END""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_settlement_budget BEFORE INSERT ON gl_open_item_allocations
        WHEN NEW.amount_minor + COALESCE((SELECT SUM(a.amount_minor) FROM gl_open_item_allocations a
          WHERE a.item_id=NEW.item_id AND NOT EXISTS
            (SELECT 1 FROM gl_open_item_allocation_voids v WHERE v.allocation_id=a.id)),0)
          > (SELECT amount_minor FROM gl_open_items WHERE id=NEW.item_id)
        OR NEW.amount_minor + COALESCE((SELECT SUM(a.amount_minor) FROM gl_open_item_allocations a
          WHERE a.line_id=NEW.line_id AND NOT EXISTS
            (SELECT 1 FROM gl_open_item_allocation_voids v WHERE v.allocation_id=a.id)),0)
          > (SELECT debit_minor+credit_minor FROM gl_lines WHERE id=NEW.line_id)
        BEGIN SELECT RAISE(ABORT,'accounting_allocation_exceeded'); END""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_schedule_entry_scope BEFORE INSERT ON gl_schedule_revisions
        WHEN NEW.entry_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM gl_entries
          WHERE id=NEW.entry_id AND profile_id=NEW.profile_id AND status='posted')
        BEGIN SELECT RAISE(ABORT,'accounting_schedule_scope'); END""")
    for table, key, target in (("gl_open_item_allocation_voids", "allocation_id", "gl_open_item_allocations"),
                               ("gl_open_item_voids", "item_id", "gl_open_items")):
        conn.execute(f"""CREATE TABLE IF NOT EXISTS {table} (
            {key} TEXT PRIMARY KEY REFERENCES {target}(id),profile_id TEXT NOT NULL,
            reason TEXT NOT NULL,idempotency_key TEXT NOT NULL,UNIQUE(profile_id,idempotency_key))""")
        for action in ("UPDATE", "DELETE"):
            conn.execute(f"""CREATE TRIGGER IF NOT EXISTS {table}_no_{action.lower()} BEFORE {action} ON {table}
                BEGIN SELECT RAISE(ABORT,'accounting_schedule_retained'); END""")
        conn.execute(f"""CREATE TRIGGER IF NOT EXISTS {table}_no_replace BEFORE INSERT ON {table}
            WHEN EXISTS (SELECT 1 FROM {table} WHERE {key}=NEW.{key}
                OR (profile_id=NEW.profile_id AND idempotency_key=NEW.idempotency_key))
            BEGIN SELECT RAISE(ABORT,'accounting_schedule_retained'); END""")
        conn.execute(f"""CREATE TRIGGER IF NOT EXISTS {table}_scope BEFORE INSERT ON {table}
            WHEN NOT EXISTS (SELECT 1 FROM {target} WHERE id=NEW.{key} AND profile_id=NEW.profile_id)
            BEGIN SELECT RAISE(ABORT,'accounting_schedule_scope'); END""")


def create_open_item(conn: Any, profile_id: str, *, direction: str, document_ref: str,
                     origin_line_id: str, evidence_id: str, due_date: str) -> dict:
    bounded_text(document_ref, "document_ref")
    iso_date(due_date)
    if direction not in ("receivable", "payable"):
        raise AppError("Invalid open-item direction", code="accounting_invalid_input")
    with atomic(conn):
        require_book(conn, profile_id)
        require_evidence(conn, profile_id, evidence_id)
        line = posted_line(conn, profile_id, origin_line_id)
        require_open_interval(conn, profile_id, line["entry_date"], line["entry_date"])
        expected = "asset" if direction == "receivable" else "liability"
        amount = line["debit_minor"] - line["credit_minor"]
        if direction == "payable":
            amount = -amount
        if line["account_kind"] != expected or amount <= 0:
            raise AppError("Open item requires an originating debit asset or credit liability line",
                           code="accounting_open_item_origin")
        if conn.execute("""SELECT 1 FROM gl_open_items i WHERE profile_id=? AND
            (origin_line_id=? OR (direction=? AND COALESCE((SELECT r.document_ref FROM gl_open_item_revisions r
              WHERE r.item_id=i.id AND r.profile_id=i.profile_id ORDER BY r.revision DESC LIMIT 1),i.document_ref)=? AND NOT EXISTS
             (SELECT 1 FROM gl_open_item_voids v WHERE v.item_id=i.id)))""",
            (profile_id, origin_line_id, direction, document_ref)).fetchone():
            raise AppError("Open item already recorded", code="accounting_open_item_duplicate")
        identifier = str(uuid.uuid4())
        conn.execute("INSERT INTO gl_open_items VALUES (?,?,?,?,?,?,?,?)", (identifier, profile_id,
            direction, document_ref, origin_line_id, evidence_id, amount, due_date))
        conn.execute("UPDATE gl_books SET revision=revision+1 WHERE profile_id=?", (profile_id,))
        return get_open_item(conn, profile_id, identifier)


def get_open_item(conn: Any, profile_id: str, item_id: str) -> dict:
    require_book(conn, profile_id)
    row = conn.execute("""SELECT i.*,i.amount_minor-COALESCE((SELECT SUM(a.amount_minor)
        FROM gl_open_item_allocations a WHERE a.item_id=i.id AND a.profile_id=i.profile_id
          AND NOT EXISTS (SELECT 1 FROM gl_open_item_allocation_voids v WHERE v.allocation_id=a.id)),0)
        AS remaining_minor,EXISTS(SELECT 1 FROM gl_open_item_voids v WHERE v.item_id=i.id) AS voided
        FROM gl_open_items i WHERE i.profile_id=? AND i.id=?""", (profile_id, item_id)).fetchone()
    if row is None:
        raise AppError("Open item not found in this book", code="accounting_open_item_not_found")
    result = dict(row)
    origin = conn.execute("SELECT account_code FROM gl_lines WHERE id=? AND profile_id=?",
                          (result["origin_line_id"], profile_id)).fetchone()
    if origin is None:
        raise AppError("Open item origin is outside the book", code="accounting_allocation_integrity")
    result["account_code"] = origin["account_code"]
    result["allocations"] = [dict(a) for a in conn.execute("""SELECT a.*,EXISTS
        (SELECT 1 FROM gl_open_item_allocation_voids v WHERE v.allocation_id=a.id) AS voided
        FROM gl_open_item_allocations a WHERE a.profile_id=? AND a.item_id=? ORDER BY a.id""", (profile_id, item_id))]
    invalid = conn.execute("""SELECT 1 FROM gl_open_item_allocations a JOIN gl_open_items i ON i.id=a.item_id
        LEFT JOIN gl_lines l ON l.id=a.line_id LEFT JOIN gl_entries e ON e.id=l.entry_id
        LEFT JOIN gl_lines o ON o.id=i.origin_line_id WHERE i.id=?
        AND NOT EXISTS (SELECT 1 FROM gl_open_item_allocation_voids v WHERE v.allocation_id=a.id)
        AND (a.profile_id!=i.profile_id OR l.profile_id!=i.profile_id OR e.profile_id!=i.profile_id
          OR l.id IS NULL OR e.status!='posted' OR l.account_code!=o.account_code
          OR (i.direction='receivable' AND l.credit_minor=0) OR (i.direction='payable' AND l.debit_minor=0)
          OR typeof(a.amount_minor)!='integer' OR a.amount_minor<=0)""", (item_id,)).fetchone()
    if invalid or result["remaining_minor"] < 0:
        raise AppError("Open item allocation integrity failed", code="accounting_allocation_integrity")
    result.update(_item_metadata(conn, profile_id, dict(row)))
    return result


def _metadata_digest(payload: dict) -> str:
    return hashlib.sha256(json.dumps(payload, sort_keys=True, separators=(',', ':'), allow_nan=False).encode()).hexdigest()


def _item_metadata(conn: Any, profile_id: str, item: dict, *, as_of: str | None = None) -> dict:
    base = {key: item[key] for key in ('id', 'profile_id', 'document_ref', 'due_date', 'evidence_id')}
    digest = _metadata_digest(base)
    current = dict(document_ref=item['document_ref'], due_date=item['due_date'], evidence_id=item['evidence_id'],
                   metadata_revision=0, metadata_digest=digest, metadata_effective_date=None, metadata_reason=None)
    previous_date = ''
    for index, row in enumerate(conn.execute('SELECT * FROM gl_open_item_revisions WHERE profile_id=? AND item_id=? ORDER BY revision', (profile_id, item['id'])), start=1):
        record = dict(row)
        payload = {key: record[key] for key in ('profile_id','item_id','revision','previous_digest','document_ref','due_date','effective_date','evidence_id','reason')}
        if (record['revision'] != index or record['effective_date'] < previous_date
            or record['previous_digest'] != digest or record['payload_digest'] != _metadata_digest(payload)):
            raise AppError('Open item metadata integrity failed', code='accounting_item_revision_integrity')
        require_evidence(conn, profile_id, record['evidence_id'])
        previous_date = record['effective_date']
        digest = record['payload_digest']
        if as_of is None or record['effective_date'] <= as_of:
            current = dict(document_ref=record['document_ref'], due_date=record['due_date'], evidence_id=record['evidence_id'],
                metadata_revision=record['revision'], metadata_digest=digest,
                metadata_effective_date=record['effective_date'], metadata_reason=record['reason'])
    return current


def revise_open_item(conn: Any, profile_id: str, *, item_id: str, expected_revision: int, expected_digest: str,
                     document_ref: str, due_date: str, effective_date: str, evidence_id: str,
                     reason: str, idempotency_key: str) -> dict:
    if type(expected_revision) is not int or expected_revision < 0:
        raise AppError('Invalid metadata revision', code='accounting_invalid_input')
    bounded_text(document_ref, 'document_ref')
    bounded_text(reason, 'reason', 2000)
    bounded_text(idempotency_key, 'idempotency_key', 128)
    bounded_text(expected_digest, 'expected_digest', 64)
    iso_date(due_date)
    iso_date(effective_date)
    payload = dict(profile_id=profile_id, item_id=item_id, revision=expected_revision+1, previous_digest=expected_digest,
                   document_ref=document_ref, due_date=due_date, effective_date=effective_date, evidence_id=evidence_id, reason=reason)
    digest = _metadata_digest(payload)
    with atomic(conn):
        require_book(conn, profile_id)
        prior = conn.execute('SELECT * FROM gl_open_item_revisions WHERE profile_id=? AND idempotency_key=?', (profile_id,idempotency_key)).fetchone()
        if prior:
            if prior['payload_digest'] != digest:
                raise AppError('Metadata revision identity changed', code='accounting_idempotency_conflict')
            return dict(prior)
        item = get_open_item(conn, profile_id, item_id)
        if item['voided']:
            raise AppError('Open item was cancelled', code='accounting_open_item_voided')
        if item['metadata_revision'] != expected_revision or item['metadata_digest'] != expected_digest:
            raise AppError('Open item metadata changed; reload before reviewing', code='accounting_revision_conflict')
        origin = posted_line(conn, profile_id, item['origin_line_id'])
        if effective_date < max(origin['entry_date'], item['metadata_effective_date'] or origin['entry_date']):
            raise AppError('Metadata revisions must be dated from origin and in sequence', code='accounting_invalid_input')
        require_open_interval(conn, profile_id, effective_date, effective_date)
        require_evidence(conn, profile_id, evidence_id)
        if conn.execute('''SELECT 1 FROM gl_open_items i WHERE profile_id=? AND id!=? AND direction=?
            AND COALESCE((SELECT r.document_ref FROM gl_open_item_revisions r WHERE r.item_id=i.id AND r.profile_id=i.profile_id
              ORDER BY r.revision DESC LIMIT 1),i.document_ref)=?
            AND NOT EXISTS(SELECT 1 FROM gl_open_item_voids v WHERE v.item_id=i.id)''',
            (profile_id,item_id,item['direction'],document_ref)).fetchone():
            raise AppError('Open item document already recorded', code='accounting_open_item_duplicate')
        identifier = str(uuid.uuid4())
        conn.execute('''INSERT INTO gl_open_item_revisions
            (id,profile_id,item_id,revision,previous_digest,payload_digest,document_ref,due_date,effective_date,evidence_id,reason,idempotency_key)
            VALUES (?,?,?,?,?,?,?,?,?,?,?,?)''', (identifier,profile_id,item_id,expected_revision+1,expected_digest,digest,
                document_ref,due_date,effective_date,evidence_id,reason,idempotency_key))
        conn.execute('UPDATE gl_books SET revision=revision+1 WHERE profile_id=?', (profile_id,))
        return dict(conn.execute('SELECT * FROM gl_open_item_revisions WHERE id=?', (identifier,)).fetchone())


def list_open_items(conn: Any, profile_id: str, *, limit: int = 100) -> list[dict]:
    require_book(conn, profile_id)
    if type(limit) is not int or not 1 <= limit <= 500:
        raise AppError("Invalid limit", code="accounting_invalid_input")
    return [get_open_item(conn, profile_id, row[0]) for row in conn.execute(
        "SELECT id FROM gl_open_items WHERE profile_id=? ORDER BY due_date,id LIMIT ?", (profile_id, limit))]


def items_page(conn: Any, profile_id: str, *, limit: int = 100, cursor: str | None = None) -> dict:
    from .paging import records_page
    return records_page(conn, profile_id, "items", limit=limit, cursor=cursor,
                        materialize=lambda identifier: get_open_item(conn, profile_id, identifier))


def allocate_settlement(conn: Any, profile_id: str, *, item_id: str, settlement_line_id: str,
                        amount_minor: int, idempotency_key: str) -> dict:
    amount = strict_minor(amount_minor)
    bounded_text(idempotency_key, "idempotency_key", 128)
    if amount <= 0:
        raise AppError("Allocation must be positive", code="accounting_invalid_input")
    with atomic(conn):
        require_book(conn, profile_id)
        prior = conn.execute("SELECT * FROM gl_open_item_allocations WHERE profile_id=? AND idempotency_key=?",
                             (profile_id, idempotency_key)).fetchone()
        if prior:
            if (prior["item_id"], prior["line_id"], prior["amount_minor"]) != (item_id, settlement_line_id, amount):
                raise AppError("Settlement identity changed", code="accounting_idempotency_conflict")
            return dict(prior)
        item = get_open_item(conn, profile_id, item_id)
        if item["voided"]:
            raise AppError("Open item was cancelled", code="accounting_open_item_voided")
        origin = posted_line(conn, profile_id, item["origin_line_id"])
        line = posted_line(conn, profile_id, settlement_line_id)
        require_open_interval(conn, profile_id, line["entry_date"], line["entry_date"])
        capacity = line["credit_minor"] - line["debit_minor"]
        if item["direction"] == "payable":
            capacity = -capacity
        if line["account_code"] != origin["account_code"] or capacity <= 0:
            raise AppError("Settlement must reduce the same control account", code="accounting_settlement_direction")
        used = conn.execute("""SELECT COALESCE(SUM(amount_minor),0) FROM gl_open_item_allocations a
            WHERE profile_id=? AND line_id=? AND NOT EXISTS
            (SELECT 1 FROM gl_open_item_allocation_voids v WHERE v.allocation_id=a.id)""", (profile_id, settlement_line_id)).fetchone()[0]
        if amount > item["remaining_minor"] or used + amount > capacity:
            raise AppError("Settlement exceeds item or line remainder", code="accounting_allocation_exceeded")
        identifier = str(uuid.uuid4())
        conn.execute("INSERT INTO gl_open_item_allocations VALUES (?,?,?,?,?,?)",
            (identifier, profile_id, item_id, settlement_line_id, amount, idempotency_key))
        conn.execute("UPDATE gl_books SET revision=revision+1 WHERE profile_id=?", (profile_id,))
        return {"id": identifier, "item_id": item_id, "line_id": settlement_line_id,
                "amount_minor": amount, "remaining_minor": item["remaining_minor"] - amount}


def _payload(value: Any) -> str:
    """Only shallow bounded exact records; no floats, embedded documents or URLs fetched."""
    if not isinstance(value, dict) or not 1 <= len(value) <= 100:
        raise AppError("Schedule requires structured fields", code="accounting_schedule_payload")
    for key, item in value.items():
        bounded_text(key, "field name", 64)
        if type(item) not in (str, int, bool, type(None)):
            raise AppError("Schedule fields must be exact scalar values", code="accounting_schedule_payload")
        if isinstance(item, str) and (len(item) > 4000 or "\x00" in item):
            raise AppError("Schedule field too long", code="accounting_schedule_payload")
        if key.endswith("_minor") or type(item) is int:
            signed_minor(item)
    result = json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False)
    if len(result.encode()) > 32768:
        raise AppError("Schedule payload exceeds 32 KiB", code="accounting_schedule_payload")
    return result


def create_schedule(conn: Any, profile_id: str, *, kind: str, label: str,
                    effective_date: str, evidence_id: str, fields: dict,
                    reason: str, entry_id: str | None = None) -> dict:
    if kind not in SCHEDULE_KINDS:
        raise AppError("Unsupported schedule kind", code="accounting_schedule_kind")
    bounded_text(label, "label")
    with atomic(conn):
        require_book(conn, profile_id)
        identifier = str(uuid.uuid4())
        conn.execute("INSERT INTO gl_schedules VALUES (?,?,?,?)", (identifier, profile_id, kind, label))
        return revise_schedule(conn, profile_id, schedule_id=identifier, expected_revision=0,
            effective_date=effective_date, evidence_id=evidence_id, fields=fields, reason=reason, entry_id=entry_id)


def revise_schedule(conn: Any, profile_id: str, *, schedule_id: str, expected_revision: int,
                    effective_date: str, evidence_id: str, fields: dict,
                    reason: str, entry_id: str | None = None) -> dict:
    iso_date(effective_date)
    bounded_text(reason, "revision reason", 2000)
    payload = _payload(fields)
    if type(expected_revision) is not int or expected_revision < 0:
        raise AppError("Invalid expected revision", code="accounting_invalid_input")
    with atomic(conn):
        require_book(conn, profile_id)
        require_evidence(conn, profile_id, evidence_id)
        require_open_interval(conn, profile_id, effective_date, effective_date)
        if not conn.execute("SELECT 1 FROM gl_schedules WHERE id=? AND profile_id=?", (schedule_id, profile_id)).fetchone():
            raise AppError("Schedule not found in this book", code="accounting_schedule_not_found")
        current = conn.execute("SELECT COALESCE(MAX(revision),0) FROM gl_schedule_revisions WHERE schedule_id=? AND profile_id=?",
                               (schedule_id, profile_id)).fetchone()[0]
        if current != expected_revision:
            raise AppError("Schedule changed; refresh before revising", code="accounting_stale_revision")
        if entry_id is not None and not conn.execute("""SELECT 1 FROM gl_entries e
            WHERE id=? AND profile_id=? AND status='posted' AND NOT EXISTS
              (SELECT 1 FROM gl_entries reversal WHERE reversal.reversal_of=e.id AND reversal.status='posted')""",
                                                     (entry_id, profile_id)).fetchone():
            raise AppError("Schedule requires a posted entry in this book", code="accounting_posted_line_required")
        identifier = str(uuid.uuid4())
        conn.execute("""INSERT INTO gl_schedule_revisions
            (id,profile_id,schedule_id,revision,effective_date,evidence_id,entry_id,payload_json,reason)
            VALUES (?,?,?,?,?,?,?,?,?)""", (identifier, profile_id, schedule_id, current + 1,
            effective_date, evidence_id, entry_id, payload, reason))
        conn.execute("UPDATE gl_books SET revision=revision+1 WHERE profile_id=?", (profile_id,))
        return {"id": identifier, "schedule_id": schedule_id, "revision": current + 1,
                "effective_date": effective_date, "evidence_id": evidence_id, "entry_id": entry_id,
                "fields": fields, "reason": reason}


def list_schedules(conn: Any, profile_id: str, *, limit: int = 100) -> list[dict]:
    require_book(conn, profile_id)
    if type(limit) is not int or not 1 <= limit <= 500:
        raise AppError("Invalid limit", code="accounting_invalid_input")
    result = []
    for row in conn.execute("""SELECT s.id,s.kind,s.label,r.revision,r.effective_date,r.evidence_id,
        r.entry_id,r.payload_json,r.reason,(SELECT MAX(head.revision) FROM gl_schedule_revisions head
          WHERE head.schedule_id=s.id AND head.profile_id=s.profile_id) AS head_revision
        FROM gl_schedules s JOIN gl_schedule_revisions r
        ON r.schedule_id=s.id AND r.profile_id=s.profile_id
        WHERE s.profile_id=? AND r.id=(SELECT r2.id FROM gl_schedule_revisions r2
          WHERE r2.schedule_id=s.id AND r2.profile_id=s.profile_id
          ORDER BY r2.effective_date DESC,r2.revision DESC LIMIT 1) ORDER BY s.id LIMIT ?""", (profile_id, limit)):
        item = dict(row)
        item["fields"] = json.loads(item.pop("payload_json"))
        result.append(item)
    return result


def schedules_page(conn: Any, profile_id: str, *, limit: int = 100, cursor: str | None = None) -> dict:
    from .paging import records_page
    def materialize(identifier: str) -> dict:
        row = conn.execute("""SELECT s.id,s.kind,s.label,r.revision,r.effective_date,r.evidence_id,
            r.entry_id,r.payload_json,r.reason,(SELECT MAX(head.revision) FROM gl_schedule_revisions head
              WHERE head.schedule_id=s.id AND head.profile_id=s.profile_id) AS head_revision
            FROM gl_schedules s JOIN gl_schedule_revisions r ON r.schedule_id=s.id AND r.profile_id=s.profile_id
            WHERE s.profile_id=? AND s.id=? ORDER BY r.effective_date DESC,r.revision DESC LIMIT 1""",
            (profile_id, identifier)).fetchone()
        if row is None:
            raise AppError("Schedule has no retained revision", code="accounting_schedule_not_found")
        record = dict(row)
        record["fields"] = json.loads(record.pop("payload_json"))
        return record
    return records_page(conn, profile_id, "schedules", limit=limit, cursor=cursor, materialize=materialize)


def validate_close(conn: Any, profile_id: str, start_date: str, end_date: str) -> dict:
    """Recomputable supporting state at the close date, not today's open-item view.

No assertion of bank completeness without a statement and no assertion of a
schedule's legal suitability. Coverage is an explicit book-level prerequisite.
"""
    require_book(conn, profile_id)
    iso_date(start_date)
    iso_date(end_date)
    statements, blockers = [], []
    for row in conn.execute("""SELECT id,start_date,end_date FROM gl_bank_statements s
        WHERE profile_id=? AND start_date<=? AND end_date>=? AND NOT EXISTS
        (SELECT 1 FROM gl_bank_statement_voids v WHERE v.statement_id=s.id) ORDER BY start_date,id""",
        (profile_id, end_date, start_date)).fetchall():
        result = reconcile_statement(conn, profile_id, row["id"])
        statements.append(result)
        if row["start_date"] < start_date or row["end_date"] > end_date:
            blockers.append({"code": "statement_crosses_close_boundary", "statement_id": row["id"]})
        for code in result["blockers"]:
            blockers.append({"code": code, "statement_id": row["id"]})
    items = [dict(row) for row in conn.execute("""SELECT i.*,i.amount_minor-COALESCE((
        SELECT SUM(a.amount_minor) FROM gl_open_item_allocations a JOIN gl_lines al ON al.id=a.line_id
          AND al.profile_id=a.profile_id JOIN gl_entries ae ON ae.id=al.entry_id AND ae.profile_id=al.profile_id
        WHERE a.item_id=i.id AND a.profile_id=i.profile_id AND ae.entry_date<=?
          AND NOT EXISTS (SELECT 1 FROM gl_open_item_allocation_voids v WHERE v.allocation_id=a.id)),0) AS remaining_minor
        FROM gl_open_items i JOIN gl_lines l ON l.id=i.origin_line_id AND l.profile_id=i.profile_id
        JOIN gl_entries e ON e.id=l.entry_id AND e.profile_id=l.profile_id
        WHERE i.profile_id=? AND e.entry_date<=? AND NOT EXISTS
        (SELECT 1 FROM gl_open_item_voids v WHERE v.item_id=i.id) ORDER BY i.id""", (end_date, profile_id, end_date))]
    for item in items:
        get_open_item(conn, profile_id, item["id"])
        item.update(_item_metadata(conn, profile_id, item, as_of=end_date))
    revisions = []
    for row in conn.execute("""SELECT s.kind,s.label,r.* FROM gl_schedules s JOIN gl_schedule_revisions r
        ON r.schedule_id=s.id AND r.profile_id=s.profile_id WHERE s.profile_id=? AND r.effective_date<=?
        AND r.id=(SELECT r2.id FROM gl_schedule_revisions r2
          WHERE r2.schedule_id=s.id AND r2.profile_id=s.profile_id AND r2.effective_date<=?
          ORDER BY r2.effective_date DESC,r2.revision DESC LIMIT 1) ORDER BY s.id""",
        (profile_id, end_date, end_date)):
        record = dict(row)
        record["fields"] = json.loads(record.pop("payload_json"))
        revisions.append(record)
    return {"blockers": blockers, "bank_statements": statements, "open_items": items,
            "schedules": revisions, "coverage_verified": False}


def void_settlement(conn: Any, profile_id: str, *, allocation_id: str, reason: str,
                     idempotency_key: str) -> dict:
    bounded_text(reason, "reason", 2000)
    bounded_text(idempotency_key, "idempotency_key", 128)
    with atomic(conn):
        require_book(conn, profile_id)
        prior = conn.execute("SELECT * FROM gl_open_item_allocation_voids WHERE profile_id=? AND idempotency_key=?",
                             (profile_id, idempotency_key)).fetchone()
        if prior:
            if prior["allocation_id"] != allocation_id or prior["reason"] != reason:
                raise AppError("Cancellation identity changed", code="accounting_idempotency_conflict")
            return dict(prior)
        row = conn.execute("SELECT * FROM gl_open_item_allocations WHERE id=? AND profile_id=?", (allocation_id, profile_id)).fetchone()
        if not row:
            raise AppError("Settlement not found", code="accounting_open_item_not_found")
        if conn.execute("SELECT 1 FROM gl_open_item_allocation_voids WHERE allocation_id=? AND profile_id=?", (allocation_id, profile_id)).fetchone():
            raise AppError("Settlement is already cancelled", code="accounting_already_voided")
        line = posted_line(conn, profile_id, row["line_id"])
        require_open_interval(conn, profile_id, line["entry_date"], line["entry_date"])
        conn.execute("INSERT INTO gl_open_item_allocation_voids VALUES (?,?,?,?)", (allocation_id, profile_id, reason, idempotency_key))
        conn.execute("UPDATE gl_books SET revision=revision+1 WHERE profile_id=?", (profile_id,))
        return {"allocation_id": allocation_id, "voided": True, "reason": reason}


def void_open_item(conn: Any, profile_id: str, *, item_id: str, reason: str, idempotency_key: str) -> dict:
    bounded_text(reason, "reason", 2000)
    bounded_text(idempotency_key, "idempotency_key", 128)
    with atomic(conn):
        require_book(conn, profile_id)
        prior = conn.execute("SELECT * FROM gl_open_item_voids WHERE profile_id=? AND idempotency_key=?",
                             (profile_id, idempotency_key)).fetchone()
        if prior:
            if prior["item_id"] != item_id or prior["reason"] != reason:
                raise AppError("Cancellation identity changed", code="accounting_idempotency_conflict")
            return dict(prior)
        item = get_open_item(conn, profile_id, item_id)
        if item["voided"]:
            raise AppError("Open item is already cancelled", code="accounting_already_voided")
        line = posted_line(conn, profile_id, item["origin_line_id"])
        require_open_interval(conn, profile_id, line["entry_date"], line["entry_date"])
        if item["remaining_minor"] != item["amount_minor"]:
            raise AppError("Cancel active settlements first", code="accounting_open_item_allocated")
        conn.execute("INSERT INTO gl_open_item_voids VALUES (?,?,?,?)", (item_id, profile_id, reason, idempotency_key))
        conn.execute("UPDATE gl_books SET revision=revision+1 WHERE profile_id=?", (profile_id,))
        return {"item_id": item_id, "voided": True, "reason": reason}
