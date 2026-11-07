"""Compact retained calculations, stable identities and changed-row writes.

See docs/reference/r4-journal-pipeline.md#storage-format-speedups.
"""
from __future__ import annotations

import json

import pytest

from kassiber.cli.handlers import get_journal_event
from kassiber.core import custody_journal
from kassiber.core.journal_calculation import (
    decode_compact,
    encode_calculation,
    legacy_text,
    stored_calculation,
)
from kassiber.core.report_explanation import explain_capital_gain, result_reference
from kassiber.db import open_db


def _book(root, method="fifo", country="generic", *, passphrase=None):
    conn = open_db(root, passphrase=passphrase)
    conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('w','W','2020')")
    conn.execute(
        "INSERT INTO profiles(id,workspace_id,label,gains_algorithm,tax_country,fiat_currency,created_at) "
        "VALUES('p','w','P',?,?,'EUR','2020')",
        (method, country),
    )
    conn.execute(
        "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) "
        "VALUES('wallet','w','p','Wallet','manual','2020')"
    )
    for tx, date, direction, amount, fee, price, kind in (
        ("a", "2024-01-01", "inbound", 100000000000, 0, "10000", "buy"),
        ("b", "2024-02-01", "inbound", 100000000000, 0, "20000", "buy"),
        ("s", "2024-03-01", "outbound", 150000000000, 1000000, "30000", "sell"),
        ("i", "2024-03-02", "inbound", 1000000000, 0, "30000", "interest"),
        ("t", "2024-04-01", "outbound", 20000000000, 0, "31000.5", "sell"),
    ):
        conn.execute(
            """INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,
            amount,fee,kind,fiat_currency,fiat_rate,fiat_rate_exact,pricing_source_kind,pricing_quality,
            pricing_provider,raw_json,created_at)
            VALUES(?,'w','p','wallet',?,?,?,'BTC',?,?,?,'EUR',?,?,'manual','exact','reviewed-receipt','{}','2020')""",
            (tx, tx, date + "T00:00:00Z", direction, amount, fee, kind, float(price), price),
        )
    conn.commit()
    _process(conn)
    return conn


def _profile(conn):
    return conn.execute("SELECT * FROM profiles WHERE id = 'p'").fetchone()


def _process(conn):
    profile = _profile(conn)
    state = custody_journal.build_ledger_state(conn, profile)
    assert state["quarantines"] == []
    custody_journal.store_ledger_state(conn, profile, state)
    conn.commit()
    return state


def _as_legacy(conn):
    """Rewrite every compact row the way a pre-compact build stored it."""
    for row in conn.execute(
        "SELECT id, calculation_compact FROM journal_entries WHERE calculation_compact IS NOT NULL"
    ).fetchall():
        conn.execute(
            "UPDATE journal_entries SET calculation_json = ?, calculation_compact = NULL WHERE id = ?",
            (legacy_text(row["calculation_compact"]), row["id"]),
        )
    conn.commit()


def _realized_calculations(state):
    return [entry["calculation"] for entry in state["entries"] if entry.get("calculation")]


def _explanations(conn):
    profile = _profile(conn)
    return {
        row["id"]: json.dumps(
            explain_capital_gain(conn, profile, result_reference(conn, profile, row["id"]))
        )
        for row in conn.execute(
            "SELECT id FROM journal_entries WHERE entry_type IN ('disposal', 'income') ORDER BY id"
        ).fetchall()
    }


@pytest.mark.parametrize("method,country", [("fifo", "generic"), ("moving_average_at", "at")])
def test_realized_entries_store_compact_calculations_that_restore_legacy_bytes(tmp_path, method, country):
    conn = _book(tmp_path / "data", method, country)
    try:
        state = _process(conn)
        calculations = _realized_calculations(state)
        assert calculations
        rows = conn.execute(
            "SELECT * FROM journal_entries WHERE entry_type IN ('disposal', 'income')"
        ).fetchall()
        assert rows and all(row["calculation_compact"] and row["calculation_json"] is None for row in rows)
        assert conn.execute(
            "SELECT COUNT(*) FROM journal_entries WHERE calculation_json IS NOT NULL"
        ).fetchone()[0] == 0
        legacy = sorted(json.dumps(calculation, sort_keys=True) for calculation in calculations)
        assert sorted(legacy_text(row["calculation_compact"]) for row in rows) == legacy
        for row in rows:
            decoded = stored_calculation(row)
            assert json.dumps(decoded) == json.dumps(json.loads(legacy_text(row["calculation_compact"])))
            assert len(row["calculation_compact"]) < len(legacy_text(row["calculation_compact"]))
    finally:
        conn.close()


def test_explanations_and_event_reads_are_identical_for_compact_and_legacy_rows(tmp_path):
    conn = _book(tmp_path / "data")
    try:
        profile = _profile(conn)
        compact = _explanations(conn)
        assert compact and all(json.loads(value)["status"] == "available" for value in compact.values())
        events = {
            entry_id: json.dumps(get_journal_event(conn, "W", "P", entry_id), sort_keys=False)
            for entry_id in compact
        }
        assert all("calculation_compact" not in json.loads(value) for value in events.values())
        _as_legacy(conn)
        assert _explanations(conn) == compact
        assert {
            entry_id: json.dumps(get_journal_event(conn, "W", "P", entry_id), sort_keys=False)
            for entry_id in compact
        } == events
        assert _profile(conn)["last_processed_at"] == profile["last_processed_at"]
    finally:
        conn.close()


