"""Freeze unchanged Python PSBT analyze/compare/entropy dispatch envelopes."""
from __future__ import annotations

import base64
from copy import deepcopy
from hashlib import new as hash_new, sha256
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).parent))
from embit.ec import PrivateKey
from embit.psbt import PSBT
from embit.script import Script, Witness
from embit.transaction import Transaction, TransactionInput, TransactionOutput
from generate_analysis_durable import capture, database
from generate_psbt_vectors import encode, sample, v2
from kassiber.core import chain_analysis_api as api
from kassiber.envelope import build_envelope, build_error_envelope
from kassiber.errors import AppError

DEST = ROOT / 'crates/wallets/tests/data/chain_analysis_psbt_reads.json'
FIXTURES = ROOT / 'tests/fixtures/chain_analysis'
PAYJOIN = json.loads((FIXTURES / 'payjoin_bip78_vectors.json').read_text())
BIP = json.loads((FIXTURES / 'psbt_bip_vectors.json').read_text())
SECRET = PrivateKey((1).to_bytes(32, 'big'))
KEY = SECRET.get_public_key().sec()
SCRIPT = b'\x00\x14' + hash_new('ripemd160', sha256(KEY).digest()).digest()
SIG = bytes.fromhex('30440220' + '12' * 32 + '0220' + '34' * 32 + '01')
N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141


def hash160(data):
    return hash_new('ripemd160', sha256(data).digest()).digest()


def evaluate(case, conn=None):
    own = conn is None
    conn = database(case.get('sql', '')) if own else conn
    result = []
    try:
        for action in case['actions']:
            kind = 'ui.chain_analysis.' + action['operation']
            args = dict(action.get('args', {}))
            for key, raw in action.get('files', {}).items():
                args[key] = bytes.fromhex(raw)
            try:
                value = build_envelope(kind, api.dispatch(conn, kind, args))
            except AppError as error:
                value = build_error_envelope(error.code, str(error), details=error.details, hint=error.hint, retryable=error.retryable)
            result.append({'wire': json.dumps(value, separators=(',', ':')), 'result': value, 'state': capture(conn), 'in_transaction': conn.in_transaction})
        return result
    finally:
        if own:
            conn.close()


def b64(psbt):
    return psbt.to_base64() if isinstance(psbt, PSBT) else base64.b64encode(psbt).decode()


def der(r, s):
    def integer(n):
        raw = n.to_bytes(33, 'big').lstrip(b'\0')
        return raw if raw[0] < 0x80 else b'\0' + raw
    body = b'\x02' + bytes([len(integer(r))]) + integer(r) + b'\x02' + bytes([len(integer(s))]) + integer(s)
    return b'\x30' + bytes([len(body)]) + body


def ordered_pair(*, duplicate_outputs=False):
    scripts = [Script(b'\x00\x14' + bytes([byte]) * 20) for byte in (31, 32, 33)]
    if duplicate_outputs:
        scripts[1] = scripts[0]
    tx = Transaction(vin=[TransactionInput(bytes([byte]) * 32, 0, sequence=0xFFFFFFFD) for byte in (11, 12)],
                     vout=[TransactionOutput(amount, script) for amount, script in zip((49_000, 50_000, 100_000), scripts)])
    original = PSBT(tx)
    for row in original.inputs:
        row.witness_utxo = TransactionOutput(100_000, Script(SCRIPT))
        row.final_scriptwitness = Witness([SIG, KEY])
    proposal = PSBT.from_base64(original.to_base64())
    for row in proposal.inputs:
        row.final_scriptwitness = None
    return original, proposal


