"""Independent Python operands and request oracle for the R2g builder boundary."""
import json
from pathlib import Path
from datetime import datetime, timezone
from copy import deepcopy
from dataclasses import fields, is_dataclass

from kassiber.core.custody_tax_projection import FinalizedTaxProjection
from kassiber.core.engines.base import TaxEngineLedgerInputs
from kassiber.core.engines.ledger_request import build_request


def wire(value):
    if isinstance(value, datetime):
        return {'$datetime': value.isoformat()}
    if is_dataclass(value):
        return {'$type': type(value).__name__, **{f.name: wire(getattr(value, f.name)) for f in fields(value)}}
    if isinstance(value, dict):
        return {k: wire(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [wire(v) for v in value]
    if isinstance(value, (set, frozenset)):
        return {'$set': [wire(v) for v in sorted(value)]}
    return value


def outcome(profile, inputs):
    # The empty control belongs to the ledger wrapper, not build_request().
    if not inputs.finalized_tax_projection.rows:
        return {'outcome': 'empty'}
    try:
        return {'outcome': 'request', 'request': build_request(profile, inputs)}
    except Exception as exc:
        return {'outcome': 'refusal', 'error': {
            'family': type(exc).__name__, 'message': str(exc),
            'details': getattr(exc, 'details', None), 'hint': getattr(exc, 'hint', None),
            'code': getattr(exc, 'code', None), 'invariant': getattr(exc, 'invariant', None),
            'retryable': getattr(exc, 'retryable', None), 'error_class': getattr(exc, 'error_class', None),
        }}


def vectors(*, include_goldens=False):
    profile = {'label': 'Default', 'tax_country': 'generic', 'fiat_currency': 'EUR', 'gains_algorithm': 'FIFO'}
    row = {'custody_finalized_tax_projection': True, 'id': 'custody-tax:1', 'journal_transaction_id': 'anchor', 'wallet_id': 'w', 'asset': 'BTC',
           'direction': 'inbound', 'amount': 1, 'fee': 0, 'occurred_at': '2024-01-01T00:00:00Z'}
    wallets = {'w': {'id': 'w', 'label': 'Wallet'}}
    cases = []

    def add(name, rows=None, pairs=(), refs=None, config=None, cross=()):
        projection = FinalizedTaxProjection((), (), (), (), ())
        # Deliberate invariant injections exercise the builder's fail-closed seam.
        object.__setattr__(projection, 'rows', tuple(deepcopy([row] if rows is None else rows)))
        object.__setattr__(projection, 'intra_pairs', tuple(deepcopy(pairs)))
        object.__setattr__(projection, 'cross_asset_pairs', tuple(deepcopy(cross)))
        inputs = TaxEngineLedgerInputs(finalized_tax_projection=projection, wallet_refs_by_id=deepcopy(wallets if refs is None else refs))
        p = {**profile, **(config or {})}
        cases.append({'case': name, 'input': {'profile': p, 'projection': wire(projection), 'wallets': wire(list(inputs.wallet_refs_by_id.items()))}, 'expected': outcome(p, inputs)})

    add('empty-skips-invalid-configuration', [], config={'label': '#'})
    add('base')
    for key, values in {
        'label': [None, '', 'Bad#', '%', '%%', '%(assets)s', '%(missing)s', '%(holders)s'],
        'tax_country': ['invalid', 'at', '', None],
        'tax_long_term_days': [-1, 0, 1, 2**100, 'x', True, 2.5],
        'fiat_currency': ['xxx', 'EUR', 'bad', None],
        'cost_basis_pool_scope': ['wallet', '', None],
        'gains_algorithm': ['lifo', 'MOVING_AVERAGE_AT', None],
        'require_coarse_review': [False, True, 'YES', 'off'],
    }.items():
        for index, value in enumerate(values):
            add(f'profile-{key}-{index}', config={key: value})
    for key, values in {
        'id': ['raw', '', None], 'journal_transaction_id': ['', 1, None],
        'wallet_id': ['missing', None], 'direction': ['unknown', None],
        'amount': [0, -1, True, 1.5, 2**100], 'fee': [-1, True, '0', 2**100],
        'amount_includes_fee': [True, [], 'false'], 'custody_component_test': [None],
        'fiat_rate_exact': ['', '-0.00', '1.2300', 'NaN', 'sNaN001', 'Infinity', 'bad'],
        'fiat_value_exact': ['0', '-0E+10', 'bad'],
        'occurred_at': [None, 123, '', 'bad', '20241301', '20240230', '00000101',
                        '2024-01-01T你你', '2024-13-01Tbad', '2024-01-01T25', '2024-01-01T123460',
                        '2024-W54-1', '2024-W01-8', '２０２４-１３-０１',
                        '2024-01-01T25:00:00+99:99', '2024-02-30', '0000-01-01', '2024-13-01',
                        '2024-01-01T24:00:00', '2024-01-01T00:60:00', '2024-01-01T00:00:60',
                        '20240101', '2024-W01-1', '2024-01-01T12', '2024-01-01T12.5',
                        '2024-01-01T12:34:56+00:00:00.5', '2024-01-01T00:00:00+01:99',
                        '2024-01-01T00:00:00+01:00:99', '2024-01-01T00:00:00+99:99',
                        '2024-01-01T00:00:00-24:00', '2024-01-01T00:00:00+24',
                        '2024-01-01T00:00:00+01:', '２０２４-０１-０１T１２:００:００Z', '2024-01-01', '2024-01-01T00:00:00z',
                        '2024-01-01T00:00:00+24:00', '0001-01-01T00:00:00+01:00',
                        '9999-12-31T23:59:59-01:00', '2024-01-01T12:34:56.1234567+02:30'],
        'kind_override': ['', 'Mining-Reward', False], 'note': ['note', 123, False],
        'privacy_boundary': ['coinjoin', 'payjoin', 'unknown'],
        'taxability_override': [False, 0, 0.0, '0', None],
        'at_regime_override': ['alt', 'neu', 'bad'], 'at_category_override': [False, 1, 'income'],
    }.items():
        for index, value in enumerate(values):
            add(f'row-{key}-{index}', [{**row, key: value}])
    add('row-field-fallback', [{**row, 'fiat_rate_exact': '1.2500', 'fiat_rate': 9,
                               'fiat_value_exact': '-0.00', 'fiat_value': 8,
                               'note': 'note', 'description': 'description', 'kind': 'income'}])
    # Coarse-price acceptance (#708): only an unaccepted or changed coarse
    # price is held; a projected row is judged by its transaction's price.
    coarse = {**row, 'pricing_quality': 'coarse_fallback', 'fiat_rate_exact': '3197.6'}
    for name, change in [
        ('unaccepted', {}),
        ('accepted-trailing-zero', {'pricing_review_accepted_price': 'rate:3197.60'}),
        ('accepted-projected', {'fiat_rate_exact': '1', 'pricing_review_price': 'rate:3197.6',
                                'pricing_review_accepted_price': 'rate:3197.6'}),
        ('changed', {'pricing_review_accepted_price': 'rate:3197.5'}),
        ('kind-changed', {'pricing_review_accepted_price': 'value:3197.6'}),
        ('projected-unpriced', {'pricing_review_price': None, 'pricing_review_accepted_price': 'rate:1'}),
        ('malformed', {'pricing_review_accepted_price': 'rate:abc'}),
        ('signaling', {'pricing_review_accepted_price': 'rate:sNaN'}),
        ('empty', {'pricing_review_accepted_price': ''}),
        ('value-only', {'fiat_rate_exact': None, 'fiat_value_exact': '12.50',
                        'pricing_review_accepted_price': 'value:12.5'}),
        ('nan-rate', {'fiat_rate_exact': 'NaN', 'pricing_review_accepted_price': 'rate:NaN'}),
        ('integer-acceptance', {'pricing_review_accepted_price': 5}),
        ('exact-quality', {'pricing_quality': 'exact', 'pricing_review_accepted_price': 'rate:abc'}),
    ]:
        add(f'row-coarse-{name}', [{**coarse, **change}], config={'require_coarse_review': True})
    add('row-order-and-wallet-order', [{**row, 'id': 'custody-tax:z'}, {**row, 'id': 'custody-tax:a'}],
        refs={'z': {'id': 'z', 'label': 'Unused'}, 'w': {'id': 'w', 'label': 'Wallet'}})
    from kassiber.core.ownership_transfers import ChainConfirmationObservation
    txid = 'a' * 64
    observed = datetime(2024, 1, 1, tzinfo=timezone.utc)
    physical = {**row, 'external_id': txid, 'raw_json': json.dumps({
        'chain': 'bitcoin', 'network': 'main', 'txid': txid, 'status': {'confirmed': False},
        'vin': [{'txid': 'b' * 64, 'vout': 1, 'prevout': {'value_sats': 13, 'scriptpubkey': '0014' + '1' * 40}}],
        'vout': [{'n': 0, 'value_sats': 10, 'scriptpubkey': '0014' + '2' * 40}]})}
    add('row-pending-hold', [physical])
    add('row-closed-confirmed', [{**physical, '_closed_confirmation_observation':
                                ChainConfirmationObservation(('bitcoin', 'main', txid), observed, True)}])
    add('row-closed-different-scope', [{**physical, '_closed_confirmation_observation':
                                      ChainConfirmationObservation(('bitcoin', 'main', 'b' * 64), observed, True)}])
    # The first error must survive multiple later competing failures.
    add('ordering-wallet-before-assets', [{**row, 'asset': 'BTC#'}], refs={'w': {'id': 'bad', 'label': 'Wallet:'}}, config={'label': '#'})
    add('ordering-assets-sorted', [{**row, 'asset': 'ETH:'}, {**row, 'asset': 'BTC#'}], config={'label': '#'})
    add('ordering-profile-before-policy', config={'label': '#', 'tax_country': 'bad'})
    add('ordering-pool-before-row', [{**row, 'id': 'raw'}], config={'cost_basis_pool_scope': 'wallet'})
    add('ordering-wallet-id-before-row', [{**row, 'id': 'raw'}], refs={'w': {'id': 'bad', 'label': 'Wallet'}})
    add('ordering-fee-before-amount', [{**row, 'amount': -1, 'fee': -1}])
    add('ordering-row-before-pairs', [{**row, 'amount': -1}], pairs=[{'out': row, 'in': row, 'group_block_rows': True}])
    out = {**row, 'id': 'custody-tax:out', 'direction': 'outbound', 'fee': 1}
    incoming = {**row, 'id': 'custody-tax:in'}
    for name, change in [('normal', {}), ('group', {'group_block_rows': True}), ('privacy', {'out': {**out, 'privacy_boundary': 'coinjoin'}}), ('unequal', {'in': {**incoming, 'amount': 2}}), ('crossed', {'in': {**incoming, 'asset': 'ETH'}})]:
        add('pair-' + name, [out, incoming], pairs=[{'out': out, 'in': incoming, **change}])
    add('pair-shared', [out, incoming], pairs=[{'out': out, 'in': incoming}] * 2)
    if not include_goldens:
        return cases
    from tests.tax_ledger_codec import decode_inputs
    for case in json.loads(Path('tax-engine/core/tests/data/ledger_fixtures.json').read_text())['cases']:
        p, inputs = decode_inputs(case['inputs'])
        cases.append({'case': 'golden-' + case['name'], 'input': {
            'profile': wire(p), 'projection': wire(inputs.finalized_tax_projection),
            'wallets': wire(list(inputs.wallet_refs_by_id.items())),
        }, 'expected': outcome(p, inputs)})
    return cases


def active_inputs(seed_inputs, observations):
    """Count distinct active operands owned by this seed, never companion output."""
    def key(value):
        return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True)
    owned = {key(value) for value in seed_inputs}
    return len({key(value) for value, result in observations
                if key(value) in owned and result.get('outcome') == 'request'
                and result['request']['rows']})


def require_activity(seed_inputs, observations):
    count = active_inputs(seed_inputs, observations)
    assert count > 0, 'vacuous fusion probe: no distinct active input belongs to the seed'
    return count
