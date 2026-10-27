#!/usr/bin/env python3
"""Prove custody's unified serde_json features preserve every ledger output byte."""
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[3]


def check():
    command = ["cargo", "run", "-p", "kassiber-tax-core", "--example", "ledger_feature_contract", "--locked", "--quiet"]
    baseline = subprocess.run(command, cwd=ROOT, check=True, capture_output=True).stdout
    unified = subprocess.run([*command, "--features", "serde_json/arbitrary_precision,serde_json/float_roundtrip"], cwd=ROOT, check=True, capture_output=True).stdout
    assert baseline == unified, "serde_json features change ledger bytes"
    count = len(baseline.splitlines())
    assert count >= 100
    print(f"ledger: {count} outputs byte-identical with and without custody serde_json features")


if __name__ == "__main__":
    check()
