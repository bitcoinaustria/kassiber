"""Real custody-review / optional-ledger integration, without provider calls."""

from decimal import Decimal

import pytest

from kassiber.backup.age_cli import AgeBackend
from kassiber.backup.pack import export_backup, import_backup
from kassiber.cli.handlers import _metadata_hooks
from kassiber.core import review_workflow as review
from kassiber.core.accounting import artifacts, evidence, ledger, projection, sources
from kassiber.core.wallets import create_wallet
from kassiber.db import resolve_database_path
from kassiber.errors import AppError
from kassiber.secrets.sqlcipher import get_row_class, open_encrypted
from tests.test_accounting_integration import book  # noqa: F401
from tests.test_accounting_projection import prepared


def _custody_book(book):
    conn, scope, args = prepared(book)
    sources.void_binding(conn, scope, binding_id=args['binding_id'],
                         reason='Include both custody endpoints', idempotency_key='replace-source')
    original = conn.execute("SELECT * FROM transactions WHERE id='acquisition'").fetchone()
    target = create_wallet(conn, original['workspace_id'], scope, 'Reviewed destination', 'custom')
    for identity, day, direction, kind, wallet, price in (
        ('dispatch', '2025-02-01', 'outbound', 'sell', original['wallet_id'], '100'),
        ('receipt', '2025-03-01', 'inbound', 'buy', target['id'], '200'),
    ):
        conn.execute('''INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,
            fingerprint,occurred_at,direction,asset,amount,fee,fiat_currency,fiat_rate_exact,
            raw_json,created_at,kind) VALUES(?,?,?,?,?,?,?,?,'BTC',50000000000,0,'EUR',?,'{}',?,?)''',
            (identity, original['workspace_id'], scope, wallet, identity, 'fp-' + identity,
             day + 'T12:00:00Z', direction, price, day + 'T12:00:00Z', kind))
    snapshot = sources.capture_sources(conn, scope)
    calculation = artifacts.capture_calculation(conn, scope, snapshot_id=snapshot['id'], period_id='2025')
    assert calculation['capture']['blockers'] == []
    event, mapping = next((event, mapping)
        for event, mapping in calculation['capture']['inputs']['source_event_map'].items()
        if mapping['journal_transaction_id'] == 'acquisition')
    binding = sources.bind_sources(conn, scope, snapshot_id=snapshot['id'],
        expected_digest=snapshot['input_digest'], economic_id='reviewed-acquisition', role='recognition',
        reason='All endpoints included', idempotency_key='source-with-endpoints',
        claims=[dict(source_id=mapping['source_id'], **claim) for claim in mapping['claim_slices']])
    proposal = projection.create_proposal(conn, scope, **{
        **args, 'artifact_id': calculation['id'], 'binding_id': binding['id'], 'event_id': event})
    conn.commit()
    return conn, scope, snapshot, calculation, proposal


def _plan_bridge(conn, scope):
    profile = conn.execute('SELECT * FROM profiles WHERE id=?', (scope,)).fetchone()
    hooks = review.ReviewHooks(metadata=_metadata_hooks())
    plan = review.plan_review(conn, profile, expected_input_version=profile['journal_input_version'],
        hooks=hooks, operations=[{'type': 'custody_component', 'request': {
            'action': 'create', 'activate': True, 'components': [{
                'component_type': 'manual_bridge', 'evidence_kind': 'manual_claim',
                'evidence_grade': 'reviewed', 'change_reason': 'Both owned endpoints reviewed',
                'legs': [
                    {'role': 'source', 'transaction': 'dispatch', 'amount_msat': 50000000000},
                    {'role': 'destination', 'transaction': 'receipt', 'amount_msat': 50000000000},
                ],
            }],
        }}])
    return profile, hooks, plan


