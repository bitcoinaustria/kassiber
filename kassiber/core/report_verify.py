"""Self-verifying XLSX layer for the generic report export.

The normal report sheets dump Kassiber's *computed* numbers as static cells:
balance, average price, cost basis, disposals and gains arrive as opaque floats
the reader has to trust. This module appends a small set of extra sheets that
let the reader **reproduce those numbers themselves** in Excel/LibreOffice:

  * ``Acquisitions`` / ``Disposals`` — the raw journal ledger, where only the
    measured inputs (msat quantities, fiat values, proceeds, cost basis) are
    hard-typed; quantities-in-BTC and per-row gain/loss are live formulas.
  * ``Control`` — a per-asset reconciliation matrix. Every headline figure
    (holdings quantity, cost basis, average price, market value, unrealized and
    realized gain) is recomputed by a live formula over the ledger sheets and
    sat next to Kassiber's own number with an OK/DIFF check.
  * ``Verify`` — a plain-language "how to verify" sheet plus the tolerance cell
    the checks reference.

Design rules:

  * The only place formulas live is these sheets — the existing value sheets are
    untouched (they remain "what Kassiber says").
  * Every formula cell is written with its Kassiber-computed result as the
    cached ``value=`` so the file shows correct numbers immediately and a
    recalculation *proves* they reproduce.
  * Reconciliation is **per asset at profile scope**. Bitcoin accounting is
    pooled per asset across wallets (per-wallet cost basis in the main report is
    an allocation of the pooled basis), and total cost basis conserves
    regardless of the lot-selection method, so ``ending basis =
    Σ acquisition fiat_value − Σ disposal cost_basis`` is a method-independent
    identity. Per-disposal cost basis under FIFO/LIFO/HIFO/LOFO is selected by
    the tax engine and cannot be reproduced by a plain formula — the Verify
    sheet says so, and the invariants on the Control sheet cover what can be
    checked for any method.

This module deliberately holds no DB access. ``reports.py`` gathers the rows and
per-asset Kassiber aggregates and hands them in, so there is no import cycle.
"""

from __future__ import annotations

from typing import Any, Callable

# Sheets appended to the workbook, in write order. "Quarantined" is added only
# when the profile has quarantined transactions (signal, not reassurance).
VERIFY_SHEET_NAMES = ("Verify", "Acquisitions", "Disposals", "Control")

# Journal entry types that increase holdings (the Acquisitions ledger) and those
# that decrease them (the Disposals ledger). Together they partition every
# entry type the engine emits.
ADD_ENTRY_TYPES = ("acquisition", "income", "transfer_in")
SUB_ENTRY_TYPES = ("disposal", "fee", "transfer_fee", "transfer_out")

DEFAULT_FIAT_TOLERANCE = 0.01  # one cent; written to the tolerance cell
QTY_TOLERANCE = 5e-9  # ~half a sat; msat→BTC is exact integer division
# Verify-sheet layout: B2 = status banner, B3 = editable tolerance.
STATUS_CELL = (1, 1)
TOLERANCE_CELL_RC = (2, 1)
TOLERANCE_CELL = "Verify!$B$3"

_MSAT_PER_BTC = 100_000_000_000  # quantity is stored in msat


# --------------------------------------------------------------------------- #
# A1 helpers
# --------------------------------------------------------------------------- #
def _col_letter(index: int) -> str:
    """0-based column index -> spreadsheet column letter (0 -> A, 26 -> AA)."""
    letters = ""
    index += 1
    while index:
        index, remainder = divmod(index - 1, 26)
        letters = chr(65 + remainder) + letters
    return letters


def _cell(col_index: int, excel_row: int, *, abs_col: bool = False, abs_row: bool = False) -> str:
    col = ("$" if abs_col else "") + _col_letter(col_index)
    row = ("$" if abs_row else "") + str(excel_row)
    return f"{col}{row}"


def _needs_quote(sheet_name: str) -> bool:
    return not sheet_name.replace("_", "").isalnum()


def _sheet_prefix(sheet_name: str) -> str:
    if _needs_quote(sheet_name):
        return "'" + sheet_name.replace("'", "''") + "'!"
    return sheet_name + "!"


def _abs_range(sheet_name: str, col_index: int, first_row: int, last_row: int) -> str:
    """Absolute single-column range like ``'Acquisitions'!$F$3:$F$120``."""
    col = _col_letter(col_index)
    return f"{_sheet_prefix(sheet_name)}${col}${first_row}:${col}${last_row}"


