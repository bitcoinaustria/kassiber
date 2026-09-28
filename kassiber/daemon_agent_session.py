"""Let external agents read the encrypted book the desktop has unlocked.

The desktop already holds the passphrase of the book it opened (typed, Touch
ID, or the OS credential store). On the user's click it hands that passphrase
to the operator broker as a `read` lease, the same lease
`kassiber operator unlock --capability read` creates, so `kassiber mcp` can
serve the book. The renderer never sees the passphrase. The broker is contacted
only for that click and when the desktop's own lease ends; status is answered
from memory.
"""

from __future__ import annotations

import logging
import time
from dataclasses import dataclass, replace
from typing import Any

from .errors import AppError


KINDS = frozenset({"ui.agent_access.unlock", "ui.agent_access.lock"})

# The lease ends with the desktop session; this bounds it if the desktop
# crashes or is killed before it can lock.
DESKTOP_AGENT_LEASE_SECONDS = 8 * 60 * 60

# How often the daemon loop retries a lock the broker did not confirm.
_RETRY_SECONDS = 5.0
# After this long past its expiry, the broker has certainly dropped the lease.
_GIVE_UP_AFTER_EXPIRY_SECONDS = 60.0

_LOGGER = logging.getLogger("kassiber.daemon.agent_session")


@dataclass(frozen=True)
class DesktopAgentLease:
    data_root: str
    # The broker's public id for this lease: locking names it, so a lease
    # someone else opened since is never ended by the desktop.
    lease_id: object
    expires_at: object
    # Measured from before the grant request, so it never ends later here
    # than in the broker.
    expires_monotonic: float
    # Measured from after the grant, so cleanup never gives up while the
    # broker may still hold the lease.
    give_up_monotonic: float
    # The broker binds a book to `brokered` mode when it grants a lease; a
    # book that was `manual` goes back to it when the desktop's lease ends.
    restore_manual: bool
    # Set when ending it failed: kept (and shown) until the broker confirms,
    # and retried from the daemon loop.
    ending: bool = False
    retry_at: float = 0.0

    def expired(self) -> bool:
        return time.monotonic() >= self.expires_monotonic


def _unlock_mode(data_root: str) -> str:
    from .operator.modes import effective_unlock_mode

    try:
        return effective_unlock_mode(data_root)
    except (AppError, OSError):
        return "manual"


def session_state(ctx: Any) -> dict[str, Any]:
    """The book's agent session as the desktop knows it, without broker I/O."""

    lease = getattr(ctx, "agent_lease", None)
    if (
        isinstance(lease, DesktopAgentLease)
        and lease.data_root == ctx.data_root
        and not lease.expired()
    ):
        return {"needed": True, "active": True, "expires_at": lease.expires_at}
    # Plaintext books and books with a remembered (unattended) unlock already
    # open for agents without a lease.
    needed = (
        getattr(ctx, "conn", None) is not None
        and bool(getattr(ctx, "db_passphrase", None))
        and _unlock_mode(ctx.data_root) != "unattended"
    )
    return {"needed": needed, "active": False, "expires_at": None}


