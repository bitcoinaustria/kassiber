import unittest

from kassiber.cli.termrender import (
    MarkdownStreamRenderer,
    render_envelope_table,
    render_markdown_table,
)


BOLD = "\x1b[1m"
BOLD_OFF = "\x1b[22m"
DIM = "\x1b[2m"
CODE = "\x1b[36m"
CODE_OFF = "\x1b[39m"


def _render(text, chunk_size=None):
    renderer = MarkdownStreamRenderer()
    if chunk_size is None:
        out = renderer.feed(text)
    else:
        out = ""
        for start in range(0, len(text), chunk_size):
            out += renderer.feed(text[start : start + chunk_size])
    return out + renderer.flush()


class MarkdownStreamRendererTest(unittest.TestCase):
    def test_chunking_never_changes_output(self):
        document = (
            "# Totals\n"
            "Your **largest** outbound was `1.5 BTC`.\n"
            "\n"
            "- first item\n"
            "- second *item\n"
            "> note about fees\n"
            "\n"
            "| asset | amount |\n"
            "| --- | --- |\n"
            "| BTC | 1.5 |\n"
            "| BTC | 0.25 |\n"
            "\n"
            "```python\n"
            "x = 1\n"
            "```\n"
            "---\n"
            "done **bold across\nlines** end\n"
        )
        whole = _render(document)
        for chunk_size in (1, 2, 3, 7, 64):
            self.assertEqual(
                _render(document, chunk_size), whole, f"chunk_size={chunk_size}"
            )

    def test_bold_and_inline_code(self):
        out = _render("a **b** `c` d\n")
        self.assertEqual(out, f"a {BOLD}b{BOLD_OFF} {CODE}c{CODE_OFF} d\n")

    def test_single_stars_stay_literal(self):
        out = _render("2*3 equals 6* maybe\n")
        self.assertEqual(out, "2*3 equals 6* maybe\n")

    def test_header_renders_bold_without_hashes(self):
        out = _render("## Summary\nplain\n")
        self.assertEqual(out, f"{BOLD}Summary{BOLD_OFF}\nplain\n")

    def test_bullets_and_blockquote(self):
        out = _render("- one\n  - nested\n> hint\n")
        self.assertIn("• one\n", out)
        self.assertIn("  • nested\n", out)
        self.assertIn(f"{DIM}┃{BOLD_OFF} hint\n", out)

    def test_fence_suppresses_markers_and_info_string(self):
        out = _render("```python\nvalue = '**not bold**'\n```\nafter\n")
        self.assertIn(f"  {CODE}value = '**not bold**'{CODE_OFF}\n", out)
        self.assertNotIn("python", out.replace("value", ""))
        self.assertNotIn(BOLD + "not bold", out)
        self.assertTrue(out.endswith("after\n"))

    def test_horizontal_rule(self):
        out = _render("---\n")
        self.assertIn("─" * 8, out)

    def test_pipe_table_renders_boxed_and_aligned(self):
        out = _render(
            "| asset | amount |\n"
            "| --- | ---: |\n"
            "| BTC | 1.5 |\n"
            "after\n"
        )
        self.assertIn("│", out)
        self.assertNotIn("---", out)
        self.assertIn(f"{BOLD}asset", out)
        self.assertIn("BTC", out)
        self.assertTrue(out.endswith("after\n"))

    def test_unterminated_table_renders_on_flush(self):
        out = _render("| a | b |\n| --- | --- |\n| 1 | 2 |")
        self.assertIn("│", out)
        self.assertIn("1", out)

    def test_open_styles_close_at_newline(self):
        out = _render("**unclosed\nnext\n")
        self.assertEqual(out, f"{BOLD}unclosed{BOLD_OFF}\nnext\n")


class RenderMarkdownTableTest(unittest.TestCase):
    def test_inline_markers_inside_cells(self):
        out = render_markdown_table(
            ["| name | value |", "| --- | --- |", "| **BTC** | `1.5` |"]
        )
        self.assertIn(f"{BOLD}BTC{BOLD_OFF}", out)
        self.assertIn(f"{CODE}1.5{CODE_OFF}", out)


class RenderEnvelopeTableTest(unittest.TestCase):
    def _transactions_envelope(self, count=3):
        rows = [
            {
                "id": f"tx-{index}",
                "occurred_at": f"2026-01-0{index + 1}T00:00:00Z",
                "direction": "outbound",
                "asset": "BTC",
                "amount": 0.5 + index,
                "amount_msat": 50_000_000_000,
                "wallet": "Cold storage",
                "metadata": {"nested": True},
            }
            for index in range(count)
        ]
        return {
            "kind": "ui.transactions.list",
            "schema_version": 1,
            "data": {"transactions": rows, "next_cursor": None},
        }

    def test_picks_priority_columns_and_skips_noise(self):
        table = render_envelope_table(self._transactions_envelope(), terminal_width=120)
        self.assertIsNotNone(table)
        self.assertIn("occurred_at", table)
        self.assertIn("direction", table)
        self.assertIn("Cold storage", table)
        self.assertNotIn("amount_msat", table)
        self.assertNotIn("tx-0", table)
        self.assertNotIn("metadata", table)

    def test_caps_rows_with_more_indicator(self):
        table = render_envelope_table(
            self._transactions_envelope(count=12), max_rows=8, terminal_width=120
        )
        self.assertIn("… 4 more rows", table)

    def test_scalar_payload_returns_none(self):
        envelope = {"kind": "status", "data": {"ok": True, "version": "0.1"}}
        self.assertIsNone(render_envelope_table(envelope))

    def test_respects_width_budget(self):
        table = render_envelope_table(self._transactions_envelope(), terminal_width=44)
        self.assertIsNotNone(table)
        for line in table.splitlines():
            visible = (
                line.replace(BOLD, "")
                .replace(BOLD_OFF, "")
                .replace(DIM, "")
                .replace(CODE, "")
                .replace(CODE_OFF, "")
            )
            self.assertLessEqual(len(visible), 60)


if __name__ == "__main__":
    unittest.main()
