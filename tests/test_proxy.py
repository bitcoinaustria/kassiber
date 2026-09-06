import io
from unittest.mock import patch
from urllib import error as urlerror, response as urlresponse
from urllib import request as urlrequest

import pytest

from kassiber.errors import AppError
from kassiber.proxy import (
    SocksUrlResponse,
    _connect_via_socks5,
    onion_proxy_failure_hints,
)


class _FakeSocket:
    def __init__(self, responses):
        self.sent = bytearray()
        self._inbox = bytearray(b"".join(responses))
        self.closed = False

    def sendall(self, data):
        self.sent.extend(data)

    def recv(self, length):
        chunk = bytes(self._inbox[:length])
        del self._inbox[:length]
        return chunk

    def close(self):
        self.closed = True


class _FakeHttpResponse:
    status = 200
    reason = "OK"
    headers = {}

    def read(self, *args):
        return b'{"ok": true}'


@pytest.mark.parametrize("transport", ["http", "socks", "electrum"])
def test_process_override_blocks_compatibility_transport_before_dns(transport, monkeypatch):
    from kassiber.proxy import urlopen_with_proxy
    from kassiber.core.sync_backends import _connect_backend_socket

    monkeypatch.setenv("KASSIBER_NO_EGRESS", "1")
    with patch("socket.create_connection") as connect, patch("socket.getaddrinfo") as dns, patch("kassiber.proxy.urlrequest.build_opener") as http:
        with pytest.raises(AppError) as caught:
            if transport == "http":
                urlopen_with_proxy(urlrequest.Request("http://127.0.0.1:8080/api"))
            elif transport == "socks":
                _connect_via_socks5("socks5h://127.0.0.1:9050", "node.invalid", 50002, 5)
            else:
                _connect_backend_socket({}, "127.0.0.1", 50001)
    assert caught.value.code == "network_egress_disabled"
    connect.assert_not_called()
    dns.assert_not_called()
    http.assert_not_called()


def test_connect_via_socks5_userpass_auth():
    fake = _FakeSocket(
        [
            b"\x05\x02",
            b"\x01\x00",
            b"\x05\x00\x00\x01\x00\x00\x00\x00\x00\x00",
        ]
    )
    with patch("kassiber.proxy.socket.create_connection", return_value=fake):
        sock = _connect_via_socks5(
            "socks5h://alice:p%40ss@127.0.0.1:9050",
            "node.example",
            50002,
            timeout=5,
        )
    assert sock is fake
    assert not fake.closed
    sent = bytes(fake.sent)
    assert sent.startswith(b"\x05\x02\x00\x02")
    assert b"\x01\x05alice\x04p@ss" in sent


def test_socks_url_response_preserves_post_method_and_body():
    calls = []

    def fake_request(_connection, method, target, **kwargs):
        calls.append((method, target, kwargs))

    def fake_getresponse(_connection):
        return _FakeHttpResponse()

    request = urlrequest.Request(
        "http://rpc.example/",
        data=b'{"method":"ping"}',
        method="POST",
    )
    with patch("kassiber.proxy._connect_via_socks5", return_value=object()), patch(
        "kassiber.proxy.http.client.HTTPConnection.request",
        fake_request,
    ), patch(
        "kassiber.proxy.http.client.HTTPConnection.getresponse",
        fake_getresponse,
    ):
        with SocksUrlResponse(
            request.full_url,
            "socks5h://127.0.0.1:9050",
            5,
            dict(request.header_items()),
            method=request.get_method(),
            data=request.data,
        ) as response:
            assert response.read() == b'{"ok": true}'
    assert calls == [
        (
            "POST",
            "/",
            {"headers": {}, "body": b'{"method":"ping"}'},
        )
    ]


def test_urlopen_with_proxy_rejects_onion_without_proxy():
    from kassiber.proxy import urlopen_with_proxy

    request = urlrequest.Request("http://examplehiddenservice.onion/api")
    with patch("kassiber.proxy.urlrequest.urlopen") as direct:
        with pytest.raises(AppError, match="Tor/SOCKS proxy"):
            urlopen_with_proxy(request, request.full_url, timeout=5)
    direct.assert_not_called()


def test_urlopen_with_proxy_forwards_explicit_tls_context_directly():
    from kassiber.proxy import urlopen_with_proxy

    request = urlrequest.Request("https://core.example/")
    context = object()
    with patch("kassiber.proxy.urlrequest.build_opener") as opener:
        urlopen_with_proxy(request, timeout=5, ssl_context=context)
    assert any(getattr(handler, "_context", None) is context for handler in opener.call_args.args)
    opener.return_value.open.assert_called_once_with(request, timeout=5)


