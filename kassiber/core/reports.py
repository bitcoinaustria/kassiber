from __future__ import annotations

import csv
import sqlite3
from collections import defaultdict
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from decimal import Decimal, ROUND_HALF_UP
from pathlib import Path
from typing import Any, Callable, Mapping, Sequence

from .austrian import kennzahl_for_disposal_category
from ..errors import AppError
from ..msat import btc_to_msat, dec, msat_to_btc
from ..tax_policy import require_tax_processing_supported

INTERVAL_CHOICES = ("hour", "day", "week", "month")
EUR_CENT = Decimal("0.01")

AUSTRIAN_E1KV_REVIEW_GATE = (
    "Review this Austrian E 1kv export with a Steuerberater before filing; "
    "Kassiber is not tax advice."
)
AUSTRIAN_E1KV_SELF_CUSTODY_ASSUMPTION = (
    "Kassiber currently maps crypto rows to the auslaendisch / self-custody "
    "E 1kv Kennzahlen 172, 174, and 176. It does not populate domestic-provider "
    "or withheld-KESt fields because that metadata is not stored yet."
)
AUSTRIAN_E1KV_DETAIL_LIMITATION = (
    "Lot acquisition dates and holding-period day counts are not persisted in "
    "journal rows; the export relies on RP2's Austrian category classification "
    "and Kassiber's journal amounts."
)
AUSTRIAN_E1KV_REPROCESS_HINT = (
    "Capital-yield income (income_capital_yield) now maps to Kennzahl 172 "
    "instead of the old 175. Re-run `journals process` after upgrading so "
    "stored at_kennzahl values match this export; any mismatches are listed "
    "in the Data Quality section."
)
AUSTRIAN_E1KV_FORM_SECTION = "E 1kv 1.3.5 Einkuenfte aus Kryptowaehrungen"
AUSTRIAN_E1KV_KENNZAHL_LABELS = {
    172: "Auslaendische laufende Einkuenfte aus Kryptowaehrungen",
    174: "Auslaendische Ueberschuesse aus realisierten Wertsteigerungen",
    176: "Auslaendische realisierte Wertverluste",
    801: "Spekulationsgeschaefte Altbestand (outside E 1kv)",
}
AUSTRIAN_E1KV_SUPPORTED_KENNZAHL_ORDER = (172, 174, 176)
AUSTRIAN_E1KV_CATEGORY_LABELS = {
    "income_general": "Laufende Einkuenfte aus Kryptowaehrungen",
    "income_capital_yield": "Laufende Einkuenfte aus Ueberlassung von Kryptowaehrungen",
    "neu_gain": "Realisierte Wertsteigerung Neuvermoegen",
    "neu_loss": "Realisierter Wertverlust Neuvermoegen",
    "neu_swap": "Krypto-zu-Krypto Tausch mit Buchwertfortfuehrung",
    "alt_spekulation": "Altbestand innerhalb Spekulationsfrist",
    "alt_taxfree": "Altbestand ausserhalb Spekulationsfrist",
}
AUSTRIAN_TAX_SECTION_ORDER = (
    "1.1",
    "1.2",
    "1.3",
    "2.1",
    "2.2",
    "3.1",
    "3.2",
    "3.3",
    "4.1",
    "4.2",
    "4.3",
    "4.4",
    "4.5",
)
AUSTRIAN_TAX_SECTION_METADATA = {
    "1.1": {
        "label": "Steuerpflichtige Einkuenfte aus dem Handel mit Kryptowaehrungen",
        "law": "27b Abs 3 EStG",
        "supported": True,
        "kennzahlen": (174, 176),
    },
    "1.2": {
        "label": "Steuerpflichtige Einkuenfte aus Margin, Derivaten und Futures",
        "law": "27 Z 4 EStG",
        "supported": False,
        "kennzahlen": (),
    },
    "1.3": {
        "label": "Steuerpflichtige Einkuenfte aus NFT-Spekulationsgeschaeften",
        "law": "31 EStG",
        "supported": False,
        "kennzahlen": (),
    },
    "2.1": {
        "label": "Steuerpflichtige laufende Einkuenfte aus der Ueberlassung von Kryptowaehrungen",
        "law": "27b Abs 2 Z 1 EStG",
        "supported": True,
        "kennzahlen": (172,),
    },
    "2.2": {
        "label": "Steuerpflichtige laufende Einkuenfte aus Leistungen zur Transaktionsverarbeitung",
        "law": "27b Abs 2 Z 2 EStG",
        "supported": True,
        "kennzahlen": (172,),
    },
    "3.1": {
        "label": "Nicht steuerbare Einkuenfte aus Spekulationsgeschaeften mit Kryptowaehrungen",
        "law": "27b Abs 3 EStG Altvermoegen",
        "supported": True,
        "kennzahlen": (801,),
    },
    "3.2": {
        "label": "Nicht steuerbare Einkuenfte mit Bewertung 0",
        "law": "27b Abs 2 Z 2 Satz 2 EStG",
        "supported": False,
        "kennzahlen": (),
    },
    "3.3": {
        "label": "Nicht steuerbare Steuergebuehren und Rueckerstattungen",
        "law": "",
        "supported": False,
        "kennzahlen": (),
    },
    "4.1": {
        "label": "Eingegangene Spenden und Schenkungen",
        "law": "",
        "supported": False,
        "kennzahlen": (),
    },
    "4.2": {
        "label": "Ausgegangene Spenden und Schenkungen",
        "law": "",
        "supported": False,
        "kennzahlen": (),
    },
    "4.3": {
        "label": "Verlorene und gestohlene Coins",
        "law": "",
        "supported": False,
        "kennzahlen": (),
    },
    "4.4": {
        "label": "Mining gewerblich",
        "law": "",
        "supported": False,
        "kennzahlen": (),
    },
    "4.5": {
        "label": "Minting",
        "law": "",
        "supported": False,
        "kennzahlen": (),
    },
}
AUSTRIAN_TAX_SECTION_GROUPS = (
    (
        "1. Steuerpflichtige Einkuenfte aus dem Handel mit Kryptowaehrungen",
        ("1.1", "1.2", "1.3"),
    ),
    ("2. Steuerpflichtige laufende Einkuenfte", ("2.1", "2.2")),
    ("3. Nicht steuerbare Einkuenfte", ("3.1", "3.2", "3.3")),
    ("4. Sonstige Ein- und Ausgaenge", ("4.1", "4.2", "4.3", "4.4", "4.5")),
)

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


def _markdown_table_cell(value):
    return str(value if value is not None else "").replace("|", "/").replace("\n", " ")


def _markdown_table_lines(headers, rows):
    lines = [
        "| " + " | ".join(_markdown_table_cell(header) for header in headers) + " |",
        "| " + " | ".join("---" for _header in headers) + " |",
    ]
    for row in rows:
        lines.append("| " + " | ".join(_markdown_table_cell(cell) for cell in row) + " |")
    return lines


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


def _require_austrian_e1kv_profile(profile):
    tax_country = str(profile["tax_country"] or "").strip().lower()
    if tax_country != "at":
        raise AppError(
            "Austrian E 1kv export requires an Austrian tax profile",
            code="validation",
            hint="Use `profiles set --tax-country at --fiat-currency EUR` and re-run `journals process` first.",
            details={"tax_country": profile["tax_country"]},
        )
    fiat_currency = str(profile["fiat_currency"] or "").strip().upper()
    if fiat_currency != "EUR":
        raise AppError(
            "Austrian E 1kv export requires EUR journal amounts",
            code="validation",
            hint="Use an Austrian profile with --fiat-currency EUR and re-run `journals process`.",
            details={"fiat_currency": profile["fiat_currency"]},
        )


