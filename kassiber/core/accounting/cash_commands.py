"""Explicit local cash-book command contracts; no network or AI dispatch."""
from ...errors import AppError
from . import cashbook, ledger

READ_ACTIONS = frozenset({'cash-snapshot', 'cash-report', 'cash-reconciliation', 'cash-assets'})
WRITE_ACTIONS = frozenset({'cash-select-account', 'cash-count', 'cash-classify', 'cash-void'})


def execute(conn, profile_id, action, payload):
    ledger.require_book(conn, profile_id)
    contracts = {
        'cash-snapshot': (cashbook.snapshot, {'start_date', 'end_date'}),
        'cash-report': (cashbook.report, {'start_date', 'end_date'}),
        'cash-reconciliation': (cashbook.reconciliation, {'as_of'}),
        'cash-assets': (cashbook.asset_statement, {'as_of'}),
        'cash-select-account': (cashbook.select_account, {'account_code', 'role', 'effective_from', 'reason', 'idempotency_key'}),
        'cash-count': (cashbook.retain_count, {'account_code', 'count_date', 'counted_minor', 'evidence_id', 'locator', 'reason', 'idempotency_key'}),
        'cash-classify': (cashbook.classify_flow, {'cash_line_id', 'offset_line_id', 'amount_minor', 'classification', 'evidence_id', 'locator', 'reason', 'idempotency_key'}),
        'cash-void': (cashbook.void_record, {'record_kind', 'record_id', 'reason', 'idempotency_key'}),
    }
    if action not in contracts:
        raise AppError('Unknown cash-book command', code='accounting_unknown_operation')
    call, keys = contracts[action]
    if not isinstance(payload, dict) or set(payload) != keys:
        raise AppError('Invalid cash-book command fields', code='accounting_invalid_fields')
    return call(conn, profile_id, **payload)
