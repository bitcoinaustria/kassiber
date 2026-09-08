"""A recovered native route preserves reviewed provenance without duplicate MOVEs."""

from dataclasses import replace

import pytest

from kassiber.cli.handlers import process_journals
from kassiber.core import custody_components, custody_gaps, custody_journal, source_funds
from kassiber.core.custody_reconciliation_store import load_current_reconciliations, replace_reconciliations
from kassiber.core.sync_replication.schema_allowlist import NEVER_SYNC_TABLES, SYNC_TABLE_MAP
from tests.test_custody_lineage_flagship import _FlagshipTreasury, _Transaction, _review_candidate, BTC
from tests.test_source_funds_custody_projection import _hooks


@pytest.fixture
def recovered_book(tmp_path):
    book = _FlagshipTreasury(tmp_path)
    try:
        book.insert([
            _Transaction('fund-a', 'a', 'inbound', BTC, '2020-01-01T00:00:00Z', kind='buy'),
            _Transaction('out-a', 'a', 'outbound', BTC, '2021-01-01T00:00:00Z', txid=book._txid(1), privacy_boundary='coinjoin'),
            _Transaction('in-c', 'c', 'inbound', BTC, '2022-01-01T00:00:00Z', txid=book._txid(2), privacy_boundary='coinjoin'),
        ])
        candidate = custody_gaps.load_gap_search_result(book.conn, 'profile')[0].candidates[0]
        reviewed = _review_candidate(book.conn, candidate)
        process_journals(book.conn, 'ws', 'profile')
        source_funds.assemble_history(book.conn, None, None, _hooks(), target_transaction_ref='in-c')
        source = source_funds.create_source(
            book.conn, None, None, _hooks(), source_type='fiat_purchase', label='Purchase',
            amount='1', acquired_at='2019-01-01T00:00:00Z',
        )
        source_funds.create_link(book.conn, None, None, _hooks(), from_source_ref=source['id'], to_transaction_ref='fund-a', allocation_amount='1')
        source_funds.create_link(book.conn, None, None, _hooks(), from_transaction_ref='fund-a', to_transaction_ref='out-a', allocation_amount='1', from_allocation_amount='1')
        before = report(book)
        assert before['explain_gates']['exportable'], before['findings']
        book.insert([
            _Transaction('in-b', 'b', 'inbound', BTC, '2021-01-01T00:00:00Z', txid=book._txid(1), privacy_boundary='coinjoin'),
            _Transaction('out-b', 'b', 'outbound', BTC, '2022-01-01T00:00:00Z', txid=book._txid(2), privacy_boundary='coinjoin'),
        ])
        process_journals(book.conn, 'ws', 'profile')
        yield book, reviewed['component_id']
    finally:
        book.close()


def report(book):
    return source_funds.build_report(book.conn, None, None, _hooks(), target_transaction_ref='in-c')


def test_recovered_native_history_preserves_reviewed_report(recovered_book):
    book, component_id = recovered_book
    assert load_current_reconciliations(book.conn, 'profile')[0]['component_id'] == component_id
    after = report(book)
    assert after['explain_gates']['exportable'], after['findings']
    assert after['source_mix'][0]['amount'] == 1
    assert book.conn.execute('SELECT COUNT(*) FROM journal_custody_projection_relations').fetchone()[0] == 2
    # Re-assembly must not add the native last-hop as a second competing source
    # for the user's already reviewed compressed disclosure.
    source_funds.assemble_history(book.conn, None, None, _hooks(), target_transaction_ref='in-c')
    assert report(book)['explain_gates']['exportable']
    links = book.conn.execute("SELECT from_transaction_id FROM source_funds_links WHERE to_transaction_id='in-c' AND state='reviewed'").fetchall()
    assert [row[0] for row in links] == ['out-a']
    assert 'journal_custody_reconciliations' in NEVER_SYNC_TABLES
    assert 'journal_custody_reconciliations' not in SYNC_TABLE_MAP


@pytest.mark.parametrize('change', ['history_retracted', 'component_retired', 'book_stale'])
def test_reconciliation_proof_never_outlives_its_inputs(recovered_book, change):
    book, component_id = recovered_book
    if change == 'history_retracted':
        book.conn.execute("UPDATE transactions SET excluded=1 WHERE id='in-b'")
        process_journals(book.conn, 'ws', 'profile')
    elif change == 'component_retired':
        custody_components.supersede_component(book.conn, component_id)
        process_journals(book.conn, 'ws', 'profile')
    else:
        book.conn.execute("UPDATE profiles SET journal_input_version=journal_input_version+1 WHERE id='profile'")
    assert not load_current_reconciliations(book.conn, 'profile')
    if change != 'history_retracted':
        assert 'stale_custody_component_lineage' in {item['code'] for item in report(book)['findings']}
    else:
        # The still-authored shortcut resumes its original accounting role.
        assert not book.conn.execute('SELECT 1 FROM journal_custody_reconciliations').fetchone()
        assert report(book)['explain_gates']['exportable']


@pytest.mark.parametrize('selection', ['absent', 'partial'])
def test_certificate_requires_complete_selected_native_claims(recovered_book, selection):
    book, _ = recovered_book
    profile = book.conn.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()
    state = custody_journal.CustodyJournalBuilder(book.conn, profile).build()
    quantity = state['custody_quantity']
    decisions = list(quantity.projection.decisions)
    index = next(i for i, decision in enumerate(decisions) if decision.selected_claim_id in state['custody_native_reconciliations'][0]['native_claim_ids'])
    decision = decisions[index]
    decisions[index] = replace(decision, selected_claim_id=None) if selection == 'absent' else replace(
        decision, source=replace(decision.source, end_msat=decision.source.end_msat - 1),
    )
    rejected_quantity = replace(quantity, projection=replace(quantity.projection, decisions=tuple(decisions)))
    replace_reconciliations(book.conn, profile, rejected_quantity, state['custody_native_reconciliations'], created_at='2026-01-01T00:00:00Z')
    assert not load_current_reconciliations(book.conn, 'profile')
