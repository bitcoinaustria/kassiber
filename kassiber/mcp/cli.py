"""`kassiber mcp serve|tools|call`: the local MCP server and its one-shot call.

Every tool call is a finite, self-contained operation, which is what lets the
server stay stateless and still work on encrypted books:

- plaintext and `unattended` books open like an ordinary non-interactive CLI
  command (never prompting) and close again after the call;
- `brokered` books submit `kassiber mcp call` to the operator broker, so the
  lease's capability and scope checks apply and this process never sees the
  passphrase;
- `manual` encrypted books return `interaction_required` with the command the
  user runs in their own terminal.

Launch does nothing: no path resolution, database open, lock, or socket until
the first `tools/call`.
"""

from __future__ import annotations

import argparse
import io
import json
import os
import sys
import threading
import time
from pathlib import Path
from typing import Any

from .. import __version__
from ..errors import AppError
from .protocol import McpServer, ToolOutcome, serve
from . import tools as mcp_tools


_BROKER_POLL_SECONDS = 0.1


def add_mcp_parser(subparsers: argparse._SubParsersAction) -> None:
    mcp = subparsers.add_parser(
        "mcp",
        help="Serve Kassiber's read-only tools to external agents over MCP (stdio)",
    )
    commands = mcp.add_subparsers(dest="mcp_command", required=True)
    serve_parser = commands.add_parser(
        "serve",
        help="Run a local stdio MCP server (MCP 2026-07-28 and 2025 handshake clients)",
    )
    _add_book_arguments(serve_parser)
    commands.add_parser("tools", help="Describe the tools the MCP server exposes")
    call = commands.add_parser("call", help="Run one MCP tool against a book and print its result")
    call.add_argument("--tool", required=True, help="MCP tool name from `kassiber mcp tools`")
    call.add_argument(
        "--arguments",
        default="{}",
        metavar="JSON",
        help="Tool arguments as a JSON object (not secret-bearing)",
    )
    _add_book_arguments(call)


def _add_book_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument(
        "--workspace",
        default=None,
        help="Book set (workspace) id or label to pin; defaults to the active one per call",
    )
    parser.add_argument(
        "--profile",
        default=None,
        help="Book (profile) id or label to pin; defaults to the active one per call",
    )


def dispatch_mcp(conn: Any, args: argparse.Namespace) -> Any:
    from ..envelope import emit

    if args.mcp_command in {"serve", "call"}:
        _require_complete_pin(args)
    if args.mcp_command == "serve":
        return run_server(args)
    if args.mcp_command == "tools":
        return emit(args, describe_tools(), kind="mcp.tools")
    if args.mcp_command == "call":
        arguments = _parse_arguments(args.arguments)
        result = mcp_tools.run_tool(
            conn,
            data_root=args.data_root,
            runtime_config=args.runtime_config,
            name=args.tool,
            arguments=arguments,
            workspace=args.workspace,
            profile=args.profile,
            project_name=_project_name(args),
        )
        return emit(args, result, kind="mcp.call")
    raise AppError("unknown mcp command", code="unknown_command")


def _require_complete_pin(args: argparse.Namespace) -> None:
    # A half pin resolves against whichever workspace or profile is active at
    # call time, so it could silently read a different book. Pin both or none.
    if bool(args.workspace) != bool(args.profile):
        raise AppError(
            "pin a book with both --workspace and --profile, or with neither",
            code="validation",
            hint="Use `kassiber workspaces list` and `kassiber profiles list` to find both.",
            retryable=False,
        )


def describe_tools() -> dict[str, Any]:
    from .protocol import LEGACY_PROTOCOL_VERSIONS, MODERN_PROTOCOL_VERSIONS

    return {
        "transport": "stdio",
        "protocol_versions": {
            "modern": list(MODERN_PROTOCOL_VERSIONS),
            "legacy_initialize": list(LEGACY_PROTOCOL_VERSIONS),
        },
        "instructions": mcp_tools.INSTRUCTIONS,
        "tools": mcp_tools.tool_definitions(),
    }


def _parse_arguments(raw: str) -> dict[str, Any]:
    try:
        value = json.loads(raw)
    except ValueError:
        raise AppError(
            "--arguments must be a JSON object",
            code="validation",
            retryable=False,
        ) from None
    if not isinstance(value, dict):
        raise AppError("--arguments must be a JSON object", code="validation", retryable=False)
    return value


def _project_name(args: argparse.Namespace) -> str | None:
    from ..projects import project_metadata_for_data_root

    metadata = project_metadata_for_data_root(args.data_root)
    name = metadata.get("name") if isinstance(metadata, dict) else None
    return name if isinstance(name, str) else None


# -- tool provider ---------------------------------------------------------


