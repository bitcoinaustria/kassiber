"""Real RP2 rail carries expose original retained sources without new allocation."""
import json
from decimal import Decimal

import pytest

from kassiber.cli.handlers import create_transaction_pair, process_journals
from kassiber.core import report_explanation_carry
from kassiber.core.report_explanation import explain_capital_gain, result_reference
from kassiber.db import open_db
from tests.custody_tax_helpers import persist_authoritative_chain_observation


@pytest.fixture
def carried_book(tmp_path, request):
    conn = open_db(tmp_path)
    conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('w','W','2020')")
    conn.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,gains_algorithm,created_at) VALUES('p','w','P','USD','generic','FIFO','2020')")
    for wallet, chain in [('btc', 'bitcoin'), ('liquid', 'liquid')]:
        conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES(?,'w','p',?,'descriptor',?,'2020')", (wallet, wallet, json.dumps({'chain': chain, 'network': 'main'})))
    transactions = [
        ('buy', 'btc', '2024-01-01', 'inbound', 'BTC', 10010000000, 0, 80000, 'buy'),
        ('swap-out', 'btc', '2024-02-01', 'outbound', 'BTC', 10000000000, 10000000, 82000, 'withdrawal'),
        ('swap-in', 'liquid', '2024-02-01', 'inbound', 'LBTC', 10000000000, 0, 82000, 'deposit'),
        ('sell', 'liquid', '2025-01-01', 'outbound', 'LBTC', 10000000000, 0, 83000, 'sell'),
    ]
    two_hops = getattr(request, 'param', False)
    if two_hops:
        transactions[-1] = ('sell', 'btc', '2025-01-01', 'outbound', 'BTC', 10000000000, 0, 83000, 'sell')
        transactions.extend([
            ('return-out', 'liquid', '2024-03-01', 'outbound', 'LBTC', 10000000000, 0, 82000, 'withdrawal'),
            ('return-in', 'btc', '2024-03-01', 'inbound', 'BTC', 10000000000, 0, 82000, 'deposit'),
        ])
    for index, (ident, wallet, date, direction, asset, amount, fee, rate, kind) in enumerate(transactions, start=1):
        txid = f'{index:064x}'
        chain = 'bitcoin' if wallet == 'btc' else 'liquid'
        raw = {'txid': txid, 'chain': chain, 'network': 'main',
               'status': {'confirmed': True, 'block_hash': '11' * 32, 'block_height': 100},
               'vin': [{'txid': f'{index + 5:064x}', 'vout': 0, 'prevout': {'scriptpubkey': '0014' + 'ff' * 20, 'value': 10000000}}],
               'vout': [{'n': 0, 'scriptpubkey': '0014' + 'ee' * 20, 'value': 10000000}]}
        conn.execute("""INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,external_id_kind,
            fingerprint,occurred_at,direction,asset,amount,fee,fiat_currency,fiat_rate,fiat_rate_exact,kind,raw_json,created_at)
            VALUES(?,'w','p',?,?,'txid',?,?,?,?,?,?,'USD',?,?,?,?,'2020')""",
                     (ident, wallet, txid, ident, date + 'T00:00:00Z', direction, asset, amount, fee, rate, str(rate), kind, json.dumps(raw)))
        persist_authoritative_chain_observation(conn, ident, observer_kind='bitcoinrpc' if chain == 'bitcoin' else 'lwk')
    conn.commit()
    create_transaction_pair(conn, 'W', 'P', 'swap-out', 'swap-in', kind='peg-in', policy='carrying-value')
    if two_hops:
        create_transaction_pair(conn, 'W', 'P', 'return-out', 'return-in', kind='peg-out', policy='carrying-value')
    process_journals(conn, 'W', 'P')
    assert not conn.execute('SELECT * FROM journal_quarantines').fetchall()
    # A reviewed conversion is not a MOVE decision; read the canonical union.
    assert not conn.execute('SELECT * FROM journal_custody_decisions').fetchall()
    profile = conn.execute('SELECT * FROM profiles').fetchone()
    row = conn.execute("SELECT * FROM journal_entries WHERE transaction_id='sell' AND entry_type='disposal'").fetchone()
    yield conn, profile, result_reference(conn, profile, row['id'])
    conn.close()


