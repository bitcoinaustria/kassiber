#!/usr/bin/env python3
"""Python reference bug: str.isdigit accepts digits that int cannot parse.

The R4 Rust port preserves this refusal; Python production is left unchanged.
"""
from pathlib import Path
import sys
ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core.transfer_matching import suggest_swap_candidates
from tests.test_transfer_matching import _row
rows = [
    _row(id='o', wallet_id='source', raw_json=dict(provider='boltz', swap_id='repro', flow='chain', send_amount_msat='²')),
    _row(id='i', wallet_id='target', direction='inbound'),
]
try:
    suggest_swap_candidates(rows)
except ValueError as error:
    assert str(error) == "invalid literal for int() with base 10: '²'"
    print(f'{type(error).__name__}: {error}')
else:
    raise AssertionError('reference no longer reproduces')
