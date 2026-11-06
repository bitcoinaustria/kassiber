import json
import unittest

from tests.rust_desktop_parity import RUST_BIN, DesktopParity

ACTIVITY = """
INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,occurred_at,confirmed_at,created_at,kind,fiat_rate,fiat_value,review_status,note,description,counterparty) VALUES
 ('h','w','p','cold','h',NULL,'BTC','outbound',5000000,1000,'2025-03-05T10:00:00Z','2025','2025-03-05','withdrawal',50000,2.5,NULL,'','',''),
 ('i','w','p','ln','i',NULL,'BTC','inbound',4999000,0,'2025-03-05T10:30:00Z','2025','2025-03-05','deposit',50000,NULL,NULL,'','',''),
 ('j','w','p','cold','j',NULL,'BTC','outbound',20000000,0,'2025-04-10T12:00:00Z','2025','2025-04-10','peg-in',60000,12,NULL,'','',''),
 ('k','w','p','liquid','k',NULL,'LBTC','inbound',19990000,0,'2025-04-10T12:20:00Z','2025','2025-04-10','peg-in',60000,11.99,NULL,'','',''),
 ('l','w','p','exchange','l',NULL,'BTC','inbound',300000000,0,'2024-10-27T00:30:00Z','2024','2024-10-27','buy',40000,120,'completed','','',''),
 ('m','w','p','exchange','m',NULL,'BTC','inbound',2000000,0,'2023-12-31T23:30:00Z','2024','2024-01-01','buy',30000,NULL,'pending','','','');
"""
RELATION = """
INSERT INTO journal_custody_economic_relations(relation_id,workspace_id,profile_id,relation_kind,source_transaction_id,target_transaction_id,source_asset,target_asset,source_amount_msat,target_amount_msat,basis_state,occurred_at,created_at)
VALUES('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','w','p','conversion','g','b','BTC','LBTC',100001,100001,'eligible','2025','2025');
UPDATE profiles SET last_processed_at='2025',last_processed_tx_count=13,journal_input_version=2,last_processed_input_version=2 WHERE id='p'; UPDATE profiles SET journal_semantics_version=2 WHERE id='p';
"""
WINDOW = {"since": "2025-01-01T00:00:00Z", "until": "2025-06-30T23:59:59Z"}


