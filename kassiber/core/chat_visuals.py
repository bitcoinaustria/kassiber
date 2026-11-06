"""Visual projections for the in-app assistant.

The assistant chooses *what* to show (a source, a measure, filters, a title);
every figure comes from the same local reads the rest of the app uses. These
functions turn those payloads into a typed visual specification, long-format
``rows`` (which the CLI prints as a table and the desktop's table view shows),
and source-specific extras such as KPI ``stats`` or a checklist. The desktop
draws them; neither surface ever plots a number the model typed itself.

A visual is display only. It never grants ownership, carries basis or decides
tax facts, and it states the gaps it knows about (stale journals, quarantines,
missing prices, truncation) as ``caveats`` instead of plotting them as zero.

``model_digest`` is what the provider receives instead of the full payload: the
specification, a few summary figures and the caveats, plus the rows themselves
only while they are few. The desktop and CLI still get every row.
"""

from __future__ import annotations

import unicodedata
from datetime import datetime, timezone
from typing import Any, Mapping, Sequence

from ..errors import AppError
from . import reports as core_reports

VISUAL_VERSION = 1

VISUAL_SOURCES = (
    "overview",
    "balance_history",
    "portfolio",
    "tax_summary",
    "cash_flow",
    "transactions",
    "transaction_flow",
    "readiness",
)

# The first measure is each source's default. Every row carries all of a
# source's measures, so the desktop can switch between them without a new read.
VISUAL_MEASURES: dict[str, tuple[str, ...]] = {
    "overview": ("market_value",),
    "balance_history": ("quantity", "market_value", "cost_basis"),
    "portfolio": ("market_value", "quantity", "cost_basis", "unrealized_pnl"),
    "tax_summary": ("gain_loss", "proceeds", "cost_basis"),
    "cash_flow": ("quantity", "fiat_value"),
    "transactions": ("amount",),
    "transaction_flow": ("amount",),
    "readiness": ("status",),
}

# Filters each source accepts, besides the shared ``source``, ``title`` and
# ``measure``. The tool schema lists their union; this map keeps one source's
# filter from silently doing nothing on another.
_SOURCE_ARGUMENTS: dict[str, frozenset[str]] = {
    "overview": frozenset(),
    "balance_history": frozenset({"interval", "start", "end", "wallet", "asset", "limit"}),
    "portfolio": frozenset({"group_by", "asset"}),
    "tax_summary": frozenset(),
    "cash_flow": frozenset({"period", "wallet"}),
    "transactions": frozenset(
        {"query", "direction", "wallet", "period", "status", "sort", "limit"}
    ),
    "transaction_flow": frozenset({"transaction"}),
    "readiness": frozenset(),
}

BALANCE_HISTORY_INTERVALS = ("hour", "day", "week", "month")
DEFAULT_BALANCE_HISTORY_INTERVAL = "month"
DEFAULT_BALANCE_HISTORY_LIMIT = 60
MAX_BALANCE_HISTORY_LIMIT = 240
BALANCE_HISTORY_REPORT_ROWS = 500
PORTFOLIO_GROUPS = ("wallet", "asset")
MAX_PORTFOLIO_ROWS = 60
CASH_FLOW_PERIODS = ("30days", "3months", "6months", "ytd", "1year", "5years", "all")
DEFAULT_CASH_FLOW_PERIOD = "1year"
TRANSACTION_PERIODS = CASH_FLOW_PERIODS
TRANSACTION_DIRECTIONS = ("inbound", "outbound")
TRANSACTION_STATUSES = ("completed", "pending", "failed", "review")
TRANSACTION_SORTS = ("recent", "oldest", "largest", "smallest")
DEFAULT_TRANSACTION_LIMIT = 10
MAX_TRANSACTION_LIMIT = 25
MAX_OVERVIEW_SERIES_POINTS = 160
MAX_OVERVIEW_WALLETS = 6
MAX_TAX_DETAILS = 200
MAX_NOTE_CHARS = 80
MAX_TITLE_CHARS = 80
MAX_FILTER_CHARS = 200
# The model sees every row up to this many; above it, only the digest.
MODEL_ROW_LIMIT = 30

# Fiat measures read the book's fiat currency; quantities keep their asset.
_FIAT_MEASURES = frozenset(
    {"market_value", "cost_basis", "unrealized_pnl", "gain_loss", "proceeds", "fiat_value"}
)
_BTC_ASSETS = frozenset({"BTC", "LBTC", "L-BTC", "LNBTC"})
# Report sources read journal-derived state; their readiness belongs on the chart.
JOURNAL_SOURCES = frozenset({"overview", "balance_history", "portfolio", "tax_summary"})


def _validation(message: str, **details: Any) -> AppError:
    return AppError(message, code="validation", details=details or None, retryable=False)


