"""SQLite storage layer for kassiber.

Owns the on-disk layout (default data root, DB filename, legacy fallbacks)
and the canonical schema. The single public entry point is `open_db`; every
other module should call it rather than opening a sqlite3 connection
directly. `open_db`:

1. Resolves `--data-root` against the current/legacy directory pair so a
   pre-rename `satbooks` store keeps working without manual migration.
2. Applies the embedded `SCHEMA` idempotently (`CREATE TABLE IF NOT
   EXISTS ...`).
3. Runs `ensure_schema_compat` — cheap `ALTER TABLE` guards for later
   column additions and the REAL→INTEGER msat migration in
   `_migrate_msat_columns`.
4. Turns on `PRAGMA foreign_keys` and hands back a `sqlite3.Connection`
   with `row_factory = sqlite3.Row` so call sites can index columns by
   name.

Call sites should never embed their own `CREATE TABLE` or `ALTER TABLE`
DDL — add it to `SCHEMA` or `ensure_schema_compat` here instead.
"""

import os
import sqlite3
from pathlib import Path

from .tax_policy import DEFAULT_LONG_TERM_DAYS, DEFAULT_TAX_COUNTRY


APP_NAME = "kassiber"
LEGACY_APP_NAME = "satbooks"
DEFAULT_DATA_ROOT = os.path.expanduser(f"~/.local/share/{APP_NAME}")
LEGACY_DATA_ROOT = os.path.expanduser(f"~/.local/share/{LEGACY_APP_NAME}")
DEFAULT_DB_FILENAME = f"{APP_NAME}.sqlite3"
LEGACY_DB_FILENAME = f"{LEGACY_APP_NAME}.sqlite3"


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
    tax_country TEXT NOT NULL DEFAULT 'generic',
    tax_long_term_days INTEGER NOT NULL DEFAULT 365,
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
    amount INTEGER NOT NULL,
    fee INTEGER NOT NULL DEFAULT 0,
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
    quantity INTEGER NOT NULL,
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