# --------------------------------------------------------------------------- #
# Formats
# --------------------------------------------------------------------------- #
def _build_formats(workbook) -> dict[str, Any]:
    return {
        "title": workbook.add_format({"bold": True, "font_size": 14, "valign": "vcenter"}),
        "header": workbook.add_format(
            {"bold": True, "font_size": 11, "valign": "top", "text_wrap": True, "bg_color": "#EFEFEF"}
        ),
        "subheader": workbook.add_format({"bold": True, "font_size": 11, "valign": "top"}),
        "text": workbook.add_format({"font_size": 11, "valign": "top", "text_wrap": True}),
        "note": workbook.add_format({"font_size": 11, "valign": "top", "text_wrap": True}),
        "int": workbook.add_format({"font_size": 11, "valign": "top", "num_format": "0"}),
        "quantity": workbook.add_format({"font_size": 11, "valign": "top", "num_format": "0.00000000"}),
        "money": workbook.add_format({"font_size": 11, "valign": "top", "num_format": "#,##0.00"}),
        "money_input": workbook.add_format(
            {"font_size": 11, "valign": "top", "num_format": "#,##0.00", "bg_color": "#FFFDEB"}
        ),
        "quantity_input": workbook.add_format(
            {"font_size": 11, "valign": "top", "num_format": "0.00000000", "bg_color": "#FFFDEB"}
        ),
        "int_input": workbook.add_format(
            {"font_size": 11, "valign": "top", "num_format": "0", "bg_color": "#FFFDEB"}
        ),
        "formula_money": workbook.add_format(
            {"font_size": 11, "valign": "top", "num_format": "#,##0.00", "bg_color": "#EAF4FF"}
        ),
        "formula_quantity": workbook.add_format(
            {"font_size": 11, "valign": "top", "num_format": "0.00000000", "bg_color": "#EAF4FF"}
        ),
        "kassiber": workbook.add_format({"font_size": 11, "valign": "top", "num_format": "#,##0.00", "italic": True}),
        "kassiber_qty": workbook.add_format(
            {"font_size": 11, "valign": "top", "num_format": "0.00000000", "italic": True}
        ),
        "check": workbook.add_format({"font_size": 11, "valign": "top", "align": "center"}),
        "tolerance": workbook.add_format(
            {"font_size": 11, "valign": "top", "num_format": "#,##0.00", "bg_color": "#FFFDEB", "border": 1}
        ),
        "ok": workbook.add_format({"bg_color": "#C6EFCE", "font_color": "#006100"}),
        "diff": workbook.add_format({"bg_color": "#FFC7CE", "font_color": "#9C0006", "bold": True}),
        "status_ok": workbook.add_format(
            {"bg_color": "#C6EFCE", "font_color": "#006100", "bold": True, "font_size": 11}
        ),
        "coarse": workbook.add_format({"bg_color": "#FFEB9C", "font_color": "#9C6500"}),
        "link": workbook.add_format(
            {"font_size": 11, "valign": "top", "text_wrap": True, "font_color": "#185FA5", "underline": 1}
        ),
    }


def _write_value(worksheet, row_index: int, column_index: int, value, cell_format) -> None:
    if value is None or value == "":
        worksheet.write_blank(row_index, column_index, None, cell_format)
    elif isinstance(value, bool):
        worksheet.write_boolean(row_index, column_index, value, cell_format)
    elif isinstance(value, (int, float)):
        worksheet.write_number(row_index, column_index, float(value), cell_format)
    else:
        worksheet.write_string(row_index, column_index, str(value), cell_format)


# --------------------------------------------------------------------------- #
# Generic data-sheet writer
# --------------------------------------------------------------------------- #
def _column_width(label: str, key: str, rows: list[dict], fmt: str) -> float:
    width = len(label) + 2
    for row in rows[:50]:
        value = row.get(key, "")
        width = max(width, min(len(str(value)), 36))
    if fmt in ("quantity", "quantity_input", "formula_quantity", "kassiber_qty"):
        width = max(width, 14)
    if fmt in ("money", "money_input", "formula_money", "kassiber"):
        width = max(width, 12)
    return max(9, min(width + 1, 40))


def _write_data_sheet(
    workbook,
    formats: dict[str, Any],
    *,
    sheet_name: str,
    title: str,
    rows: list[dict],
    columns: list[dict],
) -> dict[str, Any]:
    """Write a sheet of ``rows`` described by ``columns``.

    Each column dict has ``key``, ``label``, ``fmt`` (a format name) and an
    optional ``formula`` callable ``(colmap, excel_row, row) -> (formula, cached)``.
    When ``formula`` is present the cell is a live formula; otherwise the raw
    ``row[key]`` value is written. Returns a reference describing the written
    layout (``sheet_name``, 0-based ``col_index`` per key, and the 1-based
    ``first_data_row`` / ``last_data_row``) so other sheets can build ranges.
    """
    worksheet = workbook.add_worksheet(sheet_name)
    worksheet.set_landscape()
    worksheet.fit_to_pages(1, 0)
    worksheet.set_margins(left=0.35, right=0.35, top=0.5, bottom=0.5)

    col_index = {column["key"]: index for index, column in enumerate(columns)}
    last_column = max(len(columns) - 1, 0)

    worksheet.set_row(0, 28)
    worksheet.merge_range(0, 0, 0, last_column, title, formats["title"])
    worksheet.set_row(1, 30)
    for index, column in enumerate(columns):
        worksheet.set_column(index, index, _column_width(column["label"], column["key"], rows, column["fmt"]))
        worksheet.write_string(1, index, column["label"], formats["header"])
    worksheet.freeze_panes(2, 0)

    first_data_row = 3  # 1-based Excel row of the first data row
    row_index = 2
    if rows:
        for row in rows:
            excel_row = row_index + 1
            for index, column in enumerate(columns):
                fmt = formats[column["fmt"]]
                formula = column.get("formula")
                link_url = str(row.get(column["link_key"], "")) if column.get("link_key") else ""
                if formula is not None:
                    expr, cached = formula(col_index, excel_row, row)
                    if expr is None:
                        _write_value(worksheet, row_index, index, cached, fmt)
                    else:
                        worksheet.write_formula(row_index, index, expr, fmt, cached)
                elif column.get("link_key"):
                    name = str(row.get(column["key"], ""))
                    if link_url:
                        worksheet.write_url(row_index, index, link_url, formats["link"], name)
                    else:
                        _write_value(worksheet, row_index, index, name, formats["text"])
                else:
                    _write_value(worksheet, row_index, index, row.get(column["key"], ""), fmt)
            row_index += 1
        last_data_row = row_index  # 1-based row of the last data row
    else:
        worksheet.write_string(row_index, 0, "No rows in scope.", formats["text"])
        last_data_row = row_index + 1  # keep a valid (empty) range for SUMIFS
        row_index += 1

    worksheet.autofilter(1, 0, max(row_index - 1, 2), last_column)
    return {
        "worksheet": worksheet,
        "sheet_name": sheet_name,
        "col_index": col_index,
        "first_data_row": first_data_row,
        "last_data_row": last_data_row,
    }


