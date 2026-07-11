"""Injected dumb-object mailbox transports: folder, WebDAV, and S3."""

from __future__ import annotations

import base64
from dataclasses import dataclass
from datetime import datetime, timezone
import hashlib
import hmac
import json
import os
from pathlib import Path, PurePosixPath
import sqlite3
import tempfile
import time
from typing import Callable, Mapping, Protocol
from urllib.error import HTTPError, URLError
from urllib.parse import quote, unquote, urlencode, urljoin, urlparse
from urllib.request import Request
import uuid
import xml.etree.ElementTree as ET

from ...errors import AppError
from ...http_client import host_limiter
from ...proxy import urlopen_with_proxy
from ...retry import retry_after_seconds_from_http_error
from ...time_utils import now_iso


_HTTP_RETRY_STATUS = frozenset({429, 503})
_HTTP_MAX_ATTEMPTS = 3
_HTTP_BACKOFF_CAP_SECONDS = 8.0


def _is_loopback_host(hostname: str | None) -> bool:
    if not hostname:
        return False
    normalized = hostname.strip("[]").lower()
    if normalized == "localhost":
        return True
    try:
        import ipaddress

        return ipaddress.ip_address(normalized).is_loopback
    except ValueError:
        return False


def _retrying_open(
    *,
    request: Request,
    opener: Callable,
    timeout: int,
    sleeper: Callable[[float], None],
    max_attempts: int,
):
    attempts = max(1, int(max_attempts))
    limiter = host_limiter(request.full_url)
    for attempt in range(attempts):
        retry_after = None
        limiter.acquire()
        try:
            return opener(request, timeout=timeout)
        except HTTPError as exc:
            if exc.code not in _HTTP_RETRY_STATUS or attempt + 1 >= attempts:
                raise
            retry_after = retry_after_seconds_from_http_error(exc)
        finally:
            limiter.release()
        delay = (
            float(retry_after)
            if retry_after is not None
            else min(_HTTP_BACKOFF_CAP_SECONDS, float(2**attempt))
        )
        sleeper(delay)
    raise AssertionError("unreachable mailbox HTTP retry state")


class ObjectTransport(Protocol):
    def put(self, key: str, payload: bytes, *, if_absent: bool = False) -> None:
        raise NotImplementedError

    def get(self, key: str) -> bytes:
        raise NotImplementedError

    def list(self, prefix: str) -> list[str]:
        raise NotImplementedError

    def exists(self, key: str) -> bool:
        raise NotImplementedError


def _safe_key(key: str) -> str:
    path = PurePosixPath(str(key or ""))
    if path.is_absolute() or not path.parts or any(part in {"", ".", ".."} for part in path.parts):
        raise AppError("mailbox object key is unsafe", code="sync_transport_invalid")
    return path.as_posix()


def _decode_webdav_key(raw_key: str) -> str:
    """Decode one DAV href without allowing encoded path separators."""

    decoded_parts: list[str] = []
    for raw_part in raw_key.split("/"):
        if not raw_part:
            raise AppError("WebDAV listing key is invalid", code="sync_transport_invalid")
        try:
            decoded = unquote(raw_part, errors="strict")
        except UnicodeDecodeError as exc:
            raise AppError("WebDAV listing key is invalid", code="sync_transport_invalid") from exc
        if (
            decoded in {"", ".", ".."}
            or "/" in decoded
            or "\\" in decoded
            or "\x00" in decoded
        ):
            raise AppError("WebDAV listing key is unsafe", code="sync_transport_invalid")
        decoded_parts.append(decoded)
    return _safe_key("/".join(decoded_parts))


