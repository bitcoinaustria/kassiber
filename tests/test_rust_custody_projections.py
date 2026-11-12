import json
import unittest

from tests.rust_desktop_parity import RUST_BIN, DesktopParity

POLICY = """
INSERT INTO wallet_policy_epochs(id,workspace_id,profile_id,wallet_id,chain,network,status,created_at,retired_at) VALUES
 ('e1','w','p','cold','bitcoin','regtest','retired','2025-01-01','2025-02-01'),
 ('e2','w','p','cold','bitcoin','regtest','active','2025-02-01',NULL),
 ('e3','w','p','liquid','liquid','test','active','2025-01-05',NULL),
 ('e4','w','p','ln','bitcoin','main','active','2025-01-06',NULL),
 ('ef','w','foreign','foreign','bitcoin','main','active','2025-01-01',NULL);
INSERT INTO wallet_policy_sources(id,epoch_id,source_wallet_id,source_key,observer_kind,branch_keys_json,created_at,updated_at) VALUES
 ('s1','e1','cold','descriptor:secret-checksum','bdk','["receive","change"]','2025','2025'),
 ('s2','e2','cold','xpub:p2wpkh:zpub-private','BDK','["change","other"]','2025','2025'),
 ('s3','e2','cold','xpub:weird:x','compatibility-esplora','not json','2025','2025'),
 ('s4','e3','liquid','samourai:postmix:x','lwk','{"receive":1}','2025','2025'),
 ('s5','e3','liquid','samourai:hidden','custom-observer','[]','2025','2025'),
 ('s6','e3','liquid','plain-source','bitcoinrpc','["receive"]','2025','2025');
INSERT INTO wallet_policy_coverage_witnesses(source_id,branch_key,scanned_to_exclusive,highest_used,observer_kind,observed_at) VALUES
 ('s1','receive',20,3,'bdk','2025-01-10'),('s1','change',5,NULL,'bdk','2025-01-11'),
 ('s2','receive',7,6,'bdk','2025-02-02'),('s2','private',9,1,'bdk','2025-02-02'),
 ('s4','change',0,NULL,'lwk','2025-01-07');
"""
DECISIONS = """
INSERT INTO journal_custody_decisions(decision_id,workspace_id,profile_id,source_transaction_id,target_transaction_id,source_wallet_id,target_wallet_id,source_observation_hash,source_start_msat,source_end_msat,target_observation_hash,target_start_msat,target_end_msat,source_network,target_network,source_rail,target_rail,source_asset,target_asset,state,basis_state,basis_barrier_at,reason,atomic_group_id,component_id,occurred_at,created_at) VALUES
 ('1111111111111111111111111111111111111111111111111111111111111111','w','p','g','b','cold','ln','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',0,100001,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',0,100001,'main','main','onchain','lightning','BTC','BTC','internal_verified','eligible',NULL,'exact','bundle',NULL,'2025-01-02T00:00:00Z','2025'),
 ('2222222222222222222222222222222222222222222222222222222222222222','w','p','g','e','cold','custom','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',100001,150000,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',0,49999,'main','test','onchain','onchain','BTC','LBTC','internal_reviewed','blocked_by_prior_custody_basis','2025-01-01','reviewed',NULL,'component','2025-01-02T00:00:00Z','2025'),
 ('3333333333333333333333333333333333333333333333333333333333333333','w','p','c','a','exchange','cold','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',0,7,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',0,7,'main','main','onchain','onchain','BTC','BTC','internal_verified','eligible',NULL,'x',NULL,NULL,NULL,'2025'),
 ('4444444444444444444444444444444444444444444444444444444444444444','w','p','d','identifier-only-record','liquid','ambiguous','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',0,1,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',0,1,'liquidv1','liquidv1','liquid','liquid','LBTC','LBTC','internal_verified','eligible',NULL,'y',NULL,NULL,'2025-03-01T00:00:00Z','2025');
"""


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustCustodyProjectionsTest(DesktopParity, unittest.TestCase):
    KIND = "ui.custody.lineage.snapshot"

    def test_coverage_redacts_sources_and_matches_wire_bytes(self):
        kind = "ui.custody.coverage.snapshot"
        self.compare([..., None, {}, [], "x", {"z": 1, "a": 2}, {"workspace": "w", "profile": "p"},
                      {"profile": "Other"}, {"profile": "absent"}], kind=kind)
        self.change(POLICY)
        covered = self.compare([{}, {"profile": "foreign"}, {"workspace": "Main", "profile": "Book"}], kind=kind)
        self.assertEqual(covered[0]["data"]["summary"]["epoch_count"], 4)
        self.assertEqual(covered[0]["data"]["summary"]["covered_branch_count"], 4)
        self.change("DELETE FROM settings WHERE key IN ('context_workspace','context_profile');")
        self.compare([{}, {"workspace": "w", "profile": "p"}], kind=kind)

    def test_lineage_pages_filters_and_cursors_match_wire_bytes(self):
        self.change(DECISIONS)
        first = self.compare([{}, {"limit": 2}, {"limit": 1}, {"transaction_id": " g "},
                              {"transaction_id": "absent"}, {"limit": 1, "transaction_id": "g"}])
        self.assertEqual(first[0]["data"]["summary"]["total_count"], 4)
        cursors = [response["data"]["next_cursor"] for response in first if response["data"]["next_cursor"]]
        self.assertTrue(cursors)
        args = [{"limit": 1, "cursor": cursor} for cursor in cursors]
        args += [{"cursor": cursors[0], "limit": 3}, {"cursor": cursors[-1], "transaction_id": "b"},
                 {"cursor": "bm90LWpzb24"}, {"cursor": "e30"}, {"cursor": ""}, {"cursor": 4},
                 {"transaction_id": " "}, {"transaction_id": 3}, {"limit": 0}, {"limit": "x"},
                 {"limit": 9999}, {"z": 1}, [], "x", ..., None]
        second = self.compare(args)
        page = json.loads(json.dumps(second[0]))
        if page["data"]["next_cursor"]:
            self.compare([{"limit": 1, "cursor": page["data"]["next_cursor"]}])
        self.change("DELETE FROM settings WHERE key='context_profile';")
        self.compare([{}, {"limit": 1}, {"cursor": cursors[0]}])

    def test_sqlcipher_custody_reads_match_wire_bytes(self):
        self.encrypt("synthetic custody key")
        self.change(POLICY + DECISIONS)
        self.compare([{}, {"limit": 2}])
        self.compare([{}], kind="ui.custody.coverage.snapshot")
