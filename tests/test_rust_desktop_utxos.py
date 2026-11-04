"""Desktop wallet UTXO inventory served by the Rust daemon, compared with Python.

The inventory is a stored read: both daemons must answer byte for byte and
leave all 112 tables unchanged. Backends are created by the Python daemon.
"""
import json
import unittest

from tests import cli_runner
from tests.rust_desktop_parity import RUST_BIN, DesktopParity, run_candidate
from tests.test_rust_desktop_backends import python_session

BACKENDS = [
    {"name": "electrum1", "kind": "electrum", "url": "ssl://electrum.example:50002"},
    {"name": "node", "kind": "lnd", "url": "https://lnd.example:8080", "token": "abcd"},
    {"name": "unused", "kind": "esplora", "url": "https://unused.example/api"},
    {"name": "spcore", "kind": "bitcoinrpc", "url": "http://127.0.0.1:18443",
     "config": {"username": "synthetic-user", "password": "synthetic-password", "silent_payments": "yes"}},
    {"name": "regnet", "kind": "esplora", "url": "https://regnet.example/api", "chain": "bitcoin",
     "network": "regtest"},
]

TXID_A = "a" * 64
TXID_B = "b" * 64
WALLETS = f"""
INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES
 ('u-desc','w','p','Desc','descriptor',
  '{{"backend":"electrum1","descriptor":"wpkh(synthetic-private-descriptor)","chain":"bitcoin","network":"regtest","last_synced_at":"2025-03-02"}}','2025'),
 ('u-label','w','p','u-desc','descriptor','{{"backend":"electrum1","descriptor":"x"}}','2025'),
 ('u-regnet','w','p','Regnet','descriptor','{{"backend":"regnet","descriptor":"x","network":"main"}}','2025'),
 ('u-addr','w','p','Addr','address','{{"addresses":["bcrt1qsynthetic"]}}','2025'),
 ('u-noaddr','w','p','No addr','address','{{"backend":"electrum1"}}','2025'),
 ('u-lnd','w','p','Lnd','descriptor','{{"backend":"node","descriptor":"x"}}','2025'),
 ('u-missing','w','p','Missing','descriptor','{{"backend":"ghost","descriptor":"x"}}','2025'),
 ('u-sp-esplora','w','p','SP esplora','silent-payment','{{"sp_descriptor":"sp(x)","backend":"unused"}}','2025'),
 ('u-sp-core','w','p','SP core','silent-payment','{{"sp_descriptor":"sp(x)","backend":"spcore"}}','2025'),
 ('u-sp-lnd','w','p','SP lnd','silent-payment','{{"sp_descriptor":"sp(x)","backend":"node"}}','2025'),
 ('u-liquid-bad','w','p','Liquid bad','descriptor','{{"backend":"electrum1","chain":"liquid","descriptor":"wpkh(not-a-key)"}}','2025'),
 ('u-liquid-nodesc','w','p','Liquid none','address','{{"backend":"electrum1","chain":" Liquid ","addresses":["x"]}}','2025'),
 ('u-wasabi','w','p','Wasabi','descriptor','{{"source_format":"wasabi_bundle","source_file":"w.json","network":"regtest"}}','2025'),
 ('u-pay','w','p','Pay','descriptor','{{"sync_source":"btcpay","descriptor":"x","backend":"electrum1"}}','2025'),
 ('u-never','w','p','Never','descriptor','{{"backend":"electrum1","descriptor":"x","chain":"btc","network":"test"}}','2025'),
 ('u-bad-config','w','p','Bad config','descriptor','not json','2025'),
 ('u-many','w','p','Many','descriptor','{{"backend":"electrum1","descriptor":"x"}}','2025'),
 ('f-desc','w','foreign','Foreign desc','descriptor','{{"backend":"electrum1","descriptor":"x"}}','2025');
INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,
  occurred_at,confirmed_at,created_at,kind,kind_override,fiat_rate,fiat_value,review_status,note,description,counterparty,fiat_currency)
VALUES ('u-tx','w','p','u-desc','u-tx','{TXID_A.upper()}','BTC','inbound',1,0,'2025-02-01T00:00:00Z',NULL,'2025','deposit',
  NULL,NULL,NULL,'completed','','','',NULL);
INSERT INTO wallet_utxos(id,workspace_id,profile_id,wallet_id,backend_name,backend_kind,chain,network,asset,amount,txid,vout,
  outpoint,confirmation_status,confirmations,block_height,block_time,address,script_pubkey,address_label,branch_label,
  branch_index,address_index,anonymity_score,spent_by,excluded_from_coinjoin,key_state,anon_history_json,first_seen_at,
  last_seen_at,spent_at,raw_json) VALUES
 ('o1','w','p','u-desc','electrum1','electrum','bitcoin','regtest','BTC',150000000123,'{TXID_A}',1,'{TXID_A}:1','confirmed',3,
  100,'2025-02-01T00:00:00Z','bcrt1qaddress','0014ff','Savings','receive',0,4,7,'',1,'active','[1,2]','2025-02-01','2025-03-01',
  NULL,'{{"samourai":{{"account":"postmix"}},"other":1}}'),
 ('o2','w','p','u-desc','electrum1','electrum','bitcoin','regtest','BTC',1001,'{TXID_B}',0,'{TXID_B}:0','unconfirmed',0,
  NULL,NULL,NULL,'0014aa',NULL,NULL,NULL,NULL,NULL,NULL,0,NULL,'not json','2025-02-02','2025-02-03',NULL,'[1]'),
 ('o3','w','p','u-desc','electrum1','electrum','bitcoin','regtest','BTC',5000,'{TXID_B}',1,'{TXID_B}:1','confirmed',1,
  99,NULL,'bcrt1qspent',NULL,NULL,NULL,NULL,NULL,NULL,'cc',NULL,NULL,'{{}}','2025','2025','2025-03-03','{{}}'),
 ('o4','w','p','u-desc','old','electrum','bitcoin','regtest','BTC',777,'{TXID_B}',2,'{TXID_B}:2','confirmed',1,
  1,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,'[]','2025','2025',NULL,'{{}}'),
 ('o5','w','p','u-desc','electrum1','electrum','bitcoin','regtest','ABC',3,'{TXID_B}',3,'{TXID_B}:3','confirmed',1,
  0,NULL,'','',NULL,NULL,NULL,NULL,NULL,NULL,NULL,'','{{"a":1}}','2025','2025',NULL,'{{"samourai":3}}'),
 ('o6','w','p','u-wasabi','wasabi','wasabi_bundle','bitcoin','regtest','BTC',42000,'{TXID_A}',7,'{TXID_A}:7','confirmed',9,
  50,NULL,NULL,NULL,NULL,NULL,NULL,NULL,5,NULL,0,NULL,'[]','2025','2025-04-01',NULL,'{{}}'),
 ('o7','w','p','u-regnet','regnet','esplora','bitcoin','regtest','BTC',8,'{TXID_A}',8,'{TXID_A}:8','confirmed',1,
  5,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,'[]','2025','2025',NULL,'{{}}'),
 ('o8','w','p','u-regnet','regnet','esplora','bitcoin','main','BTC',9,'{TXID_A}',9,'{TXID_A}:9','confirmed',1,
  5,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,'[]','2025','2025',NULL,'{{}}');
INSERT INTO wallet_utxo_refreshes(wallet_id,workspace_id,profile_id,backend_name,backend_kind,chain,network,observed_count,
  active_count,last_seen_at) VALUES ('u-desc','w','p','electrum1','electrum','bitcoin','regtest',4,3,'2025-03-05');
WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 500)
INSERT INTO wallet_utxos(id,workspace_id,profile_id,wallet_id,backend_name,backend_kind,chain,network,asset,amount,txid,vout,
  outpoint,confirmation_status,first_seen_at,last_seen_at)
SELECT 'm' || i,'w','p','u-many','electrum1','electrum','bitcoin','main','BTC',1000 + i,
  printf('%064d', i),i,printf('%064d', i) || ':' || i,'confirmed','2025','2025' FROM n;
"""

