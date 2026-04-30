from __future__ import annotations

import json
import sqlite3
from collections import defaultdict
from datetime import datetime, timezone
from typing import Any

from ..backends import redact_backend_for_output
from ..errors import AppError
from ..msat import msat_to_btc
from ..time_utils import _iso_z, _parse_iso_datetime
from .repo import current_context_snapshot


MAX_UI_LIST_LIMIT = 500
MAX_UI_PREVIEW_LIMIT = 100


def _empty_overview_snapshot() -> dict[str, Any]:
    return {
        "priceEur": 0.0,
        "priceUsd": 0.0,
        "connections": [],
        "txs": [],
        "balanceSeries": [0.0] * 12,
        "portfolioSeries": [],
        "fiat": {
            "eurBalance": 0.0,
            "eurCostBasis": 0.0,
            "eurUnrealized": 0.0,
            "eurRealizedYTD": 0.0,
        },
        "status": {
            "workspace": None,
            "profile": None,
            "transactionCount": 0,
            "needsJournals": False,
            "quarantines": 0,
        },
    }


def _latest_rate(conn: sqlite3.Connection, pair: str) -> float:
    row = conn.execute(
        """
        SELECT rate
        FROM rates_cache
        WHERE pair = ?
        ORDER BY timestamp DESC, fetched_at DESC
        LIMIT 1
        """,
        (pair,),
    ).fetchone()
    return float(row["rate"]) if row else 0.0


def _latest_transaction_rate(
    conn: sqlite3.Connection,
    profile_id: str,
    fiat_currency: str,
) -> float:
    row = conn.execute(
        """
        SELECT fiat_rate
        FROM transactions
        WHERE profile_id = ?
          AND excluded = 0
          AND asset = 'BTC'
          AND fiat_currency = ?
          AND fiat_rate IS NOT NULL
        ORDER BY occurred_at DESC, created_at DESC, id DESC
        LIMIT 1
        """,
        (profile_id, fiat_currency),
    ).fetchone()
    return float(row["fiat_rate"]) if row else 0.0


def _coerce_args(args: dict[str, Any] | None) -> dict[str, Any]:
    if args is None:
        return {}
    if isinstance(args, dict):
        return args
    raise AppError(
        "ui snapshot args must be an object",
        code="validation",
        details={"type": type(args).__name__},
        retryable=False,
    )


def _coerce_limit(
    args: dict[str, Any],
    *,
    default: int,
    maximum: int,
) -> int:
    raw = args.get("limit", default)
    try:
        limit = int(raw)
    except (TypeError, ValueError):
        raise AppError(
            "limit must be an integer",
            code="validation",
            details={"limit": raw},
            retryable=False,
        ) from None
    if limit < 1:
        raise AppError(
            "limit must be positive",
            code="validation",
            details={"limit": raw},
            retryable=False,
        )
    return min(limit, maximum)


def _json_config(value: str | None) -> dict[str, Any]:
    try:
        parsed = json.loads(value or "{}")
    except (TypeError, ValueError, json.JSONDecodeError):
        return {}
    return parsed if isinstance(parsed, dict) else {}


def _string_or_empty(value: Any) -> str:
    return value.strip() if isinstance(value, str) else ""


def _wallet_backend_summary(
    kind: str,
    config: dict[str, Any],
    default_backend: Any,
) -> dict[str, str]:
    explicit_backend = _string_or_empty(config.get("backend"))
    default_backend_name = _string_or_empty(default_backend)
    source_file = _string_or_empty(config.get("source_file"))
    source_format = _string_or_empty(config.get("source_format"))
    sync_source = _string_or_empty(config.get("sync_source"))
    has_descriptor = bool(config.get("descriptor"))
    has_addresses = bool(config.get("addresses"))
    backend_name = explicit_backend
    backend_source = "explicit" if explicit_backend else "none"
    sync_mode = "not_configured"

    if sync_source == "btcpay":
        sync_mode = "btcpay"
    elif source_file and source_format:
        sync_mode = "file_import"
    elif has_descriptor and kind in {"descriptor", "xpub", "address"}:
        sync_mode = "backend_descriptor"
        if not backend_name and default_backend_name:
            backend_name = default_backend_name
            backend_source = "default"
    elif has_addresses and kind == "address":
        sync_mode = "backend_addresses"
        if not backend_name and default_backend_name:
            backend_name = default_backend_name
            backend_source = "default"

    if not backend_name:
        backend_source = "none"

    return {
        "name": backend_name,
        "source": backend_source,
        "sync_mode": sync_mode,
    }


def _active_context_and_profile(
    conn: sqlite3.Connection,
) -> tuple[dict[str, str], sqlite3.Row | None]:
    context = current_context_snapshot(conn)
    if not context["workspace_id"] or not context["profile_id"]:
        return context, None
    profile = conn.execute(
        "SELECT * FROM profiles WHERE id = ?",
        (context["profile_id"],),
    ).fetchone()
    return context, profile


def _journal_freshness(
    conn: sqlite3.Connection,
    profile: sqlite3.Row | None,
) -> dict[str, Any]:
    if profile is None:
        return {
            "status": "no_profile",
            "needs_processing": False,
            "last_processed_at": None,
            "last_processed_tx_count": 0,
            "active_transaction_count": 0,
            "journal_entry_count": 0,
            "quarantine_count": 0,
            "reason": "no active profile",
        }

    active_transactions = conn.execute(
        """
        SELECT COUNT(*) AS count
        FROM transactions
        WHERE profile_id = ? AND excluded = 0
        """,
        (profile["id"],),
    ).fetchone()["count"]
    journal_entries = conn.execute(
        "SELECT COUNT(*) AS count FROM journal_entries WHERE profile_id = ?",
        (profile["id"],),
    ).fetchone()["count"]
    quarantines = conn.execute(
        "SELECT COUNT(*) AS count FROM journal_quarantines WHERE profile_id = ?",
        (profile["id"],),
    ).fetchone()["count"]
    last_processed_at = profile["last_processed_at"]
    last_processed_tx_count = int(profile["last_processed_tx_count"] or 0)
    active_count = int(active_transactions or 0)
    if active_count == 0:
        status = "no_transactions"
        reason = "no active transactions"
    elif not last_processed_at:
        status = "not_processed"
        reason = "journals have not been processed"
    elif last_processed_tx_count != active_count:
        status = "stale"
        reason = "active transaction count changed since last processing"
    else:
        status = "current"
        reason = "journals match the active transaction count"
    return {
        "status": status,
        "needs_processing": status in {"not_processed", "stale"},
        "last_processed_at": last_processed_at,
        "last_processed_tx_count": last_processed_tx_count,
        "active_transaction_count": active_count,
        "journal_entry_count": int(journal_entries or 0),
        "quarantine_count": int(quarantines or 0),
        "reason": reason,
    }


def _map_wallet_kind(kind: str) -> str:
    normalized = (kind or "").lower().replace("_", "-")
    aliases = {
        "address": "address",
        "coreln": "core-ln",
        "btcpay_csv": "btcpay",
        "btcpay-json": "btcpay",
        "btcpay-csv": "btcpay",
        "phoenix": "phoenix",
        "phoenix-csv": "csv",
    }
    return aliases.get(normalized, normalized or "csv")


