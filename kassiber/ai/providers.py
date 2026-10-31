"""SQLite-backed CRUD for AI provider configurations.

Mirrors `kassiber.backends` for the `ai_providers` table. The stored shape:

    name              TEXT PRIMARY KEY  (lowercase)
    base_url          TEXT NOT NULL     (OpenAI-compatible API root or
                                        fixed native-provider CLI locator)
    api_key           TEXT              (nullable; never echoed in envelopes)
    default_model     TEXT              (nullable)
    kind              TEXT NOT NULL     (local | remote | tee)
    notes             TEXT
    acknowledged_at   TEXT              (set when the user confirmed first off-device use)
    enabled           INTEGER           (1 = offered for chat; 0 = hidden and refused)
    default_reasoning_effort TEXT       (nullable; seeds a session's effort, one of
                                        AI_REASONING_EFFORTS)
    api_format        TEXT NOT NULL     (auto | responses | chat_completions; HTTP only,
                                        always auto for a CLI locator)
    detected_api_format TEXT            (nullable; the wire format the endpoint was
                                        last seen to answer, cleared when base_url
                                        changes)
    omit_store_field  INTEGER NOT NULL  (0 by default: Chat Completions requests carry
                                        `store: false`; 1 only by the user's explicit
                                        choice, for an endpoint that refuses it, and the
                                        provider's own retention then applies; HTTP only)
    agent_access      TEXT NOT NULL     (full | restricted; full by default: the
                                        assistant may use every tool and receives
                                        unprojected results. restricted keeps an
                                        off-device provider to the projected,
                                        on-device-free surface. A provider on this
                                        device always has full access.)
    created_at        TEXT NOT NULL
    updated_at        TEXT NOT NULL

Default-provider pointer lives in the existing `settings` table under
`default_ai_provider`. A one-shot bootstrap seed inserts the built-in local
provider rows the first time the table is queried; it leaves a sentinel in
`settings` (`ai_providers_seeded`) so deleted bootstrap rows do not get
re-added.
"""

from __future__ import annotations

import hashlib
import ipaddress
import os
from pathlib import Path
from typing import Any, Callable, Iterable
from urllib.parse import urlparse

from ..db import get_setting, set_setting
from ..errors import AppError
from ..time_utils import now_iso
from ..util import str_or_none
from .contracts import (
    AI_API_FORMAT_AUTO,
    AI_API_FORMAT_RESPONSES,
    AI_API_FORMATS,
    AI_WIRE_API_FORMATS,
    CLI_PROVIDER_BY_LOCATOR,
    cli_provider_for_locator,
    is_cli_provider_locator,
)


AI_PROVIDER_KINDS = ("local", "remote", "tee")
AI_AGENT_ACCESS_FULL = "full"
AI_AGENT_ACCESS_RESTRICTED = "restricted"
AI_AGENT_ACCESS_MODES = (AI_AGENT_ACCESS_FULL, AI_AGENT_ACCESS_RESTRICTED)
DEFAULT_AI_PROVIDER_SETTING = "default_ai_provider"
AI_PROVIDERS_SEEDED_SETTING = "ai_providers_seeded"
AI_NATIVE_PROVIDERS_SEEDED_SETTING = "ai_native_broker_providers_seeded"
AI_ACP_PROVIDERS_SEEDED_SETTING = "ai_acp_broker_providers_seeded"
AI_NATIVE_PROVIDER_SEED_GENERATIONS = (
    (
        AI_NATIVE_PROVIDERS_SEEDED_SETTING,
        ("codex-cli://default", "claude-cli://default", "opencode-cli://default"),
    ),
    (AI_ACP_PROVIDERS_SEEDED_SETTING, ("copilot-cli://default",)),
)
DESKTOP_BUNDLE_ID = "at.bitcoinaustria.kassiber"
AI_PROVIDER_SECRET_STORE_SQLCIPHER = "sqlcipher_inline"
AI_PROVIDER_SECRET_STORES = (
    "macos_keychain",
    "windows_dpapi",
    "linux_secret_service",
    AI_PROVIDER_SECRET_STORE_SQLCIPHER,
)
AI_PROVIDER_SECRET_STATES = ("ok", "missing", "needs_reauth", "unavailable")
# Reasoning-effort levels Kassiber can label in the CLI and desktop. A stored
# provider default must be one of these; whether the provider honours it is
# still the provider's call, as for a per-turn choice. Fast mode is billed at a
# higher rate and intentionally has no stored default.
AI_REASONING_EFFORTS = ("low", "medium", "high", "xhigh", "max", "ultra")

