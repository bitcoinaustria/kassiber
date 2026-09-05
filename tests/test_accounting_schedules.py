import pytest

from kassiber.core.accounting import schedules
from kassiber.errors import AppError
from tests.test_accounting_evidence import accounting_db, post, retained


def metadata_item(conn, *, key='invoice', due='2026-03-31'):
    invoice = post(conn, 'ar', 'sales', 1000, key=key)
    return schedules.create_open_item(conn, 'p', direction='receivable', document_ref=key,
        origin_line_id=invoice['lines'][0]['id'], evidence_id=retained(conn), due_date=due)


def revision_args(item, **changes):
    return dict(item_id=item['id'], expected_revision=item['metadata_revision'], expected_digest=item['metadata_digest'],
        document_ref='corrected-invoice', due_date='2026-04-30', effective_date='2026-04-01',
        evidence_id=item['evidence_id'], reason='Reviewed payment terms and corrected document reference',
        idempotency_key='metadata-1', **changes)


def test_open_item_metadata_revision_preserves_financial_records_and_asof(accounting_db):
    from kassiber.core.accounting.supporting_commands import execute_supporting
    conn = accounting_db
    item = metadata_item(conn)
    payment = post(conn, 'bank', 'ar', 400, key='payment')
    schedules.allocate_settlement(conn, 'p', item_id=item['id'], settlement_line_id=payment['lines'][1]['id'], amount_minor=400, idempotency_key='payment')
    before = schedules.validate_close(conn, 'p', '2026-01-01', '2026-03-31')['open_items']
    financial = [tuple(row) for row in conn.execute('SELECT * FROM gl_lines ORDER BY id')]
    receipt = execute_supporting(conn, 'p', 'item-revise', revision_args(item))
    current = schedules.get_open_item(conn, 'p', item['id'])
    assert current['document_ref'] == 'corrected-invoice' and current['due_date'] == '2026-04-30'
    assert current['metadata_revision'] == 1 and current['metadata_digest'] == receipt['payload_digest']
    assert current['amount_minor'] == 1000 and current['remaining_minor'] == 600
    assert current['origin_line_id'] == item['origin_line_id']
    assert [tuple(row) for row in conn.execute('SELECT * FROM gl_lines ORDER BY id')] == financial
    assert conn.execute('SELECT document_ref,due_date FROM gl_open_items WHERE id=?', (item['id'],)).fetchone()['document_ref'] == 'invoice'
    assert schedules.validate_close(conn, 'p', '2026-01-01', '2026-03-31')['open_items'] == before
    assert schedules.validate_close(conn, 'p', '2026-01-01', '2026-04-30')['open_items'][0]['document_ref'] == 'corrected-invoice'
    assert execute_supporting(conn, 'p', 'item-revise', revision_args(item)) == receipt
    for statement in ('DELETE FROM gl_open_item_revisions', "UPDATE gl_open_item_revisions SET reason='changed'", 'INSERT OR REPLACE INTO gl_open_item_revisions SELECT * FROM gl_open_item_revisions'):
        with pytest.raises(Exception, match='retained|accounting_item_revision_integrity'):
            conn.execute(statement)


