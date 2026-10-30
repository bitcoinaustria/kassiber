"""Documents and stored commercial evidence through the hybrid CLI and JSONL daemon."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import unittest

from kassiber.db import open_db
from tests import cli_runner
from tests import test_rust_transaction_services as services

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
KINDS = ["ui.documents." + name for name in ("list", "create", "attach", "import_report")] + [
    "ui.transactions.commercial_context", "ui.btcpay.provenance.list",
    "ui.btcpay.provenance.links", "ui.btcpay.provenance.suggest",
]


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the test-harness binary")
class RustDocumentServicesTest(unittest.TestCase):
    setUp = services.RustTransactionServicesTest.setUp
    cli = services.RustTransactionServicesTest.cli
    ok = services.RustTransactionServicesTest.ok
    same_read = services.RustTransactionServicesTest.same_read

    def seed(self):
        conn = open_db(str(self.root))
        workspace, profile = conn.execute("SELECT workspace_id,id FROM profiles").fetchone()
        conn.execute("UPDATE transactions SET external_id='chain',payment_hash='hash',amount=1101,fee=100,amount_includes_fee=1 WHERE id='out'")
        for id_, kind, amount in (("payment", "payment", 1001), ("invoice", "invoice", None), ("payout", "payout", 1001)):
            conn.execute("""INSERT INTO btcpay_provenance_records(id,workspace_id,profile_id,record_type,stable_key,backend_name,store_id,invoice_id,payment_id,txid,payment_hash,amount,asset,status,fiat_currency,fiat_value_exact,created_at,updated_at)
                VALUES(?,?,?,?,?,'Merchant','store','inv',?,'chain','hash',?,'BTC','Completed','EUR','12.00','2025','2025')""", (id_, workspace, profile, kind, "stable-" + id_, id_, amount))
        conn.commit()
        conn.close()

    def daemon(self, requests, *, reference=False, root=None, allow_reference_errors=False):
        wire = "".join(json.dumps({"request_id": str(i), **r}) + "\n"
                       for i, r in enumerate([*requests, {"kind": "daemon.shutdown"}])).encode()
        argv = ["--machine", "--data-root", str(root or self.root), "daemon"]
        options = dict(input=wire, env={**self.env, "KASSIBER_HYBRID_RUST_DAEMON": "1"},
                       capture_output=True, cwd=ROOT, timeout=120)
        result = (cli_runner.run(argv, reference=reference, **options)
                  if reference or cli_runner.candidate_prefix()
                  else subprocess.run([RUST_BIN, *argv], **options))
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        if not (reference and allow_reference_errors):
            self.assertEqual(result.stderr, b"", result.stdout)
        lines = result.stdout.splitlines()
        if not reference:
            for kind in KINDS:
                self.assertIn(kind, json.loads(lines[0])["data"]["supported_kinds"])
            if path := os.environ.get("KASSIBER_DOCUMENT_COUNTS"):
                with open(path, "a", encoding="utf-8") as handle:
                    for request in requests:
                        handle.write(json.dumps({"kind": request["kind"], "served_by": "rust"}) + "\n")
        return lines[1:-1]

    def compare(self, requests):
        actual = self.daemon(requests)
        expected = self.daemon(requests, reference=True)
        self.assertEqual(len(actual), len(expected))
        for request, got, wanted in zip(requests, actual, expected):
            self.assertEqual(got, wanted, request)
        return [json.loads(line) for line in actual]

    def test_all_document_commands_keep_evidence_literal_and_refusals_byte_exact(self):
        source = self.base / "receipt.txt"
        source.write_bytes(b"synthetic evidence\n")
        doc = self.ok(self.cli("documents", "create", "--type", "invoice", "--label", "  literal  ",
                               "--external-ref", "inv", "--fiat-currency", "eur", "--fiat-value", "12.00",
                               "--issued-at", "2025-01-01T01:00:00.123+01:00"))
        self.assertEqual(doc["issued_at"], "2025-01-01T00:00:00Z")
        self.ok(self.cli("documents", "attach", "--document", doc["id"], "--file", str(source)))
        self.ok(self.cli("documents", "attach", "--document", "inv", "--url", "  127.0.0.1:1/literal  "))
        report = self.ok(self.cli("documents", "import-report", "--provider", "Calculator", "--file", str(source), "--tax-year", "2024"))
        self.assertEqual(report["document"]["document_type"], "tax_report")
        self.ok(self.cli("documents", "list", "--limit", "0"))
        for args in [
            ("list",), ("list", "--limit", "-1"),
            ("create", "--type", "invoice", "--label", "duplicate", "--external-ref", "inv"),
            ("create", "--type", "invoice", "--label", "bad", "--issued-at", "bad"),
            ("create", "--type", "invoice", "--label", "bad", "--fiat-value", "bad"),
            ("create", "--type", "invoice", "--label", "bad", "--fiat-value=--Infinity"),
            ("create", "--type", "invoice", "--label", "bad", "--issued-at", "0001-01-01T00:00:00+01:00"),
            ("attach", "--document", "missing", "--url", "literal"),
            ("attach", "--document", doc["id"]),
            ("attach", "--document", doc["id"], "--file", str(self.base)),
            ("attach", "--document", doc["id"], "--file", str(source), "--url", "literal"),
            ("import-report", "--provider", " ", "--file", str(source)),
            ("import-report", "--provider", "x", "--file", "absent"),
            ("import-report", "--provider", "x", "--file", str(source), "--tax-year", "20"),
        ]:
            self.same_read("documents", *args)
        for label in ("same", "same"):
            self.ok(self.cli("documents", "create", "--type", "other", "--label", label))
        self.same_read("documents", "attach", "--document", "same", "--url", "literal")
        conn = open_db(str(self.root))
        self.assertEqual(conn.execute("SELECT COUNT(*) FROM attachments WHERE transaction_id IS NOT NULL").fetchone()[0], 0)
        self.assertEqual(conn.execute("SELECT source_url FROM attachments WHERE attachment_type='url'").fetchone()[0], "  127.0.0.1:1/literal  ")
        self.assertEqual(conn.execute("SELECT COUNT(*) FROM journal_entries").fetchone()[0], 0)
        conn.close()
        attachments = self.base / "attachments"
        backup = self.base / "saved-attachments"
        attachments.rename(backup)
        attachments.write_bytes(b"occupied managed root")
        for reference in (False, True):
            result = self.cli("documents", "attach", "--document", "inv", "--url", "literal", reference=reference)
            self.assertEqual(result.returncode, 1)
            self.assertEqual(json.loads(result.stdout)["error"]["code"], "internal_error")
        attachments.unlink()
        backup.rename(attachments)

    def test_cli_mutations_and_stored_effects_match_reference(self):
        reference = self.base / "reference" / "data"
        shutil.copytree(self.root, reference)
        source = self.base / "prior.txt"
        source.write_bytes(b"prior report\n")
        operations = [
            ("create", "--type", "invoice", "--label", "Invoice", "--external-ref", "inv", "--fiat-value", "1.200E2"),
            ("attach", "--document", "inv", "--file", str(source)),
            ("attach", "--document", "inv", "--url", "literal://127.0.0.1:1", "--label", "   "),
            ("import-report", "--provider", " Calculator ", "--file", str(source), "--tax-year", "2024"),
        ]
        def stable(value):
            if isinstance(value, dict):
                return {k: stable(v) for k,v in value.items() if k not in {"id", "attachment_id", "document_id", "created_at", "updated_at"}}
            return value
        for operation in operations:
            actual = self.ok(self.cli("documents", *operation))
            expected = self.ok(cli_runner.run(["--machine", "--data-root", str(reference), "documents", *operation], reference=True, env=self.env, capture_output=True, timeout=120))
            self.assertEqual(stable(actual), stable(expected))
        def snapshot(root):
            conn = open_db(str(root))
            try:
                return [list(conn.execute(query)) for query in (
                    "SELECT document_type,label,external_ref,issuer,counterparty,issued_at,due_at,fiat_currency,fiat_value_exact,review_state,notes,raw_json FROM external_documents ORDER BY rowid",
                    "SELECT transaction_id,attachment_type,label,original_filename,source_url,media_type,size_bytes,sha256,copied_from_attachment_id,copied_from_transaction_id FROM attachments ORDER BY rowid",
                    "SELECT d.label,a.label FROM external_document_attachments da JOIN external_documents d ON d.id=da.document_id JOIN attachments a ON a.id=da.attachment_id ORDER BY da.rowid",
                    "SELECT journal_input_version,last_processed_at FROM profiles",
                )]
            finally:
                conn.close()
        self.assertEqual(snapshot(self.root), snapshot(reference))

    def test_document_daemon_successes_and_typed_refusals_match_python(self):
        reference = self.base / "reference" / "data"
        shutil.copytree(self.root, reference)
        source = self.base / "prior.pdf"
        source.write_bytes(b"prior report")
        requests = [
            {"kind": "ui.documents.create", "args": {"type": " TAX-REPORT ", "label": "Report", "external_ref": "report", "fiat_value": 1.25, "due_at": "1970-01-01"}},
            {"kind": "ui.documents.attach", "args": {"document": "report", "file_path": str(source), "label": "Evidence"}},
            {"kind": "ui.documents.attach", "args": {"document": "report", "url": "anything://127.0.0.1:1", "label": "   "}},
            {"kind": "ui.documents.import_report", "args": {"provider": "Provider", "file": str(source), "tax_year": 2024}},
        ]
        actual = self.daemon(requests)
        expected = self.daemon(requests, reference=True, root=reference)
        def stable(v):
            if isinstance(v, dict):
                return {k: stable(x) for k,x in v.items() if k not in {"id", "document_id", "attachment_id", "created_at", "updated_at"}}
            return v
        for got, wanted in zip(actual, expected):
            self.assertIn("data", json.loads(got), got)
            self.assertEqual(stable(json.loads(got)), stable(json.loads(wanted)))
        self.compare([
            {"kind": "ui.documents.list", "args": {"limit": limit}} for limit in (None, 0, -2, True, "1", 1.9, 10**50)
        ] + [
            {"kind": "ui.documents.create", "args": args} for args in (
                {}, {"label": []}, {"label": "x"}, {"label": "x", "type": "wrong"},
                {"label": "x", "type": "invoice", "issued_at": "bad"},
                *({"label": "x", "type": "invoice", "fiat_value": raw}
                  for raw in ("bad", "NaN", "Infinity", "1e309", "1e-14")),
                {"label": "x", "type": "invoice", "external_ref": "report"},
            )
        ] + [
            {"kind": "ui.documents.attach", "args": args} for args in ({}, {"document": []}, {"document": "report"}, {"document": "absent", "url": "literal"})
        ] + [
            {"kind": "ui.documents.import_report", "args": args} for args in ({}, {"provider": "x"}, {"provider": "x", "file": str(source), "tax_year": "bad"})
        ] + [{"kind": "ui.transactions.commercial_context", "args": {"transaction": t}} for t in (None, [], "absent", "in")])
        malformed = [
            {"kind": "ui.documents.create", "args": {"label": "\ud800", "type": "invoice"}},
            {"kind": "ui.documents.create", "args": {"label": "x", "type": "invoice\ud800"}},
            {"kind": "ui.documents.create", "args": {"label": "x", "type": "invoice", "issued_at": "2025-01-01\ud800"}},
            {"kind": "ui.documents.create", "args": {"label": "x", "type": "invoice", "fiat_value": "1\ud800"}},
            {"kind": "ui.documents.create", "args": {"label": "x", "type": "invoice", "fiat_currency": "EUR\ud800"}},
            {"kind": "ui.documents.attach", "args": {"document": "report", "url": "literal\ud800"}},
            {"kind": "ui.documents.attach", "args": {"document": "report", "file": "\ud800" + str(source)}},
            {"kind": "ui.documents.import_report", "args": {"provider": "x", "file": str(source), "tax_year": "2024\ud800"}},
        ]
        actual = self.daemon(malformed)
        expected = self.daemon(malformed, reference=True, allow_reference_errors=True)
        for got, wanted in zip(actual, expected):
            self.assertEqual(json.loads(got)["error"]["code"], json.loads(wanted)["error"]["code"])

    def test_stored_matching_and_context_reads_match_and_never_normalize_tax(self):
        self.seed()
        conn = open_db(str(self.root))
        initial_version = conn.execute("SELECT journal_input_version FROM profiles").fetchone()[0]
        conn.close()
        doc = self.ok(self.cli("documents", "create", "--type", "invoice", "--label", "Invoice", "--external-ref", "inv", "--fiat-currency", "EUR", "--fiat-value", "12.00"))
        result = self.ok(self.cli("btcpay", "provenance", "suggest"))
        self.assertEqual(result["created"], 3)
        self.ok(self.cli("btcpay", "provenance", "links"))
        self.ok(self.cli("btcpay", "provenance", "list"))
        for suffix in (("links",), ("list",), ("suggest",), ("links", "--state", "reviewed"), ("list", "--record-type", "payment")):
            self.same_read("btcpay", "provenance", *suffix)
        requests = [
            {"kind": "ui.transactions.commercial_context", "args": {"transaction": "out"}},
            {"kind": "ui.documents.list"},
            {"kind": "ui.btcpay.provenance.list"},
            {"kind": "ui.btcpay.provenance.links"},
            {"kind": "ui.btcpay.provenance.suggest"},
            {"kind": "ui.btcpay.provenance.list", "args": {"record_type": "bad"}},
            {"kind": "ui.btcpay.provenance.links", "args": {"state": "bad"}},
        ]
        self.compare(requests)
        conn = open_db(str(self.root))
        conn.execute("UPDATE commercial_links SET state='reviewed',reviewed_at='2025' WHERE transaction_id='out'")
        conn.execute("UPDATE btcpay_provenance_records SET payment_request_id='request',origin_kind='payment_request',origin_label='Request',origin_url='http://127.0.0.1:1/literal',fiat_currency='BTC' WHERE record_type='invoice'")
        conn.commit()
        conn.close()
        context = self.compare(requests[:1])[0]["data"]
        self.assertEqual(context["documents"][0]["id"], doc["id"])
        self.assertEqual(context["btcpay"][0]["invoice"]["fiat_value_exact"], "")
        conn = open_db(str(self.root))
        self.assertEqual(conn.execute("SELECT kind FROM transactions WHERE id='out'").fetchone()[0], "deposit")
        self.assertEqual(conn.execute("SELECT journal_input_version FROM profiles").fetchone()[0], initial_version)
        conn.close()
