"""Frozen isolated interpreter outputs computed only by the Python reference."""
from copy import deepcopy
import json
from pathlib import Path
import sys

from tests.custody_interpretation_partial import python_results
from tests.custody_snapshot_generator import component_snapshot, snapshot, SCENARIOS

PATH = Path(__file__).parent / "fixtures/custody_interpretation_cases.json"


def books():
    yield "shared_aliases", snapshot(SCENARIOS.index("duplicate_slot"))
    yield "shared_reservations", component_snapshot(0)
    book = deepcopy(snapshot(0))
    book["transactions"][0]["journal_transaction_id"] = "original-anchor"
    yield "shared_anchor_alias", book
    from tests.custody_interpretation_generator import SAMOURAI_SCENARIOS, samourai_snapshot
    for seed, case in enumerate(SAMOURAI_SCENARIOS):
        yield "samourai_" + case, samourai_snapshot(seed, case=case)
    yield "samourai_float_overflow", samourai_snapshot(0, case="float_overflow")


def vectors():
    return [{"case": case, "input": book, "partial": python_results(book)} for case, book in books()]


if __name__ == "__main__":
    text = json.dumps(vectors(), indent=2, ensure_ascii=True, sort_keys=True) + "\n"
    if "--write" in sys.argv:
        PATH.write_text(text)
    else:
        assert PATH.read_text() == text
