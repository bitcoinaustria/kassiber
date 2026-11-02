"""Round 5f's fourth reviewer generator, with its original RNG and distribution."""
import hashlib
import html
import json
from pathlib import Path
from urllib.parse import unquote

import pytest

from kassiber.redaction import redact_secret_text
from tests.redaction_fourth_generation import cases

SPEC = json.loads((Path(__file__).parent / 'fixtures/secret_redaction_fourth_generator.json').read_text())


def _views(output):
    views = [output, output.replace('\\', '')]
    for start in list(views):
        for _ in range(3):
            start = unquote(start)
            views.append(start)
    return views + [html.unescape(view) for view in views]


def _check(case, output):
    views = _views(output)
    secret = case['sentinel']
    for start in range(len(secret) - 3):
        assert all(secret[start:start + 4] not in view for view in views), (case, output)
    for keep in case['keep']:
        assert keep in output, (case, output)


def test_fourth_generated_secret_floor():
    digest = hashlib.sha256()
    for case in cases(SPEC['count'], SPEC['seed']):
        output = redact_secret_text(case['input'])
        _check(case, output)
        digest.update((output + '\n').encode())
    assert digest.hexdigest() == SPEC['output_sha256']


@pytest.mark.parametrize('seed', SPEC['extra_seeds'])
def test_fourth_generator_other_seeds(seed):
    # A pinned digest proves parity; other seeds keep the floor from fitting one stream.
    for case in cases(SPEC['extra_count'], seed):
        _check(case, redact_secret_text(case['input']))
