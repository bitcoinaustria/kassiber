import argparse
import json
from pathlib import Path
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parents[3]))

from embit import bip32
from embit.networks import NETWORKS
from kassiber.core.samourai import (
    SAMOURAI_ACCOUNT_TEMPLATES,
    load_samourai_source_set,
)
from kassiber.errors import AppError

DEST = Path(__file__).resolve().parents[1] / 'tests/vectors/python_samourai_imports.json'
EDGE_DEST = DEST.with_name('python_samourai_codepoints.json')


def source(template=None, network='main', descriptor=False):
    template = template or SAMOURAI_ACCOUNT_TEMPLATES[2]
    coin = 0 if network == 'main' else 1
    path = template.path.format(coin_type=coin)
    master = bip32.HDKey.from_seed(bytes(range(32)))
    key = master.derive(path).to_public()
    key.version = NETWORKS['main' if network == 'main' else 'test']['xpub']
    item = dict(section=template.section, script_type=template.script_type, root_path=path,
                xpub=key.to_base58(), fingerprint=master.my_fingerprint.hex())
    if descriptor:
        for branch, field in [(0, 'descriptor'), (1, 'change_descriptor')]:
            origin = f"[{item['fingerprint']}/{path[2:]}]{item['xpub']}/{branch}/*"
            item[field] = {'p2pkh': f'pkh({origin})', 'p2sh-p2wpkh': f'sh(wpkh({origin}))',
                           'p2wpkh': f'wpkh({origin})'}[template.script_type]
        del item['xpub']
    return item


