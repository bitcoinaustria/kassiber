"""Report-read refresh of a book with no backend configured at all (gap Q7).

Python's `_run_auto_sync_wallets` then runs `_wallets_sync_payload({"all":
True}, strict=False)`: the wallet sources only, under the report-read
trigger (policy-filtered, each job alone, no direct sync, no journal step),
or as an explicit wallet sync when forced. Both daemons answer byte for byte
(generated identifiers and clocks masked) and leave the same tables. Every
built-in backend is deleted (manual bootstrap mode with tombstones) and none
is stored, so no request can be made; the strict socket guard proves it.
"""
import unittest

from tests.rust_desktop_parity import RUST_BIN, DesktopParity
from tests.test_rust_report_capital_gains import generated, journal_shape

BOOK = """
INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025');
INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at)
 VALUES('p','w','Book','EUR','at','2025');
INSERT INTO settings(key,value) VALUES('context_workspace','w'),('context_profile','p');
DELETE FROM backends;
DELETE FROM settings WHERE key IN ('default_backend','bootstrap_default_backend');
INSERT OR REPLACE INTO settings(key,value) VALUES('backend_bootstrap_mode','manual'),
 ('bootstrap_backend_tombstones','["fulcrum","liquid","liquid-blockstream","mempool"]');
INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES
 ('a','w','p','Addresses','address','{"addresses":["bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"]}','2025'),
 ('m','w','p','Manual','custom','{}','2025');
"""
ENABLED = ("INSERT INTO settings(key,value) VALUES('freshness.policy.profile.p',"
           "'{\"report_read_sync\": true, \"background_enabled\": false, "
           "\"source_classes\": {\"onchain_wallet\": true}}');")
ONCHAIN_OFF = ("UPDATE settings SET value='{\"report_read_sync\": true, \"source_classes\": "
               "{\"onchain_wallet\": false}}' WHERE key='freshness.policy.profile.p';")
# Rust stamps journal calculation semantics 2 where Python stamps 7 (a known,
# separate difference), so the forced case keeps the journal class off.
JOURNALS_OFF = ("UPDATE settings SET value='{\"report_read_sync\": true, \"source_classes\": "
                "{\"onchain_wallet\": false, \"journals\": false}}' WHERE key='freshness.policy.profile.p';")


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustReportReadWithoutBackendsTest(DesktopParity, unittest.TestCase):
    KIND = "ui.maintenance.run"
    SEED = None

    def setUp(self):
        super().setUp()
        self.env.pop("KASSIBER_NO_EGRESS", None)
        self.env["KASSIBER_TEST_NO_EGRESS"] = "strict"

    def seed(self):
        self.change(BOOK + ENABLED)

    def test_report_read_runs_the_wallet_sources_like_python(self):
        # The first read refreshes; the second is rate limited and cached.
        responses = self.compare([{"sync": "if_enabled"}, {"sync": "if_enabled"}],
                                 mutates=True, mask=generated, tables=journal_shape)
        sync = responses[0]["data"]["maintenance"]["sync"]["data"]
        self.assertEqual(sync["ok"], False)
        self.assertEqual([(row["wallet"], row["code"]) for row in sync["results"]],
                         [("Addresses", "backend_not_configured")])
        self.assertNotIn("journals", sync)
        self.assertEqual(responses[1]["data"]["maintenance"]["sync"]["data"]["status"], "cached")

    def test_disabled_source_class_queues_nothing_like_python(self):
        self.change(ONCHAIN_OFF)
        responses = self.compare([{"sync": "if_enabled"}], mutates=True, mask=generated, tables=journal_shape)
        sync = responses[0]["data"]["maintenance"]["sync"]["data"]
        self.assertEqual((sync["ok"], sync["results"], sync["enqueued"]), (True, [], []))

    def test_forced_refresh_is_an_explicit_wallet_sync_like_python(self):
        self.change(JOURNALS_OFF)
        responses = self.compare([{"sync": "always"}], mutates=True, mask=generated, tables=journal_shape)
        sync = responses[0]["data"]["maintenance"]["sync"]["data"]
        # Forced: every wallet source runs and the journal step follows.
        self.assertEqual([row["code"] for row in sync["results"]], ["backend_not_configured"])
        self.assertEqual(sync["journals"]["status"], "disabled")
