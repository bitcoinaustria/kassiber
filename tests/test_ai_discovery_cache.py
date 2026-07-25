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