def receiver(*, flag=1, template='p2wpkh', mutate=None, key=KEY, value=100_000, single_index=1, adjust=None, heterogeneous=False):
    """Original two sender inputs plus one signed receiver input at index 1."""
    original, _ = ordered_pair()
    if heterogeneous:
        original.inputs[1].final_scriptwitness = Witness([SIG, KEY, b'\x00'])
    tx = deepcopy(original.tx)
    tx.vin.insert(1, TransactionInput(bytes([13]) * 32, 0, sequence=0xFFFFFFFD))
    tx.vout.insert(1, TransactionOutput(100_000, Script(b'\x00\x14' + b'\x44' * 20)))
    if single_index != 1:
        tx.vout = tx.vout[:single_index]
    for index, delta in (adjust or {}).items():
        tx.vout[index].value += delta
    proposal = PSBT(tx)
    for row in proposal.inputs:
        row.witness_utxo = TransactionOutput(100_000, Script(SCRIPT))
    keyhash = hash160(key)
    redeem = b'\x00\x14' + keyhash
    if template == 'p2sh':
        proposal.inputs[1].witness_utxo = TransactionOutput(value, Script(b'\xa9\x14' + hash160(redeem) + b'\x87'))
    else:
        proposal.inputs[1].witness_utxo = TransactionOutput(value, Script(redeem))
    digest = tx.sighash_segwit(1, Script(b'\x76\xa9\x14' + keyhash + b'\x88\xac'), value, sighash=flag)
    signature = SECRET.sign(digest)
    raw = signature.serialize()
    if mutate == 'high_s':
        r = int.from_bytes(raw[4:4 + raw[3]], 'big')
        s = int.from_bytes(raw[6 + raw[3]:], 'big')
        raw = der(r, N - s)
    stack = [raw + bytes([flag]), key]
    if mutate == 'flip':
        stack[0] = bytearray(stack[0])
        stack[0][10] ^= 1
        stack[0] = bytes(stack[0])
    if mutate == 'three_items':
        stack.append(b'')
    if mutate == 'wrong_key_hash':
        stack[1] = KEY[:1] + bytes([KEY[1] ^ 1]) + KEY[2:]
    if mutate == 'bad_flag':
        stack[0] = raw + b'\x04'
    proposal.inputs[1].final_scriptwitness = Witness(stack)
    if template == 'p2sh':
        proposal.inputs[1].final_scriptsig = Script(bytes([len(redeem)]) + redeem)
        if mutate == 'bad_redeem':
            proposal.inputs[1].final_scriptsig = Script(bytes([len(redeem)]) + redeem[:-1] + b'\x00')
        if mutate == 'two_pushes':
            proposal.inputs[1].final_scriptsig = Script(b'\x01\x01' + bytes([len(redeem)]) + redeem)
    elif mutate == 'scriptsig':
        proposal.inputs[1].final_scriptsig = Script(b'\x01\x01')
    if mutate == 'no_witness':
        proposal.inputs[1].final_scriptwitness = None
        proposal.inputs[1].final_scriptsig = Script(b'\x01\x01')
    return original, proposal


