"""One-shot behavioral audit for the custody-aware tax cutover.

Schema setup only creates empty local tables.  The legacy classification is
captured immediately before journal processing replaces derived rows, and the
bounded comparison is appended only inside the first successful rebuild
savepoint.  Raw transactions, descriptors, wallet configuration, and evidence
payloads never enter this boundary.

Exact monetary totals are compared only when both legacy and rebuilt journal
rows provide exact columns. Saved/filed-report impact tracking remains the
owner of amendment conclusions; this module reports behavioral facts only.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from decimal import Decimal, InvalidOperation
import hashlib
import json
import sqlite3
from typing import Any, Iterable, Mapping

from ..time_utils import now_iso


CUSTODY_TAX_BEHAVIOR_MIGRATION = "custody-tax-behavior-v1"
MIGRATION_SCHEMA_VERSION = 1
MAX_DETAILED_CHANGES = 500
_MAX_TRANSACTION_ID_CHARS = 128
_MAX_ASSET_CHARS = 32

_CLASSIFICATION_ORDER = (
    "income",
    "acquisition",
    "disposal",
    "retained",
    "suspense",
    # Read-only compatibility for behavioral baselines captured by the
    # pre-simplification journal. No current quantity emitter produces it.
    "custody_candidate",
    "conflicting",
    "quarantined",
    "fee",
    "other",
)
_CLASSIFICATION_TOKENS = frozenset((*_CLASSIFICATION_ORDER, "absent"))
_REASONS = frozenset(
    {
        "legacy_external_presumption_reclassified_by_custody",
        "new_engine_quarantine",
        "new_engine_classification_added",
        "legacy_classification_removed",
        "classification_quantity_changed",
        "classification_monetary_changed",
        "classification_changed",
    }
)
_MONETARY_FIELDS = (
    "fiat_value_exact",
    "cost_basis_exact",
    "proceeds_exact",
    "gain_loss_exact",
)


@dataclass
class _Event:
    transaction_id: str
    asset: str
    tokens: set[str] = field(default_factory=set)
    years: set[int] = field(default_factory=set)
    amounts_msat: dict[str, int | None] = field(default_factory=dict)
    monetary_exact: dict[str, dict[str, Decimal]] = field(default_factory=dict)

    @property
    def event_id(self) -> str:
        material = (
            f"{CUSTODY_TAX_BEHAVIOR_MIGRATION}\0"
            f"{self.transaction_id}\0{self.asset}"
        )
        return "ctm:" + hashlib.sha256(material.encode("utf-8")).hexdigest()

    @property
    def visible_tokens(self) -> tuple[str, ...]:
        tokens = set(self.tokens)
        if len(tokens) > 1:
            tokens.discard("fee")
        return tuple(token for token in _CLASSIFICATION_ORDER if token in tokens)

    @property
    def classification(self) -> str:
        ordered = self.visible_tokens
        return "+".join(ordered) if ordered else "other"

    @property
    def public_amounts_msat(self) -> dict[str, str | None]:
        return {
            token: (
                str(self.amounts_msat.get(token))
                if self.amounts_msat.get(token) is not None
                else None
            )
            for token in self.visible_tokens
        }

    @property
    def public_monetary_exact(self) -> dict[str, dict[str, str]]:
        return {
            token: {
                field_name: format(values[field_name], "f")
                for field_name in _MONETARY_FIELDS
                if field_name in values
            }
            for token in self.visible_tokens
            if (values := self.monetary_exact.get(token))
        }

    def add_amount(self, token: str, amount_msat: int | None) -> None:
        current = self.amounts_msat.get(token)
        if amount_msat is None or (token in self.amounts_msat and current is None):
            self.amounts_msat[token] = None
            return
        self.amounts_msat[token] = int(current or 0) + abs(int(amount_msat))

    def add_monetary(self, token: str, values: Mapping[str, Any] | None) -> None:
        if not values:
            return
        totals = self.monetary_exact.setdefault(token, {})
        for field_name in _MONETARY_FIELDS:
            value = values.get(field_name)
            if value in (None, ""):
                continue
            try:
                exact = Decimal(str(value))
            except (InvalidOperation, TypeError, ValueError):
                continue
            totals[field_name] = totals.get(field_name, Decimal("0")) + exact


def _year(value: Any) -> int | None:
    prefix = str(value or "")[:4]
    if len(prefix) != 4 or not prefix.isdigit():
        return None
    year = int(prefix)
    return year if 1900 <= year <= 9999 else None


def _journal_token(entry_type: Any) -> str:
    value = str(entry_type or "").lower()
    if value in {"transfer_in", "transfer_out"}:
        return "retained"
    if value in {"transfer_fee", "fee"}:
        return "fee"
    if value in {"income", "acquisition", "disposal"}:
        return value
    return "other"


def _issue_token(state: Any) -> str | None:
    value = str(state or "").lower()
    return {
        "custody_suspense": "suspense",
        "custody_candidate": "custody_candidate",
        "conflicting": "conflicting",
    }.get(value)


def _transaction_refs(conn: sqlite3.Connection, profile_id: str) -> dict[str, tuple[str, Any]]:
    return {
        str(row["id"]): (str(row["asset"] or "UNKNOWN").upper(), row["occurred_at"])
        for row in conn.execute(
            "SELECT id, asset, occurred_at FROM transactions WHERE profile_id = ?",
            (profile_id,),
        ).fetchall()
    }


def _classification_events(
    conn: sqlite3.Connection,
    profile_id: str,
) -> dict[str, _Event]:
    refs = _transaction_refs(conn, profile_id)
    events: dict[tuple[str, str], _Event] = {}

    def add(
        transaction_id: Any,
        asset: Any,
        token: str,
        occurred_at: Any,
        *,
        amount_msat: int | None,
        monetary_exact: Mapping[str, Any] | None = None,
    ) -> None:
        tx_id = str(transaction_id or "").strip()
        if not tx_id or token not in _CLASSIFICATION_TOKENS:
            return
        fallback_asset, fallback_time = refs.get(tx_id, ("UNKNOWN", None))
        normalized_asset = str(asset or fallback_asset).upper()
        key = (tx_id, normalized_asset)
        event = events.setdefault(key, _Event(tx_id, normalized_asset))
        event.tokens.add(token)
        event.add_amount(token, amount_msat)
        event.add_monetary(token, monetary_exact)
        observed_year = _year(occurred_at or fallback_time)
        if observed_year is not None:
            event.years.add(observed_year)

    for row in conn.execute(
        """
        SELECT transaction_id, asset, entry_type, occurred_at, quantity,
               fiat_value_exact, cost_basis_exact, proceeds_exact, gain_loss_exact
        FROM journal_entries
        WHERE profile_id = ?
        ORDER BY transaction_id, asset, entry_type, occurred_at, id
        """,
        (profile_id,),
    ).fetchall():
        add(
            row["transaction_id"],
            row["asset"],
            _journal_token(row["entry_type"]),
            row["occurred_at"],
            amount_msat=int(row["quantity"]),
            monetary_exact={field_name: row[field_name] for field_name in _MONETARY_FIELDS},
        )

    for row in conn.execute(
        """
        SELECT quarantine.transaction_id, tx.asset, tx.occurred_at
        FROM journal_quarantines quarantine
        LEFT JOIN transactions tx ON tx.id = quarantine.transaction_id
        WHERE quarantine.profile_id = ?
        ORDER BY quarantine.transaction_id
        """,
        (profile_id,),
    ).fetchall():
        add(
            row["transaction_id"],
            row["asset"],
            "quarantined",
            row["occurred_at"],
            amount_msat=None,
        )

    for row in conn.execute(
        """
        SELECT state, asset, amount_msat, occurred_at, transaction_ids_json
        FROM journal_quantity_issues
        WHERE profile_id = ?
        ORDER BY issue_id
        """,
        (profile_id,),
    ).fetchall():
        token = _issue_token(row["state"])
        if token is None:
            continue
        try:
            transaction_ids = json.loads(row["transaction_ids_json"] or "[]")
        except (TypeError, ValueError, json.JSONDecodeError):
            continue
        if not isinstance(transaction_ids, list):
            continue
        exact_amount = (
            int(row["amount_msat"])
            if len(transaction_ids) == 1 and row["amount_msat"] is not None
            else None
        )
        for transaction_id in transaction_ids:
            if isinstance(transaction_id, str):
                add(
                    transaction_id,
                    row["asset"],
                    token,
                    row["occurred_at"],
                    amount_msat=exact_amount,
                )

    return {event.event_id: event for event in events.values()}


def _count(conn: sqlite3.Connection, table: str, profile_id: str) -> int:
    return int(
        conn.execute(
            f"SELECT COUNT(*) FROM {table} WHERE profile_id = ?",
            (profile_id,),
        ).fetchone()[0]
    )


def capture_legacy_baseline(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    captured_at: str | None = None,
) -> dict[str, Any]:
    """Capture pre-rebuild classifications once, without committing."""

    report = conn.execute(
        """
        SELECT id FROM custody_tax_migration_reports
        WHERE profile_id = ? AND migration_name = ?
        """,
        (profile_id, CUSTODY_TAX_BEHAVIOR_MIGRATION),
    ).fetchone()
    if report is not None:
        return {"captured": False, "reason": "comparison_already_recorded"}
    existing = conn.execute(
        "SELECT event_count FROM custody_tax_migration_baselines WHERE profile_id = ?",
        (profile_id,),
    ).fetchone()
    if existing is not None:
        return {
            "captured": False,
            "reason": "baseline_already_captured",
            "event_count": int(existing["event_count"]),
        }

    journal_entry_count = _count(conn, "journal_entries", profile_id)
    tax_summary_count = _count(conn, "journal_tax_summary", profile_id)
    if journal_entry_count == 0 and tax_summary_count == 0:
        return {"captured": False, "reason": "no_legacy_derived_rows"}

    events = _classification_events(conn, profile_id)
    timestamp = captured_at or now_iso()
    conn.execute(
        """
        INSERT INTO custody_tax_migration_baselines(
            profile_id, workspace_id, migration_name, schema_version,
            journal_entry_count, tax_summary_count, event_count, captured_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            profile_id,
            workspace_id,
            CUSTODY_TAX_BEHAVIOR_MIGRATION,
            MIGRATION_SCHEMA_VERSION,
            journal_entry_count,
            tax_summary_count,
            len(events),
            timestamp,
        ),
    )
    conn.executemany(
        """
        INSERT INTO custody_tax_migration_baseline_events(
            profile_id, event_id, transaction_id, asset, classification,
            amounts_msat_json, monetary_exact_json, affected_years_json
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
        """,
        [
            (
                profile_id,
                event_id,
                event.transaction_id,
                event.asset,
                event.classification,
                json.dumps(
                    event.public_amounts_msat,
                    sort_keys=True,
                    separators=(",", ":"),
                ),
                json.dumps(
                    event.public_monetary_exact,
                    sort_keys=True,
                    separators=(",", ":"),
                ),
                json.dumps(sorted(event.years), separators=(",", ":")),
            )
            for event_id, event in sorted(events.items())
        ],
    )
    return {
        "captured": True,
        "event_count": len(events),
        "journal_entry_count": journal_entry_count,
        "tax_summary_count": tax_summary_count,
        "captured_at": timestamp,
    }


