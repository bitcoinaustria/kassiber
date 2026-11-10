"""Competing invalid assets refuse the same way under every hash seed.

`check_configuration()` once validated assets in set iteration order, so the
first refused asset depended on PYTHONHASHSEED. It now validates them sorted,
which the Rust fusion builder can reproduce from its operands.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import sys


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


def test_competing_asset_refusals_do_not_depend_on_the_hash_seed(tmp_path):
    outcomes = []
    for seed in range(8):
        result = subprocess.run(
            [sys.executable, "-c", REPRO], cwd=ROOT,
            env={**os.environ, "PYTHONHASHSEED": str(seed),
                 "XDG_DATA_HOME": str(tmp_path / str(seed)),
                 "KASSIBER_NO_EGRESS": "1"},
            capture_output=True, text=True, check=True, timeout=30,
        )
        outcomes.append(json.loads(result.stdout))
    assert outcomes[0] == {
        "family": "AppError",
        "message": "RP2 configuration asset contains unsupported characters",
        "code": "validation",
        "details": {"field": "asset", "value": "BTC#", "characters": ["#"]},
        "hint": (
            "Rename the wallet, profile, or asset label to remove commas, "
            "newlines, brackets, comments, or key delimiters before processing reports."
        ),
        "retryable": False,
    }
    assert all(outcome == outcomes[0] for outcome in outcomes)
