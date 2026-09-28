"""Streaming "Verify on chain" for the desktop Reconcile screen.

The on-chain tier looks transactions up one at a time on a backend, which on a
long list outlives an ordinary request. So the database work runs on the main
thread (``prepare_wallet_identify_onchain``), and a worker thread does only the
lookups and the pure classification (``finish_identify``): it writes
``ui.wallets.identify_onchain.progress`` records as it goes, the terminal
report at the end, and stops early when ``ui.wallets.identify_onchain.cancel``
names its request. The worker never touches SQLite.
"""

from __future__ import annotations

import threading
import time
from typing import Any, Callable

from .core import ownership as core_ownership
from .core import sync_backends as core_sync_backends
from .core.ui_snapshot import empty_identify_payload, prepare_wallet_identify_onchain
from .envelope import build_envelope, build_error_envelope
from .errors import AppError
from .redaction import redact_secret_text

KIND = "ui.wallets.identify_onchain"
CANCEL_KIND = "ui.wallets.identify_onchain.cancel"
PROGRESS_KIND = "ui.wallets.identify_onchain.progress"
# A cancel can race ahead of the request it names; it is held this long.
PENDING_CANCEL_TTL_SECONDS = 30.0
MAX_PENDING_CANCELS = 32


class ActiveVerifications:
    """Cancel flags for running verifications, keyed by request id."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._running: dict[str, threading.Event] = {}
        self._pending: dict[str, float] = {}

    def register(self, key: str | None) -> threading.Event:
        event = threading.Event()
        if key is None:
            return event
        now = time.monotonic()
        with self._lock:
            self._pending = {k: d for k, d in self._pending.items() if d >= now}
            if self._pending.pop(key, None) is not None:
                event.set()
            self._running[key] = event
        return event

    def unregister(self, key: str | None, event: threading.Event) -> None:
        if key is None:
            return
        with self._lock:
            if self._running.get(key) is event:
                self._running.pop(key, None)

    def cancel(self, key: str) -> bool:
        with self._lock:
            event = self._running.get(key)
            if event is not None:
                event.set()
                return True
            if len(self._pending) >= MAX_PENDING_CANCELS:
                self._pending.pop(next(iter(self._pending)))
            self._pending[key] = time.monotonic() + PENDING_CANCEL_TTL_SECONDS
        return False


def _with_request_id(envelope: dict[str, Any], request_id: object) -> dict[str, Any]:
    envelope["request_id"] = request_id
    return envelope


def _error(request_id: object, exc: BaseException) -> dict[str, Any]:
    if isinstance(exc, AppError):
        return _with_request_id(
            build_error_envelope(
                exc.code,
                redact_secret_text(str(exc)),
                hint=redact_secret_text(exc.hint) if exc.hint else None,
                retryable=bool(getattr(exc, "retryable", False)),
            ),
            request_id,
        )
    return _with_request_id(
        build_error_envelope("internal", "On-chain verification failed"),
        request_id,
    )


def _run(
    out: Any,
    request_id: object,
    plan: core_ownership.IdentifyPlan,
    backend: dict[str, Any],
    book: dict[str, Any],
    cancel_event: threading.Event,
    done: Callable[[], None],
) -> None:
    def emit(checked: int, total: int) -> None:
        out.write(
            _with_request_id(
                build_envelope(PROGRESS_KIND, {"checked": checked, "total": total}),
                request_id,
            )
        )

    try:
        emit(0, len(plan.pending))
        with core_sync_backends.verify_session(backend) as fetcher:
            report = core_ownership.finish_identify(
                plan, fetcher, progress=emit, cancelled=cancel_event.is_set
            )
        report["context"] = book
        report["cancelled"] = cancel_event.is_set()
        out.write(_with_request_id(build_envelope(KIND, report), request_id))
    except Exception as exc:  # noqa: BLE001 - the terminal record must always be written
        out.write(_error(request_id, exc))
    finally:
        done()


def start(
    ctx: Any, out: Any, request_id: object, key: str | None, args: Any
) -> dict[str, Any] | None:
    """Prepare on this thread; return the report now or stream it from a worker.

    Returns the terminal envelope when nothing needs the network (no pending
    txids), else ``None`` after the worker has started.
    """
    prepared = prepare_wallet_identify_onchain(ctx.conn, ctx.runtime_config, args)
    if prepared is None:
        return _with_request_id(build_envelope(KIND, empty_identify_payload()), request_id)
    plan, backend, book = prepared
    if not plan.pending:
        report = core_ownership.finish_identify(plan)
        report["context"] = book
        return _with_request_id(build_envelope(KIND, report), request_id)
    registry: ActiveVerifications = ctx.active_verifications
    cancel_event = registry.register(key)
    threading.Thread(
        target=_run,
        args=(
            out,
            request_id,
            plan,
            backend,
            book,
            cancel_event,
            lambda: registry.unregister(key, cancel_event),
        ),
        daemon=True,
        name="kassiber-identify-onchain",
    ).start()
    return None


def cancel(ctx: Any, request_id: object, args: Any) -> dict[str, Any]:
    target = args.get("target_request_id") if isinstance(args, dict) else None
    if not isinstance(target, str) or not target:
        raise AppError(
            f"{CANCEL_KIND} requires target_request_id",
            code="validation",
            hint=f"Pass {{target_request_id: '<active {KIND} request_id>'}}.",
        )
    running = ctx.active_verifications.cancel(target)
    return _with_request_id(
        build_envelope(CANCEL_KIND, {"cancelled": True, "running": running}),
        request_id,
    )
