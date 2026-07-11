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
from kassiber.daemon import SUPPORTED_KINDS, _ui_swap_matching_payload_from_conn
from kassiber.db import open_db
from kassiber.errors import AppError


ROOT = Path(__file__).resolve().parent.parent
NOW = "2026-01-01T00:00:00Z"
COMPONENT_KINDS = {
    "ui.transfers.components.list",
    "ui.transfers.components.get",
    "ui.transfers.components.create",
    "ui.transfers.components.update",
    "ui.transfers.components.activate",
    "ui.transfers.components.supersede",
    "ui.transfers.components.undo",
    "ui.transfers.components.bulk_resolve",
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
        result = _ui_swap_matching_payload_from_conn(
            self.conn,
            "ui.transfers.components.bulk_resolve",
            {
                "workspace": "Main",
                "profile": "Book",
                "components": [_component_spec()],
                "activate": False,
                "dry_run": False,
            },
            authored_source="ai_tool",
        )

        self.assertEqual(result["components"][0]["authored_source"], "ai_tool")
        stored = self.conn.execute(
            "SELECT authored_source FROM custody_components WHERE id = ?",
            (result["components"][0]["id"],),
        ).fetchone()
        self.assertEqual(stored["authored_source"], "ai_tool")

    def test_daemon_bulk_resolution_rejects_unbounded_batches(self):
        with self.assertRaises(AppError) as caught:
            _ui_swap_matching_payload_from_conn(
                self.conn,
                "ui.transfers.components.bulk_resolve",
                {
                    "workspace": "Main",
                    "profile": "Book",
                    "components": [_component_spec() for _ in range(51)],
                    "activate": False,
                    "dry_run": True,
                },
            )

        self.assertEqual(caught.exception.code, "validation")
        self.assertEqual(caught.exception.details["max_components"], 50)

    def test_transaction_and_untracked_wallet_error_names_the_real_conflict(self):
        spec = _component_spec()
        spec["legs"][0]["untracked_wallet"] = "Missing old wallet"

        with self.assertRaises(AppError) as caught:
            _ui_swap_matching_payload_from_conn(
                self.conn,
                "ui.transfers.components.bulk_resolve",
                {
                    "workspace": "Main",
                    "profile": "Book",
                    "components": [spec],
                    "activate": False,
                    "dry_run": True,
                },
            )

        self.assertEqual(caught.exception.code, "validation")
        self.assertIn("transaction with untracked_wallet", str(caught.exception))

    def test_full_revision_lifecycle_and_envelope_kinds(self):
        created = _dispatch_json(
            self.conn,
            self.data_root,
            "transfers",
            "components",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--json",
            json.dumps(_component_spec()),
            "--activate",
        )
        self.assertEqual(created["kind"], "transfers.components.create")
        self.assertEqual(created["data"]["effective_state"], "active")
        first_id = created["data"]["id"]

        revised = _dispatch_json(
            self.conn,
            self.data_root,
            "transfers",
            "components",
            "update",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--component-id",
            first_id,
            "--json",
            json.dumps({"notes": "reviewed again"}),
            "--activate",
        )
        self.assertEqual(revised["kind"], "transfers.components.update")
        self.assertEqual(revised["data"]["revision"], 2)
        self.assertEqual(revised["data"]["effective_state"], "active")
        second_id = revised["data"]["id"]

        superseded = _dispatch_json(
            self.conn,
            self.data_root,
            "transfers",
            "components",
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
        self.assertEqual(superseded["kind"], "transfers.components.supersede")
        self.assertEqual(superseded["data"]["state"], "superseded")

        restored = _dispatch_json(
            self.conn,
            self.data_root,
            "transfers",
            "components",
            "undo",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--component-id",
            second_id,
        )
        self.assertEqual(restored["kind"], "transfers.components.undo")
        self.assertEqual(restored["data"]["state"], "draft")
        third_id = restored["data"]["id"]

        activated = _dispatch_json(
            self.conn,
            self.data_root,
            "transfers",
            "components",
            "activate",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--component-id",
            third_id,
        )
        self.assertEqual(activated["kind"], "transfers.components.activate")
        self.assertEqual(activated["data"]["effective_state"], "active")

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
            "bulk-resolve",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--file",
            str(spec_path),
            "--draft",
            "--dry-run",
        )
        self.assertEqual(preview["kind"], "transfers.components.bulk-resolve")
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
                "bulk-resolve",
                "--workspace",
                "Main",
                "--profile",
                "Book",
                "--file",
                str(invalid_path),
                "--draft",
                "--dry-run",
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
            "bulk-resolve",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--file",
            str(spec_path),
            "--draft",
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
            "bulk-resolve",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--file",
            str(spec_path),
            "--dry-run",
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
            "bulk-resolve",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--file",
            str(spec_path),
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

    def test_failed_single_create_rolls_back_untracked_wallet_placeholder(self):
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
        spec_path.write_text(json.dumps(invalid), encoding="utf-8")

        with self.assertRaises(AppError):
            _dispatch_json(
                self.conn,
                self.data_root,
                "transfers",
                "components",
                "create",
                "--workspace",
                "Main",
                "--profile",
                "Book",
                "--file",
                str(spec_path),
                "--activate",
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
        initial_path.write_text(json.dumps(_component_spec()), encoding="utf-8")
        initial = _dispatch_json(
            self.conn,
            self.data_root,
            "transfers",
            "components",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--file",
            str(initial_path),
        )["data"]

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
                "update",
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
            created = _request(
                proc,
                {
                    "kind": "ui.transfers.components.create",
                    "request_id": "component-create",
                    "args": {
                        "workspace": "Main",
                        "profile": "Book",
                        "spec": component_spec,
                        "activate": True,
                    },
                },
            )
            self.assertEqual(created["kind"], "ui.transfers.components.create")
            self.assertEqual(created["data"]["effective_state"], "active")
            self.assertIsInstance(created["data"]["legs"][0]["amount_msat"], int)
            self.assertNotIn("evidence", created["data"])
            self.assertNotIn("conversion_metadata", created["data"])
            self.assertTrue(
                all("location_ref" not in leg for leg in created["data"]["legs"])
            )
            component_id = created["data"]["id"]

            revised = _request(
                proc,
                {
                    "kind": "ui.transfers.components.update",
                    "request_id": "component-update",
                    "args": {
                        "workspace": "Main",
                        "profile": "Book",
                        "component_id": component_id,
                        "spec": {
                            "notes": "daemon revision",
                            "legs": created["data"]["legs"],
                            "allocations": created["data"]["allocations"],
                        },
                        "activate": True,
                    },
                },
            )
            self.assertEqual(revised["data"]["revision"], 2)
            component_id = revised["data"]["id"]

            superseded = _request(
                proc,
                {
                    "kind": "ui.transfers.components.supersede",
                    "request_id": "component-supersede",
                    "args": {
                        "workspace": "Main",
                        "profile": "Book",
                        "component_id": component_id,
                        "reason": "exercise immutable undo",
                    },
                },
            )
            self.assertEqual(superseded["data"]["state"], "superseded")

            restored = _request(
                proc,
                {
                    "kind": "ui.transfers.components.undo",
                    "request_id": "component-undo",
                    "args": {
                        "workspace": "Main",
                        "profile": "Book",
                        "component_id": component_id,
                    },
                },
            )
            self.assertEqual(restored["data"]["state"], "draft")
            component_id = restored["data"]["id"]

            activated = _request(
                proc,
                {
                    "kind": "ui.transfers.components.activate",
                    "request_id": "component-activate",
                    "args": {
                        "workspace": "Main",
                        "profile": "Book",
                        "component_id": component_id,
                    },
                },
            )
            self.assertEqual(activated["data"]["effective_state"], "active")

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
                    "kind": "ui.transfers.components.update",
                    "request_id": "component-invalid",
                    "args": {
                        "workspace": "Main",
                        "profile": "Book",
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
                    "kind": "ui.transfers.components.bulk_resolve",
                    "request_id": "component-preview",
                    "args": {
                        "workspace": "Main",
                        "profile": "Book",
                        "components": [_component_spec(note="preview")],
                        "activate": False,
                        "dry_run": True,
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
            created = _request(
                proc,
                {
                    "kind": "ui.transfers.components.create",
                    "request_id": "unsafe-integer-create",
                    "args": {
                        "workspace": "Main",
                        "profile": "Book",
                        "spec": spec,
                        "activate": False,
                    },
                },
            )
            self.assertEqual(created["kind"], "ui.transfers.components.create")
            for leg in created["data"]["legs"]:
                self.assertEqual(exact, leg["amount_msat"])
                self.assertEqual(exact, leg["valuation_amount"])
            allocation = created["data"]["allocations"][0]
            self.assertEqual(exact, allocation["source_amount_msat"])
            self.assertEqual(exact, allocation["sink_amount_msat"])

            revised = _request(
                proc,
                {
                    "kind": "ui.transfers.components.update",
                    "request_id": "unsafe-integer-update",
                    "args": {
                        "workspace": "Main",
                        "profile": "Book",
                        "component_id": created["data"]["id"],
                        "spec": {
                            "notes": "exact renderer revision",
                            "legs": created["data"]["legs"],
                            "allocations": created["data"]["allocations"],
                        },
                        "activate": False,
                    },
                },
            )
            self.assertEqual(revised["data"]["revision"], 2)
            self.assertEqual(exact, revised["data"]["legs"][0]["amount_msat"])
            self.assertEqual(
                exact,
                revised["data"]["allocations"][0]["source_amount_msat"],
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
                if item["id"] == revised["data"]["id"]
            )
            self.assertEqual(exact, latest["legs"][0]["valuation_amount"])
            self.assertEqual(
                exact, latest["allocations"][0]["sink_amount_msat"]
            )

        self._with_daemon(exercise)


if __name__ == "__main__":
    unittest.main()
