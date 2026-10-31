"""Plan and apply a generic-ledger import of a staged file, bound to its plan.

This is the write half of the file-onboarding flow whose read half is
`core.file_analysis`. A caller that cannot be trusted with a path or with the
book (the in-app assistant) proposes a target wallet and a column plan; this
module answers with the exact effect of importing the staged file that way, and
applies it only when the book, the file bytes and the effect still match.

The effect is not predicted by a second parser. Planning runs the desktop/CLI
import path (`core.imports.import_file_into_wallet`, with wallet creation when
asked) inside a savepoint and rolls it back; applying runs the same path in the
same way and commits only if the recomputed plan digest equals the approved
one. So what the user approved is, row for row, what lands — and the run is an
ordinary import batch that `imports rollback` can undo.

Hard rules:

- One snapshot of the file's bytes is hashed and parsed, so the digest covers
  the bytes that are imported, not a later version of the file.
- Only file-backed wallets (`exchange-export`, `file-import`, `manual`) or a new
  `custom` wallet may be targeted. Chain-scanned wallets stay authoritative for
  their own history. Descriptor/xpub wallets are never created here.
- Anything ambiguous blocks: unrecognized columns, rejected rows, amounts whose
  unit the file does not state, row kinds taken from the amount sign without the
  user's confirmation, and an import with no new row (there would be no run to
  roll back). Re-pricing rows the wallet already holds is reported as a
  warning, because a rollback leaves it in place.
"""

from __future__ import annotations

import contextlib
import hashlib
import json
import os
import shutil
import sqlite3
import tempfile
from typing import Any, Iterator, Mapping

from .. import importers as importers_module
from ..errors import AppError
from ..time_utils import UNKNOWN_OCCURRED_AT
from . import file_analysis as core_file_analysis
from . import imports as core_imports
from . import wallet_completeness as core_wallet_completeness
from . import wallets as core_wallets
from .repo import resolve_wallet

PLAN_SCHEMA_VERSION = 1
SOURCE_FORMAT = "generic_ledger"
NEW_WALLET_KIND = "custom"
MAX_WALLET_LABEL_CHARS = 120
_SAVEPOINT = "attachment_import_plan"


@contextlib.contextmanager
def _file_snapshot(source_file: str) -> Iterator[tuple[str, str, int]]:
    """Copy the staged file's bytes once; yield (path, sha256, size).

    The copy keeps the original basename so the recorded import run names the
    file the user chose, and it is parsed instead of the original so a file
    rewritten between hashing and parsing cannot slip past the digest.
    """

    try:
        with open(source_file, "rb") as handle:
            payload = handle.read(core_file_analysis.MAX_ANALYZE_BYTES + 1)
    except OSError as exc:
        raise AppError(
            "The attached file can no longer be read",
            code="ai_attachment_missing",
            hint="Ask the user to attach the file again.",
            retryable=False,
        ) from exc
    if len(payload) > core_file_analysis.MAX_ANALYZE_BYTES:
        raise AppError(
            "The attached file is too large to import",
            code="validation",
            details={"max_bytes": core_file_analysis.MAX_ANALYZE_BYTES},
            retryable=False,
        )
    digest = hashlib.sha256(payload).hexdigest()
    directory = tempfile.mkdtemp(prefix="kassiber-import-")
    try:
        path = os.path.join(directory, os.path.basename(source_file) or "import.csv")
        with open(path, "wb") as handle:
            handle.write(payload)
        yield path, digest, len(payload)
    finally:
        shutil.rmtree(directory, ignore_errors=True)


def _canonical(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), default=str)


def _digest(value: Any) -> str:
    return hashlib.sha256(_canonical(value).encode("utf-8")).hexdigest()


def input_version(conn: sqlite3.Connection, profile_id: str) -> int:
    row = conn.execute(
        "SELECT journal_input_version FROM profiles WHERE id = ?", (profile_id,)
    ).fetchone()
    return int((row["journal_input_version"] if row else 0) or 0)


def _blocker(code: str, message: str, **remediation: Any) -> dict[str, Any]:
    blocker: dict[str, Any] = {"code": code, "message": message}
    if remediation:
        blocker["remediation"] = remediation
    return blocker


