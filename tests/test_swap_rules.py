"""Pure-function tests for the swap rules engine."""

import json
import unittest

from kassiber.core.swap_rules import (
    DEFAULT_MIN_CONFIDENCE,
    PatternSuggestion,
    SwapMatchingRule,
    apply_rules,
    detect_repeating_patterns,
    load_rule,
    predicate_matches,
    rule_specificity,
)
from kassiber.core.transfer_matching import (
    CONFIDENCE_EXACT,
    CONFIDENCE_STRONG,
    SwapCandidate,
)


def _candidate(**overrides):
    base = {
        "out_id": "o",
        "in_id": "i",
        "out_asset": "BTC",
        "in_asset": "LBTC",
        "out_amount_msat": 100_000_000_000,
        "in_amount_msat": 99_500_000_000,
        "out_wallet_id": "phoenix",
        "in_wallet_id": "liquid",
        "out_wallet_label": "Phoenix",
        "in_wallet_label": "Liquid",
        "out_wallet_kind": "phoenix",
        "in_wallet_kind": "descriptor",
        "out_occurred_at": "2026-03-14T17:30:00Z",
        "in_occurred_at": "2026-03-14T17:32:00Z",
        "confidence": CONFIDENCE_STRONG,
        "method": "heuristic",
        "swap_fee_msat": 500_000_000,
        "swap_fee_kind": "combined",
        "default_kind": "submarine-swap",
        "default_policy": "carrying-value",
        "conflict_set_id": "cluster-1",
    }
    base.update(overrides)
    return SwapCandidate(**base)


def _rule(**overrides):
    base = {
        "id": "rule-1",
        "profile_id": "prof",
        "name": "Phoenix to Liquid",
        "predicate": {},
        "kind": "submarine-swap",
        "policy": "carrying-value",
        "enabled": True,
    }
    base.update(overrides)
    return SwapMatchingRule(**base)


class LoadRuleTests(unittest.TestCase):
    def test_decodes_typical_record(self):
        record = {
            "id": "r1",
            "profile_id": "p1",
            "name": "Phoenix→Liquid",
            "predicate_json": json.dumps({"out_wallet_kind": "phoenix", "in_asset": "LBTC"}),
            "kind": "submarine-swap",
            "policy": "carrying-value",
            "enabled": 1,
        }
        rule = load_rule(record)
        self.assertEqual(rule.id, "r1")
        self.assertEqual(rule.predicate, {"out_wallet_kind": "phoenix", "in_asset": "LBTC"})
        self.assertTrue(rule.enabled)

    def test_broken_predicate_decodes_to_empty(self):
        record = {"id": "r1", "predicate_json": "{not json", "kind": "manual", "policy": "taxable"}
        self.assertEqual(load_rule(record).predicate, {})

    def test_disabled_record(self):
        record = {"id": "r1", "predicate_json": "{}", "kind": "manual", "policy": "taxable", "enabled": 0}
        self.assertFalse(load_rule(record).enabled)


class PredicateMatchesTests(unittest.TestCase):
    def test_empty_predicate_matches_everything(self):
        self.assertTrue(predicate_matches(_candidate(), {}))

    def test_wallet_id_predicate(self):
        predicate = {"out_wallet_id": "phoenix", "in_wallet_id": "liquid"}
        self.assertTrue(predicate_matches(_candidate(), predicate))
        self.assertFalse(predicate_matches(_candidate(out_wallet_id="other"), predicate))

    def test_asset_predicate(self):
        predicate = {"out_asset": "BTC", "in_asset": "LBTC"}
        self.assertTrue(predicate_matches(_candidate(), predicate))
        self.assertFalse(predicate_matches(_candidate(out_asset="LBTC"), predicate))

    def test_wallet_kind_predicate(self):
        predicate = {"out_wallet_kind": "phoenix"}
        self.assertTrue(predicate_matches(_candidate(), predicate))
        self.assertFalse(predicate_matches(_candidate(out_wallet_kind="lnd"), predicate))

    def test_max_fee_pct_cap_admits_under(self):
        # 500_000_000 / 100_000_000_000 = 0.005 (0.5%)
        self.assertTrue(predicate_matches(_candidate(), {"max_fee_pct": 0.01}))

    def test_max_fee_pct_cap_rejects_over(self):
        self.assertFalse(predicate_matches(_candidate(), {"max_fee_pct": 0.001}))

    def test_min_confidence_strong_admits_exact(self):
        self.assertTrue(
            predicate_matches(
                _candidate(confidence=CONFIDENCE_EXACT),
                {"min_confidence": CONFIDENCE_STRONG},
            )
        )

    def test_min_confidence_exact_rejects_strong(self):
        self.assertFalse(
            predicate_matches(
                _candidate(confidence=CONFIDENCE_STRONG),
                {"min_confidence": CONFIDENCE_EXACT},
            )
        )


class SpecificityTests(unittest.TestCase):
    def test_more_keys_higher_specificity(self):
        empty = _rule(predicate={})
        narrow = _rule(predicate={"out_wallet_id": "a", "in_wallet_id": "b"})
        self.assertGreater(rule_specificity(narrow), rule_specificity(empty))


