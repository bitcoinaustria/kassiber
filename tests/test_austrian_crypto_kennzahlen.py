"""Which field a classified Austrian row is stated in, by its date.

The crypto Kennzahlen (KZ 172, 174, 176) cover income and realisations from
1 March 2022, Europe/Vienna. Earlier rows followed the old rules, so every
surface states them in no field and the E 1kv refuses to state them.
"""

import unittest
from types import SimpleNamespace
from unittest.mock import patch

from kassiber.core import reports
from kassiber.core.austrian import kennzahl_for_event, stated_kennzahl
from kassiber.errors import AppError


def _row(occurred_at, category="neu_gain", kennzahl=174):
    return {
        "at_category": category,
        "occurred_at": occurred_at,
        "quantity": -100_000_000_000,
        "proceeds": 50_000,
        "cost_basis": 40_000,
        "gain_loss": 10_000,
        "entry_type": "disposal",
        "transaction_note": "",
        "description": "",
        "transaction_external_id": "tx",
        "transaction_id": "tx",
        "wallet": "Cold",
        "asset": "BTC",
        "transaction_kind": "sell",
        "at_kennzahl": kennzahl,
    }


class CryptoKennzahlenTest(unittest.TestCase):
    def test_the_fields_start_on_1_march_2022_in_vienna(self):
        # 23:59:59 on 28 February in Vienna is still the old rules.
        self.assertIsNone(kennzahl_for_event("neu_gain", "2022-02-28T22:59:59Z"))
        self.assertEqual(kennzahl_for_event("neu_gain", "2022-02-28T23:00:00Z"), 174)
        self.assertIsNone(kennzahl_for_event("income_general", "2021-06-01T10:00:00Z"))
        # KZ 801 is an E 1 field in any year.
        self.assertEqual(kennzahl_for_event("alt_spekulation", "2019-06-01T10:00:00Z"), 801)

    def test_exports_state_a_stored_field_by_its_date(self):
        self.assertIsNone(stated_kennzahl(174, "2021-05-01T00:00:00Z"))
        self.assertEqual(stated_kennzahl(174, "2024-05-01T00:00:00Z"), 174)
        self.assertEqual(stated_kennzahl(801, "2019-05-01T00:00:00Z"), 801)
        self.assertIsNone(stated_kennzahl(None, "2024-05-01T00:00:00Z"))

    def test_the_e1kv_states_no_field_for_a_row_before_march_2022(self):
        row = reports._austrian_e1kv_detail_row(_row("2022-02-10T10:00:00Z"))
        self.assertIsNone(row["kennzahl"])
        self.assertEqual(row["form_amount_eur_cents"], 0)
        self.assertEqual(reports._austrian_e1kv_detail_row(_row("2022-03-10T10:00:00Z"))["kennzahl"], 174)

    def test_the_e1kv_refuses_a_year_with_rows_under_the_old_rules(self):
        context = SimpleNamespace(workspace={"label": "W"}, profile={"label": "P"})
        rows = [
            reports._austrian_e1kv_detail_row(_row("2022-02-10T10:00:00Z")),
            reports._austrian_e1kv_detail_row(_row("2022-05-10T10:00:00Z")),
        ]
        with patch.object(reports, "_require_report_context", return_value=context), patch.object(
            reports, "_austrian_e1kv_rows", return_value=rows
        ):
            with self.assertRaises(AppError) as raised:
                reports.report_austrian_e1kv(None, "w", "p", None, tax_year=2022)
        self.assertEqual(raised.exception.code, "validation")
        self.assertEqual(raised.exception.details["rows"], 1)


if __name__ == "__main__":
    unittest.main()