def _normalize_target(request: Mapping[str, Any]) -> dict[str, Any]:
    wallet_ref = request.get("wallet")
    new_wallet = request.get("new_wallet")
    if (wallet_ref is None) == (new_wallet is None):
        raise AppError(
            "Name exactly one target: an existing wallet or a new wallet",
            code="validation",
            hint="Pass `wallet` (id or label) or `new_wallet` with a label.",
            retryable=False,
        )
    if wallet_ref is not None:
        if not isinstance(wallet_ref, str) or not wallet_ref.strip():
            raise AppError("wallet must be a non-empty string", code="validation", retryable=False)
        return {"action": "use_existing", "wallet": wallet_ref.strip()}
    if not isinstance(new_wallet, Mapping):
        raise AppError("new_wallet must be an object", code="validation", retryable=False)
    label = new_wallet.get("label")
    if not isinstance(label, str) or not label.strip() or len(label.strip()) > MAX_WALLET_LABEL_CHARS:
        raise AppError(
            f"new_wallet.label must be 1 to {MAX_WALLET_LABEL_CHARS} characters",
            code="validation",
            retryable=False,
        )
    account = new_wallet.get("account")
    if account is not None and (not isinstance(account, str) or not account.strip()):
        raise AppError("new_wallet.account must be a non-empty string", code="validation", retryable=False)
    return {
        "action": "create",
        "label": label.strip(),
        "account": account.strip() if isinstance(account, str) else None,
    }


def _analysis_blockers(
    analysis: Mapping[str, Any], *, transfers_only_confirmed: bool
) -> tuple[list[dict[str, Any]], list[str]]:
    blockers: list[dict[str, Any]] = []
    warnings: list[str] = []
    if analysis.get("route") != "generic_ledger":
        blockers.append(
            _blocker(
                "attachment_not_tabular",
                "Photos and PDFs are read by the on-device document importer, not this tool.",
                action="document_import",
                desktop_only=True,
            )
        )
        return blockers, warnings
    if analysis.get("header_is_preamble"):
        blockers.append(
            _blocker(
                "header_is_preamble",
                "The first row is a preamble, not column names; the user must remove it.",
                action="strip_preamble",
                desktop_only=True,
            )
        )
    elif not analysis.get("confident"):
        blockers.append(
            _blocker(
                "columns_not_recognized",
                "The columns were not recognized. Map them with column_map first.",
                action="supply_column_map",
                tool="ui_wallets_analyze_file",
            )
        )
    if int(analysis.get("errors") or 0):
        blockers.append(
            _blocker(
                "rows_rejected",
                f"{int(analysis['errors'])} row(s) would be rejected; the import stops at the first one.",
                action="fix_mapping",
                tool="ui_wallets_analyze_file",
            )
        )
    if analysis.get("amount_units_unconfirmed"):
        blockers.append(
            _blocker(
                "amount_units_unconfirmed",
                "The file does not state the amount unit and the amounts are too large "
                "to be BTC. The user must set the unit in the desktop or CLI import.",
                action="confirm_amount_units",
                desktop_only=True,
            )
        )
    if analysis.get("row_kinds_from_amount_sign"):
        if transfers_only_confirmed:
            warnings.append("transfers_only_confirmed")
        else:
            blockers.append(
                _blocker(
                    "row_kinds_from_amount_sign",
                    "No Type or direction column: every row would be a plain deposit or "
                    "withdrawal. Map the Type column, or set transfers_only_confirmed after "
                    "the user confirms the export is transfers only.",
                    action="map_row_kinds",
                    tool="ui_wallets_analyze_file",
                )
            )
    return blockers, warnings


