"""R2g admission blocker: identical raw facts have hash-dependent refusals.

Keep Python unchanged. The fusion compiler cannot derive this first failure
from its specified operands (profile, projection, ordered wallet references).
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import sys

import pytest


ROOT = Path(__file__).resolve().parents[1]
REPRO = r'''
import json
from kassiber.core.custody_tax_projection import FinalizedTaxProjection
from kassiber.core.engines.base import TaxEngineLedgerInputs
from kassiber.core.engines.ledger_request import build_request
from kassiber.errors import AppError

rows = tuple({
    "id": "custody-tax:" + str(index),
    "journal_transaction_id": "anchor-" + str(index),
    "custody_finalized_tax_projection": True,
    "asset": asset,
    "wallet_id": "w",
    "direction": "inbound",
    "amount": 1,
    "fee": 0,
    "occurred_at": "2024-01-01T00:00:00Z",
} for index, asset in enumerate(("BTC#", "ETH:")))
projection = FinalizedTaxProjection(rows, (), (), (), ())
inputs = TaxEngineLedgerInputs(
    finalized_tax_projection=projection,
    wallet_refs_by_id={"w": {"id": "w", "label": "Wallet"}},
)
profile = {"label": "Default", "tax_country": "generic",
           "fiat_currency": "EUR", "gains_algorithm": "FIFO"}
try:
    build_request(profile, inputs)
except AppError as exc:
    print(json.dumps({"family": type(exc).__name__, "message": str(exc),
                      "code": exc.code, "details": exc.details,
                      "hint": exc.hint, "retryable": exc.retryable}, sort_keys=True))
else:
    raise AssertionError("invalid assets were admitted")
'''


def test_fusion_reference_competing_asset_refusals(tmp_path):
    """Characterize the unfixed reference bug, including structured details."""
    outcomes = []
    for seed in (1, 3):
        result = subprocess.run(
            [sys.executable, "-c", REPRO], cwd=ROOT,
            env={**os.environ, "PYTHONHASHSEED": str(seed),
                 "XDG_DATA_HOME": str(tmp_path / str(seed)),
                 "KASSIBER_NO_EGRESS": "1"},
            capture_output=True, text=True, check=True, timeout=30,
        )
        outcomes.append(json.loads(result.stdout))
    for outcome in outcomes:
        assert outcome["family"] == "AppError"
        assert outcome["message"] == "RP2 configuration asset contains unsupported characters"
        assert outcome["code"] == "validation"
        assert outcome["retryable"] is False
        assert outcome["hint"] == (
            "Rename the wallet, profile, or asset label to remove commas, "
            "newlines, brackets, comments, or key delimiters before processing reports."
        )
    assert [outcome["details"] for outcome in outcomes] == [
        {"field": "asset", "value": "ETH:", "characters": [":"]},
        {"field": "asset", "value": "BTC#", "characters": ["#"]},
    ]
    # A first-failure equality check would fail even for Python vs Python.
    with pytest.raises(AssertionError):
        assert outcomes[0] == outcomes[1]