@dataclass
class FolderTransport:
    root: Path

    def __post_init__(self) -> None:
        self.root = Path(self.root).expanduser().resolve()
        self.root.mkdir(parents=True, exist_ok=True)

    def _path(self, key: str) -> Path:
        safe = _safe_key(key)
        path = (self.root / safe).resolve()
        try:
            path.relative_to(self.root)
        except ValueError as exc:
            raise AppError("mailbox path escaped its root", code="sync_transport_invalid") from exc
        return path

    def put(self, key: str, payload: bytes, *, if_absent: bool = False) -> None:
        destination = self._path(key)
        destination.parent.mkdir(parents=True, exist_ok=True)
        if if_absent and destination.exists():
            if destination.read_bytes() != payload:
                raise AppError("mailbox object already exists with different bytes", code="sync_mailbox_collision")
            return
        fd, name = tempfile.mkstemp(prefix=f".{destination.name}.", dir=destination.parent)
        temporary = Path(name)
        try:
            with os.fdopen(fd, "wb") as handle:
                handle.write(payload)
                handle.flush()
                os.fsync(handle.fileno())
            os.chmod(temporary, 0o600)
            os.replace(temporary, destination)
        except Exception:
            temporary.unlink(missing_ok=True)
            raise

    def get(self, key: str) -> bytes:
        path = self._path(key)
        if not path.is_file() or path.is_symlink():
            raise AppError("mailbox object was not found", code="sync_mailbox_missing", details={"key": key})
        return path.read_bytes()

    def list(self, prefix: str) -> list[str]:
        safe_prefix = _safe_key(prefix)
        base = self._path(safe_prefix)
        if base.is_file():
            return [safe_prefix]
        if not base.exists():
            return []
        output = []
        for entry in base.rglob("*"):
            if entry.is_file() and not entry.is_symlink():
                output.append(entry.relative_to(self.root).as_posix())
        return sorted(output)

    def exists(self, key: str) -> bool:
        path = self._path(key)
        return path.is_file() and not path.is_symlink()


class WebDavTransport:
    def __init__(
        self,
        *,
        base_url: str,
        username: str | None = None,
        password: str | None = None,
        opener: Callable | None = None,
        timeout: int = 30,
        proxy_url: str | None = None,
        sleeper: Callable[[float], None] = time.sleep,
        max_attempts: int = _HTTP_MAX_ATTEMPTS,
    ) -> None:
        parsed = urlparse(base_url)
        if parsed.scheme not in {"http", "https"} or not parsed.netloc:
            raise AppError("WebDAV URL must be http(s)", code="sync_transport_invalid")
        if parsed.scheme == "http" and not _is_loopback_host(parsed.hostname):
            raise AppError(
                "WebDAV credentials require HTTPS except on loopback",
                code="sync_transport_insecure",
                hint="Use an https:// WebDAV URL. Plain HTTP is allowed only for localhost testing.",
            )
        self.base_url = base_url.rstrip("/") + "/"
        self.username = username
        self.password = password
        self.proxy_url = str(proxy_url or "").strip() or None
        self.opener = opener or (
            lambda request, timeout: urlopen_with_proxy(
                request,
                timeout=timeout,
                proxy_url=self.proxy_url,
                source_label="sync mailbox",
            )
        )
        self.timeout = timeout
        self.sleeper = sleeper
        self.max_attempts = max_attempts

    def _url(self, key: str) -> str:
        return urljoin(self.base_url, "/".join(quote(part, safe="") for part in _safe_key(key).split("/")))

    def _headers(self) -> dict[str, str]:
        headers: dict[str, str] = {}
        if self.username is not None:
            token = base64.b64encode(f"{self.username}:{self.password or ''}".encode()).decode()
            headers["Authorization"] = f"Basic {token}"
        return headers

    def _open(self, request: Request, *, passthrough_status: frozenset[int] = frozenset()):
        try:
            return _retrying_open(
                request=request,
                opener=self.opener,
                timeout=self.timeout,
                sleeper=self.sleeper,
                max_attempts=self.max_attempts,
            )
        except HTTPError as exc:
            if exc.code in passthrough_status:
                raise
            raise AppError(
                "WebDAV mailbox request failed",
                code="sync_transport_http_error",
                details={"status": exc.code, "method": request.method},
                retryable=exc.code >= 500 or exc.code == 429,
            ) from exc
        except (URLError, OSError) as exc:
            raise AppError("WebDAV mailbox is unavailable", code="sync_transport_unavailable", retryable=True) from exc

    def _ensure_collections(self, key: str) -> None:
        parts = _safe_key(key).split("/")[:-1]
        for index in range(1, len(parts) + 1):
            url = self._url("/".join(parts[:index]))
            request = Request(url, method="MKCOL", headers=self._headers())
            try:
                with self._open(request, passthrough_status=frozenset({301, 405})):
                    pass
            except HTTPError as exc:
                if exc.code not in {301, 405}:
                    raise AppError(
                        "WebDAV collection could not be created",
                        code="sync_transport_http_error",
                        details={"status": exc.code},
                        retryable=exc.code >= 500,
                    ) from exc

    def put(self, key: str, payload: bytes, *, if_absent: bool = False) -> None:
        self._ensure_collections(key)
        headers = self._headers() | {"Content-Type": "application/octet-stream"}
        if if_absent:
            headers["If-None-Match"] = "*"
        request = Request(self._url(key), data=payload, method="PUT", headers=headers)
        try:
            with self._open(request):
                pass
        except AppError as exc:
            if if_absent and exc.details and exc.details.get("status") == 412:
                if self.get(key) == payload:
                    return
            raise

    def get(self, key: str) -> bytes:
        with self._open(Request(self._url(key), method="GET", headers=self._headers())) as response:
            return response.read()

    def list(self, prefix: str) -> list[str]:
        safe_prefix = _safe_key(prefix)
        headers = self._headers() | {"Depth": "infinity", "Content-Type": "application/xml"}
        body = b'<?xml version="1.0"?><propfind xmlns="DAV:"><prop><resourcetype/></prop></propfind>'
        request = Request(self._url(prefix), data=body, method="PROPFIND", headers=headers)
        with self._open(request) as response:
            payload = response.read()
        try:
            root = ET.fromstring(payload)
        except ET.ParseError as exc:
            raise AppError("WebDAV listing response is invalid", code="sync_transport_invalid") from exc
        base_path = urlparse(self.base_url).path.rstrip("/") + "/"
        output = []
        for href in root.findall(".//{DAV:}href"):
            path = urlparse(href.text or "").path
            if not path.startswith(base_path):
                continue
            raw_key = path[len(base_path) :].strip("/")
            if not raw_key or path.endswith("/"):
                continue
            key = _decode_webdav_key(raw_key)
            if key == safe_prefix or key.startswith(safe_prefix + "/"):
                output.append(key)
        return sorted(set(output))

    def exists(self, key: str) -> bool:
        request = Request(self._url(key), method="HEAD", headers=self._headers())
        try:
            with self._open(request, passthrough_status=frozenset({404})):
                return True
        except HTTPError as exc:
            if exc.code == 404:
                return False
            raise


