"""Read-only BTCPay Server Greenfield API client.

The client only issues ``GET`` requests. Kassiber never creates invoices,
payouts, or API keys on a merchant's server, and it never asks for payment
method configuration (that payload carries derivation schemes and Lightning
connection strings).

Two details matter for every caller:

- BTCPay answers permission failures with a JSON body such as
  ``{"code": "missing-permission", "missingPermission": "btcpay.store..."}``.
  The client keeps that body so errors can name the exact permission the key
  lacks instead of guessing from the endpoint.
- ``GET /api/v1/api-keys/current`` echoes the API key itself. Responses from
  that endpoint are scrubbed before they leave this module.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any, Callable, Mapping
from urllib import error as urlerror
from urllib import parse as urlparse
from urllib import request as urlrequest

from ..backends import backend_timeout, backend_value
from ..errors import AppError
from ..http_client import request_with_retry
from ..proxy import build_proxy_opener

SOURCE_LABEL = "BTCPay"
API_PREFIX = "/api/v1"
_ERROR_BODY_LIMIT = 4096


class GreenfieldHttpError(Exception):
    """A non-retryable HTTP status with BTCPay's decoded error body."""

    def __init__(self, status: int, body: Any):
        super().__init__(f"HTTP {status}")
        self.status = status
        self.body = body

    @property
    def error_code(self) -> str | None:
        if isinstance(self.body, Mapping):
            value = self.body.get("code")
            return str(value) if value else None
        return None

    @property
    def missing_permission(self) -> str | None:
        if isinstance(self.body, Mapping):
            value = self.body.get("missingPermission")
            return str(value) if value else None
        return None


def normalize_server_url(value: Any) -> str:
    """Return a canonical BTCPay base URL or raise a validation error.

    Accepts what people paste from their browser: trailing slashes, a
    trailing ``/api/v1``, a store dashboard path, or a Greenfield docs path.
    Only the scheme, host, port, and any reverse-proxy prefix before a known
    BTCPay route are kept.
    """

    raw = str(value or "").strip()
    if not raw:
        raise AppError(
            "BTCPay server URL is required",
            code="validation",
            hint="Enter the address you open BTCPay Server with, for example https://btcpay.example.com.",
        )
    if "://" not in raw:
        raise AppError(
            "BTCPay server URL must start with https:// or http://",
            code="validation",
            hint="Use the full address, for example https://btcpay.example.com.",
            details={"reason": "missing_scheme"},
        )
    try:
        parts = urlparse.urlsplit(raw)
    except ValueError as exc:
        raise AppError("BTCPay server URL is not a valid URL", code="validation") from exc
    scheme = parts.scheme.lower()
    if scheme not in {"http", "https"}:
        raise AppError(
            "BTCPay server URL must use https:// or http://",
            code="validation",
            details={"reason": "unsupported_scheme"},
        )
    if not parts.hostname:
        raise AppError("BTCPay server URL is missing a host name", code="validation")
    if parts.username or parts.password:
        raise AppError(
            "BTCPay server URL must not contain a user name or password",
            code="validation",
            hint="Remove the credentials from the URL and use a Greenfield API key instead.",
        )
    path = parts.path or ""
    lowered = path.lower()
    # Strip well-known BTCPay routes a user may paste together with the host.
    for marker in (
        "/api/v1",
        "/api-keys",
        "/stores/",
        "/apps/",
        "/invoices",
        "/i/",
        "/account",
        "/login",
        "/docs",
        "/wallets/",
        "/server/",
        "/payment-requests",
        "/pull-payments",
    ):
        index = lowered.find(marker)
        if index >= 0:
            path = path[:index]
            lowered = lowered[:index]
    path = path.rstrip("/")
    netloc = parts.netloc
    return urlparse.urlunsplit((scheme, netloc, path, "", ""))


def server_url_identity(value: Any) -> str:
    """Comparable identity for two URLs that point at the same server."""

    try:
        normalized = normalize_server_url(value)
    except AppError:
        return str(value or "").strip().rstrip("/").lower()
    parts = urlparse.urlsplit(normalized)
    host = (parts.hostname or "").lower()
    port = parts.port
    default_port = 443 if parts.scheme == "https" else 80
    netloc = host if port in (None, default_port) else f"{host}:{port}"
    return f"{parts.scheme}://{netloc}{parts.path.rstrip('/').lower()}"