def _normalize_tax_year(year):
    if year is None:
        raise AppError("--year is required for Austrian E 1kv export", code="validation")
    try:
        normalized = int(year)
    except (TypeError, ValueError) as exc:
        raise AppError("--year must be a four-digit tax year", code="validation") from exc
    if normalized < 2009 or normalized > 2100:
        raise AppError("--year must be a plausible four-digit tax year", code="validation")
    return normalized


def _eur_cents(value):
    if value is None:
        return None
    rounded = dec(value).quantize(EUR_CENT, rounding=ROUND_HALF_UP)
    return int(rounded * 100)


def _eur_from_cents(cents):
    return Decimal(int(cents or 0)) / Decimal("100")


def _xlsx_eur_from_cents(cents):
    if cents is None:
        return None
    return float(_eur_from_cents(cents))


def _report_eur_cents(cents):
    return _report_fiat(_eur_from_cents(cents))


def _at_regime_from_category(category):
    if not category:
        return ""
    if str(category).startswith("neu_"):
        return "neu"
    if str(category).startswith("alt_"):
        return "alt"
    if str(category).startswith("income_"):
        return "income"
    return ""


def _austrian_e1kv_form_amount(row, kennzahl):
    gain_loss = dec(row["gain_loss"] or 0)
    if str(row["entry_type"]) == "income":
        return gain_loss
    if kennzahl == 176:
        return abs(gain_loss)
    if kennzahl in {172, 174, 801}:
        return gain_loss
    return Decimal("0")


def _austrian_e1kv_detail_row(row):
    category = row["at_category"]
    kennzahl = kennzahl_for_disposal_category(category)
    quantity_msat = abs(int(row["quantity"] or 0))
    quantity = msat_to_btc(quantity_msat)
    proceeds = dec(row["proceeds"] or 0)
    cost_basis = dec(row["cost_basis"] or 0)
    gain_loss = dec(row["gain_loss"] or 0)
    income = gain_loss if str(row["entry_type"]) == "income" else Decimal("0")
    price_basis = income if str(row["entry_type"]) == "income" else proceeds
    price = price_basis / quantity if quantity else None
    form_amount = _austrian_e1kv_form_amount(row, kennzahl)
    occurred_at = str(row["occurred_at"])
    note = row["transaction_note"] or row["description"] or ""
    return {
        "tax_year": int(occurred_at[:4]),
        "date": occurred_at[:10],
        "tx_id": row["transaction_external_id"] or row["transaction_id"],
        "transaction_id": row["transaction_id"],
        "wallet": row["wallet"],
        "asset": row["asset"],
        "kind": row["transaction_kind"] or row["entry_type"],
        "entry_type": row["entry_type"],
        "at_category": category,
        "at_category_label": AUSTRIAN_E1KV_CATEGORY_LABELS.get(category, ""),
        "at_regime": _at_regime_from_category(category),
        "qty_msat": quantity_msat,
        "quantity": float(quantity),
        "price_eur_cents": _eur_cents(price),
        "cost_basis_eur_cents": _eur_cents(cost_basis),
        "proceeds_eur_cents": _eur_cents(proceeds),
        "gain_loss_eur_cents": _eur_cents(gain_loss),
        "income_eur_cents": _eur_cents(income),
        "form_amount_eur_cents": _eur_cents(form_amount),
        "holding_period_days": None,
        "kennzahl": kennzahl,
        "stored_kennzahl": row["at_kennzahl"],
        "form_section": AUSTRIAN_E1KV_FORM_SECTION if kennzahl in {172, 174, 176} else "",
        "note": note,
    }


def _austrian_e1kv_rows(conn, profile, tax_year):
    where = ["je.profile_id = ?", "je.at_category IS NOT NULL"]
    params: list[Any] = [profile["id"]]
    if tax_year is not None:
        where.append("substr(je.occurred_at, 1, 4) = ?")
        params.append(str(tax_year))
    rows = conn.execute(
        f"""
        SELECT
            je.occurred_at,
            je.transaction_id,
            je.entry_type,
            je.asset,
            je.quantity,
            je.fiat_value,
            COALESCE(je.cost_basis, 0) AS cost_basis,
            COALESCE(je.proceeds, 0) AS proceeds,
            COALESCE(je.gain_loss, 0) AS gain_loss,
            COALESCE(je.description, '') AS description,
            je.at_category,
            je.at_kennzahl,
            w.label AS wallet,
            t.external_id AS transaction_external_id,
            t.kind AS transaction_kind,
            t.note AS transaction_note
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        LEFT JOIN transactions t ON t.id = je.transaction_id
        WHERE {' AND '.join(where)}
        ORDER BY je.occurred_at ASC, je.created_at ASC, je.id ASC
        """,
        params,
    ).fetchall()
    return [_austrian_e1kv_detail_row(row) for row in rows]


def _austrian_e1kv_quarantines(conn, profile, tax_year):
    where = ["jq.profile_id = ?"]
    params: list[Any] = [profile["id"]]
    if tax_year is not None:
        where.append("substr(t.occurred_at, 1, 4) = ?")
        params.append(str(tax_year))
    return [
        {"reason": row["reason"], "count": int(row["count"] or 0)}
        for row in conn.execute(
            f"""
            SELECT jq.reason, COUNT(*) AS count
            FROM journal_quarantines jq
            JOIN transactions t ON t.id = jq.transaction_id
            WHERE {' AND '.join(where)}
            GROUP BY jq.reason
            ORDER BY count DESC, jq.reason ASC
            """,
            params,
        ).fetchall()
    ]


def _austrian_e1kv_summary_rows(rows):
    totals = defaultdict(lambda: {"amount": 0, "count": 0})
    for row in rows:
        kennzahl = row["kennzahl"]
        if kennzahl is None:
            continue
        totals[kennzahl]["amount"] += int(row["form_amount_eur_cents"] or 0)
        totals[kennzahl]["count"] += 1

    codes = list(AUSTRIAN_E1KV_SUPPORTED_KENNZAHL_ORDER)
    for code in sorted(code for code in totals if code not in codes):
        codes.append(code)

    return [
        {
            "kennzahl": code,
            "label": AUSTRIAN_E1KV_KENNZAHL_LABELS.get(code, ""),
            "row_count": totals[code]["count"],
            "amount_eur_cents": totals[code]["amount"],
        }
        for code in codes
    ]


def _austrian_e1kv_kennzahl_totals(summary_rows):
    return {
        str(row["kennzahl"]): {
            "label": row["label"],
            "row_count": row["row_count"],
            "amount_eur_cents": row["amount_eur_cents"],
        }
        for row in summary_rows
    }


def _austrian_tax_empty_section(section_id):
    metadata = AUSTRIAN_TAX_SECTION_METADATA[section_id]
    return {
        "section_id": section_id,
        "label": metadata["label"],
        "law": metadata["law"],
        "supported": bool(metadata["supported"]),
        "status": "supported" if metadata["supported"] else "not_modelled",
        "kennzahlen": list(metadata["kennzahlen"]),
        "totals": {
            "row_count": 0,
            "quantity_msat": 0,
            "amount_eur_cents": 0,
            "proceeds_eur_cents": 0,
            "cost_basis_eur_cents": 0,
            "gain_loss_eur_cents": 0,
            "income_eur_cents": 0,
        },
        "detail_rows": [],
    }