def _load_baseline_events(
    conn: sqlite3.Connection,
    profile_id: str,
) -> dict[str, dict[str, Any]]:
    output: dict[str, dict[str, Any]] = {}
    for row in conn.execute(
        """
        SELECT event_id, transaction_id, asset, classification,
               amounts_msat_json, monetary_exact_json, affected_years_json
        FROM custody_tax_migration_baseline_events
        WHERE profile_id = ?
        ORDER BY event_id
        """,
        (profile_id,),
    ).fetchall():
        try:
            years = json.loads(row["affected_years_json"] or "[]")
        except (TypeError, ValueError, json.JSONDecodeError):
            years = []
        try:
            amounts_msat = json.loads(row["amounts_msat_json"] or "{}")
        except (TypeError, ValueError, json.JSONDecodeError):
            amounts_msat = {}
        try:
            monetary_exact = json.loads(row["monetary_exact_json"] or "{}")
        except (TypeError, ValueError, json.JSONDecodeError):
            monetary_exact = {}
        output[str(row["event_id"])] = {
            "transaction_id": str(row["transaction_id"]),
            "asset": str(row["asset"]),
            "classification": str(row["classification"]),
            "amounts_msat": amounts_msat if isinstance(amounts_msat, dict) else {},
            "monetary_exact": (
                monetary_exact if isinstance(monetary_exact, dict) else {}
            ),
            "years": {
                int(value)
                for value in years
                if isinstance(value, int) and 1900 <= value <= 9999
            },
        }
    return output


