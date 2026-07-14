"""LWK ``ForeignStore`` buffered into Kassiber's SQLCipher observer store."""

from __future__ import annotations

from importlib.metadata import PackageNotFoundError, version
from typing import Mapping

from ...errors import AppError
from .identity import ObserverIdentity
from .store import persist_observer_values


LWK_VERSION = "0.18.0"


def require_lwk():
    try:
        import lwk
    except ModuleNotFoundError as exc:
        raise AppError(
            "Liquid descriptor refresh requires the bundled LWK dependency",
            code="dependency_missing",
            hint=f"Install Kassiber from a build that includes lwk {LWK_VERSION}.",
            details={"missing_package": "lwk"},
            retryable=False,
        ) from exc
    try:
        installed = version("lwk")
    except PackageNotFoundError:
        installed = None
    if installed != LWK_VERSION:
        raise AppError(
            "The installed LWK binding does not match Kassiber's observer format",
            code="dependency_version_mismatch",
            details={"package": "lwk", "expected": LWK_VERSION, "actual": installed},
            retryable=False,
        )
    return lwk


try:
    _ForeignStoreBase = require_lwk().ForeignStore
except AppError as exc:
    if exc.code != "dependency_missing":
        raise

    class _ForeignStoreBase:
        """Import-only placeholder where the pinned LWK has no native wheel."""


class SqlCipherForeignStore(_ForeignStoreBase):
    """Request-local store whose bytes become durable only during apply."""

    def __init__(self, identity: ObserverIdentity, values: Mapping[str, bytes]):
        self.identity = identity
        self._values = {str(key): bytes(value) for key, value in values.items()}

    def get(self, key: str) -> bytes | None:
        value = self._values.get(str(key))
        return bytes(value) if value is not None else None

    def put(self, key: str, value: bytes) -> None:
        self._values[str(key)] = bytes(value)

    def remove(self, key: str) -> None:
        self._values.pop(str(key), None)

    def persist(self, conn) -> None:
        persist_observer_values(conn, self.identity, self._values)

    def discard(self) -> None:
        self._values.clear()

    def snapshot(self) -> dict[str, bytes]:
        return dict(self._values)


__all__ = ["LWK_VERSION", "SqlCipherForeignStore", "require_lwk"]
