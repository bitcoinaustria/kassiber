"""In-memory log ring shared by the daemon and the stdlib logging bridge.

Privacy contract: log records live in RAM only — this module never touches
disk (no FileHandlers, no persistence of any kind). Secret-floor redaction
(seeds, extended keys, descriptors, Silent Payments scan material, API keys,
bearer tokens, passphrases) is applied AT INSERT via
`kassiber.redaction.redact_secret_text`, so that material never exists in any
buffer. Operational redaction (amounts, txids, addresses, paths, URLs) remains
a render/export-time concern of consumers.
Absolute filesystem paths are relativized before storage so records never
leak the home directory.
"""

from __future__ import annotations

import contextvars
import json
import logging
import os
import re
import threading
import traceback
from collections import deque
from datetime import datetime, timezone

from .redaction import is_sensitive_key, redact_operational_text, redact_secret_text
from .time_utils import now_iso


# Correlation id stamped onto records appended while a request is being
# served; the daemon dispatch loop sets it per request so worker threads and
# stdlib log calls inherit it.
current_request_id: contextvars.ContextVar[str | None] = contextvars.ContextVar(
    "current_request_id", default=None
)

_LEVELS = frozenset(("trace", "debug", "info", "warning", "error"))

_TRACEBACK_CAP = 8000
_TRACEBACK_HEAD = 1000
_TRACEBACK_TAIL = 7000

# Matches POSIX (`/a/b`, `~/a`), Windows drive (`C:\a\b`), and UNC (`\\h\s`)
# absolute paths in raw traceback text; `relativize_path` normalizes the
# separators of each match. The drive/UNC alternatives precede the bare
# separator so they win the leftmost match. Segments allow spaces so a
# `C:\Users\John Doe\...` or `/Users/John Doe/...` path is matched (and thus
# relativized) as a whole instead of leaving the username suffix unredacted;
# in tracebacks the surrounding quote/comma terminates the run.
_PATH_SEGMENT_RE = re.compile(
    r"(?:[A-Za-z]:[\\/]|[\\/]{2}|~|[\\/])[\w@.+ -]*(?:[\\/][\w@.+ -]+)+"
)


def relativize_path(path: str) -> str:
    """Strip machine-identifying prefixes from a source path.

    Pure string logic (no filesystem access): keep everything after the last
    `site-packages/` segment, else keep the trailing `kassiber/...` or
    `tests/...` portion, else swap a leading home directory for `~`, else
    fall back to the basename. Windows separators are normalized to `/` so a
    `C:\\Users\\<name>\\...` path cannot smuggle the OS username through.
    """
    if not path:
        return path
    # Display-only output, so canonicalizing to forward slashes is safe and
    # lets one set of markers cover POSIX and Windows alike.
    norm = path.replace("\\", "/")
    marker = "site-packages/"
    idx = norm.rfind(marker)
    if idx != -1:
        return norm[idx + len(marker):]
    if norm.startswith(("kassiber/", "tests/")):
        return norm
    best = -1
    for marker in ("/kassiber/", "/tests/"):
        idx = norm.rfind(marker)
        if idx > best:
            best = idx
    if best != -1:
        return norm[best + 1:]
    home = os.path.expanduser("~").replace("\\", "/").rstrip("/")
    if home and home != "~" and norm.startswith(home + "/"):
        return "~" + norm[len(home):]
    # A drive-letter or UNC root that did not match the host home directory
    # (e.g. a Windows traceback inspected on another host) still carries a
    # username segment, so fall through to the basename below — never return it
    # whole.
    if "/" in norm:
        return norm.rsplit("/", 1)[-1]
    return norm


