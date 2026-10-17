"""What an export would write, without writing it.

The desktop Export dialog shows the real column headers, the first rows, the
row count and the warnings before the user exports. Every preview calls the
same builder as its export, so a preview cannot describe a file that differs
from the one written, and it fails closed the same way (stale journals,
failed sources, quarantined transactions in scope raise the export's error).

The preview is a display read: it never writes, fetches, or syncs. Values are
returned as display strings. BIP 329 key material (xpub/pubkey references and
key origins) is shortened, because a display read must not expose it.
"""

from __future__ import annotations

from decimal import Decimal
from typing import Any, Iterable, Sequence

from ..errors import AppError
from . import metadata as core_metadata
from . import transaction_formats
from . import year_handoff
from .reports import (
    GENERIC_REPORT_INT_FIELDS,
    GENERIC_REPORT_MONEY_FIELDS,
    GENERIC_REPORT_QUANTITY_FIELDS,
    TRANSACTIONS_EXPORT_HEADERS,
    ReportHooks,
    _report_column_label,
    _row_get,
    _transactions_export_context,
)

PREVIEW_ROWS = 8
EXPORTS = ("transactions", "year_handoff", "labels")
TRANSACTION_FORMATS = ("xlsx", "csv", *transaction_formats.PLATFORM_FORMATS)
_KEY_MATERIAL_TYPES = {"xpub", "pubkey"}


def _text(value: Any, number_format: str = "text") -> str:
    if value is None or value == "":
        return ""
    if isinstance(value, bool):
        return "TRUE" if value else "FALSE"
    if isinstance(value, (int, float, Decimal)) and number_format != "text":
        number = Decimal(str(value))
        if number_format == "money":
            return f"{number:,.2f}"
        if number_format == "int":
            return f"{number:,.0f}"
        if number_format == "sats":
            return f"{number.normalize():,f}"
        return f"{number:.8f}"
    if isinstance(value, Decimal):
        return format(value.normalize(), "f")
    return str(value)


def _sheet(name: str, columns: Sequence[str], rows: Iterable[Sequence[str]], total: int) -> dict[str, Any]:
    return {
        "name": name,
        "columns": list(columns),
        "rows": [list(row) for row in rows][:PREVIEW_ROWS],
        "total": total,
    }


def _transaction_number_format(header: str) -> str:
    if header in GENERIC_REPORT_INT_FIELDS:
        return "int"
    if header in GENERIC_REPORT_MONEY_FIELDS:
        return "money"
    if header in GENERIC_REPORT_QUANTITY_FIELDS:
        return "quantity"
    return "text"


def _preview_transactions(conn, hooks: ReportHooks, export_format: str, wallet_refs):
    if export_format in transaction_formats.PLATFORM_FORMATS:
        target = transaction_formats.platform_format(export_format)
        built = transaction_formats.build_platform_rows(conn, None, None, hooks, wallet_refs=wallet_refs)
        rows = [target.build_row(row) for row in built["rows"]]
        return {
            "file_name": f"{target.filename_stem}.csv",
            "row_count": len(rows),
            "sheets": [
                _sheet(
                    target.label,
                    target.headers,
                    ([row[header] for header in target.headers] for row in rows[:PREVIEW_ROWS]),
                    len(rows),
                )
            ],
            "warnings": built["warnings"],
        }
    context = _transactions_export_context(conn, None, None, hooks, wallet_refs=wallet_refs)
    rows = context["spec"]["rows"]
    return {
        "file_name": f"kassiber-transactions.{export_format}",
        "row_count": len(rows),
        "sheets": [
            _sheet(
                "Transactions",
                [_report_column_label(header) for header in TRANSACTIONS_EXPORT_HEADERS],
                (
                    [
                        _text(row.get(header), _transaction_number_format(header))
                        for header in TRANSACTIONS_EXPORT_HEADERS
                    ]
                    for row in rows[:PREVIEW_ROWS]
                ),
                len(rows),
            )
        ],
        "warnings": [],
    }


def _preview_year_handoff(conn, hooks: ReportHooks, year):
    handoff = year_handoff.build_year_handoff(conn, None, None, hooks, year=year)
    fiat = str(_row_get(handoff["profile"], "fiat_currency") or "")
    sheets = []
    for sheet in year_handoff._handoff_sheets(handoff):
        columns = sheet["columns"]
        sheets.append(
            _sheet(
                sheet["name"],
                [column.label.format(fiat=fiat) for column in columns],
                (
                    [_text(row.get(column.key), column.format) for column in columns]
                    for row in sheet["rows"][:PREVIEW_ROWS]
                ),
                len(sheet["rows"]),
            )
        )
    period = handoff["period"]
    return {
        "file_name": f"kassiber-year-handoff-{period.year}.xlsx",
        "row_count": len(handoff["transactions"]),
        "sheets": sheets,
        "warnings": handoff["warnings"],
        "reconciled": handoff["reconciled"],
        "year": period.year,
    }


def _shorten(value: str) -> str:
    return f"{value[:6]}…{value[-4:]}" if len(value) > 14 else "…"


def _preview_labels(conn, metadata_hooks, wallet_refs):
    built = core_metadata.bip329_export_records(
        conn, None, None, metadata_hooks, wallet_refs=wallet_refs, mode="all"
    )
    records = built["records"]
    rows = []
    for record in records[:PREVIEW_ROWS]:
        record_type = str(record.get("type") or "")
        ref = str(record.get("ref") or "")
        rows.append(
            [
                record_type,
                _shorten(ref) if record_type in _KEY_MATERIAL_TYPES else ref,
                str(record.get("label") or ""),
            ]
        )
    return {
        "file_name": "kassiber-labels.jsonl",
        "row_count": len(records),
        "sheets": [_sheet("BIP 329", ["type", "ref", "label"], rows, len(records))],
        "warnings": [],
    }


def preview_export(
    conn,
    report_hooks: ReportHooks,
    metadata_hooks,
    *,
    export: str,
    export_format: str | None = None,
    wallet_ref: str | None = None,
    wallet_refs: list[str] | None = None,
    year: int | None = None,
) -> dict[str, Any]:
    if wallet_refs is None and wallet_ref:
        wallet_refs = [wallet_ref]
    if export not in EXPORTS:
        raise AppError(
            f"Unknown export '{export}'",
            code="validation",
            hint="Use one of: " + ", ".join(EXPORTS) + ".",
            retryable=False,
        )
    if export == "transactions":
        export_format = export_format or "xlsx"
        if export_format not in TRANSACTION_FORMATS:
            raise AppError(
                f"Unknown transaction export format '{export_format}'",
                code="validation",
                hint="Use one of: " + ", ".join(TRANSACTION_FORMATS) + ".",
                retryable=False,
            )
        preview = _preview_transactions(conn, report_hooks, export_format, wallet_refs)
    elif export == "year_handoff":
        preview = _preview_year_handoff(conn, report_hooks, year)
    else:
        preview = _preview_labels(conn, metadata_hooks, wallet_refs)
    preview.setdefault("reconciled", None)
    return {"export": export, "format": export_format, "wallets": wallet_refs, **preview}


__all__ = ["EXPORTS", "PREVIEW_ROWS", "TRANSACTION_FORMATS", "preview_export"]
