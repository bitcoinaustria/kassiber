"""Minimal GitHub release checks shared by the human-facing CLI surfaces.

Nothing here refreshes itself in the background: `kassiber update` and the
desktop's daily check are the only things that contact GitHub, and every other
CLI invocation just prints what one of them last cached.  The updater never
downloads or executes a release; it only prints a trusted release URL or a
package-manager command for an install method Kassiber can prove locally.
"""

from __future__ import annotations

import json
import os
import re
import stat
import sys
import tempfile
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any, BinaryIO, Callable, Mapping, TextIO
from urllib.error import HTTPError, URLError
from urllib.parse import quote
from urllib.request import HTTPRedirectHandler, Request, build_opener

from . import __version__
from .build_info import packaged_build_info
from .db import DEFAULT_CONFIG_DIRNAME, DEFAULT_STATE_ROOT
from .egress_ledger import get_egress_ledger
from .errors import AppError


_RELEASES_PER_PAGE = 20
GITHUB_RELEASES_API_URL = (
    "https://api.github.com/repos/bitcoinaustria/kassiber/releases"
    f"?per_page={_RELEASES_PER_PAGE}"
)
GITHUB_LATEST_RELEASE_API_URL = (
    "https://api.github.com/repos/bitcoinaustria/kassiber/releases/latest"
)
GITHUB_RELEASES_PAGE_URL = "https://github.com/bitcoinaustria/kassiber/releases"
CHECK_INTERVAL = timedelta(hours=20)
NETWORK_TIMEOUT_SECONDS = 5.0
CACHE_SCHEMA_VERSION = 1
CACHE_FILENAME = "update-check.json"
PREFERENCE_SCHEMA_VERSION = 1
PREFERENCE_FILENAME = "update-checks.json"
UPDATE_CACHE_ENV = "KASSIBER_UPDATE_CACHE_FILE"
UPDATE_PREFERENCE_ENV = "KASSIBER_UPDATE_PREFERENCE_FILE"
HOMEBREW_PACKAGE_ENV = "KASSIBER_HOMEBREW_PACKAGE"
DISABLE_UPDATE_CHECK_ENV = "KASSIBER_DISABLE_UPDATE_CHECK"
HOMEBREW_CASK_COMMAND = (
    "brew upgrade --cask bitcoinaustria/kassiber/kassiber"
)
HOMEBREW_FORMULA_COMMAND = (
    "brew upgrade bitcoinaustria/kassiber/kassiber-cli"
)
# A listing page inlines the full asset list of every release it names, so this
# bound is `_RELEASES_PER_PAGE` times the per-release JSON (~19 KiB at 15 build
# artifacts) plus headroom for a growing build matrix. At `per_page=100` the
# real listing outgrew a 1 MiB cap after 56 releases and every check failed.
_MAX_RESPONSE_BYTES = 2 * 1024 * 1024
_MAX_PREFERENCE_BYTES = 1024
_MAX_CACHE_BYTES = 8 * 1024
_SEMVER_RE = re.compile(
    r"^v?(?P<major>0|[1-9][0-9]*)\."
    r"(?P<minor>0|[1-9][0-9]*)\."
    r"(?P<patch>0|[1-9][0-9]*)"
    r"(?:-(?P<prerelease>[0-9A-Za-z.-]+))?"
    r"(?:\+(?P<build>[0-9A-Za-z.-]+))?$"
)


class _NoRedirectHandler(HTTPRedirectHandler):
    """Keep update metadata pinned to the fixed GitHub API origin."""

    def redirect_request(self, request, file_pointer, code, message, headers, new_url):
        del request, file_pointer, code, message, headers, new_url
        return None


def _open_without_redirects(request: Request, *, timeout: float) -> BinaryIO:
    return build_opener(_NoRedirectHandler()).open(request, timeout=timeout)


@dataclass(frozen=True)
class ParsedVersion:
    major: int
    minor: int
    patch: int
    prerelease: tuple[str, ...]


def _has_exact_schema_version(payload: Any, expected: int) -> bool:
    return bool(
        isinstance(payload, dict)
        and type(payload.get("schema_version")) is int
        and payload["schema_version"] == expected
    )


