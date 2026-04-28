"""SQLite-backed CRUD for AI provider configurations.

Mirrors `kassiber.backends` for the `ai_providers` table. The stored shape:

    name              TEXT PRIMARY KEY  (lowercase)
    base_url          TEXT NOT NULL     (OpenAI-compatible root, e.g. http://localhost:11434/v1)
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

from typing import Any, Iterable

from ..db import get_setting, set_setting
from ..errors import AppError
from ..time_utils import now_iso
from ..util import str_or_none


AI_PROVIDER_KINDS = ("local", "remote", "tee")
DEFAULT_AI_PROVIDER_SETTING = "default_ai_provider"
AI_PROVIDERS_SEEDED_SETTING = "ai_providers_seeded"

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


def normalize_base_url(value: Any) -> str:
    """Validate and canonicalize an OpenAI-compatible base URL.

    Strips whitespace and trailing slashes, requires a scheme, and raises
    `AppError(code='validation')` on bad input. Used by both the persisted
    CRUD path and the transient `ai.test_connection` handler.
    """
    base = str_or_none(value)
    if base is None:
        raise AppError(
            "AI provider base_url is required",
            code="validation",
            hint="Use the OpenAI-compatible root, e.g. http://localhost:11434/v1",
        )
    base = base.strip().rstrip("/")
    if not base:
        raise AppError("AI provider base_url is required", code="validation")
    if "://" not in base:
        raise AppError(
            f"AI provider base_url '{base}' is missing a scheme",
            code="validation",
            hint="Include http:// or https:// in the URL.",
        )
    return base


def redact_ai_provider_for_output(provider: dict, *, default_name: str | None = None) -> dict:
    """Return a safe payload for envelopes — `api_key` never included."""
    payload: dict[str, Any] = {}
    for field in AI_PROVIDER_SAFE_OUTPUT_FIELDS:
        if field in provider:
            payload[field] = provider[field]
    payload["has_api_key"] = bool(str_or_none(provider.get("api_key")))
    if default_name is not None:
        payload["is_default"] = provider.get("name") == default_name
    return payload


def _row_to_dict(row) -> dict[str, Any]:
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
            DEFAULT_BOOTSTRAP_PROVIDER["base_url"],
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
    rows = conn.execute("SELECT * FROM ai_providers ORDER BY name ASC").fetchall()
    return [_row_to_dict(row) for row in rows]


def get_db_ai_provider(conn, name: str) -> dict:
    """Fetch one provider, or raise `AppError(not_found)`."""
    seed_default_ai_provider_if_empty(conn)
    name = _normalize_name(name)
    row = conn.execute("SELECT * FROM ai_providers WHERE name = ?", (name,)).fetchone()
    if not row:
        raise AppError(
            f"AI provider '{name}' not found",
            code="not_found",
            hint="Use `kassiber ai providers list` to see configured providers.",
        )
    return _row_to_dict(row)


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
