import io
import unittest
from email.message import Message
from urllib import error as urlerror

from kassiber import http_client
from kassiber.errors import AppError


class HttpClientTest(unittest.TestCase):
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

        def url_error():
            raise urlerror.URLError("connection refused")

        with self.assertRaises(AppError) as ctx:
            http_client.request_with_retry(secret_url, url_error, max_attempts=1)
        self.assertNotIn("node.example", str(ctx.exception))
        self.assertNotIn("bc1qsecret", str(ctx.exception))

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
