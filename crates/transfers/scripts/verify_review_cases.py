#!/usr/bin/env python3
"""Compare complete native builds and live-connection native case inspection.

No Python quarantines or native-build outputs are supplied to the case reader.
The build driver independently pins every field of the complete journal state.
"""
import argparse
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[3]


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False)


def invoke(binary, requests):
    result = subprocess.run([str(binary)], input="".join(json.dumps(r)+"\n" for r in requests),
                            text=True, capture_output=True, timeout=600, check=True)
    responses = [json.loads(line) for line in result.stdout.splitlines()]
    assert len(responses) == len(requests), result.stderr
    return responses


def verify(build_binary, case_binary):
    fixture = json.loads((ROOT / "crates/transfers/tests/review_cases.json").read_text())
    builds = []
    for scenario in fixture["scenarios"]:
        sql = "\n".join([*fixture["schema"], *scenario["sql"], *fixture["objects"]])
        for encrypted in (False, True):
            for writer in (False, True):
                builds.append({**scenario["build"], "sql": sql, "name": scenario["name"],
                               "encrypted": encrypted, "writer": writer, "scenario": scenario})
    responses = invoke(build_binary, [{k:v for k,v in r.items() if k != "scenario"} for r in builds])
    requests = []
    expected = []
    for request, response in zip(builds, responses):
        assert response == request["expected"], (request["name"], response, request["expected"])
        for transaction in (("read", "autocommit") if not request["writer"] else ("write",)):
            cases=request["scenario"]["cases"]
            requests.append(dict(sql=request["sql"], profile=request["profile"], cases=[case["args"] for case in cases],
                                 encrypted=request["encrypted"], transaction=transaction,
                                 as_of=request["as_of"], entry_ids=request["entry_ids"]))
            expected.append([{key:value for key,value in case.items() if key in ("result","error")}
                             for case in cases])
    responses = invoke(case_binary, requests)
    for index, (actual, wanted) in enumerate(zip(responses, expected)):
        assert canonical(actual) == canonical(wanted), (index, requests[index]["cases"], actual, wanted)
    comparisons=sum(len(group) for group in expected)
    print(json.dumps(dict(builds=len(builds), case_comparisons=comparisons, matched=comparisons)))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("build_binary", type=Path)
    parser.add_argument("case_binary", type=Path)
    args = parser.parse_args()
    verify(args.build_binary.resolve(), args.case_binary.resolve())
