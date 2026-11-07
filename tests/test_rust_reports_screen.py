"""Main sync 1 S7: the Reports screen payload and the crypto Kennzahlen from
1 March 2022 (Europe/Vienna), against the unchanged Python daemon.

Each session compares response bytes and all logical tables, plaintext and
SQLCipher: ``ui.reports.capital_gains`` (breakdown, totals, indicative tax,
the empty-root ``jurisdictionCode``), ``ui.journals.events.list``
(``explanationReference``), and the E 1kv exports' refusals for years before
2022 and for a 2022 with January or February rows.
"""
import unittest

from kassiber.db import open_db
from tests.rust_desktop_parity import RUST_BIN, DesktopParity
from tests.test_rust_report_capital_gains import BOOK

AT = "UPDATE profiles SET tax_country='at' WHERE id='p';"
CONVERSIONS = (
    "INSERT INTO journal_custody_economic_relations(relation_id,workspace_id,profile_id,relation_kind,"
    "source_transaction_id,target_transaction_id,source_asset,target_asset,source_amount_msat,"
    "target_amount_msat,basis_state,occurred_at,target_occurred_at,created_at) VALUES"
    "(printf('%064d',6),'ws','p','conversion','out','earn','BTC','LBTC',5000,4999,'eligible',"
    "'2024-12-31T23:30:00Z','2025-01-02T00:00:00Z','d'),"
    "(printf('%064d',7),'ws','p','conversion','out','in','BTC','BTC',5001,5001,'eligible',"
    "'2024-12-31T23:30:00Z','2025-01-02T00:00:00Z','e');"
)
FIELDS = (
    "UPDATE journal_entries SET at_category='alt_spekulation',at_kennzahl=801 WHERE id='z';"
    "UPDATE journal_entries SET at_category='neu_loss',at_kennzahl=176,gain_loss=-0.5 WHERE id='first';"
)
# 23:59:59 on 28 February in Vienna is still the old rules; 1 March is not.
EARLY_2022 = (
    "UPDATE journal_entries SET occurred_at='2022-02-28T22:59:59Z' WHERE id='a';"
    "UPDATE journal_entries SET occurred_at='2022-02-28T23:00:00Z' WHERE id='income';"
    "UPDATE journal_entries SET occurred_at='2022-05-10T10:00:00Z' WHERE id IN ('z','first','swap');"
)
YEARS = [..., {}, {"year": 2021}, {"year": 2022}, {"year": 2024}, {"year": 2025}]
EXPORTS = ("ui.reports.export_austrian_e1kv_csv", "ui.reports.export_austrian_e1kv_xlsx")


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustReportsScreenTest(DesktopParity, unittest.TestCase):
    KIND = "ui.reports.capital_gains"
    SEED = None

    def seed(self):
        with open_db(self.root, passphrase=self.passphrase) as conn:
            conn.executescript(BOOK)

    def screen(self):
        self.compare(YEARS)
        self.compare([{}, {"transaction": "out"}], kind="ui.journals.events.list")

    def test_generic_book_breakdown_swaps_and_holding_period(self):
        self.change(CONVERSIONS + "UPDATE profiles SET tax_long_term_days=730 WHERE id='p';")
        self.screen()

    def test_austrian_fields_swaps_and_indicative_tax(self):
        self.change(AT + FIELDS + CONVERSIONS)
        self.screen()

    def test_rows_before_march_2022_are_in_no_field_and_withhold_the_figure(self):
        self.change(AT + EARLY_2022)
        self.screen()
        for kind in EXPORTS:
            self.compare([{"year": 2022}, {"year": 2021}, {"year": 2020}], kind=kind)

    def test_a_year_before_the_e1kv_states_only_kz_801(self):
        self.change(AT + FIELDS + "UPDATE journal_entries SET occurred_at='2021-06-01T10:00:00Z';")
        self.screen()
        for kind in EXPORTS:
            self.compare([{"year": 2021}, {"year": 2009}], kind=kind)

    def test_stale_journal_explains_nothing(self):
        # A taxability override and a swap never carry a reference either.
        self.change("UPDATE transactions SET taxability_override=0 WHERE id='earn';")
        self.compare([{}], kind="ui.journals.events.list")
        self.change("UPDATE profiles SET last_processed_at=NULL WHERE id='p';")
        self.compare([{}], kind="ui.journals.events.list")

    def test_empty_root_reports_no_country(self):
        self.change("DELETE FROM settings WHERE key IN ('context_workspace','context_profile');")
        self.compare([..., {}, {"year": 2021}])
        self.compare([{}], kind="ui.journals.events.list")

    def test_book_without_a_country_is_generic(self):
        self.change("UPDATE profiles SET tax_country='' WHERE id='p';")
        self.compare(YEARS)

    def test_sqlcipher_reports_screen(self):
        self.encrypt("synthetic reports screen key")
        self.change(AT + EARLY_2022 + CONVERSIONS)
        self.screen()
        for kind in EXPORTS:
            self.compare([{"year": 2022}, {"year": 2021}], kind=kind)
