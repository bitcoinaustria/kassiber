"""Local-only watch definitions and encrypted durable inbox."""
WATCH_SCHEMA = """
CREATE TABLE IF NOT EXISTS chain_analysis_watches (
 id TEXT PRIMARY KEY, profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
 book_id TEXT NOT NULL, domain_json TEXT NOT NULL, definition_json TEXT NOT NULL,
 enabled INTEGER NOT NULL DEFAULT 1, revision INTEGER NOT NULL DEFAULT 1,
 baseline_json TEXT NOT NULL, checkpoint TEXT NOT NULL, sequence INTEGER NOT NULL DEFAULT 0,
 checked_at TEXT NOT NULL, created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_chain_analysis_watches_profile ON chain_analysis_watches(profile_id,enabled,id);
CREATE TABLE IF NOT EXISTS chain_analysis_watch_inbox (
 id TEXT PRIMARY KEY, watch_id TEXT NOT NULL REFERENCES chain_analysis_watches(id) ON DELETE CASCADE,
 profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
 sequence INTEGER NOT NULL, code TEXT NOT NULL, observation_json TEXT NOT NULL,
 created_at TEXT NOT NULL, acknowledged_at TEXT, delivered_at TEXT,
 UNIQUE(watch_id,sequence)
);
CREATE INDEX IF NOT EXISTS idx_chain_analysis_watch_inbox_profile ON chain_analysis_watch_inbox(profile_id,created_at,id);
"""
