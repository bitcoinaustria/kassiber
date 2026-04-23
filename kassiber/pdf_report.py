"""PDF helpers for Kassiber report exports.

The preferred path renders Kassiber's line-oriented report output through
Qt rich text so the exported PDF can use styled headings, key/value rows,
and bordered tables while keeping the existing report-building pipeline
intact. If PySide6 is unavailable, Kassiber falls back to the legacy
stdlib-only text renderer.
"""

from html import escape as html_escape
from pathlib import Path
import os
import re
import textwrap


PAGE_WIDTH = 842
PAGE_HEIGHT = 595
LEFT_MARGIN = 40
RIGHT_MARGIN = 40
TOP_MARGIN = 42
BOTTOM_MARGIN = 42
FONT_SIZE = 9
LINE_HEIGHT = 11
MAX_LINE_CHARS = 138
LINES_PER_PAGE = int((PAGE_HEIGHT - TOP_MARGIN - BOTTOM_MARGIN) / LINE_HEIGHT)

TABLE_RULE_RE = re.compile(r"^\s*-+(?:\s{2,}-+)+\s*$")
NUMERIC_CELL_RE = re.compile(r"^[+-]?(?:\d[\d,]*)?(?:\.\d+)?$")
PDF_PAGE_RE = re.compile(rb"/Type /Page\b")
PDF_FONT_DIR = Path(__file__).resolve().parent / "ui" / "resources" / "fonts" / "pdf"
PDF_FONT_FILES = {
    "body_regular": "OpenSans-Regular.ttf",
    "body_bold": "OpenSans-Bold.ttf",
    "mono_regular": "RobotoMono-Regular.ttf",
}
_QT_PDF_FONT_FAMILIES = None


def _ascii_text(value):
    return str(value).encode("latin-1", "replace").decode("latin-1")


def _escape_pdf_text(value):
    return (
        _ascii_text(value)
        .replace("\\", "\\\\")
        .replace("(", "\\(")
        .replace(")", "\\)")
    )


def _ellipsize(value, width):
    text = _ascii_text(value)
    if len(text) <= width:
        return text
    if width <= 3:
        return text[:width]
    return text[: width - 3] + "..."


def wrap_report_lines(lines, width=MAX_LINE_CHARS):
    wrapped = []
    for line in lines:
        text = _ascii_text(line or "")
        if not text:
            wrapped.append("")
            continue
        pieces = textwrap.wrap(
            text,
            width=width,
            replace_whitespace=False,
            drop_whitespace=False,
            break_long_words=True,
            break_on_hyphens=False,
        )
        wrapped.extend(pieces or [""])
    return wrapped


def format_table(headers, rows, widths, align_right=None):
    align_right = set(align_right or ())

    def render_row(values):
        cells = []
        for index, (value, width) in enumerate(zip(values, widths)):
            text = _ellipsize(value, width)
            if index in align_right:
                cells.append(text.rjust(width))
            else:
                cells.append(text.ljust(width))
        return "  ".join(cells).rstrip()

    lines = [render_row(headers), "  ".join("-" * width for width in widths).rstrip()]
    for row in rows:
        lines.append(render_row(row))
    return lines


