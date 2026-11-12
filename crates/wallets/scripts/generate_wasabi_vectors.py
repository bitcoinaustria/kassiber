import argparse
import json
from pathlib import Path
import sqlite3
import tempfile
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[3]))

from kassiber.core.imports import normalize_import_record
from kassiber.errors import AppError
from kassiber.importers import load_import_records, load_wasabi_bundle
from kassiber.msat import btc_to_msat
from kassiber.fingerprints import make_transaction_fingerprint
from crates.wallets.scripts.generate_import_vectors import clean

DEST = Path(__file__).resolve().parents[1] / 'tests/vectors/python_wasabi_imports.json'
EDGE_DEST = DEST.with_name('python_wasabi_codepoints.json')
HISTORY = {'tx': 'ab' * 32, 'datetime': '2025-01-02T03:04:05Z', 'amount': 12345, 'height': 1, 'label': 'receipt', 'islikelycoinjoin': False}
COIN = {'txid': 'ab' * 32, 'vout': 0, 'amount': 12345, 'confirmed': True, 'confirmations': 2, 'keyPath': "m/84'/0'/0'/1/5", 'label': 'change', 'anonymityScore': 12, 'anonHistory': [{'round': 'one', 'X_pub': 'secret', 'nested': {'password': 'secret'}}]}


