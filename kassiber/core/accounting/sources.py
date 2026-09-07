"""Retained source identity and reviewed allocations, not an interpreter.

Custody remains authoritative for Bitcoin quantities. This Module binds its
observations and existing bank/document evidence to accounting recognition or
settlement identities. Stable source keys deliberately exclude content hashes:
changing an observation cannot evade an already retained quantity claim.
"""
from __future__ import annotations

from dataclasses import asdict
from datetime import datetime
from decimal import Decimal, InvalidOperation, localcontext
import json
from uuid import uuid4
from zoneinfo import ZoneInfo

from ...errors import AppError
from ...time_utils import now_iso
from .ledger import atomic, canonical_json, digest, require_book, strict_minor, _text, _row

MAX_SNAPSHOT_BYTES = 32 * 1024 * 1024


def ensure_schema(conn):
    for statement in (
        '''CREATE TABLE IF NOT EXISTS gl_source_snapshots(
            id TEXT PRIMARY KEY,profile_id TEXT NOT NULL REFERENCES gl_books(profile_id),
            input_digest TEXT NOT NULL,payload_json TEXT NOT NULL,created_at TEXT NOT NULL,
            UNIQUE(profile_id,id),UNIQUE(profile_id,input_digest))''',
        '''CREATE TABLE IF NOT EXISTS gl_source_bindings(
            id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,snapshot_id TEXT NOT NULL,
            economic_id TEXT NOT NULL,role TEXT NOT NULL CHECK(role IN ('recognition','settlement')),
            payload_digest TEXT NOT NULL,idempotency_key TEXT NOT NULL,reason TEXT NOT NULL,
            created_at TEXT NOT NULL,UNIQUE(profile_id,id),UNIQUE(profile_id,idempotency_key),
            FOREIGN KEY(profile_id,snapshot_id) REFERENCES gl_source_snapshots(profile_id,id))''',
        '''CREATE TABLE IF NOT EXISTS gl_source_claims(
            id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,binding_id TEXT NOT NULL,
            source_id TEXT NOT NULL,source_digest TEXT NOT NULL,unit TEXT NOT NULL CHECK(unit IN ('msat','minor')),
            start_atomic INTEGER NOT NULL CHECK(typeof(start_atomic)='integer' AND start_atomic>=0),
            end_atomic INTEGER NOT NULL CHECK(typeof(end_atomic)='integer' AND end_atomic>start_atomic),
            UNIQUE(profile_id,id),FOREIGN KEY(profile_id,binding_id) REFERENCES gl_source_bindings(profile_id,id))''',
        '''CREATE TABLE IF NOT EXISTS gl_source_binding_voids(
            binding_id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,reason TEXT NOT NULL,
            idempotency_key TEXT NOT NULL,created_at TEXT NOT NULL,UNIQUE(profile_id,idempotency_key),
            FOREIGN KEY(profile_id,binding_id) REFERENCES gl_source_bindings(profile_id,id))''',
        'CREATE INDEX IF NOT EXISTS gl_source_claim_lookup ON gl_source_claims(profile_id,source_id,start_atomic,end_atomic)',
    ):
        conn.execute(statement)
    for table, collision in (
        ('gl_source_snapshots', 'id=NEW.id OR (profile_id=NEW.profile_id AND input_digest=NEW.input_digest)'),
        ('gl_source_bindings', 'id=NEW.id OR (profile_id=NEW.profile_id AND idempotency_key=NEW.idempotency_key)'),
        ('gl_source_claims', 'id=NEW.id'),
        ('gl_source_binding_voids', 'binding_id=NEW.binding_id OR (profile_id=NEW.profile_id AND idempotency_key=NEW.idempotency_key)'),
    ):
        for action in ('UPDATE', 'DELETE'):
            conn.execute(f'''CREATE TRIGGER IF NOT EXISTS {table}_no_{action.lower()} BEFORE {action} ON {table}
                BEGIN SELECT RAISE(ABORT,'accounting_source_retained'); END''')
        conn.execute(f'''CREATE TRIGGER IF NOT EXISTS {table}_no_replace BEFORE INSERT ON {table}
            WHEN EXISTS(SELECT 1 FROM {table} WHERE {collision})
            BEGIN SELECT RAISE(ABORT,'accounting_source_retained'); END''')
    conn.execute('''CREATE TRIGGER IF NOT EXISTS gl_source_claim_overlap BEFORE INSERT ON gl_source_claims
        WHEN EXISTS(SELECT 1 FROM gl_source_claims c WHERE c.profile_id=NEW.profile_id
          AND c.source_id=NEW.source_id AND c.start_atomic<NEW.end_atomic AND c.end_atomic>NEW.start_atomic
          AND NOT EXISTS(SELECT 1 FROM gl_source_binding_voids v WHERE v.profile_id=c.profile_id AND v.binding_id=c.binding_id))
        BEGIN SELECT RAISE(ABORT,'accounting_source_claim_overlap'); END''')
    conn.execute('''CREATE TRIGGER IF NOT EXISTS gl_source_claim_snapshot BEFORE INSERT ON gl_source_claims
        WHEN NOT EXISTS(SELECT 1 FROM gl_source_bindings b JOIN gl_source_snapshots s
          ON s.id=b.snapshot_id AND s.profile_id=b.profile_id,json_each(s.payload_json,'$.sources') r
          WHERE b.id=NEW.binding_id AND b.profile_id=NEW.profile_id
            AND json_extract(r.value,'$.source_id')=NEW.source_id
            AND json_extract(r.value,'$.source_digest')=NEW.source_digest
            AND json_extract(r.value,'$.unit')=NEW.unit
            AND json_extract(r.value,'$.amount_atomic')>=NEW.end_atomic)
        BEGIN SELECT RAISE(ABORT,'accounting_source_claim_invalid'); END''')


