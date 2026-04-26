from __future__ import annotations

import sqlite3
from collections import defaultdict
from datetime import datetime, timezone
from typing import Any

from ..msat import msat_to_btc
from .repo import current_context_snapshot


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
        "activeProfileId": context["profile_id"] or (
            profile_rows[0]["id"] if profile_rows else ""
        ),
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
    needs_journals: bool,
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
                "status": (
                    "syncing"
                    if needs_journals and tx_count
                    else ("synced" if tx_count else "idle")
                ),
            }
        )
    return output


def _transactions(conn: sqlite3.Connection, profile_id: str) -> list[dict[str, Any]]:
    rows = conn.execute(
        """
        SELECT
            t.id,
            COALESCE(t.external_id, t.id) AS external_id,
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
        "connections": _connections(conn, profile["id"], balances, needs_journals),
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

    raw_args = args or {}
    limit = raw_args.get("limit", 100)
    try:
        limit = max(1, min(500, int(limit)))
    except (TypeError, ValueError):
        limit = 100

    rows = conn.execute(
        """
        SELECT
            t.id,
            COALESCE(t.external_id, t.id) AS external_id,
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
            jq.reason AS quarantine_reason
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        LEFT JOIN journal_quarantines jq ON jq.transaction_id = t.id
        WHERE t.profile_id = ?
        ORDER BY t.occurred_at DESC, t.created_at DESC, t.id DESC
        LIMIT ?
        """,
        (context["profile_id"], limit),
    ).fetchall()
    return {
        "txs": _transaction_rows_to_ui(conn, rows),
        "year": _snapshot_year(rows),
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
