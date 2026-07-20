from __future__ import annotations

import unittest

from kassiber.cli.command_registry import describe_command_catalog
from kassiber.cli.main import build_parser
from kassiber.command_capabilities import (
    CLI_CAPABILITIES,
    DAEMON_CAPABILITIES,
    Capability,
    capability_allows,
    cli_capability,
    daemon_capability,
)
from kassiber.daemon import SUPPORTED_KINDS


class CommandCapabilityRegistryTest(unittest.TestCase):
    def test_every_cli_leaf_has_one_exact_declaration(self):
        catalog = describe_command_catalog(build_parser())
        catalog_paths = {item["kind"] for item in catalog["commands"]}
        self.assertEqual(set(CLI_CAPABILITIES), catalog_paths)
        for item in catalog["commands"]:
            self.assertEqual(item["capability"], cli_capability(item["kind"]).value)

    def test_every_supported_daemon_kind_has_one_exact_declaration(self):
        self.assertEqual(set(DAEMON_CAPABILITIES), set(SUPPORTED_KINDS))
        for kind in SUPPORTED_KINDS:
            self.assertIsInstance(daemon_capability(kind), Capability)

    def test_unknown_operations_fail_closed(self):
        with self.assertRaises(KeyError):
            cli_capability("reports.future-unreviewed-export")
        with self.assertRaises(KeyError):
            daemon_capability("ui.future.unreviewed")

    def test_grants_are_cumulative_but_admin_is_never_leased(self):
        self.assertTrue(
            capability_allows(Capability.ACCOUNTING_DECISIONS, Capability.READ)
        )
        self.assertTrue(
            capability_allows(Capability.ACCOUNTING_DECISIONS, Capability.OPERATOR)
        )
        self.assertFalse(
            capability_allows(Capability.ACCOUNTING_DECISIONS, Capability.ADMIN)
        )

    def test_semantically_mutating_daemon_kinds_are_not_read_or_note_only(self):
        self.assertIs(
            daemon_capability("ui.transactions.metadata.update"),
            Capability.ACCOUNTING_DECISIONS,
        )
        self.assertIs(daemon_capability("ui.rates.latest"), Capability.OPERATOR)


if __name__ == "__main__":
    unittest.main()
