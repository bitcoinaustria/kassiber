from __future__ import annotations

"""Analyze a user-supplied exchange/broker export before importing it.

This is the shared seam behind `wallets analyze-file`, the desktop
`ui.wallets.analyze_file` kind, and the assistant's file-attachment tool. It
answers "what is this file, and can the deterministic importer read it?" for an
exchange Kassiber has no predefined importer for.

Two hard rules shape the payload:

1. **The model proposes a plan; the importer parses.** For tabular files the
   only thing an AI has to decide is which column means what — a
   ``column_map`` fed straight back into
   ``importers.preview_generic_ledger_records`` / ``load_generic_ledger_records``.
   No model output ever becomes an amount, a date, or an asset. (Photos/PDFs
   cannot work that way, which is exactly why the loopback-only OCR path in
   ``core.document_import`` carries per-cell confidence and quarantines.)
2. **Header-only for off-device models.** Inferring a column plan needs the
   header row, not the rows. ``redact_for_egress`` drops every cell value, so
   attaching a full trade history to a chat backed by a remote provider sends
   column names and counts — never amounts, dates, counterparties, or txids.
"""

import csv
import os
import re
from typing import Any, Mapping, Sequence

from .. import importers as importers_module
from ..errors import AppError
from . import document_import as core_document_import


TABULAR_EXTENSIONS = frozenset({".csv", ".tsv", ".txt", ".xlsx", ".xlsm"})
DOCUMENT_EXTENSIONS = frozenset(core_document_import.SUPPORTED_EXTENSIONS)
MAX_ANALYZE_BYTES = 64 * 1024 * 1024
MAX_HEADER_COLUMNS = 256
DEFAULT_SAMPLE_ROWS = 5
MAX_SAMPLE_ROWS = 25

# The plan shape and its validation live beside `infer_ledger_columns`, which
# produces it, and are enforced inside the importer's own `_ledger_source_records`
# so no path can consume an unvalidated plan. Re-exported here because this is
# the module CLI/daemon/AI callers already import.
PLAN_FIELDS = importers_module.LEDGER_PLAN_FIELDS
normalize_column_map = importers_module.normalize_column_map


def _analyze_path(path: str) -> str:
    raw = str(path or "").strip()
    if not raw:
        raise AppError("A file path is required", code="validation", retryable=False)
    candidate = os.path.abspath(os.path.expanduser(raw))
    if not os.path.exists(candidate):
        raise AppError(
            f"File not found: {candidate}",
            code="not_found",
            hint="Check the path.",
            retryable=False,
        )
    if not os.path.isfile(candidate):
        raise AppError("Analysis source must be a file", code="validation", retryable=False)
    size = os.path.getsize(candidate)
    if size > MAX_ANALYZE_BYTES:
        raise AppError(
            "File is too large to analyze",
            code="validation",
            hint="Split the export, or import it in per-year files.",
            details={"size_bytes": size, "max_bytes": MAX_ANALYZE_BYTES},
            retryable=False,
        )
    return candidate


def _delimiter_name(path: str) -> str | None:
    """Report the sniffed delimiter for CSV-ish files, for the caller's benefit."""
    if os.path.splitext(path)[1].lower() in {".xlsx", ".xlsm"}:
        return None
    try:
        with open(path, "r", encoding="utf-8-sig", errors="replace", newline="") as handle:
            sample = handle.read(8192)
    except OSError:
        return None
    if not sample:
        return None
    try:
        return csv.Sniffer().sniff(sample, delimiters=",;\t|").delimiter
    except csv.Error:
        return None


def file_kind(path: str) -> str:
    extension = os.path.splitext(path)[1].lower()
    if extension in {".xlsx", ".xlsm"}:
        return "spreadsheet"
    if extension in TABULAR_EXTENSIONS:
        return "delimited"
    if extension == core_document_import.PDF_EXTENSION:
        return "pdf"
    if extension in DOCUMENT_EXTENSIONS:
        return "image"
    return "unsupported"


