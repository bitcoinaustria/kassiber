"""Minimal GitHub release checks shared by the human-facing CLI surfaces.

Automatic checks mirror Codex's low-friction pattern: display a previously
cached result immediately, refresh stale metadata in a detached process, and
show the new result on a later invocation.  The updater never downloads or
executes a release; it only prints a trusted release URL or a package-manager
command for an install method Kassiber can prove locally.
"""

from __future__ import annotations

import json
import os
import re
import stat
import subprocess
import sys
import tempfile
from contextlib import contextmanager
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
from .errors import AppError


GITHUB_RELEASES_API_URL = (
    "https://api.github.com/repos/bitcoinaustria/kassiber/releases?per_page=10"
)
GITHUB_LATEST_RELEASE_API_URL = (
    "https://api.github.com/repos/bitcoinaustria/kassiber/releases/latest"
)
GITHUB_RELEASES_PAGE_URL = "https://github.com/bitcoinaustria/kassiber/releases"
CHECK_INTERVAL = timedelta(hours=20)
FAILURE_RETRY_INTERVAL = timedelta(hours=1)
NETWORK_TIMEOUT_SECONDS = 5.0
CACHE_SCHEMA_VERSION = 1
CACHE_FILENAME = "update-check.json"
PREFERENCE_SCHEMA_VERSION = 1
PREFERENCE_FILENAME = "update-checks.json"
PREFERENCE_LOCK_FILENAME = "update-checks.lock"
INTERNAL_REFRESH_ARGUMENT = "--refresh-update-cache"
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
_MAX_RESPONSE_BYTES = 256 * 1024
_MAX_PREFERENCE_BYTES = 1024
_MAX_CACHE_BYTES = 8 * 1024
_MAX_REFRESH_ATTEMPT_BYTES = 64
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


def preference_lock_path(path: Path | None = None) -> Path:
    preference = path or preference_path()
    return preference.with_name(PREFERENCE_LOCK_FILENAME)


@contextmanager
def _update_check_preference_lock(path: Path):
    """Serialize consent reads/writes with the native Rust update checker."""

    lock_path = preference_lock_path(path)
    try:
        try:
            if stat.S_ISLNK(os.lstat(lock_path).st_mode):
                raise OSError(f"Update-check lock must not be a symlink: {lock_path}")
        except FileNotFoundError:
            # Expected on first use; os.open below creates the lock safely.
            pass
        flags = (
            os.O_RDWR
            | os.O_CREAT
            | getattr(os, "O_BINARY", 0)
            | getattr(os, "O_CLOEXEC", 0)
            | getattr(os, "O_NONBLOCK", 0)
        )
        if hasattr(os, "O_NOFOLLOW"):
            flags |= os.O_NOFOLLOW
        descriptor = os.open(lock_path, flags, 0o600)
        try:
            info = os.fstat(descriptor)
            if not stat.S_ISREG(info.st_mode):
                raise OSError(
                    f"Update-check lock must be a regular file: {lock_path}"
                )
            if os.name == "nt":
                import msvcrt

                if info.st_size == 0:
                    os.write(descriptor, b"\0")
                os.lseek(descriptor, 0, os.SEEK_SET)
                msvcrt.locking(descriptor, msvcrt.LK_LOCK, 1)
            else:
                import fcntl

                os.fchmod(descriptor, 0o600)
                fcntl.flock(descriptor, fcntl.LOCK_EX)
        except BaseException:
            os.close(descriptor)
            raise
    except OSError as exc:
        raise _update_check_lock_error(exc) from exc
    try:
        yield
    finally:
        try:
            if os.name == "nt":
                import msvcrt

                os.lseek(descriptor, 0, os.SEEK_SET)
                msvcrt.locking(descriptor, msvcrt.LK_UNLCK, 1)
            else:
                import fcntl

                fcntl.flock(descriptor, fcntl.LOCK_UN)
        finally:
            os.close(descriptor)


