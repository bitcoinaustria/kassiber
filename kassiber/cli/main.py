from __future__ import annotations

import argparse
import json
import sqlite3
import sys
import traceback
from typing import Any, Sequence

from .handlers import (
    APP_NAME,
    BACKEND_KINDS,
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
    _metadata_hooks,
    _report_hooks,
    clear_quarantine,
    cmd_context_set,
    cmd_context_show,
    cmd_init,
    cmd_status,
    create_transaction_pair,
    delete_transaction_pair,
    derive_wallet_targets,
    emit,
    get_journal_event,
    import_into_wallet,
    inspect_transfer_audit,
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
    resolve_quarantine_exclude,
    resolve_quarantine_price_override,
    show_quarantine,
    sync_wallet,
)
from ..core import accounts as core_accounts
from ..core import attachments as core_attachments
from ..core import metadata as core_metadata
from ..core import rates as core_rates
from ..core import reports as core_reports
from ..core import wallets as core_wallets
from ..core.runtime import bootstrap_runtime, close_runtime, emit_error, resolve_output_format
from ..errors import AppError
from ..tax_policy import supported_tax_countries
from ..ui.dashboard import collect_ui_snapshot


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog=APP_NAME,
        description="Open-source, local-first Bitcoin accounting CLI with multi-account and multi-wallet support.",
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
    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("init")
    sub.add_parser("status")
    ui = sub.add_parser("ui")
    ui.add_argument("--workspace")
    ui.add_argument("--profile")

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
    backends_create.add_argument("--auth-header")
    backends_create.add_argument("--token")
    backends_create.add_argument("--batch-size", type=int)
    backends_create.add_argument("--timeout", type=int)
    backends_create.add_argument("--tor-proxy")
    backends_create.add_argument("--notes")

    backends_update = backends_sub.add_parser("update")
    backends_update.add_argument("name")
    backends_update.add_argument("--kind", choices=sorted(BACKEND_KINDS))
    backends_update.add_argument("--url")
    backends_update.add_argument("--chain", choices=["bitcoin", "liquid"])
    backends_update.add_argument("--network")
    backends_update.add_argument("--auth-header")
    backends_update.add_argument("--token")
    backends_update.add_argument("--batch-size", type=int)
    backends_update.add_argument("--timeout", type=int)
    backends_update.add_argument("--tor-proxy")
    backends_update.add_argument("--notes")

    backends_delete = backends_sub.add_parser("delete")
    backends_delete.add_argument("name")

    backends_set_default = backends_sub.add_parser("set-default")
    backends_set_default.add_argument("name")

    backends_sub.add_parser("clear-default")

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
        help=f"Tax country for the profile (currently supported: {', '.join(supported_tax_countries())})",
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
        help=f"Tax country for the profile (currently supported: {', '.join(supported_tax_countries())})",
    )
    profiles_set.add_argument("--tax-long-term-days", type=int)
    profiles_set.add_argument("--gains-algorithm", choices=list(RP2_ACCOUNTING_METHODS))

    accounts = sub.add_parser("accounts")
    accounts_sub = accounts.add_subparsers(dest="accounts_command", required=True)
    accounts_list = accounts_sub.add_parser("list")
    accounts_list.add_argument("--workspace")
    accounts_list.add_argument("--profile")
    accounts_create = accounts_sub.add_parser("create")
    accounts_create.add_argument("--workspace")
    accounts_create.add_argument("--profile")
    accounts_create.add_argument("--code", required=True)
    accounts_create.add_argument("--label", required=True)
    accounts_create.add_argument("--type", required=True)
    accounts_create.add_argument("--asset")

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
    wallets_create.add_argument("--account")
    wallets_create.add_argument("--backend")
    wallets_create.add_argument("--chain", choices=["bitcoin", "liquid"])
    wallets_create.add_argument("--network")
    wallets_create.add_argument("--address", action="append")
    wallets_create.add_argument("--descriptor")
    wallets_create.add_argument("--descriptor-file")
    wallets_create.add_argument("--change-descriptor")
    wallets_create.add_argument("--change-descriptor-file")
    wallets_create.add_argument("--gap-limit", type=int)
    wallets_create.add_argument("--policy-asset")
    wallets_create.add_argument("--config")
    wallets_create.add_argument("--config-file")
    wallets_create.add_argument("--source-file")
    wallets_create.add_argument("--source-format", choices=["json", "csv", "btcpay_json", "btcpay_csv", "phoenix_csv"])

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
    wallets_update.add_argument("--account")
    wallets_update.add_argument("--backend")
    wallets_update.add_argument("--chain", choices=["bitcoin", "liquid"])
    wallets_update.add_argument("--network")
    wallets_update.add_argument("--gap-limit", type=int)
    wallets_update.add_argument("--policy-asset")
    wallets_update.add_argument("--config")
    wallets_update.add_argument("--config-file")
    wallets_update.add_argument("--clear", action="append", default=[], metavar="FIELD", help="Clear a config field (repeatable)")

    wallets_delete = wallets_sub.add_parser("delete")
    wallets_delete.add_argument("--workspace")
    wallets_delete.add_argument("--profile")
    wallets_delete.add_argument("--wallet", required=True)
    wallets_delete.add_argument("--cascade", action="store_true", help="Also delete transactions and journal entries belonging to this wallet")
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
    notes_clear = notes_sub.add_parser("clear")
    notes_clear.add_argument("--workspace")
    notes_clear.add_argument("--profile")
    notes_clear.add_argument("--transaction", required=True)
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
    tags_remove = tags_sub.add_parser("remove")
    tags_remove.add_argument("--workspace")
    tags_remove.add_argument("--profile")
    tags_remove.add_argument("--transaction", required=True)
    tags_remove.add_argument("--tag", required=True)
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
    include = meta_sub.add_parser("include")
    include.add_argument("--workspace")
    include.add_argument("--profile")
    include.add_argument("--transaction", required=True)

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
    rn_clear = records_note_sub.add_parser("clear")
    rn_clear.add_argument("--workspace")
    rn_clear.add_argument("--profile")
    rn_clear.add_argument("--transaction", required=True)

    records_tag = records_sub.add_parser("tag")
    records_tag_sub = records_tag.add_subparsers(dest="records_tag_command", required=True)
    rt_add = records_tag_sub.add_parser("add")
    rt_add.add_argument("--workspace")
    rt_add.add_argument("--profile")
    rt_add.add_argument("--transaction", required=True)
    rt_add.add_argument("--tag", required=True)
    rt_remove = records_tag_sub.add_parser("remove")
    rt_remove.add_argument("--workspace")
    rt_remove.add_argument("--profile")
    rt_remove.add_argument("--transaction", required=True)
    rt_remove.add_argument("--tag", required=True)

    records_excluded = records_sub.add_parser("excluded")
    records_excluded_sub = records_excluded.add_subparsers(dest="records_excluded_command", required=True)
    re_set = records_excluded_sub.add_parser("set")
    re_set.add_argument("--workspace")
    re_set.add_argument("--profile")
    re_set.add_argument("--transaction", required=True)
    re_clear = records_excluded_sub.add_parser("clear")
    re_clear.add_argument("--workspace")
    re_clear.add_argument("--profile")
    re_clear.add_argument("--transaction", required=True)

    journals = sub.add_parser("journals")
    journals_sub = journals.add_subparsers(dest="journals_command", required=True)
    journals_process = journals_sub.add_parser("process")
    journals_process.add_argument("--workspace")
    journals_process.add_argument("--profile")
    journals_list = journals_sub.add_parser("list")
    journals_list.add_argument("--workspace")
    journals_list.add_argument("--profile")
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

    reports = sub.add_parser("reports")
    reports_sub = reports.add_subparsers(dest="reports_command", required=True)
    for report_name in ["summary", "tax-summary", "balance-sheet", "portfolio-summary", "capital-gains", "journal-entries"]:
        report = reports_sub.add_parser(report_name)
        report.add_argument("--workspace")
        report.add_argument("--profile")
        if report_name == "summary":
            report.add_argument("--wallet")

    balance_history = reports_sub.add_parser("balance-history")
    balance_history.add_argument("--workspace")
    balance_history.add_argument("--profile")
    balance_history.add_argument("--interval", choices=list(core_reports.INTERVAL_CHOICES), default="day")
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

    rates = sub.add_parser("rates")
    rates_sub = rates.add_subparsers(dest="rates_command", required=True)

    rates_pairs = rates_sub.add_parser("pairs")
    rates_pairs.set_defaults(rates_command="pairs")
    _ = rates_pairs

    rates_sync = rates_sub.add_parser("sync")
    rates_sync.add_argument("--pair")
    rates_sync.add_argument("--days", type=int, default=30)
    rates_sync.add_argument("--source", default="coingecko")

    rates_latest = rates_sub.add_parser("latest")
    rates_latest.add_argument("pair")

    rates_range = rates_sub.add_parser("range")
    rates_range.add_argument("pair")
    rates_range.add_argument("--start")
    rates_range.add_argument("--end")
    rates_range.add_argument("--limit", type=int)

    rates_set = rates_sub.add_parser("set")
    rates_set.add_argument("pair")
    rates_set.add_argument("timestamp")
    rates_set.add_argument("rate")
    rates_set.add_argument("--source", default="manual")

    return parser