DEFAULT_BOOTSTRAP_PROVIDER_NAME = "ollama"
DEFAULT_BOOTSTRAP_PROVIDERS = (
    {
        "name": "ollama",
        "display_name": "Ollama",
        "base_url": "http://localhost:11434/v1",
        "api_key": None,
        "default_model": None,
        "kind": "local",
        "notes": "Local Ollama (default OpenAI Responses-compatible endpoint).",
    },
    {
        "name": "omlx",
        "display_name": "oMLX",
        "base_url": "http://127.0.0.1:8000/v1",
        "api_key": None,
        "default_model": None,
        "kind": "local",
        "notes": "Local oMLX (Apple Silicon MLX Responses endpoint).",
    },
    {
        "name": "codex",
        "display_name": "Codex",
        "base_url": "codex-cli://default",
        "api_key": None,
        "default_model": "default",
        "kind": "remote",
        "notes": "Broker using the installed Codex app-server and existing login. Kassiber's typed tools are available; native coding tools are not.",
    },
    {
        "name": "claude",
        "display_name": "Claude",
        "base_url": "claude-cli://default",
        "api_key": None,
        "default_model": "default",
        "kind": "remote",
        "notes": "Broker using the installed Claude CLI and existing login. Kassiber's typed tools are available; native coding tools are not.",
    },
    {
        "name": "opencode",
        "display_name": "OpenCode",
        "base_url": "opencode-cli://default",
        "api_key": None,
        "default_model": None,
        "kind": "remote",
        "notes": "Broker using the installed OpenCode server and existing configuration. Kassiber's typed tools are available; native coding tools are not.",
    },
    {
        "name": "copilot",
        "display_name": "GitHub Copilot",
        "base_url": "copilot-cli://default",
        "api_key": None,
        "default_model": "default",
        "kind": "remote",
        "notes": "Broker using the installed GitHub Copilot CLI over the Agent Client Protocol and its existing login. Kassiber's typed tools are available; native coding tools are not.",
    },
)
AI_PROVIDER_SAFE_OUTPUT_FIELDS = (
    "name",
    "display_name",
    "base_url",
    "default_model",
    "kind",
    "notes",
    "acknowledged_at",
    "enabled",
    "default_reasoning_effort",
    "api_format",
    "detected_api_format",
    "omit_store_field",
    "agent_access",
    "created_at",
    "updated_at",
)


def _normalize_name(value: Any) -> str:
    name = str_or_none(value)
    if name is None:
        raise AppError("AI provider name is required", code="validation")
    return name.strip().lower()


def _normalize_display_name(value: Any, fallback: str) -> str:
    display_name = str_or_none(value)
    if display_name is None:
        return fallback
    return display_name.strip() or fallback


def _bootstrap_provider_names() -> set[str]:
    return {str(provider["name"]) for provider in DEFAULT_BOOTSTRAP_PROVIDERS}


def _default_bootstrap_provider_name() -> str:
    name = (
        os.environ.get("KASSIBER_DEFAULT_AI_PROVIDER") or DEFAULT_BOOTSTRAP_PROVIDER_NAME
    ).strip().lower()
    if name not in _bootstrap_provider_names():
        raise AppError(
            f"Unsupported default AI provider '{name}'",
            code="validation",
            hint=f"Choose one of: {', '.join(sorted(_bootstrap_provider_names()))}",
        )
    return name


def _bootstrap_provider_base_url(provider: dict[str, Any], default_name: str) -> str:
    provider_name = str(provider["name"])
    specific_env_name = f"KASSIBER_{provider_name.upper().replace('-', '_')}_AI_BASE_URL"
    if os.environ.get(specific_env_name):
        return str(os.environ[specific_env_name])
    if provider_name == default_name and os.environ.get("KASSIBER_DEFAULT_AI_BASE_URL"):
        return str(os.environ["KASSIBER_DEFAULT_AI_BASE_URL"])
    return str(provider["base_url"])


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


def normalize_reasoning_effort(value: Any) -> str | None:
    """Validate a stored default reasoning effort; empty clears it."""
    effort = str_or_none(value)
    if effort is None:
        return None
    effort = effort.strip().lower()
    if effort not in AI_REASONING_EFFORTS:
        raise AppError(
            f"Unsupported reasoning effort '{effort}'",
            code="validation",
            hint=f"Use one of: {', '.join(AI_REASONING_EFFORTS)}",
        )
    return effort


def normalize_api_format(value: Any) -> str:
    """Validate an ``api_format`` setting; empty means ``auto``.

    The CLI spells it ``chat-completions``; both spellings are accepted.
    """
    api_format = str_or_none(value)
    if api_format is None:
        return AI_API_FORMAT_AUTO
    api_format = api_format.strip().lower().replace("-", "_")
    if api_format not in AI_API_FORMATS:
        raise AppError(
            f"Unsupported AI API format '{api_format}'",
            code="validation",
            hint=f"Use one of: {', '.join(AI_API_FORMATS)}",
        )
    return api_format


def normalize_detected_api_format(value: Any) -> str | None:
    """Validate a detected wire format; empty means not detected yet."""
    detected = str_or_none(value)
    if detected is None:
        return None
    detected = detected.strip().lower().replace("-", "_")
    if detected not in AI_WIRE_API_FORMATS:
        raise AppError(
            f"Unsupported detected AI API format '{detected}'",
            code="validation",
            hint=f"Use one of: {', '.join(AI_WIRE_API_FORMATS)}, or null",
        )
    return detected


