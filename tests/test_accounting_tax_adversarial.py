"""Independent adversarial checks at the actual accounting command boundary."""

import hashlib
import json

import pytest

from kassiber.core.accounting import ledger, tax_workpapers as tax
from kassiber.core.accounting.commands import execute
from kassiber.errors import AppError
from tests.test_accounting_evidence import accounting_db
from tests.test_accounting_tax_workpapers import complete, tax_db


def test_tax_command_json_export_preserves_verifiable_report_digest(tax_db):
    paper = complete(tax_db)
    preview = tax.preview_workpaper(tax_db, 'p', workpaper_id=paper['id'])
    final = tax.finalize_workpaper(
        tax_db, 'p', workpaper_id=paper['id'], expected_revision=2,
        expected_digest=preview['input_digest'],
    )
    exported = json.loads(json.dumps(execute(tax_db, 'p', 'tax-export', {
        'final_id': final['final_id'], 'confirm_plaintext': True,
    })))
    original = tax_db.execute('SELECT report_json FROM gl_tax_finals WHERE id=?', (final['final_id'],)).fetchone()[0]
    assert exported['report_json'] == original
    assert exported['report_digest'] == hashlib.sha256(exported['report_json'].encode()).hexdigest()
    assert exported['report_digest'] == ledger.digest(json.loads(exported['report_json']))
    assert type(json.loads(exported['report_json'])['book_profit_minor']) is int
    assert exported['report']['book_profit_minor'] == '0'


@pytest.mark.parametrize('review', [
    {'state': 'reviewed_input', 'value_minor': None, 'reason': 'Unknown is not zero'},
    {'state': 'not_applicable', 'value_minor': 0, 'reason': 'Absence is not a submitted zero'},
    {'state': 'blocked', 'value_minor': 0, 'reason': 'Blocked cannot carry a value'},
])
def test_null_and_inconsistent_source_states_cannot_be_retained(tax_db, review):
    paper = complete(tax_db)
    with pytest.raises(AppError) as raised:
        execute(tax_db, 'p', 'tax-review', {
            'workpaper_id': paper['id'], 'expected_revision': 2,
            'patch': {'field_reviews': {'main.168': review}},
            'reason': 'Adversarial unknown source', 'idempotency_key': 'unknown',
        })
    assert raised.value.code == 'accounting_tax_validation'
    assert tax.get_workpaper(tax_db, 'p', workpaper_id=paper['id'])['revision'] == 2


def test_absent_operand_cannot_finalize_via_reviewed_not_applicable_total(tax_db):
    paper = complete(tax_db)
    tax.review_workpaper(tax_db, 'p', workpaper_id=paper['id'], expected_revision=2,
        patch={'field_reviews': {'main.168': None}},
        reason='Remove an operand while its former aggregate remains reviewed absent',
        idempotency_key='missing-operand')
    preview = tax.preview_workpaper(tax_db, 'p', workpaper_id=paper['id'])
    assert not preview['ready']
    assert any(item['target'] == 'main.168' for item in preview['blockers'])
    with pytest.raises(AppError) as raised:
        tax.finalize_workpaper(tax_db, 'p', workpaper_id=paper['id'], expected_revision=3,
            expected_digest=preview['input_digest'])
    assert raised.value.code == 'accounting_tax_blocked'
