"""Report blockers, maintenance, review context, evidence and graph reads make
no network traffic (privacy regression).

Product egress stays enabled, every saved backend points at an owned loopback
listener or an unresolvable `.invalid` host, and the test socket guard is armed
in strict mode, so the Rust gate refuses even loopback and Python's guard
refuses DNS. A saved backend is configuration, not consent: with report-read
refresh disabled both daemons answer byte for byte and the listener never sees
a connection. Enabling report-read refresh is the consent for its refresh; only
then does the Rust refresh reach the gate, whose strict guard aborts the
attempt (the harness build's `TestEgressBlocked`) before any socket opens. The graph's
public lookup is refused by Rust before any backend is chosen.
"""
import json
import socket
import threading
import unittest

from tests.rust_desktop_parity import RUST_BIN, DesktopParity, run_candidate
from tests.test_rust_report_capital_gains import generated, journal_shape
from tests.test_rust_transaction_graph import GRAPH


def backend(name, kind, url):
    return ("INSERT INTO backends(name,kind,url,token,timeout,config_json,created_at,updated_at) "
            f"VALUES('{name}','{kind}','{url}','',2,'{{}}','fixed','fixed');")


class Listener:
    """Counts every TCP connection to an owned loopback port."""

    def __init__(self):
        self.server = socket.socket()
        self.server.bind(("127.0.0.1", 0))
        self.server.listen()
        self.server.settimeout(0.05)
        self.port = self.server.getsockname()[1]
        self.accepted = 0
        self.stopping = False
        self.thread = threading.Thread(target=self.run)
        self.thread.start()

    def run(self):
        while not self.stopping:
            try:
                connection, _ = self.server.accept()
            except OSError:
                continue
            self.accepted += 1
            connection.close()

    def close(self):
        self.stopping = True
        self.thread.join()
        self.server.close()


READS = [
    ("ui.report.blockers", {}),
    ("ui.maintenance.run", {"sync": "never"}),
    ("ui.maintenance.run", {"sync": "if_enabled"}),
    ("ui.audit.evidence.summary", {}),
    ("ui.transactions.review_context", {"transaction": "g2"}),
    ("ui.transactions.graph", {"transaction": "g2"}),
    ("ui.transactions.graph", {"transaction": "g1", "allowPublicLookup": False}),
]


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustDesktopProjectionsNoEgressTest(DesktopParity, unittest.TestCase):
    KIND = "ui.report.blockers"

    def setUp(self):
        super().setUp()
        self.env.pop("KASSIBER_NO_EGRESS", None)
        self.env["KASSIBER_TEST_NO_EGRESS"] = "strict"
        self.listener = Listener()
        self.addCleanup(self.listener.close)
        local = f"127.0.0.1:{self.listener.port}"
        self.change(GRAPH
                    + backend("loopesplora", "esplora", f"http://{local}/api")
                    + backend("loopelectrum", "electrum", f"tcp://{local}")
                    + backend("dnsesplora", "esplora", "https://egress-probe.invalid/api")
                    + "INSERT INTO settings(key,value) VALUES('default_backend','loopesplora');"
                    + "UPDATE wallets SET config_json='{\"backend\":\"loopelectrum\",\"addresses\":[\"bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu\"],\"chain\":\"bitcoin\",\"network\":\"main\"}' WHERE id='watch';")

    def test_reads_open_no_connection_without_consent(self):
        for kind, args in READS:
            responses = self.compare([args], kind=kind, mutates=True, mask=generated, tables=journal_shape)
            self.assertIn(responses[0]["kind"], (kind, "error"), responses)
        self.assertEqual(self.listener.accepted, 0)

    def test_public_lookup_cannot_reach_a_backend_under_the_product_switch(self):
        """The explicit lookup asks the chosen backends through the egress
        gate (the strict test guard would trip on it); the product no-egress
        switch refuses each one before any connection, as a warning."""
        self.env["KASSIBER_TEST_NO_EGRESS"] = "1"
        self.env["KASSIBER_NO_EGRESS"] = "1"
        requests = self.requests([{"transaction": "g2", "allowPublicLookup": True}], kind="ui.transactions.graph")
        wire = "".join(json.dumps(r) + "\n" for r in [*requests, dict(kind="daemon.shutdown")])
        result = run_candidate(["--machine", "--data-root", str(self.root), "daemon"],
                               env=self.env, input=wire.encode())
        response = json.loads(result.stdout.splitlines()[1])
        codes = [warning["code"] for warning in response["data"]["warnings"]]
        self.assertTrue(any(code.startswith("bitcoin_reference_lookup_failed") for code in codes), codes)
        self.assertEqual(self.listener.accepted, 0)


