from decimal import Decimal

import pytest

from kassiber.core.custody_tax_projection import FinalizedTaxProjection
from kassiber.core.engines.base import TaxEngineLedgerInputs
from kassiber.core.engines.rp2 import GenericRP2TaxEngine
from tests.test_rp2_ownership_transfers import BTC, PROFILE, WALLET_REFS, _row
from tests.test_accounting_integration import book  # noqa: F401


def row(identity, day, quantity, price, direction='inbound', **extra):
    item = _row('A', direction, quantity, external_id='acq-' + identity)
    item.update(id=identity, occurred_at=day + 'T12:00:00Z', fiat_currency='EUR',
                fiat_rate_exact=str(price), fiat_rate=float(price),
                kind='buy' if direction == 'inbound' else 'sell',
                custody_finalized_tax_projection=True, journal_transaction_id=identity)
    item.update(extra)
    return item


def inputs(rows):
    return TaxEngineLedgerInputs(FinalizedTaxProjection(tuple(rows), (), (), (), ()), WALLET_REFS)


def capture(rows, **kwargs):
    engine = GenericRP2TaxEngine(dict(PROFILE, fiat_currency='EUR', tax_country='AT',
                                     gains_algorithm='moving_average_at', cost_basis_pool_scope='global'))
    return engine.capture_calculation(inputs(rows), cutoff_exclusive_utc='2026-01-01T00:00:00Z',
                                       calculation_timezone='Europe/Vienna', **kwargs)


