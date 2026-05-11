from __future__ import annotations

from collections import defaultdict
from decimal import Decimal, InvalidOperation
from importlib import resources
from pathlib import Path
from typing import Any, Iterable, Sequence

from .errors import AppError


BRAND_INK = "#222222"
BRAND_MUTED = "#666666"
BRAND_LINE = "#d9d9d9"
BRAND_SOFT = "#f7f7f7"
BRAND_ACCENT = "#e3000f"
BRAND_ACCENT_SOFT = "#fff1f2"


def _require_reportlab():
    try:
        from reportlab import rl_config
        from reportlab.lib import colors
        from reportlab.lib.pagesizes import A4, landscape
        from reportlab.lib.styles import ParagraphStyle
        from reportlab.lib.units import mm
        from reportlab.pdfbase import pdfmetrics
        from reportlab.pdfbase.ttfonts import TTFont
        from reportlab.platypus import (
            BaseDocTemplate,
            Frame,
            NextPageTemplate,
            PageBreak,
            PageTemplate,
            Paragraph,
            Spacer,
            Table,
            TableStyle,
        )
    except ImportError as exc:
        raise AppError(
            "Austrian PDF export requires the ReportLab PDF renderer",
            code="dependency_missing",
            hint="Install Kassiber project dependencies again so reportlab is available.",
        ) from exc

    return {
        "rl_config": rl_config,
        "colors": colors,
        "A4": A4,
        "landscape": landscape,
        "ParagraphStyle": ParagraphStyle,
        "mm": mm,
        "pdfmetrics": pdfmetrics,
        "TTFont": TTFont,
        "BaseDocTemplate": BaseDocTemplate,
        "Frame": Frame,
        "NextPageTemplate": NextPageTemplate,
        "PageBreak": PageBreak,
        "PageTemplate": PageTemplate,
        "Paragraph": Paragraph,
        "Spacer": Spacer,
        "Table": Table,
        "TableStyle": TableStyle,
    }


def _register_fonts(rl: dict[str, Any]) -> dict[str, str]:
    pdfmetrics = rl["pdfmetrics"]
    TTFont = rl["TTFont"]
    try:
        font_dir = resources.files("reportlab").joinpath("fonts")
        regular = font_dir.joinpath("Vera.ttf")
        bold = font_dir.joinpath("VeraBd.ttf")
        mono = font_dir.joinpath("VeraMono.ttf")
        if regular.is_file() and bold.is_file() and mono.is_file():
            for name, path in (
                ("KassiberSans", regular),
                ("KassiberSans-Bold", bold),
                ("KassiberMono", mono),
            ):
                if name not in pdfmetrics.getRegisteredFontNames():
                    pdfmetrics.registerFont(TTFont(name, str(path)))
            return {
                "regular": "KassiberSans",
                "bold": "KassiberSans-Bold",
                "mono": "KassiberMono",
            }
    except (FileNotFoundError, ModuleNotFoundError, OSError):
        pass
    return {"regular": "Helvetica", "bold": "Helvetica-Bold", "mono": "Courier"}


def _format_date(value: str | None) -> str:
    if not value:
        return ""
    text = str(value)
    if len(text) >= 10 and text[4] == "-" and text[7] == "-":
        return f"{text[8:10]}.{text[5:7]}.{text[0:4]}"
    return text


def _format_datetime(value: str | None) -> str:
    if not value:
        return ""
    text = str(value).replace("T", " ")
    if text.endswith("Z"):
        text = text[:-1]
    if len(text) >= 19 and text[4] == "-" and text[7] == "-":
        return f"{text[8:10]}.{text[5:7]}.{text[0:4]} {text[11:16]}"
    return text


def _decimal(value: Any) -> Decimal:
    if value is None or value == "":
        return Decimal("0")
    try:
        return Decimal(str(value))
    except (InvalidOperation, ValueError):
        return Decimal("0")


def _money_from_cents(value: Any, *, signed: bool = False) -> str:
    amount = Decimal(int(value or 0)) / Decimal("100")
    return _money(amount, signed=signed)


def _money(value: Any, *, signed: bool = False) -> str:
    amount = _decimal(value).quantize(Decimal("0.01"))
    sign = ""
    if signed and amount > 0:
        sign = "+"
    text = f"{abs(amount):,.2f}".replace(",", "X").replace(".", ",").replace("X", ".")
    if amount < 0:
        sign = "-"
    return f"{sign}{text}"


