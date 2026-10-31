"""Regression coverage for the Python pricing and partial-payout review."""
import json
from contextlib import ExitStack
from dataclasses import replace
from decimal import Decimal
from types import SimpleNamespace

import pytest

from kassiber.cli.handlers import create_direct_swap_payout, process_journals
from kassiber.core import commercial, custody_tax_projection as ctp, metadata, review_workflow
from kassiber.core.custody_journal import CustodyJournalBuilder
from kassiber.core.custody_quantity import QuantitySlice
from kassiber.core.custody_review_terms import delete_payout_review, list_payout_reviews
from kassiber.core.imports import normalize_import_record
from kassiber.core.rates import set_manual_rate, upsert_rate
from kassiber.core.source_funds import create_source
from kassiber.db import open_db
from kassiber.errors import AppError
from tests import cli_runner
from tests.test_btcpay_commercial_provenance import _hooks
from tests.test_custody_authored_migration import _scope, _tx, NOW


@pytest.fixture
def book(tmp_path):
    conn = open_db(tmp_path)
    _scope(conn)
    _tx(conn, 'out', 'btc', 'outbound', 'BTC', 100000, NOW)
    conn.commit()
    yield conn
    conn.close()


BAD = ['NaN', 'sNaN', 'Infinity', '1e400', '1e999', '1e-20', '1e-300', '1e-999999999',
       '1e19', '1e40', '1e300']


@pytest.mark.parametrize('value', BAD)
@pytest.mark.parametrize('path', ['metadata', 'review', 'rate', 'rate_cache', 'payout', 'import', 'document', 'source'])
def test_price_writes_reject_unsupported_values(book, value, path):
    with pytest.raises(AppError) as error:
        if path == 'metadata':
            metadata._clean_decimal_or_none(value, 'fiat_rate')
        elif path == 'review':
            review_workflow._operations([dict(type='price_override', transaction_id='out', reason='review', fiat_rate=value)])
        elif path == 'rate':
            set_manual_rate(book, 'BTC-EUR', NOW, value)
        elif path == 'rate_cache':
            upsert_rate(book, 'BTC-EUR', NOW, value, 'test')
        elif path == 'payout':
            create_direct_swap_payout(book, 'Books', 'Book', 'out', payout_asset='EUR', payout_amount='1', payout_fiat_value=value)
        elif path == 'import':
            normalize_import_record(dict(amount='1', occurred_at=NOW, fiat_rate=value))
        elif path == 'document':
            commercial.create_document(book, 'Books', 'Book', _hooks(), document_type='invoice', label='Invoice', fiat_value=value)
        else:
            create_source(book, 'Books', 'Book', SimpleNamespace(resolve_scope=_hooks().resolve_scope), source_type='fiat_purchase', label='Source', fiat_value=value)
    assert error.value.code == 'validation'


@pytest.mark.parametrize('value', BAD[:5])
@pytest.mark.parametrize('field', ['amount', 'rate', 'invoice_amount'])
def test_btcpay_rejects_nonfinite_fiat(book, value, field):
    ws, profile = _hooks().resolve_scope(book)
    invoice = dict(store_id='store', invoice_id='invoice', currency='EUR', amount='10', created_at=NOW,
                   payments=[dict(payment_id='payment', payment_method_id='BTC-CHAIN', amount='0.01',
                                  invoice_currency='EUR', invoice_amount='10', rate='1000', received_at=NOW)])
    if field == 'amount':
        invoice[field] = value
    else:
        invoice['payments'][0][field] = value
    with pytest.raises(AppError) as error:
        commercial.upsert_btcpay_provenance(book, ws, profile, backend_name='btcpay', invoices=[invoice])
    assert error.value.code == 'validation'


@pytest.mark.parametrize('amount,value', [('0.00000000001', '1e308'), ('100000000', '1e-13')])
def test_import_validates_value_derived_rate(amount, value):
    with pytest.raises(AppError) as error:
        normalize_import_record(dict(amount=amount, occurred_at=NOW, fiat_value=value))
    assert error.value.code == 'validation'


