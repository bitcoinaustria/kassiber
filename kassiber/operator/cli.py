"""Public `kassiber operator ...` commands and brokered CLI routing."""

from __future__ import annotations

import argparse
import os
import sys
from typing import Sequence

from ..command_capabilities import Capability, cli_capability
from ..core.runtime import resolve_runtime_paths
from ..errors import AppError
from ..secrets.prompt import prompt_passphrase, read_passphrase_from_fd
from .client import (
    BrokerClient,
    PreparedArguments,
    parse_duration,
    prepare_arguments,
    wipe_prepared,
)
from .modes import effective_unlock_mode, unlock_mode_status
from .native_auth import touch_id_status
from .runner import strip_database_passphrase_arguments
from .service import _wipe


_DIRECT_COMMANDS = frozenset({"commands", "daemon", "chat", "operator", "projects"})


def add_operator_parser(subparsers: argparse._SubParsersAction) -> None:
    operator = subparsers.add_parser(
        "operator",
        help="Manage terminal broker leases, modes, and queued operations",
    )
    commands = operator.add_subparsers(dest="operator_command", required=True)

    unlock = commands.add_parser("unlock", help="Create or refresh a project lease")
    duration = unlock.add_mutually_exclusive_group()
    duration.add_argument(
        "--until-lock",
        action="store_true",
        help="Keep the lease until explicit lock or broker exit (default)",
    )
    duration.add_argument(
        "--duration",
        metavar="DURATION",
        help="Lease duration such as 30m, 8h, or 2d",
    )
    unlock.add_argument(
        "--capability",
        choices=("read", "operator", "accounting_decisions"),
        default="accounting_decisions",
    )
    unlock.add_argument(
        "--auth",
        choices=("password", "touch-id"),
        default="password",
        help="Local authentication method; Touch ID is available in the macOS app bundle",
    )
    unlock.add_argument("--passphrase-fd", type=int, default=None, metavar="FD")

    status = commands.add_parser("status", help="Show public-safe broker state")
    status.add_argument("--all", action="store_true", help="Show every active project lease")
    commands.add_parser("lock", help="Drop the active project lease")

    mode = commands.add_parser("mode", help="Select manual, brokered, or unattended unlock")
    mode.add_argument("mode", choices=("manual", "brokered", "unattended"))
    mode.add_argument("--passphrase-fd", type=int, default=None, metavar="FD")

    operation = commands.add_parser("operation", help="Inspect or cancel accepted work")
    operations = operation.add_subparsers(dest="operator_operation_command", required=True)
    operation_status = operations.add_parser("status")
    operation_status.add_argument("operation_id")
    operation_cancel = operations.add_parser("cancel")
    operation_cancel.add_argument("operation_id")

    touch_id = commands.add_parser("touch-id", help="Manage operator-only Touch ID enrollment")
    touch_id_commands = touch_id.add_subparsers(
        dest="operator_touch_id_command",
        required=True,
    )
    touch_id_commands.add_parser("status")
    touch_id_enroll = touch_id_commands.add_parser("enroll")
    touch_id_enroll.add_argument("--passphrase-fd", type=int, default=None, metavar="FD")
    touch_id_forget = touch_id_commands.add_parser("forget")
    touch_id_forget.add_argument("--passphrase-fd", type=int, default=None, metavar="FD")


def dispatch_operator(args: argparse.Namespace) -> dict[str, object]:
    client = BrokerClient()
    command = args.operator_command
    if command == "operation":
        if args.operator_operation_command == "status":
            return client.operation_status(args.operation_id)
        return client.cancel(args.operation_id)

    data_root = _selected_data_root(args)
    if command == "touch-id":
        if args.operator_touch_id_command == "status":
            return touch_id_status(data_root)
        authentication = _password_secret(
            getattr(args, "passphrase_fd", None),
            non_interactive=args.non_interactive,
            label="Fresh database passphrase: ",
        )
        try:
            return client.configure_touch_id(
                data_root,
                authentication,
                configured=args.operator_touch_id_command == "enroll",
            )
        finally:
            _wipe(authentication)
    if command == "status":
        broker = client.status(None if args.all else data_root)
        if not args.all:
            broker["mode"] = unlock_mode_status(data_root)
        return broker
    if command == "lock":
        return client.lock(data_root)
    if command == "unlock":
        if args.auth == "touch-id":
            _require_interactive_auth(args.non_interactive)
            return client.unlock_touch_id(
                data_root,
                duration_seconds=(
                    parse_duration(args.duration) if args.duration else None
                ),
                capability=args.capability,
            )
        passphrase = _password_secret(
            getattr(args, "passphrase_fd", None),
            non_interactive=args.non_interactive,
            label="Database passphrase: ",
        )
        try:
            return client.unlock(
                data_root,
                passphrase,
                duration_seconds=(
                    parse_duration(args.duration) if args.duration else None
                ),
                capability=args.capability,
                authentication_method=(
                    "touch_id" if args.auth == "touch-id" else "password"
                ),
            )
        finally:
            _wipe(passphrase)
    if command == "mode":
        authentication = _password_secret(
            getattr(args, "passphrase_fd", None),
            non_interactive=args.non_interactive,
            label="Fresh database passphrase: ",
        )
        try:
            return client.set_mode(data_root, args.mode, authentication)
        finally:
            _wipe(authentication)
    raise AppError("unknown operator command", code="unknown_command")