def resolve_ai_api_format(provider: dict) -> str | None:
    """The wire format a provider speaks, or None for a CLI locator.

    An explicit ``api_format`` wins; ``auto`` uses the detected format and
    otherwise the Responses API.
    """
    if is_cli_provider_locator(provider.get("base_url")):
        return None
    api_format = provider.get("api_format") or AI_API_FORMAT_AUTO
    if api_format in AI_WIRE_API_FORMATS:
        return api_format
    detected = provider.get("detected_api_format")
    if detected in AI_WIRE_API_FORMATS:
        return detected
    return AI_API_FORMAT_RESPONSES


def _normalize_omit_store_field(value: Any) -> bool:
    if not isinstance(value, bool):
        raise AppError("AI provider omit_store_field must be a boolean", code="validation")
    return value


def normalize_agent_access(value: Any) -> str:
    if value not in AI_AGENT_ACCESS_MODES:
        raise AppError(
            "AI provider agent_access must be full or restricted",
            code="validation",
        )
    return value


def _stored_agent_access(value: Any) -> str:
    # An unknown stored value restricts rather than widening access.
    return value if value in AI_AGENT_ACCESS_MODES else AI_AGENT_ACCESS_RESTRICTED


def _validate_locator_omit_store_field(base_url: str, omit_store_field: bool) -> None:
    if omit_store_field and is_cli_provider_locator(base_url):
        raise AppError(
            "CLI agent providers do not take omit_store_field",
            code="validation",
            hint="Only an HTTP provider's Chat Completions requests carry the store field.",
        )


def _validate_locator_api_format(base_url: str, api_format: str, detected: str | None) -> None:
    if not is_cli_provider_locator(base_url):
        return
    if api_format != AI_API_FORMAT_AUTO or detected is not None:
        raise AppError(
            "CLI agent providers do not take an API format",
            code="validation",
            hint="Leave the API format on auto for a *-cli://default locator.",
        )


def _is_loopback_ai_http_url(base_url: str) -> bool:
    if not (base_url.startswith("http://") or base_url.startswith("https://")):
        return False
    try:
        parsed = urlparse(base_url)
    except ValueError:
        return False
    host = (parsed.hostname or "").strip().lower()
    if host == "localhost":
        return True
    try:
        return ipaddress.ip_address(host).is_loopback
    except ValueError:
        return False


def ai_base_url_origin(base_url: object) -> str:
    """The origin a stored API key belongs to: scheme, host and port.

    Lowercased, with the scheme's default port dropped and no userinfo, so
    `https://Host:443/v1` and `https://host/openai/v1` share one origin. A
    CLI locator is its own origin. A key is reused only within its origin,
    in the provider store, `ai.test_connection` and the desktop form alike.
    """

    text = str(base_url or "").strip()
    if is_cli_provider_locator(text):
        return text.lower()
    try:
        parts = urlparse(text)
        port = parts.port
    except ValueError:
        return text.lower()
    scheme = (parts.scheme or "").lower()
    host = (parts.hostname or "").lower()
    netloc = f"[{host}]" if ":" in host else host
    if port is not None and port != {"http": 80, "https": 443}.get(scheme):
        netloc = f"{netloc}:{port}"
    return f"{scheme}://{netloc}"


def infer_ai_provider_kind(base_url: str) -> str:
    """The privacy kind a provider gets when none is given.

    ``local`` for a loopback HTTP endpoint, ``remote`` for anything else,
    CLI agent locators included. ``tee`` (confidential inference) is a claim
    only the user can make, so it is never inferred, and an inferred remote
    provider still needs acknowledgement before it is contacted.
    """
    return "local" if _is_loopback_ai_http_url(base_url) else "remote"


