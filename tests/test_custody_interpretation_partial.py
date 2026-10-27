"""Partial parity must remain separate from the product stage inventory."""
import json
from collections import Counter

import kassiber_tax
from tests.custody_interpretation_partial import compare
from tests.custody_snapshot_generator import snapshot, SCENARIOS


def test_shared_compilation_aliases_and_product_skip():
    book = snapshot(SCENARIOS.index("duplicate_slot"))
    assert compare(book) == ("samourai", "shared")
    response = json.loads(kassiber_tax.custody(json.dumps(book)))
    assert "interpretation" in response["skipped_stages"]
    assert "interpretation" not in response["stages"]


def test_samourai_generated_active_blocked_and_permuted_paths():
    from tests.custody_interpretation_generator import SAMOURAI_SCENARIOS, samourai_snapshot
    from tests.custody_interpretation_partial import python_results
    from tests.custody_engine_compare import permuted_snapshot
    counts = Counter()
    for seed in range(2000):
        book = samourai_snapshot(seed)
        compare(book)
        result = python_results(book)["samourai"]
        blocked = bool(result["blocked_transaction_ids"])
        active = any(claim["target"] is not None for claim in result["claims"]) and not blocked
        counts["active" if active else "blocked" if blocked else "inactive"] += 1
        counts["scenario:" + SAMOURAI_SCENARIOS[seed % len(SAMOURAI_SCENARIOS)]] += 1
        if seed < len(SAMOURAI_SCENARIOS) * 2:
            compare(permuted_snapshot(book))
    assert counts["active"] > 1000 and counts["blocked"] > 200, counts
    assert all(counts["scenario:" + name] >= 58 for name in SAMOURAI_SCENARIOS)
    print("Samourai generator: " + json.dumps(dict(sorted(counts.items())), sort_keys=True))


def test_frozen_partial_vectors_and_python_text_table():
    from tests.custody_interpretation_vectors import PATH, vectors
    from tests.custody_interpretation_text import PATH as TEXT_PATH, source
    assert vectors() == json.loads(PATH.read_text())
    assert TEXT_PATH.read_text() == source()


def test_partial_discrepancy_is_recorded_even_when_caught(tmp_path, monkeypatch):
    from tests import custody_engine_compare as harness
    from tests import custody_interpretation_partial as partial
    from kassiber.core import custody_journal
    from tests.custody_snapshot_generator import templates
    records = tmp_path / "partial-records.jsonl"
    monkeypatch.setenv(harness.RECORDS_ENV, str(records))
    monkeypatch.setattr(partial, "rust_results", lambda snapshot: {"injected": True})
    monkeypatch.setattr(custody_journal, "_build_observer_factory", None)
    # Use the existing builder fixture through its narrow snapshot seam.
    class Builder:
        conn = profile = None
        def load_snapshot(self): return templates()[0]
    monkeypatch.setattr(harness, "_INSTALLED_MODE", "census")
    try:
        harness._build(Builder(), finalized=False)
    except AssertionError:
        pass
    else:
        raise AssertionError("partial mismatch was swallowed")
    assert harness.session_report(harness.read_records(records))[1] == ["custody differences: 1"]
