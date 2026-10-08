"""The ledger request builder (schema version 2, plan 20 phase 4).

``build_request`` turns one finalized custody projection into the engine's
``ledger`` request (``tax-engine/core/src/ledger/request.rs``). Python keeps
the jobs that classify evidence and check the configuration; the engine
derives everything else from the facts sent here, including every marker
value (``docs/plan/20-kassiber-tax-engine.md``, design §1.1 and §3):

- the configuration checks the phase-3 adapter ran before its first engine
  call, with the same exception classes and messages: the label tokens, the
  policy and its currency, and the INI parse of the names RP2's membership
  checks accept;
- per row: the normalized kind, the stored rate and value as exact decimals,
  coarse pricing, the text, the Austrian overrides, privacy-hop evidence,
  conflict and pending holds (over all rows of the row's asset), chain-order
  evidence, and the parsed timestamp;
- per pair: the move's legs and atomic bundle, and a cross-asset pair's
  legs, policy, and whether it is a Bitcoin rail.

The builder runs on every non-empty book before routing, so it fails closed
on the shapes ``compile_finalized_tax_projection`` cannot produce
(``INVARIANTS``, design §3.5) with an ``EngineContractError`` naming the
shape, before the walk. The oracle quarantined those through dead branches
or failed later (listed difference LD-5).

Decimals cross the boundary in the canonical form ``[-]<digits>E<exp>``
(``encode_decimal``); timestamps as ``timestamp_fields`` parses them, the way
RP2 sees them.
"""

from __future__ import annotations

import configparser
import json
import re
from datetime import datetime, timedelta, timezone
from decimal import Decimal, InvalidOperation
from functools import lru_cache
from typing import Any, Iterable, Mapping, Optional, Sequence
from zoneinfo import ZoneInfo

from ...errors import AppError
from ...tax_policy import build_tax_policy, profile_cost_basis_pool_scope, profile_value
from ...transfers import _SYNTHETIC_TRANSFER_ID_PREFIXES, is_bitcoin_rail_pair
from .. import pricing
from ..ownership_transfers import detect_conflicting_spend_ids, detect_pending_onchain_ids
from ..privacy_hops import privacy_hop_evidence_from_row
from ..transaction_kinds import normalized_transaction_kind
from .base import TaxEngineLedgerInputs
from .errors import EngineContractError, EngineValueError

SCHEMA_VERSION = 2

#: ``sys.maxsize``: any longer holding period behaves identically.
MAX_LONG_TERM_DAYS = 2**63 - 1

_EPOCH = datetime(1970, 1, 1, tzinfo=timezone.utc)
_ONE_US = timedelta(microseconds=1)
_ONE_SECOND = timedelta(seconds=1)

_CANONICAL = re.compile(r"-?(0|[1-9][0-9]*)E(0|-?[1-9][0-9]*)\Z")

_ISO = re.compile(
    r"(?P<year>\d{4})-(?P<month>\d{2})-(?P<day>\d{2})"
    r"(?:[T ](?P<hour>\d{2}):(?P<minute>\d{2})"
    r"(?::(?P<second>\d{2})(?:[.,](?P<fraction>\d+))?)?"
    r"(?P<offset>Z|z|[+-]\d{2}(?::?\d{2})?)?)?\Z"
)

# ISO 4217 alpha-3 codes, as RP2's ``pycountry`` lookup accepts them
# (case-insensitively); ``tests/tax_oracle/test_oracle_contract.py`` keeps
# them in step.
ISO_4217 = frozenset(
    """
    AED AFN ALL AMD AOA ARS AUD AWG AZN BAM BBD BDT BHD BIF BMD BND BOB BOV BRL
    BSD BTN BWP BYN BZD CAD CDF CHE CHF CHW CLF CLP CNY COP COU CRC CUP CVE CZK
    DJF DKK DOP DZD EGP ERN ETB EUR FJD FKP GBP GEL GHS GIP GMD GNF GTQ GYD HKD
    HNL HTG HUF IDR ILS INR IQD IRR ISK JMD JOD JPY KES KGS KHR KMF KPW KRW KWD
    KYD KZT LAK LBP LKR LRD LSL LYD MAD MDL MGA MKD MMK MNT MOP MRU MUR MVR MWK
    MXN MXV MYR MZN NAD NGN NIO NOK NPR NZD OMR PAB PEN PGK PHP PKR PLN PYG QAR
    RON RSD RUB RWF SAR SBD SCR SDG SEK SGD SHP SLE SOS SRD SSP STN SVC SYP SZL
    THB TJS TMT TND TOP TRY TTD TWD TZS UAH UGX USD USN UYI UYU UYW UZS VED VES
    VND VUV WST XAD XAF XAG XAU XBA XBB XBC XBD XCD XCG XDR XOF XPD XPF XPT XSU
    XTS XUA XXX YER ZAR ZMW ZWG
    """.split()
)