def unlock(ctx: Any) -> dict[str, Any]:
    from .agent_access import require_mcp_access
    from .operator.client import BrokerClient

    require_mcp_access()
    if getattr(ctx, "conn", None) is None or not getattr(ctx, "db_passphrase", None):
        raise AppError(
            "no unlocked encrypted book is open in Kassiber",
            code="agent_unlock_unavailable",
            hint="Open and unlock the encrypted book first; plaintext books need no unlock.",
            retryable=False,
        )
    mode = _unlock_mode(ctx.data_root)
    if mode == "unattended":
        raise AppError(
            "this book already opens for agents with its remembered unlock",
            code="agent_unlock_not_needed",
            retryable=False,
        )
    current = getattr(ctx, "agent_lease", None)
    if isinstance(current, DesktopAgentLease):
        if current.data_root == ctx.data_root and not current.expired() and not current.ending:
            return session_state(ctx)
        end_lease(ctx, raise_errors=True)
    passphrase = bytearray(str(ctx.db_passphrase).encode("utf-8"))
    started = time.monotonic()
    try:
        granted = BrokerClient().unlock(
            ctx.data_root,
            passphrase,
            duration_seconds=DESKTOP_AGENT_LEASE_SECONDS,
            capability="read",
            authentication_method="password",
            # A lease that already exists (say, from a terminal) is someone
            # else's session: never replace it, which could narrow it.
            only_if_locked=True,
        )
    except AppError as exc:
        if exc.code != "operator_lease_exists":
            raise
        return {**session_state(ctx), "existing_lease": True}
    finally:
        passphrase[:] = b"\0" * len(passphrase)
    granted_at = time.monotonic()
    ctx.agent_lease = DesktopAgentLease(
        data_root=ctx.data_root,
        lease_id=granted.get("lease_id"),
        expires_at=granted.get("expires_at"),
        expires_monotonic=started + DESKTOP_AGENT_LEASE_SECONDS,
        give_up_monotonic=(
            granted_at + DESKTOP_AGENT_LEASE_SECONDS + _GIVE_UP_AFTER_EXPIRY_SECONDS
        ),
        # The broker reports the mode its grant replaced, read in the same
        # transition; restore only a book that really was manual.
        restore_manual=granted.get("previous_mode") == "manual",
    )
    return session_state(ctx)


def lock(ctx: Any) -> dict[str, Any]:
    end_lease(ctx, raise_errors=True)
    return session_state(ctx)


def expire(ctx: Any) -> None:
    """From the daemon loop: end an expired lease, or retry a failed lock."""

    lease = getattr(ctx, "agent_lease", None)
    if not isinstance(lease, DesktopAgentLease):
        return
    if not (lease.ending or lease.expired()) or time.monotonic() < lease.retry_at:
        return
    end_lease(ctx)
    lease = getattr(ctx, "agent_lease", None)
    if (
        isinstance(lease, DesktopAgentLease)
        and time.monotonic() >= lease.give_up_monotonic
    ):
        # Whatever holds the endpoint now, this lease no longer exists in a
        # broker; stop retrying, and leave the mode to whoever set it since.
        ctx.agent_lease = None
        _LOGGER.warning("desktop agent lease cleanup gave up after its expiry")


def end_lease(ctx: Any, *, raise_errors: bool = False) -> None:
    """Lock the lease this desktop created, and only that one.

    Called whenever the desktop session for the book ends (lock, quit, book
    switch, passphrase rotation, restore), when agents are turned off, and
    after the lease expired. The lease is forgotten only once the broker
    confirms; otherwise it stays visible and the daemon loop retries.
    """

    lease = getattr(ctx, "agent_lease", None)
    if not isinstance(lease, DesktopAgentLease):
        return
    try:
        _lock_in_broker(lease)
    except Exception as exc:
        ctx.agent_lease = replace(
            lease, ending=True, retry_at=time.monotonic() + _RETRY_SECONDS
        )
        _LOGGER.warning(
            "desktop agent lease could not be locked",
            extra={"kb_fields": {"error": exc.__class__.__name__}},
        )
        if raise_errors:
            raise AppError(
                "the operator broker did not confirm locking the agent lease",
                code="agent_lease_lock_failed",
                hint=(
                    "Kassiber keeps retrying while it runs, and the lease ends "
                    "on its own within 8 hours."
                ),
                retryable=True,
            ) from None
        return
    ctx.agent_lease = None


def _lock_in_broker(lease: DesktopAgentLease) -> None:
    from .operator.client import BrokerClient

    client = BrokerClient()
    # One broker transition locks only this lease and restores the mode, so a
    # terminal unlock can neither be ended nor lose its mode.
    result = client.lock(
        lease.data_root,
        # Always conditional: a missing id matches no live lease.
        expected_lease_id=str(lease.lease_id),
        restore_manual=lease.restore_manual,
    )
    if result.get("broker") != "stopped":
        return
    # A lost connection is not proof the broker is gone; only a failed ping
    # is, and a broker's leases end with it.
    try:
        client.ping()
    except (OSError, EOFError):
        # The broker's leases ended with it. Its mode stays as it is: writing
        # it here could race a broker starting up and a new terminal lease.
        return
    raise AppError("the operator broker did not confirm the lock", code="operator_protocol_error")
