"""RAM-only outbound connection ledger for the desktop egress auditor.

Privacy contract: this module never persists records. It stores only bounded
connection metadata needed to make "never phones home" falsifiable: host, port,
subsystem, operation, and outbound byte count. Paths, query strings, request
bodies, headers, tokens, descriptors, and provider prompts are intentionally
discarded before insert.
"""

from __future__ import annotations

import json
import threading
from collections import Counter, deque
from dataclasses import dataclass
from pathlib import Path
from urllib import parse as urlparse

from .time_utils import now_iso


MAX_RECORDS = 5000
MAX_BYTES = 2 * 1024 * 1024
SQLITE_HEADER = b"SQLite format 3\x00"
HEADER_PREFIX_BYTES = 16

BUILT_IN_EXPECTED_ENDPOINTS = (
    ("api.exchange.coinbase.com", 443, "pricing", "Coinbase Exchange"),
    ("api.coingecko.com", 443, "pricing", "CoinGecko"),
)


@dataclass(frozen=True)
class EgressAllowlistEntry:
    host: str
    port: int | None
    subsystem: str
    label: str
    source: str = "user"
    user_allowlisted: bool = True

    def matches(self, host: str, port: int | None, subsystem: str) -> bool:
        if self.host != _normalize_host(host):
            return False
        if self.port is not None and self.port != port:
            return False
        if self.subsystem != "any" and subsystem != self.subsystem:
            return False
        return True

    def to_payload(self) -> dict:
        return {
            "host": self.host,
            "port": self.port,
            "subsystem": self.subsystem,
            "label": self.label,
            "source": self.source,
            "user_allowlisted": self.user_allowlisted,
        }


def _normalize_host(host: object) -> str:
    return str(host or "").strip().rstrip(".").lower()


def _default_port_for_scheme(scheme: str) -> int | None:
    return {
        "http": 80,
        "https": 443,
        "ssl": 50002,
        "tcp": 50001,
    }.get(str(scheme or "").lower())


def endpoint_from_url(url: object) -> tuple[str, int | None, str]:
    raw = str(url or "").strip()
    if not raw:
        return "", None, ""
    parsed = urlparse.urlsplit(raw if "://" in raw else f"//{raw}")
    scheme = (parsed.scheme or "").lower()
    return (
        _normalize_host(parsed.hostname),
        parsed.port or _default_port_for_scheme(scheme),
        scheme,
    )


def http_request_bytes_out(request, method: str | None = None) -> int:
    """Best-effort application-layer bytes sent for a urllib request.

    Only counts lengths. It never stores header values or body content.
    """

    total = 0
    request_method = method or getattr(request, "get_method", lambda: "GET")()
    total += len(str(request_method or "GET").encode("utf-8"))
    try:
        parts = urlparse.urlsplit(request.full_url)
        target = urlparse.urlunsplit(("", "", parts.path or "/", parts.query, ""))
        total += len(target.encode("utf-8"))
    except Exception:
        total += 1
    try:
        for key, value in request.header_items():
            total += len(str(key).encode("utf-8")) + len(str(value).encode("utf-8")) + 4
    except Exception:
        # Best-effort accounting: if headers cannot be enumerated, ignore them
        # and continue with partial byte estimation.
        pass
    data = getattr(request, "data", None)
    if data is not None:
        if isinstance(data, bytes):
            total += len(data)
        else:
            total += len(str(data).encode("utf-8"))
    return total


