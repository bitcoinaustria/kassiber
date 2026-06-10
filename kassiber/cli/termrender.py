"""Terminal rendering for `kassiber chat` — pretty output across all models.

Two halves, both stdlib-only:

- ``MarkdownStreamRenderer`` turns the markdown-lite that chat models emit
  into ANSI terminal output *while streaming*: bold, inline code, headers,
  bullets, blockquotes, fenced code, horizontal rules, and pipe tables
  re-drawn as box-aligned tables. Token-level streaming is preserved —
  only constructs that genuinely need lookahead (table blocks, fence
  lines) are buffered, and only until their line completes.
- ``render_envelope_table`` draws a compact, deterministic table from a
  tool-result envelope. The numbers shown come from the daemon, not from
  the model retyping them — small local models routinely mangle markdown
  tables (and occasionally digits), so the trustworthy presentation of
  tabular data never depends on the model at all.

Callers decide when ANSI is appropriate (chat.py gates on TTY and
``--plain``); this module always emits styled output.
"""

from __future__ import annotations

import re
import shutil
from typing import Any

from ..envelope import format_table_value


_BOLD = "\x1b[1m"
_BOLD_OFF = "\x1b[22m"
_DIM = "\x1b[2m"
_DIM_OFF = "\x1b[22m"
_CODE = "\x1b[36m"
_CODE_OFF = "\x1b[39m"

_TABLE_SEPARATOR_CELL = re.compile(r"^:?-+:?$")
_INLINE_BOLD = re.compile(r"\*\*(.+?)\*\*")
_INLINE_CODE = re.compile(r"`([^`]+)`")

_MAX_CELL_CHARS = 28
_DEFAULT_TERMINAL_WIDTH = 100


def _render_inline_cell(text: str) -> tuple[str, int]:
    """Render inline bold/code markers in one complete string.

    Returns (ansi_text, visible_length) so table layout can align on what
    the user actually sees.
    """
    visible = _INLINE_BOLD.sub(r"\1", text)
    visible = _INLINE_CODE.sub(r"\1", visible)
    rendered = _INLINE_BOLD.sub(rf"{_BOLD}\1{_BOLD_OFF}", text)
    rendered = _INLINE_CODE.sub(rf"{_CODE}\1{_CODE_OFF}", rendered)
    return rendered, len(visible)


def _box_table(
    header: list[tuple[str, int]],
    rows: list[list[tuple[str, int]]],
) -> str:
    """Draw an aligned table from (rendered, visible_width) cells."""
    column_count = max([len(header)] + [len(row) for row in rows]) if header or rows else 0
    if column_count == 0:
        return ""

    def _pad(cells: list[tuple[str, int]]) -> list[tuple[str, int]]:
        return cells + [("", 0)] * (column_count - len(cells))

    header = _pad(header)
    rows = [_pad(row) for row in rows]
    widths = [0] * column_count
    for cells in [header] + rows:
        for index, (_, visible) in enumerate(cells):
            widths[index] = max(widths[index], visible)

    border = _DIM + "│" + _DIM_OFF
    lines: list[str] = []

    def _format_row(cells: list[tuple[str, int]], *, bold: bool) -> str:
        parts = []
        for index, (rendered, visible) in enumerate(cells):
            text = f"{_BOLD}{rendered}{_BOLD_OFF}" if bold else rendered
            parts.append(" " + text + " " * (widths[index] - visible + 1))
        return border + border.join(parts) + border

    lines.append(_format_row(header, bold=True))
    rule = _DIM + "├" + "┼".join("─" * (width + 2) for width in widths) + "┤" + _DIM_OFF
    lines.append(rule)
    for cells in rows:
        lines.append(_format_row(cells, bold=False))
    return "\n".join(lines)


def render_markdown_table(lines: list[str]) -> str:
    """Render buffered ``|``-table lines as a box-aligned table."""
    parsed: list[list[str]] = []
    for line in lines:
        stripped = line.strip()
        if not stripped:
            continue
        cells = [cell.strip() for cell in stripped.strip("|").split("|")]
        if cells and all(_TABLE_SEPARATOR_CELL.match(cell) for cell in cells if cell):
            continue
        parsed.append(cells)
    if not parsed:
        return ""
    header = [_render_inline_cell(cell) for cell in parsed[0]]
    rows = [[_render_inline_cell(cell) for cell in row] for row in parsed[1:]]
    return _box_table(header, rows)


# ---------------------------------------------------------------------------
# Streaming markdown renderer
# ---------------------------------------------------------------------------

_LINE_START = "line_start"
_INLINE = "inline"
_FENCE = "fence"
_TABLE = "table"

_HR_CHARS = {"-", "*", "_"}


