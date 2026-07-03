"""SQLite-backed CRUD for AI provider configurations.

Mirrors `kassiber.backends` for the `ai_providers` table. The stored shape:

    name              TEXT PRIMARY KEY  (lowercase)
    base_url          TEXT NOT NULL     (OpenAI-compatible root, e.g. http://localhost:11434/v1,
                                        or fixed CLI locator claude-cli://default / codex-cli://default)
    api_key           TEXT              (nullable; never echoed in envelopes)
    default_model     TEXT              (nullable)
    kind              TEXT NOT NULL     (local | remote | tee)
    notes             TEXT
    acknowledged_at   TEXT              (set when the user confirmed first off-device use)
    created_at        TEXT NOT NULL
    updated_at        TEXT NOT NULL

Default-provider pointer lives in the existing `settings` table under
`default_ai_provider`. A one-shot bootstrap seed inserts a local Ollama row
the first time the table is queried; it leaves a sentinel in `settings`
(`ai_providers_seeded`) so deleted bootstrap rows do not get re-added.
"""

from __future__ import annotations

import hashlib
import os
from pathlib import Path
from typing import Any, Callable, Iterable

from ..db import get_setting, set_setting
from ..errors import AppError
from ..time_utils import now_iso
from ..util import str_or_none


AI_PROVIDER_KINDS = ("local", "remote", "tee")
DEFAULT_AI_PROVIDER_SETTING = "default_ai_provider"
AI_PROVIDERS_SEEDED_SETTING = "ai_providers_seeded"
DESKTOP_BUNDLE_ID = "at.bitcoinaustria.kassiber"
AI_PROVIDER_SECRET_STORE_SQLCIPHER = "sqlcipher_inline"
AI_PROVIDER_SECRET_STORES = (
    "macos_keychain",
    "windows_dpapi",
    "linux_secret_service",
    AI_PROVIDER_SECRET_STORE_SQLCIPHER,
)
AI_PROVIDER_SECRET_STATES = ("ok", "missing", "needs_reauth", "unavailable")

CLI_PROVIDER_LOCATORS = ("claude-cli://default", "codex-cli://default")

DEFAULT_BOOTSTRAP_PROVIDER = {
    "name": "ollama",
    "base_url": "http://localhost:11434/v1",
    "api_key": None,
    "default_model": None,
    "kind": "local",
    "notes": "Local Ollama (default OpenAI-compatible endpoint).",
}

AI_PROVIDER_SAFE_OUTPUT_FIELDS = (
    "name",
    "base_url",
    "default_model",
    "kind",
    "notes",
    "acknowledged_at",
    "created_at",
    "updated_at",
)


def _normalize_name(value: Any) -> str:
    name = str_or_none(value)
    if name is None:
        raise AppError("AI provider name is required", code="validation")
    return name.strip().lower()


def _normalize_kind(value: Any) -> str:
    kind = str_or_none(value)
    if kind is None:
        return "local"
    kind = kind.strip().lower()
    if kind not in AI_PROVIDER_KINDS:
        raise AppError(
            f"Unsupported AI provider kind '{kind}'",
            code="validation",
            hint=f"Choose one of: {', '.join(AI_PROVIDER_KINDS)}",
        )
    return kind


def _normalize_secret_store_id(value: Any) -> str:
    store_id = str_or_none(value) or AI_PROVIDER_SECRET_STORE_SQLCIPHER
    store_id = store_id.strip().lower()
    if store_id not in AI_PROVIDER_SECRET_STORES:
        raise AppError(
            f"Unsupported AI provider secret store '{store_id}'",
            code="validation",
            hint=f"Choose one of: {', '.join(AI_PROVIDER_SECRET_STORES)}",
        )
    return store_id


def _normalize_secret_state(value: Any) -> str:
    state = str_or_none(value) or "missing"
    state = state.strip().lower()
    if state not in AI_PROVIDER_SECRET_STATES:
        raise AppError(
            f"Unsupported AI provider secret state '{state}'",
            code="validation",
            hint=f"Choose one of: {', '.join(AI_PROVIDER_SECRET_STATES)}",
        )
    return state