def _legacy_write_text_pdf(file_path, title, lines):
    path = Path(file_path).expanduser()
    path.parent.mkdir(parents=True, exist_ok=True)

    wrapped_lines = wrap_report_lines(lines)
    pages = [
        wrapped_lines[index : index + LINES_PER_PAGE]
        for index in range(0, len(wrapped_lines), LINES_PER_PAGE)
    ] or [[]]

    objects = [None]

    def add_object(payload):
        if isinstance(payload, str):
            payload = payload.encode("latin-1")
        objects.append(payload)
        return len(objects) - 1

    catalog_id = add_object(b"")
    pages_id = add_object(b"")
    font_id = add_object(b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>")
    info_id = add_object(
        f"<< /Title ({_escape_pdf_text(title)}) /Producer ({_escape_pdf_text('kassiber pdf report')}) >>"
    )

    page_ids = []
    content_ids = []
    start_y = PAGE_HEIGHT - TOP_MARGIN - FONT_SIZE

    for page_lines in pages:
        commands = [
            "BT",
            f"/F1 {FONT_SIZE} Tf",
            f"{LINE_HEIGHT} TL",
            f"{LEFT_MARGIN} {start_y} Td",
        ]
        for index, line in enumerate(page_lines):
            if index:
                commands.append("T*")
            commands.append(f"({_escape_pdf_text(line)}) Tj")
        commands.append("ET")
        stream = "\n".join(commands).encode("latin-1")
        content_id = add_object(
            b"<< /Length %d >>\nstream\n" % len(stream) + stream + b"\nendstream"
        )
        page_id = add_object(b"")
        content_ids.append(content_id)
        page_ids.append(page_id)

    kid_refs = " ".join(f"{page_id} 0 R" for page_id in page_ids)
    objects[pages_id] = f"<< /Type /Pages /Count {len(page_ids)} /Kids [{kid_refs}] >>".encode("latin-1")
    objects[catalog_id] = f"<< /Type /Catalog /Pages {pages_id} 0 R >>".encode("latin-1")

    for page_id, content_id in zip(page_ids, content_ids):
        objects[page_id] = (
            f"<< /Type /Page /Parent {pages_id} 0 R "
            f"/MediaBox [0 0 {PAGE_WIDTH} {PAGE_HEIGHT}] "
            f"/Resources << /Font << /F1 {font_id} 0 R >> >> "
            f"/Contents {content_id} 0 R >>"
        ).encode("latin-1")

    output = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
    offsets = [0]
    for object_id in range(1, len(objects)):
        offsets.append(len(output))
        output.extend(f"{object_id} 0 obj\n".encode("latin-1"))
        output.extend(objects[object_id])
        output.extend(b"\nendobj\n")

    xref_offset = len(output)
    output.extend(f"xref\n0 {len(objects)}\n".encode("latin-1"))
    output.extend(b"0000000000 65535 f \n")
    for offset in offsets[1:]:
        output.extend(f"{offset:010d} 00000 n \n".encode("latin-1"))
    output.extend(
        (
            f"trailer\n<< /Size {len(objects)} /Root {catalog_id} 0 R /Info {info_id} 0 R >>\n"
            f"startxref\n{xref_offset}\n%%EOF\n"
        ).encode("latin-1")
    )

    path.write_bytes(output)
    return {
        "file": str(path.resolve()),
        "pages": len(page_ids),
        "bytes": len(output),
        "title": title,
    }


def _is_rule(line, char):
    stripped = (line or "").strip()
    return bool(stripped) and set(stripped) == {char}


def _looks_like_key_value(line):
    if ":" not in line:
        return False
    label, _, value = line.partition(":")
    return bool(label.strip()) and len(label.strip()) <= 32 and bool(value)


def _table_column_spans(separator_line):
    return [(match.start(), match.end()) for match in re.finditer(r"-+", separator_line)]


def _slice_table_row(line, spans):
    cells = []
    for start, end in spans:
        cells.append((line[start:end] if start < len(line) else "").strip())
    return cells


def _is_numeric_cell(value):
    return bool(value) and bool(NUMERIC_CELL_RE.fullmatch(value.replace("%", "")))


def _numeric_table_columns(headers, rows):
    numeric = set()
    width = len(headers)
    for index in range(width):
        values = [row[index] for row in rows if index < len(row) and row[index]]
        if values and all(_is_numeric_cell(value) for value in values):
            numeric.add(index)
    return numeric


def _parse_report_lines(title, lines):
    entries = list(lines)
    if len(entries) >= 2 and entries[0] == title and _is_rule(entries[1], "="):
        index = 2
    else:
        index = 0

    sections = [{"heading": None, "blocks": []}]
    current = sections[0]
    while index < len(entries):
        line = entries[index]
        if not line.strip():
            index += 1
            continue
        if index + 1 < len(entries) and _is_rule(entries[index + 1], "-") and not TABLE_RULE_RE.match(entries[index + 1]):
            current = {"heading": line.strip(), "blocks": []}
            sections.append(current)
            index += 2
            continue
        if index + 1 < len(entries) and TABLE_RULE_RE.match(entries[index + 1]):
            spans = _table_column_spans(entries[index + 1])
            headers = _slice_table_row(entries[index], spans)
            rows = []
            index += 2
            while index < len(entries):
                row_line = entries[index]
                if not row_line.strip():
                    break
                if index + 1 < len(entries) and _is_rule(entries[index + 1], "-") and not TABLE_RULE_RE.match(entries[index + 1]):
                    break
                rows.append(_slice_table_row(row_line, spans))
                index += 1
            current["blocks"].append(
                {
                    "kind": "table",
                    "headers": headers,
                    "rows": rows,
                    "numeric_columns": _numeric_table_columns(headers, rows),
                }
            )
            continue
        if _looks_like_key_value(line):
            pairs = []
            while index < len(entries):
                kv_line = entries[index]
                if not kv_line.strip() or not _looks_like_key_value(kv_line):
                    break
                label, _, value = kv_line.partition(":")
                pairs.append((label.strip(), value.strip()))
                index += 1
            current["blocks"].append({"kind": "key_value", "pairs": pairs})
            continue
        paragraphs = []
        while index < len(entries):
            paragraph_line = entries[index]
            if not paragraph_line.strip():
                break
            if index + 1 < len(entries) and _is_rule(entries[index + 1], "-") and not TABLE_RULE_RE.match(entries[index + 1]):
                break
            if index + 1 < len(entries) and TABLE_RULE_RE.match(entries[index + 1]):
                break
            if _looks_like_key_value(paragraph_line):
                break
            paragraphs.append(paragraph_line.strip())
            index += 1
        current["blocks"].append({"kind": "paragraphs", "lines": paragraphs})

    return [section for section in sections if section["heading"] or section["blocks"]]


def _bundled_pdf_font_path(key):
    return PDF_FONT_DIR / PDF_FONT_FILES[key]


def _count_pdf_pages(path):
    return len(PDF_PAGE_RE.findall(path.read_bytes()))


def _load_bundled_pdf_font_families(qfont_database):
    global _QT_PDF_FONT_FAMILIES
    if _QT_PDF_FONT_FAMILIES is not None:
        return _QT_PDF_FONT_FAMILIES

    loaded = {}
    for key in PDF_FONT_FILES:
        font_path = _bundled_pdf_font_path(key)
        if not font_path.exists():
            raise RuntimeError(f"Bundled PDF font is missing: {font_path}")
        font_id = qfont_database.addApplicationFont(str(font_path))
        if font_id < 0:
            raise RuntimeError(f"Failed to load bundled PDF font: {font_path}")
        families = qfont_database.applicationFontFamilies(font_id)
        if not families:
            raise RuntimeError(f"Bundled PDF font did not register a family: {font_path}")
        loaded[key] = families[0]

    _QT_PDF_FONT_FAMILIES = {
        "body": loaded["body_regular"],
        "mono": loaded["mono_regular"],
    }
    return _QT_PDF_FONT_FAMILIES


def _build_report_html(title, lines, body_font_family, mono_font_family):
    sections = _parse_report_lines(title, lines)
    parts = [
        "<html><head><meta charset='utf-8' />",
        "<style>",
        f"body {{ font-family: '{html_escape(body_font_family)}'; color: #1d2b2f; font-size: 10pt; line-height: 1.35; }}",
        "h1 { color: #0b6252; font-size: 22pt; margin: 0 0 6pt 0; }",
        "h2 { color: #0b6252; font-size: 14pt; margin: 18pt 0 6pt 0; padding-bottom: 4pt; border-bottom: 1px solid #d9e5e1; }",
        "div.cover { margin-bottom: 14pt; padding-bottom: 8pt; border-bottom: 2px solid #0b6252; }",
        "div.subtitle { color: #597277; font-size: 9pt; }",
        "table.kv { width: 100%; border-collapse: collapse; margin: 4pt 0 10pt 0; }",
        "table.kv td.label { width: 34%; font-weight: bold; color: #496166; padding: 3pt 10pt 3pt 0; border-bottom: 1px solid #e7eeeb; }",
        "table.kv td.value { padding: 3pt 0 3pt 10pt; border-bottom: 1px solid #e7eeeb; }",
        "table.report { width: 100%; border-collapse: collapse; margin: 6pt 0 12pt 0; }",
        "table.report th { background-color: #edf4f2; color: #203136; font-weight: bold; padding: 5pt 6pt; border: 1px solid #d6e2de; }",
        "table.report td { padding: 4pt 6pt; border: 1px solid #e3ece8; vertical-align: top; }",
        "table.report td.numeric, table.report th.numeric { text-align: right; }",
        f"table.report td.numeric {{ font-family: '{html_escape(mono_font_family)}'; }}",
        "p.note { margin: 0 0 8pt 0; color: #43555a; }",
        "</style></head><body>",
        f"<div class='cover'><h1>{html_escape(title)}</h1><div class='subtitle'>Kassiber local-first accounting report export</div></div>",
    ]

    for section in sections:
        if section["heading"]:
            parts.append(f"<h2>{html_escape(section['heading'])}</h2>")
        for block in section["blocks"]:
            if block["kind"] == "key_value":
                parts.append("<table class='kv'>")
                for label, value in block["pairs"]:
                    value_html = html_escape(value) if value else "&nbsp;"
                    parts.append(
                        "<tr>"
                        f"<td class='label'>{html_escape(label)}</td>"
                        f"<td class='value'>{value_html}</td>"
                        "</tr>"
                    )
                parts.append("</table>")
                continue
            if block["kind"] == "table":
                parts.append("<table class='report'>")
                parts.append("<tr>")
                for index, header in enumerate(block["headers"]):
                    klass = " class='numeric'" if index in block["numeric_columns"] else ""
                    parts.append(f"<th{klass}>{html_escape(header)}</th>")
                parts.append("</tr>")
                for row in block["rows"]:
                    parts.append("<tr>")
                    for index, cell in enumerate(row):
                        klass = " class='numeric'" if index in block["numeric_columns"] else ""
                        value_html = html_escape(cell) if cell else "&nbsp;"
                        parts.append(f"<td{klass}>{value_html}</td>")
                    parts.append("</tr>")
                parts.append("</table>")
                continue
            for line in block["lines"]:
                parts.append(f"<p class='note'>{html_escape(line)}</p>")

    parts.append("</body></html>")
    return "".join(parts)


def _write_qt_text_pdf(file_path, title, lines):
    from PySide6.QtCore import QMarginsF, QSizeF
    from PySide6.QtGui import (
        QFont,
        QGuiApplication,
        QFontDatabase,
        QPageLayout,
        QPageSize,
        QPdfWriter,
        QTextDocument,
    )

    path = Path(file_path).expanduser()
    path.parent.mkdir(parents=True, exist_ok=True)

    app = QGuiApplication.instance()
    if app is None:
        os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
        app = QGuiApplication(["kassiber-pdf"])

    font_families = _load_bundled_pdf_font_families(QFontDatabase)

    html = _build_report_html(title, lines, font_families["body"], font_families["mono"])
    writer = QPdfWriter(str(path))
    writer.setPageLayout(
        QPageLayout(
            QPageSize(QPageSize.PageSizeId.A4),
            QPageLayout.Orientation.Landscape,
            QMarginsF(18, 18, 18, 18),
        )
    )
    writer.setTitle(title)
    writer.setCreator("kassiber pdf report")

    document = QTextDocument()
    document.setDocumentMargin(0)
    document.setDefaultFont(QFont(font_families["body"]))
    document.setPageSize(QSizeF(writer.width(), writer.height()))
    document.setHtml(html)
    document.print_(writer)
    page_count = _count_pdf_pages(path)
    return {
        "file": str(path.resolve()),
        "pages": page_count,
        "bytes": path.stat().st_size,
        "title": title,
    }


def write_text_pdf(file_path, title, lines):
    try:
        return _write_qt_text_pdf(file_path, title, lines)
    except (ImportError, RuntimeError):
        return _legacy_write_text_pdf(file_path, title, lines)