def test_value_only_import_stores_no_inferred_rate_or_rescaled_total(book):
    from kassiber.core.imports import insert_wallet_records, ImportCoordinatorHooks, _transaction_merge_updates
    profile = book.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()
    wallet = book.execute("SELECT * FROM wallets WHERE id='btc'").fetchone()
    hooks = ImportCoordinatorHooks(lambda *_: None, lambda *_: None)
    insert_wallet_records(book, profile, wallet,
        [dict(txid='value-only', direction='inbound', asset='BTC', amount='3', fiat_value='100', occurred_at=NOW)],
        'custom', hooks)
    row = book.execute("SELECT * FROM transactions WHERE external_id='value-only'").fetchone()
    assert row['fiat_rate'] is None and row['fiat_rate_exact'] is None
    assert row['fiat_value_exact'] == '100'
    refreshed = normalize_import_record(dict(txid='value-only', direction='inbound', asset='BTC', amount='6', occurred_at=NOW))
    updates = _transaction_merge_updates(row, refreshed, 'refreshed', authoritative_chain_observer=True)
    assert updates['fiat_value'] is None and updates['fiat_value_exact'] is None


@pytest.mark.parametrize('value,stored', [('-0', '0'), ('0.0000001', '1E-7')])
def test_decimal_text_normalizes_exactly(book, value, stored):
    create_direct_swap_payout(book, 'Books', 'Book', 'out', payout_asset='EUR', payout_amount='1', payout_fiat_value=value)
    assert book.execute('SELECT payout_fiat_value_exact FROM custody_component_economic_terms').fetchone()[0] == stored


@pytest.mark.parametrize('surface', ['core', 'cli', 'daemon'])
def test_invalid_legacy_payout_list_and_delete(tmp_path, surface):
    conn = open_db(tmp_path)
    _scope(conn)
    _tx(conn, 'out', 'btc', 'outbound', 'BTC', 100000, NOW)
    conn.commit()
    payout = create_direct_swap_payout(conn, 'Books', 'Book', 'out', payout_asset='EUR', payout_amount='1', payout_fiat_value='10')
    payout_id = payout['id']
    conn.execute('DROP TRIGGER trg_custody_component_terms_immutable')
    conn.execute("UPDATE custody_component_economic_terms SET payout_fiat_value_exact='inf'")
    conn.commit()
    conn.close()
    conn = open_db(tmp_path)
    try:
        from kassiber.daemon import _ui_swap_matching_payload_from_conn
        scope = dict(workspace='Books', profile='Book')
        command = ['--machine', '--data-root', str(tmp_path),
                   'transfers', 'payouts']
        if surface == 'core':
            rows = list_payout_reviews(conn, 'profile')
        elif surface == 'daemon':
            rows = _ui_swap_matching_payload_from_conn(conn, 'ui.transfers.payouts.list', scope)['payouts']
        else:
            result = cli_runner.run(command + ['list', '--workspace', 'Books', '--profile', 'Book'], capture_output=True, text=True)
            assert result.returncode == 0, result.stdout + result.stderr
            rows = json.loads(result.stdout)['data']
        bad = next(row for row in rows if row['id'] == payout_id)
        assert bad['invalid'] and bad['payout_fiat_value'] is None
        if surface == 'core':
            deleted = delete_payout_review(conn, 'profile', payout_id)
        elif surface == 'daemon':
            deleted = _ui_swap_matching_payload_from_conn(conn, 'ui.transfers.payouts.delete', dict(**scope, payout_id=payout_id))
        else:
            result = cli_runner.run(command + ['delete', '--workspace', 'Books', '--profile', 'Book', '--payout-id', payout_id], capture_output=True, text=True)
            assert result.returncode == 0, result.stdout + result.stderr
            deleted = json.loads(result.stdout)['data']
        assert deleted['deleted_at']
        assert not list_payout_reviews(conn, 'profile')
    finally:
        conn.close()


