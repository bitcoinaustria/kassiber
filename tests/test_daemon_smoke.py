import json
import queue
import select
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from unittest import mock

from kassiber.daemon import (
    AiToolConsentState,
    AiToolRuntime,
    MAX_REQUEST_LINE_CHARS,
    ParsedAiToolCall,
    _execute_mutating_ai_tool,
)


ROOT = Path(__file__).resolve().parent.parent


def _start_daemon(data_root):
    return subprocess.Popen(
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
    )


def _read_payload(proc):
    assert proc.stdout is not None
    return json.loads(proc.stdout.readline())


def _read_payload_timeout(proc, timeout=5.0):
    assert proc.stdout is not None
    ready, _, _ = select.select([proc.stdout.fileno()], [], [], timeout)
    if not ready:
        raise AssertionError(f"daemon did not emit a payload within {timeout:.1f}s")
    return json.loads(proc.stdout.readline())


def _write_payload(proc, payload):
    assert proc.stdin is not None
    line = payload if isinstance(payload, str) else json.dumps(payload)
    proc.stdin.write(line + "\n")
    proc.stdin.flush()


def _run_cli(data_root, *args):
    result = subprocess.run(
        [
            sys.executable,
            "-m",
            "kassiber",
            "--data-root",
            str(data_root),
            "--machine",
            *args,
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    stdout = result.stdout.strip()
    if not stdout:
        raise AssertionError(
            f"CLI produced no stdout for {args}; stderr: {result.stderr}"
        )
    payload = json.loads(stdout)
    if result.returncode != 0 or payload.get("kind") == "error":
        raise AssertionError(
            f"CLI failed for {args}; code={result.returncode}; payload={payload}; stderr={result.stderr}"
        )
    return payload


def _close_daemon(proc):
    if proc.stdin is not None and not proc.stdin.closed:
        proc.stdin.close()
    stderr = proc.stderr.read() if proc.stderr is not None else ""
    if proc.stdout is not None:
        proc.stdout.close()
    if proc.stderr is not None:
        proc.stderr.close()
    return proc.wait(timeout=5), stderr


class _SlowChatHandler(BaseHTTPRequestHandler):
    request_count = 0
    request_count_lock = threading.Lock()

    def do_POST(self):
        if self.path != "/v1/chat/completions":
            self.send_response(404)
            self.end_headers()
            return
        with self.request_count_lock:
            type(self).request_count += 1
        length = int(self.headers.get("content-length") or "0")
        if length:
            self.rfile.read(length)
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("cache-control", "no-cache")
        self.end_headers()
        for chunk in ("one", "two", "three"):
            payload = {
                "choices": [
                    {
                        "delta": {"content": chunk},
                        "finish_reason": None,
                    }
                ]
            }
            try:
                self.wfile.write(f"data: {json.dumps(payload)}\n\n".encode())
                self.wfile.flush()
            except (BrokenPipeError, ConnectionResetError):
                return
            time.sleep(0.15)
        try:
            self.wfile.write(b"data: [DONE]\n\n")
            self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError):
            return

    def log_message(self, *args):
        return


def _chat_completion_response(message, finish_reason="stop"):
    return {
        "choices": [
            {
                "message": message,
                "finish_reason": finish_reason,
            }
        ]
    }


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
            payload, delay = self.server.responses.pop(0)  # type: ignore[attr-defined]
        except IndexError:
            payload, delay = _chat_completion_response(
                {"role": "assistant", "content": "done"},
            ), 0.0
        if delay:
            time.sleep(delay)
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


def _seed_workspace_with_transaction(data_root, tmp_root):
    csv_path = Path(tmp_root) / "transactions.csv"
    csv_path.write_text(
        "\n".join(
            [
                "date,txid,direction,asset,amount,fee,fiat_rate,description",
                "2026-01-01T10:00:00Z,seed-inbound-1,inbound,BTC,0.10000000,0,50000,Seed acquisition",
                "",
            ]
        ),
        encoding="utf-8",
    )
    _run_cli(data_root, "init")
    _run_cli(data_root, "workspaces", "create", "Demo")
    _run_cli(data_root, "profiles", "create", "Main", "--fiat-currency", "EUR")
    _run_cli(
        data_root,
        "wallets",
        "create",
        "--label",
        "Cold",
        "--kind",
        "address",
        "--address",
        "bc1qtestaddress0000000000000000000000000000000",
    )
    _run_cli(data_root, "wallets", "import-csv", "--wallet", "Cold", "--file", str(csv_path))
    _run_cli(data_root, "rates", "set", "BTC-EUR", "2026-01-01T00:00:00Z", "50000")


def _seed_sensitive_ai_surface(data_root):
    _run_cli(data_root, "init")
    _run_cli(data_root, "workspaces", "create", "Secure")
    _run_cli(data_root, "profiles", "create", "Main", "--fiat-currency", "EUR")
    conn = sqlite3.connect(Path(data_root) / "kassiber.sqlite3")
    try:
        workspace_id = conn.execute(
            "SELECT id FROM workspaces WHERE label = 'Secure'"
        ).fetchone()[0]
        profile_id = conn.execute(
            "SELECT id FROM profiles WHERE label = 'Main'"
        ).fetchone()[0]
        account_id = conn.execute(
            "SELECT id FROM accounts WHERE profile_id = ? AND code = 'treasury'",
            (profile_id,),
        ).fetchone()[0]
        now = "2026-01-01T00:00:00Z"
        conn.executemany(
            """
            INSERT INTO backends(
                name, kind, chain, network, url, auth_header, token,
                batch_size, timeout, tor_proxy, config_json, notes, created_at, updated_at
            )
            VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "private",
                    "esplora",
                    "bitcoin",
                    "mainnet",
                    "https://user:pass@private-node.local/secret-path?token=abc",
                    "Bearer secret-auth-header",
                    "secret-token-value",
                    50,
                    10,
                    "",
                    json.dumps(
                        {
                            "cookiefile": "/Users/dev/.bitcoin/.cookie",
                            "username": "rpcuser",
                            "password": "rpcpass",
                            "walletprefix": "secret-wallet-prefix",
                        },
                        sort_keys=True,
                    ),
                    "secret backend note",
                    now,
                    now,
                ),
                (
                    "unused",
                    "esplora",
                    "bitcoin",
                    "mainnet",
                    "https://unused-node.local/also-secret",
                    "",
                    "",
                    None,
                    None,
                    "",
                    "{}",
                    "",
                    now,
                    now,
                ),
            ],
        )
        conn.execute(
            "INSERT OR REPLACE INTO settings(key, value) VALUES('default_backend', 'private')"
        )
        conn.executemany(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, account_id, label, kind, config_json, created_at
            )
            VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "wallet-file-only",
                    workspace_id,
                    profile_id,
                    account_id,
                    "FileOnly",
                    "address",
                    json.dumps(
                        {
                            "source_file": "/tmp/sensitive-wallet-export.csv",
                            "source_format": "csv",
                        },
                        sort_keys=True,
                    ),
                    now,
                ),
                (
                    "wallet-descriptor",
                    workspace_id,
                    profile_id,
                    account_id,
                    "DescriptorLive",
                    "descriptor",
                    json.dumps(
                        {
                            "backend": "private",
                            "chain": "bitcoin",
                            "descriptor": "wpkh(xpub_descriptor_material/0/*)",
                            "network": "mainnet",
                        },
                        sort_keys=True,
                    ),
                    now,
                ),
                (
                    "wallet-default-address",
                    workspace_id,
                    profile_id,
                    account_id,
                    "DefaultAddress",
                    "address",
                    json.dumps(
                        {
                            "addresses": ["bc1qsensitiveaddressmaterial000000000000000000"],
                            "chain": "bitcoin",
                            "network": "mainnet",
                        },
                        sort_keys=True,
                    ),
                    now,
                ),
            ],
        )
        conn.commit()
    finally:
        conn.close()


class DaemonSmokeTest(unittest.TestCase):
    def test_daemon_ready_status_and_shutdown_jsonl(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-daemon-") as tmp:
            data_root = Path(tmp) / "data"
            proc = _start_daemon(data_root)

            ready = _read_payload(proc)
            self.assertEqual(ready["kind"], "daemon.ready")
            self.assertEqual(ready["schema_version"], 1)
            self.assertIn("status", ready["data"]["supported_kinds"])
            self.assertIn("ui.overview.snapshot", ready["data"]["supported_kinds"])
            self.assertIn("ui.transactions.list", ready["data"]["supported_kinds"])
            self.assertIn("ui.wallets.list", ready["data"]["supported_kinds"])
            self.assertIn("ui.backends.list", ready["data"]["supported_kinds"])
            self.assertIn("ui.reports.capital_gains", ready["data"]["supported_kinds"])
            self.assertIn("ui.journals.snapshot", ready["data"]["supported_kinds"])
            self.assertIn("ui.journals.quarantine", ready["data"]["supported_kinds"])
            self.assertIn("ui.journals.transfers.list", ready["data"]["supported_kinds"])
            self.assertIn("ui.profiles.snapshot", ready["data"]["supported_kinds"])
            self.assertIn("ui.rates.summary", ready["data"]["supported_kinds"])
            self.assertIn("ui.workspace.health", ready["data"]["supported_kinds"])
            self.assertIn("ui.workspace.delete", ready["data"]["supported_kinds"])
            self.assertIn("ui.next_actions", ready["data"]["supported_kinds"])
            self.assertIn("ui.wallets.sync", ready["data"]["supported_kinds"])
            self.assertIn("wallets.reveal_descriptor", ready["data"]["supported_kinds"])
            self.assertIn("backends.reveal_token", ready["data"]["supported_kinds"])
            self.assertIn("ai.test_connection", ready["data"]["supported_kinds"])
            self.assertIn("ai.chat", ready["data"]["supported_kinds"])
            self.assertIn("ai.chat.cancel", ready["data"]["supported_kinds"])
            self.assertIn("ai.tool_call.consent", ready["data"]["supported_kinds"])

            _write_payload(proc, {"request_id": "status-1", "kind": "status"})
            status = _read_payload(proc)
            self.assertEqual(status["request_id"], "status-1")
            self.assertEqual(status["kind"], "status")
            self.assertEqual(status["schema_version"], 1)
            self.assertEqual(status["data"]["auth"]["mode"], "local")
            self.assertEqual(status["data"]["data_root"], str(data_root))

            _write_payload(proc, {"request_id": "shutdown-1", "kind": "daemon.shutdown"})
            shutdown = _read_payload(proc)
            self.assertEqual(shutdown["request_id"], "shutdown-1")
            self.assertEqual(shutdown["kind"], "daemon.shutdown")

            code, stderr = _close_daemon(proc)
            self.assertEqual(code, 0, stderr)
            self.assertEqual(stderr, "")

    def test_ui_workspace_delete_removes_current_workspace_and_keeps_daemon_alive(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-daemon-") as tmp:
            data_root = Path(tmp) / "data"
            _seed_workspace_with_transaction(data_root, tmp)
            proc = _start_daemon(data_root)

            ready = _read_payload(proc)
            self.assertEqual(ready["kind"], "daemon.ready")

            _write_payload(
                proc,
                {
                    "request_id": "delete-workspace-1",
                    "kind": "ui.workspace.delete",
                    "args": {"confirm": "DELETE"},
                },
            )
            deleted = _read_payload(proc)
            self.assertEqual(deleted["request_id"], "delete-workspace-1")
            self.assertEqual(deleted["kind"], "ui.workspace.delete")
            self.assertEqual(deleted["data"]["workspace"]["label"], "Demo")
            self.assertEqual(deleted["data"]["removed"]["profiles"], 1)
            self.assertEqual(deleted["data"]["removed"]["wallets"], 1)
            self.assertEqual(deleted["data"]["removed"]["transactions"], 1)

            _write_payload(proc, {"request_id": "status-after-delete", "kind": "status"})
            status = _read_payload(proc)
            self.assertEqual(status["kind"], "status")
            self.assertEqual(status["data"]["workspaces"], 0)
            self.assertEqual(status["data"]["profiles"], 0)
            self.assertEqual(status["data"]["current_workspace"], "")
            self.assertEqual(status["data"]["current_profile"], "")

            _write_payload(proc, {"request_id": "shutdown-1", "kind": "daemon.shutdown"})
            shutdown = _read_payload(proc)
            self.assertEqual(shutdown["kind"], "daemon.shutdown")

            code, stderr = _close_daemon(proc)
            self.assertEqual(code, 0, stderr)
            self.assertEqual(stderr, "")

    def test_ai_tool_consent_state_times_out(self):
        consent = AiToolConsentState()
        decision = consent.wait(
            call_id="call_1",
            tool_name="ui.wallets.sync",
            cancel_event=threading.Event(),
            timeout=0.01,
        )
        self.assertEqual(decision, "consent_timeout")

    def test_ai_tool_consent_state_rejects_unexpected_call_id(self):
        consent = AiToolConsentState()
        self.assertFalse(consent.record("call_1", "allow_once"))

        consent.expect("call_1")
        self.assertTrue(consent.record("call_1", "allow_once"))
        decision = consent.wait(
            call_id="call_1",
            tool_name="ui.wallets.sync",
            cancel_event=threading.Event(),
            timeout=0.01,
        )
        self.assertEqual(decision, "allow_once")
        self.assertFalse(consent.record("call_1", "deny"))

    def test_mutating_tool_uses_daemon_main_thread_connection(self):
        task_queue = queue.Queue()
        runtime = AiToolRuntime(
            data_root="/not-used",
            runtime_config={},
            main_thread_tasks=task_queue,
        )
        call = ParsedAiToolCall(
            call_id="call_1",
            name="ui.wallets.sync",
            arguments={"all": True},
        )
        results = []

        with (
            mock.patch(
                "kassiber.daemon.open_db",
                side_effect=AssertionError("should use daemon main connection"),
            ),
            mock.patch(
                "kassiber.daemon._wallets_sync_payload",
                return_value={"results": []},
            ) as payload_mock,
        ):
            thread = threading.Thread(
                target=lambda: results.append(_execute_mutating_ai_tool(call, runtime)),
            )
            thread.start()
            task = task_queue.get(timeout=1)
            conn_marker = object()
            task.response.put((True, task.callback(conn_marker)))
            thread.join(timeout=1)

        self.assertFalse(thread.is_alive())
        payload_mock.assert_called_once()
        self.assertIs(payload_mock.call_args.args[0], conn_marker)
        self.assertEqual(results[0]["envelope"]["kind"], "ui.wallets.sync")

    def test_daemon_safe_read_tool_kinds_return_workspace_state(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-daemon-state-") as tmp:
            data_root = Path(tmp) / "data"
            _seed_workspace_with_transaction(data_root, tmp)
            proc = _start_daemon(data_root)
            self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.ready")

            _write_payload(proc, {"request_id": "health-1", "kind": "ui.workspace.health"})
            health = _read_payload_timeout(proc)
            self.assertEqual(health["kind"], "ui.workspace.health")
            self.assertEqual(health["data"]["workspace"]["label"], "Demo")
            self.assertEqual(health["data"]["profile"]["label"], "Main")
            self.assertEqual(health["data"]["counts"]["wallets"], 1)
            self.assertEqual(health["data"]["counts"]["transactions"], 1)
            self.assertEqual(health["data"]["journals"]["status"], "not_processed")
            self.assertFalse(health["data"]["reports"]["ready"])

            _write_payload(proc, {"request_id": "next-1", "kind": "ui.next_actions"})
            next_actions = _read_payload_timeout(proc)
            self.assertEqual(next_actions["kind"], "ui.next_actions")
            self.assertEqual(
                next_actions["data"]["suggestions"][0]["id"],
                "process_journals",
            )

            _write_payload(proc, {"request_id": "wallets-1", "kind": "ui.wallets.list"})
            wallets = _read_payload_timeout(proc)
            self.assertEqual(wallets["kind"], "ui.wallets.list")
            self.assertEqual(wallets["data"]["wallets"][0]["label"], "Cold")
            wallet_payload = json.dumps(wallets["data"])
            self.assertNotIn("descriptor", wallet_payload)
            self.assertNotIn("config_json", wallet_payload)

            _write_payload(proc, {"request_id": "backends-1", "kind": "ui.backends.list"})
            backends = _read_payload_timeout(proc)
            self.assertEqual(backends["kind"], "ui.backends.list")
            self.assertGreaterEqual(backends["data"]["summary"]["count"], 1)
            for backend in backends["data"]["backends"]:
                self.assertNotIn("url", backend)
                self.assertNotIn("token", backend)
                self.assertNotIn("auth_header", backend)
                self.assertNotIn("config_json", backend)
                self.assertNotIn("cookiefile", backend)
                self.assertNotIn("username", backend)
                self.assertNotIn("password", backend)
                self.assertNotIn("notes", backend)
                self.assertNotIn("source", backend)
                self.assertIn("has_url", backend)

            _write_payload(
                proc,
                {
                    "request_id": "tx-filter-1",
                    "kind": "ui.transactions.list",
                    "args": {
                        "limit": 5,
                        "direction": "inbound",
                        "asset": "BTC",
                        "wallet": "Cold",
                        "since": "2026-01-01T00:00:00Z",
                        "sort": "amount",
                        "order": "desc",
                    },
                },
            )
            transactions = _read_payload_timeout(proc)
            self.assertEqual(transactions["kind"], "ui.transactions.list")
            self.assertEqual(len(transactions["data"]["txs"]), 1)
            self.assertEqual(transactions["data"]["filters"]["wallet"], "Cold")

            _write_payload(proc, {"request_id": "rates-1", "kind": "ui.rates.summary"})
            rates = _read_payload_timeout(proc)
            self.assertEqual(rates["kind"], "ui.rates.summary")
            self.assertEqual(rates["data"]["pairs"][0]["pair"], "BTC-EUR")

            _write_payload(
                proc,
                {
                    "request_id": "quarantine-1",
                    "kind": "ui.journals.quarantine",
                    "args": {"limit": 3},
                },
            )
            quarantine = _read_payload_timeout(proc)
            self.assertEqual(quarantine["kind"], "ui.journals.quarantine")
            self.assertEqual(quarantine["data"]["summary"]["count"], 0)
            self.assertEqual(quarantine["data"]["items"], [])

            _write_payload(
                proc,
                {
                    "request_id": "transfers-1",
                    "kind": "ui.journals.transfers.list",
                    "args": {"limit": 3},
                },
            )
            transfers = _read_payload_timeout(proc)
            self.assertEqual(transfers["kind"], "ui.journals.transfers.list")
            self.assertEqual(transfers["data"]["pairs"], [])

            _write_payload(proc, {"request_id": "shutdown-1", "kind": "daemon.shutdown"})
            self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.shutdown")
            code, stderr = _close_daemon(proc)
            self.assertEqual(code, 0, stderr)
            self.assertEqual(stderr, "")

    def test_daemon_ai_read_tools_redact_backend_and_wallet_sensitive_state(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-daemon-sensitive-") as tmp:
            data_root = Path(tmp) / "data"
            _seed_sensitive_ai_surface(data_root)
            proc = _start_daemon(data_root)
            self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.ready")

            _write_payload(proc, {"request_id": "wallets-sensitive", "kind": "ui.wallets.list"})
            wallets = _read_payload_timeout(proc)
            self.assertEqual(wallets["kind"], "ui.wallets.list")
            wallet_payload = json.dumps(wallets["data"], sort_keys=True)
            self.assertNotIn("xpub_descriptor_material", wallet_payload)
            self.assertNotIn("bc1qsensitiveaddressmaterial", wallet_payload)
            self.assertNotIn("/tmp/sensitive-wallet-export.csv", wallet_payload)
            wallets_by_label = {
                wallet["label"]: wallet for wallet in wallets["data"]["wallets"]
            }
            self.assertEqual(wallets_by_label["FileOnly"]["sync_mode"], "file_import")
            self.assertEqual(wallets_by_label["FileOnly"]["backend"]["name"], "")
            self.assertEqual(wallets_by_label["FileOnly"]["backend"]["source"], "none")
            self.assertEqual(
                wallets_by_label["DescriptorLive"]["backend"]["source"],
                "explicit",
            )
            self.assertEqual(
                wallets_by_label["DefaultAddress"]["backend"]["source"],
                "default",
            )

            _write_payload(proc, {"request_id": "backends-sensitive", "kind": "ui.backends.list"})
            backends = _read_payload_timeout(proc)
            self.assertEqual(backends["kind"], "ui.backends.list")
            backend_payload = json.dumps(backends["data"], sort_keys=True)
            self.assertIn('"name": "private"', backend_payload)
            self.assertNotIn("unused", backend_payload)
            for leaked in (
                "private-node.local",
                "secret-path",
                "secret-token-value",
                "secret-auth-header",
                "rpcuser",
                "rpcpass",
                ".cookie",
                "secret-wallet-prefix",
                "secret backend note",
                "unused-node.local",
            ):
                self.assertNotIn(leaked, backend_payload)
            backend = backends["data"]["backends"][0]
            self.assertNotIn("url", backend)
            self.assertTrue(backend["has_url"])
            self.assertTrue(backend["has_token"])
            self.assertTrue(backend["has_auth_header"])
            self.assertTrue(backend["has_cookiefile"])
            self.assertTrue(backend["has_username"])
            self.assertTrue(backend["has_password"])

            _write_payload(proc, {"request_id": "shutdown-1", "kind": "daemon.shutdown"})
            self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.shutdown")
            code, stderr = _close_daemon(proc)
            self.assertEqual(code, 0, stderr)
            self.assertEqual(stderr, "")

    def test_ai_tool_consent_stale_target_returns_not_found(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-daemon-") as tmp:
            proc = _start_daemon(Path(tmp) / "data")
            self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.ready")
            _write_payload(
                proc,
                {
                    "request_id": "consent-stale-1",
                    "kind": "ai.tool_call.consent",
                    "args": {
                        "target_request_id": "missing-chat",
                        "call_id": "call_1",
                        "decision": "allow_once",
                    },
                },
            )
            response = _read_payload_timeout(proc)
            self.assertEqual(response["kind"], "ai.tool_call.consent")
            self.assertFalse(response["data"]["recorded"])
            self.assertEqual(response["data"]["reason"], "not_found")

            _write_payload(proc, {"request_id": "shutdown-1", "kind": "daemon.shutdown"})
            self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.shutdown")
            code, stderr = _close_daemon(proc)
            self.assertEqual(code, 0, stderr)
            self.assertEqual(stderr, "")

    def test_ai_chat_cancel_cooperatively_finishes_cancelled(self):
        server = ThreadingHTTPServer(("127.0.0.1", 0), _SlowChatHandler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        base_url = f"http://127.0.0.1:{server.server_port}/v1"
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-daemon-") as tmp:
                data_root = Path(tmp) / "data"
                proc = _start_daemon(data_root)

                ready = _read_payload_timeout(proc)
                self.assertEqual(ready["kind"], "daemon.ready")
                self.assertIn("ai.chat.cancel", ready["data"]["supported_kinds"])

                _write_payload(
                    proc,
                    {
                        "request_id": "provider-1",
                        "kind": "ai.providers.create",
                        "args": {
                            "name": "slow-local",
                            "base_url": base_url,
                            "kind": "local",
                        },
                    },
                )
                provider = _read_payload_timeout(proc)
                self.assertEqual(provider["kind"], "ai.providers.create")

                _write_payload(
                    proc,
                    {
                        "request_id": "chat-1",
                        "kind": "ai.chat",
                        "args": {
                            "provider": "slow-local",
                            "model": "test-model",
                            "messages": [{"role": "user", "content": "hello"}],
                        },
                    },
                )
                first_delta = _read_payload_timeout(proc)
                self.assertEqual(first_delta["request_id"], "chat-1")
                self.assertEqual(first_delta["kind"], "ai.chat.delta")

                _write_payload(
                    proc,
                    {
                        "request_id": "cancel-1",
                        "kind": "ai.chat.cancel",
                        "args": {"target_request_id": "chat-1"},
                    },
                )

                cancel_response = None
                terminal = None
                deadline = time.time() + 5
                while time.time() < deadline and (cancel_response is None or terminal is None):
                    payload = _read_payload_timeout(proc, max(0.1, deadline - time.time()))
                    if payload.get("request_id") == "cancel-1":
                        cancel_response = payload
                    if payload.get("request_id") == "chat-1" and payload.get("kind") == "ai.chat":
                        terminal = payload

                self.assertIsNotNone(cancel_response)
                self.assertEqual(cancel_response["kind"], "ai.chat.cancel")
                self.assertTrue(cancel_response["data"]["cancelled"])
                self.assertIsNotNone(terminal)
                self.assertEqual(terminal["data"]["finish_reason"], "cancelled")

                _write_payload(proc, {"request_id": "shutdown-1", "kind": "daemon.shutdown"})
                self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.shutdown")
                code, stderr = _close_daemon(proc)
                self.assertEqual(code, 0, stderr)
                self.assertEqual(stderr, "")
        finally:
            server.shutdown()
            server.server_close()

    def test_ai_chat_cancel_before_chat_registers_is_queued(self):
        with _SlowChatHandler.request_count_lock:
            _SlowChatHandler.request_count = 0
        server = ThreadingHTTPServer(("127.0.0.1", 0), _SlowChatHandler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        base_url = f"http://127.0.0.1:{server.server_port}/v1"
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-daemon-") as tmp:
                data_root = Path(tmp) / "data"
                proc = _start_daemon(data_root)

                ready = _read_payload_timeout(proc)
                self.assertEqual(ready["kind"], "daemon.ready")

                _write_payload(
                    proc,
                    {
                        "request_id": "provider-1",
                        "kind": "ai.providers.create",
                        "args": {
                            "name": "slow-local",
                            "base_url": base_url,
                            "kind": "local",
                        },
                    },
                )
                provider = _read_payload_timeout(proc)
                self.assertEqual(provider["kind"], "ai.providers.create")

                _write_payload(
                    proc,
                    {
                        "request_id": "cancel-early",
                        "kind": "ai.chat.cancel",
                        "args": {"target_request_id": "chat-early"},
                    },
                )
                cancel_response = _read_payload_timeout(proc)
                self.assertEqual(cancel_response["kind"], "ai.chat.cancel")
                self.assertTrue(cancel_response["data"]["cancelled"])
                self.assertTrue(cancel_response["data"]["queued"])

                _write_payload(
                    proc,
                    {
                        "request_id": "chat-early",
                        "kind": "ai.chat",
                        "args": {
                            "provider": "slow-local",
                            "model": "test-model",
                            "messages": [{"role": "user", "content": "hello"}],
                        },
                    },
                )
                terminal = _read_payload_timeout(proc)
                self.assertEqual(terminal["request_id"], "chat-early")
                self.assertEqual(terminal["kind"], "ai.chat")
                self.assertEqual(terminal["data"]["finish_reason"], "cancelled")
                with _SlowChatHandler.request_count_lock:
                    self.assertEqual(_SlowChatHandler.request_count, 0)

                _write_payload(proc, {"request_id": "shutdown-1", "kind": "daemon.shutdown"})
                self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.shutdown")
                code, stderr = _close_daemon(proc)
                self.assertEqual(code, 0, stderr)
                self.assertEqual(stderr, "")
        finally:
            server.shutdown()
            server.server_close()

    def test_ai_chat_read_only_tool_loop_emits_tool_records(self):
        server = _start_tool_chat_server(
            [
                (_tool_call_message("ui_overview_snapshot"), 0.0),
                (
                    _chat_completion_response(
                        {"role": "assistant", "content": "No transactions yet."},
                    ),
                    0.0,
                ),
            ]
        )
        base_url = f"http://127.0.0.1:{server.server_port}/v1"
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-daemon-") as tmp:
                proc = _start_daemon(Path(tmp) / "data")
                self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.ready")

                _write_payload(
                    proc,
                    {
                        "request_id": "provider-1",
                        "kind": "ai.providers.create",
                        "args": {
                            "name": "tool-local",
                            "base_url": base_url,
                            "kind": "local",
                        },
                    },
                )
                self.assertEqual(_read_payload_timeout(proc)["kind"], "ai.providers.create")

                _write_payload(
                    proc,
                    {
                        "request_id": "chat-tools-1",
                        "kind": "ai.chat",
                        "args": {
                            "provider": "tool-local",
                            "model": "test-model",
                            "tools_enabled": True,
                            "messages": [{"role": "user", "content": "What is pending?"}],
                        },
                    },
                )

                records = []
                terminal = None
                deadline = time.time() + 5
                while time.time() < deadline and terminal is None:
                    payload = _read_payload_timeout(proc, max(0.1, deadline - time.time()))
                    if payload.get("request_id") != "chat-tools-1":
                        continue
                    records.append(payload)
                    if payload.get("kind") == "ai.chat.delta":
                        payload = _read_payload(proc)
                        records.append(payload)
                    if payload.get("kind") == "ai.chat":
                        terminal = payload

                kinds = [record["kind"] for record in records]
                self.assertIn("ai.chat.tool_call", kinds)
                self.assertIn("ai.chat.tool_result", kinds)
                self.assertIn("ai.chat.delta", kinds)
                self.assertIsNotNone(terminal)
                self.assertEqual(terminal["data"]["finish_reason"], "stop")
                tool_call = next(
                    record for record in records if record["kind"] == "ai.chat.tool_call"
                )
                self.assertEqual(tool_call["data"]["name"], "ui.overview.snapshot")
                tool_result = next(
                    record for record in records if record["kind"] == "ai.chat.tool_result"
                )
                self.assertTrue(tool_result["data"]["ok"])
                self.assertEqual(tool_result["data"]["envelope"]["kind"], "ui.overview.snapshot")
                self.assertEqual(len(server.requests), 2)  # type: ignore[attr-defined]
                self.assertTrue(server.requests[0]["tools"])  # type: ignore[attr-defined]
                self.assertTrue(
                    any(
                        message.get("role") == "tool"
                        for message in server.requests[1]["messages"]  # type: ignore[attr-defined]
                    )
                )
                tool_names = {
                    tool["function"]["name"]
                    for tool in server.requests[0]["tools"]  # type: ignore[attr-defined]
                }
                self.assertIn("ui_overview_snapshot", tool_names)
                self.assertNotIn("ui.overview.snapshot", tool_names)

                _write_payload(proc, {"request_id": "shutdown-1", "kind": "daemon.shutdown"})
                self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.shutdown")
                code, stderr = _close_daemon(proc)
                self.assertEqual(code, 0, stderr)
                self.assertEqual(stderr, "")
        finally:
            server.shutdown()
            server.server_close()

    def test_ai_chat_pending_question_uses_health_and_next_actions_tools(self):
        server = _start_tool_chat_server(
            [
                (_tool_call_message("ui_workspace_health", call_id="call_health"), 0.0),
                (_tool_call_message("ui_next_actions", call_id="call_next"), 0.0),
                (
                    _chat_completion_response(
                        {"role": "assistant", "content": "Create a workspace first."},
                    ),
                    0.0,
                ),
            ]
        )
        base_url = f"http://127.0.0.1:{server.server_port}/v1"
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-daemon-") as tmp:
                proc = _start_daemon(Path(tmp) / "data")
                self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.ready")

                _write_payload(
                    proc,
                    {
                        "request_id": "provider-1",
                        "kind": "ai.providers.create",
                        "args": {
                            "name": "tool-local",
                            "base_url": base_url,
                            "kind": "local",
                        },
                    },
                )
                self.assertEqual(_read_payload_timeout(proc)["kind"], "ai.providers.create")

                _write_payload(
                    proc,
                    {
                        "request_id": "chat-pending-1",
                        "kind": "ai.chat",
                        "args": {
                            "provider": "tool-local",
                            "model": "test-model",
                            "tools_enabled": True,
                            "messages": [{"role": "user", "content": "What is pending?"}],
                        },
                    },
                )

                records = []
                terminal = None
                deadline = time.time() + 5
                while time.time() < deadline and terminal is None:
                    payload = _read_payload_timeout(proc, max(0.1, deadline - time.time()))
                    if payload.get("request_id") != "chat-pending-1":
                        continue
                    records.append(payload)
                    if payload.get("kind") == "ai.chat":
                        terminal = payload

                self.assertIsNotNone(terminal)
                self.assertEqual(terminal["data"]["finish_reason"], "stop")
                tool_results = [
                    record
                    for record in records
                    if record["kind"] == "ai.chat.tool_result"
                ]
                self.assertEqual(
                    [record["data"]["envelope"]["kind"] for record in tool_results],
                    ["ui.workspace.health", "ui.next_actions"],
                )
                self.assertEqual(len(server.requests), 3)  # type: ignore[attr-defined]
                first_tool_names = {
                    tool["function"]["name"]
                    for tool in server.requests[0]["tools"]  # type: ignore[attr-defined]
                }
                self.assertIn("ui_workspace_health", first_tool_names)
                self.assertIn("ui_next_actions", first_tool_names)
                self.assertTrue(
                    any(
                        message.get("role") == "tool"
                        and message.get("tool_call_id") == "call_health"
                        for message in server.requests[1]["messages"]  # type: ignore[attr-defined]
                    )
                )
                self.assertTrue(
                    any(
                        message.get("role") == "tool"
                        and message.get("tool_call_id") == "call_next"
                        for message in server.requests[2]["messages"]  # type: ignore[attr-defined]
                    )
                )

                _write_payload(proc, {"request_id": "shutdown-1", "kind": "daemon.shutdown"})
                self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.shutdown")
                code, stderr = _close_daemon(proc)
                self.assertEqual(code, 0, stderr)
                self.assertEqual(stderr, "")
        finally:
            server.shutdown()
            server.server_close()

    def test_ai_chat_unknown_tool_call_returns_tool_not_allowed(self):
        server = _start_tool_chat_server(
            [
                (_tool_call_message("erase_everything"), 0.0),
                (
                    _chat_completion_response(
                        {"role": "assistant", "content": "I cannot run that."},
                    ),
                    0.0,
                ),
            ]
        )
        base_url = f"http://127.0.0.1:{server.server_port}/v1"
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-daemon-") as tmp:
                proc = _start_daemon(Path(tmp) / "data")
                self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.ready")
                _write_payload(
                    proc,
                    {
                        "request_id": "provider-1",
                        "kind": "ai.providers.create",
                        "args": {"name": "tool-local", "base_url": base_url, "kind": "local"},
                    },
                )
                self.assertEqual(_read_payload_timeout(proc)["kind"], "ai.providers.create")
                _write_payload(
                    proc,
                    {
                        "request_id": "chat-tools-2",
                        "kind": "ai.chat",
                        "args": {
                            "provider": "tool-local",
                            "model": "test-model",
                            "tools_enabled": True,
                            "messages": [{"role": "user", "content": "Delete things"}],
                        },
                    },
                )
                result = None
                terminal = None
                deadline = time.time() + 5
                while time.time() < deadline and terminal is None:
                    payload = _read_payload_timeout(proc, max(0.1, deadline - time.time()))
                    if payload.get("request_id") != "chat-tools-2":
                        continue
                    if payload.get("kind") == "ai.chat.tool_result":
                        result = payload
                    if payload.get("kind") == "ai.chat.delta":
                        payload = _read_payload(proc)
                    if payload.get("kind") == "ai.chat":
                        terminal = payload
                self.assertIsNotNone(result)
                self.assertFalse(result["data"]["ok"])
                self.assertEqual(result["data"]["reason"], "tool_not_allowed")
                self.assertIsNotNone(terminal)

                _write_payload(proc, {"request_id": "shutdown-1", "kind": "daemon.shutdown"})
                self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.shutdown")
                code, stderr = _close_daemon(proc)
                self.assertEqual(code, 0, stderr)
                self.assertEqual(stderr, "")
        finally:
            server.shutdown()
            server.server_close()

    def test_ai_chat_mutating_tool_deny_continues_after_consent(self):
        server = _start_tool_chat_server(
            [
                (_tool_call_message("ui_wallets_sync"), 0.0),
                (
                    _chat_completion_response(
                        {"role": "assistant", "content": "Okay, I did not sync."},
                    ),
                    0.0,
                ),
            ]
        )
        base_url = f"http://127.0.0.1:{server.server_port}/v1"
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-daemon-") as tmp:
                proc = _start_daemon(Path(tmp) / "data")
                self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.ready")
                _write_payload(
                    proc,
                    {
                        "request_id": "provider-1",
                        "kind": "ai.providers.create",
                        "args": {"name": "tool-local", "base_url": base_url, "kind": "local"},
                    },
                )
                self.assertEqual(_read_payload_timeout(proc)["kind"], "ai.providers.create")
                _write_payload(
                    proc,
                    {
                        "request_id": "chat-mutating-deny",
                        "kind": "ai.chat",
                        "args": {
                            "provider": "tool-local",
                            "model": "test-model",
                            "tools_enabled": True,
                            "messages": [{"role": "user", "content": "Sync my wallets"}],
                        },
                    },
                )

                consent = None
                records = []
                deadline = time.time() + 5
                while time.time() < deadline and consent is None:
                    payload = _read_payload_timeout(proc, max(0.1, deadline - time.time()))
                    if payload.get("request_id") != "chat-mutating-deny":
                        continue
                    records.append(payload)
                    if payload.get("kind") == "ai.chat.tool_call":
                        payload = _read_payload(proc)
                        records.append(payload)
                    if payload.get("kind") == "ai.chat.tool_consent_required":
                        consent = payload

                self.assertIsNotNone(consent)
                self.assertEqual(consent["data"]["name"], "ui.wallets.sync")
                with self.assertRaises(AssertionError):
                    _read_payload_timeout(proc, 0.2)

                _write_payload(
                    proc,
                    {
                        "request_id": "consent-deny-1",
                        "kind": "ai.tool_call.consent",
                        "args": {
                            "target_request_id": "chat-mutating-deny",
                            "call_id": "call_1",
                            "decision": "deny",
                        },
                    },
                )
                consent_response = None
                result = None
                terminal = None
                deadline = time.time() + 5
                while time.time() < deadline and terminal is None:
                    payload = _read_payload_timeout(proc, max(0.1, deadline - time.time()))
                    if payload.get("request_id") == "consent-deny-1":
                        consent_response = payload
                    if payload.get("request_id") != "chat-mutating-deny":
                        continue
                    if payload.get("kind") == "ai.chat.tool_call":
                        payload = _read_payload(proc)
                    if payload.get("kind") == "ai.chat.tool_result":
                        result = payload
                    if payload.get("kind") == "ai.chat.delta":
                        payload = _read_payload(proc)
                    if payload.get("kind") == "ai.chat":
                        terminal = payload

                self.assertIsNotNone(consent_response)
                self.assertTrue(consent_response["data"]["recorded"])
                self.assertIsNotNone(result)
                self.assertFalse(result["data"]["ok"])
                self.assertEqual(result["data"]["reason"], "user_denied")
                self.assertIsNotNone(terminal)
                self.assertEqual(len(server.requests), 2)  # type: ignore[attr-defined]
                self.assertTrue(
                    any(
                        message.get("role") == "tool"
                        and "user_denied" in message.get("content", "")
                        for message in server.requests[1]["messages"]  # type: ignore[attr-defined]
                    )
                )

                _write_payload(proc, {"request_id": "shutdown-1", "kind": "daemon.shutdown"})
                self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.shutdown")
                code, stderr = _close_daemon(proc)
                self.assertEqual(code, 0, stderr)
                self.assertEqual(stderr, "")
        finally:
            server.shutdown()
            server.server_close()

    def test_ai_tool_consent_wrong_call_id_returns_not_found_for_active_chat(self):
        server = _start_tool_chat_server(
            [
                (_tool_call_message("ui_wallets_sync"), 0.0),
                (
                    _chat_completion_response(
                        {"role": "assistant", "content": "Should not happen."},
                    ),
                    0.0,
                ),
            ]
        )
        base_url = f"http://127.0.0.1:{server.server_port}/v1"
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-daemon-") as tmp:
                proc = _start_daemon(Path(tmp) / "data")
                self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.ready")
                _write_payload(
                    proc,
                    {
                        "request_id": "provider-1",
                        "kind": "ai.providers.create",
                        "args": {"name": "tool-local", "base_url": base_url, "kind": "local"},
                    },
                )
                self.assertEqual(_read_payload_timeout(proc)["kind"], "ai.providers.create")
                _write_payload(
                    proc,
                    {
                        "request_id": "chat-mutating-wrong-consent",
                        "kind": "ai.chat",
                        "args": {
                            "provider": "tool-local",
                            "model": "test-model",
                            "tools_enabled": True,
                            "messages": [{"role": "user", "content": "Sync my wallets"}],
                        },
                    },
                )
                while True:
                    payload = _read_payload_timeout(proc)
                    if (
                        payload.get("request_id") == "chat-mutating-wrong-consent"
                        and payload.get("kind") == "ai.chat.tool_call"
                    ):
                        payload = _read_payload(proc)
                    if (
                        payload.get("request_id") == "chat-mutating-wrong-consent"
                        and payload.get("kind") == "ai.chat.tool_consent_required"
                    ):
                        break

                _write_payload(
                    proc,
                    {
                        "request_id": "consent-wrong-call-1",
                        "kind": "ai.tool_call.consent",
                        "args": {
                            "target_request_id": "chat-mutating-wrong-consent",
                            "call_id": "wrong_call",
                            "decision": "allow_once",
                        },
                    },
                )
                response = _read_payload_timeout(proc)
                self.assertEqual(response["request_id"], "consent-wrong-call-1")
                self.assertEqual(response["kind"], "ai.tool_call.consent")
                self.assertFalse(response["data"]["recorded"])
                self.assertEqual(response["data"]["reason"], "not_found")

                _write_payload(
                    proc,
                    {
                        "request_id": "cancel-wrong-call-1",
                        "kind": "ai.chat.cancel",
                        "args": {"target_request_id": "chat-mutating-wrong-consent"},
                    },
                )
                cancel_response = None
                terminal = None
                deadline = time.time() + 5
                while time.time() < deadline and (cancel_response is None or terminal is None):
                    payload = _read_payload_timeout(proc, max(0.1, deadline - time.time()))
                    if payload.get("request_id") == "cancel-wrong-call-1":
                        cancel_response = payload
                    if (
                        payload.get("request_id") == "chat-mutating-wrong-consent"
                        and payload.get("kind") == "ai.chat"
                    ):
                        terminal = payload

                self.assertIsNotNone(cancel_response)
                self.assertIsNotNone(terminal)
                self.assertEqual(terminal["data"]["finish_reason"], "cancelled")

                _write_payload(proc, {"request_id": "shutdown-1", "kind": "daemon.shutdown"})
                self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.shutdown")
                code, stderr = _close_daemon(proc)
                self.assertEqual(code, 0, stderr)
                self.assertEqual(stderr, "")
        finally:
            server.shutdown()
            server.server_close()

    def test_ai_chat_mutating_tool_allow_once_executes(self):
        server = _start_tool_chat_server(
            [
                (_tool_call_message("ui_wallets_sync"), 0.0),
                (
                    _chat_completion_response(
                        {"role": "assistant", "content": "Sync finished."},
                    ),
                    0.0,
                ),
            ]
        )
        base_url = f"http://127.0.0.1:{server.server_port}/v1"
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-daemon-") as tmp:
                proc = _start_daemon(Path(tmp) / "data")
                self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.ready")
                _write_payload(
                    proc,
                    {
                        "request_id": "provider-1",
                        "kind": "ai.providers.create",
                        "args": {"name": "tool-local", "base_url": base_url, "kind": "local"},
                    },
                )
                self.assertEqual(_read_payload_timeout(proc)["kind"], "ai.providers.create")
                _write_payload(
                    proc,
                    {
                        "request_id": "chat-mutating-allow",
                        "kind": "ai.chat",
                        "args": {
                            "provider": "tool-local",
                            "model": "test-model",
                            "tools_enabled": True,
                            "messages": [{"role": "user", "content": "Sync my wallets"}],
                        },
                    },
                )
                while True:
                    payload = _read_payload_timeout(proc)
                    if (
                        payload.get("request_id") == "chat-mutating-allow"
                        and payload.get("kind") == "ai.chat.tool_call"
                    ):
                        payload = _read_payload(proc)
                    if (
                        payload.get("request_id") == "chat-mutating-allow"
                        and payload.get("kind") == "ai.chat.tool_consent_required"
                    ):
                        break
                _write_payload(
                    proc,
                    {
                        "request_id": "consent-allow-1",
                        "kind": "ai.tool_call.consent",
                        "args": {
                            "target_request_id": "chat-mutating-allow",
                            "call_id": "call_1",
                            "decision": "allow_once",
                        },
                    },
                )
                result = None
                terminal = None
                deadline = time.time() + 5
                while time.time() < deadline and terminal is None:
                    payload = _read_payload_timeout(proc, max(0.1, deadline - time.time()))
                    if payload.get("request_id") != "chat-mutating-allow":
                        continue
                    if payload.get("kind") == "ai.chat.tool_call":
                        payload = _read_payload(proc)
                    if payload.get("kind") == "ai.chat.tool_result":
                        result = payload
                    if payload.get("kind") == "ai.chat.delta":
                        payload = _read_payload(proc)
                    if payload.get("kind") == "ai.chat":
                        terminal = payload

                self.assertIsNotNone(result)
                self.assertTrue(result["data"]["ok"])
                self.assertEqual(result["data"]["envelope"]["kind"], "ui.wallets.sync")
                self.assertEqual(result["data"]["envelope"]["data"]["results"], [])
                self.assertIsNotNone(terminal)

                _write_payload(proc, {"request_id": "shutdown-1", "kind": "daemon.shutdown"})
                self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.shutdown")
                code, stderr = _close_daemon(proc)
                self.assertEqual(code, 0, stderr)
                self.assertEqual(stderr, "")
        finally:
            server.shutdown()
            server.server_close()

    def test_ai_chat_mutating_tool_allow_session_skips_second_prompt(self):
        server = _start_tool_chat_server(
            [
                (_tool_call_message("ui_wallets_sync", call_id="call_1"), 0.0),
                (_tool_call_message("ui_wallets_sync", call_id="call_2"), 0.0),
                (
                    _chat_completion_response(
                        {"role": "assistant", "content": "Both sync calls finished."},
                    ),
                    0.0,
                ),
            ]
        )
        base_url = f"http://127.0.0.1:{server.server_port}/v1"
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-daemon-") as tmp:
                proc = _start_daemon(Path(tmp) / "data")
                self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.ready")
                _write_payload(
                    proc,
                    {
                        "request_id": "provider-1",
                        "kind": "ai.providers.create",
                        "args": {"name": "tool-local", "base_url": base_url, "kind": "local"},
                    },
                )
                self.assertEqual(_read_payload_timeout(proc)["kind"], "ai.providers.create")
                _write_payload(
                    proc,
                    {
                        "request_id": "chat-mutating-session",
                        "kind": "ai.chat",
                        "args": {
                            "provider": "tool-local",
                            "model": "test-model",
                            "tools_enabled": True,
                            "messages": [{"role": "user", "content": "Sync twice"}],
                        },
                    },
                )
                while True:
                    payload = _read_payload_timeout(proc)
                    if (
                        payload.get("request_id") == "chat-mutating-session"
                        and payload.get("kind") == "ai.chat.tool_call"
                    ):
                        payload = _read_payload(proc)
                    if (
                        payload.get("request_id") == "chat-mutating-session"
                        and payload.get("kind") == "ai.chat.tool_consent_required"
                    ):
                        break
                _write_payload(
                    proc,
                    {
                        "request_id": "consent-session-1",
                        "kind": "ai.tool_call.consent",
                        "args": {
                            "target_request_id": "chat-mutating-session",
                            "call_id": "call_1",
                            "decision": "allow_session",
                        },
                    },
                )
                consent_required_count = 1
                tool_result_count = 0
                terminal = None
                deadline = time.time() + 5
                while time.time() < deadline and terminal is None:
                    payload = _read_payload_timeout(proc, max(0.1, deadline - time.time()))
                    if payload.get("request_id") != "chat-mutating-session":
                        continue
                    if payload.get("kind") == "ai.chat.tool_call":
                        payload = _read_payload(proc)
                    if payload.get("kind") == "ai.chat.tool_consent_required":
                        consent_required_count += 1
                    if payload.get("kind") == "ai.chat.tool_result":
                        tool_result_count += 1
                    if payload.get("kind") == "ai.chat.delta":
                        payload = _read_payload(proc)
                    if payload.get("kind") == "ai.chat":
                        terminal = payload

                self.assertIsNotNone(terminal)
                self.assertEqual(consent_required_count, 1)
                self.assertEqual(tool_result_count, 2)

                _write_payload(proc, {"request_id": "shutdown-1", "kind": "daemon.shutdown"})
                self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.shutdown")
                code, stderr = _close_daemon(proc)
                self.assertEqual(code, 0, stderr)
                self.assertEqual(stderr, "")
        finally:
            server.shutdown()
            server.server_close()

    def test_ai_chat_cancel_while_waiting_for_tool_consent_finishes_cancelled(self):
        server = _start_tool_chat_server(
            [
                (_tool_call_message("ui_wallets_sync"), 0.0),
                (
                    _chat_completion_response(
                        {"role": "assistant", "content": "Should not happen."},
                    ),
                    0.0,
                ),
            ]
        )
        base_url = f"http://127.0.0.1:{server.server_port}/v1"
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-daemon-") as tmp:
                proc = _start_daemon(Path(tmp) / "data")
                self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.ready")
                _write_payload(
                    proc,
                    {
                        "request_id": "provider-1",
                        "kind": "ai.providers.create",
                        "args": {"name": "tool-local", "base_url": base_url, "kind": "local"},
                    },
                )
                self.assertEqual(_read_payload_timeout(proc)["kind"], "ai.providers.create")
                _write_payload(
                    proc,
                    {
                        "request_id": "chat-mutating-cancel",
                        "kind": "ai.chat",
                        "args": {
                            "provider": "tool-local",
                            "model": "test-model",
                            "tools_enabled": True,
                            "messages": [{"role": "user", "content": "Sync my wallets"}],
                        },
                    },
                )
                while True:
                    payload = _read_payload_timeout(proc)
                    if (
                        payload.get("request_id") == "chat-mutating-cancel"
                        and payload.get("kind") == "ai.chat.tool_call"
                    ):
                        payload = _read_payload(proc)
                    if (
                        payload.get("request_id") == "chat-mutating-cancel"
                        and payload.get("kind") == "ai.chat.tool_consent_required"
                    ):
                        break
                _write_payload(
                    proc,
                    {
                        "request_id": "cancel-consent-1",
                        "kind": "ai.chat.cancel",
                        "args": {"target_request_id": "chat-mutating-cancel"},
                    },
                )
                cancel_response = None
                terminal = None
                deadline = time.time() + 5
                while time.time() < deadline and (cancel_response is None or terminal is None):
                    payload = _read_payload_timeout(proc, max(0.1, deadline - time.time()))
                    if payload.get("request_id") == "cancel-consent-1":
                        cancel_response = payload
                    if (
                        payload.get("request_id") == "chat-mutating-cancel"
                        and payload.get("kind") == "ai.chat"
                    ):
                        terminal = payload

                self.assertIsNotNone(cancel_response)
                self.assertTrue(cancel_response["data"]["cancelled"])
                self.assertIsNotNone(terminal)
                self.assertEqual(terminal["data"]["finish_reason"], "cancelled")
                self.assertEqual(len(server.requests), 1)  # type: ignore[attr-defined]

                _write_payload(proc, {"request_id": "shutdown-1", "kind": "daemon.shutdown"})
                self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.shutdown")
                code, stderr = _close_daemon(proc)
                self.assertEqual(code, 0, stderr)
                self.assertEqual(stderr, "")
        finally:
            server.shutdown()
            server.server_close()

    def test_ai_chat_mutating_tool_consent_preview_redacts_secrets(self):
        server = _start_tool_chat_server(
            [
                (
                    _tool_call_message(
                        "ui_wallets_sync",
                        arguments=json.dumps(
                            {
                                "wallet": "cold",
                                "descriptor": "wpkh(secret)",
                                "config_json": {"token": "secret"},
                            }
                        ),
                    ),
                    0.0,
                ),
            ]
        )
        base_url = f"http://127.0.0.1:{server.server_port}/v1"
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-daemon-") as tmp:
                proc = _start_daemon(Path(tmp) / "data")
                self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.ready")
                _write_payload(
                    proc,
                    {
                        "request_id": "provider-1",
                        "kind": "ai.providers.create",
                        "args": {"name": "tool-local", "base_url": base_url, "kind": "local"},
                    },
                )
                self.assertEqual(_read_payload_timeout(proc)["kind"], "ai.providers.create")
                _write_payload(
                    proc,
                    {
                        "request_id": "chat-mutating-redact",
                        "kind": "ai.chat",
                        "args": {
                            "provider": "tool-local",
                            "model": "test-model",
                            "tools_enabled": True,
                            "messages": [{"role": "user", "content": "Sync cold"}],
                        },
                    },
                )
                consent = None
                deadline = time.time() + 5
                while time.time() < deadline and consent is None:
                    payload = _read_payload_timeout(proc, max(0.1, deadline - time.time()))
                    if (
                        payload.get("request_id") == "chat-mutating-redact"
                        and payload.get("kind") == "ai.chat.tool_call"
                    ):
                        payload = _read_payload(proc)
                    if (
                        payload.get("request_id") == "chat-mutating-redact"
                        and payload.get("kind") == "ai.chat.tool_consent_required"
                    ):
                        consent = payload
                self.assertIsNotNone(consent)
                preview = consent["data"]["arguments_preview"]
                self.assertEqual(preview["wallet"], "cold")
                self.assertEqual(preview["descriptor"], "<redacted>")
                self.assertEqual(preview["config_json"], "<redacted>")

                _write_payload(
                    proc,
                    {
                        "request_id": "consent-redact-1",
                        "kind": "ai.tool_call.consent",
                        "args": {
                            "target_request_id": "chat-mutating-redact",
                            "call_id": "call_1",
                            "decision": "deny",
                        },
                    },
                )
                self.assertEqual(_read_payload_timeout(proc)["kind"], "ai.tool_call.consent")
                while True:
                    payload = _read_payload_timeout(proc)
                    if (
                        payload.get("request_id") == "chat-mutating-redact"
                        and payload.get("kind") == "ai.chat"
                    ):
                        break

                _write_payload(proc, {"request_id": "shutdown-1", "kind": "daemon.shutdown"})
                self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.shutdown")
                code, stderr = _close_daemon(proc)
                self.assertEqual(code, 0, stderr)
                self.assertEqual(stderr, "")
        finally:
            server.shutdown()
            server.server_close()

    def test_ai_chat_cancel_during_tool_loop_finishes_cancelled(self):
        server = _start_tool_chat_server(
            [
                (_tool_call_message("ui_overview_snapshot"), 0.0),
                (
                    _chat_completion_response(
                        {"role": "assistant", "content": "This should be suppressed."},
                    ),
                    0.4,
                ),
            ]
        )
        base_url = f"http://127.0.0.1:{server.server_port}/v1"
        try:
            with tempfile.TemporaryDirectory(prefix="kassiber-daemon-") as tmp:
                proc = _start_daemon(Path(tmp) / "data")
                self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.ready")
                _write_payload(
                    proc,
                    {
                        "request_id": "provider-1",
                        "kind": "ai.providers.create",
                        "args": {"name": "tool-local", "base_url": base_url, "kind": "local"},
                    },
                )
                self.assertEqual(_read_payload_timeout(proc)["kind"], "ai.providers.create")
                _write_payload(
                    proc,
                    {
                        "request_id": "chat-tools-3",
                        "kind": "ai.chat",
                        "args": {
                            "provider": "tool-local",
                            "model": "test-model",
                            "tools_enabled": True,
                            "messages": [{"role": "user", "content": "Read overview"}],
                        },
                    },
                )
                while True:
                    payload = _read_payload_timeout(proc)
                    if (
                        payload.get("request_id") == "chat-tools-3"
                        and payload.get("kind") == "ai.chat.tool_call"
                    ):
                        payload = _read_payload(proc)
                    if (
                        payload.get("request_id") == "chat-tools-3"
                        and payload.get("kind")
                        in {
                            "ai.chat.tool_call",
                            "ai.chat.tool_result",
                        }
                    ):
                        break
                _write_payload(
                    proc,
                    {
                        "request_id": "cancel-tools-1",
                        "kind": "ai.chat.cancel",
                        "args": {"target_request_id": "chat-tools-3"},
                    },
                )
                cancel_response = None
                terminal = None
                deadline = time.time() + 5
                while time.time() < deadline and (cancel_response is None or terminal is None):
                    payload = _read_payload_timeout(proc, max(0.1, deadline - time.time()))
                    if payload.get("request_id") == "cancel-tools-1":
                        cancel_response = payload
                    if (
                        payload.get("request_id") == "chat-tools-3"
                        and payload.get("kind") == "ai.chat"
                    ):
                        terminal = payload
                self.assertIsNotNone(cancel_response)
                self.assertTrue(cancel_response["data"]["cancelled"])
                self.assertIsNotNone(terminal)
                self.assertEqual(terminal["data"]["finish_reason"], "cancelled")

                _write_payload(proc, {"request_id": "shutdown-1", "kind": "daemon.shutdown"})
                self.assertEqual(_read_payload_timeout(proc)["kind"], "daemon.shutdown")
                code, stderr = _close_daemon(proc)
                self.assertEqual(code, 0, stderr)
                self.assertEqual(stderr, "")
        finally:
            server.shutdown()
            server.server_close()

    def test_daemon_error_paths_are_structured(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-daemon-") as tmp:
            proc = _start_daemon(Path(tmp) / "data")
            self.assertEqual(_read_payload(proc)["kind"], "daemon.ready")

            cases = [
                ("{", "invalid_json", None, True, None),
                ([1, 2, 3], "validation", None, True, None),
                (
                    "x" * (MAX_REQUEST_LINE_CHARS + 1),
                    "request_too_large",
                    None,
                    True,
                    False,
                ),
                ({"request_id": "missing-kind"}, "validation", "missing-kind", False, None),
                (
                    {"request_id": "numeric-kind", "kind": 42},
                    "validation",
                    "numeric-kind",
                    False,
                    None,
                ),
                (
                    {"request_id": "cancel-1", "kind": "cancel"},
                    "unsupported_kind",
                    "cancel-1",
                    False,
                    None,
                ),
                (
                    {"request_id": "ui-1", "kind": "ui.overview.snapshot"},
                    None,
                    "ui-1",
                    False,
                    None,
                ),
                (
                    {"request_id": "unknown-1", "kind": "rates.latest"},
                    "unsupported_kind",
                    "unknown-1",
                    False,
                    False,
                ),
                (
                    {"request_id": "ai-test-1", "kind": "ai.test_connection"},
                    "validation",
                    "ai-test-1",
                    False,
                    False,
                ),
                (
                    {
                        "request_id": "ai-test-2",
                        "kind": "ai.test_connection",
                        "args": {"base_url": "no-scheme"},
                    },
                    "validation",
                    "ai-test-2",
                    False,
                    False,
                ),
            ]
            for request, code, request_id, explicit_null_request_id, retryable in cases:
                with self.subTest(code=code, request=request):
                    _write_payload(proc, request)
                    response = _read_payload(proc)
                    self.assertEqual(response["schema_version"], 1)
                    if code is None:
                        self.assertIn(
                            response["kind"],
                            {"ui.overview.snapshot", "ui.transactions.list"},
                        )
                        self.assertEqual(response["data"]["txs"], [])
                    else:
                        self.assertEqual(response["kind"], "error")
                        self.assertEqual(response["error"]["code"], code)
                    if request_id is None:
                        self.assertIsNone(response.get("request_id"))
                        if explicit_null_request_id:
                            self.assertIn("request_id", response)
                    else:
                        self.assertEqual(response["request_id"], request_id)
                    if retryable is not None and code is not None:
                        self.assertEqual(response["error"]["retryable"], retryable)

            _write_payload(proc, {"request_id": "tx-1", "kind": "ui.transactions.list"})
            tx_response = _read_payload(proc)
            self.assertEqual(tx_response["request_id"], "tx-1")
            self.assertEqual(tx_response["kind"], "ui.transactions.list")
            self.assertEqual(tx_response["data"]["txs"], [])

            _write_payload(
                proc,
                {"request_id": "report-1", "kind": "ui.reports.capital_gains"},
            )
            report_response = _read_payload(proc)
            self.assertEqual(report_response["request_id"], "report-1")
            self.assertEqual(report_response["kind"], "ui.reports.capital_gains")
            self.assertEqual(report_response["data"]["lots"], [])

            _write_payload(
                proc,
                {"request_id": "journals-1", "kind": "ui.journals.snapshot"},
            )
            journals_response = _read_payload(proc)
            self.assertEqual(journals_response["request_id"], "journals-1")
            self.assertEqual(journals_response["kind"], "ui.journals.snapshot")
            self.assertEqual(journals_response["data"]["recent"], [])

            _write_payload(
                proc,
                {"request_id": "profiles-1", "kind": "ui.profiles.snapshot"},
            )
            profiles_response = _read_payload(proc)
            self.assertEqual(profiles_response["request_id"], "profiles-1")
            self.assertEqual(profiles_response["kind"], "ui.profiles.snapshot")
            self.assertEqual(profiles_response["data"]["workspaces"], [])
            self.assertEqual(profiles_response["data"]["activeProfileId"], "")

            _write_payload(
                proc,
                {
                    "request_id": "sync-1",
                    "kind": "ui.wallets.sync",
                    "args": {"all": True},
                },
            )
            sync_response = _read_payload(proc)
            self.assertEqual(sync_response["request_id"], "sync-1")
            self.assertEqual(sync_response["kind"], "ui.wallets.sync")
            self.assertEqual(sync_response["data"]["results"], [])

            _write_payload(proc, {"request_id": "shutdown-1", "kind": "daemon.shutdown"})
            self.assertEqual(_read_payload(proc)["kind"], "daemon.shutdown")
            code, stderr = _close_daemon(proc)
            self.assertEqual(code, 0, stderr)
            self.assertEqual(stderr, "")


if __name__ == "__main__":
    unittest.main()
