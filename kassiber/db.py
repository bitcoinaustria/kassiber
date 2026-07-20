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

import hashlib
import json
import os
import sqlite3
import stat
import tempfile
import uuid
from contextlib import contextmanager, suppress
from pathlib import Path

from .errors import AppError
from .fingerprints import make_transaction_fingerprint
from .msat import btc_to_msat, dec, msat_to_btc
from .secrets import sqlcipher as secrets_sqlcipher
from .tax_policy import DEFAULT_LONG_TERM_DAYS, DEFAULT_TAX_COUNTRY
from .wallet_descriptors import (
    LIQUID_POLICY_ASSET_IDS,
    default_policy_asset_id,
    normalize_asset_code,
    normalize_chain,
    normalize_network,
)


APP_NAME = "kassiber"
LEGACY_APP_NAME = "satbooks"
DEFAULT_STATE_ROOT = os.path.expanduser(f"~/.{APP_NAME}")
DEFAULT_DATA_DIRNAME = "data"
DEFAULT_CONFIG_DIRNAME = "config"
DEFAULT_EXPORTS_DIRNAME = "exports"
DEFAULT_ATTACHMENTS_DIRNAME = "attachments"
DEFAULT_SETTINGS_FILENAME = "settings.json"
DATABASE_INSTANCE_ID_SETTING = "database_instance_id"
DEFAULT_DATA_ROOT = os.path.join(DEFAULT_STATE_ROOT, DEFAULT_DATA_DIRNAME)
LEGACY_XDG_DATA_ROOT = os.path.expanduser(f"~/.local/share/{APP_NAME}")


def safe_sqlite_error_details(exc: Exception) -> dict[str, object]:
    """Return driver-neutral, non-sensitive SQLite diagnostics."""

    error_name = getattr(exc, "sqlite_errorname", None)
    error_code = getattr(exc, "sqlite_errorcode", None)
    if not (
        isinstance(error_name, str)
        and error_name.startswith("SQLITE_")
        and error_name.replace("_", "").isalnum()
    ):
        return {}
    details: dict[str, object] = {
        "error_class": f"{exc.__class__.__module__}.{exc.__class__.__qualname__}",
        "sqlite_error_name": error_name,
    }
    if type(error_code) is int and error_code >= 0:
        details["sqlite_error_code"] = error_code
    return details


def sqlite_error_is_busy(details: dict[str, object]) -> bool:
    """Recognize base and extended BUSY/LOCKED codes across DB-API drivers."""

    error_name = str(details.get("sqlite_error_name") or "")
    if error_name.startswith(("SQLITE_BUSY", "SQLITE_LOCKED")):
        return True
    error_code = details.get("sqlite_error_code")
    return type(error_code) is int and (error_code & 0xFF) in {
        sqlite3.SQLITE_BUSY,
        sqlite3.SQLITE_LOCKED,
    }
LEGACY_DATA_ROOT = os.path.expanduser(f"~/.local/share/{LEGACY_APP_NAME}")
DEFAULT_DB_FILENAME = f"{APP_NAME}.sqlite3"
LEGACY_DB_FILENAME = f"{LEGACY_APP_NAME}.sqlite3"
DB_BUSY_TIMEOUT_MS = 30_000
DB_BUSY_TIMEOUT_SECONDS = DB_BUSY_TIMEOUT_MS / 1000
DB_JOURNAL_MODE = "wal"
# `NORMAL` is the standard, crash-safe pairing for WAL: it only drops an fsync
# per commit (a power-loss can lose the last transaction, never corrupt the DB),
# which removes the dominant cost from write-heavy refresh paths (per-row sync
# inserts, the journal delete+rebuild, UTXO inventory writes).
DB_SYNCHRONOUS = "NORMAL"
# Spill temp B-trees/sort runs to RAM instead of disk during reports/journaling.
DB_TEMP_STORE = "MEMORY"
# Negative cache_size is in KiB (here ~16 MiB) rather than pages, so the page
# cache size is independent of the page size.
DB_CACHE_SIZE_KIB = -16_000
# Memory-map up to 256 MiB of the database for faster reads. This is a no-op on
# SQLCipher connections (mmap is disabled there for security), so it only helps
# the plaintext store and never interferes with the cipher keying sequence.
DB_MMAP_SIZE_BYTES = 268_435_456
SWAP_FEE_PAIR_KINDS = (
    "chain-swap",
    "peg-in",
    "peg-out",
    "reverse-submarine-swap",
    "submarine-swap",
    "swap-refund",
)

CUSTODY_DURABLE_EVIDENCE_MIGRATION = "custody-durable-evidence-v1"
_CUSTODY_MIGRATION_EXPLANATIONS = {
    "durable_transaction_anchors": (
        "Copies each extant leg transaction id into the immutable anchor so "
        "later source retraction cannot erase the reviewed reference."
    ),
    "payload_free_evidence_commitments": (
        "Seals existing active authored evidence with replicable hashes while "
        "retaining raw evidence only in the local snapshot table."
    ),
}