def _clean_text(value: Any, field: str, *, limit: int) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _validation(f"ui.visuals.render {field} must be a string", field=field)
    # Control and format characters (bidi overrides included) have no place in
    # a chart label the model wrote.
    text = "".join(
        " " if char.isspace() else char
        for char in value
        if char.isspace() or unicodedata.category(char)[0] != "C"
    ).strip()
    text = " ".join(text.split())
    if len(text) > limit:
        raise _validation(
            f"ui.visuals.render {field} is too long", field=field, max_length=limit
        )
    return text or None


def _enum(args: Mapping[str, Any], key: str, choices: Sequence[str], default: str | None) -> str | None:
    value = args.get(key, default)
    if value is None:
        return None
    if value not in choices:
        raise _validation(
            f"ui.visuals.render {key} is unsupported",
            **{key: value},
            supported=list(choices),
        )
    return str(value)


def _bounded_int(args: Mapping[str, Any], key: str, default: int, maximum: int) -> int:
    value = args.get(key, default)
    if isinstance(value, bool) or not isinstance(value, int) or not 1 <= value <= maximum:
        raise _validation(
            f"ui.visuals.render {key} must be an integer between 1 and {maximum}",
            **{key: value},
        )
    return value


def normalize_visual_args(raw_args: Mapping[str, Any] | None) -> dict[str, Any]:
    """Validate the model's request and split it into chart and read arguments."""

    args = dict(raw_args or {})
    source = args.get("source")
    if source not in VISUAL_SOURCES:
        raise _validation(
            "ui.visuals.render source is unsupported",
            source=source,
            supported=list(VISUAL_SOURCES),
        )
    allowed = {"source", "title", "measure"} | _SOURCE_ARGUMENTS[source]
    unknown = sorted(set(args) - allowed)
    if unknown:
        raise _validation(
            f"ui.visuals.render does not accept these fields for {source}",
            unknown=unknown,
            source=source,
        )
    measures = VISUAL_MEASURES[source]
    measure = args.get("measure", measures[0])
    if measure not in measures:
        raise _validation(
            f"ui.visuals.render measure is unsupported for {source}",
            measure=measure,
            supported=list(measures),
        )
    spec: dict[str, Any] = {
        "source": source,
        "measure": measure,
        "title": _clean_text(args.get("title"), "title", limit=MAX_TITLE_CHARS),
    }
    report_args: dict[str, Any] = {}

    def text(key: str) -> str | None:
        return _clean_text(args.get(key), key, limit=MAX_FILTER_CHARS)

    if source == "balance_history":
        interval = _enum(args, "interval", BALANCE_HISTORY_INTERVALS, DEFAULT_BALANCE_HISTORY_INTERVAL)
        limit = _bounded_int(args, "limit", DEFAULT_BALANCE_HISTORY_LIMIT, MAX_BALANCE_HISTORY_LIMIT)
        # The report truncates by row (one per asset and bucket); read as much
        # as it allows and keep whole buckets here instead.
        report_args = {"interval": interval, "limit": BALANCE_HISTORY_REPORT_ROWS}
        for key in ("start", "end", "wallet", "asset"):
            value = text(key)
            if value is not None:
                report_args[key] = value
        spec["interval"] = interval
        spec["limit"] = limit
    elif source == "portfolio":
        spec["group_by"] = _enum(args, "group_by", PORTFOLIO_GROUPS, "wallet")
        asset = text("asset")
        if asset is not None:
            spec["asset"] = asset
    elif source == "cash_flow":
        spec["period"] = _enum(args, "period", CASH_FLOW_PERIODS, DEFAULT_CASH_FLOW_PERIOD)
        report_args = {"period": spec["period"]}
        wallet = text("wallet")
        if wallet is not None:
            report_args["wallet"] = wallet
    elif source == "transactions":
        sort = _enum(args, "sort", TRANSACTION_SORTS, "recent")
        spec["sort"] = sort
        spec["limit"] = _bounded_int(args, "limit", DEFAULT_TRANSACTION_LIMIT, MAX_TRANSACTION_LIMIT)
        filters: dict[str, Any] = {}
        for key in ("query", "wallet"):
            value = text(key)
            if value is not None:
                filters[key] = value
        direction = _enum(args, "direction", TRANSACTION_DIRECTIONS, None)
        if direction:
            filters["direction"] = direction
        status = _enum(args, "status", TRANSACTION_STATUSES, None)
        if status:
            filters["status"] = status
        period = _enum(args, "period", TRANSACTION_PERIODS, None)
        if period and period != "all":
            filters["period"] = period
        report_args = filters
    elif source == "transaction_flow":
        transaction = text("transaction")
        if transaction is None:
            raise _validation("ui.visuals.render transaction_flow requires a transaction")
        report_args = {"transaction": transaction}
    spec["report_args"] = report_args
    return spec


