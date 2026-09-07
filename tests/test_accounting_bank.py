import io
import json

import pytest

from kassiber.core.accounting import bank, evidence
from kassiber.errors import AppError
from tests.test_accounting_evidence import accounting_db, post, retained


CSV = "row_id,date,amount_minor,description\na,2026-03-01,100,Donation\nb,2026-03-01,100,Donation\n"


def imported(conn, **overrides):
    args = dict(account_code="bank", statement_id="statement-1", csv_text=CSV,
                start_date="2026-03-01", end_date="2026-03-31")
    args.update(overrides)
    return bank.import_statement(conn, "p", **args)


def control_document(conn, *, profile='p', opening=0, closing=200, pdf=False):
    if pdf:
        from reportlab.pdfgen.canvas import Canvas
        output = io.BytesIO()
        canvas = Canvas(output)
        canvas.drawString(72, 760, f'Synthetic statement-1, March 2026. Opening {opening}; closing {closing} minor EUR units.')
        canvas.save()
        content, media = output.getvalue(), 'application/pdf'
    else:
        content = json.dumps(dict(format='kassiber-bank-control-v1', account_code='bank', statement_id='statement-1',
            start_date='2026-03-01', end_date='2026-03-31', currency='EUR', minor_unit_exponent=2,
            opening_minor=opening, closing_minor=closing)).encode()
        media = 'application/json'
    return evidence.retain_evidence(conn, profile, content=content, media_type=media, name='Synthetic control')['id']


def test_bank_identity_controls_and_full_reconciliation(accounting_db):
    conn = accounting_db
    kwargs = dict(opening_minor=0, closing_minor=200, evidence_id=retained(conn),
                  control_evidence_id=control_document(conn), control_review_reason='Reviewed exact statement balances',
                  control_locator='Opening and closing fields')
    statement = imported(conn, **kwargs)
    assert len(statement["rows"]) == 2
    assert imported(conn, **kwargs)["already_imported"]
    entry = post(conn, "bank", "sales", 200)
    rows = bank.reconcile_statement(conn, "p", statement["id"])["rows"]
    for row in rows:
        bank.allocate_bank_row(conn, "p", row_id=row["id"], line_id=entry["lines"][0]["id"],
                               amount_minor=100, idempotency_key=row["id"])
    assert bank.reconcile_statement(conn, "p", statement["id"])["reconciled"]
    with pytest.raises(AppError):
        bank.reconcile_statement(conn, "other", statement["id"])
    with pytest.raises(AppError, match="identity"):
        imported(conn, closing_minor=300)
    with pytest.raises(AppError, match="overlaps"):
        imported(conn, statement_id="overlapping")


def test_bank_missing_controls_never_complete(accounting_db):
    statement = imported(accounting_db)
    result = bank.reconcile_statement(accounting_db, "p", statement["id"])
    assert not result["reconciled"]
    assert "missing_control_balances" in result["blockers"]
    assert "missing_control_evidence" in result["blockers"]


def test_row_only_csv_cannot_prove_manually_entered_balances(accounting_db):
    conn = accounting_db
    csv_id = evidence.retain_evidence(conn, 'p', content=CSV.encode(), media_type='text/csv', name='Rows')['id']
    statement = imported(conn, evidence_id=csv_id, opening_minor=0, closing_minor=200)
    entry = post(conn, 'bank', 'sales', 200)
    for row in bank.reconcile_statement(conn, 'p', statement['id'])['rows']:
        bank.allocate_bank_row(conn, 'p', row_id=row['id'], line_id=entry['lines'][0]['id'], amount_minor=100, idempotency_key=row['id'])
    report = bank.reconcile_statement(conn, 'p', statement['id'])
    assert report['arithmetic_reconciled']
    assert not report['reconciled'] and not report['control_evidence_reviewed']
    assert report['blockers'] == ['missing_control_evidence']
    from kassiber.core.accounting.schedules import validate_close
    assert validate_close(conn, 'p', '2026-01-01', '2026-12-31')['blockers']
    bank.ensure_schema(conn)
    assert imported(conn, evidence_id=csv_id, opening_minor=0, closing_minor=200)['already_imported']
    with pytest.raises(AppError) as error:
        imported(conn, evidence_id=csv_id, opening_minor=0, closing_minor=200,
            control_evidence_id=csv_id, control_review_reason='Reviewed', control_locator='Rows')
    assert error.value.code == 'accounting_bank_control_evidence'


