"""Immutable calculation captures, kept separate from book carrying values.

Only the internal calculation Adapter may construct captures for retention.
Do not expose retain_calculation as a renderer/AI command: storing a well-formed
capture does not prove that its engine calculation or source interpretation is
correct. Replay is an explicitly separate verification level.
"""
from __future__ import annotations

from dataclasses import dataclass
from datetime import date, datetime, time, timedelta, timezone
from decimal import Decimal, InvalidOperation, ROUND_HALF_EVEN, localcontext
import json
import re
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

from ...errors import AppError
from ...time_utils import now_iso
from .ledger import atomic, canonical_json, digest, require_book, strict_minor, _row, _date
from .sources import get_snapshot, require_current

MAX_ARTIFACT_BYTES = 32 * 1024 * 1024
ADAPTER_VERSION = 'cutoff-prefix-v2'


def _dependency_revision():
    from importlib.metadata import PackageNotFoundError, distribution
    try:
        metadata=json.loads(distribution('rp2').read_text('direct_url.json') or '{}')
        revision=metadata['vcs_info']['commit_id']
        if not isinstance(revision,str) or not re.fullmatch('[0-9a-f]{40}',revision):
            raise ValueError()
        return revision
    except (PackageNotFoundError,KeyError,TypeError,ValueError) as exc:
        raise AppError('Installed RP2 revision cannot be proven',code='accounting_calculation_dependency') from exc


@dataclass(frozen=True)
class CapturedCalculation:
    profile_id: str
    source_snapshot_id: str
    source_digest: str
    dependency_revision: str
    adapter_version: str
    cutoff_exclusive_utc: str
    calculation_timezone: str
    policy: dict
    inputs: dict
    assets: list[dict]
    blockers: list[dict]
    decimal_precision: int = 32


def ensure_schema(conn):
    conn.execute('''CREATE TABLE IF NOT EXISTS gl_calculation_artifacts(
        id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,source_snapshot_id TEXT NOT NULL,
        payload_digest TEXT NOT NULL,payload_json TEXT NOT NULL,created_at TEXT NOT NULL,
        UNIQUE(profile_id,id),UNIQUE(profile_id,payload_digest),
        FOREIGN KEY(profile_id,source_snapshot_id) REFERENCES gl_source_snapshots(profile_id,id))''')
    for action in ('UPDATE', 'DELETE'):
        conn.execute(f'''CREATE TRIGGER IF NOT EXISTS gl_calculation_artifacts_no_{action.lower()}
            BEFORE {action} ON gl_calculation_artifacts BEGIN
            SELECT RAISE(ABORT,'accounting_calculation_retained'); END''')
    conn.execute('''CREATE TRIGGER IF NOT EXISTS gl_calculation_artifacts_no_replace
        BEFORE INSERT ON gl_calculation_artifacts WHEN EXISTS(SELECT 1 FROM gl_calculation_artifacts
            WHERE id=NEW.id OR (profile_id=NEW.profile_id AND payload_digest=NEW.payload_digest))
        BEGIN SELECT RAISE(ABORT,'accounting_calculation_retained'); END''')


def exact_decimal(value):
    if not isinstance(value, str) or len(value) > 160 or not re.fullmatch(r'-?(0|[1-9][0-9]*)(\.[0-9]+)?', value):
        raise AppError('Calculation values require finite exact decimal strings', code='accounting_calculation_invalid')
    try:
        result = Decimal(value)
    except InvalidOperation as exc:
        raise AppError('Invalid calculation decimal', code='accounting_calculation_invalid') from exc
    if not result.is_finite():
        raise AppError('Calculation decimal must be finite', code='accounting_calculation_invalid')
    return result