class S3Transport:
    """Small SigV4 S3-compatible object client with no SDK dependency."""

    def __init__(
        self,
        *,
        endpoint: str,
        bucket: str,
        region: str,
        access_key: str,
        secret_key: str,
        prefix: str = "",
        session_token: str | None = None,
        opener: Callable | None = None,
        timeout: int = 30,
        proxy_url: str | None = None,
        sleeper: Callable[[float], None] = time.sleep,
        max_attempts: int = _HTTP_MAX_ATTEMPTS,
    ) -> None:
        parsed = urlparse(endpoint)
        if parsed.scheme not in {"http", "https"} or not parsed.netloc:
            raise AppError("S3 endpoint must be http(s)", code="sync_transport_invalid")
        if parsed.scheme == "http" and not _is_loopback_host(parsed.hostname):
            raise AppError(
                "S3 credentials require HTTPS except on loopback",
                code="sync_transport_insecure",
                hint="Use an https:// S3 endpoint. Plain HTTP is allowed only for localhost testing.",
            )
        normalized_proxy = str(proxy_url or "").strip() or None
        if parsed.scheme == "http" and normalized_proxy is not None:
            raise AppError(
                "Plain-HTTP loopback S3 endpoints cannot use a proxy",
                code="sync_transport_insecure",
                hint="Remove the proxy for local S3 testing, or use an https:// endpoint.",
            )
        if not bucket or not access_key or not secret_key:
            raise AppError("S3 bucket and credentials are required", code="sync_transport_invalid")
        self.endpoint = endpoint.rstrip("/")
        self.bucket = bucket
        self.region = region or "us-east-1"
        self.access_key = access_key
        self.secret_key = secret_key
        self.prefix = prefix.strip("/")
        self.session_token = session_token
        self.proxy_url = normalized_proxy
        self.opener = opener or (
            lambda request, timeout: urlopen_with_proxy(
                request,
                timeout=timeout,
                proxy_url=self.proxy_url,
                source_label="sync mailbox",
            )
        )
        self.timeout = timeout
        self.sleeper = sleeper
        self.max_attempts = max_attempts

    def _object_key(self, key: str) -> str:
        safe = _safe_key(key)
        return f"{self.prefix}/{safe}" if self.prefix else safe

    def _signing_key(self, date_stamp: str) -> bytes:
        date = hmac.new(("AWS4" + self.secret_key).encode(), date_stamp.encode(), hashlib.sha256).digest()
        region = hmac.new(date, self.region.encode(), hashlib.sha256).digest()
        service = hmac.new(region, b"s3", hashlib.sha256).digest()
        return hmac.new(service, b"aws4_request", hashlib.sha256).digest()

    def _request(
        self,
        method: str,
        *,
        key: str = "",
        query: Mapping[str, str] | None = None,
        payload: bytes = b"",
        extra_headers: Mapping[str, str] | None = None,
    ):
        now = datetime.now(timezone.utc)
        amz_date = now.strftime("%Y%m%dT%H%M%SZ")
        date_stamp = now.strftime("%Y%m%d")
        object_key = self._object_key(key) if key else ""
        canonical_uri = f"/{quote(self.bucket, safe='')}/"
        if object_key:
            canonical_uri += "/".join(quote(part, safe="-_.~") for part in object_key.split("/"))
        canonical_query = urlencode(sorted((query or {}).items()), quote_via=quote, safe="-_.~")
        host = urlparse(self.endpoint).netloc
        payload_hash = hashlib.sha256(payload).hexdigest()
        headers = {
            "host": host,
            "x-amz-content-sha256": payload_hash,
            "x-amz-date": amz_date,
        }
        if self.session_token:
            headers["x-amz-security-token"] = self.session_token
        for name, value in (extra_headers or {}).items():
            headers[name.lower()] = value.strip()
        signed_header_names = sorted(headers)
        canonical_headers = "".join(f"{name}:{headers[name]}\n" for name in signed_header_names)
        signed_headers = ";".join(signed_header_names)
        canonical_request = "\n".join(
            [method, canonical_uri, canonical_query, canonical_headers, signed_headers, payload_hash]
        )
        scope = f"{date_stamp}/{self.region}/s3/aws4_request"
        string_to_sign = "\n".join(
            ["AWS4-HMAC-SHA256", amz_date, scope, hashlib.sha256(canonical_request.encode()).hexdigest()]
        )
        signature = hmac.new(self._signing_key(date_stamp), string_to_sign.encode(), hashlib.sha256).hexdigest()
        headers["authorization"] = (
            f"AWS4-HMAC-SHA256 Credential={self.access_key}/{scope}, "
            f"SignedHeaders={signed_headers}, Signature={signature}"
        )
        url = self.endpoint + canonical_uri + (f"?{canonical_query}" if canonical_query else "")
        request = Request(url, data=payload if method in {"PUT", "POST"} else None, method=method, headers=headers)
        try:
            return _retrying_open(
                request=request,
                opener=self.opener,
                timeout=self.timeout,
                sleeper=self.sleeper,
                max_attempts=self.max_attempts,
            )
        except HTTPError as exc:
            raise AppError(
                "S3 mailbox request failed",
                code="sync_transport_http_error",
                details={"status": exc.code, "method": method},
                retryable=exc.code >= 500 or exc.code == 429,
            ) from exc
        except (URLError, OSError) as exc:
            raise AppError("S3 mailbox is unavailable", code="sync_transport_unavailable", retryable=True) from exc

    def put(self, key: str, payload: bytes, *, if_absent: bool = False) -> None:
        headers = {"if-none-match": "*"} if if_absent else {}
        try:
            with self._request("PUT", key=key, payload=payload, extra_headers=headers):
                pass
        except AppError as exc:
            if if_absent and exc.details and exc.details.get("status") == 412 and self.get(key) == payload:
                return
            raise

    def get(self, key: str) -> bytes:
        with self._request("GET", key=key) as response:
            return response.read()

    def exists(self, key: str) -> bool:
        try:
            with self._request("HEAD", key=key):
                return True
        except AppError as exc:
            if exc.details and exc.details.get("status") == 404:
                return False
            raise

    def list(self, prefix: str) -> list[str]:
        requested = self._object_key(_safe_key(prefix))
        token: str | None = None
        output: list[str] = []
        while True:
            query = {"list-type": "2", "prefix": requested}
            if token:
                query["continuation-token"] = token
            with self._request("GET", query=query) as response:
                payload = response.read()
            try:
                root = ET.fromstring(payload)
            except ET.ParseError as exc:
                raise AppError("S3 listing response is invalid", code="sync_transport_invalid") from exc
            namespace = "{http://s3.amazonaws.com/doc/2006-03-01/}"
            for node in root.findall(f".//{namespace}Key") + root.findall(".//Key"):
                value = node.text or ""
                if self.prefix and value.startswith(self.prefix + "/"):
                    value = value[len(self.prefix) + 1 :]
                output.append(value)
            truncated = (root.findtext(f"{namespace}IsTruncated") or root.findtext("IsTruncated") or "false").lower() == "true"
            if not truncated:
                break
            token = root.findtext(f"{namespace}NextContinuationToken") or root.findtext("NextContinuationToken")
            if not token:
                raise AppError("S3 listing omitted continuation token", code="sync_transport_invalid")
        return sorted(set(output))


