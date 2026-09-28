"""Dual-era Model Context Protocol server core over stdio (standard library only).

Modern clients (revision 2026-07-28) send self-contained requests that carry
their protocol version and capabilities in `_meta`; there is no handshake and
no session, and `server/discover` is mandatory. Legacy clients (2025-11-25 and
earlier) open with `initialize`, which selects legacy semantics for the rest of
this stdio process. That is the spec's dual-era server behavior: the era is
chosen per message from how the client speaks, never from a stored session.

This module knows nothing about Kassiber. A `ToolProvider` supplies static,
era-neutral tool definitions and executes calls.
"""

from __future__ import annotations

import json
import queue
import threading
from dataclasses import dataclass
from typing import Any, BinaryIO, Protocol


MODERN_PROTOCOL_VERSIONS = ("2026-07-28",)
# Newest first: an unknown legacy request is offered the first entry.
# 2025-03-26 is not offered: that revision requires JSON-RPC batches.
LEGACY_PROTOCOL_VERSIONS = ("2025-11-25", "2025-06-18")
# `structuredContent` and top-level tool `title` arrived in 2025-06-18.
_LEGACY_STRUCTURED_VERSIONS = frozenset({"2025-11-25", "2025-06-18"})

META_PROTOCOL_VERSION = "io.modelcontextprotocol/protocolVersion"
META_CLIENT_CAPABILITIES = "io.modelcontextprotocol/clientCapabilities"
META_SERVER_INFO = "io.modelcontextprotocol/serverInfo"
META_SUBSCRIPTION_ID = "io.modelcontextprotocol/subscriptionId"

PARSE_ERROR = -32700
INVALID_REQUEST = -32600
METHOD_NOT_FOUND = -32601
INVALID_PARAMS = -32602
INTERNAL_ERROR = -32603
UNSUPPORTED_PROTOCOL_VERSION = -32022
# JSON-RPC implementation-defined server error: too much work in flight.
SERVER_OVERLOADED = -32000

MAX_MESSAGE_BYTES = 16 * 1024 * 1024
# Tool definitions and instructions are static per server version and carry
# no book data, so any client or cache may keep them.
STATIC_LIST_TTL_MS = 3_600_000
# Calls run one at a time; a client pipelining more than this many is refused
# rather than queued without bound (each request may be megabytes).
MAX_INFLIGHT_CALLS = 32
# Open `subscriptions/listen` streams; each holds an entry until stdin ends.
MAX_SUBSCRIPTIONS = 16
# After stdin closes, accepted calls get this long to finish and be answered.
SHUTDOWN_DRAIN_SECONDS = 30.0


class McpError(Exception):
    """A JSON-RPC protocol error (not a tool failure, which is `isError`)."""

    def __init__(self, code: int, message: str, data: Any = None) -> None:
        super().__init__(message)
        self.code = code
        self.message = message
        self.data = data


@dataclass(frozen=True)
class ToolOutcome:
    structured: dict[str, Any]
    text: str
    is_error: bool = False


class ToolProvider(Protocol):
    instructions: str

    def tool_definitions(self) -> list[dict[str, Any]]:
        ...

    def has_tool(self, name: str) -> bool:
        ...

    def call_tool(
        self,
        name: str,
        arguments: dict[str, Any],
        cancelled: threading.Event,
    ) -> ToolOutcome:
        ...


@dataclass(frozen=True)
class _Era:
    modern: bool
    version: str | None


def _valid_request_id(value: Any) -> bool:
    return isinstance(value, str) or (
        isinstance(value, int) and not isinstance(value, bool)
    )


def _id_key(value: Any) -> str | None:
    # `1` and `"1"` are different JSON-RPC ids.
    return json.dumps(value) if _valid_request_id(value) else None


def _error_response(request_id: Any, code: int, message: str, data: Any = None) -> dict[str, Any]:
    error: dict[str, Any] = {"code": code, "message": message}
    if data is not None:
        error["data"] = data
    response: dict[str, Any] = {"jsonrpc": "2.0", "error": error}
    response["id"] = request_id if _valid_request_id(request_id) else None
    return response


