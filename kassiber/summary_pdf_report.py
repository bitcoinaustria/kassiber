from __future__ import annotations

from decimal import Decimal
from pathlib import Path
from typing import Any, Mapping, Sequence

from ._pdf_common import (
    BRAND_ACCENT,
    BRAND_INK,
    BRAND_LINE,
    BRAND_MUTED,
    BRAND_SOFT,
    decimal_value,
    draw_page_header,
    escape_paragraph_text,
    register_fonts,
    require_reportlab,
)


COLOR_BALANCE = "#1f77b4"
COLOR_PROFIT = "#2ca02c"
COLOR_PURPLE = "#9467bd"
COLOR_WARNING = "#ff7f0e"
COLOR_CYAN = "#17becf"
COLOR_GRAY = "#7f7f7f"

PALETTE = (
    BRAND_ACCENT,
    COLOR_BALANCE,
    COLOR_PROFIT,
    COLOR_PURPLE,
    COLOR_WARNING,
    COLOR_CYAN,
    COLOR_GRAY,
)


def _font(rl: dict[str, Any], name: str) -> str:
    fonts = rl.get("summary_fonts") or {}
    return str(fonts.get(name) or ("Helvetica-Bold" if name == "bold" else "Helvetica"))


def _money(currency: str, value: Any) -> str:
    return f"{currency} {decimal_value(value):,.2f}"


def _btc(value: Any) -> str:
    return f"{decimal_value(value):,.8f} BTC"


def _asset_quantity_text(row: Mapping[str, Any]) -> str:
    quantities = row.get("asset_quantities") or []
    parts = []
    for item in quantities:
        asset = str(item.get("asset") or "").strip().upper()
        if asset:
            parts.append(f"{decimal_value(item.get('quantity')):,.8f} {asset}")
    if parts:
        return ", ".join(parts)
    assets = [str(asset).strip().upper() for asset in row.get("assets") or [] if str(asset).strip()]
    suffix = assets[0] if len(assets) == 1 else "units"
    quantity = row.get("quantity")
    if quantity is None:
        quantity = row.get("end_quantity")
    if quantity is None:
        quantity = row.get("total_quantity")
    return f"{decimal_value(quantity):,.8f} {suffix}"


def _signed_money(currency: str, value: Any) -> str:
    number = decimal_value(value)
    prefix = "+" if number > 0 else ""
    return f"{prefix}{currency} {number:,.2f}"


def _compact_money(currency: str, value: Any) -> str:
    number = decimal_value(value)
    sign = "-" if number < 0 else ""
    amount = abs(number)
    if amount >= Decimal("1000000"):
        return f"{sign}{currency} {amount / Decimal('1000000'):.1f}m"
    if amount >= Decimal("1000"):
        return f"{sign}{currency} {amount / Decimal('1000'):.1f}k"
    return f"{sign}{currency} {amount:.0f}"


def _compact_number(value: Any) -> str:
    number = decimal_value(value)
    sign = "-" if number < 0 else ""
    amount = abs(number)
    if amount >= Decimal("1000000"):
        return f"{sign}{amount / Decimal('1000000'):.1f}m"
    if amount >= Decimal("1000"):
        return f"{sign}{amount / Decimal('1000'):.1f}k"
    return f"{sign}{amount:.0f}"


def _compact_quantity(value: Any) -> str:
    number = decimal_value(value)
    sign = "-" if number < 0 else ""
    amount = abs(number)
    if amount >= Decimal("1"):
        return f"{sign}{amount:.4f}"
    if amount >= Decimal("0.01"):
        return f"{sign}{amount:.3f}"
    return f"{sign}{amount:.8f}"


def _pct(value: Decimal, total: Decimal) -> str:
    if total <= 0:
        return "0.0%"
    return f"{(value / total * Decimal('100')):.1f}%"


def _perf_change_text(currency: str, start: Decimal, end: Decimal) -> str:
    if start > 0:
        pct = (end - start) / start * Decimal("100")
        return f"{pct:+.1f}% vs start"
    return f"{_signed_money(currency, end - start)} vs start"


def _quantity_change_text(start: Decimal, end: Decimal) -> str:
    if start > 0:
        pct = (end - start) / start * Decimal("100")
        return f"{pct:+.1f}%"
    delta = end - start
    sign = "+" if delta > 0 else ""
    return f"{sign}{delta:.4f} units"


def _perf_summary_lines(currency: str, metrics: Mapping[str, Any], benchmark: Mapping[str, Any] | None) -> list[str]:
    start = decimal_value(metrics.get("period_start_value"))
    end = decimal_value(metrics.get("period_end_value"))
    stack_start = decimal_value(metrics.get("btc_stack_start"))
    stack_end = decimal_value(metrics.get("btc_stack_end"))
    unrealized = decimal_value(metrics.get("unrealized_pnl"))
    fiat_parts = [f"Period performance: {_perf_change_text(currency, start, end)}"]
    if benchmark and benchmark.get("change_pct") is not None:
        fiat_parts[0] += f" (BTC spot {decimal_value(benchmark['change_pct']):+.1f}%)"
    fiat_parts.append(f"Unrealized at close: {_signed_money(currency, unrealized)}")
    quantity_line = (
        f"Total quantity: {stack_start:.4f} → {stack_end:.4f} units ({_quantity_change_text(stack_start, stack_end)})"
        f" · Network + venue fees: {_btc(metrics.get('fees_btc'))} · {_money(currency, metrics.get('fees_fiat'))}"
    )
    return [" · ".join(fiat_parts), quantity_line]