class ApplyRulesTests(unittest.TestCase):
    def test_disabled_rule_skipped(self):
        candidates = [_candidate()]
        rules = [_rule(enabled=False)]
        auto, remaining = apply_rules(candidates, rules)
        self.assertEqual(auto, [])
        self.assertEqual(remaining, candidates)

    def test_more_specific_rule_wins(self):
        candidates = [_candidate()]
        rules = [
            _rule(id="generic", predicate={}, kind="manual"),
            _rule(id="specific", predicate={"out_wallet_kind": "phoenix"}, kind="submarine-swap"),
        ]
        auto, _ = apply_rules(candidates, rules)
        self.assertEqual(len(auto), 1)
        self.assertEqual(auto[0].rule_id, "specific")

    def test_conflict_cluster_blocks_auto_pair(self):
        candidates = [
            _candidate(out_id="o1", in_id="i1", conflict_set_id="cluster-X", conflict_size=2),
            _candidate(out_id="o1", in_id="i2", conflict_set_id="cluster-X", conflict_size=2),
        ]
        rules = [_rule(predicate={})]
        auto, remaining = apply_rules(candidates, rules)
        self.assertEqual(auto, [])
        self.assertEqual(len(remaining), 2)

    def test_conflict_size_blocks_auto_pair_even_when_siblings_filtered_out(self):
        # The caller may pass a filtered candidate list (e.g. swap-only view)
        # that hides the cluster sibling. The stamped conflict_size still
        # blocks auto-pairing.
        candidates = [
            _candidate(out_id="o1", in_id="i1", conflict_set_id="cluster-X", conflict_size=2),
        ]
        rules = [_rule(predicate={})]
        auto, remaining = apply_rules(candidates, rules)
        self.assertEqual(auto, [])
        self.assertEqual(len(remaining), 1)

    def test_min_confidence_filter_via_rule(self):
        # Strong candidate, rule requires exact → no match.
        candidates = [_candidate(confidence=CONFIDENCE_STRONG, conflict_set_id="solo-strong")]
        rules = [_rule(predicate={"min_confidence": CONFIDENCE_EXACT})]
        auto, remaining = apply_rules(candidates, rules)
        self.assertEqual(auto, [])
        self.assertEqual(remaining, candidates)

    def test_default_min_confidence_admits_strong(self):
        candidates = [_candidate(confidence=CONFIDENCE_STRONG, conflict_set_id="solo")]
        rules = [_rule(predicate={})]
        auto, _ = apply_rules(candidates, rules)
        self.assertEqual(len(auto), 1)

    def test_default_min_confidence_value(self):
        # Documents the constant so a future change is intentional.
        self.assertEqual(DEFAULT_MIN_CONFIDENCE, CONFIDENCE_STRONG)


class DetectRepeatingPatternsTests(unittest.TestCase):
    def _history_row(self, *, source="manual", **overrides):
        base = {
            "out_wallet_id": "phoenix",
            "in_wallet_id": "liquid",
            "out_asset": "BTC",
            "in_asset": "LBTC",
            "kind": "submarine-swap",
            "policy": "carrying-value",
            "pair_source": source,
        }
        base.update(overrides)
        return base

    def test_repeated_pattern_surfaces(self):
        history = [self._history_row() for _ in range(3)]
        suggestions = detect_repeating_patterns(history)
        self.assertEqual(len(suggestions), 1)
        suggestion = suggestions[0]
        self.assertEqual(suggestion.occurrences, 3)
        self.assertEqual(suggestion.kind, "submarine-swap")

    def test_below_threshold_filtered_out(self):
        history = [self._history_row() for _ in range(2)]
        self.assertEqual(detect_repeating_patterns(history), [])

    def test_only_manual_rows_count(self):
        history = [self._history_row(source="manual"), self._history_row(source="rule_auto"), self._history_row(source="bulk_exact")]
        self.assertEqual(detect_repeating_patterns(history, min_occurrences=2), [])

    def test_different_shapes_kept_separate(self):
        history = [
            self._history_row(),
            self._history_row(),
            self._history_row(),
            self._history_row(out_wallet_id="coreln", out_asset="LBTC", in_asset="BTC", kind="peg-out"),
            self._history_row(out_wallet_id="coreln", out_asset="LBTC", in_asset="BTC", kind="peg-out"),
            self._history_row(out_wallet_id="coreln", out_asset="LBTC", in_asset="BTC", kind="peg-out"),
        ]
        suggestions = detect_repeating_patterns(history)
        self.assertEqual(len(suggestions), 2)
        kinds = {s.kind for s in suggestions}
        self.assertEqual(kinds, {"submarine-swap", "peg-out"})

    def test_to_predicate_round_trip(self):
        suggestion = PatternSuggestion(
            out_wallet_id="phoenix",
            in_wallet_id="liquid",
            out_asset="BTC",
            in_asset="LBTC",
            kind="submarine-swap",
            policy="carrying-value",
            occurrences=3,
        )
        self.assertEqual(
            suggestion.to_predicate(),
            {
                "out_wallet_id": "phoenix",
                "in_wallet_id": "liquid",
                "out_asset": "BTC",
                "in_asset": "LBTC",
            },
        )


if __name__ == "__main__":
    unittest.main()
