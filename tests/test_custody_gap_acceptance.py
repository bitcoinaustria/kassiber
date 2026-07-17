"""Black-box acceptance lanes for the guided custody-gap workflow.

The CLI and daemon tests deliberately seed only normal wallet/transaction rows.
Reviewers identify a deterministic gap, preview the server-authored bridge, and
confirm it with the returned fingerprint; no component JSON is accepted or sent
through either public workflow.
"""

from __future__ import annotations

import json
import select
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

from kassiber.db import open_db


ROOT = Path(__file__).resolve().parent.parent
BTC = 100_000_000_000


def _seed_gap(data_root: Path) -> None:
    conn = open_db(str(data_root))
    try:
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) "
            "VALUES('ws', 'Books', '2020-01-01T00:00:00Z')"
        )
        conn.execute(
            "INSERT INTO profiles(id, workspace_id, label, created_at) "
            "VALUES('profile', 'ws', 'Book', '2020-01-01T00:00:00Z')"
        )
        for wallet_id, label in (("old", "Old vault"), ("new", "New vault")):
            conn.execute(
                """
                INSERT INTO wallets(
                    id, workspace_id, profile_id, label, kind, config_json,
                    created_at
                ) VALUES(?, 'ws', 'profile', ?, 'descriptor',
                         '{"chain":"bitcoin","network":"main"}',
                         '2020-01-01T00:00:00Z')
                """,
                (wallet_id, label),
            )
        for values in (
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
        ):
            conn.execute(
                """
                INSERT INTO transactions(
                    id, workspace_id, profile_id, wallet_id, external_id,
                    fingerprint, occurred_at, direction, asset, amount, fee,
                    privacy_boundary, raw_json, created_at
                ) VALUES(?, 'ws', 'profile', ?, ?, ?, ?, ?, 'BTC', ?, ?, ?,
                         '{}', ?)
                """,
                (
                    values[0],
                    values[1],
                    values[0],
                    f"fingerprint-{values[0]}",
                    values[5],
                    values[2],
                    values[3],
                    values[4],
                    values[6],
                    values[5],
                ),
            )
        conn.commit()
    finally:
        conn.close()


def _run_cli(data_root: Path, *args: str) -> dict:
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
    if result.returncode != 0:
        raise AssertionError(
            f"CLI {args!r} exited {result.returncode}: "
            f"{result.stdout}\n{result.stderr}"
        )
    return json.loads(result.stdout)


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
    stderr = proc.stderr.read() if proc.poll() is not None and proc.stderr else ""
    raise AssertionError(
        f"daemon did not answer {payload['request_id']!r}: {stderr}"
    )


def _stop_daemon(proc: subprocess.Popen[str]) -> None:
    if proc.stdin is not None:
        proc.stdin.close()
    try:
        proc.wait(timeout=5.0)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=5.0)
    if proc.stdout is not None:
        proc.stdout.close()
    if proc.stderr is not None:
        proc.stderr.close()


class CustodyGapCliAcceptanceTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-gap-cli-")
        self.addCleanup(self.tmp.cleanup)
        self.data_root = Path(self.tmp.name) / "data"
        _seed_gap(self.data_root)

    @staticmethod
    def _scope() -> tuple[str, ...]:
        return ("--workspace", "Books", "--profile", "Book")

    def test_guided_cli_lifecycle_never_requires_component_json(self) -> None:
        listed = _run_cli(
            self.data_root,
            "transfers",
            "gaps",
            "list",
            "--limit",
            "1",
            "--cursor",
            "0",
            *self._scope(),
        )
        gap = listed["data"]["gaps"][0]
        gap_id = gap["gap_id"]

        reviewed = _run_cli(
            self.data_root,
            "transfers",
            "gaps",
            "review",
            "--gap-id",
            gap_id,
            *self._scope(),
        )
        self.assertEqual(reviewed["data"]["gaps"][0]["gap_id"], gap_id)

        previewed = _run_cli(
            self.data_root,
            "transfers",
            "gaps",
            "plan",
            "--action",
            "create",
            "--gap-id",
            gap_id,
            *self._scope(),
        )
        preview = previewed["data"]
        self.assertTrue(preview["dry_run"])
        self.assertTrue(preview["activatable"])

        created = _run_cli(
            self.data_root,
            "transfers",
            "gaps",
            "apply",
            "--action",
            "create",
            "--gap-id",
            gap_id,
            "--expected-input-version",
            str(preview["input_version"]),
            *self._scope(),
        )["data"]
        self.assertEqual(created["status"], "resolved")

        supersede_plan = _run_cli(
            self.data_root,
            "transfers",
            "components",
            "plan",
            "--action",
            "supersede",
            "--component-id",
            created["component_id"],
            "--reason",
            "review correction",
            *self._scope(),
        )["data"]
        superseded = _run_cli(
            self.data_root,
            "transfers",
            "components",
            "apply",
            "--action",
            "supersede",
            "--component-id",
            created["component_id"],
            "--reason",
            "review correction",
            "--expected-input-version",
            str(supersede_plan["input_version"]),
            *self._scope(),
        )["data"]["component"]
        self.assertEqual(superseded["state"], "superseded")

        undo_plan = _run_cli(
            self.data_root,
            "transfers",
            "components",
            "plan",
            "--action",
            "undo",
            "--component-id",
            created["component_id"],
            "--reason",
            "restore for revision",
            *self._scope(),
        )["data"]
        revised = _run_cli(
            self.data_root,
            "transfers",
            "components",
            "apply",
            "--action",
            "undo",
            "--component-id",
            created["component_id"],
            "--reason",
            "restore for revision",
            "--expected-input-version",
            str(undo_plan["input_version"]),
            *self._scope(),
        )["data"]["component"]
        self.assertEqual(revised["state"], "draft")
        self.assertGreater(revised["revision"], superseded["revision"])

        public_payload = json.dumps(
            [listed, reviewed, previewed, created, superseded, revised]
        )
        self.assertNotIn("raw_json", public_payload)
        self.assertNotIn("source_ids", public_payload)

    def test_guided_cli_dismissal_uses_only_gap_identity(self) -> None:
        gap = _run_cli(
            self.data_root, "transfers", "gaps", "list", *self._scope()
        )["data"]["gaps"][0]
        plan = _run_cli(
            self.data_root,
            "transfers",
            "gaps",
            "plan",
            "--action",
            "dismiss",
            "--gap-id",
            gap["gap_id"],
            "--reason",
            "known external disposal",
            *self._scope(),
        )["data"]
        dismissed = _run_cli(
            self.data_root,
            "transfers",
            "gaps",
            "apply",
            "--action",
            "dismiss",
            "--gap-id",
            gap["gap_id"],
            "--expected-input-version",
            str(plan["input_version"]),
            "--reason",
            "known external disposal",
            *self._scope(),
        )["data"]
        self.assertEqual(dismissed["action"], "dismissed")
        self.assertNotIn("raw_json", json.dumps(dismissed))