def test_open_item_metadata_stale_retry_scope_duplicate_and_void_guards(accounting_db):
    conn = accounting_db
    item = metadata_item(conn)
    args = revision_args(item)
    receipt = schedules.revise_open_item(conn, 'p', **args)
    with pytest.raises(AppError) as error:
        schedules.revise_open_item(conn, 'p', **{**args, 'reason':'Changed retry'})
    assert error.value.code == 'accounting_idempotency_conflict'
    with pytest.raises(AppError) as error:
        schedules.revise_open_item(conn, 'p', **{**args, 'idempotency_key':'stale'})
    assert error.value.code == 'accounting_revision_conflict'
    with pytest.raises(AppError):
        schedules.revise_open_item(conn, 'other', **args)
    current = schedules.get_open_item(conn, 'p', item['id'])
    latest = revision_args(current)
    for changes in ({'effective_date':'2026-03-31'}, {'evidence_id':retained(conn, 'other')}, {'expected_revision':True}):
        with pytest.raises(AppError):
            schedules.revise_open_item(conn, 'p', **{**latest, 'idempotency_key':'bad', **changes})
    other = metadata_item(conn, key='second')
    with pytest.raises(AppError) as error:
        schedules.revise_open_item(conn, 'p', **{**revision_args(other), 'idempotency_key':'duplicate'})
    assert error.value.code == 'accounting_open_item_duplicate'
    with pytest.raises(AppError):
        metadata_item(conn, key='corrected-invoice')
    schedules.void_open_item(conn, 'p', item_id=item['id'], reason='Reviewed cancellation', idempotency_key='void')
    with pytest.raises(AppError) as error:
        schedules.revise_open_item(conn, 'p', **{**latest, 'idempotency_key':'voided'})
    assert error.value.code == 'accounting_open_item_voided'
    assert schedules.revise_open_item(conn, 'p', **args) == receipt
    with pytest.raises(AppError):
        schedules.create_open_item(conn, 'p', direction='receivable', document_ref='new-record', origin_line_id=item['origin_line_id'], evidence_id=item['evidence_id'], due_date='2026-04-30')


@pytest.mark.parametrize('state', ['review', 'closed'])
def test_open_item_metadata_guards_all_impacted_closes_but_allows_later_correction(accounting_db, state):
    conn = accounting_db
    item = metadata_item(conn)
    before = schedules.validate_close(conn, 'p', '2026-01-01', '2026-12-31')['open_items']
    conn.execute('UPDATE gl_periods SET state=? WHERE profile_id=?', (state,'p'))
    with pytest.raises(AppError) as error:
        schedules.revise_open_item(conn, 'p', **revision_args(item))
    assert error.value.code == 'accounting_period_closed'
    receipt = schedules.revise_open_item(conn, 'p', **{**revision_args(item), 'effective_date':'2027-01-01'})
    assert receipt['revision'] == 1
    assert schedules.validate_close(conn, 'p', '2026-01-01', '2026-12-31')['open_items'] == before


def test_open_item_metadata_raw_chain_scope_and_read_integrity(accounting_db):
    conn = accounting_db
    item = metadata_item(conn)
    args = revision_args(item)
    # Even a same-book evidence reference cannot attach a revision to another book's item.
    with pytest.raises(Exception):
        conn.execute('''INSERT INTO gl_open_item_revisions
          (id,profile_id,item_id,revision,previous_digest,payload_digest,document_ref,due_date,effective_date,evidence_id,reason,idempotency_key)
          VALUES('raw','other',?,1,?,'bad','bad','2026-04-30','2026-04-01',?,'bad','raw')''',
          (item['id'],item['metadata_digest'],retained(conn,'other')))
    first = schedules.revise_open_item(conn, 'p', **args)
    current = schedules.get_open_item(conn, 'p', item['id'])
    second = schedules.revise_open_item(conn, 'p', **{**revision_args(current), 'idempotency_key':'second', 'effective_date':'2026-05-01'})
    assert second['previous_digest'] == first['payload_digest'] and second['revision'] == 2
    assert schedules.revise_open_item(conn, 'p', **args) == first


def test_receivable_partial_settlements_and_remainders(accounting_db):
    conn = accounting_db
    evidence = retained(conn)
    invoice = post(conn, "ar", "sales", 1000, key="invoice")
    item = schedules.create_open_item(conn, "p", direction="receivable", document_ref="invoice-1",
        origin_line_id=invoice["lines"][0]["id"], evidence_id=evidence, due_date="2026-03-31")
    assert item["account_code"] == "ar"
    payment = post(conn, "bank", "ar", 400, key="payment")
    args = dict(item_id=item["id"], settlement_line_id=payment["lines"][1]["id"], amount_minor=400, idempotency_key="payment")
    allocation = schedules.allocate_settlement(conn, "p", **args)
    assert allocation["remaining_minor"] == 600
    assert schedules.allocate_settlement(conn, "p", **args)["id"] == allocation["id"]
    assert schedules.list_open_items(conn, "p")[0]["remaining_minor"] == 600
    with pytest.raises(AppError):
        schedules.allocate_settlement(conn, "p", **{**args, "idempotency_key": "again"})
    with pytest.raises(AppError):
        schedules.allocate_settlement(conn, "p", **{**args, "amount_minor": 1})
    with pytest.raises(AppError):
        schedules.get_open_item(conn, "other", item["id"])