def test_retained_csv_and_separate_reviewed_pdf_end_to_end(accounting_db):
    from kassiber.core.accounting.supporting_commands import execute_supporting
    conn = accounting_db
    csv_id = evidence.retain_evidence(conn, 'p', content=CSV.encode(), media_type='text/csv', name='Rows')['id']
    pdf_id = control_document(conn, pdf=True)
    payload = dict(csv_evidence_id=csv_id, account_code='bank', statement_id='statement-1',
        start_date='2026-03-01', end_date='2026-03-31', opening_minor=0, closing_minor=200,
        control_evidence_id=pdf_id, control_review_reason='Checked account, dates and both balances', control_locator='Page 1, statement heading and opening/closing labels')
    statement = execute_supporting(conn, 'p', 'bank-import', payload)
    entry = post(conn, 'bank', 'sales', 200)
    for row in bank.reconcile_statement(conn, 'p', statement['id'])['rows']:
        bank.allocate_bank_row(conn, 'p', row_id=row['id'], line_id=entry['lines'][0]['id'], amount_minor=100, idempotency_key=row['id'])
    report = bank.reconcile_statement(conn, 'p', statement['id'])
    assert report['arithmetic_reconciled'] and report['reconciled'] and report['control_evidence_reviewed']
    assert report['statement']['evidence_id'] == csv_id
    assert report['statement']['control_evidence_id'] == pdf_id
    assert report['control_provenance']['method'] == 'human_reviewed_pdf_locator'
    assert report['control_provenance']['content_sha256'] == evidence.require_evidence(conn, 'p', pdf_id)['content_sha256']
    with pytest.raises(AppError):
        execute_supporting(conn, 'p', 'bank-import', {**payload, 'evidence_id':pdf_id})
    with pytest.raises(Exception, match='retained'):
        conn.execute('UPDATE gl_bank_statements SET control_locator=? WHERE id=?', ('New page', statement['id']))


@pytest.mark.parametrize('changes', [dict(control_review_reason=''), dict(control_locator=''),
    dict(opening_minor=None), dict(closing_minor=None)])
def test_bank_control_review_requires_complete_review(accounting_db, changes):
    payload = dict(opening_minor=0, closing_minor=200, control_evidence_id=control_document(accounting_db),
        control_review_reason='Checked balances', control_locator='Balance fields')
    with pytest.raises(AppError):
        imported(accounting_db, **{**payload, **changes})


@pytest.mark.parametrize('closing', [201, True, 200.0, '200'])
def test_canonical_control_rejects_wrong_or_inexact_values(accounting_db, closing):
    with pytest.raises(AppError) as error:
        imported(accounting_db, opening_minor=0, closing_minor=200,
            control_evidence_id=control_document(accounting_db, closing=closing),
            control_review_reason='Checked balances', control_locator='Balance fields')
    assert error.value.code == 'accounting_bank_control_evidence'


def test_bank_control_scope_rejected_in_api_and_raw_storage(accounting_db):
    conn = accounting_db
    control = control_document(conn, profile='other')
    with pytest.raises(AppError):
        imported(conn, opening_minor=0, closing_minor=200, control_evidence_id=control,
            control_review_reason='Checked balances', control_locator='Balance fields')
    with pytest.raises(Exception, match='accounting_bank_control_scope'):
        conn.execute('''INSERT INTO gl_bank_statements
            (id,profile_id,account_code,statement_id,adapter_version,start_date,end_date,
             opening_minor,closing_minor,payload_digest,control_evidence_id,control_review_reason,control_locator)
            VALUES('raw','p','bank','raw','v1','2026-03-01','2026-03-31',0,200,'digest',?,'Reviewed','Page 1')''', (control,))


def test_canonical_control_rejects_duplicate_fields(accounting_db):
    conn = accounting_db
    original = control_document(conn)
    content = evidence.read_evidence_bytes(conn, 'p', original)
    duplicate = b'{"closing_minor":999,' + content[1:]
    identifier = evidence.retain_evidence(conn, 'p', content=duplicate, media_type='application/json', name='Ambiguous controls')['id']
    with pytest.raises(AppError) as error:
        imported(conn, opening_minor=0, closing_minor=200, control_evidence_id=identifier,
                 control_review_reason='Checked balances', control_locator='Balance fields')
    assert error.value.code == 'accounting_bank_control_evidence'


