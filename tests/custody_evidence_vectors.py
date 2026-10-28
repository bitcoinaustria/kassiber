"""Synthetic CPython evidence-stage baselines used by Rust mutation tests."""
import json
from pathlib import Path
import sys
from kassiber.core.chain_observer.provenance import canonical_graph_hash
from kassiber.core.custody_compute import compute_evidence_stage
from tests.custody_snapshot import to_jsonable
from tests.custody_snapshot_generator import SCENARIOS, snapshot

PATH = Path(__file__).parent / "fixtures/custody_evidence_cases.json"
CASES = ("duplicate_fee_keeper", "receipt_success", "receipt_deprecated", "receipt_priority", "receipt_label", "receipt_id", "duplicate_slot", "tampered_graph", "tampered_quantity", "historical_overcount", "nonaddress_overcount", "shared_fee", "zero_delta")


def vectors():
    cases = [{"case": case, "input": (book := snapshot(SCENARIOS.index(case))), "evidence": to_jsonable(compute_evidence_stage(book), redact_config=False)} for case in CASES]
    # Complete stored graphs may contain quantities above machine integer
    # ranges. A receipt mismatch must hold, without panicking or rounding.
    book = snapshot(SCENARIOS.index("receipt_success"))
    for row in book["transactions"]:
        raw = json.loads(row["raw_json"])
        raw["vout"][-1]["value_sats"] = 10**100
        row["raw_json"] = json.dumps(raw)
        row["observation_graph_hash"] = canonical_graph_hash(row["raw_json"])
    evidence = to_jsonable(compute_evidence_stage(book), redact_config=False)
    assert evidence["receipt_reconciliations"] and all(v is None for v in evidence["receipt_reconciliations"].values())
    cases.append({"case": "receipt_unbounded_graph", "input": book, "evidence": evidence})
    from tests.custody_graph_vectors import vectors as graph_vectors
    for vector in graph_vectors():
        if vector["group"]=="arithmetic_fees" and vector["case"] in ("huge_cancel","shape","float_fee","false_fee","nan_fee"):
            book=vector["input"]
            evidence=to_jsonable(compute_evidence_stage(book),redact_config=False)
            normalized=all(row.get("custody_native_fee_normalized") for row in evidence["enriched_rows"])
            assert normalized==(vector["case"] in ("huge_cancel","float_fee","false_fee")),vector["case"]
            cases.append({"case":"owned_fee_"+vector["case"],"input":book,"evidence":evidence})
    return cases


if __name__ == "__main__":
    text = json.dumps(vectors(), indent=2, ensure_ascii=True, sort_keys=True) + "\n"
    if "--write" in sys.argv:
        PATH.write_text(text)
    else:
        assert PATH.read_text() == text