# ---------------------------------------------------------------------------
# Shared helpers


def _unit(measure: str, fiat_currency: str) -> dict[str, Any]:
    if measure in _FIAT_MEASURES:
        return {"kind": "fiat", "code": fiat_currency}
    if measure == "amount":
        return {"kind": "sats", "code": "sat"}
    if measure == "status":
        return {"kind": "count", "code": None}
    # Quantities stay in each row's own asset; the series key names it.
    return {"kind": "asset", "code": None}


def _number(value: Any) -> float | None:
    if isinstance(value, bool) or value is None:
        return None
    try:
        number = float(value)
    except (TypeError, ValueError):
        return None
    return number if number == number and abs(number) != float("inf") else None


def _int(value: Any) -> int | None:
    if isinstance(value, bool) or value is None:
        return None
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def readiness_from_blockers(payload: Mapping[str, Any] | None) -> dict[str, Any]:
    """What the report blockers say about the figures a report visual draws."""

    payload = payload or {}
    ids = {
        str(blocker.get("id"))
        for blocker in payload.get("blockers") or []
        if isinstance(blocker, Mapping)
    }
    health = payload.get("health") or {}
    journals = health.get("journals") or {}
    rates = (payload.get("rates_coverage") or {}).get("summary") or {}
    return {
        "journals_stale": "journals_stale" in ids or bool(journals.get("needs_processing")),
        "quarantine": int(journals.get("quarantine_count") or 0),
        "custody_unresolved": "custody_quantity_unresolved" in ids,
        "unpriced_transactions": int(rates.get("missing_price_transactions") or 0),
        "sync_failed": "sync_failed" in ids,
    }


def _readiness_caveats(readiness: Mapping[str, Any] | None) -> list[dict[str, Any]]:
    if not readiness:
        return []
    caveats: list[dict[str, Any]] = []
    if readiness.get("journals_stale"):
        caveats.append({"code": "journals_stale"})
    quarantines = int(readiness.get("quarantine") or 0)
    if quarantines:
        caveats.append({"code": "quarantines", "count": quarantines})
    if readiness.get("custody_unresolved"):
        caveats.append({"code": "custody_unresolved"})
    unpriced = int(readiness.get("unpriced_transactions") or 0)
    if unpriced:
        caveats.append({"code": "unpriced_transactions", "count": unpriced})
    if readiness.get("sync_failed"):
        caveats.append({"code": "sync_failed"})
    return caveats


def _header(
    spec: Mapping[str, Any],
    *,
    chart: str,
    fiat_currency: str,
    **extra: Any,
) -> dict[str, Any]:
    return {
        "version": VISUAL_VERSION,
        "source": spec["source"],
        "chart": chart,
        "measure": spec["measure"],
        "measures": list(VISUAL_MEASURES[spec["source"]]),
        "title": spec.get("title"),
        "unit": _unit(spec["measure"], fiat_currency),
        "fiat_currency": fiat_currency,
        **extra,
    }


def _downsample(rows: list[Any], limit: int) -> list[Any]:
    """Keep ``limit`` evenly spaced points, always the first and the last."""

    if len(rows) <= limit:
        return rows
    step = (len(rows) - 1) / (limit - 1)
    return [rows[round(index * step)] for index in range(limit)]


# ---------------------------------------------------------------------------
# Sources


