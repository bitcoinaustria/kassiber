from __future__ import annotations

from decimal import Decimal
from pathlib import Path
from typing import Any, Iterable, Mapping, Sequence

from ._pdf_common import (
    BRAND_INK,
    BRAND_LINE,
    BRAND_MUTED,
    BRAND_SOFT,
    decimal_value as _decimal,
    draw_page_header,
    escape_paragraph_text as _escape,
    register_fonts,
    require_reportlab,
)


def _btc(value: Any) -> str:
    return f"{_decimal(value).quantize(Decimal('0.00000001')):.8f}"


def _money(value: Any) -> str:
    amount = _decimal(value).quantize(Decimal("0.01"))
    return f"{amount:,.2f}".replace(",", "X").replace(".", ",").replace("X", ".")


def _datetime(value: Any) -> str:
    if not value:
        return ""
    text = str(value).replace("T", " ")
    if text.endswith("Z"):
        text = text[:-1]
    if len(text) >= 19 and text[4] == "-" and text[7] == "-":
        return f"{text[:10]} {text[11:16]}"
    return text


def _label(value: Any) -> str:
    return str(value or "").replace("_", " ")


def _fiat(value: Any, currency: Any) -> str:
    if value is None or value == "":
        return ""
    suffix = f" {currency}" if currency else ""
    return f"{_money(value)}{suffix}"


def _pct(value: Any) -> str:
    if value is None or value == "":
        return ""
    return f"{_decimal(value).quantize(Decimal('0.1'))}%"


def _amount_with_asset(value: Any, asset: Any) -> str:
    if value is None or value == "":
        return ""
    suffix = f" {asset}" if asset else ""
    return f"{_btc(value)}{suffix}"


def _node_time(node: Mapping[str, Any]) -> str:
    return _datetime(node.get("occurred_at") or node.get("acquired_at") or "")


def _report_title(report: Mapping[str, Any]) -> str:
    context = report.get("report_context") or {}
    return str(context.get("report_title") or "Source of Funds Report")


def _explorer_links_by_txid(report: Mapping[str, Any]) -> dict[str, Mapping[str, Any]]:
    preview = report.get("disclosure_preview") or {}
    links = preview.get("explorer_links") or []
    return {
        str(link.get("txid")): link
        for link in links
        if isinstance(link, Mapping) and link.get("txid") and link.get("url")
    }


def _fit_canvas_text(canvas: Any, text: Any, max_width: float, font: str, size: float) -> str:
    rendered = str(text or "")
    if canvas.stringWidth(rendered, font, size) <= max_width:
        return rendered
    suffix = "..."
    available = max_width - canvas.stringWidth(suffix, font, size)
    if available <= 0:
        return suffix
    while rendered and canvas.stringWidth(rendered, font, size) > available:
        rendered = rendered[:-1]
    return f"{rendered.rstrip()}{suffix}"