def route_brokered_command(
    args: argparse.Namespace,
    argv: Sequence[str],
) -> int | None:
    """Submit ordinary CLI work when this project's explicit mode is brokered."""

    if os.environ.get("KASSIBER_OPERATOR_DIRECT") == "1":
        return None
    if args.command == "chat":
        data_root = _selected_data_root(args)
        if effective_unlock_mode(data_root) == "brokered":
            raise AppError(
                "CLI chat is not available while this project is in brokered mode",
                code="operator_chat_not_supported",
                hint=(
                    "Use brokered CLI commands directly, or lock the broker and "
                    "select manual mode before starting `kassiber chat`."
                ),
                retryable=False,
            )
        return None
    if args.command in _DIRECT_COMMANDS:
        return None
    from ..cli.command_registry import command_path

    path = command_path(args)
    data_root = _selected_data_root(args)
    try:
        mode = effective_unlock_mode(data_root)
    except AppError as exc:
        if path == "secrets.forget-unlock" and exc.code in {
            "operator_policy_binding_required",
            "operator_policy_binding_mismatch",
        }:
            return None
        raise
    if mode != "brokered":
        return None
    capability = cli_capability(path)
    pinned_argv = _pin_project_arguments(list(argv), data_root)
    pinned_argv, _ignored_database_secrets = strip_database_passphrase_arguments(
        pinned_argv
    )
    operator_auth_fd = getattr(args, "operator_auth_fd", None)
    admin_authentication: bytearray | None = None
    prepared: PreparedArguments | None = None
    try:
        if capability is Capability.ADMIN:
            admin_authentication = _password_secret(
                operator_auth_fd,
                non_interactive=args.non_interactive,
                label="Fresh database passphrase for admin operation: ",
            )
        prepared = prepare_arguments(pinned_argv)
        client = BrokerClient()
        accepted = client.submit(
            data_root,
            prepared,
            admin_authentication=admin_authentication,
        )
        operation_id = accepted.get("operation_id")
        if not isinstance(operation_id, str):
            raise AppError("broker did not return an operation id", code="operator_protocol_error")
        if not args.machine:
            sys.stderr.write(f"operator operation accepted: {operation_id}\n")
        try:
            completed = client.wait(operation_id)
        except KeyboardInterrupt:
            cancelled = client.cancel(operation_id)
            sys.stderr.write(
                f"operator cancellation requested: {operation_id} "
                f"({cancelled.get('state', 'unknown')})\n"
            )
            return 130
        stdout = completed.get("stdout")
        stderr = completed.get("stderr")
        if isinstance(stdout, str) and stdout:
            sys.stdout.write(stdout)
        if isinstance(stderr, str) and stderr:
            sys.stderr.write(stderr)
        output_error = completed.get("output_error")
        if isinstance(output_error, dict):
            code = str(output_error.get("code") or "operator_output_unavailable")
            message = str(
                output_error.get("message")
                or "The operator result output is unavailable."
            )
            sys.stderr.write(f"{code}: {message}\n")
            return 1
        state = completed.get("state")
        exit_code = completed.get("exit_code")
        if state == "result_unknown":
            sys.stderr.write(
                f"operator result is unknown; inspect operation {operation_id} before retrying\n"
            )
            return 1
        return int(exit_code) if isinstance(exit_code, int) else (0 if state == "completed" else 1)
    finally:
        if prepared is not None:
            wipe_prepared(prepared)
        if admin_authentication is not None:
            _wipe(admin_authentication)


def _selected_data_root(args: argparse.Namespace) -> str:
    paths = resolve_runtime_paths(
        getattr(args, "data_root", None),
        getattr(args, "env_file", None),
        getattr(args, "project", None),
    )
    return paths.data_root


def _password_secret(fd: int | None, *, non_interactive: bool, label: str) -> bytearray:
    if fd is not None:
        return bytearray(read_passphrase_from_fd(int(fd)).encode("utf-8"))
    _require_interactive_auth(non_interactive)
    return bytearray(prompt_passphrase(label).encode("utf-8"))


def _require_interactive_auth(non_interactive: bool) -> None:
    if non_interactive or not sys.stdin.isatty():
        raise AppError(
            "fresh local authentication is required",
            code="interaction_required",
            hint="Pass the passphrase through the command's dedicated fd flag.",
            retryable=False,
        )


def _pin_project_arguments(argv: list[str], data_root: str) -> list[str]:
    pinned: list[str] = ["--data-root", data_root]
    index = 0
    while index < len(argv):
        token = argv[index]
        if token in {"--project", "--data-root", "--operator-auth-fd"}:
            index += 2
            continue
        if token.startswith("--project=") or token.startswith("--data-root="):
            index += 1
            continue
        if token.startswith("--operator-auth-fd="):
            index += 1
            continue
        pinned.append(token)
        index += 1
    return pinned
