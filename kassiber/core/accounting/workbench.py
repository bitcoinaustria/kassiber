"""Read-only local worklist over authoritative books; never a completion claim.

Counts cover their stated population, independent of UI pagination. Evidence
and open-item counts are book-wide because those records can span periods.
Only statement choices are bounded; a truncation flag prevents silent scope
expansion. Financial records never cross the ordinary agent adapter here.
"""
from . import ledger


def snapshot(conn, profile_id, *, period_id):
    owned = not conn.in_transaction
    if owned:
        conn.execute("BEGIN")
    try:
        return _snapshot(conn, profile_id, period_id)
    finally:
        if owned:
            conn.rollback()


def _snapshot(conn, profile_id, period_id):
    book = ledger.require_book(conn, profile_id)
    period = ledger._period(conn, profile_id, period_id)
    rows = conn.execute("""SELECT status,COUNT(*) FROM gl_entries
        WHERE profile_id=? AND period_id=? GROUP BY status""", (profile_id, period_id))
    entry_counts = {row[0]: row[1] for row in rows}
    evidence_count = conn.execute("SELECT COUNT(*) FROM gl_evidence WHERE profile_id=?", (profile_id,)).fetchone()[0]
    statements = [dict(row) for row in conn.execute("""SELECT s.id,s.account_code,s.statement_id,s.start_date,s.end_date
        FROM gl_bank_statements s WHERE s.profile_id=? AND s.start_date<=? AND s.end_date>=?
        AND NOT EXISTS(SELECT 1 FROM gl_bank_statement_voids v WHERE v.statement_id=s.id)
        ORDER BY s.start_date,s.id LIMIT 1001""", (profile_id, period['end_date'], period['start_date']))]
    bank_pending = conn.execute("""SELECT COUNT(*) FROM gl_bank_rows r
        JOIN gl_bank_statements s ON s.id=r.statement_id AND s.profile_id=r.profile_id
        WHERE r.profile_id=? AND r.occurred_on BETWEEN ? AND ?
        AND NOT EXISTS(SELECT 1 FROM gl_bank_statement_voids v WHERE v.statement_id=s.id)
        AND abs(r.amount_minor)>COALESCE((SELECT SUM(a.amount_minor) FROM gl_bank_allocations a
          WHERE a.profile_id=r.profile_id AND a.row_id=r.id AND NOT EXISTS
          (SELECT 1 FROM gl_bank_allocation_voids v WHERE v.allocation_id=a.id)),0)""",
        (profile_id, period['start_date'], period['end_date'])).fetchone()[0]
    open_count = conn.execute("""SELECT COUNT(*) FROM gl_open_items i WHERE i.profile_id=?
        AND NOT EXISTS(SELECT 1 FROM gl_open_item_voids v WHERE v.item_id=i.id)
        AND i.amount_minor>COALESCE((SELECT SUM(a.amount_minor) FROM gl_open_item_allocations a
          WHERE a.profile_id=i.profile_id AND a.item_id=i.id AND NOT EXISTS
          (SELECT 1 FROM gl_open_item_allocation_voids v WHERE v.allocation_id=a.id)),0)""", (profile_id,)).fetchone()[0]
    unreviewed = conn.execute("""SELECT COUNT(*) FROM gl_evidence e WHERE e.profile_id=? AND NOT EXISTS
        (SELECT 1 FROM gl_evidence_extractions x JOIN gl_evidence_field_reviews r ON r.extraction_id=x.id
          WHERE x.evidence_id=e.id AND x.profile_id=e.profile_id)""", (profile_id,)).fetchone()[0]
    readiness = ledger.close_readiness(conn, profile_id, period_id=period_id)
    counts = dict(drafts=entry_counts.get('draft', 0), posted=entry_counts.get('posted', 0),
                  evidence=evidence_count, bank_unallocated=bank_pending, open_items=open_count,
                  evidence_unreviewed=unreviewed)
    items = []
    for kind, count, action, payload in (
        ('drafts', counts['drafts'], 'journal', {'period_id': period_id, 'status': 'draft'}),
        ('bank_unallocated', bank_pending, 'bank-list', {}),
        ('evidence_unreviewed', unreviewed, 'evidence-list', {}),
        ('open_items', open_count, 'item-list', {}),
    ):
        if count:
            items.append(dict(id=kind, kind=kind, status='needs_review', count=count,
                              target={'action': action, 'payload': payload}))
    for index, blocker in enumerate(readiness['blockers']):
        kind = blocker['kind']
        code = blocker.get('code', '')
        target = {'action': 'close-readiness', 'payload': {'period_id': period_id}}
        record = blocker.get('statement_id') or blocker.get('item_id') or blocker.get('evidence_id')
        if record:
            target['record_id'] = record
        items.append(dict(id=f'close:{index}', kind='close_blocker', status='blocked',
                          count=blocker.get('count', 1), code=code or kind, target=target))
    return dict(profile_id=profile_id, period_id=period_id, revision=book['revision'],
                counts=counts, items=items, readiness=readiness,
                count_scopes={'evidence': 'book', 'evidence_unreviewed': 'book', 'open_items': 'book',
                              'drafts': 'period', 'posted': 'period', 'bank_unallocated': 'period'},
                sources={'bank_statements': statements[:1000], 'truncated': len(statements)>1000},
                external_completeness_verified=False)