def inputs():
    cases = {'empty': '', 'bad-utf8': b'\xff', 'bom': '\ufeff{}', 'invalid-json': '{'}
    def add(name, value):
        cases[name] = json.dumps(value)
    for v in [None, 1, True, 'bad', {}, [], [None, 1, 'ignored']]:
        add('shape-' + repr(v), v)
    add('history', {'gethistory': [HISTORY]})
    add('bare', [HISTORY])
    add('full', {'gethistory': [HISTORY], 'listcoins': [COIN], 'getwalletinfo': {'walletName': 'Source', 'AnonScoreTarget': 50}, 'wallet_json': {'EncryptedSecret': 'secret', 'SegWitAccountKeyPath': "m/84'/0'/7'", 'accounts': [{'name': 'account', 'keyPath': "m/84'/0'/7'"}]}, 'listkeys': [{'keyState': 'Clean'}, {'keyState': 'Clean'}, {'keyState': 'Locked'}, {}], 'listpaymentsincoinjoin': [{'id': 'payment', 'amount': '12.5', 'states': [{'status': 'Pending', 'roundId': 'round', 'paymentId': 'p', 'privateKey': 'secret'}, None]}]})
    for name in ['gethistory', 'get-history', 'get_history', 'get.history', 'GETHISTORY', ' gethistory ']:
        for container in [None, 'rpc', 'responses', 'exports', 'wasabi']:
            v = {name: {'result': [HISTORY]}}
            add('section-' + name + '-' + str(container), {container: v} if container else v)
    for name in ['gethistory', 'listcoins', 'listunspentcoins', 'listkeys', 'listpaymentsincoinjoin']:
        for v in [None, [], {}, 1, 'bad', {'items': []}, {'records': [HISTORY] if name == 'gethistory' else []}]:
            add('list-' + name + '-' + repr(v), {name: v})
    for field in HISTORY:
        for v in [None, '', ' ', 0, False, [], {}, 'other']:
            add('history-' + field + '-' + repr(v), {'gethistory': [{**HISTORY, field: v}]})
        add('missing-' + field, {'gethistory': [{k: v for k, v in HISTORY.items() if k != field}]})
    for v in [0, -1, 1, True, False, 1.0, 1.5, '1', '1.0', '1e3', '1.5e-8', '2.5e-8', '0.000000015', '-0.000000025', 'NaN', 'Infinity', 'bad', '１２３', '١_٢', 1000000000000000000000000000000]:
        add('amount-' + repr(v), {'gethistory': [{**HISTORY, 'amount': v}]})
    for v in [True, False, 'TRUE', ' yes ', 'on', '1', '0', 'off', 'false', 'unknown', 1, 0]:
        add('coinjoin-' + repr(v), {'gethistory': [{**HISTORY, 'amount': 0, 'islikelycoinjoin': v}]})
    for v in ['-1.9', 1.9, '1e3', '1.0e3', True, 'NaN', '１２３', '1_000']:
        add('height-' + repr(v), {'gethistory': [{**HISTORY, 'height': v}]})
    add('aliases', {'transactions': [{'transactionId': ' CD ' * 2, 'amountSats': '-1', 'time': '2025-01-01', 'labels': ['one', 'two'], 'isLikelyCoinjoin': True, 'blockHeight': 3}]})
    add('precedence', {'gethistory': None, 'rpc': {'gethistory': [HISTORY]}})
    add('precedence-error', {'gethistory': 1, 'rpc': {'gethistory': [HISTORY]}})
    add('duplicate', [HISTORY, HISTORY])
    add('partial-failure', [HISTORY, {**HISTORY, 'datetime': 'invalid'}])
    for v in [None, [], {}, [COIN], [COIN, {**COIN, 'label': 'last'}], [{**COIN, 'txid': 'invalid'}], [{**COIN, 'vout': -1}], [{**COIN, 'amount': -1}], [{**COIN, 'spentBy': 'cd'*32}]]:
        add('coins-' + repr(v), {'gethistory': [HISTORY], 'listcoins': v})
    add('coin-distinct-outpoints', {'listcoins': [COIN, {**COIN, 'vout': 1}]})
    add('coin-precedence', {'listcoins': [COIN], 'listunspentcoins': [{**COIN, 'amount': 5, 'label': 'last'}]})
    for path in [None, '', 'm', 'm/1', 'm/1/2', "m/84'/0'/0'/0/4", '0/3', '2h/9H', 'bad/1', '-1/-2', '１２/３']:
        add('path-' + repr(path), {'listcoins': [{**COIN, 'keyPath': path}]})
    for field in ['vout', 'amount', 'confirmations', 'height', 'anonymityScore', 'spentBy', 'excludedFromCoinjoin', 'keyState']:
        for v in [None, '', 'bad', -1, True, 1.9]:
            add('coin-' + field + '-' + repr(v), {'listcoins': [{**COIN, field: v}]})
    for field in ['walletName', 'State', 'AnonScoreTarget', 'AutoCoinJoin', 'RedCoinIsolation', 'IsWatchOnly', 'IsHardwareWallet', 'MinGapLimit', 'CoinjoinStatus', 'SegWitAccountKeyPath', 'accounts']:
        for v in [None, '', 0, False, '1.9', 'bad', []]:
            add('metadata-' + field + '-' + repr(v), {'getwalletinfo': {field: v}})
    add('metadata-precedence', {'wallet_json': {'walletName': 'old', 'AnonScoreTarget': 1}, 'getwalletinfo': {'walletName': None, 'AnonScoreTarget': 2}})
    add('metadata-folded', {'wallet_json': {'wallet-name': 'first', 'wallet_name': 'last', 'MIN_GAP_LIMIT': '12'}})
    add('account-digit-refusal', {'wallet_json': {'accounts': [{'keyPath': "m/²'/0'/0'"}]}})
    add('account-nondigit', {'wallet_json': {'accounts': [{'keyPath': "m/½'/0'/0'"}]}})
    add('last-alias-false', {'history': [{'transactionid': 0, 'time': '2025-01-01', 'amountSats': 0, 'labels': []}]})
    add('payments', {'listpaymentsincoinjoin': [{}, {'amount': '1e3'}, {'state': {'status': 'ok'}}]})
    add('surrogate-section', {'get\ud800history': [HISTORY]})
    add('history-secret-fields', {'gethistory': [{**HISTORY, 'privateKey': 'secret', 'xpub': 'secret', 'raw': {'password': 'secret'}}]})
    for key in ['tx', 'label']:
        add('surrogate-' + key, {'gethistory': [{**HISTORY, key: '\ud800'}]})
    add('coin-vout-refusal-order', {'listcoins': [{**COIN, 'vout': 'bad', 'amount': 'bad', 'confirmations': 'bad'}]})
    add('coin-amount-refusal-order', {'listcoins': [{**COIN, 'amount': 'bad', 'confirmations': 'bad'}]})
    add('account-score-refusal-order', {'wallet_json': {'accounts': [{'keyPath': "m/²'/0'/0'"}], 'AnonScoreTarget': 'bad'}})
    add('account-gap-refusal-order', {'wallet_json': {'accounts': [{'keyPath': "m/²'/0'/0'"}], 'MinGapLimit': 'bad'}})
    add('metadata-large-float', {'wallet_json': {'MinGapLimit': 1.2345678901234567e20}})
    return cases


