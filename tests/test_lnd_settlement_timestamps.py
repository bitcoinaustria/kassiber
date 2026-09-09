"""Native LND settlement dates survive import, refresh and journal projection."""
from types import SimpleNamespace

import pytest

from kassiber.backends import get_db_backend
from kassiber.cli.handlers import _import_coordinator_hooks, _metadata_hooks, process_journals
from kassiber.core import accounts, wallets
from kassiber.core.lightning import lnd
from kassiber.core.metadata import update_transaction_metadata
from kassiber.core.repo import fetch_wallet_with_account
from kassiber.db import open_db

CREATED = 1767225599
SETTLED = 1767225601
CREATED_ISO = "2025-12-31T23:59:59Z"
SETTLED_ISO = "2026-01-01T00:00:01Z"


def payment():
    return {
        "payment_hash": "22" * 32, "payment_index": "1", "status": "SUCCEEDED",
        "value_msat": "100000000", "fee_msat": "10000",
        "creation_date": str(CREATED), "creation_time_ns": str(CREATED * 10**9),
        "htlcs": [{"attempt_id": "1", "status": "SUCCEEDED",
                   "attempt_time_ns": str(CREATED * 10**9),
                   "resolve_time_ns": str(SETTLED * 10**9)}],
    }


@pytest.fixture
def book(tmp_path):
    conn = open_db(tmp_path)
    workspace = accounts.create_workspace(conn, "Personal")
    profile = accounts.create_profile(conn, workspace["id"], "Main", "USD", "FIFO", "generic", 365)
    accounts.create_backend(conn, "fixture", "lnd", "https://example.invalid", token="synthetic")
    wallet_id = wallets.create_wallet(conn, workspace["id"], profile["id"], "Node", "lnd", config={"backend": "fixture"})["id"]
    state = SimpleNamespace(payment=None)

    class Client:
        def get(self, path, params=None):
            offset = int((params or {}).get("index_offset", 0))
            if path == "/v1/getinfo":
                return {"chains": [{"chain": "bitcoin", "network": "mainnet"}]}
            if path == "/v1/invoices":
                return {"invoices": [] if offset else [{
                    "r_hash": "11" * 32, "add_index": "1", "state": "SETTLED",
                    "amt_paid_msat": "1000000000", "creation_date": str(CREATED - 60),
                    "settle_date": str(CREATED - 30),
                }], "last_index_offset": "1"}
            if path == "/v1/payments":
                # ListPayments(include_incomplete=false) omits unresolved payments.
                rows = [state.payment] if state.payment and not offset else []
                return {"payments": rows, "last_index_offset": "1"}
            if path in ("/v1/channels", "/v1/channels/closed"):
                return {"channels": []}
            raise AssertionError(path)

        def post(self, path, payload):
            assert path == "/v1/switch"
            return {"forwarding_events": [], "last_offset_index": "0"}

    def sync():
        result = lnd.sync_lnd_wallet(
            conn, profile, fetch_wallet_with_account(conn, wallet_id),
            get_db_backend(conn, "fixture"), _import_coordinator_hooks(), client=Client(),
        )
        assert result["status"] == "synced"
        return result

    state.conn, state.sync = conn, sync
    try:
        yield state
    finally:
        conn.close()


def pay_row(book):
    return dict(book.conn.execute("SELECT * FROM transactions WHERE kind='lnd_pay'").fetchone())


def test_successful_settlement_drives_journal_date(book):
    book.sync()
    assert book.conn.execute("SELECT COUNT(*) FROM transactions WHERE kind='lnd_pay'").fetchone()[0] == 0
    book.payment = payment()
    book.sync()
    # Local fixture prices isolate chronology from price-coverage policy.
    book.conn.execute("UPDATE transactions SET fiat_rate=40000")
    book.conn.commit()
    process_journals(book.conn, "Personal", "Main")
    disposal = book.conn.execute("SELECT occurred_at,quantity FROM journal_entries WHERE entry_type='disposal'").fetchone()
    assert tuple(disposal) == (SETTLED_ISO, -100010000)
    original = pay_row(book)
    book.sync()
    assert pay_row(book) == original