def test_opening_a_pre_compact_book_adds_the_column_and_reads_legacy_rows(tmp_path):
    root = tmp_path / "data"
    conn = _book(root)
    _as_legacy(conn)
    explanations = _explanations(conn)
    legacy_rows = conn.execute(
        "SELECT id, calculation_json FROM journal_entries ORDER BY id"
    ).fetchall()
    conn.execute("ALTER TABLE journal_entries DROP COLUMN calculation_compact")
    conn.commit()
    conn.close()

    conn = open_db(root)
    try:
        columns = [row["name"] for row in conn.execute("PRAGMA table_info(journal_entries)")]
        assert columns[-1] == "calculation_compact"
        assert conn.execute(
            "SELECT id, calculation_json FROM journal_entries ORDER BY id"
        ).fetchall() == legacy_rows
        assert conn.execute(
            "SELECT COUNT(*) FROM journal_entries WHERE calculation_compact IS NOT NULL"
        ).fetchone()[0] == 0
        assert _explanations(conn) == explanations
        _process(conn)
        assert conn.execute(
            "SELECT COUNT(*) FROM journal_entries WHERE calculation_json IS NOT NULL"
        ).fetchone()[0] == 0
    finally:
        conn.close()


@pytest.mark.parametrize("legacy_backup", [False, True])
def test_compact_calculations_survive_encrypted_backup_and_restore(tmp_path, legacy_backup):
    from kassiber.backup.pack import export_backup, import_backup

    source = tmp_path / "source" / "data"
    target = tmp_path / "restored" / "data"
    passphrase = "synthetic journal storage test"
    conn = _book(source, passphrase=passphrase)
    try:
        if legacy_backup:
            _as_legacy(conn)
            conn.execute("ALTER TABLE journal_entries DROP COLUMN calculation_compact")
            conn.commit()
        expected = _explanations(conn)
        columns = ",".join(
            row["name"] for row in conn.execute("PRAGMA table_info(journal_entries)")
        )
        rows = [
            tuple(row)
            for row in conn.execute(f"SELECT {columns} FROM journal_entries ORDER BY id")
        ]
    finally:
        conn.close()
    archive = tmp_path / "journal.kassiber"
    export_backup(str(source), archive, passphrase, backup_passphrase="synthetic archive test")
    import_backup(archive, target, backup_passphrase="synthetic archive test", move_into_place=True)
    restored = open_db(target, passphrase=passphrase)
    try:
        assert [
            tuple(row)
            for row in restored.execute(f"SELECT {columns} FROM journal_entries ORDER BY id")
        ] == rows
        assert _explanations(restored) == expected
        assert restored.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
        if legacy_backup:
            assert restored.execute(
                "SELECT COUNT(*) FROM journal_entries WHERE calculation_compact IS NOT NULL"
            ).fetchone()[0] == 0
            _process(restored)
        assert restored.execute(
            "SELECT COUNT(*) FROM journal_entries WHERE calculation_compact IS NOT NULL"
        ).fetchone()[0] > 0
    finally:
        restored.close()


def _calculation():
    source = {
        "asset": "BTC",
        "crypto_fee_msat": 5,
        "crypto_fee_msat_exact": "5",
        "engine_event_id": "custody-tax:é:0:1:external",
        "fiat_fee_exact": "0.1",
        "occurred_at": "2024-01-01T00:00:00+00:00",
        "pricing": {
            key: None
            for key in (
                "fiat_currency", "fiat_rate_exact", "fiat_value_exact", "pricing_source_kind",
                "pricing_quality", "fiat_price_source", "pricing_timestamp", "pricing_provider",
                "pricing_pair", "pricing_fetched_at", "pricing_granularity", "pricing_method",
            )
        },
        "spot_price_exact": "1",
        "transaction_id": "tx",
    }
    fragment = {
        "quantity_msat": 7,
        "quantity_msat_exact": "7",
        "cost_basis_exact": "1",
        "proceeds_exact": "2",
        "gain_loss_exact": "1",
        "unit_basis_override_exact": None,
        "basis_authority": "acquired_lot",
        "acquisition_basis_exact": "3",
        "acquisition_quantity_msat": 9,
        "acquisition_quantity_msat_exact": "9",
        "event": source,
        "lot": {**source, "transaction_id": "lot", "pricing": {**source["pricing"], "pricing_method": "carrying_value"}},
    }
    income = {**fragment, "lot": None, "acquisition_quantity_msat": None,
              "acquisition_quantity_msat_exact": None, "basis_authority": "income"}
    return {"schema_version": 1, "engine": "kassiber_tax", "method": "FIFO",
            "fragments": [fragment, {**fragment, "event": dict(source)}, income]}


