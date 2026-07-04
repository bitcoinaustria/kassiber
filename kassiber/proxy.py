from __future__ import annotations

"""Proxy helpers shared by wallet sync, BTCPay sync, and rate fetches."""

import http.client
import io
import ipaddress
import socket
import ssl
from urllib import error as urlerror
from urllib import parse as urlparse
from urllib import request as urlrequest

from .egress_ledger import get_egress_ledger, http_request_bytes_out
from .errors import AppError


def _with_default_proxy_scheme(proxy_url, default_scheme="socks5h"):
    raw = str(proxy_url or "").strip()
    if not raw:
        return ""
    if "://" not in raw:
        return f"{default_scheme}://{raw}"
    return raw


def is_onion_endpoint(endpoint):
    raw = str(endpoint or "").strip()
    if not raw:
        return False
    parsed = urlparse.urlsplit(raw if "://" in raw else f"//{raw}")
    host = (parsed.hostname or "").rstrip(".").lower()
    return host.endswith(".onion")


def _parse_proxy_url(proxy_url, *, default_scheme="socks5h"):
    normalized = _with_default_proxy_scheme(proxy_url, default_scheme=default_scheme)
    parsed = urlparse.urlsplit(normalized)
    scheme = (parsed.scheme or default_scheme).lower()
    host = parsed.hostname
    port = parsed.port or {"socks5": 9050, "socks5h": 9050}.get(scheme)
    if not host or not port:
        raise AppError(f"Invalid proxy URL: {proxy_url}")
    username = (
        urlparse.unquote(parsed.username)
        if parsed.username is not None
        else None
    )
    password = (
        urlparse.unquote(parsed.password)
        if parsed.password is not None
        else None
    )
    if (username is None) != (password is None):
        raise AppError(
            "SOCKS5 proxy credentials must include both username and password",
            code="validation",
            hint="Use socks5h://USER:PASS@HOST:PORT, percent-encoding any special characters.",
        )
    return scheme, host, int(port), username, password


def _is_loopback_proxy_host(host):
    normalized = str(host or "").strip().lower()
    if normalized in {"localhost", "ip6-localhost"}:
        return True
    try:
        return ipaddress.ip_address(normalized).is_loopback
    except ValueError:
        return False


def _tcp_endpoint_open(host, port, timeout):
    try:
        with socket.create_connection((host, int(port)), timeout=timeout):
            return True
    except OSError:
        return False


def _exception_chain(exc):
    seen = set()
    current = exc
    while current is not None and id(current) not in seen:
        seen.add(id(current))
        yield current
        if isinstance(current, urlerror.URLError) and getattr(current, "reason", None):
            current = current.reason
            continue
        current = getattr(current, "__cause__", None) or getattr(current, "__context__", None)


def _exception_indicates_proxy_unreachable(exc):
    if exc is None:
        return True
    for item in _exception_chain(exc):
        if isinstance(item, (ConnectionRefusedError, TimeoutError, socket.timeout)):
            return True
    return False


def onion_proxy_failure_hints(endpoint, proxy_url, exc=None, *, tcp_probe=None):
    if not is_onion_endpoint(endpoint):
        return []
    if not str(proxy_url or "").strip():
        return [
            ".onion endpoints require a Tor/SOCKS proxy; configure this backend's proxy before testing."
        ]
    if not _exception_indicates_proxy_unreachable(exc):
        return []
    try:
        _scheme, host, port, _username, _password = _parse_proxy_url(proxy_url)
    except AppError:
        return []

    hints = [
        f"Tor proxy not reachable at {host}:{port}. Start Tor or edit this backend's proxy."
    ]
    probe = tcp_probe or _tcp_endpoint_open
    if port == 9050 and _is_loopback_proxy_host(host) and probe(host, 9150, 0.2):
        hints.append(
            f"Tor Browser's SOCKS proxy appears reachable at {host}:9150. "
            "To use it, edit this backend's proxy port to 9150."
        )
    return hints


def _read_exact(sock, length):
    chunks = []
    remaining = length
    while remaining > 0:
        chunk = sock.recv(remaining)
        if not chunk:
            raise AppError("Proxy closed the connection during SOCKS5 negotiation")
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


def _socks5_address(host):
    try:
        address = ipaddress.ip_address(host)
    except ValueError:
        try:
            encoded = host.encode("idna")
        except UnicodeError as exc:
            raise AppError(f"SOCKS5 proxy target host is invalid: {exc}") from exc
        if len(encoded) > 255:
            raise AppError("SOCKS5 proxy target host is too long")
        return b"\x03" + bytes([len(encoded)]) + encoded
    if address.version == 4:
        return b"\x01" + address.packed
    return b"\x04" + address.packed


