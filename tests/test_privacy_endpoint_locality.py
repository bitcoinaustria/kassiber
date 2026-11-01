"""Private network configuration does not prove that an endpoint is on-device."""

from unittest.mock import Mock

import pytest

from kassiber.core.reports import _backend_privacy_posture


@pytest.mark.parametrize("url,posture", [
    ("http://10.1.2.3:8332", "remote"),
    ("http://node.local:8332", "remote"),
    ("http://[::1", "unknown"),
    ("[::1", "unknown"),
    ("http://127.0.0.1:8332", "on_device"),
    ("http://[::1]:8332", "on_device"),
])
def test_posture_requires_a_valid_loopback_endpoint_without_resolving(monkeypatch, url, posture):
    resolve = Mock(side_effect=AssertionError("classification must stay local"))
    monkeypatch.setattr("socket.getaddrinfo", resolve)
    assert _backend_privacy_posture(url, has_proxy=False, kind="bitcoinrpc", ownership=None) == posture
    resolve.assert_not_called()
