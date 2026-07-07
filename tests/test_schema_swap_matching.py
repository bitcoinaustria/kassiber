"""Schema migrations for the swap-matching feature.

Pins the additive schema deltas introduced for the swap-candidate matcher:

* ``transactions`` gains ``payment_hash`` + ``payment_hash_source`` plus a
  partial index keyed on ``payment_hash``.
* ``transaction_pairs`` gains ``swap_fee_msat``, ``swap_fee_kind``,
  ``confidence_at_pair``, ``pair_source``, ``deleted_at``; the legacy
  table-level per-leg ``UNIQUE`` constraints are rebuilt into a partial
  exact-pair unique index scoped to ``deleted_at IS NULL`` so reviewed
  same-asset privacy links can reuse a leg while exact active duplicates
  remain blocked.
* Three new tables land alongside: ``transaction_pair_dismissals``,
  ``swap_matching_rules``, ``saved_views``.

Covers both fresh databases (``CREATE TABLE IF NOT EXISTS`` path) and
pre-feature databases that still carry the legacy table-level UNIQUE
constraints (the rebuild path through
``_migrate_legacy_transaction_pairs_uniques``).
"""

import sqlite3
import tempfile
import unittest
import uuid

from kassiber.cli.handlers import (
    create_transaction_pair,
    dismiss_transfer_candidate,
    update_transaction_pair,
)
from kassiber.core.reports import _swap_fee_summary_rows
from kassiber.db import ensure_schema_compat, open_db


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
        "VALUES(?, ?, ?, ?, 'descriptor', '{}', ?)",
        (wallet_id, workspace_id, profile_id, "test-wallet", _now()),
    )
    return workspace_id, profile_id, wallet_id


def _insert_tx(conn, *, tx_id, workspace_id, profile_id, wallet_id, asset, direction, amount_msat=1000):
    conn.execute(
        """
        INSERT INTO transactions(id, workspace_id, profile_id, wallet_id, fingerprint,
                                 occurred_at, direction, asset, amount, fee, created_at)
        VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?)
        """,
        (
            tx_id,
            workspace_id,
            profile_id,
            wallet_id,
            f"fp-{tx_id}",
            _now(),
            direction,
            asset,
            amount_msat,
            _now(),
        ),
    )