def _overview_visual(spec, payload, fiat_currency, context):
    fiat = payload.get("fiat") or {}
    completeness = fiat.get("completeness") or {}
    balance = payload.get("balanceSummary") or {}
    market_missing = bool(completeness.get("marketRateMissing"))
    basis_complete = bool(completeness.get("costBasisComplete", False))
    price = payload.get("marketRate") or {}
    stats = {
        # Without a market rate the overview's balance is valued at a
        # transaction rate; it is not a market value, so it is not shown as one.
        "portfolio_value": None if market_missing else _number(fiat.get("eurBalance")),
        "btc_balance": _number(balance.get("totalBtc")),
        "cost_basis": _number(fiat.get("eurCostBasis")),
        "unrealized": None if market_missing else _number(fiat.get("eurUnrealized")),
        "realized_ytd": _number(fiat.get("eurRealizedYTD")),
        "btc_price": None if market_missing else _number(price.get("rate")),
        "price_at": price.get("timestamp"),
        "basis_complete": basis_complete,
        "transactions": _int((payload.get("status") or {}).get("transactionCount")),
    }
    series = [
        {
            "date": point.get("date"),
            "balance_btc": _number(point.get("balanceBtc")),
            "market_value": _number(point.get("valueEur")),
            "cost_basis": _number(point.get("costBasisEur")),
        }
        for point in payload.get("portfolioSeries") or []
        if isinstance(point, Mapping) and point.get("date")
    ]
    series = _downsample(series, MAX_OVERVIEW_SERIES_POINTS)
    connections = sorted(
        (
            {
                "label": str(conn.get("label") or ""),
                "balance_btc": _number(conn.get("balance")) or 0.0,
            }
            for conn in payload.get("connections") or []
            if isinstance(conn, Mapping)
        ),
        key=lambda item: (-item["balance_btc"], item["label"]),
    )
    total = sum(max(item["balance_btc"], 0.0) for item in connections)
    wallets = [
        {**item, "share": (max(item["balance_btc"], 0.0) / total) if total else 0.0}
        for item in connections[:MAX_OVERVIEW_WALLETS]
    ]
    rest = connections[MAX_OVERVIEW_WALLETS:]
    if rest:
        rest_total = sum(max(item["balance_btc"], 0.0) for item in rest)
        wallets.append(
            {
                "label": None,
                "other_count": len(rest),
                "balance_btc": rest_total,
                "share": rest_total / total if total else 0.0,
            }
        )
    caveats: list[dict[str, Any]] = []
    reasons = set(completeness.get("reasons") or [])
    if "journals_stale" in reasons:
        caveats.append({"code": "journals_stale"})
    if completeness.get("quarantineCount"):
        caveats.append({"code": "quarantines", "count": int(completeness["quarantineCount"])})
    if "custody_unresolved" in reasons:
        caveats.append({"code": "custody_unresolved"})
    if completeness.get("missingPriceCount"):
        caveats.append({"code": "missing_prices", "count": int(completeness["missingPriceCount"])})
    if market_missing:
        caveats.append({"code": "market_rate_missing"})
    if not basis_complete and not caveats:
        caveats.append({"code": "basis_incomplete"})
    rows = [
        {"metric": key, "value": value}
        for key, value in stats.items()
        if key not in {"basis_complete", "price_at"}
    ]
    visual = _header(spec, chart="dashboard", fiat_currency=fiat_currency)
    extras = {
        "stats": stats,
        "series": series,
        "wallets": wallets,
        "completeness": {
            "state": completeness.get("state"),
            "basis_complete": basis_complete,
        },
    }
    return visual, rows, caveats, extras


def _parse_period(value: Any) -> datetime | None:
    if not isinstance(value, str) or not value:
        return None
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return None
    return parsed if parsed.tzinfo else parsed.replace(tzinfo=timezone.utc)


def _period_text(value: datetime) -> str:
    return value.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _calendar_periods(first: datetime, last: datetime, interval: str) -> list[str]:
    """Every bucket start from ``first`` to ``last``, the way the report steps."""

    periods: list[str] = []
    cursor = core_reports._floor_to_interval(first, interval)
    end = core_reports._floor_to_interval(last, interval)
    while cursor <= end and len(periods) <= MAX_BALANCE_HISTORY_LIMIT * 40:
        periods.append(_period_text(cursor))
        cursor = core_reports._next_interval(cursor, interval)
    return periods


def _balance_history_visual(spec, payload, fiat_currency, context):
    interval = spec.get("interval") or DEFAULT_BALANCE_HISTORY_INTERVAL
    source_rows = [row for row in payload.get("rows") or [] if _parse_period(row.get("period_start"))]
    by_key: dict[tuple[str, str], Mapping[str, Any]] = {}
    assets: list[str] = []
    for row in source_rows:
        period = _period_text(_parse_period(row["period_start"]))
        asset = str(row.get("asset") or "")
        if asset not in assets:
            assets.append(asset)
        by_key[(period, asset)] = row
    summary = payload.get("summary") or {}
    filters = payload.get("filters") or {}
    truncated = bool(summary.get("truncated"))
    present = sorted({period for period, _asset in by_key})
    periods: list[str] = []
    if present:
        first = _parse_period(present[0])
        if truncated and len(present) > 1:
            # The report kept its latest rows, so its oldest bucket may have
            # lost some assets. Start after it rather than plot a partial one.
            first = _parse_period(present[1])
        elif not truncated:
            # A start before the first movement shows the empty months too.
            start = _parse_period(filters.get("start"))
            if start and start < first:
                first = start
        # The report leaves out every bucket in which all balances are zero,
        # so the calendar comes from the range, not from the rows: a full exit
        # must fall to zero rather than end on the last held balance.
        last = _parse_period(filters.get("end")) or (context or {}).get("now") or datetime.now(timezone.utc)
        periods = _calendar_periods(first, max(last, _parse_period(present[-1])), interval)
    limit = int(spec.get("limit") or DEFAULT_BALANCE_HISTORY_LIMIT)
    if len(periods) > limit:
        truncated = True
        periods = periods[-limit:]
    kept = set(periods)
    # Series in the order they appear within the buckets shown.
    assets = list(dict.fromkeys(asset for period, asset in by_key if period in kept)) or assets
    rows: list[dict[str, Any]] = []
    missing_prices = 0
    for period in periods:
        for asset in assets:
            row = by_key.get((period, asset))
            if row is None:
                # No row is a zero balance: every measure of it is zero.
                rows.append(
                    {"period": period, "asset": asset, "quantity": 0.0, "market_value": 0.0, "cost_basis": 0.0}
                )
                continue
            quantity = _number(row.get("quantity"))
            market_value = _number(row.get("market_value"))
            # The report values a bucket at rate 0 when it has no price, so a
            # held balance worth exactly zero is a missing price, not zero.
            if quantity and not market_value:
                market_value = None
                missing_prices += 1
            rows.append(
                {
                    "period": period,
                    "asset": asset,
                    "quantity": quantity,
                    "market_value": market_value,
                    "cost_basis": _number(row.get("cumulative_cost_basis")),
                }
            )
    caveats: list[dict[str, Any]] = []
    if missing_prices:
        caveats.append({"code": "missing_prices", "count": missing_prices})
    if truncated:
        caveats.append({"code": "truncated"})
    visual = _header(
        spec,
        chart="line",
        fiat_currency=fiat_currency,
        x_key="period",
        series_key="asset",
        interval=interval,
        filters={k: filters.get(k) for k in ("start", "end", "wallet", "asset") if filters.get(k)},
    )
    return visual, rows, caveats, {}