def test_payable_direction_and_origin_dedup(accounting_db):
    conn = accounting_db
    invoice = post(conn, "expense", "ap", key="bill")
    evidence = retained(conn)
    args = dict(direction="payable", document_ref="bill-1", origin_line_id=invoice["lines"][1]["id"],
                evidence_id=evidence, due_date="2026-03-31")
    item = schedules.create_open_item(conn, "p", **args)
    with pytest.raises(AppError):
        schedules.create_open_item(conn, "p", **args)
    with pytest.raises(AppError):
        schedules.create_open_item(conn, "p", **{**args, "direction": "receivable"})
    payment = post(conn, "ap", "bank", key="paid")
    schedules.allocate_settlement(conn, "p", item_id=item["id"], settlement_line_id=payment["lines"][0]["id"],
                                  amount_minor=100, idempotency_key="paid")
    assert schedules.get_open_item(conn, "p", item["id"])["remaining_minor"] == 0


def test_schedule_revisions_exact_scoped_retained(accounting_db):
    conn = accounting_db
    evidence = retained(conn)
    kwargs = dict(effective_date="2026-03-01", evidence_id=evidence,
                  fields={"carrying_value_minor": 2000, "method": "reviewed manual"}, reason="Initial record")
    first = schedules.create_schedule(conn, "p", kind="asset", label="Laptop", **kwargs)
    second = schedules.revise_schedule(conn, "p", schedule_id=first["schedule_id"], expected_revision=1,
        **{**kwargs, "fields": {"carrying_value_minor": 1500}, "reason": "Reviewed depreciation"})
    assert second["revision"] == 2
    assert schedules.list_schedules(conn, "p")[0]["fields"]["carrying_value_minor"] == 1500
    with pytest.raises(AppError, match="changed"):
        schedules.revise_schedule(conn, "p", schedule_id=first["schedule_id"], expected_revision=1, **kwargs)
    with pytest.raises(AppError):
        schedules.revise_schedule(conn, "other", schedule_id=first["schedule_id"], expected_revision=2, **kwargs)
    with pytest.raises(AppError):
        schedules.revise_schedule(conn, "p", schedule_id=first["schedule_id"], expected_revision=2,
                                  **{**kwargs, "fields": {"amount": 1.5}})
    with pytest.raises(Exception, match="retained"):
        conn.execute("DELETE FROM gl_schedule_revisions WHERE id=?", (first["id"],))
    assert conn.execute("SELECT COUNT(*) FROM gl_schedule_revisions").fetchone()[0] == 2


@pytest.mark.parametrize("invalid", [None, True, False, "", "123", "not money", 1.5, 2**63, -(2**63)])
def test_schedule_minor_fields_require_exact_bounded_integers(accounting_db, invalid):
    conn = accounting_db
    args = dict(effective_date="2026-03-01", evidence_id=retained(conn), reason="Reviewed record")
    with pytest.raises(AppError):
        schedules.create_schedule(conn, "p", kind="asset", label="Invalid",
                                  fields={"value_minor": invalid}, **args)
    assert conn.execute("SELECT COUNT(*) FROM gl_schedules").fetchone()[0] == 0
    first = schedules.create_schedule(conn, "p", kind="asset", label="Valid",
        fields={"value_minor": 0, "reviewed": False, "optional_note": None}, **args)
    with pytest.raises(AppError):
        schedules.revise_schedule(conn, "p", schedule_id=first["schedule_id"], expected_revision=1,
                                  fields={"value_minor": invalid}, **args)
    assert conn.execute("SELECT COUNT(*) FROM gl_schedule_revisions").fetchone()[0] == 1
    revised = schedules.revise_schedule(conn, "p", schedule_id=first["schedule_id"], expected_revision=1,
        fields={"value_minor": -(2**63 - 1), "maximum_minor": 2**63 - 1,
                "reviewed": True, "optional_note": None}, **args)
    assert revised["fields"]["value_minor"] == -(2**63 - 1)
    assert revised["fields"]["reviewed"] is True
    assert revised["fields"]["optional_note"] is None


