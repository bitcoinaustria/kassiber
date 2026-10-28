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
    from ..agent_access import AGENT_ACCESS_MODES, GRANTS

    mcp = subparsers.add_parser(
        "mcp",
        help="Serve Kassiber's tools to external agents over MCP (stdio), within your grant",
    )
    commands = mcp.add_subparsers(dest="mcp_command", required=True)
    serve_parser = commands.add_parser(
        "serve",
        help="Run a local stdio MCP server (MCP 2026-07-28 and 2025 handshake clients)",
    )
    _add_book_arguments(serve_parser)
    commands.add_parser("tools", help="Describe the tools the MCP server exposes at the current grant")
    commands.add_parser("status", help="Show whether external agents may use Kassiber, and their grant")
    enable = commands.add_parser(
        "enable",
        help="Allow external agents to use Kassiber books (asks in your terminal)",
    )
    enable.add_argument(
        "--grant",
        choices=GRANTS,
        default=None,
        help="What agents may do: read, operator (also import, sync, process, export) or "
        "accounting_decisions (also review and decide); keeps the recorded grant if omitted, "
        "read for a first enable",
    )
    enable.add_argument(
        "--agent-access",
        choices=AGENT_ACCESS_MODES,
        default=None,
        help="full (unprojected results and custody/lineage/chain-analysis source tools) or "
        "restricted; keeps the recorded choice if omitted, restricted for a first enable",
    )
    commands.add_parser("disable", help="Stop serving external agents")
    call = commands.add_parser("call", help="Run one MCP tool against a book and print its result")
    call.add_argument("--tool", required=True, help="MCP tool name from `kassiber mcp tools`")
    call.add_argument(
        "--arguments",
        default="{}",
        metavar="JSON",
        help="Tool arguments as a JSON object (not secret-bearing)",
    )
    call.add_argument(
        "--grant",
        choices=GRANTS,
        default=None,
        help="Run with at most this grant; narrows the user's External agents grant, never widens it",
    )
    call.add_argument(
        "--client",
        default=None,
        metavar="NAME",
        help="Agent host name recorded with writes (display only)",
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
    if args.mcp_command in {"status", "enable", "disable"}:
        return emit(args, _configure_access(args), kind=f"mcp.{args.mcp_command}")
    if args.mcp_command == "call":
        arguments = _parse_arguments(args.arguments)
        grant, agent_access = _call_access(args)
        result = mcp_tools.run_tool(
            conn,
            data_root=args.data_root,
            runtime_config=args.runtime_config,
            name=args.tool,
            arguments=arguments,
            workspace=args.workspace,
            profile=args.profile,
            project_name=_project_name(args),
            grant=grant,
            agent_access=agent_access,
            client_name=args.client,
        )
        return emit(args, result, kind="mcp.call")
    raise AppError("unknown mcp command", code="unknown_command")


def _call_access(args: argparse.Namespace) -> tuple[str, str]:
    """The grant one `mcp call` runs with: the user's, only ever narrowed.

    Inside an operator broker worker the call may use no more than the broker
    admitted for it (`call_capability`, which the lease had to cover), so a
    read under a `read` lease never refreshes journals.
    """

    from ..agent_access import narrower_grant, require_mcp_access

    status = require_mcp_access()
    grant = status["grant"]
    if os.environ.get("KASSIBER_OPERATOR_CHILD") == "1":
        admitted = mcp_tools.call_capability(args.tool, args.grant).value
        grant = narrower_grant(grant, admitted)
    elif args.grant is not None:
        grant = narrower_grant(grant, args.grant)
    return grant, status["agent_access"]


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


_GRANT_CONSENT_TEXT = {
    "read": "read this machine's Kassiber books (read-only, no network)",
    "operator": (
        "read and operate this machine's Kassiber books: import, process journals, "
        "edit notes and tags, export, and sync wallets and rates, which contacts "
        "your configured backends and rate sources"
    ),
    "accounting_decisions": (
        "read, operate and make accounting decisions in this machine's Kassiber "
        "books: everything operator allows (including wallet and rate syncs that "
        "contact your backends) plus resolving quarantine, pairing and dismissing "
        "transfers, custody and source-of-funds reviews, exclusions and "
        "classification"
    ),
}
_ACCESS_CONSENT_TEXT = {
    "full": (
        "with full access: unprojected results (txids, outpoints, labels) and the "
        "custody-gap, lineage and chain-analysis source tools"
    ),
    "restricted": "with restricted access: projected results only",
}


def _configure_access(args: argparse.Namespace) -> dict[str, Any]:
    from ..agent_access import agent_access_status, set_agent_access

    if args.mcp_command == "status":
        return agent_access_status()
    if args.mcp_command == "disable":
        return set_agent_access(mcp_enabled=False)
    # Turning disclosure on, or widening what agents may do, is the user's
    # decision, never an agent's: require their own terminal (or the desktop
    # switch), like other consent changes.
    if args.non_interactive or not sys.stdin.isatty():
        raise AppError(
            "enabling external agents or changing their grant needs the user's own confirmation",
            code="interaction_required",
            hint=(
                "Ask the user to turn on External agents in Kassiber Settings > AI, "
                "or to run `kassiber mcp enable` in their own terminal."
            ),
            details={"reason": "user_consent"},
            retryable=False,
        )
    current = agent_access_status()
    grant = args.grant or current["grant"]
    agent_access = args.agent_access or current["agent_access"]
    sys.stderr.write(
        f"External agents you connect will {_GRANT_CONSENT_TEXT[grant]}, "
        f"{_ACCESS_CONSENT_TEXT[agent_access]}. Their model provider receives what "
        "they read, and your agent host's own tool approval is the only prompt "
        "before a change. Admin work (secrets, backups, deletes, passphrases) is "
        "never available to agents.\n"
        "Allow? [y/N] "
    )
    sys.stderr.flush()
    if sys.stdin.readline().strip().lower() not in {"y", "yes"}:
        return agent_access_status()
    return set_agent_access(mcp_enabled=True, grant=grant, agent_access=agent_access)


def describe_tools() -> dict[str, Any]:
    from ..agent_access import agent_access_status
    from .protocol import LEGACY_PROTOCOL_VERSIONS, MODERN_PROTOCOL_VERSIONS

    status = agent_access_status()
    return {
        "transport": "stdio",
        "protocol_versions": {
            "modern": list(MODERN_PROTOCOL_VERSIONS),
            "legacy_initialize": list(LEGACY_PROTOCOL_VERSIONS),
        },
        "grant": status["grant"],
        "agent_access": status["agent_access"],
        "instructions": mcp_tools.instructions_for(status["grant"]),
        "tools": mcp_tools.tool_definitions(
            grant=status["grant"], agent_access=status["agent_access"]
        ),
    }


def _parse_arguments(raw: str) -> dict[str, Any]:
    try:
        value = json.loads(raw, parse_constant=_reject_json_constant)
    except (ValueError, RecursionError):
        raise AppError(
            "--arguments must be a JSON object",
            code="validation",
            retryable=False,
        ) from None
    if not isinstance(value, dict):
        raise AppError("--arguments must be a JSON object", code="validation", retryable=False)
    return value


def _reject_json_constant(name: str) -> Any:
    raise ValueError(f"invalid JSON constant {name}")


def _project_name(args: argparse.Namespace) -> str | None:
    from ..projects import project_metadata_for_data_root

    metadata = project_metadata_for_data_root(args.data_root)
    name = metadata.get("name") if isinstance(metadata, dict) else None
    return name if isinstance(name, str) else None


# -- tool provider ---------------------------------------------------------


class BookToolProvider:
    """Execute MCP calls against the configured project, one call at a time.

    The listing, instructions and every call follow the user's External agents
    choice as recorded when they run, so a changed grant applies to the next
    call (and to the list the next time the host asks for it).
    """

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
        # Desktop agent sessions this server process holds, per data root.
        self._agent_sessions: dict[str, Any] = {}
        self._agent_sessions_lock = threading.Lock()

    @property
    def instructions(self) -> str:
        from ..agent_access import agent_access_status

        return mcp_tools.instructions_for(agent_access_status()["grant"])

    def tool_definitions(self) -> list[dict[str, Any]]:
        from ..agent_access import agent_access_status

        status = agent_access_status()
        return mcp_tools.tool_definitions(
            grant=status["grant"], agent_access=status["agent_access"]
        )

    def has_tool(self, name: str) -> bool:
        # Known at some grant: a call above the user's grant gets a tool error
        # naming the grant it needs. Admin-class and chat-only tools stay unknown.
        return mcp_tools.entry_for(name) is not None

    def call_tool(
        self,
        name: str,
        arguments: dict[str, Any],
        cancelled: threading.Event,
        client_name: str | None = None,
    ) -> ToolOutcome:
        try:
            structured = self._call(name, arguments, cancelled, mcp_tools.client_label(client_name))
        except AppError as exc:
            return _error_outcome(exc)
        except Exception:  # noqa: BLE001 - never echo raw exception text
            return _error_outcome(
                AppError("The Kassiber tool failed unexpectedly", code="tool_error")
            )
        # ASCII escaping matches what the stdio writer puts on the wire.
        text = json.dumps(structured, ensure_ascii=True, separators=(",", ":"))
        if len(text) > mcp_tools.MAX_RESULT_BYTES:
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
        client_name: str | None = None,
    ) -> dict[str, Any]:
        from ..agent_access import require_mcp_access
        from ..core.runtime import resolve_runtime_paths
        from ..operator.modes import effective_unlock_mode
        from ..projects import load_catalog

        # First, before any project resolution: off unless the user turned
        # external agents on and the desktop AI master switch is not off, and
        # refused when the tool is above the user's grant.
        status = require_mcp_access()
        grant, agent_access = status["grant"], status["agent_access"]
        tool = mcp_tools.entry_for(name)
        if tool is None:
            raise AppError(f"unknown Kassiber MCP tool: {name}", code="unknown_tool")
        refusal = mcp_tools.refusal_for(tool, grant=grant, agent_access=agent_access)
        if refusal is not None:
            raise refusal
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
            return self._call_brokered(
                paths, tool, arguments, cancelled, grant=grant, client_name=client_name
            )
        return self._call_direct(
            paths, name, arguments, grant=grant, agent_access=agent_access, client_name=client_name
        )

    def _call_direct(
        self,
        paths: Any,
        name: str,
        arguments: dict[str, Any],
        *,
        grant: str,
        agent_access: str,
        client_name: str | None,
    ) -> dict[str, Any]:
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
                grant=grant,
                agent_access=agent_access,
                client_name=client_name,
            )
        finally:
            close_runtime(runtime)

    def _agent_session(self, client: Any, data_root: str, *, replacing: Any = None) -> Any:
        """This process's registration for the book, opened when needed.

        `replacing` is the handle a call found stale. Parallel calls may find
        the same one: only the first replaces it, and the rest reuse that
        replacement rather than closing it (which could drop an approval).
        """

        with self._agent_sessions_lock:
            handle = self._agent_sessions.get(data_root)
            if handle is not None and handle.alive and handle is not replacing:
                return handle
            if handle is not None:
                handle.close()
            handle = client.open_agent_session(data_root)
            self._agent_sessions[data_root] = handle
            return handle

    def _call_brokered(
        self,
        paths: Any,
        tool: Any,
        arguments: dict[str, Any],
        cancelled: threading.Event,
        *,
        grant: str,
        client_name: str | None,
    ) -> dict[str, Any]:
        from ..agent_access import GRANTS, grant_allows, narrower_grant
        from ..operator.client import (
            TERMINAL_OPERATION_STATES,
            BrokerClient,
            prepare_arguments,
            wipe_prepared,
        )

        from .. import daemon

        # Validate here, in this build, before anything is queued: the broker
        # child runs the broker's build, which must not be the only check.
        name = tool.name
        daemon._validate_ai_tool_arguments(tool.entry, arguments)
        client = BrokerClient()
        # Status never starts a broker; without a lease there is nothing to
        # submit to, so do not spawn one just to be refused.
        status = client.status(paths.data_root)
        unlock_command = f"kassiber operator unlock --capability {grant}"
        if status.get("lease") != "unlocked":
            raise AppError(
                "this project has no active operator lease",
                code="interaction_required",
                hint=(
                    "Ask the user to unlock this book for agents in Kassiber "
                    "(Settings, AI, External agents), or to run "
                    f"`{unlock_command}` in their own "
                    "terminal, then retry. Never ask for the passphrase."
                ),
                details={
                    "reason": "operator_lease_required",
                    "unlock_mode": "brokered",
                    "user_command": unlock_command,
                },
                retryable=True,
            )
        # The lease bounds what this call may do as much as the grant does: a
        # read under a `read` lease runs without the journal refresh, and a
        # write needs a lease that covers it.
        leased = status.get("capability")
        effective = narrower_grant(grant, leased) if leased in GRANTS else "read"
        if not grant_allows(effective, tool.required.value):
            required_command = f"kassiber operator unlock --capability {tool.required.value}"
            raise AppError(
                f"the operator lease does not cover {name}",
                code="interaction_required",
                hint=(
                    "Ask the user to lock and unlock this book for agents in Kassiber "
                    "(Settings, AI, External agents), or to lock the lease and run "
                    f"`{required_command}` in their own terminal, then retry. "
                    "Never ask for the passphrase."
                ),
                details={
                    "reason": "operator_capability_insufficient",
                    "required": tool.required.value,
                    "leased": leased,
                    "user_command": required_command,
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
            # Absolute, so the path cannot change meaning in a worker.
            argv += ["--env-file", os.path.abspath(self._env_file)]
        argv += [
            "--machine",
            "mcp",
            "call",
            "--tool",
            name,
            "--arguments",
            json.dumps(arguments, separators=(",", ":")),
            # The broker requires its lease to cover this grant as well.
            "--grant",
            effective,
            "--workspace",
            str(workspace),
            "--profile",
            str(profile),
        ]
        if client_name:
            argv += ["--client", client_name]
        agent_scope = status.get("agent_scope") is True
        # The desktop's "Unlock for agents" session admits only processes the
        # user allowed; register this one so it can be allowed.
        handle = self._agent_session(client, paths.data_root) if agent_scope else None

        def submit(handle: Any) -> dict[str, Any]:
            prepared = prepare_arguments(argv, stdin=io.BytesIO())
            try:
                # The lease check above did not start a broker; a broker that
                # has since exited must not be restarted by an agent's read.
                return client.submit(
                    paths.data_root,
                    prepared,
                    admin_authentication=None,
                    start_broker=False,
                    # Its warning would be invisible (stderr is silenced), so
                    # an older broker that ignores caller context is refused.
                    require_caller_context=True,
                    # Agent reads run the broker's code; only this build's.
                    require_same_build=True,
                    agent_session_token=handle.token if handle is not None else None,
                )
            finally:
                wipe_prepared(prepared)

        try:
            accepted = submit(handle)
        except AppError as exc:
            reason = (exc.details or {}).get("reason") if isinstance(exc.details, dict) else None
            if not (agent_scope and reason == "agent_session_required"):
                raise
            # The broker dropped this process's session (its lease changed):
            # register again, which the user must allow again.
            handle = self._agent_session(client, paths.data_root, replacing=handle)
            accepted = submit(handle)
        operation_id = accepted.get("operation_id")
        if not isinstance(operation_id, str):
            raise AppError("broker did not return an operation id", code="operator_protocol_error")
        completed = accepted
        while completed.get("state") not in TERMINAL_OPERATION_STATES:
            if cancelled.is_set():
                # Queued work is cancelled; a running call (a write too) simply finishes.
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
            from ..ai.tools import redact_ai_tool_result

            # Redact again at this process's boundary, as the direct path does.
            return redact_ai_tool_result(data)
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
