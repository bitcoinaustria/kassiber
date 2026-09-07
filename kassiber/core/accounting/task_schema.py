"""Local encrypted task records; no replication or provider authority."""


def ensure_schema(conn):
    statements = (
        """CREATE TABLE IF NOT EXISTS gl_accounting_tasks(
          id TEXT PRIMARY KEY,profile_id TEXT NOT NULL REFERENCES gl_books(profile_id),
          period_id TEXT NOT NULL,idempotency_key TEXT NOT NULL,spec_json TEXT NOT NULL,
          request_digest TEXT NOT NULL,created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')),
          UNIQUE(profile_id,id),UNIQUE(profile_id,idempotency_key),
          FOREIGN KEY(profile_id,period_id) REFERENCES gl_periods(profile_id,id))""",
        """CREATE TABLE IF NOT EXISTS gl_accounting_task_rules(
          id TEXT PRIMARY KEY,profile_id TEXT NOT NULL REFERENCES gl_books(profile_id),
          idempotency_key TEXT NOT NULL,payload_json TEXT NOT NULL,request_digest TEXT NOT NULL,
          UNIQUE(profile_id,id),UNIQUE(profile_id,idempotency_key))""",
        """CREATE TABLE IF NOT EXISTS gl_accounting_task_rule_revocations(
          rule_id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,reason TEXT NOT NULL,
          FOREIGN KEY(profile_id,rule_id) REFERENCES gl_accounting_task_rules(profile_id,id))""",
        """CREATE TABLE IF NOT EXISTS gl_accounting_task_cancellations(
          task_id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,reason TEXT NOT NULL,
          FOREIGN KEY(profile_id,task_id) REFERENCES gl_accounting_tasks(profile_id,id))""",
        """CREATE TABLE IF NOT EXISTS gl_accounting_task_claims(
          profile_id TEXT NOT NULL,source_kind TEXT NOT NULL,source_id TEXT NOT NULL,
          task_id TEXT NOT NULL,entry_id TEXT NOT NULL,
          PRIMARY KEY(profile_id,source_kind,source_id),
          FOREIGN KEY(profile_id,task_id) REFERENCES gl_accounting_tasks(profile_id,id),
          FOREIGN KEY(profile_id,entry_id) REFERENCES gl_entries(profile_id,id))""",
        """CREATE TABLE IF NOT EXISTS gl_accounting_task_evidence_assignments(
          id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,task_id TEXT NOT NULL,evidence_id TEXT NOT NULL,
          idempotency_key TEXT NOT NULL,payload_json TEXT NOT NULL,request_digest TEXT NOT NULL,
          previous_id TEXT UNIQUE REFERENCES gl_accounting_task_evidence_assignments(id),
          UNIQUE(profile_id,idempotency_key),
          FOREIGN KEY(profile_id,task_id) REFERENCES gl_accounting_tasks(profile_id,id),
          FOREIGN KEY(profile_id,evidence_id) REFERENCES gl_evidence(profile_id,id))""",
        """CREATE TABLE IF NOT EXISTS gl_accounting_task_receipts(
          id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,task_id TEXT NOT NULL,step TEXT NOT NULL,
          idempotency_key TEXT NOT NULL,request_digest TEXT NOT NULL,result_json TEXT NOT NULL,
          created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')),
          UNIQUE(profile_id,idempotency_key),
          FOREIGN KEY(profile_id,task_id) REFERENCES gl_accounting_tasks(profile_id,id))""",
    )
    for sql in statements:
        conn.execute(sql)
    for table in ('gl_accounting_tasks', 'gl_accounting_task_rules',
                  'gl_accounting_task_rule_revocations', 'gl_accounting_task_cancellations',
                  'gl_accounting_task_claims', 'gl_accounting_task_receipts',
                  'gl_accounting_task_evidence_assignments'):
        for action in ('UPDATE', 'DELETE'):
            conn.execute(f"""CREATE TRIGGER IF NOT EXISTS {table}_no_{action.lower()}
              BEFORE {action} ON {table} BEGIN SELECT RAISE(ABORT,'accounting_task_retained'); END""")
        # REPLACE may silently delete a retained record with recursive triggers off.
        keys = {'gl_accounting_task_claims': 'profile_id=NEW.profile_id AND source_kind=NEW.source_kind AND source_id=NEW.source_id',
                'gl_accounting_task_rule_revocations': 'rule_id=NEW.rule_id',
                'gl_accounting_task_cancellations': 'task_id=NEW.task_id'}
        conflict = keys.get(table, 'id=NEW.id OR (profile_id=NEW.profile_id AND idempotency_key=NEW.idempotency_key)')
        if table == 'gl_accounting_task_evidence_assignments':
            conflict += ' OR (previous_id IS NOT NULL AND previous_id=NEW.previous_id)'
        conn.execute(f"""CREATE TRIGGER IF NOT EXISTS {table}_no_replace BEFORE INSERT ON {table}
          WHEN EXISTS(SELECT 1 FROM {table} WHERE {conflict})
          BEGIN SELECT RAISE(ABORT,'accounting_task_retained'); END""")
