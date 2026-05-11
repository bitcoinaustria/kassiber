"""Importer + insert + backfill pinning for ``transactions.payment_hash``.

Covers four landings:

1. ``normalize_phoenix_record`` exposes ``payment_hash`` /
   ``payment_hash_source`` for Lightning rows and elides them when the
   CSV's column is empty or malformed.
2. ``normalize_import_record`` round-trips the field for any importer
   that produces it.
3. ``insert_wallet_records`` actually persists the column on a fresh
   row (full DB round-trip via ``open_db``).
4. ``ensure_schema_compat`` backfills the column on legacy rows whose
   ``raw_json`` still carries the original CSV ``payment_hash`` blob.
"""

import json
import sqlite3
import tempfile
import unittest
import uuid

from kassiber.core.imports import (
    ImportCoordinatorHooks,
    insert_wallet_records,
    normalize_import_record,
)
from kassiber.db import ensure_schema_compat, open_db
from kassiber.importers import normalize_phoenix_record


def _now():
    return "2026-01-01T00:00:00Z"


def _seed_minimal_scope(conn):
    workspace_id = str(uuid.uuid4())
    profile_id = str(uuid.uuid4())
    wallet_id = str(uuid.uuid4())
    conn.execute(
        "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
        (workspace_id, f"ws-{workspace_id[:8]}", _now()),
    )
    conn.execute(
        """
        INSERT INTO profiles(id, workspace_id, label, fiat_currency, tax_country,
                             tax_long_term_days, gains_algorithm, journal_input_version,
                             last_processed_input_version, last_processed_tx_count, created_at)
        VALUES(?, ?, ?, 'EUR', 'at', 365, 'FIFO', 0, 0, 0, ?)
        """,
        (profile_id, workspace_id, "main", _now()),
    )
    conn.execute(
        "INSERT INTO wallets(id, workspace_id, profile_id, label, kind, config_json, created_at) "
        "VALUES(?, ?, ?, ?, 'phoenix', '{}', ?)",
        (wallet_id, workspace_id, profile_id, "phoenix-mobile", _now()),
    )
    return workspace_id, profile_id, wallet_id


def _fetch_profile_and_wallet(conn, profile_id, wallet_id):
    profile = conn.execute("SELECT * FROM profiles WHERE id = ?", (profile_id,)).fetchone()
    wallet = conn.execute("SELECT * FROM wallets WHERE id = ?", (wallet_id,)).fetchone()
    return profile, wallet


_HASH_OK = "ab" * 32  # 64 lowercase hex chars
_HASH_MIXED_CASE = "AB" * 32
_HASH_INVALID = "zz" * 32


def _ensure_tag_row(conn, workspace_id, profile_id, code, label):
    row = conn.execute(
        "SELECT * FROM tags WHERE profile_id = ? AND code = ?",
        (profile_id, code),
    ).fetchone()
    if row:
        return row, False
    conn.execute(
        "INSERT INTO tags(id, workspace_id, profile_id, code, label, created_at) "
        "VALUES(?, ?, ?, ?, ?, ?)",
        (str(uuid.uuid4()), workspace_id, profile_id, code, label, _now()),
    )
    row = conn.execute(
        "SELECT * FROM tags WHERE profile_id = ? AND code = ?",
        (profile_id, code),
    ).fetchone()
    return row, True


def _invalidate_journals(conn, profile_id):
    conn.execute(
        "UPDATE profiles SET journal_input_version = journal_input_version + 1 WHERE id = ?",
        (profile_id,),
    )


_HOOKS = ImportCoordinatorHooks(
    ensure_tag_row=_ensure_tag_row,
    invalidate_journals=_invalidate_journals,
)


class PhoenixNormalizerTests(unittest.TestCase):
    def _row(self, payment_hash=""):
        return {
            "date": "2026-03-14T17:30:00Z",
            "id": "11111111-aaaa-bbbb-cccc-000000000001",
            "type": "lightning_received",
            "amount_msat": "3000000",
            "amount_fiat": "1.20 USD",
            "fee_credit_msat": "0",
            "mining_fee_sat": "0",
            "mining_fee_fiat": "0 USD",
            "service_fee_msat": "0",
            "service_fee_fiat": "0 USD",
            "payment_hash": payment_hash,
            "tx_id": "",
            "destination": "",
            "description": "",
        }

    def test_lowercase_hex_payment_hash_promoted(self):
        normalized = normalize_phoenix_record(self._row(payment_hash=_HASH_OK))
        self.assertEqual(normalized["payment_hash"], _HASH_OK)
        self.assertEqual(normalized["payment_hash_source"], "importer")

    def test_mixed_case_payment_hash_normalized_to_lowercase(self):
        normalized = normalize_phoenix_record(self._row(payment_hash=_HASH_MIXED_CASE))
        self.assertEqual(normalized["payment_hash"], _HASH_OK)
        self.assertEqual(normalized["payment_hash_source"], "importer")

    def test_invalid_hex_dropped(self):
        normalized = normalize_phoenix_record(self._row(payment_hash=_HASH_INVALID))
        self.assertIsNone(normalized["payment_hash"])
        self.assertIsNone(normalized["payment_hash_source"])

    def test_short_payment_hash_dropped(self):
        normalized = normalize_phoenix_record(self._row(payment_hash="ab" * 16))
        self.assertIsNone(normalized["payment_hash"])

    def test_missing_payment_hash_is_none(self):
        normalized = normalize_phoenix_record(self._row(payment_hash=""))
        self.assertIsNone(normalized["payment_hash"])
        self.assertIsNone(normalized["payment_hash_source"])

    def test_dead_onchain_txid_field_removed(self):
        normalized = normalize_phoenix_record(self._row(payment_hash=_HASH_OK))
        self.assertNotIn("_phoenix_onchain_txid", normalized)


