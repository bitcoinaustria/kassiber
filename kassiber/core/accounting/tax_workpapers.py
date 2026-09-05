"""Encrypted, append-only, reviewed tax preparation bound to ledger closes.

No filing, signing, remote access, arbitrary policy code or implicit AI authority.
All operations share the ledger's caller-owned transaction and profile boundary.
"""
from __future__ import annotations

import base64
import binascii
from datetime import date
import html
import hashlib
import json
import re
from uuid import uuid4

from . import jurisdiction
from .ledger import atomic, canonical_json, digest, require_book, _row, _rows
from ...errors import AppError
from ...time_utils import now_iso

READ_ACTIONS = frozenset({'tax-packs', 'tax-list', 'tax-get', 'tax-preview', 'tax-export'})
WRITE_ACTIONS = frozenset({'tax-create', 'tax-review', 'tax-finalize'})
MAX_STATE_BYTES = 8 * 1024 * 1024
MAX_PATCH_BYTES = 512 * 1024
MAX_INSTANCES = 256
MAX_RECORDS = 2000


def _fail(message, code='accounting_tax_validation'):
    raise AppError(message, code=code)


def _text(value, name, limit=2000):
    if not isinstance(value, str) or not value.strip() or len(value) > limit:
        _fail(f'{name} must be nonempty bounded text')
    return value.strip()


def _identifier(value):
    if not isinstance(value, str) or not re.fullmatch(r'[A-Za-z0-9_-]{1,80}', value):
        _fail('Invalid working-paper record identifier')
    return value


def _integer(value, *, unsigned=False):
    if type(value) is not int or abs(value) > jurisdiction.MAX_FORM_MINOR or (unsigned and value < 0):
        _fail('Form amounts require exact bounded integer minor units')
    return value


def _fields(value, allowed, required=()):
    if not isinstance(value, dict) or set(value) - set(allowed) or set(required) - set(value):
        _fail('Unknown or missing tax preparation fields')
    return value


def _bounded_json(value, maximum):
    try:
        encoded = canonical_json(value)
    except (ValueError, TypeError, RecursionError) as exc:
        raise AppError('Invalid tax preparation JSON', code='accounting_tax_validation') from exc
    if len(encoded.encode()) > maximum:
        _fail('Tax preparation payload exceeds its bounded size')
    return encoded


def ensure_schema(conn):
    statements = [
        """CREATE TABLE IF NOT EXISTS gl_tax_workpapers (
            id TEXT PRIMARY KEY, profile_id TEXT NOT NULL, period_id TEXT NOT NULL,
            pack_id TEXT NOT NULL, pack_digest TEXT NOT NULL, created_at TEXT NOT NULL,
            idempotency_key TEXT NOT NULL, payload_digest TEXT NOT NULL,
            UNIQUE(profile_id,id), UNIQUE(profile_id,idempotency_key), UNIQUE(profile_id,period_id,pack_id),
            FOREIGN KEY(profile_id,period_id) REFERENCES gl_periods(profile_id,id) ON DELETE RESTRICT)""",
        """CREATE TABLE IF NOT EXISTS gl_tax_revisions (
            id TEXT PRIMARY KEY, profile_id TEXT NOT NULL, workpaper_id TEXT NOT NULL,
            revision INTEGER NOT NULL CHECK(typeof(revision)='integer' AND revision>0),
            state_json TEXT NOT NULL, state_digest TEXT NOT NULL, reason TEXT NOT NULL,
            idempotency_key TEXT NOT NULL, payload_digest TEXT NOT NULL, created_at TEXT NOT NULL,
            UNIQUE(profile_id,id), UNIQUE(profile_id,workpaper_id,revision), UNIQUE(profile_id,workpaper_id,idempotency_key),
            FOREIGN KEY(profile_id,workpaper_id) REFERENCES gl_tax_workpapers(profile_id,id) ON DELETE RESTRICT)""",
        """CREATE TABLE IF NOT EXISTS gl_tax_finals (
            id TEXT PRIMARY KEY, profile_id TEXT NOT NULL, workpaper_id TEXT NOT NULL,
            revision_id TEXT NOT NULL, close_event_id TEXT NOT NULL,
            input_digest TEXT NOT NULL, report_json TEXT NOT NULL, report_digest TEXT NOT NULL,
            created_at TEXT NOT NULL, UNIQUE(profile_id,id), UNIQUE(profile_id,workpaper_id,input_digest),
            FOREIGN KEY(profile_id,workpaper_id) REFERENCES gl_tax_workpapers(profile_id,id) ON DELETE RESTRICT,
            FOREIGN KEY(profile_id,revision_id) REFERENCES gl_tax_revisions(profile_id,id) ON DELETE RESTRICT,
            FOREIGN KEY(close_event_id) REFERENCES gl_period_events(id) ON DELETE RESTRICT)""",
        """CREATE TABLE IF NOT EXISTS gl_tax_evidence_refs (
            profile_id TEXT NOT NULL, revision_id TEXT NOT NULL, evidence_id TEXT NOT NULL,
            PRIMARY KEY(profile_id,revision_id,evidence_id),
            FOREIGN KEY(profile_id,revision_id) REFERENCES gl_tax_revisions(profile_id,id) ON DELETE RESTRICT,
            FOREIGN KEY(evidence_id) REFERENCES gl_evidence(id) ON DELETE RESTRICT)""",
        """CREATE TABLE IF NOT EXISTS gl_tax_line_refs (
            profile_id TEXT NOT NULL, revision_id TEXT NOT NULL, line_id TEXT NOT NULL,
            PRIMARY KEY(profile_id,revision_id,line_id),
            FOREIGN KEY(profile_id,revision_id) REFERENCES gl_tax_revisions(profile_id,id) ON DELETE RESTRICT,
            FOREIGN KEY(line_id) REFERENCES gl_lines(id) ON DELETE RESTRICT)""",
        """CREATE TABLE IF NOT EXISTS gl_tax_prior_refs (
            profile_id TEXT NOT NULL, revision_id TEXT NOT NULL, final_id TEXT NOT NULL,
            PRIMARY KEY(profile_id,revision_id,final_id),
            FOREIGN KEY(profile_id,revision_id) REFERENCES gl_tax_revisions(profile_id,id) ON DELETE RESTRICT,
            FOREIGN KEY(profile_id,final_id) REFERENCES gl_tax_finals(profile_id,id) ON DELETE RESTRICT)""",
    ]
    for statement in statements:
        conn.execute(statement)
    conn.execute('CREATE UNIQUE INDEX IF NOT EXISTS gl_tax_one_assessment_pack ON gl_tax_workpapers(profile_id,pack_id)')
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_tax_assessment_no_replace BEFORE INSERT ON gl_tax_workpapers
        WHEN EXISTS(SELECT 1 FROM gl_tax_workpapers WHERE profile_id=NEW.profile_id AND pack_id=NEW.pack_id)
        BEGIN SELECT RAISE(ABORT,'accounting tax replacement forbidden'); END""")
    collisions = {
        'gl_tax_workpapers': 'id=NEW.id OR (profile_id=NEW.profile_id AND (idempotency_key=NEW.idempotency_key OR (period_id=NEW.period_id AND pack_id=NEW.pack_id)))',
        'gl_tax_revisions': 'id=NEW.id OR (profile_id=NEW.profile_id AND workpaper_id=NEW.workpaper_id AND (revision=NEW.revision OR idempotency_key=NEW.idempotency_key))',
        'gl_tax_finals': 'id=NEW.id OR (profile_id=NEW.profile_id AND workpaper_id=NEW.workpaper_id AND input_digest=NEW.input_digest)',
        'gl_tax_evidence_refs': 'profile_id=NEW.profile_id AND revision_id=NEW.revision_id AND evidence_id=NEW.evidence_id',
        'gl_tax_line_refs': 'profile_id=NEW.profile_id AND revision_id=NEW.revision_id AND line_id=NEW.line_id',
        'gl_tax_prior_refs': 'profile_id=NEW.profile_id AND revision_id=NEW.revision_id AND final_id=NEW.final_id',
    }
    for table, condition in collisions.items():
        for operation in ('UPDATE', 'DELETE'):
            conn.execute(f"CREATE TRIGGER IF NOT EXISTS {table}_immutable_{operation.lower()} BEFORE {operation} ON {table} BEGIN SELECT RAISE(ABORT,'accounting tax record retained'); END")
        conn.execute(f"CREATE TRIGGER IF NOT EXISTS {table}_no_replace BEFORE INSERT ON {table} WHEN EXISTS(SELECT 1 FROM {table} WHERE {condition}) BEGIN SELECT RAISE(ABORT,'accounting tax replacement forbidden'); END")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_tax_evidence_scope BEFORE INSERT ON gl_tax_evidence_refs
        WHEN NOT EXISTS(SELECT 1 FROM gl_evidence WHERE id=NEW.evidence_id AND profile_id=NEW.profile_id)
        BEGIN SELECT RAISE(ABORT,'accounting tax evidence scope'); END""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_tax_line_scope BEFORE INSERT ON gl_tax_line_refs
        WHEN NOT EXISTS(SELECT 1 FROM gl_lines l JOIN gl_entries e ON e.id=l.entry_id AND e.profile_id=l.profile_id
            WHERE l.id=NEW.line_id AND l.profile_id=NEW.profile_id AND e.status='posted')
        BEGIN SELECT RAISE(ABORT,'accounting tax line scope'); END""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_tax_final_scope BEFORE INSERT ON gl_tax_finals
        WHEN NOT EXISTS(SELECT 1 FROM gl_period_events e JOIN gl_tax_workpapers w ON w.profile_id=e.profile_id AND w.period_id=e.period_id
            JOIN gl_tax_revisions r ON r.workpaper_id=w.id AND r.profile_id=w.profile_id
            WHERE w.id=NEW.workpaper_id AND w.profile_id=NEW.profile_id AND r.id=NEW.revision_id AND e.id=NEW.close_event_id AND e.action='close')
        BEGIN SELECT RAISE(ABORT,'accounting tax final scope'); END""")