def test_cross_rail_sale_follows_real_retained_engine_lots_and_fees(carried_book):
    conn, profile, reference = carried_book
    before = conn.total_changes
    result = explain_capital_gain(conn, profile, reference)
    assert conn.total_changes == before
    assert result['status'] == 'available'
    assert Decimal(result['totals']['cost_basis_exact']) == 8008
    assert Decimal(result['totals']['gain_loss_exact']) == 292
    lot = result['calculation']['fragments'][0]['lot']
    assert lot['transaction_id'] == 'swap-in'
    inherited = lot['inherited_basis']
    assert inherited['status'] == 'available'
    relation, = inherited['relations']
    assert relation['source_transaction_id'] == 'swap-out'
    assert relation['target_transaction_id'] == 'swap-in'
    assert relation['policy'] == 'carrying-value'
    assert relation['swap_fee_msat_exact'] == '10000000'
    assert result['custody_decisions'] == [relation]
    source, = inherited['source_calculations']
    assert source['scope'] == 'whole_source_disposal'
    assert source['totals']['quantity_msat_exact'] == '10010000000'
    fragment, = source['calculation']['fragments']
    assert Decimal(fragment['cost_basis_exact']) == 8008
    assert fragment['lot']['transaction_id'] == 'buy'
    assert fragment['lot']['spot_price_exact'] == '80000'
    assert fragment['event']['crypto_fee_msat_exact'] == '10000000'
    assert fragment['event']['fiat_fee_exact'] == '8.0080'
    stored = json.loads(conn.execute('SELECT calculation_json FROM journal_entries WHERE id=?', (reference['entry_id'],)).fetchone()[0])
    assert 'inherited_basis' not in stored['fragments'][0]['lot']


@pytest.mark.parametrize('corruption,status', [(None, 'engine_detail_unavailable'), ('{}', 'engine_detail_mismatch')])
def test_missing_or_invalid_inherited_detail_is_explicit(carried_book, corruption, status):
    conn, profile, reference = carried_book
    conn.execute("UPDATE journal_entries SET calculation_json=? WHERE transaction_id='swap-out'", (corruption,))
    conn.commit()
    result = explain_capital_gain(conn, profile, reference)
    assert result['status'] == 'available'  # The displayed sale still reconciles.
    inherited = result['calculation']['fragments'][0]['lot']['inherited_basis']
    assert inherited['status'] == status
    assert inherited['source_calculations'][0]['calculation'] is None
    assert len(inherited['relations']) == 1


def test_source_trace_is_bounded_and_does_not_invent_pool_sources(carried_book, monkeypatch):
    conn, profile, reference = carried_book
    monkeypatch.setattr(report_explanation_carry, '_MAX_RECORDS', 1)
    result = explain_capital_gain(conn, profile, reference)
    inherited = result['calculation']['fragments'][0]['lot']['inherited_basis']
    assert inherited['status'] == 'inherited_detail_truncated'
    assert inherited['source_calculations'] == []
    assert result['custody_truncated']


def test_recursive_source_trace_stops_on_cycle(carried_book):
    conn, profile, reference = carried_book
    row = conn.execute("SELECT * FROM journal_entries WHERE transaction_id='swap-out'").fetchone()
    calculation = json.loads(row['calculation_json'])
    lot = calculation['fragments'][0]['lot']
    lot.update(transaction_id='swap-in', asset='LBTC')
    lot['pricing']['pricing_method'] = 'carrying_value'
    conn.execute('UPDATE journal_entries SET calculation_json=? WHERE id=?', (json.dumps(calculation), row['id']))
    conn.commit()
    result = explain_capital_gain(conn, profile, reference)
    inherited = result['calculation']['fragments'][0]['lot']['inherited_basis']
    nested = inherited['source_calculations'][0]['calculation']['fragments'][0]['lot']['inherited_basis']
    assert nested['status'] == 'inherited_detail_truncated'
    assert nested['source_calculations'] == []
    assert result['custody_truncated']


@pytest.mark.parametrize('carried_book', [True], indirect=True)
def test_successive_real_carries_retain_original_source_recursively(carried_book):
    conn, profile, reference = carried_book
    result = explain_capital_gain(conn, profile, reference)
    assert Decimal(result['totals']['gain_loss_exact']) == 292
    inherited = result['calculation']['fragments'][0]['lot']['inherited_basis']
    assert inherited['status'] == 'available'
    assert inherited['relations'][0]['source_transaction_id'] == 'return-out'
    intermediate = inherited['source_calculations'][0]['calculation']['fragments'][0]['lot']
    assert intermediate['transaction_id'] == 'swap-in'
    original = intermediate['inherited_basis']['source_calculations'][0]['calculation']['fragments'][0]['lot']
    assert original['transaction_id'] == 'buy'
    assert original['spot_price_exact'] == '80000'
    assert {relation['source_transaction_id'] for relation in result['custody_decisions']} == {'swap-out', 'return-out'}
    assert not result['custody_truncated']
