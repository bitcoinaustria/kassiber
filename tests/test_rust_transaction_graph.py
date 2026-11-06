import json
import unittest

from tests.rust_desktop_parity import RUST_BIN, DesktopParity, run_candidate
from tests.test_rust_report_blockers import ADDR, SCRIPT
from tests.test_rust_transaction_dashboard import ACTIVITY

OTHER = "00143e34985dca6fddc9fb369940e4c7d8e2873f529c"
TX1, TX2, TX3, PREV = "a1" * 32, "b2" * 32, "c3" * 32, "d4" * 32
LTX, CTX = "e5" * 32, "f6" * 32
# A legacy one-input, one-output transaction whose hex sizes the graph.
RAW_HEX = "01000000" "01" + "00" * 36 + "00" + "ffffffff" "01" + "00" * 8 + "00" "00000000"
SPEND = {"txid": TX1, "version": 2, "locktime": 0, "vsize": 141, "raw_hex": RAW_HEX,
         "status": {"confirmed": True, "block_height": 800000},
         "vin": [{"txid": PREV, "vout": 0, "prevout": {"scriptpubkey": SCRIPT, "value": 60000,
                                                     "scriptpubkey_address": ADDR}},
                 {"txid": "00" * 32, "vout": 4294967295, "is_coinbase": True}],
         "vout": [{"n": 0, "scriptpubkey": OTHER, "value": 0.0004, "scriptpubkey_type": "v0_p2wpkh"},
                  {"n": 1, "scriptpubkey": SCRIPT, "value_sats": 19000},
                  {"n": 2, "scriptpubkey": "6a0474657374", "value": 0},
                  "junk"]}
PARTIAL = {"txid": TX2, "vin": [{"txid": TX1, "vout": 1}, {"txid": PREV, "vout": "x"}],
           "vout": [{"scriptpubkey": SCRIPT, "value": "0.00018"}, {"value": 1.5e-05, "pegout_address": "bc1qpeg"}]}
LIQUID = {"txid": LTX, "chain": "liquid", "network": "liquidv1",
          "vin": [{"txid": TX3, "vout": 0, "is_pegin": True, "prevout": {"valuecommitment": "08aa"}}],
          "vout": [{"n": 0, "valuecommitment": "09bb", "scriptpubkey": SCRIPT},
                   {"n": 1, "scriptpubkey_type": "fee", "value_sats": 250},
                   {"n": 2, "asset": "ce" * 32, "value_sats": 5, "scriptpubkey": OTHER}]}
CHANGE = {"txid": CTX, "vin": [{"txid": TX1, "vout": 1, "prevout": {"scriptpubkey": SCRIPT, "value_sats": 19000}}],
          "vout": [{"n": 0, "scriptpubkey": SCRIPT, "value_sats": 18000}, {"n": 1, "scriptpubkey": OTHER, "value_sats": 500}]}