class CustodyGapDaemonProtocolAcceptanceTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-gap-daemon-")
        self.addCleanup(self.tmp.cleanup)
        self.data_root = Path(self.tmp.name) / "data"
        _seed_gap(self.data_root)

    def test_real_jsonl_protocol_accepts_opaque_candidate_cursor(self) -> None:
        conn = open_db(str(self.data_root))
        try:
            conn.execute(
                """
                INSERT INTO transactions(
                    id, workspace_id, profile_id, wallet_id, external_id,
                    fingerprint, occurred_at, direction, asset, amount, fee,
                    raw_json, created_at
                ) VALUES(
                    'return-two', 'ws', 'profile', 'new', 'return-two',
                    'fingerprint-return-two', '2021-02-01T00:00:00Z',
                    'inbound', 'BTC', ?, 0, '{}', '2021-02-01T00:00:00Z'
                )
                """,
                (99 * BTC // 10,),
            )
            conn.commit()
        finally:
            conn.close()

        proc = _start_daemon(self.data_root)
        try:
            scope = {"workspace": "Books", "profile": "Book"}
            first = _request(
                proc,
                {
                    "request_id": "gap-page-one",
                    "kind": "ui.custody.gaps.list",
                    "args": {**scope, "limit": 1},
                },
            )["data"]
            self.assertRegex(first["next_cursor"], r"^cgr3\.")
            second = _request(
                proc,
                {
                    "request_id": "gap-page-two",
                    "kind": "ui.custody.gaps.list",
                    "args": {
                        **scope,
                        "limit": 1,
                        "cursor": first["next_cursor"],
                    },
                },
            )["data"]
            self.assertNotEqual(
                first["gaps"][0]["gap_id"], second["gaps"][0]["gap_id"]
            )
            self.assertIsNone(second["next_cursor"])
        finally:
            _stop_daemon(proc)

    def test_real_jsonl_protocol_previews_confirms_and_refreshes_gap(self) -> None:
        proc = _start_daemon(self.data_root)
        try:
            scope = {"workspace": "Books", "profile": "Book"}
            listed = _request(
                proc,
                {
                    "request_id": "gap-list",
                    "kind": "ui.custody.gaps.list",
                    "args": scope,
                },
            )
            gap = listed["data"]["gaps"][0]
            preview_args = {**scope, "action": "create", "gap_id": gap["gap_id"]}
            preview = _request(
                proc,
                {
                    "request_id": "gap-preview",
                    "kind": "ui.custody.review.plan",
                    "args": preview_args,
                },
            )["data"]
            self.assertTrue(preview["activatable"])

            create_args = {
                **preview_args,
                "expected_input_version": preview["input_version"],
            }
            created = _request(
                proc,
                {
                    "request_id": "gap-create",
                    "kind": "ui.custody.review.apply",
                    "args": create_args,
                },
            )["data"]
            self.assertEqual(created["status"], "resolved")

            refreshed = _request(
                proc,
                {
                    "request_id": "gap-refresh",
                    "kind": "ui.custody.gaps.review_context",
                    "args": {**scope, "gap_id": gap["gap_id"]},
                },
            )["data"]
            self.assertEqual(refreshed["gaps"][0]["status"], "resolved")
            self.assertNotIn(
                "raw_json",
                json.dumps([listed, preview, created, refreshed]),
            )
            self.assertEqual(
                set(preview_args), {"workspace", "profile", "action", "gap_id"}
            )
            self.assertEqual(
                set(create_args),
                {
                    "workspace",
                    "profile",
                    "action",
                    "gap_id",
                    "expected_input_version",
                },
            )
        finally:
            _stop_daemon(proc)


if __name__ == "__main__":
    unittest.main()
