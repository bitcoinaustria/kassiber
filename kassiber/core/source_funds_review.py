"""Target-scoped provenance investigation and user-input handoffs.

The source-funds report owns graph traversal and disclosure gates. This module
binds that report recipe to the reachable authored records and evidence so an
agent can resume after user input without treating provenance as tax authority.
"""
from __future__ import annotations

from contextlib import contextmanager
import hashlib
import json
import re
from typing import Any, Mapping

from ..errors import AppError
from ..msat import msat_to_btc
from . import source_funds

MAX_CONTEXT_RECORDS = 200
RECIPE_KEYS = frozenset({
    "target_amount", "report_purpose", "planned_destination", "planned_note",
    "reveal_mode", "max_depth", "recipient", "report_options",
})


def _error(message: str, code: str = "validation") -> AppError:
    return AppError(message, code=code, retryable=code == "source_funds_review_stale")


def _digest(value: Any) -> str:
    return hashlib.sha256(json.dumps(
        value, sort_keys=True, separators=(",", ":"), allow_nan=False,
    ).encode()).hexdigest()


def _require_fingerprint(value: Any) -> None:
    if not isinstance(value, str) or re.fullmatch(r"[a-f0-9]{64}", value) is None:
        raise _error("A source-funds review fingerprint is required")


@contextmanager
def _read_snapshot(conn):
    owns_transaction = not conn.in_transaction
    if owns_transaction:
        conn.execute("BEGIN")
    try:
        yield
    finally:
        if owns_transaction:
            conn.rollback()


def _recipe_options(recipe: Mapping[str, Any] | None) -> dict[str, Any]:
    if recipe is None:
        return {}
    if not isinstance(recipe, dict) or set(recipe) - RECIPE_KEYS:
        raise _error("Unsupported source-funds review recipe")
    options = dict(recipe)
    for key in RECIPE_KEYS - {"max_depth", "report_options"}:
        if key in options and options[key] is not None and not isinstance(options[key], str):
            raise _error(f"Source-funds recipe {key} must be text")
    if "max_depth" in options and (
        type(options["max_depth"]) is not int or not 1 <= options["max_depth"] <= 32
    ):
        raise _error("Source-funds review depth must be between 1 and 32")
    if "report_options" in options and not isinstance(options["report_options"], dict):
        raise _error("Source-funds report options must be an object")
    if len(json.dumps(options)) > 16_384:
        raise _error("Source-funds review recipe is too large")
    return options


def review_context(conn, profile, hooks, *, target_transaction: str, recipe=None) -> dict[str, Any]:
    """Read the canonical report and its reachable editor records consistently."""
    options = _recipe_options(recipe)
    if not isinstance(target_transaction, str) or not target_transaction.strip():
        raise _error("A source-funds target transaction is required")
    with _read_snapshot(conn):
        current = conn.execute(
            "SELECT * FROM profiles WHERE id=? AND workspace_id=?",
            (profile["id"], profile["workspace_id"]),
        ).fetchone()
        if current is None:
            raise _error("Source-funds book was not found", "not_found")
        target = hooks.resolve_transaction(conn, current["id"], target_transaction)
        if target["excluded"]:
            raise _error("Excluded transactions cannot anchor a source-funds investigation")
        report_options = dict(options)
        if "recipient" in report_options:
            report_options["recipient_ref"] = report_options.pop("recipient")
        report = source_funds.build_report(
            conn, current["workspace_id"], current["id"], hooks,
            target_transaction_ref=target["id"], **report_options,
        )
        canonical_recipe = {
            "target_amount": format(msat_to_btc(report["allocations"]["target_amount_msat"]), "f"),
            "report_purpose": report["purpose"]["type"],
            "planned_destination": report["purpose"]["planned_destination"],
            "planned_note": report["purpose"]["planned_note"],
            "reveal_mode": report["reveal_mode"],
            "max_depth": options.get("max_depth", 8),
            "recipient": (report.get("recipient") or {}).get("id"),
            "report_options": report["report_options"],
        }
        transaction_ids = sorted({target["id"], *(
            node["transaction_id"] for node in report["graph"]["nodes"]
            if node["node_type"] == "transaction"
        )})
        marks = ",".join("?" for _ in transaction_ids)
        link_rows = conn.execute(
            f"SELECT * FROM source_funds_links WHERE profile_id=? "
            f"AND to_transaction_id IN ({marks}) ORDER BY id LIMIT ?",
            (current["id"], *transaction_ids, MAX_CONTEXT_RECORDS + 1),
        ).fetchall()
        truncated = len(link_rows) > MAX_CONTEXT_RECORDS
        link_rows = link_rows[:MAX_CONTEXT_RECORDS]
        links = [source_funds._link_row_to_dict(conn, row) for row in link_rows]
        source_ids = sorted({row["from_source_id"] for row in link_rows if row["from_source_id"]})
        sources = []
        for source_id in source_ids:
            row = conn.execute(
                "SELECT * FROM source_funds_sources WHERE profile_id=? AND id=?",
                (current["id"], source_id),
            ).fetchone()
            if row is not None:
                sources.append(source_funds._source_row_to_dict(conn, row))
        attachment_ids = sorted({
            item["id"] for record in (*links, *sources) for item in record["attachments"]
        })
        evidence_where = f"transaction_id IN ({marks})"
        evidence_args = [current["id"], *transaction_ids]
        if attachment_ids:
            evidence_where += " OR id IN (" + ",".join("?" for _ in attachment_ids) + ")"
            evidence_args.extend(attachment_ids)
        evidence = [dict(row) for row in conn.execute(
            "SELECT id,transaction_id,attachment_type,label,original_filename,media_type,"
            "size_bytes,sha256,created_at FROM attachments WHERE profile_id=? AND ("
            + evidence_where + ") ORDER BY id LIMIT ?",
            (*evidence_args, MAX_CONTEXT_RECORDS + 1),
        ).fetchall()]
        truncated = truncated or len(evidence) > MAX_CONTEXT_RECORDS
        evidence = evidence[:MAX_CONTEXT_RECORDS]
        needs = {item["code"] for item in report["findings"]}
        if any(source["source_type"] in source_funds.ATTESTATION_SOURCE_TYPES for source in sources):
            needs.add("documentary_origin_attested")
        if any(not source["attachments"] for source in sources) or not evidence:
            needs.add("documentary_evidence_missing")
        packet = {
            "schema_version": 1, "domain": "source_funds",
            "workspace_id": current["workspace_id"], "profile_id": current["id"],
            "input_version": int(current["journal_input_version"] or 0),
            "case_id": "source_funds:" + target["id"],
            "target": {"transaction_id": target["id"], **{
                key: target[key] for key in ("wallet_id", "direction", "asset", "occurred_at")
            }},
            "recipe": canonical_recipe, "report": report,
            "links": links, "sources": sources, "evidence": evidence,
            "scope_truncated": truncated,
            "input_needs": sorted(needs),
        }
        # Report generation adds no clock timestamp. Authored update timestamps,
        # attachment hashes, disclosure options and journal version all bind the
        # inspection even when a provenance edit does not invalidate journals.
        packet["review_fingerprint"] = _digest(packet)
        return packet


