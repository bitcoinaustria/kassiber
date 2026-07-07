from __future__ import annotations

from decimal import Decimal, InvalidOperation
from importlib import resources
from typing import Any

from .errors import AppError


BRAND_INK = "#222222"
BRAND_MUTED = "#666666"
BRAND_LINE = "#d9d9d9"
BRAND_SOFT = "#f7f7f7"
BRAND_ACCENT = "#e3000f"
BRAND_ACCENT_SOFT = "#fff1f2"
BRAND_LINK = "#0f766e"

# Even-row (zebra) fill shared by every report table.
TABLE_ZEBRA = "#fbfbfb"


def require_reportlab(export_name: str) -> dict[str, Any]:
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
            Flowable,
            NextPageTemplate,
            PageBreak,
            PageTemplate,
            Paragraph,
            Spacer,
            Table,
            TableStyle,
        )
        from reportlab.graphics.shapes import (
            Circle,
            Drawing,
            Line,
            PolyLine,
            Rect,
            String,
            Wedge,
        )
        from reportlab.pdfgen.canvas import Canvas
    except ImportError as exc:
        raise AppError(
            f"{export_name} requires the ReportLab PDF renderer",
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
        "Flowable": Flowable,
        "NextPageTemplate": NextPageTemplate,
        "PageBreak": PageBreak,
        "PageTemplate": PageTemplate,
        "Paragraph": Paragraph,
        "Spacer": Spacer,
        "Table": Table,
        "TableStyle": TableStyle,
        "Circle": Circle,
        "Drawing": Drawing,
        "Line": Line,
        "PolyLine": PolyLine,
        "Rect": Rect,
        "String": String,
        "Wedge": Wedge,
        "Canvas": Canvas,
    }


def register_fonts(rl: dict[str, Any]) -> dict[str, str]:
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


def escape_paragraph_text(value: Any) -> str:
    text = "" if value is None else str(value)
    return (
        text.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace("\n", "<br/>")
    )


def build_report_styles(rl: dict[str, Any], fonts: dict[str, str], *, prefix: str = "") -> dict[str, Any]:
    """Shared paragraph-style scale for the ReportLab document reports.

    Kept in one place so the tax and source-of-funds exports stay
    typographically identical. Each numeric-friendly text style also gets a
    ``*_right`` variant, because a table ``ALIGN`` command never shifts a
    ``Paragraph`` (it always fills the cell width) — right alignment has to
    come from the paragraph style itself.
    """
    ParagraphStyle = rl["ParagraphStyle"]

    def style(name: str, **kwargs: Any) -> Any:
        return ParagraphStyle(f"{prefix}{name}", **kwargs)

    styles: dict[str, Any] = {
        "cover_title": style(
            "CoverTitle", fontName=fonts["bold"], fontSize=28, leading=32,
            textColor=BRAND_INK, spaceAfter=9,
        ),
        "cover_subtitle": style(
            "CoverSubtitle", fontName=fonts["regular"], fontSize=15, leading=19,
            textColor=BRAND_MUTED, spaceAfter=17,
        ),
        "h1": style(
            "H1", fontName=fonts["bold"], fontSize=17, leading=21,
            textColor=BRAND_INK, spaceBefore=4, spaceAfter=8,
        ),
        "h2": style(
            "H2", fontName=fonts["bold"], fontSize=12.5, leading=15,
            textColor=BRAND_INK, spaceBefore=8, spaceAfter=6,
        ),
        "h3": style(
            "H3", fontName=fonts["bold"], fontSize=10.5, leading=13,
            textColor=BRAND_INK, spaceBefore=6, spaceAfter=4,
        ),
        "body": style(
            "Body", fontName=fonts["regular"], fontSize=8.8, leading=11.5,
            textColor=BRAND_INK, spaceAfter=5,
        ),
        "small": style(
            "Small", fontName=fonts["regular"], fontSize=7.4, leading=9.4,
            textColor=BRAND_MUTED,
        ),
        "mono": style(
            "Mono", fontName=fonts["mono"], fontSize=7.3, leading=9,
            textColor=BRAND_INK,
        ),
        "table_header": style(
            "TableHeader", fontName=fonts["bold"], fontSize=7.6, leading=9.4,
            textColor=BRAND_INK,
        ),
    }
    for key in ("body", "small", "table_header"):
        styles[f"{key}_right"] = ParagraphStyle(
            f"{prefix}{key.title()}Right", parent=styles[key], alignment=2
        )
    return styles


