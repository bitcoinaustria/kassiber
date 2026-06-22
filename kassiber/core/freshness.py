"""SQLite-backed source freshness and daemon job orchestration helpers."""

from __future__ import annotations

import hashlib
import json
import logging
import sqlite3
import uuid
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from typing import Any, Callable, Mapping

from ..envelope import json_ready
from ..errors import AppError
from ..time_utils import now_iso, parse_iso_datetime_or_none

# Job failures recorded here also flow to the RAM-only log ring via the stdlib
# logging bridge (RingHandler), so they surface on the Logs screen rather than
# living only in structured job state.
_LOGGER = logging.getLogger(__name__)

JOB_ONCHAIN_WALLET = "onchain_wallet_history"
JOB_BTCPAY_WALLET = "btcpay_wallet_source"
JOB_BTCPAY_PROVENANCE = "btcpay_provenance"
JOB_MARKET_RATES = "market_rate_coverage"
JOB_JOURNAL_REFRESH = "journal_refresh"
JOB_TYPES = frozenset(
    {
        JOB_ONCHAIN_WALLET,
        JOB_BTCPAY_WALLET,
        JOB_BTCPAY_PROVENANCE,
        JOB_MARKET_RATES,
        JOB_JOURNAL_REFRESH,
    }
)

SOURCE_ONCHAIN = "onchain_wallet"
SOURCE_BTCPAY_WALLET = "btcpay_wallet"
SOURCE_BTCPAY_PROVENANCE = "btcpay_provenance"
SOURCE_RATES = "market_rates"
SOURCE_JOURNALS = "journals"

STATUS_FRESH = "fresh"
STATUS_QUEUED = "queued"
STATUS_SYNCING = "syncing"
STATUS_PAUSED = "paused"
STATUS_RATE_LIMITED = "rate_limited"
STATUS_PARTIALLY_STALE = "partially_stale"
STATUS_FAILED = "failed"
STATUS_BLOCKING_REPORTS = "blocking_reports"

JOB_QUEUED = "queued"
JOB_RUNNING = "running"
JOB_DONE = "done"
JOB_ERROR = "error"
JOB_CANCELLED = "cancelled"
JOB_RATE_LIMITED = "rate_limited"

PHASE_DISCOVERY = "discovery"
PHASE_BACKEND_FETCH = "backend_fetch"
PHASE_DECODE_ENRICH = "decode_enrich"
PHASE_IMPORT = "import"
PHASE_RATE_COVERAGE = "rate_coverage"
PHASE_JOURNAL_REFRESH = "journal_refresh"
PHASE_DONE = "done"
PHASE_ERROR = "error"

POLICY_SETTING_PREFIX = "freshness.policy.profile."
LEGACY_AUTO_SYNC_PREFIX = "ai.auto_sync_before_report_reads.profile."
REDACTED_TEXT = "<redacted>"
DEFAULT_BACKOFF_SECONDS = 30
MAX_BACKOFF_SECONDS = 6 * 60 * 60

JobHandler = Callable[
    [sqlite3.Connection, Mapping[str, Any], Callable[[Mapping[str, Any]], None], Callable[[], None]],
    Mapping[str, Any],
]
ProgressObserver = Callable[[Mapping[str, Any]], None]


@dataclass(frozen=True)
class FreshnessPolicy:
    background_enabled: bool
    source_classes: Mapping[str, bool]
    report_read_sync: bool

    def to_payload(self) -> dict[str, Any]:
        return {
            "background_enabled": self.background_enabled,
            "source_classes": dict(sorted(self.source_classes.items())),
            "report_read_sync": self.report_read_sync,
        }


def _json_dump(value: Any) -> str:
    return json.dumps(json_ready(value), sort_keys=True, separators=(",", ":"))


def _json_load(value: Any, default: Any) -> Any:
    if value in (None, ""):
        return default
    try:
        loaded = json.loads(value)
    except (TypeError, ValueError, json.JSONDecodeError):
        return default
    return loaded if loaded is not None else default


def _parse_iso(value: str | None) -> datetime | None:
    return parse_iso_datetime_or_none(value)


def _iso_from_now_plus(seconds: int) -> str:
    target = datetime.now(timezone.utc) + timedelta(seconds=max(0, int(seconds)))
    return target.replace(microsecond=0).isoformat().replace("+00:00", "Z")


def _stable_jitter_seconds(seed: str, max_seconds: int) -> int:
    if max_seconds <= 0:
        return 0
    digest = hashlib.sha256(seed.encode("utf-8")).digest()
    return int.from_bytes(digest[:2], "big") % (max_seconds + 1)


