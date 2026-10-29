"""Whole finalized-stage comparison through an unpackaged raw-only driver."""
import json
import subprocess
from collections import Counter
from pathlib import Path

import pytest

from kassiber.core.custody_compute import compute_custody_projection
from tests.custody_engine_compare import assert_same, permuted_snapshot
from tests.custody_gap_stage import suites
from tests.custody_snapshot import to_jsonable


def expected(book):
    try:
        return to_jsonable(compute_custody_projection(book).finalized_tax_projection, redact_config=False)
    except Exception as exc:
        return {'error': getattr(exc, 'code', type(exc).__name__)}


def test_projection_adapter_fixtures_match_original_python():
    from tests.custody_projection_adapter import vectors
    assert json.loads(Path('tests/fixtures/custody_projection_adapter.json').read_text()) == vectors()
    for filename in ('custody_projection_stage.json', 'custody_projection_admission.json'):
        for case in json.loads((Path('tests/fixtures') / filename).read_text()):
            assert_same(expected(case['input']), case['expected'], case['case'])


@pytest.fixture(scope='module')
def projection_stage_driver():
    built = subprocess.run(
        ['cargo', 'build', '--offline', '--locked', '--release', '-p', 'kassiber-custody',
         '--example', 'projection-stage', '--message-format=json'],
        capture_output=True, text=True, check=True)
    return next(record['executable'] for line in built.stdout.splitlines()
                if (record := json.loads(line)).get('executable'))


@pytest.mark.parametrize('suite,scenarios,generate', suites(), ids=lambda v: v if isinstance(v, str) else None)
def test_finalized_raw_stage(suite, scenarios, generate, projection_stage_driver):
    driver = projection_stage_driver
    counts = Counter()
    with subprocess.Popen([str(driver)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True) as process:
        for seed in range(2000):
            book = generate(seed)
            for shuffled, candidate in enumerate((book, permuted_snapshot(book)) if seed < len(scenarios)*2 else (book,)):
                process.stdin.write(json.dumps(candidate, ensure_ascii=True) + '\n')
                process.stdin.flush()
                actual = json.loads(process.stdout.readline())
                reference = expected(candidate)
                assert_same(reference, actual, f'{suite}:{seed}:{shuffled}')
                counts['permutations' if shuffled else 'books'] += 1
                counts['refusals' if 'error' in reference else 'results'] += 1
                if 'error' not in reference and not reference['rows'] and not reference['quarantines']:
                    counts['empty_results'] += 1
        process.stdin.close()
        assert process.wait() == 0
    assert counts['books'] == 2000
    assert counts['permutations'] == len(scenarios) * 2
    print(suite + ' finalized comparisons: ' + json.dumps(counts, sort_keys=True))
