"""Adapter registry — node `kind` → :class:`LightningAdapter`.

Adapters register themselves at import time (typically inside their own
module). The daemon imports node adapters lazily in ``daemon.py``; tests
can override the registry with :func:`register_adapter` /
:func:`unregister_adapter`.
"""

from __future__ import annotations

from threading import Lock
from typing import TYPE_CHECKING

from .capabilities import LightningCapabilities, lightning_capabilities_from_adapter

if TYPE_CHECKING:
    from .adapter import LightningAdapter


_registry: dict[str, "LightningAdapter"] = {}
_lock = Lock()


def register_adapter(kind: str, adapter: "LightningAdapter") -> None:
    """Register a Lightning adapter for the given connection ``kind``.

    Re-registering the same ``kind`` overwrites the previous adapter —
    intentional so tests can swap in fakes.
    """
    if not kind:
        raise ValueError("Lightning adapter kind must be a non-empty string")
    with _lock:
        _registry[kind] = adapter


def unregister_adapter(kind: str) -> None:
    """Remove an adapter (test helper; production code should not call this)."""
    with _lock:
        _registry.pop(kind, None)


def resolve_adapter(kind: str) -> "LightningAdapter | None":
    """Return the registered adapter for ``kind`` (or ``None``)."""
    with _lock:
        return _registry.get(kind)


def registered_kinds() -> tuple[str, ...]:
    """Return the kinds with a registered adapter (sorted, for stable output)."""
    with _lock:
        return tuple(sorted(_registry.keys()))


def registered_capabilities(kind: str) -> LightningCapabilities:
    """Return explicit capabilities for a registered adapter kind."""
    with _lock:
        adapter = _registry.get(kind)
    return lightning_capabilities_from_adapter(adapter)
