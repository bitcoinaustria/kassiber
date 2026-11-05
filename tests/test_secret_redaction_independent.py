"""The reviewer's independently authored input distribution stays independent."""
import hashlib
import json
from pathlib import Path

from kassiber.redaction import redact_secret_text
from tests.redaction_independent_generation import generated_cases

SPEC = json.loads((Path(__file__).parent / 'fixtures/secret_redaction_independent_generator.json').read_text())


def test_independent_generated_secret_floor():
    digest = hashlib.sha256()
    for case in generated_cases(SPEC['count'], SPEC['seed']):
        output = redact_secret_text(case['input'])
        for start in range(len(case['sentinel']) - 3):
            assert case['sentinel'][start:start + 4] not in output, (case['index'], case['input'], output)
        for preserved in case['preserved']:
            assert preserved in output, (case['index'], preserved, output)
        digest.update((output + '\n').encode())
    assert digest.hexdigest() == SPEC['output_sha256']
