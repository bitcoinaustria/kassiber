"""Plan and apply importing a document draft from chat, bound to its plan.

A photo or PDF the user attached to a chat is read by the local vision model
(`core.document_import.read_document`) into a draft of rows, each `ready` or
held back for review. This module answers, without changing the book, what
importing the draft's ready rows would do, and applies exactly that after the
user's consent:

- Only `ready` rows are imported. Rows the reader was unsure of stay for the
  desktop's document import, where the user reviews them against the page.
- The plan is bound to the draft, the document's SHA-256 (the import refuses
  a file that changed since it was read), the book's input version and a
  digest of the effect; applying refuses if any of them moved.
- Targets follow the spreadsheet import: a file-backed wallet, or a new
  `custom` wallet. The document is attached to each imported row as
  evidence, and the import is recorded as an import run. Because a rollback
  would also delete that evidence, it is the desktop's to undo (its import
  history shows what goes), not the assistant's.
"""

from __future__ import annotations

import os
import sqlite3
from typing import Any, Mapping

from ..errors import AppError
from . import attachment_import as core_attachment_import
from . import document_import as core_document_import
from . import import_batches as core_import_batches
from . import wallet_completeness as core_wallet_completeness
from . import wallets as core_wallets

PLAN_SCHEMA_VERSION = 1
SOURCE_FORMAT = core_document_import.DOCUMENT_IMPORT_FORMAT
NEW_WALLET_KIND = "custom"
_SAVEPOINT = "document_chat_import_plan"
_PLAN_TOOL = "ui_wallets_import_document_plan"
# What the model may see of a row it read: values, never the page text.
_ROW_FIELDS = ("id", "status", "flags", "confidence", "record")


def ready_row_ids(draft: Mapping[str, Any]) -> list[str]:
    return [
        str(row["id"])
        for row in draft.get("rows") or ()
        if isinstance(row, Mapping) and row.get("status") == "ready" and row.get("id")
    ]


def project_draft(draft: Mapping[str, Any], *, full_access: bool) -> dict[str, Any]:
    """The draft as the model sees it.

    With full access: each row's values, status, flags and confidence. A
    restricted provider gets the counts only, as for a spreadsheet's cells.
    The local path, the page text the reader quoted and its source regions
    never go to the model.
    """
    source = draft.get("source") if isinstance(draft.get("source"), Mapping) else {}
    projected: dict[str, Any] = {
        "source": {
            key: source.get(key)
            for key in ("kind", "sha256", "size_bytes", "pdf", *(("filename",) if full_access else ()))
            if key in source
        },
        "model": draft.get("model"),
        "confidence_threshold": draft.get("confidence_threshold"),
        "summary": draft.get("summary"),
    }
    if full_access:
        projected["rows"] = [
            {key: row.get(key) for key in _ROW_FIELDS if key in row}
            for row in draft.get("rows") or ()
            if isinstance(row, Mapping)
        ]
        projected["untrusted_document_text"] = True
    else:
        projected["row_values_withheld"] = True
    return projected


def _resolve_target(
    conn: sqlite3.Connection, profile: Mapping[str, Any], target: Mapping[str, Any]
) -> tuple[dict[str, Any], list[dict[str, Any]], Mapping[str, Any] | None]:
    described, blockers, wallet = core_attachment_import._resolve_target(conn, profile, target)
    for blocker in blockers:
        if blocker.get("remediation", {}).get("tool"):
            blocker["remediation"]["tool"] = _PLAN_TOOL
    if described.get("action") == "create":
        described["completeness_class"] = core_wallet_completeness.completeness_class(
            NEW_WALLET_KIND, {"source_format": SOURCE_FORMAT}
        )
    return described, blockers, wallet


