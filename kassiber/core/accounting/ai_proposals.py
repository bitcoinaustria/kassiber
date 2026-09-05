"""Typed AI suggestions become ordinary reviewed commands, never direct SQL postings."""
import json
from uuid import uuid4

from ...errors import AppError
from ...redaction import redact_secret_text
from . import document_text, ledger
from .commands import _minor_values, wire_values

MAX_RESULT_BYTES = 256 * 1024
OUTPUT_CONTRACT = '''For document_fields, document_sorting, or draft_entry return a
single JSON object (no markdown) with exactly schema_version:1, explanation:string,
entries:[], document_reviews:[]. Use empty arrays when facts are missing.
Each entry has period_id, entry_date (YYYY-MM-DD), description, lines:[{account_code,
debit_minor,credit_minor}], evidence_ids:[]. Only use selected report period IDs,
selected chart accounts and selected evidence IDs. All amounts are integer strings.
Each document review has extraction_id, fields:{}, spans:{}; use only selected
extractions, source page numbers and reviewed field names from the supported
document fields: document_type,document_number,issued_date,due_date,currency,
minor_unit_exponent,counterparty,net_minor,vat_minor,total_minor. Every supplied
field needs a span {page,start,end}, with zero-based Unicode-codepoint offsets
into that selected page. Dates/currency/amounts must remain unknown when missing.
document_type is invoice,receipt,credit_note,statement,other. Currency is ISO uppercase;
currency and minor_unit_exponent must accompany money. Suggest normal entries only.
You cannot approve or apply these proposals. The user reviews them separately.'''


def ensure_schema(conn):
    conn.execute('''CREATE TABLE IF NOT EXISTS gl_ai_proposal_applications(
        id TEXT PRIMARY KEY,profile_id TEXT NOT NULL REFERENCES gl_books(profile_id),
        approval_digest TEXT NOT NULL,provenance_json TEXT NOT NULL,proposal_json TEXT NOT NULL,
        applied_json TEXT NOT NULL,reason TEXT NOT NULL,created_at TEXT NOT NULL
        DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')),UNIQUE(profile_id,approval_digest))''')
    for operation in ('UPDATE','DELETE'):
        conn.execute(f'''CREATE TRIGGER IF NOT EXISTS gl_ai_proposal_no_{operation.lower()}
            BEFORE {operation} ON gl_ai_proposal_applications BEGIN SELECT RAISE(ABORT,'accounting_ai_proposal_retained'); END''')
    conn.execute('''CREATE TRIGGER IF NOT EXISTS gl_ai_proposal_no_replace BEFORE INSERT ON gl_ai_proposal_applications
        WHEN EXISTS(SELECT 1 FROM gl_ai_proposal_applications WHERE id=NEW.id
            OR (profile_id=NEW.profile_id AND approval_digest=NEW.approval_digest))
        BEGIN SELECT RAISE(ABORT,'accounting_ai_proposal_retained'); END''')


def decode_result(content):
    if not isinstance(content, str) or len(content.encode('utf-8')) > MAX_RESULT_BYTES:
        return None
    try:
        value = json.loads(content)
    except (ValueError, RecursionError):
        return None
    if not isinstance(value, dict) or set(value) != {'schema_version','explanation','entries','document_reviews'} or type(value['schema_version']) is not int or value['schema_version'] != 1:
        return None
    if not isinstance(value['explanation'], str) or len(value['explanation']) > 16000:
        return None
    if not isinstance(value['entries'], list) or len(value['entries']) > 20 or not isinstance(value['document_reviews'], list) or len(value['document_reviews']) > 10:
        return None
    return value if value['entries'] or value['document_reviews'] else None


