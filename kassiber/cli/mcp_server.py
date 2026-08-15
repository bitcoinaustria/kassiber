from __future__ import annotations

import asyncio
import json
import logging
import threading
import uuid
from typing import Any, Callable

import mcp.server.stdio
import mcp_types as types
from mcp.server import Server, ServerRequestContext

from .. import __version__
from ..ai.prompt import DEFAULT_KASSIBER_SYSTEM_PROMPT
from ..ai.tools import (
    CORE_TOOL_NAMES,
    ToolEntry,
    external_read_tool_entries,
    get_tool,
    redact_ai_tool_result,
)
from ..errors import AppError
from .chat import DaemonClient


ToolCall = Callable[[str, dict[str, Any]], dict[str, Any]]
_LOGGER = logging.getLogger(__name__)
_EXTERNAL_TOOL_ENTRIES = external_read_tool_entries()
_EXTERNAL_TOOL_NAMES = frozenset(entry.provider_name for entry in _EXTERNAL_TOOL_ENTRIES)


def _tool_entries(profile: str) -> tuple[ToolEntry, ...]:
    if profile == "core":
        return tuple(
            entry for entry in _EXTERNAL_TOOL_ENTRIES if entry.name in CORE_TOOL_NAMES
        )
    if profile == "full":
        return _EXTERNAL_TOOL_ENTRIES
    raise AppError("unknown MCP tool profile", code="validation", retryable=False)


class McpDaemonAdapter:
    """Serialize MCP calls onto one private daemon connection."""

    def __init__(self, args: Any) -> None:
        self._args = args
        self._client: DaemonClient | None = None
        self._lock = threading.Lock()

    def _daemon(self) -> DaemonClient:
        if self._client is None:
            self._client = DaemonClient(self._args)
        return self._client

    def call_tool(self, name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        entry = get_tool(name)
        if entry is None or entry.provider_name not in _EXTERNAL_TOOL_NAMES:
            return {"ok": False, "reason": "tool_not_allowed"}

        request_id = f"mcp-{uuid.uuid4()}"
        with self._lock:
            client = self._daemon()
            client.send(
                {
                    "kind": "ai.tool.read",
                    "request_id": request_id,
                    "args": {
                        "name": entry.provider_name,
                        "arguments": arguments,
                    },
                }
            )
            while True:
                response = client.read()
                if response.get("request_id") == request_id:
                    break
                if response.get("event") is True:
                    continue
                raise AppError(
                    "daemon returned an unrelated response",
                    code="daemon_protocol_error",
                    retryable=False,
                )

        error = response.get("error")
        if isinstance(error, dict):
            raise AppError(
                str(error.get("message") or "Kassiber tool call failed"),
                code=str(error.get("code") or "tool_error"),
                hint=error.get("hint") if isinstance(error.get("hint"), str) else None,
                retryable=bool(error.get("retryable", False)),
            )
        if response.get("kind") != "ai.tool.read":
            raise AppError(
                "daemon returned an unexpected tool response",
                code="daemon_protocol_error",
                retryable=False,
            )
        data = response.get("data")
        result = data.get("result") if isinstance(data, dict) else None
        if not isinstance(result, dict):
            raise AppError(
                "daemon tool response has no result",
                code="daemon_protocol_error",
                retryable=False,
            )
        return redact_ai_tool_result(result)

    def close(self) -> None:
        with self._lock:
            if self._client is not None:
                self._client.close()
                self._client = None


def _call_result(payload: dict[str, Any]) -> types.CallToolResult:
    safe = redact_ai_tool_result(payload)
    return types.CallToolResult(
        content=[
            types.TextContent(
                type="text",
                text=json.dumps(safe, sort_keys=True, separators=(",", ":")),
            )
        ],
        structured_content=safe,
        is_error=safe.get("ok") is not True,
    )


def create_mcp_server(call_tool: ToolCall, *, tool_profile: str = "core") -> Server:
    entries = _tool_entries(tool_profile)
    entry_names = frozenset(entry.provider_name for entry in entries)

    async def list_tools(
        _ctx: ServerRequestContext,
        _params: types.PaginatedRequestParams | None,
    ) -> types.ListToolsResult:
        return types.ListToolsResult(
            tools=[
                types.Tool(
                    name=entry.provider_name,
                    description=entry.description,
                    input_schema=entry.parameters,
                    annotations=types.ToolAnnotations(
                        read_only_hint=True,
                        destructive_hint=False,
                    ),
                )
                for entry in entries
            ]
        )

    async def handle_call(
        _ctx: ServerRequestContext,
        params: types.CallToolRequestParams,
    ) -> types.CallToolResult:
        if params.name not in entry_names:
            return _call_result({"ok": False, "reason": "tool_not_allowed"})
        try:
            payload = await asyncio.to_thread(
                call_tool,
                params.name,
                dict(params.arguments or {}),
            )
        except AppError as exc:
            payload = {
                "ok": False,
                "reason": exc.code or "tool_error",
                "message": str(exc),
            }
        except Exception:
            _LOGGER.exception("external MCP tool call failed")
            payload = {
                "ok": False,
                "reason": "tool_error",
                "message": "Kassiber tool execution failed unexpectedly",
            }
        return _call_result(payload)

    return Server(
        "kassiber",
        version=__version__,
        description="Read-only access to redacted Kassiber accounting context.",
        instructions=(
            DEFAULT_KASSIBER_SYSTEM_PROMPT
            + "\nThis external MCP server exposes only read-only, non-egressing tools. "
            "Use read_skill_reference(name='index') when workflow detail is needed."
        ),
        on_list_tools=list_tools,
        on_call_tool=handle_call,
    )


async def _serve(args: Any, adapter: McpDaemonAdapter) -> None:
    server = create_mcp_server(
        adapter.call_tool,
        tool_profile=str(getattr(args, "tool_profile", "core")),
    )
    async with mcp.server.stdio.stdio_server() as (read_stream, write_stream):
        await server.run(
            read_stream,
            write_stream,
            server.create_initialization_options(),
        )


def run_mcp_server(args: Any) -> None:
    adapter = McpDaemonAdapter(args)
    try:
        asyncio.run(_serve(args, adapter))
    finally:
        adapter.close()
