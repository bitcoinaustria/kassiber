"""Desktop wallet onboarding: native identification, ledger preview and
script-type detection compared byte for byte with the Python daemon.

The detection probe runs under KASSIBER_NO_EGRESS here, so both daemons stop
at the transport gate and report the same fallback without any traffic.
"""
import base64
import json
import socket
import threading
import unittest

from kassiber.db import open_db
from kassiber.egress_policy import set_offline_mode
from kassiber.core.wallets import load_wallet_descriptor_plan_from_config
from kassiber.wallet_descriptors import derive_descriptor_targets
from tests.rust_desktop_parity import DESKTOP_SQL, RUST_BIN, DesktopParity, run_candidate
from tests.test_silent_payments import _BASE, _CHANGE_CASE, _LABELED, _SCAN, _SPEND, spscan_descriptor

XPUB = (
    "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189o"
    "LKnC5fSwqPfgyP3hooxujYzAu3fDVmz"
)
MASTER = (
    "xpub661MyMwAqRbcFtXgS5sYJABqqG9YLmC4Q1Rdap9gSE8NqtwybGhePY2gZ29ESFjqJoCu1Rupje8Yt"
    "GqsefD265TMg7usUDFdp6W1EGMcet8"
)
TPUB = (
    "tpubD6NzVbkrYhZ4XgiXtGrdW5XDAPFCL9h7we1vwNCpn8tGbBcgfVYjXyhWo4E1xkh56hjod1RhGjxba"
    "TLV3X4FyWuejifB9jusQ46QzG87VKp"
)
BOLT11 = (
    "lnbc1pvjluezsp5zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zygspp5qqqsyqcyq5"
    "rqwzqfqqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqypqdpl2pkx2ctnv5sxxmmwwd5kgetjypeh2ursdae8g"
    "6twvus8g6rfwvs8qun0dfjkxaq9qrsgq357wnc5r2ueh7ck6q93dj32dlqnls087fxdwk8qakdyafkq3ya"
    "p9us6v52vjjsrvywa6rt52cm9r9zqt8r2t7mlcwspyetp5h2tztugp9lfyql"
)
PAYMENT_HASH = "0001020304050607080900010203040506070809000102030405060708090102"
XPRV = (
    "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5k"
    "ejMRNNU3TGtRBeJgk33yuGBxrMPHi"
)
HOT = {"xpub": XPUB, "script_types": ["p2wpkh", "p2tr"], "network": "main", "gap_limit": 5}
COLD = {"descriptor": f"wpkh({MASTER}/0/*)", "change_descriptor": f"wpkh({MASTER}/1/*)"}
RETIRED = {"descriptor": f"tr({MASTER}/0/*)", "scan_to_index": 3}


def targets(config, end):
    plan = load_wallet_descriptor_plan_from_config(config)
    return list(derive_descriptor_targets(plan, branch_index=None, start=0, end=end))