class BookToolProvider:
    """Execute MCP calls against the configured project, one call at a time."""

    instructions = mcp_tools.INSTRUCTIONS

    def __init__(
        self,
        *,
        data_root: str | None,
        project: str | None,
        env_file: str | None,
        workspace: str | None,
        profile: str | None,
    ) -> None:
        self._data_root = data_root
        self._project = project
        self._env_file = env_file
        self._workspace = workspace
        self._profile = profile

    def tool_definitions(self) -> list[dict[str, Any]]:
        return mcp_tools.tool_definitions()

    def has_tool(self, name: str) -> bool:
        return mcp_tools.entry_for(name) is not None

    def call_tool(
        self,
        name: str,
        arguments: dict[str, Any],
        cancelled: threading.Event,
    ) -> ToolOutcome:
        try:
            structured = self._call(name, arguments, cancelled)
        except AppError as exc:
            return _error_outcome(exc)
        except Exception:  # noqa: BLE001 - never echo raw exception text
            return _error_outcome(
                AppError("The Kassiber tool failed unexpectedly", code="tool_error")
            )
        text = json.dumps(structured, ensure_ascii=False, separators=(",", ":"))
        if len(text) > mcp_tools.MAX_RESULT_CHARS:
            return _error_outcome(
                AppError(
                    "The result is too large to return to an agent",
                    code="result_too_large",
                    hint="Narrow the request with the tool's limit or filter arguments.",
                )
            )
        return ToolOutcome(structured=structured, text=text)

    def _call(
        self,
        name: str,
        arguments: dict[str, Any],
        cancelled: threading.Event,
    ) -> dict[str, Any]:
        from ..core.runtime import resolve_runtime_paths
        from ..operator.modes import effective_unlock_mode
        from ..projects import load_catalog

        if self._data_root is None and not load_catalog().get("projects"):
            # Resolving the default project would create one; an agent's read
            # must not set up Kassiber on a machine where nothing exists yet.
            raise AppError(
                "No Kassiber project exists on this machine yet",
                code="not_initialized",
                hint="Ask the user to set Kassiber up first (desktop app or `kassiber init`).",
            )
        paths = resolve_runtime_paths(self._data_root, self._env_file, self._project)
        database = Path(paths.database)
        if not database.exists() or database.stat().st_size == 0:
            # Never let an agent's read create a new (plaintext) book.
            raise AppError(
                "No Kassiber book exists in the selected project yet",
                code="not_initialized",
                hint="Ask the user to set Kassiber up first (desktop app or `kassiber init`).",
            )
        if effective_unlock_mode(paths.data_root) == "brokered":
            return self._call_brokered(paths, name, arguments, cancelled)
        return self._call_direct(paths, name, arguments)

    def _call_direct(self, paths: Any, name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        from ..core.runtime import bootstrap_runtime, close_runtime

        args = argparse.Namespace(
            data_root=paths.data_root,
            env_file=self._env_file,
            project=None,
            non_interactive=True,
            db_passphrase_fd=None,
        )
        runtime = bootstrap_runtime(args, needs_db=True)
        try:
            return mcp_tools.run_tool(
                runtime.conn,
                data_root=paths.data_root,
                runtime_config=runtime.runtime_config,
                name=name,
                arguments=arguments,
                workspace=self._workspace,
                profile=self._profile,
                project_name=paths.project_name,
            )
        finally:
            close_runtime(runtime)

    def _call_brokered(
        self,
        paths: Any,
        name: str,
        arguments: dict[str, Any],
        cancelled: threading.Event,
    ) -> dict[str, Any]:
        from ..operator.client import (
            TERMINAL_OPERATION_STATES,
            BrokerClient,
            prepare_arguments,
            wipe_prepared,
        )

        client = BrokerClient()
        # Status never starts a broker; without a lease there is nothing to
        # submit to, so do not spawn one just to be refused.
        status = client.status(paths.data_root)
        if status.get("lease") != "unlocked":
            raise AppError(
                "this project has no active operator lease",
                code="interaction_required",
                hint=(
                    "Ask the user to run `kassiber operator unlock --capability read` "
                    "in their own terminal, then retry. Never ask for the passphrase."
                ),
                details={
                    "reason": "operator_lease_required",
                    "unlock_mode": "brokered",
                    "user_command": "kassiber operator unlock --capability read",
                },
                retryable=True,
            )
        default_scope = status.get("default_scope")
        default_scope = default_scope if isinstance(default_scope, dict) else {}
        workspace = self._workspace or default_scope.get("workspace")
        profile = self._profile or default_scope.get("profile")
        if not workspace or not profile:
            raise AppError(
                "a brokered book needs an explicit book scope",
                code="operator_scope_required",
                hint="Start the server with `kassiber mcp serve --workspace ... --profile ...`.",
                retryable=False,
            )
        argv = ["--data-root", paths.data_root]
        if self._env_file:
            argv += ["--env-file", self._env_file]
        argv += [
            "--machine",
            "mcp",
            "call",
            "--tool",
            name,
            "--arguments",
            json.dumps(arguments, separators=(",", ":")),
            "--workspace",
            str(workspace),
            "--profile",
            str(profile),
        ]
        prepared = prepare_arguments(argv, stdin=io.BytesIO())
        try:
            accepted = client.submit(paths.data_root, prepared, admin_authentication=None)
        finally:
            wipe_prepared(prepared)
        operation_id = accepted.get("operation_id")
        if not isinstance(operation_id, str):
            raise AppError("broker did not return an operation id", code="operator_protocol_error")
        completed = accepted
        while completed.get("state") not in TERMINAL_OPERATION_STATES:
            if cancelled.is_set():
                # Queued work is cancelled; a running read simply finishes.
                client.cancel(operation_id)
                raise AppError("the tool call was cancelled", code="cancelled")
            time.sleep(_BROKER_POLL_SECONDS)
            completed = client.operation_status(operation_id)
        if "stdout" not in completed and "output_error" not in completed and accepted is completed:
            # A replayed id can be answered terminal at admission, without output.
            completed = client.operation_status(operation_id)
        return _brokered_result(operation_id, completed)


def _brokered_result(operation_id: str, completed: dict[str, Any]) -> dict[str, Any]:
    output_error = completed.get("output_error")
    if isinstance(output_error, dict):
        raise AppError(
            str(output_error.get("message") or "The operator result output is unavailable."),
            code=str(output_error.get("code") or "operator_output_unavailable"),
            hint="Narrow the request with the tool's limit or filter arguments.",
        )
    stdout = completed.get("stdout")
    envelope = None
    if isinstance(stdout, str) and stdout.strip():
        try:
            envelope = json.loads(stdout)
        except ValueError:
            envelope = None
    if isinstance(envelope, dict) and envelope.get("kind") == "mcp.call":
        data = envelope.get("data")
        if isinstance(data, dict):
            return data
    if isinstance(envelope, dict) and isinstance(envelope.get("error"), dict):
        error = envelope["error"]
        raise AppError(
            str(error.get("message") or "The Kassiber tool did not complete"),
            code=str(error.get("code") or "tool_error"),
            hint=error.get("hint") if isinstance(error.get("hint"), str) else None,
            details=error.get("details") if isinstance(error.get("details"), dict) else None,
            retryable=bool(error.get("retryable")),
        )
    state = completed.get("state")
    raise AppError(
        f"the brokered tool call ended as {state}",
        code=f"operator_operation_{state}" if isinstance(state, str) else "operator_error",
        hint="Check `kassiber operator status`; if the lease ended, ask the user to unlock again.",
        details={"operation_id": operation_id, "state": state},
        retryable=state == "cancelled",
    )


def _error_outcome(exc: AppError) -> ToolOutcome:
    from ..ai.tools import redact_ai_tool_result

    error: dict[str, Any] = {
        "code": exc.code or "tool_error",
        "message": str(exc),
        "retryable": bool(exc.retryable),
    }
    if exc.hint:
        error["hint"] = exc.hint
    if isinstance(exc.details, dict):
        error["details"] = exc.details
    structured = redact_ai_tool_result({"error": error})
    text = f"{structured['error']['code']}: {structured['error']['message']}"
    if structured["error"].get("hint"):
        text += f"\nhint: {structured['error']['hint']}"
    return ToolOutcome(structured=structured, text=text, is_error=True)


# -- stdio server ----------------------------------------------------------


def run_server(args: argparse.Namespace) -> int:
    from ..log_ring import install_ring_logging

    # Logs stay in the bounded RAM ring. The client may persist our stderr,
    # so nothing (tracebacks included) is written there.
    install_ring_logging()
    protocol_in, protocol_out = _claim_stdio()
    previous_stderr = sys.stderr
    sys.stderr = open(os.devnull, "w", encoding="utf-8")
    provider = BookToolProvider(
        data_root=getattr(args, "data_root", None),
        project=getattr(args, "project", None),
        env_file=getattr(args, "env_file", None),
        workspace=args.workspace,
        profile=args.profile,
    )
    try:
        serve(McpServer(provider, name="kassiber", version=__version__), protocol_in, protocol_out)
    finally:
        sys.stderr.close()
        sys.stderr = previous_stderr
    return 0


def _claim_stdio() -> tuple[Any, Any]:
    """Move the protocol onto private descriptors.

    Nothing else in the process can then read protocol input (fd 0 becomes
    /dev/null) or corrupt protocol output: stray writes to fd 1 or
    `sys.stdout` are discarded instead of reaching the client.
    """

    sys.stdout.flush()
    in_fd = os.dup(0)
    out_fd = os.dup(1)
    null_fd = os.open(os.devnull, os.O_RDWR)
    try:
        os.dup2(null_fd, 0)
        os.dup2(null_fd, 1)
        # The host may persist our stderr; C extensions (SQLCipher) write to
        # fd 2 directly, so silence it too. Logs live in the RAM ring.
        os.dup2(null_fd, 2)
    finally:
        os.close(null_fd)
    sys.stdin = open(os.devnull, encoding="utf-8")
    sys.stdout = open(os.devnull, "w", encoding="utf-8")
    return os.fdopen(in_fd, "rb"), os.fdopen(out_fd, "wb")