def _workpaper(conn, profile_id, workpaper_id):
    require_book(conn, profile_id)
    result = _row(conn, 'SELECT * FROM gl_tax_workpapers WHERE profile_id=? AND id=?', (profile_id, workpaper_id))
    if not result:
        _fail('Working paper not found in this book', 'accounting_tax_not_found')
    return result


def _period(conn, profile_id, period_id):
    result = _row(conn, 'SELECT * FROM gl_periods WHERE profile_id=? AND id=?', (profile_id, period_id))
    if not result:
        _fail('Fiscal period not found in this book')
    return result


def _latest_revision(conn, profile_id, workpaper_id):
    result = _row(conn, 'SELECT * FROM gl_tax_revisions WHERE profile_id=? AND workpaper_id=? ORDER BY revision DESC LIMIT 1', (profile_id, workpaper_id))
    if not result or digest(json.loads(result['state_json'])) != result['state_digest']:
        _fail('Working-paper revision integrity failed', 'accounting_tax_integrity')
    return result


def _pack(workpaper):
    result = jurisdiction.get_pack(workpaper['pack_id'])
    if result['digest'] != workpaper['pack_digest']:
        _fail('Jurisdiction pack changed; preserve the old version before continuing', 'accounting_tax_pack_changed')
    return result


def _references(conn, profile_id, obj, *, validate=True):
    evidence, lines, finals = set(), set(), set()

    def walk(value):
        if isinstance(value, dict):
            for key, child in value.items():
                if key in ('evidence_ids', 'line_ids'):
                    if not isinstance(child, list) or len(child) > 100 or any(not isinstance(v, str) for v in child):
                        _fail('Evidence and line references must be bounded identifier lists')
                    (evidence if key == 'evidence_ids' else lines).update(child)
                elif key == 'source_final_id' and child is not None:
                    finals.add(_identifier(child))
                else:
                    walk(child)
        elif isinstance(value, list):
            for child in value:
                walk(child)
    walk(obj)
    if validate:
        for identifier in evidence:
            if not conn.execute('SELECT 1 FROM gl_evidence WHERE profile_id=? AND id=?', (profile_id, identifier)).fetchone():
                _fail('Evidence is not retained in this book', 'accounting_tax_scope')
        for identifier in lines:
            if not conn.execute("SELECT 1 FROM gl_lines l JOIN gl_entries e ON e.id=l.entry_id AND e.profile_id=l.profile_id WHERE l.profile_id=? AND l.id=? AND e.status='posted'", (profile_id, identifier)).fetchone():
                _fail('Source line is not posted in this book', 'accounting_tax_scope')
        for identifier in finals:
            if not conn.execute('SELECT 1 FROM gl_tax_finals WHERE profile_id=? AND id=?', (profile_id, identifier)).fetchone():
                _fail('Prior final is not retained in this book', 'accounting_tax_scope')
    return evidence, lines, finals


def _insert_revision(conn, profile_id, workpaper_id, revision, state, reason, key, request_digest):
    encoded = _bounded_json(state, MAX_STATE_BYTES)
    references = _references(conn, profile_id, state)
    identifier = uuid4().hex
    conn.execute('INSERT INTO gl_tax_revisions VALUES(?,?,?,?,?,?,?,?,?,?)',
                 (identifier, profile_id, workpaper_id, revision, encoded, digest(state), reason, key, request_digest, now_iso()))
    for table, identifiers in zip(('gl_tax_evidence_refs', 'gl_tax_line_refs', 'gl_tax_prior_refs'), references):
        conn.executemany(f'INSERT INTO {table} VALUES(?,?,?)', [(profile_id, identifier, item) for item in sorted(identifiers)])
    return identifier