def test_compact_encoding_round_trips_and_shares_equal_sources():
    calculation = _calculation()
    legacy, compact = encode_calculation(calculation)
    assert legacy is None
    encoded = json.loads(compact)
    assert encoded[0] == 2 and len(encoded[3]) == 2  # one event, one lot
    assert legacy_text(compact) == json.dumps(calculation, sort_keys=True)
    decoded = decode_compact(compact)
    assert decoded == json.loads(json.dumps(calculation, sort_keys=True))
    first, second, _ = decoded["fragments"]
    assert first["event"] == second["event"] and first["event"] is not second["event"]
    first["lot"]["inherited_basis"] = {}
    assert "inherited_basis" not in second["lot"]
    assert encode_calculation(None) == (None, None)
    assert encode_calculation({}) == (None, None)


@pytest.mark.parametrize(
    "mutate",
    [
        lambda c: c.update(extra=1),
        lambda c: c.update(schema_version=True),
        lambda c: c["fragments"][0].update(quantity_msat_exact="07"),
        lambda c: c["fragments"][0].update(quantity_msat=7.0, quantity_msat_exact="7.0"),
        lambda c: c["fragments"][0].update(cost_basis_exact=1.5),
        lambda c: c["fragments"][0]["event"].update(crypto_fee_msat=True, crypto_fee_msat_exact="True"),
        lambda c: c["fragments"][0]["event"]["pricing"].update(fiat_rate_exact=1),
        lambda c: c["fragments"][0]["event"]["pricing"].pop("pricing_method"),
        lambda c: c["fragments"][2].update(acquisition_quantity_msat_exact="0"),
        lambda c: c["fragments"].append({}),
        lambda c: c.update(fragments=()),
    ],
)
def test_calculations_outside_the_encoding_keep_legacy_bytes(mutate):
    calculation = _calculation()
    mutate(calculation)
    try:
        expected = json.dumps(calculation, sort_keys=True)
    except TypeError:
        pytest.skip("not JSON")
    assert encode_calculation(calculation) == (expected, None)


@pytest.mark.parametrize(
    "text",
    ["[1,\"e\",\"m\",[],[]]", "[2,\"e\",\"m\",[],[[1,null,null,null,null,null,null,null,0,null]]]",
     "{\"fragments\":[]}", "[2,\"e\",\"m\",[[]],[[1,null,null,null,null,null,null,null,0,null]]]", "[2,\"e\",\"m\",[],[[1.5,null,null,null,null,null,null,null,null,null]]]",
     "[2,\"e\",\"m\",[],[]", "[true,\"e\",\"m\",[],[]]"],
)
def test_malformed_compact_text_is_rejected(text):
    with pytest.raises(ValueError):
        decode_compact(text)
    assert legacy_text(text) == text


def test_corrupt_compact_detail_does_not_claim_reconciliation(tmp_path):
    conn = _book(tmp_path / "data")
    try:
        profile = _profile(conn)
        row = conn.execute("SELECT * FROM journal_entries WHERE entry_type = 'disposal' LIMIT 1").fetchone()
        reference = result_reference(conn, profile, row["id"])
        encoded = json.loads(row["calculation_compact"])
        encoded[4][0][3] = "999999"
        conn.execute(
            "UPDATE journal_entries SET calculation_compact = ? WHERE id = ?",
            (json.dumps(encoded, separators=(",", ":")), row["id"]),
        )
        conn.commit()
        result = explain_capital_gain(conn, profile, reference)
        assert result["status"] == "engine_detail_mismatch"
        assert result["calculation"] is None
        conn.execute("UPDATE journal_entries SET calculation_compact = '[2' WHERE id = ?", (row["id"],))
        conn.commit()
        assert explain_capital_gain(conn, profile, reference)["status"] == "engine_detail_mismatch"
    finally:
        conn.close()


def _stored_rows(conn, table="journal_entries", key="id"):
    return {row[key]: tuple(row) for row in conn.execute(f"SELECT rowid, * FROM {table}")}


def _store_at(conn, state, stamp, *, fresh=False):
    if fresh:
        conn.execute("DELETE FROM journal_entries WHERE profile_id='p'")
        conn.execute("DELETE FROM journal_quantity_postings WHERE profile_id='p'")
    custody_journal.store_ledger_state(conn, _profile(conn), state, created_at=stamp)
    conn.commit()


def _large_table_writes(conn):
    from contextlib import contextmanager

    @contextmanager
    def capture():
        statements = []
        def record(sql):
            sql = sql.strip()
            if any(sql.startswith(prefix + table) for prefix in ("INSERT INTO ", "DELETE FROM ")
                   for table in ("journal_entries", "journal_quantity_postings")):
                statements.append(sql)
        conn.set_trace_callback(record)
        try:
            yield statements
        finally:
            conn.set_trace_callback(None)
    return capture()