def _plan(conn, profile_id, candidate, binding):
    ledger.require_book(conn, profile_id)
    if binding['profile_id'] != profile_id or ledger.require_book(conn, profile_id)['revision'] != binding['book_revision']:
        raise AppError('Accounting suggestion inputs changed', code='accounting_stale_approval')
    selection = binding['selection']
    allowed_extractions = {row['id']: row for row in selection.get('extractions', [])}
    selected = {identity: document_text.get(conn, profile_id, extraction_id=identity) for identity in allowed_extractions}
    allowed_evidence = {record['evidence_id'] for record in selected.values()}
    allowed_periods = set(selection.get('period_ids', []))
    entries, reviews = [], []
    for index, row in enumerate(candidate['entries']):
        if not selection.get('include_chart') or not isinstance(row, dict) or set(row) != {'period_id','entry_date','description','lines','evidence_ids'}:
            raise AppError('Draft suggestion requires selected chart and exact fields', code='accounting_ai_proposal_invalid')
        if not isinstance(row['period_id'], str) or row['period_id'] not in allowed_periods or not isinstance(row['evidence_ids'], list) or any(not isinstance(x, str) or x not in allowed_evidence for x in row['evidence_ids']):
            raise AppError('Draft suggestion exceeds selected evidence or periods', code='accounting_ai_proposal_invalid')
        payload = _minor_values({key: row[key] for key in ('period_id','entry_date','description','lines')})
        payload.update(entry_kind='normal', source_ref='AI proposal: reviewed retained source',
            idempotency_key='ai-proposal-' + ledger.digest([binding, index, row]))
        entries.append({'payload': payload, 'evidence_ids': sorted(set(row['evidence_ids']))})
    seen = set()
    for row in candidate['document_reviews']:
        if not isinstance(row, dict) or set(row) != {'extraction_id','fields','spans'} or not isinstance(row['extraction_id'], str):
            raise AppError('Invalid document suggestion', code='accounting_ai_proposal_invalid')
        identity = row['extraction_id']
        if identity not in selected or identity in seen:
            raise AppError('Document suggestion exceeds selected sources', code='accounting_ai_proposal_invalid')
        seen.add(identity)
        if not isinstance(row['fields'], dict) or not isinstance(row['spans'], dict) or set(row['fields']) != set(row['spans']):
            raise AppError('Each suggested field requires selected source provenance', code='accounting_ai_proposal_invalid')
        for span in row['spans'].values():
            if not isinstance(span, dict) or span.get('page') not in allowed_extractions[identity]['pages']:
                raise AppError('Suggestion span was not disclosed', code='accounting_ai_proposal_invalid')
            original_page = selected[identity]['pages'][span['page'] - 1]
            if redact_secret_text(original_page) != original_page:
                # Disclosed offsets refer to redacted text. Without a proven
                # offset map they cannot be retained as original-source spans.
                raise AppError('Redacted source pages require manual field review', code='accounting_ai_source_redacted')
        record = selected[identity]
        reviews.append(dict(extraction_id=identity, expected_digest=record['content_digest'],
            previous_id=(record['review'] or {}).get('id'), previous_fields=(record['review'] or {}).get('fields', {}),
            fields=_minor_values(row['fields']), spans=row['spans']))
    return dict(entries=entries, document_reviews=reviews, explanation=candidate['explanation'])


def _apply_commands(conn, profile_id, plan, reason):
    entries = [ledger.create_draft(conn, profile_id, row['payload']) for row in plan['entries']]
    reviews = [document_text.review_fields(conn, profile_id, **{key: value for key, value in row.items() if key != 'previous_fields'}, reason=reason) for row in plan['document_reviews']]
    return dict(draft_ids=[row['id'] for row in entries], document_review_ids=[row['review']['id'] for row in reviews])


def preview(conn, profile_id, *, candidate, binding):
    plan = _plan(conn, profile_id, candidate, binding)
    # Exercise the SAME command guards used on apply; rollback even successful
    # previews. No duplicate validator, rows or revision changes survive.
    conn.execute('SAVEPOINT accounting_ai_proposal_preview')
    try:
        _apply_commands(conn, profile_id, plan, 'Preview only')
    finally:
        conn.execute('ROLLBACK TO accounting_ai_proposal_preview')
        conn.execute('RELEASE accounting_ai_proposal_preview')
    checksum = ledger.digest({'binding': binding, 'plan': plan})
    book = ledger.require_book(conn, profile_id)
    return wire_values(dict(plan=plan, expected_digest=checksum, book_revision=binding['book_revision'],
        currency=book['currency'], minor_unit_exponent=book['minor_unit_exponent'],
        provider=binding['provider'], effect='create_drafts_and_review_document_fields_not_post'))


def apply(conn, profile_id, *, candidate, binding, expected_digest, reason):
    reason = ledger._text(reason, 'reason', maximum=2000)
    with ledger.atomic(conn):
        reviewed = preview(conn, profile_id, candidate=candidate, binding=binding)
        if reviewed['expected_digest'] != expected_digest:
            raise AppError('AI suggestion approval changed', code='accounting_stale_approval')
        plan = _plan(conn, profile_id, candidate, binding)
        applied = _apply_commands(conn, profile_id, plan, reason)
        identifier = uuid4().hex
        conn.execute('''INSERT INTO gl_ai_proposal_applications
            (id,profile_id,approval_digest,provenance_json,proposal_json,applied_json,reason)
            VALUES(?,?,?,?,?,?,?)''', (identifier, profile_id, expected_digest,
            ledger.canonical_json(binding), ledger.canonical_json(plan), ledger.canonical_json(applied), reason))
        ledger._bump(conn, profile_id)
        return {'application_id': identifier, **applied, 'posted': False}
