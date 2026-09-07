"""Explicit local investigation operations shared by desktop, CLI and agents."""
from __future__ import annotations

from . import chain_analysis as engine
from . import chain_analysis_cases as storage
from . import chain_analysis_acquisition as acquisition
from .repo.context import resolve_scope
from . import chain_analysis_backfill as backfill


READ_OPERATIONS = frozenset({"query", "entropy", "entropy.start", "jobs.get", "jobs.cancel", "psbt.analyze", "psbt.compare", "psbt.entropy", "psbt.entropy.start", "datasets.list", "datasets.get", "datasets.query", "datasets.preview", "datasets.preview.start", "ai_context", "cases.list", "cases.get", "cases.compare", "labels.list", "acquire.plan", "watches.preview", "watches.list", "watches.inbox"})
WRITE_OPERATIONS = frozenset({"cases.save", "cases.delete", "labels.upsert", "labels.delete", "labels.import", "datasets.import", "datasets.import.start", "datasets.revoke", "datasets.discard", "datasets.discard.start", "acquire.apply", "watches.create", "watches.configure", "watches.delete", "watches.evaluate", "watches.acknowledge"})
READ_OPERATIONS |= backfill.READ_OPERATIONS
WRITE_OPERATIONS |= backfill.WRITE_OPERATIONS
READ_KINDS = frozenset(f"ui.chain_analysis.{name}" for name in READ_OPERATIONS)
WRITE_KINDS = frozenset(f"ui.chain_analysis.{name}" for name in WRITE_OPERATIONS)
KINDS = READ_KINDS | WRITE_KINDS


def dispatch(conn, kind, args=None, *, workspace=None, profile=None, source_stream=None, job_starter=None):
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
    if operation.startswith("watches."):
        from .chain_analysis_watches import dispatch as dispatch_watches
        result = dispatch_watches(conn, profile_id, operation.removeprefix("watches."), args)
        if kind in WRITE_KINDS:
            conn.commit()
        return result
    if operation.startswith("sources."):
        result = backfill.dispatch(conn, profile_id, operation, args)
        if operation in backfill.WRITE_OPERATIONS:
            conn.commit()
        return result
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
                from .chain_analysis.projection import read_index
                with read_index(conn, profile_id, observer=result["query"]["observer"]) as index:
                    resolve_subject(index, args["subject"], result["query"])
                context["subject"] = args["subject"]
            return project_ai_result(conn, profile_id, context)
    if operation == "query":
        return engine.run_analysis(conn, profile_id, args)
    if operation == "entropy":
        return engine.run_entropy(conn, profile_id, args)
    if operation == "entropy.start":
        return engine.start_entropy(conn, profile_id, args)
    if operation in {"jobs.get", "jobs.cancel"}:
        storage.arguments(args, ("job_id",), ("job_id",))
        from .chain_analysis_runtime import JOBS, scope_key
        return JOBS.get(scope_key(conn, profile_id), args["job_id"], cancel=operation == "jobs.cancel")
    if operation.startswith("psbt."):
        return _psbt(conn, profile_id, operation, args)
    if operation.startswith("datasets."):
        if operation.endswith(".start"):
            if job_starter is None:
                storage.invalid("Background dataset work requires a running daemon; use the synchronous CLI operation", "daemon_required")
            return job_starter(conn, operation, args, profile_id=profile_id)
        return _datasets(conn, profile_id, operation, args, source_stream)
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


def _psbt(conn, profile_id, operation, args):
    from .chain_analysis.psbt import analyze_psbt, compare_psbts
    from .chain_analysis_runtime import SOURCES, scope_key
    scope = scope_key(conn, profile_id)
    def read(key):
        if key in args and key + "_token" in args:
            storage.invalid("Use either selected source or PSBT text")
        if key + "_token" in args:
            with SOURCES.open(scope, args[key + "_token"], "psbt") as stream:
                return stream.read(SOURCES.LIMITS["psbt"] + 1)
        if key not in args:
            storage.invalid("Select a PSBT source")
        return args[key]
    if operation in {"psbt.analyze", "psbt.entropy", "psbt.entropy.start"}:
        fields = ("psbt", "psbt_token", "network")
        if operation != "psbt.analyze":
            fields += ("max_states", "max_duration_ms", "scenario")
        storage.arguments(args, fields, ("network",))
        result = analyze_psbt(read("psbt"), network=args["network"])
        if operation == "psbt.analyze":
            return result
        from .chain_analysis.entropy import analyze_transaction_entropy, normalize_scenario
        from .chain_analysis.index import digest
        from .chain_analysis_runtime import JOBS
        options = {"scenario": normalize_scenario(args.get("scenario"))}
        for key, default, maximum in (("max_states", 200000, 2000000), ("max_duration_ms", 1000, 30000)):
            value = args.get(key, default)
            if type(value) is not int or not 1 <= value <= maximum:
                storage.invalid(f"{key} must be between 1 and {maximum}")
            options[key] = value
        facts = result["transaction_facts"]
        context = {"schema_version": 1, "subject": "psbt:transaction", "snapshot_id": digest(result), "network": args["network"], "source_kind": "psbt"}
        def compute(progress=None, cancelled=None):
            return {**context, **analyze_transaction_entropy(facts, **options, progress=progress, cancelled=cancelled)}
        if operation == "psbt.entropy.start":
            # No input text or source grant is retained by the computation.
            return JOBS.start(scope, {**context, **options}, compute)
        return compute()
    storage.arguments(args, ("before", "after", "before_token", "after_token", "network", "payjoin"), ("network",))
    options = {"network": args["network"]}
    if "payjoin" in args:
        options["payjoin"] = args["payjoin"]
    return compare_psbts(read("before"), read("after"), **options)


def _datasets(conn, profile_id, operation, args, source_stream):
    from . import chain_analysis_datasets as datasets
    from .chain_analysis_runtime import SOURCES, scope_key
    if operation == "datasets.list":
        return datasets.list_datasets(conn, profile_id, args)
    if operation == "datasets.get":
        storage.arguments(args, ("id",), ("id",))
        return datasets.get_dataset(conn, profile_id, args["id"])
    if operation == "datasets.query":
        return datasets.query_claims(conn, profile_id, args)
    if operation == "datasets.revoke":
        storage.arguments(args, ("id", "expected_revision"), ("id", "expected_revision"))
        result = datasets.revoke_dataset(conn, profile_id, args["id"], args["expected_revision"])
        conn.commit()
        return result
    if operation == "datasets.discard":
        storage.arguments(args, ("id",), ("id",))
        return datasets.discard_dataset(conn, profile_id, args["id"])
    fields = ("source_token", "manifest", "format", "adapter", "expected_sha256")
    required = ("manifest", "expected_sha256") if operation == "datasets.import" else ("manifest",)
    storage.arguments(args, fields, required)
    options = {"format": args.get("format", "csv"), "adapter": args.get("adapter", "generic")}
    def execute(stream):
        if operation == "datasets.preview":
            result = datasets.preview_dataset(args["manifest"], stream, **options)
            if source_stream is None:
                recipe = {"manifest": result["manifest"], **options}
                SOURCES.remember_preview(scope_key(conn, profile_id), args["source_token"], recipe, result)
            return result
        return datasets.import_dataset(conn, profile_id, args["manifest"], stream, expected_sha256=args["expected_sha256"], **options)
    if source_stream is not None:
        return execute(source_stream)
    with SOURCES.open(scope_key(conn, profile_id), args.get("source_token"), "dataset") as stream:
        return execute(stream)
