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
        from tests.test_rust_hybrid_candidate import unported_read_witnesses

        witnesses = unported_read_witnesses(RUST_BIN, self.base)[:1]
        if not witnesses:
            self.skipTest("every database read is native in this build; "
                          "crates/kassiber/tests/binary.rs pins the gated leaves of product builds")
        for command in witnesses:
            absent = self.base / "absent-book"
            result = subprocess.run([RUST_BIN, "--machine", "--data-root", str(absent), *command], env=self.env, capture_output=True, timeout=120)
            self.assertEqual(json.loads(result.stdout)["error"]["code"], "not_implemented")
            self.assertFalse(absent.exists())

    def test_record_and_history_pages_match_reference_bytes_with_bound_cursors(self):
        for tx, note in (("in", "Invoice €"), ("out", "Receipt"), ("in", "Revised")):
            self.ok(self.cli("metadata", "notes", "set", "--transaction", tx, "--note", note))
        for family in (("records", "list"), ("records", "history", "list", "--transaction", "in"), ("records", "history", "activity")):
            args = ("metadata", *family, "--limit", "1")
            self.same_read(*args)
            page = self.ok(self.cli(*args))
            self.assertTrue(page["has_more"])
            cursor = page["next_cursor"]
            self.same_read(*args, "--cursor", cursor)
            self.ok(self.cli(*args, "--cursor", cursor))
            self.same_read(*args, "--cursor", cursor, "--start", "2025-01-01")
            self.same_read(*args, "--cursor", "broken")
            for limit in ("0", "1001"):
                self.same_read(*args, "--limit", limit)
        for flags in (("--wallet", " Cold "), ("--wallet", "absent"), ("--has-note",), ("--no-note",), ("--excluded",), ("--included",), ("--start", "2025-W01-1T12:00:00.123456+02:00"), ("--end", "20250101T010000Z"), ("--start", "bad"), ("--has-note", "--no-note"), ("--excluded", "--included")):
            self.same_read("metadata", "records", "list", *flags)
        for flags in (("--field", "note"), ("--field-family", "tax"), ("--field-family", "unsupported"), ("--pricing-only",), ("--ai-only",), ("--source", "GUI"), ("--source", "bad"), ("--field", "unknown"), ("--stale-only",), ("--wallet", "Cold")):
            self.same_read("metadata", "records", "history", "activity", *flags)

    def test_grouped_forward_reverts_match_reference_state_and_preserve_original_events(self):
        from kassiber.cli.handlers import _metadata_hooks
        from kassiber.core import metadata
        conn = open_db(str(self.root))
        hooks = _metadata_hooks()
        metadata.update_transaction_metadata(conn, None, None, "in", hooks, note="Original", note_set=True, tags=["Receipt", "Team"], excluded=True, review_status="review", review_status_set=True, taxable=False, taxable_set=True, at_regime="outside", at_regime_set=True, at_category="none", at_category_set=True, kind="income", kind_set=True, pricing_update={"fiat_currency": "EUR", "fiat_rate": "12345.000001", "source_kind": "manual_override", "quality": "exact", "external_ref": "invoice=42 secret=not-exported"}, source="gui")
        edited = metadata.update_transaction_metadata(conn, None, None, "in", hooks, note="Changed", note_set=True, tags=["Receipt"], excluded=False, review_status="completed", review_status_set=True, taxable=True, taxable_set=True, at_regime="neu", at_regime_set=True, at_category="neu_gain", at_category_set=True, kind=None, kind_set=True, pricing_update={"fiat_currency": "USD", "fiat_rate": "54321.000001", "source_kind": "manual_rate_cache", "quality": "coarse_fallback"}, source="gui")
        conn.close()
        reference_root = self.base / "reference-revert" / "data"
        shutil.copytree(self.root, reference_root)
        for field in ("note", "tags", "kind", None, None):
            command = ["metadata", "records", "history", "revert", "--transaction", "in", "--event", edited["history_event_id"]]
            if field:
                command.extend(("--field", field))
            candidate = self.ok(self.cli(*command))
            reference = self.ok(cli_runner.run(["--machine", "--data-root", str(reference_root), *command], reference=True, env=self.env, capture_output=True, cwd=ROOT, timeout=120))
            for result in (candidate, reference):
                result.pop("history_event_id", None)
            self.assertEqual(candidate, reference)
        def snapshot(root):
            conn = open_db(str(root))
            try:
                return [list(conn.execute(query)) for query in (
                    "SELECT field,before_value,after_value,diff_json FROM transaction_edit_fields ORDER BY rowid",
                    "SELECT source,reason,journal_input_version,journal_input_version_after,last_processed_input_version,last_processed_at,last_processed_tx_count FROM transaction_edit_events ORDER BY rowid",
                    "SELECT journal_input_version FROM profiles",
                )]
            finally:
                conn.close()
        self.assertEqual(snapshot(self.root), snapshot(reference_root))
        for extra in (("--field", "missing"), ("--field", "pricing_method"), ("--transaction", "out"), ("--event", "missing")):
            self.same_read("metadata", "records", "history", "revert", "--transaction", "in", "--event", edited["history_event_id"], *extra)

    def test_price_reverts_bound_stored_values_and_implied_valuations_like_reference(self):
        # f823b4729 and d444f4db5: a revert validates the restored price and
        # the valuations the merged row implies, before any write.
        from kassiber.cli.handlers import _metadata_hooks
        from kassiber.core import metadata
        conn = open_db(str(self.root))
        hooks = _metadata_hooks()
        for rate in ("1e15", "2"):
            edited = metadata.update_transaction_metadata(conn, None, None, "out", hooks, pricing_update={"fiat_currency": "EUR", "fiat_rate": rate, "source_kind": "manual_override", "quality": "exact"}, source="gui")
        event = edited["history_event_id"]
        # 10,000 BTC: restoring the 1e15 rate would imply a 1e19 total.
        conn.execute("UPDATE transactions SET amount=1000000000000000 WHERE id='out'")
        conn.commit()
        conn.close()
        command = ("metadata", "records", "history", "revert", "--transaction", "out", "--event", event)
        self.same_read(*command)
        self.same_read(*command, "--field", "fiat_rate")
        self.assertIn(b"transaction out: derived fiat_value is outside the supported decimal range", self.cli(*command, reference=True).stdout)
        for stored in ('"1e40"', '"NaN"', '"bad"', '"9999999999999999999.5"'):
            conn = open_db(str(self.root))
            conn.execute("UPDATE transaction_edit_fields SET before_value=? WHERE event_id=? AND field='fiat_rate'", (stored, event))
            conn.commit()
            conn.close()
            self.same_read(*command, "--field", "fiat_rate")
        conn = open_db(str(self.root))
        try:
            self.assertEqual(conn.execute("SELECT COUNT(*) FROM transaction_edit_events").fetchone()[0], 2)
        finally:
            conn.close()

    def test_daemon_history_pages_filters_and_validation_match_reference_bytes(self):
        self.ok(self.cli("metadata", "notes", "set", "--transaction", "in", "--note", "Receipt"))
        page = self.ok(self.cli("metadata", "records", "history", "activity"))
        requests = []
        for kind in ("ui.activity.history", "ui.transactions.history"):
            for args in ({"transaction": "in"}, {"transaction": "in", "limit": "1", "include_stale": False}, {"transaction": "in", "field": "note", "ai_only": True}, {"transaction": "in", "unknown": True}, {"transaction": "in", "include_stale": None}, {"transaction": 3}, {}, {"transaction": "in", "limit": 0}, {"transaction": "in", "limit": 1001}):
                requests.append({"kind": kind, "args": args})
        for args in ({}, {"transaction": "in"}, {"transaction": "in", "event": "missing"}, {"transaction": "out", "event": page["events"][0]["id"]}, {"transaction": "in", "event": page["events"][0]["id"], "field": "missing"}):
            requests.append({"kind": "ui.transactions.history.revert", "args": args})
        requests.append({"kind": "daemon.shutdown"})
        wire = "".join(json.dumps({"request_id": str(i), **request}) + "\n" for i, request in enumerate(requests)).encode()
        args = ["--machine", "--data-root", str(self.root), "daemon"]
        options = dict(input=wire, capture_output=True, cwd=ROOT, timeout=120)
        reference = cli_runner.run(args, reference=True, env=self.env, **options)
        candidate = cli_runner.run(args, env={**self.env, "KASSIBER_HYBRID_RUST_DAEMON": "1"}, **options)
        self.assertEqual((candidate.returncode, candidate.stderr), (0, b""))
        self.assertEqual(candidate.stdout.splitlines()[1:], reference.stdout.splitlines()[1:])

        reference_root = self.base / "daemon-reference" / "data"
        shutil.copytree(self.root, reference_root)
        revert_wire = (json.dumps({"request_id": "revert", "kind": "ui.transactions.history.revert", "args": {"transaction": "in", "event": page["events"][0]["id"]}}) + "\n" + json.dumps({"request_id": "stop", "kind": "daemon.shutdown"}) + "\n").encode()
        options["input"] = revert_wire
        reference = cli_runner.run(["--machine", "--data-root", str(reference_root), "daemon"], reference=True, env=self.env, **options)
        candidate = cli_runner.run(args, env={**self.env, "KASSIBER_HYBRID_RUST_DAEMON": "1"}, **options)
        expected = json.loads(reference.stdout.splitlines()[1])
        actual = json.loads(candidate.stdout.splitlines()[1])
        self.assertTrue(actual["data"]["updated"])
        for response in (actual, expected):
            response["data"].pop("history_event_id")
        self.assertEqual(actual, expected)
        conn = open_db(str(self.root))
        try:
            self.assertEqual(conn.execute("SELECT source FROM transaction_edit_events ORDER BY rowid DESC LIMIT 1").fetchone()[0], "gui")
        finally:
            conn.close()
