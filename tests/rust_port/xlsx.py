"""Strict workbook oracle and render-boundary recorder (test code only).

The recorder forwards every call to the real Python XlsxWriter while collecting
the reviewed renderer input. It refuses unknown operations/styles rather than
quietly reducing a workbook to its visible values. No product routing uses it.
"""
from __future__ import annotations

from datetime import datetime, timezone
import json
from pathlib import Path
import warnings
import zipfile
from xml.etree import ElementTree as ET

import openpyxl
from openpyxl.utils import range_boundaries
import xlsxwriter

STAMP = "2026-01-02T03:04:05Z"
CREATED = datetime(2026, 1, 2, 3, 4, 5, tzinfo=timezone.utc)
REAL_WORKBOOK = xlsxwriter.Workbook


def _xml(node):
    if node is None:
        return None
    return [node.tag, dict(sorted(node.attrib.items())), node.text, [_xml(c) for c in node]]


def canonical(path):
    """Content, typed cached results and meaningful layout, never ZIP/XML order.

    Style ids and shared-string indices are implementation details. All actual
    style properties, blank cells, differential formats, links and defined names
    are compared; cached formula results are read independently of expressions.
    """
    with zipfile.ZipFile(path) as archive:
        assert archive.testzip() is None, "corrupt workbook ZIP"
        assert len(archive.namelist()) == len(set(archive.namelist())), "duplicate ZIP member"
        for name in archive.namelist():
            if name.endswith((".xml", ".rels")):
                ET.fromstring(archive.read(name))
        root = ET.fromstring(archive.read("xl/workbook.xml"))
        ns = {"m": "http://schemas.openxmlformats.org/spreadsheetml/2006/main"}
        names = [_xml(n) for n in root.findall("m:definedNames/m:definedName", ns)]
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        book = openpyxl.load_workbook(path, data_only=False)
        cached = openpyxl.load_workbook(path, data_only=True)
    assert not caught, [str(w.message) for w in caught]
    result = {"properties": _xml(book.properties.to_tree()), "names": sorted(names, key=str), "sheets": []}
    for sheet in book:
        cells = []
        for key, cell in sorted(sheet._cells.items()):
            cached_cell = cached[sheet.title].cell(*key)
            style = dict(number=cell.number_format,
                         font=_xml(cell.font.to_tree()), fill=_xml(cell.fill.to_tree()),
                         border=_xml(cell.border.to_tree()), alignment=_xml(cell.alignment.to_tree()),
                         protection=_xml(cell.protection.to_tree()))
            link = cell.hyperlink
            cells.append(dict(at=cell.coordinate, value=cell.value, type=cell.data_type,
                              cached=[cached_cell.value, cached_cell.data_type] if cell.data_type == "f" else None,
                              style=style, link=None if link is None else [link.target, link.location, link.tooltip]))
        columns = {}
        for column in sheet.column_dimensions.values():
            # Equivalent consecutive widths can be grouped differently.
            for index in range(column.min, column.max + 1):
                columns[index] = [column.width, column.hidden, column.outlineLevel]
        conditional = []
        for entry, rules in sheet.conditional_formatting._cf_rules.items():
            encoded = []
            for rule in rules:
                node = rule.to_tree()
                node.attrib.pop("dxfId", None)
                encoded.append(_xml(node) + [_xml(rule.dxf.to_tree()) if rule.dxf else None])
            conditional.append([str(entry.sqref), encoded])
        result["sheets"].append(dict(name=sheet.title, cells=cells, merges=sorted(str(r) for r in sheet.merged_cells),
            freeze=sheet.freeze_panes, columns=columns,
            rows={k: [v.height, v.hidden, v.outlineLevel] for k, v in sheet.row_dimensions.items()},
            autofilter=sheet.auto_filter.ref, conditional=conditional,
            margins=_xml(sheet.page_margins.to_tree()), setup=_xml(sheet.page_setup.to_tree())))
    book.close()
    cached.close()
    return result


def first_difference(left, right, at="workbook"):
    if type(left) is not type(right):
        return f"{at}: {left!r} != {right!r}"
    if isinstance(left, dict):
        if left.keys() != right.keys():
            return f"{at} keys: {left.keys()} != {right.keys()}"
        for key in left:
            if difference := first_difference(left[key], right[key], f"{at}.{key}"):
                return difference
    elif isinstance(left, (tuple, list)):
        if len(left) != len(right):
            return f"{at} length: {len(left)} != {len(right)}"
        for index, (a, b) in enumerate(zip(left, right)):
            if difference := first_difference(a, b, f"{at}[{index}]"):
                return difference
    elif left != right:
        return f"{at}: {left!r} != {right!r}"
    return None


class RecordingWorkbook:
    def __init__(self, path, options=None):
        self.real = REAL_WORKBOOK(path, options)
        self.document = dict(title="", subject="", author="", comments="", created=STAMP,
                             formats=[{}], sheets=[], names=[])
        self.formats = {None: 0}
        self.real.set_properties({"created": CREATED})

    def add_format(self, properties):
        allowed = {"bold", "italic", "font_size", "font_color", "bg_color", "num_format",
                   "valign", "align", "text_wrap", "underline", "border", "bottom"}
        assert not properties.keys() - allowed, properties
        real = self.real.add_format(properties)
        self.formats[real] = len(self.document["formats"])
        self.document["formats"].append(properties)
        return real

    def add_worksheet(self, name):
        spec = dict(name=name, operations=[])
        self.document["sheets"].append(spec)
        return RecordingSheet(self, self.real.add_worksheet(name), spec["operations"])

    def set_properties(self, properties):
        assert not properties.keys() - {"title", "subject", "author", "comments", "created"}, properties
        self.document.update({k: v for k, v in properties.items() if k != "created"})
        self.real.set_properties({**properties, "created": CREATED})

    def set_calc_mode(self, mode):
        assert mode == "auto", mode
        self.real.set_calc_mode(mode)

    def define_name(self, name, formula):
        self.document["names"].append([name, formula])
        self.real.define_name(name, formula)

    def close(self):
        self.real.close()


