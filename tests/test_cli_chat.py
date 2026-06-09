import json
import subprocess
import sys
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


def _chat_completion_response(message, finish_reason="stop"):
    return {
        "choices": [
            {
                "message": message,
                "finish_reason": finish_reason,
            }
        ]
    }


def _tool_call_message(name, arguments="{}", call_id="call_1"):
    return _chat_completion_response(
        {
            "role": "assistant",
            "content": "",
            "tool_calls": [
                {
                    "id": call_id,
                    "type": "function",
                    "function": {"name": name, "arguments": arguments},
                }
            ],
        },
        finish_reason="tool_calls",
    )


class _ToolChatHandler(BaseHTTPRequestHandler):
    def do_POST(self):
        if self.path != "/v1/chat/completions":
            self.send_response(404)
            self.end_headers()
            return
        length = int(self.headers.get("content-length") or "0")
        body = json.loads(self.rfile.read(length).decode("utf-8")) if length else {}
        self.server.requests.append(body)  # type: ignore[attr-defined]
        try:
            payload = self.server.responses.pop(0)  # type: ignore[attr-defined]
        except IndexError:
            payload = _chat_completion_response({"role": "assistant", "content": "done"})
        if body.get("stream"):
            self.send_response(200)
            self.send_header("content-type", "text/event-stream")
            self.send_header("cache-control", "no-cache")
            self.end_headers()
            choice = payload["choices"][0]
            message = choice["message"]
            delta = {}
            if "tool_calls" in message:
                delta["tool_calls"] = message["tool_calls"]
            if message.get("content"):
                delta["content"] = message["content"]
            chunk = {
                "choices": [
                    {
                        "delta": delta,
                        "finish_reason": choice.get("finish_reason"),
                    }
                ]
            }
            self.wfile.write(f"data: {json.dumps(chunk)}\n\n".encode())
            self.wfile.write(b"data: [DONE]\n\n")
            self.wfile.flush()
            return
        raw = json.dumps(payload).encode("utf-8")
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def log_message(self, *args):
        return


def _start_tool_chat_server(responses):
    server = ThreadingHTTPServer(("127.0.0.1", 0), _ToolChatHandler)
    server.responses = list(responses)  # type: ignore[attr-defined]
    server.requests = []  # type: ignore[attr-defined]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server


def _stop_server(server):
    server.shutdown()
    server.server_close()