def _validate_locator_kind(base_url: str, kind: str) -> None:
    if kind != "local":
        return
    if is_cli_provider_locator(base_url):
        raise AppError(
            "CLI agent providers cannot be marked local",
            code="validation",
            hint=(
                "Use --kind remote (or tee if your configured CLI path has documented "
                "confidential inference). These CLIs may send prompts to external model providers."
            ),
        )
    if not _is_loopback_ai_http_url(base_url):
        raise AppError(
            "Local AI providers must use a loopback HTTP endpoint",
            code="validation",
            hint=(
                "Use http://localhost, http://127.0.0.1, or http://[::1] for "
                "local providers. Mark off-device endpoints as remote and acknowledge remote use."
            ),
        )


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
    ``claude-cli://default``, ``codex-cli://default``,
    ``opencode-cli://default``, or ``copilot-cli://default``.
    """
    base = str_or_none(value)
    if base is None:
        raise AppError(
            "AI provider base_url is required",
            code="validation",
            hint=(
                "Use an OpenAI-compatible API root, e.g. "
                "http://localhost:11434/v1, "
                "or a built-in *-cli://default locator."
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
            hint="Include http://, https://, or a built-in *-cli:// scheme.",
        )
    if not (base.startswith("http://") or base.startswith("https://")):
        raise AppError(
            f"Unsupported AI provider locator '{base}'",
            code="validation",
            hint="Use http(s):// or a built-in *-cli://default locator.",
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
    # The format the next chat speaks: the explicit setting, else the detected
    # one, else the Responses API. None for a CLI agent, which has its own.
    payload["effective_api_format"] = resolve_ai_api_format(provider)
    if default_name is not None:
        payload["is_default"] = provider.get("name") == default_name
    return payload


def _row_to_dict(conn, row) -> dict[str, Any]:
    return {
        "name": row["name"],
        "display_name": row["display_name"] or row["name"],
        "base_url": row["base_url"],
        "api_key": row["api_key"],
        "default_model": row["default_model"],
        "kind": row["kind"],
        "notes": row["notes"],
        "acknowledged_at": row["acknowledged_at"],
        "enabled": bool(row["enabled"]) if row["enabled"] is not None else True,
        "default_reasoning_effort": row["default_reasoning_effort"],
        "api_format": _stored_api_format(row["api_format"]),
        "detected_api_format": _stored_detected_api_format(row["detected_api_format"]),
        "omit_store_field": bool(row["omit_store_field"]),
        "agent_access": _stored_agent_access(row["agent_access"]),
        "created_at": row["created_at"],
        "updated_at": row["updated_at"],
        "secret_ref": _row_secret_ref(conn, row),
    }


def _stored_api_format(value: Any) -> str:
    # A value this version does not know reads as auto rather than failing
    # every provider read.
    return value if value in AI_API_FORMATS else AI_API_FORMAT_AUTO


def _stored_detected_api_format(value: Any) -> str | None:
    return value if value in AI_WIRE_API_FORMATS else None


def seed_default_ai_provider_if_empty(conn) -> None:
    """One-shot bootstrap for built-in local providers.

    Inserts the built-in rows when the table is empty and we have not seeded
    before. Leaves a sentinel so deleted bootstrap rows are not re-added on
    subsequent calls.
    """
    if get_setting(conn, AI_PROVIDERS_SEEDED_SETTING):
        return
    existing = conn.execute("SELECT 1 FROM ai_providers LIMIT 1").fetchone()
    if existing:
        set_setting(conn, AI_PROVIDERS_SEEDED_SETTING, "1")
        conn.commit()
        return
    default_name = _default_bootstrap_provider_name()
    ts = now_iso()
    rows = []
    for provider in DEFAULT_BOOTSTRAP_PROVIDERS:
        rows.append(
            (
                provider["name"],
                provider["display_name"],
                _bootstrap_provider_base_url(provider, default_name),
                provider["api_key"],
                provider["default_model"],
                provider["kind"],
                provider["notes"],
                now_iso() if provider["kind"] == "local" else None,
                ts,
                ts,
            )
        )
    conn.executemany(
        """
        INSERT INTO ai_providers(
            name, display_name, base_url, api_key, default_model, kind, notes, acknowledged_at, created_at, updated_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        rows,
    )
    if not get_setting(conn, DEFAULT_AI_PROVIDER_SETTING):
        set_setting(conn, DEFAULT_AI_PROVIDER_SETTING, default_name)
    set_setting(conn, AI_PROVIDERS_SEEDED_SETTING, "1")
    conn.commit()


def _free_provider_name(conn, name: str) -> str:
    """Return ``name``, or ``name-2``, ``name-3``... when a user row holds it.

    A book can already have a provider the user named like a built-in one;
    the built-in still has to arrive, so it takes the next free name.
    """

    candidate, suffix = name, 2
    while conn.execute(
        "SELECT 1 FROM ai_providers WHERE name = ? LIMIT 1", (candidate,)
    ).fetchone():
        candidate, suffix = f"{name}-{suffix}", suffix + 1
    return candidate


def seed_native_ai_providers(conn) -> None:
    """Add the built-in chat broker providers once without storing credentials.

    Each generation is seeded once per book, so a book created before the
    Agent Client Protocol agents existed still gains them, while a provider the
    user deleted is not recreated.
    """

    pending = [
        (setting, locators)
        for setting, locators in AI_NATIVE_PROVIDER_SEED_GENERATIONS
        if not get_setting(conn, setting)
    ]
    if not pending:
        return
    ts = now_iso()
    pending_locators = {locator for _, locators in pending for locator in locators}
    native_rows = [
        provider
        for provider in DEFAULT_BOOTSTRAP_PROVIDERS
        if provider["base_url"] in pending_locators
        and conn.execute(
            "SELECT 1 FROM ai_providers WHERE lower(base_url) = lower(?) LIMIT 1",
            (provider["base_url"],),
        ).fetchone()
        is None
    ]
    conn.executemany(
        """
        INSERT INTO ai_providers(
            name, display_name, base_url, api_key, default_model, kind, notes,
            acknowledged_at, created_at, updated_at
        ) VALUES(?, ?, ?, NULL, ?, ?, ?, NULL, ?, ?)
        """,
        [
            (
                _free_provider_name(conn, provider["name"]),
                provider["display_name"],
                provider["base_url"],
                provider["default_model"],
                provider["kind"],
                provider["notes"],
                ts,
                ts,
            )
            for provider in native_rows
        ],
    )
    for setting, _ in pending:
        set_setting(conn, setting, "1")
    conn.commit()


