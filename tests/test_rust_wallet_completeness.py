"""Main sync S8: wallet completeness and transaction verification.

`ui.wallets.list` and `wallets list` gained each wallet's completeness class
and scan bounds (#681); `transactions verify` checks one row against its
stored chain graph. Python's daemon and CLI are the reference; responses must
match byte for byte and no table may change.
"""
import json
import unittest

from tests import cli_runner
from tests.rust_desktop_parity import RUST_BIN, DesktopParity, run_candidate
from tests.test_rust_report_blockers import ADDR, SCRIPT
from tests.test_rust_transaction_graph import CURRENT, GRAPH, OTHER, TX1, TX2

WALLETS = """
INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES
 ('desc','w','p','Desc','descriptor','{"descriptor":"wpkh(x)","last_synced_at":" 2025-03-01T00:00:00Z ","birthday":"2024-01-01"}','2025'),
 ('desc-new','w','p','Desc new','descriptor','{"descriptor":"wpkh(y)"}','2025'),
 ('xpub-file','w','p','Xpub file','xpub','{"xpub":"x","script_types":[],"source_file":"/s.csv"}','2025'),
 ('xpub-live','w','p','Xpub live','xpub','{"xpub":"x","script_types":["p2tr"],"source_file":"/s.csv","last_synced_at":"2025"}','2025'),
 ('sp','w','p','SP','silent-payment','{"sp_descriptor":"sp(x)","sp_scan_start_height":800000,"sp_scan_start_date":" 2024-01-01 ","last_synced_at":"2025"}','2025'),
 ('sp-full','w','p','SP full','silent-payment','{"sp_descriptor":"sp(x)","sp_full_history":"YES","sp_scan_start_date":"2023-01-01","birthday":"2020-01-01","last_synced_at":"2025"}','2025'),
 ('sp-float','w','p','SP float','silent-payment','{"sp_descriptor":"sp(x)","sp_scan_start_height":800000.0,"last_synced_at":1.5,"sp_full_history":0}','2025'),
 ('sp-file','w','p','SP file','silent-payment','{"source_file":"/sp.json"}','2025'),
 ('samourai','w','p','Samourai','samourai','{"source_file":"/w.json"}','2025'),
 ('addr','w','p','Addresses','address','{"addresses":"bc1qa, ,","last_synced_at":"2025"}','2025'),
 ('addr-blank','w','p','Blank addresses','address','{"addresses":[" ",""]}','2025'),
 ('addr-dict','w','p','Dict addresses','address','{"addresses":{"bc1qb":1}}','2025'),
 ('node','w','p','Node','coreln','{"backend":"cln"}','2025'),
 ('node-file','w','p','Node file','lnd','{"source_file":"/n.csv"}','2025'),
 ('api','w','p','Kraken API','kraken','{"backend":"kraken","last_synced_at":"2025"}','2025'),
 ('api-file','w','p','Kraken file','binance','{"backend":"b","source_file":"/b.csv"}','2025'),
 ('btcpay','w','p','Store','custom','{"sync_source":" BTCPay "}','2025'),
 ('river','w','p','River','River','{"source_file":"/r.csv"}','2025'),
 ('manual','w','p','Manual','custom','{"source_format":"generic_ledger"}','2025'),
 ('manual-blank','w','p','Manual blank','custom','{"source_format":"  "}','2025'),
 ('custom-file','w','p','Custom file','custom','{"source_format":"bisq","source_file":"/c.csv"}','2025'),
 ('untracked','w','p','Untracked','untracked','{"birthday":true,"sync_source":"btcpay"}','2025'),
 ('phoenix','w','p','Phoenix','phoenix','[1,2]','2025'),
 ('empty-config','w','p','Empty config','descriptor','','2025');
INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,occurred_at,confirmed_at,created_at,excluded,raw_json) VALUES
 ('c1','w','p','desc','c1','%(a)s','BTC','outbound',5000,10,'2025-01-01T00:00:00Z','2025','2025',0,'{}'),
 ('c2','w','p','desc','c2','%(b)s','BTC','inbound',3000,0,'2025-01-02T00:00:00Z','2025','2025',0,'{}'),
 ('c3','w','p','desc','c3','%(c)s','BTC','inbound',1,0,'1970-01-01T00:00:00Z','2025','2025',0,'{}'),
 ('c4','w','p','addr','c4','%(d)s','BTC','inbound',100,0,'2025-02-01T00:00:00Z',NULL,'2025',0,'{"txid":"%(d)s","status":{"confirmed":false}}'),
 ('c5','w','p','addr','c5','%(e)s','BTC','outbound',150,0,'2025-01-15T00:00:00Z','2025','2025',0,'{}'),
 ('c6','w','p','sp','c6','%(f)s','BTC','inbound',150,0,'2025-01-15T00:00:00Z','2025','2025',1,'{}'),
 ('c7','w','p','api','c7',NULL,'ETH','outbound',7,0,'','','2025',0,'{}'),
 ('c8','w','p','api','c8',NULL,'BTC','inbound',7,0,'2025-04-01T00:00:00Z','','2025',0,'{"txid":"%(g)s","status":{"confirmed":false}}'),
 ('c12','w','p','api','c12',NULL,'BTC','outbound',5,0,'2025-03-01T00:00:00Z','2025','2025',0,'{}'),
 ('c9','w','p','river','c9',NULL,'BTC','outbound',9,0,'2025-04-02T00:00:00Z','2025','2025',0,'{}'),
 ('c10','w','p','river','c10',NULL,'BTC','outbound',9,1,'2025-04-02T00:00:00Z','2025','2025',0,'{}'),
 ('c11','w','p','river','c11',NULL,'BTC','inbound',20,0,'2025-04-02T00:00:00Z','2025','2025',0,'{}');
""" % {key: key * 64 for key in "abcdefg"}


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustWalletCompletenessTest(DesktopParity, unittest.TestCase):
    KIND = "ui.wallets.list"

    def cli(self, *args, fmt=None):
        """Run one CLI read on both sides; equal exit, bytes and tables."""
        argv = ["--data-root", str(self.root)]
        argv += ["--format", fmt] if fmt else ["--machine"]
        if self.passphrase:
            argv += ["--db-passphrase-fd", cli_runner.InputFd(self.passphrase.encode())]
        argv += list(args)
        baseline = self.snapshot()
        reference = cli_runner.run(argv, reference=True, env=self.env, capture_output=True, timeout=300)
        candidate = run_candidate(argv, env=self.env, input=b"")
        self.assertEqual((candidate.returncode, candidate.stdout, candidate.stderr),
                         (reference.returncode, reference.stdout, reference.stderr), args)
        self.assertEqual(self.snapshot(), baseline)
        return json.loads(candidate.stdout) if not fmt else candidate.stdout

    def test_classes_and_scan_bounds_match_wire_bytes(self):
        self.change(WALLETS)
        wallets = self.compare([..., None, {}])[0]["data"]["wallets"]
        classes = {wallet["id"]: wallet["completeness_class"] for wallet in wallets}
        self.assertEqual(
            {classes[key] for key in classes},
            {"descriptor-derived", "address-list", "lightning-node", "provider-api",
             "exchange-export", "file-import", "manual", "untracked"})
        bounds = {wallet["id"]: wallet["scan_bounds"] for wallet in wallets}
        self.assertEqual(bounds["desc"]["deficit_assets"], ["BTC"])
        self.assertIn("history_starts_after_first_funding", bounds["addr"]["warnings"])
        self.assertEqual(bounds["sp"]["history_before_first_transaction"], "declared_start")
        self.assertEqual(bounds["desc-new"]["warnings"], ["never_synced", "no_transactions"])
        self.compare([{}], kind="ui.wallets.list")
        # Stale journals and the no-context payload keep the same shape.
        self.change("UPDATE profiles SET journal_input_version=journal_input_version+1 WHERE id='p';")
        self.compare([{}])
        self.change("DELETE FROM settings WHERE key='context_profile';")
        self.compare([{}])

    def test_cli_list_matches_every_format(self):
        self.change(WALLETS)
        # The seed's undecodable configuration fails both CLIs the same way.
        self.assertEqual(self.cli("wallets", "list")["error"]["code"], "internal_error")
        self.change("UPDATE wallets SET config_json='{}' WHERE id='custom';")
        self.assertEqual(self.cli("wallets", "list")["error"]["code"], "internal_error")
        for raw in ("null", "3", '"x"'):
            self.change(f"UPDATE wallets SET config_json='{raw}' WHERE id='phoenix';")
            self.cli("wallets", "list")
        self.change("UPDATE wallets SET config_json='{}' WHERE id='phoenix';")
        rows = self.cli("wallets", "list")["data"]
        self.assertTrue(all("scan_bounds" in row for row in rows))
        for fmt in ("json", "table", "plain", "csv"):
            self.cli("wallets", "list", fmt=fmt)
        self.cli("wallets", "list", "--profile", "Other")
        self.cli("wallets", "list", "--profile", "absent")

    def test_malformed_sources_fail_like_python(self):
        self.change(WALLETS + """
            INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at)
            VALUES('bad-bool','w','p','A bad bool','silent-payment','{"sp_full_history":"maybe"}','2025');""")
        self.compare([{}])
        self.cli("wallets", "list")
        self.change("""DELETE FROM wallets WHERE id='bad-bool';
            INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at)
            VALUES('bad-addresses','w','p','A bad list','address','{"addresses":5}','2025');""")
        # Python's daemon reports this TypeError with a traceback; the code
        # and message are the contract.
        wire = "".join(json.dumps(r) + "\n" for r in [*self.requests([{}]), dict(kind="daemon.shutdown")])
        argv = ["--machine", "--data-root", str(self.root), "daemon"]
        lines = [cli_runner.run(argv, reference=True, env=self.env, input=wire.encode(),
                                capture_output=True, timeout=300).stdout.splitlines()[1],
                 run_candidate(argv, env=self.env, input=wire.encode()).stdout.splitlines()[1]]
        errors = [json.loads(line)["error"] for line in lines]
        self.assertEqual([(e["code"], e["message"]) for e in errors],
                         [("internal_error", "'int' object is not iterable")] * 2)

    def test_sqlcipher_list_matches_wire_bytes(self):
        self.encrypt("synthetic completeness key")
        self.change(WALLETS)
        self.compare([{}])
        self.cli("wallets", "list")


