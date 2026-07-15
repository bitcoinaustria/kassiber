"""Bounded saved/filed-report snapshots and custody amendment history.

The exported report remains outside this module.  Kassiber stores only an
application-computed content hash plus small, closed accounting summaries.  A
guided custody bridge can therefore disclose which already-saved periods may
need amendment without retaining report documents or pretending stale journal
totals are the recalculated result.  Normal Kassiber exports are recorded as
``saved``; only an explicit user action may call a report ``filed``.
"""

from __future__ import annotations

from decimal import Decimal, InvalidOperation
import hashlib
import json
from pathlib import Path
import re
import sqlite3
import uuid
from typing import Any, Mapping, Sequence

from ..errors import AppError
from ..time_utils import now_iso
from .austrian import tax_year_in_vienna


_TOKEN = re.compile(r"[a-z0-9][a-z0-9_.-]{0,63}")
_SHA256 = re.compile(r"[0-9a-f]{64}")
_AUTHORED_SOURCES = frozenset({"user", "cli", "gui", "ai_tool"})
_REPORT_STATES = frozenset({"saved", "filed"})
_GAIN_KEYS = frozenset(
    {
        "fiat_currency",
        "proceeds_exact",
        "cost_basis_exact",
        "gain_loss_exact",
        "status",
    }
)
_GAIN_STATUSES = frozenset({"final", "pending_journal_rebuild"})
_AMENDMENT_STATUSES = frozenset(
    {"no_change", "saved_report_changed", "review_required"}
)
_MAX_CLASSIFICATIONS = 32
_MAX_SCOPE_WALLETS = 256
_MAX_NOTES = 1_000
AMENDMENT_WARNING = (
    "Reviewed custody evidence overlaps this saved/filed report period. "
    "Rebuild journals and review whether an amended filing is required."
)


def _error(message: str, *, field: str | None = None) -> AppError:
    details = {"field": field} if field else None
    return AppError(
        message,
        code="filed_report_snapshot_validation",
        details=details,
        retryable=False,
    )


def _token(value: Any, field: str) -> str:
    text = str(value or "").strip().lower()
    if not _TOKEN.fullmatch(text):
        raise _error(f"{field} must be a bounded lowercase token", field=field)
    return text


def _year(value: Any, field: str) -> int:
    if isinstance(value, bool):
        raise _error(f"{field} must be a calendar year", field=field)
    try:
        year = int(value)
    except (TypeError, ValueError) as exc:
        raise _error(f"{field} must be a calendar year", field=field) from exc
    if not 1900 <= year <= 9999:
        raise _error(f"{field} must be between 1900 and 9999", field=field)
    return year


def _classification_summary(value: Mapping[str, Any] | None) -> dict[str, Any]:
    if value is None:
        return {}
    if not isinstance(value, Mapping) or len(value) > _MAX_CLASSIFICATIONS:
        raise _error(
            f"classification_summary must contain at most {_MAX_CLASSIFICATIONS} categories",
            field="classification_summary",
        )
    output: dict[str, Any] = {}
    for raw_category, raw_summary in value.items():
        category = _token(raw_category, "classification_summary.category")
        if not isinstance(raw_summary, Mapping):
            raise _error(
                "each classification summary must contain count and amount_msat",
                field="classification_summary",
            )
        unknown = set(raw_summary) - {"count", "amount_msat"}
        if unknown:
            raise _error(
                "classification summary contains unsupported fields",
                field="classification_summary",
            )
        count = raw_summary.get("count", 0)
        amount = raw_summary.get("amount_msat", 0)
        if (
            isinstance(count, bool)
            or isinstance(amount, bool)
            or not isinstance(count, int)
            or not isinstance(amount, int)
            or count < 0
            or amount < 0
        ):
            raise _error(
                "classification count and amount_msat must be non-negative integers",
                field="classification_summary",
            )
        output[category] = {"count": count, "amount_msat": amount}
    return dict(sorted(output.items()))