WALLET_IDS = ["u-desc", "u-label", "u-regnet", "u-addr", "u-noaddr", "u-lnd", "u-missing", "u-sp-esplora",
              "u-sp-core", "u-sp-lnd", "u-liquid-bad", "u-liquid-nodesc", "u-wasabi", "u-pay", "u-never",
              "u-bad-config", "u-many"]
REFUSALS = [..., None, [], "x", 3, {}, {"wallet": ""}, {"wallet": "  "}, {"wallet": 3}, {"wallet": ["u-desc"]},
            {"connection": 3}, {"wallet": "ghost"}, {"wallet": "f-desc"},
            {"wallet": "foreign desc"}]


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustWalletUtxosTest(DesktopParity, unittest.TestCase):
    KIND = "ui.wallets.utxos"

    def configure(self):
        python_session(self, [dict(kind="ui.backends.create", args=b) for b in BACKENDS])
        self.change(WALLETS)

    def test_refusals_and_missing_profile(self):
        self.configure()
        responses = self.compare(REFUSALS)
        self.assertTrue(all(r["kind"] == "error" for r in responses[:-2]), responses)
        self.change("DELETE FROM settings WHERE key='context_profile';")
        responses = self.compare(REFUSALS[:4])
        self.assertTrue(all(r["kind"] == self.KIND for r in responses), responses)

    def test_every_support_branch_and_inventory_matches_wire_bytes(self):
        self.configure()
        args = [{"wallet": wallet} for wallet in WALLET_IDS]
        args += [{"wallet": " Desc "}, {"wallet": "", "connection": "Wasabi"},
                 {"wallet": None, "connection": " u-desc "}]
        responses = self.compare(args)
        by_wallet = {r["data"]["wallet"]["id"]: r["data"] for r in responses if r["kind"] == self.KIND}
        desc = by_wallet["u-desc"]
        self.assertEqual([u["id"] for u in desc["utxos"]], ["o5", "o1", "o2"])
        self.assertEqual(desc["utxos"][1]["transaction_id"], "u-tx")
        self.assertTrue(by_wallet["u-many"]["summary"]["truncated"])
        self.assertEqual(len(by_wallet["u-many"]["utxos"]), 500)
        wire = json.dumps(responses)
        for secret in ("synthetic-private-descriptor", "synthetic-password", "0014ff"):
            self.assertNotIn(secret, wire)
        statuses = {data["support"]["reason"] for data in by_wallet.values()}
        self.assertTrue({"not_chain_backed", "backend_missing", "backend_kind", "missing_sp_capability",
                         "wasabi_import", "liquid_descriptor_required"} <= statuses, statuses)
        for name in ("electrum1", "unused"):
            python_session(self, [dict(kind="ui.backends.set_default", args={"name": name})])
            self.compare([{"wallet": "u-addr"}, {"wallet": "u-desc"}])

    def test_surrogate_reference_matches_python_crash_code_and_message(self):
        """Reference bug: Python's lookup binds the reference as a SQLite
        parameter and crashes with an unhandled UnicodeEncodeError (traceback
        on stderr). Rust refuses with the same code and message; only the
        traceback-bearing ``debug`` field cannot be reproduced."""
        self.configure()
        refs = ["\ud800", "a\udc00\udc01b", " \udfff "]
        wire = "".join(json.dumps(dict(request_id=str(i), kind=self.KIND, args={"wallet": ref})) + "\n"
                       for i, ref in enumerate(refs)).encode() + b'{"kind":"daemon.shutdown"}\n'
        argv = ["--machine", "--data-root", str(self.root), "daemon"]
        reference = cli_runner.run(argv, reference=True, env=self.env, input=wire, capture_output=True, timeout=300)
        candidate = run_candidate(argv, env=self.env, input=wire)
        self.assertIn(b"UnicodeEncodeError", reference.stderr)
        self.assertEqual((candidate.returncode, candidate.stderr), (0, b""))
        expected = [json.loads(line) for line in reference.stdout.splitlines()[1:-1]]
        actual = [json.loads(line) for line in candidate.stdout.splitlines()[1:-1]]
        self.assertEqual(len(actual), len(refs))
        for want, got in zip(expected, actual):
            self.assertTrue(want["error"].pop("debug"))
            self.assertIsNone(got["error"].pop("debug"))
            self.assertEqual(got, want)

    def test_expected_scope_and_sqlcipher_inventory(self):
        self.configure()
        scope = {"workspace_id": "w", "profile_id": "p"}
        self.compare([{"wallet": "u-desc", "expected_scope": scope},
                      {"wallet": "u-desc", "expected_scope": {**scope, "profile_id": "foreign"}}])
        self.encrypt("synthetic utxo key")
        self.configure()
        self.compare([{"wallet": "u-desc"}, {"wallet": "u-wasabi"}, "x"])


if __name__ == "__main__":
    unittest.main()