def tx_graph(txid, inputs, outputs):
    return json.dumps({
        "txid": txid,
        "vin": [{"txid": prev, "vout": vout, "prevout": {"scriptpubkey": script} if script else None}
                for prev, vout, script in inputs],
        "vout": [{"scriptpubkey": script, "value": 1000 + n} for n, script in enumerate(outputs)],
    })


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustWalletIdentifyTest(DesktopParity, unittest.TestCase):
    KIND = "ui.wallets.identify"

    def seed(self):
        hot = targets(HOT, 8)
        self.hot = {(t.branch_label, t.address_index): t for t in hot}
        cold = targets(COLD, 4)
        self.cold = {(t.branch_label, t.address_index): t for t in cold}
        self.retired = targets(RETIRED, 4)
        receive, change = self.hot[("p2wpkh receive", 2)], self.hot[("p2wpkh change", 1)]
        deep = targets(HOT, 521)[520]
        self.deep = deep
        external = "0014" + "ee" * 20
        self.txids = {name: (char * 64) for name, char in
                      (("out", "1"), ("in", "2"), ("self", "3"), ("core", "4"), ("unres", "5"), ("receipt", "6"),
                      ("pay", "c"))}
        self.listed = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
        wallets = [
            ("hot", "Hot", "xpub", HOT),
            ("cold2", "Cold storage", "descriptor", COLD),
            ("list", "Watch list", "address", {"addresses": f" {self.listed} , 1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2, {receive.address}"}),
            ("dup", "Duplicate", "address", {"addresses": [cold[0].address]}),
            ("sp", "Silent", "silent-payment", {"sp_descriptor": spscan_descriptor(_SCAN, _SPEND), "network": "main"}),
            ("sp2", "Broken silent", "silent-payment", {"sp_descriptor": "sp(spscan1qqqq)", "network": "main"}),
            ("deep", "Deep", "xpub", {"xpub": MASTER, "script_types": ["p2pkh"], "ownership_scan_to_index": "abc"}),
            ("node", "Node", "lnd", {}),
            ("node2", "Second node", "coreln", {}),
            ("liq", "Liquid list", "liquid", {"chain": "liquid", "addresses": ["lq1qqfake0000000000000000"]}),
        ]
        statements = []
        for wallet_id, label, kind, config in wallets:
            statements.append(("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                               "VALUES(?,?,?,?,?,?,?)", (wallet_id, "w", "p", label, kind, json.dumps(config), "2025")))
        statements.append(("INSERT INTO wallet_policy_epochs(id,workspace_id,profile_id,wallet_id,chain,network,status,"
                           "private_material_json,created_at) VALUES('e1','w','p','cold2','bitcoin','main','retired',?,'2024')",
                           (json.dumps(RETIRED),)))
        statements.append(("INSERT INTO wallet_policy_epochs(id,workspace_id,profile_id,wallet_id,chain,network,status,"
                           "private_material_json,created_at) VALUES('e2','w','p','deep','bitcoin','main','retired',?,'2024')",
                           (json.dumps({"xpub": MASTER, "script_types": ["p2pkh"], "scan_to_index": "x"}),)))
        utxo = ("INSERT INTO wallet_utxos(id,workspace_id,profile_id,wallet_id,chain,network,asset,amount,txid,vout,outpoint,"
                "confirmation_status,address,script_pubkey,branch_label,branch_index,address_index,anon_history_json,"
                "first_seen_at,last_seen_at,raw_json) VALUES(?,'w','p',?,?,?,'BTC',1,?,?,?,'confirmed',?,?,?,?,?,'[]','2025','2025','{}')")
        statements += [
            (utxo, ("u1", "hot", "", "", self.txids["in"], 0, self.txids["in"] + ":0", deep.address, None,
                    deep.branch_label, 0, 520)),
            (utxo, ("u2", "hot", "bitcoin", "main", "7" * 64, 1, "7" * 64 + ":1", None, external, "receive", 0, 3)),
            (utxo, ("u3", "liq", "", "", "8" * 64, 0, "8" * 64 + ":0", "bc1qghost", None, None, None, None)),
        ]
        tx = ("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,"
              "asset,amount,fee,amount_includes_fee,excluded,raw_json,payment_hash,created_at) "
              "VALUES(?,'w','p',?,?,?,'2025-01-01T00:00:00Z',?,'BTC',1,0,0,0,?,?,'2025')")
        statements += [
            (tx, ("t1", "hot", self.txids["out"], "f1", "outbound",
                  tx_graph(self.txids["out"], [(self.txids["in"], 0, None)], [external, change.script_pubkey]), None)),
            (tx, ("t2", "list", self.txids["self"], "f2", "outbound",
                  tx_graph(self.txids["self"], [("9" * 64, 3, receive.script_pubkey)],
                           [receive.script_pubkey, "6a0102", ""]), None)),
            (tx, ("t3", "list", self.txids["core"], "f3", "inbound",
                  json.dumps([{"txid": self.txids["core"].upper(), "address": self.listed, "vout": 2},
                              {"txid": self.txids["core"], "address": self.listed, "vout": True},
                              {"txid": self.txids["core"], "address": " ", "vout": 1}]), None)),
            (tx, ("t4", "node", "ln-1", "f4", "outbound", "{}", PAYMENT_HASH)),
            (tx, ("t5", "node2", "ln-2", "f5", "inbound", "{}", PAYMENT_HASH)),
            (tx, ("t6", "hot", self.txids["unres"], "f6", "inbound",
                  tx_graph(self.txids["unres"], [("a" * 64, 0, None)], [receive.script_pubkey]), None)),
            (tx, ("t7", "hot", self.txids["receipt"], "f7", "inbound",
                  tx_graph(self.txids["receipt"], [("d" * 64, 0, "0014" + "dd" * 20)], [receive.script_pubkey]), None)),
            (tx, ("t8", "list", self.txids["pay"], "f8", "outbound",
                  tx_graph(self.txids["pay"], [(self.txids["core"], 2, None), ("e" * 64, 1, "0014" + "ab" * 20)],
                           ["0014" + "ff" * 20, change.script_pubkey]), None)),
        ]
        with open_db(self.root, passphrase=self.passphrase) as conn:
            conn.executescript(DESKTOP_SQL.read_text())
            for sql, params in statements:
                conn.execute(sql, params)
        self.change("")

    def test_tokens_and_formats_match_wire_bytes(self):
        invalid_invoice = BOLT11[:-1] + ("q" if BOLT11[-1] != "q" else "p")
        beyond = "sp1qqgste7k9hx0qftg6qmwlkqtwuy6cycyavzmzj85c6qdfhjdpdjtdgqjuexzk6murw56suy3e0rd2cgqvycxttddwsvgxe2usfpxumr70xc9pkqwq"
        self.compare([
            ..., None, [], "x", {}, {"addresses": 3}, {"txids": {"a": 1}}, {"candidates": [1, True, None, 2.5, "x"]},
            {"addresses": [], "txids": "nothex"}, {"scan_to_index": True, "candidates": "BC1QAR0SRRR7XFKVY5L643LYDNW9RE59GTZZWF5MDQ"},
            {"candidates": [BOLT11, "lightning:" + BOLT11.upper(), invalid_invoice, "LNO1" + "q" * 60,
                            "lnurl1dp68gurn8ghj7", "satoshi@example.com", "a" * 64 + ":1", MASTER, XPRV,
                            f"wpkh({XPRV}/0/*)", "not an address", "#comment", "", "  ", "short",
                            _BASE, _BASE.upper(), _LABELED[1], _CHANGE_CASE["change"], beyond,
                            _BASE[:-1] + "q", "tsp1" + "q" * 20]},
            {"text": "\n".join([self.listed, "# skipped", "", "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2",
                                "aa" * 32, " " + "AB" * 32, "ab " * 21 + "a"]) + "\r\nlq1qqfake0000000000000000 xyz"},
            {"txids": ["G" * 64, "ab" * 31, "AB" * 32], "addresses": ["bc1q!", "ab" * 32]},
        ])

    def test_owned_external_ambiguous_and_retired_matches_match_wire_bytes(self):
        hot_receive = self.hot[("p2wpkh receive", 2)]
        cold_receive = self.cold[(self.cold[next(iter(self.cold))].branch_label, 0)]
        self.compare([
            {"candidates": [hot_receive.address, self.hot[("p2tr change", 7)].address, cold_receive.address,
                            self.retired[2].address, self.retired[3].address, self.deep.address,
                            "bc1qghost", self.listed, self.listed.upper(), "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2",
                            "lq1qqfake0000000000000000", "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdx"]},
            {"addresses": [self.hot[("p2wpkh receive", 7)].address], "scan_to_index": 0},
            {"addresses": [self.hot[("p2wpkh receive", 7)].address], "scan_to_index": -4},
            {"addresses": [self.retired[3].address, self.hot[("p2wpkh receive", 7)].address], "scan_to_index": 1},
        ])

    def test_txids_invoices_and_csv_harvest_match_wire_bytes(self):
        txids = list(self.txids.values()) + ["b" * 64, self.txids["out"].upper()]
        rows = ["Date;TxID;Address;Amount", f"2025;{self.txids['out']};{self.listed};1",
                f'2025;"{self.txids["self"]}";"x {self.listed}";2', "2025;deadbeef;1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2;3"]
        self.compare([
            {"txids": txids},
            {"candidates": [BOLT11], "txids": [PAYMENT_HASH]},
            {"csv_text": "﻿" + "\r\n".join(rows)},
            {"csv_text": "address,amount\n" + self.listed + ',"unterminated\n' + "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2\n"},
            {"csv_text": "a\rb," + self.listed + "\n" + self.txids["core"]},
            {"csv_text": " "},
            {"csv_text": self.listed, "text": self.listed, "candidates": [self.listed]},
        ])

    def test_no_active_profile_and_sqlcipher_match_wire_bytes(self):
        self.change("DELETE FROM settings WHERE key='context_profile';")
        self.compare([{"candidates": [self.listed]}, "bad"])
        self.change("INSERT INTO settings(key,value) VALUES('context_profile','p');")
        self.encrypt("synthetic identify key")
        self.compare([{"candidates": [self.listed, BOLT11, _BASE]}])


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustLedgerPreviewTest(DesktopParity, unittest.TestCase):
    KIND = "ui.wallets.ledger_preview"

    def upload(self, text, **extra):
        return {"source_bytes_base64": base64.b64encode(text.encode()).decode(), **extra}

    def test_uploads_and_refusals_match_wire_bytes(self):
        ledger = "Type,Date,Asset,Amount,Fee\nBuy,2025-01-02,BTC,0.5,0.0001\nSell,bad,BTC,x,0\n"
        custom = "Wann;Menge;Notiz\n2025-01-02;0,25;hi\n"
        self.compare([
            ..., None, [], {}, {"source_bytes_base64": ""}, {"source_bytes_base64": 3},
            {"source_bytes_base64": "AA==", "filename": 4}, {"source_bytes_base64": "AA==", "filename": "x.pdf"},
            {"source_bytes_base64": "AA==", "filename": ".csv"}, {"source_bytes_base64": "AA==", "filename": "x."},
            {"source_bytes_base64": "AA=A"}, {"source_bytes_base64": "AAAA=", "filename": "dir/x.CSV"},
            self.upload(ledger), self.upload(ledger, filename="ledger.TSV"), self.upload(ledger, limit=1),
            self.upload(ledger, limit="1"), self.upload(ledger, limit=-1), self.upload(ledger, limit=2.9),
            self.upload(ledger, limit=[]), self.upload(ledger, limit=True), self.upload(ledger, column_map=[]),
            self.upload(custom), self.upload(custom, column_map={"date": "Wann", "amount": "Menge"}),
            self.upload(custom, column_map={"date": "Nope"}),
            self.upload("preamble line\nName,Konto\nDate,Amount\n2025-01-01,1\n"),
            self.upload(""), self.upload("﻿"), self.upload("x", filename="x.xlsx"),
        ])

    def test_sqlcipher_preview_matches_wire_bytes(self):
        self.encrypt("synthetic preview key")
        self.compare([self.upload("Date,Amount\n2025-01-01,1\n")])


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustScriptTypeDetectionTest(DesktopParity, unittest.TestCase):
    KIND = "ui.wallets.detect_script_types"

    def test_fallbacks_and_refusals_match_wire_bytes(self):
        self.compare([
            ..., None, [], {}, {"wallet_material": " "}, {"wallet_material": 3},
            {"wallet_material": XPRV}, {"wallet_material": f"wpkh({XPUB}/0/*)"}, {"wallet_material": "xpubbogus"},
            {"wallet_material": XPUB}, {"wallet_material": XPUB, "chain": 3}, {"wallet_material": XPUB, "network": []},
            {"wallet_material": XPUB, "backend": "nope"}, {"wallet_material": XPUB, "backend": " FULCRUM "},
            {"wallet_material": XPUB, "backend": "liquid"}, {"wallet_material": XPUB, "backend": "mempool"},
            {"wallet_material": TPUB}, {"wallet_material": TPUB, "network": "signet", "backend": "nope"},
            {"wallet_material": XPUB, "chain": "doge"}, {"wallet_material": XPUB, "chain": "liquid"},
            {"wallet_material": XPUB, "network": "bogus"}, {"wallet_material": XPUB, "network": "testnet"},
        ])

    def test_saved_backends_and_offline_mode_match_wire_bytes(self):
        self.change("INSERT INTO backends(name,kind,chain,network,url,created_at,updated_at) VALUES"
                    "('core','bitcoinrpc','bitcoin','main','http://127.0.0.1:8332','2025','2025'),"
                    "('testesplora','esplora','bitcoin','test','https://example.invalid/api','2025','2025');")
        self.compare([{"wallet_material": XPUB, "backend": "core"}, {"wallet_material": XPUB, "backend": "testesplora"},
                      {"wallet_material": TPUB, "backend": "testesplora"}])
        # Offline mode alone (no process override) also stops the probe before any socket.
        set_offline_mode(True, self.base / "preferences" / "offline.json")
        del self.env["KASSIBER_NO_EGRESS"]
        self.compare([{"wallet_material": XPUB}, {"wallet_material": TPUB, "backend": "testesplora"}])


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustOnboardingEgressTest(DesktopParity, unittest.TestCase):
    """Local onboarding kinds never reach the configured default backend."""

    KIND = "ui.wallets.identify"

    def test_local_kinds_send_nothing_and_only_detection_connects(self):
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen(8)
        listener.settimeout(0.2)
        accepted = []
        stop = threading.Event()

        def serve():
            while not stop.is_set():
                try:
                    connection, _ = listener.accept()
                except OSError:
                    continue
                accepted.append(connection)
                connection.close()

        thread = threading.Thread(target=serve, daemon=True)
        thread.start()
        self.addCleanup(lambda: (stop.set(), thread.join(), listener.close()))
        port = listener.getsockname()[1]
        self.change("INSERT INTO backends(name,kind,chain,network,url,created_at,updated_at) VALUES"
                    f"('trap','esplora','bitcoin','main','http://127.0.0.1:{port}','2025','2025');"
                    "INSERT INTO settings(key,value) VALUES('default_backend','trap');")
        env = {**self.env, "KASSIBER_TEST_NO_EGRESS": "1"}
        env.pop("KASSIBER_NO_EGRESS")

        def session(requests):
            wire = "".join(json.dumps(dict(request_id=str(i), **r)) + "\n"
                           for i, r in enumerate([*requests, {"kind": "daemon.shutdown"}])).encode()
            result = run_candidate(["--machine", "--data-root", str(self.root), "daemon"], env=env, input=wire)
            self.assertEqual((result.returncode, result.stderr), (0, b""))
            return [json.loads(line) for line in result.stdout.splitlines()[1:-1]]

        upload = base64.b64encode(b"Date,Amount\n2025-01-01,1\n").decode()
        local = session([
            {"kind": "ui.wallets.identify", "args": {"candidates": ["bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq",
                                                                     "1" * 64, BOLT11]}},
            {"kind": "ui.wallets.ledger_preview", "args": {"source_bytes_base64": upload}},
            {"kind": "ui.wallets.detect_script_types", "args": {"wallet_material": f"wpkh({XPUB}/0/*)"}},
            {"kind": "ui.wallets.detect_script_types", "args": {"wallet_material": TPUB}},
        ])
        self.assertEqual([r["kind"] for r in local], ["ui.wallets.identify", "ui.wallets.ledger_preview", "error",
                                                      "ui.wallets.detect_script_types"])
        self.assertEqual(local[3]["data"]["reason"],
                         "Backend 'trap' is configured for main, but source refresh requires test")
        self.assertEqual(accepted, [])
        detected = session([{"kind": "ui.wallets.detect_script_types", "args": {"wallet_material": XPUB}}])
        self.assertEqual(detected[0]["data"]["probed"], False)
        self.assertTrue(accepted, "the explicit detection reaches the configured backend")