def test_entry_ids_are_stable_uuid8_in_engine_order(tmp_path):
    from collections import defaultdict
    from uuid import UUID
    from kassiber.core.journal_storage import journal_entry_id

    conn = _book(tmp_path / "data")
    try:
        first = custody_journal.build_ledger_state(conn, _profile(conn))
        second = custody_journal.build_ledger_state(conn, _profile(conn))
        assert [e["id"] for e in first["entries"]] == [e["id"] for e in second["entries"]]
        ordinals = defaultdict(int)
        for entry in first["entries"]:
            key = tuple(entry[field] for field in ("transaction_id", "wallet_id", "entry_type", "asset"))
            assert entry["id"] == journal_entry_id("p", *key, ordinals[key])
            assert UUID(entry["id"]).version == 8
            ordinals[key] += 1
        assert journal_entry_id("p", "a", "wallet", "acquisition", "BTC", 0) != journal_entry_id("other", "a", "wallet", "acquisition", "BTC", 0)
    finally:
        conn.close()


def test_unchanged_build_writes_no_entries_or_postings_and_preserves_creation_times(tmp_path):
    from kassiber.core.ui_snapshot import build_audit_changes_since_last_answer_snapshot
    from kassiber.db import set_setting

    conn = _book(tmp_path / "data")
    try:
        state = custody_journal.build_ledger_state(conn, _profile(conn))
        _store_at(conn, state, "2030-01-01T00:00:00Z", fresh=True)
        entries = _stored_rows(conn)
        postings = _stored_rows(conn, "journal_quantity_postings", "posting_id")
        rebuilt = custody_journal.build_ledger_state(conn, _profile(conn))
        with _large_table_writes(conn) as writes:
            _store_at(conn, rebuilt, "2030-01-02T00:00:00Z")
        assert writes == []
        assert _stored_rows(conn) == entries
        assert _stored_rows(conn, "journal_quantity_postings", "posting_id") == postings
        assert _profile(conn)["last_processed_at"] == "2030-01-02T00:00:00Z"
        set_setting(conn, "context_workspace", "w")
        set_setting(conn, "context_profile", "p")
        audit = build_audit_changes_since_last_answer_snapshot(conn, {"since": "2030-01-01T12:00:00Z"})
        assert audit["counts_since"]["journal_entries"] == 0
        assert audit["changed"] is False
        assert audit["latest"]["journal_entries"] == "2030-01-01T00:00:00Z"
        assert audit["latest"]["journals_processed_at"] == "2030-01-02T00:00:00Z"
    finally:
        conn.close()


def test_changed_entry_rewrites_its_whole_timestamp_group_in_engine_order(tmp_path):
    conn = _book(tmp_path / "data")
    try:
        state = custody_journal.build_ledger_state(conn, _profile(conn))
        entries = state["entries"]
        entries[1]["occurred_at"] = entries[0]["occurred_at"]
        tie = entries[0]["occurred_at"]
        group = [entry["id"] for entry in entries if entry["occurred_at"] == tie]
        _store_at(conn, state, "2030-01-01T00:00:00Z", fresh=True)
        old = _stored_rows(conn)
        entries[1]["description"] += " reviewed"
        with _large_table_writes(conn) as writes:
            _store_at(conn, state, "2030-01-02T00:00:00Z")
        assert sum(sql.startswith("INSERT INTO journal_entries") for sql in writes) == len(group)
        assert sum(sql.startswith("DELETE FROM journal_entries") for sql in writes) == len(group)
        assert not any("journal_quantity_postings" in sql for sql in writes)
        assert [row[0] for row in conn.execute("SELECT id FROM journal_entries WHERE occurred_at=? ORDER BY rowid", (tie,))] == group
        assert {row[0] for row in conn.execute("SELECT created_at FROM journal_entries WHERE occurred_at=?", (tie,))} == {"2030-01-02T00:00:00Z"}
        current = _stored_rows(conn)
        assert {key: value for key, value in current.items() if key not in group} == {key: value for key, value in old.items() if key not in group}
        ordered = conn.execute("SELECT id FROM journal_entries WHERE occurred_at=? ORDER BY occurred_at,created_at,id", (tie,)).fetchall()
        assert [row[0] for row in ordered] == sorted(group)
    finally:
        conn.close()


def test_inconsistent_kept_creation_times_are_repaired_as_one_group(tmp_path):
    conn = _book(tmp_path / "data")
    try:
        state = custody_journal.build_ledger_state(conn, _profile(conn))
        for entry in state["entries"]:
            entry["occurred_at"] = "2024-01-01T00:00:00Z"
        _store_at(conn, state, "2030-01-01T00:00:00Z", fresh=True)
        conn.execute("UPDATE journal_entries SET created_at='external' WHERE id=?", (state["entries"][0]["id"],))
        conn.commit()
        _store_at(conn, state, "2030-01-02T00:00:00Z")
        assert {row[0] for row in conn.execute("SELECT created_at FROM journal_entries")} == {"2030-01-02T00:00:00Z"}
    finally:
        conn.close()