def _ensure_seeded(conn) -> None:
    """Apply every seed generation before any provider read or change.

    A book that predates a generation must gain it on whichever command runs
    first, so `providers update copilot --acknowledge` works without a list.
    """

    seed_default_ai_provider_if_empty(conn)
    seed_native_ai_providers(conn)


def list_db_ai_providers(conn) -> list[dict]:
    """Return all rows from the `ai_providers` table, sorted by name."""
    _ensure_seeded(conn)
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
    _ensure_seeded(conn)
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
    _ensure_seeded(conn)
    return get_setting(conn, DEFAULT_AI_PROVIDER_SETTING)


def resolve_ai_provider(conn, name: str | None = None) -> dict:
    """Look up a provider by name, defaulting to the stored default.

    Raises `AppError(code='ai_provider_not_configured')` if no provider name
    can be resolved (e.g. table is empty and seeding is disabled by tombstone).
    """
    _ensure_seeded(conn)
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
    display_name: str | None = None,
    api_key: str | None = None,
    default_model: str | None = None,
    kind: str | None = None,
    notes: str | None = None,
    acknowledged: bool = False,
    enabled: bool = True,
    default_reasoning_effort: str | None = None,
    api_format: str | None = None,
    detected_api_format: str | None = None,
    omit_store_field: bool = False,
    agent_access: str = AI_AGENT_ACCESS_FULL,
) -> dict:
    """Insert a new AI provider row. Raises on name conflict or invalid input.

    Without a ``kind`` the provider is ``local`` on a loopback URL and
    ``remote`` otherwise (see `infer_ai_provider_kind`).
    ``detected_api_format`` lets a caller save what a connection test of the
    unsaved form found for this ``base_url``. ``omit_store_field`` is the
    user's explicit choice to send Chat Completions requests without
    `store: false`; it is never set on Kassiber's own initiative.
    """
    _ensure_seeded(conn)
    raw_name = str_or_none(name)
    name = _normalize_name(name)
    display_name = _normalize_display_name(display_name, raw_name or name)
    base_url = normalize_base_url(base_url)
    kind = _normalize_kind(kind) if str_or_none(kind) else infer_ai_provider_kind(base_url)
    _validate_locator_kind(base_url, kind)
    api_format = normalize_api_format(api_format)
    detected_api_format = normalize_detected_api_format(detected_api_format)
    _validate_locator_api_format(base_url, api_format, detected_api_format)
    omit_store_field = _normalize_omit_store_field(omit_store_field)
    _validate_locator_omit_store_field(base_url, omit_store_field)
    agent_access = normalize_agent_access(agent_access)
    api_key = str_or_none(api_key)
    default_model = str_or_none(default_model)
    notes = str_or_none(notes)
    default_reasoning_effort = normalize_reasoning_effort(default_reasoning_effort)
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
            name, display_name, base_url, api_key, default_model, kind, notes, acknowledged_at,
            enabled, default_reasoning_effort, api_format, detected_api_format,
            omit_store_field, agent_access, created_at, updated_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            name,
            display_name,
            base_url,
            api_key,
            default_model,
            kind,
            notes,
            acknowledged_at,
            1 if enabled else 0,
            default_reasoning_effort,
            api_format,
            detected_api_format,
            1 if omit_store_field else 0,
            agent_access,
            ts,
            ts,
        ),
    )
    if api_key:
        _upsert_ai_provider_secret_ref(conn, name, state="ok")
    conn.commit()
    return get_db_ai_provider(conn, name)


_UPDATE_FIELDS = (
    "display_name",
    "base_url",
    "api_key",
    "default_model",
    "kind",
    "notes",
    "default_reasoning_effort",
    "enabled",
    "api_format",
    "detected_api_format",
    "omit_store_field",
    "agent_access",
)
_CLEARABLE_FIELDS = {
    "api_key",
    "default_model",
    "notes",
    "default_reasoning_effort",
    "detected_api_format",
}