class NormalizeImportRecordTests(unittest.TestCase):
    def _record(self, **overrides):
        record = {
            "txid": "x",
            "occurred_at": "2026-03-14T17:30:00Z",
            "direction": "inbound",
            "asset": "BTC",
            "amount": "0.0001",
            "fee": "0",
            "payment_hash": _HASH_OK,
            "payment_hash_source": "importer",
        }
        record.update(overrides)
        return record

    def test_round_trips_payment_hash(self):
        normalized = normalize_import_record(self._record())
        self.assertEqual(normalized["payment_hash"], _HASH_OK)
        self.assertEqual(normalized["payment_hash_source"], "importer")

    def test_drops_invalid_hash(self):
        normalized = normalize_import_record(self._record(payment_hash="not-hex"))
        self.assertIsNone(normalized["payment_hash"])
        self.assertIsNone(normalized["payment_hash_source"])

    def test_drops_source_when_hash_missing(self):
        normalized = normalize_import_record(self._record(payment_hash=None))
        self.assertIsNone(normalized["payment_hash"])
        self.assertIsNone(normalized["payment_hash_source"])


class InsertPersistsPaymentHashTests(unittest.TestCase):
    def test_phoenix_import_writes_column(self):
        with tempfile.TemporaryDirectory() as data_root:
            conn = open_db(data_root)
            try:
                _, profile_id, wallet_id = _seed_minimal_scope(conn)
                profile, wallet = _fetch_profile_and_wallet(conn, profile_id, wallet_id)
                phoenix_row = {
                    "date": "2026-03-14T17:30:00Z",
                    "id": "11111111-aaaa-bbbb-cccc-000000000001",
                    "type": "lightning_received",
                    "amount_msat": "3000000",
                    "amount_fiat": "1.20 USD",
                    "fee_credit_msat": "0",
                    "mining_fee_sat": "0",
                    "mining_fee_fiat": "0 USD",
                    "service_fee_msat": "0",
                    "service_fee_fiat": "0 USD",
                    "payment_hash": _HASH_OK,
                    "tx_id": "",
                    "destination": "",
                    "description": "ln receive",
                }
                normalized = normalize_phoenix_record(phoenix_row)
                insert_wallet_records(
                    conn,
                    profile,
                    wallet,
                    [normalized],
                    source_label="phoenix_csv",
                    hooks=_HOOKS,
                )
                row = conn.execute(
                    "SELECT payment_hash, payment_hash_source FROM transactions WHERE wallet_id = ?",
                    (wallet_id,),
                ).fetchone()
                self.assertEqual(row["payment_hash"], _HASH_OK)
                self.assertEqual(row["payment_hash_source"], "importer")
            finally:
                conn.close()

    def test_repeat_import_backfills_payment_hash_when_absent(self):
        with tempfile.TemporaryDirectory() as data_root:
            conn = open_db(data_root)
            try:
                _, profile_id, wallet_id = _seed_minimal_scope(conn)
                profile, wallet = _fetch_profile_and_wallet(conn, profile_id, wallet_id)
                base_row = {
                    "date": "2026-03-14T17:30:00Z",
                    "id": "22222222-aaaa-bbbb-cccc-000000000002",
                    "type": "lightning_sent",
                    "amount_msat": "-5000000",
                    "amount_fiat": "-2.00 USD",
                    "fee_credit_msat": "0",
                    "mining_fee_sat": "0",
                    "mining_fee_fiat": "0 USD",
                    "service_fee_msat": "0",
                    "service_fee_fiat": "0 USD",
                    "payment_hash": "",
                    "tx_id": "",
                    "destination": "node",
                    "description": "",
                }
                without_hash = normalize_phoenix_record(base_row)
                insert_wallet_records(
                    conn, profile, wallet, [without_hash], source_label="phoenix_csv", hooks=_HOOKS
                )
                row = conn.execute(
                    "SELECT payment_hash FROM transactions WHERE wallet_id = ?",
                    (wallet_id,),
                ).fetchone()
                self.assertIsNone(row["payment_hash"])

                with_hash = normalize_phoenix_record({**base_row, "payment_hash": _HASH_OK})
                insert_wallet_records(
                    conn, profile, wallet, [with_hash], source_label="phoenix_csv", hooks=_HOOKS
                )
                row = conn.execute(
                    "SELECT payment_hash, payment_hash_source FROM transactions WHERE wallet_id = ?",
                    (wallet_id,),
                ).fetchone()
                self.assertEqual(row["payment_hash"], _HASH_OK)
                self.assertEqual(row["payment_hash_source"], "importer")
            finally:
                conn.close()