SCHEMA = """
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Local append-only schema migration reports. These contain only bounded
-- migration names/counts/explanations, never raw transactions or evidence.
CREATE TABLE IF NOT EXISTS schema_migration_audits (
    id TEXT PRIMARY KEY,
    migration_name TEXT NOT NULL UNIQUE,
    schema_version INTEGER NOT NULL CHECK(schema_version >= 1),
    impact_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TRIGGER IF NOT EXISTS trg_schema_migration_audits_immutable
BEFORE UPDATE ON schema_migration_audits
BEGIN
    SELECT RAISE(ABORT, 'schema_migration_audits_immutable');
END;

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
    require_coarse_review INTEGER NOT NULL DEFAULT 0,
    bitcoin_rail_carrying_value INTEGER NOT NULL DEFAULT 1,
    journal_input_version INTEGER NOT NULL DEFAULT 0,
    last_processed_input_version INTEGER NOT NULL DEFAULT 0,
    last_processed_at TEXT,
    last_processed_tx_count INTEGER NOT NULL DEFAULT 0,
    ownership_review_counts_json TEXT,
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

-- Private dependency-observer state. These rows live only in the main
-- SQLite/SQLCipher store; no public, audit, AI, diagnostic, or replication
-- surface selects them. ``logical_wallet_id`` lets one grouped wallet own
-- multiple observer instances while ``source_wallet_id`` identifies the
-- concrete descriptor source that is refreshed.
CREATE TABLE IF NOT EXISTS chain_observer_instances (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    logical_wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    source_wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    source_key TEXT NOT NULL,
    observer_kind TEXT NOT NULL,
    chain TEXT NOT NULL,
    network TEXT NOT NULL,
    state_version INTEGER NOT NULL,
    state_epoch INTEGER NOT NULL DEFAULT 0,
    state_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (logical_wallet_id, source_wallet_id, source_key)
);

CREATE INDEX IF NOT EXISTS idx_chain_observer_instances_profile
    ON chain_observer_instances(profile_id, logical_wallet_id, source_wallet_id);

CREATE TABLE IF NOT EXISTS chain_observer_coverage (
    observer_id TEXT NOT NULL REFERENCES chain_observer_instances(id) ON DELETE CASCADE,
    branch_key TEXT NOT NULL,
    coverage_version INTEGER NOT NULL,
    highest_used INTEGER,
    scanned_to INTEGER NOT NULL,
    details_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL,
    PRIMARY KEY (observer_id, branch_key)
);

-- Wallet-policy epochs are the durable custody-facing interpretation of
-- disposable observer state.  Their random ids do not fingerprint descriptor
-- or xpub material.  ``private_material_json`` remains inside SQLCipher and is
-- used only to recognize outputs from retired policies; it is excluded from
-- public, AI, audit, diagnostic, and replication surfaces.
CREATE TABLE IF NOT EXISTS wallet_policy_epochs (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    chain TEXT NOT NULL,
    network TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('active', 'retired')),
    private_material_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    retired_at TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_wallet_policy_epochs_one_active
    ON wallet_policy_epochs(wallet_id)
    WHERE status = 'active';

CREATE INDEX IF NOT EXISTS idx_wallet_policy_epochs_profile
    ON wallet_policy_epochs(profile_id, wallet_id, status, created_at);

CREATE TABLE IF NOT EXISTS wallet_policy_sources (
    id TEXT PRIMARY KEY,
    epoch_id TEXT NOT NULL REFERENCES wallet_policy_epochs(id) ON DELETE CASCADE,
    source_wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    source_key TEXT NOT NULL,
    observer_kind TEXT NOT NULL,
    branch_keys_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(epoch_id, source_wallet_id, source_key)
);

CREATE TABLE IF NOT EXISTS wallet_policy_coverage_witnesses (
    source_id TEXT NOT NULL REFERENCES wallet_policy_sources(id) ON DELETE CASCADE,
    branch_key TEXT NOT NULL,
    scanned_to_exclusive INTEGER NOT NULL CHECK(scanned_to_exclusive >= 0),
    highest_used INTEGER,
    observer_kind TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    PRIMARY KEY(source_id, branch_key)
);

-- Opaque dependency-owned key/value state. Values are intentionally BLOBs:
-- Kassiber namespaces and versions them but never interprets their contents.
-- The FK keeps the values inside the observer row's SQLCipher transaction.
CREATE TABLE IF NOT EXISTS chain_observer_values (
    observer_id TEXT NOT NULL REFERENCES chain_observer_instances(id) ON DELETE CASCADE,
    namespace_version INTEGER NOT NULL,
    key TEXT NOT NULL,
    value BLOB NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (observer_id, key)
);

CREATE TABLE IF NOT EXISTS transactions (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    external_id TEXT,
    -- Closed, public discriminator for the meaning of external_id. Raw import
    -- payloads stay local; peers need only this marker to retain native chain
    -- identity after replication strips raw_json.
    external_id_kind TEXT CHECK(external_id_kind IS NULL OR external_id_kind = 'txid'),
    fingerprint TEXT NOT NULL UNIQUE,
    occurred_at TEXT NOT NULL,
    confirmed_at TEXT,
    direction TEXT NOT NULL,
    asset TEXT NOT NULL,
    amount INTEGER NOT NULL,
    fee INTEGER NOT NULL DEFAULT 0,
    -- 1 when `amount` is a net wallet-balance delta with the network fee folded
    -- in and no separate fee is available (BTCPay Greenfield sync). 0 (the
    -- default) means `amount` is recipient-only and `fee` carries the miner fee
    -- (esplora/electrum/bitcoinrpc and every CSV importer).
    amount_includes_fee INTEGER NOT NULL DEFAULT 0,
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
    review_status TEXT,
    taxability_override INTEGER,
    at_regime_override TEXT,
    at_category_override TEXT,
    privacy_boundary TEXT,
    kind TEXT,
    description TEXT,
    counterparty TEXT,
    note TEXT,
    excluded INTEGER NOT NULL DEFAULT 0,
    raw_json TEXT NOT NULL DEFAULT '{}',
    payment_hash TEXT,
    payment_hash_source TEXT,
    swap_refund_funding_txid TEXT,
    swap_refund_funding_vout INTEGER,
    created_at TEXT NOT NULL
);

-- Closed authority channel from an applied dependency observer to the current
-- normalized transaction projection. Generic imports cannot write this table;
-- custody consumers must also verify the stored graph/quantity hashes against
-- the current row before elevating native evidence.
CREATE TABLE IF NOT EXISTS chain_observation_provenance (
    transaction_id TEXT PRIMARY KEY REFERENCES transactions(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    authority_version INTEGER NOT NULL CHECK(authority_version >= 1),
    observer_ids_json TEXT NOT NULL,
    observer_kinds_json TEXT NOT NULL,
    chain TEXT NOT NULL,
    network TEXT NOT NULL,
    application_revision TEXT NOT NULL,
    graph_hash TEXT NOT NULL,
    quantity_hash TEXT NOT NULL,
    fee_attribution TEXT NOT NULL CHECK(
        fee_attribution IN ('exact', 'implicit_wallet_delta', 'unknown')
    ),
    observed_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_chain_observation_provenance_profile
    ON chain_observation_provenance(profile_id, wallet_id, chain, network);

CREATE TABLE IF NOT EXISTS transaction_graph_cache (
    schema_version INTEGER NOT NULL,
    chain TEXT NOT NULL,
    network TEXT NOT NULL,
    txid TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (schema_version, chain, network, txid)
);

CREATE INDEX IF NOT EXISTS idx_transaction_graph_cache_updated
    ON transaction_graph_cache(updated_at DESC);

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

CREATE INDEX IF NOT EXISTS idx_transactions_profile_external_id
    ON transactions(profile_id, external_id) WHERE external_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_transactions_profile_active_time
    ON transactions(profile_id, excluded, occurred_at, created_at, id);

CREATE INDEX IF NOT EXISTS idx_transactions_wallet_external_match
    ON transactions(wallet_id, external_id, direction, asset, amount, fee, created_at)
    WHERE external_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_transactions_profile_economic_match
    ON transactions(profile_id, direction, asset, amount, occurred_at, created_at);

CREATE TABLE IF NOT EXISTS wallet_utxos (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    backend_name TEXT,
    backend_kind TEXT,
    chain TEXT NOT NULL,
    network TEXT NOT NULL,
    asset TEXT NOT NULL,
    amount INTEGER NOT NULL,
    txid TEXT NOT NULL,
    vout INTEGER NOT NULL,
    outpoint TEXT NOT NULL,
    confirmation_status TEXT NOT NULL,
    confirmations INTEGER,
    block_height INTEGER,
    block_time TEXT,
    address TEXT,
    script_pubkey TEXT,
    address_label TEXT,
    branch_label TEXT,
    branch_index INTEGER,
    address_index INTEGER,
    anonymity_score INTEGER,
    spent_by TEXT,
    excluded_from_coinjoin INTEGER,
    key_state TEXT,
    anon_history_json TEXT NOT NULL DEFAULT '[]',
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    spent_at TEXT,
    raw_json TEXT NOT NULL DEFAULT '{}',
    UNIQUE (wallet_id, txid, vout)
);

CREATE INDEX IF NOT EXISTS idx_wallet_utxos_wallet_active
    ON wallet_utxos(wallet_id, spent_at, asset, block_height, txid, vout);

CREATE INDEX IF NOT EXISTS idx_wallet_utxos_profile_wallet
    ON wallet_utxos(profile_id, wallet_id, asset);

CREATE TABLE IF NOT EXISTS wallet_utxo_refreshes (
    wallet_id TEXT PRIMARY KEY REFERENCES wallets(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    backend_name TEXT,
    backend_kind TEXT,
    chain TEXT NOT NULL,
    network TEXT NOT NULL,
    observed_count INTEGER NOT NULL DEFAULT 0,
    active_count INTEGER NOT NULL DEFAULT 0,
    last_seen_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_wallet_utxo_refreshes_profile
    ON wallet_utxo_refreshes(profile_id, wallet_id);

CREATE TABLE IF NOT EXISTS transaction_edit_events (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    wallet_id TEXT REFERENCES wallets(id) ON DELETE SET NULL,
    transaction_external_id TEXT,
    transaction_occurred_at TEXT,
    source TEXT NOT NULL,
    reason TEXT,
    changed_at TEXT NOT NULL,
    journal_input_version INTEGER NOT NULL DEFAULT 0,
    journal_input_version_after INTEGER NOT NULL DEFAULT 0,
    last_processed_input_version INTEGER NOT NULL DEFAULT 0,
    last_processed_at TEXT,
    last_processed_tx_count INTEGER NOT NULL DEFAULT 0,
    sync_event_id TEXT,
    sync_replica_id TEXT,
    sync_replica_seq INTEGER,
    sync_hlc TEXT,
    sync_author_member_id TEXT,
    sync_signature TEXT,
    sync_context_json TEXT
);

CREATE TABLE IF NOT EXISTS transaction_edit_fields (
    id TEXT PRIMARY KEY,
    event_id TEXT NOT NULL REFERENCES transaction_edit_events(id) ON DELETE CASCADE,
    field TEXT NOT NULL,
    before_value TEXT,
    after_value TEXT,
    diff_json TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_transaction_edit_events_profile_changed
    ON transaction_edit_events(profile_id, changed_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_transaction_edit_events_transaction_changed
    ON transaction_edit_events(transaction_id, changed_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_transaction_edit_events_source_changed
    ON transaction_edit_events(profile_id, source, changed_at DESC);

CREATE INDEX IF NOT EXISTS idx_transaction_edit_events_wallet_changed
    ON transaction_edit_events(profile_id, wallet_id, changed_at DESC);

CREATE INDEX IF NOT EXISTS idx_transaction_edit_fields_event
    ON transaction_edit_fields(event_id);

CREATE INDEX IF NOT EXISTS idx_transaction_edit_fields_field
    ON transaction_edit_fields(field, event_id);

-- Cross-device replication is strictly opt-in. Merely opening a database
-- creates these empty schema tables, but no identity, key, event, transport,
-- listener, or other behavior exists until a profile is explicitly enabled.
CREATE TABLE IF NOT EXISTS sync_books (
    profile_id TEXT PRIMARY KEY REFERENCES profiles(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    book_id TEXT NOT NULL UNIQUE,
    enabled INTEGER NOT NULL DEFAULT 1,
    local_member_id TEXT NOT NULL,
    local_device_id TEXT NOT NULL,
    local_replica_id TEXT NOT NULL,
    hmac_key_b64 TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sync_members (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    display_name TEXT NOT NULL,
    signing_public_key_b64 TEXT NOT NULL,
    role TEXT NOT NULL CHECK(role IN ('owner', 'editor', 'auditor')),
    added_hlc TEXT NOT NULL,
    added_at TEXT NOT NULL,
    revoked_hlc TEXT,
    revoked_at TEXT,
    revoked_context_json TEXT,
    inviter_member_id TEXT,
    record_signature TEXT NOT NULL,
    UNIQUE(profile_id, signing_public_key_b64)
);

CREATE INDEX IF NOT EXISTS idx_sync_members_profile_active
    ON sync_members(profile_id, role, revoked_at);

CREATE TABLE IF NOT EXISTS sync_member_private_keys (
    member_id TEXT PRIMARY KEY REFERENCES sync_members(id) ON DELETE CASCADE,
    signing_private_key_b64 TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sync_devices (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    member_id TEXT NOT NULL REFERENCES sync_members(id) ON DELETE CASCADE,
    recipient_public_key TEXT NOT NULL,
    label TEXT NOT NULL,
    paired_hlc TEXT NOT NULL,
    paired_at TEXT NOT NULL,
    last_seen_at TEXT,
    revoked_hlc TEXT,
    revoked_at TEXT,
    revoked_context_json TEXT,
    record_signer_member_id TEXT,
    record_signature TEXT NOT NULL,
    UNIQUE(profile_id, recipient_public_key)
);

CREATE INDEX IF NOT EXISTS idx_sync_devices_profile_active
    ON sync_devices(profile_id, member_id, revoked_at);

CREATE TABLE IF NOT EXISTS sync_device_private_keys (
    device_id TEXT PRIMARY KEY REFERENCES sync_devices(id) ON DELETE CASCADE,
    age_identity TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sync_replicas (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    member_id TEXT NOT NULL REFERENCES sync_members(id) ON DELETE CASCADE,
    device_id TEXT NOT NULL REFERENCES sync_devices(id) ON DELETE CASCADE,
    last_seq INTEGER NOT NULL DEFAULT 0,
    last_hlc TEXT,
    last_event_hash TEXT,
    last_seen_at TEXT,
    created_at TEXT NOT NULL,
    UNIQUE(profile_id, member_id, device_id)
);

CREATE INDEX IF NOT EXISTS idx_sync_replicas_profile
    ON sync_replicas(profile_id, id);

CREATE TABLE IF NOT EXISTS sync_events (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    replica_id TEXT NOT NULL,
    replica_seq INTEGER NOT NULL,
    hlc TEXT NOT NULL,
    author_member_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    entity_table TEXT NOT NULL,
    entity_key TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    context_json TEXT NOT NULL,
    previous_hash TEXT,
    event_hash TEXT NOT NULL,
    signature TEXT NOT NULL,
    created_at TEXT NOT NULL,
    applied_at TEXT NOT NULL,
    UNIQUE(replica_id, replica_seq)
);

CREATE INDEX IF NOT EXISTS idx_sync_events_profile_hlc
    ON sync_events(profile_id, hlc, replica_id, replica_seq);

CREATE INDEX IF NOT EXISTS idx_sync_events_entity
    ON sync_events(profile_id, entity_table, entity_key, hlc);

CREATE TABLE IF NOT EXISTS sync_row_state (
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    entity_table TEXT NOT NULL,
    entity_key TEXT NOT NULL,
    row_hash TEXT,
    last_event_id TEXT REFERENCES sync_events(id) ON DELETE SET NULL,
    last_hlc TEXT NOT NULL,
    tombstoned INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL,
    PRIMARY KEY(profile_id, entity_table, entity_key)
);

CREATE INDEX IF NOT EXISTS idx_sync_row_state_profile_table
    ON sync_row_state(profile_id, entity_table, tombstoned);

CREATE TABLE IF NOT EXISTS sync_tombstones (
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    entity_table TEXT NOT NULL,
    entity_key TEXT NOT NULL,
    event_id TEXT NOT NULL REFERENCES sync_events(id) ON DELETE CASCADE,
    hlc TEXT NOT NULL,
    deleted_by_member_id TEXT NOT NULL,
    deleted_at TEXT NOT NULL,
    gc_after TEXT,
    PRIMARY KEY(profile_id, entity_table, entity_key)
);

CREATE TABLE IF NOT EXISTS sync_ingests (
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    replica_id TEXT NOT NULL,
    first_seq INTEGER NOT NULL,
    last_seq INTEGER NOT NULL,
    bundle_hash TEXT NOT NULL,
    prior_bundle_hash TEXT,
    ingested_at TEXT NOT NULL,
    UNIQUE(profile_id, bundle_hash)
);

CREATE INDEX IF NOT EXISTS idx_sync_ingests_replica_range
    ON sync_ingests(profile_id, replica_id, first_seq, last_seq);

CREATE TABLE IF NOT EXISTS sync_conflicts (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    entity_table TEXT NOT NULL,
    entity_key TEXT NOT NULL,
    field TEXT NOT NULL,
    local_event_id TEXT NOT NULL,
    remote_event_id TEXT NOT NULL,
    local_value_json TEXT,
    remote_value_json TEXT,
    status TEXT NOT NULL DEFAULT 'open' CHECK(status IN ('open', 'resolved')),
    resolution_event_id TEXT,
    resolved_by_member_id TEXT,
    resolved_at TEXT,
    created_at TEXT NOT NULL,
    UNIQUE(profile_id, entity_table, entity_key, field, local_event_id, remote_event_id)
);

CREATE INDEX IF NOT EXISTS idx_sync_conflicts_profile_open
    ON sync_conflicts(profile_id, status, created_at);

CREATE TABLE IF NOT EXISTS sync_notices (
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    code TEXT NOT NULL,
    severity TEXT NOT NULL CHECK(severity IN ('info', 'warning', 'blocking')),
    replica_id TEXT,
    member_id TEXT,
    details_json TEXT NOT NULL DEFAULT '{}',
    acknowledged_at TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sync_notices_profile_open
    ON sync_notices(profile_id, acknowledged_at, created_at);

CREATE TABLE IF NOT EXISTS sync_bundle_exports (
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    replica_id TEXT NOT NULL,
    last_seq INTEGER NOT NULL DEFAULT 0,
    last_bundle_hash TEXT,
    exported_at TEXT,
    PRIMARY KEY(profile_id, replica_id)
);

CREATE TABLE IF NOT EXISTS sync_pending_events (
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    replica_id TEXT NOT NULL,
    replica_seq INTEGER NOT NULL,
    event_json TEXT NOT NULL,
    bundle_hash TEXT NOT NULL,
    received_at TEXT NOT NULL,
    PRIMARY KEY(profile_id, replica_id, replica_seq)
);

CREATE INDEX IF NOT EXISTS idx_sync_pending_events_next
    ON sync_pending_events(profile_id, replica_id, replica_seq);

CREATE TABLE IF NOT EXISTS sync_rejected_events (
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    replica_id TEXT NOT NULL,
    replica_seq INTEGER NOT NULL,
    event_hash TEXT NOT NULL,
    reason TEXT NOT NULL,
    received_at TEXT NOT NULL,
    PRIMARY KEY(profile_id, replica_id, replica_seq)
);

CREATE TABLE IF NOT EXISTS sync_pending_blobs (
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    bundle_hash TEXT NOT NULL,
    content_hmac TEXT NOT NULL,
    payload BLOB NOT NULL,
    PRIMARY KEY(profile_id, bundle_hash, content_hmac)
);

CREATE TABLE IF NOT EXISTS sync_field_state (
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    entity_table TEXT NOT NULL,
    entity_key TEXT NOT NULL,
    field TEXT NOT NULL,
    event_id TEXT NOT NULL REFERENCES sync_events(id) ON DELETE CASCADE,
    hlc TEXT NOT NULL,
    value_json TEXT,
    PRIMARY KEY(profile_id, entity_table, entity_key, field)
);

CREATE TABLE IF NOT EXISTS sync_id_map (
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    entity_table TEXT NOT NULL,
    wire_id TEXT NOT NULL,
    local_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY(profile_id, entity_table, wire_id)
);

CREATE TABLE IF NOT EXISTS sync_join_requests (
    id TEXT PRIMARY KEY,
    member_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    replica_id TEXT NOT NULL,
    member_name TEXT NOT NULL,
    device_label TEXT NOT NULL,
    signing_public_key_b64 TEXT NOT NULL,
    signing_private_key_b64 TEXT NOT NULL,
    recipient_public_key TEXT NOT NULL,
    age_identity TEXT NOT NULL,
    created_at TEXT NOT NULL,
    consumed_at TEXT
);

CREATE TABLE IF NOT EXISTS sync_transports (
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN ('folder', 'webdav', 's3')),
    label TEXT NOT NULL,
    config_json TEXT NOT NULL,
    credential_json TEXT NOT NULL DEFAULT '{}',
    enabled INTEGER NOT NULL DEFAULT 1,
    last_push_at TEXT,
    last_pull_at TEXT,
    last_error_at TEXT,
    last_error_code TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(profile_id, label)
);

CREATE INDEX IF NOT EXISTS idx_sync_transports_profile_enabled
    ON sync_transports(profile_id, enabled, kind);

CREATE TABLE IF NOT EXISTS sync_mailbox_heads (
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    transport_id TEXT NOT NULL REFERENCES sync_transports(id) ON DELETE CASCADE,
    replica_id TEXT NOT NULL,
    last_seq INTEGER NOT NULL,
    bundle_hash TEXT NOT NULL,
    head_hash TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    PRIMARY KEY(profile_id, transport_id, replica_id)
);

CREATE TABLE IF NOT EXISTS sync_peer_status (
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    transport_id TEXT NOT NULL REFERENCES sync_transports(id) ON DELETE CASCADE,
    replica_id TEXT NOT NULL,
    member_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    last_head_seq INTEGER NOT NULL DEFAULT 0,
    last_head_hash TEXT,
    last_seen_at TEXT,
    last_bundle_at TEXT,
    status TEXT NOT NULL DEFAULT 'never_seen',
    updated_at TEXT NOT NULL,
    PRIMARY KEY(profile_id, transport_id, replica_id)
);

CREATE TABLE IF NOT EXISTS sync_replica_acknowledgements (
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    observer_replica_id TEXT NOT NULL,
    subject_replica_id TEXT NOT NULL,
    acknowledged_seq INTEGER NOT NULL DEFAULT 0,
    observed_hlc TEXT,
    observed_at TEXT NOT NULL,
    PRIMARY KEY(profile_id, observer_replica_id, subject_replica_id)
);

CREATE TABLE IF NOT EXISTS sync_tombstone_gc_log (
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    entity_table TEXT NOT NULL,
    entity_key TEXT NOT NULL,
    delete_event_id TEXT NOT NULL,
    delete_hlc TEXT NOT NULL,
    quorum_json TEXT NOT NULL,
    horizon_days INTEGER NOT NULL,
    compacted_at TEXT NOT NULL,
    UNIQUE(profile_id, entity_table, entity_key, delete_event_id)
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
    capital_gains_type TEXT,
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

CREATE TABLE IF NOT EXISTS journal_tax_summary (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    year INTEGER NOT NULL,
    asset TEXT NOT NULL,
    transaction_type TEXT NOT NULL,
    capital_gains_type TEXT,
    quantity INTEGER NOT NULL,
    proceeds REAL NOT NULL DEFAULT 0,
    cost_basis REAL NOT NULL DEFAULT 0,
    gain_loss REAL NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS journal_account_holdings (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    account_id TEXT REFERENCES accounts(id) ON DELETE SET NULL,
    account_code TEXT,
    account_label TEXT,
    asset TEXT NOT NULL,
    quantity INTEGER NOT NULL,
    cost_basis REAL NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS journal_wallet_holdings (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    wallet_id TEXT REFERENCES wallets(id) ON DELETE CASCADE,
    wallet_label TEXT,
    account_code TEXT,
    asset TEXT NOT NULL,
    quantity INTEGER NOT NULL,
    cost_basis REAL NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);

-- Local canonical quantity state. These tables are derived independently of RP2
-- and are deliberately absent from the replication allowlist.
CREATE TABLE IF NOT EXISTS journal_quantity_postings (
    posting_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    transaction_id TEXT REFERENCES transactions(id) ON DELETE SET NULL,
    observation_hash TEXT,
    occurred_at TEXT,
    asset TEXT NOT NULL,
    location_kind TEXT NOT NULL,
    location_id TEXT NOT NULL,
    amount_msat INTEGER NOT NULL CHECK(amount_msat != 0),
    state TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY(profile_id, posting_id)
);

CREATE TABLE IF NOT EXISTS journal_quantity_issues (
    issue_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    issue_type TEXT NOT NULL,
    -- custody_candidate is accepted only so a pre-simplification derived
    -- journal can be opened and rebuilt; current production emitters use
    -- custody_suspense/conflicting and replacement removes the legacy rows.
    state TEXT NOT NULL CHECK(state IN (
        'custody_candidate', 'custody_suspense', 'conflicting'
    )),
    asset TEXT,
    amount_msat INTEGER CHECK(amount_msat IS NULL OR amount_msat > 0),
    occurred_at TEXT,
    transaction_ids_json TEXT NOT NULL DEFAULT '[]',
    reason TEXT NOT NULL,
    detail_json TEXT NOT NULL DEFAULT '{}',
    blocks_from TEXT,
    created_at TEXT NOT NULL,
    PRIMARY KEY(profile_id, issue_id)
);

CREATE TABLE IF NOT EXISTS journal_quantity_balances (
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    location_kind TEXT NOT NULL,
    location_id TEXT NOT NULL,
    asset TEXT NOT NULL,
    amount_msat INTEGER NOT NULL CHECK(amount_msat != 0),
    created_at TEXT NOT NULL,
    PRIMARY KEY(profile_id, location_kind, location_id, asset)
);

-- Canonical target-bearing custody decisions.  These rows are a derived,
-- local-only navigation index over the exact quantity arbiter: they make the
-- durable source -> destination lineage available without re-interpreting tax
-- rows or trusting presentation labels.  The observation commitments and
-- half-open slices stay stored for replacement/integrity checks, while normal
-- readers use a redacted semantic projection.
CREATE TABLE IF NOT EXISTS journal_custody_decisions (
    decision_id TEXT NOT NULL CHECK(length(decision_id) = 64),
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    source_transaction_id TEXT NOT NULL
        REFERENCES transactions(id) ON DELETE CASCADE,
    target_transaction_id TEXT NOT NULL
        REFERENCES transactions(id) ON DELETE CASCADE,
    source_observation_hash TEXT NOT NULL
        CHECK(length(source_observation_hash) = 64),
    source_start_msat INTEGER NOT NULL
        CHECK(typeof(source_start_msat) = 'integer' AND source_start_msat >= 0),
    source_end_msat INTEGER NOT NULL
        CHECK(
            typeof(source_end_msat) = 'integer'
            AND source_end_msat > source_start_msat
        ),
    target_observation_hash TEXT NOT NULL
        CHECK(length(target_observation_hash) = 64),
    target_start_msat INTEGER NOT NULL
        CHECK(typeof(target_start_msat) = 'integer' AND target_start_msat >= 0),
    target_end_msat INTEGER NOT NULL
        CHECK(
            typeof(target_end_msat) = 'integer'
            AND target_end_msat > target_start_msat
        ),
    source_wallet_id TEXT REFERENCES wallets(id) ON DELETE SET NULL,
    target_wallet_id TEXT REFERENCES wallets(id) ON DELETE SET NULL,
    source_network TEXT NOT NULL,
    target_network TEXT NOT NULL,
    source_rail TEXT NOT NULL,
    target_rail TEXT NOT NULL,
    source_asset TEXT NOT NULL,
    target_asset TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN (
        'internal_verified', 'internal_reviewed'
    )),
    basis_state TEXT NOT NULL CHECK(basis_state IN (
        'eligible', 'blocked_by_prior_custody_basis'
    )),
    basis_barrier_at TEXT,
    reason TEXT NOT NULL,
    atomic_group_id TEXT,
    component_id TEXT,
    occurred_at TEXT,
    target_occurred_at TEXT,
    created_at TEXT NOT NULL,
    CHECK(
        source_end_msat - source_start_msat =
        target_end_msat - target_start_msat
    ),
    PRIMARY KEY(profile_id, decision_id)
);

-- Stored non-quantity custody relations. Conversions and reviewed payouts are
-- economic links, not assertions that unlike native quantities are the same
-- conserved object, so they complement rather than overload MOVE decisions.
CREATE TABLE IF NOT EXISTS journal_custody_economic_relations (
    relation_id TEXT NOT NULL CHECK(length(relation_id) = 64),
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    relation_kind TEXT NOT NULL CHECK(relation_kind IN (
        'conversion', 'direct_payout'
    )),
    source_transaction_id TEXT NOT NULL
        REFERENCES transactions(id) ON DELETE CASCADE,
    target_transaction_id TEXT
        REFERENCES transactions(id) ON DELETE CASCADE,
    component_id TEXT,
    source_asset TEXT NOT NULL,
    target_asset TEXT NOT NULL,
    source_amount_msat INTEGER NOT NULL CHECK(source_amount_msat > 0),
    target_amount_msat INTEGER NOT NULL CHECK(target_amount_msat > 0),
    basis_state TEXT NOT NULL CHECK(basis_state IN (
        'eligible', 'blocked_by_prior_custody_basis'
    )),
    occurred_at TEXT,
    target_occurred_at TEXT,
    created_at TEXT NOT NULL,
    PRIMARY KEY(profile_id, relation_id)
);

-- Evidence detail is written once when a durable authored claim/component
-- explicitly binds it. Rows cannot be updated; scoped book reset/profile
-- teardown may delete them. Journal refresh never snapshots every import row.
CREATE TABLE IF NOT EXISTS custody_authored_evidence_snapshots (
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    subject_kind TEXT NOT NULL CHECK(subject_kind IN (
        'custody_component', 'custody_claim'
    )),
    subject_id TEXT NOT NULL,
    detail_hash TEXT NOT NULL,
    quantity_hash TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY(profile_id, subject_kind, subject_id, detail_hash)
);

CREATE TRIGGER IF NOT EXISTS trg_custody_authored_evidence_immutable
BEFORE UPDATE ON custody_authored_evidence_snapshots
BEGIN
    SELECT RAISE(ABORT, 'custody_authored_evidence_immutable');
END;

CREATE INDEX IF NOT EXISTS idx_journal_entries_profile_time
    ON journal_entries(profile_id, occurred_at, created_at, id);

CREATE INDEX IF NOT EXISTS idx_journal_entries_profile_type_time
    ON journal_entries(profile_id, entry_type, occurred_at, created_at, id);

CREATE INDEX IF NOT EXISTS idx_journal_entries_profile_wallet_time
    ON journal_entries(profile_id, wallet_id, occurred_at, created_at, id);

CREATE INDEX IF NOT EXISTS idx_journal_entries_profile_account_time
    ON journal_entries(profile_id, account_id, occurred_at, created_at, id);

CREATE INDEX IF NOT EXISTS idx_journal_entries_transaction
    ON journal_entries(transaction_id);

CREATE INDEX IF NOT EXISTS idx_journal_quarantines_profile
    ON journal_quarantines(profile_id, created_at);

CREATE INDEX IF NOT EXISTS idx_journal_tax_summary_profile_year
    ON journal_tax_summary(profile_id, year, asset, transaction_type, capital_gains_type);

CREATE INDEX IF NOT EXISTS idx_journal_account_holdings_profile_asset
    ON journal_account_holdings(profile_id, asset, account_code, id);

CREATE INDEX IF NOT EXISTS idx_journal_wallet_holdings_profile_asset
    ON journal_wallet_holdings(profile_id, asset, wallet_label, id);

CREATE INDEX IF NOT EXISTS idx_journal_quantity_postings_profile_time
    ON journal_quantity_postings(profile_id, occurred_at, posting_id);

CREATE INDEX IF NOT EXISTS idx_journal_quantity_issues_profile_time
    ON journal_quantity_issues(profile_id, occurred_at, issue_id);

CREATE INDEX IF NOT EXISTS idx_journal_quantity_balances_profile_asset
    ON journal_quantity_balances(profile_id, asset, location_kind, location_id);

CREATE INDEX IF NOT EXISTS idx_journal_custody_decisions_profile_time
    ON journal_custody_decisions(profile_id, occurred_at, decision_id);

CREATE INDEX IF NOT EXISTS idx_journal_custody_decisions_source
    ON journal_custody_decisions(profile_id, source_transaction_id, decision_id);

CREATE INDEX IF NOT EXISTS idx_journal_custody_decisions_target
    ON journal_custody_decisions(profile_id, target_transaction_id, decision_id);

CREATE INDEX IF NOT EXISTS idx_journal_custody_relations_profile_time
    ON journal_custody_economic_relations(
        profile_id, occurred_at, relation_id
    );

CREATE INDEX IF NOT EXISTS idx_journal_custody_relations_source
    ON journal_custody_economic_relations(
        profile_id, source_transaction_id, relation_id
    );

CREATE INDEX IF NOT EXISTS idx_journal_custody_relations_target
    ON journal_custody_economic_relations(
        profile_id, target_transaction_id, relation_id
    );

CREATE INDEX IF NOT EXISTS idx_custody_authored_evidence_subject
    ON custody_authored_evidence_snapshots(
        profile_id, subject_kind, subject_id, created_at
    );

-- Append-only marker for a report saved by Kassiber or explicitly registered
-- as saved/filed outside Kassiber.
-- The exported document itself remains in the user's chosen location; this
-- append-only row retains only its content hash and bounded accounting
-- summaries so later custody evidence can identify amendment risk without
-- turning Kassiber into a general document-versioning system.
CREATE TABLE IF NOT EXISTS filed_report_snapshots (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    report_kind TEXT NOT NULL,
    report_state TEXT NOT NULL CHECK(report_state IN ('saved', 'filed')),
    period_start_year INTEGER NOT NULL CHECK(period_start_year BETWEEN 1900 AND 9999),
    period_end_year INTEGER NOT NULL CHECK(period_end_year BETWEEN period_start_year AND 9999),
    content_sha256 TEXT NOT NULL CHECK(length(content_sha256) = 64),
    classification_summary_json TEXT NOT NULL DEFAULT '{}',
    gain_summary_json TEXT NOT NULL DEFAULT '{}',
    report_scope_json TEXT NOT NULL DEFAULT '{}',
    authored_source TEXT NOT NULL DEFAULT 'user'
        CHECK(authored_source IN ('user', 'cli', 'gui', 'ai_tool')),
    notes TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_filed_report_snapshots_period
    ON filed_report_snapshots(
        profile_id, period_start_year, period_end_year, created_at, id
    );

CREATE TRIGGER IF NOT EXISTS trg_filed_report_snapshots_immutable
BEFORE UPDATE ON filed_report_snapshots
BEGIN
    SELECT RAISE(ABORT, 'filed_report_snapshots_immutable');
END;

-- Authored review decisions for deterministic custody-gap candidates.  The
-- matcher output remains derived; each decision pins the exact candidate
-- fingerprint that was reviewed.  If imported evidence changes that
-- fingerprint, the old decision remains in history but no longer closes the
-- current candidate.
CREATE TABLE IF NOT EXISTS custody_gap_reviews (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    gap_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision >= 1),
    candidate_fingerprint TEXT NOT NULL,
    action TEXT NOT NULL CHECK(action IN ('dismissed', 'resolved')),
    event_kind TEXT NOT NULL DEFAULT 'review_decision'
        CHECK(event_kind IN (
            'review_decision', 'bridge_created', 'bridge_reopened',
            'bridge_revised', 'residual_classified'
        )),
    component_id TEXT,
    authored_source TEXT NOT NULL DEFAULT 'user'
        CHECK(authored_source IN ('user', 'cli', 'gui', 'ai_tool')),
    reason TEXT,
    snapshot_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_custody_gap_reviews_latest
    ON custody_gap_reviews(profile_id, gap_id, revision DESC);

-- The journal builder persists only the exact boundary exclusions used by its
-- arbitration run. The bounded advisory candidate population is recomputed on
-- read and never becomes a second stored custody truth.
CREATE TABLE IF NOT EXISTS journal_custody_gap_inputs (
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT PRIMARY KEY REFERENCES profiles(id) ON DELETE CASCADE,
    input_version INTEGER NOT NULL CHECK(input_version >= 0),
    ignored_ids_json TEXT NOT NULL DEFAULT '[]',
    accounting_ignored_ids_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL
);

CREATE TRIGGER IF NOT EXISTS trg_custody_gap_reviews_immutable
BEFORE UPDATE ON custody_gap_reviews
BEGIN
    SELECT RAISE(ABORT, 'custody_gap_reviews_immutable');
END;

-- Immutable completeness commitment for the normalized transaction boundary
-- authored by one custody review.  Keeping this as a child header allows an
-- upgraded peer to distinguish a complete relation set from a prefix received
-- in an earlier bundle without changing the signed shape of legacy reviews.
CREATE TABLE IF NOT EXISTS custody_gap_review_relation_sets (
    id TEXT PRIMARY KEY,
    review_id TEXT NOT NULL UNIQUE
        REFERENCES custody_gap_reviews(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    expected_source_count INTEGER NOT NULL CHECK(expected_source_count >= 0),
    expected_return_count INTEGER NOT NULL CHECK(expected_return_count >= 0),
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_custody_gap_review_relation_sets_scope
    ON custody_gap_review_relation_sets(profile_id, review_id, id);

CREATE TRIGGER IF NOT EXISTS trg_custody_gap_review_relation_set_scope_insert
BEFORE INSERT ON custody_gap_review_relation_sets
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM custody_gap_reviews r
        WHERE r.id = NEW.review_id
          AND r.workspace_id = NEW.workspace_id
          AND r.profile_id = NEW.profile_id
    ) THEN RAISE(ABORT, 'custody_gap_review_relation_set_scope_mismatch') END;
END;

CREATE TRIGGER IF NOT EXISTS trg_custody_gap_review_relation_sets_immutable
BEFORE UPDATE ON custody_gap_review_relation_sets
BEGIN
    SELECT RAISE(ABORT, 'custody_gap_review_relation_sets_immutable');
END;

-- Export-safe transaction scope for each authored custody-gap review.  The
-- review snapshot remains local/private and the candidate fingerprint is
-- intentionally one-way, so neither can answer whether a bounded auditor
-- handoff should include a componentless dismissal.  These normalized anchors
-- contain only authored row identities and survive later transaction deletion.
CREATE TABLE IF NOT EXISTS custody_gap_review_transactions (
    id TEXT PRIMARY KEY,
    review_id TEXT NOT NULL REFERENCES custody_gap_reviews(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('source', 'return')),
    transaction_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (review_id, role, transaction_id)
);

CREATE INDEX IF NOT EXISTS idx_custody_gap_review_transactions_review
    ON custody_gap_review_transactions(review_id, role, transaction_id, id);

CREATE INDEX IF NOT EXISTS idx_custody_gap_review_transactions_scope
    ON custody_gap_review_transactions(profile_id, transaction_id, review_id);

CREATE TRIGGER IF NOT EXISTS trg_custody_gap_review_transaction_scope_insert
BEFORE INSERT ON custody_gap_review_transactions
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM custody_gap_reviews r
        WHERE r.id = NEW.review_id
          AND r.workspace_id = NEW.workspace_id
          AND r.profile_id = NEW.profile_id
    ) THEN RAISE(ABORT, 'custody_gap_review_transaction_review_scope_mismatch') END;
    -- A durable review anchor must survive source retraction and must remain
    -- importable into a fresh replica after that retraction. Reject only a
    -- colliding live transaction from another book; absence is intentional.
    SELECT CASE WHEN EXISTS (
        SELECT 1 FROM transactions t WHERE t.id = NEW.transaction_id
    ) AND NOT EXISTS (
        SELECT 1 FROM transactions t
        WHERE t.id = NEW.transaction_id
          AND t.workspace_id = NEW.workspace_id
          AND t.profile_id = NEW.profile_id
    ) THEN RAISE(ABORT, 'custody_gap_review_transaction_scope_mismatch') END;
END;

CREATE TRIGGER IF NOT EXISTS trg_custody_gap_review_transactions_immutable
BEFORE UPDATE ON custody_gap_review_transactions
BEGIN
    SELECT RAISE(ABORT, 'custody_gap_review_transactions_immutable');
END;

-- Local-only, append-only evidence that a custody write proposed by the AI
-- crossed an explicit consent boundary.  Chat history is optional and cannot
-- serve as this audit trail.  Proposal payloads remain inside SQLCipher and
-- are deliberately absent from replication and public/audit-package exports.
CREATE TABLE IF NOT EXISTS custody_ai_assistance_audits (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    tool_name TEXT NOT NULL,
    daemon_kind TEXT NOT NULL,
    call_id TEXT NOT NULL,
    provider_kind TEXT NOT NULL,
    model TEXT NOT NULL,
    gap_id TEXT,
    candidate_fingerprint TEXT,
    facts_sha256 TEXT NOT NULL CHECK(length(facts_sha256) = 64),
    model_proposal_json TEXT NOT NULL DEFAULT '{}',
    final_proposal_json TEXT NOT NULL DEFAULT '{}',
    user_edited INTEGER NOT NULL DEFAULT 0 CHECK(user_edited IN (0, 1)),
    consent_decision TEXT NOT NULL CHECK(consent_decision IN (
        'allow_once', 'allow_session', 'deny', 'consent_timeout', 'cancelled'
    )),
    consent_requested_at TEXT NOT NULL,
    consent_decided_at TEXT NOT NULL,
    execution_status TEXT NOT NULL CHECK(execution_status IN (
        'executed', 'failed', 'denied', 'cancelled'
    )),
    execution_code TEXT,
    result_sha256 TEXT CHECK(result_sha256 IS NULL OR length(result_sha256) = 64),
    review_id TEXT,
    component_id TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_custody_ai_assistance_profile_time
    ON custody_ai_assistance_audits(profile_id, created_at DESC, id DESC);

CREATE TRIGGER IF NOT EXISTS trg_custody_ai_assistance_immutable
BEFORE UPDATE ON custody_ai_assistance_audits
BEGIN
    SELECT RAISE(ABORT, 'custody_ai_assistance_immutable');
END;

-- Durable activation audit history tying an authored custody review to every
-- overlapping saved/filed report. The row is sealed by the confirmed review
-- and replicates with that authored decision; it is not a mutable projection
-- of current journal state.
CREATE TABLE IF NOT EXISTS custody_filed_report_impacts (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    filed_report_snapshot_id TEXT NOT NULL
        REFERENCES filed_report_snapshots(id) ON DELETE CASCADE,
    component_id TEXT NOT NULL,
    review_id TEXT NOT NULL,
    gap_id TEXT NOT NULL,
    affected_period_start_year INTEGER NOT NULL
        CHECK(affected_period_start_year BETWEEN 1900 AND 9999),
    affected_period_end_year INTEGER NOT NULL
        CHECK(affected_period_end_year BETWEEN affected_period_start_year AND 9999),
    before_classification_summary_json TEXT NOT NULL DEFAULT '{}',
    after_classification_summary_json TEXT NOT NULL DEFAULT '{}',
    before_gain_summary_json TEXT NOT NULL DEFAULT '{}',
    after_gain_summary_json TEXT NOT NULL DEFAULT '{}',
    amendment_warning TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(filed_report_snapshot_id, review_id)
);

CREATE INDEX IF NOT EXISTS idx_custody_filed_report_impacts_profile
    ON custody_filed_report_impacts(profile_id, created_at, id);

CREATE TRIGGER IF NOT EXISTS trg_custody_filed_report_impacts_immutable
BEFORE UPDATE ON custody_filed_report_impacts
BEGIN
    SELECT RAISE(ABORT, 'custody_filed_report_impacts_immutable');
END;

-- One immutable post-rebuild closure for an activation-time filed-report
-- impact. The original impact deliberately remains pending; this child row
-- records the exact finalized journal totals and the honest amendment state.
CREATE TABLE IF NOT EXISTS custody_filed_report_impact_resolutions (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    impact_id TEXT NOT NULL UNIQUE
        REFERENCES custody_filed_report_impacts(id) ON DELETE CASCADE,
    rebuilt_at TEXT NOT NULL,
    after_classification_summary_json TEXT NOT NULL DEFAULT '{}',
    after_gain_summary_json TEXT NOT NULL DEFAULT '{}',
    classification_changed INTEGER NOT NULL CHECK(classification_changed IN (0, 1)),
    gain_changed INTEGER NOT NULL CHECK(gain_changed IN (0, 1)),
    amendment_status TEXT NOT NULL CHECK(amendment_status IN (
        'no_change', 'saved_report_changed', 'review_required'
    )),
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_custody_filed_report_impact_resolutions_profile
    ON custody_filed_report_impact_resolutions(profile_id, rebuilt_at, id);

CREATE TRIGGER IF NOT EXISTS trg_custody_filed_report_impact_resolutions_immutable
BEFORE UPDATE ON custody_filed_report_impact_resolutions
BEGIN
    SELECT RAISE(ABORT, 'custody_filed_report_impact_resolutions_immutable');
END;

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
    out_amount INTEGER,
    component_id TEXT REFERENCES custody_components(id) ON DELETE SET NULL,
    deleted_at TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS direct_swap_payouts (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    out_transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    kind TEXT NOT NULL DEFAULT 'direct-swap-payout',
    policy TEXT NOT NULL DEFAULT 'carrying-value',
    payout_asset TEXT NOT NULL,
    payout_amount INTEGER NOT NULL,
    payout_occurred_at TEXT,
    payout_fiat_value REAL,
    payout_external_id TEXT,
    counterparty TEXT,
    notes TEXT,
    swap_fee_msat INTEGER,
    swap_fee_kind TEXT,
    out_amount INTEGER,
    component_id TEXT REFERENCES custody_components(id) ON DELETE SET NULL,
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

CREATE UNIQUE INDEX IF NOT EXISTS idx_direct_swap_payouts_active_out
    ON direct_swap_payouts(profile_id, out_transaction_id) WHERE deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_direct_swap_payouts_profile_active
    ON direct_swap_payouts(profile_id) WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS custody_authored_migration_issues (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    legacy_table TEXT NOT NULL
        CHECK(legacy_table IN ('transaction_pairs', 'direct_swap_payouts')),
    legacy_source_id TEXT NOT NULL,
    issue_code TEXT NOT NULL,
    transaction_ids_json TEXT NOT NULL DEFAULT '[]',
    details_json TEXT NOT NULL DEFAULT '{}',
    resolved_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(legacy_table, legacy_source_id)
);

CREATE INDEX IF NOT EXISTS idx_custody_authored_migration_issues_profile_open
    ON custody_authored_migration_issues(profile_id, created_at, id)
    WHERE resolved_at IS NULL;

-- A loan mark on a single transaction. Collateral lock/release roles suppress
-- the outbound/inbound collateral events because the coins never left the owned
-- pool. Principal received/repaid roles suppress borrowed-principal movements
-- because they are liability principal, not acquisition/disposal of owned lots.
-- The tax engine reads (transaction_id, role) to suppress those events.
-- Removing the mark reverts the transaction to its normal classification.
CREATE TABLE IF NOT EXISTS loan_legs (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    loan_id TEXT,
    role TEXT NOT NULL,
    note TEXT,
    deleted_at TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_loan_legs_profile_active
    ON loan_legs(profile_id) WHERE deleted_at IS NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_loan_legs_active_transaction
    ON loan_legs(profile_id, transaction_id)
    WHERE deleted_at IS NULL;

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

CREATE TABLE IF NOT EXISTS ai_chat_sessions (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    provider TEXT,
    model TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_ai_chat_sessions_profile_updated
    ON ai_chat_sessions(profile_id, updated_at);

CREATE TABLE IF NOT EXISTS ai_chat_messages (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES ai_chat_sessions(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    tool_calls_json TEXT,
    provenance_json TEXT,
    finish_reason TEXT,
    created_at TEXT NOT NULL,
    UNIQUE (session_id, seq)
);

CREATE INDEX IF NOT EXISTS idx_ai_chat_messages_session
    ON ai_chat_messages(session_id, seq);

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

CREATE TABLE IF NOT EXISTS lightning_node_syncs (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    backend_name TEXT NOT NULL,
    node_id TEXT,
    node_alias TEXT,
    started_at TEXT NOT NULL,
    completed_at TEXT,
    status TEXT NOT NULL,
    fetched_counts_json TEXT NOT NULL DEFAULT '{}',
    error_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_lightning_node_syncs_wallet_started
    ON lightning_node_syncs(wallet_id, started_at DESC);

CREATE TABLE IF NOT EXISTS lightning_node_records (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    backend_name TEXT NOT NULL,
    node_id TEXT,
    record_type TEXT NOT NULL,
    external_id TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    account TEXT,
    peer_id TEXT,
    channel_id TEXT,
    direction TEXT,
    amount_msat INTEGER NOT NULL DEFAULT 0,
    fee_msat INTEGER NOT NULL DEFAULT 0,
    tag TEXT,
    status TEXT,
    currency TEXT,
    payment_hash TEXT,
    txid TEXT,
    outpoint TEXT,
    sync_id TEXT REFERENCES lightning_node_syncs(id) ON DELETE SET NULL,
    raw_json TEXT NOT NULL DEFAULT '{}',
    first_seen_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(profile_id, wallet_id, backend_name, record_type, external_id)
);

CREATE INDEX IF NOT EXISTS idx_lightning_node_records_profile_type_time
    ON lightning_node_records(profile_id, record_type, occurred_at DESC);

CREATE INDEX IF NOT EXISTS idx_lightning_node_records_wallet_type_time
    ON lightning_node_records(wallet_id, record_type, occurred_at DESC);

CREATE TABLE IF NOT EXISTS freshness_source_states (
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    source_key TEXT NOT NULL,
    source_type TEXT NOT NULL,
    source_label TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'fresh',
    state TEXT NOT NULL DEFAULT 'fresh',
    stale_reason TEXT,
    blocking_reports INTEGER NOT NULL DEFAULT 0,
    paused INTEGER NOT NULL DEFAULT 0,
    rate_limited_until TEXT,
    cooldown_reason TEXT,
    retry_count INTEGER NOT NULL DEFAULT 0,
    last_success_at TEXT,
    last_error_at TEXT,
    last_error_code TEXT,
    last_error_message TEXT,
    last_phase TEXT,
    progress_json TEXT NOT NULL DEFAULT '{}',
    checkpoint_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL,
    PRIMARY KEY(profile_id, source_key)
);

CREATE INDEX IF NOT EXISTS idx_freshness_source_states_profile_status
    ON freshness_source_states(profile_id, status, updated_at DESC);

CREATE TABLE IF NOT EXISTS freshness_jobs (
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    job_type TEXT NOT NULL,
    source_key TEXT NOT NULL,
    source_type TEXT NOT NULL,
    source_label TEXT NOT NULL,
    status TEXT NOT NULL,
    phase TEXT,
    priority INTEGER NOT NULL DEFAULT 100,
    payload_json TEXT NOT NULL DEFAULT '{}',
    progress_json TEXT NOT NULL DEFAULT '{}',
    result_json TEXT NOT NULL DEFAULT '{}',
    error_json TEXT NOT NULL DEFAULT '{}',
    attempts INTEGER NOT NULL DEFAULT 0,
    cancel_requested INTEGER NOT NULL DEFAULT 0,
    run_after TEXT,
    cooldown_until TEXT,
    created_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_freshness_jobs_profile_status
    ON freshness_jobs(profile_id, status, priority, created_at);

CREATE UNIQUE INDEX IF NOT EXISTS idx_freshness_jobs_singleflight
    ON freshness_jobs(profile_id, source_key, job_type)
    WHERE status IN ('queued', 'running', 'rate_limited');

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
    label TEXT,
    original_filename TEXT,
    stored_relpath TEXT,
    source_url TEXT,
    media_type TEXT,
    size_bytes INTEGER,
    sha256 TEXT,
    copied_from_attachment_id TEXT,
    copied_from_transaction_id TEXT,
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
    payment_request_id TEXT,
    origin_kind TEXT,
    origin_app_id TEXT,
    origin_label TEXT,
    origin_url TEXT,
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

CREATE TABLE IF NOT EXISTS btcpay_account_routes (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    backend_name TEXT NOT NULL,
    store_id TEXT NOT NULL,
    payment_method_id TEXT NOT NULL,
    action TEXT NOT NULL,
    label TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(profile_id, backend_name, store_id, payment_method_id, action)
);

CREATE INDEX IF NOT EXISTS idx_btcpay_account_routes_profile_backend
    ON btcpay_account_routes(profile_id, backend_name);

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
    display_name TEXT,
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


# Custody components are the authored, atomic interpretation layer above raw
# imported transactions.  The schema intentionally uses open TEXT identifiers
# for rails, chains, networks, assets, exposures and conservation units: adding
# another Bitcoin layer must not require a table rebuild.  Roles and lifecycle
# states are closed because they carry conservation/activation semantics.
#
# ``evidence_json``, ``conversion_metadata_json`` and leg ``location_ref`` are
# local-only detail.  The replication allowlist projects privacy-safe summary
# fields and transaction/wallet anchors, never these arbitrary JSON/ref values.
CUSTODY_COMPONENT_SCHEMA = """
CREATE TABLE IF NOT EXISTS custody_components (
    id TEXT PRIMARY KEY,
    lineage_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    component_type TEXT NOT NULL,
    conservation_mode TEXT NOT NULL DEFAULT 'quantity'
        CHECK (conservation_mode IN ('quantity', 'conversion')),
    state TEXT NOT NULL DEFAULT 'draft'
        CHECK (state IN ('draft', 'active', 'superseded')),
    evidence_kind TEXT,
    evidence_grade TEXT,
    evidence_json TEXT NOT NULL DEFAULT '{}',
    conversion_policy TEXT,
    conversion_reviewed INTEGER NOT NULL DEFAULT 0
        CHECK (conversion_reviewed IN (0, 1)),
    conversion_metadata_json TEXT NOT NULL DEFAULT '{}',
    expected_leg_count INTEGER CHECK (expected_leg_count >= 0),
    expected_allocation_count INTEGER CHECK (expected_allocation_count >= 0),
    expected_economic_term_count INTEGER
        CHECK (expected_economic_term_count >= 0),
    expected_evidence_count INTEGER CHECK (expected_evidence_count >= 0),
    authored_source TEXT DEFAULT 'user',
    notes TEXT,
    change_reason TEXT,
    -- Revision links are authored identifiers, not immediate relational
    -- dependencies.  A sync snapshot can replay two mutually-linked headers
    -- in either order, and concurrent replicas can legitimately retain two
    -- competing revisions until review.  Application validation checks the
    -- links after replay; an immediate self-FK would reject valid evidence.
    supersedes_component_id TEXT,
    superseded_by_component_id TEXT,
    activated_at TEXT,
    superseded_at TEXT,
    created_at TEXT NOT NULL
);