def _para(rl: dict[str, Any], styles: dict[str, Any], text: Any, style: str = "body"):
    return rl["Paragraph"](escape_paragraph_text(text), styles[style])


def _format_age_days(days: Any) -> str:
    if days is None:
        return "—"
    value = float(days)
    if value >= 365:
        return f"{value / 365.25:.1f} years"
    if value >= 60:
        return f"{value / 30.44:.1f} months"
    return f"{value:.0f} days"


def _holding_age_summary(holding_age: Mapping[str, Any] | None) -> str:
    if not holding_age:
        return ""
    count = int(holding_age.get("acquisition_count") or 0)
    if not count:
        return "No acquisitions recorded in scope."
    parts = []
    weighted = holding_age.get("weighted_days")
    oldest = holding_age.get("oldest_acquisition") or ""
    if weighted is not None:
        parts.append(f"Weighted-avg acquisition age: {_format_age_days(weighted)}")
    if oldest:
        parts.append(f"Oldest acquisition: {oldest[:10]}")
    parts.append(f"{count} acquisition tx in scope")
    return " · ".join(parts)


def _direction_label(direction: Any) -> str:
    text = str(direction or "").lower()
    if text == "inbound":
        return "In"
    if text == "outbound":
        return "Out"
    return text.title() or "—"


def _table_cell(cell: Any) -> Any:
    if hasattr(cell, "wrap") or hasattr(cell, "drawOn"):
        return cell
    return str(cell)


def _table(rl: dict[str, Any], rows: Sequence[Sequence[Any]], widths: Sequence[float], *, header: bool = True):
    colors = rl["colors"]
    table = rl["Table"]([[_table_cell(cell) for cell in row] for row in rows], colWidths=list(widths), repeatRows=1 if header else 0)
    commands = [
        ("FONT", (0, 0), (-1, -1), _font(rl, "regular"), 8),
        ("TEXTCOLOR", (0, 0), (-1, -1), colors.HexColor(BRAND_INK)),
        ("GRID", (0, 0), (-1, -1), 0.3, colors.HexColor(BRAND_LINE)),
        ("VALIGN", (0, 0), (-1, -1), "TOP"),
        ("LEFTPADDING", (0, 0), (-1, -1), 5),
        ("RIGHTPADDING", (0, 0), (-1, -1), 5),
        ("TOPPADDING", (0, 0), (-1, -1), 4),
        ("BOTTOMPADDING", (0, 0), (-1, -1), 4),
    ]
    if header and rows:
        commands.extend(
            [
                ("FONT", (0, 0), (-1, 0), _font(rl, "bold"), 8),
                ("BACKGROUND", (0, 0), (-1, 0), colors.HexColor(BRAND_SOFT)),
            ]
        )
    table.setStyle(rl["TableStyle"](commands))
    return table


def _metric_strip(rl: dict[str, Any], metrics: Sequence[tuple[str, str]]):
    colors = rl["colors"]
    rows = [
        [label for label, _value in metrics],
        [value for _label, value in metrics],
    ]
    col_count = max(len(metrics), 1)
    col_width = (180 * rl["mm"]) / col_count
    table = rl["Table"](rows, colWidths=[col_width] * col_count)
    table.setStyle(
        rl["TableStyle"](
            [
                ("BOX", (0, 0), (-1, -1), 0.4, colors.HexColor(BRAND_LINE)),
                ("INNERGRID", (0, 0), (-1, -1), 0.3, colors.HexColor(BRAND_LINE)),
                ("BACKGROUND", (0, 0), (-1, -1), colors.HexColor(BRAND_SOFT)),
                ("FONT", (0, 0), (-1, 0), _font(rl, "regular"), 7),
                ("FONT", (0, 1), (-1, 1), _font(rl, "bold"), 10),
                ("TEXTCOLOR", (0, 0), (-1, 0), colors.HexColor(BRAND_MUTED)),
                ("TEXTCOLOR", (0, 1), (-1, 1), colors.HexColor(BRAND_INK)),
                ("LEFTPADDING", (0, 0), (-1, -1), 6),
                ("RIGHTPADDING", (0, 0), (-1, -1), 6),
                ("TOPPADDING", (0, 0), (-1, -1), 5),
                ("BOTTOMPADDING", (0, 0), (-1, -1), 5),
            ]
        )
    )
    return table


def _series_bounds(values: Sequence[Decimal]) -> tuple[Decimal, Decimal]:
    if not values:
        return Decimal("0"), Decimal("1")
    low = min(values)
    high = max(values)
    if low == high:
        pad = abs(high) * Decimal("0.1") or Decimal("1")
        return low - pad, high + pad
    pad = (high - low) * Decimal("0.06")
    return low - pad, high + pad


def _tick_values(low: Decimal, high: Decimal, count: int = 5) -> list[Decimal]:
    if count <= 1 or high == low:
        return [low]
    step = (high - low) / Decimal(count - 1)
    return [low + (step * Decimal(idx)) for idx in range(count)]


def _scale(value: Decimal, low: Decimal, high: Decimal, size: float) -> float:
    if high == low:
        return size / 2
    return float((value - low) / (high - low)) * size


def _period_label(row: Mapping[str, Any]) -> str:
    return str(row.get("period") or row.get("period_start") or "")[:7]