class MarkdownStreamRenderer:
    """Incremental markdown-lite → ANSI converter.

    ``feed`` returns text safe to print immediately; ``flush`` drains
    anything still buffered (an unfinished table, an open style). Feeding
    the same document in any chunking yields byte-identical output.
    """

    def __init__(self) -> None:
        self._state = _LINE_START
        self._hold = ""
        self._indent = ""
        self._bold = False
        self._code = False
        self._header = False
        self._pending_stars = 0
        self._fence_info_pending = False
        self._table_lines: list[str] = []
        self._table_current = ""

    # -- public API ---------------------------------------------------------

    def feed(self, chunk: str) -> str:
        out: list[str] = []
        for char in chunk:
            out.append(self._feed_char(char))
        return "".join(out)

    def flush(self) -> str:
        out: list[str] = []
        if self._state == _LINE_START and self._hold:
            out.append(self._resolve_line_start_hold(final=True))
        if self._state == _TABLE:
            if self._table_current:
                self._table_lines.append(self._table_current)
                self._table_current = ""
            out.append(self._render_table_buffer())
        if self._state == _FENCE and self._hold:
            out.append(self._emit_fence_line(self._hold))
            self._hold = ""
        out.append(self._resolve_pending_stars())
        out.append(self._close_styles())
        return "".join(out)

    # -- per-character machine ----------------------------------------------

    def _feed_char(self, char: str) -> str:
        if self._state == _FENCE:
            return self._feed_fence(char)
        if self._state == _TABLE:
            return self._feed_table(char)
        if self._state == _LINE_START:
            return self._feed_line_start(char)
        return self._feed_inline(char)

    def _feed_line_start(self, char: str) -> str:
        flushed = ""
        if self._table_lines:
            # The previous line ended a table unless this one continues it.
            if char == "|":
                self._state = _TABLE
                self._table_current = "|"
                return ""
            flushed = self._render_table_buffer()
        if char == "\n":
            resolved = self._resolve_line_start_hold(final=True)
            self._state = _LINE_START
            return (
                flushed
                + resolved
                + self._resolve_pending_stars()
                + self._close_styles()
                + "\n"
            )
        if char == " " and not self._hold:
            self._indent += char
            return flushed
        candidate = self._hold + char
        if self._could_become_marker(candidate):
            self._hold = candidate
            return flushed
        marker = self._resolve_marker(candidate)
        if marker is not None:
            return flushed + marker
        # Not a block construct: release the held prefix through inline.
        self._hold = ""
        self._state = _INLINE
        prefix = self._indent
        self._indent = ""
        return flushed + prefix + self.feed_inline_text(candidate)

    def _could_become_marker(self, text: str) -> bool:
        if not text:
            return True
        if all(c == "#" for c in text):
            return len(text) <= 6
        if all(c == "`" for c in text):
            return len(text) <= 2
        if all(c == text[0] for c in text) and text[0] in _HR_CHARS:
            # Could still be an hr line (--- etc.); bullet/bold resolve below.
            return len(text) <= 1 or text[0] in {"-", "_"} and len(text) <= 80
        if text == ">":
            return True
        return False

    def _resolve_marker(self, text: str) -> str | None:
        indent = self._indent
        if text == "|":
            self._state = _TABLE
            self._table_current = "|"
            self._hold = ""
            self._indent = ""
            return ""
        if re.fullmatch(r"#{1,6} ", text):
            self._state = _INLINE
            self._hold = ""
            self._indent = ""
            self._header = True
            return indent + _BOLD
        if text in {"- ", "* "}:
            self._state = _INLINE
            self._hold = ""
            self._indent = ""
            return indent + "• "
        if text == "> ":
            self._state = _INLINE
            self._hold = ""
            self._indent = ""
            return indent + _DIM + "┃" + _DIM_OFF + " "
        if text == "```":
            self._state = _FENCE
            self._hold = ""
            self._indent = ""
            self._fence_info_pending = True
            return ""
        return None

    def _resolve_line_start_hold(self, *, final: bool) -> str:
        text = self._hold
        indent = self._indent
        self._hold = ""
        self._indent = ""
        if not text:
            return indent
        if len(text) >= 3 and all(c == text[0] for c in text) and text[0] in _HR_CHARS:
            width = min(len(text), 40)
            return indent + _DIM + "─" * max(width, 8) + _DIM_OFF
        self._state = _INLINE
        out = indent + self.feed_inline_text(text)
        self._state = _LINE_START
        return out

    def feed_inline_text(self, text: str) -> str:
        return "".join(self._feed_inline(char) for char in text)

    def _feed_inline(self, char: str) -> str:
        if char == "\n":
            out = self._resolve_pending_stars() + self._close_styles() + "\n"
            self._state = _LINE_START
            return out
        if self._code:
            if char == "`":
                self._code = False
                return _CODE_OFF
            return char
        if char == "*":
            self._pending_stars += 1
            if self._pending_stars == 2:
                self._pending_stars = 0
                self._bold = not self._bold
                return _BOLD if self._bold else _BOLD_OFF
            return ""
        pending = self._resolve_pending_stars()
        if char == "`":
            self._code = True
            return pending + _CODE
        return pending + char

    def _resolve_pending_stars(self) -> str:
        if self._pending_stars:
            stars = "*" * self._pending_stars
            self._pending_stars = 0
            return stars
        return ""

    def _close_styles(self) -> str:
        out = ""
        if self._bold:
            self._bold = False
            out += _BOLD_OFF
        if self._code:
            self._code = False
            out += _CODE_OFF
        if self._header:
            self._header = False
            out += _BOLD_OFF
        return out

    # -- fences ---------------------------------------------------------------

    def _feed_fence(self, char: str) -> str:
        if char != "\n":
            self._hold += char
            return ""
        line = self._hold
        self._hold = ""
        if self._fence_info_pending:
            # Remainder of the opener line (the ```lang info string).
            self._fence_info_pending = False
            return ""
        if line.strip().startswith("```"):
            self._state = _LINE_START
            return ""
        return self._emit_fence_line(line) + "\n"

    def _emit_fence_line(self, line: str) -> str:
        return "  " + _CODE + line + _CODE_OFF

    # -- tables ---------------------------------------------------------------

    def _feed_table(self, char: str) -> str:
        if char != "\n":
            self._table_current += char
            return ""
        self._table_lines.append(self._table_current)
        self._table_current = ""
        self._state = _LINE_START
        return ""

    def _render_table_buffer(self) -> str:
        lines = self._table_lines
        self._table_lines = []
        if not lines:
            return ""
        rendered = render_markdown_table(lines)
        return rendered + "\n" if rendered else ""


