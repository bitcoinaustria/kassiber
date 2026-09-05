"""CLI composition for target-scoped provenance investigation."""
from __future__ import annotations

from typing import Any

from ..core import source_funds_review
from .review import _read_json_file


def add_source_funds_review_parsers(commands: Any) -> None:
    context = commands.add_parser("review-context", help="Inspect one provenance target and its reachable evidence")
    request = commands.add_parser("request-input", help="Describe missing user input for an inspected provenance target")
    for parser in (context, request):
        parser.add_argument("--workspace")
        parser.add_argument("--profile")
        parser.add_argument("--target-transaction", required=True)
        parser.add_argument("--recipe-file", help="JSON report recipe, or an earlier review-context envelope")
    request.add_argument("--action", required=True, choices=("connect_wallet", "import_history", "attach_evidence"))
    request.add_argument("--expected-review-fingerprint", required=True)
    request.add_argument("--explanation")


def dispatch_source_funds_review(conn: Any, args: Any, hooks: Any) -> dict[str, Any]:
    _, profile = hooks.resolve_scope(conn, args.workspace, args.profile)
    recipe = _read_json_file(args.recipe_file) if args.recipe_file else {}
    if isinstance(recipe, dict):
        if isinstance(recipe.get("data"), dict):
            recipe = recipe["data"]
        if "recipe" in recipe:
            recipe = recipe["recipe"]
    options = {"target_transaction": args.target_transaction, "recipe": recipe}
    if args.source_funds_command == "request-input":
        return source_funds_review.request_input(
            conn, profile, hooks, **options, action=args.action,
            expected_review_fingerprint=args.expected_review_fingerprint,
            explanation=args.explanation,
        )
    return source_funds_review.review_context(conn, profile, hooks, **options)