# The macOS Touch ID helper (`ui-tauri/src-tauri/src/lib.rs`,
# `secret_store.rs`) reports failures as `kassiber native auth: <text>`, which
# the credential scanner reads as an `auth: <secret>` pair, so every helper
# error used to reach the user as `[redacted]`. Exactly these fixed texts, as
# the whole message, pass unchanged; anything else after `auth:` is still
# redacted. Rust's `redaction::sanitize_traceback_text` keeps the same list.
NATIVE_AUTH_ERROR_PREFIX = "kassiber native auth: "
NATIVE_AUTH_ERROR_MESSAGES = frozenset((
    "missing native auth action",
    "missing native auth account",
    "unsupported native auth action",
    "native auth output must be an inherited pipe",
    "native auth secret exceeds 16 KiB",
    "native auth launch gate requires inherited pipes",
    "native auth launch gate received an invalid release",
    "missing --ready-fd",
    "missing --go-fd",
    "missing --output-fd",
    "missing --secret-fd",
    "invalid --ready-fd",
    "invalid --go-fd",
    "invalid --output-fd",
    "invalid --secret-fd",
    "Touch ID is unavailable on this platform",
    "native-auth helper is not inside a signed app bundle",
    "native-auth helper does not support this macOS architecture",
    "native-auth helper parent is not the bundled Kassiber sidecar",
    "native-auth helper and broker signatures do not match",
    "native-auth helper parent has an unexpected signing identifier",
    "native-auth sidecar signing identifier is invalid",
    "native-auth helper has an invalid TeamIdentifier",
    "could not inspect native-auth parent process",
    "native-auth code signature verification failed",
    "native-auth code signature inspection failed",
    "native-auth code signature has no Identifier",
    "native-auth helper requires a production TeamIdentifier",
    "database passphrase must not be empty",
    "Touch ID operator account is invalid.",
    "Touch ID operator unlock is only available in the macOS desktop app.",
    "This build cannot access the protected Keychain item; reopen a production-entitled build to manage it.",
    "No saved desktop biometric passphrase was found.",
    "LocalAuthentication is unavailable on this Mac",
    "Touch ID is not available or not enrolled for this Mac user.",
    "The credential remained in the native store after deletion.",
    "secret_ref_missing",
    "secret_store_locked_or_denied",
))
# Texts that end in an OSStatus: `<head><signed 32-bit integer><tail>`.
NATIVE_AUTH_ERROR_STATUS_TEMPLATES = (
    ("macOS protected Keychain rejected the operation (", ")."),
    ("could not resolve the running native-auth parent code (", ")"),
    ("could not compile the native-auth sidecar requirement (", ")"),
    ("running native-auth parent did not match the bundled sidecar requirement (", ")"),
)


def _is_status_code(text: str) -> bool:
    digits = text[1:] if text.startswith("-") else text
    return 1 <= len(digits) <= 10 and all("0" <= ch <= "9" for ch in digits)


def is_known_native_auth_error(text: str) -> bool:
    """Whether `text` is, in full, one of the Touch ID helper's fixed errors."""

    if not text.startswith(NATIVE_AUTH_ERROR_PREFIX):
        return False
    message = text[len(NATIVE_AUTH_ERROR_PREFIX):]
    if message in NATIVE_AUTH_ERROR_MESSAGES:
        return True
    return any(
        message.startswith(head)
        and message.endswith(tail)
        and len(message) > len(head) + len(tail)
        and _is_status_code(message[len(head):len(message) - len(tail)])
        for head, tail in NATIVE_AUTH_ERROR_STATUS_TEMPLATES
    )


def sanitize_traceback_text(text: str) -> str:
    """Relativize paths, redact secrets + operational ids, and cap traceback text.

    Tracebacks are a `cleaned` egress artifact (they feed the ring's `traceback`
    field, the `internal_error` envelope's `error.debug`, and the CLI `--debug`
    envelope copy), so beyond path/secret scrubbing they also pseudonymize
    txids/amounts that backend exception messages routinely interpolate.
    A known Touch ID helper error (`is_known_native_auth_error`) passes as is.
    """
    if is_known_native_auth_error(text):
        return text
    text = _PATH_SEGMENT_RE.sub(lambda m: relativize_path(m.group(0)), text)
    text = redact_secret_text(text)
    text = redact_operational_text(text)
    if len(text) > _TRACEBACK_CAP:
        text = text[:_TRACEBACK_HEAD] + "...[truncated]..." + text[-_TRACEBACK_TAIL:]
    return text


def sanitize_exception(exc: BaseException) -> str:
    """Format an exception for ring storage (no locals, paths relativized)."""
    try:
        formatted = "".join(traceback.format_exception(type(exc), exc, exc.__traceback__))
    except Exception:
        formatted = repr(exc)
    return sanitize_traceback_text(formatted)


def _coerce_field(value, *, field_name: str | None = None) -> dict:
    """Normalize one field into the wire `{"type", "value"}` shape.

    The secret-floor backstop runs on every string value regardless of its
    declared `type`, so a credential can never ride into the ring under a
    non-`text` label (e.g. a field mislabeled `api_key`/`descriptor`). It only
    strips secret-*shaped* material, so operational values (amounts, txids,
    addresses, paths) still pass through verbatim and are masked at render.
    Anything not JSON-primitive is stringified (and floored) so snapshots stay
    wire-safe.
    """
    if field_name is not None and is_sensitive_key(field_name):
        return {"type": "text", "value": "[redacted]"}
    if isinstance(value, dict) and "type" in value and "value" in value:
        ftype = str(value["type"])
        fval = value["value"]
        if isinstance(fval, str):
            fval = redact_secret_text(fval)
        elif not isinstance(fval, (int, float, bool, type(None))):
            fval = redact_secret_text(str(fval))
        return {"type": ftype, "value": fval}
    return {"type": "text", "value": redact_secret_text(str(value))}


def _record_ts() -> str:
    return (
        datetime.now(timezone.utc)
        .isoformat(timespec="milliseconds")
        .replace("+00:00", "Z")
    )


