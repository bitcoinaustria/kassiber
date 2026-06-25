"""Custom CSV mapping engine: a declarative spec -> canonical import records.

Kassiber ships bespoke importers for known exchanges/wallets, plus a generic
``csv`` import that only works when the file's headers already match canonical
field names. This module covers the long tail: a user-authored *mapping spec*
that maps arbitrary columns onto the canonical transaction-record shape that
``kassiber.core.imports.normalize_import_record`` already consumes.

Design contract:

- **Pure, std-lib only, Decimal-exact.** No DB, no network, no dependency on
  ``imports.py``. The engine turns rows + a spec into a list of import-record
  dicts; the caller hands those to ``import_records_into_wallet``.
- **One shared reader.** ``inspect_csv`` (which powers the GUI column pickers)
  and ``read_table`` (which feeds ``apply_mapping``) use the same parsing path,
  so the delimiter and duplicate-header handling they show always match what the
  import actually does. ``csv.Sniffer`` is wrapped and never raises; duplicate
  headers are deterministically suffixed (``Amount``, ``Amount__2``).
- **Stable, machine-coded row problems.** ``apply_mapping`` never raises on row
  *data* (the spec is pre-validated); per-row issues are returned as ``problems``
  with a stable ``reason`` code the GUI localizes. Filtered rows (intentional
  skips) are distinguished from errors.
- **Idempotent txid-less imports.** Arbitrary exports frequently have no
  transaction id. When ``txid`` is unmapped/empty the engine synthesizes a
  stable ``csvmap:<hash>`` id from the row's position + content so two
  same-day/same-amount rows stay distinct *and* re-importing the same file stays
  idempotent (the dedupe fingerprint keys on ``external_id``). Mapping a real
  id/reference column is recommended when the export has one.

The spec schema and worked examples live in ``docs/reference/csv-mapping.md``.
"""

from __future__ import annotations

import csv
import hashlib
import json
import os
import re
from datetime import datetime, timezone
from decimal import Decimal
from typing import Any, Mapping, Sequence

try:  # Python 3.9+
    from zoneinfo import ZoneInfo
except ImportError:  # pragma: no cover - stdlib on supported versions
    ZoneInfo = None  # type: ignore[assignment]

from ..errors import AppError
from ..msat import MSAT_PER_BTC, SATS_PER_BTC, dec

SPEC_VERSION = 1
DEFAULT_ENCODING = "utf-8-sig"
DEFAULT_SOURCE_KIND = "generic_import"
SOURCE_LABEL = "file:mapped_csv"
INPUT_FORMAT = "mapped_csv"

ALLOWED_ASSETS = {"BTC", "LBTC"}
ALLOWED_UNITS = {"btc", "sat", "msat"}
ALLOWED_DIRECTIONS = {"inbound", "outbound"}
ALLOWED_AMOUNT_MODES = {"signed", "split", "absolute"}
ALLOWED_FILTER_OPS = {"equals", "in", "not_empty"}
# confirmed_at is deliberately excluded: it would need its own format handling,
# and an un-parseable value would break normalize_import_record's strict parse.
ALLOWED_FIELD_TARGETS = {
    "kind",
    "description",
    "counterparty",
    "payment_hash",
    "privacy_boundary",
    "amount_includes_fee",
}
ALLOWED_LAYOUTS = ("signed", "split", "absolute")

_UNIT_DIVISOR = {"btc": Decimal(1), "sat": SATS_PER_BTC, "msat": MSAT_PER_BTC}


# --------------------------------------------------------------------------- #
# Small helpers
# --------------------------------------------------------------------------- #
def _norm_str(value: Any) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _lower_list(value: Any) -> list[str]:
    if value is None:
        return []
    items = value if isinstance(value, (list, tuple)) else str(value).split(",")
    out: list[str] = []
    for item in items:
        text = str(item).strip().casefold()
        if text:
            out.append(text)
    return out


def _zone(name: Any):
    text = _norm_str(name)
    if text is None or text.upper() == "UTC":
        return timezone.utc
    if ZoneInfo is None:  # pragma: no cover
        return None
    try:
        return ZoneInfo(text)
    except Exception:
        return None


def _truthy(value: Any) -> bool:
    if isinstance(value, bool):
        return value
    return str(value).strip().casefold() in {"1", "true", "yes", "y", "t"}


def _coerce_decimal(raw: Any, decimal_sep: str) -> Decimal | None:
    """Parse a possibly locale-formatted number into ``Decimal`` (None if blank).

    Raises ``AppError`` (via ``dec``) on junk; callers translate that into a
    per-row problem.
    """
    text = str(raw).strip()
    if text == "":
        return None
    if decimal_sep == ",":
        # European: '.' groups thousands, ',' is the decimal point.
        text = text.replace(".", "").replace(",", ".")
    else:
        # Anglo: strip thousands ','.
        text = text.replace(",", "")
    return dec(text)


# --------------------------------------------------------------------------- #
# Spec loading + validation
# --------------------------------------------------------------------------- #
def _unwrap_spec(data: dict) -> dict:
    """Tolerate envelope/wrapper shapes so a saved CLI/daemon response works.

    ``wallets mapping-template`` (and the daemon kind) returns the spec nested as
    ``{... "data": {"layout", "mapping": <spec>}}`` / ``{"layout", "mapping"}``;
    unwrap those so a user can pass the saved output straight back as
    ``--mapping``. A bare spec (which never has a top-level ``mapping`` key) is
    returned unchanged.
    """
    inner = data.get("data")
    if isinstance(inner, dict) and isinstance(inner.get("mapping"), dict):
        data = inner
    if isinstance(data.get("mapping"), dict) and "amount" not in data and "timestamp" not in data:
        data = data["mapping"]
    return data