def _validate_locator_kind(base_url: str, kind: str) -> None:
    if is_cli_provider_locator(base_url) and kind == "local":
        raise AppError(
            "Claude/Codex CLI providers cannot be marked local",
            code="validation",
            hint=(
                "Use --kind remote (or tee if your configured CLI path has documented "
                "confidential inference). These CLIs may send prompts to external model providers."
            ),
        )


def is_cli_provider_locator(value: Any) -> bool:
    base = str_or_none(value)
    if base is None:
        return False
    base = base.strip().lower()
    return base in CLI_PROVIDER_LOCATORS


def _data_root_from_connection(conn) -> str:
    try:
        rows = conn.execute("PRAGMA database_list").fetchall()
    except Exception:
        return ""
    for row in rows:
        try:
            name = row["name"]
            filename = row["file"]
        except (KeyError, TypeError, IndexError):
            name = row[1] if len(row) > 1 else None
            filename = row[2] if len(row) > 2 else None
        if name == "main" and filename:
            return str(Path(str(filename)).expanduser().resolve().parent)
    return ""


def ai_provider_secret_service_id(data_root: str | None) -> str:
    """Return the non-secret service identifier for desktop AI key refs."""

    normalized_root = str(Path(data_root).expanduser().resolve()) if data_root else ""
    material = f"{DESKTOP_BUNDLE_ID}:{normalized_root}".encode("utf-8")
    return hashlib.sha256(material).hexdigest()


def _default_secret_ref(conn, provider_name: str, *, has_secret: bool) -> dict[str, Any]:
    return {
        "store_id": AI_PROVIDER_SECRET_STORE_SQLCIPHER,
        "service": ai_provider_secret_service_id(_data_root_from_connection(conn)),
        "account": provider_name,
        "state": "ok" if has_secret else "missing",
        "created_at": None,
        "rotated_at": None,
    }


def ai_provider_secret_ref_namespace(conn, provider_name: str) -> tuple[str, str]:
    """Return the only native secret service/account allowed for a provider."""

    return ai_provider_secret_service_id(_data_root_from_connection(conn)), _normalize_name(
        provider_name
    )


def _row_secret_ref(conn, row) -> dict[str, Any]:
    has_secret = bool(str_or_none(row["api_key"]))
    provider_name = _normalize_name(row["name"])
    expected_service, expected_account = ai_provider_secret_ref_namespace(conn, provider_name)
    ref = {
        "store_id": row["secret_store_id"],
        "service": row["secret_service"],
        "account": row["secret_account"],
        "state": row["secret_state"],
        "created_at": row["secret_created_at"],
        "rotated_at": row["secret_rotated_at"],
    }
    if not ref["store_id"]:
        return _default_secret_ref(conn, provider_name, has_secret=has_secret)
    store_id = _normalize_secret_store_id(ref["store_id"])
    service = str(ref["service"] or expected_service)
    account = str(ref["account"] or expected_account)
    state = _normalize_secret_state(ref["state"] or ("ok" if has_secret else "missing"))
    if store_id != AI_PROVIDER_SECRET_STORE_SQLCIPHER and (
        service != expected_service or account != expected_account
    ):
        # Database rows can come from restored/imported attacker-controlled
        # projects. Never surface or use a native ref outside Kassiber's
        # per-data-root/provider namespace; force repair by key re-entry.
        service = expected_service
        account = expected_account
        state = "unavailable"
    return {
        "store_id": store_id,
        "service": service,
        "account": account,
        "state": state,
        "created_at": ref["created_at"],
        "rotated_at": ref["rotated_at"],
    }


