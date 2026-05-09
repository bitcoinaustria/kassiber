import unittest

from kassiber.core import source_funds_hints
from kassiber.core.source_funds_hints import (
    enrich_findings_with_next_steps,
    hint_action_names,
    hint_for_code,
    known_finding_codes,
)


class HintForCodeTests(unittest.TestCase):
    def test_known_code_returns_full_hint(self):
        hint = hint_for_code("missing_history")
        self.assertEqual(hint["headline"], "Attach a root source or attest the gap")
        self.assertEqual(hint["action"], "open_source_creator")
        self.assertEqual(hint["action_args"], {"source_type": "missing_history"})
        self.assertEqual(hint["doc_anchor"], "missing-history")

    def test_unknown_code_returns_generic_hint(self):
        hint = hint_for_code("not_a_real_code")
        self.assertIn("headline", hint)
        self.assertEqual(hint["action"], "open_review_queue")
        self.assertEqual(hint["doc_anchor"], "findings")

    def test_returns_independent_dict_per_call(self):
        a = hint_for_code("path_cycle")
        b = hint_for_code("path_cycle")
        a["headline"] = "MUTATED"
        self.assertNotEqual(a["headline"], b["headline"])

    def test_action_names_are_finite_set(self):
        names = hint_action_names()
        self.assertIn("open_source_creator", names)
        self.assertIn("open_link_review", names)
        self.assertIn("open_review_queue", names)
        self.assertEqual(names, tuple(sorted(set(names))))


class EnrichFindingsTests(unittest.TestCase):
    def test_attaches_next_step_to_each_finding(self):
        findings = [
            {"code": "missing_history", "severity": "blocker", "message": "x", "ref": ""},
            {"code": "asset_mismatch", "severity": "blocker", "message": "y", "ref": "L1"},
        ]
        enriched = enrich_findings_with_next_steps(findings)
        self.assertIs(enriched, findings)
        self.assertEqual(enriched[0]["next_step"]["action"], "open_source_creator")
        self.assertEqual(enriched[1]["next_step"]["action"], "open_link_review")

    def test_does_not_overwrite_existing_next_step(self):
        custom = {"headline": "custom", "action": "custom_action", "action_args": {}, "doc_anchor": "x"}
        findings = [
            {"code": "missing_history", "severity": "blocker", "message": "x", "ref": "", "next_step": custom},
        ]
        enrich_findings_with_next_steps(findings)
        self.assertEqual(findings[0]["next_step"], custom)

    def test_unknown_code_still_gets_a_hint(self):
        findings = [{"code": "future_code", "severity": "warning", "message": "z", "ref": ""}]
        enrich_findings_with_next_steps(findings)
        self.assertIn("headline", findings[0]["next_step"])

    def test_handles_missing_code(self):
        findings = [{"severity": "warning", "message": "z", "ref": ""}]
        enrich_findings_with_next_steps(findings)
        self.assertIn("next_step", findings[0])


class HintCoverageTests(unittest.TestCase):
    """Pin the catalog. If a new finding code is added without a hint, this test
    flags it so the UI/CLI doesn't ship raw codes to users."""

    EXPECTED_CODES = {
        "missing_history",
        "missing_pricing",
        "asset_mismatch",
        "source_asset_mismatch",
        "transaction_overallocation",
        "source_overallocation",
        "path_truncated",
        "path_cycle",
        "unreviewed_link",
        "ambiguous_allocation",
        "unconfirmed_chain_data",
        "chain_observation_privacy",
        "privacy_hop_unresolved",
        "chronology_violation",
        "opening_balance_attestation",
    }

    def test_every_emitted_code_has_a_hint(self):
        catalog = set(known_finding_codes())
        missing = self.EXPECTED_CODES - catalog
        self.assertFalse(missing, f"Codes without hints: {sorted(missing)}")

    def test_module_re_export(self):
        self.assertTrue(hasattr(source_funds_hints, "enrich_findings_with_next_steps"))


if __name__ == "__main__":
    unittest.main()
