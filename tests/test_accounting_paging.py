"""No silent 500-record ceiling and no cross-book/stale continuation."""
from datetime import date, timedelta

import pytest

from kassiber.core.accounting import bank, evidence, schedules
from kassiber.core.accounting.commands import execute
from kassiber.core.accounting.paging import records_page
from kassiber.errors import AppError
from tests.test_accounting_evidence import accounting_db, post


def test_all_supporting_collections_reach_record_501(accounting_db):
    conn = accounting_db
    expected = {"evidence": set(), "statements": set(), "items": set(), "schedules": set()}
    for index in range(501):
        document = evidence.retain_evidence(conn, "p", content=f"Synthetic {index}".encode(),
            name=f"Document {index}", media_type="text/plain")
        expected["evidence"].add(document["id"])
        invoice = post(conn, "ar", "sales", key=f"invoice-{index}")
        item = schedules.create_open_item(conn, "p", direction="receivable", document_ref=f"Invoice {index}",
            origin_line_id=invoice["lines"][0]["id"], evidence_id=document["id"], due_date="2026-03-31")
        expected["items"].add(item["id"])
        schedule = schedules.create_schedule(conn, "p", kind="asset", label=f"Asset {index}",
            effective_date="2026-03-01", evidence_id=document["id"], fields={"value_minor": 100}, reason="Synthetic")
        expected["schedules"].add(schedule["schedule_id"])
        day = (date(2024, 1, 1) + timedelta(days=index)).isoformat()
        statement = bank.import_statement(conn, "p", account_code="bank", statement_id=f"Statement {index}",
            csv_text="row_id,date,amount_minor,description\n", start_date=day, end_date=day)
        expected["statements"].add(statement["id"])
    actions = {"evidence": "evidence-list", "statements": "bank-list", "items": "item-list", "schedules": "schedule-list"}
    for key, action in actions.items():
        first = execute(conn, "p", action, {"limit": 500})
        assert len(first[key]) == 500
        assert first["next_cursor"] and first["total_count"] == 501
        second = execute(conn, "p", action, {"limit": 500, "cursor": first["next_cursor"]})
        assert len(second[key]) == 1
        assert second["next_cursor"] is None
        assert first["binding"] == second["binding"]
        rows = first[key] + second[key]
        assert {row["id"] for row in rows} == expected[key]
        assert len({row["id"] for row in rows}) == 501
        if key == "statements":
            dates = [row["end_date"] for row in rows]
            assert dates == sorted(dates, reverse=True)
        with pytest.raises(AppError) as exc:
            execute(conn, "other", action, {"cursor": first["next_cursor"]})
        assert exc.value.code == "accounting_invalid_cursor"
        wrong_action = "evidence-list" if action != "evidence-list" else "bank-list"
        with pytest.raises(AppError) as exc:
            execute(conn, "p", wrong_action, {"cursor": first["next_cursor"]})
        assert exc.value.code == "accounting_invalid_cursor"


def test_evidence_cursor_rejects_insertion_drift_without_book_revision_bump(accounting_db):
    conn = accounting_db
    for index in range(2):
        evidence.retain_evidence(conn, "p", content=b"Synthetic", name=str(index), media_type="text/plain")
    first = evidence.evidence_page(conn, "p", limit=1)
    evidence.retain_evidence(conn, "p", content=b"New", name="New", media_type="text/plain")
    with pytest.raises(AppError) as exc:
        evidence.evidence_page(conn, "p", limit=1, cursor=first["next_cursor"])
    assert exc.value.code == "accounting_stale_cursor"


def test_mutation_during_page_materialization_fails_closed(accounting_db):
    conn = accounting_db
    evidence.retain_evidence(conn, "p", content=b"Synthetic", name="Original", media_type="text/plain")
    def materialize(identifier):
        evidence.retain_evidence(conn, "p", content=b"New", name="Inserted during read", media_type="text/plain")
        return evidence.require_evidence(conn, "p", identifier)
    with pytest.raises(AppError) as exc:
        records_page(conn, "p", "evidence", materialize=materialize)
    assert exc.value.code == "accounting_stale_cursor"


@pytest.mark.parametrize("limit", [0, 501, -1, True, "100"])
def test_supporting_page_limits_remain_bounded(accounting_db, limit):
    with pytest.raises(AppError):
        evidence.evidence_page(accounting_db, "p", limit=limit)


@pytest.mark.parametrize("cursor", ["", "not-base64", "e30=", 1, "x" * 2049])
def test_malformed_cursor_is_structured_error(accounting_db, cursor):
    with pytest.raises(AppError) as exc:
        evidence.evidence_page(accounting_db, "p", cursor=cursor)
    assert exc.value.code == "accounting_invalid_cursor"
