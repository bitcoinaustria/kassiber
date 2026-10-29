"""Quantity primitive and complete runtime parity against Python."""
from collections import Counter
from copy import deepcopy
import json
from pathlib import Path
import subprocess

import pytest

from tests.custody_quantity_generator import SCENARIOS,scenario,signature
from tests.custody_quantity_vectors import PATH,vectors


@pytest.fixture(scope="module")
def primitive_driver():
    built=subprocess.run(["cargo","build","--offline","--locked","-p","kassiber-custody","--example","quantity-primitives","--message-format=json"],capture_output=True,text=True,check=True)
    binary=next(record["executable"] for line in built.stdout.splitlines() if (record:=json.loads(line)).get("executable"))
    def invoke(inputs):
        result=subprocess.run([binary],input="".join(json.dumps(v)+"\n" for v in inputs),capture_output=True,text=True,check=True)
        rows=[json.loads(line) for line in result.stdout.splitlines()]
        assert len(rows)==len(inputs)
        return rows
    return invoke


def test_frozen_quantity_primitives_are_python_outputs():
    assert json.loads(PATH.read_text())==vectors()


def test_generated_quantity_primitives_count_both_engines_and_permutations(primitive_driver):
    expected_counts=json.loads(Path("tests/fixtures/custody_quantity_primitive_counts.json").read_text())
    counts={engine:{case:Counter() for case in SCENARIOS} for engine in ("Python","Rust")}
    inputs=[];expected=[];cases=[]
    for seed in range(2000):
        case,value,result=scenario(seed)
        counts["Python"][case][signature(result)]+=1
        cases.append(case);inputs.append(value);expected.append({"expected":result})
    actual=primitive_driver(inputs)
    assert actual==expected
    for case,result in zip(cases,actual):counts["Rust"][case][signature(result["expected"])]+=1
    for engine,results in counts.items():
        assert results==expected_counts,engine
        for index,case in enumerate(SCENARIOS):assert sum(results[case].values())==2000//len(SCENARIOS)+(index<2000%len(SCENARIOS))
    shuffled=deepcopy(inputs)
    for value in shuffled:value["observations"].reverse();value["claims"].reverse()
    assert primitive_driver(shuffled)==actual
    print("quantity primitive per-scenario counts: "+json.dumps(counts,sort_keys=True))


def test_recorded_quantity_primitives_replay_and_permute(primitive_driver):
    frozen=json.loads(PATH.read_text())
    actual=primitive_driver(frozen["project"])
    assert actual==[{key:v[key] for key in ("expected","error") if key in v} for v in frozen["project"]]
    # Preserve admission order for malformed observations; only successful
    # arbitration has an order-independent permutation contract.
    values=[deepcopy(v) for v in frozen["project"] if "expected" in v]
    for value in values:value["observations"].reverse();value["claims"].reverse()
    assert primitive_driver(values)==[{"expected":v["expected"]} for v in values]


def test_quantity_stage_compares_and_finalized_projection_stays_skipped():
    from tests.custody_gap_generator import snapshot
    from tests.custody_engine_compare import rust_results
    stages,skipped=rust_results(snapshot(0))
    assert set(stages)=={"components","evidence","interpretation","gaps","quantity"}
    assert skipped==("finalized_projection",)


def test_quantity_transfer_vectors_and_active_generated_counts(primitive_driver):
    from tests.custody_quantity_transfers import PATH, SCENARIOS, scenario, vectors
    assert json.loads(PATH.read_text()) == vectors()
    inputs = []
    expected = []
    cases = []
    counts = {engine: Counter() for engine in ("Python", "Rust")}
    for seed in range(1200):
        case, value, result, active = scenario(seed)
        assert active > 0
        counts["Python"][case] += active
        cases.append((case, active))
        inputs.append(value)
        expected.append({"expected": result})
    actual = primitive_driver(inputs)
    assert actual == expected
    for (case, active), result in zip(cases, actual):
        assert len(result["expected"]) == active
        counts["Rust"][case] += len(result["expected"])
    expected_counts = {case: 100 * (4 if case in {"slice", "sort"} else 1 if case in {"retained", "external"} else 2) for case in SCENARIOS}
    assert counts["Python"] == counts["Rust"] == expected_counts
    shuffled = deepcopy(inputs)
    for value in shuffled:
        value["projection"]["observations"].reverse()
        value["projection"]["decisions"].reverse()
        value["eligible"].reverse()
    assert primitive_driver(shuffled) == expected


