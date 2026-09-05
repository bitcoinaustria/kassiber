"""Bounded exact-batch review and atomic posting through the normal ledger guard."""
import json
from uuid import uuid4

from ...errors import AppError
from . import ledger


def ensure_schema(conn):
    conn.execute('''CREATE TABLE IF NOT EXISTS gl_posting_batches(
        id TEXT PRIMARY KEY,profile_id TEXT NOT NULL REFERENCES gl_books(profile_id),
        idempotency_key TEXT NOT NULL,approval_digest TEXT NOT NULL,entry_ids_json TEXT NOT NULL,
        reason TEXT NOT NULL,created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')),
        UNIQUE(profile_id,idempotency_key))''')
    for action in ('UPDATE','DELETE'):
        conn.execute(f'''CREATE TRIGGER IF NOT EXISTS gl_posting_batches_no_{action.lower()}
            BEFORE {action} ON gl_posting_batches BEGIN SELECT RAISE(ABORT,'accounting_posting_batch_retained'); END''')
    conn.execute('''CREATE TRIGGER IF NOT EXISTS gl_posting_batches_no_replace BEFORE INSERT ON gl_posting_batches
        WHEN EXISTS(SELECT 1 FROM gl_posting_batches WHERE id=NEW.id OR (profile_id=NEW.profile_id AND idempotency_key=NEW.idempotency_key))
        BEGIN SELECT RAISE(ABORT,'accounting_posting_batch_retained'); END''')


def preview(conn, profile_id, *, draft_ids):
    book = ledger.require_book(conn, profile_id)
    if not isinstance(draft_ids, list) or not 1 <= len(draft_ids) <= 50 or any(not isinstance(x, str) or not x or len(x) > 128 for x in draft_ids) or len(set(draft_ids)) != len(draft_ids):
        raise AppError('Select one to fifty distinct drafts', code='accounting_batch_invalid')
    entries = [ledger._entry(conn, profile_id, key) for key in draft_ids]
    if any(entry['status'] != 'draft' for entry in entries):
        raise AppError('A batch can only contain unposted drafts', code='accounting_batch_invalid')
    conn.execute('SAVEPOINT accounting_batch_preview')
    try:
        for entry in entries:
            ledger.post_draft(conn, profile_id, draft_id=entry['id'], expected_digest=entry['payload_digest'])
    finally:
        conn.execute('ROLLBACK TO accounting_batch_preview')
        conn.execute('RELEASE accounting_batch_preview')
    approval = {'profile_id': profile_id, 'revision': book['revision'],
        'drafts': [{'id': entry['id'], 'payload_digest': entry['payload_digest']} for entry in entries]}
    return {'entries': entries, 'expected_revision': book['revision'], 'expected_digest': ledger.digest(approval)}


def post(conn, profile_id, *, draft_ids, expected_revision, expected_digest, idempotency_key, reason):
    ledger.require_book(conn, profile_id)
    ledger._text(idempotency_key, 'idempotency_key', maximum=128)
    ledger._text(reason, 'reason', maximum=2000)
    with ledger.atomic(conn):
        existing = ledger._row(conn, 'SELECT * FROM gl_posting_batches WHERE profile_id=? AND idempotency_key=?', (profile_id, idempotency_key))
        if existing:
            if existing['approval_digest'] != expected_digest or json.loads(existing['entry_ids_json']) != draft_ids or existing['reason'] != reason:
                raise AppError('Batch retry differs from its approval', code='accounting_idempotency_conflict')
            return {'batch_id': existing['id'], 'posted_ids': draft_ids}
        if type(expected_revision) is not int or ledger.require_book(conn, profile_id)['revision'] != expected_revision:
            raise AppError('Book changed after batch review', code='accounting_stale_approval')
        reviewed = preview(conn, profile_id, draft_ids=draft_ids)
        if expected_digest != reviewed['expected_digest']:
            raise AppError('Batch payload changed after review', code='accounting_stale_approval')
        for entry in reviewed['entries']:
            ledger.post_draft(conn, profile_id, draft_id=entry['id'], expected_digest=entry['payload_digest'])
        identifier = uuid4().hex
        conn.execute('''INSERT INTO gl_posting_batches(id,profile_id,idempotency_key,approval_digest,entry_ids_json,reason)
            VALUES(?,?,?,?,?,?)''', (identifier, profile_id, idempotency_key, expected_digest, ledger.canonical_json(draft_ids), reason))
        return {'batch_id': identifier, 'posted_ids': draft_ids}
