"""Old-writer CSV rows survive economic-ID upgrades without rewriting reviews.

The compact row fixture was written with 1ec7e7da0's actual importer parsers
and the unchanged insertion/batch writer, for two identical rows per export.
It retains the exact old IDs, raw JSON, and reviewed excluded gap evidence.
"""

import json
from pathlib import Path

import pytest

from kassiber import importers
from kassiber.cli import handlers
from kassiber.core import custody_journal, import_batches, metadata, quarantine_review, review_workflow
from kassiber.core.imports import insert_wallet_records, import_bullbitcoin_records_full
from kassiber.db import open_db
from kassiber.errors import AppError
from tests.test_custody_authored_migration import _scope
from tests.test_import_csv_identities import CASES, _write


ISSUE = "synthetic_import_duplicate_review_required"
FIXTURE = Path(__file__).parent / "fixtures/historical/pre_economic_csv_ids_1ec7e7da.json"


def _insert(conn, table, row):
    columns = ', '.join(row)
    conn.execute(f"INSERT INTO {table} ({columns}) VALUES ({', '.join('?' for _ in row)})", tuple(row.values()))


def _restore(tmp_path, case_index):
    conn = open_db(tmp_path / "book")
    _scope(conn)
    case = json.loads(FIXTURE.read_text())["cases"][case_index]
    for table in ("transactions", "import_batches", "import_batch_transactions"):
        for row in case[table]:
            _insert(conn, table, row)
    conn.commit()
    profile = conn.execute("SELECT * FROM profiles").fetchone()
    wallet = conn.execute("SELECT * FROM wallets WHERE id='btc'").fetchone()
    return conn, profile, wallet


def _import(conn, profile, wallet, path, fmt, rows):
    _write(path, rows)
    records = importers.load_import_records(path, fmt)
    if fmt in {"coinfinity_csv", "pocketbitcoin_csv"}:
        result = import_bullbitcoin_records_full(conn, profile, wallet, records, "file:" + fmt,
            handlers._import_coordinator_hooks(), input_format=fmt)
    else:
        result = insert_wallet_records(conn, profile, wallet, records, "file:" + fmt,
                                        handlers._import_coordinator_hooks())
    import_batches.record_batch(conn, profile, wallet_id=wallet["id"], source_format=fmt,
        source_filename=path.name, column_map=None, outcome=result)
    conn.commit()
    return result


@pytest.mark.parametrize("case_index", range(len(CASES)))
def test_old_writer_rows_are_reused_without_rewriting_and_overlap_preserves_real_repeats(tmp_path, case_index):
    conn, profile, wallet = _restore(tmp_path, case_index)
    fmt, january, date_column = CASES[case_index]
    path = tmp_path / "export.csv"
    try:
        before = [tuple(row) for row in conn.execute("SELECT * FROM transactions ORDER BY id")]
        result = _import(conn, profile, wallet, path, fmt, [january, january])
        assert result["imported"] == 0
        assert [tuple(row) for row in conn.execute("SELECT * FROM transactions ORDER BY id")] == before
        february = {**january, date_column: january[date_column].replace("01-01", "02-01")}
        assert _import(conn, profile, wallet, path, fmt, [february, january])["imported"] == 1
        assert _import(conn, profile, wallet, path, fmt, [january] * 3)["imported"] == 1
        assert _import(conn, profile, wallet, path, fmt, [january] * 3)["imported"] == 0
        stored = conn.execute("SELECT amount FROM transactions").fetchall()
        assert len(stored) == 4
        assert sum(row["amount"] for row in stored) == 800_000_000
        assert not any(q["reason"] == ISSUE for q in review_workflow._build(conn, profile)["quarantines"])
    finally:
        conn.close()


