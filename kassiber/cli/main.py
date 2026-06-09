from __future__ import annotations

import argparse
import csv
import json
import logging
import sqlite3
import sys
import traceback
from typing import Any, Sequence

from .. import daemon as daemon_runtime
from ..ai import (
    AI_PROVIDER_KINDS,
    clear_default_ai_provider,
    create_db_ai_provider,
    delete_db_ai_provider,
    get_db_ai_provider,
    get_ai_provider_api_key_for_use,
    redact_ai_provider_for_output,
    resolve_ai_provider,
    set_default_ai_provider,
    update_db_ai_provider,
)
from ..ai.client import ai_client_for_locator
from ..ai.providers import (
    list_with_default as list_ai_providers_with_default,
)
from ..core import chat_history as core_chat_history
from .handlers import (
    APP_NAME,
    BACKEND_CLEAR_FIELD_ALIASES,
    BACKEND_KINDS,
    BTCPAY_DEFAULT_PAGE_SIZE,
    BTCPAY_DEFAULT_PAYMENT_METHOD_ID,
    DEFAULT_DATA_ROOT,
    DEFAULT_ENV_FILENAME,
    DEFAULT_EVENTS_LIMIT,
    DEFAULT_LONG_TERM_DAYS,
    DEFAULT_TAX_COUNTRY,
    OUTPUT_FORMATS,
    RP2_ACCOUNTING_METHODS,
    TRANSFER_PAIR_KINDS,
    TRANSFER_PAIR_POLICIES,
    _attachment_hooks,
    _commercial_hooks,
    _metadata_hooks,
    _report_hooks,
    clear_quarantine,
    cmd_context_set,
    cmd_context_show,
    cmd_init,
    cmd_status,
    apply_transfer_rules,
    bulk_pair_transfers,
    create_direct_swap_payout,
    chat_history_config_cli,
    clear_chat_sessions_cli,
    create_saved_view_cli,
    create_transaction_pair,
    create_transfer_rule,
    delete_chat_session_cli,
    delete_direct_swap_payout,
    delete_saved_view_cli,
    delete_transaction_pair,
    delete_transfer_rule,
    dismiss_transfer_candidate,
    list_chat_sessions_cli,
    list_saved_views_cli,
    show_chat_session_cli,
    list_transfer_rules,
    set_transfer_rule_enabled,
    suggest_transfer_candidates,
    derive_wallet_targets,
    emit,
    get_journal_event,
    import_into_wallet,
    inspect_transfer_audit,
    list_direct_swap_payouts,
    list_journal_entries,
    list_journal_events,
    list_quarantines,
    list_transaction_pairs,
    list_transactions,
    normalize_asset_code,
    normalize_chain_value,
    normalize_network_value,
    process_journals,
    resolve_scope,
    resolve_transaction,
    resolve_quarantine_exclude,
    resolve_quarantine_price_override,
    show_quarantine,
    attach_btcpay_provenance_to_wallet,
    sync_btcpay_commercial_provenance,
    sync_btcpay_into_wallet,
    sync_wallet,
)
from ..core import accounts as core_accounts
from ..core import attachments as core_attachments
from ..core import commercial as core_commercial
from ..core import lightning as core_lightning
from ..core.lightning import lnd as _core_lightning_lnd  # noqa: F401 — registers the LND adapter on import.
from ..core import metadata as core_metadata
from ..core import rates as core_rates
from ..core import reports as core_reports
from ..core import samourai as core_samourai
from ..core import source_funds as core_source_funds
from ..core import source_funds_coverage as core_source_funds_coverage
from ..core import source_funds_diagram
from ..core import source_funds_recipients as core_source_funds_recipients
from ..core import wallets as core_wallets
from ..core.runtime import bootstrap_runtime, close_runtime, emit_error, resolve_output_format
from ..diagnostics import (
    collect_public_diagnostics,
    save_public_diagnostics_report,
    write_error_diagnostics,
)
from ..backup.cli import add_backup_parser, dispatch_backup
from ..backends import preferred_explorer_base
from ..errors import AppError
from ..log_ring import sanitize_traceback_text
from ..secrets.cli import add_secrets_parser, dispatch_secrets
from ..secrets.cli_input import (
    add_secret_stdin_options,
    enforce_single_stdin_consumer,
    read_secret_from_args,
)
from ..tax_policy import supported_tax_countries
from ..wallet_descriptors import MAX_DESCRIPTOR_GAP_LIMIT
from .chat import run_chat_command


_AI_PROVIDER_KINDS_LIST = AI_PROVIDER_KINDS
_AI_PROVIDER_CLEARABLE_FIELDS = ("api_key", "default_model", "notes")


def _ai_provider_redacted(conn: sqlite3.Connection, provider: dict) -> dict:
    """Redact a provider for emit, decorating with `is_default` from the
    stored default pointer."""
    from ..ai.providers import get_default_ai_provider_name

    return redact_ai_provider_for_output(provider, default_name=get_default_ai_provider_name(conn))


def _ai_client_for(provider: dict):
    return ai_client_for_locator(
        base_url=provider["base_url"],
        api_key=get_ai_provider_api_key_for_use(provider),
    )


def _backend_extra_config(args: argparse.Namespace) -> dict[str, object] | None:
    enforce_single_stdin_consumer(
        args, ("token", "auth_header", "username", "password")
    )
    config = {}
    if getattr(args, "insecure", None) is not None:
        config["insecure"] = args.insecure
    if getattr(args, "cookiefile", None) is not None:
        config["cookiefile"] = args.cookiefile
    if getattr(args, "certificate", None) is not None:
        config["certificate"] = args.certificate
    if getattr(args, "lightning_cli", None) is not None:
        config["lightning_cli"] = args.lightning_cli
    if getattr(args, "lightning_dir", None) is not None:
        config["lightning_dir"] = args.lightning_dir
    if getattr(args, "rpc_file", None) is not None:
        config["rpc_file"] = args.rpc_file
    if getattr(args, "commando_peer_id", None) is not None:
        config["commando_peer_id"] = args.commando_peer_id
    if getattr(args, "display_name", None) is not None:
        config["display_name"] = args.display_name
    username = read_secret_from_args(args, "username")
    if username is not None:
        config["username"] = username
    password = read_secret_from_args(args, "password")
    if password is not None:
        config["password"] = password
    if getattr(args, "wallet_prefix", None) is not None:
        config["walletprefix"] = args.wallet_prefix
    return config or None


def _backend_token(args: argparse.Namespace) -> str | None:
    return read_secret_from_args(args, "token")


def _backend_auth_header(args: argparse.Namespace) -> str | None:
    return read_secret_from_args(args, "auth-header", legacy_attr="auth_header")


def _normalized_backend_clear_fields(values: Sequence[str] | None) -> list[str]:
    cleared = []
    seen = set()
    for value in values or ():
        normalized = BACKEND_CLEAR_FIELD_ALIASES[value]
        if normalized in seen:
            continue
        seen.add(normalized)
        cleared.append(normalized)
    return cleared


def _add_workspace_profile_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--workspace")
    parser.add_argument("--profile")


def _read_optional_text_file(file_path: str | None, label: str) -> str | None:
    if not file_path:
        return None
    try:
        with open(file_path, "r", encoding="utf-8") as handle:
            text = handle.read()
    except OSError as exc:
        raise AppError(
            f"Could not read {label} file",
            code="validation",
            hint="Choose a readable local file path.",
            details={"path": file_path},
            retryable=False,
        ) from exc
    text = text.strip()
    if not text:
        raise AppError(
            f"{label} file is empty",
            code="validation",
            details={"path": file_path},
            retryable=False,
        )
    if "\x00" in text:
        raise AppError(
            f"{label} file contains NUL bytes",
            code="validation",
            details={"path": file_path},
            retryable=False,
        )
    return text


def _add_austrian_e1kv_report_args(parser: argparse.ArgumentParser) -> None:
    _add_workspace_profile_args(parser)
    parser.add_argument("--year", type=int, required=True, help="Four-digit tax year")


def _add_austrian_e1kv_pdf_args(parser: argparse.ArgumentParser) -> None:
    _add_austrian_e1kv_report_args(parser)
    parser.add_argument("--file", required=True)


def _lightning_window_days(value: str) -> int:
    """argparse ``type`` for the ``--window-days`` flag.

    Mirrors the daemon's ``_coerce_int(default=30, minimum=1, maximum=365)``
    so the CLI rejects out-of-range values at parse time instead of silently
    clamping further down.
    """
    try:
        parsed = int(value)
    except (TypeError, ValueError) as exc:
        raise argparse.ArgumentTypeError(
            f"--window-days must be an integer (got {value!r})"
        ) from exc
    if parsed < 1 or parsed > 365:
        raise argparse.ArgumentTypeError(
            f"--window-days must be between 1 and 365 (got {parsed})"
        )
    return parsed


def _cli_build_lightning_snapshot(
    conn: sqlite3.Connection,
    ref: str,
    *,
    window_days: int,
    workspace_ref: str | None = None,
    profile_ref: str | None = None,
) -> tuple[dict[str, Any], core_lightning.NodeSnapshot]:
    connection = core_lightning.resolve_lightning_connection(
        conn, ref, workspace_ref=workspace_ref, profile_ref=profile_ref
    )
    kind = str(connection["kind"])
    adapter = core_lightning.resolve_adapter(kind)
    if adapter is None:
        registered = ", ".join(core_lightning.registered_kinds()) or "<none>"
        raise AppError(
            f"No Lightning sync adapter is registered for kind '{kind}'.",
            code="lightning_adapter_unavailable",
            hint=(
                f"Registered Lightning kinds: {registered}. Install the matching"
                " Lightning sync (LND or Core Lightning)."
            ),
        )
    snapshot = adapter.fetch_node_snapshot(connection, None, window_days=window_days)
    return connection, snapshot


def _cli_lightning_profitability_payload(
    conn: sqlite3.Connection,
    ref: str,
    *,
    window_days: int,
    workspace_ref: str | None = None,
    profile_ref: str | None = None,
) -> dict[str, Any]:
    connection, snapshot = _cli_build_lightning_snapshot(
        conn,
        ref,
        window_days=window_days,
        workspace_ref=workspace_ref,
        profile_ref=profile_ref,
    )
    report = core_lightning.build_profitability_report(
        connection_id=str(connection.get("id") or ""),
        connection_label=str(connection.get("label") or ""),
        connection_kind=str(connection.get("kind") or ""),
        snapshot=snapshot,
    )
    return report.to_envelope_payload()


def _cli_export_lightning_profitability_csv(
    conn: sqlite3.Connection,
    ref: str,
    file_path: str,
    *,
    window_days: int,
    workspace_ref: str | None = None,
    profile_ref: str | None = None,
) -> dict[str, Any]:
    connection, snapshot = _cli_build_lightning_snapshot(
        conn,
        ref,
        window_days=window_days,
        workspace_ref=workspace_ref,
        profile_ref=profile_ref,
    )
    report = core_lightning.build_profitability_report(
        connection_id=str(connection.get("id") or ""),
        connection_label=str(connection.get("label") or ""),
        connection_kind=str(connection.get("kind") or ""),
        snapshot=snapshot,
    )
    rows = core_lightning.profitability_csv_rows(report)
    with open(file_path, "w", newline="", encoding="utf-8") as handle:
        writer = csv.writer(handle)
        writer.writerows(rows)
    return {
        "connection": {
            "id": connection.get("id"),
            "label": connection.get("label"),
            "kind": connection.get("kind"),
        },
        "file": file_path,
        "rows": len(rows) - 1,
    }


def _source_funds_hooks() -> core_source_funds.SourceFundsHooks:
    report_hooks = _report_hooks()
    return core_source_funds.SourceFundsHooks(
        resolve_scope=resolve_scope,
        resolve_transaction=resolve_transaction,
        format_table=report_hooks.format_table,
        explorer_base=preferred_explorer_base,
    )


def _emit_austrian_e1kv_report(
    args: argparse.Namespace,
    conn: sqlite3.Connection,
    report_hooks,
) -> int:
    if args.format in {"table", "plain"}:
        return emit(
            args,
            "\n".join(
                core_reports.build_austrian_e1kv_report_lines(
                    conn,
                    args.workspace,
                    args.profile,
                    report_hooks,
                    tax_year=args.year,
                )
            ),
        )
    report = core_reports.report_austrian_e1kv(
        conn,
        args.workspace,
        args.profile,
        report_hooks,
        tax_year=args.year,
    )
    if args.format == "csv":
        rows = (
            report["summary_rows"]
            if args.reports_command == "austrian-tax-summary"
            else report["rows"]
        )
        return emit(args, rows)
    return emit(args, report)