class McpServer:
    def __init__(self, provider: ToolProvider, *, name: str, version: str) -> None:
        self._provider = provider
        self._server_info = {"name": name, "version": version}
        self._lock = threading.Lock()
        self._legacy_version: str | None = None

    # -- request handling -------------------------------------------------

    def handle(
        self,
        message: Any,
        cancelled: threading.Event | None = None,
    ) -> dict[str, Any] | None:
        """Return the response for one message, or None for notifications."""

        if not isinstance(message, dict) or message.get("jsonrpc") != "2.0":
            request_id = message.get("id") if isinstance(message, dict) else None
            return _error_response(request_id, INVALID_REQUEST, "Invalid JSON-RPC message")
        method = message.get("method")
        if not isinstance(method, str):
            if "id" in message and ("result" in message or "error" in message):
                # This server never issues requests, so replies are stray.
                return None
            return _error_response(message.get("id"), INVALID_REQUEST, "Invalid JSON-RPC message")
        if "id" not in message:
            return None
        request_id = message["id"]
        if not _valid_request_id(request_id):
            return _error_response(None, INVALID_REQUEST, "Invalid request id")
        params = message.get("params")
        if params is None:
            params = {}
        if not isinstance(params, dict):
            return _error_response(request_id, INVALID_PARAMS, "Request params must be an object")
        try:
            era = self._era_for(method, params)
            result = self._dispatch(method, params, era, cancelled)
        except McpError as exc:
            return _error_response(request_id, exc.code, exc.message, exc.data)
        except Exception:
            return _error_response(request_id, INTERNAL_ERROR, "Internal error")
        return {"jsonrpc": "2.0", "id": request_id, "result": result}

    def open_subscription(self, message: dict[str, Any]) -> dict[str, Any]:
        """Acknowledge a modern `subscriptions/listen` stream.

        Nothing here changes at runtime, so the honored filter is empty; the
        stream stays open until the client cancels it or stdin closes.
        """

        params = message.get("params")
        era = self._era_for("subscriptions/listen", params if isinstance(params, dict) else {})
        if not era.modern:
            raise McpError(METHOD_NOT_FOUND, "Method not found: subscriptions/listen")
        return {
            "jsonrpc": "2.0",
            "method": "notifications/subscriptions/acknowledged",
            "params": {
                "_meta": {META_SUBSCRIPTION_ID: message.get("id")},
                "notifications": {},
            },
        }

    def close_subscription(self, request_id: Any) -> dict[str, Any]:
        return {
            "jsonrpc": "2.0",
            "id": request_id,
            "result": {
                "resultType": "complete",
                "_meta": {
                    META_SUBSCRIPTION_ID: request_id,
                    META_SERVER_INFO: dict(self._server_info),
                },
            },
        }

    def _era_for(self, method: str, params: dict[str, Any]) -> _Era:
        meta = params.get("_meta")
        if isinstance(meta, dict) and META_PROTOCOL_VERSION in meta:
            version = meta.get(META_PROTOCOL_VERSION)
            capabilities = meta.get(META_CLIENT_CAPABILITIES)
            if not isinstance(version, str) or not isinstance(capabilities, dict):
                raise McpError(
                    INVALID_PARAMS,
                    "Request _meta requires protocolVersion and clientCapabilities",
                )
            if version not in MODERN_PROTOCOL_VERSIONS:
                raise McpError(
                    UNSUPPORTED_PROTOCOL_VERSION,
                    "Unsupported protocol version",
                    {"supported": list(MODERN_PROTOCOL_VERSIONS), "requested": version},
                )
            return _Era(modern=True, version=version)
        if method in {"initialize", "ping"}:
            # Legacy lifecycles allow ping at any time, even before initialize.
            return _Era(modern=False, version=None)
        with self._lock:
            legacy = self._legacy_version
        if legacy is None:
            raise McpError(
                INVALID_PARAMS,
                "Requests must carry protocolVersion and clientCapabilities in "
                "_meta, or follow an initialize handshake",
                {
                    "supported": list(MODERN_PROTOCOL_VERSIONS),
                    "legacy": list(LEGACY_PROTOCOL_VERSIONS),
                },
            )
        return _Era(modern=False, version=legacy)

    def _dispatch(
        self,
        method: str,
        params: dict[str, Any],
        era: _Era,
        cancelled: threading.Event | None,
    ) -> dict[str, Any]:
        if era.modern:
            if method == "server/discover":
                result: dict[str, Any] = {
                    "supportedVersions": list(MODERN_PROTOCOL_VERSIONS),
                    "capabilities": {"tools": {}},
                    "ttlMs": STATIC_LIST_TTL_MS,
                    "cacheScope": "public",
                }
                if self._provider.instructions:
                    result["instructions"] = self._provider.instructions
                return self._modern(result)
            if method == "tools/list":
                _reject_cursor(params)
                return self._modern(
                    {
                        "tools": self._tools(era),
                        "ttlMs": STATIC_LIST_TTL_MS,
                        "cacheScope": "public",
                    }
                )
            if method == "tools/call":
                return self._modern(self._call(params, era, cancelled))
            raise McpError(METHOD_NOT_FOUND, f"Method not found: {method}")
        if method == "initialize":
            return self._initialize(params)
        if method == "ping":
            return {}
        if method == "tools/list":
            _reject_cursor(params)
            return {"tools": self._tools(era)}
        if method == "tools/call":
            return self._call(params, era, cancelled)
        raise McpError(METHOD_NOT_FOUND, f"Method not found: {method}")

    def _modern(self, result: dict[str, Any]) -> dict[str, Any]:
        return {
            **result,
            "resultType": "complete",
            "_meta": {META_SERVER_INFO: dict(self._server_info)},
        }

    def _initialize(self, params: dict[str, Any]) -> dict[str, Any]:
        requested = params.get("protocolVersion")
        version = (
            requested
            if isinstance(requested, str) and requested in LEGACY_PROTOCOL_VERSIONS
            else LEGACY_PROTOCOL_VERSIONS[0]
        )
        with self._lock:
            self._legacy_version = version
        result: dict[str, Any] = {
            "protocolVersion": version,
            "capabilities": {"tools": {}},
            "serverInfo": dict(self._server_info),
        }
        if self._provider.instructions:
            result["instructions"] = self._provider.instructions
        return result

    def _tools(self, era: _Era) -> list[dict[str, Any]]:
        structured = era.modern or era.version in _LEGACY_STRUCTURED_VERSIONS
        tools = []
        for definition in self._provider.tool_definitions():
            tool = dict(definition)
            if not structured:
                tool.pop("title", None)
                tool.pop("outputSchema", None)
            tools.append(tool)
        return tools

    def _call(
        self,
        params: dict[str, Any],
        era: _Era,
        cancelled: threading.Event | None,
    ) -> dict[str, Any]:
        name = params.get("name")
        if not isinstance(name, str) or not self._provider.has_tool(name):
            raise McpError(INVALID_PARAMS, f"Unknown tool: {name}")
        arguments = params.get("arguments")
        if arguments is None:
            arguments = {}
        if not isinstance(arguments, dict):
            raise McpError(INVALID_PARAMS, "Tool arguments must be an object")
        outcome = self._provider.call_tool(name, arguments, cancelled or threading.Event())
        result: dict[str, Any] = {"content": [{"type": "text", "text": outcome.text}]}
        if era.modern or era.version in _LEGACY_STRUCTURED_VERSIONS:
            result["structuredContent"] = outcome.structured
        if outcome.is_error:
            result["isError"] = True
        return result


