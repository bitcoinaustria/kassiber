"""Ledger schema. Deliberately does not commit or use executescript."""

TABLES = (
    """CREATE TABLE IF NOT EXISTS gl_books (
        profile_id TEXT PRIMARY KEY REFERENCES profiles(id) ON DELETE RESTRICT,
        currency TEXT NOT NULL, minor_unit_exponent INTEGER NOT NULL,
        timezone TEXT NOT NULL, entity_kind TEXT NOT NULL, accounting_regime TEXT NOT NULL,
        revision INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL)""",
    """CREATE TABLE IF NOT EXISTS gl_accounts (
        profile_id TEXT NOT NULL REFERENCES gl_books(profile_id) ON DELETE RESTRICT,
        code TEXT NOT NULL, name TEXT NOT NULL,
        kind TEXT NOT NULL CHECK(kind IN ('asset','liability','equity','income','expense')),
        PRIMARY KEY(profile_id,code))""",
    """CREATE TABLE IF NOT EXISTS gl_periods (
        profile_id TEXT NOT NULL REFERENCES gl_books(profile_id) ON DELETE RESTRICT,
        id TEXT NOT NULL, start_date TEXT NOT NULL, end_date TEXT NOT NULL,
        state TEXT NOT NULL DEFAULT 'open' CHECK(state IN ('open','closed','review')),
        revision INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY(profile_id,id), CHECK(start_date<=end_date))""",
    """CREATE TABLE IF NOT EXISTS gl_entries (
        id TEXT PRIMARY KEY, profile_id TEXT NOT NULL REFERENCES gl_books(profile_id) ON DELETE RESTRICT,
        period_id TEXT NOT NULL, entry_date TEXT NOT NULL, description TEXT NOT NULL,
        entry_kind TEXT NOT NULL CHECK(entry_kind IN ('normal','opening','closing','reversal')),
        status TEXT NOT NULL CHECK(status IN ('draft','posted')),
        idempotency_key TEXT NOT NULL, payload_digest TEXT NOT NULL,
        source_ref TEXT, reversal_of TEXT REFERENCES gl_entries(id) ON DELETE RESTRICT,
        created_at TEXT NOT NULL, posted_at TEXT,
        UNIQUE(profile_id,idempotency_key), UNIQUE(profile_id,id),
        FOREIGN KEY(profile_id,period_id) REFERENCES gl_periods(profile_id,id) ON DELETE RESTRICT)""",
    """CREATE UNIQUE INDEX IF NOT EXISTS gl_single_reversal ON gl_entries(reversal_of)
        WHERE reversal_of IS NOT NULL""",
    """CREATE UNIQUE INDEX IF NOT EXISTS gl_single_opening ON gl_entries(profile_id)
        WHERE entry_kind='opening' AND status='posted'""",
    """CREATE TABLE IF NOT EXISTS gl_lines (
        id TEXT PRIMARY KEY, entry_id TEXT NOT NULL, position INTEGER NOT NULL,
        profile_id TEXT NOT NULL, account_code TEXT NOT NULL,
        account_name TEXT NOT NULL, account_kind TEXT NOT NULL,
        debit_minor INTEGER NOT NULL CHECK(typeof(debit_minor)='integer' AND debit_minor>=0),
        credit_minor INTEGER NOT NULL CHECK(typeof(credit_minor)='integer' AND credit_minor>=0),
        CHECK((debit_minor>0 AND credit_minor=0) OR (credit_minor>0 AND debit_minor=0)),
        UNIQUE(entry_id,position),
        FOREIGN KEY(profile_id,entry_id) REFERENCES gl_entries(profile_id,id) ON DELETE RESTRICT,
        FOREIGN KEY(profile_id,account_code) REFERENCES gl_accounts(profile_id,code) ON DELETE RESTRICT)""",
    """CREATE TABLE IF NOT EXISTS gl_period_events (
        id TEXT PRIMARY KEY, profile_id TEXT NOT NULL, period_id TEXT NOT NULL,
        revision INTEGER NOT NULL, action TEXT NOT NULL CHECK(action IN ('close','reopen')),
        reason TEXT NOT NULL, snapshot_json TEXT NOT NULL, snapshot_digest TEXT NOT NULL,
        created_at TEXT NOT NULL, UNIQUE(profile_id,period_id,revision),
        FOREIGN KEY(profile_id,period_id) REFERENCES gl_periods(profile_id,id) ON DELETE RESTRICT)""",
    "CREATE INDEX IF NOT EXISTS gl_entries_scope_date ON gl_entries(profile_id,status,entry_date)",
)


