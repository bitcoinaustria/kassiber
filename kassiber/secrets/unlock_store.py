"""OS credential-store integration for opt-in CLI database unlock.

The CLI and desktop deliberately use different credential namespaces.  The
database passphrase is the same value, but enrollment, access policy, and
revocation are independent.  ``LEGACY_SHARED_PASSPHRASE_SERVICE`` remains
read-only migration input for installations created before that split.
"""

from __future__ import annotations

import hashlib
import secrets as py_secrets
import sys
from pathlib import Path

import keyring
from keyring.errors import PasswordDeleteError

from ..db import (
    load_managed_settings,
    resolve_canonical_project_data_root,
    update_managed_settings,
)
from ..errors import AppError
from ..operator.policy import (
    bind_project_policy,
    require_project_policy_binding,
    stored_project_policy_database_identity,
)


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
CLI_REMEMBERED_UNLOCK_ENROLLMENT_SETTING = "cli_remembered_unlock_enrollment"
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


def remembered_unlock_account(
    data_root,
    *,
    database_identity: str | None = None,
    enrollment_id: str | None = None,
    allow_stale_binding: bool = False,
) -> str:
    """Derive the CLI credential account from an authenticated database ID."""

    if database_identity is None:
        if allow_stale_binding:
            database_identity = stored_project_policy_database_identity(data_root)
        else:
            database_identity = require_project_policy_binding(
                data_root
            ).database_identity
    canonical_root = resolve_canonical_project_data_root(data_root)
    if enrollment_id is None:
        enrollment_id = load_managed_settings(canonical_root).get(
            CLI_REMEMBERED_UNLOCK_ENROLLMENT_SETTING
        )
    if database_identity is None or not _valid_enrollment_id(enrollment_id):
        raise AppError(
            "the remembered credential is not bound to a project database",
            code="operator_policy_binding_required",
            retryable=False,
        )
    return hashlib.sha256(
        (
            "kassiber-cli-remembered-unlock-v2:"
            f"{database_identity}:{enrollment_id}"
        ).encode("utf-8")
    ).hexdigest()


def _service_account(
    data_root,
    service: str,
    *,
    database_identity: str | None = None,
    enrollment_id: str | None = None,
    cleanup: bool = False,
) -> str:
    if service == LEGACY_SHARED_PASSPHRASE_SERVICE:
        return str(resolve_canonical_project_data_root(data_root))
    return remembered_unlock_account(
        data_root,
        database_identity=database_identity,
        enrollment_id=enrollment_id,
        allow_stale_binding=cleanup,
    )


def _legacy_path_account(data_root) -> str:
    return str(resolve_canonical_project_data_root(data_root))


def cli_remembered_unlock_enabled(data_root) -> bool:
    """Return True only for the explicit non-secret CLI opt-in marker."""

    canonical_root = resolve_canonical_project_data_root(data_root)
    return (
        load_managed_settings(canonical_root).get(CLI_REMEMBERED_UNLOCK_SETTING)
        is True
    )


def cli_remembered_v2_enrolled(data_root) -> bool:
    """Return whether cleanup can address a stored DB-bound CLI account."""

    canonical_root = resolve_canonical_project_data_root(data_root)
    settings = load_managed_settings(canonical_root)
    return (
        settings.get(CLI_REMEMBERED_UNLOCK_SETTING) is True
        and stored_project_policy_database_identity(canonical_root) is not None
        and _valid_enrollment_id(
            settings.get(CLI_REMEMBERED_UNLOCK_ENROLLMENT_SETTING)
        )
    )


def set_cli_remembered_unlock_enabled(
    data_root,
    enabled: bool,
    *,
    database_identity: str | None = None,
    enrollment_id: str | None = None,
    expected_project_identity: str | None = None,
) -> None:
    """Set or clear the explicit CLI opt-in marker."""

    data_root = resolve_canonical_project_data_root(data_root)
    if enabled:
        if database_identity is not None:
            if not _valid_enrollment_id(enrollment_id):
                raise AppError(
                    "remembered unlock enrollment is invalid",
                    code="operator_policy_binding_required",
                    retryable=False,
                )
            bind_project_policy(
                data_root,
                database_identity,
                expected_project_identity=expected_project_identity,
                updates={
                    CLI_REMEMBERED_UNLOCK_SETTING: True,
                    CLI_REMEMBERED_UNLOCK_ENROLLMENT_SETTING: enrollment_id,
                    "operator_unlock_mode": "unattended",
                },
            )
            return
        else:
            require_project_policy_binding(data_root)
        update_managed_settings(
            data_root,
            updates={
                CLI_REMEMBERED_UNLOCK_SETTING: True,
                "operator_unlock_mode": "unattended",
            },
        )
    else:
        update_managed_settings(
            data_root,
            remove=(CLI_REMEMBERED_UNLOCK_SETTING,),
        )


