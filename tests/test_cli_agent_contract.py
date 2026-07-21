from __future__ import annotations

import argparse
from contextlib import redirect_stderr, redirect_stdout
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from kassiber.cli.command_registry import (
    _COMMAND_PATH_ONLY_SUBCOMMAND_ATTRS,
    command_path,
)
from kassiber.cli.main import build_parser, dispatch, main
from kassiber.envelope import _KIND_SUBCOMMAND_ATTRS, build_envelope, derive_kind


def _run(*args: str):
    stdout = io.StringIO()
    stderr = io.StringIO()
    with redirect_stdout(stdout), redirect_stderr(stderr):
        code = main(list(args))
    return json.loads(stdout.getvalue()), code, stderr.getvalue()


class CliAgentContractTests(unittest.TestCase):
    def test_every_nested_subparser_contributes_to_routing_identity(self):
        destinations: set[str] = set()

        def visit(parser: argparse.ArgumentParser) -> None:
            for action in parser._actions:
                if not isinstance(action, argparse._SubParsersAction):
                    continue
                destinations.add(action.dest)
                for child in action.choices.values():
                    visit(child)

        visit(build_parser())
        destinations.discard("command")
        self.assertEqual(
            destinations.difference(_KIND_SUBCOMMAND_ATTRS),
            set(_COMMAND_PATH_ONLY_SUBCOMMAND_ATTRS),
        )

        parser = build_parser()
        examples = {
            ("projects", "list"): "projects.list",
            (
                "metadata", "records", "tag", "add",
                "--transaction", "tx", "--tag", "reviewed",
            ): "metadata.records.tag.add",
            (
                "metadata", "records", "excluded", "clear",
                "--transaction", "tx",
            ): "metadata.records.excluded.clear",
            ("reports", "filed-snapshots", "list"): "reports.filed-snapshots.list",
        }
        for argv, expected in examples.items():
            with self.subTest(argv=argv):
                self.assertEqual(command_path(parser.parse_args(argv)), expected)

    def test_exchange_sync_dispatch_uses_runtime_config(self):
        args = build_parser().parse_args(
            ["wallets", "sync-kraken", "--backend", "kraken-live"]
        )
        runtime_config = object()
        args.runtime_config = runtime_config
        with (
            patch("kassiber.cli.main.import_exchange_api", return_value={"ok": True}) as sync,
            patch("kassiber.cli.main.emit", return_value=0),
        ):
            dispatch(object(), args)

        self.assertIs(sync.call_args.args[1], runtime_config)
        self.assertEqual(sync.call_args.kwargs["expected_backend_kind"], "kraken")

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

    def test_status_explains_that_a_fresh_data_root_needs_initialization(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            missing, code, _stderr = _run(
                "--data-root", str(data_root), "--machine", "status"
            )

            self.assertEqual(code, 1)
            self.assertEqual(missing["kind"], "error")
            self.assertEqual(missing["error"]["code"], "not_initialized")
            self.assertEqual(missing["error"]["message"], "Kassiber is not initialized yet")
            self.assertIn("kassiber init", missing["error"]["hint"])

            initialized, code, _stderr = _run(
                "--data-root", str(data_root), "--machine", "init"
            )
            self.assertEqual(code, 0, initialized)

            status, code, _stderr = _run(
                "--data-root", str(data_root), "--machine", "status"
            )
            self.assertEqual(code, 0, status)
            self.assertEqual(status["kind"], "status")

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