def load_mapping_spec(source: Any) -> dict:
    """Accept a dict, a path to a JSON file, or an inline JSON string."""
    if isinstance(source, Mapping):
        return _unwrap_spec(dict(source))
    if not isinstance(source, str):
        raise AppError(
            "Mapping must be a JSON object, a JSON string, or a file path.",
            code="csv_mapping_invalid",
        )
    text = source
    if os.path.exists(source):
        try:
            with open(source, "r", encoding="utf-8") as handle:
                text = handle.read()
        except OSError as exc:
            raise AppError(
                f"Could not read mapping file: {exc}",
                code="csv_mapping_invalid",
                hint="Check the --mapping path.",
            ) from exc
    try:
        data = json.loads(text)
    except json.JSONDecodeError as exc:
        raise AppError(
            f"Mapping is not valid JSON: {exc}",
            code="csv_mapping_invalid",
            hint="Pass a JSON object, a JSON string, or a path to a .json file.",
        ) from exc
    if not isinstance(data, dict):
        raise AppError("Mapping must be a JSON object.", code="csv_mapping_invalid")
    return _unwrap_spec(data)


def validate_mapping_spec(spec: Any) -> dict:
    """Structurally + semantically validate, fill defaults, collect all errors."""
    if not isinstance(spec, Mapping):
        raise AppError("Mapping must be a JSON object.", code="csv_mapping_invalid")
    errors: list[dict[str, str]] = []
    out: dict[str, Any] = {}

    version = spec.get("version", SPEC_VERSION)
    if version != SPEC_VERSION:
        errors.append({"field": "version", "reason": f"unsupported version {version!r}; expected {SPEC_VERSION}"})
    out["version"] = SPEC_VERSION
    out["name"] = str(spec.get("name") or "")

    asset = str(spec.get("asset") or "BTC").strip().upper()
    if asset not in ALLOWED_ASSETS:
        errors.append({"field": "asset", "reason": f"unsupported asset {asset!r}; allowed {sorted(ALLOWED_ASSETS)}"})
    out["asset"] = asset

    delimiter = spec.get("delimiter", None)
    if delimiter is not None and (not isinstance(delimiter, str) or len(delimiter) != 1):
        errors.append({"field": "delimiter", "reason": "delimiter must be a single character or null (auto-detect)"})
        delimiter = None
    out["delimiter"] = delimiter

    out["encoding"] = str(spec.get("encoding") or DEFAULT_ENCODING)

    skip_rows = spec.get("skip_rows", 0)
    try:
        skip_rows = int(skip_rows)
        if skip_rows < 0:
            raise ValueError
    except (TypeError, ValueError):
        errors.append({"field": "skip_rows", "reason": "skip_rows (lines before the header) must be a non-negative integer"})
        skip_rows = 0
    out["skip_rows"] = skip_rows

    out["timestamp"] = _validate_timestamp(spec.get("timestamp"), errors)
    out["amount"] = _validate_amount(spec.get("amount"), errors)
    out["fee"] = _validate_fee(spec.get("fee"), errors)
    out["txid"] = _validate_txid(spec.get("txid"), errors)
    out["fields"] = _validate_fields(spec.get("fields"), errors)
    out["pricing"] = _validate_pricing(spec.get("pricing"), errors)
    out["filters"] = _validate_filters(spec.get("filters"), errors)

    if errors:
        raise AppError(
            "Invalid CSV mapping.",
            code="csv_mapping_invalid",
            details={"errors": errors},
            hint="Fix the listed fields and try again.",
        )
    return out


def _validate_timestamp(ts: Any, errors: list) -> dict:
    if not isinstance(ts, Mapping):
        errors.append({"field": "timestamp", "reason": "timestamp must be an object with a 'column'"})
        return {"column": "", "format": None, "timezone": "UTC"}
    column = _norm_str(ts.get("column"))
    if not column:
        errors.append({"field": "timestamp.column", "reason": "a date column is required"})
    fmt = _norm_str(ts.get("format"))
    tzname = _norm_str(ts.get("timezone")) or "UTC"
    if _zone(tzname) is None:
        errors.append({"field": "timestamp.timezone", "reason": f"unknown time zone {tzname!r}"})
    return {"column": column or "", "format": fmt, "timezone": tzname}


