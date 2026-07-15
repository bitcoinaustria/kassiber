from __future__ import annotations

import json
import sqlite3
import tempfile
import unittest
from unittest.mock import patch

from kassiber.core import custody_gap_reviews, custody_gaps
from kassiber.db import (
    custody_gap_review_transaction_id,
    custody_gap_review_transaction_v1_id,
    open_db,
)


BTC = 100_000_000_000
NOW = "2026-01-01T00:00:00Z"


class CustodyReviewScopeMigrationTests(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.TemporaryDirectory()
        self.addCleanup(self.root.cleanup)
        self.conn = open_db(self.root.name)
        self.conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Books', ?)",
            (NOW,),
        )
        self.conn.execute(
            "INSERT INTO profiles(id, workspace_id, label, created_at) "
            "VALUES('profile', 'ws', 'Book', ?)",
            (NOW,),
        )
        for wallet_id, label in (("old", "Old vault"), ("new", "New vault")):
            self.conn.execute(
                """
                INSERT INTO wallets(
                    id, workspace_id, profile_id, label, kind,
                    config_json, created_at
                ) VALUES(?, 'ws', 'profile', ?, 'descriptor',
                         '{"chain":"bitcoin","network":"main"}', ?)
                """,
                (wallet_id, label, NOW),
            )
        self._insert_boundary_transactions()
        self.conn.commit()

    def tearDown(self):
        self.conn.close()

    def _insert_boundary_transactions(self) -> None:
        rows = (
            (
                "out",
                "old",
                "outbound",
                10 * BTC,
                10_000_000,
                "2020-01-01T00:00:00Z",
                "coinjoin",
            ),
            (
                "return",
                "new",
                "inbound",
                99 * BTC // 10,
                0,
                "2021-01-01T00:00:00Z",
                None,
            ),
        )
        for tx_id, wallet_id, direction, amount, fee, occurred_at, boundary in rows:
            self.conn.execute(
                """
                INSERT INTO transactions(
                    id, workspace_id, profile_id, wallet_id, external_id,
                    fingerprint, occurred_at, direction, asset, amount, fee,
                    privacy_boundary, raw_json, created_at
                ) VALUES(?, 'ws', 'profile', ?, ?, ?, ?, ?, 'BTC', ?, ?, ?, '{}', ?)
                """,
                (
                    tx_id,
                    wallet_id,
                    tx_id,
                    f"fp-{tx_id}",
                    occurred_at,
                    direction,
                    amount,
                    fee,
                    boundary,
                    occurred_at,
                ),
            )

    def _install_v1_relation_table(self, review_id: str) -> str:
        self.conn.execute(
            "DROP TRIGGER IF EXISTS trg_custody_gap_review_transaction_scope_insert"
        )
        self.conn.execute(
            "DROP TRIGGER IF EXISTS trg_custody_gap_review_transactions_immutable"
        )
        self.conn.execute("DROP TABLE custody_gap_review_transactions")
        self.conn.execute(
            """
            CREATE TABLE custody_gap_review_transactions(
                id TEXT PRIMARY KEY,
                review_id TEXT NOT NULL,
                workspace_id TEXT NOT NULL,
                profile_id TEXT NOT NULL,
                ordinal INTEGER NOT NULL,
                role TEXT NOT NULL,
                transaction_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                UNIQUE(review_id, role, ordinal),
                UNIQUE(review_id, role, transaction_id)
            )
            """
        )
        legacy_id = custody_gap_review_transaction_v1_id(
            review_id, "source", 0
        )
        self.conn.execute(
            """
            INSERT INTO custody_gap_review_transactions(
                id, review_id, workspace_id, profile_id, ordinal,
                role, transaction_id, created_at
            ) VALUES(?, ?, 'ws', 'profile', 0, 'source', 'out', ?)
            """,
            (legacy_id, review_id, NOW),
        )
        return legacy_id

    def test_v1_relation_rebuild_rolls_back_every_schema_step_on_failure(self):
        self._insert_legacy_review_on(
            self.conn,
            "crash-review",
            snapshot={"source_ids": ["out"]},
        )
        self._install_v1_relation_table("crash-review")
        db_path = str(
            self.conn.execute("PRAGMA database_list").fetchone()[2]
        )
        self.conn.commit()
        self.conn.close()

        with (
            patch(
                "kassiber.db.custody_gap_review_transaction_id",
                side_effect=RuntimeError("injected migration failure"),
            ),
            self.assertRaises(RuntimeError),
        ):
            open_db(self.root.name)

        raw = sqlite3.connect(db_path)
        try:
            columns = {
                row[1]
                for row in raw.execute(
                    "PRAGMA table_info(custody_gap_review_transactions)"
                ).fetchall()
            }
            tables = {
                row[0]
                for row in raw.execute(
                    "SELECT name FROM sqlite_master WHERE type = 'table'"
                ).fetchall()
            }
            self.assertIn("ordinal", columns)
            self.assertNotIn("custody_gap_review_transactions_v1", tables)
        finally:
            raw.close()

        self.conn = open_db(self.root.name)
        self.assertNotIn(
            "ordinal",
            {
                row["name"]
                for row in self.conn.execute(
                    "PRAGMA table_info(custody_gap_review_transactions)"
                ).fetchall()
            },
        )

    def test_open_recovers_legacy_table_left_by_old_half_migration(self):
        self._insert_legacy_review_on(
            self.conn,
            "half-review",
            snapshot={"source_ids": ["out"]},
        )
        self._install_v1_relation_table("half-review")
        self.conn.execute(
            "ALTER TABLE custody_gap_review_transactions "
            "RENAME TO custody_gap_review_transactions_v1"
        )
        self.conn.execute(
            """
            CREATE TABLE custody_gap_review_transactions(
                id TEXT PRIMARY KEY,
                review_id TEXT NOT NULL,
                workspace_id TEXT NOT NULL,
                profile_id TEXT NOT NULL,
                role TEXT NOT NULL,
                transaction_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                UNIQUE(review_id, role, transaction_id)
            )
            """
        )
        self.conn.commit()
        self.conn.close()

        self.conn = open_db(self.root.name)
        tables = {
            row["name"]
            for row in self.conn.execute(
                "SELECT name FROM sqlite_master WHERE type = 'table'"
            ).fetchall()
        }
        self.assertNotIn("custody_gap_review_transactions_v1", tables)
        relation = self.conn.execute(
            """
            SELECT id, review_id, role, transaction_id
            FROM custody_gap_review_transactions
            WHERE review_id = 'half-review'
            """
        ).fetchone()
        self.assertEqual(
            tuple(relation),
            (
                custody_gap_review_transaction_id(
                    "half-review", "source", "out"
                ),
                "half-review",
                "source",
                "out",
            ),
        )

    def _candidate(self):
        candidates, _ = custody_gaps.load_gap_candidates(
            self.conn, "profile", include_journal_claims=False
        )
        self.assertEqual(len(candidates), 1)
        return candidates[0]

    def _insert_legacy_review(
        self,
        review_id: str,
        *,
        gap_id: str,
        fingerprint: str,
        snapshot: dict | None = None,
    ) -> None:
        self.conn.execute(
            """
            INSERT INTO custody_gap_reviews(
                id, workspace_id, profile_id, gap_id, revision,
                candidate_fingerprint, action, event_kind, component_id,
                authored_source, reason, snapshot_json, created_at
            ) VALUES(?, 'ws', 'profile', ?, 1, ?, 'dismissed',
                     'review_decision', NULL, 'user', 'legacy dismissal', ?, ?)
            """,
            (
                review_id,
                gap_id,
                fingerprint,
                json.dumps(snapshot or {"retained_msat": 99 * BTC // 10}),
                NOW,
            ),
        )

    def _reopen(self) -> None:
        self.conn.commit()
        self.conn.close()
        self.conn = open_db(self.root.name)

    def test_exact_legacy_dismissal_recovery_is_durable_and_bounded_safe(self):
        candidate = self._candidate()
        self._insert_legacy_review(
            "legacy",
            gap_id=candidate.gap_id,
            fingerprint=custody_gap_reviews.candidate_fingerprint(candidate),
        )

        self._reopen()

        relations = [
            (row["role"], row["transaction_id"])
            for row in self.conn.execute(
                """
                SELECT role, transaction_id
                FROM custody_gap_review_transactions
                WHERE review_id = 'legacy'
                ORDER BY role, transaction_id
                """
            ).fetchall()
        ]
        self.assertEqual(relations, [("return", "return"), ("source", "out")])
        bounded = custody_gap_reviews.list_audit_review_history(
            self.conn, "profile", transaction_ids=["out"]
        )
        self.assertEqual(bounded["scope_completeness"], "complete")
        self.assertNotIn("legacy_unscoped_review_count", bounded)
        self.assertEqual(bounded["records"][0]["gap_id"], candidate.gap_id)
        full = custody_gap_reviews.list_audit_review_history(self.conn, "profile")
        self.assertEqual(full["legacy_unscoped_review_count"], 0)

    def test_fingerprint_mismatch_stays_explicit_and_later_restore_retries(self):
        candidate = self._candidate()
        self._insert_legacy_review(
            "wrong",
            gap_id=candidate.gap_id,
            fingerprint="f" * 64,
        )
        self._reopen()

        self.assertEqual(
            self.conn.execute(
                "SELECT COUNT(*) FROM custody_gap_review_transactions "
                "WHERE review_id = 'wrong'"
            ).fetchone()[0],
            0,
        )
        bounded = custody_gap_reviews.list_audit_review_history(
            self.conn, "profile", transaction_ids=["out"]
        )
        self.assertEqual(
            bounded["scope_completeness"], "legacy_unscoped_history_present"
        )
        self.assertNotIn("legacy_unscoped_review_count", bounded)
        full = custody_gap_reviews.list_audit_review_history(self.conn, "profile")
        self.assertEqual(full["legacy_unscoped_review_count"], 1)

        self.conn.execute("DELETE FROM custody_gap_reviews WHERE id = 'wrong'")
        self._insert_legacy_review(
            "restored",
            gap_id=candidate.gap_id,
            fingerprint=custody_gap_reviews.candidate_fingerprint(candidate),
        )
        self.conn.execute("DELETE FROM transactions WHERE profile_id = 'profile'")
        self._reopen()
        self.assertEqual(
            self.conn.execute(
                "SELECT COUNT(*) FROM custody_gap_review_transactions "
                "WHERE review_id = 'restored'"
            ).fetchone()[0],
            0,
        )

        self._insert_boundary_transactions()
        self._reopen()
        self.assertEqual(
            self.conn.execute(
                "SELECT COUNT(*) FROM custody_gap_review_transactions "
                "WHERE review_id = 'restored'"
            ).fetchone()[0],
            2,
        )

    def test_structural_backfill_repairs_each_relation_and_rejects_live_collision(self):
        self._insert_legacy_review(
            "snapshot",
            gap_id="snapshot-gap",
            fingerprint="a" * 64,
            snapshot={"source_ids": ["missing-source"], "return_ids": ["return"]},
        )
        self.conn.execute(
            """
            INSERT INTO custody_gap_review_transactions(
                id, review_id, workspace_id, profile_id,
                role, transaction_id, created_at
            ) VALUES('existing-return', 'snapshot', 'ws', 'profile',
                     'return', 'return', ?)
            """,
            (NOW,),
        )
        self.conn.execute(
            "INSERT INTO profiles(id, workspace_id, label, created_at) "
            "VALUES('other-profile', 'ws', 'Other', ?)",
            (NOW,),
        )
        self.conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES('other-wallet', 'ws', 'other-profile', 'Other wallet',
                     'custom', '{}', ?)
            """,
            (NOW,),
        )
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id,
                fingerprint, occurred_at, direction, asset, amount, fee,
                raw_json, created_at
            ) VALUES('missing-source', 'ws', 'other-profile', 'other-wallet',
                     'collision', 'collision-fp', ?, 'outbound', 'BTC', 1, 0,
                     '{}', ?)
            """,
            (NOW, NOW),
        )

        self._reopen()

        self.assertEqual(
            self.conn.execute(
                "SELECT COUNT(*) FROM custody_gap_review_transactions "
                "WHERE review_id = 'snapshot'"
            ).fetchone()[0],
            1,
        )
        header = self.conn.execute(
            """
            SELECT expected_source_count, expected_return_count
            FROM custody_gap_review_relation_sets
            WHERE review_id = 'snapshot'
            """
        ).fetchone()
        self.assertEqual(tuple(header), (1, 1))
        partial = custody_gap_reviews.list_audit_review_history(
            self.conn,
            "profile",
            transaction_ids=["return", "missing-source"],
        )
        self.assertTrue(
            partial["records"][0]["candidate_wide_payload_excluded"]
        )
        full = custody_gap_reviews.list_audit_review_history(self.conn, "profile")
        self.assertEqual(
            full["scope_completeness"], "legacy_unscoped_history_present"
        )
        self.conn.execute("DELETE FROM transactions WHERE id = 'missing-source'")
        self._reopen()
        rows = self.conn.execute(
            """
            SELECT role, transaction_id
            FROM custody_gap_review_transactions
            WHERE review_id = 'snapshot'
            ORDER BY role, transaction_id
            """
        ).fetchall()
        self.assertEqual(
            [(row["role"], row["transaction_id"]) for row in rows],
            [("return", "return"), ("source", "missing-source")],
        )
        self.assertEqual(
            custody_gap_reviews.list_audit_review_history(self.conn, "profile")[
                "scope_completeness"
            ],
            "complete",
        )
        complete = custody_gap_reviews.list_audit_review_history(
            self.conn,
            "profile",
            transaction_ids=["return", "missing-source"],
        )
        self.assertNotIn(
            "candidate_wide_payload_excluded",
            complete["records"][0],
        )

    def test_v1_migration_uses_signed_wire_tuple_across_divergent_alias_sets(self):
        roots = [tempfile.TemporaryDirectory(), tempfile.TemporaryDirectory()]
        for root in roots:
            self.addCleanup(root.cleanup)

        migrated_ids: list[str] = []
        for index, root in enumerate(roots):
            conn = open_db(root.name)
            conn.execute(
                "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Books', ?)",
                (NOW,),
            )
            conn.execute(
                "INSERT INTO profiles(id, workspace_id, label, created_at) "
                "VALUES('profile', 'ws', 'Book', ?)",
                (NOW,),
            )
            local_review_id = f"local-review-{index}"
            local_transaction_id = f"local-transaction-{index}"
            self._insert_legacy_review_on(
                conn,
                local_review_id,
                snapshot={"source_ids": [local_transaction_id]},
            )
            conn.execute(
                "INSERT INTO sync_id_map(profile_id, entity_table, wire_id, local_id, created_at) "
                "VALUES('profile', 'custody_gap_reviews', 'wire-review', ?, ?)",
                (local_review_id, NOW),
            )
            conn.executemany(
                "INSERT INTO sync_id_map(profile_id, entity_table, wire_id, local_id, created_at) "
                "VALUES('profile', 'transactions', ?, ?, ?)",
                (
                    ("wire-transaction", local_transaction_id, NOW),
                    (f"device-only-alias-{index}", local_transaction_id, NOW),
                ),
            )
            conn.execute("DROP TRIGGER trg_custody_gap_review_transaction_scope_insert")
            conn.execute("DROP TRIGGER trg_custody_gap_review_transactions_immutable")
            conn.execute("DROP TABLE custody_gap_review_transactions")
            conn.execute(
                """
                CREATE TABLE custody_gap_review_transactions(
                    id TEXT PRIMARY KEY,
                    review_id TEXT NOT NULL,
                    workspace_id TEXT NOT NULL,
                    profile_id TEXT NOT NULL,
                    ordinal INTEGER NOT NULL,
                    role TEXT NOT NULL,
                    transaction_id TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    UNIQUE(review_id, role, ordinal),
                    UNIQUE(review_id, role, transaction_id)
                )
                """
            )
            legacy_id = custody_gap_review_transaction_v1_id(
                "wire-review", "source", 0
            )
            conn.execute(
                """
                INSERT INTO custody_gap_review_transactions(
                    id, review_id, workspace_id, profile_id, ordinal,
                    role, transaction_id, created_at
                ) VALUES(?, ?, 'ws', 'profile', 0, 'source', ?, ?)
                """,
                (legacy_id, local_review_id, local_transaction_id, NOW),
            )
            wire_row = {
                "id": legacy_id,
                "review_id": "wire-review",
                "workspace_id": "ws",
                "profile_id": "profile",
                "ordinal": 0,
                "role": "source",
                "transaction_id": "wire-transaction",
                "created_at": NOW,
            }
            conn.execute(
                """
                INSERT INTO sync_events(
                    id, workspace_id, profile_id, replica_id, replica_seq, hlc,
                    author_member_id, event_type, entity_table, entity_key,
                    payload_json, context_json, previous_hash, event_hash,
                    signature, created_at, applied_at
                ) VALUES(?, 'ws', 'profile', ?, 1, ?, 'member', 'row.upsert',
                         'custody_gap_review_transactions', ?, ?, '{}', NULL,
                         ?, 'signed-v1', ?, ?)
                """,
                (
                    f"event-{index}",
                    f"replica-{index}",
                    f"1:0:replica-{index}",
                    json.dumps([legacy_id], separators=(",", ":")),
                    json.dumps({"row": wire_row}, separators=(",", ":")),
                    f"{index + 1:064x}",
                    NOW,
                    NOW,
                ),
            )
            conn.commit()
            conn.close()

            migrated = open_db(root.name)
            self.addCleanup(migrated.close)
            migrated_ids.append(
                str(
                    migrated.execute(
                        "SELECT id FROM custody_gap_review_transactions"
                    ).fetchone()[0]
                )
            )
            mapped = migrated.execute(
                "SELECT local_id FROM sync_id_map WHERE profile_id = 'profile' "
                "AND entity_table = 'custody_gap_review_transactions' AND wire_id = ?",
                (legacy_id,),
            ).fetchone()
            self.assertEqual(str(mapped["local_id"]), migrated_ids[-1])

        expected = custody_gap_review_transaction_id(
            "wire-review", "source", "wire-transaction"
        )
        self.assertEqual(migrated_ids, [expected, expected])

    def test_ordinal_v1_replicas_converge_to_transaction_set_identity(self):
        roots = [tempfile.TemporaryDirectory(), tempfile.TemporaryDirectory()]
        for root in roots:
            self.addCleanup(root.cleanup)

        def legacy_replica(root: str, ordered_ids: tuple[str, str]):
            conn = open_db(root)
            conn.execute(
                "INSERT INTO workspaces(id, label, created_at) "
                "VALUES('ws', 'Books', ?)",
                (NOW,),
            )
            conn.execute(
                "INSERT INTO profiles(id, workspace_id, label, created_at) "
                "VALUES('profile', 'ws', 'Book', ?)",
                (NOW,),
            )
            self._insert_legacy_review_on(
                conn,
                "legacy-set",
                snapshot={"source_ids": ["alpha", "beta"]},
            )
            conn.execute("DROP TRIGGER trg_custody_gap_review_transaction_scope_insert")
            conn.execute("DROP TRIGGER trg_custody_gap_review_transactions_immutable")
            conn.execute("DROP TABLE custody_gap_review_transactions")
            conn.execute(
                """
                CREATE TABLE custody_gap_review_transactions(
                    id TEXT PRIMARY KEY,
                    review_id TEXT NOT NULL,
                    workspace_id TEXT NOT NULL,
                    profile_id TEXT NOT NULL,
                    ordinal INTEGER NOT NULL,
                    role TEXT NOT NULL,
                    transaction_id TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    UNIQUE(review_id, role, ordinal),
                    UNIQUE(review_id, role, transaction_id)
                )
                """
            )
            for ordinal, transaction_id in enumerate(ordered_ids):
                conn.execute(
                    """
                    INSERT INTO custody_gap_review_transactions(
                        id, review_id, workspace_id, profile_id, ordinal,
                        role, transaction_id, created_at
                    ) VALUES(?, 'legacy-set', 'ws', 'profile', ?,
                             'source', ?, ?)
                    """,
                    (f"v1-{ordinal}-{transaction_id}", ordinal, transaction_id, NOW),
                )
            conn.commit()
            conn.close()
            return open_db(root)

        first = legacy_replica(roots[0].name, ("alpha", "beta"))
        second = legacy_replica(roots[1].name, ("beta", "alpha"))
        self.addCleanup(first.close)
        self.addCleanup(second.close)
        rows = []
        for conn in (first, second):
            self.assertNotIn(
                "ordinal",
                {
                    row["name"]
                    for row in conn.execute(
                        "PRAGMA table_info(custody_gap_review_transactions)"
                    ).fetchall()
                },
            )
            rows.append(
                [
                    tuple(row)
                    for row in conn.execute(
                        """
                        SELECT id, role, transaction_id
                        FROM custody_gap_review_transactions
                        WHERE review_id = 'legacy-set'
                        ORDER BY role, transaction_id
                        """
                    ).fetchall()
                ]
            )
        self.assertEqual(rows[0], rows[1])
        self.assertEqual(
            rows[0],
            [
                (
                    custody_gap_review_transaction_id(
                        "legacy-set", "source", transaction_id
                    ),
                    "source",
                    transaction_id,
                )
                for transaction_id in ("alpha", "beta")
            ],
        )

    @staticmethod
    def _insert_legacy_review_on(
        conn,
        review_id: str,
        *,
        snapshot: dict,
    ) -> None:
        conn.execute(
            """
            INSERT INTO custody_gap_reviews(
                id, workspace_id, profile_id, gap_id, revision,
                candidate_fingerprint, action, event_kind, component_id,
                authored_source, reason, snapshot_json, created_at
            ) VALUES(?, 'ws', 'profile', 'legacy-gap', 1, ?, 'dismissed',
                     'review_decision', NULL, 'user', 'legacy', ?, ?)
            """,
            (review_id, "a" * 64, json.dumps(snapshot), NOW),
        )


if __name__ == "__main__":
    unittest.main()