def _public_explorer_id(external_id: str | None) -> str | None:
    candidate = (external_id or "").strip()
    if len(candidate) != 64:
        return None
    if all(char in "0123456789abcdefABCDEF" for char in candidate):
        return candidate
    return None


def _relative_last(value: str | None) -> str:
    if not value:
        return "never"
    try:
        dt = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return value
    now = datetime.now(timezone.utc)
    delta = now - dt.astimezone(timezone.utc)
    seconds = max(0, int(delta.total_seconds()))
    if seconds < 60:
        return "just now"
    minutes = seconds // 60
    if minutes < 60:
        return f"{minutes}m ago"
    hours = minutes // 60
    if hours < 24:
        return f"{hours}h ago"
    days = hours // 24
    if days < 30:
        return f"{days}d ago"
    return dt.date().isoformat()


def _workspace_jurisdiction(tax_countries: list[str]) -> str:
    normalized = {country.strip().lower() for country in tax_countries if country}
    if not normalized:
        return "Generic"
    if normalized == {"at"}:
        return "Austria"
    if len(normalized) == 1:
        return next(iter(normalized)).upper()
    return "Mixed"


def _tax_policy_label(profile: sqlite3.Row) -> str:
    country = str(profile["tax_country"] or "generic").strip().upper()
    if country == "AT":
        return f"Austria - {profile['gains_algorithm']} - {profile['fiat_currency']}"
    elif country == "GENERIC":
        country_label = "Generic"
    else:
        country_label = country
    return (
        f"{country_label} - {profile['gains_algorithm']} - "
        f"{profile['fiat_currency']} - {profile['tax_long_term_days']} day long-term"
    )


def build_profiles_snapshot(conn: sqlite3.Connection) -> dict[str, Any]:
    context = current_context_snapshot(conn)
    profile_rows = conn.execute(
        """
        SELECT
            p.id,
            p.workspace_id,
            p.label,
            p.fiat_currency,
            p.tax_country,
            p.tax_long_term_days,
            p.gains_algorithm,
            p.last_processed_at,
            p.created_at,
            COUNT(DISTINCT a.id) AS account_count,
            COUNT(DISTINCT w.id) AS wallet_count
        FROM profiles p
        LEFT JOIN accounts a ON a.profile_id = p.id
        LEFT JOIN wallets w ON w.profile_id = p.id
        GROUP BY p.id
        ORDER BY p.created_at ASC, p.label ASC
        """
    ).fetchall()
    profiles_by_workspace: dict[str, list[dict[str, Any]]] = defaultdict(list)
    tax_countries_by_workspace: dict[str, list[str]] = defaultdict(list)
    currencies_by_workspace: dict[str, list[str]] = defaultdict(list)

    for row in profile_rows:
        is_active = row["id"] == context["profile_id"]
        tax_countries_by_workspace[row["workspace_id"]].append(row["tax_country"])
        currencies_by_workspace[row["workspace_id"]].append(row["fiat_currency"])
        profiles_by_workspace[row["workspace_id"]].append(
            {
                "id": row["id"],
                "name": row["label"],
                "role": "Owner",
                "taxPolicy": _tax_policy_label(row),
                "accounts": int(row["account_count"] or 0),
                "wallets": int(row["wallet_count"] or 0),
                "lastOpened": (
                    "Just now"
                    if is_active
                    else _relative_last(row["last_processed_at"] or row["created_at"])
                ),
                "active": is_active,
            }
        )

    workspace_rows = conn.execute(
        """
        SELECT id, label, created_at
        FROM workspaces
        ORDER BY created_at ASC, label ASC
        """
    ).fetchall()
    workspaces = []
    for row in workspace_rows:
        currencies = {
            currency.strip().upper()
            for currency in currencies_by_workspace.get(row["id"], [])
            if currency
        }
        workspaces.append(
            {
                "id": row["id"],
                "name": row["label"],
                "kind": "Personal",
                "currency": next(iter(currencies)) if len(currencies) == 1 else "Mixed",
                "jurisdiction": _workspace_jurisdiction(
                    tax_countries_by_workspace.get(row["id"], []),
                ),
                "created": (row["created_at"] or "")[:10],
                "profiles": profiles_by_workspace.get(row["id"], []),
            }
        )

    return {
        "workspaces": workspaces,
        "activeWorkspaceId": context["workspace_id"],
        "activeProfileId": context["profile_id"],
    }


def _transaction_wallet_balances(
    conn: sqlite3.Connection,
    profile_id: str,
) -> dict[str, float]:
    rows = conn.execute(
        """
        SELECT
            wallet_id,
            SUM(
                CASE
                    WHEN direction = 'inbound' THEN amount
                    WHEN direction = 'outbound' THEN -amount - fee
                    ELSE 0
                END
            ) AS quantity
        FROM transactions
        WHERE profile_id = ? AND excluded = 0 AND asset = 'BTC'
        GROUP BY wallet_id
        """,
        (profile_id,),
    ).fetchall()
    return {
        row["wallet_id"]: float(msat_to_btc(row["quantity"] or 0))
        for row in rows
    }


def _wallet_balances(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    needs_journals: bool,
) -> dict[str, float]:
    if needs_journals:
        return _transaction_wallet_balances(conn, profile_id)

    rows = conn.execute(
        """
        SELECT wallet_id, SUM(quantity) AS quantity
        FROM journal_entries
        WHERE profile_id = ? AND asset = 'BTC'
        GROUP BY wallet_id
        """,
        (profile_id,),
    ).fetchall()
    if rows:
        return {
            row["wallet_id"]: float(msat_to_btc(row["quantity"] or 0))
            for row in rows
        }

    return _transaction_wallet_balances(conn, profile_id)


def _connections(
    conn: sqlite3.Connection,
    profile_id: str,
    balances: dict[str, float],
) -> list[dict[str, Any]]:
    rows = conn.execute(
        """
        SELECT
            w.id,
            w.label,
            w.kind,
            w.created_at,
            COUNT(t.id) AS tx_count,
            MAX(t.occurred_at) AS last_tx_at
        FROM wallets w
        LEFT JOIN transactions t ON t.wallet_id = w.id AND t.excluded = 0
        WHERE w.profile_id = ?
        GROUP BY w.id
        ORDER BY w.label ASC
        """,
        (profile_id,),
    ).fetchall()
    output = []
    for row in rows:
        tx_count = int(row["tx_count"] or 0)
        output.append(
            {
                "id": row["id"],
                "kind": _map_wallet_kind(row["kind"]),
                "label": row["label"],
                "last": _relative_last(row["last_tx_at"] or row["created_at"]),
                "balance": balances.get(row["id"], 0.0),
                "status": "synced" if tx_count else "idle",
            }
        )
    return output


