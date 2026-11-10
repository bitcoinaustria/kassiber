"""Full gap stage parity and per-engine scenario counts; no live book roots."""
from collections import Counter
import json
from tests.custody_engine_compare import python_refusal
from pathlib import Path
import time

import pytest
import kassiber_tax

from tests.custody_engine_compare import assert_same,count_valuation_holds,permuted_snapshot,rust_results
from tests.custody_gap_stage import python_stage,suites
from tests.custody_gap_generator import signature
from tests.test_custody_observation_consistency import custody_book,_profile  # noqa: F401


@pytest.mark.parametrize("suite,scenarios,generate",suites(),ids=lambda v:v if isinstance(v,str) else None)
def test_every_gap_generator_matches_and_counts_both_engines(suite,scenarios,generate):
    expected=json.loads(Path("tests/fixtures/custody_gap_counts.json").read_text())[suite]
    assert set(expected)==set(scenarios)
    counts={engine:{case:Counter() for case in scenarios} for engine in ("Python","Rust")}
    finalized_refusals=Counter()
    holds=Counter()
    for seed in range(2000):
        book=generate(seed);case=scenarios[seed%len(scenarios)]
        for shuffled,b in enumerate((book,permuted_snapshot(book)) if seed<len(scenarios)*2 else (book,)):
            stage=python_stage(b)
            actual,skipped=rust_results(b, finalized_refusals=finalized_refusals)
            count_valuation_holds(b, actual, holds)
            assert "gaps" in actual and "gaps" not in skipped
            assert_same(stage,actual["gaps"],f"{suite}:{case}:{seed}:{shuffled}")
            if not shuffled:
                for engine,result in (("Python",stage),("Rust",actual["gaps"])):
                    counts[engine][case][signature(result)]+=1
    # Round 5: the fan-out books whose NaN values refused the finalized
    # projection are invalid_transaction_price holds in both engines.
    assert not finalized_refusals
    assert holds == (Counter({'finalized_projection:invalid_transaction_price': 178}) if suite == 'fanout' else Counter())
    print(suite + ' finalized valuation holds: ' + json.dumps(holds, sort_keys=True))
    for engine,actual in counts.items():
        assert actual==expected,(engine,suite,actual)
        for index,case in enumerate(scenarios):
            assert sum(actual[case].values())==2000//len(scenarios)+(index<2000%len(scenarios))
    print(suite+" gaps counts: "+json.dumps(counts,sort_keys=True))


def test_frozen_gap_vectors_are_python_outputs():
    from tests.custody_gap_vectors import PATH,STAGES_PATH,vectors,stage_vectors
    assert json.loads(PATH.read_text())==vectors()
    assert json.loads(STAGES_PATH.read_text())==stage_vectors()


def test_gap_stage_errors_follow_interpretation_and_admit_all_rows_before_ignoring():
    from tests.custody_gap_generator import snapshot
    from tests.custody_gap_stage import python_stages
    from tests.custody_gap_vectors import outcome
    for source in ("timestamp","digits"):
        book=snapshot(0)
        row=book["gap_worklist"]["rows"][0]
        if source=="timestamp":row["occurred_at"]="0001-01-01T00:00:00+01:00"
        else:row["amount"]="²"
        book["loan_legs"]=[dict(transaction_id=row["id"],role="loan_principal_out")]
        expected=outcome(lambda:python_stages(book))
        assert expected["error"]==("OverflowError" if source=="timestamp" else "ValueError")
        actual=json.loads(kassiber_tax.custody(json.dumps(book), finalized=False))
        assert actual["error"]["code"]==expected["error"] and "stages" not in actual
        assert actual["error"]["stage"] == python_refusal(book, finalized=False)[1]


def test_gaps_difference_is_recorded_even_when_caught(tmp_path,monkeypatch):
    from tests import custody_engine_compare as harness
    from kassiber.core import custody_journal
    from tests.custody_gap_generator import snapshot
    records=tmp_path/"gap-records.jsonl"
    monkeypatch.setenv(harness.RECORDS_ENV,str(records))
    monkeypatch.setenv(harness.INJECT_ENV,"gaps")
    monkeypatch.setattr(custody_journal,"_build_observer_factory",None)
    class Builder:
        conn=profile=None
        def load_snapshot(self):return snapshot(0)
    monkeypatch.setattr(harness,"_INSTALLED_MODE","census")
    with pytest.raises(AssertionError,match="gaps"):
        harness._build(Builder(),finalized=False)
    assert harness.session_report(harness.read_records(records))[1]==["custody differences: 1"]


def test_adversarial_gap_search_is_bounded_in_both_engines():
    from kassiber.core.custody_gaps import search_custody_gap_candidates
    from tests.custody_gap_vectors import row
    # Nearest amount buckets and eras precede the source. Both the amount scan
    # and era scan must stop at their ceilings instead of walking every bucket
    # for every source. This is search-only; interpretation has separate budgets.
    rows=[row(f"past-{i}",inbound=True,amount=1_000_000) for i in range(60_000)]
    rows.extend(row(f"source-{i}",occurred_at="2022-01-01T00:00:00Z",privacy_boundary="coinjoin") for i in range(300))
    rows.append(row("future",inbound=True,amount=990_000,occurred_at="2023-01-01T00:00:00Z"))
    started=time.monotonic()
    result=search_custody_gap_candidates(rows)
    assert time.monotonic()-started<20
    assert not result.search_complete and len(result.blocking_source_ids)==300


def test_gap_stage_uses_the_sql_selected_worklist(custody_book,monkeypatch):
    from kassiber.core import custody_gaps
    from kassiber.core.custody_snapshot import load_custody_snapshot
    conn,helper=custody_book
    for i in range(300):
        helper._tx(conn,tx_id=f"ordinary-{i:03}",wallet_id="wallet-a",direction="outbound",amount=1000,external_id=f"ordinary-{i:03}",raw_json="{}")
    for identifier,direction,amount in (("large-source","outbound",1_000_000),("typed-source","outbound",2_000_000),("large-return","inbound",990_000),("typed-return","inbound",1_980_000)):
        helper._tx(conn,tx_id=identifier,wallet_id="wallet-a" if direction=="outbound" else "wallet-b",direction=direction,amount=amount,external_id=identifier,raw_json="{}")
        conn.execute("UPDATE transactions SET occurred_at=? WHERE id=?",("2020-01-01T00:00:00Z" if direction=="outbound" else "2021-01-01T00:00:00Z",identifier))
    conn.execute("UPDATE transactions SET kind='whirlpool_tx0' WHERE id='typed-source'")
    monkeypatch.setattr(custody_gaps,"DEFAULT_MAX_INPUT_ROWS",5)
    book=load_custody_snapshot(conn,_profile(conn),as_of="2026-10-29T00:00:00Z")
    worklist=book["gap_worklist"]
    assert worklist["large_book"] and len(worklist["rows"])==67
    assert {"large-source","typed-source","large-return","typed-return"}<={r["id"] for r in worklist["rows"]}
    assert len(book["transactions"])==304
    conn.set_authorizer(lambda *_:1)
    expected=python_stage(book)
    actual,_=rust_results(book, finalized=False)
    assert_same(expected,actual["gaps"],"SQL-selected gap worklist")
