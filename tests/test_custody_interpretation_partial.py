"""Partial parity must remain separate from the product stage inventory."""
import json

import kassiber_tax
from tests.custody_interpretation_partial import compare
from tests.custody_snapshot_generator import snapshot, SCENARIOS


def test_shared_compilation_aliases_and_product_skip():
    book = snapshot(SCENARIOS.index("duplicate_slot"))
    assert compare(book) == ("shared",)
    response = json.loads(kassiber_tax.custody(json.dumps(book)))
    assert "interpretation" in response["skipped_stages"]
    assert "interpretation" not in response["stages"]