def _emit_austrian_e1kv_pdf(
    args: argparse.Namespace,
    conn: sqlite3.Connection,
    report_hooks,
) -> int:
    return emit(
        args,
        core_reports.export_austrian_e1kv_pdf_report(
            conn,
            args.workspace,
            args.profile,
            args.file,
            report_hooks,
            tax_year=args.year,
        ),
    )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog=APP_NAME,
        description="Open-source, local-first Bitcoin accounting with wallet buckets and multi-wallet support. CLI surface of the Kassiber suite; a desktop GUI is also available.",
    )
    parser.add_argument("--data-root", default=DEFAULT_DATA_ROOT, help="Data directory for the local SQLite store")
    parser.add_argument(
        "--env-file",
        default=None,
        help=f"Path to a dotenv file that defines named sync backends (managed default: ~/.kassiber/config/{DEFAULT_ENV_FILENAME})",
    )
    parser.add_argument(
        "--format",
        choices=list(OUTPUT_FORMATS),
        default=None,
        help="Output format: table (default interactive), json (envelope), plain (text), csv (tabular)",
    )
    parser.add_argument(
        "--machine",
        action="store_true",
        help="Machine-readable mode: implies --format json, writes a structured envelope",
    )
    parser.add_argument(
        "--output",
        default=None,
        help="Write output to this file path instead of stdout (use '-' for stdout)",
    )
    parser.add_argument(
        "--debug",
        action="store_true",
        help="Print a full traceback on error for diagnostics",
    )
    parser.add_argument(
        "--diagnostics-out",
        metavar="PATH|auto",
        help="On error, write a public-safe diagnostics report to PATH, or use 'auto' for exports/diagnostics",
    )
    parser.add_argument(
        "--db-passphrase-fd",
        type=int,
        default=None,
        metavar="FD",
        help=(
            "Read the SQLCipher database passphrase from this open file descriptor "
            "and close it after use; required for headless automation against an encrypted database."
        ),
    )
    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("daemon")
    sub.add_parser("init")
    sub.add_parser("status")

    chat = sub.add_parser(
        "chat",
        description=(
            "Kassiber AI assistant — the same daemon tool loop, consent, and "
            "cancel protocol as the desktop Assistant."
        ),
    )
    chat.add_argument(
        "prompt",
        nargs="?",
        help="One-shot prompt; pass '-' to read it from stdin. Omit for REPL mode.",
    )
    chat.add_argument("--prompt", dest="prompt_text", help="One-shot prompt text.")
    chat.add_argument("--provider", help="Provider name (defaults to the stored default)")
    chat.add_argument("--model", help="Model id (defaults to the provider's default_model)")
    chat.add_argument(
        "--system",
        help="Raw system prompt replacing the built-in Kassiber assistant prompt.",
    )
    chat.add_argument("--temperature", type=float)
    chat.add_argument("--max-tokens", type=int)
    chat.add_argument(
        "--reasoning-effort",
        choices=("auto", "low", "medium", "high"),
        default="auto",
        help="Forward a provider-specific reasoning effort option when supported.",
    )
    chat.add_argument(
        "--tool-loop-max-iterations",
        type=int,
        default=8,
        help="Maximum daemon tool-loop iterations for one assistant turn.",
    )
    chat.add_argument(
        "--no-tools",
        action="store_true",
        help="Disable daemon AI tools for this chat.",
    )
    chat.add_argument(
        "--yes",
        action="store_true",
        help="Non-interactively allow mutating AI tools for this chat session.",
    )
    chat.add_argument(
        "--allow-tool",
        action="append",
        help=(
            "Non-interactively allow only this mutating tool name; repeat or "
            "pass comma-separated names. Other mutating tools still prompt on a TTY "
            "or deny without one."
        ),
    )
    chat.add_argument(
        "--stream-json",
        action="store_true",
        help=(
            "One-shot scripting mode: emit the raw daemon stream records "
            "(ai.chat.status/delta/tool_call/tool_result and the terminal "
            "ai.chat) as NDJSON instead of rendered text."
        ),
    )
    chat.add_argument(
        "--transcript",
        metavar="PATH",
        help=(
            "Append every daemon request and stream record for this chat "
            "session to PATH as NDJSON. The file is plaintext and includes "
            "prompts and redacted tool results."
        ),
    )
    chat.add_argument(
        "--plain",
        action="store_true",
        help=(
            "Disable terminal markdown rendering and deterministic "
            "tool-result tables; print the raw model output."
        ),
    )
    chat.add_argument(
        "--incognito",
        action="store_true",
        help="Do not persist this chat to the database, regardless of the history setting.",
    )
    chat.add_argument(
        "--continue",
        dest="continue_session",
        action="store_true",
        help="Continue the most recently updated persisted chat session.",
    )
    chat.add_argument(
        "--session",
        metavar="SESSION_ID",
        help="Continue a specific persisted chat session (see `kassiber chats list`).",
    )

    chats = sub.add_parser(
        "chats",
        description=(
            "Manage persisted AI chat sessions. History is stored in the "
            "(SQLCipher-encrypted) database; the `auto` policy persists only "
            "when the database is encrypted."
        ),
    )
    chats_sub = chats.add_subparsers(dest="chats_command", required=True)
    chats_list = chats_sub.add_parser("list")
    chats_list.add_argument("--workspace")
    chats_list.add_argument("--profile")
    chats_list.add_argument("--limit", type=int, default=50)
    chats_show = chats_sub.add_parser("show")
    chats_show.add_argument("session_id")
    chats_show.add_argument("--workspace")
    chats_show.add_argument("--profile")
    chats_delete = chats_sub.add_parser("delete")
    chats_delete.add_argument("session_id")
    chats_delete.add_argument("--workspace")
    chats_delete.add_argument("--profile")
    chats_clear = chats_sub.add_parser("clear")
    chats_clear.add_argument("--workspace")
    chats_clear.add_argument("--profile")
    chats_config = chats_sub.add_parser(
        "config",
        description="Show or set the chat history policy (auto persists only on encrypted databases).",
    )
    chats_config.add_argument("--history", choices=("auto", "on", "off"))

    add_secrets_parser(sub)
    add_backup_parser(sub)

    backends = sub.add_parser("backends")
    backends_sub = backends.add_subparsers(dest="backends_command", required=True)
    backends_sub.add_parser("list")
    backends_sub.add_parser("kinds")

    backends_get = backends_sub.add_parser("get")
    backends_get.add_argument("name")

    backends_create = backends_sub.add_parser("create")
    backends_create.add_argument("name")
    backends_create.add_argument("--kind", required=True, choices=sorted(BACKEND_KINDS))
    backends_create.add_argument("--url", required=True)
    backends_create.add_argument("--chain", choices=["bitcoin", "liquid"])
    backends_create.add_argument("--network")
    backends_create.add_argument(
        "--auth-header",
        help="DEPRECATED — exposes secrets in shell history; prefer --auth-header-stdin",
    )
    backends_create.add_argument(
        "--token",
        help="DEPRECATED — exposes secrets in shell history; prefer --token-stdin",
    )
    add_secret_stdin_options(backends_create, "auth-header", label="auth header")
    add_secret_stdin_options(backends_create, "token")
    backends_create.add_argument("--batch-size", type=int)
    backends_create.add_argument("--timeout", type=int)
    backends_create.add_argument("--tor-proxy")
    backends_create.add_argument("--insecure")
    backends_create.add_argument(
        "--certificate",
        help="Path to tls.cert or PEM contents (LND).",
    )
    backends_create.add_argument("--cookiefile")
    backends_create.add_argument("--lightning-cli", dest="lightning_cli")
    backends_create.add_argument("--lightning-dir", dest="lightning_dir")
    backends_create.add_argument("--rpc-file", dest="rpc_file")
    backends_create.add_argument("--commando-peer-id", dest="commando_peer_id")
    backends_create.add_argument("--display-name", dest="display_name")
    backends_create.add_argument(
        "--username",
        help="DEPRECATED — exposes secrets in shell history; prefer --username-stdin",
    )
    backends_create.add_argument(
        "--password",
        help="DEPRECATED — exposes secrets in shell history; prefer --password-stdin",
    )
    add_secret_stdin_options(backends_create, "username")
    add_secret_stdin_options(backends_create, "password")
    backends_create.add_argument("--wallet-prefix")
    backends_create.add_argument("--notes")

    backends_update = backends_sub.add_parser("update")
    backends_update.add_argument("name")
    backends_update.add_argument("--kind", choices=sorted(BACKEND_KINDS))
    backends_update.add_argument("--url")
    backends_update.add_argument("--chain", choices=["bitcoin", "liquid"])
    backends_update.add_argument("--network")
    backends_update.add_argument(
        "--auth-header",
        help="DEPRECATED — prefer --auth-header-stdin",
    )
    backends_update.add_argument(
        "--token",
        help="DEPRECATED — prefer --token-stdin",
    )
    add_secret_stdin_options(backends_update, "auth-header", label="auth header")
    add_secret_stdin_options(backends_update, "token")
    backends_update.add_argument("--batch-size", type=int)
    backends_update.add_argument("--timeout", type=int)
    backends_update.add_argument("--tor-proxy")
    backends_update.add_argument("--insecure")
    backends_update.add_argument(
        "--certificate",
        help="Path to tls.cert or PEM contents (LND).",
    )
    backends_update.add_argument("--cookiefile")
    backends_update.add_argument("--lightning-cli", dest="lightning_cli")
    backends_update.add_argument("--lightning-dir", dest="lightning_dir")
    backends_update.add_argument("--rpc-file", dest="rpc_file")
    backends_update.add_argument("--commando-peer-id", dest="commando_peer_id")
    backends_update.add_argument("--display-name", dest="display_name")
    backends_update.add_argument(
        "--username",
        help="DEPRECATED — prefer --username-stdin",
    )
    backends_update.add_argument(
        "--password",
        help="DEPRECATED — prefer --password-stdin",
    )
    add_secret_stdin_options(backends_update, "username")
    add_secret_stdin_options(backends_update, "password")
    backends_update.add_argument("--wallet-prefix")
    backends_update.add_argument("--notes")
    backends_update.add_argument("--clear", action="append", choices=sorted(BACKEND_CLEAR_FIELD_ALIASES))

    backends_delete = backends_sub.add_parser("delete")
    backends_delete.add_argument("name")

    backends_set_default = backends_sub.add_parser("set-default")
    backends_set_default.add_argument("name")

    backends_sub.add_parser("clear-default")

    backends_reveal = backends_sub.add_parser(
        "reveal-token",
        help="Print the raw token / auth-header for a backend (requires database unlock)",
    )
    backends_reveal.add_argument("name")

    context = sub.add_parser("context")
    context_sub = context.add_subparsers(dest="context_command", required=True)
    context_sub.add_parser("show")
    context_sub.add_parser("current")
    context_set = context_sub.add_parser("set")
    context_set.add_argument("--workspace")
    context_set.add_argument("--profile")

    workspaces = sub.add_parser("workspaces")
    ws_sub = workspaces.add_subparsers(dest="workspaces_command", required=True)
    ws_sub.add_parser("list")
    ws_create = ws_sub.add_parser("create")
    ws_create.add_argument("label")

    profiles = sub.add_parser("profiles")
    profiles_sub = profiles.add_subparsers(dest="profiles_command", required=True)
    profiles_list = profiles_sub.add_parser("list")
    profiles_list.add_argument("--workspace")
    profiles_create = profiles_sub.add_parser("create")
    profiles_create.add_argument("label")
    profiles_create.add_argument("--workspace")
    profiles_create.add_argument("--fiat-currency", default="USD")
    profiles_create.add_argument(
        "--tax-country",
        default=DEFAULT_TAX_COUNTRY,
        help=f"Tax country for the book (CLI profile; currently supported: {', '.join(supported_tax_countries())})",
    )
    profiles_create.add_argument("--tax-long-term-days", type=int, default=DEFAULT_LONG_TERM_DAYS)
    profiles_create.add_argument("--gains-algorithm", choices=list(RP2_ACCOUNTING_METHODS), default="FIFO")

    profiles_get = profiles_sub.add_parser("get")
    profiles_get.add_argument("--workspace")
    profiles_get.add_argument("--profile")

    profiles_set = profiles_sub.add_parser("set")
    profiles_set.add_argument("--workspace")
    profiles_set.add_argument("--profile")
    profiles_set.add_argument("--label")
    profiles_set.add_argument("--fiat-currency")
    profiles_set.add_argument(
        "--tax-country",
        help=f"Tax country for the book (CLI profile; currently supported: {', '.join(supported_tax_countries())})",
    )
    profiles_set.add_argument("--tax-long-term-days", type=int)
    profiles_set.add_argument("--gains-algorithm", choices=list(RP2_ACCOUNTING_METHODS))

    accounts = sub.add_parser(
        "accounts",
        description="Manage wallet/reporting buckets. These are not double-entry chart-of-accounts records.",
    )
    accounts_sub = accounts.add_subparsers(dest="accounts_command", required=True)
    accounts_list = accounts_sub.add_parser("list")
    accounts_list.add_argument("--workspace")
    accounts_list.add_argument("--profile")
    accounts_create = accounts_sub.add_parser("create")
    accounts_create.add_argument("--workspace")
    accounts_create.add_argument("--profile")
    accounts_create.add_argument("--code", required=True)
    accounts_create.add_argument("--label", required=True)
    accounts_create.add_argument(
        "--type",
        required=True,
        help="Descriptive bucket type; reports do not use this for double-entry rollups.",
    )
    accounts_create.add_argument(
        "--asset",
        help="Optional descriptive asset hint; wallet and transaction assets are not constrained by it.",
    )

    wallets = sub.add_parser("wallets")
    wallets_sub = wallets.add_subparsers(dest="wallets_command", required=True)
    wallets_list = wallets_sub.add_parser("list")
    wallets_list.add_argument("--workspace")
    wallets_list.add_argument("--profile")
    wallets_create = wallets_sub.add_parser("create")
    wallets_create.add_argument("--workspace")
    wallets_create.add_argument("--profile")
    wallets_create.add_argument("--label", required=True)
    wallets_create.add_argument("--kind", required=True)
    wallets_create.add_argument("--account", help="Wallet/reporting bucket code, id, or unique label")
    wallets_create.add_argument("--backend")
    wallets_create.add_argument("--chain", choices=["bitcoin", "liquid"])
    wallets_create.add_argument("--network")
    wallets_create.add_argument("--address", action="append")
    wallets_create.add_argument(
        "--descriptor",
        help="DEPRECATED — exposes descriptor in shell history; prefer --descriptor-stdin or --descriptor-file",
    )
    wallets_create.add_argument("--descriptor-file")
    add_secret_stdin_options(wallets_create, "descriptor")
    wallets_create.add_argument(
        "--change-descriptor",
        help="DEPRECATED — prefer --change-descriptor-stdin or --change-descriptor-file",
    )
    wallets_create.add_argument("--change-descriptor-file")
    add_secret_stdin_options(
        wallets_create, "change-descriptor", label="change descriptor"
    )
    wallets_create.add_argument("--gap-limit", type=int)
    wallets_create.add_argument("--policy-asset")
    wallets_create.add_argument("--store-id")
    wallets_create.add_argument("--payment-method-id")
    wallets_create.add_argument("--config")
    wallets_create.add_argument("--config-file")
    wallets_create.add_argument("--source-file")
    wallets_create.add_argument("--source-format", choices=["json", "csv", "btcpay_json", "btcpay_csv", "phoenix_csv", "river_csv", "bullbitcoin_csv", "coinfinity_csv", "21bitcoin_csv", "pocketbitcoin_csv", "strike_csv", "wasabi_bundle"])

    wallets_sub.add_parser("kinds")

    wallets_get = wallets_sub.add_parser("get")
    wallets_get.add_argument("--workspace")
    wallets_get.add_argument("--profile")
    wallets_get.add_argument("--wallet", required=True)

    wallets_update = wallets_sub.add_parser("update")
    wallets_update.add_argument("--workspace")
    wallets_update.add_argument("--profile")
    wallets_update.add_argument("--wallet", required=True)
    wallets_update.add_argument("--label")
    wallets_update.add_argument("--account", help="Wallet/reporting bucket code, id, or unique label")
    wallets_update.add_argument("--backend")
    wallets_update.add_argument("--chain", choices=["bitcoin", "liquid"])
    wallets_update.add_argument("--network")
    wallets_update.add_argument("--gap-limit", type=int)
    wallets_update.add_argument("--policy-asset")
    wallets_update.add_argument("--store-id")
    wallets_update.add_argument("--payment-method-id")
    wallets_update.add_argument("--config")
    wallets_update.add_argument("--config-file")
    wallets_update.add_argument("--clear", action="append", default=[], metavar="FIELD", help="Clear a config field (repeatable)")

    wallets_delete = wallets_sub.add_parser("delete")
    wallets_delete.add_argument("--workspace")
    wallets_delete.add_argument("--profile")
    wallets_delete.add_argument("--wallet", required=True)
    wallets_delete.add_argument("--cascade", action="store_true", help="Also delete transactions and journal entries belonging to this wallet")

    wallets_reveal = wallets_sub.add_parser(
        "reveal-descriptor",
        help="Print the raw descriptor / change descriptor / blinding key (requires database unlock)",
    )
    wallets_reveal.add_argument("--workspace")
    wallets_reveal.add_argument("--profile")
    wallets_reveal.add_argument("--wallet", required=True)
    wallets_import_json = wallets_sub.add_parser("import-json")
    wallets_import_json.add_argument("--workspace")
    wallets_import_json.add_argument("--profile")
    wallets_import_json.add_argument("--wallet", required=True)
    wallets_import_json.add_argument("--file", required=True)
    wallets_import_csv = wallets_sub.add_parser("import-csv")
    wallets_import_csv.add_argument("--workspace")
    wallets_import_csv.add_argument("--profile")
    wallets_import_csv.add_argument("--wallet", required=True)
    wallets_import_csv.add_argument("--file", required=True)
    wallets_import_btcpay = wallets_sub.add_parser("import-btcpay")
    wallets_import_btcpay.add_argument("--workspace")
    wallets_import_btcpay.add_argument("--profile")
    wallets_import_btcpay.add_argument("--wallet", required=True)
    wallets_import_btcpay.add_argument("--file", required=True)
    wallets_import_btcpay.add_argument("--input-format", "--format", dest="input_format", choices=["json", "csv"], default="csv")
    wallets_import_phoenix = wallets_sub.add_parser("import-phoenix")
    wallets_import_phoenix.add_argument("--workspace")
    wallets_import_phoenix.add_argument("--profile")
    wallets_import_phoenix.add_argument("--wallet", required=True)
    wallets_import_phoenix.add_argument("--file", required=True)
    wallets_import_wasabi = wallets_sub.add_parser("import-wasabi")
    wallets_import_wasabi.add_argument("--workspace")
    wallets_import_wasabi.add_argument("--profile")
    wallets_import_wasabi.add_argument("--wallet", required=True)
    wallets_import_wasabi.add_argument("--file", required=True)

    wallets_import_river = wallets_sub.add_parser("import-river")
    wallets_import_river.add_argument("--workspace")
    wallets_import_river.add_argument("--profile")
    wallets_import_river.add_argument("--wallet", required=True)
    wallets_import_river.add_argument("--file", required=True)
    wallets_import_bull = wallets_sub.add_parser("import-bull", aliases=["import-bullbitcoin"])
    wallets_import_bull.add_argument("--workspace")
    wallets_import_bull.add_argument("--profile")
    wallets_import_bull.add_argument("--wallet")
    wallets_import_bull.add_argument("--file", required=True)
    wallets_import_bull.add_argument("--mode", choices=["relevant", "full"], default="relevant")
    wallets_import_coinfinity = wallets_sub.add_parser("import-coinfinity")
    wallets_import_coinfinity.add_argument("--workspace")
    wallets_import_coinfinity.add_argument("--profile")
    wallets_import_coinfinity.add_argument("--wallet")
    wallets_import_coinfinity.add_argument("--file", required=True)
    wallets_import_coinfinity.add_argument("--mode", choices=["relevant", "full"], default="relevant")
    wallets_import_21bitcoin = wallets_sub.add_parser("import-21bitcoin")
    wallets_import_21bitcoin.add_argument("--workspace")
    wallets_import_21bitcoin.add_argument("--profile")
    wallets_import_21bitcoin.add_argument("--wallet")
    wallets_import_21bitcoin.add_argument("--file", required=True)
    wallets_import_21bitcoin.add_argument("--mode", choices=["relevant", "full"], default="full")
    wallets_import_pocket = wallets_sub.add_parser("import-pocket", aliases=["import-pocketbitcoin"])
    wallets_import_pocket.add_argument("--workspace")
    wallets_import_pocket.add_argument("--profile")
    wallets_import_pocket.add_argument("--wallet")
    wallets_import_pocket.add_argument("--file", required=True)
    wallets_import_pocket.add_argument("--mode", choices=["relevant", "full"], default="relevant")
    wallets_import_strike = wallets_sub.add_parser("import-strike")
    wallets_import_strike.add_argument("--workspace")
    wallets_import_strike.add_argument("--profile")
    wallets_import_strike.add_argument("--wallet")
    wallets_import_strike.add_argument("--file", required=True)
    wallets_import_samourai = wallets_sub.add_parser("import-samourai")
    wallets_import_samourai.add_argument("--workspace")
    wallets_import_samourai.add_argument("--profile")
    wallets_import_samourai.add_argument("--label", required=True)
    wallets_import_samourai.add_argument("--account", help="Wallet/reporting bucket code, id, or unique label")
    wallets_import_samourai.add_argument("--backend")
    wallets_import_samourai.add_argument("--network")
    wallets_import_samourai.add_argument("--gap-limit", type=int)
    wallets_import_samourai.add_argument(
        "--source-set-file",
        required=True,
        help="Local JSON file containing explicit Samourai descriptor/xpub sources",
    )
    wallets_sync_btcpay = wallets_sub.add_parser("sync-btcpay")
    wallets_sync_btcpay.add_argument("--workspace")
    wallets_sync_btcpay.add_argument("--profile")
    wallets_sync_btcpay.add_argument("--wallet", required=True)
    wallets_sync_btcpay.add_argument("--backend", required=True)
    wallets_sync_btcpay.add_argument("--store-id", required=True, dest="store_id")
    wallets_sync_btcpay.add_argument(
        "--payment-method-id",
        default=BTCPAY_DEFAULT_PAYMENT_METHOD_ID,
        dest="payment_method_id",
    )
    wallets_sync_btcpay.add_argument(
        "--page-size",
        type=int,
        default=BTCPAY_DEFAULT_PAGE_SIZE,
        dest="page_size",
    )
    wallets_attach_btcpay = wallets_sub.add_parser(
        "attach-btcpay",
        help="Record a BTCPay provenance route on an existing settlement wallet",
    )
    wallets_attach_btcpay.add_argument("--workspace")
    wallets_attach_btcpay.add_argument("--profile")
    wallets_attach_btcpay.add_argument("--wallet", required=True)
    wallets_attach_btcpay.add_argument("--backend", required=True)
    wallets_attach_btcpay.add_argument("--store-id", required=True, dest="store_id")
    wallets_attach_btcpay.add_argument(
        "--payment-method-id",
        default=BTCPAY_DEFAULT_PAYMENT_METHOD_ID,
        dest="payment_method_id",
    )
    wallets_sync = wallets_sub.add_parser("sync")
    wallets_sync.add_argument("--workspace")
    wallets_sync.add_argument("--profile")
    wallets_sync.add_argument("--wallet")
    wallets_sync.add_argument("--all", action="store_true")
    wallets_derive = wallets_sub.add_parser("derive")
    wallets_derive.add_argument("--workspace")
    wallets_derive.add_argument("--profile")
    wallets_derive.add_argument("--wallet", required=True)
    wallets_derive.add_argument("--branch", default="all")
    wallets_derive.add_argument("--start", type=int, default=0)
    wallets_derive.add_argument("--count", type=int)

    transactions = sub.add_parser("transactions")
    tx_sub = transactions.add_subparsers(dest="transactions_command", required=True)
    tx_list = tx_sub.add_parser("list")
    tx_list.add_argument("--workspace")
    tx_list.add_argument("--profile")
    tx_list.add_argument("--wallet")
    tx_list.add_argument("--direction", choices=("inbound", "outbound"))
    tx_list.add_argument("--asset")
    tx_list.add_argument("--start", help="RFC3339 lower bound (inclusive) on occurred_at")
    tx_list.add_argument("--end", help="RFC3339 upper bound (inclusive) on occurred_at")
    tx_list.add_argument("--cursor", help="Opaque pagination cursor from a previous response")
    tx_list.add_argument(
        "--sort",
        choices=("occurred-at", "amount", "fiat-value", "fee"),
        default="occurred-at",
    )
    tx_list.add_argument("--order", choices=("asc", "desc"), default="desc")
    tx_list.add_argument("--limit", type=int, default=100)

    attachments = sub.add_parser("attachments")
    attachments_sub = attachments.add_subparsers(dest="attachments_command", required=True)
    attachments_add = attachments_sub.add_parser("add")
    attachments_add.add_argument("--workspace")
    attachments_add.add_argument("--profile")
    attachments_add.add_argument("--transaction", required=True)
    attachments_source = attachments_add.add_mutually_exclusive_group(required=True)
    attachments_source.add_argument("--file")
    attachments_source.add_argument("--url")
    attachments_add.add_argument("--label")
    attachments_add.add_argument("--media-type")

    attachments_list = attachments_sub.add_parser("list")
    attachments_list.add_argument("--workspace")
    attachments_list.add_argument("--profile")
    attachments_list.add_argument("--transaction")

    attachments_rename = attachments_sub.add_parser("rename")
    attachments_rename.add_argument("--workspace")
    attachments_rename.add_argument("--profile")
    attachments_rename.add_argument("attachment_id")
    attachments_rename.add_argument("--label", required=True)

    attachments_remove = attachments_sub.add_parser("remove")
    attachments_remove.add_argument("--workspace")
    attachments_remove.add_argument("--profile")
    attachments_remove.add_argument("attachment_id")

    attachments_verify = attachments_sub.add_parser("verify")
    attachments_verify.add_argument("--workspace")
    attachments_verify.add_argument("--profile")
    attachments_verify.add_argument("--transaction")

    attachments_gc = attachments_sub.add_parser("gc")
    attachments_gc.add_argument("--dry-run", action="store_true")

    metadata = sub.add_parser("metadata")
    meta_sub = metadata.add_subparsers(dest="metadata_command", required=True)
    notes = meta_sub.add_parser("notes")
    notes_sub = notes.add_subparsers(dest="notes_command", required=True)
    notes_set = notes_sub.add_parser("set")
    notes_set.add_argument("--workspace")
    notes_set.add_argument("--profile")
    notes_set.add_argument("--transaction", required=True)
    notes_set.add_argument("--note", required=True)
    notes_set.add_argument("--reason")
    notes_clear = notes_sub.add_parser("clear")
    notes_clear.add_argument("--workspace")
    notes_clear.add_argument("--profile")
    notes_clear.add_argument("--transaction", required=True)
    notes_clear.add_argument("--reason")
    tags = meta_sub.add_parser("tags")
    tags_sub = tags.add_subparsers(dest="tags_command", required=True)
    tags_list = tags_sub.add_parser("list")
    tags_list.add_argument("--workspace")
    tags_list.add_argument("--profile")
    tags_create = tags_sub.add_parser("create")
    tags_create.add_argument("--workspace")
    tags_create.add_argument("--profile")
    tags_create.add_argument("--code", required=True)
    tags_create.add_argument("--label", required=True)
    tags_add = tags_sub.add_parser("add")
    tags_add.add_argument("--workspace")
    tags_add.add_argument("--profile")
    tags_add.add_argument("--transaction", required=True)
    tags_add.add_argument("--tag", required=True)
    tags_add.add_argument("--reason")
    tags_remove = tags_sub.add_parser("remove")
    tags_remove.add_argument("--workspace")
    tags_remove.add_argument("--profile")
    tags_remove.add_argument("--transaction", required=True)
    tags_remove.add_argument("--tag", required=True)
    tags_remove.add_argument("--reason")
    bip329 = meta_sub.add_parser("bip329")
    bip329_sub = bip329.add_subparsers(dest="bip329_command", required=True)
    bip329_import = bip329_sub.add_parser("import")
    bip329_import.add_argument("--workspace")
    bip329_import.add_argument("--profile")
    bip329_import.add_argument("--wallet")
    bip329_import.add_argument("--file", required=True)
    bip329_list = bip329_sub.add_parser("list")
    bip329_list.add_argument("--workspace")
    bip329_list.add_argument("--profile")
    bip329_list.add_argument("--wallet")
    bip329_list.add_argument("--cursor")
    bip329_list.add_argument("--limit", type=int, default=core_metadata.DEFAULT_RECORDS_LIMIT)
    bip329_export = bip329_sub.add_parser("export")
    bip329_export.add_argument("--workspace")
    bip329_export.add_argument("--profile")
    bip329_export.add_argument("--wallet")
    bip329_export.add_argument("--file", required=True)
    exclude = meta_sub.add_parser("exclude")
    exclude.add_argument("--workspace")
    exclude.add_argument("--profile")
    exclude.add_argument("--transaction", required=True)
    exclude.add_argument("--reason")
    include = meta_sub.add_parser("include")
    include.add_argument("--workspace")
    include.add_argument("--profile")
    include.add_argument("--transaction", required=True)
    include.add_argument("--reason")

    records = meta_sub.add_parser("records")
    records_sub = records.add_subparsers(dest="records_command", required=True)

    records_list = records_sub.add_parser("list")
    records_list.add_argument("--workspace")
    records_list.add_argument("--profile")
    records_list.add_argument("--wallet")
    records_list.add_argument("--tag")
    records_list.add_argument("--has-note", dest="has_note", action="store_true")
    records_list.add_argument("--no-note", dest="no_note", action="store_true")
    records_list.add_argument("--excluded", action="store_true")
    records_list.add_argument("--included", action="store_true")
    records_list.add_argument("--start")
    records_list.add_argument("--end")
    records_list.add_argument("--cursor")
    records_list.add_argument("--limit", type=int, default=core_metadata.DEFAULT_RECORDS_LIMIT)

    records_get = records_sub.add_parser("get")
    records_get.add_argument("--workspace")
    records_get.add_argument("--profile")
    records_get.add_argument("--transaction", required=True)

    records_note = records_sub.add_parser("note")
    records_note_sub = records_note.add_subparsers(dest="records_note_command", required=True)
    rn_set = records_note_sub.add_parser("set")
    rn_set.add_argument("--workspace")
    rn_set.add_argument("--profile")
    rn_set.add_argument("--transaction", required=True)
    rn_set.add_argument("--note", required=True)
    rn_set.add_argument("--reason")
    rn_clear = records_note_sub.add_parser("clear")
    rn_clear.add_argument("--workspace")
    rn_clear.add_argument("--profile")
    rn_clear.add_argument("--transaction", required=True)
    rn_clear.add_argument("--reason")

    records_tag = records_sub.add_parser("tag")
    records_tag_sub = records_tag.add_subparsers(dest="records_tag_command", required=True)
    rt_add = records_tag_sub.add_parser("add")
    rt_add.add_argument("--workspace")
    rt_add.add_argument("--profile")
    rt_add.add_argument("--transaction", required=True)
    rt_add.add_argument("--tag", required=True)
    rt_add.add_argument("--reason")
    rt_remove = records_tag_sub.add_parser("remove")
    rt_remove.add_argument("--workspace")
    rt_remove.add_argument("--profile")
    rt_remove.add_argument("--transaction", required=True)
    rt_remove.add_argument("--tag", required=True)
    rt_remove.add_argument("--reason")

    records_excluded = records_sub.add_parser("excluded")
    records_excluded_sub = records_excluded.add_subparsers(dest="records_excluded_command", required=True)
    re_set = records_excluded_sub.add_parser("set")
    re_set.add_argument("--workspace")
    re_set.add_argument("--profile")
    re_set.add_argument("--transaction", required=True)
    re_set.add_argument("--reason")
    re_clear = records_excluded_sub.add_parser("clear")
    re_clear.add_argument("--workspace")
    re_clear.add_argument("--profile")
    re_clear.add_argument("--transaction", required=True)
    re_clear.add_argument("--reason")

    history = records_sub.add_parser("history")
    history_sub = history.add_subparsers(dest="history_command", required=True)
    history_list = history_sub.add_parser("list")
    history_list.add_argument("--workspace")
    history_list.add_argument("--profile")
    history_list.add_argument("--transaction", required=True)
    history_list.add_argument("--source")
    history_list.add_argument("--field-family")
    history_list.add_argument("--field")
    history_list.add_argument("--pricing-only", action="store_true")
    history_list.add_argument("--ai-only", action="store_true")
    history_list.add_argument("--stale-only", action="store_true")
    history_list.add_argument("--start")
    history_list.add_argument("--end")
    history_list.add_argument("--cursor")
    history_list.add_argument("--limit", type=int, default=core_metadata.DEFAULT_RECORDS_LIMIT)

    history_activity = history_sub.add_parser("activity")
    history_activity.add_argument("--workspace")
    history_activity.add_argument("--profile")
    history_activity.add_argument("--transaction")
    history_activity.add_argument("--wallet")
    history_activity.add_argument("--source")
    history_activity.add_argument("--field-family")
    history_activity.add_argument("--field")
    history_activity.add_argument("--pricing-only", action="store_true")
    history_activity.add_argument("--ai-only", action="store_true")
    history_activity.add_argument("--stale-only", action="store_true")
    history_activity.add_argument("--start")
    history_activity.add_argument("--end")
    history_activity.add_argument("--cursor")
    history_activity.add_argument("--limit", type=int, default=core_metadata.DEFAULT_RECORDS_LIMIT)

    history_stale = history_sub.add_parser("stale")
    history_stale.add_argument("--workspace")
    history_stale.add_argument("--profile")

    history_revert = history_sub.add_parser("revert")
    history_revert.add_argument("--workspace")
    history_revert.add_argument("--profile")
    history_revert.add_argument("--transaction", required=True)
    history_revert.add_argument("--event", required=True)
    history_revert.add_argument("--field")
    history_revert.add_argument("--reason")

    journals = sub.add_parser("journals")
    journals_sub = journals.add_subparsers(dest="journals_command", required=True)
    journals_process = journals_sub.add_parser("process")
    journals_process.add_argument("--workspace")
    journals_process.add_argument("--profile")
    journals_list = journals_sub.add_parser("list")
    journals_list.add_argument("--workspace")
    journals_list.add_argument("--profile")
    journals_list.add_argument("--cursor", help="Opaque pagination cursor from a previous response")
    journals_list.add_argument("--limit", type=int, default=200)
    journals_quarantined = journals_sub.add_parser("quarantined")
    journals_quarantined.add_argument("--workspace")
    journals_quarantined.add_argument("--profile")

    journals_transfers = journals_sub.add_parser("transfers")
    journal_transfers_sub = journals_transfers.add_subparsers(dest="journal_transfers_command", required=True)
    journal_transfers_list = journal_transfers_sub.add_parser("list")
    journal_transfers_list.add_argument("--workspace")
    journal_transfers_list.add_argument("--profile")

    journals_events = journals_sub.add_parser("events")
    events_sub = journals_events.add_subparsers(dest="events_command", required=True)
    events_list = events_sub.add_parser("list")
    events_list.add_argument("--workspace")
    events_list.add_argument("--profile")
    events_list.add_argument("--wallet")
    events_list.add_argument("--account")
    events_list.add_argument("--asset")
    events_list.add_argument("--entry-type", help="Filter by entry type (debit, credit, etc.)")
    events_list.add_argument("--start", help="RFC3339 lower bound (inclusive) on occurred_at")
    events_list.add_argument("--end", help="RFC3339 upper bound (inclusive) on occurred_at")
    events_list.add_argument("--cursor", help="Opaque pagination cursor from a previous response")
    events_list.add_argument("--limit", type=int, default=DEFAULT_EVENTS_LIMIT)
    events_get = events_sub.add_parser("get")
    events_get.add_argument("--workspace")
    events_get.add_argument("--profile")
    events_get.add_argument("--event-id", required=True)

    journals_quarantine = journals_sub.add_parser("quarantine")
    qsub = journals_quarantine.add_subparsers(dest="quarantine_command", required=True)

    q_show = qsub.add_parser("show")
    q_show.add_argument("--workspace")
    q_show.add_argument("--profile")
    q_show.add_argument("--transaction", required=True)

    q_clear = qsub.add_parser("clear")
    q_clear.add_argument("--workspace")
    q_clear.add_argument("--profile")
    q_clear.add_argument("--transaction", required=True)

    q_resolve = qsub.add_parser("resolve")
    qrsub = q_resolve.add_subparsers(dest="quarantine_resolve_command", required=True)

    q_price = qrsub.add_parser("price-override")
    q_price.add_argument("--workspace")
    q_price.add_argument("--profile")
    q_price.add_argument("--transaction", required=True)
    q_price.add_argument("--fiat-rate")
    q_price.add_argument("--fiat-value")

    q_exclude = qrsub.add_parser("exclude")
    q_exclude.add_argument("--workspace")
    q_exclude.add_argument("--profile")
    q_exclude.add_argument("--transaction", required=True)

    transfers = sub.add_parser("transfers")
    transfers_sub = transfers.add_subparsers(dest="transfers_command", required=True)
    transfers_list = transfers_sub.add_parser("list")
    transfers_list.add_argument("--workspace")
    transfers_list.add_argument("--profile")
    transfers_pair = transfers_sub.add_parser("pair")
    transfers_pair.add_argument("--workspace")
    transfers_pair.add_argument("--profile")
    transfers_pair.add_argument("--tx-out", required=True, dest="tx_out", help="Outbound transaction id or external_id")
    transfers_pair.add_argument("--tx-in", required=True, dest="tx_in", help="Inbound transaction id or external_id")
    transfers_pair.add_argument("--kind", choices=list(TRANSFER_PAIR_KINDS), default="manual")
    transfers_pair.add_argument("--policy", choices=list(TRANSFER_PAIR_POLICIES), default="carrying-value")
    transfers_pair.add_argument("--note", dest="note")
    transfers_unpair = transfers_sub.add_parser("unpair")
    transfers_unpair.add_argument("--workspace")
    transfers_unpair.add_argument("--profile")
    transfers_unpair.add_argument("--pair-id", required=True, dest="pair_id")

    transfers_payouts = transfers_sub.add_parser("payouts")
    transfers_payouts_sub = transfers_payouts.add_subparsers(dest="payouts_command", required=True)
    transfers_payouts_list = transfers_payouts_sub.add_parser("list")
    transfers_payouts_list.add_argument("--workspace")
    transfers_payouts_list.add_argument("--profile")
    transfers_payouts_create = transfers_payouts_sub.add_parser("create")
    transfers_payouts_create.add_argument("--workspace")
    transfers_payouts_create.add_argument("--profile")
    transfers_payouts_create.add_argument("--tx-out", required=True, dest="tx_out", help="Outbound transaction id or external_id")
    transfers_payouts_create.add_argument("--payout-asset", required=True, dest="payout_asset")
    transfers_payouts_create.add_argument("--payout-amount", required=True, dest="payout_amount", help="Target asset amount paid externally, in BTC units")
    transfers_payouts_create.add_argument("--payout-occurred-at", dest="payout_occurred_at")
    transfers_payouts_create.add_argument("--payout-fiat-value", dest="payout_fiat_value")
    transfers_payouts_create.add_argument("--payout-external-id", dest="payout_external_id")
    transfers_payouts_create.add_argument("--counterparty")
    transfers_payouts_create.add_argument("--policy", choices=list(TRANSFER_PAIR_POLICIES), default="carrying-value")
    transfers_payouts_create.add_argument("--note", dest="note")
    transfers_payouts_delete = transfers_payouts_sub.add_parser("delete")
    transfers_payouts_delete.add_argument("--workspace")
    transfers_payouts_delete.add_argument("--profile")
    transfers_payouts_delete.add_argument("--payout-id", required=True, dest="payout_id")

    transfers_suggest = transfers_sub.add_parser("suggest")
    transfers_suggest.add_argument("--workspace")
    transfers_suggest.add_argument("--profile")
    transfers_suggest.add_argument("--confidence", choices=("exact", "strong"))
    transfers_suggest.add_argument("--method", choices=("payment_hash", "heuristic"))
    transfers_suggest.add_argument(
        "--asset-pair",
        dest="asset_pair",
        help="Match OUT-IN asset shape, e.g. LBTC-BTC for a peg-out",
    )
    transfers_suggest.add_argument(
        "--candidate-type",
        choices=("transfer", "swap"),
        dest="candidate_type",
        help="Restrict candidates to same-asset transfers or cross-asset swaps",
    )
    transfers_suggest.add_argument(
        "--time-window-seconds",
        dest="time_window_seconds",
        type=int,
        default=24 * 60 * 60,
    )
    transfers_suggest.add_argument(
        "--fee-pct-max", dest="fee_pct_max", type=float, default=0.01
    )
    transfers_suggest.add_argument(
        "--fee-sats-min", dest="fee_sats_min", type=int, default=2500
    )

    transfers_bulk_pair = transfers_sub.add_parser("bulk-pair")
    transfers_bulk_pair.add_argument("--workspace")
    transfers_bulk_pair.add_argument("--profile")
    transfers_bulk_pair.add_argument(
        "--confidence", choices=("exact", "strong"), default="exact"
    )
    transfers_bulk_pair.add_argument("--method", choices=("payment_hash", "heuristic"))
    transfers_bulk_pair.add_argument(
        "--asset-pair",
        dest="asset_pair",
        help="Restrict to OUT-IN asset shape, e.g. LBTC-BTC for a peg-out",
    )
    transfers_bulk_pair.add_argument(
        "--candidate-type",
        choices=("transfer", "swap"),
        dest="candidate_type",
        help="Restrict candidates to same-asset transfers or cross-asset swaps",
    )
    transfers_bulk_pair.add_argument(
        "--time-window-seconds",
        dest="time_window_seconds",
        type=int,
        default=24 * 60 * 60,
    )
    transfers_bulk_pair.add_argument(
        "--fee-pct-max", dest="fee_pct_max", type=float, default=0.01
    )
    transfers_bulk_pair.add_argument(
        "--fee-sats-min", dest="fee_sats_min", type=int, default=2500
    )

    transfers_dismiss = transfers_sub.add_parser("dismiss")
    transfers_dismiss.add_argument("--workspace")
    transfers_dismiss.add_argument("--profile")
    transfers_dismiss.add_argument("--tx-out", dest="tx_out", required=True)
    transfers_dismiss.add_argument("--tx-in", dest="tx_in", required=True)
    transfers_dismiss.add_argument("--reason")
    transfers_dismiss.add_argument(
        "--expires-in-days",
        dest="expires_in_days",
        type=int,
        default=90,
        help="0 = never expire",
    )

    transfers_rules = transfers_sub.add_parser("rules")
    transfers_rules_sub = transfers_rules.add_subparsers(
        dest="transfers_rules_command", required=True
    )
    tr_rules_list = transfers_rules_sub.add_parser("list")
    tr_rules_list.add_argument("--workspace")
    tr_rules_list.add_argument("--profile")
    tr_rules_create = transfers_rules_sub.add_parser("create")
    tr_rules_create.add_argument("--workspace")
    tr_rules_create.add_argument("--profile")
    tr_rules_create.add_argument("--name")
    tr_rules_create.add_argument(
        "--predicate",
        default="{}",
        help="Inline JSON predicate (matches out_wallet_id, in_wallet_id, "
        "out_asset, in_asset, out_wallet_kind, in_wallet_kind, max_fee_pct, "
        "min_confidence)",
    )
    tr_rules_create.add_argument("--kind", choices=list(TRANSFER_PAIR_KINDS), default="manual")
    tr_rules_create.add_argument(
        "--policy", choices=list(TRANSFER_PAIR_POLICIES), default="carrying-value"
    )
    tr_rules_create.add_argument("--disabled", action="store_true")
    tr_rules_delete = transfers_rules_sub.add_parser("delete")
    tr_rules_delete.add_argument("--workspace")
    tr_rules_delete.add_argument("--profile")
    tr_rules_delete.add_argument("--rule-id", required=True, dest="rule_id")
    tr_rules_enable = transfers_rules_sub.add_parser("enable")
    tr_rules_enable.add_argument("--workspace")
    tr_rules_enable.add_argument("--profile")
    tr_rules_enable.add_argument("--rule-id", required=True, dest="rule_id")
    tr_rules_disable = transfers_rules_sub.add_parser("disable")
    tr_rules_disable.add_argument("--workspace")
    tr_rules_disable.add_argument("--profile")
    tr_rules_disable.add_argument("--rule-id", required=True, dest="rule_id")
    tr_rules_apply = transfers_rules_sub.add_parser("apply")
    tr_rules_apply.add_argument("--workspace")
    tr_rules_apply.add_argument("--profile")
    tr_rules_apply.add_argument("--confidence", choices=("exact", "strong"))
    tr_rules_apply.add_argument("--method", choices=("payment_hash", "heuristic"))
    tr_rules_apply.add_argument(
        "--asset-pair",
        dest="asset_pair",
        help="Restrict to OUT-IN asset shape, e.g. LBTC-BTC for a peg-out",
    )
    tr_rules_apply.add_argument(
        "--candidate-type",
        choices=("transfer", "swap"),
        dest="candidate_type",
        help="Restrict candidates to same-asset transfers or cross-asset swaps",
    )
    tr_rules_apply.add_argument(
        "--time-window-seconds",
        dest="time_window_seconds",
        type=int,
        default=24 * 60 * 60,
    )
    tr_rules_apply.add_argument(
        "--fee-pct-max", dest="fee_pct_max", type=float, default=0.01
    )
    tr_rules_apply.add_argument(
        "--fee-sats-min", dest="fee_sats_min", type=int, default=2500
    )

    views = sub.add_parser("views")
    views_sub = views.add_subparsers(dest="views_command", required=True)
    views_list = views_sub.add_parser("list")
    views_list.add_argument("--workspace")
    views_list.add_argument("--profile")
    views_list.add_argument("--surface")
    views_create = views_sub.add_parser("create")
    views_create.add_argument("--workspace")
    views_create.add_argument("--profile")
    views_create.add_argument("--surface", required=True)
    views_create.add_argument("--name", required=True)
    views_create.add_argument(
        "--filter",
        default="{}",
        dest="filter_json",
        help="Inline JSON filter payload (round-tripped opaque to this layer)",
    )
    views_delete = views_sub.add_parser("delete")
    views_delete.add_argument("--workspace")
    views_delete.add_argument("--profile")
    views_delete.add_argument("--view-id", required=True, dest="view_id")

    btcpay = sub.add_parser("btcpay")
    btcpay_sub = btcpay.add_subparsers(dest="btcpay_command", required=True)
    btcpay_provenance = btcpay_sub.add_parser("provenance")
    btcpay_provenance_sub = btcpay_provenance.add_subparsers(dest="btcpay_provenance_command", required=True)
    btcpay_sync = btcpay_provenance_sub.add_parser("sync")
    btcpay_sync.add_argument("--workspace")
    btcpay_sync.add_argument("--profile")
    btcpay_sync.add_argument("--backend", required=True)
    btcpay_sync.add_argument("--store-id", required=True, dest="store_id")
    btcpay_sync.add_argument("--page-size", type=int, default=BTCPAY_DEFAULT_PAGE_SIZE, dest="page_size")
    btcpay_list = btcpay_provenance_sub.add_parser("list")
    btcpay_list.add_argument("--workspace")
    btcpay_list.add_argument("--profile")
    btcpay_list.add_argument("--record-type", choices=("invoice", "payment"))
    btcpay_list.add_argument("--limit", type=int, default=100)
    btcpay_suggest = btcpay_provenance_sub.add_parser("suggest")
    btcpay_suggest.add_argument("--workspace")
    btcpay_suggest.add_argument("--profile")
    btcpay_suggest.add_argument("--limit", type=int, default=core_commercial.SUGGESTION_LIMIT)
    btcpay_links = btcpay_provenance_sub.add_parser("links")
    btcpay_links.add_argument("--workspace")
    btcpay_links.add_argument("--profile")
    btcpay_links.add_argument("--state", choices=list(core_commercial.LINK_STATES))
    btcpay_links.add_argument("--limit", type=int, default=100)
    btcpay_review = btcpay_provenance_sub.add_parser("review")
    btcpay_review.add_argument("--workspace")
    btcpay_review.add_argument("--profile")
    btcpay_review.add_argument("--link", required=True)
    btcpay_review.add_argument("--state", required=True, choices=list(core_commercial.LINK_STATES))
    btcpay_review.add_argument("--reconciliation-state", choices=list(core_commercial.RECONCILIATION_STATES))
    btcpay_review.add_argument("--commercial-kind", choices=list(core_commercial.COMMERCIAL_KINDS))
    btcpay_review.add_argument("--notes")

    documents = sub.add_parser("documents")
    documents_sub = documents.add_subparsers(dest="documents_command", required=True)
    documents_list = documents_sub.add_parser("list")
    documents_list.add_argument("--workspace")
    documents_list.add_argument("--profile")
    documents_list.add_argument("--limit", type=int, default=100)
    documents_create = documents_sub.add_parser("create")
    documents_create.add_argument("--workspace")
    documents_create.add_argument("--profile")
    documents_create.add_argument("--type", required=True, dest="document_type", choices=list(core_commercial.DOCUMENT_TYPES))
    documents_create.add_argument("--label", required=True)
    documents_create.add_argument("--external-ref")
    documents_create.add_argument("--issuer")
    documents_create.add_argument("--counterparty")
    documents_create.add_argument("--issued-at")
    documents_create.add_argument("--due-at")
    documents_create.add_argument("--fiat-currency")
    documents_create.add_argument("--fiat-value")
    documents_create.add_argument("--notes")
    documents_attach = documents_sub.add_parser("attach")
    documents_attach.add_argument("--workspace")
    documents_attach.add_argument("--profile")
    documents_attach.add_argument("--document", required=True)
    documents_attach.add_argument("--file")
    documents_attach.add_argument("--url")
    documents_attach.add_argument("--label")
    documents_attach.add_argument("--media-type")

    source_funds = sub.add_parser("source-funds")
    source_funds_sub = source_funds.add_subparsers(dest="source_funds_command", required=True)

    sf_sources = source_funds_sub.add_parser("sources")
    sf_sources_sub = sf_sources.add_subparsers(dest="source_funds_sources_command", required=True)
    sf_sources_list = sf_sources_sub.add_parser("list")
    sf_sources_list.add_argument("--workspace")
    sf_sources_list.add_argument("--profile")
    sf_sources_create = sf_sources_sub.add_parser("create")
    sf_sources_create.add_argument("--workspace")
    sf_sources_create.add_argument("--profile")
    sf_sources_create.add_argument("--type", required=True, dest="source_type", choices=list(core_source_funds.SOURCE_TYPES))
    sf_sources_create.add_argument("--label", required=True)
    sf_sources_create.add_argument("--asset", default="BTC")
    sf_sources_create.add_argument("--amount")
    sf_sources_create.add_argument("--fiat-currency")
    sf_sources_create.add_argument("--fiat-value")
    sf_sources_create.add_argument("--acquired-at")
    sf_sources_create.add_argument("--description")
    sf_sources_create.add_argument("--attachment", action="append", default=[], dest="attachments")
    sf_sources_attach = sf_sources_sub.add_parser("attach")
    sf_sources_attach.add_argument("--workspace")
    sf_sources_attach.add_argument("--profile")
    sf_sources_attach.add_argument("--source", required=True)
    sf_sources_attach.add_argument("--attachment", required=True)

    sf_links = source_funds_sub.add_parser("links")
    sf_links_sub = sf_links.add_subparsers(dest="source_funds_links_command", required=True)
    sf_links_list = sf_links_sub.add_parser("list")
    sf_links_list.add_argument("--workspace")
    sf_links_list.add_argument("--profile")
    sf_links_list.add_argument("--target-transaction")
    sf_links_list.add_argument("--state", choices=list(core_source_funds.LINK_STATES))
    sf_links_create = sf_links_sub.add_parser("create")
    sf_links_create.add_argument("--workspace")
    sf_links_create.add_argument("--profile")
    sf_links_create.add_argument("--from-transaction")
    sf_links_create.add_argument("--from-source")
    sf_links_create.add_argument("--to-transaction", required=True)
    sf_links_create.add_argument("--type", required=True, dest="link_type", choices=list(core_source_funds.LINK_TYPES))
    sf_links_create.add_argument("--state", choices=list(core_source_funds.LINK_STATES), default="reviewed")
    sf_links_create.add_argument("--confidence", choices=list(core_source_funds.CONFIDENCE_LEVELS), default="strong")
    sf_links_create.add_argument("--method", default="manual")
    sf_links_create.add_argument("--asset")
    sf_links_create.add_argument("--allocation-amount")
    sf_links_create.add_argument("--from-asset")
    sf_links_create.add_argument("--from-amount", dest="from_amount")
    sf_links_create.add_argument("--allocation-policy", choices=list(core_source_funds.ALLOCATION_POLICIES), default="explicit")
    sf_links_create.add_argument("--explanation")
    sf_links_create.add_argument("--uses-chain-observation", action="store_true")
    sf_links_create.add_argument(
        "--chain-data-confirmed",
        action="store_true",
        help="Mark this chain observation as independently confirmed; "
        "without this flag the link is created unconfirmed and "
        "cannot satisfy the export gate.",
    )
    sf_links_create.add_argument("--attachment", action="append", default=[], dest="attachments")
    sf_links_review = sf_links_sub.add_parser("review")
    sf_links_review.add_argument("--workspace")
    sf_links_review.add_argument("--profile")
    sf_links_review.add_argument("--link", required=True)
    sf_links_review.add_argument("--state", choices=list(core_source_funds.LINK_STATES))
    sf_links_review.add_argument("--type", dest="link_type", choices=list(core_source_funds.LINK_TYPES))
    sf_links_review.add_argument("--confidence", choices=list(core_source_funds.CONFIDENCE_LEVELS))
    sf_links_review.add_argument("--allocation-amount")
    sf_links_review.add_argument("--from-amount", dest="from_amount")
    sf_links_review.add_argument("--allocation-policy", choices=list(core_source_funds.ALLOCATION_POLICIES))
    sf_links_review.add_argument("--explanation")
    sf_links_review.add_argument("--uses-chain-observation", action="store_true", default=None)
    sf_links_review.add_argument("--no-chain-observation", action="store_false", dest="uses_chain_observation")
    sf_links_review.add_argument("--chain-data-confirmed", action="store_true", default=None)
    sf_links_review.add_argument("--unconfirmed-chain-data", action="store_false", dest="chain_data_confirmed")
    sf_links_attach = sf_links_sub.add_parser("attach")
    sf_links_attach.add_argument("--workspace")
    sf_links_attach.add_argument("--profile")
    sf_links_attach.add_argument("--link", required=True)
    sf_links_attach.add_argument("--attachment", required=True)
    sf_links_bulk_review = sf_links_sub.add_parser("bulk-review")
    sf_links_bulk_review.add_argument("--workspace")
    sf_links_bulk_review.add_argument("--profile")
    sf_links_bulk_review.add_argument("--target-transaction", required=True)

    sf_suggest = source_funds_sub.add_parser("suggest")
    sf_suggest.add_argument("--workspace")
    sf_suggest.add_argument("--profile")
    sf_suggest.add_argument("--target-transaction")
    sf_suggest.add_argument("--include-broad-hints", action="store_true")
    sf_suggest.add_argument("--max-suggestions", type=int, default=core_source_funds.SUGGESTION_WRITE_CAP)

    sf_cases = source_funds_sub.add_parser("cases")
    sf_cases_sub = sf_cases.add_subparsers(dest="source_funds_cases_command", required=True)
    sf_cases_list = sf_cases_sub.add_parser("list")
    sf_cases_list.add_argument("--workspace")
    sf_cases_list.add_argument("--profile")

    sf_coverage = source_funds_sub.add_parser("coverage")
    sf_coverage.add_argument("--workspace")
    sf_coverage.add_argument("--profile")
    sf_coverage.add_argument("--max-depth", type=int, default=core_source_funds_coverage.DEFAULT_MAX_DEPTH)
    sf_coverage.add_argument("--max-transactions", type=int, default=core_source_funds_coverage.DEFAULT_MAX_TRANSACTIONS)

    sf_recipients = source_funds_sub.add_parser("recipients")
    sf_recipients_sub = sf_recipients.add_subparsers(
        dest="source_funds_recipients_command", required=True
    )
    sf_recipients_list = sf_recipients_sub.add_parser("list")
    sf_recipients_list.add_argument("--workspace")
    sf_recipients_list.add_argument("--profile")
    sf_recipients_create = sf_recipients_sub.add_parser("create")
    sf_recipients_create.add_argument("--workspace")
    sf_recipients_create.add_argument("--profile")
    sf_recipients_create.add_argument("--label", required=True)
    sf_recipients_create.add_argument(
        "--kind",
        required=True,
        choices=list(core_source_funds_recipients.RECIPIENT_KINDS),
    )
    sf_recipients_create.add_argument(
        "--default-reveal-mode",
        choices=list(core_source_funds.REVEAL_MODES),
        default="standard",
    )
    sf_recipients_create.add_argument("--notes")
    sf_recipients_update = sf_recipients_sub.add_parser("update")
    sf_recipients_update.add_argument("--workspace")
    sf_recipients_update.add_argument("--profile")
    sf_recipients_update.add_argument("--recipient", required=True)
    sf_recipients_update.add_argument("--label")
    sf_recipients_update.add_argument(
        "--kind",
        choices=list(core_source_funds_recipients.RECIPIENT_KINDS),
    )
    sf_recipients_update.add_argument(
        "--default-reveal-mode",
        choices=list(core_source_funds.REVEAL_MODES),
    )
    sf_recipients_update.add_argument("--notes")
    sf_recipients_delete = sf_recipients_sub.add_parser("delete")
    sf_recipients_delete.add_argument("--workspace")
    sf_recipients_delete.add_argument("--profile")
    sf_recipients_delete.add_argument("--recipient", required=True)

    reports = sub.add_parser("reports")
    reports_sub = reports.add_subparsers(dest="reports_command", required=True)
    for report_name in ["summary", "tax-summary", "balance-sheet", "portfolio-summary", "capital-gains", "journal-entries"]:
        report = reports_sub.add_parser(report_name)
        report.add_argument("--workspace")
        report.add_argument("--profile")
        if report_name == "summary":
            report.add_argument("--wallet")

    for report_name in ("austrian-e1kv", "austrian-tax-summary"):
        _add_austrian_e1kv_report_args(reports_sub.add_parser(report_name))

    balance_history = reports_sub.add_parser("balance-history")
    balance_history.add_argument("--workspace")
    balance_history.add_argument("--profile")
    balance_history.add_argument(
        "--interval",
        choices=list(core_reports.INTERVAL_CHOICES),
        default=core_reports.DEFAULT_BALANCE_HISTORY_INTERVAL,
    )
    balance_history.add_argument("--start")
    balance_history.add_argument("--end")
    balance_history.add_argument("--wallet")
    balance_history.add_argument("--account")
    balance_history.add_argument("--asset")

    export_pdf = reports_sub.add_parser("export-pdf")
    export_pdf.add_argument("--workspace")
    export_pdf.add_argument("--profile")
    export_pdf.add_argument("--wallet")
    export_pdf.add_argument("--file", required=True)
    export_pdf.add_argument("--history-limit", type=int, default=0)

    export_summary_pdf = reports_sub.add_parser("export-summary-pdf")
    export_summary_pdf.add_argument("--workspace")
    export_summary_pdf.add_argument("--profile")
    export_summary_pdf.add_argument("--wallet", action="append", dest="wallets")
    export_summary_pdf.add_argument("--start")
    export_summary_pdf.add_argument("--end")
    export_summary_pdf.add_argument("--include-snapshot", action="store_true")
    export_summary_pdf.add_argument("--file", required=True)

    export_csv = reports_sub.add_parser("export-csv")
    export_csv.add_argument("--workspace")
    export_csv.add_argument("--profile")
    export_csv.add_argument("--wallet")
    export_csv.add_argument("--file", required=True)
    export_csv.add_argument("--history-limit", type=int, default=0)

    export_xlsx = reports_sub.add_parser("export-xlsx")
    export_xlsx.add_argument("--workspace")
    export_xlsx.add_argument("--profile")
    export_xlsx.add_argument("--wallet")
    export_xlsx.add_argument("--file", required=True)
    export_xlsx.add_argument("--history-limit", type=int, default=0)

    lightning_profitability = reports_sub.add_parser("lightning-profitability")
    lightning_profitability.add_argument("--workspace")
    lightning_profitability.add_argument("--profile")
    lightning_profitability.add_argument(
        "--connection",
        required=True,
        help="Lightning connection (wallet id or label) to report on.",
    )
    lightning_profitability.add_argument(
        "--window-days",
        type=_lightning_window_days,
        default=30,
        help="Routing window in days (1-365, default 30).",
    )

    export_lightning_profitability_csv = reports_sub.add_parser(
        "export-lightning-profitability-csv"
    )
    export_lightning_profitability_csv.add_argument("--workspace")
    export_lightning_profitability_csv.add_argument("--profile")
    export_lightning_profitability_csv.add_argument(
        "--connection",
        required=True,
        help="Lightning connection (wallet id or label) to export.",
    )
    export_lightning_profitability_csv.add_argument(
        "--window-days",
        type=_lightning_window_days,
        default=30,
        help="Routing window in days (1-365, default 30).",
    )
    export_lightning_profitability_csv.add_argument("--file", required=True)

    commercial_subledger = reports_sub.add_parser("commercial-subledger")
    commercial_subledger.add_argument("--workspace")
    commercial_subledger.add_argument("--profile")

    export_commercial_subledger = reports_sub.add_parser("export-commercial-subledger-csv")
    export_commercial_subledger.add_argument("--workspace")
    export_commercial_subledger.add_argument("--profile")
    export_commercial_subledger.add_argument("--file", required=True)

    source_funds_report = reports_sub.add_parser("source-funds")
    source_funds_report.add_argument("--workspace")
    source_funds_report.add_argument("--profile")
    source_funds_report.add_argument("--target-transaction", required=True)
    source_funds_report.add_argument("--target-amount")
    source_funds_report.add_argument(
        "--purpose",
        choices=list(core_source_funds.REPORT_PURPOSES),
        default="existing_transaction",
    )
    source_funds_report.add_argument("--planned-destination")
    source_funds_report.add_argument("--planned-note")
    source_funds_report.add_argument("--reveal-mode", choices=list(core_source_funds.REVEAL_MODES))
    source_funds_report.add_argument("--max-depth", type=int, default=8)
    source_funds_report.add_argument(
        "--diagram-detail",
        choices=list(source_funds_diagram.DIAGRAM_DETAIL_LEVELS),
        default="summary",
        help="Simplified-flow detail: 'summary' clusters long paths, 'detailed' shows more hops.",
    )
    source_funds_report.add_argument(
        "--amount-precision",
        choices=("btc", "sats"),
        default="btc",
        help="Render report amounts as BTC (8dp) or whole sats.",
    )
    source_funds_report.add_argument(
        "--mask-recipient",
        action="store_true",
        help="Mask the recipient label in the exported report.",
    )
    source_funds_report.add_argument(
        "--omit-section",
        action="append",
        choices=list(core_source_funds.OPTIONAL_REPORT_SECTIONS),
        default=[],
        help="Omit a verbose PDF section (repeatable).",
    )
    source_funds_report.add_argument(
        "--reveal-override",
        action="append",
        default=[],
        metavar="TXID=show|hide",
        help="Per-transaction reveal override (repeatable): '<tx-id>=show' or '<tx-id>=hide'.",
    )
    source_funds_report.add_argument("--save-case", action="store_true")
    source_funds_report.add_argument("--case-label")
    source_funds_report.add_argument("--recipient")

    export_source_funds_pdf = reports_sub.add_parser("export-source-funds-pdf")
    export_source_funds_pdf.add_argument("--workspace")
    export_source_funds_pdf.add_argument("--profile")
    export_source_funds_pdf.add_argument(
        "--case",
        help="Saved case id from `reports source-funds --save-case`; the snapshot freezes target, reveal mode, and report options.",
    )
    export_source_funds_pdf.add_argument("--file", required=True)

    export_source_funds_bundle = reports_sub.add_parser("export-source-funds-bundle")
    export_source_funds_bundle.add_argument("--workspace")
    export_source_funds_bundle.add_argument("--profile")
    export_source_funds_bundle.add_argument("--case")
    export_source_funds_bundle.add_argument("--file", required=True)

    for report_name in ("export-austrian-e1kv-pdf", "export-austrian"):
        _add_austrian_e1kv_pdf_args(reports_sub.add_parser(report_name))

    export_austrian_e1kv_xlsx = reports_sub.add_parser("export-austrian-e1kv-xlsx")
    export_austrian_e1kv_xlsx.add_argument("--workspace")
    export_austrian_e1kv_xlsx.add_argument("--profile")
    export_austrian_e1kv_xlsx.add_argument("--year", type=int, required=True, help="Four-digit tax year")
    export_austrian_e1kv_xlsx.add_argument("--file", required=True)

    export_austrian_e1kv_csv = reports_sub.add_parser("export-austrian-e1kv-csv")
    export_austrian_e1kv_csv.add_argument("--workspace")
    export_austrian_e1kv_csv.add_argument("--profile")
    export_austrian_e1kv_csv.add_argument("--year", type=int, required=True, help="Four-digit tax year")
    export_austrian_e1kv_csv.add_argument("--dir", required=True)

    rates = sub.add_parser("rates")
    rates_sub = rates.add_subparsers(dest="rates_command", required=True)

    rates_pairs = rates_sub.add_parser("pairs")
    rates_pairs.set_defaults(rates_command="pairs")
    _ = rates_pairs

    rates_sync = rates_sub.add_parser("sync")
    rates_sync.add_argument("--pair")
    rates_sync.add_argument("--days", type=int, default=30)
    rates_sync.add_argument(
        "--source",
        default=core_rates.RATE_SOURCE_COINBASE_EXCHANGE,
        help=f"Rate source ({', '.join(core_rates.SUPPORTED_RATE_SOURCES)})",
    )
    rates_sync.add_argument(
        "--path",
        help="Local Kraken OHLCVT .csv, .zip, or extracted directory for --source kraken-csv",
    )

    rates_rebuild = rates_sub.add_parser("rebuild")
    rates_rebuild.add_argument("--pair")
    rates_rebuild.add_argument("--days", type=int, default=30)
    rates_rebuild.add_argument(
        "--source",
        default=core_rates.RATE_SOURCE_COINBASE_EXCHANGE,
        help=f"Rate source ({', '.join(core_rates.SUPPORTED_RATE_SOURCES)})",
    )
    rates_rebuild.add_argument(
        "--path",
        help="Local Kraken OHLCVT .csv, .zip, or extracted directory for --source kraken-csv",
    )
    rates_rebuild.add_argument(
        "--reprice-transactions",
        action="store_true",
        help="Clear provider-generated transaction prices so journal processing refills them from the rebuilt cache",
    )

    rates_latest = rates_sub.add_parser("latest")
    rates_latest.add_argument("pair")

    rates_range = rates_sub.add_parser("range")
    rates_range.add_argument("pair")
    rates_range.add_argument("--start")
    rates_range.add_argument("--end")
    rates_range.add_argument("--order", choices=("asc", "desc"), default="asc")
    rates_range.add_argument("--limit", type=int)

    rates_set = rates_sub.add_parser("set")
    rates_set.add_argument("pair")
    rates_set.add_argument("timestamp")
    rates_set.add_argument("rate")
    rates_set.add_argument("--source", default="manual")
    rates_set.add_argument("--granularity")
    rates_set.add_argument("--method")

    diagnostics = sub.add_parser("diagnostics")
    diagnostics_sub = diagnostics.add_subparsers(dest="diagnostics_command", required=True)
    diagnostics_collect = diagnostics_sub.add_parser("collect")
    diagnostics_collect.add_argument(
        "--save",
        action="store_true",
        help="Also write the report under exports/diagnostics in the active Kassiber state root",
    )

    ai = sub.add_parser(
        "ai",
        description="AI provider configuration and chat over OpenAI-compatible APIs or fixed CLI adapters.",
    )
    ai_sub = ai.add_subparsers(dest="ai_command", required=True)

    ai_providers = ai_sub.add_parser("providers", description="Manage AI provider configurations.")
    ai_providers_sub = ai_providers.add_subparsers(dest="ai_providers_command", required=True)
    ai_providers_sub.add_parser("list")

    ai_providers_get = ai_providers_sub.add_parser("get")
    ai_providers_get.add_argument("name")

    ai_providers_create = ai_providers_sub.add_parser("create")
    ai_providers_create.add_argument("name")
    ai_providers_create.add_argument(
        "--base-url",
        required=True,
        help=(
            "OpenAI-compatible root, e.g. http://localhost:11434/v1; "
            "or claude-cli://default / codex-cli://default"
        ),
    )
    ai_providers_create.add_argument("--api-key", help="Deprecated argv bearer token shim; prefer --api-key-stdin or --api-key-fd")
    add_secret_stdin_options(ai_providers_create, "api-key", label="AI provider API key")
    ai_providers_create.add_argument("--default-model")
    ai_providers_create.add_argument(
        "--kind",
        choices=list(_AI_PROVIDER_KINDS_LIST),
        default="local",
        help="local = on-machine, remote = data leaves the device, tee = encrypted attestation provider",
    )
    ai_providers_create.add_argument("--notes")
    ai_providers_create.add_argument(
        "--acknowledge",
        action="store_true",
        help="Acknowledge that prompts may leave the device (auto for kind=local)",
    )

    ai_providers_update = ai_providers_sub.add_parser("update")
    ai_providers_update.add_argument("name")
    ai_providers_update.add_argument("--base-url")
    ai_providers_update.add_argument("--api-key", help="Deprecated argv bearer token shim; prefer --api-key-stdin or --api-key-fd")
    add_secret_stdin_options(ai_providers_update, "api-key", label="AI provider API key")
    ai_providers_update.add_argument("--default-model")
    ai_providers_update.add_argument("--kind", choices=list(_AI_PROVIDER_KINDS_LIST))
    ai_providers_update.add_argument("--notes")
    ai_providers_update.add_argument(
        "--clear",
        action="append",
        choices=sorted(_AI_PROVIDER_CLEARABLE_FIELDS),
        help="Null out a field (repeatable)",
    )
    ai_providers_update.add_argument("--acknowledge", action="store_true", help="Stamp acknowledged_at to now")
    ai_providers_update.add_argument("--revoke-acknowledge", action="store_true", help="Clear acknowledged_at")

    ai_providers_delete = ai_providers_sub.add_parser("delete")
    ai_providers_delete.add_argument("name")

    ai_providers_set_default = ai_providers_sub.add_parser("set-default")
    ai_providers_set_default.add_argument("name")

    ai_providers_sub.add_parser("clear-default")

    ai_models = ai_sub.add_parser("models", description="List models the configured provider exposes.")
    ai_models.add_argument("--provider", help="Provider name (defaults to the stored default)")

    return parser


