"""Backend (chain-indexer endpoint) discovery, storage, and bootstrap.

A "backend" in kassiber is a pointer to an external blockchain indexer
(esplora, a mempool-compatible API, an Electrum server, a liquid-esplora
instance, etc.) that wallets use to sync transactions. The SQLite
`backends` table is the canonical storage path. Environment / dotenv
configuration now plays a narrower role:

1. **Bootstrap / compatibility seed** — built-in defaults plus any
   `KASSIBER_BACKEND_<NAME>_<FIELD>` variables loaded by
   `load_runtime_config`.
2. **Database** — canonical rows in the `backends` table plus the stored
   default-backend settings. `seed_db_backends` copies non-ephemeral
   bootstrap config into SQLite during explicit bootstrap-import flows, and
   `merge_db_backends` then rebuilds the runtime view from SQLite.

`resolve_backend(runtime_config, name)` is the single entry point used
by sync code to fetch the selected backend dict. CRUD helpers
(`create_db_backend`, `update_db_backend`, `delete_db_backend`,
`set_default_backend`, `clear_default_backend`) are exposed for the
`kassiber backends ...` CLI surface.

Call sites should treat backend dicts as opaque — read values with
`backend_value(backend, "field", "alt_field")` rather than subscripting
directly, since env-sourced and DB-sourced dicts differ slightly.
"""

import json
import os
from pathlib import Path
from urllib.parse import urlsplit, urlunsplit

from .db import (
    DEFAULT_CONFIG_DIRNAME,
    DEFAULT_DATA_ROOT,
    get_setting,
    resolve_effective_state_root,
    set_setting,
)
from .errors import AppError
from .time_utils import now_iso
from .util import (
    normalize_chain_value,
    normalize_network_value,
    parse_bool,
    parse_int,
    str_or_none,
)


DEFAULT_BACKENDS = {
    "mempool": {
        "name": "mempool",
        "kind": "esplora",
        "chain": "bitcoin",
        "network": "main",
        "url": "https://mempool.bitcoin-austria.at/api",
        "source": "built-in default",
    },
    "fulcrum": {
        "name": "fulcrum",
        "kind": "electrum",
        "chain": "bitcoin",
        "network": "main",
        "url": "ssl://index.bitcoin-austria.at:50002",
        "batch_size": 100,
        "source": "built-in default",
    },
    "liquid": {
        "name": "liquid",
        "kind": "electrum",
        "chain": "liquid",
        "network": "liquidv1",
        "url": "ssl://les.bullbitcoin.com:995",
        "batch_size": 100,
        "source": "built-in default",
    },
}

BACKEND_KINDS = {"bitcoinrpc", "custom", "electrum", "esplora", "liquid-esplora", "mempool"}
DEFAULT_ENV_FILENAME = "backends.env"
DEFAULT_BACKEND_SETTING = "default_backend"
BOOTSTRAP_DEFAULT_BACKEND_SETTING = "bootstrap_default_backend"
BOOTSTRAP_BACKEND_TOMBSTONES_SETTING = "bootstrap_backend_tombstones"
BACKEND_DB_FIELDS = {
    "name",
    "kind",
    "chain",
    "network",
    "url",
    "auth_header",
    "token",
    "batch_size",
    "timeout",
    "tor_proxy",
    "config_json",
    "notes",
    "created_at",
    "updated_at",
}
BACKEND_RUNTIME_METADATA_FIELDS = {"config", "is_default", "source"}
BACKEND_RESERVED_FIELDS = BACKEND_DB_FIELDS | BACKEND_RUNTIME_METADATA_FIELDS
BACKEND_BOOLEAN_CONFIG_FIELDS = {"insecure"}
BACKEND_CONFIG_FIELDS = {"cookiefile", "insecure", "password", "username", "walletprefix"}
BACKEND_CONFIG_KEY_ALIASES = {
    "cookie_file": "cookiefile",
    "rpcpassword": "password",
    "rpc_password": "password",
    "rpcuser": "username",
    "rpc_user": "username",
    "wallet_prefix": "walletprefix",
}
BACKEND_CLEAR_FIELD_ALIASES = {
    "auth-header": "auth_header",
    "token": "token",
    "tor-proxy": "tor_proxy",
    "notes": "notes",
    "insecure": "insecure",
    "cookiefile": "cookiefile",
    "username": "username",
    "password": "password",
    "wallet-prefix": "walletprefix",
}
BACKEND_OUTPUT_PRESENCE_FIELDS = {
    "has_auth_header": ("auth_header",),
    "has_token": ("token",),
    "has_cookiefile": ("cookiefile", "cookie_file"),
    "has_username": ("username", "rpcuser", "rpc_user"),
    "has_password": ("password", "rpcpassword", "rpc_password"),
}
BACKEND_SAFE_OUTPUT_FIELDS = (
    "name",
    "kind",
    "chain",
    "network",
    "batch_size",
    "timeout",
    "tor_proxy",
    "notes",
    "source",
    "created_at",
    "updated_at",
    "default",
    "is_default",
)
BACKEND_SAFE_CONFIG_OUTPUT_FIELDS = ("insecure", "walletprefix")


