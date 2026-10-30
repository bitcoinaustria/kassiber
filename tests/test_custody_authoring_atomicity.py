"""Authoring must remain rollbackable after nested custody helpers return."""
import sqlite3

import pytest

from kassiber.cli import handlers
from kassiber.core import custody_component_planner as planner
from kassiber.db import open_db
from tests.test_custody_authored_migration import NOW, _scope, _tx


@pytest.fixture
def book(tmp_path):
    conn = open_db(tmp_path)
    _scope(conn)
    _tx(conn, 'out', 'btc', 'outbound', 'BTC', 100000, NOW)
    _tx(conn, 'in', 'btc2', 'inbound', 'BTC', 100000, NOW)
    _mark_current(conn)
    conn.commit()
    yield conn
    conn.close()


def _mark_current(conn):
    conn.execute(
        """UPDATE profiles SET last_processed_at=?, last_processed_tx_count=2,
           last_processed_input_version=journal_input_version WHERE id='profile'""",
        (NOW,),
    )


def _snapshot(conn):
    # Include audit, memberships, input versions and retained journal rows.
    return {row[0]: conn.execute(f'SELECT * FROM "{row[0]}" ORDER BY 1').fetchall()
            for row in conn.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")}


def _refuse_version(conn, offset):
    version = conn.execute("SELECT journal_input_version FROM profiles WHERE id='profile'").fetchone()[0]
    conn.execute(f"""CREATE TEMP TRIGGER refuse_invalidation
        BEFORE UPDATE OF journal_input_version ON profiles
        WHEN NEW.id='profile' AND NEW.journal_input_version={version + offset}
        BEGIN SELECT RAISE(ABORT, 'late invalidation'); END""")


def _payout(conn):
    return handlers.create_direct_swap_payout(conn, 'ws', 'profile', 'out',
        payout_asset='EUR', payout_amount='1', policy='taxable', payout_fiat_value='10')


def _pair(conn):
    return handlers.create_transaction_pair(conn, 'ws', 'profile', 'out', 'in')


@pytest.mark.parametrize('operation', ['payout_create', 'payout_delete', 'pair_create', 'pair_update', 'pair_delete'])
def test_late_review_invalidation_rolls_back_every_table(book, operation):
    conn = book
    if operation == 'payout_delete':
        review = _payout(conn)
        apply = lambda: handlers.delete_direct_swap_payout(conn, 'ws', 'profile', review['id'])
    elif operation in {'pair_update', 'pair_delete'}:
        review = _pair(conn)
        if operation == 'pair_update':
            apply = lambda: handlers.update_transaction_pair(conn, 'ws', 'profile', review['id'], notes='revised')
        else:
            apply = lambda: handlers.delete_transaction_pair(conn, 'ws', 'profile', review['id'])
    else:
        apply = lambda: (_payout(conn) if operation == 'payout_create' else _pair(conn))
    _mark_current(conn)
    conn.commit()
    before = _snapshot(conn)
    # Activation/supersession invalidates once; the review facade invalidates last.
    _refuse_version(conn, 2)
    with pytest.raises(sqlite3.IntegrityError, match='late invalidation'):
        apply()
    conn.rollback()
    assert _snapshot(conn) == before


def _spec():
    return dict(component_type='manual_bridge', evidence_kind='manual_claim',
                legs=[dict(role='source', transaction='out', amount_msat=100000),
                      dict(role='destination', transaction='in', amount_msat=100000)])


@pytest.mark.parametrize('action', ['create', 'revise', 'undo', 'activate', 'supersede'])
def test_component_apply_late_failure(book, action):
    conn = book
    args = dict(workspace_id='ws', profile_id='profile', specs=[_spec()],
                activate=action != 'activate', authored_source='cli')
    if action != 'create':
        plan = planner.plan_component_batch(conn, **args)
        component = planner.apply_component_batch(conn, **args, expected_input_version=plan['input_version'])['components'][0]
    if action == 'undo':
        state_args = dict(workspace_id='ws', profile_id='profile', action='supersede', component_id=component['id'])
        state_plan = planner.plan_component_state_change(conn, **state_args)
        planner.apply_component_state_change(conn, **state_args, expected_input_version=state_plan['input_version'])
    before = _snapshot(conn)
    if action == 'create':
        plan = planner.plan_component_batch(conn, **args)
        apply = lambda: planner.apply_component_batch(conn, **args, expected_input_version=plan['input_version'])
    elif action in {'revise', 'undo'}:
        args = dict(workspace_id='ws', profile_id='profile', action=action,
                    component_id=component['id'], spec={**_spec(), 'notes': 'revision'}, activate=True)
        plan = planner.plan_component_revision(conn, **args)
        apply = lambda: planner.apply_component_revision(conn, **args, expected_input_version=plan['input_version'])
    else:
        args = dict(workspace_id='ws', profile_id='profile', action=action, component_id=component['id'])
        plan = planner.plan_component_state_change(conn, **args)
        apply = lambda: planner.apply_component_state_change(conn, **args, expected_input_version=plan['input_version'])
    assert _snapshot(conn) == before  # planning is read-only
    _refuse_version(conn, 1)
    with pytest.raises(sqlite3.IntegrityError, match='late invalidation'):
        apply()
    conn.rollback()
    assert _snapshot(conn) == before


@pytest.mark.parametrize('activate', [False, True])
def test_component_commit_false_leaves_success_rollbackable(book, activate):
    args = dict(workspace_id='ws', profile_id='profile', specs=[_spec()],
                activate=activate, authored_source='cli')
    before = _snapshot(book)
    plan = planner.plan_component_batch(book, **args)
    planner.apply_component_batch(book, **args, expected_input_version=plan['input_version'], commit=False)
    assert book.in_transaction
    book.rollback()
    assert _snapshot(book) == before


@pytest.mark.parametrize('operation', ['dismiss', 'rule_create', 'rule_delete', 'rule_disable', 'rule_enable'])
def test_single_statement_authoring_failure(book, operation):
    conn = book
    if operation.startswith('rule_') and operation != 'rule_create':
        rule = handlers.create_transfer_rule(
            conn, 'ws', 'profile', enabled=operation != 'rule_enable'
        )
    before = _snapshot(conn)
    table = 'transaction_pair_dismissals' if operation == 'dismiss' else 'swap_matching_rules'
    event = {'rule_delete': 'DELETE', 'rule_disable': 'UPDATE', 'rule_enable': 'UPDATE'}.get(operation, 'INSERT')
    conn.execute(f"CREATE TEMP TRIGGER refuse_authoring AFTER {event} ON {table} BEGIN SELECT RAISE(ABORT, 'late authoring'); END")
    with pytest.raises(sqlite3.IntegrityError, match='late authoring'):
        if operation == 'dismiss':
            handlers.dismiss_transfer_candidate(conn, 'ws', 'profile', 'out', 'in')
        elif operation == 'rule_create':
            handlers.create_transfer_rule(conn, 'ws', 'profile')
        elif operation == 'rule_delete':
            handlers.delete_transfer_rule(conn, 'ws', 'profile', rule['id'])
        else:
            handlers.set_transfer_rule_enabled(conn, 'ws', 'profile', rule['id'], operation == 'rule_enable')
    conn.rollback()
    assert _snapshot(conn) == before


@pytest.fixture
def gap_book():
    from tests.test_custody_gap_lifecycle import CustodyGapLifecycleTests
    fixture = CustodyGapLifecycleTests()
    fixture.setUp()
    yield fixture
    fixture.tearDown()


@pytest.mark.parametrize('action', ['create', 'dismiss', 'reopen', 'revise', 'classify_residual'])
def test_gap_apply_last_step_failure(gap_book, action, monkeypatch):
    from kassiber.core import custody_gap_reviews as gaps
    from tests.test_custody_gap_lifecycle import _apply_review
    fixture = gap_book
    conn = fixture.conn
    candidate = fixture._candidate()
    if action in {'reopen', 'revise', 'classify_residual'}:
        _, created = fixture._create_bridge()
        if action == 'revise':
            _apply_review(conn, workspace_id='ws', profile_id='profile', gap_id=created['gap_id'], action='reopen')
            candidate = fixture._candidate()
    args = dict(workspace_id='ws', profile_id='profile', action=action)
    if action in {'create', 'dismiss', 'revise'}:
        args['candidate'] = candidate
    else:
        args['gap_id'] = created['gap_id']
    if action == 'classify_residual':
        args['classification'] = 'external_payment'
    before = _snapshot(conn)
    plan = gaps.plan_review(conn, **args)
    assert _snapshot(conn) == before
    # For bridges, impact recording is after activation, audit and invalidation.
    # Dismissals finish with invalidation and have no impact append.
    if action == 'dismiss':
        _refuse_version(conn, 1)
    else:
        def fail(*args, **kwargs):
            raise sqlite3.IntegrityError('late invalidation')
        if action in {'create', 'revise'}:
            monkeypatch.setattr(gaps.custody_filed_reports, 'append_custody_impacts', fail)
        else:
            monkeypatch.setattr(gaps, '_append_snapshot_impacts', fail)
    with pytest.raises(sqlite3.IntegrityError, match='late invalidation'):
        gaps.apply_review(conn, **args, expected_input_version=plan['input_version'])
    conn.rollback()
    assert _snapshot(conn) == before


def test_gap_commit_false_leaves_success_rollbackable(gap_book):
    from kassiber.core import custody_gap_reviews as gaps
    conn = gap_book.conn
    args = dict(workspace_id='ws', profile_id='profile', action='create', candidate=gap_book._candidate())
    before = _snapshot(conn)
    plan = gaps.plan_review(conn, **args)
    gaps.apply_review(conn, **args, expected_input_version=plan['input_version'], commit=False)
    assert conn.in_transaction
    conn.rollback()
    assert _snapshot(conn) == before


@pytest.mark.parametrize('operation', ['bulk', 'rules'])
def test_batch_pairing_late_invalidation_rolls_back(book, operation):
    conn = book
    handlers.create_transfer_rule(conn, 'ws', 'profile')
    before = _snapshot(conn)
    _refuse_version(conn, 2)
    with pytest.raises(sqlite3.IntegrityError, match='late invalidation'):
        if operation == 'bulk':
            handlers.bulk_pair_transfers(conn, 'ws', 'profile', confidence='strong')
        else:
            handlers.apply_transfer_rules(conn, 'ws', 'profile')
    conn.rollback()
    assert _snapshot(conn) == before


def test_payout_component_revision_late_failure(book):
    _payout(book)
    component_id = book.execute("SELECT id FROM custody_components WHERE state='active'").fetchone()[0]
    args = dict(workspace_id='ws', profile_id='profile', action='revise',
                component_id=component_id, spec={'notes': 'revised payout'}, activate=True)
    before = _snapshot(book)
    plan = planner.plan_component_revision(book, **args)
    _refuse_version(book, 1)
    with pytest.raises(sqlite3.IntegrityError, match='late invalidation'):
        planner.apply_component_revision(book, **args, expected_input_version=plan['input_version'])
    book.rollback()
    assert _snapshot(book) == before


@pytest.mark.parametrize('operation', ['pair', 'payout', 'component'])
def test_successful_authoring_has_one_explicit_commit(book, operation):
    statements = []
    book.set_trace_callback(statements.append)
    try:
        if operation == 'pair':
            _pair(book)
        elif operation == 'payout':
            _payout(book)
        else:
            args = dict(workspace_id='ws', profile_id='profile', specs=[_spec()],
                        activate=True, authored_source='cli')
            plan = planner.plan_component_batch(book, **args)
            planner.apply_component_batch(book, **args, expected_input_version=plan['input_version'])
    finally:
        book.set_trace_callback(None)
    assert sum(statement.strip().upper() == 'COMMIT' for statement in statements) == 1
    assert not book.in_transaction
    assert book.execute("SELECT COUNT(*) FROM custody_components WHERE state='active'").fetchone()[0] == 1
