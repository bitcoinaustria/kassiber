import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from kassiber.daemon import MAX_REQUEST_LINE_CHARS


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


def _write_payload(proc, payload):
    assert proc.stdin is not None
    line = payload if isinstance(payload, str) else json.dumps(payload)
    proc.stdin.write(line + "\n")
    proc.stdin.flush()


def _close_daemon(proc):
    if proc.stdin is not None and not proc.stdin.closed:
        proc.stdin.close()
    stderr = proc.stderr.read() if proc.stderr is not None else ""
    if proc.stdout is not None:
        proc.stdout.close()
    if proc.stderr is not None:
        proc.stderr.close()
    return proc.wait(timeout=5), stderr


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
            self.assertIn("ui.reports.capital_gains", ready["data"]["supported_kinds"])
            self.assertIn("ui.journals.snapshot", ready["data"]["supported_kinds"])
            self.assertIn("ui.profiles.snapshot", ready["data"]["supported_kinds"])
            self.assertIn("ui.wallets.sync", ready["data"]["supported_kinds"])
            self.assertIn("wallets.reveal_descriptor", ready["data"]["supported_kinds"])
            self.assertIn("backends.reveal_token", ready["data"]["supported_kinds"])
            self.assertIn("ai.test_connection", ready["data"]["supported_kinds"])
            self.assertIn("ai.chat", ready["data"]["supported_kinds"])

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