def dispatch(conn: sqlite3.Connection | None, args: argparse.Namespace) -> Any:
    if args.command == "daemon":
        return daemon_runtime.run(conn, args)
    if args.command == "chat":
        result = run_chat_command(args)
        if getattr(args, "stream_json", False):
            return None
        if args.format == "json":
            return emit(args, result.to_payload(), kind="chat")
        return None
    if args.command == "init":
        return cmd_init(conn, args)
    if args.command == "status":
        return cmd_status(conn, args)
    if args.command == "secrets":
        return emit(args, dispatch_secrets(args))
    if args.command == "backup":
        return emit(args, dispatch_backup(args))
    if args.command == "backends":
        if args.backends_command == "list":
            return emit(args, core_accounts.list_backends(args.runtime_config))
        if args.backends_command == "kinds":
            return emit(args, core_accounts.list_backend_kinds())
        if args.backends_command == "get":
            return emit(
                args,
                core_accounts.get_backend_details(conn, args.runtime_config, args.name),
            )
        if args.backends_command == "create":
            return emit(
                args,
                core_accounts.create_backend(
                    conn,
                    args.name,
                    args.kind,
                    args.url,
                    chain=args.chain,
                    network=args.network,
                    auth_header=_backend_auth_header(args),
                    token=_backend_token(args),
                    batch_size=args.batch_size,
                    timeout=args.timeout,
                    tor_proxy=args.tor_proxy,
                    config=_backend_extra_config(args),
                    notes=args.notes,
                ),
            )
        if args.backends_command == "update":
            updates = {
                "kind": args.kind,
                "url": args.url,
                "chain": args.chain,
                "network": args.network,
                "auth_header": _backend_auth_header(args),
                "token": _backend_token(args),
                "batch_size": args.batch_size,
                "timeout": args.timeout,
                "tor_proxy": args.tor_proxy,
                "config": _backend_extra_config(args),
                "notes": args.notes,
                "clear": _normalized_backend_clear_fields(args.clear),
            }
            return emit(args, core_accounts.update_backend(conn, args.name, updates))
        if args.backends_command == "delete":
            return emit(args, core_accounts.delete_backend(conn, args.name))
        if args.backends_command == "set-default":
            return emit(
                args,
                core_accounts.set_default_backend(conn, args.runtime_config, args.name),
            )
        if args.backends_command == "clear-default":
            return emit(args, core_accounts.clear_default_backend(conn, args.runtime_config))
        if args.backends_command == "reveal-token":
            return emit(
                args,
                core_accounts.reveal_backend_secrets(conn, args.runtime_config, args.name),
            )
    if args.command == "context":
        if args.context_command == "show":
            return cmd_context_show(conn, args)
        if args.context_command == "current":
            return cmd_context_show(conn, args)
        if args.context_command == "set":
            return cmd_context_set(conn, args)
    if args.command == "workspaces":
        if args.workspaces_command == "list":
            return emit(args, core_accounts.list_workspaces(conn))
        if args.workspaces_command == "create":
            return emit(args, dict(core_accounts.create_workspace(conn, args.label)))
    if args.command == "profiles":
        if args.profiles_command == "list":
            return emit(args, core_accounts.list_profiles(conn, args.workspace))
        if args.profiles_command == "create":
            return emit(
                args,
                dict(
                    core_accounts.create_profile(
                        conn,
                        args.workspace,
                        args.label,
                        args.fiat_currency,
                        args.gains_algorithm,
                        args.tax_country,
                        args.tax_long_term_days,
                    )
                ),
            )
        if args.profiles_command == "get":
            return emit(
                args,
                core_accounts.get_profile_details(conn, args.workspace, args.profile),
            )
        if args.profiles_command == "set":
            updates = {
                "label": args.label,
                "fiat_currency": args.fiat_currency,
                "tax_country": args.tax_country,
                "tax_long_term_days": args.tax_long_term_days,
                "gains_algorithm": args.gains_algorithm,
            }
            if all(v is None for v in updates.values()):
                raise AppError(
                    "profiles set requires at least one field to update",
                    code="validation",
                    hint="Pass one or more of --label, --fiat-currency, --tax-country, --tax-long-term-days, --gains-algorithm",
                )
            return emit(
                args,
                core_accounts.update_profile(conn, args.workspace, args.profile, updates),
            )
    if args.command == "accounts":
        if args.accounts_command == "list":
            return emit(args, core_accounts.list_accounts(conn, args.workspace, args.profile))
        if args.accounts_command == "create":
            return emit(
                args,
                dict(
                    core_accounts.create_account(
                        conn,
                        args.workspace,
                        args.profile,
                        args.code,
                        args.label,
                        args.type,
                        args.asset,
                    )
                ),
            )
    if args.command == "wallets":
        if args.wallets_command == "list":
            return emit(args, core_wallets.list_wallets(conn, args.workspace, args.profile))
        if args.wallets_command == "create":
            return emit(
                args,
                dict(
                    core_wallets.create_wallet(
                        conn,
                        args.workspace,
                        args.profile,
                        args.label,
                        args.kind,
                        args.account,
                        core_wallets.parse_wallet_config(args),
                    )
                ),
            )
        if args.wallets_command == "kinds":
            return emit(args, core_wallets.list_wallet_kinds())
        if args.wallets_command == "get":
            return emit(
                args,
                core_wallets.get_wallet_details(
                    conn, args.workspace, args.profile, args.wallet
                ),
            )
        if args.wallets_command == "update":
            config_updates = {}
            if args.config:
                config_updates.update(json.loads(args.config))
            if args.config_file:
                with open(args.config_file, "r", encoding="utf-8") as handle:
                    config_updates.update(json.load(handle))
            if args.backend:
                config_updates["backend"] = args.backend.strip().lower()
            if args.chain:
                config_updates["chain"] = normalize_chain_value(args.chain)
            if args.network:
                chain_for_net = normalize_chain_value(config_updates.get("chain") or args.chain)
                config_updates["network"] = normalize_network_value(chain_for_net, args.network)
            if args.gap_limit is not None:
                if args.gap_limit <= 0:
                    raise AppError("Descriptor gap limit must be positive", code="validation")
                if args.gap_limit > MAX_DESCRIPTOR_GAP_LIMIT:
                    raise AppError(
                        f"Descriptor gap limit must be {MAX_DESCRIPTOR_GAP_LIMIT} or lower",
                        code="validation",
                    )
                config_updates["gap_limit"] = args.gap_limit
            if args.policy_asset:
                config_updates["policy_asset"] = normalize_asset_code(args.policy_asset)
            has_btcpay_flag = False
            if args.store_id is not None:
                config_updates["store_id"] = core_wallets.normalize_btcpay_store_id(
                    args.store_id
                )
                has_btcpay_flag = True
            if args.payment_method_id is not None:
                payment_method_id = core_wallets.normalize_btcpay_payment_method_id(
                    args.payment_method_id
                )
                config_updates["payment_method_id"] = payment_method_id
                has_btcpay_flag = True
            if has_btcpay_flag:
                config_updates["sync_source"] = core_wallets.BTCPAY_SYNC_SOURCE
            updates = {
                "label": args.label,
                "account": args.account,
                "config": config_updates,
                "clear": args.clear,
            }
            return emit(
                args,
                core_wallets.update_wallet(
                    conn, args.workspace, args.profile, args.wallet, updates
                ),
            )
        if args.wallets_command == "delete":
            return emit(
                args,
                core_wallets.delete_wallet(
                    conn,
                    args.workspace,
                    args.profile,
                    args.wallet,
                    cascade=args.cascade,
                ),
            )
        if args.wallets_command == "reveal-descriptor":
            return emit(
                args,
                core_wallets.reveal_wallet_secrets(
                    conn, args.workspace, args.profile, args.wallet
                ),
            )
        if args.wallets_command == "import-json":
            return emit(args, import_into_wallet(conn, args.workspace, args.profile, args.wallet, args.file, "json"))
        if args.wallets_command == "import-csv":
            return emit(args, import_into_wallet(conn, args.workspace, args.profile, args.wallet, args.file, "csv"))
        if args.wallets_command == "import-btcpay":
            return emit(
                args,
                import_into_wallet(
                    conn,
                    args.workspace,
                    args.profile,
                    args.wallet,
                    args.file,
                    f"btcpay_{args.input_format}",
                ),
            )
        if args.wallets_command == "import-phoenix":
            return emit(
                args,
                import_into_wallet(
                    conn,
                    args.workspace,
                    args.profile,
                    args.wallet,
                    args.file,
                    "phoenix_csv",
                ),
            )
        if args.wallets_command == "import-wasabi":
            return emit(
                args,
                import_into_wallet(
                    conn,
                    args.workspace,
                    args.profile,
                    args.wallet,
                    args.file,
                    "wasabi_bundle",
                ),
            )
        if args.wallets_command == "import-river":
            return emit(
                args,
                import_into_wallet(
                    conn,
                    args.workspace,
                    args.profile,
                    args.wallet,
                    args.file,
                    "river_csv",
                ),
            )
        if args.wallets_command in {"import-bull", "import-bullbitcoin"}:
            return emit(
                args,
                import_into_wallet(
                    conn,
                    args.workspace,
                    args.profile,
                    args.wallet,
                    args.file,
                    "bullbitcoin_csv",
                    args.mode,
                ),
            )
        if args.wallets_command == "import-coinfinity":
            return emit(
                args,
                import_into_wallet(
                    conn,
                    args.workspace,
                    args.profile,
                    args.wallet,
                    args.file,
                    "coinfinity_csv",
                    args.mode,
                ),
            )
        if args.wallets_command == "import-21bitcoin":
            return emit(
                args,
                import_into_wallet(
                    conn,
                    args.workspace,
                    args.profile,
                    args.wallet,
                    args.file,
                    "21bitcoin_csv",
                    args.mode,
                ),
            )
        if args.wallets_command in {"import-pocket", "import-pocketbitcoin"}:
            return emit(
                args,
                import_into_wallet(
                    conn,
                    args.workspace,
                    args.profile,
                    args.wallet,
                    args.file,
                    "pocketbitcoin_csv",
                    args.mode,
                ),
            )
        if args.wallets_command == "import-strike":
            return emit(
                args,
                import_into_wallet(
                    conn,
                    args.workspace,
                    args.profile,
                    args.wallet,
                    args.file,
                    "strike_csv",
                    "full",
                ),
            )
        if args.wallets_command == "import-samourai":
            return emit(
                args,
                core_samourai.import_samourai_wallet_group(
                    conn,
                    args.workspace,
                    args.profile,
                    label=args.label,
                    account_ref=args.account,
                    backend=args.backend,
                    network=args.network,
                    gap_limit=args.gap_limit,
                    source_set_file=args.source_set_file,
                ),
            )
        if args.wallets_command == "sync-btcpay":
            return emit(
                args,
                sync_btcpay_into_wallet(
                    conn,
                    args.runtime_config,
                    args.workspace,
                    args.profile,
                    args.wallet,
                    args.backend,
                    args.store_id,
                    args.payment_method_id,
                    args.page_size,
                ),
            )
        if args.wallets_command == "attach-btcpay":
            return emit(
                args,
                attach_btcpay_provenance_to_wallet(
                    conn,
                    args.runtime_config,
                    args.workspace,
                    args.profile,
                    args.wallet,
                    args.backend,
                    args.store_id,
                    args.payment_method_id,
                ),
            )
        if args.wallets_command == "sync":
            return emit(args, sync_wallet(conn, args.runtime_config, args.workspace, args.profile, args.wallet, args.all))
        if args.wallets_command == "derive":
            return emit(
                args,
                derive_wallet_targets(
                    conn,
                    args.workspace,
                    args.profile,
                    args.wallet,
                    branch=args.branch,
                    start=args.start,
                    count=args.count,
                ),
            )
    if args.command == "transactions":
        if args.transactions_command == "list":
            transactions_payload, transactions_meta = list_transactions(
                conn,
                args.workspace,
                args.profile,
                args.wallet,
                args.limit,
                direction=args.direction,
                asset=args.asset,
                start=args.start,
                end=args.end,
                cursor=args.cursor,
                sort=args.sort,
                order=args.order,
            )
            return emit(
                args,
                transactions_payload,
                envelope_meta=transactions_meta,
            )
    if args.command == "attachments":
        attachment_hooks = _attachment_hooks()
        if args.attachments_command == "add":
            return emit(
                args,
                core_attachments.add_attachment(
                    conn,
                    args.data_root,
                    args.workspace,
                    args.profile,
                    args.transaction,
                    attachment_hooks,
                    file_path=args.file,
                    url=args.url,
                    label=args.label,
                    media_type=args.media_type,
                ),
            )
        if args.attachments_command == "list":
            return emit(
                args,
                core_attachments.list_attachments(
                    conn,
                    args.data_root,
                    args.workspace,
                    args.profile,
                    attachment_hooks,
                    tx_ref=args.transaction,
                ),
            )
        if args.attachments_command == "rename":
            return emit(
                args,
                core_attachments.rename_attachment(
                    conn,
                    args.data_root,
                    args.workspace,
                    args.profile,
                    args.attachment_id,
                    args.label,
                    attachment_hooks,
                ),
            )
        if args.attachments_command == "remove":
            return emit(
                args,
                core_attachments.remove_attachment(
                    conn,
                    args.data_root,
                    args.workspace,
                    args.profile,
                    args.attachment_id,
                    attachment_hooks,
                ),
            )
        if args.attachments_command == "verify":
            return emit(
                args,
                core_attachments.verify_attachments(
                    conn,
                    args.data_root,
                    args.workspace,
                    args.profile,
                    attachment_hooks,
                    tx_ref=args.transaction,
                ),
            )
        if args.attachments_command == "gc":
            return emit(
                args,
                core_attachments.gc_attachments(
                    conn,
                    args.data_root,
                    dry_run=args.dry_run,
                ),
            )
    if args.command == "metadata":
        metadata_hooks = _metadata_hooks()
        if args.metadata_command == "notes":
            if args.notes_command == "set":
                return emit(
                    args,
                    core_metadata.set_transaction_note(
                        conn,
                        args.workspace,
                        args.profile,
                        args.transaction,
                        args.note,
                        metadata_hooks,
                        reason=args.reason,
                    ),
                )
            if args.notes_command == "clear":
                return emit(
                    args,
                    core_metadata.clear_transaction_note(
                        conn,
                        args.workspace,
                        args.profile,
                        args.transaction,
                        metadata_hooks,
                        reason=args.reason,
                    ),
                )
        if args.metadata_command == "tags":
            if args.tags_command == "list":
                return emit(args, core_metadata.list_tags(conn, args.workspace, args.profile, metadata_hooks))
            if args.tags_command == "create":
                return emit(
                    args,
                    dict(core_metadata.create_tag(conn, args.workspace, args.profile, args.code, args.label, metadata_hooks)),
                )
            if args.tags_command == "add":
                return emit(
                    args,
                    core_metadata.add_tag_to_transaction(
                        conn,
                        args.workspace,
                        args.profile,
                        args.transaction,
                        args.tag,
                        metadata_hooks,
                        reason=args.reason,
                    ),
                )
            if args.tags_command == "remove":
                return emit(
                    args,
                    core_metadata.remove_tag_from_transaction(
                        conn,
                        args.workspace,
                        args.profile,
                        args.transaction,
                        args.tag,
                        metadata_hooks,
                        reason=args.reason,
                    ),
                )
        if args.metadata_command == "bip329":
            if args.bip329_command == "import":
                return emit(
                    args,
                    core_metadata.import_bip329_labels(
                        conn, args.workspace, args.profile, args.file, metadata_hooks, wallet_ref=args.wallet
                    ),
                )
            if args.bip329_command == "list":
                bip329_payload, bip329_meta = core_metadata.list_bip329_labels(
                    conn,
                    args.workspace,
                    args.profile,
                    metadata_hooks,
                    wallet_ref=args.wallet,
                    cursor=args.cursor,
                    limit=args.limit,
                )
                return emit(
                    args,
                    bip329_payload,
                    envelope_meta=bip329_meta,
                )
            if args.bip329_command == "export":
                return emit(
                    args,
                    core_metadata.export_bip329_labels(
                        conn, args.workspace, args.profile, args.file, metadata_hooks, wallet_ref=args.wallet
                    ),
                )
        if args.metadata_command == "exclude":
            return emit(
                args,
                core_metadata.set_transaction_excluded(
                    conn,
                    args.workspace,
                    args.profile,
                    args.transaction,
                    True,
                    metadata_hooks,
                    reason=args.reason,
                ),
            )
        if args.metadata_command == "include":
            return emit(
                args,
                core_metadata.set_transaction_excluded(
                    conn,
                    args.workspace,
                    args.profile,
                    args.transaction,
                    False,
                    metadata_hooks,
                    reason=args.reason,
                ),
            )
        if args.metadata_command == "records":
            if args.records_command == "list":
                if args.has_note and args.no_note:
                    raise AppError("--has-note and --no-note are mutually exclusive", code="validation")
                if args.excluded and args.included:
                    raise AppError("--excluded and --included are mutually exclusive", code="validation")
                has_note = True if args.has_note else (False if args.no_note else None)
                excluded = True if args.excluded else (False if args.included else None)
                return emit(
                    args,
                    core_metadata.list_transaction_records(
                        conn,
                        args.workspace,
                        args.profile,
                        metadata_hooks,
                        wallet=args.wallet,
                        tag=args.tag,
                        has_note=has_note,
                        excluded=excluded,
                        start=args.start,
                        end=args.end,
                        cursor=args.cursor,
                        limit=args.limit,
                    ),
                )
            if args.records_command == "get":
                return emit(
                    args,
                    core_metadata.get_transaction_record(
                        conn, args.workspace, args.profile, args.transaction, metadata_hooks
                    ),
                )
            if args.records_command == "note":
                if args.records_note_command == "set":
                    return emit(
                        args,
                        core_metadata.set_transaction_note(
                            conn,
                            args.workspace,
                            args.profile,
                            args.transaction,
                            args.note,
                            metadata_hooks,
                            reason=args.reason,
                        ),
                    )
                if args.records_note_command == "clear":
                    return emit(
                        args,
                        core_metadata.clear_transaction_note(
                            conn,
                            args.workspace,
                            args.profile,
                            args.transaction,
                            metadata_hooks,
                            reason=args.reason,
                        ),
                    )
            if args.records_command == "tag":
                if args.records_tag_command == "add":
                    return emit(
                        args,
                        core_metadata.add_tag_to_transaction(
                            conn,
                            args.workspace,
                            args.profile,
                            args.transaction,
                            args.tag,
                            metadata_hooks,
                            reason=args.reason,
                        ),
                    )
                if args.records_tag_command == "remove":
                    return emit(
                        args,
                        core_metadata.remove_tag_from_transaction(
                            conn,
                            args.workspace,
                            args.profile,
                            args.transaction,
                            args.tag,
                            metadata_hooks,
                            reason=args.reason,
                        ),
                    )
            if args.records_command == "excluded":
                if args.records_excluded_command == "set":
                    return emit(
                        args,
                        core_metadata.set_transaction_excluded(
                            conn,
                            args.workspace,
                            args.profile,
                            args.transaction,
                            True,
                            metadata_hooks,
                            reason=args.reason,
                        ),
                    )
                if args.records_excluded_command == "clear":
                    return emit(
                        args,
                        core_metadata.set_transaction_excluded(
                            conn,
                            args.workspace,
                            args.profile,
                            args.transaction,
                            False,
                            metadata_hooks,
                            reason=args.reason,
                        ),
                    )
            if args.records_command == "history":
                if args.history_command == "list":
                    return emit(
                        args,
                        core_metadata.list_transaction_history(
                            conn,
                            args.workspace,
                            args.profile,
                            args.transaction,
                            metadata_hooks,
                            source=args.source,
                            field_family=args.field_family,
                            field=args.field,
                            pricing_only=args.pricing_only,
                            ai_only=args.ai_only,
                            stale_only=args.stale_only,
                            start=args.start,
                            end=args.end,
                            cursor=args.cursor,
                            limit=args.limit,
                        ),
                    )
                if args.history_command == "activity":
                    return emit(
                        args,
                        core_metadata.list_activity_history(
                            conn,
                            args.workspace,
                            args.profile,
                            metadata_hooks,
                            transaction_ref=args.transaction,
                            wallet_ref=args.wallet,
                            source=args.source,
                            field_family=args.field_family,
                            field=args.field,
                            pricing_only=args.pricing_only,
                            ai_only=args.ai_only,
                            stale_only=args.stale_only,
                            start=args.start,
                            end=args.end,
                            cursor=args.cursor,
                            limit=args.limit,
                        ),
                    )
                if args.history_command == "stale":
                    return emit(
                        args,
                        core_metadata.stale_transaction_edit_summary(
                            conn,
                            args.workspace,
                            args.profile,
                            metadata_hooks,
                        ),
                    )
                if args.history_command == "revert":
                    return emit(
                        args,
                        core_metadata.revert_transaction_edit(
                            conn,
                            args.workspace,
                            args.profile,
                            args.transaction,
                            metadata_hooks,
                            event_id=args.event,
                            field=args.field,
                            reason=args.reason,
                        ),
                    )
    if args.command == "journals":
        if args.journals_command == "process":
            return emit(args, process_journals(conn, args.workspace, args.profile))
        if args.journals_command == "list":
            journal_entries_payload, journal_entries_meta = list_journal_entries(
                conn,
                args.workspace,
                args.profile,
                args.limit,
                cursor=args.cursor,
                return_meta=True,
            )
            return emit(args, journal_entries_payload, envelope_meta=journal_entries_meta)
        if args.journals_command == "transfers":
            if args.journal_transfers_command == "list":
                return emit(args, inspect_transfer_audit(conn, args.workspace, args.profile))
        if args.journals_command == "events":
            if args.events_command == "list":
                return emit(
                    args,
                    list_journal_events(
                        conn,
                        args.workspace,
                        args.profile,
                        wallet=args.wallet,
                        account=args.account,
                        asset=args.asset,
                        entry_type=args.entry_type,
                        start=args.start,
                        end=args.end,
                        cursor=args.cursor,
                        limit=args.limit,
                    ),
                )
            if args.events_command == "get":
                return emit(args, get_journal_event(conn, args.workspace, args.profile, args.event_id))
        if args.journals_command == "quarantined":
            return emit(args, list_quarantines(conn, args.workspace, args.profile))
        if args.journals_command == "quarantine":
            if args.quarantine_command == "show":
                return emit(args, show_quarantine(conn, args.workspace, args.profile, args.transaction))
            if args.quarantine_command == "clear":
                return emit(args, clear_quarantine(conn, args.workspace, args.profile, args.transaction))
            if args.quarantine_command == "resolve":
                if args.quarantine_resolve_command == "price-override":
                    return emit(
                        args,
                        resolve_quarantine_price_override(
                            conn,
                            args.workspace,
                            args.profile,
                            args.transaction,
                            fiat_rate=args.fiat_rate,
                            fiat_value=args.fiat_value,
                        ),
                    )
                if args.quarantine_resolve_command == "exclude":
                    return emit(
                        args,
                        resolve_quarantine_exclude(
                            conn, args.workspace, args.profile, args.transaction
                        ),
                    )
    if args.command == "transfers":
        if args.transfers_command == "list":
            return emit(args, list_transaction_pairs(conn, args.workspace, args.profile))
        if args.transfers_command == "pair":
            return emit(
                args,
                create_transaction_pair(
                    conn,
                    args.workspace,
                    args.profile,
                    args.tx_out,
                    args.tx_in,
                    kind=args.kind,
                    policy=args.policy,
                    notes=args.note,
                ),
            )
        if args.transfers_command == "unpair":
            return emit(
                args,
                delete_transaction_pair(conn, args.workspace, args.profile, args.pair_id),
            )
        if args.transfers_command == "payouts":
            if args.payouts_command == "list":
                return emit(args, list_direct_swap_payouts(conn, args.workspace, args.profile))
            if args.payouts_command == "create":
                return emit(
                    args,
                    create_direct_swap_payout(
                        conn,
                        args.workspace,
                        args.profile,
                        args.tx_out,
                        payout_asset=args.payout_asset,
                        payout_amount=args.payout_amount,
                        payout_occurred_at=args.payout_occurred_at,
                        payout_fiat_value=args.payout_fiat_value,
                        payout_external_id=args.payout_external_id,
                        counterparty=args.counterparty,
                        policy=args.policy,
                        notes=args.note,
                    ),
                )
            if args.payouts_command == "delete":
                return emit(
                    args,
                    delete_direct_swap_payout(
                        conn, args.workspace, args.profile, args.payout_id
                    ),
                )
        if args.transfers_command == "suggest":
            return emit(
                args,
                suggest_transfer_candidates(
                    conn,
                    args.workspace,
                    args.profile,
                    time_window_seconds=args.time_window_seconds,
                    fee_pct_max=args.fee_pct_max,
                    fee_sats_min=args.fee_sats_min,
                    confidence=getattr(args, "confidence", None),
                    asset_pair=getattr(args, "asset_pair", None),
                    method=getattr(args, "method", None),
                    candidate_type=getattr(args, "candidate_type", None),
                ),
            )
        if args.transfers_command == "bulk-pair":
            return emit(
                args,
                bulk_pair_transfers(
                    conn,
                    args.workspace,
                    args.profile,
                    confidence=args.confidence,
                    time_window_seconds=args.time_window_seconds,
                    fee_pct_max=args.fee_pct_max,
                    fee_sats_min=args.fee_sats_min,
                    asset_pair=getattr(args, "asset_pair", None),
                    method=getattr(args, "method", None),
                    candidate_type=getattr(args, "candidate_type", None),
                ),
            )
        if args.transfers_command == "dismiss":
            return emit(
                args,
                dismiss_transfer_candidate(
                    conn,
                    args.workspace,
                    args.profile,
                    args.tx_out,
                    args.tx_in,
                    reason=getattr(args, "reason", None),
                    expires_in_days=args.expires_in_days,
                ),
            )
        if args.transfers_command == "rules":
            if args.transfers_rules_command == "list":
                return emit(args, list_transfer_rules(conn, args.workspace, args.profile))
            if args.transfers_rules_command == "create":
                try:
                    predicate = json.loads(args.predicate or "{}")
                except json.JSONDecodeError as exc:
                    raise AppError(
                        f"Invalid --predicate JSON: {exc}", code="validation"
                    ) from exc
                return emit(
                    args,
                    create_transfer_rule(
                        conn,
                        args.workspace,
                        args.profile,
                        name=args.name,
                        predicate=predicate,
                        kind=args.kind,
                        policy=args.policy,
                        enabled=not args.disabled,
                    ),
                )
            if args.transfers_rules_command == "delete":
                return emit(
                    args,
                    delete_transfer_rule(conn, args.workspace, args.profile, args.rule_id),
                )
            if args.transfers_rules_command == "enable":
                return emit(
                    args,
                    set_transfer_rule_enabled(
                        conn, args.workspace, args.profile, args.rule_id, True
                    ),
                )
            if args.transfers_rules_command == "disable":
                return emit(
                    args,
                    set_transfer_rule_enabled(
                        conn, args.workspace, args.profile, args.rule_id, False
                    ),
                )
            if args.transfers_rules_command == "apply":
                return emit(
                    args,
                    apply_transfer_rules(
                        conn,
                        args.workspace,
                        args.profile,
                        time_window_seconds=args.time_window_seconds,
                        fee_pct_max=args.fee_pct_max,
                        fee_sats_min=args.fee_sats_min,
                        confidence=getattr(args, "confidence", None),
                        asset_pair=getattr(args, "asset_pair", None),
                        method=getattr(args, "method", None),
                        candidate_type=getattr(args, "candidate_type", None),
                    ),
                )
    if args.command == "views":
        if args.views_command == "list":
            return emit(
                args,
                list_saved_views_cli(
                    conn, args.workspace, args.profile, surface=getattr(args, "surface", None)
                ),
            )
        if args.views_command == "create":
            try:
                filter_payload = json.loads(args.filter_json or "{}")
            except json.JSONDecodeError as exc:
                raise AppError(
                    f"Invalid --filter JSON: {exc}", code="validation"
                ) from exc
            return emit(
                args,
                create_saved_view_cli(
                    conn,
                    args.workspace,
                    args.profile,
                    surface=args.surface,
                    name=args.name,
                    filter_payload=filter_payload,
                ),
            )
        if args.views_command == "delete":
            return emit(
                args, delete_saved_view_cli(conn, args.workspace, args.profile, args.view_id)
            )
    if args.command == "chats":
        if args.chats_command == "list":
            return emit(
                args,
                list_chat_sessions_cli(
                    conn, args.workspace, args.profile, limit=args.limit
                ),
            )
        if args.chats_command == "show":
            return emit(
                args,
                show_chat_session_cli(
                    conn, args.workspace, args.profile, args.session_id
                ),
            )
        if args.chats_command == "delete":
            return emit(
                args,
                delete_chat_session_cli(
                    conn, args.workspace, args.profile, args.session_id
                ),
            )
        if args.chats_command == "clear":
            return emit(
                args, clear_chat_sessions_cli(conn, args.workspace, args.profile)
            )
        if args.chats_command == "config":
            return emit(
                args,
                chat_history_config_cli(
                    conn,
                    history=args.history,
                    database_encrypted=core_chat_history.database_file_is_encrypted(
                        args.data_root
                    ),
                ),
            )
    if args.command == "btcpay":
        commercial_hooks = _commercial_hooks()
        if args.btcpay_command == "provenance":
            if args.btcpay_provenance_command == "sync":
                return emit(
                    args,
                    sync_btcpay_commercial_provenance(
                        conn,
                        args.runtime_config,
                        args.workspace,
                        args.profile,
                        args.backend,
                        args.store_id,
                        args.page_size,
                    ),
                )
            if args.btcpay_provenance_command == "list":
                return emit(
                    args,
                    core_commercial.list_btcpay_records(
                        conn,
                        args.workspace,
                        args.profile,
                        commercial_hooks,
                        record_type=args.record_type,
                        limit=args.limit,
                    ),
                )
            if args.btcpay_provenance_command == "suggest":
                return emit(
                    args,
                    core_commercial.suggest_links(
                        conn,
                        args.workspace,
                        args.profile,
                        commercial_hooks,
                        limit=args.limit,
                    ),
                )
            if args.btcpay_provenance_command == "links":
                return emit(
                    args,
                    core_commercial.list_links(
                        conn,
                        args.workspace,
                        args.profile,
                        commercial_hooks,
                        state=args.state,
                        limit=args.limit,
                    ),
                )
            if args.btcpay_provenance_command == "review":
                return emit(
                    args,
                    core_commercial.review_link(
                        conn,
                        args.workspace,
                        args.profile,
                        args.link,
                        commercial_hooks,
                        state=args.state,
                        reconciliation_state=args.reconciliation_state,
                        commercial_kind=args.commercial_kind,
                        notes=args.notes,
                    ),
                )
    if args.command == "documents":
        commercial_hooks = _commercial_hooks()
        if args.documents_command == "list":
            return emit(
                args,
                core_commercial.list_documents(
                    conn, args.workspace, args.profile, commercial_hooks, limit=args.limit
                ),
            )
        if args.documents_command == "create":
            return emit(
                args,
                core_commercial.create_document(
                    conn,
                    args.workspace,
                    args.profile,
                    commercial_hooks,
                    document_type=args.document_type,
                    label=args.label,
                    external_ref=args.external_ref,
                    issuer=args.issuer,
                    counterparty=args.counterparty,
                    issued_at=args.issued_at,
                    due_at=args.due_at,
                    fiat_currency=args.fiat_currency,
                    fiat_value=args.fiat_value,
                    notes=args.notes,
                ),
            )
        if args.documents_command == "attach":
            return emit(
                args,
                core_commercial.attach_document_evidence(
                    conn,
                    args.data_root,
                    args.workspace,
                    args.profile,
                    args.document,
                    commercial_hooks,
                    file_path=args.file,
                    url=args.url,
                    label=args.label,
                    media_type=args.media_type,
                ),
            )
    if args.command == "source-funds":
        source_funds_hooks = _source_funds_hooks()
        if args.source_funds_command == "sources":
            if args.source_funds_sources_command == "list":
                return emit(args, core_source_funds.list_sources(conn, args.workspace, args.profile, source_funds_hooks))
            if args.source_funds_sources_command == "create":
                return emit(
                    args,
                    core_source_funds.create_source(
                        conn,
                        args.workspace,
                        args.profile,
                        source_funds_hooks,
                        source_type=args.source_type,
                        label=args.label,
                        asset=args.asset,
                        amount=args.amount,
                        fiat_value=args.fiat_value,
                        fiat_currency=args.fiat_currency,
                        acquired_at=args.acquired_at,
                        description=args.description,
                        attachment_ids=args.attachments,
                    ),
                )
            if args.source_funds_sources_command == "attach":
                return emit(
                    args,
                    core_source_funds.attach_source_evidence(
                        conn,
                        args.workspace,
                        args.profile,
                        source_funds_hooks,
                        source_ref=args.source,
                        attachment_id=args.attachment,
                    ),
                )
        if args.source_funds_command == "links":
            if args.source_funds_links_command == "list":
                return emit(
                    args,
                    core_source_funds.list_links(
                        conn,
                        args.workspace,
                        args.profile,
                        source_funds_hooks,
                        target_transaction_ref=args.target_transaction,
                        state=args.state,
                    ),
                )
            if args.source_funds_links_command == "create":
                return emit(
                    args,
                    core_source_funds.create_link(
                        conn,
                        args.workspace,
                        args.profile,
                        source_funds_hooks,
                        from_transaction_ref=args.from_transaction,
                        from_source_ref=args.from_source,
                        to_transaction_ref=args.to_transaction,
                        link_type=args.link_type,
                        state=args.state,
                        confidence=args.confidence,
                        method=args.method,
                        asset=args.asset,
                        allocation_amount=args.allocation_amount,
                        from_asset=args.from_asset,
                        from_allocation_amount=args.from_amount,
                        allocation_policy=args.allocation_policy,
                        explanation=args.explanation,
                        uses_chain_observation=args.uses_chain_observation,
                        chain_data_confirmed=args.chain_data_confirmed,
                        attachment_ids=args.attachments,
                    ),
                )
            if args.source_funds_links_command == "review":
                return emit(
                    args,
                    core_source_funds.update_link_review(
                        conn,
                        args.workspace,
                        args.profile,
                        source_funds_hooks,
                        link_ref=args.link,
                        state=args.state,
                        link_type=args.link_type,
                        confidence=args.confidence,
                        allocation_amount=args.allocation_amount,
                        from_allocation_amount=args.from_amount,
                        allocation_policy=args.allocation_policy,
                        explanation=args.explanation,
                        uses_chain_observation=args.uses_chain_observation,
                        chain_data_confirmed=args.chain_data_confirmed,
                    ),
                )
            if args.source_funds_links_command == "attach":
                return emit(
                    args,
                    core_source_funds.attach_link_evidence(
                        conn,
                        args.workspace,
                        args.profile,
                        source_funds_hooks,
                        link_ref=args.link,
                        attachment_id=args.attachment,
                    ),
                )
            if args.source_funds_links_command == "bulk-review":
                return emit(
                    args,
                    core_source_funds.bulk_review_suggestions(
                        conn,
                        args.workspace,
                        args.profile,
                        source_funds_hooks,
                        target_transaction_ref=args.target_transaction,
                    ),
                )
        if args.source_funds_command == "suggest":
            return emit(
                args,
                core_source_funds.suggest_links(
                    conn,
                    args.workspace,
                    args.profile,
                    source_funds_hooks,
                    target_transaction_ref=args.target_transaction,
                    include_broad_hints=args.include_broad_hints,
                    max_suggestions=args.max_suggestions,
                ),
            )
        if args.source_funds_command == "cases":
            if args.source_funds_cases_command == "list":
                return emit(args, core_source_funds.list_cases(conn, args.workspace, args.profile, source_funds_hooks))
        if args.source_funds_command == "coverage":
            coverage = core_source_funds_coverage.compute_coverage(
                conn,
                args.workspace,
                args.profile,
                source_funds_hooks,
                max_depth=args.max_depth,
                max_transactions=args.max_transactions,
            )
            if args.format in {"table", "plain"}:
                return emit(args, "\n".join(core_source_funds_coverage.coverage_summary_text(coverage)))
            return emit(args, coverage)
        if args.source_funds_command == "recipients":
            workspace, profile = source_funds_hooks.resolve_scope(conn, args.workspace, args.profile)
            if args.source_funds_recipients_command == "list":
                return emit(args, core_source_funds_recipients.list_recipients(conn, profile["id"]))
            if args.source_funds_recipients_command == "create":
                return emit(
                    args,
                    core_source_funds_recipients.create_recipient(
                        conn,
                        workspace["id"],
                        profile["id"],
                        label=args.label,
                        kind=args.kind,
                        default_reveal_mode=args.default_reveal_mode,
                        notes=args.notes,
                    ),
                )
            if args.source_funds_recipients_command == "update":
                recipient = core_source_funds_recipients.resolve_recipient(conn, profile["id"], args.recipient)
                return emit(
                    args,
                    core_source_funds_recipients.update_recipient(
                        conn,
                        profile["id"],
                        recipient["id"],
                        label=args.label,
                        kind=args.kind,
                        default_reveal_mode=args.default_reveal_mode,
                        notes=args.notes,
                    ),
                )
            if args.source_funds_recipients_command == "delete":
                recipient = core_source_funds_recipients.resolve_recipient(conn, profile["id"], args.recipient)
                return emit(
                    args,
                    core_source_funds_recipients.delete_recipient(conn, profile["id"], recipient["id"]),
                )
    if args.command == "reports":
        report_hooks = _report_hooks()
        if args.reports_command == "summary":
            if args.format in {"table", "plain"}:
                return emit(
                    args,
                    "\n".join(
                        core_reports.build_summary_report_lines(
                            conn,
                            args.workspace,
                            args.profile,
                            report_hooks,
                            wallet_ref=args.wallet,
                        )
                    ),
                )
            return emit(
                args,
                core_reports.report_summary(
                    conn,
                    args.workspace,
                    args.profile,
                    report_hooks,
                    wallet_ref=args.wallet,
                ),
            )
        if args.reports_command == "tax-summary":
            return emit(
                args,
                core_reports.report_tax_summary(
                    conn,
                    args.workspace,
                    args.profile,
                    report_hooks,
                ),
            )
        if args.reports_command in {"austrian-e1kv", "austrian-tax-summary"}:
            return _emit_austrian_e1kv_report(args, conn, report_hooks)
        if args.reports_command == "balance-sheet":
            return emit(
                args,
                core_reports.report_balance_sheet(
                    conn, args.workspace, args.profile, report_hooks
                ),
            )
        if args.reports_command == "portfolio-summary":
            return emit(
                args,
                core_reports.report_portfolio_summary(
                    conn, args.workspace, args.profile, report_hooks
                ),
            )
        if args.reports_command == "capital-gains":
            return emit(
                args,
                core_reports.report_capital_gains(
                    conn, args.workspace, args.profile, report_hooks
                ),
            )
        if args.reports_command == "journal-entries":
            return emit(
                args,
                core_reports.report_journal_entries(
                    conn, args.workspace, args.profile, report_hooks
                ),
            )
        if args.reports_command == "balance-history":
            return emit(
                args,
                core_reports.report_balance_history(
                    conn,
                    args.workspace,
                    args.profile,
                    report_hooks,
                    interval=args.interval,
                    start=args.start,
                    end=args.end,
                    wallet_ref=args.wallet,
                    account_ref=args.account,
                    asset=args.asset,
                ),
            )
        if args.reports_command == "export-pdf":
            return emit(
                args,
                core_reports.export_pdf_report(
                    conn,
                    args.workspace,
                    args.profile,
                    args.file,
                    report_hooks,
                    wallet_ref=args.wallet,
                    history_limit=args.history_limit,
                ),
            )
        if args.reports_command == "export-summary-pdf":
            return emit(
                args,
                core_reports.export_summary_pdf_report(
                    conn,
                    args.workspace,
                    args.profile,
                    args.file,
                    report_hooks,
                    start=args.start,
                    end=args.end,
                    wallet_refs=args.wallets,
                    include_snapshot=args.include_snapshot,
                ),
            )
        if args.reports_command == "export-csv":
            return emit(
                args,
                core_reports.export_csv_report(
                    conn,
                    args.workspace,
                    args.profile,
                    args.file,
                    report_hooks,
                    wallet_ref=args.wallet,
                    history_limit=args.history_limit,
                ),
            )
        if args.reports_command == "export-xlsx":
            return emit(
                args,
                core_reports.export_xlsx_report(
                    conn,
                    args.workspace,
                    args.profile,
                    args.file,
                    report_hooks,
                    wallet_ref=args.wallet,
                    history_limit=args.history_limit,
                ),
            )
        if args.reports_command == "lightning-profitability":
            return emit(
                args,
                _cli_lightning_profitability_payload(
                    conn,
                    args.connection,
                    window_days=args.window_days,
                    workspace_ref=args.workspace,
                    profile_ref=args.profile,
                ),
            )
        if args.reports_command == "export-lightning-profitability-csv":
            return emit(
                args,
                _cli_export_lightning_profitability_csv(
                    conn,
                    args.connection,
                    args.file,
                    window_days=args.window_days,
                    workspace_ref=args.workspace,
                    profile_ref=args.profile,
                ),
            )
        if args.reports_command == "commercial-subledger":
            return emit(
                args,
                core_commercial.build_reviewed_subledger_rows(
                    conn,
                    args.workspace,
                    args.profile,
                    _commercial_hooks(),
                ),
            )
        if args.reports_command == "export-commercial-subledger-csv":
            return emit(
                args,
                core_commercial.export_reviewed_subledger_csv(
                    conn,
                    args.workspace,
                    args.profile,
                    args.file,
                    _commercial_hooks(),
                ),
            )
        if args.reports_command == "source-funds":
            source_funds_hooks = _source_funds_hooks()
            report = core_source_funds.build_report(
                conn,
                args.workspace,
                args.profile,
                source_funds_hooks,
                target_transaction_ref=args.target_transaction,
                target_amount=args.target_amount,
                report_purpose=args.purpose,
                planned_destination=args.planned_destination,
                planned_note=args.planned_note,
                reveal_mode=args.reveal_mode,
                max_depth=args.max_depth,
                save_case=args.save_case,
                case_label=args.case_label,
                recipient_ref=args.recipient,
                include_diagrams=True,
                report_options={
                    "diagram_detail": args.diagram_detail,
                    "amount_precision": args.amount_precision,
                    "mask_recipient": args.mask_recipient,
                    "omit_sections": args.omit_section,
                    "reveal_overrides": dict(
                        item.split("=", 1)
                        for item in (args.reveal_override or [])
                        if "=" in item
                    ),
                },
            )
            if args.format in {"table", "plain"}:
                return emit(args, "\n".join(core_source_funds.build_report_lines(report, source_funds_hooks)))
            return emit(args, report)
        if args.reports_command == "export-source-funds-pdf":
            return emit(
                args,
                core_source_funds.export_pdf(
                    conn,
                    args.workspace,
                    args.profile,
                    args.file,
                    _source_funds_hooks(),
                    case_ref=args.case,
                ),
            )
        if args.reports_command == "export-source-funds-bundle":
            return emit(
                args,
                core_source_funds.export_bundle(
                    conn,
                    args.workspace,
                    args.profile,
                    args.file,
                    _source_funds_hooks(),
                    data_root=args.data_root,
                    case_ref=args.case,
                ),
            )
        if args.reports_command in {"export-austrian-e1kv-pdf", "export-austrian"}:
            return _emit_austrian_e1kv_pdf(args, conn, report_hooks)
        if args.reports_command == "export-austrian-e1kv-xlsx":
            return emit(
                args,
                core_reports.export_austrian_e1kv_xlsx_report(
                    conn,
                    args.workspace,
                    args.profile,
                    args.file,
                    report_hooks,
                    tax_year=args.year,
                ),
            )
        if args.reports_command == "export-austrian-e1kv-csv":
            return emit(
                args,
                core_reports.export_austrian_e1kv_csv_bundle(
                    conn,
                    args.workspace,
                    args.profile,
                    args.dir,
                    report_hooks,
                    tax_year=args.year,
                ),
            )
    if args.command == "rates":
        if args.rates_command == "pairs":
            return emit(args, core_rates.list_cached_pairs(conn))
        if args.rates_command == "sync":
            return emit(
                args,
                core_rates.sync_rates(
                    conn,
                    pair=args.pair,
                    days=args.days,
                    source=args.source,
                    path=args.path,
                ),
            )
        if args.rates_command == "rebuild":
            return emit(
                args,
                core_rates.rebuild_rates_cache(
                    conn,
                    pair=args.pair,
                    days=args.days,
                    source=args.source,
                    path=args.path,
                    reprice_transactions=args.reprice_transactions,
                ),
            )
        if args.rates_command == "latest":
            return emit(args, core_rates.get_latest_rate(conn, args.pair))
        if args.rates_command == "range":
            return emit(
                args,
                core_rates.get_rate_range(
                    conn,
                    args.pair,
                    start=args.start,
                    end=args.end,
                    order=args.order,
                    limit=args.limit,
                ),
            )
        if args.rates_command == "set":
            return emit(
                args,
                core_rates.set_manual_rate(
                    conn,
                    args.pair,
                    args.timestamp,
                    args.rate,
                    source=args.source,
                    granularity=args.granularity,
                    method=args.method,
                ),
            )
    if args.command == "diagnostics":
        if args.diagnostics_command == "collect":
            report = collect_public_diagnostics(
                conn,
                args,
                runtime_config=getattr(args, "runtime_config", None),
            )
            saved_payload = None
            if args.save:
                saved = save_public_diagnostics_report(
                    report,
                    target="auto",
                    data_root=args.data_root,
                )
                if saved:
                    saved_payload = {
                        "target": saved.target,
                        "relative_path": saved.relative_path,
                        "filename": saved.path.name,
                    }
            return emit(args, {"report": report, "saved": saved_payload})
    if args.command == "ai":
        if args.ai_command == "providers":
            if args.ai_providers_command == "list":
                return emit(args, list_ai_providers_with_default(conn))
            if args.ai_providers_command == "get":
                return emit(args, _ai_provider_redacted(conn, get_db_ai_provider(conn, args.name)))
            if args.ai_providers_command == "create":
                enforce_single_stdin_consumer(args, ("api_key",))
                api_key = read_secret_from_args(args, "api-key", legacy_attr="api_key")
                created = create_db_ai_provider(
                    conn,
                    args.name,
                    args.base_url,
                    api_key=api_key,
                    default_model=args.default_model,
                    kind=args.kind,
                    notes=args.notes,
                    acknowledged=args.acknowledge,
                )
                return emit(args, _ai_provider_redacted(conn, created))
            if args.ai_providers_command == "update":
                enforce_single_stdin_consumer(args, ("api_key",))
                api_key = read_secret_from_args(args, "api-key", legacy_attr="api_key")
                updates = {
                    "base_url": args.base_url,
                    "api_key": api_key,
                    "default_model": args.default_model,
                    "kind": args.kind,
                    "notes": args.notes,
                    "clear": list(args.clear or ()),
                    "acknowledged": args.acknowledge,
                    "acknowledge_clear": args.revoke_acknowledge,
                }
                updated = update_db_ai_provider(conn, args.name, updates)
                return emit(args, _ai_provider_redacted(conn, updated))
            if args.ai_providers_command == "delete":
                return emit(args, delete_db_ai_provider(conn, args.name))
            if args.ai_providers_command == "set-default":
                return emit(args, set_default_ai_provider(conn, args.name))
            if args.ai_providers_command == "clear-default":
                return emit(args, clear_default_ai_provider(conn))
        if args.ai_command == "models":
            provider = resolve_ai_provider(conn, args.provider)
            client = _ai_client_for(provider)
            return emit(args, {"provider": provider["name"], "models": client.list_models()})
    raise AppError("Unknown command")