def _atomic_write_private(destination: Path, text: str) -> None:
    """Atomically replace `destination` with owner-only (0600) UTF-8 content."""

    destination.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    fd, temporary_name = tempfile.mkstemp(
        prefix=f".{destination.name}.",
        suffix=".tmp",
        dir=destination.parent,
    )
    temporary = Path(temporary_name)
    try:
        try:
            os.fchmod(fd, 0o600)
        except (AttributeError, OSError):
            # mkstemp already creates the file owner-only; this is hardening.
            pass
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            handle.write(text)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, destination)
    finally:
        try:
            temporary.unlink()
        except OSError:
            # Cleanup must not hide the original write/replace failure.
            pass


def read_small_private_file(path: Path, limit: int) -> bytes | None:
    """Read a regular, non-symlinked file of at most `limit` bytes, or None.

    Shared fail-closed reader for the consent file and similar small local
    contracts: symlinks, special files, and oversized content all read as
    absent rather than raising.
    """

    try:
        if stat.S_ISLNK(os.lstat(path).st_mode):
            return None
    except OSError:
        return None
    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NONBLOCK", 0)
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        descriptor = os.open(path, flags)
    except OSError:
        return None
    try:
        if not stat.S_ISREG(os.fstat(descriptor).st_mode):
            return None
        with os.fdopen(descriptor, "rb") as handle:
            descriptor = -1
            raw = handle.read(limit + 1)
    except OSError:
        return None
    finally:
        if descriptor >= 0:
            try:
                os.close(descriptor)
            except OSError:
                # Best-effort cleanup after the read path has already failed.
                pass
    return raw if len(raw) <= limit else None


def parse_version(value: str) -> ParsedVersion | None:
    match = _SEMVER_RE.fullmatch(value.strip())
    if match is None:
        return None
    prerelease_text = match.group("prerelease") or ""
    build_text = match.group("build") or ""
    prerelease = tuple(prerelease_text.split(".")) if prerelease_text else ()
    build = tuple(build_text.split(".")) if build_text else ()
    if any(not part for part in (*prerelease, *build)):
        return None
    if any(
        part.isdigit() and len(part) > 1 and part.startswith("0")
        for part in prerelease
    ):
        return None
    return ParsedVersion(
        major=int(match.group("major")),
        minor=int(match.group("minor")),
        patch=int(match.group("patch")),
        prerelease=prerelease,
    )


def _compare_prerelease(left: tuple[str, ...], right: tuple[str, ...]) -> int:
    if not left and not right:
        return 0
    if not left:
        return 1
    if not right:
        return -1
    for left_part, right_part in zip(left, right):
        if left_part == right_part:
            continue
        left_numeric = left_part.isdigit()
        right_numeric = right_part.isdigit()
        if left_numeric and right_numeric:
            return 1 if int(left_part) > int(right_part) else -1
        if left_numeric != right_numeric:
            return -1 if left_numeric else 1
        return 1 if left_part > right_part else -1
    if len(left) == len(right):
        return 0
    return 1 if len(left) > len(right) else -1


def _compare_versions(left: ParsedVersion, right: ParsedVersion) -> int:
    left_base = (left.major, left.minor, left.patch)
    right_base = (right.major, right.minor, right.patch)
    if left_base != right_base:
        return 1 if left_base > right_base else -1
    return _compare_prerelease(left.prerelease, right.prerelease)


def is_newer_version(latest: str, current: str) -> bool:
    latest_parsed = parse_version(latest)
    current_parsed = parse_version(current)
    if latest_parsed is None or current_parsed is None:
        return False
    return _compare_versions(latest_parsed, current_parsed) > 0


def current_version() -> str:
    value = str(packaged_build_info().get("version") or __version__).strip()
    return value[1:] if value.startswith("v") else value


def cache_path() -> Path:
    override = os.environ.get(UPDATE_CACHE_ENV)
    if override:
        return Path(override).expanduser()
    return Path(DEFAULT_STATE_ROOT).expanduser() / DEFAULT_CONFIG_DIRNAME / CACHE_FILENAME


def preference_path() -> Path:
    override = os.environ.get(UPDATE_PREFERENCE_ENV)
    if override:
        return Path(override).expanduser()
    return (
        Path(DEFAULT_STATE_ROOT).expanduser()
        / DEFAULT_CONFIG_DIRNAME
        / PREFERENCE_FILENAME
    )


def _environment_disables_update_checks(
    environ: Mapping[str, str] | None = None,
) -> bool:
    environment = os.environ if environ is None else environ
    return str(environment.get(DISABLE_UPDATE_CHECK_ENV) or "").strip().lower() in {
        "1",
        "true",
        "yes",
    }