def _validate_amount(amount: Any, errors: list) -> dict:
    if not isinstance(amount, Mapping):
        errors.append({"field": "amount", "reason": "amount must be an object with a 'mode'"})
        return {"mode": "signed", "column": "", "unit": "btc", "decimal_separator": "."}
    mode = (_norm_str(amount.get("mode")) or "").lower()
    unit = (_norm_str(amount.get("unit")) or "btc").lower()
    if unit not in ALLOWED_UNITS:
        errors.append({"field": "amount.unit", "reason": f"unit must be one of {sorted(ALLOWED_UNITS)}"})
        unit = "btc"
    sep = amount.get("decimal_separator") or "."
    if sep not in (".", ","):
        errors.append({"field": "amount.decimal_separator", "reason": "decimal_separator must be '.' or ','"})
        sep = "."
    out: dict[str, Any] = {"mode": mode, "unit": unit, "decimal_separator": sep}
    if mode == "signed":
        column = _norm_str(amount.get("column"))
        if not column:
            errors.append({"field": "amount.column", "reason": "signed mode needs a 'column'"})
        out["column"] = column or ""
    elif mode == "split":
        inbound = _norm_str(amount.get("inbound_column"))
        outbound = _norm_str(amount.get("outbound_column"))
        if not inbound and not outbound:
            errors.append({"field": "amount", "reason": "split mode needs inbound_column and/or outbound_column"})
        out["inbound_column"] = inbound or ""
        out["outbound_column"] = outbound or ""
    elif mode == "absolute":
        column = _norm_str(amount.get("column"))
        if not column:
            errors.append({"field": "amount.column", "reason": "absolute mode needs a 'column'"})
        out["column"] = column or ""
        out["direction"] = _validate_direction(amount.get("direction"), errors)
    else:
        errors.append({"field": "amount.mode", "reason": f"mode must be one of {sorted(ALLOWED_AMOUNT_MODES)}"})
    return out


def _validate_direction(direction: Any, errors: list) -> dict:
    if not isinstance(direction, Mapping):
        errors.append({"field": "amount.direction", "reason": "absolute mode needs a 'direction' (const or column rules)"})
        return {"const": "inbound"}
    if "const" in direction:
        const = _norm_str(direction.get("const"))
        if const not in ALLOWED_DIRECTIONS:
            errors.append({"field": "amount.direction.const", "reason": f"const must be one of {sorted(ALLOWED_DIRECTIONS)}"})
        return {"const": const or "inbound"}
    column = _norm_str(direction.get("column"))
    if not column:
        errors.append({"field": "amount.direction.column", "reason": "direction needs a 'column' or a 'const'"})
    inbound = _lower_list(direction.get("inbound_values"))
    outbound = _lower_list(direction.get("outbound_values"))
    if not inbound and not outbound:
        errors.append({"field": "amount.direction", "reason": "provide inbound_values and/or outbound_values, or use a const"})
    overlap = sorted(set(inbound) & set(outbound))
    if overlap:
        errors.append({"field": "amount.direction", "reason": f"values appear in both inbound and outbound: {overlap}"})
    default = _norm_str(direction.get("default"))
    if default is not None and default not in ALLOWED_DIRECTIONS:
        errors.append({"field": "amount.direction.default", "reason": f"default must be null or one of {sorted(ALLOWED_DIRECTIONS)}"})
    return {"column": column or "", "inbound_values": inbound, "outbound_values": outbound, "default": default}


def _validate_fee(fee: Any, errors: list) -> dict | None:
    if fee in (None, {}):
        return None
    if not isinstance(fee, Mapping):
        errors.append({"field": "fee", "reason": "fee must be an object or null"})
        return None
    column = _norm_str(fee.get("column"))
    if not column:
        errors.append({"field": "fee.column", "reason": "fee needs a 'column'"})
    unit = (_norm_str(fee.get("unit")) or "btc").lower()
    if unit not in ALLOWED_UNITS:
        errors.append({"field": "fee.unit", "reason": f"unit must be one of {sorted(ALLOWED_UNITS)}"})
        unit = "btc"
    sep = fee.get("decimal_separator") or "."
    if sep not in (".", ","):
        errors.append({"field": "fee.decimal_separator", "reason": "decimal_separator must be '.' or ','"})
        sep = "."
    return {"column": column or "", "unit": unit, "decimal_separator": sep}


def _validate_txid(txid: Any, errors: list) -> dict | None:
    if txid in (None, {}):
        return None
    if not isinstance(txid, Mapping):
        errors.append({"field": "txid", "reason": "txid must be an object or null"})
        return None
    column = _norm_str(txid.get("column"))
    if not column:
        errors.append({"field": "txid.column", "reason": "txid needs a 'column'"})
    return {"column": column or ""}


def _validate_ref(ref: Any, name: str, errors: list) -> dict | None:
    if not isinstance(ref, Mapping):
        errors.append({"field": name, "reason": "must be {\"column\": ...} or {\"const\": ...}"})
        return None
    if "const" in ref:
        return {"const": ref["const"]}
    column = _norm_str(ref.get("column"))
    if not column:
        errors.append({"field": f"{name}.column", "reason": "needs a 'column' or a 'const'"})
        return None
    return {"column": column}


def _validate_fields(fields: Any, errors: list) -> dict:
    if fields in (None, {}):
        return {}
    if not isinstance(fields, Mapping):
        errors.append({"field": "fields", "reason": "fields must be an object"})
        return {}
    out: dict[str, Any] = {}
    for target, ref in fields.items():
        if target not in ALLOWED_FIELD_TARGETS:
            errors.append({"field": f"fields.{target}", "reason": f"unknown field; allowed {sorted(ALLOWED_FIELD_TARGETS)}"})
            continue
        resolved = _validate_ref(ref, f"fields.{target}", errors)
        if resolved is not None:
            out[target] = resolved
    return out


