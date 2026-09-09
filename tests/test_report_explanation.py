"""Explanations retain actual engine fragments and reject crossed/stale scopes."""
import json
from decimal import Decimal

import pytest

from kassiber.core import custody_journal
from kassiber.core.report_explanation import explain_capital_gain, result_reference
from kassiber.db import open_db
from kassiber.errors import AppError


def _book(tmp_path, method="fifo", country="generic"):
    conn = open_db(tmp_path)
    conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('w','W','2020')")
    conn.execute("INSERT INTO profiles(id,workspace_id,label,gains_algorithm,tax_country,fiat_currency,created_at) VALUES('p','w','P',?,?,'EUR','2020')", (method, country))
    conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('wallet','w','p','Wallet','manual','2020')")
    for tx, date, direction, amount, fee, price, kind in (
        ('a','2024-01-01','inbound',100000000000,0,'10000','buy'),
        ('b','2024-02-01','inbound',100000000000,0,'20000','buy'),
        ('s','2024-03-01','outbound',150000000000,1000000,'30000','sell'),
    ):
        conn.execute("""INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,fee,kind,fiat_currency,fiat_rate,fiat_rate_exact,pricing_source_kind,pricing_quality,pricing_provider,raw_json,created_at)
        VALUES(?,'w','p','wallet',?,? ,?,'BTC',?,?,?,'EUR',?,?,'manual','exact','reviewed-receipt','{}','2020')""", (tx,tx,date+'T00:00:00Z',direction,amount,fee,kind,float(price),price))
    conn.commit()
    profile = conn.execute("SELECT * FROM profiles").fetchone()
    state = custody_journal.build_ledger_state(conn, profile)
    assert state['quarantines'] == []
    custody_journal.store_ledger_state(conn, profile, state)
    conn.commit()
    profile = conn.execute("SELECT * FROM profiles").fetchone()
    row = conn.execute("SELECT * FROM journal_entries WHERE entry_type='disposal'").fetchone()
    return conn, profile, result_reference(conn, profile, row['id'])


@pytest.mark.parametrize('method,country', [('fifo','generic'),('lifo','generic'),('hifo','generic'),('lofo','generic'),('moving_average_at','at')])
def test_actual_engine_fragments_reconcile_without_read_writes(tmp_path, method, country):
    conn, profile, reference = _book(tmp_path, method, country)
    try:
        before = conn.total_changes
        result = explain_capital_gain(conn, profile, reference)
        assert result['status'] == 'available'
        assert conn.total_changes == before
        assert not conn.in_transaction
        fragments = result['calculation']['fragments']
        assert len(fragments) >= 2
        for key in ('proceeds_exact','cost_basis_exact','gain_loss_exact'):
            assert sum(Decimal(f[key]) for f in fragments) == Decimal(result['totals'][key])
        assert {f['lot']['transaction_id'] for f in fragments} == {'a','b'}
        assert all(f['event']['transaction_id'] == 's' for f in fragments)
        assert fragments[0]['event']['crypto_fee_msat'] == 1000000
        assert fragments[0]['event']['pricing']['pricing_provider'] == 'reviewed-receipt'
        if method == 'moving_average_at':
            assert any(f['unit_basis_override_exact'] is not None for f in fragments)
            assert {Decimal(f['acquisition_basis_exact']) for f in fragments} == {Decimal(10000), Decimal(20000)}
    finally:
        conn.close()


def test_wrong_book_version_rebuild_and_legacy_fail_explicitly(tmp_path):
    conn, profile, reference = _book(tmp_path)
    try:
        for field, value in [('database_id','0'*32),('profile_id','other'),('input_version',8),('processed_at','old'),('entry_id','missing')]:
            with pytest.raises(AppError) as raised:
                explain_capital_gain(conn, profile, {**reference,field:value})
            assert raised.value.code == 'report_explanation_stale'
        conn.execute("UPDATE journal_entries SET calculation_json=NULL")
        conn.commit()
        assert explain_capital_gain(conn, profile, reference)['status'] == 'engine_detail_unavailable'
        conn.execute("UPDATE profiles SET journal_input_version=journal_input_version+1")
        conn.commit()
        with pytest.raises(AppError) as raised:
            explain_capital_gain(conn, profile, reference)
        assert raised.value.code == 'report_explanation_stale'
    finally:
        conn.close()


def test_corrupt_detail_does_not_claim_reconciliation(tmp_path):
    conn, profile, reference = _book(tmp_path)
    try:
        record = json.loads(conn.execute("SELECT calculation_json FROM journal_entries WHERE id=?", (reference['entry_id'],)).fetchone()[0])
        record['fragments'][0]['gain_loss_exact']='999999'
        conn.execute("UPDATE journal_entries SET calculation_json=? WHERE id=?", (json.dumps(record),reference['entry_id']))
        conn.commit()
        result = explain_capital_gain(conn, profile, reference)
        assert result['status'] == 'engine_detail_mismatch'
        assert result['calculation'] is None
    finally:
        conn.close()