def configure_transport(
    conn,
    *,
    profile_id: str,
    kind: str,
    label: str,
    config: Mapping[str, object],
    credentials: Mapping[str, object] | None = None,
) -> dict:
    from .identity import connection_is_encrypted

    book = conn.execute(
        "SELECT enabled FROM sync_books WHERE profile_id = ?", (profile_id,)
    ).fetchone()
    if not book:
        raise AppError("enable sync before configuring a transport", code="sync_disabled")
    if not connection_is_encrypted(conn):
        raise AppError(
            "sync transport credentials require an encrypted SQLCipher database",
            code="sync_requires_encrypted_database",
        )
    kind = str(kind or "").lower()
    label = str(label or "").strip()
    if kind not in {"folder", "webdav", "s3"} or not label:
        raise AppError("sync transport kind and label are invalid", code="validation")
    config = dict(config or {})
    credentials = dict(credentials or {})
    if kind == "folder":
        folder = Path(str(config.get("path") or "")).expanduser()
        if not str(config.get("path") or "").strip():
            raise AppError("folder transport requires path", code="validation")
        config = {"path": str(folder.resolve())}
        credentials = {}
    elif kind == "webdav":
        WebDavTransport(
            base_url=str(config.get("url") or ""),
            username=str(credentials.get("username")) if credentials.get("username") is not None else None,
            password=str(credentials.get("password")) if credentials.get("password") is not None else None,
        )
        config = {
            "url": str(config.get("url")),
            "timeout": int(config.get("timeout") or 30),
            "proxy": str(config.get("proxy") or ""),
        }
        credentials = {key: credentials[key] for key in ("username", "password") if key in credentials}
    else:
        required = ("endpoint", "bucket")
        if any(not str(config.get(key) or "").strip() for key in required) or any(
            not str(credentials.get(key) or "").strip() for key in ("access_key", "secret_key")
        ):
            raise AppError("S3 transport requires endpoint, bucket, access key, and secret key", code="validation")
        S3Transport(
            endpoint=str(config["endpoint"]),
            bucket=str(config["bucket"]),
            region=str(config.get("region") or "us-east-1"),
            prefix=str(config.get("prefix") or ""),
            access_key=str(credentials["access_key"]),
            secret_key=str(credentials["secret_key"]),
            session_token=(
                str(credentials["session_token"])
                if credentials.get("session_token") is not None
                else None
            ),
            proxy_url=str(config.get("proxy") or "") or None,
        )
        config = {
            "endpoint": str(config["endpoint"]),
            "bucket": str(config["bucket"]),
            "region": str(config.get("region") or "us-east-1"),
            "prefix": str(config.get("prefix") or ""),
            "timeout": int(config.get("timeout") or 30),
            "proxy": str(config.get("proxy") or ""),
        }
        credentials = {
            key: credentials[key]
            for key in ("access_key", "secret_key", "session_token")
            if key in credentials
        }
    transport_id = str(uuid.uuid4())
    timestamp = now_iso()
    try:
        conn.execute(
            """
            INSERT INTO sync_transports(
                id, profile_id, kind, label, config_json, credential_json,
                enabled, created_at, updated_at
            ) VALUES(?, ?, ?, ?, ?, ?, 1, ?, ?)
            """,
            (transport_id, profile_id, kind, label, json.dumps(config, sort_keys=True), json.dumps(credentials, sort_keys=True), timestamp, timestamp),
        )
    except sqlite3.IntegrityError as exc:
        raise AppError("sync transport label already exists", code="conflict") from exc
    return get_transport(conn, profile_id=profile_id, transport_id=transport_id)


