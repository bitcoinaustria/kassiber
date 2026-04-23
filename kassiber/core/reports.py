from __future__ import annotations

import sqlite3
from collections import defaultdict
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from decimal import Decimal
from typing import Any, Callable, Mapping, Sequence

from ..errors import AppError
from ..msat import btc_to_msat, dec, msat_to_btc
from ..tax_policy import require_tax_processing_supported

INTERVAL_CHOICES = ("hour", "day", "week", "month")

ScopeResolver = Callable[[sqlite3.Connection, str | None, str | None], tuple[Mapping[str, Any], Mapping[str, Any]]]
AccountResolver = Callable[[sqlite3.Connection, str, str], Mapping[str, Any]]
WalletResolver = Callable[[sqlite3.Connection, str, str], Mapping[str, Any]]
RequireProcessedJournals = Callable[[sqlite3.Connection, Mapping[str, Any]], None]
BuildLedgerState = Callable[[sqlite3.Connection, Mapping[str, Any]], Mapping[str, Any]]
ListJournalEntries = Callable[..., list[Mapping[str, Any]]]
ListWallets = Callable[..., list[Mapping[str, Any]]]
ParseIsoDateTime = Callable[[str | None, str], Any]
IsoFormatter = Callable[[Any], str]
NowIso = Callable[[], str]
FormatTable = Callable[..., list[str]]
WriteTextPdf = Callable[[str, str, Sequence[str]], Mapping[str, Any]]


@dataclass(frozen=True)
class ReportHooks:
    resolve_scope: ScopeResolver
    resolve_account: AccountResolver
    resolve_wallet: WalletResolver
    require_processed_journals: RequireProcessedJournals
    build_ledger_state: BuildLedgerState
    list_journal_entries: ListJournalEntries
    list_wallets: ListWallets
    parse_iso_datetime: ParseIsoDateTime
    iso_z: IsoFormatter
    now_iso: NowIso
    format_table: FormatTable
    write_text_pdf: WriteTextPdf


def _resolve_report_scope(conn, workspace_ref, profile_ref, hooks: ReportHooks):
    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    require_tax_processing_supported(profile)
    return workspace, profile


def report_balance_sheet(conn, workspace_ref, profile_ref, hooks: ReportHooks):
    _, profile = _resolve_report_scope(conn, workspace_ref, profile_ref, hooks)
    hooks.require_processed_journals(conn, profile)
    state = hooks.build_ledger_state(conn, profile)
    rows = []
    for (account_id, account_code, account_label, asset), value in sorted(
        state["account_holdings"].items(),
        key=lambda item: (item[0][1], item[0][3]),
    ):
        quantity = value["quantity"]
        if quantity <= 0:
            continue
        cost_basis = value["cost_basis"]
        latest_rate = state["latest_rates"].get(asset, Decimal("0"))
        market_value = quantity * latest_rate
        rows.append(
            {
                "account": account_code or account_label,
                "asset": asset,
                "quantity": float(quantity),
                "cost_basis": float(cost_basis),
                "market_value": float(market_value),
                "unrealized_pnl": float(market_value - cost_basis),
            }
        )
    return rows


def report_portfolio_summary(conn, workspace_ref, profile_ref, hooks: ReportHooks):
    _, profile = _resolve_report_scope(conn, workspace_ref, profile_ref, hooks)
    hooks.require_processed_journals(conn, profile)
    state = hooks.build_ledger_state(conn, profile)
    rows = []
    for (wallet_id, wallet_label, account_code, asset), value in sorted(
        state["wallet_holdings"].items(),
        key=lambda item: (item[0][1], item[0][3]),
    ):
        quantity = value["quantity"]
        if quantity <= 0:
            continue
        cost_basis = value["cost_basis"]
        latest_rate = state["latest_rates"].get(asset, Decimal("0"))
        market_value = quantity * latest_rate
        avg_cost = cost_basis / quantity if quantity else Decimal("0")
        rows.append(
            {
                "wallet": wallet_label,
                "account": account_code,
                "asset": asset,
                "quantity": float(quantity),
                "avg_cost": float(avg_cost),
                "cost_basis": float(cost_basis),
                "market_value": float(market_value),
                "unrealized_pnl": float(market_value - cost_basis),
            }
        )
    return rows


def report_capital_gains(conn, workspace_ref, profile_ref, hooks: ReportHooks):
    _, profile = _resolve_report_scope(conn, workspace_ref, profile_ref, hooks)
    hooks.require_processed_journals(conn, profile)
    rows = conn.execute(
        """
        SELECT
            je.occurred_at,
            w.label AS wallet,
            je.transaction_id,
            je.entry_type,
            je.asset,
            ABS(je.quantity) AS quantity,
            COALESCE(je.proceeds, 0) AS proceeds,
            COALESCE(je.cost_basis, 0) AS cost_basis,
            COALESCE(je.gain_loss, 0) AS gain_loss,
            COALESCE(je.description, '') AS description,
            je.at_category,
            je.at_kennzahl
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        WHERE je.profile_id = ? AND je.entry_type IN ('disposal', 'fee', 'transfer_fee', 'income')
        ORDER BY je.occurred_at ASC, je.created_at ASC, je.id ASC
        """,
        (profile["id"],),
    ).fetchall()
    results = []
    for row in rows:
        entry = dict(row)
        entry["quantity_msat"] = int(entry["quantity"])
        entry["quantity"] = float(msat_to_btc(entry["quantity"]))
        if entry.get("at_category") is None:
            entry.pop("at_category", None)
        if entry.get("at_kennzahl") is None:
            entry.pop("at_kennzahl", None)
        results.append(entry)
    return results