-- These are lookup indexes, deliberately not uniqueness constraints.  Local
-- mutation APIs still serialize revisions, while replication must preserve
-- concurrent drafts/actives so the conflict is visible and reviewable.
CREATE INDEX IF NOT EXISTS idx_custody_components_lineage_active
    ON custody_components(profile_id, lineage_id) WHERE state = 'active';

CREATE INDEX IF NOT EXISTS idx_custody_components_lineage_draft
    ON custody_components(profile_id, lineage_id) WHERE state = 'draft';

CREATE INDEX IF NOT EXISTS idx_custody_components_lineage_revision
    ON custody_components(profile_id, lineage_id, revision, id);

CREATE INDEX IF NOT EXISTS idx_custody_components_profile_state
    ON custody_components(profile_id, state, created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_custody_components_supersedes
    ON custody_components(supersedes_component_id)
    WHERE supersedes_component_id IS NOT NULL;

-- Typed accounting/tax terms that are not physical quantity-leg facts. These
-- immutable rows let reviewed pair and direct-payout meaning move into the
-- component aggregate without hiding policy or fee semantics in evidence JSON.
CREATE TABLE IF NOT EXISTS custody_component_economic_terms (
    id TEXT PRIMARY KEY,
    component_id TEXT NOT NULL
        REFERENCES custody_components(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
    source_leg_id TEXT NOT NULL
        REFERENCES custody_component_legs(id) ON DELETE CASCADE,
    target_leg_id TEXT NOT NULL
        REFERENCES custody_component_legs(id) ON DELETE CASCADE,
    term_kind TEXT NOT NULL
        CHECK(term_kind IN ('transaction_pair', 'direct_swap_payout')),
    legacy_source_id TEXT NOT NULL,
    source_row_hash TEXT NOT NULL CHECK(length(source_row_hash) = 64),
    review_kind TEXT NOT NULL,
    tax_policy TEXT NOT NULL,
    reviewed_source_amount_msat INTEGER
        CHECK(reviewed_source_amount_msat IS NULL OR
              typeof(reviewed_source_amount_msat) = 'integer'),
    swap_fee_msat INTEGER,
    swap_fee_kind TEXT,
    confidence_at_review TEXT,
    review_source TEXT,
    review_notes TEXT,
    payout_asset TEXT,
    payout_amount_msat INTEGER
        CHECK(payout_amount_msat IS NULL OR
              typeof(payout_amount_msat) = 'integer'),
    payout_occurred_at TEXT,
    payout_fiat_value_exact TEXT,
    payout_external_id TEXT,
    counterparty TEXT,
    created_at TEXT NOT NULL,
    UNIQUE(component_id, ordinal),
    UNIQUE(component_id, term_kind, legacy_source_id)
);

CREATE INDEX IF NOT EXISTS idx_custody_component_terms_profile_kind
    ON custody_component_economic_terms(profile_id, term_kind, created_at, component_id);

CREATE INDEX IF NOT EXISTS idx_custody_component_terms_component
    ON custody_component_economic_terms(component_id, ordinal, id);

CREATE INDEX IF NOT EXISTS idx_custody_component_terms_legacy_source
    ON custody_component_economic_terms(profile_id, term_kind, legacy_source_id,
                                        created_at, component_id);

CREATE TRIGGER IF NOT EXISTS trg_custody_component_terms_scope_insert
BEFORE INSERT ON custody_component_economic_terms
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1
        FROM custody_components c
        JOIN custody_component_legs source ON source.id = NEW.source_leg_id
        JOIN custody_component_legs target ON target.id = NEW.target_leg_id
        WHERE c.id = NEW.component_id
          AND c.workspace_id = NEW.workspace_id
          AND c.profile_id = NEW.profile_id
          AND source.component_id = c.id
          AND target.component_id = c.id
          AND source.role = 'source'
          AND target.role != 'source'
    ) THEN RAISE(ABORT, 'custody_component_terms_scope_mismatch') END;
END;

CREATE TRIGGER IF NOT EXISTS trg_custody_component_terms_immutable
BEFORE UPDATE ON custody_component_economic_terms
BEGIN
    SELECT RAISE(ABORT, 'custody_component_terms_immutable');
END;

CREATE TRIGGER IF NOT EXISTS trg_custody_component_terms_delete_immutable
BEFORE DELETE ON custody_component_economic_terms
WHEN EXISTS (SELECT 1 FROM profiles p WHERE p.id = OLD.profile_id)
AND NOT EXISTS (
    SELECT 1 FROM custody_component_purge_authorizations authorization
    WHERE authorization.profile_id = OLD.profile_id
)
BEGIN
    SELECT RAISE(ABORT, 'custody_component_terms_delete_immutable');
END;

-- Author-bound, payload-free commitments to the canonical evidence visible at
-- activation.  Raw evidence payloads remain in the local-only
-- custody_authored_evidence_snapshots table; these rows are safe to replicate.
-- The deterministic id is a hash of (component_id, ordinal), so two authors
-- cannot silently publish different evidence into the same ordinal.
CREATE TABLE IF NOT EXISTS custody_component_evidence_commitments (
    id TEXT PRIMARY KEY,
    component_id TEXT NOT NULL REFERENCES custody_components(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    quantity_hash TEXT NOT NULL CHECK (length(quantity_hash) = 64),
    detail_hash TEXT NOT NULL CHECK (length(detail_hash) = 64),
    created_at TEXT NOT NULL,
    UNIQUE (component_id, ordinal)
);

CREATE INDEX IF NOT EXISTS idx_custody_evidence_commitments_component
    ON custody_component_evidence_commitments(component_id, ordinal, id);

CREATE TRIGGER IF NOT EXISTS trg_custody_evidence_commitment_scope_insert
BEFORE INSERT ON custody_component_evidence_commitments
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM custody_components c
        WHERE c.id = NEW.component_id
          AND c.workspace_id = NEW.workspace_id
          AND c.profile_id = NEW.profile_id
    ) THEN RAISE(ABORT, 'custody_evidence_commitment_scope_mismatch') END;
END;

-- A book reset preserves the profile row, so delete-immutability cannot use
-- profile absence as its authorization signal. This local-only guard is
-- populated and cleared inside the reset transaction; it is not replicated.
CREATE TABLE IF NOT EXISTS custody_component_purge_authorizations (
    profile_id TEXT PRIMARY KEY REFERENCES profiles(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS custody_component_legs (
    id TEXT PRIMARY KEY,
    component_id TEXT NOT NULL REFERENCES custody_components(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    role TEXT NOT NULL
        CHECK (role IN ('source', 'destination', 'fee', 'external',
                        'retained', 'unresolved', 'suspense')),
    rail TEXT NOT NULL,
    chain TEXT,
    network TEXT,
    asset TEXT NOT NULL,
    exposure TEXT NOT NULL,
    conservation_unit TEXT NOT NULL,
    amount_msat INTEGER NOT NULL
        CHECK (typeof(amount_msat) = 'integer' AND amount_msat >= 0),
    valuation_unit TEXT,
    valuation_amount INTEGER
        CHECK (valuation_amount IS NULL OR
               (typeof(valuation_amount) = 'integer' AND valuation_amount >= 0)),
    occurred_at TEXT,
    transaction_id TEXT REFERENCES transactions(id) ON DELETE SET NULL,
    -- Durable evidence identity. ``transaction_id`` is a live FK and becomes
    -- NULL when an importer retracts a row; this copy must survive so the
    -- component becomes invalid instead of masquerading as a transactionless
    -- manual leg.
    anchor_transaction_id TEXT,
    wallet_id TEXT REFERENCES wallets(id) ON DELETE SET NULL,
    location_ref TEXT,
    notes TEXT,
    created_at TEXT NOT NULL,
    UNIQUE (component_id, ordinal)
);

CREATE INDEX IF NOT EXISTS idx_custody_component_legs_component
    ON custody_component_legs(component_id, ordinal, id);

CREATE INDEX IF NOT EXISTS idx_custody_component_legs_profile_transaction
    ON custody_component_legs(profile_id, transaction_id)
    WHERE transaction_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_custody_component_legs_profile_wallet
    ON custody_component_legs(profile_id, wallet_id)
    WHERE wallet_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS custody_component_allocations (
    id TEXT PRIMARY KEY,
    component_id TEXT NOT NULL REFERENCES custody_components(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    source_leg_id TEXT NOT NULL REFERENCES custody_component_legs(id) ON DELETE CASCADE,
    sink_leg_id TEXT NOT NULL REFERENCES custody_component_legs(id) ON DELETE CASCADE,
    source_amount_msat INTEGER NOT NULL
        CHECK (typeof(source_amount_msat) = 'integer' AND source_amount_msat >= 0),
    sink_amount_msat INTEGER NOT NULL
        CHECK (typeof(sink_amount_msat) = 'integer' AND sink_amount_msat >= 0),
    created_at TEXT NOT NULL,
    UNIQUE (component_id, ordinal),
    UNIQUE (component_id, source_leg_id, sink_leg_id)
);

CREATE INDEX IF NOT EXISTS idx_custody_allocations_component
    ON custody_component_allocations(component_id, ordinal, id);

CREATE INDEX IF NOT EXISTS idx_custody_allocations_source
    ON custody_component_allocations(source_leg_id);

CREATE INDEX IF NOT EXISTS idx_custody_allocations_sink
    ON custody_component_allocations(sink_leg_id);

CREATE TRIGGER IF NOT EXISTS trg_custody_allocation_scope_insert
BEFORE INSERT ON custody_component_allocations
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1
        FROM custody_components c
        JOIN custody_component_legs source ON source.id = NEW.source_leg_id
        JOIN custody_component_legs sink ON sink.id = NEW.sink_leg_id
        WHERE c.id = NEW.component_id
          AND c.workspace_id = NEW.workspace_id
          AND c.profile_id = NEW.profile_id
          AND source.component_id = c.id
          AND sink.component_id = c.id
          AND source.role = 'source'
          AND sink.role != 'source'
    ) THEN RAISE(ABORT, 'custody_allocation_scope_or_role_mismatch') END;
END;

CREATE TRIGGER IF NOT EXISTS trg_custody_allocation_scope_update
BEFORE UPDATE OF component_id, workspace_id, profile_id, source_leg_id, sink_leg_id
ON custody_component_allocations
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1
        FROM custody_components c
        JOIN custody_component_legs source ON source.id = NEW.source_leg_id
        JOIN custody_component_legs sink ON sink.id = NEW.sink_leg_id
        WHERE c.id = NEW.component_id
          AND c.workspace_id = NEW.workspace_id
          AND c.profile_id = NEW.profile_id
          AND source.component_id = c.id
          AND sink.component_id = c.id
          AND source.role = 'source'
          AND sink.role != 'source'
    ) THEN RAISE(ABORT, 'custody_allocation_scope_or_role_mismatch') END;
END;

-- Derived local guard used by atomic activation.  Multiple legs in one
-- component may anchor the same transaction (for example principal + fee),
-- while one transaction can belong to at most one effective active component.
-- This table is rebuilt/validated from components and is never replicated.
CREATE TABLE IF NOT EXISTS custody_component_transaction_memberships (
    component_id TEXT NOT NULL REFERENCES custody_components(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY (component_id, transaction_id),
    UNIQUE (profile_id, transaction_id)
);

CREATE INDEX IF NOT EXISTS idx_custody_memberships_component
    ON custody_component_transaction_memberships(component_id);

CREATE TRIGGER IF NOT EXISTS trg_custody_component_scope_insert
BEFORE INSERT ON custody_component_legs
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM custody_components c
        WHERE c.id = NEW.component_id
          AND c.workspace_id = NEW.workspace_id
          AND c.profile_id = NEW.profile_id
    ) THEN RAISE(ABORT, 'custody_leg_component_scope_mismatch') END;
    SELECT CASE WHEN NEW.transaction_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM transactions t
        WHERE t.id = NEW.transaction_id
          AND t.workspace_id = NEW.workspace_id
          AND t.profile_id = NEW.profile_id
    ) THEN RAISE(ABORT, 'custody_leg_transaction_scope_mismatch') END;
    SELECT CASE WHEN NEW.wallet_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM wallets w
        WHERE w.id = NEW.wallet_id
          AND w.workspace_id = NEW.workspace_id
          AND w.profile_id = NEW.profile_id
    ) THEN RAISE(ABORT, 'custody_leg_wallet_scope_mismatch') END;
END;

CREATE TRIGGER IF NOT EXISTS trg_custody_component_scope_update
BEFORE UPDATE OF component_id, workspace_id, profile_id, transaction_id, wallet_id
ON custody_component_legs
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM custody_components c
        WHERE c.id = NEW.component_id
          AND c.workspace_id = NEW.workspace_id
          AND c.profile_id = NEW.profile_id
    ) THEN RAISE(ABORT, 'custody_leg_component_scope_mismatch') END;
    SELECT CASE WHEN NEW.transaction_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM transactions t
        WHERE t.id = NEW.transaction_id
          AND t.workspace_id = NEW.workspace_id
          AND t.profile_id = NEW.profile_id
    ) THEN RAISE(ABORT, 'custody_leg_transaction_scope_mismatch') END;
    SELECT CASE WHEN NEW.wallet_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM wallets w
        WHERE w.id = NEW.wallet_id
          AND w.workspace_id = NEW.workspace_id
          AND w.profile_id = NEW.profile_id
    ) THEN RAISE(ABORT, 'custody_leg_wallet_scope_mismatch') END;
END;

CREATE TRIGGER IF NOT EXISTS trg_custody_component_memberships_supersede
AFTER UPDATE OF state ON custody_components
WHEN OLD.state = 'active' AND NEW.state != 'active'
BEGIN
    DELETE FROM custody_component_transaction_memberships
    WHERE component_id = NEW.id;
END;

CREATE TRIGGER IF NOT EXISTS trg_custody_component_memberships_leg_delete
AFTER DELETE ON custody_component_legs
WHEN OLD.transaction_id IS NOT NULL
BEGIN
    DELETE FROM custody_component_transaction_memberships
    WHERE component_id = OLD.component_id
      AND transaction_id = OLD.transaction_id
      AND NOT EXISTS (
          SELECT 1 FROM custody_component_legs l
          WHERE l.component_id = OLD.component_id
            AND l.transaction_id = OLD.transaction_id
      );
END;

CREATE INDEX IF NOT EXISTS idx_transaction_pairs_component
    ON transaction_pairs(component_id) WHERE component_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_direct_swap_payouts_component
    ON direct_swap_payouts(component_id) WHERE component_id IS NOT NULL;
