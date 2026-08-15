import asyncio
import os
from pathlib import Path
import queue
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from mcp import Client
from mcp.client.stdio import StdioServerParameters, stdio_client

from kassiber.cli.mcp_server import (
    McpDaemonAdapter,
    _tool_entries,
    create_mcp_server,
)
from kassiber.daemon import AiToolRuntime, ParsedAiToolCall, _execute_read_only_ai_tool


class McpServerTests(unittest.TestCase):
    def test_external_catalog_fails_closed(self):
        entries = _tool_entries("full")
        names = {entry.provider_name for entry in entries}

        self.assertTrue(all(entry.kind_class == "read_only" for entry in entries))
        self.assertTrue(all(not entry.requires_consent for entry in entries))

        self.assertIn("status", names)
        self.assertIn("ui_reports_summary", names)
        self.assertIn("read_skill_reference", names)
        self.assertNotIn("ui_wallets_sync", names)
        self.assertNotIn("ui_connections_node_snapshot", names)
        self.assertNotIn("ui_wallets_analyze_file", names)
        self.assertNotIn("ui_custody_lineage_snapshot", names)

    def test_official_mcp_protocol_lists_and_calls_tools(self):
        calls = []

        def call_tool(name, arguments):
            calls.append((name, arguments))
            return {
                "ok": True,
                "envelope": {"kind": name, "data": {"ready": True}},
            }

        async def exercise():
            server = create_mcp_server(call_tool, tool_profile="core")
            async with Client(server) as client:
                listed = await client.list_tools()
                status = next(tool for tool in listed.tools if tool.name == "status")
                self.assertTrue(status.annotations.read_only_hint)
                self.assertFalse(status.annotations.destructive_hint)
                self.assertNotIn(
                    "ui_activity_history",
                    {tool.name for tool in listed.tools},
                )

                result = await client.call_tool("status", {})
                self.assertFalse(result.is_error)
                self.assertEqual(result.structured_content["envelope"]["kind"], "status")

                denied = await client.call_tool("ui_wallets_sync", {})
                self.assertTrue(denied.is_error)
                self.assertEqual(
                    denied.structured_content,
                    {"ok": False, "reason": "tool_not_allowed"},
                )

        asyncio.run(exercise())
        self.assertEqual(calls, [("status", {})])

    def test_external_report_read_disables_automatic_network_sync(self):
        connection = sqlite3.connect(":memory:")
        runtime = AiToolRuntime(
            data_root="/tmp/kassiber-mcp-test",
            runtime_config={},
            main_thread_tasks=queue.Queue(),
            maintenance_state={
                "external_no_egress": True,
                "advertised_tools": ["ui_reports_summary"],
            },
        )
        call = ParsedAiToolCall(
            call_id="report-1",
            name="ui_reports_summary",
            arguments={},
        )
        try:
            with patch(
                "kassiber.daemon._run_scoped_ai_operation",
                side_effect=lambda _runtime, callback: callback(connection),
            ), patch(
                "kassiber.daemon._reports_summary_payload",
                return_value={"summary": {}},
            ), patch(
                "kassiber.daemon._auto_maintain_for_read",
                return_value={},
            ) as maintain:
                result = _execute_read_only_ai_tool(call, runtime)
        finally:
            connection.close()

        self.assertTrue(result["ok"])
        self.assertFalse(maintain.call_args.kwargs["sync_if_enabled"])

    def test_daemon_adapter_preserves_typed_request(self):
        class FakeDaemonClient:
            instance = None

            def __init__(self, _args):
                self.sent = []
                self.closed = False
                FakeDaemonClient.instance = self

            def send(self, payload):
                self.sent.append(payload)

            def read(self):
                request = self.sent[-1]
                return {
                    "kind": "ai.tool.read",
                    "request_id": request["request_id"],
                    "data": {
                        "result": {
                            "ok": True,
                            "envelope": {"kind": "status", "data": {}},
                        }
                    },
                }

            def close(self):
                self.closed = True

        with patch("kassiber.cli.mcp_server.DaemonClient", FakeDaemonClient):
            adapter = McpDaemonAdapter(SimpleNamespace())
            result = adapter.call_tool("status", {})
            denied = adapter.call_tool("ui_wallets_sync", {})
            adapter.close()

        client = FakeDaemonClient.instance
        self.assertTrue(result["ok"])
        self.assertEqual(denied, {"ok": False, "reason": "tool_not_allowed"})
        self.assertEqual(client.sent[0]["kind"], "ai.tool.read")
        self.assertEqual(
            client.sent[0]["args"],
            {"name": "status", "arguments": {}},
        )
        self.assertTrue(client.closed)

    def test_cli_stdio_server_end_to_end(self):
        async def exercise(data_root):
            parameters = StdioServerParameters(
                command=sys.executable,
                args=[
                    "-m",
                    "kassiber",
                    "--data-root",
                    str(data_root),
                    "mcp",
                ],
                cwd=str(Path(__file__).resolve().parents[1]),
                env=dict(os.environ),
            )
            async with Client(stdio_client(parameters)) as client:
                listed = await client.list_tools()
                names = {tool.name for tool in listed.tools}
                self.assertIn("ui_reports_summary", names)
                self.assertNotIn("ui_wallets_sync", names)

                result = await client.call_tool("status", {})
                self.assertFalse(result.is_error)
                self.assertEqual(
                    result.structured_content["envelope"]["kind"],
                    "status",
                )

                report = await client.call_tool("ui_reports_summary", {})
                self.assertFalse(report.is_error)
                self.assertEqual(
                    report.structured_content["envelope"]["kind"],
                    "ui.reports.summary",
                )

        with tempfile.TemporaryDirectory(prefix="kassiber-mcp-") as tmp:
            data_root = Path(tmp) / "data"
            for command in (
                ("init",),
                ("workspaces", "create", "Demo"),
                ("profiles", "create", "Main"),
                ("journals", "process"),
            ):
                prepared = subprocess.run(
                    [
                        sys.executable,
                        "-m",
                        "kassiber",
                        "--data-root",
                        str(data_root),
                        "--machine",
                        *command,
                    ],
                    cwd=Path(__file__).resolve().parents[1],
                    capture_output=True,
                    text=True,
                    check=False,
                )
                self.assertEqual(prepared.returncode, 0, prepared.stderr)
            asyncio.run(exercise(data_root))


if __name__ == "__main__":
    unittest.main()
