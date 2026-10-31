"""Files the user pastes into the chat, or drops into the browser preview.

The native picker and a desktop drop hand the daemon a path the user chose.
A paste has no path: the renderer holds the bytes and sends them here in
chunks, each below the daemon's request line limit. They are written to an
owner-only file in a private temporary directory, staged like a picked file,
and deleted when the staging grant is released or the daemon exits. Nothing
is fetched and nothing reaches a provider here.
"""
from __future__ import annotations

import atexit
import base64
import binascii
from dataclasses import dataclass
import os
import secrets
import shutil
import tempfile
import threading
import time
from typing import Any, Callable, Mapping

from .errors import AppError

KIND = "ui.chat.attachments.upload"

# The chat picker's filter: exports the generic ledger reads, and the
# statements and photos the document importer reads.
CHAT_ATTACHMENT_EXTENSIONS = frozenset({
    ".csv", ".tsv", ".xlsx", ".xlsm", ".pdf", ".png", ".jpg", ".jpeg", ".webp", ".gif",
})
MAX_UPLOAD_BYTES = 25 * 1024 * 1024
# A picked or dropped file is read in place; the analysis tool's own limit.
MAX_PICKED_BYTES = 64 * 1024 * 1024
# Base64 of a chunk this size stays well below MAX_REQUEST_LINE_CHARS.
MAX_CHUNK_BYTES = 512 * 1024
UPLOAD_TTL_SECONDS = 5 * 60.0
MAX_PENDING_UPLOADS = 2
MAX_FILENAME_CHARS = 120

_FIELDS = frozenset({"upload_id", "filename", "total_bytes", "offset", "chunk_base64"})


def chat_attachment_filename(raw: Any) -> str:
    """The file's own name, without any directory, as the chat shows it."""
    if not isinstance(raw, str):
        raise AppError("A file name is required", code="validation", retryable=False)
    name = raw.replace("\\", "/").rsplit("/", 1)[-1]
    name = "".join(char for char in name if char.isprintable()).strip()
    if name in {"", ".", ".."}:
        raise AppError("A file name is required", code="validation", retryable=False)
    stem, extension = os.path.splitext(name)
    if extension.lower() not in CHAT_ATTACHMENT_EXTENSIONS:
        raise AppError(
            "This file type cannot be attached to the chat",
            code="chat_attachment_unsupported",
            hint="Attach a CSV, TSV, XLSX, PDF or image file.",
            details={"extension": extension.lower()},
            retryable=False,
        )
    if len(name) > MAX_FILENAME_CHARS:
        name = stem[: MAX_FILENAME_CHARS - len(extension)] + extension
    return name


def chat_attachment_source(source_file: str) -> str:
    """A picked or dropped chat file: an existing file of a type the chat takes.

    The document importer's own check admits only images and PDFs; a chat
    attachment is usually a CSV or XLSX export.
    """
    candidate = os.path.realpath(os.path.expanduser(source_file))
    if not os.path.isfile(candidate):
        raise AppError("The attached file is not available", code="not_found", retryable=False)
    chat_attachment_filename(os.path.basename(candidate))
    if os.path.getsize(candidate) > MAX_PICKED_BYTES:
        raise AppError(
            "The file is too large to attach",
            code="chat_attachment_too_large",
            details={"max_bytes": MAX_PICKED_BYTES},
            retryable=False,
        )
    return candidate


def discard_upload(path: str) -> None:
    """Delete an uploaded file and the private directory that holds it."""
    try:
        os.unlink(path)
    except FileNotFoundError:
        pass
    except OSError:
        return
    try:
        os.rmdir(os.path.dirname(path))
    except OSError:
        pass


@dataclass
class _PendingUpload:
    upload_id: str
    path: str
    filename: str
    total_bytes: int
    received_bytes: int
    scope: tuple[str, str, str]
    last_activity: float


@dataclass(frozen=True)
class CompletedUpload:
    path: str
    filename: str
    size_bytes: int