def scale_widths(widths: Any, target_mm: float = 262.0) -> list[float]:
    """Proportionally rescale a portrait column-width tuple (mm) to fill a
    landscape frame. A landscape A4 body frame is ~273mm wide; the default
    262mm target leaves an ~11mm right cushion so the table box never kisses
    the frame edge (mirroring the tax appendix, which under-fills at 250mm).
    Ratios are preserved, so right-alignment and padding are unchanged.
    """
    total = sum(widths)
    if total <= 0:
        return [float(width) for width in widths]
    factor = target_mm / total
    return [width * factor for width in widths]


def build_report_table(
    rl: dict[str, Any],
    styles: dict[str, Any],
    rows: Any,
    *,
    widths: Any = None,
    col_widths: Any = None,
    header: bool = True,
    repeat: bool = True,
    compact: bool = False,
    right_columns: Any = (),
    body_style: str = "body",
) -> Any:
    """Shared table renderer: framed box, hairline inner grid, soft header with
    an ink underline, and zebra striping — the house table style for every
    report. ``widths`` are in millimetres; ``col_widths`` are absolute points.
    Columns listed in ``right_columns`` render with a right-aligned cell style.
    """
    Table = rl["Table"]
    TableStyle = rl["TableStyle"]
    colors = rl["colors"]
    mm = rl["mm"]
    right = set(right_columns)

    def cell_style(row_index: int, col_index: int) -> Any:
        if header and row_index == 0:
            base = "table_header"
        else:
            base = "small" if compact else body_style
        if col_index in right:
            return styles.get(f"{base}_right", styles[base])
        return styles[base]

    data = []
    for row_index, row in enumerate(rows):
        rendered = []
        for col_index, cell in enumerate(row):
            if hasattr(cell, "wrap"):
                rendered.append(cell)
            else:
                rendered.append(
                    rl["Paragraph"](escape_paragraph_text(cell), cell_style(row_index, col_index))
                )
        data.append(rendered)

    if col_widths is not None:
        resolved_widths = list(col_widths)
    elif widths is not None:
        resolved_widths = [width * mm for width in widths]
    else:
        resolved_widths = None

    table = Table(
        data,
        colWidths=resolved_widths,
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
    if header and len(data):
        commands.extend(
            [
                ("BACKGROUND", (0, 0), (-1, 0), colors.HexColor(BRAND_SOFT)),
                ("LINEBELOW", (0, 0), (-1, 0), 0.7, colors.HexColor(BRAND_INK)),
            ]
        )
    for row_index in range(1 if header else 0, len(data)):
        if row_index % 2 == 0:
            commands.append(
                ("BACKGROUND", (0, row_index), (-1, row_index), colors.HexColor(TABLE_ZEBRA))
            )
    table.setStyle(TableStyle(commands))
    return table


def decimal_value(value: Any) -> Decimal:
    if value is None or value == "":
        return Decimal("0")
    try:
        return Decimal(str(value))
    except (InvalidOperation, ValueError):
        return Decimal("0")


def draw_page_header(
    canvas: Any,
    doc: Any,
    *,
    title: str,
    fonts: dict[str, str],
    rl: dict[str, Any],
    brand_label: str = "Kassiber",
    footer_left: str = "",
    page_label: str | None = "Page",
    line_width: float = 0.4,
) -> None:
    colors = rl["colors"]
    # Use the actual page size so the masthead and footer sit correctly on
    # landscape pages too (doc.pagesize is fixed to the portrait template, so
    # relying on it drew the header off the top of a landscape page and put
    # the page number mid-width). Canvas._pagesize reflects the current
    # template's orientation — the same idiom the summary report already uses.
    width, height = getattr(canvas, "_pagesize", None) or doc.pagesize
    canvas.saveState()
    if brand_label:
        canvas.setFillColor(colors.HexColor(BRAND_INK))
        canvas.setFont(fonts["bold"], 8)
        canvas.drawString(doc.leftMargin, height - 9 * rl["mm"], brand_label)
        canvas.setFont(fonts["regular"], 7)
        canvas.setFillColor(colors.HexColor(BRAND_MUTED))
        canvas.drawRightString(width - doc.rightMargin, height - 9 * rl["mm"], title)
    else:
        canvas.setFont(fonts["regular"], 7)
        canvas.setFillColor(colors.HexColor(BRAND_MUTED))
        canvas.drawString(doc.leftMargin, height - 9 * rl["mm"], title)
    canvas.setStrokeColor(colors.HexColor(BRAND_LINE))
    canvas.setLineWidth(line_width)
    canvas.line(
        doc.leftMargin,
        height - 11 * rl["mm"],
        width - doc.rightMargin,
        height - 11 * rl["mm"],
    )
    canvas.setFont(fonts["regular"], 7)
    if footer_left:
        canvas.drawString(doc.leftMargin, 8 * rl["mm"], footer_left)
    if page_label:
        canvas.drawRightString(
            width - doc.rightMargin,
            8 * rl["mm"],
            f"{page_label} {doc.page}",
        )
    canvas.restoreState()
