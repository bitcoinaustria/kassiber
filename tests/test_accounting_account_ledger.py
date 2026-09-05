import base64
import json

import pytest

from kassiber.core.accounting import ledger as gl
from kassiber.core.accounting.account_ledger import account_ledger
from kassiber.errors import AppError
from tests.test_accounting_ledger import conn, payload, post  # noqa: F401


def test_opening_period_totals_and_paged_running_balances(conn):
    post(conn, payload('old', amount=100))
    gl.create_period(conn, 'p', period_id='2026', start_date='2026-01-01', end_date='2026-12-31')
    post(conn, payload('one', amount=30, period='2026', entry_date='2026-02-01'))
    post(conn, payload('two', amount=70, period='2026', entry_date='2026-03-01'))
    gl.create_draft(conn, 'p', payload('unposted', amount=200, period='2026', entry_date='2026-04-01'))
    page = account_ledger(conn, 'p', account_code='1000', period_id='2026', limit=1)
    assert (page['opening_minor'], page['debit_minor'], page['credit_minor'], page['closing_minor']) == (100, 100, 0, 200)
    assert page['total_count'] == 2
    assert page['rows'][0]['running_balance_minor'] == 130
    tail = account_ledger(conn, 'p', account_code='1000', period_id='2026', limit=1, cursor=page['next_cursor'])
    assert tail['rows'][0]['running_balance_minor'] == 200
    assert tail['next_cursor'] is None
    assert tail['closing_minor'] == page['closing_minor']


def test_reversal_account_without_activity_and_exact_large_turnover(conn):
    original = post(conn, payload(amount=2**62))
    post(conn, payload('second', amount=2**62))
    gl.reverse_entry(conn, 'p', entry_id=original['id'], entry_date='2025-02-02', period_id='2025', idempotency_key='undo', reason='Correction')
    result = account_ledger(conn, 'p', account_code='1000', period_id='2025')
    assert result['debit_minor'] == 2**63
    assert result['credit_minor'] == result['closing_minor'] == 2**62
    assert result['rows'][-1]['running_balance_minor'] == 2**62
    empty = account_ledger(conn, 'p', account_code='5000', period_id='2025')
    assert empty['rows'] == [] and empty['closing_minor'] == 0


def test_cursor_scope_staleness_and_forged_position(conn):
    post(conn)
    post(conn, payload('next', entry_date='2025-03-01'))
    token = account_ledger(conn, 'p', account_code='1000', period_id='2025', limit=1)['next_cursor']
    with pytest.raises(AppError) as error:
        account_ledger(conn, 'p', account_code='4000', period_id='2025', cursor=token)
    assert error.value.code == 'accounting_stale_cursor'
    forged = json.loads(base64.urlsafe_b64decode(token))
    forged['last'][1] = 'not-an-entry'
    with pytest.raises(AppError) as error:
        account_ledger(conn, 'p', account_code='1000', period_id='2025', cursor=base64.urlsafe_b64encode(json.dumps(forged).encode()).decode())
    assert error.value.code == 'accounting_invalid_cursor'
    gl.create_draft(conn, 'p', payload('changed'))
    with pytest.raises(AppError) as error:
        account_ledger(conn, 'p', account_code='1000', period_id='2025', cursor=token)
    assert error.value.code == 'accounting_stale_cursor'
    with pytest.raises(AppError):
        account_ledger(conn, 'other', account_code='1000', period_id='2025')


@pytest.mark.parametrize('limit', [True, 0, -1, 501])
def test_invalid_limit(conn, limit):
    with pytest.raises(AppError):
        account_ledger(conn, 'p', account_code='1000', period_id='2025', limit=limit)
