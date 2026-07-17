"""Public CLI and daemon contracts for versioned custody components."""

from __future__ import annotations

import io
import json
import select
import subprocess
import sys
import tempfile
import time
import unittest
from contextlib import redirect_stdout
from pathlib import Path

from kassiber.cli.main import build_parser, dispatch
from kassiber.core import custody_components
from kassiber.daemon import SUPPORTED_KINDS, _ui_swap_matching_payload_from_conn
from kassiber.db import open_db
from kassiber.errors import AppError


ROOT = Path(__file__).resolve().parent.parent
NOW = "2026-01-01T00:00:00Z"
COMPONENT_KINDS = {
    "ui.transfers.components.list",
    "ui.transfers.components.get",
    "ui.transfers.components.plan",
    "ui.transfers.components.apply",
}


def _fixture(data_root: Path) -> None:
    conn = open_db(str(data_root))
    try:
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Main', ?)",
            (NOW,),
        )
        conn.execute(
            """
            INSERT INTO profiles(id, workspace_id, label, created_at)
            VALUES('profile', 'ws', 'Book', ?)
            """,
            (NOW,),
        )
        conn.execute(
            """
            INSERT INTO accounts(
                id, workspace_id, profile_id, code, label, account_type, created_at
            ) VALUES('treasury', 'ws', 'profile', 'treasury', 'Treasury',
                     'asset', ?)
            """,
            (NOW,),
        )
        for wallet_id, label in (("source-wallet", "Source"), ("sink-wallet", "Sink")):
            conn.execute(
                """
                INSERT INTO wallets(
                    id, workspace_id, profile_id, label, kind, config_json, created_at
                ) VALUES(?, 'ws', 'profile', ?, 'custom', '{}', ?)
                """,
                (wallet_id, label, NOW),
            )
        for tx_id, wallet_id, direction, amount in (
            ("out", "source-wallet", "outbound", 100_000),
            ("in", "sink-wallet", "inbound", 99_000),
        ):
            conn.execute(
                """
                INSERT INTO transactions(
                    id, workspace_id, profile_id, wallet_id, fingerprint,
                    external_id, occurred_at, direction, asset, amount, fee,
                    created_at
                ) VALUES(?, 'ws', 'profile', ?, ?, ?, ?, ?, 'BTC', ?, 0, ?)
                """,
                (
                    tx_id,
                    wallet_id,
                    f"fingerprint-{tx_id}",
                    f"external-{tx_id}",
                    NOW,
                    direction,
                    amount,
                    NOW,
                ),
            )
        conn.commit()
    finally:
        conn.close()


def _component_spec(*, note: str = "migration") -> dict:
    return {
        "component_type": "native_transfer",
        "evidence_kind": "manual_claim",
        "evidence_grade": "reviewed",
        "notes": note,
        "legs": [
            {
                "role": "source",
                "transaction": "external-out",
                "amount_msat": 100_000,
            },
            {
                "role": "destination",
                "transaction": "external-in",
                "amount_msat": 99_000,
            },
            {
                "role": "fee",
                "transaction": "external-out",
                "amount_msat": 1_000,
            },
        ],
    }


def _dispatch_json(conn, data_root: Path, *argv: str) -> dict:
    args = build_parser().parse_args(
        ["--data-root", str(data_root), "--machine", *argv]
    )
    args.format = "json"
    args.non_interactive = True
    output = io.StringIO()
    with redirect_stdout(output):
        dispatch(conn, args)
    return json.loads(output.getvalue())


def _start_daemon(data_root: Path) -> subprocess.Popen[str]:
    return subprocess.Popen(
        [
            sys.executable,
            "-m",
            "kassiber",
            "--data-root",
            str(data_root),
            "daemon",
        ],
        cwd=ROOT,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )


def _request(proc: subprocess.Popen[str], payload: dict, timeout: float = 10.0) -> dict:
    assert proc.stdin is not None
    assert proc.stdout is not None
    proc.stdin.write(json.dumps(payload) + "\n")
    proc.stdin.flush()
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        ready, _, _ = select.select(
            [proc.stdout.fileno()], [], [], max(0.0, deadline - time.monotonic())
        )
        if not ready:
            break
        line = proc.stdout.readline()
        if not line:
            break
        envelope = json.loads(line)
        if envelope.get("request_id") == payload["request_id"]:
            return envelope
    raise AssertionError(f"daemon did not answer {payload['request_id']!r}")


class CustodyComponentCliSurfaceTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-component-cli-")
        self.addCleanup(self.tmp.cleanup)
        self.data_root = Path(self.tmp.name) / "data"
        _fixture(self.data_root)
        self.conn = open_db(str(self.data_root))
        self.addCleanup(self.conn.close)

    def test_ai_bulk_resolution_stamps_component_attribution(self):
        args = {
            "workspace": "Main",
            "profile": "Book",
            "components": [_component_spec()],
            "activate": False,
        }
        preview = _ui_swap_matching_payload_from_conn(
            self.conn,
            "ui.transfers.components.plan",
            args,
            authored_source="ai_tool",
        )
        args.update(
            expected_fingerprint=preview["fingerprint"],
        )
        result = _ui_swap_matching_payload_from_conn(
            self.conn,
            "ui.transfers.components.apply",
            args,
            authored_source="ai_tool",
        )

        self.assertEqual(result["components"][0]["authored_source"], "ai_tool")
        stored = self.conn.execute(
            "SELECT authored_source FROM custody_components WHERE id = ?",
            (result["components"][0]["id"],),
        ).fetchone()
        self.assertEqual(stored["authored_source"], "ai_tool")

    def _create_draft_component(self) -> dict:
        args = {
            "workspace": "Main",
            "profile": "Book",
            "action": "create",
            "components": [_component_spec()],
            "activate": False,
        }
        preview = _ui_swap_matching_payload_from_conn(
            self.conn, "ui.transfers.components.plan", args
        )
        result = _ui_swap_matching_payload_from_conn(
            self.conn,
            "ui.transfers.components.apply",
            {**args, "expected_fingerprint": preview["fingerprint"]},
        )
        return result["components"][0]

    def test_component_activation_preview_is_pure_and_apply_is_exact(self):
        component = self._create_draft_component()
        args = {
            "workspace": "Main",
            "profile": "Book",
            "action": "activate",
            "component_id": component["id"],
        }
        before_changes = self.conn.total_changes

        preview = _ui_swap_matching_payload_from_conn(
            self.conn, "ui.transfers.components.plan", args
        )

        self.assertTrue(preview["dry_run"])
        self.assertEqual(preview["current_state"], "draft")
        self.assertEqual(preview["resulting_state"], "active")
        self.assertEqual(self.conn.total_changes, before_changes)
        self.assertFalse(self.conn.in_transaction)
        result = _ui_swap_matching_payload_from_conn(
            self.conn,
            "ui.transfers.components.apply",
            {**args, "expected_fingerprint": preview["fingerprint"]},
        )
        self.assertEqual(result["component"]["effective_state"], "active")

    def test_component_state_apply_rejects_stale_input_version(self):
        component = self._create_draft_component()
        args = {
            "workspace": "Main",
            "profile": "Book",
            "action": "activate",
            "component_id": component["id"],
        }
        preview = _ui_swap_matching_payload_from_conn(
            self.conn, "ui.transfers.components.plan", args
        )
        self.conn.execute(
            "UPDATE profiles SET journal_input_version = journal_input_version + 1 "
            "WHERE id = 'profile'"
        )
        self.conn.commit()

        with self.assertRaises(AppError) as caught:
            _ui_swap_matching_payload_from_conn(
                self.conn,
                "ui.transfers.components.apply",
                {**args, "expected_fingerprint": preview["fingerprint"]},
            )

        self.assertEqual(caught.exception.code, "custody_review_plan_stale")
        stored = self.conn.execute(
            "SELECT state FROM custody_components WHERE id = ?", (component["id"],)
        ).fetchone()
        self.assertEqual(stored["state"], "draft")

    def test_component_revision_preview_is_pure_and_preserves_economic_terms(self):
        component = self._create_draft_component()
        source = next(leg for leg in component["legs"] if leg["role"] == "source")
        target = next(
            leg for leg in component["legs"] if leg["role"] == "destination"
        )
        custody_components.seal_component_economic_terms(
            self.conn,
            component["id"],
            [
                {
                    "id": "legacy-term",
                    "source_leg_id": source["id"],
                    "target_leg_id": target["id"],
                    "term_kind": "transaction_pair",
                    "legacy_source_id": "legacy-pair",
                    "source_row_hash": "a" * 64,
                    "review_kind": "transfer",
                    "tax_policy": "transfer",
                    "swap_fee_msat": 1_000,
                }
            ],
        )
        self.conn.commit()
        args = {
            "workspace": "Main",
            "profile": "Book",
            "action": "revise",
            "component_id": component["id"],
            "spec": {"notes": "note-only revision"},
            "activate": False,
        }
        before_changes = self.conn.total_changes

        preview = _ui_swap_matching_payload_from_conn(
            self.conn, "ui.transfers.components.plan", args
        )

        self.assertEqual(self.conn.total_changes, before_changes)
        self.assertFalse(self.conn.in_transaction)
        self.assertTrue(preview["dry_run"])
        self.assertNotIn("evidence", preview["component"])
        self.assertTrue(
            all("location_ref" not in leg for leg in preview["component"]["legs"])
        )
        result = _ui_swap_matching_payload_from_conn(
            self.conn,
            "ui.transfers.components.apply",
            {**args, "expected_fingerprint": preview["fingerprint"]},
        )
        revised = result["component"]
        self.assertEqual(revised["revision"], 2)
        self.assertEqual(len(revised["economic_terms"]), 1)
        self.assertEqual(revised["economic_terms"][0]["tax_policy"], "transfer")
        self.assertEqual(revised["economic_terms"][0]["swap_fee_msat"], 1_000)
        self.assertNotEqual(
            revised["economic_terms"][0]["id"], "legacy-term"
        )

    def test_component_revision_apply_rejects_reused_plan(self):
        component = self._create_draft_component()
        args = {
            "workspace": "Main",
            "profile": "Book",
            "action": "revise",
            "component_id": component["id"],
            "spec": {"notes": "one reviewed revision"},
            "activate": False,
        }
        preview = _ui_swap_matching_payload_from_conn(
            self.conn, "ui.transfers.components.plan", args
        )
        apply_args = {**args, "expected_fingerprint": preview["fingerprint"]}
        _ui_swap_matching_payload_from_conn(
            self.conn, "ui.transfers.components.apply", apply_args
        )

        with self.assertRaises(AppError) as caught:
            _ui_swap_matching_payload_from_conn(
                self.conn, "ui.transfers.components.apply", apply_args
            )

        self.assertEqual(caught.exception.code, "custody_component_draft_exists")

    def test_daemon_bulk_resolution_rejects_unbounded_batches(self):
        with self.assertRaises(AppError) as caught:
            _ui_swap_matching_payload_from_conn(
                self.conn,
                "ui.transfers.components.plan",
                {
                    "workspace": "Main",
                    "profile": "Book",
                    "components": [_component_spec() for _ in range(51)],
                    "activate": False,
                },
            )

        self.assertEqual(caught.exception.code, "validation")
        self.assertEqual(caught.exception.details["max_components"], 50)

    def test_bulk_resolution_rejects_reusing_an_applied_plan(self):
        args = {
            "workspace": "Main",
            "profile": "Book",
            "components": [_component_spec()],
            "activate": False,
        }
        preview = _ui_swap_matching_payload_from_conn(
            self.conn, "ui.transfers.components.plan", args
        )
        args.update(expected_fingerprint=preview["fingerprint"])
        first = _ui_swap_matching_payload_from_conn(
            self.conn, "ui.transfers.components.apply", args
        )
        with self.assertRaises(AppError) as caught:
            _ui_swap_matching_payload_from_conn(
                self.conn, "ui.transfers.components.apply", args
            )

        self.assertEqual(caught.exception.code, "custody_review_plan_stale")
        self.assertEqual(first["components"][0]["id"], preview["components"][0]["id"])
        self.assertEqual(
            self.conn.execute("SELECT COUNT(*) FROM custody_components").fetchone()[0],
            1,
        )

    def test_bulk_resolution_bounds_legs_per_component(self):
        spec = _component_spec()
        spec["legs"] = [dict(spec["legs"][0]) for _ in range(257)]
        with self.assertRaises(AppError) as caught:
            _ui_swap_matching_payload_from_conn(
                self.conn,
                "ui.transfers.components.plan",
                {
                    "workspace": "Main",
                    "profile": "Book",
                    "components": [spec],
                    "activate": False,
                },
            )

        self.assertEqual(caught.exception.details["max_legs"], 256)

    def test_bulk_preview_performs_no_database_writes(self):
        spec = _component_spec()
        spec["legs"][1] = {
            "role": "retained",
            "untracked_wallet": "Preview-only missing wallet",
            "occurred_at": "2026-06-01T00:00:00Z",
            "amount_msat": 99_000,
        }
        before_changes = self.conn.total_changes
        before_version = self.conn.execute(
            "SELECT journal_input_version FROM profiles WHERE id = 'profile'"
        ).fetchone()[0]

        preview = _ui_swap_matching_payload_from_conn(
            self.conn,
            "ui.transfers.components.plan",
            {
                "workspace": "Main",
                "profile": "Book",
                "components": [spec],
                "activate": False,
            },
        )

        self.assertTrue(preview["dry_run"])
        self.assertEqual(self.conn.total_changes, before_changes)
        self.assertFalse(self.conn.in_transaction)
        self.assertEqual(
            self.conn.execute("SELECT COUNT(*) FROM custody_components").fetchone()[0],
            0,
        )
        self.assertIsNone(
            self.conn.execute(
                "SELECT id FROM wallets WHERE label = 'Preview-only missing wallet'"
            ).fetchone()
        )
        self.assertEqual(
            self.conn.execute(
                "SELECT journal_input_version FROM profiles WHERE id = 'profile'"
            ).fetchone()[0],
            before_version,
        )

    def test_bulk_apply_rejects_a_stale_input_version(self):
        args = {
            "workspace": "Main",
            "profile": "Book",
            "components": [_component_spec()],
            "activate": False,
        }
        preview = _ui_swap_matching_payload_from_conn(
            self.conn, "ui.transfers.components.plan", args
        )
        self.conn.execute(
            "UPDATE profiles SET journal_input_version = journal_input_version + 1 "
            "WHERE id = 'profile'"
        )
        self.conn.commit()
        args.update(expected_fingerprint=preview["fingerprint"])

        with self.assertRaises(AppError) as caught:
            _ui_swap_matching_payload_from_conn(
                self.conn, "ui.transfers.components.apply", args
            )

        self.assertEqual(caught.exception.code, "custody_review_plan_stale")
        self.assertEqual(
            self.conn.execute("SELECT COUNT(*) FROM custody_components").fetchone()[0],
            0,
        )

    def test_active_preview_rejects_cross_component_anchor_conflicts(self):
        first = _component_spec(note="first")
        second = _component_spec(note="second")

        with self.assertRaises(AppError) as caught:
            _ui_swap_matching_payload_from_conn(
                self.conn,
                "ui.transfers.components.plan",
                {
                    "workspace": "Main",
                    "profile": "Book",
                    "components": [first, second],
                    "activate": True,
                },
            )

        self.assertEqual(caught.exception.code, "custody_component_not_activatable")
        self.assertIn(
            "active_transaction_membership_conflict",
            {issue["code"] for issue in caught.exception.details["issues"]},
        )
        self.assertEqual(
            self.conn.execute("SELECT COUNT(*) FROM custody_components").fetchone()[0],
            0,
        )

    def test_preview_applies_database_anchor_coverage_checks(self):
        spec = _component_spec()
        spec["legs"][0]["amount_msat"] = 99_999
        spec["legs"][2]["amount_msat"] = 999

        with self.assertRaises(AppError) as caught:
            _ui_swap_matching_payload_from_conn(
                self.conn,
                "ui.transfers.components.plan",
                {
                    "workspace": "Main",
                    "profile": "Book",
                    "components": [spec],
                    "activate": True,
                },
            )

        self.assertEqual(caught.exception.code, "custody_component_not_activatable")
        self.assertIn("anchor_coverage_mismatch", str(caught.exception.details))
        self.assertEqual(
            self.conn.execute("SELECT COUNT(*) FROM custody_components").fetchone()[0],
            0,
        )

    def test_transaction_and_untracked_wallet_error_names_the_real_conflict(self):
        spec = _component_spec()
        spec["legs"][0]["untracked_wallet"] = "Missing old wallet"

        with self.assertRaises(AppError) as caught:
            _ui_swap_matching_payload_from_conn(
                self.conn,
                "ui.transfers.components.plan",
                {
                    "workspace": "Main",
                    "profile": "Book",
                    "components": [spec],
                    "activate": False,
                },
            )

        self.assertEqual(caught.exception.code, "validation")
        self.assertIn("transaction with untracked_wallet", str(caught.exception))

    def test_full_revision_lifecycle_and_envelope_kinds(self):
        spec_json = json.dumps([_component_spec()])
        preview = _dispatch_json(
            self.conn,
            self.data_root,
            "transfers",
            "components",
            "plan",
            "--action",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--json",
            spec_json,
        )
        created = _dispatch_json(
            self.conn,
            self.data_root,
            "transfers",
            "components",
            "apply",
            "--action",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--json",
            spec_json,
            "--expected-fingerprint",
            preview["data"]["fingerprint"],
        )
        self.assertEqual(created["kind"], "transfers.components.apply")
        created_component = created["data"]["components"][0]
        self.assertEqual(created_component["effective_state"], "active")
        first_id = created_component["id"]

        revision_json = json.dumps({"notes": "reviewed again"})
        revision_plan = _dispatch_json(
            self.conn,
            self.data_root,
            "transfers",
            "components",
            "plan",
            "--action",
            "revise",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--component-id",
            first_id,
            "--json",
            revision_json,
            "--activate",
        )
        revised = _dispatch_json(
            self.conn,
            self.data_root,
            "transfers",
            "components",
            "apply",
            "--action",
            "revise",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--component-id",
            first_id,
            "--json",
            revision_json,
            "--activate",
            "--expected-fingerprint",
            revision_plan["data"]["fingerprint"],
        )
        self.assertEqual(revised["kind"], "transfers.components.apply")
        self.assertEqual(revised["data"]["component"]["revision"], 2)
        self.assertEqual(revised["data"]["component"]["effective_state"], "active")
        second_id = revised["data"]["component"]["id"]

        supersede_plan = _dispatch_json(
            self.conn,
            self.data_root,
            "transfers",
            "components",
            "plan",
            "--action",
            "supersede",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--component-id",
            second_id,
            "--reason",
            "replace evidence",
        )
        superseded = _dispatch_json(
            self.conn,
            self.data_root,
            "transfers",
            "components",
            "apply",
            "--action",
            "supersede",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--component-id",
            second_id,
            "--reason",
            "replace evidence",
            "--expected-fingerprint",
            supersede_plan["data"]["fingerprint"],
        )
        self.assertEqual(superseded["kind"], "transfers.components.apply")
        self.assertEqual(superseded["data"]["component"]["state"], "superseded")

        undo_plan = _dispatch_json(
            self.conn,
            self.data_root,
            "transfers",
            "components",
            "plan",
            "--action",
            "undo",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--component-id",
            second_id,
        )
        restored = _dispatch_json(
            self.conn,
            self.data_root,
            "transfers",
            "components",
            "apply",
            "--action",
            "undo",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--component-id",
            second_id,
            "--expected-fingerprint",
            undo_plan["data"]["fingerprint"],
        )
        self.assertEqual(restored["kind"], "transfers.components.apply")
        self.assertEqual(restored["data"]["component"]["state"], "draft")
        third_id = restored["data"]["component"]["id"]

        activate_plan = _dispatch_json(
            self.conn,
            self.data_root,
            "transfers",
            "components",
            "plan",
            "--action",
            "activate",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--component-id",
            third_id,
        )
        activated = _dispatch_json(
            self.conn,
            self.data_root,
            "transfers",
            "components",
            "apply",
            "--action",
            "activate",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--component-id",
            third_id,
            "--expected-fingerprint",
            activate_plan["data"]["fingerprint"],
        )
        self.assertEqual(activated["kind"], "transfers.components.apply")
        self.assertEqual(activated["data"]["component"]["effective_state"], "active")

        shown = _dispatch_json(
            self.conn,
            self.data_root,
            "transfers",
            "components",
            "show",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--component-id",
            third_id,
        )
        self.assertEqual(shown["kind"], "transfers.components.show")
        self.assertNotIn("evidence", shown["data"])

        listed = _dispatch_json(
            self.conn,
            self.data_root,
            "transfers",
            "components",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--effective-only",
        )
        self.assertEqual(listed["kind"], "transfers.components.list")
        self.assertEqual([item["id"] for item in listed["data"]], [third_id])

    def test_bulk_file_dry_run_is_atomic(self):
        spec_path = Path(self.tmp.name) / "components.json"
        spec_path.write_text(json.dumps([_component_spec()]), encoding="utf-8")
        preview = _dispatch_json(
            self.conn,
            self.data_root,
            "transfers",
            "components",
            "plan",
            "--action",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--file",
            str(spec_path),
            "--draft",
        )
        self.assertEqual(preview["kind"], "transfers.components.plan")
        self.assertTrue(preview["data"]["dry_run"])
        self.assertEqual(preview["data"]["summary"]["draft"], 1)
        count = self.conn.execute(
            "SELECT COUNT(*) AS count FROM custody_components"
        ).fetchone()["count"]
        self.assertEqual(count, 0)

        invalid_path = Path(self.tmp.name) / "invalid-components.json"
        invalid_path.write_text(
            json.dumps([_component_spec(), {"legs": "not-an-array"}]),
            encoding="utf-8",
        )
        with self.assertRaises(AppError):
            _dispatch_json(
                self.conn,
                self.data_root,
                "transfers",
                "components",
                "plan",
                "--action",
                "create",
                "--workspace",
                "Main",
                "--profile",
                "Book",
                "--file",
                str(invalid_path),
                "--draft",
            )
        count = self.conn.execute(
            "SELECT COUNT(*) AS count FROM custody_components"
        ).fetchone()["count"]
        self.assertEqual(count, 0)

    def test_explicit_draft_mode_cannot_be_overridden_by_json(self):
        spec = _component_spec(note="must remain draft")
        spec["activate"] = True
        document = {"activate": True, "components": [spec]}
        spec_path = Path(self.tmp.name) / "authoritative-draft.json"
        spec_path.write_text(json.dumps(document), encoding="utf-8")

        created = _dispatch_json(
            self.conn,
            self.data_root,
            "transfers",
            "components",
            "apply",
            "--action",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--file",
            str(spec_path),
            "--draft",
            "--expected-fingerprint",
            _dispatch_json(
                self.conn,
                self.data_root,
                "transfers",
                "components",
                "plan",
                "--action",
                "create",
                "--workspace",
                "Main",
                "--profile",
                "Book",
                "--file",
                str(spec_path),
                "--draft",
            )["data"]["fingerprint"],
        )

        self.assertEqual(created["data"]["summary"], {"count": 1, "active": 0, "draft": 1})
        self.assertEqual(created["data"]["components"][0]["state"], "draft")

    def test_bulk_can_create_explicit_untracked_wallet_placeholder_atomically(self):
        spec = {
            "component_type": "manual_bridge",
            "evidence_kind": "manual_migration_review",
            "evidence_grade": "reviewed",
            "legs": [
                {
                    "role": "source",
                    "transaction": "external-out",
                    "amount_msat": 100_000,
                },
                {
                    "role": "retained",
                    "untracked_wallet": "Missing migration wallet",
                    "occurred_at": "2026-06-01T00:00:00Z",
                    "amount_msat": 100_000,
                },
            ],
        }
        spec_path = Path(self.tmp.name) / "untracked-components.json"
        spec_path.write_text(json.dumps([spec]), encoding="utf-8")

        preview = _dispatch_json(
            self.conn,
            self.data_root,
            "transfers",
            "components",
            "plan",
            "--action",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--file",
            str(spec_path),
        )
        self.assertTrue(preview["data"]["dry_run"])
        self.assertIsNone(
            self.conn.execute(
                "SELECT id FROM wallets WHERE label = 'Missing migration wallet'"
            ).fetchone()
        )

        created = _dispatch_json(
            self.conn,
            self.data_root,
            "transfers",
            "components",
            "apply",
            "--action",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--file",
            str(spec_path),
            "--expected-fingerprint",
            preview["data"]["fingerprint"],
        )
        self.assertEqual(created["data"]["summary"]["active"], 1)
        wallet = self.conn.execute(
            "SELECT id, kind FROM wallets WHERE label = 'Missing migration wallet'"
        ).fetchone()
        self.assertIsNotNone(wallet)
        self.assertEqual(wallet["kind"], "untracked")
        leg = self.conn.execute(
            "SELECT wallet_id FROM custody_component_legs WHERE role = 'retained'"
        ).fetchone()
        self.assertEqual(leg["wallet_id"], wallet["id"])

    def test_failed_single_plan_never_writes_untracked_wallet_placeholder(self):
        invalid = {
            "component_type": "manual_bridge",
            "evidence_kind": "manual_migration_review",
            "evidence_grade": "reviewed",
            "legs": [
                {
                    "role": "source",
                    "transaction": "external-out",
                    "amount_msat": 100_000,
                },
                {
                    "role": "retained",
                    "untracked_wallet": "Failed create placeholder",
                    "occurred_at": "2021-06-01T00:00:00Z",
                    "amount_msat": 90_000,
                },
            ],
        }
        spec_path = Path(self.tmp.name) / "invalid-single-component.json"
        spec_path.write_text(json.dumps([invalid]), encoding="utf-8")

        with self.assertRaises(AppError):
            _dispatch_json(
                self.conn,
                self.data_root,
                "transfers",
                "components",
                "plan",
            "--action",
            "create",
                "--workspace",
                "Main",
                "--profile",
                "Book",
                "--file",
                str(spec_path),
            )

        self.assertIsNone(
            self.conn.execute(
                "SELECT id FROM wallets WHERE label = 'Failed create placeholder'"
            ).fetchone()
        )
        self.assertEqual(
            0,
            self.conn.execute(
                "SELECT COUNT(*) AS count FROM custody_components"
            ).fetchone()["count"],
        )

    def test_failed_revision_rolls_back_untracked_wallet_placeholder(self):
        initial_path = Path(self.tmp.name) / "initial-component.json"
        initial_path.write_text(json.dumps([_component_spec()]), encoding="utf-8")
        preview = _dispatch_json(
            self.conn,
            self.data_root,
            "transfers",
            "components",
            "plan",
            "--action",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--file",
            str(initial_path),
            "--draft",
        )["data"]
        initial = _dispatch_json(
            self.conn,
            self.data_root,
            "transfers",
            "components",
            "apply",
            "--action",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--file",
            str(initial_path),
            "--expected-fingerprint",
            preview["fingerprint"],
            "--draft",
        )["data"]["components"][0]

        invalid_revision = {
            "legs": [
                {
                    "role": "source",
                    "transaction": "external-out",
                    "amount_msat": 100_000,
                },
                {
                    "role": "retained",
                    "untracked_wallet": "Failed revision placeholder",
                    "occurred_at": "2021-06-01T00:00:00Z",
                    "amount_msat": 90_000,
                },
            ]
        }
        revision_path = Path(self.tmp.name) / "invalid-revision.json"
        revision_path.write_text(json.dumps(invalid_revision), encoding="utf-8")
        with self.assertRaises(AppError):
            _dispatch_json(
                self.conn,
                self.data_root,
                "transfers",
                "components",
                "plan",
                "--action",
                "revise",
                "--workspace",
                "Main",
                "--profile",
                "Book",
                "--component-id",
                initial["id"],
                "--file",
                str(revision_path),
                "--activate",
            )

        self.assertIsNone(
            self.conn.execute(
                "SELECT id FROM wallets WHERE label = 'Failed revision placeholder'"
            ).fetchone()
        )
        rows = self.conn.execute(
            "SELECT id, state FROM custody_components ORDER BY revision"
        ).fetchall()
        self.assertEqual([(initial["id"], "draft")], [(row["id"], row["state"]) for row in rows])


class CustodyComponentDaemonSurfaceTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-component-daemon-")
        self.addCleanup(self.tmp.cleanup)
        self.data_root = Path(self.tmp.name) / "data"
        _fixture(self.data_root)

    def _with_daemon(self, callback) -> None:
        proc = _start_daemon(self.data_root)
        try:
            callback(proc)
        finally:
            if proc.stdin is not None:
                proc.stdin.close()
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()

    def test_supported_kinds_and_daemon_round_trip(self):
        self.assertTrue(COMPONENT_KINDS.issubset(set(SUPPORTED_KINDS)))

        def exercise(proc):
            component_spec = _component_spec()
            component_spec["evidence"] = {"private": "local-only"}
            component_spec["conversion_metadata"] = {"private": "local-only"}
            component_spec["legs"][0]["location_ref"] = "/private/node/channel"
            preview = _request(
                proc,
                {
                    "kind": "ui.transfers.components.plan",
                    "request_id": "component-create-plan",
                    "args": {
                        "workspace": "Main",
                        "profile": "Book",
                        "components": [component_spec],
                        "activate": True,
                    },
                },
            )
            created = _request(
                proc,
                {
                    "kind": "ui.transfers.components.apply",
                    "request_id": "component-create-apply",
                    "args": {
                        "workspace": "Main",
                        "profile": "Book",
                        "components": [component_spec],
                        "activate": True,
                        "expected_fingerprint": preview["data"]["fingerprint"],
                    },
                },
            )
            self.assertEqual(created["kind"], "ui.transfers.components.apply")
            created_component = created["data"]["components"][0]
            self.assertEqual(created_component["effective_state"], "active")
            self.assertIsInstance(created_component["legs"][0]["amount_msat"], int)
            self.assertNotIn("evidence", created_component)
            self.assertNotIn("conversion_metadata", created_component)
            self.assertTrue(
                all("location_ref" not in leg for leg in created_component["legs"])
            )
            component_id = created_component["id"]

            revision_args = {
                "workspace": "Main",
                "profile": "Book",
                "action": "revise",
                "component_id": component_id,
                "spec": {
                    "notes": "daemon revision",
                    "legs": created_component["legs"],
                    "allocations": created_component["allocations"],
                },
                "activate": True,
            }
            revision_plan = _request(
                proc,
                {
                    "kind": "ui.transfers.components.plan",
                    "request_id": "component-revision-plan",
                    "args": revision_args,
                },
            )
            revised = _request(
                proc,
                {
                    "kind": "ui.transfers.components.apply",
                    "request_id": "component-revision-apply",
                    "args": {
                        **revision_args,
                        "expected_fingerprint": revision_plan["data"]["fingerprint"],
                    },
                },
            )
            self.assertEqual(revised["data"]["component"]["revision"], 2)
            component_id = revised["data"]["component"]["id"]

            supersede_args = {
                "workspace": "Main",
                "profile": "Book",
                "action": "supersede",
                "component_id": component_id,
                "reason": "exercise immutable undo",
            }
            supersede_plan = _request(
                proc,
                {
                    "kind": "ui.transfers.components.plan",
                    "request_id": "component-supersede-plan",
                    "args": supersede_args,
                },
            )
            superseded = _request(
                proc,
                {
                    "kind": "ui.transfers.components.apply",
                    "request_id": "component-supersede-apply",
                    "args": {
                        **supersede_args,
                        "expected_fingerprint": supersede_plan["data"]["fingerprint"],
                    },
                },
            )
            self.assertEqual(superseded["data"]["component"]["state"], "superseded")

            undo_args = {
                "workspace": "Main",
                "profile": "Book",
                "action": "undo",
                "component_id": component_id,
                "reason": "undo",
            }
            undo_plan = _request(
                proc,
                {
                    "kind": "ui.transfers.components.plan",
                    "request_id": "component-undo-plan",
                    "args": undo_args,
                },
            )
            restored = _request(
                proc,
                {
                    "kind": "ui.transfers.components.apply",
                    "request_id": "component-undo-apply",
                    "args": {
                        **undo_args,
                        "expected_fingerprint": undo_plan["data"]["fingerprint"],
                    },
                },
            )
            self.assertEqual(restored["data"]["component"]["state"], "draft")
            component_id = restored["data"]["component"]["id"]

            activate_args = {
                "workspace": "Main",
                "profile": "Book",
                "action": "activate",
                "component_id": component_id,
            }
            activate_plan = _request(
                proc,
                {
                    "kind": "ui.transfers.components.plan",
                    "request_id": "component-activate-plan",
                    "args": activate_args,
                },
            )
            activated = _request(
                proc,
                {
                    "kind": "ui.transfers.components.apply",
                    "request_id": "component-activate-apply",
                    "args": {
                        **activate_args,
                        "expected_fingerprint": activate_plan["data"]["fingerprint"],
                    },
                },
            )
            self.assertEqual(
                activated["data"]["component"]["effective_state"], "active"
            )

            fetched = _request(
                proc,
                {
                    "kind": "ui.transfers.components.get",
                    "request_id": "component-get",
                    "args": {
                        "workspace": "Main",
                        "profile": "Book",
                        "component_id": component_id,
                    },
                },
            )
            self.assertEqual(fetched["data"]["id"], component_id)
            self.assertEqual(fetched["data"]["authored_source"], "gui")
            self.assertNotIn("evidence", fetched["data"])

            listed = _request(
                proc,
                {
                    "kind": "ui.transfers.components.list",
                    "request_id": "component-list",
                    "args": {
                        "workspace": "Main",
                        "profile": "Book",
                        "effective_only": True,
                    },
                },
            )
            self.assertEqual(
                [item["id"] for item in listed["data"]["components"]],
                [component_id],
            )
            self.assertFalse(listed["data"]["has_more"])
            self.assertEqual(listed["data"]["limit"], 200)

            invalid = _request(
                proc,
                {
                    "kind": "ui.transfers.components.plan",
                    "request_id": "component-invalid",
                    "args": {
                        "workspace": "Main",
                        "profile": "Book",
                        "action": "revise",
                        "component_id": component_id,
                        "spec": [],
                    },
                },
            )
            self.assertEqual(invalid["kind"], "error")
            self.assertEqual(invalid["error"]["code"], "validation")

            # A draft preview can reuse the same evidence anchors because it is
            # rolled back before membership activation.
            preview = _request(
                proc,
                {
                    "kind": "ui.transfers.components.plan",
                    "request_id": "component-preview",
                    "args": {
                        "workspace": "Main",
                        "profile": "Book",
                        "components": [_component_spec(note="preview")],
                        "activate": False,
                    },
                },
            )
            self.assertTrue(preview["data"]["dry_run"])
            self.assertEqual(preview["data"]["summary"]["draft"], 1)

            after = _request(
                proc,
                {
                    "kind": "ui.transfers.components.list",
                    "request_id": "component-list-after",
                    "args": {"workspace": "Main", "profile": "Book"},
                },
            )
            self.assertEqual(len(after["data"]["components"]), 3)

        self._with_daemon(exercise)
        conn = open_db(str(self.data_root))
        try:
            latest_location = conn.execute(
                """
                SELECT l.location_ref
                FROM custody_component_legs l
                JOIN custody_components c ON c.id = l.component_id
                WHERE l.role = 'source'
                ORDER BY c.revision DESC
                LIMIT 1
                """
            ).fetchone()
            self.assertEqual("/private/node/channel", latest_location["location_ref"])
        finally:
            conn.close()

    def test_daemon_round_trips_unsafe_component_integers_as_strings(self):
        exact = "9007199254740993"

        def exercise(proc):
            spec = {
                "component_type": "swap",
                "conservation_mode": "conversion",
                "conversion_policy": "reviewed_exact_value",
                "conversion_reviewed": True,
                "legs": [
                    {
                        "id": "large-source",
                        "role": "source",
                        "wallet": "Source",
                        "occurred_at": NOW,
                        "amount_msat": exact,
                        "valuation_unit": "eur-cent",
                        "valuation_amount": exact,
                    },
                    {
                        "id": "large-destination",
                        "role": "destination",
                        "wallet": "Sink",
                        "occurred_at": NOW,
                        "amount_msat": exact,
                        "valuation_unit": "eur-cent",
                        "valuation_amount": exact,
                    },
                ],
                "allocations": [
                    {
                        "source_leg_id": "large-source",
                        "sink_leg_id": "large-destination",
                        "source_amount_msat": exact,
                        "sink_amount_msat": exact,
                    }
                ],
            }
            preview = _request(
                proc,
                {
                    "kind": "ui.transfers.components.plan",
                    "request_id": "unsafe-integer-create-plan",
                    "args": {
                        "workspace": "Main",
                        "profile": "Book",
                        "components": [spec],
                        "activate": False,
                    },
                },
            )
            created = _request(
                proc,
                {
                    "kind": "ui.transfers.components.apply",
                    "request_id": "unsafe-integer-create-apply",
                    "args": {
                        "workspace": "Main",
                        "profile": "Book",
                        "components": [spec],
                        "activate": False,
                        "expected_fingerprint": preview["data"]["fingerprint"],
                    },
                },
            )
            self.assertEqual(created["kind"], "ui.transfers.components.apply")
            created_component = created["data"]["components"][0]
            for leg in created_component["legs"]:
                self.assertEqual(exact, leg["amount_msat"])
                self.assertEqual(exact, leg["valuation_amount"])
            allocation = created_component["allocations"][0]
            self.assertEqual(exact, allocation["source_amount_msat"])
            self.assertEqual(exact, allocation["sink_amount_msat"])

            revision_args = {
                "workspace": "Main",
                "profile": "Book",
                "action": "revise",
                "component_id": created_component["id"],
                "spec": {
                    "notes": "exact renderer revision",
                    "legs": created_component["legs"],
                    "allocations": created_component["allocations"],
                },
                "activate": False,
            }
            revision_plan = _request(
                proc,
                {
                    "kind": "ui.transfers.components.plan",
                    "request_id": "unsafe-integer-revision-plan",
                    "args": revision_args,
                },
            )
            revised = _request(
                proc,
                {
                    "kind": "ui.transfers.components.apply",
                    "request_id": "unsafe-integer-revision-apply",
                    "args": {
                        **revision_args,
                        "expected_fingerprint": revision_plan["data"]["fingerprint"],
                    },
                },
            )
            self.assertEqual(
                revised["kind"], "ui.transfers.components.apply", revised
            )
            revised_component = revised["data"]["component"]
            self.assertEqual(revised_component["revision"], 2)
            self.assertEqual(
                exact, revised_component["legs"][0]["amount_msat"]
            )
            self.assertEqual(
                exact,
                revised_component["allocations"][0]["source_amount_msat"],
            )

            listed = _request(
                proc,
                {
                    "kind": "ui.transfers.components.list",
                    "request_id": "unsafe-integer-list",
                    "args": {"workspace": "Main", "profile": "Book"},
                },
            )
            latest = next(
                item
                for item in listed["data"]["components"]
                if item["id"] == revised_component["id"]
            )
            self.assertEqual(exact, latest["legs"][0]["valuation_amount"])
            self.assertEqual(
                exact, latest["allocations"][0]["sink_amount_msat"]
            )

        self._with_daemon(exercise)


if __name__ == "__main__":
    unittest.main()