def create_workpaper(conn, profile_id, *, period_id, pack_id, idempotency_key):
    book = require_book(conn, profile_id)
    pack = jurisdiction.get_pack(pack_id)
    key = _text(idempotency_key, 'idempotency_key', 160)
    request_digest = digest(dict(period_id=period_id, pack_id=pack_id))
    with atomic(conn):
        previous = _row(conn, 'SELECT * FROM gl_tax_workpapers WHERE profile_id=? AND idempotency_key=?', (profile_id, key))
        if previous:
            if previous['payload_digest'] != request_digest:
                _fail('Idempotency key was used for different tax preparation inputs')
            return get_workpaper(conn, profile_id, workpaper_id=previous['id'])
        period = _period(conn, profile_id, period_id)
        if int(period['end_date'][:4]) != pack['tax_year']:
            _fail('Pack year must match the fiscal period end year')
        if (book['currency'], book['minor_unit_exponent']) != (pack['currency'], pack['minor_unit_exponent']):
            _fail('This form requires a EUR book with cent precision')
        if conn.execute('SELECT 1 FROM gl_tax_workpapers WHERE profile_id=? AND pack_id=?', (profile_id, pack_id)).fetchone():
            _fail('Revise the existing assessment-year working paper for this book and pack')
        identifier = uuid4().hex
        conn.execute('INSERT INTO gl_tax_workpapers VALUES(?,?,?,?,?,?,?,?)',
                     (identifier, profile_id, period_id, pack_id, pack['digest'], now_iso(), key, request_digest))
        state = dict(facts={}, annex_instances=[dict(id='main', form_id='K2', label='K2')],
                     field_reviews={}, mappings=[], adjustments=[], carryforwards=[], exclusions={})
        _insert_revision(conn, profile_id, identifier, 1, state, 'Created reviewed tax preparation workspace', 'create', request_digest)
    return get_workpaper(conn, profile_id, workpaper_id=identifier)


def _validate_review(review, definition):
    _fields(review, {'state', 'value', 'value_minor', 'reason', 'evidence_ids', 'line_ids'}, {'state', 'reason'})
    state = review['state']
    if state not in ('reviewed_input', 'not_applicable', 'blocked'):
        _fail('Only the deterministic engine may author derived values')
    _text(review['reason'], 'Review reason')
    if state != 'reviewed_input':
        if 'value' in review or 'value_minor' in review:
            _fail('Blocked/not-applicable fields must not carry a value')
        if state == 'not_applicable' and definition.get('required'):
            _fail('Mandatory identity/fact cannot be declared not applicable')
        return
    kind = definition['type']
    value_key = 'value_minor' if kind == 'money' else 'value'
    if value_key not in review or ({'value', 'value_minor'} - {value_key}) & set(review):
        _fail('Reviewed field requires its correctly typed value')
    value = review[value_key]
    if kind == 'money':
        _integer(value)
        if definition.get('negative_only') and value > 0:
            _fail('This loss field requires a negative amount or zero')
    elif kind == 'boolean':
        if type(value) is not bool:
            _fail('Boolean field requires an explicit boolean')
    elif kind in ('integer', 'percent'):
        if type(value) is not int or not 0 <= value <= (10000 if kind == 'percent' else 9999):
            _fail('Integer/percentage field requires a bounded exact integer (percent in basis points)')
    elif kind == 'date':
        try:
            if not isinstance(value, str) or date.fromisoformat(value).isoformat() != value:
                raise ValueError()
        except ValueError:
            _fail('Date requires YYYY-MM-DD')
    elif kind == 'choice':
        if value not in definition['choices']:
            _fail('Invalid reviewed factual choice')
    elif kind == 'forms':
        if not isinstance(value, list) or any(not isinstance(v, str) for v in value) or len(value) != len(set(value)) or any(v not in ('K2kv', 'K2a', 'K2b', 'K11', 'K12', 'K12a') for v in value):
            _fail('Invalid required-annex selection')
    else:
        _text(value, 'Field value')


def _validate_state(conn, profile_id, pack, state):
    facts = {item['id']: item for item in pack['facts']}
    if not isinstance(state['facts'], dict) or set(state['facts']) - set(facts):
        _fail('Unknown organization fact')
    for key, review in state['facts'].items():
        _validate_review(review, facts[key])
    instances = state['annex_instances']
    if not isinstance(instances, list) or not 1 <= len(instances) <= MAX_INSTANCES:
        _fail('Invalid annex instance collection')
    targets = {}
    seen = set()
    for instance in instances:
        _fields(instance, {'id', 'form_id', 'label'}, {'id', 'form_id', 'label'})
        identifier = _identifier(instance['id'])
        if identifier in seen:
            _fail('Duplicate annex instance')
        seen.add(identifier)
        _text(instance['label'], 'Annex label', 200)
        if (instance['form_id'] == 'K2') != (identifier == 'main'):
            _fail('Exactly one main K2 form is required')
        targets.update({identifier + '.' + key: definition for key, definition in jurisdiction.fields_for(pack, instance['form_id']).items()})
    if 'main' not in seen:
        _fail('Main K2 form missing')
    if not isinstance(state['field_reviews'], dict) or set(state['field_reviews']) - set(targets):
        _fail('Review targets must exist in the selected form version')
    for key, review in state['field_reviews'].items():
        _validate_review(review, targets[key])
    for section in ('mappings', 'adjustments', 'carryforwards'):
        if not isinstance(state[section], list) or len(state[section]) > MAX_RECORDS:
            _fail('Invalid bounded supporting tax records')
        identifiers = set()
        for record in state[section]:
            if not isinstance(record, dict) or _identifier(record.get('id')) in identifiers:
                _fail('Duplicate or malformed supporting tax record')
            identifiers.add(record['id'])
            _text(record.get('reason'), 'Supporting record reason')
            if section != 'carryforwards':
                target = targets.get(record.get('field_key'))
                if target is None or target['type'] != 'money':
                    _fail('Monetary source must target a selected monetary form field')
                _integer(record.get('amount_minor'))
            if section == 'mappings':
                _fields(record, {'id', 'field_key', 'account_code', 'basis', 'amount_minor', 'multiplier', 'reason', 'evidence_ids'}, {'id', 'field_key', 'account_code', 'basis', 'amount_minor', 'multiplier', 'reason'})
                if record['basis'] not in ('movement', 'balance') or type(record['multiplier']) is not int or record['multiplier'] not in (-1, 1):
                    _fail('Mapping requires an explicit movement/balance basis and sign')
                if not conn.execute('SELECT 1 FROM gl_accounts WHERE profile_id=? AND code=?', (profile_id, record['account_code'])).fetchone():
                    _fail('Mapped account is not in this book', 'accounting_tax_scope')
            elif section == 'adjustments':
                _fields(record, {'id', 'field_key', 'amount_minor', 'category', 'reason', 'evidence_ids', 'line_ids'}, {'id', 'field_key', 'amount_minor', 'category', 'reason'})
                if record['category'] not in ('permanent', 'temporary', 'tax_opening', 'scope', 'withholding', 'specialist'):
                    _fail('Invalid book-to-tax adjustment category')
            else:
                _fields(record, {'id', 'kind', 'vintage_year', 'opening_minor', 'addition_minor', 'used_minor', 'expired_minor', 'closing_minor', 'source_final_id', 'source_carry_id', 'reason', 'evidence_ids'}, {'id', 'kind', 'vintage_year', 'opening_minor', 'addition_minor', 'used_minor', 'expired_minor', 'closing_minor', 'reason'})
                if record['kind'] not in ('loss', 'section23_allowance', 'foreign_tax_credit', 'interest', 'ebitda', 'donation', 'property_loss', 'suspended_loss', 'other_reviewed'):
                    _fail('Unknown carryforward type')
                if type(record['vintage_year']) is not int or not 1900 <= record['vintage_year'] <= pack['tax_year']:
                    _fail('Invalid carryforward vintage')
                for key in ('opening_minor', 'addition_minor', 'used_minor', 'expired_minor', 'closing_minor'):
                    _integer(record[key], unsigned=True)
                if record['opening_minor'] + record['addition_minor'] - record['used_minor'] - record['expired_minor'] != record['closing_minor']:
                    _fail('Carryforward does not reconcile')
                if record.get('source_final_id') is not None:
                    _identifier(record.get('source_carry_id'))
                elif record['opening_minor'] and not record.get('evidence_ids'):
                    _fail('Imported opening carryforward needs retained prior assessment evidence')
    if not isinstance(state['exclusions'], dict) or len(state['exclusions']) > MAX_RECORDS:
        _fail('Invalid account exclusion reviews')
    for code, review in state['exclusions'].items():
        _fields(review, {'reason', 'evidence_ids'}, {'reason'})
        _text(review['reason'], 'Account exclusion reason')
        if not conn.execute('SELECT 1 FROM gl_accounts WHERE profile_id=? AND code=?', (profile_id, code)).fetchone():
            _fail('Excluded account is not in this book')
    _references(conn, profile_id, state)


