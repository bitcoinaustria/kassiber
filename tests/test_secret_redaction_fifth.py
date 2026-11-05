"""Round 5g's fifth reviewer generator, with its original RNG and distribution."""
import hashlib
import html
import json
import re
from pathlib import Path
from urllib.parse import unquote

import pytest

from kassiber.redaction import redact_secret_text
from tests.redaction_fifth_generation import cases

SPEC = json.loads((Path(__file__).parent / 'fixtures/secret_redaction_fifth_generator.json').read_text())
# Numeric names inside the contract; `twofa` alone and the count-shaped
# `token_ttl`, `session_token_max` and `pin_len` are metadata or out of scope.
NUMERIC = {'pin', 'otp', 'wallet_pin', 'device_pin', 'user_pin', 'login_otp', 'PIN', 'OTP', 'pin_code', 'otp_code', 'totp',
           'totp_code', 'mfa_code', '2fa_code', 'verification_code', 'pincode', 'token_num', 'api_key_num'}


def _views(output):
    views = [output, output.replace('\\', '')]
    for start in list(views):
        for _ in range(3):
            start = unquote(start)
            views.append(start)
    return views + [html.unescape(view) for view in views]


def _leaks(case, output):
    """Planted secrets inside the contract that survive redaction."""
    views = _views(output)
    planted = [secret for secret, _ in case['secrets']]
    leaks = []
    for secret, kind in case['secrets']:
        # Zero-width noise inside a key and prose-only provider errors are out of scope.
        if '~noise' in kind or (kind.endswith('-oos') and not kind.startswith('path_token')):
            continue
        fragments = {secret[i:i + 4] for i in range(len(secret) - 3)}
        fragments = {f for f in fragments if not any(f in other for other in planted if other != secret)
                     and f not in case['input'].replace(secret, '\0')}
        if any(f in view for view in views for f in fragments):
            leaks.append((secret, kind))
    for number, kind in case['numeric']:
        if '~noise' not in kind and kind.split(':', 1)[1] in NUMERIC \
                and re.search(r'(?<![0-9A-Fa-f])' + number + r'(?![0-9A-Fa-f])', output):
            leaks.append((number, kind))
    return leaks


def test_fifth_generated_secret_floor():
    digest = hashlib.sha256()
    for case in cases(SPEC['count'], SPEC['seed']):
        output = redact_secret_text(case['input'])
        assert not _leaks(case, output), (case, output)
        digest.update((output + '\n').encode())
    assert digest.hexdigest() == SPEC['output_sha256']


@pytest.mark.parametrize('seed', SPEC['extra_seeds'])
def test_fifth_generator_other_seeds(seed):
    for case in cases(SPEC['extra_count'], seed):
        output = redact_secret_text(case['input'])
        assert not _leaks(case, output), (case, output)
