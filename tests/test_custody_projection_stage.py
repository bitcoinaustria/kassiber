"""Whole finalized-stage comparison through an unpackaged raw-only driver."""
import json
import subprocess
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
    for case in json.loads(Path('tests/fixtures/custody_projection_stage.json').read_text()):
        assert_same(expected(case['input']), case['expected'], case['case'])


@pytest.mark.parametrize('suite,scenarios,generate', suites(), ids=lambda v: v if isinstance(v, str) else None)
def test_finalized_raw_stage(suite, scenarios, generate):
    driver = Path('target/release/examples/projection-stage')
    assert driver.exists(), 'build the offline projection-stage example first'
    with subprocess.Popen([str(driver)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True) as process:
        for seed in range(2000):
            book = generate(seed)
            for shuffled, candidate in enumerate((book, permuted_snapshot(book)) if seed < len(scenarios)*2 else (book,)):
                process.stdin.write(json.dumps(candidate, ensure_ascii=True) + '\n')
                process.stdin.flush()
                actual = json.loads(process.stdout.readline())
                assert_same(expected(candidate), actual, f'{suite}:{seed}:{shuffled}')
        process.stdin.close()
        assert process.wait() == 0