def dispatch(conn: sqlite3.Connection | None, args: argparse.Namespace) -> Any:
    if args.command == "ui":
        if args.format in {"plain", "csv"}:
            raise AppError(
                "kassiber ui supports the interactive window or json output (`--machine`) only",
                code="format_unsupported",
            )
        if args.format == "json":
            return emit(
                args,
                collect_ui_snapshot(
                    conn,
                    args.data_root,
                    args.runtime_config,
                    workspace_ref=args.workspace,
                    profile_ref=args.profile,
                ),
                kind="ui.snapshot",
            )
        if args.output:
            raise AppError(
                "`kassiber ui` does not write the interactive window to --output; use `--machine` for a file snapshot",
                code="format_unsupported",
            )
        from ..ui.app import run

        run(
            conn,
            data_root=args.data_root,
            runtime_config=args.runtime_config,
            workspace_ref=args.workspace,
            profile_ref=args.profile,
        )
        return None
    if args.command == "init":
        return cmd_init(conn, args)
    if args.command == "status":
        return cmd_status(conn, args)
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
                    auth_header=args.auth_header,
                    token=args.token,
                    batch_size=args.batch_size,
                    timeout=args.timeout,
                    tor_proxy=args.tor_proxy,
                    notes=args.notes,
                ),
            )
        if args.backends_command == "update":
            updates = {
                "kind": args.kind,
                "url": args.url,
                "chain": args.chain,
                "network": args.network,
                "auth_header": args.auth_header,
                "token": args.token,
                "batch_size": args.batch_size,
                "timeout": args.timeout,
                "tor_proxy": args.tor_proxy,
                "notes": args.notes,
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
                config_updates["gap_limit"] = args.gap_limit
            if args.policy_asset:
                config_updates["policy_asset"] = normalize_asset_code(args.policy_asset)
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
            return emit(args, list_transactions(conn, args.workspace, args.profile, args.wallet, args.limit))
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
                        conn, args.workspace, args.profile, args.transaction, args.note, metadata_hooks
                    ),
                )
            if args.notes_command == "clear":
                return emit(
                    args,
                    core_metadata.clear_transaction_note(
                        conn, args.workspace, args.profile, args.transaction, metadata_hooks
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
                        conn, args.workspace, args.profile, args.transaction, args.tag, metadata_hooks
                    ),
                )
            if args.tags_command == "remove":
                return emit(
                    args,
                    core_metadata.remove_tag_from_transaction(
                        conn, args.workspace, args.profile, args.transaction, args.tag, metadata_hooks
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
                return emit(
                    args,
                    core_metadata.list_bip329_labels(
                        conn, args.workspace, args.profile, metadata_hooks, wallet_ref=args.wallet, limit=args.limit
                    ),
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
                    conn, args.workspace, args.profile, args.transaction, True, metadata_hooks
                ),
            )
        if args.metadata_command == "include":
            return emit(
                args,
                core_metadata.set_transaction_excluded(
                    conn, args.workspace, args.profile, args.transaction, False, metadata_hooks
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
                            conn, args.workspace, args.profile, args.transaction, args.note, metadata_hooks
                        ),
                    )
                if args.records_note_command == "clear":
                    return emit(
                        args,
                        core_metadata.clear_transaction_note(
                            conn, args.workspace, args.profile, args.transaction, metadata_hooks
                        ),
                    )
            if args.records_command == "tag":
                if args.records_tag_command == "add":
                    return emit(
                        args,
                        core_metadata.add_tag_to_transaction(
                            conn, args.workspace, args.profile, args.transaction, args.tag, metadata_hooks
                        ),
                    )
                if args.records_tag_command == "remove":
                    return emit(
                        args,
                        core_metadata.remove_tag_from_transaction(
                            conn, args.workspace, args.profile, args.transaction, args.tag, metadata_hooks
                        ),
                    )
            if args.records_command == "excluded":
                if args.records_excluded_command == "set":
                    return emit(
                        args,
                        core_metadata.set_transaction_excluded(
                            conn, args.workspace, args.profile, args.transaction, True, metadata_hooks
                        ),
                    )
                if args.records_excluded_command == "clear":
                    return emit(
                        args,
                        core_metadata.set_transaction_excluded(
                            conn, args.workspace, args.profile, args.transaction, False, metadata_hooks
                        ),
                    )
    if args.command == "journals":
        if args.journals_command == "process":
            return emit(args, process_journals(conn, args.workspace, args.profile))
        if args.journals_command == "list":
            return emit(args, list_journal_entries(conn, args.workspace, args.profile, args.limit))
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
    if args.command == "rates":
        if args.rates_command == "pairs":
            return emit(args, core_rates.list_cached_pairs(conn))
        if args.rates_command == "sync":
            return emit(
                args,
                core_rates.sync_rates(conn, pair=args.pair, days=args.days, source=args.source),
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
                    limit=args.limit,
                ),
            )
        if args.rates_command == "set":
            return emit(
                args,
                core_rates.set_manual_rate(
                    conn, args.pair, args.timestamp, args.rate, source=args.source
                ),
            )
    raise AppError("Unknown command")


def command_needs_db(args: argparse.Namespace) -> bool:
    if args.command == "backends" and getattr(args, "backends_command", None) == "kinds":
        return False
    if args.command == "wallets" and getattr(args, "wallets_command", None) == "kinds":
        return False
    return True


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    runtime = None
    try:
        args.format = resolve_output_format(args)
    except AppError as exc:
        args.format = "table"
        emit_error(args, exc)
        return 1

    try:
        runtime = bootstrap_runtime(args, needs_db=command_needs_db(args))
        dispatch(runtime.conn, args)
        return 0
    except AppError as exc:
        debug_text = None
        if args.debug:
            debug_text = traceback.format_exc()
            sys.stderr.write(debug_text)
        emit_error(args, exc, debug_text=debug_text)
        return 1
    except Exception as exc:
        debug_text = traceback.format_exc()
        if args.debug:
            sys.stderr.write(debug_text)
        wrapped = AppError(str(exc) or exc.__class__.__name__, code="internal_error")
        emit_error(args, wrapped, debug_text=debug_text if args.debug else None)
        return 1
    finally:
        if runtime is not None:
            close_runtime(runtime)