def _austrian_tax_section_id(row):
    category = row["at_category"]
    if category in {"neu_gain", "neu_loss", "neu_swap"}:
        return "1.1"
    if category == "income_capital_yield":
        return "2.1"
    if category == "income_general":
        return "2.2"
    if category in {"alt_spekulation", "alt_taxfree"}:
        return "3.1"
    return None


def _austrian_tax_sections(rows):
    sections = {
        section_id: _austrian_tax_empty_section(section_id)
        for section_id in AUSTRIAN_TAX_SECTION_ORDER
    }
    for row in rows:
        section_id = _austrian_tax_section_id(row)
        if section_id is None:
            continue
        section = sections[section_id]
        totals = section["totals"]
        section["detail_rows"].append(row)
        totals["row_count"] += 1
        totals["quantity_msat"] += int(row["qty_msat"] or 0)
        totals["amount_eur_cents"] += int(row["form_amount_eur_cents"] or 0)
        totals["proceeds_eur_cents"] += int(row["proceeds_eur_cents"] or 0)
        totals["cost_basis_eur_cents"] += int(row["cost_basis_eur_cents"] or 0)
        totals["gain_loss_eur_cents"] += int(row["gain_loss_eur_cents"] or 0)
        totals["income_eur_cents"] += int(row["income_eur_cents"] or 0)
    return sections


def _austrian_section_title(section_id, section):
    law = f" ({section['law']})" if section["law"] else ""
    return f"{section_id}. {section['label']}{law}"


def _austrian_section_amount(section):
    return _report_eur_cents(section["totals"]["amount_eur_cents"])


def _austrian_unsupported_section_lines(section_id, section):
    title = _austrian_section_title(section_id, section)
    return [
        title,
        "-" * len(title),
        "Status: not modelled in Kassiber yet; placeholder total is 0.00 EUR.",
        "Amount: 0.00 EUR",
        "",
    ]


def _austrian_disposal_split(rows):
    split = {
        "gains_proceeds": 0,
        "gains_cost_basis": 0,
        "gains_amount": 0,
        "losses_proceeds": 0,
        "losses_cost_basis": 0,
        "losses_amount": 0,
    }
    for row in rows:
        if row["kennzahl"] == 176:
            split["losses_proceeds"] += int(row["proceeds_eur_cents"] or 0)
            split["losses_cost_basis"] += int(row["cost_basis_eur_cents"] or 0)
            split["losses_amount"] += int(row["form_amount_eur_cents"] or 0)
        elif row["kennzahl"] == 174:
            split["gains_proceeds"] += int(row["proceeds_eur_cents"] or 0)
            split["gains_cost_basis"] += int(row["cost_basis_eur_cents"] or 0)
            split["gains_amount"] += int(row["form_amount_eur_cents"] or 0)
    return split


def _austrian_e1kv_overview_entries(report):
    sections = report["sections"]
    split_11 = _austrian_disposal_split(sections["1.1"]["detail_rows"])
    entries = []

    def heading(text):
        entries.append(("heading", text))

    def section(text):
        entries.append(("section", text))

    def amount(label, cents, total=False):
        entries.append(("amount", label, int(cents or 0), bool(total)))

    heading("1. Steuerpflichtige Einkünfte aus dem Handel mit Kryptowährungen")
    section("1.1. Steuerpflichtige Einkünfte aus dem An- und Verkauf von Kryptowährungen")
    amount("Veräußerungspreis", split_11["gains_proceeds"])
    amount("Anschaffungskosten", split_11["gains_cost_basis"])
    amount("Veräußerungsgewinn", split_11["gains_amount"], total=True)
    amount("Veräußerungspreis", split_11["losses_proceeds"])
    amount("Anschaffungskosten", split_11["losses_cost_basis"])
    amount("Veräußerungsverlust", split_11["losses_amount"], total=True)
    section("1.2. Steuerpflichtige Einkünfte aus Margin, Derivaten und Futures")
    amount("Gewinne aus Margin, Derivaten und Futures", 0, total=True)
    amount("Verluste aus Margin, Derivaten und Futures", 0, total=True)
    section("1.3. Steuerpflichtige Einkünfte aus NFT-Spekulationsgeschäften")
    amount("Summe Einkünfte aus NFT-Spekulationsgeschäften", 0, total=True)

    heading("2. Steuerpflichtige laufende Einkünfte")
    section("2.1. Einkünfte aus der Überlassung von Kryptowährungen")
    amount("Summe laufende Einkünfte", sections["2.1"]["totals"]["amount_eur_cents"], total=True)
    section("2.2. Einkünfte aus Leistungen zur Transaktionsverarbeitung")
    amount("Summe laufende Einkünfte", sections["2.2"]["totals"]["amount_eur_cents"], total=True)

    heading("3. Nicht steuerbare Einkünfte")
    section("3.1. Nicht steuerbare Einkünfte aus Spekulationsgeschäften")
    amount("Summe nicht steuerbare Einkünfte", sections["3.1"]["totals"]["amount_eur_cents"], total=True)
    section("3.2. Nicht steuerbare Einkünfte gem. § 27b Abs 2 Z 2 Satz 2 EStG")
    amount("Summe nicht steuerbare Einkünfte", 0, total=True)
    section("3.3. Nicht steuerbare Steuergebühren und Rückerstattungen")
    amount("Summe entrichtete Steuergebühren", 0, total=True)
    amount("Summe Rückerstattungen", 0, total=True)

    heading("4. Sonstige Ein- und Ausgänge")
    section("4.1. Eingegangene Spenden/Trinkgeld")
    amount("Summe Spenden/Trinkgeld", 0, total=True)
    section("4.2. Ausgegangene Spenden/Schenkungen")
    amount("Summe Spenden", 0, total=True)
    amount("Summe Schenkungen", 0, total=True)
    section("4.3. Gestohlene, gehackte und verlorene Coins")
    amount("Summe gestohlen/gehackt/Betrug", 0, total=True)
    amount("Summe Verlust", 0, total=True)
    section("4.4. Mining (kommerziell)")
    amount("Summe Mining", 0, total=True)
    section("4.5. Minting")
    amount("Summe Minting", 0, total=True)
    return entries


def _austrian_e1kv_assumptions(rows):
    assumptions = [
        {
            "code": "AT-E1KV-FOREIGN-SELF-CUSTODY",
            "severity": "review",
            "message": AUSTRIAN_E1KV_SELF_CUSTODY_ASSUMPTION,
        },
        {
            "code": "AT-E1KV-DETAIL-LIMITATION",
            "severity": "review",
            "message": AUSTRIAN_E1KV_DETAIL_LIMITATION,
        },
        {
            "code": "AT-E1KV-KENNZAHL-REPROCESS",
            "severity": "review",
            "message": AUSTRIAN_E1KV_REPROCESS_HINT,
        },
    ]
    if any(str(row["asset"]).upper() == "LBTC" for row in rows):
        assumptions.append(
            {
                "code": "AT-002",
                "severity": "review",
                "message": "L-BTC is treated as Kryptowaehrung like BTC for this report period.",
            }
        )
    if any(str(row["kind"]).lower() == "routing_income" for row in rows):
        assumptions.append(
            {
                "code": "AT-001",
                "severity": "review",
                "message": "Lightning routing fees are treated as laufende Einkuenfte at fair market value.",
            }
        )
    assumptions.append(
        {
            "code": "AT-REVIEW-GATE",
            "severity": "review",
            "message": AUSTRIAN_E1KV_REVIEW_GATE,
        }
    )
    return assumptions


