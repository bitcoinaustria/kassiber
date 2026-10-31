import argparse
import json
from pathlib import Path
import subprocess


def verify(directory, binary):
    records = [json.loads(path.read_text()) for path in sorted(directory.glob("[0-9]*.json"))]
    if not records:
        raise AssertionError("no review builds were captured")
    requests = [{**record, "writer": writer, "encrypted": encrypted}
                for record in records for writer in (False, True) for encrypted in (False, True)]
    child = subprocess.run(
        [str(binary)], input="".join(json.dumps(record) + "\n" for record in requests),
        text=True, capture_output=True, timeout=600, check=True,
    )
    responses = [json.loads(line) for line in child.stdout.splitlines()]
    assert len(responses) == len(requests), child.stderr
    failures = []
    for record, response in zip(requests, responses):
        if response != record["expected"]:
            failures.append({"name": record["name"], "writer": record["writer"],
                             "encrypted": record["encrypted"],
                             "actual": response, "expected": record["expected"]})
    summary = {"builds": len(records), "comparisons": len(requests),
               "matched": len(requests) - len(failures), "failures": len(failures)}
    print(json.dumps(summary))
    if failures:
        path = directory / "comparison-failures.json"
        path.write_text(json.dumps(failures, ensure_ascii=True, indent=2) + "\n")
        raise AssertionError(f"review build comparisons differ: {path}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("directory", type=Path)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    verify(args.directory, args.binary.resolve())