def _canonicalize_backend_field_name(field_name):
    key = str_or_none(field_name)
    if key is None:
        return None
    key = key.lower().replace("-", "_")
    return BACKEND_CONFIG_KEY_ALIASES.get(key, key)


def resolve_effective_env_file(env_file=None, data_root=None):
    """Pick the active backend config path.

    Defaults follow the effective data root so Kassiber's config lands in the
    same hidden home folder as the SQLite store. If an older `.env` already
    exists inside that state directory, keep using it until the user moves it.
    """
    if env_file:
        return Path(env_file).expanduser()
    state_root = Path(resolve_effective_state_root(data_root or DEFAULT_DATA_ROOT)).expanduser()
    preferred = state_root / DEFAULT_CONFIG_DIRNAME / DEFAULT_ENV_FILENAME
    legacy_candidates = (
        state_root / "config.env",
        state_root / ".env",
    )
    if preferred.exists():
        return preferred
    for legacy in legacy_candidates:
        if legacy.exists():
            return legacy
    return preferred


def load_dotenv_file(path):
    """Read a `.env` file into a flat `{key: value}` dict.

    Tolerates comments, blanks, and quoted values. Missing file -> `{}`.
    Silently ignores malformed lines (no `=`) rather than raising.
    """
    values = {}
    if not path.exists():
        return values
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        value = value.strip()
        if value.startswith(("'", '"')) and value.endswith(("'", '"')) and len(value) >= 2:
            value = value[1:-1]
        values[key] = value
    return values


def load_runtime_config(env_file):
    """Build the bootstrap backend config from env / dotenv only.

    Returns a dict with `env_file`, `env_file_exists`, `default_backend`,
    `default_backend_source`, and `backends` (a name->config dict). Call
    `seed_db_backends` and `merge_db_backends` on top to canonicalize the
    runtime view through SQLite.

    Supports both `KASSIBER_BACKEND_*` and the legacy `SATBOOKS_BACKEND_*`
    prefixes so pre-rename configs keep working.
    """
    env_path = Path(env_file).expanduser()
    file_env = load_dotenv_file(env_path)
    merged_env = {**file_env, **os.environ}
    process_env_overrides = {"backends": {}, "default_backend": False}
    dotenv_backends = set()
    backends = {name: dict(config) for name, config in DEFAULT_BACKENDS.items()}
    for prefix in ("SATBOOKS_BACKEND_", "KASSIBER_BACKEND_"):
        for key, value in merged_env.items():
            if not key.startswith(prefix):
                continue
            suffix = key[len(prefix):]
            if "_" not in suffix:
                continue
            backend_name, field_name = suffix.split("_", 1)
            backend_name = backend_name.lower()
            field_name = _canonicalize_backend_field_name(field_name)
            if not backend_name or not field_name:
                continue
            backends.setdefault(
                backend_name,
                {
                    "name": backend_name,
                    "kind": "",
                    "url": "",
                    "source": f"{env_path}" if key in file_env else "environment",
                },
            )
            backends[backend_name][field_name] = value.strip()
            backends[backend_name]["source"] = f"{env_path}" if key in file_env else "environment"
            if key in file_env:
                dotenv_backends.add(backend_name)
            if key in os.environ:
                process_env_overrides["backends"].setdefault(backend_name, set()).add(field_name)
    default_backend = (
        merged_env.get("KASSIBER_DEFAULT_BACKEND")
        or merged_env.get("SATBOOKS_DEFAULT_BACKEND")
        or "mempool"
    ).strip().lower() or "mempool"
    default_backend_source = "built-in default"
    if "KASSIBER_DEFAULT_BACKEND" in file_env or "SATBOOKS_DEFAULT_BACKEND" in file_env:
        default_backend_source = str(env_path)
    if "KASSIBER_DEFAULT_BACKEND" in os.environ or "SATBOOKS_DEFAULT_BACKEND" in os.environ:
        default_backend_source = "environment"
        process_env_overrides["default_backend"] = True
    if default_backend not in backends:
        raise AppError(
            f"Default backend '{default_backend}' is not defined. Add KASSIBER_BACKEND_{default_backend.upper()}_KIND and _URL to {env_path}."
        )
    for name, backend in backends.items():
        if not backend.get("kind") or not backend.get("url"):
            raise AppError(f"Backend '{name}' is missing kind or url configuration")
    return {
        "env_file": str(env_path),
        "env_file_exists": env_path.exists(),
        "default_backend": default_backend,
        "default_backend_source": default_backend_source,
        "dotenv_backends": sorted(dotenv_backends),
        "process_env_overrides": {
            "backends": {
                name: sorted(fields)
                for name, fields in process_env_overrides["backends"].items()
            },
            "default_backend": process_env_overrides["default_backend"],
        },
        "backends": backends,
    }