def update_db_ai_provider(conn, name: str, updates: dict) -> dict:
    """Apply a partial update to a provider row.

    `updates` may contain any of the fields in `_UPDATE_FIELDS`, plus an optional
    `clear` list naming fields to null out. `acknowledged` (truthy) sets
    `acknowledged_at` to now; `acknowledge_clear=True` resets it to null.
    `enabled` (a bool) offers or withdraws the provider for chat; the stored
    default provider cannot be disabled. A detected API format belongs to its
    endpoint, so changing `base_url` clears it unless the same update supplies
    one (a connection test of the edited form).

    `kind` is never changed implicitly, with one exception that fails safe: a
    local provider moved to a non-loopback `base_url` without a `kind` becomes
    remote. Whenever a local provider becomes remote or tee, its implicit local
    acknowledgement is dropped, so off-device use needs `acknowledged` again.
    A remote or tee provider moved to a loopback URL stays off-device, since
    a loopback gateway can still forward prompts.
    """
    _ensure_seeded(conn)
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
            hint=(
                "Pass --display-name, --base-url, --api-key, --default-model, --default-effort, "
                "--kind, --api-format, --omit-store-field, --send-store-field, --agent-access, "
                "--notes, --enable, "
                "--disable, --acknowledge, --clear FIELD, or --revoke-acknowledge"
            ),
        )
    new_enabled = updates.get("enabled")
    if new_enabled is not None and not isinstance(new_enabled, bool):
        raise AppError("AI provider enabled must be a boolean", code="validation")
    if new_enabled is False and get_setting(conn, DEFAULT_AI_PROVIDER_SETTING) == name:
        raise AppError(
            f"AI provider '{name}' is the stored default and cannot be disabled",
            code="conflict",
            hint="Set another provider as the default first, or clear it with `kassiber ai providers clear-default`.",
        )
    new_omit_store_field = (
        _normalize_omit_store_field(updates.get("omit_store_field"))
        if updates.get("omit_store_field") is not None
        else None
    )
    new_agent_access = (
        normalize_agent_access(updates.get("agent_access"))
        if updates.get("agent_access") is not None
        else None
    )
    new_effort = (
        normalize_reasoning_effort(updates.get("default_reasoning_effort"))
        if updates.get("default_reasoning_effort") is not None
        else None
    )
    new_kind = updates.get("kind")
    if new_kind is not None:
        new_kind = _normalize_kind(new_kind)
    new_base_url = updates.get("base_url")
    if new_base_url is not None:
        new_base_url = normalize_base_url(new_base_url)
    new_api_format = (
        normalize_api_format(updates.get("api_format"))
        if updates.get("api_format") is not None
        else None
    )
    new_detected = (
        normalize_detected_api_format(updates.get("detected_api_format"))
        if updates.get("detected_api_format") is not None
        else None
    )

    def resolved(field: str, fallback):
        if field in clear_fields:
            return None
        provided = updates.get(field)
        if provided is not None:
            return str_or_none(provided)
        return fallback

    if (
        new_kind is None
        and new_base_url is not None
        and row["kind"] == "local"
        and not _is_loopback_ai_http_url(new_base_url)
    ):
        new_kind = infer_ai_provider_kind(new_base_url)

    new_acknowledged_at = row["acknowledged_at"]
    if updates.get("acknowledge_clear"):
        new_acknowledged_at = None
    if row["kind"] == "local" and new_kind in ("remote", "tee"):
        # A local provider is acknowledged implicitly; that never covered
        # sending prompts off the device.
        new_acknowledged_at = None
    if updates.get("acknowledged"):
        new_acknowledged_at = now_iso()
    if new_kind == "local" and not new_acknowledged_at:
        new_acknowledged_at = now_iso()

    base_url_changed = new_base_url is not None and new_base_url != row["base_url"]
    origin_changed = base_url_changed and ai_base_url_origin(new_base_url) != ai_base_url_origin(
        row["base_url"]
    )
    if origin_changed and updates.get("api_key") is None:
        # API keys are origin-bound secrets. If a metadata-only update retargets
        # the provider to another origin, drop any inline/native secret ref so
        # the next request cannot silently send an existing bearer token
        # there. A path change on the same origin keeps the key.
        clear_fields.add("api_key")
    if base_url_changed and new_detected is None:
        # What was detected belongs to the old endpoint.
        clear_fields.add("detected_api_format")

    merged = {
        "display_name": resolved("display_name", row["display_name"] or row["name"]),
        "base_url": new_base_url if new_base_url is not None else row["base_url"],
        "api_key": resolved("api_key", row["api_key"]),
        "default_model": resolved("default_model", row["default_model"]),
        "kind": new_kind if new_kind is not None else row["kind"],
        "notes": resolved("notes", row["notes"]),
        "acknowledged_at": new_acknowledged_at,
        "enabled": (1 if new_enabled else 0) if new_enabled is not None else row["enabled"],
        "default_reasoning_effort": (
            None
            if "default_reasoning_effort" in clear_fields
            else new_effort
            if new_effort is not None
            else row["default_reasoning_effort"]
        ),
        "api_format": (
            new_api_format
            if new_api_format is not None
            # A wire format describes an HTTP endpoint. A provider moved to a
            # CLI agent without a format of its own drops a pinned one rather
            # than being refused for it; an explicit one is still validated.
            else AI_API_FORMAT_AUTO
            if base_url_changed and is_cli_provider_locator(new_base_url)
            else _stored_api_format(row["api_format"])
        ),
        "detected_api_format": (
            new_detected
            if new_detected is not None
            else None
            if "detected_api_format" in clear_fields
            else _stored_detected_api_format(row["detected_api_format"])
        ),
        "omit_store_field": (
            new_omit_store_field
            if new_omit_store_field is not None
            # The exemption was chosen for one endpoint, like a detection: a
            # new base URL that does not restate it goes back to the default.
            else False
            if base_url_changed
            else bool(row["omit_store_field"])
        ),
        "agent_access": (
            new_agent_access
            if new_agent_access is not None
            else _stored_agent_access(row["agent_access"])
        ),
    }
    _validate_locator_kind(merged["base_url"], merged["kind"])
    _validate_locator_api_format(
        merged["base_url"], merged["api_format"], merged["detected_api_format"]
    )
    _validate_locator_omit_store_field(merged["base_url"], merged["omit_store_field"])
    conn.execute(
        """
        UPDATE ai_providers
        SET display_name = ?, base_url = ?, api_key = ?, default_model = ?, kind = ?, notes = ?,
            acknowledged_at = ?, enabled = ?, default_reasoning_effort = ?, api_format = ?,
            detected_api_format = ?, omit_store_field = ?, agent_access = ?, updated_at = ?
        WHERE name = ?
        """,
        (
            merged["display_name"],
            merged["base_url"],
            merged["api_key"],
            merged["default_model"],
            merged["kind"],
            merged["notes"],
            merged["acknowledged_at"],
            merged["enabled"],
            merged["default_reasoning_effort"],
            merged["api_format"],
            merged["detected_api_format"],
            1 if merged["omit_store_field"] else 0,
            merged["agent_access"],
            now_iso(),
            name,
        ),
    )
    if "api_key" in clear_fields:
        _upsert_ai_provider_secret_ref(conn, name, state="missing")
    elif updates.get("api_key") is not None:
        _upsert_ai_provider_secret_ref(conn, name, state="ok")
    if base_url_changed or "api_key" in clear_fields or updates.get("api_key") is not None:
        # What the last check found belongs to the old endpoint or key.
        conn.execute("DELETE FROM ai_provider_checks WHERE provider_name = ?", (name,))
    conn.commit()
    return get_db_ai_provider(conn, name)


