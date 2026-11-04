"""`ui.backends.lightning.test` served by the Rust daemon, compared with Python.

Each session runs against the same owned loopback fake node (or fake CLN
command) for both daemons; the response bytes, the node requests and all 112
tables must match. Refusals and the product no-egress switch make no node
request at all.
"""
import json
import unittest

from tests.rust_desktop_parity import RUST_BIN, DesktopParity
from tests.test_rust_cln_acquisition import fake_cli, node as cln_node
from tests.test_rust_lightning_acquisition import node as lnd_node
from tests.test_rust_lightning_service import cases

KIND = "ui.backends.lightning.test"


def insert_backend(name, kind, url="", token="", **config):
    def quote(value):
        return "'" + str(value).replace("'", "''") + "'"
    values = ",".join(quote(v) for v in (name, kind, url, token))
    return ("INSERT INTO backends(name,kind,url,token,timeout,config_json,created_at,updated_at) "
            f"VALUES({values},2,{quote(json.dumps(config))},'fixed','fixed');")


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustLightningBackendTest(DesktopParity, unittest.TestCase):
    KIND = KIND

    def online(self):
        # Loopback stays allowed by the test observer; the product switch
        # would refuse every node request.
        self.env.pop("KASSIBER_NO_EGRESS", None)

    @staticmethod
    def script(case, requests_per_side):
        # The fake node answers by request index across both daemons' sessions.
        return case["calls"] * (2 * requests_per_side)

    def same_hits(self, hits, per_side):
        self.assertEqual(len(hits), 2 * per_side, hits)
        self.assertEqual(hits[:per_side], hits[per_side:])

    def test_refusals_and_non_lightning_backends_make_no_node_traffic(self):
        self.online()
        case = cases("lnd")[0]
        with lnd_node(case["calls"]) as (url, hits):
            self.change(insert_backend("chain", "esplora", "https://chain.example")
                        + insert_backend("nwcnode", "nwc", "nostr+walletconnect://x")
                        + insert_backend("node", "lnd", url, "aabb"))
            responses = self.compare([..., None, [], "x", {}, {"backend": ""}, {"backend": "  "}, {"backend": 3},
                                      {"backend": "missing"}, {"backend": "chain"}, {"backend": " NWCNODE "}])
            self.assertEqual(hits, [])
        self.assertEqual(responses[9]["data"]["logs"][-1],
                         "Backend kind 'esplora' is not a Lightning node backend.")
        self.assertEqual(responses[10]["data"]["error"]["code"], "lightning_adapter_unavailable")

    def test_lnd_snapshots_match_bytes_and_node_requests(self):
        self.online()
        for case in cases("lnd"):
            with self.subTest(case=case.get("name")), lnd_node(self.script(case, 2)) as (url, hits):
                self.change("DELETE FROM backends;" + insert_backend("node", "lnd", url, "aabb"))
                responses = self.compare([{"backend": "node"}, {"backend": " Node "}])
                self.assertTrue(all(r["data"]["ok"] for r in responses), responses)
                self.same_hits(hits, len(hits) // 2)

    def test_lnd_node_fields_follow_the_reference_coercions(self):
        self.online()
        base = cases("lnd")[0]
        infos = [
            {"alias": "Synthetic Node", "num_peers": "7", "block_height": 812345,
             "chains": [{"chain": "bitcoin", "network": " REGTEST "}]},
            {"alias": 5, "num_peers": "x", "block_height": 0, "testnet": True},
            {"alias": "", "num_peers": 2.9, "block_height": "17", "chains": [{"network": ""}, {"network": "sig"}]},
            {"testnet": "Signet", "chains": "bitcoin"},
            {"testnet": False},
        ]
        for info in infos:
            calls = [{"response": {**base["calls"][0]["response"], **info}}, *base["calls"][1:]]
            case = {**base, "calls": calls}
            with self.subTest(info=info), lnd_node(self.script(case, 1)) as (url, hits):
                self.change("DELETE FROM backends;" + insert_backend("node", "lnd", url, "aabb"))
                responses = self.compare([{"backend": "node"}])
                self.assertTrue(responses[0]["data"]["ok"], responses)
                self.same_hits(hits, len(hits) // 2)
        self.assertEqual(responses[0]["data"]["network"], "mainnet")

    def test_cln_snapshot_matches_bytes_and_commands(self):
        self.online()
        cli = fake_cli(self.base)
        self.env["KASSIBER_RUST_TEST_CLN_FIXTURE"] = str(cli)
        case = cases("coreln")[0]
        with cln_node(case["data"]) as (url, hits):
            self.change(insert_backend("cln", "coreln", "cln://local", rpc_file=url, lightning_cli=str(cli)))
            responses = self.compare([{"backend": "cln"}])
            self.assertTrue(responses[0]["data"]["ok"], responses)
            self.same_hits(hits, len(hits) // 2)

    def test_failures_report_ok_false_with_the_reference_message(self):
        self.online()
        case = cases("lnd")[0]
        with lnd_node(case["calls"]) as (url, hits):
            self.change(insert_backend("nomacaroon", "lnd", url)
                        + insert_backend("badmacaroon", "lnd", url, "not-hex")
                        + insert_backend("nourl", "lnd", "", "aabb"))
            responses = self.compare([{"backend": "nomacaroon"}, {"backend": "badmacaroon"}, {"backend": "nourl"}])
            self.assertEqual(hits, [])
        self.assertTrue(all(not r["data"]["ok"] for r in responses), responses)

    def test_product_no_egress_switch_refuses_before_any_request(self):
        case = cases("lnd")[0]
        with lnd_node(case["calls"]) as (url, hits):
            self.change(insert_backend("node", "lnd", url, "aabb"))
            responses = self.compare([{"backend": "node"}])
            self.assertEqual(hits, [])
        self.assertFalse(responses[0]["data"]["ok"])

    def test_sqlcipher_book(self):
        self.online()
        case = cases("lnd")[0]
        self.encrypt("synthetic lightning key")
        with lnd_node(self.script(case, 1)) as (url, hits):
            self.change(insert_backend("node", "lnd", url, "aabb"))
            self.compare([{"backend": "node"}])
            self.same_hits(hits, len(hits) // 2)


if __name__ == "__main__":
    unittest.main()