def test_removed_entries_are_deleted_without_rewriting_their_kept_peers(tmp_path):
    conn = _book(tmp_path / "data")
    try:
        state = custody_journal.build_ledger_state(conn, _profile(conn))
        for entry in state["entries"]:
            entry["occurred_at"] = "2024-01-01T00:00:00Z"
        _store_at(conn, state, "2030-01-01T00:00:00Z", fresh=True)
        old = _stored_rows(conn)
        removed = state["entries"].pop()["id"]
        with _large_table_writes(conn) as writes:
            _store_at(conn, state, "2030-01-02T00:00:00Z")
        assert len(writes) == 1 and writes[0].startswith("DELETE FROM journal_entries")
        assert _stored_rows(conn) == {key: value for key, value in old.items() if key != removed}
        state["entries"] = []
        _store_at(conn, state, "2030-01-03T00:00:00Z")
        assert not _stored_rows(conn)
    finally:
        conn.close()


def test_quantity_postings_write_only_changed_and_removed_rows(tmp_path):
    from kassiber.core.journal_storage import POSTING_COLUMNS, replace_quantity_postings

    conn = _book(tmp_path / "data")
    try:
        columns = ','.join(POSTING_COLUMNS)
        rows = [tuple(row) for row in conn.execute(f"SELECT {columns} FROM journal_quantity_postings ORDER BY posting_id")]
        old = _stored_rows(conn, "journal_quantity_postings", "posting_id")
        first = list(rows[0]); first[9] += 1; first[-1] = "2030-01-02T00:00:00Z"
        removed = rows[-1][0]
        with _large_table_writes(conn) as writes:
            replace_quantity_postings(conn, 'p', [tuple(first), *rows[1:-1]])
        assert sum(sql.startswith('INSERT') for sql in writes) == 1
        assert sum(sql.startswith('DELETE') for sql in writes) == 2
        actual = _stored_rows(conn, 'journal_quantity_postings', 'posting_id')
        assert removed not in actual
        assert all(actual[row[0]] == old[row[0]] for row in rows[1:-1])
        assert actual[first[0]][-1] == first[-1]
        # A duplicate matching a retained row must still fail closed.
        import sqlite3
        with pytest.raises(sqlite3.IntegrityError, match='duplicate journal_quantity_postings identifier'):
            replace_quantity_postings(conn, 'p', [tuple(first),tuple(first)])
    finally:
        conn.close()


def test_stored_real_comparison_preserves_signed_zero():
    from kassiber.core.journal_storage import _same_values

    assert _same_values((1, 0.0, 'old'), (1, 0.0, 'new'), (1,))
    assert _same_values((1, None, 'old'), (1, None, 'new'), (1,))
    assert not _same_values((1, -0.0, 'old'), (1, 0.0, 'new'), (1,))
    assert not _same_values((1, 0.0, 'old'), (1, -0.0, 'new'), (1,))
    assert not _same_values((1, 1.0, 'old'), (1, 2.0, 'new'), (1,))


@pytest.mark.parametrize("entries", [False, True])
def test_duplicate_retained_identifiers_still_fail_closed(tmp_path, entries):
    import sqlite3
    from kassiber.core.journal_storage import (
        ENTRY_COLUMNS, POSTING_COLUMNS, replace_journal_entries, replace_quantity_postings,
    )

    conn = _book(tmp_path / "data")
    try:
        table, columns, key, writer = (
            ("journal_entries", ENTRY_COLUMNS, "id", replace_journal_entries) if entries else
            ("journal_quantity_postings", POSTING_COLUMNS, "posting_id", replace_quantity_postings)
        )
        row = tuple(conn.execute(f"SELECT {','.join(columns)} FROM {table} LIMIT 1").fetchone())
        before = _stored_rows(conn, table, key)
        with pytest.raises(sqlite3.IntegrityError):
            writer(conn, "p", [row, row])
        assert _stored_rows(conn, table, key) == before
    finally:
        conn.close()


@pytest.mark.parametrize("parts,expected", [
    (("p", "a", "wallet", "acquisition", "BTC", 0), "7094e0de-d003-89b8-a630-14241e5a99a9"),
    (("p", "a", "wallet", "acquisition", "BTC", 1), "2623e539-b44c-8a46-ab54-239e227974fe"),
    (("profilé", "tx😀", 'w"\\\n', "income", "LBTC", 0), "912c938f-4716-829f-86ca-5637f570d6d0"),
])
def test_entry_id_frozen_cross_runtime_vectors(parts, expected):
    from kassiber.core.journal_storage import journal_entry_id

    assert journal_entry_id(*parts) == expected


def test_reader_counts_repeated_entry_identities_in_engine_order():
    from kassiber.core.engines.ledger_response import _EntryReader
    from kassiber.core.journal_storage import journal_entry_id

    reader = _EntryReader(
        {"id": "p", "workspace_id": "w", "gains_algorithm": "fifo"},
        {"a": {"id": "a", "occurred_at": "2024-01-01T00:00:00Z"}},
        {"Wallet": {"id": "wallet", "wallet_account_id": None}}, [], [],
    )
    item = {"row": "a", "wallet": "Wallet", "asset": "BTC", "fiat_value": "1E0",
            "quantity": "1E0", "unit_cost": "1E0", "description": "Synthetic lot"}
    first = reader.acquisition(item)
    other = reader.acquisition({**item, "asset": "LBTC"})
    second = reader.acquisition(item)
    assert first["id"] == journal_entry_id("p", "a", "wallet", "acquisition", "BTC", 0)
    assert second["id"] == journal_entry_id("p", "a", "wallet", "acquisition", "BTC", 1)
    assert other["id"] == journal_entry_id("p", "a", "wallet", "acquisition", "LBTC", 0)