def review_workpaper(conn, profile_id, *, workpaper_id, expected_revision, patch, reason, idempotency_key):
    workpaper = _workpaper(conn, profile_id, workpaper_id)
    pack = _pack(workpaper)
    reason = _text(reason, 'Revision reason')
    key = _text(idempotency_key, 'idempotency_key', 160)
    _fields(patch, {'facts', 'annex_instances', 'field_reviews', 'mappings', 'adjustments', 'carryforwards', 'exclusions'})
    _bounded_json(patch, MAX_PATCH_BYTES)
    request_digest = digest(dict(expected_revision=expected_revision, patch=patch, reason=reason))
    with atomic(conn):
        old = _row(conn, 'SELECT * FROM gl_tax_revisions WHERE profile_id=? AND workpaper_id=? AND idempotency_key=?', (profile_id, workpaper_id, key))
        if old:
            if old['payload_digest'] != request_digest:
                _fail('Idempotency key was used for different review inputs')
            return get_workpaper(conn, profile_id, workpaper_id=workpaper_id)
        revision = _latest_revision(conn, profile_id, workpaper_id)
        if type(expected_revision) is not int or expected_revision != revision['revision']:
            _fail('Tax working paper changed after review', 'accounting_stale_approval')
        state = json.loads(revision['state_json'])
        for section, value in patch.items():
            if section in ('facts', 'field_reviews', 'exclusions'):
                if not isinstance(value, dict):
                    _fail('Review patch requires an object')
                for identifier, record in value.items():
                    if record is None:
                        state[section].pop(identifier, None)
                    else:
                        state[section][identifier] = record
            else:
                state[section] = value
        _validate_state(conn, profile_id, pack, state)
        _insert_revision(conn, profile_id, workpaper_id, expected_revision + 1, state, reason, key, request_digest)
    return get_workpaper(conn, profile_id, workpaper_id=workpaper_id)


def get_workpaper(conn, profile_id, *, workpaper_id):
    workpaper = _workpaper(conn, profile_id, workpaper_id)
    revision = _latest_revision(conn, profile_id, workpaper_id)
    _pack(workpaper)
    state = json.loads(revision['state_json'])
    _references(conn, profile_id, state)
    finals = _rows(conn, 'SELECT id,revision_id,close_event_id,input_digest,report_digest,created_at FROM gl_tax_finals WHERE profile_id=? AND workpaper_id=? ORDER BY created_at,id', (profile_id, workpaper_id))
    return dict(**workpaper, revision=revision['revision'], revision_id=revision['id'], state=state,
                state_digest=revision['state_digest'], finals=finals)


def list_workpapers(conn, profile_id, *, limit=100, cursor=None):
    book = require_book(conn, profile_id)
    if type(limit) is not int or not 1 <= limit <= 500:
        _fail('Invalid tax page limit')
    counts = tuple(conn.execute('SELECT COUNT(*),COALESCE(MAX(created_at),\'\') FROM gl_tax_revisions WHERE profile_id=?', (profile_id,)).fetchone())
    binding = digest([profile_id, 'tax-list', book['revision'], counts])
    last = ''
    if cursor is not None:
        try:
            if not isinstance(cursor, str) or len(cursor) > 2048:
                raise ValueError()
            obj = json.loads(base64.b64decode(cursor, altchars=b'-_', validate=True))
            if set(obj) != {'profile_id', 'binding', 'last'} or obj['profile_id'] != profile_id:
                raise ValueError()
            if obj['binding'] != binding:
                _fail('Tax records changed; refresh', 'accounting_stale_cursor')
            last = _identifier(obj['last'])
        except (ValueError, TypeError, binascii.Error, RecursionError) as exc:
            raise AppError('Invalid tax continuation', code='accounting_invalid_cursor') from exc
    rows = _rows(conn, 'SELECT w.*, (SELECT MAX(r.revision) FROM gl_tax_revisions r WHERE r.profile_id=w.profile_id AND r.workpaper_id=w.id) AS revision FROM gl_tax_workpapers w WHERE w.profile_id=? AND w.id>? ORDER BY w.id LIMIT ?', (profile_id, last, limit + 1))
    selected = rows[:limit]
    next_cursor = base64.urlsafe_b64encode(canonical_json(dict(profile_id=profile_id, binding=binding, last=selected[-1]['id'])).encode()).decode() if len(rows) > limit else None
    return dict(workpapers=selected, next_cursor=next_cursor, binding=binding)


def _close(conn, profile_id, workpaper):
    period = _period(conn, profile_id, workpaper['period_id'])
    close = _row(conn, "SELECT * FROM gl_period_events WHERE profile_id=? AND period_id=? AND revision=? AND action='close'", (profile_id, period['id'], period['revision']))
    if period['state'] != 'closed' or not close:
        return None
    state = json.loads(close['snapshot_json'])
    if digest(state) != close['snapshot_digest']:
        _fail('Closed ledger integrity failed', 'accounting_tax_integrity')
    return dict(**close, snapshot=state)