def test_custody_review_invalidates_retained_calculation_and_unposted_projection(book):
    conn, scope, snapshot, calculation, proposal = _custody_book(book)
    profile, hooks, plan = _plan_bridge(conn, scope)
    # Preview is read-only: it cannot expire a user's existing approval.
    assert sources.require_current(conn, scope, snapshot['id'])['id'] == snapshot['id']
    assert artifacts.require_calculation_current(conn, scope, calculation['id'])['id'] == calculation['id']
    receipt = review.apply_review(conn, profile, artifact=plan, idempotency_key='bridge', hooks=hooks)
    assert receipt['status'] == 'verified'
    assert receipt['verification']['report_ready']
    assert receipt['verification']['journal_digest'] != plan['before']['journal_digest']
    for stale_read in (
        lambda: sources.require_current(conn, scope, snapshot['id']),
        lambda: artifacts.require_calculation_current(conn, scope, calculation['id']),
        lambda: projection.post_proposal(conn, scope, proposal_id=proposal['id'],
                                         expected_digest=proposal['payload_digest']),
    ):
        with pytest.raises(AppError) as exc:
            stale_read()
        assert exc.value.code == 'accounting_source_stale'
    assert ledger._entry(conn, scope, proposal['draft_id'])['status'] == 'draft'
    assert conn.execute('SELECT COUNT(*) FROM gl_projection_publications').fetchone()[0] == 0
    # History remains readable; a fresh calculation reflects carried basis.
    assert artifacts.get_calculation(conn, scope, calculation['id']) == calculation
    current = sources.capture_sources(conn, scope)
    refreshed = artifacts.capture_calculation(conn, scope, snapshot_id=current['id'], period_id='2025')
    assert current['input_digest'] != snapshot['input_digest']
    assert not refreshed['capture']['blockers']
    asset = refreshed['capture']['assets'][0]
    assert asset['gain_losses'] == []
    assert sum(Decimal(item['basis_exact']) for item in asset['open_positions']) == 100


def test_retried_custody_review_never_reposts_existing_general_ledger_entry(book):
    conn, scope, _, _, proposal = _custody_book(book)
    projection.post_proposal(conn, scope, proposal_id=proposal['id'], expected_digest=proposal['payload_digest'])
    conn.commit()
    original_entry = ledger._entry(conn, scope, proposal['draft_id'])
    original_revision = ledger.require_book(conn, scope)['revision']
    profile, hooks, plan = _plan_bridge(conn, scope)
    receipt = review.apply_review(conn, profile, artifact=plan, idempotency_key='bridge-retry', hooks=hooks)
    assert review.apply_review(conn, profile, artifact=plan, idempotency_key='bridge-retry', hooks=hooks) == receipt
    assert ledger._entry(conn, scope, proposal['draft_id']) == original_entry
    assert ledger.require_book(conn, scope)['revision'] == original_revision
    assert conn.execute('SELECT COUNT(*) FROM gl_entries').fetchone()[0] == 1
    assert conn.execute('SELECT COUNT(*) FROM gl_projection_publications').fetchone()[0] == 1
    assert conn.execute('SELECT COUNT(*) FROM review_workflow_receipts').fetchone()[0] == 1
    assert conn.execute("SELECT COUNT(*) FROM custody_components WHERE state='active'").fetchone()[0] == 1
    assert projection.validate_close(conn, scope, '2025-01-01', '2025-12-31')['blockers']


@pytest.mark.parametrize('journal_mode', ['delete', 'wal'])
def test_review_receipt_and_encrypted_ledger_survive_backup_in_both_journal_modes(book, tmp_path, journal_mode):
    pytest.importorskip('pyrage')
    conn, scope, _, _, proposal = _custody_book(book)
    # Explicit modes exercise backup compatibility, not the runtime-selection policy.
    assert conn.execute(f'PRAGMA journal_mode={journal_mode}').fetchone()[0] == journal_mode
    projection.post_proposal(conn, scope, proposal_id=proposal['id'], expected_digest=proposal['payload_digest'])
    retained = evidence.retain_evidence(conn, scope, content=b'Synthetic encrypted review evidence',
                                       name='Review evidence', media_type='text/plain')
    conn.commit()
    profile, hooks, plan = _plan_bridge(conn, scope)
    receipt = review.apply_review(conn, profile, artifact=plan, idempotency_key='restore-retry', hooks=hooks)
    archive = tmp_path / 'combined.kassiber'
    backend = AgeBackend('pyrage')
    export_backup(str(book[2]), archive, 'test-token-placeholder',
                  backup_passphrase='synthetic-archive-key', age_backend=backend)
    assert b'Synthetic encrypted review evidence' not in archive.read_bytes()
    installed = import_backup(archive, tmp_path / 'restored' / 'data',
        backup_passphrase='synthetic-archive-key', age_backend=backend, move_into_place=True)
    restored = open_encrypted(resolve_database_path(installed.installed_data_root),
                             'test-token-placeholder', row_factory=get_row_class())
    try:
        assert evidence.read_evidence_bytes(restored, scope, retained['id']) == b'Synthetic encrypted review evidence'
        assert review.get_receipt(restored, profile, receipt_id=receipt['id']) == receipt
        assert review.apply_review(restored, profile, artifact=plan, idempotency_key='restore-retry', hooks=hooks) == receipt
        assert projection.get_proposal(restored, scope, proposal['id'])['published']
        assert restored.execute('SELECT COUNT(*) FROM gl_entries').fetchone()[0] == 1
        assert restored.execute('SELECT COUNT(*) FROM review_workflow_receipts').fetchone()[0] == 1
    finally:
        restored.close()
