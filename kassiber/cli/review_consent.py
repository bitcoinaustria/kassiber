"""Terminal approval of the daemon's recomputed custody-review proposal."""
from __future__ import annotations

import json
import re
from typing import Any, TextIO

TOOL_NAME = "ui.review.apply"


def _count(value: Any) -> bool:
    return type(value) is int and value >= 0


def _effects(value: Any) -> bool:
    return (
        isinstance(value, dict)
        and _count(value.get("entries_count"))
        and _count(value.get("quarantine_count"))
        and type(value.get("report_ready")) is bool
        and isinstance(value.get("quarantines"), list)
        and isinstance(value.get("wallet_holdings"), list)
        and isinstance(value.get("journal_digest"), str)
        and re.fullmatch(r"[a-f0-9]{64}", value["journal_digest"]) is not None
    )


def _validated_preview(data: dict[str, Any]) -> dict[str, Any] | None:
    # Only the separate daemon event field is an approval surface. A field
    # nested in model-authored arguments (even one named review_preview) is not.
    preview, arguments = data.get("review_preview"), data.get("arguments_preview")
    if not isinstance(preview, dict) or not isinstance(arguments, dict):
        return None
    proposed = arguments.get("artifact")
    if (not isinstance(proposed, dict)
            or not all(isinstance(proposed.get(key), str) and proposed[key]
                       for key in ("workspace_id", "profile_id", "digest"))
            or re.fullmatch(r"[a-f0-9]{64}", proposed["digest"]) is None
            or not isinstance(arguments.get("idempotency_key"), str)
            or not arguments["idempotency_key"]):
        return None
    if preview.get("status") == "ready":
        artifact = preview.get("artifact")
        if not isinstance(artifact, dict) or artifact != proposed:
            return None
        operations = artifact.get("operations")
        valid = (
            type(artifact.get("schema_version")) is int and artifact["schema_version"] == 1
            and _count(artifact.get("base_input_version"))
            and isinstance(operations, list) and 1 <= len(operations) <= 50
            and all(isinstance(op, dict) and op.get("type") in
                    ("price_override", "exclude", "custody_component") for op in operations)
            and _effects(artifact.get("before")) and _effects(artifact.get("after"))
        )
    elif preview.get("status") == "applied":
        receipt = preview.get("receipt")
        valid = (
            isinstance(receipt, dict) and type(receipt.get("schema_version")) is int
            and receipt["schema_version"] == 1
            and receipt.get("status") == "verified"
            and all(isinstance(receipt.get(key), str) and receipt[key]
                    for key in ("id", "created_at", "artifact_digest"))
            and all(receipt.get(key) == proposed.get(key) for key in ("workspace_id", "profile_id"))
            and receipt.get("artifact_digest") == proposed.get("digest")
            and receipt.get("idempotency_key") == arguments.get("idempotency_key")
            and _count(receipt.get("result_input_version"))
            and isinstance(receipt.get("operations"), list)
            and _effects(receipt.get("verification"))
        )
    else:
        valid = False
    return preview if valid else None


def decide(data: dict[str, Any], *, interactive: bool, stdin: TextIO, out: TextIO) -> str:
    """Require a fresh human answer, independently of blanket/session flags."""
    preview = _validated_preview(data)
    if not interactive or preview is None:
        out.write("Review denied: interactive, once-only approval of a complete daemon preview is required.\n")
        out.flush()
        return "deny"
    out.write("\nDaemon-validated review (full proposal or historical receipt):\n")
    # JSON escapes terminal control characters in imported descriptions/reasons.
    out.write(json.dumps(preview, indent=2, sort_keys=True) + "\n")
    while True:
        out.write("Allow this exact review? [y] once, [n] deny, [c] cancel: ")
        out.flush()
        choice = stdin.readline().strip().lower()
        if choice in {"y", "yes"}:
            return "allow_once"
        if choice in {"", "n", "no", "deny"}:
            return "deny"
        if choice in {"c", "cancel"}:
            return "cancel"