def _upsert_ai_provider_secret_ref(
    conn,
    provider_name: str,
    *,
    store_id: str = AI_PROVIDER_SECRET_STORE_SQLCIPHER,
    state: str,
    service: str | None = None,
    account: str | None = None,
) -> dict[str, Any]:
    provider_name = _normalize_name(provider_name)
    store_id = _normalize_secret_store_id(store_id)
    state = _normalize_secret_state(state)
    service = str_or_none(service) or ai_provider_secret_service_id(_data_root_from_connection(conn))
    account = str_or_none(account) or provider_name
    ts = now_iso()
    row = conn.execute(
        "SELECT created_at FROM ai_provider_secret_refs WHERE provider_name = ?",
        (provider_name,),
    ).fetchone()
    created_at = row["created_at"] if row else ts
    conn.execute(
        """
        INSERT INTO ai_provider_secret_refs(
            provider_name, store_id, service, account, state, created_at, rotated_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(provider_name) DO UPDATE SET
            store_id = excluded.store_id,
            service = excluded.service,
            account = excluded.account,
            state = excluded.state,
            rotated_at = excluded.rotated_at
        """,
        (provider_name, store_id, service, account, state, created_at, ts),
    )
    return {
        "store_id": store_id,
        "service": service,
        "account": account,
        "state": state,
        "created_at": created_at,
        "rotated_at": ts,
    }


def _secret_ref_for_error(provider: dict) -> dict[str, Any]:
    ref = dict(provider.get("secret_ref") or {})
    store_id = ref.get("store_id") or AI_PROVIDER_SECRET_STORE_SQLCIPHER
    state = ref.get("state") or "missing"
    if store_id != AI_PROVIDER_SECRET_STORE_SQLCIPHER and state == "ok":
        state = "unavailable"
    return {
        "provider_name": provider.get("name"),
        "store_id": store_id,
        "service": ref.get("service"),
        "account": ref.get("account") or provider.get("name"),
        "state": state,
    }


def mark_ai_provider_secret_ref_state(
    conn,
    provider_name: str,
    state: str,
    *,
    store_id: str | None = None,
    service: str | None = None,
    account: str | None = None,
) -> dict[str, Any]:
    """Persist the visible state for a provider secret reference."""

    provider_name = _normalize_name(provider_name)
    provider = get_db_ai_provider(conn, provider_name)
    ref = provider.get("secret_ref") or {}
    return _upsert_ai_provider_secret_ref(
        conn,
        provider_name,
        store_id=store_id or ref.get("store_id") or AI_PROVIDER_SECRET_STORE_SQLCIPHER,
        state=state,
        service=service or ref.get("service"),
        account=account or ref.get("account"),
    )


SecretResolver = Callable[[dict[str, Any]], str | None]


def get_ai_provider_api_key_for_use(
    provider: dict,
    *,
    conn=None,
    secret_resolver: SecretResolver | None = None,
) -> str | None:
    """Return an inline API key or resolve an OS-backed provider ref."""

    ref = provider.get("secret_ref") or {}
    store_id = ref.get("store_id") or AI_PROVIDER_SECRET_STORE_SQLCIPHER
    state = ref.get("state") or (
        "ok" if str_or_none(provider.get("api_key")) else "missing"
    )
    if store_id == AI_PROVIDER_SECRET_STORE_SQLCIPHER:
        return str_or_none(provider.get("api_key"))
    if state != "ok":
        if conn is not None and provider.get("name") and state == "unavailable":
            mark_ai_provider_secret_ref_state(conn, str(provider["name"]), state)
            conn.commit()
        secret_ref = _secret_ref_for_error(provider)
        raise AppError(
            f"AI provider '{provider.get('name')}' secret is not available",
            code="secret_ref_unavailable",
            hint="Open Settings -> AI providers and re-enter or repair the provider API key.",
            details={"refs": [secret_ref], "state": state},
            retryable=True,
        )
    if secret_resolver is not None:
        try:
            secret = secret_resolver(dict(ref, provider_name=provider.get("name")))
        except AppError as exc:
            state = "unavailable"
            if conn is not None:
                if provider.get("name"):
                    mark_ai_provider_secret_ref_state(conn, str(provider["name"]), state)
                conn.commit()
            secret_ref = _secret_ref_for_error(provider)
            secret_ref["state"] = state
            raise AppError(
                f"AI provider '{provider.get('name')}' secret is not available in the OS credential store",
                code="secret_ref_unavailable",
                hint=exc.hint
                or "Open Settings -> AI providers and re-enter or repair the provider API key.",
                details={
                    "refs": [secret_ref],
                    "state": state,
                    "cause_code": exc.code,
                },
                retryable=True,
            ) from exc
        if str_or_none(secret):
            return str_or_none(secret)
        state = "missing"
    else:
        state = "unavailable"
    if conn is not None and provider.get("name"):
        mark_ai_provider_secret_ref_state(conn, str(provider["name"]), state)
        conn.commit()
    secret_ref = _secret_ref_for_error(provider)
    secret_ref["state"] = state
    raise AppError(
        f"AI provider '{provider.get('name')}' secret is not available in the OS credential store",
        code="secret_ref_unavailable",
        hint="Open Settings -> AI providers and re-enter or repair the provider API key.",
        details={"refs": [secret_ref], "state": state},
        retryable=True,
    )


