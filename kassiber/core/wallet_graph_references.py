"""Bounded, local-cache-only persistence of a synced wallet's graph references."""
from __future__ import annotations

import sqlite3
from time import monotonic
from typing import Any, Callable, Mapping

from . import transaction_graph as graph

MAX_TRANSACTIONS = 50
MAX_PREVOUT_FETCHES = 250
MAX_ROWS_SCANNED = 1000
MAX_RUN_SECONDS = 30


def fill_wallet_graph_references(
    conn: sqlite3.Connection,
    profile_id: str,
    wallet_id: str,
    runtime_config: Mapping[str, Any],
    *,
    backend_name: str,
    check_cancelled: Callable[[], None],
    after_rowid: int = 0,
) -> dict[str, Any]:
    """Called only with a live sync grant; display reads never call this function.

    The scan cursor lives in private freshness checkpoint state. It advances even
    on lookup failures, so a few unavailable transactions cannot starve the rest.
    """
    summary = {"scanned": 0, "attempted": 0, "cached": 0, "skipped": 0, "failed": 0}
    control = graph.GraphLookupControl(check_cancelled, MAX_PREVOUT_FETCHES)
    token = graph._graph_lookup_control.set(control)
    deadline = monotonic() + MAX_RUN_SECONDS
    last_rowid = after_rowid
    try:
        check_cancelled()
        rows = conn.execute(
            """SELECT t.rowid AS scan_rowid, t.*, w.kind AS wallet_kind,
                      w.config_json AS wallet_config_json
               FROM transactions t JOIN wallets w ON w.id = t.wallet_id
               WHERE t.profile_id = ? AND t.wallet_id = ? AND t.rowid > ?
               ORDER BY t.rowid LIMIT ?""",
            (profile_id, wallet_id, after_rowid, MAX_ROWS_SCANNED),
        ).fetchall()
        exhausted = True
        for row in rows:
            check_cancelled()
            if summary["attempted"] >= MAX_TRANSACTIONS or monotonic() >= deadline:
                exhausted = False
                break
            last_rowid = row["scan_rowid"]
            summary["scanned"] += 1
            raw = graph._json_obj(row["raw_json"])
            txid = graph._string_or_none(raw.get("txid")) or graph._txid_from_row(row)
            if not graph._looks_like_txid(txid):
                summary["skipped"] += 1
                continue
            liquid = graph._looks_liquid_or_confidential(row, raw)
            local = graph._enrich_reference_graph_raw(
                conn, row, raw, runtime_config, liquid=liquid, allow_public_lookup=False,
            )
            has_graph = isinstance(local.get("vin"), list) and isinstance(local.get("vout"), list)
            warning = local.get("_graphLookupWarning", {}).get("code")
            if (has_graph and (liquid or graph._bitcoin_current_graph_has_required_prevouts(local))) or warning in {
                "local_reference_conflict", "book_network_mismatch", "invalid_reference_scope",
            } or (not liquid and not graph._can_lookup_public_bitcoin_graph(row, raw)):
                summary["skipped"] += 1
                continue
            summary["attempted"] += 1
            fetched = graph._enrich_reference_graph_raw(
                conn, row, raw, runtime_config, liquid=liquid, allow_public_lookup=True,
                backend_name=backend_name,
            )
            check_cancelled()
            if fetched.get("_graphLookupWarning"):
                summary["failed"] += 1
            else:
                summary["cached"] += 1
        if exhausted and len(rows) < MAX_ROWS_SCANNED:
            last_rowid = 0
        return {
            **summary, "prevouts_requested": control.prevouts_requested,
            "freshness_checkpoint": {"after_rowid": last_rowid},
        }
    finally:
        graph._graph_lookup_control.reset(token)