def backend_value(backend, *keys):
    """First non-empty value across `keys`, or `None` if all are absent/blank."""
    for key in keys:
        value = str_or_none(backend.get(key))
        if value is not None:
            return value
    return None


def backend_timeout(backend, default=30):
    """Read `timeout` off a backend dict, coerced to int with a fallback."""
    return parse_int(backend_value(backend, "timeout"), default)


def backend_batch_size(backend, default=100):
    """Read `batch_size` off a backend dict, coerced to a positive int."""
    value = parse_int(backend_value(backend, "batch_size"), default)
    if value <= 0:
        raise AppError("Backend batch_size must be positive")
    return value


def resolve_backend(runtime_config, name=None):
    """Look up a backend by name in `runtime_config`, defaulting to the active one."""
    backend_name = (name or runtime_config["default_backend"]).strip().lower()
    backend = runtime_config["backends"].get(backend_name)
    if not backend:
        raise AppError(f"Backend '{backend_name}' is not configured in {runtime_config['env_file']}")
    return backend


def list_backends(runtime_config):
    """Flatten `runtime_config["backends"]` into display rows for the CLI."""
    rows = []
    for name, backend in sorted(runtime_config["backends"].items()):
        rows.append(
            {
                "name": name,
                "kind": backend["kind"],
                "chain": backend.get("chain", ""),
                "network": backend.get("network", ""),
                "url": backend["url"],
                "batch_size": backend.get("batch_size", ""),
                "default": "yes" if name == runtime_config["default_backend"] else "",
                "source": backend["source"],
            }
        )
    return rows


def _load_backend_config(raw_config):
    if raw_config in (None, ""):
        return {}
    try:
        payload = json.loads(raw_config)
    except (TypeError, ValueError, json.JSONDecodeError):
        return {}
    return payload if isinstance(payload, dict) else {}


def _normalize_backend_config(config):
    cleaned = {}
    for raw_key, raw_value in (config or {}).items():
        key = _canonicalize_backend_field_name(raw_key)
        if key is None:
            continue
        if key in BACKEND_RESERVED_FIELDS:
            continue
        if key in BACKEND_BOOLEAN_CONFIG_FIELDS:
            cleaned[key] = parse_bool(raw_value)
            continue
        value = str_or_none(raw_value)
        if value is None:
            continue
        cleaned[key] = value
    return cleaned


def _extract_backend_config(backend):
    return _normalize_backend_config(
        {
            key: value
            for key, value in backend.items()
            if key not in BACKEND_RESERVED_FIELDS
        }
    )