def normalize_base_url(value: Any) -> str:
    """Validate and canonicalize an AI provider locator.

    Strips whitespace and trailing slashes, requires a scheme, and raises
    `AppError(code='validation')` on bad input. Most providers use an
    OpenAI-compatible HTTP root; fixed local CLI adapters use
    ``claude-cli://default`` or ``codex-cli://default``.
    """
    base = str_or_none(value)
    if base is None:
        raise AppError(
            "AI provider base_url is required",
            code="validation",
            hint=(
                "Use an OpenAI-compatible root, e.g. http://localhost:11434/v1, "
                "or claude-cli://default / codex-cli://default."
            ),
        )
    base = base.strip().rstrip("/")
    if not base:
        raise AppError("AI provider base_url is required", code="validation")
    if is_cli_provider_locator(base):
        return base.lower()
    if "://" not in base:
        raise AppError(
            f"AI provider base_url '{base}' is missing a scheme",
            code="validation",
            hint="Include http://, https://, claude-cli://, or codex-cli://.",
        )
    if not (base.startswith("http://") or base.startswith("https://")):
        raise AppError(
            f"Unsupported AI provider locator '{base}'",
            code="validation",
            hint="Use http(s)://, claude-cli://default, or codex-cli://default.",
        )
    return base


def redact_ai_provider_for_output(provider: dict, *, default_name: str | None = None) -> dict:
    """Return a safe payload for envelopes — `api_key` never included."""
    payload: dict[str, Any] = {}
    for field in AI_PROVIDER_SAFE_OUTPUT_FIELDS:
        if field in provider:
            payload[field] = provider[field]
    ref = provider.get("secret_ref") or {}
    store_id = ref.get("store_id") or AI_PROVIDER_SECRET_STORE_SQLCIPHER
    ref_state = ref.get("state")
    inline_has_key = bool(str_or_none(provider.get("api_key")))
    if store_id == AI_PROVIDER_SECRET_STORE_SQLCIPHER:
        ref_state = "ok" if inline_has_key else "missing"
    payload["has_api_key"] = inline_has_key or (
        store_id != AI_PROVIDER_SECRET_STORE_SQLCIPHER and ref_state == "ok"
    )
    payload["secret_ref"] = {
        "store_id": store_id,
        "state": ref_state or ("ok" if payload["has_api_key"] else "missing"),
    }
    payload["supports_reasoning_effort"] = is_cli_provider_locator(provider.get("base_url"))
    if default_name is not None:
        payload["is_default"] = provider.get("name") == default_name
    return payload