def record_detected_api_format(
    conn, name: str, base_url: str, detected_api_format: str
) -> bool:
    """Store the wire format an endpoint answered in; True when stored.

    Only while the provider still points at ``base_url``: a detection made
    for an endpoint the row has since left must not describe the new one. It
    is a finding about the endpoint, not a settings edit, so `updated_at`
    stays.
    """
    detected = normalize_detected_api_format(detected_api_format)
    if detected is None or is_cli_provider_locator(base_url):
        return False
    cursor = conn.execute(
        """
        UPDATE ai_providers SET detected_api_format = ?
        WHERE name = ? AND base_url = ?
          AND (detected_api_format IS NULL OR detected_api_format != ?)
        """,
        (detected, _normalize_name(name), base_url, detected),
    )
    conn.commit()
    if cursor.rowcount:
        return True
    row = conn.execute(
        "SELECT 1 FROM ai_providers WHERE name = ? AND base_url = ? AND detected_api_format = ?",
        (_normalize_name(name), base_url, detected),
    ).fetchone()
    return row is not None


def set_db_ai_provider_api_key(conn, name: str, api_key: str | None) -> dict:
    """Set or clear an AI provider API key through the narrow secret path."""

    _ensure_seeded(conn)
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

    _ensure_seeded(conn)
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
    _ensure_seeded(conn)
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
    _ensure_seeded(conn)
    name = _normalize_name(name)
    row = conn.execute("SELECT enabled FROM ai_providers WHERE name = ?", (name,)).fetchone()
    if not row:
        raise AppError(
            f"AI provider '{name}' not found",
            code="not_found",
            hint="Use `kassiber ai providers list` to see available providers.",
        )
    if not row["enabled"]:
        raise AppError(
            f"AI provider '{name}' is disabled and cannot be the default",
            code="conflict",
            hint=f"Enable it first with `kassiber ai providers update {name} --enable`.",
        )
    set_setting(conn, DEFAULT_AI_PROVIDER_SETTING, name)
    conn.commit()
    return {"default": name}


def clear_default_ai_provider(conn) -> dict:
    """Clear the stored default AI provider."""
    _ensure_seeded(conn)
    conn.execute(
        "DELETE FROM settings WHERE key = ?",
        (DEFAULT_AI_PROVIDER_SETTING,),
    )
    conn.commit()
    return {"default": None, "cleared": True}


def _last_checked_at(conn, provider: dict) -> str | None:
    """When the provider was last checked at its current base URL, if ever."""

    row = conn.execute(
        """
        SELECT MAX(checked_at) AS checked_at FROM ai_provider_checks
        WHERE provider_name = ? AND base_url = ?
        """,
        (provider["name"], provider["base_url"]),
    ).fetchone()
    return row["checked_at"] if row else None


def _is_untouched_seed(provider: dict) -> bool:
    """A built-in row Kassiber added, still at its built-in address and name."""

    return any(
        seed["base_url"].lower() == str(provider.get("base_url") or "").lower()
        and seed["display_name"] == provider.get("display_name")
        for seed in DEFAULT_BOOTSTRAP_PROVIDERS
    )


def provider_is_set_up(provider: dict, *, is_default: bool, last_checked_at: str | None) -> bool:
    """Whether the user has done anything with this provider.

    A provider the user added always counts. A built-in one counts once it is
    the default, has been checked at its address, or (off-device) has been
    acknowledged; until then Settings folds it away as "not set up". It is a
    display grouping only: it never grants or withholds contact.
    """
    if not _is_untouched_seed(provider) or is_default or last_checked_at:
        return True
    return provider.get("kind") != "local" and bool(provider.get("acknowledged_at"))


