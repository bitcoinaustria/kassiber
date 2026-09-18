"""Bounded, process-local computation receipts over immutable analysis inputs.

Workers never receive the caller's SQLite connection. Pure entropy jobs and
explicit local-file workers have no network or accounting authority. Results describe
the submitted snapshot even if the book changes while a computation runs.
"""
from __future__ import annotations

import copy
import contextlib
import json
import os
from pathlib import Path
import secrets
import stat
import threading
import time
from dataclasses import dataclass, field
from typing import Callable

from ..errors import AppError


def scope_key(conn, profile_id: str) -> bytes:
    databases = conn.execute("PRAGMA database_list").fetchall()
    path = next((str(row[2]) for row in databases if row[1] == "main"), "")
    identity = path or f"memory-connection:{id(conn)}"
    return json.dumps([identity, profile_id], separators=(",", ":")).encode()


@dataclass
class _Job:
    scope: bytes
    request: dict
    created: float
    cancel: threading.Event = field(default_factory=threading.Event)
    status: str = "running"
    progress: dict = field(default_factory=lambda: {"phase": "queued", "states_explored": 0})
    result: dict | None = None
    error_code: str | None = None
    finished: float | None = None


class AnalysisJobs:
    def __init__(self, *, capacity=16, concurrent=2, ttl_seconds=3600, clock=time.monotonic):
        self._capacity, self._concurrent, self._ttl = capacity, concurrent, ttl_seconds
        self._clock = clock
        self._jobs: dict[str, _Job] = {}
        self._running = 0
        self._lock = threading.RLock()
        self._condition = threading.Condition(self._lock)
        self._active: dict[str, _Job] = {}
        self._quiescing = 0

    def _prune(self):
        now = self._clock()
        for ident, job in list(self._jobs.items()):
            if now - job.created >= self._ttl:
                job.cancel.set()
                if job.status != "running":
                    del self._jobs[ident]

    def start(self, scope: bytes, request: dict, compute: Callable) -> dict:
        with self._lock:
            self._prune()
            if self._quiescing or self._running >= self._concurrent:
                raise AppError("Analysis workers are busy; finish or cancel a running computation", code="chain_analysis_busy", retryable=True)
            while len(self._jobs) >= self._capacity:
                completed = [(job.created, ident) for ident, job in self._jobs.items() if job.status != "running"]
                if not completed:
                    raise AppError("Analysis capacity exhausted", code="chain_analysis_busy", retryable=True)
                del self._jobs[min(completed)[1]]
            ident = secrets.token_urlsafe(32)
            job = _Job(scope, copy.deepcopy(request), self._clock())
            self._jobs[ident] = job
            self._active[ident] = job
            self._running += 1

        def progress(value):
            with self._lock:
                job.progress = copy.deepcopy(value)

        def work():
            try:
                result = compute(progress, job.cancel.is_set)
                with self._lock:
                    job.result = result
                    # An import can commit just before cancellation arrives.
                    # Its durable receipt wins; never report a saved pack as
                    # cancelled merely because the request arrived too late.
                    job.status = "cancelled" if result.get("status") == "cancelled" else "completed"
            except Exception as exc:
                # Never retain traceback, raw PSBT, source paths or arbitrary
                # dependency messages in a user/model-visible job receipt.
                with self._lock:
                    job.error_code = None if job.cancel.is_set() else exc.code if isinstance(exc, AppError) else "chain_analysis_computation_failed"
                    job.status = "cancelled" if job.cancel.is_set() else "failed"
            finally:
                with self._lock:
                    self._running -= 1
                    self._active.pop(ident, None)
                    job.finished = self._clock()
                    self._condition.notify_all()

        threading.Thread(target=work, name="chain-analysis", daemon=True).start()
        return self.get(scope, ident)

    def get(self, scope: bytes, ident: str, *, cancel=False) -> dict:
        with self._lock:
            self._prune()
            job = self._jobs.get(ident) if isinstance(ident, str) else None
            if job is None or job.scope != scope:
                raise AppError("Analysis computation expired or belongs to another book", code="chain_analysis_job_unavailable", retryable=False)
            if cancel and job.status == "running":
                job.cancel.set()
            return copy.deepcopy({
                "schema_version": 1, "job_id": ident, "status": job.status,
                "cancel_requested": job.cancel.is_set(), "request": job.request,
                "progress": job.progress, "result": job.result, "error_code": job.error_code,
                "elapsed_ms": int(((job.finished if job.finished is not None else self._clock()) - job.created) * 1000),
                "receipt_scope": "submitted_snapshot", "local_only": True,
            })

    def clear(self, scope: bytes | None = None):
        with self._lock:
            # Receipts may already have been cleared while their workers still
            # own database connections. Cancellation must reach those too.
            for job in self._active.values():
                if scope is None or job.scope == scope:
                    job.cancel.set()
            for ident, job in list(self._jobs.items()):
                if scope is None or job.scope == scope:
                    job.cancel.set()
                    del self._jobs[ident]

    @contextlib.contextmanager
    def quiesce(self, *, timeout_seconds=5):
        """Reserve a replacement window only after worker computations exit.

        Import computations close their dedicated SQLCipher connection in
        ``finally`` before the running count is decremented. Clearing receipts
        alone cannot establish this boundary. Keep admission closed throughout
        the caller's installation, and leave the database untouched on timeout.
        """
        with self._condition:
            self._quiescing += 1
        try:
            with self._condition:
                for job in self._active.values():
                    job.cancel.set()
                deadline = time.monotonic() + timeout_seconds
                while self._running:
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        raise AppError("Wait for analysis workers to finish before restoring",
                                       code="project_in_use", retryable=True)
                    self._condition.wait(remaining)
            yield
        finally:
            with self._condition:
                self._quiescing -= 1