def update_checks_enabled(
    path: Path | None = None,
    *,
    environ: Mapping[str, str] | None = None,
) -> bool:
    """Return the explicit global update-check consent, failing closed.

    This file is deliberately separate from project data and renderer storage
    so the desktop native command and every packaged CLI invocation enforce the
    same user choice before opening a connection to GitHub.
    """

    if _environment_disables_update_checks(environ):
        return False
    destination = path or preference_path()
    raw = read_small_private_file(destination, _MAX_PREFERENCE_BYTES)
    if raw is None:
        return False
    try:
        payload = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        return False
    return bool(
        _has_exact_schema_version(payload, PREFERENCE_SCHEMA_VERSION)
        and type(payload.get("enabled")) is bool
        and payload["enabled"]
    )


def set_update_checks_enabled(enabled: bool, path: Path | None = None) -> Path:
    """Atomically persist the global update-check consent as owner-only JSON.

    `_atomic_write_private` finishes with `os.replace`, so a concurrent reader
    observes either the old consent or the new one and never a torn file. That
    is the whole ordering guarantee this preference needs; there is deliberately
    no cross-process lock, which previously made revoking consent wait on an
    in-flight network request without making revocation any more prompt.
    """

    destination = path or preference_path()
    destination.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    try:
        destination.parent.chmod(0o700)
    except OSError:
        # Best effort for existing directories; file writes remain owner-only.
        pass
    document = {
        "schema_version": PREFERENCE_SCHEMA_VERSION,
        "enabled": bool(enabled),
    }
    _atomic_write_private(
        destination,
        json.dumps(document, sort_keys=True) + "\n",
    )
    return destination


def require_update_checks_enabled(path: Path | None = None) -> None:
    if update_checks_enabled(path):
        return
    raise AppError(
        "GitHub update checks are disabled",
        code="update_checks_disabled",
        hint=(
            "Enable them in Settings > Privacy or run "
            "`kassiber update --enable-checks`."
        ),
    )


def release_url_for_tag(tag: str) -> str:
    return f"{GITHUB_RELEASES_PAGE_URL}/tag/{quote(tag, safe='')}"


def current_release_channel() -> str:
    channel = str(packaged_build_info().get("channel") or "").strip()
    return "release" if channel == "release" else "prerelease"


def _release_from_response(payload: Any) -> dict[str, Any]:
    """Pick the highest published semantic version out of a releases payload."""

    if not isinstance(payload, list):
        raise ValueError("GitHub returned an invalid releases response")
    selected: tuple[dict[str, Any], ParsedVersion] | None = None
    for item in payload:
        if not isinstance(item, dict) or bool(item.get("draft")):
            continue
        tag = str(item.get("tag_name") or "").strip()
        parsed = parse_version(tag)
        if parsed is None:
            continue
        prerelease = bool(item.get("prerelease")) or bool(parsed.prerelease)
        candidate = {
            "latest_version": tag[1:] if tag.startswith("v") else tag,
            "release_tag": tag,
            "release_url": release_url_for_tag(tag),
            "prerelease": prerelease,
        }
        if selected is None or _compare_versions(parsed, selected[1]) > 0:
            selected = (candidate, parsed)
    if selected is not None:
        return selected[0]
    raise ValueError("GitHub did not return a valid Kassiber release")


