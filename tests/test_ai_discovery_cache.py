from __future__ import annotations

import threading
import time

from kassiber.ai.discovery_cache import ProviderDiscoveryCache
from kassiber.errors import AppError


def test_discovery_cache_single_flight_and_last_good_fallback() -> None:
    cache = ProviderDiscoveryCache(ttl_seconds=60)
    entered = threading.Event()
    release = threading.Event()
    calls = 0

    def fetch() -> list[str]:
        nonlocal calls
        calls += 1
        entered.set()
        release.wait(timeout=2)
        return ["model-a"]

    results: list[object] = []
    threads = [
        threading.Thread(
            target=lambda: results.append(cache.get("models", fetch, refresh=True))
        )
        for _ in range(2)
    ]
    threads[0].start()
    assert entered.wait(timeout=2)
    threads[1].start()
    time.sleep(0.02)
    release.set()
    for thread in threads:
        thread.join(timeout=2)

    assert calls == 1
    assert [result.value for result in results] == [["model-a"], ["model-a"]]

    def fail() -> None:
        raise AppError("Model endpoint unavailable", code="ai_unavailable")

    stale = cache.get("models", fail, refresh=True)
    assert stale.value == ["model-a"]
    assert stale.stale is True
    assert stale.error == {
        "code": "ai_unavailable",
        "message": "Model endpoint unavailable",
    }


def test_cold_probe_failure_is_shared_with_waiters() -> None:
    """A failed first probe must not become one probe per waiter.

    With nothing cached there is no last-good snapshot to fall back on, so
    without sharing the failure every blocked caller ran its own serialized
    probe — turning one slow outage into N.
    """

    cache = ProviderDiscoveryCache(ttl_seconds=60)
    started = threading.Event()
    release = threading.Event()
    calls: list[int] = []
    errors: list[Exception] = []

    def slow_failing_fetch():
        calls.append(1)
        started.set()
        release.wait(5)
        raise AppError("probe failed", code="ai_unavailable")

    def waiter() -> None:
        try:
            cache.get("k", slow_failing_fetch)
        except Exception as exc:  # noqa: BLE001 - recorded for the assertion
            errors.append(exc)

    threads = [threading.Thread(target=waiter) for _ in range(3)]
    threads[0].start()
    assert started.wait(5), "first probe never started"
    for thread in threads[1:]:
        thread.start()
    time.sleep(0.1)  # let the waiters reach the condition before the probe fails
    release.set()
    for thread in threads:
        thread.join(5)

    assert len(calls) == 1, "each waiter ran its own probe"
    assert len(errors) == 3, "every caller should see the failure"


def test_stale_failure_is_not_replayed_to_a_later_call() -> None:
    cache = ProviderDiscoveryCache(ttl_seconds=60)
    calls: list[int] = []

    def fetch():
        calls.append(1)
        if len(calls) == 1:
            raise AppError("first probe failed", code="ai_unavailable")
        return {"ok": True}

    try:
        cache.get("k", fetch)
    except AppError:
        pass
    else:  # pragma: no cover - the first probe must fail
        raise AssertionError("expected the first probe to raise")

    assert cache.get("k", fetch).value == {"ok": True}
    assert len(calls) == 2
