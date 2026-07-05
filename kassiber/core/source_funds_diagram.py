"""Single-source diagram renderer for source-of-funds reports.

The same ``reportlab.graphics`` ``Drawing`` is rendered two ways:

- embedded directly in the PDF (native vector, selectable text), and
- serialized to an SVG string via ``reportlab.graphics.renderSVG`` for the
  desktop disclosure preview.

Because both outputs come from one builder applied to the same frozen
``simplified_flow`` data, the PDF and the GUI preview cannot drift. The SVG
path rewrites reportlab-internal font names to web-safe CSS stacks so a
browser renders the same Latin/€ glyphs without the bundled TTF.

This module is intentionally free of any back-edge into
``source_funds``: it consumes already-built report sub-structures so the
hot ``compute_coverage`` path never pays for diagram rendering.
"""

from __future__ import annotations

from decimal import Decimal
from typing import Any, Mapping, Sequence

from .._pdf_common import (
    BRAND_ACCENT,
    BRAND_INK,
    BRAND_LINE,
    BRAND_MUTED,
)


# Kept local (not imported from source_funds) to avoid an import cycle: this
# module is consumed by both source_funds and the PDF builder.
_ATTESTATION_KINDS = ("missing_history", "opening_balance_attestation")
_PRIVACY_LINK_TYPES = ("coinjoin", "payjoin")
_SWAP_LINK_TYPES = ("swap", "peg_in", "peg_out", "lightning_swap")

# Bitcoin-native node palette (fill, stroke, dashed).
_NODE_STYLE = {
    "target": ("#ffffff", BRAND_ACCENT, False),
    "source_real": ("#ecfdf5", "#16a34a", False),
    "source_attest": ("#fffbeb", "#d97706", True),
    "privacy": ("#fff7ed", "#ea580c", True),
    "hop": ("#ffffff", BRAND_LINE, False),
}

_LEGEND_ORDER = [
    ("target", "Target"),
    ("source_real", "Root source"),
    ("source_attest", "Attestation / missing history"),
    ("hop", "Transaction hop"),
    ("swap", "Swap / peg"),
    ("privacy", "Privacy hop (deferred)"),
]

# Deterministic ring-segment palette keyed by source-mix root kind. Anything
# not listed cycles through the fallback list so colours stay stable.
_RING_COLORS = {
    "fiat_purchase": "#2563eb",
    "prior_exchange_withdrawal": "#0ea5e9",
    "income": "#16a34a",
    "mining": "#65a30d",
    "gift": "#a855f7",
    "opening_balance_attestation": "#d97706",
    "missing_history": "#9ca3af",
    "unknown": "#6b7280",
    "wallet": "#2563eb",
    "exchange": "#0ea5e9",
    "import": "#16a34a",
    "manual": "#d97706",
    "blockchain": "#0891b2",
    "chain_sync": "#0891b2",
    "platform_export": "#2563eb",
    "manual_import": "#d97706",
}
_RING_FALLBACK = ["#2563eb", "#16a34a", "#d97706", "#a855f7", "#0891b2", "#dc2626"]


def _hex(colors: Any, value: str | None) -> Any:
    if value is None:
        return None
    return colors.HexColor(value)


def _btc(value: Any) -> str:
    if value is None or value == "":
        return ""
    return f"{Decimal(str(value)).quantize(Decimal('0.00000001')):.8f}"


def _pct(value: Any) -> str:
    if value is None or value == "":
        return ""
    return f"{Decimal(str(value)).quantize(Decimal('0.1'))}%"


def _label(value: Any) -> str:
    return str(value or "").replace("_", " ")


def _fit_text(pdfmetrics: Any, text: Any, max_width: float, font: str, size: float) -> str:
    rendered = str(text or "")
    if not rendered:
        return ""
    if pdfmetrics.stringWidth(rendered, font, size) <= max_width:
        return rendered
    ellipsis = "..."
    available = max_width - pdfmetrics.stringWidth(ellipsis, font, size)
    if available <= 0:
        return ellipsis
    while rendered and pdfmetrics.stringWidth(rendered, font, size) > available:
        rendered = rendered[:-1]
    return f"{rendered.rstrip()}{ellipsis}"


DIAGRAM_DETAIL_LEVELS = ("summary", "detailed")