"""

SCHEMA += CUSTODY_COMPONENT_SCHEMA


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


def load_managed_settings(data_root):
    """Return the managed JSON settings object, or an empty object if unreadable."""

    settings_path = resolve_settings_path(data_root)
    try:
        loaded = json.loads(settings_path.read_text(encoding="utf-8"))
    except (OSError, ValueError, json.JSONDecodeError):
        return {}
    return loaded if isinstance(loaded, dict) else {}


@contextmanager
def _managed_settings_lock(settings_path):
    """Serialize settings read-modify-write cycles across CLI processes."""

    lock_path = settings_path.with_name(f"{settings_path.name}.lock")
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    with lock_path.open("a+b") as lock_file:
        if os.name == "nt":
            import msvcrt

            lock_file.seek(0, os.SEEK_END)
            if lock_file.tell() == 0:
                lock_file.write(b"\0")
                lock_file.flush()
            lock_file.seek(0)
            msvcrt.locking(lock_file.fileno(), msvcrt.LK_LOCK, 1)
            try:
                yield
            finally:
                lock_file.seek(0)
                msvcrt.locking(lock_file.fileno(), msvcrt.LK_UNLCK, 1)
        else:
            import fcntl

            fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX)
            try:
                yield
            finally:
                fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)


def _read_managed_settings_path(settings_path):
    try:
        loaded = json.loads(settings_path.read_text(encoding="utf-8"))
    except (OSError, ValueError, json.JSONDecodeError):
        return {}
    return loaded if isinstance(loaded, dict) else {}


def _atomic_write_managed_settings(settings_path, payload):
    """Replace settings.json from the same directory after durable flush."""

    fd, temporary_name = tempfile.mkstemp(
        prefix=f".{settings_path.name}.",
        suffix=".tmp",
        dir=settings_path.parent,
    )
    temporary_path = Path(temporary_name)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(payload, handle, indent=2, sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary_path, settings_path)
    finally:
        with suppress(FileNotFoundError):
            temporary_path.unlink()


def update_managed_settings(data_root, *, updates=None, remove=()):
    """Update top-level non-secret settings while preserving the path manifest."""

    settings_path = resolve_settings_path(data_root)
    settings_path.parent.mkdir(parents=True, exist_ok=True)
    with _managed_settings_lock(settings_path):
        payload = _read_managed_settings_path(settings_path)
        for key, value in (updates or {}).items():
            payload[str(key)] = value
        for key in remove:
            payload.pop(str(key), None)
        _atomic_write_managed_settings(settings_path, payload)
    return settings_path


def mutate_managed_settings(data_root, mutator):
    """Atomically replace managed settings using a lock-held transformation."""

    settings_path = resolve_settings_path(data_root)
    settings_path.parent.mkdir(parents=True, exist_ok=True)
    with _managed_settings_lock(settings_path):
        payload = _read_managed_settings_path(settings_path)
        updated = mutator(dict(payload))
        if not isinstance(updated, dict):
            raise TypeError("managed settings mutator must return a dictionary")
        if updated != payload:
            _atomic_write_managed_settings(settings_path, updated)
    return settings_path


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
    settings_path.parent.mkdir(parents=True, exist_ok=True)
    with _managed_settings_lock(settings_path):
        existing = _read_managed_settings_path(settings_path)
        merged = dict(existing)
        merged["schema_version"] = payload["schema_version"]
        merged["app"] = payload["app"]
        existing_paths = existing.get("paths")
        merged_paths = dict(existing_paths) if isinstance(existing_paths, dict) else {}
        merged_paths.update(payload["paths"])
        merged["paths"] = merged_paths
        if merged != existing:
            _atomic_write_managed_settings(settings_path, merged)
    return settings_path


def resolve_database_path(data_root):
    """Pick `kassiber.sqlite3`, falling back to legacy `satbooks.sqlite3`."""
    root = Path(data_root).expanduser()
    current = root / DEFAULT_DB_FILENAME
    legacy = root / LEGACY_DB_FILENAME
    if current.exists() or not legacy.exists():
        return current
    return legacy


def validate_project_database_file(database):
    """Return file metadata after rejecting ambiguous project database aliases."""

    database = Path(database)
    try:
        info = database.stat()
    except FileNotFoundError:
        return None
    if not stat.S_ISREG(info.st_mode):
        raise AppError(
            "the project database is not a regular file",
            code="unsafe_project_database",
            retryable=False,
        )
    if info.st_nlink != 1:
        raise AppError(
            "the project database has multiple filesystem links",
            code="unsafe_project_database",
            hint=(
                "Remove every hard-link alias except the intended project database "
                "path before unlocking or changing its unlock policy."
            ),
            details={"link_count": int(info.st_nlink)},
            retryable=False,
        )
    return info


def resolve_canonical_project_data_root(data_root):
    """Return the symlink-resolved directory containing a safe project database."""

    effective = resolve_effective_data_root(data_root)
    database = resolve_database_path(effective).expanduser().resolve(strict=False)
    validate_project_database_file(database)
    return database.parent


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


def _configure_connection_pragmas(conn, *, encrypted=False):
    """Apply connection settings used by daemon foreground/background writers.

    `encrypted` skips `mmap_size` on SQLCipher connections: memory-mapping must
    only ever expose ciphertext pages, so we keep the encrypted store off the
    mmap path entirely rather than rely on the codec to intercept it.
    """
    conn.execute(f"PRAGMA busy_timeout = {DB_BUSY_TIMEOUT_MS}")
    conn.execute(f"PRAGMA journal_mode = {DB_JOURNAL_MODE}")
    conn.execute(f"PRAGMA synchronous = {DB_SYNCHRONOUS}")
    conn.execute(f"PRAGMA temp_store = {DB_TEMP_STORE}")
    conn.execute(f"PRAGMA cache_size = {DB_CACHE_SIZE_KIB}")
    if not encrypted:
        conn.execute(f"PRAGMA mmap_size = {DB_MMAP_SIZE_BYTES}")
    conn.execute("PRAGMA foreign_keys = ON")


def _preflight_schema_index_columns(conn):
    """Add legacy-missing columns referenced by indexes in ``SCHEMA``."""
    columns = {
        row["name"]
        for row in conn.execute("PRAGMA table_info(journal_tax_summary)").fetchall()
    }
    if columns and "capital_gains_type" not in columns:
        conn.execute("ALTER TABLE journal_tax_summary ADD COLUMN capital_gains_type TEXT")
    # The custody schema is appended to SCHEMA and creates compatibility
    # indexes over these columns.  Add them before executescript reaches those
    # indexes when opening a pre-component database.
    for table in ("transaction_pairs", "direct_swap_payouts"):
        table_columns = {
            row["name"] for row in conn.execute(f"PRAGMA table_info({table})").fetchall()
        }
        if table_columns and "component_id" not in table_columns:
            conn.execute(
                f"ALTER TABLE {table} ADD COLUMN component_id TEXT "
                "REFERENCES custody_components(id) ON DELETE SET NULL"
            )


def open_db(
    data_root,
    *,
    passphrase=None,
    require_existing_schema=False,
    expected_database_identity=None,
):
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
        conn = sqlite3.connect(db_path, timeout=DB_BUSY_TIMEOUT_SECONDS)
        conn.row_factory = sqlite3.Row
        try:
            if require_existing_schema and not database_has_core_schema(conn):
                raise AppError(
                    "database does not contain a Kassiber project schema",
                    code="invalid_project_database",
                    hint="Choose an existing Kassiber project database, not an empty or unrelated SQLite file.",
                    details={"database": str(db_path)},
                    retryable=False,
                )
            if expected_database_identity is not None:
                require_database_instance_id(conn, expected_database_identity)
            _configure_connection_pragmas(conn)
            _preflight_schema_index_columns(conn)
            conn.executescript(SCHEMA)
            ensure_schema_compat(conn)
            ensure_database_instance_id(conn)
            return conn
        except Exception:
            conn.close()
            raise

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
    try:
        if require_existing_schema and not database_has_core_schema(conn):
            raise AppError(
                "database does not contain a Kassiber project schema",
                code="invalid_project_database",
                hint="Choose an existing Kassiber project database, not an empty or unrelated SQLCipher file.",
                details={"database": str(db_path)},
                retryable=False,
            )
        if expected_database_identity is not None:
            require_database_instance_id(conn, expected_database_identity)
        _configure_connection_pragmas(conn, encrypted=True)
        _preflight_schema_index_columns(conn)
        conn.executescript(SCHEMA)
        ensure_schema_compat(conn)
        ensure_database_instance_id(conn)
        return conn
    except Exception:
        conn.close()
        raise


def ensure_database_instance_id(conn) -> str:
    """Return the durable random identity read from the database connection."""

    row = conn.execute(
        "SELECT value FROM settings WHERE key = ?",
        (DATABASE_INSTANCE_ID_SETTING,),
    ).fetchone()
    value = row["value"] if row else None
    if (
        isinstance(value, str)
        and len(value) == 32
        and all(character in "0123456789abcdef" for character in value)
    ):
        return value
    value = uuid.uuid4().hex
    conn.execute(
        """
        INSERT INTO settings(key, value)
        VALUES(?, ?)
        ON CONFLICT(key) DO UPDATE SET value = excluded.value
        """,
        (DATABASE_INSTANCE_ID_SETTING, value),
    )
    conn.commit()
    return value


def database_instance_id(conn) -> str:
    """Read the validated project identity through an already-open connection."""

    row = conn.execute(
        "SELECT value FROM settings WHERE key = ?",
        (DATABASE_INSTANCE_ID_SETTING,),
    ).fetchone()
    value = row["value"] if row else None
    if not (
        isinstance(value, str)
        and len(value) == 32
        and all(character in "0123456789abcdef" for character in value)
    ):
        raise AppError(
            "database instance identity is missing or invalid",
            code="invalid_project_database",
            retryable=False,
        )
    return value


def require_database_instance_id(conn, expected: str) -> None:
    """Reject an opened connection before migration if it is not the lease DB."""

    try:
        actual = database_instance_id(conn)
    except Exception as exc:
        raise AppError(
            "the opened database does not match the operator lease",
            code="operator_project_replaced",
            retryable=False,
        ) from exc
    if actual != expected:
        raise AppError(
            "the opened database does not match the operator lease",
            code="operator_project_replaced",
            retryable=False,
        )


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


def _ensure_column_no_commit(conn, table_name, column_name, definition):
    """Idempotent `ALTER TABLE ... ADD COLUMN` — no-op when the column exists."""
    columns = {row["name"] for row in conn.execute(f"PRAGMA table_info({table_name})").fetchall()}
    if column_name in columns:
        return False
    conn.execute(f"ALTER TABLE {table_name} ADD COLUMN {column_name} {definition}")
    return True


def ensure_column(conn, table_name, column_name, definition):
    """Idempotent `ALTER TABLE ... ADD COLUMN` — no-op when the column exists."""
    if not _ensure_column_no_commit(conn, table_name, column_name, definition):
        return
    conn.commit()


def _custody_leg_schema_supports_suspense(conn):
    row = conn.execute(
        "SELECT sql FROM sqlite_master WHERE type = 'table' "
        "AND name = 'custody_component_legs'"
    ).fetchone()
    return row is not None and "'suspense'" in str(row["sql"] or "")


def _create_custody_leg_indexes_and_scope_triggers(conn):
    """Restore the non-immutability objects around rebuilt custody child tables."""

    for statement in (
        "CREATE INDEX idx_custody_component_legs_component "
        "ON custody_component_legs(component_id, ordinal, id)",
        "CREATE INDEX idx_custody_component_legs_profile_transaction "
        "ON custody_component_legs(profile_id, transaction_id) "
        "WHERE transaction_id IS NOT NULL",
        "CREATE INDEX idx_custody_component_legs_profile_wallet "
        "ON custody_component_legs(profile_id, wallet_id) "
        "WHERE wallet_id IS NOT NULL",
        "CREATE INDEX idx_custody_allocations_component "
        "ON custody_component_allocations(component_id, ordinal, id)",
        "CREATE INDEX idx_custody_allocations_source "
        "ON custody_component_allocations(source_leg_id)",
        "CREATE INDEX idx_custody_allocations_sink "
        "ON custody_component_allocations(sink_leg_id)",
    ):
        conn.execute(statement)
    conn.execute(
        """
        CREATE TRIGGER trg_custody_allocation_scope_insert
        BEFORE INSERT ON custody_component_allocations
        BEGIN
            SELECT CASE WHEN NOT EXISTS (
                SELECT 1
                FROM custody_components c
                JOIN custody_component_legs source ON source.id = NEW.source_leg_id
                JOIN custody_component_legs sink ON sink.id = NEW.sink_leg_id
                WHERE c.id = NEW.component_id
                  AND c.workspace_id = NEW.workspace_id
                  AND c.profile_id = NEW.profile_id
                  AND source.component_id = c.id
                  AND sink.component_id = c.id
                  AND source.role = 'source'
                  AND sink.role != 'source'
            ) THEN RAISE(ABORT, 'custody_allocation_scope_or_role_mismatch') END;
        END
        """
    )
    conn.execute(
        """
        CREATE TRIGGER trg_custody_allocation_scope_update
        BEFORE UPDATE OF component_id, workspace_id, profile_id,
                         source_leg_id, sink_leg_id
        ON custody_component_allocations
        BEGIN
            SELECT CASE WHEN NOT EXISTS (
                SELECT 1
                FROM custody_components c
                JOIN custody_component_legs source ON source.id = NEW.source_leg_id
                JOIN custody_component_legs sink ON sink.id = NEW.sink_leg_id
                WHERE c.id = NEW.component_id
                  AND c.workspace_id = NEW.workspace_id
                  AND c.profile_id = NEW.profile_id
                  AND source.component_id = c.id
                  AND sink.component_id = c.id
                  AND source.role = 'source'
                  AND sink.role != 'source'
            ) THEN RAISE(ABORT, 'custody_allocation_scope_or_role_mismatch') END;
        END
        """
    )
    conn.execute(
        """
        CREATE TRIGGER trg_custody_component_scope_insert
        BEFORE INSERT ON custody_component_legs
        BEGIN
            SELECT CASE WHEN NOT EXISTS (
                SELECT 1 FROM custody_components c
                WHERE c.id = NEW.component_id
                  AND c.workspace_id = NEW.workspace_id
                  AND c.profile_id = NEW.profile_id
            ) THEN RAISE(ABORT, 'custody_leg_component_scope_mismatch') END;
            SELECT CASE WHEN NEW.transaction_id IS NOT NULL AND NOT EXISTS (
                SELECT 1 FROM transactions t
                WHERE t.id = NEW.transaction_id
                  AND t.workspace_id = NEW.workspace_id
                  AND t.profile_id = NEW.profile_id
            ) THEN RAISE(ABORT, 'custody_leg_transaction_scope_mismatch') END;
            SELECT CASE WHEN NEW.wallet_id IS NOT NULL AND NOT EXISTS (
                SELECT 1 FROM wallets w
                WHERE w.id = NEW.wallet_id
                  AND w.workspace_id = NEW.workspace_id
                  AND w.profile_id = NEW.profile_id
            ) THEN RAISE(ABORT, 'custody_leg_wallet_scope_mismatch') END;
        END
        """
    )
    conn.execute(
        """
        CREATE TRIGGER trg_custody_component_scope_update
        BEFORE UPDATE OF component_id, workspace_id, profile_id,
                         transaction_id, wallet_id
        ON custody_component_legs
        BEGIN
            SELECT CASE WHEN NOT EXISTS (
                SELECT 1 FROM custody_components c
                WHERE c.id = NEW.component_id
                  AND c.workspace_id = NEW.workspace_id
                  AND c.profile_id = NEW.profile_id
            ) THEN RAISE(ABORT, 'custody_leg_component_scope_mismatch') END;
            SELECT CASE WHEN NEW.transaction_id IS NOT NULL AND NOT EXISTS (
                SELECT 1 FROM transactions t
                WHERE t.id = NEW.transaction_id
                  AND t.workspace_id = NEW.workspace_id
                  AND t.profile_id = NEW.profile_id
            ) THEN RAISE(ABORT, 'custody_leg_transaction_scope_mismatch') END;
            SELECT CASE WHEN NEW.wallet_id IS NOT NULL AND NOT EXISTS (
                SELECT 1 FROM wallets w
                WHERE w.id = NEW.wallet_id
                  AND w.workspace_id = NEW.workspace_id
                  AND w.profile_id = NEW.profile_id
            ) THEN RAISE(ABORT, 'custody_leg_wallet_scope_mismatch') END;
        END
        """
    )
    conn.execute(
        """
        CREATE TRIGGER trg_custody_component_memberships_leg_delete
        AFTER DELETE ON custody_component_legs
        WHEN OLD.transaction_id IS NOT NULL
        BEGIN
            DELETE FROM custody_component_transaction_memberships
            WHERE component_id = OLD.component_id
              AND transaction_id = OLD.transaction_id
              AND NOT EXISTS (
                  SELECT 1 FROM custody_component_legs l
                  WHERE l.component_id = OLD.component_id
                    AND l.transaction_id = OLD.transaction_id
              );
        END
        """
    )


def _rebuild_custody_leg_role_schema(conn):
    """Expand the closed custody-leg role set without rewriting authored rows."""

    leg_columns = (
        "id, component_id, workspace_id, profile_id, ordinal, role, rail, "
        "chain, network, asset, exposure, conservation_unit, amount_msat, "
        "valuation_unit, valuation_amount, occurred_at, transaction_id, "
        "anchor_transaction_id, wallet_id, location_ref, notes, created_at"
    )
    allocation_columns = (
        "id, component_id, workspace_id, profile_id, ordinal, source_leg_id, "
        "sink_leg_id, source_amount_msat, sink_amount_msat, created_at"
    )
    term_columns = (
        "id, component_id, workspace_id, profile_id, ordinal, source_leg_id, "
        "target_leg_id, term_kind, legacy_source_id, source_row_hash, "
        "review_kind, tax_policy, reviewed_source_amount_msat, swap_fee_msat, "
        "swap_fee_kind, confidence_at_review, review_source, review_notes, "
        "payout_asset, "
        "payout_amount_msat, payout_occurred_at, payout_fiat_value_exact, "
        "payout_external_id, counterparty, created_at"
    )
    counts_before = {
        "legs": int(conn.execute("SELECT COUNT(*) FROM custody_component_legs").fetchone()[0]),
        "allocations": int(
            conn.execute("SELECT COUNT(*) FROM custody_component_allocations").fetchone()[0]
        ),
        "economic_terms": int(
            conn.execute(
                "SELECT COUNT(*) FROM custody_component_economic_terms"
            ).fetchone()[0]
        ),
    }
    previous_fk = int(conn.execute("PRAGMA foreign_keys").fetchone()[0])
    conn.commit()
    conn.execute("PRAGMA foreign_keys = OFF")
    try:
        conn.execute("BEGIN IMMEDIATE")
        for trigger in (
            "trg_custody_allocation_scope_insert",
            "trg_custody_allocation_scope_update",
            "trg_custody_component_scope_insert",
            "trg_custody_component_scope_update",
            "trg_custody_component_memberships_leg_delete",
            "trg_custody_component_leg_revision_immutable",
            "trg_custody_component_allocation_revision_immutable",
            "trg_custody_component_leg_revision_delete_immutable",
            "trg_custody_component_allocation_revision_delete_immutable",
            "trg_custody_component_leg_count_commitment",
            "trg_custody_component_allocation_count_commitment",
            "trg_custody_component_economic_term_count_commitment",
            "trg_custody_component_terms_scope_insert",
            "trg_custody_component_terms_immutable",
            "trg_custody_component_terms_delete_immutable",
        ):
            conn.execute(f"DROP TRIGGER IF EXISTS {trigger}")
        for index in (
            "idx_custody_component_legs_component",
            "idx_custody_component_legs_profile_transaction",
            "idx_custody_component_legs_profile_wallet",
            "idx_custody_allocations_component",
            "idx_custody_allocations_source",
            "idx_custody_allocations_sink",
            "idx_custody_component_terms_profile_kind",
            "idx_custody_component_terms_component",
            "idx_custody_component_terms_legacy_source",
        ):
            conn.execute(f"DROP INDEX IF EXISTS {index}")

        # SQLite rewrites dependent foreign keys when a referenced table is
        # renamed. Move the terms table too, otherwise it keeps pointing at
        # ``custody_component_legs__pre_suspense`` after this migration.
        conn.execute(
            "ALTER TABLE custody_component_economic_terms "
            "RENAME TO custody_component_economic_terms__pre_suspense"
        )
        conn.execute(
            "ALTER TABLE custody_component_allocations "
            "RENAME TO custody_component_allocations__pre_suspense"
        )
        conn.execute(
            "ALTER TABLE custody_component_legs "
            "RENAME TO custody_component_legs__pre_suspense"
        )
        conn.execute(
            """
            CREATE TABLE custody_component_legs (
                id TEXT PRIMARY KEY,
                component_id TEXT NOT NULL REFERENCES custody_components(id) ON DELETE CASCADE,
                workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
                ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
                role TEXT NOT NULL CHECK (
                    role IN ('source', 'destination', 'fee', 'external',
                             'retained', 'unresolved', 'suspense')
                ),
                rail TEXT NOT NULL,
                chain TEXT,
                network TEXT,
                asset TEXT NOT NULL,
                exposure TEXT NOT NULL,
                conservation_unit TEXT NOT NULL,
                amount_msat INTEGER NOT NULL CHECK (
                    typeof(amount_msat) = 'integer' AND amount_msat >= 0
                ),
                valuation_unit TEXT,
                valuation_amount INTEGER CHECK (
                    valuation_amount IS NULL OR
                    (typeof(valuation_amount) = 'integer' AND valuation_amount >= 0)
                ),
                occurred_at TEXT,
                transaction_id TEXT REFERENCES transactions(id) ON DELETE SET NULL,
                anchor_transaction_id TEXT,
                wallet_id TEXT REFERENCES wallets(id) ON DELETE SET NULL,
                location_ref TEXT,
                notes TEXT,
                created_at TEXT NOT NULL,
                UNIQUE (component_id, ordinal)
            )
            """
        )
        conn.execute(
            """
            CREATE TABLE custody_component_allocations (
                id TEXT PRIMARY KEY,
                component_id TEXT NOT NULL REFERENCES custody_components(id) ON DELETE CASCADE,
                workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
                ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
                source_leg_id TEXT NOT NULL REFERENCES custody_component_legs(id) ON DELETE CASCADE,
                sink_leg_id TEXT NOT NULL REFERENCES custody_component_legs(id) ON DELETE CASCADE,
                source_amount_msat INTEGER NOT NULL CHECK (
                    typeof(source_amount_msat) = 'integer' AND source_amount_msat >= 0
                ),
                sink_amount_msat INTEGER NOT NULL CHECK (
                    typeof(sink_amount_msat) = 'integer' AND sink_amount_msat >= 0
                ),
                created_at TEXT NOT NULL,
                UNIQUE (component_id, ordinal),
                UNIQUE (component_id, source_leg_id, sink_leg_id)
            )
            """
        )
        conn.execute(
            """
            CREATE TABLE custody_component_economic_terms (
                id TEXT PRIMARY KEY,
                component_id TEXT NOT NULL
                    REFERENCES custody_components(id) ON DELETE CASCADE,
                workspace_id TEXT NOT NULL
                    REFERENCES workspaces(id) ON DELETE CASCADE,
                profile_id TEXT NOT NULL
                    REFERENCES profiles(id) ON DELETE CASCADE,
                ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
                source_leg_id TEXT NOT NULL
                    REFERENCES custody_component_legs(id) ON DELETE CASCADE,
                target_leg_id TEXT NOT NULL
                    REFERENCES custody_component_legs(id) ON DELETE CASCADE,
                term_kind TEXT NOT NULL CHECK(
                    term_kind IN ('transaction_pair', 'direct_swap_payout')
                ),
                legacy_source_id TEXT NOT NULL,
                source_row_hash TEXT NOT NULL CHECK(length(source_row_hash) = 64),
                review_kind TEXT NOT NULL,
                tax_policy TEXT NOT NULL,
                reviewed_source_amount_msat INTEGER CHECK(
                    reviewed_source_amount_msat IS NULL OR
                    typeof(reviewed_source_amount_msat) = 'integer'
                ),
                swap_fee_msat INTEGER,
                swap_fee_kind TEXT,
                confidence_at_review TEXT,
                review_source TEXT,
                review_notes TEXT,
                payout_asset TEXT,
                payout_amount_msat INTEGER CHECK(
                    payout_amount_msat IS NULL OR
                    typeof(payout_amount_msat) = 'integer'
                ),
                payout_occurred_at TEXT,
                payout_fiat_value_exact TEXT,
                payout_external_id TEXT,
                counterparty TEXT,
                created_at TEXT NOT NULL,
                UNIQUE(component_id, ordinal),
                UNIQUE(component_id, term_kind, legacy_source_id)
            )
            """
        )
        conn.execute(
            f"INSERT INTO custody_component_legs({leg_columns}) "
            f"SELECT {leg_columns} FROM custody_component_legs__pre_suspense"
        )
        conn.execute(
            f"INSERT INTO custody_component_allocations({allocation_columns}) "
            f"SELECT {allocation_columns} "
            "FROM custody_component_allocations__pre_suspense"
        )
        conn.execute(
            f"INSERT INTO custody_component_economic_terms({term_columns}) "
            f"SELECT {term_columns} "
            "FROM custody_component_economic_terms__pre_suspense"
        )
        counts_after = {
            "legs": int(conn.execute("SELECT COUNT(*) FROM custody_component_legs").fetchone()[0]),
            "allocations": int(
                conn.execute("SELECT COUNT(*) FROM custody_component_allocations").fetchone()[0]
            ),
            "economic_terms": int(
                conn.execute(
                    "SELECT COUNT(*) FROM custody_component_economic_terms"
                ).fetchone()[0]
            ),
        }
        if counts_after != counts_before:
            raise RuntimeError(
                f"custody child row counts changed: {counts_before} -> {counts_after}"
            )
        conn.execute("DROP TABLE custody_component_economic_terms__pre_suspense")
        conn.execute("DROP TABLE custody_component_allocations__pre_suspense")
        conn.execute("DROP TABLE custody_component_legs__pre_suspense")
        conn.execute(
            "CREATE INDEX idx_custody_component_terms_profile_kind "
            "ON custody_component_economic_terms("
            "profile_id, term_kind, created_at, component_id)"
        )
        conn.execute(
            "CREATE INDEX idx_custody_component_terms_component "
            "ON custody_component_economic_terms(component_id, ordinal, id)"
        )
        conn.execute(
            "CREATE INDEX idx_custody_component_terms_legacy_source "
            "ON custody_component_economic_terms("
            "profile_id, term_kind, legacy_source_id, created_at, component_id)"
        )
        conn.execute(
            """
            CREATE TRIGGER trg_custody_component_terms_scope_insert
            BEFORE INSERT ON custody_component_economic_terms
            BEGIN
                SELECT CASE WHEN NOT EXISTS (
                    SELECT 1
                    FROM custody_components c
                    JOIN custody_component_legs source
                      ON source.id = NEW.source_leg_id
                    JOIN custody_component_legs target
                      ON target.id = NEW.target_leg_id
                    WHERE c.id = NEW.component_id
                      AND c.workspace_id = NEW.workspace_id
                      AND c.profile_id = NEW.profile_id
                      AND source.component_id = c.id
                      AND target.component_id = c.id
                      AND source.role = 'source'
                      AND target.role != 'source'
                ) THEN RAISE(
                    ABORT, 'custody_component_terms_scope_mismatch'
                ) END;
            END
            """
        )
        conn.execute(
            """
            CREATE TRIGGER trg_custody_component_terms_immutable
            BEFORE UPDATE ON custody_component_economic_terms
            BEGIN
                SELECT RAISE(ABORT, 'custody_component_terms_immutable');
            END
            """
        )
        conn.execute(
            """
            CREATE TRIGGER trg_custody_component_terms_delete_immutable
            BEFORE DELETE ON custody_component_economic_terms
            WHEN EXISTS (
                SELECT 1 FROM profiles p WHERE p.id = OLD.profile_id
            )
            AND NOT EXISTS (
                SELECT 1
                FROM custody_component_purge_authorizations authorization
                WHERE authorization.profile_id = OLD.profile_id
            )
            BEGIN
                SELECT RAISE(ABORT, 'custody_component_terms_delete_immutable');
            END
            """
        )
        _create_custody_leg_indexes_and_scope_triggers(conn)
        _ensure_custody_revision_immutability_triggers(conn)
        violations = [
            dict(row)
            for row in conn.execute("PRAGMA foreign_key_check").fetchall()
            if row["table"] in {
                "custody_component_legs",
                "custody_component_allocations",
                "custody_component_economic_terms",
            }
        ]
        if violations:
            raise RuntimeError(
                f"custody child foreign-key violations: {violations}"
            )
        conn.commit()
    except Exception as exc:
        conn.rollback()
        raise AppError(
            "custody component schema migration failed",
            code="custody_schema_migration_failed",
            hint="Restore the database backup or retry with a compatible Kassiber build.",
            details={"migration": "custody_leg_suspense_role"},
            retryable=False,
        ) from exc
    finally:
        conn.execute(f"PRAGMA foreign_keys = {'ON' if previous_fk else 'OFF'}")

    # Sanity-check again after restoring FK enforcement. The transactional
    # check above is the one that can still roll the migration back.
    violations = [
        dict(row)
        for row in conn.execute("PRAGMA foreign_key_check").fetchall()
        if row["table"] in {
            "custody_component_legs",
            "custody_component_allocations",
            "custody_component_economic_terms",
        }
    ]
    if violations:
        raise AppError(
            "custody component schema migration left invalid references",
            code="custody_schema_migration_failed",
            details={
                "migration": "custody_leg_suspense_role",
                "foreign_key_violations": violations,
            },
            retryable=False,
        )


def _ensure_custody_leg_role_schema(conn):
    if not _custody_leg_schema_supports_suspense(conn):
        _rebuild_custody_leg_role_schema(conn)


def _custody_evidence_commitment_id(component_id, ordinal):
    encoded = json.dumps(
        ["custody-component-evidence-v1", str(component_id), int(ordinal)],
        ensure_ascii=True,
        separators=(",", ":"),
    )
    return hashlib.sha256(encoded.encode("utf-8")).hexdigest()


def custody_gap_review_transaction_id(review_id, role, transaction_id):
    """Return the stable authored id for one review boundary anchor."""

    encoded = json.dumps(
        [
            "custody-gap-review-transaction-v2",
            str(review_id),
            str(role),
            str(transaction_id),
        ],
        ensure_ascii=True,
        separators=(",", ":"),
    )
    return hashlib.sha256(encoded.encode("utf-8")).hexdigest()


def custody_gap_review_transaction_v1_id(review_id, role, ordinal):
    """Return the retired ordinal-keyed wire id accepted during replay."""

    encoded = json.dumps(
        [
            "custody-gap-review-transaction-v1",
            str(review_id),
            str(role),
            int(ordinal),
        ],
        ensure_ascii=True,
        separators=(",", ":"),
    )
    return hashlib.sha256(encoded.encode("utf-8")).hexdigest()


def custody_gap_review_relation_set_id(review_id):
    """Return the stable authored id for a review boundary commitment."""

    encoded = json.dumps(
        ["custody-gap-review-relation-set-v1", str(review_id)],
        ensure_ascii=True,
        separators=(",", ":"),
    )
    return hashlib.sha256(encoded.encode("utf-8")).hexdigest()


def backfill_custody_gap_review_relation_set(
    conn,
    *,
    review_id,
    workspace_id,
    profile_id,
    created_at,
    expected_source_count,
    expected_return_count,
):
    """Persist a missing immutable completeness header without rewriting it."""

    cursor = conn.execute(
        """
        INSERT OR IGNORE INTO custody_gap_review_relation_sets(
            id, review_id, workspace_id, profile_id,
            expected_source_count, expected_return_count, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?)
        """,
        (
            custody_gap_review_relation_set_id(review_id),
            review_id,
            workspace_id,
            profile_id,
            int(expected_source_count),
            int(expected_return_count),
            created_at,
        ),
    )
    return max(0, int(cursor.rowcount))


def _create_custody_gap_review_transaction_aux_schema(conn):
    # ``executescript`` commits any pending transaction before running. Keep
    # these statements individually executable so the v1 table rebuild can
    # include its indexes and triggers in one savepoint.
    conn.execute(
        """
        CREATE INDEX IF NOT EXISTS idx_custody_gap_review_transactions_review
            ON custody_gap_review_transactions(
                review_id, role, transaction_id, id
            )
        """
    )
    conn.execute(
        """
        CREATE INDEX IF NOT EXISTS idx_custody_gap_review_transactions_scope
            ON custody_gap_review_transactions(
                profile_id, transaction_id, review_id
            )
        """
    )
    conn.execute(
        """
        CREATE TRIGGER IF NOT EXISTS trg_custody_gap_review_transaction_scope_insert
        BEFORE INSERT ON custody_gap_review_transactions
        BEGIN
            SELECT CASE WHEN NOT EXISTS (
                SELECT 1 FROM custody_gap_reviews r
                WHERE r.id = NEW.review_id
                  AND r.workspace_id = NEW.workspace_id
                  AND r.profile_id = NEW.profile_id
            ) THEN RAISE(
                ABORT, 'custody_gap_review_transaction_review_scope_mismatch'
            ) END;
            SELECT CASE WHEN EXISTS (
                SELECT 1 FROM transactions t WHERE t.id = NEW.transaction_id
            ) AND NOT EXISTS (
                SELECT 1 FROM transactions t
                WHERE t.id = NEW.transaction_id
                  AND t.workspace_id = NEW.workspace_id
                  AND t.profile_id = NEW.profile_id
            ) THEN RAISE(
                ABORT, 'custody_gap_review_transaction_scope_mismatch'
            ) END;
        END
        """
    )
    conn.execute(
        """
        CREATE TRIGGER IF NOT EXISTS trg_custody_gap_review_transactions_immutable
        BEFORE UPDATE ON custody_gap_review_transactions
        BEGIN
            SELECT RAISE(ABORT, 'custody_gap_review_transactions_immutable');
        END
        """
    )


def _create_custody_gap_review_transaction_table(conn):
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS custody_gap_review_transactions (
            id TEXT PRIMARY KEY,
            review_id TEXT NOT NULL
                REFERENCES custody_gap_reviews(id) ON DELETE CASCADE,
            workspace_id TEXT NOT NULL
                REFERENCES workspaces(id) ON DELETE CASCADE,
            profile_id TEXT NOT NULL
                REFERENCES profiles(id) ON DELETE CASCADE,
            role TEXT NOT NULL CHECK (role IN ('source', 'return')),
            transaction_id TEXT NOT NULL,
            created_at TEXT NOT NULL,
            UNIQUE (review_id, role, transaction_id)
        )
        """
    )


