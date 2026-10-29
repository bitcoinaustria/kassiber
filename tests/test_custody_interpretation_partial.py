"""Partial parity must remain separate from the product stage inventory."""
import json
from collections import Counter

import kassiber_tax
from tests.custody_interpretation_partial import compare
from tests.custody_snapshot_generator import snapshot, SCENARIOS
from tests.custody_interpretation_activity import assert_scenario_activity


def test_shared_compilation_aliases_and_product_skip():
    book = snapshot(SCENARIOS.index("duplicate_slot"))
    assert compare(book) == ("native", "channels", "safety", "graph", "pipeline", "consolidation", "ownership", "fanout", "recorded", "samourai", "shared")
    response = json.loads(kassiber_tax.custody(json.dumps(book)))
    assert "interpretation" in response["skipped_stages"]
    assert "interpretation" not in response["stages"]


def test_samourai_generated_active_blocked_and_permuted_paths():
    from tests.custody_interpretation_generator import SAMOURAI_SCENARIOS, samourai_snapshot
    from tests.custody_interpretation_partial import python_results
    from tests.custody_engine_compare import permuted_snapshot
    counts = Counter()
    for seed in range(2000):
        book = samourai_snapshot(seed)
        compare(book)
        result = python_results(book)["samourai"]
        blocked = bool(result["blocked_transaction_ids"])
        active = any(claim["target"] is not None for claim in result["claims"]) and not blocked
        counts["active" if active else "blocked" if blocked else "inactive"] += 1
        counts["scenario:" + SAMOURAI_SCENARIOS[seed % len(SAMOURAI_SCENARIOS)]] += 1
        counts["scenario:" + SAMOURAI_SCENARIOS[seed % len(SAMOURAI_SCENARIOS)] + ":" + ("active" if active else "blocked" if blocked else "inactive")] += 1
        if seed < len(SAMOURAI_SCENARIOS) * 2:
            compare(permuted_snapshot(book))
    assert counts["active"] > 1000 and counts["blocked"] > 200, counts
    assert all(counts["scenario:" + name] >= 58 for name in SAMOURAI_SCENARIOS)
    assert_scenario_activity(counts, "samourai", SAMOURAI_SCENARIOS)
    print("Samourai generator: " + json.dumps(dict(sorted(counts.items())), sort_keys=True))


def test_frozen_partial_vectors_and_python_text_table():
    from tests.custody_pipeline_vectors import PATH as PIPELINE_PATH, vectors as pipeline_vectors, helper_vectors as pipeline_helpers
    assert pipeline_vectors() == json.loads(PIPELINE_PATH.read_text())
    assert pipeline_helpers() == json.loads(PIPELINE_PATH.with_name("custody_pipeline_helpers.json").read_text())
    from tests.custody_consolidation_vectors import PATH as CONSOLIDATION_PATH, vectors as consolidation_vectors, helper_vectors as consolidation_helpers
    assert consolidation_vectors() == json.loads(CONSOLIDATION_PATH.read_text())
    assert consolidation_helpers() == json.loads(CONSOLIDATION_PATH.with_name("custody_consolidation_helpers.json").read_text())
    from tests.custody_ownership_vectors import PATH as OWNERSHIP_PATH, vectors as ownership_vectors, helper_vectors
    assert ownership_vectors() == json.loads(OWNERSHIP_PATH.read_text())
    assert helper_vectors() == json.loads(OWNERSHIP_PATH.with_name("custody_ownership_helpers.json").read_text())
    from tests.custody_liquid_vectors import PATH as LIQUID_PATH, vectors as liquid_vectors
    assert liquid_vectors() == json.loads(LIQUID_PATH.read_text())
    from tests.custody_graph_vectors import PATH as GRAPH_PATH, vectors as graph_vectors
    from tests.custody_snapshot import to_jsonable
    assert to_jsonable(graph_vectors(), redact_config=False) == json.loads(GRAPH_PATH.read_text())
    from tests.custody_interpretation_vectors import PATH, vectors
    from tests.custody_interpretation_text import PATH as TEXT_PATH, source
    assert vectors() == json.loads(PATH.read_text())
    assert TEXT_PATH.read_text() == source()
    from tests.custody_pair_vectors import PATH as PAIR_PATH, vectors as pair_vectors
    assert pair_vectors() == json.loads(PAIR_PATH.read_text())
    from tests.custody_transfer_vectors import PATH as TRANSFER_PATH, vectors as transfer_vectors
    assert transfer_vectors() == json.loads(TRANSFER_PATH.read_text())
    from tests.custody_fanout_vectors import PATH as FANOUT_PATH, vectors as fanout_vectors
    assert fanout_vectors() == json.loads(FANOUT_PATH.read_text())


