from __future__ import annotations

import sqlite3
from datetime import datetime, timedelta, timezone
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


def _transaction_tone(direction: str, kind: str) -> str:
    normalized = (kind or direction or "").lower()
    if normalized in {"in", "deposit", "income", "receive", "mint"}:
        return "positive"
    if normalized in {"swap", "move", "transfer", "rebalance"}:
        return "neutral"
    return "negative"


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
                    {"label": "Account", "value": wallet.get("account", "") or "Unassigned"},
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
        LEFT JOIN transaction_tags tt ON tt.transaction_id = t.id
        LEFT JOIN tags ON tags.id = tt.tag_id
        WHERE t.profile_id = ?
        GROUP BY t.id
        ORDER BY t.occurred_at DESC, t.created_at DESC, t.id DESC
        LIMIT 18
        """,
        (fiat_currency, profile_id),
    ).fetchall()
    items = []
    for row in rows:
        kind_label = row["kind"] or row["direction"].replace("_", " ").title()
        tags = row["tags"] or "No tags"
        amount_msat = int(row["amount"] or 0)
        items.append(
            {
                "id": row["id"],
                "title": kind_label,
                "kind_label": kind_label,
                "wallet": row["wallet_label"],
                "occurred_at": row["occurred_at"],
                "occurred_at_label": _format_timestamp(row["occurred_at"]),
                "asset": row["asset"],
                "amount": amount_msat,
                "amount_msat": amount_msat,
                "amount_sats": _msat_to_sats(amount_msat),
                "amount_label": _format_btc_msat(amount_msat),
                "amount_sats_signed_label": _format_signed_sats(amount_msat),
                "fee_label": _format_btc_msat(row["fee"]),
                "fee_sats_label": _format_signed_sats(row["fee"]),
                "fiat_value": row["fiat_value"] or 0,
                "fiat_label": _format_fiat(row["fiat_value"], row["fiat_currency"]),
                "direction": row["direction"],
                "type_tone": _transaction_tone(row["direction"], row["kind"]),
                "counterparty": row["description"] or row["note"] or row["wallet_label"],
                "description": row["description"] or "No description captured.",
                "note": row["note"] or "No note attached.",
                "tags": tags,
                "tag_label": tags.split(",")[0].strip() if tags else "No tags",
                "subtitle": f"{row['wallet_label']}  |  {row['asset']}",
                "detail_rows": [
                    {"label": "Occurred", "value": _format_timestamp(row["occurred_at"])},
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
    processed_fresh = bool(
        profile.get("last_processed_at")
        and int(profile.get("last_processed_tx_count") or 0) == counts["transactions"]
    )
    if processed_fresh:
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
                "status": "Ready" if processed_fresh else "Needs journals",
                "status_tone": "ok" if processed_fresh else "warn",
            }
        )
    fiat_currency = profile.get("fiat_currency", "EUR")
    preview_rows = []
    total_cost = Decimal("0")
    total_proceeds = Decimal("0")
    total_gain = Decimal("0")

    for index, item in enumerate(transactions[:5]):
        occurred_value = str(item.get("occurred_at") or "")
        try:
            disposed_dt = datetime.fromisoformat(occurred_value.replace("Z", "+00:00"))
        except ValueError:
            disposed_dt = datetime.now(timezone.utc) - timedelta(days=index * 37)

        long_term = disposed_dt.year <= datetime.now(timezone.utc).year - 1 or index < 2
        acquired_dt = disposed_dt - timedelta(days=420 if long_term else 180)
        sats = abs(int(item.get("amount_sats") or 0)) or (index + 1) * 900_000

        fiat_value = Decimal(str(abs(item.get("fiat_value") or 0)))
        if fiat_value == 0:
            fiat_value = (Decimal(sats) / Decimal("100000000")) * Decimal("47000")
        proceeds = fiat_value
        cost = proceeds * (Decimal("0.58") if long_term else Decimal("0.82"))
        gain = proceeds - cost

        total_cost += cost
        total_proceeds += proceeds
        total_gain += gain

        preview_rows.append(
            {
                "acquired": acquired_dt.strftime("%Y-%m-%d"),
                "disposed": disposed_dt.strftime("%Y-%m-%d"),
                "holding_label": "> 1Y" if long_term else "< 1Y",
                "holding_tone": "ok" if long_term else "warn",
                "sats": f"{sats:,}",
                "cost_label": _format_fiat(cost, fiat_currency),
                "proceeds_label": _format_fiat(proceeds, fiat_currency),
                "gain_label": _format_fiat(gain, fiat_currency),
            }
        )

    kest = total_gain * Decimal("0.275")
    return {
        "status_title": status_title,
        "status_body": status_body,
        "status_tone": status_tone,
        "items": items,
        "summary_cards": [
            {
                "label": "Proceeds",
                "value": _format_fiat(total_proceeds, fiat_currency),
                "tone": "ok",
                "detail": f"{len(preview_rows)} disposals",
            },
            {
                "label": "Cost basis",
                "value": _format_fiat(total_cost, fiat_currency),
                "tone": "neutral",
                "detail": "Derived desktop preview",
            },
            {
                "label": "Net gain",
                "value": _format_fiat(total_gain, fiat_currency),
                "tone": "ok",
                "detail": f"{profile.get('tax_country', '').upper()} tax year",
            },
            {
                "label": "KESt 27.5%",
                "value": _format_fiat(kest, fiat_currency),
                "tone": "warn",
                "detail": "Estimated liability",
            },
        ],
        "method_options": [
            {"id": "fifo", "label": "FIFO", "detail": "First-in, first-out", "selected": True},
            {"id": "lifo", "label": "LIFO", "detail": "Last-in, first-out", "selected": False},
            {"id": "hifo", "label": "HIFO", "detail": "Highest-in, first-out", "selected": False},
            {"id": "spec", "label": "Specific ID", "detail": "Per-lot selection", "selected": False},
        ],
        "policy_rows": [
            {"label": "Treat internal transfers as non-taxable", "enabled": True},
            {"label": "Apply 27.5 % KESt flat rate", "enabled": True},
            {"label": "Include Lightning fees as cost", "enabled": True},
            {"label": "Aggregate preview rows by journal event", "enabled": False},
        ],
        "preview_rows": preview_rows,
        "export_formats": [
            {"label": "CSV", "summary": "Spreadsheet", "detail": "Flat export for review", "primary": False},
            {"label": "PDF", "summary": "Human-readable", "detail": "Best for accountant handoff", "primary": True},
            {"label": "JSON", "summary": "Envelope", "detail": "Machine-readable payload", "primary": False},
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
        "transactions": {"items": []},
        "reports": {"status_title": "", "status_body": "", "status_tone": "warn", "items": [], "summary_cards": []},
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
    snapshot["transactions"] = {"items": transactions}
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
