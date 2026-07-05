from __future__ import annotations

from decimal import Decimal
from pathlib import Path
from typing import Any, Iterable, Mapping, Sequence

from ._pdf_common import (
    BRAND_LINK,
    build_report_styles,
    build_report_table,
    scale_widths,
    decimal_value as _decimal,
    draw_page_header,
    escape_paragraph_text as _escape,
    register_fonts,
    require_reportlab,
)

from .core.source_funds_diagram import (
    build_flow_drawing,
    build_ring_drawing,
    data_source_segments,
    detail_thresholds,
    source_mix_segments,
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


# Compact per-row provenance labels for the level tables; the data-sources
# section carries the long-form wording.
_PROVENANCE_SHORT = {
    "chain_sync": "chain",
    "platform_export": "platform",
    "manual_import": "manual",
}

# The wide detail tables (up to 9 columns) are a contiguous block near the
# end of the report; they are rendered as a single landscape island so the
# reader rotates the page once, not per section.
_LANDSCAPE_SECTION_KEYS = frozenset(
    {"flow_levels", "transaction_details", "flow_links", "graph_nodes"}
)


# Reviewer-facing fallback wording when a reviewed link has no free-text
# explanation (or the reveal mode redacts it).
_LINK_TYPE_EXPLANATIONS = {
    "self_transfer": "Reviewed transfer between wallets of the same holder.",
    "exchange_transfer": "Reviewed transfer between an exchange account and an owned wallet.",
    "trade": "Reviewed trade execution on a platform.",
    "swap": "Reviewed cross-asset swap.",
    "peg_in": "Reviewed Liquid peg-in.",
    "peg_out": "Reviewed Liquid peg-out.",
    "lightning_funding": "Reviewed Lightning channel funding.",
    "lightning_close": "Reviewed Lightning channel close.",
    "lightning_routed": "Reviewed routed Lightning payment.",
    "lightning_swap": "Reviewed Lightning submarine swap.",
    "coinjoin": "Reviewed CoinJoin boundary (traversal deferred).",
    "payjoin": "Reviewed PayJoin boundary (traversal deferred).",
    "manual_source": "Reviewed link to a documented root source.",
    "missing_history": "Reviewed missing-history attestation.",
}


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
        options = report.get("report_options") or {}
        self.amount_precision = str(options.get("amount_precision") or "btc")
        self.mask_recipient = bool(options.get("mask_recipient"))
        self.omitted_sections = set(options.get("omit_sections") or [])

    def amt(self, value: Any, asset: Any = "") -> str:
        """Format a BTC amount honoring the advanced amount-precision option."""
        if value is None or value == "":
            return ""
        if self.amount_precision == "sats":
            sats = int((_decimal(value) * Decimal(100_000_000)).to_integral_value())
            rendered = f"{sats:,} sats".replace(",", " ")
            return rendered
        suffix = f" {asset}" if asset else ""
        return f"{_btc(value)}{suffix}"

    def _styles(self) -> dict[str, Any]:
        return build_report_styles(self.rl, self.fonts, prefix="KassiberSourceFunds")

    def p(self, text: Any, style: str = "body") -> Any:
        return self.rl["Paragraph"](_escape(text), self.styles[style])

    def link_p(self, text: Any, url: Any, style: str = "small") -> Any:
        label = _escape(text)
        href = _escape(url)
        return self.rl["Paragraph"](
            f'<link href="{href}"><font color="{BRAND_LINK}"><u>{label}</u></font></link>',
            self.styles[style],
        )

    def spacer(self, height_mm: float) -> Any:
        return self.rl["Spacer"](1, height_mm * self.rl["mm"])

    def _content_width(self) -> float:
        return float(self.rl["A4"][0] - 34 * self.rl["mm"])

    def _source_mix_segments(self) -> list[dict[str, Any]]:
        return source_mix_segments(self.report)

    def _data_source_segments(self) -> list[dict[str, Any]]:
        return data_source_segments(self.report)

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
        return build_report_table(
            self.rl,
            self.styles,
            rows,
            widths=widths,
            header=header,
            repeat=repeat,
            compact=compact,
            right_columns=right_columns,
            body_style=style,
        )

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
        target_label = "Bitcoin being sold" if purpose.get("type") == "planned_exchange_sale" else "Target"
        rows: list[tuple[str, Any]] = [
            ("Generated at", _datetime(self.generated_at) or self.generated_at),
            ("Workspace", self.report.get("workspace", "")),
            ("Profile", self.report.get("profile", "")),
            ("Jurisdiction", context.get("jurisdiction_label") or ""),
            ("Fiat currency", context.get("fiat_currency") or ""),
            ("Purpose", purpose.get("label", "Already completed transaction")),
            ("Reveal mode", self.report.get("reveal_mode", "")),
            (target_label, target.get("label", "")),
            ("Amount", self.amt(target.get("required_amount"), target.get("asset", ""))),
            ("Exportable", str(bool(self.report.get("explain_gates", {}).get("exportable")))),
            ("Snapshot hash", self.snapshot_hash),
        ]
        recipient_name = "(recipient masked)" if self.mask_recipient else recipient.get("label", "")
        if recipient:
            rows.insert(6, ("Recipient", f"{recipient_name} ({recipient.get('kind', '')})"))
        overview = self.report.get("overview") or {}
        recipient_cell = (
            "(recipient masked)"
            if self.mask_recipient
            else (
                recipient.get("label")
                or purpose.get("planned_destination")
                or target.get("wallet")
                or "(self)"
            )
        )
        glance = self.table(
            [
                ["Date", "Recipient", "Amount", "Fiat value"],
                [
                    _datetime(overview.get("target_date") or target.get("occurred_at")),
                    recipient_cell,
                    self.amt(target.get("required_amount"), target.get("asset", "")),
                    _fiat(overview.get("target_fiat_value"), overview.get("target_fiat_currency"))
                    or "(not priced)",
                ],
            ],
            widths=(40, 56, 40, 40),
            compact=True,
            right_columns={2, 3},
        )
        story: list[Any] = [
            self.p(f"Kassiber {_report_title(self.report)}", "cover_title"),
            self.p(
                context.get("report_subtitle")
                or "Reviewed local evidence disclosure from a saved immutable case snapshot.",
                "cover_subtitle",
            ),
            glance,
            self.spacer(4),
            self.kv_table(rows),
            self.spacer(5),
            self.p("Contents", "h2"),
        ]
        for index, title in enumerate(self._included_section_titles(), start=1):
            story.append(self.p(f"{index}. {title}", "body"))
        if purpose.get("type") == "planned_exchange_sale":
            planned_destination = (
                "(recipient masked)"
                if self.mask_recipient
                else purpose.get("planned_destination") or "(not specified)"
            )
            planned_note = (
                "(recipient masked)"
                if self.mask_recipient
                else purpose.get("planned_note") or "(none)"
            )
            story.extend(
                [
                    self.spacer(5),
                    self.p("Planned Sale", "h2"),
                    self.kv_table(
                        [
                            ("Destination", planned_destination),
                            ("Note", planned_note),
                            ("Fiat-source note", purpose.get("fiat_purchase_note") or ""),
                        ]
                    ),
                ]
            )
        flow = self.report.get("simplified_flow") or {}
        if flow.get("levels"):
            story.extend(
                [
                    self.spacer(5),
                    self.p("Flow at a Glance", "h2"),
                    build_flow_drawing(
                        self.rl,
                        self.fonts,
                        flow,
                        width=self._content_width(),
                        max_levels=4,
                        max_nodes=2,
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
        target_amount = self.amt(
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
        options = self.report.get("report_options") or {}
        max_levels, max_nodes = detail_thresholds(options.get("diagram_detail"))
        story.append(
            build_flow_drawing(
                self.rl,
                self.fonts,
                flow,
                width=self._content_width(),
                max_levels=max_levels,
                max_nodes=max_nodes,
            )
        )
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
        overview = self.report.get("overview") or {}
        segments = self._source_mix_segments()
        if segments:
            story.append(
                build_ring_drawing(
                    self.rl,
                    self.fonts,
                    segments,
                    center_value=_btc(overview.get("target_amount")),
                    center_label=f"{overview.get('target_asset') or 'BTC'} explained",
                    width=self._content_width(),
                )
            )
        rows = [["Source", "Amount", "Asset", "Share", "Count"]]
        for row in self.report.get("source_mix") or []:
            rows.append(
                [
                    _label(row.get("source_type")),
                    self.amt(row.get("amount")),
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
        source_nodes = [
            node
            for node in (self.report.get("graph") or {}).get("nodes") or []
            if node.get("node_type") == "source"
        ]
        if source_nodes:
            detail_rows: list[list[Any]] = [
                ["Date", "Source", "Type", "Amount", "Asset", "Fiat value", "Review"]
            ]
            for node in sorted(source_nodes, key=lambda item: str(item.get("acquired_at") or "")):
                detail_rows.append(
                    [
                        _datetime(node.get("acquired_at")),
                        node.get("label", ""),
                        _label(node.get("source_type")),
                        self.amt(
                            node.get("required_amount")
                            if node.get("required_amount") is not None
                            else node.get("amount")
                        ),
                        node.get("asset", ""),
                        _fiat(node.get("fiat_value"), node.get("fiat_currency")),
                        node.get("review_state", ""),
                    ]
                )
            story.extend(
                [
                    self.spacer(4),
                    self.p("Root Source Details", "h2"),
                    self.table(
                        detail_rows,
                        widths=(20, 50, 26, 26, 13, 24, 17),
                        compact=True,
                        right_columns={3, 5},
                    ),
                ]
            )
        return story

    def data_sources(self) -> list[Any]:
        story: list[Any] = [self.p("Data Sources", "h1")]
        segments = self._data_source_segments()
        if segments:
            total_tx = int(sum(segment["value"] for segment in segments))
            story.append(
                build_ring_drawing(
                    self.rl,
                    self.fonts,
                    segments,
                    center_value=str(total_tx),
                    center_label="transactions",
                    width=self._content_width(),
                )
            )
        rows = [["Name", "Kind", "Provenance", "Transactions", "Sources", "Assets", "Period"]]
        for item in self.report.get("data_sources") or []:
            period = ""
            if item.get("first_seen") or item.get("last_seen"):
                period = f"{_datetime(item.get('first_seen'))} - {_datetime(item.get('last_seen'))}"
            provenance = str(item.get("provenance") or "")
            if provenance == "attested_source":
                provenance_label = "attested"
            else:
                provenance_label = _PROVENANCE_SHORT.get(provenance, _label(provenance))
            rows.append(
                [
                    item.get("label", ""),
                    _label(item.get("kind")),
                    provenance_label,
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
                self.table(
                    rows,
                    widths=(38, 24, 20, 17, 14, 18, 45),
                    compact=True,
                    right_columns={3, 4},
                )
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
                amount = self.amt(
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
            self.table(rows, widths=scale_widths((12, 20, 50, 28, 31, 28)), compact=True, right_columns={0, 5})
        )
        return story

    def transaction_details(self) -> list[Any]:
        story: list[Any] = [self.p("Transaction Details by Level", "h1")]
        levels = list(self.report.get("flow_levels") or [])
        if not levels:
            story.append(self.p("No transaction-level rows in this case snapshot."))
            return story
        story.append(
            self.p(
                "Level 1 is the report target; each further level moves one reviewed hop "
                "backwards towards the root sources. In/Out follow the row's direction; "
                "the data-source column states how the row entered Kassiber.",
                "small",
            )
        )
        # Case snapshots saved before the level nodes carried
        # direction/fee/provenance still have those fields on the full graph
        # nodes; fall back there so legacy exports keep correct In/Out sides.
        graph_nodes_by_id = {
            str(graph_node.get("id")): graph_node
            for graph_node in (self.report.get("graph") or {}).get("nodes") or []
            if graph_node.get("id")
        }
        rendered_any = False
        for level in levels:
            nodes = list(level.get("nodes") or [])
            if not nodes:
                continue
            rendered_any = True
            tx_count = int(level.get("transaction_count") or 0)
            source_count = int(level.get("source_count") or 0)
            counts = []
            if tx_count:
                counts.append(f"{tx_count} transaction{'' if tx_count == 1 else 's'}")
            if source_count:
                counts.append(f"{source_count} source{'' if source_count == 1 else 's'}")
            heading = f"Level {level.get('level', '')} ({', '.join(counts) or 'no rows'})"
            fiat_total = level.get("fiat_value_total")
            if fiat_total is not None:
                heading += f" — {_fiat(fiat_total, level.get('fiat_currency'))}"
            story.append(self.p(heading, "h2"))
            rows: list[list[Any]] = [
                ["Date", "Source", "Type", "In", "Out", "Fee", "Fiat", "ID / hash", "Data src"]
            ]
            for node in nodes:
                graph_node = graph_nodes_by_id.get(str(node.get("id") or ""), {})
                is_source = node.get("node_type") == "source"
                amount = self.amt(
                    node.get("required_amount")
                    if node.get("required_amount") is not None
                    else node.get("amount"),
                    node.get("asset"),
                )
                direction = str(node.get("direction") or graph_node.get("direction") or "")
                inbound = is_source or direction == "inbound"
                fee = node.get("fee")
                if fee is None:
                    fee = graph_node.get("fee")
                fee_cell = self.amt(fee) if fee and float(fee) > 0 else ""
                if is_source:
                    provenance = "attested"
                else:
                    provenance = _PROVENANCE_SHORT.get(
                        str(node.get("data_provenance") or graph_node.get("data_provenance") or ""),
                        "",
                    )
                fiat_value = (
                    node.get("fiat_value_allocated")
                    if "fiat_value_allocated" in node
                    else node.get("fiat_value")
                )
                rows.append(
                    [
                        _node_time(node),
                        node.get("label", "") if is_source else node.get("wallet", ""),
                        _label(node.get("source_type")) if is_source else _label(direction),
                        amount if inbound else "",
                        "" if inbound else amount,
                        fee_cell,
                        _fiat(fiat_value, node.get("fiat_currency")),
                        node.get("external_id") or ("" if is_source else node.get("label", "")),
                        provenance,
                    ]
                )
            story.append(
                self.table(
                    rows,
                    widths=scale_widths((20, 23, 15, 20, 20, 17, 19, 29, 13)),
                    compact=True,
                    right_columns={3, 4, 5, 6},
                )
            )
        if not rendered_any:
            story.append(self.p("No transaction-level rows in this case snapshot."))
        return story

    def flow_links(self) -> list[Any]:
        story: list[Any] = [self.p("Reviewed Flow Links", "h1")]
        edges = list((self.report.get("graph") or {}).get("edges") or [])
        if not edges:
            story.append(self.p("No reviewed links yet."))
            return story
        nodes_by_id = {
            str(node.get("id")): node
            for node in (self.report.get("graph") or {}).get("nodes") or []
            if node.get("id")
        }

        def endpoint(node_id: Any) -> str:
            node = nodes_by_id.get(str(node_id or ""))
            if not node:
                return ""
            if node.get("node_type") == "source":
                return str(node.get("label") or _label(node.get("source_type")))
            return str(node.get("wallet") or node.get("label") or "")

        rows = [["Type", "From", "To", "Amount", "Method", "Explanation"]]
        for edge in edges:
            explanation = edge.get("explanation") or _LINK_TYPE_EXPLANATIONS.get(
                str(edge.get("link_type") or ""), ""
            )
            rows.append(
                [
                    _label(edge.get("link_type")),
                    endpoint(edge.get("from")),
                    endpoint(edge.get("to")),
                    self.amt(edge.get("allocation_amount"), edge.get("asset", "")),
                    _label(edge.get("method")),
                    explanation,
                ]
            )
        story.append(
            self.p(
                "All links shown are reviewed; suggested links never reach an exported report.",
                "small",
            )
        )
        story.append(
            self.table(rows, widths=scale_widths((20, 30, 30, 26, 24, 46)), compact=True, right_columns={3})
        )
        return story

    def missing_history(self) -> list[Any]:
        gaps = list(self.report.get("gaps") or [])
        if not gaps:
            return []
        story: list[Any] = [self.p("Missing History and Gaps", "h1")]
        story.append(
            self.p(
                "These reviewed paths stop before a documented root source, or pass a "
                "boundary Kassiber will not traverse. Attested gaps are disclosed as "
                "gaps, never papered over.",
                "small",
            )
        )
        rows: list[list[Any]] = [["Severity", "Gap", "Amount", "Reference"]]
        for gap in gaps:
            amount = ""
            if gap.get("amount") is not None:
                amount = self.amt(gap.get("amount"), gap.get("asset", ""))
            rows.append(
                [
                    gap.get("severity", ""),
                    gap.get("message", ""),
                    amount,
                    gap.get("ref", ""),
                ]
            )
        story.append(
            self.table(rows, widths=(18, 90, 28, 40), compact=True, right_columns={2})
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
                        self.amt(node.get("required_amount")),
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
                        self.amt(node.get("required_amount")),
                        " | ".join(details),
                    ]
                )
        story.append(
            self.table(rows, widths=scale_widths((28, 36, 25, 17, 25, 41)), compact=True, right_columns={4})
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
        footprint_rows: list[tuple[str, Any]] = [
            ("Txids disclosed", len(txids)),
            ("Evidence files", len(attachments)),
        ]
        wallets_named = list(preview.get("wallets_named") or [])
        if wallets_named:
            footprint_rows.append(("Wallets named", ", ".join(wallets_named)))
        story.extend(
            [
                self.spacer(4),
                self.p("What sharing this report reveals", "h2"),
                self.kv_table(footprint_rows),
            ]
        )
        if preview.get("ownership_note"):
            story.append(self.p(preview["ownership_note"], "small"))
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

    def _section_plan(self) -> list[tuple[str | None, str, Any]]:
        # (section_key, contents title, builder) — keyed sections can be
        # omitted via the advanced `omit_sections` option; None keys are
        # always included. A builder returning an empty story (e.g. no
        # missing-history gaps) drops out of both the report and the
        # contents list.
        return [
            (None, "Source of Funds Overview", self.overview),
            (None, "Origin and Transaction Flow", self.narrative),
            (None, "Simplified Flow Path", self.simplified_flow),
            (None, "Data Sources", self.data_sources),
            (None, "Evidence Checklist", self.evidence_checklist),
            (None, "Review Gates", self.review_gates),
            (None, "Missing History and Gaps", self.missing_history),
            (None, "Source Mix", self.source_mix),
            ("flow_levels", "Flow Diagram Data", self.flow_levels),
            ("transaction_details", "Transaction Details by Level", self.transaction_details),
            ("flow_links", "Reviewed Flow Links", self.flow_links),
            ("graph_nodes", "Disclosure Graph Nodes", self.graph_nodes),
            (None, "Disclosure Preview", self.disclosure_preview),
            (None, "Limitations", self.limitations),
        ]

    def _included_section_titles(self) -> list[str]:
        titles = []
        for key, title, _builder in self._section_plan():
            if key and key in self.omitted_sections:
                continue
            if title == "Missing History and Gaps" and not self.report.get("gaps"):
                continue
            titles.append(title)
        return titles

    def build(self) -> list[Any]:
        story = self.cover()
        story.append(self.rl["PageBreak"]())
        # Flip the contiguous wide-detail block to landscape and restore
        # portrait afterwards. Driven by actually-emitted content (not fixed
        # plan indices) so omit_sections and empty builders keep working: an
        # entirely omitted/empty block emits no flip.
        in_landscape = False
        for key, _title, builder in self._section_plan():
            if key and key in self.omitted_sections:
                continue
            content = builder()
            if not content:
                continue
            wants_landscape = key in _LANDSCAPE_SECTION_KEYS
            if wants_landscape and not in_landscape:
                story.append(self.rl["NextPageTemplate"]("landscape"))
                story.append(self.rl["PageBreak"]())
                in_landscape = True
            elif not wants_landscape and in_landscape:
                story.append(self.rl["NextPageTemplate"]("portrait"))
                story.append(self.rl["PageBreak"]())
                in_landscape = False
            story.extend(content)
            story.append(self.spacer(7))
        if in_landscape:
            story.append(self.rl["NextPageTemplate"]("portrait"))
            story.append(self.rl["PageBreak"]())
        return story


def _on_page(canvas: Any, doc: Any, *, title: str, fonts: dict[str, str], rl: dict[str, Any]) -> None:
    draw_page_header(
        canvas,
        doc,
        title=title,
        fonts=fonts,
        rl=rl,
        brand_label="Kassiber",
        footer_left="Local-first evidence disclosure. Not legal or AML advice.",
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
    landscape = rl["landscape"]
    mm = rl["mm"]

    title = f"Kassiber {_report_title(report)}"
    # The page masthead already prints "Kassiber"; keep the running header
    # title brand-free so it is not duplicated on every page.
    running_title = _report_title(report)
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
    landscape_size = landscape(A4)
    landscape_frame = Frame(
        12 * mm, 13 * mm, landscape_size[0] - 24 * mm, landscape_size[1] - 30 * mm, id="landscape"
    )
    doc.addPageTemplates(
        [
            PageTemplate(
                id="portrait",
                frames=[frame],
                pagesize=A4,
                onPage=lambda canvas, document: _on_page(canvas, document, title=running_title, fonts=fonts, rl=rl),
            ),
            PageTemplate(
                id="landscape",
                frames=[landscape_frame],
                pagesize=landscape_size,
                onPage=lambda canvas, document: _on_page(canvas, document, title=running_title, fonts=fonts, rl=rl),
            ),
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
