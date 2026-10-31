import argparse
import copy
from dataclasses import asdict
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core.lightning import cln
from kassiber.errors import AppError

OUTPUT = ROOT / 'crates/chain/fixtures/python_cln.json'
SECRET = 'DISCARDED_CLN_SECRET'
METHODS = list(cln.CLN_ALLOWED_METHODS)


def base():
    return {
        'getinfo': {'id': SECRET, 'alias': SECRET, 'blockheight': 42, 'num_peers': 3},
        'listfunds': {'outputs': [{'amount_msat': '1000msat', 'status': 'confirmed', 'address': SECRET}]},
        'listpeerchannels': {'channels': [
            {'channel_id': '1', 'short_channel_id': '1x2x3', 'peer_id': 'peer-private-or-public', 'private': True, 'total_msat': '9007199254740993000msat', 'to_us_msat': 0, 'state': 'CHANNELD_NORMAL', 'htlcs': [SECRET]},
            {'channel_id': '2', 'short_channel_id': '2x3x4', 'peer_id': 'public-peer', 'peer_alias': 'Comma, newline\n雪', 'total_msat': 23000, 'state': 'pending_close'},
            {'channel_id': '3', 'short_channel_id': '3x4x5', 'state': 'CLOSED', 'total_msat': 99000},
        ]},
        'listforwards': {'forwards': [
            {'in_channel': '2x3x4', 'out_channel': '1x2x3', 'fee_msat': '1501msat', 'status': 'settled', 'received_time': 1, 'erring_node': SECRET},
            {'in_channel': '2x3x4', 'out_channel': '1x2x3', 'fee_msat': 1501, 'status': 'settled', 'route': [SECRET]},
        ]},
        'listpays': {'pays': [
            {'amount_msat': 10000, 'amount_sent_msat': 11999, 'status': 'complete', 'payment_preimage': SECRET, 'bolt11': SECRET},
            {'amount_msat': 'invalid-ignored-rebalance', 'status': 'paid', 'rebalance': True},
        ]},
        'listinvoices': {'invoices': [{'payment_preimage': SECRET, 'payment_secret': SECRET, 'bolt11': SECRET, 'description': SECRET}]},
        'bkpr-listincome': {'income_events': [{'tag': 'rebalance_fee', 'debit_msat': 1999}, {'tag': 'onchain_fee', 'debit_msat': 1001}]},
        'bkpr-listbalances': {'accounts': [{'balances': [{'balance_msat': 'ignored-invalid', 'secret': SECRET}]}]},
        'bkpr-listaccountevents': {'events': [{'txid': SECRET, 'tag': 'ignored', 'description': SECRET}]},
    }