def _resolve_target(
    conn: sqlite3.Connection, profile: Mapping[str, Any], target: Mapping[str, Any]
) -> tuple[dict[str, Any], list[dict[str, Any]], Mapping[str, Any] | None]:
    """Describe the target and whether it may receive a ledger import."""

    if target["action"] == "use_existing":
        wallet = resolve_wallet(conn, profile["id"], target["wallet"])
        klass = core_wallet_completeness.wallet_completeness_class(conn, str(wallet["id"]))
        described = {
            "action": "use_existing",
            "wallet_id": str(wallet["id"]),
            "label": wallet["label"],
            "kind": wallet["kind"],
            "completeness_class": klass,
        }
        blockers = []
        if klass not in core_wallet_completeness.FILE_IMPORT_TARGET_CLASSES:
            blockers.append(
                _blocker(
                    "target_not_file_wallet",
                    f"'{wallet['label']}' is a {klass} wallet; its history comes from its "
                    "own source. Import the file into a file or exchange wallet instead.",
                    action="choose_file_wallet",
                    tool="ui_wallets_import_attachment_plan",
                )
            )
        return described, blockers, wallet
    described = {
        "action": "create",
        "wallet_id": None,
        "label": target["label"],
        "kind": NEW_WALLET_KIND,
        "account": target.get("account"),
        "completeness_class": core_wallet_completeness.completeness_class(
            NEW_WALLET_KIND, {"source_format": SOURCE_FORMAT}
        ),
    }
    taken = conn.execute(
        "SELECT 1 FROM wallets WHERE profile_id = ? AND label = ?",
        (profile["id"], target["label"]),
    ).fetchone()
    blockers = []
    if taken:
        blockers.append(
            _blocker(
                "wallet_label_taken",
                f"A wallet named '{target['label']}' already exists; target it or pick another label.",
                action="choose_target",
                tool="ui_wallets_import_attachment_plan",
            )
        )
    return described, blockers, None


def _batch_effects(
    conn: sqlite3.Connection,
    profile_id: str,
    wallet_id: str,
    batch_id: str | None,
) -> dict[str, Any]:
    if not batch_id:
        return {
            "date_range": {"first": None, "last": None, "undated": 0},
            "totals": [],
            "overlap": {"existing_rows_in_date_range": 0, "external_ids_in_other_wallets": 0},
        }
    batch_rows = (
        "SELECT t.* FROM transactions t JOIN import_batch_transactions b "
        "ON b.transaction_id = t.id WHERE b.batch_id = ?"
    )
    bounds = conn.execute(
        f"""
        SELECT MIN(occurred_at) AS first, MAX(occurred_at) AS last
        FROM ({batch_rows}) WHERE occurred_at != ?
        """,
        (batch_id, UNKNOWN_OCCURRED_AT),
    ).fetchone()
    undated = conn.execute(
        f"SELECT COUNT(*) AS n FROM ({batch_rows}) WHERE occurred_at = ?",
        (batch_id, UNKNOWN_OCCURRED_AT),
    ).fetchone()["n"]
    totals = [
        {
            "asset": row["asset"],
            "direction": row["direction"],
            "rows": int(row["n"]),
            "amount_msat": int(row["amount"] or 0),
            "fee_msat": int(row["fee"] or 0),
        }
        for row in conn.execute(
            f"""
            SELECT asset, direction, COUNT(*) AS n, SUM(amount) AS amount, SUM(fee) AS fee
            FROM ({batch_rows}) GROUP BY asset, direction ORDER BY asset, direction
            """,
            (batch_id,),
        )
    ]
    in_range = 0
    if bounds["first"] is not None:
        in_range = conn.execute(
            """
            SELECT COUNT(*) AS n FROM transactions
            WHERE profile_id = ? AND wallet_id = ? AND excluded = 0
              AND occurred_at BETWEEN ? AND ?
              AND id NOT IN (SELECT transaction_id FROM import_batch_transactions WHERE batch_id = ?)
            """,
            (profile_id, wallet_id, bounds["first"], bounds["last"], batch_id),
        ).fetchone()["n"]
    elsewhere = conn.execute(
        f"""
        SELECT COUNT(DISTINCT mine.external_id) AS n
        FROM ({batch_rows}) mine
        JOIN transactions other
          ON other.profile_id = ? AND other.external_id = mine.external_id
         AND other.wallet_id != mine.wallet_id
        WHERE mine.external_id IS NOT NULL AND mine.external_id != ''
        """,
        (batch_id, profile_id),
    ).fetchone()["n"]
    return {
        "date_range": {"first": bounds["first"], "last": bounds["last"], "undated": int(undated)},
        "totals": totals,
        "overlap": {
            "existing_rows_in_date_range": int(in_range),
            "external_ids_in_other_wallets": int(elsewhere),
        },
    }


