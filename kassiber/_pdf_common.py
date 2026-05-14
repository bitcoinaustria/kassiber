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
    page_label: str = "Page",
    line_width: float = 0.4,
) -> None:
    colors = rl["colors"]
    width, height = doc.pagesize
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
    canvas.drawRightString(
        width - doc.rightMargin,
        8 * rl["mm"],
        f"{page_label} {doc.page}",
    )
    canvas.restoreState()
