"""Grouped desktop saves use the shared native metadata service on disposable books."""
import json
import os
from pathlib import Path
import shutil
import unittest

from kassiber.db import open_db
from tests import cli_runner
from tests import test_rust_transaction_services as services

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
KIND = "ui.transactions.metadata.update"


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the test-harness binary")
class RustGroupedMetadataServicesTest(unittest.TestCase):
    setUp = services.RustTransactionServicesTest.setUp
    cli = services.RustTransactionServicesTest.cli
    ok = services.RustTransactionServicesTest.ok
    same_read = services.RustTransactionServicesTest.same_read

    def daemon(self, root, updates, *, reference=False):
        wire = "".join(json.dumps({"request_id": str(i), "kind": KIND, "args": args}) + "\n"
                       for i, args in enumerate(updates))
        wire += json.dumps({"request_id": "stop", "kind": "daemon.shutdown"}) + "\n"
        result = cli_runner.run(
            ["--machine", "--data-root", str(root), "daemon"], reference=reference,
            env={**self.env, "KASSIBER_HYBRID_RUST_DAEMON": "1"},
            input=wire.encode(), capture_output=True, cwd=ROOT, timeout=120,
        )
        self.assertEqual((result.returncode, result.stderr), (0, b""), result.stdout)
        lines = result.stdout.splitlines()
        self.assertEqual(len(lines), len(updates) + 2, result.stdout)
        if not reference:
            self.assertIn(KIND, json.loads(lines[0])["data"]["supported_kinds"])
        return lines[1:-1]

    def test_grouped_saves_match_reference_values_history_and_versions(self):
        self.ok(self.cli("metadata", "tags", "create", "--code", "team-name", "--label", "Existing identity"))
        reference_root = self.base / "reference" / "data"
        shutil.copytree(self.root, reference_root)
        updates = [
            {"transaction": "in", "note": "Invoice €\n42", "tags": [" Team Name ", "team name", "ΟΣ", "ος", ""],
             "excluded": True, "review_status": "review", "taxable": False, "at_regime": "outside",
             "at_category": "none", "kind": "income", "fiat_rate": "65000.00100", "fiat_currency": " eur ",
             "pricing_source_kind": "manual_override", "pricing_external_ref": " invoice=42 ", "reason": " grouped save "},
            {"transaction": "in", "note": "", "tags": [], "excluded": False, "review_status": "completed",
             "taxable": True, "at_regime": "neu", "at_category": "neu_gain", "kind": None, "source": " CLI "},
            {"transaction": "in", "note": None, "taxable": None, "at_category": None, "at_regime": None,
             "review_status": None, "source": "ai_tool", "tags": None, "excluded": None},
            {"transaction": "in", "fiat_value": "1.234567890123456789012345678901", "pricing_source_kind": "generic_import"},
            {"transaction": "out", "kind": "sell", "fiat_rate": 12.34, "pricing_source_kind": "manual_rate_cache",
             "pricing_quality": "coarse_fallback", "fiat_currency": "usd"},
            {"transaction": "in", "fiat_currency": "usd"},
            {"transaction": "in", "fiat_rate": "ignored", "pricing_source_kind": "manual_override", "pricing_quality": "missing"},
            {"transaction": "in"},
        ]
        candidate = self.daemon(self.root, updates)
        reference = self.daemon(reference_root, updates, reference=True)
        for actual, expected in zip(candidate, reference):
            actual_data = json.loads(actual)["data"]
            expected_data = json.loads(expected)["data"]
            # Only generated event identities differ in these mutation envelopes.
            if expected_data["history_event_id"] is not None:
                actual = actual.replace(actual_data["history_event_id"].encode(), b"<event>")
                expected = expected.replace(expected_data["history_event_id"].encode(), b"<event>")
            self.assertEqual(actual, expected)

        def snapshot(root):
            conn = open_db(str(root))
            try:
                fields = [tuple(row) for row in conn.execute("SELECT field,before_value,after_value,diff_json FROM transaction_edit_fields WHERE field <> 'pricing_fetched_at' ORDER BY rowid")]
                captures = [tuple(row) for row in conn.execute("SELECT before_value,after_value FROM transaction_edit_fields WHERE field='pricing_fetched_at' ORDER BY rowid")]
                for before, after in captures:
                    for raw in (before, after):
                        if raw != "null":
                            self.assertRegex(json.loads(raw), r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$")
                return fields, len(captures), [
                    [tuple(row) for row in conn.execute(query)] for query in (
                        "SELECT source,reason,journal_input_version,journal_input_version_after,last_processed_input_version,last_processed_at,last_processed_tx_count FROM transaction_edit_events ORDER BY rowid",
                        "SELECT journal_input_version,last_processed_input_version,last_processed_at,last_processed_tx_count FROM profiles",
                        "SELECT id,kind,kind_override,amount,fee,note,excluded,review_status,taxability_override FROM transactions ORDER BY id",
                        "SELECT code,label FROM tags ORDER BY code",
                        "SELECT tt.transaction_id,t.code FROM transaction_tags tt JOIN tags t ON t.id=tt.tag_id ORDER BY tt.transaction_id,t.code",
                    )]
            finally:
                conn.close()
        self.assertEqual(snapshot(self.root), snapshot(reference_root))
        for tx in ("in", "out"):
            self.same_read("metadata", "records", "get", "--transaction", tx)

    def test_grouped_validation_errors_match_reference_bytes(self):
        invalids = [None, [], {}, {"unknown": True, "z": False}, {"transaction": 3}, {"transaction": "absent"}]
        for field in ("note", "tags", "excluded", "taxable", "review_status", "at_regime", "at_category", "kind",
                      "source", "reason", "fiat_currency", "pricing_source_kind", "pricing_quality", "pricing_external_ref"):
            for value in (3, [], {}, False):
                if field == "tags" and value == []:
                    continue
                args = {"transaction": "in", field: value}
                if field == "pricing_external_ref":
                    args.update(pricing_source_kind="manual_override", fiat_rate="42")
                invalids.append(args)
        invalids.extend({"transaction": "in", "pricing_source_kind": "manual_override", field: value}
                        for field in ("fiat_rate", "fiat_value")
                        for value in (True, "junk", "-1", "Infinity", "-Infinity", "NaN", "sNaN"))
        invalids.extend([
            {"transaction": "out", "kind": "income"}, {"transaction": "in", "kind": "sell"},
            {"transaction": "in", "source": "unsupported"}, {"transaction": "in", "note": "x" * 20001},
            {"transaction": "in", "tags": ["x" * 129]}, {"transaction": "in", "tags": [3]},
            {"transaction": "in", "pricing_source_kind": "unsupported"},
            {"transaction": "in", "pricing_source_kind": "manual_override"},
            {"transaction": "in", "pricing_quality": "unsupported"},
            {"transaction": "in", "fiat_currency": "e\u0301r"},
            {"transaction": "in", "fiat_currency": "\U00011f02ab"},
        ])
        # False taxable/excluded is valid, so omit it from the refusal corpus.
        invalids = [args for args in invalids if not isinstance(args, dict) or not any(args.get(k) is False for k in ("excluded", "taxable"))]
        reference = self.daemon(self.root, invalids, reference=True)
        candidate = self.daemon(self.root, invalids)
        self.assertEqual(candidate, reference)
        self.assertTrue(all("error" in json.loads(line) for line in candidate))
        conn = open_db(str(self.root))
        try:
            self.assertEqual(conn.execute("SELECT COUNT(*) FROM transaction_edit_events").fetchone()[0], 0)
        finally:
            conn.close()

    def test_implied_price_bounds_match_reference_bytes(self):
        # f823b4729 and d444f4db5: the payload bounds both prices and the
        # valuations each implies for the 0.00100000001 BTC quantity.
        reference_root = self.base / "bounds-reference" / "data"
        shutil.copytree(self.root, reference_root)
        updates = [{"transaction": "out", "pricing_source_kind": "manual_override", **prices} for prices in (
            {"fiat_value": "1e18"},
            {"fiat_rate": "1e10", "fiat_value": "1e17"},
            {"fiat_rate": "1e-12"},
            {"fiat_rate": "1e-11", "fiat_value": "5"},
            {"fiat_rate": "1e-10", "fiat_value": "5"},
            {"fiat_rate": "9999999999999999999.5"},
            {"fiat_rate": "1e19"},
            {"fiat_rate": "100", "fiat_value": "7"},
        )]
        reference = self.daemon(reference_root, updates, reference=True)
        candidate = self.daemon(self.root, updates)
        for actual, expected in zip(candidate, reference):
            actual_data, expected_data = json.loads(actual).get("data"), json.loads(expected).get("data")
            if expected_data and expected_data["history_event_id"]:
                actual = actual.replace(actual_data["history_event_id"].encode(), b"<event>")
                expected = expected.replace(expected_data["history_event_id"].encode(), b"<event>")
            self.assertEqual(actual, expected)
        refused = [json.loads(line)["error"]["message"] for line in reference if "error" in json.loads(line)]
        self.assertIn("transaction pricing input: derived fiat_rate is outside the supported decimal range", refused)
        self.assertIn("transaction pricing input: derived fiat_value is outside the supported decimal range", refused)

    def test_partial_prices_currency_and_noops_match_reference_bytes(self):
        # These CPython currencies exercise expansion, alphabetic categories and
        # case pairs added after Unicode 14; currency validation is not ASCII-only.
        updates = [{"transaction": "in", "fiat_currency": currency}
                   for currency in ("€UR", "ßa", "ςσπ", "\u019bab", "\u0264ab", "\ua7d3ab", "\u0345ab")]
        reference_root = self.base / "prices-reference" / "data"
        shutil.copytree(self.root, reference_root)
        for rate in ("0", "-0", "0.00000000000001", "1_234.500", "١٢.٣٤", "1e-31", "1e30"):
            updates.append({"transaction": "out", "pricing_source_kind": "wallet_export", "fiat_rate": rate})
        reference = self.daemon(reference_root, updates, reference=True)
        candidate = self.daemon(self.root, updates)
        for actual, expected in zip(candidate, reference):
            actual_data, expected_data = json.loads(actual).get("data"), json.loads(expected).get("data")
            if expected_data and expected_data["history_event_id"]:
                actual = actual.replace(actual_data["history_event_id"].encode(), b"<event>")
                expected = expected.replace(expected_data["history_event_id"].encode(), b"<event>")
            self.assertEqual(actual, expected)
        # Stable read and no-op records compare as raw bytes on the same book.
        noops = [{"transaction": "out", "pricing_source_kind": "wallet_export", "fiat_rate": "1e30"},
                 {"transaction": "out", "tags": []}, {"transaction": "out", "note": None}]
        self.assertEqual(self.daemon(self.root, noops), self.daemon(self.root, noops, reference=True))
