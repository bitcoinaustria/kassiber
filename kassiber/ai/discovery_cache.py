"""In-memory last-good cache for AI provider and model discovery."""

from __future__ import annotations

import threading
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any, Callable, Hashable

from ..errors import AppError


@dataclass(frozen=True)
class DiscoverySnapshot:
    value: Any
    checked_at: str
    stale: bool = False
    error: dict[str, str] | None = None

    def metadata(self) -> dict[str, Any]:
        return {
            "checked_at": self.checked_at,
            "stale": self.stale,
            "error": self.error,
        }


@dataclass(frozen=True)
class _CacheEntry:
    snapshot: DiscoverySnapshot
    expires_at: float


def _checked_at() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def _safe_error(exc: Exception) -> dict[str, str]:
    if isinstance(exc, AppError):
        return {
            "code": exc.code or "ai_unavailable",
            "message": str(exc),
        }
    return {
        "code": "ai_unavailable",
        "message": "Provider discovery failed.",
    }


class ProviderDiscoveryCache:
    """Deduplicate refreshes and retain the last successful discovery result."""

    def __init__(self, *, ttl_seconds: float = 60.0) -> None:
        self._ttl_seconds = ttl_seconds
        self._condition = threading.Condition()
        self._entries: dict[Hashable, _CacheEntry] = {}
        self._refreshing: set[Hashable] = set()

    def invalidate(self) -> None:
        with self._condition:
            self._entries.clear()

    def get(
        self,
        key: Hashable,
        fetch: Callable[[], Any],
        *,
        refresh: bool = False,
    ) -> DiscoverySnapshot:
        with self._condition:
            entry = self._entries.get(key)
            if entry and not refresh and entry.expires_at > time.monotonic():
                return entry.snapshot
            if key in self._refreshing:
                self._condition.wait_for(lambda: key not in self._refreshing)
                entry = self._entries.get(key)
                if entry:
                    return entry.snapshot
            self._refreshing.add(key)

        try:
            value = fetch()
        except Exception as exc:
            with self._condition:
                previous = self._entries.get(key)
                if previous:
                    snapshot = DiscoverySnapshot(
                        value=previous.snapshot.value,
                        checked_at=_checked_at(),
                        stale=True,
                        error=_safe_error(exc),
                    )
                    self._entries[key] = _CacheEntry(
                        snapshot=snapshot,
                        expires_at=time.monotonic() + self._ttl_seconds,
                    )
                    return snapshot
            raise
        else:
            snapshot = DiscoverySnapshot(value=value, checked_at=_checked_at())
            with self._condition:
                self._entries[key] = _CacheEntry(
                    snapshot=snapshot,
                    expires_at=time.monotonic() + self._ttl_seconds,
                )
            return snapshot
        finally:
            with self._condition:
                self._refreshing.discard(key)
                self._condition.notify_all()