def _update_check_lock_error(_exc: OSError) -> AppError:
    return AppError(
        "Could not safely coordinate the update-check permission",
        code="update_check_lock_failed",
        hint="Check the owner and permissions of ~/.kassiber/config/update-checks.lock.",
        retryable=True,
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
    """Atomically persist the global update-check consent as owner-only JSON."""

    destination = path or preference_path()
    destination.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    try:
        destination.parent.chmod(0o700)
    except OSError:
        # Best effort for existing directories; file writes remain owner-only.
        pass
    with _update_check_preference_lock(destination):
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


def _release_from_response(
    payload: Any,
    *,
    channel: str = "prerelease",
) -> dict[str, Any]:
    if channel == "release" and isinstance(payload, dict):
        payload = [payload]
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
        if channel == "release" and prerelease:
            continue
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
    channel = current_release_channel()
    api_url = (
        GITHUB_LATEST_RELEASE_API_URL
        if channel == "release"
        else GITHUB_RELEASES_API_URL
    )
    request = Request(
        api_url,
        headers={
            "Accept": "application/vnd.github+json",
            "User-Agent": f"kassiber/{current_version()}",
            "X-GitHub-Api-Version": "2022-11-28",
        },
        method="GET",
    )
    try:
        with opener(request, timeout=NETWORK_TIMEOUT_SECONDS) as response:
            raw = response.read(_MAX_RESPONSE_BYTES + 1)
    except (HTTPError, URLError, TimeoutError, OSError) as exc:
        raise AppError(
            "Could not check GitHub for a Kassiber update",
            code="update_check_failed",
            hint=f"Open {GITHUB_RELEASES_PAGE_URL} to check manually.",
            retryable=True,
        ) from exc
    if len(raw) > _MAX_RESPONSE_BYTES:
        raise AppError(
            "GitHub returned an unexpectedly large update response",
            code="update_check_failed",
            hint=f"Open {GITHUB_RELEASES_PAGE_URL} to check manually.",
            retryable=True,
        )
    try:
        return _release_from_response(
            json.loads(raw.decode("utf-8")),
            channel=channel,
        )
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


def _refresh_attempt_path(path: Path | None = None) -> Path:
    destination = path or cache_path()
    return destination.with_suffix(f"{destination.suffix}.attempt")


def _read_refresh_attempt(path: Path | None = None) -> datetime | None:
    raw = read_small_private_file(
        _refresh_attempt_path(path),
        _MAX_REFRESH_ATTEMPT_BYTES,
    )
    if raw is None:
        return None
    try:
        value = datetime.fromisoformat(
            raw.decode("utf-8").strip().replace("Z", "+00:00")
        )
    except (UnicodeDecodeError, ValueError):
        return None
    return value if value.tzinfo is not None else None


def _write_refresh_attempt(
    path: Path | None = None,
    *,
    now: datetime | None = None,
) -> None:
    _atomic_write_private(
        _refresh_attempt_path(path),
        f"{_isoformat(now or _utc_now())}\n",
    )


def automatic_refresh_due(
    cached: Mapping[str, Any] | None,
    *,
    path: Path | None = None,
    now: datetime | None = None,
) -> bool:
    current_time = now or _utc_now()
    if not cache_is_stale(cached, now=current_time):
        return False
    attempted_at = _read_refresh_attempt(path)
    return attempted_at is None or attempted_at < current_time - FAILURE_RETRY_INTERVAL


def check_for_update(
    *,
    path: Path | None = None,
    preference: Path | None = None,
    opener: Callable[..., BinaryIO] = _open_without_redirects,
    now: datetime | None = None,
) -> dict[str, Any]:
    consent = preference or preference_path()
    require_update_checks_enabled(consent)
    with _update_check_preference_lock(consent):
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


def _background_command() -> list[str]:
    if getattr(sys, "frozen", False):
        return [sys.executable, INTERNAL_REFRESH_ARGUMENT]
    return [sys.executable, "-m", "kassiber", INTERNAL_REFRESH_ARGUMENT]


_BACKGROUND_ENV_NAMES = frozenset(
    {
        "ALL_PROXY",
        "COMSPEC",
        "CURL_CA_BUNDLE",
        "DYLD_FALLBACK_LIBRARY_PATH",
        "DYLD_LIBRARY_PATH",
        "HOME",
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "LANG",
        "LANGUAGE",
        "LD_LIBRARY_PATH",
        "LOCALAPPDATA",
        "NO_PROXY",
        "PATH",
        "PATHEXT",
        "REQUESTS_CA_BUNDLE",
        "SSL_CERT_DIR",
        "SSL_CERT_FILE",
        "SYSTEMROOT",
        "TEMP",
        "TMP",
        "TMPDIR",
        "TZ",
        "USERPROFILE",
        "WINDIR",
        "_MEIPASS2",
        "all_proxy",
        "https_proxy",
        "http_proxy",
        "no_proxy",
    }
)


def _background_environment(
    environ: Mapping[str, str] | None = None,
) -> dict[str, str]:
    """Keep OS/runtime/proxy state without forwarding unrelated app secrets."""

    source = os.environ if environ is None else environ
    environment = {
        key: value
        for key, value in source.items()
        if key in _BACKGROUND_ENV_NAMES
        or key.startswith("LC_")
        or key.startswith("_PYI_")
    }
    return environment


def start_background_refresh(
    path: Path | None = None,
    preference: Path | None = None,
) -> None:
    destination = path or cache_path()
    consent = preference or preference_path()
    if not update_checks_enabled(consent):
        return
    environment = _background_environment()
    environment[UPDATE_CACHE_ENV] = str(destination)
    environment[UPDATE_PREFERENCE_ENV] = str(consent)
    try:
        _write_refresh_attempt(destination)
    except OSError:
        # Without the private marker, later CLI runs cannot enforce the retry
        # interval. Fail closed instead of creating an update-check storm.
        return
    kwargs: dict[str, Any] = {
        "stdin": subprocess.DEVNULL,
        "stdout": subprocess.DEVNULL,
        "stderr": subprocess.DEVNULL,
        "close_fds": True,
        "env": environment,
    }
    if os.name == "nt":
        kwargs["creationflags"] = (
            getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0)
            | getattr(subprocess, "DETACHED_PROCESS", 0)
        )
    else:
        kwargs["start_new_session"] = True
    try:
        subprocess.Popen(_background_command(), **kwargs)
    except OSError:
        return


def show_cached_update_and_refresh(
    args: Any,
    *,
    path: Path | None = None,
    preference: Path | None = None,
    stream: TextIO | None = None,
    stdout: TextIO | None = None,
) -> None:
    output = stream or sys.stderr
    if not automatic_check_allowed(
        args,
        preference=preference,
        stream=output,
        stdout=stdout,
    ):
        return
    cached = read_cache(path)
    if cached is not None and bool(cached.get("update_available")):
        output.write(render_update_status(cached, color=supports_color(output)))
        output.write("\n")
        output.flush()
    if automatic_refresh_due(cached, path=path):
        start_background_refresh(path, preference)


def refresh_cache_silently(
    path: Path | None = None,
    preference: Path | None = None,
) -> None:
    # Concurrency control is the refresh-attempt throttle written by the
    # parent before spawning this child: a racing sibling costs at most one
    # extra bounded GET racing an atomic cache replace, which is harmless.
    destination = path or cache_path()
    consent = preference or preference_path()
    if not update_checks_enabled(consent):
        return
    try:
        check_for_update(path=destination, preference=consent)
    except (AppError, OSError):
        return