# RP2 read the configuration names from an INI file, so these characters
# could not appear in a wallet, profile, or asset label (an INI-syntax rule
# phase 4 keeps, with its messages).
CONFIG_FORBIDDEN_CHARACTERS = frozenset({",", "\n", "\r", "[", "]", "=", ":", "#", ";"})

REVIEWED_PRIVACY_PAIR_KIND = "custody-reviewed-coinjoin"
DIRECTIONS = ("inbound", "outbound")
REGIME_OVERRIDES = ("alt", "neu")
HOLD_CONFLICTING_SPEND = "conflicting_spend"
HOLD_PENDING_ONCHAIN = "pending_onchain_confirmation"

# The shapes a finalized projection cannot have (design §3.5), in the order
# the builder checks them. Each refusal names one.
NON_GLOBAL_POOL_SCOPE = "non_global_pool_scope"
WALLET_REF_ID_MISMATCH = "wallet_ref_id_mismatch"
NON_SYNTHETIC_ROW_ID = "non_synthetic_row_id"
MISSING_ANCHOR = "missing_anchor"
UNKNOWN_WALLET = "unknown_wallet"
UNKNOWN_DIRECTION = "unknown_direction"
NON_INTEGER_MSAT = "non_integer_msat"
NEGATIVE_FEE = "negative_fee"
NON_POSITIVE_AMOUNT = "non_positive_amount"
AMOUNT_INCLUDES_FEE = "amount_includes_fee"
CUSTODY_COMPONENT_FIELDS = "custody_component_fields"
CORRUPT_DECIMAL_TEXT = "corrupt_decimal_text"
GROUP_BLOCK_ROWS = "group_block_rows"
UNREVIEWED_PRIVACY_PAIR = "unreviewed_privacy_pair"
SHARED_PAIR_LEG = "shared_pair_leg"
UNEQUAL_MOVE_AMOUNTS = "unequal_move_amounts"