@pytest.mark.parametrize("status", ["FAILED", "IN_FLIGHT", "UNKNOWN"])
def test_unsettled_payments_never_book(book, status):
    book.payment = {**payment(), "status": status}
    book.sync()
    assert book.conn.execute("SELECT COUNT(*) FROM transactions WHERE kind='lnd_pay'").fetchone()[0] == 0


def test_multipart_uses_last_success_not_failed_attempt(book):
    book.payment = payment()
    book.payment["htlcs"] += [
        {"status": "SUCCEEDED", "resolve_time_ns": str((SETTLED - 1) * 10**9)},
        {"status": "FAILED", "resolve_time_ns": str((SETTLED + 30) * 10**9)},
        {"status": "IN_FLIGHT", "resolve_time_ns": "0"},
    ]
    book.sync()
    assert pay_row(book)["occurred_at"] == SETTLED_ISO


@pytest.mark.parametrize("missing", [None, "0", "garbage", True, 1.5, str(2**63), "9" * 5000])
def test_missing_or_invalid_success_timing_retains_legacy_fallback(book, missing):
    book.payment = payment()
    book.payment["htlcs"].append({"status": "SUCCEEDED", "resolve_time_ns": missing})
    book.sync()
    assert pay_row(book)["occurred_at"] == CREATED_ISO


def test_seconds_granularity_unchanged(book):
    book.payment = payment()
    book.payment["htlcs"][0]["resolve_time_ns"] = str(SETTLED * 10**9 + 999_999_999)
    book.sync()
    assert pay_row(book)["confirmed_at"] == SETTLED_ISO


def test_native_refresh_preserves_authored_metadata_and_price(book):
    book.payment = payment()
    book.payment.pop("htlcs")
    book.sync()
    original = pay_row(book)
    assert original["confirmed_at"] == CREATED_ISO
    update_transaction_metadata(
        book.conn, "Personal", "Main", original["id"], _metadata_hooks(),
        note="Invoice reviewed", note_set=True, tags=["reviewed"], excluded=True,
        kind="sell", kind_set=True,
        pricing_update={"fiat_rate": "41000", "source_kind": "manual_override", "quality": "exact"},
    )
    authored = pay_row(book)
    history = list(book.conn.execute("SELECT * FROM transaction_edit_events"))
    tags = list(book.conn.execute("SELECT * FROM transaction_tags"))
    book.payment = payment()
    book.sync()
    refreshed = pay_row(book)
    assert refreshed["id"] == original["id"]
    assert refreshed["occurred_at"] == refreshed["confirmed_at"] == SETTLED_ISO
    for field in ("note", "excluded", "kind_override", "fiat_rate_exact", "fiat_value_exact", "pricing_source_kind", "pricing_timestamp", "pricing_fetched_at"):
        assert refreshed[field] == authored[field]
    assert list(book.conn.execute("SELECT * FROM transaction_edit_events")) == history
    assert list(book.conn.execute("SELECT * FROM transaction_tags")) == tags
    # A subsequent older response cannot turn settlement back into creation.
    book.payment.pop("htlcs")
    book.sync()
    assert pay_row(book) == refreshed


@pytest.mark.parametrize("source_kind,legacy_source", [
    (None, "rates_cache"), ("fmv_provider", "rates_cache"),
    ("manual_rate_cache", "rates_cache"), ("fmv_provider", None),
])
def test_date_correction_discards_only_cached_time_price(book, source_kind, legacy_source):
    book.payment = payment()
    book.payment.pop("htlcs")
    book.sync()
    # Simulate a previous app version's timestamp-priced stored observation.
    book.conn.execute("""UPDATE transactions SET fiat_rate=40000, fiat_rate_exact='40000',
        fiat_value=40, fiat_value_exact='40', fiat_price_source=?,
        pricing_source_kind=?, pricing_timestamp=?, pricing_provider='local-fixture'
        WHERE kind='lnd_pay'""", (legacy_source, source_kind, CREATED_ISO))
    book.conn.commit()
    book.payment = payment()
    book.sync()
    from kassiber.core.imports import PRICE_COLUMNS
    row = pay_row(book)
    assert all(row[column] is None for column in PRICE_COLUMNS)
    assert row["occurred_at"] == SETTLED_ISO