def _socks5_credentials(username, password):
    if username is None and password is None:
        return None
    user_bytes = str(username).encode("utf-8")
    password_bytes = str(password).encode("utf-8")
    if not user_bytes or not password_bytes:
        raise AppError(
            "SOCKS5 proxy username and password must be non-empty",
            code="validation",
        )
    if len(user_bytes) > 255 or len(password_bytes) > 255:
        raise AppError(
            "SOCKS5 proxy username and password must be at most 255 bytes",
            code="validation",
        )
    return user_bytes, password_bytes


def _authenticate_socks5(sock, username, password):
    credentials = _socks5_credentials(username, password)
    if credentials is None:
        sock.sendall(b"\x05\x01\x00")
    else:
        sock.sendall(b"\x05\x02\x00\x02")
    greeting = _read_exact(sock, 2)
    if greeting[0] != 0x05:
        raise AppError(
            f"SOCKS5 proxy returned an unexpected greeting (got {greeting!r})"
        )
    method = greeting[1]
    if method == 0x00:
        return
    if method == 0x02 and credentials is not None:
        user_bytes, password_bytes = credentials
        sock.sendall(
            b"\x01"
            + bytes([len(user_bytes)])
            + user_bytes
            + bytes([len(password_bytes)])
            + password_bytes
        )
        response = _read_exact(sock, 2)
        if response != b"\x01\x00":
            raise AppError(
                "SOCKS5 proxy username/password authentication failed",
                code="auth_error",
            )
        return
    if method == 0x02:
        raise AppError(
            "SOCKS5 proxy requires username/password authentication "
            "(method 0x02), but no proxy credentials were configured",
            code="auth_error",
            hint="Use socks5h://USER:PASS@HOST:PORT for SOCKS5 proxies that require authentication.",
        )
    if method == 0xFF:
        raise AppError("SOCKS5 proxy refused all offered authentication methods")
    raise AppError(
        f"SOCKS5 proxy selected unsupported authentication method 0x{method:02x}"
    )


def _connect_via_socks5(proxy_url, host, port, timeout):
    scheme, proxy_host, proxy_port, username, password = _parse_proxy_url(proxy_url)
    if scheme not in {"socks5", "socks5h"}:
        raise AppError(f"Unsupported SOCKS proxy transport '{scheme}'")
    sock = socket.create_connection((proxy_host, proxy_port), timeout=timeout)
    try:
        _authenticate_socks5(sock, username, password)
        request = (
            b"\x05\x01\x00"
            + _socks5_address(host)
            + int(port).to_bytes(2, byteorder="big")
        )
        sock.sendall(request)
        response = _read_exact(sock, 4)
        if response[0] != 5:
            raise AppError("SOCKS5 proxy returned an invalid response")
        if response[1] != 0:
            raise AppError(f"SOCKS5 proxy connect failed with code {response[1]}")
        atyp = response[3]
        if atyp == 1:
            _read_exact(sock, 4)
        elif atyp == 3:
            length = _read_exact(sock, 1)[0]
            _read_exact(sock, length)
        elif atyp == 4:
            _read_exact(sock, 16)
        else:
            raise AppError("SOCKS5 proxy returned an unsupported address type")
        _read_exact(sock, 2)
        return sock
    except Exception:
        sock.close()
        raise


