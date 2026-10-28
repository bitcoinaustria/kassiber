"""Contract for `kassiber mcp`: the dual-era stdio server external agents use."""

from __future__ import annotations

import contextlib
import io
import json
import os
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from kassiber.ai.tools import TOOL_CATALOG, get_tool
from kassiber.cli.command_registry import describe_command_catalog
from kassiber.cli.main import build_parser, main
from kassiber.command_capabilities import Capability, cli_capability
from kassiber.db import get_setting, open_db, pinned_context_scope, set_setting
from kassiber.errors import AppError
from kassiber.mcp import cli as mcp_cli
from kassiber.mcp import tools as mcp_tools
from kassiber.mcp.protocol import (
    INVALID_PARAMS,
    LEGACY_PROTOCOL_VERSIONS,
    META_CLIENT_CAPABILITIES,
    META_PROTOCOL_VERSION,
    META_SERVER_INFO,
    METHOD_NOT_FOUND,
    UNSUPPORTED_PROTOCOL_VERSION,
    McpServer,
    ToolOutcome,
    serve,
)
from kassiber.operator.build import build_identity
from kassiber.operator.cli import route_brokered_command
from tests.integration.env import no_egress_guard


_ACCESS_DIR: tempfile.TemporaryDirectory | None = None
_PREVIOUS_ACCESS_FILE: str | None = None


def setUpModule() -> None:
    # External agents are off by default; these tests exercise the server
    # with the user's opt-in recorded in a disposable preference file.
    global _ACCESS_DIR, _PREVIOUS_ACCESS_FILE
    from kassiber.agent_access import PATH_ENV, set_agent_access

    _ACCESS_DIR = tempfile.TemporaryDirectory()
    _PREVIOUS_ACCESS_FILE = os.environ.get(PATH_ENV)
    os.environ[PATH_ENV] = str(Path(_ACCESS_DIR.name) / "agent-access.json")
    set_agent_access(mcp_enabled=True)


def tearDownModule() -> None:
    from kassiber.agent_access import PATH_ENV

    if _PREVIOUS_ACCESS_FILE is None:
        os.environ.pop(PATH_ENV, None)
    else:
        os.environ[PATH_ENV] = _PREVIOUS_ACCESS_FILE
    if _ACCESS_DIR is not None:
        _ACCESS_DIR.cleanup()


MODERN_META = {
    META_PROTOCOL_VERSION: "2026-07-28",
    META_CLIENT_CAPABILITIES: {},
}


class _FakeProvider:
    instructions = "fake instructions"

    def __init__(self) -> None:
        self.calls: list[tuple[str, dict]] = []
        self.clients: list[str | None] = []
        self.release = threading.Event()
        self.release.set()

    def tool_definitions(self):
        return [
            {
                "name": "echo",
                "title": "Echo",
                "description": "Echo arguments",
                "inputSchema": {"type": "object", "additionalProperties": True},
                "annotations": {"title": "Echo", "readOnlyHint": True},
            }
        ]

    def has_tool(self, name):
        return name == "echo"

    def call_tool(self, name, arguments, cancelled, client_name=None):
        self.release.wait(5)
        self.calls.append((name, arguments))
        self.clients.append(client_name)
        return ToolOutcome(structured={"echo": arguments}, text=json.dumps(arguments))


def _request(request_id, method, params=None):
    message = {"jsonrpc": "2.0", "id": request_id, "method": method}
    if params is not None:
        message["params"] = params
    return message


def _serve_lines(server: McpServer, messages: list[dict]) -> list[dict]:
    reader = io.BytesIO("".join(json.dumps(m) + "\n" for m in messages).encode())
    writer = io.BytesIO()
    serve(server, reader, writer)
    return [json.loads(line) for line in writer.getvalue().decode().splitlines()]


class ProtocolEraTests(unittest.TestCase):
    def setUp(self) -> None:
        self.provider = _FakeProvider()
        self.server = McpServer(self.provider, name="kassiber", version="test")

    def test_modern_discover_is_complete_cacheable_and_identified(self):
        response = self.server.handle(_request(1, "server/discover", {"_meta": MODERN_META}))
        result = response["result"]
        self.assertEqual(result["resultType"], "complete")
        self.assertEqual(result["supportedVersions"], ["2026-07-28"])
        self.assertEqual(result["capabilities"], {"tools": {}})
        self.assertEqual(result["cacheScope"], "public")
        self.assertGreater(result["ttlMs"], 0)
        self.assertEqual(result["_meta"][META_SERVER_INFO]["name"], "kassiber")
        self.assertEqual(result["instructions"], "fake instructions")

    def test_modern_requests_need_no_handshake_and_never_set_legacy_state(self):
        listed = self.server.handle(_request(1, "tools/list", {"_meta": MODERN_META}))
        self.assertEqual(listed["result"]["tools"][0]["name"], "echo")
        self.assertEqual(listed["result"]["resultType"], "complete")
        # A later request without metadata is still refused: modern requests
        # must not establish a session.
        refused = self.server.handle(_request(2, "tools/list", {}))
        self.assertEqual(refused["error"]["code"], INVALID_PARAMS)

    def test_unsupported_modern_version_lists_supported_versions(self):
        meta = dict(MODERN_META)
        meta[META_PROTOCOL_VERSION] = "1900-01-01"
        response = self.server.handle(_request(1, "tools/list", {"_meta": meta}))
        self.assertEqual(response["error"]["code"], UNSUPPORTED_PROTOCOL_VERSION)
        self.assertEqual(
            response["error"]["data"],
            {"supported": ["2026-07-28"], "requested": "1900-01-01"},
        )

    def test_modern_request_requires_client_capabilities(self):
        response = self.server.handle(
            _request(1, "tools/list", {"_meta": {META_PROTOCOL_VERSION: "2026-07-28"}})
        )
        self.assertEqual(response["error"]["code"], INVALID_PARAMS)

    def test_legacy_initialize_echoes_supported_and_offers_newest_otherwise(self):
        for requested, expected in (
            ("2025-06-18", "2025-06-18"),
            ("2025-11-25", "2025-11-25"),
            ("2024-01-01", LEGACY_PROTOCOL_VERSIONS[0]),
        ):
            with self.subTest(requested=requested):
                server = McpServer(self.provider, name="kassiber", version="test")
                response = server.handle(
                    _request(1, "initialize", {"protocolVersion": requested, "capabilities": {}})
                )
                self.assertEqual(response["result"]["protocolVersion"], expected)
                self.assertEqual(response["result"]["capabilities"], {"tools": {}})
                listed = server.handle(_request(2, "tools/list"))
                self.assertNotIn("resultType", listed["result"])
                self.assertNotIn("ttlMs", listed["result"])

    def test_batch_era_revision_is_not_claimed(self):
        # 2025-03-26 requires JSON-RPC batches, which this server rejects.
        response = self.server.handle(_request(1, "initialize", {"protocolVersion": "2025-03-26"}))
        self.assertEqual(response["result"]["protocolVersion"], "2025-11-25")
        self.assertNotIn("2025-03-26", LEGACY_PROTOCOL_VERSIONS)

    def test_legacy_ping_works_before_initialize(self):
        self.assertEqual(self.server.handle(_request(1, "ping"))["result"], {})

    def test_unknown_tool_and_method_are_protocol_errors(self):
        unknown_tool = self.server.handle(
            _request(1, "tools/call", {"_meta": MODERN_META, "name": "shell", "arguments": {}})
        )
        self.assertEqual(unknown_tool["error"]["code"], INVALID_PARAMS)
        unknown_method = self.server.handle(
            _request(2, "resources/list", {"_meta": MODERN_META})
        )
        self.assertEqual(unknown_method["error"]["code"], METHOD_NOT_FOUND)

    def test_modern_call_returns_structured_and_text_content(self):
        result = self.server.handle(
            _request(1, "tools/call", {"_meta": MODERN_META, "name": "echo", "arguments": {"x": 2}})
        )["result"]
        self.assertEqual(result["structuredContent"], {"echo": {"x": 2}})
        self.assertEqual(json.loads(result["content"][0]["text"]), {"x": 2})
        self.assertNotIn("isError", result)

    def test_the_host_reported_client_name_reaches_the_provider(self):
        # Display-only attribution for writes: the legacy handshake's
        # clientInfo, or a modern request's clientInfo metadata.
        self.server.handle(
            _request(
                1,
                "initialize",
                {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "claude-code"}},
            )
        )
        self.server.handle(_request(2, "tools/call", {"name": "echo", "arguments": {}}))
        meta = {**MODERN_META, "io.modelcontextprotocol/clientInfo": {"name": "codex"}}
        self.server.handle(_request(3, "tools/call", {"_meta": meta, "name": "echo", "arguments": {}}))
        self.server.handle(_request(4, "tools/call", {"_meta": MODERN_META, "name": "echo", "arguments": {}}))
        self.assertEqual(self.provider.clients, ["claude-code", "codex", None])