def test_noop_rebuild_preserves_list_cursors_but_expires_explanation_references(tmp_path):
    from kassiber.cli.handlers import list_journal_entries, list_journal_events
    from kassiber.errors import AppError

    conn = _book(tmp_path / "data")
    try:
        state = custody_journal.build_ledger_state(conn, _profile(conn))
        _store_at(conn, state, "2030-01-01T00:00:00Z", fresh=True)
        first = list_journal_events(conn, "W", "P", limit=2)
        next_events = list_journal_events(conn, "W", "P", limit=2, cursor=first["next_cursor"])
        _, meta = list_journal_entries(conn, "W", "P", limit=2, return_meta=True)
        next_entries = list_journal_entries(conn, "W", "P", limit=2, cursor=meta["next_cursor"])
        entry_id = conn.execute("SELECT id FROM journal_entries WHERE entry_type='disposal'").fetchone()[0]
        reference = result_reference(conn, _profile(conn), entry_id)
        _store_at(conn, custody_journal.build_ledger_state(conn, _profile(conn)), "2030-01-02T00:00:00Z")
        assert list_journal_events(conn, "W", "P", limit=2, cursor=first["next_cursor"]) == next_events
        assert list_journal_entries(conn, "W", "P", limit=2, cursor=meta["next_cursor"]) == next_entries
        with pytest.raises(AppError) as stale:
            explain_capital_gain(conn, _profile(conn), reference)
        assert stale.value.code == "report_explanation_stale"
        assert explain_capital_gain(conn, _profile(conn), result_reference(conn, _profile(conn), entry_id))["status"] == "available"
    finally:
        conn.close()


def test_global_id_collision_rolls_back_the_rebuild_and_leaves_freshness_stale(tmp_path, monkeypatch):
    import sqlite3
    from kassiber.core.engines import ledger_response
    from kassiber.core.journal_storage import ENTRY_COLUMNS, journal_entry_id

    conn = _book(tmp_path / "data")
    try:
        conn.execute("INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('q','w','Other','2020')")
        conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('qwallet','w','q','Other','manual','2020')")
        conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,kind,raw_json,created_at) VALUES('qtx','w','q','qwallet','qtx','2024-01-01T00:00:00Z','inbound','BTC',1,'buy','{}','2020')")
        columns = ','.join(ENTRY_COLUMNS)
        foreign = list(conn.execute(f"SELECT {columns} FROM journal_entries LIMIT 1").fetchone())
        foreign[:6] = ['foreign-id', 'w', 'q', 'qtx', 'qwallet', None]
        conn.execute(f"INSERT INTO journal_entries({columns}) VALUES({','.join('?' for _ in ENTRY_COLUMNS)})", foreign)
        conn.execute("UPDATE profiles SET journal_input_version=journal_input_version+1 WHERE id='p'")
        conn.commit()
        entries = _stored_rows(conn)
        postings = _stored_rows(conn, 'journal_quantity_postings', 'posting_id')
        markers = tuple(_profile(conn))
        def collide(*parts):
            return 'foreign-id' if parts[1:] == ('a','wallet','acquisition','BTC',0) else journal_entry_id(*parts)
        monkeypatch.setattr(ledger_response, 'journal_entry_id', collide)
        with pytest.raises(sqlite3.IntegrityError):
            custody_journal.process_journals(
                conn, 'W', 'P', repair_source_overlaps=lambda *_: None,
                source_overlap_warning=lambda *_: None, auto_price=lambda *_: 0,
            )
        assert _stored_rows(conn) == entries
        assert _stored_rows(conn, 'journal_quantity_postings', 'posting_id') == postings
        assert tuple(_profile(conn)) == markers
        assert not custody_journal.projection_freshness(conn, _profile(conn))['is_current']
        assert not conn.in_transaction
    finally:
        conn.close()


def test_posting_changed_rows_are_scoped_by_the_composite_primary_key(tmp_path):
    from kassiber.core.journal_storage import POSTING_COLUMNS, replace_quantity_postings

    conn = _book(tmp_path / "data")
    try:
        conn.execute("INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('q','w','Other','2020')")
        columns = ','.join(POSTING_COLUMNS)
        row = list(conn.execute(f"SELECT {columns} FROM journal_quantity_postings LIMIT 1").fetchone())
        row[2] = 'q'
        conn.execute(f"INSERT INTO journal_quantity_postings({columns}) VALUES({','.join('?' for _ in POSTING_COLUMNS)})", row)
        expected = conn.execute(f"SELECT rowid,{columns} FROM journal_quantity_postings WHERE profile_id='q'").fetchone()
        replace_quantity_postings(conn, 'p', [])
        assert conn.execute(f"SELECT rowid,{columns} FROM journal_quantity_postings WHERE profile_id='q'").fetchone() == expected
        assert conn.execute("SELECT count(*) FROM journal_quantity_postings WHERE profile_id='p'").fetchone()[0] == 0
    finally:
        conn.close()


