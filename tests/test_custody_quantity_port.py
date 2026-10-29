"""Quantity primitive parity; the complete runtime stage remains unimplemented."""
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


def test_quantity_stage_stays_skipped_until_runtime_composition_is_complete():
    from tests.custody_gap_generator import snapshot
    from tests.custody_engine_compare import rust_results
    stages,skipped=rust_results(snapshot(0))
    assert set(stages)=={"components","evidence","interpretation","gaps"}
    assert skipped==("quantity","finalized_projection")