def _reject_constant(name: str) -> Any:
    # Python accepts NaN/Infinity, which are not JSON; refuse them at the edge.
    raise ValueError(f"invalid JSON constant {name}")


def _reject_cursor(params: dict[str, Any]) -> None:
    # Every list fits in one page, so no cursor this server issued exists.
    if params.get("cursor") is not None:
        raise McpError(INVALID_PARAMS, "Invalid cursor")


class StdioLoop:
    """Newline-delimited JSON-RPC over a byte stream pair.

    Fast methods are answered on the reader thread so the client keeps a
    responsive channel; tool calls run one at a time on a worker, matching the
    daemon's serialized database access. A cancelled call's response is
    dropped, as the spec requires.
    """

    def __init__(self, server: McpServer, reader: BinaryIO, writer: BinaryIO) -> None:
        self._server = server
        self._reader = reader
        self._writer = writer
        self._write_lock = threading.Lock()
        self._state_lock = threading.Lock()
        self._inflight: dict[str, threading.Event] = {}
        self._subscriptions: dict[str, Any] = {}
        self._calls: queue.Queue[tuple[dict[str, Any], str, threading.Event] | None] = queue.Queue()
        self._closed = threading.Event()
        self._worker = threading.Thread(
            target=self._run_calls,
            name="kassiber-mcp-tools",
            daemon=True,
        )

    def run(self) -> None:
        self._worker.start()
        try:
            while not self._closed.is_set():
                line = self._read_line()
                if line is None:
                    break
                if not line.strip():
                    continue
                try:
                    message = json.loads(line.decode("utf-8"), parse_constant=_reject_constant)
                except (UnicodeDecodeError, ValueError, RecursionError):
                    # Deeply nested JSON must not end the session.
                    self._write(_error_response(None, PARSE_ERROR, "Parse error"))
                    continue
                self._accept(message)
        finally:
            self._shutdown()

    def _read_line(self) -> bytes | None:
        line = self._reader.readline(MAX_MESSAGE_BYTES + 1)
        if not line:
            return None
        if len(line) > MAX_MESSAGE_BYTES and not line.endswith(b"\n"):
            while True:
                rest = self._reader.readline(MAX_MESSAGE_BYTES)
                if not rest or rest.endswith(b"\n"):
                    break
            self._write(_error_response(None, INVALID_REQUEST, "Message too large"))
            return b""
        return line

    def _accept(self, message: Any) -> None:
        if not isinstance(message, dict):
            self._write(_error_response(None, INVALID_REQUEST, "Invalid JSON-RPC message"))
            return
        method = message.get("method")
        if method == "notifications/cancelled" and "id" not in message:
            params = message.get("params")
            key = _id_key(params.get("requestId")) if isinstance(params, dict) else None
            with self._state_lock:
                event = self._inflight.get(key) if key else None
                self._subscriptions.pop(key, None)
            if event is not None:
                event.set()
            return
        key = _id_key(message.get("id")) if "id" in message else None
        if method == "tools/call" and key is not None:
            event = threading.Event()
            with self._state_lock:
                duplicate = key in self._inflight
                overloaded = not duplicate and len(self._inflight) >= MAX_INFLIGHT_CALLS
                if not duplicate and not overloaded:
                    self._inflight[key] = event
            if overloaded:
                self._write(
                    _error_response(
                        message.get("id"),
                        SERVER_OVERLOADED,
                        "Too many tool calls in flight; retry after earlier calls finish",
                        {"limit": MAX_INFLIGHT_CALLS},
                    )
                )
                return
            if duplicate:
                self._write(
                    _error_response(message.get("id"), INVALID_REQUEST, "Request id is already in flight")
                )
                return
            self._calls.put((message, key, event))
            return
        if method == "subscriptions/listen" and key is not None and message.get("jsonrpc") == "2.0":
            # A malformed envelope falls through to ordinary handling, which
            # answers it with the protocol's error instead of a stream.
            with self._state_lock:
                full = len(self._subscriptions) >= MAX_SUBSCRIPTIONS
            if full:
                self._write(
                    _error_response(
                        message.get("id"),
                        SERVER_OVERLOADED,
                        "Too many open subscriptions",
                        {"limit": MAX_SUBSCRIPTIONS},
                    )
                )
                return
            try:
                acknowledgement = self._server.open_subscription(message)
            except McpError as exc:
                self._write(_error_response(message.get("id"), exc.code, exc.message, exc.data))
                return
            with self._state_lock:
                self._subscriptions[key] = message.get("id")
            self._write(acknowledgement)
            return
        response = self._server.handle(message)
        if response is not None:
            self._write(response)

    def _run_calls(self) -> None:
        while True:
            item = self._calls.get()
            if item is None:
                return
            message, key, event = item
            try:
                response = None if event.is_set() else self._server.handle(message, event)
            except Exception:
                # Never leave a call unanswered or the worker dead.
                response = _error_response(message.get("id"), INTERNAL_ERROR, "Internal error")
            finally:
                with self._state_lock:
                    self._inflight.pop(key, None)
            if response is not None and not event.is_set():
                self._write(response)

    def _write(self, payload: dict[str, Any]) -> None:
        # json.dumps escapes control characters, so one message is one line.
        # ASCII escapes keep one message on one line and cannot fail to encode
        # (a lone surrogate from book text would otherwise kill the worker).
        data = json.dumps(payload, ensure_ascii=True, separators=(",", ":")).encode("ascii")
        with self._write_lock:
            if self._closed.is_set():
                return
            try:
                self._writer.write(data + b"\n")
                self._writer.flush()
            except (BrokenPipeError, OSError, ValueError):
                self._closed.set()

    def _shutdown(self) -> None:
        # EOF ends input, not the calls the client already sent: answer them
        # within a bounded wait, then close subscriptions and exit. Calls the
        # client cancelled explicitly still get no reply.
        self._calls.put(None)
        self._worker.join(timeout=SHUTDOWN_DRAIN_SECONDS)
        with self._state_lock:
            pending = list(self._inflight.values())
            subscriptions = list(self._subscriptions.values())
            self._subscriptions.clear()
        for event in pending:
            event.set()
        for request_id in subscriptions:
            self._write(self._server.close_subscription(request_id))
        self._closed.set()


def serve(server: McpServer, reader: BinaryIO, writer: BinaryIO) -> None:
    StdioLoop(server, reader, writer).run()
