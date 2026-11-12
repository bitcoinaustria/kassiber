"""Whole-stage parity, scenario census and refusal/comparison boundaries."""
from collections import Counter
import json
from tests.custody_engine_compare import python_refusal
from pathlib import Path

import pytest
import kassiber_tax

from tests.custody_engine_compare import assert_same, count_valuation_holds, rust_results, permuted_snapshot
from tests.custody_interpretation_stage import python_stage, suites
from tests.custody_snapshot import to_jsonable

FIXTURES = Path(__file__).parent / "fixtures"
METRICS = ("claims", "native_audits", "native_component_reconciliations", "non_event_transaction_ids", "blocked_transaction_ids", "transient_blocked_transaction_ids")


@pytest.mark.parametrize("suite,scenarios,generate", suites(), ids=lambda v: v if isinstance(v,str) else None)
def test_every_generator_matches_and_counts_both_engines(suite, scenarios, generate):
    matrix=json.loads((FIXTURES/"custody_interpretation_counts.json").read_text())[suite]
    assert set(matrix)==set(scenarios)
    counts={engine:{case:Counter() for case in scenarios} for engine in ("Python","Rust")}
    finalized_refusals=Counter()
    holds=Counter()
    for seed in range(2000):
        book=generate(seed);case=scenarios[seed%len(scenarios)]
        for permuted,b in enumerate((book,permuted_snapshot(book)) if seed<len(scenarios)*2 else (book,)):
            expected=to_jsonable(python_stage(b),redact_config=False)
            stages,skipped=rust_results(b, finalized_refusals=finalized_refusals)
            count_valuation_holds(b, stages, holds)
            assert "interpretation" not in skipped
            actual=stages["interpretation"]
            assert_same(expected,actual,f"{suite}:{case}:{seed}:{permuted}")
            if permuted: continue
            for engine,result in (("Python",expected),("Rust",actual)):
                signature=','.join(str(len(result[key])) for key in METRICS)
                counts[engine][case][signature]+=1
    # Round 5: the fan-out books whose NaN values refused the finalized
    # projection are invalid_transaction_price holds in both engines.
    assert not finalized_refusals
    assert holds == (Counter({'finalized_projection:invalid_transaction_price': 178}) if suite == 'fanout' else Counter())
    print(suite + ' finalized valuation holds: ' + json.dumps(holds, sort_keys=True))
    for engine,scenarios_counts in counts.items():
        assert scenarios_counts==matrix,(engine,suite,scenarios_counts)
        for index,case in enumerate(scenarios):
            assert sum(scenarios_counts[case].values())==2000//len(scenarios)+(index<2000%len(scenarios))
    print(suite+" full-stage counts: "+json.dumps(counts,sort_keys=True))


def test_full_stage_vectors_wrappers_and_refusals():
    from tests.custody_interpretation_stage_vectors import PATH, WRAPPERS, vectors, wrappers
    assert json.loads(PATH.read_text())==vectors()
    expected_wrappers=json.loads(WRAPPERS.read_text())
    assert expected_wrappers==wrappers()
    finalized_refusals=Counter()
    for v in expected_wrappers:
        response=json.loads(kassiber_tax.custody(json.dumps(v["input"]), finalized=False))
        if 'error' in v:
            assert response['error']['code']==v['error'],v['case']
            assert response['error']['stage'] == python_refusal(v['input'], finalized=False)[1]
            assert 'stages' not in response
        else:
            stages, _ = rust_results(v['input'], finalized_refusals=finalized_refusals)
            assert_same(v['expected'],stages['interpretation'],v['case'])
    assert not finalized_refusals
    from tests.custody_reconciliation_vectors import PATH as REC_PATH, HELPERS, vectors as rec_vectors, helpers
    assert json.loads(REC_PATH.read_text())==rec_vectors()
    assert json.loads(HELPERS.read_text())==helpers()


def test_interpretation_difference_is_recorded_even_when_caught(tmp_path,monkeypatch):
    from tests import custody_engine_compare as harness
    from kassiber.core import custody_journal
    from tests.custody_snapshot_generator import templates
    records=tmp_path/'stage-records.jsonl'
    monkeypatch.setenv(harness.RECORDS_ENV,str(records))
    monkeypatch.setenv(harness.INJECT_ENV,'interpretation')
    monkeypatch.setattr(custody_journal,'_build_observer_factory',None)
    class Builder:
        conn=profile=None
        def load_snapshot(self): return templates()[0]
    monkeypatch.setattr(harness,'_INSTALLED_MODE','census')
    with pytest.raises(AssertionError,match='interpretation'):
        harness._build(Builder(),finalized=False)
    assert harness.session_report(harness.read_records(records))[1]==['custody differences: 1']