def vectors():
    result = []
    with tempfile.TemporaryDirectory(prefix='wasabi-vectors-') as root:
        file = Path(root) / 'input.csv'
        for name, content in inputs().items():
            file.write_bytes(content.encode() if isinstance(content, str) else content)
            case = {'name': 'wasabi:' + name, 'format': 'wasabi_bundle', **({'text': content} if isinstance(content, str) else {'bytes': list(content)})}
            try:
                bundle = load_wasabi_bundle(str(file))
                case["bundle"] = {k: v for k, v in bundle.items() if k != "records"}
                records = []
                for row in load_import_records(str(file), 'wasabi_bundle'):
                    record = normalize_import_record(row, 'file:wasabi_bundle')
                    fingerprint = make_transaction_fingerprint('wallet', record['external_id'], record['occurred_at'], record['direction'], record['asset'], record['amount'], record['fee'])
                    record['amount'] = btc_to_msat(record['amount'])
                    record['fee'] = btc_to_msat(record['fee'])
                    if any(not -(1 << 63) <= record[k] < (1 << 63) for k in ('amount', 'fee')):
                        raise OverflowError('Python int too large to convert to SQLite INTEGER')
                    for key in ('exchange_evidence_match_by_economics', 'exchange_evidence_match_time_tolerance_seconds', 'match_existing_ignore_fee'):
                        record.pop(key)
                    record['amount_includes_fee'] = int(record['amount_includes_fee'])
                    for value in record.values():
                        if isinstance(value, str):
                            value.encode('utf-8')
                    records.append({'fields': record, 'fingerprint': fingerprint})
                case['records'] = clean(records)
            except Exception as exc:
                case['error'] = {'code': exc.code if isinstance(exc, AppError) else 'internal_error', 'message': str(exc), 'hint': exc.hint if isinstance(exc, AppError) else None}
            result.append(case)
    return result


def codepoint_vectors():
    cases = {
        'amount-surrogate': {'gethistory': [{**HISTORY, 'amount': '1\ud800'}]},
        'height-surrogate': {'gethistory': [{**HISTORY, 'height': '1\ud800'}]},
        'vout-surrogate': {'listcoins': [{**COIN, 'vout': '1\ud800'}]},
        'coinjoin-surrogate': {'gethistory': [{**HISTORY, 'islikelycoinjoin': 'true\ud800'}]},
        'key-state-surrogate': {'listkeys': [{'keyState': '\ud800'}, {'keyState': '\ud800'}, {'keyState': ''}]},
        'path-tail-surrogate': {'listcoins': [{**COIN, 'keyPath': '0\ud800/3'}]},
        'path-hint-surrogate': {'wallet_json': {'SegWitAccountKeyPath': "m/84\ud800'/0'/7'"}},
        'account-hint-surrogate': {'wallet_json': {'accounts': [{'keyPath': "m/84\ud800'/0'/7'"}]}},
        'coin-txid-surrogate-case': {'listcoins': [{**COIN, 'txid': 'AB\ud800ΟΣ'}]},
        'coin-txid-surrogate-hex': {'listcoins': [{**COIN, 'txid': COIN['txid'] + '\ud800'}]},
        'coin-address-surrogate': {'listcoins': [{**COIN, 'address': 'addr\ud800'}]},
        'coin-label-surrogate': {'listcoins': [{**COIN, 'label': 'label\ud800'}]},
        'coin-spent-by-surrogate': {'listcoins': [{**COIN, 'spentBy': 'cd' * 32 + '\ud800'}]},
        'coin-key-state-surrogate': {'listcoins': [{**COIN, 'keyState': 'state\ud800'}]},
    }
    result = []
    with tempfile.TemporaryDirectory(prefix='wasabi-codepoints-') as root:
        file = Path(root) / 'input.json'
        for name, value in cases.items():
            content = json.dumps(value)
            file.write_text(content)
            case = {'name': name, 'text': content}
            try:
                case['bundle_json'] = json.dumps(load_wasabi_bundle(str(file)), default=str, sort_keys=True)
            except AppError as error:
                case['error_json'] = json.dumps({'code': error.code, 'message': str(error), 'hint': error.hint}, sort_keys=True)
            from kassiber.cli import handlers
            from kassiber.core import imports
            with sqlite3.connect(':memory:') as conn:
                conn.row_factory = sqlite3.Row
                schema = Path(__file__).resolve().parents[2] / 'store/fixtures/creation.json'
                for statement in json.loads(schema.read_text())['statements']:
                    conn.execute(statement)
                conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('ws','Workspace','2025-01-01')")
                conn.execute("INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','ws','Book','2025-01-01')")
                conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('w','ws','p','Source','wasabi','{}','2025-01-01')")
                profile = conn.execute('SELECT * FROM profiles').fetchone()
                wallet = conn.execute('SELECT * FROM wallets').fetchone()
                try:
                    imports.import_file_into_wallet(conn, profile, wallet, str(file), 'wasabi_bundle', handlers._import_coordinator_hooks())
                except Exception as error:
                    case['coordinator_error_json'] = json.dumps(dict(code=error.code if isinstance(error, AppError) else 'internal_error', message=str(error), hint=error.hint if isinstance(error, AppError) else None), sort_keys=True)
            result.append(case)
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    for dest, cases in ((DEST, vectors()), (EDGE_DEST, codepoint_vectors())):
        value = json.dumps(cases, ensure_ascii=True, indent=2) + '\n'
        if args.check:
            if dest.read_text() != value:
                raise SystemExit(f'Wasabi vectors drifted: {dest.name}')
        else:
            dest.write_text(value)
