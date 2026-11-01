"""Round 5e's third reviewer generator, with its original RNG and distribution."""
import hashlib
import json
from pathlib import Path
from urllib.parse import unquote

from kassiber.redaction import redact_secret_text
from tests.redaction_third_generation import cases

SPEC = json.loads((Path(__file__).parent / 'fixtures/secret_redaction_third_generator.json').read_text())


def test_third_generated_secret_floor():
    digest = hashlib.sha256()
    for case in cases(SPEC['count'], SPEC['seed']):
        output = redact_secret_text(case['input'])
        views = [output, output.replace('\\', '')]
        for _ in range(3):
            views.append(unquote(views[-1]))
        secret = case['sentinel']
        if 'pctlow' in case['kind']:
            views = [view.upper() for view in views]
        for start in range(len(secret) - 3):
            assert all(secret[start:start + 4] not in view for view in views), (case, output)
        assert case['keep'] in output, (case, output)
        digest.update((output + '\n').encode())
    assert digest.hexdigest() == SPEC['output_sha256']
