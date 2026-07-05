import unittest

from kassiber._pdf_common import register_fonts, require_reportlab
from kassiber.core.source_funds_diagram import build_flow_drawing


def _line_count(drawing):
    return sum(1 for shape in drawing.contents if type(shape).__name__ == "Line")


def _strings(drawing):
    return [getattr(shape, "text", "") for shape in drawing.contents if type(shape).__name__ == "String"]


class FlowDiagramEdgeTests(unittest.TestCase):
    """Guards the regression where every Sankey edge was silently dropped.

    Levels run target (left) -> root sources (right) and edges point from a
    parent (``from``) to the child it funds (``to``), so ``from`` sits to the
    right of ``to``. The old ``if tx <= fx: continue`` guard skipped every such
    edge, leaving the flow diagram with no connectors, arrowheads, or labels.
    """

    def setUp(self):
        self.rl = require_reportlab("source-funds diagram test")
        self.fonts = register_fonts(self.rl)

    def _flow(self):
        return {
            "levels": [
                {
                    "level": 1,
                    "role": "target",
                    "nodes": [
                        {
                            "id": "t1",
                            "label": "Target sale",
                            "kind": "planned sale",
                            "asset": "BTC",
                            "amount": "1.0",
                            "node_type": "transaction",
                        }
                    ],
                },
                {
                    "level": 2,
                    "role": "source",
                    "nodes": [
                        {
                            "id": "s1",
                            "label": "Coinfinity buy",
                            "kind": "fiat_purchase",
                            "asset": "BTC",
                            "amount": "1.0",
                            "node_type": "source",
                            "source_type": "fiat_purchase",
                        }
                    ],
                },
            ],
            "edges": [
                {
                    "from": "s1",
                    "to": "t1",
                    "percent_of_target": "100.0",
                    "link_type": "self_transfer",
                }
            ],
        }

    def test_reviewed_edge_draws_connector_and_label(self):
        drawing = build_flow_drawing(self.rl, self.fonts, self._flow(), width=400.0)
        # one connector line + two arrowhead strokes for the single reviewed edge
        self.assertGreaterEqual(_line_count(drawing), 3, "flow diagram drew no edge connectors")
        self.assertTrue(
            any("100.0%" in text for text in _strings(drawing)),
            "edge percent-of-target label missing",
        )

    def test_same_column_edge_is_skipped(self):
        flow = self._flow()
        # A degenerate self-referential edge shares a column -> no connector.
        flow["edges"] = [{"from": "s1", "to": "s1", "percent_of_target": "100.0"}]
        drawing = build_flow_drawing(self.rl, self.fonts, flow, width=400.0)
        self.assertEqual(_line_count(drawing), 0)


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
