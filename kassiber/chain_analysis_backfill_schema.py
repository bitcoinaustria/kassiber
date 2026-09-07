"""Derived acquisition state; never replicated or treated as custody authority."""
SCHEMA = """
CREATE TABLE IF NOT EXISTS chain_analysis_acquisition_grants (
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    spec_json TEXT NOT NULL,
    binding_json TEXT NOT NULL,
    source_fingerprint TEXT NOT NULL,
    database_identity TEXT NOT NULL,
    status TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1,
    requests_used INTEGER NOT NULL DEFAULT 0,
    bytes_used INTEGER NOT NULL DEFAULT 0,
    next_run_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    cursor_height INTEGER,
    cursor_hash TEXT,
    verified_tip_height INTEGER,
    verified_tip_hash TEXT,
    verified_tip_at TEXT,
    lease_token TEXT,
    lease_until INTEGER,
    last_code TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_acquisition_grants_due
    ON chain_analysis_acquisition_grants(profile_id,status,next_run_at);
CREATE TABLE IF NOT EXISTS chain_analysis_reference_blocks (
    grant_id TEXT NOT NULL REFERENCES chain_analysis_acquisition_grants(id) ON DELETE CASCADE,
    block_hash TEXT NOT NULL,
    height INTEGER NOT NULL,
    parent_hash TEXT NOT NULL,
    active INTEGER NOT NULL,
    PRIMARY KEY(grant_id,block_hash)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_reference_blocks_canonical
    ON chain_analysis_reference_blocks(grant_id,height) WHERE active=1;
CREATE TABLE IF NOT EXISTS chain_analysis_reference_assertions (
    grant_id TEXT NOT NULL REFERENCES chain_analysis_acquisition_grants(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    domain_id TEXT NOT NULL,
    chain TEXT NOT NULL,
    network TEXT NOT NULL,
    occurrence_id TEXT NOT NULL,
    txid TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    status_json TEXT NOT NULL,
    active INTEGER NOT NULL,
    observed_at TEXT NOT NULL,
    PRIMARY KEY(grant_id,occurrence_id)
);
CREATE INDEX IF NOT EXISTS idx_reference_assertions_profile
    ON chain_analysis_reference_assertions(profile_id,active,txid);
"""