def _portfolio_visual(spec, payload, fiat_currency, context):
    asset_filter = spec.get("asset")
    rows: list[dict[str, Any]] = []
    missing_prices = 0
    for row in payload.get("rows") or []:
        asset = str(row.get("asset") or "")
        if asset_filter and asset.upper() != asset_filter.upper():
            continue
        quantity = _number(row.get("quantity"))
        market_value = _number(row.get("market_value"))
        unrealized = _number(row.get("unrealized_pnl"))
        if quantity and not market_value:
            # No price: neither the value nor the unrealized result is known.
            market_value = None
            unrealized = None
            missing_prices += 1
        rows.append(
            {
                "wallet": str(row.get("wallet") or ""),
                "asset": asset,
                "quantity": quantity,
                "market_value": market_value,
                "cost_basis": _number(row.get("cost_basis")),
                "unrealized_pnl": unrealized,
            }
        )
    rows.sort(key=lambda row: (-(abs(row["market_value"] or 0.0)), row["wallet"], row["asset"]))
    caveats: list[dict[str, Any]] = []
    if missing_prices:
        caveats.append({"code": "missing_prices", "count": missing_prices})
    if len(rows) > MAX_PORTFOLIO_ROWS:
        caveats.append({"code": "truncated", "count": len(rows) - MAX_PORTFOLIO_ROWS})
        rows = rows[:MAX_PORTFOLIO_ROWS]
    visual = _header(
        spec,
        chart="allocation",
        fiat_currency=fiat_currency,
        group_by=spec["group_by"],
        filters={"asset": asset_filter} if asset_filter else {},
    )
    return visual, rows, caveats, {}


def _tax_summary_visual(spec, payload, fiat_currency, context):
    rows = [
        {
            "year": int(row["year"]),
            "proceeds": _number(row.get("proceeds")),
            "cost_basis": _number(row.get("cost_basis")),
            "gain_loss": _number(row.get("gain_loss")),
        }
        for row in payload.get("rows") or []
        if row.get("row_type") == "year_total" and _int(row.get("year")) is not None
    ]
    details = [
        {
            "year": int(row["year"]),
            "asset": row.get("asset") or "",
            "transaction_type": row.get("transaction_type") or "",
            "capital_gains_type": row.get("capital_gains_type") or "",
            "quantity": _number(row.get("quantity")),
            "proceeds": _number(row.get("proceeds")),
            "cost_basis": _number(row.get("cost_basis")),
            "gain_loss": _number(row.get("gain_loss")),
        }
        for row in payload.get("rows") or []
        if row.get("row_type") == "detail" and _int(row.get("year")) is not None
    ]
    caveats: list[dict[str, Any]] = []
    if len(details) > MAX_TAX_DETAILS:
        caveats.append({"code": "truncated", "count": len(details) - MAX_TAX_DETAILS})
        details = details[-MAX_TAX_DETAILS:]
    visual = _header(spec, chart="years", fiat_currency=fiat_currency, x_key="year")
    return visual, rows, caveats, {"details": details}


