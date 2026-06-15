"""Pure-engine tests for kassiber.core.ownership (no subprocess, no network)."""

import json
import sqlite3
import unittest

from kassiber.core import ownership
from kassiber.core.ownership import OwnedIndex, OwnedMatch


def _engine_conn():
    """Minimal in-memory DB with just the tables the engine reads."""
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    conn.executescript(
        """
        CREATE TABLE accounts (id TEXT, code TEXT, label TEXT);
        CREATE TABLE wallets (
            id TEXT, profile_id TEXT, label TEXT, kind TEXT,
            config_json TEXT, account_id TEXT
        );
        CREATE TABLE wallet_utxos (
            profile_id TEXT, wallet_id TEXT, txid TEXT, vout INTEGER,
            address TEXT, branch_label TEXT, branch_index INTEGER,
            address_index INTEGER, chain TEXT, network TEXT
        );
        CREATE TABLE transactions (
            profile_id TEXT, wallet_id TEXT, external_id TEXT, raw_json TEXT
        );
        """
    )
    return conn


def _match(label="Vault", branch="receive", index=0, source="derived", **kw):
    return OwnedMatch(
        wallet_id=kw.get("wallet_id", "w1"),
        wallet_label=label,
        account=kw.get("account", "treasury"),
        chain=kw.get("chain", "bitcoin"),
        network=kw.get("network", "main"),
        branch_label=branch,
        address_index=index,
        derivation_path=kw.get("derivation_path", f"m/84'/0'/0'/0/{index}"),
        source=source,
    )


class ClassifyTokenTypeTests(unittest.TestCase):
    def test_64_hex_is_txid(self):
        self.assertEqual(ownership.classify_token_type("a" * 64), "txid")
        self.assertEqual(ownership.classify_token_type("00" * 32), "txid")

    def test_address_is_address(self):
        self.assertEqual(
            ownership.classify_token_type("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"),
            "address",
        )

    def test_64_non_hex_is_address(self):
        self.assertEqual(ownership.classify_token_type("z" * 64), "address")


class ParseTokensTests(unittest.TestCase):
    def test_forced_types_and_autodetect(self):
        parsed, invalid = ownership.parse_tokens(
            addresses=["bc1qaddr"],
            txids=["ab" * 32],
            candidates=["cd" * 32, "bc1qcandidate"],
        )
        self.assertEqual(invalid, [])
        kinds = {(p["type"], p["normalized"]) for p in parsed}
        self.assertIn(("address", "bc1qaddr"), kinds)
        self.assertIn(("txid", "ab" * 32), kinds)
        self.assertIn(("txid", "cd" * 32), kinds)
        self.assertIn(("address", "bc1qcandidate"), kinds)

    def test_dedup_and_comments_and_blanks(self):
        parsed, invalid = ownership.parse_tokens(
            file_text="bc1qaddr\n# a comment\n\nbc1qaddr\n",
        )
        self.assertEqual(invalid, [])
        self.assertEqual(len(parsed), 1)
        self.assertEqual(parsed[0]["normalized"], "bc1qaddr")

    def test_invalid_forced_txid(self):
        parsed, invalid = ownership.parse_tokens(txids=["not-a-real-txid"])
        self.assertEqual(parsed, [])
        self.assertEqual(len(invalid), 1)
        self.assertEqual(invalid[0]["type"], "txid")


class ClassifyAddressTests(unittest.TestCase):
    def test_owned_address_match(self):
        index = OwnedIndex()
        index.add_address("bc1qOWNED", _match(branch="change", index=7))
        result = ownership.classify_address(
            {"input": "bc1qowned", "type": "address", "chain": "bitcoin"}, index
        )
        self.assertEqual(result["status"], "owned")
        self.assertEqual(result["classification"], "owned_address")
        self.assertEqual(result["matches"][0]["branch"], "change")
        self.assertIn("change #7", result["note"])

    def test_external_address(self):
        index = OwnedIndex()
        result = ownership.classify_address(
            {"input": "bc1qexternal", "type": "address", "chain": "bitcoin"}, index
        )
        self.assertEqual(result["status"], "external")
        self.assertEqual(result["matches"], [])

    def test_multiple_owners_noted(self):
        index = OwnedIndex()
        index.add_address("bc1qshared", _match(label="WalletA"))
        index.add_address("bc1qshared", _match(label="WalletB", wallet_id="w2"))
        result = ownership.classify_address(
            {"input": "bc1qshared", "type": "address", "chain": "bitcoin"}, index
        )
        self.assertEqual(result["status"], "owned")
        self.assertIn("Also matches", result["note"])


