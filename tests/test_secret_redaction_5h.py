"""Round 5h regressions: word-aware names, per-key continuations and linear scans."""
import json
import subprocess
import sys
from pathlib import Path

import pytest

from kassiber.redaction import redact_secret_text
from kassiber.redaction_scanner import REDACTION_LIMIT

FIXTURES = Path(__file__).parent / 'fixtures'
CASES = json.loads((FIXTURES / 'secret_redaction_5h_cases.json').read_text())


@pytest.mark.parametrize('case', CASES, ids=lambda c: c['id'])
def test_5h_redaction(case):
    output = redact_secret_text(case['input'])
    assert output == case['expected']
    secret = case['sentinel']
    for start in range(len(secret) - 3):
        assert secret[start:start + 4] not in output, (case, output)
    for keep in case['preserved']:
        assert keep in output, (case, output)


def test_oversized_records_are_masked_whole():
    assert redact_secret_text('x' * REDACTION_LIMIT) == 'x' * REDACTION_LIMIT
    assert redact_secret_text('password=Pw9Leak ' + 'x' * REDACTION_LIMIT) == '[redacted-oversized]'


def test_each_shape_scales_linearly_to_400k():
    # Every shape on its own: a quadratic one costs ~100x for ten times the
    # input, a linear one ~10x. The constant absorbs timer noise on tiny runs.
    script = """
import json, sys, time
from kassiber.redaction import redact_secret_text
def cost(case, size):
    text = case['prefix'] + case['repeat'] * (size // len(case['repeat']) + 1) + case['suffix']
    start = time.process_time()
    redact_secret_text(text)
    return time.process_time() - start
slow = []
for case in json.load(sys.stdin):
    small, large = cost(case, 40000), cost(case, 400000)
    if large > 25 * small + 0.05:
        slow.append((case['name'], small, large))
assert not slow, slow
"""
    subprocess.run([sys.executable, '-c', script], input=(FIXTURES / 'secret_redaction_scaling_cases.json').read_text(),
                   text=True, check=True, timeout=900)