# ---------------------------------------------------------------------------
# Deterministic tool-result tables
# ---------------------------------------------------------------------------

_PRIORITY_COLUMNS = (
    "occurred_at",
    "date",
    "year",
    "period",
    "label",
    "name",
    "title",
    "kind",
    "direction",
    "asset",
    "wallet",
    "amount",
    "quantity",
    "fiat_value",
    "fiat_currency",
    "fee",
    "rate",
    "status",
    "confidence",
    "count",
    "total",
    "balance",
    "note",
)
_SKIP_COLUMNS = {"id", "schema_version"}
_SKIP_COLUMN_SUFFIXES = ("_msat", "_id", "_json")


def _primary_rows(data: Any) -> list[dict[str, Any]]:
    """Find the main list-of-objects in an envelope payload."""
    candidates: list[list[dict[str, Any]]] = []
    if isinstance(data, list):
        candidates.append(data)
    elif isinstance(data, dict):
        for value in data.values():
            if isinstance(value, list):
                candidates.append(value)
    best: list[dict[str, Any]] = []
    for candidate in candidates:
        rows = [row for row in candidate if isinstance(row, dict)]
        if len(rows) == len(candidate) and len(rows) > len(best):
            best = rows
    return best


def _select_columns(rows: list[dict[str, Any]], width_budget: int) -> list[str]:
    seen: list[str] = []
    for row in rows:
        for key in row:
            if key not in seen:
                seen.append(key)
    usable = []
    for key in seen:
        if key in _SKIP_COLUMNS or key.endswith(_SKIP_COLUMN_SUFFIXES):
            continue
        values = [row.get(key) for row in rows]
        if all(value is None or isinstance(value, (dict, list)) for value in values):
            continue
        usable.append(key)
    ordered = [key for key in _PRIORITY_COLUMNS if key in usable]
    ordered.extend(key for key in usable if key not in ordered)
    selected: list[str] = []
    used_width = 0
    for key in ordered:
        column_width = min(
            max([len(key)] + [len(format_table_value(row.get(key))) for row in rows]),
            _MAX_CELL_CHARS,
        )
        if selected and used_width + column_width + 3 > width_budget:
            continue
        selected.append(key)
        used_width += column_width + 3
    return selected


def _clip(text: str) -> str:
    if len(text) <= _MAX_CELL_CHARS:
        return text
    return text[: _MAX_CELL_CHARS - 1] + "…"


def render_envelope_table(
    envelope: dict[str, Any],
    *,
    max_rows: int = 8,
    terminal_width: int | None = None,
) -> str | None:
    """Render a tool-result envelope as a compact aligned table.

    Returns ``None`` when the payload has no list-of-objects to show —
    scalar summaries read fine in the model's own prose.
    """
    if not isinstance(envelope, dict):
        return None
    rows = _primary_rows(envelope.get("data"))
    if not rows:
        return None
    if terminal_width is None:
        terminal_width = shutil.get_terminal_size(
            (_DEFAULT_TERMINAL_WIDTH, 24)
        ).columns
    shown = rows[:max_rows]
    columns = _select_columns(shown, max(terminal_width - 2, 40))
    if not columns:
        return None
    header = [(_clip(column), len(_clip(column))) for column in columns]
    body = []
    for row in shown:
        cells = []
        for column in columns:
            text = _clip(format_table_value(row.get(column)))
            cells.append((text, len(text)))
        body.append(cells)
    table = _box_table(header, body)
    if len(rows) > max_rows:
        table += f"\n{_DIM}… {len(rows) - max_rows} more rows{_DIM_OFF}"
    return table
