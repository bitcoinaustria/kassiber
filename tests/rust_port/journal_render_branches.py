from copy import deepcopy
from dataclasses import replace
import json
from pathlib import Path
import sys
from unittest.mock import patch

from kassiber.core.custody_compute import compute_custody_projection
from kassiber.core.custody_snapshot import encode_owned_index
from kassiber.core.ownership import OwnedIndex, OwnedMatch
from tests.custody_snapshot_generator import snapshot
from tests.rust_port.journal_render_oracle import capture_projection
from tests.rust_port.journal_render_vectors import clean_record

PATH = Path(__file__).resolve().parents[2] / 'crates/journals/tests/data/render-branches.json'
TXID = 'a' * 64
SCRIPTS = {wallet: '0014' + byte * 40 for wallet, byte in
           [('wallet-a', '1'), ('wallet-b', '2'), ('wallet-c', '3')]}
REASON = 'owned_fanout_unresolved'


def graph(amount=100_000_000, outputs=None, **fields):
    return dict(chain='bitcoin', network='main', txid=TXID,
                vin=[dict(txid='b' * 64, vout=0,
                          prevout=dict(scriptpubkey=SCRIPTS['wallet-a'],
                                       value_sats=amount // 1000))],
                vout=outputs if outputs is not None else [
                    dict(n=0, scriptpubkey=SCRIPTS['wallet-b'],
                         value_sats=amount // 1000)], **fields)


def row(identifier, wallet, direction, amount=100_000_000, *, raw=None, scoped=True, **fields):
    return dict(id=identifier, workspace_id='ws-1', profile_id='profile-1',
                wallet_id=wallet, wallet_label={'wallet-a': 'Cold', 'wallet-b': 'Hot',
                                               'wallet-c': 'Savings'}[wallet],
                wallet_kind='descriptor' if scoped else 'custom',
                direction=direction, amount=amount, asset='BTC',
                occurred_at='2026-01-01T00:00:00Z',
                external_id=TXID if scoped else 'provider-id',
                external_id_kind='txid' if scoped else None,
                config_json='{"chain":"bitcoin","network":"main"}' if scoped else '{}',
                raw_json=json.dumps(raw if raw is not None else {}), **fields)


def component(out='source', inbound='target', *, kind='manual', policy='carrying-value',
              anchor=False):
    key = 'anchor_transaction_id' if anchor else 'transaction_id'
    return dict(component_type=kind, conversion_policy=policy,
                legs=[dict(id='out-leg', role='source', **{key: out}),
                      dict(id='in-leg', role='destination', **{key: inbound})],
                allocations=[dict(source_leg_id='out-leg', sink_leg_id='in-leg')])


def vectors():
    book = snapshot(0)
    base = compute_custody_projection(book)
    quantity = replace(base.quantity_state,
                       canonical_input=replace(base.quantity_state.canonical_input, events=()),
                       projection=replace(base.quantity_state.projection, observations=(),
                                          decisions=(), postings=()),
                       tax_eligibility=replace(base.quantity_state.tax_eligibility,
                                               eligible_decisions=()))
    projection = replace(base.finalized_tax_projection, rows=(), intra_pairs=(), quarantines=())
    base = replace(base, quantity_state=quantity, finalized_tax_projection=projection,
                   active_components=[], rows=[], owned_index=OwnedIndex())
    cases = []

    def add(name, *, group='exclusions', rows=None, components=(), reasons=None, index=None,
            expected_candidates=None, expected_error=None, **changes):
        rows = deepcopy(rows if rows is not None else [
            row('source', 'wallet-a', 'outbound', raw=graph()),
            row('target', 'wallet-b', 'inbound')])
        index = deepcopy(index if index is not None else OwnedIndex())
        if not index.by_script:
            for wallet in ('wallet-a', 'wallet-b', 'wallet-c'):
                index.add_script(SCRIPTS[wallet], OwnedMatch(
                    wallet, wallet, 'treasury', 'bitcoin', 'main', '', None, None,
                    'inventory', 'descriptor'))
        quarantines = [dict(transaction_id=identifier, workspace_id='ws-1',
                            profile_id='profile-1', reason=reason, detail_json='{}')
                       for identifier, reason in (reasons if reasons is not None else [('source', REASON)])]
        custody = replace(base, rows=rows, owned_index=index, active_components=list(components),
                          finalized_tax_projection=replace(projection, rows=tuple(
                              dict(item, custody_finalized_tax_projection=True) for item in rows),
                                                           quarantines=tuple(quarantines)), **changes)
        response = dict(schema_version=2, ok=True, entries=[], holdings=[], tax_summary=[],
                        carried_rows=[], intra_audit=[], quarantines=[])
        facts = encode_owned_index(index, [row('encoding', 'wallet-a', 'outbound', raw=graph(
            outputs=[dict(scriptpubkey=script, value_sats=1) for script in SCRIPTS.values()]))])
        with patch('tests.rust_port.journal_render_oracle.encode_owned_index', return_value=facts):
            record = clean_record(capture_projection(book['profile'], custody, response_override=response))
        assert 'custody' in record, (name, record)
        if expected_error is not None:
            assert record['expected'].get('error', {}).get('family') == expected_error, (name, record['expected'])
        if expected_candidates is not None:
            state = json.loads(record['expected']['state'])
            assert len(state['ownership_review_candidates']) == expected_candidates, (name, state)
        cases.append(dict(name=name, group=group, **record))

    add('graph-exact', expected_candidates=1)
    add('no-quarantines', reasons=[], expected_candidates=0)
    add('missing-source', reasons=[('absent', REASON)], expected_candidates=0)
    for reason in ('ownership_transfer_destination_ambiguous', 'ownership_transfer_source_ambiguous'):
        add(reason, reasons=[('source', reason)], expected_candidates=1)
    add('nonpairable-reason', reasons=[('source', 'missing_spot_price')], expected_candidates=0)
    for reason in ('custody_basis_barrier', 'transfer_pair_dependency_blocked',
                   'basis_provenance_incomplete', 'derived_transfer_group_blocked',
                   'at_swap_basis_carry_unresolved', 'bitcoin_rail_carry_basis_unresolved'):
        add(f'root-replaces-{reason}', reasons=[('source', reason), ('source', REASON)],
            expected_candidates=1)
    add('first-root-wins', reasons=[('source', 'missing_spot_price'), ('source', REASON)],
        expected_candidates=0)
    add('root-survives-downstream', reasons=[('source', REASON), ('source', 'custody_basis_barrier')],
        expected_candidates=1)
    for kind in ('manual', 'coinjoin', 'whirlpool', None, ''):
        add(f'reusable-kind-{kind}', components=[component(inbound='other', kind=kind)],
            rows=[row('source', 'wallet-a', 'outbound', raw=graph()),
                  row('target', 'wallet-b', 'inbound'), row('other', 'wallet-c', 'inbound')],
            expected_candidates=1)
    for name, comp in [
        ('already-active-pair', component()),
        ('active-anchor-pair', component(anchor=True)),
        ('direct-payout-claim', component(inbound=None)),
        ('empty-payout-claim', component(inbound='')),
        ('missing-source-row', component(out='absent')),
        ('missing-inbound-row', component(inbound='absent')),
        ('special-kind', component(inbound='other', kind='swap')),
        ('conversion-policy', component(inbound='other', policy='fair-market-value')),
    ]:
        add(name, components=[comp], rows=[row('source', 'wallet-a', 'outbound', raw=graph()),
            row('target', 'wallet-b', 'inbound'), row('other', 'wallet-c', 'inbound')],
            expected_candidates=0)
    add('cross-asset-component', components=[component(inbound='other')],
        rows=[row('source', 'wallet-a', 'outbound', raw=graph()),
              row('target', 'wallet-b', 'inbound'),
              dict(row('other', 'wallet-c', 'inbound'), asset='LBTC')], expected_candidates=0)
    add('unavailable-inbound', components=[component(out='other', inbound='target', kind='swap')],
        rows=[row('source', 'wallet-a', 'outbound', raw=graph()),
              row('target', 'wallet-b', 'inbound'), row('other', 'wallet-c', 'outbound')],
        expected_candidates=0)
    add('allocation-without-source', components=[component(out=None)], expected_candidates=1)
    add('disjoint-anchor-allocation', components=[component(inbound='other', anchor=True)],
        rows=[row('source', 'wallet-a', 'outbound', raw=graph()),
              row('target', 'wallet-b', 'inbound'), row('other', 'wallet-c', 'inbound')],
        expected_candidates=1)
    for name, direction, amount in [('source-inbound', 'inbound', 100_000_000),
                                    ('source-zero', 'outbound', 0),
                                    ('source-negative', 'outbound', -1)]:
        add(name, rows=[row('source', 'wallet-a', direction, amount, raw=graph()),
                        row('target', 'wallet-b', 'inbound')], expected_candidates=0)
    def add_graph(name, **kwargs):
        add(name, group='graph', **kwargs)

    for name, outputs in [
        ('outputs-aggregate', [dict(scriptpubkey=SCRIPTS['wallet-b'], value_sats=40_000),
                               dict(scriptpubkey=SCRIPTS['wallet-b'], value_sats=60_000)]),
        ('self-change', [dict(scriptpubkey=SCRIPTS['wallet-a'], value_sats=30_000),
                         dict(scriptpubkey=SCRIPTS['wallet-b'], value_sats=100_000)]),
        ('unowned-output', [dict(scriptpubkey='unowned', value_sats=30_000),
                            dict(scriptpubkey=SCRIPTS['wallet-b'], value_sats=100_000)]),
    ]:
        add_graph(name, rows=[row('source', 'wallet-a', 'outbound', raw=graph(outputs=outputs)),
                              row('target', 'wallet-b', 'inbound')], expected_candidates=1)
    for name, script, owners, expected in [
        ('ambiguous-output', 'wallet-b', ['wallet-b', 'wallet-c'], 0),
        ('self-owner-before-ambiguity', 'wallet-a', ['wallet-a', 'wallet-c'], 1),
        ('duplicate-owner-evidence', 'wallet-b', ['wallet-b', 'wallet-b'], 1),
    ]:
        index = OwnedIndex()
        for wallet in owners:
            index.add_script(SCRIPTS[script], OwnedMatch(wallet, wallet, 'treasury',
                'bitcoin', 'main', '', None, None, 'inventory', 'descriptor'))
        if script == 'wallet-a':
            index.add_script(SCRIPTS['wallet-b'], OwnedMatch('wallet-b', 'Hot', 'treasury',
                'bitcoin', 'main', '', None, None, 'inventory', 'descriptor'))
        add_graph(name, index=index,
            rows=[row('source', 'wallet-a', 'outbound', raw=graph(outputs=[
                dict(scriptpubkey=SCRIPTS[script], value_sats=30_000 if script == 'wallet-a' else 0),
                dict(scriptpubkey=SCRIPTS['wallet-b'], value_sats=100_000)])),
                row('target', 'wallet-b', 'inbound')], expected_candidates=expected)
    index = OwnedIndex()
    index.add_script(SCRIPTS['wallet-b'], OwnedMatch('wallet-b', 'Hot', 'treasury',
        'bitcoin', 'test', '', None, None, 'inventory', 'descriptor'))
    add_graph('owner-network-mismatch', index=index, expected_candidates=0)
    for name, raw, reason, expected in [
        ('unknown-output-value-fallback', graph(outputs=[dict(scriptpubkey=SCRIPTS['wallet-b'])]), REASON, 1),
        ('unknown-output-value-no-fallback', graph(outputs=[dict(scriptpubkey=SCRIPTS['wallet-b'])]),
            'ownership_transfer_destination_ambiguous', 0),
        ('no-graph-no-fallback', {}, 'ownership_transfer_destination_ambiguous', 0),
        ('empty-outputs-no-destination', graph(outputs=[]), REASON, 0),
        ('missing-graph-scope', dict(graph(), chain='unknown'), REASON, 0),
    ]:
        add_graph(name, rows=[row('source', 'wallet-a', 'outbound', raw=raw),
                              row('target', 'wallet-b', 'inbound')],
                  reasons=[('source', reason)], expected_candidates=expected)
    for name, source, inbound, expected in [
        ('physical-fallback', row('source', 'wallet-a', 'outbound'), row('target', 'wallet-b', 'inbound'), 1),
        ('legacy-fallback', row('source', 'wallet-a', 'outbound', scoped=False), row('target', 'wallet-b', 'inbound', scoped=False), 1),
        ('legacy-no-identity', dict(row('source', 'wallet-a', 'outbound', scoped=False), external_id=''), row('target', 'wallet-b', 'inbound', scoped=False), 0),
        ('legacy-identity-case', row('source', 'wallet-a', 'outbound', scoped=False), dict(row('target', 'wallet-b', 'inbound', scoped=False), external_id='PROVIDER-ID'), 0),
        ('legacy-identity-space', row('source', 'wallet-a', 'outbound', scoped=False), dict(row('target', 'wallet-b', 'inbound', scoped=False), external_id=' provider-id'), 0),
        ('legacy-hex-case', dict(row('source', 'wallet-a', 'outbound', scoped=False), external_id='A'*64), dict(row('target', 'wallet-b', 'inbound', scoped=False), external_id='a'*64), 1),
        ('legacy-asset-filter', row('source', 'wallet-a', 'outbound', scoped=False), dict(row('target', 'wallet-b', 'inbound', scoped=False), asset='LBTC'), 0),
        ('same-wallet-fallback', row('source', 'wallet-a', 'outbound'), row('target', 'wallet-a', 'inbound'), 0),
        ('zero-inbound', row('source', 'wallet-a', 'outbound'), row('target', 'wallet-b', 'inbound', 0), 0),
        ('negative-inbound', row('source', 'wallet-a', 'outbound'), row('target', 'wallet-b', 'inbound', -1), 0),
    ]:
        add_graph(name, rows=[source, inbound], expected_candidates=expected)
    add_graph('multiple-outbounds', rows=[row('source', 'wallet-a', 'outbound'),
        row('other', 'wallet-c', 'outbound'), row('target', 'wallet-b', 'inbound')], expected_candidates=0)
    add_graph('destination-asset-filter', rows=[row('source', 'wallet-a', 'outbound', raw=graph()),
        dict(row('target', 'wallet-b', 'inbound'), asset='LBTC')], expected_candidates=0)
    add_graph('nonpositive-group-cannot-offer-offgroup', rows=[
        row('source', 'wallet-a', 'outbound', scoped=False),
        row('target', 'wallet-b', 'inbound', 0, scoped=False),
        dict(row('other', 'wallet-b', 'inbound', 1, scoped=False), external_id='other')],
        expected_candidates=0)
    add_graph('graph-zero-receipt', rows=[
        row('source', 'wallet-a', 'outbound', raw=graph(0)),
        row('target', 'wallet-b', 'inbound', 0)], expected_candidates=0)
    for name, inbound in [
        ('network-filter', dict(row('target', 'wallet-b', 'inbound'), config_json='{"chain":"bitcoin","network":"test"}')),
        ('txid-filter', dict(row('target', 'wallet-b', 'inbound'), external_id='c'*64)),
    ]:
        add_graph(name, rows=[row('source', 'wallet-a', 'outbound', raw=graph()), inbound], expected_candidates=0)
    add_graph('txid-filter-without-source-scope', rows=[
        dict(row('source', 'wallet-a', 'outbound', scoped=False), external_id=TXID),
        dict(row('target', 'wallet-b', 'inbound', scoped=False), external_id=TXID),
        dict(row('other', 'wallet-b', 'inbound'), external_id='c'*64)], expected_candidates=1)
    for amount in (100_000_000, 400_001_000):
        tolerance = max(amount // 100, 2_500_000)
        for delta in (-tolerance-1, -tolerance, 0, tolerance, tolerance+1):
            add_graph(f'tolerance-{amount}-{delta}', rows=[
                row('source', 'wallet-a', 'outbound', amount, raw=graph(amount)),
                row('target', 'wallet-b', 'inbound', amount+delta)],
                expected_candidates=int(abs(delta) <= tolerance))
    add_graph('exact-preferred-to-compatible', rows=[
        row('source', 'wallet-a', 'outbound', raw=graph()), row('target', 'wallet-b', 'inbound'),
        row('other', 'wallet-b', 'inbound', 100_000_001)], expected_candidates=1)
    add_graph('exact-conflicts-keep-order', rows=[
        row('source', 'wallet-a', 'outbound', raw=graph()), row('target-z', 'wallet-b', 'inbound'),
        row('target-a', 'wallet-b', 'inbound')], expected_candidates=2)
    add_graph('compatible-conflicts-keep-order', rows=[
        row('source', 'wallet-a', 'outbound', raw=graph()), row('target-z', 'wallet-b', 'inbound', 100_000_001),
        row('target-a', 'wallet-b', 'inbound', 99_999_999)], expected_candidates=2)
    add_graph('partial-source-is-strong', rows=[
        row('source', 'wallet-a', 'outbound', 200_000_000, raw=graph()),
        row('target', 'wallet-b', 'inbound')], expected_candidates=1)
    add_graph('unscoped-destination-is-strong', rows=[
        row('source', 'wallet-a', 'outbound', raw=graph()), row('target', 'wallet-b', 'inbound', scoped=False)], expected_candidates=1)
    add_graph('ambiguous-source-is-strong', reasons=[('source', 'ownership_transfer_source_ambiguous')],
              expected_candidates=1)
    for script in (1, True, [1], {'a': 1}):
        add_graph(f'output-script-refusal-{type(script).__name__}', rows=[
            row('source', 'wallet-a', 'outbound', raw=graph(outputs=[
                dict(scriptpubkey=script, value_sats=100_000)])), row('target', 'wallet-b', 'inbound')])
    for field in ('n', 'vout'):
        raw = graph()
        (raw['vout'][0] if field == 'n' else raw['vin'][0])[field] = float('inf')
        add_graph(f'graph-index-refusal-{field}', rows=[row('source', 'wallet-a', 'outbound', raw=raw),
            row('target', 'wallet-b', 'inbound')])
    add_graph('txid-python-whitespace', rows=[
        dict(row('source', 'wallet-a', 'outbound', scoped=False), external_id='\x1c'+TXID+'\x1f'),
        dict(row('target', 'wallet-b', 'inbound', scoped=False), external_id='\x1c'+TXID+'\x1f'),
        dict(row('other', 'wallet-b', 'inbound'), external_id='c'*64)], expected_candidates=1)
    for script in (1, True, [1], {'a': 1}):
        raw = graph()
        raw['vin'][0]['prevout']['scriptpubkey'] = script
        add_graph(f'input-script-refusal-{type(script).__name__}', rows=[
            row('custody-tax:source', 'wallet-a', 'outbound', raw=raw),
            row('target', 'wallet-b', 'inbound')], reasons=[('custody-tax:source', REASON)],
            expected_error='AttributeError')
    raw = graph()
    raw['vin'] = []
    raw['vout'][0]['scriptpubkey'] = [1]
    add_graph('scope-output-script-refusal', rows=[
        row('custody-tax:source', 'wallet-a', 'outbound', raw=raw),
        row('target', 'wallet-b', 'inbound')], reasons=[('custody-tax:source', REASON)],
        expected_error='AttributeError')
    index = OwnedIndex()
    match = OwnedMatch('wallet-a', 'Cold', 'treasury', 'bitcoin', 'main', '', None, None,
                       'inventory', 'descriptor')
    index.add_outpoint('b'*64, 0, match)
    raw = graph()
    raw['vin'][0]['prevout']['scriptpubkey'] = [1]
    raw['vin'].append(dict(txid='c'*64, vout=1, prevout=dict(scriptpubkey={'bad': 1})))
    add_graph('scope-skips-owned-outpoint-script', index=index, rows=[
        row('custody-tax:source', 'wallet-a', 'outbound', raw=raw),
        row('target', 'wallet-b', 'inbound')], reasons=[('custody-tax:source', REASON)],
        expected_error='AttributeError')
    raw = graph()
    raw['vout'][0]['n'] = float('inf')
    for name, reasons, components in [
        ('no-quarantines-skips-graph-refusal', [], []),
        ('nonpairable-skips-graph-refusal', [('source', 'missing_spot_price')], []),
        ('occupied-source-skips-graph-refusal', [('source', REASON)], [component(inbound=None)]),
    ]:
        add_graph(name, rows=[row('source', 'wallet-a', 'outbound', raw=raw),
                             row('target', 'wallet-b', 'inbound')],
                  reasons=reasons, components=components, expected_candidates=0)
    add_graph('multiple-sources-sort-by-id', rows=[
        row('source-z', 'wallet-a', 'outbound', raw=graph()),
        row('source-a', 'wallet-a', 'outbound', raw=graph()),
        row('target', 'wallet-b', 'inbound')], reasons=[
            ('source-z', REASON), ('source-a', REASON)], expected_candidates=2)
    add_graph('multiple-destinations-sort-by-wallet', rows=[
        row('source', 'wallet-a', 'outbound', 200_000_000, raw=graph(outputs=[
            dict(scriptpubkey=SCRIPTS['wallet-c'], value_sats=100_000),
            dict(scriptpubkey=SCRIPTS['wallet-b'], value_sats=100_000)])),
        row('target-c', 'wallet-c', 'inbound'), row('target-b', 'wallet-b', 'inbound')],
        expected_candidates=2)
    index = OwnedIndex()
    for script, wallet in [('wallet-b', 'wallet-b'), ('wallet-c', 'wallet-b'),
                           ('wallet-c', 'wallet-c')]:
        index.add_script(SCRIPTS[script], OwnedMatch(wallet, wallet, 'treasury',
            'bitcoin', 'main', '', None, None, 'inventory', 'descriptor'))
    add_graph('ambiguity-discards-earlier-destination', index=index, rows=[
        row('source', 'wallet-a', 'outbound', raw=graph(outputs=[
            dict(scriptpubkey=SCRIPTS['wallet-b'], value_sats=100_000),
            dict(scriptpubkey=SCRIPTS['wallet-c'], value_sats=1)])),
        row('target', 'wallet-b', 'inbound')], expected_candidates=0)
    raw = graph()
    raw['txid'] = 'c' * 64
    add_graph('contradictory-graph-scope', rows=[
        row('custody-tax:source', 'wallet-a', 'outbound', raw=raw),
        row('target', 'wallet-b', 'inbound')], reasons=[('custody-tax:source', REASON)],
        expected_candidates=0, index=index)
    return cases

def renderer_vectors():
    import uuid
    from kassiber.core.engines import ledger_response
    from tests.custody_snapshot import to_jsonable
    from tests.tax_ledger_generator import corpus
    from tests.rust_port.journal_render_vectors import seeded_ids
    cases = []
    book = snapshot(0)
    original = compute_custody_projection(book)
    quantity = replace(original.quantity_state,
        canonical_input=replace(original.quantity_state.canonical_input, events=()),
        projection=replace(original.quantity_state.projection, observations=(), decisions=(), postings=()),
        tax_eligibility=replace(original.quantity_state.tax_eligibility, eligible_decisions=()))
    empty = dict(schema_version=2, ok=True, entries=[], holdings=[], tax_summary=[],
                 carried_rows=[], intra_audit=[], quarantines=[])
    base = replace(original, rows=[], owned_index=None, quantity_state=quantity,
        finalized_tax_projection=replace(original.finalized_tax_projection, rows=(), intra_pairs=(), quarantines=()))

    def add(name, custody=base, response=None, profile=None, group='other'):
        audits = []
        original_audit = ledger_response.render_audit

        def observed_audit(inputs, audit):
            result = original_audit(inputs, audit)
            audits.append(result)
            return result

        with patch.object(uuid, 'uuid4', seeded_ids(name)), \
             patch.object(ledger_response, 'render_audit', observed_audit):
            record = clean_record(capture_projection(profile or book['profile'], custody,
                response_override=deepcopy(response if response is not None else empty)))
        assert 'custody' in record, (name, record)
        if 'state' in record['expected']:
            record['expected_audit'] = to_jsonable(audits, redact_config=False)
        cases.append(dict(name=name, group=group, **record))

    observation = original.quantity_state.projection.observations[0]
    posting = next(p for p in original.quantity_state.projection.postings if p.location_kind == 'wallet')
    def balance(name, postings=(), observations=(), loan=(), channel=None, components=(), holdings=()):
        state = replace(quantity, projection=replace(quantity.projection,
            observations=tuple(observations), postings=tuple(postings)))
        add(name, replace(base, quantity_state=state, loan_legs=list(loan),
                         channel_roles=channel or {}, active_components=list(components)),
            dict(empty, holdings=list(holdings)), group='balances')
    p = replace(posting, observation_hash=observation.quantity_hash, amount_msat=1)
    balance('balance-no-differences')
    balance('balance-wallet-only', [p, replace(p, location_kind='external', amount_msat=999)])
    balance('balance-equal', [replace(p, amount_msat=100_000_000_000)], holdings=[
        dict(wallet='Cold', asset='BTC', quantity='1E0', cost_basis='0E0')])
    balance('balance-current-only', holdings=[dict(wallet='Cold', asset='BTC', quantity='1E0', cost_basis='0E0')])
    loan = [dict(transaction_id=observation.transaction_id, role='principal')]
    balance('balance-loan-reason', [p], [observation], loan=loan)
    balance('balance-null-loan-id', [p], [observation], loan=[dict(transaction_id=None, role='principal')])
    balance('balance-channel-overrides-loan', [p], [observation], loan=loan,
        channel={observation.transaction_id: 'channel_open'})
    for suspense in (-1, 0, 1):
        comp = dict(validation=dict(suspense_msat=suspense), legs=[
            dict(role='source', transaction_id=observation.transaction_id),
            dict(role='destination', transaction_id='other-id'),
            dict(role='source', transaction_id=None)])
        balance(f'balance-suspense-{suspense}', [p], [observation], loan=loan,
            channel={observation.transaction_id: 'channel_open'}, components=[comp])
    other = replace(observation, quantity_hash='other-hash', transaction_id='other-id')
    balance('balance-ambiguous-reasons', [p, replace(p, observation_hash='other-hash')],
        [observation, other], loan=[*loan, dict(transaction_id='other-id', role='collateral')])
    balance('balance-same-reason-twice', [p, p], [observation], loan=loan)
    balance('balance-missing-observation', [p], loan=loan)
    for n in (2**127+1, -2**127-1):
        balance(f'balance-wide-{n}', [replace(p, amount_msat=n), p])
    balance('balance-sorted-union', [replace(p, location_id='wallet-z', asset='LBTC'), p],
        holdings=[dict(wallet='Hot', asset='BTC', quantity='1E0', cost_basis='0E0')])

    source = dict(row('source', 'wallet-a', 'outbound'), custody_finalized_tax_projection=True)
    custody = replace(base, finalized_tax_projection=replace(base.finalized_tax_projection, rows=(source,)))
    for boundary in ('coinjoin', 'collaborative', 'payjoin', 'payment-in-coinjoin', 'sweep',
                     'unknown', None, '', ' PAYJOIN ', '\x1cPAYJOIN\x1f'):
        changed = dict(source, privacy_boundary=boundary, raw_json=json.dumps(dict(source='coordinator')))
        c = replace(custody, finalized_tax_projection=replace(custody.finalized_tax_projection, rows=(changed,)))
        response = dict(empty, quarantines=[dict(row='source', reason='privacy_hop_unresolved',
            detail_kind='privacy_hop', detail=dict(privacy_boundary='old', source='old', keep='yes'))])
        add(f'privacy-boundary-{boundary}', c, response)
    for key in ('islikelycoinjoin', 'is_likely_coinjoin', 'isLikelyCoinJoin', 'likely_coinjoin', 'likelyCoinJoin'):
        for value in (True, False, None, '', '1', 'true', 'yes', 'on', 'false', ' YES ',
                      '\x1cYES\x1f'):
            changed = dict(source, privacy_boundary='payjoin', raw_json=json.dumps({key: value}))
            c = replace(custody, finalized_tax_projection=replace(custody.finalized_tax_projection, rows=(changed,)))
            response = dict(empty, quarantines=[dict(row='source', reason='privacy_hop_unresolved',
                detail_kind='privacy_hop', detail={})])
            add(f'privacy-bool-{key}-{value!r}', c, response)
    for kind in ('pricing_review', 'hold', 'plain', 'partner_leg'):
        changed = dict(source, pricing_quality='exact', pricing_provider='provider')
        c = replace(custody, finalized_tax_projection=replace(custody.finalized_tax_projection, rows=(changed,)))
        response = dict(empty, quarantines=[dict(row='source', reason='missing_spot_price',
            detail_kind=kind, detail=dict(pricing_provider='old', keep='yes'))])
        add(f'quarantine-template-{kind}', c, response)

    for name, raw in [('mapping', dict(source='mapping', islikelycoinjoin=True)),
                      ('malformed', '{bad'), ('empty', ''), ('null', None),
                      ('list', '[1]'), ('falsey-source', '{"source":false}')]:
        changed = dict(source, privacy_boundary='payjoin', raw_json=raw)
        c = replace(custody, finalized_tax_projection=replace(custody.finalized_tax_projection, rows=(changed,)))
        response = dict(empty, quarantines=[dict(row='source', reason='privacy_hop_unresolved',
            detail_kind='privacy_hop', detail={})])
        add(f'privacy-raw-{name}', c, response)

    for label in ('', None):
        wallets = {key: dict(value) for key, value in base.wallet_refs_by_id.items()}
        wallets['wallet-a']['label'] = label
        add(f'holdings-skips-label-{label!r}', replace(base, wallet_refs_by_id=wallets),
            dict(empty, holdings=[dict(wallet='' if label == '' else 'None',
                asset='BTC', quantity='1E0', cost_basis='0E0')]))
    for direction in ('outbound', 'inbound'):
        projected = dict(source, direction=direction, journal_transaction_id='anchor')
        add(f'payout-source-direction-{direction}', replace(base,
            finalized_tax_projection=replace(base.finalized_tax_projection, rows=(projected,)),
            direct_payout_records=[dict(id='payout', out_transaction_id='anchor')]))

    for case in corpus():
        if case.name not in ('generic-ledger-fifo', 'at-ledger-fifo'):
            continue
        c = replace(original, finalized_tax_projection=case.inputs.finalized_tax_projection,
            wallet_refs_by_id=case.inputs.wallet_refs_by_id, direct_payout_records=case.inputs.direct_payout_records)
        response = capture_projection(case.profile, c)['response']
        add(f'render-{case.name}', c, response, case.profile)
        for key, values in [('at_regime', [None, 'alt', 'neu']),
                            ('regime_flows', [None, dict(out=dict(alt=1, neu=2),
                                                       **{'in': dict(alt=3, neu=4)})]),
                            ('at_pool', [None, '', 'pool'])]:
            for value in values:
                changed = deepcopy(response)
                changed['intra_audit'][0][key] = value
                add(f'audit-{case.name}-{key}-{value!r}', c, changed, case.profile)
        pairs = tuple(dict(pair, group_id='') for pair in c.finalized_tax_projection.intra_pairs)
        add(f'audit-{case.name}-ungrouped', replace(c,
            finalized_tax_projection=replace(c.finalized_tax_projection, intra_pairs=pairs)), response, case.profile)
        changed = deepcopy(response)
        changed['intra_audit'][0]['pair'] = 999
        add(f'audit-{case.name}-missing-pair', c, changed, case.profile)
        for side in ('out', 'in'):
            pair = c.finalized_tax_projection.intra_pairs[0]
            wallets = {key: value for key, value in c.wallet_refs_by_id.items()
                       if key != pair[side]['wallet_id']}
            add(f'audit-{case.name}-missing-{side}-wallet',
                replace(c, wallet_refs_by_id=wallets), response, case.profile)
        for category in ('income_general', 'income_capital_yield', 'neu_gain', 'neu_loss', 'alt_spekulation', 'unknown', None):
            changed = deepcopy(response)
            entry = next(e for e in changed['entries'] if 'at_category' in e)
            entry['at_category'] = category
            add(f'category-{case.name}-{category}', c, changed, case.profile)
    return cases


if __name__ == '__main__':
    records = vectors() + renderer_vectors()
    if '--accept' in sys.argv:
        PATH.write_text(json.dumps(records, indent=2, ensure_ascii=True) + '\n')
    else:
        assert json.loads(PATH.read_text()) == records
    print(f'{len(records)} Python branch witnesses')
