"""Desktop document import: staged grants, local-OCR previews and reviewed
imports compared line for line with the Python daemon.

The renderer only ever holds opaque tokens. Each daemon issues its own, so the
script reads them from the replies and the comparison masks them, along with
generated identifiers and the current clock. OCR reaches only the loopback
provider fixture; the product no-egress switch stays armed throughout.
"""
import json
import os
import re
import subprocess
import threading
import unittest
from http.server import ThreadingHTTPServer

from crates.wallets.scripts.generate_document_vectors import SOURCE, base
from kassiber.db import open_db
from tests import cli_runner
from tests.rust_desktop_parity import DESKTOP_SQL, RUST_BIN, DesktopParity
from tests.test_rust_document_cli import Handler
from tests.test_rust_wallet_identify_onchain import Session
from tests.test_rust_wallet_import_desktop import mask_line, mask_tables

STAGE = "internal.document_import.stage"
UPLOAD = "ui.chat.attachments.upload"
PREVIEW = "ui.wallets.document_import.preview"
IMPORT = "ui.wallets.document_import.import"


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustDocumentImportTest(DesktopParity, unittest.TestCase):
    KIND = PREVIEW

    def setUp(self):
        self.ocr = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.ocr.rows = [base(), {**base(), "amount_btc": "x", "confidence": 0.2}]
        self.ocr.hits = []
        thread = threading.Thread(target=self.ocr.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(self.ocr.server_close)
        self.addCleanup(self.ocr.shutdown)
        super().setUp()

    def seed(self):
        self.source = self.base / "receipt.png"
        self.source.write_bytes(SOURCE)
        with open_db(self.root, passphrase=self.passphrase) as conn:
            conn.executescript(DESKTOP_SQL.read_text())
            conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                         "VALUES('imports','w','p','Imports','custom','{}','2025')")
            conn.execute("INSERT INTO settings(key,value) VALUES('ai_providers_seeded','1') "
                         "ON CONFLICT(key) DO UPDATE SET value='1'")
            conn.execute("INSERT INTO ai_providers(name,base_url,kind,created_at,updated_at) "
                         "VALUES('local',?,'local','2026-01-01','2026-01-01')",
                         (f"http://127.0.0.1:{self.ocr.server_port}/v1",))
        self.change("")

    def run_both(self, script, *, mutates=False):
        """`script(session, step)` per daemon; responses, OCR traffic and books must match."""
        import shutil
        outputs, books = [], []
        for reference in (True, False):
            root = self.root
            if mutates and not reference:
                root = self.base / f"candidate-{len(list(self.base.glob('candidate-*')))}" / "data"
                shutil.copytree(self.root_before, root)
            self.ocr.hits.clear()
            argv = ["--machine", "--data-root", str(root)]
            options = dict(env=self.env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            if self.passphrase:
                argv += ["--db-passphrase-fd", cli_runner.InputFd(self.passphrase.encode())]
            if reference:
                process = cli_runner.popen([*argv, "daemon"], reference=True, **options)
            else:
                fds = []
                if self.passphrase:
                    read_end, write_end = os.pipe()
                    os.write(write_end, self.passphrase.encode())
                    os.close(write_end)
                    fds.append(read_end)
                    argv[-1] = str(read_end)
                process = subprocess.Popen([RUST_BIN, *argv, "daemon"], cwd=cli_runner.ROOT, pass_fds=fds, **options)
                for fd in fds:
                    os.close(fd)
            session = Session(process)
            if self.passphrase:
                session.send({"request_id": "unlock", "kind": "daemon.unlock",
                              "args": {"auth_response": {"passphrase_secret": self.passphrase}}})
                session.terminal("unlock")
            replies = {}

            def step(request_id, kind, args, *, last=False):
                session.send({"request_id": request_id, "kind": kind, "args": args})
                if last:
                    session.until(lambda r: r.get("request_id") == request_id)
                else:
                    session.terminal(request_id)
                replies[request_id] = json.loads(session.seen[-1])
                return replies[request_id]

            try:
                ended = script(session, step)
                if ended:
                    session.process.stdin.close()
                    session.process.wait(timeout=60)
                    seen, stderr = session.seen, session.process.stderr.read()
                else:
                    seen, stderr = session.finish()
            finally:
                if process.poll() is None:
                    process.kill()
            self.assertEqual(stderr, b"")
            tokens = sorted({value for reply in replies.values() if isinstance(reply.get("data"), dict)
                             for key, value in reply["data"].items()
                             if key in ("document_token", "upload_id")}, key=len, reverse=True)
            masked = []
            for line in seen:
                for token in tokens:
                    line = line.replace(token.encode(), b"<token>")
                # A pasted file's private directories have random names.
                line = re.sub(rb"kassiber-chat-uploads-[A-Za-z0-9_-]+/tmp[A-Za-z0-9_-]+/", b"<upload>/", line)
                masked.append(mask_line(line))
            outputs.append((masked, [(method, path, body) for method, path, body in self.ocr.hits]))
            kinds = session.kinds
            books.append(mask_tables(self.snapshot(root)))
        self.assertEqual(outputs[1], outputs[0])
        self.assertEqual(books[1], books[0])
        if path := os.environ.get("KASSIBER_DESKTOP_COUNTS"):
            with open(path, "a", encoding="utf-8") as handle:
                for record in map(json.loads, outputs[1][0]):
                    kind = kinds.get(json.dumps(record.get("request_id")))
                    if kind and kind != "daemon.shutdown":
                        handle.write(json.dumps(dict(kind=kind, served_by="rust",
                                                     ok=record["kind"] != "error")) + "\n")
        return [json.loads(line) for line in outputs[1][0]]

    def test_grants_previews_and_refusals_match_python(self):
        source = str(self.source)

        def script(session, step):
            for index, args in enumerate([None, [], {}, {"source_file": 3}, {"source_file": "/no/such/receipt.png"},
                                          {"source_file": source, "review_recipe": {}},
                                          {"source_file": source, "review_case_id": "x", "review_recipe": {}},
                                          {"source_file": source, "review_case_id": "source_funds:x"}]):
                step(f"s{index}", STAGE, args)
            token = step("stage", STAGE, {"source_file": source})["data"]["document_token"]
            step("analyze", "ui.wallets.analyze_file", {"token": token})
            # Staging and analysis never reach the OCR provider; only the preview does.
            self.assertEqual(self.ocr.hits, [])
            for index, args in enumerate([{}, {"document_token": " "}, {"document_token": "unknown"},
                                          {"document_token": token, "rows": []},
                                          {"document_token": token, "source_file": source},
                                          {"document_token": token, "confidence_threshold": "x"},
                                          {"document_token": token, "max_pages": 0}]):
                step(f"p{index}", PREVIEW, args)
            reply = step("preview", PREVIEW, {"document_token": token, "provider": "local"})
            assert "data" in reply, reply
            preview = reply["data"]
            ready = [row["id"] for row in preview["rows"] if row["status"] == "ready"]
            for index, args in enumerate([{"document_token": preview["document_token"]},
                                          {"document_token": token, "wallet": "imports", "selected_row_ids": ready},
                                          {"document_token": preview["document_token"], "wallet": "imports"},
                                          {"document_token": preview["document_token"], "wallet": "imports",
                                           "selected_row_ids": "x"},
                                          {"document_token": preview["document_token"], "wallet": "imports",
                                           "selected_row_ids": [""]},
                                          {"document_token": preview["document_token"], "wallet": "imports",
                                           "selected_row_ids": ["nope"]},
                                          {"document_token": preview["document_token"], "wallet": "imports",
                                           "selected_row_ids": [row["id"] for row in preview["rows"]]},
                                          {"document_token": preview["document_token"], "wallet": "absent",
                                           "selected_row_ids": ready},
                                          {"document_token": preview["document_token"], "wallet": "imports",
                                           "selected_row_ids": ready, "include_quarantined": True}]):
                step(f"i{index}", IMPORT, args)
            step("lock", "daemon.lock", None)
            step("after-lock", PREVIEW, {"document_token": token, "provider": "local"})
            return False

        records = self.run_both(script)
        kinds = [r["kind"] for r in records]
        self.assertIn(PREVIEW, kinds)
        self.assertIn(STAGE, kinds)

    def test_chat_attachments_stage_and_upload_like_python(self):
        """#689: a chat pick stages exports too and names their kind; a paste
        arrives in chunks into a file the daemon owns. (A chat image would
        carry Python's Pillow thumbnail, which the Rust daemon does not make,
        so the picks here are an export and a PDF.)"""
        export = self.base / "Kraken export.csv"
        export.write_text("time,amount\n2025-01-01,1\n")
        statement = self.base / "statement.pdf"
        statement.write_bytes(b"%PDF-1.4\n%%EOF\n")
        notes = self.base / "notes.txt"
        notes.write_text("x")

        def script(session, step):
            for index, args in enumerate([
                {"source_file": str(export), "purpose": "chat_attachment"},
                {"source_file": str(statement), "purpose": "chat_attachment"},
                {"source_file": str(self.source), "purpose": "document_import"},
                {"source_file": str(export)},
                {"source_file": str(export), "purpose": "x"},
                {"source_file": str(notes), "purpose": "chat_attachment"},
                {"source_file": str(self.base / "missing.csv"), "purpose": "chat_attachment"},
                {"source_file": "~/no-such-kassiber-file.csv", "purpose": "chat_attachment"},
            ]):
                step(f"s{index}", STAGE, args)
            for index, args in enumerate([
                None, {}, {"bogus": 1, "offset": 0},
                {"offset": 0}, {"offset": 0, "chunk_base64": "YWJj="},
                {"offset": 1, "chunk_base64": "YWJj", "filename": "a.csv", "total_bytes": 6},
                {"offset": 0, "chunk_base64": "YWJj", "filename": "a.txt", "total_bytes": 6},
                {"offset": 0, "chunk_base64": "YWJj", "filename": "a.csv", "total_bytes": 0},
                {"offset": 0, "chunk_base64": "YWJj", "filename": "a.csv", "total_bytes": 26 * 1024 * 1024},
                {"offset": 0, "chunk_base64": "YWJj", "upload_id": "gone"},
                {"offset": True, "chunk_base64": "YWJj", "filename": "a.csv", "total_bytes": 6},
            ]):
                step(f"u{index}", UPLOAD, args)
            first = step("first", UPLOAD, {"offset": 0, "chunk_base64": "YSxi",
                                            "filename": "C:\\Downloads\\paste.csv", "total_bytes": 9})
            assert "data" in first, first
            upload_id = first["data"]["upload_id"]
            step("renamed", UPLOAD, {"offset": 3, "chunk_base64": "CjEs", "upload_id": upload_id,
                                     "filename": "b.csv"})
            # A refused continuation ends nothing but itself; the upload goes on.
            second = step("second", UPLOAD, {"offset": 3, "chunk_base64": "CjEs", "upload_id": upload_id})
            assert second["data"]["complete"] is False, second
            done = step("done", UPLOAD, {"offset": 6, "chunk_base64": "Mgoz", "upload_id": upload_id})
            step("late", UPLOAD, {"offset": 9, "chunk_base64": "Mgoz", "upload_id": upload_id})
            step("analyze", "ui.wallets.analyze_file", {"token": done["data"]["document_token"]})
            out = step("order", UPLOAD, {"offset": 0, "chunk_base64": "YWJj", "filename": "o.csv",
                                         "total_bytes": 9})
            step("skip", UPLOAD, {"offset": 6, "chunk_base64": "YWJj", "upload_id": out["data"]["upload_id"]})
            step("after-skip", UPLOAD, {"offset": 3, "chunk_base64": "YWJj", "upload_id": out["data"]["upload_id"]})
            step("lock", "daemon.lock", None)
            step("after-lock", "ui.wallets.analyze_file", {"token": done["data"]["document_token"]})
            return False

        records = self.run_both(script)
        kinds = [r["kind"] for r in records]
        self.assertIn(UPLOAD, kinds)

    def test_reviewed_rows_import_and_end_the_session(self):
        self.import_reviewed_rows()

    def test_sqlcipher_import_matches_python(self):
        self.encrypt("synthetic document key")
        self.import_reviewed_rows()

    def test_profile_switch_withdraws_grants(self):
        def script(session, step):
            token = step("stage", STAGE, {"source_file": str(self.source)})["data"]["document_token"]
            step("switch", "ui.profiles.switch", {"workspace_id": "w", "profile_id": "p"})
            step("after-switch", PREVIEW, {"document_token": token, "provider": "local"})
            return False

        records = self.run_both(script)
        self.assertEqual(records[-2]["error"]["code"], "document_import_session_expired")

    def import_reviewed_rows(self):
        self.root_before = self.base / "before" / self.root.name
        import shutil
        shutil.copytree(self.root, self.root_before)

        def script(session, step):
            token = step("stage", STAGE, {"source_file": str(self.source)})["data"]["document_token"]
            reply = step("preview", PREVIEW, {"document_token": token, "provider": "local"})
            assert "data" in reply, reply
            preview = reply["data"]
            ready = [row["id"] for row in preview["rows"] if row["status"] == "ready"]
            reply = step("import", IMPORT, {"document_token": preview["document_token"], "wallet": "imports",
                                            "selected_row_ids": ready}, last=True)
            self.assertEqual(reply["kind"], IMPORT, reply)
            return True

        self.run_both(script, mutates=True)