def _flow_stats(stats: Mapping[str, Any] | None) -> dict[str, Any]:
    stats = stats or {}
    largest = stats.get("largest") if isinstance(stats.get("largest"), Mapping) else None
    return {
        "count": int(stats.get("count") or 0),
        "btc": _number(stats.get("btc")) or 0.0,
        "fiat": _number(stats.get("eur")) or 0.0,
        "missing_prices": int(stats.get("missingPrice") or 0),
        "largest": (
            {
                "label": str(largest.get("label") or ""),
                "btc": _number(largest.get("btc")),
                "fiat": _number(largest.get("eur")),
            }
            if largest
            else None
        ),
    }


def _cash_flow_visual(spec, payload, fiat_currency, context):
    rows: list[dict[str, Any]] = []
    missing_prices = 0
    for bucket in payload.get("buckets") or []:
        stats = bucket.get("stats") or {}
        incoming = _flow_stats(stats.get("incoming"))
        outgoing = _flow_stats(stats.get("outgoing"))
        missing = incoming["missing_prices"] + outgoing["missing_prices"]
        missing_prices += missing
        rows.append(
            {
                "period": bucket.get("bucketStart"),
                "bucket": bucket.get("bucketKey"),
                "in_btc": incoming["btc"],
                "out_btc": outgoing["btc"],
                "in_fiat": incoming["fiat"],
                "out_fiat": outgoing["fiat"],
                "in_count": incoming["count"],
                "out_count": outgoing["count"],
                "transfer_count": int((stats.get("transfers") or {}).get("count") or 0),
                "swap_count": int((stats.get("swaps") or {}).get("count") or 0),
                "missing_prices": missing,
            }
        )
    summary = payload.get("summary") or {}
    totals = {
        "incoming": _flow_stats(summary.get("incoming")),
        "outgoing": _flow_stats(summary.get("outgoing")),
        "transfers": _flow_stats(summary.get("transfers")),
        "swaps": _flow_stats(summary.get("swaps")),
        "review": int(summary.get("review") or 0),
        "pending": int(summary.get("pending") or 0),
    }
    caveats: list[dict[str, Any]] = []
    if missing_prices:
        # The page counts an unpriced movement as zero fiat; say so.
        caveats.append({"code": "missing_prices", "count": missing_prices})
    window = payload.get("window") or {}
    visual = _header(
        spec,
        chart="cash_flow",
        fiat_currency=fiat_currency,
        x_key="period",
        period=spec.get("period"),
        bucket=_cash_flow_bucket(spec.get("period")),
        window={"since": window.get("since"), "until": window.get("until")},
        filters={"wallet": spec["report_args"].get("wallet")} if spec["report_args"].get("wallet") else {},
    )
    return visual, rows, caveats, {"totals": totals}


def _cash_flow_bucket(period: str | None) -> str:
    if period == "30days":
        return "day"
    if period == "3months":
        return "week"
    if period in {"5years", "all"}:
        return "quarter"
    return "month"


def _transaction_row(tx: Mapping[str, Any]) -> dict[str, Any]:
    # Lightning amounts can be fractions of a sat; the sign must survive.
    amount = _number(tx.get("amountSat")) or 0.0
    note = str(tx.get("note") or "")
    return {
        "id": str(tx.get("id") or ""),
        "date": tx.get("occurredAt") or tx.get("date"),
        "direction": "in" if amount > 0 else "out" if amount < 0 else "none",
        "type": tx.get("type") or "",
        "asset": tx.get("asset") or "",
        "amount_sat": amount,
        "fee_sat": _number(tx.get("feeSat")) or 0.0,
        "fiat_value": _number(tx.get("eur")),
        "wallet": tx.get("account") or "",
        "counterparty": tx.get("counter") or "",
        "note": note[:MAX_NOTE_CHARS],
        "status": tx.get("queueStatus") or "",
        "tags": [str(tag) for tag in (tx.get("tags") or [])[:4]],
    }


def _transactions_visual(spec, payload, fiat_currency, context):
    rows = [_transaction_row(tx) for tx in payload.get("txs") or [] if isinstance(tx, Mapping)]
    total = _int(payload.get("total"))
    caveats: list[dict[str, Any]] = []
    if total is not None and total > len(rows):
        caveats.append({"code": "more_transactions", "count": total - len(rows)})
    # Amounts of different assets do not add up; totals only within BTC.
    btc_only = all(str(row["asset"]).upper() in _BTC_ASSETS for row in rows)
    summary = {
        "shown": len(rows),
        "total": total if total is not None else len(rows),
        "in_sat": sum(row["amount_sat"] for row in rows if row["amount_sat"] > 0) if btc_only else None,
        "out_sat": -sum(row["amount_sat"] for row in rows if row["amount_sat"] < 0) if btc_only else None,
    }
    visual = _header(
        spec,
        chart="list",
        fiat_currency=fiat_currency,
        sort=spec.get("sort"),
        filters=dict(spec["report_args"]),
    )
    return visual, rows, caveats, {"summary": summary}


_OWN = frozenset({"owned", "ambiguous"})


