import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from kassiber.core import chat_history, maintenance
from kassiber.db import open_db


ROOT = Path(__file__).resolve().parent.parent


def _run_cli(data_root, *args):
    result = subprocess.run(
        [
            sys.executable,
            "-m",
            "kassiber",
            "--data-root",
            str(data_root),
            "--machine",
            *args,
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    stdout = result.stdout.strip()
    if not stdout:
        raise AssertionError(
            f"CLI produced no stdout for {args}; stderr: {result.stderr}"
        )
    payload = json.loads(stdout)
    if result.returncode != 0 or payload.get("kind") == "error":
        raise AssertionError(
            "CLI failed for "
            f"{args}; code={result.returncode}; payload={payload}; "
            f"stderr={result.stderr}"
        )
    return payload


class CoreMaintenanceTest(unittest.TestCase):
    def test_reset_current_profile_data_preserves_connection_rows(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-core-maintenance-") as tmp:
            data_root = Path(tmp) / "data"
            csv_path = Path(tmp) / "transactions.csv"
            csv_path.write_text(
                "\n".join(
                    [
                        "date,txid,direction,asset,amount,fee,fiat_rate,description",
                        "2026-01-01T10:00:00Z,seed-inbound-1,inbound,BTC,0.10000000,0,50000,Seed",
                        "",
                    ]
                ),
                encoding="utf-8",
            )
            _run_cli(data_root, "init")
            _run_cli(data_root, "workspaces", "create", "Demo")
            _run_cli(data_root, "profiles", "create", "Main", "--fiat-currency", "EUR")
            _run_cli(
                data_root,
                "backends",
                "create",
                "local-esplora",
                "--kind",
                "esplora",
                "--url",
                "https://example.invalid/api",
            )
            _run_cli(
                data_root,
                "wallets",
                "create",
                "--label",
                "Cold",
                "--kind",
                "address",
                "--address",
                "bc1qtestaddress0000000000000000000000000000000",
            )
            _run_cli(
                data_root,
                "wallets",
                "import-csv",
                "--wallet",
                "Cold",
                "--file",
                str(csv_path),
            )
            _run_cli(
                data_root,
                "rates",
                "set",
                "BTC-EUR",
                "2026-01-01T00:00:00Z",
                "50000",
            )
            _run_cli(data_root, "journals", "process")
            receipt_path = Path(tmp) / "receipt.txt"
            receipt_path.write_text("receipt\n", encoding="utf-8")
            attachment = _run_cli(
                data_root,
                "attachments",
                "add",
                "--transaction",
                "seed-inbound-1",
                "--file",
                str(receipt_path),
            )
            stored_attachment_path = (
                data_root / "attachments" / attachment["data"]["stored_relpath"]
            )
            attachment_profile_dir = stored_attachment_path.parent

            conn = open_db(str(data_root))
            try:
                workspace_id = conn.execute(
                    "SELECT value FROM settings WHERE key = 'context_workspace'"
                ).fetchone()[0]
                profile_id = conn.execute(
                    "SELECT value FROM settings WHERE key = 'context_profile'"
                ).fetchone()[0]
                chat_session_id = chat_history.create_session(
                    conn,
                    workspace_id,
                    profile_id,
                    title="Reset should clear this",
                    provider="tool-local",
                    model="test-model",
                    commit=False,
                )["id"]
                chat_history.append_exchange(
                    conn,
                    profile_id,
                    chat_session_id,
                    user_content="old prompt",
                    assistant_content="old answer",
                    commit=True,
                )
                component_id = "reset-custody-component"
                conn.execute(
                    """
                    INSERT INTO custody_components(
                        id, lineage_id, workspace_id, profile_id, revision,
                        component_type, expected_leg_count,
                        expected_allocation_count, created_at
                    ) VALUES(?, ?, ?, ?, 1, 'transfer', 1, 0, ?)
                    """,
                    (
                        component_id,
                        "reset-custody-lineage",
                        workspace_id,
                        profile_id,
                        "2026-01-01T10:00:00Z",
                    ),
                )
                conn.execute(
                    """
                    INSERT INTO custody_component_legs(
                        id, component_id, workspace_id, profile_id, ordinal,
                        role, rail, asset, exposure, conservation_unit,
                        amount_msat, created_at
                    ) VALUES(?, ?, ?, ?, 0, 'source', 'bitcoin', 'BTC',
                             'asset', 'BTC', 1000, ?)
                    """,
                    (
                        "reset-custody-leg",
                        component_id,
                        workspace_id,
                        profile_id,
                        "2026-01-01T10:00:00Z",
                    ),
                )
                transaction_id = conn.execute(
                    "SELECT id FROM transactions WHERE external_id = 'seed-inbound-1'"
                ).fetchone()[0]
                conn.execute(
                    """
                    INSERT INTO custody_gap_reviews(
                        id, workspace_id, profile_id, gap_id, revision,
                        candidate_fingerprint, action, authored_source,
                        snapshot_json, created_at
                    ) VALUES('reset-review', ?, ?, 'reset-gap', 1, ?,
                             'dismissed', 'user', '{}', ?)
                    """,
                    (
                        workspace_id,
                        profile_id,
                        "a" * 64,
                        "2026-01-01T10:00:00Z",
                    ),
                )
                conn.execute(
                    """
                    INSERT INTO custody_gap_review_transactions(
                        id, review_id, workspace_id, profile_id,
                        role, transaction_id, created_at
                    ) VALUES('reset-review-source', 'reset-review', ?, ?,
                             'source', ?, ?)
                    """,
                    (
                        workspace_id,
                        profile_id,
                        transaction_id,
                        "2026-01-01T10:00:00Z",
                    ),
                )
                conn.execute(
                    """
                    INSERT INTO custody_gap_candidate_snapshots(
                        cache_token, profile_id, version_json,
                        summary_json, gaps_json
                    ) VALUES('reset-gap-cache', ?, '[]', '{}', '[]')
                    """,
                    (profile_id,),
                )

                payload = maintenance.reset_current_profile_data(conn, str(data_root))

                self.assertTrue(payload["reset"])
                self.assertEqual(payload["profile"]["label"], "Main")
                self.assertEqual(payload["preserved"]["wallets"], 1)
                self.assertGreaterEqual(payload["preserved"]["backends"], 1)
                self.assertEqual(payload["preserved"]["rates_cache"], 1)
                self.assertEqual(payload["removed"]["transactions"], 1)
                self.assertEqual(payload["removed"]["custody_components"], 1)
                self.assertEqual(payload["removed"]["custody_component_legs"], 1)
                self.assertEqual(payload["removed"]["custody_gap_reviews"], 1)
                self.assertEqual(
                    payload["removed"]["custody_gap_candidate_snapshots"], 1
                )
                self.assertEqual(
                    payload["removed"]["custody_gap_review_transactions"], 1
                )
                self.assertGreaterEqual(payload["removed"]["journal_entries"], 1)
                self.assertEqual(payload["removed"]["attachments"], 1)
                self.assertEqual(payload["removed"]["attachment_files"], 1)
                self.assertEqual(payload["removed"]["ai_chat_sessions"], 1)
                self.assertEqual(payload["removed"]["ai_chat_messages"], 2)
                self.assertEqual(payload["removed"]["rates_cache"], 0)
                self.assertEqual(payload["rates_scope"], "preserved")
                self.assertFalse(payload["shared_rates_cleared"])
                self.assertFalse(stored_attachment_path.exists())
                self.assertFalse(attachment_profile_dir.exists())

                for table in (
                    "transactions",
                    "journal_entries",
                    "journal_quarantines",
                    "transaction_pairs",
                    "direct_swap_payouts",
                    "ai_chat_sessions",
                    "ai_chat_messages",
                    "tags",
                    "custody_components",
                    "custody_component_legs",
                    "custody_component_allocations",
                    "custody_component_purge_authorizations",
                    "custody_gap_reviews",
                    "custody_gap_candidate_snapshots",
                    "custody_gap_review_transactions",
                ):
                    count = conn.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]
                    self.assertEqual(count, 0, table)
                self.assertEqual(
                    conn.execute("SELECT COUNT(*) FROM rates_cache").fetchone()[0],
                    1,
                )
                self.assertEqual(
                    conn.execute("SELECT COUNT(*) FROM wallets").fetchone()[0],
                    1,
                )
                self.assertEqual(
                    conn.execute("SELECT COUNT(*) FROM backends").fetchone()[0],
                    payload["preserved"]["backends"],
                )
                rates_payload = maintenance.reset_current_profile_data(
                    conn,
                    str(data_root),
                    clear_shared_rates=True,
                )
                self.assertEqual(rates_payload["removed"]["rates_cache"], 1)
                self.assertEqual(rates_payload["rates_scope"], "global")
                self.assertTrue(rates_payload["shared_rates_cleared"])
                self.assertEqual(
                    conn.execute("SELECT COUNT(*) FROM rates_cache").fetchone()[0],
                    0,
                )
                self.assertEqual(
                    conn.execute(
                        "SELECT COUNT(*) FROM rates_checked_minutes"
                    ).fetchone()[0],
                    0,
                )
            finally:
                conn.close()


if __name__ == "__main__":
    unittest.main()
