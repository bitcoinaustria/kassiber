"""#698: the assistant reads a chat's photo with the local vision model and
imports the draft's ready rows, compared record for record with Python.

`ui.wallets.read_document` resolves the reader and stores the draft on the
request loop and runs the model call between the two; `ui.wallets.
import_document.plan` and `.apply` bind the import to the draft, the
document's digest, the book's input version and the plan digest. Both
daemons run the same chained requests through the test-harness kinds against
the loopback OCR fixture, with the product no-egress switch armed; every
record, every OCR request and every table must match. Each daemon issues its
own grant and draft tokens, which the comparison masks with generated ids and
clocks.
"""
import json
import os
import select
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from http.server import ThreadingHTTPServer
from pathlib import Path

from crates.wallets.scripts.generate_document_vectors import SOURCE, base
from kassiber.db import open_db
from tests.rust_desktop_parity import RUST_BIN, DesktopParity
from tests.test_rust_document_cli import Handler
from tests.test_rust_wallet_import_desktop import mask_line, mask_tables
from tests import cli_runner
from tests.egress_guard import guarded_pythonpath

ROOT = Path(__file__).resolve().parent.parent
REFERENCE = ROOT / "tests" / "rust_ai_tool_reference.py"
REMOTE = {"provider_kind": "remote", "provider_on_device": False}
LOCAL = {"provider_kind": "local", "provider_on_device": True}
SCOPE = {"scope_workspace_id": "w", "scope_profile_id": "p"}


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustAiDocumentImportTest(DesktopParity, unittest.TestCase):
    KIND = "test.ai_tool"
    maxDiff = None

    def setUp(self):
        self.ocr = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.ocr.rows = [base(), {**base(), "amount_btc": "x", "confidence": 0.2}]
        self.ocr.hits = []
        thread = threading.Thread(target=self.ocr.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(self.ocr.server_close)
        self.addCleanup(self.ocr.shutdown)
        super().setUp()
        self.env["PYTHONPATH"] = guarded_pythonpath()
        owners = self.base / "owners"
        owners.mkdir()
        self.env["KASSIBER_RUST_TEST_OWNER_ROOT"] = str(owners)

    def seed(self):
        self.source = self.base / "receipt.png"
        self.source.write_bytes(SOURCE)
        self.change((Path(self.SEED).read_text()))
        with open_db(self.root) as conn:
            conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                         "VALUES('imports','w','p','Imports','custom','{}','2025')")
            conn.execute("INSERT INTO settings(key,value) VALUES('ai_providers_seeded','1') "
                         "ON CONFLICT(key) DO UPDATE SET value='1'")
            conn.execute("INSERT INTO ai_providers(name,base_url,kind,created_at,updated_at) "
                         "VALUES('local',?,'local','2026-01-01','2026-01-01')",
                         (f"http://127.0.0.1:{self.ocr.server_port}/v1",))
            conn.execute("INSERT INTO ai_providers(name,base_url,kind,created_at,updated_at) "
                         "VALUES('cloud','https://api.example.invalid/v1','openai','2026-01-01','2026-01-01')")
        self.change("")

    def chained(self, argv_prefix, root, steps):
        """Requests built one at a time from every earlier record."""
        process = subprocess.Popen(
            [*argv_prefix, "--machine", "--data-root", str(root), "daemon"],
            env=self.env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, cwd=ROOT,
        )
        try:
            lines = [process.stdout.readline()]
            for index, step in enumerate(steps, start=1):
                request = step([json.loads(line) for line in lines[1:]])
                process.stdin.write((json.dumps(dict(request, request_id=str(index))) + "\n").encode())
                process.stdin.flush()
                lines.append(process.stdout.readline())
            process.stdin.close()
            self.assertEqual(process.wait(timeout=300), 0)
            self.assertEqual(process.stderr.read(), b"")
        finally:
            if process.poll() is None:
                process.kill()
        return lines[1:]

    def run_both(self, steps):
        import shutil

        results = []
        for side, prefix in enumerate(([sys.executable, str(REFERENCE)], [str(Path(RUST_BIN).resolve())])):
            root = self.base / f"side-{side}" / "data"
            shutil.copytree(self.root, root)
            self.ocr.hits.clear()
            lines = self.chained(prefix, root, steps)
            records = [json.loads(line) for line in lines]
            tokens = set()
            for record in records:
                data = record.get("data") or {}
                if isinstance(data.get("document_token"), str):
                    tokens.add(data["document_token"])
                output = data.get("output")
                if isinstance(output, str):
                    envelope = json.loads(output).get("envelope") or {}
                    draft = (envelope.get("data") or {}).get("draft")
                    if isinstance(draft, str):
                        tokens.add(draft)
            masked = []
            for line in lines:
                for token in sorted(tokens, key=len, reverse=True):
                    line = line.replace(token.encode(), b"<token>")
                masked.append(mask_line(line))
            results.append((masked, [(method, path, body) for method, path, body in self.ocr.hits],
                            mask_tables(self.snapshot(root)), records))
        self.assertEqual(results[1][0], results[0][0])
        self.assertEqual(results[1][1], results[0][1])
        self.assertEqual(results[1][2], results[0][2])
        return results[1][3]

    def test_reading_and_importing_a_chat_photo_matches_python(self):
        def tool(name, arguments, call_id, state):
            return {"kind": "test.ai_tool", "args": {"name": name, "arguments": arguments, "call_id": call_id,
                    "state": {**state, "consented_call_ids": [call_id]}}}

        def files(records, kind="image"):
            token = records[0]["data"]["document_token"]
            return {"chat_files": [{"id": "file-1", "source_file": str(self.source), "filename": "receipt.png",
                                    "label": "Receipt", "kind": kind, "new": True, "token": token}]}

        def state(records, access=LOCAL, sessions=True, kind="image"):
            return {**access, **SCOPE, **files(records, kind),
                    **({"document_import_sessions": True} if sessions else {})}

        def output(records, index):
            return json.loads(records[index]["data"]["output"])

        def draft(records):
            read = output(records, 3)
            return read["envelope"]["data"]["draft"] if read.get("ok") else "missing"

        def approved(records):
            planned = output(records, 7)
            with_ = planned["envelope"]["data"]["apply_with"] if planned.get("ok") else {}
            return {"draft": draft(records), "wallet": "Imports", **with_}

        steps = [
            lambda r: {"kind": "internal.document_import.stage",
                       "args": {"source_file": str(self.source), "purpose": "chat_attachment"}},
            # Outside a chat there are no document grants; a spreadsheet kind
            # and an unknown file are refused before the reader is resolved.
            lambda r: tool("ui_wallets_read_document", {}, "call_nochat", state(r, sessions=False)),
            lambda r: tool("ui_wallets_read_document", {}, "call_kind", state(r, kind="delimited")),
            lambda r: tool("ui_wallets_read_document", {"file": "file-1"}, "call_read", state(r)),
            lambda r: tool("ui_wallets_read_document", {"provider": "cloud"}, "call_cloud", state(r)),
            lambda r: tool("ui_wallets_read_document", {"pages": [9]}, "call_pages", state(r, access=REMOTE)),
            lambda r: tool("ui_wallets_read_document", {}, "call_restricted", state(r, access=REMOTE)),
            lambda r: tool("ui_wallets_import_document_plan", {"draft": draft(r), "wallet": "Imports"},
                           "call_plan", state(r)),
            lambda r: tool("ui_wallets_import_document_plan", {"draft": draft(r), "new_wallet": {"label": "Receipts"}},
                           "call_plan_new", state(r, access=REMOTE)),
            lambda r: tool("ui_wallets_import_document_plan", {"draft": draft(r), "wallet": "cold"},
                           "call_plan_cold", state(r)),
            lambda r: tool("ui_wallets_import_document_plan", {"draft": "missing", "wallet": "Imports"},
                           "call_plan_missing", state(r)),
            lambda r: tool("ui_wallets_import_document_plan", {"wallet": "Imports"}, "call_plan_bare", state(r)),
            lambda r: {"kind": "test.ai_consent_preview", "args": {
                "name": "ui_wallets_import_document_apply", "arguments": approved(r), "call_id": "call_apply",
                "state": state(r)}},
            lambda r: tool("ui_wallets_import_document_apply", {**approved(r), "plan_digest": "0" * 64},
                           "call_tampered", state(r)),
            lambda r: tool("ui_wallets_import_document_apply", {**approved(r), "expected_source_sha256": "0" * 64},
                           "call_changed", state(r)),
            lambda r: tool("ui_wallets_import_document_apply", approved(r), "call_apply", state(r)),
            lambda r: tool("ui_wallets_import_document_apply", approved(r), "call_again", state(r)),
            lambda r: tool("ui_imports_list", {}, "call_runs", state(r)),
        ]
        records = self.run_both(steps)
        reasons = [json.loads(record["data"]["output"]).get("reason") if "output" in record.get("data", {}) else None
                   for record in records]
        self.assertEqual(reasons[1:3], ["ai_attachment_missing", "ai_attachment_unsupported"])
        read = output(records, 3)
        self.assertTrue(read["ok"], read)
        self.assertEqual(read["envelope"]["data"]["summary"]["ready"], 1)
        self.assertNotIn("evidence_text", records[3]["data"]["output"])
        # A remote provider is refused before any contact: it was never acknowledged.
        self.assertEqual(reasons[4], "ai_remote_ack_required")
        restricted = output(records, 6)["envelope"]["data"]
        self.assertTrue(restricted["row_values_withheld"])
        self.assertNotIn("rows", restricted)
        plan = output(records, 7)["envelope"]["data"]
        self.assertTrue(plan["ready"], plan)
        self.assertEqual(plan["warnings"], ["rows_held_for_review"])
        self.assertNotIn("row_ids", output(records, 8)["envelope"]["data"])
        self.assertEqual(reasons[13:15], ["import_plan_stale", "attachment_changed"])
        applied = output(records, 15)
        self.assertTrue(applied["ok"], applied)
        self.assertEqual(applied["envelope"]["data"]["evidence_attached"], 1)
        # The draft was consumed by the apply.
        self.assertEqual(reasons[16], "document_import_session_expired")
        # The model call reached only the loopback reader: one listing and
        # one OCR request per successful read.
        self.assertTrue(all(path.startswith("/v1/") for _, path, _ in self.ocr.hits))