def plan_or_apply(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    *,
    source_file: str,
    request: Mapping[str, Any],
    hooks: core_imports.ImportCoordinatorHooks,
    expected: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    """Plan an import of ``source_file`` (``expected`` None) or apply one.

    ``request`` carries the target (``wallet`` or ``new_wallet``), the optional
    ``column_map`` and ``transfers_only_confirmed``. ``expected`` carries
    ``plan_digest``, ``expected_content_sha256`` and ``expected_input_version``
    from an earlier plan; applying refuses unless all three still hold.
    """

    target = _normalize_target(request)
    column_map = request.get("column_map")
    if column_map is not None and not isinstance(column_map, Mapping):
        raise AppError("column_map must be an object", code="validation", retryable=False)
    transfers_only_confirmed = request.get("transfers_only_confirmed", False) is True
    version = input_version(conn, str(profile["id"]))

    with _file_snapshot(source_file) as (path, content_sha256, size_bytes):
        if expected is not None:
            if expected.get("expected_content_sha256") != content_sha256:
                raise AppError(
                    "The attached file changed since the import was planned",
                    code="attachment_changed",
                    hint="Plan the import again from the current file and ask for consent again.",
                    details={"remediation": {"action": "replan", "tool": "ui_wallets_import_attachment_plan", "target": {}, "desktop_only": False}},
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
                        "remediation": {"action": "replan", "tool": "ui_wallets_import_attachment_plan", "target": {}, "desktop_only": False},
                    },
                    retryable=False,
                )
        analysis = core_file_analysis.analyze_file(path, column_map=column_map, sample_rows=0)
        blockers, warnings = _analysis_blockers(
            analysis, transfers_only_confirmed=transfers_only_confirmed
        )
        described_target, target_blockers, wallet = _resolve_target(conn, profile, target)
        blockers.extend(target_blockers)
        if isinstance(column_map, Mapping) and column_map.get("type_map"):
            warnings.append("type_map_decides_tax_kind")

        rows = {"in_file": int(analysis.get("rows_read") or 0), "to_import": 0,
                "already_present": 0, "would_update_existing": 0}
        effects = _batch_effects(conn, str(profile["id"]), "", None)
        wallet_after: dict[str, Any] | None = None
        simulate = not any(
            blocker["code"] in {"attachment_not_tabular", "header_is_preamble",
                                "columns_not_recognized", "rows_rejected",
                                "target_not_file_wallet", "wallet_label_taken"}
            for blocker in blockers
        )
        apply_now = False
        conn.execute(f"SAVEPOINT {_SAVEPOINT}")
        try:
            outcome: dict[str, Any] = {}
            if simulate:
                try:
                    if wallet is None:
                        wallet = core_wallets.create_wallet(
                            conn, None, None, described_target["label"], NEW_WALLET_KIND,
                            account_ref=described_target.get("account"),
                            config={"source_format": SOURCE_FORMAT},
                            commit=False,
                        )
                    outcome = core_imports.import_file_into_wallet(
                        conn, profile, wallet, path, SOURCE_FORMAT, hooks,
                        commit=False, column_map=column_map,
                    )
                except AppError as exc:
                    blockers.append(_blocker(exc.code or "import_failed", str(exc)))
                    outcome = {}
            if outcome:
                updated = len(outcome.get("updated_records") or [])
                rows.update(
                    to_import=int(outcome.get("imported") or 0),
                    already_present=int(outcome.get("unchanged") or 0),
                    would_update_existing=updated,
                )
                wallet_id = str(wallet["id"])
                effects = _batch_effects(
                    conn, str(profile["id"]), wallet_id, outcome.get("import_batch_id")
                )
                after = core_wallet_completeness.describe_wallets(
                    conn, str(profile["id"]),
                    [dict(conn.execute(
                        "SELECT id, kind, config_json FROM wallets WHERE id = ?", (wallet_id,)
                    ).fetchone())],
                )[wallet_id]
                wallet_after = after
                if updated:
                    # The importer re-prices a row it already holds from the
                    # file's own execution price. A rollback deletes only rows
                    # the run created, so this part would stay; say so.
                    warnings.append("updates_existing_rows_not_rolled_back")
                if not rows["to_import"]:
                    # Without a new row there is no import run to roll back,
                    # so an enrichment-only import stays a local decision.
                    blockers.append(
                        _blocker(
                            "nothing_to_import",
                            "No new rows: every row is already in this wallet.",
                            action="import_locally" if updated else "none",
                            desktop_only=bool(updated),
                        )
                    )
                if rows["already_present"]:
                    warnings.append("rows_already_present")
                if effects["overlap"]["existing_rows_in_date_range"]:
                    warnings.append("overlaps_existing_history")
                if effects["overlap"]["external_ids_in_other_wallets"]:
                    warnings.append("external_ids_in_other_wallets")
                if "history_starts_after_first_funding" in after["scan_bounds"]["warnings"]:
                    warnings.append("history_starts_after_first_funding")
            plan = {
                "schema_version": PLAN_SCHEMA_VERSION,
                "workspace_id": str(profile["workspace_id"]),
                "profile_id": str(profile["id"]),
                "input_version": version,
                "attachment": {
                    "content_sha256": content_sha256,
                    "size_bytes": size_bytes,
                    "kind": (analysis.get("source") or {}).get("kind"),
                    "filename": os.path.basename(source_file),
                },
                "source_format": SOURCE_FORMAT,
                "column_map": dict(column_map) if isinstance(column_map, Mapping) else None,
                "effective_column_plan": analysis.get("applied_plan") or analysis.get("inferred_plan"),
                "transfers_only_confirmed": transfers_only_confirmed,
                "target": described_target,
                "rows": rows,
                **effects,
                "wallet_after": wallet_after,
                "warnings": warnings,
                "blockers": blockers,
                "ready": not blockers,
            }
            plan_digest = _digest(plan)
            if expected is not None:
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
                        details={"remediation": {"action": "replan", "tool": "ui_wallets_import_attachment_plan", "target": {}, "desktop_only": False}},
                        retryable=False,
                    )
                apply_now = True
        finally:
            if apply_now:
                conn.execute(f"RELEASE SAVEPOINT {_SAVEPOINT}")
            else:
                conn.execute(f"ROLLBACK TO SAVEPOINT {_SAVEPOINT}")
                conn.execute(f"RELEASE SAVEPOINT {_SAVEPOINT}")
    if not apply_now:
        return {
            **plan,
            "plan_digest": plan_digest,
            "apply_with": {
                "plan_digest": plan_digest,
                "expected_content_sha256": content_sha256,
                "expected_input_version": version,
            },
        }
    conn.commit()
    return {
        "applied": True,
        "plan_digest": plan_digest,
        "import_batch_id": outcome.get("import_batch_id"),
        "wallet": {"id": str(wallet["id"]), "label": wallet["label"],
                   "created": described_target["action"] == "create"},
        "rows": rows,
        **effects,
        "warnings": warnings,
        "journals_invalidated": bool(outcome.get("journal_invalidated")),
        "rollback": {"batch": outcome.get("import_batch_id"),
                     "tool": "ui_imports_rollback_plan"},
    }