def fetch_latest_release(
    *,
    opener: Callable[..., BinaryIO] = _open_without_redirects,
) -> dict[str, Any]:
    # Stable builds ask for the latest-stable object so a run of prereleases
    # cannot hide a stable update; that endpoint already excludes drafts and
    # prereleases and returns one release. Prerelease builds read the listing,
    # which GitHub returns newest-first, so the highest version Kassiber has
    # published is on the first page — there is nothing later to paginate to.
    stable = current_release_channel() == "release"
    api_url = GITHUB_LATEST_RELEASE_API_URL if stable else GITHUB_RELEASES_API_URL
    try:
        request = Request(
            api_url,
            headers={
                "Accept": "application/vnd.github+json",
                "User-Agent": f"kassiber/{current_version()}",
                "X-GitHub-Api-Version": "2022-11-28",
            },
            method="GET",
        )
        # The only writer of the "update" subsystem. Without this the egress
        # auditor's Update tile could never leave zero, which reads as proof
        # that no release check happened rather than as an unmonitored path.
        get_egress_ledger().record_url(
            api_url,
            subsystem="update",
            operation="http.request",
            method="GET",
        )
        with opener(request, timeout=NETWORK_TIMEOUT_SECONDS) as response:
            raw = response.read(_MAX_RESPONSE_BYTES + 1)
        if len(raw) > _MAX_RESPONSE_BYTES:
            raise ValueError("GitHub update response is too large")
        payload = json.loads(raw.decode("utf-8"))
        return _release_from_response([payload] if stable else payload)
    except (HTTPError, URLError, TimeoutError, OSError) as exc:
        raise AppError(
            "Could not check GitHub for a Kassiber update",
            code="update_check_failed",
            hint=f"Open {GITHUB_RELEASES_PAGE_URL} to check manually.",
            retryable=True,
        ) from exc
    except (UnicodeDecodeError, json.JSONDecodeError, ValueError) as exc:
        raise AppError(
            "GitHub returned an invalid Kassiber update response",
            code="update_check_failed",
            hint=f"Open {GITHUB_RELEASES_PAGE_URL} to check manually.",
            retryable=True,
        ) from exc


def _utc_now() -> datetime:
    return datetime.now(timezone.utc)


def _isoformat(value: datetime) -> str:
    return value.astimezone(timezone.utc).isoformat().replace("+00:00", "Z")


def _result_from_release(
    release: Mapping[str, Any],
    *,
    checked_at: datetime,
) -> dict[str, Any]:
    latest = str(release["latest_version"])
    current = current_version()
    result = {
        "current_version": current,
        "latest_version": latest,
        "update_available": is_newer_version(latest, current),
        "prerelease": bool(release.get("prerelease")),
        "release_url": str(release["release_url"]),
        "checked_at": _isoformat(checked_at),
    }
    install_method = detect_install_method()
    result["install_method"] = install_method
    result["update_command"] = update_command_for_method(install_method)
    return result


def _cache_document(result: Mapping[str, Any]) -> dict[str, Any]:
    return {
        "schema_version": CACHE_SCHEMA_VERSION,
        "latest_version": result["latest_version"],
        "prerelease": bool(result.get("prerelease")),
        "release_url": result["release_url"],
        "checked_at": result["checked_at"],
    }


def write_cache(result: Mapping[str, Any], path: Path | None = None) -> None:
    destination = path or cache_path()
    _atomic_write_private(
        destination,
        json.dumps(_cache_document(result), sort_keys=True) + "\n",
    )


def read_cache(path: Path | None = None) -> dict[str, Any] | None:
    source = path or cache_path()
    raw = read_small_private_file(source, _MAX_CACHE_BYTES)
    if raw is None:
        return None
    try:
        payload = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        return None
    if not _has_exact_schema_version(payload, CACHE_SCHEMA_VERSION):
        return None
    latest = str(payload.get("latest_version") or "").strip()
    release_url = str(payload.get("release_url") or "").strip()
    checked_at_text = str(payload.get("checked_at") or "").strip()
    allowed_release_urls = {
        release_url_for_tag(latest),
        release_url_for_tag(f"v{latest}"),
    }
    if parse_version(latest) is None or release_url not in allowed_release_urls:
        return None
    if current_release_channel() == "release" and bool(payload.get("prerelease")):
        return None
    try:
        checked_at = datetime.fromisoformat(checked_at_text.replace("Z", "+00:00"))
    except ValueError:
        return None
    if checked_at.tzinfo is None:
        return None
    return _result_from_release(
        {
            "latest_version": latest,
            "release_url": release_url,
            "prerelease": bool(payload.get("prerelease")),
        },
        checked_at=checked_at,
    )


def cache_is_stale(
    cached: Mapping[str, Any] | None,
    *,
    now: datetime | None = None,
) -> bool:
    if cached is None:
        return True
    try:
        checked_at = datetime.fromisoformat(
            str(cached["checked_at"]).replace("Z", "+00:00")
        )
    except (KeyError, ValueError):
        return True
    return checked_at < (now or _utc_now()) - CHECK_INTERVAL


def check_for_update(
    *,
    path: Path | None = None,
    preference: Path | None = None,
    opener: Callable[..., BinaryIO] = _open_without_redirects,
    now: datetime | None = None,
) -> dict[str, Any]:
    consent = preference or preference_path()
    require_update_checks_enabled(consent)
    result = _result_from_release(
        fetch_latest_release(opener=opener),
        checked_at=now or _utc_now(),
    )
    write_cache(result, path)
    return result


