"""Desktop backend list served by the Rust daemon, compared with Python.

Backends are created by the unchanged Python daemon first; the list itself is
compared byte for byte and must leave all 114 tables unchanged.
"""
import json
import unittest

from tests import cli_runner
from tests.rust_desktop_parity import RUST_BIN, DesktopParity

BACKENDS = [
    {"name": "node", "kind": "lnd", "url": "https://user:pw@lnd.example:8080/v1?macaroon=abcd", "token": "abcd"},
    {"name": "cln", "kind": "coreln", "url": "https://cln.example:3010", "token": "synthetic-rune"},
    {"name": "core", "kind": "bitcoinrpc", "url": "http://127.0.0.1:18443",
     "config": {"username": "synthetic-user", "password": "synthetic-password"}},
    {"name": "electrum1", "kind": "electrum", "url": "ssl://electrum.example:50002",
     "config": {"certificate": "/synthetic/ca.pem", "insecure": "yes"}},
    {"name": "btcpay1", "kind": "btcpay", "url": "https://pay.example", "token": "synthetic-api-key"},
    {"name": "unused", "kind": "esplora", "url": "https://unused.example/api"},
    {"name": "bare", "kind": "esplora", "url": "bare.example:3000"},
]

WALLETS = """
INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES
 ('w-node','w','p','Node','lnd','{"backend":" node "}','2025'),
 ('w-cln','w','p','CLN','coreln','{"backend":"cln"}','2025'),
 ('w-core','w','p','Core','descriptor','{"backend":"core","descriptor":"wpkh(synthetic)"}','2025'),
 ('w-xpub','w','p','Xpub','xpub','{"xpub":"synthetic","script_types":["p2wpkh"]}','2025'),
 ('w-xpub-empty','w','p','Xpub empty','xpub','{"xpub":"synthetic","script_types":[]}','2025'),
 ('w-addresses','w','p','Addresses','address','{"addresses":["bcrt1q"]}','2025'),
 ('w-pay','w','p','Pay','descriptor','{"sync_source":"btcpay","descriptor":"wpkh(x)","backend":"btcpay1"}','2025'),
 ('w-file','w','p','File','descriptor','{"source_file":"a.csv","source_format":"csv","descriptor":"wpkh(x)"}','2025'),
 ('w-sp','w','p','SP','silent-payment','{"sp_descriptor":"sp(x)"}','2025'),
 ('w-bare','w','p','Bare','address','{"backend":"bare"}','2025'),
 ('w-missing','w','p','Missing','address','{"backend":"ghost"}','2025'),
 ('w-nonstring','w','p','Nonstring','address','{"backend":5,"descriptor":"x"}','2025'),
 ('w-empty','w','p','Empty','address','','2025'),
 ('w-list','w','p','List','descriptor','[1]','2025'),
 ('w-foreign','w','foreign','Foreign2','descriptor','{"backend":"unused","descriptor":"x"}','2025');
"""
READ_ARGS = [..., None, [], "x", 0, {}, {"unexpected": True}]


def python_session(test, requests):
    wire = "".join(json.dumps(dict(request_id=str(i), **r)) + "\n" for i, r in enumerate(requests))
    argv = ["--machine", "--data-root", str(test.root)]
    if test.passphrase:
        argv.extend(["--db-passphrase-fd", cli_runner.InputFd(test.passphrase.encode())])
    result = cli_runner.run([*argv, "daemon"], reference=True, env=test.env,
                            input=(wire + '{"kind":"daemon.shutdown"}\n').encode(),
                            capture_output=True, timeout=300)
    test.assertEqual(result.returncode, 0, result.stderr)
    responses = [json.loads(line) for line in result.stdout.splitlines()[1:-1]]
    test.assertTrue(all(r["kind"] != "error" for r in responses), responses)
    return responses


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustBackendsListTest(DesktopParity, unittest.TestCase):
    KIND = "ui.backends.list"

    def configure(self):
        python_session(self, [dict(kind="ui.backends.create", args=b) for b in BACKENDS])
        self.change(WALLETS)

    def test_list_without_backends_or_profile(self):
        responses = self.compare(READ_ARGS)
        self.assertTrue(all(r["kind"] == self.KIND for r in responses), responses)
        self.change("DELETE FROM settings WHERE key='context_profile';")
        self.compare([{}])
        self.change("DELETE FROM settings WHERE key='context_workspace';")
        self.compare([{}])

    def test_list_redacts_and_scopes_to_active_profile_wallets(self):
        self.configure()
        responses = self.compare(READ_ARGS)
        data = responses[0]["data"]
        names = [row["name"] for row in data["backends"]]
        self.assertIn("node", names)
        self.assertNotIn("unused", names)
        wire = json.dumps(responses)
        for secret in ("synthetic-password", "synthetic-rune", "synthetic-api-key", "abcd", "lnd.example",
                       "certificate", "\"url\""):
            self.assertNotIn(secret, wire)
        for name in ("electrum1", "core", "unused"):
            python_session(self, [dict(kind="ui.backends.set_default", args={"name": name})])
            self.compare([{}])
        self.change("UPDATE settings SET value='foreign' WHERE key='context_profile';")
        self.compare([{}])
        # No foreign wallet uses `core`, so the summary must not name it.
        python_session(self, [dict(kind="ui.backends.set_default", args={"name": "core"})])
        responses = self.compare([{}])
        self.assertIsNone(responses[0]["data"]["summary"]["default_backend"])
        self.change("UPDATE workspaces SET label='' WHERE id='w';"
                    "UPDATE profiles SET label='' WHERE id='foreign';")
        self.compare([{}])

    def test_expected_scope_and_sqlcipher_list(self):
        self.configure()
        scope = {"workspace_id": "w", "profile_id": "p"}
        self.compare([{"expected_scope": scope}, {"expected_scope": {**scope, "profile_id": "x"}}])
        self.encrypt("synthetic backends key")
        self.configure()
        self.compare([{}, "x"])


if __name__ == "__main__":
    unittest.main()