@pytest.mark.parametrize('policy,asset', [('taxable', 'EUR'), ('carrying-value', 'LBTC')])
def test_split_payout_fails_closed(book, monkeypatch, policy, asset):
    create_direct_swap_payout(book, 'Books', 'Book', 'out', payout_asset=asset, payout_amount='0.0000004',
                             payout_fiat_value='10', out_amount='0.0000004', policy=policy)
    original = ctp.compile_finalized_tax_projection
    def split(profile, rows, state, **kwargs):
        decisions = []
        for decision in state.projection.decisions:
            if decision.reason == 'reviewed_direct_payout_source':
                source = decision.source
                middle = (source.start_msat + source.end_msat) // 2
                decisions.extend([replace(decision, source=QuantitySlice(source.observation_hash, source.start_msat, middle)),
                                  replace(decision, source=QuantitySlice(source.observation_hash, middle, source.end_msat))])
            else:
                decisions.append(decision)
        state = replace(state, projection=replace(state.projection, decisions=tuple(decisions)))
        if hasattr(state.tax_eligibility, 'eligible_decisions'):
            state = replace(state, tax_eligibility=replace(state.tax_eligibility, eligible_decisions=tuple(decisions)))
        return original(profile, rows, state, **kwargs)
    monkeypatch.setattr(ctp, 'compile_finalized_tax_projection', split)
    result = CustodyJournalBuilder(book, book.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()).build_custody_projection().finalized_tax_projection
    assert any(row['transaction_id'] == 'out' and row['reason'] == 'split_payout_requires_review' for row in result.quarantines)
    assert not result.rows


@pytest.mark.parametrize('rate', ['Infinity', '1e-13', '1e19', '1e40', '1e300'])
def test_legacy_price_quarantine_identifies_transaction(book, rate):
    book.execute("UPDATE transactions SET fiat_rate=?, fiat_rate_exact=? WHERE id='out'", (rate, rate))
    book.commit()
    result = process_journals(book, 'Books', 'Book')
    assert result['quarantined'] >= 1
    row = book.execute("SELECT * FROM journal_quarantines WHERE transaction_id='out'").fetchone()
    assert row['reason'] == 'invalid_transaction_price'
    assert 'transaction out:' in json.loads(row['detail_json'])['cause']
    assert not book.execute("SELECT 1 FROM journal_entries WHERE transaction_id='out'").fetchone()
    # Rebuilds still complete while the invalid assertion awaits correction.
    process_journals(book, 'Books', 'Book')


@pytest.mark.parametrize('field,value', [('fiat_rate', '1e-13'), ('fiat_value', '1e308')])
def test_metadata_rejects_unsupported_derived_price_without_writes(book, field, value):
    from kassiber.daemon import _transaction_metadata_update_payload
    from kassiber.db import set_setting
    set_setting(book, 'context_workspace', 'ws')
    set_setting(book, 'context_profile', 'profile')
    book.commit()
    before = book.total_changes
    with pytest.raises(AppError) as error:
        _transaction_metadata_update_payload(book, dict(transaction='out',
            pricing_source_kind='manual_override', pricing_quality='exact', **{field: value}), default_source='gui')
    assert error.value.code == 'validation'
    assert 'supported decimal range' in str(error.value) or 'finite decimal' in str(error.value)
    assert book.total_changes == before


def test_cached_rate_with_unsupported_derived_value_quarantines_only_its_transaction(book):
    set_manual_rate(book, 'BTC-EUR', NOW, '1e-13')
    process_journals(book, 'Books', 'Book')
    row = book.execute("SELECT * FROM transactions WHERE id='out'").fetchone()
    assert row['fiat_value_exact'] is None
    quarantine = book.execute("SELECT * FROM journal_quarantines WHERE transaction_id='out'").fetchone()
    assert quarantine['reason'] == 'invalid_transaction_price'
    assert 'cached rate' in json.loads(quarantine['detail_json'])['cause']
    set_manual_rate(book, 'BTC-EUR', NOW, '100000')
    process_journals(book, 'Books', 'Book')
    assert book.execute("SELECT fiat_value_exact FROM transactions WHERE id='out'").fetchone()[0] == '0.100000'
    assert not book.execute("SELECT 1 FROM journal_quarantines WHERE reason='invalid_transaction_price'").fetchone()


@pytest.mark.parametrize('kind,flag', [('transfer', 'authoritative_chain_observer'), ('routing_income', None)])
def test_amount_refresh_validates_recomputed_fiat_value(book, kind, flag):
    from kassiber.core.imports import _transaction_merge_updates
    book.execute("UPDATE transactions SET fiat_rate=1e-13, fiat_rate_exact='1e-13' WHERE id='out'")
    existing = book.execute("SELECT * FROM transactions WHERE id='out'").fetchone()
    normalized = normalize_import_record(dict(txid='out', occurred_at=NOW, direction='outbound',
                                              asset='BTC', amount='0.000002', kind=kind))
    with pytest.raises(AppError) as error:
        _transaction_merge_updates(existing, normalized, 'changed', **({flag: True} if flag else {}))
    assert error.value.code == 'validation'