def _safe_transport_row(row) -> dict:
    config = json.loads(row["config_json"])
    credentials = json.loads(row["credential_json"])
    if row["kind"] == "folder":
        safe_config = {"path": config.get("path")}
    elif row["kind"] == "webdav":
        parsed = urlparse(str(config.get("url") or ""))
        safe_config = {"origin": f"{parsed.scheme}://{parsed.netloc}" if parsed.netloc else None}
    else:
        parsed = urlparse(str(config.get("endpoint") or ""))
        safe_config = {
            "origin": f"{parsed.scheme}://{parsed.netloc}" if parsed.netloc else None,
            "bucket": config.get("bucket"),
            "region": config.get("region"),
        }
    return {
        "id": row["id"],
        "profile_id": row["profile_id"],
        "kind": row["kind"],
        "label": row["label"],
        "enabled": bool(row["enabled"]),
        "config": safe_config,
        "credentials_configured": bool(credentials),
        "last_push_at": row["last_push_at"],
        "last_pull_at": row["last_pull_at"],
        "last_error_at": row["last_error_at"],
        "last_error_code": row["last_error_code"],
        "created_at": row["created_at"],
        "updated_at": row["updated_at"],
    }


def get_transport(conn, *, profile_id: str, transport_id: str | None = None, label: str | None = None) -> dict:
    if (transport_id is None) == (label is None):
        raise AppError("identify transport by id or label", code="validation")
    column, value = ("id", transport_id) if transport_id is not None else ("label", label)
    row = conn.execute(
        f"SELECT * FROM sync_transports WHERE profile_id = ? AND {column} = ?",
        (profile_id, value),
    ).fetchone()
    if not row:
        raise AppError("sync transport was not found", code="not_found")
    return _safe_transport_row(row)