@pytest.mark.parametrize("case_index", range(len(CASES)))
@pytest.mark.parametrize("exclude", ["legacy", "current"])
def test_cross_batch_excess_blocks_reports_until_owner_excludes_a_row(tmp_path, case_index, exclude):
    conn, profile, wallet = _restore(tmp_path, case_index)
    fmt, january, _ = CASES[case_index]
    try:
        # Simulate 8b's first re-import: one new economic-ID copy of an old row.
        path = tmp_path / "export.csv"
        _write(path, [january])
        current, = importers.load_import_records(path, fmt)
        copied = dict(conn.execute("SELECT * FROM transactions ORDER BY id LIMIT 1").fetchone())
        copied.update(id="post-upgrade", external_id=current["txid"], fingerprint="post-upgrade",
                      created_at="2025-04-01T00:00:00Z")
        _insert(conn, "transactions", copied)
        batch = import_batches.record_batch(conn, profile, wallet_id="btc", source_format=fmt,
            source_filename="expanded.csv", column_map=None,
            outcome={"inserted_records": [{"transaction_id": "post-upgrade"}]})
        conn.execute("UPDATE import_batches SET imported_at='2025-04-01T00:00:00Z' WHERE id=?", (batch,))
        conn.execute("UPDATE transactions SET excluded=0, fiat_rate=50000, fiat_rate_exact='50000'")
        conn.commit()
        before = [tuple(row) for row in conn.execute("SELECT * FROM transactions ORDER BY id")]
        state = review_workflow._build(conn, profile)
        held = [q for q in state["quarantines"] if q["reason"] == ISSUE]
        assert len(held) == 3
        assert state["custody_quantity"].report_blocked
        assert not {q["transaction_id"] for q in held} & {e["transaction_id"] for e in state["entries"]}
        assert [tuple(row) for row in conn.execute("SELECT * FROM transactions ORDER BY id")] == before
        custody_journal.store_ledger_state(conn, profile, state)
        conn.commit()
        items = quarantine_review.review_quarantine(conn, profile, limit=20)["items"]
        assert all(item["blocks_reports"] for item in items)
        assert all("metadata exclude" in item["detail"]["resolution"] for item in items)
        from kassiber.core.reports import report_summary
        with pytest.raises(AppError):
            report_summary(conn, "ws", "profile", handlers._report_hooks())
        target = "post-upgrade" if exclude == "current" else next(q["transaction_id"] for q in held if q["transaction_id"] != "post-upgrade")
        metadata.update_transaction_metadata(conn, "ws", "profile", target, handlers._metadata_hooks(),
            excluded=True, reason="Reviewed duplicated import occurrence")
        state = review_workflow._build(conn, profile)
        assert not any(q["reason"] == ISSUE for q in state["quarantines"])
        assert conn.execute("SELECT COUNT(*) FROM transactions").fetchone()[0] == 3
        assert conn.execute("SELECT COUNT(*) FROM transaction_edit_events WHERE transaction_id=?", (target,)).fetchone()[0] == 1
    finally:
        conn.close()


@pytest.mark.parametrize("case_index", range(len(CASES)))
def test_genuine_identical_repeats_in_one_old_export_are_not_flagged(tmp_path, case_index):
    conn, profile, wallet = _restore(tmp_path, case_index)
    try:
        conn.execute("UPDATE transactions SET excluded=0, fiat_rate=50000, fiat_rate_exact='50000'")
        conn.commit()
        state = review_workflow._build(conn, profile)
        assert not any(q["reason"] == ISSUE for q in state["quarantines"])
    finally:
        conn.close()


def test_old_row_indices_are_numbered_numerically_after_batch_time(tmp_path):
    from kassiber.core.import_identity import stored_synthetic_occurrences
    conn, profile, _ = _restore(tmp_path, 4)
    try:
        rows = conn.execute("SELECT id, external_id FROM transactions ORDER BY external_id").fetchall()
        conn.execute("UPDATE transactions SET external_id=? WHERE id=?", (rows[0]["external_id"].rsplit(":", 1)[0] + ":10", rows[0]["id"]))
        groups = stored_synthetic_occurrences(conn, profile_id=profile["id"])
        members, = groups.values()
        assert [row["ordinal"] for row in members] == [2, 10]
    finally:
        conn.close()


def test_version_two_journals_require_rebuilding_before_duplicate_detection(tmp_path):
    conn, profile, _ = _restore(tmp_path, 4)
    try:
        conn.execute("""UPDATE profiles SET last_processed_at='2025-03-01T00:00:00Z',
            last_processed_tx_count=2, last_processed_input_version=journal_input_version,
            required_journal_semantics_version=2""")
        conn.execute("UPDATE profiles SET journal_semantics_version=2")
        conn.commit()
        assert not custody_journal.projection_freshness(conn, profile["id"])["is_current"]
    finally:
        conn.close()