def _validate_pricing(pricing: Any, errors: list) -> dict | None:
    if pricing in (None, {}):
        return None
    if not isinstance(pricing, Mapping):
        errors.append({"field": "pricing", "reason": "pricing must be an object or null"})
        return None
    out: dict[str, Any] = {}
    for key in ("fiat_currency", "fiat_rate", "fiat_value"):
        if pricing.get(key) is not None:
            resolved = _validate_ref(pricing.get(key), f"pricing.{key}", errors)
            if resolved is not None:
                out[key] = resolved
    out["source_kind"] = _norm_str(pricing.get("source_kind")) or DEFAULT_SOURCE_KIND
    quality = _norm_str(pricing.get("quality"))
    if quality:
        out["quality"] = quality
    sep = pricing.get("decimal_separator") or "."
    if sep not in (".", ","):
        errors.append({"field": "pricing.decimal_separator", "reason": "decimal_separator must be '.' or ','"})
        sep = "."
    out["decimal_separator"] = sep
    return out


def _validate_filters(filters: Any, errors: list) -> list:
    if filters in (None, []):
        return []
    if not isinstance(filters, (list, tuple)):
        errors.append({"field": "filters", "reason": "filters must be a list"})
        return []
    out: list[dict[str, Any]] = []
    for index, entry in enumerate(filters):
        if not isinstance(entry, Mapping):
            errors.append({"field": f"filters[{index}]", "reason": "each filter must be an object"})
            continue
        column = _norm_str(entry.get("column"))
        op = (_norm_str(entry.get("op")) or "").lower()
        if not column:
            errors.append({"field": f"filters[{index}].column", "reason": "filter needs a 'column'"})
        if op not in ALLOWED_FILTER_OPS:
            errors.append({"field": f"filters[{index}].op", "reason": f"op must be one of {sorted(ALLOWED_FILTER_OPS)}"})
        resolved: dict[str, Any] = {"column": column or "", "op": op or "not_empty"}
        if op in ("equals", "in"):
            if entry.get("value") is None:
                errors.append({"field": f"filters[{index}].value", "reason": f"{op} needs a 'value'"})
            resolved["value"] = entry.get("value")
        out.append(resolved)
    return out


def collect_referenced_columns(spec: Mapping[str, Any]) -> set[str]:
    """Every CSV column name the (validated) spec reads."""
    cols: set[str] = set()

    def add(ref: Any) -> None:
        if isinstance(ref, Mapping) and ref.get("column"):
            cols.add(ref["column"])

    timestamp = spec.get("timestamp") or {}
    if timestamp.get("column"):
        cols.add(timestamp["column"])
    amount = spec.get("amount") or {}
    for key in ("column", "inbound_column", "outbound_column"):
        if amount.get(key):
            cols.add(amount[key])
    direction = amount.get("direction") or {}
    if isinstance(direction, Mapping) and direction.get("column"):
        cols.add(direction["column"])
    add(spec.get("fee"))
    add(spec.get("txid"))
    for ref in (spec.get("fields") or {}).values():
        add(ref)
    pricing = spec.get("pricing") or {}
    for key in ("fiat_currency", "fiat_rate", "fiat_value"):
        add(pricing.get(key))
    for entry in spec.get("filters") or []:
        if isinstance(entry, Mapping) and entry.get("column"):
            cols.add(entry["column"])
    return cols


def validate_columns(spec: Mapping[str, Any], headers: Sequence[str]) -> None:
    """Raise if the spec references a column the CSV does not have."""
    header_set = set(headers)
    missing = sorted(col for col in collect_referenced_columns(spec) if col not in header_set)
    if missing:
        raise AppError(
            f"Mapping references columns not in the CSV: {', '.join(missing)}",
            code="csv_mapping_invalid",
            details={"missing_columns": missing, "available_columns": list(headers)},
            hint="Column names are case- and space-sensitive; check the header row.",
        )


# --------------------------------------------------------------------------- #
# CSV reading (shared by inspect_csv + read_table)
# --------------------------------------------------------------------------- #
def _dedupe_headers(headers: Sequence[str]) -> list[str]:
    seen: dict[str, int] = {}
    used: set[str] = set()
    out: list[str] = []
    for raw in headers:
        name = (raw or "").strip()
        if name not in used:
            candidate = name
        else:
            count = seen.get(name, 1)
            count += 1
            candidate = f"{name}__{count}"
            # Guard against a literal "Amount__2" column already present.
            while candidate in used:
                count += 1
                candidate = f"{name}__{count}"
            seen[name] = count
        seen.setdefault(name, 1)
        used.add(candidate)
        out.append(candidate)
    return out


def _sniff_delimiter(file_path: str, encoding: str, skip_rows: int) -> str:
    try:
        with open(file_path, "r", encoding=encoding, newline="") as handle:
            for _ in range(skip_rows):
                next(handle, None)
            sample = handle.read(8192)
    except (OSError, UnicodeError, LookupError):
        return ","
    try:
        return csv.Sniffer().sniff(sample, delimiters=",;\t|").delimiter
    except Exception:
        return ","


def _read_parsed(file_path: str, delimiter: str, encoding: str, skip_rows: int):
    if not os.path.exists(file_path):
        raise AppError(
            f"CSV file not found: {file_path}",
            code="not_found",
            hint="Pass an existing file path.",
        )
    try:
        with open(file_path, "r", encoding=encoding, newline="") as handle:
            for _ in range(skip_rows):
                next(handle, None)
            reader = csv.reader(handle, delimiter=delimiter)
            try:
                header_row = next(reader)
            except StopIteration:
                return [], []
            headers = _dedupe_headers(header_row)
            rows: list[dict[str, str]] = []
            for raw in reader:
                if not raw or all((cell or "").strip() == "" for cell in raw):
                    continue
                rows.append({key: (raw[i] if i < len(raw) else "") for i, key in enumerate(headers)})
            return headers, rows
    except (OSError, UnicodeError, LookupError, csv.Error) as exc:
        raise AppError(
            f"Could not read CSV: {exc}",
            code="csv_mapping_invalid",
            hint="Check the delimiter and encoding.",
        ) from exc


