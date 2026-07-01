"""Shared redaction helpers for runtime secret egress boundaries."""

from __future__ import annotations

import re
import secrets
from typing import Any


SENSITIVE_KEY_PARTS = (
    "api_key",
    "api-key",
    "auth_header",
    "auth-header",
    "auth_response",
    "cookie",
    "descriptor",
    "mnemonic",
    "password",
    "passphrase",
    "private",
    "recovery",
    "secret",
    "seed",
    "scan_key",
    "scankey",
    "scan_private",
    "scanprivate",
    "silent_payment",
    "silentpayment",
    "sp_descriptor",
    "spscan",
    "spspend",
    "token",
    "xprv",
)

_PRIVATE_KEY_RE = re.compile(r"\b(?:xprv|tprv|yprv|zprv|uprv|vprv)[1-9A-HJ-NP-Za-km-z]{20,}\b")
_EXTENDED_KEY_RE = re.compile(r"\b(?:xpub|tpub|ypub|zpub|upub|vpub)[1-9A-HJ-NP-Za-km-z]{20,}\b")
_DESCRIPTOR_RE = re.compile(r"\b(?:wpkh|sh|wsh|tr|pkh|combo)\([^)\n]{16,}\)", re.IGNORECASE)
_SP_DESCRIPTOR_RE = re.compile(r"\bsp\([^)\n]{8,}\)", re.IGNORECASE)
_SP_KEY_RE = re.compile(r"\b(?:t?spscan|t?spspend)1q[023456789acdefghjklmnpqrstuvwxyz]{8,}\b", re.IGNORECASE)
_SP_ADDRESS_RE = re.compile(r"\b(?:t?sp)1q[023456789acdefghjklmnpqrstuvwxyz]{20,}\b", re.IGNORECASE)
_BEARER_RE = re.compile(r"\b[Bb]earer\s+[A-Za-z0-9._~+/-]+=*")
_SK_SECRET_RE = re.compile(r"\bsk-[A-Za-z0-9._~+/-]{6,}\b")
_ASSIGNED_SECRET_RE = re.compile(
    r"(?P<key>\b(?:api[_-]?key|auth[_-]?header|cookie|descriptor|mnemonic|"
    r"passphrase|password|recovery[_-]?phrase|scan[_-]?key|secret|"
    r"seed(?:[_-]?(?:phrase|words))?|silent[_-]?payment[_-]?scan[_-]?key|"
    r"sp[_-]?descriptor|spscan|spspend|token)\b)"
    r"(?P<sep>\s*[:=]\s*)"
    r"(?P<quote>['\"]?)"
    r"(?P<value>[^'\"\s,;}{]+)",
    re.IGNORECASE,
)
_JSON_SECRET_RE = re.compile(
    r"(?P<prefix>['\"](?:api[_-]?key|auth[_-]?header|cookie|descriptor|mnemonic|"
    r"passphrase|password|recovery[_-]?phrase|scan[_-]?key|secret|"
    r"seed(?:[_-]?(?:phrase|words))?|silent[_-]?payment[_-]?scan[_-]?key|"
    r"sp[_-]?descriptor|spscan|spspend|token)"
    r"['\"]\s*:\s*['\"])"
    r"(?P<value>[^'\"]+)",
    re.IGNORECASE,
)
_RECOVERY_ASSIGNMENT_RE = re.compile(
    r"(?P<key>\b(?:mnemonic|recovery[_-]?phrase|seed(?:[_-]?(?:phrase|words))?)\b)"
    r"(?P<sep>\s*[:=]\s*)"
    r"(?P<quote>['\"]?)"
    r"(?P<value>[^,;}{]*?)"
    r"(?=(?:\s+[A-Za-z0-9_.-]+\s*[:=])|[,;}'\"]|$)",
    re.IGNORECASE,
)

