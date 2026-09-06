"""Explicit local investigation operations shared by desktop, CLI and agents."""
from __future__ import annotations

from . import chain_analysis as engine
from . import chain_analysis_cases as storage
from . import chain_analysis_acquisition as acquisition
from .repo.context import resolve_scope


READ_OPERATIONS = frozenset({"query", "entropy", "ai_context", "cases.list", "cases.get", "cases.compare", "labels.list", "acquire.plan"})
WRITE_OPERATIONS = frozenset({"cases.save", "cases.delete", "labels.upsert", "labels.delete", "labels.import", "acquire.apply"})
READ_KINDS = frozenset(f"ui.chain_analysis.{name}" for name in READ_OPERATIONS)
WRITE_KINDS = frozenset(f"ui.chain_analysis.{name}" for name in WRITE_OPERATIONS)
KINDS = READ_KINDS | WRITE_KINDS


def dispatch(conn, kind, args=None, *, workspace=None, profile=None):
    if kind not in KINDS:
        storage.invalid("Unsupported chain analysis operation")
    if args is None:
        args = {}
    if not isinstance(args, dict):
        storage.invalid("Chain analysis arguments must be an object")
    args = dict(args)
    expected_scope = args.pop("expected_scope", None)
    current_workspace, current_profile = resolve_scope(conn, workspace, profile)
    if expected_scope is not None and expected_scope != {"workspace_id": current_workspace["id"], "profile_id": current_profile["id"]}:
        storage.invalid("The active book changed; reopen the investigation in its original book", "scope_changed")
    profile_id = current_profile["id"]
    operation = kind.removeprefix("ui.chain_analysis.")
    if operation == "ai_context":
        storage.arguments(args, ("query", "subject", "expected_snapshot_id"), ("query", "expected_snapshot_id"))
        from .chain_analysis_ai import project_ai_result
        from .chain_analysis.query import resolve_subject
        with storage.atomic(conn):
            result = engine.run_analysis(conn, profile_id, args["query"])
            if result["snapshot_id"] != args["expected_snapshot_id"]:
                storage.invalid("Investigation changed; refresh before opening assistant context", "chain_analysis_stale")
            context = {"query": result["query"], "snapshot_id": result["snapshot_id"]}
            if args.get("subject"):
                storage.text_value(args["subject"], "subject", 1024)
                resolve_subject(engine.build_index(conn, profile_id), args["subject"], result["query"])
                context["subject"] = args["subject"]
            return project_ai_result(conn, profile_id, context)
    if operation == "query":
        return engine.run_analysis(conn, profile_id, args)
    if operation == "entropy":
        return engine.run_entropy(conn, profile_id, args)
    if operation == "cases.list":
        return storage.list_cases(conn, profile_id, args)
    if operation == "cases.get":
        storage.arguments(args, ("id",), ("id",))
        return storage.get_case(conn, profile_id, args["id"])
    if operation == "cases.compare":
        return storage.compare_case(conn, profile_id, args)
    if operation == "labels.list":
        storage.arguments(args, ())
        return storage.list_labels(conn, profile_id)
    if operation == "acquire.plan":
        return acquisition.plan_acquisition(conn, profile_id, args)
    if operation == "cases.save":
        result = storage.save_case(conn, profile_id, args)
    elif operation == "cases.delete":
        storage.arguments(args, ("id",), ("id",))
        result = storage.delete_case(conn, profile_id, args["id"])
    elif operation == "labels.upsert":
        result = storage.upsert_label(conn, profile_id, args)
    elif operation == "labels.delete":
        result = storage.delete_label(conn, profile_id, args)
    elif operation == "labels.import":
        result = storage.import_labels(conn, profile_id, args)
    else:
        result = acquisition.apply_acquisition(conn, profile_id, args)
    conn.commit()
    return result
