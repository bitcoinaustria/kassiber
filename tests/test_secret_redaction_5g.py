"""Shared regressions for the fail-closed scanner contract."""
import json
from pathlib import Path

import pytest

from kassiber.redaction import redact_secret_text

CASES = json.loads((Path(__file__).parent / 'fixtures/secret_redaction_5g_cases.json').read_text())


@pytest.mark.parametrize('case', CASES, ids=lambda c: c['id'])
def test_fail_closed_redaction(case):
    output = redact_secret_text(case['input'])
    if 'expected' in case:
        assert output == case['expected']
    secret = case['sentinel']
    if secret:
        assert secret not in output, (case, output)
        for start in range(len(secret) - 3):
            assert secret[start:start + 4] not in output, (case, output)
    for keep in case['preserved']:
        assert keep in output, (case, output)
