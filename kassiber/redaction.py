"""Shared redaction helpers for runtime secret egress boundaries."""

from __future__ import annotations

import re
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
    "token",
    "xprv",
)

_PRIVATE_KEY_RE = re.compile(r"\b(?:xprv|tprv|yprv|zprv|uprv|vprv)[1-9A-HJ-NP-Za-km-z]{20,}\b")
_EXTENDED_KEY_RE = re.compile(r"\b(?:xpub|tpub|ypub|zpub|upub|vpub)[1-9A-HJ-NP-Za-km-z]{20,}\b")
_DESCRIPTOR_RE = re.compile(r"\b(?:wpkh|sh|wsh|tr|pkh|combo)\([^)\n]{16,}\)", re.IGNORECASE)
_BEARER_RE = re.compile(r"\b[Bb]earer\s+[A-Za-z0-9._~+/-]+=*")
_SK_SECRET_RE = re.compile(r"\bsk-[A-Za-z0-9._~+/-]{6,}\b")
_ASSIGNED_SECRET_RE = re.compile(
    r"(?P<key>\b(?:api[_-]?key|auth[_-]?header|cookie|descriptor|mnemonic|"
    r"passphrase|password|recovery[_-]?phrase|secret|seed(?:[_-]?(?:phrase|words))?|token)\b)"
    r"(?P<sep>\s*[:=]\s*)"
    r"(?P<quote>['\"]?)"
    r"(?P<value>[^'\"\s,;}{]+)",
    re.IGNORECASE,
)
_JSON_SECRET_RE = re.compile(
    r"(?P<prefix>['\"](?:api[_-]?key|auth[_-]?header|cookie|descriptor|mnemonic|"
    r"passphrase|password|recovery[_-]?phrase|secret|seed(?:[_-]?(?:phrase|words))?|token)"
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
# They are replaced with STABLE pseudonyms so cross-line correlation survives.
# The FNV-1a hash + token shapes mirror the webview redactor
# (ui-tauri/src/lib/appLogs.ts) so a value pseudonymized here and one
# pseudonymized in the UI collapse to the same token across the merged stream.
_TXID_RE = re.compile(r"\b[0-9a-f]{64}\b", re.IGNORECASE)
_AMOUNT_UNITS = (
    "BTC|XBT|LBTC|sats?|msats?|EUR|USD|CHF|GBP|JPY|CAD|AUD|NZD|SEK|NOK|DKK|PLN|CZK|HUF"
)
_AMOUNT_NUMBER = r"[+-]?(?:(?:\d{1,3}(?:[,_ .]\d{3})+)|\d+)(?:[.,]\d+)?"
_CURRENCY_SYMBOLS = "€$£¥₿"
_AMOUNT_RE = re.compile(
    "|".join(
        (
            rf"\b(?:{_AMOUNT_UNITS})\s*{_AMOUNT_NUMBER}\b",
            rf"\b{_AMOUNT_NUMBER}\s*(?:{_AMOUNT_UNITS})\b",
            rf"[{_CURRENCY_SYMBOLS}]\s*{_AMOUNT_NUMBER}\b",
            rf"\b{_AMOUNT_NUMBER}\s*[{_CURRENCY_SYMBOLS}]",
        )
    ),
    re.IGNORECASE,
)
_AMOUNT_NUM_RE = re.compile(r"[+-]?\d[\d.,_ ]*\d|[+-]?\d")


def _stable_hash(value: str) -> str:
    """FNV-1a (32-bit), byte-for-byte identical to the webview `stableHash`."""
    h = 0x811C9DC5
    for ch in value:
        h = (h ^ (ord(ch) & 0xFFFFFFFF)) & 0xFFFFFFFF
        h = (h * 0x01000193) & 0xFFFFFFFF
    return format(h, "08x")


def redact_operational_text(value: str) -> str:
    """Pseudonymize txids and unit-tagged amounts in free text crossing egress.

    A txid becomes ``txid#<fnv>`` and a unit-tagged amount becomes
    ``amount#<fnv>``. Addresses/paths are intentionally left readable (the
    operational tier keeps them for debugging). Bare unit-less integers cannot
    be safely auto-detected without context and are left as-is; structure such
    values into typed log fields instead of interpolating them into messages.
    """
    text = _TXID_RE.sub(lambda m: f"txid#{_stable_hash(m.group(0).lower())}", value)

    def _amount(match: "re.Match[str]") -> str:
        token = match.group(0)
        start = match.start()
        # A market-rate tail ("BTC/EUR 64000.12") is public data, not the
        # user's amount; its number is preceded by the pair separator.
        if start > 0 and text[start - 1] in "/-":
            return token
        num_match = _AMOUNT_NUM_RE.search(token)
        num = num_match.group(0) if num_match else token
        unit = re.sub(r"\s+", "", token.replace(num, "", 1))
        key = f"{re.sub(r'[,_ ]', '', num)}|{unit.lower()}"
        return f"amount#{_stable_hash(key)}"

    return _AMOUNT_RE.sub(_amount, text)


def is_sensitive_key(key: str) -> bool:
    lowered = key.lower().replace("-", "_")
    return any(part.replace("-", "_") in lowered for part in SENSITIVE_KEY_PARTS)


def redact_secret_text(value: str) -> str:
    """Redact secret-shaped material from text that may cross a trust boundary."""

    text = _RECOVERY_ASSIGNMENT_RE.sub(r"\g<key>\g<sep>\g<quote>[redacted]", value)
    text = _PRIVATE_KEY_RE.sub("[redacted-private-key]", text)
    text = _EXTENDED_KEY_RE.sub("[redacted-extended-key]", text)
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