def ensure_schema(conn):
    for statement in TABLES:
        conn.execute(statement)
    for table in ('gl_accounts', 'gl_period_events'):
        for operation in ('UPDATE', 'DELETE'):
            conn.execute(f"""CREATE TRIGGER IF NOT EXISTS {table}_immutable_{operation.lower()}
                BEFORE {operation} ON {table} BEGIN
                SELECT RAISE(ABORT,'accounting immutable record'); END""")
    # REPLACE deletes may not fire DELETE triggers when recursive_triggers is off.
    # Reject the conflicting insert itself, independently of that connection flag.
    for table, match in (
        ('gl_books', 'profile_id=NEW.profile_id'),
        ('gl_accounts', 'profile_id=NEW.profile_id AND code=NEW.code'),
        ('gl_periods', 'profile_id=NEW.profile_id AND id=NEW.id'),
        ('gl_period_events', 'id=NEW.id OR (profile_id=NEW.profile_id AND period_id=NEW.period_id AND revision=NEW.revision)'),
        ('gl_entries', 'id=NEW.id OR (profile_id=NEW.profile_id AND idempotency_key=NEW.idempotency_key)'),
        ('gl_lines', 'id=NEW.id OR (entry_id=NEW.entry_id AND position=NEW.position)'),
    ):
        conn.execute(f"""CREATE TRIGGER IF NOT EXISTS {table}_no_replace
            BEFORE INSERT ON {table} WHEN EXISTS(SELECT 1 FROM {table} WHERE {match})
            BEGIN SELECT RAISE(ABORT,'accounting record replacement forbidden'); END""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_book_config_immutable
        BEFORE UPDATE ON gl_books WHEN NEW.profile_id!=OLD.profile_id OR
        NEW.currency!=OLD.currency OR NEW.minor_unit_exponent!=OLD.minor_unit_exponent OR
        NEW.timezone!=OLD.timezone OR NEW.entity_kind!=OLD.entity_kind OR
        NEW.accounting_regime!=OLD.accounting_regime OR NEW.created_at!=OLD.created_at
        BEGIN SELECT RAISE(ABORT,'accounting immutable configuration'); END""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_entry_insert_draft
        BEFORE INSERT ON gl_entries WHEN NEW.status!='draft'
        BEGIN SELECT RAISE(ABORT,'accounting entries must start as drafts'); END""")
    for operation in ('UPDATE', 'DELETE'):
        conn.execute(f"""CREATE TRIGGER IF NOT EXISTS gl_posted_entry_{operation.lower()}
            BEFORE {operation} ON gl_entries WHEN OLD.status='posted'
            BEGIN SELECT RAISE(ABORT,'accounting posted entry immutable'); END""")
    for operation in ('INSERT', 'UPDATE', 'DELETE'):
        condition = []
        if operation != 'DELETE':
            condition.append("EXISTS(SELECT 1 FROM gl_entries WHERE id=NEW.entry_id AND status='posted')")
        if operation != 'INSERT':
            condition.append("EXISTS(SELECT 1 FROM gl_entries WHERE id=OLD.entry_id AND status='posted')")
        conn.execute(f"""CREATE TRIGGER IF NOT EXISTS gl_posted_line_{operation.lower()}
            BEFORE {operation} ON gl_lines WHEN {' OR '.join(condition)}
            BEGIN SELECT RAISE(ABORT,'accounting posted lines immutable'); END""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_post_validate
        BEFORE UPDATE OF status ON gl_entries WHEN NEW.status='posted' BEGIN
        SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM gl_periods p
            WHERE p.profile_id=NEW.profile_id AND p.id=NEW.period_id AND p.state='open'
            AND NEW.entry_date BETWEEN p.start_date AND p.end_date)
            THEN RAISE(ABORT,'accounting period not open') END;
        SELECT CASE WHEN (SELECT COUNT(*) FROM gl_lines WHERE entry_id=NEW.id)<2
            OR (SELECT COALESCE(SUM(debit_minor),0) FROM gl_lines WHERE entry_id=NEW.id)
            !=(SELECT COALESCE(SUM(credit_minor),0) FROM gl_lines WHERE entry_id=NEW.id)
            THEN RAISE(ABORT,'accounting unbalanced posting') END;
        SELECT CASE WHEN EXISTS(SELECT 1 FROM gl_lines l LEFT JOIN gl_accounts a
            ON a.profile_id=l.profile_id AND a.code=l.account_code WHERE l.entry_id=NEW.id
            AND (l.profile_id!=NEW.profile_id OR a.code IS NULL OR l.account_name!=a.name
            OR l.account_kind!=a.kind)) THEN RAISE(ABORT,'accounting invalid chart snapshot') END;
        END""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_period_dates_immutable
        BEFORE UPDATE ON gl_periods WHEN NEW.profile_id!=OLD.profile_id OR NEW.id!=OLD.id
        OR NEW.start_date!=OLD.start_date OR NEW.end_date!=OLD.end_date
        BEGIN SELECT RAISE(ABORT,'accounting fiscal interval immutable'); END""")