def _axis_label(row: Mapping[str, Any], total: int) -> str:
    label = _period_label(row)
    if total > 12 and len(label) == 7 and label[4] == "-":
        return label[2:]
    return label


def _axis_label_font_size(total: int, base: float) -> float:
    if total > 18:
        return 3.6
    if total > 12:
        return 4.1
    return base


def _axis_label_indexes(total: int) -> set[int]:
    if total <= 12:
        return set(range(total))
    step = max(1, round(total / 10))
    indexes = set(range(0, total, step))
    indexes.add(0)
    indexes.add(total - 1)
    return indexes


def _axis_label_y(bottom: float, idx: int, total: int) -> float:
    if total > 12 and idx % 2:
        return bottom - 13
    return bottom - 8


def _line_chart(rl: dict[str, Any], title: str, rows: Sequence[Mapping[str, Any]], currency: str):
    colors = rl["colors"]
    Drawing = rl["Drawing"]
    Line = rl["Line"]
    PolyLine = rl["PolyLine"]
    Circle = rl["Circle"]
    String = rl["String"]
    Rect = rl["Rect"]
    width = 180 * rl["mm"]
    height = 60 * rl["mm"]
    left = 48
    right = 48
    bottom = 25
    top = 28
    plot_w = width - left - right
    plot_h = height - bottom - top
    drawing = Drawing(width, height)
    drawing.add(String(0, height - 10, title, fontName=_font(rl, "bold"), fontSize=9, fillColor=colors.HexColor(BRAND_INK)))
    if not rows:
        drawing.add(Line(left, bottom, left + plot_w, bottom, strokeColor=colors.HexColor(BRAND_LINE), strokeWidth=0.5))
        drawing.add(String(left, bottom + plot_h / 2, "No balance history in scope.", fontName=_font(rl, "regular"), fontSize=8, fillColor=colors.HexColor(BRAND_MUTED)))
        return drawing

    fiat_values = [decimal_value(row.get("market_value")) for row in rows]
    btc_values = [decimal_value(row.get("quantity")) for row in rows]
    fiat_low, fiat_high = _series_bounds(fiat_values)
    btc_low, btc_high = _series_bounds(btc_values)
    for idx, fiat_tick in enumerate(_tick_values(fiat_low, fiat_high)):
        y = bottom + _scale(fiat_tick, fiat_low, fiat_high, plot_h)
        drawing.add(Line(left, y, left + plot_w, y, strokeColor=colors.HexColor(BRAND_LINE), strokeWidth=0.3))
        drawing.add(String(left - 4, y - 2, _compact_money(currency, fiat_tick), fontName=_font(rl, "regular"), fontSize=5.8, fillColor=colors.HexColor(BRAND_MUTED), textAnchor="end"))
        quantity_tick = btc_low + ((btc_high - btc_low) * Decimal(idx) / Decimal(4))
        drawing.add(String(left + plot_w + 4, y - 2, _compact_quantity(quantity_tick), fontName=_font(rl, "regular"), fontSize=5.8, fillColor=colors.HexColor(BRAND_MUTED)))
    drawing.add(Line(left, bottom, left + plot_w, bottom, strokeColor=colors.HexColor(BRAND_LINE), strokeWidth=0.6))
    drawing.add(Line(left, bottom, left, bottom + plot_h, strokeColor=colors.HexColor(BRAND_LINE), strokeWidth=0.6))
    drawing.add(Line(left + plot_w, bottom, left + plot_w, bottom + plot_h, strokeColor=colors.HexColor(BRAND_LINE), strokeWidth=0.6))
    drawing.add(Rect(width - 152, height - 15, 5, 5, strokeColor=colors.HexColor(BRAND_ACCENT), fillColor=colors.HexColor(BRAND_ACCENT)))
    drawing.add(String(width - 144, height - 15, "Market value", fontName=_font(rl, "regular"), fontSize=6.4, fillColor=colors.HexColor(BRAND_MUTED)))
    drawing.add(Rect(width - 94, height - 15, 5, 5, strokeColor=colors.HexColor(COLOR_GRAY), fillColor=colors.HexColor(COLOR_GRAY)))
    drawing.add(String(width - 86, height - 15, "Cost basis", fontName=_font(rl, "regular"), fontSize=6.4, fillColor=colors.HexColor(BRAND_MUTED)))
    drawing.add(Rect(width - 39, height - 15, 5, 5, strokeColor=colors.HexColor(COLOR_BALANCE), fillColor=colors.HexColor(COLOR_BALANCE)))
    drawing.add(String(width - 31, height - 15, "Units", fontName=_font(rl, "regular"), fontSize=6.4, fillColor=colors.HexColor(BRAND_MUTED)))
    count = max(len(rows) - 1, 1)
    label_size = _axis_label_font_size(len(rows), 5.2)
    label_indexes = _axis_label_indexes(len(rows))
    fiat_points = []
    cost_points = []
    btc_points = []
    for idx, row in enumerate(rows):
        x = left + plot_w * (idx / count)
        fiat_points.append((x, bottom + _scale(decimal_value(row.get("market_value")), fiat_low, fiat_high, plot_h)))
        cost_points.append((x, bottom + _scale(decimal_value(row.get("cumulative_cost_basis")), fiat_low, fiat_high, plot_h)))
        btc_points.append((x, bottom + _scale(decimal_value(row.get("quantity")), btc_low, btc_high, plot_h)))
        if idx in label_indexes:
            drawing.add(String(x, _axis_label_y(bottom, idx, len(rows)), _axis_label(row, len(rows)), fontName=_font(rl, "regular"), fontSize=label_size, fillColor=colors.HexColor(BRAND_MUTED), textAnchor="middle"))
    if len(fiat_points) == 1:
        x, y = fiat_points[0]
        drawing.add(Line(x - 2, y, x + 2, y, strokeColor=colors.HexColor(BRAND_ACCENT), strokeWidth=1.4))
    else:
        drawing.add(PolyLine(cost_points, strokeColor=colors.HexColor(COLOR_GRAY), strokeWidth=0.9))
        drawing.add(PolyLine(fiat_points, strokeColor=colors.HexColor(BRAND_ACCENT), strokeWidth=1.4))
        drawing.add(PolyLine(btc_points, strokeColor=colors.HexColor(COLOR_BALANCE), strokeWidth=1.1))
    for x, y in (fiat_points[0], fiat_points[-1]):
        drawing.add(Circle(x, y, 2, strokeColor=colors.HexColor(BRAND_ACCENT), fillColor=colors.white, strokeWidth=0.8))
    for x, y in (btc_points[0], btc_points[-1]):
        drawing.add(Circle(x, y, 1.7, strokeColor=colors.HexColor(COLOR_BALANCE), fillColor=colors.white, strokeWidth=0.8))
    first_row = rows[0]
    last_row = rows[-1]
    drawing.add(String(left + 3, bottom + plot_h + 5, f"Start {_compact_money(currency, first_row.get('market_value'))}", fontName=_font(rl, "regular"), fontSize=6, fillColor=colors.HexColor(BRAND_MUTED)))
    drawing.add(String(left + plot_w - 3, bottom + plot_h + 5, f"End {_compact_money(currency, last_row.get('market_value'))}", fontName=_font(rl, "bold"), fontSize=6, fillColor=colors.HexColor(BRAND_INK), textAnchor="end"))
    drawing.add(String(left, 5, "Left axis market value · right axis total units", fontName=_font(rl, "regular"), fontSize=6.3, fillColor=colors.HexColor(BRAND_MUTED)))
    if rows and rows[-1].get("period_partial"):
        drawing.add(String(left + 142, 5, f"Final period capped at {str(rows[-1].get('period_end', ''))[:10]}", fontName=_font(rl, "regular"), fontSize=6.3, fillColor=colors.HexColor(BRAND_MUTED)))
    return drawing


