"""Gap oracle stops before quantity arbitration; Rust never receives verdicts."""
from kassiber.core.custody_compute import compute_custody_decisions


def python_stages(snapshot):
    class Complete(Exception):
        pass
    stages = {}
    def observe(name, value):
        stages[name] = value
        if name == "gaps":
            raise Complete
    try:
        compute_custody_decisions(snapshot, stage_observer=observe)
    except Complete:
        return stages
    raise AssertionError("Python did not emit gaps")


def python_stage(snapshot):
    return python_stages(snapshot)["gaps"]


def suites():
    from tests.custody_interpretation_stage import suites as previous
    from tests.custody_gap_generator import SCENARIOS,snapshot
    return (*previous(),("gaps",SCENARIOS,snapshot))


if __name__ == "__main__":
    from collections import Counter
    import json
    from pathlib import Path
    from tests.custody_gap_generator import signature
    matrix={}
    for suite,scenarios,generate in suites():
        counts={case:Counter() for case in scenarios}
        for seed in range(2000):
            counts[scenarios[seed%len(scenarios)]][signature(python_stage(generate(seed)))]+=1
        matrix[suite]=counts
        print(suite,flush=True)
    Path("tests/fixtures/custody_gap_counts.json").write_text(json.dumps(matrix,sort_keys=True,indent=2)+"\n")
