from __future__ import annotations

import json
import tempfile
import unittest

from kassiber.core import custody_gap_reviews, custody_gaps
from kassiber.db import custody_gap_review_transaction_id, open_db


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