def _donut_chart(rl: dict[str, Any], title: str, rows: Sequence[Mapping[str, Any]], currency: str):
    colors = rl["colors"]
    Drawing = rl["Drawing"]
    String = rl["String"]
    Wedge = rl["Wedge"]
    Circle = rl["Circle"]
    width = 180 * rl["mm"]
    height = 58 * rl["mm"]
    drawing = Drawing(width, height)
    drawing.add(String(0, height - 10, title, fontName=_font(rl, "bold"), fontSize=9, fillColor=colors.HexColor(BRAND_INK)))
    visible_rows = [row for row in rows if decimal_value(row.get("market_value")) > 0]
    total = sum(decimal_value(row.get("market_value")) for row in visible_rows)
    cx = 31 * rl["mm"]
    cy = 27 * rl["mm"]
    radius = 19 * rl["mm"]
    if total <= 0:
        drawing.add(Circle(cx, cy, radius, strokeColor=colors.HexColor(BRAND_LINE), fillColor=colors.HexColor(BRAND_SOFT)))
        drawing.add(String(8, 9, "No holdings in scope.", fontName=_font(rl, "regular"), fontSize=8, fillColor=colors.HexColor(BRAND_MUTED)))
        return drawing
    angle = 90.0
    for idx, row in enumerate(visible_rows):
        value = decimal_value(row.get("market_value"))
        sweep = float(value / total) * 360.0
        color = colors.HexColor(PALETTE[idx % len(PALETTE)])
        drawing.add(Wedge(cx, cy, radius, angle, angle + sweep, fillColor=color, strokeColor=colors.white, strokeWidth=0.4))
        angle += sweep
    drawing.add(Circle(cx, cy, radius * 0.52, strokeColor=colors.white, fillColor=colors.white))
    drawing.add(String(cx, cy + 3, "Period end", fontName=_font(rl, "regular"), fontSize=6.2, fillColor=colors.HexColor(BRAND_MUTED), textAnchor="middle"))
    drawing.add(String(cx, cy - 7, _compact_money(currency, total), fontName=_font(rl, "bold"), fontSize=8, fillColor=colors.HexColor(BRAND_INK), textAnchor="middle"))
    y = height - 22
    for idx, row in enumerate(visible_rows[:6]):
        color = colors.HexColor(PALETTE[idx % len(PALETTE)])
        value = decimal_value(row.get("market_value"))
        drawing.add(rl["Rect"](75 * rl["mm"], y - 6, 5, 5, strokeColor=color, fillColor=color))
        label = str(row.get("wallet") or "Wallet")[:24]
        drawing.add(String(75 * rl["mm"] + 8, y - 4, label, fontName=_font(rl, "bold"), fontSize=6.7, fillColor=colors.HexColor(BRAND_INK)))
        drawing.add(String(75 * rl["mm"] + 8, y - 12, f"{_money(currency, value)} · {_pct(value, total)}", fontName=_font(rl, "regular"), fontSize=6.2, fillColor=colors.HexColor(BRAND_MUTED)))
        y -= 17
    if len(visible_rows) > 6:
        hidden_value = sum(decimal_value(row.get("market_value")) for row in visible_rows[6:])
        drawing.add(rl["Rect"](75 * rl["mm"], y - 6, 5, 5, strokeColor=colors.HexColor(COLOR_GRAY), fillColor=colors.HexColor(BRAND_SOFT)))
        drawing.add(String(75 * rl["mm"] + 8, y - 6, f"+{len(visible_rows) - 6} more · {_money(currency, hidden_value)} · {_pct(hidden_value, total)}", fontName=_font(rl, "regular"), fontSize=6.2, fillColor=colors.HexColor(BRAND_MUTED)))
    return drawing