if __name__ == "__main__":
    unittest.main()


class _BlockingOcr(Handler):
    """The OCR fixture, holding its model answer until the test releases it."""

    def do_POST(self):
        self.server.started.set()
        self.server.release.wait(20)
        super().do_POST()
        self.server.answered.set()


@unittest.skipUnless(RUST_BIN and os.name == "posix", "set KASSIBER_RUST_BIN to the harness build")
class RustAiDocumentReaderOffLoopTest(unittest.TestCase):
    """The reader's model call runs on the chat's worker: while the local
    model reads the photo, the daemon keeps answering other requests. Both
    daemons are checked; the Rust one is the subject."""

    def setUp(self):
        from tests.test_ai_core_http_transport import ScriptedProvider
        from tests.test_rust_shell_parity import _environment

        directory = tempfile.TemporaryDirectory(prefix="kassiber-ai-reader-")
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name)
        self.root = self.base / "data"
        self.env = _environment(self.base / "preferences")
        self.env["HOME"] = os.environ["HOME"]
        self.env.update({"KASSIBER_STATE_ROOT": str(self.base / "state"),
                         "KASSIBER_RUST_TEST_AI_TIME": "2026-11-05T12:00:00Z",
                         "KASSIBER_RUST_TEST_AI_CHECKED_AT": "2026-11-05T12:00:00.123456Z",
                         "KASSIBER_NO_EGRESS": "1", "KASSIBER_TEST_NO_EGRESS": "1"})
        self.source = self.base / "receipt.png"
        self.source.write_bytes(SOURCE)
        self.chat = ScriptedProvider()
        self.addCleanup(self.chat.stop)
        self.ocr = ThreadingHTTPServer(("127.0.0.1", 0), _BlockingOcr)
        self.ocr.rows = [base()]
        self.ocr.hits = []
        thread = threading.Thread(target=self.ocr.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(self.ocr.server_close)
        self.addCleanup(self.ocr.shutdown)
        for args in (["init"], ["workspaces", "create", "Demo"], ["profiles", "create", "Main"]):
            result = cli_runner.run(["--machine", "--data-root", str(self.root), *args], reference=True,
                                    capture_output=True, env=self.env, cwd=ROOT, timeout=120)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def run_daemon(self, command, root):
        from tests.test_rust_ai_chat import _answer, _function_call
        from tests.test_ai_core_http_transport import responses_stream

        self.ocr.started, self.ocr.release, self.ocr.answered = (
            threading.Event(), threading.Event(), threading.Event())
        with self.chat.lock:
            self.chat.routes.clear()
            self.chat.requests.clear()
        self.chat.queue("POST", "responses",
                        responses_stream([_function_call("call_read", "ui_wallets_read_document",
                                                         '{"file":"file-1","provider":"ocr"}')]),
                        _answer("One row is ready."))
        process = subprocess.Popen([*command, "--data-root", str(root), "daemon"], stdin=subprocess.PIPE,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=self.env, cwd=ROOT)
        self.addCleanup(lambda: process.poll() is None and process.kill())
        buffer = b""

        def record(timeout=60):
            nonlocal buffer
            deadline = time.monotonic() + timeout
            while b"\n" not in buffer:
                ready, _, _ = select.select([process.stdout], [], [], max(0, deadline - time.monotonic()))
                self.assertTrue(ready, "timed out")
                chunk = os.read(process.stdout.fileno(), 65536)
                if not chunk:
                    self.fail(process.stderr.read()[-2000:])
                buffer += chunk
            line, buffer = buffer.split(b"\n", 1)
            return json.loads(line)

        def send(request):
            process.stdin.write(json.dumps(request).encode() + b"\n")
            process.stdin.flush()

        def until(request_id):
            while (reply := record()).get("request_id") != request_id:
                pass
            return reply

        self.assertEqual(record()["kind"], "daemon.ready")
        for index, request in enumerate([
            {"kind": "ai.providers.create", "args": {"name": "tool-local", "base_url": self.chat.base_url,
                                                     "kind": "local", "default_model": "test-model"}},
            {"kind": "ai.providers.set_default", "args": {"name": "tool-local"}},
            {"kind": "ai.providers.create", "args": {"name": "ocr", "kind": "local",
                                                     "base_url": f"http://127.0.0.1:{self.ocr.server_port}/v1"}},
            {"kind": "internal.document_import.stage",
             "args": {"source_file": str(self.source), "purpose": "chat_attachment"}},
        ]):
            send({"request_id": f"s{index}", **request})
            reply = until(f"s{index}")
            self.assertNotEqual(reply["kind"], "error", reply)
        token = reply["data"]["document_token"]
        send({"request_id": "chat", "kind": "ai.chat", "args": {
            "model": "test-model", "tools_enabled": True, "tool_profile": "full",
            "messages": [{"role": "user", "content": "Read the receipt"}],
            "attachments": [{"token": token, "new": True}]}})
        while (seen := record())["kind"] != "ai.chat.tool_consent_required":
            self.assertEqual(seen.get("request_id"), "chat", seen)
        send({"request_id": "allow", "kind": "ai.tool_call.consent", "args": {
            "target_request_id": "chat", "call_id": seen["data"]["call_id"], "decision": "allow_once"}})
        self.assertTrue(self.ocr.started.wait(60), "the reader never called the model")
        # The model is still reading: another request is answered meanwhile.
        send({"request_id": "status", "kind": "status"})
        records = []
        while (seen := record()).get("request_id") != "status":
            records.append(seen)
        self.assertFalse(self.ocr.answered.is_set(), "status waited for the model")
        self.assertEqual(seen["kind"], "status")
        self.ocr.release.set()
        while not ((seen := record()).get("request_id") == "chat" and seen["kind"] in {"ai.chat", "error"}):
            records.append(seen)
        results = [r for r in records + [seen] if r["kind"] == "ai.chat.tool_result"]
        self.records = records
        send({"request_id": "stop", "kind": "daemon.shutdown"})
        until("stop")
        process.stdin.close()
        self.assertEqual(process.wait(timeout=60), 0)
        return seen, results

    def test_other_requests_are_answered_while_the_model_reads(self):
        import shutil

        for side, command in (("python", [sys.executable, "-m", "tests.rust_port.ai_chat_reference"]),
                              ("rust", [str(Path(RUST_BIN).resolve())])):
            with self.subTest(side=side):
                root = self.base / side / "data"
                shutil.copytree(self.root, root)
                terminal, results = self.run_daemon(command, root)
                self.assertEqual(terminal["kind"], "ai.chat", terminal)
                self.assertEqual(len(results), 1, (terminal, results))
                self.assertTrue(results[0]["data"]["ok"], results[0])
                self.assertEqual(results[0]["data"]["envelope"]["data"]["summary"]["rows"], 1)
