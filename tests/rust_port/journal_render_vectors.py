"""Rebuild immutable synthetic R4-11 build-state vectors (explicit --accept)."""
from dataclasses import replace
from copy import deepcopy
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
    realized = next(e for e in normal_response['entries'] if e.get('fragments'))
    fragment = realized['fragments'][0]
    for source in ('event', 'lot'):
        values = [(field, value, field) for field, value in
                  [('spot_price', '123E1'), ('fiat_fee', '-0E-2'), ('crypto_fee', '1E-8')]]
        for field in ('spot_price', 'fiat_fee', 'crypto_fee'):
            coefficient, exponent = fragment[source][field].split('E')
            shifted = coefficient if coefficient in ('0', '-0') else coefficient + '0'
            values.append((field, f'{shifted}E{int(exponent)-1}', f'{field}-representation'))
        for field, value, name in values:
            response = deepcopy(normal_response)
            entry = next(e for e in response['entries'] if e.get('fragments'))
            changed = deepcopy(fragment)
            changed[source][field] = value
            entry['fragments'] = [deepcopy(fragment), changed, deepcopy(fragment)]
            add(f'source-cache-{source}-{name}', snapshot(0)['profile'], base, response)
    response = deepcopy(normal_response)
    entry = next(e for e in response['entries'] if e.get('fragments'))
    other_asset = deepcopy(entry)
    other_asset['asset'] = 'LBTC'
    response['entries'] += [other_asset, deepcopy(entry)]
    add('source-cache-asset', snapshot(0)['profile'], base, response)
    for name, anchor in [('null', None), ('number', 123)]:
        rows = tuple(dict(row, journal_transaction_id=anchor)
                     for row in base.finalized_tax_projection.rows)
        projection = replace(base.finalized_tax_projection, rows=rows)
        add(f'anchor-stringification-{name}', snapshot(0)['profile'],
            replace(base, finalized_tax_projection=projection), normal_response)
    for method in (None, 123, '\x1c FIFO\x1f', '\u3000FIFO\u3000'):
        add(f'method-stringification-{method}',
            dict(snapshot(0)['profile'], gains_algorithm=method), base, normal_response)

    # UTC fast rendering shares calendar validation with the existing parser.
    for name, timestamp in [
        ('leap', '2024-02-29T23:59:59Z'),
        ('century', '2000-02-29T00:00:00Z'),
        ('invalid-leap', '2023-02-29T00:00:00Z'),
        ('invalid-century', '1900-02-29T00:00:00Z'),
        ('invalid-day', '2024-04-31T00:00:00Z'),
        ('invalid-year', '0000-01-01T00:00:00Z'),
        ('invalid-month', '2024-13-01T00:00:00Z'),
        ('hour', '2024-01-01T24:00:00Z'),
        ('minute', '2024-01-01T00:60:00Z'),
        ('second', '2024-01-01T00:00:60Z'),
        ('clock-text', '2024-01-01T00:!0:00Z'),
        ('fraction', '2024-02-29T12:34:56.123Z'),
        ('offset', '2024-02-29T12:34:56+01:30'),
        ('lowercase', '2024-02-29t12:34:56z'),
        ('missing-zone', '2024-02-29T12:34:56'),
        ('unicode', '２０２４-０２-２９T１２:３４:５６Z'),
        ('numeric', 123), ('null', None), ('list', []),
    ]:
        rows = tuple(dict(row, occurred_at=timestamp)
                     for row in base.finalized_tax_projection.rows)
        add(f'source-timestamp-{name}', snapshot(0)['profile'],
            replace(base, finalized_tax_projection=replace(base.finalized_tax_projection, rows=rows)),
            normal_response)

    empty_response = dict(schema_version=2, ok=True, entries=[], quarantines=[],
                          intra_audit=[], carried_rows=[], holdings=[], tax_summary=[])
    profile = snapshot(0)['profile']
    source = dict(base.finalized_tax_projection.rows[0], id='source-slice',
                  direction='outbound', journal_transaction_id='source', amount=123)
    projected = replace(base.finalized_tax_projection, rows=(source,),
                        intra_pairs=(), quarantines=())
    payout = dict(id='payout', out_transaction_id='source', payout_amount=10)
    for field in ('out_amount', 'out_amount_msat', 'payout_amount', 'swap_fee_msat'):
        for name, value in [('invalid', 'bad'), ('fraction-text', '1.5'),
                            ('exponent-text', '1e3'), ('list', [1]), ('dict', {'a': 1}),
                            ('unicode', '\u3000+１２_３\u3000'), ('leading-zero', '00123'),
                            ('fraction', -123.9), ('bool', True), ('null', None),
                            ('empty-list', []), ('empty-dict', {})]:
            add(f'payout-{field}-{name}', profile,
                replace(base, finalized_tax_projection=projected,
                        direct_payout_records=[dict(payout, **{field: value})]),
                empty_response, group='boundary')
    for name, rows, record in [
        ('missing-source', (), payout),
        ('ambiguous-source', (source, dict(source, id='other')), payout),
        ('asset-filter', (source,), dict(payout, out_asset='OTHER')),
        ('missing-id', (source,), dict(payout, id=None, payout_amount='bad')),
        ('missing-anchor', (source,), dict(payout, out_transaction_id='', payout_amount='bad')),
        ('refusal-order', (source,), dict(payout, out_amount='bad', payout_amount='later', swap_fee_msat='last')),
        ('asset-before-amount', (source,), dict(payout, out_asset='OTHER', out_amount='bad')),
        ('amount-filter', (source, dict(source, id='other', amount=124)), dict(payout, out_amount=123)),
        ('zero-fallback', (source,), dict(payout, out_amount=0, out_amount_msat=123)),
        ('negative-subunit', (dict(source, amount=0),), dict(payout, out_amount=-0.9)),
        ('negative-zero-text', (dict(source, amount=0),), dict(payout, out_amount='-0')),
    ]:
        add(f'payout-{name}', profile, replace(base, finalized_tax_projection=replace(
            projected, rows=rows), direct_payout_records=[record]), empty_response, group='boundary')

    for text in ('1E999', '-1E999', '1E-999', '-0E0', 'bad', '1.0', None, 1, True, []):
        response = dict(empty_response, quarantines=[dict(row=source['id'],
            reason='missing_spot_price', detail_kind='plain',
            detail={'nested': [dict(float=text)]})])
        add(f'detail-float-{text!r}', profile, replace(base, finalized_tax_projection=projected),
            response, group='boundary')
    response = dict(empty_response, tax_summary=[dict(year=2024, asset='BTC',
        transaction_type='sell', long_term=False, quantity='1E0',
        proceeds='1E999', cost_basis='-1E999', gain_loss='0E0')])
    add('summary-float-overflow', profile, base, response, group='boundary')

    for name, mutate in [
        ('missing-row', lambda response: response['entries'][0].update(row='unknown')),
        ('missing-wallet-and-row', lambda response: response['entries'][0].update(
            wallet='unknown', row='unknown')),
    ]:
        import copy
        response = copy.deepcopy(normal_response)
        mutate(response)
        add(name, profile, base, response, group='boundary')
    for key in ('wallet_account_id', 'account_code', 'account_label'):
        refs = {key: dict(value) for key, value in base.wallet_refs_by_id.items()}
        del refs['wallet-a'][key]
        response = dict(empty_response, holdings=[dict(asset='BTC', wallet='Cold',
            quantity='1E0', cost_basis='2E0')])
        add(f'holding-missing-{key}', profile, replace(base, wallet_refs_by_id=refs),
            response, group='boundary')
    for key in ('workspace_id',):
        malformed_profile = dict(profile)
        del malformed_profile[key]
        add(f'entry-profile-missing-{key}', malformed_profile, base,
            normal_response, group='boundary')
    for name, wallet_id in [('list', []), ('dict', {})]:
        refs = {key: dict(value) for key, value in base.wallet_refs_by_id.items()}
        refs['wallet-a']['id'] = wallet_id
        add(f'entry-wallet-id-{name}', profile, replace(base, wallet_refs_by_id=refs),
            normal_response, group='boundary')
    for name, first, second in [('bool-int', True, 1), ('int-float', 1, 1.0),
                                ('zero-float', 0, -0.0), ('list', [], []),
                                ('dict', {}, {})]:
        refs = {key: dict(value) for key, value in base.wallet_refs_by_id.items()}
        for wallet, value in [('wallet-a', first), ('wallet-b', second)]:
            refs[wallet].update(wallet_account_id=value, account_code='same', account_label='same')
        response = dict(empty_response, holdings=[dict(asset='BTC', wallet=wallet,
            quantity='1E0', cost_basis='2E0') for wallet in ('Cold', 'Hot')])
        add(f'holding-key-{name}', profile, replace(base, wallet_refs_by_id=refs),
            response, group='boundary')

    quarantine = dict(transaction_id='source', workspace_id='ws-1', profile_id='profile-1',
                      reason='owned_fanout_unresolved', detail_json='{}')
    for reason in ('owned_fanout_unresolved', 'missing_spot_price'):
        for text in ('bad', [1], {'a': 1}, '1_0', ' １２３ ', -1.2):
            rows = [dict(source, id='source', external_id='CSV-ID', wallet_id='source-wallet'),
                    dict(source, id='target', direction='inbound', external_id='CSV-ID',
                         wallet_id='target-wallet', amount=text)]
            c = replace(base, rows=rows, active_components=[], finalized_tax_projection=replace(
                projected, rows=(), quarantines=(dict(quarantine, reason=reason),)))
            add(f'ownership-inbound-{reason}-{text!r}', profile, c, empty_response, group='boundary')
    for text in ('bad', [1], {'a': 1}, ' １２３ '):
        add(f'balance-suspense-{text!r}', profile, replace(base,
            active_components=[dict(validation=dict(suspense_msat=text), legs=[])]),
            empty_response, group='boundary')

    # Independently injected engine results cover refusals before row rendering.
    for stage in ('prepare', 'validate', 'compute_multi', 'compute', 'classify', 'request', 'unsupported', 'unknown'):
        for error_class in ('ValueError', 'TypeError', 'RuntimeError', 'AppError', 'InvalidOperation', 'DivisionByZero', 'Overflow', 'BuiltinValueError', 'Unsupported'):
            response = {'schema_version': 2, 'ok': False,
                        'error': {'stage': stage, 'asset': 'BTC',
                                  'inner': {'class': error_class, 'message': "failure é 'quoted'"}}}
            add(f'refusal-{stage}-{error_class}', snapshot(0)['profile'], base, response, 'refusals')
    return cases


def machine_msat_vectors():
    from decimal import Decimal, localcontext
    from kassiber.msat import btc_to_msat
    coefficients = (1, 5, 49, 50, 51, 2**63 - 1, 2**63, 2**64 - 1, 2**64, 2**64 + 1, 5 * 10**19)
    with localcontext() as context:
        context.prec = 32
        return [(text, btc_to_msat(Decimal(text)))
                for coefficient in coefficients
                for exponent in (-31, -30, -20, -12, -11, -1, 0, 8)
                for text in (f'{coefficient}E{exponent}', f'-{coefficient}E{exponent}')]


def main():
    result = json.dumps(vectors(), ensure_ascii=True, separators=(',', ':'), allow_nan=False) + '\n'
    machine = json.dumps(machine_msat_vectors(), separators=(',', ':')) + '\n'
    machine_path = PATH.with_name('msat-machine.json')
    if '--accept' in sys.argv:
        PATH.write_text(result)
        machine_path.write_text(machine)
    else:
        assert PATH.read_text() == result, 'render vectors changed; review then use --accept'
        assert machine_path.read_text() == machine, 'msat machine vectors changed'
    print(f'{len(json.loads(result))} synthetic journal render vectors')


if __name__ == '__main__':
    main()