def report_journal_entries(conn, workspace_ref, profile_ref, hooks: ReportHooks):
    _, profile = _resolve_report_scope(conn, workspace_ref, profile_ref, hooks)
    hooks.require_processed_journals(conn, profile)
    return hooks.list_journal_entries(conn, profile["workspace_id"], profile["id"], limit=1000)


def _floor_to_interval(dt, interval):
    if interval == "hour":
        return dt.replace(minute=0, second=0, microsecond=0)
    if interval == "day":
        return dt.replace(hour=0, minute=0, second=0, microsecond=0)
    if interval == "week":
        floored = dt.replace(hour=0, minute=0, second=0, microsecond=0)
        return floored - timedelta(days=floored.weekday())
    if interval == "month":
        return dt.replace(day=1, hour=0, minute=0, second=0, microsecond=0)
    raise AppError(f"Unknown interval '{interval}'", code="validation")


def _next_interval(dt, interval):
    if interval == "hour":
        return dt + timedelta(hours=1)
    if interval == "day":
        return dt + timedelta(days=1)
    if interval == "week":
        return dt + timedelta(days=7)
    if interval == "month":
        if dt.month == 12:
            return dt.replace(year=dt.year + 1, month=1)
        return dt.replace(month=dt.month + 1)
    raise AppError(f"Unknown interval '{interval}'", code="validation")


