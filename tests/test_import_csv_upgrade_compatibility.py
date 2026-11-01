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
