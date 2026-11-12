import argparse
import copy
from dataclasses import asdict
import json
from pathlib import Path
import sys
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core.lightning import cln
from kassiber.core.lightning.types import snapshot_to_dict, snapshot_to_dict_for_ai
from kassiber.errors import AppError

OUTPUT = ROOT / 'crates/chain/fixtures/python_cln.json'
SECRET = 'DISCARDED_CLN_SECRET'
# Routing windows end at `now_iso()`; freeze it so the vectors are stable.
NOW_ISO = '2025-01-10T00:00:00Z'
NOW = 1736467200
HOUR, DAY = 3600, 86400
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
            {'in_channel': '2x3x4', 'out_channel': '1x2x3', 'fee_msat': '1501msat', 'status': 'settled', 'received_time': NOW - HOUR, 'erring_node': SECRET},
            {'in_channel': '2x3x4', 'out_channel': '1x2x3', 'fee_msat': 1501, 'status': 'settled', 'resolved_time': NOW - 2 * DAY, 'route': [SECRET]},
        ]},
        'listpays': {'pays': [
            {'amount_msat': 10000, 'amount_sent_msat': 11999, 'status': 'complete', 'completed_at': NOW - HOUR, 'payment_preimage': SECRET, 'bolt11': SECRET},
            {'amount_msat': 'invalid-ignored-rebalance', 'status': 'paid', 'created_at': NOW - HOUR, 'rebalance': True},
        ]},
        'listinvoices': {'invoices': [{'payment_preimage': SECRET, 'payment_secret': SECRET, 'bolt11': SECRET, 'description': SECRET}]},
        'bkpr-listincome': {'income_events': [{'tag': 'rebalance_fee', 'debit_msat': 1999, 'timestamp': NOW - HOUR}, {'tag': 'onchain_fee', 'debit_msat': 1001, 'timestamp': NOW - 400 * DAY}]},
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
    # Node-snapshot fields: info, timestamps, initiator, invoice/pay tallies, forward ids and aliases.
    data = base()
    data['getinfo'].update(id='  operator-node  ', version=' v24.02 ', network='')
    yield 'snapshot-info-identity-version', data, 30, []
    for index, stamp in enumerate([None, '', 0, '0', 0.0, False, True, -1, 1.9, '1.9', ' 7 ', '1_000', 'nan', 'inf', '-inf', [], {},
                                    253402300799, 253402300800, 6 * 10**16, 7 * 10**16, -62135596800, -62135596801, -7 * 10**16,
                                    2**63, -2**63, 2**64, 10**400, '1e400', '\ud800']):
        data = base()
        data['listpeerchannels']['channels'][0].update(opened_at=stamp, closed_at=stamp)
        data['listforwards']['forwards'][0].update(resolved_time=stamp, received_time=1)
        yield f'snapshot-timestamp-{index}', data, 30, []
    for index, (opener, funder) in enumerate([('local', None), ('remote', 'LOCAL'), ('LOCAL', 'local'), (None, None), (1, 'LOCAL ')]):
        data = base()
        data['listpeerchannels']['channels'][0].update(opener=opener, funder=funder)
        yield f'snapshot-initiator-{index}', data, 30, []
    data = base()
    data['listinvoices']['invoices'] = [{'status': s} for s in ['paid', 'PAID', ' expired ', 'unpaid', None, 1]] + ['ignored']
    data['listpays']['pays'] = [{'status': s, 'amount_msat': 1, 'amount_sent_msat': 2} for s in ['complete', 'Completed', 'paid', 'failed', 'ERROR', 'pending', None]]
    yield 'snapshot-invoice-and-pay-tallies', data, 30, []
    data = base()
    data['listpeerchannels']['channels'].append({'channel_id': '9', 'peer_id': '2x3x4', 'peer_alias': 'Channel-named peer', 'state': 'CHANNELD_NORMAL'})
    data['listforwards']['forwards'] = [
        {'in_channel': ' 2x3x4 ', 'out_channel': '1x2x3', 'fee_msat': 3, 'in_msat': '5000msat', 'out_msat': 4997.0, 'status': s, 'received_time': 1.5, 'resolved_time': None}
        for s in ['settled', 'offered', 'pending', 'failed', 'local_failed', None]
    ] + [{'status': 'settled', 'received_time': {'b': 1, 'a': [2.5, None, '\u00e9']}}]
    yield 'snapshot-forward-ids-statuses-and-channel-keyed-aliases', data, 30, []
    for connected in [False, True, None, 0]:
        data = base()
        data['listpeerchannels']['channels'][0].update(state='CHANNELD_NORMAL', peer_connected=connected)
        yield f'snapshot-normal-connected-{connected}', data, 30, []
    data = base()
    data['listpeerchannels']['channels'] = [
        {'channel_id': str(i), 'state': state, 'total_msat': 1000 * (i + 1), 'to_us_msat': 400 * (i + 1)}
        for i, state in enumerate(['channeld_normal', 'ONCHAIND_THEIR_UNILATERAL', 'closingd_complete', 'breach_seen', 'dualopend_awaiting_lockin', 'closingd_sigexchange'])
    ]
    data['listpeerchannels']['channels'][1]['funding_msat'] = []
    yield 'snapshot-open-closed-split-and-totals', data, 30, []
    yield from window_cases()


