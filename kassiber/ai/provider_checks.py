"""The last explicit check of each AI provider, kept across restarts.

Settings → Assistant shows what a provider's last check found ("Installed
v2.1 · Signed in · checked 3 days ago") instead of "Not checked yet" after
every launch. Only a check the user started writes here (a Settings check, or
the picker's Check models); reading it contacts nothing.

A result belongs to the endpoint it was found at: it is stored with the
provider's base URL and read back only while the provider still has that URL.
A new API key or address clears it, as it clears the in-memory discovery
cache. Model lists are capped so one large catalog cannot bloat the book.
"""

from __future__ import annotations

import json
from typing import Any

from .discovery_cache import DiscoverySnapshot

CHECK_KIND_RUNTIME = "runtime"
CHECK_KIND_MODELS = "models"
_CHECK_KINDS = (CHECK_KIND_RUNTIME, CHECK_KIND_MODELS)
MAX_SAVED_MODELS = 1000


def _bounded(kind: str, value: Any) -> Any:
    if kind == CHECK_KIND_MODELS and isinstance(value, list):
        return value[:MAX_SAVED_MODELS]
    if kind == CHECK_KIND_RUNTIME and isinstance(value, list):
        return [
            {**status, "models": status["models"][:MAX_SAVED_MODELS]}
            if isinstance(status, dict) and isinstance(status.get("models"), list)
            else status
            for status in value
        ]
    return value


def save_provider_check(
    conn, provider_name: str, kind: str, base_url: str, snapshot: DiscoverySnapshot
) -> None:
    """Keep ``snapshot`` as the provider's last ``kind`` check at ``base_url``."""

    if kind not in _CHECK_KINDS:
        raise ValueError(f"unknown provider check kind: {kind}")
    conn.execute(
        """
        INSERT INTO ai_provider_checks(
            provider_name, check_kind, base_url, checked_at, stale, error_json, value_json
        ) VALUES(?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(provider_name, check_kind) DO UPDATE SET
            base_url = excluded.base_url,
            checked_at = excluded.checked_at,
            stale = excluded.stale,
            error_json = excluded.error_json,
            value_json = excluded.value_json
        """,
        (
            provider_name,
            kind,
            base_url,
            snapshot.checked_at,
            1 if snapshot.stale else 0,
            json.dumps(snapshot.error) if snapshot.error else None,
            json.dumps(_bounded(kind, snapshot.value), default=str),
        ),
    )
    conn.commit()


def load_provider_check(
    conn, provider_name: str, kind: str, base_url: str
) -> DiscoverySnapshot | None:
    """The provider's last ``kind`` check, if it was made at ``base_url``."""

    row = conn.execute(
        """
        SELECT base_url, checked_at, stale, error_json, value_json
        FROM ai_provider_checks WHERE provider_name = ? AND check_kind = ?
        """,
        (provider_name, kind),
    ).fetchone()
    if row is None or row["base_url"] != base_url:
        return None
    try:
        value = json.loads(row["value_json"])
        error = json.loads(row["error_json"]) if row["error_json"] else None
    except (TypeError, ValueError):
        return None
    return DiscoverySnapshot(
        value=value,
        checked_at=row["checked_at"],
        stale=bool(row["stale"]),
        error=error if isinstance(error, dict) else None,
    )


def clear_provider_checks(conn, provider_name: str) -> None:
    """Forget a provider's checks: its key, address or row changed."""

    conn.execute("DELETE FROM ai_provider_checks WHERE provider_name = ?", (provider_name,))
    conn.commit()
