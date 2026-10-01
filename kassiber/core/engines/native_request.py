"""Request documents for Kassiber's native tax engine (``kassiber_tax``).

This module turns RP2-style constructor arguments into the engine's JSON
request format (schema version 1, see ``docs/reference/tax-engine.md``) and
renders the values RP2 itself would render. It uses only the standard
library and never imports RP2, so the native backend can run without it.

* Decimals cross the boundary as ``[-]<digits>E<exponent>``, which keeps the
  coefficient and exponent exactly (``encode_decimal``/``decode_decimal``).
* Timestamps are parsed once here, the way RP2 sees them: the absolute
  instant, the date and year in the timestamp's own offset, the
  Europe/Vienna date, ``str(datetime)``, and ``datetime.isoformat()``.
* ``entry_text`` reproduces ``str(transaction)``, which RP2 embeds in its
  negative-balance and duplicate-entry messages.

Timestamps are ISO 8601 strings with an offset, the only form Kassiber
stores. RP2 parses with ``dateutil``, which also accepts free-form dates;
for those this module reports dateutil's "Unknown string format" error.
"""

from __future__ import annotations

import json
import re
from datetime import datetime, timedelta, timezone
from decimal import ROUND_HALF_EVEN, Context, Decimal, localcontext
from functools import lru_cache
from typing import Any, Mapping, Optional
from zoneinfo import ZoneInfo

SCHEMA_VERSION = 1

#: RP2's context: 32 significant digits, round-half-even.
RP2_CONTEXT = Context(prec=32, rounding=ROUND_HALF_EVEN)

#: ``sys.maxsize``: any longer holding period behaves identically.
MAX_LONG_TERM_DAYS = 2**63 - 1

_EPOCH = datetime(1970, 1, 1, tzinfo=timezone.utc)
_ONE_US = timedelta(microseconds=1)
_ONE_SECOND = timedelta(seconds=1)

_CLASS_NAMES = {"in": "InTransaction", "out": "OutTransaction", "intra": "IntraTransaction"}

_CANONICAL = re.compile(r"-?(0|[1-9][0-9]*)E(0|-?[1-9][0-9]*)\Z")

_ISO = re.compile(
    r"(?P<year>\d{4})-(?P<month>\d{2})-(?P<day>\d{2})"
    r"(?:[T ](?P<hour>\d{2}):(?P<minute>\d{2})"
    r"(?::(?P<second>\d{2})(?:[.,](?P<fraction>\d+))?)?"
    r"(?P<offset>Z|z|[+-]\d{2}(?::?\d{2})?)?)?\Z"
)


# ---------------------------------------------------------------------------
# Decimals


def encode_decimal(value: Decimal) -> str:
    """Return the canonical boundary form of a finite ``Decimal``."""
    if not isinstance(value, Decimal):
        raise TypeError(f"expected a Decimal, got {value!r}")
    if not value.is_finite():
        raise ValueError(f"the engine accepts only finite decimals, got {value}")
    sign, digits, exponent = value.as_tuple()
    coefficient = "".join(str(digit) for digit in digits) or "0"
    return f"{'-' if sign else ''}{int(coefficient)}E{exponent}"


def encode_optional_decimal(value: Optional[Decimal]) -> Optional[str]:
    """``encode_decimal``, passing ``None`` through as ``null``."""
    return None if value is None else encode_decimal(value)


def decode_decimal(text: str) -> Decimal:
    """Parse the canonical form back into the identical ``Decimal``."""
    if not isinstance(text, str) or not _CANONICAL.match(text):
        raise ValueError(f"not a canonical engine decimal: {text!r}")
    # Construction from a string is exact; no context rounding applies.
    return Decimal(text)


def decode_optional_decimal(text: Optional[str]) -> Optional[Decimal]:
    """``decode_decimal``, passing ``null`` through as ``None``."""
    return None if text is None else decode_decimal(text)


# ---------------------------------------------------------------------------
# Timestamps


@lru_cache(maxsize=1)
def _vienna() -> ZoneInfo:
    return ZoneInfo("Europe/Vienna")