def command_needs_db(args: argparse.Namespace) -> bool:
    if args.command == "daemon":
        return False
    if args.command == "chat":
        return False
    if args.command == "backends" and getattr(args, "backends_command", None) == "kinds":
        return False
    if args.command == "wallets" and getattr(args, "wallets_command", None) == "kinds":
        return False
    if args.command == "secrets":
        return False
    if args.command == "backup":
        return False
    return True


def command_persists_bootstrap(args: argparse.Namespace) -> bool:
    if args.command == "init":
        return True
    if args.command == "backends" and getattr(args, "backends_command", None) in {
        "update",
        "delete",
        "set-default",
        "clear-default",
    }:
        return True
    return False


def _configure_cli_logging(args: argparse.Namespace) -> None:
    """Send library log records to stderr for non-daemon CLI runs.

    The daemon installs its own RAM-only ring handler and must keep stderr
    clean, so it is skipped here. Idempotent: repeated `main()` calls in tests
    do not stack handlers.
    """
    if args.command == "daemon":
        return
    root = logging.getLogger()
    level = logging.DEBUG if getattr(args, "debug", False) else logging.WARNING
    for handler in root.handlers:
        if getattr(handler, "_kassiber_cli_handler", False):
            handler.setLevel(level)
            break
    else:
        handler = logging.StreamHandler(sys.stderr)
        handler.setLevel(level)
        handler.setFormatter(logging.Formatter("%(levelname)s %(name)s: %(message)s"))
        handler._kassiber_cli_handler = True  # type: ignore[attr-defined]
        root.addHandler(handler)
    if root.level == logging.NOTSET or root.level > level:
        root.setLevel(level)


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    _configure_cli_logging(args)

    runtime = None
    try:
        args.format = resolve_output_format(args)
    except AppError as exc:
        args.format = "table"
        emit_error(args, exc)
        return 1

    try:
        runtime = bootstrap_runtime(
            args,
            needs_db=command_needs_db(args),
            persist_bootstrap=command_persists_bootstrap(args),
        )
        dispatch(runtime.conn, args)
        return 0
    except AppError as exc:
        debug_text = None
        if args.debug:
            raw_traceback = traceback.format_exc()
            sys.stderr.write(raw_traceback)
            debug_text = sanitize_traceback_text(raw_traceback)
        write_error_diagnostics(
            args,
            runtime,
            exc,
            stack=traceback.extract_tb(exc.__traceback__),
            unhandled=False,
        )
        emit_error(args, exc, debug_text=debug_text)
        return 1
    except Exception as exc:
        raw_traceback = traceback.format_exc()
        if args.debug:
            sys.stderr.write(raw_traceback)
        wrapped = AppError(str(exc) or exc.__class__.__name__, code="internal_error")
        write_error_diagnostics(
            args,
            runtime,
            exc,
            stack=traceback.extract_tb(exc.__traceback__),
            unhandled=True,
        )
        emit_error(
            args,
            wrapped,
            debug_text=sanitize_traceback_text(raw_traceback) if args.debug else None,
        )
        return 1
    finally:
        if runtime is not None:
            close_runtime(runtime)
