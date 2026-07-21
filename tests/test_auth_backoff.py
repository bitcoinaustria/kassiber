from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from kassiber.errors import AppError
from kassiber.secrets.auth_backoff import (
    AUTH_BACKOFF_MAX_SECONDS,
    AUTH_FAILURES_BEFORE_BACKOFF,
    AuthAttemptBackoff,
)


class AuthAttemptBackoffTest(unittest.TestCase):
    def test_lockout_starts_at_the_declared_failure_threshold(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.secrets.auth_backoff.time.time",
            return_value=1_000.0,
        ):
            backoff = AuthAttemptBackoff(str(Path(tmp) / "backoff.json"))
            for _ in range(AUTH_FAILURES_BEFORE_BACKOFF - 1):
                backoff.record_failure()
                backoff.check("operator_unlock")

            backoff.record_failure()
            with self.assertRaises(AppError) as raised:
                backoff.check("operator_unlock")

        self.assertEqual(raised.exception.code, "local_auth_rate_limited")
        self.assertEqual(
            raised.exception.details["retry_after_seconds"],
            5,
        )

    def test_backward_clock_step_cannot_exceed_the_maximum_lockout(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            state_path = Path(tmp) / "backoff.json"
            state_path.write_text(
                json.dumps({"failures": 9, "locked_until": 1_005.0}),
                encoding="utf-8",
            )
            backoff = AuthAttemptBackoff(str(state_path))

            with mock.patch(
                "kassiber.secrets.auth_backoff.time.time",
                return_value=100.0,
            ):
                with self.assertRaises(AppError) as raised:
                    backoff.check("operator_unlock")

            persisted = json.loads(state_path.read_text(encoding="utf-8"))

        self.assertEqual(
            raised.exception.details["retry_after_seconds"],
            int(AUTH_BACKOFF_MAX_SECONDS),
        )
        self.assertEqual(
            persisted["locked_until"],
            100.0 + AUTH_BACKOFF_MAX_SECONDS,
        )

    def test_expired_deadline_is_cleared_without_losing_failure_history(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            state_path = Path(tmp) / "backoff.json"
            state_path.write_text(
                json.dumps({"failures": 3, "locked_until": 100.0}),
                encoding="utf-8",
            )
            backoff = AuthAttemptBackoff(str(state_path))
            with mock.patch(
                "kassiber.secrets.auth_backoff.time.time",
                return_value=101.0,
            ):
                backoff.check("operator_unlock")

            persisted = json.loads(state_path.read_text(encoding="utf-8"))
            self.assertEqual(persisted["locked_until"], 0.0)
            self.assertEqual(persisted["failures"], 3)


if __name__ == "__main__":
    unittest.main()