def _v1_review_relation_wire_identity(conn, row):
    """Recover the portable relation tuple from its original signed upsert.

    Alias catalogs are intentionally device-local projections and two peers
    may know different subsets.  A migrated v1 row therefore takes its review
    and transaction identities from the immutable signed event that authored
    that exact ordinal row. A row never captured before upgrade has no portable
    identity yet and safely falls back to its local tuple; capture will derive
    its first v2 wire identity after migration.
    """

    entity_key = json.dumps([str(row["id"])], ensure_ascii=True, separators=(",", ":"))
    events = conn.execute(
        """
        SELECT payload_json
        FROM sync_events
        WHERE profile_id = ?
          AND entity_table = 'custody_gap_review_transactions'
          AND entity_key = ?
          AND event_type = 'row.upsert'
        ORDER BY replica_id, replica_seq, id
        """,
        (row["profile_id"], entity_key),
    ).fetchall()
    for event in events:
        try:
            payload = json.loads(str(event["payload_json"] or "{}"))
        except (TypeError, ValueError, json.JSONDecodeError):
            continue
        wire_row = payload.get("row") if isinstance(payload, dict) else None
        if not isinstance(wire_row, dict):
            continue
        if (
            str(wire_row.get("id") or "") == str(row["id"])
            and str(wire_row.get("role") or "") == str(row["role"])
            and type(wire_row.get("ordinal")) is int
        ):
            review_id = str(wire_row.get("review_id") or "")
            transaction_id = str(wire_row.get("transaction_id") or "")
            if review_id and transaction_id:
                return review_id, transaction_id
    return str(row["review_id"]), str(row["transaction_id"])


def _ensure_custody_gap_review_transaction_schema(conn):
    """Replace the unreleased ordinal-keyed relation shape with set identity."""

    table_names = {
        str(row["name"])
        for row in conn.execute(
            "SELECT name FROM sqlite_master WHERE type = 'table'"
        ).fetchall()
    }
    columns = {
        str(row["name"])
        for row in conn.execute(
            "PRAGMA table_info(custody_gap_review_transactions)"
        ).fetchall()
    }
    legacy_exists = "custody_gap_review_transactions_v1" in table_names
    if "ordinal" not in columns and not legacy_exists:
        return False
    savepoint = "migrate_custody_gap_review_transactions_v2"
    conn.execute(f"SAVEPOINT {savepoint}")
    try:
        for object_kind, object_name in (
            ("TRIGGER", "trg_custody_gap_review_transaction_scope_insert"),
            ("TRIGGER", "trg_custody_gap_review_transactions_immutable"),
            ("INDEX", "idx_custody_gap_review_transactions_review"),
            ("INDEX", "idx_custody_gap_review_transactions_scope"),
        ):
            conn.execute(f"DROP {object_kind} IF EXISTS {object_name}")
        if "ordinal" in columns:
            if legacy_exists:
                raise AppError(
                    "Custody review relation migration state is ambiguous.",
                    code="custody_review_relation_migration_conflict",
                    retryable=False,
                )
            conn.execute(
                "ALTER TABLE custody_gap_review_transactions "
                "RENAME TO custody_gap_review_transactions_v1"
            )
            _create_custody_gap_review_transaction_table(conn)
        else:
            # Recover a database left by the old non-atomic rebuild after its
            # legacy rename. SCHEMA may already have recreated an empty/partial
            # v2 table, so merge by stable relation id instead of replacing it.
            _create_custody_gap_review_transaction_table(conn)
        rows = conn.execute(
            """
            SELECT id, review_id, workspace_id, profile_id, role,
                   transaction_id, created_at
            FROM custody_gap_review_transactions_v1
            ORDER BY review_id, role, transaction_id, id
            """,
        ).fetchall()
        for row in rows:
            wire_review_id, wire_transaction_id = _v1_review_relation_wire_identity(
                conn, row
            )
            relation_id = custody_gap_review_transaction_id(
                wire_review_id,
                row["role"],
                wire_transaction_id,
            )
            conn.execute(
                """
                INSERT OR IGNORE INTO custody_gap_review_transactions(
                    id, review_id, workspace_id, profile_id,
                    role, transaction_id, created_at
                ) VALUES(?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    relation_id,
                    row["review_id"],
                    row["workspace_id"],
                    row["profile_id"],
                    row["role"],
                    row["transaction_id"],
                    row["created_at"],
                ),
            )
            # A delayed signed v1 tombstone/upsert still names the ordinal row
            # id. Redirect that portable alias before dropping the legacy row.
            conn.execute(
                """
                UPDATE sync_id_map
                SET local_id = ?
                WHERE profile_id = ?
                  AND entity_table = 'custody_gap_review_transactions'
                  AND local_id = ?
                """,
                (relation_id, row["profile_id"], row["id"]),
            )
            conn.execute(
                """
                INSERT INTO sync_id_map(
                    profile_id, entity_table, wire_id, local_id, created_at
                ) VALUES(?, 'custody_gap_review_transactions', ?, ?, ?)
                ON CONFLICT(profile_id, entity_table, wire_id)
                DO UPDATE SET local_id = excluded.local_id
                """,
                (
                    row["profile_id"],
                    row["id"],
                    relation_id,
                    row["created_at"],
                ),
            )
        conn.execute("DROP TABLE custody_gap_review_transactions_v1")
        _create_custody_gap_review_transaction_aux_schema(conn)
        conn.execute(f"RELEASE SAVEPOINT {savepoint}")
    except Exception:
        conn.execute(f"ROLLBACK TO SAVEPOINT {savepoint}")
        conn.execute(f"RELEASE SAVEPOINT {savepoint}")
        raise
    return True


def backfill_custody_gap_review_relations(
    conn,
    *,
    review_id,
    workspace_id,
    profile_id,
    created_at,
    relations,
):
    """Insert any missing durable review anchors without rewriting history.

    Existing databases and replicas can contain only a prefix of the relation
    rows.  Repair therefore works per ``(role, transaction_id)`` instead of
    treating the first child row as proof that the whole review was migrated.
    An authored transaction identity remains valid after source retraction;
    only a currently-live row owned by another book makes that identity unsafe
    to attach here.
    """

    grouped = {"source": [], "return": []}
    for role, transaction_id in relations:
        normalized_role = str(role)
        normalized_id = str(transaction_id or "")
        if normalized_role not in grouped or not normalized_id:
            continue
        if normalized_id not in grouped[normalized_role]:
            grouped[normalized_role].append(normalized_id)

    existing_rows = conn.execute(
        """
        SELECT role, transaction_id
        FROM custody_gap_review_transactions
        WHERE review_id = ?
        ORDER BY role, transaction_id, id
        """,
        (review_id,),
    ).fetchall()
    existing_ids = {"source": set(), "return": set()}
    for row in existing_rows:
        role = str(row["role"])
        if role not in existing_ids:
            continue
        existing_ids[role].add(str(row["transaction_id"]))

    inserted = 0
    for role in ("source", "return"):
        for transaction_id in sorted(grouped[role]):
            if transaction_id in existing_ids[role]:
                continue
            collision = conn.execute(
                """
                SELECT 1
                FROM transactions
                WHERE id = ?
                  AND (workspace_id != ? OR profile_id != ?)
                LIMIT 1
                """,
                (transaction_id, workspace_id, profile_id),
            ).fetchone()
            if collision is not None:
                continue
            cursor = conn.execute(
                """
                INSERT OR IGNORE INTO custody_gap_review_transactions(
                    id, review_id, workspace_id, profile_id,
                    role, transaction_id, created_at
                ) VALUES(?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    custody_gap_review_transaction_id(
                        review_id, role, transaction_id
                    ),
                    review_id,
                    workspace_id,
                    profile_id,
                    role,
                    transaction_id,
                    created_at,
                ),
            )
            if int(cursor.rowcount) > 0:
                inserted += int(cursor.rowcount)
                existing_ids[role].add(transaction_id)
    return inserted


def _backfill_custody_gap_review_transactions(conn):
    """Normalize only transaction identities already durably authored.

    Older candidate fingerprints cannot be reversed here, and current derived
    candidates may have changed since a review.  This lower-layer pass is
    therefore limited to legacy snapshot id arrays and component leg anchors.
    The custody review layer separately performs an exact fingerprint-checked
    candidate recovery for old componentless dismissals whose snapshots never
    contained those arrays.
    """

    inserted = 0
    reviews = conn.execute(
        """
        SELECT id, workspace_id, profile_id, component_id, snapshot_json, created_at
        FROM custody_gap_reviews
        ORDER BY created_at, id
        """
    ).fetchall()
    for review in reviews:
        relation_ids = {"source": [], "return": []}
        try:
            snapshot = json.loads(str(review["snapshot_json"] or "{}"))
        except (TypeError, ValueError, json.JSONDecodeError):
            snapshot = {}
        if isinstance(snapshot, dict):
            for role, key in (("source", "source_ids"), ("return", "return_ids")):
                values = snapshot.get(key)
                if not isinstance(values, list):
                    continue
                for value in values:
                    if not isinstance(value, str) or not value:
                        continue
                    if value not in relation_ids[role]:
                        relation_ids[role].append(value)

        if review["component_id"]:
            legs = conn.execute(
                """
                SELECT role, COALESCE(anchor_transaction_id, transaction_id) AS transaction_id
                FROM custody_component_legs
                WHERE component_id = ?
                  AND workspace_id = ?
                  AND profile_id = ?
                  AND role IN ('source', 'fee', 'destination')
                  AND COALESCE(anchor_transaction_id, transaction_id) IS NOT NULL
                ORDER BY ordinal, id
                """,
                (
                    review["component_id"],
                    review["workspace_id"],
                    review["profile_id"],
                ),
            ).fetchall()
            for leg in legs:
                role = "return" if leg["role"] == "destination" else "source"
                transaction_id = str(leg["transaction_id"])
                if transaction_id not in relation_ids[role]:
                    relation_ids[role].append(transaction_id)

        inserted += backfill_custody_gap_review_relations(
            conn,
            review_id=review["id"],
            workspace_id=review["workspace_id"],
            profile_id=review["profile_id"],
            created_at=review["created_at"],
            relations=(
                *(("source", value) for value in relation_ids["source"]),
                *(("return", value) for value in relation_ids["return"]),
            ),
        )
        if relation_ids["source"] or relation_ids["return"]:
            backfill_custody_gap_review_relation_set(
                conn,
                review_id=review["id"],
                workspace_id=review["workspace_id"],
                profile_id=review["profile_id"],
                created_at=review["created_at"],
                expected_source_count=len(relation_ids["source"]),
                expected_return_count=len(relation_ids["return"]),
            )
    return inserted


def _backfill_legacy_componentless_review_transactions(conn):
    """Run the higher-layer deterministic recovery after DB initialization.

    The cheap guard avoids importing or running the custody matcher for normal
    databases.  The local runtime import keeps the foundational DB module free
    of a module-load back-edge while allowing a semantic migration that cannot
    safely be implemented by reversing a one-way candidate fingerprint.
    """

    pending = conn.execute(
        """
        SELECT 1
        FROM custody_gap_reviews r
        WHERE r.component_id IS NULL
          AND r.action = 'dismissed'
          AND COALESCE(r.event_kind, 'review_decision') = 'review_decision'
          AND (
              NOT EXISTS (
                  SELECT 1 FROM custody_gap_review_relation_sets s
                  WHERE s.review_id = r.id
              )
              OR EXISTS (
                  SELECT 1
                  FROM custody_gap_review_relation_sets s
                  WHERE s.review_id = r.id
                    AND (
                        s.expected_source_count != (
                            SELECT COUNT(*)
                            FROM custody_gap_review_transactions x
                            WHERE x.review_id = r.id AND x.role = 'source'
                        )
                        OR s.expected_return_count != (
                            SELECT COUNT(*)
                            FROM custody_gap_review_transactions x
                            WHERE x.review_id = r.id AND x.role = 'return'
                        )
                    )
              )
          )
        LIMIT 1
        """
    ).fetchone()
    if pending is None:
        return 0
    from .core.custody_gap_reviews import (
        backfill_legacy_componentless_review_relations,
    )

    return backfill_legacy_componentless_review_relations(conn)


def _custody_replicable_detail_hash(payload_json):
    payload = json.loads(payload_json)
    if not isinstance(payload, dict):
        raise ValueError("custody evidence payload is invalid")
    # Keep migration backfills byte-for-byte equivalent to new activation
    # commitments. Observer lifecycle enrichment (mempool -> confirmed,
    # reorg position, improved timestamps, richer raw graph detail) is retained
    # in the immutable local snapshot but is not an ownership contradiction.
    for volatile_key in (
        "fingerprint",
        "occurred_at",
        "confirmed_at",
        "raw_json",
    ):
        payload.pop(volatile_key, None)
    encoded = json.dumps(
        payload,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    )
    return hashlib.sha256(encoded.encode("utf-8")).hexdigest()


def _backfill_local_custody_evidence_commitments(conn):
    """Migrate only evidence that an older local activation already bound.

    Current transaction rows are deliberately never consulted here.  A
    received component without the author's commitments must remain
    ineffective instead of being blessed by whatever this replica happens to
    know today.  A genuinely transactionless active component is the sole safe
    zero-evidence backfill.
    """

    migrated = 0
    rows = conn.execute(
        """
        SELECT c.id, c.workspace_id, c.profile_id, c.state, c.created_at
        FROM custody_components c
        WHERE c.expected_evidence_count IS NULL
        ORDER BY c.created_at, c.id
        """
    ).fetchall()
    for component in rows:
        snapshots = conn.execute(
            """
            SELECT quantity_hash, detail_hash, payload_json, created_at
            FROM custody_authored_evidence_snapshots
            WHERE profile_id = ?
              AND subject_kind = 'custody_component'
              AND subject_id = ?
            ORDER BY quantity_hash, detail_hash
            """,
            (component["profile_id"], component["id"]),
        ).fetchall()
        if snapshots:
            commitment_snapshots = sorted(
                (
                    snapshot["quantity_hash"],
                    _custody_replicable_detail_hash(snapshot["payload_json"]),
                    snapshot["created_at"],
                )
                for snapshot in snapshots
            )
            for ordinal, (quantity_hash, detail_hash, snapshot_created_at) in enumerate(
                commitment_snapshots
            ):
                conn.execute(
                    """
                    INSERT OR IGNORE INTO custody_component_evidence_commitments(
                        id, component_id, workspace_id, profile_id, ordinal,
                        quantity_hash, detail_hash, created_at
                    ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
                    """,
                    (
                        _custody_evidence_commitment_id(component["id"], ordinal),
                        component["id"],
                        component["workspace_id"],
                        component["profile_id"],
                        ordinal,
                        quantity_hash,
                        detail_hash,
                        snapshot_created_at or component["created_at"],
                    ),
                )
            conn.execute(
                "UPDATE custody_components SET expected_evidence_count = ? WHERE id = ?",
                (len(commitment_snapshots), component["id"]),
            )
            migrated += 1
            continue
        if component["state"] != "active":
            continue
        anchored = conn.execute(
            """
            SELECT 1 FROM custody_component_legs
            WHERE component_id = ?
              AND COALESCE(anchor_transaction_id, transaction_id) IS NOT NULL
            LIMIT 1
            """,
            (component["id"],),
        ).fetchone()
        if anchored is None:
            conn.execute(
                "UPDATE custody_components SET expected_evidence_count = 0 WHERE id = ?",
                (component["id"],),
            )
            migrated += 1
    return migrated