def _decimal_text(value: Any, field: str) -> str:
    text = str(value or "").strip()
    try:
        number = Decimal(text)
    except (InvalidOperation, ValueError) as exc:
        raise _error(f"{field} must be an exact decimal string", field=field) from exc
    if not number.is_finite():
        raise _error(f"{field} must be finite", field=field)
    return text


def _gain_summary(value: Mapping[str, Any] | None) -> dict[str, str]:
    if value is None:
        return {}
    if not isinstance(value, Mapping) or set(value) - _GAIN_KEYS:
        raise _error(
            "gain_summary contains unsupported fields",
            field="gain_summary",
        )
    output: dict[str, str] = {}
    if "fiat_currency" in value:
        currency = str(value["fiat_currency"] or "").strip().upper()
        if not re.fullmatch(r"[A-Z]{3,12}", currency):
            raise _error(
                "gain_summary.fiat_currency is invalid",
                field="gain_summary.fiat_currency",
            )
        output["fiat_currency"] = currency
    for field in ("proceeds_exact", "cost_basis_exact", "gain_loss_exact"):
        if field in value:
            output[field] = _decimal_text(value[field], f"gain_summary.{field}")
    if "status" in value:
        status = str(value["status"] or "").strip().lower()
        if status not in _GAIN_STATUSES:
            raise _error("gain_summary.status is invalid", field="gain_summary.status")
        output["status"] = status
    return output


def _json(value: Mapping[str, Any]) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"))


def _json_object(value: Any) -> dict[str, Any]:
    try:
        parsed = json.loads(str(value or "{}"))
    except (TypeError, ValueError, json.JSONDecodeError):
        return {}
    return parsed if isinstance(parsed, dict) else {}


def _report_scope(value: Mapping[str, Any] | None) -> dict[str, Any]:
    """Validate the small, replayable journal scope attached to a snapshot."""

    if value is None:
        return {}
    if not isinstance(value, Mapping) or set(value) - {
        "wallet_ids",
        "occurred_at_start",
        "occurred_at_end",
    }:
        raise _error("report_scope contains unsupported fields", field="report_scope")
    output: dict[str, Any] = {}
    if "wallet_ids" in value:
        raw_wallets = value["wallet_ids"]
        if (
            not isinstance(raw_wallets, Sequence)
            or isinstance(raw_wallets, (str, bytes))
            or len(raw_wallets) > _MAX_SCOPE_WALLETS
        ):
            raise _error(
                f"report_scope.wallet_ids must contain at most {_MAX_SCOPE_WALLETS} ids",
                field="report_scope.wallet_ids",
            )
        wallets = sorted({str(item or "").strip() for item in raw_wallets})
        if any(not wallet or len(wallet) > 128 for wallet in wallets):
            raise _error("report_scope.wallet_ids is invalid", field="report_scope.wallet_ids")
        if wallets:
            output["wallet_ids"] = wallets
    for field in ("occurred_at_start", "occurred_at_end"):
        if field not in value or value[field] in (None, ""):
            continue
        timestamp = str(value[field]).strip()
        if len(timestamp) > 64 or not re.fullmatch(
            r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z", timestamp
        ):
            raise _error(f"report_scope.{field} must be RFC3339 UTC", field=f"report_scope.{field}")
        output[field] = timestamp
    start = output.get("occurred_at_start")
    end = output.get("occurred_at_end")
    if start and end and end < start:
        raise _error(
            "report_scope.occurred_at_end must not precede occurred_at_start",
            field="report_scope.occurred_at_end",
        )
    return output


def _exact_decimal(row: Mapping[str, Any], exact_field: str, fallback_field: str) -> Decimal:
    raw = row[exact_field]
    if raw in (None, ""):
        raw = row[fallback_field]
    return Decimal(str(raw or 0))


def _decimal_text_exact(value: Decimal) -> str:
    return format(value, "f")