INVARIANTS = (
    # A wallet-scoped pool raises the oracle's own AppError instead
    # (``tax_policy.normalize_cost_basis_pool_scope``), so it is compared
    # like any other failure.
    NON_GLOBAL_POOL_SCOPE,
    WALLET_REF_ID_MISMATCH,
    NON_SYNTHETIC_ROW_ID,
    MISSING_ANCHOR,
    UNKNOWN_WALLET,
    UNKNOWN_DIRECTION,
    NON_INTEGER_MSAT,
    NEGATIVE_FEE,
    NON_POSITIVE_AMOUNT,
    AMOUNT_INCLUDES_FEE,
    CUSTODY_COMPONENT_FIELDS,
    CORRUPT_DECIMAL_TEXT,
    GROUP_BLOCK_ROWS,
    UNREVIEWED_PRIVACY_PAIR,
    SHARED_PAIR_LEG,
    UNEQUAL_MOVE_AMOUNTS,
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
    # Join the digits directly: int() would reject coefficients longer than
    # Python's integer string-conversion limit.
    coefficient = "".join(str(digit) for digit in digits).lstrip("0") or "0"
    return f"{'-' if sign else ''}{coefficient}E{exponent}"


def decode_decimal(text: str) -> Decimal:
    """Parse the canonical form back into the identical ``Decimal``."""
    if not isinstance(text, str) or not _CANONICAL.match(text):
        raise ValueError(f"not a canonical engine decimal: {text!r}")
    # Construction from a string is exact; no context rounding applies.
    return Decimal(text)


def decimal_text(*values: Any) -> Optional[str]:
    """The first non-empty stored value as the request carries it.

    ``pricing.decimal_from_exact`` reads the same value. A finite one is
    canonical; a non-finite one (``NaN``, ``-Infinity``, ...) is CPython's
    text for it, so the engine sees what the stored row held.
    """
    for value in values:
        if value not in (None, ""):
            number = Decimal(str(value))
            return encode_decimal(number) if number.is_finite() else str(number)
    return None


def _corrupt_decimal(*values: Any) -> Optional[Any]:
    # The first non-empty value is the one the adapter parses.
    for value in values:
        if value in (None, ""):
            continue
        try:
            Decimal(str(value))
        except InvalidOperation:
            return value
        return None
    return None


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
# Configuration


def _get(row: Any, key: str, default: Any = None) -> Any:
    if row is None:
        return default
    if hasattr(row, "keys") and key not in row.keys():
        return default
    if hasattr(row, "get"):
        return row.get(key, default)
    return row[key]


def _profile_str(profile: Mapping[str, Any], key: str) -> str:
    if hasattr(profile, "keys") and key in profile.keys():
        value = profile[key]
        if value is None:
            return ""
        return str(value).strip()
    return ""


def _config_token(value: Any, field: str) -> str:
    text = str(value or "").strip()
    if not text:
        raise AppError(
            "RP2 configuration value is required",
            code="validation",
            details={"field": field},
            retryable=False,
        )
    forbidden = sorted(char for char in CONFIG_FORBIDDEN_CHARACTERS if char in text)
    if forbidden:
        raise AppError(
            f"RP2 configuration {field} contains unsupported characters",
            code="validation",
            hint=(
                "Rename the wallet, profile, or asset label to remove commas, "
                "newlines, brackets, comments, or key delimiters before processing reports."
            ),
            details={"field": field, "value": text, "characters": forbidden},
            retryable=False,
        )
    return text


def _string_set(field: str, section: configparser.SectionProxy) -> frozenset[str]:
    # RP2 read these fields from an INI file with a default ConfigParser:
    # ``%`` interpolation applies and each comma-separated element is
    # stripped. The same section is parsed from memory so the same values
    # pass or fail the same way; messages lack RP2's file-path prefix
    # (listed difference LD-3).
    text = section[field]
    if not text or not text.strip():
        raise EngineValueError(f"field '{field}' in section '{section.name}' cannot be empty")
    result: set[str] = set()
    for element in (value.strip() for value in text.split(",")):
        if not element:
            raise EngineValueError(f"field '{field}' in section '{section.name}' cannot contain empty elements")
        if element in result:
            raise EngineValueError(f"field '{field}' in section '{section.name}' contains duplicate elements: {element}")
        result.add(element)
    return frozenset(result)


def configuration_names(assets: Sequence[str], exchanges: Sequence[str], holders: Sequence[str]) -> dict[str, list[str]]:
    """The names RP2's membership checks accept, as its INI parser reads them."""
    parser = configparser.ConfigParser()
    parser.read_string(
        "[general]\n"
        f"assets = {', '.join(assets)}\n"
        f"exchanges = {', '.join(exchanges)}\n"
        f"holders = {', '.join(holders)}\n"
    )
    general = parser["general"]
    return {field: sorted(_string_set(field, general)) for field in ("assets", "exchanges", "holders")}


def _policy(profile: Mapping[str, Any]) -> dict[str, Any]:
    """The request's ``policy``, failing as the adapter's country objects did."""
    try:
        policy = build_tax_policy(profile)
    except ValueError as exc:
        raise AppError(str(exc)) from exc
    if policy.tax_country == "at":
        country: dict[str, Any] = {"kind": "at"}
    else:
        currency = policy.fiat_currency
        if currency.upper() not in ISO_4217:
            raise EngineValueError(f"Parameter 'currency_iso_code' has non-ISO-4217 format value {currency}.")
        country = {"kind": "generic", "long_term_days": min(policy.long_term_days, MAX_LONG_TERM_DAYS)}
    label = profile_value(profile, "gains_algorithm")
    method = str(label).strip().lower()
    return {
        "country": country,
        "method": method if method in set(policy.accounting_methods) else None,
        "method_label": str(label),
        "require_coarse_review": _requires_coarse_review(profile),
    }


def _requires_coarse_review(profile: Mapping[str, Any]) -> bool:
    # Coarse pricing is accepted by default; a profile that opts into
    # ``require_coarse_review`` quarantines it for review.
    if not hasattr(profile, "keys") or "require_coarse_review" not in profile.keys():
        return False
    value = profile["require_coarse_review"]
    if isinstance(value, str):
        return value.strip().lower() in {"1", "true", "yes", "on"}
    return bool(value)


def check_configuration(
    profile: Mapping[str, Any],
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]],
    rows: Sequence[Mapping[str, Any]],
) -> tuple[dict[str, Any], dict[str, list[str]]]:
    """The policy and configuration names, or the adapter's configuration failure.

    The checks run in the adapter's order: every labelled wallet (used or
    not), every asset, and the profile label as a token; at least one wallet
    and one asset; the policy, with the currency for a generic book; then the
    INI parse of the names.
    """
    wallet_labels = [str(item["label"]) for item in wallet_refs_by_id.values() if item.get("label") not in (None, "")]
    assets = {str(_get(row, "asset")) for row in rows}
    sorted_wallet_labels = sorted(_config_token(label, "wallet_label") for label in wallet_labels)
    sorted_assets = sorted(_config_token(asset, "asset") for asset in assets)
    holder_label = _config_token(_profile_str(profile, "label"), "profile_label")
    if not sorted_wallet_labels:
        raise AppError("RP2 configuration requires at least one wallet")
    if not sorted_assets:
        raise AppError("RP2 configuration requires at least one asset")
    policy = _policy(profile)
    return policy, configuration_names(sorted_assets, sorted_wallet_labels, [holder_label])