def _normalize_backend_config_patch(config):
    cleaned = {}
    cleared = set()
    for raw_key, raw_value in (config or {}).items():
        key = _canonicalize_backend_field_name(raw_key)
        if key is None:
            continue
        if key in BACKEND_RESERVED_FIELDS:
            continue
        if key in BACKEND_BOOLEAN_CONFIG_FIELDS:
            if str_or_none(raw_value) is None:
                cleared.add(key)
                continue
            cleaned[key] = parse_bool(raw_value)
            continue
        value = str_or_none(raw_value)
        if value is None:
            cleared.add(key)
            continue
        cleaned[key] = value
    return cleaned, cleared


def redact_backend_url(url):
    value = str_or_none(url)
    if value is None:
        return ""
    try:
        parts = urlsplit(value)
    except ValueError:
        return value
    if not parts.scheme or not parts.netloc:
        return value
    hostname = parts.hostname or ""
    if not hostname:
        return value
    host = hostname
    if ":" in host and not host.startswith("["):
        host = f"[{host}]"
    if parts.port is not None:
        host = f"{host}:{parts.port}"
    if parts.username or parts.password:
        host = f"<redacted>@{host}"
    return urlunsplit((parts.scheme, host, parts.path, "", ""))


def redact_backend_for_output(backend):
    payload = {}
    for field in BACKEND_SAFE_OUTPUT_FIELDS:
        if field in backend:
            payload[field] = backend[field]
    if "url" in backend:
        payload["url"] = redact_backend_url(backend.get("url"))
    for field in BACKEND_SAFE_CONFIG_OUTPUT_FIELDS:
        if field not in backend:
            continue
        if field in BACKEND_BOOLEAN_CONFIG_FIELDS:
            value = backend.get(field)
            try:
                payload[field] = value if isinstance(value, bool) else parse_bool(value)
            except AppError:
                payload[field] = value
            continue
        value = str_or_none(backend.get(field))
        if value is not None:
            payload[field] = value
    for flag, keys in BACKEND_OUTPUT_PRESENCE_FIELDS.items():
        payload[flag] = any(str_or_none(backend.get(key)) is not None for key in keys)
    return payload


def _available_backend_names(conn):
    rows = conn.execute("SELECT name FROM backends ORDER BY name ASC").fetchall()
    return {row["name"] for row in rows}


def _wallet_backend_references(conn, backend_name):
    rows = conn.execute(
        """
        SELECT
            w.label AS wallet_label,
            p.label AS profile_label,
            ws.label AS workspace_label,
            w.config_json
        FROM wallets w
        JOIN profiles p ON p.id = w.profile_id
        JOIN workspaces ws ON ws.id = w.workspace_id
        ORDER BY ws.label ASC, p.label ASC, w.label ASC
        """
    ).fetchall()
    matches = []
    for row in rows:
        try:
            config = json.loads(row["config_json"] or "{}")
        except (TypeError, ValueError, json.JSONDecodeError):
            continue
        if str_or_none(config.get("backend")) != backend_name:
            continue
        matches.append(f"{row['workspace_label']}/{row['profile_label']}/{row['wallet_label']}")
    return matches


def _load_bootstrap_backend_tombstones(conn):
    raw = get_setting(conn, BOOTSTRAP_BACKEND_TOMBSTONES_SETTING)
    if not raw:
        return set()
    try:
        payload = json.loads(raw)
    except (TypeError, ValueError, json.JSONDecodeError):
        return set()
    if not isinstance(payload, list):
        return set()
    return {
        name
        for name in (str_or_none(item) for item in payload)
        if name is not None
    }


def _save_bootstrap_backend_tombstones(conn, names):
    set_setting(conn, BOOTSTRAP_BACKEND_TOMBSTONES_SETTING, json.dumps(sorted(names)))


def _process_env_backend_fields(runtime_config, name):
    backends = runtime_config.get("process_env_overrides", {}).get("backends", {})
    return set(backends.get(name, ()))


def _dotenv_backend_names(runtime_config):
    return set(runtime_config.get("dotenv_backends", ()))


def _process_env_default_backend_override(runtime_config):
    return bool(runtime_config.get("process_env_overrides", {}).get("default_backend"))


def _fallback_backend_name(names):
    if "mempool" in names:
        return "mempool"
    if names:
        return sorted(names)[0]
    raise AppError(
        "No backends are configured",
        code="config_error",
        hint="Create a backend with `kassiber backends create`, or seed one through your dotenv bootstrap config.",
    )