# --------------------------------------------------------------------------- #
# Ledger sheets (Acquisitions / Disposals)
# --------------------------------------------------------------------------- #
def _qty_from_msat(msat_key: str) -> Callable:
    def builder(colmap, excel_row, row):
        cell = _cell(colmap[msat_key], excel_row)
        return f"={cell}/{_MSAT_PER_BTC}", float(row.get("quantity", 0.0))

    return builder


def _unit_price_formula(qty_key: str, fiat_key: str) -> Callable:
    def builder(colmap, excel_row, row):
        qty_cell = _cell(colmap[qty_key], excel_row)
        fiat_cell = _cell(colmap[fiat_key], excel_row)
        qty = float(row.get("quantity", 0.0))
        fiat = float(row.get("fiat_value", 0.0))
        cached = fiat / qty if qty else 0.0
        return f"=IF({qty_cell}=0,0,{fiat_cell}/{qty_cell})", cached

    return builder


def _gain_formula(proceeds_key: str, basis_key: str) -> Callable:
    def builder(colmap, excel_row, row):
        proceeds_cell = _cell(colmap[proceeds_key], excel_row)
        basis_cell = _cell(colmap[basis_key], excel_row)
        cached = float(row.get("proceeds", 0.0)) - float(row.get("cost_basis", 0.0))
        return f"={proceeds_cell}-{basis_cell}", cached

    return builder


def _gain_check_formula(gain_key: str, kassiber_key: str) -> Callable:
    def builder(colmap, excel_row, row):
        gain_cell = _cell(colmap[gain_key], excel_row)
        kassiber_cell = _cell(colmap[kassiber_key], excel_row)
        recomputed = float(row.get("proceeds", 0.0)) - float(row.get("cost_basis", 0.0))
        cached = "OK" if abs(recomputed - float(row.get("gain_loss", 0.0))) <= DEFAULT_FIAT_TOLERANCE else "DIFF"
        expr = f'=IF(ABS({gain_cell}-{kassiber_cell})<={TOLERANCE_CELL},"OK","DIFF "&TEXT({gain_cell}-{kassiber_cell},"0.00"))'
        return expr, cached

    return builder


def _acquisitions_columns() -> list[dict]:
    return [
        {"key": "occurred_at", "label": "Occurred At", "fmt": "text"},
        {"key": "wallet", "label": "Wallet", "fmt": "text"},
        {"key": "transaction_id", "label": "Transaction ID", "fmt": "text"},
        {"key": "asset", "label": "Asset", "fmt": "text"},
        {"key": "entry_type", "label": "Type", "fmt": "text"},
        {"key": "quantity_msat", "label": "Quantity msat (input)", "fmt": "int_input"},
        {"key": "quantity", "label": "Quantity BTC (=msat/1e11)", "fmt": "formula_quantity", "formula": _qty_from_msat("quantity_msat")},
        {"key": "fiat_value", "label": "Fiat Value (input)", "fmt": "money_input"},
        {"key": "unit_price", "label": "Unit Price (=fiat/qty)", "fmt": "formula_money", "formula": _unit_price_formula("quantity", "fiat_value")},
        {"key": "gain_loss", "label": "Income Gain (input)", "fmt": "money_input"},
        {"key": "taxable", "label": "Taxable", "fmt": "int"},
        {"key": "pricing_source", "label": "Pricing Source", "fmt": "text"},
        {"key": "pricing_quality", "label": "Pricing Quality", "fmt": "text"},
        {"key": "description", "label": "Description", "fmt": "text"},
        {"key": "tags", "label": "Tags", "fmt": "text"},
    ]