JOBS = AnalysisJobs()


class AnalysisSources:
    """Native-picker grants, pinned to a regular file and one book.

    Only a trusted adapter calls stage with a path. The ordinary daemon and AI
    operations accept the resulting token. No source bytes are saved or sent
    to a provider. Mutation after selection requires selecting the file again.
    """
    LIMITS = {"psbt": 4 * 1024 * 1024, "dataset": 8 * 1024 * 1024 * 1024}

    def __init__(self, *, ttl_seconds=1800, capacity=32, clock=time.monotonic):
        self._ttl, self._capacity, self._clock = ttl_seconds, capacity, clock
        self._sources = {}
        self._previews = {}
        self._lock = threading.Lock()

    @staticmethod
    def _identity(info):
        return (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns)

    def stage(self, scope: bytes, path: str, purpose: str) -> dict:
        if purpose not in self.LIMITS:
            raise AppError("Unsupported analysis source purpose", code="validation")
        try:
            source = Path(path).expanduser().resolve(strict=True)
            info = source.stat()
            if not stat.S_ISREG(info.st_mode) or not 0 < info.st_size <= self.LIMITS[purpose]:
                raise ValueError("source size or type")
        except (OSError, ValueError, TypeError):
            raise AppError("Selected analysis source is unavailable or exceeds its size limit", code="chain_analysis_source_unavailable") from None
        token = secrets.token_urlsafe(32)
        with self._lock:
            now = self._clock()
            self._sources = {key: item for key, item in self._sources.items() if now - item[0] < self._ttl}
            while len(self._sources) >= self._capacity:
                del self._sources[min(self._sources, key=lambda key: self._sources[key][0])]
            self._sources[token] = (now, scope, str(source), purpose, self._identity(info))
        return {"source_token": token, "purpose": purpose, "size_bytes": info.st_size, "filename": source.name}

    @contextlib.contextmanager
    def open(self, scope: bytes, token: str, purpose: str):
        with self._lock:
            entry = self._sources.get(token) if isinstance(token, str) else None
            if entry is None or entry[1] != scope or entry[3] != purpose or self._clock() - entry[0] >= self._ttl:
                raise AppError("Select the analysis source again in its original book", code="chain_analysis_source_expired")
        try:
            fd = os.open(entry[2], os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0))
            stream = os.fdopen(fd, "rb")
        except OSError:
            raise AppError("Selected analysis source is unavailable", code="chain_analysis_source_unavailable") from None
        with stream:
            def verify():
                info = os.fstat(stream.fileno())
                if not stat.S_ISREG(info.st_mode) or self._identity(info) != entry[4]:
                    raise AppError("Selected analysis source changed; select and preview it again", code="chain_analysis_source_changed")
            verify()
            class CheckedReader:
                def read(self, size=-1):
                    result = stream.read(size)
                    verify()
                    return result
                def readline(self, size=-1):
                    result = stream.readline(size)
                    verify()
                    return result
            # Validation occurs before bytes/EOF reach the parser. A context
            # exit must never throw after a caller has committed those bytes.
            yield CheckedReader()

    def remember_preview(self, scope, token, recipe, result):
        key = (scope, token, json.dumps(recipe, sort_keys=True, separators=(",", ":")))
        with self._lock:
            self._previews[key] = (self._clock(), copy.deepcopy(result))
            while len(self._previews) > self._capacity * 2:
                del self._previews[min(self._previews, key=lambda item: self._previews[item][0])]

    def preview(self, scope, token, recipe, expected_sha256):
        # A completed validation is reused for consent, never synchronously
        # repeated on the daemon thread. Import verifies the bytes once more
        # in its cancellable worker before activation.
        with self.open(scope, token, "dataset"):
            pass
        key = (scope, token, json.dumps(recipe, sort_keys=True, separators=(",", ":")))
        with self._lock:
            saved = self._previews.get(key)
            if saved is None or self._clock() - saved[0] >= self._ttl or saved[1].get("sha256") != expected_sha256:
                raise AppError("Complete a local dataset preview for these exact source, manifest and hash inputs", code="chain_analysis_preview_required")
            return copy.deepcopy(saved[1])

    def clear(self):
        with self._lock:
            self._sources.clear()
            self._previews.clear()


SOURCES = AnalysisSources()


def clear_runtime():
    JOBS.clear()
    SOURCES.clear()
