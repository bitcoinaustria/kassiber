"""OS credential-store integration for opt-in CLI database unlock.

The CLI and desktop deliberately use different credential namespaces.  The
database passphrase is the same value, but enrollment, access policy, and
revocation are independent.  ``LEGACY_SHARED_PASSPHRASE_SERVICE`` remains
read-only migration input for installations created before that split.
"""

from __future__ import annotations

import secrets as py_secrets
import sys
from pathlib import Path

import keyring
from keyring.errors import PasswordDeleteError

from ..db import load_managed_settings, update_managed_settings


CLI_REMEMBERED_PASSPHRASE_SERVICE = "Kassiber CLI Database Passphrase"
LEGACY_SHARED_PASSPHRASE_SERVICE = "Kassiber Database Passphrase"
DESKTOP_BIOMETRIC_STALE_MARKER_SERVICE = "Kassiber Desktop Biometric Invalidated"
DESKTOP_BIOMETRY_CURRENT_SET_MARKER_SERVICE = (
    "Kassiber Desktop Biometric Enrollment (Current Set)"
)
DESKTOP_APPLICATION_GATE_MARKER_SERVICE = (
    "Kassiber Desktop Biometric Enrollment (Application Gate)"
)
CLI_REMEMBERED_UNLOCK_SETTING = "cli_remembered_unlock"
CLI_LEGACY_UNLOCK_QUARANTINED_SETTING = "cli_legacy_unlock_quarantined"
DESKTOP_BIOMETRIC_STALE_SETTING = "desktop_biometric_stale"

_ACCESS_POLICY_BY_PLATFORM = {
    "macos": "macos_keychain_application_acl",
    "windows": "windows_dpapi_user_scope",
    "linux": "linux_secret_service_session",
    "unsupported": "unsupported",
}


def _platform_name() -> str:
    if sys.platform == "darwin":
        return "macos"
    if sys.platform == "win32":
        return "windows"
    if sys.platform.startswith("linux"):
        return "linux"
    return "unsupported"


def remembered_unlock_access_policy() -> str:
    """Return the public-safe code describing the CLI credential boundary."""

    return _ACCESS_POLICY_BY_PLATFORM[_platform_name()]


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


def cli_legacy_unlock_quarantined(data_root) -> bool:
    """Return whether a retained legacy item is owned but unusable by CLI."""

    return load_managed_settings(data_root).get(
        CLI_LEGACY_UNLOCK_QUARANTINED_SETTING
    ) is True


def set_cli_unlock_state(
    data_root,
    *,
    enabled: bool,
    legacy_quarantined: bool,
) -> None:
    """Atomically update CLI opt-in and retained-legacy quarantine state."""

    updates = {}
    remove = []
    if enabled:
        updates[CLI_REMEMBERED_UNLOCK_SETTING] = True
    else:
        remove.append(CLI_REMEMBERED_UNLOCK_SETTING)
    if legacy_quarantined:
        updates[CLI_LEGACY_UNLOCK_QUARANTINED_SETTING] = True
    else:
        remove.append(CLI_LEGACY_UNLOCK_QUARANTINED_SETTING)
    update_managed_settings(data_root, updates=updates, remove=tuple(remove))


def _backend_priority(backend) -> float:
    try:
        return float(backend.priority)
    except Exception:
        return 0.0


def _backend_is_native(backend) -> bool:
    """Accept only the platform stores covered by Kassiber's threat model."""

    module = type(backend).__module__
    platform = _platform_name()
    allowed_module = {
        "macos": "keyring.backends.macOS",
        "windows": "keyring.backends.Windows",
        "linux": "keyring.backends.SecretService",
    }.get(platform)
    if allowed_module is None:
        return False
    if module == allowed_module:
        return _backend_priority(backend) > 0
    if module != "keyring.backends.chainer":
        return False
    active = [
        child
        for child in getattr(backend, "backends", ())
        if _backend_priority(child) > 0
    ]
    return bool(active) and all(_backend_is_native(child) for child in active)


def _native_keyring_available() -> bool:
    try:
        return _backend_is_native(keyring.get_keyring())
    except Exception:
        return False


def _load_service_with_availability(
    data_root,
    service: str,
) -> tuple[bool, str | None]:
    if not _native_keyring_available():
        return False, None
    try:
        value = keyring.get_password(
            service,
            remembered_unlock_account(data_root),
        )
    except Exception:
        return False, None
    return True, value if isinstance(value, str) and value else None


def _delete_service(data_root, service: str) -> bool:
    if not _native_keyring_available():
        return False
    try:
        keyring.delete_password(service, remembered_unlock_account(data_root))
    except PasswordDeleteError:
        # Windows and Secret Service use PasswordDeleteError for a missing
        # item, while macOS wraps every Security.framework deletion failure in
        # the same exception. Verify absence instead of turning an ACL/signing
        # failure into a false-success revocation result.
        pass
    except Exception:
        return False
    try:
        remaining = keyring.get_password(
            service,
            remembered_unlock_account(data_root),
        )
    except Exception:
        return False
    return remaining in (None, "")


