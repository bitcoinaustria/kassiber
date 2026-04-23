from __future__ import annotations

import sqlite3
from datetime import datetime, timezone
from decimal import Decimal
from typing import Any

from ..backends import load_runtime_config, merge_db_backends
from ..core import accounts as core_accounts
from ..core import wallets as core_wallets
from ..core.repo import current_context_snapshot, resolve_scope, wallet_transaction_count
from ..core.runtime import build_status_payload, ensure_runtime_layout, resolve_runtime_paths
from ..db import open_db
from ..errors import AppError
from ..msat import msat_to_btc

_REPORT_SPECS = (
    ("balance-sheet", "Balance Sheet", "Current holdings, cost basis, and unrealized PnL."),
    ("portfolio-summary", "Portfolio Summary", "Per-wallet allocation and average cost."),
    ("capital-gains", "Capital Gains", "Realized disposals and tax-facing gains or losses."),
    ("journal-entries", "Journal Entries", "Bookkeeping-grade journal output."),
    ("balance-history", "Balance History", "Balance-over-time curve for the active profile."),
)

_TRANSACTION_TYPE_LABELS = {
    "consolidation": "Consolidation",
    "deposit": "Income",
    "expense": "Expense",
    "fee": "Fee",
    "income": "Income",
    "lightning_received": "Income",
    "lightning_sent": "Expense",
    "melt": "Melt",
    "mint": "Mint",
    "move": "Transfer",
    "payment": "Expense",
    "rebalance": "Rebalance",
    "receive": "Income",
    "received": "Income",
    "send": "Expense",
    "sent": "Expense",
    "swap": "Swap",
    "swap_in": "Swap",
    "swap_out": "Swap",
    "transfer": "Transfer",
    "transfer_fee": "Fee",
    "transfer_in": "Transfer",
    "transfer_out": "Transfer",
    "withdrawal": "Expense",
}


def _format_count(value: int) -> str:
    return f"{int(value):,}"


