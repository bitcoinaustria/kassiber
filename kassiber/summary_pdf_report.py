from __future__ import annotations

from decimal import Decimal
from pathlib import Path
from typing import Any, Mapping, Sequence

from ._pdf_common import (
    BRAND_ACCENT,
    BRAND_ACCENT_SOFT,
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


PALETTE = (
    "#e3000f",
    "#1f77b4",
    "#2ca02c",
    "#9467bd",
    "#ff7f0e",
    "#17becf",
    "#7f7f7f",
)


def _font(rl: dict[str, Any], name: str) -> str:
    fonts = rl.get("summary_fonts") or {}
    return str(fonts.get(name) or ("Helvetica-Bold" if name == "bold" else "Helvetica"))


def _money(currency: str, value: Any) -> str:
    return f"{currency} {decimal_value(value):,.2f}"


def _btc(value: Any) -> str:
    return f"{decimal_value(value):,.8f} BTC"


def _signed_money(currency: str, value: Any) -> str:
    number = decimal_value(value)
    prefix = "+" if number > 0 else ""
    return f"{prefix}{currency} {number:,.2f}"


def _para(rl: dict[str, Any], styles: dict[str, Any], text: Any, style: str = "body"):
    return rl["Paragraph"](escape_paragraph_text(text), styles[style])


def _table(rl: dict[str, Any], rows: Sequence[Sequence[Any]], widths: Sequence[float], *, header: bool = True):
    colors = rl["colors"]
    table = rl["Table"]([[str(cell) for cell in row] for row in rows], colWidths=list(widths), repeatRows=1 if header else 0)
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


def _metric_strip(rl: dict[str, Any], metrics: Sequence[tuple[str, str, str]]):
    colors = rl["colors"]
    rows = [
        [label for label, _value, _sub in metrics],
        [value for _label, value, _sub in metrics],
        [sub for _label, _value, sub in metrics],
    ]
    table = rl["Table"](rows, colWidths=[45 * rl["mm"], 45 * rl["mm"], 45 * rl["mm"], 45 * rl["mm"]])
    table.setStyle(
        rl["TableStyle"](
            [
                ("BOX", (0, 0), (-1, -1), 0.4, colors.HexColor(BRAND_LINE)),
                ("INNERGRID", (0, 0), (-1, -1), 0.3, colors.HexColor(BRAND_LINE)),
                ("BACKGROUND", (0, 0), (-1, -1), colors.HexColor(BRAND_SOFT)),
                ("FONT", (0, 0), (-1, -1), _font(rl, "regular"), 7),
                ("FONT", (0, 1), (-1, 1), _font(rl, "bold"), 10),
                ("TEXTCOLOR", (0, 0), (-1, 0), colors.HexColor(BRAND_MUTED)),
                ("TEXTCOLOR", (0, 1), (-1, 1), colors.HexColor(BRAND_INK)),
                ("TEXTCOLOR", (0, 2), (-1, 2), colors.HexColor(BRAND_MUTED)),
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
    return low, high


def _scale(value: Decimal, low: Decimal, high: Decimal, size: float) -> float:
    if high == low:
        return size / 2
    return float((value - low) / (high - low)) * size


def _line_chart(rl: dict[str, Any], title: str, rows: Sequence[Mapping[str, Any]], currency: str):
    colors = rl["colors"]
    Drawing = rl["Drawing"]
    Line = rl["Line"]
    PolyLine = rl["PolyLine"]
    String = rl["String"]
    width = 180 * rl["mm"]
    height = 78 * rl["mm"]
    left = 24
    right = 24
    bottom = 24
    top = 28
    plot_w = width - left - right
    plot_h = height - bottom - top
    drawing = Drawing(width, height)
    drawing.add(String(0, height - 10, title, fontName=_font(rl, "bold"), fontSize=9, fillColor=colors.HexColor(BRAND_INK)))
    drawing.add(Line(left, bottom, left + plot_w, bottom, strokeColor=colors.HexColor(BRAND_LINE), strokeWidth=0.5))
    drawing.add(Line(left, bottom, left, bottom + plot_h, strokeColor=colors.HexColor(BRAND_LINE), strokeWidth=0.5))
    if not rows:
        drawing.add(String(left, bottom + plot_h / 2, "No balance history in scope.", fontName=_font(rl, "regular"), fontSize=8, fillColor=colors.HexColor(BRAND_MUTED)))
        return drawing

    fiat_values = [decimal_value(row.get("market_value")) for row in rows]
    btc_values = [decimal_value(row.get("quantity")) for row in rows]
    fiat_low, fiat_high = _series_bounds(fiat_values)
    btc_low, btc_high = _series_bounds(btc_values)
    count = max(len(rows) - 1, 1)
    fiat_points = []
    btc_points = []
    for idx, row in enumerate(rows):
        x = left + plot_w * (idx / count)
        fiat_points.append((x, bottom + _scale(decimal_value(row.get("market_value")), fiat_low, fiat_high, plot_h)))
        btc_points.append((x, bottom + _scale(decimal_value(row.get("quantity")), btc_low, btc_high, plot_h)))
    if len(fiat_points) == 1:
        x, y = fiat_points[0]
        drawing.add(Line(x - 2, y, x + 2, y, strokeColor=colors.HexColor(BRAND_ACCENT), strokeWidth=1.4))
    else:
        drawing.add(PolyLine(fiat_points, strokeColor=colors.HexColor(BRAND_ACCENT), strokeWidth=1.4))
        drawing.add(PolyLine(btc_points, strokeColor=colors.HexColor("#1f77b4"), strokeWidth=1.1))
    drawing.add(String(left, 8, f"Fiat axis: {_money(currency, fiat_low)} to {_money(currency, fiat_high)}", fontName=_font(rl, "regular"), fontSize=6.5, fillColor=colors.HexColor(BRAND_MUTED)))
    drawing.add(String(width - 95, 8, f"BTC axis: {_btc(btc_low)} to {_btc(btc_high)}", fontName=_font(rl, "regular"), fontSize=6.5, fillColor=colors.HexColor(BRAND_MUTED)))
    return drawing


def _donut_chart(rl: dict[str, Any], title: str, rows: Sequence[Mapping[str, Any]], currency: str):
    colors = rl["colors"]
    Drawing = rl["Drawing"]
    String = rl["String"]
    Wedge = rl["Wedge"]
    Circle = rl["Circle"]
    width = 85 * rl["mm"]
    height = 70 * rl["mm"]
    drawing = Drawing(width, height)
    drawing.add(String(0, height - 10, title, fontName=_font(rl, "bold"), fontSize=9, fillColor=colors.HexColor(BRAND_INK)))
    total = sum(decimal_value(row.get("market_value")) for row in rows)
    cx = 30 * rl["mm"]
    cy = 32 * rl["mm"]
    radius = 22 * rl["mm"]
    if total <= 0:
        drawing.add(Circle(cx, cy, radius, strokeColor=colors.HexColor(BRAND_LINE), fillColor=colors.HexColor(BRAND_SOFT)))
        drawing.add(String(8, 9, "No holdings in scope.", fontName=_font(rl, "regular"), fontSize=8, fillColor=colors.HexColor(BRAND_MUTED)))
        return drawing
    angle = 90.0
    for idx, row in enumerate(rows):
        value = decimal_value(row.get("market_value"))
        sweep = float(value / total) * 360.0
        color = colors.HexColor(PALETTE[idx % len(PALETTE)])
        drawing.add(Wedge(cx, cy, radius, angle, angle + sweep, fillColor=color, strokeColor=colors.white, strokeWidth=0.4))
        angle += sweep
    drawing.add(Circle(cx, cy, radius * 0.52, strokeColor=colors.white, fillColor=colors.white))
    y = height - 24
    for idx, row in enumerate(rows[:6]):
        color = colors.HexColor(PALETTE[idx % len(PALETTE)])
        drawing.add(rl["Rect"](60 * rl["mm"], y - 6, 5, 5, strokeColor=color, fillColor=color))
        label = str(row.get("wallet") or "Wallet")[:18]
        drawing.add(String(60 * rl["mm"] + 8, y - 6, f"{label} {_money(currency, row.get('market_value'))}", fontName=_font(rl, "regular"), fontSize=6.5, fillColor=colors.HexColor(BRAND_MUTED)))
        y -= 9
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
    width = 85 * rl["mm"]
    height = 70 * rl["mm"]
    drawing = Drawing(width, height)
    drawing.add(String(0, height - 10, title, fontName=_font(rl, "bold"), fontSize=9, fillColor=colors.HexColor(BRAND_INK)))
    left = 20
    bottom = 22
    plot_w = width - 28
    plot_h = height - 42
    drawing.add(Line(left, bottom, left + plot_w, bottom, strokeColor=colors.HexColor(BRAND_LINE), strokeWidth=0.5))
    if not rows:
        drawing.add(String(left, bottom + plot_h / 2, "No rows in scope.", fontName=_font(rl, "regular"), fontSize=8, fillColor=colors.HexColor(BRAND_MUTED)))
        return drawing
    if paired:
        values = [decimal_value(row.get("inflow_volume")) for row in rows] + [decimal_value(row.get("outflow_volume")) for row in rows]
    else:
        values = [abs(decimal_value(row.get("realized_pnl"))) for row in rows]
    max_value = max(values) if values else Decimal("0")
    max_value = max_value or Decimal("1")
    bar_slot = plot_w / max(len(rows), 1)
    for idx, row in enumerate(rows):
        x = left + idx * bar_slot + 2
        if paired:
            inflow_h = _scale(decimal_value(row.get("inflow_volume")), Decimal("0"), max_value, plot_h)
            outflow_h = _scale(decimal_value(row.get("outflow_volume")), Decimal("0"), max_value, plot_h)
            drawing.add(Rect(x, bottom, max(bar_slot / 2 - 2, 1), inflow_h, fillColor=colors.HexColor("#2ca02c"), strokeColor=None))
            drawing.add(Rect(x + bar_slot / 2, bottom, max(bar_slot / 2 - 2, 1), outflow_h, fillColor=colors.HexColor(BRAND_ACCENT), strokeColor=None))
        else:
            value = decimal_value(row.get("realized_pnl"))
            bar_h = _scale(abs(value), Decimal("0"), max_value, plot_h)
            color = "#2ca02c" if value >= 0 else BRAND_ACCENT
            drawing.add(Rect(x, bottom, max(bar_slot - 4, 1), bar_h, fillColor=colors.HexColor(color), strokeColor=None))
    drawing.add(String(left, 8, f"Max {_money(currency, max_value)}", fontName=_font(rl, "regular"), fontSize=6.5, fillColor=colors.HexColor(BRAND_MUTED)))
    return drawing


def write_summary_pdf(file_path: str | Path, report: Mapping[str, Any]) -> Mapping[str, Any]:
    rl = require_reportlab("Summary PDF report")
    rl["rl_config"].invariant = 1
    fonts = register_fonts(rl)
    rl["summary_fonts"] = fonts
    path = Path(file_path).expanduser()
    path.parent.mkdir(parents=True, exist_ok=True)
    colors = rl["colors"]
    styles = {
        "title": rl["ParagraphStyle"]("Title", fontName=fonts["bold"], fontSize=22, leading=26, textColor=colors.HexColor(BRAND_INK)),
        "h2": rl["ParagraphStyle"]("H2", fontName=fonts["bold"], fontSize=12, leading=16, spaceBefore=10, spaceAfter=5, textColor=colors.HexColor(BRAND_INK)),
        "body": rl["ParagraphStyle"]("Body", fontName=fonts["regular"], fontSize=8.5, leading=12, textColor=colors.HexColor(BRAND_INK)),
        "muted": rl["ParagraphStyle"]("Muted", fontName=fonts["regular"], fontSize=8, leading=11, textColor=colors.HexColor(BRAND_MUTED)),
    }
    doc = rl["BaseDocTemplate"](
        str(path),
        pagesize=rl["A4"],
        leftMargin=14 * rl["mm"],
        rightMargin=14 * rl["mm"],
        topMargin=18 * rl["mm"],
        bottomMargin=14 * rl["mm"],
        title=str(report.get("title") or "Kassiber Summary Report"),
        author="Kassiber",
    )
    frame = rl["Frame"](doc.leftMargin, doc.bottomMargin, doc.width, doc.height, id="normal")
    footer_left = str(report.get("timeframe", {}).get("label") or "")
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
        ),
    )
    doc.addPageTemplates([template])
    story: list[Any] = []
    currency = str(report.get("fiat_currency") or "")
    metrics = report.get("metrics") or {}
    story.append(_para(rl, styles, report.get("title") or "Kassiber Summary Report", "title"))
    story.append(_para(rl, styles, f"{report.get('workspace')} / {report.get('profile')}", "body"))
    story.append(_para(rl, styles, f"Timeframe: {report.get('timeframe', {}).get('label', '')} · Generated: {report.get('generated_at', '')}", "muted"))
    story.append(rl["Spacer"](1, 6))
    snapshot = report.get("snapshot")
    if snapshot:
        story.append(_para(rl, styles, f"As of today: {_money(currency, snapshot.get('total_market_value'))} · {_btc(snapshot.get('total_quantity'))}", "h2"))
        rows = [["Wallet", "Assets", "Balance", "Market value"]]
        for row in snapshot.get("wallets", []):
            rows.append([row.get("wallet", ""), ", ".join(row.get("assets") or []), _btc(row.get("quantity")), _money(currency, row.get("market_value"))])
        story.append(_table(rl, rows, [44 * rl["mm"], 34 * rl["mm"], 38 * rl["mm"], 45 * rl["mm"]]))
        story.append(rl["Spacer"](1, 8))

    story.append(
        _metric_strip(
            rl,
            [
                ("Start value", _money(currency, metrics.get("period_start_value")), "First period bucket"),
                ("End value", _money(currency, metrics.get("period_end_value")), "Period close"),
                ("Net flow", _signed_money(currency, metrics.get("net_flow")), "Inbound less outbound"),
                ("Realized PnL", _signed_money(currency, metrics.get("realized_pnl")), "Non-tax summary"),
            ],
        )
    )
    story.append(rl["Spacer"](1, 6))
    story.append(_para(rl, styles, f"Fees: {_btc(metrics.get('fees_btc'))} · {_money(currency, metrics.get('fees_fiat'))}", "muted"))
    story.append(rl["PageBreak"]())
    story.append(_para(rl, styles, "Portfolio Movement", "h2"))
    story.append(_line_chart(rl, "Total balance over time", report.get("balance_history") or [], currency))
    story.append(rl["Spacer"](1, 8))
    story.append(
        _table(
            rl,
            [[_donut_chart(rl, "Holdings by wallet", report.get("wallet_holdings") or [], currency), _bar_chart(rl, "Realized PnL per period", report.get("realized_pnl_periods") or [], currency)]],
            [90 * rl["mm"], 90 * rl["mm"]],
            header=False,
        )
    )
    story.append(rl["Spacer"](1, 8))
    story.append(_bar_chart(rl, "Inflows vs outflows volume", report.get("flow_periods") or [], currency, paired=True))
    story.append(rl["PageBreak"]())
    story.append(_para(rl, styles, "Wallet Appendix", "h2"))
    appendix = [["Wallet", "Scope", "Tx count", "End balance", "End value"]]
    for row in report.get("wallet_appendix") or []:
        appendix.append([row.get("wallet", ""), row.get("scope", ""), row.get("tx_count", 0), _btc(row.get("end_quantity")), _money(currency, row.get("end_market_value"))])
    story.append(_table(rl, appendix, [42 * rl["mm"], 38 * rl["mm"], 22 * rl["mm"], 38 * rl["mm"], 42 * rl["mm"]]))
    story.append(rl["Spacer"](1, 8))
    story.append(_para(rl, styles, "This summary report is a portfolio and treasury view. It intentionally omits tax tables; use the tax PDF for tax filing support.", "muted"))
    doc.build(story)
    return {
        "file": str(path.resolve()),
        "pages": doc.page,
        "bytes": path.stat().st_size,
        "title": str(report.get("title") or "Kassiber Summary Report"),
    }