def test_settlement_void_and_item_correction(accounting_db):
    conn = accounting_db
    evidence = retained(conn)
    invoice = post(conn, "ar", "sales", 100, key="invoice")
    item = schedules.create_open_item(conn, "p", direction="receivable", document_ref="invoice-1",
        origin_line_id=invoice["lines"][0]["id"], evidence_id=evidence, due_date="2026-03-31")
    payment = post(conn, "bank", "ar", 100, key="payment")
    allocation = schedules.allocate_settlement(conn, "p", item_id=item["id"],
        settlement_line_id=payment["lines"][1]["id"], amount_minor=100, idempotency_key="payment")
    with pytest.raises(AppError, match="Cancel active"):
        schedules.void_open_item(conn, "p", item_id=item["id"], reason="Wrong invoice", idempotency_key="void-item")
    schedules.void_settlement(conn, "p", allocation_id=allocation["id"], reason="Wrong match", idempotency_key="void-allocation")
    assert schedules.get_open_item(conn, "p", item["id"])["remaining_minor"] == 100
    schedules.void_open_item(conn, "p", item_id=item["id"], reason="Wrong invoice", idempotency_key="void-item")
    corrected = post(conn, "ar", "sales", 200, key="corrected")
    replacement = schedules.create_open_item(conn, "p", direction="receivable", document_ref="invoice-1",
        origin_line_id=corrected["lines"][0]["id"], evidence_id=evidence, due_date="2026-03-31")
    assert replacement["amount_minor"] == 200
    assert schedules.get_open_item(conn, "p", item["id"])["voided"]


def test_second_item_and_settlement_void_returns_typed_error(accounting_db):
    conn = accounting_db
    invoice = post(conn, 'ar', 'sales', 100, key='invoice')
    item = schedules.create_open_item(conn, 'p', direction='receivable', document_ref='invoice',
        origin_line_id=invoice['lines'][0]['id'], evidence_id=retained(conn), due_date='2026-03-31')
    payment = post(conn, 'bank', 'ar', 100, key='payment')
    settlement = schedules.allocate_settlement(conn, 'p', item_id=item['id'], settlement_line_id=payment['lines'][1]['id'], amount_minor=100, idempotency_key='allocate')
    args = dict(allocation_id=settlement['id'], reason='Reviewed wrong settlement', idempotency_key='void')
    schedules.void_settlement(conn, 'p', **args)
    assert schedules.void_settlement(conn, 'p', **args)['allocation_id'] == settlement['id']
    with pytest.raises(AppError) as error:
        schedules.void_settlement(conn, 'p', **{**args, 'idempotency_key':'new-key'})
    assert error.value.code == 'accounting_already_voided'
    args = dict(item_id=item['id'], reason='Reviewed wrong item', idempotency_key='void-item')
    schedules.void_open_item(conn, 'p', **args)
    assert schedules.void_open_item(conn, 'p', **args)['item_id'] == item['id']
    with pytest.raises(AppError) as error:
        schedules.void_open_item(conn, 'p', **{**args, 'idempotency_key':'new-item-key'})
    assert error.value.code == 'accounting_already_voided'


