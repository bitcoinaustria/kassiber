"""Unlocked-book local work lifecycle, with identity-fenced worker connections.

worker_tick is the integration seam for other explicitly authorized local work.
It never inherits the freshness worker's transport grants or provider settings.
"""
from __future__ import annotations

import logging
import threading

from .db import database_instance_id, open_db
from .envelope import build_event_envelope
from .errors import AppError
from .time_utils import now_iso
from .core import chain_analysis_watches as watches

_LOG = logging.getLogger(__name__)
POLL_SECONDS = 5


def has_pending_work(conn):
    return conn.execute("SELECT 1 FROM chain_analysis_watches WHERE enabled=1 LIMIT 1").fetchone() is not None


def worker_tick(conn, *, cancelled=lambda: False):
    """One catch-up pass. Root may compose backfill before this local pass."""
    if not has_pending_work(conn):
        return
    for row in conn.execute("SELECT DISTINCT profile_id FROM chain_analysis_watches WHERE enabled=1 ORDER BY profile_id").fetchall():
        watches.evaluate_due(conn, row["profile_id"], cancelled=cancelled)
        conn.commit()


def deliver_pending(conn, out, *, cancelled=lambda: False):
    """At-least-once generic wake-up; inbox acknowledgement is independent.

    A UI that was closed/missed the event reads the durable inbox on reconnect.
    Repeated generic wakes dedupe in the UI; identities never leave this table.
    """
    rows = conn.execute("SELECT id FROM chain_analysis_watch_inbox WHERE delivered_at IS NULL AND acknowledged_at IS NULL LIMIT 100").fetchall()
    if not rows or cancelled():
        return
    out.write(build_event_envelope("ui.chain_analysis.watches.changed", {"local_only": True}))
    if cancelled():
        return
    conn.executemany("UPDATE chain_analysis_watch_inbox SET delivered_at=? WHERE id=?", [(now_iso(), row["id"]) for row in rows])
    conn.commit()


def start_worker(ctx, *, passphrase=None):
    if ctx.conn is None or getattr(ctx, "watch_worker", None) is not None and ctx.watch_worker.is_alive():
        return
    try:
        watches.require_encrypted(ctx.conn)
    except AppError:
        return
    identity = database_instance_id(ctx.conn)
    root = ctx.data_root
    secret = passphrase or getattr(ctx, "db_passphrase", None)
    stop = threading.Event()
    ctx.watch_stop_event = stop

    def run():
        conn = None
        try:
            conn = open_db(root, passphrase=secret, require_existing_schema=True, expected_database_identity=identity)
            while not stop.is_set():
                try:
                    worker_tick(conn, cancelled=stop.is_set)
                    deliver_pending(conn, ctx.out, cancelled=stop.is_set)
                except Exception as error:
                    conn.rollback()
                    if not stop.is_set():
                        # No source IDs, error text, database path or private
                        # watch title enters operational output.
                        _LOG.warning("Local evidence watch pass deferred (%s)", type(error).__name__)
                stop.wait(POLL_SECONDS)
        except Exception as error:
            _LOG.warning("Local evidence watch worker unavailable (%s)", type(error).__name__)
        finally:
            if conn is not None:
                conn.close()

    ctx.watch_worker = threading.Thread(target=run, name="local-evidence-watches", daemon=True)
    ctx.watch_worker.start()


def stop_worker(ctx, *, require_stopped=True):
    worker = getattr(ctx, "watch_worker", None)
    if worker is None:
        return True
    ctx.watch_stop_event.set()
    worker.join(timeout=2)
    if worker.is_alive():
        if require_stopped:
            raise AppError("Local evidence work is stopping; try again", code="project_operation_in_progress", retryable=True)
        return False
    ctx.watch_worker = None
    return True