class EgressLedger:
    """Bounded, thread-safe in-memory ledger of outbound network activity."""

    def __init__(self, max_records: int = MAX_RECORDS, max_bytes: int = MAX_BYTES):
        self._lock = threading.Lock()
        self._records: deque[tuple[dict, int]] = deque()
        self._next_id = 1
        self._evicted_through = 0
        self._buffer_bytes = 0
        self.max_records = max_records
        self.max_bytes = max_bytes
        self.created_at = now_iso()

    def record(
        self,
        *,
        subsystem: str,
        host: object,
        port: int | None = None,
        scheme: str | None = None,
        operation: str = "connect",
        method: str | None = None,
        bytes_out: int = 0,
        via_proxy: bool = False,
    ) -> int:
        host_norm = _normalize_host(host)
        if not host_norm:
            return 0
        try:
            port_value = int(port) if port is not None else None
        except (TypeError, ValueError):
            port_value = None
        try:
            byte_count = max(0, int(bytes_out))
        except (TypeError, ValueError):
            byte_count = 0
        record = {
            "id": 0,
            "ts": now_iso(),
            "subsystem": str(subsystem or "unknown"),
            "host": host_norm,
            "port": port_value,
            "scheme": str(scheme or ""),
            "operation": str(operation or "connect"),
            "method": str(method or "").upper() or None,
            "bytes_out": byte_count,
            "via_proxy": bool(via_proxy),
        }
        with self._lock:
            record["id"] = self._next_id
            self._next_id += 1
            size = len(json.dumps(record, separators=(",", ":")))
            self._records.append((record, size))
            self._buffer_bytes += size
            while len(self._records) > 1 and (
                len(self._records) > self.max_records or self._buffer_bytes > self.max_bytes
            ):
                evicted, evicted_size = self._records.popleft()
                self._buffer_bytes -= evicted_size
                self._evicted_through = evicted["id"]
            return int(record["id"])

    def record_url(
        self,
        url: object,
        *,
        subsystem: str,
        operation: str = "http.request",
        method: str | None = None,
        bytes_out: int = 0,
        via_proxy: bool = False,
    ) -> int:
        host, port, scheme = endpoint_from_url(url)
        return self.record(
            subsystem=subsystem,
            host=host,
            port=port,
            scheme=scheme,
            operation=operation,
            method=method,
            bytes_out=bytes_out,
            via_proxy=via_proxy,
        )

    def snapshot(
        self,
        *,
        after_id: int = 0,
        limit: int = 500,
        allowlist: list[EgressAllowlistEntry] | None = None,
        allowlist_complete: bool = False,
        db_header: dict | None = None,
    ) -> dict:
        after_id = max(0, int(after_id))
        limit = max(0, min(int(limit), 2000))
        allowlist_entries = list(allowlist or [])
        with self._lock:
            records = []
            all_records = [record for record, _size in self._records]
            for record, _size in self._records:
                if record["id"] <= after_id:
                    continue
                records.append(_annotate_record(record, allowlist_entries, allowlist_complete))
                if len(records) >= limit:
                    break
            totals_by_subsystem = Counter(record["subsystem"] for record in all_records)
            bytes_by_subsystem = Counter()
            unexpected_count = 0
            for record in all_records:
                bytes_by_subsystem[record["subsystem"]] += int(record.get("bytes_out") or 0)
                annotated = _annotate_record(record, allowlist_entries, allowlist_complete)
                if annotated["allowlist_status"] == "unexpected":
                    unexpected_count += 1
            return {
                "records": records,
                "last_id": self._next_id - 1,
                "gap": after_id < self._evicted_through,
                "started_at": self.created_at,
                "buffer_bytes": self._buffer_bytes,
                "max_bytes": self.max_bytes,
                "allowlist_complete": bool(allowlist_complete),
                "allowlist": [entry.to_payload() for entry in allowlist_entries],
                "db_header": db_header or {},
                "summary": {
                    "total_records": len(all_records),
                    "unexpected": unexpected_count,
                    "update": int(totals_by_subsystem.get("update", 0)),
                    "by_subsystem": {
                        key: {
                            "records": int(totals_by_subsystem[key]),
                            "bytes_out": int(bytes_by_subsystem[key]),
                        }
                        for key in sorted(totals_by_subsystem)
                    },
                },
            }


def _annotate_record(
    record: dict,
    allowlist: list[EgressAllowlistEntry],
    allowlist_complete: bool,
) -> dict:
    payload = dict(record)
    if not allowlist_complete:
        payload.update(
            {
                "allowlist_status": "unknown",
                "allowlist_label": None,
                "allowlist_source": None,
                "user_allowlisted": False,
            }
        )
        return payload
    for entry in allowlist:
        if entry.matches(
            str(record.get("host") or ""),
            record.get("port"),
            str(record.get("subsystem") or ""),
        ):
            payload.update(
                {
                    "allowlist_status": "expected",
                    "allowlist_label": entry.label,
                    "allowlist_source": entry.source,
                    "user_allowlisted": bool(entry.user_allowlisted),
                }
            )
            return payload
    payload.update(
        {
            "allowlist_status": "unexpected",
            "allowlist_label": None,
            "allowlist_source": None,
            "user_allowlisted": False,
        }
    )
    return payload


def built_in_allowlist_entries() -> list[EgressAllowlistEntry]:
    return [
        EgressAllowlistEntry(
            host=host,
            port=port,
            subsystem=subsystem,
            label=label,
            source="built-in",
            user_allowlisted=False,
        )
        for host, port, subsystem, label in BUILT_IN_EXPECTED_ENDPOINTS
    ]


def db_header_proof(path: str | Path) -> dict:
    db_path = Path(path).expanduser()
    try:
        raw = db_path.read_bytes()[:HEADER_PREFIX_BYTES]
    except FileNotFoundError:
        return {
            "exists": False,
            "classification": "missing",
            "sqlite_plaintext_header": False,
            "encrypted_like": False,
            "prefix_hex": "",
        }
    except OSError as exc:
        return {
            "exists": False,
            "classification": "unreadable",
            "sqlite_plaintext_header": False,
            "encrypted_like": False,
            "prefix_hex": "",
            "error": str(exc),
        }
    plaintext = raw == SQLITE_HEADER
    return {
        "exists": True,
        "classification": "plaintext-sqlite" if plaintext else "ciphertext-like",
        "sqlite_plaintext_header": plaintext,
        "encrypted_like": bool(raw) and not plaintext,
        "prefix_hex": raw.hex(),
    }


_GLOBAL_LEDGER = EgressLedger()


def get_egress_ledger() -> EgressLedger:
    return _GLOBAL_LEDGER


__all__ = [
    "EgressAllowlistEntry",
    "EgressLedger",
    "built_in_allowlist_entries",
    "db_header_proof",
    "endpoint_from_url",
    "get_egress_ledger",
    "http_request_bytes_out",
]