def _make_simplified_flow_chart(rl: dict[str, Any], fonts: dict[str, str], flow: Mapping[str, Any]) -> Any:
    Flowable = rl["Flowable"]

    class SimplifiedFlowChart(Flowable):
        def __init__(self) -> None:
            super().__init__()
            self._width = 0.0
            self._height = 0.0
            self._levels = self._visible_levels()

        def _visible_levels(self) -> list[dict[str, Any]]:
            levels = [dict(level) for level in flow.get("levels") or []]
            if len(levels) > 6:
                hidden = len(levels) - 5
                levels = [
                    *levels[:3],
                    {
                        "role": "omitted",
                        "nodes": [
                            {
                                "id": "synthetic:omitted",
                                "label": f"{hidden} path levels omitted",
                                "kind": "continued path",
                                "asset": "",
                                "amount": None,
                                "deferred_privacy_hop": False,
                            }
                        ],
                    },
                    *levels[-2:],
                ]
            visible: list[dict[str, Any]] = []
            for level in levels:
                nodes = list(level.get("nodes") or [])
                if len(nodes) > 4:
                    extra = len(nodes) - 3
                    nodes = [
                        *nodes[:3],
                        {
                            "id": f"synthetic:extra:{level.get('level', len(visible))}",
                            "label": f"{extra} more items",
                            "kind": "additional reviewed nodes",
                            "asset": "",
                            "amount": None,
                            "deferred_privacy_hop": False,
                        },
                    ]
                next_level = dict(level)
                next_level["nodes"] = nodes
                visible.append(next_level)
            return visible

        def wrap(self, availWidth: float, availHeight: float) -> tuple[float, float]:
            max_rows = max((len(level.get("nodes") or []) for level in self._levels), default=1)
            box_h = 34
            row_gap = 8
            self._width = availWidth
            self._height = max(68, 18 + (max_rows * box_h) + max(0, max_rows - 1) * row_gap + 10)
            return self._width, self._height

        def _positions(self) -> dict[str, tuple[float, float, float, float]]:
            level_count = max(1, len(self._levels))
            col_gap = 8
            box_h = 34
            row_gap = 8
            col_w = (self._width - col_gap * (level_count - 1)) / level_count
            box_w = max(52, col_w)
            usable_h = self._height - 18
            positions: dict[str, tuple[float, float, float, float]] = {}
            for col, level in enumerate(self._levels):
                nodes = list(level.get("nodes") or [])
                group_h = len(nodes) * box_h + max(0, len(nodes) - 1) * row_gap
                base_y = max(4, (usable_h - group_h) / 2)
                x = col * (col_w + col_gap)
                for row, node in enumerate(nodes):
                    y = base_y + (len(nodes) - row - 1) * (box_h + row_gap)
                    node_id = str(node.get("id") or f"synthetic:{col}:{row}")
                    positions[node_id] = (x, y, box_w, box_h)
            return positions

        def _draw_arrow(
            self,
            x1: float,
            y1: float,
            x2: float,
            y2: float,
            *,
            deferred: bool,
        ) -> None:
            canvas = self.canv
            colors = rl["colors"]
            canvas.saveState()
            canvas.setStrokeColor(colors.HexColor("#a3a3a3" if deferred else BRAND_MUTED))
            canvas.setLineWidth(0.7)
            if deferred:
                canvas.setDash(2, 2)
            end_x = max(x1 + 5, x2 - 5)
            canvas.line(x1 + 2, y1, end_x, y2)
            canvas.setDash()
            canvas.line(end_x, y2, end_x - 4, y2 + 2.5)
            canvas.line(end_x, y2, end_x - 4, y2 - 2.5)
            canvas.restoreState()

        def draw(self) -> None:
            canvas = self.canv
            colors = rl["colors"]
            positions = self._positions()

            for edge in flow.get("edges") or []:
                from_box = positions.get(str(edge.get("from") or ""))
                to_box = positions.get(str(edge.get("to") or ""))
                if not from_box or not to_box:
                    continue
                from_x, from_y, from_w, from_h = from_box
                to_x, to_y, _to_w, to_h = to_box
                if to_x <= from_x:
                    continue
                self._draw_arrow(
                    from_x + from_w,
                    from_y + from_h / 2,
                    to_x,
                    to_y + to_h / 2,
                    deferred=bool(edge.get("deferred_privacy_hop")),
                )

            for col, level in enumerate(self._levels):
                role = _label(level.get("role") or "flow")
                nodes = list(level.get("nodes") or [])
                if not nodes:
                    continue
                first_box = positions.get(str(nodes[0].get("id") or ""))
                if first_box:
                    x, _y, w, _h = first_box
                    canvas.setFont(fonts["bold"], 6.5)
                    canvas.setFillColor(colors.HexColor(BRAND_MUTED))
                    canvas.drawCentredString(x + w / 2, self._height - 7, role.upper())
                for row, node in enumerate(nodes):
                    box = positions.get(str(node.get("id") or f"synthetic:{col}:{row}"))
                    if not box:
                        continue
                    x, y, w, h = box
                    deferred = bool(node.get("deferred_privacy_hop"))
                    is_target = level.get("role") == "target"
                    is_source = node.get("node_type") == "source"
                    fill = "#fff1f2" if deferred else BRAND_SOFT if is_source else "#ffffff"
                    stroke = "#e3000f" if is_target or deferred else BRAND_LINE
                    canvas.setFillColor(colors.HexColor(fill))
                    canvas.setStrokeColor(colors.HexColor(stroke))
                    canvas.setLineWidth(0.9 if is_target or deferred else 0.5)
                    canvas.roundRect(x, y, w, h, 4, stroke=1, fill=1)

                    label = _fit_canvas_text(canvas, node.get("label"), w - 8, fonts["bold"], 6.6)
                    kind = "privacy hop deferred" if deferred else _label(node.get("kind"))
                    kind = _fit_canvas_text(canvas, kind, w - 8, fonts["regular"], 5.8)
                    amount = _fit_canvas_text(
                        canvas,
                        _amount_with_asset(node.get("amount"), node.get("asset")),
                        w - 8,
                        fonts["mono"],
                        5.7,
                    )
                    canvas.setFillColor(colors.HexColor(BRAND_INK))
                    canvas.setFont(fonts["bold"], 6.6)
                    canvas.drawString(x + 4, y + h - 10, label)
                    canvas.setFont(fonts["regular"], 5.8)
                    canvas.setFillColor(colors.HexColor(BRAND_MUTED))
                    canvas.drawString(x + 4, y + h - 20, kind)
                    if amount:
                        canvas.setFont(fonts["mono"], 5.7)
                        canvas.drawString(x + 4, y + 6, amount)

    return SimplifiedFlowChart()