def parse_timestamp(value: str) -> datetime:
    """Parse ``value`` as RP2 does; raise ``ValueError`` with its message.

    Fractional seconds beyond microseconds are truncated, ``Z`` and
    ``+00:00`` both mean UTC, and the parsed offset is kept. A value
    without an offset parses to a naive datetime.
    """
    match = _ISO.match(value)
    if match is None:
        raise ValueError(f"Unknown string format: {value}")
    parts = match.groupdict()
    fraction = (parts["fraction"] or "")[:6].ljust(6, "0")
    tzinfo: Optional[timezone] = None
    offset = parts["offset"]
    if offset is not None:
        if offset in ("Z", "z"):
            tzinfo = timezone.utc
        else:
            digits = offset[1:].replace(":", "")
            minutes = int(digits[:2]) * 60 + (int(digits[2:4]) if len(digits) > 2 else 0)
            delta = timedelta(minutes=minutes if offset[0] == "+" else -minutes)
            tzinfo = timezone.utc if not delta else timezone(delta)
    try:
        return datetime(
            int(parts["year"]),
            int(parts["month"]),
            int(parts["day"]),
            int(parts["hour"] or 0),
            int(parts["minute"] or 0),
            int(parts["second"] or 0),
            int(fraction),
            tzinfo=tzinfo,
        )
    except ValueError as exc:
        raise ValueError(f"{exc}: {value}") from None


def timestamp_fields(value: Any) -> dict[str, Any]:
    """The ``ts`` object of an entry, or the error RP2 raises for ``value``."""
    if not isinstance(value, str):
        return {
            "raw": str(value),
            "error": f"Parameter 'timestamp' has non-string value {value!r}",
            "error_class": "TypeError",
        }
    try:
        parsed = parse_timestamp(value)
    except ValueError as exc:
        return {"raw": value, "error": f"Error parsing parameter 'timestamp': {exc}"}
    offset = parsed.utcoffset()
    if offset is None:
        return {"raw": value, "error": f"Parameter 'timestamp' value has no timezone info: {value}"}
    try:
        vienna: Optional[str] = parsed.astimezone(_vienna()).date().isoformat()
    except (OverflowError, ValueError):
        vienna = None
    return {
        "raw": value,
        "us": (parsed - _EPOCH) // _ONE_US,
        "offset_s": offset // _ONE_SECOND,
        "date": parsed.date().isoformat(),
        "year": parsed.year,
        "vienna_date": vienna,
        "display": str(parsed),
        "iso": parsed.isoformat(),
    }


# ---------------------------------------------------------------------------
# Entries


def _decimal_field(name: str, value: Any) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, Decimal):
        raise TypeError(f"Parameter '{name}' has non-RP2Decimal value {value!r}")
    return encode_decimal(value)


def _row(row: Any) -> int:
    if isinstance(row, bool) or not isinstance(row, int):
        raise TypeError(f"Parameter 'row' has non-integer value {row!r}")
    return row


def _unique_id(unique_id: Any) -> Optional[str]:
    if unique_id is None:
        return None
    if not isinstance(unique_id, (str, int, float)):
        raise TypeError(f"Parameter 'unique_id' has non-string value {unique_id!r}")
    return str(unique_id)


def _notes(notes: Any) -> Optional[str]:
    if not notes:
        return None
    if not isinstance(notes, str):
        raise TypeError(f"Parameter 'notes' has non-string value {notes!r}")
    return notes


def _string(name: str, value: Any) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, str):
        raise TypeError(f"Parameter '{name}' has non-string value {value!r}")
    return value


def in_entry(
    *,
    timestamp: Any,
    asset: str,
    exchange: Any,
    holder: Any,
    transaction_type: Any,
    spot_price: Optional[Decimal],
    crypto_in: Optional[Decimal],
    crypto_fee: Optional[Decimal] = None,
    fiat_in_no_fee: Optional[Decimal] = None,
    fiat_in_with_fee: Optional[Decimal] = None,
    fiat_fee: Optional[Decimal] = None,
    row: int,
    unique_id: Any = None,
    notes: Any = None,
) -> dict[str, Any]:
    """An ``in`` entry for ``InTransaction(...)`` called with these arguments."""
    return {
        "kind": "in",
        "row": _row(row),
        "unique_id": _unique_id(unique_id),
        "asset": asset,
        "transaction_type": _string("transaction_type", transaction_type),
        "notes": _notes(notes),
        "exchange": _string("exchange", exchange),
        "holder": _string("holder", holder),
        "spot_price": _decimal_field("spot_price", spot_price),
        "crypto_in": _decimal_field("crypto_in", crypto_in),
        "crypto_fee": _decimal_field("crypto_fee", crypto_fee),
        "fiat_in_no_fee": _decimal_field("fiat_in_no_fee", fiat_in_no_fee),
        "fiat_in_with_fee": _decimal_field("fiat_in_with_fee", fiat_in_with_fee),
        "fiat_fee": _decimal_field("fiat_fee", fiat_fee),
        "ts": timestamp_fields(timestamp),
    }