def _bar_chart(
    rl: dict[str, Any],
    title: str,
    rows: Sequence[Mapping[str, Any]],
    currency: str,
    *,
    paired: bool = False,
):
    colors = rl["colors"]
    Drawing = rl["Drawing"]
    Rect = rl["Rect"]
    String = rl["String"]
    Line = rl["Line"]
    width = 180 * rl["mm"]
    height = 64 * rl["mm"]
    drawing = Drawing(width, height)
    drawing.add(String(0, height - 10, title, fontName=_font(rl, "bold"), fontSize=9, fillColor=colors.HexColor(BRAND_INK)))
    left = 42
    bottom = 25
    right = 10
    plot_w = width - left - right
    plot_h = height - 45
    if not rows:
        drawing.add(Line(left, bottom, left + plot_w, bottom, strokeColor=colors.HexColor(BRAND_LINE), strokeWidth=0.5))
        drawing.add(String(left, bottom + plot_h / 2, "No rows in scope.", fontName=_font(rl, "regular"), fontSize=8, fillColor=colors.HexColor(BRAND_MUTED)))
        return drawing
    if paired:
        drawing.add(Line(left, bottom, left + plot_w, bottom, strokeColor=colors.HexColor(BRAND_LINE), strokeWidth=0.5))
        values = [decimal_value(row.get("inflow_volume")) for row in rows] + [decimal_value(row.get("outflow_volume")) for row in rows]
        raw_max_value = max(values) if values else Decimal("0")
        max_value = (raw_max_value * Decimal("1.10")) if raw_max_value else Decimal("1")
        total_inflow = sum(decimal_value(row.get("inflow_volume")) for row in rows)
        total_outflow = sum(decimal_value(row.get("outflow_volume")) for row in rows)
        for tick in _tick_values(Decimal("0"), max_value, 4):
            y = bottom + _scale(tick, Decimal("0"), max_value, plot_h)
            drawing.add(Line(left, y, left + plot_w, y, strokeColor=colors.HexColor(BRAND_LINE), strokeWidth=0.25))
            drawing.add(String(left - 4, y - 2, _compact_money(currency, tick), fontName=_font(rl, "regular"), fontSize=5.8, fillColor=colors.HexColor(BRAND_MUTED), textAnchor="end"))
        drawing.add(Rect(width - 92, height - 15, 5, 5, strokeColor=colors.HexColor(COLOR_PROFIT), fillColor=colors.HexColor(COLOR_PROFIT)))
        drawing.add(String(width - 84, height - 15, "Inflow", fontName=_font(rl, "regular"), fontSize=6.5, fillColor=colors.HexColor(BRAND_MUTED)))
        drawing.add(Rect(width - 52, height - 15, 5, 5, strokeColor=colors.HexColor(BRAND_ACCENT), fillColor=colors.HexColor(BRAND_ACCENT)))
        drawing.add(String(width - 44, height - 15, "Outflow", fontName=_font(rl, "regular"), fontSize=6.5, fillColor=colors.HexColor(BRAND_MUTED)))
    else:
        pnl_values = [decimal_value(row.get("realized_pnl")) for row in rows]
        raw_low = min([Decimal("0"), *pnl_values])
        raw_high = max([Decimal("0"), *pnl_values])
        low = raw_low
        high = raw_high
        if low == high:
            high = Decimal("1")
        else:
            pad = (high - low) * Decimal("0.10")
            low -= pad
            high += pad
        baseline = bottom + _scale(Decimal("0"), low, high, plot_h)
        for tick in _tick_values(low, high, 5):
            y = bottom + _scale(tick, low, high, plot_h)
            drawing.add(Line(left, y, left + plot_w, y, strokeColor=colors.HexColor(BRAND_LINE), strokeWidth=0.25))
            drawing.add(String(left - 4, y - 2, _compact_money(currency, tick), fontName=_font(rl, "regular"), fontSize=5.8, fillColor=colors.HexColor(BRAND_MUTED), textAnchor="end"))
        drawing.add(Line(left, baseline, left + plot_w, baseline, strokeColor=colors.HexColor(BRAND_LINE), strokeWidth=0.5))
        drawing.add(Rect(width - 111, height - 15, 5, 5, strokeColor=colors.HexColor(COLOR_PROFIT), fillColor=colors.HexColor(COLOR_PROFIT)))
        drawing.add(String(width - 103, height - 15, "Gain", fontName=_font(rl, "regular"), fontSize=6.5, fillColor=colors.HexColor(BRAND_MUTED)))
        drawing.add(Rect(width - 73, height - 15, 5, 5, strokeColor=colors.HexColor(BRAND_ACCENT), fillColor=colors.HexColor(BRAND_ACCENT)))
        drawing.add(String(width - 65, height - 15, "Loss", fontName=_font(rl, "regular"), fontSize=6.5, fillColor=colors.HexColor(BRAND_MUTED)))
    bar_slot = plot_w / max(len(rows), 1)
    label_size = _axis_label_font_size(len(rows), 5)
    label_indexes = _axis_label_indexes(len(rows))
    for idx, row in enumerate(rows):
        x = left + idx * bar_slot + 2
        if idx in label_indexes:
            drawing.add(String(x + bar_slot / 2, _axis_label_y(bottom, idx, len(rows)), _axis_label(row, len(rows)), fontName=_font(rl, "regular"), fontSize=label_size, fillColor=colors.HexColor(BRAND_MUTED), textAnchor="middle"))
        if paired:
            inflow_h = _scale(decimal_value(row.get("inflow_volume")), Decimal("0"), max_value, plot_h)
            outflow_h = _scale(decimal_value(row.get("outflow_volume")), Decimal("0"), max_value, plot_h)
            drawing.add(Rect(x, bottom, max(bar_slot / 2 - 2, 1), inflow_h, fillColor=colors.HexColor(COLOR_PROFIT), strokeColor=None))
            drawing.add(Rect(x + bar_slot / 2, bottom, max(bar_slot / 2 - 2, 1), outflow_h, fillColor=colors.HexColor(BRAND_ACCENT), strokeColor=None))
            if len(rows) <= 12:
                if decimal_value(row.get("inflow_volume")):
                    drawing.add(String(x + max(bar_slot / 4 - 1, 1), min(bottom + inflow_h + 3, bottom + plot_h - 5), _compact_number(row.get("inflow_volume")), fontName=_font(rl, "regular"), fontSize=5.2, fillColor=colors.HexColor(BRAND_INK), textAnchor="middle"))
                if decimal_value(row.get("outflow_volume")):
                    drawing.add(String(x + bar_slot * 0.75, min(bottom + outflow_h + 3, bottom + plot_h - 5), _compact_number(row.get("outflow_volume")), fontName=_font(rl, "regular"), fontSize=5.2, fillColor=colors.HexColor(BRAND_INK), textAnchor="middle"))
        else:
            value = decimal_value(row.get("realized_pnl"))
            scaled_value = bottom + _scale(value, low, high, plot_h)
            y = min(baseline, scaled_value)
            bar_h = abs(scaled_value - baseline)
            if value != 0 and bar_h < 1:
                bar_h = 1
            color = COLOR_PROFIT if value >= 0 else BRAND_ACCENT
            drawing.add(Rect(x, y, max(bar_slot - 4, 1), bar_h, fillColor=colors.HexColor(color), strokeColor=None))
            if value and len(rows) <= 18:
                inside_loss = value < 0 and bar_h >= 9
                label_y = y + bar_h - 7 if inside_loss else y + bar_h + 3
                label_color = "#ffffff" if inside_loss else BRAND_INK
                label_y = max(bottom + 3, min(label_y, bottom + plot_h - 6))
                drawing.add(String(x + bar_slot / 2, label_y, _compact_money(currency, value), fontName=_font(rl, "bold"), fontSize=5.2, fillColor=colors.HexColor(label_color), textAnchor="middle"))
    if paired:
        drawing.add(String(left, 4, f"Max period {_money(currency, raw_max_value)} · Total in {_money(currency, total_inflow)} · Total out {_money(currency, total_outflow)}", fontName=_font(rl, "regular"), fontSize=6.5, fillColor=colors.HexColor(BRAND_MUTED)))
    else:
        drawing.add(String(left, 4, f"Range {_signed_money(currency, raw_low)} to {_signed_money(currency, raw_high)}", fontName=_font(rl, "regular"), fontSize=6.5, fillColor=colors.HexColor(BRAND_MUTED)))
    return drawing


