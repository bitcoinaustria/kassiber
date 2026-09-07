import json
import unittest
import uuid

from kassiber.core.book_network import apply_book_network, guard_observation, guard_wallet, inventory_book_network, plan_book_network, require_book_accounting, require_chain_domain, observation_matches_binding
from kassiber.errors import AppError
from tests import test_transaction_graph as fixtures


class BookNetworkTests(unittest.TestCase):
    def setUp(self):
        self.book = fixtures.TransactionGraphTest()
        self.book.setUp()
        self.addCleanup(self.book.tearDown)
        self.conn = self.book.conn

    def bind(self, environment="main", instance=None):
        args = {"environment": environment, "chain_instance_id": instance, "declared_wallet_ids": ["wallet-a", "wallet-b", "wallet-c"]}
        plan = plan_book_network(self.conn, "profile-1", args)
        self.assertTrue(plan["can_apply"], plan["blockers"])
        return apply_book_network(self.conn, "profile-1", {**args, "plan_id": plan["plan_id"]})

    def test_main_bitcoin_liquid_lightning_share_environment_but_not_chain_identity(self):
        self.bind()
        btc = require_chain_domain(self.conn, "profile-1", "bitcoin", "main")
        ln = require_chain_domain(self.conn, "profile-1", "lightning", "mainnet")
        liquid = require_chain_domain(self.conn, "profile-1", "liquid", "liquidv1")
        self.assertEqual(btc["domain_id"], ln["domain_id"])
        self.assertNotEqual(btc["domain_id"], liquid["domain_id"])
        with self.assertRaises(AppError):
            require_chain_domain(self.conn, "profile-1", "bitcoin", "regtest")

    def test_plan_is_bound_to_all_history_not_only_input_version(self):
        args = {"environment": "main", "declared_wallet_ids": ["wallet-a", "wallet-b", "wallet-c"]}
        plan = plan_book_network(self.conn, "profile-1", args)
        self.book._tx("new", "wallet-a", "inbound", 1000, "a" * 64, {"network": "main"})
        with self.assertRaisesRegex(AppError, "stale"):
            apply_book_network(self.conn, "profile-1", {**args, "plan_id": plan["plan_id"]})

    def test_unknown_scope_is_explicit_declaration_not_rewritten_evidence(self):
        self.conn.execute("UPDATE wallets SET config_json='{}'")
        self.book._tx("unknown", "wallet-a", "inbound", 1000, "a" * 64, {})
        before = self.conn.execute("SELECT raw_json FROM transactions").fetchone()[0]
        plan = plan_book_network(self.conn, "profile-1", {"environment": "main"})
        self.assertFalse(plan["can_apply"])
        self.bind()
        self.assertEqual(self.conn.execute("SELECT raw_json FROM transactions").fetchone()[0], before)

    def test_mixed_history_blocks_binding_and_accounting(self):
        self.conn.execute("UPDATE wallets SET config_json='{}'")
        self.book._tx("main", "wallet-a", "inbound", 1000, "a" * 64, {"network": "main"})
        self.book._tx("test", "wallet-b", "inbound", 1000, "b" * 64, {"network": "regtest"})
        self.assertEqual(inventory_book_network(self.conn, "profile-1")["state"], "mixed")
        with self.assertRaises(AppError):
            require_book_accounting(self.conn, "profile-1")

    def test_regtest_instances_are_not_genesis_aliases(self):
        self.conn.execute("UPDATE wallets SET config_json='{}'")
        instance = str(uuid.uuid4())
        self.bind("regtest", instance)
        domain = require_chain_domain(self.conn, "profile-1", "bitcoin", "regtest")
        self.assertEqual(domain["chain_instance_id"], instance)
        with self.assertRaises(AppError):
            require_chain_domain(self.conn, "profile-1", "bitcoin", "regtest", chain_instance_id=str(uuid.uuid4()))
        with self.assertRaises(AppError):
            guard_observation(self.conn, "profile-1", {"raw_json": {"network": "regtest", "chain_instance_id": str(uuid.uuid4())}})

    def test_wallet_history_network_cannot_be_relabelled(self):
        with self.assertRaises(AppError):
            guard_wallet(self.conn, "profile-1", {"network": "regtest"}, previous_config={"network": "main"})
        guard_wallet(self.conn, "profile-1", {"network": "mainnet"}, previous_config={"network": "main"})

    def test_contradictory_raw_and_config_never_admitted(self):
        with self.assertRaises(AppError):
            guard_observation(self.conn, "profile-1", {"raw_json": {"network": "regtest"}, "wallet_config_json": {"network": "main"}})

    def test_binding_immutable_and_no_signet_liquid_guess(self):
        self.conn.execute("UPDATE wallets SET config_json='{}'")
        self.bind("signet")
        with self.assertRaises(AppError):
            require_chain_domain(self.conn, "profile-1", "liquid", "liquidtestnet")
        with self.assertRaises(Exception):
            self.conn.execute("UPDATE book_network_bindings SET environment='main'")


    def test_shared_regtest_cache_requires_the_exact_declared_instance(self):
        self.conn.execute("UPDATE wallets SET config_json='{}'")
        instance = str(uuid.uuid4())
        binding = self.bind("regtest", instance)
        row = {"config_json": {"chain": "bitcoin", "network": "regtest"}}
        self.assertTrue(observation_matches_binding(binding, row))
        self.assertFalse(observation_matches_binding(binding, row, unscoped=True))
        row["raw_json"] = {"chain_instance_id": instance}
        self.assertTrue(observation_matches_binding(binding, row, unscoped=True))
        row["raw_json"] = {"chain_instance_id": str(uuid.uuid4())}
        self.assertFalse(observation_matches_binding(binding, row))

    def test_acquired_reference_network_is_part_of_binding_inventory(self):
        self.book._tx("main", "wallet-a", "inbound", 1000, "b" * 64, {"network": "main"})
        self.conn.execute("INSERT INTO chain_analysis_observations VALUES(?,?,?,?,?,?,?,?)", ("profile-1", "bitcoin", "regtest", "a" * 64, "{}", "{}", "local", "2026-01-01"))
        plan = plan_book_network(self.conn, "profile-1", {"environment": "main", "declared_wallet_ids": ["wallet-a", "wallet-b", "wallet-c"]})
        self.assertFalse(plan["can_apply"])
        self.assertIn("reference_domain_mismatch", [item["code"] for item in plan["blockers"]])
        self.assertEqual(plan["inventory"]["state"], "mixed")

    def test_onchain_txid_without_network_does_not_guess_main(self):
        self.conn.execute("UPDATE wallets SET config_json='{}'")
        self.bind("regtest", str(uuid.uuid4()))
        guard_observation(self.conn, "profile-1", {"asset":"BTC", "external_id":"a" * 64, "raw_json":{}})

    def test_bullbitcoin_rail_column_is_not_network_evidence(self):
        self.bind("main")
        for rail in ("bitcoin", "liquid", "lightning"):
            guard_observation(self.conn, "profile-1", {"raw_json":{"source":"bullbitcoin_wallet_csv", "network":rail}})

    def test_canonical_source_columns_are_scope_evidence_without_overriding_raw(self):
        self.conn.execute("UPDATE wallets SET config_json='{}'")
        instance = str(uuid.uuid4())
        binding = self.bind("regtest", instance)
        row = {"chain":"bitcoin", "network":"regtest", "raw_json":{"chain_instance_id":instance}}
        self.assertTrue(observation_matches_binding(binding, row, unscoped=True))
        row["raw_json"]["network"] = "main"
        self.assertFalse(observation_matches_binding(binding, row, unscoped=True))