def _seedable_runtime_backend(name, backend):
    if backend.get("source") == "environment":
        return None
    return {
        "name": name,
        "kind": backend_value(backend, "kind"),
        "chain": backend_value(backend, "chain"),
        "network": backend_value(backend, "network"),
        "url": backend_value(backend, "url"),
        "auth_header": backend_value(backend, "auth_header"),
        "token": backend_value(backend, "token"),
        "batch_size": parse_int(backend_value(backend, "batch_size"), None),
        "timeout": parse_int(backend_value(backend, "timeout"), None),
        "tor_proxy": backend_value(backend, "tor_proxy"),
        "config": _extract_backend_config(backend),
    }


def _insert_seed_backend(conn, payload):
    kind = _validate_backend_kind(payload["kind"])
    url = payload["url"]
    if not url or not url.strip():
        raise AppError("Backend url is required", code="validation")
    chain = payload["chain"]
    if chain:
        chain = normalize_chain_value(chain)
    network = payload["network"]
    if network:
        network = normalize_network_value(chain, network)
    batch_size = payload["batch_size"]
    if batch_size is not None and batch_size <= 0:
        raise AppError("Backend batch size must be positive", code="validation")
    timeout = payload["timeout"]
    if timeout is not None and timeout <= 0:
        raise AppError("Backend timeout must be positive", code="validation")
    ts = now_iso()
    cursor = conn.execute(
        """
        INSERT INTO backends(name, kind, chain, network, url, auth_header, token, batch_size, timeout, tor_proxy, config_json, notes, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(name) DO NOTHING
        """,
        (
            payload["name"],
            kind,
            chain,
            network,
            url.strip(),
            payload["auth_header"],
            payload["token"],
            batch_size,
            timeout,
            payload["tor_proxy"],
            json.dumps(_normalize_backend_config(payload["config"]), sort_keys=True),
            None,
            ts,
            ts,
        ),
    )
    return cursor.rowcount > 0


def seed_db_backends(conn, runtime_config):
    """Persist built-in / dotenv bootstrap backends during explicit import flows.

    SQLite remains the canonical storage path, so built-ins and dotenv-backed
    definitions can be copied into SQLite when the caller explicitly asks for
    bootstrap import. Process-level environment-only overrides stay ephemeral
    and are not auto-written into the database.
    """
    try:
        conn.execute("BEGIN IMMEDIATE")
        changed = False
        tombstones = _load_bootstrap_backend_tombstones(conn)
        resurrected = tombstones & _dotenv_backend_names(runtime_config)
        if resurrected:
            tombstones -= resurrected
            _save_bootstrap_backend_tombstones(conn, tombstones)
            changed = True

        for name, backend in sorted(runtime_config["backends"].items()):
            if name in tombstones:
                continue
            payload = _seedable_runtime_backend(name, backend)
            if payload is None:
                continue
            if _insert_seed_backend(conn, payload):
                changed = True

        existing_names = _available_backend_names(conn)
        bootstrap_default = get_setting(conn, BOOTSTRAP_DEFAULT_BACKEND_SETTING)
        if not bootstrap_default:
            candidate = runtime_config["default_backend"]
            if candidate not in existing_names:
                candidate = _fallback_backend_name(existing_names)
            set_setting(conn, BOOTSTRAP_DEFAULT_BACKEND_SETTING, candidate)
            bootstrap_default = candidate
            changed = True

        stored_default = get_setting(conn, DEFAULT_BACKEND_SETTING)
        if not stored_default:
            set_setting(conn, DEFAULT_BACKEND_SETTING, bootstrap_default)
            changed = True
        conn.commit()
    except Exception:
        conn.rollback()
        raise
    return runtime_config


def _backend_row_to_dict(row):
    payload = {
        "name": row["name"],
        "kind": row["kind"],
        "chain": row["chain"] or "",
        "network": row["network"] or "",
        "url": row["url"],
        "batch_size": row["batch_size"],
        "auth_header": row["auth_header"] or "",
        "token": row["token"] or "",
        "timeout": row["timeout"],
        "tor_proxy": row["tor_proxy"] or "",
        "notes": row["notes"] or "",
        "source": "database",
        "created_at": row["created_at"],
        "updated_at": row["updated_at"],
    }
    payload.update(_load_backend_config(row["config_json"]))
    return payload