def _json_exact(value, *, depth=0):
    if depth > 30:
        raise AppError('Calculation capture is nested too deeply', code='accounting_calculation_invalid')
    if value is None or type(value) in (bool, int, str):
        return
    if isinstance(value, list):
        for item in value:
            _json_exact(item, depth=depth+1)
        return
    if isinstance(value, dict) and all(isinstance(key, str) for key in value):
        for key, item in value.items():
            if key.endswith('_msat'):
                strict_minor(abs(item)) if type(item) is int else strict_minor(item)
            if key.endswith('_exact') and item is not None:
                exact_decimal(item)
            _json_exact(item, depth=depth+1)
        return
    raise AppError('Calculation captures cannot contain floats or runtime objects', code='accounting_calculation_invalid')


def _validate(payload):
    _json_exact(payload)
    expected = {'schema_version','adapter_id','adapter_version','dependency_revision','profile_id',
                'source_snapshot_id','source_digest','currency','cutoff_exclusive_utc','calculation_timezone',
                'decimal_precision','rounding_mode','policy','inputs','assets','blockers'}
    if set(payload) != expected or type(payload['schema_version']) is not int or payload['schema_version'] != 1 or payload['adapter_id'] != 'rp2':
        raise AppError('Unsupported calculation capture schema', code='accounting_calculation_invalid')
    if not isinstance(payload['dependency_revision'], str) or not re.fullmatch('[0-9a-f]{40}', payload['dependency_revision']):
        raise AppError('A calculation must identify the exact dependency revision', code='accounting_calculation_invalid')
    if not isinstance(payload['adapter_version'], str) or not payload['adapter_version'] or len(payload['adapter_version']) > 80:
        raise AppError('A calculation must identify its Adapter version', code='accounting_calculation_invalid')
    try:
        cutoff = datetime.fromisoformat(payload['cutoff_exclusive_utc'].replace('Z', '+00:00'))
        if cutoff.tzinfo is None or cutoff.utcoffset() != timezone.utc.utcoffset(cutoff):
            raise ValueError()
        ZoneInfo(payload['calculation_timezone'])
    except (TypeError, AttributeError, ValueError, ZoneInfoNotFoundError) as exc:
        raise AppError('Calculation cutoff requires UTC and a valid calendar timezone', code='accounting_calculation_invalid') from exc
    precision = payload['decimal_precision']
    if type(precision) is not int or not 16 <= precision <= 128 or payload['rounding_mode'] != 'ROUND_HALF_EVEN':
        raise AppError('Unsupported calculation decimal context', code='accounting_calculation_invalid')
    if not isinstance(payload['policy'], dict) or not isinstance(payload['inputs'], dict) or not isinstance(payload['assets'], list) or not isinstance(payload['blockers'], list):
        raise AppError('Calculation capture is incomplete', code='accounting_calculation_invalid')
    required_inputs = {'finalized_projection','prepared_transactions','source_event_map','basis_overrides'}
    if not required_inputs <= payload['inputs'].keys() or payload['inputs'].keys() - required_inputs - {'cutoff_relations','execution_basis','custody_relations','same_asset_moves'}:
        raise AppError('Retain finalized and prepared calculation inputs plus source mapping and overrides', code='accounting_calculation_invalid')
    assets = set()
    for asset in payload['assets']:
        if not isinstance(asset, dict) or set(asset) != {'asset','acquisitions','gain_losses','open_positions','custody_balances','transfers'}:
            raise AppError('Calculation asset capture is incomplete', code='accounting_calculation_invalid')
        name = asset['asset']
        if not isinstance(name, str) or not name or name in assets:
            raise AppError('Calculation assets need unique identities', code='accounting_calculation_invalid')
        assets.add(name)
        for collection in ('acquisitions','gain_losses','open_positions','custody_balances','transfers'):
            if not isinstance(asset[collection], list):
                raise AppError('Invalid calculation result collection', code='accounting_calculation_invalid')
        for acquisition in asset['acquisitions']:
            required = {'event_id','source_ids','rp2_unique_id','quantity_msat','original_fiat_value_exact','effective_basis_exact'}
            if not isinstance(acquisition, dict) or set(acquisition) != required:
                raise AppError('Acquisition capture must distinguish original and effective values', code='accounting_calculation_invalid')
            strict_minor(acquisition['quantity_msat'])
            if not isinstance(acquisition['source_ids'], list) or not acquisition['source_ids'] or not all(isinstance(x, str) and x for x in acquisition['source_ids']):
                raise AppError('Acquisitions require original source identities', code='accounting_calculation_invalid')
            if exact_decimal(acquisition['original_fiat_value_exact']) < 0 or exact_decimal(acquisition['effective_basis_exact']) < 0:
                raise AppError('Acquisition values cannot be negative', code='accounting_calculation_invalid')
        result_ids = set()
        for result in asset['gain_losses']:
            required = {'row_id','event_id','lot_id','quantity_msat','basis_exact','proceeds_exact','gain_exact','unit_basis_override_exact','transaction_type'}
            if not isinstance(result, dict) or not required <= result.keys() or result.keys() - required - {'category'}:
                raise AppError('Disposal capture is missing exact calculation fields', code='accounting_calculation_invalid')
            if not isinstance(result['row_id'], str) or not result['row_id'] or result['row_id'] in result_ids:
                raise AppError('Calculation result identities must distinguish basis fragments', code='accounting_calculation_invalid')
            result_ids.add(result['row_id'])
            strict_minor(result['quantity_msat'])
            with localcontext() as ctx:
                ctx.prec, ctx.rounding = precision, ROUND_HALF_EVEN
                basis = exact_decimal(result['basis_exact'])
                proceeds = exact_decimal(result['proceeds_exact'])
                if exact_decimal(result['gain_exact']) != proceeds - basis:
                    raise AppError('Captured gain does not reconcile with proceeds and basis', code='accounting_calculation_invalid')
        for position in asset['open_positions']:
            if not isinstance(position, dict) or not {'lot_id','pool_id','quantity_msat','basis_exact'} <= position.keys():
                raise AppError('Open positions require exact quantity and basis', code='accounting_calculation_invalid')
            strict_minor(position['quantity_msat'])
            if exact_decimal(position['basis_exact']) < 0:
                raise AppError('Open position basis cannot be negative', code='accounting_calculation_invalid')
        for balance in asset['custody_balances']:
            if not isinstance(balance, dict) or set(balance) != {'wallet_id','quantity_msat'}:
                raise AppError('Custody balance capture requires wallet and exact quantity', code='accounting_calculation_invalid')
        for transfer in asset['transfers']:
            required = {'event_id','from_asset','to_asset','quantity_sent_msat','quantity_received_msat','fee_msat','basis_carried_exact'}
            if not isinstance(transfer, dict) or set(transfer) != required:
                raise AppError('Transfer capture must retain quantity and carried basis', code='accounting_calculation_invalid')
            for key in ('quantity_sent_msat','quantity_received_msat','fee_msat'):
                strict_minor(transfer[key])
            if exact_decimal(transfer['basis_carried_exact']) < 0:
                raise AppError('Carried basis cannot be negative', code='accounting_calculation_invalid')


