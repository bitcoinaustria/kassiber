"""Rebuild immutable synthetic R4-11 build-state vectors (explicit --accept)."""
from dataclasses import replace
import hashlib
import json
from pathlib import Path
import sys
import uuid
from unittest.mock import patch

from kassiber.core.custody_compute import compute_custody_projection
from tests.custody_snapshot_generator import snapshot
from tests.rust_port.journal_render_oracle import capture, capture_projection
from tests.tax_ledger_generator import corpus

PATH = Path(__file__).resolve().parents[2] / 'crates/journals/tests/data/render.json'


def seeded_ids(name):
    counter = 0
    def next_id():
        nonlocal counter
        digest = hashlib.sha256(f'{name}:{counter}'.encode()).digest()[:16]
        counter += 1
        return uuid.UUID(bytes=digest, version=4)
    return next_id


def clean_record(record):
    return {k: v for k, v in record.items() if k not in
            ('python_build_seconds', 'python_render_seconds', 'activity')}


def vectors():
    cases = []
    base = compute_custody_projection(snapshot(0))

    def add(name, profile, custody, response=None, group='ledger'):
        with patch.object(uuid, 'uuid4', seeded_ids(name)):
            record = capture_projection(profile, custody, response_override=response)
        # A request-builder refusal has no render operands and belongs to R4-10.
        if record['response'] is None and 'error' in record['expected']:
            return
        cases.append({'name': name, 'group': group, **clean_record(record)})

    for case in corpus():
        c = replace(base, finalized_tax_projection=case.inputs.finalized_tax_projection,
                    wallet_refs_by_id=case.inputs.wallet_refs_by_id,
                    direct_payout_records=case.inputs.direct_payout_records)
        add(case.name, case.profile, c)

    from tests.custody_fanout_generator import fanout_snapshot
    from tests.custody_reconciliation_generator import snapshot as reconciliation
    from tests.custody_safety_generator import safety_snapshot
    for name, book in [('fanout-32', fanout_snapshot(32)), ('fanout-33', fanout_snapshot(33)),
                       ('fanout-34', fanout_snapshot(34)), ('reconciliation', reconciliation(0)),
                       ('residual-suspense', reconciliation(6)), ('safety-0', safety_snapshot(0)),
                       ('loan', snapshot(7)), ('channel', snapshot(6))]:
        with patch.object(uuid, 'uuid4', seeded_ids(name)):
            record = capture(book)
        cases.append({'name': name, 'group': 'joins', **clean_record(record)})

    # Unscoped provider identifiers are literal; only 64-hex txids case-fold.
    for name, external, other in [('literal-same', 'CSV-ID', 'CSV-ID'),
                                  ('literal-case', 'CSV-ID', 'csv-id'),
                                  ('literal-space', ' CSV-ID', 'CSV-ID'),
                                  ('missing-identity', None, None),
                                  ('hex-case', 'A' * 64, 'a' * 64)]:
        source = dict(base.rows[0], id='source', external_id=external,
                      direction='outbound', amount=10_000_000, raw_json='{}')
        target = dict(base.rows[1], id='target', external_id=other,
                      direction='inbound', amount=10_000_000, wallet_id='wallet-b',
                      wallet_label='Hot', raw_json='{}')
        quarantine = dict(transaction_id='source', workspace_id='ws-1',
                          profile_id='profile-1', reason='owned_fanout_unresolved', detail_json='{}')
        projection = replace(base.finalized_tax_projection, rows=(), intra_pairs=(), quarantines=(quarantine,))
        custody = replace(base, rows=[source, target], finalized_tax_projection=projection)
        add(name, snapshot(0)['profile'], custody, group='joins')

    empty = replace(base.finalized_tax_projection, rows=(), intra_pairs=(), quarantines=())
    for role in ('loan', 'lightning'):
        c = replace(base, finalized_tax_projection=empty,
                    loan_legs=[dict(transaction_id='buy', role='principal')] if role == 'loan' else [],
                    channel_roles={'buy': 'channel_open'} if role == 'lightning' else {},
                    active_components=[])
        add(f'{role}-difference-reason', snapshot(0)['profile'], c, group='joins')
    for seed in (1, 2):
        book = snapshot(seed)
        c = compute_custody_projection(book)
        for reason in ('owned_fanout_unresolved', 'ownership_transfer_source_ambiguous'):
            quarantines = tuple(dict(transaction_id=row['id'], workspace_id='ws-1',
                                     profile_id='profile-1', reason=reason, detail_json='{}')
                                for row in c.rows if row['direction'] == 'outbound')
            projection = replace(c.finalized_tax_projection, rows=(), intra_pairs=(), quarantines=quarantines)
            add(f'graph-proof-{seed}-{reason}', book['profile'],
                replace(c, finalized_tax_projection=projection, active_components=[]), group='joins')

    # The template, rather than the reason, chooses detail enrichment.
    for kind in ('pricing_review', 'privacy_hop', 'hold', 'plain', 'partner_leg'):
        row = dict(base.finalized_tax_projection.rows[0], privacy_boundary='payjoin',
                   raw_json='{"source":"coordinator","islikelycoinjoin":"yes"}')
        projection = replace(base.finalized_tax_projection, rows=(row,), intra_pairs=(), quarantines=())
        response = dict(schema_version=2, ok=True, entries=[], holdings=[], tax_summary=[],
                        carried_rows=[], intra_audit=[], quarantines=[dict(row=row['id'],
                            reason='missing_spot_price', detail_kind=kind,
                            detail=dict(value={'float': '123E-2'}, nested=[{'float': '-0E0'}]))])
        add(f'detail-template-{kind}', snapshot(0)['profile'],
            replace(base, finalized_tax_projection=projection), response, group='ledger')

    wallets = {key: dict(value) for key, value in base.wallet_refs_by_id.items()}
    wallets['wallet-b']['label'] = wallets['wallet-a']['label']
    response = dict(schema_version=2, ok=True, entries=[], quarantines=[], intra_audit=[],
                    carried_rows=[], tax_summary=[], holdings=[
                        dict(asset=asset, wallet=label, quantity=quantity, cost_basis='1E0')
                        for asset, label, quantity in [('BTC', 'Cold', '12345678901234567890123456789012E-32'),
                                                       ('LBTC', 'Cold', '1E0'), ('BTC', 'Unknown', '10E0'),
                                                       ('BTC', 'Hot', '1E-32'), ('BTC', 'Cold', '1E-32')]])
    add('holding-order-labels-and-rounding', snapshot(0)['profile'],
        replace(base, wallet_refs_by_id=wallets), response, group='ledger')

    normal_response = capture_projection(snapshot(0)['profile'], base)['response']
    for name, anchor in [('null', None), ('number', 123)]:
        rows = tuple(dict(row, journal_transaction_id=anchor)
                     for row in base.finalized_tax_projection.rows)
        projection = replace(base.finalized_tax_projection, rows=rows)
        add(f'anchor-stringification-{name}', snapshot(0)['profile'],
            replace(base, finalized_tax_projection=projection), normal_response)
    for method in (None, 123):
        add(f'method-stringification-{method}',
            dict(snapshot(0)['profile'], gains_algorithm=method), base, normal_response)

    # Independently injected engine results cover refusals before row rendering.
    for stage in ('prepare', 'validate', 'compute_multi', 'compute', 'classify', 'request', 'unsupported', 'unknown'):
        for error_class in ('ValueError', 'TypeError', 'RuntimeError', 'AppError', 'InvalidOperation', 'DivisionByZero', 'Overflow', 'BuiltinValueError', 'Unsupported'):
            response = {'schema_version': 2, 'ok': False,
                        'error': {'stage': stage, 'asset': 'BTC',
                                  'inner': {'class': error_class, 'message': "failure é 'quoted'"}}}
            add(f'refusal-{stage}-{error_class}', snapshot(0)['profile'], base, response, 'refusals')
    return cases


def main():
    result = json.dumps(vectors(), ensure_ascii=True, separators=(',', ':'), allow_nan=False) + '\n'
    if '--accept' in sys.argv:
        PATH.write_text(result)
    else:
        assert PATH.read_text() == result, 'render vectors changed; review then use --accept'
    print(f'{len(json.loads(result))} synthetic journal render vectors')


if __name__ == '__main__':
    main()