def test_partial_discrepancy_is_recorded_even_when_caught(tmp_path, monkeypatch):
    from tests import custody_engine_compare as harness
    from tests import custody_interpretation_partial as partial
    from kassiber.core import custody_journal
    from tests.custody_snapshot_generator import templates
    records = tmp_path / "partial-records.jsonl"
    monkeypatch.setenv(harness.RECORDS_ENV, str(records))
    monkeypatch.setattr(partial, "rust_results", lambda snapshot: {"injected": True})
    monkeypatch.setattr(custody_journal, "_build_observer_factory", None)
    # Use the existing builder fixture through its narrow snapshot seam.
    class Builder:
        conn = profile = None
        def load_snapshot(self): return templates()[0]
    monkeypatch.setattr(harness, "_INSTALLED_MODE", "census")
    try:
        harness._build(Builder(), finalized=False)
    except AssertionError:
        pass
    else:
        raise AssertionError("partial mismatch was swallowed")
    assert harness.session_report(harness.read_records(records))[1] == ["custody differences: 1"]


def test_recorded_generated_active_blocked_and_permuted_paths():
    from tests.custody_interpretation_generator import RECORDED_SCENARIOS, recorded_snapshot
    from tests.custody_interpretation_partial import python_results
    from tests.custody_engine_compare import permuted_snapshot
    counts = Counter()
    for seed in range(2000):
        book = recorded_snapshot(seed)
        compare(book)
        result = python_results(book)["recorded"]
        blocked = bool(result["blocked_transaction_ids"])
        active = any(claim["target"] is not None for claim in result["claims"]) and not blocked
        counts["active" if active else "blocked" if blocked else "inactive"] += 1
        counts["scenario:" + RECORDED_SCENARIOS[seed % len(RECORDED_SCENARIOS)]] += 1
        counts["scenario:" + RECORDED_SCENARIOS[seed % len(RECORDED_SCENARIOS)] + ":" + ("active" if active else "blocked" if blocked else "inactive")] += 1
        if seed < len(RECORDED_SCENARIOS) * 2:
            compare(permuted_snapshot(book))
    assert counts["active"] > 500 and counts["blocked"] > 100 and counts["inactive"] > 500, counts
    assert all(counts["scenario:" + name] >= 43 for name in RECORDED_SCENARIOS)
    assert_scenario_activity(counts, "recorded", RECORDED_SCENARIOS)
    print("Recorded generator: " + json.dumps(dict(sorted(counts.items())), sort_keys=True))


def test_fanout_generated_active_blocked_and_permuted_paths():
    from tests.custody_fanout_generator import FANOUT_SCENARIOS, fanout_snapshot
    from tests.custody_interpretation_partial import python_results
    from tests.custody_engine_compare import permuted_snapshot
    counts = Counter()
    for seed in range(2000):
        book = fanout_snapshot(seed)
        compare(book)
        result = python_results(book)["fanout"]
        state = "blocked" if result["blocked_transaction_ids"] else "active" if any(claim["target"] is not None for claim in result["claims"]) else "inactive"
        counts[state] += 1
        case = FANOUT_SCENARIOS[seed % len(FANOUT_SCENARIOS)]
        counts[f"scenario:{case}:{state}"] += 1
        if seed < len(FANOUT_SCENARIOS) * 2:
            compare(permuted_snapshot(book))
    assert_scenario_activity(counts, "fanout", FANOUT_SCENARIOS)
    print("Fan-out generator: " + json.dumps(dict(sorted(counts.items())), sort_keys=True))


def test_graph_generated_successful_reference_ownership_and_consolidation():
    from kassiber.core.custody_compute import compute_custody_decisions
    from kassiber.core.custody_snapshot import decode_owned_index
    from kassiber.core.ownership_transfers import derive_profile_transfers
    from tests.custody_graph_generator import GRAPH_SCENARIOS, graph_snapshot
    from tests.custody_engine_compare import permuted_snapshot
    counts=Counter()
    ownership_counts = {producer: Counter() for producer in ("Python", "Rust")}
    from tests.custody_interpretation_partial import python_results, rust_results
    for seed in range(2000):
        case=GRAPH_SCENARIOS[seed%len(GRAPH_SCENARIOS)]
        book=graph_snapshot(seed)
        compare(book)
        derived=derive_profile_transfers(book["transactions"],index=decode_owned_index(book["owned_index"]),wallet_refs_by_id={w["id"]:w for w in book["wallets"]})
        expected=derived.consolidation if case.endswith("consolidation") else derived.ownership
        assert not expected.blocked_sources and len(expected.derived_pairs)==(2 if case.endswith("consolidation") else 1),(seed,case,expected)
        stages={}
        compute_custody_decisions(book,stage_observer=lambda name,value:stages.setdefault(name,value))
        interpretation=stages["interpretation"]
        assert not interpretation.blocked_transaction_ids and not interpretation.quarantines,(seed,case)
        if case.endswith("consolidation"):
            assert len(interpretation.claims)==2 and all(claim.target is not None and claim.reason=="multi_source_consolidation" for claim in interpretation.claims),(seed,case)
        else:
            # A rowless owned destination is carried through the native audit
            # path; it has no imported target quantity to claim against.
            assert len(interpretation.native_audits)==1,(seed,case)
        counts[case]+=1
        for producer, result in (("Python", python_results(book)), ("Rust", rust_results(book))):
            result = result["consolidation" if case.endswith("consolidation") else "ownership"]
            assert "error" not in result and not result["derivation"]["blocked_sources"], (seed, case, producer, result)
            assert len(result["derivation"]["derived_pairs"]) == (2 if case.endswith("consolidation") else 1), (seed, case, producer, result)
            assert len(result["native_audits"]) == (0 if case.endswith("consolidation") else 1) and not result["blocked_transaction_ids"] and not result["quarantines"], (seed, case, producer, result)
            if case.endswith("consolidation"):
                assert len(result["claims"]) == 2 and all(claim["target"] is not None for claim in result["claims"]), (seed, case, producer, result)
            ownership_counts[producer][case] += 1
        if seed<len(GRAPH_SCENARIOS)*2:compare(permuted_snapshot(book))
    assert counts==Counter({case:250 for case in GRAPH_SCENARIOS})
    expected_ownership = Counter({case:250 for case in GRAPH_SCENARIOS})
    assert ownership_counts["Python"] == ownership_counts["Rust"] == expected_ownership
    print("Graph reference pipeline producer counts: " + json.dumps(dict(sorted(counts.items()))))
    print("Isolated graph producer counts: " + json.dumps(ownership_counts, sort_keys=True))