def cases():
    yield 'base-discard', base(), 30, []
    yield 'empty', {k: {} for k in METHODS}, 30, []
    for window in [1, 30, 365]:
        yield f'window-unfiltered-{window}', base(), window, []
    amounts = [None, '', True, False, 0, -1, -1001, 1501.0, 1.5, '1501', '1,501msat', '1.5sat', '-1.501sat', '0.00000001501btc', {'msat': '1501'}, {'millisatoshis': '1501'}, {'millisatoshi': 1501}, {'unknown': SECRET}, [], '١٥٠١', '1_501', 'NaNmsat', 'Infinitysat', 'bad', 2**120,
               '12345678901234567890123456789012345msat', '12345678901234567890123456789012355msat', '12345678901234567890123456789012500msat', '12345678901234567890123456789013500msat', '1e-1000100msat', '1e1000000sat']
    for slot in ['forward-fee', 'forward-failed-amount', 'capacity', 'local', 'base-fee', 'funds', 'payment', 'income']:
        for index, amount in enumerate(amounts):
            data = base()
            if slot == 'forward-fee': data['listforwards']['forwards'][0]['fee_msat'] = amount
            elif slot == 'forward-failed-amount': data['listforwards']['forwards'].append({'status': 'failed', 'in_msat': amount})
            elif slot == 'capacity': data['listpeerchannels']['channels'][0]['total_msat'] = amount
            elif slot == 'local': data['listpeerchannels']['channels'][0]['to_us_msat'] = amount
            elif slot == 'base-fee': data['listpeerchannels']['channels'][0]['fee_base_msat'] = amount
            elif slot == 'funds': data['listfunds']['outputs'][0]['amount_msat'] = amount
            elif slot == 'payment': data['listpays']['pays'][0]['amount_sent_msat'] = amount
            elif slot == 'income': data['bkpr-listincome']['income_events'][0]['debit_msat'] = amount
            yield f'{slot}-{index}', data, 30, []
    for status in ['settled', 'SETTLED', 'complete', 'completed', 'offered', 'pending', 'failed', None, ' settled ']:
        data = base()
        data['listforwards']['forwards'][0]['status'] = status
        data['listpays']['pays'][0]['status'] = status
        yield f'status-{status}', data, 30, []
    for state in ['CHANNELD_NORMAL', 'closingd_complete', 'onchaind_our_unilateral', 'breach', 'shutdown', 'closing', 'await', 'open', 'normal', 'close', 'unknown', '', 'channeld_shutting_down']:
        data = base()
        data['listpeerchannels']['channels'][0].update(state=state, peer_connected=False)
        yield f'channel-state-{state}', data, 30, []
    for private in [True, False, 'true', 1, 0, None]:
        for alias in [None, '', '  Local  ', '\ud800', '雪🚀']:
            data = base()
            data['listpeerchannels']['channels'][0].update(private=private, is_private=True, peer_alias=alias)
            yield f'alias-{private!r}-{alias!r}', data, 30, []
    for index, row in enumerate([
        {'channel_id': '1', 'peer_id': 'x', 'alias': '1x2x3'},
        {'channel_id': '1', 'short_channel_id': ' 1x2x3\n', 'peer_id': 'x'},
        {'channel_id': '1', 'short_channel_id': 'bad-secret'},
        {'channel_id': '1', 'funding_outpoint': 'bad-secret'},
        {'channel_id': '1', 'funding': {'txid': 'ab'*32, 'output': 0, 'secret': SECRET}},
        {'channel_id': '1', 'funding': {'txid': 'ab'*32, 'output': 0, 'outnum': 1, 'secret': SECRET}},
        {'channel_id': '1', 'funding_txid': 'ab'*32, 'funding_outnum': 0},
        {'channel_id': '1', 'total_msat': 0, 'our_amount_msat': 1501, 'their_amount_msat': -999},
        {'channel_id': '1', 'fee_proportional_millionths': 'bad'},
        {'channel_id': '1', 'fee_proportional_millionths': float('nan')},
        {'channel_id': '1', 'opened_at': float('inf')},
        {'channel_id': '1', 'closed_at': '1e100'},
        {'channel_id': '1', 'opened_at': 'bad'},
        {'channel_id': '1', 'state': 'CLOSED', 'fee_base_msat': 'bad'},
    ]):
        data = base()
        data['listpeerchannels']['channels'] = [row]
        yield f'channel-shape-{index}', data, 30, []
    data = base()
    data['listpeerchannels']['channels'] = [{'channel_id': '1', 'to_us_msat': True, 'their_amount_msat': 999}]
    data['listforwards']['forwards'] = [{'status': 'settled', 'fee_msat': True}, {'status': 'settled', 'fee_msat': 999}]
    yield 'boolean-msat-does-not-cross-sat-boundary', data, 30, []
    for method in METHODS:
        yield f'optional-failure-{method}', base(), 30, [method]
        data = base()
        data[method] = {'ignored': [None, 'bad', SECRET]}
        yield f'malformed-resource-{method}', data, 30, []
    for key in ['blockheight', 'num_peers']:
        for number in [float('inf'), float('nan'), 'bad', True]:
            data = base()
            data['getinfo'][key] = number
            yield f'info-{key}-{number}', data, 30, []
    data = base()
    data['listfunds']['outputs'] = [{'status': 'spent', 'amount_msat': 'bad'}, {'reserved_to_block': 0, 'amount_msat': 'bad'}]
    yield 'funds-reservation-zero-is-string', data, 30, []
    data = base()
    data['listpeerchannels']['channels'].append({'channel_id': '4', 'peer_id': 'public-peer', 'peer_alias': 'Last alias wins'})
    yield 'alias-map-last-wins', data, 30, []


def evaluate(name, data, window, failures):
    calls = []
    def caller(method, args):
        calls.append({'method': method, 'args': args})
        if method in failures:
            raise AppError('synthetic failure', code='sync_error')
        return copy.deepcopy(data[method])
    result = {'name': name, 'data': data, 'window': window, 'failures': failures}
    try:
        raw = cln.fetch_core_lightning_snapshot({}, rpc_call=caller)
        snapshot = cln.build_node_snapshot(raw, window_days=window)
        result['expected'] = {
            'channels': [{'id': c.id, 'peer_alias': c.peer_alias, 'capacity_sat': c.capacity_sat, 'earned_routing_sat': c.earned_routing_sat} for c in snapshot.channels],
            'routing': asdict(snapshot.routing) if snapshot.routing else None,
        }
    except Exception as error:
        result['error'] = {'type': type(error).__name__, 'code': getattr(error, 'code', 'internal_error')}
    result['calls'] = calls
    return result


def render():
    return json.dumps([evaluate(*case) for case in cases()], ensure_ascii=True, indent=2) + '\n'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    rendered = render()
    if args.check:
        if OUTPUT.read_text() != rendered:
            raise SystemExit('CLN acquisition vectors differ from Python')
    else:
        OUTPUT.write_text(rendered)
    print(f'CLN acquisition: {len(json.loads(rendered))} reference cases')


if __name__ == '__main__':
    main()