def _disposals_columns() -> list[dict]:
    return [
        {"key": "occurred_at", "label": "Occurred At", "fmt": "text"},
        {"key": "wallet", "label": "Wallet", "fmt": "text"},
        {"key": "transaction_id", "label": "Transaction ID", "fmt": "text"},
        {"key": "asset", "label": "Asset", "fmt": "text"},
        {"key": "entry_type", "label": "Type", "fmt": "text"},
        {"key": "quantity_msat", "label": "Quantity msat (input)", "fmt": "int_input"},
        {"key": "quantity", "label": "Quantity BTC (=msat/1e11)", "fmt": "formula_quantity", "formula": _qty_from_msat("quantity_msat")},
        {"key": "proceeds", "label": "Proceeds (input)", "fmt": "money_input"},
        {"key": "cost_basis", "label": "Cost Basis (input)", "fmt": "money_input"},
        {"key": "gain_loss", "label": "Gain/Loss (=proceeds-basis)", "fmt": "formula_money", "formula": _gain_formula("proceeds", "cost_basis")},
        {"key": "gain_loss_kassiber", "label": "Gain/Loss (Kassiber)", "fmt": "kassiber"},
        {"key": "gain_check", "label": "Check", "fmt": "check", "formula": _gain_check_formula("gain_loss", "gain_loss_kassiber")},
        {"key": "taxable", "label": "Taxable", "fmt": "int"},
        {"key": "pricing_source", "label": "Pricing Source", "fmt": "text"},
        {"key": "pricing_quality", "label": "Pricing Quality", "fmt": "text"},
        {"key": "description", "label": "Description", "fmt": "text"},
        {"key": "tags", "label": "Tags", "fmt": "text"},
    ]


def _evidence_columns() -> list[dict]:
    return [
        {"key": "occurred_at", "label": "Occurred At", "fmt": "text"},
        {"key": "wallet", "label": "Wallet", "fmt": "text"},
        {"key": "transaction_id", "label": "Transaction ID", "fmt": "text"},
        {"key": "asset", "label": "Asset", "fmt": "text"},
        {"key": "type", "label": "Type", "fmt": "text"},
        {"key": "name", "label": "Name (link)", "fmt": "link", "link_key": "url"},
        {"key": "reference", "label": "Reference", "fmt": "text"},
    ]


def _quarantined_columns() -> list[dict]:
    return [
        {"key": "occurred_at", "label": "Occurred At", "fmt": "text"},
        {"key": "transaction_id", "label": "Transaction ID", "fmt": "text"},
        {"key": "asset", "label": "Asset", "fmt": "text"},
        {"key": "amount", "label": "Amount BTC", "fmt": "quantity"},
        {"key": "reason", "label": "Quarantine Reason", "fmt": "text"},
        {"key": "detail", "label": "Detail", "fmt": "text"},
        {"key": "description", "label": "Description", "fmt": "text"},
    ]


def _conditional_check_format(worksheet, formats, ref: dict, check_key: str) -> None:
    """Colour OK green and DIFF red across a check column's data range."""
    if check_key not in ref["col_index"]:
        return
    col = ref["col_index"][check_key]
    first = ref["first_data_row"] - 1
    last = ref["last_data_row"] - 1
    cell_range = f"{_cell(col, first + 1)}:{_cell(col, last + 1)}"
    worksheet.conditional_format(
        cell_range, {"type": "text", "criteria": "containing", "value": "DIFF", "format": formats["diff"]}
    )
    worksheet.conditional_format(
        cell_range, {"type": "text", "criteria": "containing", "value": "OK", "format": formats["ok"]}
    )


def _conditional_quality_format(worksheet, formats, ref: dict, key: str) -> None:
    """Highlight estimated (coarse_fallback / missing) pricing-quality cells."""
    if key not in ref["col_index"]:
        return
    col = ref["col_index"][key]
    cell_range = f"{_cell(col, ref['first_data_row'])}:{_cell(col, ref['last_data_row'])}"
    for token in ("coarse", "missing"):
        worksheet.conditional_format(
            cell_range,
            {"type": "text", "criteria": "containing", "value": token, "format": formats["coarse"]},
        )


# --------------------------------------------------------------------------- #
# Control reconciliation sheet
# --------------------------------------------------------------------------- #
def _sumifs_minus(add_ref, sub_ref, value_key, asset_cell) -> str:
    # Earn/income coins are emitted by the engine TWICE: as an `acquisition`
    # lot (which enters holdings) and as an `income` line (the taxable
    # recognition). The lot already carries the quantity and basis, so the
    # holdings add-side counts only acquisition + transfer_in (explicit equality
    # criteria — Apple Numbers drops the "<>income" not-equal condition string).
    add_vals = _abs_range(add_ref["sheet_name"], add_ref["col_index"][value_key], add_ref["first_data_row"], add_ref["last_data_row"])
    add_assets = _abs_range(add_ref["sheet_name"], add_ref["col_index"]["asset"], add_ref["first_data_row"], add_ref["last_data_row"])
    add_types = _abs_range(add_ref["sheet_name"], add_ref["col_index"]["entry_type"], add_ref["first_data_row"], add_ref["last_data_row"])
    sub_vals = _abs_range(sub_ref["sheet_name"], sub_ref["col_index"][value_key], sub_ref["first_data_row"], sub_ref["last_data_row"])
    sub_assets = _abs_range(sub_ref["sheet_name"], sub_ref["col_index"]["asset"], sub_ref["first_data_row"], sub_ref["last_data_row"])
    return (
        f'SUMIFS({add_vals},{add_assets},{asset_cell},{add_types},"acquisition")'
        f'+SUMIFS({add_vals},{add_assets},{asset_cell},{add_types},"transfer_in")'
        f"-SUMIFS({sub_vals},{sub_assets},{asset_cell})"
    )