# --- Operational pseudonymization (txids + amounts) -------------------------
# Unlike the secret floor above (credentials), txids and amounts are not
# secrets but ARE the wallet fingerprint: a single txid/amount lets an AI
# debugging session (or a public bug report) tie a log back to a real wallet.
# Txids are replaced with stable pseudonyms so cross-line transaction
# correlation survives. Amount pseudonyms include a runtime-only salt because
# exact amounts are low-entropy and otherwise reversible by dictionary search.
# {64,} (not {64}) so a >=65-hex run — two concatenated txids, a txid glued to
# trailing hex — still has a word boundary and gets pseudonymized as one token
# instead of slipping through the {64}\b gap.
_TXID_RE = re.compile(r"\b[0-9a-f]{64,}\b", re.IGNORECASE)
_AMOUNT_UNITS = (
    "BTC|XBT|LBTC|sats?|msats?|EUR|USD|CHF|GBP|JPY|CAD|AUD|NZD|SEK|NOK|DKK|PLN|CZK|HUF"
)
_AMOUNT_NUMBER = r"[+-]?(?:(?:\d{1,3}(?:[,_ .]\d{3})+)|\d+)(?:[.,]\d+)?"
_CURRENCY_SYMBOLS = "€$£¥₿"
_AMOUNT_PSEUDONYM_SALT = secrets.token_hex(16)
_AMOUNT_RE = re.compile(
    "|".join(
        (
            rf"\b(?:{_AMOUNT_UNITS})\s*{_AMOUNT_NUMBER}\b",
            rf"(?<![A-Za-z0-9]){_AMOUNT_NUMBER}\s*(?:{_AMOUNT_UNITS})\b",
            rf"[{_CURRENCY_SYMBOLS}]\s*{_AMOUNT_NUMBER}\b",
            rf"(?<![A-Za-z0-9]){_AMOUNT_NUMBER}\s*[{_CURRENCY_SYMBOLS}]",
        )
    ),
    re.IGNORECASE,
)
_AMOUNT_NUM_RE = re.compile(r"[+-]?\d[\d.,_ ]*\d|[+-]?\d")
_MARKET_RATE_PAIR_PREFIX_RE = re.compile(rf"(?:^|\b)(?:{_AMOUNT_UNITS})$", re.IGNORECASE)
# Glued/keyed sat amounts: the unit is part of an identifier (amount_sat=50000,
# fee_msat: 100000, value_sats=...), so the standalone-unit detector above never
# fires and the integer LOOKS protected while leaking. Match key=value / key:value
# where the key ends in a sat/msat unit and pseudonymize just the number.
_KEYED_AMOUNT_RE = re.compile(
    r"\b([A-Za-z][A-Za-z0-9_]*(?:msats?|sats?))\b(['\"]?\s*[:=]\s*['\"]?)([+-]?\d[\d.,_]*\d|[+-]?\d)",
    re.IGNORECASE,
)


def _stable_hash(value: str) -> str:
    """FNV-1a (32-bit), byte-for-byte identical to the webview `stableHash`."""
    h = 0x811C9DC5
    for ch in value:
        h = (h ^ (ord(ch) & 0xFFFFFFFF)) & 0xFFFFFFFF
        h = (h * 0x01000193) & 0xFFFFFFFF
    return format(h, "08x")


def _is_market_rate_tail(text: str, start: int) -> bool:
    if start <= 0 or text[start - 1] not in "/-":
        return False
    return bool(_MARKET_RATE_PAIR_PREFIX_RE.search(text[: start - 1].rstrip()))


