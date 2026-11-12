"""Desktop settings and status reads make no network traffic (privacy regression).

Product egress is left enabled, every saved backend points at an owned
loopback listener or an unresolvable `.invalid` host, and the book's
freshness policy enables report-read sync. The test socket guard is armed in
strict mode, so the Rust gate refuses even loopback and Python's guard refuses
DNS. Both daemons must still answer byte for byte, and the listener must never
see a connection. A saved backend or policy is configuration, not consent.
"""
import socket
import threading
import unittest

from tests.rust_desktop_parity import RUST_BIN, DesktopParity
from tests.test_rust_desktop_lightning_test import insert_backend

READS = [
    ("ui.freshness.status", {}),
    ("ui.maintenance.settings", {}),
    ("ui.backends.list", {}),
    ("ui.wallets.utxos", {"wallet": "e-esplora"}),
    ("ui.wallets.utxos", {"wallet": "e-electrum"}),
    ("ui.wallets.utxos", {"wallet": "e-core"}),
    ("ui.wallets.utxos", {"wallet": "e-sp"}),
    ("ui.wallets.utxos", {"wallet": "e-default"}),
]
WRITES = [
    ("ui.freshness.configure", {"report_read_sync": True, "market_rate_provider": "mempool",
                                "source_classes": {"onchain_wallet": True, "market_rates": True,
                                                   "btcpay_wallet": True, "btcpay_provenance": True}}),
    ("ui.maintenance.configure", {"auto_sync_before_report_reads": True, "require_coarse_review": True}),
]


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


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustDesktopReadsNoEgressTest(DesktopParity, unittest.TestCase):
    KIND = "ui.freshness.status"

    def setUp(self):
        super().setUp()
        self.env.pop("KASSIBER_NO_EGRESS", None)
        self.env["KASSIBER_TEST_NO_EGRESS"] = "strict"
        self.listener = Listener()
        self.addCleanup(self.listener.close)
        local = f"127.0.0.1:{self.listener.port}"
        self.change(
            insert_backend("loopesplora", "esplora", f"http://{local}/api")
            + insert_backend("loopelectrum", "electrum", f"tcp://{local}")
            + insert_backend("loopnode", "lnd", f"https://{local}", "aabb")
            + insert_backend("dnscore", "bitcoinrpc", "http://egress-probe.invalid:8332",
                             username="u", password="p", silent_payments="yes")
            + insert_backend("dnsesplora", "esplora", "https://egress-probe.invalid/api")
            + """
INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES
 ('e-esplora','w','p','E esplora','descriptor','{"backend":"loopesplora","descriptor":"wpkh(x)"}','2025'),
 ('e-electrum','w','p','E electrum','address','{"backend":"loopelectrum","addresses":["bcrt1q"]}','2025'),
 ('e-core','w','p','E core','descriptor','{"backend":"dnscore","descriptor":"wpkh(x)"}','2025'),
 ('e-sp','w','p','E sp','silent-payment','{"backend":"dnscore","sp_descriptor":"sp(x)"}','2025'),
 ('e-node','w','p','E node','lnd','{"backend":"loopnode"}','2025'),
 ('e-default','w','p','E default','xpub','{"xpub":"x","script_types":["p2wpkh"]}','2025');
INSERT INTO settings(key,value) VALUES('default_backend','dnsesplora');
INSERT INTO freshness_source_states(profile_id,source_key,source_type,source_label,status,state,blocking_reports,
  updated_at) VALUES('p','onchain_wallet:e-esplora','onchain_wallet','E esplora','stale','stale',1,'2025');
""")

    def session(self, kind, args, **options):
        return self.compare([args], kind=kind, **options)

    def test_status_settings_list_and_inventory_reads_open_no_connection(self):
        for kind, args in READS:
            responses = self.session(kind, args)
            self.assertEqual(responses[0]["kind"], kind, responses)
        for kind, args in WRITES:
            responses = self.session(kind, args, mutates=True)
            self.assertEqual(responses[0]["kind"], kind, responses)
        for kind, args in READS:
            self.session(kind, args)
        self.assertEqual(self.listener.accepted, 0)


if __name__ == "__main__":
    unittest.main()