def _record_custody_durable_evidence_migration(
    conn,
    *,
    anchor_column_present_before,
    anchored_legs_before,
    anchored_legs_after,
    evidence_column_present_before,
    evidence_commitments_before,
    evidence_commitments_after,
):
    """Persist one bounded, payload-free before/after migration report."""

    changes = [
        {
            "name": "durable_transaction_anchors",
            "before": {
                "column_present": bool(anchor_column_present_before),
                "anchored_leg_count": int(anchored_legs_before),
            },
            "after": {
                "column_present": True,
                "anchored_leg_count": int(anchored_legs_after),
            },
            "rows_changed": max(
                0, int(anchored_legs_after) - int(anchored_legs_before)
            ),
            "explanation": _CUSTODY_MIGRATION_EXPLANATIONS[
                "durable_transaction_anchors"
            ],
        },
        {
            "name": "payload_free_evidence_commitments",
            "before": {
                "header_column_present": bool(evidence_column_present_before),
                "commitment_count": int(evidence_commitments_before),
            },
            "after": {
                "header_column_present": True,
                "commitment_count": int(evidence_commitments_after),
            },
            "rows_changed": max(
                0,
                int(evidence_commitments_after)
                - int(evidence_commitments_before),
            ),
            "explanation": _CUSTODY_MIGRATION_EXPLANATIONS[
                "payload_free_evidence_commitments"
            ],
        },
    ]
    impact = {
        "schema_version": 1,
        "migration": CUSTODY_DURABLE_EVIDENCE_MIGRATION,
        "changes": changes,
    }
    conn.execute(
        """
        INSERT OR IGNORE INTO schema_migration_audits(
            id, migration_name, schema_version, impact_json, created_at
        ) VALUES(?, ?, 1, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        """,
        (
            CUSTODY_DURABLE_EVIDENCE_MIGRATION,
            CUSTODY_DURABLE_EVIDENCE_MIGRATION,
            json.dumps(impact, sort_keys=True, separators=(",", ":")),
        ),
    )


_LEGACY_OWNERSHIP_HISTORY_KEY = "ownership_history"
_POLICY_EPOCH_PRIVATE_FIELDS = frozenset(
    {
        "descriptor",
        "change_descriptor",
        "xpub",
        "script_types",
        "addresses",
        "blinding_key",
        "chain",
        "network",
        "samourai",
        "ownership_scan_to_index",
        "gap_limit",
        "synthesize_change",
    }
)


def _migrate_inline_ownership_history(conn) -> int:
    """Move retired private policies out of wallet config exactly once."""

    rows = conn.execute(
        "SELECT * FROM wallets WHERE config_json LIKE ?",
        (f'%"{_LEGACY_OWNERSHIP_HISTORY_KEY}"%',),
    ).fetchall()
    migrated = 0
    for wallet in rows:
        try:
            config = json.loads(wallet["config_json"] or "{}")
        except (TypeError, ValueError, json.JSONDecodeError):
            continue
        if not isinstance(config, dict):
            continue
        history = config.pop(_LEGACY_OWNERSHIP_HISTORY_KEY, None)
        if not isinstance(history, list):
            continue
        existing = {
            str(row["private_material_json"])
            for row in conn.execute(
                "SELECT private_material_json FROM wallet_policy_epochs "
                "WHERE wallet_id = ? AND status = 'retired'",
                (wallet["id"],),
            ).fetchall()
        }
        for item in history:
            if not isinstance(item, dict):
                continue
            material = {
                key: item[key]
                for key in sorted(_POLICY_EPOCH_PRIVATE_FIELDS)
                if item.get(key) not in (None, "", [])
            }
            legacy_scan_to = item.get("scan_to_index")
            if legacy_scan_to not in (None, ""):
                try:
                    material["ownership_scan_to_index"] = max(
                        int(material.get("ownership_scan_to_index") or 0),
                        int(legacy_scan_to),
                    )
                except (TypeError, ValueError):
                    pass
            if not material:
                continue
            material_json = json.dumps(material, sort_keys=True)
            if material_json in existing:
                continue
            try:
                chain = normalize_chain(material.get("chain") or config.get("chain"))
                network = normalize_network(
                    chain,
                    material.get("network") or config.get("network"),
                )
            except ValueError:
                chain = "bitcoin"
                network = "main"
            conn.execute(
                """
                INSERT INTO wallet_policy_epochs(
                    id, workspace_id, profile_id, wallet_id, chain, network,
                    status, private_material_json, created_at, retired_at
                ) VALUES(?, ?, ?, ?, ?, ?, 'retired', ?, ?, ?)
                """,
                (
                    str(uuid.uuid4()),
                    wallet["workspace_id"],
                    wallet["profile_id"],
                    wallet["id"],
                    chain,
                    network,
                    material_json,
                    wallet["created_at"],
                    wallet["created_at"],
                ),
            )
            existing.add(material_json)
        conn.execute(
            "UPDATE wallets SET config_json = ? WHERE id = ?",
            (json.dumps(config, sort_keys=True), wallet["id"]),
        )
        migrated += 1
    return migrated


def ensure_schema_compat(conn):
    """Apply one-shot backfills not covered by `CREATE TABLE IF NOT EXISTS`.

    Anything added after the initial schema shipped belongs here so
    existing databases pick it up on the next `open_db`.
    """
    migrated_ownership_history = _migrate_inline_ownership_history(conn)
    migrated_source_links = conn.execute(
        "UPDATE source_funds_links SET method = 'custody_component' "
        "WHERE method IN ('transaction_pair', 'same_onchain_scope', "
        "'utxo_spend', 'payment_hash')"
    ).rowcount
    if migrated_ownership_history or migrated_source_links:
        conn.commit()
    # Derived gap pages and normalized candidate caches carry no authored
    # evidence or rollback history. Preserve only the latest journal builder's
    # ignored-boundary inputs, then remove every cached candidate table.
    conn.execute("DROP TABLE IF EXISTS custody_gap_candidate_snapshots")
    conn.execute("DROP TABLE IF EXISTS custody_gap_projection_rows")
    if conn.execute(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' "
        "AND name = 'custody_gap_candidate_projections'"
    ).fetchone():
        for row in conn.execute(
            """
            SELECT projection.profile_id, profile.workspace_id,
                   projection.version_json, projection.ignored_ids_json,
                   projection.accounting_ignored_ids_json, projection.created_at
            FROM custody_gap_candidate_projections projection
            JOIN profiles profile ON profile.id = projection.profile_id
            WHERE projection.producer_kind = 'journal'
              AND projection.rowid = (
                  SELECT MAX(candidate.rowid)
                  FROM custody_gap_candidate_projections candidate
                  WHERE candidate.profile_id = projection.profile_id
                    AND candidate.producer_kind = 'journal'
              )
            """
        ).fetchall():
            try:
                version = json.loads(row["version_json"])
                input_version = int(version[-1]) if version else 0
            except (TypeError, ValueError, json.JSONDecodeError):
                continue
            conn.execute(
                """
                INSERT OR REPLACE INTO journal_custody_gap_inputs(
                    workspace_id, profile_id, input_version, ignored_ids_json,
                    accounting_ignored_ids_json, created_at
                ) VALUES(?, ?, ?, ?, ?, ?)
                """,
                (
                    row["workspace_id"],
                    row["profile_id"],
                    input_version,
                    row["ignored_ids_json"],
                    row["accounting_ignored_ids_json"],
                    row["created_at"],
                ),
            )
    for table in (
        "custody_gap_candidate_boundaries",
        "custody_gap_candidates",
        "custody_gap_candidate_projections",
    ):
        conn.execute(f"DROP TABLE IF EXISTS {table}")
    # The custody tax-cutover audit recorded the engine's own first rebuild as
    # a "legacy" baseline on fresh books. The module is deleted; remove its
    # local-only tables (their immutability triggers drop with them).
    for table in (
        "custody_tax_migration_baseline_events",
        "custody_tax_migration_baselines",
        "custody_tax_migration_reports",
    ):
        conn.execute(f"DROP TABLE IF EXISTS {table}")
    ensure_column(
        conn,
        "chain_observer_instances",
        "state_epoch",
        "INTEGER NOT NULL DEFAULT 0",
    )
    ensure_column(
        conn,
        "filed_report_snapshots",
        "report_scope_json",
        "TEXT NOT NULL DEFAULT '{}'",
    )
    ensure_column(
        conn,
        "custody_gap_reviews",
        "authored_source",
        "TEXT NOT NULL DEFAULT 'user'",
    )
    ensure_column(
        conn,
        "custody_gap_reviews",
        "event_kind",
        "TEXT NOT NULL DEFAULT 'review_decision'",
    )
    ensure_column(conn, "profiles", "tax_country", f"TEXT NOT NULL DEFAULT '{DEFAULT_TAX_COUNTRY}'")
    ensure_column(conn, "profiles", "tax_long_term_days", f"INTEGER NOT NULL DEFAULT {DEFAULT_LONG_TERM_DAYS}")
    ensure_column(conn, "profiles", "require_coarse_review", "INTEGER NOT NULL DEFAULT 0")
    ensure_column(conn, "profiles", "bitcoin_rail_carrying_value", "INTEGER NOT NULL DEFAULT 1")
    ensure_column(conn, "profiles", "journal_input_version", "INTEGER NOT NULL DEFAULT 0")
    ensure_column(conn, "profiles", "last_processed_input_version", "INTEGER NOT NULL DEFAULT 0")
    # Cached count of unresolved swap/transfer candidates, written when the
    # matcher runs during journal processing, surfaced as a side-nav hint.
    # NULL = never computed (no badge).
    ensure_column(conn, "profiles", "swap_candidate_count", "INTEGER")
    # Cached pairable ownership-proof counts, written by journal processing.
    # Report blockers read this instead of rebuilding every wallet descriptor
    # index on the daemon's serial request loop.
    ownership_review_cache_added = _ensure_column_no_commit(
        conn, "profiles", "ownership_review_counts_json", "TEXT"
    )
    if ownership_review_cache_added:
        # Existing books can already contain ownership quarantines created by
        # an older build. Force just those books through journal processing so
        # the new cache is populated instead of silently hiding their blocker.
        conn.execute(
            """
            UPDATE profiles
            SET last_processed_at = NULL,
                last_processed_tx_count = 0,
                journal_input_version = journal_input_version + 1
            WHERE EXISTS (
                SELECT 1
                FROM journal_quarantines q
                WHERE q.profile_id = profiles.id
                  AND q.reason IN (
                    'ownership_transfer_destination_ambiguous',
                    'ownership_transfer_source_ambiguous',
                    'owned_fanout_unresolved'
                  )
            )
            """
        )
        conn.commit()
    ensure_column(conn, "backends", "batch_size", "INTEGER")
    ensure_column(conn, "backends", "config_json", "TEXT NOT NULL DEFAULT '{}'")
    ensure_column(conn, "journal_entries", "at_category", "TEXT")
    ensure_column(conn, "journal_entries", "at_kennzahl", "INTEGER")
    ensure_column(conn, "journal_entries", "capital_gains_type", "TEXT")
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
    conn.execute(
        """
        CREATE INDEX IF NOT EXISTS idx_transactions_wallet_pricing_ref
            ON transactions(wallet_id, pricing_external_ref, direction, asset, amount, created_at)
            WHERE pricing_external_ref IS NOT NULL
        """
    )
    ensure_column(conn, "transactions", "commercial_applied_link_id", "TEXT")
    ensure_column(conn, "transactions", "review_status", "TEXT")
    ensure_column(conn, "transactions", "taxability_override", "INTEGER")
    ensure_column(conn, "transactions", "at_regime_override", "TEXT")
    ensure_column(conn, "transactions", "at_category_override", "TEXT")
    ensure_column(conn, "transactions", "privacy_boundary", "TEXT")
    ensure_column(conn, "transaction_edit_events", "sync_event_id", "TEXT")
    ensure_column(conn, "transaction_edit_events", "sync_replica_id", "TEXT")
    ensure_column(conn, "transaction_edit_events", "sync_replica_seq", "INTEGER")
    ensure_column(conn, "transaction_edit_events", "sync_hlc", "TEXT")
    ensure_column(conn, "transaction_edit_events", "sync_author_member_id", "TEXT")
    ensure_column(conn, "transaction_edit_events", "sync_signature", "TEXT")
    ensure_column(conn, "transaction_edit_events", "sync_context_json", "TEXT")
    ensure_column(conn, "sync_members", "revoked_context_json", "TEXT")
    ensure_column(conn, "sync_devices", "revoked_context_json", "TEXT")
    ensure_column(conn, "sync_devices", "record_signer_member_id", "TEXT")
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_transaction_edit_events_sync_event "
        "ON transaction_edit_events(sync_event_id) WHERE sync_event_id IS NOT NULL"
    )
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_transaction_edit_events_sync_replica_seq "
        "ON transaction_edit_events(sync_replica_id, sync_replica_seq) "
        "WHERE sync_replica_id IS NOT NULL"
    )
    ensure_column(conn, "journal_entries", "fiat_value_exact", "TEXT")
    ensure_column(conn, "journal_entries", "unit_cost_exact", "TEXT")
    ensure_column(conn, "journal_entries", "cost_basis_exact", "TEXT")
    ensure_column(conn, "journal_entries", "proceeds_exact", "TEXT")
    ensure_column(conn, "journal_entries", "gain_loss_exact", "TEXT")
    ensure_column(conn, "journal_entries", "pricing_source_kind", "TEXT")
    ensure_column(conn, "journal_entries", "pricing_quality", "TEXT")
    ensure_column(conn, "journal_tax_summary", "capital_gains_type", "TEXT")
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
    ensure_column(conn, "wallet_utxos", "anonymity_score", "INTEGER")
    ensure_column(conn, "wallet_utxos", "script_pubkey", "TEXT")
    ensure_column(conn, "wallet_utxos", "spent_by", "TEXT")
    ensure_column(conn, "wallet_utxos", "excluded_from_coinjoin", "INTEGER")
    ensure_column(conn, "wallet_utxos", "key_state", "TEXT")
    ensure_column(conn, "wallet_utxos", "anon_history_json", "TEXT NOT NULL DEFAULT '[]'")
    ensure_column(conn, "loan_legs", "loan_id", "TEXT")
    ensure_column(conn, "ai_providers", "display_name", "TEXT")
    conn.execute(
        "UPDATE ai_providers SET display_name = name WHERE display_name IS NULL OR TRIM(display_name) = ''"
    )
    conn.commit()
    _ensure_ai_provider_secret_refs_schema(conn)
    _ensure_bip329_wallet_agnostic_schema(conn)
    _drop_legacy_source_funds_recipients_unique(conn)
    _migrate_msat_columns(conn)
    # Added after the msat rebuild, whose fixed column list would otherwise drop
    # it on a legacy REAL-typed database.
    ensure_column(conn, "transactions", "amount_includes_fee", "INTEGER NOT NULL DEFAULT 0")
    ensure_column(
        conn,
        "transactions",
        "external_id_kind",
        "TEXT CHECK(external_id_kind IS NULL OR external_id_kind = 'txid')",
    )
    ensure_column(
        conn,
        "journal_custody_decisions",
        "source_network",
        "TEXT NOT NULL DEFAULT 'unknown'",
    )
    ensure_column(
        conn,
        "journal_custody_decisions",
        "target_network",
        "TEXT NOT NULL DEFAULT 'unknown'",
    )
    ensure_column(
        conn,
        "journal_custody_decisions",
        "source_rail",
        "TEXT NOT NULL DEFAULT 'unknown'",
    )
    ensure_column(
        conn,
        "journal_custody_decisions",
        "target_rail",
        "TEXT NOT NULL DEFAULT 'unknown'",
    )
    if _ensure_custody_projection_table_shapes(conn):
        conn.commit()
    _ensure_custody_projection_relation_view(conn)
    _migrate_attachment_table_shape(conn)
    ensure_column(conn, "attachments", "copied_from_attachment_id", "TEXT")
    ensure_column(conn, "attachments", "copied_from_transaction_id", "TEXT")
    _backfill_liquid_asset_codes(conn)
    _ensure_swap_matching_schema(conn)
    _ensure_direct_swap_payout_schema(conn)
    if _ensure_custody_economic_term_review_notes(conn):
        conn.commit()
    _ensure_legacy_custody_write_freeze_triggers(conn)
    legacy_leg_columns = {
        row["name"]
        for row in conn.execute("PRAGMA table_info(custody_component_legs)")
    }
    legacy_component_columns = {
        row["name"]
        for row in conn.execute("PRAGMA table_info(custody_components)")
    }
    anchor_column_present_before = "anchor_transaction_id" in legacy_leg_columns
    evidence_column_present_before = (
        "expected_evidence_count" in legacy_component_columns
    )
    anchored_legs_before = (
        int(
            conn.execute(
                "SELECT COUNT(*) FROM custody_component_legs "
                "WHERE anchor_transaction_id IS NOT NULL"
            ).fetchone()[0]
        )
        if anchor_column_present_before
        else 0
    )
    evidence_commitments_before = int(
        conn.execute(
            "SELECT COUNT(*) FROM custody_component_evidence_commitments"
        ).fetchone()[0]
    )
    # Evidence migration below inspects each leg's durable anchor to
    # distinguish a genuinely transactionless active component from one whose
    # author evidence is missing.  Older custody schemas do not have that
    # column, so establish and populate it before running the backfill.
    ensure_column(conn, "custody_component_legs", "occurred_at", "TEXT")
    added_anchor_column = _ensure_column_no_commit(
        conn, "custody_component_legs", "anchor_transaction_id", "TEXT"
    )
    conn.execute(
        "UPDATE custody_component_legs "
        "SET anchor_transaction_id = transaction_id "
        "WHERE anchor_transaction_id IS NULL AND transaction_id IS NOT NULL"
    )
    added_leg_commitment = _ensure_column_no_commit(
        conn, "custody_components", "expected_leg_count", "INTEGER"
    )
    added_allocation_commitment = _ensure_column_no_commit(
        conn, "custody_components", "expected_allocation_count", "INTEGER"
    )
    added_economic_term_commitment = _ensure_column_no_commit(
        conn, "custody_components", "expected_economic_term_count", "INTEGER"
    )
    added_evidence_commitment = _ensure_column_no_commit(
        conn, "custody_components", "expected_evidence_count", "INTEGER"
    )
    added_authored_source = _ensure_column_no_commit(
        conn, "custody_components", "authored_source", "TEXT DEFAULT 'user'"
    )
    if added_leg_commitment:
        conn.execute(
            "UPDATE custody_components SET expected_leg_count = "
            "(SELECT COUNT(*) FROM custody_component_legs l "
            " WHERE l.component_id = custody_components.id) "
            "WHERE expected_leg_count IS NULL"
        )
    if added_allocation_commitment:
        conn.execute(
            "UPDATE custody_components SET expected_allocation_count = "
            "(SELECT COUNT(*) FROM custody_component_allocations a "
            " WHERE a.component_id = custody_components.id) "
            "WHERE expected_allocation_count IS NULL"
        )
    if added_economic_term_commitment:
        conn.execute(
            "UPDATE custody_components SET expected_economic_term_count = "
            "(SELECT COUNT(*) FROM custody_component_economic_terms terms "
            " WHERE terms.component_id = custody_components.id) "
            "WHERE expected_economic_term_count IS NULL"
        )
    if added_authored_source:
        conn.execute(
            "UPDATE custody_components SET authored_source = 'user' "
            "WHERE authored_source IS NULL OR authored_source = ''"
        )
    migrated_evidence_commitments = _backfill_local_custody_evidence_commitments(conn)
    anchored_legs_after = int(
        conn.execute(
            "SELECT COUNT(*) FROM custody_component_legs "
            "WHERE anchor_transaction_id IS NOT NULL"
        ).fetchone()[0]
    )
    evidence_commitments_after = int(
        conn.execute(
            "SELECT COUNT(*) FROM custody_component_evidence_commitments"
        ).fetchone()[0]
    )
    if added_anchor_column or added_evidence_commitment or migrated_evidence_commitments:
        _record_custody_durable_evidence_migration(
            conn,
            anchor_column_present_before=anchor_column_present_before,
            anchored_legs_before=anchored_legs_before,
            anchored_legs_after=anchored_legs_after,
            evidence_column_present_before=evidence_column_present_before,
            evidence_commitments_before=evidence_commitments_before,
            evidence_commitments_after=evidence_commitments_after,
        )
    if (
        added_anchor_column
        or added_leg_commitment
        or added_allocation_commitment
        or added_economic_term_commitment
        or added_evidence_commitment
        or added_authored_source
        or migrated_evidence_commitments
    ):
        conn.commit()
    _ensure_custody_leg_role_schema(conn)
    if _ensure_custody_gap_review_transaction_schema(conn):
        conn.commit()
    if _backfill_custody_gap_review_transactions(conn):
        conn.commit()
    if _backfill_legacy_componentless_review_transactions(conn):
        conn.commit()
    _ensure_custody_revision_immutability_triggers(conn)
    # Converge legacy reviewed pairs/payouts into immutable draft component
    # aggregates without changing which substrate currently drives journals.
    # The import is local to keep the core module independent from schema
    # bootstrap and to avoid a module-level db -> core -> db cycle.
    from .core.custody_authored_migration import (
        refresh_legacy_authored_components,
    )

    migration = refresh_legacy_authored_components(conn)
    if migration.changed:
        conn.commit()
    _ensure_commercial_reconciliation_schema(conn)
    _ensure_freshness_schema(conn)
    _ensure_transaction_graph_cache_schema(conn)


def _ensure_custody_economic_term_review_notes(conn):
    """Add immutable per-review notes and backfill them before reader cutover."""

    columns = {
        row["name"]
        for row in conn.execute(
            "PRAGMA table_info(custody_component_economic_terms)"
        ).fetchall()
    }
    if "review_notes" in columns:
        return False
    conn.execute(
        "ALTER TABLE custody_component_economic_terms ADD COLUMN review_notes TEXT"
    )
    # The only update to an authored term is this deterministic schema
    # migration from the source row already committed by source_row_hash.
    conn.execute("DROP TRIGGER IF EXISTS trg_custody_component_terms_immutable")
    conn.execute(
        """
        UPDATE custody_component_economic_terms AS term
        SET review_notes = CASE term.term_kind
            WHEN 'transaction_pair' THEN (
                SELECT pair.notes
                FROM transaction_pairs pair
                WHERE pair.profile_id = term.profile_id
                  AND pair.id = term.legacy_source_id
            )
            WHEN 'direct_swap_payout' THEN (
                SELECT payout.notes
                FROM direct_swap_payouts payout
                WHERE payout.profile_id = term.profile_id
                  AND payout.id = term.legacy_source_id
            )
        END
        """
    )
    conn.execute(
        """
        CREATE TRIGGER trg_custody_component_terms_immutable
        BEFORE UPDATE ON custody_component_economic_terms
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_terms_immutable');
        END
        """
    )
    return True