def _realized_sumifs(add_ref, sub_ref, asset_cell) -> str:
    parts = []
    for ref in (sub_ref, add_ref):
        vals = _abs_range(ref["sheet_name"], ref["col_index"]["gain_loss"], ref["first_data_row"], ref["last_data_row"])
        assets = _abs_range(ref["sheet_name"], ref["col_index"]["asset"], ref["first_data_row"], ref["last_data_row"])
        taxable = _abs_range(ref["sheet_name"], ref["col_index"]["taxable"], ref["first_data_row"], ref["last_data_row"])
        parts.append(f"SUMIFS({vals},{assets},{asset_cell},{taxable},1)")
    return "+".join(parts)


def _control_columns(add_ref, sub_ref) -> list[dict]:
    """Per-asset reconciliation columns.

    Each metric is a triple: a live recompute formula, Kassiber's own number,
    and an OK/DIFF check comparing them within the tolerance cell.
    """

    def asset_cell(colmap, excel_row):
        return _cell(colmap["asset"], excel_row, abs_col=True)

    def qty_recompute(colmap, excel_row, row):
        expr = "=" + _sumifs_minus(add_ref, sub_ref, "quantity", asset_cell(colmap, excel_row))
        return expr, float(row.get("quantity", 0.0))

    def basis_recompute(colmap, excel_row, row):
        # ending basis = Σ acquisition fiat_value − Σ disposal cost_basis.
        # Exclude `income` rows on the add side: the earned coins' basis is
        # already carried by their paired `acquisition` lot (see _sumifs_minus).
        add_vals = _abs_range(add_ref["sheet_name"], add_ref["col_index"]["fiat_value"], add_ref["first_data_row"], add_ref["last_data_row"])
        add_assets = _abs_range(add_ref["sheet_name"], add_ref["col_index"]["asset"], add_ref["first_data_row"], add_ref["last_data_row"])
        add_types = _abs_range(add_ref["sheet_name"], add_ref["col_index"]["entry_type"], add_ref["first_data_row"], add_ref["last_data_row"])
        sub_vals = _abs_range(sub_ref["sheet_name"], sub_ref["col_index"]["cost_basis"], sub_ref["first_data_row"], sub_ref["last_data_row"])
        sub_assets = _abs_range(sub_ref["sheet_name"], sub_ref["col_index"]["asset"], sub_ref["first_data_row"], sub_ref["last_data_row"])
        cell = asset_cell(colmap, excel_row)
        expr = (
            f'=SUMIFS({add_vals},{add_assets},{cell},{add_types},"acquisition")'
            f'+SUMIFS({add_vals},{add_assets},{cell},{add_types},"transfer_in")'
            f"-SUMIFS({sub_vals},{sub_assets},{cell})"
        )
        return expr, float(row.get("cost_basis", 0.0))

    def avg_cost_recompute(colmap, excel_row, row):
        qty_cell = _cell(colmap["holdings_qty"], excel_row)
        basis_cell = _cell(colmap["cost_basis"], excel_row)
        cached = float(row["cost_basis"]) / float(row["quantity"]) if row.get("quantity") else 0.0
        return f"=IF({qty_cell}=0,0,{basis_cell}/{qty_cell})", cached

    def market_value_recompute(colmap, excel_row, row):
        qty_cell = _cell(colmap["holdings_qty"], excel_row)
        rate_cell = _cell(colmap["rate"], excel_row)
        return f"={qty_cell}*{rate_cell}", float(row.get("market_value", 0.0))

    def unrealized_recompute(colmap, excel_row, row):
        mv_cell = _cell(colmap["market_value"], excel_row)
        basis_cell = _cell(colmap["cost_basis"], excel_row)
        return f"={mv_cell}-{basis_cell}", float(row.get("unrealized_pnl", 0.0))

    def realized_recompute(colmap, excel_row, row):
        expr = "=" + _realized_sumifs(add_ref, sub_ref, asset_cell(colmap, excel_row))
        return expr, float(row.get("realized_gain", 0.0))

    def _avg_value(row):
        return float(row["cost_basis"]) / float(row["quantity"]) if row.get("quantity") else 0.0

    def check(recompute_key, kassiber_key, recompute_fn, tol):
        # The cached OK/DIFF is computed from the row so it reflects reality
        # before any recalc: a desynced recompute vs Kassiber value shows DIFF.
        def builder(colmap, excel_row, row):
            a = _cell(colmap[recompute_key], excel_row)
            b = _cell(colmap[kassiber_key], excel_row)
            recomputed = float(recompute_fn(row) or 0.0)
            kassiber = float(row.get(kassiber_key, 0.0) or 0.0)
            cached = "OK" if abs(recomputed - kassiber) <= tol else f"DIFF {recomputed - kassiber:.8f}"
            return (
                f'=IF(ABS({a}-{b})<={TOLERANCE_CELL},"OK","DIFF "&TEXT({a}-{b},"0.00000000"))',
                cached,
            )

        return builder

    return [
        {"key": "asset", "label": "Asset", "fmt": "subheader"},
        {"key": "rate", "label": "Market Rate (input)", "fmt": "money_input"},
        {"key": "rate_source", "label": "Rate Source", "fmt": "text"},
        {"key": "rate_as_of", "label": "Rate As Of", "fmt": "text"},
        {"key": "holdings_qty", "label": "Holdings BTC (recompute)", "fmt": "formula_quantity", "formula": qty_recompute},
        {"key": "holdings_qty_kassiber", "label": "Holdings BTC (Kassiber)", "fmt": "kassiber_qty"},
        {"key": "qty_check", "label": "Balance Check", "fmt": "check", "formula": check("holdings_qty", "holdings_qty_kassiber", lambda r: r.get("quantity", 0.0), QTY_TOLERANCE)},
        {"key": "cost_basis", "label": "Cost Basis (recompute)", "fmt": "formula_money", "formula": basis_recompute},
        {"key": "cost_basis_kassiber", "label": "Cost Basis (Kassiber)", "fmt": "kassiber"},
        {"key": "basis_check", "label": "Basis Check", "fmt": "check", "formula": check("cost_basis", "cost_basis_kassiber", lambda r: r.get("cost_basis", 0.0), DEFAULT_FIAT_TOLERANCE)},
        {"key": "avg_cost", "label": "Avg Price (recompute)", "fmt": "formula_money", "formula": avg_cost_recompute},
        {"key": "avg_cost_kassiber", "label": "Avg Price (Kassiber)", "fmt": "kassiber"},
        {"key": "avg_check", "label": "Avg Price Check", "fmt": "check", "formula": check("avg_cost", "avg_cost_kassiber", _avg_value, DEFAULT_FIAT_TOLERANCE)},
        {"key": "market_value", "label": "Market Value (recompute)", "fmt": "formula_money", "formula": market_value_recompute},
        {"key": "market_value_kassiber", "label": "Market Value (Kassiber)", "fmt": "kassiber"},
        {"key": "mv_check", "label": "Market Check", "fmt": "check", "formula": check("market_value", "market_value_kassiber", lambda r: r.get("market_value", 0.0), DEFAULT_FIAT_TOLERANCE)},
        {"key": "unrealized_pnl", "label": "Unrealized (recompute)", "fmt": "formula_money", "formula": unrealized_recompute},
        {"key": "unrealized_kassiber", "label": "Unrealized (Kassiber)", "fmt": "kassiber"},
        {"key": "unrealized_check", "label": "Unrealized Check", "fmt": "check", "formula": check("unrealized_pnl", "unrealized_kassiber", lambda r: r.get("unrealized_pnl", 0.0), DEFAULT_FIAT_TOLERANCE)},
        {"key": "realized_gain", "label": "Realized Gain (recompute)", "fmt": "formula_money", "formula": realized_recompute},
        {"key": "realized_gain_kassiber", "label": "Realized Gain (Kassiber)", "fmt": "kassiber"},
        {"key": "realized_check", "label": "Realized Check", "fmt": "check", "formula": check("realized_gain", "realized_gain_kassiber", lambda r: r.get("realized_gain", 0.0), DEFAULT_FIAT_TOLERANCE)},
    ]