def urlopen_with_proxy(
    request,
    url=None,
    timeout=30,
    proxy_url=None,
    *,
    source_label="backend",
):
    proxy = str(proxy_url or "").strip()
    target_url = url or request.full_url
    source = str(source_label or "").strip().lower()
    if "rate" in source or "price" in source:
        subsystem = "pricing"
    else:
        subsystem = "sync"
    get_egress_ledger().record_url(
        target_url,
        subsystem=subsystem,
        operation="http.request",
        method=request.get_method(),
        bytes_out=http_request_bytes_out(request),
        via_proxy=bool(proxy),
    )
    if not proxy:
        if is_onion_endpoint(target_url):
            raise AppError(
                f".onion {source_label} URLs require a Tor/SOCKS proxy",
                code="network_proxy_required",
                hint=(
                    "Configure a proxy for this endpoint; Kassiber will not "
                    "connect to .onion hosts directly."
                ),
            )
        return urlrequest.urlopen(request, timeout=timeout)
    normalized_proxy = _with_default_proxy_scheme(proxy)
    scheme = urlparse.urlsplit(normalized_proxy).scheme.lower()
    if scheme in {"http", "https"}:
        opener = urlrequest.build_opener(
            urlrequest.ProxyHandler(
                {"http": normalized_proxy, "https": normalized_proxy}
            )
        )
        return opener.open(request, timeout=timeout)
    if scheme not in {"socks5", "socks5h"}:
        raise AppError(
            f"Unsupported {source_label} proxy transport '{scheme or proxy}'",
            code="validation",
            hint=(
                "Use http://, https://, socks5://, socks5h://, or HOST:PORT "
                "for proxy settings."
            ),
        )
    return SocksUrlResponse(
        url or request.full_url,
        normalized_proxy,
        timeout,
        dict(request.header_items()),
        method=request.get_method(),
        data=getattr(request, "data", None),
    )


class SocksUrlResponse:
    def __init__(self, url, proxy_url, timeout, headers, *, method="GET", data=None):
        self._url = url
        self._proxy_url = proxy_url
        self._timeout = timeout
        self._headers = headers
        self._method = method
        self._data = data
        self._connection = None
        self._response = None

    def __enter__(self):
        parsed = urlparse.urlsplit(self._url)
        if parsed.scheme not in {"http", "https"} or not parsed.hostname:
            raise AppError(
                f"Unsupported backend URL for proxy fetch: {self._url}",
                code="validation",
            )
        port = parsed.port or (443 if parsed.scheme == "https" else 80)
        target = urlparse.urlunsplit(("", "", parsed.path or "/", parsed.query, ""))
        proxy_url = self._proxy_url
        timeout = self._timeout

        class SocksHTTPConnection(http.client.HTTPConnection):
            def connect(self):
                self.sock = _connect_via_socks5(
                    proxy_url,
                    parsed.hostname,
                    port,
                    timeout,
                )

        class SocksHTTPSConnection(http.client.HTTPSConnection):
            def connect(self):
                raw_sock = _connect_via_socks5(
                    proxy_url,
                    parsed.hostname,
                    port,
                    timeout,
                )
                context = ssl.create_default_context()
                self.sock = context.wrap_socket(
                    raw_sock,
                    server_hostname=parsed.hostname,
                )

        connection_class = (
            SocksHTTPSConnection
            if parsed.scheme == "https"
            else SocksHTTPConnection
        )
        self._connection = connection_class(parsed.hostname, port, timeout=timeout)
        try:
            request_kwargs = {"headers": self._headers}
            if self._data is not None:
                request_kwargs["body"] = self._data
            self._connection.request(self._method, target, **request_kwargs)
            self._response = self._connection.getresponse()
            if self._response.status >= 400:
                body = self._response.read()
                raise urlerror.HTTPError(
                    self._url,
                    self._response.status,
                    self._response.reason,
                    self._response.headers,
                    io.BytesIO(body),
                )
        except urlerror.HTTPError:
            self.close()
            raise
        except OSError as exc:
            self.close()
            raise urlerror.URLError(exc) from exc
        return self

    def read(self, *args):
        if self._response is None:
            return b""
        return self._response.read(*args)

    @property
    def status(self):
        return getattr(self._response, "status", None)

    @property
    def reason(self):
        return getattr(self._response, "reason", None)

    @property
    def headers(self):
        return getattr(self._response, "headers", {})

    def close(self):
        if self._connection is not None:
            self._connection.close()
            self._connection = None

    def __exit__(self, exc_type, exc, traceback):
        self.close()
        return False


class ProxyUrlOpener:
    def __init__(self, proxy_url=None, *, source_label="backend"):
        self.proxy_url = proxy_url
        self.source_label = source_label

    def open(self, request, timeout=30):
        return urlopen_with_proxy(
            request,
            request.full_url,
            timeout,
            proxy_url=self.proxy_url,
            source_label=self.source_label,
        )


def build_proxy_opener(proxy_url=None, *, source_label="backend"):
    return ProxyUrlOpener(proxy_url, source_label=source_label)


__all__ = [
    "ProxyUrlOpener",
    "SocksUrlResponse",
    "_connect_via_socks5",
    "_read_exact",
    "_socks5_address",
    "build_proxy_opener",
    "is_onion_endpoint",
    "onion_proxy_failure_hints",
    "urlopen_with_proxy",
]