def test_later_pool_revaluation_cannot_change_cutoff_basis():
    prefix = [row('a', '2024-01-01', BTC, 100), row('b', '2024-02-01', BTC, 300),
              row('sale', '2025-06-01', BTC // 2, 500, 'outbound')]
    later = [row('futurebuy', '2026-02-01', BTC, 900),
             row('futuresale', '2026-03-01', BTC // 2, 500, 'outbound')]
    first, second = capture(prefix), capture(prefix + later)
    assert first == second
    assert not first.blockers
    asset = first.assets[0]
    assert sum(Decimal(item['basis_exact']) for item in asset['open_positions']) == 300
    assert sum(item['quantity_msat'] for item in asset['open_positions']) == BTC * 3 // 2
    assert sum(Decimal(item['basis_exact']) for item in asset['gain_losses']) == 100


def test_subcent_quantity_is_not_dropped():
    result = capture([row('tiny', '2025-01-01', 1, 100)])
    assert not result.blockers
    assert result.assets[0]['open_positions'][0]['quantity_msat'] == 1
    assert Decimal(result.assets[0]['open_positions'][0]['basis_exact']) == Decimal('0.000000001')


def test_same_asset_cross_cutoff_move_retains_transit_not_future_receipt():
    acquisition = row('initial', '2025-01-01', BTC, 100)
    outgoing = row('dispatch', '2025-12-31', BTC // 2, 300, 'outbound', fee=1000)
    incoming = row('receipt', '2026-01-02', BTC // 2, 999999, wallet_id='B')
    projection = FinalizedTaxProjection((acquisition, outgoing, incoming),
        ({'out': outgoing, 'in': incoming, 'pair_id': 'reviewed-pair', 'source': 'manual'},), (), (), ('move',))
    engine = GenericRP2TaxEngine(dict(PROFILE, fiat_currency='EUR', cost_basis_pool_scope='global'))
    result = engine.capture_calculation(TaxEngineLedgerInputs(projection, WALLET_REFS),
        cutoff_exclusive_utc='2026-01-01T00:00:00Z', calculation_timezone='Europe/Vienna')
    assert not result.blockers
    assert len(result.inputs['cutoff_relations']) == 1
    relation = result.inputs['cutoff_relations'][0]
    assert relation['in_id'] == 'receipt'
    balances = {item['wallet_id']: item['quantity_msat'] for item in result.assets[0]['custody_balances']}
    assert balances['A'] == BTC // 2 - 1000
    assert balances.get('B', 0) == 0
    assert balances[relation['transit_wallet_id']] == BTC // 2
    assert all(item['timestamp'] < '2026-01-01' for item in result.inputs['prepared_transactions'])
    assert len(result.assets[0]['acquisitions']) == 1


def test_real_encrypted_capture_maps_custody_identity_and_local_midnight(book):
    from kassiber.core.accounting import artifacts, sources
    from kassiber.core.wallets import create_wallet

    conn, scope, _ = book
    sources.ensure_schema(conn)
    artifacts.ensure_schema(conn)
    workspace_id = conn.execute('SELECT workspace_id FROM profiles WHERE id=?', (scope,)).fetchone()[0]
    wallet = create_wallet(conn, workspace_id, scope, 'Capture source', 'custom')
    conn.execute('''INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,
        occurred_at,direction,asset,amount,fee,fiat_currency,fiat_rate_exact,raw_json,created_at,kind)
        VALUES('tiny',?,?,?,'tiny-source','tiny-fingerprint','2025-01-01T23:30:00Z','inbound',
        'BTC',1,0,'EUR','1','{}','2025-01-01T23:30:00Z','buy')''', (workspace_id, scope, wallet['id']))
    snapshot = sources.capture_sources(conn, scope)
    captured = artifacts.capture_calculation(conn, scope, snapshot_id=snapshot['id'], period_id='2025')
    assert not captured['capture']['blockers']
    assert captured['capture']['cutoff_exclusive_utc'] == '2025-12-31T23:00:00Z'
    acquisition = captured['capture']['assets'][0]['acquisitions'][0]
    assert acquisition['source_ids'] == [snapshot['snapshot']['sources'][0]['source_id']]
    assert acquisition['quantity_msat'] == 1
    assert artifacts.capture_calculation(conn, scope, snapshot_id=snapshot['id'], period_id='2025')['id'] == captured['id']


def test_cross_asset_future_receipt_is_only_basis_reference_not_pool_inventory():
    acquisition = row('initial', '2025-01-01', BTC * 2, 100)
    destination_initial = row('liquidinitial', '2025-01-01', BTC, 300, asset='LBTC', wallet_id='B')
    outgoing = row('dispatch', '2025-12-30', BTC, 999, 'outbound')
    incoming = row('receipt', '2026-01-02', BTC, 999999, asset='LBTC', wallet_id='B')
    destination_sale = row('liquidsale', '2025-12-31', BTC // 2, 500, 'outbound', asset='LBTC', wallet_id='B')
    future_sale = row('futuresale', '2026-01-03', BTC // 2, 500, 'outbound', asset='LBTC', wallet_id='B')
    projection = FinalizedTaxProjection(tuple([acquisition, destination_initial, outgoing, incoming, destination_sale, future_sale]), (),
        ({'out_id': 'dispatch', 'in_id': 'receipt', 'out_asset': 'BTC', 'in_asset': 'LBTC',
          'pair_id': 'reviewed-cross', 'policy': 'carrying-value', 'kind': 'custody_cross_rail'},), (), ('cross',))
    engine = GenericRP2TaxEngine(dict(PROFILE, fiat_currency='EUR', tax_country='AT',
        gains_algorithm='moving_average_at', cost_basis_pool_scope='global'))
    result = engine.capture_calculation(TaxEngineLedgerInputs(projection, WALLET_REFS),
        cutoff_exclusive_utc='2026-01-01T00:00:00Z', calculation_timezone='Europe/Vienna')
    assert not result.blockers
    liquid = next(asset for asset in result.assets if asset['asset'] == 'LBTC')
    assert sum(Decimal(item['basis_exact']) for item in liquid['open_positions']) == 150
    assert sum(Decimal(item['basis_exact']) for item in liquid['gain_losses']) == 150
    assert sum(item['quantity_msat'] for item in liquid['custody_balances']) == BTC // 2
    assert len(liquid['acquisitions']) == 1
    assert result.inputs['cutoff_relations'][0]['basis_carried_exact'] == '100'
    source = next(asset for asset in result.assets if asset['asset'] == 'BTC')
    assert source['gain_losses'][0]['basis_exact'] == '999'
    assert source['transfers'][0]['basis_carried_exact'] == '100'
    fact = next(item for item in result.inputs['execution_basis'] if item['event_id'] == 'dispatch')
    assert fact['unit_basis_exact'] == '100'
    assert fact['display_unit_basis_override_exact'] == '999'


@pytest.mark.parametrize('country,method', [('AT', 'moving_average_at'), ('generic', 'FIFO'), ('generic', 'LIFO')])
def test_execution_observer_leaves_engine_result_identical(country, method):
    from dataclasses import asdict
    from kassiber.core.engines.rp2 import _rp2_configuration

    engine = GenericRP2TaxEngine(dict(PROFILE, fiat_currency='EUR', tax_country=country,
                                     gains_algorithm=method, cost_basis_pool_scope='global'))
    population = inputs([row('one', '2024-01-01', BTC, 100), row('two', '2024-02-01', BTC, 300),
                         row('sale', '2025-01-01', BTC * 3 // 2, 500, 'outbound')])
    with _rp2_configuration(engine.profile, [r['label'] for r in WALLET_REFS.values()], ['BTC']) as configuration:
        first = asdict(engine._build_finalized_ledger_state(population, configuration))
        captured = {}
        second = asdict(engine._build_finalized_ledger_state(population, configuration, calculation_capture=captured))
    for result in (first, second):
        for entry in result['entries']:
            entry.pop('id')  # Journal IDs are independently generated UUIDs.
    assert first == second
    assert len(captured['execution']) == 2
    assert sum(item['quantity_msat'] for item in captured['execution']) == BTC * 3 // 2


@pytest.mark.parametrize('second_day', ['2025-01-01', '2025-01-02'])
def test_cross_asset_chain_retains_execution_or_rejects_existing_same_instant_cycle(second_day):
    from kassiber.errors import AppError
    rows = [row('initial', '2024-01-01', BTC, 100),
            row('aout', '2025-01-01', BTC, 999, 'outbound'),
            row('ain', '2025-01-01', BTC, 999, asset='LBTC', wallet_id='B'),
            row('bout', second_day, BTC, 999, 'outbound', asset='LBTC', wallet_id='B'),
            row('bin', second_day, BTC, 999),
            row('laterbuy', '2025-02-01', BTC, 900),
            row('latersale', '2025-03-01', BTC // 2, 1000, 'outbound')]
    pairs = tuple(dict(pair_id=key, out_id=key+'out', in_id=key+'in', out_asset=out_asset,
                       in_asset=in_asset, policy='carrying-value', kind='custody_cross_rail')
                  for key, out_asset, in_asset in [('a', 'BTC', 'LBTC'), ('b', 'LBTC', 'BTC')])
    projection = FinalizedTaxProjection(tuple(rows), (), pairs, (), ('a','b'))
    engine = GenericRP2TaxEngine(dict(PROFILE, fiat_currency='EUR', tax_country='AT',
        gains_algorithm='moving_average_at', cost_basis_pool_scope='global'))
    population = TaxEngineLedgerInputs(projection, WALLET_REFS)
    if second_day == '2025-01-01':
        for calculate in (lambda: engine.build_ledger_state(population), lambda: engine.capture_calculation(population,
            cutoff_exclusive_utc='2026-01-01T00:00:00Z', calculation_timezone='Europe/Vienna')):
            with pytest.raises(AppError, match='Cyclic Austrian swap basis dependency'):
                calculate()
        return
    result = engine.capture_calculation(population,
        cutoff_exclusive_utc='2026-01-01T00:00:00Z', calculation_timezone='Europe/Vienna')
    assert not result.blockers
    assert {transfer['basis_carried_exact'] for asset in result.assets for transfer in asset['transfers']} == {'100'}
    btc = next(asset for asset in result.assets if asset['asset'] == 'BTC')
    later = next(item for item in btc['acquisitions'] if item['rp2_unique_id'] == 'bin')
    assert Decimal(later['effective_basis_exact']) == 500


def test_capture_rejects_naive_cutoff_and_unknown_calendar():
    from kassiber.errors import AppError
    engine = GenericRP2TaxEngine(PROFILE)
    for cutoff, calendar in [('2025-12-31T00:00:00', 'Europe/Vienna'), ('2026-01-01T00:00:00Z', 'Not/AZone')]:
        with pytest.raises(AppError) as exc:
            engine.capture_calculation(inputs([]), cutoff_exclusive_utc=cutoff, calculation_timezone=calendar)
        assert exc.value.code == 'accounting_calculation_time'


def test_changed_dependency_contract_fails_typed_without_global_patch(monkeypatch):
    from rp2.accounting_engine import AccountingEngine
    from kassiber.errors import AppError
    monkeypatch.setattr(AccountingEngine, 'get_acquired_lot_for_taxable_event', lambda self: None)
    with pytest.raises(AppError) as exc:
        capture([row('buy','2025-01-01',BTC,100),row('sell','2025-02-01',BTC,200,'outbound')])
    assert exc.value.code == 'accounting_calculation_dependency'