# --------------------------------------------------------------------------- #
# Verify (README) sheet
# --------------------------------------------------------------------------- #
def _verify_readme_rows(
    *, gains_algorithm: str | None, tax_country: str | None, fiat_currency: str, wallet_scope_label: str | None
) -> list[tuple[str, str]]:
    method = str(gains_algorithm or "").upper() or "(unset)"
    tax_country = str(tax_country or "").lower()
    is_moving_average = tax_country == "at"
    method_line = (
        f"Active lot-selection method: {method}."
        + (
            "  Austrian profile — disposals use the moving-average cost base."
            if is_moving_average
            else ""
        )
    )
    lot_caveat = (
        "Per-disposal cost basis under FIFO / LIFO / HIFO / LOFO is chosen by the "
        "tax engine and CANNOT be re-derived by a plain spreadsheet formula — a "
        "spreadsheet cannot re-run lot selection. What the Control sheet verifies "
        "instead are the identities that hold for ANY method: ending holdings, "
        "ending cost basis (Σ acquisition fiat value − Σ disposal cost basis), "
        "average price, market value, unrealized and realized gain. Each disposal "
        "row still checks gain = proceeds − cost basis directly."
    )
    rows: list[tuple[str, str]] = [
        ("What this proves", "subheader"),
        (
            "The Acquisitions and Disposals sheets hold the raw journal ledger. Only "
            "the highlighted input cells (msat quantities, fiat values, proceeds, cost "
            "basis) are hard numbers. Everything else — quantities in BTC, per-row "
            "gain/loss, and every figure on the Control sheet — is a live formula. The "
            "Control sheet recomputes each headline figure from those inputs and shows "
            "it next to Kassiber's own number with an OK / DIFF check.",
            "text",
        ),
        ("Formula legend", "subheader"),
        (
            "Quantity BTC = Quantity msat ÷ 100,000,000,000   "
            "(1 BTC = 100,000,000 sat = 100,000,000,000 msat; 1 sat = msat ÷ 1000)",
            "text",
        ),
        ("Gain / Loss = Proceeds − Cost Basis", "text"),
        ("Holdings BTC = Σ Acquisitions quantity (excluding income lines) − Σ Disposals quantity (per asset)", "text"),
        ("Cost Basis = Σ Acquisition fiat value (excluding income lines) − Σ Disposal cost basis (per asset)", "text"),
        ("Average Price = Cost Basis ÷ Holdings BTC", "text"),
        ("Market Value = Holdings BTC × Market Rate", "text"),
        ("Unrealized = Market Value − Cost Basis", "text"),
        ("Realized Gain = Σ taxable Disposal gains + Σ taxable income gains (per asset)", "text"),
        (
            "Income/earn coins appear twice in the ledger: as an 'acquisition' lot (the coins "
            "entering holdings) and as an 'income' line (the taxable recognition). The holdings "
            "formulas above exclude the 'income' rows so the earned coins are counted once; the "
            "realized-gain formula includes them. Filter the Type column to see this.",
            "text",
        ),
        ("Lot-selection method", "subheader"),
        (method_line, "text"),
        (lot_caveat, "text"),
        ("Scope", "subheader"),
        (
            "Verification is computed per ASSET across the WHOLE profile. Bitcoin "
            "accounting is pooled per asset across all wallets — per-wallet cost basis "
            "in the main report is an allocation of that pooled basis, so it is not "
            "reconcilable row-by-row. Transfers between your own wallets net to zero at "
            "profile scope (only the network transfer fee reduces holdings).",
            "text",
        ),
    ]
    if wallet_scope_label:
        rows.append(
            (
                f"Note: the value sheets in this workbook are filtered to wallet "
                f"'{wallet_scope_label}', but the verification sheets below cover the "
                f"entire profile for the reason above.",
                "text",
            )
        )
    rows.extend(
        [
            ("Recalculation", "subheader"),
            (
                "This file ships with cached results, so it already shows the correct "
                "numbers. The formulas are real. To force a recompute and prove they "
                "reproduce:",
                "text",
            ),
            (
                "  • LibreOffice Calc: press Ctrl+Shift+F9 (Data ▸ Calculate ▸ "
                "Recalculate). For automatic recalc on open, enable Tools ▸ Options ▸ "
                "LibreOffice Calc ▸ Formula ▸ Recalculation on File Load ▸ Always "
                "recalculate.",
                "text",
            ),
            (
                "  • Excel: ensure Formulas ▸ Calculation Options ▸ Automatic. No "
                "iterative calculation is needed — no formula references its own cell: "
                "each ledger row uses only same-row cells, and the Control sheet "
                "aggregates the ledger sheets with SUMIFS.",
                "text",
            ),
            (
                "Tip: set the tolerance in B3 to 0, recalculate, and confirm every "
                "check still reads OK.",
                "text",
            ),
            ("Pricing provenance", "subheader"),
            (
                "The Acquisitions and Disposals sheets carry a Pricing Source and "
                "Pricing Quality column for every priced row; rows marked "
                "'coarse_fallback' or 'missing' used an estimated price, not an exact "
                "quote. The Control sheet shows, per asset, which market rate valued "
                "your holdings, its source, and when it was captured.",
                "text",
            ),
            ("Notes, tags & evidence", "subheader"),
            (
                "Each Acquisitions/Disposals row shows its description and tags. The "
                "main report's Transactions sheet is the full per-transaction record: "
                "description, note, counterparty, tags, and an Attachments column "
                "where a linked URL is shown as a clickable link behind its name "
                "(multiple links are listed one per line). The Evidence sheet lists "
                "every attachment as its own row with a clickable link. Match a ledger "
                "row to its evidence by the Transaction ID.",
                "text",
            ),
            ("Missing rows", "subheader"),
            (
                "Excluded and Austrian neu_swap rows are intentionally absent (they are "
                "not taxable disposals). The 'Taxable' column marks which ledger rows "
                "feed the realized-gain check. Transactions Kassiber could not classify "
                "are listed on the Quarantined sheet (when present) and are not in any "
                "figure above.",
                "text",
            ),
        ]
    )
    return rows