class BackfillFromRawJsonTests(unittest.TestCase):
    """Pre-feature rows that still carry the original CSV payload should
    surface ``payment_hash`` once ``ensure_schema_compat`` runs over them.
    """

    def _insert_legacy_row(self, conn, workspace_id, profile_id, wallet_id, *, tx_id, raw_json):
        conn.execute(
            """
            INSERT INTO transactions(id, workspace_id, profile_id, wallet_id, fingerprint,
                occurred_at, direction, asset, amount, fee, raw_json, created_at)
            VALUES(?, ?, ?, ?, ?, ?, 'inbound', 'BTC', 1000, 0, ?, ?)
            """,
            (
                tx_id,
                workspace_id,
                profile_id,
                wallet_id,
                f"fp-{tx_id}",
                _now(),
                raw_json,
                _now(),
            ),
        )

    def test_valid_payment_hash_promoted_from_raw_json(self):
        with tempfile.TemporaryDirectory() as data_root:
            conn = open_db(data_root)
            try:
                workspace_id, profile_id, wallet_id = _seed_minimal_scope(conn)
                self._insert_legacy_row(
                    conn,
                    workspace_id,
                    profile_id,
                    wallet_id,
                    tx_id="legacy-1",
                    raw_json=json.dumps({"payment_hash": _HASH_OK, "noise": "ok"}),
                )
                conn.execute("UPDATE transactions SET payment_hash = NULL WHERE id = 'legacy-1'")
                conn.commit()

                ensure_schema_compat(conn)

                row = conn.execute(
                    "SELECT payment_hash, payment_hash_source FROM transactions WHERE id = 'legacy-1'"
                ).fetchone()
                self.assertEqual(row["payment_hash"], _HASH_OK)
                self.assertEqual(row["payment_hash_source"], "importer_backfill")
            finally:
                conn.close()

    def test_invalid_or_missing_raw_json_value_skipped(self):
        with tempfile.TemporaryDirectory() as data_root:
            conn = open_db(data_root)
            try:
                workspace_id, profile_id, wallet_id = _seed_minimal_scope(conn)
                for label, raw in [
                    ("no-key", "{}"),
                    ("short", json.dumps({"payment_hash": "ab" * 10})),
                    ("non-hex", json.dumps({"payment_hash": "zz" * 32})),
                    ("not-string", json.dumps({"payment_hash": 42})),
                    ("broken-json", "{not json"),
                ]:
                    self._insert_legacy_row(
                        conn,
                        workspace_id,
                        profile_id,
                        wallet_id,
                        tx_id=f"legacy-{label}",
                        raw_json=raw,
                    )
                conn.execute("UPDATE transactions SET payment_hash = NULL")
                conn.commit()

                ensure_schema_compat(conn)

                rows = conn.execute(
                    "SELECT payment_hash FROM transactions WHERE payment_hash IS NOT NULL"
                ).fetchall()
                self.assertEqual(len(rows), 0)
            finally:
                conn.close()

    def test_does_not_overwrite_existing_payment_hash(self):
        with tempfile.TemporaryDirectory() as data_root:
            conn = open_db(data_root)
            try:
                workspace_id, profile_id, wallet_id = _seed_minimal_scope(conn)
                self._insert_legacy_row(
                    conn,
                    workspace_id,
                    profile_id,
                    wallet_id,
                    tx_id="legacy-keep",
                    raw_json=json.dumps({"payment_hash": _HASH_OK}),
                )
                # Pre-set a different value so we can assert it's preserved.
                preserved = "cd" * 32
                conn.execute(
                    "UPDATE transactions SET payment_hash = ?, payment_hash_source = 'importer' "
                    "WHERE id = 'legacy-keep'",
                    (preserved,),
                )
                conn.commit()

                ensure_schema_compat(conn)

                row = conn.execute(
                    "SELECT payment_hash, payment_hash_source FROM transactions WHERE id = 'legacy-keep'"
                ).fetchone()
                self.assertEqual(row["payment_hash"], preserved)
                self.assertEqual(row["payment_hash_source"], "importer")
            finally:
                conn.close()


if __name__ == "__main__":
    unittest.main()
