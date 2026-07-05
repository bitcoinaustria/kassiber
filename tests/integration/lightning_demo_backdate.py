#!/usr/bin/env python3
"""Backdate demo Lightning history across the full-accounting time window.

Real ``lightningd``/``lnd`` stamp invoice/payment settle times from their own
wall clock, and ``bitcoind setmocktime`` (how the on-chain demo backdates) does
not reach them. So the regtest demo imports every Lightning row at "now", which
makes the ledger show years of on-chain history next to a single burst of
Lightning activity.

This module is a **demo-only** post-import step. After ``wallets sync`` has
imported the real Lightning rows (real payment hashes, amounts, fees) it rewrites
their ``occurred_at`` so they spread across the same ``base_time``..``latest_time``
window the on-chain scenario uses. It never touches production code paths, and it
only rewrites timestamps — amounts, hashes, channels and live balances are left
exactly as the node reported them. The node snapshot (channels/balances) stays
"now", which is correct: it is a live view, exactly like the on-chain wallet
balance is "now" while its transactions span years.

Determinism: dates are a pure function of ``(seed, payment_hash)``, so re-running
after a ``demo-tick`` re-sync reassigns the *same* dates. The import layer only
overwrites ``occurred_at`` when it is ``UNKNOWN_OCCURRED_AT`` (see
``kassiber/core/imports.py``), so a live re-sync will not revert the backdated
value.
"""

from __future__ import annotations

import hashlib
import json
import os
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any, Iterable, Mapping, Sequence

from kassiber.core.lightning.cln import _stable_hash
from kassiber.db import open_db, resolve_database_path

# Transaction kinds emitted by the Lightning adapters (see cln.py/lnd.py import
# helpers). Only these ledger rows are backdated.
LN_TRANSACTION_KINDS = ("cln_invoice", "cln_pay", "lnd_invoice", "lnd_pay")

# lightning_node_records.record_type values that carry a payment_hash and should
# align with their derived transaction rows. ``balance`` rows are point-in-time
# ("now") and are intentionally left alone; ``forward_day`` rows are rebucketed
# separately because their external_id encodes the day.
LN_RECORD_TYPES_WITH_HASH = ("income", "invoice", "pay")


def _parse_iso(value: str | datetime) -> datetime:
    if isinstance(value, datetime):
        return value.astimezone(timezone.utc)
    return datetime.fromisoformat(str(value).replace("Z", "+00:00")).astimezone(
        timezone.utc
    )


def _to_iso(dt: datetime) -> str:
    return dt.astimezone(timezone.utc).isoformat().replace("+00:00", "Z")


def _key_hash(key: str, seed: int, salt: str) -> int:
    digest = hashlib.sha256(f"{seed}:{salt}:{key}".encode("utf-8")).digest()
    return int.from_bytes(digest[:8], "big")


def assign_historical_dates(
    stable_keys: Iterable[str],
    window_start: str | datetime,
    window_end: str | datetime,
    *,
    seed: int = 0,
) -> dict[str, str]:
    """Spread ``stable_keys`` across ``[window_start, window_end]`` deterministically.

    Keys are sorted, then placed at evenly-spaced slots across the window with a
    per-key seeded jitter (bounded to its slot) and a per-key business-hours
    timestamp. The mapping is a pure function of ``(seed, key)`` given a stable
    key set, so re-runs are idempotent. Returns ``{key: iso8601}``.
    """
    start = _parse_iso(window_start)
    end = _parse_iso(window_end)
    if start >= end:
        raise ValueError("window_start must be strictly before window_end")

    keys = sorted({key for key in stable_keys if key})
    count = len(keys)
    if count == 0:
        return {}

    span_seconds = (end - start).total_seconds()
    slot = span_seconds / (2 * count)
    result: dict[str, str] = {}
    for index, key in enumerate(keys):
        base = span_seconds * (index + 0.5) / count
        # Deterministic jitter in [-slot, +slot] so rows are not perfectly even.
        jitter = ((_key_hash(key, seed, "jitter") % 20_001) / 20_000.0 - 0.5) * 2 * slot
        offset = max(0.0, min(span_seconds, base + jitter))
        moment = start + timedelta(seconds=offset)
        # Business-hours time-of-day (06:00–17:59) so rows read like real sales.
        hour = 6 + (_key_hash(key, seed, "hour") % 12)
        minute = _key_hash(key, seed, "minute") % 60
        moment = moment.replace(hour=hour, minute=minute, second=0, microsecond=0)
        if moment < start:
            moment = start
        elif moment > end:
            moment = end
        result[key] = _to_iso(moment)
    return result


def _resolve_profile_ids(
    conn: Any,
    workspace_label: str | None,
    profile_label: str | None,
) -> list[str]:
    if not workspace_label or not profile_label:
        return []
    rows = conn.execute(
        """
        SELECT p.id AS id
        FROM profiles p
        JOIN workspaces ws ON ws.id = p.workspace_id
        WHERE lower(ws.label) = lower(?) AND lower(p.label) = lower(?)
        """,
        (workspace_label, profile_label),
    ).fetchall()
    return [str(row["id"]) for row in rows]