def _write_verify_readme(
    workbook, formats, *, gains_algorithm, tax_country, fiat_currency, wallet_scope_label, run_metadata
):
    worksheet = workbook.add_worksheet("Verify")
    worksheet.set_column(0, 0, 30)
    worksheet.set_column(1, 1, 74)
    worksheet.set_margins(left=0.4, right=0.4, top=0.5, bottom=0.5)

    meta = run_metadata or {}
    method = str(gains_algorithm or "").upper() or "(unset)"
    if str(tax_country or "").lower() == "at":
        method += " (Austrian moving-average cost base)"

    # Header block (A = label, B = value). B2 is the status banner (written by
    # the caller after the check sheets exist); B3 is the editable tolerance.
    worksheet.set_row(0, 26)
    worksheet.write_string(0, 0, "How to verify this report", formats["title"])
    worksheet.write_string(STATUS_CELL[0], 0, "Verification status", formats["subheader"])
    worksheet.write_string(TOLERANCE_CELL_RC[0], 0, "Check tolerance (edit to retighten all checks)", formats["subheader"])
    worksheet.write_number(TOLERANCE_CELL_RC[0], TOLERANCE_CELL_RC[1], DEFAULT_FIAT_TOLERANCE, formats["tolerance"])

    meta_rows = [
        ("Generated at", meta.get("generated_at", "")),
        ("Kassiber version", meta.get("kassiber_version", "")),
        ("Lot-selection method", method),
        ("Fiat currency", fiat_currency),
        ("Tax country", str(tax_country or "")),
        ("Journals last processed", meta.get("last_processed_at", "")),
        ("Processed tx count", meta.get("processed_tx_count", "")),
        ("Wallet scope", meta.get("wallet_scope", "All wallets")),
    ]
    row_index = TOLERANCE_CELL_RC[0] + 1
    for label, value in meta_rows:
        worksheet.write_string(row_index, 0, label, formats["subheader"])
        _write_value(worksheet, row_index, 1, value, formats["text"])
        row_index += 1

    row_index += 1  # blank spacer
    for text, kind in _verify_readme_rows(
        gains_algorithm=gains_algorithm,
        tax_country=tax_country,
        fiat_currency=fiat_currency,
        wallet_scope_label=wallet_scope_label,
    ):
        worksheet.set_row(row_index, 24 if kind == "subheader" else 30)
        if text:
            worksheet.merge_range(row_index, 0, row_index, 1, text, formats.get(kind, formats["text"]))
        else:
            worksheet.write_blank(row_index, 0, None, formats["text"])
        row_index += 1
    return worksheet


