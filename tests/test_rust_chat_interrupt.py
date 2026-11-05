"""Cooperative Ctrl-C in the Rust `kassiber chat` (R6-8, closing R6-7's gap).

`kassiber chat` runs on a pseudo-terminal that is its controlling terminal,
so a typed Ctrl-C reaches it the way a user's does: as SIGINT to the
terminal's foreground process group. A loopback provider holds its answer
open after the first delta, so the Ctrl-C lands mid-turn:

- during a turn it sends `ai.chat.cancel`, waits for the daemon's terminal
  record and keeps the session (Python's `KeyboardInterrupt` branch of
  `_stream_turn_records`); the daemon child, in its own process group,
  never sees the Ctrl-C;
- at the prompt it ends the REPL normally;
- a one-shot `--machine` chat returns the cancelled turn as its payload.

Python's own CLI cannot pass the first case: its daemon child shares the
terminal's process group and dies
(tests/rust_port/repro_chat_ctrl_c_kills_daemon.py), so this runs against
Rust alone.
"""

from __future__ import annotations

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import pty
import select
import subprocess
import sys
import tempfile
import threading
import time
import unittest

from tests.test_rust_shell_parity import _environment

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
WAIT = 60.0


class _SlowHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_GET(self) -> None:  # noqa: N802 - http.server API
        body = json.dumps({"object": "list", "data": [{"id": "test-model", "object": "model"}]}).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self) -> None:  # noqa: N802 - http.server API
        server: SlowProvider = self.server  # type: ignore[assignment]
        length = int(self.headers.get("content-length") or "0")
        self.rfile.read(length)
        with server.lock:
            server.posts.append(self.path)
        if self.path != "/v1/responses":
            self.send_response(404)
            self.send_header("content-length", "0")
            self.end_headers()
            return
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("connection", "close")
        self.end_headers()
        first = {"type": "response.output_text.delta", "output_index": 0, "delta": "Thinking"}
        try:
            self.wfile.write(f"data: {json.dumps(first)}\n\n".encode())
            self.wfile.flush()
            server.streaming.set()
            # Hold the answer open until the test lets it finish.
            server.release.wait(WAIT)
            done = {
                "type": "response.completed",
                "response": {
                    "id": "resp_1",
                    "status": "completed",
                    "output": [
                        {
                            "type": "message",
                            "role": "assistant",
                            "status": "completed",
                            "content": [{"type": "output_text", "text": "Thinking done"}],
                        }
                    ],
                },
            }
            self.wfile.write(f"data: {json.dumps(done)}\n\ndata: [DONE]\n\n".encode())
            self.wfile.flush()
        except OSError:
            pass
        self.close_connection = True

    def log_message(self, *args) -> None:
        return


class SlowProvider(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self) -> None:
        super().__init__(("127.0.0.1", 0), _SlowHandler)
        self.lock = threading.Lock()
        self.posts: list[str] = []
        self.streaming = threading.Event()
        self.release = threading.Event()
        self._thread = threading.Thread(target=self.serve_forever, kwargs={"poll_interval": 0.05}, daemon=True)
        self._thread.start()

    def stop(self) -> None:
        self.release.set()
        self.shutdown()
        self.server_close()


class Terminal:
    """`kassiber chat` with a pseudo-terminal as its controlling terminal."""

    def __init__(self, argv, env, cwd):
        self.master, slave = pty.openpty()

        def controlling_terminal():
            # The new session's leader takes the terminal, which makes its
            # process group the terminal's foreground group.
            import fcntl
            import termios

            fcntl.ioctl(0, termios.TIOCSCTTY, 0)

        self.process = subprocess.Popen(
            argv,
            stdin=slave,
            stdout=slave,
            stderr=slave,
            env=env,
            cwd=cwd,
            start_new_session=True,
            preexec_fn=controlling_terminal,
        )
        os.close(slave)
        self.output = b""

    def read_until(self, marker: bytes, timeout: float = WAIT) -> bytes:
        deadline = time.monotonic() + timeout
        while marker not in self.output:
            left = deadline - time.monotonic()
            if left <= 0:
                raise AssertionError(f"no {marker!r} in {self.output[-2000:]!r}")
            ready, _, _ = select.select([self.master], [], [], min(left, 0.2))
            if ready:
                try:
                    chunk = os.read(self.master, 65536)
                except OSError:
                    chunk = b""
                if not chunk:
                    if self.process.poll() is not None:
                        raise AssertionError(f"chat exited before {marker!r}: {self.output[-2000:]!r}")
                    continue
                self.output += chunk
        return self.output

    def type(self, text: bytes) -> None:
        os.write(self.master, text)

    def drain(self) -> None:
        while True:
            ready, _, _ = select.select([self.master], [], [], 0.2)
            if not ready:
                return
            try:
                chunk = os.read(self.master, 65536)
            except OSError:
                return
            if not chunk:
                return
            self.output += chunk

    def wait(self, timeout: float = WAIT) -> int:
        try:
            return self.process.wait(timeout=timeout)
        finally:
            self.drain()

    def close(self) -> None:
        if self.process.poll() is None:
            self.process.kill()
            self.process.wait(timeout=10)
        os.close(self.master)