def _profile_filter(profile_ids: Sequence[str], column: str = "profile_id") -> tuple[str, list[str]]:
    if not profile_ids:
        return "", []
    placeholders = ", ".join("?" for _ in profile_ids)
    return f" AND {column} IN ({placeholders})", list(profile_ids)


def _rebucket_forward_day_records(
    conn: Any,
    profile_ids: Sequence[str],
    date_iso_map: Mapping[str, str],
) -> int:
    """Delete-and-reaggregate ``forward_day`` rows onto their new days.

    Several forwards can collapse onto the same new day/channel, which would
    collide on ``external_id`` (``_stable_hash(("fw_day", day, channel_id))``).
    So we sum ``(amount_msat, fee_msat, forward_count)`` per new
    ``(day, channel_id)`` and reinsert with the recomputed id.
    """
    clause, params = _profile_filter(profile_ids)
    rows = conn.execute(
        f"SELECT * FROM lightning_node_records WHERE record_type = 'forward_day'{clause}",
        params,
    ).fetchall()
    if not rows:
        return 0

    # Group source rows onto their reassigned day, scoped per identity so two
    # wallets never merge their forwards.
    merged: dict[tuple[str, str, str], dict[str, Any]] = {}
    for row in rows:
        external_id = str(row["external_id"])
        new_iso = date_iso_map.get(external_id)
        if not new_iso:
            continue
        new_day = new_iso[:10]
        channel_id = str(row["channel_id"] or "unknown")
        identity = (
            str(row["workspace_id"]),
            str(row["profile_id"]),
            str(row["wallet_id"]),
            str(row["backend_name"]),
            row["node_id"],
        )
        key = (identity, new_day, channel_id)
        bucket = merged.setdefault(
            key,  # type: ignore[arg-type]
            {
                "identity": identity,
                "day": new_day,
                "channel_id": channel_id,
                "amount_msat": 0,
                "fee_msat": 0,
                "forward_count": 0,
                "currency": row["currency"],
                "sync_id": row["sync_id"],
                "first_seen_at": row["first_seen_at"],
                "updated_at": row["updated_at"],
                "raw_json": row["raw_json"] or "{}",
            },
        )
        bucket["amount_msat"] += int(row["amount_msat"] or 0)
        bucket["fee_msat"] += int(row["fee_msat"] or 0)
        bucket["forward_count"] += int(json.loads(row["raw_json"] or "{}").get("forward_count") or 0) or 1

    conn.execute(
        f"DELETE FROM lightning_node_records WHERE record_type = 'forward_day'{clause}",
        params,
    )

    inserted = 0
    for bucket in merged.values():
        workspace_id, profile_id, wallet_id, backend_name, node_id = bucket["identity"]
        day = bucket["day"]
        channel_id = bucket["channel_id"]
        external_id = _stable_hash(("fw_day", day, channel_id))
        raw = json.loads(bucket["raw_json"] or "{}")
        raw["forward_count"] = bucket["forward_count"]
        conn.execute(
            """
            INSERT INTO lightning_node_records (
                id, workspace_id, profile_id, wallet_id, backend_name, node_id,
                record_type, external_id, occurred_at, account, peer_id,
                channel_id, direction, amount_msat, fee_msat, tag, status,
                currency, payment_hash, txid, outpoint, sync_id, raw_json,
                first_seen_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, 'forward_day', ?, ?, ?, NULL, ?, 'inbound',
                      ?, ?, 'routed', NULL, ?, NULL, NULL, NULL, ?, ?, ?, ?)
            """,
            (
                f"{wallet_id}:{backend_name}:forward_day:{external_id}",
                workspace_id,
                profile_id,
                wallet_id,
                backend_name,
                node_id,
                external_id,
                f"{day}T00:00:00Z",
                channel_id,
                channel_id,
                bucket["amount_msat"],
                bucket["fee_msat"],
                bucket["currency"],
                bucket["sync_id"],
                bucket["first_seen_at"],
                bucket["updated_at"],
                json.dumps(raw, separators=(",", ":"), sort_keys=True),
            ),
        )
        inserted += 1
    return inserted


def _load_window(scenario_path: str | None) -> tuple[str, str]:
    if not scenario_path:
        raise ValueError("scenario_path is required to derive the backdate window")
    data = json.loads(Path(scenario_path).read_text(encoding="utf-8"))
    base_time = str(data["base_time"])
    latest_time = str(data["latest_time"])
    return base_time, latest_time


