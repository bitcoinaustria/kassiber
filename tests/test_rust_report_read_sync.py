"""Report-read refresh: the automatic wallet refresh a book's freshness policy
enables before report reads and maintenance runs, compared with Python.

The only source is a regtest descriptor wallet whose default backend serves
mainnet, so each refresh job fails before any request, exactly as Python's
does; the egress regression proves the no-traffic side separately.
"""
import re
import unittest

from tests.rust_desktop_parity import RUST_BIN, DesktopParity
from tests.test_rust_report_capital_gains import generated

TPUB = ("tpubDC8msFGeGuwnKG9Upg7DM2b4DaRqg3CUZa5g8v2SRQ6K4NSkxUgd7HsL2XVWbVm39yBA4LAxysQAm397z"
        "wQSQoQgewGiYZqrA9DsP4zbQ1M")
SYNC_BOOK = """
INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025');
INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at) VALUES('p','w','Book','EUR','at','2025');
INSERT INTO settings(key,value) VALUES('context_workspace','w'),('context_profile','p');
INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES
 ('desc','w','p','Vault','descriptor','{"descriptor":"wpkh([73c5da0a/84h/1h/0h]%(key)s/0/*)","change_descriptor":"wpkh([73c5da0a/84h/1h/0h]%(key)s/1/*)","chain":"bitcoin","network":"regtest","gap_limit":2}','2025');
""" % {"key": TPUB} + """
"""
POLICY = "INSERT INTO settings(key,value) VALUES('freshness.policy.profile.p','%s');"
RETRY = re.compile(r'"retry_after_seconds":\d+')


def masked(line):
    return RETRY.sub('"retry_after_seconds":<n>', generated(line))


def freshness_shape(snapshot):
    return {table: sorted(generated(repr(row)) for row in rows) for table, rows in snapshot.items()}


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustReportReadSyncTest(DesktopParity, unittest.TestCase):
    KIND = "ui.report.blockers"
    SEED = None

    def seed(self):
        self.change(SYNC_BOOK)

    def session(self, args, kind=None):
        return self.compare(args, kind=kind, mutates=True, mask=masked, tables=freshness_shape)

    def test_policy_without_source_classes_runs_nothing(self):
        self.change(POLICY % '{"report_read_sync": true}')
        responses = self.session([{}, {}])
        self.assertTrue(all(r["kind"] == self.KIND for r in responses))

    def test_failed_refresh_blocks_and_is_rate_limited(self):
        self.change(POLICY % '{"report_read_sync": true, "source_classes": {"onchain_wallet": true}}')
        responses = self.session([{}, {}, None])
        self.assertEqual(responses[0]["data"]["blockers"][0]["id"], "sync_failed")

    def test_maintenance_runs_sync_modes(self):
        self.change(POLICY % '{"report_read_sync": true, "source_classes": {"onchain_wallet": true}}')
        responses = self.session([{"sync": "if_enabled"}, {"sync": "if_enabled"}, {"sync": "always"},
                                  {"sync": "never"}], kind="ui.maintenance.run")
        self.assertIn("sync", responses[0]["data"]["maintenance"])
        self.change("DELETE FROM settings WHERE key LIKE 'freshness.policy.%';")
        self.session([{"sync": "if_enabled"}, {"sync": "always"}], kind="ui.maintenance.run")

    def test_disabled_policy_never_refreshes(self):
        self.session([{}], kind="ui.report.blockers")
        self.session([{"sync": "if_enabled"}], kind="ui.maintenance.run")

    def test_sqlcipher_report_read_refresh(self):
        self.encrypt("synthetic sync key")
        self.change(POLICY % '{"report_read_sync": true, "source_classes": {"onchain_wallet": true}}')
        self.session([{}, {}])