def _quantity(value: Any) -> str:
    amount = _decimal(value).quantize(Decimal("0.00000001"))
    text = f"{amount:,.8f}".replace(",", "X").replace(".", ",").replace("X", ".")
    return text.rstrip("0").rstrip(",") if "," in text else text


def _escape(value: Any) -> str:
    text = "" if value is None else str(value)
    return (
        text.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace("\n", "<br/>")
    )


class _AustrianReportBuilder:
    def __init__(
        self,
        *,
        report: dict[str, Any],
        profile: dict[str, Any],
        portfolio_rows: Sequence[dict[str, Any]],
        transaction_rows: Sequence[dict[str, Any]],
        section_specs: Sequence[dict[str, Any]],
        generated_at: str,
        rl: dict[str, Any],
        fonts: dict[str, str],
    ) -> None:
        self.report = report
        self.profile = profile
        self.portfolio_rows = list(portfolio_rows)
        self.transaction_rows = list(transaction_rows)
        self.section_specs = list(section_specs)
        self.generated_at = generated_at
        self.rl = rl
        self.fonts = fonts
        self.styles = self._styles()

    def _styles(self) -> dict[str, Any]:
        ParagraphStyle = self.rl["ParagraphStyle"]
        return {
            "cover_title": ParagraphStyle(
                "KassiberCoverTitle",
                fontName=self.fonts["bold"],
                fontSize=28,
                leading=32,
                textColor=BRAND_INK,
                spaceAfter=10,
            ),
            "cover_subtitle": ParagraphStyle(
                "KassiberCoverSubtitle",
                fontName=self.fonts["regular"],
                fontSize=15,
                leading=19,
                textColor=BRAND_MUTED,
                spaceAfter=18,
            ),
            "h1": ParagraphStyle(
                "KassiberH1",
                fontName=self.fonts["bold"],
                fontSize=17,
                leading=21,
                textColor=BRAND_INK,
                spaceBefore=4,
                spaceAfter=8,
            ),
            "h2": ParagraphStyle(
                "KassiberH2",
                fontName=self.fonts["bold"],
                fontSize=12.5,
                leading=15,
                textColor=BRAND_INK,
                spaceBefore=8,
                spaceAfter=6,
            ),
            "h3": ParagraphStyle(
                "KassiberH3",
                fontName=self.fonts["bold"],
                fontSize=10.5,
                leading=13,
                textColor=BRAND_INK,
                spaceBefore=6,
                spaceAfter=4,
            ),
            "body": ParagraphStyle(
                "KassiberBody",
                fontName=self.fonts["regular"],
                fontSize=8.8,
                leading=11.5,
                textColor=BRAND_INK,
                spaceAfter=5,
            ),
            "small": ParagraphStyle(
                "KassiberSmall",
                fontName=self.fonts["regular"],
                fontSize=7.4,
                leading=9.4,
                textColor=BRAND_MUTED,
            ),
            "mono": ParagraphStyle(
                "KassiberMono",
                fontName=self.fonts["mono"],
                fontSize=7.3,
                leading=9,
                textColor=BRAND_INK,
            ),
        }

    def p(self, text: Any, style: str = "body") -> Any:
        Paragraph = self.rl["Paragraph"]
        return Paragraph(_escape(text), self.styles[style])

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
        data = [
            [
                cell if hasattr(cell, "wrap") else self.p(cell, "small" if compact else style)
                for cell in row
            ]
            for row in rows
        ]
        col_widths = [width * mm for width in widths] if widths else None
        table = Table(
            data,
            colWidths=col_widths,
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
                    ("TEXTCOLOR", (0, 0), (-1, 0), colors.HexColor(BRAND_INK)),
                    ("FONTNAME", (0, 0), (-1, 0), self.fonts["bold"]),
                    ("LINEBELOW", (0, 0), (-1, 0), 0.7, colors.HexColor(BRAND_INK)),
                ]
            )
        for row_index in range(1 if header else 0, len(rows)):
            if row_index % 2 == 0:
                commands.append(
                    (
                        "BACKGROUND",
                        (0, row_index),
                        (-1, row_index),
                        colors.HexColor("#fbfbfb"),
                    )
                )
        for column in right_columns:
            commands.append(("ALIGN", (column, 0), (column, -1), "RIGHT"))
        table.setStyle(TableStyle(commands))
        return table

    def kv_table(self, rows: Sequence[tuple[str, Any]], *, widths=(42, 58)) -> Any:
        return self.table(
            [[label, value] for label, value in rows],
            widths=widths,
            header=False,
            repeat=False,
            compact=True,
        )

    def _money_total(self, kennzahl: int) -> str:
        row = self.report.get("kennzahl_totals", {}).get(str(kennzahl), {})
        return _money_from_cents(row.get("amount_eur_cents"))

    def _overview_amount(self, section_id: str) -> int:
        return int(
            self.report.get("sections", {})
            .get(section_id, {})
            .get("totals", {})
            .get("amount_eur_cents", 0)
            or 0
        )

    def _period_metrics(self) -> dict[str, Any]:
        dated_rows = [row for row in self.transaction_rows if row.get("occurred_at")]
        dates = sorted(str(row["occurred_at"]) for row in dated_rows)
        return {
            "transactions": len(self.transaction_rows),
            "wallets": len({row.get("wallet") for row in self.transaction_rows if row.get("wallet")}),
            "first": dates[0] if dates else None,
            "last": dates[-1] if dates else None,
        }

    def build(self) -> list[Any]:
        story: list[Any] = []
        story.extend(self.cover())
        story.extend(self.taxable_summary())
        story.extend(self.taxable_details())
        story.extend(self.tax_free_summary())
        story.extend(self.holdings())
        story.extend(self.special_cases())
        story.extend(self.explanations())
        story.extend(self.transaction_appendix())
        story.extend(self.finanzonline_summary())
        story.extend(self.faq())
        return story

    def cover(self) -> list[Any]:
        tax_year = self.report["tax_year"]
        period = f"01.01.{tax_year} - 31.12.{tax_year}"
        tabs = self.table(
            [["Steuerbericht", "Transaktionsdetails", "Steuerformulare"]],
            widths=(45, 58, 50),
            compact=True,
            right_columns=(),
        )
        metrics = self._period_metrics()
        rows = [
            ("Zeitraum", period),
            ("Workspace", self.report.get("workspace", "")),
            ("Profil", self.report.get("profile", "")),
            ("Währung", "€ (EUR)"),
            ("Steuerjahr", tax_year),
            ("Berechnungsart", self.profile.get("gains_algorithm", "")),
            ("Report erstellt am", _format_datetime(self.generated_at)),
            ("Transaktionen", metrics["transactions"]),
            ("Genutzte Wallets", metrics["wallets"]),
            ("Erste Transaktion", _format_date(metrics["first"])),
            ("Letzte Transaktion", _format_date(metrics["last"])),
        ]
        return [
            self.p("Kassiber Steuerbericht", "cover_title"),
            self.p(f"{tax_year} Österreich", "cover_subtitle"),
            tabs,
            self.spacer(9),
            self.p("Berichtsumfang", "h1"),
            self.p(
                "Diese PDF fasst die in Kassiber importierten und verarbeiteten Transaktionen "
                "für den angegebenen Zeitraum zusammen. Sie verwendet die lokale Kassiber-Datenbank "
                "und keine Beispiel- oder Mockdaten.",
            ),
            self.kv_table(rows, widths=(45, 75)),
            self.spacer(6),
            self.p("Inhaltsverzeichnis", "h2"),
            self.table(
                [
                    ["1", "Steuerpflichtige Gesamtübersicht"],
                    ["2", "Steuerpflichtige Detailübersicht"],
                    ["3", "Steuerfreie / nicht steuerbare Übersicht"],
                    ["4", "Bestandsübersicht"],
                    ["5", "Besonderheiten"],
                    ["6", "Erläuterungen"],
                    ["7", "Transaktionsdetails"],
                    ["8", "Steuerformulare"],
                    ["9", "FAQ"],
                ],
                widths=(12, 105),
                header=False,
                compact=True,
            ),
            self.spacer(8),
            self.notice_box(
                "Prüfung erforderlich",
                self.report.get("review_gate", ""),
                accent=True,
            ),
            self.rl["PageBreak"](),
        ]

    def notice_box(self, title: str, text: str, *, accent: bool = False) -> Any:
        colors = self.rl["colors"]
        bg = BRAND_ACCENT_SOFT if accent else BRAND_SOFT
        line = BRAND_ACCENT if accent else BRAND_LINE
        box = self.table(
            [[self.p(title, "h3")], [self.p(text)]],
            widths=(170,),
            header=False,
            repeat=False,
        )
        box.setStyle(
            self.rl["TableStyle"](
                [
                    ("BACKGROUND", (0, 0), (-1, -1), colors.HexColor(bg)),
                    ("BOX", (0, 0), (-1, -1), 0.8, colors.HexColor(line)),
                ]
            )
        )
        return box

    def taxable_summary(self) -> list[Any]:
        rows = [
            ["1.1.", "Einkünfte aus der Überlassung von Kapital", "E 1kv KZ 862", self._money_total(862)],
            ["1.2.", "Einkünfte aus nicht verbrieften Derivaten", "E 1kv KZ 857", self._money_total(857)],
            ["1.3.", "Einkünfte aus verbrieften Derivaten", "E 1kv KZ 995", self._money_total(995)],
            ["1.4.", "Verluste aus verbrieften Derivaten", "E 1kv KZ 896", self._money_total(896)],
            [
                "1.5.",
                "Laufende Einkünfte - § 27b Abs. 2 EStG",
                "E 1kv KZ 172",
                self._money_total(172),
            ],
            [
                "1.6.",
                "Überschüsse aus realisierten Wertsteigerungen - § 27b Abs. 3 EStG",
                "E 1kv KZ 174",
                self._money_total(174),
            ],
            [
                "1.7.",
                "Verluste aus realisierten Wertsteigerungen - § 27b Abs. 3 EStG",
                "E 1kv KZ 176",
                self._money_total(176),
            ],
            ["2.1.", "Inländische realisierte Wertsteigerungen mit Steuerabzug", "E 1kv KZ 173", "*"],
            ["2.2.", "Inländische realisierte Wertverluste mit Steuerabzug", "E 1kv KZ 175", "*"],
            ["3.", "Einkünfte aus Spekulationsgeschäften - § 31 EStG", "E 1 KZ 801", self._money_total(801)],
            ["4.", "Einkünfte aus Leistungen - § 29 Z. 3 EStG", "E 1 KZ 803", self._money_total(803)],
        ]
        return [
            self.p("Gesamtübersicht", "h1"),
            self.p("Steuerpflichtig", "cover_subtitle"),
            self.table(
                [["Abschnitt", "Beschreibung", "Formular", "Betrag EUR"], *rows],
                widths=(17, 92, 33, 28),
                right_columns={3},
            ),
            self.spacer(5),
            self.p(
                "* Kassiber speichert derzeit keine strukturierte Information zu inländischen "
                "Kryptodienstleistern mit einbehaltener KESt. Diese Felder bleiben daher als "
                "prüfpflichtige Platzhalter sichtbar.",
                "small",
            ),
            self.rl["PageBreak"](),
        ]

    def taxable_details(self) -> list[Any]:
        story = [self.p("Detailübersicht", "h1"), self.p("Steuerpflichtig", "cover_subtitle")]
        for spec in self.section_specs[:5]:
            story.extend(self.section_spec(spec))
        story.append(self.rl["PageBreak"]())
        return story

    def tax_free_summary(self) -> list[Any]:
        rows = [
            ["3.1.", "Nicht steuerbare Einkünfte aus Spekulationsgeschäften", _money_from_cents(self._overview_amount("3.1"))],
            ["3.2.", "Nicht steuerbare Einkünfte gem. § 27b Abs. 2 Z 2 Satz 2 EStG", _money_from_cents(self._overview_amount("3.2"))],
            ["3.3.", "Nicht steuerbare Steuergebühren und Rückerstattungen", _money_from_cents(self._overview_amount("3.3"))],
            ["4.1.", "Eingegangene Spenden/Trinkgeld", _money_from_cents(self._overview_amount("4.1"))],
            ["4.2.", "Ausgegangene Spenden/Schenkungen", _money_from_cents(self._overview_amount("4.2"))],
            ["4.3.", "Gestohlene, gehackte und verlorene Coins", _money_from_cents(self._overview_amount("4.3"))],
            ["4.4.", "Mining (kommerziell)", _money_from_cents(self._overview_amount("4.4"))],
            ["4.5.", "Minting", _money_from_cents(self._overview_amount("4.5"))],
        ]
        story = [
            self.p("Gesamtübersicht", "h1"),
            self.p("Steuerfrei / nicht steuerbar", "cover_subtitle"),
            self.table(
                [["Abschnitt", "Beschreibung", "Betrag EUR"], *rows],
                widths=(18, 118, 30),
                right_columns={2},
            ),
            self.spacer(8),
            self.p("Detailübersicht", "h1"),
        ]
        for spec in self.section_specs[5:]:
            story.extend(self.section_spec(spec))
        story.append(self.rl["PageBreak"]())
        return story

    def section_spec(self, spec: dict[str, Any]) -> list[Any]:
        story = [self.p(spec["title"], "h2")]
        headers = list(spec.get("headers", []))
        rows = list(spec.get("rows", []))
        formats = list(spec.get("row_format_names", []))
        table_rows = [headers]
        for row in rows:
            table_rows.append(
                [
                    self.render_cell(value, formats[index] if index < len(formats) else "text")
                    for index, value in enumerate(row)
                ]
            )
        if len(table_rows) == 1:
            table_rows.append(["Keine Zeilen im Berichtszeitraum.", *[""] * (len(headers) - 1)])
        total_rows = spec.get("total_rows", [])
        for label, value in total_rows:
            table_rows.append([label, *[""] * max(0, len(headers) - 2), _money(value)])
        column_count = max(1, len(headers))
        usable_width = 166
        width = max(16, usable_width / column_count)
        right_columns = {index for index, name in enumerate(headers) if "EUR" in name or "Anzahl" in name or "Gesamt" in name}
        story.append(
            self.table(
                table_rows,
                widths=[width] * column_count,
                compact=True,
                right_columns=right_columns,
            )
        )
        story.append(self.spacer(4))
        return story

    def render_cell(self, value: Any, format_name: str) -> str:
        if value is None or value == "":
            return ""
        if format_name == "money":
            return _money(value)
        if format_name == "quantity":
            return _quantity(value)
        if format_name == "int":
            return str(int(value))
        return str(value)

    def holdings(self) -> list[Any]:
        rows = [["Wallet", "Bucket", "Asset", "Bestand", "Avg Cost", "Cost Basis", "Market", "Unreal."]]
        for row in self.portfolio_rows:
            rows.append(
                [
                    row.get("wallet", ""),
                    row.get("account", "") or "",
                    row.get("asset", ""),
                    _quantity(row.get("quantity")),
                    _money(row.get("avg_cost")),
                    _money(row.get("cost_basis")),
                    _money(row.get("market_value")),
                    _money(row.get("unrealized_pnl"), signed=True),
                ]
            )
        if len(rows) == 1:
            rows.append(["Keine aktuellen Bestände.", "", "", "", "", "", "", ""])

        flow_buckets: dict[tuple[str, str], dict[str, Any]] = defaultdict(
            lambda: {
                "tx_count": 0,
                "inbound_count": 0,
                "outbound_count": 0,
                "inbound_amount": Decimal("0"),
                "outbound_amount": Decimal("0"),
                "fee_amount": Decimal("0"),
            }
        )
        for transaction in self.transaction_rows:
            bucket = flow_buckets[(transaction.get("wallet", ""), transaction.get("asset", ""))]
            bucket["tx_count"] += 1
            if transaction.get("direction") == "inbound":
                bucket["inbound_count"] += 1
                bucket["inbound_amount"] += _decimal(transaction.get("amount"))
            elif transaction.get("direction") == "outbound":
                bucket["outbound_count"] += 1
                bucket["outbound_amount"] += _decimal(transaction.get("amount"))
            bucket["fee_amount"] += _decimal(transaction.get("fee"))

        flow_rows = [["Wallet", "Asset", "Tx", "In", "Out", "Zufluss", "Abfluss", "Gebühren"]]
        for (wallet, asset), row in sorted(flow_buckets.items()):
            flow_rows.append(
                [
                    wallet,
                    asset,
                    row.get("tx_count", 0),
                    row.get("inbound_count", 0),
                    row.get("outbound_count", 0),
                    _quantity(row.get("inbound_amount")),
                    _quantity(row.get("outbound_amount")),
                    _quantity(row.get("fee_amount")),
                ]
            )
        if len(flow_rows) == 1:
            flow_rows.append(["Keine Bewegungen im Scope.", "", "", "", "", "", "", ""])

        return [
            self.p("Aufstellung der Bestände und Bestandsveränderungen", "h1"),
            self.p(
                "Die Bestände stammen aus dem aktuellen verarbeiteten Journalzustand. Zufluss, Abfluss "
                "und Gebühren werden für das ausgewählte Steuerjahr aus den importierten Transaktionen "
                "pro Wallet und Asset verdichtet.",
            ),
            self.table(rows, widths=(27, 22, 14, 25, 22, 24, 24, 24), compact=True, right_columns={3, 4, 5, 6, 7}),
            self.spacer(7),
            self.p("Bestandsveränderungen", "h2"),
            self.table(flow_rows, widths=(30, 14, 13, 13, 13, 27, 27, 27), compact=True, right_columns={2, 3, 4, 5, 6, 7}),
            self.rl["PageBreak"](),
        ]

    def special_cases(self) -> list[Any]:
        quarantines = self.report.get("data_quality", {}).get("quarantines", [])
        mismatches = self.report.get("data_quality", {}).get("kennzahl_mismatches", [])
        rows = [["Kategorie", "Auswirkung", "Prüfhinweis"]]
        if quarantines:
            for row in quarantines:
                rows.append(
                    [
                        row.get("reason", ""),
                        f"{int(row.get('count', 0))} Transaktionen nicht exportiert",
                        "Journal erneut prüfen und Quarantäne vor der Einreichung auflösen.",
                    ]
                )
        else:
            rows.append(["Quarantäne", "Keine quarantinierten Transaktionen im Steuerjahr.", ""])
        if mismatches:
            for row in mismatches:
                rows.append(
                    [
                        "Kennzahl-Abweichung",
                        f"{row.get('tx_id', '')}: {row.get('stored_kennzahl', '')} -> {row.get('export_kennzahl', '')}",
                        "journals process nach Upgrades erneut ausführen.",
                    ]
                )
        else:
            rows.append(["Kennzahl-Abweichungen", "Keine gespeicherten Abweichungen sichtbar.", ""])
        rows.append(
            [
                "Nicht modellierte Abschnitte",
                "Margin/Derivate, NFTs, Spenden, verlorene Coins, kommerzielles Mining und Minting bleiben Null-Platzhalter.",
                "Nur mit strukturierter Klassifikation befüllen; nicht aus Freitext erraten.",
            ]
        )
        return [
            self.p("Besonderheiten", "h1"),
            self.p(
                "Dieser Abschnitt sammelt prüfpflichtige Sachverhalte, die nicht als normale "
                "Steuerbeträge verschwinden sollen.",
            ),
            self.table(rows, widths=(46, 63, 61), compact=True),
            self.rl["PageBreak"](),
        ]

    def explanations(self) -> list[Any]:
        assumption_rows = [["Code", "Hinweis"]]
        for item in self.report.get("assumptions", []):
            assumption_rows.append([item.get("code", ""), item.get("message", "")])
        quality_rows = [["Prüfung", "Status"]]
        quarantines = self.report.get("data_quality", {}).get("quarantines", [])
        mismatches = self.report.get("data_quality", {}).get("kennzahl_mismatches", [])
        quality_rows.append(["Quarantäne", f"{sum(int(row.get('count', 0)) for row in quarantines)} Transaktionen"])
        quality_rows.append(["Kennzahl-Abweichungen", f"{len(mismatches)} Zeilen"])
        return [
            self.p("Erläuterungen", "h1"),
            self.p(
                "Kassiber ist ein lokales Bitcoin-Buchhaltungswerkzeug. Diese Auswertung bleibt "
                "eine prüfpflichtige Übergabe an Steuerpflichtige und Berater; sie ersetzt keine "
                "steuerliche Beratung und keine Vollständigkeitsprüfung der importierten Daten.",
            ),
            self.p(
                "Austrian-spezifische Beträge werden aus den verarbeiteten RP2-Journalen und "
                "Kassibers Kennzahl-Mapping erzeugt. Nicht modellierte Abschnitte werden mit "
                "Nullwerten ausgewiesen, damit offene Themen sichtbar bleiben.",
            ),
            self.table(assumption_rows, widths=(42, 128), compact=True),
            self.spacer(7),
            self.p("Datenqualität", "h2"),
            self.table(quality_rows, widths=(55, 65), compact=True),
            self.spacer(7),
            self.notice_box("Review Gate", self.report.get("review_gate", ""), accent=True),
            self.rl["NextPageTemplate"]("landscape"),
            self.rl["PageBreak"](),
        ]

    def transaction_appendix(self) -> list[Any]:
        rows = [[
            "Datum",
            "Wallet",
            "Referenz",
            "Typ / Notiz",
            "Eingang",
            "Ausgang",
            "Gebühr",
            "Kosten EUR",
            "Erlös EUR",
            "Gewinn EUR",
            "KZ",
        ]]
        tax_by_tx: dict[str, dict[str, Any]] = defaultdict(
            lambda: {
                "cost_basis_eur_cents": 0,
                "proceeds_eur_cents": 0,
                "gain_loss_eur_cents": 0,
                "kennzahlen": set(),
            }
        )
        for row in self.report.get("rows", []):
            bucket = tax_by_tx[str(row.get("transaction_id") or "")]
            bucket["cost_basis_eur_cents"] += int(row.get("cost_basis_eur_cents") or 0)
            bucket["proceeds_eur_cents"] += int(row.get("proceeds_eur_cents") or 0)
            bucket["gain_loss_eur_cents"] += int(row.get("gain_loss_eur_cents") or 0)
            if row.get("kennzahl") is not None:
                bucket["kennzahlen"].add(str(row.get("kennzahl")))

        for row in self.transaction_rows:
            tax = tax_by_tx.get(str(row.get("transaction_id") or ""), {})
            direction = str(row.get("direction") or "")
            amount = f"{_quantity(row.get('amount'))} {row.get('asset', '')}".strip()
            fee = f"{_quantity(row.get('fee'))} {row.get('asset', '')}".strip()
            label_parts = [
                row.get("kind", "") or row.get("description", ""),
                row.get("note", ""),
                f"Tags: {row.get('tags')}" if row.get("tags") else "",
            ]
            rows.append(
                [
                    _format_datetime(row.get("occurred_at")),
                    row.get("wallet", ""),
                    row.get("external_id", "") or f"#{row.get('transaction_id', '')}",
                    " | ".join(str(part) for part in label_parts if part),
                    amount if direction == "inbound" else "",
                    amount if direction == "outbound" else "",
                    fee if row.get("fee") else "",
                    _money_from_cents(tax.get("cost_basis_eur_cents")),
                    _money_from_cents(tax.get("proceeds_eur_cents")),
                    _money_from_cents(tax.get("gain_loss_eur_cents"), signed=True),
                    ", ".join(sorted(tax.get("kennzahlen", set()))),
                ]
            )
        if len(rows) == 1:
            rows.append(["Keine Transaktionen im Berichtszeitraum.", "", "", "", "", "", "", "", "", "", ""])
        return [
            self.p("Transaktionsübersicht", "h1"),
            self.p(
                "Diese Appendix-Tabelle nutzt die echten importierten Kassiber-Transaktionen "
                "und ergänzt, wo vorhanden, die zugeordneten Journalbeträge und Kennzahlen.",
            ),
            self.table(
                rows,
                widths=(20, 23, 28, 35, 25, 25, 21, 21, 21, 21, 10),
                compact=True,
                right_columns={7, 8, 9},
            ),
            self.rl["NextPageTemplate"]("portrait"),
            self.rl["PageBreak"](),
        ]

    def finanzonline_summary(self) -> list[Any]:
        form_rows = [
            ["801", "Einkünfte aus Spekulationsgeschäften (§ 31)", self._money_total(801)],
            ["803", "Einkünfte aus Leistungen im Sinne des § 29 Z. 3 EStG", self._money_total(803)],
            ["857", "Sonstige tarifsteuerpflichtige Einkünfte aus Kapitalvermögen", self._money_total(857)],
            ["862", "Einkünfte aus der Überlassung von Kapital", self._money_total(862)],
            ["995", "Einkünfte aus verbrieften Derivaten", self._money_total(995)],
            ["896", "Verluste aus verbrieften Derivaten", self._money_total(896)],
            ["172", "Ausländische laufende Einkünfte", self._money_total(172)],
            ["174", "Ausländische Überschüsse aus realisierten Wertsteigerungen", self._money_total(174)],
            ["176", "Ausländische Verluste", self._money_total(176)],
            ["171", "Inländische laufende Einkünfte", "*"],
            ["173", "Inländische Überschüsse aus realisierten Wertsteigerungen", "*"],
            ["175", "Inländische Verluste", "*"],
        ]
        return [
            self.p("Steuerformulare", "h1"),
            self.p("Online - FinanzOnline", "cover_subtitle"),
            self.table(
                [["Kennzahl", "Feld", "Betrag EUR"], *form_rows],
                widths=(24, 112, 34),
                right_columns={2},
            ),
            self.spacer(6),
            self.p(
                "* Bitte Werte von inländischen Kryptodienstleistern separat prüfen. "
                "Kassiber befüllt diese Felder erst, wenn Provider- und KESt-Metadaten modelliert sind.",
                "small",
            ),
        ]

    def faq(self) -> list[Any]:
        rows = [
            [
                "Welche Daten verwendet dieser Bericht?",
                "Nur die im lokalen Kassiber-Profil gespeicherten, nicht ausgeschlossenen Transaktionen und verarbeiteten Journalzeilen.",
            ],
            [
                "Warum stehen manche Abschnitte auf 0,00?",
                "Kassiber zeigt nicht modellierte Bereiche bewusst als Platzhalter, damit offene Klassifikationen sichtbar bleiben.",
            ],
            [
                "Kann ich die Kennzahlen direkt einreichen?",
                "Nein. Der Bericht ist eine prüfpflichtige Übergabe und muss vor Einreichung mit einem Steuerberater oder der eigenen Sachverhaltsprüfung abgeglichen werden.",
            ],
            [
                "Warum sind inländische Kennzahlen mit * markiert?",
                "Provider- und KESt-Metadaten sind noch nicht strukturiert modelliert. Diese Werte müssen separat aus dem jeweiligen Anbieterreport geprüft werden.",
            ],
        ]
        story = [self.p("FAQ", "h1")]
        for question, answer in rows:
            story.append(self.p(question, "h2"))
            story.append(self.p(answer))
        return story