def test_same_second_rebuilds_expire_every_explanation_reference(tmp_path, monkeypatch):
    from kassiber.errors import AppError

    stamp = "2030-01-01T00:00:00Z"
    monkeypatch.setattr(custody_journal, 'now_iso', lambda: stamp)
    conn = _book(tmp_path / 'data')
    try:
        entry_id = conn.execute("SELECT id FROM journal_entries WHERE entry_type='disposal'").fetchone()[0]
        references = []
        for _ in range(3):
            reference = result_reference(conn, _profile(conn), entry_id)
            references.append(reference)
            _process(conn)
            assert explain_capital_gain(conn, _profile(conn), result_reference(conn, _profile(conn), entry_id))['status'] == 'available'
            for previous in references:
                with pytest.raises(AppError) as stale:
                    explain_capital_gain(conn, _profile(conn), previous)
                assert stale.value.code == 'report_explanation_stale'
        assert len({reference['processed_at'] for reference in references}) == len(references)
        assert {row[0] for row in conn.execute("SELECT created_at FROM journal_entries")} == {stamp}
    finally:
        conn.close()


@pytest.mark.parametrize("second_clock", ["2030-01-01T00:00:00Z", "2029-01-01T00:00:00Z"])
def test_rebuild_marker_uses_persisted_state_with_a_stale_profile(tmp_path, second_clock):
    conn = _book(tmp_path / "data")
    try:
        profile = _profile(conn)
        state = custody_journal.build_ledger_state(conn, profile)
        custody_journal.store_ledger_state(conn, profile, state, created_at="2030-01-01T00:00:00Z")
        result = custody_journal.store_ledger_state(conn, profile, state, created_at=second_clock)
        assert result["processed_at"] == "2030-01-01T00:00:00.000001Z"
        assert _profile(conn)["last_processed_at"] == result["processed_at"]
    finally:
        conn.close()



@pytest.mark.parametrize("encrypted", [False, True])
def test_yearly_realized_sum_has_explicit_order_independent_of_scan_direction(tmp_path, encrypted):
    from kassiber.core.ui_snapshot import _fiat_snapshot

    conn = _book(tmp_path / "data", passphrase="synthetic YTD test" if encrypted else None)
    try:
        # Even SQLite's compensated SUM can depend on row order. Use future
        # dates so this covers the real SQLite clock, not only a patched Python
        # clock. The result must use journal order for every physical scan.
        conn.execute("UPDATE journal_entries SET gain_loss=NULL")
        ids = [row[0] for row in conn.execute("SELECT id FROM journal_entries ORDER BY id LIMIT 5")]
        assert len(ids) == 5
        for day, (entry_id, gain) in enumerate(zip(ids, (1e16, 1e-16, 1., -1e16, -1.)), 1):
            conn.execute("UPDATE journal_entries SET occurred_at=?,gain_loss=? WHERE id=?",
                         (f"2099-01-{day:02d}T00:00:00Z", gain, entry_id))
        conn.execute("PRAGMA reverse_unordered_selects=OFF")
        forward = _fiat_snapshot(conn, "p", "EUR", 0, {})["eurRealizedYTD"]
        conn.execute("PRAGMA reverse_unordered_selects=ON")
        assert _fiat_snapshot(conn, "p", "EUR", 0, {})["eurRealizedYTD"] == forward
    finally:
        conn.close()



def test_quarantine_freshness_compares_fractional_processing_markers_as_instants(tmp_path):
    from kassiber.core import freshness, quarantine_review

    conn = _book(tmp_path / "data")
    try:
        state = custody_journal.build_ledger_state(conn, _profile(conn))
        for _ in range(2):
            _store_at(conn, state, "2030-01-01T00:00:00Z")
        assert _profile(conn)["last_processed_at"] == "2030-01-01T00:00:00.000001Z"
        for stamp, expected in (("2030-01-01T00:00:00Z", None),
                                ("2030-01-01T00:00:01Z", "synthetic_failure")):
            freshness.upsert_source_state(
                conn, profile_id="p", source_key=freshness.journal_source_key("p"),
                source_type="journals", source_label="Journals", status="error",
                last_success_at="2029-01-01T00:00:00Z", last_error_at=stamp,
                last_error_code="synthetic_failure", last_error_message="Synthetic failure",
            )
            error = quarantine_review._journal_freshness(conn, _profile(conn))["last_error"]
            assert (error["code"] if error else None) == expected
    finally:
        conn.close()
def _cache_settings(conn):
    return tuple(conn.execute(f"PRAGMA {name}").fetchone()[0]
                 for name in ("cache_size", "journal_mode", "synchronous"))