def _write_status_banner(verify_ws, formats, control_ref, sub_ref):
    """Write the workbook-level OK/DIFF banner into the Verify sheet's B2.

    Counts non-OK results across every Control check column plus the Disposals
    gain check, so a reviewer gets a single one-glance verdict. Uses
    ``COUNTA - COUNTIF(..,"OK")`` rather than a ``"DIFF*"`` wildcard, which
    Apple Numbers drops on import.
    """
    counts = []
    check_keys = ("qty_check", "basis_check", "avg_check", "mv_check", "unrealized_check", "realized_check")
    for check_key in check_keys:
        col = control_ref["col_index"][check_key]
        rng = _abs_range(control_ref["sheet_name"], col, control_ref["first_data_row"], control_ref["last_data_row"])
        counts.append(f'(COUNTA({rng})-COUNTIF({rng},"OK"))')
    gain_col = sub_ref["col_index"]["gain_check"]
    gain_rng = _abs_range(sub_ref["sheet_name"], gain_col, sub_ref["first_data_row"], sub_ref["last_data_row"])
    counts.append(f'(COUNTA({gain_rng})-COUNTIF({gain_rng},"OK"))')
    total = "+".join(counts)
    expr = f'=IF(({total})=0,"ALL CHECKS OK","MISMATCH — see the highlighted check columns")'
    verify_ws.write_formula(STATUS_CELL[0], STATUS_CELL[1], expr, formats["status_ok"], "ALL CHECKS OK")
    verify_ws.conditional_format(
        f"{_cell(STATUS_CELL[1], STATUS_CELL[0] + 1)}",
        {"type": "text", "criteria": "containing", "value": "MISMATCH", "format": formats["diff"]},
    )


# --------------------------------------------------------------------------- #
# Public entry point
# --------------------------------------------------------------------------- #
def augment_workbook(
    workbook,
    *,
    gains_algorithm: str | None,
    tax_country: str | None,
    fiat_currency: str,
    wallet_scope_label: str | None,
    run_metadata: dict | None = None,
    acquisitions: list[dict],
    disposals: list[dict],
    asset_rows: list[dict],
    quarantines: list[dict] | None = None,
    attachments: list[dict] | None = None,
) -> list[str]:
    """Append the verification sheets to an open xlsxwriter workbook.

    ``acquisitions`` / ``disposals`` are journal ledger rows (already partitioned
    by add/sub entry type). ``asset_rows`` are the per-asset Kassiber aggregates
    used as the reconciliation targets. ``quarantines`` (if any) get their own
    sheet. Returns the list of sheet names added.
    """
    formats = _build_formats(workbook)
    verify_ws = _write_verify_readme(
        workbook,
        formats,
        gains_algorithm=gains_algorithm,
        tax_country=tax_country,
        fiat_currency=fiat_currency,
        wallet_scope_label=wallet_scope_label,
        run_metadata=run_metadata,
    )

    add_ref = _write_data_sheet(
        workbook,
        formats,
        sheet_name="Acquisitions",
        title="Acquisitions ledger (inputs add to holdings)",
        rows=acquisitions,
        columns=_acquisitions_columns(),
    )
    _conditional_quality_format(add_ref["worksheet"], formats, add_ref, "pricing_quality")
    sub_ref = _write_data_sheet(
        workbook,
        formats,
        sheet_name="Disposals",
        title="Disposals ledger (inputs reduce holdings)",
        rows=disposals,
        columns=_disposals_columns(),
    )
    _conditional_check_format(sub_ref["worksheet"], formats, sub_ref, "gain_check")
    _conditional_quality_format(sub_ref["worksheet"], formats, sub_ref, "pricing_quality")

    control_ref = _write_data_sheet(
        workbook,
        formats,
        sheet_name="Control",
        title="Control — recompute every figure and compare to Kassiber",
        rows=asset_rows,
        columns=_control_columns(add_ref, sub_ref),
    )
    for check_key in ("qty_check", "basis_check", "avg_check", "mv_check", "unrealized_check", "realized_check"):
        _conditional_check_format(control_ref["worksheet"], formats, control_ref, check_key)

    _write_status_banner(verify_ws, formats, control_ref, sub_ref)

    sheets = list(VERIFY_SHEET_NAMES)
    if attachments:
        _write_data_sheet(
            workbook,
            formats,
            sheet_name="Evidence",
            title="Linked evidence — one clickable link per attachment",
            rows=attachments,
            columns=_evidence_columns(),
        )
        sheets.append("Evidence")
    if quarantines:
        _write_data_sheet(
            workbook,
            formats,
            sheet_name="Quarantined",
            title="Quarantined transactions (excluded from every figure above)",
            rows=quarantines,
            columns=_quarantined_columns(),
        )
        sheets.append("Quarantined")
    return sheets
