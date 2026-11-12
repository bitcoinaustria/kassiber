"""Whole-process parity for `ui.reports.export_audit_package`.

Each session runs the unchanged Python daemon (with the report clock pinned
by ``tests/rust_ai_tool_reference.py``) and then the Rust daemon against the
same book and state root. Responses must be byte-identical, every exported
file (manifest, copied evidence, permission bits and copied modification
times) must match, and no table may change. The package renders no report,
so there is nothing for the shared report design to replace.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import unittest

from tests.rust_desktop_parity import RUST_BIN, DesktopParity, run_candidate
from tests.test_rust_evidence_summary import EVIDENCE

ROOT = Path(__file__).resolve().parent.parent
REFERENCE = ROOT / "tests" / "rust_ai_tool_reference.py"
CLOCK = "2026-03-04T05:06:07Z"
KIND = "ui.reports.export_audit_package"
PDF = b"%PDF"
GOOD = hashlib.sha256(PDF).hexdigest()

# References urllib must parse, redact and put back together exactly as
# Python does: credentials, secret-like query and fragment keys, `;params`,
# empty and repeated pairs, percent and plus escapes, opaque schemes.
URLS = [
    "https://shop.example/inv?Api%5FKey=s&x=+y&z",
    "https://bob:pw@Docs.Example.COM:8443/doc;v=1?id=1#x",
    "http://h.example/a;#frag?token=1&b=%E2%82%AC",
    "https://h.example/p;?sig=1",
    "HTTPS://user@[::1]:80/x?password=&q=a b",
    "ftp://u:@host/f#key",
    "mailto:x@example.com?subject=hi&auth=1",
    "example.com/path?secret=1",
    "//host/p?token=é%zz&ü=1",
    "https://h/x#?a=1&b",
    "  https://h/x?a=%2B&Bearer=1  ",
    "custom:opaque;p?api_key=1",
    "https://h/x?&&a&=b&passwordless",
    "https://Grüße.example/€?KEY=~v*",
    "https://h/x#plain",
]
ATTACHMENTS = EVIDENCE + """
INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,original_filename,stored_relpath,source_url,media_type,size_bytes,sha256,copied_from_attachment_id,copied_from_transaction_id,created_at) VALUES
 ('at-good','w','p','h','file','Board decision','Grüße Board/../minutes 2024.pdf','p/good.pdf',NULL,'application/pdf',4,'{GOOD}','at-file','i','2025-01-11'),
 ('../at odd/ü','w','p','h','file',NULL,'  ','p/good.pdf',NULL,NULL,NULL,NULL,NULL,NULL,'2025-01-12'),
 ('at-unhashed','w','p','i','file','Receipt','áb.txt','p/receipt.txt',NULL,'text/plain',NULL,NULL,NULL,NULL,'2025-01-13'),
 ('at-blank','w','p','i','file','Blank path',NULL,'  ',NULL,NULL,NULL,NULL,NULL,NULL,'2025-01-14');
