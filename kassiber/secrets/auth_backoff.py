"""Shared retry-smoothing for database passphrase verification attempts.

The deadline uses UTC epoch time because daemon and broker processes must share
it across restarts.  It is not an adversarial rate-limit boundary: SQLCipher's
KDF provides the brute-force cost, and the logged-in OS user can edit this
state.  Loaded deadlines are clamped to the configured maximum so a backward
clock correction or corrupt state cannot create an unbounded local lockout.
"""

from __future__ import annotations

import json
import threading
import time
from pathlib import Path

from ..errors import AppError


AUTH_FAILURES_BEFORE_BACKOFF = 3
AUTH_BACKOFF_BASE_SECONDS = 5.0
AUTH_BACKOFF_MAX_SECONDS = 30.0
AUTH_BACKOFF_FILENAME = "auth_backoff.json"


class AuthAttemptBackoff:
    """Persisted, per-database throttling shared by daemon and broker auth."""

    def __init__(self, state_path: str | None = None) -> None:
        self._lock = threading.Lock()
        self._failures = 0
        self._locked_until = 0.0
        self._state_path = state_path

    def check(self, scope: str) -> None:
        now = time.time()
        with self._lock:
            self._load_locked()
            retry_after = self._locked_until - now
            if retry_after <= 0:
                if self._locked_until:
                    self._locked_until = 0.0
                    self._persist_locked()
                return
            if retry_after > AUTH_BACKOFF_MAX_SECONDS:
                retry_after = AUTH_BACKOFF_MAX_SECONDS
                self._locked_until = now + retry_after
                self._persist_locked()
        raise AppError(
            "too many failed passphrase attempts",
            code="local_auth_rate_limited",
            details={
                "scope": scope,
                "throttle": "database",
                "retry_after_seconds": max(1, int(retry_after + 0.999)),
            },
            hint="Wait before trying the passphrase again.",
            retryable=True,
        )

    def record_success(self) -> None:
        with self._lock:
            self._failures = 0
            self._locked_until = 0.0
            self._persist_locked()

    def record_failure(self) -> None:
        now = time.time()
        with self._lock:
            self._load_locked()
            self._failures += 1
            if self._failures < AUTH_FAILURES_BEFORE_BACKOFF:
                self._persist_locked()
                return
            delay = min(
                AUTH_BACKOFF_MAX_SECONDS,
                AUTH_BACKOFF_BASE_SECONDS
                * 2 ** (self._failures - AUTH_FAILURES_BEFORE_BACKOFF),
            )
            self._locked_until = now + delay
            self._persist_locked()

    def _load_locked(self) -> None:
        if not self._state_path:
            return
        try:
            with open(self._state_path, encoding="utf-8") as handle:
                payload = json.load(handle)
        except FileNotFoundError:
            return
        except (OSError, json.JSONDecodeError, TypeError, ValueError):
            return
        self._failures = max(0, int(payload.get("failures", 0)))
        self._locked_until = max(0.0, float(payload.get("locked_until", 0.0)))

    def _persist_locked(self) -> None:
        if not self._state_path:
            return
        try:
            if self._failures <= 0 and self._locked_until <= 0:
                try:
                    Path(self._state_path).unlink()
                except FileNotFoundError:
                    pass
                return
            state_path = Path(self._state_path)
            state_path.parent.mkdir(parents=True, exist_ok=True)
            tmp_path = state_path.with_suffix(state_path.suffix + ".tmp")
            tmp_path.write_text(
                json.dumps(
                    {
                        "failures": self._failures,
                        "locked_until": self._locked_until,
                    },
                    sort_keys=True,
                )
                + "\n",
                encoding="utf-8",
            )
            tmp_path.replace(state_path)
        except OSError:
            return
