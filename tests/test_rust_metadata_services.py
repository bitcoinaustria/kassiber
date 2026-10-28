"""Partial R5 metadata boundary: local edits and their durable audit effects."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import unittest

from kassiber.db import open_db
from tests import cli_runner
from tests import test_rust_transaction_services as transaction_services

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the test-harness binary")
class RustMetadataServicesTest(unittest.TestCase):
    def setUp(self):
        transaction_services.RustTransactionServicesTest.setUp(self)

    cli = transaction_services.RustTransactionServicesTest.cli
    ok = transaction_services.RustTransactionServicesTest.ok
    same_read = transaction_services.RustTransactionServicesTest.same_read

    def test_all_registered_metadata_commands_round_trip(self):
        self.ok(self.cli("metadata", "tags", "create", "--code", "receipt", "--label", "Receipt"))
        self.ok(self.cli("metadata", "tags", "list"))
        for family in (("notes",), ("records", "note")):
            self.ok(self.cli("metadata", *family, "set", "--transaction", "in", "--note", "Invoice \"42\"\n€", "--reason", "matched receipt"))
            self.ok(self.cli("metadata", *family, "clear", "--transaction", "in"))
        for family in (("tags",), ("records", "tag")):
            self.ok(self.cli("metadata", *family, "add", "--transaction", "in", "--tag", "receipt"))
            self.ok(self.cli("metadata", *family, "remove", "--transaction", "in", "--tag", "receipt"))
        for command in (("exclude",), ("include",), ("records", "excluded", "set"), ("records", "excluded", "clear")):
            self.ok(self.cli("metadata", *command, "--transaction", "in"))
        changed = self.ok(self.cli("metadata", "records", "kind", "set", "--transaction", "in", "--kind", "income"))
        self.assertEqual(changed["kind"], "deposit")
        self.assertEqual(changed["kind_override"], "income")
        self.same_read("metadata", "records", "get", "--transaction", "in")
        self.ok(self.cli("metadata", "records", "kind", "clear", "--transaction", "in"))
        self.ok(self.cli("metadata", "records", "get", "--transaction", "in"))
        self.ok(self.cli("metadata", "records", "history", "stale"))
        self.same_read("metadata", "tags", "list")
        self.same_read("metadata", "records", "history", "stale")
        self.same_read("metadata", "records", "kind", "set", "--transaction", "out", "--kind", "income")
        self.same_read("metadata", "tags", "create", "--code", "RECEIPT", "--label", "Duplicate")
        self.same_read("metadata", "records", "get", "--transaction", "absent")

    def test_reference_and_rust_write_identical_history_values(self):
        reference_root = self.base / "reference" / "data"
        shutil.copytree(self.root, reference_root)
        commands = [
            ("tags", "create", "--code", "different", "--label", " ΟΣ Team "),
            ("tags", "add", "--transaction", "in", "--tag", "different"),
            ("tags", "add", "--transaction", "in", "--tag", "different"),
            ("notes", "set", "--transaction", "in", "--note", "", "--reason", "  spaced  "),
            ("records", "note", "set", "--transaction", "in", "--note", "A\n€"),
            ("records", "note", "clear", "--transaction", "in"),
            ("records", "kind", "set", "--transaction", "in", "--kind", "income"),
            ("records", "kind", "clear", "--transaction", "in"),
            ("exclude", "--transaction", "in"),
            ("include", "--transaction", "in"),
            ("tags", "remove", "--transaction", "in", "--tag", "ος-team"),
        ]
        for command in commands:
            candidate = self.ok(self.cli("metadata", *command))
            reference = self.ok(cli_runner.run(["--machine", "--data-root", str(reference_root), "metadata", *command], reference=True, env=self.env, capture_output=True, cwd=ROOT, timeout=120))
            for result in (candidate, reference):
                result.pop("id", None)
                result.pop("created_at", None)
                result.pop("history_event_id", None)
            self.assertEqual(candidate, reference)

        def snapshot(root):
            conn = open_db(str(root))
            try:
                return [list(conn.execute(query)) for query in (
                    "SELECT field,before_value,after_value,diff_json FROM transaction_edit_fields ORDER BY rowid",
                    "SELECT source,reason,journal_input_version,journal_input_version_after,last_processed_input_version,last_processed_at,last_processed_tx_count FROM transaction_edit_events ORDER BY rowid",
                    "SELECT journal_input_version,last_processed_at,last_processed_tx_count FROM profiles",
                    "SELECT code,label FROM tags ORDER BY code",
                    "SELECT t.code,t.label FROM transaction_tags tt JOIN tags t ON t.id=tt.tag_id ORDER BY t.code",
                )]
            finally:
                conn.close()
        self.assertEqual(snapshot(self.root), snapshot(reference_root))

    def test_daemon_stale_summary_and_validation_match_reference_bytes(self):
        self.ok(self.cli("metadata", "notes", "set", "--transaction", "in", "--note", "Receipt"))
        requests = [{"kind": "ui.activity.stale"}, {"kind": "ui.activity.stale", "args": {"unknown": True}}, {"kind": "ui.activity.stale", "args": []}, {"kind": "daemon.shutdown"}]
        wire = "".join(json.dumps({"request_id": str(i), **request}) + "\n" for i, request in enumerate(requests)).encode()
        options = dict(env=self.env, input=wire, capture_output=True, cwd=ROOT, timeout=120)
        args = ["--machine", "--data-root", str(self.root), "daemon"]
        reference = cli_runner.run(args, reference=True, **options)
        candidate = cli_runner.run(args, env={**self.env, "KASSIBER_HYBRID_RUST_DAEMON": "1"}, input=wire, capture_output=True, cwd=ROOT, timeout=120)
        self.assertEqual((candidate.returncode, candidate.stderr), (0, b""))
        self.assertEqual(candidate.stdout.splitlines()[1:], reference.stdout.splitlines()[1:])

    def test_unported_metadata_and_following_families_refuse_before_bootstrap(self):
        for command in (("metadata", "records", "list"), ("metadata", "bip329", "list"), ("metadata", "records", "history", "list", "--transaction", "in"), ("metadata", "records", "history", "activity"), ("metadata", "records", "history", "revert", "--transaction", "in", "--event", "event"), ("transactions", "list"), ("documents", "list")):
            absent = self.base / "absent-book"
            result = subprocess.run([RUST_BIN, "--machine", "--data-root", str(absent), *command], env=self.env, capture_output=True, timeout=120)
            self.assertEqual(json.loads(result.stdout)["error"]["code"], "not_implemented")
            self.assertFalse(absent.exists())