def _assessment_inputs(conn, profile_id, year):
    """One assessment includes all fiscal periods ending in its calendar year.

    The full membership and close revisions are bound, so creating another
    period or reopening any member invalidates an earlier final report.
    """
    from .ledger import financial_statements, trial_balance
    periods = _rows(conn, 'SELECT * FROM gl_periods WHERE profile_id=? AND end_date>=? AND end_date<=? ORDER BY end_date,start_date,id',
                    (profile_id, f'{year}-01-01', f'{year}-12-31'))
    members, movement, totals, balance, profit = [], {}, {}, {}, 0
    for period in periods:
        close = _close(conn, profile_id, {'period_id': period['id']})
        statements = close['snapshot']['statements'] if close else financial_statements(conn, profile_id, period_id=period['id'])
        members.append(dict(period_id=period['id'], start_date=period['start_date'], end_date=period['end_date'],
                            period_state=period['state'], period_revision=period['revision'],
                            close_id=close['id'] if close else None, close_digest=close['snapshot_digest'] if close else None))
        profit += statements['profit_minor']
        for row in statements['profit_and_loss']:
            code = row['account_code']
            movement[code] = movement.get(code, 0) + row['balance_minor']
        for row in trial_balance(conn, profile_id, period_id=period['id'], exclude_closing=True)['rows']:
            code = row['account_code']
            totals[code] = totals.get(code, 0) + row['balance_minor']
        balance = {row['account_code']: row['balance_minor'] for row in statements['balance_sheet']}
    return dict(periods=members, movement=movement, movement_totals=totals, balance_totals=balance, profit_minor=profit)


def preview_workpaper(conn, profile_id, *, workpaper_id):
    workpaper = get_workpaper(conn, profile_id, workpaper_id=workpaper_id)
    pack = _pack(workpaper)
    state = workpaper['state']
    _validate_state(conn, profile_id, pack, state)
    close = _close(conn, profile_id, workpaper)
    period = _period(conn, profile_id, workpaper['period_id'])
    assessment = _assessment_inputs(conn, profile_id, pack['tax_year'])
    movement = assessment['movement']
    movement_totals = assessment['movement_totals']
    balance_totals = assessment['balance_totals']
    blockers = []

    def block(code, target, message):
        blockers.append(dict(code=code, target=target, message=message))

    for member in assessment['periods']:
        if not member['close_id']:
            block('ledger_not_closed', member['period_id'], 'Finalize every accounting close belonging to this assessment year')
    facts = state['facts']
    for definition in pack['facts']:
        review = facts.get(definition['id'])
        if not review or review['state'] != 'reviewed_input':
            block('fact_unreviewed', definition['id'], definition['label'])

    def fact(key):
        review = facts.get(key, {})
        return review.get('value') if review.get('state') == 'reviewed_input' else None

    if fact('liability') not in (None, 'unlimited') or fact('section7_3') is True:
        block('different_form_route', 'K2', 'Reviewed facts require another corporate form route; bookkeeping remains available')
    for key in ('all_sources_reviewed', 'custodian_offsets_reviewed', 'foreign_credits_reviewed', 'carryforwards_reviewed'):
        if fact(key) is False:
            block('review_incomplete', key, 'Complete the corresponding review before finalization')
    selected_forms = {i['form_id'] for i in state['annex_instances'] if i['form_id'] != 'K2'}
    required_forms = fact('required_annexes')
    if required_forms is not None and set(required_forms) != selected_forms:
        block('annex_coverage', 'required_annexes', 'Selected annexes differ from the reviewed required-annex inventory')
    if fact('group_parent') is True and 'K12a' in selected_forms:
        block('different_annex_route', 'K12a', 'A group parent uses K12a-G; do not apply the standalone annex')
    mapped = {}
    consumed = {}
    for item in state['mappings']:
        basis = movement_totals if item['basis'] == 'movement' else balance_totals
        available = basis.get(item['account_code'], 0)
        amount = item['amount_minor']
        key = item['basis'] + ':' + item['account_code']
        consumed[key] = consumed.get(key, 0) + amount
        if (amount > 0 and available <= 0) or (amount < 0 and available >= 0) or abs(consumed[key]) > abs(available):
            block('mapping_budget', item['id'], 'Mapping exceeds or reverses the source account balance')
        mapped[item['field_key']] = mapped.get(item['field_key'], 0) + amount * item['multiplier']
    for item in state['adjustments']:
        mapped[item['field_key']] = mapped.get(item['field_key'], 0) + item['amount_minor']
    for code, amount in movement.items():
        remaining = amount - consumed.get('movement:' + code, 0)
        if remaining and code not in state['exclusions']:
            block('unmapped_book_result', code, 'Allocate the remaining P&L balance or explicitly review its exclusion')
    outputs = {}
    for instance in state['annex_instances']:
        definitions = jurisdiction.fields_for(pack, instance['form_id'])
        fields = {}
        values = {}
        for identifier, definition in definitions.items():
            key = instance['id'] + '.' + identifier
            review = state['field_reviews'].get(key)
            fields[identifier] = dict(**definition, field_key=key, state='blocked', reason='Not yet reviewed')
            if review:
                fields[identifier].update(review)
                if review['state'] == 'blocked':
                    block('explicit_review_block', key, review['reason'])
                if review['state'] == 'reviewed_input':
                    values[identifier] = review.get('value_minor', review.get('value'))
                elif review['state'] == 'not_applicable' and definition['type'] == 'money':
                    values[identifier] = 0
            if key in mapped:
                if review and review['state'] == 'reviewed_input' and review.get('value_minor') != mapped[key]:
                    block('source_value_mismatch', key, 'Reviewed value differs from mapped ledger sources and adjustments')
                if review and review['state'] == 'not_applicable' and mapped[key] != 0:
                    block('not_applicable_has_value', key, 'A not-applicable field has a nonzero source amount')
                values[identifier] = mapped[key]
                fields[identifier].update(state='derived', value_minor=mapped[key], reason='Reviewed ledger allocation and adjustments', derivation={'operation': 'source_mapping', 'value_minor': mapped[key]})
        absent = {key for key, field in fields.items() if field['state'] == 'not_applicable'}
        for identifier, derivation in jurisdiction.derive_fields(instance['form_id'], values, not_applicable=absent).items():
            value = derivation['value_minor']
            previous = fields[identifier]
            if previous['state'] == 'reviewed_input' and previous.get('value_minor') != value:
                block('arithmetic_mismatch', previous['field_key'], 'Reviewed value conflicts with form arithmetic')
            if previous['state'] == 'not_applicable' and value != 0:
                block('not_applicable_has_value', previous['field_key'], 'Not-applicable review conflicts with a nonzero derived amount')
            if previous['field_key'] in mapped and mapped[previous['field_key']] != value:
                block('derived_source_mismatch', previous['field_key'], 'Mapped aggregate conflicts with its detailed form operands')
            fields[identifier].update(state='derived', value_minor=value, derivation=derivation, reason='Versioned form arithmetic')
        outputs[instance['id']] = dict(**instance, fields=fields)
    _annex_controls(outputs, fact, block, assessment['periods'])
    _carry_controls(conn, profile_id, workpaper, state['carryforwards'], outputs, block)
    for instance in outputs.values():
        for field in instance['fields'].values():
            if field['state'] == 'blocked':
                block('field_unreviewed', field['field_key'], field['label'])
            if 'value_minor' in field and abs(field['value_minor']) > jurisdiction.MAX_FORM_MINOR:
                block('form_overflow', field['field_key'], 'Derived amount exceeds the published form precision')
            if field.get('negative_only') and field.get('value_minor', 0) > 0:
                block('loss_sign', field['field_key'], 'Derived loss amount must be negative or zero')
    input_binding = dict(workpaper_id=workpaper_id, revision=workpaper['revision'], state_digest=workpaper['state_digest'],
                         pack_digest=pack['digest'], close_id=close['id'] if close else None,
                         close_digest=close['snapshot_digest'] if close else None,
                         period_state=period['state'], period_revision=period['revision'],
                         assessment_periods=assessment['periods'])
    report = dict(schema_version=1, purpose='reviewed_tax_working_paper', profile_id=profile_id,
                  pack_id=pack['pack_id'], tax_year=pack['tax_year'], currency=pack['currency'],
                  minor_unit_exponent=2, binding=input_binding, input_digest=digest(input_binding),
                  state=state, forms=list(outputs.values()), sources=pack['sources'], law_sources=pack['law_sources'],
                  source_resolutions=pack['source_resolutions'], book_profit_minor=assessment['profit_minor'],
                  ledger_sources=[dict(account_code=code, basis=basis_name, balance_minor=amount,
                                       allocated_minor=consumed.get(basis_name + ':' + code, 0),
                                       remaining_minor=amount - consumed.get(basis_name + ':' + code, 0))
                                  for basis_name, amounts in (('movement', movement_totals), ('balance', balance_totals))
                                  for code, amount in sorted(amounts.items())],
                  blockers=blockers, ready=not blockers, filed=False,
                  verification=dict(content_digests=True,
                                    named_form_arithmetic=not any(b['code'] in ('arithmetic_mismatch', 'derived_source_mismatch', 'annex_total_mismatch', 'annex_total_incomplete', 'form_overflow') for b in blockers),
                                    arithmetic_fields=[f['field_key'] for item in outputs.values() for f in item['fields'].values() if f.get('derivation', {}).get('operation') not in (None, 'source_mapping')],
                                    ledger_source_coverage=not any(b['code'] in ('unmapped_book_result', 'mapping_budget', 'source_value_mismatch') for b in blockers),
                                    tax_liability_certified=False, electronically_submitted=False))
    _bounded_json(report, 4 * MAX_STATE_BYTES)
    return report


