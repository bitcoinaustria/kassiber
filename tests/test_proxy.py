from unittest.mock import patch
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


def test_onion_proxy_failure_hints_suggests_tor_browser_port():
    hints = onion_proxy_failure_hints(
        "tcp://abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcd.onion:50001",
        "127.0.0.1:9050",
        ConnectionRefusedError("refused"),
        tcp_probe=lambda host, port, timeout: (host, port) == ("127.0.0.1", 9150),
    )

    assert hints[0] == (
        "Tor proxy not reachable at 127.0.0.1:9050. "
        "Start Tor or edit this backend's proxy."
    )
    assert hints[1] == (
        "Tor Browser's SOCKS proxy appears reachable at 127.0.0.1:9150. "
        "To use it, edit this backend's proxy port to 9150."
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
        tcp_probe=lambda host, port, timeout: True,
    )

    assert hints == []
