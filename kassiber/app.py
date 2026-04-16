import argparse
import csv
import hashlib
import json
import os
import sqlite3
import sys
import uuid
from collections import defaultdict
from datetime import datetime, timezone
from decimal import Decimal, InvalidOperation
from pathlib import Path
from urllib import error as urlerror
from urllib import parse as urlparse
from urllib import request as urlrequest

from . import __version__


APP_NAME = "kassiber"
LEGACY_APP_NAME = "satbooks"
DEFAULT_DATA_ROOT = os.path.expanduser(f"~/.local/share/{APP_NAME}")
LEGACY_DATA_ROOT = os.path.expanduser(f"~/.local/share/{LEGACY_APP_NAME}")
DEFAULT_DB_FILENAME = f"{APP_NAME}.sqlite3"
LEGACY_DB_FILENAME = f"{LEGACY_APP_NAME}.sqlite3"
DEFAULT_ENV_FILE = ".env"
SATS_PER_BTC = Decimal("100000000")
ACCOUNT_TYPES = {"asset", "liability", "equity", "income", "expense"}
WALLET_KINDS = [
    "descriptor",
    "xpub",
    "address",
    "coreln",
    "lnd",
    "nwc",
    "phoenix",
    "river",
    "custom",
]
INBOUND_DIRECTIONS = {"in", "inbound", "receive", "received", "deposit", "credit", "buy"}
OUTBOUND_DIRECTIONS = {"out", "outbound", "send", "sent", "withdrawal", "withdraw", "debit", "sell"}
DEFAULT_BACKENDS = {
    "mempool": {
        "name": "mempool",
        "kind": "esplora",
        "url": "https://mempool.space/api",
        "source": "built-in default",
    }
}

SCHEMA = """
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS workspaces (
    id TEXT PRIMARY KEY,
    label TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS profiles (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    fiat_currency TEXT NOT NULL DEFAULT 'USD',
    gains_algorithm TEXT NOT NULL DEFAULT 'FIFO',
    last_processed_at TEXT,
    last_processed_tx_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    UNIQUE (workspace_id, label)
);

CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    code TEXT NOT NULL,
    label TEXT NOT NULL,
    account_type TEXT NOT NULL,
    asset TEXT,
    created_at TEXT NOT NULL,
    UNIQUE (profile_id, code)
);

CREATE TABLE IF NOT EXISTS wallets (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    account_id TEXT REFERENCES accounts(id) ON DELETE SET NULL,
    label TEXT NOT NULL,
    kind TEXT NOT NULL,
    config_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    UNIQUE (profile_id, label)
);

CREATE TABLE IF NOT EXISTS transactions (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    external_id TEXT,
    fingerprint TEXT NOT NULL UNIQUE,
    occurred_at TEXT NOT NULL,
    direction TEXT NOT NULL,
    asset TEXT NOT NULL,
    amount REAL NOT NULL,
    fee REAL NOT NULL DEFAULT 0,
    fiat_currency TEXT,
    fiat_rate REAL,
    fiat_value REAL,
    kind TEXT,
    description TEXT,
    counterparty TEXT,
    note TEXT,
    excluded INTEGER NOT NULL DEFAULT 0,
    raw_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS tags (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    code TEXT NOT NULL,
    label TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (profile_id, code)
);

CREATE TABLE IF NOT EXISTS transaction_tags (
    transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    tag_id TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (transaction_id, tag_id)
);

CREATE TABLE IF NOT EXISTS journal_entries (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    account_id TEXT REFERENCES accounts(id) ON DELETE SET NULL,
    occurred_at TEXT NOT NULL,
    entry_type TEXT NOT NULL,
    asset TEXT NOT NULL,
    quantity REAL NOT NULL,
    fiat_value REAL NOT NULL DEFAULT 0,
    unit_cost REAL NOT NULL DEFAULT 0,
    cost_basis REAL,
    proceeds REAL,
    gain_loss REAL,
    description TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS journal_quarantines (
    transaction_id TEXT PRIMARY KEY REFERENCES transactions(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    reason TEXT NOT NULL,
    detail_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);
"""


class AppError(Exception):
    pass


def load_dotenv_file(path):
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
    env_path = Path(env_file).expanduser()
    file_env = load_dotenv_file(env_path)
    merged_env = {**file_env, **os.environ}
    backends = {name: dict(config) for name, config in DEFAULT_BACKENDS.items()}
    for prefix in ("SATBOOKS_BACKEND_", "KASSIBER_BACKEND_"):
        for key, value in merged_env.items():
            if not key.startswith(prefix):
                continue
            suffix = key[len(prefix):]
            parts = suffix.rsplit("_", 1)
            if len(parts) != 2:
                continue
            backend_name, field_name = parts
            backend_name = backend_name.lower()
            field_name = field_name.lower()
            if field_name not in {"kind", "url"}:
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
    default_backend = (
        merged_env.get("KASSIBER_DEFAULT_BACKEND")
        or merged_env.get("SATBOOKS_DEFAULT_BACKEND")
        or "mempool"
    ).strip().lower() or "mempool"
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
        "backends": backends,
    }


def dec(value, default="0"):
    if value is None or value == "":
        return Decimal(default)
    try:
        return Decimal(str(value))
    except InvalidOperation as exc:
        raise AppError(f"Invalid decimal value: {value}") from exc


def now_iso():
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def parse_timestamp(value):
    if not value:
        raise AppError("Missing occurred_at/date value")
    raw = str(value).strip()
    if len(raw) == 10:
        raw = f"{raw}T00:00:00+00:00"
    elif raw.endswith("Z"):
        raw = raw[:-1] + "+00:00"
    dt = datetime.fromisoformat(raw)
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    else:
        dt = dt.astimezone(timezone.utc)
    return dt.replace(microsecond=0).isoformat().replace("+00:00", "Z")


def normalize_code(value):
    code = str(value).strip().lower().replace(" ", "-")
    if not code:
        raise AppError("Code cannot be empty")
    return code


def normalize_wallet_kind(value):
    kind = str(value).strip().lower()
    if kind not in WALLET_KINDS:
        raise AppError(f"Unsupported wallet kind '{value}'. Supported: {', '.join(WALLET_KINDS)}")
    return kind


def normalize_addresses(values):
    if values is None:
        return []
    if isinstance(values, str):
        values = [part.strip() for part in values.split(",")]
    output = []
    seen = set()
    for value in values:
        address = str(value).strip()
        if not address or address in seen:
            continue
        seen.add(address)
        output.append(address)
    return output


def normalize_direction(direction, amount):
    if direction:
        value = str(direction).strip().lower()
        if value in INBOUND_DIRECTIONS:
            return "inbound"
        if value in OUTBOUND_DIRECTIONS:
            return "outbound"
        raise AppError(f"Unsupported direction '{direction}'")
    return "outbound" if dec(amount) < 0 else "inbound"


def ensure_data_root(data_root):
    path = Path(data_root).expanduser()
    path.mkdir(parents=True, exist_ok=True)
    return path


def resolve_effective_data_root(data_root):
    requested = Path(data_root).expanduser()
    if requested == Path(DEFAULT_DATA_ROOT).expanduser():
        legacy = Path(LEGACY_DATA_ROOT).expanduser()
        if not requested.exists() and legacy.exists():
            return legacy
    return requested


def resolve_database_path(data_root):
    root = Path(data_root).expanduser()
    current = root / DEFAULT_DB_FILENAME
    legacy = root / LEGACY_DB_FILENAME
    if current.exists() or not legacy.exists():
        return current
    return legacy


def resolve_backend(runtime_config, name=None):
    backend_name = (name or runtime_config["default_backend"]).strip().lower()
    backend = runtime_config["backends"].get(backend_name)
    if not backend:
        raise AppError(f"Backend '{backend_name}' is not configured in {runtime_config['env_file']}")
    return backend