def window_cases():
    # Round 8: the requested window bounds every routing fact; an undated
    # relevant fact or a failed routing resource leaves the window unknown.
    stamps = [None, '', True, False, 0, NOW, NOW + 1, NOW - DAY, NOW - DAY - 1, NOW - 365 * DAY, NOW - 365 * DAY - 1,
              float(NOW - HOUR), str(NOW - HOUR), f' {NOW - HOUR} ', f'{NOW - HOUR}.5', f'{NOW - HOUR:_}', '١٧٣٦٤٦٣٦٠٠',
              f'{NOW - HOUR}e0', 'NaN', 'Infinity', '-Infinity', 'bad', [], {}, '\ud800', 1e300, -1e300, 2**80]
    for slot in ['forward-resolved', 'forward-received', 'pay-completed', 'pay-created', 'income']:
        for index, stamp in enumerate(stamps):
            for window in [1, 365]:
                data = base()
                forward, pay, event = data['listforwards']['forwards'][0], data['listpays']['pays'][0], data['bkpr-listincome']['income_events'][0]
                if slot == 'forward-resolved': forward['resolved_time'] = stamp
                elif slot == 'forward-received': forward['received_time'] = stamp
                elif slot == 'pay-completed': pay['completed_at'] = stamp
                elif slot == 'pay-created': pay.pop('completed_at'); pay['created_at'] = stamp
                else: event['timestamp'] = stamp
                yield f'window-{slot}-{index}-{window}', data, window, []
    data = base()
    data['listforwards']['forwards'][1].pop('resolved_time')
    yield 'window-undated-settled-forward', data, 30, []
    data = base()
    data['listforwards']['forwards'].append({'status': 'failed', 'fee_msat': 'bad', 'out_channel': '1x2x3'})
    yield 'window-undated-failed-forward-ignored', data, 30, []
    data = base()
    data['listpays']['pays'].append({'status': 'pending', 'amount_msat': 1})
    data['bkpr-listincome']['income_events'].append({'tag': 'invoice', 'credit_msat': 1})
    yield 'window-undated-irrelevant-rows-ignored', data, 30, []
    data = base()
    data['listforwards']['forwards'][0]['fee_msat'] = 'bad'
    data['listforwards']['forwards'][0]['received_time'] = NOW - 2 * DAY
    yield 'window-excluded-bad-fee-still-validated-by-forward-rows', data, 1, []
    data = base()
    data['listpays']['pays'][0]['amount_sent_msat'] = 'bad'
    data['listpays']['pays'][0]['completed_at'] = NOW - 2 * DAY
    yield 'window-excluded-bad-payment-not-summed', data, 1, []
    for method in ['listpeerchannels', 'listforwards', 'listpays', 'bkpr-listincome', 'listfunds', 'listinvoices']:
        yield f'window-failure-{method}', base(), 30, [method]
        data = base()
        data['listforwards']['forwards'][0]['fee_msat'] = 'bad'
        yield f'window-failure-{method}-skips-summary', data, 30, [method]
    yield 'window-empty-known-zero', {k: {} for k in METHODS}, 365, []
    data = base()
    data['listpeerchannels']['channels'][0]['funding'] = {'txid': 'ab' * 32, 'output': 0, 'outnum': 1}
    data['listpeerchannels']['channels'][0].pop('channel_id')
    data['listpeerchannels']['channels'][0].pop('short_channel_id')
    yield 'funding-output-zero-kept', data, 30, []


def evaluate(name, data, window, failures):
    calls = []
    def caller(method, args):
        calls.append({'method': method, 'args': args})
        if method in failures:
            raise AppError('synthetic failure', code='sync_error')
        return copy.deepcopy(data[method])
    result = {'name': name, 'data': data, 'window': window, 'now': NOW, 'failures': failures}
    try:
        raw = cln.fetch_core_lightning_snapshot({}, rpc_call=caller)
        with patch.object(cln, 'now_iso', lambda: NOW_ISO):
            snapshot = cln.build_node_snapshot(raw, window_days=window)
        result['expected'] = {
            'channels': [{'id': c.id, 'peer_alias': c.peer_alias, 'capacity_sat': c.capacity_sat, 'earned_routing_sat': c.earned_routing_sat} for c in snapshot.channels],
            'routing': asdict(snapshot.routing) if snapshot.routing else None,
        }
        result['snapshot'] = snapshot_to_dict(snapshot)
        result['ai_snapshot'] = snapshot_to_dict_for_ai(snapshot)
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
