"""Saved-report dependencies on local, authoritative chain observations.

This is an attachment to saved reports and the existing change inbox, not a
watch scheduler. Only sync publication and existing watch transitions call it.
Dependencies describe the historical engine input set, not consumed lot shares.
No report bytes, chain graph, acquisition permission or filing state is changed.
"""
from __future__ import annotations

import json
import sqlite3
import uuid
from typing import Any

from ..errors import AppError
from ..time_utils import now_iso
from . import custody_filed_reports as reports
from .ownership_transfers import capture_confirmation_observation
from .onchain import stored_tx_mapping


_OBSERVATION_SELECT = """
 SELECT t.*, w.kind AS wallet_kind, w.config_json AS wallet_config_json,
 o.authority_version AS observation_authority_version,
 o.graph_hash AS observation_graph_hash, o.quantity_hash AS observation_quantity_hash,
 o.observed_at AS observation_observed_at, o.observer_ids_json AS observation_observer_ids,
 o.observer_kinds_json AS observation_observer_kinds, o.fee_attribution AS observation_fee_attribution,
 o.chain AS observation_chain, o.network AS observation_network
 FROM transactions t JOIN wallets w ON w.id=t.wallet_id
 LEFT JOIN chain_observation_provenance o ON o.transaction_id=t.id
"""