def read_table(file_path: str, *, delimiter: str | None = None, encoding: str = DEFAULT_ENCODING, skip_rows: int = 0):
    """Return ``(headers, rows)`` using the shared reader (dup headers suffixed)."""
    resolved = delimiter or _sniff_delimiter(file_path, encoding, skip_rows)
    return _read_parsed(file_path, resolved, encoding, skip_rows)


def inspect_csv(
    file_path: str,
    *,
    delimiter: str | None = None,
    encoding: str = DEFAULT_ENCODING,
    skip_rows: int = 0,
    sample: int = 20,
) -> dict:
    """Detect delimiter + headers + a bounded row sample for the column pickers."""
    resolved = delimiter or _sniff_delimiter(file_path, encoding, skip_rows)
    headers, rows = _read_parsed(file_path, resolved, encoding, skip_rows)
    try:
        sample = max(0, int(sample))
    except (TypeError, ValueError):
        sample = 20
    return {
        "delimiter": resolved,
        "encoding": encoding,
        "headers": headers,
        "sample_rows": rows[:sample],
        "row_count_estimate": len(rows),
    }


# --------------------------------------------------------------------------- #
# Row mapping
# --------------------------------------------------------------------------- #
class _RowError(Exception):
    def __init__(self, column: str | None, reason: str, detail: str | None = None):
        super().__init__(reason)
        self.column = column
        self.reason = reason
        self.detail = detail


def _safe_dec(raw: str, sep: str, column: str | None) -> Decimal:
    try:
        value = _coerce_decimal(raw, sep)
    except AppError as exc:
        raise _RowError(column, "bad_amount", f"could not parse number {raw!r}") from exc
    if value is None:
        raise _RowError(column, "amount_missing", "empty amount")
    return value


def _resolve_amount(row: Mapping[str, Any], amount: Mapping[str, Any]):
    unit = amount["unit"]
    sep = amount.get("decimal_separator", ".")
    divisor = _UNIT_DIVISOR[unit]
    mode = amount["mode"]

    if mode == "signed":
        raw = (row.get(amount["column"]) or "").strip()
        if raw == "":
            raise _RowError(amount["column"], "amount_missing", "empty amount")
        value = _safe_dec(raw, sep, amount["column"])
        direction = "outbound" if value < 0 else "inbound"
        return abs(value) / divisor, direction

    if mode == "split":
        inbound_col = amount.get("inbound_column") or None
        outbound_col = amount.get("outbound_column") or None
        in_raw = (row.get(inbound_col) or "").strip() if inbound_col else ""
        out_raw = (row.get(outbound_col) or "").strip() if outbound_col else ""
        in_value = _safe_dec(in_raw, sep, inbound_col) if in_raw else None
        out_value = _safe_dec(out_raw, sep, outbound_col) if out_raw else None
        in_has = in_value is not None and in_value != 0
        out_has = out_value is not None and out_value != 0
        if in_has and out_has:
            raise _RowError(None, "split_ambiguous", "both inbound and outbound columns have a value")
        if not in_has and not out_has:
            raise _RowError(None, "amount_missing", "neither inbound nor outbound column has a value")
        if in_has:
            return abs(in_value) / divisor, "inbound"
        return abs(out_value) / divisor, "outbound"

    # absolute
    raw = (row.get(amount["column"]) or "").strip()
    if raw == "":
        raise _RowError(amount["column"], "amount_missing", "empty amount")
    value = _safe_dec(raw, sep, amount["column"])
    direction = _resolve_direction(row, amount["direction"])
    return abs(value) / divisor, direction


def _resolve_direction(row: Mapping[str, Any], direction: Mapping[str, Any]) -> str:
    if "const" in direction:
        return direction["const"]
    cell = (row.get(direction["column"]) or "").strip().casefold()
    if cell in set(direction.get("inbound_values", [])):
        return "inbound"
    if cell in set(direction.get("outbound_values", [])):
        return "outbound"
    if direction.get("default"):
        return direction["default"]
    raise _RowError(direction["column"], "direction_unresolved", f"unrecognized direction value {cell!r}")


def _resolve_fee(row: Mapping[str, Any], fee: Mapping[str, Any] | None) -> Decimal:
    if not fee:
        return Decimal(0)
    raw = (row.get(fee["column"]) or "").strip()
    if raw == "":
        return Decimal(0)
    try:
        value = _coerce_decimal(raw, fee.get("decimal_separator", "."))
    except AppError as exc:
        raise _RowError(fee["column"], "bad_fee", f"could not parse fee {raw!r}") from exc
    if value is None:
        return Decimal(0)
    return abs(value) / _UNIT_DIVISOR[fee["unit"]]