class RecordingSheet:
    def __init__(self, workbook, real, operations):
        self.book, self.real, self.operations = workbook, real, operations

    def add(self, op, **values):
        self.operations.append(dict(op=op, **values))

    def write(self, row, col, value, style=None):
        if value is None or value == "":
            return self.write_blank(row, col, None, style)
        if isinstance(value, bool):
            return self.write_boolean(row, col, value, style)
        if isinstance(value, (float, int)):
            return self.write_number(row, col, value, style)
        assert not str(value).startswith(("=", "http:", "https:", "mailto:", "internal:", "external:")), value
        return self.write_string(row, col, str(value), style)

    def write_string(self, row, col, value, style=None):
        self.add("cell", row=row, col=col, style=self.book.formats[style], value=dict(kind="string", value=value))
        return self.real.write_string(row, col, value, style)

    def write_blank(self, row, col, value, style=None):
        assert value is None
        # Unformatted blanks are omitted by Python.
        if style is not None:
            self.add("cell", row=row, col=col, style=self.book.formats[style], value=dict(kind="blank"))
        return self.real.write_blank(row, col, value, style)

    def write_boolean(self, row, col, value, style=None):
        self.add("cell", row=row, col=col, style=self.book.formats[style], value=dict(kind="boolean", value=bool(value)))
        return self.real.write_boolean(row, col, value, style)

    def write_number(self, row, col, value, style=None):
        self.add("cell", row=row, col=col, style=self.book.formats[style], value=dict(kind="number", value=value))
        return self.real.write_number(row, col, value, style)

    def write_formula(self, row, col, expression, style=None, value=0):
        # Unlike ordinary numeric cells, Python writes formula caches using
        # str(value), retaining the full round-trippable float representation.
        result = str(value)
        self.add("cell", row=row, col=col, style=self.book.formats[style],
                 value=dict(kind="formula", value=dict(expression=expression, result=result)))
        return self.real.write_formula(row, col, expression, style, value)

    def write_url(self, row, col, url, style=None, string=None):
        self.add("cell", row=row, col=col, style=self.book.formats[style],
                 value=dict(kind="link", value=dict(url=url, text=string or url)))
        return self.real.write_url(row, col, url, style, string)

    def merge_range(self, first_row, first_col, last_row, last_col, text, style):
        self.add("merge", first_row=first_row, first_col=first_col, last_row=last_row, last_col=last_col,
                 text=text, style=self.book.formats[style])
        # Some one-column Python sheets attempt a one-cell merge. Suppress that
        # write-time warning; openpyxl load warnings are never suppressed.
        with warnings.catch_warnings():
            warnings.filterwarnings("ignore", message="Can't merge single cell")
            return self.real.merge_range(first_row, first_col, last_row, last_col, text, style)

    def set_column(self, first, last, width):
        self.add("column", first=first, last=last, width=width)
        return self.real.set_column(first, last, width)

    def set_row(self, row, height):
        self.add("row", row=row, height=height)
        return self.real.set_row(row, height)

    def freeze_panes(self, row, col):
        self.add("freeze", row=row, col=col)
        return self.real.freeze_panes(row, col)

    def autofilter(self, first_row, first_col, last_row, last_col):
        self.add("autofilter", first_row=first_row, first_col=first_col, last_row=last_row, last_col=last_col)
        return self.real.autofilter(first_row, first_col, last_row, last_col)

    def set_landscape(self):
        self.add("landscape")
        return self.real.set_landscape()

    def fit_to_pages(self, width, height):
        self.add("fit", width=width, height=height)
        return self.real.fit_to_pages(width, height)

    def set_margins(self, left, right, top, bottom):
        self.add("margins", left=left, right=right, top=top, bottom=bottom)
        return self.real.set_margins(left=left, right=right, top=top, bottom=bottom)

    def conditional_format(self, cell_range, options):
        assert set(options) == {"type", "criteria", "value", "format"}, options
        assert options["type"] == "text" and options["criteria"] == "containing", options
        a, b, c, d = range_boundaries(cell_range)
        self.add("contains", first_row=b-1, first_col=a-1, last_row=d-1, last_col=c-1,
                 text=options["value"], style=self.book.formats[options["format"]])
        return self.real.conditional_format(cell_range, options)


def design_content(path, native):
    """A workbook in the shared report design (docs/reference/report-design.md).

    Its layout differs from Python's workbook by design, so content is not
    compared cell by cell: both must load cleanly, and the native one opens on
    the design's summary sheet. Data parity is pinned by the model tests.
    """
    canonical(path)
    if native:
        book = openpyxl.load_workbook(path, read_only=True)
        assert book.sheetnames[0] == "summary", book.sheetnames
    return "<workbook>"


def design_wire(raw, *keys):
    """Response bytes with the envelope keys the design changes masked."""
    payload = json.loads(raw)
    for container in (payload.get("data"), payload.get("ok")):
        if isinstance(container, dict):
            for key in keys:
                if key in container:
                    container[key] = "<design>"
    return json.dumps(payload, sort_keys=True).encode()
