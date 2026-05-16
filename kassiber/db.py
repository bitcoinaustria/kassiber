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

import json
import os
import sqlite3
from pathlib import Path

from .errors import AppError
from .fingerprints import make_transaction_fingerprint
from .msat import btc_to_msat, dec, msat_to_btc
from .secrets import sqlcipher as secrets_sqlcipher
from .tax_policy import DEFAULT_LONG_TERM_DAYS, DEFAULT_TAX_COUNTRY
from .wallet_descriptors import LIQUID_POLICY_ASSET_IDS


APP_NAME = "kassiber"
LEGACY_APP_NAME = "satbooks"
DEFAULT_STATE_ROOT = os.path.expanduser(f"~/.{APP_NAME}")
DEFAULT_DATA_DIRNAME = "data"
DEFAULT_CONFIG_DIRNAME = "config"
DEFAULT_EXPORTS_DIRNAME = "exports"
DEFAULT_ATTACHMENTS_DIRNAME = "attachments"
DEFAULT_SETTINGS_FILENAME = "settings.json"
DEFAULT_DATA_ROOT = os.path.join(DEFAULT_STATE_ROOT, DEFAULT_DATA_DIRNAME)
LEGACY_XDG_DATA_ROOT = os.path.expanduser(f"~/.local/share/{APP_NAME}")
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
    journal_input_version INTEGER NOT NULL DEFAULT 0,
    last_processed_input_version INTEGER NOT NULL DEFAULT 0,
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
    confirmed_at TEXT,
    direction TEXT NOT NULL,
    asset TEXT NOT NULL,
    amount INTEGER NOT NULL,
    fee INTEGER NOT NULL DEFAULT 0,
    fiat_currency TEXT,
    fiat_rate REAL,
    fiat_value REAL,
    fiat_price_source TEXT,
    fiat_rate_exact TEXT,
    fiat_value_exact TEXT,
    pricing_source_kind TEXT,
    pricing_provider TEXT,
    pricing_pair TEXT,
    pricing_timestamp TEXT,
    pricing_fetched_at TEXT,
    pricing_granularity TEXT,
    pricing_method TEXT,
    pricing_external_ref TEXT,
    pricing_quality TEXT,
    commercial_applied_link_id TEXT,
    kind TEXT,
    description TEXT,
    counterparty TEXT,
    note TEXT,
    excluded INTEGER NOT NULL DEFAULT 0,
    raw_json TEXT NOT NULL DEFAULT '{}',
    payment_hash TEXT,
    payment_hash_source TEXT,
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
    fiat_value_exact TEXT,
    unit_cost_exact TEXT,
    cost_basis_exact TEXT,
    proceeds_exact TEXT,
    gain_loss_exact TEXT,
    pricing_source_kind TEXT,
    pricing_quality TEXT,
    description TEXT,
    at_category TEXT,
    at_kennzahl INTEGER,
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