class _SourceFundsPdfBuilder:
    def __init__(
        self,
        *,
        report: Mapping[str, Any],
        generated_at: str,
        rl: dict[str, Any],
        fonts: dict[str, str],
        snapshot_hash: str,
    ) -> None:
        self.report = report
        self.generated_at = generated_at
        self.rl = rl
        self.fonts = fonts
        self.snapshot_hash = snapshot_hash
        self.styles = self._styles()

    def _styles(self) -> dict[str, Any]:
        ParagraphStyle = self.rl["ParagraphStyle"]
        return {
            "cover_title": ParagraphStyle(
                "KassiberSourceFundsCoverTitle",
                fontName=self.fonts["bold"],
                fontSize=27,
                leading=32,
                textColor=BRAND_INK,
                spaceAfter=9,
            ),
            "cover_subtitle": ParagraphStyle(
                "KassiberSourceFundsCoverSubtitle",
                fontName=self.fonts["regular"],
                fontSize=14,
                leading=18,
                textColor=BRAND_MUTED,
                spaceAfter=17,
            ),
            "h1": ParagraphStyle(
                "KassiberSourceFundsH1",
                fontName=self.fonts["bold"],
                fontSize=16,
                leading=20,
                textColor=BRAND_INK,
                spaceBefore=4,
                spaceAfter=8,
            ),
            "h2": ParagraphStyle(
                "KassiberSourceFundsH2",
                fontName=self.fonts["bold"],
                fontSize=11.5,
                leading=14,
                textColor=BRAND_INK,
                spaceBefore=8,
                spaceAfter=5,
            ),
            "body": ParagraphStyle(
                "KassiberSourceFundsBody",
                fontName=self.fonts["regular"],
                fontSize=8.8,
                leading=11.3,
                textColor=BRAND_INK,
                spaceAfter=5,
            ),
            "small": ParagraphStyle(
                "KassiberSourceFundsSmall",
                fontName=self.fonts["regular"],
                fontSize=7.4,
                leading=9.3,
                textColor=BRAND_MUTED,
            ),
            "mono": ParagraphStyle(
                "KassiberSourceFundsMono",
                fontName=self.fonts["mono"],
                fontSize=7.1,
                leading=8.8,
                textColor=BRAND_INK,
            ),
            "table_header": ParagraphStyle(
                "KassiberSourceFundsTableHeader",
                fontName=self.fonts["bold"],
                fontSize=7.6,
                leading=9.4,
                textColor=BRAND_INK,
            ),
        }

    def p(self, text: Any, style: str = "body") -> Any:
        return self.rl["Paragraph"](_escape(text), self.styles[style])

    def link_p(self, text: Any, url: Any, style: str = "small") -> Any:
        label = _escape(text)
        href = _escape(url)
        return self.rl["Paragraph"](
            f'<link href="{href}"><font color="#0f766e"><u>{label}</u></font></link>',
            self.styles[style],
        )

    def spacer(self, height_mm: float) -> Any:
        return self.rl["Spacer"](1, height_mm * self.rl["mm"])

    def table(
        self,
        rows: Sequence[Sequence[Any]],
        *,
        widths: Sequence[float] | None = None,
        header: bool = True,
        repeat: bool = True,
        compact: bool = False,
        right_columns: Iterable[int] = (),
        style: str = "body",
    ) -> Any:
        Table = self.rl["Table"]
        TableStyle = self.rl["TableStyle"]
        colors = self.rl["colors"]
        mm = self.rl["mm"]
        data = []
        for row_index, row in enumerate(rows):
            rendered = []
            for cell in row:
                if hasattr(cell, "wrap"):
                    rendered.append(cell)
                elif header and row_index == 0:
                    rendered.append(self.p(cell, "table_header"))
                else:
                    rendered.append(self.p(cell, "small" if compact else style))
            data.append(rendered)
        table = Table(
            data,
            colWidths=[width * mm for width in widths] if widths else None,
            repeatRows=1 if header and repeat else 0,
            hAlign="LEFT",
            splitByRow=True,
        )
        commands: list[tuple[Any, ...]] = [
            ("BOX", (0, 0), (-1, -1), 0.4, colors.HexColor(BRAND_LINE)),
            ("INNERGRID", (0, 0), (-1, -1), 0.25, colors.HexColor(BRAND_LINE)),
            ("VALIGN", (0, 0), (-1, -1), "TOP"),
            ("LEFTPADDING", (0, 0), (-1, -1), 4),
            ("RIGHTPADDING", (0, 0), (-1, -1), 4),
            ("TOPPADDING", (0, 0), (-1, -1), 3 if compact else 4),
            ("BOTTOMPADDING", (0, 0), (-1, -1), 3 if compact else 4),
        ]
        if header and rows:
            commands.extend(
                [
                    ("BACKGROUND", (0, 0), (-1, 0), colors.HexColor(BRAND_SOFT)),
                    ("LINEBELOW", (0, 0), (-1, 0), 0.7, colors.HexColor(BRAND_INK)),
                ]
            )
        for row_index in range(1 if header else 0, len(rows)):
            if row_index % 2 == 0:
                commands.append(
                    ("BACKGROUND", (0, row_index), (-1, row_index), colors.HexColor("#fbfbfb"))
                )
        for column in right_columns:
            commands.append(("ALIGN", (column, 0), (column, -1), "RIGHT"))
        table.setStyle(TableStyle(commands))
        return table

    def kv_table(self, rows: Sequence[tuple[str, Any]], *, widths: Sequence[float] = (43, 118)) -> Any:
        return self.table(
            [[label, value] for label, value in rows],
            widths=widths,
            header=False,
            repeat=False,
            compact=True,
        )

    def cover(self) -> list[Any]:
        target = self.report["target"]
        purpose = self.report.get("purpose", {})
        recipient = self.report.get("recipient") or {}
        context = self.report.get("report_context") or {}
        target_label = "Funds anchor" if purpose.get("type") == "planned_exchange_sale" else "Target"
        rows: list[tuple[str, Any]] = [
            ("Generated at", self.generated_at),
            ("Workspace", self.report.get("workspace", "")),
            ("Profile", self.report.get("profile", "")),
            ("Jurisdiction", context.get("jurisdiction_label") or ""),
            ("Fiat currency", context.get("fiat_currency") or ""),
            ("Purpose", purpose.get("label", "Already completed transaction")),
            ("Reveal mode", self.report.get("reveal_mode", "")),
            (target_label, target.get("label", "")),
            ("Amount", f"{_btc(target.get('required_amount'))} {target.get('asset', '')}"),
            ("Exportable", str(bool(self.report.get("explain_gates", {}).get("exportable")))),
            ("Snapshot hash", self.snapshot_hash),
        ]
        if recipient:
            rows.insert(6, ("Recipient", f"{recipient.get('label', '')} ({recipient.get('kind', '')})"))
        story: list[Any] = [
            self.p(f"Kassiber {_report_title(self.report)}", "cover_title"),
            self.p(
                context.get("report_subtitle")
                or "Reviewed local evidence disclosure from a saved immutable case snapshot.",
                "cover_subtitle",
            ),
            self.kv_table(rows),
        ]
        if purpose.get("type") == "planned_exchange_sale":
            story.extend(
                [
                    self.spacer(5),
                    self.p("Planned Sale", "h2"),
                    self.kv_table(
                        [
                            ("Destination", purpose.get("planned_destination") or "(not specified)"),
                            ("Note", purpose.get("planned_note") or "(none)"),
                            ("Fiat-source note", purpose.get("fiat_purchase_note") or ""),
                        ]
                    ),
                ]
            )
        return story

    def evidence_checklist(self) -> list[Any]:
        context = self.report.get("report_context") or {}
        checklist = list(context.get("evidence_checklist") or [])
        story: list[Any] = [self.p("Evidence Checklist", "h1")]
        if not checklist:
            story.append(self.p("No evidence checklist is defined for this report context."))
            return story
        rows = [["#", "Requirement"]]
        for index, item in enumerate(checklist, start=1):
            rows.append([index, item])
        story.append(self.table(rows, widths=(10, 150), compact=True, right_columns={0}))
        deferred = list(context.get("deferred") or [])
        if deferred:
            story.append(
                self.p(
                    "Deferred for later templates: " + "; ".join(str(item) for item in deferred) + ".",
                    "small",
                )
            )
        return story

    def review_gates(self) -> list[Any]:
        gates = self.report.get("explain_gates", {})
        story: list[Any] = [self.p("Review Gates", "h1")]
        story.append(
            self.kv_table(
                [
                    ("Status", "Exportable" if gates.get("exportable") else "Blocked"),
                    ("Blockers", len(gates.get("blockers") or [])),
                    ("Warnings", len(gates.get("warnings") or [])),
                ],
                widths=(34, 70),
            )
        )
        findings = list(self.report.get("findings") or [])
        if findings:
            rows = [["Severity", "Code", "Message", "Reference"]]
            for finding in findings:
                next_step = finding.get("next_step") if isinstance(finding, Mapping) else None
                message = str(finding.get("message", ""))
                if isinstance(next_step, Mapping) and next_step.get("headline"):
                    message = f"{message} Next step: {next_step['headline']}"
                rows.append(
                    [finding.get("severity", ""), finding.get("code", ""), message, finding.get("ref", "")]
                )
            story.extend(
                [self.spacer(5), self.table(rows, widths=(22, 30, 83, 35), compact=True)]
            )
        else:
            story.append(self.p("No blockers or warnings."))
        return story

    def overview(self) -> list[Any]:
        overview = self.report.get("overview") or {}
        target = self.report.get("target") or {}
        time_range = overview.get("time_range") or {}
        story: list[Any] = [self.p("Source of Funds Overview", "h1")]
        target_amount = _amount_with_asset(
            overview.get("target_amount"),
            overview.get("target_asset") or target.get("asset"),
        )
        fiat_value = _fiat(
            overview.get("target_fiat_value"),
            overview.get("target_fiat_currency") or target.get("fiat_currency"),
        )
        story.append(
            self.kv_table(
                [
                    ("Target", overview.get("target_label") or target.get("label", "")),
                    ("Date", _datetime(overview.get("target_date"))),
                    ("Wallet/source", overview.get("target_wallet") or target.get("wallet", "")),
                    ("Amount", target_amount),
                    ("Fiat value", fiat_value or "(not priced)"),
                    (
                        "Time range",
                        f"{_datetime(time_range.get('start'))} - {_datetime(time_range.get('end'))}",
                    ),
                    ("Transactions", overview.get("transaction_count", 0)),
                    ("Reviewed links", overview.get("link_count", 0)),
                    ("Data sources", overview.get("data_source_count", 0)),
                    ("Source categories", overview.get("source_category_count", 0)),
                ],
                widths=(38, 122),
            )
        )
        return story

    def narrative(self) -> list[Any]:
        narrative = self.report.get("narrative") or {}
        paragraphs = list(narrative.get("paragraphs") or [])
        story: list[Any] = [self.p("Origin and Transaction Flow", "h1")]
        if paragraphs:
            for paragraph in paragraphs:
                story.append(self.p(paragraph))
            if narrative.get("generated_by") == "local_rule_summary":
                story.append(
                    self.p(
                        "Summary generated locally from the saved review graph; no external AI service was used.",
                        "small",
                    )
                )
        else:
            story.append(self.p("No local narrative is available for this case snapshot."))
        return story

    def simplified_flow(self) -> list[Any]:
        flow = self.report.get("simplified_flow") or {}
        levels = list(flow.get("levels") or [])
        story: list[Any] = [self.p("Simplified Flow Path", "h1")]
        if not levels:
            story.append(self.p("No simplified flow path is available for this case snapshot."))
            return story
        note = flow.get("note")
        if note:
            story.append(self.p(note, "small"))
        story.append(_make_simplified_flow_chart(self.rl, self.fonts, flow))
        deferred = list(flow.get("deferred_privacy_hops") or [])
        if deferred:
            story.append(
                self.p(
                    (
                        f"CoinJoin/PayJoin traversal deferred for {len(deferred)} reviewed "
                        f"privacy hop{'' if len(deferred) == 1 else 's'}; supporting evidence "
                        "is listed separately and unrelated participant inputs are not disclosed."
                    ),
                    "small",
                )
            )
        return story

    def source_mix(self) -> list[Any]:
        story: list[Any] = [self.p("Source Mix", "h1")]
        rows = [["Source", "Amount", "Asset", "Share", "Count"]]
        for row in self.report.get("source_mix") or []:
            rows.append(
                [
                    _label(row.get("source_type")),
                    _btc(row.get("amount")),
                    self.report.get("allocations", {}).get("asset", ""),
                    _pct(row.get("percent_of_target")),
                    row.get("count", 0),
                ]
            )
        if len(rows) == 1:
            story.append(self.p("No reviewed root sources yet."))
        else:
            story.append(
                self.table(rows, widths=(58, 28, 22, 22, 16), compact=True, right_columns={1, 3, 4})
            )
        return story

    def data_sources(self) -> list[Any]:
        story: list[Any] = [self.p("Data Sources", "h1")]
        rows = [["Name", "Kind", "Transactions", "Sources", "Assets", "Period"]]
        for item in self.report.get("data_sources") or []:
            period = ""
            if item.get("first_seen") or item.get("last_seen"):
                period = f"{_datetime(item.get('first_seen'))} - {_datetime(item.get('last_seen'))}"
            rows.append(
                [
                    item.get("label", ""),
                    _label(item.get("kind")),
                    item.get("transaction_count", 0),
                    item.get("source_count", 0),
                    ", ".join(item.get("assets") or []),
                    period,
                ]
            )
        if len(rows) == 1:
            story.append(self.p("No data-source rollups in this case snapshot."))
        else:
            story.append(
                self.table(rows, widths=(42, 28, 17, 16, 20, 45), compact=True, right_columns={2, 3})
            )
        return story

    def flow_levels(self) -> list[Any]:
        story: list[Any] = [self.p("Flow Diagram Data", "h1")]
        levels = list(self.report.get("flow_levels") or [])
        if not levels:
            story.append(self.p("No flow levels in this case snapshot."))
            return story
        rows = [["Level", "Role", "Node", "Type", "Date", "Amount"]]
        for level in levels:
            for node in level.get("nodes") or []:
                node_type = node.get("source_type") or node.get("direction") or node.get("node_type")
                amount = _amount_with_asset(
                    node.get("required_amount") if node.get("required_amount") is not None else node.get("amount"),
                    node.get("asset"),
                )
                rows.append(
                    [
                        level.get("level", ""),
                        _label(level.get("role")),
                        node.get("label", ""),
                        _label(node_type),
                        _node_time(node),
                        amount,
                    ]
                )
        story.append(
            self.table(rows, widths=(12, 20, 50, 28, 31, 28), compact=True, right_columns={0, 5})
        )
        return story

    def transaction_details(self) -> list[Any]:
        story: list[Any] = [self.p("Transaction Details", "h1")]
        levels = list(self.report.get("flow_levels") or [])
        tx_rows: list[list[Any]] = [["Level", "Date", "Source", "Type", "Amount", "Asset", "Fiat", "ID"]]
        for level in levels:
            for node in level.get("nodes") or []:
                if node.get("node_type") != "transaction":
                    continue
                tx_rows.append(
                    [
                        level.get("level", ""),
                        _node_time(node),
                        node.get("wallet", ""),
                        _label(node.get("direction")),
                        _btc(node.get("required_amount") if node.get("required_amount") is not None else node.get("amount")),
                        node.get("asset", ""),
                        _fiat(node.get("fiat_value"), node.get("fiat_currency")),
                        node.get("external_id") or node.get("label", ""),
                    ]
                )
        if len(tx_rows) == 1:
            story.append(self.p("No transaction-level rows in this case snapshot."))
        else:
            story.append(
                self.table(
                    tx_rows,
                    widths=(10, 24, 28, 20, 22, 13, 21, 34),
                    compact=True,
                    right_columns={0, 4, 6},
                )
            )
        return story

    def flow_links(self) -> list[Any]:
        story: list[Any] = [self.p("Reviewed Flow Links", "h1")]
        edges = list((self.report.get("graph") or {}).get("edges") or [])
        if not edges:
            story.append(self.p("No reviewed links yet."))
            return story
        rows = [["Type", "State", "Method", "Amount", "Policy", "Explanation"]]
        for edge in edges:
            rows.append(
                [
                    _label(edge.get("link_type")),
                    edge.get("state", ""),
                    _label(edge.get("method")),
                    f"{_btc(edge.get('allocation_amount'))} {edge.get('asset', '')}",
                    _label(edge.get("allocation_policy")),
                    edge.get("explanation") or "",
                ]
            )
        story.append(
            self.table(rows, widths=(22, 20, 28, 28, 22, 52), compact=True, right_columns={3})
        )
        return story

    def graph_nodes(self) -> list[Any]:
        story: list[Any] = [self.p("Disclosure Graph Nodes", "h1")]
        nodes = list((self.report.get("graph") or {}).get("nodes") or [])
        if not nodes:
            story.append(self.p("No graph nodes."))
            return story
        rows = [["Kind", "Label", "Date", "Asset", "Amount", "Details"]]
        for node in nodes:
            if node.get("node_type") == "source":
                details = []
                if node.get("fiat_value") not in (None, ""):
                    details.append(_fiat(node.get("fiat_value"), node.get("fiat_currency")))
                if node.get("description"):
                    details.append(node.get("description", ""))
                rows.append(
                    [
                        _label(node.get("source_type") or "source"),
                        node.get("label", ""),
                        _datetime(node.get("acquired_at")),
                        node.get("asset", ""),
                        _btc(node.get("required_amount")),
                        " | ".join(details),
                    ]
                )
            else:
                details = []
                if node.get("wallet"):
                    details.append(f"Wallet: {node['wallet']}")
                if node.get("direction"):
                    details.append(_label(node["direction"]))
                if node.get("description"):
                    details.append(node.get("description", ""))
                rows.append(
                    [
                        "transaction",
                        node.get("label", ""),
                        _datetime(node.get("occurred_at")),
                        node.get("asset", ""),
                        _btc(node.get("required_amount")),
                        " | ".join(details),
                    ]
                )
        story.append(
            self.table(rows, widths=(28, 36, 25, 17, 25, 41), compact=True, right_columns={4})
        )
        return story

    def disclosure_preview(self) -> list[Any]:
        preview = self.report.get("disclosure_preview", {})
        story: list[Any] = [self.p("Disclosure Preview", "h1")]
        txids = preview.get("txids") or []
        explorer_links = _explorer_links_by_txid(self.report)
        if txids:
            rows: list[list[Any]] = [["Txid", "Explorer"]]
            for txid in txids:
                link = explorer_links.get(str(txid))
                rows.append(
                    [
                        str(txid),
                        self.link_p(link.get("label") or "Open explorer", link.get("url"))
                        if link
                        else "(not a public on-chain txid)",
                    ]
                )
            story.append(self.table(rows, widths=(112, 58), compact=True))
        else:
            story.append(self.kv_table([("Txids", "(none in this reveal mode)")]))
        attachments = list(preview.get("attachments") or [])
        if attachments:
            rows = [["Label", "Type", "Media/SHA256", "Location"]]
            for item in attachments:
                media = item.get("media_type") or ""
                if item.get("sha256"):
                    media = f"{media} {item['sha256']}".strip()
                rows.append(
                    [
                        item.get("label", ""),
                        item.get("attachment_type", ""),
                        media,
                        item.get("source_url") or item.get("stored_relpath") or "",
                    ]
                )
            story.extend(
                [self.spacer(5), self.table(rows, widths=(55, 33, 40, 42), compact=True)]
            )
        else:
            story.append(self.p("No evidence attachments in this disclosure."))
        story.append(self.p(preview.get("privacy_note", "")))
        excluded = preview.get("excluded") or []
        if excluded:
            story.append(self.kv_table([("Excluded from disclosure", ", ".join(excluded))]))
        return story

    def limitations(self) -> list[Any]:
        return [
            self.p("Limitations", "h1"),
            self.p(
                "Kassiber reports reviewed local evidence. It does not certify ownership, "
                "perform chain-surveillance scoring, or provide legal or AML advice."
            ),
            self.p("Opening balances are rendered as attested prior-history stops, not as real root sources."),
            self.p("Suggested links and unconfirmed chain observations are never used as PDF proof."),
        ]

    def build(self) -> list[Any]:
        story = self.cover()
        story.append(self.rl["PageBreak"]())
        for section in (
            self.overview(),
            self.narrative(),
            self.simplified_flow(),
            self.data_sources(),
            self.evidence_checklist(),
            self.review_gates(),
            self.source_mix(),
            self.flow_levels(),
            self.transaction_details(),
            self.flow_links(),
            self.graph_nodes(),
            self.disclosure_preview(),
            self.limitations(),
        ):
            story.extend(section)
            story.append(self.spacer(7))
        return story