def _entry_year(occurred_at: Any, *, tax_country: str) -> int | None:
    text = str(occurred_at or "")
    if tax_country == "at":
        try:
            return int(tax_year_in_vienna(text))
        except (TypeError, ValueError):
            return None
    if len(text) >= 4 and text[:4].isdigit():
        return int(text[:4])
    return None


def report_period_years(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    period_start_year: int | None = None,
    period_end_year: int | None = None,
    fallback_at: str | None = None,
) -> tuple[int, int]:
    """Return a closed calendar-year range for one exported artifact."""

    if period_start_year is not None:
        start = _year(period_start_year, "period_start_year")
        end = _year(
            period_end_year if period_end_year is not None else start,
            "period_end_year",
        )
        if end < start:
            raise _error(
                "period_end_year must not precede period_start_year",
                field="period_end_year",
            )
        return start, end
    profile = conn.execute(
        "SELECT tax_country FROM profiles WHERE id = ?", (profile_id,)
    ).fetchone()
    if profile is None:
        raise AppError("profile was not found", code="not_found")
    tax_country = str(profile["tax_country"] or "").strip().lower()
    years = [
        year
        for row in conn.execute(
            "SELECT occurred_at FROM journal_entries WHERE profile_id = ?",
            (profile_id,),
        ).fetchall()
        if (year := _entry_year(row["occurred_at"], tax_country=tax_country))
        is not None
    ]
    if years:
        return min(years), max(years)
    fallback = str(fallback_at or now_iso())
    if len(fallback) < 4 or not fallback[:4].isdigit():
        raise _error("fallback_at must contain a calendar year", field="fallback_at")
    year = _year(fallback[:4], "fallback_at")
    return year, year


def current_report_summaries(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    period_start_year: int,
    period_end_year: int,
    report_scope: Mapping[str, Any] | None = None,
) -> dict[str, dict[str, Any]]:
    """Compute bounded exact summaries from the current finalized journals."""

    start = _year(period_start_year, "period_start_year")
    end = _year(period_end_year, "period_end_year")
    if end < start:
        raise _error(
            "period_end_year must not precede period_start_year",
            field="period_end_year",
        )
    profile = conn.execute(
        "SELECT fiat_currency, tax_country FROM profiles WHERE id = ?",
        (profile_id,),
    ).fetchone()
    if profile is None:
        raise AppError("profile was not found", code="not_found")
    tax_country = str(profile["tax_country"] or "").strip().lower()
    scope = _report_scope(report_scope)
    wallet_ids = frozenset(scope.get("wallet_ids", ()))
    occurred_at_start = scope.get("occurred_at_start")
    occurred_at_end = scope.get("occurred_at_end")
    rows = conn.execute(
        """
        SELECT je.occurred_at, je.wallet_id, je.entry_type, je.quantity,
               je.proceeds, je.proceeds_exact,
               je.cost_basis, je.cost_basis_exact,
               je.gain_loss, je.gain_loss_exact,
               je.at_category,
               COALESCE(t.taxability_override, 1) AS taxability
        FROM journal_entries je
        LEFT JOIN transactions t ON t.id = je.transaction_id
        WHERE je.profile_id = ?
        ORDER BY je.occurred_at, je.created_at, je.id
        """,
        (profile_id,),
    ).fetchall()
    classifications: dict[str, dict[str, int]] = {}
    proceeds = Decimal("0")
    cost_basis = Decimal("0")
    gain_loss = Decimal("0")
    for row in rows:
        occurred_at = str(row["occurred_at"] or "")
        if wallet_ids and str(row["wallet_id"]) not in wallet_ids:
            continue
        if occurred_at_start and occurred_at < occurred_at_start:
            continue
        if occurred_at_end and occurred_at > occurred_at_end:
            continue
        year = _entry_year(row["occurred_at"], tax_country=tax_country)
        if year is None or not start <= year <= end:
            continue
        entry_type = _token(row["entry_type"], "journal_entry.entry_type")
        bucket = classifications.setdefault(
            entry_type, {"count": 0, "amount_msat": 0}
        )
        bucket["count"] += 1
        bucket["amount_msat"] += abs(int(row["quantity"] or 0))
        if (
            entry_type not in {"disposal", "income"}
            or int(row["taxability"] or 0) == 0
            or str(row["at_category"] or "") == "neu_swap"
        ):
            continue
        proceeds += _exact_decimal(row, "proceeds_exact", "proceeds")
        cost_basis += _exact_decimal(row, "cost_basis_exact", "cost_basis")
        gain_loss += _exact_decimal(row, "gain_loss_exact", "gain_loss")
    return {
        "classification_summary": _classification_summary(classifications),
        "gain_summary": _gain_summary(
            {
                "fiat_currency": str(profile["fiat_currency"] or "").upper(),
                "proceeds_exact": _decimal_text_exact(proceeds),
                "cost_basis_exact": _decimal_text_exact(cost_basis),
                "gain_loss_exact": _decimal_text_exact(gain_loss),
                "status": "final",
            }
        ),
    }


