"""PDF helpers for Kassiber report exports.

Renders Kassiber's line-oriented report output to a self-contained PDF
using only the Python standard library — no third-party PDF dependency,
no native renderer. The output is a fixed-pitch text layout suitable for
review/audit handoffs; it deliberately trades typographic polish for
zero runtime dependencies.

Known limitation: text is encoded as Latin-1 with `replace` (see
`_ascii_text`). Characters outside Latin-1 — `€`, `₿`, `↔`, non-European
scripts — are silently substituted with `?` in the rendered PDF. This
affects Austrian E 1kv exports and any user content with non-Latin-1
glyphs. A Unicode-safe renderer is tracked as a follow-up in TODO.md.
"""

from pathlib import Path
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


def write_text_pdf(file_path, title, lines):
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