def _austrian_e1kv_mismatches(rows):
    mismatches = []
    for row in rows:
        stored = row["stored_kennzahl"]
        current = row["kennzahl"]
        if stored is not None and current is not None and int(stored) != int(current):
            mismatches.append(
                {
                    "tx_id": row["tx_id"],
                    "at_category": row["at_category"],
                    "stored_kennzahl": stored,
                    "export_kennzahl": current,
                }
            )
    return mismatches


def _austrian_e1kv_mismatch_table_rows(report):
    return [
        [
            row["tx_id"],
            AUSTRIAN_E1KV_CATEGORY_LABELS.get(row["at_category"], row["at_category"]),
            row["stored_kennzahl"],
            row["export_kennzahl"],
        ]
        for row in report["data_quality"]["kennzahl_mismatches"]
    ]


def report_austrian_e1kv(conn, workspace_ref, profile_ref, hooks: ReportHooks, tax_year=None):
    workspace, profile = _resolve_report_scope(conn, workspace_ref, profile_ref, hooks)
    _require_austrian_e1kv_profile(profile)
    hooks.require_processed_journals(conn, profile)
    normalized_year = _normalize_tax_year(tax_year)
    rows = _austrian_e1kv_rows(conn, profile, normalized_year)
    quarantines = _austrian_e1kv_quarantines(conn, profile, normalized_year)
    summary_rows = _austrian_e1kv_summary_rows(rows)
    return {
        "workspace": workspace["label"],
        "profile": profile["label"],
        "tax_year": normalized_year,
        "fiat_currency": profile["fiat_currency"],
        "tax_country": profile["tax_country"],
        "form": "E 1kv",
        "form_section": AUSTRIAN_E1KV_FORM_SECTION,
        "review_gate": AUSTRIAN_E1KV_REVIEW_GATE,
        "assumptions": _austrian_e1kv_assumptions(rows),
        "summary_rows": summary_rows,
        "kennzahl_totals": _austrian_e1kv_kennzahl_totals(summary_rows),
        "section_order": list(AUSTRIAN_TAX_SECTION_ORDER),
        "sections": _austrian_tax_sections(rows),
        "rows": rows,
        "data_quality": {
            "quarantines": quarantines,
            "kennzahl_mismatches": _austrian_e1kv_mismatches(rows),
        },
    }


def _build_austrian_e1kv_report_lines(conn, workspace_ref, profile_ref, hooks: ReportHooks, tax_year=None):
    report = report_austrian_e1kv(conn, workspace_ref, profile_ref, hooks, tax_year=tax_year)
    scope = str(report["tax_year"])
    title = f"Kassiber Austrian E 1kv / Steuerbericht - {report['profile']} ({scope})"
    lines = [title, "=" * len(title), ""]
    lines.extend(
        _report_kv_lines(
            [
                ("Workspace", report["workspace"]),
                ("Profile", report["profile"]),
                ("Tax year", scope),
                ("Fiat currency", report["fiat_currency"]),
                ("Tax country", report["tax_country"]),
                ("Form", report["form"]),
                ("Section", report["form_section"]),
            ]
        )
    )

    lines.extend(["", "Hinweise zum Berichtsumfang", "---------------------------"])
    for assumption in report["assumptions"]:
        lines.append(f"{assumption['code']}: {assumption['message']}")

    lines.extend(["", "FinanzOnline Kennzahlen", "-----------------------"])
    lines.extend(
        _markdown_table_lines(
            ["KZ", "Description", "Rows", "Amount EUR"],
            [
                [
                    str(row["kennzahl"]),
                    row["label"],
                    _report_count(row["row_count"]),
                    _report_eur_cents(row["amount_eur_cents"]),
                ]
                for row in report["summary_rows"]
            ],
        )
    )

    lines.extend(["", "I. Übersicht", "------------"])
    for entry in _austrian_e1kv_overview_entries(report):
        if entry[0] == "heading":
            lines.extend(["", entry[1], "-" * len(entry[1])])
        elif entry[0] == "section":
            lines.extend(["", entry[1]])
        else:
            _kind, label, cents, _is_total = entry
            lines.extend(_report_kv_lines([(label, f"{_report_eur_cents(cents)} EUR")], label_width=54))

    lines.extend(["", "II. Detail Sections", "-------------------"])
    for spec in _austrian_e1kv_section_table_specs(report):
        lines.extend(["", spec["title"], "-" * len(spec["title"])])
        if spec["rows"]:
            lines.extend(
                _markdown_table_lines(
                    spec["headers"],
                    [
                        _austrian_e1kv_render_table_row(row, spec["row_format_names"])
                        for row in spec["rows"]
                    ],
                )
            )
        else:
            lines.append("No rows in scope.")
        if spec["total_rows"]:
            lines.append("")
            lines.extend(
                _report_kv_lines(
                    [
                        (label, f"{_austrian_e1kv_render_cell(value, 'money')} EUR")
                        for label, value in spec["total_rows"]
                    ],
                    label_width=54,
                )
            )

    lines.extend(["", "Data Quality", "------------"])
    quarantines = report["data_quality"]["quarantines"]
    mismatches = report["data_quality"]["kennzahl_mismatches"]
    if quarantines:
        lines.append("Quarantined transactions remain outside this export:")
        lines.extend(
            hooks.format_table(
                ["Reason", "Count"],
                [[row["reason"], _report_count(row["count"])] for row in quarantines],
                [32, 8],
                align_right={1},
            )
        )
    else:
        lines.append("No quarantined transactions in scope.")
    if mismatches:
        lines.append("Some rows had stale stored Kennzahlen; the export used the current category mapping.")
        lines.extend(
            _markdown_table_lines(
                ["Tx ID", "Category", "Stored KZ", "Export KZ"],
                _austrian_e1kv_mismatch_table_rows(report),
            )
        )
    else:
        lines.append("Stored Kennzahlen match the current export mapping.")

    lines.extend(["", "Review Footer", "-------------", AUSTRIAN_E1KV_REVIEW_GATE])
    return title, lines, report


def build_austrian_e1kv_report_lines(conn, workspace_ref, profile_ref, hooks: ReportHooks, tax_year=None):
    _, lines, _ = _build_austrian_e1kv_report_lines(
        conn,
        workspace_ref,
        profile_ref,
        hooks,
        tax_year=tax_year,
    )
    return lines


def export_austrian_e1kv_pdf_report(conn, workspace_ref, profile_ref, file_path, hooks: ReportHooks, tax_year=None):
    title, lines, report = _build_austrian_e1kv_report_lines(
        conn,
        workspace_ref,
        profile_ref,
        hooks,
        tax_year=tax_year,
    )
    written = dict(hooks.write_text_pdf(file_path, title, lines))
    written["tax_year"] = report["tax_year"]
    written["form"] = report["form"]
    written["assumptions"] = report["assumptions"]
    return written