def test_http_proxy_opener_receives_explicit_tls_context():
    from kassiber.proxy import urlopen_with_proxy

    request = urlrequest.Request("https://core.example/")
    context = object()
    with patch("kassiber.proxy.urlrequest.build_opener") as opener:
        urlopen_with_proxy(
            request,
            timeout=5,
            proxy_url="http://127.0.0.1:8080",
            ssl_context=context,
        )
    handlers = opener.call_args.args
    assert any(getattr(handler, "_context", None) is context for handler in handlers)


def test_socks_https_uses_explicit_tls_context():
    class Context:
        def wrap_socket(self, raw_socket, server_hostname=None):
            return raw_socket

    context = Context()
    class WrappedSocket:
        def close(self):
            pass

    wrapped = WrappedSocket()

    def connect_for_request(connection, *_args, **_kwargs):
        connection.connect()

    with patch("kassiber.proxy._connect_via_socks5", return_value=object()), patch.object(
        context,
        "wrap_socket",
        return_value=wrapped,
        create=True,
    ) as wrap_socket, patch(
        "kassiber.proxy.http.client.HTTPSConnection.request",
        connect_for_request,
    ), patch(
        "kassiber.proxy.http.client.HTTPSConnection.getresponse",
        return_value=_FakeHttpResponse(),
    ):
        with SocksUrlResponse(
            "https://core.example/",
            "socks5h://127.0.0.1:9050",
            5,
            {},
            ssl_context=context,
        ):
            pass
    wrap_socket.assert_called_once()


def test_onion_proxy_failure_hints_do_not_probe_an_alternate_port():
    with patch("kassiber.proxy.socket.create_connection") as connect, patch("socket.getaddrinfo") as dns:
        hints = onion_proxy_failure_hints(
            "tcp://abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcd.onion:50001",
            "127.0.0.1:9050",
            ConnectionRefusedError("refused"),
        )
    connect.assert_not_called()
    dns.assert_not_called()

    assert hints[0] == (
        "Tor proxy not reachable at 127.0.0.1:9050. "
        "Start Tor or edit this backend's proxy."
    )
    assert hints[1] == (
        "If you use Tor Browser, its SOCKS proxy normally uses port 9150. "
        "Edit this backend's proxy port to use it; no alternate port was probed."
    )


def test_onion_proxy_failure_hints_require_proxy_without_fallback():
    hints = onion_proxy_failure_hints("http://examplehiddenservice.onion/api", "")

    assert hints == [
        ".onion endpoints require a Tor/SOCKS proxy; configure this backend's proxy before testing."
    ]


def test_onion_proxy_failure_hints_ignore_backend_failure_after_proxy_connects():
    hints = onion_proxy_failure_hints(
        "ssl://examplehiddenservice.onion:50002",
        "127.0.0.1:9050",
        RuntimeError("SOCKS5 proxy connect failed with code 4"),
    )

    assert hints == []


@pytest.mark.parametrize("scheme", ["http", "https"])
@pytest.mark.parametrize("configured_proxy", [None, "http://user:pass@chosen-proxy.invalid:8080"])
def test_backend_http_uses_only_explicit_route(scheme, configured_proxy, monkeypatch):
    """Exercise urllib's actual handler chain, stopping at the transport."""
    from kassiber.proxy import urlopen_with_proxy

    sent = []

    class CaptureHTTP(urlrequest.HTTPHandler):
        def http_open(self, request):
            sent.append((request.host, request.get_header("Authorization"), request.get_header("Proxy-authorization")))
            result = urlresponse.addinfourl(io.BytesIO(b"{}"), {}, request.full_url, 200)
            result.msg = "OK"
            return result

    class CaptureHTTPS(urlrequest.HTTPSHandler):
        https_open = CaptureHTTP.http_open

    # A hostile ambient proxy must neither replace the selected direct route
    # nor bypass the selected proxy through NO_PROXY / system exclusions.
    monkeypatch.setenv("http_proxy", "http://ambient-proxy.invalid:8081")
    monkeypatch.setenv("https_proxy", "http://ambient-proxy.invalid:8081")
    monkeypatch.setenv("no_proxy", "*")
    original = urlrequest.build_opener
    with patch("kassiber.proxy.urlrequest.build_opener", side_effect=lambda *handlers: original(CaptureHTTP(), CaptureHTTPS(), *handlers)), patch("urllib.request.getproxies") as discovery, patch("urllib.request.proxy_bypass", return_value=True) as bypass:
        discovery.return_value = {"http": "http://ambient-proxy.invalid:8081", "https": "http://ambient-proxy.invalid:8081"}
        request = urlrequest.Request(f"{scheme}://chosen-node.invalid/api", headers={"Authorization": "Bearer synthetic"})
        with urlopen_with_proxy(request, proxy_url=configured_proxy) as response:
            assert response.read() == b"{}"
    discovery.assert_not_called()
    bypass.assert_not_called()
    assert sent == [("chosen-proxy.invalid:8080" if configured_proxy else "chosen-node.invalid", "Bearer synthetic", "Basic dXNlcjpwYXNz" if configured_proxy else None)]


