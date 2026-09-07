"""Exercise admission/publication at the real service boundaries."""
import json
import unittest
import uuid
from types import SimpleNamespace
from unittest.mock import patch

from kassiber.core.book_network import apply_book_network, plan_book_network, wallet_for_network_sync
from kassiber.core import transaction_graph as graph
from kassiber.core import sync
from kassiber.core.imports import insert_wallet_records
from kassiber.errors import AppError
from tests import test_transaction_graph as fixtures


class NetworkBoundaryTests(unittest.TestCase):
    def setUp(self):
        self.book = fixtures.TransactionGraphTest()
        self.book.setUp()
        self.addCleanup(self.book.tearDown)
        self.conn = self.book.conn
        self.conn.execute("UPDATE wallets SET config_json='{}'")
        self.profile = dict(self.conn.execute("SELECT * FROM profiles WHERE id='profile-1'").fetchone())
        self.wallet = dict(self.conn.execute("SELECT * FROM wallets WHERE id='wallet-a'").fetchone())

    def bind(self, environment="main"):
        args = {"environment":environment, "chain_instance_id":str(uuid.uuid4()) if environment == "regtest" else None, "declared_wallet_ids":["wallet-a","wallet-b","wallet-c"]}
        plan = plan_book_network(self.conn, "profile-1", args)
        return apply_book_network(self.conn, "profile-1", {**args,"plan_id":plan["plan_id"]})

    def test_batch_preflight_rejects_second_wrong_network_before_any_mutation(self):
        self.bind()
        records = [{"txid":str(index) * 64,"occurred_at":fixtures.NOW,"direction":"inbound","asset":"BTC","amount":"0.01","fee":"0","raw_json":json.dumps({"network":network})} for index,network in ((1,"main"),(2,"regtest"))]
        before = self.conn.total_changes
        with self.assertRaises(AppError):
            insert_wallet_records(self.conn,self.profile,self.wallet,records,"test",SimpleNamespace())
        self.assertEqual(self.conn.total_changes,before)
        self.assertEqual(self.conn.execute("SELECT COUNT(*) FROM transactions").fetchone()[0],0)

    def test_regtest_cache_is_not_reused_in_another_declared_instance(self):
        binding = self.bind("regtest")
        txid = "a" * 64
        payload = {"txid":txid,"vin":[{"coinbase":"0101"}],"vout":[{"value":1000,"scriptpubkey":"0014"+"11"*20}]}
        graph._store_graph_lookup_cache(self.conn,"bitcoin","regtest",txid,payload)
        self.assertIsNotNone(graph._load_graph_lookup_cache(self.conn,"bitcoin","regtest",txid))
        self.conn.execute("UPDATE transaction_graph_cache SET payload_json=?",(json.dumps({**payload,"chain_instance_id":str(uuid.uuid4())}),))
        self.assertIsNone(graph._load_graph_lookup_cache(self.conn,"bitcoin","regtest",txid))
        self.assertNotIn("chain_instance_id",payload)
        self.assertTrue(binding["chain_instance_id"])

    def test_prefetch_from_before_binding_cannot_publish_after_binding(self):
        self.bind()
        fetched = sync.WalletBackendFetch(backend={},sync_state=None,normalized_records=[],adapter_meta={},kind="esplora",started=0,force_full=False)
        with patch.object(sync,"apply_fetch_observer_updates",side_effect=AssertionError("must not publish")):
            with self.assertRaises(AppError) as caught:
                sync.sync_wallet_from_backend(self.conn,{},self.profile,self.wallet,SimpleNamespace(),prefetched=fetched)
        self.assertEqual(caught.exception.code,"stale_context")

    def test_reviewed_routing_default_does_not_relabel_historical_wallet_config(self):
        binding = self.bind("regtest")
        wallet = {**self.wallet,"kind":"coreln"}
        effective = wallet_for_network_sync(self.conn,"profile-1",wallet)
        self.assertEqual(json.loads(effective["config_json"])["network"],"regtest")
        self.assertEqual(json.loads(effective["config_json"])["chain_instance_id"],binding["chain_instance_id"])
        self.assertEqual(self.conn.execute("SELECT config_json FROM wallets WHERE id='wallet-a'").fetchone()[0],"{}")

    def test_wallet_update_normalization_keeps_the_bound_regtest_default(self):
        from kassiber.core.wallets import update_wallet
        self.conn.execute("UPDATE wallets SET kind='custom' WHERE id='wallet-a'")
        binding = self.bind("regtest")
        update_wallet(self.conn,"ws-1","profile-1","wallet-a",{"config":{"chain":"bitcoin"}})
        config = json.loads(self.conn.execute("SELECT config_json FROM wallets WHERE id='wallet-a'").fetchone()[0])
        self.assertEqual(config["network"],"regtest")
        self.assertEqual(config["chain_instance_id"],binding["chain_instance_id"])
        from kassiber.core.book_network import require_book_accounting
        require_book_accounting(self.conn,"profile-1")