def _on_page(canvas: Any, doc: Any, *, title: str, fonts: dict[str, str], rl: dict[str, Any]) -> None:
    colors = rl["colors"]
    width, height = doc.pagesize
    canvas.saveState()
    canvas.setFillColor(colors.HexColor(BRAND_INK))
    canvas.setFont(fonts["bold"], 8)
    canvas.drawString(doc.leftMargin, height - 9 * rl["mm"], "Kassiber")
    canvas.setFont(fonts["regular"], 7)
    canvas.setFillColor(colors.HexColor(BRAND_MUTED))
    canvas.drawRightString(width - doc.rightMargin, height - 9 * rl["mm"], title)
    canvas.setStrokeColor(colors.HexColor(BRAND_LINE))
    canvas.setLineWidth(0.4)
    canvas.line(doc.leftMargin, height - 11 * rl["mm"], width - doc.rightMargin, height - 11 * rl["mm"])
    canvas.setFont(fonts["regular"], 7)
    canvas.drawString(doc.leftMargin, 8 * rl["mm"], "Local-first Steuerbericht. Keine Steuerberatung.")
    canvas.drawRightString(width - doc.rightMargin, 8 * rl["mm"], f"Seite {doc.page}")
    canvas.restoreState()


def write_austrian_e1kv_pdf(
    file_path: str | Path,
    *,
    report: dict[str, Any],
    profile: dict[str, Any],
    portfolio_rows: Sequence[dict[str, Any]],
    transaction_rows: Sequence[dict[str, Any]],
    section_specs: Sequence[dict[str, Any]],
    generated_at: str,
) -> dict[str, Any]:
    rl = _require_reportlab()
    rl["rl_config"].warnOnMissingFontGlyphs = 0
    fonts = _register_fonts(rl)
    path = Path(file_path).expanduser()
    path.parent.mkdir(parents=True, exist_ok=True)

    BaseDocTemplate = rl["BaseDocTemplate"]
    Frame = rl["Frame"]
    PageTemplate = rl["PageTemplate"]
    A4 = rl["A4"]
    landscape = rl["landscape"]
    mm = rl["mm"]

    title = f"Kassiber Steuerbericht {report['tax_year']} Österreich"
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
        subject="Austrian cryptocurrency tax report",
    )
    portrait_frame = Frame(doc.leftMargin, doc.bottomMargin, doc.width, doc.height, id="portrait")
    landscape_size = landscape(A4)
    landscape_frame = Frame(12 * mm, 13 * mm, landscape_size[0] - 24 * mm, landscape_size[1] - 30 * mm, id="landscape")
    doc.addPageTemplates(
        [
            PageTemplate(
                id="portrait",
                frames=[portrait_frame],
                pagesize=A4,
                onPage=lambda canvas, document: _on_page(canvas, document, title=title, fonts=fonts, rl=rl),
            ),
            PageTemplate(
                id="landscape",
                frames=[landscape_frame],
                pagesize=landscape_size,
                onPage=lambda canvas, document: _on_page(canvas, document, title=title, fonts=fonts, rl=rl),
            ),
        ]
    )
    builder = _AustrianReportBuilder(
        report=report,
        profile=profile,
        portfolio_rows=portfolio_rows,
        transaction_rows=transaction_rows,
        section_specs=section_specs,
        generated_at=generated_at,
        rl=rl,
        fonts=fonts,
    )
    doc.build(builder.build())
    return {
        "file": str(path.resolve()),
        "pages": max(1, int(getattr(doc, "page", 0) or 0)),
        "bytes": path.stat().st_size,
        "title": title,
        "renderer": "reportlab",
    }