def out_entry(
    *,
    timestamp: Any,
    asset: str,
    exchange: Any,
    holder: Any,
    transaction_type: Any,
    spot_price: Optional[Decimal],
    crypto_out_no_fee: Optional[Decimal],
    crypto_fee: Optional[Decimal],
    crypto_out_with_fee: Optional[Decimal] = None,
    fiat_out_no_fee: Optional[Decimal] = None,
    fiat_fee: Optional[Decimal] = None,
    row: int,
    unique_id: Any = None,
    notes: Any = None,
) -> dict[str, Any]:
    """An ``out`` entry for ``OutTransaction(...)`` called with these arguments."""
    return {
        "kind": "out",
        "row": _row(row),
        "unique_id": _unique_id(unique_id),
        "asset": asset,
        "transaction_type": _string("transaction_type", transaction_type),
        "notes": _notes(notes),
        "exchange": _string("exchange", exchange),
        "holder": _string("holder", holder),
        "spot_price": _decimal_field("spot_price", spot_price),
        "crypto_out_no_fee": _decimal_field("crypto_out_no_fee", crypto_out_no_fee),
        "crypto_fee": _decimal_field("crypto_fee", crypto_fee),
        "crypto_out_with_fee": _decimal_field("crypto_out_with_fee", crypto_out_with_fee),
        "fiat_out_no_fee": _decimal_field("fiat_out_no_fee", fiat_out_no_fee),
        "fiat_fee": _decimal_field("fiat_fee", fiat_fee),
        "ts": timestamp_fields(timestamp),
    }


def intra_entry(
    *,
    timestamp: Any,
    asset: str,
    from_exchange: Any,
    from_holder: Any,
    to_exchange: Any,
    to_holder: Any,
    spot_price: Optional[Decimal],
    crypto_sent: Optional[Decimal],
    crypto_received: Optional[Decimal],
    row: int,
    unique_id: Any = None,
    notes: Any = None,
) -> dict[str, Any]:
    """An ``intra`` entry for ``IntraTransaction(...)`` called with these arguments."""
    return {
        "kind": "intra",
        "row": _row(row),
        "unique_id": _unique_id(unique_id),
        "asset": asset,
        "notes": _notes(notes),
        "from_exchange": _string("from_exchange", from_exchange),
        "from_holder": _string("from_holder", from_holder),
        "to_exchange": _string("to_exchange", to_exchange),
        "to_holder": _string("to_holder", to_holder),
        "spot_price": _decimal_field("spot_price", spot_price),
        "crypto_sent": _decimal_field("crypto_sent", crypto_sent),
        "crypto_received": _decimal_field("crypto_received", crypto_received),
        "ts": timestamp_fields(timestamp),
    }


def _value(derived: Mapping[str, Any], name: str) -> Decimal:
    value = derived[name]
    return decode_decimal(value) if isinstance(value, str) else value