def analyze_file(
    source_file: str,
    *,
    column_map: Any = None,
    sample_rows: int = DEFAULT_SAMPLE_ROWS,
) -> dict[str, Any]:
    """Describe an arbitrary export and whether the ledger importer can read it.

    Tabular files are analyzed locally with no AI call at all: the header is
    matched against the importer's own column aliases and, when that is enough,
    the file is previewed through the real per-row normalizer so the caller sees
    exactly what would import. Photos and PDFs are not analyzed here — they need
    the loopback-only vision path and are reported as such.
    """
    path = _analyze_path(source_file)
    kind = file_kind(path)
    filename = os.path.basename(path)
    source = {
        "path": path,
        "filename": filename,
        "kind": kind,
        "size_bytes": os.path.getsize(path),
        "delimiter": _delimiter_name(path) if kind == "delimited" else None,
    }

    if kind in {"pdf", "image"}:
        return {
            "source": source,
            "route": "document_import",
            "supported": True,
            "next_step": {
                "action": "document_import_preview",
                "reason": (
                    "Photos and PDFs are read by a local vision model, which never "
                    "leaves this device. Preview the draft rows, then import the "
                    "ones you confirm."
                ),
            },
        }

    if kind == "unsupported":
        raise AppError(
            f"Cannot analyze '{os.path.splitext(filename)[1] or '(no extension)'}' files",
            code="validation",
            hint="Export the history as CSV or XLSX, or supply a PDF/photo statement.",
            details={
                "supported_extensions": sorted(TABULAR_EXTENSIONS | DOCUMENT_EXTENSIONS),
            },
            retryable=False,
        )

    # One read: the preview reports the header row it used, so the plan can be
    # validated against the same headers the importer actually saw.
    bound = max(0, min(int(sample_rows or 0), MAX_SAMPLE_ROWS))
    plan = normalize_column_map(column_map)
    preview = importers_module.preview_generic_ledger_records(
        path, limit=bound, column_map=plan
    )
    headers = list(preview.get("headers") or [])[:MAX_HEADER_COLUMNS]
    native = bool(preview.get("template"))
    # Re-validate now that the real headers are known, so a plan naming a column
    # the file does not have is rejected here rather than silently ignored.
    plan = normalize_column_map(column_map, headers)
    inferred = importers_module.infer_ledger_columns(headers)

    confident = bool(preview.get("confident", True))
    return {
        "source": source,
        "route": "generic_ledger",
        "supported": True,
        "template": native,
        "headers": headers,
        # The inferred plan is the model's starting point when it has to correct
        # a mapping; `detected` names each column the importer already recognized.
        "inferred_plan": inferred["plan"],
        "inferred_confident": inferred["confident"],
        "detected": inferred["detected"],
        "applied_plan": plan,
        "confident": confident,
        "rows_read": preview.get("rows_read", 0),
        "mapped": preview.get("mapped", 0),
        "errors": preview.get("errors", 0),
        "problems": preview.get("problems", []),
        "preview": preview.get("preview", []),
        "truncated": bool(preview.get("truncated")),
        "next_step": _next_step(native=native, confident=confident, preview=preview),
    }


def _next_step(*, native: bool, confident: bool, preview: Mapping[str, Any]) -> dict[str, str]:
    if not confident:
        return {
            "action": "supply_column_map",
            "reason": (
                "The columns in this file were not recognized. Map them with "
                "column_map (at least a date column and one amount column), or "
                "fill in the blank ledger template instead."
            ),
        }
    if not preview.get("mapped"):
        return {
            "action": "fix_rows",
            "reason": "The columns were recognized but no row could be normalized.",
        }
    if preview.get("errors"):
        return {
            "action": "review_problems",
            "reason": (
                "Some rows would be rejected. Import the rest, or correct the "
                "mapping and preview again."
            ),
        }
    return {
        "action": "import",
        "reason": (
            "Every row normalized. Import with source_format=generic_ledger, "
            "passing the same column_map."
            if not native
            else "This is the native ledger template; import it as generic_ledger."
        ),
    }


# Cell-bearing keys dropped before an analysis reaches an off-device model.
# `problems` carries per-row validation messages that quote offending cells, and
# `preview` is normalized row data — both are exactly the trade history the
# header-only policy exists to keep on this device.
_EGRESS_DROPPED_KEYS = ("problems", "preview")
# `path` is a local filesystem path; `filename` is routinely personal
# ("Umsatzliste_AT61…csv", "statement_max.mustermann.pdf") and says nothing a
# column plan needs. Extension and kind carry the useful part.
_EGRESS_DROPPED_SOURCE_KEYS = ("path", "filename")

