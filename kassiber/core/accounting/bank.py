"""Canonical-v1 bank CSV ingestion and exact, line-backed reconciliation.

This is a documented interchange adapter, not a claim to support a bank's
export. Row IDs belong to the statement, not to transaction content hashes.
"""
from __future__ import annotations

import csv
from datetime import date
import hashlib
import io
import json
import re
import uuid
from typing import Any

from kassiber.errors import AppError
from .ledger import atomic, require_book, strict_minor
from .evidence import bounded_text, require_evidence, read_evidence_bytes

ADAPTER_VERSION = "kassiber-canonical-bank-csv-v1"
CSV_COLUMNS = ["row_id", "date", "amount_minor", "description"]


def signed_minor(value: Any) -> int:
    if type(value) is not int:
        raise AppError("Amounts must be exact integer minor units", code="accounting_invalid_input")
    strict_minor(abs(value))
    return value


def require_open_interval(conn: Any, profile_id: str, start_date: str, end_date: str) -> None:
    """As-of supporting facts also affect later closes, including gap dates."""
    if conn.execute("""SELECT 1 FROM gl_periods WHERE profile_id=? AND state!='open'
        AND end_date>=?""", (profile_id, start_date)).fetchone():
        raise AppError("Reopen affected accounting period before changing supporting records",
                       code="accounting_period_closed")