class StdioLoopTests(unittest.TestCase):
    def test_every_accepted_call_is_answered_before_exit_on_eof(self):
        provider = _FakeProvider()
        server = McpServer(provider, name="kassiber", version="test")
        responses = _serve_lines(
            server,
            [
                _request(1, "tools/call", {"_meta": MODERN_META, "name": "echo", "arguments": {"n": 1}}),
                _request(2, "tools/call", {"_meta": MODERN_META, "name": "echo", "arguments": {"n": 2}}),
            ],
        )
        self.assertEqual(sorted(r["id"] for r in responses), [1, 2])

    def test_cancelled_call_gets_no_reply(self):
        provider = _FakeProvider()
        provider.release.clear()
        server = McpServer(provider, name="kassiber", version="test")
        messages = [
            _request(1, "tools/call", {"_meta": MODERN_META, "name": "echo", "arguments": {}}),
            _request(2, "tools/call", {"_meta": MODERN_META, "name": "echo", "arguments": {}}),
            {"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 2}},
        ]
        reader = io.BytesIO("".join(json.dumps(m) + "\n" for m in messages).encode())
        writer = io.BytesIO()
        timer = threading.Timer(0.3, provider.release.set)
        timer.start()
        try:
            serve(server, reader, writer)
        finally:
            timer.cancel()
        ids = [json.loads(line)["id"] for line in writer.getvalue().decode().splitlines()]
        self.assertEqual(ids, [1])

    def test_malformed_lines_get_errors_and_the_loop_continues(self):
        server = McpServer(_FakeProvider(), name="kassiber", version="test")
        reader = io.BytesIO(
            b"not json\n"
            + (json.dumps(_request(1, "server/discover", {"_meta": MODERN_META})) + "\n").encode()
        )
        writer = io.BytesIO()
        serve(server, reader, writer)
        lines = [json.loads(line) for line in writer.getvalue().decode().splitlines()]
        self.assertEqual(lines[0]["error"]["code"], -32700)
        self.assertEqual(lines[1]["id"], 1)

    def test_subscription_is_acknowledged_with_an_empty_filter(self):
        server = McpServer(_FakeProvider(), name="kassiber", version="test")
        responses = _serve_lines(
            server,
            [
                _request(
                    9,
                    "subscriptions/listen",
                    {"_meta": MODERN_META, "notifications": {"toolsListChanged": True}},
                )
            ],
        )
        self.assertEqual(responses[0]["method"], "notifications/subscriptions/acknowledged")
        self.assertEqual(responses[0]["params"]["notifications"], {})
        # Closing stdin tears the stream down with a final result.
        self.assertEqual(responses[-1]["id"], 9)


class SubscriptionLimitTests(unittest.TestCase):
    def test_a_malformed_subscription_envelope_gets_an_error_not_a_stream(self):
        server = McpServer(_FakeProvider(), name="kassiber", version="test")
        bad = _request(7, "subscriptions/listen", {"_meta": MODERN_META, "notifications": {}})
        bad["jsonrpc"] = "invalid"
        responses = _serve_lines(server, [bad])
        self.assertTrue(all("method" not in item for item in responses))
        self.assertIn("error", responses[0])

    def test_open_subscriptions_are_capped(self):
        from kassiber.mcp.protocol import MAX_SUBSCRIPTIONS

        server = McpServer(_FakeProvider(), name="kassiber", version="test")
        requests = [
            _request(100 + index, "subscriptions/listen", {"_meta": MODERN_META, "notifications": {}})
            for index in range(MAX_SUBSCRIPTIONS + 1)
        ]
        responses = _serve_lines(server, requests)
        refused = [item for item in responses if "error" in item]
        self.assertEqual([item["id"] for item in refused], [100 + MAX_SUBSCRIPTIONS])
        self.assertEqual(refused[0]["error"]["data"], {"limit": MAX_SUBSCRIPTIONS})


class CatalogProjectionTests(unittest.TestCase):
    """What each External agents grant lists, classified by the broker registry."""

    WIDEST = {"grant": "accounting_decisions", "agent_access": "full"}
    LEGACY = {"grant": "read", "agent_access": "restricted"}

    def test_the_read_grant_lists_only_local_read_only_tools(self):
        from kassiber import daemon

        for access in ("restricted", "full"):
            for tool in mcp_tools.available_tools(grant="read", agent_access=access):
                with self.subTest(tool=tool.name, access=access):
                    entry = tool.entry
                    self.assertIs(get_tool(entry.name), entry)
                    self.assertEqual(entry.kind_class, "read_only")
                    self.assertFalse(entry.egresses)
                    self.assertFalse(tool.egress)
                    self.assertFalse(entry.requires_consent)
                    self.assertIs(tool.required, Capability.READ)
                    if access == "restricted":
                        self.assertNotIn(entry.daemon_kind, daemon._LOCAL_CUSTODY_READ_DAEMON_KINDS)
                        self.assertFalse(entry.provider_name.startswith("ui_chain_analysis_sources_"))
                        self.assertNotIn(entry.provider_name, daemon._ON_DEVICE_ONLY_TOOL_NAMES)

    def test_grants_are_cumulative_and_restricted_access_hides_full_only_tools(self):
        previous: set[str] = set()
        for grant in ("read", "operator", "accounting_decisions"):
            restricted = {t.name for t in mcp_tools.available_tools(grant=grant, agent_access="restricted")}
            full = {t.name for t in mcp_tools.available_tools(grant=grant, agent_access="full")}
            with self.subTest(grant=grant):
                self.assertLess(previous, restricted)
                self.assertLess(restricted, full)
                self.assertTrue(all(mcp_tools.entry_for(name).full_access_only for name in full - restricted))
            previous = restricted
        # Today's read surface is kept, and widened to every eligible read.
        self.assertGreater(len(mcp_tools.available_tools(**self.LEGACY)), 46)

    def test_tools_are_classified_by_the_broker_registry(self):
        from kassiber import daemon
        from kassiber.command_capabilities import daemon_capability

        for tool in mcp_tools.mcp_tools():
            entry = tool.entry
            with self.subTest(tool=tool.name):
                self.assertIsNot(tool.required, Capability.ADMIN)
                if tool.egress:
                    self.assertIn(tool.required, {Capability.OPERATOR, Capability.ACCOUNTING_DECISIONS})
                if entry.daemon_kind is None or entry.name in mcp_tools._REGISTRY_ALIASES:
                    continue
                registry = daemon_capability(entry.daemon_kind)
                if entry.kind_class == "read_only" and entry.daemon_kind in daemon._DIRECT_AUTO_JOURNAL_REFRESH_KINDS:
                    # Operator only for the journal refresh that MCP runs at
                    # the operator grant.
                    self.assertIs(registry, Capability.OPERATOR)
                    self.assertIs(tool.required, Capability.READ)
                elif not tool.egress:
                    self.assertIs(tool.required, registry)
        required = {t.name: t.required for t in mcp_tools.mcp_tools()}
        self.assertIs(required["journals_process"], Capability.OPERATOR)
        self.assertIs(required["wallets_sync"], Capability.OPERATOR)
        self.assertIs(required["review_apply"], Capability.ACCOUNTING_DECISIONS)
        self.assertIs(required["custody_review_apply"], Capability.ACCOUNTING_DECISIONS)
        self.assertIs(required["connections_node_snapshot"], Capability.OPERATOR)

    def test_admin_class_chat_only_and_unclassified_tools_are_never_exposed(self):
        from kassiber.command_capabilities import DAEMON_CAPABILITIES

        exposed = {tool.entry.name for tool in mcp_tools.mcp_tools()}
        for entry in TOOL_CATALOG:
            with self.subTest(tool=entry.name):
                if DAEMON_CAPABILITIES.get(entry.daemon_kind or "") is Capability.ADMIN:
                    self.assertNotIn(entry.name, exposed)
                if entry.name in mcp_tools.UNAVAILABLE_TOOL_REASONS:
                    self.assertNotIn(entry.name, exposed)
                if entry.kind_class == "mutating" and entry.name in exposed:
                    self.assertIn(entry.name, mcp_tools.MUTATING_TOOL_HINTS)
        self.assertNotIn("ui.imports.rollback", exposed)
        self.assertIsNone(mcp_tools.entry_for("imports_rollback"))
        self.assertIsNone(mcp_tools.entry_for("wallets_import_attachment_apply"))

    def test_every_mutating_catalog_tool_is_reviewed(self):
        # A new mutating tool stays off MCP until it is classified here (or
        # is admin-class or chat-only), so drift fails closed and loudly.
        from kassiber.command_capabilities import DAEMON_CAPABILITIES

        for entry in TOOL_CATALOG:
            if entry.kind_class != "mutating":
                continue
            with self.subTest(tool=entry.name):
                self.assertTrue(
                    entry.name in mcp_tools.MUTATING_TOOL_HINTS
                    or entry.name in mcp_tools.UNAVAILABLE_TOOL_REASONS
                    or DAEMON_CAPABILITIES.get(entry.daemon_kind or "") is Capability.ADMIN,
                    "classify this mutating tool's MCP hints or exclusion",
                )
        self.assertFalse(set(mcp_tools.MUTATING_TOOL_HINTS) - {e.name for e in TOOL_CATALOG})

    def test_the_report_export_alias_covers_every_daemon_target(self):
        from kassiber import daemon

        schema = get_tool("ui.reports.export").parameters["properties"]
        for report in schema["report"]["enum"]:
            for export_format in schema["format"]["enum"]:
                try:
                    kind, _ = daemon._ai_report_export_target(
                        {
                            "report": report,
                            "format": export_format,
                            "departure_date": "2026-01-01",
                            "destination": "eu_eea",
                        }
                    )
                except AppError:
                    continue
                with self.subTest(report=report, format=export_format):
                    self.assertIn(kind, mcp_tools._REPORT_EXPORT_KINDS)

    def test_definitions_are_portable_across_hosts_and_annotated_truthfully(self):
        definitions = mcp_tools.tool_definitions(**self.WIDEST)
        names = [definition["name"] for definition in definitions]
        self.assertEqual(len(names), len(set(names)))
        for definition in definitions:
            tool = mcp_tools.entry_for(definition["name"])
            with self.subTest(tool=definition["name"]):
                # Hosts prefix server names (mcp__kassiber__) and some model
                # APIs cap tool names at 64 underscore/alnum characters.
                self.assertRegex(definition["name"], r"^[a-z0-9_]+$")
                self.assertLessEqual(len("mcp__kassiber__" + definition["name"]), 64)
                self.assertLessEqual(len(definition["description"]), mcp_tools.MAX_DESCRIPTION_CHARS)
                self.assertEqual(definition["inputSchema"].get("type"), "object")
                annotations = definition["annotations"]
                self.assertEqual(annotations["title"], definition["title"])
                self.assertEqual(annotations["readOnlyHint"], tool.entry.kind_class == "read_only")
                self.assertEqual(annotations["openWorldHint"], tool.egress)
                if annotations["readOnlyHint"]:
                    self.assertFalse(annotations["destructiveHint"])
                    self.assertTrue(annotations["idempotentHint"])
        by_name = {d["name"]: d["annotations"] for d in definitions}
        # A dismissal has an undo (transfers_dismissals_delete); a payout delete has none.
        self.assertFalse(by_name["transfers_dismiss"]["destructiveHint"])
        self.assertTrue(by_name["transfers_payouts_delete"]["destructiveHint"])
        self.assertFalse(by_name["transfers_pair"]["destructiveHint"])
        for egress in (
            "wallets_sync", "rates_latest", "rates_rebuild", "maintenance_run", "maintenance_configure",
            "chain_analysis_acquire_apply", "chain_analysis_sources_authorize", "connections_node_snapshot",
        ):
            self.assertTrue(by_name[egress]["openWorldHint"], egress)
        self.assertFalse(by_name["journals_process"]["openWorldHint"])
        for grant in ("read", "operator", "accounting_decisions"):
            self.assertLessEqual(len(mcp_tools.instructions_for(grant)), mcp_tools.MAX_DESCRIPTION_CHARS)
        self.assertIn("read-only", mcp_tools.instructions_for("read"))
        self.assertNotIn("read-only", mcp_tools.instructions_for("operator"))

    def test_the_read_grant_reaches_no_mutating_or_egressing_tool(self):
        exposed = {tool.entry.name for tool in mcp_tools.available_tools(grant="read", agent_access="full")}
        for entry in TOOL_CATALOG:
            if entry.kind_class == "mutating" or entry.egresses:
                self.assertNotIn(entry.name, exposed)
        for name in ("ui.wallets.sync", "ui.rates.latest", "ui.maintenance.run"):
            self.assertNotIn(name, exposed)

    def test_the_broker_classifies_an_mcp_call_by_its_tool_and_claimed_grant(self):
        from kassiber.operator.service import _parse_argv

        def required(*extra):
            return _parse_argv(["mcp", "call", *extra])[2]

        self.assertIs(required("--tool", "reports_balance_sheet"), Capability.READ)
        self.assertIs(required("--tool", "journals_process"), Capability.OPERATOR)
        self.assertIs(required("--tool", "transactions_metadata_update"), Capability.ACCOUNTING_DECISIONS)
        self.assertIs(required("--tool", "reports_balance_sheet", "--grant", "operator"), Capability.OPERATOR)
        # Unknown and never-exposed tools keep the read baseline; the call
        # itself refuses them.
        self.assertIs(required("--tool", "imports_rollback"), Capability.READ)
        self.assertIs(cli_capability("mcp.call"), Capability.READ)


def _run_cli(*argv: str) -> tuple[dict, int]:
    stdout = io.StringIO()
    with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(io.StringIO()):
        code = main(list(argv))
    return json.loads(stdout.getvalue()), code


class _TwoBookFixture(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.data_root = str(Path(self._tmp.name) / "data")
        for argv in (
            ("init",),
            ("workspaces", "create", "Personal"),
            ("profiles", "create", "Main", "--workspace", "Personal"),
            ("workspaces", "create", "Biz"),
            ("profiles", "create", "Ops", "--workspace", "Biz"),
        ):
            payload, code = _run_cli("--data-root", self.data_root, "--machine", *argv)
            self.assertEqual(code, 0, payload)

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def provider(self, **kwargs):
        return mcp_cli.BookToolProvider(
            data_root=self.data_root,
            project=None,
            env_file=None,
            workspace=kwargs.get("workspace"),
            profile=kwargs.get("profile"),
        )


class BookExecutionTests(_TwoBookFixture):
    def test_pinned_call_reads_its_book_without_touching_the_shared_context(self):
        outcome = self.provider(workspace="Personal", profile="Main").call_tool(
            "overview_snapshot", {}, threading.Event()
        )
        self.assertFalse(outcome.is_error, outcome.structured)
        self.assertEqual(
            outcome.structured["book"],
            {"project": None, "workspace": "Personal", "profile": "Main"},
        )
        conn = open_db(self.data_root)
        try:
            profile_id = get_setting(conn, "context_profile")
            label = conn.execute("SELECT label FROM profiles WHERE id = ?", (profile_id,)).fetchone()["label"]
        finally:
            conn.close()
        self.assertEqual(label, "Ops")

    def test_reads_never_rebuild_journals(self):
        outcome = self.provider(workspace="Personal", profile="Main").call_tool(
            "reports_balance_sheet", {}, threading.Event()
        )
        conn = open_db(self.data_root)
        try:
            processed = conn.execute(
                "SELECT last_processed_at FROM profiles WHERE label = 'Main'"
            ).fetchone()["last_processed_at"]
        finally:
            conn.close()
        self.assertIsNone(processed)
        if outcome.is_error:
            self.assertIn("journals", outcome.structured["error"]["message"])

    def test_tool_execution_runs_with_sqlite_query_only(self):
        from kassiber import daemon

        observed = {}
        original = daemon._execute_read_only_ai_tool

        def spy(call, runtime):
            def probe(conn):
                observed["query_only"] = conn.execute("PRAGMA query_only").fetchone()[0]
                return {}
            daemon._run_on_daemon_main_thread(runtime, probe)
            return original(call, runtime)

        with mock.patch.object(daemon, "_execute_read_only_ai_tool", side_effect=spy):
            outcome = self.provider(workspace="Personal", profile="Main").call_tool(
                "overview_snapshot", {}, threading.Event()
            )
        self.assertFalse(outcome.is_error, outcome.structured)
        self.assertEqual(observed["query_only"], 1)

    def test_results_redact_local_paths(self):
        outcome = self.provider().call_tool("status", {}, threading.Event())
        self.assertFalse(outcome.is_error, outcome.structured)
        self.assertNotIn(self.data_root, outcome.text)

    def test_invalid_arguments_are_tool_errors_not_crashes(self):
        outcome = self.provider().call_tool("transactions_list", {"bogus": 1}, threading.Event())
        self.assertTrue(outcome.is_error)
        self.assertEqual(outcome.structured["error"]["code"], "validation")

    def test_unknown_book_is_a_tool_error(self):
        outcome = self.provider(workspace="Nope", profile="Main").call_tool(
            "status", {}, threading.Event()
        )
        self.assertTrue(outcome.is_error)
        self.assertEqual(outcome.structured["error"]["code"], "not_found")

    def test_launch_listing_and_local_reads_open_no_socket_even_loopback(self):
        server = McpServer(self.provider(), name="kassiber", version="test")
        with no_egress_guard(enabled=True, allow_loopback=False):
            responses = _serve_lines(
                server,
                [
                    _request(1, "server/discover", {"_meta": MODERN_META}),
                    _request(2, "tools/list", {"_meta": MODERN_META}),
                    _request(
                        3,
                        "tools/call",
                        {"_meta": MODERN_META, "name": "wallets_list", "arguments": {}},
                    ),
                ],
            )
        self.assertEqual(sorted(r["id"] for r in responses), [1, 2, 3])
        self.assertTrue(all("error" not in r for r in responses), responses)

    def test_mcp_call_cli_emits_the_same_result(self):
        payload, code = _run_cli(
            "--data-root", self.data_root, "--machine",
            "mcp", "call", "--tool", "overview_snapshot",
            "--workspace", "Personal", "--profile", "Main",
        )
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload["kind"], "mcp.call")
        self.assertEqual(payload["data"]["book"]["profile"], "Main")
        failed, code = _run_cli(
            "--data-root", self.data_root, "--machine", "mcp", "call", "--tool", "shell"
        )
        self.assertEqual(code, 1)
        self.assertEqual(failed["error"]["code"], "unknown_tool")


class MissingBookTests(unittest.TestCase):
    def test_a_fresh_machine_gets_no_default_project_from_an_agent_read(self):
        with tempfile.TemporaryDirectory() as state:
            catalog = Path(state) / "projects.json"
            with mock.patch("kassiber.projects.catalog_path", return_value=catalog), mock.patch(
                "kassiber.projects.ensure_default_project",
                side_effect=AssertionError("must not create a project"),
            ):
                provider = mcp_cli.BookToolProvider(
                    data_root=None, project=None, env_file=None, workspace=None, profile=None
                )
                outcome = provider.call_tool("status", {}, threading.Event())
            self.assertEqual(outcome.structured["error"]["code"], "not_initialized")
            self.assertFalse(catalog.exists())

    def test_mcp_call_cli_never_creates_a_missing_book(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            payload, code = _run_cli(
                "--data-root", str(data_root), "--machine", "mcp", "call", "--tool", "status"
            )
            self.assertEqual(code, 1)
            self.assertEqual(payload["error"]["code"], "not_initialized")
            self.assertFalse((data_root / "kassiber.sqlite3").exists())

    def test_half_pins_are_refused(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            _run_cli("--data-root", str(data_root), "--machine", "init")
            payload, code = _run_cli(
                "--data-root", str(data_root), "--machine",
                "mcp", "call", "--tool", "status", "--profile", "Main",
            )
            self.assertEqual(code, 1)
            self.assertEqual(payload["error"]["code"], "validation")

    def test_a_missing_database_is_never_created_by_an_agent_read(self):
        with tempfile.TemporaryDirectory() as root:
            data_root = Path(root) / "data"
            provider = mcp_cli.BookToolProvider(
                data_root=str(data_root), project=None, env_file=None, workspace=None, profile=None
            )
            outcome = provider.call_tool("status", {}, threading.Event())
            self.assertTrue(outcome.is_error)
            self.assertEqual(outcome.structured["error"]["code"], "not_initialized")
            self.assertFalse((data_root / "kassiber.sqlite3").exists())


class EncryptedManualBookTests(unittest.TestCase):
    def test_manual_encrypted_book_asks_for_a_user_unlock_and_never_prompts(self):
        from kassiber.db import resolve_database_path
        from kassiber.secrets.migration import create_empty_encrypted_database
        from kassiber.secrets.sqlcipher import sqlcipher_available

        if not sqlcipher_available():
            self.skipTest("SQLCipher is required")
        with tempfile.TemporaryDirectory() as root:
            create_empty_encrypted_database(resolve_database_path(root), "tracer-pass-12345")
            conn = open_db(root, passphrase="tracer-pass-12345")
            conn.close()
            provider = mcp_cli.BookToolProvider(
                data_root=root, project=None, env_file=None, workspace=None, profile=None
            )
            with mock.patch(
                "kassiber.secrets.prompt.prompt_passphrase",
                side_effect=AssertionError("an MCP call must never prompt"),
            ):
                outcome = provider.call_tool("status", {}, threading.Event())
            self.assertTrue(outcome.is_error)
            error = outcome.structured["error"]
            self.assertEqual(error["code"], "interaction_required")
            self.assertEqual(error["details"]["reason"], "database_passphrase")
            self.assertEqual(error["details"]["unlock_mode"], "manual")
            self.assertEqual(error["details"]["user_command"], "kassiber operator unlock")
            self.assertIn("never ask", error["hint"])

            payload, code = _run_cli("--data-root", root, "--machine", "status")
            self.assertEqual(code, 1)
            self.assertEqual(payload["error"]["details"]["reason"], "database_passphrase")


class BrokeredRoutingTests(_TwoBookFixture):
    def _brokered(self):
        return mock.patch(
            "kassiber.operator.modes.effective_unlock_mode",
            return_value="brokered",
        )

    def test_no_lease_asks_for_a_user_unlock_without_starting_a_broker(self):
        with self._brokered(), mock.patch(
            "kassiber.operator.client.BrokerClient.status",
            return_value={"broker": "stopped", "lease": "locked"},
        ), mock.patch(
            "kassiber.operator.client.BrokerClient.ensure_running",
            side_effect=AssertionError("must not start a broker"),
        ), mock.patch(
            "kassiber.operator.client.BrokerClient.submit",
            side_effect=AssertionError("must not submit"),
        ):
            outcome = self.provider().call_tool("status", {}, threading.Event())
        self.assertTrue(outcome.is_error)
        error = outcome.structured["error"]
        self.assertEqual(error["code"], "interaction_required")
        self.assertEqual(error["details"]["reason"], "operator_lease_required")
        self.assertIn("operator unlock", error["hint"])
        self.assertIn("Never ask for the passphrase", error["hint"])

    def test_lease_runs_one_scoped_mcp_call_operation(self):
        captured: dict = {}

        def submit(_client, data_root, prepared, *, admin_authentication, **options):
            captured["argv"] = list(prepared.argv)
            captured["admin"] = admin_authentication
            captured["start_broker"] = options.get("start_broker", True)
            captured["require_caller_context"] = options.get("require_caller_context")
            return {"operation_id": "gen.client.op", "state": "queued"}

        child = {
            "kind": "mcp.call",
            "schema_version": 1,
            "data": {"tool": "status", "book": {"profile": "Main"}, "data": {"ok": 1}},
        }
        with self._brokered(), mock.patch(
            "kassiber.operator.client.BrokerClient.status",
            return_value={
                "lease": "unlocked",
                "default_scope": {"workspace": "ws-id", "profile": "book-id"},
            },
        ), mock.patch(
            "kassiber.operator.client.BrokerClient.submit",
            autospec=True,
            side_effect=submit,
        ), mock.patch(
            "kassiber.operator.client.BrokerClient.operation_status",
            return_value={
                "operation_id": "gen.client.op",
                "state": "completed",
                "exit_code": 0,
                "stdout": json.dumps(child),
                "stderr": "",
            },
        ):
            outcome = self.provider().call_tool(
                "transactions_list", {"limit": 3}, threading.Event()
            )
        self.assertFalse(outcome.is_error, outcome.structured)
        self.assertEqual(outcome.structured["data"], {"ok": 1})
        argv = captured["argv"]
        self.assertIsNone(captured["admin"])
        self.assertIs(captured["start_broker"], False)
        self.assertIs(captured["require_caller_context"], True)
        self.assertEqual(argv[argv.index("mcp") : argv.index("mcp") + 2], ["mcp", "call"])
        self.assertEqual(argv[argv.index("--workspace") + 1], "ws-id")
        self.assertEqual(argv[argv.index("--profile") + 1], "book-id")
        self.assertEqual(json.loads(argv[argv.index("--arguments") + 1]), {"limit": 3})
        self.assertNotIn("--db-passphrase-fd", argv)

    def test_an_agent_lease_registers_this_process_once_and_again_when_dropped(self):
        opened: list[str] = []

        class Handle:
            alive = True
            token = "session-token"

            def close(self):
                self.alive = False

        def open_session(_client, data_root):
            opened.append(data_root)
            return Handle()

        submits = iter(
            [
                AppError(
                    "no session",
                    code="interaction_required",
                    details={"reason": "agent_session_required"},
                ),
                {"operation_id": "gen.client.op", "state": "queued"},
                {"operation_id": "gen.client.op2", "state": "queued"},
            ]
        )

        tokens: list[object] = []

        def submit(*_args, **kwargs):
            tokens.append(kwargs.get("agent_session_token"))
            outcome = next(submits)
            if isinstance(outcome, Exception):
                raise outcome
            return outcome

        child = {"kind": "mcp.call", "schema_version": 1, "data": {"tool": "status", "data": {}}}
        provider = self.provider()
        with self._brokered(), mock.patch(
            "kassiber.operator.client.BrokerClient.status",
            return_value={
                "lease": "unlocked",
                "agent_scope": True,
                "default_scope": {"workspace": "w", "profile": "p"},
            },
        ), mock.patch(
            "kassiber.operator.client.BrokerClient.open_agent_session",
            autospec=True,
            side_effect=open_session,
        ), mock.patch(
            "kassiber.operator.client.BrokerClient.submit",
            side_effect=submit,
        ), mock.patch(
            "kassiber.operator.client.BrokerClient.operation_status",
            return_value={"state": "completed", "exit_code": 0, "stdout": json.dumps(child), "stderr": ""},
        ):
            first = provider.call_tool("status", {}, threading.Event())
            second = provider.call_tool("status", {}, threading.Event())
        self.assertFalse(first.is_error, first.structured)
        self.assertFalse(second.is_error, second.structured)
        # One registration, one re-registration after the broker dropped it,
        # and the live session is reused for the next call.
        self.assertEqual(len(opened), 2)
        # Every call presents the session's token.
        self.assertEqual(tokens, ["session-token"] * 3)

    def test_parallel_calls_with_a_stale_session_replace_it_only_once(self):
        class Handle:
            def __init__(self, name):
                self.name = name
                self.alive = True
                self.token = name

            def close(self):
                self.alive = False

        opened = []

        class Client:
            def open_agent_session(self, data_root):
                opened.append(data_root)
                return Handle(f"h{len(opened)}")

        provider = self.provider()
        client = Client()
        stale = provider._agent_session(client, "/book")
        # Call A replaces the stale registration ...
        fresh = provider._agent_session(client, "/book", replacing=stale)
        # ... and call B, which saw the same stale one, reuses A's.
        again = provider._agent_session(client, "/book", replacing=stale)
        self.assertIs(again, fresh)
        self.assertTrue(fresh.alive)
        self.assertFalse(stale.alive)
        self.assertEqual(len(opened), 2)

    def test_a_waiting_agent_is_told_to_ask_the_user(self):
        waiting = AppError(
            "waiting for the user to allow this agent in Kassiber",
            code="interaction_required",
            hint="Ask the user to allow this agent in Kassiber under Settings, AI, External agents, then retry.",
            details={"reason": "agent_pairing_required"},
            retryable=True,
        )

        class Handle:
            alive = True
            token = "session-token"

            def close(self):
                self.alive = False

        with self._brokered(), mock.patch(
            "kassiber.operator.client.BrokerClient.status",
            return_value={
                "lease": "unlocked",
                "agent_scope": True,
                "default_scope": {"workspace": "w", "profile": "p"},
            },
        ), mock.patch(
            "kassiber.operator.client.BrokerClient.open_agent_session",
            return_value=Handle(),
        ), mock.patch(
            "kassiber.operator.client.BrokerClient.submit",
            side_effect=waiting,
        ):
            outcome = self.provider().call_tool("status", {}, threading.Event())
        self.assertTrue(outcome.is_error)
        self.assertEqual(outcome.structured["error"]["details"]["reason"], "agent_pairing_required")
        self.assertIn("allow this agent in Kassiber", outcome.structured["error"]["hint"])
        self.assertTrue(outcome.structured["error"]["retryable"])

    def test_no_lease_points_at_the_desktop_unlock_too(self):
        with self._brokered(), mock.patch(
            "kassiber.operator.client.BrokerClient.status",
            return_value={"broker": "stopped", "lease": "locked"},
        ):
            outcome = self.provider().call_tool("status", {}, threading.Event())
        self.assertIn("unlock this book for agents in Kassiber", outcome.structured["error"]["hint"])

    def test_brokered_child_errors_become_tool_errors(self):
        child = {
            "kind": "error",
            "schema_version": 1,
            "error": {"code": "operator_capability_denied", "message": "denied", "retryable": False},
        }
        with self._brokered(), mock.patch(
            "kassiber.operator.client.BrokerClient.status",
            return_value={"lease": "unlocked", "default_scope": {"workspace": "w", "profile": "p"}},
        ), mock.patch(
            "kassiber.operator.client.BrokerClient.submit",
            return_value={"operation_id": "gen.client.op", "state": "queued"},
        ), mock.patch(
            "kassiber.operator.client.BrokerClient.operation_status",
            return_value={"state": "failed", "exit_code": 1, "stdout": json.dumps(child), "stderr": ""},
        ):
            outcome = self.provider().call_tool("status", {}, threading.Event())
        self.assertTrue(outcome.is_error)
        self.assertEqual(outcome.structured["error"]["code"], "operator_capability_denied")


class CommandSurfaceTests(unittest.TestCase):
    def test_mcp_commands_are_catalogued_with_their_effects(self):
        catalog = describe_command_catalog(build_parser(), ["mcp"])
        by_command = {command["command"]: command for command in catalog["commands"]}
        self.assertEqual(
            set(by_command),
            {"mcp serve", "mcp tools", "mcp call", "mcp status", "mcp enable", "mcp disable"},
        )
        for command in ("mcp status", "mcp enable", "mcp disable"):
            self.assertFalse(by_command[command]["needs_database"])
        # Turning disclosure on is never a leasable capability.
        self.assertIs(cli_capability("mcp.enable"), Capability.ADMIN)
        self.assertEqual(by_command["mcp serve"]["effect"], "interactive")
        self.assertFalse(by_command["mcp serve"]["needs_database"])
        self.assertFalse(by_command["mcp tools"]["needs_database"])
        self.assertTrue(by_command["mcp call"]["needs_database"])
        # The command's broker baseline is read; the tool it runs may write
        # within the user's grant, and the broker classifies each tool.
        self.assertEqual(by_command["mcp call"]["effect"], "mutating")
        self.assertEqual(by_command["mcp tools"]["effect"], "read_only")
        for path in ("mcp.serve", "mcp.tools", "mcp.call"):
            self.assertIs(cli_capability(path), Capability.READ)
        flags = {arg["name"]: arg for arg in by_command["mcp enable"]["arguments"]}
        self.assertEqual(flags["grant"]["choices"], ["read", "operator", "accounting_decisions"])
        self.assertEqual(flags["agent_access"]["choices"], ["full", "restricted"])

    def test_only_mcp_call_is_routed_through_the_broker(self):
        for command, expected_routed in (("serve", False), ("tools", False), ("call", True)):
            with self.subTest(command=command):
                argv = ["mcp", command] + (["--tool", "status"] if command == "call" else [])
                args = build_parser().parse_args(argv)
                with mock.patch(
                    "kassiber.operator.cli._selected_data_root",
                    return_value="/canonical-project",
                ), mock.patch(
                    "kassiber.operator.cli.effective_unlock_mode",
                    return_value="manual",
                ) as mode:
                    self.assertIsNone(route_brokered_command(args, ["mcp", command]))
                self.assertEqual(mode.called, expected_routed)

    def test_a_one_shot_mcp_call_insists_on_this_builds_broker(self):
        captured = {}

        def submit(_client, data_root, prepared, **options):
            captured.update(options)
            return {"operation_id": "g.op", "state": "queued"}

        with tempfile.TemporaryDirectory() as tmp:
            args = mock.Mock(
                command="mcp",
                mcp_command="call",
                data_root=tmp,
                env_file=None,
                project=None,
                operator_auth_fd=None,
                non_interactive=True,
                machine=True,
            )
            with contextlib.redirect_stderr(io.StringIO()), mock.patch(
                "kassiber.operator.cli.effective_unlock_mode", return_value="brokered"
            ), mock.patch(
                "kassiber.cli.command_registry.command_path", return_value="mcp.call"
            ), mock.patch(
                "kassiber.operator.cli.BrokerClient.submit", autospec=True, side_effect=submit
            ), mock.patch(
                "kassiber.operator.cli.BrokerClient.wait",
                return_value={"state": "completed", "exit_code": 0},
            ):
                route_brokered_command(args, ["mcp", "call", "--tool", "status"])
        self.assertIs(captured.get("require_same_build"), True)

    def test_the_broker_refuses_to_queue_a_long_lived_server(self):
        from kassiber.operator.service import OperatorService

        service = OperatorService("generation", lambda *_: None)
        try:
            with self.assertRaises(AppError) as raised:
                service.submit("/unused", ["mcp", "serve"])
            self.assertEqual(raised.exception.code, "operator_command_not_brokerable")
        finally:
            service.close()


class PinnedContextTests(unittest.TestCase):
    def test_pin_overrides_reads_and_refuses_writes_of_context_only(self):
        with tempfile.TemporaryDirectory() as root:
            conn = open_db(root)
            try:
                set_setting(conn, "context_profile", "stored")
                with pinned_context_scope("ws", "book"):
                    self.assertEqual(get_setting(conn, "context_profile"), "book")
                    self.assertEqual(get_setting(conn, "context_workspace"), "ws")
                    set_setting(conn, "unrelated", "ok")
                    with self.assertRaises(AppError) as raised:
                        set_setting(conn, "context_profile", "other")
                    self.assertEqual(raised.exception.code, "scope_pinned")
                self.assertEqual(get_setting(conn, "context_profile"), "stored")
            finally:
                conn.close()


class StdioProcessTests(unittest.TestCase):
    def test_server_process_keeps_stdout_pure_and_exits_on_eof(self):
        with tempfile.TemporaryDirectory() as root:
            messages = [
                _request(1, "initialize", {"protocolVersion": "2025-11-25", "capabilities": {}}),
                {"jsonrpc": "2.0", "method": "notifications/initialized"},
                _request(2, "tools/list"),
                _request(3, "server/discover", {"_meta": MODERN_META}),
            ]
            completed = subprocess.run(
                [sys.executable, "-m", "kassiber", "--data-root", str(Path(root) / "data"), "mcp", "serve"],
                input="".join(json.dumps(m) + "\n" for m in messages).encode(),
                capture_output=True,
                timeout=120,
                env={**os.environ, "KASSIBER_NO_EGRESS": "1"},
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertEqual(completed.stderr, b"")
            lines = completed.stdout.decode().splitlines()
            responses = [json.loads(line) for line in lines]
            self.assertEqual([r["id"] for r in responses], [1, 2, 3])
            self.assertEqual(responses[0]["result"]["protocolVersion"], "2025-11-25")
            self.assertEqual(len(responses[1]["result"]["tools"]), len(mcp_tools.available_tools(grant="read", agent_access="restricted")))
            # Launch and listing touched nothing on disk.
            self.assertFalse((Path(root) / "data").exists())


if __name__ == "__main__":
    unittest.main()


class PublishedSchemaEnforcementTests(unittest.TestCase):
    def test_every_published_keyword_is_enforced_by_the_shared_validator(self):
        from kassiber.daemon import _validate_ai_schema_value

        schema = {
            "type": "object",
            "properties": {
                "code": {"type": "string", "minLength": 2, "maxLength": 4, "pattern": "^[a-f0-9]+$"},
                "count": {"type": "integer", "exclusiveMinimum": 0, "exclusiveMaximum": 10},
            },
        }
        for value in ({"code": "a"}, {"code": "abcde"}, {"code": "zz"}, {"count": 0}, {"count": 10}):
            with self.subTest(value=value), self.assertRaises(AppError) as raised:
                _validate_ai_schema_value(value, schema, path="tool")
            self.assertEqual(raised.exception.code, "validation")
        _validate_ai_schema_value({"code": "ab12", "count": 9}, schema, path="tool")

    def test_out_of_bounds_mcp_arguments_are_refused_before_execution(self):
        definitions = {d["name"]: d for d in mcp_tools.tool_definitions(grant="accounting_decisions", agent_access="full")}
        constrained = [
            (name, key, spec)
            for name, definition in definitions.items()
            for key, spec in definition["inputSchema"].get("properties", {}).items()
            if isinstance(spec, dict) and "exclusiveMinimum" in spec
        ]
        self.assertTrue(constrained)
        name, key, spec = constrained[0]
        from kassiber.daemon import _validate_ai_tool_arguments

        with self.assertRaises(AppError):
            _validate_ai_tool_arguments(mcp_tools.entry_for(name).entry, {key: spec["exclusiveMinimum"]})


class ReviewHardeningTests(_TwoBookFixture):
    def test_a_broker_that_exits_after_the_lease_check_is_not_restarted(self):
        from kassiber.operator.client import BrokerClient

        with mock.patch(
            "kassiber.operator.modes.effective_unlock_mode", return_value="brokered"
        ), mock.patch.object(
            BrokerClient, "status",
            return_value={"lease": "unlocked", "default_scope": {"workspace": "w", "profile": "p"}},
        ), mock.patch.object(
            BrokerClient, "ping", side_effect=ConnectionRefusedError()
        ), mock.patch.object(
            BrokerClient, "ensure_running", side_effect=AssertionError("must not start a broker")
        ):
            outcome = self.provider().call_tool("status", {}, threading.Event())
        self.assertTrue(outcome.is_error)
        self.assertEqual(outcome.structured["error"]["details"]["reason"], "operator_lease_required")

    def test_oversized_results_are_refused_by_their_encoded_size(self):
        big = {"tool": "status", "data": "\U0001F600" * (mcp_tools.MAX_RESULT_BYTES // 12 + 1)}
        with mock.patch.object(mcp_cli.BookToolProvider, "_call", return_value=big):
            outcome = self.provider().call_tool("status", {}, threading.Event())
        # Each non-BMP character escapes to 12 ASCII bytes on the wire.
        self.assertTrue(outcome.is_error)
        self.assertEqual(outcome.structured["error"]["code"], "result_too_large")


class StdioRobustnessTests(unittest.TestCase):
    def test_deeply_nested_json_is_a_parse_error_not_a_crash(self):
        server = McpServer(_FakeProvider(), name="kassiber", version="test")
        nested = b"[" * 200_000 + b"]" * 200_000 + b"\n"
        follow = (json.dumps(_request(1, "server/discover", {"_meta": MODERN_META})) + "\n").encode()
        writer = io.BytesIO()
        serve(server, io.BytesIO(nested + follow), writer)
        lines = [json.loads(line) for line in writer.getvalue().decode().splitlines()]
        self.assertEqual(lines[0]["error"]["code"], -32700)
        self.assertEqual(lines[1]["id"], 1)

    def test_calls_beyond_the_in_flight_limit_are_refused(self):
        from kassiber.mcp import protocol

        provider = _FakeProvider()
        provider.release.clear()
        server = McpServer(provider, name="kassiber", version="test")
        count = protocol.MAX_INFLIGHT_CALLS + 3
        messages = [
            _request(n, "tools/call", {"_meta": MODERN_META, "name": "echo", "arguments": {}})
            for n in range(count)
        ]
        timer = threading.Timer(0.5, provider.release.set)
        timer.start()
        try:
            responses = _serve_lines(server, messages)
        finally:
            timer.cancel()
        refused = [r for r in responses if r.get("error", {}).get("code") == protocol.SERVER_OVERLOADED]
        self.assertTrue(refused)
        self.assertEqual(len(responses), count)


class OutdatedBrokerMcpTests(_TwoBookFixture):
    def test_mcp_refuses_a_broker_that_ignores_caller_context(self):
        from kassiber.operator.client import BrokerClient

        with mock.patch(
            "kassiber.operator.modes.effective_unlock_mode", return_value="brokered"
        ), mock.patch.object(
            BrokerClient, "status",
            return_value={"lease": "unlocked", "default_scope": {"workspace": "w", "profile": "p"}},
        ), mock.patch.object(
            BrokerClient, "ping", return_value={"generation": "old", "build": build_identity()}
        ), mock.patch.object(
            BrokerClient, "_submit_once", side_effect=AssertionError("must not submit")
        ):
            outcome = self.provider().call_tool("status", {}, threading.Event())
        self.assertEqual(outcome.structured["error"]["code"], "operator_broker_outdated")

    def test_mcp_refuses_a_broker_of_another_build(self):
        from kassiber.operator.client import BrokerClient

        other = {**build_identity(), "origin": "0" * 12}
        with mock.patch(
            "kassiber.operator.modes.effective_unlock_mode", return_value="brokered"
        ), mock.patch.object(
            BrokerClient, "status",
            return_value={"lease": "unlocked", "default_scope": {"workspace": "w", "profile": "p"}},
        ), mock.patch.object(
            BrokerClient, "ping",
            return_value={"generation": "g", "caller_context": True, "build": other},
        ), mock.patch.object(
            BrokerClient, "_submit_once", side_effect=AssertionError("must not submit")
        ):
            outcome = self.provider().call_tool("status", {}, threading.Event())
        self.assertEqual(outcome.structured["error"]["code"], "operator_broker_build_mismatch")

    def test_invalid_arguments_are_refused_before_anything_is_queued(self):
        from kassiber.operator.client import BrokerClient

        with mock.patch(
            "kassiber.operator.modes.effective_unlock_mode", return_value="brokered"
        ), mock.patch.object(
            BrokerClient, "status", side_effect=AssertionError("must not contact the broker")
        ):
            outcome = self.provider().call_tool("status", {"x": 1}, threading.Event())
        self.assertEqual(outcome.structured["error"]["code"], "validation")


class StrictJsonTests(unittest.TestCase):
    def test_non_json_constants_are_parse_errors(self):
        server = McpServer(_FakeProvider(), name="kassiber", version="test")
        line = b'{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"echo","arguments":{"x":NaN}}}\n'
        writer = io.BytesIO()
        serve(server, io.BytesIO(line), writer)
        response = json.loads(writer.getvalue().decode().splitlines()[0])
        self.assertEqual(response["error"]["code"], -32700)

    def test_one_shot_arguments_reject_non_json_constants(self):
        with self.assertRaises(AppError) as raised:
            mcp_cli._parse_arguments('{"limit": Infinity}')
        self.assertEqual(raised.exception.code, "validation")

    def test_fresh_machine_mcp_call_creates_no_project(self):
        with tempfile.TemporaryDirectory() as state:
            catalog = Path(state) / "projects.json"
            with mock.patch("kassiber.projects.catalog_path", return_value=catalog), mock.patch(
                "kassiber.projects.ensure_default_project",
                side_effect=AssertionError("must not create a project"),
            ), mock.patch(
                "kassiber.cli.main._maybe_migrate_default_state_root"
            ):
                payload, code = _run_cli("--machine", "mcp", "call", "--tool", "status")
            self.assertEqual(code, 1)
            self.assertEqual(payload["error"]["code"], "not_initialized")
            self.assertFalse(catalog.exists())


class AgentAccessTests(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.path = Path(self._tmp.name) / "agent-access.json"

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def test_off_by_default_and_fails_closed_on_bad_files(self):
        from kassiber.agent_access import agent_access_status

        self.assertEqual(agent_access_status(self.path)["reason"], "mcp_disabled")
        for content in ("not json", '{"schema_version": 2, "mcp_enabled": true}', '{"mcp_enabled": true}'):
            self.path.write_text(content)
            with self.subTest(content=content):
                self.assertFalse(agent_access_status(self.path)["mcp_available"])

    def test_the_ai_master_switch_blocks_and_restores_agent_access(self):
        from kassiber.agent_access import agent_access_status, set_agent_access

        self.assertTrue(set_agent_access(mcp_enabled=True, path=self.path)["mcp_available"])
        off = set_agent_access(ai_features_enabled=False, path=self.path)
        self.assertEqual((off["mcp_available"], off["reason"]), (False, "ai_features_disabled"))
        self.assertTrue(off["mcp_enabled"], "the user's own choice is kept")
        self.assertTrue(set_agent_access(ai_features_enabled=True, path=self.path)["mcp_available"])
        self.assertTrue(agent_access_status(self.path)["mcp_available"])
        if os.name != "nt":
            self.assertEqual(self.path.stat().st_mode & 0o777, 0o600)

    def test_a_disabled_server_refuses_calls_before_touching_any_book(self):
        from kassiber.agent_access import PATH_ENV, set_agent_access

        with tempfile.TemporaryDirectory() as root:
            provider = mcp_cli.BookToolProvider(
                data_root=str(Path(root) / "never-created"),
                project=None, env_file=None, workspace=None, profile=None,
            )
            with mock.patch.dict(os.environ, {PATH_ENV: str(self.path)}):
                refused = provider.call_tool("status", {}, threading.Event())
                self.assertEqual(refused.structured["error"]["code"], "mcp_disabled")
                set_agent_access(mcp_enabled=True, ai_features_enabled=False, path=self.path)
                master = provider.call_tool("status", {}, threading.Event())
                self.assertEqual(master.structured["error"]["code"], "ai_features_disabled")
            self.assertFalse((Path(root) / "never-created").exists())

    def test_one_shot_call_is_refused_when_disabled(self):
        from kassiber.agent_access import PATH_ENV

        with tempfile.TemporaryDirectory() as root, mock.patch.dict(os.environ, {PATH_ENV: str(self.path)}):
            payload, code = _run_cli(
                "--data-root", str(Path(root) / "data"), "--machine", "mcp", "call", "--tool", "status"
            )
        self.assertEqual(code, 1)
        self.assertEqual(payload["error"]["code"], "mcp_disabled")

    def test_agents_cannot_enable_themselves_but_can_always_disable(self):
        from kassiber.agent_access import PATH_ENV, agent_access_status, set_agent_access

        with mock.patch.dict(os.environ, {PATH_ENV: str(self.path)}):
            payload, code = _run_cli("--machine", "mcp", "enable")
            self.assertEqual(code, 1)
            self.assertEqual(payload["error"]["code"], "interaction_required")
            self.assertFalse(agent_access_status(self.path)["mcp_enabled"])
            set_agent_access(mcp_enabled=True, path=self.path)
            payload, code = _run_cli("--machine", "mcp", "disable")
            self.assertEqual(code, 0, payload)
            self.assertFalse(payload["data"]["mcp_enabled"])
            payload, code = _run_cli("--machine", "mcp", "status")
            self.assertEqual(payload["kind"], "mcp.status")

    def test_the_desktop_configures_access_through_validated_daemon_kinds(self):
        from kassiber import daemon
        from kassiber.agent_access import PATH_ENV

        self.assertIn("ui.agent_access.status", daemon.SUPPORTED_KINDS)
        self.assertIn("ui.agent_access.configure", daemon.SUPPORTED_KINDS)
        # No book is open: status and configure need none.
        ctx = SimpleNamespace(conn=None, db_passphrase=None, data_root=str(self.path.parent), agent_lease=None)
        with mock.patch.dict(os.environ, {PATH_ENV: str(self.path)}):
            status = daemon._agent_access_payload(
                ctx,
                "ui.agent_access.configure",
                {"args": {"mcp_enabled": True, "ai_features_enabled": True}},
            )
            self.assertTrue(status["mcp_available"])
            self.assertEqual(status["session"], {"needed": False, "active": False, "expires_at": None})
            for bad in ({}, {"mcp_enabled": "yes"}, {"mcp_enabled": True, "path": "/x"}):
                with self.subTest(bad=bad), self.assertRaises(AppError):
                    daemon._agent_access_payload(ctx, "ui.agent_access.configure", {"args": bad})
            with self.assertRaises(AppError):
                daemon._agent_access_payload(ctx, "ui.agent_access.status", {"args": {"x": 1}})


class _GrantFixture(unittest.TestCase):
    """One book with one transaction and its own External agents preference."""

    TXID = "ab" * 32

    def setUp(self) -> None:
        from kassiber.agent_access import PATH_ENV

        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        root = Path(self._tmp.name)
        self.access_path = root / "agent-access.json"
        env = mock.patch.dict(os.environ, {PATH_ENV: str(self.access_path)})
        env.start()
        self.addCleanup(env.stop)
        self.data_root = str(root / "data")
        tx_file = root / "tx.json"
        tx_file.write_text(
            json.dumps(
                [
                    {
                        "date": "2024-01-01",
                        "direction": "inbound",
                        "asset": "BTC",
                        "amount": "0.001",
                        "fee": "0",
                        "fiat_rate": 40000,
                        "txid": self.TXID,
                    }
                ]
            )
        )
        for argv in (
            ("init",),
            ("workspaces", "create", "Personal"),
            ("profiles", "create", "Main", "--workspace", "Personal"),
            ("wallets", "create", "--workspace", "Personal", "--profile", "Main", "--label", "W", "--kind", "phoenix"),
            ("wallets", "import-json", "--workspace", "Personal", "--profile", "Main", "--wallet", "W", "--file", str(tx_file)),
        ):
            payload, code = _run_cli("--data-root", self.data_root, "--machine", *argv)
            self.assertEqual(code, 0, payload)
        conn = open_db(self.data_root)
        try:
            self.tx_id = conn.execute("SELECT id FROM transactions").fetchone()["id"]
        finally:
            conn.close()

    def grant(self, grant: str | None, agent_access: str | None = None) -> None:
        if grant is None:
            # A preference written before grants existed.
            self.access_path.write_text(json.dumps({"schema_version": 1, "mcp_enabled": True}))
            self.access_path.chmod(0o600)
            return
        from kassiber.agent_access import set_agent_access

        set_agent_access(mcp_enabled=True, grant=grant, agent_access=agent_access or "full")

    def provider(self) -> mcp_cli.BookToolProvider:
        return mcp_cli.BookToolProvider(
            data_root=self.data_root, project=None, env_file=None, workspace="Personal", profile="Main"
        )

    def call(self, name: str, arguments: dict | None = None, client: str | None = None) -> dict:
        return self.provider().call_tool(name, arguments or {}, threading.Event(), client_name=client).structured

    def row(self, sql: str, *params):
        conn = open_db(self.data_root)
        try:
            return conn.execute(sql, params).fetchone()
        finally:
            conn.close()

    def processed(self):
        return self.row("SELECT last_processed_at FROM profiles WHERE label = 'Main'")[0]


class GrantExecutionTests(_GrantFixture):
    def test_a_legacy_preference_keeps_read_and_restricted(self):
        from kassiber.agent_access import agent_access_status

        self.grant(None)
        status = agent_access_status()
        self.assertEqual(
            (status["grant"], status["agent_access"], status["access_recorded"]),
            ("read", "restricted", False),
        )
        listed = {d["name"] for d in self.provider().tool_definitions()}
        self.assertEqual(
            listed, {t.name for t in mcp_tools.available_tools(grant="read", agent_access="restricted")}
        )
        self.assertNotIn("journals_process", listed)
        self.assertNotIn("custody_gaps_list", listed)
        self.assertIn("read-only", self.provider().instructions)
        refused = self.call("journals_process")["error"]
        self.assertEqual(refused["code"], "mcp_grant_insufficient")
        self.assertEqual(refused["details"], {"required": "operator", "granted": "read"})
        self.assertIn("kassiber mcp enable --grant operator", refused["hint"])
        self.assertEqual(self.call("custody_gaps_list")["error"]["code"], "agent_access_restricted")
        # Reads stay strictly read-only: no journal rebuild.
        self.call("reports_balance_sheet")
        self.assertIsNone(self.processed())

    def test_writes_are_refused_below_their_grant_and_change_nothing(self):
        self.grant("operator")
        refused = self.call("transactions_metadata_update", {"transaction": self.tx_id, "note": "x"})
        self.assertEqual(refused["error"]["code"], "mcp_grant_insufficient")
        self.assertEqual(refused["error"]["details"]["required"], "accounting_decisions")
        self.assertIsNone(self.row("SELECT note FROM transactions WHERE id = ?", self.tx_id)[0])
        self.grant("read")
        self.assertEqual(self.call("journals_process")["error"]["code"], "mcp_grant_insufficient")
        self.assertIsNone(self.processed())

    def test_operator_reads_refresh_journals_locally_but_stay_query_only(self):
        from kassiber import daemon

        self.grant("operator")
        observed = []
        original = daemon._execute_read_only_ai_tool

        def spy(call, runtime):
            observed.append(
                daemon._run_on_daemon_main_thread(
                    runtime, lambda conn: conn.execute("PRAGMA query_only").fetchone()[0]
                )
            )
            return original(call, runtime)

        with mock.patch.object(daemon, "_execute_read_only_ai_tool", side_effect=spy):
            result = self.call("reports_balance_sheet")
        self.assertNotIn("error", result, result)
        self.assertEqual(observed, [1])
        self.assertIsNotNone(self.processed())
        self.assertNotIn("read-only", self.provider().instructions)

    def test_a_write_within_the_grant_runs_through_the_assistant_executor(self):
        from kassiber import daemon

        self.grant("accounting_decisions")
        seen = {}
        original = daemon._execute_mutating_ai_tool

        def spy(call, runtime, *, custody_audit=None):
            seen["query_only"] = daemon._run_on_daemon_main_thread(
                runtime, lambda conn: conn.execute("PRAGMA query_only").fetchone()[0]
            )
            seen["audit"] = custody_audit
            seen["advertised"] = runtime.maintenance_state["advertised_tools"]
            return original(call, runtime, custody_audit=custody_audit)

        with mock.patch.object(daemon, "_execute_mutating_ai_tool", side_effect=spy):
            result = self.call(
                "transactions_metadata_update",
                {"transaction": self.tx_id, "note": "agent note"},
                client="claude-code\u0007",
            )
        self.assertNotIn("error", result, result)
        self.assertEqual(result["kind"], "ui.transactions.metadata.update")
        self.assertEqual(seen["query_only"], 0)
        self.assertEqual(
            (seen["audit"].provider_kind, seen["audit"].model, seen["audit"].consent_decision),
            ("mcp", "claude-code", "mcp_host"),
        )
        self.assertIn("ui_transactions_metadata_update", seen["advertised"])
        self.assertEqual(self.row("SELECT note FROM transactions WHERE id = ?", self.tx_id)[0], "agent note")
        # Attributed like any assistant edit in the append-only history.
        self.assertEqual(
            self.row("SELECT source FROM transaction_edit_events WHERE transaction_id = ?", self.tx_id)[0],
            "ai_tool",
        )
        self.assertNotIn("error", self.call("journals_process"))
        self.assertIsNotNone(self.processed())

    def test_custody_writes_are_audited_as_approved_by_the_mcp_host(self):
        self.grant("accounting_decisions", "full")
        result = self.call(
            "custody_review_apply",
            {"action": "create", "gap_id": "custody-gap:missing", "expected_input_version": 0},
            client="codex",
        )
        self.assertIn("error", result)
        audit = self.row(
            "SELECT provider_kind, model, consent_decision, execution_status, tool_name "
            "FROM custody_ai_assistance_audits"
        )
        self.assertIsNotNone(audit, "a custody write over MCP must leave an audit record")
        self.assertEqual(
            tuple(audit), ("mcp", "codex", "mcp_host", "failed", "ui.custody.review.apply")
        )

    def test_writes_need_a_server_pinned_to_one_book(self):
        # Unpinned, each call follows the active book, so a task could read
        # one book and change another; reads stay available.
        self.grant("accounting_decisions")
        unpinned = mcp_cli.BookToolProvider(
            data_root=self.data_root, project=None, env_file=None, workspace=None, profile=None
        )
        refused = unpinned.call_tool(
            "transactions_metadata_update", {"transaction": self.tx_id, "note": "x"}, threading.Event()
        ).structured
        self.assertEqual(refused["error"]["code"], "book_pin_required")
        self.assertIsNone(self.row("SELECT note FROM transactions WHERE id = ?", self.tx_id)[0])
        self.assertNotIn("error", unpinned.call_tool("status", {}, threading.Event()).structured)

    def test_a_readonly_refusal_is_recognised_from_either_driver(self):
        import sqlite3

        class SqlcipherError(Exception):
            pass

        self.assertTrue(mcp_tools._is_readonly_refusal(SqlcipherError("attempt to write a readonly database")))
        stdlib = sqlite3.OperationalError("attempt to write a readonly database")
        stdlib.sqlite_errorcode = 8
        self.assertTrue(mcp_tools._is_readonly_refusal(stdlib))
        self.assertFalse(mcp_tools._is_readonly_refusal(SqlcipherError("database is locked")))

    def test_admin_tools_are_never_listed_or_callable(self):
        self.grant("accounting_decisions", "full")
        provider = self.provider()
        listed = {d["name"] for d in provider.tool_definitions()}
        self.assertNotIn("imports_rollback", listed)
        self.assertFalse(provider.has_tool("imports_rollback"))
        server = McpServer(provider, name="kassiber", version="test")
        response = server.handle(
            _request(1, "tools/call", {"_meta": MODERN_META, "name": "imports_rollback", "arguments": {}})
        )
        self.assertEqual(response["error"]["code"], INVALID_PARAMS)

    def test_launch_listing_and_reads_at_the_widest_grant_open_no_socket(self):
        from kassiber.core import freshness

        self.grant("accounting_decisions", "full")
        conn = open_db(self.data_root)
        try:
            profile_id = conn.execute("SELECT id FROM profiles WHERE label = 'Main'").fetchone()["id"]
            # Even with the book's opt-in sync before report reads, an agent's
            # read never syncs: only the local journal refresh runs.
            freshness.set_policy(conn, profile_id, report_read_sync=True)
            conn.commit()
        finally:
            conn.close()
        server = McpServer(self.provider(), name="kassiber", version="test")
        with no_egress_guard(enabled=True, allow_loopback=False):
            responses = _serve_lines(
                server,
                [
                    _request(1, "server/discover", {"_meta": MODERN_META}),
                    _request(2, "tools/list", {"_meta": MODERN_META}),
                    _request(3, "tools/call", {"_meta": MODERN_META, "name": "reports_balance_sheet", "arguments": {}}),
                    _request(4, "tools/call", {"_meta": MODERN_META, "name": "wallets_list", "arguments": {}}),
                    _request(5, "tools/call", {"_meta": MODERN_META, "name": "review_worklist", "arguments": {}}),
                ],
            )
        self.assertEqual(sorted(r["id"] for r in responses), [1, 2, 3, 4, 5])
        self.assertTrue(all("error" not in r for r in responses), responses)
        for response in responses[2:]:
            self.assertNotIn("isError", response["result"], response)
        listed = {tool["name"] for tool in responses[1]["result"]["tools"]}
        self.assertIn("wallets_sync", listed)
        self.assertIsNotNone(self.processed())

    def test_an_operator_child_runs_with_no_more_than_the_broker_admitted(self):
        import argparse

        self.grant("accounting_decisions")

        def access(tool, grant=None, child=False):
            env = {"KASSIBER_OPERATOR_CHILD": "1"} if child else {}
            with mock.patch.dict(os.environ, env):
                if not child:
                    os.environ.pop("KASSIBER_OPERATOR_CHILD", None)
                return mcp_cli._call_access(argparse.Namespace(tool=tool, grant=grant))[0]

        self.assertEqual(access("reports_balance_sheet", child=True), "read")
        self.assertEqual(access("journals_process", child=True), "operator")
        self.assertEqual(access("reports_balance_sheet", "operator", child=True), "operator")
        # An explicit --grant is a ceiling inside a broker worker too.
        self.assertEqual(access("journals_process", "read", child=True), "read")
        self.assertEqual(access("transfers_pair", "operator", child=True), "operator")
        self.assertEqual(access("reports_balance_sheet"), "accounting_decisions")
        self.assertEqual(access("reports_balance_sheet", "read"), "read")
        self.grant("read")
        # A claimed grant never widens the user's.
        self.assertEqual(access("reports_balance_sheet", "accounting_decisions"), "read")


class BrokeredGrantTests(_GrantFixture):
    def _run(self, status, name, arguments, *, client=None):
        captured: dict = {}

        def submit(_client, data_root, prepared, *, admin_authentication, **options):
            captured["argv"] = list(prepared.argv)
            return {"operation_id": "gen.client.op", "state": "queued"}

        child = {"kind": "mcp.call", "schema_version": 1, "data": {"tool": name, "data": {"ok": 1}}}
        with mock.patch(
            "kassiber.operator.modes.effective_unlock_mode", return_value="brokered"
        ), mock.patch(
            "kassiber.operator.client.BrokerClient.status", return_value=status
        ), mock.patch(
            "kassiber.operator.client.BrokerClient.submit", autospec=True, side_effect=submit
        ), mock.patch(
            "kassiber.operator.client.BrokerClient.operation_status",
            return_value={"operation_id": "gen.client.op", "state": "completed", "exit_code": 0,
                          "stdout": json.dumps(child), "stderr": ""},
        ):
            outcome = self.provider().call_tool(name, arguments, threading.Event(), client_name=client)
        return outcome, captured

    def test_a_brokered_write_needs_a_lease_that_covers_it(self):
        self.grant("accounting_decisions")
        lease = {"lease": "unlocked", "capability": "read", "default_scope": {"workspace": "w", "profile": "p"}}
        outcome, captured = self._run(lease, "journals_process", {})
        self.assertTrue(outcome.is_error)
        error = outcome.structured["error"]
        self.assertEqual(error["code"], "interaction_required")
        self.assertEqual(error["details"]["reason"], "operator_capability_insufficient")
        self.assertEqual(error["details"]["user_command"], "kassiber operator unlock --capability operator")
        self.assertEqual(captured, {})

        outcome, captured = self._run({**lease, "capability": "operator"}, "journals_process", {}, client="claude")
        self.assertFalse(outcome.is_error, outcome.structured)
        argv = captured["argv"]
        # The broker checks its lease covers this claimed grant too.
        self.assertEqual(argv[argv.index("--grant") + 1], "operator")
        # One token, so a host-chosen name never parses as another option.
        self.assertIn("--client=claude", argv)

    def test_a_dash_led_client_name_cannot_become_an_option(self):
        self.grant("operator")
        lease = {"lease": "unlocked", "capability": "operator", "default_scope": {"workspace": "w", "profile": "p"}}
        outcome, captured = self._run(lease, "journals_process", {}, client="--grant accounting_decisions")
        self.assertFalse(outcome.is_error, outcome.structured)
        argv = captured["argv"]
        self.assertIn("--client=grant accounting_decisions", argv)
        self.assertEqual(argv.count("--grant"), 1)
        self.assertEqual(argv[argv.index("--grant") + 1], "operator")
        self.assertEqual(mcp_tools.client_label("--help"), "help")
        self.assertIsNone(mcp_tools.client_label("\u0007"))
        self.assertEqual(mcp_tools.client_label("Claude Code 2.1"), "Claude Code 2.1")

    def test_no_lease_asks_for_an_unlock_at_the_users_grant(self):
        self.grant("operator")
        outcome, captured = self._run({"lease": "locked"}, "status", {})
        self.assertEqual(outcome.structured["error"]["details"]["user_command"],
                         "kassiber operator unlock --capability operator")
        self.assertEqual(captured, {})


class GrantPreferenceTests(unittest.TestCase):
    def setUp(self) -> None:
        from kassiber.agent_access import PATH_ENV

        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.path = Path(self._tmp.name) / "agent-access.json"
        env = mock.patch.dict(os.environ, {PATH_ENV: str(self.path)})
        env.start()
        self.addCleanup(env.stop)

    def test_grants_are_validated_recorded_together_and_fail_closed(self):
        from kassiber.agent_access import agent_access_status, set_agent_access

        for bad in ({"grant": "admin"}, {"grant": "write"}, {"agent_access": "all"}):
            with self.subTest(bad=bad), self.assertRaises(AppError):
                set_agent_access(**bad)
        status = set_agent_access(mcp_enabled=True, grant="operator")
        self.assertEqual((status["grant"], status["agent_access"], status["access_recorded"]),
                         ("operator", "restricted", True))
        stored = json.loads(self.path.read_text())
        self.assertEqual((stored["grant"], stored["agent_access"]), ("operator", "restricted"))
        # Unrelated updates keep the choice.
        self.assertEqual(set_agent_access(ai_features_enabled=False)["grant"], "operator")
        # A value this build does not know (or admin) reads as read.
        for value in ("admin", "everything", 3):
            self.path.write_text(json.dumps({"schema_version": 1, "mcp_enabled": True, "grant": value,
                                             "agent_access": "root"}))
            with self.subTest(value=value):
                status = agent_access_status()
                self.assertEqual((status["grant"], status["agent_access"]), ("read", "restricted"))

    def test_agents_cannot_widen_their_own_grant(self):
        from kassiber.agent_access import agent_access_status, set_agent_access

        set_agent_access(mcp_enabled=True)
        before = self.path.read_text()
        payload, code = _run_cli("--machine", "mcp", "enable", "--grant", "accounting_decisions",
                                 "--agent-access", "full")
        self.assertEqual(code, 1)
        self.assertEqual(payload["error"]["code"], "interaction_required")
        self.assertEqual(self.path.read_text(), before)
        self.assertEqual(agent_access_status()["grant"], "read")
        status, code = _run_cli("--machine", "mcp", "status")
        self.assertEqual(code, 0)
        self.assertEqual((status["data"]["grant"], status["data"]["agent_access"]), ("read", "restricted"))

    def test_enable_asks_in_the_users_terminal_and_records_the_grant(self):
        import argparse

        args = argparse.Namespace(mcp_command="enable", non_interactive=False,
                                  grant="accounting_decisions", agent_access="full")
        prompt = io.StringIO()
        with mock.patch.object(mcp_cli.sys, "stdin") as stdin, contextlib.redirect_stderr(prompt):
            stdin.isatty.return_value = True
            stdin.readline.return_value = "n\n"
            declined = mcp_cli._configure_access(args)
            stdin.readline.return_value = "y\n"
            accepted = mcp_cli._configure_access(args)
        self.assertFalse(declined["mcp_enabled"])
        self.assertEqual((accepted["mcp_enabled"], accepted["grant"], accepted["agent_access"]),
                         (True, "accounting_decisions", "full"))
        text = prompt.getvalue()
        self.assertIn("accounting decisions", text)
        self.assertIn("contact your backends", text)
        self.assertIn("never available to agents", text)

    def test_the_desktop_sets_the_grant_and_a_new_grant_ends_its_agent_lease(self):
        from kassiber import daemon

        ctx = SimpleNamespace(conn=None, db_passphrase=None, data_root=str(self.path.parent), agent_lease=None)
        with mock.patch.object(daemon.daemon_agent_session, "end_lease") as end_lease:
            status = daemon._agent_access_payload(
                ctx, "ui.agent_access.configure",
                {"args": {"mcp_enabled": True, "ai_features_enabled": True, "grant": "accounting_decisions",
                          "agent_access": "full"}},
            )
            self.assertEqual((status["grant"], status["agent_access"]), ("accounting_decisions", "full"))
            self.assertEqual(end_lease.call_count, 1)
            daemon._agent_access_payload(ctx, "ui.agent_access.configure", {"args": {"agent_access": "restricted"}})
            self.assertEqual(end_lease.call_count, 1, "an unchanged grant keeps the lease")
            for bad in ({"grant": "admin"}, {"grant": 2}, {"agent_access": True}):
                with self.subTest(bad=bad), self.assertRaises(AppError):
                    daemon._agent_access_payload(ctx, "ui.agent_access.configure", {"args": bad})
        read = daemon._agent_access_payload(ctx, "ui.agent_access.status", {"args": {}})
        self.assertEqual((read["grant"], read["agent_access"]), ("accounting_decisions", "restricted"))