def _resolve_timestamp(raw_ts: str, ts: Mapping[str, Any]) -> str:
    if raw_ts == "":
        raise _RowError(ts["column"], "bad_timestamp", "empty date")
    fmt = ts.get("format")
    if fmt:
        try:
            parsed = datetime.strptime(raw_ts, fmt)
        except (ValueError, TypeError) as exc:
            raise _RowError(ts["column"], "bad_timestamp", f"does not match format {fmt!r}: {raw_ts!r}") from exc
    else:
        norm = raw_ts
        if len(norm) == 10:
            norm = f"{norm}T00:00:00"
        elif norm.endswith("Z"):
            norm = norm[:-1] + "+00:00"
        try:
            parsed = datetime.fromisoformat(norm)
        except ValueError as exc:
            raise _RowError(ts["column"], "bad_timestamp", f"not an ISO date: {raw_ts!r}") from exc
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=_zone(ts.get("timezone")) or timezone.utc)
    parsed = parsed.astimezone(timezone.utc)
    return parsed.replace(microsecond=0).isoformat().replace("+00:00", "Z")


def _resolve_ref(row: Mapping[str, Any], ref: Mapping[str, Any] | None) -> Any:
    if not ref:
        return None
    if "const" in ref:
        return ref["const"]
    column = ref.get("column")
    if not column:
        return None
    return row.get(column)


def _filter_passes(row: Mapping[str, Any], flt: Mapping[str, Any]) -> bool:
    value = (row.get(flt["column"]) or "").strip()
    op = flt["op"]
    if op == "not_empty":
        return value != ""
    if op == "equals":
        # Distinguish a missing value (None) from a falsy one (0 / false): a
        # hand-authored JSON spec may legitimately use a numeric/boolean value.
        target = flt.get("value")
        target = "" if target is None else str(target)
        return value.casefold() == target.strip().casefold()
    if op == "in":
        raw = flt.get("value")
        if isinstance(raw, (list, tuple)):
            options = raw
        else:
            options = ("" if raw is None else str(raw)).split(",")
        return value.casefold() in {str(opt).strip().casefold() for opt in options}
    return True


def _apply_fields(record: dict, row: Mapping[str, Any], fields: Mapping[str, Any]) -> None:
    for target, ref in fields.items():
        value = _resolve_ref(row, ref)
        if value is None or (isinstance(value, str) and value.strip() == ""):
            continue
        if target == "amount_includes_fee":
            record["amount_includes_fee"] = _truthy(value)
        elif isinstance(value, str):
            record[target] = value.strip()
        else:
            record[target] = value


def _apply_pricing(record: dict, row: Mapping[str, Any], pricing: Mapping[str, Any] | None) -> None:
    if not pricing:
        return
    currency = _resolve_ref(row, pricing.get("fiat_currency"))
    if currency not in (None, ""):
        record["fiat_currency"] = str(currency).strip().upper()
    sep = pricing.get("decimal_separator", ".")
    priced = False
    for key in ("fiat_rate", "fiat_value"):
        raw = _resolve_ref(row, pricing.get(key))
        if raw in (None, ""):
            continue
        try:
            parsed = _coerce_decimal(raw, sep)
        except AppError:
            parsed = None  # non-blocking: row still imports, just unpriced for this field
        if parsed is not None:
            record[key] = format(parsed, "f")
            priced = True
    if priced:
        record["pricing_source_kind"] = pricing.get("source_kind") or DEFAULT_SOURCE_KIND
        if pricing.get("quality"):
            record["pricing_quality"] = pricing["quality"]


def _resolve_txid(row: Mapping[str, Any], spec: Mapping[str, Any], index: int, raw_json: str) -> str:
    txid_spec = spec.get("txid")
    if txid_spec and txid_spec.get("column"):
        value = (row.get(txid_spec["column"]) or "").strip()
        if value:
            return value
    digest = hashlib.sha1(f"{index}|{raw_json}".encode("utf-8")).hexdigest()[:24]
    return f"csvmap:{digest}"


def _map_row(row: Mapping[str, Any], spec: Mapping[str, Any], index: int):
    for flt in spec["filters"]:
        if not _filter_passes(row, flt):
            return None, {"row": index, "kind": "filtered", "column": flt["column"], "reason": f"filtered_{flt['op']}", "detail": None}
    try:
        amount, direction = _resolve_amount(row, spec["amount"])
        occurred_at = _resolve_timestamp((row.get(spec["timestamp"]["column"]) or "").strip(), spec["timestamp"])
        fee = _resolve_fee(row, spec.get("fee"))
    except _RowError as exc:
        return None, {"row": index, "kind": "error", "column": exc.column, "reason": exc.reason, "detail": exc.detail}

    raw_json = json.dumps(dict(row), sort_keys=True, ensure_ascii=False)
    record: dict[str, Any] = {
        "txid": _resolve_txid(row, spec, index, raw_json),
        "occurred_at": occurred_at,
        "direction": direction,
        "asset": spec["asset"],
        "amount": amount,
        "fee": fee,
        "raw_json": raw_json,
    }
    _apply_fields(record, row, spec["fields"])
    _apply_pricing(record, row, spec.get("pricing"))
    return record, None


def apply_mapping(rows, spec: Mapping[str, Any]):
    """Transform rows into canonical import records.

    Returns ``(records, problems)``. ``spec`` must already be validated. The
    function never raises on row data; each row yields exactly one outcome — a
    record (success) or one ``problem`` (filtered or error).
    """
    records: list[dict[str, Any]] = []
    problems: list[dict[str, Any]] = []
    for index, row in enumerate(rows, start=1):
        record, problem = _map_row(row, spec, index)
        if problem is not None:
            problems.append(problem)
        if record is not None:
            records.append(record)
    return records, problems