@pytest.mark.parametrize("case_index", range(len(CASES)))
def test_fresh_identical_occurrences_and_reimports_work_for_every_importer(tmp_path, case_index):
    conn = open_db(tmp_path / "book")
    try:
        _scope(conn)
        conn.commit()
        profile = conn.execute("SELECT * FROM profiles").fetchone()
        wallet = conn.execute("SELECT * FROM wallets WHERE id='btc'").fetchone()
        fmt, january, _ = CASES[case_index]
        path = tmp_path / "export.csv"
        assert _import(conn, profile, wallet, path, fmt, [january, january])["imported"] == 2
        assert _import(conn, profile, wallet, path, fmt, [january, january])["imported"] == 0
        assert conn.execute("SELECT COUNT(*) FROM transactions").fetchone()[0] == 2
        assert not any(q["reason"] == ISSUE for q in review_workflow._build(conn, profile)["quarantines"])
    finally:
        conn.close()


def test_api_dividend_epoch_ids_are_not_mistaken_for_csv_row_indices(tmp_path):
    from kassiber.core.import_identity import stored_synthetic_occurrences
    conn, profile, wallet = _restore(tmp_path, 1)
    try:
        conn.execute("DELETE FROM transactions")
        # The Binance API importer keys an ID-less dividend by its epoch time;
        # its JSON cells are numbers, never a CSV export's text cells.
        conn.execute("""INSERT INTO transactions(id, workspace_id, profile_id, wallet_id, external_id,
            fingerprint, occurred_at, direction, asset, amount, fee, kind, raw_json, created_at)
            VALUES('api', 'ws', 'profile', 'btc', 'binance:1735725600000', 'api',
            '2025-01-01T10:00:00Z', 'inbound', 'BTC', 200000000, 0, 'income', ?, '2025-01-02T00:00:00Z')""",
            (json.dumps({"amount": "0.002", "asset": "BTC", "divTime": 1735725600000}),))
        conn.commit()
        assert stored_synthetic_occurrences(conn, profile_id=profile["id"]) == {}
    finally:
        conn.close()


@pytest.mark.parametrize("case_index", [0, 2, 3, 4])
def test_old_rows_are_recognized_when_an_earlier_parser_spelled_the_amount_differently(tmp_path, case_index):
    conn, profile, wallet = _restore(tmp_path, case_index)
    fmt, january, _ = CASES[case_index]
    try:
        conn.execute("UPDATE transactions SET external_id=replace(external_id, ':0.002:', ':0.00200000:')")
        assert conn.execute("SELECT COUNT(*) FROM transactions WHERE external_id LIKE '%:0.00200000:%'").fetchone()[0] == 2
        conn.commit()
        assert _import(conn, profile, wallet, tmp_path / "export.csv", fmt, [january, january])["imported"] == 0
    finally:
        conn.close()


def _fresh(tmp_path):
    conn = open_db(tmp_path / "book")
    _scope(conn)
    conn.commit()
    profile = conn.execute("SELECT * FROM profiles").fetchone()
    wallet = conn.execute("SELECT * FROM wallets WHERE id='btc'").fetchone()
    return conn, profile, wallet


def _batch_ids(conn):
    return [row[0] for row in conn.execute("SELECT id FROM import_batches ORDER BY imported_at, rowid")]


@pytest.mark.parametrize("case_index", range(len(CASES)))
def test_rolled_back_first_import_does_not_swallow_a_genuine_repeat(tmp_path, case_index):
    conn, profile, wallet = _fresh(tmp_path)
    fmt, january, _ = CASES[case_index]
    path = tmp_path / "export.csv"
    try:
        assert _import(conn, profile, wallet, path, fmt, [january])["imported"] == 1
        assert _import(conn, profile, wallet, path, fmt, [january, january])["imported"] == 1
        import_batches.rollback_batch(conn, profile, _batch_ids(conn)[0],
                                      invalidate_journals=lambda *args, **kwargs: None)
        assert conn.execute("SELECT COUNT(*) FROM transactions").fetchone()[0] == 1
        # The surviving row is occurrence 2; occurrence 1 is genuinely missing.
        assert _import(conn, profile, wallet, path, fmt, [january, january])["imported"] == 1
        assert conn.execute("SELECT COUNT(*) FROM transactions").fetchone()[0] == 2
        assert _import(conn, profile, wallet, path, fmt, [january, january])["imported"] == 0
        assert not any(q["reason"] == ISSUE for q in review_workflow._build(conn, profile)["quarantines"])
    finally:
        conn.close()