def is_onion_url(value: Any) -> bool:
    try:
        host = urlparse.urlsplit(str(value or "")).hostname or ""
    except ValueError:
        return False
    return host.lower().endswith(".onion")


def is_loopback_url(value: Any) -> bool:
    try:
        host = (urlparse.urlsplit(str(value or "")).hostname or "").lower()
    except ValueError:
        return False
    return host in {"localhost", "127.0.0.1", "::1"} or host.startswith("127.")


@dataclass
class GreenfieldClient:
    """Minimal Greenfield client bound to one server URL and API key."""

    base_url: str
    token: str
    timeout: int = 30
    opener: Any = None
    source_label: str = SOURCE_LABEL
    request_log: list[str] = field(default_factory=list)

    @classmethod
    def from_backend(
        cls,
        backend: Mapping[str, Any],
        *,
        opener: Any = None,
        opener_factory: Callable[[Mapping[str, Any]], Any] | None = None,
        missing_token_hint: str | None = None,
    ) -> "GreenfieldClient":
        base = backend_value(backend, "url")
        if not base:
            raise AppError("BTCPay instance is missing 'url'", code="config_error")
        token = backend_value(backend, "token")
        if not token:
            raise AppError(
                "BTCPay instance is missing 'token' (api key)",
                code="config_error",
                hint=missing_token_hint
                or "Store the api key with `kassiber backends update --token-stdin` or `--token-fd FD`.",
            )
        if opener is None:
            opener = (opener_factory or default_opener)(backend)
        return cls(
            base_url=normalize_server_url(base),
            token=token,
            timeout=backend_timeout(backend),
            opener=opener,
        )

    def url(self, path: str, query: Mapping[str, Any] | list[tuple[str, Any]] | None = None) -> str:
        path = path if path.startswith("/") else f"/{path}"
        url = f"{self.base_url}{path}"
        if query:
            items = query.items() if isinstance(query, Mapping) else query
            encoded = urlparse.urlencode(
                [(key, _query_value(value)) for key, value in items if value is not None]
            )
            if encoded:
                url = f"{url}?{encoded}"
        return url

    def get_json(
        self,
        path: str,
        query: Mapping[str, Any] | list[tuple[str, Any]] | None = None,
        *,
        context: str | None = None,
        permission_hint: str | None = None,
        not_found_hint: str | None = None,
    ) -> Any:
        url = self.url(path, query)
        try:
            return self.get_url(url)
        except GreenfieldHttpError as exc:
            raise greenfield_app_error(
                exc,
                context=context,
                permission_hint=permission_hint,
                not_found_hint=not_found_hint,
            ) from exc

    def get_url(self, url: str) -> Any:
        """GET ``url`` and decode JSON, raising ``GreenfieldHttpError`` on 4xx/5xx."""

        request = urlrequest.Request(
            url,
            headers={
                "Accept": "application/json",
                "Authorization": f"token {self.token}",
            },
        )
        self.request_log.append(url)

        def open_once():
            try:
                with self.opener.open(request, timeout=self.timeout) as response:
                    raw = response.read()
            except urlerror.HTTPError as exc:
                if exc.code in {429, 503}:
                    raise
                raise GreenfieldHttpError(exc.code, _decode_error_body(exc)) from exc
            if not raw:
                return None
            try:
                return json.loads(raw.decode("utf-8"))
            except (UnicodeDecodeError, ValueError) as exc:
                raise AppError(
                    "BTCPay returned a response that is not JSON",
                    code="protocol_error",
                    hint="Check that the URL points at BTCPay Server and not at a login page or proxy.",
                ) from exc

        try:
            return request_with_retry(url, open_once, source_label=self.source_label)
        except AppError as exc:
            cause = exc.__cause__
            if isinstance(cause, urlerror.URLError) and not isinstance(cause, urlerror.HTTPError):
                raise AppError(
                    "Could not reach the BTCPay server",
                    code="network_error",
                    retryable=True,
                    hint=(
                        "Check the server URL, your network or Tor proxy, and that BTCPay is running."
                    ),
                    details={"reason": str(getattr(cause, "reason", "") or "")[:200]},
                ) from cause
            raise

    def get_optional_json(self, path: str, query=None, *, tolerate=(401, 403, 404, 405)) -> tuple[Any, GreenfieldHttpError | None]:
        """GET that reports tolerated HTTP failures instead of raising."""

        try:
            return self.get_url(self.url(path, query)), None
        except GreenfieldHttpError as exc:
            if exc.status in tolerate:
                return None, exc
            raise greenfield_app_error(exc) from exc


