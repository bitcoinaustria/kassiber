import argparse
import io
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


def _run(data_root, *args, input_text=None):
    # stdin must not inherit the test runner's terminal: the non-TTY consent
    # policy is part of what these tests pin down.
    stdin_kwargs = (
        {"stdin": subprocess.DEVNULL} if input_text is None else {"input": input_text}
    )
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
        **stdin_kwargs,
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

    def test_chat_rendered_pipe_keeps_stdout_clean(self):
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
                    "--provider",
                    "tool-local",
                    "Check local status",
                )
        finally:
            _stop_server(server)

        self.assertEqual(result.returncode, 0, result.stderr)
        # Piped stdout carries only the answer; progress, tool announcements,
        # and the provenance footer go to stderr.
        self.assertEqual(result.stdout, "Kassiber is ready.\n")
        self.assertIn("Tool: status", result.stderr)
        self.assertIn("tools: status", result.stderr)

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

    def test_chat_no_tools_sends_no_system_prompt_or_tools(self):
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
                    "--no-tools",
                    "hello",
                )
        finally:
            _stop_server(server)

        self.assertEqual(payload["data"]["message"]["content"], "ok")
        request = server.requests[0]  # type: ignore[attr-defined]
        self.assertNotIn("tools", request)
        self.assertTrue(
            all(message["role"] != "system" for message in request["messages"])
        )

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

    def test_chat_dash_reads_prompt_from_stdin(self):
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
                result = _run(
                    data_root,
                    "--machine",
                    "chat",
                    "-",
                    "--provider",
                    "tool-local",
                    input_text="What is my status?\n",
                )
        finally:
            _stop_server(server)

        self.assertEqual(result.returncode, 0, result.stderr)
        payload = json.loads(result.stdout)
        self.assertEqual(payload["data"]["message"]["content"], "ok")
        last = server.requests[0]["messages"][-1]  # type: ignore[attr-defined]
        self.assertEqual(last["role"], "user")
        self.assertEqual(last["content"], "What is my status?")

    def test_chat_transcript_records_full_session(self):
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
                transcript = Path(tmp) / "chat.ndjson"
                _seed_provider(data_root, f"http://127.0.0.1:{server.server_port}/v1")
                result = _run(
                    data_root,
                    "--machine",
                    "chat",
                    "--transcript",
                    str(transcript),
                    "--provider",
                    "tool-local",
                    "hello",
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                records = [
                    json.loads(line)
                    for line in transcript.read_text().splitlines()
                    if line
                ]
        finally:
            _stop_server(server)

        kinds = [record.get("kind") for record in records]
        self.assertIn("daemon.ready", kinds)
        # Outbound ai.chat request (has args) and the terminal record (has data).
        self.assertTrue(
            any(r.get("kind") == "ai.chat" and "args" in r for r in records)
        )
        self.assertTrue(
            any(
                r.get("kind") == "ai.chat"
                and isinstance(r.get("data"), dict)
                and r["data"].get("finish_reason") == "stop"
                for r in records
            )
        )

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


class CliChatPersistenceTest(unittest.TestCase):
    def _chat(self, data_root, *extra, prompt="hello"):
        return _run_json(data_root, "chat", "--provider", "tool-local", *extra, prompt)

    def test_chat_persists_when_history_on(self):
        server = _start_tool_chat_server(
            [_chat_completion_response({"role": "assistant", "content": "stored answer"})]
        )
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-chat-") as tmp:
                data_root = Path(tmp) / "data"
                _seed_provider(data_root, f"http://127.0.0.1:{server.server_port}/v1")
                config = _run_json(data_root, "chats", "config", "--history", "on")
                self.assertEqual(config["data"]["history"], "on")
                self.assertTrue(config["data"]["history_enabled"])

                payload = self._chat(data_root, prompt="What changed this week?")
                session_id = payload["data"]["session_id"]
                self.assertIsInstance(session_id, str)

                listed = _run_json(data_root, "chats", "list")
                self.assertEqual(listed["kind"], "chats.list")
                sessions = listed["data"]["sessions"]
                self.assertEqual(len(sessions), 1)
                self.assertEqual(sessions[0]["id"], session_id)
                self.assertEqual(sessions[0]["title"], "What changed this week?")
                self.assertEqual(sessions[0]["message_count"], 2)

                shown = _run_json(data_root, "chats", "show", session_id)
                messages = shown["data"]["messages"]
                self.assertEqual(messages[0]["role"], "user")
                self.assertEqual(messages[0]["content"], "What changed this week?")
                self.assertEqual(messages[1]["role"], "assistant")
                self.assertEqual(messages[1]["content"], "stored answer")
                self.assertEqual(messages[1]["finish_reason"], "stop")
                self.assertIsInstance(messages[1]["provenance"], dict)
        finally:
            _stop_server(server)

    def test_chat_auto_policy_skips_plaintext_database(self):
        server = _start_tool_chat_server(
            [_chat_completion_response({"role": "assistant", "content": "ok"})]
        )
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-chat-") as tmp:
                data_root = Path(tmp) / "data"
                _seed_provider(data_root, f"http://127.0.0.1:{server.server_port}/v1")
                payload = self._chat(data_root)
                self.assertIsNone(payload["data"]["session_id"])
                listed = _run_json(data_root, "chats", "list")
                self.assertEqual(listed["data"]["sessions"], [])
                self.assertEqual(listed["data"]["history_mode"], "auto")
        finally:
            _stop_server(server)

    def test_chat_incognito_skips_persistence(self):
        server = _start_tool_chat_server(
            [_chat_completion_response({"role": "assistant", "content": "ok"})]
        )
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-chat-") as tmp:
                data_root = Path(tmp) / "data"
                _seed_provider(data_root, f"http://127.0.0.1:{server.server_port}/v1")
                _run_json(data_root, "chats", "config", "--history", "on")
                payload = self._chat(data_root, "--incognito")
                self.assertIsNone(payload["data"]["session_id"])
                listed = _run_json(data_root, "chats", "list")
                self.assertEqual(listed["data"]["sessions"], [])
        finally:
            _stop_server(server)

    def test_chat_continue_appends_to_existing_session(self):
        server = _start_tool_chat_server(
            [
                _chat_completion_response({"role": "assistant", "content": "answer one"}),
                _chat_completion_response({"role": "assistant", "content": "answer two"}),
            ]
        )
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-chat-") as tmp:
                data_root = Path(tmp) / "data"
                _seed_provider(data_root, f"http://127.0.0.1:{server.server_port}/v1")
                _run_json(data_root, "chats", "config", "--history", "on")
                first = self._chat(data_root, prompt="first question")
                second = self._chat(
                    data_root, "--continue", prompt="second question"
                )
                self.assertEqual(
                    first["data"]["session_id"], second["data"]["session_id"]
                )

                listed = _run_json(data_root, "chats", "list")
                self.assertEqual(len(listed["data"]["sessions"]), 1)
                self.assertEqual(listed["data"]["sessions"][0]["message_count"], 4)

                # The continued turn carried the stored history back to the model.
                contents = [
                    message.get("content")
                    for message in server.requests[1]["messages"]  # type: ignore[attr-defined]
                ]
                self.assertIn("first question", contents)
                self.assertIn("answer one", contents)
                self.assertIn("second question", contents)
        finally:
            _stop_server(server)

    def test_chats_delete_and_clear(self):
        server = _start_tool_chat_server(
            [
                _chat_completion_response({"role": "assistant", "content": "one"}),
                _chat_completion_response({"role": "assistant", "content": "two"}),
            ]
        )
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-chat-") as tmp:
                data_root = Path(tmp) / "data"
                _seed_provider(data_root, f"http://127.0.0.1:{server.server_port}/v1")
                _run_json(data_root, "chats", "config", "--history", "on")
                first = self._chat(data_root, prompt="first chat")
                self._chat(data_root, prompt="second chat")

                listed = _run_json(data_root, "chats", "list")
                self.assertEqual(len(listed["data"]["sessions"]), 2)

                deleted = _run_json(
                    data_root, "chats", "delete", first["data"]["session_id"]
                )
                self.assertEqual(
                    deleted["data"]["deleted"], first["data"]["session_id"]
                )
                listed = _run_json(data_root, "chats", "list")
                self.assertEqual(len(listed["data"]["sessions"]), 1)

                cleared = _run_json(data_root, "chats", "clear")
                self.assertEqual(cleared["data"]["deleted"], 1)
                listed = _run_json(data_root, "chats", "list")
                self.assertEqual(listed["data"]["sessions"], [])
        finally:
            _stop_server(server)

    def test_chat_history_off_blocks_continuation_writes(self):
        server = _start_tool_chat_server(
            [
                _chat_completion_response({"role": "assistant", "content": "answer one"}),
                _chat_completion_response({"role": "assistant", "content": "answer two"}),
            ]
        )
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-chat-") as tmp:
                data_root = Path(tmp) / "data"
                _seed_provider(data_root, f"http://127.0.0.1:{server.server_port}/v1")
                _run_json(data_root, "chats", "config", "--history", "on")
                first = self._chat(data_root, prompt="first question")
                _run_json(data_root, "chats", "config", "--history", "off")

                # Continuation still replays stored context to the model, but
                # the off policy blocks any new write.
                second = self._chat(data_root, "--continue", prompt="second question")
                self.assertIsNone(second["data"]["session_id"])
                shown = _run_json(
                    data_root, "chats", "show", first["data"]["session_id"]
                )
                self.assertEqual(len(shown["data"]["messages"]), 2)
        finally:
            _stop_server(server)

    def test_daemon_request_without_persist_intent_stays_ephemeral(self):
        # Pins the GUI-shaped contract: an ai.chat request carrying neither
        # `persist` nor `session_id` never writes history, even with the
        # policy set to `on`.
        server = _start_tool_chat_server(
            [_chat_completion_response({"role": "assistant", "content": "ok"})]
        )
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-chat-") as tmp:
                data_root = Path(tmp) / "data"
                _seed_provider(data_root, f"http://127.0.0.1:{server.server_port}/v1")
                _run_json(data_root, "chats", "config", "--history", "on")
                daemon = subprocess.Popen(
                    [
                        sys.executable,
                        "-m",
                        "kassiber",
                        "--data-root",
                        str(data_root),
                        "daemon",
                    ],
                    cwd=ROOT,
                    stdin=subprocess.PIPE,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    text=True,
                    bufsize=1,
                )
                try:
                    self.assertEqual(
                        json.loads(daemon.stdout.readline())["kind"], "daemon.ready"
                    )
                    daemon.stdin.write(
                        json.dumps(
                            {
                                "kind": "ai.chat",
                                "request_id": "gui-1",
                                "args": {
                                    "model": "test-model",
                                    "messages": [
                                        {"role": "user", "content": "hello"}
                                    ],
                                },
                            }
                        )
                        + "\n"
                    )
                    daemon.stdin.flush()
                    while True:
                        record = json.loads(daemon.stdout.readline())
                        if record.get("kind") == "ai.chat":
                            self.assertIsNone(record["data"]["session_id"])
                            break
                        self.assertNotEqual(record.get("kind"), "error", record)
                finally:
                    daemon.stdin.close()
                    daemon.wait(timeout=10)
                    daemon.stdout.close()
                    daemon.stderr.close()
                listed = _run_json(data_root, "chats", "list")
                self.assertEqual(listed["data"]["sessions"], [])
        finally:
            _stop_server(server)

    def test_chats_config_round_trip(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-chat-") as tmp:
            data_root = Path(tmp) / "data"
            _run_json(data_root, "init")
            _run_json(data_root, "workspaces", "create", "Demo")
            _run_json(data_root, "profiles", "create", "Main")
            shown = _run_json(data_root, "chats", "config")
            self.assertEqual(shown["data"]["history"], "auto")
            self.assertFalse(shown["data"]["history_enabled"])
            self.assertFalse(shown["data"]["database_encrypted"])
            updated = _run_json(data_root, "chats", "config", "--history", "on")
            self.assertEqual(updated["data"]["history"], "on")
            self.assertTrue(updated["data"]["history_enabled"])


class _FakeTtyInput(io.StringIO):
    def isatty(self):
        return True


class _FakeTtyOutput(io.StringIO):
    def isatty(self):
        return True


def _chat_namespace(data_root, **overrides):
    values = dict(
        data_root=str(data_root),
        env_file=None,
        db_passphrase_fd=None,
        format=None,
        prompt=None,
        prompt_text=None,
        provider="tool-local",
        model=None,
        system=None,
        temperature=None,
        max_tokens=None,
        reasoning_effort="auto",
        tool_loop_max_iterations=8,
        no_tools=False,
        yes=False,
        allow_tool=None,
        stream_json=False,
        transcript=None,
        incognito=False,
        continue_session=False,
        session=None,
        plain=False,
    )
    values.update(overrides)
    return argparse.Namespace(**values)


class CliChatRenderTest(unittest.TestCase):
    def _run_tty_chat(self, content, **overrides):
        from kassiber.cli.chat import run_chat_command

        server = _start_tool_chat_server(
            [_chat_completion_response({"role": "assistant", "content": content})]
        )
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-chat-") as tmp:
                data_root = Path(tmp) / "data"
                _seed_provider(data_root, f"http://127.0.0.1:{server.server_port}/v1")
                args = _chat_namespace(data_root, prompt="hello", **overrides)
                stdout = _FakeTtyOutput()
                session = run_chat_command(
                    args, stdin=io.StringIO(""), stdout=stdout
                )
        finally:
            _stop_server(server)
        return session, stdout.getvalue()

    def test_tty_output_renders_markdown(self):
        session, output = self._run_tty_chat("**Ready** to `help`.")
        self.assertIn("\x1b[1mReady\x1b[22m", output)
        self.assertIn("\x1b[36mhelp\x1b[39m", output)
        # The conversation history keeps the raw markdown, not ANSI.
        self.assertEqual(session.turns[0].content, "**Ready** to `help`.")

    def test_plain_disables_markdown_rendering(self):
        _, output = self._run_tty_chat("**Ready** to help.", plain=True)
        self.assertIn("**Ready**", output)
        self.assertNotIn("\x1b[1m", output)


class CliChatReplTest(unittest.TestCase):
    def test_repl_commands_and_turn(self):
        from kassiber.cli.chat import run_chat_command

        server = _start_tool_chat_server(
            [
                _chat_completion_response(
                    {"role": "assistant", "content": "Kassiber is ready."},
                ),
            ]
        )
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-chat-") as tmp:
                data_root = Path(tmp) / "data"
                _seed_provider(data_root, f"http://127.0.0.1:{server.server_port}/v1")
                args = argparse.Namespace(
                    data_root=str(data_root),
                    env_file=None,
                    db_passphrase_fd=None,
                    format=None,
                    prompt=None,
                    prompt_text=None,
                    provider="tool-local",
                    model=None,
                    system=None,
                    temperature=None,
                    max_tokens=None,
                    reasoning_effort="auto",
                    tool_loop_max_iterations=8,
                    no_tools=False,
                    yes=False,
                    allow_tool=None,
                    stream_json=False,
                    transcript=None,
                )
                stdin = _FakeTtyInput(
                    "/help\n"
                    "/model\n"
                    "/allow ui_journals_process\n"
                    "/allowed\n"
                    "/bogus\n"
                    "hello\n"
                    "/exit\n"
                )
                stdout = io.StringIO()
                session = run_chat_command(args, stdin=stdin, stdout=stdout)
        finally:
            _stop_server(server)

        output = stdout.getvalue()
        self.assertIn("/provider [name]", output)
        self.assertIn("model: test-model", output)
        self.assertIn("Allowed for this session: ui.journals.process", output)
        self.assertIn("ui.journals.process  (this session)", output)
        self.assertIn("Unknown command /bogus", output)
        self.assertIn("Kassiber is ready.", output)
        self.assertEqual(len(session.turns), 1)


if __name__ == "__main__":
    unittest.main()