def redact_operational_text(value: str) -> str:
    """Pseudonymize txids and unit-tagged amounts in free text crossing egress.

    A txid becomes ``txid#<fnv>`` and a unit-tagged amount becomes
    ``amount#<salted-fnv>``. Addresses/paths are intentionally left readable
    (the operational tier keeps them for debugging). Bare unit-less integers
    cannot be safely auto-detected without context and are left as-is; structure
    such values into typed log fields instead of interpolating them into
    messages.
    """
    text = _TXID_RE.sub(lambda m: f"txid#{_stable_hash(m.group(0).lower())}", value)

    def _amount_pseudonym(num: str, unit: str) -> str:
        key = f"{_AMOUNT_PSEUDONYM_SALT}|{re.sub(r'[,_ ]', '', num)}|{unit.lower()}"
        return f"amount#{_stable_hash(key)}"

    def _keyed_amount(match: "re.Match[str]") -> str:
        key, sep, num = match.group(1), match.group(2), match.group(3)
        unit = "msat" if "msat" in key.lower() else "sat"
        return f"{key}{sep}{_amount_pseudonym(num, unit)}"

    # Keyed amounts first (amount_sat=50000); their numbers are then gone before
    # the standalone-unit pass runs, so neither double-masks the other.
    text = _KEYED_AMOUNT_RE.sub(_keyed_amount, text)

    def _amount(match: "re.Match[str]") -> str:
        token = match.group(0)
        start = match.start()
        # A market-rate tail ("BTC/EUR 64000.12") is public data, not the
        # user's amount; its number is preceded by the pair separator.
        if _is_market_rate_tail(text, start):
            return token
        num_match = _AMOUNT_NUM_RE.search(token)
        num = num_match.group(0) if num_match else token
        unit = re.sub(r"\s+", "", token.replace(num, "", 1))
        return _amount_pseudonym(num, unit)

    return _AMOUNT_RE.sub(_amount, text)


def redact_operational_value(value: Any, *, depth: int = 0) -> Any:
    """Recursively pseudonymize txids/amounts in string leaves of a structure.

    Companion to `redact_secret_value` (which scrubs secret KEYS): this scrubs
    operational VALUES so a txid/amount riding inside `error.details`
    (e.g. a backend stderr blob or a `response_preview`) is pseudonymized before
    the envelope reaches the UI or a disk write, not left for the read-back
    render step.
    """
    if depth > 8:
        return value
    if isinstance(value, dict):
        return {key: redact_operational_value(item, depth=depth + 1) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [redact_operational_value(item, depth=depth + 1) for item in value]
    if isinstance(value, str):
        return redact_operational_text(value)
    return value


def is_sensitive_key(key: str) -> bool:
    lowered = key.lower().replace("-", "_")
    return any(part.replace("-", "_") in lowered for part in SENSITIVE_KEY_PARTS)


def redact_secret_text(value: str) -> str:
    """Redact secret-shaped material from text that may cross a trust boundary."""

    text = _RECOVERY_ASSIGNMENT_RE.sub(r"\g<key>\g<sep>\g<quote>[redacted]", value)
    text = _PRIVATE_KEY_RE.sub("[redacted-private-key]", text)
    text = _EXTENDED_KEY_RE.sub("[redacted-extended-key]", text)
    text = _SP_DESCRIPTOR_RE.sub("[redacted-silent-payment-descriptor]", text)
    text = _SP_KEY_RE.sub("[redacted-silent-payment-key]", text)
    text = _SP_ADDRESS_RE.sub("[redacted-silent-payment-address]", text)
    text = _DESCRIPTOR_RE.sub("[redacted-descriptor]", text)
    text = _BEARER_RE.sub("Bearer [redacted]", text)
    text = _JSON_SECRET_RE.sub(r"\g<prefix>[redacted]", text)
    text = _ASSIGNED_SECRET_RE.sub(r"\g<key>\g<sep>\g<quote>[redacted]", text)
    text = _SK_SECRET_RE.sub("[redacted-secret]", text)
    return text


def redact_secret_value(value: Any, *, depth: int = 0) -> Any:
    """Recursively redact known secret keys and secret-looking string values."""

    if depth > 8:
        return "[truncated]"
    if isinstance(value, dict):
        redacted: dict[Any, Any] = {}
        for key, item in value.items():
            key_text = str(key)
            if is_sensitive_key(key_text):
                redacted[key] = "[redacted]"
            else:
                redacted[key] = redact_secret_value(item, depth=depth + 1)
        return redacted
    if isinstance(value, list):
        return [redact_secret_value(item, depth=depth + 1) for item in value]
    if isinstance(value, tuple):
        return [redact_secret_value(item, depth=depth + 1) for item in value]
    if isinstance(value, str):
        return redact_secret_text(value)
    return value


def provider_error_body_preview(body: str, *, limit: int = 512) -> tuple[str, bool]:
    """Return a size-limited, redacted preview of an untrusted provider body."""

    truncated = len(body) > limit
    return redact_secret_text(body[:limit]), truncated