def _flow_label(node: Mapping[str, Any], side: str, position: int) -> tuple[str, str]:
    if node.get("overflow"):
        return "overflow", str(node.get("label") or "")
    if side == "fee":
        return "fee", "Miner fee"
    # A node's label is its outpoint; its wallet names are a separate field.
    # Only the user's own wallets are named; other parties' outpoints and
    # addresses stay out of the visual even where the graph read has them.
    wallet = node.get("wallet")
    if node.get("ownership") in _OWN and isinstance(wallet, str) and wallet.strip():
        return "wallet", wallet.strip()
    return "generic", f"{'Input' if side == 'input' else 'Output'} {position + 1}"


def _flow_row(node: Mapping[str, Any], side: str, position: int) -> dict[str, Any]:
    label_kind, label = _flow_label(node, side, position)
    value = node.get("valueSats")
    row: dict[str, Any] = {
        "side": side,
        "label": label,
        "label_kind": label_kind,
        "position": position,
        "ownership": node.get("ownership") or "unknown",
        "value_sats": value if isinstance(value, int) and not isinstance(value, bool) else None,
        "value_state": node.get("valueState") or ("known" if isinstance(value, int) else "missing"),
    }
    if node.get("overflow"):
        row["overflow_count"] = int(node.get("overflowCount") or 0)
    return row


def _transaction_flow_visual(spec, payload, fiat_currency, context):
    rows: list[dict[str, Any]] = []
    for side, key in (("input", "inputs"), ("output", "outputs")):
        for position, node in enumerate(payload.get(key) or []):
            if isinstance(node, Mapping):
                rows.append(_flow_row(node, side, position))
    fee = payload.get("fee")
    if isinstance(fee, Mapping):
        rows.append(_flow_row(fee, "fee", 0))
    caveats: list[dict[str, Any]] = []
    support = payload.get("supportLevel")
    if not rows or support in {"graphless", "unsupported"}:
        caveats.append({"code": "graph_unavailable"})
    hidden = sum(1 for row in rows if row["value_state"] != "known")
    if hidden:
        caveats.append({"code": "amounts_hidden", "count": hidden})
    accounting = payload.get("accounting") or {}
    if accounting.get("quarantine"):
        caveats.append({"code": "transaction_quarantined"})
    resolved = (context or {}).get("transaction")
    transaction = _transaction_row(resolved) if isinstance(resolved, Mapping) else None
    visual = _header(spec, chart="flow", fiat_currency=fiat_currency)
    return visual, rows, caveats, {"transaction": transaction}


def _check(check: str, status: str, *, count: int | None = None, detail: str | None = None, action: str | None = None) -> dict[str, Any]:
    return {"check": check, "status": status, "count": count, "detail": detail, "action": action, "title": None}


def _readiness_visual(spec, payload, fiat_currency, context):
    health = payload.get("health") or {}
    journals = health.get("journals") or {}
    queue = health.get("review_queue") or {}
    rates = (payload.get("rates_coverage") or {}).get("summary") or {}
    custody = payload.get("custody_quantity") or {}
    counts = health.get("counts") or {}
    checks: list[dict[str, Any]] = []
    has_transactions = bool(counts.get("active_transactions") or counts.get("transactions"))
    checks.append(
        _check("transactions", "ok" if has_transactions else "warning",
               count=int(counts.get("active_transactions") or 0),
               action=None if has_transactions else "import")
    )
    stale = bool(journals.get("needs_processing"))
    checks.append(
        _check("journals", "warning" if stale else "ok",
               detail=journals.get("last_processed_at"),
               action="process_journals" if stale else None)
    )
    quarantines = int(journals.get("quarantine_count") or counts.get("quarantines") or 0)
    checks.append(
        _check("quarantine", "blocked" if quarantines else "ok", count=quarantines,
               action="review_quarantine" if quarantines else None)
    )
    missing = int(rates.get("missing_price_transactions") or 0)
    checks.append(
        _check("prices", "warning" if missing else "ok", count=missing,
               detail=str(int(rates.get("cache_coverable_missing") or 0)) if missing else None,
               action="fill_prices" if missing else None)
    )
    custody_issues = int(custody.get("issue_count") or 0)
    checks.append(
        _check("custody", "blocked" if custody_issues else "ok", count=custody_issues,
               action="review_custody" if custody_issues else None)
    )
    review = int(queue.get("review") or 0)
    checks.append(
        _check("review_queue", "warning" if review else "ok", count=review,
               action="open_review" if review else None)
    )
    # These blockers already have their own check above.
    covered = {
        "journals_stale", "journal_quarantine", "custody_quantity_unresolved",
        "missing_prices", "no_transactions",
    }
    for blocker in payload.get("blockers") or []:
        if not isinstance(blocker, Mapping) or blocker.get("id") in covered:
            continue
        item = _check(
            "blocker",
            "blocked" if blocker.get("severity") == "blocking" else "warning",
            detail=str(blocker.get("detail") or "") or None,
        )
        item["title"] = str(blocker.get("title") or blocker.get("id") or "")
        checks.append(item)
    ready = bool(payload.get("ready"))
    passed = sum(1 for item in checks if item["status"] == "ok")
    visual = _header(spec, chart="checklist", fiat_currency=fiat_currency)
    return visual, checks, [], {
        "summary": {"ready": ready, "passed": passed, "total": len(checks)},
    }


