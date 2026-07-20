"""Machine-readable CLI command metadata derived from the argparse tree.

This is intentionally an incremental registry: argparse remains the parser
source of truth, while bootstrap/effect annotations live here so agents do not
have to scrape human help text or duplicate the CLI's database rules.
"""

from __future__ import annotations

import argparse
from typing import Any

from ..command_capabilities import Capability, cli_capability
from ..envelope import derive_kind
from ..errors import AppError


_NO_BOOTSTRAP_DATABASE_PREFIXES = {
    "backup",
    "commands",
    "projects",
    "operator",
    "secrets",
}
_NO_BOOTSTRAP_DATABASE_PATHS = {
    "backends.kinds",
    "chat",
    "daemon",
    "wallets.kinds",
    "wallets.ledger-template",
}
_SECRET_DEST_FRAGMENTS = (
    "api_key",
    "auth_header",
    "operator_auth",
    "passphrase",
    "password",
    "secret",
    "token",
)


def command_path(args: argparse.Namespace) -> str:
    return derive_kind(args)


def command_needs_database(args: argparse.Namespace) -> bool:
    path = command_path(args)
    top_level = path.split(".", 1)[0]
    if top_level in _NO_BOOTSTRAP_DATABASE_PREFIXES:
        return False
    if path in _NO_BOOTSTRAP_DATABASE_PATHS:
        return False
    if path == "wallets.import-ledger" and bool(getattr(args, "dry_run", False)):
        return False
    return True


def _effect_for_path(path: str) -> str:
    capability = cli_capability(path)
    if path in {"chat", "daemon"}:
        return "interactive"
    return "read_only" if capability is Capability.READ else "mutating"


def _argument_metadata(action: argparse.Action) -> dict[str, Any] | None:
    if isinstance(action, argparse._HelpAction):
        return None
    flags = list(action.option_strings)
    positional = not flags
    required = bool(getattr(action, "required", False))
    if positional:
        required = action.nargs not in ("?", "*")
    choices = list(action.choices) if action.choices is not None else None
    return {
        "name": action.dest,
        "flags": flags,
        "positional": positional,
        "required": required,
        "repeatable": isinstance(action, argparse._AppendAction),
        "choices": choices,
        "secret_input": any(part in action.dest for part in _SECRET_DEST_FRAGMENTS),
        "help": action.help if action.help is not argparse.SUPPRESS else None,
    }


def _subparser_help(action: argparse._SubParsersAction) -> dict[str, str | None]:
    return {
        choice.dest: choice.help if choice.help is not argparse.SUPPRESS else None
        for choice in action._choices_actions
    }


def _walk_parser(
    parser: argparse.ArgumentParser,
    *,
    path: tuple[str, ...] = (),
    command_help: str | None = None,
) -> list[dict[str, Any]]:
    subparser_action = next(
        (
            action
            for action in parser._actions
            if isinstance(action, argparse._SubParsersAction)
        ),
        None,
    )
    if subparser_action is not None:
        help_by_name = _subparser_help(subparser_action)
        commands: list[dict[str, Any]] = []
        for name, child in subparser_action.choices.items():
            commands.extend(
                _walk_parser(
                    child,
                    path=(*path, name),
                    command_help=help_by_name.get(name),
                )
            )
        return commands

    dotted_path = ".".join(path)
    arguments = [
        metadata
        for action in parser._actions
        if (metadata := _argument_metadata(action)) is not None
    ]
    destinations = {argument["name"] for argument in arguments}
    top_level = path[0] if path else ""
    capability = cli_capability(dotted_path)
    needs_database = not (
        top_level in _NO_BOOTSTRAP_DATABASE_PREFIXES
        or dotted_path in _NO_BOOTSTRAP_DATABASE_PATHS
    )
    return [
        {
            "path": list(path),
            "command": " ".join(path),
            "kind": dotted_path,
            "help": command_help or parser.description,
            "effect": _effect_for_path(dotted_path),
            "capability": capability.value,
            "needs_database": needs_database,
            "supports_cursor": "cursor" in destinations,
            "supports_dry_run": "dry_run" in destinations,
            "scope_flags": [
                name for name in ("workspace", "profile", "wallet") if name in destinations
            ],
            "may_prompt": any(argument["secret_input"] for argument in arguments),
            "arguments": arguments,
        }
    ]


def describe_command_catalog(
    parser: argparse.ArgumentParser,
    requested_path: list[str] | None = None,
) -> dict[str, Any]:
    commands = _walk_parser(parser)
    query = list(requested_path or ())
    if query:
        commands = [command for command in commands if command["path"][: len(query)] == query]
        if not commands:
            raise AppError(
                f"unknown command path: {' '.join(query)}",
                code="unknown_command",
                details={"path": query},
                retryable=False,
            )
    return {
        "query": query,
        "count": len(commands),
        "global": {
            "machine_implies_json": True,
            "supports_non_interactive": True,
            "secret_transport": "file_descriptor_or_stdin_only",
        },
        "commands": commands,
    }