def _json(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"))


def _observation(row: sqlite3.Row) -> dict[str, Any] | None:
    closed = capture_confirmation_observation(row)
    if closed is None:
        return None
    payload = stored_tx_mapping(row["raw_json"], allow_nested=True) or {}
    status = payload.get("status") or {}
    block_hash = status.get("block_hash")
    if not isinstance(block_hash, str) or len(block_hash) != 64:
        block_hash = None
    height = status.get("block_height")
    return {
        "status": "observed", "confirmed": closed.confirmed,
        "block_hash": block_hash if closed.confirmed else None,
        "block_height": height if type(height) is int and closed.confirmed else None,
        "quantity_hash": row["observation_quantity_hash"],
    }


def capture_export_dependencies(conn: sqlite3.Connection, snapshot: dict[str, Any]) -> None:
    """Freeze only booked input observations, including earlier pooled basis.

    A wallet/date filter selects report assets, not an independent tax pool.
    All booked inputs of those assets through the report end can affect its
    deterministic engine result. Excluded and unbooked rows never qualify.
    Legacy/external snapshots are deliberately not retroactively reconstructed.
    """
    profile_id = snapshot["profile_id"]
    country = conn.execute("SELECT tax_country FROM profiles WHERE id=?", (profile_id,)).fetchone()[0]
    scope = snapshot["report_scope"]
    wallets = set(scope.get("wallet_ids", ()))
    entries = conn.execute("SELECT transaction_id,wallet_id,asset,occurred_at FROM journal_entries WHERE profile_id=?", (profile_id,)).fetchall()
    assets = set()
    includes_holdings = snapshot["report_kind"] in {"full-report.csv", "full-report.xlsx", "full-report.pdf", "summary.pdf"}
    for row in entries:
        occurred = row["occurred_at"]
        year = reports._entry_year(occurred, tax_country=country)
        if (year is not None and year <= snapshot["period_end_year"]
            and (includes_holdings or snapshot["period_start_year"] <= year)
            and (not wallets or row["wallet_id"] in wallets)
            and (includes_holdings or not scope.get("occurred_at_start") or occurred >= scope["occurred_at_start"])
            and (not scope.get("occurred_at_end") or occurred <= scope["occurred_at_end"])):
            assets.add(row["asset"])
    included = set()
    for row in entries:
        year = reports._entry_year(row["occurred_at"], tax_country=country)
        if (row["asset"] in assets and year is not None and year <= snapshot["period_end_year"]
            and (not scope.get("occurred_at_end") or row["occurred_at"] <= scope["occurred_at_end"])):
            included.add(row["transaction_id"])
    for row in conn.execute(_OBSERVATION_SELECT + " WHERE t.profile_id=? AND t.excluded=0", (profile_id,)):
        if row["id"] not in included:
            continue
        observation = _observation(row)
        closed = capture_confirmation_observation(row)
        if observation is None or closed is None:
            continue
        conn.execute("""INSERT INTO filed_report_chain_dependencies
            (snapshot_id,profile_id,transaction_id,wallet_id,chain,network,txid,observation_json)
            VALUES(?,?,?,?,?,?,?,?)""", (
                snapshot["id"], profile_id, row["id"], row["wallet_id"], *closed.physical_scope,
                _json(observation),
            ))


def _current(conn: sqlite3.Connection, dependency: sqlite3.Row) -> dict[str, Any]:
    from .book_network import guard_observation
    try:
        guard_observation(conn, dependency["profile_id"], {"chain": dependency["chain"], "network": dependency["network"]}, operation="report_dependency")
    except AppError:
        return {"status": "unavailable", "reason": "domain_changed"}
    rows = conn.execute(_OBSERVATION_SELECT + " WHERE t.profile_id=? AND t.wallet_id=? AND LOWER(t.external_id)=?", (
        dependency["profile_id"], dependency["wallet_id"], dependency["txid"],
    )).fetchall()
    # Keep the original authored row identity when present. A deleted/reimported
    # tx may only restore coverage if it has a unique closed wallet observation.
    original = [row for row in rows if row["id"] == dependency["transaction_id"]]
    rows = original or rows
    if len(rows) != 1:
        return {"status": "unavailable", "reason": "observation_missing"}
    closed = capture_confirmation_observation(rows[0])
    if closed is None or closed.physical_scope != (dependency["chain"], dependency["network"], dependency["txid"]):
        return {"status": "unavailable", "reason": "authority_unavailable"}
    return _observation(rows[0]) or {"status": "unavailable", "reason": "authority_unavailable"}


def _last_observation(conn: sqlite3.Connection, dependency: sqlite3.Row) -> dict[str, Any]:
    row = conn.execute("""SELECT inbox.observation_json FROM filed_report_chain_impacts impact
        JOIN chain_analysis_watch_inbox inbox ON inbox.id=impact.inbox_id
        WHERE impact.snapshot_id=? AND impact.transaction_id=? ORDER BY inbox.rowid DESC LIMIT 1""",
        (dependency["snapshot_id"], dependency["transaction_id"])).fetchone()
    return json.loads(row[0])["after"] if row else json.loads(dependency["observation_json"])


def _append(conn: sqlite3.Connection, dependency: sqlite3.Row, before: dict, after: dict, code: str) -> bool:
    if before == after:
        return False
    ident, event_id, timestamp = str(uuid.uuid4()), str(uuid.uuid4()), now_iso()
    conn.execute("""INSERT INTO chain_analysis_watch_inbox
        (id,watch_id,profile_id,sequence,code,observation_json,created_at)
        VALUES(?,NULL,?,0,?,?,?)""", (event_id, dependency["profile_id"], code, _json({"before": before, "after": after}), timestamp))
    conn.execute("""INSERT INTO filed_report_chain_impacts
        (id,snapshot_id,profile_id,transaction_id,inbox_id,created_at) VALUES(?,?,?,?,?,?)""",
        (ident, dependency["snapshot_id"], dependency["profile_id"], dependency["transaction_id"], event_id, timestamp))
    return True


def record_wallet_changes(conn: sqlite3.Connection, profile_id: str, wallet_id: str, *, retracted_txids=()) -> int:
    """Called inside the wallet publication savepoint, after authority is stored."""
    retracted = {str(value).lower() for value in retracted_txids}
    count = 0
    observations = {}
    for dependency in conn.execute("SELECT * FROM filed_report_chain_dependencies WHERE profile_id=? AND wallet_id=?", (profile_id, wallet_id)).fetchall():
        before = _last_observation(conn, dependency)
        key = (dependency["transaction_id"], dependency["chain"], dependency["network"], dependency["txid"])
        if key not in observations:
            observations[key] = _current(conn, dependency)
        after = observations[key]
        if after["status"] == "unavailable" and dependency["txid"] in retracted:
            after = {"status": "retracted", "reason": "authoritative_wallet_retraction"}
        elif after["status"] == "unavailable" and before["status"] == "retracted":
            # Omission on a later successful refresh does not retract the same
            # transaction again or turn the proven removal into a coverage claim.
            continue
        code = ("report_input_retracted" if after["status"] == "retracted" else
                "coverage_lost" if after["status"] == "unavailable" else
                "threshold_reversed" if before.get("confirmed") and not after.get("confirmed") else "evidence_changed")
        count += _append(conn, dependency, before, after, code)
    return count


def record_watch_change(conn: sqlite3.Connection, profile_id: str, definition: dict, after: dict, code: str) -> None:
    """Attach uncertainty from an existing physical confirmation watch only.

    A graph watch cannot retract a wallet accounting input. Coverage loss and
    reversals prompt review; resolution waits for a fresh wallet observation.
    """
    if definition["rule"] != "confirmations" or code not in {"coverage_lost", "threshold_reversed"}:
        return
    parts = str(definition["query"].get("subject", "")).split(":")
    if len(parts) == 1 and len(parts[0]) == 64:
        query = definition["query"]
        parts = [query.get("chain"), query.get("network"), "tx", parts[0]]
    if len(parts) != 4 or parts[2] != "tx":
        return
    for dependency in conn.execute("SELECT * FROM filed_report_chain_dependencies WHERE profile_id=? AND chain=? AND network=? AND txid=?", (profile_id, parts[0], parts[1], parts[3])).fetchall():
        uncertain = {"status": "unavailable", "reason": "watch_coverage_lost" if code == "coverage_lost" else "watch_confirmation_reversed"}
        _append(conn, dependency, _last_observation(conn, dependency), uncertain, code)


def inbox_report_impact(conn: sqlite3.Connection, profile_id: str, inbox_id: str) -> dict[str, Any] | None:
    row = conn.execute("""SELECT impact.*, resolution.rebuilt_at, resolution.summary_json
        FROM filed_report_chain_impacts impact LEFT JOIN filed_report_chain_resolutions resolution ON resolution.impact_id=impact.id
        WHERE impact.profile_id=? AND impact.inbox_id=?""", (profile_id, inbox_id)).fetchone()
    if row is None:
        return None
    snapshot = reports.get_filed_report_snapshot(conn, row["snapshot_id"], profile_id=profile_id)
    return {
        "id": row["id"], "snapshot_id": snapshot["id"], "report_kind": snapshot["report_kind"],
        "report_state": snapshot["report_state"], "content_sha256": snapshot["content_sha256"],
        "period_start_year": snapshot["period_start_year"], "period_end_year": snapshot["period_end_year"],
        "report_scope": snapshot["report_scope"], "transaction_id": row["transaction_id"],
        "transaction_available": conn.execute("SELECT 1 FROM transactions WHERE profile_id=? AND id=?", (profile_id, row["transaction_id"])).fetchone() is not None,
        "before_gain_summary": snapshot["gain_summary"],
        "resolution": json.loads(row["summary_json"]) if row["summary_json"] else None,
        "rebuilt_at": row["rebuilt_at"],
        "filing_review_required": snapshot["report_state"] == "filed",
    }


def resolve_pending_chain_impacts(conn: sqlite3.Connection, profile_id: str, rebuilt_at: str) -> int:
    """Append exact results only from the existing report-ready rebuild seam.

    Rebuild is not evidence recovery: unresolved watch uncertainty requires a
    later sync publication. A confirmed retraction is authoritative removal.
    Filing review remains the user's responsibility, even after arithmetic is
    recomputed or the inbox item is acknowledged.
    """
    rows = conn.execute("""SELECT impact.*, inbox.observation_json FROM filed_report_chain_impacts impact
        JOIN chain_analysis_watch_inbox inbox ON inbox.id=impact.inbox_id
        LEFT JOIN filed_report_chain_resolutions resolution ON resolution.impact_id=impact.id
        WHERE impact.profile_id=? AND resolution.impact_id IS NULL""", (profile_id,)).fetchall()
    count = 0
    for row in rows:
        dependency = conn.execute("SELECT * FROM filed_report_chain_dependencies WHERE snapshot_id=? AND transaction_id=?", (row["snapshot_id"], row["transaction_id"])).fetchone()
        if dependency is None:
            continue
        latest = _last_observation(conn, dependency)
        current = _current(conn, dependency)
        if latest["status"] == "unavailable" or (latest["status"] != "retracted" and (current["status"] != "observed" or not current["confirmed"])):
            continue
        snapshot = reports.get_filed_report_snapshot(conn, row["snapshot_id"], profile_id=profile_id)
        summaries = reports.current_report_summaries(conn, profile_id,
            period_start_year=snapshot["period_start_year"], period_end_year=snapshot["period_end_year"], report_scope=snapshot["report_scope"])
        result = reports.compare_report_summaries(snapshot, summaries)
        conn.execute("INSERT INTO filed_report_chain_resolutions(impact_id,rebuilt_at,summary_json,created_at) VALUES(?,?,?,?)", (row["id"], rebuilt_at, _json(result), now_iso()))
        count += 1
    return count