@pytest.mark.parametrize("case_index", range(len(CASES)))
def test_old_rows_fill_only_occurrences_without_a_current_row(tmp_path, case_index):
    from kassiber.core.import_identity import occurrence_slots, stored_synthetic_occurrences
    conn, profile, wallet = _restore(tmp_path, case_index)
    fmt, january, _ = CASES[case_index]
    path = tmp_path / "export.csv"
    try:
        # One old row plus a current occurrence 1 from a later import.
        first, second = conn.execute("SELECT * FROM transactions ORDER BY external_id").fetchall()
        _write(path, [january])
        current, = importers.load_import_records(path, fmt)
        conn.execute("UPDATE transactions SET external_id=?, created_at='2025-04-01T00:00:00Z' WHERE id=?",
                     (current["txid"], second["id"]))
        conn.commit()
        members, = stored_synthetic_occurrences(conn, profile_id=profile["id"]).values()
        slots = occurrence_slots(members)
        assert {number: row["id"] for number, row in slots.items()} == {1: second["id"], 2: first["id"]}
        assert _import(conn, profile, wallet, path, fmt, [january] * 3)["imported"] == 1
        assert _import(conn, profile, wallet, path, fmt, [january] * 3)["imported"] == 0
    finally:
        conn.close()


def _shift_month(row, month):
    copied = dict(row)
    for field in ("external_id", "raw_json", "occurred_at", "fingerprint"):
        copied[field] = copied[field].replace("2025-01-01", f"2025-{month}-01")
    copied["id"] = copied["fingerprint"] = f"{row['id']}-{month}"
    return copied


@pytest.mark.parametrize("case_index", range(len(CASES)))
def test_duplicate_whose_first_copy_predates_import_batches_is_held(tmp_path, case_index):
    conn, profile, wallet = _restore(tmp_path, case_index)
    fmt, january, date_column = CASES[case_index]
    try:
        # Old code imported [jan] before batches existed, then the cumulative
        # [feb, jan] as one batch: the shifted position stored jan again.
        first, second = conn.execute("SELECT * FROM transactions ORDER BY external_id").fetchall()
        batch, = _batch_ids(conn)
        conn.execute("DELETE FROM import_batch_transactions WHERE transaction_id=?", (first["id"],))
        conn.execute("UPDATE transactions SET created_at='2025-01-15T00:00:00Z' WHERE id=?", (first["id"],))
        february = _shift_month(first, "02")
        _insert(conn, "transactions", february)
        _insert(conn, "import_batch_transactions", {"batch_id": batch, "transaction_id": february["id"]})
        conn.execute("UPDATE transactions SET excluded=0, fiat_rate=50000, fiat_rate_exact='50000'")
        conn.commit()
        path = tmp_path / "export.csv"
        march = {**january, date_column: january[date_column].replace("01-01", "03-01")}
        february_row = {**january, date_column: january[date_column].replace("01-01", "02-01")}
        assert _import(conn, profile, wallet, path, fmt, [march, february_row, january])["imported"] == 1
        conn.execute("UPDATE transactions SET fiat_rate=50000, fiat_rate_exact='50000'")
        conn.commit()
        before = [tuple(row) for row in conn.execute("SELECT * FROM transactions ORDER BY id")]
        state = review_workflow._build(conn, profile)
        held = sorted(q["transaction_id"] for q in state["quarantines"] if q["reason"] == ISSUE)
        assert held == sorted([first["id"], second["id"]])
        assert state["custody_quantity"].report_blocked
        assert [tuple(row) for row in conn.execute("SELECT * FROM transactions ORDER BY id")] == before
        metadata.update_transaction_metadata(conn, "ws", "profile", first["id"], handlers._metadata_hooks(),
            excluded=True, reason="Reviewed duplicated import occurrence")
        assert not any(q["reason"] == ISSUE for q in review_workflow._build(conn, profile)["quarantines"])
    finally:
        conn.close()


