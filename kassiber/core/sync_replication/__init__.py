"""Transport-independent authored-event replication for Kassiber books.

The package deliberately does not import or start any transport. A profile
that has never opted in only has empty schema tables; it gains no keys,
background work, or listener. Transports are injected by the CLI/daemon layers.
"""

from .clock import HybridLogicalClock, observe_clock, tick_clock
from .events import author_event, sync_enabled, verify_event
from .identity import disable_sync, enable_sync, sync_status

__all__ = [
    "HybridLogicalClock",
    "author_event",
    "disable_sync",
    "enable_sync",
    "observe_clock",
    "sync_enabled",
    "sync_status",
    "tick_clock",
    "verify_event",
]