def _monetary_changed(
    old: Mapping[str, Any],
    new: Mapping[str, Any],
) -> bool:
    """Compare only exact fields available on both sides of the cutover."""

    for token in set(old) & set(new):
        old_values = old.get(token)
        new_values = new.get(token)
        if not isinstance(old_values, Mapping) or not isinstance(new_values, Mapping):
            continue
        for field_name in set(old_values) & set(new_values) & set(_MONETARY_FIELDS):
            try:
                old_value = Decimal(str(old_values[field_name]))
                new_value = Decimal(str(new_values[field_name]))
            except (InvalidOperation, TypeError, ValueError):
                # Baselines are written by this module, so malformed values imply
                # corruption and must not silently compare equal.
                return True
            if old_value != new_value:
                return True
    return False


def _change_reason(
    old: str,
    new: str,
    *,
    quantity_changed: bool,
    monetary_changed: bool,
) -> str:
    old_tokens = set(old.split("+"))
    new_tokens = set(new.split("+"))
    if old_tokens & {"disposal", "acquisition"} and new_tokens & {
        "retained",
        "suspense",
    }:
        return "legacy_external_presumption_reclassified_by_custody"
    if "quarantined" in new_tokens:
        return "new_engine_quarantine"
    if old == "absent":
        return "new_engine_classification_added"
    if new == "absent":
        return "legacy_classification_removed"
    if quantity_changed:
        return "classification_quantity_changed"
    if monetary_changed:
        return "classification_monetary_changed"
    return "classification_changed"


