"""Pure-function tests for the swap rules engine."""

import json
import unittest

from kassiber.core.swap_rules import (
    DEFAULT_MIN_CONFIDENCE,
    SwapMatchingRule,
    apply_rules,
    load_rule,
    predicate_matches,
    rule_specificity,
    validate_rule_predicate,
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

    def test_broken_predicate_disables_rule(self):
        record = {"id": "r1", "predicate_json": "{not json", "kind": "manual", "policy": "taxable"}
        rule = load_rule(record)
        self.assertEqual(rule.predicate, {})
        self.assertFalse(rule.enabled)
        self.assertEqual(rule.invalid_reason, "predicate JSON is invalid")

    def test_disabled_record(self):
        record = {"id": "r1", "predicate_json": "{}", "kind": "manual", "policy": "taxable", "enabled": 0}
        self.assertFalse(load_rule(record).enabled)

    def test_string_disabled_records(self):
        for enabled in ("0", "false", "no", "off"):
            with self.subTest(enabled=enabled):
                record = {
                    "id": "r1",
                    "predicate_json": "{}",
                    "kind": "manual",
                    "policy": "taxable",
                    "enabled": enabled,
                }
                self.assertFalse(load_rule(record).enabled)

    def test_string_enabled_record(self):
        record = {"id": "r1", "predicate_json": "{}", "kind": "manual", "policy": "taxable", "enabled": "true"}
        self.assertTrue(load_rule(record).enabled)

    def test_missing_enabled_defaults_to_enabled(self):
        record = {"id": "r1", "predicate_json": "{}", "kind": "manual", "policy": "taxable"}
        self.assertTrue(load_rule(record).enabled)

    def test_unknown_predicate_field_disables_rule(self):
        record = {
            "id": "r1",
            "predicate_json": json.dumps({"unknown": "value"}),
            "kind": "manual",
            "policy": "taxable",
        }
        rule = load_rule(record)
        self.assertFalse(rule.enabled)
        self.assertEqual(rule.invalid_reason, "predicate has unsupported field(s): unknown")

    def test_invalid_kind_and_policy_disable_rule(self):
        record = {
            "id": "r1",
            "predicate_json": "{}",
            "kind": "bogus",
            "policy": "wrong",
        }
        rule = load_rule(record)
        self.assertFalse(rule.enabled)
        self.assertEqual(rule.kind, "manual")
        self.assertEqual(rule.policy, "carrying-value")
        self.assertEqual(
            rule.invalid_reason,
            "unsupported kind 'bogus'; unsupported policy 'wrong'",
        )


class PredicateMatchesTests(unittest.TestCase):
    def test_empty_predicate_matches_everything(self):
        self.assertTrue(predicate_matches(_candidate(), {}))

    def test_wallet_id_predicate(self):
        predicate = {"out_wallet_id": "phoenix", "in_wallet_id": "liquid"}
        self.assertTrue(predicate_matches(_candidate(), predicate))
        self.assertFalse(predicate_matches(_candidate(out_wallet_id="other"), predicate))

    def test_unknown_field_fails_closed(self):
        self.assertFalse(predicate_matches(_candidate(), {"unknown": "value"}))

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

    def test_boolean_max_fee_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "must be a number"):
            validate_rule_predicate({"max_fee_pct": True})

    def test_invalid_max_fee_pct_fails_closed(self):
        self.assertFalse(predicate_matches(_candidate(), {"max_fee_pct": "nan"}))

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

    def test_loaded_string_disabled_rule_skipped(self):
        candidates = [_candidate()]
        loaded_rule = load_rule(
            {
                "id": "rule-1",
                "profile_id": "prof",
                "predicate_json": "{}",
                "kind": "submarine-swap",
                "policy": "carrying-value",
                "enabled": "false",
            }
        )
        auto, remaining = apply_rules(candidates, [loaded_rule])
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


if __name__ == "__main__":
    unittest.main()
