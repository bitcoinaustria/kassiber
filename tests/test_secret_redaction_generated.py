"""A hundred thousand deterministic compositions, including preservation checks."""
import hashlib

from kassiber.redaction import redact_secret_text
from tests.redaction_generation import SPEC, generated_cases


def test_generated_secret_floor():
    digest = hashlib.sha256()
    for case in generated_cases():
        output = redact_secret_text(case['input'])
        for start in range(len(case['sentinel']) - 3):
            assert case['sentinel'][start:start + 4] not in output, (case['index'], case['input'], output)
        for preserved in case['preserved']:
            assert preserved in output, (case['index'], preserved, output)
        digest.update((output + '\n').encode())
    assert digest.hexdigest() == SPEC['output_sha256']
