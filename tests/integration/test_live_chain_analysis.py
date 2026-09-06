"""Disposable Core consensus/index oracle, never a synthetic accounting book.

Every outpoint and signature below is created by independent Core wallets. Only
this fixture signs/broadcasts; the production analysis calls remain read-only.
"""
from __future__ import annotations

from decimal import Decimal
import json
import os
import tempfile
import time
import unittest
import uuid

from embit.psbt import PSBT
from unittest.mock import patch

from kassiber.core.chain_analysis import build_index, run_analysis, run_entropy
from kassiber.core.chain_analysis.psbt import analyze_psbt, compare_psbts
from kassiber.core.chain_analysis_acquisition import apply_acquisition, plan_acquisition
from kassiber.core.chain_analysis_cases import compare_case, get_case, save_case
from kassiber.core.privacy_mirror import build_privacy_mirror
from kassiber.db import open_db
from tests.integration.env import skip_unless_integration
from tests.integration.test_live_bitcoin_core_regtest import _rpc


def _msat(btc):
    value = Decimal(str(btc)) * 100_000_000_000
    if value != value.to_integral_value():
        raise AssertionError("Core returned a non-integral millisatoshi amount")
    return str(int(value))


def _feature(snapshot, code):
    return next(row for row in snapshot["features"] if row["code"] == code)


def _without_derivations(encoded):
    """Receiver response removes wallet metadata, as required by BIP78."""
    proposal = PSBT.from_base64(encoded)
    proposal.xpubs.clear()
    for scope in (*proposal.inputs, *proposal.outputs):
        scope.bip32_derivations.clear()
        scope.taproot_bip32_derivations.clear()
    return proposal.to_base64()


@skip_unless_integration
@unittest.skipUnless(os.environ.get("KASSIBER_DISPOSABLE_CHAIN_ANALYSIS") == "1",
                     "requires the dedicated disposable chain-analysis lane")