def _file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def artifact_content_sha256(paths: Sequence[str | Path]) -> str:
    """Hash one artifact, or a deterministic manifest for a file bundle."""

    materialized = tuple(Path(path).expanduser() for path in paths)
    if not materialized:
        raise _error("at least one exported artifact is required", field="paths")
    for path in materialized:
        if not path.is_file():
            raise _error("exported artifact is not a regular file", field="paths")
    if len(materialized) == 1:
        return _file_sha256(materialized[0])
    manifest = [
        {
            "name": path.name,
            "bytes": path.stat().st_size,
            "sha256": _file_sha256(path),
        }
        for path in sorted(materialized, key=lambda item: item.name)
    ]
    encoded = json.dumps(
        manifest, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def register_saved_report_export(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    report_kind: str,
    artifact_paths: Sequence[str | Path],
    period_start_year: int | None = None,
    period_end_year: int | None = None,
    report_scope: Mapping[str, Any] | None = None,
    created_at: str | None = None,
    notes: str | None = None,
) -> dict[str, Any]:
    """Register a completed Kassiber export as an append-only saved snapshot."""

    timestamp = created_at or now_iso()
    kind = _token(report_kind, "report_kind")
    start, end = report_period_years(
        conn,
        profile_id,
        period_start_year=period_start_year,
        period_end_year=period_end_year,
        fallback_at=timestamp,
    )
    digest = artifact_content_sha256(artifact_paths)
    bounded_scope = _report_scope(report_scope)
    summaries = current_report_summaries(
        conn,
        profile_id,
        period_start_year=start,
        period_end_year=end,
        report_scope=bounded_scope,
    )
    # Each completed export is a distinct user action, even when its bytes match a
    # previous export. A content-derived identity would collapse audit events and
    # can collide across replicas whose local timestamps differ.
    snapshot_id = str(uuid.uuid4())
    create_filed_report_snapshot(
        conn,
        workspace_id=workspace_id,
        profile_id=profile_id,
        report_kind=kind,
        report_state="saved",
        period_start_year=start,
        period_end_year=end,
        content_sha256=digest,
        classification_summary=summaries["classification_summary"],
        gain_summary=summaries["gain_summary"],
        report_scope=bounded_scope,
        authored_source="user",
        notes=notes or "Automatically registered after a completed Kassiber export.",
        snapshot_id=snapshot_id,
        created_at=timestamp,
    )
    conn.commit()
    return get_filed_report_snapshot(conn, snapshot_id, profile_id=profile_id)


def create_filed_report_snapshot(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    report_kind: str,
    report_state: str,
    period_start_year: int,
    period_end_year: int,
    content_sha256: str,
    classification_summary: Mapping[str, Any] | None = None,
    gain_summary: Mapping[str, Any] | None = None,
    report_scope: Mapping[str, Any] | None = None,
    authored_source: str = "user",
    notes: str | None = None,
    snapshot_id: str | None = None,
    created_at: str | None = None,
) -> dict[str, Any]:
    """Append one user-authored saved/filed report marker."""

    kind = _token(report_kind, "report_kind")
    state = str(report_state or "").strip().lower()
    if state not in _REPORT_STATES:
        raise _error("report_state must be saved or filed", field="report_state")
    start_year = _year(period_start_year, "period_start_year")
    end_year = _year(period_end_year, "period_end_year")
    if end_year < start_year:
        raise _error(
            "period_end_year must not precede period_start_year",
            field="period_end_year",
        )
    digest = str(content_sha256 or "").strip().lower()
    if not _SHA256.fullmatch(digest):
        raise _error("content_sha256 must be a lowercase SHA-256 digest", field="content_sha256")
    source = str(authored_source or "").strip().lower()
    if source not in _AUTHORED_SOURCES:
        raise _error("authored_source is invalid", field="authored_source")
    note_text = None if notes in (None, "") else str(notes).strip()
    if note_text is not None and len(note_text) > _MAX_NOTES:
        raise _error("notes is too long", field="notes")
    scope = conn.execute(
        "SELECT workspace_id FROM profiles WHERE id = ?", (profile_id,)
    ).fetchone()
    if not scope or str(scope["workspace_id"]) != workspace_id:
        raise AppError(
            "filed report snapshot targets another book",
            code="scope_mismatch",
            retryable=False,
        )
    row_id = snapshot_id or str(uuid.uuid4())
    timestamp = created_at or now_iso()
    classifications = _classification_summary(classification_summary)
    gains = _gain_summary(gain_summary)
    bounded_scope = _report_scope(report_scope)
    conn.execute(
        """
        INSERT INTO filed_report_snapshots(
            id, workspace_id, profile_id, report_kind, report_state,
            period_start_year, period_end_year, content_sha256,
            classification_summary_json, gain_summary_json, report_scope_json, authored_source,
            notes, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            row_id,
            workspace_id,
            profile_id,
            kind,
            state,
            start_year,
            end_year,
            digest,
            _json(classifications),
            _json(gains),
            _json(bounded_scope),
            source,
            note_text,
            timestamp,
        ),
    )
    return get_filed_report_snapshot(conn, row_id, profile_id=profile_id)


def get_filed_report_snapshot(
    conn: sqlite3.Connection,
    snapshot_id: str,
    *,
    profile_id: str | None = None,
) -> dict[str, Any]:
    row = conn.execute(
        "SELECT * FROM filed_report_snapshots WHERE id = ?", (snapshot_id,)
    ).fetchone()
    if not row or (profile_id is not None and row["profile_id"] != profile_id):
        raise AppError("filed report snapshot was not found", code="not_found")
    return _public_snapshot(row)


def list_filed_report_snapshots(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    affected_years: Sequence[int] = (),
) -> list[dict[str, Any]]:
    years = sorted({_year(year, "affected_year") for year in affected_years})
    rows = conn.execute(
        """
        SELECT * FROM filed_report_snapshots
        WHERE profile_id = ?
        ORDER BY period_start_year, period_end_year, created_at, id
        """,
        (profile_id,),
    ).fetchall()
    return [
        _public_snapshot(row)
        for row in rows
        if not years
        or any(
            int(row["period_start_year"]) <= year <= int(row["period_end_year"])
            for year in years
        )
    ]


def _public_snapshot(row: Mapping[str, Any]) -> dict[str, Any]:
    return {
        "id": row["id"],
        "workspace_id": row["workspace_id"],
        "profile_id": row["profile_id"],
        "report_kind": row["report_kind"],
        "report_state": row["report_state"],
        "period_start_year": int(row["period_start_year"]),
        "period_end_year": int(row["period_end_year"]),
        "content_sha256": row["content_sha256"],
        "classification_summary": _json_object(row["classification_summary_json"]),
        "gain_summary": _json_object(row["gain_summary_json"]),
        "report_scope": _json_object(row["report_scope_json"]),
        "authored_source": row["authored_source"],
        "notes": row["notes"],
        "created_at": row["created_at"],
    }


def candidate_affected_years(
    candidate: Any,
    *,
    downstream_years: Sequence[int] = (),
) -> tuple[int, ...]:
    years = {
        int(text[:4])
        for value in (getattr(candidate, "started_at", None), getattr(candidate, "ended_at", None))
        if len(text := str(value or "")) >= 4 and text[:4].isdigit()
    }
    years.update(_year(year, "affected_year") for year in downstream_years)
    return tuple(sorted(years))


def bridge_after_classification_summary(candidate: Any) -> dict[str, Any]:
    summary: dict[str, Any] = {}
    retained = int(getattr(candidate, "retained_msat", 0) or 0)
    residual = int(getattr(candidate, "residual_msat", 0) or 0)
    fee = int(getattr(candidate, "source_fee_msat", 0) or 0)
    if retained:
        summary["internal_retained"] = {"count": 1, "amount_msat": retained}
    if residual:
        summary["custody_suspense"] = {"count": 1, "amount_msat": residual}
    if fee:
        summary["network_fee"] = {"count": 1, "amount_msat": fee}
    return _classification_summary(summary)


def preview_custody_impacts(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    candidate: Any,
    downstream_years: Sequence[int] = (),
    after_classification_summary: Mapping[str, Any] | None = None,
) -> list[dict[str, Any]]:
    years = candidate_affected_years(candidate, downstream_years=downstream_years)
    after_classification = (
        _classification_summary(after_classification_summary)
        if after_classification_summary is not None
        else bridge_after_classification_summary(candidate)
    )
    impacts = []
    for snapshot in list_filed_report_snapshots(
        conn, profile_id, affected_years=years
    ):
        overlap = [
            year
            for year in years
            if snapshot["period_start_year"] <= year <= snapshot["period_end_year"]
        ]
        impacts.append(
            {
                "filed_report_snapshot_id": snapshot["id"],
                "report_kind": snapshot["report_kind"],
                "report_state": snapshot["report_state"],
                "affected_period_start_year": min(overlap),
                "affected_period_end_year": max(overlap),
                "before_classification_summary": snapshot["classification_summary"],
                "after_classification_summary": after_classification,
                "before_gain_summary": snapshot["gain_summary"],
                "after_gain_summary": {"status": "pending_journal_rebuild"},
                "amendment_warning": AMENDMENT_WARNING,
            }
        )
    return impacts


def append_custody_impacts(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    component_id: str,
    review_id: str,
    gap_id: str,
    candidate: Any,
    downstream_years: Sequence[int] = (),
    after_classification_summary: Mapping[str, Any] | None = None,
    created_at: str | None = None,
) -> list[dict[str, Any]]:
    """Persist the previewed amendment warnings exactly once per review.

    This sealed row is activation audit history, not a mutable current-report
    projection. It therefore replicates with the authored snapshot and review.
    Exact post-review gains remain explicitly pending until journals rebuild.
    """

    timestamp = created_at or now_iso()
    previews = preview_custody_impacts(
        conn,
        profile_id=profile_id,
        candidate=candidate,
        downstream_years=downstream_years,
        after_classification_summary=after_classification_summary,
    )
    for preview in previews:
        impact_id = str(
            uuid.uuid5(
                uuid.NAMESPACE_URL,
                "kassiber:custody-filed-impact:"
                f"{preview['filed_report_snapshot_id']}:{review_id}",
            )
        )
        conn.execute(
            """
            INSERT OR IGNORE INTO custody_filed_report_impacts(
                id, workspace_id, profile_id, filed_report_snapshot_id,
                component_id, review_id, gap_id, affected_period_start_year,
                affected_period_end_year, before_classification_summary_json,
                after_classification_summary_json, before_gain_summary_json,
                after_gain_summary_json, amendment_warning, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                impact_id,
                workspace_id,
                profile_id,
                preview["filed_report_snapshot_id"],
                component_id,
                review_id,
                gap_id,
                preview["affected_period_start_year"],
                preview["affected_period_end_year"],
                _json(preview["before_classification_summary"]),
                _json(preview["after_classification_summary"]),
                _json(preview["before_gain_summary"]),
                _json(preview["after_gain_summary"]),
                preview["amendment_warning"],
                timestamp,
            ),
        )
    return list_custody_impacts(conn, profile_id, review_id=review_id)


def list_custody_impacts(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    review_id: str | None = None,
    transaction_ids: Sequence[str] | None = None,
) -> list[dict[str, Any]]:
    where = "profile_id = ?"
    params: list[Any] = [profile_id]
    if review_id is not None:
        where += " AND review_id = ?"
        params.append(review_id)
    if transaction_ids is not None:
        selected = tuple(sorted({str(value) for value in transaction_ids if value}))
        if not selected:
            return []
        placeholders = ",".join("?" for _ in selected)
        where += f"""
            AND EXISTS (
                SELECT 1 FROM custody_component_legs l
                WHERE l.profile_id = custody_filed_report_impacts.profile_id
                  AND l.component_id = custody_filed_report_impacts.component_id
                  AND COALESCE(l.anchor_transaction_id, l.transaction_id)
                      IN ({placeholders})
            )
        """
        params.extend(selected)
    rows = conn.execute(
        f"SELECT * FROM custody_filed_report_impacts WHERE {where} "
        "ORDER BY affected_period_start_year, created_at, id",
        params,
    ).fetchall()
    resolution_rows = conn.execute(
        """
        SELECT * FROM custody_filed_report_impact_resolutions
        WHERE profile_id = ?
        ORDER BY rebuilt_at, created_at, id
        """,
        (profile_id,),
    ).fetchall()
    resolutions = {
        str(row["impact_id"]): _public_impact_resolution(row)
        for row in resolution_rows
    }
    return [
        {
            "id": row["id"],
            "filed_report_snapshot_id": row["filed_report_snapshot_id"],
            "component_id": row["component_id"],
            "review_id": row["review_id"],
            "gap_id": row["gap_id"],
            "affected_period_start_year": int(row["affected_period_start_year"]),
            "affected_period_end_year": int(row["affected_period_end_year"]),
            "before_classification_summary": _json_object(
                row["before_classification_summary_json"]
            ),
            "after_classification_summary": _json_object(
                row["after_classification_summary_json"]
            ),
            "before_gain_summary": _json_object(row["before_gain_summary_json"]),
            "after_gain_summary": _json_object(row["after_gain_summary_json"]),
            "amendment_warning": row["amendment_warning"],
            "resolution": resolutions.get(str(row["id"])),
            "created_at": row["created_at"],
        }
        for row in rows
    ]


def _public_impact_resolution(row: Mapping[str, Any]) -> dict[str, Any]:
    return {
        "id": row["id"],
        "impact_id": row["impact_id"],
        "rebuilt_at": row["rebuilt_at"],
        "after_classification_summary": _json_object(
            row["after_classification_summary_json"]
        ),
        "after_gain_summary": _json_object(row["after_gain_summary_json"]),
        "classification_changed": bool(row["classification_changed"]),
        "gain_changed": bool(row["gain_changed"]),
        "amendment_status": row["amendment_status"],
        "created_at": row["created_at"],
    }


def list_custody_impact_resolutions(
    conn: sqlite3.Connection,
    profile_id: str,
) -> list[dict[str, Any]]:
    rows = conn.execute(
        """
        SELECT * FROM custody_filed_report_impact_resolutions
        WHERE profile_id = ?
        ORDER BY rebuilt_at, created_at, id
        """,
        (profile_id,),
    ).fetchall()
    return [_public_impact_resolution(row) for row in rows]


def resolve_pending_custody_impacts(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    rebuilt_at: str,
    created_at: str | None = None,
) -> list[dict[str, Any]]:
    """Append exact post-rebuild results for every unresolved custody impact.

    The caller must invoke this only after a successful, report-ready journal
    rebuild.  The activation-time impact stays sealed; this table is its one
    immutable resolution, computed from the freshly persisted journals.
    """

    timestamp = created_at or now_iso()
    rows = conn.execute(
        """
        SELECT impact.*, snapshot.report_state, snapshot.report_scope_json
        FROM custody_filed_report_impacts impact
        JOIN filed_report_snapshots snapshot
          ON snapshot.id = impact.filed_report_snapshot_id
        LEFT JOIN custody_filed_report_impact_resolutions resolution
          ON resolution.impact_id = impact.id
        WHERE impact.profile_id = ? AND resolution.id IS NULL
        ORDER BY impact.affected_period_start_year, impact.created_at, impact.id
        """,
        (profile_id,),
    ).fetchall()
    created_ids: list[str] = []
    for row in rows:
        summaries = current_report_summaries(
            conn,
            profile_id,
            period_start_year=int(row["affected_period_start_year"]),
            period_end_year=int(row["affected_period_end_year"]),
            report_scope=_json_object(row["report_scope_json"]),
        )
        before_classification = _json_object(
            row["before_classification_summary_json"]
        )
        before_gain = _json_object(row["before_gain_summary_json"])
        after_classification = summaries["classification_summary"]
        after_gain = summaries["gain_summary"]
        classification_changed = before_classification != after_classification
        gain_changed = before_gain != after_gain
        changed = classification_changed or gain_changed
        if not changed:
            amendment_status = "no_change"
        elif str(row["report_state"]) == "filed":
            amendment_status = "review_required"
        else:
            amendment_status = "saved_report_changed"
        if amendment_status not in _AMENDMENT_STATUSES:
            raise AssertionError("unsupported custody amendment status")
        resolution_id = str(
            uuid.uuid5(
                uuid.NAMESPACE_URL,
                f"kassiber:custody-filed-impact-resolution:{row['id']}",
            )
        )
        conn.execute(
            """
            INSERT INTO custody_filed_report_impact_resolutions(
                id, workspace_id, profile_id, impact_id, rebuilt_at,
                after_classification_summary_json, after_gain_summary_json,
                classification_changed, gain_changed, amendment_status,
                created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                resolution_id,
                workspace_id,
                profile_id,
                row["id"],
                rebuilt_at,
                _json(after_classification),
                _json(after_gain),
                int(classification_changed),
                int(gain_changed),
                amendment_status,
                timestamp,
            ),
        )
        created_ids.append(resolution_id)
    if not created_ids:
        return []
    placeholders = ", ".join("?" for _ in created_ids)
    resolved = conn.execute(
        f"SELECT * FROM custody_filed_report_impact_resolutions "
        f"WHERE id IN ({placeholders}) ORDER BY rebuilt_at, created_at, id",
        created_ids,
    ).fetchall()
    return [_public_impact_resolution(row) for row in resolved]


__all__ = [
    "AMENDMENT_WARNING",
    "append_custody_impacts",
    "artifact_content_sha256",
    "bridge_after_classification_summary",
    "candidate_affected_years",
    "create_filed_report_snapshot",
    "current_report_summaries",
    "get_filed_report_snapshot",
    "list_custody_impact_resolutions",
    "list_custody_impacts",
    "list_filed_report_snapshots",
    "preview_custody_impacts",
    "register_saved_report_export",
    "report_period_years",
    "resolve_pending_custody_impacts",
]
