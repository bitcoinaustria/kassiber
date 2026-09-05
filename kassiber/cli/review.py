"""Portable accounting review commands over the shared review workflow."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

from ..errors import AppError
from .handlers import _metadata_hooks, resolve_scope

_MAX_REVIEW_FILE_BYTES = 1_000_000


def add_review_parser(subparsers: Any) -> None:
    review = subparsers.add_parser("review", help="Inspect, preview, apply, and verify accounting repairs")
    commands = review.add_subparsers(dest="review_command", required=True)
    cases = commands.add_parser("cases", help="Read a version-bound page of unresolved cases")
    cases.add_argument("--limit", type=int, default=20)
    cases.add_argument("--cursor", help="next_cursor returned by the preceding page")
    plan = commands.add_parser("plan", help="Preview typed operations without changing the book")
    plan.add_argument("--operations-file", required=True, help="JSON file containing an operations array")
    plan.add_argument("--expected-input-version", required=True, type=int)
    apply = commands.add_parser("apply", help="Apply a reviewed portable artifact and return its verification receipt")
    apply.add_argument("--artifact-file", required=True, help="Artifact JSON or the review.plan JSON envelope")
    apply.add_argument("--idempotency-key", required=True, help="Reuse this key when retrying the same application")
    receipt = commands.add_parser("receipt", help="Read a durable application and verification receipt")
    selector = receipt.add_mutually_exclusive_group(required=True)
    selector.add_argument("--receipt-id")
    selector.add_argument("--idempotency-key")
    for parser in (cases, plan, apply, receipt):
        parser.add_argument("--workspace")
        parser.add_argument("--profile")


def _read_json_file(filename: str) -> Any:
    path = Path(filename).expanduser()
    try:
        with path.open("rb") as source:
            payload = source.read(_MAX_REVIEW_FILE_BYTES + 1)
    except OSError as exc:
        raise AppError("Unable to read the review input file", code="validation") from exc
    if len(payload) > _MAX_REVIEW_FILE_BYTES:
        raise AppError("Review input file is too large", code="validation",
                       details={"maximum_bytes": _MAX_REVIEW_FILE_BYTES})
    try:
        return json.loads(payload)
    except (ValueError, UnicodeDecodeError) as exc:
        raise AppError("Review input file must contain valid JSON", code="validation") from exc


def dispatch_review(conn: Any, args: argparse.Namespace) -> dict[str, Any]:
    from ..core import review_workflow

    _workspace, profile = resolve_scope(conn, args.workspace, args.profile)
    hooks = review_workflow.ReviewHooks(metadata=_metadata_hooks())
    command = args.review_command
    if command == "cases":
        return review_workflow.inspect_cases(conn, profile, limit=args.limit, cursor=args.cursor)
    if command == "plan":
        operations = _read_json_file(args.operations_file)
        if isinstance(operations, dict):
            operations = operations.get("operations")
        if not isinstance(operations, list):
            raise AppError("Review operations file must contain an array or an operations object", code="validation")
        return review_workflow.plan_review(conn, profile, operations=operations,
                                          expected_input_version=args.expected_input_version, hooks=hooks)
    if command == "apply":
        artifact = _read_json_file(args.artifact_file)
        if isinstance(artifact, dict) and artifact.get("kind") in {"review.plan", "ui.review.plan"}:
            artifact = artifact.get("data")
        if not isinstance(artifact, dict):
            raise AppError("Review artifact file must contain a plan object", code="validation")
        return review_workflow.apply_review(conn, profile, artifact=artifact,
                                           idempotency_key=args.idempotency_key, hooks=hooks,
                                           authored_source="cli")
    if command == "receipt":
        return review_workflow.get_receipt(conn, profile, receipt_id=args.receipt_id,
                                          idempotency_key=args.idempotency_key)
    raise AppError("Unsupported review command", code="validation")