def inputs():
    cases = []

    def add(name, payload, network='main', gap=None, **kwargs):
        cases.append(dict(name=name, text=json.dumps(payload), network=network,
                          gap_limit=gap, **kwargs))

    for network in ['main', 'test', 'signet', 'regtest']:
        for i, template in enumerate(SAMOURAI_ACCOUNT_TEMPLATES):
            for descriptor in [False, True]:
                add(f'account-{network}-{i}-{descriptor}',
                    {'children' if descriptor else 'xpubs': [source(template, network, descriptor)]}, network)
    base = source()
    desc = source(descriptor=True)
    for value in [None, [], 1, True, 'bad', {}, {'children': []}, {'xpubs': []}]:
        add('shape-' + repr(value), value)
    for field in ['children', 'sources', 'xpubs']:
        for value in [None, '', 0, False, {}, 'bad', 1, [None], [0], ['bad']]:
            add(f'container-{field}-{value!r}', {field: value})
    for field in ['section', 'script_type', 'root_path', 'xpub', 'fingerprint', 'gap_limit']:
        for value in [None, '', ' ', False, True, 0, -1, 12.9, {}, [], 'bad', '\ud800']:
            add(f'xpub-{field}-{value!r}', {'xpubs': [{**base, field: value}]})
        add('xpub-missing-' + field, {'xpubs': [{k: v for k, v in base.items() if k != field}]})
    for field in ['descriptor', 'change_descriptor', 'section', 'script_type', 'root_path', 'label']:
        for value in [None, '', ' ', False, 12, {}, [], 'bad', '\ud800']:
            add(f'descriptor-{field}-{value!r}', {'children': [{**desc, field: value}]})
    for value in [0, False, True, -1, 1, 99, 100, 101, 5000, 5001, '１２３', '1_000',
                  '1.5', 1.9, -1.9, 'NaN', float('nan'), float('inf'), -(10**70), 10**70]:
        add('gap-' + repr(value), {'xpubs': [{**base, 'gap_limit': value}]})
    for value in [0, -1, 1, 99, 101, 5001]:
        add('cli-gap-' + repr(value), {'xpubs': [base]}, gap=value)
        add('override-cli-gap-' + repr(value), {'xpubs': [{**base, 'gap_limit': 123}]}, gap=value)
    for network in ['mainnet', 'bitcoin', 'testnet', 'invalid', ' MAIN ', '']:
        add('network-' + network, {'xpubs': [base]}, network=network)
    for value in ['test', 'mainnet', 'invalid', False, {}, 12, '\ud800']:
        add('declared-network-' + repr(value), {'network': value, 'xpubs': [base]})
    for field in ['minimum_mix_count', 'mix_count', 'target_mix_count', 'pool_denomination_sat',
                  'coordinator_fee_sat', 'miner_fee_sat', 'ricochet_hops']:
        for value in [None, '', -1, 0, True, 2.9, -0.9, '１２３', '1_000', '1.5',
                      float('nan'), float('inf'), 10**70, [], {}, '\ud800']:
            add(f'metadata-{field}-{value!r}', {'xpubs': [{**base, field: value}]})
    enums = {
        'mix_count_confidence': ['minimum', 'exact', 'estimated', 'unknown'],
        'tx0_role': ['deposit', 'premix', 'badbank', 'fee'],
        'whirlpool_event': ['tx0', 'premix_pending', 'first_mix', 'remix', 'mix_to_wallet', 'external_spend'],
        'privacy_event': ['coinjoin', 'payjoin', 'tx0', 'first_mix', 'remix', 'ricochet', 'exit'],
        'exit_kind': ['cold_storage', 'external_spend', 'ricochet', 'toxic_change_spend'],
    }
    for field, values in enums.items():
        for value in [*values, ' ' + values[0].upper() + ' ', 'bad', None, [], {}, '\ud800']:
            add(f'metadata-{field}-{value!r}', {'children': [{**desc, 'samourai': {field: value}}]})
    for value in [None, '', 'AB' * 32, 'ab' * 31, 'gg' * 32, 'ab ' * 21 + 'a',
                  'a b' * 21 + 'a', ['ab' * 32, 'invalid', 'CD' * 32], [], 12, '\ud800']:
        for field in ['round_txid', 'round_txids']:
            add(f'metadata-{field}-{value!r}', {'xpubs': [{**base, field: value}]})
    add('inline-provenance-precedence', {'xpubs': [{**base, 'mix_count': 4, 'samourai': {
        'mix_count': 2, 'privacy_boundary': False, 'role': 'parent', 'xpub': 'discard',
        'participant_inputs': ['discard'], 'group_id': 'discard'}}]})
    add('xpub-label-ignored', {'xpubs': [{**base, 'label': 'ignored'}]})
    add('descriptor-no-change', {'children': [{k: v for k, v in desc.items() if k != 'change_descriptor'}]})
    add('descriptor-origin-mismatch', {'children': [{**desc, 'descriptor': desc['descriptor'].replace("84'/0'/0'", "84'/0'/1'")}]})
    add('descriptor-receive-only', {'children': [{**desc, 'descriptor': desc['descriptor'].replace('/0/*', '/0/0'), 'change_descriptor': None}]})
    add('descriptor-no-origin', {'children': [{**desc, 'descriptor': f"wpkh({base['xpub']}/0/*)", 'change_descriptor': f"wpkh({base['xpub']}/1/*)"}]})
    add('descriptor-multipath', {'children': [{**desc, 'descriptor': desc['descriptor'].replace('/0/*', '/<0;1>/*'), 'change_descriptor': None}]})
    add('descriptor-private', {'children': [{**desc, 'descriptor': 'wpkh(xprv-not-accepted/0/*)'}]})
    add('alias-sources', {'children': [], 'sources': [desc]})
    add('alias-precedence', {'children': [desc], 'sources': ['ignored']})
    add('all-accounts', {'children': [source(t, descriptor=True) for t in SAMOURAI_ACCOUNT_TEMPLATES]})
    add('mixed', {'children': [desc], 'xpubs': [source(SAMOURAI_ACCOUNT_TEMPLATES[6])]})
    add('duplicate-labels', {'children': [desc, desc]})
    add('distinct-labels', {'children': [desc, {**desc, 'label': 'Other'}]})
    add('late-bad-source', {'xpubs': [base, {**base, 'xpub': 'bad'}]})
    add('safe-codepoints', {'children': [{**desc, 'label': ' Ελληνικά\u001c receipt '}]})
    add('metadata-descriptor-order', {'children': [{**desc, 'descriptor': 'bad', 'script_type': 'p2wpkh', 'mix_count': float('inf')}]})
    add('metadata-xpub-order', {'xpubs': [{**base, 'xpub': 'bad', 'mix_count': float('inf')}]})
    for network in ['main', 'test']:
        key_source = source(network=network)
        key = bip32.HDKey.from_base58(key_source['xpub'])
        for prefix in ['ypub', 'zpub', 'Ypub', 'Zpub']:
            key.version = NETWORKS[network][prefix]
            add('key-family-' + key.to_base58()[:4], {'xpubs': [{**key_source, 'xpub': key.to_base58()}]}, network)
    postmix = source(SAMOURAI_ACCOUNT_TEMPLATES[6], descriptor=True)
    add('declared-script-mismatch', {'children': [{**postmix,
        'descriptor': postmix['descriptor'].replace('wpkh(', 'pkh('),
        'change_descriptor': postmix['change_descriptor'].replace('wpkh(', 'pkh(')}]})
    add('partial-hex-pairs', {'xpubs': [{**base, 'round_txid': 'a b' * 20 + 'aaaa'}]})
    add('network-refusal-order', {'network': 'invalid-declared', 'xpubs': [base]}, network='invalid-requested')
    add('network-declared-default', {'network': 'test', 'xpubs': [source(network='test')]}, network='')
    for name, content in [('empty', ''), ('invalid-json', '{'), ('bom', '\ufeff{}')]:
        cases.append(dict(name=name, text=content, network='main', gap_limit=None))
    cases.append(dict(name='bad-utf8', bytes=[255], network='main', gap_limit=None))
    return cases