def retain_calculation(conn, profile_id, *, capture):
    """Internal engine-only ingestion. Content checks are not calculation replay."""
    book = require_book(conn, profile_id)
    if type(capture) is not CapturedCalculation or capture.profile_id != profile_id:
        raise AppError('A calculation capture must belong to this book', code='accounting_scope_changed')
    payload = dict(schema_version=1, adapter_id='rp2', adapter_version=capture.adapter_version,
                   dependency_revision=capture.dependency_revision, profile_id=profile_id,
                   source_snapshot_id=capture.source_snapshot_id, source_digest=capture.source_digest,
                   currency=book['currency'], cutoff_exclusive_utc=capture.cutoff_exclusive_utc,
                   calculation_timezone=capture.calculation_timezone, decimal_precision=capture.decimal_precision,
                   rounding_mode='ROUND_HALF_EVEN', policy=capture.policy, inputs=capture.inputs,
                   assets=capture.assets, blockers=capture.blockers)
    _validate(payload)
    text = canonical_json(payload)
    if len(text.encode()) > MAX_ARTIFACT_BYTES:
        raise AppError('Calculation artifact exceeds the retention limit', code='accounting_calculation_limit')
    checksum = digest(payload)
    with atomic(conn):
        existing = _row(conn, 'SELECT id FROM gl_calculation_artifacts WHERE profile_id=? AND payload_digest=?', (profile_id, checksum))
        if existing:
            return get_calculation(conn, profile_id, existing['id'])
        snapshot = require_current(conn, profile_id, capture.source_snapshot_id)
        if snapshot['input_digest'] != capture.source_digest:
            raise AppError('Calculation input no longer matches its source snapshot', code='accounting_source_stale')
        if capture.policy != snapshot['snapshot']['calculation_policy']:
            raise AppError('Calculation policy differs from the retained source policy', code='accounting_calculation_policy')
        conn.execute('INSERT INTO gl_calculation_artifacts VALUES(?,?,?,?,?,?)',
                     (checksum, profile_id, capture.source_snapshot_id, checksum, text, now_iso()))
        conn.execute('UPDATE gl_books SET revision=revision+1 WHERE profile_id=?', (profile_id,))
        return get_calculation(conn, profile_id, checksum)


