from __future__ import annotations

"""Rate-limit-safe outbound HTTP request helper shared by sync backends and the
rates fetchers.

Centralizes two concerns so every outbound sync/rate request gets them, no
matter which subsystem or backend kind is fetching:

1. **Per-host concurrency cap.** A module-level ``BoundedSemaphore`` keyed by
   host bounds how many requests can be in flight against a single host at once.
   Users routinely point Kassiber at public infrastructure (the default
   ``mempool.space`` Esplora, plus Coinbase / CoinGecko for rates) that
   throttles, so parallel wallet/rate fetches must never collectively exceed
   one host's request budget. The semaphore is sized to a fixed constant equal
   to the per-wallet worker cap, so the bound is deterministic regardless of
   which backend is seen first.

2. **Bounded 429/503 retry.** Throttling (429) and transient unavailability
   (503) are retried a few times, honoring (and clamping) ``Retry-After`` or
   else using exponential backoff with jitter. On exhaustion the same retryable
   ``rate_limited`` ``AppError`` is re-raised so a caller's coarse scheduler
   backoff still fires as the outer safety net. Non-retryable HTTP errors and
   unreachable-host errors keep their immediate-raise behavior.

``sleeper``/``rng`` are injectable so tests stay deterministic; production uses
the stdlib ``time.sleep`` and ``random`` module.
"""

import random
import threading
import time
from urllib import error as urlerror
from urllib import parse as urlparse

from .errors import AppError
from .redaction import redact_operational_text, redact_secret_text
from .retry import retry_after_seconds_from_http_error

# Matches the per-wallet HTTP worker cap in
# ``kassiber.core.sync_backends._bounded_http_workers`` so a single host never
# sees more concurrency than one wallet already issued.
HOST_CONCURRENCY = 8
RETRY_STATUS = frozenset({429, 503})
MAX_ATTEMPTS = 3
BACKOFF_BASE_SECONDS = 0.5
BACKOFF_CAP_SECONDS = 8.0
# Keep the in-request retry well under any caller's scheduler backoff window so
# the two never fight; a server asking for longer is deferred to the scheduler.
MAX_CUMULATIVE_WAIT_SECONDS = 20.0

_HOST_LIMITERS: dict[str, threading.BoundedSemaphore] = {}
_HOST_LIMITER_LOCK = threading.Lock()


def host_limiter(url):
    """Return the shared per-host concurrency semaphore for ``url``.

    Lazily created under a single lock with double-checked locking so two
    threads can never install two semaphores for one host (which would double
    the effective concurrency).
    """
    host = urlparse.urlsplit(url).netloc.lower()
    limiter = _HOST_LIMITERS.get(host)
    if limiter is None:
        with _HOST_LIMITER_LOCK:
            limiter = _HOST_LIMITERS.get(host)
            if limiter is None:
                limiter = threading.BoundedSemaphore(HOST_CONCURRENCY)
                _HOST_LIMITERS[host] = limiter
    return limiter


def _rate_limited_error(url, exc, source_label):
    if exc.code == 429:
        message = f"{source_label} rate limited the request for {url} (HTTP 429)"
    else:
        message = f"{source_label} is temporarily unavailable for {url} (HTTP {exc.code})"
    return AppError(
        message,
        code="rate_limited",
        retryable=True,
        details={"retry_after_seconds": retry_after_seconds_from_http_error(exc)},
    )


def request_with_retry(
    url,
    opener,
    *,
    source_label="Backend",
    sleeper=None,
    rng=None,
    max_attempts=None,
    on_retry=None,
):
    """Run ``opener()`` under the per-host limiter with bounded 429/503 retry.

    ``opener`` performs exactly one request+read round-trip and returns the
    decoded body. ``on_retry(retry_number, max_retries, wait_seconds)`` (if
    given) fires on the main/worker thread just before each backoff sleep so a
    caller can surface "rate limited, retrying" progress instead of appearing to
    hang.
    """
    sleeper = sleeper if sleeper is not None else time.sleep
    rng = rng if rng is not None else random
    attempts = max(1, int(max_attempts if max_attempts is not None else MAX_ATTEMPTS))
    limiter = host_limiter(url)
    cumulative = 0.0
    last_error = None
    for attempt in range(attempts):
        limiter.acquire()
        try:
            return opener()
        except urlerror.HTTPError as exc:
            if exc.code not in RETRY_STATUS:
                detail = exc.read().decode("utf-8", errors="replace")
                # The server response body is untrusted free text that routinely
                # echoes txids/amounts; scrub secrets + pseudonymize operational
                # ids at the source so the error never carries them raw into the
                # ring/envelope/disk (the render pass is a backstop, not the only
                # line of defense).
                detail = redact_operational_text(redact_secret_text(detail[:200]))
                raise AppError(
                    f"HTTP {exc.code} from backend for {url}: {detail}"
                ) from exc
            last_error = exc
            retry_after = retry_after_seconds_from_http_error(exc)
        except urlerror.URLError as exc:
            raise AppError(f"Failed to reach backend {url}: {exc.reason}") from exc
        finally:
            limiter.release()
        # Only retryable statuses (429/503) reach here; the semaphore is released
        # before we sleep so a backoff never holds a host slot.
        if retry_after is not None:
            delay = float(retry_after)
        else:
            backoff = min(BACKOFF_CAP_SECONDS, BACKOFF_BASE_SECONDS * (2**attempt))
            delay = backoff + rng.uniform(0.0, BACKOFF_BASE_SECONDS)
        is_last = attempt + 1 >= attempts
        if is_last or cumulative + delay > MAX_CUMULATIVE_WAIT_SECONDS:
            raise _rate_limited_error(url, last_error, source_label)
        if on_retry is not None:
            on_retry(attempt + 1, attempts - 1, delay)
        sleeper(delay)
        cumulative += delay
    raise _rate_limited_error(url, last_error, source_label)


__all__ = [
    "HOST_CONCURRENCY",
    "MAX_ATTEMPTS",
    "RETRY_STATUS",
    "host_limiter",
    "request_with_retry",
]
