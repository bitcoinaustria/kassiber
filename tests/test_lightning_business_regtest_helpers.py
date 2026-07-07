from __future__ import annotations

import subprocess
import sys
import textwrap
import unittest

from tests.integration.lightning_business_regtest import (
    _read_payload_timeout,
    _shutdown_daemon_process,
)


class LightningBusinessRegtestHelperTests(unittest.TestCase):
    def test_shutdown_teardown_kills_process_before_reading_stderr(self) -> None:
        script = textwrap.dedent(
            """
            import json
            import sys
            import time

            print(json.dumps({"kind": "daemon.ready"}), flush=True)
            for line in sys.stdin:
                payload = json.loads(line)
                if payload.get("kind") == "daemon.shutdown":
                    print(
                        json.dumps(
                            {
                                "kind": "daemon.shutdown",
                                "request_id": payload.get("request_id"),
                            }
                        ),
                        flush=True,
                    )
                    time.sleep(60)
            """
        )
        proc = subprocess.Popen(
            [sys.executable, "-c", script],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        ready = _read_payload_timeout(proc, 1.0)
        self.assertEqual(ready["kind"], "daemon.ready")

        with self.assertRaisesRegex(AssertionError, "daemon exited"):
            _shutdown_daemon_process(proc, timeout=0.2)

        self.assertIsNotNone(proc.poll())


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
