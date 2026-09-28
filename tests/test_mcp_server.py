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
from kassiber.operator.cli import route_brokered_command
from tests.integration.env import no_egress_guard


MODERN_META = {
    META_PROTOCOL_VERSION: "2026-07-28",
    META_CLIENT_CAPABILITIES: {},
}


class _FakeProvider:
    instructions = "fake instructions"

    def __init__(self) -> None:
        self.calls: list[tuple[str, dict]] = []
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

    def call_tool(self, name, arguments, cancelled):
        self.release.wait(5)
        self.calls.append((name, arguments))
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
    def test_every_exposed_tool_is_a_local_read_only_catalog_tool(self):
        from kassiber import daemon

        excluded_kinds = set(daemon._LOCAL_CUSTODY_READ_DAEMON_KINDS) | {
            "ui.workspace.overview.snapshot",
            "ui.profiles.snapshot",
            "ui.wallets.analyze_file",
            "ui.audit.changes_since_last_answer",
        }
        for entry in mcp_tools.mcp_entries():
            with self.subTest(tool=entry.name):
                self.assertIs(get_tool(entry.name), entry)
                self.assertEqual(entry.kind_class, "read_only")
                self.assertFalse(entry.egresses)
                self.assertFalse(entry.requires_consent)
                self.assertNotIn(entry.daemon_kind, excluded_kinds)
                self.assertFalse(entry.name.startswith(("ui.chain_analysis.", "ui.accounting.", "ui.custody.")))

    def test_definitions_are_portable_across_hosts(self):
        definitions = mcp_tools.tool_definitions()
        names = [definition["name"] for definition in definitions]
        self.assertEqual(len(names), len(set(names)))
        for definition in definitions:
            with self.subTest(tool=definition["name"]):
                # Hosts prefix server names (mcp__kassiber__) and some model
                # APIs cap tool names at 64 underscore/alnum characters.
                self.assertRegex(definition["name"], r"^[a-z0-9_]+$")
                self.assertLessEqual(len("mcp__kassiber__" + definition["name"]), 64)
                self.assertLessEqual(len(definition["description"]), mcp_tools.MAX_DESCRIPTION_CHARS)
                self.assertEqual(definition["inputSchema"].get("type"), "object")
                self.assertEqual(
                    definition["annotations"],
                    {
                        "title": definition["title"],
                        "readOnlyHint": True,
                        "destructiveHint": False,
                        "idempotentHint": True,
                        "openWorldHint": False,
                    },
                )
        self.assertLessEqual(len(mcp_tools.INSTRUCTIONS), mcp_tools.MAX_DESCRIPTION_CHARS)

    def test_no_mutating_or_egressing_catalog_tool_is_reachable(self):
        exposed = {entry.name for entry in mcp_tools.mcp_entries()}
        for entry in TOOL_CATALOG:
            if entry.kind_class == "mutating" or entry.egresses:
                self.assertNotIn(entry.name, exposed)


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
            outcome = self.provider().call_tool("status", {"x": 1}, threading.Event())
        self.assertFalse(outcome.is_error, outcome.structured)
        self.assertEqual(outcome.structured["data"], {"ok": 1})
        argv = captured["argv"]
        self.assertIsNone(captured["admin"])
        self.assertIs(captured["start_broker"], False)
        self.assertIs(captured["require_caller_context"], True)
        self.assertEqual(argv[argv.index("mcp") : argv.index("mcp") + 2], ["mcp", "call"])
        self.assertEqual(argv[argv.index("--workspace") + 1], "ws-id")
        self.assertEqual(argv[argv.index("--profile") + 1], "book-id")
        self.assertEqual(json.loads(argv[argv.index("--arguments") + 1]), {"x": 1})
        self.assertNotIn("--db-passphrase-fd", argv)

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
    def test_mcp_commands_are_catalogued_as_local_reads(self):
        catalog = describe_command_catalog(build_parser(), ["mcp"])
        by_command = {command["command"]: command for command in catalog["commands"]}
        self.assertEqual(set(by_command), {"mcp serve", "mcp tools", "mcp call"})
        self.assertEqual(by_command["mcp serve"]["effect"], "interactive")
        self.assertFalse(by_command["mcp serve"]["needs_database"])
        self.assertFalse(by_command["mcp tools"]["needs_database"])
        self.assertTrue(by_command["mcp call"]["needs_database"])
        for path in ("mcp.serve", "mcp.tools", "mcp.call"):
            self.assertIs(cli_capability(path), Capability.READ)

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
            self.assertEqual(len(responses[1]["result"]["tools"]), len(mcp_tools.mcp_entries()))
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
        definitions = {d["name"]: d for d in mcp_tools.tool_definitions()}
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
            _validate_ai_tool_arguments(mcp_tools.entry_for(name), {key: spec["exclusiveMinimum"]})


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
            BrokerClient, "ping", return_value={"generation": "old"}
        ), mock.patch.object(
            BrokerClient, "_submit_once", side_effect=AssertionError("must not submit")
        ):
            outcome = self.provider().call_tool("status", {}, threading.Event())
        self.assertEqual(outcome.structured["error"]["code"], "operator_broker_outdated")


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