def finalize_first_rebuild(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    rebuilt_at: str,
) -> dict[str, Any] | None:
    """Append the first bounded post-rebuild comparison without committing."""

    existing = conn.execute(
        """
        SELECT comparison_json FROM custody_tax_migration_reports
        WHERE profile_id = ? AND migration_name = ?
        """,
        (profile_id, CUSTODY_TAX_BEHAVIOR_MIGRATION),
    ).fetchone()
    if existing is not None:
        try:
            value = json.loads(existing["comparison_json"])
        except (TypeError, ValueError, json.JSONDecodeError):
            return None
        return value if isinstance(value, dict) else None
    baseline = conn.execute(
        """
        SELECT * FROM custody_tax_migration_baselines
        WHERE profile_id = ? AND migration_name = ?
        """,
        (profile_id, CUSTODY_TAX_BEHAVIOR_MIGRATION),
    ).fetchone()
    if baseline is None:
        return None

    old_events = _load_baseline_events(conn, profile_id)
    if len(old_events) != int(baseline["event_count"]):
        raise sqlite3.IntegrityError("custody_tax_migration_baseline_incomplete")
    rebuilt_events = _classification_events(conn, profile_id)
    changes: list[dict[str, Any]] = []
    for event_id in sorted(set(old_events) | set(rebuilt_events)):
        old = old_events.get(event_id)
        rebuilt = rebuilt_events.get(event_id)
        old_classification = old["classification"] if old else "absent"
        new_classification = rebuilt.classification if rebuilt else "absent"
        old_amounts = old["amounts_msat"] if old else {}
        new_amounts = rebuilt.public_amounts_msat if rebuilt else {}
        old_monetary = old["monetary_exact"] if old else {}
        new_monetary = rebuilt.public_monetary_exact if rebuilt else {}
        quantity_changed = old_amounts != new_amounts
        monetary_changed = _monetary_changed(old_monetary, new_monetary)
        if (
            old_classification == new_classification
            and not quantity_changed
            and not monetary_changed
        ):
            continue
        transaction_id = (
            old["transaction_id"] if old is not None else rebuilt.transaction_id
        )
        asset = old["asset"] if old is not None else rebuilt.asset
        years = set(old["years"] if old is not None else ())
        if rebuilt is not None:
            years.update(rebuilt.years)
        changes.append(
            {
                "event_id": event_id,
                "transaction_id": str(transaction_id)[:_MAX_TRANSACTION_ID_CHARS],
                "asset": str(asset)[:_MAX_ASSET_CHARS],
                "old_classification": old_classification,
                "new_classification": new_classification,
                "old_amounts_msat": old_amounts,
                "new_amounts_msat": new_amounts,
                "old_monetary_exact": old_monetary,
                "new_monetary_exact": new_monetary,
                "reason": _change_reason(
                    old_classification,
                    new_classification,
                    quantity_changed=quantity_changed,
                    monetary_changed=monetary_changed,
                ),
                "affected_years": sorted(years),
            }
        )

    bounded_changes = changes[:MAX_DETAILED_CHANGES]
    status = (
        "complete"
        if int(baseline["journal_entry_count"]) > 0
        else "baseline_without_event_detail"
    )
    comparison = {
        "schema_version": MIGRATION_SCHEMA_VERSION,
        "migration": CUSTODY_TAX_BEHAVIOR_MIGRATION,
        "status": status,
        "baseline_captured_at": baseline["captured_at"],
        "rebuilt_at": rebuilt_at,
        "summary": {
            "baseline_journal_entry_count": int(baseline["journal_entry_count"]),
            "baseline_tax_summary_count": int(baseline["tax_summary_count"]),
            "baseline_event_count": int(baseline["event_count"]),
            "rebuilt_event_count": len(rebuilt_events),
            "changed_event_count": len(changes),
            "detailed_change_count": len(bounded_changes),
            "changes_truncated": len(changes) > len(bounded_changes),
        },
        "changes": bounded_changes,
    }
    encoded = json.dumps(comparison, sort_keys=True, separators=(",", ":"))
    report_id = f"{CUSTODY_TAX_BEHAVIOR_MIGRATION}:{profile_id}"
    conn.execute(
        """
        INSERT INTO custody_tax_migration_reports(
            id, workspace_id, profile_id, migration_name, schema_version,
            status, baseline_captured_at, rebuilt_at, baseline_event_count,
            rebuilt_event_count, changed_event_count, detailed_change_count,
            changes_truncated, comparison_json, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            report_id,
            workspace_id,
            profile_id,
            CUSTODY_TAX_BEHAVIOR_MIGRATION,
            MIGRATION_SCHEMA_VERSION,
            status,
            baseline["captured_at"],
            rebuilt_at,
            int(baseline["event_count"]),
            len(rebuilt_events),
            len(changes),
            len(bounded_changes),
            int(len(changes) > len(bounded_changes)),
            encoded,
            rebuilt_at,
        ),
    )
    return comparison


def _safe_classification(value: Any) -> str:
    tokens = str(value or "").split("+")
    if not tokens or any(token not in _CLASSIFICATION_TOKENS for token in tokens):
        return "other"
    return "+".join(tokens)[:96]


def _safe_amounts(value: Any) -> dict[str, str | None]:
    if not isinstance(value, Mapping):
        return {}
    output: dict[str, str | None] = {}
    for token in _CLASSIFICATION_ORDER:
        if token not in value:
            continue
        raw = value[token]
        if raw is None:
            output[token] = None
            continue
        text = str(raw)
        if text.isdigit() and len(text) <= 40:
            output[token] = text
    return output


def _safe_monetary(value: Any) -> dict[str, dict[str, str]]:
    if not isinstance(value, Mapping):
        return {}
    output: dict[str, dict[str, str]] = {}
    for token in _CLASSIFICATION_ORDER:
        fields = value.get(token)
        if not isinstance(fields, Mapping):
            continue
        safe_fields: dict[str, str] = {}
        for field_name in _MONETARY_FIELDS:
            raw = fields.get(field_name)
            if raw in (None, "") or len(str(raw)) > 64:
                continue
            try:
                safe_fields[field_name] = format(Decimal(str(raw)), "f")
            except (InvalidOperation, TypeError, ValueError):
                continue
        if safe_fields:
            output[token] = safe_fields
    return output


def list_redacted_reports(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    transaction_ids: Iterable[str] | None = None,
) -> list[dict[str, Any]]:
    """Return the bounded audit-package vocabulary; never raw baseline rows."""

    output: list[dict[str, Any]] = []
    allowed_transaction_ids = (
        None
        if transaction_ids is None
        else {str(transaction_id) for transaction_id in transaction_ids}
    )
    rows = conn.execute(
        """
        SELECT migration_name, schema_version, status, comparison_json, created_at
        FROM custody_tax_migration_reports
        WHERE profile_id = ?
        ORDER BY created_at, migration_name
        """,
        (profile_id,),
    ).fetchall()
    for row in rows:
        try:
            raw = json.loads(row["comparison_json"] or "{}")
        except (TypeError, ValueError, json.JSONDecodeError):
            raw = {}
        raw_summary = raw.get("summary") if isinstance(raw, dict) else {}
        if not isinstance(raw_summary, dict):
            raw_summary = {}
        summary = {
            key: max(0, int(raw_summary.get(key) or 0))
            for key in (
                "baseline_journal_entry_count",
                "baseline_tax_summary_count",
                "baseline_event_count",
                "rebuilt_event_count",
                "changed_event_count",
                "detailed_change_count",
            )
        }
        summary["changes_truncated"] = bool(
            raw_summary.get("changes_truncated", False)
        )
        changes = []
        raw_changes = raw.get("changes") if isinstance(raw, dict) else []
        for change in raw_changes[:MAX_DETAILED_CHANGES] if isinstance(raw_changes, list) else ():
            if not isinstance(change, dict):
                continue
            transaction_id = str(change.get("transaction_id") or "")
            if (
                allowed_transaction_ids is not None
                and transaction_id not in allowed_transaction_ids
            ):
                continue
            reason = str(change.get("reason") or "")
            if reason not in _REASONS:
                reason = "classification_changed"
            years = change.get("affected_years")
            changes.append(
                {
                    "event_id": str(change.get("event_id") or "")[:80],
                    "transaction_id": transaction_id[:128],
                    "asset": str(change.get("asset") or "")[:32],
                    "old_classification": _safe_classification(
                        change.get("old_classification")
                    ),
                    "new_classification": _safe_classification(
                        change.get("new_classification")
                    ),
                    "old_amounts_msat": _safe_amounts(
                        change.get("old_amounts_msat")
                    ),
                    "new_amounts_msat": _safe_amounts(
                        change.get("new_amounts_msat")
                    ),
                    "old_monetary_exact": _safe_monetary(
                        change.get("old_monetary_exact")
                    ),
                    "new_monetary_exact": _safe_monetary(
                        change.get("new_monetary_exact")
                    ),
                    "reason": reason,
                    "affected_years": sorted(
                        {
                            int(year)
                            for year in years
                            if isinstance(year, int) and 1900 <= year <= 9999
                        }
                    )
                    if isinstance(years, list)
                    else [],
                }
            )
        if allowed_transaction_ids is not None:
            summary["changed_event_count"] = len(changes)
            summary["detailed_change_count"] = len(changes)
        summary["transaction_scope_filtered"] = allowed_transaction_ids is not None
        output.append(
            {
                "migration_name": str(row["migration_name"])[:64],
                "schema_version": int(row["schema_version"]),
                "status": str(row["status"]),
                "baseline_captured_at": str(raw.get("baseline_captured_at") or "")[:40]
                if isinstance(raw, dict)
                else "",
                "rebuilt_at": str(raw.get("rebuilt_at") or "")[:40]
                if isinstance(raw, dict)
                else "",
                "summary": summary,
                "changes": changes,
                "created_at": row["created_at"],
            }
        )
    return output


__all__ = [
    "CUSTODY_TAX_BEHAVIOR_MIGRATION",
    "MAX_DETAILED_CHANGES",
    "capture_legacy_baseline",
    "finalize_first_rebuild",
    "list_redacted_reports",
]
