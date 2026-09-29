"""Whether Kassiber may open outbound connections on this machine.

Two switches block egress at the shared transport boundaries:

- ``KASSIBER_NO_EGRESS=1``, the operator's process override, and
- offline mode, the user's persisted choice (Settings, or the connection
  panel's switch), in the manner of Sparrow Wallet's "Offline".

Offline mode lives in a small owner-only file under the state root's config
directory, next to the update-check consent, so the desktop daemon and every
CLI invocation read the same choice before opening a connection, and it holds
from the first request after launch rather than once a UI has loaded. A file
that exists but cannot be read counts as offline: the safe default for a
privacy switch is the one that sends nothing.
"""

from __future__ import annotations

import ipaddress
import json
import os
from pathlib import Path
from urllib.parse import urlsplit

from .db import DEFAULT_CONFIG_DIRNAME, default_state_root
from .errors import AppError
from .private_files import atomic_write_private, read_small_private_file

NO_EGRESS_ENV = "KASSIBER_NO_EGRESS"
OFFLINE_PREFERENCE_ENV = "KASSIBER_OFFLINE_PREFERENCE_FILE"
OFFLINE_FILENAME = "offline-mode.json"
OFFLINE_SCHEMA_VERSION = 1
_MAX_PREFERENCE_BYTES = 4096
_TRUTHY = {"1", "true", "yes", "on"}

REASON_ENVIRONMENT = "environment"
REASON_OFFLINE_MODE = "offline_mode"


def preference_path() -> Path:
    override = os.environ.get(OFFLINE_PREFERENCE_ENV)
    if override:
        return Path(override).expanduser()
    return default_state_root() / DEFAULT_CONFIG_DIRNAME / OFFLINE_FILENAME


def offline_mode_enabled(path: Path | None = None) -> bool:
    destination = path or preference_path()
    try:
        exists = destination.exists() or destination.is_symlink()
    except OSError:
        return True
    if not exists:
        return False
    raw = read_small_private_file(destination, _MAX_PREFERENCE_BYTES)
    if raw is None:
        return True
    try:
        payload = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        return True
    if (
        not isinstance(payload, dict)
        or payload.get("schema_version") != OFFLINE_SCHEMA_VERSION
        or type(payload.get("enabled")) is not bool
    ):
        return True
    return payload["enabled"]


def set_offline_mode(enabled: bool, path: Path | None = None) -> Path:
    destination = path or preference_path()
    destination.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    try:
        destination.parent.chmod(0o700)
    except OSError:
        # Best effort for existing directories; the file itself is owner-only.
        pass
    document = {"schema_version": OFFLINE_SCHEMA_VERSION, "enabled": bool(enabled)}
    atomic_write_private(destination, json.dumps(document, sort_keys=True) + "\n")
    return destination


def _environment_blocks_egress() -> bool:
    return str(os.environ.get(NO_EGRESS_ENV) or "").strip().lower() in _TRUTHY


def egress_block_reason() -> str | None:
    """Why outbound connections are blocked right now, or ``None``."""
    if _environment_blocks_egress():
        return REASON_ENVIRONMENT
    if offline_mode_enabled():
        return REASON_OFFLINE_MODE
    return None


def offline_status() -> dict[str, bool]:
    """The switch state the desktop shows, without contacting anything.

    ``offline`` is the user's switch alone. ``environment_blocked`` reports
    ``KASSIBER_NO_EGRESS``, which covers fewer paths than the switch, so the
    desktop keeps the switch usable while it is set.
    """
    return {
        "offline": offline_mode_enabled(),
        "environment_blocked": _environment_blocks_egress(),
    }


def _offline_error(subject: str) -> AppError:
    return AppError(
        f"{subject} disabled: Kassiber is in offline mode",
        code="network_egress_disabled",
        hint="Turn off offline mode to connect.",
        retryable=False,
    )


def egress_disabled_error(subject: str = "Outbound requests are") -> AppError | None:
    """The error to raise while egress is blocked; ``subject`` carries its verb."""
    reason = egress_block_reason()
    if reason is None:
        return None
    if reason == REASON_OFFLINE_MODE:
        return _offline_error(subject)
    return AppError(
        f"{subject} disabled by {NO_EGRESS_ENV}",
        code="network_egress_disabled",
        retryable=False,
    )


def require_egress_enabled(subject: str = "Outbound requests are") -> None:
    """Raise before opening a transport while egress is blocked."""
    error = egress_disabled_error(subject)
    if error is not None:
        raise error


def _is_loopback_url(url: str) -> bool:
    try:
        parsed = urlsplit(url)
    except ValueError:
        return False
    if parsed.scheme not in {"http", "https"}:
        return False
    host = (parsed.hostname or "").rstrip(".").lower()
    if host == "localhost":
        return True
    try:
        return ipaddress.ip_address(host).is_loopback
    except ValueError:
        return False


def require_online(subject: str, *, on_device_url: str | None = None) -> None:
    """Refuse a connection outside the shared transport while offline.

    For the paths ``KASSIBER_NO_EGRESS`` does not cover by design — AI
    providers, LAN device sync and update checks — offline mode still has to
    hold. ``on_device_url`` is the base URL of an AI provider already marked
    local; it passes only on loopback. A loopback URL alone is not enough: a
    remote or TEE provider behind a local gateway or tunnel would still send
    the request off the machine.
    """
    if not offline_mode_enabled():
        return
    if on_device_url is not None and _is_loopback_url(on_device_url):
        return
    raise _offline_error(subject)