@pytest.mark.parametrize("case_index", range(len(CASES)))
def test_unbatched_rows_alone_are_one_export(tmp_path, case_index):
    conn, profile, _ = _restore(tmp_path, case_index)
    try:
        conn.execute("DELETE FROM import_batch_transactions")
        conn.execute("UPDATE transactions SET excluded=0, fiat_rate=50000, fiat_rate_exact='50000'")
        conn.commit()
        assert not any(q["reason"] == ISSUE for q in review_workflow._build(conn, profile)["quarantines"])
    finally:
        conn.close()


@pytest.mark.parametrize("case_index", [0, 2, 3, 4])
def test_old_id_with_another_date_is_not_a_positional_occurrence(tmp_path, case_index):
    conn, profile, wallet = _restore(tmp_path, case_index)
    fmt, january, _ = CASES[case_index]
    try:
        first, second = conn.execute("SELECT * FROM transactions ORDER BY external_id").fetchall()
        conn.execute("UPDATE transactions SET external_id=replace(external_id, '2025-01-01', '2024-12-31') WHERE id=?",
                     (first["id"],))
        conn.commit()
        changed = conn.execute("SELECT external_id, raw_json FROM transactions WHERE id=?", (first["id"],)).fetchone()
        assert importers.synthetic_csv_identity(changed["external_id"], changed["raw_json"]) is None
        assert importers.synthetic_csv_identity(second["external_id"], second["raw_json"]) is not None
        assert _import(conn, profile, wallet, tmp_path / "export.csv", fmt, [january, january])["imported"] == 1
    finally:
        conn.close()


@pytest.mark.parametrize("case_index", range(len(CASES)))
def test_current_occurrence_reimport_still_reconciles_its_stored_row(tmp_path, case_index):
    conn, profile, wallet = _fresh(tmp_path)
    fmt, january, _ = CASES[case_index]
    path = tmp_path / "export.csv"
    try:
        assert _import(conn, profile, wallet, path, fmt, [january])["imported"] == 1
        fingerprint = conn.execute("SELECT fingerprint FROM transactions").fetchone()[0]
        conn.execute("UPDATE transactions SET fingerprint='stale'")
        conn.commit()
        # An exact current ID goes through the normal merge, not the old-row skip.
        result = _import(conn, profile, wallet, path, fmt, [january])
        assert (result["imported"], len(result["updated_records"])) == (0, 1)
        assert conn.execute("SELECT fingerprint FROM transactions").fetchone()[0] == fingerprint
    finally:
        conn.close()


def _false_positive(tmp_path, case_index):
    """Old code imported [jan], then [jan, jan] whose second row is genuine."""
    conn, profile, wallet = _restore(tmp_path, case_index)
    first, second = conn.execute("SELECT * FROM transactions ORDER BY external_id").fetchall()
    batch = "repeat-batch"
    _insert(conn, "import_batches", {**dict(conn.execute("SELECT * FROM import_batches").fetchone()),
                                     "id": batch, "imported_at": "2025-04-01T00:00:00Z", "rows_inserted": 1})
    conn.execute("UPDATE import_batch_transactions SET batch_id=? WHERE transaction_id=?", (batch, second["id"]))
    conn.execute("UPDATE transactions SET excluded=0, fiat_rate=50000, fiat_rate_exact='50000'")
    conn.commit()
    return conn, profile, wallet, first, second, batch