def _annex_controls(outputs, fact, block, assessment_periods):
    def value(instance, key):
        field = instance['fields'].get(key, {})
        if field.get('state') == 'not_applicable':
            return 0
        return field.get('value_minor', field.get('value')) if field.get('state') in ('reviewed_input', 'derived') else None

    main = outputs['main']
    interest_periods = set()
    expected_periods = {(p['start_date'], p['end_date']) for p in assessment_periods}
    # Exact cross-annex aggregates are checked, not silently auto-filled when
    # an instance lacks a reviewed income-class/sphere classification.
    buckets = {key: [] for key in ('LF_A', 'GW_A', 'LF_B', 'GW_B', 'VV_A', 'VV_B', '599', '289', '290', '291', '168', '177', '170', '178')}
    for instance in outputs.values():
        kind = instance['form_id']
        if kind == 'K2a':
            income_class = value(instance, 'INCOME_CLASS')
            if income_class not in ('LF', 'GW'):
                block('annex_classification', instance['id'], 'Business annex requires reviewed LF or GW income classification')
            else:
                buckets[income_class + '_A'].append(value(instance, 'TAX_PROFIT'))
            for source, target in (('9267', '599'), ('9081', '289'), ('9088', '290'), ('9089', '291')):
                buckets[target].append(value(instance, source))
        elif kind == 'K2b':
            buckets['VV_A'].append(value(instance, 'TAX_PROFIT'))
        elif kind == 'K11':
            income_class = value(instance, 'INCOME_CLASS')
            if income_class not in ('LF', 'GW', 'VV'):
                block('annex_classification', instance['id'], 'Participation requires reviewed LF, GW or VV income classification')
            elif value(instance, 'BVM') is not True:
                buckets[income_class + '_B'].append(value(instance, 'RENT_RESULT' if income_class == 'VV' else 'BUSINESS_RESULT'))
        elif kind == 'K2kv':
            for source, target in (('297', '289'), ('298', '290'), ('299', '291')):
                buckets[target].append(value(instance, source))
        elif kind == 'K12':
            if not any(value(instance, flag) is True for flag in ('BET_10A7', 'BET_102', 'BEH_10A4', 'BST_10A6')):
                block('participation_type_missing', instance['id'], 'Review the participation type required by K12')
            amount = value(instance, 'BETR_K12')
            if amount is not None and amount <= 0 and any(value(instance, flag) is True for flag in ('BET_10A7', 'BET_102')):
                block('participation_income_positive', instance['id'] + '.BETR_K12', 'Method-switch participation income must be positive')
            credit = value(instance, 'ANRECH')
            burden = [value(instance, key) for key in ('KOESTVB', 'QUELLST', 'VORBEL')]
            if credit is not None and None not in burden and credit > sum(burden):
                block('participation_credit_source_cap', instance['id'] + '.ANRECH', 'Total credit cannot exceed evidenced foreign tax burdens')
            income = [value(instance, key) for key in ('BETR_K12', 'HINZUBET')]
            # The published K12 check still says 25%; the K2 parent check says
            # 23%. Never infer a corporate rate from that legacy annex check.
            if credit is not None and None not in income and credit > jurisdiction.round_ratio(max(0, sum(income)), 23):
                block('participation_credit_cap', instance['id'] + '.ANRECH', 'Credit exceeds the 2025 K2 parent-form 23% ceiling')
        elif kind == 'K12a':
            dates = (value(instance, 'WJA_12A'), value(instance, 'WJE_12A'))
            if dates not in expected_periods:
                block('interest_fiscal_period', instance['id'], 'K12a dates must identify a fiscal period included in this assessment')
            elif dates in interest_periods:
                block('duplicate_interest_period', instance['id'], 'Use one K12a per fiscal period, not duplicated interest allowances')
            interest_periods.add(dates)
            for source, target in (('NAB_ZINS', '168'), ('ABZ_ZIVT', '177'), ('EBIT_K12', '170'), ('VERR_EVT', '178')):
                buckets[target].append(value(instance, source))
            if value(instance, 'WEG_WJ5') not in (None, 0):
                block('ebitda_expiry_year', instance['id'], 'The official 2025 K12a excludes fifth-year expiration')
    for target, amounts in buckets.items():
        if amounts and None not in amounts:
            main_value = value(main, target)
            if main_value is not None and main_value != sum(amounts):
                block('annex_total_mismatch', 'main.' + target, 'Main-form value differs from applicable annex totals')
        elif amounts:
            block('annex_total_incomplete', 'main.' + target, 'Required annex total has unreviewed inputs')
    if interest_periods and interest_periods != expected_periods:
        block('interest_fiscal_coverage', 'K12a', 'Provide one K12a for each fiscal period ending in the assessment year')
    for base, credits in (('840', ('841',)), ('599', ('318', '319')), ('289', ('290', '291')), ('293', ('294', '295'))):
        income = value(main, base)
        amounts = [value(main, c) for c in credits]
        if income is not None and None not in amounts and sum(amounts) > jurisdiction.round_ratio(max(0, income), 23):
            block('foreign_credit_cap', 'main.' + base, 'Credit exceeds 23% of the corresponding income in the 2025 BMF check')
    if fact('entity_type') != 'foundation':
        for field in main['fields'].values():
            if field['group'] == '16 Privatstiftungen' and value(main, field['id']) not in (None, 0):
                block('foundation_scope', field['field_key'], 'Private-foundation fields require the reviewed foundation route')
    for left, right in (('CONSTITUTION_FILED', 'CONSTITUTION_ATTACHED'), ('PIPELINE_FLAT', 'PIPELINE_EXPERT'), ('FOUNDATION_REVOKED', 'FOUNDATION_OTHER')):
        if value(main, left) is True and value(main, right) is True:
            block('exclusive_election', 'main.' + left, 'Mutually exclusive form elections selected')
    if value(main, 'INTEREST_STANDALONE') is True and any(value(main, key) not in (None, 0) for key in ('168', '170')):
        block('interest_exemption_conflict', 'main.INTEREST_STANDALONE', 'Standalone exemption conflicts with current non-deductible interest/EBITDA carry')
    if value(main, 'INTEREST_EQUITY') is True and value(main, '168') not in (None, 0):
        block('interest_election_conflict', 'main.168', 'Equity-ratio exception conflicts with non-deductible interest')


