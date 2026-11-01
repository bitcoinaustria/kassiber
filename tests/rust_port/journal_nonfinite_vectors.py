"""Capture non-finite operands at every response consumption boundary."""
from copy import deepcopy
from dataclasses import fields, is_dataclass, replace
import json
from pathlib import Path
from unittest.mock import patch
import uuid

from kassiber.core.custody_compute import compute_custody_projection
from kassiber.core.custody_snapshot import encode_owned_index
from kassiber.core.engines.ledger_response import FRAGMENT_PRICING_FIELDS
from tests.custody_snapshot import to_jsonable
from tests.custody_snapshot_generator import snapshot
from tests.rust_port.journal_render_oracle import capture_projection
from tests.rust_port.journal_render_vectors import clean_record, seeded_ids
from tests.rust_port.journal_native_vectors import capture as capture_ledger
from tests.tax_ledger_generator import corpus

PATH = Path(__file__).resolve().parents[2] / 'crates/journals/tests/data/render-nonfinite.json'
VALUES = (('nan', float('nan')), ('positive-infinity', float('inf')),
          ('negative-infinity', -float('inf')))


def operands(value):
    if isinstance(value, float):
        return value
    if is_dataclass(value):
        return {'$type': type(value).__name__,
                **{f.name: operands(getattr(value, f.name)) for f in fields(value)}}
    if isinstance(value, dict) and all(isinstance(key, str) for key in value):
        return {key: operands(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [operands(item) for item in value]
    return to_jsonable(value, redact_config=False)


def paths(value, prefix=()):
    yield prefix
    if isinstance(value, dict):
        for key, item in value.items():
            yield from paths(item, (*prefix, key))
    elif isinstance(value, list):
        for i, item in enumerate(value):
            yield from paths(item, (*prefix, i))


def changed(value, path, replacement):
    if not path:
        return replacement
    result = deepcopy(value)
    parent = result
    for key in path[:-1]:
        parent = parent[key]
    parent[path[-1]] = replacement
    return result


def patches(base, value, prefix=()):
    if isinstance(base, dict) and isinstance(value, dict) and list(base) == list(value):
        return [patch for key in base for patch in patches(base[key], value[key], (*prefix, key))]
    if isinstance(base, list) and isinstance(value, list) and len(base) == len(value):
        return [patch for i, item in enumerate(base) for patch in patches(item, value[i], (*prefix, i))]
    if not isinstance(value, (dict, list)) and type(base) is type(value) and base == value:
        return []
    return [[list(prefix), deepcopy(value)]]


def vectors():
    book = snapshot(0)
    base = compute_custody_projection(book)
    template = next(case for case in corpus() if case.name == 'generic-ledger-fifo')
    transfers = replace(base, finalized_tax_projection=template.inputs.finalized_tax_projection,
                        wallet_refs_by_id=template.inputs.wallet_refs_by_id,
                        direct_payout_records=template.inputs.direct_payout_records)
    result = []
    bases = []

    def add(name, profile, custody, response):
        with patch.object(uuid, 'uuid4', seeded_ids(name)):
            record = clean_record(capture_projection(profile, custody, response_override=response))
        facts = replace(custody, owned_index=encode_owned_index(custody.owned_index, custody.rows))
        record['custody'] = operands(facts)
        record['wallets'] = operands(list(custody.wallet_refs_by_id.values()))
        ledger = capture_ledger(record)
        inputs = {key: record[key] for key in ('profile', 'custody', 'wallets', 'response')}
        result.append(dict(name=name, base=len(bases) - 1, patches=patches(bases[-1], inputs),
                           entry_ids=record['entry_ids'], expected=record['expected'],
                           ledger=ledger))

    for label, profile, custody in [('ordinary', book['profile'], base),
                                    ('transfers', template.profile, transfers)]:
        normal = capture_projection(profile, custody)['response']
        facts = replace(custody, owned_index=encode_owned_index(custody.owned_index, custody.rows))
        bases.append(dict(profile=profile, custody=operands(facts),
                          wallets=operands(list(custody.wallet_refs_by_id.values())), response=normal))
        for path in paths(normal):
            for name, value in VALUES:
                add(f'{label}-response-{path!r}-{name}', profile, custody,
                    changed(normal, path, value))
        for stage in ('carried_rows', 'intra_audit', 'entries', 'holdings', 'quarantines', 'tax_summary'):
            for name, value in VALUES:
                response = deepcopy(normal)
                for item in response[stage]:
                    item['ignored_nonfinite'] = [value, {'nested': value}]
                response['ignored_nonfinite'] = value
                add(f'{label}-ignored-{stage}-{name}', profile, custody, response)
        for stage in ('request', 'unsupported', 'prepare', 'classify', 'validate',
                      'compute_multi', 'compute', 'unknown'):
            error_response = dict(normal, ok=False, error=dict(
                stage=stage, asset='BTC', inner=dict(class_='ValueError', message='nonfinite witness')))
            error_response['error']['inner']['class'] = error_response['error']['inner'].pop('class_')
            for path in (('error',), ('error', 'stage'), ('error', 'asset'),
                         ('error', 'inner'), ('error', 'inner', 'class'), ('error', 'inner', 'message')):
                for name, value in VALUES:
                    response = changed(error_response, path, value)
                    for key in normal.keys() - {'schema_version', 'ok'}:
                        response[key] = value
                    add(f'{label}-refusal-{stage}-{path!r}-{name}', profile, custody, response)
        for name, value in VALUES:
            response = deepcopy(normal)
            response['carried_rows'] = [dict(row=normal['entries'][0]['row'], value=value, rate=value)]
            add(f'{label}-carry-native-{name}', profile, custody, response)
            response = deepcopy(normal)
            response['holdings'].append(dict(wallet='unknown', asset=value,
                                             quantity=value, cost_basis=value))
            add(f'{label}-ignored-holding-decimals-{name}', profile, custody, response)
            response = deepcopy(normal)
            row_id = normal['entries'][0]['row']
            response['quarantines'].append(dict(row=row_id, detail_kind='plain', reason=value,
                                                detail={'raw': value, 'nested': [value, {'v': value}]}))
            add(f'{label}-recursive-detail-{name}', profile, custody, response)
            response['quarantines'][-1]['detail'] = {'tagged': {'float': value}}
            add(f'{label}-typed-detail-refusal-{name}', profile, custody, response)
            for field in ('workspace_id', 'id', 'gains_algorithm'):
                add(f'{label}-profile-{field}-{name}', dict(profile, **{field: value}), custody, normal)
            for field in ('rates', 'warnings', 'component_blockers', 'custody_transfers',
                          'gap_ignored_transaction_ids', 'gap_accounting_ignored_transaction_ids'):
                add(f'{label}-passthrough-{field}-{name}', profile,
                    replace(custody, **{field: [value, {'b': value, 'a': [value]}]}), normal)
            for field in dict.fromkeys(('fiat_rate', 'fiat_value', *FRAGMENT_PRICING_FIELDS,
                                       'journal_transaction_id', 'occurred_at', 'external_id')):
                rows = tuple(dict(row, **{field: value}) for row in custody.finalized_tax_projection.rows)
                add(f'{label}-projection-row-{field}-{name}', profile, replace(custody,
                    finalized_tax_projection=replace(custody.finalized_tax_projection, rows=rows)), normal)
            for field in ('id', 'label', 'wallet_account_id', 'account_code', 'account_label'):
                refs = {key: dict(wallet, **{field: value})
                        for key, wallet in custody.wallet_refs_by_id.items()}
                add(f'{label}-wallet-{field}-{name}', profile,
                    replace(custody, wallet_refs_by_id=refs), normal)
            for field in ('suspense_msat', 'ignored'):
                component = dict(validation={field: value}, legs=[], allocations=[])
                add(f'{label}-component-validation-{field}-{name}', profile,
                    replace(custody, active_components=[component]), normal)
            for field in ('transaction_id', 'role'):
                leg = dict(transaction_id='source', role='out')
                leg[field] = value
                add(f'{label}-loan-{field}-{name}', profile,
                    replace(custody, loan_legs=[leg]), normal)
            add(f'{label}-channel-role-{name}', profile,
                replace(custody, channel_roles={'source': value}), normal)
            for field in ('out_amount', 'out_amount_msat', 'payout_amount'):
                payout = dict(id='payout', out_transaction_id='source', **{field: value})
                add(f'{label}-payout-{field}-{name}', profile,
                    replace(custody, direct_payout_records=[payout]), normal)
                payout['id'] = None
                add(f'{label}-ignored-payout-{field}-{name}', profile,
                    replace(custody, direct_payout_records=[payout]), normal)
            for field in ('cross_asset_pairs', 'quarantines'):
                projection = custody.finalized_tax_projection
                extra = ([dict(transaction_id='nonfinite', reason='missing_spot_price',
                               detail_json=value)] if field == 'quarantines' else [[value, {'raw': value}]])
                add(f'{label}-projection-{field}-{name}', profile, replace(custody,
                    finalized_tax_projection=replace(projection, **{field: tuple(extra)})), normal)
            postings = custody.quantity_state.projection.postings
            for ignored in (False, True):
                posting = replace(postings[0], amount_msat=value,
                                  location_kind='ignored' if ignored else 'wallet')
                quantity = replace(custody.quantity_state, projection=replace(
                    custody.quantity_state.projection, postings=(posting,)))
                add(f'{label}-posting-amount-ignored-{ignored}-{name}', profile,
                    replace(custody, quantity_state=quantity), normal)
            for amounts in ((value, 17), (17, value), (value, -value),
                            (10**400, value), (value, 10**400), (value, None)):
                postings = tuple(replace(postings[0], amount_msat=amount, location_kind='wallet')
                                 for amount in amounts)
                quantity = replace(custody.quantity_state, projection=replace(
                    custody.quantity_state.projection, postings=postings))
                add(f'{label}-posting-sequence-{amounts!r}-{name}', profile,
                    replace(custody, quantity_state=quantity), normal)
    return dict(bases=bases, cases=result)


if __name__ == '__main__':
    data = vectors()
    PATH.write_text('{"bases":' + json.dumps(data['bases'], ensure_ascii=True, separators=(',', ':'))
                    + ',"cases":[\n' + ',\n'.join(json.dumps(case, ensure_ascii=True, separators=(',', ':'))
                                                  for case in data['cases']) + ']}\n')
