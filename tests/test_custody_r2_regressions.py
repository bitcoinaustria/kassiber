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
    from tests.custody_python_text import PATH, render
    assert PATH.read_text() == render()


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
    assert response["error"]["stage"] == "components"
    assert "stages" not in response


@pytest.mark.parametrize("field", ["amount", "fee"])
def test_real_msat_is_an_explicit_rust_refusal(field):
    book = snapshot(0)
    book["transactions"][0][field] = 1.5
    assert compute_evidence_stage(book)
    response = json.loads(kassiber_tax.custody(json.dumps(book)))
    assert response["error"]["code"] == "custody_snapshot_invalid"
    assert response["error"]["stage"] == "snapshot"
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


def test_component_fact_coalesce_preserves_empty_anchor():
    from kassiber.core.custody_component_facts import ComponentFacts
    header = {"id": "component", "profile_id": "p", "state": "active", "created_at": "now", "revision": 1}
    leg = {"anchor_transaction_id": "", "transaction_id": "tx"}
    facts = ComponentFacts({"components": [{"header": header, "legs": [leg]}], "revision_headers": [], "anchors": {}, "anchor_wallets": []})
    assert facts.read("active_headers", ("p", "tx")) == []
    assert facts.read("active_headers", ("p", "")) == [header]


def test_component_commitments_remain_profile_scoped():
    from kassiber.core.custody_component_facts import ComponentFacts
    component = {"header": {"id": "c"}, "commitments": [{"profile_id": "a"}, {"profile_id": "b"}]}
    facts = ComponentFacts({"components": [component], "revision_headers": [], "anchors": {}, "anchor_wallets": []})
    assert facts.read("commitments", ("c", "a")) == [{"profile_id": "a"}]


def test_raw_review_rows_preserve_concurrent_and_reopened_rules():
    from kassiber.core.custody_gap_reviews import dismissed_fingerprints_from_rows
    base = {"gap_id": "gap", "candidate_fingerprint": "fingerprint", "action": "dismissed", "component_id": None, "event_kind": "review_decision"}
    assert dismissed_fingerprints_from_rows([base]) == {"gap": "fingerprint"}
    assert dismissed_fingerprints_from_rows([base, {**base, "action": "resolved"}]) == {}
    assert dismissed_fingerprints_from_rows([{**base, "event_kind": "bridge_reopened"}]) == {}


def test_rates_support_legacy_columns_and_have_an_id_tiebreak():
    import sqlite3
    from kassiber.core.custody_journal import transaction_rate_rows, latest_transaction_rates_for_profile
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    try:
        conn.execute("CREATE TABLE transactions(id TEXT, profile_id TEXT, excluded INTEGER, occurred_at TEXT, created_at TEXT, asset TEXT, fiat_rate REAL, fiat_value REAL, amount INTEGER)")
        for identifier, rate in [("a", 100), ("z", 200)]:
            conn.execute("INSERT INTO transactions VALUES(?, 'p', 0, 'now', 'now', 'BTC', ?, NULL, 1)", (identifier, rate))
        assert [row["fiat_rate"] for row in transaction_rate_rows(conn, "p")] == [200, 100]
        assert latest_transaction_rates_for_profile(conn, "p")["BTC"] == 200
    finally:
        conn.close()


def test_component_cross_profile_revision_targets_remain_visible():
    from kassiber.core.custody_component_facts import ComponentFacts
    target = {"id": "foreign", "profile_id": "other", "lineage_id": "other", "revision": 1}
    anchor = {"id": "foreign-anchor", "profile_id": "other"}
    facts = ComponentFacts({"components": [], "revision_headers": [target], "anchors": {"foreign-anchor": anchor}, "anchor_wallets": []})
    assert facts.read("header", ("foreign",)) == target
    assert facts.read("anchor", ("foreign-anchor",)) == anchor
    assert facts.read("canonical_anchors", ("current", "foreign-anchor")) == []


def test_custody_mutation_catalog_applies():
    import importlib.util
    path = Path(__file__).resolve().parents[1] / "crates/custody/scripts/mutation_check.py"
    spec = importlib.util.spec_from_file_location("custody_mutation_check", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    assert module.checker.problems(module.checker.load(module.checker.MUTATIONS)) == []