AUSTRIAN_E1KV_XLSX_SHEETS = (
    "Übersicht",
    "1.1.",
    "1.2.",
    "1.3.",
    "2.1.",
    "2.2.",
    "3.1.",
    "3.2.",
    "3.3.",
    "4.1.",
    "4.2.",
    "4.3.",
    "4.4.",
    "4.5.",
    "Erläuterungen zum Steuerreport",
)

AUSTRIAN_E1KV_XLSX_TITLES = {
    "1.1": "1.1. Steuerpflichtige Einkünfte aus dem An- und Verkauf von Kryptowährungen gem. § 27b Abs 3 EStG",
    "1.2": "1.2. Steuerpflichtige Einkünfte aus Margin, Derivaten und Futures",
    "1.3": "1.3. Steuerpflichtige Einkünfte aus NFT-Spekulationsgeschäften",
    "2.1": "2.1. Steuerpflichtige laufende Einkünfte aus der Überlassung von Kryptowährungen",
    "2.2": "2.2. Steuerpflichtige laufende Einkünfte aus Leistungen zur Transaktionsverarbeitung",
    "3.1": "3.1. Nicht steuerbare Einkünfte aus Spekulationsgeschäften mit Kryptowährungen und NFTs",
    "3.2": "3.2. Nicht steuerbare Einkünfte gem. § 27b Abs 2 Z 2 Satz 2 EStG",
    "3.3": "3.3. Nicht steuerbare Steuergebühren und Rückerstattungen",
    "4.1": "4.1. Eingegangene Spenden/Trinkgeld",
    "4.2": "4.2. Ausgegangene Spenden/Schenkungen",
    "4.3": "4.3. Gestohlene, gehackte und verlorene Coins",
    "4.4": "4.4. Mining (kommerziell)",
    "4.5": "4.5. Minting",
}

AUSTRIAN_E1KV_XLSX_DISPOSAL_HEADERS = (
    "Börse",
    "Anlage",
    "Anzahl",
    "Erwerbsdatum",
    "Verkaufsdatum",
    "Kauf/Eingang bei",
    "Typ",
    "Kostenbasis in EUR",
    "Erlös in EUR",
    "Gewinn/Verlust in EUR",
)
AUSTRIAN_E1KV_XLSX_HOLDING_HEADERS = (
    "Börse",
    "Anlage",
    "Anzahl",
    "Erwerbsdatum",
    "Verkaufsdatum",
    "Kauf/Eingang bei",
    "Haltedauer in Tagen",
    "Typ",
    "Kostenbasis in EUR",
    "Erlös in EUR",
    "Gewinn/Verlust in EUR",
)
AUSTRIAN_E1KV_XLSX_INCOME_HEADERS = (
    "Börse",
    "Typ",
    "Datum des Eingangs",
    "Anzahl",
    "Währung",
    "Hinweis",
    "Wert in EUR zum Zeitpunkt des Eingangs",
)
AUSTRIAN_E1KV_XLSX_MARGIN_HEADERS = (
    "Börse",
    "Typ",
    "Datum",
    "Anzahl",
    "Währung",
    "Gesamt",
)
AUSTRIAN_E1KV_XLSX_FEE_HEADERS = (
    "Börse",
    "Datum der Gebühr",
    "Anzahl",
    "Währung",
    "Erlös in EUR",
    "Anzahl",
    "Währung",
    "Hinweis",
)
AUSTRIAN_E1KV_XLSX_OUTGOING_HEADERS = (
    "Börse",
    "Typ",
    "Datum der Auszahlung",
    "Anzahl",
    "Währung",
    "Hinweis",
    "Kostenbasis in EUR",
    "Wert bei Auszahlung in EUR",
)


def _austrian_e1kv_xlsx_formats(workbook):
    return {
        "overview_title": workbook.add_format(
            {"bold": True, "font_size": 16, "valign": "vcenter"}
        ),
        "overview_group": workbook.add_format(
            {"bold": True, "font_size": 12, "valign": "vcenter"}
        ),
        "overview_section": workbook.add_format(
            {"bold": True, "font_size": 11, "valign": "vcenter"}
        ),
        "overview_label": workbook.add_format({"font_size": 11, "valign": "vcenter"}),
        "overview_money": workbook.add_format(
            {"font_size": 11, "valign": "vcenter", "num_format": "#,##0.00"}
        ),
        "overview_currency": workbook.add_format({"font_size": 11, "valign": "vcenter"}),
        "overview_total_label": workbook.add_format(
            {"bold": True, "font_size": 11, "valign": "vcenter"}
        ),
        "overview_total_money": workbook.add_format(
            {"bold": True, "font_size": 11, "valign": "vcenter", "num_format": "#,##0.00"}
        ),
        "detail_title": workbook.add_format(
            {"bold": True, "font_size": 13, "align": "center", "valign": "vcenter", "text_wrap": True}
        ),
        "header": workbook.add_format(
            {"bold": True, "font_size": 11, "valign": "top", "text_wrap": True}
        ),
        "text": workbook.add_format({"font_size": 11, "valign": "top", "text_wrap": True}),
        "int": workbook.add_format({"font_size": 11, "valign": "top", "num_format": "0"}),
        "quantity": workbook.add_format(
            {"font_size": 11, "valign": "top", "num_format": "0.00000000"}
        ),
        "money": workbook.add_format(
            {"font_size": 11, "valign": "top", "num_format": "#,##0.00"}
        ),
        "total_label": workbook.add_format(
            {"bold": True, "font_size": 11, "valign": "vcenter", "text_wrap": True}
        ),
        "total_money": workbook.add_format(
            {"bold": True, "font_size": 11, "valign": "vcenter", "num_format": "#,##0.00"}
        ),
        "explanation_title": workbook.add_format(
            {"bold": True, "font_size": 16, "valign": "vcenter"}
        ),
        "explanation_heading": workbook.add_format(
            {"bold": True, "font_size": 12, "valign": "top"}
        ),
        "explanation_text": workbook.add_format(
            {"font_size": 11, "valign": "top", "text_wrap": True}
        ),
    }


def _xlsx_write_value(worksheet, row_index, column_index, value, cell_format):
    if value is None or value == "":
        worksheet.write_blank(row_index, column_index, None, cell_format)
    elif isinstance(value, bool):
        worksheet.write_boolean(row_index, column_index, value, cell_format)
    elif isinstance(value, (int, float, Decimal)):
        worksheet.write_number(row_index, column_index, float(value), cell_format)
    else:
        worksheet.write_string(row_index, column_index, str(value), cell_format)


def _austrian_e1kv_xlsx_hint(row):
    tx_id = str(row.get("tx_id") or "")
    note = str(row.get("note") or "")
    return tx_id or note


def _austrian_e1kv_xlsx_category(row):
    return row.get("at_category_label") or row.get("at_category") or row.get("kind") or ""


def _austrian_e1kv_xlsx_disposal_values(row, include_holding_days=False):
    values = [
        row.get("wallet") or "",
        row.get("asset") or "",
        row.get("quantity"),
        "",
        row.get("date") or "",
        "",
    ]
    if include_holding_days:
        values.append(row.get("holding_period_days"))
    values.extend(
        [
            _austrian_e1kv_xlsx_category(row),
            _xlsx_eur_from_cents(row.get("cost_basis_eur_cents")),
            _xlsx_eur_from_cents(row.get("proceeds_eur_cents")),
            _xlsx_eur_from_cents(row.get("gain_loss_eur_cents")),
        ]
    )
    return values