def _transactions(conn: sqlite3.Connection, profile_id: str) -> list[dict[str, Any]]:
    rows = conn.execute(
        """
        SELECT
            t.id,
            t.external_id AS external_id,
            t.occurred_at,
            t.confirmed_at,
            w.label AS wallet,
            t.direction,
            t.amount,
            COALESCE(t.fiat_value, 0) AS fiat_value,
            COALESCE(t.fiat_rate, 0) AS fiat_rate,
            COALESCE(t.kind, '') AS kind,
            COALESCE(t.description, '') AS description,
            COALESCE(t.counterparty, '') AS counterparty,
            COALESCE(t.note, '') AS note,
            t.excluded,
            jq.reason AS quarantine_reason
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        LEFT JOIN journal_quarantines jq ON jq.transaction_id = t.id
        WHERE t.profile_id = ?
        ORDER BY t.occurred_at DESC, t.created_at DESC, t.id DESC
        LIMIT 20
        """,
        (profile_id,),
    ).fetchall()
    tags_by_transaction = {row["id"]: [] for row in rows}
    if rows:
        placeholders = ", ".join("?" for _ in rows)
        tag_rows = conn.execute(
            f"""
            SELECT tt.transaction_id, tags.label
            FROM transaction_tags tt
            JOIN tags ON tags.id = tt.tag_id
            WHERE tt.transaction_id IN ({placeholders})
            ORDER BY tt.transaction_id ASC, tags.code ASC
            """,
            [row["id"] for row in rows],
        ).fetchall()
        for tag in tag_rows:
            tags_by_transaction[tag["transaction_id"]].append(tag["label"])

    output = []
    for row in rows:
        sign = 1 if row["direction"] == "inbound" else -1
        tags = tags_by_transaction.get(row["id"], [])
        if not tags and row["quarantine_reason"]:
            tags = ["Review"]
        elif not tags:
            tags = [row["kind"] or row["direction"]]
        amount_btc = float(msat_to_btc(row["amount"] or 0))
        output.append(
            {
                "id": row["id"],
                "externalId": row["external_id"],
                "explorerId": _public_explorer_id(row["external_id"]),
                "date": (row["occurred_at"] or "")[:16].replace("T", " "),
                "type": _transaction_type(
                    row["kind"],
                    row["direction"],
                    row["quarantine_reason"],
                ),
                "account": row["wallet"],
                "counter": (
                    row["counterparty"]
                    or row["description"]
                    or row["note"]
                    or row["external_id"]
                    or row["id"]
                ),
                "amountSat": int(round(sign * amount_btc * 100_000_000)),
                "eur": sign * abs(float(row["fiat_value"] or 0)),
                "rate": float(row["fiat_rate"] or 0),
                "tag": ", ".join(str(tag) for tag in tags if tag) or "Unlabeled",
                "conf": 1 if row["confirmed_at"] else 0,
                "internal": (row["kind"] or "").lower() == "transfer",
            }
        )
    return output


def _transaction_type(kind: str, direction: str, quarantine_reason: str | None) -> str:
    if quarantine_reason:
        return "Fee" if quarantine_reason == "missing_fee_price" else "Transfer"
    normalized = (kind or "").lower()
    if "transfer" in normalized:
        return "Transfer"
    if "swap" in normalized:
        return "Swap"
    if "fee" in normalized:
        return "Fee"
    if direction == "inbound":
        return "Income"
    return "Expense"


def _balance_series(conn: sqlite3.Connection, profile_id: str) -> list[float]:
    rows = conn.execute(
        """
        SELECT occurred_at, direction, amount, fee
        FROM transactions
        WHERE profile_id = ? AND excluded = 0 AND asset = 'BTC'
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        (profile_id,),
    ).fetchall()
    if not rows:
        return [0.0] * 12
    by_month: dict[str, int] = defaultdict(int)
    for row in rows:
        key = (row["occurred_at"] or "")[:7]
        if not key:
            continue
        amount = int(row["amount"] or 0)
        fee = int(row["fee"] or 0)
        by_month[key] += amount if row["direction"] == "inbound" else -amount - fee
    months = sorted(by_month)[-12:]
    cumulative = 0
    series = []
    for month in sorted(by_month):
        cumulative += by_month[month]
        if month in months:
            series.append(float(msat_to_btc(cumulative)))
    if len(series) < 12:
        series = [series[0]] * (12 - len(series)) + series
    return series[-12:]


def _rate_from_transaction(row: sqlite3.Row) -> float | None:
    if row["fiat_rate"] is not None:
        return float(row["fiat_rate"])
    if row["fiat_value"] is not None and row["amount"]:
        amount_btc = float(msat_to_btc(row["amount"]))
        if amount_btc:
            return abs(float(row["fiat_value"])) / amount_btc
    return None


def _portfolio_cost_basis_by_date(
    conn: sqlite3.Connection,
    profile_id: str,
) -> dict[str, float]:
    rows = conn.execute(
        """
        SELECT occurred_at, quantity, fiat_value, COALESCE(cost_basis, 0) AS cost_basis
        FROM journal_entries
        WHERE profile_id = ? AND asset = 'BTC'
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        (profile_id,),
    ).fetchall()
    cost_basis = 0.0
    by_date: dict[str, float] = {}
    for row in rows:
        date_key = (row["occurred_at"] or "")[:10]
        if not date_key:
            continue
        quantity = int(row["quantity"] or 0)
        if quantity >= 0:
            cost_basis += float(row["fiat_value"] or 0)
        else:
            cost_basis -= float(row["cost_basis"] or 0)
        by_date[date_key] = cost_basis
    return by_date


