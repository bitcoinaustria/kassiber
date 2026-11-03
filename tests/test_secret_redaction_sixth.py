"""Round 5h's sixth reviewer generator, with its original RNG and distribution."""
import hashlib
import html
import json
import re
from pathlib import Path
from urllib.parse import unquote

import pytest

from kassiber.redaction import redact_secret_text
from tests.redaction_sixth_generation import cases

SPEC = json.loads((Path(__file__).parent / 'fixtures/secret_redaction_sixth_generator.json').read_text())


def _views(output):
    views = [output, output.replace('\\', '')]
    for start in list(views):
        for _ in range(3):
            start = unquote(start)
            views.append(start)
    return views + [html.unescape(view) for view in views]


def _leaks(case, output):
    """Planted secrets that survive redaction; every class is inside the contract."""
    views = _views(output)
    planted = [secret for secret, _ in case['secrets']]
    leaks = []
    for secret, kind in case['secrets']:
        fragments = {secret[i:i + 4] for i in range(len(secret) - 3)}
        fragments = {f for f in fragments if not any(f in other for other in planted if other != secret)
                     and f not in case['input'].replace(secret, '\0')}
        if any(f in view for view in views for f in fragments):
            leaks.append((secret, kind))
    for number, kind in case['numeric']:
        # A code that also appears as readable data (a token count) is ambiguous.
        pattern = r'(?<![0-9])' + number + r'(?![0-9])'
        if len(re.findall(pattern, case['input'])) == 1 and re.search(pattern, output):
            leaks.append((number, kind))
    return leaks


def test_sixth_generated_secret_floor():
    digest = hashlib.sha256()
    for case in cases(SPEC['count'], SPEC['seed']):
        output = redact_secret_text(case['input'])
        assert not _leaks(case, output), (case, output)
        digest.update((output + '\n').encode())
    assert digest.hexdigest() == SPEC['output_sha256']


@pytest.mark.parametrize('seed', SPEC['extra_seeds'])
def test_sixth_generator_other_seeds(seed):
    for case in cases(SPEC['extra_count'], seed):
        output = redact_secret_text(case['input'])
        assert not _leaks(case, output), (case, output)