def detect_install_method(
    *,
    executable: str | None = None,
    argv0: str | None = None,
    environ: Mapping[str, str] | None = None,
) -> str:
    environment = os.environ if environ is None else environ
    explicit = str(environment.get(HOMEBREW_PACKAGE_ENV) or "").strip().lower()
    if explicit in {"cask", "formula"}:
        return f"homebrew_{explicit}"
    candidates = [executable or sys.executable, argv0 or sys.argv[0]]
    candidate_paths = [*candidates]
    for value in candidates:
        try:
            candidate_paths.append(str(Path(value).expanduser().resolve(strict=False)))
        except OSError:
            # The unresolved executable path is still included in candidates.
            pass
    normalized = "\n".join(
        value.replace("\\", "/").lower() for value in candidate_paths
    )
    if "/cellar/kassiber-cli/" in normalized:
        return "homebrew_formula"
    # Linux .deb/.rpm installs report "manual" on purpose: package ownership
    # alone cannot prove a signed Kassiber repository installed the package,
    # so until a live repository URL and archive-key fingerprint are pinned in
    # code, the only safe guidance is the GitHub release page.
    return "manual"


def update_command_for_method(method: str) -> str | None:
    if method == "homebrew_cask":
        return HOMEBREW_CASK_COMMAND
    if method == "homebrew_formula":
        return HOMEBREW_FORMULA_COMMAND
    return None


def supports_color(stream: TextIO, environ: Mapping[str, str] | None = None) -> bool:
    environment = os.environ if environ is None else environ
    return bool(
        getattr(stream, "isatty", lambda: False)()
        and environment.get("TERM", "") != "dumb"
        and "NO_COLOR" not in environment
        and environment.get("CLICOLOR") != "0"
    )


def render_update_status(result: Mapping[str, Any], *, color: bool) -> str:
    current = str(result["current_version"])
    latest = str(result["latest_version"])
    release_url = str(result["release_url"])
    if not bool(result.get("update_available")):
        return f"Kassiber {current} is current (latest: {latest}).\n"

    title = f"✨ Update available: Kassiber {current} → {latest}"
    command = result.get("update_command")
    if color:
        title = f"\033[1;36m{title}\033[0m"
    if isinstance(command, str) and command:
        instruction = f"Run {command} to update."
        if color:
            instruction = f"Run \033[36m{command}\033[0m to update."
    else:
        instruction = "Download and install the release manually."
    return f"{title}\n  {instruction}\n  Release notes: {release_url}\n"


def automatic_check_allowed(
    args: Any,
    *,
    preference: Path | None = None,
    stream: TextIO | None = None,
    stdout: TextIO | None = None,
) -> bool:
    output = stream or sys.stderr
    command_output = stdout or sys.stdout
    if not (
        bool(getattr(output, "isatty", lambda: False)())
        and bool(getattr(command_output, "isatty", lambda: False)())
    ):
        return False
    if not update_checks_enabled(preference):
        return False
    if not packaged_build_info():
        return False
    if os.environ.get("KASSIBER_OPERATOR_CHILD") == "1":
        return False
    return not (
        bool(getattr(args, "machine", False))
        or bool(getattr(args, "non_interactive", False))
        or bool(getattr(args, "output", None))
        or getattr(args, "format", "table") != "table"
        or getattr(args, "command", None) in {"daemon", "update", "verify-download"}
    )


def show_cached_update(
    args: Any,
    *,
    path: Path | None = None,
    preference: Path | None = None,
    stream: TextIO | None = None,
    stdout: TextIO | None = None,
) -> None:
    """Print a banner for an update an earlier authorized check already found.

    This reads only local state. `kassiber update` and the desktop's daily check
    write that cache; nothing here spawns a refresh, so an ordinary command can
    never turn into an outbound request.
    """

    output = stream or sys.stderr
    if not automatic_check_allowed(
        args,
        preference=preference,
        stream=output,
        stdout=stdout,
    ):
        return
    cached = read_cache(path)
    if cached is None or cache_is_stale(cached):
        return
    if bool(cached.get("update_available")):
        output.write(render_update_status(cached, color=supports_color(output)))
        output.write("\n")
        output.flush()