def _ensure_custody_projection_relation_view(conn):
    """Expose one normalized read seam over the two derived custody row shapes."""

    conn.execute("DROP VIEW IF EXISTS journal_custody_projection_relations")
    conn.execute(
        """
        CREATE VIEW journal_custody_projection_relations AS
        SELECT
            decision.profile_id,
            decision.decision_id AS id,
            'move' AS relation_kind,
            COALESCE(term.review_kind, component.component_type,
                     decision.reason) AS kind,
            COALESCE(term.tax_policy, component.conversion_policy,
                     'carrying-value') AS policy,
            term.swap_fee_msat,
            term.swap_fee_kind,
            term.confidence_at_review AS confidence_at_pair,
            COALESCE(term.review_source, component.authored_source,
                     'journal_builder') AS pair_source,
            COALESCE(term.review_notes, component.notes) AS notes,
            NULL AS target_external_id,
            NULL AS counterparty,
            NULL AS target_fiat_value_exact,
            decision.source_transaction_id AS out_transaction_id,
            decision.target_transaction_id AS in_transaction_id,
            decision.source_end_msat - decision.source_start_msat AS out_amount,
            decision.target_end_msat - decision.target_start_msat AS in_amount,
            decision.source_asset AS out_asset,
            decision.target_asset AS in_asset,
            decision.source_rail,
            decision.target_rail,
            decision.basis_state,
            decision.component_id,
            decision.occurred_at,
            decision.target_occurred_at,
            decision.created_at
        FROM journal_custody_decisions decision
        LEFT JOIN custody_components component
          ON component.id = decision.component_id
        LEFT JOIN custody_component_economic_terms term
          ON term.id = (
            SELECT candidate.id
            FROM custody_component_economic_terms candidate
            JOIN custody_component_legs source_leg
              ON source_leg.id = candidate.source_leg_id
            JOIN custody_component_legs target_leg
              ON target_leg.id = candidate.target_leg_id
            WHERE candidate.component_id = decision.component_id
              AND COALESCE(source_leg.anchor_transaction_id,
                           source_leg.transaction_id) =
                    decision.source_transaction_id
              AND COALESCE(target_leg.anchor_transaction_id,
                           target_leg.transaction_id) =
                    decision.target_transaction_id
            ORDER BY candidate.ordinal, candidate.id
            LIMIT 1
          )
        UNION ALL
        SELECT
            relation.profile_id,
            relation.relation_id AS id,
            relation.relation_kind,
            COALESCE(term.review_kind, component.component_type,
                     relation.relation_kind) AS kind,
            COALESCE(term.tax_policy, component.conversion_policy,
                     'taxable') AS policy,
            term.swap_fee_msat,
            term.swap_fee_kind,
            term.confidence_at_review AS confidence_at_pair,
            COALESCE(term.review_source, component.authored_source,
                     'journal_builder') AS pair_source,
            COALESCE(term.review_notes, component.notes) AS notes,
            term.payout_external_id AS target_external_id,
            term.counterparty,
            term.payout_fiat_value_exact AS target_fiat_value_exact,
            relation.source_transaction_id AS out_transaction_id,
            relation.target_transaction_id AS in_transaction_id,
            relation.source_amount_msat AS out_amount,
            relation.target_amount_msat AS in_amount,
            relation.source_asset AS out_asset,
            relation.target_asset AS in_asset,
            NULL AS source_rail,
            NULL AS target_rail,
            relation.basis_state,
            relation.component_id,
            relation.occurred_at,
            relation.target_occurred_at,
            relation.created_at
        FROM journal_custody_economic_relations relation
        LEFT JOIN custody_components component
          ON component.id = relation.component_id
        LEFT JOIN custody_component_economic_terms term
          ON term.id = (
            SELECT candidate.id
            FROM custody_component_economic_terms candidate
            JOIN custody_component_legs source_leg
              ON source_leg.id = candidate.source_leg_id
            JOIN custody_component_legs target_leg
              ON target_leg.id = candidate.target_leg_id
            WHERE candidate.component_id = relation.component_id
              AND COALESCE(source_leg.anchor_transaction_id,
                           source_leg.transaction_id) =
                    relation.source_transaction_id
              AND (
                relation.target_transaction_id IS NULL
                OR COALESCE(target_leg.anchor_transaction_id,
                            target_leg.transaction_id) =
                     relation.target_transaction_id
              )
              AND candidate.term_kind = CASE relation.relation_kind
                    WHEN 'direct_payout' THEN 'direct_swap_payout'
                    ELSE 'transaction_pair'
                  END
            ORDER BY candidate.ordinal, candidate.id
            LIMIT 1
          )
        """
    )


def _ensure_custody_projection_table_shapes(conn):
    """Drop copied review semantics from rebuildable local journal tables."""

    semantic_columns = {
        "journal_custody_decisions": (
            "review_kind",
            "policy",
            "confidence_at_review",
            "review_source",
            "notes",
            "swap_fee_msat",
            "swap_fee_kind",
        ),
        "journal_custody_economic_relations": (
            "review_kind",
            "policy",
            "swap_fee_msat",
            "swap_fee_kind",
            "notes",
            "confidence_at_review",
            "review_source",
            "target_external_id",
            "counterparty",
            "target_fiat_value_exact",
        ),
    }
    existing_columns = {
        table: {
            row["name"]
            for row in conn.execute(f"PRAGMA table_info({table})")
        }
        for table in semantic_columns
    }
    pending = [
        (table, column)
        for table, candidates in semantic_columns.items()
        for column in candidates
        if column in existing_columns[table]
    ]
    if not pending:
        return False

    conn.execute("SAVEPOINT custody_projection_table_shapes")
    try:
        conn.execute("DROP VIEW IF EXISTS journal_custody_projection_relations")
        for table, column in pending:
            conn.execute(f'ALTER TABLE "{table}" DROP COLUMN "{column}"')
    except Exception:
        conn.execute("ROLLBACK TO SAVEPOINT custody_projection_table_shapes")
        conn.execute("RELEASE SAVEPOINT custody_projection_table_shapes")
        raise
    conn.execute("RELEASE SAVEPOINT custody_projection_table_shapes")
    return True


def _ensure_custody_revision_immutability_triggers(conn):
    """Keep authored custody economics append-only below every API surface.

    Component lifecycle columns are intentionally mutable. Transaction and
    wallet FKs on legs may be cleared by ``ON DELETE SET NULL``; an exact
    transaction anchor may reconnect if the same transaction row is restored.
    All other changes require a newly-authored component revision.
    """

    for trigger in (
        "trg_custody_component_revision_immutable",
        "trg_custody_component_leg_revision_immutable",
        "trg_custody_component_allocation_revision_immutable",
        "trg_custody_component_revision_delete_immutable",
        "trg_custody_component_leg_revision_delete_immutable",
        "trg_custody_component_allocation_revision_delete_immutable",
        "trg_custody_component_evidence_revision_immutable",
        "trg_custody_component_evidence_revision_delete_immutable",
        "trg_custody_component_leg_count_commitment",
        "trg_custody_component_allocation_count_commitment",
        "trg_custody_component_economic_term_count_commitment",
        "trg_custody_component_evidence_count_commitment",
    ):
        conn.execute(f"DROP TRIGGER IF EXISTS {trigger}")
    conn.execute(
        """
        CREATE TRIGGER trg_custody_component_revision_immutable
        BEFORE UPDATE ON custody_components
        WHEN OLD.id IS NOT NEW.id
          OR OLD.lineage_id IS NOT NEW.lineage_id
          OR OLD.workspace_id IS NOT NEW.workspace_id
          OR OLD.profile_id IS NOT NEW.profile_id
          OR OLD.revision IS NOT NEW.revision
          OR OLD.component_type IS NOT NEW.component_type
          OR OLD.conservation_mode IS NOT NEW.conservation_mode
          OR OLD.evidence_kind IS NOT NEW.evidence_kind
          OR OLD.evidence_grade IS NOT NEW.evidence_grade
          OR OLD.evidence_json IS NOT NEW.evidence_json
          OR OLD.conversion_policy IS NOT NEW.conversion_policy
          OR OLD.conversion_reviewed IS NOT NEW.conversion_reviewed
          OR OLD.conversion_metadata_json IS NOT NEW.conversion_metadata_json
          OR OLD.expected_leg_count IS NOT NEW.expected_leg_count
          OR OLD.expected_allocation_count IS NOT NEW.expected_allocation_count
          OR OLD.expected_economic_term_count IS NOT NEW.expected_economic_term_count
          OR (
              OLD.expected_evidence_count IS NOT NEW.expected_evidence_count
              AND NOT (
                  OLD.expected_evidence_count IS NULL
                  AND NEW.expected_evidence_count IS NOT NULL
                  AND NEW.expected_evidence_count >= 0
                  AND (
                      (OLD.state IN ('active', 'superseded') AND OLD.state = NEW.state)
                      OR (OLD.state = 'draft' AND NEW.state = 'active')
                  )
              )
          )
          OR OLD.authored_source IS NOT NEW.authored_source
          OR OLD.notes IS NOT NEW.notes
          OR OLD.supersedes_component_id IS NOT NEW.supersedes_component_id
          OR OLD.created_at IS NOT NEW.created_at
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_revision_immutable');
        END
        """
    )
    conn.execute(
        """
        CREATE TRIGGER trg_custody_component_leg_revision_immutable
        BEFORE UPDATE ON custody_component_legs
        WHEN OLD.id IS NOT NEW.id
          OR OLD.component_id IS NOT NEW.component_id
          OR OLD.workspace_id IS NOT NEW.workspace_id
          OR OLD.profile_id IS NOT NEW.profile_id
          OR OLD.ordinal IS NOT NEW.ordinal
          OR OLD.role IS NOT NEW.role
          OR OLD.rail IS NOT NEW.rail
          OR OLD.chain IS NOT NEW.chain
          OR OLD.network IS NOT NEW.network
          OR OLD.asset IS NOT NEW.asset
          OR OLD.exposure IS NOT NEW.exposure
          OR OLD.conservation_unit IS NOT NEW.conservation_unit
          OR OLD.amount_msat IS NOT NEW.amount_msat
          OR OLD.valuation_unit IS NOT NEW.valuation_unit
          OR OLD.valuation_amount IS NOT NEW.valuation_amount
          OR OLD.occurred_at IS NOT NEW.occurred_at
          OR OLD.anchor_transaction_id IS NOT NEW.anchor_transaction_id
          OR OLD.location_ref IS NOT NEW.location_ref
          OR OLD.notes IS NOT NEW.notes
          OR OLD.created_at IS NOT NEW.created_at
          OR NOT (
              OLD.transaction_id IS NEW.transaction_id
              OR NEW.transaction_id IS NULL
              OR (
                  OLD.transaction_id IS NULL
                  AND NEW.transaction_id IS OLD.anchor_transaction_id
              )
          )
          OR NOT (
              OLD.wallet_id IS NEW.wallet_id
              OR NEW.wallet_id IS NULL
          )
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_leg_revision_immutable');
        END
        """
    )
    conn.execute(
        """
        CREATE TRIGGER trg_custody_component_evidence_revision_immutable
        BEFORE UPDATE ON custody_component_evidence_commitments
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_evidence_revision_immutable');
        END
        """
    )
    conn.execute(
        """
        CREATE TRIGGER trg_custody_component_revision_delete_immutable
        BEFORE DELETE ON custody_components
        WHEN EXISTS (
            SELECT 1 FROM profiles p WHERE p.id = OLD.profile_id
        )
        AND NOT EXISTS (
            SELECT 1 FROM custody_component_purge_authorizations authorization
            WHERE authorization.profile_id = OLD.profile_id
        )
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_revision_delete_immutable');
        END
        """
    )
    conn.execute(
        """
        CREATE TRIGGER trg_custody_component_leg_revision_delete_immutable
        BEFORE DELETE ON custody_component_legs
        WHEN EXISTS (
            SELECT 1 FROM profiles p WHERE p.id = OLD.profile_id
        )
        AND NOT EXISTS (
            SELECT 1 FROM custody_component_purge_authorizations authorization
            WHERE authorization.profile_id = OLD.profile_id
        )
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_leg_revision_delete_immutable');
        END
        """
    )
    conn.execute(
        """
        CREATE TRIGGER trg_custody_component_allocation_revision_delete_immutable
        BEFORE DELETE ON custody_component_allocations
        WHEN EXISTS (
            SELECT 1 FROM profiles p WHERE p.id = OLD.profile_id
        )
        AND NOT EXISTS (
            SELECT 1 FROM custody_component_purge_authorizations authorization
            WHERE authorization.profile_id = OLD.profile_id
        )
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_allocation_revision_delete_immutable');
        END
        """
    )
    conn.execute(
        """
        CREATE TRIGGER trg_custody_component_evidence_revision_delete_immutable
        BEFORE DELETE ON custody_component_evidence_commitments
        WHEN EXISTS (
            SELECT 1 FROM profiles p WHERE p.id = OLD.profile_id
        )
        AND NOT EXISTS (
            SELECT 1 FROM custody_component_purge_authorizations authorization
            WHERE authorization.profile_id = OLD.profile_id
        )
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_evidence_revision_delete_immutable');
        END
        """
    )
    conn.execute(
        """
        CREATE TRIGGER trg_custody_component_leg_count_commitment
        BEFORE INSERT ON custody_component_legs
        WHEN NOT EXISTS (
            SELECT 1 FROM custody_component_legs current WHERE current.id = NEW.id
        )
        AND
        (
            SELECT c.expected_leg_count FROM custody_components c
            WHERE c.id = NEW.component_id
        ) IS NOT NULL
        AND (
            SELECT COUNT(*) FROM custody_component_legs l
            WHERE l.component_id = NEW.component_id
        ) >= (
            SELECT c.expected_leg_count FROM custody_components c
            WHERE c.id = NEW.component_id
        )
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_leg_count_commitment');
        END
        """
    )
    conn.execute(
        """
        CREATE TRIGGER trg_custody_component_allocation_count_commitment
        BEFORE INSERT ON custody_component_allocations
        WHEN NOT EXISTS (
            SELECT 1 FROM custody_component_allocations current WHERE current.id = NEW.id
        )
        AND
        (
            SELECT c.expected_allocation_count FROM custody_components c
            WHERE c.id = NEW.component_id
        ) IS NOT NULL
        AND (
            SELECT COUNT(*) FROM custody_component_allocations a
            WHERE a.component_id = NEW.component_id
        ) >= (
            SELECT c.expected_allocation_count FROM custody_components c
            WHERE c.id = NEW.component_id
        )
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_allocation_count_commitment');
        END
        """
    )
    conn.execute(
        """
        CREATE TRIGGER trg_custody_component_economic_term_count_commitment
        BEFORE INSERT ON custody_component_economic_terms
        WHEN NOT EXISTS (
            SELECT 1 FROM custody_component_economic_terms current
            WHERE current.id = NEW.id
        )
        AND
        (
            SELECT c.expected_economic_term_count FROM custody_components c
            WHERE c.id = NEW.component_id
        ) IS NOT NULL
        AND (
            SELECT COUNT(*) FROM custody_component_economic_terms terms
            WHERE terms.component_id = NEW.component_id
        ) >= (
            SELECT c.expected_economic_term_count FROM custody_components c
            WHERE c.id = NEW.component_id
        )
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_economic_term_count_commitment');
        END
        """
    )
    conn.execute(
        """
        CREATE TRIGGER trg_custody_component_evidence_count_commitment
        BEFORE INSERT ON custody_component_evidence_commitments
        WHEN NOT EXISTS (
            SELECT 1 FROM custody_component_evidence_commitments current
            WHERE current.id = NEW.id
        )
        AND
        (
            SELECT c.expected_evidence_count FROM custody_components c
            WHERE c.id = NEW.component_id
        ) IS NOT NULL
        AND (
            SELECT COUNT(*) FROM custody_component_evidence_commitments evidence
            WHERE evidence.component_id = NEW.component_id
        ) >= (
            SELECT c.expected_evidence_count FROM custody_components c
            WHERE c.id = NEW.component_id
        )
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_evidence_count_commitment');
        END
        """
    )
    conn.execute(
        """
        CREATE TRIGGER trg_custody_component_allocation_revision_immutable
        BEFORE UPDATE ON custody_component_allocations
        WHEN OLD.id IS NOT NEW.id
          OR OLD.component_id IS NOT NEW.component_id
          OR OLD.workspace_id IS NOT NEW.workspace_id
          OR OLD.profile_id IS NOT NEW.profile_id
          OR OLD.ordinal IS NOT NEW.ordinal
          OR OLD.source_leg_id IS NOT NEW.source_leg_id
          OR OLD.sink_leg_id IS NOT NEW.sink_leg_id
          OR OLD.source_amount_msat IS NOT NEW.source_amount_msat
          OR OLD.sink_amount_msat IS NOT NEW.sink_amount_msat
          OR OLD.created_at IS NOT NEW.created_at
        BEGIN
            SELECT RAISE(ABORT, 'custody_component_allocation_revision_immutable');
        END
        """
    )


def _decode_json_object(raw_json):
    try:
        payload = json.loads(raw_json or "{}")
    except (TypeError, ValueError, json.JSONDecodeError):
        return {}
    return payload if isinstance(payload, dict) else {}


def _ensure_bip329_wallet_agnostic_schema(conn):
    groups = conn.execute(
        """
        SELECT profile_id, record_type, ref
        FROM bip329_labels
        GROUP BY profile_id, record_type, ref
        HAVING COUNT(*) > 1
        """
    ).fetchall()
    for group in groups:
        rows = conn.execute(
            """
            SELECT *
            FROM bip329_labels
            WHERE profile_id = ?
              AND record_type = ?
              AND ref = ?
            ORDER BY created_at ASC, id ASC
            """,
            (group["profile_id"], group["record_type"], group["ref"]),
        ).fetchall()
        if not rows:
            continue
        label = None
        origin = None
        spendable = None
        data = {}
        for row in rows:
            if row["label"] is not None:
                label = row["label"]
            if row["origin"] is not None:
                origin = row["origin"]
            if row["spendable"] is not None:
                spendable = row["spendable"]
            data.update(_decode_json_object(row["data_json"]))
        canonical = rows[-1]
        conn.execute(
            """
            UPDATE bip329_labels
            SET wallet_id = NULL,
                label = ?,
                origin = ?,
                spendable = ?,
                data_json = ?
            WHERE id = ?
            """,
            (label, origin, spendable, json.dumps(data, sort_keys=True), canonical["id"]),
        )
        conn.execute(
            """
            DELETE FROM bip329_labels
            WHERE profile_id = ?
              AND record_type = ?
              AND ref = ?
              AND id != ?
            """,
            (group["profile_id"], group["record_type"], group["ref"], canonical["id"]),
        )
    conn.execute("UPDATE bip329_labels SET wallet_id = NULL WHERE wallet_id IS NOT NULL")
    conn.execute(
        """
        CREATE UNIQUE INDEX IF NOT EXISTS idx_bip329_labels_profile_object
            ON bip329_labels(profile_id, record_type, ref)
        """
    )
    conn.execute(
        """
        CREATE INDEX IF NOT EXISTS idx_bip329_labels_profile_created
            ON bip329_labels(profile_id, created_at DESC, id DESC)
        """
    )
    conn.commit()


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


def _ensure_freshness_schema(conn):
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS freshness_source_states (
            profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
            source_key TEXT NOT NULL,
            source_type TEXT NOT NULL,
            source_label TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'fresh',
            state TEXT NOT NULL DEFAULT 'fresh',
            stale_reason TEXT,
            blocking_reports INTEGER NOT NULL DEFAULT 0,
            paused INTEGER NOT NULL DEFAULT 0,
            rate_limited_until TEXT,
            cooldown_reason TEXT,
            retry_count INTEGER NOT NULL DEFAULT 0,
            last_success_at TEXT,
            last_error_at TEXT,
            last_error_code TEXT,
            last_error_message TEXT,
            last_phase TEXT,
            progress_json TEXT NOT NULL DEFAULT '{}',
            checkpoint_json TEXT NOT NULL DEFAULT '{}',
            updated_at TEXT NOT NULL,
            PRIMARY KEY(profile_id, source_key)
        )
        """
    )
    conn.execute(
        """
        CREATE INDEX IF NOT EXISTS idx_freshness_source_states_profile_status
            ON freshness_source_states(profile_id, status, updated_at DESC)
        """
    )
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS freshness_jobs (
            id TEXT PRIMARY KEY,
            profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
            job_type TEXT NOT NULL,
            source_key TEXT NOT NULL,
            source_type TEXT NOT NULL,
            source_label TEXT NOT NULL,
            status TEXT NOT NULL,
            phase TEXT,
            priority INTEGER NOT NULL DEFAULT 100,
            payload_json TEXT NOT NULL DEFAULT '{}',
            progress_json TEXT NOT NULL DEFAULT '{}',
            result_json TEXT NOT NULL DEFAULT '{}',
            error_json TEXT NOT NULL DEFAULT '{}',
            attempts INTEGER NOT NULL DEFAULT 0,
            cancel_requested INTEGER NOT NULL DEFAULT 0,
            run_after TEXT,
            cooldown_until TEXT,
            created_at TEXT NOT NULL,
            started_at TEXT,
            finished_at TEXT,
            updated_at TEXT NOT NULL
        )
        """
    )
    conn.execute(
        """
        CREATE INDEX IF NOT EXISTS idx_freshness_jobs_profile_status
            ON freshness_jobs(profile_id, status, priority, created_at)
        """
    )
    conn.execute(
        """
        CREATE UNIQUE INDEX IF NOT EXISTS idx_freshness_jobs_singleflight
            ON freshness_jobs(profile_id, source_key, job_type)
            WHERE status IN ('queued', 'running', 'rate_limited')
        """
    )
    for table in ("freshness_source_states", "freshness_jobs"):
        for column, definition in (
            ("source_type", "TEXT NOT NULL DEFAULT 'source'"),
            ("source_label", "TEXT NOT NULL DEFAULT ''"),
            ("updated_at", "TEXT NOT NULL DEFAULT ''"),
        ):
            ensure_column(conn, table, column, definition)