def edge_inputs():
    base = source()
    desc = source(descriptor=True)
    cases = []
    for field, value in [
        ('xpub', base['xpub'] + '\ud800'), ('fingerprint', base['fingerprint'] + '\ud800'),
        ('section', 'depo\ud800sit'), ('section', '\ud800deposit'), ('section', 'deposit\ud800'),
        ('script_type', 'p2wpkh\ud800'), ('round_txid', 'ab' * 32 + '\ud800'),
        ('round_txids', ['ab' * 32, 'cd' * 32 + '\ud800']),
        ('mix_count_confidence', 'exact\ud800'), ('gap_limit', '100\ud800'),
        ('root_path', base['root_path'] + '\ud800'),
    ]:
        cases.append(dict(name=f'xpub-codepoints-{field}-{len(cases)}',
            text=json.dumps({'xpubs': [{**base, field: value}]}), network='main', gap_limit=None))
    for field, value in [
        ('descriptor', desc['descriptor'].replace('wpkh', 'w\ud800pkh')),
        ('descriptor', desc['descriptor'] + '\ud800'),
        ('change_descriptor', desc['change_descriptor'] + '\ud800'),
        ('label', 'receipt\ud800'), ('label', '\ud800\ud801receipt'),
    ]:
        cases.append(dict(name=f'descriptor-codepoints-{field}-{len(cases)}',
            text=json.dumps({'children': [{**desc, field: value}]}), network='main', gap_limit=None))
    cases.append(dict(name='private-descriptor-codepoint-order', text=json.dumps({'children': [{**desc,
        'descriptor': 'wpkh(xprv-should-never-load/0/*)\ud800'}]}), network='main', gap_limit=None))
    cases.append(dict(name='private-xpub-codepoint-order', text=json.dumps({'xpubs': [{**base,
        'xpub': 'xprv-should-never-load', 'fingerprint': base['fingerprint'] + '\ud800'}]}), network='main', gap_limit=None))
    duplicate = {**desc, 'label': 'receipt\ud800'}
    cases.append(dict(name='duplicate-label-codepoint-order', text=json.dumps({'children': [duplicate, duplicate]}),
        network='main', gap_limit=None))
    cases.append(dict(name='label-binding-codepoint-order', text=json.dumps({'children': [
        {**desc, 'label': 'zzzz\ud800'}, {**desc, 'label': 'a\ud800'}]}), network='main', gap_limit=None))
    cases.append(dict(name='round-reference-whitespace', text=json.dumps({'xpubs': [
        {**base, 'round_txid': 'ab ' * 20 + 'abcd'}]}), network='main', gap_limit=None))
    for network, family in [('test', 'ypub'), ('main', 'upub')]:
        item = source(network=network)
        key = bip32.HDKey.from_base58(item['xpub'])
        key.version = NETWORKS['main' if family == 'ypub' else 'test']['ypub']
        item['xpub'] = key.to_base58()
        cases.append(dict(name='key-family-mismatched-' + family, text=json.dumps({'xpubs': [item]}),
            network=network, gap_limit=None))
    for name, text in [('json-integer-limit', '{"ignored":' + '9' * 4301 + '}'),
                       ('json-array-depth', '[' * 1200 + '0' + ']' * 1200),
                       ('json-object-depth', '{"x":' * 1200 + '0' + '}' * 1200)]:
        cases.append(dict(name=name, text=text, network='main', gap_limit=None))
    return cases


def vectors(inputs_fn=inputs):
    result = []
    with tempfile.TemporaryDirectory(prefix='samourai-vectors-') as root:
        path = Path(root) / 'source.json'
        for case in inputs_fn():
            path.write_bytes(bytes(case['bytes']) if 'bytes' in case else case['text'].encode())
            try:
                sources = load_samourai_source_set(str(path), network=case['network'], gap_limit=case['gap_limit'])
                case['sources_json'] = json.dumps(sources, sort_keys=True)
                if case['name'].startswith('key-family-'):
                    from crates.wallets.scripts.generate_vectors import plan_result
                    case['plans_json'] = json.dumps([plan_result(item['config'], end=2) for item in sources], sort_keys=True)
            except Exception as error:
                case['error_json'] = json.dumps(dict(code=error.code if isinstance(error, AppError) else 'internal_error',
                    message=str(error), hint=error.hint if isinstance(error, AppError) else None,
                    details=error.details if isinstance(error, AppError) else None), sort_keys=True)
            result.append(case)
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    for path, input_fn in [(DEST, inputs), (EDGE_DEST, edge_inputs)]:
        content = json.dumps(vectors(input_fn), indent=2, ensure_ascii=True) + '\n'
        if args.check:
            if path.read_text() != content:
                raise SystemExit('Samourai vectors drifted')
        else:
            path.write_text(content)


if __name__ == '__main__':
    main()
