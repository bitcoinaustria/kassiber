"""Desktop freshness and maintenance settings served by the Rust daemon.

Every session is compared byte for byte with the unchanged Python daemon in a
disposable root; mutating sessions compare all 112 tables afterwards.
"""
import json
from pathlib import Path
import subprocess
import sys
import unittest

from tests.rust_desktop_parity import RUST_BIN, DesktopParity

ROOT = Path(__file__).resolve().parents[1]
CATALOG = ROOT / "crates/wallets/scripts/desktop_settings_mutations.json"

FRESHNESS = """
INSERT INTO freshness_source_states(profile_id,source_key,source_type,source_label,status,state,stale_reason,
  blocking_reports,paused,rate_limited_until,cooldown_reason,retry_count,last_success_at,last_error_at,last_error_code,
  last_error_message,last_phase,progress_json,checkpoint_json,updated_at) VALUES
 ('p','onchain_wallet:cold','onchain_wallet','Cold','error','partially_stale','fetch_failed',1,0,NULL,NULL,2,
  '2025-01-01T00:00:00Z','2025-01-02T00:00:00Z','backend_unreachable',
  'GET https://user:pw@esplora.example/api/tx/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:0 failed: host=''10.0.0.9'' token=abc.',
  'backend_fetch','{"phase":"backend_fetch","backend_url":"http://secret.example","token":"t","nested":{"descriptor":"wpkh(x)","xpub":""}}',
  '{"tip":"secret-checkpoint"}','2025-01-02T00:00:00Z'),
 ('p','market_rates:p','market_rates','Rates','rate_limited','fresh',NULL,0,1,'2030-01-01T00:00:00Z','http_429',0,
  NULL,NULL,NULL,NULL,NULL,'{}','{}','2025-01-03T00:00:00Z'),
 ('foreign','onchain_wallet:foreign','onchain_wallet','Foreign','error','stale',NULL,1,0,NULL,NULL,0,NULL,NULL,NULL,
  'private foreign failure',NULL,'{}','{}','2025');
INSERT INTO freshness_jobs(id,profile_id,job_type,source_key,source_type,source_label,status,phase,priority,payload_json,
  progress_json,result_json,error_json,attempts,cancel_requested,run_after,cooldown_until,created_at,started_at,finished_at,updated_at)
VALUES
 ('j1','p','onchain_wallet_history','onchain_wallet:cold','onchain_wallet','Cold','queued',NULL,50,
  '{"wallet_id":"cold","wallet_label":"Cold","url":"tcp://electrum.example:50001","api_key":"k","checkpoint":{"x":1}}',
  '{}','{}','{"message":"see ssl://node.example:50002!"}',0,0,NULL,NULL,'2025-01-01T00:00:00Z',NULL,NULL,'2025'),
 ('j2','p','market_rate_coverage','market_rates:p','market_rates','Rates','rate_limited','rate_coverage',100,'not json',
  '{}','{}','{}',1,1,NULL,'2030-01-01T00:00:00Z','2025-01-02T00:00:00Z',NULL,NULL,'2025'),
 ('j3','p','journal_refresh','journals:p','journals','Journals','running','journal_refresh',10,'{}','{"phase":"x"}',
  '{}','{}',1,0,NULL,NULL,'2025-01-03T00:00:00Z','2025',NULL,'2025'),
 ('j4','p','onchain_wallet_history','onchain_wallet:cold','onchain_wallet','Cold','done',NULL,50,'{}','{}','{}','{}',
  1,0,NULL,NULL,'2024',NULL,NULL,'2024'),
 ('j5','foreign','journal_refresh','journals:foreign','journals','Journals','queued',NULL,10,'{}','{}','{}','{}',
  0,0,NULL,NULL,'2025',NULL,NULL,'2025');
UPDATE transactions SET pricing_quality='coarse_fallback' WHERE id IN ('a','b','g');
UPDATE transactions SET excluded=1 WHERE id='g';
"""

NO_CONTEXT = "DELETE FROM settings WHERE key IN ('context_workspace','context_profile');"
POLICIES = [
    "",
    "INSERT INTO settings(key,value) VALUES('ai.auto_sync_before_report_reads.profile.p',' Yes ');",
    "INSERT INTO settings(key,value) VALUES('freshness.policy.profile.p','[1]');",
    "INSERT OR REPLACE INTO settings(key,value) VALUES('freshness.policy.profile.p',"
    "'{\"background_enabled\":1,\"source_classes\":{\"market_rates\":\"x\",\"bogus\":true,\"journals\":0}}');",
    "INSERT OR REPLACE INTO settings(key,value) VALUES('market_rate_provider',' MEMPOOL ');",
    "INSERT OR REPLACE INTO settings(key,value) VALUES('market_rate_provider','kraken-csv');",
    "UPDATE profiles SET fiat_currency='usd ', require_coarse_review=1, bitcoin_rail_carrying_value=0 WHERE id='p';",
    "UPDATE profiles SET fiat_currency='CHF' WHERE id='p';",
]
READ_ARGS = [..., None, [], "x", 3, {}, {"unexpected": True}]