def default_policy() -> FreshnessPolicy:
    source_classes = {
        SOURCE_ONCHAIN: False,
        SOURCE_BTCPAY_WALLET: False,
        SOURCE_BTCPAY_PROVENANCE: False,
        SOURCE_RATES: True,
        SOURCE_JOURNALS: True,
    }
    return FreshnessPolicy(
        background_enabled=False,
        source_classes=source_classes,
        report_read_sync=False,
    )


def policy_setting_key(profile_id: str) -> str:
    return f"{POLICY_SETTING_PREFIX}{profile_id}"


def legacy_auto_sync_setting_key(profile_id: str) -> str:
    return f"{LEGACY_AUTO_SYNC_PREFIX}{profile_id}"


def get_policy(conn: sqlite3.Connection, profile_id: str) -> FreshnessPolicy:
    row = conn.execute(
        "SELECT value FROM settings WHERE key = ?",
        (policy_setting_key(profile_id),),
    ).fetchone()
    default = default_policy()
    if row is None:
        legacy = conn.execute(
            "SELECT value FROM settings WHERE key = ?",
            (legacy_auto_sync_setting_key(profile_id),),
        ).fetchone()
        if legacy and str(legacy["value"]).strip().lower() in {"1", "true", "yes", "on"}:
            source_classes = dict(default.source_classes)
            source_classes[SOURCE_ONCHAIN] = True
            source_classes[SOURCE_BTCPAY_WALLET] = True
            source_classes[SOURCE_BTCPAY_PROVENANCE] = True
            return FreshnessPolicy(
                background_enabled=False,
                source_classes=source_classes,
                report_read_sync=True,
            )
        return default
    payload = _json_load(row["value"], {})
    if not isinstance(payload, dict):
        return default
    source_classes = dict(default.source_classes)
    configured = payload.get("source_classes")
    if isinstance(configured, dict):
        for key, value in configured.items():
            if key in source_classes:
                source_classes[key] = bool(value)
    return FreshnessPolicy(
        background_enabled=bool(payload.get("background_enabled", default.background_enabled)),
        source_classes=source_classes,
        report_read_sync=bool(payload.get("report_read_sync", default.report_read_sync)),
    )


def set_policy(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    background_enabled: bool | None = None,
    report_read_sync: bool | None = None,
    source_classes: Mapping[str, bool] | None = None,
) -> FreshnessPolicy:
    current = get_policy(conn, profile_id)
    merged_classes = dict(current.source_classes)
    if source_classes is not None:
        for key, value in source_classes.items():
            if key not in merged_classes:
                raise AppError(
                    f"Unsupported freshness source class '{key}'",
                    code="validation",
                    retryable=False,
                    details={"source_class": key},
                )
            merged_classes[key] = bool(value)
    updated = FreshnessPolicy(
        background_enabled=current.background_enabled
        if background_enabled is None
        else bool(background_enabled),
        source_classes=merged_classes,
        report_read_sync=current.report_read_sync
        if report_read_sync is None
        else bool(report_read_sync),
    )
    conn.execute(
        """
        INSERT INTO settings(key, value)
        VALUES(?, ?)
        ON CONFLICT(key) DO UPDATE SET value = excluded.value
        """,
        (policy_setting_key(profile_id), _json_dump(updated.to_payload())),
    )
    return updated


def source_key(kind: str, identity: str) -> str:
    return f"{kind}:{identity}"


def rate_source_key(profile_id: str) -> str:
    return source_key(SOURCE_RATES, profile_id)


def journal_source_key(profile_id: str) -> str:
    return source_key(SOURCE_JOURNALS, profile_id)


def _row_payload(row: sqlite3.Row) -> dict[str, Any]:
    payload = dict(row)
    for key in ("payload_json", "progress_json", "result_json", "error_json", "checkpoint_json"):
        if key in payload:
            payload[key[:-5] if key.endswith("_json") else key] = _json_load(payload.pop(key), {})
    for key in ("blocking_reports", "paused", "cancel_requested"):
        if key in payload:
            payload[key] = bool(payload[key])
    return payload


def _normalize_source_status(
    *,
    status: str,
    paused: bool = False,
    blocking_reports: bool = False,
    rate_limited_until: str | None = None,
) -> str:
    if paused:
        return STATUS_PAUSED
    retry_at = _parse_iso(rate_limited_until)
    if retry_at is not None and retry_at > datetime.now(timezone.utc):
        return STATUS_RATE_LIMITED
    if blocking_reports:
        return STATUS_BLOCKING_REPORTS
    return status