def _has_table(conn, table):
    return bool(conn.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?", (table,)).fetchone())


def _source(kind, key, *, occurred_on, direction, amount, unit, facts):
    strict_minor(amount)
    result = dict(source_id=digest(dict(kind=kind, key=key)), kind=kind, source_key=key,
                  occurred_on=occurred_on, direction=direction, amount_atomic=amount, unit=unit, facts=facts)
    return {**result, 'source_digest': digest(result)}


def preview_sources(conn, profile_id):
    """Read current canonical sources. Never runs RP2 or writes journal state."""
    book = require_book(conn, profile_id)
    records, blockers, decisions = [], [], []
    if _has_table(conn, 'transactions') and conn.execute('SELECT 1 FROM transactions WHERE profile_id=? AND excluded=0 LIMIT 1', (profile_id,)).fetchone():
        from ..custody_journal import CustodyJournalBuilder
        profile = _row(conn, 'SELECT * FROM profiles WHERE id=?', (profile_id,))
        custody = CustodyJournalBuilder(conn, profile).build_custody_decisions()
        raw_by_id = {str(row['id']): row for row in custody.rows}
        for observation in custody.quantity_state.projection.observations:
            if observation.profile_id != profile_id:
                raise AppError('Custody source belongs to another book', code='accounting_scope_changed')
            raw = raw_by_id.get(observation.anchor_transaction_id, {})
            prices = {key: raw[key] for key in ('fiat_currency','fiat_rate_exact','fiat_value_exact','pricing_source_kind','pricing_quality') if key in raw.keys()}
            facts = dict(anchor_transaction_id=observation.anchor_transaction_id,
                         observation_hash=observation.quantity_hash, evidence_digest=observation.evidence_detail_hash,
                         occurred_at=observation.occurred_at, asset=observation.asset, wallet_id=observation.wallet_id,
                         fee_msat=observation.fee_msat, amount_includes_fee=observation.amount_includes_fee,
                         prices=prices)
            # Native event + wallet + side survives value/interpretation revision.
            key = dict(event=asdict(observation.event_key), wallet_id=observation.wallet_id,
                       asset=observation.asset, direction=observation.direction)
            occurred = datetime.fromisoformat(observation.occurred_at.replace('Z', '+00:00'))
            if occurred.tzinfo is None:
                raise AppError('Source timestamp needs an explicit timezone', code='accounting_source_time')
            day = occurred.astimezone(ZoneInfo(book['timezone'])).date().isoformat()
            amount = observation.amount_msat
            if observation.direction == 'outbound' and not observation.amount_includes_fee:
                amount += observation.fee_msat
            records.append(_source('custody', key, occurred_on=day, direction=observation.direction,
                                   amount=amount, unit='msat', facts=facts))
        decisions = [asdict(item) for item in custody.quantity_state.projection.decisions]
        blockers.extend(dict(code=issue.issue_type, occurred_at=issue.occurred_at,
                             transaction_ids=list(issue.transaction_ids)) for issue in custody.quantity_state.issues)
        blockers.extend(dict(code='custody_component_blocked', component_id=item.get('id')) for item in custody.component_blockers)
    if _has_table(conn, 'gl_bank_rows'):
        for row in conn.execute('''SELECT r.*,s.account_code,s.statement_id AS statement_ref,s.payload_digest
            FROM gl_bank_rows r JOIN gl_bank_statements s ON s.id=r.statement_id AND s.profile_id=r.profile_id
            WHERE r.profile_id=? AND NOT EXISTS(SELECT 1 FROM gl_bank_statement_voids v WHERE v.statement_id=s.id)''', (profile_id,)):
            key = dict(account_code=row['account_code'], statement_id=row['statement_ref'], row_id=row['row_id'])
            records.append(_source('bank', key, occurred_on=row['occurred_on'], direction='inbound' if row['amount_minor'] > 0 else 'outbound',
                                   amount=abs(row['amount_minor']), unit='minor',
                                   facts=dict(bank_row_id=row['id'], statement_digest=row['payload_digest'], description=row['description'])))
    documents = []
    if _has_table(conn, 'external_documents'):
        for raw in conn.execute('SELECT * FROM external_documents WHERE profile_id=? ORDER BY id', (profile_id,)):
            doc = dict(raw)
            metadata = {key: doc.get(key) for key in ('id','document_type','external_ref','issued_at','due_at',
                                                     'fiat_currency','fiat_value_exact','review_state','updated_at')}
            documents.append(metadata)
            if doc.get('document_type') not in ('invoice', 'receipt'):
                continue
            try:
                with localcontext() as ctx:
                    ctx.prec = 80
                    value = Decimal(doc.get('fiat_value_exact') or '') * 10**book['minor_unit_exponent']
                    if not value.is_finite() or value <= 0 or value != value.to_integral_value() or value > 2**63 - 1:
                        raise ValueError()
                    amount = int(value)
                issued = str(doc.get('issued_at') or '')
                if len(issued) == 10:
                    day = datetime.strptime(issued, '%Y-%m-%d').date().isoformat()
                else:
                    instant = datetime.fromisoformat(issued.replace('Z', '+00:00'))
                    if instant.tzinfo is None:
                        raise ValueError()
                    day = instant.astimezone(ZoneInfo(book['timezone'])).date().isoformat()
                if doc.get('fiat_currency') != book['currency']:
                    raise ValueError()
            except (InvalidOperation, ValueError, TypeError, OverflowError):
                blockers.append(dict(code='accounting_document_needs_valuation', document_id=doc['id']))
                continue
            # Invoice direction requires an accounting decision; its type does
            # not establish whether this organization issued or received it.
            records.append(_source('document', dict(document_id=doc['id']), occurred_on=day,
                                   direction='unspecified', amount=amount, unit='minor', facts=metadata))
    # Existing commercial identities and reviewed links are retained as facts;
    # this Module does not infer or rewrite their classification authority.
    commercial = []
    if _has_table(conn, 'commercial_links'):
        commercial = [dict(row) for row in conn.execute('''SELECT id,document_id,transaction_id,btcpay_record_id,
            link_type,state,allocation_amount,allocation_fiat_exact,commercial_kind,updated_at
            FROM commercial_links WHERE profile_id=? ORDER BY id''', (profile_id,))]
    identities = [row['source_id'] for row in records]
    if len(set(identities)) != len(identities):
        raise AppError('Multiple observations need an unambiguous source identity', code='accounting_source_identity_conflict')
    profile_row = _row(conn, 'SELECT * FROM profiles WHERE id=?', (profile_id,))
    payload = dict(schema_version=1, profile_id=profile_id, currency=book['currency'],
                   minor_unit_exponent=book['minor_unit_exponent'], timezone=book['timezone'],
                   journal_input_version=profile_row.get('journal_input_version', 0),
                   calculation_policy={key: profile_row.get(key) for key in ('tax_country','tax_long_term_days','gains_algorithm','fiat_currency',
                                                                              'cost_basis_pool_scope','bitcoin_rail_carrying_value')},
                   sources=sorted(records, key=lambda x: x['source_id']), decisions=decisions,
                   documents=documents, commercial_links=commercial, blockers=blockers)
    return {**payload, 'input_digest': digest(payload)}


def capture_sources(conn, profile_id):
    with atomic(conn):
        payload = preview_sources(conn, profile_id)
        checksum = payload.pop('input_digest')
        text = canonical_json(payload)
        if len(text.encode()) > MAX_SNAPSHOT_BYTES:
            raise AppError('Accounting source snapshot exceeds the retention limit', code='accounting_source_limit')
        existing = _row(conn, 'SELECT id FROM gl_source_snapshots WHERE profile_id=? AND input_digest=?', (profile_id, checksum))
        if existing:
            return get_snapshot(conn, profile_id, existing['id'])
        snapshot_id = uuid4().hex
        conn.execute('INSERT INTO gl_source_snapshots VALUES(?,?,?,?,?)', (snapshot_id, profile_id, checksum, text, now_iso()))
        conn.execute('UPDATE gl_books SET revision=revision+1 WHERE profile_id=?', (profile_id,))
        return get_snapshot(conn, profile_id, snapshot_id)


def get_snapshot(conn, profile_id, snapshot_id):
    require_book(conn, profile_id)
    row = _row(conn, 'SELECT * FROM gl_source_snapshots WHERE profile_id=? AND id=?', (profile_id, snapshot_id))
    if not row:
        raise AppError('Source snapshot was not found in this book', code='not_found')
    payload = json.loads(row.pop('payload_json'))
    if digest(payload) != row['input_digest'] or payload['profile_id'] != profile_id:
        raise AppError('Retained source snapshot failed verification', code='accounting_source_corrupt')
    return {**row, 'snapshot': payload}


def require_current(conn, profile_id, snapshot_id):
    saved = get_snapshot(conn, profile_id, snapshot_id)
    if preview_sources(conn, profile_id)['input_digest'] != saved['input_digest']:
        raise AppError('Accounting sources changed; review a new snapshot', code='accounting_source_stale')
    return saved


def get_binding(conn, profile_id, binding_id):
    require_book(conn, profile_id)
    row = _row(conn, 'SELECT * FROM gl_source_bindings WHERE profile_id=? AND id=?', (profile_id, binding_id))
    if not row:
        raise AppError('Source binding was not found in this book', code='not_found')
    row['claims'] = [dict(r) for r in conn.execute('SELECT * FROM gl_source_claims WHERE profile_id=? AND binding_id=? ORDER BY source_id,start_atomic', (profile_id, binding_id))]
    row['voided'] = bool(conn.execute('SELECT 1 FROM gl_source_binding_voids WHERE profile_id=? AND binding_id=?', (profile_id, binding_id)).fetchone())
    return row


def bind_sources(conn, profile_id, *, snapshot_id, expected_digest, economic_id, role, claims, reason, idempotency_key):
    """Assign exact source slices once; mixed units are never summed together."""
    require_book(conn, profile_id)
    for name, value in (('economic_id', economic_id), ('reason', reason), ('idempotency_key', idempotency_key)):
        _text(value, name, maximum=1000 if name == 'reason' else 128)
    # The retained snapshot is itself bounded to 32 MiB. Opening migration may
    # claim its complete history server-side; transport still bounds payloads.
    if role not in ('recognition', 'settlement') or not isinstance(claims, list) or not 1 <= len(claims) <= 100_000:
        raise AppError('Invalid source binding', code='accounting_validation')
    normalized = []
    for claim in claims:
        if not isinstance(claim, dict) or set(claim) != {'source_id','start_atomic','end_atomic'}:
            raise AppError('A source claim needs an exact slice', code='accounting_validation')
        _text(claim['source_id'], 'source_id', maximum=64)
        start, end = strict_minor(claim['start_atomic']), strict_minor(claim['end_atomic'])
        if start >= end:
            raise AppError('Source slice must have positive length', code='accounting_validation')
        normalized.append(dict(source_id=claim['source_id'], start_atomic=start, end_atomic=end))
    normalized.sort(key=lambda c: (c['source_id'], c['start_atomic']))
    commitment = digest(dict(snapshot_id=snapshot_id, expected_digest=expected_digest, economic_id=economic_id,
                             role=role, claims=normalized, reason=reason))
    with atomic(conn):
        existing = _row(conn, 'SELECT id,payload_digest FROM gl_source_bindings WHERE profile_id=? AND idempotency_key=?', (profile_id, idempotency_key))
        if existing:
            if existing['payload_digest'] != commitment:
                raise AppError('Idempotency key belongs to a different source binding', code='accounting_idempotency_conflict')
            return get_binding(conn, profile_id, existing['id'])
        snapshot = require_current(conn, profile_id, snapshot_id)
        if expected_digest != snapshot['input_digest']:
            raise AppError('Source approval does not match the snapshot', code='accounting_stale_approval')
        available = {r['source_id']: r for r in snapshot['snapshot']['sources']}
        previous = None
        for claim in normalized:
            source = available.get(claim['source_id'])
            if source is None or claim['end_atomic'] > source['amount_atomic']:
                raise AppError('Source claim exceeds this snapshot', code='accounting_source_exceeded')
            from .bank import require_open_interval
            require_open_interval(conn, profile_id, source['occurred_on'], source['occurred_on'])
            if previous and previous['source_id'] == claim['source_id'] and previous['end_atomic'] > claim['start_atomic']:
                raise AppError('Source claims overlap', code='accounting_source_claim_overlap')
            previous = claim
            if conn.execute('''SELECT 1 FROM gl_source_claims c WHERE c.profile_id=? AND c.source_id=?
                AND c.start_atomic<? AND c.end_atomic>? AND NOT EXISTS(
                SELECT 1 FROM gl_source_binding_voids v WHERE v.profile_id=c.profile_id AND v.binding_id=c.binding_id)''',
                (profile_id, claim['source_id'], claim['end_atomic'], claim['start_atomic'])).fetchone():
                raise AppError('This source quantity is already assigned', code='accounting_source_claim_overlap')
        binding_id = uuid4().hex
        conn.execute('INSERT INTO gl_source_bindings VALUES(?,?,?,?,?,?,?,?,?)',
                     (binding_id, profile_id, snapshot_id, economic_id, role, commitment, idempotency_key, reason, now_iso()))
        for claim in normalized:
            source = available[claim['source_id']]
            conn.execute('INSERT INTO gl_source_claims VALUES(?,?,?,?,?,?,?,?)',
                         (uuid4().hex, profile_id, binding_id, source['source_id'], source['source_digest'], source['unit'], claim['start_atomic'], claim['end_atomic']))
        conn.execute('UPDATE gl_books SET revision=revision+1 WHERE profile_id=?', (profile_id,))
        return get_binding(conn, profile_id, binding_id)


def void_binding(conn, profile_id, *, binding_id, reason, idempotency_key):
    require_book(conn, profile_id)
    _text(reason, 'reason')
    _text(idempotency_key, 'idempotency_key', maximum=128)
    with atomic(conn):
        existing = _row(conn, 'SELECT * FROM gl_source_binding_voids WHERE profile_id=? AND idempotency_key=?', (profile_id, idempotency_key))
        if existing:
            if existing['binding_id'] != binding_id or existing['reason'] != reason:
                raise AppError('Idempotency key belongs to another source correction', code='accounting_idempotency_conflict')
            return get_binding(conn, profile_id, binding_id)
        binding = get_binding(conn, profile_id, binding_id)
        if binding['voided']:
            raise AppError('This source binding is already void', code='accounting_source_binding_void')
        if _has_table(conn, 'gl_projection_proposals'):
            from .projection import require_binding_releasable
            require_binding_releasable(conn, profile_id, binding_id)
        from .bank import require_open_interval
        snapshot = get_snapshot(conn, profile_id, binding['snapshot_id'])
        sources = {r['source_id']: r for r in snapshot['snapshot']['sources']}
        for claim in binding['claims']:
            source = sources[claim['source_id']]
            require_open_interval(conn, profile_id, source['occurred_on'], source['occurred_on'])
        conn.execute('INSERT INTO gl_source_binding_voids VALUES(?,?,?,?,?)', (binding_id, profile_id, reason, idempotency_key, now_iso()))
        conn.execute('UPDATE gl_books SET revision=revision+1 WHERE profile_id=?', (profile_id,))
        return get_binding(conn, profile_id, binding_id)


def source_coverage(conn, profile_id):
    """Coverage means reviewed assignments, not posted or externally complete."""
    with atomic(conn):
        current = preview_sources(conn, profile_id)
        rows, stale = [], []
        current_by_id = {r['source_id']: r for r in current['sources']}
        assigned = {}
        for claim in conn.execute('''SELECT c.* FROM gl_source_claims c WHERE c.profile_id=? AND NOT EXISTS(
            SELECT 1 FROM gl_source_binding_voids v WHERE v.profile_id=c.profile_id AND v.binding_id=c.binding_id)''', (profile_id,)):
            source = current_by_id.get(claim['source_id'])
            if source is None or source['source_digest'] != claim['source_digest']:
                stale.append(dict(binding_id=claim['binding_id'], source_id=claim['source_id']))
            assigned[claim['source_id']] = assigned.get(claim['source_id'], 0) + claim['end_atomic'] - claim['start_atomic']
        for source in current['sources']:
            used = assigned.get(source['source_id'], 0)
            rows.append(dict(source_id=source['source_id'], unit=source['unit'], amount_atomic=source['amount_atomic'],
                             assigned_atomic=used, remaining_atomic=source['amount_atomic'] - used))
        return dict(input_digest=current['input_digest'], rows=rows, stale_bindings=stale,
                    blockers=current['blockers'], external_completeness_verified=False)
