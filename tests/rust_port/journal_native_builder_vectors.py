from copy import deepcopy
from dataclasses import replace
import json
from pathlib import Path
from unittest.mock import patch
import uuid

from kassiber.core.custody_compute import compute_custody_projection
from tests.custody_snapshot_generator import snapshot
from tests.rust_port.journal_render_oracle import capture_projection
from tests.rust_port.journal_render_vectors import clean_record, seeded_ids

PATH = Path(__file__).resolve().parents[2] / 'crates/journals/tests/data/render-native-builder.json'


def vectors():
    book = snapshot(0)
    custody = compute_custody_projection(book)
    response = capture_projection(book['profile'], custody)['response']
    result = []

    def add(name, profile=None, c=None, r=None):
        with patch.object(uuid, 'uuid4', seeded_ids(name)):
            record = capture_projection(book['profile'] if profile is None else profile,
                                        custody if c is None else c,
                                        response_override=response if r is None else r)
        result.append(dict(name=name, group='native-builder', **clean_record(record)))

    for name, value in [('surrogate', '\ud800'), ('literal', '\\ud800'),
                        ('nan', float('nan')), ('positive-infinity', float('inf')),
                        ('negative-infinity', -float('inf'))]:
        add(f'workspace-{name}', profile=dict(book['profile'], workspace_id=value))
        add(f'rate-{name}', c=replace(custody, rates={'BTC': value}))
        result[-1]['custody']['rates'] = {'BTC': value}
        rows = tuple(dict(row, pricing_provider=value) for row in custody.finalized_tax_projection.rows)
        add(f'provenance-{name}', c=replace(custody, finalized_tax_projection=replace(
            custody.finalized_tax_projection, rows=rows)))
        for row in result[-1]['custody']['finalized_tax_projection']['rows']:
            row['pricing_provider'] = value
        r = deepcopy(response)
        r['entries'][0]['description'] = value
        add(f'description-{name}', r=r)
    for name, value in [('list', ['opaque', {'b': 2, 'a': 1}]),
                        ('mapping', {'b': 2, 'a': 1}), ('null', None)]:
        r = deepcopy(response)
        r['entries'][0]['description'] = value
        add(f'opaque-description-{name}', r=r)
    add('method-mapping-order', profile=dict(book['profile'], gains_algorithm={'b': 2, 'a': 1}))
    rows = tuple(dict(row, journal_transaction_id={'b': 2, 'a': 1}) for row in custody.finalized_tax_projection.rows)
    add('anchor-mapping-order', c=replace(custody, finalized_tax_projection=replace(
        custody.finalized_tax_projection, rows=rows)))
    for name, value in [('malformed', '00E-1'), ('surrogate', '\udfff'), ('nan', float('nan'))]:
        r = deepcopy(response)
        r['holdings'].append(dict(wallet='unknown', asset='BTC', quantity=value, cost_basis=value))
        add(f'ignored-holding-{name}', r=r)
    refs = {key: dict(wallet, id='\ud800' if i == 0 else wallet['id'])
            for i, (key, wallet) in enumerate(custody.wallet_refs_by_id.items())}
    add('balance-surrogate-wallet', c=replace(custody, wallet_refs_by_id=refs))
    add('nested-rate-native-values', c=replace(custody, rates={'BTC': {'b': '\udfff', 'a': float('nan')}}))
    result[-1]['custody']['rates'] = {'BTC': {'b': '\udfff', 'a': float('nan')}}
    return result


if __name__ == '__main__':
    PATH.write_text(json.dumps(vectors(), ensure_ascii=True, indent=2, allow_nan=True) + '\n')
