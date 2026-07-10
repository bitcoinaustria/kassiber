"""Hybrid logical clock primitives used by replication events.

The encoded form is lexicographically sortable and carries the replica id as
the final deterministic tiebreak. Wall time alone is never used to decide
whether one financial edit overwrites another; event context/version vectors
carry causality and the HLC only provides a stable replay order.
"""

from __future__ import annotations

from dataclasses import dataclass
import time


_PHYSICAL_WIDTH = 16
_COUNTER_WIDTH = 10


@dataclass(frozen=True, order=True)
class HybridLogicalClock:
    physical_ms: int
    counter: int
    replica_id: str

    def __post_init__(self) -> None:
        if self.physical_ms < 0:
            raise ValueError("HLC physical time must be non-negative")
        if self.counter < 0:
            raise ValueError("HLC counter must be non-negative")
        if not self.replica_id or ":" in self.replica_id:
            raise ValueError("HLC replica_id must be non-empty and cannot contain ':'")

    def encode(self) -> str:
        return (
            f"{self.physical_ms:0{_PHYSICAL_WIDTH}d}:"
            f"{self.counter:0{_COUNTER_WIDTH}d}:{self.replica_id}"
        )

    @classmethod
    def parse(cls, value: str) -> "HybridLogicalClock":
        parts = str(value or "").split(":", 2)
        if len(parts) != 3:
            raise ValueError("invalid HLC encoding")
        try:
            physical_ms = int(parts[0])
            counter = int(parts[1])
        except ValueError as exc:
            raise ValueError("invalid HLC numeric component") from exc
        return cls(physical_ms=physical_ms, counter=counter, replica_id=parts[2])


def _now_ms() -> int:
    return time.time_ns() // 1_000_000


def tick_clock(
    last: HybridLogicalClock | str | None,
    replica_id: str,
    *,
    now_ms: int | None = None,
) -> HybridLogicalClock:
    """Advance the local clock for a newly authored event."""

    previous = HybridLogicalClock.parse(last) if isinstance(last, str) else last
    physical_now = _now_ms() if now_ms is None else int(now_ms)
    if previous is None or physical_now > previous.physical_ms:
        return HybridLogicalClock(physical_now, 0, replica_id)
    return HybridLogicalClock(previous.physical_ms, previous.counter + 1, replica_id)


def observe_clock(
    local: HybridLogicalClock | str | None,
    remote: HybridLogicalClock | str,
    replica_id: str,
    *,
    now_ms: int | None = None,
) -> HybridLogicalClock:
    """Advance a local clock after observing a verified remote event."""

    local_clock = HybridLogicalClock.parse(local) if isinstance(local, str) else local
    remote_clock = HybridLogicalClock.parse(remote) if isinstance(remote, str) else remote
    physical_now = _now_ms() if now_ms is None else int(now_ms)
    local_physical = local_clock.physical_ms if local_clock else 0
    max_physical = max(physical_now, local_physical, remote_clock.physical_ms)
    if local_clock and max_physical == local_clock.physical_ms == remote_clock.physical_ms:
        counter = max(local_clock.counter, remote_clock.counter) + 1
    elif local_clock and max_physical == local_clock.physical_ms:
        counter = local_clock.counter + 1
    elif max_physical == remote_clock.physical_ms:
        counter = remote_clock.counter + 1
    else:
        counter = 0
    return HybridLogicalClock(max_physical, counter, replica_id)