def get_calculation(conn, profile_id, artifact_id):
    require_book(conn, profile_id)
    row = _row(conn, 'SELECT * FROM gl_calculation_artifacts WHERE profile_id=? AND id=?', (profile_id, artifact_id))
    if not row:
        raise AppError('Calculation artifact was not found in this book', code='not_found')
    payload = json.loads(row.pop('payload_json'))
    if digest(payload) != row['payload_digest'] or payload.get('profile_id') != profile_id:
        raise AppError('Calculation artifact failed content verification', code='accounting_calculation_corrupt')
    _validate(payload)
    snapshot = get_snapshot(conn, profile_id, row['source_snapshot_id'])
    if snapshot['input_digest'] != payload['source_digest']:
        raise AppError('Calculation artifact source commitment is invalid', code='accounting_calculation_corrupt')
    return {**row, 'capture': payload, 'verification': dict(content_digest='verified',
             result_arithmetic='verified', calculation_replay='not_performed', source_completeness='not_proven')}


def require_calculation_current(conn, profile_id, artifact_id):
    artifact = get_calculation(conn, profile_id, artifact_id)
    require_current(conn, profile_id, artifact['source_snapshot_id'])
    if artifact['capture']['blockers']:
        raise AppError('Calculation has unresolved inputs', code='accounting_calculation_blocked')
    if artifact['capture']['dependency_revision'] != _dependency_revision() or artifact['capture']['adapter_version'] != ADAPTER_VERSION:
        raise AppError('Calculation implementation changed; retain and review a new run',code='accounting_calculation_stale')
    return artifact