def request_input(
    conn, profile, hooks, *, action, target_transaction, expected_review_fingerprint,
    recipe=None, explanation=None,
) -> dict[str, Any]:
    if action not in ("connect_wallet", "import_history", "attach_evidence"):
        raise _error("Unsupported source-funds input action")
    _require_fingerprint(expected_review_fingerprint)
    if explanation is not None and (
        not isinstance(explanation, str) or not 1 <= len(explanation.strip()) <= 1000
        or any(ord(char) < 32 and char not in "\n\t" for char in explanation)
    ):
        raise _error("Source-funds input explanation must be bounded plain text")
    context = review_context(conn, profile, hooks, target_transaction=target_transaction, recipe=recipe)
    if context["review_fingerprint"] != expected_review_fingerprint:
        raise _error("Source-funds evidence changed; inspect the target again", "source_funds_review_stale")
    if context["scope_truncated"]:
        raise _error("Source-funds review scope is truncated; narrow the investigation")
    if not context["input_needs"]:
        raise _error("This source-funds investigation has no missing documentary input")
    packet = {
        key: context[key] for key in (
            "schema_version", "domain", "workspace_id", "profile_id", "input_version",
            "review_fingerprint", "recipe",
        )
    }
    packet.update(
        action=action, target_transaction=context["target"]["transaction_id"],
        cases=[{**context["target"], "case_id": context["case_id"], "reason": "evidence_review"}],
        explanation=explanation.strip() if explanation is not None else None,
    )
    return {**packet, "request_id": _digest(packet)}


def save_reviewed_case(
    conn, profile, hooks, *, target_transaction, expected_review_fingerprint,
    recipe=None, case_label=None,
) -> dict[str, Any]:
    """Save the inspected report only while its provenance inputs still match.

    The existing snapshot writer commits the case. Hold the SQLite writer lock
    from revalidation through that commit, rather than checking an old report
    and then rebuilding against potentially changed evidence.
    """
    _require_fingerprint(expected_review_fingerprint)
    if conn.in_transaction:
        raise _error("A reviewed source-funds save requires an idle database transaction")
    conn.execute("BEGIN IMMEDIATE")
    try:
        context = review_context(conn, profile, hooks, target_transaction=target_transaction, recipe=recipe)
        if context["review_fingerprint"] != expected_review_fingerprint:
            raise _error("Source-funds evidence changed; inspect before saving", "source_funds_review_stale")
        if context["scope_truncated"]:
            raise _error("Source-funds review scope is truncated; narrow the investigation before saving")
        options = dict(context["recipe"])
        options["recipient_ref"] = options.pop("recipient")
        return source_funds.build_report(
            conn, context["workspace_id"], context["profile_id"], hooks,
            target_transaction_ref=context["target"]["transaction_id"],
            **options, save_case=True, case_label=case_label, include_diagrams=True,
        )
    except BaseException:
        conn.rollback()
        raise
