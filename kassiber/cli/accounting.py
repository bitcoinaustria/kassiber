"""First-class local accounting CLI, sharing guarded core with scoped agents."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

from ..core.accounting.commands import ACTIONS, execute
from ..core.repo import resolve_scope
from ..errors import AppError
from ..secrets.prompt import read_passphrase_from_fd


MAX_PAYLOAD_CHARS = 32 * 1024 * 1024
# JSON can escape every snapshot character as six ASCII characters. Preserve
# the export/verifier roundtrip without enlarging the broker's secret frames.
MAX_VERIFY_PAYLOAD_CHARS = 6 * 64 * 1024 * 1024 + 128 * 1024
_ACTION_HELP = {
    "workbench": "Local accounting worklist and readiness; payload: period_id.",
    "task-create": "Select statement_ids (an empty list is allowed), optional evidence_ids/draft_ids, period_id and idempotency_key; no posting. Optional projection:{artifact_id,policy_id,events:[{event_id,binding_id?,category?}]} selects actual Bitcoin sources; prepare/post advances one chronological Bitcoin source at a time. Alternatively explicitly select include_period_statements:true.",
    "task-list": "List local accounting tasks and durable progress; empty payload.",
    "task-get": "Resume a task by task_id; includes source coverage, exceptions and receipts.",
    "task-preview": "Preview task_id + step (prepare, post, close, tax_finalize, export_close, export_tax); returns exact effects, expected_digest and expected_revision.",
    "task-apply": "Apply one reviewed step using task_id, step, expected_digest, expected_revision, idempotency_key and confirmed:true. Exports also require confirm_plaintext:true; retain the full response to save artifact bytes.",
    "task-cancel": "Cancel task_id with a reason; already committed entries remain unchanged.",
    "task-source-assign": "Explicitly assign selected evidence to a bank row or reviewed posting; requires confirmed:true. For kind:projection, use task-projection-assign-preview and supply task_id,event_id,binding_id,category,reason,expected_digest,expected_revision,idempotency_key,confirmed:true.",
    "task-projection-assign-preview": "Locally review a binding/category for an already selected Bitcoin task source: task_id,event_id,binding_id,category,reason. No new source selection or posting; returns expected_digest and expected_revision.",
    "task-amend-preview": "Locally review adding exact retained evidence_ids to task_id + period_id with a reason; returns expected_digest and expected_revision. No source discovery or posting.",
    "task-amend": "Append the reviewed evidence_ids to the same task using task_id, period_id, reason, expected_digest, expected_revision, idempotency_key and confirmed:true. Prior receipts remain; old approvals expire. Local only, not an AI tool.",
    "rule-create": "Approve an exact bank description/account/direction rule for draft preparation only.",
    "rule-list": "List explicit preparation rules; empty payload.",
    "rule-revoke": "Revoke a preparation rule by rule_id and reason; existing postings remain unchanged.",
    "verify-package": "Verify a close package, accounting.export-close envelope, or accounting.task-apply export_close envelope without opening a book.",
}


def add_accounting_parser(sub):
    parser = sub.add_parser("accounting", help="Opt-in double-entry books (separate from Bitcoin tax journals)")
    commands = parser.add_subparsers(dest="accounting_command", required=True)
    for action in sorted(ACTIONS | {"verify-package"}):
        command = commands.add_parser(action, help=_ACTION_HELP.get(action), description=_ACTION_HELP.get(action))
        if action != "verify-package":
            command.add_argument("--workspace", required=True)
            command.add_argument("--profile", required=True)
        inputs = command.add_mutually_exclusive_group()
        inputs.add_argument("--payload", help="JSON object (visible in shell history; prefer --payload-stdin). Amounts use integer minor units.")
        inputs.add_argument("--payload-stdin", action="store_true", help="Read a JSON object from stdin; supported by the operator broker.")
        inputs.add_argument("--payload-fd", type=int, help="Read a JSON object from an inherited fd (up to 8 KiB).")
        inputs.add_argument("--payload-file", help="Read a bounded local JSON file; requires --payload-sha256 to bind approved bytes.")
        command.add_argument("--payload-sha256", help="SHA-256 of the exact --payload-file bytes (required for file input).")


def dispatch_accounting(conn, args: argparse.Namespace):
    limit = MAX_VERIFY_PAYLOAD_CHARS if args.accounting_command == "verify-package" else MAX_PAYLOAD_CHARS
    input_file = getattr(args, "payload_file", None)
    expected_hash = getattr(args, "payload_sha256", None)
    if expected_hash is not None and input_file is None:
        raise AppError("A payload digest requires file input", code="accounting_invalid_fields")
    if input_file is not None:
        if not isinstance(expected_hash, str) or len(expected_hash) != 64 or any(c not in "0123456789abcdef" for c in expected_hash):
            raise AppError("File input requires the approved lowercase SHA-256 digest", code="accounting_payload_digest_required")
        try:
            with Path(input_file).expanduser().open("rb") as source:
                raw = source.read(limit + 1)
            if len(raw) > limit:
                raise AppError("Accounting payload is too large", code="accounting_payload_too_large")
            if hashlib.sha256(raw).hexdigest() != expected_hash:
                raise AppError("Accounting input changed after approval", code="accounting_stale_approval")
            text = raw.decode("utf-8")
        except (OSError, UnicodeError) as exc:
            raise AppError("Could not read the approved UTF-8 accounting input", code="accounting_input_unavailable") from exc
    elif args.payload_fd is not None:
        text = read_passphrase_from_fd(args.payload_fd)
    elif args.payload_stdin:
        text = sys.stdin.read(limit + 1)
    else:
        text = args.payload or "{}"
    if len(text) > limit:
        raise AppError("Accounting payload is too large", code="accounting_payload_too_large")
    try:
        payload = json.loads(text)
    except (ValueError, RecursionError) as exc:
        raise AppError("Accounting payload must be valid JSON", code="accounting_invalid_fields") from exc
    if args.accounting_command == "verify-package":
        from ..core.accounting.package import FORMAT, verify_package

        if isinstance(payload, dict) and payload.get("kind") == "accounting.export-close" and payload.get("schema_version") == 1:
            payload = payload.get("data")
        elif isinstance(payload, dict) and payload.get("kind") == "accounting.task-apply":
            data = payload.get("data")
            receipt = data.get("receipt") if isinstance(data, dict) else None
            result = data.get("result") if isinstance(data, dict) else None
            if (type(payload.get("schema_version")) is not int or payload["schema_version"] != 1
                or not isinstance(receipt, dict) or receipt.get("step") != "export_close"
                or not isinstance(result, dict) or result.get("format") != FORMAT
                or result.get("artifact_state") != "prepared"):
                raise AppError("Expected a task export_close accounting package", code="accounting_package_invalid")
            payload = result
        return {"verification": verify_package(payload)}
    workspace, profile = resolve_scope(conn, args.workspace, args.profile)
    try:
        result = execute(conn, profile["id"], args.accounting_command, payload)
        conn.commit()
    except Exception:
        conn.rollback()
        raise
    return {"workspace_id": workspace["id"], "profile_id": profile["id"], **result}