def list_db_backends(conn):
    """Return all rows from the `backends` table, sorted by name."""
    rows = conn.execute("SELECT * FROM backends ORDER BY name ASC").fetchall()
    return [_backend_row_to_dict(row) for row in rows]


def get_db_backend(conn, name):
    """Fetch a single backend row by name, or raise `AppError(not_found)`."""
    row = conn.execute("SELECT * FROM backends WHERE name = ?", (name.lower(),)).fetchone()
    if not row:
        raise AppError(
            f"Backend '{name}' not found in the database",
            code="not_found",
            hint="Use `kassiber backends list` to see configured backends, or `kassiber backends create` to add one.",
        )
    return _backend_row_to_dict(row)


def merge_db_backends(conn, runtime_config):
    """Overlay SQLite-backed backends and the stored default on bootstrap config.

    Mutates and returns `runtime_config`. DB rows overwrite any bootstrap
    backend with the same name. A stored `default_backend` setting, if
    present, also overrides the bootstrap default — and raises if it names a
    backend that is not available in the merged runtime view.
    """
    tombstones = _load_bootstrap_backend_tombstones(conn) - _dotenv_backend_names(runtime_config)
    for name in list(runtime_config["backends"]):
        if name in tombstones and not _process_env_backend_fields(runtime_config, name):
            runtime_config["backends"].pop(name, None)
    rows = conn.execute("SELECT * FROM backends").fetchall()
    for row in rows:
        name = row["name"].lower()
        db_backend = _backend_row_to_dict(row)
        env_fields = _process_env_backend_fields(runtime_config, name)
        if not env_fields:
            runtime_config["backends"][name] = db_backend
            continue
        env_backend = runtime_config["backends"].get(name, {})
        merged = dict(db_backend)
        for field in env_fields:
            merged[field] = env_backend.get(field, "")
        merged["source"] = "environment"
        runtime_config["backends"][name] = merged
    if _process_env_default_backend_override(runtime_config):
        if runtime_config["default_backend"] not in runtime_config["backends"]:
            raise AppError(
                f"Environment default backend '{runtime_config['default_backend']}' is not configured",
                code="config_error",
                hint="Define that backend in the current process or remove the env override.",
            )
        return runtime_config
    override = get_setting(conn, DEFAULT_BACKEND_SETTING)
    if override:
        if override not in runtime_config["backends"]:
            raise AppError(
                f"Stored default backend '{override}' is not configured",
                code="config_error",
                hint="Run `kassiber backends set-default <name>` with a valid backend, or `backends clear-default` to fall back to the env default.",
            )
        runtime_config["default_backend"] = override
    return runtime_config


def _validate_backend_kind(kind):
    if kind.lower() not in BACKEND_KINDS:
        raise AppError(
            f"Unsupported backend kind '{kind}'",
            code="validation",
            hint=f"Choose one of: {', '.join(sorted(BACKEND_KINDS))}",
        )
    return kind.lower()