def detail_thresholds(detail: Any) -> tuple[int, int]:
    """Map a diagram-detail level to (max_levels, max_nodes) clustering caps.

    ``summary`` (default) keeps the legible collapsed view; ``detailed`` lets
    power users disclose more of the path before clustering kicks in. Surfaced
    as an advanced, snapshot-frozen report option.
    """
    if str(detail or "summary").strip().lower() == "detailed":
        return (16, 10)
    return (6, 4)


def visible_levels(
    levels: Sequence[Mapping[str, Any]],
    *,
    max_levels: int = 6,
    max_nodes: int = 4,
) -> list[dict[str, Any]]:
    """Collapse long/wide paths so the simplified chart stays legible.

    ``max_levels`` / ``max_nodes`` are surfaced as advanced settings later;
    the defaults match the previously hard-coded behaviour.
    """
    levels = [dict(level) for level in levels]
    if max_levels and len(levels) > max_levels:
        hidden = len(levels) - (max_levels - 1)
        head = max(1, (max_levels - 1) // 2 + 1)
        tail = (max_levels - 1) - head
        levels = [
            *levels[:head],
            {
                "role": "omitted",
                "nodes": [
                    {
                        "id": "synthetic:omitted",
                        "label": f"{hidden} path levels omitted",
                        "kind": "continued path",
                        "asset": "",
                        "amount": None,
                        "node_type": "omitted",
                        "deferred_privacy_hop": False,
                    }
                ],
            },
            *(levels[-tail:] if tail else []),
        ]
    out: list[dict[str, Any]] = []
    for index, level in enumerate(levels):
        nodes = list(level.get("nodes") or [])
        if max_nodes and len(nodes) > max_nodes:
            extra = len(nodes) - (max_nodes - 1)
            nodes = [
                *nodes[: max_nodes - 1],
                {
                    "id": f"synthetic:extra:{level.get('level', index)}",
                    "label": f"{extra} more items",
                    "kind": "additional reviewed nodes",
                    "asset": "",
                    "amount": None,
                    "node_type": "omitted",
                    "deferred_privacy_hop": False,
                },
            ]
        next_level = dict(level)
        next_level["nodes"] = nodes
        out.append(next_level)
    return out


def source_mix_segments(report: Mapping[str, Any]) -> list[dict[str, Any]]:
    """Donut segments for the source-mix ring (one per reviewed root kind)."""
    return [
        {
            "key": row.get("source_type"),
            "label": _label(row.get("source_type")),
            "value": float(Decimal(str(row.get("amount") or 0))),
        }
        for row in report.get("source_mix") or []
        if Decimal(str(row.get("amount") or 0)) > 0
    ]


def data_source_segments(report: Mapping[str, Any]) -> list[dict[str, Any]]:
    """Donut segments for the data-source ring (transactions by provenance).

    Falls back to the legacy by-kind grouping for case snapshots saved
    before ``data_provenance_summary`` existed.
    """
    segments = [
        {
            "key": str(row.get("provenance") or "manual_import"),
            "label": str(row.get("label") or _label(row.get("provenance"))),
            "value": float(row.get("count") or 0),
        }
        for row in report.get("data_provenance_summary") or []
        if (row.get("count") or 0) > 0
    ]
    if segments:
        return segments
    by_kind: dict[str, int] = {}
    for item in report.get("data_sources") or []:
        kind = str(item.get("kind") or "other")
        by_kind[kind] = by_kind.get(kind, 0) + int(item.get("transaction_count") or 0)
    return [
        {"key": kind, "label": _label(kind), "value": float(count)}
        for kind, count in sorted(by_kind.items())
        if count > 0
    ]


def _node_category(node: Mapping[str, Any], role: str) -> str:
    if role == "target":
        return "target"
    if node.get("deferred_privacy_hop"):
        return "privacy"
    if node.get("node_type") == "source":
        if node.get("kind") in _ATTESTATION_KINDS:
            return "source_attest"
        return "source_real"
    return "hop"


def _edge_color(colors: Any, edge: Mapping[str, Any]) -> str:
    link_type = edge.get("link_type")
    if edge.get("deferred_privacy_hop") or link_type in _PRIVACY_LINK_TYPES:
        return "#ea580c"
    if link_type in _SWAP_LINK_TYPES:
        return "#2563eb"
    return BRAND_MUTED


def build_flow_drawing(
    rl: dict[str, Any],
    fonts: dict[str, str],
    flow: Mapping[str, Any],
    *,
    width: float = 500.0,
    max_levels: int = 6,
    max_nodes: int = 4,
) -> Any:
    """Build a weighted Sankey-style flow ``Drawing`` from ``simplified_flow``.

    Edges carry value + % labels and a thickness proportional to the share of
    the target amount they fund. Returns a ``reportlab.graphics`` ``Drawing``,
    which is itself a platypus ``Flowable`` and embeds straight into the story.
    """
    Drawing = rl["Drawing"]
    Rect = rl["Rect"]
    String = rl["String"]
    Line = rl["Line"]
    colors = rl["colors"]
    pdfmetrics = rl["pdfmetrics"]

    levels = visible_levels(flow.get("levels") or [], max_levels=max_levels, max_nodes=max_nodes)
    if not levels:
        d = Drawing(width, 24)
        d.add(String(0, 8, "No simplified flow path available.", fontName=fonts["regular"], fontSize=8, fillColor=_hex(colors, BRAND_MUTED)))
        return d

    box_h = 40.0
    row_gap = 12.0
    col_gap = 16.0
    header_h = 12.0
    legend_h = 16.0
    pad = 4.0

    level_count = len(levels)
    col_w = (width - col_gap * (level_count - 1)) / level_count
    box_w = max(60.0, col_w)
    max_rows = max((len(level.get("nodes") or []) for level in levels), default=1)
    body_h = max_rows * box_h + max(0, max_rows - 1) * row_gap
    height = pad + legend_h + body_h + header_h + pad

    drawing = Drawing(width, height)

    body_top = height - pad - header_h

    # Position every node box, keyed by id.
    positions: dict[str, tuple[float, float, float, float]] = {}
    for col, level in enumerate(levels):
        nodes = list(level.get("nodes") or [])
        group_h = len(nodes) * box_h + max(0, len(nodes) - 1) * row_gap
        base_top = body_top - (body_h - group_h) / 2
        x = col * (col_w + col_gap)
        for row, node in enumerate(nodes):
            y = base_top - box_h - row * (box_h + row_gap)
            positions[str(node.get("id") or f"synthetic:{col}:{row}")] = (x, y, box_w, box_h)

    # Edges first so node boxes paint over the line ends; labels are drawn
    # last (on top of the boxes) so narrow columns can't clip them.
    present_categories: set[str] = set()
    has_swap_edge = False
    edge_labels: list[tuple[float, float, str, str]] = []
    for edge in flow.get("edges") or []:
        from_box = positions.get(str(edge.get("from") or ""))
        to_box = positions.get(str(edge.get("to") or ""))
        if not from_box or not to_box:
            continue
        fx, fy, fw, fh = from_box
        tx, ty, _tw, th = to_box
        if fx == tx:
            # Same column (e.g. clustered/omitted synthetic nodes) — there is
            # no meaningful horizontal connector to draw.
            continue
        link_type = edge.get("link_type")
        if link_type in _SWAP_LINK_TYPES:
            has_swap_edge = True
        deferred = bool(edge.get("deferred_privacy_hop")) or link_type in _PRIVACY_LINK_TYPES
        color = _edge_color(colors, edge)
        percent = edge.get("percent_of_target")
        try:
            share = float(percent) / 100.0 if percent is not None else 0.0
        except (TypeError, ValueError):
            share = 0.0
        stroke_w = max(0.6, min(4.0, 0.6 + share * 4.0))
        # Levels run target (left) -> root sources (right), and each edge points
        # from a parent (``from``) to the child it funds (``to``) one hop closer
        # to the target. The parent is therefore normally to the RIGHT of the
        # child, so connect the two inner edges and land the arrowhead on the
        # child regardless of column order (the old ``tx <= fx`` guard skipped
        # every edge, leaving the diagram with no connectors at all).
        from_mid_y = fy + fh / 2
        to_mid_y = ty + th / 2
        if fx > tx:
            start_x = fx          # parent inner (left) edge
            tip_x = tx + _tw      # child inner (right) edge; arrow points left
            barb_dx = 5
        else:
            start_x = fx + fw     # parent inner (right) edge
            tip_x = tx            # child inner (left) edge; arrow points right
            barb_dx = -5
        drawing.add(
            Line(
                start_x,
                from_mid_y,
                tip_x,
                to_mid_y,
                strokeColor=_hex(colors, color),
                strokeWidth=stroke_w,
                strokeDashArray=[2, 2] if deferred else None,
            )
        )
        drawing.add(Line(tip_x, to_mid_y, tip_x + barb_dx, to_mid_y + 3, strokeColor=_hex(colors, color), strokeWidth=stroke_w))
        drawing.add(Line(tip_x, to_mid_y, tip_x + barb_dx, to_mid_y - 3, strokeColor=_hex(colors, color), strokeWidth=stroke_w))
        pct_text = _pct(percent)
        if pct_text:
            edge_labels.append(((start_x + tip_x) / 2, (from_mid_y + to_mid_y) / 2, pct_text, color))

    # Role headers + node boxes.
    for col, level in enumerate(levels):
        nodes = list(level.get("nodes") or [])
        if not nodes:
            continue
        role = str(level.get("role") or "flow")
        first_box = positions.get(str(nodes[0].get("id") or ""))
        if first_box:
            hx, _hy, hw, _hh = first_box
            drawing.add(
                String(
                    hx + hw / 2,
                    height - pad - 8,
                    _label(role).upper(),
                    fontName=fonts["bold"],
                    fontSize=6.0,
                    fillColor=_hex(colors, BRAND_MUTED),
                    textAnchor="middle",
                )
            )
        for row, node in enumerate(nodes):
            box = positions.get(str(node.get("id") or f"synthetic:{col}:{row}"))
            if not box:
                continue
            x, y, w, h = box
            category = _node_category(node, role)
            present_categories.add(category)
            fill, stroke, dashed = _NODE_STYLE.get(category, _NODE_STYLE["hop"])
            drawing.add(
                Rect(
                    x,
                    y,
                    w,
                    h,
                    rx=4,
                    ry=4,
                    fillColor=_hex(colors, fill),
                    strokeColor=_hex(colors, stroke),
                    strokeWidth=1.1 if category in {"target", "privacy", "source_attest"} else 0.6,
                    strokeDashArray=[2, 2] if dashed else None,
                )
            )
            label = _fit_text(pdfmetrics, node.get("label"), w - 8, fonts["bold"], 6.6)
            kind = "privacy hop deferred" if node.get("deferred_privacy_hop") else _label(node.get("kind"))
            kind = _fit_text(pdfmetrics, kind, w - 8, fonts["regular"], 5.8)
            amount = _fit_text(
                pdfmetrics,
                f"{_btc(node.get('amount'))} {node.get('asset', '')}".strip() if node.get("amount") is not None else "",
                w - 8,
                fonts["mono"],
                5.7,
            )
            drawing.add(String(x + 4, y + h - 11, label, fontName=fonts["bold"], fontSize=6.6, fillColor=_hex(colors, BRAND_INK)))
            drawing.add(String(x + 4, y + h - 21, kind, fontName=fonts["regular"], fontSize=5.8, fillColor=_hex(colors, BRAND_MUTED)))
            if amount:
                drawing.add(String(x + 4, y + 6, amount, fontName=fonts["mono"], fontSize=5.7, fillColor=_hex(colors, BRAND_INK)))

    # Edge percentage labels on top of the boxes so narrow gaps can't clip them.
    for mid_x, mid_y, pct_text, color in edge_labels:
        drawing.add(
            String(
                mid_x,
                mid_y + 1,
                pct_text,
                fontName=fonts["bold"],
                fontSize=5.4,
                fillColor=_hex(colors, color),
                textAnchor="middle",
            )
        )

    # Legend along the bottom.
    legend_items = [
        (cat, text)
        for cat, text in _LEGEND_ORDER
        if cat in present_categories or (cat == "swap" and has_swap_edge)
    ]
    lx = 0.0
    for cat, text in legend_items:
        if cat == "swap":
            fill, stroke = "#dbeafe", "#2563eb"
        else:
            fill, stroke, _dash = _NODE_STYLE.get(cat, _NODE_STYLE["hop"])
        drawing.add(Rect(lx, pad + 2, 8, 8, rx=1.5, ry=1.5, fillColor=_hex(colors, fill), strokeColor=_hex(colors, stroke), strokeWidth=0.6))
        drawing.add(String(lx + 11, pad + 3, text, fontName=fonts["regular"], fontSize=6.0, fillColor=_hex(colors, BRAND_MUTED)))
        lx += 15 + pdfmetrics.stringWidth(text, fonts["regular"], 6.0) + 12

    return drawing


def build_ring_drawing(
    rl: dict[str, Any],
    fonts: dict[str, str],
    segments: Sequence[Mapping[str, Any]],
    *,
    center_value: str = "",
    center_label: str = "",
    width: float = 240.0,
    diameter: float = 90.0,
) -> Any:
    """Donut chart with a legend, drawn from ``{key,label,value}`` segments."""
    Drawing = rl["Drawing"]
    String = rl["String"]
    Wedge = rl["Wedge"]
    Circle = rl["Circle"]
    colors = rl["colors"]
    pdfmetrics = rl["pdfmetrics"]

    rows = [
        {
            "key": str(seg.get("key") or ""),
            "label": str(seg.get("label") or seg.get("key") or ""),
            "value": float(seg.get("value") or 0.0),
        }
        for seg in segments
    ]
    total = sum(max(0.0, row["value"]) for row in rows)
    legend_h = len(rows) * 11 + 6
    height = max(diameter + 12, legend_h)
    drawing = Drawing(width, height)

    cx = diameter / 2 + 4
    cy = height / 2
    radius = diameter / 2

    if total <= 0:
        drawing.add(Circle(cx, cy, radius, fillColor=_hex(colors, "#f3f4f6"), strokeColor=_hex(colors, BRAND_LINE), strokeWidth=0.6))
    else:
        start = 90.0
        for index, row in enumerate(rows):
            value = max(0.0, row["value"])
            if value <= 0:
                continue
            sweep = value / total * 360.0
            end = start - sweep
            color = _RING_COLORS.get(row["key"]) or _RING_FALLBACK[index % len(_RING_FALLBACK)]
            drawing.add(
                Wedge(
                    cx,
                    cy,
                    radius,
                    end,
                    start,
                    fillColor=_hex(colors, color),
                    strokeColor=_hex(colors, "#ffffff"),
                    strokeWidth=0.8,
                )
            )
            start = end
    # Punch the donut hole.
    drawing.add(Circle(cx, cy, radius * 0.58, fillColor=_hex(colors, "#ffffff"), strokeColor=None))
    if center_value:
        drawing.add(String(cx, cy + 1, _fit_text(pdfmetrics, center_value, radius * 1.1, fonts["bold"], 9), fontName=fonts["bold"], fontSize=9, fillColor=_hex(colors, BRAND_INK), textAnchor="middle"))
    if center_label:
        drawing.add(String(cx, cy - 9, _fit_text(pdfmetrics, center_label, radius * 1.1, fonts["regular"], 6), fontName=fonts["regular"], fontSize=6, fillColor=_hex(colors, BRAND_MUTED), textAnchor="middle"))

    legend_x = diameter + 14
    legend_top = cy + legend_h / 2 - 8
    for index, row in enumerate(rows):
        color = _RING_COLORS.get(row["key"]) or _RING_FALLBACK[index % len(_RING_FALLBACK)]
        ly = legend_top - index * 11
        drawing.add(rl["Rect"](legend_x, ly, 7, 7, rx=1.5, ry=1.5, fillColor=_hex(colors, color), strokeColor=None))
        share = (row["value"] / total * 100.0) if total > 0 else 0.0
        text = _fit_text(pdfmetrics, f"{_label(row['label'])}  {share:.1f}%", width - legend_x - 18, fonts["regular"], 6.4)
        drawing.add(String(legend_x + 11, ly + 0.5, text, fontName=fonts["regular"], fontSize=6.4, fillColor=_hex(colors, BRAND_INK)))
    return drawing


def _websafe_font_map(fonts: dict[str, str]) -> dict[str, tuple[str, str]]:
    sans = "'DejaVu Sans', Helvetica, Arial, sans-serif"
    mono = "'DejaVu Sans Mono', 'Courier New', Courier, monospace"
    return {
        fonts["bold"]: (sans, "700"),
        fonts["regular"]: (sans, "400"),
        fonts["mono"]: (mono, "400"),
    }


def render_drawing_to_svg(rl: dict[str, Any], fonts: dict[str, str], drawing: Any) -> str:
    """Serialize a ``Drawing`` to an inline-embeddable SVG string.

    reportlab's ``renderSVG`` references fonts by their internal name and never
    embeds glyph data, so the family names are rewritten to web-safe CSS stacks
    (longest first, so the bold name is replaced before the regular prefix).
    """
    from reportlab.graphics import renderSVG

    svg = renderSVG.drawToString(drawing)
    font_map = _websafe_font_map(fonts)
    for name in sorted(font_map, key=len, reverse=True):
        family, weight = font_map[name]
        svg = svg.replace(
            f"font-family: {name};",
            f"font-family: {family}; font-weight: {weight};",
        )
    start = svg.find("<svg")
    if start > 0:
        svg = svg[start:]
    return svg