def test_supplied_value_does_not_bypass_derived_value_bound():
    with pytest.raises(AppError) as error:
        normalize_import_record(dict(amount='0.000001', occurred_at=NOW, fiat_rate='1e-13', fiat_value='1'))
    assert error.value.code == 'validation'


@pytest.mark.parametrize('field,value', [('fiat_rate', '1e-13'), ('fiat_value', '1e308')])
def test_history_revert_rejects_unsupported_derived_price_without_writes(book, field, value):
    from kassiber.cli.handlers import _metadata_hooks
    book.execute(f"UPDATE transactions SET {field}=?, {field}_exact=? WHERE id='out'", (float(value), value))
    book.commit()
    corrected = metadata.update_transaction_metadata(book, 'Books', 'Book', 'out', _metadata_hooks(),
        pricing_update=dict(fiat_rate='100000', source_kind='manual_override', quality='exact'))
    before = book.total_changes
    with pytest.raises(AppError) as error:
        metadata.revert_transaction_edit(book, 'Books', 'Book', 'out', _metadata_hooks(),
            event_id=corrected['history_event_id'], field=field)
    assert error.value.code == 'validation'
    assert 'derived' in str(error.value) or field in str(error.value)
    assert book.total_changes == before
    assert book.execute("SELECT fiat_rate_exact, fiat_value_exact FROM transactions WHERE id='out'").fetchone()[:] == ('100000', '0.100000')


def test_component_terms_validate_derived_payout_rate():
    from kassiber.core.custody_components import normalize_economic_terms
    legs = [dict(id='source', ordinal=0, role='source', amount_msat=1),
            dict(id='target', ordinal=1, role='external_sink', amount_msat=1)]
    term = dict(source_leg_id='source', target_leg_id='target', term_kind='direct_swap_payout',
                source_row_hash='a'*64, payout_fiat_value_exact='1e308')
    with pytest.raises(AppError) as error:
        normalize_economic_terms([term], legs)
    assert error.value.code == 'validation'


def test_invalid_price_preserves_prior_entries_and_blocks_later_basis(book):
    _tx(book, 'prior', 'btc', 'inbound', 'BTC', 100000000, '2025-01-01T00:00:00Z')
    _tx(book, 'later', 'btc', 'outbound', 'BTC', 100000, '2027-01-01T00:00:00Z')
    book.execute("UPDATE transactions SET fiat_rate=100000, fiat_rate_exact='100000'")
    book.execute("UPDATE transactions SET fiat_rate=1e-13, fiat_rate_exact='1e-13' WHERE id='out'")
    book.commit()
    process_journals(book, 'Books', 'Book')
    assert book.execute("SELECT 1 FROM journal_entries WHERE transaction_id='prior'").fetchone()
    assert not book.execute("SELECT 1 FROM journal_entries WHERE transaction_id IN ('out','later')").fetchone()
    reasons = {row['transaction_id']: row['reason'] for row in book.execute('SELECT * FROM journal_quarantines')}
    assert reasons['out'] == 'invalid_transaction_price'
    assert reasons['later'] == 'custody_basis_barrier'


@pytest.mark.parametrize('export_format', ['csv', 'xlsx'])
def test_export_survives_legacy_nonfinite_price(book, tmp_path, export_format):
    book.execute("UPDATE transactions SET fiat_rate='inf', fiat_rate_exact='Infinity' WHERE id='out'")
    book.commit()
    output = tmp_path / ('transactions.' + export_format)
    # XLSX remains a Python reference boundary; CSV is a registered Rust slice.
    result = cli_runner.run(['--machine', '--data-root', str(tmp_path),
                             'transactions', 'export', '--export-format', export_format, '--workspace', 'Books', '--profile', 'Book',
                             '--file', str(output)], reference=export_format != 'csv', capture_output=True, text=True)
    assert result.returncode == 0, result.stdout + result.stderr
    assert output.stat().st_size > 0
    if export_format == 'csv':
        assert 'inf' not in output.read_text().lower()


