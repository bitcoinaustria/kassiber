"""Seeded raw books compare the R2 evidence stage over varied custody shapes."""
import json

from kassiber.core.custody_compute import compute_evidence_stage, compute_custody_projection
from tests.custody_engine_compare import compare_rust, assert_same
from tests.custody_snapshot_generator import SCENARIOS, snapshot, permuted, templates


def test_two_thousand_generated_snapshots_match_rust():
    for seed in range(2000):
        book = snapshot(seed)
        compared, skipped = compare_rust(book, compute_evidence_stage(book))
        assert compared[:2] == ("components", "evidence"), (seed, compared, skipped)
        assert "interpretation" in compared
        if seed < len(SCENARIOS) * 2:
            shuffled = permuted(book)
            compare_rust(shuffled, compute_evidence_stage(shuffled))


def test_generated_snapshots_are_deterministic_and_run_every_python_stage():
    # Rebuild the authored template too: its original UUIDs and timestamps
    # must not make the seed depend on the process-local template cache.
    authored_seed = SCENARIOS.index("excluded_anchor")
    expected = json.dumps(snapshot(authored_seed), sort_keys=True)
    templates.cache_clear()
    assert json.dumps(snapshot(authored_seed), sort_keys=True) == expected
    for seed in range(len(SCENARIOS) * 2):
        book = snapshot(seed)
        assert json.dumps(book, sort_keys=True) == json.dumps(snapshot(seed), sort_keys=True)
        stages = {}
        original = compute_custody_projection(book, stage_observer=lambda name, value: stages.setdefault(name, value))
        if seed == SCENARIOS.index("channel"):
            assert set(original.channel_roles) == {"funding", "closing"}
        if seed == SCENARIOS.index("gap"):
            assert stages["gaps"].candidates
        shuffled = compute_custody_projection(permuted(book))
        for field in ("active_components", "interpretation", "quantity_state", "custody_transfers", "finalized_tax_projection", "rates"):
            assert_same(getattr(original, field), getattr(shuffled, field), f"unordered reads, seed {seed}, {field}")


def test_latest_rate_ties_preserve_the_observed_database_order():
    book = snapshot(0)
    book["rate_rows"] = [
        {"asset": "BTC", "fiat_rate": None, "fiat_value": None, "amount": 1, "fiat_rate_exact": "100.0", "fiat_value_exact": None},
        {"asset": "BTC", "fiat_rate": None, "fiat_value": None, "amount": 1, "fiat_rate_exact": "200.0", "fiat_value_exact": None},
    ]
    first = compute_custody_projection(book)
    book["rate_rows"].reverse()
    second = compute_custody_projection(book)
    assert str(first.rates["BTC"]) == "100.0"
    assert str(second.rates["BTC"]) == "200.0"
    compare_rust(book, compute_evidence_stage(book))


def test_generated_corpus_exercises_receipt_success_duplicates_and_tampering():
    from collections import Counter
    from kassiber.core.custody_evidence import canonical_event_key
    counts = Counter()
    for seed in range(2000):
        book = snapshot(seed)
        stage = compute_evidence_stage(book)
        counts["reconciled_books"] += any(value is not None for value in stage["receipt_reconciliations"].values())
        counts["reconciled_receipts"] += sum(value is not None for value in stage["receipt_reconciliations"].values())
        slots = Counter()
        for row in book["transactions"]:
            try:
                key = canonical_event_key(row)
            except ValueError:
                continue
            slots[(key, row["wallet_id"], row["direction"], row["asset"])] += 1
        counts["duplicate_slot_books"] += any(count > 1 for count in slots.values())
        counts["duplicate_slots"] += sum(count > 1 for count in slots.values())
        for event in stage["canonical_input"].events:
            for leg in event.legs:
                counts[leg.observation_authority_reason] += 1
    assert counts["reconciled_books"] >= 300, counts
    assert counts["duplicate_slot_books"] >= 100, counts
    assert counts["graph_hash_mismatch"] >= 60, counts
    assert counts["quantity_hash_mismatch"] >= 60, counts
    print(dict(counts))


def test_checked_in_evidence_cases_and_read_input_permutations():
    from tests.custody_evidence_vectors import vectors, PATH
    expected = json.loads(PATH.read_text())
    assert vectors() == expected
    for case in expected:
        book = case["input"]
        shuffled = permuted(book)
        stage = compute_evidence_stage(book)
        shuffled_stage = compute_evidence_stage(shuffled)
        assert_same(stage, shuffled_stage, case["case"])
        compare_rust(shuffled, shuffled_stage)
    receipt = next(case["input"] for case in expected if case["case"] == "receipt_id")
    shuffled = permuted(receipt)
    assert receipt["receipt_sources"] != shuffled["receipt_sources"]
    assert receipt["owned_index"]["by_script"] != shuffled["owned_index"]["by_script"]
    assert len({wallet["label"] for wallet in receipt["wallets"]}) < len(receipt["wallets"])
