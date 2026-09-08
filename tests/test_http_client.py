import io
import json
import traceback
import unittest
from email.message import Message
from urllib import error as urlerror
from unittest.mock import Mock, patch

from kassiber import http_client
from kassiber.envelope import build_error_envelope
from kassiber.errors import AppError
from kassiber.log_ring import sanitize_traceback_text


class HttpClientTest(unittest.TestCase):
    def test_deadline_bounds_host_slot_wait(self):
        limiter = Mock()
        limiter.acquire.return_value = False
        opener = Mock()
        with patch.object(http_client, "host_limiter", return_value=limiter), \
             patch.object(http_client.time, "monotonic", return_value=7):
            with self.assertRaises(AppError) as raised:
                http_client.request_with_retry("https://private.example", opener, deadline=10)
        self.assertEqual(raised.exception.code, "backend_timeout")
        self.assertNotIn("private.example", str(raised.exception))
        limiter.acquire.assert_called_once_with(timeout=3)
        limiter.release.assert_not_called()
        opener.assert_not_called()

    def test_deadline_stops_backoff_before_next_attempt(self):
        headers = Message()
        headers["Retry-After"] = "10"
        opener = Mock(side_effect=urlerror.HTTPError(
            "https://private.example", 429, "err", headers, io.BytesIO(b"private body")))
        sleeper = Mock()
        with patch.object(http_client.time, "monotonic", return_value=1):
            with self.assertRaises(AppError) as raised:
                http_client.request_with_retry(
                    "https://private.example", opener, deadline=10, sleeper=sleeper)
        self.assertEqual(raised.exception.code, "backend_timeout")
        sleeper.assert_not_called()
        opener.assert_called_once()

    def test_rpc_retries_shrink_transport_timeout_with_shared_deadline(self):
        from kassiber.core import sync_backends
        clock = [0.0]
        timeouts = []
        headers = Message()
        headers["Retry-After"] = "1"

        def urlopen(_request, url, timeout, **_kwargs):
            timeouts.append(timeout)
            clock[0] += 4
            raise urlerror.HTTPError(url, 429, "err", headers, io.BytesIO(b""))

        def sleep(delay):
            clock[0] += delay

        with patch.object(sync_backends, "urlopen_with_proxy", side_effect=urlopen), \
             patch.object(http_client.time, "monotonic", side_effect=lambda: clock[0]), \
             patch.object(http_client.time, "sleep", side_effect=sleep):
            with self.assertRaises(AppError) as raised:
                sync_backends.bitcoinrpc_call(
                    {"url": "http://127.0.0.1:18443", "username": "fixture", "password": "fixture"},
                    "getrawtransaction", ["ab" * 32, True],
                    timeout=5, deadline=9)
        self.assertEqual(raised.exception.code, "backend_timeout")
        self.assertEqual(timeouts, [5, 4])
        self.assertEqual(clock[0], 9)

    def _assert_secret_url_absent_from_error_boundary(
        self, exc: AppError, secret_url: str
    ) -> None:
        self.assertEqual(exc.code, "app_error")
        self.assertIsNone(exc.details)
        self.assertIsNone(exc.hint)
        self.assertFalse(exc.retryable)
        debug = sanitize_traceback_text(
            "".join(traceback.format_exception(type(exc), exc, exc.__traceback__))
        )
        envelope = build_error_envelope(
            exc.code,
            str(exc),
            details=exc.details,
            hint=exc.hint,
            retryable=exc.retryable,
            debug=debug,
        )
        serialized = json.dumps(envelope, sort_keys=True)
        self.assertIn("<redacted-host>", serialized)
        for secret in (
            secret_url,
            "node.example",
            "/api/address",
            "bc1qsecret",
            "token=abc",
        ):
            self.assertNotIn(secret, serialized)

    def test_error_messages_do_not_expose_url_path_or_query(self):
        secret_url = "https://node.example/api/address/bc1qsecret?token=abc"

        def http_500():
            raise urlerror.HTTPError(secret_url, 500, "err", Message(), io.BytesIO(b"boom"))

        with self.assertRaises(AppError) as ctx:
            http_client.request_with_retry(secret_url, http_500, max_attempts=1)

        message = str(ctx.exception)
        self.assertIn("HTTP 500", message)
        self.assertIn("<redacted-host>", message)
        self.assertNotIn("node.example", message)
        self.assertNotIn("/api/address", message)
        self.assertNotIn("bc1qsecret", message)
        self.assertNotIn("token=abc", message)
        self._assert_secret_url_absent_from_error_boundary(ctx.exception, secret_url)

        def url_error():
            raise urlerror.URLError("connection refused")

        with self.assertRaises(AppError) as ctx:
            http_client.request_with_retry(secret_url, url_error, max_attempts=1)
        self.assertNotIn("node.example", str(ctx.exception))
        self.assertNotIn("bc1qsecret", str(ctx.exception))
        self._assert_secret_url_absent_from_error_boundary(ctx.exception, secret_url)

    def test_host_limiter_is_keyed_by_normalized_hostname(self):
        first = http_client.host_limiter("https://shared.example/a")
        again = http_client.host_limiter("https://shared.example/b?q=1")
        with_userinfo = http_client.host_limiter("https://user:pass@SHARED.example/c")
        other_port = http_client.host_limiter("https://shared.example:8443/d")
        other = http_client.host_limiter("https://other.example/a")

        self.assertIs(first, again)
        self.assertIs(first, with_userinfo)
        self.assertIs(first, other_port)
        self.assertIsNot(first, other)


if __name__ == "__main__":
    unittest.main()
