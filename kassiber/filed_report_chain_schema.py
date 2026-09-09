"""Local observer dependencies and append-only attachments to the change inbox.

Physical chain evidence is installation-local; these tables never replicate.
The existing saved/filed snapshot remains the report's immutable authority.
"""

SCHEMA = """
CREATE TABLE IF NOT EXISTS filed_report_chain_dependencies (
 snapshot_id TEXT NOT NULL REFERENCES filed_report_snapshots(id) ON DELETE CASCADE,
 profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
 transaction_id TEXT NOT NULL, wallet_id TEXT NOT NULL,
 chain TEXT NOT NULL, network TEXT NOT NULL, txid TEXT NOT NULL,
 observation_json TEXT NOT NULL,
 PRIMARY KEY(snapshot_id, transaction_id)
);
CREATE INDEX IF NOT EXISTS idx_report_chain_dependencies_wallet
 ON filed_report_chain_dependencies(profile_id,wallet_id,transaction_id);
CREATE INDEX IF NOT EXISTS idx_report_chain_dependencies_subject
 ON filed_report_chain_dependencies(profile_id,chain,network,txid);
CREATE TABLE IF NOT EXISTS filed_report_chain_impacts (
 id TEXT PRIMARY KEY,
 snapshot_id TEXT NOT NULL REFERENCES filed_report_snapshots(id) ON DELETE CASCADE,
 profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
 transaction_id TEXT NOT NULL,
 inbox_id TEXT NOT NULL UNIQUE,
 created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_report_chain_impacts_dependency
 ON filed_report_chain_impacts(snapshot_id,transaction_id,created_at,id);
CREATE TABLE IF NOT EXISTS filed_report_chain_resolutions (
 impact_id TEXT PRIMARY KEY REFERENCES filed_report_chain_impacts(id) ON DELETE CASCADE,
 rebuilt_at TEXT NOT NULL, summary_json TEXT NOT NULL, created_at TEXT NOT NULL
);
CREATE TRIGGER IF NOT EXISTS trg_report_chain_impacts_delete_inbox
 AFTER DELETE ON filed_report_chain_impacts BEGIN
 DELETE FROM chain_analysis_watch_inbox WHERE id=OLD.inbox_id; END;
CREATE TRIGGER IF NOT EXISTS trg_report_chain_dependencies_immutable
 BEFORE UPDATE ON filed_report_chain_dependencies BEGIN
 SELECT RAISE(ABORT, 'report_chain_dependencies_immutable'); END;
CREATE TRIGGER IF NOT EXISTS trg_report_chain_impacts_immutable
 BEFORE UPDATE ON filed_report_chain_impacts BEGIN
 SELECT RAISE(ABORT, 'report_chain_impacts_immutable'); END;
CREATE TRIGGER IF NOT EXISTS trg_report_chain_resolutions_immutable
 BEFORE UPDATE ON filed_report_chain_resolutions BEGIN
 SELECT RAISE(ABORT, 'report_chain_resolutions_immutable'); END;
"""