# A "header" is only a header if the file has one. `analyze_file` takes the first
# non-empty row, which for a preamble line or a headerless export is *data* — the
# exact class of file this feature exists for. These patterns catch the cells that
# would leak a value, so header-only egress cannot smuggle a row.
_HEADERISH_NUMBER = re.compile(r"^[+-]?[\d.,\s']*\d[\d.,\s']*$")
_HEADERISH_DATE = re.compile(r"\d{4}-\d{2}-\d{2}|\d{1,2}[./]\d{1,2}[./]\d{2,4}")
_HEADERISH_HEX = re.compile(r"[0-9a-fA-F]{32,}")
_HEADERISH_IBAN = re.compile(r"\b[A-Z]{2}\d{2}[A-Z0-9]{10,}\b")
_HEADERISH_EMAIL = re.compile(r"[^@\s]+@[^@\s]+\.[A-Za-z]{2,}")
_HEADERISH_LONG = 64

WITHHELD_HEADER = "[withheld]"


def _header_is_safe_to_disclose(value: str) -> bool:
    """Whether a header cell is a column name rather than a data value."""
    text = value.strip()
    if not text:
        return True
    if len(text) > _HEADERISH_LONG:
        return False
    return not (
        _HEADERISH_NUMBER.match(text)
        or _HEADERISH_DATE.search(text)
        or _HEADERISH_HEX.search(text)
        or _HEADERISH_IBAN.search(text)
        or _HEADERISH_EMAIL.search(text)
    )


def redact_headers_for_egress(headers: Sequence[str]) -> list[str]:
    """Disclose a header row only if it is actually a header row.

    Judged for the row as a whole, not cell by cell. A genuine header contains no
    amounts, dates or hashes, so a single data-shaped cell means this row is a
    data row — and then its *text* cells are data too. Withholding them
    individually would still leak a counterparty name, which is indistinguishable
    from a column name in isolation.
    """
    cells = [str(header) for header in headers]
    if all(_header_is_safe_to_disclose(cell) for cell in cells):
        return cells
    return [WITHHELD_HEADER if cell.strip() else cell for cell in cells]


def redact_for_egress(analysis: Mapping[str, Any]) -> dict[str, Any]:
    """Reduce an analysis to what may reach an off-device model.

    Column names and counts survive, because that is all it takes to propose or
    correct a column plan. Every cell value is dropped: amounts, timestamps,
    counterparties, txids, and the local file path.
    """
    safe = {
        key: value
        for key, value in analysis.items()
        if key not in _EGRESS_DROPPED_KEYS
    }
    source = safe.get("source")
    if isinstance(source, Mapping):
        safe["source"] = {
            key: value
            for key, value in source.items()
            if key not in _EGRESS_DROPPED_SOURCE_KEYS
        }
        filename = str(source.get("filename") or "")
        safe["source"]["extension"] = os.path.splitext(filename)[1].lower() or None
    if isinstance(safe.get("headers"), Sequence) and not isinstance(
        safe.get("headers"), str
    ):
        headers = redact_headers_for_egress(safe["headers"])
        safe["headers"] = headers
        # A file whose "header" row was withheld has no usable header at all, so
        # say so rather than letting the model treat [withheld] as a column name.
        safe["headers_withheld"] = headers.count(WITHHELD_HEADER)
    # The inferred/applied plans name columns, so they can echo a withheld cell.
    for key in ("inferred_plan", "applied_plan"):
        plan = safe.get(key)
        if isinstance(plan, Mapping):
            safe[key] = {
                field: (
                    value
                    if not isinstance(value, str)
                    or _header_is_safe_to_disclose(value)
                    else WITHHELD_HEADER
                )
                for field, value in plan.items()
            }
    detected = safe.get("detected")
    if isinstance(detected, list):
        safe["detected"] = [
            {
                **entry,
                "column": (
                    entry.get("column")
                    if _header_is_safe_to_disclose(str(entry.get("column") or ""))
                    else WITHHELD_HEADER
                ),
            }
            if isinstance(entry, Mapping)
            else entry
            for entry in detected
        ]
    if "problems" in analysis:
        safe["problem_rows"] = len(analysis["problems"] or [])
    safe["cell_values_withheld"] = True
    return safe