def test_quantity_composition_frozen_python_outputs():
    from tests.custody_quantity_composition_vectors import PATH, vectors
    assert json.loads(PATH.read_text()) == vectors()


def test_quantity_composition_generated_active_counts_both_engines(primitive_driver):
    from tests.custody_quantity_composition_generator import COUNTS, activity, scenario, scenarios
    counts = {engine: Counter() for engine in ('Python', 'Rust')}
    values = []
    expected = []
    names = []
    for i in range(len(scenarios())):
        for variant in range(20):
            name, operands, results, active = scenario(i, variant)
            assert active > 0
            values.extend(operands)
            expected.extend(results)
            names.append(name)
            counts['Python'][name] += active
    actual = primitive_driver(values)
    assert actual == expected
    for i, name in enumerate(names):
        active = sum(activity(v) for v in actual[2*i:2*i+2])
        assert active > 0
        counts['Rust'][name] += active
    assert counts['Python'] == counts['Rust'] == json.loads(COUNTS.read_text())
    assert len(counts['Python']) == len(scenarios())

    # Re-evaluate reversed operands independently: malformed admission order
    # may change a refusal, while every positive companion remains active.
    from tests.custody_quantity_composition_generator import evaluate
    permuted = deepcopy(values)
    for value in permuted:
        if value['op'] == 'reviewed':
            value['observations'].reverse()
            for field in ('legs', 'allocations', 'economic_terms'):
                value['component'].get(field, []).reverse()
        else:
            value['audit'].reverse()
            value['canonical']['events'].reverse()
            for event in value['canonical']['events']:
                for field in ('legs', 'evidence_snapshots', 'observation_aliases'):
                    event[field].reverse()
    assert primitive_driver(permuted) == [evaluate(v) for v in permuted]


def test_quantity_runtime_frozen_outputs():
    from tests.custody_quantity_runtime_vectors import PATH, vectors
    assert json.loads(PATH.read_text()) == vectors()


def test_quantity_runtime_generated_active_counts_both_engines(primitive_driver):
    from tests.custody_quantity_runtime_generator import COUNTS, activity, scenario, scenarios
    counts = {engine: Counter() for engine in ('Python', 'Rust')}
    values=[]; expected=[]; names=[]
    for i in range(len(scenarios())):
        for variant in range(20):
            name,operands,results,active=scenario(i,variant)
            assert active>0
            values.extend(operands);expected.extend(results);names.append(name)
            counts['Python'][name]+=active
    actual=primitive_driver(values)
    assert actual==expected
    for i,name in enumerate(names):
        active=sum(activity(v) for v in actual[2*i:2*i+2])
        assert active>0
        counts['Rust'][name]+=active
    assert counts['Python']==counts['Rust']==json.loads(COUNTS.read_text())
    assert len(counts['Python'])==len(scenarios())
    from tests.custody_quantity_runtime_vectors import evaluate
    permuted=deepcopy(values)
    for v in permuted:
        v['canonical']['events'].reverse()
        for event in v['canonical']['events']:
            for field in ('legs','observation_aliases','evidence_snapshots'):
                event[field].reverse()
        if v['op']=='tax_eligibility':
            v['projection']['decisions'].reverse()
        elif v['op'] in ('runtime','component_runtime'):
            v['components'].reverse()
    assert primitive_driver(permuted)==[evaluate(v) for v in permuted]


def test_quantity_runtime_wrappers_are_python_outputs():
    from tests.custody_quantity_stage import wrappers
    assert json.loads(Path('tests/fixtures/custody_quantity_wrappers.json').read_text())==wrappers()