def _run(data_root, *args):
    # stdin must not inherit the test runner's terminal: the non-TTY consent
    # policy is part of what these tests pin down.
    return subprocess.run(
        [
            sys.executable,
            "-m",
            "kassiber",
            "--data-root",
            str(data_root),
            *args,
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
        timeout=15,
        stdin=subprocess.DEVNULL,
    )


def _run_json(data_root, *args):
    result = _run(data_root, "--machine", *args)
    if result.returncode != 0:
        raise AssertionError(
            f"command failed: {args}; stdout={result.stdout}; stderr={result.stderr}"
        )
    return json.loads(result.stdout)


def _seed_provider(data_root, base_url):
    _run_json(data_root, "init")
    _run_json(data_root, "workspaces", "create", "Demo")
    _run_json(data_root, "profiles", "create", "Main")
    _run_json(
        data_root,
        "ai",
        "providers",
        "create",
        "tool-local",
        "--base-url",
        base_url,
        "--kind",
        "local",
        "--default-model",
        "test-model",
    )
    _run_json(data_root, "ai", "providers", "set-default", "tool-local")


class CliChatTest(unittest.TestCase):
    def test_chat_runs_daemon_tool_loop(self):
        server = _start_tool_chat_server(
            [
                _tool_call_message("status"),
                _chat_completion_response(
                    {"role": "assistant", "content": "Kassiber is ready."},
                ),
            ]
        )
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-chat-") as tmp:
                data_root = Path(tmp) / "data"
                _seed_provider(data_root, f"http://127.0.0.1:{server.server_port}/v1")
                payload = _run_json(
                    data_root,
                    "chat",
                    "--provider",
                    "tool-local",
                    "Check local status",
                )
        finally:
            _stop_server(server)

        self.assertEqual(payload["kind"], "chat")
        self.assertEqual(payload["data"]["message"]["content"], "Kassiber is ready.")
        self.assertEqual(payload["data"]["tool_calls"][0]["name"], "status")
        self.assertEqual(payload["data"]["tool_calls"][0]["status"], "done")
        self.assertEqual(len(server.requests), 2)  # type: ignore[attr-defined]
        self.assertTrue(
            any(
                message.get("role") == "tool"
                for message in server.requests[1]["messages"]  # type: ignore[attr-defined]
            )
        )

    def test_chat_yes_approves_mutating_consent(self):
        server = _start_tool_chat_server(
            [
                _tool_call_message("ui_journals_process"),
                _chat_completion_response(
                    {"role": "assistant", "content": "Journals processed."},
                ),
            ]
        )
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-chat-") as tmp:
                data_root = Path(tmp) / "data"
                _seed_provider(data_root, f"http://127.0.0.1:{server.server_port}/v1")
                payload = _run_json(
                    data_root,
                    "chat",
                    "--provider",
                    "tool-local",
                    "--yes",
                    "Process journals",
                )
        finally:
            _stop_server(server)

        tool = next(
            item
            for item in payload["data"]["tool_calls"]
            if item["name"] == "ui.journals.process"
        )
        self.assertEqual(tool["status"], "done")
        self.assertNotEqual(tool.get("reason"), "user_denied")
        self.assertTrue(
            any(
                message.get("role") == "tool"
                and "user_denied" not in message.get("content", "")
                for message in server.requests[1]["messages"]  # type: ignore[attr-defined]
            )
        )

    def test_chat_allow_tool_approves_only_listed_mutating_tool(self):
        server = _start_tool_chat_server(
            [
                _tool_call_message("ui_journals_process"),
                _chat_completion_response(
                    {"role": "assistant", "content": "Journals processed."},
                ),
            ]
        )
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-chat-") as tmp:
                data_root = Path(tmp) / "data"
                _seed_provider(data_root, f"http://127.0.0.1:{server.server_port}/v1")
                # Wire-name form; pins the catalog mapping back to the dotted
                # display name used in consent prompts.
                payload = _run_json(
                    data_root,
                    "chat",
                    "--provider",
                    "tool-local",
                    "--allow-tool",
                    "ui_journals_process",
                    "Process journals",
                )
        finally:
            _stop_server(server)

        tool = next(
            item
            for item in payload["data"]["tool_calls"]
            if item["name"] == "ui.journals.process"
        )
        self.assertEqual(tool["status"], "done")
        self.assertTrue(
            any(
                message.get("role") == "tool"
                and "user_denied" not in message.get("content", "")
                for message in server.requests[1]["messages"]  # type: ignore[attr-defined]
            )
        )

    def test_chat_stream_json_emits_daemon_records(self):
        server = _start_tool_chat_server(
            [
                _tool_call_message("status"),
                _chat_completion_response(
                    {"role": "assistant", "content": "Kassiber is ready."},
                ),
            ]
        )
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-chat-") as tmp:
                data_root = Path(tmp) / "data"
                _seed_provider(data_root, f"http://127.0.0.1:{server.server_port}/v1")
                result = _run(
                    data_root,
                    "chat",
                    "--stream-json",
                    "--provider",
                    "tool-local",
                    "Check local status",
                )
        finally:
            _stop_server(server)

        self.assertEqual(result.returncode, 0, result.stderr)
        records = [json.loads(line) for line in result.stdout.splitlines() if line]
        kinds = [record["kind"] for record in records]
        self.assertIn("ai.chat.tool_call", kinds)
        self.assertIn("ai.chat.tool_result", kinds)
        self.assertIn("ai.chat.delta", kinds)
        self.assertEqual(kinds[-1], "ai.chat")
        self.assertEqual(records[-1]["data"]["finish_reason"], "stop")

    def test_chat_stream_json_denies_mutating_tools(self):
        server = _start_tool_chat_server(
            [
                _tool_call_message("ui_journals_process"),
                _chat_completion_response(
                    {"role": "assistant", "content": "I did not process journals."},
                ),
            ]
        )
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-chat-") as tmp:
                data_root = Path(tmp) / "data"
                _seed_provider(data_root, f"http://127.0.0.1:{server.server_port}/v1")
                result = _run(
                    data_root,
                    "chat",
                    "--stream-json",
                    "--provider",
                    "tool-local",
                    "Process journals",
                )
        finally:
            _stop_server(server)

        self.assertEqual(result.returncode, 0, result.stderr)
        records = [json.loads(line) for line in result.stdout.splitlines() if line]
        mutating_call = next(
            r
            for r in records
            if r["kind"] == "ai.chat.tool_call"
            and r["data"]["kind_class"] == "mutating"
        )
        denied = next(
            r
            for r in records
            if r["kind"] == "ai.chat.tool_result"
            and r["data"]["call_id"] == mutating_call["data"]["call_id"]
        )
        self.assertFalse(denied["data"]["ok"])
        self.assertEqual(denied["data"]["reason"], "user_denied")

    def test_chat_stream_json_rejects_machine_format(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-chat-") as tmp:
            data_root = Path(tmp) / "data"
            result = _run(data_root, "--machine", "chat", "--stream-json", "hello")
        self.assertNotEqual(result.returncode, 0)
        payload = json.loads(result.stdout)
        self.assertEqual(payload["error"]["code"], "validation")

    def test_chat_system_flag_replaces_kassiber_prompt(self):
        server = _start_tool_chat_server(
            [
                _chat_completion_response(
                    {"role": "assistant", "content": "ok"},
                ),
            ]
        )
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-chat-") as tmp:
                data_root = Path(tmp) / "data"
                _seed_provider(data_root, f"http://127.0.0.1:{server.server_port}/v1")
                payload = _run_json(
                    data_root,
                    "chat",
                    "--provider",
                    "tool-local",
                    "--system",
                    "You are terse.",
                    "hello",
                )
        finally:
            _stop_server(server)

        self.assertEqual(payload["data"]["message"]["content"], "ok")
        first = server.requests[0]["messages"][0]  # type: ignore[attr-defined]
        self.assertEqual(first["role"], "system")
        self.assertEqual(first["content"], "You are terse.")

    def test_chat_non_tty_denies_mutating_consent_without_allow_policy(self):
        server = _start_tool_chat_server(
            [
                _tool_call_message("ui_journals_process"),
                _chat_completion_response(
                    {"role": "assistant", "content": "I did not process journals."},
                ),
            ]
        )
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-chat-") as tmp:
                data_root = Path(tmp) / "data"
                _seed_provider(data_root, f"http://127.0.0.1:{server.server_port}/v1")
                payload = _run_json(
                    data_root,
                    "chat",
                    "--provider",
                    "tool-local",
                    "Process journals",
                )
        finally:
            _stop_server(server)

        tool = next(
            item
            for item in payload["data"]["tool_calls"]
            if item["name"] == "ui.journals.process"
        )
        self.assertEqual(tool["status"], "denied")
        self.assertEqual(tool["reason"], "user_denied")
        self.assertTrue(
            any(
                message.get("role") == "tool"
                and "user_denied" in message.get("content", "")
                for message in server.requests[1]["messages"]  # type: ignore[attr-defined]
            )
        )


if __name__ == "__main__":
    unittest.main()