def _carry_controls(conn, profile_id, workpaper, records, outputs, block):
    def form_amount(form, key):
        field = form['fields'].get(key, {})
        return 0 if field.get('state') == 'not_applicable' else field.get('value_minor')

    # Explicit register-to-form reconciliation, not inferred eligibility or a
    # claim that every kind of carryforward has the same statutory treatment.
    for kind, component, field in (('loss', 'opening_minor', '619'),
                                    ('section23_allowance', 'used_minor', '825'),
                                    ('foreign_tax_credit', 'used_minor', '850')):
        stated = form_amount(outputs['main'], field)
        expected = sum(record[component] for record in records if record['kind'] == kind)
        if stated is not None and stated != expected:
            block('carry_form_mismatch', 'main.' + field, 'Form value differs from the corresponding reviewed carryforward register')
    interest_forms = [form for form in outputs.values() if form['form_id'] == 'K12a']
    for kind, component, field in (('interest', 'used_minor', 'ABZ_ZIVT'), ('ebitda', 'used_minor', 'VERR_EVT')):
        amounts = [form_amount(form, field) for form in interest_forms]
        if amounts and None not in amounts and sum(amounts) != sum(record[component] for record in records if record['kind'] == kind):
            block('carry_form_mismatch', kind, 'Interest/EBITDA use differs from the reviewed carryforward register')
    seen_sources = set()
    for record in records:
        if record['kind'] == 'ebitda' and record['vintage_year'] < _pack(workpaper)['tax_year'] - 5 and record['closing_minor']:
            block('expired_carryforward', record['id'], 'Expired EBITDA vintage cannot remain available')
        if record.get('source_final_id'):
            identity = (record['source_final_id'], record['source_carry_id'])
            if identity in seen_sources:
                block('duplicate_carry_source', record['id'], 'A prior carryforward may be continued only once in this working paper')
            seen_sources.add(identity)
            source = _row(conn, 'SELECT f.*,w.period_id FROM gl_tax_finals f JOIN gl_tax_workpapers w ON w.id=f.workpaper_id AND w.profile_id=f.profile_id WHERE f.profile_id=? AND f.id=?', (profile_id, record['source_final_id']))
            if not source:
                block('carry_source_missing', record['id'], 'Prior carryforward source is unavailable')
                continue
            source_report = json.loads(source['report_json'])
            if digest(source_report) != source['report_digest']:
                _fail('Prior final integrity failed', 'accounting_tax_integrity')
            source_period = _period(conn, profile_id, source['period_id'])
            target_period = _period(conn, profile_id, workpaper['period_id'])
            prior = next((r for r in source_report['state']['carryforwards'] if r['id'] == record['source_carry_id']), None)
            if source_report['tax_year'] >= _pack(workpaper)['tax_year'] or source_period['end_date'] >= target_period['start_date'] or prior is None or prior['kind'] != record['kind'] or prior['vintage_year'] != record['vintage_year'] or prior['closing_minor'] != record['opening_minor']:
                block('carry_source_mismatch', record['id'], 'Prior carryforward identity/vintage/opening balance does not match')
            current_close = _close(conn, profile_id, {'period_id': source['period_id']})
            if not current_close or current_close['id'] != source['close_event_id']:
                block('carry_source_stale', record['id'], 'Prior accounting close changed; re-review its carryforward')
            if _assessment_inputs(conn, profile_id, source_report['tax_year'])['periods'] != source_report['binding'].get('assessment_periods'):
                block('carry_source_stale', record['id'], 'Prior assessment-year period membership or closes changed')
            if _latest_revision(conn, profile_id, source['workpaper_id'])['id'] != source['revision_id']:
                block('carry_source_stale', record['id'], 'Prior tax working paper has a newer review')
            # A vintage must follow the last finalized continuation, not fork
            # from an older final and consume the same opening balance twice.
            candidates = _rows(conn, '''SELECT f.report_json,w.period_id FROM gl_tax_finals f
                JOIN gl_tax_workpapers w ON w.id=f.workpaper_id AND w.profile_id=f.profile_id
                JOIN gl_tax_revisions r ON r.id=f.revision_id AND r.profile_id=f.profile_id
                WHERE f.profile_id=? AND w.id!=? AND w.id!=?
                AND r.revision=(SELECT MAX(rr.revision) FROM gl_tax_revisions rr WHERE rr.profile_id=r.profile_id AND rr.workpaper_id=r.workpaper_id)''',
                (profile_id, workpaper['id'], source['workpaper_id']))
            for candidate in candidates:
                candidate_period = _period(conn, profile_id, candidate['period_id'])
                if candidate_period['end_date'] < target_period['start_date']:
                    others = json.loads(candidate['report_json'])['state']['carryforwards']
                    if any((other.get('source_final_id'), other.get('source_carry_id')) == identity for other in others):
                        block('carry_source_already_continued', record['id'], 'Use the latest intervening final carryforward, not its already-consumed ancestor')


