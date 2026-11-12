"""Shared redaction helpers for runtime secret egress boundaries."""

from __future__ import annotations

import base64
import json
import re
import secrets
from typing import Any, Iterable
from urllib import parse as urlparse

from kassiber.redaction_scanner import redact_credentials


SENSITIVE_KEY_PARTS = (
    "api_key",
    "api-key",
    "authorization",
    "auth_header",
    "auth-header",
    "auth_response",
    "blinding",
    "cookie",
    "descriptor",
    "mnemonic",
    "password",
    "rpcauth",
    "rpc_auth",
    "rune",
    "macaroon",
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

_PRIVATE_KEY_RE = re.compile(r"(?<![A-Za-z0-9_])(?:xprv|tprv|yprv|zprv|uprv|vprv)[A-Za-z0-9]{16,}(?![A-Za-z0-9_])", re.IGNORECASE | re.ASCII)
_EXTENDED_KEY_RE = re.compile(r"(?<![A-Za-z0-9_])(?:xpub|tpub|ypub|zpub|upub|vpub)[1-9A-HJ-NP-Za-km-z]{20,}(?![A-Za-z0-9_])", re.IGNORECASE | re.ASCII)
_DESCRIPTOR_RE = re.compile(r"(?<![A-Za-z0-9_])(?:wpkh|sh|wsh|tr|pkh|combo)\([^)\n]{16,}\)", re.IGNORECASE | re.ASCII)
_SP_DESCRIPTOR_RE = re.compile(r"(?<![A-Za-z0-9_])sp\([^)\n]{8,}\)", re.IGNORECASE | re.ASCII)
_SP_KEY_RE = re.compile(r"(?<![A-Za-z0-9_])(?:t?spscan|t?spspend)1q[023456789acdefghjklmnpqrstuvwxyz]{8,}(?![A-Za-z0-9_])", re.IGNORECASE | re.ASCII)
_SP_ADDRESS_RE = re.compile(r"(?<![A-Za-z0-9_])(?:t?sp)1q[023456789acdefghjklmnpqrstuvwxyz]{20,}(?![A-Za-z0-9_])", re.IGNORECASE | re.ASCII)
_WIF_RE = re.compile(r"(?<![A-Za-z0-9_])(?:5[1-9A-HJ-NP-Za-km-z]{49,50}|[KL][1-9A-HJ-NP-Za-km-z]{50,51})(?![A-Za-z0-9_])", re.ASCII)
_BEARER_RE = re.compile(r"(?<![A-Za-z0-9_])[Bb]earer[ \t\r\n\v\f\u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000\ufeff]+[A-Za-z0-9._~+/-]+=*", re.ASCII)
_NOSTR_SECRET_RE = re.compile(r"(?<![A-Za-z0-9_])nsec1[023456789acdefghjklmnpqrstuvwxyz]{20,}(?![A-Za-z0-9_])", re.IGNORECASE | re.ASCII)
_SK_SECRET_RE = re.compile(r"(?<![A-Za-z0-9_])sk-[A-Za-z0-9._~+/-]{6,}(?![A-Za-z0-9_])", re.ASCII)
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
# Complete ID shapes only: do not exempt partial UUIDs or standalone cad1.
_AMOUNT_ID_RE = re.compile(
    r"(?<![A-Za-z0-9_-])(?:[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}|[0-9a-f]{32})"
    r"(?![A-Za-z0-9_-])", re.IGNORECASE,
)
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

    def _redact_segment(segment: str) -> str:
        def _amount(match: re.Match[str]) -> str:
            token = match.group(0)
            if _is_market_rate_tail(segment, match.start()):
                return token
            num_match = _AMOUNT_NUM_RE.search(token)
            num = num_match.group(0) if num_match else token
            unit = re.sub(r"\s+", "", token.replace(num, "", 1))
            return _amount_pseudonym(num, unit)

        def _keyed_amount(match: re.Match[str]) -> str:
            key, sep, num = match.group(1), match.group(2), match.group(3)
            unit = "msat" if "msat" in key.lower() else "sat"
            return f"{key}{sep}{_amount_pseudonym(num, unit)}"

        return _KEYED_AMOUNT_RE.sub(_keyed_amount, _AMOUNT_RE.sub(_amount, segment))

    # Shield only the ID itself. An amount regex can start in an all-digit ID
    # tail and swallow a neighboring amount; exempting that whole match leaks
    # the amount. Scan the surrounding segments independently instead.
    parts = []
    cursor = 0
    for identity in _AMOUNT_ID_RE.finditer(text):
        parts.extend((_redact_segment(text[cursor:identity.start()]), identity.group(0)))
        cursor = identity.end()
    parts.append(_redact_segment(text[cursor:]))
    return "".join(parts)


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

    text = redact_credentials(value)
    text = _PRIVATE_KEY_RE.sub("[redacted-private-key]", text)
    text = _NOSTR_SECRET_RE.sub("[redacted-private-key]", text)
    text = _WIF_RE.sub("[redacted-private-key]", text)
    text = _EXTENDED_KEY_RE.sub("[redacted-extended-key]", text)
    text = _SP_DESCRIPTOR_RE.sub("[redacted-silent-payment-descriptor]", text)
    text = _SP_KEY_RE.sub("[redacted-silent-payment-key]", text)
    text = _SP_ADDRESS_RE.sub("[redacted-silent-payment-address]", text)
    text = _DESCRIPTOR_RE.sub("[redacted-descriptor]", text)
    text = _BEARER_RE.sub("Bearer [redacted]", text)
    text = _SK_SECRET_RE.sub("[redacted-secret]", text)
    return text


WITHHELD_CREDENTIAL = "<redacted>"


_HEX_FORM_RE = re.compile(r"[0-9A-Fa-f]+")
# Request headers whose whole value authenticates the request.
CREDENTIAL_HEADERS = frozenset(
    {
        "authorization",
        "proxy-authorization",
        "cookie",
        "grpc-metadata-macaroon",
        "api-key",
        "x-api-key",
        "x-mbx-apikey",
        "cb-access-key",
        "rune",
    }
)
_QUERY_CREDENTIAL_PARTS = ("key", "auth", "sig", "access")


def credential_forms(credentials: Iterable[str | None]) -> tuple[str, ...]:
    """Every spelling of ``credentials`` the scrub withholds, longest first.

    A credential is withheld as sent, percent-encoded (``quote`` and
    ``quote_plus``) and JSON-escaped (with and without ``\\/``). Ties in length
    sort by text, so the result never depends on set iteration order.
    """

    forms: set[str] = set()
    for credential in credentials:
        if not credential:
            continue
        credential = str(credential)
        escaped = json.dumps(credential)[1:-1]
        forms.update(
            (
                credential,
                urlparse.quote(credential, safe=""),
                urlparse.quote_plus(credential, safe=""),
                escaped,
                escaped.replace("/", "\\/"),
            )
        )
    return tuple(sorted(forms, key=lambda form: (-len(form), form)))


def _withhold_forms(value: str, forms: tuple[str, ...]) -> str:
    for form in forms:
        if _HEX_FORM_RE.fullmatch(form):
            # Hex means the same bytes in either case (an LND macaroon).
            value = re.sub(
                re.escape(form),
                lambda _match: WITHHELD_CREDENTIAL,
                value,
                flags=re.IGNORECASE | re.ASCII,
            )
        else:
            value = value.replace(form, WITHHELD_CREDENTIAL)
    return value


def withhold_credentials(value: str, credentials: Iterable[str | None]) -> str:
    """Replace each literal credential a server echoed back with ``<redacted>``.

    Pattern redaction cannot recognize a bare API key or secret, so callers
    that hold the credential they sent scrub it from untrusted response text
    before that text reaches an error, and before any truncation, which could
    otherwise keep a prefix. Percent- and JSON-escaped spellings are withheld
    too, and a hex credential in either letter case.
    """

    return _withhold_forms(value, credential_forms(credentials))


def _withhold_value(value: Any, forms: tuple[str, ...], depth: int) -> Any:
    if depth > 8:
        return "[truncated]"
    if isinstance(value, dict):
        return {
            (_withhold_forms(key, forms) if isinstance(key, str) else key): _withhold_value(
                item, forms, depth + 1
            )
            for key, item in value.items()
        }
    if isinstance(value, (list, tuple)):
        return [_withhold_value(item, forms, depth + 1) for item in value]
    if isinstance(value, str):
        return _withhold_forms(value, forms)
    return value


def withhold_credentials_value(value: Any, credentials: Iterable[str | None]) -> Any:
    """``withhold_credentials`` over every string key and leaf of a structure."""

    forms = credential_forms(credentials)
    return _withhold_value(value, forms, 0) if forms else value


def withhold_error_credentials(error: Any, credentials: Iterable[str | None]) -> Any:
    """Withhold ``credentials`` from an ``AppError``'s message, hint and details.

    Used where a caller still holds the credentials when an error leaves it:
    the error is rewritten in place and returned, so ``raise`` keeps its
    traceback and cause.
    """

    forms = credential_forms(credentials)
    if not forms:
        return error
    if error.args:
        error.args = (_withhold_forms(str(error.args[0]), forms), *error.args[1:])
    if isinstance(getattr(error, "hint", None), str):
        error.hint = _withhold_forms(error.hint, forms)
    if getattr(error, "details", None) is not None:
        error.details = _withhold_value(error.details, forms, 0)
    return error


def authorization_credentials(value: Any) -> list[str]:
    """The credentials one ``Authorization``-style header value carries.

    The whole value, the part after its scheme, and for ``Basic`` the decoded
    ``user:password`` and the password. A user name alone is an identifier,
    not a credential; withholding a short one would garble any message.
    """

    text = str(value or "").strip()
    if not text:
        return []
    found = [text]
    scheme, _, rest = text.partition(" ")
    rest = rest.strip()
    if rest:
        found.append(rest)
        if scheme.lower() == "basic":
            try:
                decoded = base64.b64decode(rest, validate=True).decode("utf-8")
            except (ValueError, UnicodeDecodeError):
                decoded = ""
            if decoded:
                found.append(decoded)
                _user, separator, password = decoded.partition(":")
                if separator and password:
                    found.append(password)
    return found


def basic_credentials(username: Any, password: Any) -> list[str]:
    """``password``, ``user:password`` and its ``Basic`` base64.

    The user name alone is not withheld (see ``authorization_credentials``).
    """

    user = str(username or "")
    secret = str(password or "")
    if not secret:
        return []
    pair = f"{user}:{secret}"
    return [secret, pair, base64.b64encode(pair.encode("utf-8")).decode("ascii")]


def url_credentials(url: Any, *, default_scheme: str | None = None) -> list[str]:
    """Credentials embedded in a URL: its userinfo and credential-like query values.

    ``default_scheme`` lets a bare ``user:pass@host:port`` proxy location parse.
    Query values count when their key names a secret, a key, auth, a signature
    or an access token.
    """

    raw = str(url or "").strip()
    if not raw:
        return []
    if default_scheme and "://" not in raw:
        raw = f"{default_scheme}://{raw}"
    try:
        parts = urlparse.urlsplit(raw)
    except ValueError:
        return []
    found: list[str] = []
    userinfo, at, _host = parts.netloc.rpartition("@")
    if at and userinfo:
        user, separator, password = userinfo.partition(":")
        if separator and password:
            found.append(userinfo)
            found.extend(basic_credentials(urlparse.unquote(user), urlparse.unquote(password)))
            found.append(password)
        elif not separator:
            # Userinfo without a password is a token in the user position.
            found.extend((userinfo, urlparse.unquote(userinfo)))
    for key, item in urlparse.parse_qsl(parts.query, keep_blank_values=False):
        lowered = key.lower()
        if is_sensitive_key(key) or any(part in lowered for part in _QUERY_CREDENTIAL_PARTS):
            found.append(item)
    return [item for item in found if item]


def header_credentials(headers: Any) -> list[str]:
    """The credentials a request's headers carry (see ``CREDENTIAL_HEADERS``)."""

    found: list[str] = []
    items = headers.items() if hasattr(headers, "items") else (headers or ())
    for name, value in items:
        lowered = str(name).lower()
        if lowered in {"authorization", "proxy-authorization"}:
            found.extend(authorization_credentials(value))
        elif lowered in CREDENTIAL_HEADERS or is_sensitive_key(lowered):
            text = str(value or "").strip()
            if text:
                found.append(text)
    return found


_BACKEND_USERNAME_FIELDS = ("username", "rpcuser", "rpc_user")
_BACKEND_PASSWORD_FIELDS = ("password", "rpcpassword", "rpc_password")


def backend_credentials(backend: Any) -> tuple[str, ...]:
    """Every credential a backend config holds, in the forms Kassiber sends.

    The token (API key, macaroon, rune), the auth header, RPC user and
    password with their ``Basic`` form, and userinfo or credential-like query
    values in the backend and proxy URLs. A cookie file's content is read only
    where the request is built; see ``request_credentials``.
    """

    if not hasattr(backend, "get"):
        return ()

    def text(key: str) -> str:
        # Only text and integers are credential values; anything else (a
        # float, bool or list in config JSON) is not sent as one.
        value = backend.get(key)
        if isinstance(value, str):
            return value.strip()
        if isinstance(value, int) and not isinstance(value, bool):
            return str(value)
        return ""

    found: list[str] = []
    token = text("token")
    if token:
        found.append(token)
    found.extend(authorization_credentials(text("auth_header")))
    users = [text(key) for key in _BACKEND_USERNAME_FIELDS if text(key)]
    passwords = [text(key) for key in _BACKEND_PASSWORD_FIELDS if text(key)]
    for user in users or [""]:
        for password in passwords:
            found.extend(basic_credentials(user, password))
    found.extend(url_credentials(text("url")))
    for key in ("tor_proxy", "proxy"):
        found.extend(url_credentials(text(key), default_scheme="socks5h"))
    return tuple(item for item in found if item)


def request_credentials(*, url: Any = None, headers: Any = None, proxy_url: Any = None) -> tuple[str, ...]:
    """Every credential one outbound request carries: URL, headers and proxy."""

    return (
        *url_credentials(url),
        *header_credentials(headers),
        *url_credentials(proxy_url, default_scheme="socks5h"),
    )


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


def provider_error_body_preview(
    body: str, *, limit: int = 512, withheld: Iterable[str | None] = ()
) -> tuple[str, bool]:
    """Return a size-limited, redacted preview of an untrusted provider body.

    ``withheld`` credentials are withheld before the cut, which could
    otherwise keep a prefix.
    """

    body = withhold_credentials(body, withheld)
    truncated = len(body) > limit
    return redact_secret_text(body[:limit]), truncated