def test_bank_negative_rows_and_invalid_controls(accounting_db):
    negative = CSV.replace(",100,", ",-100,")
    assert imported(accounting_db, csv_text=negative, opening_minor=200, closing_minor=0)["movement_minor"] == -200
    with pytest.raises(AppError, match="differs"):
        imported(accounting_db, statement_id="invalid", start_date="2026-04-01", end_date="2026-04-30",
                 csv_text=CSV.replace("2026-03", "2026-04"), opening_minor=0, closing_minor=999)


@pytest.mark.parametrize("csv_text", [CSV.replace("b,", "a,"), CSV.replace(",100,", ",1.00,"),
                                      CSV.replace("2026-03-01", "2026-3-1"), CSV.replace("row_id", "id")])
def test_bank_parser_rejects_ambiguous_input(csv_text):
    with pytest.raises(AppError):
        bank.preview_statement(csv_text, start_date="2026-03-01", end_date="2026-03-31")


def test_bank_allocation_no_cross_book_wrong_direction_or_overuse(accounting_db):
    conn = accounting_db
    statement = imported(conn)
    rows = bank.reconcile_statement(conn, "p", statement["id"])["rows"]
    wrong = post(conn, "expense", "bank", key="wrong")
    other = post(conn, "bank", "sales", key="other", profile="other")
    for line_id in (wrong["lines"][1]["id"], other["lines"][0]["id"]):
        with pytest.raises(AppError):
            bank.allocate_bank_row(conn, "p", row_id=rows[0]["id"], line_id=line_id, amount_minor=100, idempotency_key=line_id)
    entry = post(conn, "bank", "sales", 100)
    args = dict(row_id=rows[0]["id"], line_id=entry["lines"][0]["id"], amount_minor=100, idempotency_key="once")
    first = bank.allocate_bank_row(conn, "p", **args)
    assert bank.allocate_bank_row(conn, "p", **args)["id"] == first["id"]
    with pytest.raises(AppError):
        bank.allocate_bank_row(conn, "p", **{**args, "row_id": rows[1]["id"], "idempotency_key": "twice"})


def test_bank_correction_is_append_only_and_releases_capacity(accounting_db):
    conn = accounting_db
    statement = imported(conn)
    row = bank.reconcile_statement(conn, "p", statement["id"])["rows"][0]
    entry = post(conn, "bank", "sales", 100)
    allocation = bank.allocate_bank_row(conn, "p", row_id=row["id"], line_id=entry["lines"][0]["id"],
                                         amount_minor=100, idempotency_key="allocate")
    with pytest.raises(AppError, match="Cancel active"):
        bank.void_statement(conn, "p", statement_id=statement["id"], reason="Wrong import", idempotency_key="void-statement")
    for _ in range(2):
        bank.void_bank_allocation(conn, "p", allocation_id=allocation["id"], reason="Wrong match", idempotency_key="void-match")
    with pytest.raises(AppError):
        bank.void_bank_allocation(conn, "other", allocation_id=allocation["id"], reason="Wrong match", idempotency_key="void-match")
    assert bank.reconcile_statement(conn, "p", statement["id"])["rows"][0]["remaining_minor"] == 100
    bank.void_statement(conn, "p", statement_id=statement["id"], reason="Wrong import", idempotency_key="void-statement")
    assert "statement_voided" in bank.reconcile_statement(conn, "p", statement["id"])["blockers"]
    replacement = imported(conn, statement_id="corrected")
    assert replacement["id"] != statement["id"]
    assert conn.execute("SELECT COUNT(*) FROM gl_bank_rows").fetchone()[0] == 4
    with pytest.raises(Exception, match="retained"):
        conn.execute("INSERT OR REPLACE INTO gl_bank_statements SELECT * FROM gl_bank_statements WHERE id=?", (statement["id"],))