XPUB = ("xpub6CatWdiZiodmUeTDp8LT5or8nmbKNcuyvz7WyksVFkKB4RHwCD3XyuvPEbvqAQY3rAPshWcMLoP2fMFMKHPJ4ZeZ"
        "XYVUhLv1VMrjPC7PW6V")
REFRESH_BOOK = """
INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025');
INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at) VALUES('p','w','Book','EUR','at','2025');
INSERT INTO settings(key,value) VALUES('context_workspace','w'),('context_profile','p');
INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES
 ('desc','w','p','Vault','descriptor','{"backend":"loopesplora","descriptor":"wpkh([73c5da0a/84h/0h/0h]%(key)s/0/*)","change_descriptor":"wpkh([73c5da0a/84h/0h/0h]%(key)s/1/*)","chain":"bitcoin","network":"main","gap_limit":2}','2025');
""" % {"key": XPUB}


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustReportReadRefreshEgressTest(DesktopParity, unittest.TestCase):
    KIND = "ui.report.blockers"
    SEED = None

    def seed(self):
        self.change(REFRESH_BOOK)

    def setUp(self):
        super().setUp()
        self.env.pop("KASSIBER_NO_EGRESS", None)
        self.env["KASSIBER_TEST_NO_EGRESS"] = "strict"
        self.listener = Listener()
        self.addCleanup(self.listener.close)
        # No timeout or token: the plain Esplora transport the native refresh runs.
        self.change("INSERT INTO backends(name,kind,url,created_at,updated_at) "
                    f"VALUES('loopesplora','esplora','http://127.0.0.1:{self.listener.port}/api','fixed','fixed');")

    def rust(self, requests):
        wire = "".join(json.dumps(r) + "\n" for r in [*requests, dict(kind="daemon.shutdown")])
        result = run_candidate(["--machine", "--data-root", str(self.root), "daemon"],
                               env=self.env, input=wire.encode())
        return [json.loads(line) for line in result.stdout.splitlines()[1:-1]]

    def test_report_read_refresh_waits_for_its_policy(self):
        # A saved backend and wallet are configuration: nothing is enqueued.
        responses = self.rust(self.requests([{}, {}]) + self.requests([{"sync": "if_enabled"}], kind="ui.maintenance.run"))
        self.assertTrue(all(r["kind"] in ("ui.report.blockers", "ui.maintenance.run") for r in responses), responses)
        self.assertNotIn("sync", responses[2]["data"]["maintenance"])
        self.assertEqual(self.listener.accepted, 0)
        # Enabling the policy is the consent. The refresh now reaches the gate,
        # whose strict test guard aborts the attempt before any socket opens.
        self.change("INSERT INTO settings(key,value) VALUES('freshness.policy.profile.p',"
                    "'{\"report_read_sync\": true, \"source_classes\": {\"onchain_wallet\": true}}');")
        responses = self.rust(self.requests([{}]))
        self.assertEqual((responses[0]["kind"], responses[0]["error"]["code"]), ("error", "internal_error"),
                         responses)
        self.assertEqual(self.listener.accepted, 0)