GRAPH = ACTIVITY + """
INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES
 ('watch','w','p','Watch','address','{"addresses":["%(addr)s"],"chain":"bitcoin","network":"mainnet"}','2025');
INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,occurred_at,confirmed_at,created_at,kind,raw_json,privacy_boundary) VALUES
 ('g1','w','p','watch','g1','%(tx1)s','BTC','outbound',41000000,1000000,'2025-05-01T00:00:00Z','2025','2025','withdrawal','%(spend)s',NULL),
 ('g2','w','p','watch','g2','%(tx2)s','BTC','inbound',18000000,0,'2025-05-02T00:00:00Z',NULL,'2025','deposit','%(partial)s','coinjoin'),
 ('g3','w','p','liquid','g3','%(ltx)s','LBTC','inbound',1000,0,'2025-05-03T00:00:00Z',NULL,'2025','peg-in','%(liquid)s',NULL),
 ('g4','w','p','exchange','g4','not-a-txid','BTC','inbound',5,0,'2025-05-04T00:00:00Z',NULL,'2025','buy','{}',NULL),
 ('g5','w','p','liquid','g5','%(tx3)s','LBTC','inbound',7,0,'2025-05-05T00:00:00Z',NULL,'2025','deposit','[]',NULL),
 ('g6','w','p','watch','g6','%(ctx)s','BTC','outbound',500000,500000,'2025-05-06T00:00:00Z',NULL,'2025','withdrawal','%(change)s',NULL);
INSERT INTO wallet_utxos(workspace_id,profile_id,wallet_id,chain,network,asset,amount,txid,vout,outpoint,confirmation_status,first_seen_at,last_seen_at,address,script_pubkey,branch_label,block_height,spent_by) VALUES
 ('w','p','watch','bitcoin','mainnet','BTC',60000000,'%(prev)s',0,'%(prev)s:0','confirmed','2025','2025','%(addr)s','%(script)s','change',799990,'%(tx1)s'),
 ('w','p','watch','bitcoin','main','BTC',19000000,'%(tx1)s',1,'%(tx1)s:1','confirmed','2025','2025','%(addr)s','%(script)s','receive',NULL,'%(tx2)s'),
 ('w','p','watch','bitcoin','main','BTC',18000000,'%(ctx)s',0,'%(ctx)s:0','confirmed','2025','2025','%(addr)s','%(script)s','m/1 change',NULL,NULL);
INSERT INTO journal_quarantines(transaction_id,workspace_id,profile_id,reason,detail_json,created_at)
 VALUES('g1','w','p','missing_fee_price','{"asset":"BTC","descriptor":"x","note":"n","nested":{"a":1}}','2025');
""" % {"addr": ADDR, "script": SCRIPT, "tx1": TX1, "tx2": TX2, "tx3": TX3, "ltx": LTX, "prev": PREV,
       "ctx": CTX, "change": json.dumps(CHANGE), "spend": json.dumps(SPEND), "partial": json.dumps(PARTIAL), "liquid": json.dumps(LIQUID)}