def list_with_default(conn) -> dict:
    """List providers + active default in one call (used by the CLI/daemon)."""
    providers = list_db_ai_providers(conn)
    default_name = get_default_ai_provider_name(conn)
    rows = []
    for provider in providers:
        row = redact_ai_provider_for_output(provider, default_name=default_name)
        last_checked_at = _last_checked_at(conn, provider)
        row["last_checked_at"] = last_checked_at
        row["set_up"] = provider_is_set_up(
            provider,
            is_default=provider.get("name") == default_name,
            last_checked_at=last_checked_at,
        )
        rows.append(row)
    return {"providers": rows, "default": default_name}


def acknowledge_remote_use(conn, name: str) -> dict:
    """Stamp explicit off-device acknowledgement for an AI provider."""
    _ensure_seeded(conn)
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
            "or confirm the provider in Settings → Assistant."
        ),
        details={"provider": name, "kind": kind},
        retryable=False,
    )


def require_ai_test_target_acknowledged(
    *,
    base_url: str,
    kind: str | None,
    acknowledged: bool,
    saved: dict | None = None,
) -> None:
    """Gate a connection test before it contacts anything.

    The test sends a prompt over HTTP and starts a CLI agent, so an
    off-device target needs the same acknowledgement as chat. Only a provider
    marked local whose URL is loopback HTTP is on this device; a ``local``
    claim for any other URL counts as off-device. Off-device targets pass
    with the caller's explicit ``acknowledged`` (the provider form's
    checkbox), or with the saved provider's own acknowledgement when that row
    is off-device too: acknowledging a local row never covered sending
    prompts off the device, as when a local provider becomes remote.
    """
    if kind == "local" and _is_loopback_ai_http_url(base_url):
        return
    if acknowledged:
        return
    if (
        saved is not None
        and saved.get("kind") in ("remote", "tee")
        and saved.get("acknowledged_at")
    ):
        return
    name = str_or_none((saved or {}).get("name"))
    raise AppError(
        "Testing this AI provider sends a prompt off this device and needs explicit acknowledgement first",
        code="ai_remote_ack_required",
        hint=(
            "Tick the acknowledgement in the provider form, or acknowledge the saved provider "
            + (f"with `kassiber ai providers update {name} --acknowledge`." if name else "first.")
        ),
        details={"kind": kind or "remote", **({"provider": name} if name else {})},
        retryable=False,
    )


def require_ai_provider_enabled(provider: dict) -> None:
    """Refuse to use a provider the user switched off."""
    if provider.get("enabled", True):
        return
    name = str_or_none(provider.get("name")) or "<unknown>"
    raise AppError(
        f"AI provider '{name}' is disabled",
        code="ai_provider_disabled",
        hint=(
            f"Enable it with `kassiber ai providers update {name} --enable`, "
            "or switch it on in Settings → Assistant."
        ),
        details={"provider": name},
        retryable=False,
    )


def require_ai_provider_contactable(provider: dict) -> None:
    """Gate every path that contacts a provider: chat, model listing, checks.

    A switched-off provider is refused first, then an off-device provider the
    user never acknowledged. CLI, daemon and import paths share this one gate
    so none of them can reach a provider the others would refuse.
    """
    require_ai_provider_enabled(provider)
    require_ai_provider_acknowledged(provider)


def require_ai_provider_endpoint(provider: dict, expected_base_url: object) -> None:
    """Refuse a request bound to an endpoint the provider row no longer has.

    A caller that names the ``base_url`` it means to check (the desktop does)
    must not reach a different endpoint because the row was edited meanwhile,
    in Settings or from another process. ``None`` keeps older callers working.
    """
    if expected_base_url is None:
        return
    if not isinstance(expected_base_url, str):
        raise AppError("AI provider base_url must be a string", code="validation")
    name = str_or_none(provider.get("name")) or "<unknown>"
    if normalize_base_url(expected_base_url) == normalize_base_url(provider.get("base_url")):
        return
    raise AppError(
        f"AI provider '{name}' points at a different endpoint than the one requested",
        code="ai_provider_changed",
        hint="The provider was edited meanwhile. Review it and check it again.",
        details={"provider": name},
        retryable=False,
    )


def contactable_cli_runtimes(conn) -> tuple[str, ...]:
    """CLI runtimes the all-runtimes probe may start, in locator order.

    A runtime qualifies only through a provider row that passes
    `require_ai_provider_contactable`: enabled, and acknowledged for
    off-device use. A runtime with no such row, including one with no row at
    all or a freshly seeded unacknowledged row, is never started, so the
    unscoped probe can reach nothing a per-provider check would refuse.
    """
    allowed: set[str] = set()
    for provider in list_db_ai_providers(conn):
        runtime = cli_provider_for_locator(provider.get("base_url"))
        if runtime is None or runtime in allowed:
            continue
        try:
            require_ai_provider_contactable(provider)
        except AppError:
            continue
        allowed.add(runtime)
    return tuple(
        runtime
        for runtime in dict.fromkeys(CLI_PROVIDER_BY_LOCATOR.values())
        if runtime in allowed
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