def _cover_flowables(rl, styles, report):
    return [
        _para(rl, styles, report.get("title") or "Kassiber Summary Report", "title"),
        _para(rl, styles, f"{report.get('workspace')} / {report.get('profile')}", "body"),
        _para(rl, styles, f"Timeframe: {report.get('timeframe', {}).get('label', '')} · Generated: {report.get('generated_at', '')}", "muted"),
        rl["Spacer"](1, 6),
    ]


def _snapshot_flowables(rl, styles, report, currency):
    snapshot = report.get("snapshot")
    if not snapshot:
        return []
    rows = [["Wallet", "Assets", "Balance", "Market value"]]
    for row in snapshot.get("wallets", []):
        rows.append([
            row.get("wallet", ""),
            ", ".join(row.get("assets") or []),
            _asset_quantity_text(row),
            _money(currency, row.get("market_value")),
        ])
    return [
        _para(
            rl,
            styles,
            f"Current snapshot: {_money(currency, snapshot.get('total_market_value'))} · {_asset_quantity_text(snapshot)}",
            "h2",
        ),
        _table(rl, rows, [44 * rl["mm"], 34 * rl["mm"], 38 * rl["mm"], 45 * rl["mm"]]),
        rl["Spacer"](1, 8),
    ]


def _kpi_flowables(rl, styles, report, currency):
    metrics = report.get("metrics") or {}
    flowables = [
        _metric_strip(rl, [
            ("Start value", _money(currency, metrics.get("period_start_value"))),
            ("End value", _money(currency, metrics.get("period_end_value"))),
            ("Net flow", _signed_money(currency, metrics.get("net_flow"))),
            ("Realized PnL", _signed_money(currency, metrics.get("realized_pnl"))),
            ("Fees", _money(currency, metrics.get("fees_fiat"))),
        ]),
        rl["Spacer"](1, 5),
    ]
    for line in _perf_summary_lines(currency, metrics, report.get("benchmark")):
        flowables.append(_para(rl, styles, line, "muted"))
    flowables.append(rl["Spacer"](1, 8))
    return flowables