def cli_legacy_unlock_quarantined(data_root) -> bool:
    """Return whether a retained legacy item is owned but unusable by CLI."""

    return load_managed_settings(resolve_canonical_project_data_root(data_root)).get(
        CLI_LEGACY_UNLOCK_QUARANTINED_SETTING
    ) is True


def set_cli_unlock_state(
    data_root,
    *,
    enabled: bool,
    legacy_quarantined: bool,
    database_identity: str | None = None,
    enrollment_id: str | None = None,
    expected_project_identity: str | None = None,
) -> None:
    """Atomically update CLI opt-in and retained-legacy quarantine state."""

    data_root = resolve_canonical_project_data_root(data_root)
    updates = {}
    remove = []
    if enabled:
        if database_identity is not None:
            if not _valid_enrollment_id(enrollment_id):
                raise AppError(
                    "remembered unlock enrollment is invalid",
                    code="operator_policy_binding_required",
                    retryable=False,
                )
            updates[CLI_REMEMBERED_UNLOCK_SETTING] = True
            updates[CLI_REMEMBERED_UNLOCK_ENROLLMENT_SETTING] = enrollment_id
            updates["operator_unlock_mode"] = "unattended"
            if legacy_quarantined:
                updates[CLI_LEGACY_UNLOCK_QUARANTINED_SETTING] = True
            else:
                remove.append(CLI_LEGACY_UNLOCK_QUARANTINED_SETTING)
            bind_project_policy(
                data_root,
                database_identity,
                expected_project_identity=expected_project_identity,
                updates=updates,
                remove=tuple(remove),
            )
            return
        else:
            require_project_policy_binding(data_root)
        updates[CLI_REMEMBERED_UNLOCK_SETTING] = True
        updates["operator_unlock_mode"] = "unattended"
    else:
        remove.append(CLI_REMEMBERED_UNLOCK_SETTING)
    if legacy_quarantined:
        updates[CLI_LEGACY_UNLOCK_QUARANTINED_SETTING] = True
    else:
        remove.append(CLI_LEGACY_UNLOCK_QUARANTINED_SETTING)
    update_managed_settings(data_root, updates=updates, remove=tuple(remove))


def remembered_unlock_database_identity(data_root) -> str:
    """Return the authenticated DB identity pinned to remembered unlock."""

    return require_project_policy_binding(data_root).database_identity


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
            _service_account(data_root, service),
        )
    except Exception:
        return False, None
    return True, value if isinstance(value, str) and value else None


def _delete_service(
    data_root,
    service: str,
    *,
    database_identity: str | None = None,
    enrollment_id: str | None = None,
    legacy_cli: bool = False,
) -> bool:
    if not _native_keyring_available():
        return False
    try:
        account = (
            _legacy_path_account(data_root)
            if legacy_cli
            else _service_account(
                data_root,
                service,
                database_identity=database_identity,
                enrollment_id=enrollment_id,
                cleanup=True,
            )
        )
        keyring.delete_password(service, account)
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
            account,
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

    require_project_policy_binding(data_root)

    _available, passphrase = _load_service_with_availability(
        data_root,
        CLI_REMEMBERED_PASSPHRASE_SERVICE,
    )
    if passphrase is not None:
        return passphrase

    # Unbound path-keyed legacy credentials are never tried automatically.
    # Password re-enrollment writes a DB-bound v2 account before deleting them.
    return None


def store_remembered_passphrase(
    data_root,
    passphrase,
    *,
    database_identity: str | None = None,
    enrollment_id: str | None = None,
) -> bool:
    """Store the CLI-only passphrase, returning False on OS-store rejection."""

    if not _native_keyring_available():
        return False
    try:
        keyring.set_password(
            CLI_REMEMBERED_PASSPHRASE_SERVICE,
            _service_account(
                data_root,
                CLI_REMEMBERED_PASSPHRASE_SERVICE,
                database_identity=database_identity,
                enrollment_id=enrollment_id,
            ),
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
        resolve_canonical_project_data_root(data_root),
        updates={DESKTOP_BIOMETRIC_STALE_SETTING: generation},
    )
    return generation