def _row_to_dict(conn, row) -> dict[str, Any]:
    return {
        "name": row["name"],
        "base_url": row["base_url"],
        "api_key": row["api_key"],
        "default_model": row["default_model"],
        "kind": row["kind"],
        "notes": row["notes"],
        "acknowledged_at": row["acknowledged_at"],
        "created_at": row["created_at"],
        "updated_at": row["updated_at"],
        "secret_ref": _row_secret_ref(conn, row),
    }


def seed_default_ai_provider_if_empty(conn) -> None:
    """One-shot bootstrap: insert a local Ollama row when the table is empty
    and we have not seeded before. Leaves a sentinel so deleted bootstrap
    rows are not re-added on subsequent calls."""
    if get_setting(conn, AI_PROVIDERS_SEEDED_SETTING):
        return
    existing = conn.execute("SELECT 1 FROM ai_providers LIMIT 1").fetchone()
    if existing:
        set_setting(conn, AI_PROVIDERS_SEEDED_SETTING, "1")
        conn.commit()
        return
    ts = now_iso()
    conn.execute(
        """
        INSERT INTO ai_providers(
            name, base_url, api_key, default_model, kind, notes, acknowledged_at, created_at, updated_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            DEFAULT_BOOTSTRAP_PROVIDER["name"],
            os.environ.get("KASSIBER_DEFAULT_AI_BASE_URL", DEFAULT_BOOTSTRAP_PROVIDER["base_url"]),
            DEFAULT_BOOTSTRAP_PROVIDER["api_key"],
            DEFAULT_BOOTSTRAP_PROVIDER["default_model"],
            DEFAULT_BOOTSTRAP_PROVIDER["kind"],
            DEFAULT_BOOTSTRAP_PROVIDER["notes"],
            now_iso() if DEFAULT_BOOTSTRAP_PROVIDER["kind"] == "local" else None,
            ts,
            ts,
        ),
    )
    if not get_setting(conn, DEFAULT_AI_PROVIDER_SETTING):
        set_setting(conn, DEFAULT_AI_PROVIDER_SETTING, DEFAULT_BOOTSTRAP_PROVIDER["name"])
    set_setting(conn, AI_PROVIDERS_SEEDED_SETTING, "1")
    conn.commit()


def list_db_ai_providers(conn) -> list[dict]:
    """Return all rows from the `ai_providers` table, sorted by name."""
    seed_default_ai_provider_if_empty(conn)
    rows = conn.execute(
        """
        SELECT
            p.*,
            r.store_id AS secret_store_id,
            r.service AS secret_service,
            r.account AS secret_account,
            r.state AS secret_state,
            r.created_at AS secret_created_at,
            r.rotated_at AS secret_rotated_at
        FROM ai_providers p
        LEFT JOIN ai_provider_secret_refs r ON r.provider_name = p.name
        ORDER BY p.name ASC
        """
    ).fetchall()
    return [_row_to_dict(conn, row) for row in rows]


def get_db_ai_provider(conn, name: str) -> dict:
    """Fetch one provider, or raise `AppError(not_found)`."""
    seed_default_ai_provider_if_empty(conn)
    name = _normalize_name(name)
    row = conn.execute(
        """
        SELECT
            p.*,
            r.store_id AS secret_store_id,
            r.service AS secret_service,
            r.account AS secret_account,
            r.state AS secret_state,
            r.created_at AS secret_created_at,
            r.rotated_at AS secret_rotated_at
        FROM ai_providers p
        LEFT JOIN ai_provider_secret_refs r ON r.provider_name = p.name
        WHERE p.name = ?
        """,
        (name,),
    ).fetchone()
    if not row:
        raise AppError(
            f"AI provider '{name}' not found",
            code="not_found",
            hint="Use `kassiber ai providers list` to see configured providers.",
        )
    return _row_to_dict(conn, row)


def get_default_ai_provider_name(conn) -> str | None:
    seed_default_ai_provider_if_empty(conn)
    return get_setting(conn, DEFAULT_AI_PROVIDER_SETTING)


def resolve_ai_provider(conn, name: str | None = None) -> dict:
    """Look up a provider by name, defaulting to the stored default.

    Raises `AppError(code='ai_provider_not_configured')` if no provider name
    can be resolved (e.g. table is empty and seeding is disabled by tombstone).
    """
    seed_default_ai_provider_if_empty(conn)
    if name is None:
        name = get_setting(conn, DEFAULT_AI_PROVIDER_SETTING)
    if not name:
        raise AppError(
            "No AI provider is configured",
            code="ai_provider_not_configured",
            hint="Run `kassiber ai providers create` or set a default with `kassiber ai providers set-default`.",
        )
    return get_db_ai_provider(conn, name)


def create_db_ai_provider(
    conn,
    name: str,
    base_url: str,
    *,
    api_key: str | None = None,
    default_model: str | None = None,
    kind: str = "local",
    notes: str | None = None,
    acknowledged: bool = False,
) -> dict:
    """Insert a new AI provider row. Raises on name conflict or invalid input."""
    seed_default_ai_provider_if_empty(conn)
    name = _normalize_name(name)
    base_url = normalize_base_url(base_url)
    kind = _normalize_kind(kind)
    _validate_locator_kind(base_url, kind)
    api_key = str_or_none(api_key)
    default_model = str_or_none(default_model)
    notes = str_or_none(notes)
    existing = conn.execute("SELECT 1 FROM ai_providers WHERE name = ?", (name,)).fetchone()
    if existing:
        raise AppError(
            f"AI provider '{name}' already exists",
            code="conflict",
            hint="Use `kassiber ai providers update` to change an existing provider.",
        )
    ts = now_iso()
    acknowledged_at = ts if (acknowledged or kind == "local") else None
    conn.execute(
        """
        INSERT INTO ai_providers(
            name, base_url, api_key, default_model, kind, notes, acknowledged_at, created_at, updated_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (name, base_url, api_key, default_model, kind, notes, acknowledged_at, ts, ts),
    )
    if api_key:
        _upsert_ai_provider_secret_ref(conn, name, state="ok")
    conn.commit()
    return get_db_ai_provider(conn, name)


_UPDATE_FIELDS = ("base_url", "api_key", "default_model", "kind", "notes")
_CLEARABLE_FIELDS = {"api_key", "default_model", "notes"}


def update_db_ai_provider(conn, name: str, updates: dict) -> dict:
    """Apply a partial update to a provider row.

    `updates` may contain any of the fields in `_UPDATE_FIELDS`, plus an optional
    `clear` list naming fields to null out. `acknowledged` (truthy) sets
    `acknowledged_at` to now; `acknowledge_clear=True` resets it to null.
    """
    seed_default_ai_provider_if_empty(conn)
    name = _normalize_name(name)
    row = conn.execute("SELECT * FROM ai_providers WHERE name = ?", (name,)).fetchone()
    if not row:
        raise AppError(
            f"AI provider '{name}' not found",
            code="not_found",
        )
    clear_fields = {field for field in (updates.get("clear") or []) if field in _CLEARABLE_FIELDS}
    unsupported_clear = set(updates.get("clear") or []) - _CLEARABLE_FIELDS
    if unsupported_clear:
        raise AppError(
            f"Unsupported AI provider clear field(s): {', '.join(sorted(unsupported_clear))}",
            code="validation",
            hint=f"Clearable fields: {', '.join(sorted(_CLEARABLE_FIELDS))}",
        )
    nontrivial = any(updates.get(field) is not None for field in _UPDATE_FIELDS)
    if not nontrivial and not clear_fields and "acknowledged" not in updates and "acknowledge_clear" not in updates:
        raise AppError(
            "ai providers update requires at least one field to change",
            code="validation",
            hint="Pass --base-url, --api-key, --default-model, --kind, --notes, --acknowledge, --clear FIELD, or --revoke-acknowledge",
        )
    new_kind = updates.get("kind")
    if new_kind is not None:
        new_kind = _normalize_kind(new_kind)
    new_base_url = updates.get("base_url")
    if new_base_url is not None:
        new_base_url = normalize_base_url(new_base_url)

    def resolved(field: str, fallback):
        if field in clear_fields:
            return None
        provided = updates.get(field)
        if provided is not None:
            return str_or_none(provided)
        return fallback

    new_acknowledged_at = row["acknowledged_at"]
    if updates.get("acknowledge_clear"):
        new_acknowledged_at = None
    if updates.get("acknowledged"):
        new_acknowledged_at = now_iso()
    if new_kind == "local" and not new_acknowledged_at:
        new_acknowledged_at = now_iso()

    merged = {
        "base_url": new_base_url if new_base_url is not None else row["base_url"],
        "api_key": resolved("api_key", row["api_key"]),
        "default_model": resolved("default_model", row["default_model"]),
        "kind": new_kind if new_kind is not None else row["kind"],
        "notes": resolved("notes", row["notes"]),
        "acknowledged_at": new_acknowledged_at,
    }
    _validate_locator_kind(merged["base_url"], merged["kind"])
    conn.execute(
        """
        UPDATE ai_providers
        SET base_url = ?, api_key = ?, default_model = ?, kind = ?, notes = ?, acknowledged_at = ?, updated_at = ?
        WHERE name = ?
        """,
        (
            merged["base_url"],
            merged["api_key"],
            merged["default_model"],
            merged["kind"],
            merged["notes"],
            merged["acknowledged_at"],
            now_iso(),
            name,
        ),
    )
    if "api_key" in clear_fields:
        _upsert_ai_provider_secret_ref(conn, name, state="missing")
    elif updates.get("api_key") is not None:
        _upsert_ai_provider_secret_ref(conn, name, state="ok")
    conn.commit()
    return get_db_ai_provider(conn, name)


def set_db_ai_provider_api_key(conn, name: str, api_key: str | None) -> dict:
    """Set or clear an AI provider API key through the narrow secret path."""

    seed_default_ai_provider_if_empty(conn)
    name = _normalize_name(name)
    row = conn.execute("SELECT 1 FROM ai_providers WHERE name = ?", (name,)).fetchone()
    if not row:
        raise AppError(
            f"AI provider '{name}' not found",
            code="not_found",
        )
    normalized_key = str_or_none(api_key)
    state = "ok" if normalized_key else "missing"
    conn.execute(
        "UPDATE ai_providers SET api_key = ?, updated_at = ? WHERE name = ?",
        (normalized_key, now_iso(), name),
    )
    _upsert_ai_provider_secret_ref(conn, name, state=state)
    conn.commit()
    return get_db_ai_provider(conn, name)


def set_db_ai_provider_native_secret_ref(
    conn,
    name: str,
    *,
    store_id: str,
    service: str,
    account: str,
    state: str = "ok",
) -> dict:
    """Point a provider at an OS-backed secret and clear inline bytes."""

    seed_default_ai_provider_if_empty(conn)
    name = _normalize_name(name)
    row = conn.execute("SELECT 1 FROM ai_providers WHERE name = ?", (name,)).fetchone()
    if not row:
        raise AppError(
            f"AI provider '{name}' not found",
            code="not_found",
        )
    expected_service, expected_account = ai_provider_secret_ref_namespace(conn, name)
    if service != expected_service or account != expected_account:
        raise AppError(
            "AI provider native secret ref is outside this project's namespace",
            code="validation",
            hint="Re-enter the provider API key so Kassiber can create a project-scoped native secret.",
        )
    conn.execute(
        "UPDATE ai_providers SET api_key = NULL, updated_at = ? WHERE name = ?",
        (now_iso(), name),
    )
    _upsert_ai_provider_secret_ref(
        conn,
        name,
        store_id=store_id,
        state=state,
        service=service,
        account=account,
    )
    conn.commit()
    return get_db_ai_provider(conn, name)


def delete_db_ai_provider(conn, name: str) -> dict:
    """Delete a provider, refusing if it is the active stored default."""
    seed_default_ai_provider_if_empty(conn)
    name = _normalize_name(name)
    row = conn.execute("SELECT name FROM ai_providers WHERE name = ?", (name,)).fetchone()
    if not row:
        raise AppError(
            f"AI provider '{name}' not found",
            code="not_found",
        )
    if get_setting(conn, DEFAULT_AI_PROVIDER_SETTING) == name:
        raise AppError(
            f"AI provider '{name}' is the stored default; clear it with `kassiber ai providers clear-default` first",
            code="conflict",
        )
    conn.execute("DELETE FROM ai_providers WHERE name = ?", (name,))
    conn.commit()
    return {"name": name, "deleted": True}


def set_default_ai_provider(conn, name: str) -> dict:
    """Persist `default_ai_provider=<name>` in `settings`."""
    seed_default_ai_provider_if_empty(conn)
    name = _normalize_name(name)
    row = conn.execute("SELECT 1 FROM ai_providers WHERE name = ?", (name,)).fetchone()
    if not row:
        raise AppError(
            f"AI provider '{name}' not found",
            code="not_found",
            hint="Use `kassiber ai providers list` to see available providers.",
        )
    set_setting(conn, DEFAULT_AI_PROVIDER_SETTING, name)
    conn.commit()
    return {"default": name}


def clear_default_ai_provider(conn) -> dict:
    """Clear the stored default AI provider."""
    seed_default_ai_provider_if_empty(conn)
    conn.execute(
        "DELETE FROM settings WHERE key = ?",
        (DEFAULT_AI_PROVIDER_SETTING,),
    )
    conn.commit()
    return {"default": None, "cleared": True}


def list_with_default(conn) -> dict:
    """List providers + active default in one call (used by the CLI/daemon)."""
    providers = list_db_ai_providers(conn)
    default_name = get_default_ai_provider_name(conn)
    return {
        "providers": [redact_ai_provider_for_output(p, default_name=default_name) for p in providers],
        "default": default_name,
    }


def acknowledge_remote_use(conn, name: str) -> dict:
    """Stamp explicit off-device acknowledgement for an AI provider."""
    seed_default_ai_provider_if_empty(conn)
    name = _normalize_name(name)
    row = conn.execute(
        "SELECT 1 FROM ai_providers WHERE name = ?",
        (name,),
    ).fetchone()
    if not row:
        raise AppError(
            f"AI provider '{name}' not found",
            code="not_found",
        )
    ts = now_iso()
    conn.execute(
        "UPDATE ai_providers SET acknowledged_at = ?, updated_at = ? WHERE name = ?",
        (ts, ts, name),
    )
    conn.commit()
    return {"name": name, "acknowledged_at": ts}


def require_ai_provider_acknowledged(provider: dict) -> None:
    """Require explicit acknowledgement before sending chat to off-device AI."""
    if provider.get("kind") == "local" or provider.get("acknowledged_at"):
        return
    name = str_or_none(provider.get("name")) or "<unknown>"
    kind = str_or_none(provider.get("kind")) or "remote"
    raise AppError(
        f"AI provider '{name}' requires explicit off-device acknowledgement before chat",
        code="ai_remote_ack_required",
        hint=(
            f"Run `kassiber ai providers update {name} --acknowledge`, "
            "or confirm the provider in Settings → AI providers."
        ),
        details={"provider": name, "kind": kind},
        retryable=False,
    )


def filter_clear_fields(values: Iterable[str] | None) -> list[str]:
    """Helper for CLI `--clear` flag normalization."""
    if not values:
        return []
    seen: set[str] = set()
    out: list[str] = []
    for value in values:
        if value not in _CLEARABLE_FIELDS:
            raise AppError(
                f"Unsupported AI provider clear field '{value}'",
                code="validation",
                hint=f"Clearable fields: {', '.join(sorted(_CLEARABLE_FIELDS))}",
            )
        if value in seen:
            continue
        seen.add(value)
        out.append(value)
    return out