def withhold_cell_values(payload: Mapping[str, Any]) -> dict[str, Any]:
    """Project a plan/result for a provider whose agent access is restricted.

    Counts, the column plan, warnings and blockers stay; dates, amounts and the
    filename are values from the file and do not cross to the model.
    """

    projected = dict(payload)
    if isinstance(projected.get("attachment"), Mapping):
        projected["attachment"] = {
            key: value for key, value in projected["attachment"].items() if key != "filename"
        }
    if "date_range" in projected:
        projected["date_range"] = None
    if "totals" in projected:
        projected["totals"] = [
            {"asset": row.get("asset"), "direction": row.get("direction"), "rows": row.get("rows")}
            for row in projected.get("totals") or []
        ]
    if isinstance(projected.get("wallet_after"), Mapping):
        projected["wallet_after"] = {
            "completeness_class": projected["wallet_after"].get("completeness_class"),
            "warnings": (projected["wallet_after"].get("scan_bounds") or {}).get("warnings", []),
        }
    projected["cell_values_withheld"] = True
    return projected


# Asset hints declare what a column is denominated in: a value, not a name.
# A chat caller may not set them (see daemon._analyze_attached_file_for_ai).
ASSET_HINT_FIELDS = tuple(importers_module.LEDGER_ASSET_HINT_FIELDS)