# --------------------------------------------------------------------------- #
# Preview + template
# --------------------------------------------------------------------------- #
def to_json_safe_record(record: Mapping[str, Any]) -> dict:
    """A JSON-safe view of an engine record for preview (Decimals -> strings)."""
    out: dict[str, Any] = {}
    for key, value in record.items():
        if key == "raw_json":
            continue  # original row is not needed in preview and may be large/noisy
        out[key] = format(value, "f") if isinstance(value, Decimal) else value
    return out


def build_preview(records, problems, *, limit: int | None = None, mapping_name: str = "") -> dict:
    """The shared preview/dry-run payload used by the CLI and the daemon."""
    if limit is not None:
        limit = max(0, limit)
    errors = [p for p in problems if p["kind"] == "error"]
    filtered = [p for p in problems if p["kind"] == "filtered"]
    shown = records[:limit] if limit is not None else records
    bounded_problems = problems[:limit] if limit is not None else problems
    return {
        "mapping_name": mapping_name,
        "rows_read": len(records) + len(problems),
        "mapped": len(records),
        "errors": len(errors),
        "filtered": len(filtered),
        "problems": bounded_problems,
        "preview": [to_json_safe_record(r) for r in shown],
        "truncated": bool(
            limit is not None and (len(records) > limit or len(problems) > limit)
        ),
    }


def mapping_template(layout: str = "signed") -> dict:
    """A documented starter spec (placeholder column names) that passes validate."""
    layout = (layout or "signed").strip().lower()
    if layout not in ALLOWED_LAYOUTS:
        raise AppError(
            f"Unknown layout {layout!r}",
            code="validation",
            hint=f"Use one of {list(ALLOWED_LAYOUTS)}.",
        )
    base: dict[str, Any] = {
        "version": SPEC_VERSION,
        "name": "My wallet export",
        "asset": "BTC",
        "delimiter": None,
        "encoding": DEFAULT_ENCODING,
        "skip_rows": 0,
        "timestamp": {"column": "Date", "format": None, "timezone": "UTC"},
        "fee": {"column": "Fee", "unit": "btc"},
        "txid": {"column": "Txid"},
        "fields": {"description": {"column": "Note"}},
        "pricing": None,
        "filters": [],
    }
    if layout == "signed":
        base["amount"] = {"mode": "signed", "column": "Amount BTC", "unit": "btc"}
    elif layout == "split":
        base["amount"] = {
            "mode": "split",
            "inbound_column": "Received BTC",
            "outbound_column": "Sent BTC",
            "unit": "btc",
        }
    else:  # absolute
        base["amount"] = {
            "mode": "absolute",
            "column": "Amount",
            "unit": "btc",
            "direction": {
                "column": "Type",
                "inbound_values": ["deposit", "buy", "receive"],
                "outbound_values": ["withdrawal", "sell", "send"],
                "default": None,
            },
        }
    return validate_mapping_spec(base)


# --------------------------------------------------------------------------- #
# Auto-detection + fill-in example
# --------------------------------------------------------------------------- #
# The example template uses unambiguous headers so inference is always confident;
# inference also recognizes common real-world aliases for arbitrary exports.
EXAMPLE_HEADERS = ["Date", "Direction", "Amount", "Fee", "Currency", "Price", "Note"]
EXAMPLE_ROWS = [
    ["2026-01-15", "in", "0.05", "0.00001", "EUR", "40000", "Bought on an exchange"],
    ["2026-01-20", "out", "0.01", "0.000005", "EUR", "42000", "Sent to a friend"],
]

_DATE_ALIASES = {"date", "time", "timestamp", "datetime", "date time", "when", "datum", "zeitpunkt", "executed at", "created at", "trade date"}
_AMOUNT_ALIASES = {"amount", "btc", "amount btc", "btc amount", "quantity", "qty", "menge", "betrag", "amount in btc"}
_SENT_ALIASES = {"sent", "sent btc", "outgoing", "out amount", "amount sent", "gesendet", "abgang", "debit amount", "withdrawal amount"}
_RECEIVED_ALIASES = {"received", "received btc", "incoming", "in amount", "amount received", "erhalten", "eingang", "credit amount", "deposit amount"}
_DIRECTION_ALIASES = {"direction", "type", "side", "in out", "transaction type", "tx type", "richtung", "art", "kind of transaction"}
_FEE_ALIASES = {"fee", "fees", "miner fee", "network fee", "tx fee", "transaction fee", "gebuehr", "gebuhr", "gebuehren"}
_TXID_ALIASES = {"txid", "tx id", "transaction id", "tx hash", "hash", "id", "reference", "ref", "transaktions id", "transaktion"}
_FIAT_CURRENCY_ALIASES = {"currency", "fiat", "fiat currency", "waehrung", "wahrung"}
_FIAT_RATE_ALIASES = {"price", "rate", "unit price", "price per btc", "btc price", "fiat rate", "spot", "spot price", "kurs", "preis"}
_FIAT_VALUE_ALIASES = {"total", "total value", "fiat value", "proceeds", "gesamtwert"}
_NOTE_ALIASES = {"note", "notes", "description", "memo", "label", "comment", "notiz", "beschreibung", "kommentar", "verwendungszweck"}
_KIND_ALIASES = {"kind", "category", "kategorie", "label type"}
_COUNTERPARTY_ALIASES = {"counterparty", "payee", "recipient", "gegenpartei", "empfaenger", "empfanger"}

