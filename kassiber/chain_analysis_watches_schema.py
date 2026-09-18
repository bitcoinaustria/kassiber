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
 id TEXT PRIMARY KEY, watch_id TEXT REFERENCES chain_analysis_watches(id) ON DELETE CASCADE,
 profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
 sequence INTEGER NOT NULL, code TEXT NOT NULL, observation_json TEXT NOT NULL,
 created_at TEXT NOT NULL, acknowledged_at TEXT, delivered_at TEXT,
 UNIQUE(watch_id,sequence)
);
CREATE INDEX IF NOT EXISTS idx_chain_analysis_watch_inbox_profile ON chain_analysis_watch_inbox(profile_id,created_at,id);
"""


def ensure_inbox_sources(conn):
    """Allow sync-origin report events, retaining every old cursor and receipt."""
    columns = conn.execute("PRAGMA table_info(chain_analysis_watch_inbox)").fetchall()
    if not any(row[1] == "watch_id" and row[3] for row in columns):
        return
    conn.execute("SAVEPOINT report_inbox_sources")
    try:
        cleanup_trigger = conn.execute("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='trg_report_chain_impacts_delete_inbox'").fetchone()
        conn.execute("DROP TRIGGER IF EXISTS trg_report_chain_impacts_delete_inbox")
        conn.execute("""CREATE TABLE chain_analysis_watch_inbox_new (
            id TEXT PRIMARY KEY, watch_id TEXT REFERENCES chain_analysis_watches(id) ON DELETE CASCADE,
            profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
            sequence INTEGER NOT NULL, code TEXT NOT NULL, observation_json TEXT NOT NULL,
            created_at TEXT NOT NULL, acknowledged_at TEXT, delivered_at TEXT,
            UNIQUE(watch_id,sequence))""")
        # Preserve rowid, which is the public pagination cursor, including holes.
        conn.execute("INSERT INTO chain_analysis_watch_inbox_new(rowid,id,watch_id,profile_id,sequence,code,observation_json,created_at,acknowledged_at,delivered_at) SELECT rowid,* FROM chain_analysis_watch_inbox")
        conn.execute("DROP TABLE chain_analysis_watch_inbox")
        conn.execute("ALTER TABLE chain_analysis_watch_inbox_new RENAME TO chain_analysis_watch_inbox")
        conn.execute("CREATE INDEX idx_chain_analysis_watch_inbox_profile ON chain_analysis_watch_inbox(profile_id,created_at,id)")
        if cleanup_trigger:
            conn.execute(cleanup_trigger[0])
        conn.execute("RELEASE report_inbox_sources")
    except BaseException:
        conn.execute("ROLLBACK TO report_inbox_sources")
        conn.execute("RELEASE report_inbox_sources")
        raise
