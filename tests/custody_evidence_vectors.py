"""Synthetic CPython evidence-stage baselines used by Rust mutation tests."""
import json
from pathlib import Path
import sys
from kassiber.core.custody_compute import compute_evidence_stage
from tests.custody_snapshot import to_jsonable
from tests.custody_snapshot_generator import SCENARIOS, snapshot

PATH = Path(__file__).parent / "fixtures/custody_evidence_cases.json"
CASES = ("duplicate_fee_keeper", "receipt_success", "receipt_deprecated", "receipt_priority", "receipt_label", "receipt_id", "duplicate_slot", "tampered_graph", "tampered_quantity", "historical_overcount", "nonaddress_overcount", "shared_fee", "zero_delta")


def vectors():
    return [{"case": case, "input": (book := snapshot(SCENARIOS.index(case))), "evidence": to_jsonable(compute_evidence_stage(book), redact_config=False)} for case in CASES]


if __name__ == "__main__":
    text = json.dumps(vectors(), indent=2, ensure_ascii=True, sort_keys=True) + "\n"
    if "--write" in sys.argv:
        PATH.write_text(text)
    else:
        assert PATH.read_text() == text