def upsert_source_state(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    source_key: str,
    source_type: str,
    source_label: str,
    status: str,
    state: str | None = None,
    stale_reason: str | None = None,
    blocking_reports: bool = False,
    paused: bool | None = None,
    rate_limited_until: str | None = None,
    cooldown_reason: str | None = None,
    retry_count: int | None = None,
    last_success_at: str | None = None,
    last_error_at: str | None = None,
    last_error_code: str | None = None,
    last_error_message: str | None = None,
    last_phase: str | None = None,
    progress: Mapping[str, Any] | None = None,
    checkpoint: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    existing = conn.execute(
        """
        SELECT *
        FROM freshness_source_states
        WHERE profile_id = ? AND source_key = ?
        """,
        (profile_id, source_key),
    ).fetchone()
    previous = _row_payload(existing) if existing else {}
    final_paused = bool(previous.get("paused")) if paused is None else bool(paused)
    final_retry_count = (
        int(previous.get("retry_count") or 0)
        if retry_count is None
        else int(retry_count)
    )
    final_checkpoint = (
        checkpoint
        if checkpoint is not None
        else previous.get("checkpoint", {})
    )
    final_progress = progress if progress is not None else previous.get("progress", {})
    now = now_iso()
    effective_status = _normalize_source_status(
        status=status,
        paused=final_paused,
        blocking_reports=blocking_reports,
        rate_limited_until=rate_limited_until,
    )
    conn.execute(
        """
        INSERT INTO freshness_source_states(
            profile_id, source_key, source_type, source_label, status, state,
            stale_reason, blocking_reports, paused, rate_limited_until,
            cooldown_reason, retry_count, last_success_at, last_error_at,
            last_error_code, last_error_message, last_phase, progress_json,
            checkpoint_json, updated_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(profile_id, source_key) DO UPDATE SET
            source_type = excluded.source_type,
            source_label = excluded.source_label,
            status = excluded.status,
            state = excluded.state,
            stale_reason = excluded.stale_reason,
            blocking_reports = excluded.blocking_reports,
            paused = excluded.paused,
            rate_limited_until = excluded.rate_limited_until,
            cooldown_reason = excluded.cooldown_reason,
            retry_count = excluded.retry_count,
            last_success_at = COALESCE(excluded.last_success_at, freshness_source_states.last_success_at),
            last_error_at = excluded.last_error_at,
            last_error_code = excluded.last_error_code,
            last_error_message = excluded.last_error_message,
            last_phase = excluded.last_phase,
            progress_json = excluded.progress_json,
            checkpoint_json = excluded.checkpoint_json,
            updated_at = excluded.updated_at
        """,
        (
            profile_id,
            source_key,
            source_type,
            source_label,
            effective_status,
            state or effective_status,
            stale_reason,
            1 if blocking_reports else 0,
            1 if final_paused else 0,
            rate_limited_until,
            cooldown_reason,
            final_retry_count,
            last_success_at,
            last_error_at,
            last_error_code,
            last_error_message,
            last_phase,
            _json_dump(redact_freshness_payload(final_progress)),
            _json_dump(final_checkpoint or {}),
            now,
        ),
    )
    return get_source_state(conn, profile_id, source_key) or {}


def get_source_state(
    conn: sqlite3.Connection,
    profile_id: str,
    key: str,
) -> dict[str, Any] | None:
    row = conn.execute(
        """
        SELECT *
        FROM freshness_source_states
        WHERE profile_id = ? AND source_key = ?
        """,
        (profile_id, key),
    ).fetchone()
    return _row_payload(row) if row else None


def list_source_states(conn: sqlite3.Connection, profile_id: str) -> list[dict[str, Any]]:
    rows = conn.execute(
        """
        SELECT *
        FROM freshness_source_states
        WHERE profile_id = ?
        ORDER BY source_type ASC, source_label ASC, source_key ASC
        """,
        (profile_id,),
    ).fetchall()
    return [_row_payload(row) for row in rows]


def pause_source(conn: sqlite3.Connection, profile_id: str, key: str) -> dict[str, Any]:
    state = get_source_state(conn, profile_id, key)
    if state is None:
        raise AppError("Freshness source was not found", code="not_found")
    return upsert_source_state(
        conn,
        profile_id=profile_id,
        source_key=key,
        source_type=state["source_type"],
        source_label=state["source_label"],
        status=STATUS_PAUSED,
        paused=True,
        stale_reason=state.get("stale_reason"),
        blocking_reports=bool(state.get("blocking_reports")),
        checkpoint=state.get("checkpoint", {}),
    )


def resume_source(conn: sqlite3.Connection, profile_id: str, key: str) -> dict[str, Any]:
    state = get_source_state(conn, profile_id, key)
    if state is None:
        raise AppError("Freshness source was not found", code="not_found")
    status = STATUS_PARTIALLY_STALE if state.get("stale_reason") else STATUS_FRESH
    return upsert_source_state(
        conn,
        profile_id=profile_id,
        source_key=key,
        source_type=state["source_type"],
        source_label=state["source_label"],
        status=status,
        paused=False,
        stale_reason=state.get("stale_reason"),
        blocking_reports=bool(state.get("blocking_reports")),
        checkpoint=state.get("checkpoint", {}),
    )


def _pending_job_for_source(
    conn: sqlite3.Connection,
    profile_id: str,
    key: str,
    job_type: str,
) -> sqlite3.Row | None:
    return conn.execute(
        """
        SELECT *
        FROM freshness_jobs
        WHERE profile_id = ?
          AND source_key = ?
          AND job_type = ?
          AND status IN ('queued', 'running', 'rate_limited')
        ORDER BY created_at ASC
        LIMIT 1
        """,
        (profile_id, key, job_type),
    ).fetchone()


def _set_cancelled(
    conn: sqlite3.Connection,
    job: Mapping[str, Any],
) -> dict[str, Any]:
    now = now_iso()
    conn.execute(
        """
        UPDATE freshness_jobs
        SET cancel_requested = 1, status = ?, phase = ?, finished_at = ?,
            updated_at = ?
        WHERE id = ?
        """,
        (
            JOB_CANCELLED,
            job.get("phase") or PHASE_ERROR,
            now,
            now,
            job["id"],
        ),
    )
    state = get_source_state(conn, job["profile_id"], job["source_key"])
    upsert_source_state(
        conn,
        profile_id=job["profile_id"],
        source_key=job["source_key"],
        source_type=job["source_type"],
        source_label=job["source_label"],
        status=STATUS_PARTIALLY_STALE,
        stale_reason="cancelled",
        blocking_reports=True,
        last_phase=job.get("phase"),
        checkpoint=(state or {}).get("checkpoint", {}),
    )
    return _load_job(conn, job["id"])


def enqueue_job(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    job_type: str,
    source_key: str,
    source_type: str,
    source_label: str,
    payload: Mapping[str, Any] | None = None,
    priority: int = 100,
    run_after: str | None = None,
    single_flight: bool = True,
) -> dict[str, Any]:
    if job_type not in JOB_TYPES:
        raise AppError(
            f"Unsupported freshness job type '{job_type}'",
            code="validation",
            retryable=False,
        )
    state = get_source_state(conn, profile_id, source_key)
    if state and state.get("paused"):
        return upsert_source_state(
            conn,
            profile_id=profile_id,
            source_key=source_key,
            source_type=source_type,
            source_label=source_label,
            status=STATUS_PAUSED,
            paused=True,
            stale_reason="paused",
            checkpoint=state.get("checkpoint", {}),
        )
    if single_flight:
        existing = _pending_job_for_source(conn, profile_id, source_key, job_type)
        if existing is not None:
            return _row_payload(existing)
    job_id = str(uuid.uuid4())
    now = now_iso()
    conn.execute(
        """
        INSERT INTO freshness_jobs(
            id, profile_id, job_type, source_key, source_type, source_label,
            status, phase, priority, payload_json, progress_json, result_json,
            error_json, attempts, cancel_requested, run_after, cooldown_until,
            created_at, started_at, finished_at, updated_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            job_id,
            profile_id,
            job_type,
            source_key,
            source_type,
            source_label,
            JOB_QUEUED,
            PHASE_DISCOVERY,
            int(priority),
            _json_dump(redact_freshness_payload(payload or {})),
            "{}",
            "{}",
            "{}",
            0,
            0,
            run_after,
            None,
            now,
            None,
            None,
            now,
        ),
    )
    upsert_source_state(
        conn,
        profile_id=profile_id,
        source_key=source_key,
        source_type=source_type,
        source_label=source_label,
        status=STATUS_QUEUED,
        stale_reason="queued",
        progress={"phase": PHASE_DISCOVERY},
        checkpoint=(state or {}).get("checkpoint", {}),
    )
    row = conn.execute("SELECT * FROM freshness_jobs WHERE id = ?", (job_id,)).fetchone()
    return _row_payload(row)


def cancel_job(conn: sqlite3.Connection, job_id: str) -> dict[str, Any]:
    row = conn.execute("SELECT * FROM freshness_jobs WHERE id = ?", (job_id,)).fetchone()
    if row is None:
        raise AppError("Freshness job was not found", code="not_found")
    if row["status"] in {JOB_DONE, JOB_ERROR, JOB_CANCELLED}:
        return _row_payload(row)
    job = _row_payload(row)
    if job["status"] == JOB_RUNNING:
        now = now_iso()
        conn.execute(
            """
            UPDATE freshness_jobs
            SET cancel_requested = 1, updated_at = ?
            WHERE id = ?
            """,
            (now, job_id),
        )
        update_job_progress(
            conn,
            job,
            {"phase": job.get("phase") or PHASE_BACKEND_FETCH, "cancellation_requested": True},
        )
        return _load_job(conn, job_id)
    return _set_cancelled(conn, job)


def _load_job(conn: sqlite3.Connection, job_id: str) -> dict[str, Any]:
    row = conn.execute("SELECT * FROM freshness_jobs WHERE id = ?", (job_id,)).fetchone()
    if row is None:
        raise AppError("Freshness job was not found", code="not_found")
    return _row_payload(row)


def recover_interrupted_jobs(
    conn: sqlite3.Connection,
    *,
    profile_id: str | None = None,
) -> list[dict[str, Any]]:
    params: list[Any] = [JOB_RUNNING]
    profile_filter = ""
    if profile_id:
        profile_filter = "AND profile_id = ?"
        params.append(profile_id)
    rows = conn.execute(
        f"""
        SELECT *
        FROM freshness_jobs
        WHERE status = ?
          {profile_filter}
        ORDER BY priority ASC, created_at ASC, id ASC
        """,
        params,
    ).fetchall()
    if not rows:
        return []
    now = now_iso()
    recovered: list[dict[str, Any]] = []
    for row in rows:
        job = _row_payload(row)
        state = get_source_state(conn, job["profile_id"], job["source_key"])
        conn.execute(
            """
            UPDATE freshness_jobs
            SET status = ?, phase = ?, cancel_requested = 0,
                run_after = COALESCE(run_after, ?),
                error_json = ?, updated_at = ?
            WHERE id = ?
            """,
            (
                JOB_QUEUED,
                PHASE_DISCOVERY,
                now,
                _json_dump(
                    {
                        "code": "worker_interrupted",
                        "message": "Previous daemon worker stopped before this job finished; the job was requeued.",
                        "retryable": True,
                    }
                ),
                now,
                job["id"],
            ),
        )
        upsert_source_state(
            conn,
            profile_id=job["profile_id"],
            source_key=job["source_key"],
            source_type=job["source_type"],
            source_label=job["source_label"],
            status=STATUS_BLOCKING_REPORTS,
            stale_reason="worker_interrupted",
            blocking_reports=True,
            last_phase=job.get("phase"),
            progress={"phase": PHASE_DISCOVERY, "recovered": True},
            checkpoint=(state or {}).get("checkpoint", {}),
        )
        updated = conn.execute("SELECT * FROM freshness_jobs WHERE id = ?", (job["id"],)).fetchone()
        recovered.append(_row_payload(updated))
    return recovered


def _next_due_job(conn: sqlite3.Connection, profile_id: str | None = None) -> dict[str, Any] | None:
    now = now_iso()
    params: list[Any] = [JOB_QUEUED, JOB_RATE_LIMITED, now, now]
    profile_filter = ""
    if profile_id:
        profile_filter = "AND profile_id = ?"
        params.append(profile_id)
    row = conn.execute(
        f"""
        SELECT *
        FROM freshness_jobs
        WHERE status IN (?, ?)
          AND (run_after IS NULL OR run_after <= ?)
          AND (cooldown_until IS NULL OR cooldown_until <= ?)
          AND NOT EXISTS (
            SELECT 1
            FROM freshness_source_states
            WHERE freshness_source_states.profile_id = freshness_jobs.profile_id
              AND freshness_source_states.source_key = freshness_jobs.source_key
              AND freshness_source_states.paused = 1
          )
          {profile_filter}
        ORDER BY priority ASC, created_at ASC, id ASC
        LIMIT 1
        """,
        params,
    ).fetchone()
    return _row_payload(row) if row else None


def update_job_progress(
    conn: sqlite3.Connection,
    job: Mapping[str, Any],
    progress: Mapping[str, Any],
) -> None:
    redacted = redact_freshness_payload(progress)
    phase = str(redacted.get("phase") or job.get("phase") or PHASE_BACKEND_FETCH)
    now = now_iso()
    conn.execute(
        """
        UPDATE freshness_jobs
        SET phase = ?, progress_json = ?, updated_at = ?
        WHERE id = ?
        """,
        (phase, _json_dump(redacted), now, job["id"]),
    )
    state = get_source_state(conn, job["profile_id"], job["source_key"])
    upsert_source_state(
        conn,
        profile_id=job["profile_id"],
        source_key=job["source_key"],
        source_type=job["source_type"],
        source_label=job["source_label"],
        status=STATUS_SYNCING,
        stale_reason="syncing",
        last_phase=phase,
        progress=redacted,
        checkpoint=(state or {}).get("checkpoint", {}),
    )


def _check_cancelled(conn: sqlite3.Connection, job_id: str) -> None:
    row = conn.execute(
        "SELECT cancel_requested, status FROM freshness_jobs WHERE id = ?",
        (job_id,),
    ).fetchone()
    if row is None:
        raise AppError("Freshness job disappeared", code="state_not_ready")
    if row["cancel_requested"] or row["status"] == JOB_CANCELLED:
        raise AppError(
            "Freshness job was cancelled",
            code="cancelled",
            retryable=False,
        )


def _mark_running(conn: sqlite3.Connection, job: Mapping[str, Any]) -> dict[str, Any]:
    now = now_iso()
    conn.execute(
        """
        UPDATE freshness_jobs
        SET status = ?, phase = ?, attempts = attempts + 1,
            started_at = COALESCE(started_at, ?), updated_at = ?
        WHERE id = ? AND status IN ('queued', 'rate_limited')
        """,
        (JOB_RUNNING, PHASE_DISCOVERY, now, now, job["id"]),
    )
    update_job_progress(conn, {**job, "status": JOB_RUNNING}, {"phase": PHASE_DISCOVERY})
    return _load_job(conn, job["id"])


def _retry_after_from_error(exc: AppError, job: Mapping[str, Any]) -> tuple[str | None, str | None]:
    details = exc.details if isinstance(exc.details, dict) else {}
    retry_after = details.get("retry_after_seconds") or details.get("retry_after")
    if retry_after is not None:
        try:
            return _iso_from_now_plus(int(retry_after)), "retry-after"
        except (TypeError, ValueError):
            pass
    text = str(exc).lower()
    code = str(exc.code or "").lower()
    if "429" in text or "rate" in code:
        attempts = int(job.get("attempts") or 0) + 1
        base = min(MAX_BACKOFF_SECONDS, DEFAULT_BACKOFF_SECONDS * (2 ** max(0, attempts - 1)))
        jitter = _stable_jitter_seconds(f"{job['id']}:{attempts}", min(base // 4, 300))
        return _iso_from_now_plus(base + jitter), "exponential-backoff"
    return None, None


def _mark_success(
    conn: sqlite3.Connection,
    job: Mapping[str, Any],
    result: Mapping[str, Any],
) -> dict[str, Any]:
    redacted = redact_freshness_payload(result)
    checkpoint = result.get("freshness_checkpoint")
    if not isinstance(checkpoint, dict):
        state = get_source_state(conn, job["profile_id"], job["source_key"])
        checkpoint = (state or {}).get("checkpoint", {})
    now = now_iso()
    conn.execute(
        """
        UPDATE freshness_jobs
        SET status = ?, phase = ?, result_json = ?, finished_at = ?,
            updated_at = ?
        WHERE id = ?
        """,
        (JOB_DONE, PHASE_DONE, _json_dump(redacted), now, now, job["id"]),
    )
    status = STATUS_PARTIALLY_STALE if redacted.get("partial_success") else STATUS_FRESH
    upsert_source_state(
        conn,
        profile_id=job["profile_id"],
        source_key=job["source_key"],
        source_type=job["source_type"],
        source_label=job["source_label"],
        status=status,
        stale_reason="partial_success" if redacted.get("partial_success") else None,
        blocking_reports=bool(redacted.get("blocking_reports")),
        retry_count=0,
        last_success_at=now,
        last_error_at=None,
        last_error_code=None,
        last_error_message=None,
        last_phase=PHASE_DONE,
        progress={"phase": PHASE_DONE},
        checkpoint=checkpoint,
    )
    return _load_job(conn, job["id"])


def _mark_error(
    conn: sqlite3.Connection,
    job: Mapping[str, Any],
    exc: AppError,
) -> dict[str, Any]:
    cooldown_until, cooldown_reason = _retry_after_from_error(exc, job)
    status = JOB_RATE_LIMITED if cooldown_until else JOB_ERROR
    source_status = STATUS_RATE_LIMITED if cooldown_until else STATUS_FAILED
    source_name = job.get("source_label") or job.get("source_key") or "source"
    # Log only the source label + error code, never str(exc): the raw message
    # can carry operational data (e.g. backend URLs / inline credentials). The
    # message is still persisted for the UI snapshot, but URLs embedded in it are
    # scrubbed at the render boundary (daemon_freshness._freshness_snapshot_for_ui);
    # redact_freshness_payload below only scrubs secret *keys*, not URLs inside a
    # free-text value. The RAM log ring has no render step, so the message must
    # never reach it.
    error_code = exc.code or "freshness_job_failed"
    # The exception TYPE (set by run_job for swallowed non-AppErrors) is a code
    # identifier, never runtime data, so it is safe for the RAM log ring and
    # makes an otherwise-opaque "freshness_job_failed" diagnosable.
    error_class = exc.details.get("error_class") if isinstance(exc.details, dict) else None
    if cooldown_until:
        _LOGGER.warning("Freshness %s rate-limited (%s)", source_name, error_code)
    elif error_class:
        _LOGGER.error("Freshness %s failed (%s; %s)", source_name, error_code, error_class)
    else:
        _LOGGER.error("Freshness %s failed (%s)", source_name, error_code)
    now = now_iso()
    error_payload = redact_freshness_payload(
        {
            "code": exc.code or "freshness_job_failed",
            "message": str(exc),
            "hint": exc.hint,
            "retryable": exc.retryable,
            "details": exc.details,
            "rate_limited_until": cooldown_until,
        }
    )
    conn.execute(
        """
        UPDATE freshness_jobs
        SET status = ?, phase = ?, error_json = ?, cooldown_until = ?,
            run_after = ?, finished_at = CASE WHEN ? IS NULL THEN ? ELSE finished_at END,
            updated_at = ?
        WHERE id = ?
        """,
        (
            status,
            PHASE_ERROR,
            _json_dump(error_payload),
            cooldown_until,
            cooldown_until,
            cooldown_until,
            now,
            now,
            job["id"],
        ),
    )
    state = get_source_state(conn, job["profile_id"], job["source_key"])
    upsert_source_state(
        conn,
        profile_id=job["profile_id"],
        source_key=job["source_key"],
        source_type=job["source_type"],
        source_label=job["source_label"],
        status=source_status,
        stale_reason=exc.code or "freshness_job_failed",
        blocking_reports=True,
        rate_limited_until=cooldown_until,
        cooldown_reason=cooldown_reason,
        retry_count=int(job.get("attempts") or 0),
        last_error_at=now,
        last_error_code=exc.code or "freshness_job_failed",
        last_error_message=str(exc),
        last_phase=PHASE_ERROR,
        progress={"phase": PHASE_ERROR},
        checkpoint=(state or {}).get("checkpoint", {}),
    )
    return _load_job(conn, job["id"])


def run_job(
    conn: sqlite3.Connection,
    job_id: str,
    handlers: Mapping[str, JobHandler],
    *,
    progress_observer: ProgressObserver | None = None,
) -> dict[str, Any]:
    job = _load_job(conn, job_id)
    if job["status"] in {JOB_DONE, JOB_ERROR, JOB_CANCELLED}:
        return job
    handler = handlers.get(job["job_type"])
    if handler is None:
        raise AppError(
            f"No freshness handler is configured for '{job['job_type']}'",
            code="config_error",
        )
    job = _mark_running(conn, job)
    conn.commit()

    def progress(payload: Mapping[str, Any]) -> None:
        _check_cancelled(conn, job["id"])
        update_job_progress(conn, job, payload)
        if progress_observer is not None:
            progress_observer(
                {
                    "job_id": job["id"],
                    "job_type": job["job_type"],
                    "source_key": job["source_key"],
                    "source_type": job["source_type"],
                    "source_label": job["source_label"],
                    **redact_freshness_payload(payload),
                }
            )
        conn.commit()

    def check_cancelled() -> None:
        _check_cancelled(conn, job["id"])

    try:
        check_cancelled()
        result = handler(conn, job, progress, check_cancelled)
        check_cancelled()
    except AppError as exc:
        if exc.code == "cancelled":
            updated = _set_cancelled(conn, job)
            conn.commit()
            return updated
        updated = _mark_error(conn, job, exc)
        conn.commit()
        return updated
    except Exception as exc:
        # Carry the exception's fully-qualified TYPE (never str(exc), which can
        # hold backend URLs/credentials) so a swallowed non-AppError failure —
        # e.g. an RP2/Liquid balance error during the journal refresh — is
        # diagnosable from the log instead of a bare "freshness_job_failed".
        updated = _mark_error(
            conn,
            job,
            AppError(
                str(exc) or exc.__class__.__name__,
                code="freshness_job_failed",
                retryable=True,
                details={
                    "error_class": f"{exc.__class__.__module__}.{exc.__class__.__qualname__}"
                },
            ),
        )
        conn.commit()
        return updated
    updated = _mark_success(conn, job, result or {})
    conn.commit()
    return updated


def run_due_jobs(
    conn: sqlite3.Connection,
    handlers: Mapping[str, JobHandler],
    *,
    profile_id: str | None = None,
    limit: int = 10,
    progress_observer: ProgressObserver | None = None,
) -> list[dict[str, Any]]:
    results = []
    for _ in range(max(0, int(limit))):
        job = _next_due_job(conn, profile_id=profile_id)
        if job is None:
            break
        results.append(
            run_job(
                conn,
                job["id"],
                handlers,
                progress_observer=progress_observer,
            )
        )
    return results


def list_jobs(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    active_only: bool = False,
) -> list[dict[str, Any]]:
    status_filter = "AND status IN ('queued', 'running', 'rate_limited')" if active_only else ""
    rows = conn.execute(
        f"""
        SELECT *
        FROM freshness_jobs
        WHERE profile_id = ?
          {status_filter}
        ORDER BY
          CASE status
            WHEN 'running' THEN 0
            WHEN 'queued' THEN 1
            WHEN 'rate_limited' THEN 2
            ELSE 3
          END,
          priority ASC,
          created_at DESC,
          id ASC
        """,
        (profile_id,),
    ).fetchall()
    return [_row_payload(row) for row in rows]


def build_snapshot(conn: sqlite3.Connection, profile_id: str) -> dict[str, Any]:
    sources = list_source_states(conn, profile_id)
    active_jobs = list_jobs(conn, profile_id, active_only=True)
    blocking = [source for source in sources if source.get("blocking_reports")]
    rate_limited = [
        source
        for source in sources
        if source.get("status") == STATUS_RATE_LIMITED or source.get("rate_limited_until")
    ]
    return {
        "policy": get_policy(conn, profile_id).to_payload(),
        "sources": sources,
        "jobs": active_jobs,
        "summary": {
            "sources": len(sources),
            "active_jobs": len(active_jobs),
            "blocking_reports": len(blocking),
            "rate_limited": len(rate_limited),
        },
    }


SECRET_KEYS = {
    "token",
    "auth_header",
    "authorization",
    "password",
    "secret",
    "api_key",
    "cookie",
    "descriptor",
    "xpub",
    "zpub",
    "ypub",
    "url",
    "backend_url",
}


def redact_freshness_payload(value: Any) -> Any:
    if isinstance(value, Mapping):
        redacted: dict[str, Any] = {}
        for key, item in value.items():
            lowered = str(key).lower()
            if lowered in {"backend_url", "url"}:
                redacted[f"has_{lowered}"] = bool(item)
                continue
            if lowered in SECRET_KEYS or any(part in lowered for part in ("token", "password", "secret")):
                redacted[key] = REDACTED_TEXT if item else None
                continue
            redacted[key] = redact_freshness_payload(item)
        return redacted
    if isinstance(value, list):
        return [redact_freshness_payload(item) for item in value]
    return value


__all__ = [
    "FreshnessPolicy",
    "JOB_BTCPAY_PROVENANCE",
    "JOB_BTCPAY_WALLET",
    "JOB_JOURNAL_REFRESH",
    "JOB_MARKET_RATES",
    "JOB_ONCHAIN_WALLET",
    "PHASE_BACKEND_FETCH",
    "PHASE_DECODE_ENRICH",
    "PHASE_DISCOVERY",
    "PHASE_DONE",
    "PHASE_ERROR",
    "PHASE_IMPORT",
    "PHASE_JOURNAL_REFRESH",
    "PHASE_RATE_COVERAGE",
    "SOURCE_BTCPAY_PROVENANCE",
    "SOURCE_BTCPAY_WALLET",
    "SOURCE_JOURNALS",
    "SOURCE_ONCHAIN",
    "SOURCE_RATES",
    "build_snapshot",
    "cancel_job",
    "default_policy",
    "enqueue_job",
    "get_policy",
    "get_source_state",
    "journal_source_key",
    "list_jobs",
    "list_source_states",
    "pause_source",
    "rate_source_key",
    "redact_freshness_payload",
    "recover_interrupted_jobs",
    "resume_source",
    "run_due_jobs",
    "run_job",
    "set_policy",
    "source_key",
    "update_job_progress",
    "upsert_source_state",
]
