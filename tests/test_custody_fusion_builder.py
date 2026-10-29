"""The unmodified Python request builder is the independent R2g oracle."""
import json
from pathlib import Path
import subprocess

from tests.custody_fusion_builder import vectors


def test_fusion_builder_fixtures_match_python():
    assert json.loads(Path('tests/fixtures/custody_fusion_builder.json').read_text()) == vectors()


def test_fusion_builder_probe():
    built = subprocess.run(['cargo', 'build', '--offline', '--locked', '-p', 'kassiber-custody',
                            '--example', 'ledger-request', '--message-format=json'],
                           capture_output=True, text=True, check=True)
    driver = next(r['executable'] for line in built.stdout.splitlines()
                  if (r := json.loads(line)).get('executable'))
    cases = vectors(include_goldens=True)
    result = subprocess.run([driver], input=''.join(json.dumps(c['input']) + '\n' for c in cases),
                            capture_output=True, text=True, check=True)
    for case, line in zip(cases, result.stdout.splitlines(), strict=True):
        actual = json.loads(line)
        if 'error' in actual:
            assert actual['error'].pop('stage')
        assert actual == case['expected'], case['case']


def test_fusion_builder_from_rust_finalized_projection():
    from kassiber.core.custody_compute import compute_custody_projection
    from kassiber.core.engines.base import TaxEngineLedgerInputs
    from tests.custody_fusion_builder import outcome
    from tests.custody_gap_stage import suites

    built = subprocess.run(['cargo', 'build', '--offline', '--locked', '-p', 'kassiber-custody',
                            '--example', 'ledger-request', '--message-format=json'],
                           capture_output=True, text=True, check=True)
    driver = next(r['executable'] for line in built.stdout.splitlines()
                  if (r := json.loads(line)).get('executable'))
    from collections import Counter
    from tests.custody_fusion_builder import active_inputs
    counts = Counter()
    count = 0
    with subprocess.Popen([driver], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True) as process:
        for suite, scenarios, generate in suites():
            for seed in range(len(scenarios)):
                book = generate(seed)
                refs = {w['id']: {k: v for k, v in w.items()
                                 if k not in {'config_json', 'config_text_mentions_samourai'}}
                        for w in book['wallets']}
                process.stdin.write(json.dumps({'snapshot': book, 'wallets': list(refs.items())}) + '\n')
                process.stdin.flush()
                actual = json.loads(process.stdout.readline())
                try:
                    result = compute_custody_projection(book)
                except Exception as exc:
                    assert actual['outcome'] == 'upstream_refusal', (suite, seed, actual)
                    assert actual['error']['code'] == getattr(exc, 'code', type(exc).__name__)
                else:
                    expected = outcome(book['profile'], TaxEngineLedgerInputs(
                        finalized_tax_projection=result.finalized_tax_projection,
                        wallet_refs_by_id=refs))
                    if 'error' in actual:
                        actual['error'].pop('stage')
                    assert actual == expected, (suite, seed)
                counts[actual['outcome']] += 1
                # Only the current generated seed can contribute activity.
                counts['active_distinct_seed_inputs'] += active_inputs([book], [(book, actual)])
                count += 1
        process.stdin.close()
        assert process.wait() == 0
    assert count == sum(len(scenarios) for _, scenarios, _ in suites())
    assert counts['active_distinct_seed_inputs'] > 0
    print(f'raw builder comparisons: {count}; {dict(counts)}')


def test_vacuous_fusion_probe_fails():
    import pytest
    from tests.custody_fusion_builder import active_inputs, require_activity

    seed = {'seed': 1}
    companion = {'seed': 2}
    active = {'outcome': 'request', 'request': {'rows': [{'id': 'active'}]}}
    empty = {'outcome': 'empty'}
    with pytest.raises(AssertionError, match='vacuous fusion probe'):
        require_activity([seed], [(seed, empty), (companion, active)])
    with pytest.raises(AssertionError, match='vacuous fusion probe'):
        require_activity([seed], [(seed, empty)])
    assert active_inputs([seed], [(seed, active), (seed, active), (companion, active)]) == 1