class ChatUploads:
    """Chunked uploads in progress, in one private directory per daemon."""

    def __init__(
        self,
        *,
        ttl_seconds: float = UPLOAD_TTL_SECONDS,
        max_pending: int = MAX_PENDING_UPLOADS,
        clock: Callable[[], float] = time.monotonic,
    ) -> None:
        self._ttl_seconds = ttl_seconds
        self._max_pending = max_pending
        self._clock = clock
        self._lock = threading.Lock()
        self._pending: dict[str, _PendingUpload] = {}
        self._root: str | None = None

    def receive(
        self,
        args: Mapping[str, Any],
        *,
        workspace_id: str,
        profile_id: str,
        data_root: str,
    ) -> dict[str, Any] | CompletedUpload:
        """Append one chunk; returns progress, or the finished file."""
        unknown = sorted(set(args) - _FIELDS)
        if unknown:
            raise AppError(
                "Unknown upload fields", code="validation",
                details={"fields": unknown}, retryable=False,
            )
        chunk = _decode_chunk(args.get("chunk_base64"))
        offset = args.get("offset")
        if not isinstance(offset, int) or isinstance(offset, bool) or offset < 0:
            raise AppError("The upload offset is invalid", code="validation", retryable=False)
        scope = (workspace_id, profile_id, data_root)
        with self._lock:
            now = self._clock()
            self._prune_locked(now)
            upload_id = args.get("upload_id")
            if upload_id is None:
                if offset != 0:
                    raise AppError("A new upload starts at offset 0", code="validation", retryable=False)
                upload = self._start_locked(args, scope, now)
            else:
                upload = self._pending.get(upload_id) if isinstance(upload_id, str) else None
                if upload is None or upload.scope != scope:
                    raise AppError(
                        "The upload is no longer available",
                        code="chat_attachment_upload_expired",
                        hint="Attach the file again.",
                        retryable=False,
                    )
                if "filename" in args or "total_bytes" in args:
                    raise AppError(
                        "Only the first chunk names the file", code="validation", retryable=False,
                    )
            if offset != upload.received_bytes:
                self._abort_locked(upload)
                raise AppError(
                    "Upload chunks arrived out of order",
                    code="chat_attachment_upload_expired",
                    hint="Attach the file again.",
                    retryable=False,
                )
            if upload.received_bytes + len(chunk) > upload.total_bytes:
                self._abort_locked(upload)
                raise AppError("The upload is larger than announced", code="validation", retryable=False)
            try:
                with open(upload.path, "ab") as handle:
                    handle.write(chunk)
            except OSError as exc:
                self._abort_locked(upload)
                raise AppError(
                    "Could not keep the attached file", code="chat_attachment_storage_failed",
                    retryable=True,
                ) from exc
            upload.received_bytes += len(chunk)
            upload.last_activity = now
            if upload.received_bytes < upload.total_bytes:
                return {"upload_id": upload.upload_id, "received_bytes": upload.received_bytes}
            self._pending.pop(upload.upload_id, None)
            return CompletedUpload(
                path=upload.path, filename=upload.filename, size_bytes=upload.total_bytes,
            )

    def clear(self) -> None:
        with self._lock:
            for upload in list(self._pending.values()):
                self._abort_locked(upload)

    def _start_locked(
        self,
        args: Mapping[str, Any],
        scope: tuple[str, str, str],
        now: float,
    ) -> _PendingUpload:
        filename = chat_attachment_filename(args.get("filename"))
        total = args.get("total_bytes")
        if not isinstance(total, int) or isinstance(total, bool) or total < 1:
            raise AppError("The file is empty", code="validation", retryable=False)
        if total > MAX_UPLOAD_BYTES:
            raise AppError(
                "The file is too large to attach",
                code="chat_attachment_too_large",
                hint="Attach a file of up to 25 MB, or choose it with the file picker.",
                details={"max_bytes": MAX_UPLOAD_BYTES},
                retryable=False,
            )
        while len(self._pending) >= self._max_pending:
            oldest = min(self._pending.values(), key=lambda entry: entry.last_activity)
            self._abort_locked(oldest)
        directory = tempfile.mkdtemp(dir=self._root_locked())
        path = os.path.join(directory, filename)
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        os.close(descriptor)
        upload = _PendingUpload(
            upload_id=secrets.token_urlsafe(16),
            path=path,
            filename=filename,
            total_bytes=total,
            received_bytes=0,
            scope=scope,
            last_activity=now,
        )
        self._pending[upload.upload_id] = upload
        return upload

    def _root_locked(self) -> str:
        if self._root is None or not os.path.isdir(self._root):
            # mkdtemp creates the directory owner-only (0700).
            self._root = tempfile.mkdtemp(prefix="kassiber-chat-uploads-")
            atexit.register(shutil.rmtree, self._root, True)
        return self._root

    def _abort_locked(self, upload: _PendingUpload) -> None:
        self._pending.pop(upload.upload_id, None)
        discard_upload(upload.path)

    def _prune_locked(self, now: float) -> None:
        for upload in list(self._pending.values()):
            if now - upload.last_activity >= self._ttl_seconds:
                self._abort_locked(upload)


def _decode_chunk(raw: Any) -> bytes:
    if not isinstance(raw, str) or not raw:
        raise AppError("An upload chunk is required", code="validation", retryable=False)
    # 4 base64 characters carry 3 bytes; reject before decoding anything larger.
    if len(raw) > (MAX_CHUNK_BYTES // 3 + 1) * 4:
        raise AppError("The upload chunk is too large", code="validation", retryable=False)
    try:
        chunk = base64.b64decode(raw, validate=True)
    except (binascii.Error, ValueError) as exc:
        raise AppError("The upload chunk is not base64", code="validation", retryable=False) from exc
    if not chunk or len(chunk) > MAX_CHUNK_BYTES:
        raise AppError("The upload chunk is too large", code="validation", retryable=False)
    return chunk