def entry_text(entry: Mapping[str, Any], derived: Mapping[str, Any]) -> str:
    """``str(transaction)`` for a valid entry.

    ``derived`` holds the constructor's stored values, as ``check_entry``
    returns them (canonical strings) or as ``Decimal``s.
    """
    kind = entry["kind"]
    stamp = parse_timestamp(entry["ts"]["raw"]).strftime("%Y-%m-%d %H:%M:%S.%f %z")
    type_name = f"TransactionType.{str(derived['transaction_type']).upper()}"

    def fixed(name: str, places: int) -> str:
        with localcontext(RP2_CONTEXT):
            return format(_value(derived, name), f".{places}f")

    lines = [f"{_CLASS_NAMES[kind]}:", f"id={entry['row']}", f"timestamp={stamp}", f"asset={entry['asset']}"]
    if kind == "in":
        lines += [
            f"exchange={entry['exchange']}",
            f"holder={entry['holder']}",
            f"transaction_type={type_name}",
            f"spot_price={fixed('spot_price', 4)}",
            f"crypto_in={fixed('crypto_in', 8)}",
            f"fiat_fee={fixed('fiat_fee', 4)}",
            f"fiat_in_no_fee={fixed('fiat_in_no_fee', 4)}",
            f"fiat_in_with_fee={fixed('fiat_in_with_fee', 4)}",
            f"unique_id={derived['unique_id']}",
            f"is_taxable={derived['is_taxable']}",
            f"fiat_taxable_amount={fixed('fiat_taxable_amount', 4)}",
            "from_lot=",
            "to_lots=",
            f"cost_basis_timestamp={stamp}",
        ]
    elif kind == "out":
        lines += [
            f"exchange={entry['exchange']}",
            f"holder={entry['holder']}",
            f"transaction_type={type_name}",
            f"spot_price={fixed('spot_price', 4)}",
            f"crypto_out_no_fee={fixed('crypto_out_no_fee', 8)}",
            f"crypto_fee={fixed('crypto_fee', 8)}",
            f"unique_id={derived['unique_id']}",
            f"is_taxable={derived['is_taxable']}",
            f"fiat_taxable_amount={fixed('fiat_taxable_amount', 4)}",
        ]
    else:
        lines += [
            f"from_exchange={entry['from_exchange']}",
            f"from_holder={entry['from_holder']}",
            f"to_exchange={entry['to_exchange']}",
            f"to_holder={entry['to_holder']}",
            f"transaction_type={type_name}",
            f"spot_price={fixed('spot_price', 4)}",
            f"crypto_sent={fixed('crypto_sent', 8)}",
            f"crypto_received={fixed('crypto_received', 8)}",
            f"crypto_fee={fixed('crypto_fee', 8)}",
            f"fiat_fee={fixed('fiat_fee', 4)}",
            f"unique_id={derived['unique_id']}",
            f"is_taxable={derived['is_taxable']}",
            f"fiat_taxable_amount={fixed('fiat_taxable_amount', 4)}",
        ]
    return "\n  ".join(lines)


def with_text(entry: Mapping[str, Any], text: str) -> dict[str, Any]:
    """A copy of ``entry`` carrying its ``str(transaction)`` rendering."""
    return {**entry, "text": text}


# ---------------------------------------------------------------------------
# Requests


def check_entry_request(
    entry: Mapping[str, Any],
    *,
    assets: Optional[list[str]] = None,
    exchanges: Optional[list[str]] = None,
    holders: Optional[list[str]] = None,
) -> dict[str, Any]:
    """A ``check_entry`` request; membership is checked when names are given."""
    request: dict[str, Any] = {
        "schema_version": SCHEMA_VERSION,
        "operation": "check_entry",
        "entry": dict(entry),
    }
    if assets is not None or exchanges is not None or holders is not None:
        request["config"] = {
            "assets": list(assets or []),
            "exchanges": list(exchanges or []),
            "holders": list(holders or []),
        }
    return request


def country_spec(country: str, long_term_days: Optional[int] = None) -> dict[str, Any]:
    """The request's ``country``: ``generic`` with a holding period, or ``at``."""
    if country == "at":
        return {"kind": "at"}
    if country != "generic":
        raise ValueError(f"unsupported country {country!r}")
    if isinstance(long_term_days, bool) or not isinstance(long_term_days, int) or long_term_days < 0:
        raise ValueError(f"long_term_days must be a non-negative int, got {long_term_days!r}")
    return {"kind": "generic", "long_term_days": min(long_term_days, MAX_LONG_TERM_DAYS)}


def compute_request(
    *,
    method: str,
    assets: Mapping[str, list[Mapping[str, Any]]],
    country: Mapping[str, Any],
    operation: str = "compute",
) -> dict[str, Any]:
    """A ``compute`` (one asset), ``compute_multi``, or ``validate`` request."""
    return {
        "schema_version": SCHEMA_VERSION,
        "operation": operation,
        "country": dict(country),
        "method": method,
        "assets": [{"asset": asset, "entries": [dict(e) for e in entries]} for asset, entries in assets.items()],
    }


def dumps(document: Mapping[str, Any]) -> str:
    """Serialize a request compactly and deterministically."""
    return json.dumps(document, ensure_ascii=False, separators=(",", ":"))


def loads(text: str) -> dict[str, Any]:
    """Parse a response document."""
    document = json.loads(text)
    if document.get("schema_version") != SCHEMA_VERSION:
        raise ValueError(f"unexpected engine schema_version in {text[:80]!r}")
    return document
