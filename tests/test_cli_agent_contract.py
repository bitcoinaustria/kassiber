from __future__ import annotations

from contextlib import redirect_stderr, redirect_stdout
import io
import json
from pathlib import Path
import tempfile
import unittest

from kassiber.cli.main import build_parser, main
from kassiber.envelope import build_envelope, derive_kind


def _run(*args: str):
    stdout = io.StringIO()
    stderr = io.StringIO()
    with redirect_stdout(stdout), redirect_stderr(stderr):
        code = main(list(args))
    return json.loads(stdout.getvalue()), code, stderr.getvalue()


class CliAgentContractTests(unittest.TestCase):
    def test_command_catalog_is_machine_readable_and_filterable(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            payload, code, _stderr = _run(
                "--data-root",
                str(data_root),
                "--machine",
                "commands",
                "describe",
                "wallets",
                "sync",
            )

            self.assertEqual(code, 0, payload)
            self.assertEqual(payload["kind"], "commands.describe")
            self.assertEqual(payload["data"]["count"], 1)
            command = payload["data"]["commands"][0]
            self.assertEqual(command["command"], "wallets sync")
            self.assertEqual(command["effect"], "mutating")
            self.assertTrue(command["needs_database"])
            self.assertIn("wallet", command["scope_flags"])
            self.assertFalse((data_root / "kassiber.sqlite3").exists())

    def test_preview_document_command_is_catalogued_as_read_only(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            payload, code, _stderr = _run(
                "--data-root",
                str(data_root),
                "--machine",
                "commands",
                "describe",
                "wallets",
                "preview-document",
            )

            self.assertEqual(code, 0, payload)
            self.assertEqual(payload["data"]["count"], 1)
            command = payload["data"]["commands"][0]
            self.assertEqual(command["command"], "wallets preview-document")
            self.assertEqual(command["kind"], "wallets.preview-document")
            self.assertEqual(command["effect"], "read_only")
            self.assertTrue(command["needs_database"])
            self.assertFalse((data_root / "kassiber.sqlite3").exists())

    def test_machine_mode_never_prompts_for_secret_input(self):
        with tempfile.TemporaryDirectory() as root:
            payload, code, _stderr = _run(
                "--data-root",
                str(Path(root) / "data"),
                "--machine",
                "secrets",
                "init",
            )

            self.assertEqual(code, 1)
            self.assertEqual(payload["error"]["code"], "interaction_required")
            self.assertIn("--new-passphrase-fd", payload["error"]["hint"])

    def test_health_and_next_actions_are_direct_cli_reads(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            initialized, code, _stderr = _run(
                "--data-root", str(data_root), "--machine", "init"
            )
            self.assertEqual(code, 0, initialized)

            health, code, _stderr = _run(
                "--data-root", str(data_root), "--machine", "health"
            )
            self.assertEqual(code, 0, health)
            self.assertEqual(health["kind"], "health")
            self.assertIn("reports", health["data"])

            actions, code, _stderr = _run(
                "--data-root", str(data_root), "--machine", "next-actions"
            )
            self.assertEqual(code, 0, actions)
            self.assertEqual(actions["kind"], "next-actions")
            self.assertIn("suggestions", actions["data"])

    def test_paginated_payloads_receive_standard_page_metadata(self):
        envelope = build_envelope(
            "items.list",
            {"items": [], "next_cursor": "opaque", "has_more": True},
        )
        self.assertEqual(
            envelope["data"]["page"],
            {"next_cursor": "opaque", "has_more": True},
        )
        self.assertEqual(envelope["data"]["next_cursor"], "opaque")

    def test_backup_subcommands_have_exact_envelope_kinds(self):
        parser = build_parser()
        exported = parser.parse_args(
            ["backup", "export", "--file", "/tmp/example.kassiber", "--recipient", "age1x"]
        )
        imported = parser.parse_args(
            ["backup", "import", "/tmp/example.kassiber", "--identity-file", "/tmp/id"]
        )
        self.assertEqual(derive_kind(exported), "backup.export")
        self.assertEqual(derive_kind(imported), "backup.import")


if __name__ == "__main__":
    unittest.main()