def list_transports(conn, *, profile_id: str) -> list[dict]:
    return [
        _safe_transport_row(row)
        for row in conn.execute(
            "SELECT * FROM sync_transports WHERE profile_id = ? ORDER BY label, id",
            (profile_id,),
        ).fetchall()
    ]


def delete_transport(conn, *, profile_id: str, transport_id: str) -> dict:
    current = get_transport(conn, profile_id=profile_id, transport_id=transport_id)
    conn.execute(
        "DELETE FROM sync_transports WHERE profile_id = ? AND id = ?",
        (profile_id, transport_id),
    )
    return {"id": current["id"], "label": current["label"], "deleted": True}


def load_transport(conn, *, profile_id: str, transport_id: str | None = None, label: str | None = None) -> tuple[dict, ObjectTransport]:
    if (transport_id is None) == (label is None):
        raise AppError("identify transport by id or label", code="validation")
    column, value = ("id", transport_id) if transport_id is not None else ("label", label)
    row = conn.execute(
        f"SELECT * FROM sync_transports WHERE profile_id = ? AND {column} = ? AND enabled = 1",
        (profile_id, value),
    ).fetchone()
    if not row:
        raise AppError("enabled sync transport was not found", code="not_found")
    config = json.loads(row["config_json"])
    credentials = json.loads(row["credential_json"])
    if row["kind"] == "folder":
        transport: ObjectTransport = FolderTransport(Path(config["path"]))
    elif row["kind"] == "webdav":
        transport = WebDavTransport(
            base_url=config["url"],
            username=credentials.get("username"),
            password=credentials.get("password"),
            timeout=int(config.get("timeout") or 30),
            proxy_url=config.get("proxy"),
        )
    else:
        transport = S3Transport(
            endpoint=config["endpoint"],
            bucket=config["bucket"],
            region=config.get("region") or "us-east-1",
            prefix=config.get("prefix") or "",
            access_key=credentials["access_key"],
            secret_key=credentials["secret_key"],
            session_token=credentials.get("session_token"),
            timeout=int(config.get("timeout") or 30),
            proxy_url=config.get("proxy"),
        )
    return dict(row), transport
