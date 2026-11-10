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
    stages = {}
    try:
        return to_jsonable(compute_custody_projection(book, stage_observer=lambda n, v: stages.setdefault(n, v)).finalized_tax_projection, redact_config=False)
    except Exception as exc:
        from kassiber.core.custody_compute import STAGES
        return {'error': getattr(exc, 'code', type(exc).__name__), 'stage': next(n for n in STAGES if n not in stages)}


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
    assert counts['empty_results'] == {'evidence': 48, 'channels': 193, 'gaps': 64}.get(suite, 0)
    print(suite + ' finalized comparisons: ' + json.dumps(counts, sort_keys=True))


def test_real_writer_payout_books(tmp_path, projection_stage_driver):
    from tests.custody_projection_payout_books import books
    inputs = list(books(tmp_path))
    requests = [(name, candidate) for name, book in inputs
                for candidate in (book, permuted_snapshot(book))]
    result = subprocess.run([projection_stage_driver],
        input=''.join(json.dumps(book, ensure_ascii=True) + '\n' for _, book in requests),
        capture_output=True, text=True, check=True)
    outputs = [json.loads(line) for line in result.stdout.splitlines()]
    assert len(outputs) == len(requests)
    for (name, book), actual in zip(requests, outputs):
        stages = {}
        try:
            compute_custody_projection(book, stage_observer=lambda n, v: stages.setdefault(n, v))
        except Exception:
            pass  # The live refusal is compared below, including its stage.
        quantity = stages['quantity']
        assert len(quantity.reviewed_direct_payouts) == 1, name
        if name.startswith('legacy_'):
            assert quantity.reviewed_direct_payouts[0]['payout_fiat_value'] == name.removeprefix('legacy_')
        if name.startswith('partial_'):
            assert any(d.source.start_msat > 0 for d in quantity.projection.decisions), name
        if name == 'reviewed_conversion':
            assert quantity.reviewed_conversion_pairs, name
        reference = expected(book)
        assert_same(reference, actual, name)
        if name in {'carry', 'at_eth', 'legacy_１２.５'}:
            assert reference['cross_asset_pairs'], name
        if name in {'taxable', 'tiny', 'huge', 'partial_taxable'}:
            assert any(r.get('pricing_method') == 'direct_payout_review' for r in reference['rows']), name
        if name.startswith('partial_'):
            residual = [r for r in reference['rows'] if r.get('custody_slice_start_msat') == 0
                        and r.get('custody_slice_end_msat') == 60000]
            assert len(residual) == 1
            assert residual[0].get('pricing_method') != 'direct_payout_review'
            if name == 'partial_carry':
                assert len(reference['cross_asset_pairs']) == 1
                assert len([r for r in reference['rows'] if r['id'].startswith('direct-payout:')]) == 2
        if name == 'no_value':
            assert any(q['reason'] == 'at_swap_price_required' for q in reference['quarantines'])
        assert 'error' in reference or reference['rows'] or reference['quarantines'], name + ' is vacuous'
    assert len(inputs) == 14


def test_projection_row_mapping_order_is_not_a_contract(projection_stage_driver):
    """Fusion consumes named fields; row/pair sequence order remains observable."""
    case = json.loads(Path('tests/fixtures/custody_projection_stage.json').read_text())[0]
    reference = expected(case['input'])
    assert len(reference['rows']) > 1
    response = subprocess.run([projection_stage_driver], input=json.dumps(case['input']) + '\n',
                              capture_output=True, text=True, check=True)
    actual = json.loads(response.stdout)
    assert list(reference['rows'][0]) != list(actual['rows'][0])
    assert_same(reference, actual)
    reordered = dict(reference, rows=[dict(reversed(list(row.items()))) for row in reference['rows']])
    assert list(reference['rows'][0]) != list(reordered['rows'][0])
    assert_same(reference, reordered)
    with pytest.raises(AssertionError):
        assert_same(reference, dict(reference, rows=list(reversed(reference['rows']))))


def test_review_witnesses_match_live_reference():
    from tests.custody_projection_quarantines import evaluate
    for case in json.loads(Path('tests/fixtures/custody_projection_review.json').read_text()):
        assert_same(evaluate(case), {'expected': case['expected']}, case['case'])


def test_binding_retains_refusal_stages_after_partial_carry_fix(tmp_path):
    import kassiber_tax
    from tests.custody_projection_payout_books import books
    from tests.test_custody_quantity_stage import raw_refusal_snapshot
    from tests.custody_projection_payout_books import LEGACY_HUGE
    payout_books = dict(books(tmp_path))
    assert 'error' not in json.loads(kassiber_tax.custody(json.dumps(payout_books['partial_carry'])))
    from tests.custody_fanout_generator import fanout_snapshot
    stages = []
    # A review stored before round 5's price limit refuses at finalization.
    for book in (raw_refusal_snapshot(), payout_books['legacy_' + LEGACY_HUGE]):
        reference = expected(book)
        actual = json.loads(kassiber_tax.custody(json.dumps(book)))['error']
        assert reference['error'] == actual['code']
        assert reference['stage'] == actual['stage']
        stages.append(actual['stage'])
    assert stages == ['components', 'finalized_projection']
    # Round 5 turned this book's invalid stored price from a finalization
    # refusal into an invalid_transaction_price hold, in both engines.
    book = fanout_snapshot(24)
    reference = expected(book)
    actual = json.loads(kassiber_tax.custody(json.dumps(book)))['stages']['finalized_projection']
    assert 'invalid_transaction_price' in {q['reason'] for q in reference['quarantines']}
    assert_same(reference, actual)


def test_migration_surrogate_blocker_finalized_parity():
    import kassiber_tax
    from tests.custody_interpretation_stage_vectors import WRAPPERS
    cases = [case for case in json.loads(WRAPPERS.read_text()) if case['case'].startswith('migration_1_')]
    assert len(cases) == 7
    for case in cases:
        reference = expected(case['input'])
        assert 'error' not in reference
        actual = json.loads(kassiber_tax.custody(json.dumps(case['input'])))
        assert_same(reference, actual['stages']['finalized_projection'], case['case'])
