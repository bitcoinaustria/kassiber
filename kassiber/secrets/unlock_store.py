"""OS credential-store integration for opt-in CLI database unlock.

The service and account namespace intentionally match the desktop shell's
Touch ID enrollment. The non-secret CLI opt-in marker remains in the managed
settings file so a desktop-only enrollment is never consumed implicitly.
"""

from __future__ import annotations

from pathlib import Path
import sys

import keyring
from keyring.errors import PasswordDeleteError

from ..db import load_managed_settings, update_managed_settings


TOUCH_ID_PASSPHRASE_SERVICE = "Kassiber Database Passphrase"
CLI_REMEMBERED_UNLOCK_SETTING = "cli_remembered_unlock"


def _platform_name() -> str:
    if sys.platform == "darwin":
        return "macos"
    if sys.platform == "win32":
        return "windows"
    if sys.platform.startswith("linux"):
        return "linux"
    return "unsupported"


def remembered_unlock_account(data_root) -> str:
    """Derive the desktop-compatible per-data-root credential account."""

    selected = Path(data_root)
    try:
        return str(selected.resolve())
    except (OSError, RuntimeError):
        return str(selected)


def cli_remembered_unlock_enabled(data_root) -> bool:
    """Return True only for the explicit non-secret CLI opt-in marker."""

    return load_managed_settings(data_root).get(CLI_REMEMBERED_UNLOCK_SETTING) is True


def set_cli_remembered_unlock_enabled(data_root, enabled: bool) -> None:
    """Set or clear the explicit CLI opt-in marker."""

    if enabled:
        update_managed_settings(
            data_root,
            updates={CLI_REMEMBERED_UNLOCK_SETTING: True},
        )
    else:
        update_managed_settings(
            data_root,
            remove=(CLI_REMEMBERED_UNLOCK_SETTING,),
        )


def _load_with_availability(data_root) -> tuple[bool, str | None]:
    if _platform_name() == "unsupported":
        return False, None
    try:
        value = keyring.get_password(
            TOUCH_ID_PASSPHRASE_SERVICE,
            remembered_unlock_account(data_root),
        )
    except Exception:
        return False, None
    return True, value if isinstance(value, str) and value else None


def load_remembered_passphrase(data_root) -> str | None:
    """Load the shared OS-store passphrase, degrading every store error to None."""

    _available, passphrase = _load_with_availability(data_root)
    return passphrase


def store_remembered_passphrase(data_root, passphrase) -> bool:
    """Store the shared passphrase, returning False when the OS store rejects it."""

    if _platform_name() == "unsupported":
        return False
    try:
        keyring.set_password(
            TOUCH_ID_PASSPHRASE_SERVICE,
            remembered_unlock_account(data_root),
            passphrase,
        )
    except Exception:
        return False
    return True


def delete_remembered_passphrase(data_root) -> bool:
    """Delete the shared passphrase; a missing item is an idempotent success."""

    if _platform_name() == "unsupported":
        return False
    try:
        keyring.delete_password(
            TOUCH_ID_PASSPHRASE_SERVICE,
            remembered_unlock_account(data_root),
        )
    except PasswordDeleteError:
        return True
    except Exception:
        return False
    return True


def remembered_unlock_status(data_root) -> dict[str, object]:
    """Return public-safe platform, availability, enrollment, and opt-in state."""

    available, passphrase = _load_with_availability(data_root)
    return {
        "platform": _platform_name(),
        "available": available,
        "configured": passphrase is not None,
        "cli_enabled": cli_remembered_unlock_enabled(data_root),
    }
