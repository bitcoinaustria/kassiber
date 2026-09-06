"""Daemon adapters for cancellable local attribution-file work.

Preview workers need no database. Imports open a separate connection bound to
the original database identity/profile; they never use the main-thread handle.
The core owns streaming validation, invisible staging and atomic activation.
"""
from __future__ import annotations

import copy
import time

from .db import database_instance_id, open_db
from .errors import AppError
from .core import chain_analysis_datasets as datasets
from .core.chain_analysis_cases import arguments
from .core.chain_analysis_runtime import JOBS, SOURCES, scope_key


def job_starter(data_root, passphrase=None):
    # A chat keeps the project/unlock context that was captured when it began.
    # Neither this closure nor the passphrase enters a job request or receipt.
    def start(conn, operation, args, *, profile_id):
        importing = operation == "datasets.import.start"
        discarding = operation == "datasets.discard.start"
        allowed = ("source_token", "manifest", "format", "adapter", "expected_sha256")
        required = ("source_token", "manifest", "expected_sha256") if importing else ("source_token", "manifest")
        arguments(args, ("id",) if discarding else allowed, ("id",) if discarding else required)
        manifest = None if discarding else datasets.normalize_manifest(args["manifest"])
        scope = scope_key(conn, profile_id)
        token = args.get("source_token")
        # Refuse stale/foreign selections before creating a running receipt.
        if not discarding:
            with SOURCES.open(scope, token, "dataset"):
                pass
        else:
            if datasets.get_dataset(conn, profile_id, args["id"])["status"] not in {"staging", "failed"}:
                raise AppError("Only incomplete imports can be discarded", code="validation")
        identity = database_instance_id(conn)
        options = {"format": args.get("format", "csv"), "adapter": args.get("adapter", "generic")}
        expected_hash = args.get("expected_sha256")
        submitted = {"operation": operation, "manifest": manifest, **options, "expected_sha256": expected_hash}
        if discarding:
            submitted["id"] = args["id"]
        submitted = copy.deepcopy(submitted)
        def compute(progress, cancelled):
            began = time.monotonic()
            def stopped():
                return cancelled() or time.monotonic() - began >= 1800
            def update(value):
                progress({"phase": "discarding" if discarding else "staging" if importing else "validating", **value})
            worker = None
            try:
                if stopped():
                    return {"status": "cancelled"}
                if importing or discarding:
                    worker = open_db(data_root, passphrase=passphrase, require_existing_schema=True, expected_database_identity=identity)
                if discarding:
                    return datasets.discard_dataset(worker, profile_id, submitted["id"], progress=update, cancelled=stopped)
                with SOURCES.open(scope, token, "dataset") as stream:
                    if importing:
                        return datasets.import_dataset(worker, profile_id, submitted["manifest"], stream,
                            expected_sha256=expected_hash, **options, progress=update, cancelled=stopped)
                    result = datasets.preview_dataset(submitted["manifest"], stream, **options, progress=update, cancelled=stopped)
                    if not stopped():
                        SOURCES.remember_preview(scope, token, {"manifest": result["manifest"], **options}, result)
                    return result
            except AppError as exc:
                if stopped():
                    return {"status": "cancelled", "reason": "cancelled" if cancelled() else "duration_limit"}
                raise exc
            finally:
                if worker is not None:
                    worker.close()
        return JOBS.start(scope, submitted, compute)
    return start
