"""A file pasted into the chat: chunked, owner-only, staged like a pick, deleted after.

Pinned here: the chunks reassemble to the exact bytes; the file is staged with
the same token and metadata a picked file gets, and no path reaches the
renderer; the name keeps no directory; unsupported types, oversized files and
out-of-order chunks are refused and leave nothing behind; and the file goes
when its grant is released or expires.
"""

from __future__ import annotations

import base64
import json
import os
import queue
import stat
import subprocess
import sys
import tempfile
import threading
import unittest

from kassiber import daemon
from kassiber import daemon_chat_uploads
from kassiber.db import open_db
from kassiber.errors import AppError


REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
KIND = daemon_chat_uploads.KIND


class _Clock:
    def __init__(self):
        self.now = 1000.0

    def __call__(self):
        return self.now


class ChatAttachmentUploadTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="kassiber-chat-upload-")
        self.addCleanup(self._tmp.cleanup)
        self.data_root = os.path.join(self._tmp.name, "data")
        for args in (
            ("init",),
            ("workspaces", "create", "W"),
            ("profiles", "create", "P", "--workspace", "W",
             "--fiat-currency", "EUR", "--tax-country", "at"),
        ):
            subprocess.run(
                [sys.executable, "-m", "kassiber", "--data-root", self.data_root,
                 "--machine", *args],
                capture_output=True, text=True, cwd=REPO_ROOT, check=True,
            )
        self.conn = open_db(self.data_root)
        self.addCleanup(self.conn.close)
        self.clock = _Clock()
        self.ctx = daemon.DaemonContext(
            conn=self.conn,
            data_root=self.data_root,
            runtime_config={},
            active_ai_chats=daemon.ActiveAiChats(),
            main_thread_tasks=queue.Queue(),
            auth_backoff=daemon.AuthAttemptBackoff(),
            input_lines=queue.Queue(),
            deferred_input_lines=[],
            out=None,
            freshness_stop_event=threading.Event(),
            document_import_sessions=daemon.DocumentImportSessions(clock=self.clock),
        )
        self.addCleanup(self.ctx.document_import_sessions.clear)

    def request(self, kind, args):
        try:
            envelope, _ = daemon.handle_request(
                self.ctx, {"kind": kind, "request_id": "req-1", "args": args}, out=None,
            )
        except AppError as exc:
            return {"kind": "error", "error": {"code": exc.code}}
        return envelope

    def upload(self, content, filename="export.csv", chunk_size=daemon_chat_uploads.MAX_CHUNK_BYTES):
        envelopes = []
        upload_id = None
        for offset in range(0, len(content), chunk_size):
            args = {
                "offset": offset,
                "chunk_base64": base64.b64encode(content[offset:offset + chunk_size]).decode(),
            }
            if upload_id is None:
                args.update(filename=filename, total_bytes=len(content))
            else:
                args["upload_id"] = upload_id
            envelope = self.request(KIND, args)
            envelopes.append(envelope)
            if envelope["kind"] != KIND or envelope["data"]["complete"]:
                break
            upload_id = envelope["data"]["upload_id"]
        return envelopes

    def staged_path(self, token):
        workspace, profile = daemon.resolve_scope(self.conn, None, None)
        return self.ctx.document_import_sessions.source_for_preview(
            token,
            workspace_id=str(workspace["id"]),
            profile_id=str(profile["id"]),
            data_root=self.data_root,
        )

    def test_a_pasted_file_is_staged_like_a_picked_one(self):
        content = b"date,amount\n" + b"2024-01-01,0.1\n" * 90_000
        envelopes = self.upload(content)
        self.assertGreater(len(envelopes), 2)
        for partial in envelopes[:-1]:
            self.assertIs(partial["data"]["complete"], False)
        data = envelopes[-1]["data"]
        self.assertIs(data["complete"], True)
        self.assertEqual(
            data["source"],
            {"filename": "export.csv", "media_type": "text/csv",
             "size_bytes": len(content), "kind": "delimited"},
        )
        path = self.staged_path(data["document_token"])
        with open(path, "rb") as handle:
            self.assertEqual(handle.read(), content)
        self.assertEqual(stat.S_IMODE(os.stat(path).st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(os.stat(os.path.dirname(path)).st_mode), 0o700)
        # The renderer gets a token and safe metadata, never where the file is.
        for envelope in envelopes:
            self.assertNotIn(os.path.dirname(path), json.dumps(envelope))

    def test_a_name_keeps_no_directory(self):
        data = self.upload(b"%PDF-1.4\n", filename="../../etc/Kontoauszug.pdf")[-1]["data"]
        self.assertEqual(data["source"]["filename"], "Kontoauszug.pdf")
        self.assertEqual(data["source"]["kind"], "pdf")
        path = self.staged_path(data["document_token"])
        self.assertEqual(os.path.basename(path), "Kontoauszug.pdf")
        self.assertTrue(os.path.basename(os.path.dirname(os.path.dirname(path))).startswith(
            "kassiber-chat-uploads-"))

    def test_the_file_goes_with_its_grant(self):
        data = self.upload(b"a,b\n1,2\n")[-1]["data"]
        path = self.staged_path(data["document_token"])
        self.ctx.document_import_sessions.consume(data["document_token"])
        self.assertFalse(os.path.exists(path))
        self.assertFalse(os.path.exists(os.path.dirname(path)))

        data = self.upload(b"a,b\n1,2\n")[-1]["data"]
        path = self.staged_path(data["document_token"])
        self.clock.now += daemon.DOCUMENT_IMPORT_SESSION_TTL_SECONDS + 1
        self.ctx.document_import_sessions.stage(
            source_file=os.path.join(self._tmp.name, "other.csv"),
            workspace_id="w", profile_id="p", data_root=self.data_root,
        )
        self.assertFalse(os.path.exists(path))

        data = self.upload(b"a,b\n1,2\n")[-1]["data"]
        path = self.staged_path(data["document_token"])
        self.ctx.document_import_sessions.clear()
        self.assertFalse(os.path.exists(path))

    def test_a_picked_file_is_never_deleted(self):
        picked = os.path.join(self._tmp.name, "mine.csv")
        with open(picked, "w", encoding="utf-8") as handle:
            handle.write("a,b\n1,2\n")
        envelope = self.request(
            "internal.document_import.stage",
            {"source_file": picked, "purpose": "chat_attachment"},
        )
        self.assertEqual(envelope["data"]["source"]["kind"], "delimited")
        self.ctx.document_import_sessions.clear()
        self.assertTrue(os.path.exists(picked))

    def test_the_chat_picker_stages_exports_and_the_ocr_picker_does_not(self):
        picked = os.path.join(self._tmp.name, "Umsatzliste.xlsx")
        with open(picked, "wb") as handle:
            handle.write(b"PK\x03\x04")
        chat = self.request(
            "internal.document_import.stage",
            {"source_file": picked, "purpose": "chat_attachment"},
        )
        self.assertEqual(chat["data"]["source"]["kind"], "spreadsheet")
        ocr = self.request("internal.document_import.stage", {"source_file": picked})
        self.assertEqual(ocr["error"]["code"], "document_import_source_unavailable")
        archive = os.path.join(self._tmp.name, "wallet.zip")
        with open(archive, "wb") as handle:
            handle.write(b"PK")
        refused = self.request(
            "internal.document_import.stage",
            {"source_file": archive, "purpose": "chat_attachment"},
        )
        self.assertEqual(refused["error"]["code"], "chat_attachment_unsupported")

    def test_refusals_leave_nothing_behind(self):
        def code(envelope):
            return envelope["error"]["code"]

        self.assertEqual(code(self.upload(b"MZ", filename="setup.exe")[-1]), "chat_attachment_unsupported")
        too_large = self.request(KIND, {
            "filename": "big.csv", "total_bytes": daemon_chat_uploads.MAX_UPLOAD_BYTES + 1,
            "offset": 0, "chunk_base64": base64.b64encode(b"a").decode(),
        })
        self.assertEqual(code(too_large), "chat_attachment_too_large")
        self.assertEqual(code(self.request(KIND, {
            "filename": "a.csv", "total_bytes": 4, "offset": 0, "chunk_base64": "not base64!",
        })), "validation")
        self.assertEqual(code(self.request(KIND, {
            "filename": "a.csv", "total_bytes": 4, "offset": 0,
            "chunk_base64": base64.b64encode(b"ab").decode(), "path": "/etc/passwd",
        })), "validation")

        first = self.request(KIND, {
            "filename": "a.csv", "total_bytes": 4, "offset": 0,
            "chunk_base64": base64.b64encode(b"ab").decode(),
        })["data"]
        uploads = self.ctx.document_import_sessions.uploads
        pending = uploads._pending[first["upload_id"]].path
        self.assertTrue(os.path.exists(pending))
        out_of_order = self.request(KIND, {
            "upload_id": first["upload_id"], "offset": 1,
            "chunk_base64": base64.b64encode(b"cd").decode(),
        })
        self.assertEqual(code(out_of_order), "chat_attachment_upload_expired")
        self.assertFalse(os.path.exists(pending))

        first = self.request(KIND, {
            "filename": "a.csv", "total_bytes": 4, "offset": 0,
            "chunk_base64": base64.b64encode(b"ab").decode(),
        })["data"]
        pending = uploads._pending[first["upload_id"]].path
        overrun = self.request(KIND, {
            "upload_id": first["upload_id"], "offset": 2,
            "chunk_base64": base64.b64encode(b"cde").decode(),
        })
        self.assertEqual(code(overrun), "validation")
        self.assertFalse(os.path.exists(pending))

        first = self.request(KIND, {
            "filename": "a.csv", "total_bytes": 4, "offset": 0,
            "chunk_base64": base64.b64encode(b"ab").decode(),
        })["data"]
        pending = uploads._pending[first["upload_id"]].path
        self.clock.now += daemon_chat_uploads.UPLOAD_TTL_SECONDS + 1
        expired = self.request(KIND, {
            "upload_id": first["upload_id"], "offset": 2,
            "chunk_base64": base64.b64encode(b"cd").decode(),
        })
        self.assertEqual(code(expired), "chat_attachment_upload_expired")
        self.assertFalse(os.path.exists(pending))


class ChatUploadScopeTests(unittest.TestCase):
    def test_an_upload_continues_only_in_its_book(self):
        uploads = daemon_chat_uploads.ChatUploads()
        self.addCleanup(uploads.clear)
        started = uploads.receive(
            {"filename": "a.csv", "total_bytes": 4, "offset": 0,
             "chunk_base64": base64.b64encode(b"ab").decode()},
            workspace_id="w1", profile_id="p1", data_root="/books",
        )
        with self.assertRaises(AppError) as raised:
            uploads.receive(
                {"upload_id": started["upload_id"], "offset": 2,
                 "chunk_base64": base64.b64encode(b"cd").decode()},
                workspace_id="w2", profile_id="p1", data_root="/books",
            )
        self.assertEqual(raised.exception.code, "chat_attachment_upload_expired")


if __name__ == "__main__":
    unittest.main()