""".replace("{GOOD}", GOOD) + "".join(
    f"INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,source_url,created_at) "
    f"VALUES('url-{index:02d}','w','p','k','url',NULL,'{url}','2025-02-{index + 1:02d}');\n"
    for index, url in enumerate(URLS)
)


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustAuditPackageTest(DesktopParity, unittest.TestCase):
    KIND = KIND

    def setUp(self):
        super().setUp()
        self.env["KASSIBER_RUST_TEST_REPORT_TIME"] = CLOCK
        self.env["KASSIBER_RUST_TEST_OWNER_ROOT"] = str(self.base / "owners")
        (self.base / "owners").mkdir(mode=0o700)

    @property
    def exports(self):
        # The state root owning `data/` holds `exports/` and `attachments/`.
        return self.root.parent / "exports"

    def place_files(self):
        attachments = self.root.parent / "attachments" / "p"
        attachments.mkdir(parents=True, exist_ok=True)
        (attachments / "present.pdf").write_bytes(PDF)
        (attachments / "good.pdf").write_bytes(PDF)
        os.chmod(attachments / "good.pdf", 0o640)
        os.utime(attachments / "good.pdf", ns=(1_600_000_000_123_456_789, 1_600_000_001_987_654_321))
        (attachments / "receipt.txt").write_bytes("Beleg €\n".encode())

    def tree(self):
        """Every exported path: its type, permission bits, bytes, and for
        copied evidence the modification time `copy2` carries over."""
        result = {}
        for path in sorted(self.exports.rglob("*")):
            relative = str(path.relative_to(self.exports))
            info = path.stat()
            entry = {"mode": stat.S_IMODE(info.st_mode), "dir": path.is_dir()}
            if path.is_file():
                entry["bytes"] = path.read_bytes()
                if "evidence" in path.parts:
                    entry["mtime_ns"] = info.st_mtime_ns
            result[relative] = entry
        return result

    def session(self, argv_prefix, requests):
        wire = [dict(request_id=str(index), kind=KIND, **({} if args is ... else {"args": args}))
                for index, args in enumerate(requests)]
        wire.append({"request_id": "stop", "kind": "daemon.shutdown"})
        payload = "".join(json.dumps(request) + "\n" for request in wire).encode()
        argv = ["--machine", "--data-root", str(self.root)]
        if self.passphrase:
            from tests import cli_runner
            argv += ["--db-passphrase-fd", cli_runner.InputFd(self.passphrase.encode())]
        argv.append("daemon")
        if argv_prefix is None:
            result = run_candidate(argv, env=self.env, input=payload)
        else:
            fds = []
            resolved = []
            for arg in argv:
                if isinstance(arg, str):
                    resolved.append(arg)
                else:
                    read_end, write_end = os.pipe()
                    os.write(write_end, arg.data)
                    os.close(write_end)
                    fds.append(read_end)
                    resolved.append(str(read_end))
            try:
                result = subprocess.run([*argv_prefix, *resolved], env=self.env, input=payload,
                                        capture_output=True, timeout=300, cwd=ROOT, pass_fds=fds)
            finally:
                for fd in fds:
                    os.close(fd)
        self.assertEqual((result.returncode, result.stderr), (0, b""), result.stdout[-3000:])
        return result.stdout.splitlines()

    def compare_packages(self, requests):
        self.place_files()
        baseline = self.snapshot()
        shutil.rmtree(self.exports, ignore_errors=True)
        expected = self.session([sys.executable, str(REFERENCE)], requests)
        expected_tree = self.tree()
        self.assertEqual(self.snapshot(), baseline)
        shutil.rmtree(self.exports, ignore_errors=True)
        actual = self.session(None, requests)
        actual_tree = self.tree()
        self.assertEqual(self.snapshot(), baseline)
        ready = json.loads(actual[0])["data"]["supported_kinds"]
        self.assertNotEqual(ready, json.loads(expected[0])["data"]["supported_kinds"])
        self.assertIn(KIND, ready)
        self.assertEqual(len(actual), len(expected))
        for request, have, want in zip(requests, actual[1:], expected[1:]):
            self.assertEqual(have, want, request)
        self.assertEqual(sorted(actual_tree), sorted(expected_tree))
        for path in expected_tree:
            self.assertEqual(actual_tree[path], expected_tree[path], path)
        # Staging leaves no hidden directory behind.
        self.assertFalse([path for path in actual_tree if "/." in "/" + path])
        responses = [json.loads(line) for line in actual[1:-1]]
        if path := os.environ.get("KASSIBER_DESKTOP_COUNTS"):
            with open(path, "a", encoding="utf-8") as handle:
                for response in responses:
                    handle.write(json.dumps(dict(
                        kind=KIND, served_by="rust", request_id=response["request_id"],
                        ok=response["kind"] != "error",
                        error_code=response.get("error", {}).get("code"))) + "\n")
        return responses, actual_tree

    def test_whole_book_options_and_redacted_urls_match_python(self):
        self.change(ATTACHMENTS)
        responses, tree = self.compare_packages([
            ..., {}, {"include_url_references": False}, {"include_copied_attachments": False},
            {"include_review_state": False}, {"include_journal_state": False, "include_edit_history": True},
            {"include_copied_attachments": False, "include_url_references": False,
             "include_review_state": False, "include_journal_state": False},
        ])
        first = responses[0]["data"]
        self.assertGreater(first["evidence_file_count"], 1)
        self.assertGreater(first["url_reference_count"], len(URLS) - 1)
        manifest = json.loads(tree["reports/" + first["filename"] + "/manifest.json"]["bytes"])
        codes = {warning["code"] for warning in manifest["package"]["warnings"]}
        self.assertTrue({"attachment_hash_mismatch", "attachment_storage_path_invalid",
                         "secret_bearing_url_redacted", "sensitive_material_excluded"} <= codes, codes)
        self.assertTrue(any(item["redacted_url"] for item in manifest["package"]["url_references"]))

    def test_bounded_scopes_and_refusals_match_python(self):
        self.change(ATTACHMENTS)
        self.compare_packages([
            {"transaction": " h "}, {"transactions": ["i", "h", "h"]}, {"transactions": ["k"]},
            {"source_funds_case": "case"}, {"source_funds_case": " case ", "include_edit_history": True},
            "x", [], {"transaction": 3}, {"transaction": "h", "transactions": ["i"]},
            {"transactions": []}, {"include_url_references": "no"}, {"transaction": "absent"},
            {"source_funds_case": "tampered"}, {"transaction": "i"},
        ])
        self.change("DELETE FROM settings WHERE key IN ('context_workspace','context_profile');")
        self.compare_packages([{}, {"transaction": "h"}])

    def test_sqlcipher_package_matches_python(self):
        self.encrypt("synthetic audit key")
        self.change(ATTACHMENTS)
        self.compare_packages([{}, {"transactions": ["h", "i"]}])


if __name__ == "__main__":
    unittest.main()