def load_remembered_passphrase(data_root) -> str | None:
    """Load the CLI credential, migrating the old shared item when necessary.

    The explicit marker is checked here as well as by the runtime caller.  This
    keeps accidental direct callers from turning a desktop-only legacy item
    into implicit CLI enrollment.
    """

    if (
        not cli_remembered_unlock_enabled(data_root)
        or cli_legacy_unlock_quarantined(data_root)
    ):
        return None

    _available, passphrase = _load_service_with_availability(
        data_root,
        CLI_REMEMBERED_PASSPHRASE_SERVICE,
    )
    if passphrase is not None:
        return passphrase

    _available, legacy = _load_service_with_availability(
        data_root,
        LEGACY_SHARED_PASSPHRASE_SERVICE,
    )
    if legacy is None:
        return None

    # The CLI marker disambiguates old shared entries conservatively: when it
    # is enabled, the legacy item belongs to CLI migration.  Desktop biometric
    # enrollment must be re-established in its independently protected store.
    if store_remembered_passphrase(data_root, legacy):
        _delete_service(data_root, LEGACY_SHARED_PASSPHRASE_SERVICE)
    return legacy


def store_remembered_passphrase(data_root, passphrase) -> bool:
    """Store the CLI-only passphrase, returning False on OS-store rejection."""

    if not _native_keyring_available():
        return False
    try:
        keyring.set_password(
            CLI_REMEMBERED_PASSPHRASE_SERVICE,
            remembered_unlock_account(data_root),
            passphrase,
        )
    except Exception:
        return False
    return True


def mark_desktop_biometric_passphrase_stale(data_root) -> str | None:
    """Arm a cross-process stale guard before rotating the database key.

    Desktop Keychain items are protected by a per-binary ACL, so the Python
    sidecar cannot reliably probe marker passwords written by the Tauri binary.
    Managed settings are deliberately non-secret and readable by both sides.
    The desktop clears this guard only after it has refreshed or removed its
    own protected credential.
    """

    if _platform_name() != "macos":
        return None
    generation = py_secrets.token_urlsafe(32)
    update_managed_settings(
        data_root,
        updates={DESKTOP_BIOMETRIC_STALE_SETTING: generation},
    )
    return generation


def delete_remembered_passphrase(data_root) -> bool:
    """Delete only the CLI credential; a missing item is an idempotent success."""

    return _delete_service(data_root, CLI_REMEMBERED_PASSPHRASE_SERVICE)


def delete_legacy_shared_passphrase(data_root) -> bool:
    """Delete migration-only shared state after both consumers are separated."""

    return _delete_service(data_root, LEGACY_SHARED_PASSPHRASE_SERVICE)


def refresh_remembered_passphrase_after_rotation(
    data_root,
    passphrase,
) -> dict[str, object] | None:
    """Refresh an enrolled CLI credential after a successful database rekey.

    Failure disables CLI remembered unlock rather than leaving a credential
    that is known to be stale.  The database rotation itself remains complete.
    """

    if not cli_remembered_unlock_enabled(data_root):
        return None
    if store_remembered_passphrase(data_root, passphrase):
        return None

    credential_deleted = delete_remembered_passphrase(data_root)
    legacy_credential_deleted = delete_legacy_shared_passphrase(data_root)
    marker_cleared = False
    legacy_quarantined = False
    if legacy_credential_deleted:
        try:
            set_cli_unlock_state(
                data_root,
                enabled=False,
                legacy_quarantined=False,
            )
            marker_cleared = True
        except OSError:
            # The rotation already succeeded. Report marker_cleared=False so
            # callers can surface the non-fatal managed-settings failure.
            pass
    else:
        try:
            set_cli_unlock_state(
                data_root,
                enabled=False,
                legacy_quarantined=True,
            )
            marker_cleared = True
            legacy_quarantined = True
        except OSError:
            # The rotation already succeeded. Report marker/quarantine state
            # conservatively instead of turning cleanup into a rekey failure.
            pass
    return {
        "code": "remembered_unlock_update_failed",
        "message": (
            "The database passphrase changed, but the CLI remembered-unlock "
            "copy could not be updated safely. Review the cleanup fields and "
            "re-enroll after removing any retained legacy credential."
        ),
        "credential_deleted": credential_deleted,
        "legacy_credential_deleted": legacy_credential_deleted,
        "legacy_quarantined": legacy_quarantined,
        "cli_marker_cleared": marker_cleared,
    }


def remembered_unlock_status(data_root) -> dict[str, object]:
    """Return public-safe platform, availability, enrollment, and opt-in state."""

    cli_enabled = cli_remembered_unlock_enabled(data_root)
    legacy_quarantined = cli_legacy_unlock_quarantined(data_root)
    if cli_enabled and not legacy_quarantined:
        available, passphrase = _load_service_with_availability(
            data_root,
            CLI_REMEMBERED_PASSPHRASE_SERVICE,
        )
        if passphrase is None:
            legacy_available, legacy = _load_service_with_availability(
                data_root,
                LEGACY_SHARED_PASSPHRASE_SERVICE,
            )
            available = available and legacy_available
            if legacy is not None:
                if store_remembered_passphrase(data_root, legacy):
                    _delete_service(data_root, LEGACY_SHARED_PASSPHRASE_SERVICE)
                passphrase = legacy
    else:
        # A desktop enrollment is private to the desktop until the CLI marker
        # is explicitly enabled. Status must not probe that secret merely to
        # report whether the native backend is installed.
        available, passphrase = _native_keyring_available(), None
    return {
        "platform": _platform_name(),
        "access_policy": remembered_unlock_access_policy(),
        "available": available,
        "configured": passphrase is not None,
        "cli_enabled": cli_enabled,
        "legacy_quarantined": legacy_quarantined,
    }
