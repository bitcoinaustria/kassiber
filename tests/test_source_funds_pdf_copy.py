"""Reader-facing PDF copy must distinguish original values from allocations."""
from unittest import TestCase
from unittest.mock import Mock

from kassiber.source_funds_pdf_report import _SourceFundsPdfBuilder


class SourceFundsPdfCopyTest(TestCase):
    def builder(self, report):
        # Capture section contents before layout; keep the real amount renderer.
        builder = object.__new__(_SourceFundsPdfBuilder)
        builder.report = report
        builder.amount_precision = "btc"
        builder.p = Mock(side_effect=lambda text, *args: text)
        builder.table = Mock(side_effect=lambda rows, **kwargs: rows)
        builder.spacer = Mock(return_value=None)
        return builder

    def test_root_source_fiat_is_explicitly_original_not_allocated(self):
        builder = self.builder({"graph": {"nodes": [{
            "node_type": "source", "label": "Purchase", "amount": "0.01",
            "required_amount": "0.006", "asset": "BTC", "fiat_value": "1001",
            "fiat_currency": "EUR", "review_state": "reviewed",
        }]}})
        story = builder.source_mix()
        rows = builder.table.call_args.args[0]
        self.assertEqual(rows[0][3], "Allocated")
        self.assertEqual(rows[0][5], "Original fiat")
        self.assertEqual(rows[1][3], "0.00600000")
        self.assertEqual(rows[1][5], "1.001,00 EUR")
        self.assertIn("Original fiat values cover each full source, not just the allocated amount.", story)
        builder.graph_nodes()
        graph_rows = builder.table.call_args.args[0]
        self.assertEqual(graph_rows[1][3], "BTC")
        self.assertEqual(graph_rows[1][4], "0.00600000")
        self.assertEqual(graph_rows[1][5], "Original source fiat: 1.001,00 EUR")

    def test_checklist_omits_implementation_roadmap(self):
        builder = self.builder({"report_context": {
            "evidence_checklist": ["Purchase evidence is attached."],
            "deferred": ["German localization", "ReportLab renderer", "CoinJoin/PayJoin traversal"],
        }})
        text = str(builder.evidence_checklist())
        self.assertIn("Purchase evidence is attached.", text)
        self.assertNotIn("localization", text)
        self.assertNotIn("ReportLab", text)
        self.assertNotIn("Deferred", text)

    def test_actual_privacy_boundary_limitation_survives_copy_cleanup(self):
        builder = self.builder({"simplified_flow": {"deferred_privacy_hops": [{"id": "link"}]}})
        text = " ".join(builder.limitations())
        self.assertIn("does not trace ownership through them", text)
        self.assertIn("Suggested links and unconfirmed chain observations", text)
        self.assertIn("attested prior-history stops", text)
        builder.report = {}
        self.assertNotIn("CoinJoin", " ".join(builder.limitations()))