@pytest.mark.parametrize("status", [301, 302, 303, 307, 308])
@pytest.mark.parametrize("configured_proxy", [None, "http://chosen-proxy.invalid:8080"])
def test_backend_http_refuses_redirects_by_default(status, configured_proxy):
    from kassiber.proxy import urlopen_with_proxy

    sent = []

    class RedirectHTTP(urlrequest.HTTPHandler):
        def http_open(self, request):
            sent.append(request.full_url)
            response = urlresponse.addinfourl(io.BytesIO(b""), {"location": "http://unapproved.invalid/stolen"}, request.full_url, status)
            response.msg = "Redirect"
            return response

    original = urlrequest.build_opener
    with patch("kassiber.proxy.urlrequest.build_opener", side_effect=lambda *handlers: original(RedirectHTTP(), *handlers)):
        with pytest.raises(urlerror.HTTPError) as caught:
            urlopen_with_proxy(urlrequest.Request("http://chosen-node.invalid/api", headers={"Authorization": "Bearer synthetic"}), proxy_url=configured_proxy)
        assert caught.value.code == status
        caught.value.close()
    assert sent == ["http://chosen-node.invalid/api"]


@pytest.mark.parametrize("target_scheme", ["http", "https"])
def test_https_proxy_is_rejected_before_ledger_dns_or_socket(target_scheme):
    from kassiber.proxy import urlopen_with_proxy

    request = urlrequest.Request(f"{target_scheme}://node.invalid/api")
    with patch("kassiber.proxy.get_egress_ledger") as ledger, patch(
        "socket.getaddrinfo", side_effect=AssertionError("Unexpected DNS")
    ) as dns, patch(
        "socket.create_connection", side_effect=AssertionError("Unexpected socket")
    ) as connect:
        with pytest.raises(AppError, match="HTTPS proxy"):
            urlopen_with_proxy(
                request, proxy_url="https://user:pass@proxy.invalid:8443"
            )
    ledger.assert_not_called()
    dns.assert_not_called()
    connect.assert_not_called()


@pytest.mark.parametrize("certificate_valid", [True, False])
def test_explicit_http_connect_keeps_target_tls_and_auth_separate(certificate_valid):
    """Use the real urllib/http.client stack with only sockets and TLS faked."""
    import ssl

    from kassiber.proxy import urlopen_with_proxy

    steps = []

    class FakeSocket:
        def __init__(self, phase, response):
            self.phase = phase
            self.response = response

        def sendall(self, payload):
            steps.append((self.phase, bytes(payload)))

        def makefile(self, *_args, **_kwargs):
            return io.BytesIO(self.response)

        def setsockopt(self, *_args):
            pass

        def close(self):
            pass

    raw = FakeSocket("proxy", b"HTTP/1.1 200 Connection established\r\n\r\n")
    secured = FakeSocket(
        "target_tls", b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}"
    )
    context = ssl.create_default_context()
    assert context.check_hostname is True
    assert context.verify_mode == ssl.CERT_REQUIRED

    def wrap_socket(sock, *, server_hostname):
        assert sock is raw
        steps.append(("tls", server_hostname))
        if not certificate_valid:
            raise ssl.SSLCertVerificationError("Synthetic target certificate failure")
        return secured

    request = urlrequest.Request(
        "https://node.invalid/api", headers={"Authorization": "Bearer target-only"}
    )
    with patch("socket.create_connection", return_value=raw) as connect, patch(
        "socket.getaddrinfo", side_effect=AssertionError("Unexpected DNS")
    ), patch.object(context, "wrap_socket", side_effect=wrap_socket):
        if certificate_valid:
            with urlopen_with_proxy(
                request, proxy_url="http://user:pass@proxy.invalid:8080", ssl_context=context
            ) as response:
                assert response.read() == b"{}"
        else:
            with pytest.raises(urlerror.URLError) as caught:
                urlopen_with_proxy(
                    request, proxy_url="http://user:pass@proxy.invalid:8080", ssl_context=context
                )
            assert isinstance(caught.value.reason, ssl.SSLCertVerificationError)

    assert connect.call_count == 1
    assert connect.call_args.args[0] == ("proxy.invalid", 8080)
    assert steps[0][0] == "proxy"
    assert b"CONNECT node.invalid:443 " in steps[0][1]
    assert b"Proxy-Authorization: Basic dXNlcjpwYXNz" in steps[0][1]
    assert b"Bearer target-only" not in steps[0][1]
    assert steps[1] == ("tls", "node.invalid")
    if certificate_valid:
        assert steps[2][0] == "target_tls"
        assert b"Authorization: Bearer target-only" in steps[2][1]
        assert b"Proxy-Authorization:" not in steps[2][1]
    else:
        assert len(steps) == 2  # No target request after failed certificate verification.
