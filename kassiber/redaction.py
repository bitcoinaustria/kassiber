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
    "password",
    "passphrase",
    "private",
    "secret",
    "token",
    "xprv",
)

_PRIVATE_KEY_RE = re.compile(r"\b(?:xprv|tprv|yprv|zprv|uprv|vprv)[1-9A-HJ-NP-Za-km-z]{20,}\b")
_EXTENDED_KEY_RE = re.compile(r"\b(?:xpub|tpub|ypub|zpub|upub|vpub)[1-9A-HJ-NP-Za-km-z]{20,}\b")
_DESCRIPTOR_RE = re.compile(r"\b(?:wpkh|sh|wsh|tr|pkh|combo)\([^)\n]{16,}\)", re.IGNORECASE)
_BEARER_RE = re.compile(r"\b[Bb]earer\s+[A-Za-z0-9._~+/-]+=*")
_SK_SECRET_RE = re.compile(r"\bsk-[A-Za-z0-9._~+/-]{6,}\b")
_ASSIGNED_SECRET_RE = re.compile(
    r"(?P<key>\b(?:api[_-]?key|auth[_-]?header|cookie|descriptor|passphrase|password|secret|token)\b)"
    r"(?P<sep>\s*[:=]\s*)"
    r"(?P<quote>['\"]?)"
    r"(?P<value>[^'\"\s,;}{]+)",
    re.IGNORECASE,
)
_JSON_SECRET_RE = re.compile(
    r"(?P<prefix>['\"](?:api[_-]?key|auth[_-]?header|cookie|descriptor|passphrase|password|secret|token)['\"]\s*:\s*['\"])"
    r"(?P<value>[^'\"]+)",
    re.IGNORECASE,
)


def is_sensitive_key(key: str) -> bool:
    lowered = key.lower().replace("-", "_")
    return any(part.replace("-", "_") in lowered for part in SENSITIVE_KEY_PARTS)


def redact_secret_text(value: str) -> str:
    """Redact secret-shaped material from text that may cross a trust boundary."""

    text = _PRIVATE_KEY_RE.sub("[redacted-private-key]", value)
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