class FreshSchemaTests(unittest.TestCase):
    def test_open_db_creates_new_tables_and_columns(self):
        with tempfile.TemporaryDirectory() as data_root:
            conn = open_db(data_root)
            try:
                tables = {
                    row["name"]
                    for row in conn.execute(
                        "SELECT name FROM sqlite_master WHERE type='table'"
                    ).fetchall()
                }
                self.assertIn("transaction_pair_dismissals", tables)
                self.assertIn("swap_matching_rules", tables)
                self.assertIn("saved_views", tables)
                # loan_legs is the minimal loan-mark store; the facility tables
                # were removed when loans collapsed to a per-tx mark.
                self.assertIn("loan_legs", tables)
                self.assertNotIn("loans", tables)
                self.assertNotIn("loan_escrow_positions", tables)

                leg_cols = {row["name"] for row in conn.execute("PRAGMA table_info(loan_legs)").fetchall()}
                for name in ("transaction_id", "loan_id", "role"):
                    self.assertIn(name, leg_cols)
                for gone in ("on_chain_present", "escrow_address", "amount", "policy"):
                    self.assertNotIn(gone, leg_cols)

                tx_cols = {row["name"] for row in conn.execute("PRAGMA table_info(transactions)").fetchall()}
                self.assertIn("payment_hash", tx_cols)
                self.assertIn("payment_hash_source", tx_cols)

                pair_cols = {row["name"] for row in conn.execute("PRAGMA table_info(transaction_pairs)").fetchall()}
                for name in (
                    "swap_fee_msat",
                    "swap_fee_kind",
                    "confidence_at_pair",
                    "pair_source",
                    "deleted_at",
                    "out_amount",
                ):
                    self.assertIn(name, pair_cols)

                payout_cols = {
                    row["name"]
                    for row in conn.execute("PRAGMA table_info(direct_swap_payouts)").fetchall()
                }
                self.assertIn("out_amount", payout_cols)
            finally:
                conn.close()

    def test_partial_unique_indexes_replace_table_level_constraints(self):
        with tempfile.TemporaryDirectory() as data_root:
            conn = open_db(data_root)
            try:
                index_names = {
                    row["name"]
                    for row in conn.execute(
                        "SELECT name FROM sqlite_master WHERE type='index'"
                    ).fetchall()
                }
                self.assertIn("idx_transaction_pairs_active_out", index_names)
                self.assertIn("idx_transaction_pairs_active_in", index_names)
                self.assertIn("idx_transaction_pairs_active_pair", index_names)
                self.assertIn("idx_transaction_pairs_profile_active", index_names)
                self.assertIn("idx_transactions_payment_hash", index_names)
                # One active loan mark per transaction is a partial unique
                # index; pin it so a future drop is caught (the tax pipeline relies
                # on it for the loan-role lookup).
                self.assertIn("idx_loan_legs_active_transaction", index_names)
                self.assertIn("idx_loan_legs_profile_active", index_names)

                table_sql = conn.execute(
                    "SELECT sql FROM sqlite_master WHERE type='table' AND name='transaction_pairs'"
                ).fetchone()["sql"]
                self.assertNotIn("UNIQUE (profile_id, out_transaction_id)", table_sql)
                self.assertNotIn("UNIQUE (profile_id, in_transaction_id)", table_sql)
            finally:
                conn.close()

    def test_reused_leg_schema_allows_many_links_but_blocks_exact_duplicates(self):
        with tempfile.TemporaryDirectory() as data_root:
            conn = open_db(data_root)
            try:
                workspace_id, profile_id, wallet_id = _seed_minimal_scope(conn)
                _insert_tx(
                    conn,
                    tx_id="tx-out",
                    workspace_id=workspace_id,
                    profile_id=profile_id,
                    wallet_id=wallet_id,
                    asset="BTC",
                    direction="outbound",
                )
                _insert_tx(
                    conn,
                    tx_id="tx-in",
                    workspace_id=workspace_id,
                    profile_id=profile_id,
                    wallet_id=wallet_id,
                    asset="BTC",
                    direction="inbound",
                )
                _insert_tx(
                    conn,
                    tx_id="tx-in-2",
                    workspace_id=workspace_id,
                    profile_id=profile_id,
                    wallet_id=wallet_id,
                    asset="BTC",
                    direction="inbound",
                )
                conn.execute(
                    """
                    INSERT INTO transaction_pairs(id, workspace_id, profile_id,
                        out_transaction_id, in_transaction_id, kind, policy,
                        deleted_at, created_at)
                    VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?)
                    """,
                    ("pair-1", workspace_id, profile_id, "tx-out", "tx-in",
                     "whirlpool", "carrying-value", _now(), _now()),
                )
                # Same legs, new active pair: must not raise because the old
                # link is soft-deleted.
                conn.execute(
                    """
                    INSERT INTO transaction_pairs(id, workspace_id, profile_id,
                        out_transaction_id, in_transaction_id, kind, policy,
                        deleted_at, created_at)
                    VALUES(?, ?, ?, ?, ?, ?, ?, NULL, ?)
                    """,
                    ("pair-2", workspace_id, profile_id, "tx-out", "tx-in",
                     "whirlpool", "carrying-value", _now()),
                )
                # A same-asset privacy hop can reuse an outbound leg to link
                # to another inbound leg. Cross-asset one-to-one enforcement
                # lives in the handler because the table does not store assets.
                conn.execute(
                    """
                    INSERT INTO transaction_pairs(id, workspace_id, profile_id,
                        out_transaction_id, in_transaction_id, kind, policy,
                        deleted_at, created_at)
                    VALUES(?, ?, ?, ?, ?, ?, ?, NULL, ?)
                    """,
                    ("pair-3", workspace_id, profile_id, "tx-out", "tx-in-2",
                     "whirlpool", "carrying-value", _now()),
                )
                # Exact active duplicates are still blocked.
                with self.assertRaises(sqlite3.IntegrityError):
                    conn.execute(
                        """
                        INSERT INTO transaction_pairs(id, workspace_id, profile_id,
                            out_transaction_id, in_transaction_id, kind, policy,
                            deleted_at, created_at)
                        VALUES(?, ?, ?, ?, ?, ?, ?, NULL, ?)
                        """,
                        ("pair-4", workspace_id, profile_id, "tx-out", "tx-in",
                         "whirlpool", "carrying-value", _now()),
                    )
            finally:
                conn.close()

    def test_create_transaction_pair_allows_reusing_one_side(self):
        with tempfile.TemporaryDirectory() as data_root:
            conn = open_db(data_root)
            try:
                workspace_id, profile_id, wallet_id = _seed_minimal_scope(conn)
                _insert_tx(
                    conn,
                    tx_id="tx-out",
                    workspace_id=workspace_id,
                    profile_id=profile_id,
                    wallet_id=wallet_id,
                    asset="BTC",
                    direction="outbound",
                    amount_msat=2000,
                )
                for tx_id in ("tx-in-1", "tx-in-2"):
                    _insert_tx(
                        conn,
                        tx_id=tx_id,
                        workspace_id=workspace_id,
                        profile_id=profile_id,
                        wallet_id=wallet_id,
                        asset="BTC",
                        direction="inbound",
                        amount_msat=1000,
                )

                first = create_transaction_pair(
                    conn,
                    workspace_id,
                    profile_id,
                    "tx-out",
                    "tx-in-1",
                    kind="whirlpool",
                )
                second = create_transaction_pair(
                    conn,
                    workspace_id,
                    profile_id,
                    "tx-out",
                    "tx-in-2",
                    kind="whirlpool",
                )
                self.assertNotEqual(first["id"], second["id"])
                self.assertEqual(first["kind"], "whirlpool")
                self.assertIsNone(first["swap_fee_msat"])

                with self.assertRaisesRegex(Exception, "already paired"):
                    create_transaction_pair(
                        conn, workspace_id, profile_id, "tx-out", "tx-in-1"
                    )
                with self.assertRaisesRegex(Exception, "must remain one-to-one"):
                    update_transaction_pair(
                        conn,
                        workspace_id,
                        profile_id,
                        second["id"],
                        kind="submarine-swap",
                    )
            finally:
                conn.close()

    def test_schema_compat_clears_stale_same_asset_privacy_swap_fees(self):
        with tempfile.TemporaryDirectory() as data_root:
            conn = open_db(data_root)
            try:
                workspace_id, profile_id, wallet_id = _seed_minimal_scope(conn)
                for tx_id, asset, direction in [
                    ("same-out", "BTC", "outbound"),
                    ("same-in", "BTC", "inbound"),
                    ("cross-out", "BTC", "outbound"),
                    ("cross-in", "LBTC", "inbound"),
                ]:
                    _insert_tx(
                        conn,
                        tx_id=tx_id,
                        workspace_id=workspace_id,
                        profile_id=profile_id,
                        wallet_id=wallet_id,
                        asset=asset,
                        direction=direction,
                    )
                conn.execute(
                    """
                    INSERT INTO transaction_pairs(id, workspace_id, profile_id,
                        out_transaction_id, in_transaction_id, kind, policy,
                        swap_fee_msat, swap_fee_kind, deleted_at, created_at)
                    VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?)
                    """,
                    (
                        "same-pair",
                        workspace_id,
                        profile_id,
                        "same-out",
                        "same-in",
                        "whirlpool",
                        "carrying-value",
                        123,
                        "loss",
                        _now(),
                    ),
                )
                conn.execute(
                    """
                    INSERT INTO transaction_pairs(id, workspace_id, profile_id,
                        out_transaction_id, in_transaction_id, kind, policy,
                        swap_fee_msat, swap_fee_kind, deleted_at, created_at)
                    VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    """,
                    (
                        "deleted-same-pair",
                        workspace_id,
                        profile_id,
                        "same-out",
                        "same-in",
                        "whirlpool",
                        "carrying-value",
                        789,
                        "loss",
                        _now(),
                        _now(),
                    ),
                )
                conn.execute(
                    """
                    INSERT INTO transaction_pairs(id, workspace_id, profile_id,
                        out_transaction_id, in_transaction_id, kind, policy,
                        swap_fee_msat, swap_fee_kind, deleted_at, created_at)
                    VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?)
                    """,
                    (
                        "cross-pair",
                        workspace_id,
                        profile_id,
                        "cross-out",
                        "cross-in",
                        "manual",
                        "carrying-value",
                        456,
                        "loss",
                        _now(),
                    ),
                )
                conn.commit()

                fee_rows = _swap_fee_summary_rows(conn, profile_id)
                total_row = next(row for row in fee_rows if row["row_type"] == "swap_fees_total")
                self.assertEqual(total_row["total_swap_fee_msat"], 456)

                ensure_schema_compat(conn)
                rows = {
                    row["id"]: row
                    for row in conn.execute(
                        "SELECT id, swap_fee_msat, swap_fee_kind FROM transaction_pairs"
                    ).fetchall()
                }
                self.assertIsNone(rows["same-pair"]["swap_fee_msat"])
                self.assertIsNone(rows["same-pair"]["swap_fee_kind"])
                self.assertEqual(rows["deleted-same-pair"]["swap_fee_msat"], 789)
                self.assertEqual(rows["deleted-same-pair"]["swap_fee_kind"], "loss")
                self.assertEqual(rows["cross-pair"]["swap_fee_msat"], 456)
                self.assertEqual(rows["cross-pair"]["swap_fee_kind"], "loss")
            finally:
                conn.close()

    def test_create_transaction_pair_rejects_reusing_cross_asset_leg(self):
        with tempfile.TemporaryDirectory() as data_root:
            conn = open_db(data_root)
            try:
                workspace_id, profile_id, wallet_id = _seed_minimal_scope(conn)
                _insert_tx(
                    conn,
                    tx_id="tx-out",
                    workspace_id=workspace_id,
                    profile_id=profile_id,
                    wallet_id=wallet_id,
                    asset="LBTC",
                    direction="outbound",
                    amount_msat=2000,
                )
                for tx_id in ("tx-in-1", "tx-in-2"):
                    _insert_tx(
                        conn,
                        tx_id=tx_id,
                        workspace_id=workspace_id,
                        profile_id=profile_id,
                        wallet_id=wallet_id,
                        asset="BTC",
                        direction="inbound",
                        amount_msat=1000,
                    )
                _insert_tx(
                    conn,
                    tx_id="tx-in-lbtc",
                    workspace_id=workspace_id,
                    profile_id=profile_id,
                    wallet_id=wallet_id,
                    asset="LBTC",
                    direction="inbound",
                    amount_msat=1000,
                )

                create_transaction_pair(
                    conn,
                    workspace_id,
                    profile_id,
                    "tx-out",
                    "tx-in-1",
                    kind="submarine-swap",
                    policy="taxable",
                )
                with self.assertRaisesRegex(Exception, "must remain one-to-one"):
                    create_transaction_pair(
                        conn,
                        workspace_id,
                        profile_id,
                        "tx-out",
                        "tx-in-2",
                        kind="submarine-swap",
                        policy="taxable",
                    )
                with self.assertRaisesRegex(Exception, "must remain one-to-one"):
                    create_transaction_pair(
                        conn,
                        workspace_id,
                        profile_id,
                        "tx-out",
                        "tx-in-lbtc",
                        kind="whirlpool",
                        policy="carrying-value",
                    )
            finally:
                conn.close()

    def test_dismissals_unique_blocks_duplicates(self):
        with tempfile.TemporaryDirectory() as data_root:
            conn = open_db(data_root)
            try:
                workspace_id, profile_id, wallet_id = _seed_minimal_scope(conn)
                _insert_tx(
                    conn,
                    tx_id="tx-out",
                    workspace_id=workspace_id,
                    profile_id=profile_id,
                    wallet_id=wallet_id,
                    asset="LBTC",
                    direction="outbound",
                )
                _insert_tx(
                    conn,
                    tx_id="tx-in",
                    workspace_id=workspace_id,
                    profile_id=profile_id,
                    wallet_id=wallet_id,
                    asset="BTC",
                    direction="inbound",
                )
                conn.execute(
                    """
                    INSERT INTO transaction_pair_dismissals(id, workspace_id, profile_id,
                        out_transaction_id, in_transaction_id, created_at, expires_at)
                    VALUES(?, ?, ?, ?, ?, ?, NULL)
                    """,
                    ("dis-1", workspace_id, profile_id, "tx-out", "tx-in", _now()),
                )
                with self.assertRaises(sqlite3.IntegrityError):
                    conn.execute(
                        """
                        INSERT INTO transaction_pair_dismissals(id, workspace_id, profile_id,
                            out_transaction_id, in_transaction_id, created_at, expires_at)
                        VALUES(?, ?, ?, ?, ?, ?, NULL)
                        """,
                        ("dis-2", workspace_id, profile_id, "tx-out", "tx-in", _now()),
                    )
            finally:
                conn.close()

    def test_dismiss_transfer_candidate_is_idempotent(self):
        with tempfile.TemporaryDirectory() as data_root:
            conn = open_db(data_root)
            try:
                workspace_id, profile_id, wallet_id = _seed_minimal_scope(conn)
                _insert_tx(
                    conn,
                    tx_id="tx-out",
                    workspace_id=workspace_id,
                    profile_id=profile_id,
                    wallet_id=wallet_id,
                    asset="LBTC",
                    direction="outbound",
                )
                _insert_tx(
                    conn,
                    tx_id="tx-in",
                    workspace_id=workspace_id,
                    profile_id=profile_id,
                    wallet_id=wallet_id,
                    asset="BTC",
                    direction="inbound",
                )

                first = dismiss_transfer_candidate(
                    conn,
                    workspace_id,
                    profile_id,
                    "tx-out",
                    "tx-in",
                    reason=None,
                    expires_in_days=7,
                )
                second = dismiss_transfer_candidate(
                    conn,
                    workspace_id,
                    profile_id,
                    "tx-out",
                    "tx-in",
                    reason="not-a-swap",
                    expires_in_days=14,
                )

                self.assertEqual(second["id"], first["id"])
                self.assertEqual(second["reason"], "not-a-swap")
                self.assertNotEqual(second["expires_at"], first["expires_at"])
                count = conn.execute(
                    """
                    SELECT COUNT(*) AS count
                    FROM transaction_pair_dismissals
                    WHERE profile_id = ? AND out_transaction_id = ? AND in_transaction_id = ?
                    """,
                    (profile_id, "tx-out", "tx-in"),
                ).fetchone()["count"]
                self.assertEqual(count, 1)
            finally:
                conn.close()