def test_generic_import_cannot_claim_live_settlement_authority(book):
    from kassiber.core.imports import insert_wallet_records

    book.payment = payment()
    book.payment.pop("htlcs")
    book.sync()
    original = pay_row(book)
    record = lnd._lnd_payment_import(payment())
    record["authoritative_settlement"] = True
    record["authoritative_settlement_ids"] = [record["id"]]
    wallet = fetch_wallet_with_account(book.conn, original["wallet_id"])
    profile = book.conn.execute("SELECT * FROM profiles WHERE id=?", (wallet["profile_id"],)).fetchone()
    insert_wallet_records(book.conn, profile, wallet, [record], "generic", _import_coordinator_hooks())
    assert pay_row(book)["confirmed_at"] == CREATED_ISO


def test_settlement_evidence_does_not_persist_payment_secrets_or_routes(book):
    book.payment = payment()
    book.payment.update(payment_preimage="PREIMAGE_SECRET", payment_request="BOLT11_SECRET")
    book.payment["htlcs"][0].update(
        preimage="HTLC_SECRET", route={"hops": [{"pub_key": "ROUTE_SECRET"}]},
        failure={"failure_source_pubkey": "FAILURE_SECRET"},
    )
    book.sync()
    persisted = str(pay_row(book))
    assert "SECRET" not in persisted
    assert "resolve_time_ns" not in persisted


def test_existing_date_correction_invalidates_and_rebuilds_journal(book):
    book.payment = payment()
    book.payment.pop("htlcs")
    book.sync()
    book.conn.execute("UPDATE transactions SET fiat_rate=40000")
    book.conn.commit()
    process_journals(book.conn, "Personal", "Main")
    assert book.conn.execute("SELECT occurred_at FROM journal_entries WHERE entry_type='disposal'").fetchone()[0] == CREATED_ISO
    before = book.conn.execute("SELECT journal_input_version FROM profiles").fetchone()[0]
    original_id = pay_row(book)["id"]
    book.payment = payment()
    book.sync()
    assert book.conn.execute("SELECT journal_input_version FROM profiles").fetchone()[0] > before
    assert pay_row(book)["id"] == original_id
    process_journals(book.conn, "Personal", "Main")
    disposal = book.conn.execute("SELECT occurred_at,quantity FROM journal_entries WHERE entry_type='disposal'").fetchone()
    assert tuple(disposal) == (SETTLED_ISO, -100010000)


@pytest.mark.parametrize("field,value", [
    ("payment_hash", "33" * 32),
    ("payment_hash_source", "generic"),
    ("raw_json", '{"chain":"lightning","network":"regtest"}'),
    ("raw_json", '{}'),
])
def test_conflicting_native_identity_cannot_authorize_date_correction(book, field, value):
    book.payment = payment()
    book.payment.pop("htlcs")
    book.sync()
    # Existing imported evidence conflicts with the live exact-payment claim.
    book.conn.execute(f"UPDATE transactions SET {field}=? WHERE kind='lnd_pay'", (value,))
    book.conn.commit()
    book.payment = payment()
    book.sync()
    assert pay_row(book)["occurred_at"] == CREATED_ISO


@pytest.mark.parametrize("attempts", [None, 42, "malformed", {}])
def test_malformed_attempt_collection_keeps_legacy_fallback(book, attempts):
    book.payment = payment()
    book.payment["htlcs"] = attempts
    book.sync()
    assert pay_row(book)["confirmed_at"] == CREATED_ISO