def capture_calculation(conn, profile_id, *, snapshot_id, period_id, boundary='closing', as_of_date=None):
    """Run and retain the existing engine over a complete pre-cutoff prefix.

    This is the only command-facing construction path. The caller selects a
    retained current source snapshot and fiscal period, never result values.
    Book and tax carrying policy remain separate: this is a tax calculation,
    not authorization to adopt that basis in the general ledger.
    """
    from ..custody_journal import CustodyJournalBuilder
    from ..engines.base import TaxEngineLedgerInputs
    from ..engines.rp2 import GenericRP2TaxEngine

    with atomic(conn):
        book = require_book(conn, profile_id)
        snapshot = require_current(conn, profile_id, snapshot_id)
        period = _row(conn, 'SELECT * FROM gl_periods WHERE profile_id=? AND id=?', (profile_id, period_id))
        if not period:
            raise AppError('Calculation period was not found in this book', code='not_found')
        if boundary not in ('opening','closing'):
            raise AppError('Select opening or closing calculation boundary',code='accounting_calculation_time')
        if as_of_date is not None and (boundary!='closing' or not isinstance(as_of_date,str) or not period['start_date']<=as_of_date<=period['end_date']):
            raise AppError('Valuation cutoff must be a date in the selected fiscal period',code='accounting_calculation_time')
        if as_of_date is not None:
            _date(as_of_date)
        profile = _row(conn, 'SELECT * FROM profiles WHERE id=?', (profile_id,))
        if profile['fiat_currency'] != book['currency']:
            raise AppError('Tax and book currency differ; explicit conversion is required', code='accounting_calculation_policy')
        if profile.get('cost_basis_pool_scope', 'global') != 'global':
            raise AppError('Calculation capture requires the existing global inventory authority', code='accounting_calculation_policy')
        revision = _dependency_revision()
        custody = CustodyJournalBuilder(conn, profile).build_custody_projection()
        next_day = date.fromisoformat(period['start_date']) if boundary=='opening' else date.fromisoformat(as_of_date or period['end_date']) + timedelta(days=1)
        cutoff = datetime.combine(next_day, time.min, ZoneInfo(book['timezone'])).astimezone(timezone.utc)
        result = GenericRP2TaxEngine(profile).capture_calculation(TaxEngineLedgerInputs(
            custody.finalized_tax_projection, custody.wallet_refs_by_id),
            cutoff_exclusive_utc=cutoff.isoformat(), calculation_timezone=book['timezone'])
        for blocker in snapshot['snapshot']['blockers']:
            occurred=blocker.get('occurred_at')
            if not occurred or datetime.fromisoformat(occurred.replace('Z','+00:00'))<cutoff:
                result.blockers.append(blocker)
        source_lookup = {(item['facts'].get('anchor_transaction_id'), item['facts'].get('observation_hash')): item['source_id']
                         for item in snapshot['snapshot']['sources'] if item['kind'] == 'custody'}
        for event_id, mapping in result.inputs['source_event_map'].items():
            source_id = source_lookup.get((mapping.get('journal_transaction_id'), mapping.get('custody_quantity_hash')))
            if not source_id:
                result.blockers.append(dict(code='accounting_calculation_source_missing', event_id=event_id))
            else:
                mapping['source_id'] = source_id
        # Canonical principal slices and attributed fee quantities occupy
        # separate intervals in the stable source's exact outgoing budget.
        # This permits independent fee rows without claiming principal twice.
        source_rows = {item['source_id']: item for item in snapshot['snapshot']['sources']}
        fee_offsets = {identity: item['amount_atomic'] - item['facts'].get('fee_msat', 0)
                       for identity, item in source_rows.items() if item['kind'] == 'custody' and item['direction'] == 'outbound'}
        for projected in sorted(result.inputs['finalized_projection'], key=lambda item: item['id']):
            mapping = result.inputs['source_event_map'][projected['id']]
            if not mapping.get('source_id') or projected['id'].startswith('accounting-transit:'):
                continue
            slices = []
            start, end = mapping.get('custody_slice_start_msat'), mapping.get('custody_slice_end_msat')
            if type(start) is int and type(end) is int and end > start:
                slices.append(dict(start_atomic=start, end_atomic=end))
            fee = projected.get('fee', 0)
            if projected.get('direction') == 'outbound' and fee:
                offset = fee_offsets[mapping['source_id']]
                if offset + fee > source_rows[mapping['source_id']]['amount_atomic']:
                    result.blockers.append(dict(code='accounting_calculation_fee_coverage', event_id=projected['id']))
                else:
                    slices.append(dict(start_atomic=offset, end_atomic=offset+fee))
                    fee_offsets[mapping['source_id']] = offset + fee
            mapping['claim_slices'] = slices
        for asset in result.assets:
            for acquisition in asset['acquisitions']:
                mapping = result.inputs['source_event_map'].get(acquisition['rp2_unique_id'], {})
                acquisition['source_ids'] = [mapping['source_id']] if mapping.get('source_id') else acquisition['source_ids']
        return retain_calculation(conn, profile_id, capture=CapturedCalculation(
            profile_id, snapshot_id, snapshot['input_digest'], revision, ADAPTER_VERSION,
            result.cutoff_exclusive_utc, result.calculation_timezone, snapshot['snapshot']['calculation_policy'],
            result.inputs, result.assets, result.blockers))