def test_gap_dated_schedule_change_cannot_modify_later_closed_asof(accounting_db):
    from kassiber.core.accounting import ledger
    conn = accounting_db
    evidence = retained(conn)
    args = dict(effective_date='2025-01-01', evidence_id=evidence, fields={'value_minor':100}, reason='Imported earlier schedule')
    first = schedules.create_schedule(conn, 'p', kind='asset', label='Historical asset', **args)
    ledger.close_period(conn, 'p', period_id='2026', expected_revision=ledger.require_book(conn, 'p')['revision'])
    before = schedules.validate_close(conn, 'p', '2026-01-01', '2026-12-31')
    with pytest.raises(AppError, match='Reopen'):
        schedules.revise_schedule(conn, 'p', schedule_id=first['schedule_id'], expected_revision=1,
            **{**args, 'effective_date':'2025-06-01', 'fields':{'value_minor':200}})
    with pytest.raises(AppError, match='Reopen'):
        schedules.create_schedule(conn, 'p', kind='asset', label='Late historical asset',
            **{**args, 'effective_date':'2025-07-01'})
    assert schedules.validate_close(conn, 'p', '2026-01-01', '2026-12-31') == before


def test_closed_period_blocks_support_changes_and_future_settlement_preserves_asof(accounting_db):
    conn = accounting_db
    evidence = retained(conn)
    invoice = post(conn, "ar", "sales", 100, key="invoice")
    item = schedules.create_open_item(conn, "p", direction="receivable", document_ref="invoice-1",
        origin_line_id=invoice["lines"][0]["id"], evidence_id=evidence, due_date="2026-03-31")
    payment = post(conn, "bank", "ar", 100, key="payment", day="2026-04-01")
    schedules.allocate_settlement(conn, "p", item_id=item["id"], settlement_line_id=payment["lines"][1]["id"],
                                  amount_minor=100, idempotency_key="payment")
    snapshot = schedules.validate_close(conn, "p", "2026-03-01", "2026-03-31")
    assert snapshot["open_items"][0]["remaining_minor"] == 100
    assert schedules.get_open_item(conn, "p", item["id"])["remaining_minor"] == 0
    conn.execute("UPDATE gl_periods SET state='closed' WHERE profile_id='p'")
    with pytest.raises(AppError, match="Reopen"):
        schedules.create_schedule(conn, "p", kind="asset", label="Late", effective_date="2026-03-01",
                                  evidence_id=evidence, fields={"amount_minor": 100}, reason="Late record")
    assert schedules.list_schedules(conn, "p") == []


def test_schedule_effective_date_precedes_revision_order(accounting_db):
    conn = accounting_db
    args = dict(evidence_id=retained(conn), reason="Reviewed")
    first = schedules.create_schedule(conn, "p", kind="asset", label="Asset", effective_date="2026-01-01",
                                      fields={"value_minor": 100}, **args)
    schedules.revise_schedule(conn, "p", schedule_id=first["schedule_id"], expected_revision=1,
                              effective_date="2026-07-01", fields={"value_minor": 200}, **args)
    schedules.revise_schedule(conn, "p", schedule_id=first["schedule_id"], expected_revision=2,
                              effective_date="2026-03-01", fields={"value_minor": 150}, **args)
    assert schedules.validate_close(conn, "p", "2026-01-01", "2026-12-31")["schedules"][0]["fields"]["value_minor"] == 200
    assert schedules.validate_close(conn, "p", "2026-01-01", "2026-06-30")["schedules"][0]["fields"]["value_minor"] == 150


def test_raw_cross_book_settlement_and_void_rejected(accounting_db):
    conn = accounting_db
    invoice = post(conn, "ar", "sales", 100, key="invoice")
    item = schedules.create_open_item(conn, "p", direction="receivable", document_ref="invoice-1",
        origin_line_id=invoice["lines"][0]["id"], evidence_id=retained(conn), due_date="2026-03-31")
    foreign = post(conn, "bank", "ar", 100, profile="other")
    with pytest.raises(Exception, match="scope"):
        conn.execute("INSERT INTO gl_open_item_allocations VALUES ('forged','p',?,?,100,'forged')",
                     (item["id"], foreign["lines"][1]["id"]))
    with pytest.raises(Exception, match="scope"):
        conn.execute("INSERT INTO gl_open_item_voids VALUES (?,'other','forged','forged')", (item["id"],))