_BUILDERS = {
    "overview": _overview_visual,
    "balance_history": _balance_history_visual,
    "portfolio": _portfolio_visual,
    "tax_summary": _tax_summary_visual,
    "cash_flow": _cash_flow_visual,
    "transactions": _transactions_visual,
    "transaction_flow": _transaction_flow_visual,
    "readiness": _readiness_visual,
}


def build_chat_visual(
    spec: Mapping[str, Any],
    payload: Mapping[str, Any],
    *,
    fiat_currency: str,
    readiness: Mapping[str, Any] | None = None,
    local_reference: Mapping[str, Any] | None = None,
    context: Mapping[str, Any] | None = None,
    now: datetime | None = None,
) -> dict[str, Any]:
    """Project a source payload into the assistant's visual envelope data."""

    currency = str(fiat_currency or "").upper() or "EUR"
    moment = now or datetime.now(timezone.utc)
    visual, rows, caveats, extras = _BUILDERS[spec["source"]](
        spec, payload, currency, {**(context or {}), "now": moment}
    )
    if spec["source"] in JOURNAL_SOURCES and spec["source"] != "overview":
        # The overview carries its own completeness block.
        caveats = _readiness_caveats(readiness) + caveats
    visual["as_of"] = _period_text(moment)
    result: dict[str, Any] = {
        "visual": visual,
        "rows": rows,
        "caveats": caveats,
        **extras,
    }
    if local_reference:
        result["local_reference"] = dict(local_reference)
    result["digest"] = _digest(spec["source"], result)
    return result


# ---------------------------------------------------------------------------
# What the model reads


def _series_digest(values: list[float]) -> dict[str, Any]:
    if not values:
        return {}
    return {
        "first": values[0],
        "last": values[-1],
        "min": min(values),
        "max": max(values),
        "change": values[-1] - values[0],
    }


def _digest(source: str, result: Mapping[str, Any]) -> dict[str, Any]:
    rows = result.get("rows") or []
    if source == "overview":
        return {"stats": result.get("stats"), "wallets": result.get("wallets")}
    if source == "balance_history":
        by_asset: dict[str, dict[str, list[float]]] = {}
        for row in rows:
            asset = by_asset.setdefault(str(row.get("asset")), {})
            for measure in ("quantity", "market_value", "cost_basis"):
                value = row.get(measure)
                if value is not None:
                    asset.setdefault(measure, []).append(value)
        return {
            "periods": len({row.get("period") for row in rows}),
            "first_period": rows[0].get("period") if rows else None,
            "last_period": rows[-1].get("period") if rows else None,
            "assets": {
                asset: {measure: _series_digest(values) for measure, values in measures.items()}
                for asset, measures in by_asset.items()
            },
        }
    if source == "cash_flow":
        return {"buckets": len(rows), "totals": result.get("totals")}
    if source == "tax_summary":
        return {"years": rows}
    if source == "transactions":
        return {"summary": result.get("summary")}
    if source == "transaction_flow":
        return {
            "transaction": result.get("transaction"),
            "inputs": sum(1 for row in rows if row.get("side") == "input"),
            "outputs": sum(1 for row in rows if row.get("side") == "output"),
        }
    if source == "readiness":
        return {"summary": result.get("summary")}
    return {}


def model_digest(data: Mapping[str, Any]) -> dict[str, Any]:
    """The part of a visual the provider needs to describe it.

    The chart, its summary figures and every caveat; the rows themselves only
    while they are few, so a long series does not fill the model's context.
    """

    rows = list(data.get("rows") or [])
    digest: dict[str, Any] = {
        "shown_to_user": True,
        "visual": data.get("visual"),
        "caveats": data.get("caveats") or [],
        # The auto-read context keeps only `summary` when an entry is too
        # large, so the caveats and the shown-to-user note travel inside it too.
        "summary": {
            **(data.get("digest") or {}),
            "caveats": data.get("caveats") or [],
            "shown_to_user": True,
        },
        "row_count": len(rows),
    }
    if len(rows) <= MODEL_ROW_LIMIT:
        digest["rows"] = rows
    else:
        digest["rows_omitted"] = len(rows)
    return digest