@pytest.mark.parametrize("encrypted", [False, True])
@pytest.mark.parametrize("fail", [False, True])
def test_process_scopes_the_write_cache_and_restores_exact_driver_settings(tmp_path, encrypted, fail):
    conn = _book(tmp_path / 'data', passphrase='synthetic cache test' if encrypted else None)
    try:
        conn.execute('PRAGMA cache_size = 73')
        expected = _cache_settings(conn)
        entries = _stored_rows(conn)
        profile = tuple(_profile(conn))
        observed = []
        def progress(update):
            observed.append(_cache_settings(conn))
            if fail and update['phase'] == 'storing':
                raise RuntimeError('injected cache scope failure')
        def process():
            return custody_journal.process_journals(
                conn, 'W', 'P', repair_source_overlaps=lambda *_: None,
                source_overlap_warning=lambda *_: None, auto_price=lambda *_: 0,
                progress_observer=progress,
            )
        if fail:
            with pytest.raises(RuntimeError, match='cache scope failure'):
                process()
            assert tuple(_profile(conn)) == profile
            assert _stored_rows(conn) == entries
        else:
            assert process()['entries_created'] == len(entries)
        assert observed and all(settings == (-131072, *expected[1:]) for settings in observed)
        assert _cache_settings(conn) == expected
        assert not conn.in_transaction
    finally:
        conn.close()


def test_nested_journal_cache_restores_each_scope_after_an_exception(tmp_path):
    from kassiber.core.journal_storage import journal_write_cache

    conn = open_db(tmp_path / 'data')
    try:
        conn.execute('PRAGMA cache_size = -64000')
        conn.execute('PRAGMA cache_spill = OFF')
        expected = _cache_settings(conn)
        with pytest.raises(RuntimeError, match='outer'):
            with journal_write_cache(conn):
                assert _cache_settings(conn) == (-131072, *expected[1:])
                assert conn.execute('PRAGMA cache_spill').fetchone()[0] == 0
                with pytest.raises(RuntimeError, match='inner'):
                    with journal_write_cache(conn):
                        raise RuntimeError('inner')
                assert _cache_settings(conn) == (-131072, *expected[1:])
                raise RuntimeError('outer')
        assert _cache_settings(conn) == expected
        assert conn.execute('PRAGMA cache_spill').fetchone()[0] == 0
        assert not conn.in_transaction
    finally:
        conn.close()


@pytest.mark.parametrize("encrypted", [False, True])
@pytest.mark.parametrize("fail", [False, True])
def test_review_cache_preserves_caller_transaction_and_commit_false(tmp_path, monkeypatch, encrypted, fail):
    from kassiber.cli.handlers import _metadata_hooks
    from kassiber.core import review_workflow as review
    from tests.test_review_workflow import ReviewWorkflowTest

    conn = open_db(tmp_path / 'data', passphrase='synthetic review cache' if encrypted else None)
    try:
        ReviewWorkflowTest.seed(conn)
        profile = _profile(conn)
        hooks = review.ReviewHooks(metadata=_metadata_hooks())
        artifact = review.plan_review(
            conn, profile,
            operations=[{'type':'price_override','transaction_id':'a','fiat_rate':'20000','reason':'Synthetic receipt'}],
            expected_input_version=0, hooks=hooks,
        )
        conn.execute('PRAGMA cache_size = 97')
        expected = _cache_settings(conn)
        original = custody_journal.store_ledger_state
        calls = []
        def store(*args, **kwargs):
            calls.append(_cache_settings(conn))
            if fail:
                raise RuntimeError('injected review cache failure')
            return original(*args, **kwargs)
        monkeypatch.setattr(custody_journal, 'store_ledger_state', store)
        conn.execute('BEGIN')
        conn.execute("UPDATE transactions SET note='caller-owned' WHERE id='a'")
        def apply():
            return review.apply_review(conn, profile, artifact=artifact, idempotency_key='cache', hooks=hooks, commit=False)
        if fail:
            with pytest.raises(RuntimeError, match='review cache failure'):
                apply()
            assert conn.execute('SELECT count(*) FROM review_workflow_receipts').fetchone()[0] == 0
            assert conn.execute('SELECT count(*) FROM transaction_edit_events').fetchone()[0] == 0
        else:
            assert apply()['status'] == 'verified'
            assert conn.execute('SELECT count(*) FROM review_workflow_receipts').fetchone()[0] == 1
        # The before and after tax previews each store inside a rolled-back
        # savepoint, then the verified state is stored; a failure stops at the
        # first preview. Every store runs with the scoped cache.
        assert calls == [(-131072, *expected[1:])] * (1 if fail else 3)
        assert _cache_settings(conn) == expected
        assert conn.in_transaction
        assert conn.execute("SELECT note FROM transactions WHERE id='a'").fetchone()[0] == 'caller-owned'
        conn.rollback()
        assert conn.execute('SELECT count(*) FROM review_workflow_receipts').fetchone()[0] == 0
        assert conn.execute('SELECT count(*) FROM transaction_edit_events').fetchone()[0] == 0
        assert conn.execute('SELECT count(*) FROM journal_entries').fetchone()[0] == 0
        assert conn.execute("SELECT note FROM transactions WHERE id='a'").fetchone()[0] is None
    finally:
        conn.close()