CREATE TABLE IF NOT EXISTS bip329_labels (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    wallet_id TEXT REFERENCES wallets(id) ON DELETE SET NULL,
    record_type TEXT NOT NULL,
    ref TEXT NOT NULL,
    label TEXT,
    origin TEXT,
    spendable INTEGER,
    data_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS backends (
    name TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    chain TEXT,
    network TEXT,
    url TEXT NOT NULL,
    auth_header TEXT,
    token TEXT,
    timeout INTEGER,
    tor_proxy TEXT,
    notes TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS rates_cache (
    pair TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    rate REAL NOT NULL,
    source TEXT NOT NULL,
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (pair, timestamp, source)
);

CREATE INDEX IF NOT EXISTS idx_rates_cache_pair_ts
    ON rates_cache(pair, timestamp DESC);
"""


def ensure_data_root(data_root):
    """Create `data_root` (and any missing parents) and return it as `Path`."""
    path = Path(data_root).expanduser()
    path.mkdir(parents=True, exist_ok=True)
    return path


def resolve_effective_data_root(data_root):
    """Redirect the default data root to the legacy `satbooks` dir when present.

    Preserves user data across the kassiber rename: only kicks in when the
    user passed the default `~/.local/share/kassiber` path and that dir
    does not exist but `~/.local/share/satbooks` does.
    """
    requested = Path(data_root).expanduser()
    if requested == Path(DEFAULT_DATA_ROOT).expanduser():
        legacy = Path(LEGACY_DATA_ROOT).expanduser()
        if not requested.exists() and legacy.exists():
            return legacy
    return requested


def resolve_database_path(data_root):
    """Pick `kassiber.sqlite3`, falling back to legacy `satbooks.sqlite3`."""
    root = Path(data_root).expanduser()
    current = root / DEFAULT_DB_FILENAME
    legacy = root / LEGACY_DB_FILENAME
    if current.exists() or not legacy.exists():
        return current
    return legacy


def open_db(data_root):
    """Open (and lazily migrate) the SQLite store rooted at `data_root`.

    Returns a `sqlite3.Connection` with `row_factory = Row` and foreign
    keys enabled. Safe to call repeatedly — schema creation uses
    `IF NOT EXISTS` and migrations are conditional on the column type.
    """
    root = ensure_data_root(resolve_effective_data_root(data_root))
    conn = sqlite3.connect(resolve_database_path(root))
    conn.row_factory = sqlite3.Row
    conn.executescript(SCHEMA)
    ensure_schema_compat(conn)
    conn.execute("PRAGMA foreign_keys = ON")
    return conn


def set_setting(conn, key, value):
    """Upsert a single row into the `settings` key/value table."""
    conn.execute(
        """
        INSERT INTO settings(key, value)
        VALUES(?, ?)
        ON CONFLICT(key) DO UPDATE SET value = excluded.value
        """,
        (key, value),
    )


def get_setting(conn, key):
    """Return the value for `key` in the `settings` table, or `None` if absent."""
    row = conn.execute("SELECT value FROM settings WHERE key = ?", (key,)).fetchone()
    return row["value"] if row else None


def ensure_column(conn, table_name, column_name, definition):
    """Idempotent `ALTER TABLE ... ADD COLUMN` — no-op when the column exists."""
    columns = {row["name"] for row in conn.execute(f"PRAGMA table_info({table_name})").fetchall()}
    if column_name in columns:
        return
    conn.execute(f"ALTER TABLE {table_name} ADD COLUMN {column_name} {definition}")
    conn.commit()


def ensure_schema_compat(conn):
    """Apply one-shot backfills not covered by `CREATE TABLE IF NOT EXISTS`.

    Anything added after the initial schema shipped belongs here so
    existing databases pick it up on the next `open_db`.
    """
    ensure_column(conn, "profiles", "tax_country", f"TEXT NOT NULL DEFAULT '{DEFAULT_TAX_COUNTRY}'")
    ensure_column(conn, "profiles", "tax_long_term_days", f"INTEGER NOT NULL DEFAULT {DEFAULT_LONG_TERM_DAYS}")
    _migrate_msat_columns(conn)


def _column_is_real(conn, table_name, column_name):
    for row in conn.execute(f"PRAGMA table_info({table_name})").fetchall():
        if row["name"] == column_name:
            return (row["type"] or "").upper() == "REAL"
    return False


def _migrate_msat_columns(conn):
    """Rebuild transactions / journal_entries tables to store amounts as INTEGER msat.

    Safe on fresh databases (columns are already INTEGER -> no-op) and on
    pre-migration databases created with REAL amount/fee/quantity columns.
    Existing float BTC values are multiplied into msat with ROUND_HALF_UP.
    """
    if _column_is_real(conn, "transactions", "amount") or _column_is_real(conn, "transactions", "fee"):
        conn.executescript(
            """
            CREATE TABLE transactions__msat_new (
                id TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
                wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
                external_id TEXT,
                fingerprint TEXT NOT NULL UNIQUE,
                occurred_at TEXT NOT NULL,
                direction TEXT NOT NULL,
                asset TEXT NOT NULL,
                amount INTEGER NOT NULL,
                fee INTEGER NOT NULL DEFAULT 0,
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
            INSERT INTO transactions__msat_new SELECT
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset,
                CAST(ROUND(amount * 100000000000.0) AS INTEGER),
                CAST(ROUND(fee * 100000000000.0) AS INTEGER),
                fiat_currency, fiat_rate, fiat_value,
                kind, description, counterparty, note, excluded, raw_json, created_at
            FROM transactions;
            DROP TABLE transactions;
            ALTER TABLE transactions__msat_new RENAME TO transactions;
            """
        )
        conn.commit()
    if _column_is_real(conn, "journal_entries", "quantity"):
        conn.executescript(
            """
            CREATE TABLE journal_entries__msat_new (
                id TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
                transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
                wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
                account_id TEXT REFERENCES accounts(id) ON DELETE SET NULL,
                occurred_at TEXT NOT NULL,
                entry_type TEXT NOT NULL,
                asset TEXT NOT NULL,
                quantity INTEGER NOT NULL,
                fiat_value REAL NOT NULL DEFAULT 0,
                unit_cost REAL NOT NULL DEFAULT 0,
                cost_basis REAL,
                proceeds REAL,
                gain_loss REAL,
                description TEXT,
                created_at TEXT NOT NULL
            );
            INSERT INTO journal_entries__msat_new SELECT
                id, workspace_id, profile_id, transaction_id, wallet_id, account_id,
                occurred_at, entry_type, asset,
                CAST(ROUND(quantity * 100000000000.0) AS INTEGER),
                fiat_value, unit_cost, cost_basis, proceeds, gain_loss, description, created_at
            FROM journal_entries;
            DROP TABLE journal_entries;
            ALTER TABLE journal_entries__msat_new RENAME TO journal_entries;
            """
        )
        conn.commit()