def delete_remembered_passphrase(
    data_root,
    *,
    database_identity: str | None = None,
    enrollment_id: str | None = None,
) -> bool:
    """Delete only the CLI credential; a missing item is an idempotent success."""

    return _delete_service(
        data_root,
        CLI_REMEMBERED_PASSPHRASE_SERVICE,
        database_identity=database_identity,
        enrollment_id=enrollment_id,
    )


def delete_legacy_cli_remembered_passphrase(data_root) -> bool:
    """Delete the pre-binding CLI service account at the canonical path."""

    return _delete_service(
        data_root,
        CLI_REMEMBERED_PASSPHRASE_SERVICE,
        legacy_cli=True,
    )


def delete_legacy_shared_passphrase(data_root) -> bool:
    """Delete migration-only shared state after both consumers are separated."""

    return _delete_service(data_root, LEGACY_SHARED_PASSPHRASE_SERVICE)


def enable_remembered_unlock_authenticated(
    data_root,
    *,
    database_identity: str,
    enrollment_id: str,
    expected_project_identity: str,
) -> None:
    """Atomically bind and enable the complete unattended policy."""

    if not _valid_enrollment_id(enrollment_id):
        raise AppError(
            "remembered unlock enrollment is invalid",
            code="operator_policy_binding_required",
            retryable=False,
        )
    bind_project_policy(
        data_root,
        database_identity,
        expected_project_identity=expected_project_identity,
        updates={
            CLI_REMEMBERED_UNLOCK_SETTING: True,
            CLI_REMEMBERED_UNLOCK_ENROLLMENT_SETTING: enrollment_id,
            "operator_unlock_mode": "unattended",
        },
        remove=(CLI_LEGACY_UNLOCK_QUARANTINED_SETTING,),
    )


def disable_remembered_unlock(data_root, *, legacy_quarantined: bool) -> None:
    """Atomically force manual mode and disable every CLI unlock marker."""

    data_root = resolve_canonical_project_data_root(data_root)
    updates: dict[str, object] = {"operator_unlock_mode": "manual"}
    if legacy_quarantined:
        updates[CLI_LEGACY_UNLOCK_QUARANTINED_SETTING] = True
    remove = [
        CLI_REMEMBERED_UNLOCK_SETTING,
        CLI_REMEMBERED_UNLOCK_ENROLLMENT_SETTING,
    ]
    if not legacy_quarantined:
        remove.append(CLI_LEGACY_UNLOCK_QUARANTINED_SETTING)
    update_managed_settings(data_root, updates=updates, remove=tuple(remove))


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
    legacy_cli_credential_deleted = delete_legacy_cli_remembered_passphrase(data_root)
    legacy_shared_credential_deleted = delete_legacy_shared_passphrase(data_root)
    marker_cleared = False
    legacy_quarantined = not (
        legacy_cli_credential_deleted and legacy_shared_credential_deleted
    )
    try:
        disable_remembered_unlock(
            data_root,
            legacy_quarantined=legacy_quarantined,
        )
        marker_cleared = True
    except OSError:
        # The rotation already succeeded. Report marker_cleared=False so
        # callers can surface the non-fatal managed-settings failure.
        pass
    return {
        "code": "remembered_unlock_update_failed",
        "message": (
            "The database passphrase changed, but the CLI remembered-unlock "
            "copy could not be updated safely. Review the cleanup fields and "
            "re-enroll after removing any retained legacy credential."
        ),
        "credential_deleted": credential_deleted,
        "legacy_cli_credential_deleted": legacy_cli_credential_deleted,
        "legacy_shared_credential_deleted": legacy_shared_credential_deleted,
        "legacy_quarantined": legacy_quarantined,
        "cli_marker_cleared": marker_cleared,
    }


def remembered_unlock_status(data_root) -> dict[str, object]:
    """Return public-safe platform, availability, enrollment, and opt-in state."""

    cli_enabled = cli_remembered_unlock_enabled(data_root)
    legacy_quarantined = cli_legacy_unlock_quarantined(data_root)
    try:
        binding = require_project_policy_binding(data_root)
        binding_state = "valid"
    except AppError as exc:
        if exc.code == "operator_policy_binding_mismatch":
            binding_state = "mismatch"
        elif exc.code == "operator_policy_binding_required":
            binding_state = "missing"
        else:
            raise
        binding = None
    if cli_enabled and not legacy_quarantined and binding is not None:
        available, passphrase = _load_service_with_availability(
            data_root,
            CLI_REMEMBERED_PASSPHRASE_SERVICE,
        )
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
        "binding_state": binding_state,
    }


def _valid_enrollment_id(value: object) -> bool:
    return (
        isinstance(value, str)
        and len(value) == 32
        and all(character in "0123456789abcdef" for character in value)
    )