def test_second_bank_void_identity_returns_typed_error(accounting_db):
    conn = accounting_db
    statement = imported(conn)
    entry = post(conn, 'bank', 'sales', 100)
    row = bank.reconcile_statement(conn, 'p', statement['id'])['rows'][0]
    allocation = bank.allocate_bank_row(conn, 'p', row_id=row['id'],
        line_id=entry['lines'][0]['id'], amount_minor=100, idempotency_key='allocate')
    args = dict(allocation_id=allocation['id'], reason='Reviewed wrong match', idempotency_key='void')
    bank.void_bank_allocation(conn, 'p', **args)
    assert bank.void_bank_allocation(conn, 'p', **args)['allocation_id'] == allocation['id']
    with pytest.raises(AppError) as error:
        bank.void_bank_allocation(conn, 'p', **{**args, 'idempotency_key':'new-key'})
    assert error.value.code == 'accounting_already_voided'
    args = dict(statement_id=statement['id'], reason='Reviewed wrong statement', idempotency_key='void-statement')
    bank.void_statement(conn, 'p', **args)
    assert bank.void_statement(conn, 'p', **args)['statement_id'] == statement['id']
    with pytest.raises(AppError) as error:
        bank.void_statement(conn, 'p', **{**args, 'idempotency_key':'new-statement-key'})
    assert error.value.code == 'accounting_already_voided'


def test_bank_allocation_guards_statement_interval_not_only_later_line(accounting_db):
    conn = accounting_db
    statement = imported(conn, csv_text=CSV.replace('2026-', '2025-'),
                         start_date='2025-03-01', end_date='2025-03-31')
    row = bank.reconcile_statement(conn, 'p', statement['id'])['rows'][0]
    entry = post(conn, 'bank', 'sales', 100, day='2026-04-01')
    # Defensive invariant test: normal close also rejects an unallocated
    # statement. Simulate a retained non-open period independently of that guard.
    conn.execute("INSERT INTO gl_periods VALUES('p','march','2025-03-01','2025-03-31','closed',1)")
    with pytest.raises(AppError, match='Reopen'):
        bank.allocate_bank_row(conn, 'p', row_id=row['id'],
            line_id=entry['lines'][0]['id'], amount_minor=100, idempotency_key='late')
    assert conn.execute('SELECT COUNT(*) FROM gl_bank_allocations').fetchone()[0] == 0


def test_bank_raw_scope_amount_and_source_guard(accounting_db):
    conn = accounting_db
    evidence = retained(conn)
    statement = imported(conn, evidence_id=evidence)
    row = bank.reconcile_statement(conn, "p", statement["id"])["rows"][0]
    other = post(conn, "bank", "sales", 100, profile="other")
    for amount in (100, 1.5):
        with pytest.raises(Exception):
            conn.execute("INSERT INTO gl_bank_allocations VALUES ('forged','p',?,?,?,'forged')",
                         (row["id"], other["lines"][0]["id"], amount))
    with pytest.raises(Exception, match="scope"):
        conn.execute("INSERT INTO gl_bank_statement_voids VALUES (?,'other','forged','forged')", (statement["id"],))
    from kassiber.core.accounting import ledger
    ledger.create_account(conn, "p", code="bank2", name="Second bank", kind="asset")
    with pytest.raises(AppError, match="already assigned"):
        imported(conn, account_code="bank2", evidence_id=evidence)
    # Simulate a legacy database predating the insertion scope trigger.
    conn.execute("DROP TRIGGER gl_bank_allocation_scope")
    conn.execute("INSERT INTO gl_bank_allocations VALUES ('legacy','p',?,?,100,'legacy')", (row["id"], other["lines"][0]["id"]))
    assert "invalid_allocation_integrity" in bank.reconcile_statement(conn, "p", statement["id"])["blockers"]


def test_void_allocation_then_reverse_requires_new_posting(accounting_db):
    from kassiber.core.accounting import ledger
    conn = accounting_db
    statement = imported(conn)
    row = bank.reconcile_statement(conn, "p", statement["id"])["rows"][0]
    entry = post(conn, "bank", "sales", 100)
    allocation = bank.allocate_bank_row(conn, "p", row_id=row["id"], line_id=entry["lines"][0]["id"],
                                         amount_minor=100, idempotency_key="allocate")
    args = dict(entry_id=entry["id"], entry_date="2026-03-02", period_id="2026", idempotency_key="reverse", reason="Correction")
    with pytest.raises(AppError):
        ledger.reverse_entry(conn, "p", **args)
    bank.void_bank_allocation(conn, "p", allocation_id=allocation["id"], reason="Correction", idempotency_key="void")
    reversed_entry = ledger.reverse_entry(conn, "p", **args)
    if reversed_entry["status"] == "draft":
        ledger.post_draft(conn, "p", draft_id=reversed_entry["id"], expected_digest=reversed_entry["payload_digest"])
    with pytest.raises(AppError):
        bank.allocate_bank_row(conn, "p", row_id=row["id"], line_id=entry["lines"][0]["id"],
                               amount_minor=100, idempotency_key="again")
