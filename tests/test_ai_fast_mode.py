"""Fast serving mode (Codex Fast service tier, Claude Opus fast mode).

The option is broker-only: it must cross to the provider broker only when it
is exactly ``True``, be rejected when it is not a boolean, never reach an
OpenAI-compatible HTTP body, and survive model-metadata normalization.
"""

from __future__ import annotations

import argparse
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from kassiber import daemon
from kassiber.ai.broker_client import BrokerAIClient
from kassiber.ai.client import _responses_options
from kassiber.ai.model_metadata import safe_model_capabilities
from kassiber.cli.chat import _chat_options
from kassiber.errors import AppError


FAKE_BROKER = """
import json, sys
request = json.loads(sys.stdin.readline())
with open(sys.argv[1] if len(sys.argv) > 1 else %(capture)r, "w") as handle:
    json.dump(request.get("options"), handle)
print(json.dumps({"type":"delta","content":"ok"}), flush=True)
print(json.dumps({"type":"done","finish_reason":"stop"}), flush=True)
"""


class BrokerFastModeTest(unittest.TestCase):
    def _options_sent(self, options: dict) -> object:
        with tempfile.TemporaryDirectory(prefix="kassiber-fast-test-") as tmp:
            capture = Path(tmp) / "options.json"
            script = Path(tmp) / "fake_broker.py"
            script.write_text(FAKE_BROKER % {"capture": str(capture)}, encoding="utf-8")
            with patch.dict("os.environ", {
                "KASSIBER_AI_BROKER_NODE": sys.executable,
                "KASSIBER_AI_PROVIDER_BROKER": str(script),
            }):
                client = BrokerAIClient(locator="codex-cli://default")
                chunks = list(client.stream_chat(
                    messages=[{"role": "user", "content": "Reply with just: ok"}],
                    model="gpt-5.4",
                    options=options,
                ))
            self.assertEqual(chunks[-1].finish_reason, "stop")
            return json.loads(capture.read_text(encoding="utf-8"))

    def test_forwards_only_an_explicit_true(self):
        self.assertEqual(
            self._options_sent({"fast_mode": True, "reasoning_effort": "high"}),
            {"reasoning_effort": "high", "fast_mode": True},
        )
        self.assertEqual(self._options_sent({"fast_mode": False}), {})

    def test_rejects_non_boolean_before_spawn(self):
        client = BrokerAIClient(locator="claude-cli://default")
        for value in ("true", 1, None):
            with self.subTest(value=value), patch("subprocess.Popen") as spawn:
                with self.assertRaises(AppError) as raised:
                    list(client.stream_chat(model="opus", options={"fast_mode": value}))
                self.assertEqual(raised.exception.code, "ai_request_invalid")
                spawn.assert_not_called()


class HttpFastModeTest(unittest.TestCase):
    def test_never_reaches_an_openai_compatible_body(self):
        body = _responses_options({"fast_mode": True, "reasoning_effort": "low"})
        self.assertNotIn("fast_mode", body)
        self.assertEqual(body["reasoning"]["effort"], "low")


class ModelMetadataFastModeTest(unittest.TestCase):
    def test_keeps_support_and_bounded_description(self):
        metadata = safe_model_capabilities({
            "supports_fast_mode": True,
            "fast_mode_description": "  2x speed, increased usage  ",
        })
        self.assertEqual(metadata["supports_fast_mode"], True)
        self.assertEqual(metadata["fast_mode_description"], "2x speed, increased usage")
        long = safe_model_capabilities({
            "supports_fast_mode": True,
            "fast_mode_description": "x" * 500,
        })
        self.assertLessEqual(len(long["fast_mode_description"]), 96)

    def test_drops_malformed_values(self):
        self.assertEqual(safe_model_capabilities({"supports_fast_mode": "yes"}), {})
        self.assertEqual(
            safe_model_capabilities({
                "supports_fast_mode": False,
                "fast_mode_description": "ignored without support",
            }),
            {"supports_fast_mode": False},
        )


class DaemonFastModeTest(unittest.TestCase):
    def _args(self, options: dict) -> dict:
        return daemon._ai_chat_args({
            "model": "gpt-5.4",
            "messages": [{"role": "user", "content": "Reply with just: ok"}],
            "options": options,
        })

    def test_accepts_boolean_fast_mode(self):
        self.assertEqual(self._args({"fast_mode": True})["options"], {"fast_mode": True})
        self.assertEqual(self._args({"fast_mode": False})["options"], {"fast_mode": False})

    def test_rejects_non_boolean_fast_mode(self):
        for value in ("true", 1, None, {}):
            with self.subTest(value=value):
                with self.assertRaises(AppError) as raised:
                    self._args({"fast_mode": value})
                self.assertEqual(raised.exception.code, "validation")


class CliFastFlagTest(unittest.TestCase):
    def test_fast_flag_adds_the_option(self):
        args = argparse.Namespace(
            temperature=None, max_tokens=None, reasoning_effort="auto", fast=True,
        )
        self.assertEqual(_chat_options(args), {"fast_mode": True})
        args.fast = False
        self.assertIsNone(_chat_options(args))

    def test_parser_exposes_fast(self):
        from kassiber.cli.main import build_parser

        parsed = build_parser().parse_args(["chat", "--fast", "hello"])
        self.assertIs(parsed.fast, True)
        self.assertIs(build_parser().parse_args(["chat", "hello"]).fast, False)


if __name__ == "__main__":
    unittest.main()