def _import(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    wallet: Mapping[str, Any],
    *,
    source_file: str,
    draft: Mapping[str, Any],
    hooks: core_document_import.DocumentImportHooks,
    data_root: str,
    attach_evidence: bool,
) -> dict[str, Any]:
    """Import the draft's ready rows and record the run; the caller commits."""
    source = draft.get("source") if isinstance(draft.get("source"), Mapping) else {}
    outcome = core_document_import.import_document_draft(
        conn,
        source_file=source_file,
        data_root=data_root,
        wallet=wallet,
        profile=profile,
        rows=[row for row in draft.get("rows") or () if isinstance(row, Mapping)],
        hooks=hooks,
        include_quarantined=False,
        selected_row_ids=ready_row_ids(draft),
        expected_source_sha256=str(source.get("sha256") or ""),
        confidence_threshold=draft.get("confidence_threshold"),
        attach_evidence=attach_evidence,
        commit=False,
    )
    # Records-based imports record no run of their own; this one is a file
    # import like any other, so it gets one.
    batch_id = core_import_batches.record_batch(
        conn,
        profile,
        wallet_id=str(wallet["id"]),
        source_format=SOURCE_FORMAT,
        source_filename=os.path.basename(str(source.get("filename") or source_file)),
        column_map=None,
        outcome=outcome,
    )
    if batch_id:
        outcome["import_batch_id"] = batch_id
    return outcome


def _create_target_wallet(
    conn: sqlite3.Connection, described: Mapping[str, Any]
) -> Mapping[str, Any]:
    return core_wallets.create_wallet(
        conn,
        None,
        None,
        described["label"],
        NEW_WALLET_KIND,
        account_ref=described.get("account"),
        config={"source_format": SOURCE_FORMAT},
        commit=False,
    )