_INFER_INBOUND = ["in", "inbound", "buy", "bought", "deposit", "receive", "received", "credit", "incoming"]
_INFER_OUTBOUND = ["out", "outbound", "sell", "sold", "withdrawal", "withdraw", "send", "sent", "debit", "outgoing", "payment"]


def _norm_header(header: str) -> str:
    text = re.sub(r"[^a-z0-9äöüß ]+", " ", (header or "").strip().casefold())
    return re.sub(r"\s+", " ", text).strip()


def infer_mapping(headers: Sequence[str]) -> dict:
    """Best-effort guess of a mapping spec from column headers.

    Returns ``{"spec", "detected", "confident"}`` where ``detected`` is a list of
    ``{"column", "field"}`` (for a human-readable summary) and ``confident`` is
    True only when a date column and an amount layout were recognized. When not
    confident the spec is incomplete (the caller should fall back to the example
    template or manual mapping rather than importing).
    """
    norm = {header: _norm_header(header) for header in headers}
    used: set[str] = set()
    detected: list[dict[str, str]] = []

    def take(aliases: set[str]) -> str | None:
        for header in headers:
            if header in used:
                continue
            if norm[header] in aliases:
                used.add(header)
                return header
        return None

    date_col = take(_DATE_ALIASES)
    sent_col = take(_SENT_ALIASES)
    received_col = take(_RECEIVED_ALIASES)
    amount_col = None if (sent_col and received_col) else take(_AMOUNT_ALIASES)
    direction_col = take(_DIRECTION_ALIASES)
    fee_col = take(_FEE_ALIASES)
    txid_col = take(_TXID_ALIASES)
    currency_col = take(_FIAT_CURRENCY_ALIASES)
    rate_col = take(_FIAT_RATE_ALIASES)
    value_col = take(_FIAT_VALUE_ALIASES)
    note_col = take(_NOTE_ALIASES)
    kind_col = take(_KIND_ALIASES)
    counterparty_col = take(_COUNTERPARTY_ALIASES)

    spec: dict[str, Any] = {
        "version": SPEC_VERSION,
        "name": "",
        "asset": "BTC",
        "delimiter": None,
        "encoding": DEFAULT_ENCODING,
        "skip_rows": 0,
        "timestamp": {"column": date_col or "", "format": None, "timezone": "UTC"},
        "fields": {},
        "filters": [],
    }
    if date_col:
        detected.append({"column": date_col, "field": "date"})

    if sent_col and received_col:
        spec["amount"] = {
            "mode": "split",
            "inbound_column": received_col,
            "outbound_column": sent_col,
            "unit": "btc",
        }
        detected.append({"column": received_col, "field": "received"})
        detected.append({"column": sent_col, "field": "sent"})
        amount_ok = True
    elif amount_col and direction_col:
        spec["amount"] = {
            "mode": "absolute",
            "column": amount_col,
            "unit": "btc",
            "direction": {
                "column": direction_col,
                "inbound_values": list(_INFER_INBOUND),
                "outbound_values": list(_INFER_OUTBOUND),
                "default": None,
            },
        }
        detected.append({"column": amount_col, "field": "amount"})
        detected.append({"column": direction_col, "field": "direction"})
        amount_ok = True
    elif amount_col:
        spec["amount"] = {"mode": "signed", "column": amount_col, "unit": "btc"}
        detected.append({"column": amount_col, "field": "amount"})
        amount_ok = True
    else:
        spec["amount"] = {"mode": "signed", "column": "", "unit": "btc"}
        amount_ok = False

    if fee_col:
        spec["fee"] = {"column": fee_col, "unit": "btc"}
        detected.append({"column": fee_col, "field": "fee"})
    if txid_col:
        spec["txid"] = {"column": txid_col}
        detected.append({"column": txid_col, "field": "txid"})
    if note_col:
        spec["fields"]["description"] = {"column": note_col}
        detected.append({"column": note_col, "field": "description"})
    if kind_col:
        spec["fields"]["kind"] = {"column": kind_col}
        detected.append({"column": kind_col, "field": "kind"})
    if counterparty_col:
        spec["fields"]["counterparty"] = {"column": counterparty_col}
        detected.append({"column": counterparty_col, "field": "counterparty"})
    if rate_col or value_col or currency_col:
        pricing: dict[str, Any] = {"source_kind": DEFAULT_SOURCE_KIND, "decimal_separator": "."}
        if currency_col:
            pricing["fiat_currency"] = {"column": currency_col}
            detected.append({"column": currency_col, "field": "fiat_currency"})
        if rate_col:
            pricing["fiat_rate"] = {"column": rate_col}
            detected.append({"column": rate_col, "field": "fiat_rate"})
        if value_col:
            pricing["fiat_value"] = {"column": value_col}
            detected.append({"column": value_col, "field": "fiat_value"})
        spec["pricing"] = pricing

    confident = bool(date_col) and amount_ok
    return {"spec": spec, "detected": detected, "confident": confident}


def example_csv() -> str:
    """A small fill-in template (canonical-friendly headers + sample BTC rows)."""
    import io as _io

    buffer = _io.StringIO()
    writer = csv.writer(buffer)
    writer.writerow(EXAMPLE_HEADERS)
    for row in EXAMPLE_ROWS:
        writer.writerow(row)
    return buffer.getvalue()
