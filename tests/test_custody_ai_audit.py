from __future__ import annotations

import sqlite3
import tempfile
import unittest
from pathlib import Path

from kassiber.core import custody_ai_audit
from kassiber.db import open_db


NOW = "2026-07-15T10:00:00Z"


class CustodyAiAuditTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-custody-ai-audit-")
        self.addCleanup(self.tmp.cleanup)
        self.conn = open_db(Path(self.tmp.name))
        self.addCleanup(self.conn.close)
        self.conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Book', ?)",
            (NOW,),
        )
        self.conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, created_at
            ) VALUES('profile', 'ws', 'Owner', 'EUR', 'generic', 365, 'FIFO', ?)
            """,
            (NOW,),
        )
        self.conn.commit()

    def test_append_only_record_is_independent_of_chat_history_and_redacted_for_export(self):
        proposal = {
            "gap_id": "gap:whirlpool",
            "expected_input_version": 7,
            "reason": "private board context",
            "descriptor": "must-not-be-stored",
        }
        record = custody_ai_audit.append_assistance_record(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            tool_name="ui.custody.review.apply",
            daemon_kind="ui.custody.review.apply",
            call_id="call-1",
            provider_kind="local",
            model="local-model",
            model_proposal=proposal,
            final_proposal=proposal,
            consent_decision="allow_once",
            consent_requested_at=NOW,
            consent_decided_at="2026-07-15T10:00:01Z",
            execution_status="executed",
            result={
                "gap_id": "gap:whirlpool",
                "review_id": "review-1",
                "component_id": "component-1",
                "status": "resolved",
            },
        )
        self.conn.commit()

        stored = self.conn.execute(
            "SELECT * FROM custody_ai_assistance_audits WHERE id = ?",
            (record["id"],),
        ).fetchone()
        self.assertIn("private board context", stored["model_proposal_json"])
        self.assertNotIn("descriptor", stored["model_proposal_json"])
        self.assertEqual(stored["review_id"], "review-1")
        self.assertEqual(stored["component_id"], "component-1")
        self.assertEqual(
            self.conn.execute("SELECT COUNT(*) FROM ai_chat_messages").fetchone()[0],
            0,
        )

        exported = custody_ai_audit.redacted_audit_summary(self.conn, "profile")
        self.assertEqual(exported["count"], 1)
        self.assertFalse(exported["raw_proposals_included"])
        self.assertFalse(exported["replicated"])
        self.assertNotIn("model", exported["records"][0])
        self.assertNotIn("model_proposal_json", exported["records"][0])
        self.assertNotIn("private board context", str(exported))
        self.assertEqual(exported["records"][0]["facts_sha256"], record["facts_sha256"])

        with self.assertRaises(Exception):
            self.conn.execute(
                "UPDATE custody_ai_assistance_audits SET execution_status = 'failed' WHERE id = ?",
                (record["id"],),
            )

    def test_denial_is_recorded_but_raw_ai_audit_never_replicates(self):
        custody_ai_audit.append_assistance_record(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            tool_name="ui.custody.review.apply",
            daemon_kind="ui.custody.review.apply",
            call_id="call-denied",
            provider_kind="local",
            model="local-model",
            model_proposal={
                "gap_id": "gap:1",
                "expected_input_version": 8,
            },
            final_proposal=None,
            consent_decision="deny",
            consent_requested_at=NOW,
            consent_decided_at=NOW,
            execution_status="denied",
            execution_code="user_denied",
        )
        self.conn.commit()

        row = self.conn.execute(
            "SELECT consent_decision, execution_status, execution_code "
            "FROM custody_ai_assistance_audits"
        ).fetchone()
        self.assertEqual(tuple(row), ("deny", "denied", "user_denied"))

    def test_explicitly_empty_final_proposal_is_preserved_as_a_user_edit(self):
        record = custody_ai_audit.append_assistance_record(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            tool_name="ui.custody.review.apply",
            daemon_kind="ui.custody.review.apply",
            call_id="call-cleared",
            provider_kind="local",
            model="local-model",
            model_proposal={
                "gap_id": "gap:cleared",
                "expected_input_version": 10,
            },
            final_proposal={},
            consent_decision="allow_once",
            consent_requested_at=NOW,
            consent_decided_at=NOW,
            execution_status="executed",
        )

        stored = self.conn.execute(
            "SELECT final_proposal_json, user_edited, facts_sha256 "
            "FROM custody_ai_assistance_audits WHERE id = ?",
            (record["id"],),
        ).fetchone()
        self.assertEqual(stored["final_proposal_json"], "{}")
        self.assertEqual(stored["user_edited"], 1)
        self.assertEqual(
            stored["facts_sha256"],
            custody_ai_audit._sha256_json(
                {
                    "daemon_kind": "ui.custody.review.apply",
                    "gap_id": None,
                    "candidate_fingerprint": None,
                    "input_version": None,
                    "proposal": {},
                }
            ),
        )

    def test_transaction_scoped_summary_excludes_unrelated_assistance(self):
        for component_id, anchor in (
            ("component-selected", "tx-selected"),
            ("component-private", "tx-private"),
        ):
            self.conn.execute(
                """
                INSERT INTO custody_components(
                    id, lineage_id, workspace_id, profile_id, revision,
                    component_type, conservation_mode, state,
                    expected_leg_count, expected_allocation_count,
                    authored_source, created_at
                ) VALUES(?, ?, 'ws', 'profile', 1, 'manual_bridge', 'quantity',
                         'active', 1, 0, 'user', ?)
                """,
                (component_id, component_id, NOW),
            )
            self.conn.execute(
                """
                INSERT INTO custody_component_legs(
                    id, component_id, workspace_id, profile_id, ordinal, role,
                    rail, asset, exposure, conservation_unit, amount_msat,
                    anchor_transaction_id, created_at
                ) VALUES(?, ?, 'ws', 'profile', 0, 'source', 'untracked',
                         'BTC', 'bitcoin', 'msat', 1, ?, ?)
                """,
                (f"leg-{component_id}", component_id, anchor, NOW),
            )
            custody_ai_audit.append_assistance_record(
                self.conn,
                workspace_id="ws",
                profile_id="profile",
                tool_name="ui.custody.review.apply",
                daemon_kind="ui.custody.review.apply",
                call_id=f"call-{component_id}",
                provider_kind="local",
                model="local-model",
                model_proposal={
                    "gap_id": f"gap-{component_id}",
                    "expected_input_version": 9,
                },
                final_proposal=None,
                consent_decision="allow_once",
                consent_requested_at=NOW,
                consent_decided_at=NOW,
                execution_status="executed",
                result={"component_id": component_id},
            )
        self.conn.commit()

        scoped = custody_ai_audit.redacted_audit_summary(
            self.conn,
            "profile",
            transaction_ids=("tx-selected",),
        )
        self.assertEqual(scoped["count"], 1)
        self.assertEqual(scoped["records"][0]["component_id"], "component-selected")
        self.assertNotIn("gap-component-private", str(scoped))

    def _append(self, record_id: str, consent_decision: str, provider_kind: str = "local") -> None:
        custody_ai_audit.append_assistance_record(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            tool_name="ui.custody.review.apply",
            daemon_kind="ui.custody.review.apply",
            call_id=record_id,
            provider_kind=provider_kind,
            model="claude-code" if provider_kind == "mcp" else "local-model",
            model_proposal={"gap_id": "gap:1", "expected_input_version": 1},
            final_proposal=None,
            consent_decision=consent_decision,
            consent_requested_at=NOW,
            consent_decided_at=NOW,
            execution_status="executed",
        )

    def test_an_mcp_host_approval_is_a_recorded_consent_decision(self):
        self._append("mcp-call", "mcp_host", provider_kind="mcp")
        self.conn.commit()
        summary = custody_ai_audit.redacted_audit_summary(self.conn, "profile")
        self.assertEqual(
            [(r["provider_kind"], r["consent_decision"]) for r in summary["records"]],
            [("mcp", "mcp_host")],
        )
        with self.assertRaises(ValueError):
            self._append("other", "auto_approved")

    def test_an_existing_audit_table_learns_mcp_host_without_losing_records(self):
        # A book from before MCP writes: the old CHECK refuses `mcp_host`.
        self._append("before", "allow_once")
        self.conn.commit()
        sql = self.conn.execute(
            "SELECT sql FROM sqlite_master WHERE name = 'custody_ai_assistance_audits'"
        ).fetchone()[0]
        old_sql = sql.replace(",\n        'mcp_host'", "")
        self.assertNotIn("'mcp_host'", old_sql)
        self.conn.execute("PRAGMA foreign_keys = OFF")
        self.conn.execute("ALTER TABLE custody_ai_assistance_audits RENAME TO audit_copy")
        self.conn.execute(old_sql)
        self.conn.execute("INSERT INTO custody_ai_assistance_audits SELECT * FROM audit_copy")
        self.conn.execute("DROP TABLE audit_copy")
        self.conn.execute(
            "CREATE TRIGGER trg_custody_ai_assistance_immutable BEFORE UPDATE ON "
            "custody_ai_assistance_audits BEGIN SELECT RAISE(ABORT, 'custody_ai_assistance_immutable'); END"
        )
        self.conn.commit()
        with self.assertRaises(sqlite3.IntegrityError):
            self._append("refused", "mcp_host", provider_kind="mcp")
        self.conn.rollback()
        self.conn.close()

        self.conn = open_db(Path(self.tmp.name))
        self.addCleanup(self.conn.close)
        self._append("after", "mcp_host", provider_kind="mcp")
        self.conn.commit()
        rows = self.conn.execute(
            "SELECT call_id, consent_decision FROM custody_ai_assistance_audits ORDER BY created_at, call_id"
        ).fetchall()
        self.assertEqual(sorted(tuple(row) for row in rows), [("after", "mcp_host"), ("before", "allow_once")])
        names = {
            row[0]
            for row in self.conn.execute(
                "SELECT name FROM sqlite_master WHERE tbl_name = 'custody_ai_assistance_audits'"
            )
        }
        self.assertIn("trg_custody_ai_assistance_immutable", names)
        self.assertIn("idx_custody_ai_assistance_profile_time", names)
        with self.assertRaises(sqlite3.DatabaseError):
            self.conn.execute("UPDATE custody_ai_assistance_audits SET model = 'x'")
        self.assertEqual(self.conn.execute("PRAGMA integrity_check").fetchone()[0], "ok")
        # A current table is left alone on the next open.
        before = self.conn.execute(
            "SELECT sql FROM sqlite_master WHERE name = 'custody_ai_assistance_audits'"
        ).fetchone()[0]
        self.conn.close()
        self.conn = open_db(Path(self.tmp.name))
        self.addCleanup(self.conn.close)
        self.assertEqual(
            self.conn.execute(
                "SELECT sql FROM sqlite_master WHERE name = 'custody_ai_assistance_audits'"
            ).fetchone()[0],
            before,
        )


if __name__ == "__main__":
    unittest.main()