def _children(pid: int) -> list[int]:
    children = []
    for task in Path(f"/proc/{pid}/task").iterdir():
        text = (task / "children").read_text().split()
        children += [int(child) for child in text]
    return children


@unittest.skipUnless(
    RUST_BIN and sys.platform.startswith("linux"),
    "set KASSIBER_RUST_BIN to a test-harness build (Linux /proc and ptys)",
)
class RustChatInterruptTest(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="kassiber-chat-ctrl-c-")
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name).resolve()
        self.root = self.base / "data"
        self.env = _environment(self.base / "preferences")
        self.env.update(
            {
                "KASSIBER_STATE_ROOT": str(self.base / "state"),
                "KASSIBER_RUST_TEST_OWNER_ROOT": str(self.base / "owners"),
                "KASSIBER_OPERATOR_RUNTIME_DIR": str(self.base / "run"),
                "KASSIBER_OPERATOR_ALLOW_TEST_RUNTIME_DIR": "1",
                # The loopback provider is the only peer.
                "KASSIBER_TEST_NO_EGRESS": "1",
                "TERM": "dumb",
            }
        )
        (self.base / "owners").mkdir()
        self.provider = SlowProvider()
        self.addCleanup(self.provider.stop)
        base_url = f"http://127.0.0.1:{self.provider.server_port}/v1"
        for args in (
            ["init"],
            ["workspaces", "create", "Demo"],
            ["profiles", "create", "Main"],
            ["ai", "providers", "create", "slow", "--base-url", base_url, "--kind", "local", "--default-model", "test-model"],
            ["ai", "providers", "set-default", "slow"],
        ):
            result = subprocess.run(
                [RUST_BIN, "--machine", "--data-root", str(self.root), *args],
                capture_output=True,
                env=self.env,
                cwd=self.base,
                timeout=120,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def terminal(self, *args, machine=False) -> Terminal:
        flags = ["--machine"] if machine else []
        terminal = Terminal(
            [RUST_BIN, *flags, "--data-root", str(self.root), "chat", "--no-tools", *args],
            self.env,
            self.base,
        )
        self.addCleanup(terminal.close)
        return terminal

    def test_ctrl_c_cancels_the_turn_and_a_second_at_the_prompt_exits(self):
        chat = self.terminal()
        chat.read_until(b"> ")
        chat.type(b"hello\r")
        self.assertTrue(self.provider.streaming.wait(WAIT), chat.output[-2000:])
        chat.read_until(b"Thinking")
        daemons = _children(chat.process.pid)
        self.assertEqual(len(daemons), 1, daemons)
        daemon = daemons[0]
        # The daemon child leads its own process group: the terminal's
        # Ctrl-C reaches only the chat.
        self.assertEqual(os.getpgid(daemon), daemon)
        self.assertNotEqual(os.getpgid(daemon), os.getpgid(chat.process.pid))
        before = len(chat.output)
        chat.type(b"\x03")
        chat.read_until(b"Cancelled.")
        after = chat.read_until(b"> ")[before:]
        self.assertIn(b"Cancelled.", after)
        # The session and its daemon survive the cancelled turn.
        self.assertIsNone(chat.process.poll())
        self.assertEqual(_children(chat.process.pid), [daemon])
        self.assertEqual(self.provider.posts, ["/v1/responses"])
        # A second Ctrl-C at the prompt ends the REPL normally.
        chat.type(b"\x03")
        self.assertEqual(chat.wait(), 0, chat.output[-2000:])
        with self.assertRaises(ProcessLookupError):
            os.kill(daemon, 0)

    def test_the_session_goes_on_after_a_cancelled_turn(self):
        chat = self.terminal()
        chat.read_until(b"> ")
        chat.type(b"first\r")
        self.assertTrue(self.provider.streaming.wait(WAIT))
        chat.read_until(b"Thinking")
        chat.type(b"\x03")
        chat.read_until(b"Cancelled.")
        chat.read_until(b"> ")
        # The next turn runs to its end.
        self.provider.release.set()
        start = len(chat.output)
        chat.type(b"second\r")
        deadline = time.monotonic() + WAIT
        while chat.output[start:].count(b"> ") < 1 or b"slow/test-model" not in chat.output[start:]:
            self.assertLess(time.monotonic(), deadline, chat.output[-2000:])
            chat.read_until(b"> " , timeout=WAIT)
            chat.drain()
        self.assertNotIn(b"Cancelled.", chat.output[start:])
        chat.type(b"/exit\r")
        self.assertEqual(chat.wait(), 0, chat.output[-2000:])
        self.assertEqual(self.provider.posts, ["/v1/responses", "/v1/responses"])

    def test_a_one_shot_chat_returns_the_cancelled_turn(self):
        chat = self.terminal("hello", machine=True)
        self.assertTrue(self.provider.streaming.wait(WAIT), chat.output[-2000:])
        time.sleep(0.2)
        chat.type(b"\x03")
        self.assertEqual(chat.wait(), 0, chat.output[-2000:])
        text = chat.output.decode()
        payload = json.loads(text[text.index("{"):])
        self.assertEqual(payload["kind"], "chat")
        self.assertEqual(payload["data"]["finish_reason"], "cancelled")


if __name__ == "__main__":
    unittest.main()