class ClassifyTxidTests(unittest.TestCase):
    def _index(self):
        index = OwnedIndex()
        index.by_outpoint["aa" * 32 + ":0"] = _match(branch="change", index=3)
        index.add_script("00146f6e656473637269707400000000000000000000", _match())
        return index

    def test_self_transfer(self):
        index = self._index()
        legs = {
            "inputs": [{"outpoint": "aa" * 32 + ":0", "script": None}],
            "outputs": [{"n": 0, "script": "00146f6e656473637269707400000000000000000000"}],
            "chain": "bitcoin",
            "source": "chain",
        }
        result = ownership.classify_txid(
            {"input": "bb" * 32, "normalized": "bb" * 32, "type": "txid"}, index, legs
        )
        self.assertEqual(result["classification"], "self_transfer")
        self.assertEqual(result["owned_inputs"], 1)
        self.assertEqual(result["external_outputs"], 0)

    def test_outbound_payment_with_change(self):
        index = self._index()
        legs = {
            "inputs": [{"outpoint": "aa" * 32 + ":0", "script": None}],
            "outputs": [
                {"n": 0, "script": "00146f6e656473637269707400000000000000000000"},
                {"n": 1, "script": "0014deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"},
            ],
            "chain": "bitcoin",
            "source": "chain",
        }
        result = ownership.classify_txid(
            {"input": "bb" * 32, "normalized": "bb" * 32, "type": "txid"}, index, legs
        )
        self.assertEqual(result["classification"], "outbound_payment")
        self.assertEqual(result["external_outputs"], 1)

    def test_inbound_receipt(self):
        index = self._index()
        legs = {
            "inputs": [{"outpoint": "ff" * 32 + ":9", "script": None}],
            "outputs": [{"n": 0, "script": "00146f6e656473637269707400000000000000000000"}],
            "chain": "bitcoin",
            "source": "chain",
        }
        result = ownership.classify_txid(
            {"input": "bb" * 32, "normalized": "bb" * 32, "type": "txid"}, index, legs
        )
        self.assertEqual(result["classification"], "inbound_receipt")
        self.assertEqual(result["owned_inputs"], 0)

    def test_external_transaction(self):
        index = self._index()
        legs = {
            "inputs": [{"outpoint": "ff" * 32 + ":9", "script": None}],
            "outputs": [{"n": 0, "script": "0014deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"}],
            "chain": "bitcoin",
            "source": "chain",
        }
        result = ownership.classify_txid(
            {"input": "bb" * 32, "normalized": "bb" * 32, "type": "txid"}, index, legs
        )
        self.assertEqual(result["status"], "external")
        self.assertEqual(result["classification"], "external")

    def test_no_legs_but_local_txid(self):
        index = OwnedIndex()
        index.note_txid("cc" * 32, "w1", "Vault")
        result = ownership.classify_txid(
            {"input": "cc" * 32, "normalized": "cc" * 32, "type": "txid"}, index, None
        )
        self.assertEqual(result["status"], "owned")
        self.assertEqual(result["classification"], "touches_wallet")
        self.assertIn("Vault", result["wallets"])

    def test_no_legs_unknown(self):
        index = OwnedIndex()
        result = ownership.classify_txid(
            {"input": "dd" * 32, "normalized": "dd" * 32, "type": "txid"}, index, None
        )
        self.assertEqual(result["status"], "unknown")