class LogRing:
    """Bounded, thread-safe in-memory ring of canonical log records."""

    def __init__(self, max_records: int = 5000, max_bytes: int = 4 * 1024 * 1024):
        self._lock = threading.Lock()
        self._records: deque[tuple[dict, int]] = deque()
        self._next_id = 1
        self._evicted_through = 0
        self._buffer_bytes = 0
        self.max_records = max_records
        self.max_bytes = max_bytes
        self.created_at = now_iso()

    @property
    def buffer_bytes(self) -> int:
        with self._lock:
            return self._buffer_bytes

    def append(self, level, module, file, line, msg, fields=None, request_id=None) -> int:
        if level not in _LEVELS:
            level = "info"
        clean_fields = {}
        if fields:
            for key, value in fields.items():
                field_name = str(key)
                clean_fields[field_name] = _coerce_field(
                    value,
                    field_name=field_name,
                )
        rid = request_id if request_id is not None else current_request_id.get()
        if rid and "request_id" not in clean_fields:
            clean_fields["request_id"] = {"type": "text", "value": str(rid)}
        try:
            line_no = int(line)
        except (TypeError, ValueError):
            line_no = 0
        record = {
            "id": 0,
            "ts": _record_ts(),
            "level": level,
            "module": str(module),
            "file": relativize_path(str(file)),
            "line": line_no,
            "msg": redact_secret_text(str(msg)),
            "fields": clean_fields,
        }
        with self._lock:
            record["id"] = self._next_id
            self._next_id += 1
            size = len(json.dumps(record))
            self._records.append((record, size))
            self._buffer_bytes += size
            while len(self._records) > 1 and (
                len(self._records) > self.max_records or self._buffer_bytes > self.max_bytes
            ):
                evicted, evicted_size = self._records.popleft()
                self._buffer_bytes -= evicted_size
                self._evicted_through = evicted["id"]
            return record["id"]

    def snapshot(self, after_id: int = 0, limit: int = 500) -> dict:
        """Snapshot for the `ui.logs.snapshot` daemon kind (read-only)."""
        after_id = max(0, int(after_id))
        limit = max(0, min(int(limit), 2000))
        with self._lock:
            records = []
            for record, _size in self._records:
                if record["id"] <= after_id:
                    continue
                records.append(record)
                if len(records) >= limit:
                    break
            return {
                "records": records,
                "last_id": self._next_id - 1,
                "gap": after_id < self._evicted_through,
                "started_at": self.created_at,
                "buffer_bytes": self._buffer_bytes,
                "max_bytes": self.max_bytes,
            }


class RingHandler(logging.Handler):
    """Bridge stdlib logging records into a `LogRing`.

    Must never raise and never write to stderr — a failing log line cannot be
    allowed to take down or spam the process it is observing.
    """

    def __init__(self, ring: LogRing):
        super().__init__(level=logging.NOTSET)
        self.ring = ring

    def emit(self, record: logging.LogRecord) -> None:
        try:
            if record.levelno >= logging.ERROR:
                level = "error"
            elif record.levelno >= logging.WARNING:
                level = "warning"
            elif record.levelno >= logging.INFO:
                level = "info"
            elif record.levelno >= logging.DEBUG:
                level = "debug"
            else:
                level = "trace"
            try:
                msg = record.getMessage()
            except Exception:
                msg = str(record.msg)
            fields = getattr(record, "kb_fields", None)
            fields = dict(fields) if isinstance(fields, dict) else {}
            if record.exc_info:
                exc = record.exc_info[1]
                if exc is not None:
                    fields["traceback"] = {"type": "text", "value": sanitize_exception(exc)}
            self.ring.append(
                level=level,
                module=record.name,
                file=relativize_path(record.pathname),
                line=record.lineno,
                msg=msg,
                fields=fields,
            )
        except Exception:
            # A logging handler must never raise or write to stderr: a single
            # bad log line cannot be allowed to disrupt or spam the process it
            # observes. Drop the record silently (handleError below is
            # overridden to a no-op for the same reason).
            pass

    def handleError(self, record: logging.LogRecord) -> None:
        pass


_singleton: LogRing | None = None
_singleton_lock = threading.Lock()


def get_log_ring() -> LogRing:
    """Return the process-wide ring, creating it without touching logging."""
    global _singleton
    with _singleton_lock:
        if _singleton is None:
            _singleton = LogRing()
        return _singleton


def install_ring_logging() -> LogRing:
    """Attach one `RingHandler` to the root logger (idempotent)."""
    ring = get_log_ring()
    root = logging.getLogger()
    with _singleton_lock:
        if not any(isinstance(handler, RingHandler) for handler in root.handlers):
            root.addHandler(RingHandler(ring))
        root.setLevel(logging.DEBUG)
    return ring
