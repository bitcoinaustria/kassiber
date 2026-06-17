"""Catch drift between the desktop connection catalog and the daemon.

The TS catalog at ``ui-tauri/src/lib/connectionCatalog.tsx`` carries
presentation metadata (icons, copy, ordering) for the Add Connection
modal. Each ``status: "ready"`` entry references a ``walletKind`` and
sometimes a ``sourceFormat``. If those drift away from the daemon's
authoritative ``WALLET_KINDS`` / ``_UI_WALLET_SOURCE_FORMATS`` lists,
the modal will surface a connection the daemon will reject at create
time. This test parses the catalog and verifies alignment.
"""

from __future__ import annotations

import inspect
import re
import unittest
from pathlib import Path

from kassiber.core.wallets import WALLET_KINDS
from kassiber.daemon import (
    SUPPORTED_KINDS,
    _UI_WALLET_SOURCE_FORMATS,
    _create_btcpay_connection_payload,
)


_CATALOG_PATH = (
    Path(__file__).resolve().parent.parent
    / "ui-tauri"
    / "src"
    / "lib"
    / "connectionCatalog.tsx"
)
_TAURI_LIB_PATH = (
    Path(__file__).resolve().parent.parent
    / "ui-tauri"
    / "src-tauri"
    / "src"
    / "lib.rs"
)
_VITE_CONFIG_PATH = (
    Path(__file__).resolve().parent.parent / "ui-tauri" / "vite.config.ts"
)
_DESKTOP_MUTATION_KINDS = (
    "ui.backends.options",
    "ui.backends.public_defaults",
    "ui.backends.electrum.test",
    "ui.transactions.metadata.update",
    "ui.onboarding.complete",
    "ui.wallets.create",
    "ui.wallets.preview_descriptor",
    "ui.profiles.create",
    "ui.profiles.rename",
    "ui.profiles.switch",
    "ui.profiles.reset_data",
    "ui.workspace.create",
    "ui.workspace.rename",
    "ui.connections.sources",
    "ui.connections.btcpay.create",
    "ui.connections.btcpay.discover",
    "ui.connections.btcpay.test",
    "ui.metadata.bip329.import",
    "ui.rates.kraken_csv.import",
    "ui.rates.latest",
    "ui.rates.rebuild",
    "ui.wallets.update",
    "ui.wallets.delete",
    "ai.providers.set_api_key",
    "ai.providers.move_api_key",
)
_DESKTOP_RATE_READ_KINDS = (
    "ui.rates.summary",
    "ui.rates.coverage",
)
_DESKTOP_SWAP_MATCHING_KINDS = (
    "ui.transfers.suggest",
    "ui.transfers.list",
    "ui.transfers.payouts.list",
    "ui.transfers.payouts.create",
    "ui.transfers.payouts.delete",
    "ui.transfers.pair",
    "ui.transfers.unpair",
    "ui.transfers.bulk_pair",
    "ui.transfers.dismiss",
    "ui.transfers.rules.list",
    "ui.transfers.rules.create",
    "ui.transfers.rules.delete",
    "ui.transfers.rules.set_enabled",
    "ui.transfers.rules.apply",
    "ui.saved_views.list",
    "ui.saved_views.create",
    "ui.saved_views.delete",
)
_DESKTOP_COMMERCIAL_RECONCILIATION_KINDS = (
    "ui.btcpay.provenance.sync",
    "ui.btcpay.provenance.list",
    "ui.btcpay.provenance.suggest",
    "ui.btcpay.provenance.links",
    "ui.btcpay.provenance.review",
    "ui.transactions.commercial_context",
    "ui.documents.list",
    "ui.documents.create",
    "ui.documents.attach",
)


def _split_entries(text: str) -> list[str]:
    """Cheap entry split that respects nested braces and matching quotes."""
    entries: list[str] = []
    depth = 0
    in_string: str | None = None
    escape = False
    start = -1
    for index, char in enumerate(text):
        if in_string:
            if escape:
                escape = False
            elif char == "\\":
                escape = True
            elif char == in_string:
                in_string = None
            continue
        if char in ('"', "'", "`"):
            in_string = char
            continue
        if char == "{":
            if depth == 0:
                start = index
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0 and start >= 0:
                entries.append(text[start : index + 1])
                start = -1
    return entries


def _extract_field(entry: str, field: str) -> str | None:
    match = re.search(rf'\b{re.escape(field)}\s*:\s*"([^"]+)"', entry)
    return match.group(1) if match else None


class ConnectionCatalogDriftTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.catalog_text = _CATALOG_PATH.read_text(encoding="utf-8")
        cls.tauri_lib_text = _TAURI_LIB_PATH.read_text(encoding="utf-8")
        cls.vite_config_text = _VITE_CONFIG_PATH.read_text(encoding="utf-8")

    def test_catalog_file_exists(self):
        self.assertTrue(
            _CATALOG_PATH.exists(),
            f"Connection catalog missing at {_CATALOG_PATH}",
        )

    def test_btcpay_create_uses_a_known_wallet_kind(self):
        """The BTCPay setup path hard-codes its wallet kind in
        ``_create_btcpay_connection_payload`` rather than reading it
        from the catalog. Make sure that hard-coded value still
        resolves to a real wallet kind so the catalog's BTCPay entry
        keeps working even after refactors there.
        """
        source = inspect.getsource(_create_btcpay_connection_payload)
        match = re.search(
            r"core_wallets\.create_wallet\([^)]*?(?:wallet_label|label),\s*\"(?P<kind>[^\"]+)\"",
            source,
            re.DOTALL,
        )
        self.assertIsNotNone(
            match,
            "could not locate hard-coded wallet kind in _create_btcpay_connection_payload",
        )
        self.assertIn(
            match.group("kind"),
            WALLET_KINDS,
            "BTCPay setup hard-codes a wallet kind that WALLET_KINDS no longer contains",
        )

    def _rust_allowlist(self) -> set[str]:
        match = re.search(
            r"ALLOWED_DAEMON_KINDS[^=]*=\s*&\[(?P<body>.*?)\];",
            self.tauri_lib_text,
            re.DOTALL,
        )
        self.assertIsNotNone(match, "could not find Tauri daemon allowlist")
        return set(re.findall(r'"([^"]+)"', match.group("body")))

    def _vite_allowlist(self) -> set[str]:
        match = re.search(
            r"ALLOWED_BRIDGE_KINDS\s*=\s*new Set\(\[(?P<body>.*?)\]\);",
            self.vite_config_text,
            re.DOTALL,
        )
        self.assertIsNotNone(match, "could not find Vite bridge allowlist")
        return set(re.findall(r'"([^"]+)"', match.group("body")))

    def test_desktop_mutation_kinds_are_allowed_by_desktop_boundaries(self):
        rust_kinds = self._rust_allowlist()
        vite_kinds = self._vite_allowlist()
        for kind in _DESKTOP_MUTATION_KINDS:
            self.assertIn(kind, rust_kinds, f"{kind} is missing from Tauri daemon allowlist")
            self.assertIn(kind, vite_kinds, f"{kind} is missing from Vite bridge allowlist")

    def test_rate_read_kinds_are_allowed_by_desktop_boundaries(self):
        rust_kinds = self._rust_allowlist()
        vite_kinds = self._vite_allowlist()
        for kind in _DESKTOP_RATE_READ_KINDS:
            self.assertIn(kind, rust_kinds, f"{kind} is missing from Tauri daemon allowlist")
            self.assertIn(kind, vite_kinds, f"{kind} is missing from Vite bridge allowlist")

    def test_swap_matching_kinds_are_allowed_by_desktop_boundaries(self):
        rust_kinds = self._rust_allowlist()
        vite_kinds = self._vite_allowlist()
        for kind in _DESKTOP_SWAP_MATCHING_KINDS:
            self.assertIn(kind, rust_kinds, f"{kind} is missing from Tauri daemon allowlist")
            self.assertIn(kind, vite_kinds, f"{kind} is missing from Vite bridge allowlist")

    def test_commercial_reconciliation_kinds_are_allowed_by_desktop_boundaries(self):
        rust_kinds = self._rust_allowlist()
        vite_kinds = self._vite_allowlist()
        for kind in _DESKTOP_COMMERCIAL_RECONCILIATION_KINDS:
            self.assertIn(kind, rust_kinds, f"{kind} is missing from Tauri daemon allowlist")
            self.assertIn(kind, vite_kinds, f"{kind} is missing from Vite bridge allowlist")

    def test_desktop_allowlists_are_subset_of_daemon_supported_kinds(self):
        """The Tauri command boundary and the dev Vite bridge may only
        forward kinds the Python daemon actually handles. Drift here
        would surface as runtime ``unknown kind`` errors against whatever
        shell forwarded a kind the daemon dropped. The reverse direction
        (daemon-only kinds) is intentional — AI read tools, daemon
        lifecycle commands, and reveal kinds stay off the desktop surface.
        """

        daemon_kinds = set(SUPPORTED_KINDS)
        rust_kinds = self._rust_allowlist()
        vite_kinds = self._vite_allowlist()
        self.assertTrue(
            rust_kinds.issubset(daemon_kinds),
            "Tauri allowlist contains kinds the daemon does not support: "
            f"{sorted(rust_kinds - daemon_kinds)}",
        )
        self.assertTrue(
            vite_kinds.issubset(daemon_kinds),
            "Vite-bridge allowlist contains kinds the daemon does not support: "
            f"{sorted(vite_kinds - daemon_kinds)}",
        )

    def test_tauri_and_vite_bridge_allowlists_match(self):
        """`pnpm dev:bridge` is the browser-loopback equivalent of the Tauri
        command boundary, so any daemon kind allowed by one should be allowed
        by the other. Drift here leaves browser dev mode unable to exercise
        whole feature areas (e.g. source-funds) against the real daemon while
        the packaged desktop shell works. If a future kind is intentionally
        Tauri-only, allowlist it explicitly here rather than weakening this
        assertion.
        """

        rust_kinds = self._rust_allowlist()
        vite_kinds = self._vite_allowlist()
        self.assertEqual(
            rust_kinds,
            vite_kinds,
            "Tauri and Vite-bridge daemon kind allowlists have drifted. "
            "Update both lists together (see ui-tauri/src-tauri/src/lib.rs "
            "and ui-tauri/vite.config.ts).",
        )

    def test_logs_snapshot_kind_is_allowed_by_desktop_boundaries(self):
        """The RAM-only log bridge polls ``ui.logs.snapshot`` from both desktop
        shells. It must stay in the daemon's supported kinds and both forwarding
        allowlists or the Logs screen silently loses the daemon/supervisor layers
        (``kind_not_allowed`` in packaged mode, HTTP 403 in dev-browser mode).
        """

        self.assertIn("ui.logs.snapshot", set(SUPPORTED_KINDS))
        self.assertIn("ui.logs.snapshot", self._rust_allowlist())
        self.assertIn("ui.logs.snapshot", self._vite_allowlist())

    def test_refresh_kinds_are_stream_capable_in_desktop_boundaries(self):
        tauri_streaming = re.search(
            r"STREAMING_DAEMON_KINDS[^=]*=\s*&\[(?P<body>.*?)\];",
            self.tauri_lib_text,
            re.DOTALL,
        )
        vite_streaming = re.search(
            r"STREAM_CAPABLE_BRIDGE_KINDS[^=]*=\s*new Set\(\[(?P<body>.*?)\]\);",
            self.vite_config_text,
            re.DOTALL,
        )
        self.assertIsNotNone(tauri_streaming, "could not find Tauri streaming kind list")
        self.assertIsNotNone(vite_streaming, "could not find Vite stream-capable kind list")
        for kind in ("ui.wallets.sync", "ui.freshness.run"):
            self.assertIn(
                f'"{kind}"',
                tauri_streaming.group("body"),
                f"{kind} emits progress records and must be marked streaming",
            )
            self.assertIn(
                f'"{kind}"',
                vite_streaming.group("body"),
                f"{kind} emits progress records and must be stream-capable in dev bridge",
            )

    def test_ready_entries_reference_known_wallet_kinds(self):
        # We only want the array literal that lives behind CONNECTION_SOURCES.
        match = re.search(
            r"CONNECTION_SOURCES[^=]*=\s*\[(?P<body>.*?)\];",
            self.catalog_text,
            re.DOTALL,
        )
        self.assertIsNotNone(match, "could not find CONNECTION_SOURCES literal")
        entries = _split_entries(match.group("body"))
        self.assertGreater(len(entries), 0, "no source entries parsed")

        for entry in entries:
            status = _extract_field(entry, "status")
            if status != "ready":
                continue
            wallet_kind = _extract_field(entry, "walletKind")
            source_format = _extract_field(entry, "sourceFormat")
            setup_kind = _extract_field(entry, "setupKind")
            entry_id = _extract_field(entry, "id") or "<unknown>"
            if setup_kind in (None, "backend-settings", "bip329", "btcpay"):
                # Backend-only / label-import / BTCPay entries do not declare a
                # walletKind; the daemon picks the wallet kind for them.
                continue
            self.assertIsNotNone(
                wallet_kind,
                f"ready catalog entry '{entry_id}' has no walletKind",
            )
            self.assertIn(
                wallet_kind,
                WALLET_KINDS,
                f"catalog entry '{entry_id}' references unknown walletKind '{wallet_kind}'",
            )
            if source_format is not None:
                self.assertIn(
                    source_format,
                    _UI_WALLET_SOURCE_FORMATS,
                    f"catalog entry '{entry_id}' references unknown sourceFormat '{source_format}'",
                )


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