def finalize_workpaper(conn, profile_id, *, workpaper_id, expected_revision, expected_digest):
    _workpaper(conn, profile_id, workpaper_id)
    with atomic(conn):
        report = preview_workpaper(conn, profile_id, workpaper_id=workpaper_id)
        if type(expected_revision) is not int or expected_revision != report['binding']['revision'] or expected_digest != report['input_digest']:
            _fail('Working-paper inputs changed after final review', 'accounting_stale_approval')
        if not report['ready']:
            raise AppError('Tax working paper has unresolved review controls', code='accounting_tax_blocked', details={'blockers': report['blockers']})
        previous = _row(conn, 'SELECT id FROM gl_tax_finals WHERE profile_id=? AND workpaper_id=? AND input_digest=?', (profile_id, workpaper_id, expected_digest))
        if previous:
            return _final_receipt(conn, profile_id, previous['id'])
        revision = _latest_revision(conn, profile_id, workpaper_id)
        identifier = uuid4().hex
        report['finalized_at'] = now_iso()
        report['final_id'] = identifier
        conn.execute('INSERT INTO gl_tax_finals VALUES(?,?,?,?,?,?,?,?,?)',
                     (identifier, profile_id, workpaper_id, revision['id'], report['binding']['close_id'], expected_digest,
                      _bounded_json(report, 4 * MAX_STATE_BYTES), digest(report), now_iso()))
    return _final_receipt(conn, profile_id, identifier)


def _final_receipt(conn, profile_id, final_id):
    final = _row(conn, 'SELECT id,report_digest,input_digest,created_at FROM gl_tax_finals WHERE profile_id=? AND id=?', (profile_id, final_id))
    return dict(final_id=final['id'], report_digest=final['report_digest'], input_digest=final['input_digest'], created_at=final['created_at'])


def export_workpaper(conn, profile_id, *, final_id, confirm_plaintext=False):
    require_book(conn, profile_id)
    if confirm_plaintext is not True:
        _fail('Explicit plaintext export confirmation is required', 'accounting_plaintext_confirmation_required')
    final = _row(conn, 'SELECT * FROM gl_tax_finals WHERE profile_id=? AND id=?', (profile_id, final_id))
    if not final:
        _fail('Final working paper not found in this book', 'accounting_tax_not_found')
    report_json = final['report_json']
    report = json.loads(report_json)
    if (hashlib.sha256(report_json.encode('utf-8')).hexdigest() != final['report_digest']
            or digest(report) != final['report_digest'] or report.get('profile_id') != profile_id):
        _fail('Final working-paper integrity failed', 'accounting_tax_integrity')
    current = preview_workpaper(conn, profile_id, workpaper_id=final['workpaper_id'])
    stale = current['input_digest'] != final['input_digest'] or not current['ready']
    rows = []
    for form in report['forms']:
        rows.append(f'<h2>{html.escape(form["label"])} ({html.escape(form["form_id"])})</h2><table><thead><tr><th>Field</th><th>Meaning</th><th>Value</th><th>Review</th></tr></thead><tbody>')
        for field in form['fields'].values():
            value = field.get('value_minor', field.get('value', ''))
            if field['type'] == 'money' and type(value) is int:
                value = ('-' if value < 0 else '') + str(abs(value) // 100) + '.' + str(abs(value) % 100).zfill(2)
            elif field['type'] == 'percent' and type(value) is int:
                value = f'{value // 100}.{value % 100:02d}%'
            rows.append('<tr>' + ''.join(f'<td>{html.escape(str(v))}</td>' for v in (field['kennzahl'] or field['id'], field['label'], value, field['state'] + ': ' + field['reason'])) + '</tr>')
        rows.append('</tbody></table>')
    identity = (f'<p>Final report: {html.escape(final_id)}<br>Report SHA-256: '
                f'{html.escape(final["report_digest"])}<br>Verify this digest against the exact UTF-8 '
                'report_json string in the JSON export, not this HTML rendering or the display report.</p>')
    document = '<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src \'none\'; style-src \'unsafe-inline\'"><title>Reviewed tax working paper</title><style>body{font:14px system-ui;margin:32px}table{border-collapse:collapse;width:100%}td,th{border:1px solid #999;padding:6px;text-align:left}h2{break-before:page}</style></head><body><h1>K2 preparation2025</h1><p>Reviewed working paper — not signed, filed or certified. Amounts EUR. Retain alongside the closed ledger and evidence.</p>' + identity + ('<p>STALE: a newer review or accounting close exists.</p>' if stale else '') + ''.join(rows) + '</body></html>'
    return dict(final_id=final_id, stale=stale, report=report, report_json=report_json,
                report_digest=final['report_digest'],
                html=document, html_sha256=hashlib.sha256(document.encode('utf-8')).hexdigest(),
                verification_levels=report['verification'], verification_contract={
                    'version': 1,
                    'report_digest': 'SHA-256 of the exact UTF-8 bytes of report_json; do not parse or reserialize before hashing',
                    'report_json': 'Immutable retained final report, including exact integer amounts',
                    'report': 'Display representation; transport may encode amounts as decimal strings; not the report_digest input',
                    'html_sha256': 'SHA-256 of the exact UTF-8 bytes of html in this export; current rendering including staleness notice, not immutable final identity',
                    'stale': 'Current freshness assessment outside the immutable final report',
                    'assurance': 'Content integrity only; not a signature, tax certification or proof of filing',
                })


def execute(conn, profile_id, action, payload):
    require_book(conn, profile_id)
    contracts = {
        'tax-packs': (set(), set(), lambda **_: {'packs': jurisdiction.list_packs(), 'definition': jurisdiction.get_pack(jurisdiction.AT_PACK_ID)}),
        'tax-list': ({'limit', 'cursor'}, set(), lambda **p: list_workpapers(conn, profile_id, **p)),
        'tax-get': ({'workpaper_id'}, {'workpaper_id'}, lambda **p: get_workpaper(conn, profile_id, **p)),
        'tax-preview': ({'workpaper_id'}, {'workpaper_id'}, lambda **p: preview_workpaper(conn, profile_id, **p)),
        'tax-export': ({'final_id', 'confirm_plaintext'}, {'final_id', 'confirm_plaintext'}, lambda **p: export_workpaper(conn, profile_id, **p)),
        'tax-create': ({'period_id', 'pack_id', 'idempotency_key'}, {'period_id', 'pack_id', 'idempotency_key'}, lambda **p: create_workpaper(conn, profile_id, **p)),
        'tax-review': ({'workpaper_id', 'expected_revision', 'patch', 'reason', 'idempotency_key'}, {'workpaper_id', 'expected_revision', 'patch', 'reason', 'idempotency_key'}, lambda **p: review_workpaper(conn, profile_id, **p)),
        'tax-finalize': ({'workpaper_id', 'expected_revision', 'expected_digest'}, {'workpaper_id', 'expected_revision', 'expected_digest'}, lambda **p: finalize_workpaper(conn, profile_id, **p)),
    }
    if action not in contracts:
        _fail('Unknown tax preparation action')
    allowed, required, call = contracts[action]
    return call(**_fields(payload, allowed, required))