def _ensure_transaction_graph_cache_schema(conn):
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS transaction_graph_cache (
            schema_version INTEGER NOT NULL,
            chain TEXT NOT NULL,
            network TEXT NOT NULL,
            txid TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (schema_version, chain, network, txid)
        )
        """
    )
    conn.execute(
        """
        CREATE INDEX IF NOT EXISTS idx_transaction_graph_cache_updated
            ON transaction_graph_cache(updated_at DESC)
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
            payment_request_id TEXT,
            origin_kind TEXT,
            origin_app_id TEXT,
            origin_label TEXT,
            origin_url TEXT,
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
    ensure_column(conn, "btcpay_provenance_records", "payment_request_id", "TEXT")
    ensure_column(conn, "btcpay_provenance_records", "origin_kind", "TEXT")
    ensure_column(conn, "btcpay_provenance_records", "origin_app_id", "TEXT")
    ensure_column(conn, "btcpay_provenance_records", "origin_label", "TEXT")
    ensure_column(conn, "btcpay_provenance_records", "origin_url", "TEXT")
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_btcpay_provenance_profile_payment_request "
        "ON btcpay_provenance_records(profile_id, payment_request_id) "
        "WHERE payment_request_id IS NOT NULL"
    )
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS btcpay_account_routes (
            id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
            backend_name TEXT NOT NULL,
            store_id TEXT NOT NULL,
            payment_method_id TEXT NOT NULL,
            action TEXT NOT NULL,
            label TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(profile_id, backend_name, store_id, payment_method_id, action)
        )
        """
    )
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_btcpay_account_routes_profile_backend "
        "ON btcpay_account_routes(profile_id, backend_name)"
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


def _migrate_attachment_table_shape(conn):
    table_sql = conn.execute(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='attachments'"
    ).fetchone()
    legacy_table = "attachments_legacy_shape"
    legacy_sql = conn.execute(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name=?",
        (legacy_table,),
    ).fetchone()
    if not legacy_sql:
        legacy_table = "attachments_legacy_notnull_tx"
        legacy_sql = conn.execute(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name=?",
            (legacy_table,),
        ).fetchone()
    current_sql = (table_sql[0] if table_sql else "") or ""
    copied_provenance_fk_columns = {
        row["from"] if hasattr(row, "keys") else row[3]
        for row in conn.execute("PRAGMA foreign_key_list(attachments)").fetchall()
        if (row["from"] if hasattr(row, "keys") else row[3])
        in {"copied_from_attachment_id", "copied_from_transaction_id"}
    }
    if (
        not legacy_sql
        and "transaction_id TEXT NOT NULL" not in current_sql
        and "label TEXT NOT NULL" not in current_sql
        and not copied_provenance_fk_columns
    ):
        _repair_attachment_child_fks(conn)
        return
    previous_fk_state = conn.execute("PRAGMA foreign_keys").fetchone()[0]
    previous_legacy_state = conn.execute("PRAGMA legacy_alter_table").fetchone()[0]
    conn.commit()
    conn.execute("PRAGMA foreign_keys = OFF")
    conn.execute("PRAGMA legacy_alter_table = ON")
    conn.execute("BEGIN IMMEDIATE")
    try:
        if not legacy_sql:
            conn.execute(f"ALTER TABLE attachments RENAME TO {legacy_table}")
        else:
            conn.execute("DROP TABLE IF EXISTS attachments")
        conn.execute(
            """
            CREATE TABLE attachments (
                id TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
                transaction_id TEXT REFERENCES transactions(id) ON DELETE CASCADE,
                attachment_type TEXT NOT NULL,
                label TEXT,
                original_filename TEXT,
                stored_relpath TEXT,
                source_url TEXT,
                media_type TEXT,
                size_bytes INTEGER,
                sha256 TEXT,
                copied_from_attachment_id TEXT,
                copied_from_transaction_id TEXT,
                created_at TEXT NOT NULL
            )
            """
        )
        _ensure_column_no_commit(
            conn,
            "attachments",
            "copied_from_attachment_id",
            "TEXT",
        )
        _ensure_column_no_commit(
            conn,
            "attachments",
            "copied_from_transaction_id",
            "TEXT",
        )
        legacy_columns = {
            row["name"] if hasattr(row, "keys") else row[1]
            for row in conn.execute(f"PRAGMA table_info({legacy_table})").fetchall()
        }
        copied_from_attachment_expr = (
            "copied_from_attachment_id"
            if "copied_from_attachment_id" in legacy_columns
            else "NULL"
        )
        copied_from_transaction_expr = (
            "copied_from_transaction_id"
            if "copied_from_transaction_id" in legacy_columns
            else "NULL"
        )
        conn.execute(
            f"""
            INSERT OR IGNORE INTO attachments(
                id, workspace_id, profile_id, transaction_id, attachment_type, label,
                original_filename, stored_relpath, source_url, media_type,
                size_bytes, sha256, copied_from_attachment_id,
                copied_from_transaction_id, created_at
            )
            SELECT id, workspace_id, profile_id, transaction_id, attachment_type,
                   CASE
                       WHEN attachment_type = 'url' AND label = source_url THEN NULL
                       ELSE label
                   END AS label,
                   original_filename, stored_relpath, source_url, media_type,
                   size_bytes, sha256, {copied_from_attachment_expr},
                   {copied_from_transaction_expr}, created_at
            FROM {legacy_table}
            """
        )
        conn.execute(f"DROP TABLE {legacy_table}")
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_attachments_profile_tx_created "
            "ON attachments(profile_id, transaction_id, created_at DESC)"
        )
        conn.execute("COMMIT")
    except Exception:
        conn.execute("ROLLBACK")
        raise
    finally:
        conn.execute(
            f"PRAGMA legacy_alter_table = {'ON' if previous_legacy_state else 'OFF'}"
        )
        conn.execute(f"PRAGMA foreign_keys = {'ON' if previous_fk_state else 'OFF'}")
    _repair_attachment_child_fks(conn)


def _attachment_fk_targets(conn, table_name):
    table_exists = conn.execute(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name = ?",
        (table_name,),
    ).fetchone()
    if not table_exists:
        return set()
    return {
        row["table"] if hasattr(row, "keys") else row[2]
        for row in conn.execute(f"PRAGMA foreign_key_list({table_name})").fetchall()
        if (row["from"] if hasattr(row, "keys") else row[3]) == "attachment_id"
    }


def _repair_attachment_child_fks(conn):
    """Repair child tables whose attachment FK was rewritten to a temp table."""

    child_tables = (
        (
            "source_funds_link_attachments",
            """
            CREATE TABLE source_funds_link_attachments (
                link_id TEXT NOT NULL REFERENCES source_funds_links(id) ON DELETE CASCADE,
                attachment_id TEXT NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
                created_at TEXT NOT NULL,
                PRIMARY KEY(link_id, attachment_id)
            )
            """,
            "link_id, attachment_id, created_at",
        ),
        (
            "source_funds_source_attachments",
            """
            CREATE TABLE source_funds_source_attachments (
                source_id TEXT NOT NULL REFERENCES source_funds_sources(id) ON DELETE CASCADE,
                attachment_id TEXT NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
                created_at TEXT NOT NULL,
                PRIMARY KEY(source_id, attachment_id)
            )
            """,
            "source_id, attachment_id, created_at",
        ),
        (
            "external_document_attachments",
            """
            CREATE TABLE external_document_attachments (
                document_id TEXT NOT NULL REFERENCES external_documents(id) ON DELETE CASCADE,
                attachment_id TEXT NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
                created_at TEXT NOT NULL,
                PRIMARY KEY(document_id, attachment_id)
            )
            """,
            "document_id, attachment_id, created_at",
        ),
    )
    broken = [
        (table, create_sql, columns)
        for table, create_sql, columns in child_tables
        if _attachment_fk_targets(conn, table) - {"attachments"}
    ]
    if not broken:
        return

    previous_fk_state = conn.execute("PRAGMA foreign_keys").fetchone()[0]
    previous_legacy_state = conn.execute("PRAGMA legacy_alter_table").fetchone()[0]
    conn.commit()
    conn.execute("PRAGMA foreign_keys = OFF")
    conn.execute("PRAGMA legacy_alter_table = ON")
    conn.execute("BEGIN IMMEDIATE")
    try:
        for table, create_sql, columns in broken:
            legacy_table = f"{table}__legacy_attachment_fk"
            conn.execute(f"ALTER TABLE {table} RENAME TO {legacy_table}")
            conn.execute(create_sql)
            conn.execute(
                f"INSERT OR IGNORE INTO {table} ({columns}) "
                f"SELECT {columns} FROM {legacy_table}"
            )
            conn.execute(f"DROP TABLE {legacy_table}")
        conn.execute("COMMIT")
    except Exception:
        conn.execute("ROLLBACK")
        raise
    finally:
        conn.execute(
            f"PRAGMA legacy_alter_table = {'ON' if previous_legacy_state else 'OFF'}"
        )
        conn.execute(f"PRAGMA foreign_keys = {'ON' if previous_fk_state else 'OFF'}")


def _ensure_swap_matching_schema(conn):
    """Add swap-matching columns + active-pair indexes for transaction_pairs.

    Splits into four ordered steps:
      1. Drop the legacy table-level ``UNIQUE`` constraints on
         ``transaction_pairs`` so soft-deleted pairs don't block re-pairing the
         same legs. Rebuilds the table only when the legacy constraints are
         actually present.
      2. ``ensure_column`` the new nullable columns on existing tables.
      3. Index ``transactions.payment_hash`` for the matcher's exact-lookup
         path.
      4. Re-create active-pair indexes. The per-leg indexes are deliberately
         non-unique: reviewed CoinJoin / missing-intermediate-wallet flows can
         link one transaction to several counterparties. A separate exact-pair
         partial unique index still blocks duplicate active links.
    """
    _migrate_legacy_transaction_pairs_uniques(conn)
    ensure_column(conn, "transactions", "payment_hash", "TEXT")
    ensure_column(conn, "transactions", "payment_hash_source", "TEXT")
    # Links an inbound HTLC refund back to its on-chain funding (lockup) txid so
    # the matcher can pair a failed swap's send + refund even within one wallet.
    ensure_column(conn, "transactions", "swap_refund_funding_txid", "TEXT")
    ensure_column(conn, "transactions", "swap_refund_funding_vout", "INTEGER")
    ensure_column(conn, "transaction_pairs", "swap_fee_msat", "INTEGER")
    ensure_column(conn, "transaction_pairs", "swap_fee_kind", "TEXT")
    ensure_column(conn, "transaction_pairs", "confidence_at_pair", "TEXT")
    ensure_column(conn, "transaction_pairs", "pair_source", "TEXT")
    ensure_column(conn, "transaction_pairs", "deleted_at", "TEXT")
    # Portion of the out leg (msat) that participates in a cross-asset swap when
    # the spend is split between a same-asset self-transfer and a peg. NULL means
    # the whole out leg is paired (the default / existing behavior).
    ensure_column(conn, "transaction_pairs", "out_amount", "INTEGER")
    ensure_column(
        conn,
        "transaction_pairs",
        "component_id",
        "TEXT REFERENCES custody_components(id) ON DELETE SET NULL",
    )
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_transactions_payment_hash "
        "ON transactions(payment_hash) WHERE payment_hash IS NOT NULL"
    )
    conn.execute("DROP INDEX IF EXISTS idx_transaction_pairs_active_out")
    conn.execute("DROP INDEX IF EXISTS idx_transaction_pairs_active_in")
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_transaction_pairs_active_out "
        "ON transaction_pairs(profile_id, out_transaction_id) WHERE deleted_at IS NULL"
    )
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_transaction_pairs_active_in "
        "ON transaction_pairs(profile_id, in_transaction_id) WHERE deleted_at IS NULL"
    )
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_transaction_pairs_active_pair "
        "ON transaction_pairs(profile_id, out_transaction_id, in_transaction_id) "
        "WHERE deleted_at IS NULL"
    )
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_transaction_pairs_profile_active "
        "ON transaction_pairs(profile_id) WHERE deleted_at IS NULL"
    )
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_transaction_pairs_component "
        "ON transaction_pairs(component_id) WHERE component_id IS NOT NULL"
    )
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_transaction_pairs_component_pending "
        "ON transaction_pairs(id) WHERE component_id IS NULL"
    )
    _clear_stale_same_asset_swap_fees(conn)
    conn.commit()
    _backfill_payment_hash_from_raw_json(conn)


def _clear_stale_same_asset_swap_fees(conn):
    rows = conn.execute(
        """
        SELECT p.id,
               p.kind,
               out_tx.asset AS out_asset,
               in_tx.asset AS in_asset
        FROM transaction_pairs p
        JOIN transactions out_tx ON out_tx.id = p.out_transaction_id
        JOIN transactions in_tx ON in_tx.id = p.in_transaction_id
        WHERE p.swap_fee_msat IS NOT NULL
          AND p.deleted_at IS NULL
        """
    ).fetchall()
    stale_ids = []
    for row in rows:
        try:
            out_asset = normalize_asset_code(row["out_asset"])
            in_asset = normalize_asset_code(row["in_asset"])
        except (TypeError, ValueError):
            continue
        if out_asset != in_asset:
            continue
        if str(row["kind"] or "").strip().lower() in SWAP_FEE_PAIR_KINDS:
            continue
        stale_ids.append((row["id"],))
    if not stale_ids:
        return
    conn.executemany(
        "UPDATE transaction_pairs SET swap_fee_msat = NULL, swap_fee_kind = NULL WHERE id = ?",
        stale_ids,
    )


def _ensure_direct_swap_payout_schema(conn):
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS direct_swap_payouts (
            id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
            out_transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
            kind TEXT NOT NULL DEFAULT 'direct-swap-payout',
            policy TEXT NOT NULL DEFAULT 'carrying-value',
            payout_asset TEXT NOT NULL,
            payout_amount INTEGER NOT NULL,
            payout_occurred_at TEXT,
            payout_fiat_value REAL,
            payout_external_id TEXT,
            counterparty TEXT,
            notes TEXT,
            swap_fee_msat INTEGER,
            swap_fee_kind TEXT,
            out_amount INTEGER,
            component_id TEXT REFERENCES custody_components(id) ON DELETE SET NULL,
            deleted_at TEXT,
            created_at TEXT NOT NULL
        )
        """
    )
    ensure_column(conn, "direct_swap_payouts", "out_amount", "INTEGER")
    ensure_column(
        conn,
        "direct_swap_payouts",
        "component_id",
        "TEXT REFERENCES custody_components(id) ON DELETE SET NULL",
    )
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_direct_swap_payouts_active_out "
        "ON direct_swap_payouts(profile_id, out_transaction_id) WHERE deleted_at IS NULL"
    )
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_direct_swap_payouts_profile_active "
        "ON direct_swap_payouts(profile_id) WHERE deleted_at IS NULL"
    )
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_direct_swap_payouts_component "
        "ON direct_swap_payouts(component_id) WHERE component_id IS NOT NULL"
    )
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_direct_swap_payouts_component_pending "
        "ON direct_swap_payouts(id) WHERE component_id IS NULL"
    )
    conn.commit()


def _ensure_legacy_custody_write_freeze_triggers(conn):
    """Reject compatibility-row changes while their component is active.

    Local mutation services retire the linked revision first. Replication or
    older code that tries to mutate a booked compatibility row therefore fails
    closed instead of leaving the row and authored aggregate inconsistent.
    """

    conn.execute(
        """
        CREATE TRIGGER IF NOT EXISTS trg_transaction_pairs_component_write_frozen
        BEFORE UPDATE ON transaction_pairs
        WHEN OLD.component_id IS NOT NULL
          AND EXISTS (
              SELECT 1 FROM custody_components c
              WHERE c.id = OLD.component_id AND c.state = 'active'
          )
          AND (
              NEW.out_transaction_id IS NOT OLD.out_transaction_id OR
              NEW.in_transaction_id IS NOT OLD.in_transaction_id OR
              NEW.kind IS NOT OLD.kind OR
              NEW.policy IS NOT OLD.policy OR
              NEW.notes IS NOT OLD.notes OR
              NEW.swap_fee_msat IS NOT OLD.swap_fee_msat OR
              NEW.swap_fee_kind IS NOT OLD.swap_fee_kind OR
              NEW.confidence_at_pair IS NOT OLD.confidence_at_pair OR
              NEW.pair_source IS NOT OLD.pair_source OR
              NEW.out_amount IS NOT OLD.out_amount OR
              NEW.component_id IS NOT OLD.component_id OR
              NEW.deleted_at IS NOT OLD.deleted_at
          )
        BEGIN
            SELECT RAISE(ABORT, 'legacy_custody_review_write_frozen');
        END
        """
    )
    conn.execute(
        """
        CREATE TRIGGER IF NOT EXISTS trg_direct_swap_payouts_component_write_frozen
        BEFORE UPDATE ON direct_swap_payouts
        WHEN OLD.component_id IS NOT NULL
          AND EXISTS (
              SELECT 1 FROM custody_components c
              WHERE c.id = OLD.component_id AND c.state = 'active'
          )
          AND (
              NEW.out_transaction_id IS NOT OLD.out_transaction_id OR
              NEW.kind IS NOT OLD.kind OR
              NEW.policy IS NOT OLD.policy OR
              NEW.payout_asset IS NOT OLD.payout_asset OR
              NEW.payout_amount IS NOT OLD.payout_amount OR
              NEW.payout_occurred_at IS NOT OLD.payout_occurred_at OR
              NEW.payout_fiat_value IS NOT OLD.payout_fiat_value OR
              NEW.payout_external_id IS NOT OLD.payout_external_id OR
              NEW.counterparty IS NOT OLD.counterparty OR
              NEW.notes IS NOT OLD.notes OR
              NEW.swap_fee_msat IS NOT OLD.swap_fee_msat OR
              NEW.swap_fee_kind IS NOT OLD.swap_fee_kind OR
              NEW.out_amount IS NOT OLD.out_amount OR
              NEW.component_id IS NOT OLD.component_id OR
              NEW.deleted_at IS NOT OLD.deleted_at
          )
        BEGIN
            SELECT RAISE(ABORT, 'legacy_custody_review_write_frozen');
        END
        """
    )


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
    legacy_columns = {
        column["name"]
        for column in conn.execute("PRAGMA table_info(transaction_pairs)").fetchall()
    }
    component_select = "component_id" if "component_id" in legacy_columns else "NULL"
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
                component_id TEXT REFERENCES custody_components(id) ON DELETE SET NULL,
                created_at TEXT NOT NULL
            )
            """
        )
        conn.execute(
            f"""
            INSERT INTO transaction_pairs
            (id, workspace_id, profile_id, out_transaction_id, in_transaction_id,
             kind, policy, notes, component_id, created_at)
            SELECT id, workspace_id, profile_id, out_transaction_id, in_transaction_id,
                   kind, policy, notes, {component_select}, created_at
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


def _recreate_msat_migration_indexes(conn):
    """Restore indexes dropped by SQLite table rebuild migrations."""
    conn.executescript(
        """
        CREATE INDEX IF NOT EXISTS idx_transactions_profile_external_id
            ON transactions(profile_id, external_id) WHERE external_id IS NOT NULL;

        CREATE INDEX IF NOT EXISTS idx_transactions_profile_active_time
            ON transactions(profile_id, excluded, occurred_at, created_at, id);

        CREATE INDEX IF NOT EXISTS idx_transactions_wallet_external_match
            ON transactions(wallet_id, external_id, direction, asset, amount, fee, created_at)
            WHERE external_id IS NOT NULL;

        CREATE INDEX IF NOT EXISTS idx_transactions_profile_economic_match
            ON transactions(profile_id, direction, asset, amount, occurred_at, created_at);

        CREATE INDEX IF NOT EXISTS idx_transactions_wallet_pricing_ref
            ON transactions(wallet_id, pricing_external_ref, direction, asset, amount, created_at)
            WHERE pricing_external_ref IS NOT NULL;

        CREATE INDEX IF NOT EXISTS idx_journal_entries_profile_time
            ON journal_entries(profile_id, occurred_at, created_at, id);

        CREATE INDEX IF NOT EXISTS idx_journal_entries_profile_type_time
            ON journal_entries(profile_id, entry_type, occurred_at, created_at, id);

        CREATE INDEX IF NOT EXISTS idx_journal_entries_profile_wallet_time
            ON journal_entries(profile_id, wallet_id, occurred_at, created_at, id);

        CREATE INDEX IF NOT EXISTS idx_journal_entries_profile_account_time
            ON journal_entries(profile_id, account_id, occurred_at, created_at, id);

        CREATE INDEX IF NOT EXISTS idx_journal_entries_transaction
            ON journal_entries(transaction_id);

        CREATE INDEX IF NOT EXISTS idx_journal_tax_summary_profile_year
            ON journal_tax_summary(profile_id, year, asset, transaction_type, capital_gains_type);

        CREATE INDEX IF NOT EXISTS idx_journal_account_holdings_profile_asset
            ON journal_account_holdings(profile_id, asset, account_code, id);

        CREATE INDEX IF NOT EXISTS idx_journal_wallet_holdings_profile_asset
            ON journal_wallet_holdings(profile_id, asset, wallet_label, id);
        """
    )


def _migrate_msat_columns(conn):
    """Rebuild BTC-denominated tables to store amounts as INTEGER msat.

    Safe on fresh databases (columns are already INTEGER -> no-op) and on
    pre-migration databases created with REAL amount/fee/quantity columns.
    Existing float BTC values are multiplied into msat with ROUND_HALF_UP.
    """
    migrate_transactions = _column_is_real(
        conn, "transactions", "amount"
    ) or _column_is_real(conn, "transactions", "fee")
    migrate_journal_entries = _column_is_real(conn, "journal_entries", "quantity")
    migrate_journal_tax_summary = _column_is_real(conn, "journal_tax_summary", "quantity")
    migrate_journal_account_holdings = _column_is_real(conn, "journal_account_holdings", "quantity")
    migrate_journal_wallet_holdings = _column_is_real(conn, "journal_wallet_holdings", "quantity")
    if not any(
        (
            migrate_transactions,
            migrate_journal_entries,
            migrate_journal_tax_summary,
            migrate_journal_account_holdings,
            migrate_journal_wallet_holdings,
        )
    ):
        return

    conn.commit()
    previous_fk_state = conn.execute("PRAGMA foreign_keys").fetchone()[0]
    conn.execute("PRAGMA foreign_keys = OFF")
    try:
        if migrate_transactions:
            # Custody scope triggers query ``transactions``. SQLite
            # validates trigger bodies while the legacy REAL table is dropped /
            # renamed, even with foreign keys disabled, so suspend only those
            # derived guards for the atomic rebuild and restore the idempotent
            # custody schema afterward.
            conn.executescript(
                """
                DROP TRIGGER IF EXISTS trg_custody_component_scope_insert;
                DROP TRIGGER IF EXISTS trg_custody_component_scope_update;
                DROP TRIGGER IF EXISTS trg_custody_gap_review_transaction_scope_insert;
                """
            )
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
                    commercial_applied_link_id TEXT,
                    review_status TEXT,
                    taxability_override INTEGER,
                    at_regime_override TEXT,
                    at_category_override TEXT,
                    privacy_boundary TEXT,
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
                    commercial_applied_link_id, review_status,
                    taxability_override, at_regime_override,
                    at_category_override, privacy_boundary,
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
                    capital_gains_type TEXT,
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
                    at_category, at_kennzahl, capital_gains_type, created_at
                FROM journal_entries;
                DROP TABLE journal_entries;
                ALTER TABLE journal_entries__msat_new RENAME TO journal_entries;
                COMMIT;
                """
            )
        if migrate_journal_tax_summary:
            conn.executescript(
                """
                BEGIN;
                CREATE TABLE journal_tax_summary__msat_new (
                    id TEXT PRIMARY KEY,
                    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
                    year INTEGER NOT NULL,
                    asset TEXT NOT NULL,
                    transaction_type TEXT NOT NULL,
                    capital_gains_type TEXT,
                    quantity INTEGER NOT NULL,
                    proceeds REAL NOT NULL DEFAULT 0,
                    cost_basis REAL NOT NULL DEFAULT 0,
                    gain_loss REAL NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL
                );
                INSERT INTO journal_tax_summary__msat_new SELECT
                    id, workspace_id, profile_id, year, asset, transaction_type,
                    capital_gains_type,
                    CAST(ROUND(quantity * 100000000000.0) AS INTEGER),
                    proceeds, cost_basis, gain_loss, created_at
                FROM journal_tax_summary;
                DROP TABLE journal_tax_summary;
                ALTER TABLE journal_tax_summary__msat_new RENAME TO journal_tax_summary;
                COMMIT;
                """
            )
        if migrate_journal_account_holdings:
            conn.executescript(
                """
                BEGIN;
                CREATE TABLE journal_account_holdings__msat_new (
                    id TEXT PRIMARY KEY,
                    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
                    account_id TEXT REFERENCES accounts(id) ON DELETE SET NULL,
                    account_code TEXT,
                    account_label TEXT,
                    asset TEXT NOT NULL,
                    quantity INTEGER NOT NULL,
                    cost_basis REAL NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL
                );
                INSERT INTO journal_account_holdings__msat_new SELECT
                    id, workspace_id, profile_id, account_id, account_code, account_label,
                    asset,
                    CAST(ROUND(quantity * 100000000000.0) AS INTEGER),
                    cost_basis, created_at
                FROM journal_account_holdings;
                DROP TABLE journal_account_holdings;
                ALTER TABLE journal_account_holdings__msat_new RENAME TO journal_account_holdings;
                COMMIT;
                """
            )
        if migrate_journal_wallet_holdings:
            conn.executescript(
                """
                BEGIN;
                CREATE TABLE journal_wallet_holdings__msat_new (
                    id TEXT PRIMARY KEY,
                    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
                    wallet_id TEXT REFERENCES wallets(id) ON DELETE CASCADE,
                    wallet_label TEXT,
                    account_code TEXT,
                    asset TEXT NOT NULL,
                    quantity INTEGER NOT NULL,
                    cost_basis REAL NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL
                );
                INSERT INTO journal_wallet_holdings__msat_new SELECT
                    id, workspace_id, profile_id, wallet_id, wallet_label, account_code,
                    asset,
                    CAST(ROUND(quantity * 100000000000.0) AS INTEGER),
                    cost_basis, created_at
                FROM journal_wallet_holdings;
                DROP TABLE journal_wallet_holdings;
                ALTER TABLE journal_wallet_holdings__msat_new RENAME TO journal_wallet_holdings;
                COMMIT;
                """
            )
        _recreate_msat_migration_indexes(conn)
        if migrate_transactions:
            conn.executescript(CUSTODY_COMPONENT_SCHEMA)
            _create_custody_gap_review_transaction_aux_schema(conn)
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
    affected_profile_ids = {row["profile_id"] for row in affected_rows}
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
    legacy_rows = conn.execute(
        f"""
        SELECT t.id, t.profile_id, t.external_id, t.asset, t.raw_json,
               w.config_json
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        WHERE upper(replace(t.asset, '-', '')) = 'LBTC'
           OR lower(t.asset) IN ({placeholders})
        """,
        policy_asset_hexes,
    ).fetchall()
    for row in legacy_rows:
        try:
            raw = json.loads(row["raw_json"] or "{}")
            config = json.loads(row["config_json"] or "{}")
        except (TypeError, ValueError, json.JSONDecodeError):
            continue
        if not isinstance(raw, dict) or not isinstance(config, dict):
            continue
        try:
            if normalize_chain(config.get("chain")) != "liquid":
                continue
            network = normalize_network("liquid", config.get("network"))
        except ValueError:
            continue
        explicit_policy_asset = normalize_asset_code(config.get("policy_asset"))
        if explicit_policy_asset == "LBTC":
            explicit_policy_asset = ""
        asset_id = explicit_policy_asset or default_policy_asset_id(network)
        asset_id = normalize_asset_code(asset_id)
        if len(asset_id) != 64:
            continue
        txid = str(raw.get("txid") or row["external_id"] or "").strip().lower()
        if len(txid) != 64 or any(char not in "0123456789abcdef" for char in txid):
            continue
        # Never overwrite contradictory evidence. This migration only fills the
        # identity fields emitted by current Liquid sync for legacy observations
        # whose wallet config proves the missing network and policy asset.
        expected = {
            "txid": txid,
            "chain": "liquid",
            "network": network,
            "asset_id": asset_id,
            "asset": "LBTC",
        }
        existing_txid = str(raw.get("txid") or "").strip().lower()
        existing_chain = str(raw.get("chain") or "").strip()
        existing_network = str(raw.get("network") or "").strip()
        existing_asset_id = normalize_asset_code(raw.get("asset_id"))
        existing_asset = normalize_asset_code(raw.get("asset"))
        try:
            chain_conflicts = bool(existing_chain) and normalize_chain(existing_chain) != "liquid"
            network_conflicts = bool(existing_network) and normalize_network(
                "liquid", existing_network
            ) != network
        except ValueError:
            continue
        if (
            (existing_txid and existing_txid != txid)
            or chain_conflicts
            or network_conflicts
            or (existing_asset_id and existing_asset_id != asset_id)
            or (existing_asset and existing_asset not in {"LBTC", asset_id})
        ):
            continue
        updated_raw = {**raw, **expected}
        if updated_raw == raw:
            continue
        conn.execute(
            "UPDATE transactions SET raw_json = ? WHERE id = ?",
            (json.dumps(updated_raw, sort_keys=True), row["id"]),
        )
        affected_profile_ids.add(row["profile_id"])
    affected_profile_ids = sorted(affected_profile_ids)
    if not affected_profile_ids:
        return
    profile_placeholders = ",".join("?" for _ in affected_profile_ids)
    conn.execute(
        f"UPDATE profiles "
        f"SET last_processed_at = NULL, "
        f"last_processed_tx_count = 0, "
        f"ownership_review_counts_json = NULL, "
        f"journal_input_version = journal_input_version + 1 "
        f"WHERE id IN ({profile_placeholders})",
        affected_profile_ids,
    )
    conn.commit()