def _data_integrity_flowables(rl, styles, report, currency):
    data_integrity = report.get("data_integrity") or {}
    priced_total = int(data_integrity.get("total_transactions") or 0)
    priced_count = int(data_integrity.get("priced_transactions") or 0)
    priced_pct = decimal_value(data_integrity.get("priced_percentage"))
    journal_status = (data_integrity.get("journals") or {}).get("status") or "unknown"
    rows = [
        ["Signal", "Status"],
        ["Priced transactions", f"{priced_count} / {priced_total} ({priced_pct:.1f}%)"],
        ["Journals", str(journal_status).replace("_", " ").title()],
        ["Quarantines", str(int(data_integrity.get("quarantine_count") or 0))],
    ]
    internal = data_integrity.get("internal_transfers") or {}
    if int(internal.get("count") or 0):
        rows.append([
            "Internal transfers (excluded from flow)",
            f"{int(internal['count'])} tx · {_money(currency, internal.get('fiat_volume'))}",
        ])
    quarantine_reasons = data_integrity.get("quarantine_reasons") or []
    if quarantine_reasons:
        for r in quarantine_reasons:
            rows.append([f"Quarantine: {r.get('reason', '')}", str(int(r.get("count") or 0))])
    else:
        rows.append(["Quarantine reasons", "None in scope"])
    return [
        _para(rl, styles, "Data Integrity", "h2"),
        _table(rl, rows, [70 * rl["mm"], 92 * rl["mm"]]),
    ]


def _movement_flowables(rl, styles, report, currency):
    return [
        _para(rl, styles, "Portfolio Movement", "h2"),
        _line_chart(rl, "Total balance over time", report.get("balance_history") or [], currency),
    ]


def _composition_flowables(rl, styles, report, currency):
    flowables = [
        _para(rl, styles, "Portfolio Composition", "h2"),
        _donut_chart(rl, "Holdings by wallet at period end", report.get("wallet_holdings") or [], currency),
    ]
    holding_age_text = _holding_age_summary(report.get("holding_age"))
    if holding_age_text:
        flowables.append(_para(rl, styles, holding_age_text, "muted"))
    flowables.append(rl["Spacer"](1, 7))
    return flowables


def _disposal_table_flowables(rl, styles, top_disposals, currency):
    if not top_disposals:
        return []
    rows = [["Date", "Wallet", "Quantity", "Proceeds", "Cost basis", "Gain/Loss"]]
    for row in top_disposals:
        rows.append([
            str(row.get("occurred_at", ""))[:10],
            str(row.get("wallet", ""))[:22],
            _btc(-decimal_value(row.get("quantity"))),
            _money(currency, row.get("proceeds")),
            _money(currency, row.get("cost_basis")),
            _signed_money(currency, row.get("gain_loss")),
        ])
    return [
        rl["Spacer"](1, 4),
        _para(rl, styles, "Largest disposals", "body"),
        _table(rl, rows, [22 * rl["mm"], 36 * rl["mm"], 30 * rl["mm"], 30 * rl["mm"], 30 * rl["mm"], 34 * rl["mm"]]),
    ]


def _movement_table_flowables(rl, styles, top_movements, currency):
    if not top_movements:
        return []
    rows = [["Date", "Wallet", "Dir", "Asset", "Amount", "Value", "Counterparty"]]
    for row in top_movements:
        rows.append([
            str(row.get("occurred_at", ""))[:10],
            str(row.get("wallet", ""))[:22],
            _direction_label(row.get("direction")),
            str(row.get("asset", "")),
            _btc(row.get("quantity")),
            _money(currency, row.get("fiat_value")),
            str(row.get("counterparty", ""))[:28],
        ])
    return [
        rl["Spacer"](1, 4),
        _para(rl, styles, "Largest activity", "body"),
        _table(rl, rows, [22 * rl["mm"], 30 * rl["mm"], 12 * rl["mm"], 14 * rl["mm"], 28 * rl["mm"], 30 * rl["mm"], 46 * rl["mm"]]),
    ]


def _activity_flowables(rl, styles, report, currency):
    flowables = [
        _para(rl, styles, "Period Activity", "h2"),
        _bar_chart(rl, "Realized PnL per period", report.get("realized_pnl_periods") or [], currency),
    ]
    flowables.extend(_disposal_table_flowables(rl, styles, report.get("top_disposals") or [], currency))
    flowables.append(rl["Spacer"](1, 7))
    flowables.append(_bar_chart(rl, "Inflows vs outflows volume", report.get("flow_periods") or [], currency, paired=True))
    flowables.extend(_movement_table_flowables(rl, styles, report.get("top_movements") or [], currency))
    return flowables


