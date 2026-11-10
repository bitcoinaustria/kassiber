import unittest

from tests.rust_desktop_parity import RUST_BIN, DesktopParity
from tests.test_rust_report_capital_gains import generated, journal_shape

LIVE = ("INSERT OR REPLACE INTO settings(key,value) VALUES('freshness.policy.profile.p',"
        "'{\"source_classes\": {\"market_rates\": true}}');")
PRICED = ("UPDATE transactions SET fiat_rate=95000,fiat_value=0.095,pricing_source_kind='fmv_provider',"
          "pricing_provider='kraken-csv',pricing_pair='BTC-EUR',fiat_price_source='rates_cache' WHERE id='b';")


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustRatesRebuildTest(DesktopParity, unittest.TestCase):
    KIND = "ui.rates.rebuild"

    def kraken(self):
        path = self.base / "XBTEUR_1.csv"
        path.write_text("1738367940,95000,95100,94900,95050.123456789,1,2\n"
                        "1738368000,95100,95200,95000,95150,1,2\n"
                        "1738368060,95200,95300,95100,95250,1,2\n")
        return str(path)

    def test_refusals_and_live_opt_in_match_wire_bytes(self):
        self.compare([[], "x", 4, {"pair": 3}, {"path": 3}, {"days": 0}, {"days": "x"},
                      {"days": -2}, {"days": [1]}, {"source": "bogus"}, {"source": "  BOGUS "},
                      {"pair": "BTC-JPY", "source": "kraken-csv", "path": "x"},
                      {"source": "kraken-csv"}, {"source": "kraken-csv", "pair": "  "},
                      {"source": "mempool", "path": "x"}, {}, {"source": "coingecko"},
                      {"source": "MEMPOOL", "pair": "BTC-USD"}], mutates=True)
        self.change("UPDATE profiles SET fiat_currency='JPY' WHERE id='p';")
        self.compare([{}, {"source": "kraken-csv", "path": "x"}], mutates=True)
        self.change("DELETE FROM settings WHERE key IN ('context_workspace','context_profile');")
        self.compare([{}, {"source": "kraken-csv", "path": "x"},
                      {"source": "kraken-csv", "path": "x", "pair": "BTC-EUR", "reprice_transactions": False}],
                     mutates=True)

    def test_enabled_live_sources_stop_at_the_offline_gate(self):
        self.change(LIVE)
        self.compare([{}, {"source": "coingecko", "days": 2}, {"source": "mempool", "pair": "BTC-USD"}],
                     mutates=True)

    def test_local_kraken_rebuild_reprices_and_rebuilds_journals(self):
        path = self.kraken()
        self.change(PRICED)
        self.compare([{"source": "kraken-csv", "path": path, "reprice_transactions": False}],
                     mutates=True, mask=generated, tables=journal_shape)
        self.compare([{"source": "kraken-csv", "path": " " + path + " ", "days": "3"}],
                     mutates=True, mask=generated, tables=journal_shape)
        self.compare([{"source": "kraken-csv", "path": path, "pair": "BTC-EUR", "days": 1.9}],
                     mutates=True, mask=generated, tables=journal_shape)

    def test_sqlcipher_rebuild_matches_wire_bytes_and_tables(self):
        self.encrypt("synthetic rates key")
        path = self.kraken()
        self.compare([{"source": "kraken-csv", "path": path}, {"source": "coingecko"}],
                     mutates=True, mask=generated, tables=journal_shape)