VERIFY_TX = {name: name[0] * 64 for name in ("1ok", "2fee", "3missing", "4several", "5excluded", "6ambiguous", "7nofee", "8vals")}
THIRD = "0014" + "11" * 20
PREVOUT = {"scriptpubkey": SCRIPT, "value": 60000, "scriptpubkey_address": ADDR}


def spend(txid, fee, outputs, prevout=PREVOUT):
    raw = {"txid": txid, "vin": [{"txid": "9" * 64, "vout": 0, "prevout": prevout}], "vout": outputs}
    if fee is not None:
        raw["fee"] = fee
    return json.dumps(raw).replace("'", "''")


OUTS = [{"n": 0, "scriptpubkey": OTHER, "value_sats": 39000}, {"n": 1, "scriptpubkey": SCRIPT, "value_sats": 20000},
        {"n": 2, "scriptpubkey": "6a0474657374", "value_sats": 0}]
VERIFY = GRAPH + """
INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES
 ('twin','w','p','Twin','custom','{}','2025');
INSERT INTO wallet_utxos(workspace_id,profile_id,wallet_id,chain,network,asset,amount,txid,vout,outpoint,confirmation_status,first_seen_at,last_seen_at,address,script_pubkey,branch_label,block_height,spent_by) VALUES
 ('w','p','cold','bitcoin','main','BTC',1000,'%(seen)s',0,'%(seen)s:0','confirmed','2025','2025',NULL,'0014%(cold)s','internal',NULL,NULL),
 ('w','p','twin','bitcoin','main','BTC',1000,'%(seen)s',1,'%(seen)s:1','confirmed','2025','2025',NULL,'%(third)s','receive',NULL,NULL),
 ('w','p','ln','bitcoin','main','BTC',1000,'%(seen)s',2,'%(seen)s:2','confirmed','2025','2025',NULL,'%(third)s','external',NULL,NULL);
INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,occurred_at,confirmed_at,created_at,kind,raw_json,excluded,amount_includes_fee) VALUES
 ('v1','w','p','watch','v1','%(1ok)s','BTC','outbound',39000000,1000000,'2025-06-01T00:00:00Z','2025','2025','withdrawal','%(r1)s',0,0),
 ('v1x','w','p','exchange','v1x','%(1ok)s','BTC','inbound',39000000,0,'2025-06-01T00:00:00Z','2025','2025','deposit','{}',0,0),
 ('v1c','w','p','cold','v1c','%(1ok)s','BTC','inbound',1,0,'2025-06-01T00:00:00Z','2025','2025','deposit','{}',0,0),
 ('v1t','w','p','twin','v1t','%(1ok)s','BTC','inbound',1,0,'2025-06-01T00:00:00Z','2025','2025','deposit','{}',0,0),
 ('v2','w','p','watch','v2','%(2fee)s','BTC','outbound',39500000,500000,'2025-06-02T00:00:00Z','2025','2025','withdrawal','%(r2)s',0,1),
 ('v3','w','p','exchange','v3','%(3missing)s','BTC','outbound',40000000,0,'2025-06-03T00:00:00Z','2025','2025','withdrawal','%(r3)s',0,0),
 ('v4','w','p','watch','v4','%(4several)s','BTC','outbound',39000000,1000000,'2025-06-04T00:00:00Z','2025','2025','withdrawal','%(r4)s',0,0),
 ('v4b','w','p','watch','v4b','%(4several)s','BTC','outbound',1,0,'2025-06-04T00:00:00Z','2025','2025','withdrawal','{}',0,0),
 ('v5','w','p','watch','v5','%(5excluded)s','BTC','outbound',39000000,1000000,'2025-06-05T00:00:00Z','2025','2025','withdrawal','%(r5)s',1,0),
 ('v6','w','p','watch','v6','%(6ambiguous)s','BTC','outbound',39000000,1000000,'2025-06-06T00:00:00Z','2025','2025','withdrawal','%(r6)s',0,0),
 ('v7','w','p','watch','v7','%(7nofee)s','BTC','outbound',39000000,1000000,'2025-06-07T00:00:00Z','2025','2025','withdrawal','%(r7)s',0,0),
 ('v8','w','p','watch','v8','%(8vals)s','BTC','outbound',39000000,1000000,'2025-06-08T00:00:00Z','2025','2025','withdrawal','%(r8)s',0,0);
""" % {
    **VERIFY_TX, "seen": "5" * 64, "cold": "22" * 20, "third": THIRD,
    "r1": spend(VERIFY_TX["1ok"], 1000, OUTS),
    "r2": spend(VERIFY_TX["2fee"], 900, OUTS),
    "r3": spend(VERIFY_TX["3missing"], 1000, OUTS),
    "r4": spend(VERIFY_TX["4several"], 1000, OUTS),
    "r5": spend(VERIFY_TX["5excluded"], 1000, OUTS),
    "r6": spend(VERIFY_TX["6ambiguous"], 1000, [*OUTS, {"n": 3, "scriptpubkey": THIRD, "value_sats": 0}]),
    "r7": spend(VERIFY_TX["7nofee"], None, OUTS),
    "r8": spend(VERIFY_TX["8vals"], 1000, [{"n": 0, "scriptpubkey": SCRIPT}, {"n": 1, "scriptpubkey": OTHER, "value_sats": 70000}]),
}


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustTransactionVerifyTest(DesktopParity, unittest.TestCase):
    cli = RustWalletCompletenessTest.cli

    def verify(self, refs):
        return [self.cli("transactions", "verify", "--transaction", ref) for ref in refs]

    def test_refusals_and_graphless_rows_match_bytes(self):
        self.change(GRAPH)
        self.verify(["", " ", "absent", "g4", "g5", "h", "k", "a"])
        self.cli("transactions", "verify", "--transaction", "g1", "--profile", "absent")
        self.cli("transactions", "verify")
        self.change("DELETE FROM settings WHERE key IN ('context_workspace','context_profile');")
        self.verify(["g1"])

    def test_stored_graphs_match_bytes(self):
        self.change(VERIFY)
        refs = ["g1", TX1, TX2, "g3", "g6", " g2 ", *(f"v{n}" for n in range(1, 9)), VERIFY_TX["1ok"]]
        results = self.verify(refs)
        statuses = {result["data"]["status"] for result in results if result["kind"] != "error"}
        self.assertEqual(statuses, {"ok", "mismatch", "partial"})
        ok = results[6]["data"]
        self.assertEqual(ok["conservation"]["status"], "ok")
        self.assertFalse(ok["network_contacted"])
        rendered = json.dumps(results)
        for secret in (ADDR, SCRIPT, OTHER):
            self.assertNotIn(secret, rendered)
        for fmt in ("table", "plain"):
            self.cli("transactions", "verify", "--transaction", "v1", fmt=fmt)

    def test_current_projection_and_sqlcipher_match_bytes(self):
        self.encrypt("synthetic verify key")
        self.change(VERIFY + CURRENT)
        self.verify(["g1", "g3", "v1", "v6"])