CONFIGURE_REFUSALS = [
    [], "x", {"bogus": 1, "also": 2, "source_classes": {}}, {"source_classes": []},
    {"source_classes": "x"}, {"market_rate_provider": 3}, {"market_rate_provider": ["a"]},
    {"auto_sync_before_report_reads": 1}, {"auto_sync_before_report_reads": "yes"},
    {"market_rate_provider": "kraken-csv"}, {"market_rate_provider": "\ud800"},
    {"source_classes": {"onchain_wallet": True, "bogus": True}},
    {"market_rate_provider": "mempool", "source_classes": {"bogus": 1}},
]
CONFIGURE_WRITES = [
    ..., None, {}, {"background_enabled": 1}, {"background_enabled": None, "report_read_sync": "x"},
    {"source_classes": {"market_rates": 1, "journals": 0}}, {"source_classes": None},
    {"market_rate_provider": " CoinGecko\t"}, {"market_rate_provider": ""},
    {"auto_sync_before_report_reads": True, "report_read_sync": False, "source_classes": {"journals": True}},
    {"auto_sync_before_report_reads": False}, {"auto_sync_before_report_reads": None, "report_read_sync": 0},
]


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustFreshnessSettingsTest(DesktopParity, unittest.TestCase):
    KIND = "ui.freshness.status"

    def setUp(self):
        super().setUp()
        self.change(FRESHNESS)

    def test_status_reads_match_wire_bytes_across_policies(self):
        for sql in POLICIES:
            with self.subTest(sql=sql):
                self.change(sql)
                responses = self.compare(READ_ARGS)
                self.assertTrue(all(r["kind"] == self.KIND for r in responses), responses)
                self.compare(READ_ARGS, kind="ui.maintenance.settings")
        status = responses[0]["data"]
        self.assertEqual([s["source_key"] for s in status["sources"]],
                         ["market_rates:p", "onchain_wallet:cold"])
        self.assertNotIn("secret-checkpoint", str(responses))

    def test_status_without_or_with_mismatched_context(self):
        self.change("UPDATE settings SET value='foreign' WHERE key='context_profile';")
        self.compare([{}], kind="ui.maintenance.settings")
        self.compare([{}])
        self.change("INSERT INTO workspaces(id,label,created_at) VALUES('w2','','2025');"
                    "UPDATE settings SET value='w2' WHERE key='context_workspace';")
        self.compare([{}], kind="ui.maintenance.settings")
        self.compare([{}])
        self.change(NO_CONTEXT)
        for kind in self.KIND, "ui.maintenance.settings", "ui.freshness.configure", "ui.maintenance.configure":
            self.compare([{}, {"bogus": 1}, {"require_coarse_review": 3}], kind=kind)

    def test_configure_refusals_leave_every_table_unchanged(self):
        for kind in "ui.freshness.configure", "ui.maintenance.configure":
            responses = self.compare(CONFIGURE_REFUSALS, kind=kind)
            self.assertTrue(all(r["kind"] == "error" for r in responses), responses)
        self.compare([{"require_coarse_review": 1}, {"bitcoin_rail_carrying_value": "no"},
                      {"require_coarse_review": True, "bitcoin_rail_carrying_value": 0},
                      {"require_coarse_review": None, "source_classes": 1}],
                     kind="ui.maintenance.configure")

    def test_configure_writes_match_bytes_and_tables(self):
        for kind in "ui.freshness.configure", "ui.maintenance.configure":
            with self.subTest(kind=kind):
                responses = self.compare(CONFIGURE_WRITES, kind=kind, mutates=True)
                self.assertTrue(all(r["kind"] == kind for r in responses), responses)
                self.compare(READ_ARGS[:2])

    def test_maintenance_profile_flags_commit_before_policy_validation(self):
        self.compare([{"require_coarse_review": True, "bitcoin_rail_carrying_value": False},
                      {"require_coarse_review": True},
                      {"require_coarse_review": False, "market_rate_provider": "bogus"},
                      {"bitcoin_rail_carrying_value": True, "source_classes": {"bogus": True}},
                      {"bitcoin_rail_carrying_value": False, "auto_sync_before_report_reads": True,
                       "market_rate_provider": "mempool"}],
                     kind="ui.maintenance.configure", mutates=True)

    def test_expected_scope_and_sqlcipher_sessions(self):
        scope = {"workspace_id": "w", "profile_id": "p"}
        self.compare([{"expected_scope": scope}, {"expected_scope": {**scope, "profile_id": "foreign"}},
                      {"expected_scope": "x"}])
        self.compare([{"expected_scope": scope, "background_enabled": True},
                      {"expected_scope": {**scope, "profile_id": "foreign"}, "background_enabled": True}],
                     kind="ui.freshness.configure", mutates=True)
        self.encrypt("synthetic settings key")
        self.change(FRESHNESS)
        self.compare([{}])
        self.compare([{}], kind="ui.maintenance.settings")
        self.compare([{"report_read_sync": True}, {"market_rate_provider": "coingecko"}],
                     kind="ui.freshness.configure", mutates=True)
        self.compare([{"require_coarse_review": True}], kind="ui.maintenance.configure", mutates=True)


class DesktopSettingsMutationCatalogTest(unittest.TestCase):
    def test_catalog_applies_and_names_existing_tests(self):
        entries = json.loads(CATALOG.read_text())
        result = subprocess.run(
            [sys.executable, str(ROOT / "crates/wallets/scripts/desktop_settings_mutation_check.py"), "--check"],
            cwd=ROOT, capture_output=True, timeout=60, check=True,
        )
        self.assertEqual(result.stdout, f"{len(entries)} entries apply\n".encode())
        self.assertTrue(all(test.startswith("tests/test_rust_desktop_") for entry in entries
                            for test in entry["expect_failing"]))


if __name__ == "__main__":
    unittest.main()