def _on_page(canvas: Any, doc: Any, *, title: str, fonts: dict[str, str], rl: dict[str, Any]) -> None:
    draw_page_header(
        canvas,
        doc,
        title=title,
        fonts=fonts,
        rl=rl,
        brand_label="",
        page_label="Page",
        line_width=0.3,
    )


def write_source_funds_pdf(
    file_path: str | Path,
    *,
    report: Mapping[str, Any],
    generated_at: str,
    snapshot_hash: str,
) -> dict[str, Any]:
    rl = require_reportlab("Source-of-funds PDF export")
    rl["rl_config"].warnOnMissingFontGlyphs = 0
    fonts = register_fonts(rl)
    path = Path(file_path).expanduser()
    path.parent.mkdir(parents=True, exist_ok=True)

    BaseDocTemplate = rl["BaseDocTemplate"]
    Frame = rl["Frame"]
    PageTemplate = rl["PageTemplate"]
    A4 = rl["A4"]
    mm = rl["mm"]

    title = f"Kassiber {_report_title(report)}"
    doc = BaseDocTemplate(
        str(path),
        pagesize=A4,
        leftMargin=17 * mm,
        rightMargin=17 * mm,
        topMargin=18 * mm,
        bottomMargin=15 * mm,
        pageCompression=0,
        title=title,
        author="Kassiber",
        subject="Source-of-funds evidence report",
    )
    frame = Frame(doc.leftMargin, doc.bottomMargin, doc.width, doc.height, id="portrait")
    doc.addPageTemplates(
        [
            PageTemplate(
                id="portrait",
                frames=[frame],
                pagesize=A4,
                onPage=lambda canvas, document: _on_page(canvas, document, title=title, fonts=fonts, rl=rl),
            )
        ]
    )
    builder = _SourceFundsPdfBuilder(
        report=report,
        generated_at=generated_at,
        rl=rl,
        fonts=fonts,
        snapshot_hash=snapshot_hash,
    )
    doc.build(builder.build())
    return {
        "file": str(path.resolve()),
        "pages": max(1, int(getattr(doc, "page", 0) or 0)),
        "bytes": path.stat().st_size,
        "title": title,
        "renderer": "reportlab",
    }
