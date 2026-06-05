from __future__ import annotations

import socket
from html import unescape
from html.parser import HTMLParser
from typing import Any, Callable
from urllib import error as urlerror
from urllib import parse as urlparse
from urllib import request as urlrequest

MAX_PREVIEW_BYTES = 256 * 1024
PREVIEW_TIMEOUT_SECONDS = 4.0
_HTML_TYPES = {"text/html", "application/xhtml+xml"}


class _TitleParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self._in_title = False
        self._title_parts: list[str] = []
        self.meta: dict[str, str] = {}

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        normalized_tag = tag.lower()
        if normalized_tag == "title":
            self._in_title = True
            return
        if normalized_tag != "meta":
            return
        attr_map = {(key or "").lower(): value or "" for key, value in attrs}
        key = (attr_map.get("property") or attr_map.get("name") or "").lower()
        content = _clean_text(attr_map.get("content"))
        if key and content and key not in self.meta:
            self.meta[key] = content

    def handle_endtag(self, tag: str) -> None:
        if tag.lower() == "title":
            self._in_title = False

    def handle_data(self, data: str) -> None:
        if self._in_title:
            self._title_parts.append(data)

    @property
    def title(self) -> str:
        return _clean_text(" ".join(self._title_parts))


def _clean_text(value: str | None) -> str:
    return " ".join(unescape(value or "").split()).strip()


def _strip_known_suffix(title: str, site_name: str) -> str:
    cleaned = _clean_text(title)
    suffixes = [
        " - Google Docs",
        " - Google Sheets",
        " - Google Slides",
        " - Google Drive",
        " - Google Forms",
    ]
    if site_name:
        suffixes.append(f" - {site_name}")
        suffixes.append(f" | {site_name}")
    for suffix in suffixes:
        if cleaned.endswith(suffix):
            candidate = cleaned[: -len(suffix)].strip()
            if candidate:
                return candidate
    return cleaned


def _display_url(parsed: urlparse.SplitResult) -> str:
    host = (parsed.hostname or parsed.netloc).lower()
    if host.startswith("www."):
        host = host[4:]
    path = parsed.path.rstrip("/")
    return f"{host}{path}" if path and path != "/" else host


def _path_title(path: str) -> str:
    parts = [part for part in path.split("/") if part]
    if not parts:
        return ""
    candidate = parts[-1]
    if len(candidate) >= 16 and all(char.isalnum() or char == "-" for char in candidate):
        return ""
    return _clean_text(
        urlparse.unquote(candidate)
        .rsplit(".", 1)[0]
        .replace("-", " ")
        .replace("_", " ")
        .replace("+", " ")
    )


def fallback_url_label(url: str) -> str:
    parsed = urlparse.urlsplit((url or "").strip())
    if parsed.scheme and parsed.scheme not in {"http", "https"}:
        return "Link attachment"
    host = (parsed.hostname or "").lower()
    if not host:
        return _clean_text(url) or "Link attachment"
    if host.startswith("www."):
        host = host[4:]
    if host in {"docs.google.com", "drive.google.com"}:
        path = parsed.path
        if path.startswith("/document/d/"):
            return "Google Doc"
        if path.startswith("/spreadsheets/d/"):
            return "Google Sheet"
        if path.startswith("/presentation/d/"):
            return "Google Slides deck"
        if path.startswith("/forms/d/"):
            return "Google Form"
        if path.startswith("/drawings/d/"):
            return "Google Drawing"
        if path.startswith("/file/d/"):
            return "Google Drive file"
        if path.startswith("/drive/folders/"):
            return "Google Drive folder"
        return (
            "Google Drive link"
            if host == "drive.google.com"
            else "Google Workspace link"
        )
    title = _path_title(parsed.path)
    if title and title.lower() != host:
        return f"{host} - {title}"
    return host or "Link attachment"


def _base_payload(url: str, parsed: urlparse.SplitResult) -> dict[str, Any]:
    return {
        "url": url,
        "display_url": _display_url(parsed),
        "label": fallback_url_label(url),
        "title": "",
        "site_name": "",
        "content_type": "",
        "available": False,
        "error_code": "",
        "truncated": False,
    }


def _decode_html(raw: bytes, content_type: str) -> str:
    charset = "utf-8"
    for part in content_type.split(";")[1:]:
        key, _, value = part.strip().partition("=")
        if key.lower() == "charset" and value.strip():
            charset = value.strip()
            break
    return raw.decode(charset, errors="replace")


UrlOpen = Callable[..., Any]


def preview_url(
    raw_url: str,
    *,
    opener: UrlOpen = urlrequest.urlopen,
    timeout: float = PREVIEW_TIMEOUT_SECONDS,
    max_bytes: int = MAX_PREVIEW_BYTES,
) -> dict[str, Any]:
    url = (raw_url or "").strip()
    parsed = urlparse.urlsplit(url)
    if parsed.scheme not in {"http", "https"} or not parsed.netloc:
        return {
            **_base_payload(url, parsed),
            "error_code": "unsupported_url",
        }
    if parsed.username or parsed.password:
        return {
            **_base_payload(url, parsed),
            "error_code": "embedded_credentials",
        }

    payload = _base_payload(url, parsed)
    request = urlrequest.Request(
        url,
        headers={
            "accept": "text/html,application/xhtml+xml",
            "user-agent": "Kassiber link preview",
        },
    )
    try:
        with opener(request, timeout=timeout) as response:
            content_type = response.headers.get("content-type", "")
            payload["content_type"] = content_type
            media_type = content_type.split(";", 1)[0].strip().lower()
            if media_type and media_type not in _HTML_TYPES:
                payload["error_code"] = "unsupported_content_type"
                return payload
            raw = response.read(max_bytes + 1)
    except (OSError, TimeoutError, socket.timeout, urlerror.URLError, ValueError) as exc:
        payload["error_code"] = "fetch_failed"
        payload["error"] = str(exc)
        return payload

    payload["truncated"] = len(raw) > max_bytes
    parser = _TitleParser()
    try:
        parser.feed(_decode_html(raw[:max_bytes], str(payload["content_type"])))
    except Exception:
        payload["error_code"] = "parse_failed"
        return payload

    site_name = _clean_text(
        parser.meta.get("og:site_name") or parser.meta.get("application-name")
    )
    title = _clean_text(
        parser.meta.get("og:title")
        or parser.meta.get("twitter:title")
        or parser.title
    )
    title = _strip_known_suffix(title, site_name)
    if title:
        payload.update(
            {
                "available": True,
                "title": title,
                "site_name": site_name,
                "label": title,
                "error_code": "",
            }
        )
    else:
        payload["error_code"] = "title_not_found"
    return payload
