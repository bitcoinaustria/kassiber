"""Custody admission and legacy projection must never emit nonfinite prices."""
import json
import subprocess
import sys

import pytest

from kassiber.cli.handlers import create_direct_swap_payout
from kassiber.core.custody_components import normalize_economic_terms
from kassiber.core.custody_tax_projection import _direct_payout_settlement_rows, _projected_slice_row
from kassiber.core.custody_quantity import QuantitySlice
from kassiber.core.imports import normalize_import_record
from kassiber.daemon import _ui_swap_matching_payload_from_conn
from kassiber.db import open_db
from kassiber.errors import AppError
from tests.test_custody_authored_migration import _scope, _tx, _legacy_rows, NOW
from kassiber.core.custody_journal import CustodyJournalBuilder
from tests.test_custody_quantity import _observation


@pytest.mark.parametrize('bad', ['Infinity', '-Infinity', 'NaN', 'sNaN', '1e999', 'junk'])
@pytest.mark.parametrize('surface', ['handler', 'cli', 'daemon'])
def test_payout_admission_is_atomic(tmp_path, bad, surface):
    conn = open_db(tmp_path)
    try:
        _scope(conn)
        _tx(conn, 'out', 'btc', 'outbound', 'BTC', 100000000000, NOW)
        conn.commit()
        before = conn.total_changes
        args = dict(payout_asset='BTC', payout_amount='1', payout_fiat_value=bad)
        if surface == 'cli':
            result = subprocess.run(
                [sys.executable, '-m', 'kassiber', '--data-root', str(tmp_path),
                 '--machine', 'transfers', 'payouts', 'create',
                 '--workspace', 'Books', '--profile', 'Book', '--tx-out', 'out',
                 '--payout-asset', 'BTC', '--payout-amount', '1',
                 '--payout-fiat-value=' + bad],
                capture_output=True, text=True,
            )
            payload = json.loads(result.stdout)
            assert payload['error']['code'] == 'validation', payload
        else:
            with pytest.raises(AppError) as caught:
                if surface == 'handler':
                    create_direct_swap_payout(conn, 'Books', 'Book', 'out', **args)
                else:
                    _ui_swap_matching_payload_from_conn(conn, 'ui.transfers.payouts.create', dict(workspace='Books', profile='Book', tx_out='out', **args))
            assert caught.value.code == 'validation'
            assert conn.total_changes == before
        assert conn.execute('SELECT count(*) FROM custody_components').fetchone()[0] == 0
    finally:
        conn.close()


@pytest.mark.parametrize('bad', ['Infinity', '-Infinity', 'NaN', 'sNaN', '1e999', 'junk'])
def test_legacy_settlement_refuses_bad_price(bad):
    source = _observation('source', 'wallet', 'outbound', 100)
    with pytest.raises(AppError) as caught:
        _direct_payout_settlement_rows(source, dict(payout_amount=1, payout_fiat_value=bad), {})
    assert caught.value.code == 'validation'


@pytest.mark.parametrize('field', ['fiat_rate', 'fiat_value'])
@pytest.mark.parametrize('bad', ['Infinity', 'NaN', '1e999'])
def test_import_price_admission(field, bad):
    with pytest.raises(AppError) as caught:
        normalize_import_record(dict(amount='1', occurred_at=NOW, **{field: bad}))
    assert caught.value.code == 'validation'


@pytest.mark.parametrize('bad', ['Infinity', 'NaN', '1e999', 'junk'])
def test_component_terms_reject_bad_price(bad):
    legs = [dict(id='source', ordinal=0, role='source'), dict(id='target', ordinal=1, role='external_sink')]
    term = dict(source_leg_id='source', target_leg_id='target', term_kind='direct_swap_payout', source_row_hash='a'*64, payout_fiat_value_exact=bad)
    with pytest.raises(AppError) as caught:
        normalize_economic_terms([term], legs)
    assert caught.value.code == 'validation'


@pytest.mark.parametrize('field', ['fiat_rate', 'fiat_rate_exact', 'fiat_value', 'fiat_value_exact'])
@pytest.mark.parametrize('bad', ['Infinity', 'NaN', 'sNaN', '1e999'])
def test_legacy_observation_pricing_refused(field, bad):
    source = _observation('source', 'wallet', 'outbound', 100)
    with pytest.raises(AppError) as caught:
        _projected_slice_row(source, QuantitySlice(source.quantity_hash, 0, 100),
                             {'source': {'amount': 100, field: bad}}, side='external')
    assert caught.value.code == 'validation'


def test_finite_value_with_overflowing_settlement_rate_refused():
    source = _observation('source', 'wallet', 'outbound', 100)
    with pytest.raises(AppError) as caught:
        _direct_payout_settlement_rows(source, dict(payout_amount=1, payout_fiat_value='1e308'), {})
    assert caught.value.code == 'validation'


def test_valid_settlement_keeps_exact_price():
    source = _observation('source', 'wallet', 'outbound', 100)
    inbound, outbound = _direct_payout_settlement_rows(source, dict(payout_amount=100000000000, payout_fiat_value='12.50'), {})
    assert inbound['fiat_rate_exact'] == '12.50'
    assert outbound['fiat_value_exact'] == '12.50'