CREATE TABLE IF NOT EXISTS transaction_pairs (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    out_transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    in_transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    kind TEXT NOT NULL DEFAULT 'manual',
    policy TEXT NOT NULL DEFAULT 'carrying-value',
    notes TEXT,
    swap_fee_msat INTEGER,
    swap_fee_kind TEXT,
    confidence_at_pair TEXT,
    pair_source TEXT,
    deleted_at TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS transaction_pair_dismissals (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    out_transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    in_transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    reason TEXT,
    created_at TEXT NOT NULL,
    expires_at TEXT,
    UNIQUE (profile_id, out_transaction_id, in_transaction_id)
);

CREATE INDEX IF NOT EXISTS idx_transaction_pair_dismissals_profile
    ON transaction_pair_dismissals(profile_id, expires_at);

CREATE TABLE IF NOT EXISTS swap_matching_rules (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    name TEXT,
    predicate_json TEXT NOT NULL DEFAULT '{}',
    kind TEXT NOT NULL DEFAULT 'manual',
    policy TEXT NOT NULL DEFAULT 'carrying-value',
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_swap_matching_rules_profile_enabled
    ON swap_matching_rules(profile_id, enabled);

CREATE TABLE IF NOT EXISTS saved_views (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    surface TEXT NOT NULL,
    name TEXT NOT NULL,
    filter_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (profile_id, surface, name)
);

CREATE INDEX IF NOT EXISTS idx_saved_views_profile_surface
    ON saved_views(profile_id, surface);

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
    batch_size INTEGER,
    timeout INTEGER,
    tor_proxy TEXT,
    config_json TEXT NOT NULL DEFAULT '{}',
    notes TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS rates_cache (
    pair TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    rate REAL NOT NULL,
    rate_exact TEXT,
    source TEXT NOT NULL,
    fetched_at TEXT NOT NULL,
    granularity TEXT,
    method TEXT,
    open_rate REAL,
    open_rate_exact TEXT,
    high_rate REAL,
    high_rate_exact TEXT,
    low_rate REAL,
    low_rate_exact TEXT,
    close_rate REAL,
    close_rate_exact TEXT,
    volume REAL,
    volume_exact TEXT,
    trades INTEGER,
    PRIMARY KEY (pair, timestamp, source)
);

CREATE INDEX IF NOT EXISTS idx_rates_cache_pair_ts
    ON rates_cache(pair, timestamp DESC);

CREATE TABLE IF NOT EXISTS rates_checked_minutes (
    pair TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    source TEXT NOT NULL,
    checked_at TEXT NOT NULL,
    granularity TEXT,
    method TEXT,
    PRIMARY KEY (pair, timestamp, source)
);

CREATE INDEX IF NOT EXISTS idx_rates_checked_minutes_pair_ts
    ON rates_checked_minutes(pair, timestamp DESC);

CREATE TABLE IF NOT EXISTS attachments (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    transaction_id TEXT REFERENCES transactions(id) ON DELETE CASCADE,
    attachment_type TEXT NOT NULL,
    label TEXT NOT NULL,
    original_filename TEXT,
    stored_relpath TEXT,
    source_url TEXT,
    media_type TEXT,
    size_bytes INTEGER,
    sha256 TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_attachments_profile_tx_created
    ON attachments(profile_id, transaction_id, created_at DESC);

CREATE TABLE IF NOT EXISTS btcpay_provenance_records (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    backend_name TEXT,
    store_id TEXT NOT NULL,
    payment_method_id TEXT,
    record_type TEXT NOT NULL,
    stable_key TEXT NOT NULL,
    invoice_id TEXT,
    payment_id TEXT,
    order_id TEXT,
    status TEXT,
    occurred_at TEXT,
    asset TEXT,
    amount INTEGER,
    txid TEXT,
    payment_hash TEXT,
    destination TEXT,
    fiat_currency TEXT,
    fiat_value_exact TEXT,
    fiat_rate_exact TEXT,
    pricing_timestamp TEXT,
    raw_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(profile_id, stable_key)
);

CREATE INDEX IF NOT EXISTS idx_btcpay_provenance_profile_invoice
    ON btcpay_provenance_records(profile_id, invoice_id, record_type);

CREATE INDEX IF NOT EXISTS idx_btcpay_provenance_profile_txid
    ON btcpay_provenance_records(profile_id, txid) WHERE txid IS NOT NULL;

CREATE TABLE IF NOT EXISTS external_documents (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    document_type TEXT NOT NULL,
    label TEXT NOT NULL,
    external_ref TEXT,
    issuer TEXT,
    counterparty TEXT,
    issued_at TEXT,
    due_at TEXT,
    fiat_currency TEXT,
    fiat_value_exact TEXT,
    review_state TEXT NOT NULL DEFAULT 'draft',
    notes TEXT,
    raw_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_external_documents_profile_ref
    ON external_documents(profile_id, external_ref);

CREATE TABLE IF NOT EXISTS external_document_attachments (
    document_id TEXT NOT NULL REFERENCES external_documents(id) ON DELETE CASCADE,
    attachment_id TEXT NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY(document_id, attachment_id)
);

CREATE TABLE IF NOT EXISTS commercial_links (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    btcpay_record_id TEXT REFERENCES btcpay_provenance_records(id) ON DELETE CASCADE,
    document_id TEXT REFERENCES external_documents(id) ON DELETE CASCADE,
    transaction_id TEXT REFERENCES transactions(id) ON DELETE CASCADE,
    link_type TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'suggested',
    confidence TEXT NOT NULL DEFAULT 'unknown',
    method TEXT NOT NULL DEFAULT 'manual',
    allocation_amount INTEGER,
    allocation_fiat_exact TEXT,
    reconciliation_state TEXT NOT NULL DEFAULT 'unreviewed',
    commercial_kind TEXT,
    applied_transaction_snapshot_json TEXT,
    reviewed_record_snapshot_json TEXT,
    reviewed_record_snapshot_sha256 TEXT,
    notes TEXT,
    reviewed_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (btcpay_record_id IS NOT NULL OR document_id IS NOT NULL),
    CHECK (transaction_id IS NOT NULL OR document_id IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS idx_commercial_links_profile_state
    ON commercial_links(profile_id, state, reconciliation_state);

CREATE UNIQUE INDEX IF NOT EXISTS idx_commercial_links_unique_payment_tx_active
    ON commercial_links(
        profile_id,
        COALESCE(btcpay_record_id, ''),
        COALESCE(transaction_id, ''),
        link_type
    ) WHERE state != 'rejected' AND link_type = 'btcpay_payment_transaction';

CREATE UNIQUE INDEX IF NOT EXISTS idx_commercial_links_unique_other_active
    ON commercial_links(
        profile_id,
        COALESCE(btcpay_record_id, ''),
        COALESCE(document_id, ''),
        COALESCE(transaction_id, ''),
        link_type
    ) WHERE state != 'rejected' AND link_type != 'btcpay_payment_transaction';

CREATE UNIQUE INDEX IF NOT EXISTS idx_commercial_links_one_reviewed_btcpay_payment
    ON commercial_links(profile_id, btcpay_record_id)
    WHERE state = 'reviewed'
      AND link_type = 'btcpay_payment_transaction'
      AND btcpay_record_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_external_documents_profile_external_ref_unique
    ON external_documents(profile_id, external_ref)
    WHERE external_ref IS NOT NULL AND external_ref != '';

CREATE TABLE IF NOT EXISTS source_funds_sources (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    source_type TEXT NOT NULL,
    label TEXT NOT NULL,
    asset TEXT NOT NULL,
    amount INTEGER,
    fiat_currency TEXT,
    fiat_value REAL,
    acquired_at TEXT,
    description TEXT,
    review_state TEXT NOT NULL DEFAULT 'reviewed',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_source_funds_sources_profile_type
    ON source_funds_sources(profile_id, source_type, created_at DESC);

CREATE TABLE IF NOT EXISTS source_funds_links (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    from_source_id TEXT REFERENCES source_funds_sources(id) ON DELETE CASCADE,
    from_transaction_id TEXT REFERENCES transactions(id) ON DELETE CASCADE,
    to_transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    link_type TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'suggested',
    confidence TEXT NOT NULL DEFAULT 'unknown',
    method TEXT NOT NULL DEFAULT 'manual',
    asset TEXT NOT NULL,
    allocation_amount INTEGER,
    from_asset TEXT,
    from_allocation_amount INTEGER,
    allocation_policy TEXT NOT NULL DEFAULT 'unknown',
    explanation TEXT,
    uses_chain_observation INTEGER NOT NULL DEFAULT 0,
    chain_data_confirmed INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (
        (from_source_id IS NOT NULL AND from_transaction_id IS NULL)
        OR (from_source_id IS NULL AND from_transaction_id IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS idx_source_funds_links_profile_to
    ON source_funds_links(profile_id, to_transaction_id, state);

CREATE INDEX IF NOT EXISTS idx_source_funds_links_profile_from_tx
    ON source_funds_links(profile_id, from_transaction_id);

CREATE TABLE IF NOT EXISTS source_funds_link_attachments (
    link_id TEXT NOT NULL REFERENCES source_funds_links(id) ON DELETE CASCADE,
    attachment_id TEXT NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY (link_id, attachment_id)
);

CREATE TABLE IF NOT EXISTS source_funds_source_attachments (
    source_id TEXT NOT NULL REFERENCES source_funds_sources(id) ON DELETE CASCADE,
    attachment_id TEXT NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY (source_id, attachment_id)
);

CREATE TABLE IF NOT EXISTS source_funds_cases (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    target_transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    target_external_id TEXT,
    target_amount INTEGER NOT NULL,
    asset TEXT NOT NULL,
    label TEXT,
    reveal_mode TEXT NOT NULL,
    status TEXT NOT NULL,
    snapshot_hash TEXT NOT NULL,
    snapshot_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_source_funds_cases_profile_created
    ON source_funds_cases(profile_id, created_at DESC);

CREATE TABLE IF NOT EXISTS source_funds_snapshots (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL REFERENCES source_funds_cases(id) ON DELETE CASCADE,
    snapshot_hash TEXT NOT NULL,
    snapshot_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS source_funds_recipients (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    kind TEXT NOT NULL,
    default_reveal_mode TEXT NOT NULL DEFAULT 'standard',
    notes TEXT,
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_source_funds_recipients_active_label
    ON source_funds_recipients(profile_id, label) WHERE active = 1;

CREATE TABLE IF NOT EXISTS ai_providers (
    name TEXT PRIMARY KEY,
    base_url TEXT NOT NULL,
    api_key TEXT,
    default_model TEXT,
    kind TEXT NOT NULL,
    notes TEXT,
    acknowledged_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS ai_provider_secret_refs (
    provider_name TEXT PRIMARY KEY REFERENCES ai_providers(name) ON DELETE CASCADE,
    store_id TEXT NOT NULL,
    service TEXT NOT NULL,
    account TEXT NOT NULL,
    state TEXT NOT NULL,
    created_at TEXT NOT NULL,
    rotated_at TEXT
);
"""


def ensure_data_root(data_root):
    """Create `data_root` (and any missing parents) and return it as `Path`."""
    path = Path(data_root).expanduser()
    path.mkdir(parents=True, exist_ok=True)
    return path


def resolve_effective_data_root(data_root):
    """Resolve the active data root, honoring older home/XDG locations.

    Kassiber now prefers a single hidden home folder (`~/.kassiber`) so
    repo checkouts stay stateless by default. Existing users keep working:
    when the caller requested the default hidden-home path and it does not
    exist yet, fall back to the older XDG-style locations.
    """
    requested = Path(data_root).expanduser()
    if requested == Path(DEFAULT_DATA_ROOT).expanduser():
        for legacy in (
            Path(LEGACY_XDG_DATA_ROOT).expanduser(),
            Path(LEGACY_DATA_ROOT).expanduser(),
        ):
            if not requested.exists() and legacy.exists():
                return legacy
    return requested


def resolve_effective_state_root(data_root):
    """Return the root directory that owns `data/`, `config/`, and `exports/`."""
    effective_data_root = Path(resolve_effective_data_root(data_root)).expanduser()
    legacy_roots = {
        Path(LEGACY_XDG_DATA_ROOT).expanduser(),
        Path(LEGACY_DATA_ROOT).expanduser(),
    }
    if effective_data_root in legacy_roots:
        return effective_data_root
    if effective_data_root.name == DEFAULT_DATA_DIRNAME:
        return effective_data_root.parent
    return effective_data_root


def resolve_config_root(data_root):
    """Return the directory that holds human-editable config files."""
    return Path(resolve_effective_state_root(data_root)).expanduser() / DEFAULT_CONFIG_DIRNAME


def resolve_exports_root(data_root):
    """Return the default directory for user-generated exports/report files."""
    return Path(resolve_effective_state_root(data_root)).expanduser() / DEFAULT_EXPORTS_DIRNAME


def resolve_attachments_root(data_root):
    """Return the default directory for locally-managed attachment blobs."""
    return Path(resolve_effective_state_root(data_root)).expanduser() / DEFAULT_ATTACHMENTS_DIRNAME


def resolve_settings_path(data_root):
    """Return the managed JSON settings file path for the active state root."""
    return resolve_config_root(data_root) / DEFAULT_SETTINGS_FILENAME


def ensure_settings_file(data_root, env_file):
    """Create or refresh the managed `settings.json` state manifest."""
    settings_path = resolve_settings_path(data_root)
    payload = {
        "schema_version": 1,
        "app": APP_NAME,
        "paths": {
            "state_root": str(resolve_effective_state_root(data_root)),
            "data_root": str(resolve_effective_data_root(data_root)),
            "database": str(resolve_database_path(resolve_effective_data_root(data_root))),
            "config_root": str(resolve_config_root(data_root)),
            "settings_file": str(settings_path),
            "env_file": str(Path(env_file).expanduser()),
            "exports_root": str(resolve_exports_root(data_root)),
            "attachments_root": str(resolve_attachments_root(data_root)),
        },
    }
    existing = {}
    if settings_path.exists():
        try:
            loaded = json.loads(settings_path.read_text(encoding="utf-8"))
            if isinstance(loaded, dict):
                existing = loaded
        except (OSError, ValueError, json.JSONDecodeError):
            existing = {}
    merged = dict(existing)
    merged["schema_version"] = payload["schema_version"]
    merged["app"] = payload["app"]
    existing_paths = existing.get("paths")
    merged_paths = dict(existing_paths) if isinstance(existing_paths, dict) else {}
    merged_paths.update(payload["paths"])
    merged["paths"] = merged_paths
    if merged == existing:
        return settings_path
    settings_path.write_text(json.dumps(merged, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return settings_path


def resolve_database_path(data_root):
    """Pick `kassiber.sqlite3`, falling back to legacy `satbooks.sqlite3`."""
    root = Path(data_root).expanduser()
    current = root / DEFAULT_DB_FILENAME
    legacy = root / LEGACY_DB_FILENAME
    if current.exists() or not legacy.exists():
        return current
    return legacy


CORE_SCHEMA_TABLES = frozenset({"settings", "workspaces", "profiles"})


def database_has_core_schema(conn):
    rows = conn.execute(
        """
        SELECT name FROM sqlite_master
        WHERE type = 'table'
          AND name IN ('settings', 'workspaces', 'profiles')
        """
    ).fetchall()
    names = {row["name"] if hasattr(row, "keys") else row[0] for row in rows}
    if names != CORE_SCHEMA_TABLES:
        return False
    required_columns = {
        "settings": {"key", "value"},
        "workspaces": {"id", "label"},
        "profiles": {"id", "workspace_id", "label", "fiat_currency"},
    }
    for table, expected in required_columns.items():
        columns = {
            row["name"] if hasattr(row, "keys") else row[1]
            for row in conn.execute(f"PRAGMA table_info({table})").fetchall()
        }
        if not expected.issubset(columns):
            return False
    return True


def open_db(data_root, *, passphrase=None, require_existing_schema=False):
    """Open (and lazily migrate) the SQLite store rooted at `data_root`.

    Returns a connection with `row_factory = Row` and foreign keys
    enabled. Safe to call repeatedly — schema creation uses `IF NOT
    EXISTS` and migrations are conditional on the column type.

    When `passphrase` is provided the database is opened through the
    SQLCipher driver. The keying PRAGMAs (`PRAGMA key`,
    `cipher_compatibility`, `kdf_iter`, `cipher_page_size`) are issued in
    the documented order and verified by reading `sqlite_master` before
    the schema script runs. When `passphrase` is `None` the legacy
    plaintext code path is preserved for backwards compatibility.
    """
    root = ensure_data_root(resolve_effective_data_root(data_root))
    db_path = resolve_database_path(root)

    file_present = db_path.exists() and db_path.stat().st_size > 0
    plaintext_header = (
        secrets_sqlcipher.looks_like_plaintext_sqlite(db_path) if file_present else False
    )
    if require_existing_schema and not file_present:
        raise AppError(
            "database does not contain a Kassiber project schema",
            code="invalid_project_database",
            hint="Choose an existing Kassiber project database, not an empty or missing database file.",
            details={"database": str(db_path)},
            retryable=False,
        )

    if passphrase is None:
        if file_present and not plaintext_header:
            raise AppError(
                "database is encrypted; supply a passphrase via --db-passphrase-fd",
                code="passphrase_required",
                hint="Use `kassiber --db-passphrase-fd <fd> <command>` or rely on the GUI unlock prompt.",
                retryable=False,
            )
        conn = sqlite3.connect(db_path)
        conn.row_factory = sqlite3.Row
        if require_existing_schema and not database_has_core_schema(conn):
            conn.close()
            raise AppError(
                "database does not contain a Kassiber project schema",
                code="invalid_project_database",
                hint="Choose an existing Kassiber project database, not an empty or unrelated SQLite file.",
                details={"database": str(db_path)},
                retryable=False,
            )
        conn.executescript(SCHEMA)
        ensure_schema_compat(conn)
        conn.execute("PRAGMA foreign_keys = ON")
        return conn

    if file_present and plaintext_header:
        raise AppError(
            "database file at this path is plaintext SQLite",
            code="plaintext_database",
            hint="Run `kassiber secrets init` to migrate the existing database before opening it with a passphrase.",
            details={"database": str(db_path)},
            retryable=False,
        )

    conn = secrets_sqlcipher.open_encrypted(
        db_path,
        passphrase,
        row_factory=secrets_sqlcipher.get_row_class(),
    )
    if require_existing_schema and not database_has_core_schema(conn):
        conn.close()
        raise AppError(
            "database does not contain a Kassiber project schema",
            code="invalid_project_database",
            hint="Choose an existing Kassiber project database, not an empty or unrelated SQLCipher file.",
            details={"database": str(db_path)},
            retryable=False,
        )
    conn.executescript(SCHEMA)
    ensure_schema_compat(conn)
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
    ensure_column(conn, "profiles", "journal_input_version", "INTEGER NOT NULL DEFAULT 0")
    ensure_column(conn, "profiles", "last_processed_input_version", "INTEGER NOT NULL DEFAULT 0")
    ensure_column(conn, "backends", "batch_size", "INTEGER")
    ensure_column(conn, "backends", "config_json", "TEXT NOT NULL DEFAULT '{}'")
    ensure_column(conn, "journal_entries", "at_category", "TEXT")
    ensure_column(conn, "journal_entries", "at_kennzahl", "INTEGER")
    ensure_column(conn, "transactions", "confirmed_at", "TEXT")
    ensure_column(conn, "transactions", "fiat_price_source", "TEXT")
    ensure_column(conn, "transactions", "fiat_rate_exact", "TEXT")
    ensure_column(conn, "transactions", "fiat_value_exact", "TEXT")
    ensure_column(conn, "transactions", "pricing_source_kind", "TEXT")
    ensure_column(conn, "transactions", "pricing_provider", "TEXT")
    ensure_column(conn, "transactions", "pricing_pair", "TEXT")
    ensure_column(conn, "transactions", "pricing_timestamp", "TEXT")
    ensure_column(conn, "transactions", "pricing_fetched_at", "TEXT")
    ensure_column(conn, "transactions", "pricing_granularity", "TEXT")
    ensure_column(conn, "transactions", "pricing_method", "TEXT")
    ensure_column(conn, "transactions", "pricing_external_ref", "TEXT")
    ensure_column(conn, "transactions", "pricing_quality", "TEXT")
    ensure_column(conn, "transactions", "commercial_applied_link_id", "TEXT")
    ensure_column(conn, "journal_entries", "fiat_value_exact", "TEXT")
    ensure_column(conn, "journal_entries", "unit_cost_exact", "TEXT")
    ensure_column(conn, "journal_entries", "cost_basis_exact", "TEXT")
    ensure_column(conn, "journal_entries", "proceeds_exact", "TEXT")
    ensure_column(conn, "journal_entries", "gain_loss_exact", "TEXT")
    ensure_column(conn, "journal_entries", "pricing_source_kind", "TEXT")
    ensure_column(conn, "journal_entries", "pricing_quality", "TEXT")
    ensure_column(conn, "rates_cache", "rate_exact", "TEXT")
    ensure_column(conn, "rates_cache", "granularity", "TEXT")
    ensure_column(conn, "rates_cache", "method", "TEXT")
    ensure_column(conn, "rates_cache", "open_rate", "REAL")
    ensure_column(conn, "rates_cache", "open_rate_exact", "TEXT")
    ensure_column(conn, "rates_cache", "high_rate", "REAL")
    ensure_column(conn, "rates_cache", "high_rate_exact", "TEXT")
    ensure_column(conn, "rates_cache", "low_rate", "REAL")
    ensure_column(conn, "rates_cache", "low_rate_exact", "TEXT")
    ensure_column(conn, "rates_cache", "close_rate", "REAL")
    ensure_column(conn, "rates_cache", "close_rate_exact", "TEXT")
    ensure_column(conn, "rates_cache", "volume", "REAL")
    ensure_column(conn, "rates_cache", "volume_exact", "TEXT")
    ensure_column(conn, "rates_cache", "trades", "INTEGER")
    ensure_column(conn, "source_funds_cases", "recipient_id", "TEXT")
    ensure_column(conn, "source_funds_cases", "recipient_label_snapshot", "TEXT")
    ensure_column(conn, "source_funds_cases", "recipient_kind_snapshot", "TEXT")
    ensure_column(conn, "source_funds_cases", "recipient_reveal_mode_snapshot", "TEXT")
    ensure_column(conn, "source_funds_cases", "target_external_id", "TEXT")
    _backfill_source_funds_target_external_id(conn)
    ensure_column(conn, "source_funds_recipients", "active", "INTEGER NOT NULL DEFAULT 1")
    _ensure_ai_provider_secret_refs_schema(conn)
    _drop_legacy_source_funds_recipients_unique(conn)
    _migrate_msat_columns(conn)
    _migrate_nullable_attachment_transactions(conn)
    _backfill_liquid_asset_codes(conn)
    _ensure_swap_matching_schema(conn)
    _ensure_commercial_reconciliation_schema(conn)


def _ensure_ai_provider_secret_refs_schema(conn):
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS ai_provider_secret_refs (
            provider_name TEXT PRIMARY KEY REFERENCES ai_providers(name) ON DELETE CASCADE,
            store_id TEXT NOT NULL,
            service TEXT NOT NULL,
            account TEXT NOT NULL,
            state TEXT NOT NULL,
            created_at TEXT NOT NULL,
            rotated_at TEXT
        )
        """
    )


def _ensure_commercial_reconciliation_schema(conn):
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS btcpay_provenance_records (
            id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
            backend_name TEXT,
            store_id TEXT NOT NULL,
            payment_method_id TEXT,
            record_type TEXT NOT NULL,
            stable_key TEXT NOT NULL,
            invoice_id TEXT,
            payment_id TEXT,
            order_id TEXT,
            status TEXT,
            occurred_at TEXT,
            asset TEXT,
            amount INTEGER,
            txid TEXT,
            payment_hash TEXT,
            destination TEXT,
            fiat_currency TEXT,
            fiat_value_exact TEXT,
            fiat_rate_exact TEXT,
            pricing_timestamp TEXT,
            raw_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(profile_id, stable_key)
        )
        """
    )
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_btcpay_provenance_profile_invoice "
        "ON btcpay_provenance_records(profile_id, invoice_id, record_type)"
    )
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_btcpay_provenance_profile_txid "
        "ON btcpay_provenance_records(profile_id, txid) WHERE txid IS NOT NULL"
    )
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS external_documents (
            id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
            document_type TEXT NOT NULL,
            label TEXT NOT NULL,
            external_ref TEXT,
            issuer TEXT,
            counterparty TEXT,
            issued_at TEXT,
            due_at TEXT,
            fiat_currency TEXT,
            fiat_value_exact TEXT,
            review_state TEXT NOT NULL DEFAULT 'draft',
            notes TEXT,
            raw_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        """
    )
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_external_documents_profile_ref "
        "ON external_documents(profile_id, external_ref)"
    )
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS external_document_attachments (
            document_id TEXT NOT NULL REFERENCES external_documents(id) ON DELETE CASCADE,
            attachment_id TEXT NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
            created_at TEXT NOT NULL,
            PRIMARY KEY(document_id, attachment_id)
        )
        """
    )
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS commercial_links (
            id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
            btcpay_record_id TEXT REFERENCES btcpay_provenance_records(id) ON DELETE CASCADE,
            document_id TEXT REFERENCES external_documents(id) ON DELETE CASCADE,
            transaction_id TEXT REFERENCES transactions(id) ON DELETE CASCADE,
            link_type TEXT NOT NULL,
            state TEXT NOT NULL DEFAULT 'suggested',
            confidence TEXT NOT NULL DEFAULT 'unknown',
            method TEXT NOT NULL DEFAULT 'manual',
            allocation_amount INTEGER,
            allocation_fiat_exact TEXT,
            reconciliation_state TEXT NOT NULL DEFAULT 'unreviewed',
            commercial_kind TEXT,
            applied_transaction_snapshot_json TEXT,
            reviewed_record_snapshot_json TEXT,
            reviewed_record_snapshot_sha256 TEXT,
            notes TEXT,
            reviewed_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            CHECK (btcpay_record_id IS NOT NULL OR document_id IS NOT NULL),
            CHECK (transaction_id IS NOT NULL OR document_id IS NOT NULL)
        )
        """
    )
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_commercial_links_profile_state "
        "ON commercial_links(profile_id, state, reconciliation_state)"
    )
    duplicate_external_ref = conn.execute(
        """
        SELECT 1
        FROM external_documents
        WHERE external_ref IS NOT NULL AND external_ref != ''
        GROUP BY profile_id, external_ref
        HAVING COUNT(*) > 1
        LIMIT 1
        """
    ).fetchone()
    if not duplicate_external_ref:
        conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_external_documents_profile_external_ref_unique "
            "ON external_documents(profile_id, external_ref) "
            "WHERE external_ref IS NOT NULL AND external_ref != ''"
        )
    ensure_column(conn, "commercial_links", "applied_transaction_snapshot_json", "TEXT")
    ensure_column(conn, "commercial_links", "reviewed_record_snapshot_json", "TEXT")
    ensure_column(conn, "commercial_links", "reviewed_record_snapshot_sha256", "TEXT")
    conn.execute("DROP INDEX IF EXISTS idx_commercial_links_unique_active")
    conn.execute("DROP INDEX IF EXISTS idx_commercial_links_unique_payment_tx_active")
    conn.execute("DROP INDEX IF EXISTS idx_commercial_links_unique_other_active")
    conn.execute("DROP INDEX IF EXISTS idx_commercial_links_one_reviewed_btcpay_payment")
    conn.execute(
        """
        CREATE UNIQUE INDEX idx_commercial_links_unique_payment_tx_active
            ON commercial_links(
                profile_id,
                COALESCE(btcpay_record_id, ''),
                COALESCE(transaction_id, ''),
                link_type
            ) WHERE state != 'rejected' AND link_type = 'btcpay_payment_transaction'
        """
    )
    conn.execute(
        """
        CREATE UNIQUE INDEX idx_commercial_links_unique_other_active
            ON commercial_links(
                profile_id,
                COALESCE(btcpay_record_id, ''),
                COALESCE(document_id, ''),
                COALESCE(transaction_id, ''),
                link_type
            ) WHERE state != 'rejected' AND link_type != 'btcpay_payment_transaction'
        """
    )
    conn.execute(
        """
        CREATE UNIQUE INDEX idx_commercial_links_one_reviewed_btcpay_payment
            ON commercial_links(profile_id, btcpay_record_id)
            WHERE state = 'reviewed'
              AND link_type = 'btcpay_payment_transaction'
              AND btcpay_record_id IS NOT NULL
        """
    )
    conn.commit()


def _migrate_nullable_attachment_transactions(conn):
    table_sql = conn.execute(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='attachments'"
    ).fetchone()
    legacy_sql = conn.execute(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='attachments_legacy_notnull_tx'"
    ).fetchone()
    current_sql = (table_sql[0] if table_sql else "") or ""
    if not legacy_sql and "transaction_id TEXT NOT NULL" not in current_sql:
        return
    previous_fk_state = conn.execute("PRAGMA foreign_keys").fetchone()[0]
    conn.commit()
    conn.execute("PRAGMA foreign_keys = OFF")
    conn.execute("BEGIN IMMEDIATE")
    try:
        if not legacy_sql:
            conn.execute("ALTER TABLE attachments RENAME TO attachments_legacy_notnull_tx")
        conn.execute(
            """
            CREATE TABLE IF NOT EXISTS attachments (
                id TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
                transaction_id TEXT REFERENCES transactions(id) ON DELETE CASCADE,
                attachment_type TEXT NOT NULL,
                label TEXT NOT NULL,
                original_filename TEXT,
                stored_relpath TEXT,
                source_url TEXT,
                media_type TEXT,
                size_bytes INTEGER,
                sha256 TEXT,
                created_at TEXT NOT NULL
            )
            """
        )
        conn.execute(
            """
            INSERT OR IGNORE INTO attachments
            SELECT id, workspace_id, profile_id, transaction_id, attachment_type, label,
                   original_filename, stored_relpath, source_url, media_type,
                   size_bytes, sha256, created_at
            FROM attachments_legacy_notnull_tx
            """
        )
        conn.execute("DROP TABLE attachments_legacy_notnull_tx")
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_attachments_profile_tx_created "
            "ON attachments(profile_id, transaction_id, created_at DESC)"
        )
        conn.execute("COMMIT")
    except Exception:
        conn.execute("ROLLBACK")
        raise
    finally:
        conn.execute(f"PRAGMA foreign_keys = {'ON' if previous_fk_state else 'OFF'}")


def _ensure_swap_matching_schema(conn):
    """Add swap-matching columns + partial-unique indexes for transaction_pairs.

    Splits into four ordered steps:
      1. Drop the legacy table-level ``UNIQUE`` constraints on
         ``transaction_pairs`` so soft-deleted pairs don't block re-pairing the
         same legs. Rebuilds the table only when the legacy constraints are
         actually present.
      2. ``ensure_column`` the new nullable columns on existing tables.
      3. Index ``transactions.payment_hash`` for the matcher's exact-lookup
         path.
      4. Re-create the active-pair partial unique indexes that replace the
         legacy table-level constraints.
    """
    _migrate_legacy_transaction_pairs_uniques(conn)
    ensure_column(conn, "transactions", "payment_hash", "TEXT")
    ensure_column(conn, "transactions", "payment_hash_source", "TEXT")
    ensure_column(conn, "transaction_pairs", "swap_fee_msat", "INTEGER")
    ensure_column(conn, "transaction_pairs", "swap_fee_kind", "TEXT")
    ensure_column(conn, "transaction_pairs", "confidence_at_pair", "TEXT")
    ensure_column(conn, "transaction_pairs", "pair_source", "TEXT")
    ensure_column(conn, "transaction_pairs", "deleted_at", "TEXT")
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_transactions_payment_hash "
        "ON transactions(payment_hash) WHERE payment_hash IS NOT NULL"
    )
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_transaction_pairs_active_out "
        "ON transaction_pairs(profile_id, out_transaction_id) WHERE deleted_at IS NULL"
    )
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_transaction_pairs_active_in "
        "ON transaction_pairs(profile_id, in_transaction_id) WHERE deleted_at IS NULL"
    )
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_transaction_pairs_profile_active "
        "ON transaction_pairs(profile_id) WHERE deleted_at IS NULL"
    )
    conn.commit()
    _backfill_payment_hash_from_raw_json(conn)


def _backfill_payment_hash_from_raw_json(conn):
    """Populate ``transactions.payment_hash`` for rows imported before this
    column existed.

    Phoenix CSV exports carry a top-level ``payment_hash`` field which the
    importer stashes verbatim into ``raw_json``. Surfacing it as a queryable
    column lets the matcher use exact payment-hash equality to pair the
    Lightning leg of a submarine swap with the on-chain leg deterministically.

    Strictly conservative — only updates rows where ``payment_hash`` is NULL
    and ``raw_json`` parses as JSON with a top-level ``payment_hash`` that is
    exactly 64 lowercase hex characters. Tags such rows with
    ``payment_hash_source = 'importer_backfill'`` so future audits can tell
    them apart from in-flight importer writes.
    """
    rows = conn.execute(
        """
        SELECT id, raw_json
        FROM transactions
        WHERE payment_hash IS NULL
          AND raw_json LIKE '%payment_hash%'
        """
    ).fetchall()
    if not rows:
        return
    updates = []
    for row in rows:
        try:
            payload = json.loads(row["raw_json"] or "{}")
        except (TypeError, ValueError, json.JSONDecodeError):
            continue
        if not isinstance(payload, dict):
            continue
        candidate = payload.get("payment_hash")
        if not isinstance(candidate, str):
            continue
        text = candidate.strip().lower()
        if len(text) != 64:
            continue
        try:
            bytes.fromhex(text)
        except ValueError:
            continue
        updates.append((text, row["id"]))
    if not updates:
        return
    conn.executemany(
        "UPDATE transactions SET payment_hash = ?, payment_hash_source = 'importer_backfill' "
        "WHERE id = ? AND payment_hash IS NULL",
        updates,
    )
    conn.commit()


def _migrate_legacy_transaction_pairs_uniques(conn):
    """Replace the table-level ``UNIQUE`` constraints with partial indexes.

    The original ``transaction_pairs`` schema declared ``UNIQUE (profile_id,
    out_transaction_id)`` / ``UNIQUE (profile_id, in_transaction_id)`` directly
    on the table, which forces hard deletes when a user wants to unpair and
    re-pair the same legs. Replacing those with partial unique indexes
    (``WHERE deleted_at IS NULL``) lets us soft-delete pairs without losing
    the constraint on active rows.
    """
    row = conn.execute(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='transaction_pairs'"
    ).fetchone()
    if not row:
        return
    table_sql = (row["sql"] if hasattr(row, "keys") else row[0]) or ""
    if "UNIQUE (profile_id, out_transaction_id)" not in table_sql:
        return
    previous_fk_state = conn.execute("PRAGMA foreign_keys").fetchone()[0]
    conn.execute("PRAGMA foreign_keys = OFF")
    try:
        conn.execute("ALTER TABLE transaction_pairs RENAME TO transaction_pairs_legacy")
        conn.execute(
            """
            CREATE TABLE transaction_pairs (
                id TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
                out_transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
                in_transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
                kind TEXT NOT NULL DEFAULT 'manual',
                policy TEXT NOT NULL DEFAULT 'carrying-value',
                notes TEXT,
                created_at TEXT NOT NULL
            )
            """
        )
        conn.execute(
            """
            INSERT INTO transaction_pairs
            (id, workspace_id, profile_id, out_transaction_id, in_transaction_id,
             kind, policy, notes, created_at)
            SELECT id, workspace_id, profile_id, out_transaction_id, in_transaction_id,
                   kind, policy, notes, created_at
            FROM transaction_pairs_legacy
            """
        )
        conn.execute("DROP TABLE transaction_pairs_legacy")
        conn.commit()
    except Exception:
        conn.rollback()
        raise
    finally:
        conn.execute(f"PRAGMA foreign_keys = {'ON' if previous_fk_state else 'OFF'}")


def _backfill_source_funds_target_external_id(conn):
    """Persist the target's external_id at save time on each case row.

    list_cases used to live-join transactions.external_id, which let a
    later txn rename rewrite history. Snapshot the value once so the
    case row is the authoritative answer.
    """
    conn.execute(
        """
        UPDATE source_funds_cases
        SET target_external_id = (
            SELECT t.external_id
            FROM transactions t
            WHERE t.id = source_funds_cases.target_transaction_id
        )
        WHERE target_external_id IS NULL
        """
    )
    conn.commit()


def _drop_legacy_source_funds_recipients_unique(conn):
    """Replace the table-level UNIQUE (profile_id, label) constraint with a
    partial unique index that excludes soft-deleted rows.

    Without this, ``delete_recipient`` (which marks rows ``active = 0``)
    leaves the legacy unique covering the inactive row, so a later
    create with the same label hits IntegrityError.
    """
    row = conn.execute(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='source_funds_recipients'"
    ).fetchone()
    if not row:
        return
    table_sql = (row["sql"] if hasattr(row, "keys") else row[0]) or ""
    if "UNIQUE (profile_id, label)" not in table_sql:
        return
    conn.execute("ALTER TABLE source_funds_recipients RENAME TO source_funds_recipients_legacy")
    conn.execute(
        """
        CREATE TABLE source_funds_recipients (
            id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
            label TEXT NOT NULL,
            kind TEXT NOT NULL,
            default_reveal_mode TEXT NOT NULL DEFAULT 'standard',
            notes TEXT,
            active INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        """
    )
    conn.execute(
        """
        INSERT INTO source_funds_recipients
        (id, workspace_id, profile_id, label, kind, default_reveal_mode, notes,
         active, created_at, updated_at)
        SELECT id, workspace_id, profile_id, label, kind, default_reveal_mode, notes,
               COALESCE(active, 1), created_at, updated_at
        FROM source_funds_recipients_legacy
        """
    )
    conn.execute("DROP TABLE source_funds_recipients_legacy")
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_source_funds_recipients_active_label "
        "ON source_funds_recipients(profile_id, label) WHERE active = 1"
    )
    conn.commit()


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
    migrate_transactions = _column_is_real(conn, "transactions", "amount") or _column_is_real(conn, "transactions", "fee")
    migrate_journal_entries = _column_is_real(conn, "journal_entries", "quantity")
    if not migrate_transactions and not migrate_journal_entries:
        return

    conn.commit()
    previous_fk_state = conn.execute("PRAGMA foreign_keys").fetchone()[0]
    conn.execute("PRAGMA foreign_keys = OFF")
    try:
        if migrate_transactions:
            conn.executescript(
                """
                BEGIN;
                CREATE TABLE transactions__msat_new (
                    id TEXT PRIMARY KEY,
                    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
                    wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
                    external_id TEXT,
                    fingerprint TEXT NOT NULL UNIQUE,
                    occurred_at TEXT NOT NULL,
                    confirmed_at TEXT,
                    direction TEXT NOT NULL,
                    asset TEXT NOT NULL,
                    amount INTEGER NOT NULL,
                    fee INTEGER NOT NULL DEFAULT 0,
                    fiat_currency TEXT,
                    fiat_rate REAL,
                    fiat_value REAL,
                    fiat_price_source TEXT,
                    fiat_rate_exact TEXT,
                    fiat_value_exact TEXT,
                    pricing_source_kind TEXT,
                    pricing_provider TEXT,
                    pricing_pair TEXT,
                    pricing_timestamp TEXT,
                    pricing_fetched_at TEXT,
                    pricing_granularity TEXT,
                    pricing_method TEXT,
                    pricing_external_ref TEXT,
                    pricing_quality TEXT,
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
                    occurred_at, confirmed_at, direction, asset,
                    CAST(ROUND(amount * 100000000000.0) AS INTEGER),
                    CAST(ROUND(fee * 100000000000.0) AS INTEGER),
                    fiat_currency, fiat_rate, fiat_value, fiat_price_source,
                    fiat_rate_exact, fiat_value_exact, pricing_source_kind,
                    pricing_provider, pricing_pair, pricing_timestamp,
                    pricing_fetched_at, pricing_granularity, pricing_method,
                    pricing_external_ref, pricing_quality,
                    kind, description, counterparty, note, excluded, raw_json, created_at
                FROM transactions;
                DROP TABLE transactions;
                ALTER TABLE transactions__msat_new RENAME TO transactions;
                COMMIT;
                """
            )
        if migrate_journal_entries:
            conn.executescript(
                """
                BEGIN;
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
                    fiat_value_exact TEXT,
                    unit_cost_exact TEXT,
                    cost_basis_exact TEXT,
                    proceeds_exact TEXT,
                    gain_loss_exact TEXT,
                    pricing_source_kind TEXT,
                    pricing_quality TEXT,
                    description TEXT,
                    at_category TEXT,
                    at_kennzahl INTEGER,
                    created_at TEXT NOT NULL
                );
                INSERT INTO journal_entries__msat_new SELECT
                    id, workspace_id, profile_id, transaction_id, wallet_id, account_id,
                    occurred_at, entry_type, asset,
                    CAST(ROUND(quantity * 100000000000.0) AS INTEGER),
                    fiat_value, unit_cost, cost_basis, proceeds, gain_loss,
                    fiat_value_exact, unit_cost_exact, cost_basis_exact,
                    proceeds_exact, gain_loss_exact, pricing_source_kind,
                    pricing_quality, description,
                    at_category, at_kennzahl, created_at
                FROM journal_entries;
                DROP TABLE journal_entries;
                ALTER TABLE journal_entries__msat_new RENAME TO journal_entries;
                COMMIT;
                """
            )
    except Exception:
        conn.rollback()
        raise
    finally:
        conn.execute(f"PRAGMA foreign_keys = {'ON' if previous_fk_state else 'OFF'}")


def _raw_decimal_for_fingerprint(row, raw_key, msat_key):
    stored_msat = int(row[msat_key] or 0)
    try:
        payload = json.loads(row["raw_json"] or "{}")
    except (TypeError, ValueError, json.JSONDecodeError):
        payload = {}
    if isinstance(payload, dict):
        raw_value = payload.get(raw_key)
        if raw_value not in (None, ""):
            try:
                # Raw imports may carry signed amounts; fingerprint inputs are normalized positive values.
                value = abs(dec(raw_value))
            except (AppError, TypeError, ValueError):
                value = None
            if value is not None and btc_to_msat(value) == stored_msat:
                return value
    return msat_to_btc(stored_msat)


def _backfilled_transaction_fingerprint(row, asset_code):
    return make_transaction_fingerprint(
        row["wallet_id"],
        row["external_id"] or "",
        row["occurred_at"],
        row["direction"],
        asset_code,
        _raw_decimal_for_fingerprint(row, "amount", "amount"),
        _raw_decimal_for_fingerprint(row, "fee", "fee"),
    )


def _backfill_liquid_asset_codes(conn):
    """Heal Liquid transactions whose asset was stored as a raw policy-asset hex.

    Early Liquid descriptor wallets could be created with a symbolic ``policy_asset``
    (e.g. ``L-BTC``), which made the sync decoder leave the 64-char hex asset id on
    each record instead of normalizing to ``LBTC`` — auto-pricing then skipped them
    because the fiat-rate alias is keyed on ``LBTC``. Rewrite the hex to ``LBTC`` and
    invalidate only the profiles that owned hex rows so the next ``journals process``
    reprices them, leaving untouched any profile that already had clean ``LBTC`` data.
    """
    policy_asset_hexes = tuple(sorted({value.lower() for value in LIQUID_POLICY_ASSET_IDS.values() if value}))
    if not policy_asset_hexes:
        return
    placeholders = ",".join("?" for _ in policy_asset_hexes)
    affected_rows = conn.execute(
        f"""
        SELECT id, profile_id, wallet_id, external_id, occurred_at, direction,
               amount, fee, raw_json
        FROM transactions
        WHERE lower(asset) IN ({placeholders})
        """,
        policy_asset_hexes,
    ).fetchall()
    affected_profile_ids = sorted({row["profile_id"] for row in affected_rows})
    if not affected_profile_ids:
        return
    for row in affected_rows:
        fingerprint = _backfilled_transaction_fingerprint(row, "LBTC")
        collision = conn.execute(
            "SELECT id FROM transactions WHERE fingerprint = ? AND id != ? LIMIT 1",
            (fingerprint, row["id"]),
        ).fetchone()
        if collision:
            conn.execute("UPDATE transactions SET asset = 'LBTC' WHERE id = ?", (row["id"],))
        else:
            conn.execute(
                "UPDATE transactions SET asset = 'LBTC', fingerprint = ? WHERE id = ?",
                (fingerprint, row["id"]),
            )
    profile_placeholders = ",".join("?" for _ in affected_profile_ids)
    conn.execute(
        f"UPDATE profiles "
        f"SET last_processed_at = NULL, "
        f"last_processed_tx_count = 0, "
        f"journal_input_version = journal_input_version + 1 "
        f"WHERE id IN ({profile_placeholders})",
        affected_profile_ids,
    )
    conn.commit()
