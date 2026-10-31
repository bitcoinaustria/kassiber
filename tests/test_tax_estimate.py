"""The indicative Austrian figure the assistant quotes instead of a recalled rate."""

from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from kassiber import daemon
from kassiber.core.tax_estimate import austrian_indicative_tax
from kassiber.db import open_db

ROOT = Path(__file__).resolve().parents[1]


def _rows(**amounts: int) -> list[dict]:
    return [
        {"kennzahl": int(code.removeprefix("kz")), "amount_eur_cents": cents}
        for code, cents in amounts.items()
    ]


class AustrianIndicativeTaxTest(unittest.TestCase):
    def test_income_and_gains_net_against_losses_at_the_special_rate(self):
        result = austrian_indicative_tax(
            tax_year=2025, kennzahl_rows=_rows(kz172=10_000, kz174=150_000, kz176=60_000)
        )
        self.assertEqual(result["status"], "indicative")
        self.assertEqual(result["rate"], "0.275")
        self.assertEqual(result["special_rate_base_eur_cents"], 100_000)
        self.assertEqual(result["estimated_tax_eur_cents"], 27_500)
        self.assertEqual(result["progressive_rate_amounts_eur_cents"], {})

    def test_rounds_half_up_to_the_cent(self):
        result = austrian_indicative_tax(tax_year=2025, kennzahl_rows=_rows(kz174=2))
        self.assertEqual(result["estimated_tax_eur_cents"], 1)

    def test_a_net_loss_estimates_zero(self):
        result = austrian_indicative_tax(
            tax_year=2025, kennzahl_rows=_rows(kz174=1_000, kz176=5_000)
        )
        self.assertEqual(result["special_rate_base_eur_cents"], -4_000)
        self.assertEqual(result["estimated_tax_eur_cents"], 0)

    def test_report_blockers_withhold_the_figure(self):
        result = austrian_indicative_tax(
            tax_year=2025,
            kennzahl_rows=_rows(kz174=150_000),
            blocker_ids=["quarantine", "journals_stale", "quarantine"],
        )
        self.assertEqual(result["status"], "withheld")
        self.assertIsNone(result["estimated_tax_eur_cents"])
        self.assertEqual(result["withheld_by"], ["journals_stale", "quarantine"])
        # The base stays visible; only the tax figure is withheld.
        self.assertEqual(result["special_rate_base_eur_cents"], 150_000)

    def test_altbestand_speculation_is_reported_but_never_estimated(self):
        result = austrian_indicative_tax(
            tax_year=2021, kennzahl_rows=_rows(kz174=0, kz801=40_000)
        )
        self.assertEqual(result["estimated_tax_eur_cents"], 0)
        self.assertEqual(result["progressive_rate_amounts_eur_cents"], {"801": 40_000})
        self.assertTrue(any("progressive" in note for note in result["notes"]))

    def test_notes_say_what_the_figure_leaves_out(self):
        notes = " ".join(austrian_indicative_tax(tax_year=2025, kennzahl_rows=[])["notes"])
        self.assertIn("not tax advice", notes)
        self.assertIn("this book only", notes)
        self.assertIn("KESt", notes)


class AssistantTaxSummaryTest(unittest.TestCase):
    """The assistant's tax summary carries the core figure for an Austrian book."""

    def _cli(self, root, *args):
        result = subprocess.run(
            [sys.executable, "-m", "kassiber", "--data-root", str(root), "--machine", *args],
            cwd=ROOT, capture_output=True, text=True, check=False,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def _csv(self, tmp, name, rows):
        path = Path(tmp) / name
        path.write_text(
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n" + "".join(rows),
            encoding="utf-8",
        )
        return str(path)

    def test_estimate_follows_the_book_and_is_withheld_once_journals_are_stale(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-tax-estimate-") as tmp:
            root = Path(tmp) / "data"
            self._cli(root, "init")
            self._cli(root, "workspaces", "create", "Demo")
            self._cli(root, "profiles", "create", "Main", "--fiat-currency", "EUR", "--tax-country", "at")
            self._cli(root, "wallets", "create", "--label", "Cold", "--kind", "address",
                      "--address", "bc1qtestaddress0000000000000000000000000000000")
            self._cli(root, "wallets", "import-csv", "--wallet", "Cold", "--file", self._csv(tmp, "a.csv", [
                "2025-01-01T10:00:00Z,buy-1,inbound,BTC,0.10000000,0,40000,Buy\n",
                "2026-02-01T10:00:00Z,sell-1,outbound,BTC,0.05000000,0,60000,Sell\n",
            ]))
            self._cli(root, "journals", "process")

            conn = open_db(str(root))
            try:
                ready = daemon._reports_tax_summary_payload(
                    conn, {"year": 2026}, include_indicative_tax=True
                )["indicative_tax"]
                desktop = daemon._reports_tax_summary_payload(conn, {"year": 2026})
            finally:
                conn.close()
            self.assertEqual(ready["status"], "indicative")
            self.assertEqual(ready["special_rate_base_eur_cents"], 100_000)
            self.assertEqual(ready["estimated_tax_eur_cents"], 27_500)
            # Only the assistant's read pays for the blocker check.
            self.assertNotIn("indicative_tax", desktop)

            self._cli(root, "wallets", "import-csv", "--wallet", "Cold", "--file", self._csv(tmp, "b.csv", [
                "2026-03-01T10:00:00Z,sell-2,outbound,BTC,0.01000000,0,70000,Sell\n",
            ]))
            conn = open_db(str(root))
            try:
                blockers = daemon.build_report_blockers_snapshot(conn)["blockers"]
                stale = daemon._indicative_tax_for_ai(conn, 2026)
            finally:
                conn.close()
            self.assertTrue(blockers)
            self.assertEqual(stale["status"], "withheld")
            self.assertIsNone(stale["estimated_tax_eur_cents"])


if __name__ == "__main__":
    unittest.main()