@pytest.mark.parametrize('bad', ['Infinity', '-Infinity', 'NaN', '1e999'])
def test_legacy_book_bad_payout_becomes_journal_blocker(tmp_path, bad):
    conn = open_db(tmp_path)
    _legacy_rows(conn)
    conn.execute("UPDATE direct_swap_payouts SET payout_fiat_value = ? WHERE id = 'payout'", (bad,))
    conn.commit()
    conn.close()
    migrated = open_db(tmp_path)
    try:
        issue = migrated.execute("SELECT * FROM custody_authored_migration_issues WHERE legacy_source_id = 'payout' AND resolved_at IS NULL").fetchone()
        assert issue is not None
        profile = migrated.execute("SELECT * FROM profiles WHERE id = 'profile'").fetchone()
        decisions = CustodyJournalBuilder(migrated, profile).build_custody_decisions()
        assert 'payout-out' in decisions.interpretation.blocked_transaction_ids
        assert decisions.direct_payout_records == []
        assert any(q['reason'] == 'custody_authored_migration_incomplete' for q in decisions.interpretation.quarantines)
    finally:
        migrated.close()


@pytest.mark.parametrize('policy,asset', [('taxable', 'BTC'), ('carrying-value', 'LBTC')])
@pytest.mark.parametrize('bad', ['Infinity', 'NaN', '1e308'])
def test_finalized_projection_refuses_bad_stored_payout(policy, asset, bad):
    from tests.custody_tax_helpers import finalized_tax_inputs
    from tests.test_custody_pre_tax_projection import _row

    row = _row('source', 'wallet', 'outbound', 100, NOW)
    with pytest.raises(AppError) as caught:
        finalized_tax_inputs(
            dict(id='profile', tax_country='generic'), rows=[row],
            wallet_refs_by_id={'wallet': {'label': 'wallet'}},
            direct_payout_records=[dict(
                id='payout', out_transaction_id='source', policy=policy,
                payout_asset=asset, payout_amount=100, payout_fiat_value=bad,
                payout_occurred_at=NOW,
            )],
        )
    assert caught.value.code == 'validation'


@pytest.mark.parametrize('field', ['fiat_rate', 'fiat_value'])
def test_import_batch_rejects_bad_price_before_writes(tmp_path, field):
    from kassiber.core.imports import insert_wallet_records, ImportCoordinatorHooks

    conn = open_db(tmp_path)
    try:
        _scope(conn)
        conn.commit()
        profile = conn.execute("SELECT * FROM profiles WHERE id = 'profile'").fetchone()
        wallet = conn.execute("SELECT * FROM wallets WHERE id = 'btc'").fetchone()
        before = conn.total_changes
        with pytest.raises(AppError) as caught:
            insert_wallet_records(conn, profile, wallet,
                [dict(amount='1', occurred_at=NOW, **{field: 'Infinity'})],
                'custom', ImportCoordinatorHooks(lambda *_: None, lambda *_: None))
        assert caught.value.code == 'validation'
        assert conn.total_changes == before
        assert conn.execute('SELECT count(*) FROM transactions').fetchone()[0] == 0
    finally:
        conn.close()


@pytest.mark.parametrize('bad', ['Infinity', 'NaN'])
def test_component_write_rejects_nonfinite_terms_atomically(tmp_path, bad):
    from kassiber.core.custody_components import create_component, get_component

    source = open_db(tmp_path / 'source')
    target = open_db(tmp_path / 'target')
    try:
        for conn in (source, target):
            _scope(conn)
            _tx(conn, 'out', 'btc', 'outbound', 'BTC', 100000000000, NOW)
            conn.commit()
        payout = create_direct_swap_payout(source, 'Books', 'Book', 'out',
            payout_asset='BTC', payout_amount='1', payout_fiat_value='10')
        spec = get_component(source, payout['component_id'])
        spec['economic_terms'][0]['payout_fiat_value_exact'] = bad
        before = target.total_changes
        with pytest.raises(AppError) as caught:
            create_component(target, workspace_id='ws', profile_id='profile',
                component_type=spec['component_type'], legs=spec['legs'],
                allocations=spec['allocations'], economic_terms=spec['economic_terms'],
                created_at=NOW)
        assert caught.value.code == 'validation'
        assert target.total_changes == before
        assert target.execute('SELECT count(*) FROM custody_components').fetchone()[0] == 0
    finally:
        source.close()
        target.close()



def test_payout_admission_rejects_derived_rate_overflow(tmp_path):
    conn = open_db(tmp_path)
    try:
        _scope(conn)
        _tx(conn, 'out', 'btc', 'outbound', 'BTC', 100, NOW)
        conn.commit()
        before = conn.total_changes
        with pytest.raises(AppError) as caught:
            create_direct_swap_payout(conn, 'Books', 'Book', 'out',
                payout_asset='BTC', payout_amount='0.00000000001',
                payout_fiat_value='1e308')
        assert caught.value.code == 'validation'
        assert conn.total_changes == before
    finally:
        conn.close()


def test_sliced_absolute_value_refuses_derived_rate_overflow():
    source = _observation('source', 'wallet', 'outbound', 100)
    with pytest.raises(AppError) as caught:
        _projected_slice_row(source, QuantitySlice(source.quantity_hash, 0, 100),
            {'source': {'amount': 100, 'fiat_value': '1e308'}}, side='external')
    assert caught.value.code == 'validation'