class LocalTxLegsTests(unittest.TestCase):
    def test_legs_from_esplora_shape(self):
        raw = (
            '{"vin":[{"txid":"AA","vout":0,"prevout":{"scriptpubkey":"0014aa"}}],'
            '"vout":[{"scriptpubkey":"0014bb"},{"scriptpubkey":"0014cc"}]}'
        )
        legs = ownership._legs_from_local_tx_json(raw)
        self.assertEqual(legs["chain"], "bitcoin")
        self.assertEqual(legs["inputs"][0]["outpoint"], "aa:0")
        self.assertEqual(legs["inputs"][0]["script"], "0014aa")
        self.assertEqual([o["script"] for o in legs["outputs"]], ["0014bb", "0014cc"])

    def test_legs_from_electrum_decode_shape(self):
        # Electrum-synced raw_json uses script_hex on outputs and has no inline
        # prevout script; output ownership must still resolve.
        raw = (
            '{"vin":[{"txid":"AA","vout":1,"sequence":4294967295}],'
            '"vout":[{"n":0,"value":1,"script_hex":"0014bb"},'
            '{"n":1,"value":2,"script_hex":"0014cc"}]}'
        )
        legs = ownership._legs_from_local_tx_json(raw)
        self.assertEqual(legs["inputs"][0]["outpoint"], "aa:1")
        self.assertIsNone(legs["inputs"][0]["script"])
        self.assertEqual([o["script"] for o in legs["outputs"]], ["0014bb", "0014cc"])

    def test_electrum_shape_classifies_receipt_not_external(self):
        # Regression: the esplora-only parser read None scripts for Electrum
        # rows, misclassifying an owned receipt as external.
        index = OwnedIndex()
        index.add_script("0014bb", _match(branch="receive", index=0))
        legs = ownership._legs_from_local_tx_json(
            '{"vin":[{"txid":"AA","vout":1}],"vout":[{"n":0,"script_hex":"0014bb"}]}'
        )
        result = ownership.classify_txid(
            {"input": "bb" * 32, "normalized": "bb" * 32, "type": "txid"}, index, legs
        )
        self.assertEqual(result["status"], "owned")
        self.assertEqual(result["classification"], "inbound_receipt")
        self.assertEqual(result["owned_outputs"], 1)

    def test_legs_from_non_tx_returns_none(self):
        self.assertIsNone(ownership._legs_from_local_tx_json('{"component": {}}'))
        self.assertIsNone(ownership._legs_from_local_tx_json("not json"))


class LegEdgeCaseTests(unittest.TestCase):
    def test_liquid_fee_output_does_not_break_self_transfer(self):
        # The Liquid fee output carries an empty scriptPubKey; it must not count
        # as an external recipient and flip a self-transfer to outbound payment.
        index = OwnedIndex()
        index.by_outpoint["aa" * 32 + ":0"] = _match()
        index.add_script("0014owned00000000000000000000000000000000", _match(branch="change"))
        legs = {
            "inputs": [{"outpoint": "aa" * 32 + ":0", "script": None}],
            "outputs": [
                {"n": 0, "script": "0014owned00000000000000000000000000000000"},
                {"n": 1, "script": ""},  # Liquid fee output
            ],
            "chain": "liquid",
            "source": "chain",
        }
        result = ownership.classify_txid(
            {"input": "bb" * 32, "normalized": "bb" * 32, "type": "txid"}, index, legs
        )
        self.assertEqual(result["classification"], "self_transfer")
        self.assertEqual(result["external_outputs"], 0)

    def test_zero_output_does_not_assert_self_transfer(self):
        index = OwnedIndex()
        index.by_outpoint["aa" * 32 + ":0"] = _match()
        legs = {
            "inputs": [{"outpoint": "aa" * 32 + ":0", "script": None}],
            "outputs": [],
            "chain": "bitcoin",
            "source": "chain",
        }
        result = ownership.classify_txid(
            {"input": "bb" * 32, "normalized": "bb" * 32, "type": "txid"}, index, legs
        )
        self.assertNotEqual(result["classification"], "self_transfer")
        self.assertEqual(result["classification"], "undetermined")


class ChainDetectAndDedupTests(unittest.TestCase):
    def test_detect_chain(self):
        self.assertEqual(ownership._detect_chain("lq1qexample"), "liquid")
        self.assertEqual(ownership._detect_chain("ex1qexample"), "liquid")
        self.assertEqual(ownership._detect_chain("bc1qexample"), "bitcoin")
        self.assertEqual(ownership._detect_chain("1someBase58"), "")

    def test_case_variant_bech32_dedup(self):
        parsed, invalid = ownership.parse_tokens(
            candidates=["BC1QABCDEFGH", "bc1qabcdefgh"]
        )
        self.assertEqual(invalid, [])
        self.assertEqual(len(parsed), 1)

    def test_malformed_address_is_invalid(self):
        parsed, invalid = ownership.parse_tokens(addresses=["bc1q has a space"])
        self.assertEqual(parsed, [])
        self.assertEqual(len(invalid), 1)
        self.assertEqual(invalid[0]["type"], "address")

    def test_liquid_confidential_address_owned_via_string_fallback(self):
        # _script_hex_for_address can't canonicalize a Liquid confidential
        # address, so ownership rides the address-string index.
        index = OwnedIndex()
        index.add_address("lq1qwconfidentialexampleaddress", _match(chain="liquid", network="liquidv1"))
        result = ownership.classify_address(
            {"input": "lq1qwconfidentialexampleaddress", "type": "address", "chain": "liquid"},
            index,
        )
        self.assertEqual(result["status"], "owned")