def create_db_backend(
    conn,
    name,
    kind,
    url,
    chain=None,
    network=None,
    auth_header=None,
    token=None,
    batch_size=None,
    timeout=None,
    tor_proxy=None,
    config=None,
    notes=None,
):
    """Insert a new backend row. Raises on name conflict or invalid kind/url."""
    name = name.strip().lower()
    if not name:
        raise AppError("Backend name is required", code="validation")
    kind = _validate_backend_kind(kind)
    if not url or not url.strip():
        raise AppError("Backend url is required", code="validation")
    if chain:
        chain = normalize_chain_value(chain)
    if network:
        network = normalize_network_value(chain, network)
    if batch_size is not None and batch_size <= 0:
        raise AppError("Backend batch size must be positive", code="validation")
    if timeout is not None and timeout <= 0:
        raise AppError("Backend timeout must be positive", code="validation")
    normalized_config = _normalize_backend_config(config)
    existing = conn.execute("SELECT 1 FROM backends WHERE name = ?", (name,)).fetchone()
    if existing:
        raise AppError(
            f"Backend '{name}' already exists",
            code="conflict",
            hint="Use `kassiber backends update` to change an existing backend.",
        )
    ts = now_iso()
    conn.execute(
        """
        INSERT INTO backends(name, kind, chain, network, url, auth_header, token, batch_size, timeout, tor_proxy, config_json, notes, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            name,
            kind,
            chain,
            network,
            url.strip(),
            auth_header,
            token,
            batch_size,
            timeout,
            tor_proxy,
            json.dumps(normalized_config, sort_keys=True),
            notes,
            ts,
            ts,
        ),
    )
    tombstones = _load_bootstrap_backend_tombstones(conn)
    if name in tombstones:
        tombstones.remove(name)
        _save_bootstrap_backend_tombstones(conn, tombstones)
    conn.commit()
    return get_db_backend(conn, name)


def update_db_backend(conn, name, updates):
    """Apply a partial update to a backend row.

    `updates` is a dict where `None` values mean "leave alone" — so the
    caller can pass every field unconditionally. Raises if every field is
    `None` (nothing to update) or if the backend does not exist.
    """
    name = name.strip().lower()
    row = conn.execute("SELECT * FROM backends WHERE name = ?", (name,)).fetchone()
    if not row:
        raise AppError(
            f"Backend '{name}' not found in the database",
            code="not_found",
            hint="Only DB-backed backends can be updated. Use `kassiber backends create` first.",
        )
    clear_fields = set(updates.get("clear") or [])
    if all(v is None for key, v in updates.items() if key != "clear") and not clear_fields:
        raise AppError(
            "backends update requires at least one field to change",
            code="validation",
            hint="Pass one or more of --kind, --url, --chain, --network, --auth-header, --token, --batch-size, --timeout, --tor-proxy, --insecure, --cookiefile, --username, --password, --wallet-prefix, --notes, or --clear <field>",
        )
    unsupported_clear_fields = clear_fields - set(BACKEND_CLEAR_FIELD_ALIASES.values())
    if unsupported_clear_fields:
        raise AppError(
            f"Unsupported backend clear field(s): {', '.join(sorted(unsupported_clear_fields))}",
            code="validation",
            hint=f"Choose one of: {', '.join(sorted(BACKEND_CLEAR_FIELD_ALIASES))}",
        )
    conflicting_clear_fields = [
        field
        for field in ("auth_header", "token", "tor_proxy", "notes")
        if field in clear_fields and updates.get(field) is not None
    ]
    if conflicting_clear_fields:
        raise AppError(
            f"Cannot set and clear backend field(s) in the same update: {', '.join(sorted(conflicting_clear_fields))}",
            code="validation",
        )

    new_kind = updates.get("kind")
    if new_kind is not None:
        new_kind = _validate_backend_kind(new_kind)
    new_url = updates.get("url")
    if new_url is not None:
        if not new_url.strip():
            raise AppError("Backend url cannot be empty", code="validation")
        new_url = new_url.strip()
    new_chain = updates.get("chain")
    if new_chain is not None:
        new_chain = normalize_chain_value(new_chain)
    new_network = updates.get("network")
    if new_network is not None:
        chain_for_net = new_chain or row["chain"]
        new_network = normalize_network_value(chain_for_net, new_network)
    new_batch_size = updates.get("batch_size")
    if new_batch_size is not None and new_batch_size <= 0:
        raise AppError("Backend batch size must be positive", code="validation")
    new_timeout = updates.get("timeout")
    if new_timeout is not None and new_timeout <= 0:
        raise AppError("Backend timeout must be positive", code="validation")
    new_config = updates.get("config")
    merged_config = _load_backend_config(row["config_json"])
    config_patch, config_clears = _normalize_backend_config_patch(new_config)
    for field in clear_fields & BACKEND_CONFIG_FIELDS:
        config_clears.add(field)
    if new_config is not None:
        conflicting = set(config_patch) & config_clears
        if conflicting:
            raise AppError(
                f"Cannot set and clear backend config field(s) in the same update: {', '.join(sorted(conflicting))}",
                code="validation",
            )
    for field in config_clears:
        merged_config.pop(field, None)
    merged_config.update(config_patch)

    merged = {
        "kind": new_kind if new_kind is not None else row["kind"],
        "url": new_url if new_url is not None else row["url"],
        "chain": new_chain if new_chain is not None else row["chain"],
        "network": new_network if new_network is not None else row["network"],
        "auth_header": None if "auth_header" in clear_fields else (
            updates.get("auth_header") if updates.get("auth_header") is not None else row["auth_header"]
        ),
        "token": None if "token" in clear_fields else (
            updates.get("token") if updates.get("token") is not None else row["token"]
        ),
        "batch_size": new_batch_size if new_batch_size is not None else row["batch_size"],
        "timeout": new_timeout if new_timeout is not None else row["timeout"],
        "tor_proxy": None if "tor_proxy" in clear_fields else (
            updates.get("tor_proxy") if updates.get("tor_proxy") is not None else row["tor_proxy"]
        ),
        "config_json": json.dumps(merged_config, sort_keys=True),
        "notes": None if "notes" in clear_fields else (
            updates.get("notes") if updates.get("notes") is not None else row["notes"]
        ),
    }

    conn.execute(
        """
        UPDATE backends
        SET kind = ?, url = ?, chain = ?, network = ?, auth_header = ?, token = ?, batch_size = ?, timeout = ?, tor_proxy = ?, config_json = ?, notes = ?, updated_at = ?
        WHERE name = ?
        """,
        (
            merged["kind"],
            merged["url"],
            merged["chain"],
            merged["network"],
            merged["auth_header"],
            merged["token"],
            merged["batch_size"],
            merged["timeout"],
            merged["tor_proxy"],
            merged["config_json"],
            merged["notes"],
            now_iso(),
            name,
        ),
    )
    conn.commit()
    return get_db_backend(conn, name)


def delete_db_backend(conn, name):
    """Delete a DB-backed backend, refusing if it is the active stored default."""
    name = name.strip().lower()
    row = conn.execute("SELECT name FROM backends WHERE name = ?", (name,)).fetchone()
    if not row:
        raise AppError(
            f"Backend '{name}' not found in the database",
            code="not_found",
            hint="Only DB-backed backends can be deleted; env-sourced backends are removed from your .env file instead.",
        )
    if get_setting(conn, DEFAULT_BACKEND_SETTING) == name:
        raise AppError(
            f"Backend '{name}' is the stored default; clear it with `kassiber backends clear-default` first",
            code="conflict",
        )
    wallet_refs = _wallet_backend_references(conn, name)
    if wallet_refs:
        raise AppError(
            f"Backend '{name}' is still referenced by wallet configuration",
            code="conflict",
            hint=f"Repoint or update these wallets first: {', '.join(wallet_refs[:5])}",
            details={"wallet_refs": wallet_refs},
        )
    conn.execute("DELETE FROM backends WHERE name = ?", (name,))
    tombstones = _load_bootstrap_backend_tombstones(conn)
    tombstones.add(name)
    _save_bootstrap_backend_tombstones(conn, tombstones)
    conn.commit()
    return {"name": name, "deleted": True}


def set_default_backend(conn, runtime_config, name):
    """Persist `default_backend=<name>` in `settings`; mutate runtime_config to match."""
    name = name.strip().lower()
    if name not in runtime_config["backends"]:
        raise AppError(
            f"Backend '{name}' is not configured",
            code="not_found",
            hint="Use `kassiber backends list` to see available backends.",
        )
    row = conn.execute("SELECT 1 FROM backends WHERE name = ?", (name,)).fetchone()
    if not row:
        raise AppError(
            f"Backend '{name}' only exists as an environment override and cannot be stored as the canonical default",
            code="conflict",
            hint="Create or import that backend into SQLite first, then run `kassiber backends set-default` again.",
        )
    set_setting(conn, DEFAULT_BACKEND_SETTING, name)
    conn.commit()
    runtime_config["default_backend"] = name
    return {"default_backend": name}


def clear_default_backend(conn, runtime_config):
    """Reset the stored default to the bootstrap SQLite default."""
    available_names = _available_backend_names(conn)
    default_name = get_setting(conn, BOOTSTRAP_DEFAULT_BACKEND_SETTING)
    if default_name not in available_names:
        default_name = _fallback_backend_name(available_names)
        set_setting(conn, BOOTSTRAP_DEFAULT_BACKEND_SETTING, default_name)
    set_setting(conn, DEFAULT_BACKEND_SETTING, default_name)
    conn.commit()
    runtime_config["default_backend"] = default_name
    return {"default_backend": default_name, "cleared": True}
