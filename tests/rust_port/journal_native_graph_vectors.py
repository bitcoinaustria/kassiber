from copy import deepcopy
from dataclasses import replace
import json
from pathlib import Path
from unittest.mock import patch

from kassiber.core.custody_compute import compute_custody_projection
from kassiber.core.custody_snapshot import encode_owned_index
from kassiber.core.ownership import OwnedIndex, OwnedMatch
from tests.custody_snapshot_generator import snapshot
from tests.rust_port.journal_render_branches import graph, row, component, SCRIPTS, REASON
from tests.rust_port.journal_render_oracle import capture_projection
from tests.rust_port.journal_render_vectors import clean_record

PATH = Path(__file__).resolve().parents[2] / 'crates/journals/tests/data/render-native-graph.json'


def vectors():
    book = snapshot(0)
    base = compute_custody_projection(book)
    quantity = replace(base.quantity_state,
                       projection=replace(base.quantity_state.projection, observations=(),
                                          decisions=(), postings=()))
    projection = replace(base.finalized_tax_projection, rows=(), intra_pairs=(), quarantines=())
    base = replace(base, quantity_state=quantity, finalized_tax_projection=projection,
                   active_components=[], rows=[], owned_index=OwnedIndex())
    response = dict(schema_version=2, ok=True, entries=[], holdings=[], tax_summary=[],
                    carried_rows=[], intra_audit=[], quarantines=[])
    result = []

    def add(name, rows=None, *, index=None, components=(), reasons=None):
        rows = deepcopy(rows if rows is not None else [
            row('source', 'wallet-a', 'outbound', raw=graph()),
            row('target', 'wallet-b', 'inbound')])
        if index is None:
            index = OwnedIndex()
            for wallet in ('wallet-a', 'wallet-b', 'wallet-c'):
                index.add_script(SCRIPTS[wallet], OwnedMatch(
                    wallet, wallet, 'treasury', 'bitcoin', 'main', '', None, None,
                    'inventory', 'descriptor'))
        quarantines = [dict(transaction_id=identifier, workspace_id='ws-1',
                            profile_id='profile-1', reason=reason, detail_json='{}')
                       for identifier, reason in (reasons if reasons is not None else [(rows[0]['id'], REASON)])]
        custody = replace(base, rows=rows, owned_index=index, active_components=list(components),
                          finalized_tax_projection=replace(projection, quarantines=tuple(quarantines)))
        facts = encode_owned_index(index, [row('encoding', 'wallet-a', 'outbound', raw=graph(
            outputs=[dict(scriptpubkey=script, value_sats=1) for script in index.by_script]))])
        with patch('tests.rust_port.journal_render_oracle.encode_owned_index', return_value=facts):
            record = clean_record(capture_projection(book['profile'], custody, response_override=response))
        record['custody']['rows'] = deepcopy(rows)
        record['custody']['active_components'] = deepcopy(list(components))
        record['custody']['owned_index'] = facts
        record['custody']['finalized_tax_projection']['quarantines'] = deepcopy(quarantines)
        result.append(dict(name=name, group='native-graph', **record))

    add('graph-baseline')
    values = [('surrogate', '\ud800'), ('literal', '\\ud800'),
              ('nan', float('nan')), ('inf', float('inf')), ('negative-inf', -float('inf')),
              ('wide', 10**90), ('mapping', {'b': 2, 'a': 1}),
              ('list', ['\udfff', {'b': 2, 'a': 1}]), ('null', None)]
    for name, value in values:
        for field in ('wallet_label', 'wallet_kind', 'occurred_at'):
            for side in (0, 1):
                rows = [row('source', 'wallet-a', 'outbound', raw=graph()),
                        row('target', 'wallet-b', 'inbound')]
                rows[side][field] = value
                add(f'candidate-{side}-{field}-{name}', rows)
        rows = [row('source', 'wallet-a', 'outbound', raw=graph()),
                row('target', 'wallet-b', 'inbound')]
        rows[0]['raw_json'] = dict(graph(), ignored=value)
        add(f'ignored-raw-{name}', rows)
        rows = [row('source', 'wallet-a', 'outbound', scoped=False),
                row('target', 'wallet-b', 'inbound', scoped=False)]
        for r in rows:
            r['external_id'] = value
            r['asset'] = {'b': 2, 'a': 1}
        add(f'legacy-group-{name}', rows)
    for wallet in ('\ud800', '\\ud800'):
        rows = [row('source', 'wallet-a', 'outbound', raw=graph()),
                row('target', 'wallet-b', 'inbound')]
        rows[1]['wallet_id'] = wallet
        index = OwnedIndex()
        for script, owner in [(SCRIPTS['wallet-a'], 'wallet-a'), (SCRIPTS['wallet-b'], wallet)]:
            index.add_script(script, OwnedMatch(owner, owner, 'treasury', 'bitcoin', 'main', '', None, None,
                                               'inventory', 'descriptor'))
        add(f'wallet-{wallet!r}', rows, index=index)
    for script in ('\ud800', '\\ud800', '\udfffΣ', 'Σ\udfffA'):
        raw = graph()
        raw['vout'][0]['scriptpubkey'] = script
        rows = [row('source', 'wallet-a', 'outbound', raw=raw),
                row('target', 'wallet-b', 'inbound')]
        index = OwnedIndex()
        index.add_script(script, OwnedMatch('wallet-b', 'Hot', 'treasury', 'bitcoin', 'main', '', None, None,
                                           'inventory', 'descriptor'))
        add(f'owned-script-{script!r}', rows, index=index)
    for name, value in values:
        rows = [row('source', 'wallet-a', 'outbound', raw=graph()),
                row('target', 'wallet-b', 'inbound')]
        rows[0]['id'] = value
        add(f'source-id-{name}', rows)
        for key in ('n', 'value_sats', 'scriptpubkey', 'role', 'asset', 'asset_id'):
            raw = graph()
            raw['vout'][0][key] = value
            rows = [row('source', 'wallet-a', 'outbound', raw=raw),
                    row('target', 'wallet-b', 'inbound')]
            add(f'output-{key}-{name}', rows)
        rows = [row('source', 'wallet-a', 'outbound', raw=graph()),
                row('target', 'wallet-b', 'inbound')]
        for key in ('chain', 'network'):
            rows[0]['config_json'] = {key: value}
            add(f'config-{key}-{name}', rows)
    for name, value in [('mapping', {'b': 2, 'a': 1}), ('surrogate', '\ud800'), ('nan', float('nan'))]:
        rows = [row('source', 'wallet-a', 'outbound', raw=graph()),
                row('target', 'wallet-b', 'inbound')]
        rows[1]['amount'] = value
        add(f'inbound-refusal-{name}', rows)
    for name, value in values:
        comp = component()
        comp['ignored'] = value
        add(f'ignored-component-{name}', components=[comp])
    for name, value in values:
        add(f'ignored-nonpairable-index-{name}', reasons=[('source', 'missing_spot_price')],
            components=[dict(component(), ignored=value)])
    return result


if __name__ == '__main__':
    PATH.write_text(json.dumps(vectors(), ensure_ascii=True, indent=2, allow_nan=True) + '\n')