def demo() -> None:
    """Self-check: plan validation is the money path, so pin its refusals."""
    import tempfile

    headers = ["When", "Side", "BTC Amount", "Fee", "EUR Value"]

    plan = normalize_column_map(
        {"date": "When", "type": "Side", "amount": "BTC Amount", "fee": "Fee"},
        headers,
    )
    assert plan["date"] == "When" and plan["amount"] == "BTC Amount"
    # Asset hints are derived from the chosen column names.
    assert plan["amount_header_asset"] == "BTC"

    def refuses(raw, *, code="validation"):
        try:
            normalize_column_map(raw, headers)
        except AppError as exc:
            assert exc.code == code, (raw, exc.code)
            return True
        raise AssertionError(f"expected refusal for {raw!r}")

    refuses({"date": "When", "amount": "BTC Amount", "evil": "Fee"})  # unknown field
    refuses({"date": "When", "amount": "Nope"})  # column not in file
    refuses({"date": "When", "amount": "BTC Amount", "fee": "BTC Amount"})  # double claim
    refuses({"amount": "BTC Amount"})  # no date
    refuses({"date": "When"})  # no amount
    refuses({"date": "When", "amount": "BTC Amount", "amount_header_asset": "DOGE"})
    refuses("date=When")  # not a mapping
    # A vocabulary map may rename a Type, never invent a tax kind.
    refuses(
        {
            "date": "When",
            "type": "Side",
            "amount": "BTC Amount",
            "type_map": {"ACQ-MKT": "Tax Free Bonus"},
        }
    )
    assert normalize_column_map(None) is None

    with tempfile.TemporaryDirectory() as tmp:
        # An export from an exchange with no predefined importer, whose column
        # names happen to match the importer's aliases: no AI needed at all.
        path = os.path.join(tmp, "xyz-export.csv")
        with open(path, "w", encoding="utf-8", newline="") as handle:
            handle.write("Timestamp,Side,BTC Amount,Fee,Currency,Total\n")
            handle.write("2024-03-01T10:00:00Z,Buy,0.5,0.0001,EUR,21000\n")
            handle.write("2024-04-02T10:00:00Z,Sell,0.25,0.0001,EUR,12000\n")
        analysis = analyze_file(path)
        assert analysis["route"] == "generic_ledger", analysis
        assert analysis["confident"] is True, analysis
        assert analysis["mapped"] == 2, analysis
        assert analysis["next_step"]["action"] == "import", analysis
        assert analysis["source"]["delimiter"] == ",", analysis

        redacted = redact_for_egress(analysis)
        assert redacted["headers"] == analysis["headers"]
        assert "preview" not in redacted and "problems" not in redacted
        assert "path" not in redacted["source"]
        assert redacted["problem_rows"] == 0
        assert redacted["cell_values_withheld"] is True
        # The whole point of header-only egress: no cell value survives it.
        flattened = repr(redacted)
        for leaked in ("0.5", "21000", "2024-03-01", tmp):
            assert leaked not in flattened, (leaked, flattened)

        # An export whose headers the importer cannot recognize reports that and
        # asks for a plan, instead of raising or guessing at a money column.
        opaque = os.path.join(tmp, "opaque.csv")
        with open(opaque, "w", encoding="utf-8", newline="") as handle:
            handle.write("Ausführung,Richtung,Stück\n")
            handle.write("2024-03-01T10:00:00Z,Buy,0.5\n")
        unknown = analyze_file(opaque)
        assert unknown["confident"] is False, unknown
        assert unknown["next_step"]["action"] == "supply_column_map", unknown
        # ...and the plan an assistant proposes from those headers makes the same
        # file importable, with the deterministic importer doing every conversion.
        mapped = analyze_file(
            opaque,
            column_map={"date": "Ausführung", "type": "Richtung", "amount": "Stück"},
        )
        assert mapped["confident"] is True and mapped["mapped"] == 1, mapped
        assert mapped["preview"][0]["amount"] == "0.5", mapped["preview"]

        # An export whose Type values are a house vocabulary: recognized columns
        # are not enough, and the importer refuses to guess what "ACQ-MKT" means
        # rather than booking it as something. A type_map resolves it.
        house = os.path.join(tmp, "house.csv")
        with open(house, "w", encoding="utf-8", newline="") as handle:
            handle.write("Trade Date,Action,Qty,Currency,Total\n")
            handle.write("2024-05-01T09:00:00Z,ACQ-MKT,1.5,EUR,60000\n")
        guessed = analyze_file(house)
        assert guessed["mapped"] == 0 and guessed["errors"] == 1, guessed
        named = analyze_file(
            house, column_map={"date": "Trade Date", "type": "Action", "amount": "Qty",
                               "fiat_currency": "Currency", "fiat_value": "Total",
                               "type_map": {"ACQ-MKT": "Buy"}}
        )
        assert named["mapped"] == 1 and named["errors"] == 0, named
        assert named["preview"][0]["kind"] == "buy", named["preview"]

    print("file_analysis demo OK")


if __name__ == "__main__":
    demo()