def plan_or_apply(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    *,
    source_file: str,
    draft: Mapping[str, Any],
    request: Mapping[str, Any],
    hooks: core_document_import.DocumentImportHooks,
    data_root: str,
    expected: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    """Plan importing ``draft``'s ready rows (``expected`` None), or apply it.

    ``request`` carries the target (``wallet`` or ``new_wallet``). ``expected``
    carries ``plan_digest``, ``expected_source_sha256`` and
    ``expected_input_version`` from an earlier plan.
    """

    target = core_attachment_import._normalize_target(request)
    version = core_attachment_import.input_version(conn, str(profile["id"]))
    source = draft.get("source") if isinstance(draft.get("source"), Mapping) else {}
    source_sha256 = str(source.get("sha256") or "")
    if expected is not None:
        if expected.get("expected_source_sha256") != source_sha256:
            raise AppError(
                "The document changed since it was read",
                code="attachment_changed",
                hint="Read the document again, plan again and ask for consent again.",
                retryable=False,
            )
        if expected.get("expected_input_version") != version:
            raise AppError(
                "The book changed since the import was planned",
                code="import_plan_stale",
                hint="Plan the import again against the current book.",
                details={
                    "expected_input_version": expected.get("expected_input_version"),
                    "current_input_version": version,
                    "remediation": {"action": "replan", "tool": _PLAN_TOOL, "target": {}, "desktop_only": False},
                },
                retryable=False,
            )
    summary = draft.get("summary") if isinstance(draft.get("summary"), Mapping) else {}
    ready = ready_row_ids(draft)
    described_target, blockers, wallet = _resolve_target(conn, profile, target)
    warnings: list[str] = []
    if not ready:
        blockers.append(
            core_attachment_import._blocker(
                "nothing_ready",
                "The reader was sure of no row; review the document in the desktop's document import.",
                action="review_in_desktop",
                desktop_only=True,
            )
        )
    held_back = int(summary.get("quarantined") or 0)
    if held_back:
        warnings.append("rows_held_for_review")
    rows = {"read": int(summary.get("rows") or 0), "ready": len(ready), "held_for_review": held_back,
            "to_import": 0, "already_present": 0}
    effects = core_attachment_import._batch_effects(conn, str(profile["id"]), "", None)
    simulate = not any(
        blocker["code"] in {"nothing_ready", "target_not_file_wallet", "wallet_label_taken"}
        for blocker in blockers
    )
    conn.execute(f"SAVEPOINT {_SAVEPOINT}")
    try:
        if simulate:
            try:
                planned_wallet = wallet if wallet is not None else _create_target_wallet(conn, described_target)
                outcome = _import(
                    conn, profile, planned_wallet, source_file=source_file, draft=draft,
                    hooks=hooks, data_root=data_root, attach_evidence=False,
                )
            except AppError as exc:
                blockers.append(core_attachment_import._blocker(exc.code or "import_failed", str(exc)))
                outcome = {}
            if outcome:
                rows.update(
                    to_import=int(outcome.get("imported") or 0),
                    already_present=int(outcome.get("unchanged") or 0),
                )
                effects = core_attachment_import._batch_effects(
                    conn, str(profile["id"]), str(planned_wallet["id"]), outcome.get("import_batch_id")
                )
                if outcome.get("updated_records"):
                    warnings.append("updates_existing_rows_not_rolled_back")
                if not rows["to_import"]:
                    blockers.append(
                        core_attachment_import._blocker(
                            "nothing_to_import",
                            "No new rows: every ready row is already in this wallet.",
                            action="none",
                        )
                    )
                if rows["already_present"]:
                    warnings.append("rows_already_present")
                if effects["overlap"]["existing_rows_in_date_range"]:
                    warnings.append("overlaps_existing_history")
        plan = {
            "schema_version": PLAN_SCHEMA_VERSION,
            "workspace_id": str(profile["workspace_id"]),
            "profile_id": str(profile["id"]),
            "input_version": version,
            "document": {"sha256": source_sha256, "kind": source.get("kind")},
            "source_format": SOURCE_FORMAT,
            "row_ids": ready,
            "target": described_target,
            "rows": rows,
            **effects,
            "warnings": warnings,
            "blockers": blockers,
            "ready": not blockers,
        }
        plan_digest = core_attachment_import._digest(plan)
    finally:
        # The simulation never stays, even when applying: the real import
        # below runs once more with the document attached as evidence.
        conn.execute(f"ROLLBACK TO SAVEPOINT {_SAVEPOINT}")
        conn.execute(f"RELEASE SAVEPOINT {_SAVEPOINT}")
    if expected is None:
        return {
            **plan,
            "plan_digest": plan_digest,
            "apply_with": {
                "plan_digest": plan_digest,
                "expected_source_sha256": source_sha256,
                "expected_input_version": version,
            },
        }
    if plan["blockers"]:
        raise AppError(
            "This import is blocked",
            code="import_plan_blocked",
            details={"blockers": plan["blockers"]},
            retryable=False,
        )
    if expected.get("plan_digest") != plan_digest:
        raise AppError(
            "The import no longer matches the approved plan",
            code="import_plan_stale",
            hint="Plan the import again and ask for consent again.",
            details={"remediation": {"action": "replan", "tool": _PLAN_TOOL, "target": {}, "desktop_only": False}},
            retryable=False,
        )
    try:
        target_wallet = wallet if wallet is not None else _create_target_wallet(conn, described_target)
        outcome = _import(
            conn, profile, target_wallet, source_file=source_file, draft=draft,
            hooks=hooks, data_root=data_root, attach_evidence=True,
        )
        conn.commit()
    except Exception:
        conn.rollback()
        raise
    return {
        "applied": True,
        "plan_digest": plan_digest,
        "import_batch_id": outcome.get("import_batch_id"),
        "wallet": {"id": str(target_wallet["id"]), "label": target_wallet["label"],
                   "created": described_target["action"] == "create"},
        "rows": {**rows, "imported": int(outcome.get("imported") or 0)},
        "evidence_attached": len(outcome.get("attached_evidence") or []),
        **effects,
        "warnings": warnings,
        "journals_invalidated": bool(outcome.get("journal_invalidated")),
        "rollback": {
            "batch": outcome.get("import_batch_id"),
            "desktop_only": True,
            "why": "the document is kept as evidence on the imported rows",
        },
    }


def withhold_values(payload: Mapping[str, Any]) -> dict[str, Any]:
    """A plan or result for a restricted provider: counts and target, no values."""

    projected = core_attachment_import.withhold_cell_values(payload)
    projected.pop("row_ids", None)
    return projected
