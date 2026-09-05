"""Ordinary agent task adapter: opaque progress, never financial disclosure.

The runtime is created per chat/provider/scope. RAM-only review handles expire
and cannot survive a restart. The existing daemon scope and once-only consent
loop must wrap calls; the old tool-free disclosure grants are not involved.
"""
from dataclasses import dataclass, field
from contextlib import contextmanager
import hashlib
import json
import re
import secrets
import time

from .core.accounting import ledger, tasks
from .core.accounting.package import MAX_SNAPSHOT_BYTES
from .errors import AppError

READ_KINDS = frozenset({'ui.accounting.task_get', 'ui.accounting.task_preview'})
WRITE_KINDS = frozenset({'ui.accounting.task_apply', 'ui.accounting.task_cancel'})
MAX_LOCAL_EXPORT_BYTES = MAX_SNAPSHOT_BYTES  # Encoded JSONL sideband, including string escaping.


@contextmanager
def owned_read_transaction(conn):
    """Release task-created read locks without committing or discarding caller work."""
    owned = not conn.in_transaction
    try:
        yield
    finally:
        if owned and conn.in_transaction:
            conn.rollback()


def _fail(code='accounting_task_approval_expired'):
    raise AppError('Refresh the local accounting task and review this action again', code=code)


def _opaque(value):
    return value if isinstance(value, str) and re.fullmatch(r'[a-f0-9]{32}', value) else None


def summary(value):
    """Positive projection: no names, text, amounts, paths or content hashes."""
    task = value.get('task', value)
    statuses = ('ready', 'draft', 'posted', 'covered', 'exception')
    counts = {status: sum(1 for row in task.get('coverage', []) if row.get('status') == status)
              for status in statuses}
    receipts = task.get('receipts', [])
    result = {
        'task_id': _opaque(task.get('id')),
        'state': task.get('state') if task.get('state') in ('active', 'attention', 'cancelled', 'completed') else 'attention',
        'counts': counts,
        'source_count': len(task.get('coverage', [])),
        'next_step': task.get('next_step') if task.get('next_step') in tasks.STEPS else None,
        'receipts': [{'id': _opaque(r.get('id')), 'step': r.get('step') if r.get('step') in tasks.STEPS else None,
                      'artifact_prepared': r.get('result', {}).get('artifact_state') == 'prepared'} for r in receipts],
        'file_saved': False,
    }
    if type(value.get('ready')) is bool:
        result['ready'] = value['ready']
        result['blocker_count'] = len(value.get('blockers', []))
    if type(value.get('already_applied')) is bool:
        result['already_applied'] = value['already_applied']
    return result


@dataclass
class TaskApprovals:
    pending: dict = field(default_factory=dict, repr=False)

    def issue(self, profile_id, reviewed):
        now = time.monotonic()
        self.pending = {k: v for k, v in self.pending.items() if v['expires'] > now}
        if len(self.pending) >= 32:
            _fail('accounting_task_approval_limit')
        handle = secrets.token_urlsafe(32)
        self.pending[handle] = dict(profile_id=profile_id, task_id=reviewed['id'], step=reviewed['step'],
            expected_digest=reviewed['expected_digest'], expected_revision=reviewed['expected_revision'], expires=now+300)
        return handle

    def get(self, profile_id, args):
        grant = self.pending.get(args.get('approval_id'))
        if not grant or grant['expires'] <= time.monotonic() or grant['profile_id'] != profile_id or grant['task_id'] != args.get('task_id'):
            _fail()
        return grant


def consent_preview(conn, profile_id, args, approvals):
    grant = approvals.get(profile_id, args)
    reviewed = tasks.execute(conn, profile_id, 'task-preview', {'task_id': grant['task_id'], 'step': grant['step']})
    if not reviewed['ready'] or reviewed['expected_digest'] != grant['expected_digest'] or reviewed['expected_revision'] != grant['expected_revision']:
        _fail('accounting_stale_approval')
    book = ledger.require_book(conn, profile_id)
    from .core.accounting.commands import wire_values
    return {'status': 'ready', 'step': grant['step'], 'preview': wire_values(reviewed),
            'book': {'currency': book['currency'], 'minor_unit_exponent': book['minor_unit_exponent']}}


def execute(conn, profile_id, kind, args, approvals, *, local_export=None):
    """Only invoked through the daemon's pinned read/mutation callbacks."""
    try:
        if kind not in READ_KINDS | WRITE_KINDS or not isinstance(args, dict) or not _opaque(args.get('task_id')):
            _fail('accounting_task_invalid')
        required = {'task_id'}
        if kind == 'ui.accounting.task_preview':
            required.add('step')
        elif kind == 'ui.accounting.task_apply':
            required |= {'approval_id', 'idempotency_key'}
        if set(args) != required:
            _fail('accounting_task_invalid')
        if kind == 'ui.accounting.task_apply':
            grant = approvals.get(profile_id, args)
            # Consume before execution; failure needs a new preview/approval.
            approvals.pending.pop(args['approval_id'])
            payload = {key: grant[key] for key in ('task_id', 'step', 'expected_digest', 'expected_revision')}
            payload.update(idempotency_key=args['idempotency_key'], confirmed=True)
            if grant['step'] in ('export_close', 'export_tax'):
                payload['confirm_plaintext'] = True
            value = tasks.execute(conn, profile_id, 'task-apply', payload)
            if local_export is not None and grant['step'] in ('export_close', 'export_tax'):
                artifact = value['result']
                encoded = json.dumps(artifact, sort_keys=True, ensure_ascii=False, separators=(',', ':'), allow_nan=False)
                release = dict(task_id=grant['task_id'], step=grant['step'], artifact_json=encoded,
                               sha256=hashlib.sha256(encoded.encode('utf-8')).hexdigest())
                if len(json.dumps(release, ensure_ascii=True).encode('utf-8')) > MAX_LOCAL_EXPORT_BYTES:
                    return {**summary(value), 'delivery_code': 'accounting_export_too_large'}
                local_export.update(release)
            return summary(value)
        if kind == 'ui.accounting.task_cancel':
            return summary(tasks.execute(conn, profile_id, 'task-cancel', {
                'task_id': args['task_id'], 'reason': 'Explicitly approved cancellation through the task assistant'}))
        value = tasks.execute(conn, profile_id, kind.removeprefix('ui.accounting.').replace('_', '-'), args)
        result = summary(value)
        if kind == 'ui.accounting.task_preview' and value['ready']:
            result['approval_id'] = approvals.issue(profile_id, value)
        return result
    except AppError as exc:
        # Domain messages/details may contain selected financial records.
        code = exc.code if isinstance(exc.code, str) and re.fullmatch(r'[a-z_]{1,80}', exc.code) else 'accounting_task_failed'
        _fail(code)