def test_composed_pipeline_generated_per_scenario_producer_controls():
    from tests.custody_pipeline_generator import PIPELINE_SCENARIOS,pipeline_snapshot
    from tests.custody_interpretation_partial import python_results,rust_results
    from tests.custody_engine_compare import assert_same,permuted_snapshot
    active={
        'bitcoin_consolidation':(2,0,0),'liquid_consolidation':(2,0,0),
        'bitcoin_fanout':(0,2,0),'liquid_fanout':(0,2,0),
        'bitcoin_ownership':(0,0,1),'liquid_ownership':(0,0,1),
        'bitcoin_mixed':(2,2,1),'liquid_mixed':(2,2,1),
        'bitcoin_normalized':(2,0,0),'bitcoin_collaborative':(2,0,0),
        'bitcoin_residual':(0,0,1),'bitcoin_blocked':(0,0,0),
    }
    counts={engine:{case:Counter() for case in PIPELINE_SCENARIOS} for engine in ('Python','Rust')}
    for seed in range(2000):
        case=PIPELINE_SCENARIOS[seed%len(PIPELINE_SCENARIOS)];book=pipeline_snapshot(seed)
        expected,actual=python_results(book),rust_results(book)
        assert_same(expected,actual,'composed pipeline')
        for engine,result in (('Python',expected['pipeline']),('Rust',actual['pipeline'])):
            assert 'error' not in result,(seed,case,engine,result)
            assert tuple(len(result['derivation'][name]['derived_pairs']) for name in ('consolidation','fanout','ownership'))==active[case],(seed,case,engine)
            blocked=case=='bitcoin_blocked'
            assert len(result['derivation_quarantines'])==(2 if blocked else 0),(seed,case,engine,result)
            assert not result['blocked_transaction_ids'] and not result['quarantines'],(seed,case,engine,result)
            for name,number in zip(('consolidation','fanout','ownership'),active[case]):
                counts[engine][case][name]+=int(bool(number))
            counts[engine][case]['blocked']+=int(blocked)
            if not blocked:
                assert len(result['claims'])==active[case][0]+active[case][1],(seed,case,engine,result)
                assert len(result['native_audits'])==active[case][2],(seed,case,engine,result)
        if seed<len(PIPELINE_SCENARIOS)*2:
            compare(permuted_snapshot(book))
            from kassiber.core.custody_compute import compute_custody_decisions
            from tests.custody_snapshot import to_jsonable
            stages={}
            compute_custody_decisions(book,stage_observer=lambda name,value:stages.setdefault(name,value))
            full=to_jsonable(stages['interpretation'],redact_config=False)
            assert_same(full['claims'],expected['pipeline']['claims'],'reference composed claims')
            assert_same(full['native_audits'],expected['pipeline']['native_audits'],'reference composed audits')
    assert counts['Python']==counts['Rust']
    for index,case in enumerate(PIPELINE_SCENARIOS):
        number=2000//len(PIPELINE_SCENARIOS)+(index<2000%len(PIPELINE_SCENARIOS))
        for name,amount in zip(('consolidation','fanout','ownership'),active[case]):
            assert counts['Python'][case][name]==(number if amount else 0),(case,name,counts)
        assert counts['Python'][case]['blocked']==(number if case=='bitcoin_blocked' else 0)
    print('Composed pipeline successes by engine/scenario/producer: '+json.dumps(counts,sort_keys=True))
