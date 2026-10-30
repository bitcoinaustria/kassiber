"""Journal storage format: compact retained calculations.

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


def _book(root, method="fifo", country="generic"):
    conn = open_db(root)
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