def _format_timestamp(value: str | None, empty: str = "Not yet") -> str:
    if not value:
        return empty
    try:
        dt = datetime.fromisoformat(str(value).replace("Z", "+00:00"))
    except ValueError:
        return str(value)
    return dt.astimezone(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")


def _format_day(value: str | None, empty: str = "--") -> str:
    if not value:
        return empty
    try:
        dt = datetime.fromisoformat(str(value).replace("Z", "+00:00"))
    except ValueError:
        return str(value)
    return dt.astimezone(timezone.utc).strftime("%Y-%m-%d")


def _format_btc_msat(value: int | float | Decimal | None) -> str:
    if value in (None, ""):
        return "0.00000000 BTC"
    btc_value = msat_to_btc(int(value))
    return f"{btc_value:,.8f} BTC"


def _format_fiat(value: int | float | Decimal | None, fiat_currency: str) -> str:
    if value in (None, ""):
        return f"0.00 {fiat_currency}"
    return f"{Decimal(str(value)):,.2f} {fiat_currency}"


def _msat_to_sats(value: int | float | Decimal | None) -> int:
    if value in (None, ""):
        return 0
    return int(Decimal(str(value)) / Decimal("1000"))


def _format_signed_sats(value: int | float | Decimal | None) -> str:
    sats = _msat_to_sats(value)
    if sats > 0:
        return f"+ {sats:,}"
    if sats < 0:
        return f"- {abs(sats):,}"
    return "0"


def _titleize_token(value: str | None, empty: str = "Activity") -> str:
    token = str(value or "").strip().replace("_", " ").replace("-", " ")
    return token.title() if token else empty


def _transaction_type_label(direction: str, kind: str) -> str:
    normalized_kind = str(kind or "").strip().lower()
    if normalized_kind in _TRANSACTION_TYPE_LABELS:
        return _TRANSACTION_TYPE_LABELS[normalized_kind]
    if "fee" in normalized_kind:
        return "Fee"
    if normalized_kind.startswith("swap"):
        return "Swap"
    if normalized_kind.startswith("transfer"):
        return "Transfer"
    normalized_direction = str(direction or "").strip().lower()
    if normalized_direction in {"in", "inbound", "receive", "received"}:
        return "Income"
    if normalized_direction in {"out", "outbound", "send", "sent"}:
        return "Expense"
    return _titleize_token(kind or direction)


def _amount_tone(amount_msat: int) -> str:
    if amount_msat > 0:
        return "positive"
    if amount_msat < 0:
        return "negative"
    return "neutral"


def _signed_amount_for_display(amount_msat: int, direction: str | None) -> int:
    normalized_direction = str(direction or "").strip().lower()
    if normalized_direction in {"out", "outbound", "send", "sent"}:
        return -abs(amount_msat)
    if normalized_direction in {"in", "inbound", "receive", "received"}:
        return abs(amount_msat)
    return amount_msat


def _transaction_badge_tone(type_label: str) -> str:
    normalized = str(type_label or "").strip().lower()
    if normalized in {"income", "expense", "transfer", "swap", "consolidation", "rebalance", "mint", "melt", "fee"}:
        return type_label
    if normalized == "positive":
        return "Income"
    if normalized == "negative":
        return "Expense"
    if normalized == "neutral":
        return "Transfer"
    return "muted"


def _report_header_eyebrow(tax_country: str, fiat_currency: str) -> str:
    country_label = "GENERIC POLICY" if tax_country == "generic" else f"{tax_country.upper()} POLICY"
    return f"REPORT  |  {country_label}  |  {fiat_currency}"


def _report_preview_subtitle(
    supports_runtime_reports: bool,
    processed_fresh: bool,
    quarantine_count: int,
    status_body: str,
) -> str:
    if not supports_runtime_reports:
        return "Computed capital gains preview is unavailable for this tax policy. Recent local inputs are shown instead."
    if quarantine_count:
        return "Recent local inputs are visible below, but quarantines still block a trustworthy computed preview."
    if processed_fresh:
        return "Recent local inputs feeding the current read-only report surface."
    return status_body


def _report_preview_title(preview_rows: list[dict[str, Any]]) -> str:
    return "Recent transaction inputs" if preview_rows else "Preview unavailable"


def _report_empty_hint(preview_rows: list[dict[str, Any]], status_body: str) -> str:
    if preview_rows:
        return ""
    return status_body


def _transaction_limit() -> int:
    return 200


def _transaction_history_label(total_count: int, visible_count: int) -> str:
    if total_count <= 0:
        return "LOCAL SNAPSHOT"
    if total_count > visible_count:
        return f"LATEST {visible_count} OF {total_count}"
    return "LOCAL SNAPSHOT"


def _transaction_filter_options(items: list[dict[str, Any]]) -> list[str]:
    seen = set()
    output = []
    for item in items:
        label = str(item.get("type_label") or "").strip()
        if not label or label in seen:
            continue
        seen.add(label)
        output.append(label)
    return output


def _connection_status(wallet: dict[str, Any], transaction_count: int) -> tuple[str, str]:
    if transaction_count > 0:
        return "Ready", "ok"
    if wallet.get("source_file"):
        return "Imported source", "warn"
    if wallet.get("backend") or wallet.get("descriptor") or wallet.get("addresses"):
        return "Configured", "warn"
    return "New", "warn"


def _build_welcome_section(status: dict[str, Any], has_profile: bool) -> dict[str, Any]:
    title = "Welcome."
    body = "Kassiber keeps your books in your own hands. No cloud, no middleman, no breach-in-waiting."
    residency_note = "Austrian tax defaults (FIFO, EUR, KESt 27.5%) will be applied. Editable later."
    if not has_profile:
        residency_note = "Create your first local workspace from the CLI for now. This onboarding flow becomes live in the next phase."
    return {
        "title": title,
        "body": body,
        "workspace_value": status.get("current_workspace") or "My Books",
        "name_placeholder": "e.g. Alice",
        "residency_options": [
            {"code": "AT", "label": "AT"},
            {"code": "DE", "label": "DE"},
            {"code": "CH", "label": "CH"},
            {"code": "EU", "label": "EU"},
            {"code": "OTHER", "label": "Other"},
        ],
        "residency_note": residency_note,
        "stamp_caption": "LOCAL  •  SELF-HOSTED  •  BITCOIN-ONLY",
    }


def _profile_counts(conn: sqlite3.Connection, profile_id: str) -> dict[str, int]:
    transaction_count = conn.execute(
        "SELECT COUNT(*) AS count FROM transactions WHERE profile_id = ? AND excluded = 0",
        (profile_id,),
    ).fetchone()["count"]
    journal_count = conn.execute(
        "SELECT COUNT(*) AS count FROM journal_entries WHERE profile_id = ?",
        (profile_id,),
    ).fetchone()["count"]
    quarantine_count = conn.execute(
        "SELECT COUNT(*) AS count FROM journal_quarantines WHERE profile_id = ?",
        (profile_id,),
    ).fetchone()["count"]
    tag_count = conn.execute(
        "SELECT COUNT(*) AS count FROM tags WHERE profile_id = ?",
        (profile_id,),
    ).fetchone()["count"]
    return {
        "transactions": transaction_count,
        "journal_entries": journal_count,
        "quarantines": quarantine_count,
        "tags": tag_count,
    }


def _build_connection_items(
    conn: sqlite3.Connection,
    workspace_id: str,
    profile_id: str,
) -> list[dict[str, Any]]:
    wallets = core_wallets.list_wallets(conn, workspace_id, profile_id)
    items = []
    for wallet in wallets:
        transaction_count = wallet_transaction_count(conn, wallet["id"])
        status_label, status_tone = _connection_status(wallet, transaction_count)
        wallet_stats = conn.execute(
            """
            SELECT
                COALESCE(SUM(amount), 0) AS balance_msat,
                MAX(occurred_at) AS last_occurred_at
            FROM transactions
            WHERE wallet_id = ? AND excluded = 0
            """,
            (wallet["id"],),
        ).fetchone()
        balance_msat = int(wallet_stats["balance_msat"] or 0)
        last_occurred_at = wallet_stats["last_occurred_at"]
        descriptor_value = wallet.get("descriptor") or "Not set"
        backend_value = wallet.get("backend") or "Local import / none"
        reference_value = wallet.get("source_file") or wallet.get("addresses") or descriptor_value
        items.append(
            {
                "id": wallet["id"],
                "label": wallet["label"],
                "subtitle": f"{wallet['kind'].upper()} on {wallet.get('chain') or 'bitcoin'}",
                "kind": wallet["kind"],
                "account": wallet.get("account", ""),
                "chain": wallet.get("chain", "") or "bitcoin",
                "network": wallet.get("network", "") or "mainnet",
                "backend": backend_value,
                "descriptor": descriptor_value,
                "reference": reference_value,
                "source_file": wallet.get("source_file", ""),
                "source_format": wallet.get("source_format", ""),
                "altbestand": "Yes" if wallet.get("altbestand") == "yes" else "No",
                "balance_msat": balance_msat,
                "balance_label": _format_btc_msat(balance_msat),
                "balance_short": f"{msat_to_btc(balance_msat):,.4f}",
                "transaction_count": transaction_count,
                "transaction_count_label": _format_count(transaction_count),
                "status_label": status_label,
                "status_tone": status_tone,
                "last_activity": last_occurred_at or "",
                "last_activity_label": _format_timestamp(last_occurred_at),
                "created_at": wallet.get("created_at", ""),
                "created_at_label": _format_timestamp(wallet.get("created_at")),
                "detail_rows": [
                    {"label": "Bucket", "value": wallet.get("account", "") or "Unassigned"},
                    {"label": "Backend", "value": backend_value},
                    {"label": "Balance", "value": _format_btc_msat(balance_msat)},
                    {"label": "Reference", "value": reference_value},
                    {"label": "Transactions", "value": _format_count(transaction_count)},
                    {"label": "Last activity", "value": _format_timestamp(last_occurred_at)},
                    {"label": "Altbestand", "value": "Yes" if wallet.get("altbestand") == "yes" else "No"},
                ],
            }
        )
    return items


def _build_transaction_items(conn: sqlite3.Connection, profile_id: str, fiat_currency: str) -> list[dict[str, Any]]:
    rows = conn.execute(
        """
        SELECT
            t.id,
            t.occurred_at,
            a.label AS account_label,
            w.label AS wallet_label,
            t.direction,
            t.asset,
            t.amount,
            t.fee,
            t.fiat_value,
            COALESCE(t.fiat_currency, ?) AS fiat_currency,
            COALESCE(t.kind, '') AS kind,
            COALESCE(t.description, '') AS description,
            COALESCE(t.note, '') AS note,
            COALESCE(GROUP_CONCAT(tags.code, ', '), '') AS tags
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        LEFT JOIN accounts a ON a.id = w.account_id
        LEFT JOIN transaction_tags tt ON tt.transaction_id = t.id
        LEFT JOIN tags ON tags.id = tt.tag_id
        WHERE t.profile_id = ? AND t.excluded = 0
        GROUP BY t.id
        ORDER BY t.occurred_at DESC, t.created_at DESC, t.id DESC
        LIMIT ?
        """,
        (fiat_currency, profile_id, _transaction_limit()),
    ).fetchall()
    items = []
    for row in rows:
        amount_msat = int(row["amount"] or 0)
        signed_amount_msat = _signed_amount_for_display(amount_msat, row["direction"])
        type_label = _transaction_type_label(row["direction"], row["kind"])
        event_label = _titleize_token(row["kind"] or row["direction"])
        tags = row["tags"] or "No tags"
        account_label = row["account_label"] or row["wallet_label"] or "Unassigned"
        items.append(
            {
                "id": row["id"],
                "title": event_label,
                "kind_label": type_label,
                "type_label": type_label,
                "event_label": event_label,
                "type_badge_tone": _transaction_badge_tone(type_label),
                "type_tone": _amount_tone(signed_amount_msat),
                "account_label": account_label,
                "wallet": row["wallet_label"],
                "occurred_at": row["occurred_at"],
                "occurred_at_label": _format_timestamp(row["occurred_at"]),
                "occurred_on_label": _format_day(row["occurred_at"]),
                "asset": row["asset"],
                "amount": signed_amount_msat,
                "amount_msat": signed_amount_msat,
                "amount_sats": _msat_to_sats(signed_amount_msat),
                "amount_label": _format_btc_msat(signed_amount_msat),
                "amount_sats_signed_label": _format_signed_sats(signed_amount_msat),
                "fee_label": _format_btc_msat(row["fee"]),
                "fee_sats_label": _format_signed_sats(row["fee"]),
                "fiat_value": row["fiat_value"] or 0,
                "fiat_label": _format_fiat(row["fiat_value"], row["fiat_currency"]),
                "direction": row["direction"],
                "counterparty": row["description"] or row["note"] or row["wallet_label"],
                "description": row["description"] or "No description captured.",
                "note": row["note"] or "No note attached.",
                "tags": tags,
                "tag_label": tags.split(",")[0].strip() if tags else "No tags",
                "subtitle": f"{account_label}  |  {row['wallet_label']}  |  {row['asset']}",
                "detail_rows": [
                    {"label": "Occurred", "value": _format_timestamp(row["occurred_at"])},
                    {"label": "Event", "value": event_label},
                    {"label": "Bucket", "value": account_label},
                    {"label": "Wallet", "value": row["wallet_label"]},
                    {"label": "Direction", "value": row["direction"].replace("_", " ").title()},
                    {"label": "Amount", "value": _format_btc_msat(row["amount"])},
                    {"label": "Fee", "value": _format_btc_msat(row["fee"])},
                    {"label": "Fiat", "value": _format_fiat(row["fiat_value"], row["fiat_currency"])},
                    {"label": "Tags", "value": tags},
                ],
            }
        )
    return items


def _build_report_section(
    profile: dict[str, Any],
    counts: dict[str, int],
    transactions: list[dict[str, Any]],
) -> dict[str, Any]:
    tax_country = str(profile.get("tax_country") or "generic").lower()
    gains_algorithm = str(profile.get("gains_algorithm") or "FIFO").upper()
    processed_fresh = bool(
        profile.get("last_processed_at")
        and int(profile.get("last_processed_tx_count") or 0) == counts["transactions"]
    )
    supports_runtime_reports = tax_country == "generic"
    if not supports_runtime_reports:
        status_title = "Report preview unavailable"
        status_body = (
            f"{tax_country.upper()} tax processing is not available in Kassiber yet. "
            "This desktop surface only shows readiness and profile policy metadata."
        )
        status_tone = "warn"
    elif counts["quarantines"]:
        status_title = "Reports blocked by quarantines"
        status_body = "Resolve or exclude quarantined transactions before trusting downstream reports."
        status_tone = "warn"
    elif processed_fresh:
        status_title = "Reports ready"
        status_body = "Journal data looks fresh enough for the read-only reports surface."
        status_tone = "ok"
    else:
        status_title = "Reports waiting on journals"
        status_body = "Run `kassiber journals process` so the tax-report surfaces can trust the current transactions."
        status_tone = "warn"
    items = []
    for report_id, label, summary in _REPORT_SPECS:
        items.append(
            {
                "id": report_id,
                "label": label,
                "summary": summary,
                "status": (
                    "Ready"
                    if supports_runtime_reports and processed_fresh and counts["quarantines"] == 0
                    else ("Unavailable" if not supports_runtime_reports else "Needs attention")
                ),
                "status_tone": (
                    "ok"
                    if supports_runtime_reports and processed_fresh and counts["quarantines"] == 0
                    else "warn"
                ),
            }
        )
    fiat_currency = str(profile.get("fiat_currency") or "EUR").upper()
    preview_rows = [
        {
            "occurred": item.get("occurred_at_label", ""),
            "occurred_on_label": item.get("occurred_on_label", ""),
            "wallet": item.get("wallet", ""),
            "account_label": item.get("account_label", ""),
            "kind_label": item.get("kind_label", ""),
            "type_label": item.get("type_label", ""),
            "type_badge_tone": item.get("type_badge_tone", "muted"),
            "amount_label": item.get("amount_label", ""),
            "fiat_label": item.get("fiat_label", ""),
            "tag_label": item.get("tag_label", ""),
        }
        for item in transactions[:6]
    ]
    method_details = {
        "FIFO": "First-in, first-out",
        "LIFO": "Last-in, first-out",
        "HIFO": "Highest-in, first-out",
        "LOFO": "Lowest-in, first-out",
    }
    return {
        "header_eyebrow": _report_header_eyebrow(tax_country, fiat_currency),
        "status_title": status_title,
        "status_body": status_body,
        "status_tone": status_tone,
        "items": items,
        "summary_cards": [
            {
                "label": "Transactions",
                "value": _format_count(counts["transactions"]),
                "tone": "ok",
                "detail": "Included in the current profile",
            },
            {
                "label": "Journal entries",
                "value": _format_count(counts["journal_entries"]),
                "tone": "ok" if counts["journal_entries"] else "warn",
                "detail": "Derived after processing",
            },
            {
                "label": "Quarantines",
                "value": _format_count(counts["quarantines"]),
                "tone": "warn" if counts["quarantines"] else "ok",
                "detail": "Need review before trusting reports",
            },
            {
                "label": "Lot method",
                "value": gains_algorithm,
                "tone": "neutral",
                "detail": f"{fiat_currency} profile policy",
            },
        ],
        "preview_title": _report_preview_title(preview_rows),
        "preview_subtitle": _report_preview_subtitle(
            supports_runtime_reports,
            processed_fresh,
            counts["quarantines"],
            status_body,
        ),
        "preview_empty_hint": _report_empty_hint(preview_rows, status_body),
        "method_options": [
            {
                "id": algorithm.lower(),
                "label": algorithm,
                "detail": method_details[algorithm],
                "selected": gains_algorithm == algorithm,
            }
            for algorithm in ("FIFO", "LIFO", "HIFO", "LOFO")
        ],
        "policy_rows": [
            {
                "label": "Tax policy",
                "detail": tax_country.upper(),
            },
            {
                "label": "Journals",
                "detail": status_title,
            },
            {
                "label": "Cost basis pooling",
                "detail": "Per asset across all wallets in the active profile.",
            },
            {
                "label": "Pricing source",
                "detail": "Stored transaction and journal pricing, not live rates queries.",
            },
        ],
        "preview_rows": preview_rows,
        "export_formats": [
            {"label": "CSV", "summary": "Spreadsheet", "detail": "Tabular export for review", "primary": True},
            {"label": "JSON", "summary": "Envelope", "detail": "Machine-readable report payload", "primary": False},
            {"label": "PLAIN", "summary": "Terminal", "detail": "Human-readable CLI output", "primary": False},
        ],
    }


def _build_settings_section(status: dict[str, Any], profile: dict[str, Any] | None) -> dict[str, Any]:
    cards = [
        {"label": "settings.json", "value": status.get("settings_file", ""), "hint": "Managed UI and runtime settings manifest."},
        {"label": "env file", "value": status.get("env_file", ""), "hint": "Runtime backend seed file."},
        {"label": "data root", "value": status.get("data_root", ""), "hint": "SQLite system of record and state root."},
        {"label": "exports", "value": status.get("exports_root", ""), "hint": "Report and CSV output destination."},
        {"label": "attachments", "value": status.get("attachments_root", ""), "hint": "Managed attachment storage for transaction files."},
    ]
    if profile:
        cards.insert(
            0,
            {
                "label": "active profile",
                "value": (
                    f"{profile.get('label', '')}  |  {profile.get('fiat_currency', '')}  |  "
                    f"{profile.get('tax_country', '').upper()}  |  {profile.get('gains_algorithm', '')}"
                ),
                "hint": "The current report policy the desktop shell is reading from.",
            },
        )
    return {
        "cards": cards,
        "privacy_rows": [
            {
                "label": "Hide sensitive data",
                "detail": "Blur balances, addresses, and amounts across the UI.",
                "enabled": True,
            },
            {
                "label": "Clear clipboard after 30s",
                "detail": "Auto-clear copied addresses and exported values.",
                "enabled": True,
            },
        ],
        "lock_rows": [
            {
                "label": "Auto-lock when idle",
                "detail": "Require a passphrase after inactivity.",
                "enabled": True,
            },
            {
                "label": "Require passphrase on launch",
                "detail": "Prompt before opening the current workspace.",
                "enabled": True,
            },
            {
                "label": "Lock on window close",
                "detail": "Clear decrypted state when the app exits.",
                "enabled": True,
            },
        ],
        "idle_options": [1, 5, 15, 30, 60],
        "active_idle_option": 5,
        "backend_rows": [
            {
                "label": status.get("default_backend", "") or "mempool.space",
                "value": "Default sync backend",
                "status": "active",
            },
            {
                "label": "CoinGecko",
                "value": "Rates source",
                "status": "active",
            },
            {
                "label": status.get("env_file", "") or "No env file",
                "value": "Runtime env seed",
                "status": "local",
            },
        ],
        "data_actions": [
            {"label": "Backup", "detail": "Archive runtime state"},
            {"label": "Restore", "detail": "Recover from backup"},
            {"label": "Logs", "detail": "Inspect local desktop logs"},
        ],
    }



def _empty_shell(notices: list[str] | None = None) -> dict[str, Any]:
    return {
        "phase": 1,
        "window_title": "Kassiber - local",
        "project_label": "No project selected",
        "current_workspace_label": "",
        "current_profile_label": "",
        "connection_count": 0,
        "is_empty": True,
        "empty_state_title": "Create a profile in the CLI first",
        "empty_state_body": (
            "Kassiber's Phase 1 desktop shell is ready, but it still depends on the existing "
            "workspace/profile setup from the CLI."
        ),
        "placeholder_title": "Dashboard coming next",
        "placeholder_body": (
            "The PySide6 shell is in place. Read-only dashboard tiles land in Phase 2."
        ),
        "notices": notices or [],
    }


def collect_ui_snapshot(
    conn: sqlite3.Connection,
    data_root: str,
    runtime_config: dict[str, Any],
    workspace_ref: str | None = None,
    profile_ref: str | None = None,
) -> dict[str, Any]:
    status = build_status_payload(conn, data_root)
    status["default_backend"] = runtime_config.get("default_backend", "")
    status["env_file"] = runtime_config.get("env_file", "")
    welcome = _build_welcome_section(status, has_profile=status.get("profiles", 0) > 0)

    snapshot: dict[str, Any] = {
        "status": status,
        "context": current_context_snapshot(conn),
        "scope": None,
        "profiles": [],
        "shell": _empty_shell(),
        "welcome": welcome,
        "overview": {"metrics": [], "highlights": []},
        "connections": {"items": []},
        "transactions": {"items": [], "total_count": 0, "history_label": "LOCAL SNAPSHOT", "filter_options": []},
        "reports": {
            "header_eyebrow": "REPORT  |  GENERIC POLICY  |  EUR",
            "status_title": "",
            "status_body": "",
            "status_tone": "warn",
            "items": [],
            "summary_cards": [],
            "preview_title": "Preview unavailable",
            "preview_subtitle": "",
            "preview_empty_hint": "",
        },
        "settings": _build_settings_section(status, None),
    }
    explicit_scope = bool(workspace_ref or profile_ref)

    try:
        workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    except AppError as exc:
        if explicit_scope:
            raise
        snapshot["shell"] = _empty_shell([str(exc)])
        return snapshot

    profiles = core_accounts.list_profiles(conn, workspace["id"])
    wallets = core_wallets.list_wallets(conn, workspace["id"], profile["id"])
    connection_count = len(wallets)
    profile_details = core_accounts.get_profile_details(conn, workspace["id"], profile["id"])
    counts = _profile_counts(conn, profile["id"])
    connections = _build_connection_items(conn, workspace["id"], profile["id"])
    transactions = _build_transaction_items(conn, profile["id"], profile_details["fiat_currency"])
    reports = _build_report_section(profile_details, counts, transactions)
    notices = [
        "The Add Connection modal is a Phase 1 placeholder. Use the CLI for wallet creation and sync today.",
    ]
    if connection_count:
        notices.append("Connections already exist. This shell now exposes read-only mockup routes while the tile-level dashboard fills in.")

    snapshot["scope"] = {
        "workspace_id": workspace["id"],
        "workspace_label": workspace["label"],
        "profile_id": profile["id"],
        "profile_label": profile["label"],
    }
    snapshot["profiles"] = profiles
    snapshot["welcome"] = _build_welcome_section(status, has_profile=True)
    snapshot["shell"] = {
        "phase": 1,
        "window_title": "Kassiber - local",
        "project_label": f"{workspace['label']} / {profile['label']}",
        "current_workspace_label": workspace["label"],
        "current_profile_label": profile["label"],
        "connection_count": connection_count,
        "is_empty": counts["transactions"] == 0,
        "has_data": counts["transactions"] > 0,
        "empty_state_title": "Add a connection",
        "empty_state_body": "Add a connection and automatically sync your transaction data to get started.",
        "placeholder_title": "Connections detected",
        "placeholder_body": (
            "Your current project already has connection data. The routed desktop shell is now in place "
            "so the final dashboard tiles can drop into real screens instead of one placeholder card."
        ),
        "notices": notices,
    }
    snapshot["overview"] = {
        "metrics": [
            {"label": "Connections", "value": _format_count(connection_count), "tone": "ok" if connection_count else "warn"},
            {"label": "Transactions", "value": _format_count(counts["transactions"]), "tone": "ok" if counts["transactions"] else "warn"},
            {"label": "Journal entries", "value": _format_count(counts["journal_entries"]), "tone": "ok" if counts["journal_entries"] else "warn"},
            {"label": "Quarantines", "value": _format_count(counts["quarantines"]), "tone": "warn" if counts["quarantines"] else "ok"},
        ],
        "highlights": [
            {
                "title": "Profile",
                "value": f"{profile_details['fiat_currency']}  |  {profile_details['tax_country'].upper()}",
                "body": f"{profile_details['gains_algorithm']} accounting  |  {_format_count(counts['tags'])} tags available",
            },
            {
                "title": "Latest activity",
                "value": transactions[0]["occurred_at_label"] if transactions else "No imported data yet",
                "body": transactions[0]["subtitle"] if transactions else "Connect a wallet or import a file to populate the dashboard.",
            },
            {
                "title": "Report readiness",
                "value": reports["status_title"],
                "body": reports["status_body"],
            },
        ],
    }
    snapshot["connections"] = {"items": connections}
    snapshot["transactions"] = {
        "items": transactions,
        "total_count": counts["transactions"],
        "history_label": _transaction_history_label(counts["transactions"], len(transactions)),
        "filter_options": _transaction_filter_options(transactions),
    }
    snapshot["reports"] = reports
    snapshot["settings"] = _build_settings_section(status, profile_details)
    return snapshot


def load_ui_snapshot(
    data_root: str | None = None,
    env_file: str | None = None,
    workspace_ref: str | None = None,
    profile_ref: str | None = None,
) -> dict[str, Any]:
    paths = ensure_runtime_layout(resolve_runtime_paths(data_root, env_file))
    runtime_config = load_runtime_config(paths.env_file)
    conn = open_db(paths.data_root)
    try:
        merge_db_backends(conn, runtime_config)
        return collect_ui_snapshot(
            conn,
            paths.data_root,
            runtime_config,
            workspace_ref=workspace_ref,
            profile_ref=profile_ref,
        )
    finally:
        conn.close()


__all__ = ["collect_ui_snapshot", "load_ui_snapshot"]