class LiveChainAnalysisTest(unittest.TestCase):
    def setUp(self):
        self.url = os.environ["KASSIBER_REGTEST_CORE_URL"]
        self.username = os.environ["KASSIBER_REGTEST_RPC_USER"]
        self.password = os.environ["KASSIBER_REGTEST_RPC_PASSWORD"]
        self.names = []
        self.addCleanup(self._unload_wallets)
        self.assertEqual(self.rpc("getblockchaininfo")["chain"], "regtest")
        self.assertIn("txospenderindex", self.rpc("getindexinfo"))
        self.miner = self.wallet()
        self.mining_address = self.address(self.miner)
        self.mine(101)
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.conn = open_db(self.directory.name)
        self.addCleanup(self.conn.close)
        now = "2026-09-06T12:00:00Z"
        self.conn.execute("INSERT INTO workspaces VALUES('ws','Live investigation',?)", (now,))
        self.conn.execute("INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','ws','Regtest',?)", (now,))
        self.conn.execute("INSERT INTO backends(name,kind,chain,network,url,config_json,created_at,updated_at) VALUES('node','bitcoinrpc','bitcoin','regtest',?,?,?,?)", (self.url, json.dumps({"username": self.username, "password": self.password}), now, now))
        self.conn.commit()

    def rpc(self, method, params=None, wallet=None):
        return _rpc(self.url, self.username, self.password, method, params, wallet)

    def wallet(self):
        name = f"chain-analysis-{uuid.uuid4().hex[:12]}"
        self.rpc("createwallet", [name])
        self.names.append(name)
        return name

    def address(self, wallet, kind="bech32"):
        return self.rpc("getnewaddress", ["", kind], wallet)

    def mine(self, count=1):
        self.rpc("generatetoaddress", [count, self.mining_address])
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            rows, tip = self.rpc("getindexinfo"), self.rpc("getblockcount")
            if all(rows.get(name, {}).get("synced") and rows[name].get("best_block_height", -1) >= tip for name in ("txindex", "txospenderindex")):
                return
            time.sleep(0.1)
        self.fail("Core transaction/spender indexes did not reach the mined tip")

    def fund(self, wallet, amount=1, kind="bech32"):
        address = self.address(wallet, kind)
        txid = self.rpc("sendtoaddress", [address, amount], self.miner)
        self.mine()
        rows = self.rpc("listunspent", [1, 9999999, [address]], wallet)
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["txid"], txid)
        return {"txid": txid, "vout": rows[0]["vout"]}

    def create_psbt(self, inputs, outputs, *, sequence=0xFFFFFFFD, locktime=0):
        return self.rpc("createpsbt", [[{**row, "sequence": sequence} for row in inputs], outputs, locktime])

    def process(self, encoded, *wallets, sign=True):
        for wallet in wallets:
            encoded = self.rpc("walletprocesspsbt", [encoded, sign, "ALL", True], wallet)["psbt"]
        return encoded

    def broadcast(self, encoded):
        final = self.rpc("finalizepsbt", [encoded])
        self.assertTrue(final["complete"])
        accepted = self.rpc("testmempoolaccept", [[final["hex"]]])[0]
        self.assertTrue(accepted["allowed"], accepted)
        txid = self.rpc("sendrawtransaction", [final["hex"]])
        self.mine()
        self.assertGreater(self.rpc("getrawtransaction", [txid, 1])["confirmations"], 0)
        return txid, self.rpc("decoderawtransaction", [final["hex"]])

    def acquire(self, subject, *, direction="backward", depth=1, maximum=1):
        args = {"backend": "node", "subject": subject, "chain": "bitcoin", "network": "regtest", "direction": direction, "depth": depth, "max_transactions": maximum}
        plan = plan_acquisition(self.conn, "p", args)
        result = apply_acquisition(self.conn, "p", {"plan": plan})
        self.assertLessEqual(result["request_count"], plan["effects"]["max_requests"])
        self.assertLessEqual(result["acquired_count"], maximum)
        return result

    def query(self, subject, **args):
        return run_analysis(self.conn, "p", {"mode": "trace", "subject": subject, "chain": "bitcoin", "network": "regtest", "direction": "forward", "depth": 12, **args})

    def assert_reference_only(self):
        for table in ("transactions", "wallets", "chain_observation_provenance", "custody_components"):
            self.assertEqual(self.conn.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0], 0, table)
        index = build_index(self.conn, "p")
        self.assertTrue(all(not node.get("wallet_ids") for node in index.nodes.values()))
        # Signature categories survive in features, but no raw stack material
        # or wallet derivations can survive reference-observation persistence.
        def inspect(value):
            if isinstance(value, dict):
                self.assertFalse({"witness", "txinwitness", "scriptSig", "raw_hex", "preimage", "bip32_derivs", "xpubs"} & value.keys())
                for child in value.values():
                    inspect(child)
            elif isinstance(value, list):
                for child in value:
                    inspect(child)
        for row in self.conn.execute("SELECT payload_json FROM chain_analysis_observations"):
            inspect(json.loads(row[0]))
            self.assertNotIn(self.password, row[0])
        # Mirror must summarize acquired observations without contacting even
        # the configured loopback node. An investigation alone cannot invent
        # wallet ownership, and public workbench handoffs use the same snapshot.
        with patch("socket.getaddrinfo", side_effect=AssertionError("implicit DNS")), patch("socket.socket.connect", side_effect=AssertionError("implicit network")):
            mirror = build_privacy_mirror(self.conn, "p", redacted=False)
            self.assertEqual(mirror["summary"]["status"], "unavailable")
            self.assertEqual(mirror["summary"]["owned_output_count"], 0)
            for finding in mirror["findings"]:
                target = run_analysis(self.conn, "p", finding["investigation"]["query"])
                self.assertEqual(target["snapshot_id"], mirror["investigation"]["snapshot_id"])

    def test_mirror_personal_relevance_uses_core_owned_output_and_shared_evidence(self):
        alice, bob = self.wallet(), self.wallet()
        owned, counterparty = self.fund(alice), self.fund(bob)
        signed = self.process(self.create_psbt([owned, counterparty], [{self.address(alice): 1.4}, {self.address(bob): 0.5999}]), alice, bob)
        txid, _ = self.broadcast(signed)
        self.acquire(txid)
        funding = self.rpc("getrawtransaction", [owned["txid"], 1])
        output = funding["vout"][owned["vout"]]
        self.assertTrue(self.rpc("getaddressinfo", [output["scriptPubKey"]["address"]], alice)["ismine"])
        now = "2026-09-06T12:00:00Z"
        self.conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('alice','ws','p','Alice','descriptor','{}',?)", (now,))
        self.conn.execute("INSERT INTO wallet_utxos(id,workspace_id,profile_id,wallet_id,chain,network,asset,amount,txid,vout,outpoint,confirmation_status,script_pubkey,spent_by,first_seen_at,last_seen_at) VALUES('owned','ws','p','alice','bitcoin','regtest','BTC',?,?,?,?, 'confirmed',?,?,?,?)", (int(_msat(output["value"])), owned["txid"], owned["vout"], f"{owned['txid']}:{owned['vout']}", output["scriptPubKey"]["hex"], txid, now, now))
        self.conn.commit()
        before = self.conn.total_changes
        with patch("socket.getaddrinfo", side_effect=AssertionError("implicit DNS")), patch("socket.socket.connect", side_effect=AssertionError("implicit network")):
            mirror = build_privacy_mirror(self.conn, "p", redacted=False)
            common_input = next(row for row in mirror["findings"] if row["code"] == "common_input_control")
            self.assertEqual(common_input["relevance"], "own_spend")
            self.assertIn("assumes_no_undetected_collaboration", common_input["assumptions"])
            self.assertEqual(mirror["summary"]["owned_output_count"], 1)
            self.assertEqual(self.conn.total_changes, before)
            # We know this fixture is collaborative because both Core wallets
            # signed it. The public observer only gets a conditional hypothesis.
            self.assertNotEqual(common_input["authority"], "observed")
            reopened = run_analysis(self.conn, "p", common_input["investigation"]["query"])
            self.assertEqual(reopened["snapshot_id"], common_input["investigation"]["snapshot_id"])

    def test_missing_intermediate_is_a_frontier_then_acquisition_recovers_exact_path_and_case(self):
        middle, destination, final_wallet = self.wallet(), self.wallet(), self.wallet()
        funding = self.fund(middle)["txid"]
        spend = self.rpc("sendtoaddress", [self.address(destination), 0.7], middle)
        self.mine()
        onward = self.rpc("sendtoaddress", [self.address(final_wallet), 0.5], destination)
        self.mine()
        # Observe only both endpoints. A valid outpoint on the last transaction
        # cannot conjure the omitted transaction's inputs or an ownership link.
        limited = self.acquire(funding, direction="forward")
        self.assertFalse(limited["complete"])
        self.assertIn("transaction_limit", {row["reason"] for row in limited["frontier"]})
        self.acquire(onward)
        path_args = {"mode": "path", "target": onward, "observer": "public"}
        before = self.query(funding, **path_args)
        self.assertEqual(before["paths"], [])
        self.assertTrue(before["frontier"])
        case = save_case(self.conn, "p", {"title": "Missing intermediate", "query": before["query"], "expected_snapshot_id": before["snapshot_id"]})
        observed = self.acquire(funding, direction="forward", depth=4, maximum=10)
        self.assertTrue(observed["complete"], observed)
        self.assertEqual(set(observed["transaction_ids"]), {funding, spend, onward})
        after = self.query(funding, **path_args)
        self.assertEqual(len(after["paths"]), 1)
        path = after["paths"][0]
        self.assertEqual(path["kind"], "physical")
        self.assertEqual(len(path["edge_ids"]), 4)
        edges = {row["id"]: row for row in after["edges"]}
        self.assertEqual([edges[ident]["kind"] for ident in path["edge_ids"]], ["creates", "spends", "creates", "spends"])
        self.assertEqual(after["clusters"], [])
        comparison = compare_case(self.conn, "p", {"id": case["id"]})
        self.assertTrue(comparison["changed"])
        self.assertIn(spend, {row.get("txid") for row in comparison["added_nodes"]})
        self.assertEqual(get_case(self.conn, "p", case["id"])["result"]["paths"], [])
        # Reopening the book must retain feature evidence after witness discard.
        snapshot = after["snapshot_id"]
        other = open_db(self.directory.name)
        try:
            self.assertEqual(run_analysis(other, "p", after["query"])["snapshot_id"], snapshot)
        finally:
            other.close()
        trace = self.query(funding, observer="public")
        later_case = save_case(self.conn, "p", {"title": "Before another confirmed spend", "query": trace["query"], "expected_snapshot_id": trace["snapshot_id"]})
        tail = self.rpc("sendtoaddress", [self.address(middle), 0.3], final_wallet)
        self.mine()
        self.acquire(funding, direction="forward", depth=5, maximum=10)
        later = compare_case(self.conn, "p", {"id": later_case["id"]})
        self.assertTrue(later["changed"])
        self.assertIn(tail, {row.get("txid") for row in later["added_nodes"]})
        self.assertNotIn(tail, {row.get("txid") for row in get_case(self.conn, "p", later_case["id"])["result"]["nodes"]})
        self.assert_reference_only()

    def test_core_psbt_and_persisted_feature_parity_across_four_input_script_types(self):
        wallets = [self.wallet() for _ in range(4)]
        inputs = [self.fund(wallet, kind=kind) for wallet, kind in zip(wallets, ("legacy", "p2sh-segwit", "bech32", "bech32m"))]
        outputs = [{self.address(self.miner, kind): amount} for kind, amount in (("bech32m", 1), ("bech32", 1), ("legacy", 1.9999))]
        empty = self.create_psbt(inputs, outputs, locktime=self.rpc("getblockcount"))
        unknown = analyze_psbt(empty, network="regtest")
        self.assertIsNone(unknown["totals"]["input_msat"])
        self.assertIsNone(unknown["totals"]["fee_msat"])
        self.assertEqual(unknown["coverage"]["missing_input_indices"], [0, 1, 2, 3])
        signed = self.process(empty, *wallets)
        parsed = analyze_psbt(signed, network="regtest")
        core = self.rpc("decodepsbt", [signed])
        self.assertEqual(parsed["totals"]["fee_msat"], _msat(core["fee"]))
        self.assertEqual(_feature(parsed["features"], "input_script_types")["value"]["counts"], {"p2pkh": 1, "p2sh": 1, "p2wpkh": 1, "p2tr": 1})
        signatures = _feature(parsed["features"], "signature_encodings")["value"]
        self.assertEqual(signatures["count"], 4)
        self.assertEqual(sorted(row["algorithm"] for row in signatures["observations"]), ["ecdsa", "ecdsa", "ecdsa", "schnorr"])
        self.assertTrue(all(row["low_r"] and row["low_s"] for row in signatures["observations"] if row["algorithm"] == "ecdsa"))
        txid, decoded = self.broadcast(signed)
        self.assertEqual(parsed["totals"]["final_vsize"], decoded["vsize"])
        self.assertEqual(Decimal(parsed["totals"]["final_fee_rate_sat_vb"]), Decimal(parsed["totals"]["fee_msat"]) / 1000 / decoded["vsize"])
        self.acquire(txid)
        result = self.query(txid)
        snapshot = next(row["features"] for row in result["transaction_features"] if row["subject"].endswith(txid))
        for code in ("transaction_version", "sequences", "absolute_locktime", "input_script_types", "output_script_types", "signature_encodings", "output_value_pattern", "output_ordering"):
            self.assertEqual(_feature(snapshot, code)["value"], _feature(parsed["features"], code)["value"], code)
        self.assertNotIn(signed, json.dumps(result))
        self.assert_reference_only()

    def test_equal_output_joint_transaction_preserves_ambiguity_and_model_assumptions(self):
        alice, bob = self.wallet(), self.wallet()
        inputs = [self.fund(alice), self.fund(bob)]
        original = self.create_psbt(inputs, [{self.address(alice): 0.9999}, {self.address(bob): 0.9999}])
        alice_signed = self.process(original, alice)
        self.assertFalse(self.rpc("finalizepsbt", [alice_signed])["complete"])
        signed = self.process(alice_signed, bob)
        txid, _ = self.broadcast(signed)
        self.acquire(txid)
        result = self.query(txid, direction="both", observer="public")
        self.assertEqual(result["clusters"], [])
        self.assertFalse(any(row["kind"] == "hypothesis" for row in result["edges"]))
        self.assertIn("equal_output_groups", {row["code"] for row in result["findings"]})
        # Two equal-input/equal-output participants have three financial-flow
        # partitions, including the all-in-one partition. This is conditional
        # ambiguity, never a measurement of the participants' actual identities.
        entropy = run_entropy(self.conn, "p", {"subject": txid, "chain": "bitcoin", "network": "regtest", "observer": "public"})
        self.assertEqual(entropy["status"], "exact", entropy)
        self.assertEqual(entropy["interpretation_count"], "3")
        self.assertFalse(entropy["deterministic_links"])
        self.assertTrue(entropy["assumptions"])
        with_hypotheses = self.query(txid, direction="both", observer="public", include_hypotheses=True)
        for edge in with_hypotheses["edges"]:
            if edge.get("rule") == "common_input_control":
                self.assertIn("assumes_no_undetected_collaboration", edge["premises"])
                self.assertFalse(edge["accounting_authority"])
        self.assert_reference_only()

    def test_real_payjoin_receiver_signature_fee_and_order_checks_then_core_acceptance(self):
        sender, receiver = self.wallet(), self.wallet()
        sender_input, receiver_input = self.fund(sender), self.fund(receiver, 0.25)
        payment, change = self.address(receiver), self.address(sender)
        original = self.process(self.create_psbt([sender_input], [{payment: 0.2}, {change: 0.79999}]), sender)
        # The receiver contributes 0.25 BTC and pays the 1000-sat fee increase.
        unsigned = self.create_psbt([sender_input, receiver_input], [{payment: 0.44999}, {change: 0.79999}])
        proposal = _without_derivations(self.process(self.process(unsigned, sender, sign=False), receiver))
        constraints = {"payment_output_index": 0, "additional_fee_output_index": 1, "max_additional_fee_contribution_msat": "0"}
        before_mempool = self.rpc("getrawmempool")
        compared = compare_psbts(original, proposal, network="regtest", payjoin=constraints)
        self.assertEqual(self.rpc("getrawmempool"), before_mempool)
        self.assertEqual(compared["delta"]["fee_msat"], "1000000")
        self.assertEqual(compared["payjoin"]["status"], "checks_passed", compared["payjoin"])
        self.assertEqual(compared["payjoin"]["receiver_contribution_msat"], "25000000000")
        self.assertIsNone(compared["payjoin"]["safe_to_sign"])
        self.assertFalse(compared["payjoin"]["negotiation_verified"])
        self.assertIsNone(compared["after"]["totals"]["final_fee_rate_sat_vb"])
        # Mutating a receiver-signed proposal must invalidate its signature and
        # output-order checks, even though scripts and aggregate fees still match.
        reordered = PSBT.from_base64(proposal)
        reordered.outputs.reverse()
        rejected = compare_psbts(original, reordered.to_base64(), network="regtest", payjoin=constraints)
        failed = {row["code"] for row in rejected["payjoin"]["checks"] if row["status"] == "failed"}
        self.assertIn("original_output_order_preserved", failed)
        self.assertIn("receiver_signature_commitments_valid", failed)
        finalized = self.process(proposal, sender)
        txid, decoded = self.broadcast(finalized)
        self.assertEqual(analyze_psbt(finalized, network="regtest")["totals"]["final_vsize"], decoded["vsize"])
        self.acquire(txid)
        self.assert_reference_only()

    def test_relative_lock_and_rbf_features_match_core_consensus_readiness(self):
        wallet = self.wallet()
        point = self.fund(wallet)
        encoded = self.process(self.create_psbt([point], [{self.address(self.miner): 0.9999}], sequence=2), wallet)
        result = analyze_psbt(encoded, network="regtest")
        sequence = _feature(result["features"], "sequences")["value"]
        self.assertTrue(sequence["signals_bip125"])
        self.assertEqual(sequence["relative_locks"], [{"input_index": 0, "unit": "blocks", "minimum": 2}])
        self.assertIn("relative_lock_constraints", {row["code"] for row in result["findings"]})
        final = self.rpc("finalizepsbt", [encoded])
        rejected = self.rpc("testmempoolaccept", [[final["hex"]]])[0]
        self.assertFalse(rejected["allowed"])
        self.assertEqual(rejected["reject-reason"], "non-BIP68-final")
        self.mine()
        txid, _ = self.broadcast(encoded)
        self.acquire(txid)
        # Absolute height locks are a different prerequisite from BIP68. A
        # non-final sequence enables nLockTime without signalling opt-in RBF.
        point = self.fund(wallet)
        height = self.rpc("getblockcount") + 2
        locked = self.process(self.create_psbt([point], [{self.address(self.miner): 0.9999}], sequence=0xFFFFFFFE, locktime=height), wallet)
        features = analyze_psbt(locked, network="regtest")["features"]
        self.assertEqual(_feature(features, "absolute_locktime")["value"], {"value": height, "kind": "height", "enabled": True})
        self.assertFalse(_feature(features, "sequences")["value"]["signals_bip125"])
        self.assertEqual(_feature(features, "sequences")["value"]["relative_locks"], [])
        raw = self.rpc("finalizepsbt", [locked])["hex"]
        rejected = self.rpc("testmempoolaccept", [[raw]])[0]
        self.assertFalse(rejected["allowed"])
        self.assertEqual(rejected["reject-reason"], "non-final")
        self.mine(3)
        txid, _ = self.broadcast(locked)
        self.acquire(txid)
        self.assert_reference_only()

    def _unload_wallets(self):
        for name in reversed(self.names):
            try:
                self.rpc("unloadwallet", [name])
            except Exception:
                # EXIT cleanup still removes this lane's entire disposable node.
                pass