CURRENT = """
UPDATE profiles SET last_processed_at='2025',last_processed_tx_count=19,journal_input_version=2,
 last_processed_input_version=2,journal_semantics_version=2 WHERE id='p';
INSERT INTO journal_custody_decisions(decision_id,workspace_id,profile_id,source_transaction_id,target_transaction_id,source_observation_hash,source_start_msat,source_end_msat,target_observation_hash,target_start_msat,target_end_msat,source_wallet_id,target_wallet_id,source_network,target_network,source_rail,target_rail,source_asset,target_asset,state,basis_state,reason,atomic_group_id,component_id,occurred_at,created_at)
 VALUES('eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee','w','p','g1','g2','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',0,18000000,'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',0,18000000,'watch','watch','main','main','bitcoin','bitcoin','BTC','BTC','internal_verified','eligible','owned_destination',NULL,NULL,'2025-05-01','2025');
INSERT INTO journal_custody_economic_relations(relation_id,workspace_id,profile_id,relation_kind,source_transaction_id,target_transaction_id,source_asset,target_asset,source_amount_msat,target_amount_msat,basis_state,occurred_at,created_at)
 VALUES('ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff','w','p','conversion','g1','g3','BTC','LBTC',5000,4000,'eligible','2025','2025');
"""


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustTransactionGraphTest(DesktopParity, unittest.TestCase):
    KIND = "ui.transactions.graph"

    def test_refusals_and_empty_payloads_match_wire_bytes(self):
        self.change(GRAPH)
        self.compare([..., None, [], "x", {}, {"transaction": " "}, {"transaction": None},
                      {"z": 1, "a": 2, "transaction": "g1"}, {"transaction": "absent"},
                      {"transaction": "g1", "allowPublicLookup": True, "allow_public_lookup": False},
                      {"transaction": "g1", "allowPublicLookup": "maybe"},
                      {"transaction": "g1", "allow_public_lookup": 1},
                      {"transaction": "g1", "allowPublicLookup": 1, "allow_public_lookup": True}])
        self.change("DELETE FROM settings WHERE key IN ('context_workspace','context_profile');")
        self.compare([{"transaction": "g1"}, {"transaction": "absent"}])

    def test_local_graphs_match_wire_bytes(self):
        self.change(GRAPH)
        args = [{"transaction": ref} for ref in ("g1", TX1, TX2, "g3", "g4", "g5", "g6", "h", "k", "a", " g2 ")]
        args += [{"transaction": "g1", "allowPublicLookup": False},
                 {"transaction": "g4", "allow_public_lookup": "TRUE"},
                 {"transaction": "g3", "allowPublicLookup": True}]
        responses = self.compare(args)
        graph = responses[0]["data"]
        self.assertEqual(graph["supportLevel"], "partial")
        self.assertIn("owned", {node["ownership"] for node in graph["inputs"] + graph["outputs"]})
        self.assertEqual(graph["transaction"]["size"], 60)
        self.assertEqual(responses[6]["data"]["outputs"][0]["role"], "change")

    def test_current_custody_projection_annotates_the_graph(self):
        self.change(GRAPH + CURRENT)
        self.compare([{"transaction": ref} for ref in ("g1", "g2", "g3")])

    def test_several_linked_groups_keep_one_order_under_every_hash_seed(self):
        # tests/rust_port/repro_graph_group_order_hashseed.py: Python sorts the
        # group ids (round r7-6) where it used set order; Rust always sorted.
        extra = "".join(
            "INSERT INTO journal_custody_decisions(decision_id,workspace_id,profile_id,source_transaction_id,"
            "target_transaction_id,source_observation_hash,source_start_msat,source_end_msat,target_observation_hash,"
            "target_start_msat,target_end_msat,source_wallet_id,target_wallet_id,source_network,target_network,"
            "source_rail,target_rail,source_asset,target_asset,state,basis_state,reason,atomic_group_id,component_id,"
            f"occurred_at,created_at) VALUES('{name * 64}','w','p','g1','{target}','{'a' * 64}',0,1000,'{'b' * 64}',"
            "0,1000,'watch','watch','main','main','bitcoin','bitcoin','BTC','BTC','internal_verified','eligible',"
            f"'owned_destination',NULL,NULL,'2025-05-01','2025');\n"
            for name, target in (("1", "g2"), ("9", "g6"), ("5", "g2"), ("c", "g6"))
        )
        # Output 0 of g1 lands in another owned wallet: an owned destination.
        extra += ("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES"
                  " ('second','w','p','Second','address','{\"chain\":\"bitcoin\",\"network\":\"mainnet\"}','2025');\n"
                  "INSERT INTO wallet_utxos(workspace_id,profile_id,wallet_id,chain,network,asset,amount,txid,vout,outpoint,"
                  "confirmation_status,first_seen_at,last_seen_at,address,script_pubkey,branch_label,block_height,spent_by)"
                  f" VALUES('w','p','second','bitcoin','main','BTC',40000000,'{TX1}',0,'{TX1}:0','confirmed','2025','2025',"
                  f"NULL,'{OTHER}','receive',NULL,NULL);\n")
        self.change(GRAPH + CURRENT + extra)
        # The merged book's open-time migration resets the built semantics:
        # stamp them again so the custody projection is current.
        self.change("UPDATE profiles SET journal_semantics_version=2 WHERE id='p';")
        groups = []
        for seed in ("0", "1", "2", "3"):
            self.env["PYTHONHASHSEED"] = seed
            responses = self.compare([{"transaction": "g1"}])
            groups.append([[a.get("groupId") for a in node["annotations"] if a["code"] == "linked_transfer_group"]
                           for node in responses[0]["data"]["outputs"]])
        self.assertEqual(len({json.dumps(order) for order in groups}), 1)
        self.assertGreaterEqual(max(len(order) for order in groups[0]), 2, groups[0])

    def test_public_lookup_is_refused_before_any_backend(self):
        self.change(GRAPH)
        requests = self.requests([{"transaction": "g2", "allowPublicLookup": True}])
        wire = "".join(json.dumps(r) + "\n" for r in [*requests, dict(kind="daemon.shutdown")])
        result = run_candidate(["--machine", "--data-root", str(self.root), "daemon"],
                               env=self.env, input=wire.encode())
        response = json.loads(result.stdout.splitlines()[1])
        self.assertEqual(response["error"]["code"], "not_implemented")

    def test_sqlcipher_graph_matches_wire_bytes(self):
        self.encrypt("synthetic graph key")
        self.change(GRAPH + CURRENT)
        self.compare([{"transaction": "g1"}, {"transaction": "g3"}])