def _appendix_flowables(rl, styles, report, currency):
    rows = [["Wallet", "Scope", "Tx count", "End balance", "End value"]]
    for row in report.get("wallet_appendix") or []:
        rows.append([
            row.get("wallet", ""),
            row.get("scope", ""),
            row.get("tx_count", 0),
            _asset_quantity_text(row),
            _money(currency, row.get("end_market_value")),
        ])
    return [
        rl["Spacer"](1, 7),
        _para(rl, styles, "Wallet Appendix", "h2"),
        _table(rl, rows, [42 * rl["mm"], 38 * rl["mm"], 22 * rl["mm"], 38 * rl["mm"], 42 * rl["mm"]]),
        rl["Spacer"](1, 8),
        _para(rl, styles, "This summary report is a portfolio and treasury view. It intentionally omits tax tables; use the tax PDF for tax filing support.", "muted"),
    ]


def _build_styles(rl, fonts):
    colors = rl["colors"]
    return {
        "title": rl["ParagraphStyle"]("Title", fontName=fonts["bold"], fontSize=18, leading=22, textColor=colors.HexColor(BRAND_INK)),
        "h2": rl["ParagraphStyle"]("H2", fontName=fonts["bold"], fontSize=11.5, leading=15, spaceBefore=8, spaceAfter=5, textColor=colors.HexColor(BRAND_INK)),
        "body": rl["ParagraphStyle"]("Body", fontName=fonts["regular"], fontSize=8.5, leading=12, textColor=colors.HexColor(BRAND_INK)),
        "muted": rl["ParagraphStyle"]("Muted", fontName=fonts["regular"], fontSize=8, leading=11, textColor=colors.HexColor(BRAND_MUTED)),
    }


def _build_doc_template(rl, file_path, report, fonts):
    profile_label = str(report.get("profile") or "")
    workspace_label = str(report.get("workspace") or "")
    timeframe_label = str(report.get("timeframe", {}).get("label") or "")
    book_label = " / ".join(part for part in (workspace_label, profile_label) if part)
    doc = rl["BaseDocTemplate"](
        file_path,
        pagesize=rl["A4"],
        leftMargin=14 * rl["mm"],
        rightMargin=14 * rl["mm"],
        topMargin=18 * rl["mm"],
        bottomMargin=14 * rl["mm"],
        title=str(report.get("title") or "Kassiber Summary Report"),
        author="Kassiber",
        subject=f"Treasury summary for {book_label} · {timeframe_label}" if book_label else "Treasury summary",
        keywords="kassiber, bitcoin, treasury, portfolio, summary",
    )
    frame = rl["Frame"](doc.leftMargin, doc.bottomMargin, doc.width, doc.height, id="normal")
    footer_left = " · ".join(part for part in (book_label, timeframe_label) if part) or timeframe_label
    template = rl["PageTemplate"](
        id="Summary",
        frames=[frame],
        onPage=lambda canvas, doc_obj: draw_page_header(
            canvas,
            doc_obj,
            title="Summary report",
            fonts=fonts,
            rl=rl,
            footer_left=footer_left,
            page_label=None,
        ),
    )
    doc.addPageTemplates([template])
    return doc


def _numbered_canvas_factory(rl: dict[str, Any]):
    base_canvas = rl["Canvas"]

    class NumberedCanvas(base_canvas):
        def __init__(self, *args, **kwargs):
            super().__init__(*args, **kwargs)
            self._saved_page_states = []

        def showPage(self):
            self._saved_page_states.append(dict(self.__dict__))
            self._startPage()

        def save(self):
            total = len(self._saved_page_states)
            for state in self._saved_page_states:
                self.__dict__.update(state)
                self._draw_page_total(total)
                super().showPage()
            super().save()

        def _draw_page_total(self, total: int):
            self.saveState()
            self.setFont(rl.get("summary_fonts", {}).get("regular", "Helvetica"), 7)
            self.setFillColor(rl["colors"].HexColor(BRAND_MUTED))
            width, _ = self._pagesize
            self.drawRightString(
                width - 14 * rl["mm"],
                8 * rl["mm"],
                f"Page {self._pageNumber} of {total}",
            )
            self.restoreState()

    return NumberedCanvas


def write_summary_pdf(file_path: str | Path, report: Mapping[str, Any]) -> Mapping[str, Any]:
    rl = require_reportlab("Summary PDF report")
    rl["rl_config"].invariant = 1
    fonts = register_fonts(rl)
    rl["summary_fonts"] = fonts
    path = Path(file_path).expanduser()
    path.parent.mkdir(parents=True, exist_ok=True)
    styles = _build_styles(rl, fonts)
    doc = _build_doc_template(rl, str(path), report, fonts)
    currency = str(report.get("fiat_currency") or "")
    story: list[Any] = []
    story.extend(_cover_flowables(rl, styles, report))
    story.extend(_snapshot_flowables(rl, styles, report, currency))
    story.extend(_kpi_flowables(rl, styles, report, currency))
    story.extend(_data_integrity_flowables(rl, styles, report, currency))
    story.extend(_movement_flowables(rl, styles, report, currency))
    story.append(rl["PageBreak"]())
    story.extend(_composition_flowables(rl, styles, report, currency))
    story.extend(_activity_flowables(rl, styles, report, currency))
    story.extend(_appendix_flowables(rl, styles, report, currency))
    doc.build(story, canvasmaker=_numbered_canvas_factory(rl))
    return {
        "file": str(path.resolve()),
        "pages": doc.page,
        "bytes": path.stat().st_size,
        "title": str(report.get("title") or "Kassiber Summary Report"),
    }