def _portfolio_series(
    conn: sqlite3.Connection,
    profile_id: str,
    fallback_rate: float,
    final_balance_btc: float,
    final_value_eur: float,
) -> list[dict[str, Any]]:
    rows = conn.execute(
        """
        SELECT occurred_at, direction, amount, fee, fiat_rate, fiat_value
        FROM transactions
        WHERE profile_id = ? AND excluded = 0 AND asset = 'BTC'
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        (profile_id,),
    ).fetchall()
    if not rows:
        return []

    cost_basis_by_date = _portfolio_cost_basis_by_date(conn, profile_id)
    quantity_msat = 0
    latest_rate = fallback_rate
    output: list[dict[str, Any]] = []
    current_date = ""
    day_cost_basis = 0.0

    for row in rows:
        date_key = (row["occurred_at"] or "")[:10]
        if not date_key:
            continue
        if current_date and date_key != current_date:
            balance_btc = float(msat_to_btc(quantity_msat))
            output.append(
                {
                    "date": current_date,
                    "label": current_date,
                    "balanceBtc": balance_btc,
                    "valueEur": balance_btc * latest_rate,
                    "costBasisEur": day_cost_basis,
                }
            )

        current_date = date_key
        amount = int(row["amount"] or 0)
        fee = int(row["fee"] or 0)
        quantity_msat += amount if row["direction"] == "inbound" else -amount - fee
        row_rate = _rate_from_transaction(row)
        if row_rate is not None:
            latest_rate = row_rate
        day_cost_basis = cost_basis_by_date.get(date_key, day_cost_basis)

    if current_date:
        output.append(
            {
                "date": current_date,
                "label": current_date,
                "balanceBtc": final_balance_btc,
                "valueEur": final_value_eur,
                "costBasisEur": day_cost_basis,
            }
        )
    return output


def _fiat_snapshot(
    conn: sqlite3.Connection,
    profile_id: str,
    price_eur: float,
    balances: dict[str, float],
) -> dict[str, float]:
    market_value = sum(balances.values()) * price_eur
    cost_row = conn.execute(
        """
        SELECT SUM(COALESCE(cost_basis, 0)) AS cost_basis
        FROM journal_entries
        WHERE profile_id = ? AND entry_type IN ('acquisition', 'income', 'transfer_in')
        """,
        (profile_id,),
    ).fetchone()
    realized_row = conn.execute(
        """
        SELECT SUM(COALESCE(gain_loss, 0)) AS gain_loss
        FROM journal_entries
        WHERE profile_id = ?
          AND gain_loss IS NOT NULL
          AND occurred_at >= date('now', 'start of year')
        """,
        (profile_id,),
    ).fetchone()
    cost_basis = float(cost_row["cost_basis"] or 0)
    return {
        "eurBalance": float(market_value),
        "eurCostBasis": cost_basis,
        "eurUnrealized": float(market_value - cost_basis),
        "eurRealizedYTD": float(realized_row["gain_loss"] or 0),
    }


def build_overview_snapshot(conn: sqlite3.Connection) -> dict[str, Any]:
    context = current_context_snapshot(conn)
    if not context["workspace_id"] or not context["profile_id"]:
        return _empty_overview_snapshot()

    profile = conn.execute(
        "SELECT * FROM profiles WHERE id = ?",
        (context["profile_id"],),
    ).fetchone()
    if profile is None:
        return _empty_overview_snapshot()

    active_transactions = conn.execute(
        """
        SELECT COUNT(*) AS count
        FROM transactions
        WHERE profile_id = ? AND excluded = 0
        """,
        (profile["id"],),
    ).fetchone()["count"]
    needs_journals = (
        not profile["last_processed_at"]
        or int(profile["last_processed_tx_count"] or 0)
        != int(active_transactions or 0)
    )
    quarantines = conn.execute(
        "SELECT COUNT(*) AS count FROM journal_quarantines WHERE profile_id = ?",
        (profile["id"],),
    ).fetchone()["count"]

    price_eur = _latest_rate(conn, "BTC-EUR") or _latest_transaction_rate(
        conn,
        profile["id"],
        "EUR",
    )
    price_usd = _latest_rate(conn, "BTC-USD") or _latest_transaction_rate(
        conn,
        profile["id"],
        "USD",
    )
    balances = _wallet_balances(
        conn,
        profile["id"],
        needs_journals=needs_journals,
    )
    fiat = _fiat_snapshot(conn, profile["id"], price_eur, balances)
    snapshot = {
        "priceEur": price_eur,
        "priceUsd": price_usd,
        "connections": _connections(conn, profile["id"], balances),
        "txs": _transactions(conn, profile["id"]),
        "balanceSeries": _balance_series(conn, profile["id"]),
        "portfolioSeries": _portfolio_series(
            conn,
            profile["id"],
            price_eur,
            sum(balances.values()),
            fiat["eurBalance"],
        ),
        "fiat": fiat,
        "status": {
            "workspace": context["workspace_label"] or None,
            "profile": context["profile_label"] or None,
            "transactionCount": int(active_transactions or 0),
            "needsJournals": needs_journals,
            "quarantines": int(quarantines or 0),
        },
    }
    return snapshot


def build_transactions_snapshot(
    conn: sqlite3.Connection,
    args: dict[str, Any] | None = None,
) -> dict[str, Any]:
    context = current_context_snapshot(conn)
    if not context["workspace_id"] or not context["profile_id"]:
        return {"txs": [], "year": datetime.now(timezone.utc).year}

    raw_args = _coerce_args(args)
    unknown = sorted(
        set(raw_args)
        - {
            "limit",
            "direction",
            "asset",
            "wallet",
            "since",
            "sort",
            "order",
        }
    )
    if unknown:
        raise AppError(
            "ui.transactions.list received unsupported filters",
            code="validation",
            details={"unknown": unknown},
            retryable=False,
        )
    limit = _coerce_limit(raw_args, default=100, maximum=MAX_UI_LIST_LIMIT)
    filters = ["t.profile_id = ?"]
    params: list[Any] = [context["profile_id"]]
    direction = raw_args.get("direction")
    if direction is not None:
        if direction not in {"inbound", "outbound"}:
            raise AppError(
                "direction must be inbound or outbound",
                code="validation",
                details={"direction": direction},
                retryable=False,
            )
        filters.append("t.direction = ?")
        params.append(direction)
    asset = raw_args.get("asset")
    asset_filter = None
    if asset is not None:
        if not isinstance(asset, str) or not asset.strip():
            raise AppError("asset must be a non-empty string", code="validation")
        asset_filter = asset.strip().upper()
        filters.append("upper(t.asset) = ?")
        params.append(asset_filter)
    wallet = raw_args.get("wallet")
    wallet_filter = None
    if wallet is not None:
        if not isinstance(wallet, str) or not wallet.strip():
            raise AppError("wallet must be a non-empty string", code="validation")
        wallet_filter = wallet.strip()
        filters.append("(t.wallet_id = ? OR lower(w.label) = lower(?))")
        params.extend([wallet_filter, wallet_filter])
    since = raw_args.get("since")
    since_filter = None
    if since is not None:
        if not isinstance(since, str) or not since.strip():
            raise AppError("since must be an RFC3339 timestamp", code="validation")
        since_filter = _iso_z(_parse_iso_datetime(since, "since"))
        filters.append("t.occurred_at >= ?")
        params.append(since_filter)

    sort = raw_args.get("sort", "occurred-at")
    sort_columns = {
        "occurred-at": "t.occurred_at",
        "amount": "t.amount",
        "fiat-value": "COALESCE(t.fiat_value, 0)",
        "fee": "t.fee",
    }
    if sort not in sort_columns:
        raise AppError(
            "sort must be one of: occurred-at, amount, fiat-value, fee",
            code="validation",
            details={"sort": sort},
            retryable=False,
        )
    order = raw_args.get("order", "desc")
    if order not in {"asc", "desc"}:
        raise AppError(
            "order must be asc or desc",
            code="validation",
            details={"order": order},
            retryable=False,
        )
    order_sql = str(order).upper()
    if sort == "occurred-at":
        order_by = f"t.occurred_at {order_sql}, t.created_at {order_sql}, t.id {order_sql}"
    else:
        order_by = (
            f"{sort_columns[sort]} {order_sql}, "
            "t.occurred_at DESC, t.created_at DESC, t.id DESC"
        )
    params.append(limit)

    rows = conn.execute(
        f"""
        SELECT
            t.id,
            t.external_id AS external_id,
            t.occurred_at,
            t.confirmed_at,
            w.label AS wallet,
            t.direction,
            t.asset,
            t.amount,
            t.fee,
            COALESCE(t.fiat_value, 0) AS fiat_value,
            COALESCE(t.fiat_rate, 0) AS fiat_rate,
            COALESCE(t.kind, '') AS kind,
            COALESCE(t.description, '') AS description,
            COALESCE(t.counterparty, '') AS counterparty,
            COALESCE(t.note, '') AS note,
            jq.reason AS quarantine_reason
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        LEFT JOIN journal_quarantines jq ON jq.transaction_id = t.id
        WHERE {' AND '.join(filters)}
        ORDER BY {order_by}
        LIMIT ?
        """,
        params,
    ).fetchall()
    return {
        "txs": _transaction_rows_to_ui(conn, rows),
        "year": _snapshot_year(rows),
        "filters": {
            "limit": limit,
            "direction": direction,
            "asset": asset_filter,
            "wallet": wallet_filter,
            "since": since_filter,
            "sort": sort,
            "order": order,
        },
    }


def _snapshot_year(rows: list[sqlite3.Row]) -> int:
    for row in rows:
        occurred_at = row["occurred_at"] or ""
        if len(occurred_at) >= 4 and occurred_at[:4].isdigit():
            return int(occurred_at[:4])
    return datetime.now(timezone.utc).year


def build_capital_gains_snapshot(conn: sqlite3.Connection) -> dict[str, Any]:
    context = current_context_snapshot(conn)
    if not context["workspace_id"] or not context["profile_id"]:
        return {
            "jurisdictionCode": "AT",
            "year": datetime.now(timezone.utc).year,
            "method": "fifo",
            "lots": [],
            "status": {
                "needsJournals": False,
                "quarantines": 0,
            },
        }

    profile = conn.execute(
        "SELECT * FROM profiles WHERE id = ?",
        (context["profile_id"],),
    ).fetchone()
    if profile is None:
        return {
            "jurisdictionCode": "AT",
            "year": datetime.now(timezone.utc).year,
            "method": "fifo",
            "lots": [],
            "status": {
                "needsJournals": False,
                "quarantines": 0,
            },
        }

    active_transactions = conn.execute(
        """
        SELECT COUNT(*) AS count
        FROM transactions
        WHERE profile_id = ? AND excluded = 0
        """,
        (profile["id"],),
    ).fetchone()["count"]
    needs_journals = (
        not profile["last_processed_at"]
        or int(profile["last_processed_tx_count"] or 0)
        != int(active_transactions or 0)
    )
    quarantines = conn.execute(
        "SELECT COUNT(*) AS count FROM journal_quarantines WHERE profile_id = ?",
        (profile["id"],),
    ).fetchone()["count"]
    rows = conn.execute(
        """
        SELECT occurred_at, quantity, cost_basis, proceeds, gain_loss
        FROM journal_entries
        WHERE profile_id = ?
          AND entry_type IN ('disposal', 'fee', 'transfer_fee')
        ORDER BY occurred_at DESC, created_at DESC, id DESC
        LIMIT 200
        """,
        (profile["id"],),
    ).fetchall()
    latest_year = _snapshot_year(rows)
    lots = [
        {
            "acquired": "",
            "disposed": (row["occurred_at"] or "")[:10],
            "sats": int(round(abs(float(msat_to_btc(row["quantity"] or 0))) * 100_000_000)),
            "costEur": float(row["cost_basis"] or 0),
            "proceedsEur": float(row["proceeds"] or 0),
            "type": "ST",
        }
        for row in reversed(rows)
        if (row["occurred_at"] or "").startswith(str(latest_year))
    ]
    return {
        "jurisdictionCode": (profile["tax_country"] or "AT").upper(),
        "year": latest_year,
        "method": str(profile["gains_algorithm"] or "fifo").lower(),
        "lots": lots,
        "status": {
            "needsJournals": needs_journals,
            "quarantines": int(quarantines or 0),
        },
    }


def build_journals_snapshot(conn: sqlite3.Connection) -> dict[str, Any]:
    context = current_context_snapshot(conn)
    if not context["workspace_id"] or not context["profile_id"]:
        return {
            "status": {
                "workspace": None,
                "profile": None,
                "transactionCount": 0,
                "journalEntryCount": 0,
                "needsJournals": False,
                "quarantines": 0,
                "lastProcessedAt": None,
            },
            "entryTypes": [],
            "recent": [],
        }

    profile = conn.execute(
        "SELECT * FROM profiles WHERE id = ?",
        (context["profile_id"],),
    ).fetchone()
    if profile is None:
        return {
            "status": {
                "workspace": None,
                "profile": None,
                "transactionCount": 0,
                "journalEntryCount": 0,
                "needsJournals": False,
                "quarantines": 0,
                "lastProcessedAt": None,
            },
            "entryTypes": [],
            "recent": [],
        }

    active_transactions = conn.execute(
        """
        SELECT COUNT(*) AS count
        FROM transactions
        WHERE profile_id = ? AND excluded = 0
        """,
        (profile["id"],),
    ).fetchone()["count"]
    journal_entries = conn.execute(
        "SELECT COUNT(*) AS count FROM journal_entries WHERE profile_id = ?",
        (profile["id"],),
    ).fetchone()["count"]
    quarantines = conn.execute(
        "SELECT COUNT(*) AS count FROM journal_quarantines WHERE profile_id = ?",
        (profile["id"],),
    ).fetchone()["count"]
    needs_journals = (
        not profile["last_processed_at"]
        or int(profile["last_processed_tx_count"] or 0)
        != int(active_transactions or 0)
    )
    entry_rows = conn.execute(
        """
        SELECT entry_type, COUNT(*) AS count, SUM(COALESCE(gain_loss, 0)) AS gain_loss
        FROM journal_entries
        WHERE profile_id = ?
        GROUP BY entry_type
        ORDER BY count DESC, entry_type ASC
        """,
        (profile["id"],),
    ).fetchall()
    recent_rows = conn.execute(
        """
        SELECT je.occurred_at, je.entry_type, je.asset, je.quantity, je.fiat_value,
               COALESCE(je.gain_loss, 0) AS gain_loss, w.label AS wallet
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        WHERE je.profile_id = ?
        ORDER BY je.occurred_at DESC, je.created_at DESC, je.id DESC
        LIMIT 12
        """,
        (profile["id"],),
    ).fetchall()
    return {
        "status": {
            "workspace": context["workspace_label"] or None,
            "profile": context["profile_label"] or None,
            "transactionCount": int(active_transactions or 0),
            "journalEntryCount": int(journal_entries or 0),
            "needsJournals": needs_journals,
            "quarantines": int(quarantines or 0),
            "lastProcessedAt": profile["last_processed_at"],
        },
        "entryTypes": [
            {
                "type": row["entry_type"],
                "count": int(row["count"] or 0),
                "gainLossEur": float(row["gain_loss"] or 0),
            }
            for row in entry_rows
        ],
        "recent": [
            {
                "date": (row["occurred_at"] or "")[:16].replace("T", " "),
                "type": row["entry_type"],
                "wallet": row["wallet"],
                "asset": row["asset"],
                "quantity": float(msat_to_btc(row["quantity"] or 0)),
                "fiatValueEur": float(row["fiat_value"] or 0),
                "gainLossEur": float(row["gain_loss"] or 0),
            }
            for row in recent_rows
        ],
    }


def build_wallets_list_snapshot(
    conn: sqlite3.Connection,
    runtime_config: dict[str, object] | None = None,
) -> dict[str, Any]:
    context, profile = _active_context_and_profile(conn)
    if profile is None:
        return {
            "wallets": [],
            "summary": {
                "workspace": None,
                "profile": None,
                "count": 0,
                "transaction_count": 0,
                "needs_journals": False,
            },
        }

    freshness = _journal_freshness(conn, profile)
    rows = conn.execute(
        """
        SELECT
            w.id,
            w.label,
            w.kind,
            w.config_json,
            w.created_at,
            a.code AS account_code,
            a.label AS account_label,
            COUNT(t.id) AS tx_count,
            MAX(t.occurred_at) AS last_tx_at
        FROM wallets w
        LEFT JOIN accounts a ON a.id = w.account_id
        LEFT JOIN transactions t ON t.wallet_id = w.id AND t.excluded = 0
        WHERE w.profile_id = ?
        GROUP BY w.id
        ORDER BY w.label ASC
        """,
        (profile["id"],),
    ).fetchall()
    runtime_backends = (
        runtime_config.get("backends", {})
        if isinstance(runtime_config, dict)
        else {}
    )
    default_backend = (
        runtime_config.get("default_backend")
        if isinstance(runtime_config, dict)
        else None
    )
    wallets = []
    for row in rows:
        config = _json_config(row["config_json"])
        backend_summary = _wallet_backend_summary(row["kind"], config, default_backend)
        backend_name = backend_summary["name"]
        backend = (
            runtime_backends.get(str(backend_name))
            if isinstance(runtime_backends, dict) and backend_name
            else None
        )
        tx_count = int(row["tx_count"] or 0)
        wallets.append(
            {
                "id": row["id"],
                "label": row["label"],
                "kind": row["kind"],
                "account": {
                    "code": row["account_code"] or "",
                    "label": row["account_label"] or "",
                },
                "backend": {
                    "name": str(backend_name) if backend_name else "",
                    "source": backend_summary["source"],
                    "kind": str(backend.get("kind") or "") if isinstance(backend, dict) else "",
                },
                "chain": str(config.get("chain") or ""),
                "network": str(config.get("network") or ""),
                "sync_mode": backend_summary["sync_mode"],
                "sync_source": str(config.get("sync_source") or config.get("source_format") or ""),
                "transaction_count": tx_count,
                "last_transaction_at": row["last_tx_at"],
                "sync_status": "has_transactions" if tx_count else "empty",
                "journals_stale": freshness["needs_processing"] and tx_count > 0,
                "created_at": row["created_at"],
            }
        )

    return {
        "wallets": wallets,
        "summary": {
            "workspace": context["workspace_label"] or None,
            "profile": context["profile_label"] or None,
            "count": len(wallets),
            "transaction_count": sum(wallet["transaction_count"] for wallet in wallets),
            "needs_journals": bool(freshness["needs_processing"]),
        },
    }


def build_backends_list_snapshot(
    conn: sqlite3.Connection,
    runtime_config: dict[str, object],
) -> dict[str, Any]:
    runtime_backends = runtime_config.get("backends", {})
    default_backend = str(runtime_config.get("default_backend") or "")
    allowed_backend_fields = {
        "name",
        "kind",
        "chain",
        "network",
        "batch_size",
        "timeout",
        "insecure",
        "has_auth_header",
        "has_token",
        "has_cookiefile",
        "has_username",
        "has_password",
    }
    context, profile = _active_context_and_profile(conn)
    referenced_names: set[str] = set()
    if profile is not None:
        wallet_rows = conn.execute(
            """
            SELECT kind, config_json
            FROM wallets
            WHERE profile_id = ?
            ORDER BY label ASC
            """,
            (profile["id"],),
        ).fetchall()
        for row in wallet_rows:
            backend = _wallet_backend_summary(
                row["kind"],
                _json_config(row["config_json"]),
                default_backend,
            )
            if backend["name"]:
                referenced_names.add(backend["name"])

    rows = []
    if isinstance(runtime_backends, dict):
        for name, backend in sorted(runtime_backends.items()):
            if str(name) not in referenced_names:
                continue
            if not isinstance(backend, dict):
                continue
            safe = redact_backend_for_output(
                {
                    "name": name,
                    "kind": backend.get("kind", ""),
                    "chain": backend.get("chain", ""),
                    "network": backend.get("network", ""),
                    "url": backend.get("url", ""),
                    "batch_size": backend.get("batch_size", ""),
                    "source": backend.get("source", ""),
                    "auth_header": backend.get("auth_header", ""),
                    "token": backend.get("token", ""),
                    "cookiefile": backend.get("cookiefile", ""),
                    "username": backend.get("username", ""),
                    "password": backend.get("password", ""),
                    "config_json": backend.get("config_json", ""),
                }
            )
            safe = {
                key: value
                for key, value in safe.items()
                if key in allowed_backend_fields
            }
            safe["has_url"] = bool(_string_or_empty(backend.get("url")))
            safe["is_default"] = str(name) == default_backend
            rows.append(safe)
    return {
        "backends": rows,
        "summary": {
            "workspace": context["workspace_label"] or None,
            "profile": context["profile_label"] or None,
            "count": len(rows),
            "default_backend": default_backend if default_backend in referenced_names else None,
            "scope": "active_profile",
        },
    }


def build_journals_quarantine_snapshot(
    conn: sqlite3.Connection,
    args: dict[str, Any] | None = None,
) -> dict[str, Any]:
    raw_args = _coerce_args(args)
    unknown = sorted(set(raw_args) - {"limit"})
    if unknown:
        raise AppError(
            "ui.journals.quarantine received unsupported arguments",
            code="validation",
            details={"unknown": unknown},
            retryable=False,
        )
    limit = _coerce_limit(raw_args, default=20, maximum=MAX_UI_PREVIEW_LIMIT)
    context, profile = _active_context_and_profile(conn)
    if profile is None:
        return {
            "summary": {
                "workspace": None,
                "profile": None,
                "count": 0,
                "by_reason": [],
                "limit": limit,
            },
            "items": [],
        }

    reason_rows = conn.execute(
        """
        SELECT reason, COUNT(*) AS count
        FROM journal_quarantines
        WHERE profile_id = ?
        GROUP BY reason
        ORDER BY count DESC, reason ASC
        """,
        (profile["id"],),
    ).fetchall()
    rows = conn.execute(
        """
        SELECT
            q.transaction_id,
            q.reason,
            q.detail_json,
            q.created_at,
            t.external_id,
            t.occurred_at,
            t.confirmed_at,
            t.direction,
            t.asset,
            t.amount,
            t.fee,
            w.label AS wallet
        FROM journal_quarantines q
        JOIN transactions t ON t.id = q.transaction_id
        JOIN wallets w ON w.id = t.wallet_id
        WHERE q.profile_id = ?
        ORDER BY q.created_at DESC, t.occurred_at DESC, q.transaction_id DESC
        LIMIT ?
        """,
        (profile["id"], limit),
    ).fetchall()
    total = sum(int(row["count"] or 0) for row in reason_rows)
    return {
        "summary": {
            "workspace": context["workspace_label"] or None,
            "profile": context["profile_label"] or None,
            "count": total,
            "by_reason": [
                {"reason": row["reason"], "count": int(row["count"] or 0)}
                for row in reason_rows
            ],
            "limit": limit,
        },
        "items": [
            {
                "transaction_id": row["transaction_id"],
                "external_id": row["external_id"] or "",
                "occurred_at": row["occurred_at"],
                "confirmed_at": row["confirmed_at"],
                "wallet": row["wallet"],
                "direction": row["direction"],
                "asset": row["asset"],
                "amount": float(msat_to_btc(row["amount"] or 0)),
                "amount_msat": int(row["amount"] or 0),
                "fee": float(msat_to_btc(row["fee"] or 0)),
                "fee_msat": int(row["fee"] or 0),
                "reason": row["reason"],
                "detail": _json_config(row["detail_json"]),
                "created_at": row["created_at"],
            }
            for row in rows
        ],
    }


def build_journals_transfers_list_snapshot(
    conn: sqlite3.Connection,
    args: dict[str, Any] | None = None,
) -> dict[str, Any]:
    raw_args = _coerce_args(args)
    unknown = sorted(set(raw_args) - {"limit"})
    if unknown:
        raise AppError(
            "ui.journals.transfers.list received unsupported arguments",
            code="validation",
            details={"unknown": unknown},
            retryable=False,
        )
    limit = _coerce_limit(raw_args, default=20, maximum=MAX_UI_PREVIEW_LIMIT)
    context, profile = _active_context_and_profile(conn)
    if profile is None:
        return {
            "summary": {
                "workspace": None,
                "profile": None,
                "manual_pairs": 0,
                "same_asset_pairs": 0,
                "cross_asset_pairs": 0,
                "journal_transfer_entries": 0,
                "limit": limit,
            },
            "pairs": [],
        }

    summary = conn.execute(
        """
        SELECT
            COUNT(*) AS manual_pairs,
            SUM(CASE WHEN tout.asset = tin.asset THEN 1 ELSE 0 END) AS same_asset_pairs,
            SUM(CASE WHEN tout.asset <> tin.asset THEN 1 ELSE 0 END) AS cross_asset_pairs
        FROM transaction_pairs p
        JOIN transactions tout ON tout.id = p.out_transaction_id
        JOIN transactions tin ON tin.id = p.in_transaction_id
        WHERE p.profile_id = ?
        """,
        (profile["id"],),
    ).fetchone()
    journal_transfer_entries = conn.execute(
        """
        SELECT COUNT(*) AS count
        FROM journal_entries
        WHERE profile_id = ? AND entry_type IN ('transfer_out', 'transfer_in', 'transfer_fee')
        """,
        (profile["id"],),
    ).fetchone()["count"]
    rows = conn.execute(
        """
        SELECT
            p.id,
            p.kind,
            p.policy,
            p.created_at,
            p.out_transaction_id,
            p.in_transaction_id,
            tout.external_id AS out_external_id,
            tout.occurred_at AS out_occurred_at,
            tout.asset AS out_asset,
            tout.amount AS out_amount,
            wout.label AS out_wallet,
            tin.external_id AS in_external_id,
            tin.occurred_at AS in_occurred_at,
            tin.asset AS in_asset,
            tin.amount AS in_amount,
            win.label AS in_wallet
        FROM transaction_pairs p
        JOIN transactions tout ON tout.id = p.out_transaction_id
        JOIN transactions tin ON tin.id = p.in_transaction_id
        JOIN wallets wout ON wout.id = tout.wallet_id
        JOIN wallets win ON win.id = tin.wallet_id
        WHERE p.profile_id = ?
        ORDER BY p.created_at DESC
        LIMIT ?
        """,
        (profile["id"], limit),
    ).fetchall()
    return {
        "summary": {
            "workspace": context["workspace_label"] or None,
            "profile": context["profile_label"] or None,
            "manual_pairs": int(summary["manual_pairs"] or 0),
            "same_asset_pairs": int(summary["same_asset_pairs"] or 0),
            "cross_asset_pairs": int(summary["cross_asset_pairs"] or 0),
            "journal_transfer_entries": int(journal_transfer_entries or 0),
            "limit": limit,
        },
        "pairs": [
            {
                "id": row["id"],
                "kind": row["kind"],
                "policy": row["policy"],
                "created_at": row["created_at"],
                "out": {
                    "transaction_id": row["out_transaction_id"],
                    "external_id": row["out_external_id"] or "",
                    "occurred_at": row["out_occurred_at"],
                    "wallet": row["out_wallet"],
                    "asset": row["out_asset"],
                    "amount": float(msat_to_btc(row["out_amount"] or 0)),
                    "amount_msat": int(row["out_amount"] or 0),
                },
                "in": {
                    "transaction_id": row["in_transaction_id"],
                    "external_id": row["in_external_id"] or "",
                    "occurred_at": row["in_occurred_at"],
                    "wallet": row["in_wallet"],
                    "asset": row["in_asset"],
                    "amount": float(msat_to_btc(row["in_amount"] or 0)),
                    "amount_msat": int(row["in_amount"] or 0),
                },
            }
            for row in rows
        ],
    }


def build_rates_summary_snapshot(conn: sqlite3.Connection) -> dict[str, Any]:
    rows = conn.execute(
        """
        SELECT
            pair,
            COUNT(*) AS sample_count,
            MIN(timestamp) AS first_timestamp,
            MAX(timestamp) AS last_timestamp
        FROM rates_cache
        GROUP BY pair
        ORDER BY pair ASC
        """
    ).fetchall()
    latest_rows = conn.execute(
        """
        SELECT pair, timestamp, rate, source, fetched_at
        FROM (
            SELECT
                pair,
                timestamp,
                rate,
                source,
                fetched_at,
                ROW_NUMBER() OVER (
                    PARTITION BY pair
                    ORDER BY timestamp DESC,
                             CASE WHEN source = 'manual' THEN 0 ELSE 1 END ASC,
                             fetched_at DESC,
                             source ASC
                ) AS rn
            FROM rates_cache
        )
        WHERE rn = 1
        ORDER BY pair ASC
        """
    ).fetchall()
    latest_by_pair = {row["pair"]: dict(row) for row in latest_rows}
    pairs = []
    for row in rows:
        latest = latest_by_pair.get(row["pair"])
        pairs.append(
            {
                "pair": row["pair"],
                "sample_count": int(row["sample_count"] or 0),
                "first_timestamp": row["first_timestamp"],
                "last_timestamp": row["last_timestamp"],
                "latest": {
                    "timestamp": latest["timestamp"],
                    "rate": float(latest["rate"]),
                    "source": latest["source"],
                    "fetched_at": latest["fetched_at"],
                }
                if latest
                else None,
            }
        )
    return {"pairs": pairs, "summary": {"cached_pair_count": len(pairs)}}


def build_workspace_health_snapshot(conn: sqlite3.Connection) -> dict[str, Any]:
    context, profile = _active_context_and_profile(conn)
    if profile is None:
        return {
            "workspace": None,
            "profile": None,
            "counts": {
                "wallets": 0,
                "transactions": 0,
                "active_transactions": 0,
                "journal_entries": 0,
                "quarantines": 0,
                "rate_pairs": 0,
            },
            "journals": _journal_freshness(conn, None),
            "reports": {
                "ready": False,
                "hints": ["Create or select a workspace and profile first."],
            },
        }

    freshness = _journal_freshness(conn, profile)
    wallet_count = conn.execute(
        "SELECT COUNT(*) AS count FROM wallets WHERE profile_id = ?",
        (profile["id"],),
    ).fetchone()["count"]
    transaction_count = conn.execute(
        "SELECT COUNT(*) AS count FROM transactions WHERE profile_id = ?",
        (profile["id"],),
    ).fetchone()["count"]
    rate_pairs = conn.execute(
        "SELECT COUNT(DISTINCT pair) AS count FROM rates_cache",
    ).fetchone()["count"]
    hints: list[str] = []
    if int(wallet_count or 0) == 0:
        hints.append("Create a wallet before syncing or importing transactions.")
    elif int(transaction_count or 0) == 0:
        hints.append("Sync wallets or import wallet files before journal processing.")
    if freshness["needs_processing"]:
        hints.append("Run journal processing before trusting reports.")
    if freshness["quarantine_count"]:
        hints.append("Review quarantined transactions before tax export.")
    reports_ready = (
        int(wallet_count or 0) > 0
        and int(transaction_count or 0) > 0
        and freshness["status"] == "current"
        and freshness["quarantine_count"] == 0
    )
    if reports_ready:
        hints.append("Reports are ready from the current processed journal state.")
    return {
        "workspace": {
            "id": context["workspace_id"],
            "label": context["workspace_label"],
        },
        "profile": {
            "id": profile["id"],
            "label": profile["label"],
            "fiat_currency": profile["fiat_currency"],
            "tax_country": profile["tax_country"],
            "tax_long_term_days": int(profile["tax_long_term_days"] or 0),
            "gains_algorithm": profile["gains_algorithm"],
        },
        "counts": {
            "wallets": int(wallet_count or 0),
            "transactions": int(transaction_count or 0),
            "active_transactions": freshness["active_transaction_count"],
            "journal_entries": freshness["journal_entry_count"],
            "quarantines": freshness["quarantine_count"],
            "rate_pairs": int(rate_pairs or 0),
        },
        "journals": freshness,
        "reports": {
            "ready": reports_ready,
            "hints": hints,
        },
    }


def build_next_actions_snapshot(conn: sqlite3.Connection) -> dict[str, Any]:
    health = build_workspace_health_snapshot(conn)
    suggestions: list[dict[str, Any]] = []
    if health["profile"] is None:
        suggestions.append(
            {
                "id": "create_workspace_profile",
                "title": "Create a workspace and profile",
                "reason": "Kassiber needs an active accounting scope before wallets or reports.",
                "mutating": True,
                "requires_consent": True,
            }
        )
        return {"health": health, "suggestions": suggestions}

    counts = health["counts"]
    journals = health["journals"]
    if counts["wallets"] == 0:
        suggestions.append(
            {
                "id": "create_wallet",
                "title": "Create a wallet",
                "reason": "No wallets exist in the active profile.",
                "mutating": True,
                "requires_consent": True,
            }
        )
    elif counts["transactions"] == 0:
        suggestions.append(
            {
                "id": "sync_or_import",
                "title": "Sync wallets or import transactions",
                "reason": "Wallets exist but the active profile has no transactions yet.",
                "mutating": True,
                "requires_consent": True,
                "daemon_kind": "ui.wallets.sync",
            }
        )
    elif journals["needs_processing"]:
        suggestions.append(
            {
                "id": "process_journals",
                "title": "Process journals",
                "reason": journals["reason"],
                "mutating": True,
                "requires_consent": True,
                "daemon_kind": "ui.journals.process",
            }
        )
    if journals["quarantine_count"]:
        suggestions.append(
            {
                "id": "review_quarantine",
                "title": "Review quarantine",
                "reason": f"{journals['quarantine_count']} transaction(s) need review before reports are complete.",
                "mutating": False,
                "requires_consent": False,
                "daemon_kind": "ui.journals.quarantine",
            }
        )
    if (
        counts["transactions"] > 0
        and not journals["needs_processing"]
        and journals["quarantine_count"] == 0
    ):
        suggestions.append(
            {
                "id": "run_report",
                "title": "Run reports",
                "reason": "Journals are current and no quarantine is blocking report review.",
                "mutating": False,
                "requires_consent": False,
                "daemon_kind": "ui.reports.capital_gains",
            }
        )
    if not suggestions:
        suggestions.append(
            {
                "id": "inspect_workspace",
                "title": "Inspect workspace",
                "reason": "No urgent blocker was detected from the current safe snapshot.",
                "mutating": False,
                "requires_consent": False,
                "daemon_kind": "ui.workspace.health",
            }
        )
    return {"health": health, "suggestions": suggestions}


def _transaction_rows_to_ui(
    conn: sqlite3.Connection,
    rows: list[sqlite3.Row],
) -> list[dict[str, Any]]:
    tags_by_transaction = {row["id"]: [] for row in rows}
    if rows:
        placeholders = ", ".join("?" for _ in rows)
        tag_rows = conn.execute(
            f"""
            SELECT tt.transaction_id, tags.label
            FROM transaction_tags tt
            JOIN tags ON tags.id = tt.tag_id
            WHERE tt.transaction_id IN ({placeholders})
            ORDER BY tt.transaction_id ASC, tags.code ASC
            """,
            [row["id"] for row in rows],
        ).fetchall()
        for tag in tag_rows:
            tags_by_transaction[tag["transaction_id"]].append(tag["label"])

    output = []
    for row in rows:
        sign = 1 if row["direction"] == "inbound" else -1
        tags = tags_by_transaction.get(row["id"], [])
        if not tags and row["quarantine_reason"]:
            tags = ["Review"]
        elif not tags:
            tags = [row["kind"] or row["direction"]]
        amount_btc = float(msat_to_btc(row["amount"] or 0))
        output.append(
            {
                "id": row["id"],
                "externalId": row["external_id"],
                "explorerId": _public_explorer_id(row["external_id"]),
                "date": (row["occurred_at"] or "")[:16].replace("T", " "),
                "type": _transaction_type(
                    row["kind"],
                    row["direction"],
                    row["quarantine_reason"],
                ),
                "account": row["wallet"],
                "counter": (
                    row["counterparty"]
                    or row["description"]
                    or row["note"]
                    or row["external_id"]
                    or row["id"]
                ),
                "amountSat": int(round(sign * amount_btc * 100_000_000)),
                "eur": sign * abs(float(row["fiat_value"] or 0)),
                "rate": float(row["fiat_rate"] or 0),
                "tag": ", ".join(str(tag) for tag in tags if tag) or "Unlabeled",
                "conf": 1 if row["confirmed_at"] else 0,
                "internal": (row["kind"] or "").lower() == "transfer",
            }
        )
    return output