def default_opener(backend: Mapping[str, Any]):
    return build_proxy_opener(
        backend_value(backend, "tor_proxy", "proxy"),
        source_label=SOURCE_LABEL,
    )


def greenfield_app_error(
    exc: GreenfieldHttpError,
    *,
    context: str | None = None,
    permission_hint: str | None = None,
    not_found_hint: str | None = None,
) -> AppError:
    """Translate a Greenfield HTTP failure into a precise ``AppError``."""

    from .permissions import permission_label

    details: dict[str, Any] = {"http_status": exc.status}
    if exc.error_code:
        details["btcpay_code"] = exc.error_code
    where = f" for {context}" if context else ""
    if exc.status == 401:
        return AppError(
            "BTCPay rejected the API key (HTTP 401)",
            code="auth_error",
            hint=(
                "The key is unknown, revoked, or belongs to another server. "
                "Create a new key in BTCPay (Account → API Keys) and update this connection."
            ),
            details=details,
        )
    if exc.status == 403:
        missing = exc.missing_permission
        if missing:
            details["missing_permission"] = missing
            label = permission_label(missing)
            return AppError(
                f"BTCPay API key is missing the '{label}' permission{where}",
                code="auth_error",
                hint=permission_hint
                or (
                    f"Create a key that includes {missing} for this store, or use "
                    "`kassiber btcpay key-url` to open a pre-filled authorization page."
                ),
                details=details,
            )
        return AppError(
            f"BTCPay API key is missing a required permission{where} (HTTP 403)",
            code="auth_error",
            hint=permission_hint or "Check the permissions granted to the Greenfield API key.",
            details=details,
        )
    if exc.status == 404:
        code = exc.error_code or ""
        if "store" in code:
            message = "BTCPay store not found (HTTP 404)"
        elif "payment-method" in code or "paymentmethod" in code:
            message = "BTCPay payment method is not configured for this store (HTTP 404)"
        else:
            message = f"BTCPay resource not found{where} (HTTP 404)"
        return AppError(
            message,
            code="not_found",
            hint=not_found_hint or "Verify the store ID and payment method (default BTC-CHAIN).",
            details=details,
        )
    message = None
    if isinstance(exc.body, Mapping):
        message = exc.body.get("message")
    return AppError(
        f"BTCPay returned HTTP {exc.status}{where}" + (f": {str(message)[:200]}" if message else ""),
        code="protocol_error",
        details=details,
    )


def scrub_api_key_payload(payload: Any) -> dict[str, Any]:
    """Drop the echoed secret from ``/api-keys/current``."""

    if not isinstance(payload, Mapping):
        return {}
    permissions = payload.get("permissions")
    return {
        "label": str(payload.get("label") or "") or None,
        "permissions": [str(item) for item in permissions] if isinstance(permissions, list) else [],
    }


def _decode_error_body(exc: urlerror.HTTPError) -> Any:
    try:
        raw = exc.read(_ERROR_BODY_LIMIT)
    except Exception:  # pragma: no cover - defensive
        return None
    if not raw:
        return None
    try:
        decoded = json.loads(raw.decode("utf-8", errors="replace"))
    except ValueError:
        return None
    if isinstance(decoded, list) and decoded and isinstance(decoded[0], Mapping):
        # Validation errors arrive as a list of {path, message}.
        return {"code": "validation", "message": str(decoded[0].get("message") or "")}
    return decoded if isinstance(decoded, Mapping) else None


def _query_value(value: Any) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    return str(value)


__all__ = [
    "API_PREFIX",
    "GreenfieldClient",
    "GreenfieldHttpError",
    "default_opener",
    "greenfield_app_error",
    "is_loopback_url",
    "is_onion_url",
    "normalize_server_url",
    "scrub_api_key_payload",
    "server_url_identity",
]