def ensure_schema(conn: Any) -> None:
    statements = [
        """CREATE TABLE IF NOT EXISTS gl_bank_statements (
          id TEXT PRIMARY KEY,profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE RESTRICT,
          account_code TEXT NOT NULL,statement_id TEXT NOT NULL,adapter_version TEXT NOT NULL,
          start_date TEXT NOT NULL,end_date TEXT NOT NULL,
          opening_minor INTEGER CHECK(opening_minor IS NULL OR typeof(opening_minor)='integer'),
          closing_minor INTEGER CHECK(closing_minor IS NULL OR typeof(closing_minor)='integer'),
          evidence_id TEXT,payload_digest TEXT NOT NULL,UNIQUE(profile_id,account_code,statement_id),
          UNIQUE(profile_id,id),FOREIGN KEY(profile_id,account_code) REFERENCES gl_accounts(profile_id,code),
          FOREIGN KEY(profile_id,evidence_id) REFERENCES gl_evidence(profile_id,id))""",
        """CREATE TABLE IF NOT EXISTS gl_bank_rows (
          id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,statement_id TEXT NOT NULL,row_id TEXT NOT NULL,
          occurred_on TEXT NOT NULL,amount_minor INTEGER NOT NULL,description TEXT NOT NULL,
          UNIQUE(statement_id,row_id),UNIQUE(profile_id,id),CHECK(typeof(amount_minor)='integer' AND amount_minor != 0),
          FOREIGN KEY(profile_id,statement_id) REFERENCES gl_bank_statements(profile_id,id))""",
        """CREATE TABLE IF NOT EXISTS gl_bank_allocations (
          id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,row_id TEXT NOT NULL,line_id TEXT NOT NULL,
          amount_minor INTEGER NOT NULL CHECK(typeof(amount_minor)='integer' AND amount_minor > 0),idempotency_key TEXT NOT NULL,
          UNIQUE(profile_id,idempotency_key),
          FOREIGN KEY(profile_id,row_id) REFERENCES gl_bank_rows(profile_id,id),
          FOREIGN KEY(line_id) REFERENCES gl_lines(id))""",
    ]
    for statement in statements:
        conn.execute(statement)
    columns = {row[1] for row in conn.execute('PRAGMA table_info(gl_bank_statements)')}
    for name, declaration in (
        ('control_evidence_id', 'TEXT REFERENCES gl_evidence(id) ON DELETE RESTRICT'),
        ('control_review_reason', 'TEXT'), ('control_locator', 'TEXT')):
        if name not in columns:
            conn.execute(f'ALTER TABLE gl_bank_statements ADD COLUMN {name} {declaration}')
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_bank_control_scope BEFORE INSERT ON gl_bank_statements
        WHEN (NEW.control_evidence_id IS NULL AND (NEW.control_review_reason IS NOT NULL OR NEW.control_locator IS NOT NULL))
        OR (NEW.control_evidence_id IS NOT NULL AND (
          NEW.control_review_reason IS NULL OR length(trim(NEW.control_review_reason))=0 OR length(NEW.control_review_reason)>2000
          OR NEW.control_locator IS NULL OR length(trim(NEW.control_locator))=0 OR length(NEW.control_locator)>1000
          OR NEW.opening_minor IS NULL OR NEW.closing_minor IS NULL
          OR NOT EXISTS(SELECT 1 FROM gl_evidence e WHERE e.id=NEW.control_evidence_id AND e.profile_id=NEW.profile_id)))
        BEGIN SELECT RAISE(ABORT,'accounting_bank_control_scope'); END""")
    for table, key, target in (("gl_bank_allocation_voids", "allocation_id", "gl_bank_allocations"),
                               ("gl_bank_statement_voids", "statement_id", "gl_bank_statements")):
        conn.execute(f"""CREATE TABLE IF NOT EXISTS {table} (
            {key} TEXT PRIMARY KEY REFERENCES {target}(id),profile_id TEXT NOT NULL,
            reason TEXT NOT NULL,idempotency_key TEXT NOT NULL,UNIQUE(profile_id,idempotency_key))""")
        for action in ("UPDATE", "DELETE"):
            conn.execute(f"""CREATE TRIGGER IF NOT EXISTS {table}_no_{action.lower()} BEFORE {action} ON {table}
                BEGIN SELECT RAISE(ABORT,'accounting_bank_retained'); END""")
        conn.execute(f"""CREATE TRIGGER IF NOT EXISTS {table}_no_replace BEFORE INSERT ON {table}
            WHEN EXISTS (SELECT 1 FROM {table} WHERE {key}=NEW.{key}
                OR (profile_id=NEW.profile_id AND idempotency_key=NEW.idempotency_key))
            BEGIN SELECT RAISE(ABORT,'accounting_bank_retained'); END""")
        conn.execute(f"""CREATE TRIGGER IF NOT EXISTS {table}_scope BEFORE INSERT ON {table}
            WHEN NOT EXISTS (SELECT 1 FROM {target} WHERE id=NEW.{key} AND profile_id=NEW.profile_id)
            BEGIN SELECT RAISE(ABORT,'accounting_bank_scope'); END""")
    for table in ("gl_bank_statements", "gl_bank_rows", "gl_bank_allocations"):
        for action in ("UPDATE", "DELETE"):
            conn.execute(f"""CREATE TRIGGER IF NOT EXISTS {table}_no_{action.lower()}
                BEFORE {action} ON {table} BEGIN SELECT RAISE(ABORT,'accounting_bank_retained'); END""")
    for table, conflict in (
        ("gl_bank_statements", "id=NEW.id OR (profile_id=NEW.profile_id AND account_code=NEW.account_code AND statement_id=NEW.statement_id)"),
        ("gl_bank_rows", "id=NEW.id OR (statement_id=NEW.statement_id AND row_id=NEW.row_id)"),
        ("gl_bank_allocations", "id=NEW.id OR (profile_id=NEW.profile_id AND idempotency_key=NEW.idempotency_key)"),
    ):
        conn.execute(f"""CREATE TRIGGER IF NOT EXISTS {table}_no_replace BEFORE INSERT ON {table}
            WHEN EXISTS (SELECT 1 FROM {table} WHERE {conflict})
            BEGIN SELECT RAISE(ABORT,'accounting_bank_retained'); END""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_bank_allocation_scope BEFORE INSERT ON gl_bank_allocations
        WHEN NOT EXISTS (SELECT 1 FROM gl_bank_rows r JOIN gl_bank_statements s
          ON s.id=r.statement_id AND s.profile_id=r.profile_id JOIN gl_lines l
          ON l.id=NEW.line_id AND l.profile_id=r.profile_id JOIN gl_entries e
          ON e.id=l.entry_id AND e.profile_id=l.profile_id
          WHERE r.id=NEW.row_id AND r.profile_id=NEW.profile_id AND e.status='posted'
          AND NOT EXISTS (SELECT 1 FROM gl_entries reversal WHERE reversal.reversal_of=e.id AND reversal.status='posted')
          AND l.account_code=s.account_code AND ((r.amount_minor>0 AND l.debit_minor>0)
            OR (r.amount_minor<0 AND l.credit_minor>0))
          AND NOT EXISTS (SELECT 1 FROM gl_bank_statement_voids v WHERE v.statement_id=s.id))
        BEGIN SELECT RAISE(ABORT,'accounting_bank_scope'); END""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_bank_allocation_budget BEFORE INSERT ON gl_bank_allocations
        WHEN NEW.amount_minor + COALESCE((SELECT SUM(a.amount_minor) FROM gl_bank_allocations a
          WHERE a.row_id=NEW.row_id AND NOT EXISTS (SELECT 1 FROM gl_bank_allocation_voids v WHERE v.allocation_id=a.id)),0)
          > (SELECT abs(amount_minor) FROM gl_bank_rows WHERE id=NEW.row_id)
        OR NEW.amount_minor + COALESCE((SELECT SUM(a.amount_minor) FROM gl_bank_allocations a
          WHERE a.line_id=NEW.line_id AND NOT EXISTS (SELECT 1 FROM gl_bank_allocation_voids v WHERE v.allocation_id=a.id)),0)
          > (SELECT debit_minor+credit_minor FROM gl_lines WHERE id=NEW.line_id)
        BEGIN SELECT RAISE(ABORT,'accounting_allocation_exceeded'); END""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_bank_evidence_single_source BEFORE INSERT ON gl_bank_statements
        WHEN NEW.evidence_id IS NOT NULL AND EXISTS (SELECT 1 FROM gl_bank_statements s
          WHERE s.profile_id=NEW.profile_id AND s.evidence_id=NEW.evidence_id
            AND NOT EXISTS (SELECT 1 FROM gl_bank_statement_voids v WHERE v.statement_id=s.id))
        BEGIN SELECT RAISE(ABORT,'accounting_bank_source_reused'); END""")


def iso_date(value: Any) -> str:
    try:
        if not isinstance(value, str) or date.fromisoformat(value).isoformat() != value:
            raise ValueError()
    except ValueError as exc:
        raise AppError("Expected ISO calendar date", code="accounting_invalid_input") from exc
    return value


def preview_statement(csv_text: str, *, start_date: str, end_date: str,
                      opening_minor: int | None = None, closing_minor: int | None = None) -> dict:
    iso_date(start_date)
    iso_date(end_date)
    if start_date > end_date:
        raise AppError("Invalid statement interval", code="accounting_invalid_input")
    if not isinstance(csv_text, str) or len(csv_text.encode("utf-8")) > 4 * 1024 * 1024:
        raise AppError("CSV exceeds 4 MiB", code="accounting_bank_size")
    for value in (opening_minor, closing_minor):
        if value is not None:
            signed_minor(value)
    rows, seen = [], set()
    try:
        reader = csv.DictReader(io.StringIO(csv_text), strict=True)
        if reader.fieldnames != CSV_COLUMNS:
            raise AppError("Canonical CSV requires row_id,date,amount_minor,description", code="accounting_bank_format")
        for row in reader:
            if len(rows) >= 10000 or None in row or any(value is None for value in row.values()):
                raise AppError("Invalid or oversized CSV row", code="accounting_bank_format")
            row_id = bounded_text(row["row_id"], "row_id", 128)
            if row_id in seen:
                raise AppError("Duplicate statement row identity", code="accounting_bank_duplicate_row")
            seen.add(row_id)
            occurred = iso_date(row["date"])
            if not start_date <= occurred <= end_date:
                raise AppError("Row outside statement interval", code="accounting_bank_date")
            if not re.fullmatch(r"-?(0|[1-9][0-9]*)", row["amount_minor"]):
                raise AppError("Amounts must be exact integer minor units", code="accounting_bank_format")
            amount = signed_minor(int(row["amount_minor"]))
            if amount == 0 or len(row["description"]) > 2000 or "\x00" in row["description"]:
                raise AppError("Invalid bank amount or description", code="accounting_bank_format")
            rows.append({"row_id": row_id, "occurred_on": occurred, "amount_minor": amount,
                         "description": row["description"]})
    except (csv.Error, ValueError) as exc:
        raise AppError("Malformed canonical CSV", code="accounting_bank_format") from exc
    movement = sum(row["amount_minor"] for row in rows)
    signed_minor(movement)
    controlled = opening_minor is not None and closing_minor is not None
    control_matches = controlled and opening_minor + movement == closing_minor
    return {"adapter_version": ADAPTER_VERSION, "start_date": start_date, "end_date": end_date,
            "opening_minor": opening_minor, "closing_minor": closing_minor, "rows": rows,
            "movement_minor": movement, "control_balances_present": controlled,
            "control_matches": bool(control_matches)}


def _validate_control_evidence(conn, profile_id, *, account_code, statement_id, start_date, end_date,
                               opening_minor, closing_minor, control_evidence_id=None,
                               control_review_reason=None, control_locator=None):
    if control_evidence_id is None:
        if control_review_reason is not None or control_locator is not None:
            raise AppError('Control review requires retained control evidence', code='accounting_bank_control_evidence')
        return None
    bounded_text(control_review_reason, 'control_review_reason', 2000)
    bounded_text(control_locator, 'control_locator', 1000)
    if opening_minor is None or closing_minor is None:
        raise AppError('Control evidence requires both reviewed balances', code='accounting_bank_control_evidence')
    metadata = require_evidence(conn, profile_id, control_evidence_id)
    content = read_evidence_bytes(conn, profile_id, control_evidence_id)
    if metadata['media_type'] == 'application/pdf' and content.startswith(b'%PDF-'):
        method = 'human_reviewed_pdf_locator'
    elif metadata['media_type'] in ('application/json', 'application/vnd.kassiber.bank-control+json'):
        try:
            if len(content) > 65536:
                raise ValueError()
            def unique_fields(pairs):
                result = {}
                for key, value in pairs:
                    if key in result:
                        raise ValueError('Duplicate control field')
                    result[key] = value
                return result
            control = json.loads(content, object_pairs_hook=unique_fields)
            book = require_book(conn, profile_id)
            expected = dict(format='kassiber-bank-control-v1', account_code=account_code, statement_id=statement_id,
                            start_date=start_date, end_date=end_date, opening_minor=opening_minor,
                            closing_minor=closing_minor, currency=book['currency'], minor_unit_exponent=book['minor_unit_exponent'])
            if not isinstance(control, dict) or set(control) != set(expected) or any(
                type(control[key]) is not type(value) or control[key] != value for key, value in expected.items()):
                raise ValueError()
        except (UnicodeError, ValueError, RecursionError) as exc:
            raise AppError('Canonical control evidence must match exact statement identity and balances',
                           code='accounting_bank_control_evidence') from exc
        method = 'reviewed_exact_control_record'
    else:
        raise AppError('Use a retained PDF with a reviewed locator or a canonical JSON balance-control record; row-only CSV is not balance evidence',
                       code='accounting_bank_control_evidence')
    return dict(evidence_id=control_evidence_id, content_sha256=metadata['content_sha256'],
                review_reason=control_review_reason, locator=control_locator, method=method)


def import_statement(conn: Any, profile_id: str, *, account_code: str, statement_id: str,
                     csv_text: str, start_date: str, end_date: str,
                     opening_minor: int | None = None, closing_minor: int | None = None,
                     evidence_id: str | None = None, control_evidence_id: str | None = None,
                     control_review_reason: str | None = None, control_locator: str | None = None) -> dict:
    preview = preview_statement(csv_text, start_date=start_date, end_date=end_date,
                                opening_minor=opening_minor, closing_minor=closing_minor)
    bounded_text(statement_id, "statement_id", 128)
    controls = dict(control_evidence_id=control_evidence_id, control_review_reason=control_review_reason, control_locator=control_locator)
    # Keep legacy identity retry stable when no new control review is supplied.
    committed = {**preview, 'evidence_id': evidence_id, **(controls if any(value is not None for value in controls.values()) else {})}
    digest = hashlib.sha256(json.dumps(committed, sort_keys=True,
                                      separators=(",", ":")).encode()).hexdigest()
    with atomic(conn):
        require_book(conn, profile_id)
        account = conn.execute("SELECT kind FROM gl_accounts WHERE profile_id=? AND code=?",
                               (profile_id, account_code)).fetchone()
        if account is None or account["kind"] != "asset":
            raise AppError("Bank account must be a book asset account", code="accounting_bank_account")
        if evidence_id is not None:
            require_evidence(conn, profile_id, evidence_id)
        _validate_control_evidence(conn, profile_id, account_code=account_code, statement_id=statement_id,
            start_date=start_date, end_date=end_date, opening_minor=opening_minor, closing_minor=closing_minor, **controls)
        existing = conn.execute("""SELECT id,payload_digest FROM gl_bank_statements
            WHERE profile_id=? AND account_code=? AND statement_id=?""",
            (profile_id, account_code, statement_id)).fetchone()
        if existing:
            if existing["payload_digest"] != digest:
                raise AppError("Statement identity reused with changed content", code="accounting_idempotency_conflict")
            return {"id": existing["id"], "already_imported": True, **preview}
        require_open_interval(conn, profile_id, start_date, end_date)
        if evidence_id is not None and conn.execute("""SELECT 1 FROM gl_bank_statements s
            WHERE s.profile_id=? AND s.evidence_id=? AND NOT EXISTS
            (SELECT 1 FROM gl_bank_statement_voids v WHERE v.statement_id=s.id)""", (profile_id, evidence_id)).fetchone():
            raise AppError("Statement evidence is already assigned; cancel the original assignment before correcting it",
                           code="accounting_bank_source_reused")
        if conn.execute("""SELECT 1 FROM gl_bank_statements s WHERE profile_id=? AND account_code=?
            AND start_date<=? AND end_date>=? AND NOT EXISTS
            (SELECT 1 FROM gl_bank_statement_voids v WHERE v.statement_id=s.id)""",
            (profile_id, account_code, end_date, start_date)).fetchone():
            raise AppError("Statement coverage overlaps an existing import; review source coverage first",
                           code="accounting_bank_overlap")
        if preview["control_balances_present"] and not preview["control_matches"]:
            raise AppError("Statement opening plus movements differs from closing", code="accounting_bank_control")
        identifier = str(uuid.uuid4())
        conn.execute("""INSERT INTO gl_bank_statements
            (id,profile_id,account_code,statement_id,adapter_version,start_date,end_date,opening_minor,closing_minor,
             evidence_id,payload_digest,control_evidence_id,control_review_reason,control_locator)
            VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)""",
            (identifier, profile_id, account_code, statement_id, ADAPTER_VERSION, start_date,
             end_date, opening_minor, closing_minor, evidence_id, digest, control_evidence_id, control_review_reason, control_locator))
        for row in preview["rows"]:
            conn.execute("INSERT INTO gl_bank_rows VALUES (?,?,?,?,?,?,?)", (str(uuid.uuid4()),
                profile_id, identifier, row["row_id"], row["occurred_on"], row["amount_minor"], row["description"]))
        conn.execute("UPDATE gl_books SET revision=revision+1 WHERE profile_id=?", (profile_id,))
        return {"id": identifier, "already_imported": False, **preview}


def posted_line(conn: Any, profile_id: str, line_id: str) -> dict:
    row = conn.execute("""SELECT l.*,e.entry_date,e.entry_kind FROM gl_lines l JOIN gl_entries e
        ON e.id=l.entry_id AND e.profile_id=l.profile_id
        WHERE l.id=? AND l.profile_id=? AND e.status='posted' AND NOT EXISTS
          (SELECT 1 FROM gl_entries reversal WHERE reversal.reversal_of=e.id AND reversal.status='posted')""",
        (line_id, profile_id)).fetchone()
    if row is None:
        raise AppError("Posted line not found in this book", code="accounting_posted_line_required")
    return dict(row)


def allocate_bank_row(conn: Any, profile_id: str, *, row_id: str, line_id: str,
                      amount_minor: int, idempotency_key: str) -> dict:
    amount = strict_minor(amount_minor)
    bounded_text(idempotency_key, "idempotency_key", 128)
    if amount <= 0:
        raise AppError("Allocation must be positive", code="accounting_invalid_input")
    with atomic(conn):
        require_book(conn, profile_id)
        prior = conn.execute("SELECT * FROM gl_bank_allocations WHERE profile_id=? AND idempotency_key=?",
                             (profile_id, idempotency_key)).fetchone()
        if prior:
            if (prior["row_id"], prior["line_id"], prior["amount_minor"]) != (row_id, line_id, amount):
                raise AppError("Allocation identity changed", code="accounting_idempotency_conflict")
            return dict(prior)
        row = conn.execute("""SELECT r.*,s.account_code,s.start_date AS statement_start,s.end_date AS statement_end FROM gl_bank_rows r JOIN gl_bank_statements s
            ON r.statement_id=s.id AND r.profile_id=s.profile_id WHERE r.id=? AND r.profile_id=?""",
            (row_id, profile_id)).fetchone()
        if row is None:
            raise AppError("Bank row not found in this book", code="accounting_bank_row_not_found")
        if conn.execute("SELECT 1 FROM gl_bank_statement_voids WHERE statement_id=?", (row["statement_id"],)).fetchone():
            raise AppError("Statement was cancelled", code="accounting_bank_statement_voided")
        line = posted_line(conn, profile_id, line_id)
        require_open_interval(conn, profile_id, line["entry_date"], line["entry_date"])
        require_open_interval(conn, profile_id, row["statement_start"], row["statement_end"])
        delta = line["debit_minor"] - line["credit_minor"]
        if line["account_code"] != row["account_code"] or delta * row["amount_minor"] <= 0:
            raise AppError("Bank allocation account/direction mismatch", code="accounting_bank_allocation")
        for column, identity, capacity in (("row_id", row_id, abs(row["amount_minor"])),
                                          ("line_id", line_id, abs(delta))):
            used = conn.execute(f"""SELECT COALESCE(SUM(amount_minor),0) FROM gl_bank_allocations a
                WHERE profile_id=? AND {column}=? AND NOT EXISTS
                (SELECT 1 FROM gl_bank_allocation_voids v WHERE v.allocation_id=a.id)""",
                                (profile_id, identity)).fetchone()[0]
            if used + amount > capacity:
                raise AppError("Bank allocation exceeds remaining amount", code="accounting_allocation_exceeded")
        identifier = str(uuid.uuid4())
        conn.execute("INSERT INTO gl_bank_allocations VALUES (?,?,?,?,?,?)",
                     (identifier, profile_id, row_id, line_id, amount, idempotency_key))
        conn.execute("UPDATE gl_books SET revision=revision+1 WHERE profile_id=?", (profile_id,))
        return {"id": identifier, "row_id": row_id, "line_id": line_id, "amount_minor": amount}


def reconcile_statement(conn: Any, profile_id: str, statement_id: str) -> dict:
    require_book(conn, profile_id)
    statement = conn.execute("SELECT * FROM gl_bank_statements WHERE profile_id=? AND id=?",
                             (profile_id, statement_id)).fetchone()
    if statement is None:
        raise AppError("Statement not found in this book", code="accounting_bank_statement_not_found")
    rows = [dict(row) for row in conn.execute("""SELECT r.*,abs(r.amount_minor)-COALESCE(
        (SELECT SUM(a.amount_minor) FROM gl_bank_allocations a WHERE a.row_id=r.id AND a.profile_id=r.profile_id
          AND NOT EXISTS (SELECT 1 FROM gl_bank_allocation_voids v WHERE v.allocation_id=a.id)),0)
        AS remaining_minor FROM gl_bank_rows r WHERE r.profile_id=? AND r.statement_id=? ORDER BY occurred_on,row_id""",
        (profile_id, statement_id))]
    for row in rows:
        row["allocations"] = [dict(a) for a in conn.execute("""SELECT a.*,EXISTS
            (SELECT 1 FROM gl_bank_allocation_voids v WHERE v.allocation_id=a.id) AS voided
            FROM gl_bank_allocations a WHERE a.profile_id=? AND a.row_id=? ORDER BY a.id""", (profile_id, row["id"]))]
    def balance(operator: str, day: str) -> int:
        date_predicate = "(e.entry_date < ? OR (e.entry_date = ? AND e.entry_kind='opening'))" if operator == "<" else "e.entry_date <= ?"
        dates = (day, day) if operator == "<" else (day,)
        return sum(row[0] - row[1] for row in conn.execute(f"""SELECT l.debit_minor,l.credit_minor
            FROM gl_lines l JOIN gl_entries e ON e.id=l.entry_id AND e.profile_id=l.profile_id
            WHERE l.profile_id=? AND l.account_code=? AND e.status='posted' AND {date_predicate}""",
            (profile_id, statement["account_code"], *dates)))
    opening = balance("<", statement["start_date"])
    closing = balance("<=", statement["end_date"])
    blockers = []
    invalid = conn.execute("""SELECT 1 FROM gl_bank_allocations a JOIN gl_bank_rows r ON r.id=a.row_id
        LEFT JOIN gl_lines l ON l.id=a.line_id LEFT JOIN gl_entries e ON e.id=l.entry_id
        WHERE r.statement_id=? AND (a.profile_id!=r.profile_id OR l.profile_id!=r.profile_id
          OR e.profile_id!=r.profile_id OR l.id IS NULL OR e.status!='posted'
          OR l.account_code!=? OR typeof(a.amount_minor)!='integer' OR a.amount_minor<=0
          OR (r.amount_minor>0 AND l.debit_minor=0) OR (r.amount_minor<0 AND l.credit_minor=0))
        AND NOT EXISTS (SELECT 1 FROM gl_bank_allocation_voids v WHERE v.allocation_id=a.id)""",
        (statement_id, statement["account_code"])).fetchone()
    if invalid:
        blockers.append("invalid_allocation_integrity")
    if conn.execute("SELECT 1 FROM gl_bank_statement_voids WHERE statement_id=?", (statement_id,)).fetchone():
        blockers.append("statement_voided")
    if statement["opening_minor"] is None or statement["closing_minor"] is None:
        blockers.append("missing_control_balances")
    if any(row["remaining_minor"] for row in rows):
        blockers.append("unallocated_bank_rows")
    if statement["opening_minor"] is not None and opening != statement["opening_minor"]:
        blockers.append("opening_balance_mismatch")
    if statement["closing_minor"] is not None and closing != statement["closing_minor"]:
        blockers.append("closing_balance_mismatch")
    arithmetic_reconciled = not blockers
    control_provenance = None
    if not statement['control_evidence_id']:
        blockers.append('missing_control_evidence')
    else:
        try:
            control_provenance = _validate_control_evidence(conn, profile_id, **{key: statement[key] for key in (
                'account_code', 'statement_id', 'start_date', 'end_date', 'opening_minor', 'closing_minor',
                'control_evidence_id', 'control_review_reason', 'control_locator')})
        except AppError:
            blockers.append('invalid_control_evidence')
    return {"statement": dict(statement), "rows": rows, "ledger_opening_minor": opening,
            "ledger_closing_minor": closing, "arithmetic_reconciled": arithmetic_reconciled,
            "control_evidence_reviewed": control_provenance is not None, "control_provenance": control_provenance,
            "reconciled": not blockers, "blockers": blockers}


def void_bank_allocation(conn: Any, profile_id: str, *, allocation_id: str, reason: str,
                         idempotency_key: str) -> dict:
    bounded_text(reason, "reason", 2000)
    bounded_text(idempotency_key, "idempotency_key", 128)
    with atomic(conn):
        require_book(conn, profile_id)
        prior = conn.execute("SELECT * FROM gl_bank_allocation_voids WHERE profile_id=? AND idempotency_key=?",
                             (profile_id, idempotency_key)).fetchone()
        if prior:
            if prior["allocation_id"] != allocation_id or prior["reason"] != reason:
                raise AppError("Cancellation identity changed", code="accounting_idempotency_conflict")
            return dict(prior)
        row = conn.execute("SELECT * FROM gl_bank_allocations WHERE id=? AND profile_id=?", (allocation_id, profile_id)).fetchone()
        if not row:
            raise AppError("Allocation not found", code="accounting_bank_allocation")
        if conn.execute("SELECT 1 FROM gl_bank_allocation_voids WHERE allocation_id=? AND profile_id=?", (allocation_id, profile_id)).fetchone():
            raise AppError("Allocation is already cancelled", code="accounting_already_voided")
        line = posted_line(conn, profile_id, row["line_id"])
        require_open_interval(conn, profile_id, line["entry_date"], line["entry_date"])
        statement = conn.execute("""SELECT s.start_date,s.end_date FROM gl_bank_rows r
            JOIN gl_bank_statements s ON s.id=r.statement_id WHERE r.id=? AND r.profile_id=?""",
            (row["row_id"], profile_id)).fetchone()
        require_open_interval(conn, profile_id, statement["start_date"], statement["end_date"])
        conn.execute("INSERT INTO gl_bank_allocation_voids VALUES (?,?,?,?)", (allocation_id, profile_id, reason, idempotency_key))
        conn.execute("UPDATE gl_books SET revision=revision+1 WHERE profile_id=?", (profile_id,))
        return {"allocation_id": allocation_id, "voided": True, "reason": reason}


def void_statement(conn: Any, profile_id: str, *, statement_id: str, reason: str, idempotency_key: str) -> dict:
    bounded_text(reason, "reason", 2000)
    bounded_text(idempotency_key, "idempotency_key", 128)
    with atomic(conn):
        require_book(conn, profile_id)
        prior = conn.execute("SELECT * FROM gl_bank_statement_voids WHERE profile_id=? AND idempotency_key=?",
                             (profile_id, idempotency_key)).fetchone()
        if prior:
            if prior["statement_id"] != statement_id or prior["reason"] != reason:
                raise AppError("Cancellation identity changed", code="accounting_idempotency_conflict")
            return dict(prior)
        statement = conn.execute("SELECT * FROM gl_bank_statements WHERE id=? AND profile_id=?", (statement_id, profile_id)).fetchone()
        if not statement:
            raise AppError("Statement not found", code="accounting_bank_statement_not_found")
        if conn.execute("SELECT 1 FROM gl_bank_statement_voids WHERE statement_id=? AND profile_id=?", (statement_id, profile_id)).fetchone():
            raise AppError("Statement is already cancelled", code="accounting_already_voided")
        require_open_interval(conn, profile_id, statement["start_date"], statement["end_date"])
        if conn.execute("""SELECT 1 FROM gl_bank_allocations a JOIN gl_bank_rows r ON r.id=a.row_id
            WHERE r.statement_id=? AND a.profile_id=? AND NOT EXISTS
            (SELECT 1 FROM gl_bank_allocation_voids v WHERE v.allocation_id=a.id)""", (statement_id, profile_id)).fetchone():
            raise AppError("Cancel active bank allocations first", code="accounting_bank_allocated")
        conn.execute("INSERT INTO gl_bank_statement_voids VALUES (?,?,?,?)", (statement_id, profile_id, reason, idempotency_key))
        conn.execute("UPDATE gl_books SET revision=revision+1 WHERE profile_id=?", (profile_id,))
        return {"statement_id": statement_id, "voided": True, "reason": reason}


def list_statements(conn: Any, profile_id: str, *, limit: int = 100) -> list[dict]:
    require_book(conn, profile_id)
    if type(limit) is not int or not 1 <= limit <= 500:
        raise AppError("Invalid statement limit", code="accounting_invalid_input")
    return [dict(row) for row in conn.execute("""SELECT s.*,EXISTS
        (SELECT 1 FROM gl_bank_statement_voids v WHERE v.statement_id=s.id) AS voided
        FROM gl_bank_statements s WHERE s.profile_id=? ORDER BY end_date DESC,id LIMIT ?""", (profile_id, limit))]


def statements_page(conn: Any, profile_id: str, *, limit: int = 100, cursor: str | None = None) -> dict:
    from .paging import records_page
    def materialize(identifier: str) -> dict:
        return dict(conn.execute("""SELECT s.*,EXISTS(SELECT 1 FROM gl_bank_statement_voids v
            WHERE v.statement_id=s.id) AS voided FROM gl_bank_statements s WHERE profile_id=? AND id=?""",
            (profile_id, identifier)).fetchone())
    return records_page(conn, profile_id, "statements", limit=limit, cursor=cursor, materialize=materialize)