def _austrian_e1kv_xlsx_income_values(row):
    return [
        row.get("wallet") or "",
        row.get("kind") or row.get("entry_type") or "",
        row.get("date") or "",
        row.get("quantity"),
        row.get("asset") or "",
        _austrian_e1kv_xlsx_hint(row),
        _xlsx_eur_from_cents(row.get("form_amount_eur_cents")),
    ]


def _austrian_e1kv_render_cell(value, format_name):
    if value is None or value == "":
        return ""
    if format_name == "money":
        return _report_fiat(Decimal(str(value)))
    if format_name == "quantity":
        return f"{Decimal(str(value)):.8f}"
    if format_name == "int":
        return str(int(value))
    return str(value)


def _austrian_e1kv_render_table_row(values, format_names):
    return [
        _austrian_e1kv_render_cell(
            value,
            format_names[index] if index < len(format_names) else "text",
        )
        for index, value in enumerate(values)
    ]


def _austrian_e1kv_section_table_specs(report):
    sections = report["sections"]
    split_11 = _austrian_disposal_split(sections["1.1"]["detail_rows"])
    disposal_formats = ("text", "text", "quantity", "text", "text", "text", "text", "money", "money", "money")
    holding_formats = (
        "text",
        "text",
        "quantity",
        "text",
        "text",
        "text",
        "int",
        "text",
        "money",
        "money",
        "money",
    )
    income_formats = ("text", "text", "text", "quantity", "text", "text", "money")
    margin_formats = ("text", "text", "text", "quantity", "text", "money")
    fee_formats = ("text", "text", "quantity", "text", "money", "quantity", "text", "text")
    outgoing_formats = ("text", "text", "text", "quantity", "text", "text", "money", "money")
    return [
        {
            "sheet_name": "1.1.",
            "filename": "01_1.1.csv",
            "title": AUSTRIAN_E1KV_XLSX_TITLES["1.1"],
            "headers": AUSTRIAN_E1KV_XLSX_DISPOSAL_HEADERS,
            "rows": [_austrian_e1kv_xlsx_disposal_values(row) for row in sections["1.1"]["detail_rows"]],
            "row_format_names": disposal_formats,
            "total_rows": [
                ("Summe Einkünfte realisierten Wertsteigerungen", _xlsx_eur_from_cents(split_11["gains_amount"])),
                ("Summe realisierte Wertverluste", _xlsx_eur_from_cents(split_11["losses_amount"])),
            ],
            "value_column": 9,
            "column_widths": (18, 12, 14, 15, 15, 18, 24, 18, 18, 20),
        },
        {
            "sheet_name": "1.2.",
            "filename": "02_1.2.csv",
            "title": AUSTRIAN_E1KV_XLSX_TITLES["1.2"],
            "headers": AUSTRIAN_E1KV_XLSX_MARGIN_HEADERS,
            "rows": [],
            "row_format_names": margin_formats,
            "total_rows": [
                ("Summe Gewinne aus Margin, Derivaten, Futures", 0.0),
                ("Summe Verluste aus Margin, Derivaten, Futures", 0.0),
            ],
            "value_column": 5,
            "column_widths": (18, 22, 16, 14, 12, 18),
        },
        {
            "sheet_name": "1.3.",
            "filename": "03_1.3.csv",
            "title": AUSTRIAN_E1KV_XLSX_TITLES["1.3"],
            "headers": AUSTRIAN_E1KV_XLSX_HOLDING_HEADERS,
            "rows": [],
            "row_format_names": holding_formats,
            "total_rows": [("Summe Einkünfte aus NFT-Spekulationsgeschäften", 0.0)],
            "value_column": 10,
            "column_widths": (18, 12, 14, 15, 15, 18, 16, 24, 18, 18, 20),
        },
        *[
            {
                "sheet_name": f"{section_id}.",
                "filename": f"0{index}_{section_id}.csv",
                "title": AUSTRIAN_E1KV_XLSX_TITLES[section_id],
                "headers": AUSTRIAN_E1KV_XLSX_INCOME_HEADERS,
                "rows": [_austrian_e1kv_xlsx_income_values(row) for row in sections[section_id]["detail_rows"]],
                "row_format_names": income_formats,
                "total_rows": [
                    (
                        "Summe laufende Einkünfte",
                        _xlsx_eur_from_cents(sections[section_id]["totals"]["amount_eur_cents"]),
                    )
                ],
                "value_column": 6,
                "column_widths": (18, 24, 18, 14, 12, 36, 24),
            }
            for index, section_id in ((4, "2.1"), (5, "2.2"))
        ],
        {
            "sheet_name": "3.1.",
            "filename": "06_3.1.csv",
            "title": AUSTRIAN_E1KV_XLSX_TITLES["3.1"],
            "headers": AUSTRIAN_E1KV_XLSX_HOLDING_HEADERS,
            "rows": [
                _austrian_e1kv_xlsx_disposal_values(row, include_holding_days=True)
                for row in sections["3.1"]["detail_rows"]
            ],
            "row_format_names": holding_formats,
            "total_rows": [
                ("Summe nicht steuerbare Einkünfte", _xlsx_eur_from_cents(sections["3.1"]["totals"]["amount_eur_cents"]))
            ],
            "value_column": 10,
            "column_widths": (18, 12, 14, 15, 15, 18, 16, 24, 18, 18, 20),
        },
        {
            "sheet_name": "3.2.",
            "filename": "07_3.2.csv",
            "title": AUSTRIAN_E1KV_XLSX_TITLES["3.2"],
            "headers": AUSTRIAN_E1KV_XLSX_INCOME_HEADERS,
            "rows": [],
            "row_format_names": income_formats,
            "total_rows": [("Summe nicht steuerbare Einkünfte", 0.0)],
            "value_column": 6,
            "column_widths": (18, 24, 18, 14, 12, 36, 24),
        },
        {
            "sheet_name": "3.3.",
            "filename": "08_3.3.csv",
            "title": AUSTRIAN_E1KV_XLSX_TITLES["3.3"],
            "headers": AUSTRIAN_E1KV_XLSX_FEE_HEADERS,
            "rows": [],
            "row_format_names": fee_formats,
            "total_rows": [
                ("Summe entrichtete Steuergebühren", 0.0),
                ("Summe der Rückerstattungen", 0.0),
            ],
            "value_column": 4,
            "column_widths": (18, 18, 14, 12, 18, 14, 12, 32),
        },
        {
            "sheet_name": "4.1.",
            "filename": "09_4.1.csv",
            "title": AUSTRIAN_E1KV_XLSX_TITLES["4.1"],
            "headers": AUSTRIAN_E1KV_XLSX_INCOME_HEADERS,
            "rows": [],
            "row_format_names": income_formats,
            "total_rows": [("Summe Spenden/Trinkgeld", 0.0)],
            "value_column": 6,
            "column_widths": (18, 24, 18, 14, 12, 36, 24),
        },
        {
            "sheet_name": "4.2.",
            "filename": "10_4.2.csv",
            "title": AUSTRIAN_E1KV_XLSX_TITLES["4.2"],
            "headers": AUSTRIAN_E1KV_XLSX_OUTGOING_HEADERS,
            "rows": [],
            "row_format_names": outgoing_formats,
            "total_rows": [("Summe Spenden", 0.0), ("Summe Schenkungen", 0.0)],
            "value_column": 7,
            "column_widths": (18, 22, 18, 14, 12, 36, 18, 24),
        },
        {
            "sheet_name": "4.3.",
            "filename": "11_4.3.csv",
            "title": AUSTRIAN_E1KV_XLSX_TITLES["4.3"],
            "headers": AUSTRIAN_E1KV_XLSX_OUTGOING_HEADERS,
            "rows": [],
            "row_format_names": outgoing_formats,
            "total_rows": [("Summe gestohlen/gehackt/Betrug", 0.0), ("Summe Verlust", 0.0)],
            "value_column": 7,
            "column_widths": (18, 22, 18, 14, 12, 36, 18, 24),
        },
        *[
            {
                "sheet_name": f"{section_id}.",
                "filename": f"{index}_{section_id}.csv",
                "title": AUSTRIAN_E1KV_XLSX_TITLES[section_id],
                "headers": AUSTRIAN_E1KV_XLSX_INCOME_HEADERS,
                "rows": [],
                "row_format_names": income_formats,
                "total_rows": [(total_label, 0.0)],
                "value_column": 6,
                "column_widths": (18, 24, 18, 14, 12, 36, 24),
            }
            for index, section_id, total_label in (("12", "4.4", "Summe Mining"), ("13", "4.5", "Summe Minting"))
        ],
    ]