@pytest.mark.parametrize('value', ['1e400', '1e-20', '1e-999999999', '1e19', '1e40', '1e300'])
def test_daemon_metadata_rejects_price_without_writes(book, value):
    from kassiber.daemon import _transaction_metadata_update_payload
    from kassiber.db import set_setting

    set_setting(book, 'context_workspace', 'ws')
    set_setting(book, 'context_profile', 'profile')
    book.commit()
    before = book.total_changes
    with pytest.raises(AppError) as error:
        _transaction_metadata_update_payload(book, dict(transaction='out', fiat_rate=value), default_source='user')
    assert error.value.code == 'validation'
    assert book.total_changes == before


@pytest.mark.parametrize('rate', ['1e18', '9999999999999997952'])
def test_largest_supported_prices_process_without_quantize_failure(book, rate):
    from kassiber.core.custody_pricing import finite_decimal
    assert finite_decimal(rate, 'rate') == Decimal(rate)
    book.execute("UPDATE transactions SET amount=50000000000, direction='inbound', fiat_rate=?, fiat_rate_exact=? WHERE id='out'", (float(rate), rate))
    book.commit()
    assert process_journals(book, 'Books', 'Book')['quarantined'] == 0


@pytest.mark.parametrize('rate', ['1e19', '1e40', '1e300'])
@pytest.mark.parametrize('legacy_source', ['transaction', 'rate_cache'])
def test_large_legacy_price_quarantines_sale_without_aborting_book(book, rate, legacy_source):
    _tx(book, 'buy', 'btc', 'inbound', 'BTC', 100000000000, '2025-01-01T00:00:00Z')
    book.execute("UPDATE transactions SET fiat_rate=50000, fiat_rate_exact='50000' WHERE id='buy'")
    book.execute("UPDATE transactions SET amount=50000000000 WHERE id='out'")
    if legacy_source == 'transaction':
        book.execute("UPDATE transactions SET fiat_rate=?, fiat_rate_exact=? WHERE id='out'", (float(rate), rate))
    else:
        upsert_rate(book, 'BTC-EUR', NOW, '50000', 'manual')
        book.execute('UPDATE rates_cache SET rate=?, rate_exact=?', (float(rate), rate))
    book.commit()
    process_journals(book, 'Books', 'Book')
    bad = book.execute("SELECT * FROM journal_quarantines WHERE transaction_id='out'").fetchone()
    assert bad['reason'] == 'invalid_transaction_price'
    assert 'supported decimal range' in json.loads(bad['detail_json'])['cause']
    assert book.execute("SELECT 1 FROM journal_entries WHERE transaction_id='buy'").fetchone()
    assert not book.execute("SELECT 1 FROM journal_entries WHERE transaction_id='out'").fetchone()


@pytest.mark.parametrize('bad_rate', [None, '1e-14', '1e40'])
def test_invalid_price_barrier_preserves_later_missing_price_cause(book, bad_rate):
    """price_q.py: an invalid sale must not hide the next sale's own cause."""
    _tx(book, 'buy', 'btc', 'inbound', 'BTC', 300000000000, '2025-01-01T00:00:00Z')
    _tx(book, 'sell-missing', 'btc', 'outbound', 'BTC', 50000000000, '2026-02-01T00:00:00Z')
    _tx(book, 'sell-ok', 'btc', 'outbound', 'BTC', 50000000000, '2026-03-01T00:00:00Z')
    _tx(book, 'receipt-missing', 'btc', 'inbound', 'BTC', 50000000000, '2026-04-01T00:00:00Z')
    book.execute("UPDATE transactions SET fiat_rate=50000, fiat_rate_exact='50000' WHERE id IN ('buy','sell-ok')")
    book.execute("UPDATE transactions SET amount=50000000000, fiat_rate=?, fiat_rate_exact=? WHERE id='out'", (float(bad_rate) if bad_rate else None, bad_rate))
    book.commit()
    process_journals(book, 'Books', 'Book')
    quarantines = {row['transaction_id']: row for row in book.execute('SELECT * FROM journal_quarantines')}
    assert quarantines['sell-missing']['reason'] == 'missing_spot_price'
    assert quarantines['receipt-missing']['reason'] == 'missing_spot_price'
    if bad_rate:
        assert quarantines['out']['reason'] == 'invalid_transaction_price'
        additional = json.loads(quarantines['sell-missing']['detail_json'])['additional_reasons']
        assert any(item['reason'] == 'custody_basis_barrier' for item in additional)
    assert {row[0] for row in book.execute('SELECT DISTINCT transaction_id FROM journal_entries')} == {'buy'}
