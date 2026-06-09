from datetime import datetime, timezone
import unittest

from kassiber.retry import retry_after_seconds


class RetryAfterHelpersTest(unittest.TestCase):
    def test_retry_after_seconds_accepts_integer_seconds(self):
        self.assertEqual(retry_after_seconds("90"), 90)
        self.assertEqual(retry_after_seconds("-5"), 0)

    def test_retry_after_seconds_accepts_http_date(self):
        now = datetime(2026, 6, 4, 12, 0, 0, tzinfo=timezone.utc)
        self.assertEqual(
            retry_after_seconds("Thu, 04 Jun 2026 12:01:30 GMT", now=now),
            90,
        )

    def test_retry_after_seconds_clamps_expired_http_date(self):
        now = datetime(2026, 6, 4, 12, 0, 0, tzinfo=timezone.utc)
        self.assertEqual(
            retry_after_seconds("Thu, 04 Jun 2026 11:59:00 GMT", now=now),
            0,
        )

    def test_retry_after_seconds_ignores_invalid_values(self):
        self.assertIsNone(retry_after_seconds(None))
        self.assertIsNone(retry_after_seconds(""))
        self.assertIsNone(retry_after_seconds("not a retry value"))


if __name__ == "__main__":
    unittest.main()
