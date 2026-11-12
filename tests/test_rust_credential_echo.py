"""Python and the Rust binary withhold an echoed AI provider key alike.

`ai providers test` runs natively in Rust. Against the same loopback provider,
which repeats every credential the request carried, both runtimes must show
the same scrubbed error: code, message and the details that carry server text
(`body`, `body_truncated`, `status`). Hints differ between the runtimes for
reasons unrelated to credentials, so they are not compared.

Needs `KASSIBER_RUST_BIN`, a build of `crates/kassiber`.
"""

import json
import os
import tempfile
import unittest
from pathlib import Path

from tests import cli_runner
from tests.test_credential_echo import AI_KEY, CredentialEchoAssertions, EchoServer, json_echo

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")


@unittest.skipUnless(RUST_BIN, "KASSIBER_RUST_BIN is not set")
class RustAiProviderEchoParityTest(CredentialEchoAssertions, unittest.TestCase):
    def run_side(self, prefix, root, *args, secret=None):
        env = {key: value for key, value in os.environ.items() if key not in {"XDG_DATA_HOME", "KASSIBER_TEST_CLI"}}
        env.setdefault("KASSIBER_OFFLINE_PREFERENCE_FILE", str(root.parent / "offline-mode.json"))
        if prefix is not None:
            env["KASSIBER_TEST_CLI"] = json.dumps(prefix)
        return cli_runner.run(
            ["--data-root", str(root), "--machine", *args],
            capture_output=True, text=True, env=env, check=False,
            input=None if secret is None else f"{secret}\n", secret_input=secret is not None,
        )

    def test_provider_test_errors_match(self):
        outputs = {}
        with EchoServer(json_echo(401)) as server, tempfile.TemporaryDirectory() as tmp:
            for side, prefix in (("python", None), ("rust", [str(Path(RUST_BIN).resolve())])):
                root = Path(tmp) / side / "data"
                for args, secret in (
                    (("init",), None),
                    (("ai", "providers", "create", "remote", "--base-url", f"{server.url}/v1",
                      "--api-key-stdin", "--default-model", "m", "--kind", "remote"), AI_KEY),
                    (("ai", "providers", "update", "remote", "--acknowledge"), None),
                ):
                    # Setup runs in Python on both sides; only the test is compared.
                    result = self.run_side(None, root, *args, secret=secret)
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                result = self.run_side(prefix, root, "ai", "providers", "test", "remote")
                self.assertWithheld({side: result.stdout + result.stderr}, (AI_KEY,))
                error = json.loads(result.stdout)["error"]
                details = error.get("details") or {}
                outputs[side] = {
                    "code": error["code"],
                    "message": error["message"],
                    "details": {key: details.get(key) for key in ("body", "body_truncated", "status")},
                }
        self.assertEqual(outputs["rust"], outputs["python"])
        self.assertIn("<redacted>", outputs["python"]["details"]["body"])


if __name__ == "__main__":
    unittest.main()
