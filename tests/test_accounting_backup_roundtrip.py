"""Actual tar/age export and installed restore of a multi-year accounting book."""
import pytest

from kassiber.backup.age_cli import AgeBackend
from kassiber.backup.pack import export_backup, import_backup
from kassiber.core.accounting import artifacts, document_text, evidence, ledger, projection, schedules
from kassiber.core.accounting.package import export_close, verify_package
from kassiber.db import resolve_database_path
from kassiber.errors import AppError
from kassiber.secrets.sqlcipher import get_row_class, open_encrypted
from tests.test_accounting_integration import book  # noqa: F401
from tests.test_accounting_projection import prepared


def test_full_age_archive_recovers_evidence_open_items_artifacts_and_two_closes(book, tmp_path):
    pytest.importorskip('pyrage')
    conn, profile, root = book
    ledger.create_account(conn, profile, code='ar', name='Receivables', kind='asset')
    retained = evidence.retain_evidence(conn, profile, content=b'Synthetic retained invoice 42 EUR 5.00',
        name='Invoice 42', media_type='text/plain')
    extracted = document_text.extract(conn, profile, evidence_id=retained['id'])
    draft = ledger.create_draft(conn, profile, dict(period_id='2025',entry_date='2025-01-02',
        idempotency_key='invoice',description='Synthetic invoice42',lines=[
            dict(account_code='ar',debit_minor=500),dict(account_code='sales',credit_minor=500)]))
    posted = ledger.post_draft(conn, profile, draft_id=draft['id'],expected_digest=draft['payload_digest'])
    item = schedules.create_open_item(conn, profile, direction='receivable',document_ref='Invoice42',
        origin_line_id=posted['lines'][0]['id'],evidence_id=retained['id'],due_date='2025-01-31')
    _, _, args = prepared(book)
    proposal = projection.create_proposal(conn, profile, **args)
    projection.post_proposal(conn,profile,proposal_id=proposal['id'],expected_digest=proposal['payload_digest'])
    close_2025 = ledger.close_period(conn,profile,period_id='2025',expected_revision=ledger.require_book(conn,profile)['revision'])
    ledger.create_period(conn,profile,period_id='2026',start_date='2026-01-01',end_date='2026-12-31')
    first_artifact = artifacts.get_calculation(conn,profile,args['artifact_id'])
    artifact_2026 = artifacts.capture_calculation(conn,profile,snapshot_id=first_artifact['source_snapshot_id'],period_id='2026')
    close_2026 = ledger.close_period(conn,profile,period_id='2026',expected_revision=ledger.require_book(conn,profile)['revision'])
    conn.commit()
    archive = tmp_path / 'complete.kassiber'
    backend = AgeBackend('pyrage')
    exported = export_backup(str(root),archive,'test-token-placeholder',backup_passphrase='synthetic-outer-archive-key',age_backend=backend)
    assert exported.output_path == archive
    assert b'Synthetic retained invoice' not in archive.read_bytes()
    installed = import_backup(archive,tmp_path/'restored'/'data',backup_passphrase='synthetic-outer-archive-key',
        age_backend=backend,move_into_place=True)
    assert installed.temporary_artifacts_cleaned
    restored = open_encrypted(resolve_database_path(installed.installed_data_root),'test-token-placeholder',row_factory=get_row_class())
    try:
        assert evidence.read_evidence_bytes(restored,profile,retained['id']) == b'Synthetic retained invoice 42 EUR 5.00'
        assert document_text.get(restored,profile,extraction_id=extracted['id'])['content_digest'] == extracted['content_digest']
        assert schedules.get_open_item(restored,profile,item['id'])['remaining_minor'] == 500
        assert artifacts.get_calculation(restored,profile,artifact_2026['id'])['payload_digest'] == artifact_2026['payload_digest']
        assert projection.get_proposal(restored,profile,proposal['id'])['published']
        for original in (close_2025,close_2026):
            package = export_close(restored,profile,close_id=original['id'])
            assert package['snapshot_digest'] == original['snapshot_digest']
            assert verify_package(package)['ledger_arithmetic'] == 'verified'
        ledger.reopen_period(restored,profile,period_id='2025',reason='Restored book correction',expected_revision=ledger.require_book(restored,profile)['revision'])
        assert ledger._period(restored,profile,'2026')['state'] == 'review'
        assert export_close(restored,profile,close_id=close_2026['id'])['snapshot_digest'] == close_2026['snapshot_digest']
    finally:
        restored.close()
    with pytest.raises(AppError):
        import_backup(archive,tmp_path/'wrong-key'/'data',backup_passphrase='wrong synthetic key',age_backend=backend,move_into_place=True)
    assert not (tmp_path/'wrong-key'/'data').exists()