def backdate_ln_records(
    data_root: str | Path,
    scenario_path: str | None,
    *,
    workspace_label: str | None = None,
    profile_label: str | None = None,
    seed: int = 0,
) -> dict[str, Any]:
    """Rewrite imported Lightning ``occurred_at`` across the scenario window."""
    window_start, window_end = _load_window(scenario_path)
    db_path = resolve_database_path(Path(data_root))
    if not db_path.exists():
        raise FileNotFoundError(f"No demo book database at {db_path}")

    conn = open_db(Path(data_root))
    try:
        profile_ids = _resolve_profile_ids(conn, workspace_label, profile_label)
        tx_clause, tx_params = _profile_filter(profile_ids)
        kind_placeholders = ", ".join("?" for _ in LN_TRANSACTION_KINDS)

        tx_rows = conn.execute(
            f"""
            SELECT id, payment_hash
            FROM transactions
            WHERE kind IN ({kind_placeholders}){tx_clause}
            """,
            [*LN_TRANSACTION_KINDS, *tx_params],
        ).fetchall()

        rec_clause, rec_params = _profile_filter(profile_ids)
        rt_placeholders = ", ".join("?" for _ in LN_RECORD_TYPES_WITH_HASH)
        rec_rows = conn.execute(
            f"""
            SELECT id, external_id, payment_hash
            FROM lightning_node_records
            WHERE record_type IN ({rt_placeholders}){rec_clause}
            """,
            [*LN_RECORD_TYPES_WITH_HASH, *rec_params],
        ).fetchall()

        fwd_clause, fwd_params = _profile_filter(profile_ids)
        fwd_rows = conn.execute(
            f"SELECT external_id FROM lightning_node_records WHERE record_type = 'forward_day'{fwd_clause}",
            fwd_params,
        ).fetchall()

        # A single date map keyed by payment_hash so a transaction and its source
        # node record (same hash) land on the same day. Rows without a hash fall
        # back to their own id/external_id so they still get a stable date.
        hash_keys = {
            str(row["payment_hash"]) for row in tx_rows if row["payment_hash"]
        } | {str(row["payment_hash"]) for row in rec_rows if row["payment_hash"]}
        tx_fallback_keys = {
            f"tx:{row['id']}" for row in tx_rows if not row["payment_hash"]
        }
        rec_fallback_keys = {
            f"rec:{row['external_id']}" for row in rec_rows if not row["payment_hash"]
        }
        forward_keys = {str(row["external_id"]) for row in fwd_rows}

        date_map = assign_historical_dates(
            hash_keys | tx_fallback_keys | rec_fallback_keys | forward_keys,
            window_start,
            window_end,
            seed=seed,
        )

        updated_tx = 0
        for row in tx_rows:
            key = str(row["payment_hash"]) if row["payment_hash"] else f"tx:{row['id']}"
            new_iso = date_map.get(key)
            if not new_iso:
                continue
            conn.execute(
                "UPDATE transactions SET occurred_at = ?, confirmed_at = ? WHERE id = ?",
                (new_iso, new_iso, row["id"]),
            )
            updated_tx += 1

        updated_rec = 0
        for row in rec_rows:
            key = (
                str(row["payment_hash"])
                if row["payment_hash"]
                else f"rec:{row['external_id']}"
            )
            new_iso = date_map.get(key)
            if not new_iso:
                continue
            conn.execute(
                "UPDATE lightning_node_records SET occurred_at = ? WHERE id = ?",
                (new_iso, row["id"]),
            )
            updated_rec += 1

        rebucketed = _rebucket_forward_day_records(conn, profile_ids, date_map)

        conn.commit()
    finally:
        conn.close()

    span_days = (
        _parse_iso(window_end) - _parse_iso(window_start)
    ).days
    return {
        "window_start": window_start,
        "window_end": window_end,
        "span_days": span_days,
        "transactions_backdated": updated_tx,
        "node_records_backdated": updated_rec,
        "forward_day_rows": rebucketed,
        "seed": seed,
    }


def run() -> dict[str, Any]:
    data_root = os.environ.get("KASSIBER_LIGHTNING_BUSINESS_DATA_ROOT") or os.environ.get(
        "KASSIBER_DEMO_DATA_ROOT"
    )
    if not data_root:
        raise SystemExit(
            "Set KASSIBER_LIGHTNING_BUSINESS_DATA_ROOT to the demo data root."
        )
    scenario_path = os.environ.get("KASSIBER_DEMO_BACKDATE_SCENARIO")
    seed = int(os.environ.get("KASSIBER_DEMO_BACKDATE_SEED") or "0")
    workspace_label = os.environ.get("KASSIBER_LIGHTNING_BUSINESS_WORKSPACE")
    profile_label = os.environ.get("KASSIBER_LIGHTNING_BUSINESS_PROFILE")
    summary = backdate_ln_records(
        data_root,
        scenario_path,
        workspace_label=workspace_label,
        profile_label=profile_label,
        seed=seed,
    )
    print(json.dumps(summary, indent=2))
    return summary


if __name__ == "__main__":  # pragma: no cover
    run()
    sys.exit(0)
