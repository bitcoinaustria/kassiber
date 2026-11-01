"""Retain the unchanged Python label validator's Liquid legacy behavior."""
from embit import base58
from kassiber.core.chain_analysis_cases import validate_subject


def reproduce():
    p2pkh = base58.encode_check(b'\x39' + b'\x11' * 20)
    try:
        validate_subject(p2pkh, 'liquid', 'main')
    except RuntimeError as error:
        assert str(error) == 'Invalid address'
    else:
        raise AssertionError('reference unexpectedly accepted Liquid P2PKH')
    bitcoin_test_p2sh = base58.encode_check(b'\xc4' + b'\x11' * 20)
    assert validate_subject(bitcoin_test_p2sh, 'liquid', 'regtest') == bitcoin_test_p2sh
    return {'p2pkh_error': 'Invalid address', 'accepted_bitcoin_test_p2sh': bitcoin_test_p2sh}


if __name__ == '__main__':
    import json
    print(json.dumps(reproduce(), sort_keys=True))