# ---------------------------------------------------------------------------
# Invariants


def _refuse(invariant: str, detail: str) -> EngineContractError:
    return EngineContractError("RequestError", f"{invariant}: {detail}", invariant=invariant)


def _is_msat(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def _check_row(row: Mapping[str, Any], wallet_ids: set[str]) -> None:
    row_id = str(_get(row, "id") or "")
    if not row_id.startswith(_SYNTHETIC_TRANSFER_ID_PREFIXES):
        raise _refuse(NON_SYNTHETIC_ROW_ID, f"row {row_id!r} is not a projection row")
    anchor = _get(row, "journal_transaction_id")
    if not isinstance(anchor, str) or not anchor:
        raise _refuse(MISSING_ANCHOR, f"row {row_id!r} has no journal transaction")
    if str(_get(row, "wallet_id")) not in wallet_ids:
        raise _refuse(UNKNOWN_WALLET, f"row {row_id!r} names a wallet the wallet references do not list")
    direction = _get(row, "direction")
    if direction not in DIRECTIONS:
        raise _refuse(UNKNOWN_DIRECTION, f"row {row_id!r} has direction {direction!r}")
    amount, fee = _get(row, "amount"), _get(row, "fee")
    if not _is_msat(amount) or not _is_msat(fee):
        raise _refuse(NON_INTEGER_MSAT, f"row {row_id!r} has amount {amount!r} and fee {fee!r}")
    if fee < 0:
        raise _refuse(NEGATIVE_FEE, f"row {row_id!r} has fee {fee}")
    if (direction == "inbound" and amount <= 0) or (direction == "outbound" and amount + fee <= 0):
        raise _refuse(NON_POSITIVE_AMOUNT, f"{direction} row {row_id!r} has amount {amount} and fee {fee}")
    if _get(row, "amount_includes_fee"):
        raise _refuse(AMOUNT_INCLUDES_FEE, f"row {row_id!r} includes its fee in its amount")
    if hasattr(row, "keys") and any(str(key).startswith("custody_component_") for key in row.keys()):
        raise _refuse(CUSTODY_COMPONENT_FIELDS, f"row {row_id!r} carries custody component fields")
    for field in ("fiat_rate", "fiat_value"):
        corrupt = _corrupt_decimal(_get(row, f"{field}_exact"), _get(row, field))
        if corrupt is not None:
            raise _refuse(CORRUPT_DECIMAL_TEXT, f"row {row_id!r} has {field} {corrupt!r}")


def _has_privacy_evidence(row: Mapping[str, Any]) -> bool:
    return privacy_hop_evidence_from_row(row) is not None


def _check_pairs(intra_pairs: Iterable[Mapping[str, Any]]) -> None:
    legs: set[str] = set()
    for pair in intra_pairs:
        out_row, in_row = pair["out"], pair["in"]
        name = f"{_get(out_row, 'id')!r} -> {_get(in_row, 'id')!r}"
        if pair.get("group_block_rows"):
            raise _refuse(GROUP_BLOCK_ROWS, f"move {name} blocks rows outside its pair")
        if (_has_privacy_evidence(out_row) or _has_privacy_evidence(in_row)) and pair.get(
            "kind"
        ) != REVIEWED_PRIVACY_PAIR_KIND:
            raise _refuse(UNREVIEWED_PRIVACY_PAIR, f"move {name} crosses a privacy boundary without review")
        for leg in (out_row, in_row):
            leg_id = str(_get(leg, "id"))
            if leg_id in legs:
                raise _refuse(SHARED_PAIR_LEG, f"row {leg_id!r} is a leg of two moves")
            legs.add(leg_id)
        if _get(out_row, "amount") != _get(in_row, "amount"):
            raise _refuse(UNEQUAL_MOVE_AMOUNTS, f"move {name} sends {_get(out_row, 'amount')!r} and receives {_get(in_row, 'amount')!r}")


def check_invariants(inputs: TaxEngineLedgerInputs) -> None:
    """Refuse a projection with a shape ``compile_finalized_tax_projection`` cannot produce."""
    wallet_refs = inputs.wallet_refs_by_id
    for key, ref in wallet_refs.items():
        if str(key) != str(_get(ref, "id")):
            raise _refuse(WALLET_REF_ID_MISMATCH, f"wallet reference {str(key)!r} has id {_get(ref, 'id')!r}")
    wallet_ids = {str(key) for key in wallet_refs}
    projection = inputs.finalized_tax_projection
    for row in projection.rows:
        _check_row(row, wallet_ids)
    _check_pairs(projection.intra_pairs)


# ---------------------------------------------------------------------------
# The request


def _text(value: Any) -> Optional[str]:
    if value is None:
        return None
    return value if isinstance(value, str) else str(value)


def _holds(rows: Sequence[Mapping[str, Any]]) -> dict[str, str]:
    """Each held row's hold, detected over all projected rows of its asset."""
    rows_by_asset: dict[str, list[Mapping[str, Any]]] = {}
    for row in rows:
        rows_by_asset.setdefault(str(_get(row, "asset")), []).append(row)
    holds: dict[str, str] = {}
    for asset_rows in rows_by_asset.values():
        conflicts = detect_conflicting_spend_ids(asset_rows)
        pending = detect_pending_onchain_ids(asset_rows)
        for row in asset_rows:
            row_id = str(row["id"])
            if row_id in conflicts:
                holds[row_id] = HOLD_CONFLICTING_SPEND
            elif row_id in pending:
                holds[row_id] = HOLD_PENDING_ONCHAIN
    return holds


def _chain_order(evidence: Any) -> Optional[dict[str, Any]]:
    if evidence is None:
        return None
    return {
        "scope": [str(part) for part in evidence.scope],
        "parents": [[str(part) for part in parent] for parent in sorted(evidence.parents)],
    }


def _row_text(row: Mapping[str, Any]) -> Optional[str]:
    for key in ("note", "description", "kind"):
        value = _get(row, key)
        if value:
            return _text(value)
    return None


def _request_row(row: Mapping[str, Any], holds: Mapping[str, str]) -> dict[str, Any]:
    row_id = str(row["id"])
    regime_override = _get(row, "at_regime_override")
    category_override = _get(row, "at_category_override")
    return {
        "id": row_id,
        "anchor": row["journal_transaction_id"],
        "asset": str(_get(row, "asset")),
        "wallet_id": str(row["wallet_id"]),
        "direction": row["direction"],
        "ts": timestamp_fields(_get(row, "occurred_at")),
        "amount_msat": row["amount"],
        "fee_msat": row["fee"],
        "rate": decimal_text(_get(row, "fiat_rate_exact"), _get(row, "fiat_rate")),
        "value": decimal_text(_get(row, "fiat_value_exact"), _get(row, "fiat_value")),
        "coarse": _get(row, "pricing_quality") == pricing.QUALITY_COARSE_FALLBACK,
        "kind": normalized_transaction_kind(row),
        "text": _row_text(row),
        "regime_override": regime_override if regime_override in REGIME_OVERRIDES else None,
        "category_override": str(category_override) if category_override else None,
        "taxability_off": _get(row, "taxability_override") == 0,
        "privacy_hop": _has_privacy_evidence(row),
        "hold": holds.get(row_id),
        "chain_order": _chain_order(_get(row, "custody_chain_order")),
    }


def _pair_group_id(pair: Mapping[str, Any]) -> Optional[str]:
    raw = pair.get("group_id")
    if raw in (None, ""):
        return None
    return str(raw)


def _request_cross_pair(pair: Mapping[str, Any]) -> dict[str, Any]:
    return {
        "pair_id": _text(pair.get("pair_id")),
        "out_id": str(pair["out_id"]),
        "in_id": str(pair["in_id"]),
        "out_transaction_id": _text(pair.get("out_transaction_id")),
        "in_transaction_id": _text(pair.get("in_transaction_id")),
        "out_asset": _text(pair.get("out_asset")),
        "in_asset": _text(pair.get("in_asset")),
        "policy": _text(pair.get("policy")),
        "bitcoin_rail": is_bitcoin_rail_pair(pair.get("out_asset"), pair.get("in_asset")),
    }


def build_request(
    profile: Mapping[str, Any],
    inputs: TaxEngineLedgerInputs,
    *,
    operation: str = "ledger",
) -> dict[str, Any]:
    """The ``ledger`` (or ``prepare``) request for a non-empty finalized projection.

    Raises what the adapter raised before its first engine call (the
    configuration checks, the crossed-asset move assertion, and a pool scope
    the policy does not allow), and ``EngineContractError`` for a shape the
    projection cannot have.
    """
    projection = inputs.finalized_tax_projection
    rows = projection.rows
    policy, config = check_configuration(profile, inputs.wallet_refs_by_id, rows)
    for pair in projection.intra_pairs:
        if str(_get(pair["out"], "asset")) != str(_get(pair["in"], "asset")):
            raise AssertionError("finalized same-asset move pair crossed assets")
    profile_cost_basis_pool_scope(profile)
    check_invariants(inputs)
    holds = _holds(rows)
    return {
        "schema_version": SCHEMA_VERSION,
        "operation": operation,
        "policy": policy,
        "config": config,
        "holder": _text(profile_value(profile, "label")),
        "wallets": [{"id": str(ref["id"]), "label": _text(ref.get("label"))} for ref in inputs.wallet_refs_by_id.values()],
        "rows": [_request_row(row, holds) for row in rows],
        "intra_pairs": [
            {
                "out": str(pair["out"]["id"]),
                "in": str(pair["in"]["id"]),
                "source": _text(pair.get("source")),
                "group_id": _pair_group_id(pair),
            }
            for pair in projection.intra_pairs
        ],
        "cross_asset_pairs": [_request_cross_pair(pair) for pair in projection.cross_asset_pairs],
    }


def dumps(document: Mapping[str, Any]) -> str:
    """A request as the engine reads it: compact, deterministic JSON."""
    return json.dumps(document, ensure_ascii=False, separators=(",", ":"))


__all__ = [
    "INVARIANTS",
    "ISO_4217",
    "MAX_LONG_TERM_DAYS",
    "SCHEMA_VERSION",
    "build_request",
    "check_configuration",
    "check_invariants",
    "configuration_names",
    "decimal_text",
    "decode_decimal",
    "dumps",
    "encode_decimal",
    "parse_timestamp",
    "timestamp_fields",
]
