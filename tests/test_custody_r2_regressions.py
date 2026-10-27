"""Admission and parsing regressions at the raw R2 seam."""
import json
from pathlib import Path

import pytest
import kassiber_tax
from kassiber.core.custody_compute import compute_custody_decisions, compute_evidence_stage
from tests.custody_engine_compare import compare_rust
from tests.custody_snapshot_generator import snapshot
from tests.custody_semantic_vectors import vectors


def test_cpython_semantic_vectors_are_current():
    assert json.loads((Path(__file__).parent / "fixtures/custody_semantics.json").read_text()) == vectors()


@pytest.mark.parametrize("gate", ["mixed", "conflicted", "binding", "country"])
def test_rust_refuses_the_same_accounting_gates(gate):
    book = snapshot(0)
    book["book_network"] = {"binding": None, "observations": [], "references": []}
    if gate == "country":
        book["profile"]["tax_country"] = "unsupported"
    elif gate == "binding":
        book["book_network"]["binding"] = {"environment": "regtest", "chain_instance_id": None}
        book["wallets"][0]["config_json"] = '{"network":"main"}'
    else:
        book["wallets"][0]["config_json"] = '{"network":"main"}'
        book["wallets"][1]["config_json"] = '{"network":"regtest"}' if gate == "mixed" else '{"network":"Testnet4"}'
    with pytest.raises(Exception) as caught:
        compute_custody_decisions(book)
    response = json.loads(kassiber_tax.custody(json.dumps(book)))
    expected_code = getattr(caught.value, "code", type(caught.value).__name__)
    assert response.get("error", {}).get("code") == expected_code
    assert "stages" not in response


@pytest.mark.parametrize("field", ["amount", "fee"])
def test_real_msat_is_an_explicit_rust_refusal(field):
    book = snapshot(0)
    book["transactions"][0][field] = 1.5
    assert compute_evidence_stage(book)
    response = json.loads(kassiber_tax.custody(json.dumps(book)))
    assert response["error"]["code"] == "custody_snapshot_invalid"
    assert f"{field} must be an exact signed-64 msat integer" in response["error"]["message"]


@pytest.mark.parametrize("part", ["snapshot", "component", "worklist"])
def test_unknown_request_fields_are_refused(part):
    book = snapshot(0)
    if part == "component":
        book["components"] = [{"header": {}, "legs": [], "allocations": [], "economic_terms": [], "commitments": [], "surprise": True}]
    else:
        (book if part == "snapshot" else book["gap_worklist"])["surprise"] = True
    response = json.loads(kassiber_tax.custody(json.dumps(book)))
    assert response.get("error", {}).get("code") == "custody_snapshot_invalid"
