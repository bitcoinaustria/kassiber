"""Partial parity must remain separate from the product stage inventory."""
import json
from collections import Counter

import kassiber_tax
from tests.custody_interpretation_partial import compare
from tests.custody_snapshot_generator import snapshot, SCENARIOS
from tests.custody_interpretation_activity import assert_scenario_activity


def test_shared_compilation_aliases_and_product_skip():
    book = snapshot(SCENARIOS.index("duplicate_slot"))
    assert compare(book) == ("fanout", "recorded", "samourai", "shared")
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
        counts["scenario:" + SAMOURAI_SCENARIOS[seed % len(SAMOURAI_SCENARIOS)] + ":" + ("active" if active else "blocked" if blocked else "inactive")] += 1
        if seed < len(SAMOURAI_SCENARIOS) * 2:
            compare(permuted_snapshot(book))
    assert counts["active"] > 1000 and counts["blocked"] > 200, counts
    assert all(counts["scenario:" + name] >= 58 for name in SAMOURAI_SCENARIOS)
    assert_scenario_activity(counts, "samourai", SAMOURAI_SCENARIOS)
    print("Samourai generator: " + json.dumps(dict(sorted(counts.items())), sort_keys=True))


def test_frozen_partial_vectors_and_python_text_table():
    from tests.custody_interpretation_vectors import PATH, vectors
    from tests.custody_interpretation_text import PATH as TEXT_PATH, source
    assert vectors() == json.loads(PATH.read_text())
    assert TEXT_PATH.read_text() == source()
    from tests.custody_pair_vectors import PATH as PAIR_PATH, vectors as pair_vectors
    assert pair_vectors() == json.loads(PAIR_PATH.read_text())
    from tests.custody_transfer_vectors import PATH as TRANSFER_PATH, vectors as transfer_vectors
    assert transfer_vectors() == json.loads(TRANSFER_PATH.read_text())
    from tests.custody_fanout_vectors import PATH as FANOUT_PATH, vectors as fanout_vectors
    assert fanout_vectors() == json.loads(FANOUT_PATH.read_text())


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


def test_recorded_generated_active_blocked_and_permuted_paths():
    from tests.custody_interpretation_generator import RECORDED_SCENARIOS, recorded_snapshot
    from tests.custody_interpretation_partial import python_results
    from tests.custody_engine_compare import permuted_snapshot
    counts = Counter()
    for seed in range(2000):
        book = recorded_snapshot(seed)
        compare(book)
        result = python_results(book)["recorded"]
        blocked = bool(result["blocked_transaction_ids"])
        active = any(claim["target"] is not None for claim in result["claims"]) and not blocked
        counts["active" if active else "blocked" if blocked else "inactive"] += 1
        counts["scenario:" + RECORDED_SCENARIOS[seed % len(RECORDED_SCENARIOS)]] += 1
        counts["scenario:" + RECORDED_SCENARIOS[seed % len(RECORDED_SCENARIOS)] + ":" + ("active" if active else "blocked" if blocked else "inactive")] += 1
        if seed < len(RECORDED_SCENARIOS) * 2:
            compare(permuted_snapshot(book))
    assert counts["active"] > 500 and counts["blocked"] > 100 and counts["inactive"] > 500, counts
    assert all(counts["scenario:" + name] >= 43 for name in RECORDED_SCENARIOS)
    assert_scenario_activity(counts, "recorded", RECORDED_SCENARIOS)
    print("Recorded generator: " + json.dumps(dict(sorted(counts.items())), sort_keys=True))


def test_fanout_generated_active_blocked_and_permuted_paths():
    from tests.custody_fanout_generator import FANOUT_SCENARIOS, fanout_snapshot
    from tests.custody_interpretation_partial import python_results
    from tests.custody_engine_compare import permuted_snapshot
    counts = Counter()
    for seed in range(2000):
        book = fanout_snapshot(seed)
        compare(book)
        result = python_results(book)["fanout"]
        state = "blocked" if result["blocked_transaction_ids"] else "active" if any(claim["target"] is not None for claim in result["claims"]) else "inactive"
        counts[state] += 1
        case = FANOUT_SCENARIOS[seed % len(FANOUT_SCENARIOS)]
        counts[f"scenario:{case}:{state}"] += 1
        if seed < len(FANOUT_SCENARIOS) * 2:
            compare(permuted_snapshot(book))
    assert_scenario_activity(counts, "fanout", FANOUT_SCENARIOS)
    print("Fan-out generator: " + json.dumps(dict(sorted(counts.items())), sort_keys=True))
