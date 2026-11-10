"""The frozen exchange withdrawal books still give Python's frozen result.

crates/custody/tests/exchange_txid.rs compares the Rust engine with the same
frozen projections, so together they pin Rust-vs-Python parity.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from kassiber.core.custody_compute import compute_custody_projection
from tests.custody_snapshot import to_jsonable

FIXTURE = Path(__file__).parent / "fixtures/custody_exchange_txid_stage.json"
CASES = json.loads(FIXTURE.read_text())


@pytest.mark.parametrize("case", CASES, ids=[case["case"] for case in CASES])
def test_python_reproduces_the_frozen_exchange_txid_projection(case):
    result = compute_custody_projection(case["input"]).finalized_tax_projection
    assert to_jsonable(result, redact_config=False) == case["expected"]
    assert ("exchange_txid_exact" in json.dumps(case["expected"])) == case["case"].endswith(
        "_exact"
    )