def report_balance_history(
    conn,
    workspace_ref,
    profile_ref,
    hooks: ReportHooks,
    interval="day",
    start=None,
    end=None,
    wallet_ref=None,
    account_ref=None,
    asset=None,
):
    if interval not in INTERVAL_CHOICES:
        raise AppError(
            f"Unsupported interval '{interval}'",
            code="validation",
            hint=f"Choose one of: {', '.join(INTERVAL_CHOICES)}",
        )
    _, profile = _resolve_report_scope(conn, workspace_ref, profile_ref, hooks)
    hooks.require_processed_journals(conn, profile)
    start_dt = hooks.parse_iso_datetime(start, "start")
    end_dt = hooks.parse_iso_datetime(end, "end")
    if start_dt and end_dt and start_dt > end_dt:
        raise AppError("--start must not be after --end", code="validation")

    sql = """
        SELECT
            je.occurred_at,
            je.asset,
            je.quantity,
            je.fiat_value,
            COALESCE(je.cost_basis, 0) AS cost_basis
        FROM journal_entries je
        LEFT JOIN accounts a ON a.id = je.account_id
        WHERE je.profile_id = ?
    """
    params = [profile["id"]]
    if wallet_ref:
        wallet = hooks.resolve_wallet(conn, profile["id"], wallet_ref)
        sql += " AND je.wallet_id = ?"
        params.append(wallet["id"])
    if account_ref:
        account = hooks.resolve_account(conn, profile["id"], account_ref)
        sql += " AND je.account_id = ?"
        params.append(account["id"])
    if asset:
        sql += " AND je.asset = ?"
        params.append(asset)
    sql += " ORDER BY je.occurred_at ASC, je.created_at ASC, je.id ASC"
    rows = conn.execute(sql, params).fetchall()
    rate_rows = conn.execute(
        """
        SELECT occurred_at, asset, amount, fiat_rate, fiat_value
        FROM transactions
        WHERE profile_id = ? AND excluded = 0
          AND (fiat_rate IS NOT NULL OR fiat_value IS NOT NULL)
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        (profile["id"],),
    ).fetchall()

    if not rows and not (start_dt and end_dt):
        return []

    events = []
    for row in rows:
        row_dt = hooks.parse_iso_datetime(row["occurred_at"], "occurred_at")
        events.append(
            (
                row_dt,
                row["asset"],
                msat_to_btc(row["quantity"]),
                dec(row["fiat_value"]),
                dec(row["cost_basis"]),
            )
        )
    rate_events = []
    for row in rate_rows:
        rate = None
        if row["fiat_rate"] is not None:
            rate = dec(row["fiat_rate"])
        elif row["fiat_value"] is not None and row["amount"]:
            rate = dec(row["fiat_value"]) / msat_to_btc(row["amount"])
        if rate is None:
            continue
        rate_events.append((hooks.parse_iso_datetime(row["occurred_at"], "occurred_at"), row["asset"], rate))

    first_event_dt = events[0][0] if events else None
    range_start = start_dt or first_event_dt or datetime.now(timezone.utc)
    range_end = end_dt or datetime.now(timezone.utc)
    if range_start > range_end:
        return []

    cumulative = defaultdict(lambda: Decimal("0"))
    cumulative_fiat = defaultdict(lambda: Decimal("0"))
    event_idx = 0
    rate_idx = 0
    current_rates = {}
    bucket_start = _floor_to_interval(range_start, interval)
    end_cap = _floor_to_interval(range_end, interval)

    results = []
    while bucket_start <= end_cap:
        bucket_end = _next_interval(bucket_start, interval)
        while event_idx < len(events) and events[event_idx][0] < bucket_end:
            _, ev_asset, ev_qty, ev_fiat, ev_cost_basis = events[event_idx]
            cumulative[ev_asset] += ev_qty
            if ev_qty >= 0:
                cumulative_fiat[ev_asset] += ev_fiat
            else:
                cumulative_fiat[ev_asset] -= ev_cost_basis
            event_idx += 1
        while rate_idx < len(rate_events) and rate_events[rate_idx][0] < bucket_end:
            _, rate_asset, rate = rate_events[rate_idx]
            current_rates[rate_asset] = rate
            rate_idx += 1
        emitted_assets = set(cumulative.keys()) if asset is None else {asset}
        for ev_asset in sorted(emitted_assets):
            qty = cumulative.get(ev_asset, Decimal("0"))
            if qty == 0 and asset is None:
                continue
            rate = current_rates.get(ev_asset, Decimal("0"))
            results.append(
                {
                    "period_start": hooks.iso_z(bucket_start),
                    "period_end": hooks.iso_z(bucket_end - timedelta(seconds=1)),
                    "asset": ev_asset,
                    "quantity": float(qty),
                    "cumulative_cost_basis": float(cumulative_fiat.get(ev_asset, Decimal("0"))),
                    "market_value": float(qty * rate),
                }
            )
        bucket_start = bucket_end
    return results


def _report_kv_lines(pairs, label_width=28):
    return [f"{label + ':':<{label_width}} {value}" for label, value in pairs]


def _report_btc(value):
    return f"{float(value):,.8f}"


def _report_fiat(value):
    return f"{float(value):,.2f}"


def _report_count(value):
    return f"{int(value or 0):,}"


def _aggregate_balance_rows_from_portfolio(portfolio_rows):
    grouped = {}
    for row in portfolio_rows:
        key = (row["account"], row["asset"])
        bucket = grouped.setdefault(
            key,
            {
                "account": row["account"],
                "asset": row["asset"],
                "quantity": 0.0,
                "cost_basis": 0.0,
                "market_value": 0.0,
                "unrealized_pnl": 0.0,
            },
        )
        bucket["quantity"] += float(row["quantity"])
        bucket["cost_basis"] += float(row["cost_basis"])
        bucket["market_value"] += float(row["market_value"])
        bucket["unrealized_pnl"] += float(row["unrealized_pnl"])
    return [
        grouped[key]
        for key in sorted(grouped.keys(), key=lambda item: (item[0] or "", item[1] or ""))
    ]


def _scope_wallets(conn, workspace_ref, profile_ref, hooks: ReportHooks, wallet=None):
    wallets = hooks.list_wallets(conn, workspace_ref, profile_ref)
    if wallet is None:
        return wallets
    return [row for row in wallets if row["id"] == wallet["id"]]


def _report_query_rows(conn, profile, wallet=None):
    tx_filters = ["t.profile_id = ?"]
    tx_params = [profile["id"]]
    if wallet:
        tx_filters.append("t.wallet_id = ?")
        tx_params.append(wallet["id"])
    tx_where = " AND ".join(tx_filters)

    summary = conn.execute(
        f"""
        SELECT
            COUNT(*) AS total_transactions,
            SUM(CASE WHEN t.excluded = 0 THEN 1 ELSE 0 END) AS active_transactions,
            SUM(CASE WHEN t.excluded = 1 THEN 1 ELSE 0 END) AS excluded_transactions,
            SUM(CASE WHEN t.excluded = 0 AND t.direction = 'inbound' THEN 1 ELSE 0 END) AS inbound_transactions,
            SUM(CASE WHEN t.excluded = 0 AND t.direction = 'outbound' THEN 1 ELSE 0 END) AS outbound_transactions,
            COUNT(DISTINCT CASE WHEN t.excluded = 0 THEN t.asset END) AS asset_count,
            MIN(CASE WHEN t.excluded = 0 THEN t.occurred_at END) AS first_transaction_at,
            MAX(CASE WHEN t.excluded = 0 THEN t.occurred_at END) AS last_transaction_at,
            SUM(CASE WHEN t.excluded = 0 AND (t.fiat_rate IS NOT NULL OR t.fiat_value IS NOT NULL) THEN 1 ELSE 0 END) AS priced_transactions,
            SUM(CASE WHEN t.excluded = 0 AND COALESCE(TRIM(t.note), '') != '' THEN 1 ELSE 0 END) AS noted_transactions
        FROM transactions t
        WHERE {tx_where}
        """,
        tx_params,
    ).fetchone()

    tagged_transactions = conn.execute(
        f"""
        SELECT COUNT(DISTINCT tt.transaction_id) AS count
        FROM transaction_tags tt
        JOIN transactions t ON t.id = tt.transaction_id
        WHERE {tx_where} AND t.excluded = 0
        """,
        tx_params,
    ).fetchone()["count"]

    journal_filters = ["je.profile_id = ?"]
    journal_params = [profile["id"]]
    if wallet:
        journal_filters.append("je.wallet_id = ?")
        journal_params.append(wallet["id"])
    journal_where = " AND ".join(journal_filters)
    journal_entries = conn.execute(
        f"SELECT COUNT(*) AS count FROM journal_entries je WHERE {journal_where}",
        journal_params,
    ).fetchone()["count"]

    quarantines = conn.execute(
        f"""
        SELECT COUNT(*) AS count
        FROM journal_quarantines jq
        JOIN transactions t ON t.id = jq.transaction_id
        WHERE {tx_where}
        """,
        tx_params,
    ).fetchone()["count"]

    flow_by_asset = conn.execute(
        f"""
        SELECT
            t.asset,
            COUNT(*) AS tx_count,
            SUM(CASE WHEN t.direction = 'inbound' THEN 1 ELSE 0 END) AS inbound_count,
            SUM(CASE WHEN t.direction = 'outbound' THEN 1 ELSE 0 END) AS outbound_count,
            SUM(CASE WHEN t.direction = 'inbound' THEN t.amount ELSE 0 END) AS inbound_amount,
            SUM(CASE WHEN t.direction = 'outbound' THEN t.amount ELSE 0 END) AS outbound_amount,
            SUM(t.fee) AS fee_amount
        FROM transactions t
        WHERE {tx_where} AND t.excluded = 0
        GROUP BY t.asset
        ORDER BY t.asset ASC
        """,
        tx_params,
    ).fetchall()

    flow_by_wallet = conn.execute(
        f"""
        SELECT
            w.label AS wallet,
            t.asset,
            COUNT(*) AS tx_count,
            SUM(CASE WHEN t.direction = 'inbound' THEN 1 ELSE 0 END) AS inbound_count,
            SUM(CASE WHEN t.direction = 'outbound' THEN 1 ELSE 0 END) AS outbound_count,
            SUM(CASE WHEN t.direction = 'inbound' THEN t.amount ELSE 0 END) AS inbound_amount,
            SUM(CASE WHEN t.direction = 'outbound' THEN t.amount ELSE 0 END) AS outbound_amount,
            SUM(t.fee) AS fee_amount,
            MIN(t.occurred_at) AS first_at,
            MAX(t.occurred_at) AS last_at
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        WHERE {tx_where} AND t.excluded = 0
        GROUP BY w.label, t.asset
        ORDER BY w.label ASC, t.asset ASC
        """,
        tx_params,
    ).fetchall()

    quarantine_rows = conn.execute(
        f"""
        SELECT jq.reason, COUNT(*) AS count
        FROM journal_quarantines jq
        JOIN transactions t ON t.id = jq.transaction_id
        WHERE {tx_where}
        GROUP BY jq.reason
        ORDER BY count DESC, jq.reason ASC
        """,
        tx_params,
    ).fetchall()

    transactions = conn.execute(
        f"""
        SELECT
            t.occurred_at,
            w.label AS wallet,
            t.direction,
            t.asset,
            t.amount,
            t.fee,
            COALESCE(t.description, '') AS description
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        WHERE {tx_where} AND t.excluded = 0
        ORDER BY t.occurred_at ASC, t.created_at ASC, t.id ASC
        """,
        tx_params,
    ).fetchall()

    return {
        "summary": summary,
        "tagged_transactions": tagged_transactions,
        "journal_entries": journal_entries,
        "quarantines": quarantines,
        "flow_by_asset": flow_by_asset,
        "flow_by_wallet": flow_by_wallet,
        "quarantine_rows": quarantine_rows,
        "transactions": transactions,
    }


def _summary_rollups(balance_rows, capital_rows):
    return {
        "holdings": {
            "cost_basis": float(sum(float(row["cost_basis"]) for row in balance_rows)),
            "market_value": float(sum(float(row["market_value"]) for row in balance_rows)),
            "unrealized_pnl": float(sum(float(row["unrealized_pnl"]) for row in balance_rows)),
        },
        "realized": {
            "proceeds": float(sum(float(row["proceeds"]) for row in capital_rows)),
            "cost_basis": float(sum(float(row["cost_basis"]) for row in capital_rows)),
            "gain_loss": float(sum(float(row["gain_loss"]) for row in capital_rows)),
        },
    }


def _summary_flow_rows(rows):
    return [
        {
            "asset": row["asset"],
            "tx_count": int(row["tx_count"] or 0),
            "inbound_count": int(row["inbound_count"] or 0),
            "outbound_count": int(row["outbound_count"] or 0),
            "inbound_amount": float(msat_to_btc(row["inbound_amount"] or 0)),
            "inbound_amount_msat": int(row["inbound_amount"] or 0),
            "outbound_amount": float(msat_to_btc(row["outbound_amount"] or 0)),
            "outbound_amount_msat": int(row["outbound_amount"] or 0),
            "fee_amount": float(msat_to_btc(row["fee_amount"] or 0)),
            "fee_amount_msat": int(row["fee_amount"] or 0),
        }
        for row in rows
    ]


def _build_summary_context(conn, workspace_ref, profile_ref, hooks: ReportHooks, wallet_ref=None):
    workspace, profile = _resolve_report_scope(conn, workspace_ref, profile_ref, hooks)
    wallet = hooks.resolve_wallet(conn, profile["id"], wallet_ref) if wallet_ref else None
    hooks.require_processed_journals(conn, profile)

    scope_wallets = _scope_wallets(conn, workspace["id"], profile["id"], hooks, wallet=wallet)
    portfolio_rows = report_portfolio_summary(conn, workspace["id"], profile["id"], hooks)
    if wallet:
        portfolio_rows = [row for row in portfolio_rows if row["wallet"] == wallet["label"]]
    balance_rows = _aggregate_balance_rows_from_portfolio(portfolio_rows)

    capital_rows = report_capital_gains(conn, workspace["id"], profile["id"], hooks)
    if wallet:
        capital_rows = [row for row in capital_rows if row["wallet"] == wallet["label"]]

    query_rows = _report_query_rows(conn, profile, wallet=wallet)
    summary = query_rows["summary"]
    rollups = _summary_rollups(balance_rows, capital_rows)

    return {
        "workspace": workspace,
        "profile": profile,
        "wallet": wallet,
        "scope_wallets": scope_wallets,
        "portfolio_rows": portfolio_rows,
        "balance_rows": balance_rows,
        "capital_rows": capital_rows,
        "query_rows": query_rows,
        "summary": summary,
        "rollups": rollups,
    }


def report_summary(conn, workspace_ref, profile_ref, hooks: ReportHooks, wallet_ref=None):
    context = _build_summary_context(conn, workspace_ref, profile_ref, hooks, wallet_ref=wallet_ref)
    workspace = context["workspace"]
    profile = context["profile"]
    wallet = context["wallet"]
    scope_wallets = context["scope_wallets"]
    query_rows = context["query_rows"]
    summary = context["summary"]
    rollups = context["rollups"]

    return {
        "workspace": workspace["label"],
        "profile": profile["label"],
        "wallet": wallet["label"] if wallet else None,
        "fiat_currency": profile["fiat_currency"],
        "tax_country": profile["tax_country"],
        "tax_long_term_days": int(profile["tax_long_term_days"] or 0),
        "gains_algorithm": profile["gains_algorithm"],
        "last_processed_at": profile["last_processed_at"],
        "processed_tx_count": int(profile["last_processed_tx_count"] or 0),
        "metrics": {
            "wallets_in_scope": len(scope_wallets),
            "assets_in_scope": int(summary["asset_count"] or 0),
            "active_transactions": int(summary["active_transactions"] or 0),
            "excluded_transactions": int(summary["excluded_transactions"] or 0),
            "inbound_transactions": int(summary["inbound_transactions"] or 0),
            "outbound_transactions": int(summary["outbound_transactions"] or 0),
            "journal_entries": int(query_rows["journal_entries"] or 0),
            "quarantines": int(query_rows["quarantines"] or 0),
            "priced_transactions": int(summary["priced_transactions"] or 0),
            "transactions_with_notes": int(summary["noted_transactions"] or 0),
            "transactions_with_tags": int(query_rows["tagged_transactions"] or 0),
            "first_transaction_at": summary["first_transaction_at"],
            "last_transaction_at": summary["last_transaction_at"],
        },
        "holdings": rollups["holdings"],
        "realized": rollups["realized"],
        "asset_flow": _summary_flow_rows(query_rows["flow_by_asset"]),
    }


def build_summary_report_lines(conn, workspace_ref, profile_ref, hooks: ReportHooks, wallet_ref=None):
    context = _build_summary_context(conn, workspace_ref, profile_ref, hooks, wallet_ref=wallet_ref)
    workspace = context["workspace"]
    profile = context["profile"]
    wallet = context["wallet"]
    scope_wallets = context["scope_wallets"]
    query_rows = context["query_rows"]
    summary = context["summary"]
    rollups = context["rollups"]

    title_scope = wallet["label"] if wallet else profile["label"]
    title = f"Kassiber Summary Report - {title_scope}"
    lines = [title, "=" * len(title), ""]
    lines.extend(
        _report_kv_lines(
            [
                ("Workspace", workspace["label"]),
                ("Profile", profile["label"]),
                ("Wallet scope", wallet["label"] if wallet else "All wallets"),
                ("Fiat currency", profile["fiat_currency"]),
                ("Tax country", profile["tax_country"]),
                ("Tax long-term days", profile["tax_long_term_days"]),
                ("Gains algorithm", profile["gains_algorithm"]),
                ("Last processed at", profile["last_processed_at"] or ""),
                ("Processed tx count", _report_count(profile["last_processed_tx_count"])),
            ]
        )
    )

    lines.extend(["", "Activity", "--------"])
    lines.extend(
        _report_kv_lines(
            [
                ("Wallets in scope", _report_count(len(scope_wallets))),
                ("Assets in scope", _report_count(summary["asset_count"])),
                ("Transactions (active)", _report_count(summary["active_transactions"])),
                ("Transactions (excluded)", _report_count(summary["excluded_transactions"])),
                ("Inbound transactions", _report_count(summary["inbound_transactions"])),
                ("Outbound transactions", _report_count(summary["outbound_transactions"])),
                ("Journal entries", _report_count(query_rows["journal_entries"])),
                ("Quarantines", _report_count(query_rows["quarantines"])),
                ("Priced transactions", _report_count(summary["priced_transactions"])),
                ("Transactions with notes", _report_count(summary["noted_transactions"])),
                ("Transactions with tags", _report_count(query_rows["tagged_transactions"])),
                ("First transaction", summary["first_transaction_at"] or ""),
                ("Last transaction", summary["last_transaction_at"] or ""),
            ]
        )
    )

    lines.extend(["", "Financial Summary", "-----------------"])
    lines.extend(
        _report_kv_lines(
            [
                ("Holdings cost basis", _report_fiat(rollups["holdings"]["cost_basis"])),
                ("Holdings market value", _report_fiat(rollups["holdings"]["market_value"])),
                ("Unrealized PnL", _report_fiat(rollups["holdings"]["unrealized_pnl"])),
                ("Realized proceeds", _report_fiat(rollups["realized"]["proceeds"])),
                ("Realized cost basis", _report_fiat(rollups["realized"]["cost_basis"])),
                ("Realized gain/loss", _report_fiat(rollups["realized"]["gain_loss"])),
            ]
        )
    )

    lines.extend(["", "Asset Flow", "----------"])
    asset_flow_rows = [
        [
            row["asset"],
            _report_count(row["tx_count"]),
            _report_count(row["inbound_count"]),
            _report_count(row["outbound_count"]),
            _report_btc(msat_to_btc(row["inbound_amount"] or 0)),
            _report_btc(msat_to_btc(row["outbound_amount"] or 0)),
            _report_btc(msat_to_btc(row["fee_amount"] or 0)),
        ]
        for row in query_rows["flow_by_asset"]
    ]
    if asset_flow_rows:
        lines.extend(
            hooks.format_table(
                ["Asset", "Tx", "In", "Out", "Inbound", "Outbound", "Fees"],
                asset_flow_rows,
                [6, 6, 6, 6, 14, 14, 14],
                align_right={1, 2, 3, 4, 5, 6},
            )
        )
    else:
        lines.append("No active transactions in scope.")
    return lines


def _tax_summary_total_row(
    row_type,
    *,
    year=None,
    asset="",
    quantity: Decimal | None = None,
    proceeds=Decimal("0"),
    cost_basis=Decimal("0"),
    gain_loss=Decimal("0"),
):
    return {
        "row_type": row_type,
        "year": year,
        "asset": asset,
        "transaction_type": "",
        "capital_gains_type": "",
        "quantity": float(quantity) if quantity is not None else None,
        "quantity_msat": btc_to_msat(quantity) if quantity is not None else None,
        "proceeds": float(proceeds),
        "cost_basis": float(cost_basis),
        "gain_loss": float(gain_loss),
    }


def report_tax_summary(conn, workspace_ref, profile_ref, hooks: ReportHooks):
    _, profile = _resolve_report_scope(conn, workspace_ref, profile_ref, hooks)
    hooks.require_processed_journals(conn, profile)
    state = hooks.build_ledger_state(conn, profile)
    detail_rows = sorted(
        state["tax_summary"],
        key=lambda row: (
            int(row["year"]),
            row["asset"],
            row["transaction_type"],
            row["capital_gains_type"],
        ),
    )
    if not detail_rows:
        return []

    grouped_by_year = defaultdict(
        lambda: {
            "assets": set(),
            "quantity": Decimal("0"),
            "proceeds": Decimal("0"),
            "cost_basis": Decimal("0"),
            "gain_loss": Decimal("0"),
        }
    )
    grand = {
        "assets": set(),
        "quantity": Decimal("0"),
        "proceeds": Decimal("0"),
        "cost_basis": Decimal("0"),
        "gain_loss": Decimal("0"),
    }
    grouped_rows = defaultdict(list)
    for row in detail_rows:
        quantity = dec(row["quantity"])
        proceeds = dec(row["proceeds"])
        cost_basis = dec(row["cost_basis"])
        gain_loss = dec(row["gain_loss"])
        year = int(row["year"])
        grouped_rows[year].append({"row_type": "detail", **row})
        grouped_by_year[year]["assets"].add(row["asset"])
        grouped_by_year[year]["quantity"] += quantity
        grouped_by_year[year]["proceeds"] += proceeds
        grouped_by_year[year]["cost_basis"] += cost_basis
        grouped_by_year[year]["gain_loss"] += gain_loss
        grand["assets"].add(row["asset"])
        grand["quantity"] += quantity
        grand["proceeds"] += proceeds
        grand["cost_basis"] += cost_basis
        grand["gain_loss"] += gain_loss

    rows = []
    for year in sorted(grouped_rows):
        year_asset = next(iter(grouped_by_year[year]["assets"])) if len(grouped_by_year[year]["assets"]) == 1 else ""
        year_quantity = grouped_by_year[year]["quantity"] if year_asset else None
        rows.extend(grouped_rows[year])
        rows.append(
            _tax_summary_total_row(
                "year_total",
                year=year,
                asset=year_asset,
                quantity=year_quantity,
                proceeds=grouped_by_year[year]["proceeds"],
                cost_basis=grouped_by_year[year]["cost_basis"],
                gain_loss=grouped_by_year[year]["gain_loss"],
            )
        )
    rows.append(
        _tax_summary_total_row(
            "grand_total",
            asset=next(iter(grand["assets"])) if len(grand["assets"]) == 1 else "",
            quantity=grand["quantity"] if len(grand["assets"]) == 1 else None,
            proceeds=grand["proceeds"],
            cost_basis=grand["cost_basis"],
            gain_loss=grand["gain_loss"],
        )
    )
    return rows


def build_pdf_report_lines(conn, workspace_ref, profile_ref, hooks: ReportHooks, wallet_ref=None, history_limit=None):
    workspace, profile = _resolve_report_scope(conn, workspace_ref, profile_ref, hooks)
    wallet = hooks.resolve_wallet(conn, profile["id"], wallet_ref) if wallet_ref else None
    hooks.require_processed_journals(conn, profile)

    scope_wallets = _scope_wallets(conn, workspace["id"], profile["id"], hooks, wallet=wallet)
    portfolio_rows = report_portfolio_summary(conn, workspace["id"], profile["id"], hooks)
    if wallet:
        portfolio_rows = [row for row in portfolio_rows if row["wallet"] == wallet["label"]]
    balance_rows = _aggregate_balance_rows_from_portfolio(portfolio_rows)

    capital_rows = report_capital_gains(conn, workspace["id"], profile["id"], hooks)
    if wallet:
        capital_rows = [row for row in capital_rows if row["wallet"] == wallet["label"]]
    history_rows = report_balance_history(
        conn,
        workspace["id"],
        profile["id"],
        hooks,
        interval="month",
        wallet_ref=wallet["id"] if wallet else None,
    )
    if history_limit is not None and int(history_limit) > 0:
        history_rows = history_rows[-int(history_limit) :]

    query_rows = _report_query_rows(conn, profile, wallet=wallet)
    summary = query_rows["summary"]

    rollups = _summary_rollups(balance_rows, capital_rows)
    holdings_cost_basis = rollups["holdings"]["cost_basis"]
    holdings_market_value = rollups["holdings"]["market_value"]
    holdings_unrealized = rollups["holdings"]["unrealized_pnl"]
    realized_proceeds = rollups["realized"]["proceeds"]
    realized_cost_basis = rollups["realized"]["cost_basis"]
    realized_gain_loss = rollups["realized"]["gain_loss"]

    title_scope = wallet["label"] if wallet else profile["label"]
    title = f"Kassiber PDF Report - {title_scope}"

    lines = [title, "=" * len(title), ""]
    lines.extend(
        _report_kv_lines(
            [
                ("Generated at", hooks.now_iso()),
                ("Workspace", workspace["label"]),
                ("Profile", profile["label"]),
                ("Wallet scope", wallet["label"] if wallet else "All wallets"),
                ("Fiat currency", profile["fiat_currency"]),
                ("Tax country", profile["tax_country"]),
                ("Tax long-term days", profile["tax_long_term_days"]),
                ("Gains algorithm", profile["gains_algorithm"]),
                ("Last processed at", profile["last_processed_at"] or ""),
                ("Processed tx count", _report_count(profile["last_processed_tx_count"])),
            ]
        )
    )

    lines.extend(["", "Executive Summary", "-----------------"])
    lines.extend(
        _report_kv_lines(
            [
                ("Wallets in scope", _report_count(len(scope_wallets))),
                ("Assets in scope", _report_count(summary["asset_count"])),
                ("Transactions (active)", _report_count(summary["active_transactions"])),
                ("Transactions (excluded)", _report_count(summary["excluded_transactions"])),
                ("Inbound transactions", _report_count(summary["inbound_transactions"])),
                ("Outbound transactions", _report_count(summary["outbound_transactions"])),
                ("Journal entries", _report_count(query_rows["journal_entries"])),
                ("Quarantines", _report_count(query_rows["quarantines"])),
                ("Priced transactions", _report_count(summary["priced_transactions"])),
                ("Transactions with notes", _report_count(summary["noted_transactions"])),
                ("Transactions with tags", _report_count(query_rows["tagged_transactions"])),
                ("First transaction", summary["first_transaction_at"] or ""),
                ("Last transaction", summary["last_transaction_at"] or ""),
                ("Holdings cost basis", _report_fiat(holdings_cost_basis)),
                ("Holdings market value", _report_fiat(holdings_market_value)),
                ("Unrealized PnL", _report_fiat(holdings_unrealized)),
                ("Realized proceeds", _report_fiat(realized_proceeds)),
                ("Realized cost basis", _report_fiat(realized_cost_basis)),
                ("Realized gain/loss", _report_fiat(realized_gain_loss)),
            ]
        )
    )

    lines.extend(["", "Wallet Inventory", "----------------"])
    wallet_table_rows = [
        [
            row["label"],
            row["kind"],
            row["chain"],
            row["network"],
            row["backend"],
            row["gap_limit"],
        ]
        for row in scope_wallets
    ]
    if wallet_table_rows:
        lines.extend(
            hooks.format_table(
                ["Wallet", "Kind", "Chain", "Network", "Backend", "Gap"],
                wallet_table_rows,
                [18, 12, 8, 10, 12, 5],
            )
        )
    else:
        lines.append("No wallets in scope.")

    lines.extend(["", "Asset Flow Summary", "------------------"])
    asset_flow_rows = [
        [
            row["asset"],
            _report_count(row["tx_count"]),
            _report_count(row["inbound_count"]),
            _report_count(row["outbound_count"]),
            _report_btc(msat_to_btc(row["inbound_amount"] or 0)),
            _report_btc(msat_to_btc(row["outbound_amount"] or 0)),
            _report_btc(msat_to_btc(row["fee_amount"] or 0)),
        ]
        for row in query_rows["flow_by_asset"]
    ]
    if asset_flow_rows:
        lines.extend(
            hooks.format_table(
                ["Asset", "Tx", "In", "Out", "Inbound", "Outbound", "Fees"],
                asset_flow_rows,
                [6, 6, 6, 6, 14, 14, 14],
                align_right={1, 2, 3, 4, 5, 6},
            )
        )
    else:
        lines.append("No active transactions in scope.")

    lines.extend(["", "Wallet Transaction Metrics", "--------------------------"])
    wallet_flow_rows = [
        [
            row["wallet"],
            row["asset"],
            _report_count(row["tx_count"]),
            _report_count(row["inbound_count"]),
            _report_count(row["outbound_count"]),
            _report_btc(msat_to_btc(row["inbound_amount"] or 0)),
            _report_btc(msat_to_btc(row["outbound_amount"] or 0)),
            _report_btc(msat_to_btc(row["fee_amount"] or 0)),
        ]
        for row in query_rows["flow_by_wallet"]
    ]
    if wallet_flow_rows:
        lines.extend(
            hooks.format_table(
                ["Wallet", "Asset", "Tx", "In", "Out", "Inbound", "Outbound", "Fees"],
                wallet_flow_rows,
                [18, 6, 6, 6, 6, 14, 14, 14],
                align_right={2, 3, 4, 5, 6, 7},
            )
        )
    else:
        lines.append("No wallet transaction metrics available.")

    lines.extend(["", "Balance Sheet", "-------------"])
    balance_table_rows = [
        [
            row["account"],
            row["asset"],
            _report_btc(row["quantity"]),
            _report_fiat(row["cost_basis"]),
            _report_fiat(row["market_value"]),
            _report_fiat(row["unrealized_pnl"]),
        ]
        for row in balance_rows
    ]
    if balance_table_rows:
        lines.extend(
            hooks.format_table(
                ["Bucket", "Asset", "Quantity", "Cost Basis", "Market Value", "Unrealized"],
                balance_table_rows,
                [16, 6, 14, 14, 14, 14],
                align_right={2, 3, 4, 5},
            )
        )
    else:
        lines.append("No current holdings in scope.")

    lines.extend(["", "Portfolio Summary", "-----------------"])
    portfolio_table_rows = [
        [
            row["wallet"],
            row["account"],
            row["asset"],
            _report_btc(row["quantity"]),
            _report_fiat(row["avg_cost"]),
            _report_fiat(row["cost_basis"]),
            _report_fiat(row["market_value"]),
            _report_fiat(row["unrealized_pnl"]),
        ]
        for row in portfolio_rows
    ]
    if portfolio_table_rows:
        lines.extend(
            hooks.format_table(
                ["Wallet", "Bucket", "Asset", "Quantity", "Avg Cost", "Cost Basis", "Market", "Unreal."],
                portfolio_table_rows,
                [16, 12, 6, 12, 12, 12, 12, 12],
                align_right={3, 4, 5, 6, 7},
            )
        )
    else:
        lines.append("No portfolio rows available.")

    lines.extend(["", "Capital Gains Summary", "---------------------"])
    if capital_rows:
        grouped_capital = {}
        for row in capital_rows:
            key = (row["wallet"], row["asset"])
            bucket = grouped_capital.setdefault(
                key,
                {
                    "wallet": row["wallet"],
                    "asset": row["asset"],
                    "count": 0,
                    "proceeds": 0.0,
                    "cost_basis": 0.0,
                    "gain_loss": 0.0,
                },
            )
            bucket["count"] += 1
            bucket["proceeds"] += float(row["proceeds"])
            bucket["cost_basis"] += float(row["cost_basis"])
            bucket["gain_loss"] += float(row["gain_loss"])
        lines.extend(
            hooks.format_table(
                ["Wallet", "Asset", "Rows", "Proceeds", "Cost Basis", "Gain/Loss"],
                [
                    [
                        bucket["wallet"],
                        bucket["asset"],
                        _report_count(bucket["count"]),
                        _report_fiat(bucket["proceeds"]),
                        _report_fiat(bucket["cost_basis"]),
                        _report_fiat(bucket["gain_loss"]),
                    ]
                    for bucket in grouped_capital.values()
                ],
                [16, 6, 6, 14, 14, 14],
                align_right={2, 3, 4, 5},
            )
        )
        lines.extend(["", "Capital Gains Detail", "--------------------"])
        detail_rows = [
            [
                row["occurred_at"][:10],
                row["wallet"],
                row["asset"],
                _report_btc(row["quantity"]),
                _report_fiat(row["proceeds"]),
                _report_fiat(row["cost_basis"]),
                _report_fiat(row["gain_loss"]),
            ]
            for row in capital_rows
        ]
        lines.extend(
            hooks.format_table(
                ["Date", "Wallet", "Asset", "Qty", "Proceeds", "Basis", "Gain/Loss"],
                detail_rows,
                [10, 16, 6, 12, 12, 12, 12],
                align_right={3, 4, 5, 6},
            )
        )
    else:
        lines.append("No realized disposals in scope.")

    lines.extend(["", "Balance History", "---------------"])
    if history_rows:
        lines.extend(
            hooks.format_table(
                ["Period Start", "Asset", "Quantity", "Cost Basis", "Market Value"],
                [
                    [
                        row["period_start"][:10],
                        row["asset"],
                        _report_btc(row["quantity"]),
                        _report_fiat(row["cumulative_cost_basis"]),
                        _report_fiat(row["market_value"]),
                    ]
                    for row in history_rows
                ],
                [12, 6, 14, 14, 14],
                align_right={2, 3, 4},
            )
        )
    else:
        lines.append("No balance history rows available.")

    lines.extend(["", "Data Quality", "------------"])
    if query_rows["quarantine_rows"]:
        lines.extend(
            hooks.format_table(
                ["Reason", "Count"],
                [[row["reason"], _report_count(row["count"])] for row in query_rows["quarantine_rows"]],
                [28, 10],
                align_right={1},
            )
        )
    else:
        lines.append("No quarantined transactions.")

    lines.extend(["", "Transactions", "------------"])
    if query_rows["transactions"]:
        lines.extend(
            hooks.format_table(
                ["Date", "Wallet", "Dir", "Asset", "Amount", "Fee", "Description"],
                [
                    [
                        row["occurred_at"][:10],
                        row["wallet"],
                        row["direction"][:3],
                        row["asset"],
                        _report_btc(msat_to_btc(row["amount"] or 0)),
                        _report_btc(msat_to_btc(row["fee"] or 0)),
                        row["description"],
                    ]
                    for row in query_rows["transactions"]
                ],
                [10, 14, 3, 6, 12, 12, 28],
                align_right={4, 5},
            )
        )
    else:
        lines.append("No transactions in scope.")

    return title, lines


def export_pdf_report(conn, workspace_ref, profile_ref, file_path, hooks: ReportHooks, wallet_ref=None, history_limit=None):
    title, lines = build_pdf_report_lines(
        conn,
        workspace_ref,
        profile_ref,
        hooks,
        wallet_ref=wallet_ref,
        history_limit=history_limit,
    )
    written = dict(hooks.write_text_pdf(file_path, title, lines))
    written["wallet"] = wallet_ref or ""
    return written


__all__ = [
    "INTERVAL_CHOICES",
    "ReportHooks",
    "build_pdf_report_lines",
    "export_pdf_report",
    "report_balance_history",
    "report_balance_sheet",
    "report_capital_gains",
    "report_journal_entries",
    "report_portfolio_summary",
    "build_summary_report_lines",
    "report_summary",
    "report_tax_summary",
]
