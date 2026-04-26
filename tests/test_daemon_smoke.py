import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class DaemonSmokeTest(unittest.TestCase):
    def test_daemon_ready_status_and_shutdown_jsonl(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-daemon-") as tmp:
            data_root = Path(tmp) / "data"
            proc = subprocess.Popen(
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
            assert proc.stdin is not None
            assert proc.stdout is not None

            ready = json.loads(proc.stdout.readline())
            self.assertEqual(ready["kind"], "daemon.ready")
            self.assertEqual(ready["schema_version"], 1)
            self.assertIn("status", ready["data"]["supported_kinds"])

            proc.stdin.write(
                json.dumps({"request_id": "status-1", "kind": "status"}) + "\n"
            )
            proc.stdin.flush()
            status = json.loads(proc.stdout.readline())
            self.assertEqual(status["request_id"], "status-1")
            self.assertEqual(status["kind"], "status")
            self.assertEqual(status["schema_version"], 1)
            self.assertEqual(status["data"]["auth"]["mode"], "local")
            self.assertEqual(status["data"]["data_root"], str(data_root))

            proc.stdin.write(
                json.dumps({"request_id": "shutdown-1", "kind": "daemon.shutdown"})
                + "\n"
            )
            proc.stdin.flush()
            shutdown = json.loads(proc.stdout.readline())
            self.assertEqual(shutdown["request_id"], "shutdown-1")
            self.assertEqual(shutdown["kind"], "daemon.shutdown")

            proc.stdin.close()
            stderr = proc.stderr.read() if proc.stderr is not None else ""
            if proc.stdout is not None:
                proc.stdout.close()
            if proc.stderr is not None:
                proc.stderr.close()
            self.assertEqual(proc.wait(timeout=5), 0, stderr)


if __name__ == "__main__":
    unittest.main()
