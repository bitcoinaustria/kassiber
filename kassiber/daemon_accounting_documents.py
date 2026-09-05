"""Bounded cancellable document workers; all SQLite use stays on the main loop."""
from dataclasses import dataclass, field
import threading
import time
from typing import Any

from .core.accounting import document_text, evidence, ledger
from .core.accounting.commands import wire_values
from .core.repo import resolve_scope
from .envelope import build_envelope
from .errors import AppError

KINDS = ("ui.accounting.document_cancel",)


def _scope(ctx):
    if ctx.conn is None:
        raise AppError("Unlock the accounting book", code="accounting_document_cancelled")
    workspace, profile = resolve_scope(ctx.conn)
    ledger.require_book(ctx.conn, profile["id"])
    return (ctx.data_root, ctx.ownership_generation, workspace["id"], profile["id"])


@dataclass
class _Job:
    request_id: str
    scope: tuple
    source: dict
    cancel: threading.Event = field(default_factory=threading.Event)
    done: threading.Event = field(default_factory=threading.Event)
    result: Any = None
    error_code: str | None = None
    thread: threading.Thread | None = None


class DocumentJobs:
    def __init__(self):
        self.jobs: dict[str, _Job] = {}

    def start(self, ctx, request_id, args):
        if not isinstance(request_id, str) or not request_id or len(request_id) > 128:
            raise AppError("Cancellable document request ID required", code="accounting_invalid_fields")
        if not isinstance(args, dict) or set(args) != {"profile_id", "payload"} or not isinstance(args["payload"], dict) or set(args["payload"]) - {"evidence_id", "method", "ocr_pages", "ocr_language"} or "evidence_id" not in args["payload"]:
            raise AppError("Invalid document extraction fields", code="accounting_invalid_fields")
        scope = _scope(ctx)
        if args["profile_id"] != scope[3]:
            raise AppError("Accounting book changed", code="accounting_scope_changed")
        if self.jobs:
            raise AppError("Finish or cancel the active extraction first", code="accounting_document_busy")
        evidence.bounded_text(args["payload"]["evidence_id"], "evidence_id", 200)
        source = evidence.require_evidence(ctx.conn, scope[3], args["payload"]["evidence_id"])
        content = evidence.read_evidence_bytes(ctx.conn, scope[3], source["id"])
        options = {key: value for key, value in args["payload"].items() if key != "evidence_id"}
        job = _Job(request_id, scope, source)
        self.jobs[request_id] = job

        def work():
            nonlocal content
            try:
                job.result = document_text.extract_bytes(content, source["media_type"], cancel=job.cancel, **options)
            except AppError as error:
                job.error_code = error.code
            except Exception:
                job.error_code = "accounting_document_parse_failed"
            finally:
                content = b""
                if job.cancel.is_set():
                    job.result = None
                job.done.set()

        job.thread = threading.Thread(target=work, daemon=True, name="kassiber-document-text")
        job.thread.start()

    def cancel(self, ctx, args):
        scope = _scope(ctx)
        if not isinstance(args, dict) or set(args) != {"profile_id", "payload"} or args["profile_id"] != scope[3] or not isinstance(args["payload"], dict) or set(args["payload"]) != {"evidence_id"}:
            raise AppError("Invalid document cancellation scope", code="accounting_scope_changed")
        evidence.bounded_text(args["payload"]["evidence_id"], "evidence_id", 200)
        for job in self.jobs.values():
            if job.scope == scope and job.source["id"] == args["payload"]["evidence_id"]:
                job.cancel.set()
        return {"cancelled": True}

    def cancel_all(self):
        for job in self.jobs.values():
            job.cancel.set()
            job.result = None

    def shutdown(self, timeout=3.0):
        """Reap cancelled workers before daemon exit; never read/write SQLite.

        Lock uses nonblocking cancel_all. Process shutdown additionally waits
        for trusted worker cancellation to kill/reap its isolated parser group.
        A deadline failure is explicit, never reported as successful teardown.
        """
        if isinstance(timeout, bool) or not isinstance(timeout, (int, float)) or not 0 < timeout <= 30:
            raise AppError("Invalid document shutdown budget", code="accounting_invalid_fields")
        self.cancel_all()
        deadline = time.monotonic() + timeout
        for key, job in list(self.jobs.items()):
            if job.thread is not None:
                job.thread.join(max(0.0, deadline - time.monotonic()))
            if job.thread is not None and job.thread.is_alive():
                raise AppError("Local document worker did not stop within the shutdown budget",
                               code="accounting_document_shutdown_timeout")
            job.result = None
            self.jobs.pop(key, None)

    def poll(self, ctx, out):
        for key, job in list(self.jobs.items()):
            try:
                if _scope(ctx) != job.scope:
                    job.cancel.set()
            except AppError:
                job.cancel.set()
            if not job.done.is_set():
                continue
            code = "accounting_document_cancelled" if job.cancel.is_set() else job.error_code
            try:
                if code:
                    raise AppError("Local document extraction did not complete", code=code)
                pages, method, version = job.result
                result = document_text._retain(ctx.conn, job.scope[3], job.source, pages, method, version)
                ctx.conn.commit()
                response = build_envelope("ui.accounting.document_extract", wire_values({"workspace_id": job.scope[2], "profile_id": job.scope[3], **result}))
            except Exception as error:
                if ctx.conn is not None:
                    ctx.conn.rollback()
                response = {"kind": "error", "schema_version": 1, "error": {
                    "code": error.code if isinstance(error, AppError) else "accounting_document_parse_failed",
                    "message": "Local document extraction did not complete", "hint": None,
                    "details": None, "retryable": False, "debug": None}}
            finally:
                job.result = None
                self.jobs.pop(key, None)
            response["request_id"] = job.request_id
            out.write(response)