def vectors():
    cases = []

    def add(name, actions, sql=''):
        case = {'name': name, 'sql': sql, 'actions': actions}
        case['expected'] = evaluate(case)
        cases.append(case)

    def act(operation, args=None, files=None):
        action = {'operation': operation, 'args': {} if args is None else args}
        if files:
            action['files'] = {key: value.hex() for key, value in files.items()}
        return action

    base = encode(*sample())
    text = base64.b64encode(base).decode()
    signed = PSBT.parse(base)
    signed.inputs[0].final_scriptwitness = Witness([SIG, KEY])
    prev = Transaction(vin=[TransactionInput(b'\x99' * 32, 0)], vout=[TransactionOutput(100000, Script(SCRIPT))])
    second = PSBT(Transaction(vin=[TransactionInput(prev.txid(), 0, sequence=0xFFFFFFFD), TransactionInput(b'\x98' * 32, 1, sequence=0xFFFFFFFF)],
                              vout=[TransactionOutput(99000, Script(SCRIPT)), TransactionOutput(4000, Script(b'\x6a\x01\x01'))], locktime=100))
    second.inputs[0].non_witness_utxo = prev
    second.inputs[0].final_scriptwitness = Witness([SIG, KEY])
    second.inputs[1].witness_utxo = TransactionOutput(5000, Script(SCRIPT))
    second.inputs[1].partial_sigs = {SECRET.get_public_key(): SIG}
    mixed = second.to_base64()
    payloads = {
        'base64': text, 'hex': base.hex(), 'spaced': ' ' + text[:10] + '\n' + text[10:] + '\t',
        'v2': b64(encode(*v2())), 'signed': signed.to_base64(), 'mixed-signatures': mixed,
        'payjoin-original': PAYJOIN['Original PSBT'], 'payjoin-proposal': PAYJOIN['payjoin proposal'],
        'payjoin-filled': PAYJOIN["payjoin proposal filled with sender's information"],
    }
    for name, value in payloads.items():
        for network in ['main', 'test']:
            add(f'analyze-{name}-{network}', [act('psbt.analyze', {'psbt': value, 'network': network})])
    for index, vector in enumerate(BIP):
        add(f"analyze-bip-{index}-{vector['bip']}", [act('psbt.analyze', {'psbt': vector['psbt'], 'network': 'test'})])
    for name, raw in [('binary', base), ('binary-base64', text.encode()), ('binary-hex', base.hex().encode()),
                      ('binary-nonascii', b'\xff\xfe' + text.encode()), ('binary-truncated', base[:-3]), ('binary-mixed', base64.b64decode(mixed))]:
        add('analyze-' + name, [act('psbt.analyze', {'psbt': None, 'network': 'main'}, {'psbt': raw})])
    for index, args in enumerate([
        {'psbt': text}, {'psbt': text, 'network': 'bad'}, {'psbt': text, 'network': 5}, {'psbt': 'zz', 'network': 'bad'},
        {'network': 'main'}, {'psbt': text, 'psbt_token': 'x', 'network': 'main'}, {'psbt_token': 'unknown', 'network': 'main'},
        {'psbt_token': 7, 'network': 'main'}, {'psbt': 5, 'network': 'main'}, {'psbt': None, 'network': 'main'},
        {'psbt': [text], 'network': 'main'}, {'psbt': 'cHNidP8\udc00', 'network': 'main'}, {'psbt': '', 'network': 'main'},
        {'psbt': text, 'network': 'main', 'scenario': {}}, {'psbt': text, 'network': 'main', 'unknown': 1},
        {'psbt': text, 'network': 'main', 'expected_scope': {'workspace_id': 'ws', 'profile_id': 'p'}},
        {'psbt': text, 'network': 'main', 'expected_scope': {'workspace_id': 'ws', 'profile_id': 'other'}},
        {'psbt': text, 'network': 'main', 'expected_scope': None}, {'psbt': '70736274ff0', 'network': 'main'},
        {'psbt': '70736274FF', 'network': 'main'}, {'psbt': base.hex().upper().replace('70736274FF', '70736274ff', 1), 'network': 'main'},
    ]):
        add(f'analyze-validation-{index}', [act('psbt.analyze', args)])
    for index, args in enumerate([
        {}, {'max_states': 1}, {'max_states': 2}, {'max_duration_ms': 30000}, {'max_states': True}, {'max_states': 0},
        {'max_states': 2000001}, {'max_duration_ms': 30001}, {'max_duration_ms': 1.0},
        {'scenario': {'kind': 'coinjoin_intrafees', 'max_received_fee_msat': '1000', 'max_paid_fee_msat': '2000'}},
        {'scenario': {'kind': 'independent', 'protocol': 'whirlpool'}}, {'scenario': {'bad': True}}, {'scenario': None},
        {'scenario': []}, {'scenario': {'kind': 'independent', 'max_paid_fee_msat': '1000'}},
        {'scenario': {'bad': True}, 'max_states': 0}, {'subject': 'x'},
    ]):
        add(f'entropy-{index}', [act('psbt.entropy', {'psbt': text, 'network': 'main', **args})])
    for name, value in [('payjoin', PAYJOIN['payjoin proposal']), ('mixed', mixed), ('v2', b64(encode(*v2()))), ('invalid', 'zz')]:
        add('entropy-' + name, [act('psbt.entropy', {'psbt': value, 'network': 'test', 'scenario': {'bad': True}})])
    add('entropy-binary', [act('psbt.entropy', {'psbt': None, 'network': 'main', 'max_states': 3}, {'psbt': base})])
    original, proposal = PAYJOIN['Original PSBT'], PAYJOIN['payjoin proposal']
    constraints = {'payment_output_index': 1, 'additional_fee_output_index': 0, 'max_additional_fee_contribution_msat': '182000'}
    for index, payjoin in enumerate([
        constraints, {**constraints, 'max_additional_fee_contribution_msat': '181000'}, {**constraints, 'minimum_fee_rate_sat_vb': '2'},
        {**constraints, 'minimum_fee_rate_sat_vb': '0'}, {**constraints, 'minimum_fee_rate_sat_vb': ' 1.5 '},
        {**constraints, 'minimum_fee_rate_sat_vb': '1_0'}, {**constraints, 'minimum_fee_rate_sat_vb': '-0'},
        {**constraints, 'minimum_fee_rate_sat_vb': '-1'}, {**constraints, 'minimum_fee_rate_sat_vb': 'NaN'},
        {**constraints, 'minimum_fee_rate_sat_vb': 'abc'}, {**constraints, 'minimum_fee_rate_sat_vb': '1' * 41},
        {**constraints, 'minimum_fee_rate_sat_vb': 2}, {**constraints, 'minimum_fee_rate_sat_vb': None},
        {**constraints, 'minimum_fee_rate_sat_vb': '1e2'}, {**constraints, 'minimum_fee_rate_sat_vb': '١'},
        {'payment_output_index': 1}, {'payment_output_index': 0}, {'payment_output_index': 1, 'allow_output_substitution': True},
        {'payment_output_index': True}, {'payment_output_index': 5}, {'payment_output_index': -1}, {'payment_output_index': 1.0},
        {}, [], 'x', {'payment_output_index': 1, 'unknown': 1},
        {'payment_output_index': 1, 'additional_fee_output_index': 1}, {'payment_output_index': 1, 'additional_fee_output_index': None},
        {'payment_output_index': 1, 'additional_fee_output_index': False}, {'payment_output_index': 1, 'max_additional_fee_contribution_msat': 1000},
        {'payment_output_index': 1, 'max_additional_fee_contribution_msat': '1001'}, {'payment_output_index': 1, 'max_additional_fee_contribution_msat': ''},
        {'payment_output_index': 1, 'max_additional_fee_contribution_msat': '1' * 20}, {'payment_output_index': 1, 'max_additional_fee_contribution_msat': '2100000000000000001000'[:19]},
        {'payment_output_index': 1, 'max_additional_fee_contribution_msat': '+1000'}, {'payment_output_index': 1, 'allow_output_substitution': 1},
        {'payment_output_index': 1, 'max_additional_fee_contribution_msat': None},
    ]):
        add(f'compare-payjoin-{index}', [act('psbt.compare', {'before': original, 'after': proposal, 'network': 'test', 'payjoin': payjoin})])
    add('compare-filled', [act('psbt.compare', {'before': original, 'after': payloads['payjoin-filled'], 'network': 'test', 'payjoin': constraints})])
    add('compare-reverse', [act('psbt.compare', {'before': proposal, 'after': original, 'network': 'test', 'payjoin': {'payment_output_index': 0}})])
    add('compare-plain', [act('psbt.compare', {'before': text, 'after': mixed, 'network': 'main'})])
    add('compare-same', [act('psbt.compare', {'before': mixed, 'after': mixed, 'network': 'regtest', 'payjoin': {'payment_output_index': 0}})])
    add('compare-null-payjoin', [act('psbt.compare', {'before': text, 'after': text, 'network': 'main', 'payjoin': None})])
    add('compare-binary', [act('psbt.compare', {'before': None, 'after': None, 'network': 'test', 'payjoin': constraints},
                               {'before': base64.b64decode(original), 'after': proposal.encode()})])
    for index, args in enumerate([
        {'before': 'zz', 'after': text, 'network': 'bad'}, {'before': 'zz', 'after': 'yy', 'network': 'main'},
        {'before': text, 'after': 'yy', 'network': 'main'}, {'before': text, 'network': 'main'}, {'after': text, 'network': 'main'},
        {'before': text, 'before_token': 'x', 'after': text, 'network': 'main'}, {'before': text, 'after_token': 'x', 'network': 'main'},
        {'before': text, 'after': text}, {'before': text, 'after': text, 'network': 'main', 'psbt': text},
        {'before': b64(encode(*sample()[:2], [[], []])), 'after': text, 'network': 'main'},
    ]):
        add(f'compare-validation-{index}', [act('psbt.compare', args)])
    for flag in (1, 2, 3, 0x81, 0x82, 0x83):
        for template in ('p2wpkh', 'p2sh'):
            pair = receiver(flag=flag, template=template)
            add(f'payjoin-signature-{template}-{flag}', [act('psbt.compare', {'before': b64(pair[0]), 'after': b64(pair[1]), 'network': 'test', 'payjoin': {'payment_output_index': 0}})])
    for mutate, template in [('high_s', 'p2wpkh'), ('flip', 'p2wpkh'), ('three_items', 'p2wpkh'), ('wrong_key_hash', 'p2wpkh'),
                             ('bad_flag', 'p2wpkh'), ('scriptsig', 'p2wpkh'), ('no_witness', 'p2wpkh'), ('bad_redeem', 'p2sh'),
                             ('two_pushes', 'p2sh'), ('high_s', 'p2sh')]:
        pair = receiver(template=template, mutate=mutate)
        add(f'payjoin-signature-{template}-{mutate}', [act('psbt.compare', {'before': b64(pair[0]), 'after': b64(pair[1]), 'network': 'test', 'payjoin': {'payment_output_index': 0, 'minimum_fee_rate_sat_vb': '1'}})])
    uncompressed = SECRET.get_public_key()
    uncompressed.compressed = False
    for name, key in [('uncompressed', uncompressed.sec()), ('hybrid', b'\x06' + uncompressed.sec()[1:]), ('short', KEY[:-1])]:
        try:
            pair = receiver(key=key)
        except Exception:
            continue
        add(f'payjoin-signature-key-{name}', [act('psbt.compare', {'before': b64(pair[0]), 'after': b64(pair[1]), 'network': 'test', 'payjoin': {'payment_output_index': 0}})])
    # SIGHASH_SINGLE beyond the proposal outputs commits to a zero hash.
    original, proposal = receiver(flag=3)
    add('payjoin-single-in-range', [act('psbt.compare', {'before': b64(original), 'after': b64(proposal), 'network': 'test', 'payjoin': {'payment_output_index': 2}})])
    # Structural Payjoin scenarios.
    for side in ('inputs', 'outputs'):
        original, proposal = ordered_pair()
        getattr(proposal, side).reverse()
        add('payjoin-shuffled-' + side, [act('psbt.compare', {'before': b64(original), 'after': b64(proposal), 'network': 'test', 'payjoin': {'payment_output_index': 0}})])
    original, proposal = ordered_pair()
    payment = proposal.outputs.pop(0)
    payment.script_pubkey = Script(b'\x00\x14' + b'\x45' * 20)
    proposal.outputs.append(payment)
    for substitute in (True, False):
        add(f'payjoin-substitution-{substitute}', [act('psbt.compare', {'before': b64(original), 'after': b64(proposal), 'network': 'test', 'payjoin': {'payment_output_index': 0, 'allow_output_substitution': substitute}})])
    proposal.outputs[0], proposal.outputs[1] = proposal.outputs[1], proposal.outputs[0]
    add('payjoin-substitution-shuffled', [act('psbt.compare', {'before': b64(original), 'after': b64(proposal), 'network': 'test', 'payjoin': {'payment_output_index': 0, 'allow_output_substitution': True}})])
    original, proposal = ordered_pair(duplicate_outputs=True)
    add('payjoin-duplicates', [act('psbt.compare', {'before': b64(original), 'after': b64(proposal), 'network': 'test', 'payjoin': {'payment_output_index': 2}})])
    proposal.outputs.pop(0)
    proposal.outputs[0].value = 99_000
    add('payjoin-duplicates-cardinality', [act('psbt.compare', {'before': b64(original), 'after': b64(proposal), 'network': 'test', 'payjoin': {'payment_output_index': 2}})])
    _, proposal = ordered_pair(duplicate_outputs=True)
    proposal.outputs[0].value = proposal.outputs[1].value = 1
    add('payjoin-duplicates-value', [act('psbt.compare', {'before': b64(original), 'after': b64(proposal), 'network': 'test', 'payjoin': {'payment_output_index': 2, 'additional_fee_output_index': 0, 'max_additional_fee_contribution_msat': '5000000'}})])
    original, proposal = ordered_pair()
    proposal.inputs[1] = deepcopy(proposal.inputs[0])
    add('payjoin-duplicate-input', [act('psbt.compare', {'before': b64(original), 'after': b64(proposal), 'network': 'test', 'payjoin': {'payment_output_index': 0}})])
    # Proposal output 3 is the original fee output 2 after the receiver insertion.
    for change, maximum, extra in [(-1000, '2000000', {}), (0, '0', {}), (1000, '0', {}), (-60_000, '200000000', {}),
                                   (-1000, '1000', {'minimum_fee_rate_sat_vb': '0.5'}), (-1000, '999000', {})]:
        for heterogeneous in (False, True):
            original, proposal = receiver(adjust={3: change}, heterogeneous=heterogeneous)
            add(f'payjoin-fee-output-{change}-{maximum}-{heterogeneous}-{len(extra)}', [act('psbt.compare', {'before': b64(original), 'after': b64(proposal), 'network': 'test', 'payjoin': {'payment_output_index': 0, 'additional_fee_output_index': 2, 'max_additional_fee_contribution_msat': maximum, **extra}})])
    original, proposal = receiver(adjust={0: 1000, 3: -1000})
    add('payjoin-payment-increase', [act('psbt.compare', {'before': b64(original), 'after': b64(proposal), 'network': 'test', 'payjoin': {'payment_output_index': 0, 'additional_fee_output_index': 2}})])
    original, proposal = receiver(adjust={2: -1})
    add('payjoin-protected-decrease', [act('psbt.compare', {'before': b64(original), 'after': b64(proposal), 'network': 'test', 'payjoin': {'payment_output_index': 0}})])
    original, proposal = ordered_pair()
    proposal.inputs[0].witness_utxo = None
    add('payjoin-missing-prevout', [act('psbt.compare', {'before': b64(original), 'after': b64(proposal), 'network': 'test', 'payjoin': {'payment_output_index': 0}})])
    proposal.inputs[1].witness_utxo = TransactionOutput(1, Script(SCRIPT))
    add('payjoin-changed-prevout', [act('psbt.compare', {'before': b64(original), 'after': b64(proposal), 'network': 'test', 'payjoin': {'payment_output_index': 0}})])
    original, proposal = ordered_pair()
    proposal.tx.locktime = 5
    proposal.tx.version = 1
    proposal.inputs[0].sequence = 0xFFFFFFFE
    add('payjoin-version-locktime-sequence', [act('psbt.compare', {'before': b64(original), 'after': b64(PSBT.parse(proposal.serialize())), 'network': 'test', 'payjoin': {'payment_output_index': 0}})])
    return cases


if __name__ == '__main__':
    data = vectors()
    DEST.write_text(json.dumps(data, ensure_ascii=True, sort_keys=True, separators=(',', ':')) + '\n')
    print(f'{len(data)} cases, {sum(len(c["expected"]) for c in data)} states')