def _without_main_sync_fields(line):
    """Drop the fields main #672 to #714 added and slices S8 to S10 port
    (`bucketSize`, `history.years`), so the alias buckets compare on this base."""
    value = json.loads(line)
    data = value.get("data")
    if isinstance(data, dict):
        data.pop("bucketSize", None)
        if isinstance(data.get("history"), dict):
            data["history"].pop("years", None)
    return json.dumps(value, sort_keys=True)


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustTransactionDashboardTest(DesktopParity, unittest.TestCase):
    KIND = "ui.transactions.dashboard"

    def test_refusals_and_missing_context_match_wire_bytes(self):
        self.compare([..., None, [], 1, "x", {"z": 1, "a": 2}, {"resolveAuto": "maybe"},
                      {"resolveAuto": None}, {"period": ""}, {"period": 3}, {"period": "weekly"},
                      {"period": "1year"}, {"period": "ytd", "since": "2025-01-01"},
                      {"period": "all", "since": "bad"}, {"period": "all", "until": "2025-13-01"},
                      {"period": "all", "since": "2025-02-01", "until": "2025-01-01"},
                      {"period": "all", "timezone": "Mars/Base"}, {"period": "all", "timezone": " "},
                      {"period": "all", "wallet": ""}, {"period": "all", "wallet": 4}])
        self.change("DELETE FROM settings WHERE key IN ('context_workspace','context_profile');")
        self.compare([..., None, {}, {"resolveAuto": True}, {"resolveAuto": "off"}, {"z": 1},
                      {"resolveAuto": 2}, {"period": "bogus"}])

    def test_periods_buckets_wallets_and_flows_match_wire_bytes(self):
        self.change(ACTIVITY)
        args = [{"period": "all"}, {"period": "MAX", "resolveAuto": "yes"},
                {"period": " all ", "wallet": "cold"}, {"period": "all", "wallet": "Cold"},
                {"period": "all", "wallet": "absent"}, {"period": "all", "wallet": "ln"},
                {"period": "all", "since": "2025-02-01T00:00:00Z"},
                {"period": "all", "until": "2024-12-31T23:00:00+01:00"}]
        for period in ("30days", "3months", "6m", "ytd", "1year", "1y", "5years", "10y", "15years", "30d"):
            args.append({"period": period, **WINDOW})
            args.append({"period": period, "since": "2024-10-20T00:00:00Z",
                         "until": "2025-04-10T12:00:00Z", "timezone": "Europe/Vienna"})
        for timezone in ("America/New_York", "Asia/Kathmandu", "Pacific/Chatham", "Etc/GMT+12", "UTC"):
            args.append({"period": "3months", "timezone": timezone, **WINDOW})
            args.append({"period": "all", "timezone": timezone, "resolveAuto": 1})
        responses = self.compare(args)
        self.assertTrue(all(response["kind"] == self.KIND for response in responses))
        self.assertEqual(responses[0]["data"]["candidates"][0]["out_id"], "h")

    def test_period_aliases_bucket_like_their_canonical_names(self):
        # tests/rust_port/repro_dashboard_bucket_aliases.py: 30d buckets by day,
        # 3m by week and 5y by quarter, like 30days, 3months and 5years.
        self.change(ACTIVITY)
        args = []
        for period in ("30d", "30day", "3m", "3month", "6m", "1y", "1years", "5y", "5year", "10y", "15year"):
            args.append({"period": period, **WINDOW})
            args.append({"period": period, "since": "2024-10-20T00:00:00Z",
                         "until": "2025-04-10T12:00:00Z", "timezone": "Europe/Vienna"})
        responses = self.compare(args, mask=_without_main_sync_fields)
        keys = {arg["period"]: [bucket["bucketKey"] for bucket in response["data"]["buckets"]]
                for arg, response in zip(args[::2], responses[::2])}
        self.assertEqual(keys["30d"][:2], ["2025-01-01", "2025-01-02"])
        self.assertEqual(keys["3m"][:2], ["2024-12-30", "2025-01-06"])
        self.assertEqual(keys["5y"], ["2025-Q1", "2025-Q2"])

    def test_auto_period_selection_matches_wire_bytes(self):
        self.change(ACTIVITY)
        args = []
        for until in ("2025-04-30T00:00:00Z", "2024-02-29T12:00:00Z", "2028-02-29T12:00:00Z",
                      "2045-01-01T00:00:00Z", "2023-12-31T23:30:00Z", "2020-01-01T00:00:00Z"):
            for timezone in ("UTC", "Europe/Vienna", "Pacific/Kiritimati"):
                args.append({"period": "all", "until": until, "timezone": timezone, "resolveAuto": True})
                args.append({"period": "all", "until": until, "timezone": timezone,
                             "resolveAuto": True, "wallet": "exchange"})
        self.compare(args)
        self.change("UPDATE transactions SET amount=1 WHERE profile_id='p';")
        self.compare([{"period": "all", "until": "2025-04-30T00:00:00Z", "resolveAuto": True}])
        self.change("DELETE FROM transactions WHERE profile_id='p';")
        self.compare([{"period": "all", "resolveAuto": True}, {"period": "all"},
                      {"period": "30days", **WINDOW}])

    def test_ambiguous_midnight_folds_match_bucket_offsets(self):
        # Havana repeats 00:00-01:00 on 2026-11-01; Santiago skips midnight.
        self.change("""
            INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,occurred_at,confirmed_at,created_at)
            VALUES('fold','w','p','cold','fold','BTC','inbound',5000000,0,'2026-11-01T05:30:00Z','2026','2026'),
                  ('gap','w','p','cold','gap','BTC','inbound',5000000,0,'2026-09-06T04:30:00Z','2026','2026');
        """)
        args = []
        for timezone in ("America/Havana", "America/Santiago", "Asia/Beirut"):
            for period in ("30days", "3months", "1year", "all"):
                args.append({"period": period, "since": "2026-11-01T05:10:00Z",
                             "until": "2026-12-31T00:00:00Z", "timezone": timezone})
                args.append({"period": period, "since": "2026-09-06T04:10:00Z",
                             "until": "2026-11-02T00:00:00Z", "timezone": timezone})
            args.append({"period": "all", "timezone": timezone})
        self.compare(args)

    def test_reviewed_pairs_conflicts_and_stale_journals_match_wire_bytes(self):
        self.change(ACTIVITY + RELATION)
        args = [{"period": "all"}, {"period": "3months", **WINDOW}, {"period": "all", "wallet": "ln"},
                {"period": "all", "since": "2025-01-15T00:00:00Z"}]
        current = self.compare(args)
        self.assertTrue(current[0]["data"]["metricsScope"]["pairsGrouped"])
        self.change("UPDATE profiles SET last_processed_input_version=1 WHERE id='p';")
        stale = self.compare(args)
        self.assertFalse(stale[0]["data"]["metricsScope"]["pairsGrouped"])
        self.change("UPDATE profiles SET last_processed_input_version=2 WHERE id='p'; UPDATE profiles SET journal_semantics_version=2 WHERE id='p';"
                    "UPDATE transactions SET fiat_rate=NULL,fiat_value=NULL WHERE id IN ('h','b');")
        self.compare(args + [{"period": "all", "wallet": "cold"}])

    def test_sqlcipher_dashboard_matches_wire_bytes_and_all_tables(self):
        self.encrypt("synthetic dashboard key")
        self.change(ACTIVITY + RELATION)
        self.compare([{"period": "all", "resolveAuto": True, "until": "2025-05-01T00:00:00Z"},
                      {"period": "1year", **WINDOW, "timezone": "Europe/Vienna"}, {"z": 1}])