def http_get_json(url, timeout=30):
    request = urlrequest.Request(
        url,
        headers={
            "Accept": "application/json",
            "User-Agent": f"{APP_NAME}/{__version__}",
        },
    )
    try:
        with urlrequest.urlopen(request, timeout=timeout) as response:
            return json.loads(response.read().decode("utf-8"))
    except urlerror.HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace")
        raise AppError(f"HTTP {exc.code} from backend for {url}: {detail[:200]}") from exc
    except urlerror.URLError as exc:
        raise AppError(f"Failed to reach backend {url}: {exc.reason}") from exc


def open_db(data_root):
    root = ensure_data_root(resolve_effective_data_root(data_root))
    conn = sqlite3.connect(resolve_database_path(root))
    conn.row_factory = sqlite3.Row
    conn.executescript(SCHEMA)
    conn.execute("PRAGMA foreign_keys = ON")
    return conn


def json_ready(value):
    if isinstance(value, sqlite3.Row):
        return {k: json_ready(value[k]) for k in value.keys()}
    if isinstance(value, dict):
        return {k: json_ready(v) for k, v in value.items()}
    if isinstance(value, list):
        return [json_ready(v) for v in value]
    if isinstance(value, Decimal):
        return float(value)
    return value


def emit(args, payload):
    if args.format == "json":
        print(json.dumps(json_ready(payload), indent=2, sort_keys=False))
        return
    if isinstance(payload, list):
        print_table(payload)
    elif isinstance(payload, dict):
        rows = [{"field": key, "value": value} for key, value in payload.items()]
        print_table(rows)
    else:
        print(payload)


def print_table(rows):
    if not rows:
        print("(no rows)")
        return
    normalized = [{key: format_table_value(value) for key, value in row.items()} for row in rows]
    headers = list(normalized[0].keys())
    widths = {header: len(header) for header in headers}
    for row in normalized:
        for header in headers:
            widths[header] = max(widths[header], len(row.get(header, "")))
    header_line = "  ".join(header.ljust(widths[header]) for header in headers)
    separator = "  ".join("-" * widths[header] for header in headers)
    print(header_line)
    print(separator)
    for row in normalized:
        print("  ".join(row.get(header, "").ljust(widths[header]) for header in headers))


def format_table_value(value):
    if value is None:
        return ""
    if isinstance(value, float):
        return f"{value:.8f}".rstrip("0").rstrip(".")
    return str(value)


def set_setting(conn, key, value):
    conn.execute(
        """
        INSERT INTO settings(key, value)
        VALUES(?, ?)
        ON CONFLICT(key) DO UPDATE SET value = excluded.value
        """,
        (key, value),
    )


def get_setting(conn, key):
    row = conn.execute("SELECT value FROM settings WHERE key = ?", (key,)).fetchone()
    return row["value"] if row else None