class FlattenAndRedactTests(unittest.TestCase):
    def test_flatten_rows_share_keys(self):
        address_row = ownership.classify_address(
            {"input": "bc1qx", "type": "address", "chain": "bitcoin"}, OwnedIndex()
        )
        txid_row = ownership.classify_txid(
            {"input": "ee" * 32, "normalized": "ee" * 32, "type": "txid"},
            OwnedIndex(),
            None,
        )
        flat_addr = ownership.flatten_result_row(address_row)
        flat_txid = ownership.flatten_result_row(txid_row)
        self.assertEqual(set(flat_addr.keys()), set(flat_txid.keys()))

    def test_redact_drops_geometry(self):
        index = OwnedIndex()
        index.add_address("bc1qowned", _match(branch="change", index=12))
        owned = ownership.classify_address(
            {"input": "bc1qowned", "type": "address", "chain": "bitcoin"}, index
        )
        redacted = ownership.redact_result_for_ai(owned)
        self.assertNotIn("matches", redacted)
        self.assertNotIn("change #12", redacted["note"])
        self.assertEqual(redacted["wallets"], ["Vault"])
        self.assertEqual(redacted["status"], "owned")

    def test_redact_txid_drops_legs_and_geometry(self):
        # A txid result carries per-leg outpoints + branch labels; the AI
        # surface must not see any of it.
        index = OwnedIndex()
        index.by_outpoint["aa" * 32 + ":0"] = _match(branch="change", index=3)
        index.add_script("0014owned", _match())
        legs = {
            "inputs": [{"outpoint": "aa" * 32 + ":0", "script": None}],
            "outputs": [
                {"n": 0, "script": "0014owned"},
                {"n": 1, "script": "0014deadbeef"},
            ],
            "chain": "bitcoin",
            "source": "chain",
        }
        txid_result = ownership.classify_txid(
            {"input": "bb" * 32, "normalized": "bb" * 32, "type": "txid"}, index, legs
        )
        self.assertIn("legs", txid_result)  # full result has legs
        redacted = ownership.redact_result_for_ai(txid_result)
        self.assertNotIn("legs", redacted)
        self.assertNotIn("outpoint", json.dumps(redacted))
        self.assertNotIn("branch", json.dumps(redacted))
        self.assertEqual(redacted["status"], "owned")
        self.assertEqual(redacted["classification"], "outbound_payment")


class IdentifyVerifyTierTests(unittest.TestCase):
    def test_failed_verify_degrades_to_unknown_with_warning(self):
        conn = _engine_conn()

        def raiser(_txid, _chain):
            raise RuntimeError("HTTP 404 Not Found")

        report = ownership.identify(
            conn,
            "p1",
            txids=["ab" * 32],
            scan_to_index=0,
            verify_fetcher=raiser,
        )
        self.assertEqual(report["summary"]["unknown"], 1)
        result = report["results"][0]
        self.assertEqual(result["status"], "unknown")
        self.assertTrue(report["warnings"])
        self.assertIn("On-chain verify", report["warnings"][0])

    def test_successful_verify_classifies_outbound_payment(self):
        conn = _engine_conn()
        # Seed one owned outpoint so the verified tx's input resolves to a wallet.
        conn.execute(
            "INSERT INTO wallet_utxos(profile_id, wallet_id, txid, vout, address, "
            "branch_label, branch_index, address_index, chain, network) "
            "VALUES(?,?,?,?,?,?,?,?,?,?)",
            ("p1", "w1", "aa" * 32, 0, None, "receive", 0, 3, "bitcoin", "main"),
        )
        conn.execute(
            "INSERT INTO wallets(id, profile_id, label, kind, config_json, account_id) "
            "VALUES(?,?,?,?,?,?)",
            ("w1", "p1", "Vault", "descriptor", "{}", None),
        )
        conn.commit()

        def fetcher(_txid, _chain):
            return {
                "inputs": [{"outpoint": "aa" * 32 + ":0", "script": None}],
                "outputs": [{"n": 0, "script": "0014deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"}],
                "chain": "bitcoin",
                "source": "chain",
            }

        report = ownership.identify(
            conn,
            "p1",
            txids=["bb" * 32],
            scan_to_index=0,
            verify_fetcher=fetcher,
        )
        result = report["results"][0]
        self.assertEqual(result["status"], "owned")
        self.assertEqual(result["classification"], "outbound_payment")
        self.assertEqual(result["owned_inputs"], 1)
        self.assertEqual(result["external_outputs"], 1)
        self.assertEqual(result["match_source"], "chain")


if __name__ == "__main__":
    unittest.main()