@pytest.mark.parametrize("case_index", range(len(CASES)))
def test_owner_can_confirm_genuinely_distinct_rows_without_dropping_a_trade(tmp_path, case_index):
    from kassiber.daemon import _validate_ai_review_operations
    conn, profile, _, first, second, _ = _false_positive(tmp_path, case_index)
    try:
        hooks = review_workflow.ReviewHooks(metadata=handlers._metadata_hooks())
        held = sorted(q["transaction_id"] for q in review_workflow._build(conn, profile)["quarantines"]
                      if q["reason"] == ISSUE)
        assert held == sorted([first["id"], second["id"]])
        case = next(case for case in review_workflow.inspect_cases(conn, profile)["cases"] if case["reason"] == ISSUE)
        assert case["supported_operations"] == ["exclude", "confirm_distinct_import_rows"]
        assert case["import_duplicate"]["transaction_ids"] == held
        operation = {"type": "confirm_distinct_import_rows", "transaction_id": case["transaction_id"],
                     "expected_fingerprint": case["import_duplicate"]["group_fingerprint"],
                     "reason": "Both rows are separate purchases in the provider statement"}
        with pytest.raises(AppError) as ai_error:
            _validate_ai_review_operations([operation])
        assert ai_error.value.code == "interaction_required"
        version = conn.execute("SELECT journal_input_version FROM profiles").fetchone()[0]
        with pytest.raises(AppError) as stale:
            review_workflow.plan_review(conn, profile, operations=[{**operation, "expected_fingerprint": "0" * 64}],
                                        expected_input_version=version, hooks=hooks)
        assert stale.value.code == "review_case_changed"
        with pytest.raises(AppError):
            review_workflow.plan_review(conn, profile, operations=[{**operation, "pair_id": "x"}],
                                        expected_input_version=version, hooks=hooks)
        before = [tuple(row) for row in conn.execute("SELECT * FROM transactions ORDER BY id")]
        plan = review_workflow.plan_review(conn, profile, operations=[operation],
                                           expected_input_version=version, hooks=hooks)
        assert conn.execute("SELECT COUNT(*) FROM synthetic_import_duplicate_reviews").fetchone()[0] == 0
        assert {q["reason"] for q in plan["before"]["quarantines"]} >= {ISSUE}
        assert ISSUE not in {q["reason"] for q in plan["after"]["quarantines"]}
        receipt = review_workflow.apply_review(conn, profile, artifact=plan, idempotency_key="distinct", hooks=hooks)
        assert receipt["status"] == "verified"
        assert receipt["transaction_ids"] == held
        assert [tuple(row) for row in conn.execute("SELECT * FROM transactions ORDER BY id")] == before
        state = review_workflow._build(conn, profile)
        assert ISSUE not in {q["reason"] for q in state["quarantines"]}
        assert {first["id"], second["id"]} <= {e["transaction_id"] for e in state["entries"]}
        with pytest.raises(AppError):
            review_workflow.apply_review(conn, profile, artifact=plan, idempotency_key="replay", hooks=hooks)
        stored = conn.execute("SELECT * FROM synthetic_import_duplicate_reviews").fetchone()
        assert (stored["reason"], stored["authored_source"]) == (operation["reason"], "user")
        import sqlite3
        with pytest.raises(sqlite3.DatabaseError):
            conn.execute("UPDATE synthetic_import_duplicate_reviews SET reason='edited'")
    finally:
        conn.close()


@pytest.mark.parametrize("case_index", [0, 4])
def test_confirmation_covers_only_the_group_it_reviewed(tmp_path, case_index):
    from kassiber.core.import_identity import confirm_distinct, held_duplicate_groups
    conn, profile, _, first, second, _ = _false_positive(tmp_path, case_index)
    try:
        group, = held_duplicate_groups(conn, profile["id"])
        confirm_distinct(conn, profile, transaction_id=first["id"], expected_fingerprint=group["group_fingerprint"],
                         reason="Separate purchases", authored_source="cli")
        conn.commit()
        assert held_duplicate_groups(conn, profile["id"]) == []
        with pytest.raises(AppError) as again:
            confirm_distinct(conn, profile, transaction_id=first["id"], expected_fingerprint=group["group_fingerprint"],
                             reason="Separate purchases", authored_source="cli")
        assert again.value.code == "review_case_changed"
        # A later third copy changes the group: the hold returns for all three.
        copied = {**dict(second), "id": "third", "fingerprint": "third", "created_at": "2025-05-01T00:00:00Z"}
        _insert(conn, "transactions", copied)
        batch = import_batches.record_batch(conn, profile, wallet_id="btc", source_format=CASES[case_index][0],
            source_filename="later.csv", column_map=None, outcome={"inserted_records": [{"transaction_id": "third"}]})
        conn.execute("UPDATE import_batches SET imported_at='2025-05-01T00:00:00Z' WHERE id=?", (batch,))
        conn.commit()
        held = sorted(q["transaction_id"] for q in review_workflow._build(conn, profile)["quarantines"]
                      if q["reason"] == ISSUE)
        assert held == sorted([first["id"], second["id"], "third"])
    finally:
        conn.close()