def resolve_workspace(conn, ref=None):
    ref = ref or get_setting(conn, "context_workspace")
    if not ref:
        raise AppError("No workspace selected. Create one or run `kassiber context set --workspace ...`.")
    row = conn.execute(
        "SELECT * FROM workspaces WHERE id = ? OR lower(label) = lower(?) LIMIT 1",
        (ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Workspace '{ref}' not found")
    return row


def resolve_profile(conn, workspace_id, ref=None):
    ref = ref or get_setting(conn, "context_profile")
    if not ref:
        raise AppError("No profile selected. Create one or run `kassiber context set --profile ...`.")
    row = conn.execute(
        """
        SELECT * FROM profiles
        WHERE workspace_id = ? AND (id = ? OR lower(label) = lower(?))
        LIMIT 1
        """,
        (workspace_id, ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Profile '{ref}' not found in the selected workspace")
    return row


def resolve_scope(conn, workspace_ref=None, profile_ref=None):
    workspace = resolve_workspace(conn, workspace_ref)
    profile = resolve_profile(conn, workspace["id"], profile_ref)
    return workspace, profile


def resolve_account(conn, profile_id, ref):
    row = conn.execute(
        """
        SELECT * FROM accounts
        WHERE profile_id = ? AND (id = ? OR lower(code) = lower(?) OR lower(label) = lower(?))
        LIMIT 1
        """,
        (profile_id, ref, ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Account '{ref}' not found")
    return row


def resolve_wallet(conn, profile_id, ref):
    row = conn.execute(
        """
        SELECT w.*, a.code AS account_code, a.label AS account_label
        FROM wallets w
        LEFT JOIN accounts a ON a.id = w.account_id
        WHERE w.profile_id = ? AND (w.id = ? OR lower(w.label) = lower(?))
        LIMIT 1
        """,
        (profile_id, ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Wallet '{ref}' not found")
    return row


def resolve_transaction(conn, profile_id, ref):
    row = conn.execute(
        """
        SELECT * FROM transactions
        WHERE profile_id = ? AND (id = ? OR external_id = ?)
        LIMIT 1
        """,
        (profile_id, ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Transaction '{ref}' not found")
    return row


def resolve_tag(conn, profile_id, ref):
    row = conn.execute(
        """
        SELECT * FROM tags
        WHERE profile_id = ? AND (id = ? OR lower(code) = lower(?) OR lower(label) = lower(?))
        LIMIT 1
        """,
        (profile_id, ref, ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Tag '{ref}' not found")
    return row


def invalidate_journals(conn, profile_id):
    conn.execute(
        "UPDATE profiles SET last_processed_at = NULL, last_processed_tx_count = 0 WHERE id = ?",
        (profile_id,),
    )


def init_app(conn):
    set_setting(conn, "app_version", __version__)
    conn.commit()


def create_workspace(conn, label):
    workspace_id = str(uuid.uuid4())
    conn.execute(
        "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
        (workspace_id, label, now_iso()),
    )
    set_setting(conn, "context_workspace", workspace_id)
    conn.commit()
    return conn.execute("SELECT * FROM workspaces WHERE id = ?", (workspace_id,)).fetchone()


def list_workspaces(conn):
    current = get_setting(conn, "context_workspace")
    rows = conn.execute(
        "SELECT id, label, created_at FROM workspaces ORDER BY created_at ASC"
    ).fetchall()
    return [
        {
            "id": row["id"],
            "label": row["label"],
            "current": "yes" if row["id"] == current else "",
            "created_at": row["created_at"],
        }
        for row in rows
    ]


def ensure_default_accounts(conn, workspace_id, profile_id):
    defaults = [
        ("treasury", "Treasury", "asset", "BTC"),
        ("fees", "Fees", "expense", "BTC"),
        ("external", "External", "equity", None),
    ]
    created_at = now_iso()
    for code, label, account_type, asset in defaults:
        exists = conn.execute(
            "SELECT 1 FROM accounts WHERE profile_id = ? AND code = ?",
            (profile_id, code),
        ).fetchone()
        if exists:
            continue
        conn.execute(
            """
            INSERT INTO accounts(id, workspace_id, profile_id, code, label, account_type, asset, created_at)
            VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (str(uuid.uuid4()), workspace_id, profile_id, code, label, account_type, asset, created_at),
        )


def create_profile(conn, workspace_ref, label, fiat_currency, gains_algorithm):
    workspace = resolve_workspace(conn, workspace_ref)
    profile_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO profiles(
            id, workspace_id, label, fiat_currency, gains_algorithm, created_at
        ) VALUES(?, ?, ?, ?, ?, ?)
        """,
        (profile_id, workspace["id"], label, fiat_currency.upper(), gains_algorithm.upper(), now_iso()),
    )
    ensure_default_accounts(conn, workspace["id"], profile_id)
    set_setting(conn, "context_workspace", workspace["id"])
    set_setting(conn, "context_profile", profile_id)
    conn.commit()
    return conn.execute("SELECT * FROM profiles WHERE id = ?", (profile_id,)).fetchone()


def list_profiles(conn, workspace_ref=None):
    workspace = resolve_workspace(conn, workspace_ref)
    current = get_setting(conn, "context_profile")
    rows = conn.execute(
        """
        SELECT id, label, fiat_currency, gains_algorithm, created_at
        FROM profiles
        WHERE workspace_id = ?
        ORDER BY created_at ASC
        """,
        (workspace["id"],),
    ).fetchall()
    return [
        {
            "id": row["id"],
            "label": row["label"],
            "fiat_currency": row["fiat_currency"],
            "gains_algorithm": row["gains_algorithm"],
            "current": "yes" if row["id"] == current else "",
            "created_at": row["created_at"],
        }
        for row in rows
    ]


def create_account(conn, workspace_ref, profile_ref, code, label, account_type, asset=None):
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    code = normalize_code(code)
    account_type = account_type.lower()
    if account_type not in ACCOUNT_TYPES:
        raise AppError(f"Unsupported account type '{account_type}'. Supported: {', '.join(sorted(ACCOUNT_TYPES))}")
    account_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO accounts(id, workspace_id, profile_id, code, label, account_type, asset, created_at)
        VALUES(?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (account_id, workspace["id"], profile["id"], code, label, account_type, asset.upper() if asset else None, now_iso()),
    )
    conn.commit()
    return conn.execute("SELECT * FROM accounts WHERE id = ?", (account_id,)).fetchone()


def list_accounts(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        """
        SELECT id, code, label, account_type, COALESCE(asset, '') AS asset, created_at
        FROM accounts
        WHERE profile_id = ?
        ORDER BY code ASC
        """,
        (profile["id"],),
    ).fetchall()
    return [dict(row) for row in rows]


def parse_wallet_config(args):
    config = {}
    if getattr(args, "config", None):
        config.update(json.loads(args.config))
    if getattr(args, "config_file", None):
        with open(args.config_file, "r", encoding="utf-8") as handle:
            config.update(json.load(handle))
    if getattr(args, "backend", None):
        config["backend"] = args.backend.strip().lower()
    addresses = normalize_addresses(getattr(args, "address", None))
    existing_addresses = normalize_addresses(config.get("addresses"))
    if addresses or existing_addresses:
        config["addresses"] = normalize_addresses(existing_addresses + addresses)
    if getattr(args, "source_file", None):
        config["source_file"] = os.path.abspath(args.source_file)
    if getattr(args, "source_format", None):
        config["source_format"] = args.source_format
    return config


def create_wallet(conn, workspace_ref, profile_ref, label, kind, account_ref=None, config=None):
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    if account_ref:
        account = resolve_account(conn, profile["id"], account_ref)
    else:
        account = resolve_account(conn, profile["id"], "treasury")
    normalized_kind = normalize_wallet_kind(kind)
    config = config or {}
    if normalized_kind == "address" and not config.get("addresses") and not config.get("source_file"):
        raise AppError("Address wallets require at least one --address or a file-based source")
    wallet_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO wallets(id, workspace_id, profile_id, account_id, label, kind, config_json, created_at)
        VALUES(?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            wallet_id,
            workspace["id"],
            profile["id"],
            account["id"],
            label,
            normalized_kind,
            json.dumps(config, sort_keys=True),
            now_iso(),
        ),
    )
    conn.commit()
    return conn.execute("SELECT * FROM wallets WHERE id = ?", (wallet_id,)).fetchone()


def list_wallets(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        """
        SELECT
            w.id,
            w.label,
            w.kind,
            COALESCE(a.code, '') AS account_code,
            COALESCE(a.label, '') AS account_label,
            w.config_json,
            w.created_at
        FROM wallets w
        LEFT JOIN accounts a ON a.id = w.account_id
        WHERE w.profile_id = ?
        ORDER BY w.label ASC
        """,
        (profile["id"],),
    ).fetchall()
    output = []
    for row in rows:
        config = json.loads(row["config_json"] or "{}")
        output.append(
            {
                "id": row["id"],
                "label": row["label"],
                "kind": row["kind"],
                "account": row["account_code"] or row["account_label"],
                "backend": config.get("backend", ""),
                "addresses": ",".join(normalize_addresses(config.get("addresses"))),
                "source_format": config.get("source_format", ""),
                "source_file": config.get("source_file", ""),
                "created_at": row["created_at"],
            }
        )
    return output


def load_import_records(file_path, input_format):
    if input_format == "json":
        with open(file_path, "r", encoding="utf-8") as handle:
            payload = json.load(handle)
        if isinstance(payload, dict):
            payload = payload.get("transactions", [])
        if not isinstance(payload, list):
            raise AppError("JSON import must be a list of transaction objects")
        return payload
    if input_format == "csv":
        with open(file_path, "r", encoding="utf-8", newline="") as handle:
            return list(csv.DictReader(handle))
    raise AppError(f"Unsupported input format '{input_format}'")


def make_transaction_fingerprint(wallet_id, external_id, occurred_at, direction, asset, amount, fee):
    payload = json.dumps(
        {
            "wallet_id": wallet_id,
            "external_id": external_id,
            "occurred_at": occurred_at,
            "direction": direction,
            "asset": asset,
            "amount": str(amount),
            "fee": str(fee),
        },
        sort_keys=True,
    )
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def normalize_import_record(record):
    raw_amount = dec(record.get("amount"))
    direction = normalize_direction(record.get("direction"), raw_amount)
    amount = abs(raw_amount)
    fee = abs(dec(record.get("fee"), "0"))
    fiat_rate = record.get("fiat_rate")
    fiat_value = record.get("fiat_value")
    rate = dec(fiat_rate) if fiat_rate not in (None, "") else None
    value = dec(fiat_value) if fiat_value not in (None, "") else None
    if value is None and rate is not None:
        value = amount * rate
    raw_json = record.get("raw_json")
    if raw_json is None:
        raw_json = json.dumps(json_ready(record), sort_keys=True)
    elif not isinstance(raw_json, str):
        raw_json = json.dumps(json_ready(raw_json), sort_keys=True)
    return {
        "external_id": str(record.get("txid") or record.get("id") or ""),
        "occurred_at": parse_timestamp(record.get("occurred_at") or record.get("timestamp") or record.get("date")),
        "direction": direction,
        "asset": str(record.get("asset") or "BTC").upper(),
        "amount": amount,
        "fee": fee,
        "fiat_rate": rate,
        "fiat_value": value,
        "kind": record.get("kind"),
        "description": record.get("description"),
        "counterparty": record.get("counterparty"),
        "raw_json": raw_json,
    }


def insert_wallet_records(conn, profile, wallet, records, source_label):
    imported = 0
    skipped = 0
    for record in records:
        normalized = normalize_import_record(record)
        fingerprint = make_transaction_fingerprint(
            wallet["id"],
            normalized["external_id"],
            normalized["occurred_at"],
            normalized["direction"],
            normalized["asset"],
            normalized["amount"],
            normalized["fee"],
        )
        exists = conn.execute(
            "SELECT 1 FROM transactions WHERE fingerprint = ?",
            (fingerprint,),
        ).fetchone()
        if exists:
            skipped += 1
            continue
        tx_id = str(uuid.uuid4())
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset, amount, fee, fiat_currency,
                fiat_rate, fiat_value, kind, description, counterparty, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                tx_id,
                profile["workspace_id"],
                profile["id"],
                wallet["id"],
                normalized["external_id"] or None,
                fingerprint,
                normalized["occurred_at"],
                normalized["direction"],
                normalized["asset"],
                float(normalized["amount"]),
                float(normalized["fee"]),
                profile["fiat_currency"],
                float(normalized["fiat_rate"]) if normalized["fiat_rate"] is not None else None,
                float(normalized["fiat_value"]) if normalized["fiat_value"] is not None else None,
                normalized["kind"],
                normalized["description"],
                normalized["counterparty"],
                normalized["raw_json"],
                now_iso(),
            ),
        )
        imported += 1
    invalidate_journals(conn, profile["id"])
    conn.commit()
    return {
        "wallet": wallet["label"],
        "source": source_label,
        "imported": imported,
        "skipped": skipped,
    }


def import_into_wallet(conn, workspace_ref, profile_ref, wallet_ref, file_path, input_format):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    records = load_import_records(file_path, input_format)
    outcome = insert_wallet_records(conn, profile, wallet, records, f"file:{input_format}")
    outcome["input_format"] = input_format
    outcome["file"] = os.path.abspath(file_path)
    return outcome


def fetch_esplora_transactions(base_url, address, max_pages=20):
    encoded = urlparse.quote(address, safe="")
    transactions = []
    last_seen = None
    for _ in range(max_pages):
        if last_seen:
            url = f"{base_url.rstrip('/')}/address/{encoded}/txs/chain/{last_seen}"
        else:
            url = f"{base_url.rstrip('/')}/address/{encoded}/txs/chain"
        page = http_get_json(url)
        if not page:
            break
        transactions.extend(page)
        last_seen = page[-1]["txid"]
        if len(page) < 25:
            break
    return transactions


def sats_to_btc(value):
    return dec(value) / SATS_PER_BTC


def record_from_esplora_tx(tx, tracked_addresses, backend_name):
    tracked = set(tracked_addresses)
    received_sats = sum(
        dec(vout.get("value", 0))
        for vout in tx.get("vout", [])
        if vout.get("scriptpubkey_address") in tracked
    )
    sent_sats = Decimal("0")
    for vin in tx.get("vin", []):
        prevout = vin.get("prevout") or {}
        if prevout.get("scriptpubkey_address") in tracked:
            sent_sats += dec(prevout.get("value", 0))
    if received_sats == 0 and sent_sats == 0:
        return None
    fee_sats = dec(tx.get("fee"), "0")
    if received_sats > sent_sats:
        direction = "inbound"
        amount = sats_to_btc(received_sats - sent_sats)
        fee = Decimal("0")
        kind = "deposit"
    else:
        direction = "outbound"
        gross_out_sats = sent_sats - received_sats
        amount_sats = gross_out_sats - fee_sats
        if amount_sats < 0:
            amount_sats = Decimal("0")
        amount = sats_to_btc(amount_sats)
        fee = sats_to_btc(fee_sats)
        kind = "withdrawal" if amount > 0 else "fee"
    block_time = ((tx.get("status") or {}).get("block_time"))
    occurred_at = (
        datetime.fromtimestamp(int(block_time), tz=timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")
        if block_time
        else now_iso()
    )
    return {
        "txid": tx.get("txid"),
        "occurred_at": occurred_at,
        "direction": direction,
        "asset": "BTC",
        "amount": amount,
        "fee": fee,
        "fiat_rate": None,
        "fiat_value": None,
        "kind": kind,
        "description": f"Synced from {backend_name}",
        "counterparty": None,
        "raw_json": json.dumps(tx, sort_keys=True),
    }


def sync_wallet_from_backend(conn, runtime_config, workspace_ref, profile_ref, wallet):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    config = json.loads(wallet["config_json"] or "{}")
    backend = resolve_backend(runtime_config, config.get("backend"))
    addresses = normalize_addresses(config.get("addresses"))
    if not addresses:
        return {
            "wallet": wallet["label"],
            "status": "skipped",
            "reason": "no addresses configured for backend sync",
        }
    if backend["kind"] != "esplora":
        return {
            "wallet": wallet["label"],
            "status": "skipped",
            "reason": f"backend kind '{backend['kind']}' is not implemented yet",
        }
    transactions_by_txid = {}
    for address in addresses:
        for tx in fetch_esplora_transactions(backend["url"], address):
            transactions_by_txid[tx["txid"]] = tx
    normalized_records = []
    for tx in sorted(
        transactions_by_txid.values(),
        key=lambda item: ((item.get("status") or {}).get("block_time") or 0, item.get("txid", "")),
    ):
        normalized = record_from_esplora_tx(tx, addresses, backend["name"])
        if normalized:
            normalized_records.append(normalized)
    outcome = insert_wallet_records(conn, profile, wallet, normalized_records, f"backend:{backend['name']}")
    outcome["backend"] = backend["name"]
    outcome["backend_kind"] = backend["kind"]
    outcome["backend_url"] = backend["url"]
    outcome["addresses"] = ",".join(addresses)
    return outcome


def sync_wallet(conn, runtime_config, workspace_ref, profile_ref, wallet_ref=None, sync_all=False):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    if sync_all:
        wallets = conn.execute("SELECT * FROM wallets WHERE profile_id = ? ORDER BY label ASC", (profile["id"],)).fetchall()
    else:
        if not wallet_ref:
            raise AppError("Provide --wallet or use --all")
        wallets = [resolve_wallet(conn, profile["id"], wallet_ref)]
    results = []
    for wallet in wallets:
        config = json.loads(wallet["config_json"] or "{}")
        source_file = config.get("source_file")
        source_format = config.get("source_format")
        addresses = normalize_addresses(config.get("addresses"))
        if source_file and source_format:
            outcome = import_into_wallet(
                conn,
                profile["workspace_id"],
                profile["id"],
                wallet["id"],
                source_file,
                source_format,
            )
            results.append({"wallet": wallet["label"], "status": "synced", **outcome})
            continue
        if addresses:
            outcome = sync_wallet_from_backend(conn, runtime_config, profile["workspace_id"], profile["id"], wallet)
            if outcome.get("status") == "skipped":
                results.append(outcome)
            else:
                results.append({"wallet": wallet["label"], "status": "synced", **outcome})
            continue
        results.append(
            {
                "wallet": wallet["label"],
                "status": "skipped",
                "reason": "no file source or backend addresses configured",
            }
        )
    return results


def list_transactions(conn, workspace_ref, profile_ref, wallet_ref=None, limit=100):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    params = [profile["id"]]
    wallet_clause = ""
    if wallet_ref:
        wallet = resolve_wallet(conn, profile["id"], wallet_ref)
        wallet_clause = "AND t.wallet_id = ?"
        params.append(wallet["id"])
    params.append(limit)
    rows = conn.execute(
        f"""
        SELECT
            t.id,
            COALESCE(t.external_id, '') AS external_id,
            t.occurred_at,
            w.label AS wallet,
            t.direction,
            t.asset,
            t.amount,
            t.fee,
            COALESCE(t.fiat_rate, 0) AS fiat_rate,
            COALESCE(t.fiat_value, 0) AS fiat_value,
            COALESCE(t.kind, '') AS kind,
            COALESCE(t.description, '') AS description,
            COALESCE(t.note, '') AS note,
            CASE WHEN t.excluded = 1 THEN 'yes' ELSE '' END AS excluded,
            COALESCE(GROUP_CONCAT(tags.code, ','), '') AS tags
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        LEFT JOIN transaction_tags tt ON tt.transaction_id = t.id
        LEFT JOIN tags ON tags.id = tt.tag_id
        WHERE t.profile_id = ? {wallet_clause}
        GROUP BY t.id
        ORDER BY t.occurred_at DESC, t.created_at DESC
        LIMIT ?
        """,
        params,
    ).fetchall()
    return [dict(row) for row in rows]


def set_transaction_note(conn, workspace_ref, profile_ref, tx_ref, note):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    tx = resolve_transaction(conn, profile["id"], tx_ref)
    conn.execute("UPDATE transactions SET note = ? WHERE id = ?", (note, tx["id"]))
    conn.commit()
    return {"transaction_id": tx["id"], "note": note}


def clear_transaction_note(conn, workspace_ref, profile_ref, tx_ref):
    return set_transaction_note(conn, workspace_ref, profile_ref, tx_ref, None)


def set_transaction_excluded(conn, workspace_ref, profile_ref, tx_ref, excluded):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    tx = resolve_transaction(conn, profile["id"], tx_ref)
    conn.execute("UPDATE transactions SET excluded = ? WHERE id = ?", (1 if excluded else 0, tx["id"]))
    invalidate_journals(conn, profile["id"])
    conn.commit()
    return {"transaction_id": tx["id"], "excluded": bool(excluded)}


def create_tag(conn, workspace_ref, profile_ref, code, label):
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    tag_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO tags(id, workspace_id, profile_id, code, label, created_at)
        VALUES(?, ?, ?, ?, ?, ?)
        """,
        (tag_id, workspace["id"], profile["id"], normalize_code(code), label, now_iso()),
    )
    conn.commit()
    return conn.execute("SELECT * FROM tags WHERE id = ?", (tag_id,)).fetchone()


def list_tags(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        "SELECT id, code, label, created_at FROM tags WHERE profile_id = ? ORDER BY code ASC",
        (profile["id"],),
    ).fetchall()
    return [dict(row) for row in rows]


def add_tag_to_transaction(conn, workspace_ref, profile_ref, tx_ref, tag_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    tx = resolve_transaction(conn, profile["id"], tx_ref)
    tag = resolve_tag(conn, profile["id"], tag_ref)
    conn.execute(
        "INSERT OR IGNORE INTO transaction_tags(transaction_id, tag_id) VALUES(?, ?)",
        (tx["id"], tag["id"]),
    )
    conn.commit()
    return {"transaction_id": tx["id"], "tag": tag["code"], "status": "added"}


def remove_tag_from_transaction(conn, workspace_ref, profile_ref, tx_ref, tag_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    tx = resolve_transaction(conn, profile["id"], tx_ref)
    tag = resolve_tag(conn, profile["id"], tag_ref)
    conn.execute(
        "DELETE FROM transaction_tags WHERE transaction_id = ? AND tag_id = ?",
        (tx["id"], tag["id"]),
    )
    conn.commit()
    return {"transaction_id": tx["id"], "tag": tag["code"], "status": "removed"}


def available_quantity(lots):
    total = Decimal("0")
    for lot in lots:
        total += lot["quantity"]
    return total


def consume_lots(lots, quantity, algorithm):
    remaining = dec(quantity)
    cost_basis = Decimal("0")
    while remaining > 0:
        if not lots:
            raise AppError("Not enough lots to consume")
        lot = lots[0] if algorithm == "FIFO" else lots[-1]
        take = min(remaining, lot["quantity"])
        cost_basis += take * lot["unit_cost"]
        lot["quantity"] -= take
        remaining -= take
        if lot["quantity"] <= Decimal("0"):
            if algorithm == "FIFO":
                lots.pop(0)
            else:
                lots.pop()
    return cost_basis


def latest_rates_for_profile(conn, profile_id):
    rows = conn.execute(
        """
        SELECT asset, fiat_rate, fiat_value, amount
        FROM transactions
        WHERE profile_id = ? AND excluded = 0
        ORDER BY occurred_at DESC, created_at DESC
        """,
        (profile_id,),
    ).fetchall()
    rates = {}
    for row in rows:
        asset = row["asset"]
        if asset in rates:
            continue
        if row["fiat_rate"] is not None:
            rates[asset] = dec(row["fiat_rate"])
        elif row["fiat_value"] is not None and row["amount"]:
            rates[asset] = dec(row["fiat_value"]) / dec(row["amount"])
    return rates


def build_ledger_state(conn, profile):
    rows = conn.execute(
        """
        SELECT
            t.*,
            w.label AS wallet_label,
            w.kind AS wallet_kind,
            w.account_id AS wallet_account_id,
            COALESCE(a.code, 'treasury') AS account_code,
            COALESCE(a.label, 'Treasury') AS account_label
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        LEFT JOIN accounts a ON a.id = w.account_id
        WHERE t.profile_id = ? AND t.excluded = 0
        ORDER BY t.occurred_at ASC, t.created_at ASC, t.id ASC
        """,
        (profile["id"],),
    ).fetchall()
    algorithm = str(profile["gains_algorithm"]).upper()
    lots_by_wallet_asset = defaultdict(list)
    entries = []
    quarantines = []
    rates = latest_rates_for_profile(conn, profile["id"])
    for row in rows:
        amount = dec(row["amount"])
        fee = dec(row["fee"])
        fiat_value = dec(row["fiat_value"]) if row["fiat_value"] is not None else Decimal("0")
        fiat_rate = dec(row["fiat_rate"]) if row["fiat_rate"] is not None else None
        if fiat_value == 0 and fiat_rate is not None:
            fiat_value = amount * fiat_rate
        asset = row["asset"]
        wallet_ref = {
            "wallet_id": row["wallet_id"],
            "wallet_label": row["wallet_label"],
            "account_id": row["wallet_account_id"],
            "account_code": row["account_code"],
            "account_label": row["account_label"],
        }
        wallet_asset_key = (row["wallet_id"], asset)
        if row["direction"] == "inbound":
            unit_cost = (fiat_value / amount) if amount > 0 else Decimal("0")
            lots_by_wallet_asset[wallet_asset_key].append(
                {
                    **wallet_ref,
                    "quantity": amount,
                    "unit_cost": unit_cost,
                    "asset": asset,
                }
            )
            entries.append(
                {
                    "id": str(uuid.uuid4()),
                    "workspace_id": profile["workspace_id"],
                    "profile_id": profile["id"],
                    "transaction_id": row["id"],
                    "wallet_id": row["wallet_id"],
                    "account_id": row["wallet_account_id"],
                    "occurred_at": row["occurred_at"],
                    "entry_type": "acquisition",
                    "asset": asset,
                    "quantity": amount,
                    "fiat_value": fiat_value,
                    "unit_cost": unit_cost,
                    "cost_basis": None,
                    "proceeds": None,
                    "gain_loss": None,
                    "description": row["description"] or row["kind"] or "Inbound transaction",
                }
            )
            continue
        needed = amount + fee
        if available_quantity(lots_by_wallet_asset[wallet_asset_key]) < needed:
            quarantines.append(
                {
                    "transaction_id": row["id"],
                    "workspace_id": profile["workspace_id"],
                    "profile_id": profile["id"],
                    "reason": "insufficient_lots",
                    "detail_json": json.dumps(
                        {
                            "wallet": row["wallet_label"],
                            "asset": asset,
                            "required": float(needed),
                            "available": float(available_quantity(lots_by_wallet_asset[wallet_asset_key])),
                        },
                        sort_keys=True,
                    ),
                }
            )
            continue
        cost_basis = consume_lots(lots_by_wallet_asset[wallet_asset_key], amount, algorithm)
        gain_loss = fiat_value - cost_basis
        entries.append(
            {
                "id": str(uuid.uuid4()),
                "workspace_id": profile["workspace_id"],
                "profile_id": profile["id"],
                "transaction_id": row["id"],
                "wallet_id": row["wallet_id"],
                "account_id": row["wallet_account_id"],
                "occurred_at": row["occurred_at"],
                "entry_type": "disposal",
                "asset": asset,
                "quantity": -amount,
                "fiat_value": fiat_value,
                "unit_cost": Decimal("0"),
                "cost_basis": cost_basis,
                "proceeds": fiat_value,
                "gain_loss": gain_loss,
                "description": row["description"] or row["kind"] or "Outbound transaction",
            }
        )
        if fee > 0:
            fee_basis = consume_lots(lots_by_wallet_asset[wallet_asset_key], fee, algorithm)
            entries.append(
                {
                    "id": str(uuid.uuid4()),
                    "workspace_id": profile["workspace_id"],
                    "profile_id": profile["id"],
                    "transaction_id": row["id"],
                    "wallet_id": row["wallet_id"],
                    "account_id": row["wallet_account_id"],
                    "occurred_at": row["occurred_at"],
                    "entry_type": "fee",
                    "asset": asset,
                    "quantity": -fee,
                    "fiat_value": Decimal("0"),
                    "unit_cost": Decimal("0"),
                    "cost_basis": fee_basis,
                    "proceeds": Decimal("0"),
                    "gain_loss": -fee_basis,
                    "description": f"Network fee for {row['description'] or row['external_id'] or row['id']}",
                }
            )
    account_holdings = defaultdict(lambda: {"quantity": Decimal("0"), "cost_basis": Decimal("0")})
    wallet_holdings = defaultdict(lambda: {"quantity": Decimal("0"), "cost_basis": Decimal("0")})
    for (wallet_id, asset), lots in lots_by_wallet_asset.items():
        for lot in lots:
            account_key = (
                lot["account_id"],
                lot["account_code"],
                lot["account_label"],
                asset,
            )
            wallet_key = (
                wallet_id,
                lot["wallet_label"],
                lot["account_code"],
                asset,
            )
            account_holdings[account_key]["quantity"] += lot["quantity"]
            account_holdings[account_key]["cost_basis"] += lot["quantity"] * lot["unit_cost"]
            wallet_holdings[wallet_key]["quantity"] += lot["quantity"]
            wallet_holdings[wallet_key]["cost_basis"] += lot["quantity"] * lot["unit_cost"]
    return {
        "entries": entries,
        "quarantines": quarantines,
        "account_holdings": account_holdings,
        "wallet_holdings": wallet_holdings,
        "latest_rates": rates,
    }


def process_journals(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    state = build_ledger_state(conn, profile)
    conn.execute("DELETE FROM journal_entries WHERE profile_id = ?", (profile["id"],))
    conn.execute("DELETE FROM journal_quarantines WHERE profile_id = ?", (profile["id"],))
    created_at = now_iso()
    for entry in state["entries"]:
        conn.execute(
            """
            INSERT INTO journal_entries(
                id, workspace_id, profile_id, transaction_id, wallet_id, account_id,
                occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
                cost_basis, proceeds, gain_loss, description, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                entry["id"],
                entry["workspace_id"],
                entry["profile_id"],
                entry["transaction_id"],
                entry["wallet_id"],
                entry["account_id"],
                entry["occurred_at"],
                entry["entry_type"],
                entry["asset"],
                float(entry["quantity"]),
                float(entry["fiat_value"]),
                float(entry["unit_cost"]),
                float(entry["cost_basis"]) if entry["cost_basis"] is not None else None,
                float(entry["proceeds"]) if entry["proceeds"] is not None else None,
                float(entry["gain_loss"]) if entry["gain_loss"] is not None else None,
                entry["description"],
                created_at,
            ),
        )
    for quarantine in state["quarantines"]:
        conn.execute(
            """
            INSERT INTO journal_quarantines(
                transaction_id, workspace_id, profile_id, reason, detail_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?)
            """,
            (
                quarantine["transaction_id"],
                quarantine["workspace_id"],
                quarantine["profile_id"],
                quarantine["reason"],
                quarantine["detail_json"],
                created_at,
            ),
        )
    tx_count = conn.execute(
        "SELECT COUNT(*) AS count FROM transactions WHERE profile_id = ? AND excluded = 0",
        (profile["id"],),
    ).fetchone()["count"]
    conn.execute(
        "UPDATE profiles SET last_processed_at = ?, last_processed_tx_count = ? WHERE id = ?",
        (created_at, tx_count, profile["id"]),
    )
    conn.commit()
    return {
        "profile": profile["label"],
        "entries_created": len(state["entries"]),
        "quarantined": len(state["quarantines"]),
        "processed_transactions": tx_count,
        "processed_at": created_at,
    }


def list_journal_entries(conn, workspace_ref, profile_ref, limit=200):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        """
        SELECT
            je.id,
            je.occurred_at,
            w.label AS wallet,
            COALESCE(a.code, '') AS account,
            je.entry_type,
            je.asset,
            je.quantity,
            je.fiat_value,
            COALESCE(je.cost_basis, 0) AS cost_basis,
            COALESCE(je.proceeds, 0) AS proceeds,
            COALESCE(je.gain_loss, 0) AS gain_loss,
            COALESCE(je.description, '') AS description
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        LEFT JOIN accounts a ON a.id = je.account_id
        WHERE je.profile_id = ?
        ORDER BY je.occurred_at DESC, je.created_at DESC
        LIMIT ?
        """,
        (profile["id"], limit),
    ).fetchall()
    return [dict(row) for row in rows]


def list_quarantines(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        """
        SELECT
            q.transaction_id,
            t.external_id,
            t.occurred_at,
            w.label AS wallet,
            t.asset,
            t.amount,
            t.fee,
            q.reason,
            q.detail_json
        FROM journal_quarantines q
        JOIN transactions t ON t.id = q.transaction_id
        JOIN wallets w ON w.id = t.wallet_id
        WHERE q.profile_id = ?
        ORDER BY t.occurred_at DESC
        """,
        (profile["id"],),
    ).fetchall()
    output = []
    for row in rows:
        detail = json.loads(row["detail_json"] or "{}")
        output.append(
            {
                "transaction_id": row["transaction_id"],
                "external_id": row["external_id"] or "",
                "occurred_at": row["occurred_at"],
                "wallet": row["wallet"],
                "asset": row["asset"],
                "amount": row["amount"],
                "fee": row["fee"],
                "reason": row["reason"],
                "detail": json.dumps(detail, sort_keys=True),
            }
        )
    return output


def require_processed_journals(conn, profile):
    current_count = conn.execute(
        "SELECT COUNT(*) AS count FROM transactions WHERE profile_id = ? AND excluded = 0",
        (profile["id"],),
    ).fetchone()["count"]
    if not profile["last_processed_at"] or current_count != profile["last_processed_tx_count"]:
        raise AppError("Reports require fresh journals. Run `kassiber journals process` first.")


def report_balance_sheet(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    require_processed_journals(conn, profile)
    state = build_ledger_state(conn, profile)
    rows = []
    for (account_id, account_code, account_label, asset), value in sorted(
        state["account_holdings"].items(),
        key=lambda item: (item[0][1], item[0][3]),
    ):
        quantity = value["quantity"]
        if quantity <= 0:
            continue
        cost_basis = value["cost_basis"]
        latest_rate = state["latest_rates"].get(asset, Decimal("0"))
        market_value = quantity * latest_rate
        rows.append(
            {
                "account": account_code or account_label,
                "asset": asset,
                "quantity": float(quantity),
                "cost_basis": float(cost_basis),
                "market_value": float(market_value),
                "unrealized_pnl": float(market_value - cost_basis),
            }
        )
    return rows


def report_portfolio_summary(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    require_processed_journals(conn, profile)
    state = build_ledger_state(conn, profile)
    rows = []
    for (wallet_id, wallet_label, account_code, asset), value in sorted(
        state["wallet_holdings"].items(),
        key=lambda item: (item[0][1], item[0][3]),
    ):
        quantity = value["quantity"]
        if quantity <= 0:
            continue
        cost_basis = value["cost_basis"]
        latest_rate = state["latest_rates"].get(asset, Decimal("0"))
        market_value = quantity * latest_rate
        avg_cost = cost_basis / quantity if quantity else Decimal("0")
        rows.append(
            {
                "wallet": wallet_label,
                "account": account_code,
                "asset": asset,
                "quantity": float(quantity),
                "avg_cost": float(avg_cost),
                "cost_basis": float(cost_basis),
                "market_value": float(market_value),
                "unrealized_pnl": float(market_value - cost_basis),
            }
        )
    return rows


def report_capital_gains(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    require_processed_journals(conn, profile)
    rows = conn.execute(
        """
        SELECT
            je.occurred_at,
            w.label AS wallet,
            je.transaction_id,
            je.entry_type,
            je.asset,
            ABS(je.quantity) AS quantity,
            COALESCE(je.proceeds, 0) AS proceeds,
            COALESCE(je.cost_basis, 0) AS cost_basis,
            COALESCE(je.gain_loss, 0) AS gain_loss,
            COALESCE(je.description, '') AS description
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        WHERE je.profile_id = ? AND je.entry_type IN ('disposal', 'fee')
        ORDER BY je.occurred_at ASC
        """,
        (profile["id"],),
    ).fetchall()
    return [dict(row) for row in rows]


def report_journal_entries(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    require_processed_journals(conn, profile)
    return list_journal_entries(conn, profile["workspace_id"], profile["id"], limit=1000)


def show_status(conn, data_root):
    workspace_id = get_setting(conn, "context_workspace")
    profile_id = get_setting(conn, "context_profile")
    workspace = conn.execute("SELECT label FROM workspaces WHERE id = ?", (workspace_id,)).fetchone() if workspace_id else None
    profile = conn.execute("SELECT label FROM profiles WHERE id = ?", (profile_id,)).fetchone() if profile_id else None
    counts = {
        "workspaces": conn.execute("SELECT COUNT(*) AS count FROM workspaces").fetchone()["count"],
        "profiles": conn.execute("SELECT COUNT(*) AS count FROM profiles").fetchone()["count"],
        "accounts": conn.execute("SELECT COUNT(*) AS count FROM accounts").fetchone()["count"],
        "wallets": conn.execute("SELECT COUNT(*) AS count FROM wallets").fetchone()["count"],
        "transactions": conn.execute("SELECT COUNT(*) AS count FROM transactions").fetchone()["count"],
        "journal_entries": conn.execute("SELECT COUNT(*) AS count FROM journal_entries").fetchone()["count"],
        "quarantines": conn.execute("SELECT COUNT(*) AS count FROM journal_quarantines").fetchone()["count"],
    }
    return {
        "version": __version__,
        "data_root": str(resolve_effective_data_root(data_root)),
        "current_workspace": workspace["label"] if workspace else "",
        "current_profile": profile["label"] if profile else "",
        **counts,
    }


def list_backends(runtime_config):
    rows = []
    for name, backend in sorted(runtime_config["backends"].items()):
        rows.append(
            {
                "name": name,
                "kind": backend["kind"],
                "url": backend["url"],
                "default": "yes" if name == runtime_config["default_backend"] else "",
                "source": backend["source"],
            }
        )
    return rows


def cmd_init(conn, args):
    init_app(conn)
    emit(
        args,
        {
            "version": __version__,
            "data_root": str(resolve_effective_data_root(args.data_root)),
            "database": str(resolve_database_path(resolve_effective_data_root(args.data_root))),
        },
    )


def cmd_status(conn, args):
    payload = show_status(conn, args.data_root)
    payload["default_backend"] = args.runtime_config["default_backend"]
    payload["env_file"] = args.runtime_config["env_file"]
    emit(args, payload)


def cmd_context_show(conn, args):
    workspace_id = get_setting(conn, "context_workspace")
    profile_id = get_setting(conn, "context_profile")
    workspace = conn.execute("SELECT id, label FROM workspaces WHERE id = ?", (workspace_id,)).fetchone() if workspace_id else None
    profile = conn.execute("SELECT id, label FROM profiles WHERE id = ?", (profile_id,)).fetchone() if profile_id else None
    emit(
        args,
        {
            "workspace_id": workspace["id"] if workspace else "",
            "workspace_label": workspace["label"] if workspace else "",
            "profile_id": profile["id"] if profile else "",
            "profile_label": profile["label"] if profile else "",
        },
    )


def cmd_context_set(conn, args):
    if args.workspace:
        workspace = resolve_workspace(conn, args.workspace)
        set_setting(conn, "context_workspace", workspace["id"])
        if args.profile:
            profile = resolve_profile(conn, workspace["id"], args.profile)
            set_setting(conn, "context_profile", profile["id"])
        conn.commit()
    elif args.profile:
        workspace = resolve_workspace(conn)
        profile = resolve_profile(conn, workspace["id"], args.profile)
        set_setting(conn, "context_profile", profile["id"])
        conn.commit()
    else:
        raise AppError("Provide --workspace and/or --profile")
    cmd_context_show(conn, args)


def build_parser():
    parser = argparse.ArgumentParser(
        prog=APP_NAME,
        description="Open-source, local-first Bitcoin accounting CLI with multi-account and multi-wallet support.",
    )
    parser.add_argument("--data-root", default=DEFAULT_DATA_ROOT, help="Data directory for the local SQLite store")
    parser.add_argument("--env-file", default=DEFAULT_ENV_FILE, help="Path to a .env file that defines named sync backends")
    parser.add_argument("--format", choices=["table", "json"], default="table", help="Output format")
    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("init")
    sub.add_parser("status")

    backends = sub.add_parser("backends")
    backends_sub = backends.add_subparsers(dest="backends_command", required=True)
    backends_sub.add_parser("list")

    context = sub.add_parser("context")
    context_sub = context.add_subparsers(dest="context_command", required=True)
    context_sub.add_parser("show")
    context_set = context_sub.add_parser("set")
    context_set.add_argument("--workspace")
    context_set.add_argument("--profile")

    workspaces = sub.add_parser("workspaces")
    ws_sub = workspaces.add_subparsers(dest="workspaces_command", required=True)
    ws_sub.add_parser("list")
    ws_create = ws_sub.add_parser("create")
    ws_create.add_argument("label")

    profiles = sub.add_parser("profiles")
    profiles_sub = profiles.add_subparsers(dest="profiles_command", required=True)
    profiles_list = profiles_sub.add_parser("list")
    profiles_list.add_argument("--workspace")
    profiles_create = profiles_sub.add_parser("create")
    profiles_create.add_argument("label")
    profiles_create.add_argument("--workspace")
    profiles_create.add_argument("--fiat-currency", default="USD")
    profiles_create.add_argument("--gains-algorithm", choices=["FIFO", "LIFO"], default="FIFO")

    accounts = sub.add_parser("accounts")
    accounts_sub = accounts.add_subparsers(dest="accounts_command", required=True)
    accounts_list = accounts_sub.add_parser("list")
    accounts_list.add_argument("--workspace")
    accounts_list.add_argument("--profile")
    accounts_create = accounts_sub.add_parser("create")
    accounts_create.add_argument("--workspace")
    accounts_create.add_argument("--profile")
    accounts_create.add_argument("--code", required=True)
    accounts_create.add_argument("--label", required=True)
    accounts_create.add_argument("--type", required=True)
    accounts_create.add_argument("--asset")

    wallets = sub.add_parser("wallets")
    wallets_sub = wallets.add_subparsers(dest="wallets_command", required=True)
    wallets_list = wallets_sub.add_parser("list")
    wallets_list.add_argument("--workspace")
    wallets_list.add_argument("--profile")
    wallets_create = wallets_sub.add_parser("create")
    wallets_create.add_argument("--workspace")
    wallets_create.add_argument("--profile")
    wallets_create.add_argument("--label", required=True)
    wallets_create.add_argument("--kind", required=True)
    wallets_create.add_argument("--account")
    wallets_create.add_argument("--backend")
    wallets_create.add_argument("--address", action="append")
    wallets_create.add_argument("--config")
    wallets_create.add_argument("--config-file")
    wallets_create.add_argument("--source-file")
    wallets_create.add_argument("--source-format", choices=["json", "csv"])
    wallets_import_json = wallets_sub.add_parser("import-json")
    wallets_import_json.add_argument("--workspace")
    wallets_import_json.add_argument("--profile")
    wallets_import_json.add_argument("--wallet", required=True)
    wallets_import_json.add_argument("--file", required=True)
    wallets_import_csv = wallets_sub.add_parser("import-csv")
    wallets_import_csv.add_argument("--workspace")
    wallets_import_csv.add_argument("--profile")
    wallets_import_csv.add_argument("--wallet", required=True)
    wallets_import_csv.add_argument("--file", required=True)
    wallets_sync = wallets_sub.add_parser("sync")
    wallets_sync.add_argument("--workspace")
    wallets_sync.add_argument("--profile")
    wallets_sync.add_argument("--wallet")
    wallets_sync.add_argument("--all", action="store_true")

    transactions = sub.add_parser("transactions")
    tx_sub = transactions.add_subparsers(dest="transactions_command", required=True)
    tx_list = tx_sub.add_parser("list")
    tx_list.add_argument("--workspace")
    tx_list.add_argument("--profile")
    tx_list.add_argument("--wallet")
    tx_list.add_argument("--limit", type=int, default=100)

    metadata = sub.add_parser("metadata")
    meta_sub = metadata.add_subparsers(dest="metadata_command", required=True)
    notes = meta_sub.add_parser("notes")
    notes_sub = notes.add_subparsers(dest="notes_command", required=True)
    notes_set = notes_sub.add_parser("set")
    notes_set.add_argument("--workspace")
    notes_set.add_argument("--profile")
    notes_set.add_argument("--transaction", required=True)
    notes_set.add_argument("--note", required=True)
    notes_clear = notes_sub.add_parser("clear")
    notes_clear.add_argument("--workspace")
    notes_clear.add_argument("--profile")
    notes_clear.add_argument("--transaction", required=True)
    tags = meta_sub.add_parser("tags")
    tags_sub = tags.add_subparsers(dest="tags_command", required=True)
    tags_list = tags_sub.add_parser("list")
    tags_list.add_argument("--workspace")
    tags_list.add_argument("--profile")
    tags_create = tags_sub.add_parser("create")
    tags_create.add_argument("--workspace")
    tags_create.add_argument("--profile")
    tags_create.add_argument("--code", required=True)
    tags_create.add_argument("--label", required=True)
    tags_add = tags_sub.add_parser("add")
    tags_add.add_argument("--workspace")
    tags_add.add_argument("--profile")
    tags_add.add_argument("--transaction", required=True)
    tags_add.add_argument("--tag", required=True)
    tags_remove = tags_sub.add_parser("remove")
    tags_remove.add_argument("--workspace")
    tags_remove.add_argument("--profile")
    tags_remove.add_argument("--transaction", required=True)
    tags_remove.add_argument("--tag", required=True)
    exclude = meta_sub.add_parser("exclude")
    exclude.add_argument("--workspace")
    exclude.add_argument("--profile")
    exclude.add_argument("--transaction", required=True)
    include = meta_sub.add_parser("include")
    include.add_argument("--workspace")
    include.add_argument("--profile")
    include.add_argument("--transaction", required=True)

    journals = sub.add_parser("journals")
    journals_sub = journals.add_subparsers(dest="journals_command", required=True)
    journals_process = journals_sub.add_parser("process")
    journals_process.add_argument("--workspace")
    journals_process.add_argument("--profile")
    journals_list = journals_sub.add_parser("list")
    journals_list.add_argument("--workspace")
    journals_list.add_argument("--profile")
    journals_list.add_argument("--limit", type=int, default=200)
    journals_quarantined = journals_sub.add_parser("quarantined")
    journals_quarantined.add_argument("--workspace")
    journals_quarantined.add_argument("--profile")

    reports = sub.add_parser("reports")
    reports_sub = reports.add_subparsers(dest="reports_command", required=True)
    for report_name in ["balance-sheet", "portfolio-summary", "capital-gains", "journal-entries"]:
        report = reports_sub.add_parser(report_name)
        report.add_argument("--workspace")
        report.add_argument("--profile")

    return parser


def dispatch(conn, args):
    if args.command == "init":
        return cmd_init(conn, args)
    if args.command == "status":
        return cmd_status(conn, args)
    if args.command == "backends":
        if args.backends_command == "list":
            return emit(args, list_backends(args.runtime_config))
    if args.command == "context":
        if args.context_command == "show":
            return cmd_context_show(conn, args)
        if args.context_command == "set":
            return cmd_context_set(conn, args)
    if args.command == "workspaces":
        if args.workspaces_command == "list":
            return emit(args, list_workspaces(conn))
        if args.workspaces_command == "create":
            return emit(args, dict(create_workspace(conn, args.label)))
    if args.command == "profiles":
        if args.profiles_command == "list":
            return emit(args, list_profiles(conn, args.workspace))
        if args.profiles_command == "create":
            return emit(
                args,
                dict(
                    create_profile(
                        conn,
                        args.workspace,
                        args.label,
                        args.fiat_currency,
                        args.gains_algorithm,
                    )
                ),
            )
    if args.command == "accounts":
        if args.accounts_command == "list":
            return emit(args, list_accounts(conn, args.workspace, args.profile))
        if args.accounts_command == "create":
            return emit(
                args,
                dict(
                    create_account(
                        conn,
                        args.workspace,
                        args.profile,
                        args.code,
                        args.label,
                        args.type,
                        args.asset,
                    )
                ),
            )
    if args.command == "wallets":
        if args.wallets_command == "list":
            return emit(args, list_wallets(conn, args.workspace, args.profile))
        if args.wallets_command == "create":
            return emit(
                args,
                dict(
                    create_wallet(
                        conn,
                        args.workspace,
                        args.profile,
                        args.label,
                        args.kind,
                        args.account,
                        parse_wallet_config(args),
                    )
                ),
            )
        if args.wallets_command == "import-json":
            return emit(args, import_into_wallet(conn, args.workspace, args.profile, args.wallet, args.file, "json"))
        if args.wallets_command == "import-csv":
            return emit(args, import_into_wallet(conn, args.workspace, args.profile, args.wallet, args.file, "csv"))
        if args.wallets_command == "sync":
            return emit(args, sync_wallet(conn, args.runtime_config, args.workspace, args.profile, args.wallet, args.all))
    if args.command == "transactions":
        if args.transactions_command == "list":
            return emit(args, list_transactions(conn, args.workspace, args.profile, args.wallet, args.limit))
    if args.command == "metadata":
        if args.metadata_command == "notes":
            if args.notes_command == "set":
                return emit(args, set_transaction_note(conn, args.workspace, args.profile, args.transaction, args.note))
            if args.notes_command == "clear":
                return emit(args, clear_transaction_note(conn, args.workspace, args.profile, args.transaction))
        if args.metadata_command == "tags":
            if args.tags_command == "list":
                return emit(args, list_tags(conn, args.workspace, args.profile))
            if args.tags_command == "create":
                return emit(args, dict(create_tag(conn, args.workspace, args.profile, args.code, args.label)))
            if args.tags_command == "add":
                return emit(args, add_tag_to_transaction(conn, args.workspace, args.profile, args.transaction, args.tag))
            if args.tags_command == "remove":
                return emit(args, remove_tag_from_transaction(conn, args.workspace, args.profile, args.transaction, args.tag))
        if args.metadata_command == "exclude":
            return emit(args, set_transaction_excluded(conn, args.workspace, args.profile, args.transaction, True))
        if args.metadata_command == "include":
            return emit(args, set_transaction_excluded(conn, args.workspace, args.profile, args.transaction, False))
    if args.command == "journals":
        if args.journals_command == "process":
            return emit(args, process_journals(conn, args.workspace, args.profile))
        if args.journals_command == "list":
            return emit(args, list_journal_entries(conn, args.workspace, args.profile, args.limit))
        if args.journals_command == "quarantined":
            return emit(args, list_quarantines(conn, args.workspace, args.profile))
    if args.command == "reports":
        if args.reports_command == "balance-sheet":
            return emit(args, report_balance_sheet(conn, args.workspace, args.profile))
        if args.reports_command == "portfolio-summary":
            return emit(args, report_portfolio_summary(conn, args.workspace, args.profile))
        if args.reports_command == "capital-gains":
            return emit(args, report_capital_gains(conn, args.workspace, args.profile))
        if args.reports_command == "journal-entries":
            return emit(args, report_journal_entries(conn, args.workspace, args.profile))
    raise AppError("Unknown command")


def command_needs_db(args):
    return args.command not in {"backends"}


def main(argv=None):
    parser = build_parser()
    args = parser.parse_args(argv)
    args.runtime_config = load_runtime_config(args.env_file)
    conn = open_db(args.data_root) if command_needs_db(args) else None
    try:
        dispatch(conn, args)
        return 0
    except AppError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    finally:
        if conn is not None:
            conn.close()
