"""Check `kassiber chat --file` attaches the file.

    uv run --locked python tests/rust_port/repro_chat_file_token_key.py [RUST_BIN]

`_stage_chat_attachments` (kassiber/cli/chat.py) staged each `--file` with
`internal.document_import.stage` and then read the grant from
``data["token"]``, but the daemon returns it as ``data["document_token"]``.
Every `--file` therefore failed with `ai_attachment_failed` ("could not attach
the file to this chat") after the daemon had already staged it. The Rust CLI
reads `document_token` (crates/cli/src/chat/mod.rs).

This runs a one-shot `chat --file` through the whole command line, Python's
or the given Rust binary, against a loopback provider on a disposable data
root under target/, and checks that the turn succeeds and the provider's
request names the file. Exit 0 means fixed.
"""
from __future__ import annotations

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
from tests.egress_guard import guarded_pythonpath

ROOT = Path(__file__).resolve().parents[2]


class _Provider(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def _send(self, status, body, content_type="application/json"):
        self.send_response(status)
        self.send_header("content-type", content_type)
        self.send_header("content-length", str(len(body)))
        self.send_header("connection", "close")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):  # noqa: N802 - http.server API
        self._send(200, json.dumps({"object": "list", "data": [{"id": "test-model", "object": "model"}]}).encode())

    def do_POST(self):  # noqa: N802 - http.server API
        body = self.rfile.read(int(self.headers.get("content-length") or "0"))
        self.server.posts.append((self.path, body))
        if self.path != "/v1/responses":
            self._send(404, b"")
            return
        done = {"type": "response.completed", "response": {"id": "resp_1", "status": "completed", "output": [
            {"type": "message", "role": "assistant", "status": "completed",
             "content": [{"type": "output_text", "text": "Seen."}]}]}}
        self._send(200, f"data: {json.dumps(done)}\n\ndata: [DONE]\n\n".encode(), "text/event-stream")

    def log_message(self, *args):
        return


def main() -> int:
    prefix = [sys.argv[1]] if len(sys.argv) > 1 else [sys.executable, "-m", "kassiber"]
    scratch = ROOT / "target"
    scratch.mkdir(exist_ok=True)
    server = ThreadingHTTPServer(("127.0.0.1", 0), _Provider)
    server.posts = []
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix="chat-file-repro-", dir=scratch.resolve()) as directory:
            base = Path(directory)
            root = base / "data"
            export = base / "export.csv"
            export.write_text("time,amount\n2025-01-01,1\n")
            env = {
                **os.environ,
                "PYTHONPATH": guarded_pythonpath(),
                "KASSIBER_OFFLINE_PREFERENCE_FILE": str(base / "offline-mode.json"),
            }
            env.pop("XDG_DATA_HOME", None)
            cli = [*prefix, "--machine", "--data-root", str(root)]
            base_url = f"http://127.0.0.1:{server.server_port}/v1"
            for command in (["init"], ["workspaces", "create", "Personal"],
                            ["profiles", "create", "--workspace", "Personal", "--fiat-currency", "EUR", "Main"],
                            ["ai", "providers", "create", "fake", "--base-url", base_url, "--kind", "local",
                             "--default-model", "test-model"],
                            ["ai", "providers", "set-default", "fake"]):
                subprocess.run([*cli, *command], env=env, cwd=ROOT, check=True, capture_output=True, timeout=300)
            result = subprocess.run(
                [*cli, "chat", "--no-tools", "--file", str(export), "--prompt", "What did I attach?"],
                env=env, cwd=ROOT, capture_output=True, text=True, timeout=300, stdin=subprocess.DEVNULL,
            )
    finally:
        server.shutdown()
        server.server_close()
    sent = b"".join(body for path, body in server.posts if path == "/v1/responses")
    attached = result.returncode == 0 and b"file-1" in sent
    print(f"exit {result.returncode}; provider saw file-1: {b'file-1' in sent}")
    if not attached:
        print(result.stdout[-2000:] or result.stderr[-2000:])
    return 0 if attached else 1


if __name__ == "__main__":
    raise SystemExit(main())