class LegacyUniqueMigrationTests(unittest.TestCase):
    """Simulate the pre-feature ``transaction_pairs`` shape and confirm
    ``ensure_schema_compat`` rebuilds it cleanly.

    Builds a fully-migrated database via ``open_db``, surgically downgrades
    only ``transaction_pairs`` to its legacy form (table-level UNIQUE
    constraints, no new columns), seeds a legacy row, then re-runs
    ``ensure_schema_compat`` and asserts the migration ran end-to-end.
    """

    def _downgrade_transaction_pairs_to_legacy(self, conn):
        conn.execute("PRAGMA foreign_keys = OFF")
        conn.executescript(
            """
            DROP INDEX IF EXISTS idx_transaction_pairs_active_out;
            DROP INDEX IF EXISTS idx_transaction_pairs_active_in;
            DROP INDEX IF EXISTS idx_transaction_pairs_profile_active;
            DROP TABLE IF EXISTS transaction_pairs;
            CREATE TABLE transaction_pairs (
                id TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
                out_transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
                in_transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
                kind TEXT NOT NULL DEFAULT 'manual',
                policy TEXT NOT NULL DEFAULT 'carrying-value',
                notes TEXT,
                created_at TEXT NOT NULL,
                UNIQUE (profile_id, out_transaction_id),
                UNIQUE (profile_id, in_transaction_id)
            );
            """
        )
        conn.commit()
        conn.execute("PRAGMA foreign_keys = ON")

    def test_legacy_unique_constraint_rebuilt_in_place(self):
        with tempfile.TemporaryDirectory() as data_root:
            conn = open_db(data_root)
            try:
                workspace_id, profile_id, wallet_id = _seed_minimal_scope(conn)
                _insert_tx(
                    conn,
                    tx_id="tx-out",
                    workspace_id=workspace_id,
                    profile_id=profile_id,
                    wallet_id=wallet_id,
                    asset="LBTC",
                    direction="outbound",
                )
                _insert_tx(
                    conn,
                    tx_id="tx-in",
                    workspace_id=workspace_id,
                    profile_id=profile_id,
                    wallet_id=wallet_id,
                    asset="BTC",
                    direction="inbound",
                )

                self._downgrade_transaction_pairs_to_legacy(conn)
                # Confirm the downgrade actually re-introduced the table-level UNIQUE.
                downgraded_sql = conn.execute(
                    "SELECT sql FROM sqlite_master WHERE type='table' AND name='transaction_pairs'"
                ).fetchone()["sql"]
                self.assertIn("UNIQUE (profile_id, out_transaction_id)", downgraded_sql)

                conn.execute(
                    """
                    INSERT INTO transaction_pairs(id, workspace_id, profile_id,
                        out_transaction_id, in_transaction_id, kind, policy, created_at)
                    VALUES('legacy-pair', ?, ?, 'tx-out', 'tx-in',
                           'submarine-swap', 'carrying-value', ?)
                    """,
                    (workspace_id, profile_id, _now()),
                )
                conn.commit()

                ensure_schema_compat(conn)

                migrated_sql = conn.execute(
                    "SELECT sql FROM sqlite_master WHERE type='table' AND name='transaction_pairs'"
                ).fetchone()["sql"]
                self.assertNotIn("UNIQUE (profile_id, out_transaction_id)", migrated_sql)
                self.assertNotIn("UNIQUE (profile_id, in_transaction_id)", migrated_sql)

                legacy = conn.execute(
                    "SELECT id, kind, policy FROM transaction_pairs WHERE id = 'legacy-pair'"
                ).fetchone()
                self.assertEqual(legacy["id"], "legacy-pair")
                self.assertEqual(legacy["kind"], "submarine-swap")
                self.assertEqual(legacy["policy"], "carrying-value")

                pair_cols = {row["name"] for row in conn.execute("PRAGMA table_info(transaction_pairs)").fetchall()}
                self.assertIn("deleted_at", pair_cols)
                self.assertIn("swap_fee_msat", pair_cols)

                index_names = {
                    row["name"]
                    for row in conn.execute(
                        "SELECT name FROM sqlite_master WHERE type='index'"
                    ).fetchall()
                }
                self.assertIn("idx_transaction_pairs_active_out", index_names)
                self.assertIn("idx_transaction_pairs_active_in", index_names)
                self.assertIn("idx_transaction_pairs_active_pair", index_names)
            finally:
                conn.close()


if __name__ == "__main__":
    unittest.main()