def _austrian_e1kv_xlsx_write_total_row(worksheet, row_index, label, value, value_column, formats):
    if value_column > 0:
        worksheet.merge_range(row_index, 0, row_index, value_column - 1, label, formats["total_label"])
    else:
        worksheet.write_string(row_index, 0, label, formats["total_label"])
    _xlsx_write_value(worksheet, row_index, value_column, value, formats["total_money"])


def _austrian_e1kv_xlsx_write_detail_sheet(
    workbook,
    sheet_name,
    title,
    headers,
    rows,
    row_format_names,
    total_rows,
    value_column,
    formats,
    column_widths,
):
    worksheet = workbook.add_worksheet(sheet_name)
    worksheet.set_landscape()
    worksheet.fit_to_pages(1, 0)
    worksheet.set_margins(left=0.35, right=0.35, top=0.5, bottom=0.5)

    last_column = len(headers) - 1
    worksheet.set_row(0, 31)
    worksheet.merge_range(0, 0, 0, last_column, title, formats["detail_title"])
    worksheet.set_row(1, 34)
    for column_index, header in enumerate(headers):
        width = column_widths[column_index] if column_index < len(column_widths) else 16
        worksheet.set_column(column_index, column_index, width)
        worksheet.write_string(1, column_index, header, formats["header"])

    row_index = 2
    if rows:
        for values in rows:
            worksheet.set_row(row_index, 22)
            for column_index, value in enumerate(values):
                format_name = row_format_names[column_index] if column_index < len(row_format_names) else "text"
                _xlsx_write_value(worksheet, row_index, column_index, value, formats[format_name])
            row_index += 1
    else:
        worksheet.set_row(row_index, 20)
        for column_index in range(len(headers)):
            worksheet.write_blank(row_index, column_index, None, formats["text"])
        row_index += 1

    row_index += 1
    for label, value in total_rows:
        worksheet.set_row(row_index, 22)
        _austrian_e1kv_xlsx_write_total_row(
            worksheet,
            row_index,
            label,
            value,
            value_column,
            formats,
        )
        row_index += 1
    return worksheet


def _austrian_e1kv_xlsx_write_overview(report, workbook, formats):
    worksheet = workbook.add_worksheet("Übersicht")
    worksheet.set_column(0, 0, 72)
    worksheet.set_column(1, 1, 20)
    worksheet.set_column(2, 2, 8)
    worksheet.set_margins(left=0.35, right=0.35, top=0.5, bottom=0.5)

    worksheet.set_row(0, 28)
    worksheet.merge_range(0, 0, 0, 2, "I. Übersicht", formats["overview_title"])
    row_index = 2
    for entry in _austrian_e1kv_overview_entries(report):
        worksheet.set_row(row_index, 25)
        if entry[0] == "heading":
            worksheet.merge_range(row_index, 0, row_index, 2, entry[1], formats["overview_group"])
            row_index += 2
        elif entry[0] == "section":
            worksheet.merge_range(row_index, 0, row_index, 2, entry[1], formats["overview_section"])
            row_index += 1
        else:
            _kind, label, cents, total = entry
            label_format = formats["overview_total_label"] if total else formats["overview_label"]
            value_format = formats["overview_total_money"] if total else formats["overview_money"]
            worksheet.write_string(row_index, 0, label, label_format)
            worksheet.write_number(row_index, 1, float(_eur_from_cents(cents)), value_format)
            worksheet.write_string(row_index, 2, "EUR", formats["overview_currency"])
            row_index += 1
    return worksheet


def _austrian_e1kv_xlsx_write_explanations(report, workbook, formats):
    worksheet = workbook.add_worksheet("Erläuterungen zum Steuerreport")
    worksheet.set_column(0, 0, 105)
    worksheet.set_margins(left=0.35, right=0.35, top=0.5, bottom=0.5)

    rows = [
        ("Erläuterungen zum Steuerreport", "explanation_title"),
        ("", "explanation_text"),
        ("Berichtsumfang", "explanation_heading"),
        (
            "Dieser XLSX-Export ist als jährliche Arbeitsunterlage für die österreichische "
            "E 1kv-Prüfung aufgebaut. Die Übersicht fasst die Beträge zusammen; die "
            "nummerierten Blätter enthalten die dazugehörigen Detailzeilen oder explizite "
            "Null-Platzhalter für noch nicht modellierte Bereiche.",
            "explanation_text",
        ),
        ("Prüfung", "explanation_heading"),
        (report["review_gate"], "explanation_text"),
        ("Aktuelle Kassiber-Annahmen", "explanation_heading"),
    ]
    rows.extend((f"{row['code']}: {row['message']}", "explanation_text") for row in report["assumptions"])
    rows.extend(
        [
            ("Datenqualität", "explanation_heading"),
            (
                f"Quarantäne-Gründe im Jahr: {len(report['data_quality']['quarantines'])}; "
                f"abweichende gespeicherte Kennzahlen: {len(report['data_quality']['kennzahl_mismatches'])}. "
                "Quarantinierte Transaktionen bleiben außerhalb dieser Arbeitsmappe, bis sie aufgelöst sind.",
                "explanation_text",
            ),
        ]
    )
    mismatches = _austrian_e1kv_mismatch_table_rows(report)
    if mismatches:
        rows.append(("Kennzahl-Abweichungen", "explanation_heading"))
        rows.append(("Transaktion | Kategorie | gespeichert | Export", "explanation_text"))
        rows.extend(
            (
                f"{tx_id} | {category} | {stored_kennzahl} | {export_kennzahl}",
                "explanation_text",
            )
            for tx_id, category, stored_kennzahl, export_kennzahl in mismatches
        )
    rows.extend(
        [
            ("Nicht modellierte Blätter", "explanation_heading"),
            (
                "Margin/Derivate/Futures, NFT-Spekulationsgeschäfte, Steuergebühren, "
                "Spenden/Schenkungen, verlorene Coins, kommerzielles Mining und Minting "
                "werden heute als leere Nullabschnitte dargestellt, weil Kassiber dafür "
                "noch keine strukturierten Steuerereignisse speichert.",
                "explanation_text",
            ),
        ]
    )
    for row_index, (text, format_name) in enumerate(rows):
        worksheet.set_row(row_index, 26 if format_name.endswith("heading") or format_name.endswith("title") else 46)
        if text:
            worksheet.write_string(row_index, 0, text, formats[format_name])
        else:
            worksheet.write_blank(row_index, 0, None, formats[format_name])
    return worksheet


def _austrian_e1kv_xlsx_write_section_sheets(report, workbook, formats):
    for spec in _austrian_e1kv_section_table_specs(report):
        _austrian_e1kv_xlsx_write_detail_sheet(
            workbook,
            spec["sheet_name"],
            spec["title"],
            spec["headers"],
            spec["rows"],
            spec["row_format_names"],
            spec["total_rows"],
            spec["value_column"],
            formats,
            spec["column_widths"],
        )


def export_austrian_e1kv_xlsx_report(conn, workspace_ref, profile_ref, file_path, hooks: ReportHooks, tax_year=None):
    import xlsxwriter

    report = report_austrian_e1kv(conn, workspace_ref, profile_ref, hooks, tax_year=tax_year)
    path = Path(file_path).expanduser()
    path.parent.mkdir(parents=True, exist_ok=True)

    workbook = xlsxwriter.Workbook(str(path))
    workbook.set_properties(
        {
            "title": f"Kassiber Austrian E 1kv Report - {report['profile']} ({report['tax_year']})",
            "subject": "Austrian E 1kv cryptocurrency tax handoff",
            "author": "Kassiber",
            "comments": AUSTRIAN_E1KV_REVIEW_GATE,
        }
    )
    formats = _austrian_e1kv_xlsx_formats(workbook)
    _austrian_e1kv_xlsx_write_overview(report, workbook, formats)
    _austrian_e1kv_xlsx_write_section_sheets(report, workbook, formats)
    _austrian_e1kv_xlsx_write_explanations(report, workbook, formats)
    workbook.close()
    return {
        "file": str(path.resolve()),
        "bytes": path.stat().st_size,
        "form": report["form"],
        "tax_year": report["tax_year"],
        "sheets": list(AUSTRIAN_E1KV_XLSX_SHEETS),
        "rows": len(report["rows"]),
        "summary_rows": len(report["summary_rows"]),
    }


def _write_csv_rows(path, rows):
    with path.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.writer(handle)
        writer.writerows(rows)


def _austrian_e1kv_overview_csv_rows(report):
    rows = [["I. Übersicht"]]
    for entry in _austrian_e1kv_overview_entries(report):
        if entry[0] == "heading":
            rows.extend([[], [entry[1]]])
        elif entry[0] == "section":
            rows.append([entry[1]])
        else:
            _kind, label, cents, _total = entry
            rows.append([label, _report_eur_cents(cents), "EUR"])
    return rows


def _austrian_e1kv_detail_csv_rows(spec):
    rows = [[spec["title"]], list(spec["headers"])]
    if spec["rows"]:
        rows.extend(
            _austrian_e1kv_render_table_row(row, spec["row_format_names"])
            for row in spec["rows"]
        )
    else:
        rows.append(["No rows in scope."])
    rows.append([])
    total_width = max(len(spec["headers"]), spec["value_column"] + 1)
    for label, value in spec["total_rows"]:
        total_row = [""] * total_width
        total_row[0] = label
        total_row[spec["value_column"]] = _austrian_e1kv_render_cell(value, "money")
        rows.append(total_row)
    return rows


def _austrian_e1kv_explanation_csv_rows(report):
    rows = [
        ["Erläuterungen zum Steuerreport"],
        [],
        ["Berichtsumfang"],
        [
            "Dieser CSV-Bundle-Export ist als jährliche Arbeitsunterlage für die "
            "österreichische E 1kv-Prüfung aufgebaut. Die Übersicht fasst die "
            "Beträge zusammen; die nummerierten Dateien enthalten Detailzeilen "
            "oder explizite Null-Platzhalter für noch nicht modellierte Bereiche."
        ],
        ["Prüfung"],
        [report["review_gate"]],
        ["Aktuelle Kassiber-Annahmen"],
    ]
    rows.extend([[f"{row['code']}: {row['message']}"] for row in report["assumptions"]])
    rows.extend(
        [
            ["Datenqualität"],
            [
                f"Quarantäne-Gründe im Jahr: {len(report['data_quality']['quarantines'])}; "
                f"abweichende gespeicherte Kennzahlen: {len(report['data_quality']['kennzahl_mismatches'])}. "
                "Quarantinierte Transaktionen bleiben außerhalb dieses Bundles, bis sie aufgelöst sind."
            ],
        ]
    )
    mismatches = _austrian_e1kv_mismatch_table_rows(report)
    if mismatches:
        rows.extend(
            [
                ["Kennzahl-Abweichungen"],
                ["Transaktion", "Kategorie", "Gespeicherte KZ", "Export KZ"],
                *mismatches,
            ]
        )
    rows.extend(
        [
            ["Nicht modellierte Dateien"],
            [
                "Margin/Derivate/Futures, NFT-Spekulationsgeschäfte, Steuergebühren, "
                "Spenden/Schenkungen, verlorene Coins, kommerzielles Mining und Minting "
                "werden heute als leere Nullabschnitte dargestellt, weil Kassiber dafür "
                "noch keine strukturierten Steuerereignisse speichert."
            ],
        ]
    )
    return rows


def export_austrian_e1kv_csv_bundle(conn, workspace_ref, profile_ref, dir_path, hooks: ReportHooks, tax_year=None):
    report = report_austrian_e1kv(conn, workspace_ref, profile_ref, hooks, tax_year=tax_year)
    directory = Path(dir_path).expanduser()
    directory.mkdir(parents=True, exist_ok=True)

    files = []

    def write_file(sheet_name, filename, rows):
        path = directory / filename
        _write_csv_rows(path, rows)
        files.append(
            {
                "sheet": sheet_name,
                "file": str(path.resolve()),
                "bytes": path.stat().st_size,
                "rows": max(len(rows) - 1, 0),
            }
        )

    write_file("Übersicht", "00_uebersicht.csv", _austrian_e1kv_overview_csv_rows(report))
    for spec in _austrian_e1kv_section_table_specs(report):
        write_file(spec["sheet_name"], spec["filename"], _austrian_e1kv_detail_csv_rows(spec))
    write_file(
        "Erläuterungen zum Steuerreport",
        "99_erlaeuterungen_zum_steuerreport.csv",
        _austrian_e1kv_explanation_csv_rows(report),
    )
    return {
        "dir": str(directory.resolve()),
        "form": report["form"],
        "tax_year": report["tax_year"],
        "sheets": list(AUSTRIAN_E1KV_XLSX_SHEETS),
        "files": files,
        "rows": len(report["rows"]),
        "summary_rows": len(report["summary_rows"]),
    }


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
    "build_austrian_e1kv_report_lines",
    "build_pdf_report_lines",
    "export_austrian_e1kv_csv_bundle",
    "export_austrian_e1kv_pdf_report",
    "export_austrian_e1kv_xlsx_report",
    "export_pdf_report",
    "report_austrian_e1kv",
    "report_balance_history",
    "report_balance_sheet",
    "report_capital_gains",
    "report_journal_entries",
    "report_portfolio_summary",
    "build_summary_report_lines",
    "report_summary",
    "report_tax_summary",
]
